//! Byte-level behavior of `TcpFactory` and `TcpTransport` over real loopback sockets.
//!
//! Every listener binds `127.0.0.1:0`; nothing leaves the machine and nothing sleeps on
//! real time. Each test is bounded by a `Deadline` of a few seconds so a regression fails
//! instead of hanging. Timing of deadline expiry is covered on paused time in the crate's
//! unit tests, not here (ADR-0014).
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bytes::{Bytes, BytesMut};
    use stonecrab::testkit::{Script, ScriptedPeer, Step, poll_once};
    use stonecrab::transport::{TcpFactory, Transport, TransportFactory};
    use stonecrab::{Deadline, Endpoint, ErrorKind, Operation, PlcError, ResourceLimits, Result};
    use tokio::io::AsyncReadExt;
    use tokio::net::{TcpListener, TcpStream};
    use tokio::task::JoinSet;

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

    fn endpoint(listener: &TcpListener) -> Endpoint {
        Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: listener.local_addr().expect("address").port(),
        }
    }

    /// A connected transport and the accepted peer socket ("the device").
    async fn connected(limits: ResourceLimits) -> (Box<dyn Transport>, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let transport = TcpFactory::default()
            .open(&endpoint(&listener), limits, deadline())
            .await
            .expect("open against a listening socket");
        let (device, _) = listener.accept().await.expect("accept");
        (transport, device)
    }

    /// A transport opened against a `ScriptedPeer` that runs `script` on the connection.
    async fn connected_to_script(
        limits: ResourceLimits,
        script: Script,
    ) -> (Box<dyn Transport>, ScriptedPeer) {
        let peer = ScriptedPeer::spawn(script).expect("spawn the scripted peer");
        let transport = TcpFactory::default()
            .open(&peer.endpoint(), limits, deadline())
            .await
            .expect("open against the scripted peer");
        (transport, peer)
    }

    /// Receive until `buffer` holds at least `want` bytes; returns the size of each read.
    async fn recv_until(
        transport: &mut dyn Transport,
        buffer: &mut BytesMut,
        want: usize,
    ) -> Result<Vec<usize>> {
        let mut reads = Vec::new();
        while buffer.len() < want {
            reads.push(transport.recv(buffer, deadline()).await?);
        }
        Ok(reads)
    }

    async fn expect_err<T>(result: impl Future<Output = Result<T>>) -> PlcError {
        result.await.err().expect("expected an error")
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (refused connect)
    #[tokio::test]
    async fn connecting_to_a_closed_loopback_port_is_disconnected_before_the_deadline() {
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            endpoint(&listener)
        };
        let limit = deadline();
        let error = expect_err(TcpFactory::default().open(&closed, limits(64, 128), limit)).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "refused connection");
        assert!(error.is_retryable(), "a refused connect may be retried");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "connect diagnostics"
        );
        assert!(
            !limit.is_elapsed(Instant::now()),
            "failed before the deadline"
        );
    }

    #[tokio::test]
    async fn the_opened_transport_reports_its_peer_and_limits() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let target = endpoint(&listener);
        let wanted = limits(64, 128);
        let transport = TcpFactory::default()
            .open(&target, wanted, deadline())
            .await
            .expect("open");
        assert_eq!(transport.peer(), &target, "peer is the endpoint as given");
        assert_eq!(
            transport.limits(),
            &wanted,
            "limits are the ones passed to open"
        );
    }

    #[tokio::test]
    async fn a_custom_endpoint_is_rejected_as_invalid_config() {
        let custom = Endpoint::Custom("sim://bench".to_owned());
        let error =
            expect_err(TcpFactory::default().open(&custom, limits(64, 128), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "not a tcp endpoint");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "connect diagnostics"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (fragmented delivery);
    // TRANSPORT-STREAM-DELIVERY
    #[tokio::test]
    async fn a_frame_written_in_fragments_accumulates_across_recv_calls() {
        // The peer waits for a "next" byte before each fragment, so every fragment is
        // read before the following one is written.
        let script = Script::strict(vec![
            Step::Expect(b"go".to_vec()),
            Step::ReplyFragmented(vec![b"012".to_vec()]),
            Step::Expect(b"n".to_vec()),
            Step::ReplyFragmented(vec![b"345".to_vec()]),
            Step::Expect(b"n".to_vec()),
            Step::ReplyFragmented(vec![b"6789".to_vec()]),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        let frame = b"0123456789";
        let mut buffer = BytesMut::new();
        let mut received = 0;
        for (index, size) in [3_usize, 3, 4].into_iter().enumerate() {
            let request: &'static [u8] = if index == 0 { b"go" } else { b"n" };
            transport
                .send(Bytes::from_static(request), deadline())
                .await
                .expect("send");
            received += size;
            let reads = recv_until(transport.as_mut(), &mut buffer, received)
                .await
                .expect("recv");
            assert_eq!(
                reads.iter().sum::<usize>(),
                size,
                "bytes added by this fragment"
            );
            assert_eq!(buffer.len(), received, "buffer holds everything so far");
        }
        assert_eq!(
            &buffer[..],
            frame,
            "fragments joined into the original frame"
        );
        let report = peer.close().await.expect("close the peer");
        assert!(
            report.mismatches.is_empty(),
            "the peer saw what it expected"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (coalesced delivery)
    #[tokio::test]
    async fn two_frames_written_together_arrive_in_one_recv() {
        let script = Script::strict(vec![
            Step::Expect(b"go".to_vec()),
            Step::ReplyCoalescedWithNext,
            Step::Reply(b"first-frame|".to_vec()),
            Step::Reply(b"second-frame".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        transport
            .send(Bytes::from_static(b"go"), deadline())
            .await
            .expect("send");
        let mut buffer = BytesMut::new();
        let read = transport.recv(&mut buffer, deadline()).await.expect("recv");
        assert_eq!(read, 24, "a single recv returned both frames");
        assert_eq!(
            &buffer[..],
            b"first-frame|second-frame",
            "both frames present"
        );
        peer.close().await.expect("close the peer");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (peer close mid-frame)
    #[tokio::test]
    async fn peer_close_mid_frame_reads_zero_then_every_call_is_disconnected() {
        let script = Script::new(vec![
            Step::CloseAt(10),
            Step::Reply(b"half a frame, cut short".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        let mut buffer = BytesMut::new();
        recv_until(transport.as_mut(), &mut buffer, 10)
            .await
            .expect("partial bytes arrive");
        assert_eq!(
            &buffer[..],
            b"half a fra",
            "the partial frame was delivered"
        );

        let closed = transport
            .recv(&mut buffer, deadline())
            .await
            .expect("end of stream");
        assert_eq!(closed, 0, "0 means the peer closed");
        assert_eq!(buffer.len(), 10, "nothing appended at end of stream");

        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after close");
        let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after close");
        let report = peer.close().await.expect("close the peer");
        assert!(report.closed, "the script closed the connection");
    }

    #[tokio::test]
    async fn a_connection_reset_surfaces_as_disconnected() {
        let (mut transport, device) = connected(limits(64, 128)).await;
        device.set_zero_linger().expect("linger 0");
        drop(device);
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "reset while reading");
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "unusable after a failed read"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (buffer bound)
    #[tokio::test]
    async fn recv_refuses_a_full_buffer_without_reading() {
        let script = Script::new(vec![Step::Reply(b"ABCD".to_vec())]);
        let (mut transport, peer) = connected_to_script(limits(4, 8), script).await;
        let mut full = BytesMut::from(&b"12345678"[..]);
        let error = expect_err(transport.recv(&mut full, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "buffer at the bound"
        );
        assert_eq!(&full[..], b"12345678", "buffer untouched");

        let mut over = BytesMut::from(&b"123456789"[..]);
        let error = expect_err(transport.recv(&mut over, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "buffer above the bound"
        );

        let mut empty = BytesMut::new();
        let read = transport.recv(&mut empty, deadline()).await.expect("recv");
        assert_eq!(
            (read, &empty[..]),
            (4, &b"ABCD"[..]),
            "the refused calls consumed nothing"
        );
        peer.close().await.expect("close the peer");
    }

    #[tokio::test]
    async fn recv_reads_no_more_than_the_free_space() {
        let script = Script::new(vec![Step::Reply(b"ABCDEFGHIJKL".to_vec())]);
        let (mut transport, peer) = connected_to_script(limits(4, 8), script).await;
        let mut buffer = BytesMut::from(&b"12345"[..]);
        let mut total = 0;
        while total < 3 {
            total += transport.recv(&mut buffer, deadline()).await.expect("recv");
        }
        assert_eq!(total, 3, "only the free 3 bytes were read");
        assert_eq!(buffer.len(), 8, "buffer is exactly at the bound");
        assert_eq!(buffer.get(..5), Some(&b"12345"[..]), "existing bytes kept");
        buffer.clear();
        let mut rest = 0;
        while rest < 9 {
            rest += transport.recv(&mut buffer, deadline()).await.expect("recv");
            if buffer.len() >= 8 {
                buffer.clear();
            }
        }
        assert_eq!(rest, 9, "the other bytes were still on the socket");
        peer.close().await.expect("close the peer");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound on send)
    #[tokio::test]
    async fn send_refuses_a_frame_over_the_limit_and_sends_nothing() {
        let script = Script::strict(vec![
            Step::Expect(b"1234".to_vec()),
            Step::Reply(b"ok".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(4, 8), script).await;
        let error = expect_err(transport.send(Bytes::from_static(b"12345"), deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "frame over max_frame_bytes"
        );
        transport
            .send(Bytes::from_static(b"1234"), deadline())
            .await
            .expect("frame at the limit");
        let mut buffer = BytesMut::new();
        recv_until(transport.as_mut(), &mut buffer, 2)
            .await
            .expect("the peer answers once it has the frame");
        let report = peer.close().await.expect("close the peer");
        assert_eq!(
            report.received, b"1234",
            "the refused frame never reached the wire"
        );
        assert!(
            report.mismatches.is_empty(),
            "the peer saw what it expected"
        );
    }

    #[tokio::test]
    async fn send_delivers_the_bytes_in_order() {
        let script = Script::strict(vec![
            Step::Expect(b"onetwo".to_vec()),
            Step::Reply(b"ok".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        transport
            .send(Bytes::from_static(b"one"), deadline())
            .await
            .expect("first");
        transport
            .send(Bytes::new(), deadline())
            .await
            .expect("empty frame");
        transport
            .send(Bytes::from_static(b"two"), deadline())
            .await
            .expect("second");
        let mut buffer = BytesMut::new();
        recv_until(transport.as_mut(), &mut buffer, 2)
            .await
            .expect("the peer answers once it has every byte");
        let report = peer.close().await.expect("close the peer");
        assert_eq!(report.received, b"onetwo", "bytes arrive in order");
        assert!(
            report.mismatches.is_empty(),
            "the peer saw what it expected"
        );
    }

    #[tokio::test]
    async fn shutdown_completes_tells_the_peer_and_closes_the_transport() {
        let (mut transport, mut device) = connected(limits(64, 128)).await;
        transport.shutdown(deadline()).await.expect("shutdown");
        let mut rest = Vec::new();
        let read = device
            .read_to_end(&mut rest)
            .await
            .expect("peer sees end of stream");
        assert_eq!(read, 0, "the peer received the half-close");

        let error = expect_err(transport.send(Bytes::from_static(b"x"), deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after shutdown");
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after shutdown");
        transport
            .shutdown(deadline())
            .await
            .expect("a second shutdown is a no-op");
        transport
            .shutdown(Deadline::at(Instant::now()))
            .await
            .expect("a repeat shutdown succeeds even with an elapsed deadline");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (elapsed deadline)
    #[tokio::test]
    async fn an_elapsed_deadline_fails_with_timeout_before_any_io() {
        let script = Script::strict(vec![
            Step::Expect(b"ok".to_vec()),
            Step::Reply(b"back".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        let elapsed = Deadline::at(Instant::now());

        let error = expect_err(transport.send(Bytes::from_static(b"late"), elapsed)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "send");
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, elapsed)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "recv");
        let error = expect_err(transport.shutdown(elapsed)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "shutdown");
        let closed = Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: 9,
        };
        let error = expect_err(TcpFactory::default().open(&closed, limits(64, 128), elapsed)).await;
        assert_eq!(error.kind(), ErrorKind::Timeout, "open");

        // None of the timed-out calls touched the transport: it still works.
        transport
            .send(Bytes::from_static(b"ok"), deadline())
            .await
            .expect("still usable");
        recv_until(transport.as_mut(), &mut buffer, 4)
            .await
            .expect("recv still works and nothing was consumed");
        assert_eq!(&buffer[..], b"back", "the peer's reply");
        let report = peer.close().await.expect("close the peer");
        assert_eq!(
            report.received, b"ok",
            "nothing but the later frame was sent"
        );
    }

    #[tokio::test]
    async fn a_dropped_recv_loses_nothing() {
        let script = Script::strict(vec![
            Step::Expect(b"go".to_vec()),
            Step::Reply(b"data".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        let mut buffer = BytesMut::new();
        {
            let mut pending = transport.recv(&mut buffer, deadline());
            assert!(poll_once(&mut pending).is_pending(), "nothing to read yet");
        }
        transport
            .send(Bytes::from_static(b"go"), deadline())
            .await
            .expect("send");
        recv_until(transport.as_mut(), &mut buffer, 4)
            .await
            .expect("recv after a cancelled one");
        assert_eq!(&buffer[..], b"data", "the transport stayed usable");
        peer.close().await.expect("close the peer");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (send is not cancel-safe)
    #[tokio::test]
    #[cfg_attr(
        windows,
        ignore = "Windows loopback accepts a 32 MiB write at once (IOCP), so the write never pends; the drop-mid-write poison path is exercised on Unix and by the unit test on a reset connection"
    )]
    async fn a_send_cut_short_poisons_the_transport() {
        const FRAME: usize = 32 * 1024 * 1024;
        let (mut transport, _device) = connected(limits(FRAME, FRAME)).await;
        let payload = Bytes::from(vec![0_u8; FRAME]);
        {
            // The peer never reads, so the socket buffers fill and the write must pend.
            let mut sending = transport.send(payload, deadline());
            assert!(
                poll_once(&mut sending).is_pending(),
                "the write cannot complete"
            );
        }
        let error = expect_err(transport.send(Bytes::from_static(b"next"), deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "a partial frame may be on the wire"
        );
        let mut buffer = BytesMut::new();
        let error = expect_err(transport.recv(&mut buffer, deadline())).await;
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "recv on a poisoned transport"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_boxed_transport_can_be_driven_from_another_worker_thread() {
        let script = Script::strict(vec![
            Step::Reply(b"echo".to_vec()),
            Step::Expect(b"echo".to_vec()),
            Step::Reply(b"done".to_vec()),
        ]);
        let (mut transport, peer) = connected_to_script(limits(64, 128), script).await;
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let mut buffer = BytesMut::new();
            recv_until(transport.as_mut(), &mut buffer, 4).await?;
            transport.send(buffer.split().freeze(), deadline()).await?;
            recv_until(transport.as_mut(), &mut buffer, 4).await?;
            transport.shutdown(deadline()).await?;
            Ok::<_, PlcError>(buffer)
        });
        let finished = tasks
            .join_next()
            .await
            .expect("one task")
            .expect("task joined");
        assert_eq!(
            &finished.expect("task succeeded")[..],
            b"done",
            "peer's last word"
        );
        let report = peer.close().await.expect("close the peer");
        assert_eq!(report.received, b"echo", "the task echoed the bytes");
    }
}
