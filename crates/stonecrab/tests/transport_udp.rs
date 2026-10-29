//! Datagram behavior of `UdpFactory` and `UdpTransport` over real loopback sockets (slice S61,
//! ADR-0027), and what `FramedTransport` does with datagram boundaries.
//!
//! Every socket binds `127.0.0.1:0`; nothing leaves the machine and nothing sleeps on real
//! time. Each call is bounded by a `Deadline` of a few seconds so a regression fails instead
//! of hanging; a closed port is noticed when the operating system's ICMP report arrives. A
//! receive that must time out runs on Tokio's paused clock, as the pending calls of
//! `transport_serial.rs` do: with nothing in flight and every task idle, the clock moves to
//! the deadline at once, and it is resumed before anything is sent again.
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::{Duration, Instant};

    use bytes::{Bytes, BytesMut};
    use stonecrab::testkit::LengthPrefixedFramer;
    use stonecrab::transport::{FramedTransport, Transport, TransportFactory, UdpFactory};
    use stonecrab::{Deadline, Endpoint, ErrorKind, Operation, PlcError, ResourceLimits, Result};
    use tokio::net::UdpSocket;

    fn deadline() -> Deadline {
        Deadline::after(Instant::now(), Duration::from_secs(5)).expect("representable")
    }

    fn limits(frame: usize, buffered: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(frame)
            .max_buffered_bytes(buffered)
            .build()
            .expect("valid limits")
    }

    fn endpoint(device: &UdpSocket) -> Endpoint {
        Endpoint::Custom(device.local_addr().expect("address").to_string())
    }

    /// An open transport, the device socket it is connected to, and the transport's own
    /// address as the device sees it (learnt from a first datagram, which is consumed).
    async fn connected(limits: ResourceLimits) -> (Box<dyn Transport>, UdpSocket, SocketAddr) {
        let device = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let mut transport = UdpFactory::default()
            .open(&endpoint(&device), limits, deadline())
            .await
            .expect("open");
        transport
            .send(Bytes::from_static(b"hi"), deadline())
            .await
            .expect("send");
        let mut first = [0_u8; 8];
        let (_, client) = device.recv_from(&mut first).await.expect("first datagram");
        (transport, device, client)
    }

    async fn expect_err<T>(result: impl Future<Output = Result<T>>) -> PlcError {
        result.await.err().expect("expected an error")
    }

    // covers: S61 / UTR-UDP a connected peer, one datagram each way
    #[tokio::test]
    async fn udp_round_trip_with_a_loopback_peer() {
        let device = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let peer = endpoint(&device);
        let limits = limits(260, 520);
        let mut transport = UdpFactory::default()
            .open(&peer, limits, deadline())
            .await
            .expect("open");
        assert_eq!(transport.peer(), &peer, "the endpoint it was opened for");
        assert_eq!(transport.limits(), &limits, "the limits it was opened with");
        assert!(transport.delivers_datagrams(), "a datagram transport");

        transport
            .send(Bytes::from_static(b"request"), deadline())
            .await
            .expect("send");
        let mut request = [0_u8; 16];
        let (length, client) = device.recv_from(&mut request).await.expect("recv");
        assert_eq!(request.get(..length), Some(&b"request"[..]), "one datagram");

        device.send_to(b"reply", client).await.expect("reply");
        let mut buffer = BytesMut::new();
        let read = transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!((read, &buffer[..]), (5, &b"reply"[..]));
    }

    // covers: S61 / UTR-UDP-RING one datagram per receive, boundaries kept
    #[tokio::test]
    async fn udp_datagram_boundaries_are_kept() {
        let (mut transport, device, client) = connected(limits(260, 520)).await;
        device.send_to(b"first", client).await.expect("send");
        device.send_to(b"second", client).await.expect("send");
        let mut buffer = BytesMut::new();
        let first = transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!((first, &buffer[..]), (5, &b"first"[..]), "only the first");
        let second = transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!(second, 6, "the second datagram, alone");
        assert_eq!(&buffer[..], b"firstsecond", "appended after the first");
    }

    // covers: S61 / UTR-UDP datagrams from another address are ignored
    #[tokio::test]
    async fn udp_datagram_from_another_peer_is_ignored() {
        let (mut transport, device, client) = connected(limits(260, 520)).await;
        let stranger = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        stranger
            .send_to(b"spoofed", client)
            .await
            .expect("the stranger sends");
        device.send_to(b"genuine", client).await.expect("send");
        let mut buffer = BytesMut::new();
        transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!(
            &buffer[..],
            b"genuine",
            "the stranger's datagram never arrives"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (buffer bound); S61 / UTR-UDP-RING
    #[tokio::test]
    async fn udp_oversized_datagram_is_a_resource_limit_not_a_silent_drop() {
        let (mut transport, device, client) = connected(limits(8, 8)).await;
        device.send_to(b"123456789", client).await.expect("send");
        device.send_to(b"12345678", client).await.expect("send");
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "nine bytes, eight free"
        );
        assert!(buffer.is_empty(), "nothing of it was appended");
        let read = transport
            .recv(&mut buffer, deadline())
            .await
            .expect("still usable");
        assert_eq!(
            (read, &buffer[..]),
            (8, &b"12345678"[..]),
            "an exact fit is whole"
        );

        // A full buffer reads nothing, and a smaller free space is the bound.
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "the buffer is full");
        let mut partial = BytesMut::from(&b"12345"[..]);
        device.send_to(b"abcd", client).await.expect("send");
        let error = expect_err(transport.recv(&mut partial, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "four bytes, three free"
        );
        assert_eq!(&partial[..], b"12345", "the buffer is as it was");
    }

    // covers: S61 / UTR-UDP the send bound
    #[tokio::test]
    async fn udp_send_rejects_a_payload_over_the_frame_limit() {
        let (mut transport, device, _client) = connected(limits(4, 8)).await;
        let error = expect_err(transport.send(Bytes::from_static(b"12345"), deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "five bytes, limit four"
        );
        transport
            .send(Bytes::from_static(b"1234"), deadline())
            .await
            .expect("an exact fit is sent and the transport is usable");
        let mut datagram = [0_u8; 8];
        let (length, _) = device.recv_from(&mut datagram).await.expect("recv");
        assert_eq!(
            datagram.get(..length),
            Some(&b"1234"[..]),
            "the refused one never left"
        );

        // The largest IPv4 UDP payload bounds a frame limit above it.
        let (mut transport, _device, _client) = connected(limits(70_000, 70_000)).await;
        let error = expect_err(transport.send(Bytes::from(vec![0_u8; 65_508]), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "over 65 507 bytes");
    }

    #[tokio::test]
    async fn an_empty_datagram_is_skipped_and_never_reads_as_a_close() {
        let (mut transport, device, client) = connected(limits(260, 520)).await;
        device.send_to(b"", client).await.expect("send");
        device.send_to(b"x", client).await.expect("send");
        let mut buffer = BytesMut::new();
        let read = transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!((read, &buffer[..]), (1, &b"x"[..]));
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (refused connect); S61 / ADR-0027
    #[tokio::test]
    async fn a_closed_port_is_disconnected_once_the_icmp_report_arrives() {
        let closed = {
            let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
            endpoint(&socket)
        };
        let mut transport = UdpFactory::default()
            .open(&closed, limits(260, 520), deadline())
            .await
            .expect("opening sends nothing, so it succeeds");
        transport
            .send(Bytes::from_static(b"anyone?"), deadline())
            .await
            .expect("the datagram leaves");
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "port unreachable: {error}"
        );
        let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "unusable afterwards");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[tokio::test]
    async fn a_receive_with_nothing_to_read_times_out_and_stays_usable() {
        let (mut transport, device, client) = connected(limits(260, 520)).await;
        let mut buffer = BytesMut::new();
        tokio::time::pause();
        let started = tokio::time::Instant::now();
        let short =
            Deadline::after(started.into_std(), Duration::from_millis(200)).expect("deadline");
        let error = expect_err(transport.recv(&mut buffer, short)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "nothing arrived");
        assert!(
            started.elapsed() >= Duration::from_millis(200),
            "not before the deadline"
        );
        let error = expect_err(transport.send(Bytes::from_static(b"x"), short)).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "an elapsed deadline sends nothing"
        );
        let error = expect_err(transport.recv(&mut buffer, short)).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "an elapsed deadline reads nothing"
        );
        tokio::time::resume();
        device.send_to(b"late", client).await.expect("send");
        transport
            .recv(&mut buffer, deadline())
            .await
            .expect("still usable");
        assert_eq!(&buffer[..], b"late", "the datagram was not lost");
    }

    #[tokio::test]
    async fn shutdown_closes_once_and_later_calls_are_disconnected() {
        let (mut transport, _device, _client) = connected(limits(260, 520)).await;
        let elapsed = Deadline::at(Instant::now());
        let error = expect_err(transport.shutdown(elapsed)).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "an elapsed deadline leaves it open"
        );
        transport.shutdown(deadline()).await.expect("shutdown");
        transport
            .shutdown(elapsed)
            .await
            .expect("a repeat is a no-op, even with an elapsed deadline");
        let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after shutdown");
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after shutdown");
    }

    // covers: S61 / UTR-UDP-OPTIONS the endpoint is host:port and nothing else
    #[tokio::test]
    async fn open_refuses_an_endpoint_that_is_not_host_and_port() {
        let factory = UdpFactory::default();
        let tcp = Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: 502,
        };
        for endpoint in [
            tcp,
            Endpoint::Custom("127.0.0.1".to_owned()),
            Endpoint::Custom("127.0.0.1:port".to_owned()),
            Endpoint::Custom("[::1]".to_owned()),
        ] {
            let error = expect_err(factory.open(&endpoint, limits(260, 520), deadline())).await;
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidConfig,
                "{endpoint}: {error}"
            );
            assert_eq!(
                error.diagnostics().operation,
                Some(Operation::Connect),
                "{endpoint}"
            );
        }
        let error = expect_err(factory.open(
            &Endpoint::Custom("127.0.0.1:502".to_owned()),
            limits(260, 520),
            Deadline::at(Instant::now()),
        ))
        .await;
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "an elapsed connect deadline"
        );
    }

    // covers: S61 / UTR-UDP-RING a datagram that ends inside a frame is never joined to the next
    #[tokio::test]
    async fn a_datagram_that_ends_inside_a_frame_is_dropped_not_joined_to_the_next() {
        let (transport, device, client) = connected(limits(64, 128)).await;
        let mut framed = FramedTransport::new(transport, LengthPrefixedFramer::new(64));
        // A prefix of four, then two bytes: the rest would have to come from the next one.
        device
            .send_to(&[0, 4, b'a', b'b'], client)
            .await
            .expect("send");
        device
            .send_to(&[0, 2, b'o', b'k'], client)
            .await
            .expect("send");
        let error = expect_err(framed.next_frame(deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::MalformedData,
            "a truncated datagram"
        );
        assert_eq!(framed.buffered_len(), 0, "its bytes were dropped");
        let frame = framed.next_frame(deadline()).await.expect("next datagram");
        assert_eq!(frame, b"ok", "the next datagram is a frame of its own");
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization; S61 / UTR-UDP-RING
    #[tokio::test]
    async fn resync_over_datagrams_drops_the_rest_of_a_datagram_without_reading() {
        let (transport, device, client) = connected(limits(64, 128)).await;
        let mut framed = FramedTransport::new(transport, LengthPrefixedFramer::new(64));
        // Two frames in one datagram, then a stray byte.
        device
            .send_to(&[0, 1, b'a', 0, 1, b'b', 0], client)
            .await
            .expect("send");
        assert_eq!(framed.next_frame(deadline()).await.expect("first"), b"a");
        assert_eq!(
            framed
                .next_frame(Deadline::at(Instant::now()))
                .await
                .expect("no I/O"),
            b"b",
            "the second frame of the datagram needs no read"
        );
        assert_eq!(framed.buffered_len(), 1, "the stray byte");
        framed
            .resync(Deadline::at(Instant::now()))
            .await
            .expect("drops without reading, so an elapsed deadline is fine");
        assert_eq!(framed.buffered_len(), 0, "dropped");
        device
            .send_to(&[0, 2, b'o', b'k'], client)
            .await
            .expect("send");
        assert_eq!(framed.next_frame(deadline()).await.expect("next"), b"ok");
    }
}
