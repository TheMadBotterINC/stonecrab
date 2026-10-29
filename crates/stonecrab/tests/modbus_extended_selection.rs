//! Array selections on the extended registers (ADR-0022, addendum of 2026-10-29: slices S48
//! and S50 together) through `ModbusTcpDriver` on the fake transport and the fake clock: a
//! range read inside one file and across the end of a file, an index, a list written across
//! the end of a file, the refusals before any I/O, and the limits at and past each bound.
//!
//! Two kinds of test share the file, as in `tests/modbus_extended.rs`. The strict-script tests
//! write the wire bytes out by hand from the MBAP layout and the V1.1b section 6.14 and 6.15
//! PDU layouts; no byte comes from the encoder under test, and a frame the script does not
//! expect is a mismatch. The device tests run the first-party responder of
//! `tests/support/responder.rs`, which serves function codes 20 and 21 from a sparse store, so
//! a list written can be read back and the store looked at. No interop peer serves function
//! codes 20 and 21 (pymodbus 3.8.6, libmodbus 3.1.11 and modbus-tk do not), so these tests are
//! a fake transport with a first-party device, not a second implementation. All timing is on
//! the fake clock (decision D21).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the module is shared with the contract tests; these tests arm no fault"
)]
mod responder;

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, InteropMap, Script, Step, block_on_with,
    };
    use stonecrab::{
        AddressOptions, Connection, Driver, ErrorKind, ItemError, PlcValue, ReadRequest,
        ReadResponse, Result, Tag, TagId, WriteOutcome, WriteRequest, WriteResponse,
    };

    use crate::responder::Device;

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

    fn list(values: impl IntoIterator<Item = PlcValue>) -> PlcValue {
        PlcValue::List(values.into_iter().collect())
    }

    /// `count` UINTs counting up from `first`.
    fn words(count: u16, first: u16) -> PlcValue {
        list((0..count).map(|n| PlcValue::U16(first.saturating_add(n))))
    }

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        driver: ModbusTcpDriver,
        options: AddressOptions,
        connection: Box<dyn Connection>,
    }

    impl Rig {
        /// A read-write connection of `ModbusTcpDriver` over `script`; `query` adds
        /// connection parameters.
        fn with(query: &str, script: Script) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            factory.push_script(script);
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

        /// Over one strict script: every frame is expected, in order.
        fn open(steps: Vec<Step>) -> Self {
            Self::with("", Script::strict(steps))
        }

        /// To the first-party device; `query` adds connection parameters.
        fn device(query: &str, device: &Device) -> Self {
            Self::with(query, Script::new(vec![Step::React(device.reactor())]))
        }

        fn tag(&self, id: &str, text: &str) -> Tag {
            let address = self
                .driver
                .parse_address(text, &self.options)
                .unwrap_or_else(|e| panic!("{text}: {e}"));
            Tag::new(TagId::new(id).expect("tag id"), address)
        }

        fn read(&self, addresses: &[(&str, &str)]) -> Result<ReadResponse> {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for (id, text) in addresses {
                builder = builder.tag(self.tag(id, text));
            }
            block_on_with(&self.clock, self.connection.read(builder.build()?))
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

    // ---------------------------------------------------------------------------- reads

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED / a range inside one file is one
    // sub-request of the file and record of its first element, and one list item
    #[test]
    fn an_extended_range_inside_a_file_is_one_sub_request_and_one_list() {
        // Address 30005 is record 5 of file 4; three INTs from there are records 5 to 7.
        let request = [0x14, 0x07, 0x06, 0x00, 0x04, 0x00, 0x05, 0x00, 0x03];
        let reply = [0x14, 0x08, 0x07, 0x06, 0x00, 0x01, 0xFF, 0xFF, 0x7F, 0xFF];
        let rig = Rig::open(Vec::from(exchange(1, &request, &reply)));
        let response = rig
            .read(&[("ints", "extended-register:30005[0..2]:INT")])
            .expect("read");
        assert_eq!(
            value(&response, "ints"),
            list([PlcValue::I16(1), PlcValue::I16(-1), PlcValue::I16(32_767)]),
            "one list of three INTs"
        );
        assert_eq!(response.split_into().get(), 1, "one request");
        assert_eq!(rig.sent(), vec![adu(1, &request)], "the request");
    }

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED, MB-QUIRK-B / a range across the end of a
    // file is one request of one sub-request per file, and its elements are read from the
    // registers of both sub-responses joined in order, so an element may straddle the files
    #[test]
    fn an_extended_range_across_a_file_end_joins_its_elements_in_order() {
        // Two DINTs from address 9999: record 9999 of file 1, then records 0 to 2 of file 2.
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x03,
        ];
        let reply = [
            0x14, 0x0C, 0x03, 0x06, 0x12, 0x34, 0x07, 0x06, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
        ];
        let rig = Rig::open(Vec::from(exchange(1, &request, &reply)));
        let response = rig
            .read(&[("pair", "extended-register:9999[0..1]:DINT")])
            .expect("read");
        assert_eq!(
            value(&response, "pair"),
            list([
                PlcValue::I32(0x1234_5678),
                PlcValue::I32(i32::from_be_bytes([0x9A, 0xBC, 0xDE, 0xF0])),
            ]),
            "the first DINT takes its high word from file 1 and its low word from file 2"
        );
        assert_eq!(response.split_into().get(), 1, "one request");
        assert_eq!(rig.sent(), vec![adu(1, &request)], "the request");
    }

    // covers: MB-READ-MERGE, MB-ADDR-SELECTION, MB-ADDR-EXTENDED / an extended list is one tag
    // to the batcher: it merges with the adjacent tags into one request across the end of a
    // file and yields one item
    #[test]
    fn an_extended_list_merges_with_adjacent_tags_across_a_file_end() {
        // Registers 9998 and 9999 of file 1 and records 0 and 1 of file 2: one request.
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0E, 0x00, 0x02, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x02,
        ];
        let reply = [
            0x14, 0x0C, 0x05, 0x06, 0x00, 0x01, 0x00, 0x02, 0x05, 0x06, 0x00, 0x03, 0x00, 0x04,
        ];
        let rig = Rig::open(Vec::from(exchange(1, &request, &reply)));
        let response = rig
            .read(&[
                ("one", "extended-register:10001[0..0]:UINT"),
                ("list", "extended-register:9998[0..1]:INT"),
                ("scalar", "extended-register:10000:INT"),
            ])
            .expect("read");
        assert_eq!(
            value(&response, "list"),
            list([PlcValue::I16(1), PlcValue::I16(2)]),
            "the list"
        );
        assert_eq!(value(&response, "scalar"), PlcValue::I16(3), "the scalar");
        assert_eq!(
            value(&response, "one"),
            list([PlcValue::U16(4)]),
            "a range of one is a list of one"
        );
        assert_eq!(response.split_into().get(), 1, "one request");
        assert_eq!(rig.sent(), vec![adu(1, &request)], "the request");
    }

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED / an index reads the scalar at the element's
    // own extended address, which may lie across the end of a file or in the next one
    #[test]
    fn an_extended_index_reads_the_element_at_its_own_address() {
        // `[1]` of the DINTs from 9997 is the DINT at 9999: cut at the end of file 1 exactly
        // as `extended-register:9999:DINT` is.
        let across = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let across_reply = [0x14, 0x08, 0x03, 0x06, 0x12, 0x34, 0x03, 0x06, 0x56, 0x78];
        // `[6;1]` of the DINTs from 9990 is element 5, address 10000: record 0 of file 2.
        let next = [0x14, 0x07, 0x06, 0x00, 0x02, 0x00, 0x00, 0x00, 0x02];
        let next_reply = [0x14, 0x06, 0x05, 0x06, 0xFF, 0xFF, 0xFF, 0xFE];
        let mut steps = Vec::new();
        steps.extend(exchange(1, &across, &across_reply));
        steps.extend(exchange(2, &across, &across_reply));
        steps.extend(exchange(3, &next, &next_reply));
        let rig = Rig::open(steps);
        for (id, text) in [
            ("indexed", "extended-register:9997[1]:DINT"),
            ("direct", "extended-register:9999:DINT"),
        ] {
            let response = rig.read(&[(id, text)]).expect(text);
            assert_eq!(value(&response, id), PlcValue::I32(0x1234_5678), "{text}");
        }
        let response = rig.read(&[("based", "6x09990[6;1]:DINT")]).expect("read");
        assert_eq!(value(&response, "based"), PlcValue::I32(-2), "element 5");
        assert_eq!(
            rig.sent(),
            vec![adu(1, &across), adu(2, &across), adu(3, &next)],
            "the index and the address send the same request"
        );
    }

    // covers: MB-EXC-02, MB-ADDR-SELECTION, MB-ADDR-EXTENDED / a device exception fails an
    // extended list as a whole
    #[test]
    fn an_exception_fails_the_whole_extended_list() {
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let rig = Rig::open(Vec::from(exchange(1, &request, &[0x94, 0x02])));
        let response = rig
            .read(&[("pair", "extended-register:9999[0..1]:INT")])
            .expect("a response");
        let error = response
            .get(&TagId::new("pair").expect("tag id"))
            .expect("the item")
            .outcome()
            .as_ref()
            .expect_err("an item error");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
    }

    // covers: MB-LIMIT-REGISTERS, MB-LIMIT-EXT, MB-ADDR-EXTENDED, MB-READ-MERGE / the widest
    // extended list, 120 registers, is one request even across the end of a file and at the
    // end of the last file; one wider, or past 655349999, never becomes a tag
    #[test]
    fn an_extended_list_fills_one_file_record_read_and_is_refused_past_it() {
        let device = Device::from_map(&register_map());
        let seed: Vec<u16> = (0..120).map(|n| 1_000 + n).collect();
        // 50 registers in file 1 and 70 in file 2; then the last 120 of file 65535.
        device.put_extended(9_950, &seed);
        device.put_extended(655_349_880, &seed);
        let rig = Rig::device("", &device);
        let expected = list(seed.iter().copied().map(PlcValue::U16));
        for text in [
            "extended-register:9950[0..119]:UINT",
            "extended-register:655349880[0..119]:UINT",
        ] {
            let before = device.reads();
            let response = rig.read(&[("wide", text)]).expect(text);
            assert_eq!(value(&response, "wide"), expected, "{text}");
            assert_eq!(response.split_into().get(), 1, "{text}: one chunk");
            assert_eq!(
                device.reads().saturating_sub(before),
                1,
                "{text}: one request"
            );
        }
        for text in [
            // 121 registers, and 125, upstream's limit.
            "extended-register:9950[0..120]:UINT",
            "extended-register:1[0..124]:INT",
            "extended-register:1[0..60]:DINT",
            // The last register at 655350000, and the first.
            "extended-register:655349881[0..119]:UINT",
            "extended-register:655349999[1]:UINT",
            // Address 0, moved by its selection.
            "extended-register:0[1]:UINT",
        ] {
            let error = rig
                .driver
                .parse_address(text, &rig.options)
                .expect_err(text);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{text}");
        }
        assert_eq!(device.reads(), 2, "only the two reads went out");
    }

    // covers: MB-LIMIT-REGISTERS, MB-ADDR-SELECTION, MB-ADDR-EXTENDED / an extended list wider
    // than the configured max-registers-per-request is ResourceLimit with nothing sent, and
    // one as wide is one request
    #[test]
    fn an_extended_list_wider_than_the_configured_bound_is_refused_before_any_io() {
        let device = Device::from_map(&register_map());
        let rig = Rig::device("&max-registers-per-request=10", &device);
        let error = rig
            .read(&[("wide", "extended-register:9995[0..10]:INT")])
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "eleven registers");
        assert!(error.to_string().contains("11 registers"), "{error}");
        assert_eq!(device.reads(), 0, "nothing was sent");
        let fits = rig
            .read(&[("fits", "extended-register:9995[0..9]:INT")])
            .expect("ten registers");
        assert_eq!(fits.split_into().get(), 1, "one chunk");
        assert_eq!(device.reads(), 1, "one request");
    }

    // --------------------------------------------------------------------------- writes

    // covers: MB-TCP-WRITE, MB-ADDR-SELECTION, MB-ADDR-EXTENDED / a list written across the
    // end of a file is one Write File Record request of one sub-request per file, confirmed
    // by its echo
    #[test]
    fn a_list_written_across_a_file_end_is_one_request_confirmed_by_its_echo() {
        // Two DINTs from address 9999: the first register in file 1, the other three in
        // file 2.
        let request = [
            0x15, 0x16, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x11, 0x11, 0x06, 0x00, 0x02,
            0x00, 0x00, 0x00, 0x03, 0x22, 0x22, 0x33, 0x33, 0x44, 0x44,
        ];
        let rig = Rig::open(Vec::from(exchange(1, &request, &request)));
        let response = rig.write(vec![(
            "pair",
            "extended-register:9999[0..1]:DINT",
            list([PlcValue::I32(0x1111_2222), PlcValue::I32(0x3333_4444)]),
        )]);
        assert!(
            matches!(outcome(&response, "pair"), WriteOutcome::Confirmed),
            "{response:?}"
        );
        assert_eq!(
            rig.sent(),
            vec![adu(1, &request)],
            "one request for the list"
        );
    }

    // covers: MB-TCP-WRITE, MB-ADDR-SELECTION, MB-ADDR-EXTENDED / a list written across the
    // end of a file lands in both files, touches nothing around it, and reads back whole
    #[test]
    fn a_list_written_across_a_file_end_lands_in_both_files_and_reads_back() {
        let device = Device::from_map(&register_map());
        let rig = Rig::device("", &device);
        let pair = list([PlcValue::I32(0x1111_2222), PlcValue::I32(-2)]);
        let response = rig.write(vec![(
            "pair",
            "extended-register:9999[0..1]:DINT",
            pair.clone(),
        )]);
        assert!(
            matches!(outcome(&response, "pair"), WriteOutcome::Confirmed),
            "{response:?}"
        );
        assert_eq!(device.applied(), 1, "one write file record");
        assert_eq!(
            [9_998, 9_999, 10_000, 10_001, 10_002, 10_003].map(|address| device.extended(address)),
            [0, 0x1111, 0x2222, 0xFFFF, 0xFFFE, 0],
            "the registers of both files, and none around them"
        );
        let read = rig
            .read(&[("pair", "extended-register:9999[0..1]:DINT")])
            .expect("read");
        assert_eq!(value(&read, "pair"), pair, "reads back");
        assert_eq!(device.reads(), 2, "one request each way");
    }

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED / a value that does not fit an extended
    // selection is refused before any I/O: a list of another length or with a mismatched
    // element is the item's rejection, a scalar to a range or a list to an index the request
    // builder's
    #[test]
    fn a_value_that_does_not_fit_an_extended_selection_is_refused_before_any_io() {
        let rig = Rig::open(vec![]);
        let response = rig.write(vec![
            (
                "short",
                "extended-register:9999[0..2]:INT",
                list([PlcValue::I16(1), PlcValue::I16(2)]),
            ),
            (
                "long",
                "extended-register:9999[0..0]:INT",
                list([PlcValue::I16(1), PlcValue::I16(2)]),
            ),
            (
                "mixed",
                "extended-register:20[0..1]:INT",
                list([PlcValue::I16(1), PlcValue::U16(2)]),
            ),
            (
                "nested",
                "extended-register:30[0..0]:INT",
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
            ("extended-register:9999[0..1]:DINT", PlcValue::I32(1)),
            ("extended-register:9997[1]:DINT", list([PlcValue::I32(1)])),
        ] {
            let tag = rig.tag("t", text);
            let refused = rig.write_request(vec![(tag, value)]);
            assert_eq!(
                refused.map(drop).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{text}: the request builder refuses the mismatch"
            );
        }
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: MB-TCP-WRITE, MB-LIMIT-REGISTERS, MB-ADDR-EXTENDED / one Write File Record
    // carries 251 data bytes, 7 per sub-request beside 2 per register: every extended list
    // fits inside one file (120 registers are 247 bytes) and up to 118 registers fit across
    // the end of a file (250 bytes); 119 or more across a file end read but are refused as a
    // write, with no request
    #[test]
    fn an_extended_list_write_is_refused_unsent_past_the_write_file_record_bound() {
        let device = Device::from_map(&register_map());
        let rig = Rig::device("", &device);
        let response = rig.write(vec![
            (
                "inside",
                "extended-register:1[0..119]:UINT",
                words(120, 1_000),
            ),
            (
                "across",
                "extended-register:19941[0..117]:UINT",
                words(118, 2_000),
            ),
            (
                "past",
                "extended-register:29941[0..118]:UINT",
                words(119, 3_000),
            ),
            (
                "dints",
                "extended-register:39941[0..59]:DINT",
                list((0..60).map(PlcValue::I32)),
            ),
        ]);
        for id in ["inside", "across"] {
            assert!(
                matches!(outcome(&response, id), WriteOutcome::Confirmed),
                "{id}: {response:?}"
            );
        }
        for id in ["past", "dints"] {
            assert_eq!(
                rejection(&response, id).kind(),
                ErrorKind::ResourceLimit,
                "{id}: wider than one write"
            );
        }
        assert_eq!(device.reads(), 2, "the refused lists sent nothing");
        assert_eq!(device.applied(), 2, "two writes applied");
        assert_eq!(
            [1, 120, 19_941, 19_999, 20_000, 20_058, 29_941]
                .map(|address| device.extended(address)),
            [1_000, 1_119, 2_000, 2_058, 2_059, 2_117, 0],
            "the two written lists, and nothing of the refused one"
        );
        let read = rig
            .read(&[("past", "extended-register:29941[0..118]:UINT")])
            .expect("a list too wide to write still reads");
        assert_eq!(
            value(&read, "past"),
            list(std::iter::repeat_n(PlcValue::U16(0), 119)),
            "never written"
        );
        assert_eq!(read.split_into().get(), 1, "one request");
    }
}
