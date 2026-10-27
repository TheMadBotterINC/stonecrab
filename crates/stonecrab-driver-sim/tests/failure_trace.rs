//! The `tracing` events of the failure script name the step they consume
//! (`CODING_STANDARDS` 14.2).
//!
//! This file holds one test on purpose, for the reason given in `spans.rs`: a scoped
//! `tracing` dispatcher only behaves if it is the only test of its file under
//! `cargo test`.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use stonecrab_core::{
        Connection, ConnectionConfig, Driver, Endpoint, PlcValue, ProtocolStatus, ReadRequest, Tag,
        TagId,
    };
    use stonecrab_driver_sim::{FailureStep, SimAddress, SimDevice, SimDriver, SimTag};
    use stonecrab_testkit::{FakeClock, block_on_with};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Dispatch, Event, Level, Metadata, Subscriber};

    /// `(level, step field, message)` of every event that names a step.
    type Recorded = Vec<(Level, String, String)>;

    #[derive(Default)]
    struct Recorder {
        events: Mutex<Recorded>,
        next: AtomicU64,
    }

    #[derive(Default)]
    struct Fields {
        step: Option<String>,
        message: String,
    }

    impl Visit for Fields {
        fn record_str(&mut self, field: &Field, value: &str) {
            if field.name() == "step" {
                self.step = Some(value.to_owned());
            }
        }
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            if field.name() == "message" {
                self.message = format!("{value:?}");
            }
        }
    }

    impl Subscriber for Recorder {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed).saturating_add(1))
        }
        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut fields = Fields::default();
            event.record(&mut fields);
            if let Some(step) = fields.step {
                self.events
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push((*event.metadata().level(), step, fields.message));
            }
        }
        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    fn temperature() -> Tag {
        Tag::new(
            TagId::new("temperature").expect("identity"),
            Arc::new(SimAddress::parse("STATE/temperature:f32").expect("address")),
        )
    }

    // covers: ALPHA_PLAN S12 / tracing debug events name the step consumed
    #[test]
    fn every_consumed_step_is_named_in_a_debug_event() {
        let recorder = Arc::new(Recorder::default());
        let dispatch = Dispatch::new(Arc::clone(&recorder));
        tracing::dispatcher::with_default(&dispatch, || {
            let clock = Arc::new(FakeClock::default());
            let device = SimDevice::new();
            device
                .set("temperature", SimTag::new(PlcValue::F32(71.5)))
                .expect("temperature");
            // A connect meets the first step, which means nothing to it: a warning.
            device
                .push_failure(FailureStep::Backpressure)
                .expect("room in script");
            let driver = SimDriver::new()
                .with_clock(Arc::<FakeClock>::clone(&clock))
                .with_device("bench", device)
                .expect("device");
            let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
                .build()
                .expect("config");
            let connection: Box<dyn Connection> =
                block_on_with(&clock, driver.connect(config)).expect("connects");
            let bench = driver.device("bench").expect("bench device");
            bench
                .push_failures([
                    FailureStep::Delay(std::time::Duration::from_millis(1)),
                    FailureStep::Timeout,
                    FailureStep::DeviceException(ProtocolStatus::new("sim", 9)),
                    FailureStep::Malformed,
                    FailureStep::PartialFailure(Vec::new()),
                    FailureStep::Backpressure,
                    FailureStep::UnknownAfterTransmit,
                    FailureStep::Disconnect,
                ])
                .expect("room in script");
            for _ in 0..8 {
                let read = ReadRequest::builder(connection.limits())
                    .tag(temperature())
                    .build()
                    .expect("read");
                drop(block_on_with(&clock, connection.read(read)));
            }
        });
        let recorded = recorder
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let steps: Vec<(&Level, &str)> = recorded
            .iter()
            .map(|(level, step, _)| (level, step.as_str()))
            .collect();
        assert_eq!(
            steps,
            [
                (&Level::DEBUG, "backpressure"),
                (&Level::WARN, "backpressure"),
                (&Level::DEBUG, "delay"),
                (&Level::DEBUG, "timeout"),
                (&Level::DEBUG, "device-exception"),
                (&Level::DEBUG, "malformed"),
                (&Level::DEBUG, "partial-failure"),
                (&Level::DEBUG, "backpressure"),
                (&Level::DEBUG, "unknown-after-transmit"),
                (&Level::DEBUG, "disconnect"),
            ],
            "the debug event comes first, then (on a connect only) the warning"
        );
        assert!(
            recorded
                .iter()
                .any(|(level, _, message)| *level == Level::WARN && message.contains("ignored")),
            "the warning says the step was ignored"
        );
    }
}
