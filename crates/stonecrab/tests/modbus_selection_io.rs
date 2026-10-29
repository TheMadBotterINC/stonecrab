//! Lists on the wire (slice S48, ADR-0022): array selections read from all four areas and
//! written to coils and holding registers through `ModbusTcpDriver` over the fake transport
//! and the fake clock.
//!
//! Every frame is written out by hand from the MBAP layout and the V1.1b PDU layouts; no
//! byte comes from the encoder under test, except the two upstream exchanges, which are
//! the `fixtures/modbus/tcp/adu_upstream-driver-suite-array-element-*` vectors the S04b
//! adapter serialized from `DriverTestsuite.xml` ("Array element read request" and "Array
//! element write request"). Scripts are strict: a frame the script does not expect is a
//! mismatch, and an empty script proves that nothing was sent.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::f32::consts::PI;
    use std::path::Path;
    use std::sync::Arc;

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, GoldenVector, Script, Step, block_on_with, load_dir,
    };
    use stonecrab::{
        AddressOptions, Connection, Driver, ErrorKind, ItemError, Operation, PlcValue, ReadRequest,
        ReadResponse, Result, Tag, TagId, WordOrder, WriteOutcome, WriteRequest, WriteResponse,
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

    /// The PDU of a function `function` request: start and quantity, then `tail`.
    fn pdu(function: u8, start: u16, quantity: u16, tail: &[u8]) -> Vec<u8> {
        let mut bytes = vec![function];
        bytes.extend_from_slice(&start.to_be_bytes());
        bytes.extend_from_slice(&quantity.to_be_bytes());
        bytes.extend_from_slice(tail);
        bytes
    }

    /// A register read reply carrying `words`.
    fn registers_reply(function: u8, words: &[u16]) -> Vec<u8> {
        let mut bytes = vec![
            function,
            u8::try_from(words.len().saturating_mul(2)).expect("at most 250 bytes"),
        ];
        bytes.extend(words.iter().flat_map(|word| word.to_be_bytes()));
        bytes
    }

    fn list(values: impl IntoIterator<Item = PlcValue>) -> PlcValue {
        PlcValue::List(values.into_iter().collect())
    }

    fn bools(bits: &[bool]) -> PlcValue {
        list(bits.iter().copied().map(PlcValue::Bool))
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

        fn read_tags(&self, tags: Vec<Tag>) -> Result<ReadResponse> {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            block_on_with(&self.clock, self.connection.read(builder.build()?))
        }

        fn read(&self, addresses: &[(&str, &str)]) -> Result<ReadResponse> {
            self.read_tags(
                addresses
                    .iter()
                    .map(|(id, text)| self.tag(id, text))
                    .collect(),
            )
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

    // ---- reads ---------------------------------------------------------------------------

    // covers: MB-ADDR-SELECTION, MB-TCP-READ-COIL, MB-TCP-READ-DI, MB-TCP-READ-HR,
    // MB-TCP-READ-IR / a range reads one list item from each area, an index one scalar
    #[test]
    fn a_range_reads_one_list_item_from_each_area_and_an_index_reads_its_element() {
        let mut steps = Vec::new();
        // Coils 1 to 10: true, false, true, true, false, true, false, false, true, false.
        steps.extend(exchange(
            1,
            &pdu(0x01, 0, 10, &[]),
            &[0x01, 0x02, 0x2D, 0x01],
        ));
        // Discrete inputs 5 to 7: false, true, true.
        steps.extend(exchange(2, &pdu(0x02, 4, 3, &[]), &[0x02, 0x01, 0b110]));
        // Holding registers 1 to 4: two REALs, 0x40490FDB each, the f32 nearest to pi.
        steps.extend(exchange(
            3,
            &pdu(0x03, 0, 4, &[]),
            &registers_reply(0x03, &[0x4049, 0x0FDB, 0x4049, 0x0FDB]),
        ));
        // Input registers 3 to 6, declared from 1: two DINTs, -2 and 7.
        steps.extend(exchange(
            4,
            &pdu(0x04, 2, 4, &[]),
            &registers_reply(0x04, &[0xFFFF, 0xFFFE, 0x0000, 0x0007]),
        ));
        // The third DINT from holding register 1: registers 5 and 6, 65536.
        steps.extend(exchange(
            5,
            &pdu(0x03, 4, 2, &[]),
            &registers_reply(0x03, &[0x0001, 0x0000]),
        ));
        let rig = Rig::open("", steps);
        for (id, text, expected) in [
            (
                "coils",
                "coil:1[0..9]",
                bools(&[
                    true, false, true, true, false, true, false, false, true, false,
                ]),
            ),
            (
                "inputs",
                "discrete-input:5[0..2]",
                bools(&[false, true, true]),
            ),
            (
                "reals",
                "holding-register:1[0..1]:REAL",
                list([PlcValue::F32(PI), PlcValue::F32(PI)]),
            ),
            (
                "dints",
                "input-register:3[1..2;1]:DINT",
                list([PlcValue::I32(-2), PlcValue::I32(7)]),
            ),
            ("third", "holding-register:1[2]:DINT", PlcValue::I32(65_536)),
        ] {
            let response = rig.read(&[(id, text)]).expect("a read");
            assert_eq!(response.items().len(), 1, "{text}: one item");
            assert_eq!(value(&response, id), expected, "{text}");
        }
        assert_eq!(rig.sent().len(), 5, "one request per read");
    }

    // covers: MB-ADDR-SELECTION / a list of one is a list, not its element
    #[test]
    fn a_one_element_range_reads_a_list_of_one() {
        let mut steps = Vec::new();
        steps.extend(exchange(
            1,
            &pdu(0x03, 4, 1, &[]),
            &registers_reply(0x03, &[0xFFF9]),
        ));
        steps.extend(exchange(
            2,
            &pdu(0x03, 4, 1, &[]),
            &registers_reply(0x03, &[0xFFF9]),
        ));
        let rig = Rig::open("", steps);
        let one = rig
            .read(&[("one", "holding-register:1[4..4]:INT")])
            .expect("a read");
        assert_eq!(
            value(&one, "one"),
            list([PlcValue::I16(-7)]),
            "a list of one"
        );
        let alone = rig
            .read(&[("alone", "holding-register:1[4]:INT")])
            .expect("a read");
        assert_eq!(
            value(&alone, "alone"),
            PlcValue::I16(-7),
            "the element alone"
        );
    }

    // covers: MB-BYTEORDER-PRECEDENCE, MB-BYTEORDER-LE-SWAP, MB-BYTEORDER-LE / byte and
    // word order apply to each element, from the tag, then the address, then the default
    #[test]
    fn byte_and_word_order_apply_to_each_element() {
        let mut steps = Vec::new();
        let swapped = [0x0304, 0x0102, 0x0708, 0x0506];
        steps.extend(exchange(
            1,
            &pdu(0x03, 0, 4, &[]),
            &registers_reply(0x03, &swapped),
        ));
        steps.extend(exchange(
            2,
            &pdu(0x03, 0, 4, &[]),
            &registers_reply(0x03, &swapped),
        ));
        steps.extend(exchange(
            3,
            &pdu(0x04, 0, 2, &[]),
            &registers_reply(0x04, &[0x3412, 0x7856]),
        ));
        let rig = Rig::open("", steps);
        let expected = list([PlcValue::U32(0x0102_0304), PlcValue::U32(0x0506_0708)]);
        let block = rig
            .read(&[(
                "block",
                "holding-register:1[0..1]:UDINT{byte-order: 'LITTLE_ENDIAN_BYTE_SWAP'}",
            )])
            .expect("a read");
        assert_eq!(value(&block, "block"), expected, "the address block");
        let field = rig
            .tag("field", "holding-register:1[0..1]:UDINT")
            .with_word_order(WordOrder::LowWordFirst);
        let tagged = rig.read_tags(vec![field]).expect("a read");
        assert_eq!(value(&tagged, "field"), expected, "the tag field");
        let little = rig
            .read(&[(
                "little",
                "input-register:1[0..1]:INT{byte-order: 'LITTLE_ENDIAN'}",
            )])
            .expect("a read");
        assert_eq!(
            value(&little, "little"),
            list([PlcValue::I16(0x1234), PlcValue::I16(0x5678)]),
            "each register swapped on its own"
        );
    }

    // covers: MB-ADDR-SELECTION / an element narrower than a register has a register of
    // its own, as it does alone (the divergence from the upstream packing)
    #[test]
    fn eight_bit_and_bool_elements_take_one_register_each() {
        let mut steps = Vec::new();
        steps.extend(exchange(
            1,
            &pdu(0x03, 0, 3, &[]),
            &registers_reply(0x03, &[0xFFFE, 0x0007, 0x1280]),
        ));
        steps.extend(exchange(
            2,
            &pdu(0x04, 9, 2, &[]),
            &registers_reply(0x04, &[0xFF00, 0x0001]),
        ));
        let rig = Rig::open("", steps);
        let sints = rig
            .read(&[("sints", "holding-register:1[0..2]:SINT")])
            .expect("a read");
        assert_eq!(
            value(&sints, "sints"),
            list([PlcValue::I8(-2), PlcValue::I8(7), PlcValue::I8(-128)]),
            "the low byte of each register, the high byte padding"
        );
        let flags = rig
            .read(&[("flags", "input-register:10[0..1]:BOOL")])
            .expect("a read");
        assert_eq!(
            value(&flags, "flags"),
            bools(&[false, true]),
            "bit 0 of each register"
        );
    }

    // covers: MB-READ-MERGE, MB-LIMIT-REGISTERS / a selection is one tag to the batcher,
    // merged with its neighbours and counted whole against the request bound
    #[test]
    fn a_selection_is_one_tag_that_the_batcher_counts_whole() {
        let mut steps = Vec::new();
        steps.extend(exchange(
            1,
            &pdu(0x03, 0, 6, &[]),
            &registers_reply(0x03, &[1, 2, 3, 4, 5, 6]),
        ));
        let words: Vec<u16> = (1..=125).collect();
        steps.extend(exchange(
            2,
            &pdu(0x03, 0, 125, &[]),
            &registers_reply(0x03, &words),
        ));
        steps.extend(exchange(
            3,
            &pdu(0x03, 125, 1, &[]),
            &registers_reply(0x03, &[126]),
        ));
        let rig = Rig::open("", steps);
        let merged = rig
            .read(&[
                ("a", "holding-register:1[0..2]:INT"),
                ("b", "holding-register:4:INT"),
                ("c", "holding-register:5[0..1]:INT"),
            ])
            .expect("a read");
        assert_eq!(merged.split_into().get(), 1, "one request for three tags");
        assert_eq!(
            value(&merged, "a"),
            list([PlcValue::I16(1), PlcValue::I16(2), PlcValue::I16(3)])
        );
        assert_eq!(value(&merged, "b"), PlcValue::I16(4));
        assert_eq!(
            value(&merged, "c"),
            list([PlcValue::I16(5), PlcValue::I16(6)])
        );
        let split = rig
            .read(&[
                ("big", "holding-register:1[0..124]:UINT"),
                ("next", "holding-register:126:UINT"),
            ])
            .expect("a read");
        assert_eq!(
            split.split_into().get(),
            2,
            "125 registers fill a request, the next one starts another"
        );
        assert_eq!(
            value(&split, "big"),
            list(words.iter().copied().map(PlcValue::U16))
        );
        assert_eq!(value(&split, "next"), PlcValue::U16(126));
    }

    // covers: MB-LIMIT-REGISTERS, MB-LIMIT-COILS, MB-CFG-MAX-REGISTERS, MB-CFG-MAX-COILS / a list wider than
    // the configured bound is a resource limit, with nothing sent
    #[test]
    fn a_list_wider_than_the_configured_bound_is_refused_before_any_io() {
        let rig = Rig::open(
            "&max-registers-per-request=10&max-coils-per-request=8",
            vec![],
        );
        for (text, unit) in [
            ("holding-register:1[0..10]:INT", "11 registers"),
            ("input-register:1[0..2]:LREAL", "12 registers"),
            ("coil:1[0..8]", "9 bits"),
        ] {
            let error = rig.read(&[("wide", text)]).expect_err("refused");
            assert_eq!(error.kind(), ErrorKind::ResourceLimit, "{text}");
            assert!(error.to_string().contains(unit), "{text}: {error}");
        }
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: MB-EXC-02 / a device exception fails the list item as a whole
    #[test]
    fn an_exception_fails_the_whole_list() {
        let steps = Vec::from(exchange(1, &pdu(0x03, 0, 4, &[]), &[0x83, 0x02]));
        let rig = Rig::open("", steps);
        let response = rig
            .read(&[("reals", "holding-register:1[0..1]:REAL")])
            .expect("a response");
        let item = response
            .get(&TagId::new("reals").expect("tag id"))
            .expect("the item");
        let error = item.outcome().as_ref().expect_err("an item error");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
    }

    // covers: MB-TCP-PING, MB-ADDR-SELECTION / a range as the ping address is read whole,
    // as upstream's onPing reads the tag's element count (Conn:402-412)
    #[test]
    fn a_ping_address_with_a_range_reads_the_whole_range() {
        // Four DINTs from holding register 10 (wire offset 9) are eight registers; the
        // reply is not decoded, so its values do not matter.
        let request = pdu(0x03, 9, 8, &[]);
        let reply = registers_reply(0x03, &[0; 8]);
        let rig = Rig::open(
            "&ping-address=holding-register:10[0..3]:DINT",
            Vec::from(exchange(1, &request, &reply)),
        );
        block_on_with(&rig.clock, rig.connection.ping()).expect("the ping is answered");
        assert_eq!(
            rig.sent(),
            vec![adu(1, &request)],
            "one read of eight registers"
        );
    }

    /// The upstream vector of `case` and `direction`.
    fn upstream(case: &str, direction: &str) -> GoldenVector {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let name = format!("tcp/adu_upstream-driver-suite-array-element-{case}-ex1-{direction}");
        load_dir(&dir)
            .expect("fixtures/modbus loads")
            .into_iter()
            .find(|vector| vector.name == name)
            .unwrap_or_else(|| panic!("the vector {name}"))
    }

    // covers: MB-ADDR-SELECTION, MB-TCP-READ-HR / the "Array element read request" of the
    // upstream DriverTestsuite.xml: the same request, and the list the XML reports
    #[test]
    fn the_upstream_array_element_read_is_one_request_and_a_list_of_two_reals() {
        let request = upstream("read-request", "request");
        let reply = upstream("read-request", "response");
        let rig = Rig::open(
            "",
            vec![
                Step::Expect(request.bytes.clone()),
                Step::Reply(reply.bytes.clone()),
            ],
        );
        let response = rig
            .read(&[("hurz", "holding-register:1[0..1]:REAL")])
            .expect("a read");
        let reported: f32 = "3.1415927".parse().expect("the XML value");
        assert_eq!(
            value(&response, "hurz"),
            list([PlcValue::F32(reported), PlcValue::F32(reported)]),
            "the PlcList of two PlcREAL the suite reports"
        );
        assert_eq!(
            rig.sent(),
            vec![request.bytes],
            "the upstream request, once"
        );
    }

    // ---- writes --------------------------------------------------------------------------

    // covers: MB-TCP-WRITE, MB-ADDR-SELECTION / a list goes out as FC15 to coils and FC16 to
    // registers, FC06 only for a one-register payload, each element laid out as alone
    #[test]
    fn a_list_goes_out_as_fc15_for_coils_and_fc16_for_registers() {
        let mut steps = Vec::new();
        steps.extend(echoed(1, &pdu(0x0F, 0, 10, &[0x02, 0x2D, 0x01])));
        steps.extend(echoed(
            2,
            &pdu(
                0x10,
                0,
                4,
                &[0x08, 0x40, 0x49, 0x0F, 0xDB, 0x40, 0x49, 0x0F, 0xDB],
            ),
        ));
        steps.extend(echoed(3, &[0x06, 0x00, 0x04, 0xFF, 0xF9]));
        steps.extend(echoed(
            4,
            &pdu(0x10, 2, 3, &[0x06, 0x00, 0xFE, 0x00, 0x07, 0x00, 0x80]),
        ));
        steps.extend(echoed(
            5,
            &pdu(
                0x10,
                0,
                4,
                &[0x08, 0x03, 0x04, 0x01, 0x02, 0x07, 0x08, 0x05, 0x06],
            ),
        ));
        steps.extend(echoed(6, &pdu(0x0F, 7, 1, &[0x01, 0x01])));
        let rig = Rig::open("", steps);
        let response = rig.write(vec![
            (
                "coils",
                "coil:1[0..9]",
                bools(&[
                    true, false, true, true, false, true, false, false, true, false,
                ]),
            ),
            (
                "reals",
                "holding-register:1[0..1]:REAL",
                list([PlcValue::F32(PI), PlcValue::F32(PI)]),
            ),
            (
                "one",
                "holding-register:1[4..4]:INT",
                list([PlcValue::I16(-7)]),
            ),
            (
                "sints",
                "holding-register:3[0..2]:SINT",
                list([PlcValue::I8(-2), PlcValue::I8(7), PlcValue::I8(-128)]),
            ),
            (
                "swapped",
                "holding-register:1[0..1]:UDINT{byte-order: 'LITTLE_ENDIAN_BYTE_SWAP'}",
                list([PlcValue::U32(0x0102_0304), PlcValue::U32(0x0506_0708)]),
            ),
            ("lone", "coil:8[0..0]", bools(&[true])),
        ]);
        assert_eq!(response.confirmed_count(), 6, "{response:?}");
        assert_eq!(rig.sent().len(), 6, "one frame per item");
    }

    // covers: MB-ADDR-SELECTION / a value that does not match the selection is refused
    // before any I/O, with the kind the write validation uses for a mismatched value
    #[test]
    fn a_value_that_does_not_match_the_selection_is_refused_before_any_io() {
        let rig = Rig::open("", vec![]);
        let response = rig.write(vec![
            (
                "short",
                "holding-register:1[0..2]:INT",
                list([PlcValue::I16(1), PlcValue::I16(2)]),
            ),
            ("long", "coil:1[0..1]", bools(&[true, false, true])),
            (
                "mixed",
                "holding-register:10[0..1]:INT",
                list([PlcValue::I16(1), PlcValue::U16(2)]),
            ),
            (
                "nested",
                "holding-register:20[0..0]:INT",
                list([list([PlcValue::I16(1)])]),
            ),
        ]);
        for id in ["short", "long", "mixed", "nested"] {
            assert_eq!(
                rejection(&response, id).kind(),
                ErrorKind::InvalidConfig,
                "{id}"
            );
        }
        for (text, value) in [
            ("holding-register:1[0..1]:INT", PlcValue::I16(1)),
            ("holding-register:1[1]:INT", list([PlcValue::I16(1)])),
            ("coil:1", bools(&[true])),
        ] {
            let tag = rig.tag("t", text);
            let refused = rig.write_request(vec![(tag, value)]);
            assert_eq!(
                refused.map(drop).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{text}: the request builder refuses the mismatch"
            );
        }
        let read_only = rig.write(vec![(
            "inputs",
            "input-register:1[0..1]:INT",
            list([PlcValue::I16(1), PlcValue::I16(2)]),
        )]);
        assert_eq!(
            rejection(&read_only, "inputs").kind(),
            ErrorKind::Unsupported(Operation::Write),
            "an input register list is read-only"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: MB-LIMIT-REGISTERS, MB-LIMIT-COILS / one write carries at most 123 registers
    // or 1968 coils: a wider list reads but is refused as a write, with no request
    #[test]
    fn a_list_wider_than_one_write_is_refused_and_the_widest_that_fits_goes_out() {
        let words: Vec<u16> = (0..123).collect();
        let mut registers = vec![246_u8];
        registers.extend(words.iter().flat_map(|word| word.to_be_bytes()));
        let mut coils = vec![246_u8];
        coils.extend(std::iter::repeat_n(0xFF, 246));
        let mut steps = Vec::new();
        steps.extend(echoed(1, &pdu(0x10, 0, 123, &registers)));
        steps.extend(echoed(2, &pdu(0x0F, 0, 1_968, &coils)));
        let rig = Rig::open("", steps);
        let response = rig.write(vec![
            (
                "fits",
                "holding-register:1[0..122]:INT",
                list(
                    words
                        .iter()
                        .map(|word| PlcValue::I16(i16::try_from(*word).expect("small"))),
                ),
            ),
            (
                "registers",
                "holding-register:1[0..123]:INT",
                list((0..124).map(PlcValue::I16)),
            ),
            (
                "coils_fit",
                "coil:1[0..1967]",
                list(std::iter::repeat_n(PlcValue::Bool(true), 1_968)),
            ),
            (
                "coils",
                "coil:1[0..1968]",
                list(std::iter::repeat_n(PlcValue::Bool(true), 1_969)),
            ),
        ]);
        assert!(matches!(
            outcome(&response, "fits"),
            WriteOutcome::Confirmed
        ));
        assert!(matches!(
            outcome(&response, "coils_fit"),
            WriteOutcome::Confirmed
        ));
        for id in ["registers", "coils"] {
            assert_eq!(
                rejection(&response, id).kind(),
                ErrorKind::ResourceLimit,
                "{id}"
            );
        }
        assert_eq!(rig.sent().len(), 2, "only the two that fit went out");
    }
}
