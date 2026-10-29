//! `ModbusConnection::read` over `FakeTransportFactory`, `FakeTransport` and `FakeClock`:
//! batching, the disclosure of splits, mixed success, byte order precedence and the checks
//! made before any I/O.
//!
//! The wire bytes are written out by hand from the MBAP layout (transaction id, protocol
//! id 0, length, unit id, PDU, all big-endian) and the PDU examples of the Modbus
//! Application Protocol Specification V1.1b sections 6.1, 6.3 and 7; no byte comes from the
//! codec under test. Timing is on the fake clock only (decision D21).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection, UnitId};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, FixtureAddress, Script, Step, block_on_with,
    };
    use stonecrab::{
        AddressOptions, ByteOrder, Clock, Connection, ConnectionConfig, ConnectionConfigBuilder,
        ConnectionState, Endpoint, ErrorKind, FailureCategory, Operation, PlcValue, PlcValueType,
        ReadRequest, ReadResponse, ResourceLimits, Result, SubscriptionMode, Tag, TagId, WordOrder,
        WriteRequest,
    };

    fn millis(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    /// An ADU as it goes on the wire.
    fn adu(txn: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let length = u16::try_from(pdu.len().saturating_add(1)).expect("a short PDU");
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0]);
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(unit);
        bytes.extend_from_slice(pdu);
        bytes
    }

    /// A read request: function code, wire start address and quantity.
    fn read_request(txn: u16, unit: u8, function: u8, start: u16, quantity: u16) -> Vec<u8> {
        let mut pdu = vec![function];
        pdu.extend_from_slice(&start.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        adu(txn, unit, &pdu)
    }

    /// A register reply: function code, byte count and the registers.
    fn registers_reply(txn: u16, unit: u8, function: u8, registers: &[u16]) -> Vec<u8> {
        let count = u8::try_from(registers.len().saturating_mul(2)).expect("at most 125");
        let mut pdu = vec![function, count];
        for register in registers {
            pdu.extend_from_slice(&register.to_be_bytes());
        }
        adu(txn, unit, &pdu)
    }

    /// A bit reply: function code, byte count and the packed bytes.
    fn bits_reply(txn: u16, unit: u8, function: u8, packed: &[u8]) -> Vec<u8> {
        let count = u8::try_from(packed.len()).expect("at most 250");
        let mut pdu = vec![function, count];
        pdu.extend_from_slice(packed);
        adu(txn, unit, &pdu)
    }

    /// An exception reply: the function code with the high bit set, then the code.
    fn exception_reply(txn: u16, unit: u8, function: u8, code: u8) -> Vec<u8> {
        adu(txn, unit, &[function | 0x80, code])
    }

    fn tag(id: &str, address: &str) -> Tag {
        let address = ModbusAddress::parse(address, &AddressOptions::default()).expect("address");
        Tag::new(TagId::new(id).expect("tag id"), Arc::new(address))
    }

    fn base_config() -> ConnectionConfigBuilder {
        ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        /// An open connection with the default configuration to a peer running `steps`.
        fn open(steps: Vec<Step>) -> Self {
            let config = base_config().build().expect("config");
            Self::with(config, ConnectionDefaults::default(), vec![steps])
        }

        /// An open connection to peers running `scripts`, one per transport.
        fn with(
            config: ConnectionConfig,
            defaults: ConnectionDefaults,
            scripts: Vec<Vec<Step>>,
        ) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            for steps in scripts {
                factory.push_script(Script::strict(steps));
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

        fn request(&self, tags: Vec<Tag>) -> ReadRequest {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            builder.build().expect("request")
        }

        fn read(&self, tags: Vec<Tag>) -> Result<ReadResponse> {
            block_on_with(&self.clock, self.connection.read(self.request(tags)))
        }

        /// The frames written to the `transport`th connection.
        fn sent(&self, transport: usize) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .get(transport)
                .expect("transport was opened")
                .sent()
                .iter()
                .map(|frame| frame.to_vec())
                .collect()
        }
    }

    fn value_of(response: &ReadResponse, id: &str) -> PlcValue {
        let id = TagId::new(id).expect("tag id");
        response
            .get(&id)
            .expect("the item")
            .outcome()
            .as_ref()
            .expect("a value")
            .clone()
    }

    fn error_of<T>(result: Result<T>) -> stonecrab::PlcError {
        result.err().expect("an error")
    }

    /// Replies to every holding-register read with the register number as its value, and to
    /// every coil read with a bit set where the bit number is a multiple of three.
    fn server() -> Step {
        Step::respond(|request| {
            let txn = u16::from_be_bytes(request.get(0..2)?.try_into().ok()?);
            let unit = *request.get(6)?;
            let function = *request.get(7)?;
            let start = u16::from_be_bytes(request.get(8..10)?.try_into().ok()?);
            let quantity = u16::from_be_bytes(request.get(10..12)?.try_into().ok()?);
            if function == 0x03 {
                let registers: Vec<u16> = (0..quantity).map(|i| start.wrapping_add(i)).collect();
                Some(registers_reply(txn, unit, function, &registers))
            } else {
                let mut packed = vec![0_u8; usize::from(quantity).div_ceil(8)];
                for bit in 0..quantity {
                    if start.wrapping_add(bit) % 3 == 0 {
                        let slot = packed.get_mut(usize::from(bit >> 3))?;
                        *slot |= 1 << (bit & 7);
                    }
                }
                Some(bits_reply(txn, unit, function, &packed))
            }
        })
    }

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting
    #[test]
    fn a_read_decodes_the_v11b_example_in_one_chunk_with_the_exact_request_bytes() {
        // V1.1b section 6.3: registers 108 to 110 of the device hold 555, 0 and 100.
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x03, 107, 3)),
            Step::Reply(registers_reply(1, 1, 0x03, &[555, 0, 100])),
        ]);
        let response = rig
            .read(vec![
                tag("a", "holding-register:108"),
                tag("b", "holding-register:109"),
                tag("c", "holding-register:110"),
            ])
            .expect("the response");
        assert_eq!(value_of(&response, "a"), PlcValue::I16(555), "a");
        assert_eq!(value_of(&response, "b"), PlcValue::I16(0), "b");
        assert_eq!(value_of(&response, "c"), PlcValue::I16(100), "c");
        assert_eq!(
            response.split_into().get(),
            1,
            "adjacent tags are one chunk"
        );
        assert_eq!(
            rig.sent(0),
            vec![vec![0, 1, 0, 0, 0, 6, 1, 3, 0x00, 0x6B, 0x00, 0x03]],
            "one request on the wire, byte for byte"
        );
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Connected,
            "still connected"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success (split disclosure)
    #[test]
    fn two_areas_split_into_two_chunks_and_every_tag_appears_once_in_request_order() {
        let rig = Rig::open(vec![
            // The coil chunk comes first: chunks are ordered by area, then address.
            Step::Expect(read_request(1, 1, 0x01, 0, 1)),
            Step::Reply(bits_reply(1, 1, 0x01, &[0x01])),
            Step::Expect(read_request(2, 1, 0x03, 9, 1)),
            Step::Reply(registers_reply(2, 1, 0x03, &[77])),
        ]);
        let response = rig
            .read(vec![
                tag("register", "holding-register:10"),
                tag("coil", "coil:1"),
            ])
            .expect("the response");
        assert_eq!(response.split_into().get(), 2, "two chunks disclosed");
        let ids: Vec<&str> = response
            .items()
            .iter()
            .map(|item| item.tag().as_str())
            .collect();
        assert_eq!(
            ids,
            ["register", "coil"],
            "every tag once, in request order"
        );
        assert_eq!(
            value_of(&response, "register"),
            PlcValue::I16(77),
            "register"
        );
        assert_eq!(value_of(&response, "coil"), PlcValue::Bool(true), "coil");
        assert_eq!(rig.sent(0).len(), 2, "one frame per chunk");
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success (split disclosure)
    #[test]
    fn a_63_dint_span_splits_at_125_registers() {
        // 63 DINTs are 126 registers: 62 fit a chunk (124 registers), the last one splits.
        let rig = Rig::open(vec![server()]);
        let tags = (0..63_u16)
            .map(|i| {
                tag(
                    &format!("t{i}"),
                    &format!("holding-register:{}:DINT", 1 + 2 * u32::from(i)),
                )
            })
            .collect();
        let response = rig.read(tags).expect("the response");
        assert_eq!(response.split_into().get(), 2, "two chunks");
        assert_eq!(response.items().len(), 63, "every tag once");
        let sent = rig.sent(0);
        assert_eq!(
            sent,
            vec![
                read_request(1, 1, 0x03, 0, 124),
                read_request(2, 1, 0x03, 124, 2)
            ],
            "124 registers, then the last DINT"
        );
        // The register at wire address r holds r, so DINT k is (2k << 16) | (2k + 1).
        assert_eq!(value_of(&response, "t0"), PlcValue::I32(1), "first DINT");
        assert_eq!(
            value_of(&response, "t62"),
            PlcValue::I32(0x007C_007D),
            "the DINT in the second chunk"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success (split disclosure)
    #[test]
    fn exactly_125_registers_fit_one_chunk_and_the_126th_splits() {
        let rig = Rig::open(vec![server()]);
        let tags = (0..125_u16)
            .map(|i| {
                tag(
                    &format!("t{i}"),
                    &format!("holding-register:{}", 1 + u32::from(i)),
                )
            })
            .collect();
        let response = rig.read(tags).expect("the response");
        assert_eq!(
            response.split_into().get(),
            1,
            "125 registers are one chunk"
        );

        let tags = (0..126_u16)
            .map(|i| {
                tag(
                    &format!("t{i}"),
                    &format!("holding-register:{}", 1 + u32::from(i)),
                )
            })
            .collect();
        let response = rig.read(tags).expect("the response");
        assert_eq!(response.split_into().get(), 2, "126 registers are two");
        let sent = rig.sent(0);
        let quantities: Vec<u16> = sent
            .iter()
            .map(|frame| {
                u16::from_be_bytes(
                    frame
                        .get(10..12)
                        .expect("quantity")
                        .try_into()
                        .expect("two"),
                )
            })
            .collect();
        assert_eq!(quantities, [125, 125, 1], "the wire quantities");
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success (split disclosure)
    #[test]
    fn coils_beyond_2000_split_into_a_second_chunk() {
        let limits = ResourceLimits::builder()
            .max_items_per_request(2_100)
            .build()
            .expect("limits");
        let config = base_config().limits(limits).build().expect("config");
        let rig = Rig::with(config, ConnectionDefaults::default(), vec![vec![server()]]);
        let tags = (0..2_001_u32)
            .map(|i| tag(&format!("c{i}"), &format!("coil:{}", 1 + i)))
            .collect();
        let response = rig.read(tags).expect("the response");
        assert_eq!(response.split_into().get(), 2, "2000 coils, then one");
        assert_eq!(
            rig.sent(0),
            vec![
                read_request(1, 1, 0x01, 0, 2000),
                read_request(2, 1, 0x01, 2000, 1)
            ],
            "the wire"
        );
        // Bit n is set where n is a multiple of three: coil 1 is wire bit 0.
        assert_eq!(value_of(&response, "c0"), PlcValue::Bool(true), "bit 0");
        assert_eq!(value_of(&response, "c1"), PlcValue::Bool(false), "bit 1");
        assert_eq!(value_of(&response, "c3"), PlcValue::Bool(true), "bit 3");
        assert_eq!(
            value_of(&response, "c2000"),
            PlcValue::Bool(false),
            "bit 2000 is not a multiple of three"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success, unsupported capability
    #[test]
    fn a_chunk_exception_fails_only_its_members_and_the_next_chunk_still_runs() {
        let rig = Rig::open(vec![
            // The coil chunk is refused: exception 02, ILLEGAL DATA ADDRESS.
            Step::Expect(read_request(1, 1, 0x01, 0, 1)),
            Step::Reply(exception_reply(1, 1, 0x01, 0x02)),
            Step::Expect(read_request(2, 1, 0x03, 0, 2)),
            Step::Reply(registers_reply(2, 1, 0x03, &[5, 6])),
        ]);
        let response = rig
            .read(vec![
                tag("coil", "coil:1"),
                tag("first", "holding-register:1"),
                tag("second", "holding-register:2"),
            ])
            .expect("the read as a whole succeeds");
        assert_eq!(response.split_into().get(), 2, "two chunks");
        assert_eq!(value_of(&response, "first"), PlcValue::I16(5), "first");
        assert_eq!(value_of(&response, "second"), PlcValue::I16(6), "second");
        let refused = response
            .get(&TagId::new("coil").expect("id"))
            .expect("item")
            .outcome()
            .as_ref()
            .expect_err("the coil was refused");
        assert_eq!(refused.kind(), ErrorKind::InvalidAddress, "kind");
        let status = refused.status().expect("status");
        assert_eq!(
            (status.protocol(), status.code(), status.name()),
            ("modbus", 2, Some("ILLEGAL DATA ADDRESS")),
            "the raw code and the name survive"
        );
        let diagnostics = refused.error().diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "an exception leaves the connection connected"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Protocol),
            "category"
        );
        assert_eq!(rig.sent(0).len(), 2, "both chunks were requested");
    }

    // covers: MB-READ-GAP
    #[test]
    fn a_hole_between_two_tags_is_never_read_so_it_cannot_fail_either() {
        // Registers 3 to 9 are not mapped in this device. Reading through the hole as one
        // ten-register request would be refused with exception 02 and fail all three tags;
        // two requests that stop at the tags' own registers both succeed.
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x03, 0, 2)),
            Step::Reply(registers_reply(1, 1, 0x03, &[5, 6])),
            Step::Expect(read_request(2, 1, 0x03, 9, 1)),
            Step::Reply(registers_reply(2, 1, 0x03, &[7])),
        ]);
        let response = rig
            .read(vec![
                tag("first", "holding-register:1"),
                tag("second", "holding-register:2"),
                tag("far", "holding-register:10"),
            ])
            .expect("the read succeeds");
        assert_eq!(response.split_into().get(), 2, "the hole splits the read");
        assert_eq!(value_of(&response, "first"), PlcValue::I16(5), "first");
        assert_eq!(value_of(&response, "second"), PlcValue::I16(6), "second");
        assert_eq!(value_of(&response, "far"), PlcValue::I16(7), "far");
        assert_eq!(
            rig.sent(0),
            [
                read_request(1, 1, 0x03, 0, 2),
                read_request(2, 1, 0x03, 9, 1)
            ],
            "no request covers an address no tag asked for"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success, device exception
    #[test]
    fn an_exception_in_the_last_chunk_fails_every_member_of_that_chunk() {
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x01, 0, 1)),
            Step::Reply(bits_reply(1, 1, 0x01, &[0x00])),
            // Two adjacent registers share the refused chunk: exception 04.
            Step::Expect(read_request(2, 1, 0x03, 0, 2)),
            Step::Reply(exception_reply(2, 1, 0x03, 0x04)),
        ]);
        let response = rig
            .read(vec![
                tag("coil", "coil:1"),
                tag("first", "holding-register:1"),
                tag("second", "holding-register:2"),
            ])
            .expect("the read as a whole succeeds");
        assert_eq!(value_of(&response, "coil"), PlcValue::Bool(false), "coil");
        for id in ["first", "second"] {
            let error = response
                .get(&TagId::new(id).expect("id"))
                .expect("item")
                .outcome()
                .as_ref()
                .expect_err("refused");
            assert_eq!(error.kind(), ErrorKind::DeviceException, "{id}: kind");
            let status = error.status().expect("status");
            assert_eq!(
                (status.code(), status.name()),
                (4, Some("SERVER DEVICE FAILURE")),
                "{id}: raw code and name"
            );
        }
        assert_eq!(response.iter_failed().count(), 2, "exactly the two members");
        assert_eq!(response.iter_ok().count(), 1, "and the coil is fine");
    }

    // covers: SPEC mandatory-failure-cases / F3 mixed success, unsupported capability
    #[test]
    fn illegal_function_is_unsupported_read_for_the_members() {
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x04, 0, 1)),
            Step::Reply(exception_reply(1, 1, 0x04, 0x01)),
        ]);
        let response = rig
            .read(vec![tag("level", "input-register:1")])
            .expect("the response");
        let error = response
            .items()
            .first()
            .expect("item")
            .outcome()
            .as_ref()
            .expect_err("refused");
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Read),
            "code 1 is an unsupported capability"
        );
    }

    // covers: SPEC mandatory-failure-cases / F6 timestamp absence
    #[test]
    fn source_time_is_none_and_sampled_at_is_the_instant_each_chunk_was_answered() {
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x01, 0, 1)),
            Step::Delay(millis(250)),
            Step::Reply(bits_reply(1, 1, 0x01, &[0x01])),
            Step::Expect(read_request(2, 1, 0x03, 0, 1)),
            Step::Delay(millis(400)),
            Step::Reply(registers_reply(2, 1, 0x03, &[9])),
        ]);
        let response = rig
            .read(vec![
                tag("coil", "coil:1"),
                tag("register", "holding-register:1"),
            ])
            .expect("the response");
        let [coil, register] = response.items() else {
            panic!("two items");
        };
        assert_eq!(coil.source_time(), None, "no device timestamp (coil)");
        assert_eq!(
            register.source_time(),
            None,
            "no device timestamp (register)"
        );
        assert_eq!(response.started_at(), rig.t0, "started when the read began");
        assert!(
            coil.sampled_at() > response.started_at(),
            "sampled after the start"
        );
        assert_eq!(
            coil.sampled_at(),
            rig.t0 + millis(250),
            "the instant the first chunk's reply arrived"
        );
        assert_eq!(
            register.sampled_at(),
            rig.t0 + millis(650),
            "the instant the second chunk's reply arrived"
        );
        assert_eq!(response.finished_at(), rig.t0 + millis(650), "finished");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register (count above one)
    #[test]
    fn a_count_above_one_is_invalid_config_and_nothing_is_sent() {
        let rig = Rig::open(vec![server()]);
        let counted = tag("array", "holding-register:1")
            .with_count(std::num::NonZeroU32::new(4).expect("four"));
        let error = error_of(rig.read(vec![counted]));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        assert!(
            error.to_string().contains("array"),
            "names the tag: {error}"
        );
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert!(rig.sent(0).is_empty(), "nothing reached the wire");
        assert_eq!(rig.connection.state(), ConnectionState::Connected, "state");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid address
    #[test]
    fn a_foreign_address_is_invalid_address_naming_the_tag_and_nothing_is_sent() {
        let rig = Rig::open(vec![server()]);
        let foreign = Tag::new(
            TagId::new("alien").expect("id"),
            Arc::new(FixtureAddress::new(
                "secret-address-text",
                Some(PlcValueType::I16),
            )),
        );
        let error = error_of(rig.read(vec![foreign]));
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind");
        let text = error.to_string();
        assert!(text.contains("alien"), "names the tag id: {text}");
        assert!(
            !text.contains("secret-address-text"),
            "never the address text: {text}"
        );
        assert!(rig.sent(0).is_empty(), "nothing reached the wire");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid address
    #[test]
    fn a_foreign_address_is_found_before_a_count_above_one() {
        let rig = Rig::open(vec![server()]);
        let counted = tag("array", "holding-register:1")
            .with_count(std::num::NonZeroU32::new(2).expect("two"));
        let foreign = Tag::new(
            TagId::new("alien").expect("id"),
            Arc::new(FixtureAddress::new("x", None)),
        );
        let error = error_of(rig.read(vec![counted, foreign]));
        assert_eq!(
            error.kind(),
            ErrorKind::InvalidAddress,
            "every address is checked before any count"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 request over the connection's own limits
    #[test]
    fn a_request_over_the_connections_item_limit_is_a_resource_limit_with_nothing_sent() {
        let limits = ResourceLimits::builder()
            .max_items_per_request(2)
            .build()
            .expect("limits");
        let config = base_config().limits(limits).build().expect("config");
        let rig = Rig::with(config, ConnectionDefaults::default(), vec![vec![server()]]);
        // The request was built against roomier limits than the connection's own.
        let roomy = ResourceLimits::builder().build().expect("limits");
        let mut builder = ReadRequest::builder(roomy);
        for (id, n) in [("a", 1), ("b", 2), ("c", 3)] {
            builder = builder.tag(tag(id, &format!("holding-register:{n}")));
        }
        let request = builder.build().expect("request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.read(request)));
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "kind");
        assert!(rig.sent(0).is_empty(), "nothing reached the wire");
    }

    // covers: SPEC mandatory-failure-cases / F3 unsupported capability; SPEC connection and
    // resource behavior / read-only policy
    #[test]
    fn a_read_only_write_is_refused_with_nothing_sent_and_the_capabilities_say_so() {
        let rig = Rig::open(vec![server()]);
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("level", "holding-register:1"), PlcValue::I16(1))
            .build()
            .expect("write request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.write(request)));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "kind"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Write),
            "operation"
        );
        assert!(rig.sent(0).is_empty(), "nothing reached the wire");

        let capabilities = rig.connection.capabilities();
        assert!(capabilities.read, "read");
        assert!(
            capabilities.write,
            "the driver can write; the policy decides"
        );
        assert_eq!(
            capabilities.subscribe,
            SubscriptionMode::Unsupported,
            "subscribe"
        );
        assert!(
            !capabilities.browse && !capabilities.discover,
            "browse, discover"
        );
        assert_eq!(
            capabilities.max_pdu_bytes.map(std::num::NonZeroUsize::get),
            Some(253),
            "PDU"
        );
        assert_eq!(capabilities.concurrency.get(), 1, "concurrency");
        assert!(capabilities.supports(Operation::Write), "supports(Write)");
    }

    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn a_tag_with_its_own_byte_order_decodes_differently_from_the_default() {
        // Registers [1, 2] twice: the first DINT uses the default (big-endian, high word
        // first), the second its own LITTLE_ENDIAN pair.
        let rig = Rig::open(vec![
            Step::Expect(read_request(1, 1, 0x03, 0, 4)),
            Step::Reply(registers_reply(1, 1, 0x03, &[1, 2, 1, 2])),
        ]);
        let little = tag("little", "holding-register:3:DINT")
            .with_byte_order(ByteOrder::LittleEndian)
            .with_word_order(WordOrder::LowWordFirst);
        let response = rig
            .read(vec![tag("default", "holding-register:1:DINT"), little])
            .expect("the response");
        assert_eq!(
            value_of(&response, "default"),
            PlcValue::I32(0x0001_0002),
            "BIG_ENDIAN: bytes 00 01 00 02"
        );
        assert_eq!(
            value_of(&response, "little"),
            PlcValue::I32(0x0200_0100),
            "LITTLE_ENDIAN: the same bytes reversed, 02 00 01 00"
        );
        assert_eq!(response.split_into().get(), 1, "one chunk");
    }

    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn tag_field_beats_address_block_beats_the_connection_default() {
        let mut defaults = ConnectionDefaults::default();
        defaults.byte_order = ByteOrder::LittleEndian;
        defaults.word_order = WordOrder::LowWordFirst;
        let config = base_config().build().expect("config");
        let rig = Rig::with(
            config,
            defaults,
            vec![vec![
                Step::Expect(read_request(1, 1, 0x03, 0, 6)),
                Step::Reply(registers_reply(1, 1, 0x03, &[1, 2, 1, 2, 1, 2])),
            ]],
        );
        // The connection default is LITTLE_ENDIAN here. The address block names
        // BIG_ENDIAN for the second tag; the third also has the tag's own LITTLE_ENDIAN.
        let by_block = tag("block", "holding-register:3:DINT{byte-order: 'BIG_ENDIAN'}");
        let by_tag = tag("field", "holding-register:5:DINT{byte-order: 'BIG_ENDIAN'}")
            .with_byte_order(ByteOrder::LittleEndian)
            .with_word_order(WordOrder::LowWordFirst);
        let response = rig
            .read(vec![
                tag("default", "holding-register:1:DINT"),
                by_block,
                by_tag,
            ])
            .expect("the response");
        assert_eq!(
            value_of(&response, "default"),
            PlcValue::I32(0x0200_0100),
            "the connection default applies when nothing else says"
        );
        assert_eq!(
            value_of(&response, "block"),
            PlcValue::I32(0x0001_0002),
            "the address block beats the default"
        );
        assert_eq!(
            value_of(&response, "field"),
            PlcValue::I32(0x0200_0100),
            "the tag field beats the address block"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register (unit routing)
    #[test]
    fn the_unit_comes_from_the_address_else_the_connection_default() {
        let mut defaults = ConnectionDefaults::default();
        defaults.unit_id = UnitId::from(7);
        let config = base_config().build().expect("config");
        let rig = Rig::with(
            config,
            defaults,
            vec![vec![
                // Chunks are ordered by unit (none, the default, comes before 10).
                Step::Expect(read_request(1, 7, 0x03, 0, 1)),
                Step::Reply(registers_reply(1, 7, 0x03, &[11])),
                Step::Expect(read_request(2, 10, 0x03, 0, 1)),
                Step::Reply(registers_reply(2, 10, 0x03, &[22])),
            ]],
        );
        let response = rig
            .read(vec![
                tag("routed", "holding-register:1{unit-id: 10}"),
                tag("plain", "holding-register:1"),
            ])
            .expect("the response");
        assert_eq!(
            value_of(&response, "plain"),
            PlcValue::I16(11),
            "default unit"
        );
        assert_eq!(value_of(&response, "routed"), PlcValue::I16(22), "unit 10");
        assert_eq!(response.split_into().get(), 2, "one chunk per unit");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register (function codes)
    #[test]
    fn each_area_uses_its_function_code_and_bits_unpack_lsb_first() {
        let rig = Rig::open(vec![
            // Coils 1 to 3 read as 0b101, discrete input 2 as set, then the registers.
            Step::Expect(read_request(1, 1, 0x01, 0, 3)),
            Step::Reply(bits_reply(1, 1, 0x01, &[0b0000_0101])),
            Step::Expect(read_request(2, 1, 0x02, 1, 1)),
            Step::Reply(bits_reply(2, 1, 0x02, &[0x01])),
            Step::Expect(read_request(3, 1, 0x03, 0, 1)),
            Step::Reply(registers_reply(3, 1, 0x03, &[0xFFFE])),
            Step::Expect(read_request(4, 1, 0x04, 0, 2)),
            Step::Reply(registers_reply(4, 1, 0x04, &[0x4049, 0x0FDB])),
        ]);
        let response = rig
            .read(vec![
                tag("ir", "input-register:1:REAL"),
                tag("c1", "coil:1"),
                tag("c2", "coil:2"),
                tag("c3", "coil:3"),
                tag("di", "discrete-input:2"),
                tag("hr", "holding-register:1:UINT"),
            ])
            .expect("the response");
        assert_eq!(response.split_into().get(), 4, "one chunk per area");
        assert_eq!(value_of(&response, "c1"), PlcValue::Bool(true), "c1");
        assert_eq!(value_of(&response, "c2"), PlcValue::Bool(false), "c2");
        assert_eq!(value_of(&response, "c3"), PlcValue::Bool(true), "c3");
        assert_eq!(value_of(&response, "di"), PlcValue::Bool(true), "di");
        assert_eq!(value_of(&response, "hr"), PlcValue::U16(0xFFFE), "hr");
        assert_eq!(
            value_of(&response, "ir"),
            PlcValue::F32(f32::from_bits(0x4049_0FDB)),
            "REAL"
        );
        assert!(
            rig.factory
                .probes()
                .first()
                .expect("transport")
                .report()
                .mismatches
                .is_empty(),
            "every request matched the script"
        );
    }

    #[test]
    fn debug_shows_the_endpoint_and_state_but_no_payload() {
        let rig = Rig::open(vec![]);
        let shown = format!("{:?}", rig.connection);
        assert!(shown.contains("ModbusConnection"), "type: {shown}");
        assert!(shown.contains("fake://plc"), "endpoint: {shown}");
        assert!(shown.contains("Connected"), "state: {shown}");
    }
}
