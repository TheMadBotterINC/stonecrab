//! Extended registers through `ModbusConnection` on the fake transport and the fake clock
//! (slice S50): the file record requests (function codes 20 and 21) a tag goes out under, the
//! cut at the end of each 10000-register file, the first and last addressable registers, the
//! batching of adjacent tags, the device exceptions and the outcomes of a write.
//!
//! Two kinds of test share the file. The strict-script tests write the wire bytes out by hand
//! from the MBAP layout, the V1.1b section 6.14 and 6.15 PDU layouts and the file and record
//! arithmetic of `fixtures/modbus/tcp/adu_upstream-*-extended-registers-*` (the frames
//! upstream builds for a tag cut at a file end); no byte comes from the encoder under test.
//! The device tests run the first-party responder of `tests/support/responder.rs`, which serves
//! the file record codes from a sparse store, so a value written can be read back.
//!
//! No interop peer serves function codes 20 and 21 (pymodbus 3.8.6, libmodbus 3.1.11 and
//! modbus-tk do not), so these tests are the evidence for the rows they cover: a fake
//! transport with a first-party device, not a second implementation. All timing is on the
//! fake clock (decision D21).
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
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, InteropMap, Script, Step, block_on_with,
    };
    use stonecrab::{
        AccessPolicy, AddressOptions, Clock, Connection, ConnectionConfig, Endpoint, ErrorKind,
        Operation, PlcValue, ReadRequest, ReadResponse, Result, Tag, TagId, Timeouts,
        UnknownReason, WriteOutcome, WriteRequest, WriteResponse,
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

    fn tag(id: &str, address: &str) -> Tag {
        let address = ModbusAddress::parse(address, &AddressOptions::default()).expect("address");
        Tag::new(TagId::new(id).expect("tag id"), Arc::new(address))
    }

    fn config(access: AccessPolicy) -> ConnectionConfig {
        ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
            .access(access)
            .build()
            .expect("config")
    }

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        /// A read-write connection over one strict script.
        fn open(steps: Vec<Step>) -> Self {
            Self::with(config(AccessPolicy::ReadWrite), Script::strict(steps))
        }

        /// A read-write connection to a first-party device.
        fn device(device: &Device) -> Self {
            Self::with(
                config(AccessPolicy::ReadWrite),
                Script::new(vec![Step::React(device.reactor())]),
            )
        }

        fn with(config: ConnectionConfig, script: Script) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            factory.push_script(script);
            let t0 = clock.now();
            let connection = block_on_with(
                &clock,
                ModbusConnection::open(
                    config,
                    Arc::new(factory.clone()),
                    Arc::new(clock.clone()),
                    ConnectionDefaults::default(),
                ),
            )
            .expect("open");
            Self {
                clock,
                factory,
                connection,
                t0,
            }
        }

        fn read(&self, tags: &[(&str, &str)]) -> Result<ReadResponse> {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for (id, address) in tags {
                builder = builder.tag(tag(id, address));
            }
            let request = builder.build()?;
            block_on_with(&self.clock, self.connection.read(request))
        }

        fn write(&self, items: Vec<(Tag, PlcValue)>) -> Result<WriteResponse> {
            let mut builder = WriteRequest::builder(self.connection.limits());
            for (tag, value) in items {
                builder = builder.item(tag, value);
            }
            let request = builder.build()?;
            block_on_with(&self.clock, self.connection.write(request))
        }

        /// The frames the first transport received.
        fn sent(&self) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .first()
                .expect("a transport was opened")
                .sent()
                .iter()
                .map(|frame| frame.to_vec())
                .collect()
        }

        fn elapsed(&self) -> Duration {
            self.clock.now().saturating_duration_since(self.t0)
        }
    }

    fn value(response: &ReadResponse, id: &str) -> PlcValue {
        response
            .get(&TagId::new(id).expect("tag id"))
            .expect("the item")
            .outcome()
            .as_ref()
            .expect("a value")
            .clone()
    }

    fn failure<'a>(response: &'a ReadResponse, id: &str) -> &'a stonecrab::ItemError {
        response
            .get(&TagId::new(id).expect("tag id"))
            .expect("the item")
            .outcome()
            .as_ref()
            .expect_err("an item error")
    }

    fn outcome<'a>(response: &'a WriteResponse, id: &str) -> &'a WriteOutcome {
        response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("a result for {id}"))
            .outcome()
    }

    fn rejection<'a>(response: &'a WriteResponse, id: &str) -> &'a stonecrab::ItemError {
        let found = outcome(response, id);
        let WriteOutcome::Rejected(error) = found else {
            panic!("{id} was expected to be rejected, got {found:?}");
        };
        error
    }

    // ---------------------------------------------------------------------------- reads

    // covers: MB-ADDR-EXTENDED, MB-TCP-READ-HR / a span inside a file is one sub-request of
    // the file and record the address names
    #[test]
    fn a_read_inside_a_file_is_one_sub_request() {
        // Address 5 is file 1, record 5; the reply carries 42.
        let request = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x05, 0x00, 0x01];
        let reply = [0x14, 0x04, 0x03, 0x06, 0x00, 0x2A];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request)),
            Step::Reply(adu(1, &reply)),
        ]);
        let response = rig.read(&[("a", "extended-register:5:INT")]).expect("read");
        assert_eq!(value(&response, "a"), PlcValue::I16(42));
        assert_eq!(response.split_into().get(), 1);
        assert_eq!(rig.sent(), vec![adu(1, &request)], "one request");
    }

    // The frames are those of the upstream parser-suite vectors "Read Extended Registers
    // Request/Response Split File Record" (a tag of two registers cut at a file end), which
    // upstream's connection answers with the first sub-response only (MB-QUIRK-B).
    // covers: MB-ADDR-EXTENDED, MB-QUIRK-B / a span across a file end is one request of two
    // sub-requests, and the data of both comes back, in order
    #[test]
    fn a_read_across_a_file_end_is_cut_and_joined_in_order() {
        // Address 9999 is file 1 record 9999; its second register is file 2 record 0.
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let reply = [0x14, 0x08, 0x03, 0x06, 0x12, 0x34, 0x03, 0x06, 0x56, 0x78];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request)),
            Step::Reply(adu(1, &reply)),
        ]);
        let response = rig
            .read(&[("d", "extended-register:9999:DINT")])
            .expect("read");
        assert_eq!(
            value(&response, "d"),
            PlcValue::I32(0x1234_5678),
            "the high word from file 1, the low word from file 2"
        );
        assert_eq!(rig.sent().len(), 1, "one request");
    }

    // covers: MB-ADDR-EXTENDED / a span that ends exactly at the end of a file needs no
    // second sub-request
    #[test]
    fn a_read_that_ends_at_the_end_of_a_file_is_one_sub_request() {
        let dint = [0x14, 0x07, 0x06, 0x00, 0x01, 0x27, 0x0E, 0x00, 0x02];
        let int = [0x14, 0x07, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &dint)),
            Step::Reply(adu(1, &[0x14, 0x06, 0x05, 0x06, 0x00, 0x01, 0x00, 0x02])),
            Step::Expect(adu(2, &int)),
            Step::Reply(adu(2, &[0x14, 0x04, 0x03, 0x06, 0xFF, 0xD6])),
        ]);
        let first = rig
            .read(&[("d", "extended-register:9998:DINT")])
            .expect("read");
        assert_eq!(value(&first, "d"), PlcValue::I32(0x0001_0002));
        let last = rig
            .read(&[("i", "extended-register:9999:INT")])
            .expect("read");
        assert_eq!(value(&last, "i"), PlcValue::I16(-42));
        assert_eq!(rig.sent(), vec![adu(1, &dint), adu(2, &int)]);
    }

    // covers: MB-ADDR-EXTENDED, MB-LIMIT-EXT / the first addressable register is address 1
    // (record 1 of file 1), the last is record 9999 of file 65535
    #[test]
    fn the_first_and_the_last_addressable_registers_are_read() {
        let first = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01];
        let start_of_last_file = [0x14, 0x07, 0x06, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x01];
        let last = [0x14, 0x07, 0x06, 0xFF, 0xFF, 0x27, 0x0F, 0x00, 0x01];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &first)),
            Step::Reply(adu(1, &[0x14, 0x04, 0x03, 0x06, 0x00, 0x01])),
            Step::Expect(adu(2, &start_of_last_file)),
            Step::Reply(adu(2, &[0x14, 0x04, 0x03, 0x06, 0x00, 0x02])),
            Step::Expect(adu(3, &last)),
            Step::Reply(adu(3, &[0x14, 0x04, 0x03, 0x06, 0x00, 0x03])),
        ]);
        for (address, expected) in [
            ("extended-register:1", 1),
            ("extended-register:655340000", 2),
            ("extended-register:655349999", 3),
        ] {
            let response = rig.read(&[("r", address)]).expect(address);
            assert_eq!(value(&response, "r"), PlcValue::I16(expected), "{address}");
        }
        assert_eq!(rig.sent().len(), 3);
    }

    // covers: MB-ADDR-EXTENDED / an address that cannot be sent never becomes a tag
    #[test]
    fn logical_address_0_and_the_end_of_the_files_are_refused_before_a_tag_exists() {
        for text in [
            "extended-register:0",
            "6x00000",
            "extended-register:655350000",
            "extended-register:655349999:DINT",
        ] {
            let error = ModbusAddress::parse(text, &AddressOptions::default())
                .expect_err("an address nobody can send");
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{text}");
        }
    }

    // covers: MB-READ-MERGE, MB-ADDR-EXTENDED / adjacent extended tags are one request, a gap
    // starts another
    #[test]
    fn adjacent_extended_tags_share_a_request_and_a_gap_starts_another() {
        // Registers 10 to 13 in one sub-request; register 20 alone.
        let block = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x0A, 0x00, 0x04];
        let alone = [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x14, 0x00, 0x01];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &block)),
            Step::Reply(adu(
                1,
                &[
                    0x14, 0x0A, 0x09, 0x06, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03, 0x00, 0x04,
                ],
            )),
            Step::Expect(adu(2, &alone)),
            Step::Reply(adu(2, &[0x14, 0x04, 0x03, 0x06, 0x00, 0x09])),
        ]);
        let response = rig
            .read(&[
                ("z", "extended-register:20:INT"),
                ("c", "extended-register:12:DINT"),
                ("b", "6x00011"),
                ("a", "extended-register:10"),
            ])
            .expect("read");
        assert_eq!(value(&response, "a"), PlcValue::I16(1));
        assert_eq!(value(&response, "b"), PlcValue::I16(2));
        assert_eq!(value(&response, "c"), PlcValue::I32(0x0003_0004));
        assert_eq!(value(&response, "z"), PlcValue::I16(9));
        assert_eq!(response.split_into().get(), 2, "two wire requests");
        assert_eq!(rig.sent(), vec![adu(1, &block), adu(2, &alone)]);
    }

    // covers: MB-READ-MERGE, MB-ADDR-EXTENDED / a block that crosses a file end is one request
    // of two sub-requests, and each tag gets its own register
    #[test]
    fn a_block_of_tags_across_a_file_end_is_one_request() {
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let reply = [0x14, 0x08, 0x03, 0x06, 0x00, 0x0A, 0x03, 0x06, 0x00, 0x0B];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request)),
            Step::Reply(adu(1, &reply)),
        ]);
        let response = rig
            .read(&[
                ("last", "extended-register:9999"),
                ("first", "extended-register:10000"),
            ])
            .expect("read");
        assert_eq!(value(&response, "last"), PlcValue::I16(10));
        assert_eq!(value(&response, "first"), PlcValue::I16(11));
        assert_eq!(response.split_into().get(), 1);
    }

    // covers: MB-EXC-01, MB-EXC-02, MB-EXC-06, MB-ADDR-EXTENDED / a device exception answers
    // its whole chunk as for the other function codes and the next chunk still runs
    #[test]
    fn a_device_exception_fails_its_chunk_and_the_next_chunk_still_runs() {
        let at = |record: u8| [0x14, 0x07, 0x06, 0x00, 0x01, 0x00, record, 0x00, 0x01];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &at(5))),
            Step::Reply(adu(1, &[0x94, 0x02])),
            Step::Expect(adu(2, &at(20))),
            Step::Reply(adu(2, &[0x94, 0x06])),
            Step::Expect(adu(3, &at(40))),
            Step::Reply(adu(3, &[0x94, 0x01])),
            Step::Expect(adu(4, &at(60))),
            Step::Reply(adu(4, &[0x14, 0x04, 0x03, 0x06, 0x00, 0x07])),
        ]);
        let response = rig
            .read(&[
                ("illegal-address", "extended-register:5"),
                ("busy", "extended-register:20"),
                ("illegal-function", "extended-register:40"),
                ("fine", "extended-register:60"),
            ])
            .expect("exceptions are item errors");
        let refused = failure(&response, "illegal-address");
        assert_eq!(refused.kind(), ErrorKind::InvalidAddress);
        assert_eq!(
            refused.status().map(stonecrab::ProtocolStatus::code),
            Some(2),
            "the exception code is the protocol status"
        );
        let busy = failure(&response, "busy");
        assert_eq!(busy.kind(), ErrorKind::DeviceException);
        assert_eq!(busy.status().map(stonecrab::ProtocolStatus::code), Some(6));
        assert_eq!(
            failure(&response, "illegal-function").kind(),
            ErrorKind::Unsupported(Operation::Read)
        );
        assert_eq!(value(&response, "fine"), PlcValue::I16(7));
    }

    // covers: MB-READ-MATCH, MB-ADDR-EXTENDED / a reply that does not answer every sub-request
    // with the registers asked for is malformed, never a short value
    #[test]
    fn a_reply_that_does_not_answer_every_sub_request_is_malformed() {
        let request = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let replies: [(&str, &[u8]); 3] = [
            // The second sub-response is missing: what upstream's first-item rule hides.
            ("one sub-response", &[0x14, 0x04, 0x03, 0x06, 0x12, 0x34]),
            (
                "a second sub-response of two registers",
                &[
                    0x14, 0x0A, 0x03, 0x06, 0x12, 0x34, 0x05, 0x06, 0x56, 0x78, 0x00, 0x00,
                ],
            ),
            (
                "three sub-responses",
                &[
                    0x14, 0x0C, 0x03, 0x06, 0x12, 0x34, 0x03, 0x06, 0x56, 0x78, 0x03, 0x06, 0x00,
                    0x00,
                ],
            ),
        ];
        for (name, reply) in replies {
            let rig = Rig::open(vec![
                Step::Expect(adu(1, &request)),
                Step::Reply(adu(1, reply)),
            ]);
            let error = rig
                .read(&[("d", "extended-register:9999:DINT")])
                .expect_err(name);
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{name}");
        }
    }

    // ---------------------------------------------------------------------------- writes

    // covers: MB-TCP-WRITE, MB-ADDR-EXTENDED / an extended register is written by function 21,
    // one request per item, confirmed by the echo
    #[test]
    fn a_write_inside_a_file_is_one_write_file_record() {
        let request = [
            0x15, 0x09, 0x06, 0x00, 0x01, 0x00, 0x05, 0x00, 0x01, 0x00, 0x2A,
        ];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request)),
            Step::Reply(adu(1, &request)),
        ]);
        let response = rig
            .write(vec![(
                tag("w", "extended-register:5:INT"),
                PlcValue::I16(42),
            )])
            .expect("write");
        assert!(matches!(outcome(&response, "w"), WriteOutcome::Confirmed));
        assert_eq!(rig.sent(), vec![adu(1, &request)]);
    }

    // The frame is that of the upstream parser-suite vector "Write Extended Registers Request
    // Split File Record": one request of two sub-requests, one register each.
    // covers: MB-TCP-WRITE, MB-ADDR-EXTENDED / a value across a file end is one request, so
    // one outcome
    #[test]
    fn a_write_across_a_file_end_is_one_request_of_two_sub_requests() {
        let request = [
            0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00, 0x02,
            0x00, 0x00, 0x00, 0x01, 0x56, 0x78,
        ];
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request)),
            Step::Reply(adu(1, &request)),
        ]);
        let response = rig
            .write(vec![(
                tag("w", "extended-register:9999:DINT"),
                PlcValue::I32(0x1234_5678),
            )])
            .expect("write");
        assert!(matches!(outcome(&response, "w"), WriteOutcome::Confirmed));
        assert_eq!(rig.sent().len(), 1, "one request for the one value");
    }

    // covers: MB-WRITE-ACCESS, MB-ADDR-EXTENDED / a read-only connection writes no extended
    // register
    #[test]
    fn a_read_only_connection_refuses_an_extended_write_before_anything_is_sent() {
        let rig = Rig::with(config(AccessPolicy::ReadOnly), Script::strict(Vec::new()));
        let error = rig
            .write(vec![(
                tag("w", "extended-register:5:INT"),
                PlcValue::I16(42),
            )])
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::Unsupported(Operation::Write));
        assert!(rig.sent().is_empty(), "nothing went out");
    }

    // covers: MB-WRITE-ECHO, MB-TCP-WRITE, MB-ADDR-EXTENDED / only an echo of the whole request
    // confirms a write; anything else is an unknown outcome, sent once, and halts the rest
    #[test]
    fn a_reply_that_is_not_the_echo_of_the_request_is_unknown() {
        let request = [
            0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00, 0x02,
            0x00, 0x00, 0x00, 0x01, 0x56, 0x78,
        ];
        let replies: [(&str, &[u8]); 6] = [
            (
                "another value",
                &[
                    0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00,
                    0x02, 0x00, 0x00, 0x00, 0x01, 0x56, 0x79,
                ],
            ),
            (
                "another file",
                &[
                    0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00,
                    0x03, 0x00, 0x00, 0x00, 0x01, 0x56, 0x78,
                ],
            ),
            (
                "another record",
                &[
                    0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0E, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00,
                    0x02, 0x00, 0x00, 0x00, 0x01, 0x56, 0x78,
                ],
            ),
            (
                "the first sub-request only",
                &[
                    0x15, 0x09, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34,
                ],
            ),
            ("another function", &[0x10, 0x27, 0x0F, 0x00, 0x02]),
            ("a truncated echo", &[0x15, 0x12, 0x06, 0x00]),
        ];
        for (name, reply) in replies {
            let rig = Rig::open(vec![
                Step::Expect(adu(1, &request)),
                Step::Reply(adu(1, reply)),
            ]);
            let response = rig
                .write(vec![
                    (
                        tag("w", "extended-register:9999:DINT"),
                        PlcValue::I32(0x1234_5678),
                    ),
                    (tag("next", "extended-register:5:INT"), PlcValue::I16(1)),
                ])
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                matches!(outcome(&response, "w"), WriteOutcome::Unknown(_)),
                "{name}: {response:?}"
            );
            assert_eq!(
                rejection(&response, "next").kind(),
                ErrorKind::Cancelled,
                "{name}: the write after an unknown one is not sent"
            );
            assert_eq!(rig.sent().len(), 1, "{name}: sent once, never replayed");
        }
    }

    // covers: MB-EXC-01, MB-EXC-02, MB-EXC-04, MB-TCP-WRITE, MB-ADDR-EXTENDED / a device
    // exception rejects the write with the code as its status
    #[test]
    fn a_device_exception_rejects_an_extended_write() {
        let request = |record: u8| {
            [
                0x15, 0x09, 0x06, 0x00, 0x01, 0x00, record, 0x00, 0x01, 0x00, 0x01,
            ]
        };
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &request(1))),
            Step::Reply(adu(1, &[0x95, 0x02])),
            Step::Expect(adu(2, &request(2))),
            Step::Reply(adu(2, &[0x95, 0x01])),
            Step::Expect(adu(3, &request(3))),
            Step::Reply(adu(3, &[0x95, 0x04])),
        ]);
        let response = rig
            .write(vec![
                (tag("address", "extended-register:1:INT"), PlcValue::I16(1)),
                (tag("function", "extended-register:2:INT"), PlcValue::I16(1)),
                (tag("failure", "extended-register:3:INT"), PlcValue::I16(1)),
            ])
            .expect("a refused write is an outcome");
        assert_eq!(response.rejected_count(), 3);
        for (id, kind, code) in [
            ("address", ErrorKind::InvalidAddress, 2),
            ("function", ErrorKind::Unsupported(Operation::Write), 1),
            ("failure", ErrorKind::DeviceException, 4),
        ] {
            let error = rejection(&response, id);
            assert_eq!(error.kind(), kind, "{id}");
            assert_eq!(
                error.status().map(stonecrab::ProtocolStatus::code),
                Some(code),
                "{id}"
            );
        }
    }

    // covers: MB-WRITE-UNKNOWN, MB-TCP-WRITE, MB-ADDR-EXTENDED / a write that went out and got no
    // answer is an unknown outcome and is not repeated
    #[test]
    fn an_extended_write_with_no_answer_is_unknown_and_is_not_replayed() {
        let request = [
            0x15, 0x09, 0x06, 0x00, 0x01, 0x00, 0x05, 0x00, 0x01, 0x00, 0x2A,
        ];
        let rig = Rig::open(vec![Step::Expect(adu(1, &request))]);
        let response = rig
            .write(vec![(
                tag("w", "extended-register:5:INT"),
                PlcValue::I16(42),
            )])
            .expect("an unanswered write is an outcome");
        assert!(
            matches!(
                outcome(&response, "w"),
                WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit)
            ),
            "{response:?}"
        );
        assert_eq!(rig.elapsed(), Timeouts::default().request);
        assert_eq!(rig.sent().len(), 1, "sent once");
    }

    // ------------------------------------------------------------------- the first-party device

    fn seeded() -> (Device, Rig) {
        let device = Device::from_map(&register_map());
        let rig = Rig::device(&device);
        (device, rig)
    }

    // covers: MB-TCP-WRITE, MB-ADDR-EXTENDED, MB-QUIRK-B / a value written across a file end
    // lands in both files and reads back whole
    #[test]
    fn a_value_written_across_a_file_end_reads_back_whole() {
        let (device, rig) = seeded();
        let response = rig
            .write(vec![(
                tag("w", "extended-register:9999:DINT"),
                PlcValue::I32(0x1234_5678),
            )])
            .expect("write");
        assert!(matches!(outcome(&response, "w"), WriteOutcome::Confirmed));
        assert_eq!(device.applied(), 1, "one write file record");
        assert_eq!(
            device.extended(9_998),
            0,
            "the register before is untouched"
        );
        assert_eq!(device.extended(9_999), 0x1234, "the last record of file 1");
        assert_eq!(device.extended(10_000), 0x5678, "record 0 of file 2");
        assert_eq!(
            device.extended(10_001),
            0,
            "the register after is untouched"
        );
        let read = rig
            .read(&[("r", "extended-register:9999:DINT")])
            .expect("read");
        assert_eq!(value(&read, "r"), PlcValue::I32(0x1234_5678));
        assert_eq!(rig.sent().len(), 2, "one request each way");
    }

    // covers: MB-TCP-WRITE, MB-ADDR-EXTENDED, MB-LIMIT-EXT / the first and last addressable
    // registers are written and read back
    #[test]
    fn the_first_and_the_last_registers_are_written_and_read_back() {
        let (device, rig) = seeded();
        let response = rig
            .write(vec![
                (tag("first", "extended-register:1:UINT"), PlcValue::U16(11)),
                (
                    tag("last", "extended-register:655349999:UINT"),
                    PlcValue::U16(22),
                ),
            ])
            .expect("write");
        assert_eq!(response.confirmed_count(), 2);
        assert_eq!(device.extended(1), 11);
        assert_eq!(device.extended(655_349_999), 22);
        let read = rig
            .read(&[
                ("first", "extended-register:1:UINT"),
                ("last", "extended-register:655349999:UINT"),
            ])
            .expect("read");
        assert_eq!(value(&read, "first"), PlcValue::U16(11));
        assert_eq!(value(&read, "last"), PlcValue::U16(22));
        assert_eq!(read.split_into().get(), 2, "far apart, so two requests");
    }

    // covers: MB-EXC-02, MB-ADDR-EXTENDED / a file the device does not have is ILLEGAL DATA
    // ADDRESS for a read and for a write, and a refused write changes nothing
    #[test]
    fn a_file_the_device_does_not_have_is_an_illegal_data_address() {
        let (device, rig) = seeded();
        device.limit_extended_files(3);
        device.put_extended(29_999, &[7]);
        let read = rig
            .read(&[
                ("inside", "extended-register:29999"),
                ("outside", "extended-register:30001"),
            ])
            .expect("read");
        assert_eq!(
            value(&read, "inside"),
            PlcValue::I16(7),
            "file 3, record 9999"
        );
        assert_eq!(
            failure(&read, "outside").kind(),
            ErrorKind::InvalidAddress,
            "file 4"
        );
        let write = rig
            .write(vec![(
                tag("w", "extended-register:29999:DINT"),
                PlcValue::I32(-1),
            )])
            .expect("write");
        assert_eq!(
            rejection(&write, "w").kind(),
            ErrorKind::InvalidAddress,
            "the second sub-request names file 4"
        );
        assert_eq!(device.applied(), 0, "nothing applied");
        assert_eq!(
            device.extended(29_999),
            7,
            "the first sub-request was not kept"
        );
    }

    // covers: MB-READ-MERGE, MB-LIMIT-REGISTERS, MB-ADDR-EXTENDED / a run of extended tags is cut
    // at 120 registers, which a file record response can carry, not at 125
    #[test]
    fn a_run_of_extended_tags_is_cut_at_120_registers() {
        for (count, chunks) in [(120_u32, 1_u32), (121, 2), (240, 2), (241, 3)] {
            let (device, rig) = seeded();
            let seed: Vec<u16> = (1..=u16::try_from(count).expect("small")).collect();
            device.put_extended(1, &seed);
            let ids: Vec<String> = (1..=count).map(|n| format!("t{n}")).collect();
            let addresses: Vec<String> = (1..=count)
                .map(|n| format!("extended-register:{n}:UINT"))
                .collect();
            let tags: Vec<(&str, &str)> = ids
                .iter()
                .map(String::as_str)
                .zip(addresses.iter().map(String::as_str))
                .collect();
            let response = rig.read(&tags).expect("read");
            assert_eq!(response.split_into().get(), chunks, "{count} registers");
            for (id, expected) in ids.iter().zip(&seed) {
                assert_eq!(value(&response, id), PlcValue::U16(*expected), "{id}");
            }
        }
    }

    // covers: MB-READ-MERGE, MB-ADDR-EXTENDED / a full block across a file end carries the
    // registers of both files in order
    #[test]
    fn a_full_block_across_a_file_end_reads_the_registers_of_both_files() {
        let (device, rig) = seeded();
        let seed: Vec<u16> = (0..120).map(|n| 1_000 + n).collect();
        // Registers 9950 to 10069: 50 in file 1 and 70 in file 2.
        device.put_extended(9_950, &seed);
        let ids: Vec<String> = (0..120).map(|n| format!("t{n}")).collect();
        let addresses: Vec<String> = (0..120)
            .map(|n| format!("extended-register:{}:UINT", 9_950 + n))
            .collect();
        let tags: Vec<(&str, &str)> = ids
            .iter()
            .map(String::as_str)
            .zip(addresses.iter().map(String::as_str))
            .collect();
        let response = rig.read(&tags).expect("read");
        assert_eq!(response.split_into().get(), 1, "one request");
        for (id, expected) in ids.iter().zip(&seed) {
            assert_eq!(value(&response, id), PlcValue::U16(*expected), "{id}");
        }
        assert_eq!(rig.sent().len(), 1);
    }
}
