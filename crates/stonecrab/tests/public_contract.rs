//! Consumer-side checks of the public facade and thread-safety contracts.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    use stonecrab::{
        ByteOrder, CancellationToken, Capabilities, Clock, ConnectionState, Deadline, Diagnostics,
        ErrorKind, FailureCategory, ItemError, OpFuture, Operation, PlcError, PlcValue,
        PlcValueType, ProtocolAddress, ProtocolStatus, ReadItem, ReadRequest, ReadRequestBuilder,
        ReadResponse, ReadResponseBuilder, ResourceLimits, ResourceLimitsBuilder, Secret,
        StateCell, StateWatch, SubscriptionMode, Tag, TagId, UnknownReason, WordOrder,
        WriteOutcome,
    };

    #[test]
    fn public_foundation_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<ByteOrder>();
        assert_send_sync::<CancellationToken>();
        assert_send_sync::<Capabilities>();
        assert_send_sync::<ConnectionState>();
        assert_send_sync::<Deadline>();
        assert_send_sync::<Diagnostics>();
        assert_send_sync::<ErrorKind>();
        assert_send_sync::<FailureCategory>();
        assert_send_sync::<Operation>();
        assert_send_sync::<PlcError>();
        assert_send_sync::<PlcValue>();
        assert_send_sync::<PlcValueType>();
        assert_send_sync::<ResourceLimits>();
        assert_send_sync::<ResourceLimitsBuilder>();
        assert_send_sync::<Secret<String>>();
        assert_send_sync::<StateCell>();
        assert_send_sync::<StateWatch>();
        assert_send_sync::<SubscriptionMode>();
        assert_send_sync::<TagId>();
        assert_send_sync::<Tag>();
        assert_send_sync::<Box<dyn ProtocolAddress>>();
        assert_send_sync::<ProtocolStatus>();
        assert_send_sync::<ItemError>();
        assert_send_sync::<ReadItem>();
        assert_send_sync::<ReadRequest>();
        assert_send_sync::<ReadRequestBuilder>();
        assert_send_sync::<ReadResponse>();
        assert_send_sync::<ReadResponseBuilder>();
        assert_send_sync::<UnknownReason>();
        assert_send_sync::<WordOrder>();
        assert_send_sync::<WriteOutcome>();
    }

    #[test]
    fn op_future_is_send_and_clock_is_an_object_safe_send_sync_trait() {
        fn assert_send<T: Send>() {}
        fn assert_dyn_clock<T: Clock + ?Sized + Send + Sync>() {}
        assert_send::<OpFuture<'static, ()>>();
        assert_dyn_clock::<dyn Clock>();
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
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
}
