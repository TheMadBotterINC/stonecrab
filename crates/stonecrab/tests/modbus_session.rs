//! `ModbusSession` over `FakeTransportFactory`, `FakeTransport` and `FakeClock`.
//!
//! The wire bytes are written out by hand from the MBAP layout (transaction id, protocol
//! id 0, length, unit id, PDU, all big-endian) and the PDU examples of the Modbus
//! Application Protocol Specification V1.1b sections 6.3 and 7; no byte comes from the
//! codec under test. Timing is on the fake clock only (decision D21): nothing here sleeps,
//! and a future that could never finish fails the test instead of hanging it.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::fmt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, PoisonError};
    use std::task::Poll;
    use std::thread::{self, ThreadId};
    use std::time::{Duration, Instant};

    use stonecrab::modbus::{
        ExceptionCode, FunctionCode, ModbusPdu, ModbusSession, SessionStats, TransactionId, UnitId,
        exception_item_error,
    };
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
    };
    use stonecrab::transport::GuardConfig;
    use stonecrab::{
        CancellationToken, Clock, ConnectionState, Deadline, Endpoint, ErrorKind, FailureCategory,
        Operation, PlcError, ResourceLimits, Result,
    };
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Event, Level, Metadata, Subscriber};

    /// V1.1b section 6.3: read three holding registers from wire address 107 (logical 108).
    const REQUEST_PDU: [u8; 5] = [0x03, 0x00, 0x6B, 0x00, 0x03];
    /// The reply of the same section: registers 555, 0 and 100.
    const REPLY_PDU: [u8; 8] = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
    /// A second, different reply, so a test can tell which reply it got: 1, 2 and 3.
    const OTHER_REPLY_PDU: [u8; 8] = [0x03, 0x06, 0x00, 0x01, 0x00, 0x02, 0x00, 0x03];
    const UNIT: u8 = 0x11;

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
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

    fn request_adu(txn: u16) -> Vec<u8> {
        adu(txn, UNIT, &REQUEST_PDU)
    }

    fn reply_adu(txn: u16) -> Vec<u8> {
        adu(txn, UNIT, &REPLY_PDU)
    }

    fn request_pdu() -> ModbusPdu {
        ModbusPdu::ReadHoldingRegisters {
            start: 107,
            quantity: 3,
        }
    }

    fn registers(values: &[u16]) -> ModbusPdu {
        ModbusPdu::ReadRegistersResponse {
            function: FunctionCode::ReadHoldingRegisters,
            registers: values.to_vec(),
        }
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    /// Debug text without a `{:?}` in the test body.
    struct Shown<'a>(&'a dyn fmt::Debug);

    impl fmt::Display for Shown<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Debug::fmt(self.0, f)
        }
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        session: ModbusSession,
        t0: Instant,
    }

    impl Rig {
        /// Request timeout 5 s, connect timeout 2 s, frame limit 260, buffer 1024.
        fn new() -> Self {
            Self::starting_at(None)
        }

        fn starting_at(first: Option<TransactionId>) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            let limits = ResourceLimits::builder()
                .max_frame_bytes(260)
                .max_buffered_bytes(1024)
                .max_pending_requests(4)
                .build()
                .expect("valid limits");
            let mut config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
            config.timeouts.connect = secs(2);
            config.timeouts.request = secs(5);
            let session =
                ModbusSession::new(config, Arc::new(factory.clone()), Arc::new(clock.clone()));
            let session = match first {
                Some(first) => session.starting_at(first),
                None => session,
            };
            let t0 = clock.now();
            Self {
                clock,
                factory,
                session,
                t0,
            }
        }

        /// A rig connected to a peer that runs `steps` (a strict script).
        fn connected(steps: Vec<Step>) -> Self {
            let rig = Self::new();
            rig.push(steps);
            rig.connect().expect("connect");
            rig
        }

        fn push(&self, steps: Vec<Step>) {
            self.factory.push_script(Script::strict(steps));
        }

        fn connect(&self) -> Result<()> {
            block_on_with(&self.clock, self.session.connect())
        }

        fn deadline(&self, seconds: u64) -> Deadline {
            Deadline::after(self.clock.now(), secs(seconds)).expect("representable")
        }

        /// One holding-register read of unit 17 with a deadline `seconds` from now.
        fn read(&self, seconds: u64) -> Result<ModbusPdu> {
            let request = request_pdu();
            block_on_with(
                &self.clock,
                self.session
                    .request(&request, UnitId::from(UNIT), self.deadline(seconds), None),
            )
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

        /// The instant `seconds` after the rig was built.
        fn at(&self, seconds: u64) -> Instant {
            self.t0.checked_add(secs(seconds)).expect("representable")
        }

        fn stats(&self) -> SessionStats {
            self.session.stats()
        }
    }

    /// A responder that answers every request with `REPLY_PDU` under the request's own
    /// transaction and unit id.
    fn echoing_responder() -> Step {
        Step::respond(|request| {
            let txn = u16::from_be_bytes(*request.first_chunk::<2>()?);
            let unit = *request.get(6)?;
            Some(adu(txn, unit, &REPLY_PDU))
        })
    }

    fn txn_of(frame: &[u8]) -> u16 {
        u16::from_be_bytes(*frame.first_chunk::<2>().expect("a frame has a header"))
    }

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting
    #[test]
    fn a_holding_register_read_round_trips_with_the_exact_request_bytes() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(reply_adu(1)),
        ]);
        assert_eq!(rig.session.state(), ConnectionState::Connected, "connected");
        let reply = rig.read(1).expect("the reply");
        assert_eq!(reply, registers(&[555, 0, 100]), "decoded registers");
        assert_eq!(
            rig.sent(0),
            vec![vec![
                0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x11, 0x03, 0x00, 0x6B, 0x00, 0x03
            ]],
            "the request on the wire, byte for byte"
        );
        assert_eq!(rig.session.state(), ConnectionState::Connected, "state");
        let stats = rig.stats();
        assert_eq!(
            (
                stats.late_replies,
                stats.mismatched_replies,
                stats.guard.requests
            ),
            (0, 0, 1),
            "nothing discarded, one request sent"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 incorrect transaction and unit ids, delayed replies after timeout
    #[test]
    fn reply_with_wrong_txn_is_discarded() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            // The first reply has every bit of its transaction id flipped: 0xFFFE.
            Step::WrongTransactionId,
            Step::Reply(reply_adu(1)),
            Step::Reply(reply_adu(1)),
        ]);
        let reply = rig.read(1).expect("the correct reply that follows");
        assert_eq!(
            reply,
            registers(&[555, 0, 100]),
            "the second reply is accepted"
        );
        let stats = rig.stats();
        assert_eq!(stats.mismatched_replies, 1, "counted as mismatched");
        assert_eq!(stats.late_replies, 0, "0xFFFE was never given up on");
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1)],
            "one request, nothing resent"
        );
        assert_eq!(rig.session.state(), ConnectionState::Connected, "state");
    }

    // covers: SPEC mandatory-failure-cases / F2 incorrect transaction and unit ids
    #[test]
    fn reply_with_wrong_unit_is_discarded() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, 0x12, &OTHER_REPLY_PDU)),
            Step::Reply(reply_adu(1)),
        ]);
        let reply = rig.read(1).expect("the reply from the right unit");
        assert_eq!(
            reply,
            registers(&[555, 0, 100]),
            "the reply of unit 17, not the one of unit 18"
        );
        let stats = rig.stats();
        assert_eq!(
            (stats.mismatched_replies, stats.late_replies),
            (1, 0),
            "the right transaction id with the wrong unit is mismatched, not late"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 incorrect transaction and unit ids
    #[test]
    fn reply_with_wrong_function_code_is_discarded() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            // A well-formed reply to function 04, and an exception for function 04.
            Step::Reply(adu(1, UNIT, &[0x04, 0x02, 0x00, 0x07])),
            Step::Reply(adu(1, UNIT, &[0x84, 0x02])),
            Step::Reply(reply_adu(1)),
        ]);
        let reply = rig.read(1).expect("the reply to function 03");
        assert_eq!(
            reply,
            registers(&[555, 0, 100]),
            "function 03 answers function 03"
        );
        assert_eq!(
            rig.stats().mismatched_replies,
            2,
            "both wrong functions counted"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn late_reply_after_timeout_is_discarded_and_the_next_correlation_succeeds() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            // The reply to request 1 arrives three seconds in, after its 1 s deadline.
            Step::LateReply(secs(3)),
            Step::Reply(reply_adu(1)),
            Step::Expect(request_adu(2)),
            Step::Reply(adu(2, UNIT, &OTHER_REPLY_PDU)),
        ]);
        let timed_out = error_of(rig.read(1));
        assert_eq!(timed_out.kind(), ErrorKind::Timeout, "no reply in time");
        assert_eq!(
            rig.session.state(),
            ConnectionState::Connected,
            "stream intact"
        );
        assert_eq!(rig.stats().guard.timeouts, 1, "counted as a timeout");
        assert_eq!(
            rig.clock.now(),
            rig.at(1),
            "the deadline, on the fake clock"
        );

        let reply = rig.read(5).expect("the second request succeeds");
        assert_eq!(
            reply,
            registers(&[1, 2, 3]),
            "the reply of request 2, not the stale one of request 1"
        );
        let stats = rig.stats();
        assert_eq!(stats.late_replies, 1, "the stale reply is counted as late");
        assert_eq!(stats.mismatched_replies, 0, "and not as mismatched");
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1), request_adu(2)],
            "the ids went on increasing"
        );
        assert_eq!(rig.clock.now(), rig.at(3), "waited for the stale reply");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn only_the_most_recent_abandoned_transactions_are_remembered_as_late() {
        // 40 requests that the peer never answers, then one answer that carries a reply
        // for the first, one for the last of them and the right one, in a single write.
        let mut asked = 0_u32;
        let rig = Rig::connected(vec![Step::respond(move |request| {
            asked = asked.saturating_add(1);
            if asked < 41 {
                return None;
            }
            let txn = u16::from_be_bytes(*request.first_chunk::<2>()?);
            let mut bytes = reply_adu(1);
            bytes.extend_from_slice(&reply_adu(40));
            bytes.extend_from_slice(&adu(txn, UNIT, &OTHER_REPLY_PDU));
            Some(bytes)
        })]);
        for attempt in 1..=40 {
            let error = error_of(rig.read(1));
            assert_eq!(
                error.kind(),
                ErrorKind::Timeout,
                "request {attempt} times out"
            );
        }
        let reply = rig.read(5).expect("the 41st request");
        assert_eq!(reply, registers(&[1, 2, 3]), "its own reply");
        let stats = rig.stats();
        assert_eq!(
            stats.late_replies, 1,
            "transaction 40 is among the 32 remembered"
        );
        assert_eq!(
            stats.mismatched_replies, 1,
            "transaction 1 was forgotten, so it is only a stray"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn a_request_refused_before_the_wire_is_not_remembered_as_given_up() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(2)),
            // A stray frame carrying transaction 1, which was refused before any I/O.
            Step::Reply(reply_adu(1)),
            Step::Reply(reply_adu(2)),
        ]);
        let token = CancellationToken::new();
        token.cancel();
        let request = request_pdu();
        let cancelled = error_of(block_on_with(
            &rig.clock,
            rig.session
                .request(&request, UnitId::from(UNIT), rig.deadline(1), Some(&token)),
        ));
        assert_eq!(cancelled.kind(), ErrorKind::Cancelled, "cancelled");
        assert!(rig.sent(0).is_empty(), "nothing sent");

        rig.read(1).expect("the second request");
        let stats = rig.stats();
        assert_eq!(
            (stats.mismatched_replies, stats.late_replies),
            (1, 0),
            "transaction 1 never reached the wire, so its reply is a stray, not late"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting
    #[test]
    fn read_times_out_after_request_timeout_on_fake_clock() {
        let rig = Rig::connected(vec![Step::Expect(request_adu(1))]);
        let error = error_of(rig.read(2));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(rig.clock.now(), rig.at(2), "exactly the deadline");
        let diagnostics = error.diagnostics();
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(diagnostics.latency, Some(secs(2)), "latency");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "state at the failure"
        );
        assert_eq!(diagnostics.retries, 0, "retries");
        assert!(diagnostics.category.is_some(), "category");
        assert_eq!(rig.stats().guard.timeouts, 1, "guard timeout counter");
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout
    #[test]
    fn the_deadline_also_bounds_the_wait_after_a_discarded_reply() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, 0x12, &REPLY_PDU)),
        ]);
        let error = error_of(rig.read(2));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the right reply never comes"
        );
        assert_eq!(rig.clock.now(), rig.at(2), "within the one deadline");
        assert_eq!(
            rig.stats().mismatched_replies,
            1,
            "the wrong unit was discarded"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "the error of the second wait is stamped too"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport closed during a discarded reply
    #[test]
    fn a_close_after_a_discarded_reply_is_disconnected() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, 0x12, &REPLY_PDU)),
            Step::CloseAt(0),
        ]);
        let error = error_of(rig.read(2));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(
            rig.stats().mismatched_replies,
            1,
            "the stray reply was counted"
        );
        assert!(
            matches!(rig.session.state(), ConnectionState::Reconnecting { .. }),
            "the next request reopens"
        );
    }

    fn exception_round_trip(code: u8) -> (ModbusPdu, ConnectionState) {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, UNIT, &[0x83, code])),
        ]);
        let reply = rig
            .read(1)
            .expect("an exception is an answer, not an error");
        (reply, rig.session.state())
    }

    /// An exception with `code` comes back as `Ok`, leaves the connection `Connected` and
    /// maps to `kind` with the raw code and the V1.1b name kept.
    fn assert_exception(code: u8, kind: ErrorKind, name: &str) {
        let (reply, state) = exception_round_trip(code);
        assert_eq!(state, ConnectionState::Connected, "code {code}: state");
        let ModbusPdu::Exception {
            function,
            code: received,
        } = reply
        else {
            panic!("code {code}: expected an exception PDU");
        };
        assert_eq!(
            function,
            FunctionCode::ReadHoldingRegisters,
            "code {code}: function"
        );
        assert_eq!(received.as_u8(), code, "code {code}: raw code");
        let item = exception_item_error(function, received);
        assert_eq!(item.kind(), kind, "code {code}: kind");
        let status = item.status().expect("the status is always kept");
        assert_eq!(
            (status.protocol(), status.code(), status.name()),
            ("modbus", u32::from(code), Some(name)),
            "code {code}: status"
        );
        assert_eq!(item.error().kind(), kind, "code {code}: error kind");
        let diagnostics = item.error().diagnostics();
        assert_eq!(
            diagnostics.operation,
            Some(Operation::Read),
            "code {code}: operation"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Protocol),
            "code {code}: category"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_01_maps_to_unsupported_read() {
        assert_exception(
            1,
            ErrorKind::Unsupported(Operation::Read),
            "ILLEGAL FUNCTION",
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_02_maps_to_invalid_address() {
        assert_exception(2, ErrorKind::InvalidAddress, "ILLEGAL DATA ADDRESS");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_03_maps_to_device_exception_with_raw_code() {
        assert_exception(3, ErrorKind::DeviceException, "ILLEGAL DATA VALUE");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_04_maps_to_device_exception() {
        assert_exception(4, ErrorKind::DeviceException, "SERVER DEVICE FAILURE");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_05_maps_to_device_exception_not_success() {
        assert_exception(5, ErrorKind::DeviceException, "ACKNOWLEDGE");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_06_maps_to_device_exception() {
        assert_exception(6, ErrorKind::DeviceException, "SERVER DEVICE BUSY");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_07_maps_to_device_exception() {
        assert_exception(7, ErrorKind::DeviceException, "NEGATIVE ACKNOWLEDGE");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_08_maps_to_device_exception() {
        assert_exception(8, ErrorKind::DeviceException, "MEMORY PARITY ERROR");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_0a_maps_to_device_exception() {
        assert_exception(10, ErrorKind::DeviceException, "GATEWAY PATH UNAVAILABLE");
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_0b_maps_to_device_exception() {
        assert_exception(
            11,
            ErrorKind::DeviceException,
            "GATEWAY TARGET DEVICE FAILED TO RESPOND",
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn exception_unknown_code_is_retained_as_device_exception() {
        // The wire decoder refuses a code outside the ten defined ones, so the session
        // reports the reply as malformed (MB-EXC-UNKNOWN)...
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, UNIT, &[0x83, 0x09])),
        ]);
        let error = error_of(rig.read(1));
        assert_eq!(
            error.kind(),
            ErrorKind::MalformedData,
            "refused on the wire"
        );
        // ...and the mapping itself keeps a number a caller constructs.
        let item = exception_item_error(FunctionCode::ReadCoils, ExceptionCode::Unknown(9));
        assert_eq!(item.kind(), ErrorKind::DeviceException, "kind");
        let status = item.status().expect("status");
        assert_eq!(
            (status.code(), status.name()),
            (9, Some("UNKNOWN")),
            "raw code kept"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn an_exception_leaves_the_connection_connected_and_the_next_read_works() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, UNIT, &[0x83, 0x01])),
            Step::Expect(request_adu(2)),
            Step::Reply(adu(2, UNIT, &[0x83, 0x02])),
            Step::Expect(request_adu(3)),
            Step::Reply(adu(3, UNIT, &[0x83, 0x05])),
            Step::Expect(request_adu(4)),
            Step::Reply(adu(4, UNIT, &REPLY_PDU)),
        ]);
        let mut seen = Vec::new();
        for _ in 0..3 {
            let ModbusPdu::Exception { function, code } = rig.read(1).expect("exception") else {
                panic!("expected an exception PDU");
            };
            seen.push(exception_item_error(function, code).kind());
            assert_eq!(rig.session.state(), ConnectionState::Connected, "state");
        }
        assert_eq!(
            seen,
            vec![
                ErrorKind::Unsupported(Operation::Read),
                ErrorKind::InvalidAddress,
                ErrorKind::DeviceException
            ],
            "codes 1, 2 and 5"
        );
        assert_eq!(
            rig.read(1).expect("a normal read after three exceptions"),
            registers(&[555, 0, 100]),
            "the connection is still good"
        );
        let stats = rig.stats();
        assert_eq!(
            (
                stats.guard.reconnects,
                stats.guard.resyncs,
                stats.mismatched_replies
            ),
            (0, 0, 0),
            "no reconnect, no resync, nothing discarded"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 byte count against quantity
    #[test]
    fn a_reply_whose_byte_count_does_not_fit_the_quantity_is_malformed_data() {
        // Two registers for a request of three.
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, UNIT, &[0x03, 0x04, 0x00, 0x01, 0x00, 0x02])),
            Step::Expect(request_adu(2)),
            Step::Reply(reply_adu(2)),
        ]);
        let error = error_of(rig.read(1));
        assert_eq!(error.kind(), ErrorKind::MalformedData, "kind");
        let diagnostics = error.diagnostics();
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Protocol),
            "category"
        );
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "state"
        );
        assert!(diagnostics.latency.is_some(), "latency");
        assert!(error.source().is_some(), "the parse error is the source");

        // The framer cut exactly one ADU, so the stream is aligned: no resync is needed.
        assert_eq!(
            rig.read(1).expect("the next request"),
            registers(&[555, 0, 100]),
            "recovers on the same connection"
        );
        let stats = rig.stats();
        assert_eq!(
            (stats.guard.resyncs, stats.guard.reconnects),
            (0, 0),
            "no resync"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators
    #[test]
    fn a_reply_pdu_that_does_not_decode_is_malformed_data() {
        // A byte count of zero.
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(adu(1, UNIT, &[0x03, 0x00])),
        ]);
        let error = error_of(rig.read(1));
        assert_eq!(error.kind(), ErrorKind::MalformedData, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Protocol),
            "category"
        );
        assert_eq!(rig.session.state(), ConnectionState::Connected, "state");
    }

    // covers: SPEC mandatory-failure-cases / F1 request bounds
    #[test]
    fn a_request_that_cannot_be_encoded_is_refused_before_any_io() {
        let rig = Rig::connected(vec![]);
        let unit = UnitId::from(UNIT);
        let cases = [
            (
                ModbusPdu::ReadHoldingRegisters {
                    start: 0,
                    quantity: 0,
                },
                ErrorKind::MalformedData,
            ),
            (
                ModbusPdu::ReadHoldingRegisters {
                    start: 0,
                    quantity: 126,
                },
                ErrorKind::ResourceLimit,
            ),
            (registers(&[1]), ErrorKind::MalformedData),
        ];
        for (pdu, kind) in cases {
            let error = error_of(block_on_with(
                &rig.clock,
                rig.session.request(&pdu, unit, rig.deadline(1), None),
            ));
            assert_eq!(error.kind(), kind, "kind");
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Validation),
                "category"
            );
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Read),
                "operation"
            );
        }
        assert!(rig.sent(0).is_empty(), "nothing reached the wire");
        assert_eq!(rig.stats().guard.requests, 0, "no request was sent");
    }

    // covers: SPEC mandatory-failure-cases / F2 request before the connection is open
    #[test]
    fn a_request_before_connect_is_disconnected_with_read_diagnostics() {
        let rig = Rig::new();
        let error = error_of(rig.read(1));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert_eq!(
            error.diagnostics().connection_state,
            Some(ConnectionState::Disconnected),
            "state"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 truncated frames
    #[test]
    fn a_truncated_reply_then_close_is_disconnected_and_the_session_recovers_after_reconnect() {
        let rig = Rig::new();
        // The first connection delivers ten bytes of the reply and then closes.
        rig.push(vec![
            Step::Expect(request_adu(1)),
            Step::Truncate(10),
            Step::Reply(reply_adu(1)),
            Step::CloseAt(0),
        ]);
        rig.push(vec![
            Step::Expect(request_adu(2)),
            Step::Reply(reply_adu(2)),
        ]);
        rig.connect().expect("connect");
        let error = error_of(rig.read(1));
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "ten bytes and then end of stream"
        );
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert!(
            matches!(
                rig.session.state(),
                ConnectionState::Reconnecting { attempt: 1, .. }
            ),
            "the guard schedules the reconnect"
        );

        let reply = rig
            .read(5)
            .expect("the next request reopens the connection");
        assert_eq!(reply, registers(&[555, 0, 100]), "reply");
        assert_eq!(rig.session.state(), ConnectionState::Connected, "recovered");
        assert_eq!(rig.factory.opens().len(), 2, "one reconnect");
        assert_eq!(
            rig.sent(1),
            vec![request_adu(2)],
            "ids continue across the reconnect"
        );
        assert_eq!(rig.stats().guard.reconnects, 1, "counted");
    }

    // covers: SPEC mandatory-failure-cases / F2 close mid-reply
    #[test]
    fn close_mid_reply_is_disconnected_and_reconnecting() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::CloseAt(7),
            Step::Reply(reply_adu(1)),
        ]);
        let error = error_of(rig.read(1));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert!(
            matches!(rig.session.state(), ConnectionState::Reconnecting { .. }),
            "state"
        );
        assert!(
            error.is_retryable(),
            "a lost connection is retryable by policy"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (transmitting)
    #[test]
    fn cancel_before_flush_is_cancelled_with_nothing_sent() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(2)),
            Step::Reply(reply_adu(2)),
        ]);
        let token = CancellationToken::new();
        token.cancel();
        let request = request_pdu();
        let error = error_of(block_on_with(
            &rig.clock,
            rig.session
                .request(&request, UnitId::from(UNIT), rig.deadline(1), Some(&token)),
        ));
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert!(rig.sent(0).is_empty(), "nothing was sent");
        assert_eq!(rig.stats().guard.requests, 0, "no request counted");
        assert_eq!(rig.session.state(), ConnectionState::Connected, "state");
        assert_eq!(
            rig.read(1).expect("the session is usable"),
            registers(&[555, 0, 100]),
            "next request"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting)
    #[test]
    fn cancel_after_flush_is_cancelled_and_the_next_request_resyncs() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            // Three bytes of the reply arrive, the rest three seconds later.
            Step::Reply(reply_adu(1).get(..3).expect("prefix").to_vec()),
            Step::Delay(secs(3)),
            Step::Reply(reply_adu(1).get(3..).expect("rest").to_vec()),
            Step::Expect(request_adu(2)),
            Step::Reply(adu(2, UNIT, &OTHER_REPLY_PDU)),
        ]);
        let token = CancellationToken::new();
        let request = request_pdu();
        let mut first =
            rig.session
                .request(&request, UnitId::from(UNIT), rig.deadline(10), Some(&token));
        assert!(
            poll_once(&mut first).is_pending(),
            "awaiting the rest of the reply"
        );
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut first) else {
            panic!("a cancelled request resolves at once");
        };
        let error = error_of(outcome);
        assert_eq!(error.kind(), ErrorKind::Cancelled, "kind");
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1)],
            "the frame did reach the wire"
        );
        assert_eq!(
            rig.session.state(),
            ConnectionState::Connected,
            "stream intact"
        );
        drop(first);

        let reply = rig.read(10).expect("the next request");
        assert_eq!(
            reply,
            registers(&[1, 2, 3]),
            "its own reply, after the abandoned one was resynced away"
        );
        let stats = rig.stats();
        assert_eq!(
            stats.guard.resyncs, 1,
            "the guard resynchronized before the send"
        );
        assert_eq!(stats.guard.requests, 2, "two requests sent");
        assert_eq!(
            (stats.late_replies, stats.mismatched_replies),
            (0, 0),
            "the resync dropped the stale reply before the session saw it"
        );
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1), request_adu(2)],
            "the wire"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 transmitting and awaiting
    #[test]
    fn one_request_in_flight_at_a_time() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Expect(request_adu(2)),
            // The reply to the dropped request comes only now, then the one to the next.
            Step::Reply(reply_adu(1)),
            Step::Reply(adu(2, UNIT, &OTHER_REPLY_PDU)),
        ]);
        let request = request_pdu();
        let mut first = rig
            .session
            .request(&request, UnitId::from(UNIT), rig.deadline(10), None);
        assert!(
            poll_once(&mut first).is_pending(),
            "the first request waits for its reply"
        );

        let refused = error_of(rig.read(1));
        assert_eq!(
            refused.kind(),
            ErrorKind::Backpressure,
            "a second request is refused"
        );
        assert_eq!(
            refused.diagnostics().operation,
            Some(Operation::Read),
            "operation"
        );
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1)],
            "the refused one sent nothing"
        );

        // Dropping the first request frees the slot (and the refused one took no id).
        drop(first);
        assert_eq!(
            rig.read(1).expect("a request after the first was dropped"),
            registers(&[1, 2, 3]),
            "next request"
        );
        assert_eq!(
            rig.sent(0),
            vec![request_adu(1), request_adu(2)],
            "the wire"
        );
        assert_eq!(
            (rig.stats().late_replies, rig.stats().mismatched_replies),
            (1, 0),
            "the dropped request's id was remembered, so its reply is late"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transaction id wrap
    #[test]
    fn transaction_ids_increase_and_wrap_from_fffe_to_1() {
        let rig = Rig::new();
        rig.push(vec![echoing_responder()]);
        rig.connect().expect("connect");
        for _ in 0..3 {
            rig.read(1).expect("read");
        }
        let ids: Vec<u16> = rig.sent(0).iter().map(|frame| txn_of(frame)).collect();
        assert_eq!(ids, vec![1, 2, 3], "ids start at 1 and increase");

        let rig = Rig::starting_at(Some(TransactionId::LAST));
        rig.push(vec![echoing_responder()]);
        rig.connect().expect("connect");
        for _ in 0..3 {
            rig.read(1).expect("read");
        }
        let ids: Vec<u16> = rig.sent(0).iter().map(|frame| txn_of(frame)).collect();
        assert_eq!(
            ids,
            vec![0xFFFE, 1, 2],
            "0xFFFE wraps to 1; 0 and 0xFFFF are never used"
        );
    }

    #[test]
    fn close_ends_the_session_and_a_later_request_is_disconnected() {
        let rig = Rig::connected(vec![]);
        block_on_with(&rig.clock, rig.session.close()).expect("close");
        assert_eq!(rig.session.state(), ConnectionState::Closed, "state");
        let error = error_of(rig.read(1));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
    }

    #[test]
    fn the_watch_follows_the_connection_and_debug_shows_no_payload() {
        let rig = Rig::new();
        let watch = rig.session.watch_state();
        assert_eq!(
            watch.current(),
            ConnectionState::Disconnected,
            "before connect"
        );
        rig.push(vec![]);
        rig.connect().expect("connect");
        assert_eq!(watch.current(), ConnectionState::Connected, "after connect");
        let text = Shown(&rig.session).to_string();
        assert!(text.contains("ModbusSession"), "type name: {text}");
        assert!(text.contains("fake://plc"), "endpoint: {text}");
        assert!(text.contains("Connected"), "state: {text}");
    }

    // ---- tracing ----

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
        fn next_seq(&self) -> u64 {
            self.seq.fetch_add(1, Ordering::Relaxed)
        }

        fn record(&self, metadata: &Metadata<'_>) -> Recorded {
            Recorded {
                seq: self.next_seq(),
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

    /// The process-wide subscriber of this test binary. A global default keeps the callsite
    /// interests stable while tests run in parallel threads (a scoped subscriber would race
    /// the interest cache of a callsite another test hit first); every record carries its
    /// thread and a sequence number, and `traced` keeps only the ones of its own call.
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
        // tracing-core files a callsite before it asks the default subscriber for its
        // interest, so a callsite another test first hits while `collector` installs the
        // tap can keep the `never` it got from no subscriber, and its spans are lost.
        // Recompute every interest now that the tap is the global default.
        tracing::callsite::rebuild_interest_cache();
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

    // covers: SPEC mandatory-failure-cases / F2 incorrect transaction and unit ids (logged)
    #[test]
    fn a_discarded_reply_is_logged_at_warn_with_the_expected_and_received_txn() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::WrongTransactionId,
            Step::Reply(reply_adu(1)),
            Step::Reply(reply_adu(1)),
        ]);
        let (_, events) = traced(|| {
            rig.read(1).expect("the correct reply that follows");
        });
        let warnings: Vec<&Recorded> = events
            .iter()
            .filter(|event| event.level == Level::WARN)
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "exactly one warning for one discarded reply"
        );
        let fields = &warnings.first().expect("one warning").fields;
        assert_eq!(
            fields.get("expected_txn").map(String::as_str),
            Some("1"),
            "expected"
        );
        assert_eq!(
            fields.get("received_txn").map(String::as_str),
            Some("65534"),
            "received (0xFFFE)"
        );
        assert_eq!(
            fields.get("late").map(String::as_str),
            Some("false"),
            "not late"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 delayed replies after timeout (logged)
    #[test]
    fn a_late_reply_is_logged_as_late() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::LateReply(secs(3)),
            Step::Reply(reply_adu(1)),
            Step::Expect(request_adu(2)),
            Step::Reply(reply_adu(2)),
        ]);
        let (_, events) = traced(|| {
            assert!(rig.read(1).is_err(), "the first request times out");
            rig.read(5).expect("the second request");
        });
        let warning = events
            .iter()
            .find(|event| event.level == Level::WARN)
            .expect("a warning");
        assert_eq!(
            warning.fields.get("late").map(String::as_str),
            Some("true"),
            "late"
        );
        assert_eq!(
            warning.fields.get("expected_txn").map(String::as_str),
            Some("2"),
            "expected"
        );
        assert_eq!(
            warning.fields.get("received_txn").map(String::as_str),
            Some("1"),
            "received"
        );
    }

    #[test]
    fn the_request_span_carries_scheme_endpoint_op_and_txn() {
        let rig = Rig::connected(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(reply_adu(1)),
        ]);
        let (spans, _) = traced(|| {
            rig.read(1).expect("reply");
        });
        let span = spans
            .iter()
            .find(|span| span.name == "request")
            .expect("a request span");
        assert_eq!(span.level, Level::DEBUG, "debug level");
        let field = |name: &str| span.fields.get(name).map(String::as_str);
        assert_eq!(field("scheme"), Some("modbus-tcp"), "scheme");
        assert_eq!(field("endpoint"), Some("fake://plc"), "endpoint");
        assert_eq!(field("op"), Some("read"), "op");
        assert_eq!(field("txn"), Some("1"), "txn");
    }

    #[test]
    fn a_tcp_endpoint_is_shown_as_host_and_port_only() {
        // The span shows what Endpoint displays: host and port, nothing else.
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![
            Step::Expect(request_adu(1)),
            Step::Reply(reply_adu(1)),
        ]));
        let limits = ResourceLimits::builder().build().expect("limits");
        let config = GuardConfig::new(
            Endpoint::Tcp {
                host: "plc.example".to_owned(),
                port: 502,
            },
            limits,
        );
        let session = ModbusSession::new(config, Arc::new(factory), Arc::new(clock.clone()));
        block_on_with(&clock, session.connect()).expect("connect");
        let (spans, _) = traced(|| {
            let request = request_pdu();
            let deadline = Deadline::after(clock.now(), secs(1)).expect("deadline");
            block_on_with(
                &clock,
                session.request(&request, UnitId::from(UNIT), deadline, None),
            )
            .expect("reply");
        });
        let span = spans
            .iter()
            .find(|span| span.name == "request")
            .expect("span");
        assert_eq!(
            span.fields.get("endpoint").map(String::as_str),
            Some("plc.example:502"),
            "endpoint"
        );
    }
}
