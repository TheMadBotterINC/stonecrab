//! Text on the wire (slice S49, ADR-0025): `CHAR`, `WCHAR`, `STRING(n)` and `WSTRING(n)`
//! parsed, sized, read and written through `ModbusTcpDriver` over the fake transport and
//! the fake clock.
//!
//! Every frame is written out by hand from the MBAP layout and the V1.1b PDU layouts, and
//! the text bytes from the character tables of UTF-8 and UTF-16; no byte comes from the
//! encoder under test. Cases transcribed from upstream name their file; the upstream tests
//! drive `ModbusRegisterCodec` and the tag classes directly, and here the same bytes and
//! values go through the driver instead. Scripts are strict: a frame the script does not
//! expect is a mismatch, and an empty script proves that nothing was sent.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroUsize};
    use std::sync::Arc;

    use proptest::prelude::*;
    use stonecrab::modbus::{
        BatchLimits, Batcher, ModbusAddress, ModbusArea, ModbusDataType, ModbusTcpDriver,
        TypedDecoder,
    };
    use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    use stonecrab::{
        AddressOptions, ByteOrder, Connection, Driver, ErrorKind, ItemError, PlcValue,
        ProtocolAddress, ReadRequest, ReadResponse, Result, Tag, TagId, WordOrder, WriteOutcome,
        WriteRequest, WriteResponse,
    };

    /// An ADU of unit 1 as it goes on the wire.
    fn adu(txn: u16, pdu: &[u8]) -> Vec<u8> {
        let length = u16::try_from(pdu.len().saturating_add(1)).expect("a short PDU");
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(1);
        bytes.extend_from_slice(pdu);
        bytes
    }

    /// One request and its reply, as two script steps.
    fn exchange(txn: u16, request: &[u8], reply: &[u8]) -> [Step; 2] {
        [
            Step::Expect(adu(txn, request)),
            Step::Reply(adu(txn, reply)),
        ]
    }

    /// A write request and the echo that confirms it (the first five PDU bytes).
    fn echoed(txn: u16, request: &[u8]) -> [Step; 2] {
        let echo = request.get(..5).expect("a write request PDU");
        exchange(txn, request, echo)
    }

    /// An FC03 read of `quantity` holding registers from wire address `start`.
    fn read_holding(start: u16, quantity: u16) -> Vec<u8> {
        let mut bytes = vec![0x03];
        bytes.extend_from_slice(&start.to_be_bytes());
        bytes.extend_from_slice(&quantity.to_be_bytes());
        bytes
    }

    /// An FC03 reply carrying `data`, the register bytes as they go on the wire.
    fn holding_reply(data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x03, u8::try_from(data.len()).expect("at most 250 bytes")];
        bytes.extend_from_slice(data);
        bytes
    }

    /// An FC16 write of `data` (whole registers) at wire address `start`.
    fn write_multiple(start: u16, data: &[u8]) -> Vec<u8> {
        let quantity = u16::try_from(data.len().div_ceil(2)).expect("a few registers");
        let mut bytes = vec![0x10];
        bytes.extend_from_slice(&start.to_be_bytes());
        bytes.extend_from_slice(&quantity.to_be_bytes());
        bytes.push(u8::try_from(data.len()).expect("at most 246 bytes"));
        bytes.extend_from_slice(data);
        bytes
    }

    fn text(value: &str) -> PlcValue {
        PlcValue::String(value.to_owned())
    }

    fn list(values: impl IntoIterator<Item = PlcValue>) -> PlcValue {
        PlcValue::List(values.into_iter().collect())
    }

    fn none() -> AddressOptions {
        AddressOptions::default()
    }

    fn parse(text: &str) -> ModbusAddress {
        ModbusAddress::parse(text, &none()).unwrap_or_else(|e| panic!("{text:?} rejected: {e}"))
    }

    /// The registers one read of `text` asks for, from the batcher's plan.
    fn registers_read(text: &str) -> u16 {
        let tag = Tag::new(TagId::new("t").expect("tag id"), Arc::new(parse(text)));
        let chunks = Batcher::plan(&[tag], &BatchLimits::default()).expect("a plan");
        let [chunk] = chunks.as_slice() else {
            panic!("{text}: one chunk, got {chunks:?}");
        };
        chunk.quantity
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        driver: ModbusTcpDriver,
        options: AddressOptions,
        connection: Box<dyn Connection>,
    }

    impl Rig {
        /// A read-write connection of `ModbusTcpDriver` over one strict script; `query`
        /// adds connection parameters.
        fn open(query: &str, steps: Vec<Step>) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            factory.push_script(Script::strict(steps));
            let driver = ModbusTcpDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_transport_factory(Arc::new(factory.clone()));
            let config = driver
                .parse_config(&format!("modbus-tcp://192.0.2.10?access=read-write{query}"))
                .expect("config");
            let options = config.address_options();
            let connection = block_on_with(&clock, driver.connect(config)).expect("connect");
            Self {
                clock,
                factory,
                driver,
                options,
                connection,
            }
        }

        fn tag(&self, id: &str, text: &str) -> Tag {
            let address = self
                .driver
                .parse_address(text, &self.options)
                .unwrap_or_else(|e| panic!("{text}: {e}"));
            Tag::new(TagId::new(id).expect("tag id"), address)
        }

        fn read(&self, addresses: &[(&str, &str)]) -> ReadResponse {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for (id, text) in addresses {
                builder = builder.tag(self.tag(id, text));
            }
            let request = builder.build().expect("a read request");
            block_on_with(&self.clock, self.connection.read(request)).expect("a read")
        }

        fn write_request(&self, items: Vec<(Tag, PlcValue)>) -> Result<WriteRequest> {
            let mut builder = WriteRequest::builder(self.connection.limits());
            for (tag, value) in items {
                builder = builder.item(tag, value);
            }
            builder.build()
        }

        fn write(&self, items: Vec<(&str, &str, PlcValue)>) -> WriteResponse {
            let items = items
                .into_iter()
                .map(|(id, text, value)| (self.tag(id, text), value))
                .collect();
            let request = self.write_request(items).expect("write request");
            block_on_with(&self.clock, self.connection.write(request)).expect("a write")
        }

        /// The frames the transport received, and that it saw no unexpected frame.
        fn sent(&self) -> Vec<Vec<u8>> {
            let probes = self.factory.probes();
            let probe = probes.first().expect("a transport");
            assert!(
                probe.report().mismatches.is_empty(),
                "every frame was expected: {:?}",
                probe.report().mismatches
            );
            probe.sent().iter().map(|frame| frame.to_vec()).collect()
        }
    }

    fn value(response: &ReadResponse, id: &str) -> PlcValue {
        match response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("an item for {id}"))
            .outcome()
        {
            Ok(value) => value.clone(),
            Err(error) => panic!("{id} failed: {error}"),
        }
    }

    fn failure<'a>(response: &'a ReadResponse, id: &str) -> &'a ItemError {
        match response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("an item for {id}"))
            .outcome()
        {
            Ok(value) => panic!("{id} was expected to fail, read {value:?}"),
            Err(error) => error,
        }
    }

    fn outcome<'a>(response: &'a WriteResponse, id: &str) -> &'a WriteOutcome {
        response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("a result for {id}"))
            .outcome()
    }

    fn rejection<'a>(response: &'a WriteResponse, id: &str) -> &'a ItemError {
        let found = outcome(response, id);
        let WriteOutcome::Rejected(error) = found else {
            panic!("{id} was expected to be rejected, got {found:?}");
        };
        error
    }

    // ---- addresses and sizes -------------------------------------------------------------

    // covers: MB-TYPE-STRING, MB-TYPE-WSTRING, MB-LIMIT-REGISTERS / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusStringTest.java
    // (sizesTheRequestFromTheDeclaredLength, keepsTheDeclaredLengthOnTheTag,
    // leavesOtherDataTypesUnchanged) and plc4go/internal/modbus/Reader_test.go
    // (TestReader_asksForTheRegistersAStringOccupies)
    #[test]
    fn a_string_is_sized_from_its_declared_length() {
        assert_eq!(registers_read("holding-register:1:STRING(12)"), 6);
        assert_eq!(registers_read("holding-register:1[0..2]:STRING(12)"), 18);
        assert_eq!(registers_read("holding-register:1:WSTRING(12)"), 12);
        assert_eq!(registers_read("holding-register:1:STRING(20)"), 10);
        let three = parse("holding-register:1[0..2]:STRING(20)");
        assert_eq!(three.string_length().map(NonZeroU16::get), Some(20));
        assert_eq!(three.elements().map(NonZeroU16::get), Some(3));
        assert_eq!(parse("holding-register:1:INT").string_length(), None);
        assert_eq!(registers_read("holding-register:1[0..6]:INT"), 7);
        assert_eq!(registers_read("holding-register:1[0..6]:DINT"), 14);
        // An odd length rounds up, and the largest string fits one read exactly.
        assert_eq!(registers_read("holding-register:1:STRING(3)"), 2);
        assert_eq!(registers_read("holding-register:1:STRING(250)"), 125);
        assert_eq!(registers_read("input-register:1:WSTRING(125)"), 125);
        assert_eq!(registers_read("extended-register:1:STRING(240)"), 120);
    }

    // covers: MB-TYPE-CHAR, MB-ADDR-SELECTION / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/base/tag/ModbusSelectionOffsetTest.java
    // (aSubRegisterValueStillOccupiesARegister, aSelectionStartingInsideARegisterIsRejected,
    // stringsAdvanceByTheirDeclaredLength): characters pack two to a register, and a
    // selection may not start inside one
    #[test]
    fn characters_pack_two_to_a_register_and_a_selection_starts_on_a_register() {
        assert_eq!(registers_read("holding-register:1:CHAR"), 1);
        assert_eq!(registers_read("holding-register:1[0..1]:CHAR"), 1);
        assert_eq!(registers_read("holding-register:1[0..2]:CHAR"), 2);
        assert_eq!(registers_read("holding-register:1[0..249]:CHAR"), 125);
        assert_eq!(registers_read("holding-register:1:WCHAR"), 1);
        assert_eq!(registers_read("holding-register:1[0..1]:WCHAR"), 2);
        assert_eq!(parse("holding-register:1[2..3]:CHAR").offset(), 1);
        assert_eq!(parse("holding-register:1[2]:STRING(20)").offset(), 20);
        assert_eq!(parse("holding-register:1[2]:STRING(3)").offset(), 3);
        assert_eq!(parse("holding-register:1[1]:WSTRING(3)").offset(), 3);
        assert_eq!(parse("extended-register:1[2]:CHAR").offset(), 2);
        for text in [
            "holding-register:1[1..2]:CHAR",
            "holding-register:1[1]:CHAR",
            "holding-register:1[3;0]:CHAR",
            "holding-register:1[1]:STRING(3)",
            "holding-register:1[3..4]:STRING(5)",
            "extended-register:1[1]:CHAR",
        ] {
            let error = ModbusAddress::parse(text, &none()).expect_err(text);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{text}");
            assert!(
                error.to_string().contains("odd byte offset"),
                "{text}: {error}"
            );
        }
        // Past the read limit in characters: 251 CHARs are 126 registers.
        let wide = ModbusAddress::parse("holding-register:1[0..250]:CHAR", &none());
        assert_eq!(wide.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    }

    // covers: MB-TYPE-STRING, MB-ADDR-SELECTION / the canonical text writes the length and
    // reads back equal, and the builder demands what the text demands
    #[test]
    fn the_canonical_text_writes_the_length_and_the_builder_checks_it() {
        for text in [
            "holding-register:1:STRING(20)",
            "input-register:7:WSTRING(3){unit-id: 4}",
            "holding-register:2[0..2]:STRING(4){byte-order: 'BIG_ENDIAN_BYTE_SWAP'}",
            "holding-register:1[0..9]:CHAR",
            "extended-register:30000:WCHAR",
        ] {
            let address = parse(text);
            assert_eq!(address.canonical(), text, "canonical");
            assert_eq!(parse(&address.canonical()), address, "{text}: reads back");
        }
        let length = NonZeroU16::new(4).expect("four");
        let built = ModbusAddress::builder(ModbusArea::HoldingRegister, 1)
            .data_type(ModbusDataType::WString)
            .string_length(length)
            .build()
            .expect("a WSTRING(4)");
        assert_eq!(
            built,
            parse("holding-register:2:WSTRING(4)"),
            "builder and text agree"
        );
        let missing = ModbusAddress::builder(ModbusArea::HoldingRegister, 1)
            .data_type(ModbusDataType::String)
            .build();
        assert_eq!(
            missing.map_err(|e| e.kind()),
            Err(ErrorKind::InvalidAddress)
        );
        let stray = ModbusAddress::builder(ModbusArea::HoldingRegister, 1)
            .string_length(length)
            .build();
        assert_eq!(stray.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    }

    // covers: MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING / the public
    // TypedDecoder functions take one register of a text type, a STRING as STRING(1)
    #[test]
    fn the_typed_decoder_reads_and_writes_one_register_of_text() {
        let budget = NonZeroUsize::new(16).expect("a budget");
        let (big, high) = (ByteOrder::BigEndian, WordOrder::HighWordFirst);
        for (data_type, register, value, written) in [
            (ModbusDataType::Char, 0x4142, PlcValue::Char('A'), 0x4100),
            (ModbusDataType::WChar, 0x03A9, PlcValue::Char('Ω'), 0x03A9),
            (ModbusDataType::String, 0x4142, text("A"), 0x4100),
            (ModbusDataType::WString, 0x0041, text("A"), 0x0041),
        ] {
            let read = TypedDecoder::decode_registers(&[register], data_type, big, high, budget);
            assert_eq!(read.ok(), Some(value.clone()), "{data_type:?}: read");
            let encoded = TypedDecoder::encode_registers(&value, data_type, big, high);
            assert_eq!(encoded.ok(), Some(vec![written]), "{data_type:?}: written");
        }
        let swapped = TypedDecoder::decode_registers(
            &[0x4142],
            ModbusDataType::Char,
            ByteOrder::LittleEndian,
            high,
            budget,
        );
        assert_eq!(swapped.ok(), Some(PlcValue::Char('B')), "byte-swapped");
        let invalid =
            TypedDecoder::decode_registers(&[0x8041], ModbusDataType::Char, big, high, budget);
        let error = invalid.expect_err("0x80 is no UTF-8 character");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "invalid");
        assert_eq!(
            error.status().and_then(stonecrab::ProtocolStatus::raw),
            Some(&[0x80, 0x41][..]),
            "the register kept"
        );
        let short = TypedDecoder::decode_registers(&[], ModbusDataType::WChar, big, high, budget);
        assert_eq!(
            short.map_err(|error| error.kind()),
            Err(ErrorKind::MalformedData),
            "no register"
        );
        let refused =
            TypedDecoder::encode_registers(&PlcValue::Char('é'), ModbusDataType::Char, big, high);
        assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    }

    // ---- reads ---------------------------------------------------------------------------

    // covers: MB-TYPE-STRING, MB-TYPE-WSTRING, MB-TCP-READ-HR, MB-TCP-READ-IR / transcribed
    // from upstream plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusStringTest.java
    // (readsASingleString, readsAnArrayOfStrings, readsAWideString) and
    // plc4go/internal/modbus/Reader_test.go (TestReader_decodesAStringOfTheDeclaredLength)
    #[test]
    fn upstream_string_reads_decode_through_the_driver() {
        let mut steps = Vec::new();
        steps.extend(exchange(
            1,
            &read_holding(0, 6),
            &holding_reply(b"Hello Toddy!"),
        ));
        steps.extend(exchange(
            2,
            &read_holding(0, 6),
            &holding_reply(b"abcdefghijkl"),
        ));
        // "Hi" in UTF-16BE: 0x0048 0x0069.
        steps.extend(exchange(
            3,
            &[0x04, 0x00, 0x00, 0x00, 0x02],
            &[0x04, 0x04, 0x00, 0x48, 0x00, 0x69],
        ));
        steps.extend(exchange(4, &read_holding(0, 3), &holding_reply(b"hello\0")));
        let rig = Rig::open("", steps);
        for (id, address, expected) in [
            (
                "single",
                "holding-register:1:STRING(12)",
                text("Hello Toddy!"),
            ),
            (
                "array",
                "holding-register:1[0..2]:STRING(4)",
                list([text("abcd"), text("efgh"), text("ijkl")]),
            ),
            ("wide", "input-register:1:WSTRING(2)", text("Hi")),
            ("declared", "holding-register:1:STRING(6)", text("hello")),
        ] {
            let response = rig.read(&[(id, address)]);
            assert_eq!(value(&response, id), expected, "{address}");
        }
        assert_eq!(rig.sent().len(), 4, "one request per read");
    }

    // covers: MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING / characters are
    // their own bytes, a string ends at its first NUL, and spaces and non-ASCII text are kept
    #[test]
    fn characters_and_strings_read_as_the_bytes_say() {
        let mut steps = Vec::new();
        // CHAR 'A' alone, then three packed CHARs 'x', 'y', 'z' and a pad byte.
        steps.extend(exchange(1, &read_holding(9, 1), &holding_reply(b"AB")));
        steps.extend(exchange(2, &read_holding(9, 2), &holding_reply(b"xyz\0")));
        // WCHAR U+03A9 and a WSTRING(4) "Grüß" (ü U+00FC, ß U+00DF).
        steps.extend(exchange(
            3,
            &read_holding(19, 5),
            &holding_reply(&[0x03, 0xA9, 0x00, 0x47, 0x00, 0x72, 0x00, 0xFC, 0x00, 0xDF]),
        ));
        // STRING(8): "ab", NUL, then bytes that are no UTF-8 and are not read.
        steps.extend(exchange(
            4,
            &read_holding(29, 4),
            &holding_reply(&[b'a', b'b', 0x00, 0xFF, 0xFE, b'c', b'd', b'e']),
        ));
        // STRING(6) with trailing spaces and UTF-8 "é" (0xC3 0xA9); a NUL CHAR.
        steps.extend(exchange(
            5,
            &read_holding(39, 4),
            &holding_reply(&[b'a', 0xC3, 0xA9, b' ', b' ', b' ', 0x00, 0x00]),
        ));
        let rig = Rig::open("", steps);
        let single = rig.read(&[("single", "holding-register:10:CHAR")]);
        assert_eq!(
            value(&single, "single"),
            PlcValue::Char('A'),
            "the first byte"
        );
        let run = rig.read(&[("run", "holding-register:10[0..2]:CHAR")]);
        assert_eq!(
            value(&run, "run"),
            list([
                PlcValue::Char('x'),
                PlcValue::Char('y'),
                PlcValue::Char('z')
            ]),
            "packed, the pad byte unread"
        );
        let wide = rig.read(&[
            ("omega", "holding-register:20:WCHAR"),
            ("name", "holding-register:21:WSTRING(4)"),
        ]);
        assert_eq!(value(&wide, "omega"), PlcValue::Char('Ω'), "WCHAR");
        assert_eq!(value(&wide, "name"), text("Grüß"), "WSTRING, merged");
        let ended = rig.read(&[("ended", "holding-register:30:STRING(8)")]);
        assert_eq!(value(&ended, "ended"), text("ab"), "the first NUL ends it");
        let kept = rig.read(&[
            ("spaces", "holding-register:40:STRING(6)"),
            ("nul", "holding-register:43:CHAR"),
        ]);
        assert_eq!(value(&kept, "spaces"), text("aé   "), "spaces are text");
        assert_eq!(
            value(&kept, "nul"),
            PlcValue::Char('\0'),
            "a NUL CHAR is a NUL"
        );
        assert_eq!(
            rig.sent().len(),
            5,
            "the two-tag reads are one request each"
        );
    }

    // covers: MB-TYPE-WSTRING / a character outside the Basic Multilingual Plane is one
    // surrogate pair, high unit first, read and written as one character; the pair the other
    // way round is an unpaired surrogate
    #[test]
    fn a_surrogate_pair_reads_and_writes_as_one_character() {
        // U+1F600 in UTF-16 is the high surrogate 0xD83D followed by the low one 0xDE00.
        let pair = [0xD8, 0x3D, 0xDE, 0x00];
        let reversed = [0xDE, 0x00, 0xD8, 0x3D];
        let mut steps = Vec::from(exchange(1, &read_holding(0, 2), &holding_reply(&pair)));
        steps.extend(exchange(2, &read_holding(0, 2), &holding_reply(&reversed)));
        steps.extend(echoed(3, &write_multiple(0, &pair)));
        let rig = Rig::open("", steps);
        let address = "holding-register:1:WSTRING(2)";
        let read = rig.read(&[("smile", address)]);
        assert_eq!(
            value(&read, "smile"),
            text("\u{1F600}"),
            "two units, one character"
        );
        let swapped = rig.read(&[("swapped", address)]);
        let error = failure(&swapped, "swapped");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "the low unit first");
        assert!(
            error
                .to_string()
                .contains("unpaired UTF-16 surrogate 0xDE00"),
            "{error}"
        );
        assert_eq!(
            error.status().and_then(|status| status.raw()),
            Some(&reversed[..]),
            "the registers as received"
        );
        let written = rig.write(vec![("smile", address, text("\u{1F600}"))]);
        assert_eq!(written.confirmed_count(), 1, "{written:?}");
        assert_eq!(rig.sent().len(), 3, "two reads and one write");
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP,
    // MB-TYPE-STRING, MB-TYPE-CHAR, MB-TYPE-WSTRING / text is swapped within each register
    // under the two byte-swap modes only, as upstream applies the modes to strings, and an
    // odd length takes its last byte from the swapped register
    #[test]
    fn text_follows_the_byte_swap_modes_and_never_reverses_registers() {
        let plain = b"ABCD";
        let swapped = b"BADC";
        let mut steps = Vec::new();
        let mut txn = 0;
        for data in [plain, plain, swapped, swapped] {
            txn += 1;
            steps.extend(exchange(txn, &read_holding(0, 2), &holding_reply(data)));
        }
        // STRING(3) "ABC" and a lone CHAR 'C' byte-swapped: "BA" then the pad and 'C'.
        steps.extend(exchange(5, &read_holding(0, 2), &holding_reply(b"BA\0C")));
        steps.extend(exchange(6, &read_holding(1, 1), &holding_reply(b"\0C")));
        // WSTRING(2) "Hi" byte-swapped: each code unit low byte first.
        steps.extend(exchange(
            7,
            &read_holding(0, 2),
            &holding_reply(&[0x48, 0x00, 0x69, 0x00]),
        ));
        let rig = Rig::open("", steps);
        for mode in [
            "BIG_ENDIAN",
            "LITTLE_ENDIAN",
            "BIG_ENDIAN_BYTE_SWAP",
            "LITTLE_ENDIAN_BYTE_SWAP",
        ] {
            let address = format!("holding-register:1:STRING(4){{byte-order: '{mode}'}}");
            let response = rig.read(&[("text", &address)]);
            assert_eq!(value(&response, "text"), text("ABCD"), "{mode}");
        }
        let swap = "{byte-order: 'BIG_ENDIAN_BYTE_SWAP'}";
        let odd = rig.read(&[("odd", &format!("holding-register:1:STRING(3){swap}"))]);
        assert_eq!(
            value(&odd, "odd"),
            text("ABC"),
            "the whole register is swapped"
        );
        let lone = rig.read(&[("lone", &format!("holding-register:2:CHAR{swap}"))]);
        assert_eq!(value(&lone, "lone"), PlcValue::Char('C'), "the low byte");
        let wide = rig.read(&[("wide", &format!("holding-register:1:WSTRING(2){swap}"))]);
        assert_eq!(
            value(&wide, "wide"),
            text("Hi"),
            "UTF-16 swapped per register"
        );
        assert_eq!(rig.sent().len(), 7, "one request per read");
    }

    // covers: MB-TYPE-STRING, MB-TYPE-WSTRING, MB-TYPE-CHAR, MB-TYPE-WCHAR / SPEC
    // mandatory-failure-cases / F7 malformed data: text that does not decode fails its
    // item, keeps the bytes, and never fails a neighbour
    #[test]
    fn text_that_does_not_decode_fails_its_item_with_the_bytes_kept() {
        let mut steps = Vec::new();
        // STRING(4) "a", 0xFF, "b", NUL beside an INT 7; WSTRING(2) with a lone high
        // surrogate 0xD800; CHAR 0x80; WCHAR 0xDC00; a list of two CHARs whose second is 0xC3.
        steps.extend(exchange(
            1,
            &read_holding(0, 3),
            &holding_reply(&[b'a', 0xFF, b'b', 0x00, 0x00, 0x07]),
        ));
        steps.extend(exchange(
            2,
            &read_holding(9, 2),
            &holding_reply(&[0xD8, 0x00, 0x00, 0x41]),
        ));
        steps.extend(exchange(
            3,
            &read_holding(19, 1),
            &holding_reply(&[0x80, 0x41]),
        ));
        steps.extend(exchange(
            4,
            &read_holding(29, 1),
            &holding_reply(&[0xDC, 0x00]),
        ));
        steps.extend(exchange(
            5,
            &read_holding(39, 1),
            &holding_reply(&[b'o', 0xC3]),
        ));
        let rig = Rig::open("", steps);
        let mixed = rig.read(&[
            ("bad", "holding-register:1:STRING(4)"),
            ("count", "holding-register:3:INT"),
        ]);
        assert_eq!(
            value(&mixed, "count"),
            PlcValue::I16(7),
            "the neighbour reads"
        );
        let bad = failure(&mixed, "bad");
        assert_eq!(bad.kind(), ErrorKind::MalformedData, "STRING");
        assert!(bad.to_string().contains("UTF-8 from byte 1"), "{bad}");
        let status = bad.status().expect("a decode status");
        assert_eq!(status.name(), Some("decode"), "status name");
        assert_eq!(
            status.raw(),
            Some(&[b'a', 0xFF, b'b', 0x00][..]),
            "bytes kept"
        );
        for (id, address, needle, raw) in [
            (
                "surrogate",
                "holding-register:10:WSTRING(2)",
                "0xD800",
                &[0xD8, 0x00, 0x00, 0x41][..],
            ),
            (
                "high",
                "holding-register:20:CHAR",
                "UTF-8",
                &[0x80, 0x41][..],
            ),
            (
                "low",
                "holding-register:30:WCHAR",
                "0xDC00",
                &[0xDC, 0x00][..],
            ),
            (
                "pair",
                "holding-register:40[0..1]:CHAR",
                "UTF-8",
                &[b'o', 0xC3][..],
            ),
        ] {
            let response = rig.read(&[(id, address)]);
            let error = failure(&response, id);
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{address}");
            assert!(error.to_string().contains(needle), "{address}: {error}");
            assert_eq!(
                error.status().and_then(|status| status.raw()),
                Some(raw),
                "{address}: the registers as received"
            );
        }
        assert_eq!(rig.sent().len(), 5, "one request per read");
    }

    // covers: MB-EXC-02, MB-TYPE-STRING / a device exception fails a text item whole
    #[test]
    fn an_exception_fails_a_text_item() {
        let steps = Vec::from(exchange(1, &read_holding(0, 5), &[0x83, 0x02]));
        let rig = Rig::open("", steps);
        let response = rig.read(&[("name", "holding-register:1[0..1]:STRING(5)")]);
        let error = failure(&response, "name");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
        assert_eq!(error.status().map(stonecrab::ProtocolStatus::code), Some(2));
    }

    // covers: MB-ADDR-EXTENDED, MB-TYPE-STRING / text on the extended registers reads
    // through FC20 like any other type
    #[test]
    fn an_extended_string_reads_through_fc20() {
        // File 1, record 1, two registers; the reply's sub-response holds "OK!" and a NUL.
        let request = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x01, 0x00, 0x02];
        let reply = [0x14, 0x06, 0x05, 0x06, b'O', b'K', b'!', 0x00];
        let rig = Rig::open("", Vec::from(exchange(1, &request, &reply)));
        let response = rig.read(&[("status", "extended-register:1:STRING(4)")]);
        assert_eq!(value(&response, "status"), text("OK!"));
    }

    // ---- writes --------------------------------------------------------------------------

    // covers: MB-TCP-WRITE, MB-TYPE-STRING, MB-TYPE-WSTRING / transcribed from upstream
    // plc4go/internal/modbus/RegisterCodec_test.go (TestRoundTripsAStringOfTheDeclaredLength,
    // TestRoundTripsAWStringOfTheDeclaredLength, TestPacksSeveralStrings,
    // TestPacksAnOddNumberOfStringsWithoutAPadByte) and the "Hello Toddy!" value of
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusEncodeTest.java:
    // a string is padded with NUL bytes to its declared length, and several pack back to back
    #[test]
    fn strings_are_written_padded_to_their_declared_length() {
        let cases: [(&str, PlcValue, Vec<u8>); 6] = [
            (
                "holding-register:8:STRING(12)",
                text("Hello Toddy!"),
                b"Hello Toddy!".to_vec(),
            ),
            (
                "holding-register:8:STRING(6)",
                text("hi"),
                b"hi\0\0\0\0".to_vec(),
            ),
            (
                "holding-register:8:WSTRING(4)",
                text("hi"),
                vec![0x00, b'h', 0x00, b'i', 0, 0, 0, 0],
            ),
            (
                "holding-register:8[0..1]:STRING(4)",
                list([text("ab"), text("cd")]),
                b"ab\0\0cd\0\0".to_vec(),
            ),
            (
                "holding-register:8[0..2]:STRING(4)",
                list([text("ab"), text("cd"), text("ef")]),
                b"ab\0\0cd\0\0ef\0\0".to_vec(),
            ),
            (
                "holding-register:8[0..2]:WSTRING(2)",
                list([text("ab"), text("cd"), text("ef")]),
                vec![0, b'a', 0, b'b', 0, b'c', 0, b'd', 0, b'e', 0, b'f'],
            ),
        ];
        let mut steps = Vec::new();
        for (txn, (_, _, data)) in (1..).zip(&cases) {
            steps.extend(echoed(txn, &write_multiple(7, data)));
        }
        let rig = Rig::open("", steps);
        for (address, written, _) in cases {
            let response = rig.write(vec![("s", address, written)]);
            assert_eq!(response.confirmed_count(), 1, "{address}: {response:?}");
        }
        assert_eq!(rig.sent().len(), 6, "one request per write");
    }

    // covers: MB-TCP-WRITE, MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING / a character takes
    // the first byte of its register, characters pack, an odd run is padded, a one-register
    // value is FC06, and the byte-swap modes swap each register
    #[test]
    fn characters_and_odd_lengths_are_padded_to_whole_registers() {
        let mut steps = Vec::new();
        steps.extend(echoed(1, &[0x06, 0x00, 0x09, b'A', 0x00]));
        steps.extend(echoed(2, &write_multiple(9, b"xyz\0")));
        steps.extend(echoed(3, &[0x06, 0x00, 0x09, 0x03, 0xA9]));
        steps.extend(echoed(4, &write_multiple(9, b"abc\0")));
        steps.extend(echoed(5, &write_multiple(9, b"BADC")));
        steps.extend(echoed(6, &[0x06, 0x00, 0x09, 0x00, b'C']));
        steps.extend(echoed(7, &[0x06, 0x00, 0x09, b'o', b'k']));
        let rig = Rig::open("", steps);
        let swap = "{byte-order: 'LITTLE_ENDIAN_BYTE_SWAP'}";
        for (address, written) in [
            ("holding-register:10:CHAR".to_owned(), PlcValue::Char('A')),
            (
                "holding-register:10[0..2]:CHAR".to_owned(),
                list([
                    PlcValue::Char('x'),
                    PlcValue::Char('y'),
                    PlcValue::Char('z'),
                ]),
            ),
            ("holding-register:10:WCHAR".to_owned(), PlcValue::Char('Ω')),
            ("holding-register:10:STRING(3)".to_owned(), text("abc")),
            (format!("holding-register:10:STRING(4){swap}"), text("ABCD")),
            (
                format!("holding-register:10:CHAR{swap}"),
                PlcValue::Char('C'),
            ),
            ("holding-register:10:STRING(2)".to_owned(), text("ok")),
        ] {
            let response = rig.write(vec![("c", &address, written)]);
            assert_eq!(response.confirmed_count(), 1, "{address}: {response:?}");
        }
        assert_eq!(rig.sent().len(), 7, "one request per write");
    }

    // covers: MB-TCP-WRITE, MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING /
    // SPEC mandatory-failure-cases / F7 values that do not fit are refused before any I/O
    #[test]
    fn text_that_does_not_fit_is_refused_before_any_io() {
        let rig = Rig::open("", vec![]);
        let response = rig.write(vec![
            ("long", "holding-register:1:STRING(4)", text("hello")),
            ("bytes", "holding-register:10:STRING(4)", text("ééé")),
            ("wide", "holding-register:20:WSTRING(2)", text("abc")),
            ("pair", "holding-register:30:WSTRING(1)", text("😀")),
            ("accent", "holding-register:40:CHAR", PlcValue::Char('é')),
            ("astral", "holding-register:50:WCHAR", PlcValue::Char('😀')),
            ("nul", "holding-register:60:STRING(8)", text("a\0b")),
            ("wnul", "holding-register:70:WSTRING(8)", text("a\0b")),
            (
                "short",
                "holding-register:80[0..2]:STRING(2)",
                list([text("a"), text("b")]),
            ),
            (
                "mixed",
                "holding-register:90[0..1]:CHAR",
                list([PlcValue::Char('a'), text("b")]),
            ),
            ("input", "input-register:1:STRING(2)", text("no")),
        ]);
        for (id, needle) in [
            ("long", "takes 5 bytes of UTF-8, more than the 4"),
            ("bytes", "takes 6 bytes of UTF-8"),
            ("wide", "takes 6 bytes of UTF-16, more than the 4"),
            ("pair", "takes 4 bytes of UTF-16, more than the 2"),
            ("accent", "takes 2 bytes of UTF-8, more than the 1"),
            ("astral", "takes 4 bytes of UTF-16, more than the 2"),
            ("nul", "NUL"),
            ("wnul", "NUL"),
            ("short", "a list of exactly 3"),
            ("mixed", "not a CHAR value"),
        ] {
            let error = rejection(&response, id);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{id}");
            assert!(error.to_string().contains(needle), "{id}: {error}");
        }
        assert!(
            matches!(
                rejection(&response, "input").kind(),
                ErrorKind::Unsupported(_)
            ),
            "an input register is read-only"
        );
        for (address, written) in [
            ("holding-register:1:STRING(4)", PlcValue::Char('a')),
            ("holding-register:1:CHAR", text("a")),
            ("holding-register:1[0..1]:CHAR", text("ab")),
        ] {
            let tag = rig.tag("t", address);
            let refused = rig.write_request(vec![(tag, written)]);
            assert_eq!(
                refused.map(drop).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{address}: the request builder refuses the mismatch"
            );
        }
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: MB-TCP-WRITE, MB-TYPE-STRING, MB-TYPE-WSTRING, MB-LIMIT-REGISTERS / a string
    // that one read carries but one FC16 write does not (more than 123 registers) is refused
    // with ResourceLimit before any I/O, as a numeric list is; STRING(246) is 123 and is sent
    #[test]
    fn a_string_wider_than_one_write_is_refused_with_resource_limit() {
        let mut padded = b"x".to_vec();
        padded.resize(246, 0);
        let rig = Rig::open("", Vec::from(echoed(1, &write_multiple(0, &padded))));
        let fits = rig.write(vec![("fits", "holding-register:1:STRING(246)", text("x"))]);
        assert_eq!(fits.confirmed_count(), 1, "123 registers: {fits:?}");
        for address in [
            "holding-register:1:STRING(247)",
            "holding-register:1:STRING(250)",
            "holding-register:1:WSTRING(124)",
        ] {
            let response = rig.write(vec![("wide", address, text("x"))]);
            let error = rejection(&response, "wide");
            assert_eq!(error.kind(), ErrorKind::ResourceLimit, "{address}");
            assert!(
                error.to_string().contains("exceeds the maximum of 123"),
                "{address}: {error}"
            );
        }
        assert_eq!(rig.sent().len(), 1, "only STRING(246) was sent");
    }

    // ---- properties ----------------------------------------------------------------------

    /// Text addresses: an area, a number, a selection or none, a text type and, for a
    /// string, a length; wide enough that some pass one read and are refused.
    fn text_address() -> impl Strategy<Value = String> {
        (
            prop::sample::select(vec![
                "holding-register",
                "input-register",
                "extended-register",
            ]),
            1_u32..=1000,
            prop::option::of((0_u16..=8, 0_u16..=40)),
            prop::sample::select(vec!["CHAR", "WCHAR", "STRING", "WSTRING"]),
            1_u16..=60,
        )
            .prop_map(|(area, number, selection, name, length)| {
                let selection = selection
                    .map(|(lo, extra)| format!("[{lo}..{}]", lo.saturating_add(extra)))
                    .unwrap_or_default();
                let length = if name.ends_with("STRING") {
                    format!("({length})")
                } else {
                    String::new()
                };
                format!("{area}:{number}{selection}:{name}{length}")
            })
    }

    proptest! {
        // covers: MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING,
        // MB-ADDR-SELECTION / every text address that parses renders a canonical text that
        // reads back equal, and is never wider than one read
        #[test]
        fn prop_text_canonical_round_trips(text in text_address()) {
            if let Ok(address) = ModbusAddress::parse(&text, &none()) {
                let again = ModbusAddress::parse(&address.canonical(), &none());
                prop_assert_eq!(again.as_ref().ok(), Some(&address));
                prop_assert!(registers_read(&address.canonical()) <= 125);
            }
        }
    }
}
