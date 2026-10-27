//! Lifecycle of `ModbusConnection` across a lost peer: request-driven reconnect, the fate of
//! a request in flight, transaction ids, and shutdown during the backoff.
//!
//! Peers are `FakeTransportFactory` plans: each `open` hands out the next script, and a
//! script that ends in `CloseAt` is a peer that closes the connection once it has read the
//! request. All timing is on the fake clock (decision D21); the backoff has no jitter, so
//! `next_at` is exact.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::{Duration, Instant};

    use stonecrab_core::{
        AddressOptions, Clock, Connection, ConnectionConfig, ConnectionState, Deadline, Endpoint,
        ErrorKind, Operation, PlcError, PlcValue, ReadRequest, ReadResponse, ReconnectPolicy,
        Result, RetryPolicy, Tag, TagId, Timeouts,
    };
    use stonecrab_modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    use stonecrab_testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
    };

    fn millis(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

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

    /// The read of holding register 1.
    fn request(txn: u16) -> Vec<u8> {
        adu(txn, &[0x03, 0x00, 0x00, 0x00, 0x01])
    }

    /// The reply to it carrying `value`.
    fn reply(txn: u16, value: u16) -> Vec<u8> {
        let [high, low] = value.to_be_bytes();
        adu(txn, &[0x03, 0x02, high, low])
    }

    fn tag() -> Tag {
        let address = ModbusAddress::parse("holding-register:1", &AddressOptions::default())
            .expect("address");
        Tag::new(TagId::new("a").expect("tag id"), Arc::new(address))
    }

    fn txn_of(frame: &[u8]) -> u16 {
        u16::from_be_bytes(
            frame
                .get(..2)
                .expect("a header")
                .try_into()
                .expect("two bytes"),
        )
    }

    /// A peer that answers one read, then a second read, with the given values.
    fn answers(first: u16, value: u16) -> Vec<Step> {
        vec![
            Step::Expect(request(first)),
            Step::Reply(reply(first, value)),
        ]
    }

    /// A peer that reads one request and closes the connection without answering.
    fn closes_after(txn: u16) -> Vec<Step> {
        vec![Step::Expect(request(txn)), Step::CloseAt(0)]
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        /// Open against the `scripts`, one per transport, with a 100 ms to 1 s backoff
        /// without jitter, `reconnect_attempts` retries (unlimited for `None`) and a read
        /// retry policy of `max_attempts` for `Disconnected`.
        fn open(
            scripts: Vec<Vec<Step>>,
            max_attempts: u32,
            reconnect_attempts: Option<u32>,
        ) -> Self {
            let mut reconnect = ReconnectPolicy::default();
            reconnect.initial = millis(100);
            reconnect.max = secs(1);
            reconnect.jitter = 0.0;
            reconnect.max_attempts = reconnect_attempts.and_then(NonZeroU32::new);
            let mut retry = RetryPolicy::default();
            retry.max_attempts = NonZeroU32::new(max_attempts).expect("positive");
            retry.retry_on = std::iter::once(ErrorKind::Disconnected).collect();
            let mut timeouts = Timeouts::default();
            timeouts.connect = secs(2);
            timeouts.request = secs(5);
            let config =
                ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                    .timeouts(timeouts)
                    .reconnect(reconnect)
                    .retry(retry)
                    .build()
                    .expect("config");
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

        fn request(&self, deadline: Option<Deadline>) -> ReadRequest {
            let mut builder = ReadRequest::builder(self.connection.limits()).tag(tag());
            if let Some(deadline) = deadline {
                builder = builder.deadline(deadline);
            }
            builder.build().expect("request")
        }

        fn read(&self) -> Result<ReadResponse> {
            block_on_with(&self.clock, self.connection.read(self.request(None)))
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

    fn value_of(response: &ReadResponse) -> PlcValue {
        response
            .items()
            .first()
            .expect("an item")
            .outcome()
            .as_ref()
            .expect("a value")
            .clone()
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    // covers: SPEC mandatory-failure-cases / device restart, network outage, recovery, explicit shutdown during reconnect
    // The peer closes while this read is in flight; the retry then rides the backoff.
    #[test]
    fn read_after_peer_close_observes_reconnecting_then_connected_and_succeeds() {
        // The read may be retried once, so one call rides out the outage: the first
        // transport closes after reading the request, the second answers.
        let rig = Rig::open(vec![closes_after(1), answers(2, 4242)], 2, None);
        let watch = rig.connection.watch_state();
        let mut seen = vec![watch.current()];
        let mut read = rig.connection.read(rig.request(None));
        let response = loop {
            match poll_once(&mut read) {
                Poll::Ready(outcome) => break outcome.expect("the read succeeds"),
                Poll::Pending => {
                    seen.push(watch.current());
                    assert!(
                        rig.clock.advance_to_next_sleeper().is_some(),
                        "a pending read waits on the backoff"
                    );
                }
            }
        };
        seen.push(watch.current());

        let [connected, reconnecting, again] = seen.as_slice() else {
            panic!("expected three observations, saw {seen:?}");
        };
        assert_eq!(*connected, ConnectionState::Connected, "before the outage");
        assert_eq!(
            *reconnecting,
            ConnectionState::Reconnecting {
                attempt: 1,
                next_at: rig.t0 + millis(100)
            },
            "waiting out the 100 ms backoff"
        );
        assert_eq!(*again, ConnectionState::Connected, "after the reopen");
        assert_eq!(
            value_of(&response),
            PlcValue::I16(4242),
            "the second transport's reply"
        );
        assert_eq!(
            rig.sent(0),
            vec![request(1)],
            "the first transport saw the first request"
        );
        assert_eq!(
            rig.sent(1),
            vec![request(2)],
            "the second saw the retry, with the next id"
        );
        assert_eq!(rig.factory.opens().len(), 2, "one reopen");
    }

    // covers: SPEC mandatory-failure-cases / device restart, network outage, recovery
    #[test]
    fn pending_read_fails_disconnected_and_is_not_replayed_after_reconnect() {
        let rig = Rig::open(vec![closes_after(1), answers(2, 7)], 1, None);
        let error = error_of(rig.read());
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "the request in flight is lost"
        );
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(diagnostics.retries, 0, "the default policy made no retry");
        assert!(
            matches!(
                diagnostics.connection_state,
                Some(ConnectionState::Reconnecting { attempt: 1, .. })
            ),
            "state at the failure: {:?}",
            diagnostics.connection_state
        );
        assert!(
            error.to_string().starts_with("chunk 1 of 1:"),
            "names the chunk: {error}"
        );
        assert!(
            matches!(rig.connection.state(), ConnectionState::Reconnecting { .. }),
            "request-driven: the connection waits for the next request to reopen"
        );
        assert_eq!(rig.factory.opens().len(), 1, "nothing reopened yet");

        // The next read reopens and sends its own request; the lost one is not replayed.
        let response = rig.read().expect("the next read");
        assert_eq!(value_of(&response), PlcValue::I16(7), "the new reply");
        let replayed = rig.sent(1);
        assert_eq!(
            replayed,
            vec![request(2)],
            "the second transport saw only the new request"
        );
        assert!(
            !replayed.contains(&request(1)),
            "the request that was in flight when the peer closed is never replayed"
        );
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Connected,
            "recovered"
        );
    }

    // covers: SPEC mandatory-failure-cases / device restart, network outage, recovery
    #[test]
    fn transaction_ids_continue_after_reconnect() {
        let rig = Rig::open(
            vec![
                vec![
                    Step::Expect(request(1)),
                    Step::Reply(reply(1, 1)),
                    Step::Expect(request(2)),
                    Step::CloseAt(0),
                ],
                answers(3, 3),
            ],
            1,
            None,
        );
        drop(rig.read().expect("the first read"));
        assert_eq!(
            error_of(rig.read()).kind(),
            ErrorKind::Disconnected,
            "the outage"
        );
        drop(rig.read().expect("the read after the reconnect"));
        let first_on_the_second = rig.sent(1);
        assert_eq!(
            first_on_the_second.first().map(|frame| txn_of(frame)),
            Some(3),
            "the counter belongs to the connection, not the transport: 1, 2, then 3"
        );
    }

    // covers: SPEC mandatory-failure-cases / explicit shutdown during reconnect
    #[test]
    fn close_during_backoff_yields_closed() {
        let rig = Rig::open(vec![closes_after(1)], 1, None);
        assert_eq!(
            error_of(rig.read()).kind(),
            ErrorKind::Disconnected,
            "the outage"
        );
        assert!(
            matches!(rig.connection.state(), ConnectionState::Reconnecting { .. }),
            "backoff"
        );

        // The next read is parked in the backoff when the application closes.
        let mut waiting = rig.connection.read(rig.request(None));
        assert!(
            poll_once(&mut waiting).is_pending(),
            "waiting out the backoff"
        );
        // Both futures are polled, as an executor would: the parked read gives up its
        // place when it sees the close, and the close finishes once it has.
        let mut closing = rig.connection.close();
        let (mut parked, mut closed) = (None, false);
        for _ in 0..16 {
            if parked.is_none()
                && let Poll::Ready(outcome) = poll_once(&mut waiting)
            {
                parked = Some(outcome);
            }
            if !closed && let Poll::Ready(outcome) = poll_once(&mut closing) {
                outcome.expect("close");
                closed = true;
            }
        }
        assert!(closed, "the close finished");
        assert_eq!(rig.connection.state(), ConnectionState::Closed, "closed");
        let parked = parked.expect("a read in the backoff ends when the connection closes");
        assert_eq!(
            error_of(parked).kind(),
            ErrorKind::Disconnected,
            "the parked read"
        );
        assert_eq!(
            rig.factory.opens().len(),
            1,
            "the backoff never ended in an open"
        );

        // A later read is the terminal-state error, without I/O.
        let later = error_of(rig.read());
        assert_eq!(later.kind(), ErrorKind::Disconnected, "terminal state");
        assert_eq!(
            later.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert_eq!(rig.sent(0).len(), 1, "nothing more was sent");
        assert_eq!(rig.factory.opens().len(), 1, "and nothing was opened");
        block_on_with(&rig.clock, rig.connection.close()).expect("close is idempotent");
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Closed,
            "still closed"
        );
    }

    // covers: SPEC mandatory-failure-cases / network outage, recovery
    #[test]
    fn reconnect_exhaustion_yields_failed() {
        // One reopen is allowed; the peer refuses it.
        let rig = Rig::open(vec![closes_after(1)], 1, Some(1));
        rig.factory.push_refusal();
        assert_eq!(
            error_of(rig.read()).kind(),
            ErrorKind::Disconnected,
            "the outage"
        );
        let refused = error_of(rig.read());
        assert_eq!(
            refused.kind(),
            ErrorKind::Disconnected,
            "the reopen is refused"
        );
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "the budget is spent"
        );
        let opens = rig.factory.opens().len();

        let terminal = error_of(rig.read());
        assert_eq!(terminal.kind(), ErrorKind::Disconnected, "terminal state");
        assert_eq!(
            rig.factory.opens().len(),
            opens,
            "no further open is attempted"
        );
        block_on_with(&rig.clock, rig.connection.close()).expect("close");
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "close leaves a failed connection as it is"
        );
    }

    // covers: SPEC mandatory-failure-cases / network outage, recovery
    #[test]
    fn a_deadline_shorter_than_the_backoff_is_a_timeout_and_the_connection_keeps_reconnecting() {
        // The read that times out in the backoff still takes transaction id 2.
        let rig = Rig::open(vec![closes_after(1), answers(3, 9)], 1, None);
        assert_eq!(
            error_of(rig.read()).kind(),
            ErrorKind::Disconnected,
            "the outage"
        );
        let deadline = Deadline::after(rig.clock.now(), millis(50)).expect("deadline");
        let error = error_of(block_on_with(
            &rig.clock,
            rig.connection.read(rig.request(Some(deadline))),
        ));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the 100 ms backoff outlasts the deadline"
        );
        assert!(
            matches!(rig.connection.state(), ConnectionState::Reconnecting { .. }),
            "still waiting to reopen"
        );
        assert_eq!(rig.factory.opens().len(), 1, "no reopen yet");
        let response = rig.read().expect("a read with time to spare reconnects");
        assert_eq!(value_of(&response), PlcValue::I16(9), "recovered");
    }

    // covers: SPEC mandatory-failure-cases / network outage (open)
    #[test]
    fn open_fails_when_the_peer_refuses_or_the_connect_times_out() {
        let build = || {
            let mut timeouts = Timeouts::default();
            timeouts.connect = secs(2);
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .timeouts(timeouts)
                .build()
                .expect("config")
        };
        for (plan, kind, waited) in [
            ("refusal", ErrorKind::Disconnected, Duration::ZERO),
            ("connect timeout", ErrorKind::Timeout, secs(2)),
        ] {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            if kind == ErrorKind::Timeout {
                factory.push_connect_timeout();
            } else {
                factory.push_refusal();
            }
            let started = clock.now();
            let error = error_of(block_on_with(
                &clock,
                ModbusConnection::open(
                    build(),
                    Arc::new(factory),
                    Arc::new(clock.clone()),
                    ConnectionDefaults::default(),
                ),
            ));
            assert_eq!(error.kind(), kind, "{plan}: kind");
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Connect),
                "{plan}: operation"
            );
            assert_eq!(clock.now() - started, waited, "{plan}: time spent");
        }
    }
}
