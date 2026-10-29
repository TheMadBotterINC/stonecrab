//! The driver and connection contracts, exercised through `DriverRegistry` against the
//! simulator on the testkit's `FakeClock` (`CODING_STANDARDS` 13.5: the simulator is the
//! sanctioned double of `Driver` and `Connection`).
#![cfg(all(feature = "sim", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use stonecrab::sim::{SimAddress, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{
        CountingWaker, FakeClock, FixtureAddress, block_on_with, poll_once, poll_once_with,
    };
    use stonecrab::{
        AccessPolicy, AddressOptions, CancellationToken, Capabilities, Clock, Connection,
        ConnectionConfig, ConnectionState, Deadline, Diagnostics, Driver, DriverRegistry, Endpoint,
        ErrorKind, FailureCategory, OpFuture, Operation, PlcError, PlcValue, PlcValueType,
        ProtocolStatus, ReadRequest, ReadResponse, Result, RetryPolicy, SubscriptionMode, Tag,
        TagId, Timeouts, WriteOutcome, WriteRequest,
    };

    const MS: Duration = Duration::from_millis(1);

    /// A simulator on a fake clock with one device, `bench`, and a registry holding it.
    struct Bench {
        clock: Arc<FakeClock>,
        driver: Arc<SimDriver>,
        registry: DriverRegistry,
        /// Counts the reads that reach the `temperature` tag.
        reads: Arc<AtomicU32>,
    }

    fn bench_device(latency: Duration) -> (SimDevice, Arc<AtomicU32>) {
        let device = SimDevice::new().with_latency(latency);
        let (temperature, reads) = SimTag::new(PlcValue::F32(71.5)).with_read_counter();
        device.set("temperature", temperature).expect("temperature");
        device
            .ramp("count", PlcValue::U16(0), PlcValue::U16(1))
            .expect("count");
        device
            .set("setpoint", SimTag::new(PlcValue::I16(20)))
            .expect("setpoint");
        device
            .set("locked", SimTag::new(PlcValue::I16(5)).read_only())
            .expect("locked");
        let mut sealed = SimTag::new(PlcValue::I16(1));
        sealed.readable = false;
        device.set("sealed", sealed).expect("sealed");
        // The stored type (u8) differs from the type the tests' address names (i16).
        device
            .set("typed", SimTag::new(PlcValue::U8(9)))
            .expect("typed");
        (device, reads)
    }

    impl Bench {
        fn new(latency: Duration) -> Self {
            let clock = Arc::new(FakeClock::default());
            let (device, reads) = bench_device(latency);
            let driver = Arc::new(
                SimDriver::new()
                    .with_clock(Arc::<FakeClock>::clone(&clock))
                    .with_device("bench", device)
                    .expect("device"),
            );
            let mut registry = DriverRegistry::new();
            registry
                .register(Arc::<SimDriver>::clone(&driver))
                .expect("register");
            Self {
                clock,
                driver,
                registry,
                reads,
            }
        }

        fn connect(&self) -> Box<dyn Connection> {
            self.connect_uri("sim://bench")
        }

        fn connect_uri(&self, uri: &str) -> Box<dyn Connection> {
            block_on_with(&self.clock, self.registry.connect(uri)).expect("connects")
        }

        fn connect_config(&self, config: ConnectionConfig) -> Result<Box<dyn Connection>> {
            block_on_with(&self.clock, self.driver.connect(config))
        }

        fn run<T>(&self, future: OpFuture<'_, T>) -> Result<T> {
            block_on_with(&self.clock, future)
        }

        fn reads(&self) -> u32 {
            self.reads.load(Ordering::Relaxed)
        }

        fn device(&self) -> Arc<SimDevice> {
            self.driver.device("bench").expect("bench device")
        }
    }

    fn tag(id: &str, address: &str) -> Tag {
        Tag::new(
            TagId::new(id).expect("identity"),
            Arc::new(SimAddress::parse(address).expect("address")),
        )
    }

    fn temperature() -> Tag {
        tag("temperature", "STATE/temperature:f32")
    }

    fn read_request(connection: &dyn Connection, tags: &[Tag]) -> ReadRequest {
        tags.iter()
            .fold(ReadRequest::builder(connection.limits()), |builder, tag| {
                builder.tag(tag.clone())
            })
            .build()
            .expect("valid read")
    }

    fn write_request(connection: &dyn Connection, items: &[(Tag, PlcValue)]) -> WriteRequest {
        items
            .iter()
            .fold(
                WriteRequest::builder(connection.limits()),
                |builder, (tag, value)| builder.item(tag.clone(), value.clone()),
            )
            .build()
            .expect("valid write")
    }

    fn value_of(response: &ReadResponse, id: &str) -> Option<PlcValue> {
        let id = TagId::new(id).expect("identity");
        response
            .get(&id)
            .and_then(|item| item.outcome().as_ref().ok().cloned())
    }

    fn failure<T>(result: Result<T>) -> PlcError {
        result.err().expect("the operation must fail")
    }

    fn assert_diagnostics(
        error: &PlcError,
        operation: Operation,
        state: ConnectionState,
        category: FailureCategory,
    ) {
        let diagnostics: &Diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(operation), "operation");
        assert_eq!(diagnostics.connection_state, Some(state), "state");
        assert_eq!(diagnostics.category, Some(category), "category");
        assert!(diagnostics.latency.is_some(), "latency is measured");
    }

    /// The simulator under another scheme: the only way to put two schemes in one registry
    /// is a second driver, and this wrapper delegates every method to `SimDriver`.
    struct AliasDriver {
        scheme: &'static str,
        inner: SimDriver,
    }

    impl Driver for AliasDriver {
        fn scheme(&self) -> &'static str {
            self.scheme
        }
        fn capabilities(&self) -> Capabilities {
            self.inner.capabilities()
        }
        fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
            self.inner.parse_config(uri)
        }
        fn parse_address(
            &self,
            text: &str,
            options: &AddressOptions,
        ) -> Result<Arc<dyn stonecrab::ProtocolAddress>> {
            self.inner.parse_address(text, options)
        }
        fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
            self.inner.connect(config)
        }
    }

    // ---- registry -------------------------------------------------------------------

    // covers: ALPHA_PLAN S11a / registry registration and lookup
    #[test]
    fn registration_lookup_and_listing() {
        let bench = Bench::new(Duration::ZERO);
        assert_eq!(bench.registry.schemes(), ["sim"], "one scheme");
        let found = bench.registry.driver("sim").expect("registered");
        assert_eq!(found.scheme(), "sim", "the driver answers for its scheme");
        assert!(
            bench.registry.driver("SIM").is_none(),
            "lookup is case-sensitive"
        );
        assert!(bench.registry.driver("modbus-tcp").is_none(), "unknown");
        let shown = format!("{:?}", bench.registry);
        assert!(shown.contains("sim"), "Debug names the schemes: {shown}");
        assert!(
            DriverRegistry::default().schemes().is_empty(),
            "empty default"
        );
    }

    // covers: ALPHA_PLAN S11a / duplicate scheme is InvalidConfig
    #[test]
    fn a_duplicate_scheme_is_invalid_config_and_keeps_the_first_driver() {
        let bench = Bench::new(Duration::ZERO);
        let mut registry = DriverRegistry::new();
        let first: Arc<dyn Driver> = Arc::new(SimDriver::new());
        registry.register(Arc::clone(&first)).expect("first");
        let error = failure(registry.register(Arc::new(SimDriver::new())));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "the registry knows the operation"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "a rejection before I/O"
        );
        let kept = registry.driver("sim").expect("still registered");
        assert!(Arc::ptr_eq(&kept, &first), "the first driver stays");
        assert_eq!(bench.registry.schemes().len(), 1, "independent registries");
    }

    // covers: ALPHA_PLAN S11a / schemes() is sorted
    #[test]
    fn schemes_are_listed_sorted_whatever_the_registration_order() {
        let mut registry = DriverRegistry::new();
        for scheme in ["zulu", "sim", "alpha"] {
            let driver = AliasDriver {
                scheme,
                inner: SimDriver::new(),
            };
            registry
                .register(Arc::new(driver))
                .expect("distinct schemes");
        }
        assert_eq!(registry.schemes(), ["alpha", "sim", "zulu"], "sorted");
    }

    // covers: ALPHA_PLAN S11a / unknown scheme is Unsupported(Connect)
    #[test]
    fn an_unknown_scheme_is_unsupported_connect_from_parse_and_connect() {
        let bench = Bench::new(Duration::ZERO);
        let parse = failure(bench.registry.parse("modbus-tcp://192.0.2.10:502"));
        assert_eq!(
            parse.kind(),
            ErrorKind::Unsupported(Operation::Connect),
            "parse"
        );
        assert_eq!(
            parse.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );
        assert_eq!(
            parse.diagnostics().category,
            Some(FailureCategory::Validation),
            "category"
        );
        let connect = failure(bench.run(bench.registry.connect("modbus-tcp://192.0.2.10:502")));
        assert_eq!(
            connect.kind(),
            ErrorKind::Unsupported(Operation::Connect),
            "connect"
        );
    }

    // This one fails before a driver is involved, so no connection state exists; the
    // registry must not invent one.
    #[test]
    fn registry_errors_do_not_invent_a_connection_state_or_latency() {
        let bench = Bench::new(Duration::ZERO);
        let error = failure(bench.registry.parse("modbus-tcp://192.0.2.10:502"));
        assert_eq!(error.diagnostics().connection_state, None, "no state");
        assert_eq!(error.diagnostics().latency, None, "no latency");
        let connect = failure(bench.run(bench.registry.connect("modbus-tcp://192.0.2.10:502")));
        assert_eq!(
            connect.kind(),
            ErrorKind::Unsupported(Operation::Connect),
            "connect"
        );
    }

    // covers: ALPHA_PLAN S11a / the registry extracts the scheme with ConnectionUri
    #[test]
    fn a_malformed_connection_string_is_invalid_config_from_parse_and_connect() {
        let bench = Bench::new(Duration::ZERO);
        for text in [
            "",
            "not a uri",
            "sim:/bench",
            "sim://",
            "SIM://bench",
            "sim://u:p@bench",
        ] {
            let parse = failure(bench.registry.parse(text));
            assert_eq!(parse.kind(), ErrorKind::InvalidConfig, "parse {text:?}");
            let connect = failure(bench.run(bench.registry.connect(text)));
            assert_eq!(connect.kind(), ErrorKind::InvalidConfig, "connect {text:?}");
        }
    }

    #[test]
    fn parse_returns_the_driver_and_a_configuration_without_opening_anything() {
        let bench = Bench::new(Duration::ZERO);
        let (driver, config) = bench
            .registry
            .parse("sim://bench?latency-ms=5&seed=7&address-convention=plc4x")
            .expect("parses");
        assert_eq!(driver.scheme(), "sim", "driver");
        assert_eq!(config.scheme(), "sim", "scheme");
        assert_eq!(
            config.endpoint(),
            &Endpoint::Custom("bench?latency-ms=5&seed=7".to_owned()),
            "the device name, with the options connect reads back"
        );
        assert_eq!(config.access(), AccessPolicy::ReadWrite, "simulator policy");
        assert_eq!(config.timeouts(), &Timeouts::default(), "default timeouts");
        assert_eq!(
            config.address_options().convention(),
            Some("plc4x"),
            "the shared key is carried"
        );
        assert_eq!(bench.clock.sleeper_count(), 0, "nothing was opened");
        let (_, plain) = bench.registry.parse("sim://bench").expect("plain");
        assert_eq!(
            plain.endpoint(),
            &Endpoint::Custom("bench".to_owned()),
            "the endpoint is the device name when there are no options"
        );
        assert_eq!(plain.address_options().convention(), None, "no convention");
    }

    #[test]
    fn parse_config_rejects_everything_it_does_not_understand() {
        let bench = Bench::new(Duration::ZERO);
        for text in [
            "sim://nobody",
            "sim://bench?bogus=1",
            "sim://bench?latency-ms=abc",
            "sim://bench?latency-ms=-1",
            "sim://bench?latency-ms=3600001",
            "sim://bench?seed=-1",
            "sim://bench?seed=",
            "sim:tcp://bench",
            "modbus-tcp://bench",
        ] {
            let error = failure(bench.driver.parse_config(text));
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{text}");
        }
        let at_limit = bench.driver.parse_config("sim://bench?latency-ms=3600000");
        assert!(at_limit.is_ok(), "one hour is the largest latency");
    }

    #[test]
    fn connect_rejects_configurations_that_are_not_sim_ones() {
        let bench = Bench::new(Duration::ZERO);
        let tcp = Endpoint::Tcp {
            host: "192.0.2.10".to_owned(),
            port: 502,
        };
        let cases = [
            ("sim", tcp),
            ("other", Endpoint::Custom("bench".to_owned())),
            ("sim", Endpoint::Custom("nobody".to_owned())),
            ("sim", Endpoint::Custom("bench?bogus=1".to_owned())),
            ("sim", Endpoint::Custom("has space".to_owned())),
        ];
        for (scheme, endpoint) in cases {
            let config = ConnectionConfig::builder(scheme, endpoint)
                .build()
                .expect("a valid configuration of another driver");
            let error = failure(bench.connect_config(config));
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{scheme}");
        }
    }

    #[test]
    fn parse_address_ignores_the_convention_and_reports_bad_text() {
        let bench = Bench::new(Duration::ZERO);
        let plc4x = AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("options");
        for options in [AddressOptions::default(), plc4x] {
            let address = bench
                .driver
                .parse_address("STATE/boiler:f32", &options)
                .expect("parses");
            assert_eq!(address.protocol(), "sim", "protocol");
            assert_eq!(address.canonical(), "STATE/boiler:f32", "canonical");
            assert_eq!(address.value_type_hint(), Some(PlcValueType::F32), "hint");
        }
        let error = failure(
            bench
                .driver
                .parse_address("STDOUT/log:i16", &AddressOptions::default()),
        );
        assert_eq!(
            error.kind(),
            ErrorKind::InvalidAddress,
            "STDOUT is not SINK"
        );
    }

    #[test]
    fn the_driver_advertises_read_and_write_with_one_operation_at_a_time() {
        let driver = SimDriver::default();
        let capabilities = driver.capabilities();
        assert!(capabilities.read && capabilities.write, "read and write");
        assert_eq!(
            capabilities.subscribe,
            SubscriptionMode::Polling,
            "subscribe"
        );
        assert!(
            !capabilities.browse && !capabilities.discover,
            "browse, discover"
        );
        assert_eq!(capabilities.concurrency.get(), 1, "concurrency");
        assert_eq!(capabilities.max_items_per_request, None, "no device yet");
        assert_eq!(driver.scheme(), "sim", "scheme");
    }

    #[test]
    fn device_names_are_validated_and_unique() {
        let too_long = "n".repeat(65);
        for name in ["", "has space", "a/b", "q?x", too_long.as_str()] {
            let refused = SimDriver::new().with_device(name, SimDevice::new());
            assert_eq!(
                refused.err().map(|e| e.kind()),
                Some(ErrorKind::InvalidConfig),
                "{name:?}"
            );
        }
        let longest = "n".repeat(64);
        let driver = SimDriver::new()
            .with_device(&longest, SimDevice::new())
            .and_then(|d| d.with_device("bench-1_x.y", SimDevice::new()))
            .expect("valid names");
        assert!(driver.device("bench-1_x.y").is_some(), "found");
        assert!(driver.device("other").is_none(), "not found");
        let duplicate = driver.with_device("bench-1_x.y", SimDevice::new());
        assert_eq!(
            duplicate.as_ref().err().map(PlcError::kind),
            Some(ErrorKind::InvalidConfig),
            "duplicate"
        );
        let shown = format!(
            "{:?}",
            SimDriver::new().with_device("bench", SimDevice::new())
        );
        assert!(shown.contains("bench"), "Debug names the devices: {shown}");
    }

    // ---- reads and writes through the registry --------------------------------------

    // covers: ALPHA_PLAN S11b-2 exit / registry.connect("sim://bench") reads and writes
    #[test]
    fn the_registry_connects_reads_state_random_and_a_ramp_and_writes_state_and_sink() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
        let noise = tag("noise", "RANDOM/noise:u32");
        let tags = [
            temperature(),
            tag("count", "STATE/count:u16"),
            noise.clone(),
        ];

        let first = bench
            .run(connection.read(read_request(&*connection, &tags)))
            .expect("first read");
        let expected_noise = SimDevice::new().random(PlcValueType::U32).expect("scalar");
        assert_eq!(
            value_of(&first, "temperature"),
            Some(PlcValue::F32(71.5)),
            "STATE"
        );
        assert_eq!(
            value_of(&first, "count"),
            Some(PlcValue::U16(0)),
            "ramp starts"
        );
        assert_eq!(
            value_of(&first, "noise"),
            Some(expected_noise.clone()),
            "RANDOM, seeded"
        );
        assert_eq!(first.split_into().get(), 1, "never split");
        let ids: Vec<&str> = first.items().iter().map(|i| i.tag().as_str()).collect();
        assert_eq!(ids, ["temperature", "count", "noise"], "request order");
        for item in first.items() {
            assert_eq!(item.source_time(), None, "no device timestamp");
            assert_eq!(item.sampled_at(), bench.clock.now(), "sampled on the clock");
        }

        let second = bench
            .run(connection.read(read_request(&*connection, &tags)))
            .expect("second read");
        assert_eq!(
            value_of(&second, "count"),
            Some(PlcValue::U16(1)),
            "ramp steps"
        );
        assert_ne!(
            value_of(&second, "noise"),
            Some(expected_noise),
            "RANDOM moves"
        );

        let setpoint = tag("setpoint", "STATE/setpoint:i16");
        let log = tag("log", "SINK/log:u8");
        let written = bench
            .run(connection.write(write_request(
                &*connection,
                &[
                    (setpoint.clone(), PlcValue::I16(25)),
                    (log, PlcValue::U8(1)),
                ],
            )))
            .expect("write");
        assert_eq!(written.confirmed_count(), 2, "both confirmed");
        assert_eq!(
            bench.device().sink_writes(),
            1,
            "SINK counted and discarded"
        );
        let after = bench
            .run(connection.read(read_request(&*connection, &[setpoint])))
            .expect("read back");
        assert_eq!(
            value_of(&after, "setpoint"),
            Some(PlcValue::I16(25)),
            "STATE written"
        );
        assert_eq!(bench.reads(), 2, "temperature was read twice");
    }

    #[test]
    fn connections_to_one_device_share_it_and_a_seed_makes_random_reproducible() {
        let bench = Bench::new(Duration::ZERO);
        let one = bench.connect_uri("sim://bench?seed=42");
        let noise = tag("noise", "RANDOM/noise:u64");
        let expected = SimDevice::new()
            .with_seed(42)
            .random(PlcValueType::U64)
            .expect("scalar");
        let read = |connection: &dyn Connection| {
            let response = bench
                .run(connection.read(read_request(connection, std::slice::from_ref(&noise))))
                .expect("read");
            value_of(&response, "noise")
        };
        assert_eq!(read(&*one), Some(expected.clone()), "seeded at connect");
        let two = bench.connect_uri("sim://bench?seed=42");
        assert_eq!(read(&*two), Some(expected), "reseeded at the next connect");

        let setpoint = tag("setpoint", "STATE/setpoint:i16");
        drop(
            bench
                .run(one.write(write_request(
                    &*one,
                    &[(setpoint.clone(), PlcValue::I16(33))],
                )))
                .expect("write through one"),
        );
        let seen = bench
            .run(two.read(read_request(&*two, &[setpoint])))
            .expect("read through two");
        assert_eq!(
            value_of(&seen, "setpoint"),
            Some(PlcValue::I16(33)),
            "shared device"
        );
    }

    // ---- lifecycle -------------------------------------------------------------------

    // covers: ALPHA_PLAN S11b-2 / lifecycle observed through a StateWatch, by value
    #[test]
    fn close_moves_through_closing_to_closed_and_is_observed_by_value() {
        let bench = Bench::new(5 * MS);
        let connection = bench.connect();
        let watch = connection.watch_state();
        assert_eq!(
            watch.current(),
            ConnectionState::Connected,
            "starts connected"
        );

        let mut closing = connection.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "the goodbye takes the latency"
        );
        assert_eq!(connection.state(), ConnectionState::Closing, "state()");
        assert_eq!(
            watch.current(),
            ConnectionState::Closing,
            "the watch agrees"
        );
        let mut changed = watch.changed();
        assert!(
            matches!(
                poll_once(&mut changed),
                Poll::Ready(Ok(ConnectionState::Closing))
            ),
            "the change to Closing is reported"
        );
        bench.run(closing).expect("closes");
        assert_eq!(connection.state(), ConnectionState::Closed, "closed");
        assert!(
            matches!(
                poll_once(&mut watch.changed()),
                Poll::Ready(Ok(ConnectionState::Closed))
            ),
            "the change to Closed is reported"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (operation after close)
    #[test]
    fn operations_after_close_are_disconnected_and_close_is_idempotent() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.run(connection.close()).expect("close");
        bench
            .run(connection.close())
            .expect("a second close succeeds");
        assert_eq!(connection.state(), ConnectionState::Closed, "still closed");

        let read =
            failure(bench.run(connection.read(read_request(&*connection, &[temperature()]))));
        assert_eq!(read.kind(), ErrorKind::Disconnected, "read");
        assert_diagnostics(
            &read,
            Operation::Read,
            ConnectionState::Closed,
            FailureCategory::Transport,
        );
        let write = failure(bench.run(connection.write(write_request(
            &*connection,
            &[(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(1))],
        ))));
        assert_eq!(write.kind(), ErrorKind::Disconnected, "write");
        assert_diagnostics(
            &write,
            Operation::Write,
            ConnectionState::Closed,
            FailureCategory::Transport,
        );
        assert_eq!(bench.reads(), 0, "the device was never read");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (close while waiting)
    #[test]
    fn close_ends_a_waiting_read_with_disconnected_and_no_device_read() {
        let bench = Bench::new(Duration::from_secs(1));
        let connection = bench.connect();
        let mut waiting = connection.read(read_request(&*connection, &[temperature()]));
        assert!(poll_once(&mut waiting).is_pending(), "waits for the device");
        let mut closing = connection.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "the goodbye waits too"
        );
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("closing must end the waiting read");
        };
        let error = failure(outcome);
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Closing,
            FailureCategory::Transport,
        );
        bench.run(closing).expect("closes");
        assert_eq!(bench.reads(), 0, "no device read, no partial response");
    }

    #[test]
    fn close_is_bounded_by_the_request_timeout() {
        let bench = Bench::new(Duration::from_secs(10));
        let connection = bench.connect_uri("sim://bench?latency-ms=1");
        let start = bench.clock.now();
        bench
            .run(connection.close())
            .expect("closes after the 1 ms goodbye");
        assert_eq!(
            bench.clock.now() - start,
            MS,
            "the goodbye took the latency"
        );

        let slow = bench.connect_config(
            ConnectionConfig::builder("sim", Endpoint::Custom("bench?latency-ms=4000".into()))
                .timeouts({
                    let mut timeouts = Timeouts::default();
                    timeouts.connect = Duration::from_secs(5);
                    timeouts.request = Duration::from_secs(2);
                    timeouts
                })
                .build()
                .expect("config"),
        );
        let slow = slow.expect("connects after 4 s");
        let start = bench.clock.now();
        bench
            .run(slow.close())
            .expect("close never fails on latency");
        assert_eq!(
            bench.clock.now() - start,
            Duration::from_secs(2),
            "bounded by timeouts.request"
        );
        assert_eq!(slow.state(), ConnectionState::Closed, "closed anyway");
    }

    // covers: SPEC mandatory-failure-cases / F4 timeouts (connect)
    #[test]
    fn connect_waits_the_latency_and_is_bounded_by_the_connect_timeout() {
        let bench = Bench::new(Duration::ZERO);
        let start = bench.clock.now();
        let connection = bench.connect_uri("sim://bench?latency-ms=100");
        assert_eq!(
            bench.clock.now() - start,
            100 * MS,
            "the connect took the latency"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");

        let start = bench.clock.now();
        let error = failure(bench.run(bench.registry.connect("sim://bench?latency-ms=6000")));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "6 s exceeds the 5 s default"
        );
        assert_eq!(
            bench.clock.now() - start,
            Duration::from_secs(5),
            "the connect waited for its deadline"
        );
        assert_diagnostics(
            &error,
            Operation::Connect,
            ConnectionState::Connecting,
            FailureCategory::Transport,
        );
        assert_eq!(
            error.diagnostics().latency,
            Some(Duration::from_secs(5)),
            "latency"
        );

        let at_the_bound =
            failure(bench.run(bench.registry.connect("sim://bench?latency-ms=5000")));
        assert_eq!(
            at_the_bound.kind(),
            ErrorKind::Timeout,
            "a reply at the deadline is too late"
        );
    }

    #[test]
    fn an_absurd_timeout_is_a_typed_error_not_an_overflow() {
        let bench = Bench::new(Duration::ZERO);
        let mut timeouts = Timeouts::default();
        timeouts.connect = Duration::MAX;
        let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .timeouts(timeouts)
            .build()
            .expect("config");
        let error = failure(bench.connect_config(config));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "connect");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );

        let mut timeouts = Timeouts::default();
        timeouts.request = Duration::MAX;
        let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .timeouts(timeouts)
            .access(AccessPolicy::ReadWrite)
            .build()
            .expect("config");
        let connection = bench.connect_config(config).expect("connects");
        let read =
            failure(bench.run(connection.read(read_request(&*connection, &[temperature()]))));
        assert_eq!(
            read.kind(),
            ErrorKind::InvalidConfig,
            "default read deadline"
        );
        let write = failure(bench.run(connection.write(write_request(
            &*connection,
            &[(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(1))],
        ))));
        assert_eq!(
            write.kind(),
            ErrorKind::InvalidConfig,
            "default write deadline"
        );
        let close = failure(bench.run(connection.close()));
        assert_eq!(close.kind(), ErrorKind::InvalidConfig, "close bound");
        assert_eq!(
            connection.state(),
            ConnectionState::Connected,
            "close changed nothing"
        );
        assert_eq!(bench.reads(), 0, "the device was never read");
    }

    #[test]
    fn an_absurd_device_latency_is_a_typed_error_not_an_overflow() {
        let bench = Bench::new(Duration::MAX);
        let error = failure(bench.run(bench.registry.connect("sim://bench")));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "category"
        );
    }

    // ---- single in-flight operation -----------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F6 queues (zero-length queue: Backpressure)
    #[test]
    fn second_concurrent_read_fails_backpressure_without_device_access() {
        let bench = Bench::new(Duration::from_secs(1));
        let connection = bench.connect();
        let counter = CountingWaker::new();
        let waker = counter.waker();

        let mut first = connection.read(read_request(&*connection, &[temperature()]));
        assert!(
            poll_once_with(&mut first, &waker).is_pending(),
            "first waits"
        );
        assert_eq!(
            bench.clock.sleeper_count(),
            1,
            "the latency sleeper is registered"
        );
        assert_eq!(
            bench.reads(),
            0,
            "the device is read only after the latency"
        );

        let mut second = connection.read(read_request(&*connection, &[temperature()]));
        let Poll::Ready(outcome) = poll_once(&mut second) else {
            panic!("a second read must be refused at once");
        };
        let error = failure(outcome);
        assert_eq!(error.kind(), ErrorKind::Backpressure, "kind");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        let mut write = connection.write(write_request(
            &*connection,
            &[(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(1))],
        ));
        let Poll::Ready(outcome) = poll_once(&mut write) else {
            panic!("a concurrent write must be refused at once");
        };
        let refused = failure(outcome);
        assert_eq!(refused.kind(), ErrorKind::Backpressure, "write kind");
        assert_eq!(
            refused.diagnostics().operation,
            Some(Operation::Write),
            "write op"
        );
        assert_eq!(bench.reads(), 0, "the counter did not move");
        assert_eq!(counter.count(), 0, "nothing woke the first read yet");

        assert!(
            bench.clock.advance_to_next_sleeper().is_some(),
            "time passes"
        );
        assert_eq!(counter.count(), 1, "the sleeper woke the first read");
        let Poll::Ready(outcome) = poll_once_with(&mut first, &waker) else {
            panic!("the first read finishes once its latency has passed");
        };
        drop(outcome.expect("first read succeeds"));
        assert_eq!(bench.reads(), 1, "exactly one device read");

        let third = bench.run(connection.read(read_request(&*connection, &[temperature()])));
        assert!(third.is_ok(), "the slot was released");
        assert_eq!(bench.reads(), 2, "and the next read reached the device");
    }

    #[test]
    fn dropping_a_waiting_operation_releases_the_slot() {
        let bench = Bench::new(Duration::from_secs(1));
        let connection = bench.connect();
        let mut first = connection.read(read_request(&*connection, &[temperature()]));
        assert!(poll_once(&mut first).is_pending(), "first waits");
        drop(first);
        assert_eq!(bench.clock.sleeper_count(), 0, "the sleeper went with it");
        let again = bench.run(connection.read(read_request(&*connection, &[temperature()])));
        assert!(again.is_ok(), "the slot is free again");
        assert_eq!(
            bench.reads(),
            1,
            "the dropped read never touched the device"
        );
    }

    // ---- cancellation and deadlines ---------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F4 cancellation (before transmit)
    #[test]
    fn a_cancelled_token_before_transmit_is_cancelled_with_no_device_access() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let token = CancellationToken::new();
        token.cancel();

        let read = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .cancel(token.clone())
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(read)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(bench.reads(), 0, "no device read");

        let write = WriteRequest::builder(connection.limits())
            .item(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(99))
            .cancel(token)
            .build()
            .expect("request");
        let error = failure(bench.run(connection.write(write)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "write");
        let check = bench
            .run(connection.read(read_request(
                &*connection,
                &[tag("setpoint", "STATE/setpoint:i16")],
            )))
            .expect("read back");
        assert_eq!(
            value_of(&check, "setpoint"),
            Some(PlcValue::I16(20)),
            "not written"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (while waiting)
    #[test]
    fn cancelling_while_waiting_ends_the_call_with_cancelled_and_no_partial_response() {
        let bench = Bench::new(Duration::from_secs(1));
        let connection = bench.connect();
        let start = bench.clock.now();
        let token = CancellationToken::new();
        let read = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .cancel(token.clone())
            .build()
            .expect("request");
        let mut waiting = connection.read(read);
        assert!(poll_once(&mut waiting).is_pending(), "waits for the device");
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("cancelling must wake the wait");
        };
        let error = failure(outcome);
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Internal,
        );
        assert_eq!(bench.reads(), 0, "no device read");
        assert_eq!(bench.clock.now(), start, "no time passed");
        assert!(
            bench
                .run(connection.read(read_request(&*connection, &[temperature()])))
                .is_ok(),
            "the slot was released"
        );

        let write_token = CancellationToken::new();
        let write = WriteRequest::builder(connection.limits())
            .item(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(77))
            .cancel(write_token.clone())
            .build()
            .expect("request");
        let mut pending = connection.write(write);
        assert!(poll_once(&mut pending).is_pending(), "the write waits");
        write_token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut pending) else {
            panic!("cancelling must wake the write");
        };
        assert_eq!(failure(outcome).kind(), ErrorKind::Cancelled, "write");
        assert_eq!(
            bench.device().get("setpoint").map(|t| t.value),
            Some(PlcValue::I16(20)),
            "the device was not changed"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 timeouts (deadline already elapsed)
    #[test]
    fn an_elapsed_deadline_is_a_timeout_with_no_device_access() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let elapsed = Deadline::at(bench.clock.now());
        let read = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .deadline(elapsed)
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(read)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
        assert_eq!(error.diagnostics().retries, 0, "nothing retried by default");
        assert_eq!(bench.reads(), 0, "no device read");

        let write = WriteRequest::builder(connection.limits())
            .item(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(99))
            .deadline(elapsed)
            .build()
            .expect("request");
        let error = failure(bench.run(connection.write(write)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "write");
        assert_eq!(bench.device().sink_writes(), 0, "untouched");
    }

    // covers: SPEC mandatory-failure-cases / F4 timeouts (deadline passes while waiting)
    #[test]
    fn a_latency_that_reaches_the_deadline_times_out_when_the_deadline_is_reached() {
        let bench = Bench::new(Duration::from_secs(1));
        let connection = bench.connect();
        let start = bench.clock.now();
        let deadline = Deadline::after(start, 100 * MS).expect("deadline");
        let read = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .deadline(deadline)
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(read)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            bench.clock.now() - start,
            100 * MS,
            "waited for the deadline"
        );
        assert_eq!(error.diagnostics().latency, Some(100 * MS), "latency");
        assert_eq!(bench.reads(), 0, "no partial response, no device read");

        let start = bench.clock.now();
        let at_the_bound = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .deadline(Deadline::after(start, Duration::from_secs(1)).expect("deadline"))
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(at_the_bound)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "equality means elapsed");

        let roomy = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .deadline(Deadline::after(bench.clock.now(), Duration::from_secs(2)).expect("deadline"))
            .build()
            .expect("request");
        let before = bench.clock.now();
        drop(
            bench
                .run(connection.read(roomy))
                .expect("a roomy deadline succeeds"),
        );
        assert_eq!(
            bench.clock.now() - before,
            Duration::from_secs(1),
            "latency passed"
        );
        assert_eq!(bench.reads(), 1, "one device read");
    }

    // covers: ADR-0007 decision 7 / the default-deadline rule
    #[test]
    fn an_absent_deadline_uses_timeouts_request() {
        let bench = Bench::new(Duration::from_secs(3));
        let mut timeouts = Timeouts::default();
        timeouts.request = Duration::from_secs(2);
        let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .timeouts(timeouts)
            .access(AccessPolicy::ReadWrite)
            .build()
            .expect("config");
        let connection = bench.connect_config(config).expect("connects");

        let start = bench.clock.now();
        let error =
            failure(bench.run(connection.read(read_request(&*connection, &[temperature()]))));
        assert_eq!(error.kind(), ErrorKind::Timeout, "3 s latency exceeds 2 s");
        assert_eq!(
            bench.clock.now() - start,
            Duration::from_secs(2),
            "request timeout"
        );

        let error = failure(bench.run(connection.write(write_request(
            &*connection,
            &[(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(1))],
        ))));
        assert_eq!(error.kind(), ErrorKind::Timeout, "write uses the same rule");

        let explicit = ReadRequest::builder(connection.limits())
            .tag(temperature())
            .deadline(
                Deadline::after(bench.clock.now(), Duration::from_secs(10)).expect("deadline"),
            )
            .build()
            .expect("request");
        drop(
            bench
                .run(connection.read(explicit))
                .expect("an explicit deadline wins"),
        );
    }

    // ---- validation before any device access --------------------------------------------

    // covers: ALPHA_PLAN S11b-2 / foreign address rejected before I/O
    #[test]
    fn a_foreign_address_or_a_multi_element_tag_is_rejected_before_io() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let foreign = Tag::new(
            TagId::new("foreign").expect("identity"),
            Arc::new(FixtureAddress::new("40001", Some(PlcValueType::I16))),
        );
        let array = tag("array", "STATE/temperature:f32")
            .with_count(std::num::NonZeroU32::new(3).expect("positive"));
        for (bad, value) in [(foreign, PlcValue::I16(1)), (array, PlcValue::F32(1.0))] {
            let read =
                failure(bench.run(
                    connection.read(read_request(&*connection, &[temperature(), bad.clone()])),
                ));
            assert_eq!(read.kind(), ErrorKind::InvalidAddress, "read");
            assert_diagnostics(
                &read,
                Operation::Read,
                ConnectionState::Connected,
                FailureCategory::Validation,
            );
            let write = failure(
                bench.run(connection.write(write_request(&*connection, &[(bad.clone(), value)]))),
            );
            assert_eq!(write.kind(), ErrorKind::InvalidAddress, "write");
        }
        assert_eq!(
            bench.reads(),
            0,
            "the valid tag in the batch was not read either"
        );
    }

    #[test]
    fn the_device_item_limit_is_advertised_and_enforced() {
        let clock = Arc::new(FakeClock::default());
        let (device, reads) = bench_device(Duration::ZERO);
        let limited = device.with_max_items(std::num::NonZeroUsize::new(2).expect("positive"));
        let driver = SimDriver::new()
            .with_clock(Arc::<FakeClock>::clone(&clock))
            .with_device("bench", limited)
            .expect("device");
        let config = driver.parse_config("sim://bench").expect("parses");
        let connection = block_on_with(&clock, driver.connect(config)).expect("connects");
        assert_eq!(
            connection
                .capabilities()
                .max_items_per_request
                .map(std::num::NonZeroUsize::get),
            Some(2),
            "the connection reports the device limit"
        );
        assert_eq!(
            driver.capabilities().max_items_per_request,
            None,
            "the driver cannot"
        );
        let three = [
            temperature(),
            tag("count", "STATE/count:u16"),
            tag("setpoint", "STATE/setpoint:i16"),
        ];
        let error = failure(block_on_with(
            &clock,
            connection.read(read_request(&*connection, &three)),
        ));
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "three items");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(reads.load(Ordering::Relaxed), 0, "nothing was read");
        let two = block_on_with(
            &clock,
            connection.read(read_request(&*connection, &three[..2])),
        );
        assert!(two.is_ok(), "two items fit");
        assert_eq!(
            connection.limits().max_items_per_request().get(),
            1024,
            "default budget"
        );
        let caps = connection.capabilities();
        assert!(caps.read && caps.write, "read and write");
        assert_eq!(caps.concurrency.get(), 1, "one at a time");
        assert_eq!(
            caps.subscribe,
            SubscriptionMode::Polling,
            "polling subscriptions"
        );
    }

    // ---- per-item results ---------------------------------------------------------------

    // covers: ALPHA_PLAN S11b-2 / STATE unset is a per-item ItemError with status sim 2
    #[test]
    fn an_unset_state_name_is_a_per_item_error_with_status_sim_2() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let ghost = tag("ghost", "STATE/ghost:i16");
        let response = bench
            .run(connection.read(read_request(&*connection, &[ghost, temperature()])))
            .expect("partial failure is data, not a batch failure");
        let item = response.items().first().expect("first item");
        let error = item.outcome().as_ref().expect_err("the unset name fails");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind");
        let status = error.status().expect("device status");
        assert_eq!((status.protocol(), status.code()), ("sim", 2), "status");
        assert_diagnostics(
            error.error(),
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Protocol,
        );
        assert_eq!(response.iter_failed().count(), 1, "one failure");
        assert_eq!(
            value_of(&response, "temperature"),
            Some(PlcValue::F32(71.5)),
            "the other tag was read"
        );
    }

    #[test]
    fn unreadable_write_only_and_mistyped_reads_are_per_item_errors() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let tags = [
            tag("sealed", "STATE/sealed:i16"),
            tag("log", "SINK/log:u8"),
            tag("typed", "STATE/typed:i16"),
        ];
        let response = bench
            .run(connection.read(read_request(&*connection, &tags)))
            .expect("partial failure is data");
        let kinds: Vec<ErrorKind> = response
            .items()
            .iter()
            .map(|item| item.outcome().as_ref().expect_err("fails").kind())
            .collect();
        assert_eq!(
            kinds,
            [
                ErrorKind::Unsupported(Operation::Read),
                ErrorKind::Unsupported(Operation::Read),
                ErrorKind::InvalidConfig
            ],
            "not readable, write-only, wrong stored type"
        );
        for item in response.items() {
            let error = item.outcome().as_ref().expect_err("fails");
            assert_eq!(error.status(), None, "only an unset name carries a status");
            assert_eq!(
                error.error().diagnostics().category,
                Some(FailureCategory::Protocol),
                "device-side failures are Protocol"
            );
        }
    }

    // ---- writes -------------------------------------------------------------------------

    // covers: ALPHA_PLAN S11b-2 / AccessPolicy::ReadOnly is rejected before anything else
    #[test]
    fn a_read_only_policy_rejects_writes_before_any_device_access() {
        let bench = Bench::new(Duration::ZERO);
        let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .build()
            .expect("config");
        assert_eq!(
            config.access(),
            AccessPolicy::ReadOnly,
            "the default policy"
        );
        let connection = bench.connect_config(config).expect("connects");
        let items = [
            (tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(99)),
            (tag("log", "SINK/log:u8"), PlcValue::U8(1)),
        ];
        let error = failure(bench.run(connection.write(write_request(&*connection, &items))));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "kind"
        );
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(bench.device().sink_writes(), 0, "nothing counted");
        assert_eq!(
            bench.device().get("setpoint").map(|t| t.value),
            Some(PlcValue::I16(20)),
            "nothing written"
        );
        assert!(
            bench
                .run(connection.read(read_request(&*connection, &[temperature()])))
                .is_ok(),
            "reads are unaffected"
        );
        bench.run(connection.close()).expect("close");
        let after_close = failure(bench.run(connection.write(write_request(&*connection, &items))));
        assert_eq!(
            after_close.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "the policy answers before the lifecycle does"
        );
    }

    // covers: ALPHA_PLAN S11b-2 / non-writable STATE and RANDOM writes are Rejected
    #[test]
    fn writes_the_device_refuses_are_rejected_per_item_and_change_nothing() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let items = [
            (tag("locked", "STATE/locked:i16"), PlcValue::I16(1)),
            (tag("noise", "RANDOM/noise:u32"), PlcValue::U32(1)),
            (tag("ghost", "STATE/ghost:i16"), PlcValue::I16(1)),
            (tag("typed", "STATE/typed:i16"), PlcValue::I16(1)),
            (tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(26)),
        ];
        let response = bench
            .run(connection.write(write_request(&*connection, &items)))
            .expect("rejections are data");
        assert_eq!(response.rejected_count(), 4, "four rejected");
        assert_eq!(response.confirmed_count(), 1, "one confirmed");
        assert_eq!(response.unknown_count(), 0, "nothing unknown");
        let rejected: Vec<(&str, ErrorKind, Option<u32>)> = response
            .results()
            .iter()
            .filter_map(|result| match result.outcome() {
                WriteOutcome::Rejected(error) => Some((
                    result.tag().as_str(),
                    error.kind(),
                    error.status().map(ProtocolStatus::code),
                )),
                WriteOutcome::Confirmed | WriteOutcome::Unknown(_) | _ => None,
            })
            .collect();
        assert_eq!(
            rejected,
            [
                ("locked", ErrorKind::Unsupported(Operation::Write), None),
                ("noise", ErrorKind::Unsupported(Operation::Write), None),
                ("ghost", ErrorKind::InvalidAddress, Some(2)),
                ("typed", ErrorKind::InvalidConfig, None),
            ],
            "each refusal and its status"
        );
        for result in response.results() {
            if let WriteOutcome::Rejected(error) = result.outcome() {
                assert_diagnostics(
                    error.error(),
                    Operation::Write,
                    ConnectionState::Connected,
                    FailureCategory::Protocol,
                );
            }
        }
        let device = bench.device();
        assert_eq!(
            device.get("locked").map(|t| t.value),
            Some(PlcValue::I16(5)),
            "locked"
        );
        assert_eq!(
            device.get("typed").map(|t| t.value),
            Some(PlcValue::U8(9)),
            "typed"
        );
        assert_eq!(
            device.get("setpoint").map(|t| t.value),
            Some(PlcValue::I16(26)),
            "applied"
        );
    }

    // ---- retry ---------------------------------------------------------------------------

    fn retrying(attempts: u32, retry_on: &[ErrorKind]) -> ConnectionConfig {
        let mut retry = RetryPolicy::default();
        retry.max_attempts = std::num::NonZeroU32::new(attempts).expect("positive");
        retry.retry_on = retry_on.iter().copied().collect();
        ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .retry(retry)
            .access(AccessPolicy::ReadWrite)
            .build()
            .expect("config")
    }

    // covers: ALPHA_PLAN S11b-2 / read retry per RetryPolicy, whole request, retryable kinds
    #[test]
    fn a_read_is_one_attempt_by_default_and_retries_the_whole_request_when_allowed() {
        let bench = Bench::new(Duration::from_secs(1));
        let tight = |connection: &dyn Connection, clock: &FakeClock| {
            ReadRequest::builder(connection.limits())
                .tag(temperature())
                .deadline(Deadline::after(clock.now(), 100 * MS).expect("deadline"))
                .build()
                .expect("request")
        };

        let single = bench.connect();
        let error = failure(bench.run(single.read(tight(&*single, &bench.clock))));
        assert_eq!(error.kind(), ErrorKind::Timeout, "default policy");
        assert_eq!(error.diagnostics().retries, 0, "one attempt by default");

        let retrying_connection = bench
            .connect_config(retrying(3, &[ErrorKind::Timeout]))
            .expect("connects");
        let error = failure(
            bench.run(retrying_connection.read(tight(&*retrying_connection, &bench.clock))),
        );
        assert_eq!(error.kind(), ErrorKind::Timeout, "still a timeout");
        assert_eq!(
            error.diagnostics().retries,
            2,
            "three attempts, two retries"
        );

        let unlisted = bench
            .connect_config(retrying(3, &[ErrorKind::Disconnected]))
            .expect("connects");
        let error = failure(bench.run(unlisted.read(tight(&*unlisted, &bench.clock))));
        assert_eq!(
            error.diagnostics().retries,
            0,
            "Timeout is not in this retry set"
        );

        let write_config = bench
            .connect_config(retrying(3, &[ErrorKind::Timeout]))
            .expect("connects");
        let write = WriteRequest::builder(write_config.limits())
            .item(tag("setpoint", "STATE/setpoint:i16"), PlcValue::I16(1))
            .deadline(Deadline::after(bench.clock.now(), 100 * MS).expect("deadline"))
            .build()
            .expect("request");
        let error = failure(bench.run(write_config.write(write)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "write times out");
        assert_eq!(error.diagnostics().retries, 0, "a write is never retried");
        assert_eq!(bench.reads(), 0, "no attempt reached the device");

        let ok = bench
            .run(retrying_connection.read(read_request(&*retrying_connection, &[temperature()])))
            .expect("succeeds");
        assert_eq!(ok.items().len(), 1, "one item");
        assert_eq!(bench.reads(), 1, "a successful read is not repeated");
    }
}
