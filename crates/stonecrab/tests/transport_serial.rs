//! `SerialFactory` and `SerialTransport` over a real pseudo-terminal pair.
//!
//! The pair comes from `serial2::SerialPort::pair` (see `support/serial_pair.rs`). It exists on
//! Unix and fails on macOS, where it applies the pair's settings with the baud-rate ioctl that
//! a macOS pseudo-terminal refuses with `ENOTTY`. The pair tests therefore run on Unix other
//! than macOS; on Windows and macOS they are skipped by name (`windows_has_no_virtual_pair`,
//! `macos_refuses_the_serial2_pair`), and the crate's unit tests, which drive the worker through
//! in-memory ports, still run, as do the configuration tests below. The transport end is handed
//! to `SerialTransport::from_port`, because `serial2` does not say where the slave end lives
//! (only Linux can name it, through `/proc/self/fd`); the tests that open by name through the
//! factory (`by_name`) therefore run on Linux only. No device is touched. Deadline expiry and
//! cancellation run on Tokio's paused clock while a call is pending (the clock advances only
//! when every task is idle, and a pending port call is idle), and each call otherwise carries
//! a generous deadline so a regression fails instead of hanging. The timing of the worker
//! itself is covered by the unit tests (ADR-0014 addendum S58).
#![cfg(feature = "serial")]
#![forbid(unsafe_code)]

#[cfg(all(unix, not(target_os = "macos"), test))]
#[path = "support/serial_pair.rs"]
mod serial_pair;

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stonecrab::transport::{SerialFactory, TransportFactory};
    use stonecrab::{Deadline, Endpoint, ErrorKind, Operation, PlcError, ResourceLimits, Result};

    /// A deadline on Tokio's clock, which is the plain clock unless a test has paused it.
    fn deadline() -> Deadline {
        Deadline::after(
            tokio::time::Instant::now().into_std(),
            Duration::from_secs(30),
        )
        .expect("representable")
    }

    fn limits(frame: usize, buffered: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(frame)
            .max_buffered_bytes(buffered)
            .build()
            .expect("valid limits")
    }

    async fn expect_err<T>(result: impl Future<Output = Result<T>>) -> PlcError {
        result.await.err().expect("expected an error")
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (invalid configuration)
    #[tokio::test]
    async fn configuration_errors_are_typed_and_nothing_is_opened() {
        let factory = SerialFactory::default();
        let mut no_baud = SerialFactory::default();
        no_baud.baud_rate = 0;
        let tcp = Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: 502,
        };
        let empty = Endpoint::Custom(String::new());
        let named = Endpoint::Custom("/dev/stonecrab-no-such-port".to_owned());

        for (name, error) in [
            (
                "a tcp endpoint",
                expect_err(factory.open(&tcp, limits(64, 128), deadline())).await,
            ),
            (
                "an empty port name",
                expect_err(factory.open(&empty, limits(64, 128), deadline())).await,
            ),
            (
                "a zero baud rate",
                expect_err(no_baud.open(&named, limits(64, 128), deadline())).await,
            ),
        ] {
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{name}");
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Connect),
                "{name}"
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_missing_device_is_disconnected_and_an_elapsed_deadline_is_a_timeout() {
        let missing = Endpoint::Custom("/dev/stonecrab-no-such-port".to_owned());
        let factory = SerialFactory::default();
        let error = expect_err(factory.open(&missing, limits(64, 128), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "no such device");
        assert!(error.is_retryable(), "a missing port may appear later");

        let elapsed = Deadline::at(std::time::Instant::now());
        let error = expect_err(factory.open(&missing, limits(64, 128), elapsed)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline already passed");
    }

    /// The tests that need a virtual pair.
    #[cfg(all(unix, not(target_os = "macos")))]
    mod pty {
        use bytes::{Bytes, BytesMut};
        use stonecrab::transport::{SerialTransport, Transport};

        use super::*;
        use crate::serial_pair::Pair;

        fn endpoint(pair: &Pair) -> Endpoint {
            Endpoint::Custom(pair.port_name().to_owned())
        }

        /// A transport over the transport end of `pair`, with limits of `frame` and twice that.
        fn attach_with(pair: &mut Pair, frame: usize) -> Box<dyn Transport> {
            let peer = endpoint(pair);
            let slave = pair.take_slave().expect("the slave is still there");
            let transport =
                SerialTransport::from_port(slave, peer, limits(frame, frame.saturating_mul(2)))
                    .expect("wrap the slave of a pseudo-terminal");
            Box::new(transport)
        }

        fn attach(pair: &mut Pair) -> Box<dyn Transport> {
            attach_with(pair, 64)
        }

        /// Receive until `buffer` holds at least `want` bytes.
        async fn recv_until(transport: &mut dyn Transport, buffer: &mut BytesMut, want: usize) {
            while buffer.len() < want {
                transport.recv(buffer, deadline()).await.expect("recv");
            }
        }

        /// Give up on a `recv` that is waiting for bytes, on a paused clock.
        async fn abandon_a_recv(transport: &mut dyn Transport, buffer: &mut BytesMut) {
            tokio::time::pause();
            let abandoned =
                tokio::time::timeout(Duration::from_secs(1), transport.recv(buffer, deadline()))
                    .await;
            tokio::time::resume();
            assert!(
                abandoned.is_err(),
                "the caller gave up while the read waited"
            );
            assert!(buffer.is_empty(), "a dropped recv appends nothing");
        }

        #[tokio::test]
        async fn bytes_travel_both_ways() {
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach(&mut pair);
            assert_eq!(
                transport.peer(),
                &endpoint(&pair),
                "peer is the endpoint as given"
            );
            assert_eq!(transport.limits(), &limits(64, 128), "limits as given");

            transport
                .send(Bytes::from_static(b"ping"), deadline())
                .await
                .expect("send");
            assert_eq!(
                pair.receive(4).expect("device read"),
                b"ping",
                "the device received the frame"
            );

            pair.send(b"pong").expect("the device writes");
            let mut reply = BytesMut::new();
            recv_until(&mut *transport, &mut reply, 4).await;
            assert_eq!(&reply[..], b"pong", "the transport received the reply");
            transport.shutdown(deadline()).await.expect("shutdown");
        }

        // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
        #[tokio::test]
        async fn a_recv_whose_deadline_passes_is_a_timeout_and_the_transport_stays_usable() {
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach(&mut pair);
            let mut buffer = BytesMut::new();

            tokio::time::pause();
            let error = expect_err(transport.recv(&mut buffer, deadline())).await;
            tokio::time::resume();
            assert_eq!(error.kind(), ErrorKind::Timeout, "nothing arrived in time");
            assert!(buffer.is_empty(), "a timed-out recv appends nothing");

            pair.send(b"late").expect("the device writes");
            recv_until(&mut *transport, &mut buffer, 4).await;
            assert_eq!(&buffer[..], b"late", "the transport was not poisoned");
            transport.shutdown(deadline()).await.expect("shutdown");
        }

        // covers: SPEC mandatory-failure-cases / F4 cancellation (a dropped recv loses nothing)
        #[tokio::test]
        async fn a_dropped_recv_loses_nothing() {
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach(&mut pair);
            let mut buffer = BytesMut::new();
            abandon_a_recv(&mut *transport, &mut buffer).await;

            pair.send(b"kept").expect("the device writes");
            recv_until(&mut *transport, &mut buffer, 4).await;
            assert_eq!(
                &buffer[..],
                b"kept",
                "bytes after the drop reach the next recv"
            );
            transport.shutdown(deadline()).await.expect("shutdown");
        }

        // covers: SPEC mandatory-failure-cases / F4 cancellation (close while a read waits)
        #[tokio::test]
        async fn shutdown_after_a_read_was_abandoned_joins_the_worker_and_releases_the_port() {
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach(&mut pair);
            let mut buffer = BytesMut::new();
            abandon_a_recv(&mut *transport, &mut buffer).await;

            transport
                .shutdown(deadline())
                .await
                .expect("shutdown joins the worker");
            assert!(pair.transport_end_closed(), "the worker dropped the port");
            transport
                .shutdown(deadline())
                .await
                .expect("a repeat shutdown is a no-op");
            let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
            assert_eq!(error.kind(), ErrorKind::Disconnected, "send after shutdown");
            let error = expect_err(transport.recv(&mut buffer, deadline())).await;
            assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after shutdown");
        }

        // covers: SPEC mandatory-failure-cases / F2 transport (peer close)
        #[tokio::test]
        async fn the_peer_closing_ends_the_stream_and_poisons_the_transport() {
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach(&mut pair);
            pair.close();

            let mut buffer = BytesMut::new();
            // A closed pseudo-terminal reads as an end of stream or as an I/O error (Linux
            // reports `EIO`), depending on the platform; the worker reports a hang-up as an
            // end of stream and passes any other error on as `Disconnected`, and the unit
            // tests pin each mapping.
            match transport.recv(&mut buffer, deadline()).await {
                Ok(0) => {}
                Err(error) => {
                    assert_eq!(error.kind(), ErrorKind::Disconnected, "read after close");
                }
                Ok(read) => panic!("read {read} bytes from a closed peer"),
            }
            let error = expect_err(transport.recv(&mut buffer, deadline())).await;
            assert_eq!(
                error.kind(),
                ErrorKind::Disconnected,
                "poisoned after the peer closed"
            );
            let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
            assert_eq!(
                error.kind(),
                ErrorKind::Disconnected,
                "poisoned for send too"
            );
            transport.shutdown(deadline()).await.expect("shutdown");
        }

        // covers: SPEC mandatory-failure-cases / F2 transport (a write into a stalled line)
        #[tokio::test]
        async fn a_send_into_a_line_that_stops_taking_bytes_ends_at_its_deadline_and_frees_the_worker()
         {
            const FRAME: usize = 4 * 1024 * 1024;
            let mut pair = Pair::new().expect("a pseudo-terminal pair");
            let mut transport = attach_with(&mut pair, FRAME);

            // The frame is far larger than any pseudo-terminal buffer and the device reads only
            // a few bytes of it, so the worker starts writing and then meets a line that takes
            // no more.
            let error = {
                let send = transport.send(Bytes::from(vec![0x5a; FRAME]), deadline());
                tokio::pin!(send);
                let mut started = false;
                for _ in 0..1000 {
                    tokio::select! {
                        biased;
                        done = &mut send => panic!("the frame cannot have been taken whole: {done:?}"),
                        () = tokio::task::yield_now() => {}
                    }
                    if !pair.receive(1).expect("device read").is_empty() {
                        started = true;
                        break;
                    }
                }
                assert!(started, "the worker began writing the frame");

                tokio::time::pause();
                let error = expect_err(send).await;
                tokio::time::resume();
                error
            };
            assert_eq!(
                error.kind(),
                ErrorKind::Timeout,
                "send ended at its deadline"
            );

            // The worker is not stuck in the write: it leaves within a slice of the caller
            // going, so closing joins it well inside this deadline.
            transport
                .shutdown(deadline())
                .await
                .expect("the worker left the stalled write and was joined");
        }

        /// Opening by name sets a baud rate and needs the path of the slave, so these run on
        /// Linux only.
        #[cfg(target_os = "linux")]
        mod by_name {
            use stonecrab::transport::{DataBits, FlowControl, Parity, StopBits};

            use super::*;

            #[tokio::test]
            async fn the_factory_opens_a_port_by_name_with_every_line_setting() {
                let pair = Pair::new().expect("a pseudo-terminal pair");
                let mut factory = SerialFactory::default();
                factory.baud_rate = 19_200;
                factory.data_bits = DataBits::Seven;
                factory.parity = Parity::Even;
                factory.stop_bits = StopBits::Two;
                factory.flow_control = FlowControl::XonXoff;
                let mut transport = factory
                    .open(&endpoint(&pair), limits(64, 128), deadline())
                    .await
                    .expect("open the slave by name");

                transport
                    .send(Bytes::from_static(b"by name"), deadline())
                    .await
                    .expect("send");
                assert_eq!(
                    pair.receive(7).expect("device read"),
                    b"by name",
                    "bytes reach the device"
                );
                pair.send(b"ok").expect("the device writes");
                let mut reply = BytesMut::new();
                recv_until(&mut *transport, &mut reply, 2).await;
                assert_eq!(&reply[..], b"ok", "bytes reach the transport");
                transport.shutdown(deadline()).await.expect("shutdown");
            }
        }
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "windows_has_no_virtual_pair: the virtual serial pair is a pseudo-terminal \
                (serial2::SerialPort::pair), which exists on Unix only"]
    fn windows_has_no_virtual_pair() {}

    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "macos_refuses_the_serial2_pair: serial2::SerialPort::pair applies its settings \
                with the baud-rate ioctl, which a macOS pseudo-terminal refuses with ENOTTY"]
    fn macos_refuses_the_serial2_pair() {}
}
