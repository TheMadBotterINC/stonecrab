use std::future::Future;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener as StdTcpListener};
use std::time::Duration;

use stonecrab_core::Endpoint;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::script::{Action, Machine, Script, ScriptReport};

/// The longest a [`ScriptedPeer`] sleeps for one [`Step::Delay`](crate::Step::Delay) or
/// [`Step::LateReply`](crate::Step::LateReply): 200 milliseconds.
///
/// Loopback tests cover bytes only (decision D21); timing is tested on
/// [`FakeTransport`](crate::FakeTransport) and the [`FakeClock`](crate::FakeClock). A
/// longer request is cut to this bound, so a test cannot stall on real time.
///
/// # Examples
/// ```
/// use std::time::Duration;
///
/// assert_eq!(stonecrab_testkit::MAX_PEER_DELAY, Duration::from_millis(200));
/// ```
pub const MAX_PEER_DELAY: Duration = Duration::from_millis(200);

/// Cap a requested delay at [`MAX_PEER_DELAY`].
fn clamp_delay(requested: Duration) -> Duration {
    requested.min(MAX_PEER_DELAY)
}

/// Run `future` until it finishes or the stop signal fires; `None` means stop.
async fn until_stop<T>(
    future: impl Future<Output = T>,
    stop: &mut oneshot::Receiver<()>,
) -> Option<T> {
    tokio::select! {
        output = future => Some(output),
        _ = stop => None,
    }
}

/// Sleep for the clamped `requested` time; false when told to stop meanwhile.
async fn pause(requested: Duration, stop: &mut oneshot::Receiver<()>) -> bool {
    if requested.is_zero() {
        return true;
    }
    until_stop(tokio::time::sleep(clamp_delay(requested)), stop)
        .await
        .is_some()
}

/// Carry out one action on the socket; false when the peer should stop.
async fn perform(stream: &mut TcpStream, action: Action, stop: &mut oneshot::Receiver<()>) -> bool {
    match action {
        Action::Delay(by) => pause(by, stop).await,
        Action::Write {
            chunks,
            late,
            close,
        } => {
            if !pause(late, stop).await {
                return false;
            }
            for chunk in chunks {
                let written = until_stop(
                    async {
                        stream.write_all(&chunk).await?;
                        stream.flush().await
                    },
                    stop,
                )
                .await;
                if !matches!(written, Some(Ok(()))) {
                    return false;
                }
                // Give the receiver a chance to see this piece on its own.
                tokio::task::yield_now().await;
            }
            if close {
                half_close(stream, stop).await
            } else {
                true
            }
        }
        Action::Close => half_close(stream, stop).await,
    }
}

/// Send a FIN but keep the socket, so unread client bytes cannot turn it into a reset.
async fn half_close(stream: &mut TcpStream, stop: &mut oneshot::Receiver<()>) -> bool {
    until_stop(stream.shutdown(), stop).await.is_some()
}

/// Run the script on the accepted connection until the client closes or `stop` fires.
async fn serve(mut stream: TcpStream, machine: &mut Machine, stop: &mut oneshot::Receiver<()>) {
    drop(stream.set_nodelay(true));
    let mut chunk = [0_u8; 4096];
    loop {
        while let Some(action) = machine.next() {
            if !perform(&mut stream, action, stop).await {
                return;
            }
        }
        match until_stop(stream.read(&mut chunk), stop).await {
            Some(Ok(read)) if read > 0 => machine.feed(chunk.get(..read).unwrap_or_default()),
            Some(Ok(_) | Err(_)) | None => return,
        }
    }
}

/// The peer task: accept one connection, then serve it.
async fn run(
    listener: TcpListener,
    mut machine: Machine,
    mut stop: oneshot::Receiver<()>,
) -> ScriptReport {
    let accepted = until_stop(listener.accept(), &mut stop).await;
    drop(listener);
    if let Some(Ok((stream, _))) = accepted {
        serve(stream, &mut machine, &mut stop).await;
    }
    machine.into_report()
}

/// A scripted TCP peer on `127.0.0.1`, for byte-level tests of real sockets.
///
/// [`spawn`](Self::spawn) binds an ephemeral loopback port and starts one task that
/// accepts a single connection and runs the [`Script`] on it. Nothing leaves the machine.
/// Connect to [`addr`](Self::addr) or [`endpoint`](Self::endpoint), drive the exchange,
/// then call [`close`](Self::close) for the [`ScriptReport`].
///
/// After the script's last step the peer keeps the connection open, recording any further
/// bytes, until the client closes or [`close`](Self::close) is called; a
/// [`Step::CloseAt`](crate::Step::CloseAt) or strict failure sends a FIN instead and
/// keeps reading, so unread client bytes cannot cause a reset. The listener is dropped
/// after the first accept, so a second connection is refused.
///
/// # Time
/// Loopback tests cover bytes only (decision D21): [`Step::Delay`](crate::Step::Delay)
/// and [`Step::LateReply`](crate::Step::LateReply) sleep on the Tokio timer, for at most
/// [`MAX_PEER_DELAY`]. Deadline, late-reply and connect-timeout behavior belongs on
/// [`FakeTransport`](crate::FakeTransport).
///
/// # Task ownership
/// This type owns the only `tokio::spawn` in the workspace and its `JoinHandle`
/// (coding standards 8.7): [`close`](Self::close) stops and joins the task, and dropping
/// the peer aborts it if it is still running. The stop signal is a one-slot channel;
/// there are no unbounded channels.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// A Stonecrab test double with no upstream counterpart.
///
/// # Examples
/// ```
/// use stonecrab_testkit::{Script, ScriptedPeer, Step};
/// use tokio::io::{AsyncReadExt, AsyncWriteExt};
/// use tokio::net::TcpStream;
///
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(async {
///     let peer = ScriptedPeer::spawn(Script::strict(vec![
///         Step::Expect(b"ping".to_vec()),
///         Step::Reply(b"pong".to_vec()),
///     ]))?;
///     let mut client = TcpStream::connect(peer.addr()).await?;
///     client.write_all(b"ping").await?;
///     let mut reply = [0_u8; 4];
///     client.read_exact(&mut reply).await?;
///     assert_eq!(&reply, b"pong");
///
///     let report = peer.close().await?;
///     assert_eq!(report.received, b"ping");
///     assert!(report.mismatches.is_empty());
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug)]
pub struct ScriptedPeer {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<ScriptReport>>,
}

impl ScriptedPeer {
    /// Bind `127.0.0.1:0` and start the peer task running `script`.
    ///
    /// The port is chosen by the operating system and readable at once from
    /// [`port`](Self::port). Connections made before the task is scheduled wait in the
    /// kernel backlog.
    ///
    /// # Errors
    /// An [`io::Error`] if the loopback socket cannot be bound or registered, or if the
    /// caller is not inside a Tokio runtime.
    ///
    /// # Panics
    /// Panics if the current runtime was built without its IO driver (`enable_io` or
    /// `enable_all`); `#[tokio::test]` enables it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeer};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let peer = ScriptedPeer::spawn(Script::default())?;
    ///     assert_ne!(peer.port(), 0);
    ///     assert!(peer.addr().ip().is_loopback());
    ///     Ok::<(), std::io::Error>(())
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[expect(
        clippy::disallowed_methods,
        reason = "ScriptedPeer owns the JoinHandle: joined on close, aborted on drop (CODING_STANDARDS 8.7)"
    )]
    pub fn spawn(script: Script) -> io::Result<Self> {
        tokio::runtime::Handle::try_current().map_err(io::Error::other)?;
        let std_listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        std_listener.set_nonblocking(true)?;
        let addr = std_listener.local_addr()?;
        let listener = TcpListener::from_std(std_listener)?;
        let (stop, stopped) = oneshot::channel();
        let task = tokio::spawn(run(listener, Machine::new(script), stopped));
        Ok(Self {
            addr,
            stop: Some(stop),
            task: Some(task),
        })
    }

    /// The socket address the peer listens on.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeer};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let peer = ScriptedPeer::spawn(Script::default())?;
    ///     assert_eq!(peer.addr().port(), peer.port());
    ///     Ok::<(), std::io::Error>(())
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The listening port.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeer};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let peer = ScriptedPeer::spawn(Script::default())?;
    ///     assert_ne!(peer.port(), 0);
    ///     Ok::<(), std::io::Error>(())
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The peer as an `Endpoint::Tcp { host: "127.0.0.1", port }`, ready for a factory.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::Endpoint;
    /// use stonecrab_testkit::{Script, ScriptedPeer};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let peer = ScriptedPeer::spawn(Script::default())?;
    ///     let Endpoint::Tcp { host, port } = peer.endpoint() else {
    ///         panic!("a TCP endpoint");
    ///     };
    ///     assert_eq!((host.as_str(), port), ("127.0.0.1", peer.port()));
    ///     Ok::<(), std::io::Error>(())
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    #[must_use]
    pub fn endpoint(&self) -> Endpoint {
        Endpoint::Tcp {
            host: Ipv4Addr::LOCALHOST.to_string(),
            port: self.addr.port(),
        }
    }

    /// Stop the peer, wait for its task and return what the script saw and did.
    ///
    /// Steps that have not finished are abandoned, so wait for the effects a test asserts
    /// on (a reply read, a connection closed) before calling this. Only bytes the peer
    /// task already read are in the report.
    ///
    /// # Errors
    /// An [`io::Error`] if the peer task panicked (a [`Step::Respond`](crate::Step::Respond)
    /// closure that panics, say) or was cancelled.
    ///
    /// # Cancel safety
    /// Cancel-safe: the stop signal is sent before the first await, so a dropped
    /// `close()` leaves the task to end on its own.
    ///
    /// # Wire effects
    /// Closes the peer's socket; any bytes still unsent are dropped.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeer, Step};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let peer = ScriptedPeer::spawn(Script::new(vec![Step::Reply(b"never connected".to_vec())]))?;
    ///     let report = peer.close().await?;
    ///     assert!(report.ran.is_empty(), "no client, so nothing ran");
    ///     Ok::<(), std::io::Error>(())
    /// })?;
    /// # Ok::<(), std::io::Error>(())
    /// ```
    pub async fn close(mut self) -> io::Result<ScriptReport> {
        if let Some(stop) = self.stop.take() {
            stop.send(()).ok();
        }
        let Some(task) = self.task.take() else {
            return Ok(ScriptReport::default());
        };
        task.await.map_err(io::Error::other)
    }
}

impl Drop for ScriptedPeer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Instant;

    use super::*;
    use crate::Step;

    async fn read_exactly(client: &mut TcpStream, count: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; count];
        client.read_exact(&mut bytes).await.expect("read");
        bytes
    }

    async fn read_to_end(client: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).await.expect("read to end");
        bytes
    }

    #[test]
    fn delays_are_clamped_to_the_documented_maximum() {
        assert_eq!(
            clamp_delay(Duration::from_millis(5)),
            Duration::from_millis(5),
            "short"
        );
        assert_eq!(
            clamp_delay(Duration::from_secs(3600)),
            MAX_PEER_DELAY,
            "long"
        );
        assert_eq!(
            MAX_PEER_DELAY,
            Duration::from_millis(200),
            "documented bound"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (scripted loopback peer)
    #[tokio::test]
    async fn expect_and_reply_run_in_order_and_the_report_records_them() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(b"ping".to_vec()),
            Step::Reply(b"pong".to_vec()),
        ]))
        .expect("spawn");
        assert_eq!(peer.addr().port(), peer.port(), "port accessor");
        assert_eq!(
            peer.endpoint(),
            Endpoint::Tcp {
                host: "127.0.0.1".to_owned(),
                port: peer.port()
            },
            "endpoint"
        );
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        client.write_all(b"ping").await.expect("write");
        assert_eq!(read_exactly(&mut client, 4).await, b"pong", "reply");
        let report = peer.close().await.expect("close");
        assert_eq!(report.received, b"ping", "received");
        assert_eq!(report.ran, vec![0, 1], "both steps ran");
        assert!(!report.failed && !report.closed, "clean");
    }

    #[tokio::test]
    async fn a_strict_mismatch_is_recorded_and_the_peer_closes() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(b"good".to_vec()),
            Step::Reply(b"unreachable".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        client.write_all(b"evil").await.expect("write");
        assert!(
            read_to_end(&mut client).await.is_empty(),
            "closed with no reply"
        );
        let report = peer.close().await.expect("close");
        assert!(report.failed && report.closed, "strict failure");
        assert_eq!(report.mismatches.len(), 1, "one mismatch");
        let mismatch = report.mismatches.first().expect("recorded");
        assert_eq!(
            (
                mismatch.step,
                mismatch.expected.as_slice(),
                mismatch.actual.as_slice()
            ),
            (0, &b"good"[..], &b"evil"[..]),
            "mismatch contents"
        );
    }

    #[tokio::test]
    async fn a_lenient_mismatch_does_not_stop_the_script() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::Expect(b"good".to_vec()),
            Step::Reply(b"still here".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        client.write_all(b"evil").await.expect("write");
        assert_eq!(read_exactly(&mut client, 10).await, b"still here", "reply");
        let report = peer.close().await.expect("close");
        assert_eq!(report.mismatches.len(), 1, "recorded");
        assert!(!report.failed, "not failed");
    }

    #[tokio::test]
    async fn fragmented_and_coalesced_replies_arrive_as_the_same_bytes() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::ReplyFragmented(vec![b"01".to_vec(), b"23".to_vec(), b"45".to_vec()]),
            Step::ReplyCoalescedWithNext,
            Step::Reply(b"ab".to_vec()),
            Step::Reply(b"cd".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(
            read_exactly(&mut client, 10).await,
            b"012345abcd",
            "all bytes in order"
        );
        let report = peer.close().await.expect("close");
        assert_eq!(report.ran, vec![0, 1, 2, 3], "all steps ran");
    }

    #[tokio::test]
    async fn truncate_corrupt_and_wrong_transaction_id_edit_the_reply() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::WrongTransactionId,
            Step::Corrupt {
                offset: 2,
                bytes: vec![0x7F],
            },
            Step::Truncate(4),
            Step::Reply(vec![0x00, 0x01, 0x02, 0x03, 0x04, 0x05]),
            Step::Reply(b"next".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(
            read_exactly(&mut client, 8).await,
            [0xFF, 0xFE, 0x7F, 0x03, b'n', b'e', b'x', b't'],
            "edited reply, then the next one untouched, connection still open"
        );
        peer.close().await.expect("close");
    }

    #[tokio::test]
    async fn close_at_sends_a_prefix_then_end_of_stream() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::CloseAt(4),
            Step::Reply(b"half a frame".to_vec()),
            Step::Reply(b"never".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(read_to_end(&mut client).await, b"half", "prefix then FIN");
        let report = peer.close().await.expect("close");
        assert!(report.closed && !report.failed, "closed by the script");
        assert_eq!(report.ran, vec![0, 1], "later steps never ran");
    }

    #[tokio::test]
    async fn respond_serves_several_requests_and_may_stay_silent() {
        let calls = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&calls);
        let peer = ScriptedPeer::spawn(Script::new(vec![Step::respond(move |request| {
            let seen = counter.fetch_add(1, Ordering::SeqCst).saturating_add(1);
            (seen != 2).then(|| request.iter().rev().copied().collect())
        })]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        client.write_all(b"abc").await.expect("write");
        assert_eq!(read_exactly(&mut client, 3).await, b"cba", "first answer");
        // The silent second request: wait until the peer has seen it, without sleeping.
        client.write_all(b"x").await.expect("write");
        while calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
        client.write_all(b"yz").await.expect("write");
        assert_eq!(read_exactly(&mut client, 2).await, b"zy", "third answer");
        let report = peer.close().await.expect("close");
        assert_eq!(report.responses, 3, "three requests");
        assert_eq!(report.ran, vec![0], "respond counted once");
    }

    #[tokio::test]
    async fn delay_and_late_reply_wait_a_bounded_time_before_the_reply() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::Delay(Duration::from_millis(20)),
            Step::Reply(b"a".to_vec()),
            Step::LateReply(Duration::from_secs(3600)),
            Step::Reply(b"b".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        let started = Instant::now();
        assert_eq!(read_exactly(&mut client, 2).await, b"ab", "both replies");
        assert!(
            started.elapsed() >= MAX_PEER_DELAY,
            "the hour-long late reply waited the bounded maximum"
        );
        peer.close().await.expect("close");
    }

    #[tokio::test]
    async fn bytes_after_the_script_ends_are_recorded_and_a_client_close_ends_the_task() {
        let peer =
            ScriptedPeer::spawn(Script::new(vec![Step::Reply(b"hi".to_vec())])).expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(read_exactly(&mut client, 2).await, b"hi", "banner");
        client.write_all(b"extra").await.expect("write");
        client.shutdown().await.expect("client FIN");
        // The peer drops its socket on seeing our FIN, after reading "extra".
        assert!(
            read_to_end(&mut client).await.is_empty(),
            "peer saw the FIN"
        );
        let report = peer.close().await.expect("close");
        assert_eq!(report.received, b"extra", "late bytes recorded");
    }

    #[tokio::test]
    async fn only_one_connection_is_accepted_and_the_listener_then_closes() {
        let peer =
            ScriptedPeer::spawn(Script::new(vec![Step::Reply(b"one".to_vec())])).expect("spawn");
        let mut first = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(
            read_exactly(&mut first, 3).await,
            b"one",
            "first client served"
        );
        let second = TcpStream::connect(peer.addr()).await;
        assert!(
            second.is_err(),
            "the listener was dropped after the first accept"
        );
        peer.close().await.expect("close");
    }

    #[tokio::test]
    async fn close_stops_a_peer_stuck_on_an_unfinished_step() {
        let peer = ScriptedPeer::spawn(Script::new(vec![Step::Expect(b"never sent".to_vec())]))
            .expect("spawn");
        let _client = TcpStream::connect(peer.addr()).await.expect("connect");
        let report = peer.close().await.expect("close joins the task");
        assert!(report.ran.is_empty(), "the expectation never completed");
    }

    #[tokio::test]
    async fn a_writer_blocked_on_a_client_that_never_reads_is_still_stopped_by_close() {
        let big = vec![0_u8; 64 * 1024 * 1024];
        let peer = ScriptedPeer::spawn(Script::new(vec![Step::Reply(big)])).expect("spawn");
        let _client = TcpStream::connect(peer.addr()).await.expect("connect");
        tokio::task::yield_now().await;
        peer.close()
            .await
            .expect("close interrupts the blocked write");
    }

    #[tokio::test]
    async fn a_panicking_responder_surfaces_as_an_error_from_close() {
        let peer = ScriptedPeer::spawn(Script::new(vec![Step::respond(|_| {
            std::panic::resume_unwind(Box::new("responder failed"))
        })]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        client.write_all(b"x").await.expect("write");
        assert_eq!(
            client.read_u8().await.ok(),
            None,
            "the task died and the socket closed"
        );
        assert!(peer.close().await.is_err(), "the panic is reported");
    }

    #[tokio::test]
    async fn dropping_the_peer_aborts_the_task_and_releases_the_connection() {
        let peer = ScriptedPeer::spawn(Script::new(vec![
            Step::Reply(b"hi".to_vec()),
            Step::Expect(b"x".to_vec()),
        ]))
        .expect("spawn");
        let mut client = TcpStream::connect(peer.addr()).await.expect("connect");
        assert_eq!(
            read_exactly(&mut client, 2).await,
            b"hi",
            "the connection was accepted"
        );
        drop(peer);
        assert!(
            read_to_end(&mut client).await.is_empty(),
            "connection closed by the abort"
        );
    }

    #[tokio::test]
    async fn spawn_outside_a_runtime_is_an_error_not_a_panic() {
        let result = std::thread::spawn(|| ScriptedPeer::spawn(Script::default()).map(|_| ()))
            .join()
            .expect("thread");
        assert!(result.is_err(), "no runtime on that thread");
    }
}
