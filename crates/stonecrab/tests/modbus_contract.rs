//! The common driver contract suite (slice S13) run against `modbus-tcp` over the fake
//! transport (slice S25b).
//!
//! The suite lives in the testkit and runs through `&dyn Driver`. `ModbusTcpDriver` is
//! built with the fake clock and a `FakeTransportFactory` that serves a fresh
//! `FakeTransport` for every connect, each answering through a first-party MBAP responder
//! (`tests/support/responder.rs`) over the interop register map. Everything runs on the
//! testkit's `FakeClock`, so no case waits on real time (`CODING_STANDARDS` 13.4).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
mod responder;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::sync::Arc;

    use stonecrab::modbus::{ExceptionCode, ModbusTcpDriver};
    use stonecrab::testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, FakeClock,
        FakeTransportFactory, FixtureAddress, InteropMap, Reaction, Script, Step, block_on_with,
    };
    use stonecrab::{
        Connection, ConnectionState, Driver, ErrorKind, FailureCategory, Operation, PlcValue,
        ProtocolStatus, ReadRequest, ReadResponse, Tag, TagId,
    };

    use crate::responder::Device;

    /// The connection string of the run; the fake factory ignores the endpoint.
    const URI: &str = "modbus-tcp://127.0.0.1:5020";

    /// The same with writes granted, as the suite needs for a driver that supports write.
    const WRITE_URI: &str = "modbus-tcp://127.0.0.1:5020?access=read-write";

    /// Transports the factory can hand out: the suite connects well under a hundred times
    /// (a connect per case, the retry cases, the cleanup after each armed fault). A run
    /// that needed more would fail loudly with `Internal`, not hang.
    const PLANS: usize = 256;

    /// A readable holding register: logical 13 is the UINT 42424 of `MB-INTEROP-HR-04`.
    const READABLE: &str = "holding-register:13:UINT";

    /// An address that parses and lies far outside the 201-register block, so the device
    /// answers exception 02 (ILLEGAL DATA ADDRESS).
    const REJECTED: &str = "holding-register:60000:UINT";

    /// A holding register of the write area 100 to 200, which starts at zero
    /// (`MB-INTEROP-COIL`, the interop register map).
    const WRITABLE: &str = "holding-register:100:UINT";

    const UNSUPPORTED_WRITE: &str = "an unsupported write performs no I/O";
    const DISCONNECT_READ: &str = "a disconnect during a read yields Disconnected";
    const DISCONNECT_DIAGNOSTICS: &str =
        "a disconnect carries diagnostics naming operation, state and category";
    const READ_ONLY_POLICY: &str = "a read-only policy rejects a write with no I/O";
    const UNKNOWN_NOT_REPLAYED: &str = "an unknown write outcome is never replayed";
    const UNSUPPORTED_PING: &str = "an unsupported ping performs no I/O and changes no state";
    const SUPPORTED_PING: &str = "a supported ping answers and changes no state";
    const SILENT_PING: &str = "a ping to a silent device yields Timeout";
    const UNSUPPORTED_SUBSCRIBE: &str =
        "an unsupported subscribe performs no I/O and changes no state";

    /// The subscription cases (S55): the driver polls, on the fake clock.
    const SUBSCRIPTION_CASES: [&str; 6] = [
        "a cyclic subscription delivers one event per tick and discloses its interval",
        "a change-of-state subscription delivers a first value and then only changes",
        "a polling driver refuses an event subscription with no I/O",
        "unsubscribing stops the polling reads",
        "closing the connection ends its subscriptions after the queued events",
        "a stalled polling read skips the ticks that came due and never bursts",
    ];

    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// The driver, the fake world behind it and the fixture that wires the suite to it.
    struct Bench {
        clock: Arc<FakeClock>,
        factory: FakeTransportFactory,
        device: Device,
        driver: ModbusTcpDriver,
        fixture: ContractFixture,
    }

    /// Frames sent across every transport the factory created, plus the opens: the I/O
    /// the "no I/O" cases say must not move.
    fn io_of(factory: &FakeTransportFactory) -> u64 {
        let frames: usize = factory
            .probes()
            .iter()
            .map(|probe| probe.sent().len())
            .sum();
        u64::try_from(frames.saturating_add(factory.opens().len())).unwrap_or(u64::MAX)
    }

    impl Bench {
        fn new() -> Self {
            let clock = Arc::new(FakeClock::default());
            let device = Device::from_map(&register_map());
            let factory = FakeTransportFactory::new(FakeClock::clone(&clock));
            for _ in 0..PLANS {
                factory.push_script(Script::new(vec![Step::React(device.reactor())]));
            }
            let driver = ModbusTcpDriver::new()
                .with_clock(Arc::<FakeClock>::clone(&clock))
                .with_transport_factory(Arc::new(factory.clone()));
            let probe_factory = factory.clone();
            let counter = device.clone();
            let timeout = device.clone();
            let exception = device.clone();
            let disconnect = device.clone();
            let unknown = device.clone();
            let fixture = ContractFixture::new(
                Arc::<FakeClock>::clone(&clock),
                READABLE,
                REJECTED,
                Arc::new(FixtureAddress::new("elsewhere", None)),
                move || io_of(&probe_factory),
                move || counter.reads(),
            )
            .with_writable(WRITABLE, PlcValue::U16(1))
            .with_inject_unknown_after_transmit(move || unknown.arm_apply_then_swallow())
            .with_inject_disconnect(move || disconnect.arm_disconnect())
            .with_inject_timeout(move || timeout.arm_swallow())
            .with_inject_device_exception(move || {
                exception.arm_exception(ExceptionCode::ServerDeviceFailure);
            });
            Self {
                clock,
                factory,
                device,
                driver,
                fixture,
            }
        }

        fn run(&self, uri: &str) -> ContractReport {
            let report = block_on_with(
                &self.clock,
                ContractSuite::run(&self.driver, uri, &self.fixture),
            )
            .expect("the suite always resolves");
            writeln!(std::io::stdout().lock(), "{uri}\n{report}").expect("stdout");
            report
        }

        /// Connect through the driver, the way the suite does.
        fn connect(&self) -> Box<dyn Connection> {
            let config = self.driver.parse_config(URI).expect("config");
            block_on_with(&self.clock, self.driver.connect(config)).expect("connect")
        }

        fn tag(&self, id: &str, text: &str) -> Tag {
            let config = self.driver.parse_config(URI).expect("config");
            let address = self
                .driver
                .parse_address(text, &config.address_options())
                .expect("address");
            Tag::new(TagId::new(id).expect("tag id"), address)
        }

        fn request(connection: &dyn Connection, tags: Vec<Tag>) -> ReadRequest {
            let mut builder = ReadRequest::builder(connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            builder.build().expect("request")
        }

        fn read(
            &self,
            connection: &dyn Connection,
            tags: Vec<Tag>,
        ) -> stonecrab::Result<ReadResponse> {
            block_on_with(
                &self.clock,
                connection.read(Self::request(connection, tags)),
            )
        }
    }

    // covers: SPEC test-layers / contract suite against modbus-tcp over the fake transport
    #[test]
    fn the_suite_is_green_against_modbus_tcp_over_the_fake_transport() {
        let report = Bench::new().run(WRITE_URI);
        assert!(report.is_green(), "the suite is green:\n{report}");
        assert_eq!(report.failed(), 0, "no failures:\n{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "every case has an outcome:\n{report}"
        );
        assert_eq!(
            report.skipped(),
            3,
            "exactly the unsupported-write, the unsupported-ping and the unsupported-subscribe \
             skips:\n{report}"
        );
        // The driver supports ping, so the unsupported-ping case has nothing to prove here
        // and the two cases of a supported ping run.
        assert_eq!(
            report.outcome(UNSUPPORTED_PING),
            Some(&CaseOutcome::Skipped(
                "driver reports ping supported; the case needs it unsupported".to_owned()
            )),
            "the unsupported-ping case is skipped by name:\n{report}"
        );
        for name in [SUPPORTED_PING, SILENT_PING] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }
        // The driver subscribes by polling (S55): the refusal case has nothing to prove, and
        // the subscription cases run on the fake clock (the timeout hook is there).
        assert_eq!(
            report.outcome(UNSUPPORTED_SUBSCRIBE),
            Some(&CaseOutcome::Skipped(
                "driver reports subscribe supported; the case needs it unsupported".to_owned()
            )),
            "the unsupported-subscribe case is skipped by name:\n{report}"
        );
        for name in SUBSCRIPTION_CASES {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }
        // The write cases need a driver that supports write, and this one does: the
        // read-only policy is refused with no I/O, and a write whose confirmation is lost
        // (the device applied it, the reply is swallowed) is Unknown and applied once.
        for name in [READ_ONLY_POLICY, UNKNOWN_NOT_REPLAYED] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }
        // The responder closes the peer on demand (`Reaction::Close`), so the fixture has
        // the hook and the two disconnect cases run.
        for name in [DISCONNECT_READ, DISCONNECT_DIAGNOSTICS] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }
        assert_eq!(
            report.outcome(UNSUPPORTED_WRITE),
            Some(&CaseOutcome::Skipped(
                "driver reports write supported; the case needs it unsupported".to_owned()
            )),
            "the driver writes, so the unsupported-write case has nothing to prove:\n{report}"
        );
        for name in [
            "a read correlates every tag with its result in request order",
            "a partial failure keeps the successes",
            "an absent deadline uses the connection timeout",
            "a timeout carries diagnostics naming operation, state and category",
            "a device exception carries diagnostics naming operation, state and category",
            "read retry stops after the configured maximum attempts",
            "read retry skips non-retryable kinds",
            "a foreign address is rejected with no I/O",
        ] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name} ran and passed:\n{report}"
            );
        }
    }

    #[test]
    fn the_suite_leaves_plans_unused_and_no_fault_armed() {
        let bench = Bench::new();
        let report = bench.run(WRITE_URI);
        assert!(report.is_green(), "{report}");
        // The capabilities case confirms one write; the unknown case sends one more and the
        // lost confirmation does not make the driver send it again.
        assert_eq!(
            bench.device.applied(),
            2,
            "one confirmed write and one unconfirmed, each applied once and never replayed"
        );
        assert!(
            bench.factory.pending() > 0,
            "the suite did not exhaust the {PLANS} transports it was offered"
        );
        assert_eq!(bench.device.armed(), 0, "every armed fault was used");
        assert!(bench.device.reads() > 0, "the device served reads");
        assert_eq!(
            bench.factory.opens().len(),
            PLANS - bench.factory.pending(),
            "every connect took one transport"
        );
    }

    #[test]
    fn the_suite_reports_a_connection_string_the_driver_rejects_as_failures() {
        let bench = Bench::new();
        let report = bench.run("modbus-tcp:udp://127.0.0.1:5020");
        assert_eq!(report.passed(), 0, "nothing can pass:\n{report}");
        assert!(report.failed() > 0, "failures are reported:\n{report}");
        assert!(!report.is_green(), "the report is not green");
        assert!(bench.factory.opens().is_empty(), "nothing was opened");
    }

    #[test]
    fn the_fixture_address_reads_the_documented_value() {
        let bench = Bench::new();
        let connection = bench.connect();
        let response = bench
            .read(&*connection, vec![bench.tag("level", READABLE)])
            .expect("read");
        assert_eq!(
            response
                .items()
                .first()
                .and_then(|item| item.outcome().as_ref().ok()),
            Some(&PlcValue::U16(42_424)),
            "MB-INTEROP-HR-04: UINT 42424 at logical register 13"
        );
        assert_eq!(bench.device.reads(), 1, "one request reached the device");
    }

    #[test]
    fn the_four_areas_serve_the_map_values_and_an_out_of_range_span_is_refused() {
        let bench = Bench::new();
        let connection = bench.connect();
        let tags = vec![
            bench.tag("coil", "coil:1"),
            bench.tag("coil-off", "coil:2"),
            bench.tag("input", "discrete-input:2"),
            bench.tag("holding", "holding-register:14:DINT"),
            bench.tag("register", "input-register:3:UINT"),
            bench.tag("past-end", REJECTED),
            bench.tag("limit", "holding-register:200:UINT"),
        ];
        let response = bench.read(&*connection, tags).expect("read");
        let value = |id: &str| {
            response
                .get(&TagId::new(id).expect("id"))
                .expect("item")
                .outcome()
                .as_ref()
                .ok()
                .cloned()
        };
        assert_eq!(
            value("coil"),
            Some(PlcValue::Bool(true)),
            "MB-INTEROP-COIL 1"
        );
        assert_eq!(
            value("coil-off"),
            Some(PlcValue::Bool(false)),
            "MB-INTEROP-COIL 2"
        );
        assert_eq!(
            value("input"),
            Some(PlcValue::Bool(true)),
            "MB-INTEROP-DI 2"
        );
        assert_eq!(
            value("holding"),
            Some(PlcValue::I32(-242_442_424)),
            "MB-INTEROP-HR-05 DINT"
        );
        assert_eq!(
            value("register"),
            Some(PlcValue::U16(42_424)),
            "MB-INTEROP-IR UINT"
        );
        assert_eq!(
            value("limit"),
            Some(PlcValue::U16(0)),
            "the last declared holding register (logical 200)"
        );
        let error = response
            .get(&TagId::new("past-end").expect("id"))
            .expect("item")
            .outcome()
            .as_ref()
            .expect_err("outside the block");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind");
        assert_eq!(
            error.status().map(ProtocolStatus::code),
            Some(2),
            "exception 02"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 timeout drives the deadline on the fake clock
    #[test]
    fn a_swallowed_request_times_out_at_the_request_timeout_on_the_fake_clock() {
        let bench = Bench::new();
        let connection = bench.connect();
        let timeout = *bench.driver.parse_config(URI).expect("config").timeouts();
        bench.device.arm_swallow();
        let started = stonecrab::Clock::now(&*bench.clock);
        let error = bench
            .read(&*connection, vec![bench.tag("level", READABLE)])
            .expect_err("no reply");
        let elapsed = stonecrab::Clock::now(&*bench.clock).saturating_duration_since(started);
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_eq!(
            elapsed, timeout.request,
            "the fake clock moved to the deadline"
        );
        assert_eq!(bench.device.reads(), 1, "the swallowed request was counted");
        assert_eq!(bench.device.armed(), 0, "and used the fault");
    }

    #[test]
    fn a_refused_request_is_a_device_exception_with_code_four() {
        let bench = Bench::new();
        let connection = bench.connect();
        bench
            .device
            .arm_exception(ExceptionCode::ServerDeviceFailure);
        let response = bench
            .read(&*connection, vec![bench.tag("level", READABLE)])
            .expect("a refused chunk is a per-item error");
        let error = response
            .items()
            .first()
            .and_then(|item| item.outcome().as_ref().err())
            .expect("refused");
        assert_eq!(error.kind(), ErrorKind::DeviceException);
        assert_eq!(error.status().map(ProtocolStatus::code), Some(4));
        let after = bench
            .read(&*connection, vec![bench.tag("level", READABLE)])
            .expect("the next request is answered");
        assert_eq!(after.iter_ok().count(), 1, "the fault was used once");
    }

    // covers: SPEC mandatory-failure-cases / F5 network outage, through the driver
    #[test]
    fn a_peer_that_closes_yields_disconnected_with_diagnostics_through_the_driver() {
        let bench = Bench::new();
        // The first request of the connection is transaction 1, unit 1, holding register
        // at wire address 12; the peer reads it and closes without answering.
        let factory = FakeTransportFactory::new(FakeClock::clone(&bench.clock));
        factory.push_script(Script::strict(vec![
            Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0, 12, 0, 1]),
            Step::CloseAt(0),
        ]));
        factory.push_script(Script::new(vec![Step::React(bench.device.reactor())]));
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::<FakeClock>::clone(&bench.clock))
            .with_transport_factory(Arc::new(factory.clone()));
        let config = driver.parse_config(URI).expect("config");
        let address = driver
            .parse_address(READABLE, &config.address_options())
            .expect("address");
        let address_again = Arc::<dyn stonecrab::ProtocolAddress>::clone(&address);
        let connection = block_on_with(&bench.clock, driver.connect(config)).expect("connect");
        let watch = connection.watch_state();
        let request = ReadRequest::builder(connection.limits())
            .tag(Tag::new(TagId::new("level").expect("id"), address))
            .build()
            .expect("request");
        let error = block_on_with(&bench.clock, connection.read(request)).expect_err("closed");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read));
        assert_eq!(diagnostics.category, Some(FailureCategory::Transport));
        assert!(matches!(
            diagnostics.connection_state,
            Some(ConnectionState::Reconnecting { .. })
        ));
        let probes = factory.probes();
        let first = probes.first().expect("the first transport");
        let report = first.report();
        assert!(
            report.mismatches.is_empty() && !report.failed,
            "the peer saw exactly the expected request: {:?}",
            report.mismatches
        );
        assert!(
            matches!(watch.current(), ConnectionState::Reconnecting { .. }),
            "the state watch reports the reconnect"
        );
        let again = ReadRequest::builder(connection.limits())
            .tag(Tag::new(TagId::new("level").expect("id"), address_again))
            .build()
            .expect("request");
        let response = block_on_with(&bench.clock, connection.read(again)).expect("recovered");
        assert_eq!(response.iter_ok().count(), 1, "the follow-up read succeeds");
        assert_eq!(
            watch.current(),
            ConnectionState::Connected,
            "the state watch reports the recovery"
        );
    }

    #[test]
    fn the_responder_echoes_ids_and_refuses_what_it_does_not_serve() {
        let device = Device::from_map(&register_map());
        let mut react = device.reactor();
        // Read two holding registers at wire address 12 (logical 13), transaction 0x1234,
        // unit 7: the answer echoes both ids.
        assert_eq!(
            react(&[0x12, 0x34, 0, 0, 0, 6, 7, 3, 0, 12, 0, 2]),
            Reaction::Reply(vec![
                0x12, 0x34, 0, 0, 0, 7, 7, 3, 4, 0xA5, 0xB8, 0xF1, 0x8C
            ])
        );
        // Function 0x2B is not served: ILLEGAL FUNCTION.
        assert_eq!(
            react(&[0, 9, 0, 0, 0, 3, 1, 0x2B, 0]),
            Reaction::Reply(vec![0, 9, 0, 0, 0, 3, 1, 0xAB, 1])
        );
        // Quantity 0 for a known function: ILLEGAL DATA VALUE.
        assert_eq!(
            react(&[0, 10, 0, 0, 0, 6, 1, 3, 0, 0, 0, 0]),
            Reaction::Reply(vec![0, 10, 0, 0, 0, 3, 1, 0x83, 3])
        );
        // A span past the block: ILLEGAL DATA ADDRESS.
        assert_eq!(
            react(&[0, 11, 0, 0, 0, 6, 1, 4, 0, 20, 0, 2]),
            Reaction::Reply(vec![0, 11, 0, 0, 0, 3, 1, 0x84, 2])
        );
        // Bytes that are not a request get no answer and are not counted.
        assert_eq!(react(&[1, 2, 3]), Reaction::Silence);
        assert_eq!(
            react(&[0, 1, 0, 1, 0, 6, 1, 3, 0, 0, 0, 1]),
            Reaction::Silence
        );
        assert_eq!(device.reads(), 4, "only the four requests were counted");
    }

    #[test]
    fn the_responder_applies_writes_and_loses_a_confirmation_on_demand() {
        let device = Device::from_map(&register_map());
        let mut react = device.reactor();
        // Function 06 at wire address 99 (logical 100, the first register of the write
        // area): the echo repeats the request and the register holds the value.
        assert_eq!(
            react(&[0, 1, 0, 0, 0, 6, 1, 6, 0, 99, 0x12, 0x34]),
            Reaction::Reply(vec![0, 1, 0, 0, 0, 6, 1, 6, 0, 99, 0x12, 0x34])
        );
        assert_eq!(device.holding(100), Some(0x1234), "function 06 applied");
        // Function 16 writes two registers; the echo is the start and the quantity.
        assert_eq!(
            react(&[0, 2, 0, 0, 0, 11, 1, 0x10, 0, 100, 0, 2, 4, 0, 0x0A, 1, 2]),
            Reaction::Reply(vec![0, 2, 0, 0, 0, 6, 1, 0x10, 0, 100, 0, 2])
        );
        assert_eq!(
            (device.holding(101), device.holding(102)),
            (Some(0x000A), Some(0x0102)),
            "function 16 applied"
        );
        // Function 15 writes three coils from wire address 99: true, false, true.
        assert_eq!(
            react(&[0, 3, 0, 0, 0, 8, 1, 0x0F, 0, 99, 0, 3, 1, 0b101]),
            Reaction::Reply(vec![0, 3, 0, 0, 0, 6, 1, 0x0F, 0, 99, 0, 3])
        );
        assert_eq!(
            (device.coil(100), device.coil(101), device.coil(102)),
            (Some(true), Some(false), Some(true)),
            "function 15 applied"
        );
        assert_eq!(device.applied(), 3, "three writes applied");
        // A write past the block is ILLEGAL DATA ADDRESS and changes nothing.
        assert_eq!(
            react(&[0, 4, 0, 0, 0, 6, 1, 6, 0xEA, 0x60, 0, 1]),
            Reaction::Reply(vec![0, 4, 0, 0, 0, 3, 1, 0x86, 2])
        );
        assert_eq!(device.applied(), 3, "the refused write was not applied");
        // A lost confirmation: the register changes, the client hears nothing, or the
        // connection closes.
        device.arm_apply_then_swallow();
        assert_eq!(
            react(&[0, 5, 0, 0, 0, 6, 1, 6, 0, 99, 0, 7]),
            Reaction::Silence
        );
        assert_eq!(device.holding(100), Some(7), "applied, then silent");
        device.arm_apply_then_disconnect();
        assert_eq!(
            react(&[0, 6, 0, 0, 0, 6, 1, 6, 0, 99, 0, 8]),
            Reaction::Close
        );
        assert_eq!(device.holding(100), Some(8), "applied, then closed");
        assert_eq!(device.applied(), 5, "five writes applied");
        assert_eq!(device.armed(), 0, "both faults used");
    }

    #[test]
    fn each_fault_maps_to_its_reaction_and_is_used_once() {
        let device = Device::from_map(&register_map());
        let mut react = device.reactor();
        let request = [0, 1, 0, 0, 0, 6, 1, 3, 0, 12, 0, 1];
        device.arm_disconnect();
        device.arm_swallow();
        device.arm_exception(ExceptionCode::ServerDeviceFailure);
        assert_eq!(device.armed(), 3, "three faults queued");
        assert_eq!(
            react(&request),
            Reaction::Close,
            "a disconnect closes, unanswered"
        );
        assert_eq!(react(&request), Reaction::Silence, "a swallow is silent");
        assert_eq!(
            react(&request),
            Reaction::Reply(vec![0, 1, 0, 0, 0, 3, 1, 0x83, 4]),
            "an exception is a refusal"
        );
        assert_eq!(device.armed(), 0, "every fault was used once");
        assert!(
            matches!(react(&request), Reaction::Reply(reply) if reply.len() == 11),
            "the next request is answered"
        );
        assert_eq!(device.reads(), 4, "the disconnect request counts as read");
    }
}
