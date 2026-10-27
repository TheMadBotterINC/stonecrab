//! Consumer-side checks of the public facade and thread-safety contracts.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use stonecrab::{
        Capabilities, ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory,
        Operation, PlcError, PlcValue, PlcValueType, ResourceLimits, ResourceLimitsBuilder, Secret,
        SubscriptionMode, TagId,
    };

    #[test]
    fn public_foundation_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
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
        assert_send_sync::<SubscriptionMode>();
        assert_send_sync::<TagId>();
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
}
