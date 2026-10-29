//! `ConnectionGuard` over `FakeTransportFactory`, `FakeTransport`, `FakeClock` and the
//! testkit's `LengthPrefixedFramer`.
//!
//! Timing is on the fake clock only (decision D21): nothing here sleeps, and a future that
//! could never finish fails the test instead of hanging it. Frames are `Vec<u8>` payloads;
//! the wire form of `abc` is `00 03 61 62 63`. Lifecycle sequences are collected with a
//! waker that reads the state on every `StateCell::set`, so nothing is coalesced.
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::task::{Poll, Wake, Waker};
    use std::time::{Duration, Instant};

    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, Step, block_on_with,
        poll_once, poll_once_with,
    };
    use stonecrab::transport::{ConnectionGuard, GuardConfig, GuardStats};
    use stonecrab::{
        CancellationToken, Clock, ConnectionConfig, ConnectionState, Deadline, Endpoint, ErrorKind,
        FailureCategory, OpFuture, Operation, PlcError, ReconnectPolicy, ResourceLimits, Result,
        StateWatch,
    };

    type Guard = ConnectionGuard<Vec<u8>, LengthPrefixedFramer>;

    const MS: Duration = Duration::from_millis(1);

    fn ms(count: u32) -> Duration {
        MS.saturating_mul(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    /// The wire form of `payload` under the test framer.
    fn wire(payload: &[u8]) -> Vec<u8> {
        let length = u16::try_from(payload.len()).expect("test payloads are short");
        let mut bytes = length.to_be_bytes().to_vec();
        bytes.extend_from_slice(payload);
        bytes
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    /// Records every state the guard sets, in order. The waker runs inside
    /// `StateCell::set`, reads the state that was just set and re-arms itself, so two
    /// sets between polls are both kept (a bare `StateWatch` would coalesce them).
    struct StateLog {
        seen: Mutex<Vec<ConnectionState>>,
        watch: &'static StateWatch,
        armed: Mutex<Option<OpFuture<'static, ConnectionState>>>,
    }

    impl StateLog {
        fn start(watch: StateWatch) -> Arc<Self> {
            // Leaked so the change future can be 'static; a test process is short-lived.
            let watch: &'static StateWatch = Box::leak(Box::new(watch));
            let log = Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                watch,
                armed: Mutex::new(None),
            });
            log.arm();
            log
        }

        fn arm(self: &Arc<Self>) {
            let waker = Waker::from(Arc::clone(self));
            let mut change = self.watch.changed();
            if poll_once_with(&mut change, &waker).is_ready() {
                change = self.watch.changed();
                assert!(
                    poll_once_with(&mut change, &waker).is_pending(),
                    "a consumed change leaves the watch pending"
                );
            }
            *self.armed.lock().unwrap_or_else(PoisonError::into_inner) = Some(change);
        }

        fn states(&self) -> Vec<ConnectionState> {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    impl Wake for StateLog {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(self.watch.current());
            self.arm();
        }
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        guard: Guard,
        log: Arc<StateLog>,
        t0: Instant,
    }

    /// Request timeout 3 s, connect timeout 2 s, frame limit 256, buffer 1024.
    fn guard_config(pending: usize, reconnect: ReconnectPolicy) -> GuardConfig {
        let limits = ResourceLimits::builder()
            .max_frame_bytes(256)
            .max_buffered_bytes(1024)
            .max_pending_requests(pending)
            .build()
            .expect("valid limits");
        let mut config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
        config.timeouts.connect = secs(2);
        config.timeouts.request = secs(3);
        config.reconnect = reconnect;
        config
    }

    fn policy(initial: u32, max: u32, max_attempts: Option<u32>) -> ReconnectPolicy {
        let mut policy = ReconnectPolicy::default();
        policy.initial = ms(initial);
        policy.max = ms(max);
        policy.max_attempts = max_attempts.and_then(std::num::NonZeroU32::new);
        policy
    }

    impl Rig {
        /// Backoff 250 ms doubling up to 30 s, sample 0.5 (no jitter), four places.
        fn new() -> Self {
            Self::with(4, policy(250, 30_000, None), 0.5)
        }

        fn with(pending: usize, reconnect: ReconnectPolicy, sample: f64) -> Self {
            Self::with_config(guard_config(pending, reconnect), sample)
        }

        fn with_config(config: GuardConfig, sample: f64) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            let guard = ConnectionGuard::new(
                config,
                Arc::new(factory.clone()),
                Box::new(|| LengthPrefixedFramer::new(64)),
                Arc::new(clock.clone()),
            )
            .with_jitter_source(move || sample);
            let log = StateLog::start(guard.watch_state());
            let t0 = clock.now();
            Self {
                clock,
                factory,
                guard,
                log,
                t0,
            }
        }

        fn deadline(&self, seconds: u64) -> Deadline {
            Deadline::after(self.clock.now(), secs(seconds)).expect("representable")
        }

        fn connect(&self) -> Result<()> {
            block_on_with(&self.clock, self.guard.connect())
        }

        /// One request with a deadline `seconds` from now, driven to completion.
        fn request(&self, payload: &[u8], seconds: u64) -> Result<Vec<u8>> {
            let deadline = self.deadline(seconds);
            block_on_with(
                &self.clock,
                self.guard.request(payload.to_vec(), deadline, None),
            )
        }

        fn push(&self, steps: Vec<Step>) {
            self.factory.push_script(Script::strict(steps));
        }

        /// Connect to a transport that answers `ping` with `pong`, once.
        fn connected() -> Self {
            let rig = Self::new();
            rig.push(vec![
                Step::Expect(wire(b"ping")),
                Step::Reply(wire(b"pong")),
            ]);
            rig.connect().expect("connect");
            rig
        }

        /// A connected rig whose peer reads `first`, then closes without answering.
        fn dropped_after(first: &[u8]) -> Self {
            let rig = Self::new();
            rig.push(vec![Step::Expect(wire(first)), Step::CloseAt(0)]);
            rig.connect().expect("connect");
            rig
        }

        fn states(&self) -> Vec<ConnectionState> {
            self.log.states()
        }

        fn shut_down(&self, transport: usize) -> bool {
            self.factory
                .probes()
                .get(transport)
                .expect("transport was opened")
                .is_shut_down()
        }

        fn at(&self, millis: u32) -> Instant {
            self.t0.checked_add(ms(millis)).expect("representable")
        }

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

    fn reconnecting(attempt: u32, next_at: Instant) -> ConnectionState {
        ConnectionState::Reconnecting { attempt, next_at }
    }

    #[test]
    fn connecting_walks_disconnected_connecting_connected_and_requests_round_trip() {
        let rig = Rig::connected();
        assert_eq!(
            rig.states(),
            vec![ConnectionState::Connecting, ConnectionState::Connected],
            "state sequence of connect"
        );
        let open = rig.factory.opens().remove(0);
        assert_eq!(
            open.deadline.instant(),
            rig.at(2000),
            "the open is bounded by timeouts.connect"
        );
        assert_eq!(
            open.endpoint,
            Endpoint::Custom("fake://plc".to_owned()),
            "endpoint"
        );
        assert_eq!(open.limits.max_pending_requests().get(), 4, "limits");

        rig.connect().expect("connect while connected does nothing");
        assert_eq!(rig.factory.opens().len(), 1, "no second open");

        assert_eq!(rig.request(b"ping", 1).expect("reply"), frame(b"pong"));
        assert_eq!(rig.sent(0), vec![wire(b"ping")], "one frame on the wire");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 0),
            "one request, nothing else"
        );
        assert_eq!(rig.states().len(), 2, "a request changes no state");
    }

    /// A frame as the guard hands it over: the payload, without the length prefix.
    fn frame(payload: &[u8]) -> Vec<u8> {
        payload.to_vec()
    }

    fn stats(reconnects: u64, resyncs: u64, requests: u64, timeouts: u64) -> GuardStats {
        let mut stats = GuardStats::default();
        stats.reconnects = reconnects;
        stats.resyncs = resyncs;
        stats.requests = requests;
        stats.timeouts = timeouts;
        stats
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (outage and recovery)
    #[test]
    fn the_backoff_schedule_is_250_500_1000_then_the_cap_at_the_sample_one_half() {
        let rig = Rig::with(4, policy(250, 1000, None), 0.5);
        for _ in 0..4 {
            rig.factory.push_refusal();
        }
        let mut errors = Vec::new();
        for _ in 0..4 {
            errors.push(error_of(rig.connect()));
        }
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                reconnecting(2, rig.at(750)),
                ConnectionState::Connecting,
                reconnecting(3, rig.at(1750)),
                ConnectionState::Connecting,
                reconnecting(4, rig.at(2750)),
            ],
            "delays 250, 500, 1000 and 1000 (capped), each measured from the failure"
        );
        let kinds: Vec<_> = errors.iter().map(PlcError::kind).collect();
        assert_eq!(kinds, vec![ErrorKind::Disconnected; 4], "refusals");
        let retries: Vec<_> = errors.iter().map(|e| e.diagnostics().retries).collect();
        assert_eq!(
            retries,
            vec![0, 1, 2, 3],
            "the reconnect attempt in diagnostics"
        );
        let waited: Vec<_> = errors
            .iter()
            .map(|e| e.diagnostics().latency.expect("latency"))
            .collect();
        assert_eq!(
            waited,
            vec![Duration::ZERO, ms(250), ms(500), ms(1000)],
            "each connect call spent the previous backoff asleep on the clock"
        );
        let first = errors.first().expect("first error").diagnostics();
        assert_eq!(first.operation, Some(Operation::Connect), "operation");
        assert_eq!(first.category, Some(FailureCategory::Transport), "category");
        assert_eq!(
            first.connection_state,
            Some(ConnectionState::Connecting),
            "state at the failure"
        );
        assert_eq!(rig.factory.opens().len(), 4, "one open per attempt");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (outage and recovery)
    #[test]
    fn jitter_samples_zero_and_one_bound_each_delay() {
        // initial 250 ms doubling, cap 1000 ms, jitter 0.2: the cap applies before jitter.
        let cases = [
            (0.0, [200_u32, 400, 800, 800]),
            (1.0, [300, 600, 1200, 1200]),
        ];
        for (sample, delays) in cases {
            let rig = Rig::with(4, policy(250, 1000, None), sample);
            for _ in 0..4 {
                rig.factory.push_refusal();
            }
            for _ in 0..4 {
                error_of(rig.connect());
            }
            let mut expected = Vec::new();
            let mut at = 0_u32;
            for (attempt, delay) in (1..).zip(delays) {
                expected.push(ConnectionState::Connecting);
                at = at.saturating_add(delay);
                expected.push(reconnecting(attempt, rig.at(at)));
            }
            assert_eq!(
                rig.states(),
                expected,
                "delays {delays:?} at sample {sample}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (exhausted budget)
    #[test]
    fn an_exhausted_retry_budget_fails_the_guard_and_every_later_request() {
        let rig = Rig::with(4, policy(250, 30_000, Some(2)), 0.5);
        rig.push(vec![Step::Expect(wire(b"a")), Step::CloseAt(0)]);
        rig.connect().expect("connect");
        rig.factory.push_refusal();
        rig.factory.push_refusal();

        let lost = error_of(rig.request(b"a", 10));
        assert_eq!(lost.kind(), ErrorKind::Disconnected, "peer closed");
        let first = error_of(rig.request(b"b", 10));
        assert_eq!(first.kind(), ErrorKind::Disconnected, "first refusal");
        let last = error_of(rig.request(b"c", 10));
        assert_eq!(last.kind(), ErrorKind::Disconnected, "second refusal");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "two retries were allowed, the budget is spent"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                reconnecting(2, rig.at(750)),
                ConnectionState::Connecting,
                ConnectionState::Failed(ErrorKind::Disconnected),
            ],
            "state sequence"
        );

        let opens = rig.factory.opens().len();
        let later = error_of(rig.request(b"d", 10));
        assert_eq!(later.kind(), ErrorKind::Disconnected, "later request");
        assert_eq!(
            later.diagnostics().connection_state,
            Some(ConnectionState::Failed(ErrorKind::Disconnected)),
            "state at the failure"
        );
        assert_eq!(rig.factory.opens().len(), opens, "nothing was opened");
        assert_eq!(
            error_of(rig.connect()).kind(),
            ErrorKind::Disconnected,
            "connect on a failed guard"
        );
        block_on_with(&rig.clock, rig.guard.close()).expect("close is a no-op");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "terminal states are never reused"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (exhausted budget)
    #[test]
    fn an_open_failure_that_is_not_transient_fails_with_that_kind() {
        let rig = Rig::new();
        // Nothing queued: the double answers `Internal`, which no retry can fix.
        let error = error_of(rig.connect());
        assert_eq!(error.kind(), ErrorKind::Internal, "kind passes through");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Failed(ErrorKind::Internal),
            "failed, not reconnecting"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Failed(ErrorKind::Internal)
            ],
            "state sequence"
        );
    }

    #[test]
    fn reading_a_further_frame_after_the_transport_was_lost_is_disconnected_without_a_reopen() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        let before = rig.guard.state();
        let deadline = rig.deadline(10);
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard.next_reply(deadline, None),
        ));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "the reply is gone");
        assert_eq!(rig.guard.state(), before, "no reopen, the schedule is kept");
        assert_eq!(rig.guard.stats().reconnects, 0, "nothing was opened");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (exhausted budget)
    #[test]
    fn a_peer_that_accepts_and_then_closes_spends_the_retry_budget() {
        let rig = Rig::with(4, policy(250, 30_000, Some(3)), 0.5);
        rig.push(vec![Step::Expect(wire(b"a")), Step::CloseAt(0)]);
        rig.connect().expect("connect");
        for _ in 0..3 {
            rig.push(vec![Step::Expect(wire(b"x")), Step::CloseAt(0)]);
        }
        let first = error_of(rig.request(b"a", 10));
        assert_eq!(first.kind(), ErrorKind::Disconnected, "first close");
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "attempt 1, 250 ms"
        );
        let second = error_of(rig.request(b"x", 10));
        assert_eq!(second.kind(), ErrorKind::Disconnected, "second close");
        assert_eq!(
            rig.guard.state(),
            reconnecting(2, rig.at(750)),
            "an open alone does not reset the schedule"
        );
        let third = error_of(rig.request(b"x", 10));
        assert_eq!(third.kind(), ErrorKind::Disconnected, "third close");
        assert_eq!(
            rig.guard.state(),
            reconnecting(3, rig.at(1750)),
            "attempt 3, 1000 ms"
        );
        let last = error_of(rig.request(b"x", 10));
        assert_eq!(last.kind(), ErrorKind::Disconnected, "fourth close");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "the budget is spent"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (connecting)
    #[test]
    fn an_open_cut_short_by_the_callers_deadline_does_not_spend_the_retry_budget() {
        let rig = Rig::with(4, policy(250, 30_000, Some(1)), 0.5);
        rig.push(vec![Step::Expect(wire(b"a")), Step::CloseAt(0)]);
        rig.connect().expect("connect");
        error_of(rig.request(b"a", 10));
        rig.factory.push_connect_timeout();
        let error = error_of(rig.request(b"b", 1));
        assert_eq!(error.kind(), ErrorKind::Timeout, "the caller's deadline");
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "the schedule is kept and the budget is not spent"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[test]
    fn close_during_the_backoff_ends_closed_and_nothing_is_opened_again() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "backing off"
        );

        block_on_with(&rig.clock, rig.guard.close()).expect("close");
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Closing,
                ConnectionState::Closed
            ],
            "state sequence"
        );
        assert_eq!(
            rig.clock.now(),
            rig.t0,
            "close did not wait out the backoff"
        );
        let after = error_of(rig.request(b"b", 10));
        assert_eq!(after.kind(), ErrorKind::Disconnected, "request after close");
        assert_eq!(
            after.diagnostics().connection_state,
            Some(ConnectionState::Closed),
            "state at the failure"
        );
        assert_eq!(
            error_of(rig.connect()).kind(),
            ErrorKind::Disconnected,
            "connect"
        );
        assert_eq!(rig.factory.opens().len(), 1, "no reconnect after close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[test]
    fn close_interrupts_a_request_that_is_sleeping_in_the_backoff() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        let deadline = rig.deadline(10);
        let mut waiting = rig.guard.request(wire(b"b"), deadline, None);
        assert!(poll_once(&mut waiting).is_pending(), "asleep until next_at");
        let mut closing = rig.guard.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "close waits for the slot"
        );
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Closing,
            "closing at once"
        );

        let error = error_of(ready(poll_once(&mut waiting)));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "the sleeper is woken"
        );
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Closing),
            "state at the failure"
        );
        ready(poll_once(&mut closing)).expect("close completes");
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Closing,
                ConnectionState::Closed
            ],
            "state sequence"
        );
        assert_eq!(rig.clock.now(), rig.t0, "nothing waited for the backoff");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[test]
    fn close_interrupts_a_connect_that_waits_out_the_backoff() {
        let rig = Rig::new();
        rig.factory.push_refusal();
        error_of(rig.connect());
        let mut connecting = rig.guard.connect();
        assert!(
            poll_once(&mut connecting).is_pending(),
            "asleep until next_at"
        );
        let mut closing = rig.guard.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "close waits for the slot"
        );
        let error = error_of(ready(poll_once(&mut connecting)));
        ready(poll_once(&mut closing)).expect("close");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "connect is cut short"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );
        assert_eq!(rig.guard.state(), ConnectionState::Closed, "closed");
    }

    fn ready<T>(poll: Poll<T>) -> T {
        match poll {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("expected the future to be finished"),
        }
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (connecting)
    #[test]
    fn a_connect_deadline_that_passes_is_a_timeout_with_connect_diagnostics() {
        let rig = Rig::new();
        rig.factory.push_connect_timeout();
        let error = error_of(rig.connect());
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Connect), "operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connecting),
            "state at the failure"
        );
        assert_eq!(
            diagnostics.latency,
            Some(secs(2)),
            "latency is timeouts.connect"
        );
        assert_eq!(diagnostics.retries, 0, "first attempt");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert_eq!(
            rig.clock.now(),
            rig.at(2000),
            "the fake clock stopped at the deadline"
        );
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(2250)),
            "a timeout leaves a retry pending"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (connecting)
    #[test]
    fn a_token_cancelled_during_the_backoff_sleep_is_cancelled_and_keeps_the_schedule() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        let token = CancellationToken::new();
        let deadline = rig.deadline(10);
        let mut waiting = rig.guard.request(wire(b"b"), deadline, Some(&token));
        assert!(poll_once(&mut waiting).is_pending(), "asleep until next_at");
        rig.clock.advance(ms(100));
        token.cancel();
        let error = error_of(ready(poll_once(&mut waiting)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            error.diagnostics().connection_state,
            Some(reconnecting(1, rig.at(250))),
            "state at the failure"
        );
        assert_eq!(error.diagnostics().latency, Some(ms(100)), "latency");
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "schedule kept"
        );
        assert_eq!(rig.factory.opens().len(), 1, "no open was attempted");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (connecting)
    #[test]
    fn a_token_cancelled_while_the_open_is_pending_restores_the_state() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        rig.factory.push_connect_timeout();
        let token = CancellationToken::new();
        let deadline = rig.deadline(10);
        let mut opening = rig.guard.request(wire(b"b"), deadline, Some(&token));
        assert!(poll_once(&mut opening).is_pending(), "backoff");
        rig.clock.advance(ms(250));
        assert!(poll_once(&mut opening).is_pending(), "the open is parked");
        assert_eq!(rig.guard.state(), ConnectionState::Connecting, "opening");
        token.cancel();
        let error = error_of(ready(poll_once(&mut opening)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Connecting),
            "state at the failure"
        );
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "back to the state before the open"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                reconnecting(1, rig.at(250))
            ],
            "state sequence"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (connecting)
    #[test]
    fn dropping_a_call_that_is_opening_restores_the_state() {
        let rig = Rig::new();
        rig.factory.push_connect_timeout();
        let mut connecting = rig.guard.connect();
        assert!(
            poll_once(&mut connecting).is_pending(),
            "the open is parked"
        );
        assert_eq!(rig.guard.state(), ConnectionState::Connecting, "opening");
        drop(connecting);
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Disconnected,
            "a dropped open leaves nothing behind"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (queued)
    #[test]
    fn a_request_cancelled_while_queued_frees_its_place_and_sends_nothing() {
        let rig = Rig::with(2, policy(250, 30_000, None), 0.5);
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::LateReply(ms(500)),
            Step::Reply(wire(b"A")),
            Step::Expect(wire(b"d")),
            Step::Reply(wire(b"D")),
        ]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(5);
        let token = CancellationToken::new();
        let mut first = rig.guard.request(frame(b"a"), deadline, None);
        let mut second = rig.guard.request(frame(b"b"), deadline, Some(&token));
        assert!(poll_once(&mut first).is_pending(), "a awaits its reply");
        assert!(poll_once(&mut second).is_pending(), "b is queued behind a");

        token.cancel();
        let error = error_of(ready(poll_once(&mut second)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "category"
        );

        // The freed place admits another request instead of refusing it.
        let mut fourth = rig.guard.request(frame(b"d"), deadline, None);
        assert!(
            poll_once(&mut fourth).is_pending(),
            "d is queued, not refused"
        );
        rig.clock.advance(ms(500));
        assert_eq!(ready(poll_once(&mut first)).expect("a"), frame(b"A"));
        assert_eq!(ready(poll_once(&mut fourth)).expect("d"), frame(b"D"));
        assert_eq!(
            rig.sent(0),
            vec![wire(b"a"), wire(b"d")],
            "the cancelled request never reached the wire"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (transmitting)
    #[test]
    fn a_token_cancelled_before_the_flush_is_cancelled_and_nothing_is_sent() {
        let rig = Rig::connected();
        let token = CancellationToken::new();
        token.cancel();
        let deadline = rig.deadline(1);
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard.request(frame(b"ping"), deadline, Some(&token)),
        ));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "rejected before any I/O"
        );
        assert!(rig.sent(0).is_empty(), "sent() is empty");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "state unchanged"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 0, 0),
            "nothing counted as sent"
        );
        assert_eq!(
            rig.request(b"ping", 1).expect("the guard is still usable"),
            frame(b"pong")
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting)
    #[test]
    fn a_token_cancelled_while_awaiting_the_reply_is_cancelled_after_the_wire() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::LateReply(secs(2)),
            Step::Reply(wire(b"A")),
        ]);
        rig.connect().expect("connect");
        let token = CancellationToken::new();
        let deadline = rig.deadline(5);
        let mut request = rig.guard.request(frame(b"a"), deadline, Some(&token));
        assert!(poll_once(&mut request).is_pending(), "awaiting the reply");
        rig.clock.advance(ms(100));
        token.cancel();
        let error = error_of(ready(poll_once(&mut request)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(error.diagnostics().latency, Some(ms(100)), "latency");
        assert_eq!(
            rig.sent(0),
            vec![wire(b"a")],
            "the frame did reach the wire"
        );
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "the stream is intact"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 0),
            "counted as sent, not as a timeout"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (queued)
    #[test]
    fn requests_beyond_max_pending_requests_fail_with_backpressure_before_any_io() {
        let rig = Rig::with(2, policy(250, 30_000, None), 0.5);
        rig.push(vec![Step::Expect(wire(b"a"))]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(5);
        let mut first = rig.guard.request(frame(b"a"), deadline, None);
        let mut second = rig.guard.request(frame(b"b"), deadline, None);
        assert!(poll_once(&mut first).is_pending(), "a is in flight");
        assert!(poll_once(&mut second).is_pending(), "b is queued");

        let mut third = rig.guard.request(frame(b"c"), deadline, None);
        let error = error_of(ready(poll_once(&mut third)));
        assert_eq!(error.kind(), ErrorKind::Backpressure, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(
            rig.sent(0),
            vec![wire(b"a")],
            "c was refused before any I/O"
        );
        assert!(
            error.is_retryable(),
            "backpressure is retryable by the read policy"
        );
    }

    #[test]
    fn queued_requests_take_the_slot_in_arrival_order() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::LateReply(ms(500)),
            Step::Reply(wire(b"A")),
            Step::Expect(wire(b"b")),
            Step::Reply(wire(b"B")),
            Step::Expect(wire(b"c")),
            Step::Reply(wire(b"C")),
        ]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(5);
        let mut a = rig.guard.request(frame(b"a"), deadline, None);
        let mut b = rig.guard.request(frame(b"b"), deadline, None);
        let mut c = rig.guard.request(frame(b"c"), deadline, None);
        assert!(poll_once(&mut a).is_pending(), "a in flight");
        assert!(poll_once(&mut b).is_pending(), "b queued first");
        assert!(poll_once(&mut c).is_pending(), "c queued second");
        rig.clock.advance(ms(500));
        assert_eq!(ready(poll_once(&mut a)).expect("a"), frame(b"A"));
        // Poll c before b: the slot was handed to b when a let go.
        assert!(poll_once(&mut c).is_pending(), "c still waits its turn");
        assert_eq!(ready(poll_once(&mut b)).expect("b"), frame(b"B"));
        assert_eq!(ready(poll_once(&mut c)).expect("c"), frame(b"C"));
        assert_eq!(
            rig.sent(0),
            vec![wire(b"a"), wire(b"b"), wire(b"c")],
            "arrival order"
        );
    }

    #[test]
    fn a_queued_request_whose_deadline_passes_times_out_without_sending() {
        let rig = Rig::new();
        rig.push(vec![Step::Expect(wire(b"a"))]);
        rig.connect().expect("connect");
        let long = rig.deadline(60);
        let short = rig.deadline(1);
        let mut first = rig.guard.request(frame(b"a"), long, None);
        let mut second = rig.guard.request(frame(b"b"), short, None);
        assert!(poll_once(&mut first).is_pending(), "a in flight");
        assert!(poll_once(&mut second).is_pending(), "b queued");
        rig.clock.advance(secs(1));
        let error = error_of(ready(poll_once(&mut second)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(rig.sent(0), vec![wire(b"a")], "b never reached the wire");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 1),
            "one sent, one timeout"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn timeout_after_flush_resyncs_then_the_next_request_succeeds() {
        let rig = Rig::new();
        let first_reply = wire(b"first");
        rig.push(vec![
            Step::Expect(wire(b"one")),
            // The reply starts to arrive, then stalls past the request deadline.
            Step::Reply(first_reply.get(..3).expect("prefix").to_vec()),
            Step::Delay(secs(3)),
            Step::Reply(first_reply.get(3..).expect("rest").to_vec()),
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"second")),
        ]);
        rig.connect().expect("connect");

        let timed_out = error_of(rig.request(b"one", 1));
        assert_eq!(
            timed_out.kind(),
            ErrorKind::Timeout,
            "the caller sees Timeout"
        );
        assert_eq!(timed_out.diagnostics().latency, Some(secs(1)), "latency");
        assert_eq!(
            timed_out.diagnostics().connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "the stream is intact"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 1),
            "counted as sent and as a timeout"
        );

        let reply = rig.request(b"two", 5).expect("the next request succeeds");
        assert_eq!(reply, frame(b"second"), "its own reply, not the late one");
        assert_eq!(rig.sent(0), vec![wire(b"one"), wire(b"two")], "the wire");
        assert_eq!(rig.guard.stats(), stats(0, 1, 2, 1), "one resync");
        assert_eq!(
            rig.clock.now(),
            rig.at(3000),
            "the resync waited for the rest of the late reply"
        );
        assert_eq!(rig.states().len(), 2, "the lifecycle never left Connected");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_whole_late_reply_is_dropped_by_reading_a_further_frame_without_sending() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"one")),
            Step::LateReply(secs(2)),
            Step::Reply(wire(b"late")),
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"second")),
            Step::Expect(wire(b"three")),
            Step::Reply(wire(b"third")),
        ]);
        rig.connect().expect("connect");
        assert_eq!(
            error_of(rig.request(b"one", 1)).kind(),
            ErrorKind::Timeout,
            "timeout"
        );
        assert_eq!(
            rig.guard.stats().resyncs,
            0,
            "nothing was buffered, so nothing to resync"
        );

        // The guard has no correlation: the late frame is the first one after the send.
        let reply = rig.request(b"two", 5).expect("a frame");
        assert_eq!(
            reply,
            frame(b"late"),
            "the late reply, for the session to drop"
        );
        // The session drops it and reads on, without sending, within the same deadline.
        let deadline = rig.deadline(5);
        let own =
            block_on_with(&rig.clock, rig.guard.next_reply(deadline, None)).expect("its own reply");
        assert_eq!(own, frame(b"second"), "the reply to the second request");
        assert_eq!(
            rig.request(b"three", 5).expect("in step again"),
            frame(b"third"),
            "the stream is aligned again"
        );
        assert_eq!(
            rig.sent(0),
            vec![wire(b"one"), wire(b"two"), wire(b"three")],
            "reading sent nothing"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 3, 1),
            "no resync, one timeout, reads are not requests"
        );
    }

    #[test]
    fn reading_a_further_frame_times_out_leaves_the_guard_connected_and_needs_a_connection() {
        let rig = Rig::new();
        rig.push(vec![Step::Expect(wire(b"one")), Step::Reply(wire(b"uno"))]);
        let deadline = rig.deadline(1);
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard.next_reply(deadline, None),
        ));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "not connected yet");
        rig.connect().expect("connect");
        let deadline = rig.deadline(1);
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard.next_reply(deadline, None),
        ));
        assert_eq!(error.kind(), ErrorKind::Timeout, "nothing arrives");
        assert_eq!(rig.guard.state(), ConnectionState::Connected, "state");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 0, 1),
            "a timeout, no request"
        );
        assert_eq!(rig.request(b"one", 5).expect("usable"), frame(b"uno"));
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_timeout_with_nothing_received_needs_no_resync() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"one")),
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"second")),
        ]);
        rig.connect().expect("connect");
        assert_eq!(
            error_of(rig.request(b"one", 1)).kind(),
            ErrorKind::Timeout,
            "timeout"
        );
        let reply = rig.request(b"two", 1).expect("the next request succeeds");
        assert_eq!(reply, frame(b"second"), "reply");
        assert_eq!(rig.guard.stats(), stats(0, 0, 2, 1), "no resync was needed");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn resync_that_never_sees_a_frame_boundary_fails_disconnected_at_the_deadline() {
        let rig = Rig::new();
        let partial = wire(b"never finished");
        rig.push(vec![
            Step::Expect(wire(b"one")),
            Step::Reply(partial.get(..3).expect("prefix").to_vec()),
        ]);
        rig.connect().expect("connect");
        assert_eq!(
            error_of(rig.request(b"one", 1)).kind(),
            ErrorKind::Timeout,
            "timeout"
        );

        let error = error_of(rig.request(b"two", 2));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "no boundary before the deadline"
        );
        assert_eq!(
            error.diagnostics().latency,
            Some(secs(2)),
            "failed at the deadline"
        );
        assert_eq!(
            rig.clock.now(),
            rig.at(3000),
            "the request deadline, to the instant"
        );
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(
            rig.sent(0),
            vec![wire(b"one")],
            "the second frame was never sent"
        );
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(3250)),
            "reconnecting"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(0, 1, 1, 1),
            "one resync, one timeout"
        );

        // The next request reopens the connection and succeeds.
        rig.push(vec![
            Step::Expect(wire(b"three")),
            Step::Reply(wire(b"third")),
        ]);
        assert_eq!(
            rig.request(b"three", 5).expect("recovered"),
            frame(b"third")
        );
        assert_eq!(rig.guard.stats(), stats(1, 1, 2, 1), "one reconnect");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_reply_that_violates_the_framer_is_resynced_before_the_next_request() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"one")),
            // A header declaring 65535 bytes, over the framer maximum, then a clean frame.
            Step::Reply(vec![0xFF, 0xFF, 0xFF, 0xFF]),
            Step::Reply(wire(b"debris")),
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"second")),
        ]);
        rig.connect().expect("connect");
        let error = error_of(rig.request(b"one", 1));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "the framing error passes through"
        );
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "the stream is intact"
        );
        let reply = rig.request(b"two", 1).expect("the next request succeeds");
        assert_eq!(reply, frame(b"second"), "reply");
        assert_eq!(rig.guard.stats(), stats(0, 1, 2, 0), "resynced once");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting)
    #[test]
    fn a_token_cancelled_during_a_resync_is_cancelled_and_the_resync_can_resume() {
        let rig = Rig::new();
        let partial = wire(b"abandoned");
        rig.push(vec![
            Step::Expect(wire(b"one")),
            Step::Reply(partial.get(..3).expect("prefix").to_vec()),
            Step::Delay(secs(4)),
            Step::Reply(partial.get(3..).expect("rest").to_vec()),
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"second")),
        ]);
        rig.connect().expect("connect");
        assert_eq!(
            error_of(rig.request(b"one", 1)).kind(),
            ErrorKind::Timeout,
            "timeout"
        );

        let token = CancellationToken::new();
        let deadline = rig.deadline(10);
        let mut request = rig.guard.request(frame(b"two"), deadline, Some(&token));
        assert!(poll_once(&mut request).is_pending(), "resyncing");
        token.cancel();
        let error = error_of(ready(poll_once(&mut request)));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(rig.sent(0), vec![wire(b"one")], "nothing was sent");
        assert_eq!(rig.request(b"two", 10).expect("resumed"), frame(b"second"));
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_deadline_passing_during_the_resync_times_out_the_send_and_keeps_the_stream() {
        let rig = Rig::new();
        let partial = wire(b"abandoned");
        rig.push(vec![
            Step::Expect(wire(b"one")),
            Step::Reply(partial.get(..3).expect("prefix").to_vec()),
            Step::Delay(secs(2)),
            Step::Reply(partial.get(3..).expect("rest").to_vec()),
        ]);
        rig.connect().expect("connect");
        assert_eq!(
            error_of(rig.request(b"one", 1)).kind(),
            ErrorKind::Timeout,
            "timeout"
        );

        let deadline = rig.deadline(5);
        let mut request = rig.guard.request(frame(b"two"), deadline, None);
        assert!(poll_once(&mut request).is_pending(), "resyncing");
        // The task is not scheduled until long after the deadline: both the rest of the
        // late reply and the deadline are in the past when the resync resumes.
        rig.clock.advance(secs(20));
        let error = error_of(ready(poll_once(&mut request)));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the send finds the deadline passed"
        );
        assert_eq!(rig.sent(0), vec![wire(b"one")], "nothing more was written");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "nothing was written, so the stream is still trusted"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[test]
    fn device_restart_reconnects_and_the_next_request_succeeds() {
        let rig = Rig::new();
        let reply = wire(b"before");
        rig.push(vec![
            Step::Expect(wire(b"one")),
            // The device dies halfway through its reply.
            Step::CloseAt(3),
            Step::Reply(reply),
        ]);
        rig.connect().expect("connect");

        let error = error_of(rig.request(b"one", 5));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "peer closed mid-frame"
        );
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "reconnecting"
        );

        rig.push(vec![
            Step::Expect(wire(b"two")),
            Step::Reply(wire(b"after")),
        ]);
        let reply = rig
            .request(b"two", 5)
            .expect("the restarted device answers");
        assert_eq!(reply, frame(b"after"), "a fresh stream, no leftovers");
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                ConnectionState::Connected,
            ],
            "state sequence"
        );
        assert_eq!(
            rig.clock.now(),
            rig.at(250),
            "the request waited out the backoff"
        );
        assert_eq!(rig.guard.stats(), stats(1, 0, 2, 0), "one reconnect");
        assert_eq!(
            rig.sent(1),
            vec![wire(b"two")],
            "the second transport carried it"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (outage and recovery)
    #[test]
    fn outage_and_recovery_walk_the_backoff_and_reset_it_on_success() {
        let rig = Rig::dropped_after(b"a");
        for _ in 0..3 {
            rig.factory.push_refusal();
        }
        rig.push(vec![
            Step::Expect(wire(b"e")),
            Step::Reply(wire(b"E")),
            Step::Expect(wire(b"f")),
            Step::CloseAt(0),
        ]);

        let lost = error_of(rig.request(b"a", 10));
        assert_eq!(lost.kind(), ErrorKind::Disconnected, "outage begins");
        let mut during = Vec::new();
        for payload in [b"b", b"c", b"d"] {
            during.push(error_of(rig.request(payload, 10)));
        }
        let attempts: Vec<_> = during.iter().map(|e| e.diagnostics().retries).collect();
        assert_eq!(attempts, vec![1, 2, 3], "each refusal names its attempt");
        for error in &during {
            assert_eq!(error.kind(), ErrorKind::Disconnected, "refused");
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Connect),
                "operation"
            );
        }
        assert_eq!(
            rig.guard.state(),
            reconnecting(4, rig.at(3750)),
            "250 + 500 + 1000 + 2000 ms"
        );

        assert_eq!(rig.request(b"e", 10).expect("recovered"), frame(b"E"));
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "connected again"
        );
        assert_eq!(
            rig.clock.now(),
            rig.at(3750),
            "the waits were 250, 500, 1000, 2000"
        );

        // The schedule starts over after a success.
        error_of(rig.request(b"f", 10));
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(4000)),
            "attempt 1, 250 ms"
        );
        assert_eq!(
            rig.guard.stats(),
            stats(1, 0, 3, 0),
            "one successful reconnect"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[test]
    fn close_fails_the_in_flight_and_queued_requests_with_disconnected() {
        let rig = Rig::new();
        rig.push(vec![Step::Expect(wire(b"a"))]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(60);
        let mut running = rig.guard.request(frame(b"a"), deadline, None);
        let mut queued = rig.guard.request(frame(b"b"), deadline, None);
        assert!(poll_once(&mut running).is_pending(), "a is in flight");
        assert!(poll_once(&mut queued).is_pending(), "b is queued");

        let mut closing = rig.guard.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "close waits for the slot"
        );
        let queued_error = error_of(ready(poll_once(&mut queued)));
        let running_error = error_of(ready(poll_once(&mut running)));
        for error in [&queued_error, &running_error] {
            assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
            assert_eq!(
                error.diagnostics().connection_state,
                Some(ConnectionState::Closing),
                "state at the failure"
            );
        }
        ready(poll_once(&mut closing)).expect("close");
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                ConnectionState::Closing,
                ConnectionState::Closed,
            ],
            "state sequence"
        );
        assert!(rig.shut_down(0), "the transport was shut down");
        assert_eq!(rig.sent(0), vec![wire(b"a")], "b never reached the wire");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[test]
    fn close_with_a_peer_that_never_reads_completes_by_the_request_timeout_and_ends_closed() {
        let rig = Rig::new();
        rig.push(vec![Step::Expect(wire(b"a"))]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(60);
        let mut stuck = rig.guard.request(frame(b"a"), deadline, None);
        assert!(
            poll_once(&mut stuck).is_pending(),
            "a holds the slot and is never polled again"
        );

        let started = rig.clock.now();
        let error = error_of(block_on_with(&rig.clock, rig.guard.close()));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the slot was not freed in time"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Close),
            "operation"
        );
        assert_eq!(
            rig.clock.now() - started,
            secs(3),
            "bounded by timeouts.request, to the instant"
        );
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Closed,
            "closed regardless"
        );
        assert!(!rig.shut_down(0), "the holder still owns the transport");
        drop(stuck);
        block_on_with(&rig.clock, rig.guard.close()).expect("a second close is a no-op");
    }

    #[test]
    fn a_request_before_connect_is_refused_without_io_and_so_is_one_during_close() {
        let rig = Rig::new();
        let before = error_of(rig.request(b"a", 1));
        assert_eq!(before.kind(), ErrorKind::Disconnected, "never connected");
        assert_eq!(
            before.diagnostics().connection_state,
            Some(ConnectionState::Disconnected),
            "state at the failure"
        );
        assert!(rig.factory.opens().is_empty(), "nothing was opened");

        rig.push(vec![Step::Expect(wire(b"a"))]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(60);
        let mut stuck = rig.guard.request(frame(b"a"), deadline, None);
        assert!(poll_once(&mut stuck).is_pending(), "a holds the slot");
        let mut closing = rig.guard.close();
        assert!(
            poll_once(&mut closing).is_pending(),
            "close waits for the slot"
        );
        let during = error_of(rig.request(b"b", 1));
        assert_eq!(during.kind(), ErrorKind::Disconnected, "closing");
        assert_eq!(
            during.diagnostics().connection_state,
            Some(ConnectionState::Closing),
            "state at the failure"
        );
        assert_eq!(rig.sent(0), vec![wire(b"a")], "b never reached the wire");
        drop(stuck);
        ready(poll_once(&mut closing)).expect("close");
    }

    #[test]
    fn a_shutdown_that_the_transport_refuses_is_reported_and_the_guard_is_closed_anyway() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let mut config = guard_config(2, policy(250, 30_000, None));
        // A zero request timeout leaves the shutdown with an elapsed deadline.
        config.timeouts.request = Duration::ZERO;
        let guard: Guard = ConnectionGuard::new(
            config,
            Arc::new(factory),
            Box::new(|| LengthPrefixedFramer::new(64)),
            Arc::new(clock.clone()),
        );
        block_on_with(&clock, guard.connect()).expect("connect");
        let error = error_of(block_on_with(&clock, guard.close()));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the transport's own refusal"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Close),
            "operation"
        );
        assert_eq!(guard.state(), ConnectionState::Closed, "closed anyway");
    }

    #[test]
    fn a_dropped_request_leaves_the_slot_free_and_the_phase_in_flight() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::Expect(wire(b"b")),
            Step::Reply(wire(b"B")),
        ]);
        rig.connect().expect("connect");
        assert!(
            format!("{:?}", rig.guard).contains("Idle"),
            "idle after connect"
        );
        let deadline = rig.deadline(5);
        let mut abandoned = rig.guard.request(frame(b"a"), deadline, None);
        assert!(poll_once(&mut abandoned).is_pending(), "awaiting");
        assert!(
            format!("{:?}", rig.guard).contains("slot in use"),
            "slot held"
        );
        drop(abandoned);
        let shown = format!("{:?}", rig.guard);
        assert!(
            shown.contains("InFlight"),
            "the abandoned exchange is remembered: {shown}"
        );
        assert!(shown.contains("fake://plc"), "endpoint shown: {shown}");
        assert_eq!(
            rig.request(b"b", 5).expect("the slot was released"),
            frame(b"B")
        );
        assert!(format!("{:?}", rig.guard).contains("Idle"), "idle again");
    }

    #[test]
    fn a_frame_over_the_limit_is_refused_and_the_guard_stays_usable() {
        let rig = Rig::connected();
        let error = error_of(rig.request(&[7_u8; 100], 1));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "over the framer maximum"
        );
        assert!(rig.sent(0).is_empty(), "nothing was sent");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "state unchanged"
        );
        assert_eq!(
            rig.request(b"ping", 1).expect("still usable"),
            frame(b"pong")
        );
    }

    #[test]
    fn the_default_deadline_is_timeouts_request_on_the_guards_clock() {
        let rig = Rig::new();
        rig.push(vec![Step::Expect(wire(b"x"))]);
        rig.connect().expect("connect");
        let started = rig.clock.now();
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard.request_with_default_deadline(frame(b"x"), None),
        ));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(rig.clock.now() - started, secs(3), "timeouts.request");
        assert_eq!(error.diagnostics().latency, Some(secs(3)), "latency");
    }

    #[test]
    fn a_request_deadline_shorter_than_the_backoff_times_out_asleep() {
        let rig = Rig::dropped_after(b"a");
        error_of(rig.request(b"a", 10));
        let error = error_of(rig.request(b"b", 0));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "an elapsed deadline is refused at once"
        );
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard
                .request(frame(b"b"), Deadline::at(rig.at(100)), None),
        ));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the deadline came before next_at"
        );
        assert_eq!(rig.clock.now(), rig.at(100), "woken by the deadline");
        assert_eq!(
            rig.guard.state(),
            reconnecting(1, rig.at(250)),
            "the schedule is kept"
        );
    }

    #[test]
    fn the_guard_config_comes_from_a_connection_config_without_its_credentials() {
        let mut reconnect = ReconnectPolicy::default();
        reconnect.initial = ms(100);
        let source = ConnectionConfig::builder("fake", Endpoint::Custom("fake://plc".to_owned()))
            .reconnect(reconnect)
            .credentials(stonecrab::Credentials::new("operator", "hunter2"))
            .build()
            .expect("valid configuration");
        let config = GuardConfig::from(&source);
        assert_eq!(config.endpoint, *source.endpoint(), "endpoint");
        assert_eq!(config.timeouts, *source.timeouts(), "timeouts");
        assert_eq!(config.reconnect, reconnect, "reconnect");
        assert_eq!(config.limits, *source.limits(), "limits");
        assert!(!format!("{config:?}").contains("hunter2"), "no credentials");
    }

    #[test]
    fn timeouts_the_clock_cannot_represent_are_invalid_config() {
        let clock = FakeClock::default();
        let build = |connect: Duration, request: Duration| -> Guard {
            let mut config = guard_config(2, policy(250, 30_000, None));
            config.timeouts.connect = connect;
            config.timeouts.request = request;
            ConnectionGuard::new(
                config,
                Arc::new(FakeTransportFactory::new(clock.clone())),
                Box::new(|| LengthPrefixedFramer::new(64)),
                Arc::new(clock.clone()),
            )
        };

        let guard = build(Duration::MAX, secs(3));
        let error = error_of(block_on_with(&clock, guard.connect()));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "connect timeout");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );
        assert_eq!(
            guard.state(),
            ConnectionState::Failed(ErrorKind::InvalidConfig),
            "no retry can fix it"
        );

        let guard = build(secs(2), Duration::MAX);
        let error = error_of(block_on_with(
            &clock,
            guard.request_with_default_deadline(frame(b"x"), None),
        ));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "request timeout");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "category"
        );
        // `close` falls back to an immediate bound rather than failing.
        block_on_with(&clock, guard.close()).expect("closing a guard that never connected");
        assert_eq!(guard.state(), ConnectionState::Closed, "closed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_requests_on_real_wakers_all_get_their_own_reply() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        // An echo peer: every frame it reads comes straight back.
        factory.push_script(Script::new(vec![Step::respond(|request| {
            Some(request.to_vec())
        })]));
        let guard: Arc<Guard> = Arc::new(ConnectionGuard::new(
            guard_config(8, policy(250, 30_000, None)),
            Arc::new(factory.clone()),
            Box::new(|| LengthPrefixedFramer::new(64)),
            Arc::new(clock.clone()),
        ));
        guard.connect().await.expect("connect");
        let deadline = Deadline::after(clock.now(), secs(5)).expect("representable");
        let mut tasks = tokio::task::JoinSet::new();
        for index in 0_u8..8 {
            let guard = Arc::clone(&guard);
            tasks.spawn(async move { guard.request(vec![index], deadline, None).await });
        }
        let mut replies = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            replies.push(joined.expect("task").expect("reply"));
        }
        replies.sort();
        let expected: Vec<Vec<u8>> = (0_u8..8).map(|index| vec![index]).collect();
        assert_eq!(replies, expected, "each caller got its own frame back");
        let probe = factory.probes().remove(0);
        assert_eq!(probe.sent().len(), 8, "eight frames, one at a time");
        assert_eq!(guard.stats(), stats(0, 0, 8, 0), "stats");
        guard.close().await.expect("close");
    }

    #[test]
    fn the_default_jitter_source_stays_inside_the_policy_bounds() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_refusal();
        let guard: Guard = ConnectionGuard::new(
            guard_config(2, policy(250, 30_000, None)),
            Arc::new(factory),
            Box::new(|| LengthPrefixedFramer::new(64)),
            Arc::new(clock.clone()),
        );
        let t0 = clock.now();
        error_of(block_on_with(&clock, guard.connect()));
        let ConnectionState::Reconnecting { attempt, next_at } = guard.state() else {
            panic!("expected a pending reconnect");
        };
        assert_eq!(attempt, 1, "first retry");
        let delay = next_at - t0;
        assert!(
            delay >= ms(200) && delay <= ms(300),
            "250 ms +/- 20%: {delay:?}"
        );
    }

    // The supervisor (`ConnectionGuard::supervise`, ADR-0014 addendum S57). These tests run
    // the guard on a current-thread Tokio runtime with the fake clock: the test moves the
    // clock, then yields until the task has acted on it. Nothing waits on real time.

    /// Yield to the supervisor until `done` holds; fail the test after 1000 yields.
    async fn settle(done: impl Fn() -> bool, what: &str) {
        for _ in 0..1000 {
            if done() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the supervisor never got there: {what}");
    }

    impl Rig {
        /// Connect and start the supervisor.
        async fn supervised(&self) {
            self.guard.connect().await.expect("connect");
            assert!(self.guard.supervise(), "a runtime is current");
        }

        /// Yield until the guard is in `state`.
        async fn reaches(&self, state: ConnectionState) {
            settle(|| self.guard.state() == state, &format!("{state:?}")).await;
        }

        /// A `ping` whose deadline, 1 s, passes after the frame went out: the stream is
        /// intact and owes a reply.
        async fn times_out_at_one_second(&self) {
            let deadline = self.deadline(1);
            let asked = self.guard.request(frame(b"ping"), deadline, None);
            let moved = async {
                settle(|| !self.sent(0).is_empty(), "the frame went out").await;
                assert!(self.clock.set(self.at(1_000)), "forward to the deadline");
            };
            let (reply, ()) = tokio::join!(asked, moved);
            assert_eq!(error_of(reply).kind(), ErrorKind::Timeout, "timeout");
        }
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_supervised_guard_notices_a_drop_and_reconnects_without_a_request() {
        let rig = Rig::new();
        rig.push(vec![Step::Delay(secs(10)), Step::CloseAt(0)]);
        rig.push(vec![
            Step::Expect(wire(b"ping")),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        assert!(rig.guard.supervise(), "a second call starts no second task");

        // The peer closes at 10 s while nothing is asked of it.
        assert!(rig.clock.set(rig.at(10_000)), "forward");
        rig.reaches(reconnecting(1, rig.at(10_250))).await;
        assert!(rig.clock.set(rig.at(10_250)), "forward");
        rig.reaches(ConnectionState::Connected).await;
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(10_250)),
                ConnectionState::Connecting,
                ConnectionState::Connected,
            ],
            "every state of the drop and the reconnect, published through the watch"
        );
        assert_eq!(rig.factory.opens().len(), 2, "reopened once");
        assert!(rig.sent(0).is_empty(), "no request ever ran");
        assert_eq!(rig.guard.stats(), stats(1, 0, 0, 0), "stats");

        // A request takes the slot from the reader and runs on the new transport.
        let deadline = rig.deadline(5);
        let reply = rig.guard.request(frame(b"ping"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"pong", "reply");
        rig.guard.close().await.expect("close");
        assert_eq!(rig.guard.state(), ConnectionState::Closed, "closed");
        assert_eq!(rig.clock.sleeper_count(), 0, "the reader is gone");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (exhausted budget)
    #[tokio::test]
    async fn background_attempts_follow_the_backoff_and_end_failed_when_the_budget_is_spent() {
        let rig = Rig::with(4, policy(250, 1000, Some(3)), 0.5);
        rig.push(vec![Step::CloseAt(0)]);
        for _ in 0..3 {
            rig.factory.push_refusal();
        }
        rig.supervised().await;

        for (attempt, at) in [(1, 250), (2, 750), (3, 1750)] {
            rig.reaches(reconnecting(attempt, rig.at(at))).await;
            assert!(rig.clock.set(rig.at(at)), "forward to the attempt");
        }
        rig.reaches(ConnectionState::Failed(ErrorKind::Disconnected))
            .await;
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                reconnecting(2, rig.at(750)),
                ConnectionState::Connecting,
                reconnecting(3, rig.at(1750)),
                ConnectionState::Connecting,
                ConnectionState::Failed(ErrorKind::Disconnected),
            ],
            "delays 250, 500 and 1000 ms, then the budget of three retries is spent"
        );
        let opened_at: Vec<_> = rig
            .factory
            .opens()
            .iter()
            .map(|open| open.deadline.instant())
            .collect();
        assert_eq!(
            opened_at,
            vec![rig.at(2000), rig.at(2250), rig.at(2750), rig.at(3750)],
            "each open began at its next_at, bounded by timeouts.connect"
        );
        settle(|| rig.clock.sleeper_count() == 0, "the task has ended").await;
        rig.guard
            .close()
            .await
            .expect("closing a failed guard is a no-op");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "terminal"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (outage and recovery)
    #[tokio::test]
    async fn a_request_during_a_background_reconnect_waits_within_its_deadline() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        rig.push(vec![
            Step::Expect(wire(b"ping")),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        rig.reaches(reconnecting(1, rig.at(250))).await;

        let deadline = rig.deadline(5);
        let (reply, ()) = tokio::join!(rig.guard.request(frame(b"ping"), deadline, None), async {
            // The supervisor's backoff, the request's backoff and the request's deadline.
            settle(|| rig.clock.sleeper_count() == 3, "both wait").await;
            assert!(rig.clock.set(rig.at(250)), "forward");
        });
        assert_eq!(
            reply.expect("reply"),
            b"pong",
            "the request waited, then ran"
        );
        assert_eq!(rig.factory.opens().len(), 2, "one reopen between them");
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                ConnectionState::Connected,
            ],
            "state sequence"
        );
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (outage and recovery)
    #[tokio::test]
    async fn a_request_whose_deadline_ends_in_the_backoff_times_out_and_the_reconnect_goes_on() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        rig.push(Vec::new());
        rig.supervised().await;
        rig.reaches(reconnecting(1, rig.at(250))).await;

        let deadline = Deadline::after(rig.clock.now(), ms(100)).expect("representable");
        let (result, ()) = tokio::join!(rig.guard.request(frame(b"x"), deadline, None), async {
            settle(|| rig.clock.sleeper_count() == 3, "both wait").await;
            assert!(rig.clock.set(rig.at(100)), "forward to the deadline");
        });
        let error = error_of(result);
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the request's own deadline"
        );
        assert_eq!(error.diagnostics().latency, Some(ms(100)), "latency");
        assert_eq!(rig.factory.opens().len(), 1, "nothing was opened for it");

        assert!(rig.clock.set(rig.at(250)), "forward");
        rig.reaches(ConnectionState::Connected).await;
        assert_eq!(rig.factory.opens().len(), 2, "the supervisor reopened");
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[tokio::test]
    async fn close_during_the_background_backoff_stops_and_joins_the_supervisor() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        rig.supervised().await;
        rig.reaches(reconnecting(1, rig.at(250))).await;
        settle(|| rig.clock.sleeper_count() == 1, "asleep in the backoff").await;

        rig.guard.close().await.expect("close");
        assert_eq!(rig.clock.now(), rig.t0, "close took no time on the clock");
        assert_eq!(
            rig.clock.sleeper_count(),
            0,
            "the backoff sleep is gone: the task was joined"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Closing,
                ConnectionState::Closed,
            ],
            "state sequence"
        );
        assert!(rig.clock.set(rig.at(10_000)), "forward past the backoff");
        assert_eq!(rig.factory.opens().len(), 1, "nothing was reopened");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[tokio::test]
    async fn close_during_a_background_open_cuts_it_and_joins_the_supervisor() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        rig.factory.push_connect_timeout();
        rig.supervised().await;
        rig.reaches(reconnecting(1, rig.at(250))).await;
        assert!(rig.clock.set(rig.at(250)), "forward");
        rig.reaches(ConnectionState::Connecting).await;

        rig.guard.close().await.expect("close");
        assert_eq!(
            rig.clock.now(),
            rig.at(250),
            "close took no time on the clock"
        );
        assert_eq!(
            rig.clock.sleeper_count(),
            0,
            "the parked open is gone: the task was joined"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                reconnecting(1, rig.at(250)),
                ConnectionState::Connecting,
                ConnectionState::Closing,
                ConnectionState::Closed,
            ],
            "state sequence"
        );
        assert_eq!(rig.factory.opens().len(), 2, "one open, abandoned");
    }

    /// The default rig's configuration with an idle bound of 5 s.
    fn idle_config() -> GuardConfig {
        let mut config = guard_config(4, policy(250, 30_000, None));
        config.timeouts.idle = Some(secs(5));
        config
    }

    #[tokio::test]
    async fn an_idle_link_is_closed_after_timeouts_idle_and_the_next_request_reopens_it() {
        let rig = Rig::with_config(idle_config(), 0.5);
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::Reply(wire(b"a")),
            Step::Expect(wire(b"b")),
            Step::Reply(wire(b"b")),
        ]);
        rig.push(vec![Step::Expect(wire(b"c")), Step::Reply(wire(b"c"))]);
        rig.supervised().await;
        let ask = |payload: &'static [u8]| {
            let deadline = rig.deadline(5);
            rig.guard.request(frame(payload), deadline, None)
        };

        ask(b"a").await.expect("a");
        assert!(rig.clock.set(rig.at(4_000)), "forward");
        // An exchange at 4 s moves the bound to 9 s.
        ask(b"b").await.expect("b");
        settle(|| rig.clock.sleeper_count() == 1, "reading until 9 s").await;
        assert!(rig.clock.set(rig.at(8_999)), "forward");
        tokio::task::yield_now().await;
        assert!(!rig.shut_down(0), "still in use within the bound");
        assert!(rig.clock.set(rig.at(9_000)), "forward");
        settle(|| rig.shut_down(0), "the idle link is shut down").await;
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "the connection stays available"
        );

        assert!(rig.clock.set(rig.at(20_000)), "forward");
        assert_eq!(ask(b"c").await.expect("c"), b"c", "reply");
        let reopen = rig.factory.opens().remove(1);
        assert_eq!(
            reopen.deadline.instant(),
            rig.at(22_000),
            "reopened at once, bounded by timeouts.connect"
        );
        assert_eq!(
            rig.states(),
            vec![
                ConnectionState::Connecting,
                ConnectionState::Connected,
                ConnectionState::Connecting,
                ConnectionState::Connected,
            ],
            "the idle close sets no state; the reopen does"
        );
        assert_eq!(rig.guard.stats(), stats(1, 0, 3, 0), "stats");
        rig.guard.close().await.expect("close");
        assert!(rig.shut_down(1), "the new transport was shut down by close");
    }

    #[test]
    fn without_a_supervisor_the_idle_bound_is_not_enforced() {
        let rig = Rig::with_config(idle_config(), 0.5);
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::Reply(wire(b"a")),
            Step::Expect(wire(b"b")),
            Step::Reply(wire(b"b")),
        ]);
        rig.connect().expect("connect");
        assert!(!rig.guard.supervise(), "no Tokio runtime is current");
        rig.request(b"a", 1).expect("a");
        rig.clock.advance(secs(60));
        rig.request(b"b", 1).expect("b on the same transport");
        assert_eq!(rig.factory.opens().len(), 1, "never reopened");
        assert!(!rig.shut_down(0), "never closed for idleness");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[tokio::test]
    async fn a_frame_nobody_asked_for_does_not_answer_the_next_request() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Delay(secs(1)),
            Step::Reply(wire(b"junk")),
            Step::Expect(wire(b"ping")),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        assert!(rig.clock.set(rig.at(1_000)), "the junk arrives");
        // The reader holds the slot throughout: yield until it has read and kept the frame.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }

        let deadline = rig.deadline(5);
        let reply = rig.guard.request(frame(b"ping"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"pong", "the junk did not answer it");
        assert_eq!(rig.guard.stats(), stats(0, 0, 1, 0), "no resync was needed");
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[tokio::test]
    async fn bytes_the_framer_refuses_while_idle_are_realigned_by_the_reader() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Delay(secs(1)),
            // An oversized header, then a whole frame the resync locks onto.
            Step::Reply(vec![0xFF, 0xFF, 0, 1, b'x']),
            Step::Expect(wire(b"ping")),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        assert!(rig.clock.set(rig.at(1_000)), "the bytes arrive");
        // The reader holds the slot throughout, so there is no state to wait on: yield until
        // it has taken the bytes (it then waits for the frame boundary they lack).
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }

        let deadline = rig.deadline(5);
        let reply = rig.guard.request(frame(b"ping"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"pong", "reply");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 0),
            "the reader found the boundary itself: the request had no resync to do"
        );
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (shutdown during reconnect)
    #[tokio::test]
    async fn a_zero_request_timeout_leaves_the_supervisor_waiting_and_close_aborts_it() {
        let mut config = guard_config(4, policy(250, 30_000, None));
        config.timeouts.request = Duration::ZERO;
        let rig = Rig::with_config(config, 0.5);
        rig.push(Vec::new());
        rig.supervised().await;
        settle(
            || format!("{:?}", rig.guard).contains("Idle"),
            "the supervisor let go of the slot",
        )
        .await;
        assert_eq!(rig.clock.sleeper_count(), 0, "there is nothing to read for");
        // The close bound is `timeouts.request` from now, already over before the task
        // could run: it is aborted and the call says so.
        let error = error_of(rig.guard.close().await);
        assert_eq!(error.kind(), ErrorKind::Timeout, "the join was cut off");
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Closed,
            "closed regardless"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_drop_after_a_timed_out_request_is_noticed_without_another_request() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"ping")),
            Step::Delay(secs(10)),
            Step::CloseAt(0),
        ]);
        rig.supervised().await;

        // The request times out at 1 s: the stream is intact but owes a reply.
        let deadline = rig.deadline(1);
        let asked = rig.guard.request(frame(b"ping"), deadline, None);
        let moved = async {
            settle(|| !rig.sent(0).is_empty(), "the frame went out").await;
            assert!(rig.clock.set(rig.at(1_000)), "forward to the deadline");
        };
        let (reply, ()) = tokio::join!(asked, moved);
        assert_eq!(error_of(reply).kind(), ErrorKind::Timeout, "timeout");
        assert_eq!(rig.guard.state(), ConnectionState::Connected, "still up");

        // The peer closes at 10 s while nothing is asked of it.
        assert!(rig.clock.set(rig.at(10_000)), "forward");
        rig.reaches(reconnecting(1, rig.at(10_250))).await;
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_reconnected_link_that_stays_up_for_a_read_starts_the_budget_over() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        rig.push(vec![Step::Delay(secs(5)), Step::CloseAt(0)]);
        rig.push(Vec::new());
        rig.supervised().await;
        rig.reaches(reconnecting(1, rig.at(250))).await;
        assert!(rig.clock.set(rig.at(250)), "forward to the attempt");
        rig.reaches(ConnectionState::Connected).await;

        // The new link lives through a whole read (timeouts.request, 3 s)...
        assert!(rig.clock.set(rig.at(3_250)), "forward past the read");
        for _ in 0..20 {
            tokio::task::yield_now().await;
        }
        // ...so its later drop is a first failure, not a second.
        assert!(rig.clock.set(rig.at(5_250)), "forward to the drop");
        rig.reaches(reconnecting(1, rig.at(5_500))).await;
        rig.guard.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_guard_that_is_closed_starts_no_supervisor() {
        let rig = Rig::new();
        rig.push(Vec::new());
        rig.supervised().await;
        rig.guard.close().await.expect("close");
        assert!(!rig.guard.supervise(), "nothing is left to tend");
    }

    // The request/reader hand-over on real wakers: a request that loses the race for the
    // slot would wait on a reader with no reason to stop, and the fake clock never fires
    // its deadline, so the test would hang instead of passing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn back_to_back_requests_never_wait_behind_the_reader() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::new(vec![Step::respond(|request| {
            Some(request.to_vec())
        })]));
        let guard: Guard = ConnectionGuard::new(
            guard_config(8, policy(250, 30_000, None)),
            Arc::new(factory),
            Box::new(|| LengthPrefixedFramer::new(64)),
            Arc::new(clock.clone()),
        );
        guard.connect().await.expect("connect");
        assert!(guard.supervise(), "a runtime is current");
        let deadline = Deadline::after(clock.now(), secs(5)).expect("representable");
        for index in 0_u8..=255 {
            let reply = guard.request(vec![index], deadline, None).await;
            assert_eq!(reply.expect("reply"), vec![index], "echo");
        }
        guard.close().await.expect("close");
    }

    // A session that reads on after a stale reply is between two guard calls: the reader
    // must not take the real reply that follows.
    #[tokio::test]
    async fn the_reader_keeps_the_reply_a_session_reads_next() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::ReplyCoalescedWithNext,
            Step::Reply(wire(b"stale")),
            Step::Reply(wire(b"real")),
        ]);
        rig.supervised().await;
        let deadline = rig.deadline(5);
        let first = rig.guard.request(frame(b"a"), deadline, None).await;
        assert_eq!(first.expect("first reply"), b"stale");
        // Give the reader every chance to run before the session reads again.
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let second = rig.guard.next_reply(deadline, None).await;
        assert_eq!(
            second.expect("second reply"),
            b"real",
            "kept for the session"
        );
        rig.guard.close().await.expect("close");
    }

    #[tokio::test]
    async fn a_frame_the_reader_kept_is_dropped_by_the_next_request() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::ReplyCoalescedWithNext,
            Step::Reply(wire(b"a")),
            Step::Reply(wire(b"extra")),
            Step::Expect(wire(b"b")),
            Step::Reply(wire(b"b")),
        ]);
        rig.supervised().await;
        let deadline = rig.deadline(5);
        let first = rig.guard.request(frame(b"a"), deadline, None).await;
        assert_eq!(first.expect("first reply"), b"a");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        let second = rig.guard.request(frame(b"b"), deadline, None).await;
        assert_eq!(
            second.expect("second reply"),
            b"b",
            "the extra frame is gone"
        );
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_supervisor_started_before_connect_waits_for_the_connection() {
        let rig = Rig::new();
        rig.push(vec![Step::CloseAt(0)]);
        assert!(rig.guard.supervise(), "a runtime is current");
        // Hand the thread to the task once: with nothing to tend it waits for news.
        tokio::task::yield_now().await;
        assert!(
            rig.factory.opens().is_empty(),
            "the supervisor never connects"
        );
        rig.guard.connect().await.expect("connect");
        rig.reaches(reconnecting(1, rig.at(250))).await;
        rig.guard.close().await.expect("close");
        assert_eq!(rig.clock.sleeper_count(), 0, "joined");
    }

    #[tokio::test]
    async fn a_zero_idle_bound_closes_the_link_after_every_exchange() {
        let mut config = guard_config(4, policy(250, 30_000, None));
        config.timeouts.idle = Some(Duration::ZERO);
        let rig = Rig::with_config(config, 0.5);
        rig.push(Vec::new());
        rig.push(vec![Step::Expect(wire(b"a")), Step::Reply(wire(b"a"))]);
        rig.supervised().await;
        settle(|| rig.shut_down(0), "closed right after the open").await;

        let deadline = rig.deadline(5);
        let reply = rig.guard.request(frame(b"a"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"a", "reopened for the request");
        settle(|| rig.shut_down(1), "closed right after the exchange").await;
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "still available"
        );
        assert_eq!(rig.factory.opens().len(), 2, "one open per use");
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    // The late reply of a timed-out request is dropped by the reader, which reads on: the
    // idle bound is not suspended by it.
    #[tokio::test]
    async fn a_late_reply_the_reader_drops_leaves_the_idle_bound_in_force() {
        let rig = Rig::with_config(idle_config(), 0.5);
        rig.push(vec![
            Step::Expect(wire(b"ping")),
            Step::Delay(secs(2)),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        rig.times_out_at_one_second().await;

        assert!(rig.clock.set(rig.at(2_000)), "the late reply arrives");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        // The timed-out exchange ended at 1 s, so the bound is 6 s.
        assert!(rig.clock.set(rig.at(5_999)), "forward");
        tokio::task::yield_now().await;
        assert!(!rig.shut_down(0), "still within the bound");
        assert!(rig.clock.set(rig.at(6_000)), "forward to the bound");
        settle(|| rig.shut_down(0), "the idle link is shut down").await;
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "still available"
        );
        assert_eq!(rig.guard.stats(), stats(0, 0, 1, 1), "stats");
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_late_reply_the_reader_drops_leaves_a_later_drop_noticed() {
        let rig = Rig::with_config(idle_config(), 0.5);
        rig.push(vec![
            Step::Expect(wire(b"ping")),
            Step::Delay(secs(2)),
            Step::Reply(wire(b"pong")),
            Step::Delay(secs(1)),
            Step::CloseAt(0),
        ]);
        rig.push(Vec::new());
        rig.supervised().await;
        rig.times_out_at_one_second().await;

        assert!(rig.clock.set(rig.at(2_000)), "the late reply arrives");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(rig.clock.set(rig.at(3_000)), "the peer closes");
        rig.reaches(reconnecting(1, rig.at(3_250))).await;
        rig.guard.close().await.expect("close");
    }

    // A frame kept for `next_reply` does not keep an idle link open; it goes with it.
    #[tokio::test]
    async fn a_kept_frame_goes_with_the_idle_close_and_next_reply_is_disconnected() {
        let rig = Rig::with_config(idle_config(), 0.5);
        rig.push(vec![Step::Delay(secs(1)), Step::Reply(wire(b"junk"))]);
        rig.push(vec![Step::Expect(wire(b"c")), Step::Reply(wire(b"c"))]);
        rig.supervised().await;
        assert!(rig.clock.set(rig.at(1_000)), "the junk arrives and is kept");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(rig.clock.set(rig.at(5_000)), "forward to the bound");
        settle(|| rig.shut_down(0), "the idle link is shut down").await;
        assert_eq!(
            rig.guard.state(),
            ConnectionState::Connected,
            "still available"
        );

        let deadline = rig.deadline(5);
        let error = error_of(rig.guard.next_reply(deadline, None).await);
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "the kept frame went with the transport"
        );
        let reply = rig.guard.request(frame(b"c"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"c", "reopened for the request");
        assert_eq!(rig.factory.opens().len(), 2, "one reopen");
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[tokio::test]
    async fn a_peer_close_behind_a_kept_frame_is_noticed() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Delay(secs(1)),
            Step::Reply(wire(b"junk")),
            Step::Delay(secs(1)),
            Step::CloseAt(0),
        ]);
        rig.push(Vec::new());
        rig.supervised().await;
        assert!(rig.clock.set(rig.at(1_000)), "the junk arrives and is kept");
        for _ in 0..50 {
            tokio::task::yield_now().await;
        }
        assert!(rig.clock.set(rig.at(2_000)), "the peer closes");
        rig.reaches(reconnecting(1, rig.at(2_250))).await;
        rig.guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (buffer bound)
    // Behind a kept frame the buffer fills up: the frame is dropped and the reader realigns
    // the stream, as a request would.
    #[tokio::test]
    async fn a_full_buffer_behind_a_kept_frame_is_realigned_by_the_reader() {
        let rig = Rig::new();
        let mut flood = vec![0xFF; 1100];
        flood.extend_from_slice(&wire(b"y"));
        rig.push(vec![
            Step::Delay(secs(1)),
            Step::Reply(wire(b"x")),
            Step::Reply(flood),
            Step::Expect(wire(b"ping")),
            Step::Reply(wire(b"pong")),
        ]);
        rig.supervised().await;
        assert!(
            rig.clock.set(rig.at(1_000)),
            "the frame and the flood arrive"
        );
        for _ in 0..200 {
            tokio::task::yield_now().await;
        }

        let deadline = rig.deadline(5);
        let reply = rig.guard.request(frame(b"ping"), deadline, None).await;
        assert_eq!(reply.expect("reply"), b"pong", "reply");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 0),
            "the reader realigned the stream itself"
        );
        assert_eq!(rig.factory.opens().len(), 1, "the link was kept");
        rig.guard.close().await.expect("close");
    }

    // A session that correlates replies (a mismatched one, then its own) on a link the
    // supervisor closes after every exchange: the slot is held from the send to the
    // accepted reply, so the idle close never comes between two reads. On real threads the
    // supervisor runs in parallel; with request plus next_reply most rounds lost the reply.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_correlated_exchange_keeps_its_reply_under_a_zero_idle_bound() {
        const ROUNDS: usize = 300;
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        // Every peer answers every frame with a stale frame and the real one in one write.
        // Whether the link is closed between two rounds is the scheduler's choice, so each
        // peer serves any number of rounds.
        for _ in 0..=ROUNDS {
            factory.push_script(Script::new(vec![Step::respond(|_| {
                let mut bytes = wire(b"stale");
                bytes.extend_from_slice(&wire(b"real"));
                Some(bytes)
            })]));
        }
        let mut config = guard_config(4, policy(250, 30_000, None));
        config.timeouts.idle = Some(Duration::ZERO);
        let guard: Guard = ConnectionGuard::new(
            config,
            Arc::new(factory),
            Box::new(|| LengthPrefixedFramer::new(64)),
            Arc::new(clock.clone()),
        );
        guard.connect().await.expect("connect");
        assert!(guard.supervise(), "a runtime is current");
        let deadline = Deadline::after(clock.now(), secs(5)).expect("representable");
        for round in 0..ROUNDS {
            let mut refused = 0_u32;
            let mut real = |reply: &Vec<u8>| {
                let wanted = reply.as_slice() == b"real";
                refused += u32::from(!wanted);
                wanted
            };
            let reply = guard
                .request_matching(frame(b"a"), deadline, None, &mut real)
                .await;
            assert_eq!(reply.expect("reply"), b"real", "round {round}");
            assert_eq!(refused, 1, "round {round}: the stale frame was refused");
            tokio::task::yield_now().await;
        }
        guard.close().await.expect("close");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_correlated_exchange_drops_refused_frames_and_times_out_within_its_deadline() {
        let rig = Rig::new();
        rig.push(vec![
            Step::Expect(wire(b"a")),
            Step::Reply(wire(b"other")),
            Step::Delay(secs(5)),
            Step::Reply(wire(b"a")),
        ]);
        rig.connect().expect("connect");
        let deadline = rig.deadline(1);
        let mut refused = Vec::new();
        let mut own = |reply: &Vec<u8>| {
            let wanted = reply.as_slice() == b"a";
            if !wanted {
                refused.push(reply.clone());
            }
            wanted
        };
        let error = error_of(block_on_with(
            &rig.clock,
            rig.guard
                .request_matching(frame(b"a"), deadline, None, &mut own),
        ));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "one deadline for every read"
        );
        assert_eq!(rig.clock.now(), rig.at(1_000), "at the deadline");
        assert_eq!(refused, vec![b"other".to_vec()], "the refused frame");
        assert_eq!(
            rig.guard.stats(),
            stats(0, 0, 1, 1),
            "one request, one timeout"
        );
        assert_eq!(rig.guard.state(), ConnectionState::Connected, "still up");
    }

    // A supervisor that ended with the runtime it ran on does not count as running: the
    // next call starts one on the runtime current then.
    // covers: SPEC mandatory-failure-cases / F5 reconnect (device restart)
    #[test]
    fn a_supervisor_that_ended_with_its_runtime_is_started_again() {
        let rig = Rig::new();
        rig.push(vec![Step::Delay(secs(10)), Step::CloseAt(0)]);
        rig.push(Vec::new());
        let first = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        first.block_on(async {
            rig.supervised().await;
            tokio::task::yield_now().await;
        });
        drop(first);

        let second = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        second.block_on(async {
            assert!(rig.guard.supervise(), "a new task on this runtime");
            assert!(rig.guard.supervise(), "and only one");
            assert!(rig.clock.set(rig.at(10_000)), "the peer goes away");
            rig.reaches(reconnecting(1, rig.at(10_250))).await;
            rig.guard.close().await.expect("close");
        });
        assert_eq!(rig.clock.sleeper_count(), 0, "joined");
    }
}
