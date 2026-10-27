//! The `tracing` spans of the simulator connection (`CODING_STANDARDS` 14.2).
//!
//! This file holds one test on purpose. A scoped `tracing` dispatcher races with the
//! callsite-interest cache when other threads of the same process hit the same callsites
//! first, so under `cargo test` (threads, one process per test file) it only behaves if it
//! is the file's only test. Nextest runs every test in its own process either way.
#![cfg(all(feature = "sim", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, PoisonError};

    use stonecrab::sim::{SimAddress, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{FakeClock, block_on_with};
    use stonecrab::{Connection, DriverRegistry, PlcValue, ReadRequest, Tag, TagId, WriteRequest};
    use tracing::field::{Field, Visit};
    use tracing::span::{Attributes, Id, Record};
    use tracing::{Dispatch, Event, Metadata, Subscriber};

    type Recorded = Vec<(String, BTreeMap<String, String>)>;

    #[derive(Default)]
    struct Recorder {
        spans: Mutex<Recorded>,
        next: AtomicU64,
    }

    #[derive(Default)]
    struct Fields(BTreeMap<String, String>);

    impl Visit for Fields {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.insert(field.name().to_owned(), value.to_owned());
        }
        fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
            self.0.insert(field.name().to_owned(), format!("{value:?}"));
        }
    }

    impl Subscriber for Recorder {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, span: &Attributes<'_>) -> Id {
            let mut fields = Fields::default();
            span.record(&mut fields);
            self.spans
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((span.metadata().name().to_owned(), fields.0));
            Id::from_u64(self.next.fetch_add(1, Ordering::Relaxed).saturating_add(1))
        }
        fn record(&self, _span: &Id, _values: &Record<'_>) {}
        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
        fn event(&self, _event: &Event<'_>) {}
        fn enter(&self, _span: &Id) {}
        fn exit(&self, _span: &Id) {}
    }

    fn tag(id: &str, address: &str) -> Tag {
        Tag::new(
            TagId::new(id).expect("identity"),
            Arc::new(SimAddress::parse(address).expect("address")),
        )
    }

    // covers: CODING_STANDARDS 14.2 / spans carry scheme, endpoint, op and request_id
    #[test]
    fn every_operation_opens_a_span_with_a_monotonic_request_id() {
        let recorder = Arc::new(Recorder::default());
        let dispatch = Dispatch::new(Arc::clone(&recorder));
        tracing::dispatcher::with_default(&dispatch, || {
            let clock = Arc::new(FakeClock::default());
            let device = SimDevice::new();
            device
                .set("setpoint", SimTag::new(PlcValue::I16(20)))
                .expect("setpoint");
            let mut registry = DriverRegistry::new();
            registry
                .register(Arc::new(
                    SimDriver::new()
                        .with_clock(Arc::<FakeClock>::clone(&clock))
                        .with_device("bench", device)
                        .expect("device"),
                ))
                .expect("register");
            let connection: Box<dyn Connection> =
                block_on_with(&clock, registry.connect("sim://bench")).expect("connects");
            let setpoint = tag("setpoint", "STATE/setpoint:i16");
            let read = ReadRequest::builder(connection.limits())
                .tag(setpoint.clone())
                .build()
                .expect("read");
            drop(block_on_with(&clock, connection.read(read)).expect("read"));
            let write = WriteRequest::builder(connection.limits())
                .item(setpoint, PlcValue::I16(1))
                .build()
                .expect("write");
            drop(block_on_with(&clock, connection.write(write)).expect("write"));
            block_on_with(&clock, connection.close()).expect("close");
        });
        let recorded = recorder
            .spans
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let summary: Vec<(&str, &str, &str, &str, &str)> = recorded
            .iter()
            .map(|(name, fields)| {
                let field = |key: &str| fields.get(key).map_or("", String::as_str);
                (
                    name.as_str(),
                    field("scheme"),
                    field("endpoint"),
                    field("op"),
                    field("request_id"),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                ("connect", "sim", "bench", "connect", "0"),
                ("read", "sim", "bench", "read", "1"),
                ("write", "sim", "bench", "write", "2"),
                ("close", "sim", "bench", "close", "3"),
            ],
            "one span per operation, ids counting up per connection"
        );
    }
}
