//! `EventPump` (ADR-0031, docs/parity/tools.md "Event pump"): named batches started and
//! stopped by name, the interval trigger with its initial delay, a fetch that outlasts its
//! interval, the pause after failed fetches, batches on different connections and on a
//! lease from the connection cache, close, a dropped `recv` and a slow consumer, against the
//! simulator and against Modbus TCP over the fake transport.
//!
//! All timing is on the fake clock: an event's time is the clock's reading when `recv`
//! returns it, and nothing sleeps on real time.
#![cfg(all(feature = "sim", feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::sim::{FailureStep, RealtimeClock, SimAddress, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on, block_on_with, poll_once,
    };
    use stonecrab::{
        BatchEvent, Clock, Connection, ConnectionCache, ConnectionState, Deadline, DriverRegistry,
        ErrorKind, EventPump, PlcValue, ReadRequest, ResourceLimits, Result, Tag, TagBatch, TagId,
    };

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    fn kind<T>(result: Result<T>) -> ErrorKind {
        result.err().map(|error| error.kind()).expect("an error")
    }

    fn tag(id: &str, address: &str) -> Tag {
        Tag::new(
            TagId::new(id).expect("id"),
            Arc::new(SimAddress::parse(address).expect("address")),
        )
    }

    fn default_limits() -> ResourceLimits {
        ResourceLimits::builder().build().expect("the defaults")
    }

    fn level() -> Tag {
        tag("level", "STATE/level:u16")
    }

    fn request(limits: ResourceLimits, tags: Vec<Tag>) -> ReadRequest {
        tags.into_iter()
            .fold(
                ReadRequest::builder(limits),
                stonecrab::ReadRequestBuilder::tag,
            )
            .build()
            .expect("request")
    }

    /// The values of a successful event, in the order of its response.
    fn values(event: BatchEvent) -> Vec<(String, PlcValue)> {
        event
            .outcome
            .expect("a response")
            .items()
            .iter()
            .map(|item| {
                let value = item.outcome().as_ref().expect("a value").clone();
                (item.tag().as_str().to_owned(), value)
            })
            .collect()
    }

    /// The simulator with two devices on the fake clock: `bench` holds `level` (u16 840)
    /// and `temp` (f32 21.5), `line` holds `speed` (u16 12); every tag counts its reads.
    struct Rig {
        clock: FakeClock,
        start: Instant,
        registry: Arc<DriverRegistry>,
        bench: Arc<SimDevice>,
        level_reads: Arc<AtomicU32>,
        temp_reads: Arc<AtomicU32>,
        speed_reads: Arc<AtomicU32>,
    }

    impl Rig {
        fn new() -> Self {
            let clock = FakeClock::default();
            let bench = SimDevice::new();
            let (level, level_reads) = SimTag::new(PlcValue::U16(840)).with_read_counter();
            let (temp, temp_reads) = SimTag::new(PlcValue::F32(21.5)).with_read_counter();
            bench.set("level", level).expect("level");
            bench.set("temp", temp).expect("temp");
            let line = SimDevice::new();
            let (speed, speed_reads) = SimTag::new(PlcValue::U16(12)).with_read_counter();
            line.set("speed", speed).expect("speed");
            let driver = SimDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_device("bench", bench)
                .expect("bench")
                .with_device("line", line)
                .expect("line");
            let bench = driver.device("bench").expect("the bench");
            let mut registry = DriverRegistry::new();
            registry.register(Arc::new(driver)).expect("register");
            Self {
                start: clock.now(),
                clock,
                registry: Arc::new(registry),
                bench,
                level_reads,
                temp_reads,
                speed_reads,
            }
        }

        fn connect(&self, uri: &str) -> Box<dyn Connection> {
            block_on_with(&self.clock, self.registry.connect(uri)).expect("connect")
        }

        /// A batch named `name` reading `level` on `connection` every `interval`.
        fn level_batch<'a>(
            name: &str,
            connection: &'a dyn Connection,
            interval: Duration,
        ) -> TagBatch<'a> {
            let request = request(connection.limits(), vec![level()]);
            TagBatch::on_connection(name, connection, request, interval).expect("batch")
        }

        /// The next event and when it came, measured from the rig's start.
        fn next(&self, pump: &mut EventPump<'_>) -> (BatchEvent, Duration) {
            let event = block_on_with(&self.clock, pump.recv())
                .expect("recv")
                .expect("an event");
            (event, self.clock.now().duration_since(self.start))
        }

        /// The next event's batch, skipped count and time.
        fn next_summary(&self, pump: &mut EventPump<'_>) -> (String, u64, Duration) {
            let (event, at) = self.next(pump);
            assert!(event.outcome.is_ok(), "{:?}", event.outcome);
            (event.batch, event.skipped, at)
        }

        fn ended(&self, pump: &mut EventPump<'_>) -> bool {
            block_on_with(&self.clock, pump.recv())
                .expect("recv")
                .is_none()
        }
    }

    fn reads(counter: &AtomicU32) -> u32 {
        counter.load(Ordering::SeqCst)
    }

    fn summary(name: &str, skipped: u64, at: Duration) -> (String, u64, Duration) {
        (name.to_owned(), skipped, at)
    }

    // covers: docs/parity/tools.md TOOL-EP-PUMP / add, start, stop and remove by name
    #[test]
    fn event_pump_starts_and_stops_a_batch_by_name() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("added");
        assert_eq!(pump.is_started("tank"), Some(false), "added stopped");
        assert!(rig.ended(&mut pump), "nothing is started");
        assert_eq!(kind(pump.start_batch("pond")), ErrorKind::InvalidConfig);
        assert_eq!(kind(pump.stop_batch("pond")), ErrorKind::InvalidConfig);

        pump.start_batch("tank").expect("started");
        assert_eq!(pump.is_started("tank"), Some(true));
        assert_eq!(
            kind(pump.start_batch("tank")),
            ErrorKind::InvalidConfig,
            "already started, as upstream refuses"
        );
        let (event, at) = rig.next(&mut pump);
        assert_eq!((event.batch.as_str(), at), ("tank", Duration::ZERO));
        assert_eq!(
            values(event),
            vec![("level".to_owned(), PlcValue::U16(840))]
        );
        assert_eq!(rig.next_summary(&mut pump), summary("tank", 0, secs(1)));

        pump.stop_batch("tank").expect("stopped");
        pump.stop_batch("tank")
            .expect("stopping a stopped batch does nothing");
        assert_eq!(pump.is_started("tank"), Some(false));
        assert!(rig.ended(&mut pump), "a stopped batch is not fetched");
        assert_eq!(reads(&rig.level_reads), 2);

        assert_eq!(
            kind(pump.add_batch(Rig::level_batch("tank", &*bench, secs(5)))),
            ErrorKind::InvalidConfig,
            "one batch per name"
        );
        pump.start_all().expect("start all");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, secs(1)),
            "a restart fetches at once"
        );
        pump.stop_all();
        assert!(rig.ended(&mut pump));

        let removed = pump.remove_batch("tank").expect("the batch");
        assert_eq!(removed.name(), "tank");
        assert_eq!(pump.is_started("tank"), None, "gone");
        assert!(pump.remove_batch("tank").is_none());
        pump.add_batch(removed)
            .expect("a removed batch can come back");
        assert_eq!(pump.is_started("tank"), Some(false));
    }

    // covers: docs/parity/tools.md TOOL-EP-PUMP / close stops and forgets every batch
    #[test]
    fn event_pump_close_stops_every_batch() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let line = rig.connect("sim://line");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("tank");
        let belt = request(line.limits(), vec![tag("speed", "STATE/speed:u16")]);
        pump.add_batch(TagBatch::on_connection("belt", &*line, belt, secs(1)).expect("belt"))
            .expect("belt");
        pump.start_all().expect("start all");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, Duration::ZERO)
        );
        assert!(
            format!("{pump:?}").contains("queued: 1"),
            "the belt's event waits: {pump:?}"
        );

        pump.close();
        assert!(rig.ended(&mut pump), "the queued event is forgotten too");
        assert_eq!(pump.is_started("tank"), None);
        assert_eq!(
            kind(pump.add_batch(Rig::level_batch("pond", &*bench, secs(1)))),
            ErrorKind::Disconnected
        );
        assert_eq!(kind(pump.start_batch("tank")), ErrorKind::Disconnected);
        assert_eq!(kind(pump.start_all()), ErrorKind::Disconnected);
        pump.close();
        assert!(rig.ended(&mut pump), "a second close does nothing");
        assert_eq!(
            (bench.state(), line.state()),
            (ConnectionState::Connected, ConnectionState::Connected),
            "the pump only borrows its connections"
        );
        assert_eq!((reads(&rig.level_reads), reads(&rig.speed_reads)), (1, 1));
    }

    // covers: docs/parity/tools.md TOOL-EP-PUMP / stop and restart one batch while another runs
    #[test]
    fn event_pump_restarts_one_batch_while_another_runs() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let line = rig.connect("sim://line");
        rig.bench
            .push_failure(FailureStep::Delay(ms(2500)))
            .expect("a slow bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)).initial_delay(ms(300)))
            .expect("tank");
        let belt = request(line.limits(), vec![tag("speed", "STATE/speed:u16")]);
        pump.add_batch(TagBatch::on_connection("belt", &*line, belt, secs(1)).expect("belt"))
            .expect("belt");
        pump.start_all().expect("start all");
        for at in [Duration::ZERO, secs(1), secs(2)] {
            assert_eq!(rig.next_summary(&mut pump), summary("belt", 0, at));
        }

        pump.stop_batch("tank")
            .expect("stopped, its read from 300 ms dropped");
        rig.clock.advance(ms(200));
        pump.start_batch("tank").expect("restarted at 2.2 s");
        let events: Vec<_> = (0..4).map(|_| rig.next_summary(&mut pump)).collect();
        assert_eq!(
            events,
            vec![
                summary("tank", 0, ms(2500)),
                summary("belt", 0, secs(3)),
                summary("tank", 0, ms(3500)),
                summary("belt", 0, secs(4)),
            ],
            "the tank restarts on a grid of its own, its count reset; the belt keeps its ticks"
        );
        assert_eq!(reads(&rig.speed_reads), 5);
    }

    // covers: docs/parity/tools.md TOOL-EP-TIMER / the initial delay, then the interval
    #[test]
    fn event_pump_timer_fires_at_the_interval_after_the_initial_delay() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)).initial_delay(ms(500)))
            .expect("added");
        pump.start_batch("tank").expect("started");
        for at in [ms(500), ms(1500), ms(2500)] {
            assert_eq!(rig.next_summary(&mut pump), summary("tank", 0, at));
        }
        assert_eq!(reads(&rig.level_reads), 3);
    }

    // covers: docs/parity/tools.md TOOL-EP-TIMER / a fetch that outlasts its interval
    #[test]
    fn event_pump_timer_skips_a_tick_while_a_poll_is_running() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        rig.bench
            .push_failure(FailureStep::Delay(ms(2500)))
            .expect("a slow read");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("added");
        pump.start_batch("tank").expect("started");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 2, ms(2500)),
            "the ticks at 1 s and 2 s fell due while the read ran"
        );
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, secs(3)),
            "the next read waits for the next tick: no burst"
        );
        assert_eq!(rig.next_summary(&mut pump), summary("tank", 0, secs(4)));
        assert_eq!(reads(&rig.level_reads), 3, "one read per fetch");
    }

    // covers: docs/parity/tools.md TOOL-EP-READ / one read with every tag of the batch
    #[test]
    fn event_pump_batch_fetch_issues_one_read_with_all_batch_tags() {
        let rig = Rig::new();
        // Every operation on this connection takes 10 ms: one read, one wait.
        let bench = rig.connect("sim://bench?latency-ms=10");
        let connected = rig.clock.now().duration_since(rig.start);
        assert_eq!(connected, ms(10), "the connect took one wait");
        let both = request(bench.limits(), vec![level(), tag("temp", "STATE/temp:f32")]);
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(TagBatch::on_connection("tank", &*bench, both, secs(1)).expect("batch"))
            .expect("added");
        pump.start_batch("tank").expect("started");
        let (event, at) = rig.next(&mut pump);
        assert_eq!(at, connected + ms(10), "one round trip, not two");
        assert_eq!(
            values(event),
            vec![
                ("level".to_owned(), PlcValue::U16(840)),
                ("temp".to_owned(), PlcValue::F32(21.5)),
            ],
            "both tags, in the request's order"
        );
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, connected + ms(1010))
        );
        assert_eq!(
            (reads(&rig.level_reads), reads(&rig.temp_reads)),
            (2, 2),
            "each tag once per fetch"
        );
    }

    // covers: docs/parity/tools.md TOOL-EP-READ / each event names its batch
    #[test]
    fn event_pump_delivers_each_response_with_its_batch_name() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("fast", &*bench, secs(1)))
            .expect("fast");
        let temp = request(bench.limits(), vec![tag("temp", "STATE/temp:f32")]);
        pump.add_batch(TagBatch::on_connection("slow", &*bench, temp, secs(2)).expect("slow"))
            .expect("slow");
        pump.start_all().expect("start all");
        let mut seen = Vec::new();
        for _ in 0..5 {
            let (event, at) = rig.next(&mut pump);
            let batch = event.batch.clone();
            let (id, _value) = values(event).remove(0);
            seen.push((batch, id, at));
        }
        let expected = [
            ("fast", "level", 0),
            ("slow", "temp", 0),
            ("fast", "level", 1),
            ("fast", "level", 2),
            ("slow", "temp", 2),
        ]
        .map(|(batch, id, at)| (batch.to_owned(), id.to_owned(), secs(at)));
        assert_eq!(seen, expected);
    }

    // covers: docs/parity/tools.md TOOL-EP-MULTI / a slow device does not hold up another
    #[test]
    fn event_pump_batches_on_different_connections_run_independently() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let line = rig.connect("sim://line");
        rig.bench
            .push_failure(FailureStep::Delay(ms(2500)))
            .expect("a slow bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("tank");
        let speed = request(line.limits(), vec![tag("speed", "STATE/speed:u16")]);
        pump.add_batch(TagBatch::on_connection("belt", &*line, speed, secs(1)).expect("belt"))
            .expect("belt");
        pump.start_all().expect("start all");
        let events: Vec<_> = (0..6).map(|_| rig.next_summary(&mut pump)).collect();
        assert_eq!(
            events,
            vec![
                summary("belt", 0, Duration::ZERO),
                summary("belt", 0, secs(1)),
                summary("belt", 0, secs(2)),
                summary("tank", 2, ms(2500)),
                summary("tank", 0, secs(3)),
                summary("belt", 0, secs(3)),
            ],
            "the belt keeps its ticks while the tank's read waits"
        );
    }

    // covers: docs/parity/tools.md TOOL-EP-MULTI / a batch on a lease from the connection cache
    #[test]
    fn event_pump_reads_a_batch_through_a_lease_from_the_cache() {
        let rig = Rig::new();
        let cache =
            ConnectionCache::builder(Arc::clone(&rig.registry), Arc::new(rig.clock.clone()))
                .build()
                .expect("cache");
        let shared = request(default_limits(), vec![level()]);
        let batch =
            TagBatch::on_cache("tank", &cache, "sim://bench", shared, secs(1)).expect("batch");
        assert!(format!("{batch:?}").contains("sim://bench"), "{batch:?}");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(batch).expect("added");
        pump.start_batch("tank").expect("started");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, Duration::ZERO)
        );
        assert_eq!(
            (cache.len(), cache.active_leases()),
            (1, 0),
            "the lease went back after the fetch"
        );

        // Someone else holds the connection when the next tick falls due: the fetch waits
        // in the cache's queue, and reads once the lease comes back.
        let lease = block_on_with(&rig.clock, cache.get("sim://bench")).expect("a lease");
        rig.clock.advance(secs(1));
        assert!(
            poll_once(&mut pump.recv()).is_pending(),
            "waiting for the lease"
        );
        assert_eq!(cache.active_leases(), 1);
        drop(lease);
        assert_eq!(rig.next_summary(&mut pump), summary("tank", 0, secs(1)));
        assert_eq!(cache.active_leases(), 0);
        assert_eq!(reads(&rig.level_reads), 2);
    }

    // covers: docs/parity/tools.md TOOL-EP-READ / a failed fetch is reported, then paused
    #[test]
    fn event_pump_reports_a_failed_read_and_the_batch_goes_on() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        // Two reads that time out after the connection's default request timeout (5 s).
        rig.bench
            .push_failures([FailureStep::Timeout, FailureStep::Timeout])
            .expect("two failures");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, ms(500)))
            .expect("added");
        pump.start_batch("tank").expect("started");

        let (failed, at) = rig.next(&mut pump);
        assert_eq!(kind(failed.outcome), ErrorKind::Timeout);
        assert_eq!(
            (failed.batch.as_str(), failed.skipped, at),
            ("tank", 10, secs(5))
        );
        // A one-second pause: the tick at 5.5 s is skipped, the one at 6 s fetches and
        // fails again, and the pause doubles to two seconds.
        let (failed, at) = rig.next(&mut pump);
        assert_eq!(kind(failed.outcome), ErrorKind::Timeout);
        assert_eq!((failed.skipped, at), (1 + 10, secs(11)));
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 3, secs(13)),
            "11.5, 12 and 12.5 s fall in the pause; 13 s succeeds"
        );
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, ms(13_500)),
            "a success ends the pause"
        );

        // A batch on a cache whose lease cannot be had fails like a read.
        let cache =
            ConnectionCache::builder(Arc::clone(&rig.registry), Arc::new(rig.clock.clone()))
                .build()
                .expect("cache");
        let nowhere = request(default_limits(), vec![level()]);
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(
            TagBatch::on_cache("ghost", &cache, "opc-ua://nowhere", nowhere, secs(1))
                .expect("batch"),
        )
        .expect("added");
        pump.start_batch("ghost").expect("started");
        let (failed, _at) = rig.next(&mut pump);
        assert!(
            matches!(
                failed.outcome.map(|_| ()).err().map(|e| e.kind()),
                Some(ErrorKind::Unsupported(_))
            ),
            "no driver serves the scheme"
        );
    }

    /// The Modbus read of holding register 1 (FC03, quantity 1, unit 1).
    fn modbus_read(txn: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 6, 1, 0x03, 0x00, 0x00, 0x00, 0x01]);
        bytes
    }

    fn modbus_reply(txn: u16, value: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 5, 1, 0x03, 0x02]);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    // covers: docs/parity/tools.md TOOL-EP-OPERATIONS / read is the only operation
    #[test]
    fn event_pump_issues_no_operation_besides_read() {
        let clock = FakeClock::default();
        let start = clock.now();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![
            Step::Expect(modbus_read(1)),
            Step::Reply(modbus_reply(1, 7)),
            Step::Expect(modbus_read(2)),
            Step::Reply(modbus_reply(2, 8)),
        ]));
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(driver)).expect("register");
        let connection = block_on_with(&clock, registry.connect("modbus-tcp://127.0.0.1:5020"))
            .expect("connect");
        let address = registry
            .driver("modbus-tcp")
            .expect("registered")
            .parse_address("4x00001:UINT", &stonecrab::AddressOptions::default())
            .expect("address");
        let register = Tag::new(TagId::new("register").expect("id"), address);
        let request = request(connection.limits(), vec![register]);
        let mut pump = EventPump::new(&clock);
        pump.add_batch(
            TagBatch::on_connection("plc", &*connection, request, secs(1)).expect("batch"),
        )
        .expect("added");
        pump.start_batch("plc").expect("started");
        for (value, at) in [(7, Duration::ZERO), (8, secs(1))] {
            let event = block_on_with(&clock, pump.recv())
                .expect("recv")
                .expect("an event");
            assert_eq!(clock.now().duration_since(start), at);
            assert_eq!(
                values(event),
                vec![("register".to_owned(), PlcValue::U16(value))]
            );
        }
        pump.close();
        let sent: Vec<Vec<u8>> = factory
            .probes()
            .first()
            .expect("one transport")
            .sent()
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        assert_eq!(
            sent,
            vec![modbus_read(1), modbus_read(2)],
            "two reads and nothing else: no ping, no write"
        );
    }

    // covers: SPEC mandatory-failure-cases / cancellation while waiting and while a read runs
    #[test]
    fn event_pump_recv_dropped_while_waiting_loses_nothing() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("added");
        pump.start_batch("tank").expect("started");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, Duration::ZERO)
        );

        assert!(poll_once(&mut pump.recv()).is_pending(), "waiting for 1 s");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, secs(1)),
            "the dropped wait cost nothing"
        );

        // Dropped while its read waits on the device: the read stays with the batch and
        // goes on at the next call instead of starting again.
        rig.bench
            .push_failure(FailureStep::Delay(ms(500)))
            .expect("a slow read");
        rig.clock.advance(secs(1));
        assert!(poll_once(&mut pump.recv()).is_pending(), "the read runs");
        assert_eq!(reads(&rig.level_reads), 2, "the device is not reached yet");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, ms(2500)),
            "the same read, finished"
        );
        assert_eq!(reads(&rig.level_reads), 3, "read once");

        // Stopping the batch drops a read in flight.
        rig.bench
            .push_failure(FailureStep::Delay(ms(500)))
            .expect("a slow read");
        rig.clock.advance(ms(500));
        assert!(poll_once(&mut pump.recv()).is_pending(), "the read runs");
        pump.stop_batch("tank").expect("stopped");
        assert!(rig.ended(&mut pump));
        assert_eq!(
            reads(&rig.level_reads),
            3,
            "the dropped read never reached the device"
        );
    }

    // covers: SPEC 'Connection and resource behavior' / a slow consumer is bounded
    #[test]
    fn event_pump_holds_at_most_one_event_per_batch_for_a_slow_consumer() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let line = rig.connect("sim://line");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("tank");
        let speed = request(line.limits(), vec![tag("speed", "STATE/speed:u16")]);
        pump.add_batch(TagBatch::on_connection("belt", &*line, speed, secs(1)).expect("belt"))
            .expect("belt");
        pump.start_all().expect("start all");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 0, Duration::ZERO)
        );
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("belt", 0, Duration::ZERO)
        );

        // The consumer is away for ten seconds: nothing runs and nothing piles up.
        rig.clock.advance(secs(10));
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("tank", 9, secs(10)),
            "the latest tick is fetched late, once; the nine before it are skipped"
        );
        assert!(format!("{pump:?}").contains("queued: 1"), "{pump:?}");
        assert_eq!(rig.next_summary(&mut pump), summary("belt", 9, secs(10)));
        assert_eq!(
            (reads(&rig.level_reads), reads(&rig.speed_reads)),
            (2, 2),
            "one read per batch for the whole absence"
        );
        assert_eq!(rig.next_summary(&mut pump), summary("tank", 0, secs(11)));
    }

    /// The simulator's default `RealtimeClock` resolves every sleep at once.
    #[test]
    fn event_pump_refuses_a_clock_that_does_not_sleep() {
        let clock = RealtimeClock::new();
        let mut registry = DriverRegistry::new();
        let device = SimDevice::new();
        device
            .set("level", SimTag::new(PlcValue::U16(840)))
            .expect("level");
        registry
            .register(Arc::new(
                SimDriver::new()
                    .with_device("bench", device)
                    .expect("device"),
            ))
            .expect("register");
        let bench = block_on(registry.connect("sim://bench")).expect("connect");
        let mut pump = EventPump::new(&clock);
        pump.add_batch(Rig::level_batch("tank", &*bench, secs(1)))
            .expect("added");
        pump.start_batch("tank").expect("started");
        let first = block_on(pump.recv())
            .expect("recv")
            .expect("the first fetch");
        assert!(first.outcome.is_ok());
        assert_eq!(kind(block_on(pump.recv())), ErrorKind::InvalidConfig);
    }

    // covers: docs/parity/tools.md TOOL-EP-TIMER / a tick past the end of the clock's time
    #[test]
    fn event_pump_batch_whose_next_tick_lies_past_the_end_of_time_stops_fetching() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let mut pump = EventPump::new(&rig.clock);
        pump.add_batch(Rig::level_batch("once", &*bench, Duration::MAX))
            .expect("once");
        pump.add_batch(Rig::level_batch("never", &*bench, secs(1)).initial_delay(Duration::MAX))
            .expect("never");
        pump.start_all().expect("start all");
        assert_eq!(
            rig.next_summary(&mut pump),
            summary("once", 0, Duration::ZERO)
        );
        assert!(rig.ended(&mut pump), "neither has a tick left");
        assert_eq!(
            (pump.is_started("once"), pump.is_started("never")),
            (Some(true), Some(true))
        );
        assert_eq!(reads(&rig.level_reads), 1);
    }

    // covers: docs/parity/tools.md TOOL-EP-PUMP / what a batch refuses
    #[test]
    fn tag_batch_refuses_a_blank_name_a_zero_interval_and_a_deadline() {
        let rig = Rig::new();
        let bench = rig.connect("sim://bench");
        let plain = request(bench.limits(), vec![level()]);
        assert_eq!(
            kind(TagBatch::on_connection(
                " ",
                &*bench,
                plain.clone(),
                secs(1)
            )),
            ErrorKind::InvalidConfig
        );
        assert_eq!(
            kind(TagBatch::on_connection(
                "tank",
                &*bench,
                plain.clone(),
                Duration::ZERO
            )),
            ErrorKind::InvalidConfig
        );
        let deadline = Deadline::after(rig.clock.now(), secs(5)).expect("deadline");
        let bounded = ReadRequest::builder(bench.limits())
            .tag(level())
            .deadline(deadline)
            .build()
            .expect("request");
        assert_eq!(
            kind(TagBatch::on_connection("tank", &*bench, bounded, secs(1))),
            ErrorKind::InvalidConfig,
            "a deadline would expire for every later fetch"
        );
        let batch = TagBatch::on_connection("tank", &*bench, plain, secs(1)).expect("batch");
        let shown = format!("{batch:?}");
        assert!(
            shown.contains("\"tank\"") && shown.contains("connection"),
            "{shown}"
        );
    }

    #[test]
    fn event_pump_types_are_send() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send::<EventPump<'static>>();
        assert_send_sync::<TagBatch<'static>>();
        assert_send_sync::<BatchEvent>();
    }
}
