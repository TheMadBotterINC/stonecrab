//! `ModbusConnection` outside the batching and retry tables: one read in flight,
//! cancellation, deadlines, the terminal states, `open` validation and tracing.
//!
//! Wire bytes are written out by hand from the MBAP layout and the V1.1b PDU examples;
//! all timing is on the fake clock (decision D21).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt;
    use std::num::NonZeroU32;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};
    use std::task::Poll;
    use std::thread::{self, ThreadId};
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
    };
    use stonecrab::{
        AddressOptions, CancellationToken, Clock, Connection, ConnectionConfig, ConnectionState,
        Credentials, Deadline, Endpoint, ErrorKind, FailureCategory, Operation, PlcError, PlcValue,
        ReadRequest, ReadResponse, ResourceLimits, Result, RetryPolicy, Tag, TagId, Timeouts,
    };
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

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

    fn tag(id: &str, address: &str) -> Tag {
        let address = ModbusAddress::parse(address, &AddressOptions::default()).expect("address");
        Tag::new(TagId::new(id).expect("tag id"), Arc::new(address))
    }

    fn config(endpoint: Endpoint) -> ConnectionConfig {
        ConnectionConfig::builder("modbus-tcp", endpoint)
            .build()
            .expect("config")
    }

    fn fake_endpoint() -> Endpoint {
        Endpoint::Custom("fake://plc".to_owned())
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        fn open(steps: Vec<Step>) -> Self {
            Self::with(config(fake_endpoint()), steps)
        }

        fn with(config: ConnectionConfig, steps: Vec<Step>) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            factory.push_script(Script::strict(steps));
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

        fn request(
            &self,
            deadline: Option<Deadline>,
            token: Option<&CancellationToken>,
        ) -> ReadRequest {
            let mut builder =
                ReadRequest::builder(self.connection.limits()).tag(tag("a", "holding-register:1"));
            if let Some(deadline) = deadline {
                builder = builder.deadline(deadline);
            }
            if let Some(token) = token {
                builder = builder.cancel(token.clone());
            }
            builder.build().expect("request")
        }

        fn read(&self) -> Result<ReadResponse> {
            block_on_with(&self.clock, self.connection.read(self.request(None, None)))
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .first()
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

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting
    #[test]
    fn a_second_read_while_one_is_in_flight_is_backpressure_before_any_io() {
        let rig = Rig::open(vec![
            Step::Expect(request(1)),
            Step::Expect(request(2)),
            // The reply to the dropped first read arrives only now, then the one to the next.
            Step::Reply(reply(1, 111)),
            Step::Reply(reply(2, 222)),
        ]);
        let mut first = rig.connection.read(rig.request(None, None));
        assert!(
            poll_once(&mut first).is_pending(),
            "the first read waits for its reply"
        );

        let refused = error_of(rig.read());
        assert_eq!(refused.kind(), ErrorKind::Backpressure, "a concurrent read");
        let diagnostics = refused.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert_eq!(
            rig.sent(),
            vec![request(1)],
            "the refused read sent nothing"
        );

        // Dropping the first read frees the slot; the next read discards the stale reply.
        drop(first);
        let response = rig.read().expect("a read after the first was dropped");
        assert_eq!(value_of(&response), PlcValue::I16(222), "its own reply");
        assert_eq!(rig.sent(), vec![request(1), request(2)], "the wire");
        assert_eq!(rig.connection.state(), ConnectionState::Connected, "state");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (transmitting)
    #[test]
    fn cancel_before_flush_is_cancelled_with_nothing_sent() {
        let rig = Rig::open(vec![Step::Expect(request(1)), Step::Reply(reply(1, 5))]);
        let token = CancellationToken::new();
        token.cancel();
        let error = error_of(block_on_with(
            &rig.clock,
            rig.connection.read(rig.request(None, Some(&token))),
        ));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
        let response = rig.read().expect("the next read");
        assert_eq!(
            value_of(&response),
            PlcValue::I16(5),
            "the connection is usable and no transaction id was spent"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting)
    #[test]
    fn cancel_during_a_chunk_is_cancelled_and_the_next_read_succeeds_after_the_guard_resyncs() {
        let whole = reply(1, 5);
        let rest = whole.get(3..).expect("rest").to_vec();
        let prefix = whole.get(..3).expect("prefix").to_vec();
        let rig = Rig::open(vec![
            Step::Expect(request(1)),
            // Three bytes of the reply arrive, the rest three seconds later.
            Step::Reply(prefix),
            Step::Delay(secs(3)),
            Step::Reply(rest),
            Step::Expect(request(2)),
            Step::Reply(reply(2, 6)),
        ]);
        let token = CancellationToken::new();
        let mut first = rig.connection.read(rig.request(None, Some(&token)));
        assert!(
            poll_once(&mut first).is_pending(),
            "awaiting the rest of the reply"
        );
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut first) else {
            panic!("a cancelled read resolves at once");
        };
        let error = error_of(outcome);
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            rig.sent(),
            vec![request(1)],
            "the frame had reached the wire"
        );
        assert_eq!(
            rig.connection.state(),
            ConnectionState::Connected,
            "the stream is intact"
        );
        drop(first);

        let response = rig.read().expect("the next read");
        assert_eq!(
            value_of(&response),
            PlcValue::I16(6),
            "its own reply, after the abandoned one was resynchronized away"
        );
        assert_eq!(rig.sent(), vec![request(1), request(2)], "the wire");
    }

    // covers: SPEC mandatory-failure-cases / F4 deadlines
    #[test]
    fn an_expired_request_deadline_is_timeout_without_io() {
        let rig = Rig::open(vec![Step::Expect(request(1)), Step::Reply(reply(1, 5))]);
        let deadline = Deadline::at(rig.clock.now());
        let error = error_of(block_on_with(
            &rig.clock,
            rig.connection.read(rig.request(Some(deadline), None)),
        ));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
        assert_eq!(rig.clock.now(), rig.t0, "no time passed");
        let response = rig.read().expect("the next read");
        assert_eq!(value_of(&response), PlcValue::I16(5), "and no id was spent");
    }

    // covers: SPEC mandatory-failure-cases / F5 explicit shutdown
    #[test]
    fn a_read_on_a_closed_connection_is_disconnected_without_io() {
        let rig = Rig::open(vec![]);
        block_on_with(&rig.clock, rig.connection.close()).expect("close");
        assert_eq!(rig.connection.state(), ConnectionState::Closed, "closed");
        let error = error_of(rig.read());
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Closed),
            "the state at the failure"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
        assert!(
            rig.factory
                .probes()
                .first()
                .expect("transport")
                .is_shut_down(),
            "close shut the transport down"
        );
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration
    #[test]
    fn a_request_timeout_the_clock_cannot_represent_is_invalid_config_before_any_io() {
        let mut timeouts = Timeouts::default();
        timeouts.request = Duration::MAX;
        let config = ConnectionConfig::builder("modbus-tcp", fake_endpoint())
            .timeouts(timeouts)
            .build()
            .expect("config");
        let rig = Rig::with(config, vec![]);
        let error = error_of(rig.read());
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "kind");
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Validation),
            "category"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration
    #[test]
    fn open_refuses_another_scheme_and_a_frame_limit_below_a_modbus_frame() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let attempt = |config: ConnectionConfig| {
            block_on_with(
                &clock,
                ModbusConnection::open(
                    config,
                    Arc::new(factory.clone()),
                    Arc::new(clock.clone()),
                    ConnectionDefaults::default(),
                ),
            )
        };

        let other = ConnectionConfig::builder("sim", fake_endpoint())
            .build()
            .expect("config");
        let error = error_of(attempt(other));
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "scheme");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "operation"
        );

        let tiny = ResourceLimits::builder()
            .max_frame_bytes(259)
            .max_buffered_bytes(259)
            .build()
            .expect("limits");
        let small = ConnectionConfig::builder("modbus-tcp", fake_endpoint())
            .limits(tiny)
            .build()
            .expect("config");
        assert_eq!(
            error_of(attempt(small)).kind(),
            ErrorKind::InvalidConfig,
            "frame limit"
        );
        assert!(factory.opens().is_empty(), "nothing was opened for either");

        let exact = ResourceLimits::builder()
            .max_frame_bytes(260)
            .max_buffered_bytes(260)
            .build()
            .expect("limits");
        let fits = ConnectionConfig::builder("modbus-tcp", fake_endpoint())
            .limits(exact)
            .build()
            .expect("config");
        let connection = attempt(fits).expect("260 bytes is a whole frame");
        assert_eq!(
            connection.limits(),
            exact,
            "limits() is the configuration's"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    // ---- tracing ----

    /// Debug text without a `{:?}` in the test body.
    struct Shown<'a>(&'a dyn fmt::Debug);

    impl fmt::Display for Shown<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self.0, f)
        }
    }

    /// One recorded event or span: where and when it happened, its name and level, and its
    /// fields rendered as text.
    #[derive(Debug, Clone)]
    struct Recorded {
        seq: u64,
        thread: ThreadId,
        name: String,
        level: Level,
        fields: BTreeMap<String, String>,
    }

    #[derive(Default)]
    struct Collector {
        spans: Mutex<BTreeMap<u64, Recorded>>,
        events: Mutex<Vec<Recorded>>,
        seq: AtomicU64,
    }

    impl Collector {
        fn record(&self, metadata: &Metadata<'_>) -> Recorded {
            Recorded {
                seq: self.seq.fetch_add(1, Ordering::Relaxed),
                thread: thread::current().id(),
                name: metadata.name().to_owned(),
                level: *metadata.level(),
                fields: BTreeMap::new(),
            }
        }
    }

    struct Fields<'a>(&'a mut BTreeMap<String, String>);

    impl Visit for Fields<'_> {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }

        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0
                .insert(field.name().to_owned(), Shown(value).to_string());
        }
    }

    /// The process-wide subscriber of this test binary: a global default keeps the callsite
    /// interests stable while tests run in parallel threads, and `traced` keeps only the
    /// records of its own thread.
    struct Tap(Arc<Collector>);

    impl Subscriber for Tap {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let mut recorded = self.0.record(span.metadata());
            span.record(&mut Fields(&mut recorded.fields));
            let id = recorded.seq.saturating_add(1);
            self.0
                .spans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(id, recorded);
            Id::from_u64(id)
        }

        fn record(&self, span: &Id, values: &Record<'_>) {
            if let Some(recorded) = self
                .0
                .spans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_mut(&span.into_u64())
            {
                values.record(&mut Fields(&mut recorded.fields));
            }
        }

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut recorded = self.0.record(event.metadata());
            event.record(&mut Fields(&mut recorded.fields));
            self.0
                .events
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(recorded);
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    fn collector() -> &'static Collector {
        static COLLECTOR: OnceLock<Arc<Collector>> = OnceLock::new();
        COLLECTOR.get_or_init(|| {
            let collector = Arc::new(Collector::default());
            tracing::subscriber::set_global_default(Tap(Arc::clone(&collector)))
                .expect("the first install");
            collector
        })
    }

    /// Run `body` and return the spans and events it made on this thread.
    fn traced(body: impl FnOnce()) -> (Vec<Recorded>, Vec<Recorded>) {
        let collector = collector();
        let start = collector.seq.load(Ordering::Relaxed);
        let me = thread::current().id();
        body();
        let mine = |recorded: &&Recorded| recorded.thread == me && recorded.seq >= start;
        let spans = collector
            .spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter(mine)
            .cloned()
            .collect();
        let events = collector
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .filter(mine)
            .cloned()
            .collect();
        (spans, events)
    }

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting (logged)
    #[test]
    fn the_read_span_carries_scheme_redacted_endpoint_op_items_and_chunks() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![
            Step::Expect(adu(1, &[0x01, 0x00, 0x00, 0x00, 0x01])),
            Step::Reply(adu(1, &[0x01, 0x01, 0x01])),
            Step::Expect(request(2)),
            Step::Reply(reply(2, 5)),
        ]));
        let with_credentials = ConnectionConfig::builder(
            "modbus-tcp",
            Endpoint::Tcp {
                host: "plc.example".to_owned(),
                port: 502,
            },
        )
        .credentials(Credentials::new("operator", "example-only"))
        .build()
        .expect("config");
        let mut opened = None;
        let (open_spans, _) = traced(|| {
            opened = Some(
                block_on_with(
                    &clock,
                    ModbusConnection::open(
                        with_credentials,
                        Arc::new(factory.clone()),
                        Arc::new(clock.clone()),
                        ConnectionDefaults::default(),
                    ),
                )
                .expect("open"),
            );
        });
        let connection = opened.expect("a connection");
        let connect = open_spans
            .iter()
            .find(|span| span.name == "connect")
            .expect("a connect span");
        assert_eq!(connect.level, Level::INFO, "connect level");
        assert_eq!(
            connect.fields.get("op").map(String::as_str),
            Some("connect"),
            "connect op"
        );

        let request = ReadRequest::builder(connection.limits())
            .tag(tag("coil", "coil:1"))
            .tag(tag("register", "holding-register:1"))
            .build()
            .expect("request");
        let (spans, events) = traced(|| {
            drop(block_on_with(&clock, connection.read(request)).expect("the read"));
        });
        let read = spans
            .iter()
            .find(|span| span.name == "read")
            .expect("a read span");
        assert_eq!(read.level, Level::DEBUG, "debug level");
        let field = |name: &str| read.fields.get(name).map(String::as_str);
        assert_eq!(field("scheme"), Some("modbus-tcp"), "scheme");
        assert_eq!(field("endpoint"), Some("plc.example:502"), "endpoint");
        assert_eq!(field("op"), Some("read"), "op");
        assert_eq!(field("items"), Some("2"), "items");
        assert_eq!(field("chunks"), Some("2"), "chunks, recorded once planned");
        let txns: Vec<&str> = spans
            .iter()
            .filter(|span| span.name == "request")
            .filter_map(|span| span.fields.get("txn").map(String::as_str))
            .collect();
        assert_eq!(
            txns,
            ["1", "2"],
            "the session's request span nests per chunk"
        );

        let (close_spans, _) = traced(|| {
            block_on_with(&clock, connection.close()).expect("close");
        });
        let close = close_spans
            .iter()
            .find(|span| span.name == "close")
            .expect("a close span");
        assert_eq!(
            close.fields.get("op").map(String::as_str),
            Some("close"),
            "close op"
        );

        for record in open_spans
            .iter()
            .chain(&spans)
            .chain(&close_spans)
            .chain(&events)
        {
            for value in record.fields.values() {
                assert!(
                    !value.contains("example-only") && !value.contains("operator"),
                    "no credential in {}: {value}",
                    record.name
                );
            }
        }
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded (logged)
    #[test]
    fn a_retry_is_logged_at_debug_with_the_chunk_the_attempt_and_the_kind() {
        let mut timeouts = Timeouts::default();
        timeouts.request = secs(1);
        let mut retry = RetryPolicy::default();
        retry.max_attempts = NonZeroU32::new(2).expect("two");
        let config = ConnectionConfig::builder("modbus-tcp", fake_endpoint())
            .timeouts(timeouts)
            .retry(retry)
            .build()
            .expect("config");
        let rig = Rig::with(
            config,
            vec![
                Step::Expect(request(1)),
                Step::Delay(Duration::from_millis(1500)),
                Step::Reply(reply(1, 1)),
                Step::Expect(request(2)),
                Step::Reply(reply(2, 2)),
            ],
        );
        let (_, events) = traced(|| {
            let response = rig.read().expect("the second attempt succeeds");
            assert_eq!(value_of(&response), PlcValue::I16(2), "its reply");
        });
        let retry = events
            .iter()
            .find(|event| {
                event.fields.get("message").map(String::as_str) == Some("retrying the chunk")
            })
            .expect("a retry event");
        assert_eq!(retry.level, Level::DEBUG, "debug level");
        let field = |name: &str| retry.fields.get(name).map(String::as_str);
        assert_eq!(field("chunk"), Some("0"), "chunk index");
        assert_eq!(field("attempts"), Some("1"), "attempts made");
        assert_eq!(field("kind"), Some("Timeout"), "kind");
    }

    #[test]
    fn write_has_a_span_and_does_no_io() {
        let rig = Rig::open(vec![]);
        let request = stonecrab::WriteRequest::builder(rig.connection.limits())
            .item(tag("a", "holding-register:1"), PlcValue::I16(1))
            .build()
            .expect("write request");
        let (spans, _) = traced(|| {
            let error = error_of(block_on_with(&rig.clock, rig.connection.write(request)));
            assert_eq!(
                error.kind(),
                ErrorKind::Unsupported(Operation::Write),
                "kind"
            );
        });
        let write = spans
            .iter()
            .find(|span| span.name == "write")
            .expect("a write span");
        assert_eq!(
            write.fields.get("op").map(String::as_str),
            Some("write"),
            "op"
        );
        assert!(rig.sent().is_empty(), "nothing was sent");
    }
}
