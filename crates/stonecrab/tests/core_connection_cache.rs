//! `ConnectionCache` (ADR-0030, docs/parity/tools.md "Connection cache"): leases by
//! connection string, the arrival-order queue and its bound, the lease, wait, idle, ping and
//! close bounds, the admin operations and close, against the simulator and against Modbus
//! TCP over the fake transport (the simulator has no ping).
//!
//! All timing is on the fake clock: a connect, a close or a ping that waits shows up as the
//! clock moving, and a reused connection as the clock standing still.
#![cfg(all(feature = "sim", feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::task::Poll;
    use std::time::Duration;

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::sim::{FailureStep, RealtimeClock, SimAddress, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{
        CountingWaker, FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
        poll_once_with,
    };
    use stonecrab::{
        Clock, Connection, ConnectionCache, ConnectionCacheBuilder, ConnectionState,
        DriverRegistry, ErrorKind, FailureCategory, LeasedConnection, OpFuture, Operation,
        PlcError, PlcValue, QueuePolicy, ReadRequest, ReadResponse, ResourceLimits, Result,
        SubscribeRequest, SubscriptionKind, SubscriptionMode, Tag, TagId, WriteRequest,
    };

    const BENCH: &str = "sim://bench";
    /// The same device with a latency: every connect and close waits 10 ms.
    const SLOW: &str = "sim://bench?latency-ms=10";
    const MODBUS: &str = "modbus-tcp://127.0.0.1:5020";

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    fn kind<T>(result: Result<T>) -> ErrorKind {
        result.err().map(|error| error.kind()).expect("an error")
    }

    fn error<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    /// A cache over the simulator, whose device `bench` holds `boiler` (f32, with a read
    /// counter).
    struct SimRig {
        clock: FakeClock,
        device: Arc<SimDevice>,
        reads: Arc<std::sync::atomic::AtomicU32>,
        cache: ConnectionCache,
    }

    impl SimRig {
        fn new() -> Self {
            Self::with(|builder| builder)
        }

        fn with(configure: impl FnOnce(ConnectionCacheBuilder) -> ConnectionCacheBuilder) -> Self {
            let clock = FakeClock::default();
            let device = SimDevice::new();
            let (tag, reads) = SimTag::new(PlcValue::F32(71.5)).with_read_counter();
            device.set("boiler", tag).expect("tag");
            let driver = SimDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_device("bench", device)
                .expect("device");
            let device = driver.device("bench").expect("the device");
            let mut registry = DriverRegistry::new();
            registry.register(Arc::new(driver)).expect("register");
            let builder = ConnectionCache::builder(Arc::new(registry), Arc::new(clock.clone()));
            let cache = configure(builder).build().expect("cache");
            Self {
                clock,
                device,
                reads,
                cache,
            }
        }

        fn get(&self, uri: &str) -> Result<LeasedConnection> {
            block_on_with(&self.clock, self.cache.get(uri))
        }

        /// How long, on the fake clock, `get` took.
        fn timed_get(&self, uri: &str) -> (Result<LeasedConnection>, Duration) {
            let start = self.clock.now();
            let lease = self.get(uri);
            (lease, self.clock.now().duration_since(start))
        }

        /// Read `boiler` through `lease`; the call's error, or the item's value.
        fn read(&self, lease: &LeasedConnection) -> Result<PlcValue> {
            let request = ReadRequest::builder(lease.limits()).tag(boiler()).build()?;
            let response = block_on_with(&self.clock, lease.read(request))?;
            Ok(value_of(&response))
        }

        fn reads(&self) -> u32 {
            self.reads.load(Ordering::SeqCst)
        }
    }

    fn boiler() -> Tag {
        Tag::new(
            TagId::new("boiler").expect("id"),
            Arc::new(SimAddress::parse("STATE/boiler:f32").expect("address")),
        )
    }

    fn value_of(response: &ReadResponse) -> PlcValue {
        let item = response.items().first().expect("an item");
        item.outcome().as_ref().expect("a value").clone()
    }

    /// The request of the Modbus default ping (`4x00001:BOOL`: holding register 1, unit 1).
    fn ping_request(txn: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 6, 1, 0x03, 0x00, 0x00, 0x00, 0x01]);
        bytes
    }

    fn ping_reply(txn: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 5, 1, 0x03, 0x02, 0x00, 0x01]);
        bytes
    }

    /// A cache over Modbus TCP on the fake transport, whose connections ping.
    struct ModbusRig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        cache: ConnectionCache,
    }

    impl ModbusRig {
        fn with(configure: impl FnOnce(ConnectionCacheBuilder) -> ConnectionCacheBuilder) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            let driver = ModbusTcpDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_transport_factory(Arc::new(factory.clone()));
            let mut registry = DriverRegistry::new();
            registry.register(Arc::new(driver)).expect("register");
            let builder = ConnectionCache::builder(Arc::new(registry), Arc::new(clock.clone()));
            Self {
                cache: configure(builder).build().expect("cache"),
                clock,
                factory,
            }
        }

        fn get(&self) -> Result<LeasedConnection> {
            block_on_with(&self.clock, self.cache.get(MODBUS))
        }

        fn opens(&self) -> usize {
            self.factory.opens().len()
        }

        /// What the transport of the `index`-th open sent.
        fn sent(&self, index: usize) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .get(index)
                .expect("that transport")
                .sent()
                .iter()
                .map(|frame| frame.to_vec())
                .collect()
        }
    }

    // covers: docs/parity/tools.md TOOL-CC-CACHE / one connection reused per string
    #[test]
    fn connection_cache_returns_the_same_connection_for_the_same_string() {
        let rig = SimRig::new();
        let (first, opening) = rig.timed_get(SLOW);
        let first = first.expect("first lease");
        assert_eq!(opening, ms(10), "the first lease opened the connection");
        assert_eq!(rig.read(&first).expect("read"), PlcValue::F32(71.5));
        let watch = first.watch_state();
        drop(first);
        let (second, reusing) = rig.timed_get(SLOW);
        let second = second.expect("second lease");
        assert_eq!(reusing, Duration::ZERO, "the second lease reused it");
        assert_eq!(rig.read(&second).expect("read"), PlcValue::F32(71.5));
        assert_eq!(watch.current(), ConnectionState::Connected, "never closed");
        assert_eq!(rig.cache.len(), 1, "one entry");
    }

    // covers: docs/parity/tools.md TOOL-CC-CACHE / the string is the key, exactly as given
    #[test]
    fn connection_cache_creates_one_connection_per_connection_string() {
        let rig = SimRig::new();
        let plain = rig.get(BENCH).expect("plain");
        let (slow, opening) = rig.timed_get(SLOW);
        let slow = slow.expect("another string is not queued behind the first");
        assert_eq!(opening, ms(10), "its own connection");
        assert_eq!(rig.cache.active_leases(), 2, "one lease per string");
        assert_eq!(
            rig.cache.connection_strings(),
            vec![BENCH.to_owned(), SLOW.to_owned()]
        );
        drop((plain, slow));
        assert_eq!((rig.cache.len(), rig.cache.active_leases()), (2, 0));
    }

    // covers: docs/parity/tools.md TOOL-CC-LEASE / one lease at a time per string
    #[test]
    fn connection_cache_grants_one_lease_at_a_time_per_string() {
        let rig = SimRig::new();
        let first = rig.get(BENCH).expect("first lease");
        let waker = CountingWaker::new();
        let mut second = rig.cache.get(BENCH);
        assert!(
            poll_once_with(&mut second, &waker.waker()).is_pending(),
            "queued"
        );
        assert!(
            poll_once_with(&mut second, &waker.waker()).is_pending(),
            "still queued"
        );
        assert_eq!(rig.cache.active_leases(), 1, "one lease");
        drop(first);
        assert!(waker.count() >= 1, "the return wakes the waiter");
        let Poll::Ready(second) = poll_once(&mut second) else {
            panic!("the returned connection goes to the waiter");
        };
        let second = second.expect("second lease");
        assert_eq!(rig.read(&second).expect("read"), PlcValue::F32(71.5));
        assert_eq!(rig.cache.active_leases(), 1, "still one lease");
    }

    // covers: docs/parity/tools.md TOOL-CC-QUEUE / waiters in arrival order
    #[test]
    fn connection_cache_waiters_are_served_in_arrival_order() {
        let rig = SimRig::new();
        let holder = rig.get(BENCH).expect("holder");
        let mut first = rig.cache.get(BENCH);
        let mut second = rig.cache.get(BENCH);
        let mut third = rig.cache.get(BENCH);
        for waiter in [&mut first, &mut second, &mut third] {
            assert!(poll_once(waiter).is_pending(), "queued");
        }
        drop(holder);
        assert!(poll_once(&mut third).is_pending(), "the third is not next");
        assert!(
            poll_once(&mut second).is_pending(),
            "the second is not next"
        );
        let Poll::Ready(Ok(lease)) = poll_once(&mut first) else {
            panic!("the first waiter is served first");
        };
        drop(lease);
        assert!(
            poll_once(&mut third).is_pending(),
            "the third is still not next"
        );
        let Poll::Ready(Ok(lease)) = poll_once(&mut second) else {
            panic!("then the second");
        };
        drop(lease);
        assert!(
            matches!(poll_once(&mut third), Poll::Ready(Ok(_))),
            "then the third"
        );
    }

    // covers: SPEC mandatory-failure-cases / queue exhaustion (wait time exhausted)
    #[test]
    fn connection_cache_wait_past_the_maximum_is_a_timeout() {
        let rig = SimRig::with(|builder| builder.max_wait_time(secs(2)));
        let holder = rig.get(BENCH).expect("holder");
        let start = rig.clock.now();
        let timeout = error(rig.get(BENCH));
        assert_eq!(timeout.kind(), ErrorKind::Timeout);
        assert_eq!(timeout.diagnostics().operation, Some(Operation::Connect));
        assert_eq!(
            timeout.diagnostics().category,
            Some(FailureCategory::Validation)
        );
        assert_eq!(
            rig.clock.now().duration_since(start),
            secs(2),
            "at the wait time"
        );
        assert_eq!(rig.cache.active_leases(), 1, "the holder keeps its lease");
        assert_eq!(rig.read(&holder).expect("read"), PlcValue::F32(71.5));

        let no_wait = SimRig::with(|builder| builder.max_wait_time(Duration::ZERO));
        let held = no_wait.get(BENCH).expect("held");
        let mut refused = no_wait.cache.get(BENCH);
        assert!(
            matches!(poll_once(&mut refused), Poll::Ready(Err(ref error)) if error.kind() == ErrorKind::Timeout),
            "a zero wait time fails at once"
        );
        drop(held);
        assert!(no_wait.get(BENCH).is_ok(), "a free string needs no wait");
    }

    // covers: SPEC mandatory-failure-cases / cancellation while queued, queue exhaustion
    #[test]
    fn connection_cache_a_cancelled_waiter_leaves_the_queue() {
        let limits = ResourceLimits::builder()
            .max_pending_requests(3)
            .build()
            .expect("limits");
        let rig = SimRig::with(|builder| builder.limits(limits));
        let holder = rig.get(BENCH).expect("holder");
        let mut first = rig.cache.get(BENCH);
        let mut second = rig.cache.get(BENCH);
        assert!(poll_once(&mut first).is_pending(), "queued");
        assert!(poll_once(&mut second).is_pending(), "queued");
        let full = error(rig.get(BENCH));
        assert_eq!(
            full.kind(),
            ErrorKind::Backpressure,
            "holder and two waiters fill three"
        );
        assert_eq!(full.diagnostics().operation, Some(Operation::Connect));

        drop(first);
        let mut late = rig.cache.get(BENCH);
        assert!(
            poll_once(&mut late).is_pending(),
            "the cancelled place is free again"
        );
        drop(holder);
        assert!(
            poll_once(&mut late).is_pending(),
            "the cancelled waiter is not served"
        );
        let Poll::Ready(Ok(lease)) = poll_once(&mut second) else {
            panic!("the second waiter moved to the front");
        };
        drop(lease);
        assert!(
            matches!(poll_once(&mut late), Poll::Ready(Ok(_))),
            "then the late one"
        );

        // A caller dropped on its turn, before it polled again, lets the next one in.
        let holder = rig.get(BENCH).expect("holder again");
        let mut abandoned = rig.cache.get(BENCH);
        let mut next = rig.cache.get(BENCH);
        assert!(poll_once(&mut abandoned).is_pending(), "queued");
        assert!(poll_once(&mut next).is_pending(), "queued");
        drop(holder);
        drop(abandoned);
        assert!(
            matches!(poll_once(&mut next), Poll::Ready(Ok(_))),
            "next is served"
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-LEASE / the maximum lease time
    #[test]
    fn connection_cache_lease_is_invalidated_after_the_maximum_lease_time() {
        let rig = SimRig::with(|builder| builder.max_lease_time(secs(10)));
        let lease = rig.get(BENCH).expect("lease");
        assert_eq!(rig.read(&lease).expect("read"), PlcValue::F32(71.5));
        rig.clock.advance(secs(10));
        let expired = error(rig.read(&lease));
        assert_eq!(expired.kind(), ErrorKind::Timeout, "the lease expired");
        assert_eq!(expired.diagnostics().operation, Some(Operation::Read));
        assert_eq!(rig.reads(), 1, "the refused read never reached the device");
        assert_eq!(
            rig.cache.active_leases(),
            1,
            "counted until taken over or dropped"
        );

        // A waiter takes the connection over from the expired lease.
        let (taken, waited) = rig.timed_get(BENCH);
        let taken = taken.expect("the waiter takes over");
        assert_eq!(
            waited,
            Duration::ZERO,
            "the expired lease does not hold it up"
        );
        assert_eq!(rig.read(&taken).expect("read"), PlcValue::F32(71.5));
        assert_eq!(
            kind(rig.read(&lease)),
            ErrorKind::Timeout,
            "the old lease stays refused"
        );
        drop(lease);
        assert_eq!(
            rig.read(&taken).expect("read"),
            PlcValue::F32(71.5),
            "unaffected"
        );

        // A waiter that queued before the expiry is served at the expiry.
        let start = rig.clock.now();
        let waiter = rig.get(BENCH).expect("served when the lease expires");
        assert_eq!(
            rig.clock.now().duration_since(start),
            secs(10),
            "at the lease expiry"
        );
        drop((taken, waiter));
    }

    // covers: docs/parity/tools.md TOOL-CC-LEASE / returning a lease, early with close
    #[test]
    fn connection_cache_dropping_a_lease_returns_the_connection() {
        let rig = SimRig::new();
        let lease = rig.get(BENCH).expect("lease");
        assert_eq!((rig.cache.len(), rig.cache.active_leases()), (1, 1));
        drop(lease);
        assert_eq!(
            (rig.cache.len(), rig.cache.active_leases()),
            (1, 0),
            "kept, not leased"
        );

        let lease = rig.get(BENCH).expect("lease");
        let mut waiter = rig.cache.get(BENCH);
        assert!(poll_once(&mut waiter).is_pending(), "queued");
        block_on_with(&rig.clock, lease.close()).expect("close gives the lease back");
        assert_eq!(
            lease.state(),
            ConnectionState::Connected,
            "the connection stays open"
        );
        let refused = error(rig.read(&lease));
        assert_eq!(
            refused.kind(),
            ErrorKind::Disconnected,
            "a given-back lease is refused"
        );
        assert_eq!(refused.diagnostics().operation, Some(Operation::Read));
        let Poll::Ready(Ok(next)) = poll_once(&mut waiter) else {
            panic!("the waiter is served after close");
        };
        drop(lease);
        assert_eq!(
            rig.read(&next).expect("read"),
            PlcValue::F32(71.5),
            "dropping later is harmless"
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-CLOSED / SPEC mandatory-failure-cases explicit shutdown
    #[test]
    fn connection_cache_get_after_close_is_a_typed_error() {
        let rig = SimRig::new();
        let lease = rig.get(BENCH).expect("lease");
        let mut waiter = rig.cache.get(BENCH);
        assert!(poll_once(&mut waiter).is_pending(), "queued");
        block_on_with(&rig.clock, rig.cache.close()).expect("close");
        assert!(rig.cache.is_empty(), "emptied");
        assert_eq!(
            lease.state(),
            ConnectionState::Closed,
            "the connection was closed"
        );
        assert_eq!(
            kind(rig.read(&lease)),
            ErrorKind::Disconnected,
            "the lease is refused"
        );
        let Poll::Ready(Err(error)) = poll_once(&mut waiter) else {
            panic!("the waiter fails");
        };
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        let refused = error_after_close(&rig);
        assert_eq!(refused.kind(), ErrorKind::Disconnected);
        assert_eq!(refused.diagnostics().operation, Some(Operation::Connect));
        assert!(
            refused.to_string().contains("connection cache is closed"),
            "{refused}"
        );
        block_on_with(&rig.clock, rig.cache.close()).expect("closing again does nothing");
        drop(lease);
        assert!(rig.cache.is_empty(), "a late return adds nothing");
    }

    fn error_after_close(rig: &SimRig) -> PlcError {
        error(rig.get(BENCH))
    }

    // covers: docs/parity/tools.md TOOL-CC-IDLE-PING / a ping before reusing an idle connection
    #[test]
    fn connection_cache_pings_an_idle_connection_before_reuse() {
        let rig = ModbusRig::with(|builder| builder.idle_ping_threshold(secs(20)));
        rig.factory.push_script(Script::strict(vec![
            Step::Expect(ping_request(1)),
            Step::Reply(ping_reply(1)),
        ]));
        drop(rig.get().expect("first lease"));
        drop(rig.get().expect("recently used: no ping"));
        assert!(rig.sent(0).is_empty(), "no ping yet");
        rig.clock.advance(secs(20));
        let lease = rig.get().expect("pinged, then lent");
        assert_eq!(rig.sent(0), vec![ping_request(1)], "one ping");
        assert_eq!(rig.opens(), 1, "the same connection");
        assert!(lease.capabilities().ping, "a connection that pings");
    }

    // covers: docs/parity/tools.md TOOL-CC-IDLE-PING / a failed ping replaces the connection
    #[test]
    fn connection_cache_replaces_a_connection_whose_ping_fails() {
        let rig = ModbusRig::with(|builder| builder.ping_timeout(secs(1)));
        // The first connection never answers its ping; the second is fresh.
        rig.factory
            .push_script(Script::strict(vec![Step::Expect(ping_request(1))]));
        rig.factory.push_script(Script::strict(Vec::new()));
        drop(rig.get().expect("first lease"));
        rig.clock.advance(secs(30));
        let start = rig.clock.now();
        let lease = rig.get().expect("a replacement");
        assert_eq!(
            rig.clock.now().duration_since(start),
            secs(1),
            "the ping timeout"
        );
        assert_eq!(rig.opens(), 2, "a second connection");
        assert_eq!(rig.sent(0), vec![ping_request(1)], "the failed ping");
        assert_eq!(lease.state(), ConnectionState::Connected);
    }

    // covers: docs/parity/tools.md TOOL-CC-IDLE-PING / a lease that expired forces the ping
    #[test]
    fn connection_cache_pings_a_connection_whose_lease_expired() {
        let rig = ModbusRig::with(|builder| {
            builder
                .max_lease_time(secs(10))
                .idle_ping_threshold(secs(600))
        });
        rig.factory.push_script(Script::strict(vec![
            Step::Expect(ping_request(1)),
            Step::Reply(ping_reply(1)),
        ]));
        let lease = rig.get().expect("first lease");
        rig.clock.advance(secs(11));
        drop(lease);
        drop(rig.get().expect("pinged, then lent"));
        assert_eq!(
            rig.sent(0),
            vec![ping_request(1)],
            "pinged although not idle long"
        );
        drop(rig.get().expect("good again: no ping"));
        assert_eq!(rig.sent(0).len(), 1, "one ping only");
    }

    // covers: docs/parity/tools.md TOOL-CC-IDLE-PING / no ping without Capabilities.ping
    #[test]
    fn connection_cache_skips_the_ping_for_a_driver_without_ping() {
        let rig = SimRig::new();
        let lease = rig.get(SLOW).expect("lease");
        assert!(!lease.capabilities().ping, "the simulator does not ping");
        drop(lease);
        rig.clock.advance(secs(60));
        let (lease, took) = rig.timed_get(SLOW);
        let lease = lease.expect("reused");
        assert_eq!(took, Duration::ZERO, "no ping and no new connection");

        // A failure on first use marks the connection, and the next lease replaces it.
        rig.device
            .push_failure(FailureStep::Disconnect)
            .expect("step");
        assert_eq!(
            kind(rig.read(&lease)),
            ErrorKind::Disconnected,
            "the link is lost"
        );
        let old = lease.watch_state();
        drop(lease);
        let (lease, took) = rig.timed_get(SLOW);
        let lease = lease.expect("a replacement");
        assert_eq!(
            took,
            ms(20),
            "the failed one closed, then a new one opened, each waiting the latency"
        );
        assert_eq!(
            old.current(),
            ConnectionState::Closed,
            "the failed connection was closed"
        );
        assert_eq!(rig.read(&lease).expect("read"), PlcValue::F32(71.5));

        // A timeout through the lease marks it too; the replacement closes the old one.
        rig.device.push_failure(FailureStep::Timeout).expect("step");
        assert_eq!(kind(rig.read(&lease)), ErrorKind::Timeout);
        drop(lease);
        let (_lease, took) = rig.timed_get(SLOW);
        assert_eq!(
            took,
            ms(20),
            "a close and a connect, each waiting the latency"
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-TIMEOUTS / the idle timeout
    #[test]
    fn connection_cache_idle_timeout_closes_an_unused_connection() {
        let rig = SimRig::with(|builder| builder.max_idle_time(secs(60)));
        let lease = rig.get(BENCH).expect("lease");
        let watch = lease.watch_state();
        drop(lease);
        rig.clock.advance(secs(59));
        drop(rig.get(SLOW).expect("another string"));
        assert_eq!(rig.cache.len(), 2, "not idle long enough");
        rig.clock.advance(secs(1));
        assert_eq!(rig.cache.len(), 2, "noticed only when the cache is called");
        let other = rig.get(SLOW).expect("another string again");
        assert_eq!(
            rig.cache.connection_strings(),
            vec![SLOW.to_owned()],
            "evicted"
        );
        assert_eq!(watch.current(), ConnectionState::Closed, "and closed");
        drop(other);
        let (lease, took) = rig.timed_get(BENCH);
        assert!(lease.is_ok(), "the next get opens a new one");
        assert_eq!(took, Duration::ZERO, "no latency on this string");
        assert_eq!(rig.cache.len(), 2);
    }

    // covers: docs/parity/tools.md TOOL-CC-TIMEOUTS / connect bounded by timeouts.connect
    #[test]
    fn connection_cache_connect_is_bounded_by_the_connect_timeout() {
        let rig = SimRig::new();
        let (lease, took) = rig.timed_get("sim://bench?latency-ms=60000");
        assert_eq!(
            kind(lease),
            ErrorKind::Timeout,
            "the driver's connect timeout"
        );
        assert_eq!(took, secs(5), "timeouts.connect, the default");
        assert!(
            rig.cache.is_empty(),
            "a failed first connect leaves no entry"
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-TIMEOUTS / close bounded by the close timeout
    #[test]
    fn connection_cache_close_is_bounded_by_the_close_timeout() {
        let rig = SimRig::with(|builder| builder.close_timeout(secs(1)));
        let uri = "sim://bench?latency-ms=3000";
        let lease = rig.get(uri).expect("lease");
        let watch = lease.watch_state();
        drop(lease);
        let start = rig.clock.now();
        assert!(block_on_with(&rig.clock, rig.cache.remove(uri)).expect("remove"));
        assert_eq!(
            rig.clock.now().duration_since(start),
            secs(1),
            "abandoned at the close timeout"
        );
        assert_eq!(
            watch.current(),
            ConnectionState::Closing,
            "the close did not finish"
        );
        assert!(rig.cache.is_empty());
    }

    // covers: docs/parity/tools.md TOOL-CC-BUILDER / zero and unrepresentable durations
    #[test]
    fn connection_cache_builder_rejects_a_zero_lease_time() {
        type Setting = fn(ConnectionCacheBuilder, Duration) -> ConnectionCacheBuilder;
        let settings: [(&str, Setting); 6] = [
            ("max_idle_time", ConnectionCacheBuilder::max_idle_time),
            ("max_lease_time", ConnectionCacheBuilder::max_lease_time),
            ("ping_timeout", ConnectionCacheBuilder::ping_timeout),
            ("close_timeout", ConnectionCacheBuilder::close_timeout),
            ("max_wait_time", ConnectionCacheBuilder::max_wait_time),
            (
                "idle_ping_threshold",
                ConnectionCacheBuilder::idle_ping_threshold,
            ),
        ];
        let builder = || {
            let clock = FakeClock::default();
            ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(clock))
        };
        for (index, (name, set)) in settings.into_iter().enumerate() {
            let zero = set(builder(), Duration::ZERO).build();
            if index < 4 {
                let refused = error(zero);
                assert_eq!(refused.kind(), ErrorKind::InvalidConfig, "{name}");
                assert!(refused.to_string().contains(name), "{name}: {refused}");
            } else {
                assert!(zero.is_ok(), "{name} may be zero");
            }
            let huge = error(set(builder(), Duration::MAX).build());
            assert_eq!(huge.kind(), ErrorKind::InvalidConfig, "{name}");
            assert!(
                huge.to_string().contains("does not fit the clock"),
                "{name}: {huge}"
            );
        }
        let debug = format!("{:?}", builder().max_lease_time(secs(7)));
        assert!(debug.contains("max_lease_time: 7s"), "{debug}");
    }

    // covers: docs/parity/tools.md TOOL-CC-ADMIN / the counts and the list
    #[test]
    fn connection_cache_counts_cached_connections_and_active_leases() {
        let rig = SimRig::new();
        assert!(rig.cache.is_empty());
        assert_eq!((rig.cache.len(), rig.cache.active_leases()), (0, 0));
        let slow = rig.get(SLOW).expect("slow");
        let plain = rig.get(BENCH).expect("plain");
        assert_eq!((rig.cache.len(), rig.cache.active_leases()), (2, 2));
        drop(slow);
        assert_eq!((rig.cache.len(), rig.cache.active_leases()), (2, 1));
        let mut waiter = rig.cache.get(BENCH);
        assert!(poll_once(&mut waiter).is_pending(), "queued");
        assert_eq!(rig.cache.active_leases(), 1, "a waiter is not a lease");
        assert_eq!(
            rig.cache.connection_strings(),
            vec![BENCH.to_owned(), SLOW.to_owned()],
            "sorted"
        );
        let debug = format!("{:?}", rig.cache);
        assert!(
            debug.contains(SLOW) && debug.contains("max_pending: 16"),
            "{debug}"
        );
        let debug = format!("{plain:?}");
        assert!(
            debug.contains("LeasedConnection") && debug.contains(BENCH),
            "{debug}"
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-ADMIN / remove force-closes a leased connection
    #[test]
    fn connection_cache_remove_force_closes_the_connection() {
        let rig = SimRig::new();
        let lease = rig.get(BENCH).expect("lease");
        let mut waiter = rig.cache.get(BENCH);
        assert!(poll_once(&mut waiter).is_pending(), "queued");
        assert!(
            block_on_with(&rig.clock, rig.cache.remove(BENCH)).expect("remove"),
            "it was cached"
        );
        assert_eq!(
            lease.state(),
            ConnectionState::Closed,
            "force-closed while leased"
        );
        assert_eq!(
            kind(rig.read(&lease)),
            ErrorKind::Disconnected,
            "the lease is refused"
        );
        assert_eq!(rig.reads(), 0, "nothing reached the device");
        let Poll::Ready(Err(error)) = poll_once(&mut waiter) else {
            panic!("the waiter of the removed string fails");
        };
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert!(rig.cache.is_empty());
        assert!(
            !block_on_with(&rig.clock, rig.cache.remove(BENCH)).expect("again"),
            "gone"
        );

        // The next get opens a new connection, which the old lease cannot reach. A caller
        // that waited for an entry that was removed, and asked again for meanwhile, is not
        // in the new entry's queue.
        let fresh = rig.get(BENCH).expect("a new connection");
        let mut old_waiter = rig.cache.get(BENCH);
        assert!(poll_once(&mut old_waiter).is_pending(), "queued");
        assert!(block_on_with(&rig.clock, rig.cache.remove(BENCH)).expect("remove"));
        let renewed = rig.get(BENCH).expect("a third connection");
        let Poll::Ready(Err(error)) = poll_once(&mut old_waiter) else {
            panic!("a waiter of a removed entry is not in the new one");
        };
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        drop((lease, fresh));
        assert_eq!(
            rig.cache.active_leases(),
            1,
            "old leases' drops change nothing"
        );
        assert_eq!(rig.read(&renewed).expect("read"), PlcValue::F32(71.5));
    }

    // covers: SPEC mandatory-failure-cases / cancellation while connecting (removed meanwhile)
    #[test]
    fn connection_cache_remove_during_connect_closes_the_new_connection() {
        let rig = SimRig::new();
        let uri = "sim://bench?latency-ms=3000";
        let mut connecting = rig.cache.get(uri);
        assert!(poll_once(&mut connecting).is_pending(), "connecting");
        assert_eq!(rig.cache.len(), 1, "an entry while connecting");
        assert_eq!(rig.cache.active_leases(), 0, "not a lease yet");
        assert!(
            block_on_with(&rig.clock, rig.cache.remove(uri)).expect("remove"),
            "it was there"
        );
        let refused = error(block_on_with(&rig.clock, connecting));
        assert_eq!(refused.kind(), ErrorKind::Disconnected);
        assert!(refused.to_string().contains("removed"), "{refused}");
        assert!(rig.cache.is_empty(), "the late connection is not cached");
    }

    // covers: SPEC mandatory-failure-cases / cancellation while connecting (removed during the replacement's close)
    #[test]
    fn connection_cache_remove_during_a_replacement_close_opens_nothing() {
        let rig = SimRig::new();
        let uri = "sim://bench?latency-ms=3000";
        let lease = rig.get(uri).expect("lease");
        rig.device
            .push_failure(FailureStep::Disconnect)
            .expect("step");
        assert_eq!(kind(rig.read(&lease)), ErrorKind::Disconnected, "broken");
        let old = lease.watch_state();
        drop(lease);
        let start = rig.clock.now();
        let mut replacing = rig.cache.get(uri);
        assert!(
            poll_once(&mut replacing).is_pending(),
            "closing the old one"
        );
        assert!(
            block_on_with(&rig.clock, rig.cache.remove(uri)).expect("remove"),
            "it was there"
        );
        let refused = error(block_on_with(&rig.clock, replacing));
        assert_eq!(refused.kind(), ErrorKind::Disconnected);
        assert!(refused.to_string().contains("removed"), "{refused}");
        assert_eq!(
            rig.clock.now().duration_since(start),
            secs(3),
            "the old one's close and no connect after it"
        );
        assert_eq!(old.current(), ConnectionState::Closed, "the old one closed");
        assert!(rig.cache.is_empty(), "nothing cached");
    }

    // covers: SPEC mandatory-failure-cases / explicit shutdown while a ping runs
    #[test]
    fn connection_cache_close_during_an_idle_ping_fails_the_caller() {
        let rig = ModbusRig::with(|builder| builder.ping_timeout(secs(1)));
        rig.factory
            .push_script(Script::strict(vec![Step::Expect(ping_request(1))]));
        drop(rig.get().expect("first lease"));
        rig.clock.advance(secs(30));
        let mut pinging = rig.cache.get(MODBUS);
        assert!(poll_once(&mut pinging).is_pending(), "pinging");
        block_on_with(&rig.clock, rig.cache.close()).expect("close");
        assert_eq!(
            kind(block_on_with(&rig.clock, pinging)),
            ErrorKind::Disconnected
        );
        assert_eq!(rig.opens(), 1, "nothing reopened");
    }

    // covers: docs/parity/tools.md TOOL-CC-OPERATIONS / the lease passes calls through
    #[test]
    fn connection_cache_lease_passes_a_read_through_to_the_connection() {
        let rig = SimRig::new();
        let lease = rig.get(BENCH).expect("lease");
        assert_eq!(rig.read(&lease).expect("read"), PlcValue::F32(71.5));
        assert_eq!(rig.reads(), 1, "the device served it");
        let tag = boiler();
        let write = WriteRequest::builder(lease.limits())
            .item(tag.clone(), PlcValue::F32(72.0))
            .build()
            .expect("write");
        let written = block_on_with(&rig.clock, lease.write(write)).expect("write");
        assert_eq!(written.confirmed_count(), 1);
        assert_eq!(rig.read(&lease).expect("read"), PlcValue::F32(72.0));
        assert_eq!(
            kind(block_on_with(&rig.clock, lease.ping())),
            ErrorKind::Unsupported(Operation::Ping),
            "the simulator's own answer"
        );
        let request = SubscribeRequest::builder(
            lease.limits(),
            SubscriptionKind::Cyclic(secs(1)),
            QueuePolicy::Backpressure,
        )
        .tag(tag)
        .build()
        .expect("subscribe request");
        let subscription = block_on_with(&rig.clock, lease.subscribe(request)).expect("subscribe");
        assert_eq!(subscription.mode(), SubscriptionMode::Polling);
        assert!(lease.capabilities().read && lease.capabilities().write);
        assert_eq!(lease.state(), ConnectionState::Connected);
        assert_eq!(
            rig.cache.active_leases(),
            1,
            "an unsupported ping marks nothing"
        );
    }

    /// A clock whose sleeps resolve at once, as the simulator's `RealtimeClock` does.
    #[test]
    fn a_waiter_on_a_clock_that_never_sleeps_waits_for_a_wake_up_without_spinning() {
        let clock: Arc<dyn Clock> = Arc::new(RealtimeClock::new());
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::new(
                SimDriver::new()
                    .with_device("bench", SimDevice::new())
                    .expect("device"),
            ))
            .expect("register");
        let cache = ConnectionCache::builder(Arc::new(registry), clock)
            .build()
            .expect("cache");
        let lease = stonecrab::testkit::block_on(cache.get(BENCH)).expect("lease");
        let waker = CountingWaker::new();
        let mut waiter: OpFuture<'_, LeasedConnection> = cache.get(BENCH);
        assert!(
            poll_once_with(&mut waiter, &waker.waker()).is_pending(),
            "parked"
        );
        assert!(
            poll_once_with(&mut waiter, &waker.waker()).is_pending(),
            "still parked"
        );
        assert_eq!(waker.count(), 0, "it does not wake itself");
        drop(lease);
        assert!(waker.count() >= 1, "the return wakes it");
        assert!(matches!(poll_once(&mut waiter), Poll::Ready(Ok(_))));
    }
}
