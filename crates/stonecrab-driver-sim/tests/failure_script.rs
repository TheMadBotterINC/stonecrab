//! The failure script (slice S12): every `FailureStep` on read and on write, on connect
//! where it applies, with diagnostics and the retry interaction, all on the testkit's
//! `FakeClock` (`CODING_STANDARDS` 13.5: the simulator is the sanctioned fault injector).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::Poll;
    use std::time::Duration;

    use stonecrab_core::{
        AccessPolicy, CancellationToken, Clock, Connection, ConnectionConfig, ConnectionState,
        Deadline, Diagnostics, Driver, Endpoint, ErrorKind, FailureCategory, ItemError, OpFuture,
        Operation, PlcError, PlcValue, ProtocolStatus, ReadRequest, ReadResponse, Result,
        RetryPolicy, Tag, TagId, Timeouts, UnknownReason, WriteOutcome, WriteRequest,
        WriteResponse,
    };
    use stonecrab_driver_sim::{
        FailureScript, FailureStep, SimAddress, SimDevice, SimDriver, SimTag,
    };
    use stonecrab_testkit::{FakeClock, block_on_with, poll_once};

    const MS: Duration = Duration::from_millis(1);
    const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    /// A simulator on a fake clock with one device, `bench`.
    struct Bench {
        clock: Arc<FakeClock>,
        driver: Arc<SimDriver>,
        /// Counts the reads that reach the `temperature` tag.
        temperature_reads: Arc<AtomicU32>,
        /// Counts the reads that reach the `pressure` tag.
        pressure_reads: Arc<AtomicU32>,
    }

    impl Bench {
        fn new(latency: Duration) -> Self {
            let clock = Arc::new(FakeClock::default());
            let device = SimDevice::new().with_latency(latency);
            let (temperature, temperature_reads) =
                SimTag::new(PlcValue::F32(71.5)).with_read_counter();
            let (pressure, pressure_reads) = SimTag::new(PlcValue::F32(2.5)).with_read_counter();
            device.set("temperature", temperature).expect("temperature");
            device.set("pressure", pressure).expect("pressure");
            device
                .ramp("count", PlcValue::U16(0), PlcValue::U16(1))
                .expect("count");
            device
                .set("setpoint", SimTag::new(PlcValue::I16(20)))
                .expect("setpoint");
            device
                .set("spare", SimTag::new(PlcValue::I16(5)))
                .expect("spare");
            device
                .set("locked", SimTag::new(PlcValue::I16(5)).read_only())
                .expect("locked");
            let driver = Arc::new(
                SimDriver::new()
                    .with_clock(Arc::<FakeClock>::clone(&clock))
                    .with_device("bench", device)
                    .expect("device"),
            );
            Self {
                clock,
                driver,
                temperature_reads,
                pressure_reads,
            }
        }

        fn device(&self) -> Arc<SimDevice> {
            self.driver.device("bench").expect("bench device")
        }

        /// Push one step, panicking if the script is full.
        fn script(&self, steps: impl IntoIterator<Item = FailureStep>) {
            self.device().push_failures(steps).expect("room in script");
        }

        fn pending(&self) -> usize {
            self.device().pending_failures()
        }

        fn config(attempts: u32, retry_on: &[ErrorKind]) -> ConnectionConfig {
            let mut retry = RetryPolicy::default();
            retry.max_attempts = NonZeroU32::new(attempts).expect("positive");
            retry.retry_on = retry_on.iter().copied().collect();
            ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
                .retry(retry)
                .access(AccessPolicy::ReadWrite)
                .build()
                .expect("config")
        }

        fn connect(&self) -> Box<dyn Connection> {
            self.connect_with(Self::config(1, &[])).expect("connects")
        }

        fn connect_with(&self, config: ConnectionConfig) -> Result<Box<dyn Connection>> {
            block_on_with(&self.clock, self.driver.connect(config))
        }

        fn run<T>(&self, future: OpFuture<'_, T>) -> Result<T> {
            block_on_with(&self.clock, future)
        }

        fn read(&self, connection: &dyn Connection, ids: &[&str]) -> Result<ReadResponse> {
            self.run(connection.read(read_of(connection, ids)))
        }

        fn write(
            &self,
            connection: &dyn Connection,
            items: &[(&str, PlcValue)],
        ) -> Result<WriteResponse> {
            self.run(connection.write(write_of(connection, items)))
        }

        fn setpoint(&self) -> Option<PlcValue> {
            self.device().get("setpoint").map(|tag| tag.value)
        }

        fn temperature_reads(&self) -> u32 {
            self.temperature_reads.load(Ordering::Relaxed)
        }

        fn pressure_reads(&self) -> u32 {
            self.pressure_reads.load(Ordering::Relaxed)
        }

        fn deadline_in(&self, span: Duration) -> Deadline {
            Deadline::after(self.clock.now(), span).expect("deadline")
        }
    }

    fn tag(id: &str) -> Tag {
        let address = match id {
            "temperature" | "pressure" => format!("STATE/{id}:f32"),
            "count" => "STATE/count:u16".to_owned(),
            "log" => "SINK/log:i16".to_owned(),
            _ => format!("STATE/{id}:i16"),
        };
        Tag::new(
            TagId::new(id).expect("identity"),
            Arc::new(SimAddress::parse(&address).expect("address")),
        )
    }

    fn id(name: &str) -> TagId {
        TagId::new(name).expect("identity")
    }

    fn read_of(connection: &dyn Connection, ids: &[&str]) -> ReadRequest {
        read_builder(connection, ids).build().expect("valid read")
    }

    fn read_builder(
        connection: &dyn Connection,
        ids: &[&str],
    ) -> stonecrab_core::ReadRequestBuilder {
        ids.iter().fold(
            ReadRequest::builder(connection.limits()),
            |builder, name| builder.tag(tag(name)),
        )
    }

    fn write_of(connection: &dyn Connection, items: &[(&str, PlcValue)]) -> WriteRequest {
        write_builder(connection, items)
            .build()
            .expect("valid write")
    }

    fn write_builder(
        connection: &dyn Connection,
        items: &[(&str, PlcValue)],
    ) -> stonecrab_core::WriteRequestBuilder {
        items.iter().fold(
            WriteRequest::builder(connection.limits()),
            |builder, (name, value)| builder.item(tag(name), value.clone()),
        )
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

    fn item_of<'a>(response: &'a ReadResponse, name: &str) -> &'a stonecrab_core::ReadItem {
        response
            .get(&id(name))
            .expect("the item is in the response")
    }

    fn item_error<'a>(response: &'a ReadResponse, name: &str) -> &'a ItemError {
        item_of(response, name)
            .outcome()
            .as_ref()
            .expect_err("the item must fail")
    }

    fn value_of(response: &ReadResponse, name: &str) -> Option<PlcValue> {
        item_of(response, name).outcome().as_ref().ok().cloned()
    }

    fn rejection<'a>(response: &'a WriteResponse, name: &str) -> &'a ItemError {
        match response.get(&id(name)).expect("in the response").outcome() {
            WriteOutcome::Rejected(error) => error,
            outcome @ (WriteOutcome::Confirmed | WriteOutcome::Unknown(_) | _) => {
                panic!("expected a rejection for {name}, got {outcome:?}")
            }
        }
    }

    fn assert_item_diagnostics(error: &ItemError, operation: Operation) {
        let diagnostics = error.error().diagnostics();
        assert_eq!(diagnostics.operation, Some(operation), "item operation");
        assert_eq!(
            diagnostics.connection_state,
            Some(ConnectionState::Connected),
            "item state"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Protocol),
            "item category"
        );
        assert!(diagnostics.latency.is_some(), "item latency");
    }

    // ---- Delay ------------------------------------------------------------------------

    // covers: ALPHA_PLAN S12 / Delay waits on the clock, then proceeds with the device
    #[test]
    fn delay_waits_on_the_clock_then_serves_a_read_and_applies_a_write() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Delay(30 * MS)]);
        let start = bench.clock.now();
        let response = bench.read(&*connection, &["temperature"]).expect("reads");
        assert_eq!(
            value_of(&response, "temperature"),
            Some(PlcValue::F32(71.5)),
            "served normally"
        );
        assert_eq!(bench.clock.now() - start, 30 * MS, "the read waited");
        assert_eq!(bench.temperature_reads(), 1, "one device read");
        assert_eq!(bench.pending(), 0, "the step was consumed");

        bench.script([FailureStep::Delay(20 * MS)]);
        let start = bench.clock.now();
        let response = bench
            .write(&*connection, &[("setpoint", PlcValue::I16(77))])
            .expect("writes");
        assert_eq!(response.confirmed_count(), 1, "confirmed");
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(77)), "applied");
        assert_eq!(bench.clock.now() - start, 20 * MS, "the write waited");
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    #[test]
    fn delay_comes_before_the_device_latency() {
        let bench = Bench::new(5 * MS);
        let connection = bench.connect();
        bench.script([FailureStep::Delay(30 * MS)]);
        let start = bench.clock.now();
        drop(bench.read(&*connection, &["temperature"]).expect("reads"));
        assert_eq!(
            bench.clock.now() - start,
            35 * MS,
            "the scripted delay and then the device latency"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 timeouts (a delay past the deadline)
    #[test]
    fn a_delay_past_the_deadline_is_a_timeout_with_no_device_access() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Delay(10 * DEFAULT_TIMEOUT)]);
        let start = bench.clock.now();
        let request = read_builder(&*connection, &["temperature"])
            .deadline(bench.deadline_in(100 * MS))
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "read");
        assert_eq!(
            bench.clock.now() - start,
            100 * MS,
            "waited for the deadline"
        );
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
        assert_eq!(bench.temperature_reads(), 0, "the device was not read");
        assert_eq!(bench.pending(), 0, "the step was consumed");

        bench.script([FailureStep::Delay(10 * DEFAULT_TIMEOUT)]);
        let request = write_builder(&*connection, &[("setpoint", PlcValue::I16(1))])
            .deadline(bench.deadline_in(100 * MS))
            .build()
            .expect("request");
        let error = failure(bench.run(connection.write(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "write");
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (awaiting a response)
    #[test]
    fn cancelling_during_a_delay_is_cancelled_and_nothing_reaches_the_device() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Delay(Duration::from_secs(10))]);
        let start = bench.clock.now();
        let token = CancellationToken::new();
        let request = read_builder(&*connection, &["temperature"])
            .cancel(token.clone())
            .build()
            .expect("request");
        let mut waiting = connection.read(request);
        assert!(poll_once(&mut waiting).is_pending(), "the delay waits");
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("cancelling must wake the delay");
        };
        let error = failure(outcome);
        assert_eq!(error.kind(), ErrorKind::Cancelled, "read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Internal,
        );
        assert_eq!(bench.temperature_reads(), 0, "no device read");
        assert_eq!(bench.clock.now(), start, "no time passed");
        drop(waiting);
        drop(
            bench
                .read(&*connection, &["temperature"])
                .expect("the slot was released"),
        );

        bench.script([FailureStep::Delay(Duration::from_secs(10))]);
        let token = CancellationToken::new();
        let request = write_builder(&*connection, &[("setpoint", PlcValue::I16(1))])
            .cancel(token.clone())
            .build()
            .expect("request");
        let mut waiting = connection.write(request);
        assert!(poll_once(&mut waiting).is_pending(), "the delay waits");
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("cancelling must wake the delay");
        };
        assert_eq!(failure(outcome).kind(), ErrorKind::Cancelled, "write");
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
    }

    #[test]
    fn closing_during_a_delay_is_disconnected() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Delay(Duration::from_secs(10))]);
        let mut waiting = connection.read(read_of(&*connection, &["temperature"]));
        assert!(poll_once(&mut waiting).is_pending(), "the delay waits");
        bench.run(connection.close()).expect("closes");
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("closing must wake the delay");
        };
        assert_eq!(failure(outcome).kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(bench.temperature_reads(), 0, "no device read");
    }

    #[test]
    fn a_delay_too_long_to_add_up_is_a_typed_error() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Delay(Duration::MAX)]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(bench.temperature_reads(), 0, "no device read");
    }

    // ---- Timeout (timeout after transmit) --------------------------------------------

    // covers: ALPHA_PLAN S12 / Timeout touches the device, then loses the reply
    #[test]
    fn timeout_touches_the_device_and_then_waits_for_the_deadline() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Timeout]);
        let start = bench.clock.now();
        let error = failure(bench.read(&*connection, &["temperature", "count"]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "read");
        assert_eq!(
            bench.clock.now() - start,
            DEFAULT_TIMEOUT,
            "waited for timeouts.request"
        );
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
        assert_eq!(
            error.diagnostics().latency,
            Some(DEFAULT_TIMEOUT),
            "latency"
        );
        assert_eq!(error.diagnostics().retries, 0, "one attempt");
        assert_eq!(bench.temperature_reads(), 1, "the read reached the device");
        assert_eq!(connection.state(), ConnectionState::Connected, "still open");
        let next = bench.read(&*connection, &["count"]).expect("reads");
        assert_eq!(
            value_of(&next, "count"),
            Some(PlcValue::U16(1)),
            "the lost read stepped the ramp"
        );

        bench.script([FailureStep::Timeout]);
        let request = write_builder(&*connection, &[("setpoint", PlcValue::I16(88))])
            .deadline(bench.deadline_in(200 * MS))
            .build()
            .expect("request");
        let start = bench.clock.now();
        let error = failure(bench.run(connection.write(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "write");
        assert_eq!(bench.clock.now() - start, 200 * MS, "the explicit deadline");
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
        assert_eq!(
            bench.setpoint(),
            Some(PlcValue::I16(88)),
            "the write was applied before the reply was lost"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "still open");
    }

    #[test]
    fn cancelling_while_the_reply_is_lost_is_cancelled_but_the_device_was_touched() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Timeout]);
        let token = CancellationToken::new();
        let request = read_builder(&*connection, &["temperature"])
            .cancel(token.clone())
            .build()
            .expect("request");
        let mut waiting = connection.read(request);
        assert!(poll_once(&mut waiting).is_pending(), "waits for the reply");
        assert_eq!(bench.temperature_reads(), 1, "the device was read");
        token.cancel();
        let Poll::Ready(outcome) = poll_once(&mut waiting) else {
            panic!("cancelling must wake the wait");
        };
        assert_eq!(failure(outcome).kind(), ErrorKind::Cancelled, "kind");
    }

    // ---- read retry ---------------------------------------------------------------------

    // covers: SPEC 'Connection and resource behavior' / read retry bounded
    #[test]
    fn read_retry_counts_device_reads_up_to_max_attempts() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench
            .connect_with(Bench::config(3, &[ErrorKind::Timeout]))
            .expect("connects");
        bench.script([
            FailureStep::Timeout,
            FailureStep::Timeout,
            FailureStep::Timeout,
        ]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "the final result");
        assert_eq!(
            error.diagnostics().retries,
            2,
            "three attempts, two retries"
        );
        assert_eq!(
            bench.temperature_reads(),
            3,
            "the device was read once per attempt"
        );
        assert_eq!(bench.pending(), 0, "each attempt took its own step");

        // A script of one timeout and then nothing: the second attempt succeeds.
        bench.script([FailureStep::Timeout]);
        let before = bench.temperature_reads();
        let response = bench.read(&*connection, &["temperature"]).expect("retried");
        assert_eq!(
            value_of(&response, "temperature"),
            Some(PlcValue::F32(71.5)),
            "success on attempt 2"
        );
        assert_eq!(
            bench.temperature_reads() - before,
            2,
            "one lost read and one served read"
        );
    }

    #[test]
    fn a_script_longer_than_the_attempts_keeps_its_remaining_steps() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench
            .connect_with(Bench::config(3, &[ErrorKind::Timeout]))
            .expect("connects");
        bench.script(vec![FailureStep::Timeout; 4]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "result");
        assert_eq!(bench.temperature_reads(), 3, "max_attempts device reads");
        assert_eq!(bench.pending(), 1, "the fourth step is still queued");
    }

    #[test]
    fn the_default_policy_makes_one_attempt_per_read() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Timeout, FailureStep::Timeout]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.diagnostics().retries, 0, "no retry");
        assert_eq!(bench.temperature_reads(), 1, "one device read");
        assert_eq!(
            bench.pending(),
            1,
            "the second step waits for the next read"
        );
    }

    #[test]
    fn an_explicit_deadline_is_shared_by_every_attempt() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench
            .connect_with(Bench::config(3, &[ErrorKind::Timeout]))
            .expect("connects");
        bench.script(vec![FailureStep::Timeout; 3]);
        let request = read_builder(&*connection, &["temperature"])
            .deadline(bench.deadline_in(Duration::from_secs(60)))
            .build()
            .expect("request");
        let error = failure(bench.run(connection.read(request)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "result");
        assert_eq!(error.diagnostics().retries, 2, "attempts were made");
        assert_eq!(
            bench.temperature_reads(),
            1,
            "the first attempt used the whole deadline; the others failed at the gate"
        );
        assert_eq!(bench.pending(), 2, "and took no step");
    }

    #[test]
    fn a_retryable_scripted_failure_is_retried_and_a_device_exception_is_not() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench
            .connect_with(Bench::config(
                3,
                &[ErrorKind::Backpressure, ErrorKind::Disconnected],
            ))
            .expect("connects");
        bench.script([FailureStep::Backpressure]);
        let response = bench.read(&*connection, &["temperature"]).expect("retried");
        assert_eq!(response.items().len(), 1, "served on the second attempt");
        assert_eq!(bench.temperature_reads(), 1, "one device read");

        // A device exception is a per-item failure, not a failed call: nothing to retry.
        bench.script([
            FailureStep::DeviceException(ProtocolStatus::new("sim", 9)),
            FailureStep::Timeout,
        ]);
        let response = bench.read(&*connection, &["temperature"]).expect("answers");
        assert_eq!(
            item_error(&response, "temperature").kind(),
            ErrorKind::DeviceException,
            "refused"
        );
        assert_eq!(bench.pending(), 1, "no second attempt took the next step");

        // A scripted disconnect is retried, and the retry fails at the gate.
        bench.device().clear_failures();
        bench.script([FailureStep::Disconnect, FailureStep::Timeout]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(error.diagnostics().retries, 2, "attempts were made");
        assert_eq!(bench.pending(), 1, "the later attempts took no step");
    }

    // ---- Disconnect -----------------------------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F5 network outage (the connection fails for good)
    #[test]
    fn disconnect_fails_the_connection_and_every_later_operation() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let watch = connection.watch_state();

        bench.script([FailureStep::Disconnect]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Failed(ErrorKind::Disconnected),
            FailureCategory::Transport,
        );
        assert_eq!(
            connection.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "the connection failed"
        );
        assert_eq!(
            watch.current(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "observable through the watch"
        );
        assert_eq!(bench.temperature_reads(), 0, "the device was not touched");

        bench.script([FailureStep::Timeout]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "later read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Failed(ErrorKind::Disconnected),
            FailureCategory::Transport,
        );
        let error = failure(bench.write(&*connection, &[("setpoint", PlcValue::I16(1))]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "later write");
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
        assert_eq!(bench.temperature_reads(), 0, "still untouched");
        assert_eq!(bench.pending(), 1, "the later operations consumed no step");

        bench.run(connection.close()).expect("closes");
        assert_eq!(
            connection.state(),
            ConnectionState::Closed,
            "close still reaches Closed"
        );
    }

    #[test]
    fn disconnect_on_a_write_leaves_the_device_unchanged() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Disconnect]);
        let error = failure(bench.write(&*connection, &[("setpoint", PlcValue::I16(1))]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "write");
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Failed(ErrorKind::Disconnected),
            FailureCategory::Transport,
        );
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
    }

    // ---- DeviceException ------------------------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F3 device exception
    #[test]
    fn device_exception_fails_every_item_with_the_status_and_keeps_the_connection() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let status = ProtocolStatus::new("sim", 9).with_name("scripted exception");

        bench.script([FailureStep::DeviceException(status.clone())]);
        let response = bench
            .read(&*connection, &["temperature", "pressure"])
            .expect("the call succeeds; the items fail");
        assert_eq!(response.items().len(), 2, "every item is reported");
        for name in ["temperature", "pressure"] {
            let error = item_error(&response, name);
            assert_eq!(error.kind(), ErrorKind::DeviceException, "{name} kind");
            assert_eq!(error.status(), Some(&status), "{name} carries the status");
            assert_item_diagnostics(error, Operation::Read);
        }
        assert_eq!(
            (bench.temperature_reads(), bench.pressure_reads()),
            (0, 0),
            "the device was not touched"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");

        bench.script([FailureStep::DeviceException(status.clone())]);
        let response = bench
            .write(
                &*connection,
                &[("setpoint", PlcValue::I16(1)), ("log", PlcValue::I16(1))],
            )
            .expect("the call succeeds; the items are rejected");
        assert_eq!(response.rejected_count(), 2, "every item is rejected");
        for name in ["setpoint", "log"] {
            let error = rejection(&response, name);
            assert_eq!(error.kind(), ErrorKind::DeviceException, "{name} kind");
            assert_eq!(error.status(), Some(&status), "{name} carries the status");
            assert_item_diagnostics(error, Operation::Write);
        }
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
        assert_eq!(bench.device().sink_writes(), 0, "nothing counted");
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    // ---- Malformed -------------------------------------------------------------------------

    // covers: ALPHA_PLAN S12 / Malformed is per-item MalformedData on a read, whole-call on a write
    #[test]
    fn malformed_fails_each_read_item_and_the_whole_write() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Malformed]);
        let response = bench
            .read(&*connection, &["temperature", "pressure"])
            .expect("the call succeeds; the items fail");
        for name in ["temperature", "pressure"] {
            let error = item_error(&response, name);
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{name} kind");
            assert!(error.status().is_none(), "{name}: no device status");
            assert_item_diagnostics(error, Operation::Read);
        }
        assert_eq!(bench.temperature_reads(), 0, "the device was not touched");

        bench.script([FailureStep::Malformed]);
        let error = failure(bench.write(&*connection, &[("setpoint", PlcValue::I16(1))]));
        assert_eq!(error.kind(), ErrorKind::MalformedData, "write");
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Connected,
            FailureCategory::Protocol,
        );
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    // ---- PartialFailure ----------------------------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F3 mixed success within a batch
    // covers: SPEC mandatory-failure-cases / F6 timestamp absence
    #[test]
    fn partial_failure_fails_the_named_read_items_and_serves_the_rest() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::PartialFailure(vec![
            id("pressure"),
            id("not-in-the-request"),
        ])]);
        let sampled = bench.clock.now();
        let response = bench
            .read(&*connection, &["temperature", "pressure", "count"])
            .expect("the call succeeds");
        assert_eq!(
            value_of(&response, "temperature"),
            Some(PlcValue::F32(71.5)),
            "served"
        );
        assert_eq!(
            value_of(&response, "count"),
            Some(PlcValue::U16(0)),
            "served"
        );
        let error = item_error(&response, "pressure");
        assert_eq!(error.kind(), ErrorKind::DeviceException, "kind");
        assert_eq!(
            error.status(),
            Some(&ProtocolStatus::new("sim", 4)),
            "the partial-failure status"
        );
        assert_item_diagnostics(error, Operation::Read);
        assert_eq!(
            (bench.temperature_reads(), bench.pressure_reads()),
            (1, 0),
            "the failed item was not served"
        );
        for name in ["temperature", "pressure", "count"] {
            let item = item_of(&response, name);
            assert_eq!(item.sampled_at(), sampled, "{name}: sampled_at is present");
            assert_eq!(item.source_time(), None, "{name}: no source time");
        }
        assert_eq!(
            response
                .items()
                .iter()
                .map(|item| item.tag().as_str())
                .collect::<Vec<_>>(),
            ["temperature", "pressure", "count"],
            "request order is kept"
        );
    }

    #[test]
    fn partial_failure_rejects_the_named_write_items_and_applies_the_rest() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::PartialFailure(vec![id("spare")])]);
        let response = bench
            .write(
                &*connection,
                &[
                    ("setpoint", PlcValue::I16(41)),
                    ("spare", PlcValue::I16(42)),
                    ("log", PlcValue::I16(1)),
                ],
            )
            .expect("the call succeeds");
        assert_eq!(response.confirmed_count(), 2, "two confirmed");
        assert_eq!(response.rejected_count(), 1, "one rejected");
        let error = rejection(&response, "spare");
        assert_eq!(error.kind(), ErrorKind::DeviceException, "kind");
        assert_eq!(
            error.status(),
            Some(&ProtocolStatus::new("sim", 4)),
            "status"
        );
        assert_item_diagnostics(error, Operation::Write);
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(41)), "applied");
        assert_eq!(
            bench.device().get("spare").map(|tag| tag.value),
            Some(PlcValue::I16(5)),
            "the named item was not applied"
        );
        assert_eq!(bench.device().sink_writes(), 1, "the sink write counted");
    }

    #[test]
    fn a_partial_failure_naming_nothing_in_the_request_changes_nothing() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::PartialFailure(Vec::new())]);
        let response = bench.read(&*connection, &["temperature"]).expect("reads");
        assert_eq!(response.iter_failed().count(), 0, "nothing failed");
        assert_eq!(bench.pending(), 0, "the step was consumed");
    }

    // covers: SPEC mandatory-failure-cases / F3 unsupported capability (a SINK read)
    #[test]
    fn partial_failure_does_not_mask_the_devices_own_item_errors() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::PartialFailure(vec![id("temperature")])]);
        let response = bench
            .run(
                connection.read(
                    ReadRequest::builder(connection.limits())
                        .tag(tag("temperature"))
                        .tag(Tag::new(
                            id("sink"),
                            Arc::new(SimAddress::parse("SINK/sink:i16").expect("address")),
                        ))
                        .build()
                        .expect("request"),
                ),
            )
            .expect("the call succeeds");
        assert_eq!(
            item_error(&response, "temperature").status(),
            Some(&ProtocolStatus::new("sim", 4)),
            "scripted"
        );
        assert!(
            matches!(
                item_error(&response, "sink").kind(),
                ErrorKind::Unsupported(Operation::Read)
            ),
            "the device's own refusal of a write-only address stays"
        );
    }

    // ---- Backpressure ------------------------------------------------------------------------

    #[test]
    fn backpressure_fails_the_operation_and_releases_the_slot() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();

        bench.script([FailureStep::Backpressure]);
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Backpressure, "read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(bench.temperature_reads(), 0, "the device was not read");
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
        drop(
            bench
                .read(&*connection, &["temperature"])
                .expect("the slot was released"),
        );

        bench.script([FailureStep::Backpressure]);
        let error = failure(bench.write(&*connection, &[("setpoint", PlcValue::I16(1))]));
        assert_eq!(error.kind(), ErrorKind::Backpressure, "write");
        assert_diagnostics(
            &error,
            Operation::Write,
            ConnectionState::Connected,
            FailureCategory::Validation,
        );
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(20)), "not applied");
        drop(
            bench
                .write(&*connection, &[("setpoint", PlcValue::I16(2))])
                .expect("the slot was released"),
        );
    }

    // ---- UnknownAfterTransmit -----------------------------------------------------------------

    // covers: ALPHA_PLAN S12 / UnknownAfterTransmit yields Unknown(TimeoutAfterTransmit)
    #[test]
    fn unknown_after_transmit_write_is_never_replayed() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench
            .connect_with(Bench::config(
                3,
                &[ErrorKind::Timeout, ErrorKind::Disconnected],
            ))
            .expect("connects");
        // A retry in the policy and a second step queued behind: a replay would show.
        bench.script([FailureStep::UnknownAfterTransmit, FailureStep::Timeout]);
        let start = bench.clock.now();
        let response = bench
            .write(
                &*connection,
                &[
                    ("setpoint", PlcValue::I16(99)),
                    ("log", PlcValue::I16(1)),
                    ("locked", PlcValue::I16(7)),
                ],
            )
            .expect("the call succeeds with unknown outcomes");
        assert_eq!(response.unknown_count(), 3, "every item is unknown");
        for result in response.results() {
            assert!(
                matches!(
                    result.outcome(),
                    WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit)
                ),
                "{}: {:?}",
                result.tag().as_str(),
                result.outcome()
            );
        }
        assert_eq!(bench.setpoint(), Some(PlcValue::I16(99)), "applied once");
        assert_eq!(bench.device().sink_writes(), 1, "the sink saw one write");
        assert_eq!(
            bench.device().get("locked").map(|tag| tag.value),
            Some(PlcValue::I16(5)),
            "a refused item is unchanged, and still unknown"
        );
        assert_eq!(
            bench.pending(),
            1,
            "nothing was retried: the next step waits"
        );
        assert_eq!(
            bench.clock.now(),
            start,
            "the call did not wait for a deadline"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    #[test]
    fn unknown_after_transmit_on_a_read_behaves_like_timeout() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::UnknownAfterTransmit]);
        let start = bench.clock.now();
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(bench.clock.now() - start, DEFAULT_TIMEOUT, "waited");
        assert_eq!(bench.temperature_reads(), 1, "the device was read");
        assert_diagnostics(
            &error,
            Operation::Read,
            ConnectionState::Connected,
            FailureCategory::Transport,
        );
    }

    // ---- the script itself --------------------------------------------------------------------

    // covers: ALPHA_PLAN S12 / one step per operation, only when the device is reached
    #[test]
    fn a_step_is_taken_only_when_the_operation_reaches_the_device() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        let read_only = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .build()
            .expect("config");
        let guarded = bench.connect_with(read_only).expect("connects");
        bench.script([FailureStep::Disconnect]);

        // Rejected before any I/O: the step stays queued.
        let foreign = Tag::new(
            id("foreign"),
            Arc::new(stonecrab_testkit::FixtureAddress::new("fixture:0", None)),
        );
        let request = ReadRequest::builder(connection.limits())
            .tag(foreign)
            .build()
            .expect("request");
        assert_eq!(
            failure(bench.run(connection.read(request))).kind(),
            ErrorKind::InvalidAddress,
            "foreign address"
        );
        let token = CancellationToken::new();
        token.cancel();
        let request = read_builder(&*connection, &["temperature"])
            .cancel(token)
            .build()
            .expect("request");
        assert_eq!(
            failure(bench.run(connection.read(request))).kind(),
            ErrorKind::Cancelled,
            "cancelled token"
        );
        let request = read_builder(&*connection, &["temperature"])
            .deadline(Deadline::at(bench.clock.now()))
            .build()
            .expect("request");
        assert_eq!(
            failure(bench.run(connection.read(request))).kind(),
            ErrorKind::Timeout,
            "elapsed deadline"
        );
        assert_eq!(bench.pending(), 1, "no rejected operation took the step");

        // The in-flight slot: a second operation is refused while the first waits.
        let slow = Bench::new(Duration::from_secs(1));
        let slow_connection = slow.connect();
        let mut first = slow_connection.read(read_of(&*slow_connection, &["temperature"]));
        assert!(poll_once(&mut first).is_pending(), "the first read waits");
        slow.script([FailureStep::Malformed]);
        let error = failure(slow.read(&*slow_connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Backpressure, "the real slot");
        assert_eq!(slow.pending(), 1, "the refused read took no step");
        drop(first);

        // A read-only policy refuses the write first.
        let error = failure(bench.write(&*guarded, &[("setpoint", PlcValue::I16(1))]));
        assert_eq!(
            error.kind(),
            ErrorKind::Unsupported(Operation::Write),
            "policy"
        );
        assert_eq!(bench.pending(), 1, "the refused write took no step");

        // Reaching the device takes it.
        let error = failure(bench.read(&*connection, &["temperature"]));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "now it fires");
        assert_eq!(bench.pending(), 0, "and is consumed");
    }

    #[test]
    fn close_ignores_the_script() {
        let bench = Bench::new(Duration::ZERO);
        let connection = bench.connect();
        bench.script([FailureStep::Disconnect, FailureStep::Timeout]);
        bench.run(connection.close()).expect("closes");
        assert_eq!(connection.state(), ConnectionState::Closed, "closed");
        assert_eq!(bench.pending(), 2, "close took no step");
    }

    #[test]
    fn the_script_is_the_devices_and_connections_share_it_in_arrival_order() {
        let bench = Bench::new(Duration::ZERO);
        let first = bench.connect();
        let second = bench.connect();
        bench.script([FailureStep::Backpressure, FailureStep::Malformed]);
        let error = failure(bench.read(&*second, &["temperature"]));
        assert_eq!(
            error.kind(),
            ErrorKind::Backpressure,
            "first step, second connection"
        );
        let response = bench.read(&*first, &["temperature"]).expect("answers");
        assert_eq!(
            item_error(&response, "temperature").kind(),
            ErrorKind::MalformedData,
            "second step, first connection"
        );
        assert_eq!(bench.pending(), 0, "drained");
    }

    #[test]
    fn the_script_is_bounded_on_the_device() {
        let bench = Bench::new(Duration::ZERO);
        bench
            .device()
            .push_failures(vec![FailureStep::Timeout; FailureScript::MAX_STEPS])
            .expect("fits");
        let error = bench
            .device()
            .push_failure(FailureStep::Timeout)
            .expect_err("full");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "kind");
        bench.device().clear_failures();
        assert_eq!(bench.pending(), 0, "cleared");
    }

    // ---- connect ------------------------------------------------------------------------------

    fn connect_config(connect: Duration) -> ConnectionConfig {
        let mut timeouts = Timeouts::default();
        timeouts.connect = connect;
        ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .timeouts(timeouts)
            .access(AccessPolicy::ReadWrite)
            .build()
            .expect("config")
    }

    // covers: SPEC mandatory-failure-cases / F3 timeout (connecting; no connect token exists, so
    // cancel-while-connecting is replaced by the connect deadline and by dropping the future)
    #[test]
    fn a_connect_delay_past_the_connect_deadline_is_a_timeout() {
        let bench = Bench::new(Duration::ZERO);
        bench.script([FailureStep::Delay(Duration::from_secs(10))]);
        let start = bench.clock.now();
        let error = failure(bench.connect_with(connect_config(50 * MS)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            bench.clock.now() - start,
            50 * MS,
            "waited for the deadline"
        );
        assert_diagnostics(
            &error,
            Operation::Connect,
            ConnectionState::Connecting,
            FailureCategory::Transport,
        );
        assert_eq!(bench.pending(), 0, "the step was consumed");
    }

    #[test]
    fn a_connect_delay_within_the_deadline_only_delays() {
        let bench = Bench::new(5 * MS);
        bench.script([FailureStep::Delay(20 * MS)]);
        let start = bench.clock.now();
        let connection = bench
            .connect_with(connect_config(Duration::from_secs(1)))
            .expect("connects");
        assert_eq!(
            bench.clock.now() - start,
            25 * MS,
            "the delay and the latency"
        );
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");
    }

    #[test]
    fn dropping_a_connect_during_its_delay_leaves_the_script_consumed_and_the_device_alone() {
        let bench = Bench::new(Duration::ZERO);
        bench.script([FailureStep::Delay(Duration::from_secs(1))]);
        let mut connecting = bench.driver.connect(connect_config(Duration::from_secs(5)));
        assert!(poll_once(&mut connecting).is_pending(), "the connect waits");
        drop(connecting);
        assert_eq!(bench.pending(), 0, "the step is spent");
        bench.connect();
    }

    #[test]
    fn a_connect_timeout_step_fails_at_the_connect_deadline() {
        let bench = Bench::new(Duration::ZERO);
        bench.script([FailureStep::Timeout]);
        let start = bench.clock.now();
        let error = failure(bench.connect_with(connect_config(Duration::from_secs(2))));
        assert_eq!(error.kind(), ErrorKind::Timeout, "kind");
        assert_eq!(
            bench.clock.now() - start,
            Duration::from_secs(2),
            "at the connect deadline"
        );
        assert_diagnostics(
            &error,
            Operation::Connect,
            ConnectionState::Connecting,
            FailureCategory::Transport,
        );
    }

    #[test]
    fn a_connect_disconnect_step_fails_the_connect() {
        let bench = Bench::new(Duration::ZERO);
        bench.script([FailureStep::Disconnect]);
        let error = failure(bench.connect_with(Bench::config(1, &[])));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_diagnostics(
            &error,
            Operation::Connect,
            ConnectionState::Failed(ErrorKind::Disconnected),
            FailureCategory::Transport,
        );
        bench.connect();
    }

    #[test]
    fn steps_that_mean_nothing_to_a_connect_are_consumed_and_ignored() {
        let bench = Bench::new(Duration::ZERO);
        bench.script([
            FailureStep::DeviceException(ProtocolStatus::new("sim", 1)),
            FailureStep::Malformed,
            FailureStep::PartialFailure(vec![id("temperature")]),
            FailureStep::Backpressure,
            FailureStep::UnknownAfterTransmit,
        ]);
        for remaining in (0..5).rev() {
            let connection = bench.connect();
            assert_eq!(connection.state(), ConnectionState::Connected, "connects");
            assert_eq!(bench.pending(), remaining, "one step per connect");
        }
    }

    // ---- diagnostics --------------------------------------------------------------------------

    // covers: ALPHA_PLAN S12 / diagnostics populated on every scripted failure
    #[test]
    fn every_scripted_failure_carries_operation_state_latency_retries_and_category() {
        struct Case {
            name: &'static str,
            step: FailureStep,
            kind: ErrorKind,
            state: ConnectionState,
            category: FailureCategory,
        }
        let connected = ConnectionState::Connected;
        let failed = ConnectionState::Failed(ErrorKind::Disconnected);
        let cases = [
            Case {
                name: "timeout",
                step: FailureStep::Timeout,
                kind: ErrorKind::Timeout,
                state: connected,
                category: FailureCategory::Transport,
            },
            Case {
                name: "unknown-after-transmit",
                step: FailureStep::UnknownAfterTransmit,
                kind: ErrorKind::Timeout,
                state: connected,
                category: FailureCategory::Transport,
            },
            Case {
                name: "disconnect",
                step: FailureStep::Disconnect,
                kind: ErrorKind::Disconnected,
                state: failed,
                category: FailureCategory::Transport,
            },
            Case {
                name: "backpressure",
                step: FailureStep::Backpressure,
                kind: ErrorKind::Backpressure,
                state: connected,
                category: FailureCategory::Validation,
            },
        ];
        for case in cases {
            let bench = Bench::new(Duration::ZERO);
            let connection = bench.connect();
            bench.script([case.step.clone()]);
            let error = failure(bench.read(&*connection, &["temperature"]));
            assert_eq!(error.kind(), case.kind, "{} read kind", case.name);
            assert_diagnostics(&error, Operation::Read, case.state, case.category);
            assert_eq!(error.diagnostics().retries, 0, "{} retries", case.name);

            // A write that meets UnknownAfterTransmit succeeds with unknown outcomes (see
            // its own test); every other step fails the write as it failed the read.
            if case.name != "unknown-after-transmit" {
                let connection = bench.connect();
                bench.script([case.step]);
                let error = failure(bench.write(&*connection, &[("setpoint", PlcValue::I16(3))]));
                assert_eq!(error.kind(), case.kind, "{} write kind", case.name);
                assert_diagnostics(&error, Operation::Write, case.state, case.category);
                assert_eq!(error.diagnostics().retries, 0, "{} retries", case.name);
            }
        }
    }
}
