//! `ModbusConnection::write` on the fake transport and the fake clock (slice S46): the
//! function code each area and type goes out under, the byte and word order of the payload,
//! the access policy, and the outcome of every way a write can end.
//!
//! Wire bytes are written out by hand from the MBAP layout, the V1.1b PDU examples and the
//! register words of `fixtures/modbus/interop/register-map.toml`; no byte comes from the
//! encoder under test. All timing is on the fake clock (decision D21), and
//! `sent()` of the fake transport is the transmission count: a replayed write would be a
//! second frame.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection, ModbusTcpDriver};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, FixtureAddress, Script, Step, block_on_with, poll_once,
    };
    use stonecrab::{
        AccessPolicy, AddressOptions, ByteOrder, CancellationToken, Clock, Connection,
        ConnectionConfig, ConnectionState, Deadline, Driver, Endpoint, ErrorKind, FailureCategory,
        Operation, PlcError, PlcValue, ProtocolStatus, ReadRequest, Result, RetryPolicy, Tag,
        TagId, Timeouts, UnknownReason, WordOrder, WriteOutcome, WriteRequest, WriteResponse,
    };

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    /// An ADU of unit 1 as it goes on the wire.
    fn adu(txn: u16, pdu: &[u8]) -> Vec<u8> {
        unit_adu(txn, 1, pdu)
    }

    /// An ADU of `unit` as it goes on the wire.
    fn unit_adu(txn: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let length = u16::try_from(pdu.len().saturating_add(1)).expect("a short PDU");
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(unit);
        bytes.extend_from_slice(pdu);
        bytes
    }

    /// The read of holding register 1 (transaction `txn`) and its reply carrying 5.
    fn read_frame(txn: u16) -> Vec<u8> {
        adu(txn, &[0x03, 0x00, 0x00, 0x00, 0x01])
    }

    fn read_reply(txn: u16) -> Vec<u8> {
        adu(txn, &[0x03, 0x02, 0x00, 0x05])
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

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        /// A read-write connection over one strict script.
        fn open(steps: Vec<Step>) -> Self {
            Self::with(
                config(AccessPolicy::ReadWrite),
                ConnectionDefaults::default(),
                vec![Script::strict(steps)],
            )
        }

        fn with(
            config: ConnectionConfig,
            defaults: ConnectionDefaults,
            scripts: Vec<Script>,
        ) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            for script in scripts {
                factory.push_script(script);
            }
            let t0 = clock.now();
            let connection = block_on_with(
                &clock,
                ModbusConnection::open(
                    config,
                    Arc::new(factory.clone()),
                    Arc::new(clock.clone()),
                    defaults,
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

        fn request(&self, items: Vec<(Tag, PlcValue)>) -> WriteRequest {
            let mut builder = WriteRequest::builder(self.connection.limits());
            for (tag, value) in items {
                builder = builder.item(tag, value);
            }
            builder.build().expect("write request")
        }

        fn write(&self, items: Vec<(Tag, PlcValue)>) -> Result<WriteResponse> {
            block_on_with(&self.clock, self.connection.write(self.request(items)))
        }

        fn read_request(&self) -> ReadRequest {
            ReadRequest::builder(self.connection.limits())
                .tag(tag("a", "holding-register:1"))
                .build()
                .expect("read request")
        }

        fn read(&self) -> Result<stonecrab::ReadResponse> {
            block_on_with(&self.clock, self.connection.read(self.read_request()))
        }

        /// The frames the `index`th transport received.
        fn sent_on(&self, index: usize) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .get(index)
                .expect("that transport was opened")
                .sent()
                .iter()
                .map(|frame| frame.to_vec())
                .collect()
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            self.sent_on(0)
        }

        fn elapsed(&self) -> Duration {
            self.clock.now().saturating_duration_since(self.t0)
        }
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    fn outcome<'a>(response: &'a WriteResponse, id: &str) -> &'a WriteOutcome {
        response
            .get(&TagId::new(id).expect("tag id"))
            .unwrap_or_else(|| panic!("a result for {id}"))
            .outcome()
    }

    fn is_unknown(response: &WriteResponse, id: &str, reason: UnknownReason) -> bool {
        matches!(outcome(response, id), WriteOutcome::Unknown(actual) if *actual == reason)
    }

    /// The error of a rejected item.
    fn rejection<'a>(response: &'a WriteResponse, id: &str) -> &'a stonecrab::ItemError {
        let found = outcome(response, id);
        let WriteOutcome::Rejected(error) = found else {
            panic!("{id} was expected to be rejected, got {found:?}");
        };
        error
    }

    /// The REAL the upstream files write, parsed from the text they carry.
    fn f32_of(text: &str) -> f32 {
        text.parse().expect("a REAL")
    }

    fn f64_of(text: &str) -> f64 {
        text.parse().expect("a LREAL")
    }

    // covers: MB-TCP-WRITE / each area and type goes out under the function code upstream
    // picks, with the words the interop register map documents
    #[test]
    fn every_area_and_type_is_written_with_the_function_code_upstream_picks() {
        // (address, value, request PDU, reply PDU). Function 15 for every coil, function 06
        // for a payload of one register and function 16 for the rest.
        let cases: Vec<(&str, PlcValue, Vec<u8>, Vec<u8>)> = vec![
            (
                "coil:20",
                PlcValue::Bool(true),
                vec![0x0F, 0x00, 0x13, 0x00, 0x01, 0x01, 0x01],
                vec![0x0F, 0x00, 0x13, 0x00, 0x01],
            ),
            (
                "coil:20",
                PlcValue::Bool(false),
                vec![0x0F, 0x00, 0x13, 0x00, 0x01, 0x01, 0x00],
                vec![0x0F, 0x00, 0x13, 0x00, 0x01],
            ),
            // V1.1b section 6.6: write register 2 with 0x0003.
            (
                "holding-register:2:UINT",
                PlcValue::U16(3),
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
            ),
            (
                "holding-register:1:BOOL",
                PlcValue::Bool(true),
                vec![0x06, 0x00, 0x00, 0x00, 0x01],
                vec![0x06, 0x00, 0x00, 0x00, 0x01],
            ),
            (
                "holding-register:2:BYTE",
                PlcValue::U8(42),
                vec![0x06, 0x00, 0x01, 0x00, 0x2A],
                vec![0x06, 0x00, 0x01, 0x00, 0x2A],
            ),
            (
                "holding-register:10:SINT",
                PlcValue::I8(-42),
                vec![0x06, 0x00, 0x09, 0x00, 0xD6],
                vec![0x06, 0x00, 0x09, 0x00, 0xD6],
            ),
            (
                "holding-register:12:INT",
                PlcValue::I16(-2424),
                vec![0x06, 0x00, 0x0B, 0xF6, 0x88],
                vec![0x06, 0x00, 0x0B, 0xF6, 0x88],
            ),
            (
                "holding-register:4:DWORD",
                PlcValue::U32(4_242_442_424),
                vec![0x10, 0x00, 0x03, 0x00, 0x02, 0x04, 0xFC, 0xDE, 0x88, 0xB8],
                vec![0x10, 0x00, 0x03, 0x00, 0x02],
            ),
            (
                "holding-register:14:DINT",
                PlcValue::I32(-242_442_424),
                vec![0x10, 0x00, 0x0D, 0x00, 0x02, 0x04, 0xF1, 0x8C, 0x9F, 0x48],
                vec![0x10, 0x00, 0x0D, 0x00, 0x02],
            ),
            (
                "holding-register:26:REAL",
                PlcValue::F32(f32_of("3.141593")),
                vec![0x10, 0x00, 0x19, 0x00, 0x02, 0x04, 0x40, 0x49, 0x0F, 0xDC],
                vec![0x10, 0x00, 0x19, 0x00, 0x02],
            ),
            (
                "holding-register:6:LWORD",
                PlcValue::U64(4_242_442_424_242_424_242),
                vec![
                    0x10, 0x00, 0x05, 0x00, 0x04, 0x08, 0x3A, 0xE0, 0x2E, 0xE8, 0x4D, 0x04, 0x49,
                    0xB2,
                ],
                vec![0x10, 0x00, 0x05, 0x00, 0x04],
            ),
            (
                "holding-register:18:LINT",
                PlcValue::I64(-4_242_442_424_242_424_242),
                vec![
                    0x10, 0x00, 0x11, 0x00, 0x04, 0x08, 0xC5, 0x1F, 0xD1, 0x17, 0xB2, 0xFB, 0xB6,
                    0x4E,
                ],
                vec![0x10, 0x00, 0x11, 0x00, 0x04],
            ),
            (
                "holding-register:28:LREAL",
                PlcValue::F64(f64_of("2.71828182846")),
                vec![
                    0x10, 0x00, 0x1B, 0x00, 0x04, 0x08, 0x40, 0x05, 0xBF, 0x0A, 0x8B, 0x14, 0x5F,
                    0xCF,
                ],
                vec![0x10, 0x00, 0x1B, 0x00, 0x04],
            ),
        ];
        let mut steps = Vec::new();
        let mut frames = Vec::new();
        for (index, (_, _, request, reply)) in cases.iter().enumerate() {
            let txn = u16::try_from(index.saturating_add(1)).expect("a few cases");
            frames.push(adu(txn, request));
            steps.push(Step::Expect(adu(txn, request)));
            steps.push(Step::Reply(adu(txn, reply)));
        }
        let rig = Rig::open(steps);
        let items = cases
            .iter()
            .enumerate()
            .map(|(index, (address, value, _, _))| {
                (tag(&format!("t{index}"), address), value.clone())
            })
            .collect();
        let response = rig.write(items).expect("the write completes");
        assert_eq!(
            response.confirmed_count(),
            cases.len(),
            "every item echoed: {response:?}"
        );
        assert_eq!(rig.sent(), frames, "one frame per item, in request order");
        assert_eq!(rig.connection.state(), ConnectionState::Connected, "state");
    }

    // covers: MB-TCP-WRITE, MB-BYTEORDER-PRECEDENCE / the order of the payload follows the
    // tag, then the address block, then the connection default
    #[test]
    fn byte_and_word_order_follow_the_tag_then_the_address_then_the_connection_default() {
        let value = PlcValue::U32(0x0102_0304);
        let write = |bytes: [u8; 4]| {
            let mut pdu = vec![0x10, 0x00, 0x00, 0x00, 0x02, 0x04];
            pdu.extend_from_slice(&bytes);
            pdu
        };
        let echo = vec![0x10, 0x00, 0x00, 0x00, 0x02];
        let plain = || tag("t", "holding-register:1:DWORD");
        let block = || tag("t", "holding-register:1:DWORD{byte-order: 'LITTLE_ENDIAN'}");
        let cases: Vec<(&str, Tag, [u8; 4])> = vec![
            ("the default BIG_ENDIAN", plain(), [1, 2, 3, 4]),
            (
                "LITTLE_ENDIAN on the tag",
                plain()
                    .with_byte_order(ByteOrder::LittleEndian)
                    .with_word_order(WordOrder::LowWordFirst),
                [4, 3, 2, 1],
            ),
            (
                "BIG_ENDIAN_BYTE_SWAP on the tag",
                plain()
                    .with_byte_order(ByteOrder::LittleEndian)
                    .with_word_order(WordOrder::HighWordFirst),
                [2, 1, 4, 3],
            ),
            (
                "LITTLE_ENDIAN_BYTE_SWAP on the tag",
                plain()
                    .with_byte_order(ByteOrder::BigEndian)
                    .with_word_order(WordOrder::LowWordFirst),
                [3, 4, 1, 2],
            ),
            ("LITTLE_ENDIAN in the address block", block(), [4, 3, 2, 1]),
            (
                "a tag field over the address block, field by field",
                block().with_byte_order(ByteOrder::BigEndian),
                [3, 4, 1, 2],
            ),
        ];
        let mut steps = Vec::new();
        for (index, (_, _, bytes)) in cases.iter().enumerate() {
            let txn = u16::try_from(index.saturating_add(1)).expect("a few cases");
            steps.push(Step::Expect(adu(txn, &write(*bytes))));
            steps.push(Step::Reply(adu(txn, &echo)));
        }
        let rig = Rig::open(steps);
        for (name, tag, _) in cases {
            let response = rig
                .write(vec![(tag, value.clone())])
                .expect("the write completes");
            assert_eq!(response.confirmed_count(), 1, "{name}: {response:?}");
        }

        // With nothing on the tag or the address, the connection default decides.
        let mut defaults = ConnectionDefaults::default();
        defaults.byte_order = ByteOrder::LittleEndian;
        defaults.word_order = WordOrder::LowWordFirst;
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            defaults,
            vec![Script::strict(vec![
                Step::Expect(adu(1, &write([4, 3, 2, 1]))),
                Step::Reply(adu(1, &echo)),
            ])],
        );
        let response = rig.write(vec![(plain(), value)]).expect("the write");
        assert_eq!(response.confirmed_count(), 1, "the connection default");
    }

    // covers: MB-TCP-WRITE, MB-CFG-UNIT-ID / the unit comes from the address, else the default
    #[test]
    fn the_unit_comes_from_the_address_else_the_connection_default() {
        let mut defaults = ConnectionDefaults::default();
        defaults.unit_id = stonecrab::modbus::UnitId::from(9);
        let pdu = [0x06, 0x00, 0x00, 0x00, 0x01];
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            defaults,
            vec![Script::strict(vec![
                Step::Expect(unit_adu(1, 9, &pdu)),
                Step::Reply(unit_adu(1, 9, &pdu)),
                Step::Expect(unit_adu(2, 7, &pdu)),
                Step::Reply(unit_adu(2, 7, &pdu)),
            ])],
        );
        let response = rig
            .write(vec![
                (tag("default", "holding-register:1:UINT"), PlcValue::U16(1)),
                (
                    tag("named", "holding-register:1:UINT{unit-id: 7}"),
                    PlcValue::U16(1),
                ),
            ])
            .expect("the write completes");
        assert_eq!(response.confirmed_count(), 2, "{response:?}");
    }

    // covers: SPEC connection-and-resource-behavior / read-only policy; MB-WRITE-ACCESS
    #[test]
    fn a_read_only_connection_refuses_a_write_before_anything_else() {
        let rig = Rig::with(
            config(AccessPolicy::ReadOnly),
            ConnectionDefaults::default(),
            vec![Script::strict(vec![])],
        );
        assert!(rig.connection.capabilities().write, "the capability");
        let items = || vec![(tag("a", "holding-register:1:UINT"), PlcValue::U16(1))];
        let error = error_of(rig.write(items()));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "kind"
        );
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Write), "operation");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "state"
        );
        assert!(rig.sent().is_empty(), "nothing sent");
        assert_eq!(rig.elapsed(), Duration::ZERO, "no time passed");

        // The policy is the first word: not a cancelled token, not a closed connection.
        let token = CancellationToken::new();
        token.cancel();
        let cancelled = WriteRequest::builder(rig.connection.limits())
            .item(tag("a", "holding-register:1:UINT"), PlcValue::U16(1))
            .cancel(token)
            .build()
            .expect("request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.write(cancelled)));
        assert_eq!(error.kind(), ErrorKind::Unsupported(Operation::Write));
        block_on_with(&rig.clock, rig.connection.close()).expect("close");
        let error = error_of(rig.write(items()));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "a closed connection is refused for its policy first"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register or node; F4 cancellation
    // before transmit; F5 explicit shutdown
    #[test]
    fn a_write_that_cannot_begin_fails_the_call_with_nothing_sent() {
        let rig = Rig::open(vec![]);
        let one = || vec![(tag("a", "holding-register:1:UINT"), PlcValue::U16(1))];

        // A tag of another driver.
        let foreign = Tag::new(
            TagId::new("f").expect("tag id"),
            Arc::new(FixtureAddress::new("elsewhere", None)),
        );
        let error = error_of(rig.write(vec![(foreign, PlcValue::U16(1))]));
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "foreign address");
        assert_eq!(error.diagnostics().operation, Some(Operation::Write));

        // An element count above one is refused: a range selection in the address reads
        // several elements.
        let many = tag("m", "holding-register:1:UINT").with_count(NonZeroU32::new(2).expect("two"));
        let error = error_of(rig.write(vec![(many, PlcValue::U16(1))]));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "element count");

        // A token that fired before the call.
        let token = CancellationToken::new();
        token.cancel();
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("a", "holding-register:1:UINT"), PlcValue::U16(1))
            .cancel(token)
            .build()
            .expect("request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.write(request)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "cancelled token");

        // A deadline that had passed.
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("a", "holding-register:1:UINT"), PlcValue::U16(1))
            .deadline(Deadline::at(rig.clock.now()))
            .build()
            .expect("request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.write(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "elapsed deadline");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Write),
            "operation"
        );

        // A closed connection.
        block_on_with(&rig.clock, rig.connection.close()).expect("close");
        let error = error_of(rig.write(one()));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "closed");
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Closed),
            "the state at the failure"
        );
        assert!(rig.sent().is_empty(), "nothing was sent by any of them");
        assert_eq!(rig.elapsed(), Duration::ZERO, "no time passed");
    }

    // covers: MB-TCP-WRITE, MB-EXC-01, MB-EXC-02, MB-EXC-04 / an echo confirms and an exception
    // rejects, each item on its own
    #[test]
    fn an_echo_confirms_and_a_device_exception_rejects_each_item_on_its_own() {
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &[0x06, 0x00, 0x00, 0x00, 0x01])),
            Step::Reply(adu(1, &[0x86, 0x01])),
            Step::Expect(adu(2, &[0x06, 0x00, 0x01, 0x00, 0x02])),
            Step::Reply(adu(2, &[0x86, 0x02])),
            Step::Expect(adu(3, &[0x0F, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01])),
            Step::Reply(adu(3, &[0x8F, 0x04])),
            Step::Expect(adu(4, &[0x10, 0x00, 0x02, 0x00, 0x02, 0x04, 0, 0, 0, 3])),
            Step::Reply(adu(4, &[0x90, 0x06])),
            Step::Expect(adu(5, &[0x06, 0x00, 0x04, 0x00, 0x05])),
            Step::Reply(adu(5, &[0x06, 0x00, 0x04, 0x00, 0x05])),
        ]);
        let response = rig
            .write(vec![
                (
                    tag("illegal-function", "holding-register:1:UINT"),
                    PlcValue::U16(1),
                ),
                (
                    tag("illegal-address", "holding-register:2:UINT"),
                    PlcValue::U16(2),
                ),
                (tag("device-failure", "coil:1"), PlcValue::Bool(true)),
                (tag("busy", "holding-register:3:UDINT"), PlcValue::U32(3)),
                (tag("fine", "holding-register:5:UINT"), PlcValue::U16(5)),
            ])
            .expect("a refused write is an outcome, not an error");
        assert_eq!(
            (
                response.confirmed_count(),
                response.rejected_count(),
                response.unknown_count()
            ),
            (1, 4, 0),
            "{response:?}"
        );
        assert!(matches!(
            outcome(&response, "fine"),
            WriteOutcome::Confirmed
        ));
        for (id, kind, code, name) in [
            (
                "illegal-function",
                ErrorKind::Unsupported(Operation::Write),
                1,
                "ILLEGAL FUNCTION",
            ),
            (
                "illegal-address",
                ErrorKind::InvalidAddress,
                2,
                "ILLEGAL DATA ADDRESS",
            ),
            (
                "device-failure",
                ErrorKind::DeviceException,
                4,
                "SERVER DEVICE FAILURE",
            ),
            ("busy", ErrorKind::DeviceException, 6, "SERVER DEVICE BUSY"),
        ] {
            let error = rejection(&response, id);
            assert_eq!(error.kind(), kind, "{id}: kind");
            let status = error.status().unwrap_or_else(|| panic!("{id}: a status"));
            assert_eq!(
                (status.protocol(), status.code(), status.name()),
                ("modbus", code, Some(name)),
                "{id}: the device's own code"
            );
            let diagnostics = error.error().diagnostics();
            assert_eq!(
                diagnostics.operation,
                Some(Operation::Write),
                "{id}: operation"
            );
            assert_eq!(
                diagnostics.category,
                Some(FailureCategory::Protocol),
                "{id}: category"
            );
            assert_eq!(
                diagnostics.connection_state,
                Some(ConnectionState::Connected),
                "{id}: an exception is an answer"
            );
        }
        assert_eq!(
            response.results().iter().filter_map(|r| r.status()).count(),
            0,
            "a rejection keeps its status on the item error, not on the result"
        );
        assert_eq!(rig.sent().len(), 5, "one frame per item");
        assert_eq!(rig.connection.state(), ConnectionState::Connected, "state");
    }

    // covers: SPEC errors / ambiguous write never replayed; F2 timeout; MB-WRITE-UNKNOWN
    #[test]
    fn a_write_with_no_answer_is_unknown_and_is_never_replayed() {
        // A policy that would retry a timeout twice, which it must not do for a write.
        let mut retry = RetryPolicy::default();
        retry.max_attempts = NonZeroU32::new(3).expect("three");
        retry.retry_on = [ErrorKind::Timeout, ErrorKind::Disconnected]
            .into_iter()
            .collect();
        let configured =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .access(AccessPolicy::ReadWrite)
                .retry(retry)
                .build()
                .expect("config");
        let write = adu(1, &[0x06, 0x00, 0x00, 0x00, 0x07]);
        let rig = Rig::with(
            configured,
            ConnectionDefaults::default(),
            vec![Script::strict(vec![
                Step::Expect(write.clone()),
                // No reply: the device may or may not have applied it.
                Step::Expect(read_frame(2)),
                Step::Reply(read_reply(2)),
            ])],
        );
        let response = rig
            .write(vec![(
                tag("w", "holding-register:1:UINT"),
                PlcValue::U16(7),
            )])
            .expect("a write with no answer is an outcome");
        assert!(
            is_unknown(&response, "w", UnknownReason::TimeoutAfterTransmit),
            "{response:?}"
        );
        assert_eq!(response.unknown_count(), 1);
        assert_eq!(
            rig.elapsed(),
            Timeouts::default().request,
            "one request timeout, not one per attempt"
        );
        assert_eq!(rig.sent(), vec![write], "the frame went out once");
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Connected,
            "a timeout does not drop the link"
        );

        // The next operation is a read with its own transaction id; the write is not resent.
        drop(rig.read().expect("the connection works afterwards"));
        assert_eq!(rig.sent().len(), 2, "the write, then the read");
        assert_eq!(rig.sent().get(1), Some(&read_frame(2)), "the read");
    }

    // covers: SPEC mandatory-failure-cases / F2 timeout; the request deadline bounds a write
    #[test]
    fn the_request_deadline_bounds_the_wait_for_the_confirmation() {
        let rig = Rig::open(vec![Step::Expect(adu(1, &[0x06, 0x00, 0x00, 0x00, 0x01]))]);
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("w", "holding-register:1:UINT"), PlcValue::U16(1))
            .deadline(Deadline::after(rig.clock.now(), secs(1)).expect("deadline"))
            .build()
            .expect("request");
        let response = block_on_with(&rig.clock, rig.connection.write(request)).expect("outcome");
        assert!(is_unknown(
            &response,
            "w",
            UnknownReason::TimeoutAfterTransmit
        ));
        assert_eq!(rig.elapsed(), secs(1), "the deadline, not the 5 s timeout");
    }

    // covers: SPEC errors / ambiguous write never replayed; F5 network outage
    #[test]
    fn a_link_lost_after_the_frame_went_out_is_unknown_and_nothing_more_is_sent() {
        let write = adu(
            1,
            &[0x10, 0x00, 0x00, 0x00, 0x02, 0x04, 0x00, 0x01, 0x00, 0x02],
        );
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            ConnectionDefaults::default(),
            vec![
                Script::strict(vec![Step::Expect(write.clone()), Step::CloseAt(0)]),
                Script::strict(vec![]),
            ],
        );
        let response = rig
            .write(vec![(
                tag("w", "holding-register:1:UDINT"),
                PlcValue::U32(0x0001_0002),
            )])
            .expect("a lost link after the frame is an outcome");
        assert!(
            is_unknown(&response, "w", UnknownReason::DisconnectedAfterTransmit),
            "{response:?}"
        );
        assert_eq!(rig.sent(), vec![write], "one frame");
        assert_eq!(
            rig.factory.opens().len(),
            1,
            "the driver did not reconnect to send it again: reconnecting waits for the \
             next request"
        );
        assert!(
            matches!(rig.connection.state(), ConnectionState::Reconnecting { .. }),
            "the state shows the lost link"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting); SPEC errors /
    // ambiguous write never replayed
    #[test]
    fn a_cancelled_token_after_the_frame_went_out_is_unknown() {
        let write = adu(1, &[0x06, 0x00, 0x00, 0x00, 0x01]);
        let rig = Rig::open(vec![Step::Expect(write.clone())]);
        let token = CancellationToken::new();
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("w", "holding-register:1:UINT"), PlcValue::U16(1))
            .cancel(token.clone())
            .build()
            .expect("request");
        let mut pending = rig.connection.write(request);
        assert!(
            poll_once(&mut pending).is_pending(),
            "the write waits for its echo"
        );
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut pending) else {
            panic!("a cancelled write resolves at once");
        };
        let response = outcome.expect("a cancelled write that went out is an outcome");
        assert!(
            is_unknown(&response, "w", UnknownReason::CancelledAfterTransmit),
            "{response:?}"
        );
        assert_eq!(rig.sent(), vec![write], "the frame had reached the wire");
    }

    // covers: MB-TCP-WRITE, MB-WRITE-ECHO / the reply must echo the request, with the function
    // code of the request
    #[test]
    fn a_reply_that_does_not_echo_the_request_is_unknown() {
        let request = [0x10, 0x00, 0x00, 0x00, 0x02, 0x04, 0x00, 0x01, 0x00, 0x02];
        let single = [0x06, 0x00, 0x00, 0x00, 0x01];
        // (name, request PDU, the reply PDU that does not confirm it)
        let cases: [(&str, &[u8], &[u8]); 5] = [
            // The upstream all-datatypes files pair an FC06 request with an FC16 response:
            // Stonecrab waits for the echo of the function it sent (docs/parity MB-WRITE-ECHO).
            (
                "FC06 answered as FC16",
                &single,
                &[0x10, 0x00, 0x00, 0x00, 0x01],
            ),
            (
                "FC06 echoing another value",
                &single,
                &[0x06, 0x00, 0x00, 0x00, 0x09],
            ),
            (
                "FC06 echoing another address",
                &single,
                &[0x06, 0x00, 0x05, 0x00, 0x01],
            ),
            (
                "FC16 echoing another quantity",
                &request,
                &[0x10, 0x00, 0x00, 0x00, 0x01],
            ),
            ("FC16 with a truncated echo", &request, &[0x10, 0x00, 0x00]),
        ];
        for (name, sent, reply) in cases {
            let rig = Rig::open(vec![Step::Expect(adu(1, sent)), Step::Reply(adu(1, reply))]);
            let (id, value) = if sent.len() == single.len() {
                ("holding-register:1:UINT", PlcValue::U16(1))
            } else {
                ("holding-register:1:UDINT", PlcValue::U32(0x0001_0002))
            };
            let response = rig
                .write(vec![(tag("w", id), value)])
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                matches!(outcome(&response, "w"), WriteOutcome::Unknown(_)),
                "{name}: {response:?}"
            );
            assert_eq!(rig.sent().len(), 1, "{name}: sent once, never replayed");
        }
    }

    /// The defaults of a connection opened with `single-coil-write=fc05`.
    fn fc05_defaults() -> ConnectionDefaults {
        let mut defaults = ConnectionDefaults::default();
        defaults.single_coil_fc05 = true;
        defaults
    }

    // covers: MB-CFG-SINGLE-COIL, MB-TCP-WRITE / function 05 for exactly one coil, and nothing else
    // changes
    #[test]
    fn single_coil_write_fc05_sends_function_05_for_one_coil_and_the_upstream_function_otherwise() {
        let list =
            |bits: &[bool]| PlcValue::List(bits.iter().map(|bit| PlcValue::Bool(*bit)).collect());
        // (address, value, request PDU, reply PDU). V1.1b section 6.5: ON is FF00, OFF is 0000.
        let cases: Vec<(&str, PlcValue, Vec<u8>, Vec<u8>)> = vec![
            (
                "coil:20",
                PlcValue::Bool(true),
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
            ),
            (
                "coil:20",
                PlcValue::Bool(false),
                vec![0x05, 0x00, 0x13, 0x00, 0x00],
                vec![0x05, 0x00, 0x13, 0x00, 0x00],
            ),
            // A selection of one element is one coil too.
            (
                "coil:20[0..0]",
                list(&[true]),
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
            ),
            // Two coils are function 15, as upstream sends them.
            (
                "coil:20[0..1]",
                list(&[true, false]),
                vec![0x0F, 0x00, 0x13, 0x00, 0x02, 0x01, 0x01],
                vec![0x0F, 0x00, 0x13, 0x00, 0x02],
            ),
            // Registers are not coils.
            (
                "holding-register:2:UINT",
                PlcValue::U16(3),
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
            ),
            (
                "holding-register:2:BOOL",
                PlcValue::Bool(true),
                vec![0x06, 0x00, 0x01, 0x00, 0x01],
                vec![0x06, 0x00, 0x01, 0x00, 0x01],
            ),
        ];
        let mut steps = Vec::new();
        for (index, (_, _, request, reply)) in cases.iter().enumerate() {
            let txn = u16::try_from(index + 1).expect("a few frames");
            steps.push(Step::Expect(adu(txn, request)));
            steps.push(Step::Reply(adu(txn, reply)));
        }
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            fc05_defaults(),
            vec![Script::strict(steps)],
        );
        for (index, (address, value, _, _)) in cases.iter().enumerate() {
            let response = rig
                .write(vec![(tag("w", address), value.clone())])
                .unwrap_or_else(|error| panic!("{address}: {error}"));
            assert!(
                matches!(outcome(&response, "w"), WriteOutcome::Confirmed),
                "case {index} {address}: {response:?}"
            );
        }
        assert_eq!(rig.sent().len(), cases.len(), "one frame a write");
    }

    // covers: MB-CFG-SINGLE-COIL, MB-WRITE-ECHO / the echo of function 05 is checked and the
    // outcomes are those of every other write
    #[test]
    fn a_function_05_write_is_confirmed_by_its_echo_and_every_other_end_keeps_the_s46_outcome() {
        let sent = [0x05, 0x00, 0x13, 0xFF, 0x00];
        let write = || (tag("w", "coil:20"), PlcValue::Bool(true));
        // Exceptions reject the item, as for the other functions.
        for (reply, kind, code) in [
            ([0x85, 0x01], ErrorKind::Unsupported(Operation::Write), 1),
            ([0x85, 0x02], ErrorKind::InvalidAddress, 2),
            ([0x85, 0x03], ErrorKind::DeviceException, 3),
            ([0x85, 0x04], ErrorKind::DeviceException, 4),
        ] {
            let rig = Rig::with(
                config(AccessPolicy::ReadWrite),
                fc05_defaults(),
                vec![Script::strict(vec![
                    Step::Expect(adu(1, &sent)),
                    Step::Reply(adu(1, &reply)),
                ])],
            );
            let response = rig.write(vec![write()]).expect("an outcome");
            let error = rejection(&response, "w");
            assert_eq!(error.kind(), kind, "exception {code}");
            assert_eq!(
                error.status().map(ProtocolStatus::code),
                Some(code),
                "the device's own code"
            );
            assert_eq!(rig.sent().len(), 1, "exception {code}: sent once");
        }
        // A reply that does not echo the request is not a confirmation: Unknown, sent once.
        let cases: [(&str, &[u8]); 6] = [
            ("another address", &[0x05, 0x00, 0x14, 0xFF, 0x00]),
            ("the other state", &[0x05, 0x00, 0x13, 0x00, 0x00]),
            ("a state that is neither", &[0x05, 0x00, 0x13, 0x12, 0x34]),
            (
                "function 15 for function 05",
                &[0x0F, 0x00, 0x13, 0x00, 0x01],
            ),
            (
                "function 06 for function 05",
                &[0x06, 0x00, 0x13, 0xFF, 0x00],
            ),
            ("a truncated echo", &[0x05, 0x00, 0x13]),
        ];
        for (name, reply) in cases {
            let rig = Rig::with(
                config(AccessPolicy::ReadWrite),
                fc05_defaults(),
                vec![Script::strict(vec![
                    Step::Expect(adu(1, &sent)),
                    Step::Reply(adu(1, reply)),
                ])],
            );
            let response = rig.write(vec![write()]).expect("an outcome");
            assert!(
                matches!(outcome(&response, "w"), WriteOutcome::Unknown(_)),
                "{name}: {response:?}"
            );
            assert_eq!(rig.sent().len(), 1, "{name}: sent once, never replayed");
        }
        // No answer: Unknown after the transmit, after one request timeout, never replayed.
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            fc05_defaults(),
            vec![Script::strict(vec![Step::Expect(adu(1, &sent))])],
        );
        let response = rig.write(vec![write()]).expect("an outcome");
        assert!(
            is_unknown(&response, "w", UnknownReason::TimeoutAfterTransmit),
            "{response:?}"
        );
        assert_eq!(rig.elapsed(), Timeouts::default().request);
        assert_eq!(rig.sent().len(), 1, "the frame went out once");
        // The first Unknown halts the batch, as before.
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            fc05_defaults(),
            vec![Script::strict(vec![Step::Expect(adu(1, &sent))])],
        );
        let response = rig
            .write(vec![
                write(),
                (tag("later", "coil:21"), PlcValue::Bool(false)),
            ])
            .expect("an outcome");
        assert!(matches!(
            rejection(&response, "later").kind(),
            ErrorKind::Cancelled
        ));
        assert_eq!(rig.sent().len(), 1, "nothing after the unknown outcome");
    }

    // covers: MB-CFG-SINGLE-COIL / the connection string selects function 05, and the default
    // stays function 15
    #[test]
    fn the_connection_string_selects_function_05_for_a_single_coil() {
        for (query, request, reply) in [
            (
                "",
                vec![0x0F, 0x00, 0x13, 0x00, 0x01, 0x01, 0x01],
                vec![0x0F, 0x00, 0x13, 0x00, 0x01],
            ),
            (
                "&single-coil-write=fc15",
                vec![0x0F, 0x00, 0x13, 0x00, 0x01, 0x01, 0x01],
                vec![0x0F, 0x00, 0x13, 0x00, 0x01],
            ),
            (
                "&single-coil-write=fc05",
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
                vec![0x05, 0x00, 0x13, 0xFF, 0x00],
            ),
        ] {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            factory.push_script(Script::strict(vec![
                Step::Expect(adu(1, &request)),
                Step::Reply(adu(1, &reply)),
            ]));
            let driver = ModbusTcpDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_transport_factory(Arc::new(factory.clone()));
            let config = driver
                .parse_config(&format!("modbus-tcp://192.0.2.10?access=read-write{query}"))
                .expect("config");
            let address = driver
                .parse_address("coil:20", &config.address_options())
                .expect("address");
            let connection = block_on_with(&clock, driver.connect(config)).expect("connect");
            let request = WriteRequest::builder(connection.limits())
                .item(
                    Tag::new(TagId::new("w").expect("id"), address),
                    PlcValue::Bool(true),
                )
                .build()
                .expect("request");
            let response =
                block_on_with(&clock, connection.write(request)).expect("a write resolves");
            assert_eq!(response.confirmed_count(), 1, "{query:?}");
        }
        let driver = ModbusTcpDriver::new();
        for refused in ["fc5", "FC05", "05", "true", ""] {
            let error = driver
                .parse_config(&format!(
                    "modbus-tcp://192.0.2.10?single-coil-write={refused}"
                ))
                .expect_err("refused");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{refused:?}");
        }
    }

    // covers: SPEC mandatory-failure-cases / F4 one operation in flight
    #[test]
    fn a_write_and_a_read_exclude_each_other_before_any_io() {
        // A write while a read waits for its reply.
        let rig = Rig::open(vec![Step::Expect(read_frame(1))]);
        let mut reading = rig.connection.read(rig.read_request());
        assert!(poll_once(&mut reading).is_pending(), "the read waits");
        let refused = error_of(rig.write(vec![(
            tag("w", "holding-register:1:UINT"),
            PlcValue::U16(1),
        )]));
        assert_eq!(
            refused.kind(),
            ErrorKind::Backpressure,
            "a write during a read"
        );
        assert_eq!(refused.diagnostics().operation, Some(Operation::Write));
        assert_eq!(
            refused.diagnostics().category,
            Some(FailureCategory::Validation)
        );
        assert_eq!(
            rig.sent(),
            vec![read_frame(1)],
            "the refused write sent nothing"
        );
        drop(reading);

        // A read and a ping while a write waits for its echo.
        let write = adu(1, &[0x06, 0x00, 0x00, 0x00, 0x01]);
        let rig = Rig::open(vec![Step::Expect(write.clone())]);
        let mut writing = rig.connection.write(rig.request(vec![(
            tag("w", "holding-register:1:UINT"),
            PlcValue::U16(1),
        )]));
        assert!(poll_once(&mut writing).is_pending(), "the write waits");
        assert_eq!(
            error_of(rig.read()).kind(),
            ErrorKind::Backpressure,
            "a read during a write"
        );
        assert_eq!(
            error_of(block_on_with(&rig.clock, rig.connection.ping())).kind(),
            ErrorKind::Backpressure,
            "a ping during a write"
        );
        assert_eq!(rig.sent(), vec![write], "only the write is on the wire");
        drop(writing);
        assert!(
            rig.connection.state() == ConnectionState::Connected,
            "dropping a write frees the slot and keeps the link"
        );
    }

    // covers: MB-TCP-WRITE, MB-WRITE-HALT, MB-QUIRK-C / a batch of mixed fates: each item has
    // its own outcome, nothing is sent after an unknown one, and read-only areas never go out
    #[test]
    fn a_batch_gives_each_item_its_outcome_and_stops_sending_after_an_unknown() {
        let rig = Rig::open(vec![
            Step::Expect(adu(1, &[0x0F, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01])),
            Step::Reply(adu(1, &[0x0F, 0x00, 0x00, 0x00, 0x01])),
            Step::Expect(adu(2, &[0x06, 0x00, 0x00, 0x00, 0x07])),
            Step::Reply(adu(2, &[0x86, 0x04])),
            // The third request is not answered.
            Step::Expect(adu(3, &[0x06, 0x00, 0x01, 0x00, 0x08])),
        ]);
        let response = rig
            .write(vec![
                (tag("a", "coil:1"), PlcValue::Bool(true)),
                (tag("input", "discrete-input:1"), PlcValue::Bool(true)),
                (tag("sensor", "input-register:1:UINT"), PlcValue::U16(1)),
                (tag("b", "holding-register:1:UINT"), PlcValue::U16(7)),
                (tag("c", "holding-register:2:UINT"), PlcValue::U16(8)),
                (tag("d", "holding-register:3:UINT"), PlcValue::U16(9)),
                (tag("late-input", "discrete-input:2"), PlcValue::Bool(true)),
            ])
            .expect("the write completes");
        let order: Vec<String> = response
            .results()
            .iter()
            .map(|result| result.tag().to_string())
            .collect();
        assert_eq!(
            order,
            ["a", "input", "sensor", "b", "c", "d", "late-input"],
            "request order"
        );
        assert!(matches!(outcome(&response, "a"), WriteOutcome::Confirmed));
        for id in ["input", "sensor", "late-input"] {
            let error = rejection(&response, id);
            assert_eq!(
                error.kind(),
                ErrorKind::Unsupported(Operation::Write),
                "{id}"
            );
            assert_eq!(
                error.error().diagnostics().category,
                Some(FailureCategory::Validation),
                "{id}: rejected locally, before any I/O"
            );
        }
        let device = rejection(&response, "b");
        assert_eq!(device.kind(), ErrorKind::DeviceException);
        assert_eq!(device.status().map(ProtocolStatus::code), Some(4));
        assert!(is_unknown(
            &response,
            "c",
            UnknownReason::TimeoutAfterTransmit
        ));
        let skipped = rejection(&response, "d");
        assert_eq!(
            skipped.kind(),
            ErrorKind::Cancelled,
            "not sent after an unknown"
        );
        assert!(skipped.to_string().contains("not sent"), "{skipped}");
        assert_eq!(
            (
                response.confirmed_count(),
                response.rejected_count(),
                response.unknown_count()
            ),
            (1, 5, 1)
        );
        assert_eq!(rig.sent().len(), 3, "a, b and c went out; d did not");
    }

    // covers: MB-TCP-WRITE / a request that cannot go out is that item's rejection, not an
    // error of the call
    #[test]
    fn a_request_that_cannot_go_out_is_that_items_rejection() {
        let write = adu(1, &[0x06, 0x00, 0x00, 0x00, 0x01]);
        let rig = Rig::with(
            config(AccessPolicy::ReadWrite),
            ConnectionDefaults::default(),
            vec![Script::strict(vec![
                Step::Expect(write.clone()),
                Step::CloseAt(0),
            ])],
        );
        let item = |id: &str, register: u16| {
            (
                tag(id, &format!("holding-register:{register}:UINT")),
                PlcValue::U16(1),
            )
        };
        // The peer closes without answering: the frame went out, so the write is unknown.
        let first = rig.write(vec![item("first", 1)]).expect("an outcome");
        assert!(is_unknown(
            &first,
            "first",
            UnknownReason::DisconnectedAfterTransmit
        ));

        // The link cannot be reopened: nothing of this call goes out, which is certain, so
        // the items are rejected (safe to submit again), each with the error that stopped it.
        rig.factory.push_refusal();
        rig.factory.push_refusal();
        let second = rig
            .write(vec![item("second", 2), item("third", 3)])
            .expect("every item has an outcome");
        for id in ["second", "third"] {
            let error = rejection(&second, id);
            assert_eq!(error.kind(), ErrorKind::Disconnected, "{id}");
            let diagnostics = error.error().diagnostics();
            assert_eq!(
                diagnostics.operation,
                Some(Operation::Write),
                "{id}: operation"
            );
            assert_eq!(
                diagnostics.category,
                Some(FailureCategory::Transport),
                "{id}: category"
            );
        }
        assert_eq!(rig.sent(), vec![write], "nothing more was sent");
        assert_eq!(
            rig.factory.opens().len(),
            3,
            "each item tried one reopen, and both were refused"
        );
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration
    #[test]
    fn a_request_timeout_the_clock_cannot_represent_rejects_the_write_before_any_io() {
        let mut timeouts = Timeouts::default();
        timeouts.request = Duration::MAX;
        let configured =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .access(AccessPolicy::ReadWrite)
                .timeouts(timeouts)
                .build()
                .expect("config");
        let rig = Rig::with(
            configured,
            ConnectionDefaults::default(),
            vec![Script::strict(vec![])],
        );
        let response = rig
            .write(vec![(
                tag("w", "holding-register:1:UINT"),
                PlcValue::U16(1),
            )])
            .expect("an outcome");
        let error = rejection(&response, "w");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        assert_eq!(
            error.error().diagnostics().operation,
            Some(Operation::Write)
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: MB-TCP-WRITE, MB-CFG-ACCESS / the connection string grants writes through the
    // driver, and a connection without the parameter is read-only
    #[test]
    fn the_driver_grants_writes_only_when_the_connection_string_says_so() {
        for (query, writes) in [
            ("", false),
            ("?access=read-only", false),
            ("?access=read-write", true),
        ] {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            let pdu = [0x06, 0x00, 0x00, 0x00, 0x01];
            factory.push_script(Script::strict(if writes {
                vec![Step::Expect(adu(1, &pdu)), Step::Reply(adu(1, &pdu))]
            } else {
                vec![]
            }));
            let driver = ModbusTcpDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_transport_factory(Arc::new(factory.clone()));
            assert!(driver.capabilities().write, "the driver can write");
            let config = driver
                .parse_config(&format!("modbus-tcp://192.0.2.10{query}"))
                .expect("config");
            let address = driver
                .parse_address("holding-register:1:UINT", &config.address_options())
                .expect("address");
            let connection = block_on_with(&clock, driver.connect(config)).expect("connect");
            let request = WriteRequest::builder(connection.limits())
                .item(
                    Tag::new(TagId::new("w").expect("id"), address),
                    PlcValue::U16(1),
                )
                .build()
                .expect("request");
            let result = block_on_with(&clock, connection.write(request));
            if writes {
                assert_eq!(result.expect("a write").confirmed_count(), 1, "{query}");
            } else {
                assert_eq!(
                    error_of(result).kind(),
                    ErrorKind::Unsupported(Operation::Write),
                    "{query}"
                );
                let probes = factory.probes();
                assert!(
                    probes.first().expect("transport").sent().is_empty(),
                    "{query}: nothing sent"
                );
            }
        }
    }
}
