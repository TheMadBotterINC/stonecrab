//! Consumer-side checks of the public facade and thread-safety contracts.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use stonecrab::{
        AccessPolicy, AddressOptions, AddressOptionsBuilder, ByteOrder, CancellationToken,
        Capabilities, Clock, Connection, ConnectionConfig, ConnectionConfigBuilder,
        ConnectionState, ConnectionUri, Credentials, Deadline, Diagnostics, Driver, DriverOptions,
        DriverRegistry, Endpoint, ErrorKind, FailureCategory, ItemError, LossCounters, OpFuture,
        Operation, PlcError, PlcValue, PlcValueType, ProtocolAddress, ProtocolStatus, QueuePolicy,
        ReadItem, ReadRequest, ReadRequestBuilder, ReadResponse, ReadResponseBuilder,
        ReconnectPolicy, ResourceLimits, ResourceLimitsBuilder, RetryPolicy, Secret, StateCell,
        StateWatch, SubscribeRequest, SubscribeRequestBuilder, Subscription, SubscriptionKind,
        SubscriptionMode, SubscriptionSender, Tag, TagId, Timeouts, UnknownReason, WordOrder,
        WriteItem, WriteOutcome, WriteRequest, WriteRequestBuilder, WriteResponse,
        WriteResponseBuilder, WriteResult,
    };

    #[test]
    fn public_foundation_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<AccessPolicy>();
        assert_send_sync::<AddressOptions>();
        assert_send_sync::<AddressOptionsBuilder>();
        assert_send_sync::<ByteOrder>();
        assert_send_sync::<CancellationToken>();
        assert_send_sync::<Capabilities>();
        assert_send_sync::<ConnectionConfig>();
        assert_send_sync::<ConnectionConfigBuilder>();
        assert_send_sync::<Credentials>();
        assert_send_sync::<Endpoint>();
        assert_send_sync::<ConnectionState>();
        assert_send_sync::<ConnectionUri>();
        assert_send_sync::<Deadline>();
        assert_send_sync::<Diagnostics>();
        assert_send_sync::<DriverRegistry>();
        assert_send_sync::<ErrorKind>();
        assert_send_sync::<FailureCategory>();
        assert_send_sync::<Operation>();
        assert_send_sync::<PlcError>();
        assert_send_sync::<PlcValue>();
        assert_send_sync::<PlcValueType>();
        assert_send_sync::<ReconnectPolicy>();
        assert_send_sync::<ResourceLimits>();
        assert_send_sync::<ResourceLimitsBuilder>();
        assert_send_sync::<RetryPolicy>();
        assert_send_sync::<Secret<String>>();
        assert_send_sync::<StateCell>();
        assert_send_sync::<StateWatch>();
        assert_send_sync::<SubscriptionMode>();
        assert_send_sync::<SubscriptionKind>();
        assert_send_sync::<QueuePolicy>();
        assert_send_sync::<SubscribeRequest>();
        assert_send_sync::<SubscribeRequestBuilder>();
        assert_send_sync::<Subscription>();
        assert_send_sync::<SubscriptionSender>();
        assert_send_sync::<LossCounters>();
        assert_send_sync::<TagId>();
        assert_send_sync::<Timeouts>();
        assert_send_sync::<Tag>();
        assert_send_sync::<Box<dyn ProtocolAddress>>();
        assert_send_sync::<Box<dyn DriverOptions>>();
        assert_send_sync::<ProtocolStatus>();
        assert_send_sync::<ItemError>();
        assert_send_sync::<ReadItem>();
        assert_send_sync::<ReadRequest>();
        assert_send_sync::<ReadRequestBuilder>();
        assert_send_sync::<ReadResponse>();
        assert_send_sync::<ReadResponseBuilder>();
        assert_send_sync::<UnknownReason>();
        assert_send_sync::<WordOrder>();
        assert_send_sync::<WriteItem>();
        assert_send_sync::<WriteOutcome>();
        assert_send_sync::<WriteRequest>();
        assert_send_sync::<WriteRequestBuilder>();
        assert_send_sync::<WriteResponse>();
        assert_send_sync::<WriteResponseBuilder>();
        assert_send_sync::<WriteResult>();
    }

    #[test]
    fn op_future_is_send_and_clock_is_an_object_safe_send_sync_trait() {
        fn assert_send<T: Send>() {}
        fn assert_dyn_clock<T: Clock + ?Sized + Send + Sync>() {}
        assert_send::<OpFuture<'static, ()>>();
        assert_dyn_clock::<dyn Clock>();
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    // covers: ALPHA_PLAN S11a / object safety of Driver and Connection (13.5: the traits
    // are not implemented here; the simulator exercises them)
    #[test]
    fn driver_and_connection_are_object_safe_send_sync_traits() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        fn assert_send<T: Send>() {}
        // A function taking a trait object only compiles if the trait is object safe.
        fn take_driver(_driver: &dyn Driver) {}
        fn take_connection(_connection: &dyn Connection) {}
        assert_send_sync::<dyn Driver>();
        assert_send_sync::<dyn Connection>();
        assert_send_sync::<Arc<dyn Driver>>();
        assert_send_sync::<Box<dyn Connection>>();
        assert_send::<OpFuture<'static, Box<dyn Connection>>>();
        assert_send::<OpFuture<'static, Subscription>>();
        std::hint::black_box((take_driver, take_connection));
    }

    #[test]
    fn facade_token_and_watch_can_be_polled_without_a_runtime() {
        let mut cx = Context::from_waker(Waker::noop());

        let token = CancellationToken::new();
        let child = token.child();
        let mut cancelled = child.cancelled();
        assert!(
            cancelled.as_mut().poll(&mut cx).is_pending(),
            "live token pends"
        );
        token.cancel();
        assert!(
            matches!(cancelled.as_mut().poll(&mut cx), Poll::Ready(Ok(()))),
            "child resolves after the parent is cancelled"
        );

        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let mut changed = watch.changed();
        assert!(changed.as_mut().poll(&mut cx).is_pending(), "no change yet");
        cell.set(ConnectionState::Connected);
        assert!(
            matches!(
                changed.as_mut().poll(&mut cx),
                Poll::Ready(Ok(ConnectionState::Connected))
            ),
            "watch reports the new state"
        );
    }

    // covers: SPEC mandatory-failure-cases / F6 slow consumer (the queue needs no runtime)
    #[test]
    fn facade_subscription_queue_can_be_polled_without_a_runtime() {
        let mut cx = Context::from_waker(Waker::noop());
        let limits = ResourceLimits::builder()
            .max_queued_events(1)
            .build()
            .expect("valid budgets");
        let (sender, subscription) =
            SubscriptionSender::polling(Duration::from_secs(1), QueuePolicy::DropOldest, limits);
        let tag = TagId::new("level").expect("valid identity");
        for value in [1, 2] {
            let item = ReadItem::new(tag.clone(), Ok(PlcValue::U16(value)), Instant::now());
            assert!(
                sender.send(item).as_mut().poll(&mut cx).is_ready(),
                "dropping never waits"
            );
        }
        assert_eq!(subscription.loss().dropped, 1, "one event made room");
        let mut next = subscription.recv();
        assert!(
            matches!(
                next.as_mut().poll(&mut cx),
                Poll::Ready(Ok(Some(ref item))) if item.outcome().as_ref().ok() == Some(&PlcValue::U16(2))
            ),
            "the newest event stayed"
        );
        drop(next);
        assert_eq!(subscription.mode(), SubscriptionMode::Polling, "disclosed");
        assert_eq!(
            subscription.interval(),
            Some(Duration::from_secs(1)),
            "interval"
        );
        assert_eq!(subscription.unsubscribe().dropped, 1, "final counts");
        assert!(sender.is_closed(), "the producer sees the end");
    }

    #[test]
    fn facade_exposes_foundation_without_runtime_or_network_setup() {
        let tag = TagId::new("energy_counter").expect("valid application identifier");
        let reading = PlcValue::U64(u64::MAX);
        let now = Instant::now();
        let deadline = Deadline::after(now, Duration::from_secs(1)).expect("valid duration");
        let mut capabilities = Capabilities::default();
        capabilities.read = true;
        assert!(
            capabilities.supports(Operation::Read),
            "caller declared read support"
        );
        assert_eq!(
            tag.as_str(),
            "energy_counter",
            "correlation identity stable"
        );
        assert_eq!(
            reading.as_u64(),
            Some(u64::MAX),
            "large counters remain exact"
        );
        assert_eq!(reading.as_f64(), None, "no silent precision loss");
        assert_eq!(
            deadline.remaining(now),
            Some(Duration::from_secs(1)),
            "explicit clock reading"
        );
    }

    #[test]
    fn facade_reexports_ordering_types_and_additive_ord() {
        use std::collections::BTreeSet;

        assert!(
            ByteOrder::BigEndian < ByteOrder::LittleEndian,
            "byte order is Ord"
        );
        assert!(
            WordOrder::HighWordFirst < WordOrder::LowWordFirst,
            "word order is Ord"
        );
        let retry: BTreeSet<ErrorKind> = [ErrorKind::Timeout, ErrorKind::Disconnected]
            .into_iter()
            .collect();
        assert!(retry.contains(&ErrorKind::Timeout), "kinds fit a BTreeSet");
        assert!(Operation::Read < Operation::Write, "operations are Ord");
        assert!(PlcValueType::I16 < PlcValueType::U16, "value types are Ord");
    }

    #[test]
    fn facade_builds_a_configuration_and_extracts_address_options() {
        let config = ConnectionConfig::builder(
            "modbus-tcp",
            Endpoint::Tcp {
                host: "192.0.2.10".to_owned(),
                port: 502,
            },
        )
        .credentials(Credentials::new("operator", "example-only"))
        .address_convention("plc4x")
        .build()
        .expect("valid configuration");
        let options = config.address_options();
        assert_eq!(options.convention(), Some("plc4x"), "carrier extracted");
        assert_eq!(config.access(), AccessPolicy::ReadOnly, "read-only default");
        assert!(
            !config.redacted_display().contains("example-only"),
            "display is redacted"
        );
        assert_eq!(
            config.reconnect().next_delay(1, 0.5),
            Duration::from_millis(500),
            "pure backoff is reachable through the facade"
        );
    }
}
