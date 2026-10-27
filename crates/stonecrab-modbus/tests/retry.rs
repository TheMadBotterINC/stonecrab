//! Read retry and per-chunk deadlines of `ModbusConnection`, on `FakeTransportFactory`,
//! `FakeTransport` and `FakeClock`.
//!
//! `sent().len()` of the fake transport is the transmission count: a retry is a second
//! request frame on the wire with the next transaction id. The wire bytes are written out
//! by hand from the MBAP layout and the V1.1b PDU examples. Nothing sleeps on real time
//! (decision D21).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab_core::{
        AddressOptions, Clock, Connection, ConnectionConfig, ConnectionConfigBuilder,
        ConnectionState, Deadline, Endpoint, ErrorKind, FailureCategory, Operation, PlcError,
        PlcValue, ReadRequest, ReadResponse, Result, RetryPolicy, Tag, TagId, Timeouts,
        WriteRequest,
    };
    use stonecrab_modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};

    fn millis(count: u64) -> Duration {
        Duration::from_millis(count)
    }

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

    /// A read request of unit 1: function code, wire start address and quantity.
    fn read_request(txn: u16, function: u8, start: u16, quantity: u16) -> Vec<u8> {
        let mut pdu = vec![function];
        pdu.extend_from_slice(&start.to_be_bytes());
        pdu.extend_from_slice(&quantity.to_be_bytes());
        adu(txn, 1, &pdu)
    }

    /// A register reply of unit 1.
    fn registers_reply(txn: u16, function: u8, registers: &[u16]) -> Vec<u8> {
        let count = u8::try_from(registers.len().saturating_mul(2)).expect("at most 125");
        let mut pdu = vec![function, count];
        for register in registers {
            pdu.extend_from_slice(&register.to_be_bytes());
        }
        adu(txn, 1, &pdu)
    }

    /// A bit reply of unit 1.
    fn bits_reply(txn: u16, function: u8, packed: &[u8]) -> Vec<u8> {
        let count = u8::try_from(packed.len()).expect("at most 250");
        let mut pdu = vec![function, count];
        pdu.extend_from_slice(packed);
        adu(txn, 1, &pdu)
    }

    /// An exception reply of unit 1.
    fn exception_reply(txn: u16, function: u8, code: u8) -> Vec<u8> {
        adu(txn, 1, &[function | 0x80, code])
    }

    fn tag(id: &str, address: &str) -> Tag {
        let address = ModbusAddress::parse(address, &AddressOptions::default()).expect("address");
        Tag::new(TagId::new(id).expect("tag id"), Arc::new(address))
    }

    fn policy(max_attempts: u32, kinds: &[ErrorKind]) -> RetryPolicy {
        let mut policy = RetryPolicy::default();
        policy.max_attempts = NonZeroU32::new(max_attempts).expect("positive");
        policy.retry_on = kinds.iter().copied().collect();
        policy
    }

    fn timeouts(request: Duration) -> Timeouts {
        let mut timeouts = Timeouts::default();
        timeouts.connect = secs(2);
        timeouts.request = request;
        timeouts
    }

    fn base_config(request: Duration, retry: RetryPolicy) -> ConnectionConfigBuilder {
        ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
            .timeouts(timeouts(request))
            .retry(retry)
    }

    struct Rig {
        clock: FakeClock,
        factory: FakeTransportFactory,
        connection: ModbusConnection,
        t0: Instant,
    }

    impl Rig {
        fn open(config: ConnectionConfig, steps: Vec<Step>) -> Self {
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

        fn request(&self, tags: Vec<Tag>, deadline: Option<Deadline>) -> ReadRequest {
            let mut builder = ReadRequest::builder(self.connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            if let Some(deadline) = deadline {
                builder = builder.deadline(deadline);
            }
            builder.build().expect("request")
        }

        fn read(&self, tags: Vec<Tag>) -> Result<ReadResponse> {
            block_on_with(&self.clock, self.connection.read(self.request(tags, None)))
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

        fn elapsed(&self) -> Duration {
            self.clock.now().saturating_duration_since(self.t0)
        }
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn read_default_policy_sends_once() {
        let rig = Rig::open(
            base_config(secs(1), RetryPolicy::default())
                .build()
                .expect("config"),
            vec![Step::Expect(read_request(1, 0x03, 0, 1))],
        );
        let error = error_of(rig.read(vec![tag("a", "holding-register:1")]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            rig.sent().len(),
            1,
            "one transmission: the default policy never retries"
        );
        assert_eq!(error.diagnostics().retries, 0, "no retries performed");
        assert_eq!(rig.elapsed(), secs(1), "the request timeout, exactly");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn read_retries_timeout_once_then_succeeds_within_max_attempts() {
        let rig = Rig::open(
            base_config(secs(1), policy(2, &[ErrorKind::Timeout]))
                .build()
                .expect("config"),
            vec![
                Step::Expect(read_request(1, 0x03, 0, 1)),
                // The first reply is 1.5 s in, after the 1 s chunk deadline; it is stale by
                // the time the second attempt, sent at 1 s, can read it.
                Step::Delay(millis(1500)),
                Step::Reply(registers_reply(1, 0x03, &[111])),
                Step::Expect(read_request(2, 0x03, 0, 1)),
                Step::Reply(registers_reply(2, 0x03, &[222])),
            ],
        );
        let response = rig
            .read(vec![tag("a", "holding-register:1")])
            .expect("the second attempt succeeds");
        let value = response
            .items()
            .first()
            .expect("item")
            .outcome()
            .as_ref()
            .expect("a value")
            .clone();
        assert_eq!(value, PlcValue::I16(222), "the reply of the second attempt");
        assert_eq!(
            rig.sent(),
            vec![read_request(1, 0x03, 0, 1), read_request(2, 0x03, 0, 1)],
            "two transmissions, the second with the next transaction id"
        );
        assert_eq!(rig.sent().len(), 2, "the transmission count");
        assert_eq!(
            rig.elapsed(),
            millis(1500),
            "the stale reply arrived at 1.5 s and was discarded before the right one"
        );
        assert_eq!(rig.connection.state(), ConnectionState::Connected, "state");
        assert_eq!(response.split_into().get(), 1, "still one chunk");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn read_does_not_retry_device_exception() {
        let rig = Rig::open(
            base_config(
                secs(1),
                policy(3, &[ErrorKind::Timeout, ErrorKind::DeviceException]),
            )
            .build()
            .expect("config"),
            vec![
                Step::Expect(read_request(1, 0x03, 0, 1)),
                // 06, SERVER DEVICE BUSY: an answer, never a reason to send again.
                Step::Reply(exception_reply(1, 0x03, 0x06)),
            ],
        );
        let response = rig
            .read(vec![tag("a", "holding-register:1")])
            .expect("an exception is an answer");
        assert_eq!(rig.sent().len(), 1, "one transmission");
        let error = response
            .items()
            .first()
            .expect("item")
            .outcome()
            .as_ref()
            .expect_err("the member carries the exception");
        assert_eq!(error.kind(), ErrorKind::DeviceException, "kind");
        assert_eq!(
            error.status().map(stonecrab_core::ProtocolStatus::code),
            Some(6),
            "raw code"
        );
        assert_eq!(error.error().diagnostics().retries, 0, "no retries");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn a_kind_outside_the_retry_set_is_not_retried() {
        let rig = Rig::open(
            base_config(secs(1), policy(3, &[ErrorKind::Disconnected]))
                .build()
                .expect("config"),
            vec![Step::Expect(read_request(1, 0x03, 0, 1))],
        );
        let error = error_of(rig.read(vec![tag("a", "holding-register:1")]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            rig.sent().len(),
            1,
            "a timeout is not in the set, so one transmission"
        );
        assert_eq!(rig.elapsed(), secs(1), "one chunk deadline");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn max_attempts_exhausted_fails_with_the_retries_in_the_diagnostics() {
        let rig = Rig::open(
            base_config(secs(1), policy(3, &[ErrorKind::Timeout]))
                .build()
                .expect("config"),
            vec![],
        );
        let error = error_of(rig.read(vec![tag("a", "holding-register:1")]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            rig.sent(),
            vec![
                read_request(1, 0x03, 0, 1),
                read_request(2, 0x03, 0, 1),
                read_request(3, 0x03, 0, 1)
            ],
            "exactly max_attempts transmissions"
        );
        let diagnostics = error.diagnostics();
        assert_eq!(
            diagnostics.retries, 2,
            "two retries performed after the first attempt"
        );
        assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "state"
        );
        assert_eq!(
            diagnostics.latency,
            Some(secs(3)),
            "latency of the whole read"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert!(
            error.to_string().starts_with("chunk 1 of 1:"),
            "the chunk is named: {error}"
        );
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn a_retry_repeats_only_the_chunk_that_failed() {
        let rig = Rig::open(
            base_config(secs(1), policy(2, &[ErrorKind::Timeout]))
                .build()
                .expect("config"),
            vec![
                Step::Expect(read_request(1, 0x01, 0, 1)),
                Step::Reply(bits_reply(1, 0x01, &[0x01])),
                Step::Expect(read_request(2, 0x03, 0, 1)),
                Step::Delay(millis(1500)),
                Step::Reply(registers_reply(2, 0x03, &[5])),
                Step::Expect(read_request(3, 0x03, 0, 1)),
                Step::Reply(registers_reply(3, 0x03, &[6])),
            ],
        );
        let response = rig
            .read(vec![
                tag("coil", "coil:1"),
                tag("register", "holding-register:1"),
            ])
            .expect("the response");
        assert_eq!(
            rig.sent(),
            vec![
                read_request(1, 0x01, 0, 1),
                read_request(2, 0x03, 0, 1),
                read_request(3, 0x03, 0, 1)
            ],
            "the coil chunk was sent once, the register chunk twice"
        );
        assert_eq!(response.split_into().get(), 2, "two chunks");
        let register = response
            .get(&TagId::new("register").expect("id"))
            .expect("item")
            .outcome()
            .as_ref()
            .expect("value")
            .clone();
        assert_eq!(register, PlcValue::I16(6), "the retried chunk's reply");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn retry_stops_when_the_request_deadline_has_passed() {
        let rig = Rig::open(
            base_config(secs(1), policy(5, &[ErrorKind::Timeout]))
                .build()
                .expect("config"),
            vec![],
        );
        let deadline = Deadline::after(rig.clock.now(), millis(1500)).expect("deadline");
        let request = rig.request(vec![tag("a", "holding-register:1")], Some(deadline));
        let error = error_of(block_on_with(&rig.clock, rig.connection.read(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            rig.sent().len(),
            2,
            "attempt 1 ends at 1 s, attempt 2 at the 1.5 s request deadline, and no third \
             attempt although the policy allows five"
        );
        assert_eq!(error.diagnostics().retries, 1, "one retry performed");
        assert_eq!(rig.elapsed(), millis(1500), "the request deadline");
    }

    /// How long a read of one register takes to time out against a silent peer, with the
    /// connection's request timeout and an optional request deadline, both from the start.
    fn time_to_timeout(request_timeout: Duration, deadline: Option<Duration>) -> Duration {
        let rig = Rig::open(
            base_config(request_timeout, RetryPolicy::default())
                .build()
                .expect("config"),
            vec![],
        );
        let deadline =
            deadline.map(|after| Deadline::after(rig.clock.now(), after).expect("deadline"));
        let request = rig.request(vec![tag("a", "holding-register:1")], deadline);
        let error = error_of(block_on_with(&rig.clock, rig.connection.read(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        rig.elapsed()
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn the_chunk_deadline_is_the_smaller_of_the_request_deadline_and_the_request_timeout() {
        assert_eq!(
            time_to_timeout(secs(2), None),
            secs(2),
            "no request deadline: timeouts.request"
        );
        assert_eq!(
            time_to_timeout(secs(5), Some(secs(1))),
            secs(1),
            "a request deadline inside timeouts.request wins"
        );
        assert_eq!(
            time_to_timeout(secs(2), Some(secs(10))),
            secs(2),
            "timeouts.request bounds a chunk even under a far request deadline"
        );
    }

    /// A peer that answers a coil chunk and a register chunk, each `1.5 s` after its
    /// request.
    fn slow_peer() -> Vec<Step> {
        vec![
            Step::Expect(read_request(1, 0x01, 0, 1)),
            Step::Delay(millis(1500)),
            Step::Reply(bits_reply(1, 0x01, &[0x01])),
            Step::Expect(read_request(2, 0x03, 0, 1)),
            Step::Delay(millis(1500)),
            Step::Reply(registers_reply(2, 0x03, &[9])),
        ]
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn every_chunk_gets_its_own_request_timeout() {
        let rig = Rig::open(
            base_config(secs(2), RetryPolicy::default())
                .build()
                .expect("config"),
            slow_peer(),
        );
        let response = rig
            .read(vec![
                tag("coil", "coil:1"),
                tag("register", "holding-register:1"),
            ])
            .expect("each chunk answers within its own 2 s");
        assert_eq!(response.split_into().get(), 2, "two chunks");
        assert_eq!(
            rig.elapsed(),
            secs(3),
            "3 s in all, more than one request timeout"
        );
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn the_request_deadline_still_bounds_a_later_chunk() {
        let rig = Rig::open(
            base_config(secs(2), RetryPolicy::default())
                .build()
                .expect("config"),
            slow_peer(),
        );
        let deadline = Deadline::after(rig.clock.now(), millis(2500)).expect("deadline");
        let request = rig.request(
            vec![tag("coil", "coil:1"), tag("register", "holding-register:1")],
            Some(deadline),
        );
        let error = error_of(block_on_with(&rig.clock, rig.connection.read(request)));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the second chunk ran out of time"
        );
        assert!(
            error.to_string().starts_with("chunk 2 of 2:"),
            "the second chunk is named: {error}"
        );
        assert_eq!(rig.elapsed(), millis(2500), "at the request deadline");
        assert_eq!(rig.sent().len(), 2, "both chunks were requested");
    }

    // covers: SPEC connection-and-resource-behavior / read retry bounded
    #[test]
    fn writes_are_never_retried_and_send_nothing() {
        let rig = Rig::open(
            base_config(
                secs(1),
                policy(
                    3,
                    &[
                        ErrorKind::Timeout,
                        ErrorKind::Disconnected,
                        ErrorKind::Unsupported(Operation::Write),
                    ],
                ),
            )
            .build()
            .expect("config"),
            vec![],
        );
        let request = WriteRequest::builder(rig.connection.limits())
            .item(tag("a", "holding-register:1"), PlcValue::I16(1))
            .build()
            .expect("write request");
        let error = error_of(block_on_with(&rig.clock, rig.connection.write(request)));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "kind"
        );
        assert_eq!(
            error.diagnostics().retries,
            0,
            "no retries, even for a listed kind"
        );
        assert!(rig.sent().is_empty(), "nothing sent");
        assert_eq!(rig.elapsed(), Duration::ZERO, "no time passed");
    }
}
