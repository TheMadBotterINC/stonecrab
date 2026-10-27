//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::testkit::LengthPrefixedFramer;
    use stonecrab::transport::{
        ConnectionGuard, FrameError, Framer, GuardConfig, GuardStats, Jitter, SystemClock,
        Transport, TransportFactory,
    };

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<FrameError>();
        assert_send_sync::<Jitter>();
        assert_send_sync::<SystemClock>();
        assert_send_sync::<GuardConfig>();
        assert_send_sync::<GuardStats>();
    }

    #[test]
    fn the_connection_guard_is_send_sync_and_static_for_a_sendable_frame_and_framer() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<ConnectionGuard<Vec<u8>, LengthPrefixedFramer>>();
    }

    #[test]
    fn trait_objects_are_send_and_factories_are_shareable() {
        fn assert_send<T: Send + ?Sized>() {}
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send::<dyn Transport>();
        assert_send::<Box<dyn Transport>>();
        assert_send_sync::<dyn TransportFactory>();
        assert_send_sync::<Box<dyn TransportFactory>>();
    }

    #[test]
    fn the_framer_trait_is_object_safe_and_send() {
        fn assert_send<T: Send + ?Sized>() {}
        assert_send::<dyn Framer<Vec<u8>>>();
    }

    #[test]
    fn tcp_types_are_send_sync_and_static() {
        use stonecrab::transport::{TcpFactory, TcpTransport};

        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<TcpFactory>();
        assert_send_sync::<TcpTransport>();
    }
}
