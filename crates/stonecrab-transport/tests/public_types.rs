//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab_transport::{FrameError, Jitter, SystemClock, Transport, TransportFactory};

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<FrameError>();
        assert_send_sync::<Jitter>();
        assert_send_sync::<SystemClock>();
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

    #[cfg(feature = "tcp")]
    #[test]
    fn tcp_types_are_send_sync_and_static() {
        use stonecrab_transport::{TcpFactory, TcpTransport};

        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<TcpFactory>();
        assert_send_sync::<TcpTransport>();
    }
}
