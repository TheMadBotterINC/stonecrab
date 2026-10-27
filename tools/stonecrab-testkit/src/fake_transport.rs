use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use bytes::{Bytes, BytesMut};
use stonecrab_core::{
    Clock, Deadline, Diagnostics, Endpoint, ErrorKind, FailureCategory, OpFuture, Operation,
    PlcError, ResourceLimits, Result,
};
use stonecrab_transport::Transport;

use crate::FakeClock;
use crate::script::{Action, Machine, Script, ScriptReport};

/// A transport failure of `kind`, shaped like the ones `TcpTransport` returns.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn failure(
    kind: ErrorKind,
    operation: Option<Operation>,
    site: &str,
    detail: &str,
) -> PlcError {
    let mut diagnostics = Diagnostics::default();
    diagnostics.operation = operation;
    diagnostics.category = Some(FailureCategory::Transport);
    PlcError::new(kind, format!("{site}: {detail}")).with_diagnostics(diagnostics)
}

/// One thing the peer puts on the wire, and when it becomes readable.
#[derive(Debug)]
struct Delivery {
    at: Instant,
    payload: Payload,
}

#[derive(Debug)]
enum Payload {
    Data(Vec<u8>),
    Close,
}

/// What `recv` found at the front of the queue.
enum Taken {
    Served(usize),
    Closed,
    NotBefore(Instant),
    Nothing,
}

/// State shared by a [`FakeTransport`] and its [`FakeTransportProbe`]s.
#[derive(Debug)]
struct Inner {
    machine: Machine,
    sent: Vec<Bytes>,
    queue: VecDeque<Delivery>,
    /// The script's own time: where the next scheduled delivery is measured from.
    timeline: Instant,
    usable: bool,
    shut_down: bool,
}

impl Inner {
    /// Run the script as far as it can go without more input and schedule its output.
    fn pump(&mut self) {
        while let Some(action) = self.machine.next() {
            match action {
                Action::Delay(by) => {
                    self.timeline = self.timeline.checked_add(by).unwrap_or(self.timeline);
                }
                Action::Write {
                    chunks,
                    late,
                    close,
                } => {
                    let at = self.timeline.checked_add(late).unwrap_or(self.timeline);
                    for chunk in chunks {
                        self.queue.push_back(Delivery {
                            at,
                            payload: Payload::Data(chunk),
                        });
                    }
                    if close {
                        self.queue.push_back(Delivery {
                            at,
                            payload: Payload::Close,
                        });
                    }
                }
                Action::Close => self.queue.push_back(Delivery {
                    at: self.timeline,
                    payload: Payload::Close,
                }),
            }
        }
    }

    /// Serve the front delivery into `buffer` if it is due at `now`.
    fn take(&mut self, now: Instant, free: usize, buffer: &mut BytesMut) -> Taken {
        let Some(front) = self.queue.front_mut() else {
            return Taken::Nothing;
        };
        if front.at > now {
            return Taken::NotBefore(front.at);
        }
        match &mut front.payload {
            Payload::Close => {
                self.queue.pop_front();
                self.usable = false;
                Taken::Closed
            }
            Payload::Data(data) => {
                let take = data.len().min(free);
                let rest = data.split_off(take);
                buffer.extend_from_slice(data);
                if rest.is_empty() {
                    self.queue.pop_front();
                } else {
                    *data = rest;
                }
                Taken::Served(take)
            }
        }
    }
}

/// A shared handle on the state of one [`FakeTransport`].
///
/// A transport handed out as a `Box<dyn Transport>` cannot be inspected any more; the
/// probe can. Made by [`FakeTransport::probe`], or for every transport a
/// [`FakeTransportFactory`](crate::FakeTransportFactory) opened by
/// [`FakeTransportFactory::probes`](crate::FakeTransportFactory::probes). Clones share
/// the same state.
///
/// # Examples
/// ```
/// use stonecrab_core::ResourceLimits;
/// use stonecrab_testkit::{FakeClock, FakeTransport, Script};
///
/// let limits = ResourceLimits::builder().build()?;
/// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
/// let probe = transport.probe();
/// assert!(probe.sent().is_empty());
/// assert!(!probe.is_shut_down());
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Debug, Clone)]
pub struct FakeTransportProbe {
    inner: Arc<Mutex<Inner>>,
}

impl FakeTransportProbe {
    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut inner)
    }

    /// Every frame the client sent, in order.
    ///
    /// A frame refused before sending (an elapsed deadline, a closed transport, a frame
    /// over the limit) is not logged, so an empty log proves nothing reached the wire.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use bytes::Bytes;
    /// use stonecrab_core::{Clock, Deadline, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script, block_on_with};
    /// use stonecrab_transport::Transport;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let mut transport = FakeTransport::new(Script::default(), clock.clone(), limits);
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
    /// block_on_with(&clock, transport.send(Bytes::from_static(b"hi"), deadline))?;
    /// assert_eq!(transport.probe().sent(), vec![Bytes::from_static(b"hi")]);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn sent(&self) -> Vec<Bytes> {
        self.with(|inner| inner.sent.clone())
    }

    /// What the script has seen and done so far.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ResourceLimits;
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script};
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let probe = FakeTransport::new(Script::default(), FakeClock::default(), limits).probe();
    /// assert!(probe.report().ran.is_empty());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn report(&self) -> ScriptReport {
        self.with(|inner| inner.machine.report().clone())
    }

    /// True once `shutdown` has been called on the transport.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ResourceLimits;
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script};
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let probe = FakeTransport::new(Script::default(), FakeClock::default(), limits).probe();
    /// assert!(!probe.is_shut_down());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn is_shut_down(&self) -> bool {
        self.with(|inner| inner.shut_down)
    }
}

/// An in-memory [`Transport`] driven by a [`Script`] and a [`FakeClock`].
///
/// The peer is the script. [`send`](Transport::send) logs the frame (see
/// [`FakeTransportProbe::sent`]) and feeds it to the script, which answers
/// at once: whatever the script now wants to say is queued for `recv`.
/// [`recv`](Transport::recv) serves the queue, one write per call (a
/// [`Step::ReplyFragmented`](crate::Step::ReplyFragmented) piece, or a coalesced group),
/// into the free space of the caller's buffer, so `buffer` never grows past
/// `limits().max_buffered_bytes()` exactly as `TcpTransport` guarantees; a write that does
/// not fit is served over several calls.
///
/// # Time
/// Nothing sleeps on real time. The script has its own timeline, which starts at the
/// clock's reading when the transport is built and catches up to the clock when `send`
/// feeds it. [`Step::Delay`](crate::Step::Delay) and
/// [`Step::LateReply`](crate::Step::LateReply) move delivery times along that timeline.
/// When `recv` finds nothing due it sleeps on the [`FakeClock`] until the earliest of the
/// delivery time and the call's deadline, so [`block_on_with`](crate::block_on_with)
/// drives it; if the deadline comes first (or ties) the call fails with `Timeout`, and a
/// call with nothing queued at all waits out its deadline, as a silent peer would.
///
/// # Closing and poisoning
/// [`Step::CloseAt`](crate::Step::CloseAt) and a strict script failure queue an end of
/// stream: after the bytes before it, `recv` returns `Ok(0)` once and the transport is
/// then unusable, so every later `send` and `recv` fails with `Disconnected`. `shutdown`
/// is idempotent, like `TcpTransport`'s. A `recv` that times out or is dropped consumes
/// nothing and leaves the transport usable. Unlike TCP, `send` never waits and never
/// poisons the transport.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-DEADLINES
/// A Stonecrab test double with no upstream counterpart; it follows the `Transport`
/// contract, deadlines included.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use bytes::{Bytes, BytesMut};
/// use stonecrab_core::{Clock, Deadline, ResourceLimits};
/// use stonecrab_testkit::{FakeClock, FakeTransport, Script, Step, block_on_with};
/// use stonecrab_transport::Transport;
///
/// let clock = FakeClock::default();
/// let script = Script::strict(vec![
///     Step::Expect(b"ping".to_vec()),
///     Step::Delay(Duration::from_secs(2)),
///     Step::Reply(b"pong".to_vec()),
/// ]);
/// let limits = ResourceLimits::builder().build()?;
/// let mut transport = FakeTransport::new(script, clock.clone(), limits);
/// let started = clock.now();
/// let deadline = Deadline::after(started, Duration::from_secs(5))?;
///
/// block_on_with(&clock, transport.send(Bytes::from_static(b"ping"), deadline))?;
/// let mut reply = BytesMut::new();
/// let read = block_on_with(&clock, transport.recv(&mut reply, deadline))?;
/// assert_eq!((read, &reply[..]), (4, &b"pong"[..]));
/// assert_eq!(clock.now() - started, Duration::from_secs(2), "the delay ran on the fake clock");
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Debug)]
pub struct FakeTransport {
    inner: Arc<Mutex<Inner>>,
    clock: FakeClock,
    peer: Endpoint,
    limits: ResourceLimits,
}

impl FakeTransport {
    /// A transport whose peer runs `script` on `clock`, enforcing `limits`.
    ///
    /// The peer endpoint is `Endpoint::Custom("fake://transport")`; change it with
    /// [`with_peer`](Self::with_peer). Steps that need no input (a leading `Reply`, say)
    /// are scheduled immediately.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ResourceLimits;
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script, Step};
    /// use stonecrab_transport::Transport;
    ///
    /// let script = Script::new(vec![Step::Reply(b"banner".to_vec())]);
    /// let limits = ResourceLimits::builder().max_frame_bytes(8).build()?;
    /// let transport = FakeTransport::new(script, FakeClock::default(), limits);
    /// assert_eq!(transport.limits(), &limits);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn new(script: Script, clock: FakeClock, limits: ResourceLimits) -> Self {
        let mut inner = Inner {
            machine: Machine::new(script),
            sent: Vec::new(),
            queue: VecDeque::new(),
            timeline: clock.now(),
            usable: true,
            shut_down: false,
        };
        inner.pump();
        Self {
            inner: Arc::new(Mutex::new(inner)),
            clock,
            peer: Endpoint::Custom("fake://transport".to_owned()),
            limits,
        }
    }

    /// Report `peer` from [`Transport::peer`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script};
    /// use stonecrab_transport::Transport;
    ///
    /// let peer = Endpoint::Custom("fake://plc".to_owned());
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits)
    ///     .with_peer(peer.clone());
    /// assert_eq!(transport.peer(), &peer);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn with_peer(mut self, peer: Endpoint) -> Self {
        self.peer = peer;
        self
    }

    /// The frames passed to `send` so far, in order.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use std::time::Duration;
    /// use stonecrab_core::{Clock, Deadline, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script, block_on_with};
    /// use stonecrab_transport::Transport;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let mut transport = FakeTransport::new(Script::default(), clock.clone(), limits);
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
    /// block_on_with(&clock, transport.send(Bytes::from_static(b"hi"), deadline))?;
    /// assert_eq!(transport.sent(), vec![Bytes::from_static(b"hi")]);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn sent(&self) -> Vec<Bytes> {
        self.with(|inner| inner.sent.clone())
    }

    /// A handle for inspecting this transport after it has been boxed or moved.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ResourceLimits;
    /// use stonecrab_testkit::{FakeClock, FakeTransport, Script};
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
    /// let probe = transport.probe();
    /// drop(transport);
    /// assert!(probe.sent().is_empty());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn probe(&self) -> FakeTransportProbe {
        FakeTransportProbe {
            inner: Arc::clone(&self.inner),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut inner)
    }

    fn ensure_before(
        &self,
        deadline: Deadline,
        operation: Option<Operation>,
        site: &str,
    ) -> Result<()> {
        if deadline.is_elapsed(self.clock.now()) {
            Err(failure(
                ErrorKind::Timeout,
                operation,
                site,
                "deadline already elapsed",
            ))
        } else {
            Ok(())
        }
    }

    fn ensure_usable(&self, site: &str) -> Result<()> {
        if self.with(|inner| inner.usable) {
            Ok(())
        } else {
            Err(failure(
                ErrorKind::Disconnected,
                None,
                site,
                "transport is no longer usable",
            ))
        }
    }
}

impl Transport for FakeTransport {
    /// Logs `bytes` and feeds them to the script, within `deadline`.
    ///
    /// Checks run in this order, each before anything is logged: the deadline has not
    /// passed, the transport is usable, `bytes` fits in `max_frame_bytes`. The call
    /// never waits. The deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: the future completes on its first poll.
    ///
    /// # Wire effects
    /// None on a real wire; the frame is appended to the `sent()` log.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            const SITE: &str = "fake send";
            self.ensure_before(deadline, None, SITE)?;
            self.ensure_usable(SITE)?;
            let limit = self.limits.max_frame_bytes().get();
            if bytes.len() > limit {
                let detail = format!(
                    "frame of {} bytes exceeds the {limit} byte frame limit",
                    bytes.len()
                );
                return Err(failure(ErrorKind::ResourceLimit, None, SITE, &detail));
            }
            let now = self.clock.now();
            self.with(|inner| {
                inner.timeline = inner.timeline.max(now);
                inner.machine.feed(&bytes);
                inner.sent.push(bytes);
                inner.pump();
            });
            Ok(())
        })
    }

    /// Appends the next write the script has due to `buffer`, within `deadline`.
    ///
    /// Checks run in this order, each before any wait: the deadline has not passed, the
    /// transport is usable, `buffer` is below `max_buffered_bytes`. Then it serves what is
    /// due or sleeps on the `FakeClock` for the earliest of the delivery and the
    /// deadline. The deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: nothing is taken from the queue until the bytes are appended.
    ///
    /// # Wire effects
    /// None.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            const SITE: &str = "fake receive";
            self.ensure_before(deadline, None, SITE)?;
            self.ensure_usable(SITE)?;
            let bound = self.limits.max_buffered_bytes().get();
            let free = bound.saturating_sub(buffer.len());
            if free == 0 {
                let detail = format!("receive buffer already holds the {bound} byte limit");
                return Err(failure(ErrorKind::ResourceLimit, None, SITE, &detail));
            }
            loop {
                let now = self.clock.now();
                match self.with(|inner| inner.take(now, free, buffer)) {
                    Taken::Served(read) => return Ok(read),
                    Taken::Closed => return Ok(0),
                    Taken::NotBefore(at) if at < deadline.instant() => {
                        self.clock.sleep_until(at).await?;
                    }
                    Taken::NotBefore(_) | Taken::Nothing => {
                        self.clock.sleep_until(deadline.instant()).await?;
                        return Err(failure(ErrorKind::Timeout, None, SITE, "deadline elapsed"));
                    }
                }
            }
        })
    }

    /// Closes the transport within `deadline`; a repeat call is a no-op.
    ///
    /// A deadline that has already passed fails with `Timeout` and leaves the transport
    /// open, except on a repeat call, which succeeds. The deadline is the `deadline`
    /// argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: the future completes on its first poll.
    ///
    /// # Wire effects
    /// None on a real wire; [`FakeTransportProbe::is_shut_down`] turns true.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.with(|inner| inner.shut_down) {
                return Ok(());
            }
            self.ensure_before(deadline, Some(Operation::Close), "fake shutdown")?;
            self.with(|inner| {
                inner.shut_down = true;
                inner.usable = false;
            });
            Ok(())
        })
    }

    fn peer(&self) -> &Endpoint {
        &self.peer
    }

    fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{Step, block_on_with, poll_once};

    const SECOND: Duration = Duration::from_secs(1);

    fn limits(frame: usize, buffered: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(frame)
            .max_buffered_bytes(buffered)
            .build()
            .expect("valid limits")
    }

    fn transport(steps: Vec<Step>, clock: &FakeClock) -> FakeTransport {
        FakeTransport::new(Script::new(steps), clock.clone(), limits(64, 128))
    }

    fn in_secs(clock: &FakeClock, seconds: u64) -> Deadline {
        Deadline::after(clock.now(), Duration::from_secs(seconds)).expect("representable")
    }

    fn error_of<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    fn send(clock: &FakeClock, transport: &mut FakeTransport, bytes: &'static [u8]) -> Result<()> {
        let deadline = in_secs(clock, 60);
        block_on_with(clock, transport.send(Bytes::from_static(bytes), deadline))
    }

    fn recv(
        clock: &FakeClock,
        transport: &mut FakeTransport,
        buffer: &mut BytesMut,
        seconds: u64,
    ) -> Result<usize> {
        let deadline = in_secs(clock, seconds);
        block_on_with(clock, transport.recv(buffer, deadline))
    }

    #[test]
    fn the_doubles_are_send_and_the_probe_is_shareable() {
        fn assert_send<T: Send>() {}
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send::<FakeTransport>();
        assert_send_sync::<FakeTransportProbe>();
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (sent log, expectations)
    #[test]
    fn send_logs_every_frame_and_the_script_checks_it_against_expect() {
        let clock = FakeClock::default();
        let mut transport = FakeTransport::new(
            Script::strict(vec![
                Step::Expect(b"ping".to_vec()),
                Step::Reply(b"pong".to_vec()),
            ]),
            clock.clone(),
            limits(64, 128),
        );
        let probe = transport.probe();
        send(&clock, &mut transport, b"ping").expect("send");
        let mut buffer = BytesMut::new();
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 5).expect("recv"),
            4,
            "bytes read"
        );
        assert_eq!(&buffer[..], b"pong", "reply");
        assert_eq!(probe.sent(), vec![Bytes::from_static(b"ping")], "sent log");
        let report = probe.report();
        assert_eq!(report.ran, vec![0, 1], "both steps ran");
        assert!(report.mismatches.is_empty() && !report.failed, "clean");
        send(&clock, &mut transport, b"more").expect("send after the script ended");
        assert_eq!(probe.sent().len(), 2, "later frames are logged too");
        assert_eq!(probe.report().received, b"pingmore", "received bytes");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (frame bound on send)
    #[test]
    fn send_refuses_an_oversized_frame_an_elapsed_deadline_and_logs_nothing() {
        let clock = FakeClock::default();
        let mut transport = FakeTransport::new(Script::default(), clock.clone(), limits(4, 8));
        let probe = transport.probe();
        let error = error_of(send(&clock, &mut transport, b"12345"));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "frame over the limit"
        );
        let elapsed = Deadline::at(clock.now());
        let error = error_of(block_on_with(
            &clock,
            transport.send(Bytes::from_static(b"1"), elapsed),
        ));
        assert_eq!(error.kind(), ErrorKind::Timeout, "elapsed deadline");
        assert!(probe.sent().is_empty(), "nothing reached the wire");
        send(&clock, &mut transport, b"1234").expect("a frame at the limit");
        assert_eq!(probe.sent().len(), 1, "only the good frame is logged");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (buffer bound)
    #[test]
    fn recv_never_grows_the_buffer_past_the_bound_and_serves_a_write_over_several_calls() {
        let clock = FakeClock::default();
        let mut transport = FakeTransport::new(
            Script::new(vec![Step::Reply(b"0123456789AB".to_vec())]),
            clock.clone(),
            limits(4, 8),
        );
        let mut buffer = BytesMut::from(&b"xx"[..]);
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 5).expect("first"),
            6,
            "free space"
        );
        assert_eq!(&buffer[..], b"xx012345", "exactly at the bound");
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 5));
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "full buffer refused"
        );
        assert_eq!(&buffer[..], b"xx012345", "buffer untouched");
        buffer.clear();
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 5).expect("rest"),
            6,
            "remainder"
        );
        assert_eq!(&buffer[..], b"6789AB", "nothing was lost");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (fragmented and coalesced delivery)
    #[test]
    fn fragments_arrive_one_write_per_recv_and_coalesced_replies_in_one() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![
                Step::ReplyFragmented(vec![b"01".to_vec(), Vec::new(), b"234".to_vec()]),
                Step::ReplyCoalescedWithNext,
                Step::Reply(b"ab".to_vec()),
                Step::Reply(b"cd".to_vec()),
            ],
            &clock,
        );
        let mut buffer = BytesMut::new();
        let reads: Vec<usize> = (0..3)
            .map(|_| recv(&clock, &mut transport, &mut buffer, 5).expect("recv"))
            .collect();
        assert_eq!(
            reads,
            vec![2, 3, 4],
            "two fragments (the empty one skipped), one group"
        );
        assert_eq!(&buffer[..], b"01234abcd", "bytes in order");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (truncated, corrupt, wrong id)
    #[test]
    fn truncate_corrupt_and_wrong_transaction_id_reach_the_caller() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![
                Step::WrongTransactionId,
                Step::Corrupt {
                    offset: 2,
                    bytes: vec![0xAA],
                },
                Step::Truncate(4),
                Step::Reply(vec![1, 2, 3, 4, 5, 6]),
                Step::Reply(vec![7]),
            ],
            &clock,
        );
        let mut buffer = BytesMut::new();
        recv(&clock, &mut transport, &mut buffer, 5).expect("first");
        assert_eq!(&buffer[..], &[0xFE, 0xFD, 0xAA, 4], "edited and cut");
        recv(&clock, &mut transport, &mut buffer, 5).expect("second");
        assert_eq!(buffer.last(), Some(&7), "the next reply is untouched");
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 1));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "truncation does not close; the peer is silent"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (peer close mid-frame)
    #[test]
    fn close_at_delivers_a_prefix_then_zero_then_disconnected() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![Step::CloseAt(3), Step::Reply(b"half a frame".to_vec())],
            &clock,
        );
        let mut buffer = BytesMut::new();
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 5).expect("prefix"),
            3,
            "prefix"
        );
        assert_eq!(&buffer[..], b"hal", "prefix bytes");
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 5).expect("eof"),
            0,
            "end of stream"
        );
        assert_eq!(buffer.len(), 3, "nothing appended at end of stream");
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 5));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after close");
        let error = error_of(send(&clock, &mut transport, b"x"));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after close");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (delayed reply, deadline first)
    #[test]
    fn delay_runs_on_the_fake_clock_and_a_deadline_that_passes_first_is_a_timeout() {
        let clock = FakeClock::default();
        let start = clock.now();
        let mut transport = transport(
            vec![Step::Delay(10 * SECOND), Step::Reply(b"late".to_vec())],
            &clock,
        );
        let mut buffer = BytesMut::new();
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 5));
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline first");
        assert_eq!(
            clock.now(),
            start + 5 * SECOND,
            "the clock stopped at the deadline"
        );
        assert!(buffer.is_empty(), "nothing consumed");
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 30).expect("later"),
            4,
            "reply"
        );
        assert_eq!(
            clock.now(),
            start + 10 * SECOND,
            "the clock stopped at the delivery"
        );
        assert_eq!(&buffer[..], b"late", "the reply survived the timeout");
    }

    #[test]
    fn a_tie_between_delivery_and_deadline_is_a_timeout() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![Step::Delay(5 * SECOND), Step::Reply(b"x".to_vec())],
            &clock,
        );
        let mut buffer = BytesMut::new();
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 5));
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline wins ties");
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 1).expect("now due"),
            1,
            "then served"
        );
    }

    #[test]
    fn a_dropped_recv_leaves_the_queue_alone() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![Step::Delay(2 * SECOND), Step::Reply(b"r".to_vec())],
            &clock,
        );
        let mut buffer = BytesMut::new();
        let deadline = in_secs(&clock, 60);
        {
            let mut pending = transport.recv(&mut buffer, deadline);
            assert!(poll_once(&mut pending).is_pending(), "not due yet");
            assert_eq!(clock.sleeper_count(), 1, "sleeping on the fake clock");
        }
        assert_eq!(clock.sleeper_count(), 0, "dropping removed the sleeper");
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 60).expect("recv"),
            1,
            "still there"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (late reply)
    #[test]
    fn a_late_reply_holds_back_the_replies_behind_it_but_not_the_script() {
        let clock = FakeClock::default();
        let start = clock.now();
        let mut transport = transport(
            vec![
                Step::Expect(b"q".to_vec()),
                Step::LateReply(5 * SECOND),
                Step::Reply(b"A".to_vec()),
                Step::Reply(b"B".to_vec()),
            ],
            &clock,
        );
        send(&clock, &mut transport, b"q").expect("send");
        let mut buffer = BytesMut::new();
        recv(&clock, &mut transport, &mut buffer, 10).expect("A");
        assert_eq!(clock.now(), start + 5 * SECOND, "A arrived late");
        recv(&clock, &mut transport, &mut buffer, 10).expect("B");
        assert_eq!(
            clock.now(),
            start + 5 * SECOND,
            "B was ready and followed A at once"
        );
        assert_eq!(&buffer[..], b"AB", "in order");
    }

    #[test]
    fn the_script_timeline_catches_up_to_the_clock_when_a_frame_is_sent() {
        let clock = FakeClock::default();
        let mut transport = transport(
            vec![
                Step::Expect(b"q".to_vec()),
                Step::Delay(2 * SECOND),
                Step::Reply(b"r".to_vec()),
            ],
            &clock,
        );
        clock.advance(10 * SECOND);
        send(&clock, &mut transport, b"q").expect("send");
        let sent_at = clock.now();
        let mut buffer = BytesMut::new();
        recv(&clock, &mut transport, &mut buffer, 30).expect("recv");
        assert_eq!(
            clock.now(),
            sent_at + 2 * SECOND,
            "delay measured from the send"
        );
    }

    #[test]
    fn a_leading_reply_is_readable_before_any_send() {
        let clock = FakeClock::default();
        let mut transport = transport(vec![Step::Reply(b"banner".to_vec())], &clock);
        let mut buffer = BytesMut::new();
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 1).expect("banner"),
            6,
            "banner"
        );
    }

    #[test]
    fn a_silent_peer_makes_recv_wait_out_its_deadline() {
        let clock = FakeClock::default();
        let start = clock.now();
        let mut transport = transport(vec![Step::Expect(b"never".to_vec())], &clock);
        let mut buffer = BytesMut::new();
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 7));
        assert_eq!(error.kind(), ErrorKind::Timeout, "nothing to deliver");
        assert_eq!(
            clock.now(),
            start + 7 * SECOND,
            "waited exactly to the deadline"
        );
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 1));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the transport stayed usable"
        );
    }

    #[test]
    fn respond_answers_each_frame_and_silence_times_out() {
        let clock = FakeClock::default();
        let mut turns = 0_u8;
        let mut transport = transport(
            vec![Step::respond(move |request| {
                turns += 1;
                (turns != 2).then(|| request.iter().map(|byte| byte.wrapping_add(1)).collect())
            })],
            &clock,
        );
        let mut buffer = BytesMut::new();
        send(&clock, &mut transport, &[1, 2]).expect("first");
        recv(&clock, &mut transport, &mut buffer, 1).expect("first answer");
        assert_eq!(&buffer[..], &[2, 3], "answered");
        buffer.clear();
        send(&clock, &mut transport, &[9]).expect("second");
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 1));
        assert_eq!(
            error.kind(),
            ErrorKind::Timeout,
            "the responder stayed silent"
        );
        send(&clock, &mut transport, &[4]).expect("third");
        recv(&clock, &mut transport, &mut buffer, 1).expect("third answer");
        assert_eq!(&buffer[..], &[5], "answered again");
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (strict script failure)
    #[test]
    fn a_strict_mismatch_is_reported_and_the_peer_then_closes() {
        let clock = FakeClock::default();
        let mut transport = FakeTransport::new(
            Script::strict(vec![
                Step::Expect(b"abc".to_vec()),
                Step::Reply(b"never".to_vec()),
            ]),
            clock.clone(),
            limits(64, 128),
        );
        let probe = transport.probe();
        send(&clock, &mut transport, b"abd").expect("the send itself succeeds");
        let report = probe.report();
        assert!(report.failed && report.closed, "script failed");
        assert_eq!(
            report.mismatches,
            vec![crate::Mismatch {
                step: 0,
                expected: b"abc".to_vec(),
                actual: b"abd".to_vec()
            }],
            "mismatch recorded"
        );
        let mut buffer = BytesMut::new();
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 1).expect("eof"),
            0,
            "closed"
        );
    }

    #[test]
    fn shutdown_is_idempotent_closes_the_transport_and_honors_the_deadline() {
        let clock = FakeClock::default();
        let mut transport = transport(Vec::new(), &clock);
        let probe = transport.probe();
        let elapsed = Deadline::at(clock.now());
        let error = error_of(block_on_with(&clock, transport.shutdown(elapsed)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "elapsed deadline");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Close),
            "shaped like TcpTransport"
        );
        assert!(!probe.is_shut_down(), "left open");
        block_on_with(&clock, transport.shutdown(in_secs(&clock, 1))).expect("shutdown");
        assert!(probe.is_shut_down(), "closed");
        block_on_with(&clock, transport.shutdown(elapsed))
            .expect("repeat succeeds even when elapsed");
        let error = error_of(send(&clock, &mut transport, b"x"));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "send after shutdown");
        let mut buffer = BytesMut::new();
        let error = error_of(recv(&clock, &mut transport, &mut buffer, 1));
        assert_eq!(error.kind(), ErrorKind::Disconnected, "recv after shutdown");
    }

    #[test]
    fn an_elapsed_deadline_fails_recv_before_anything_is_served() {
        let clock = FakeClock::default();
        let mut transport = transport(vec![Step::Reply(b"r".to_vec())], &clock);
        let mut buffer = BytesMut::new();
        let elapsed = Deadline::at(clock.now());
        let error = error_of(block_on_with(&clock, transport.recv(&mut buffer, elapsed)));
        assert_eq!(error.kind(), ErrorKind::Timeout, "elapsed deadline");
        assert_eq!(
            recv(&clock, &mut transport, &mut buffer, 1).expect("still queued"),
            1,
            "reply kept"
        );
    }

    #[test]
    fn peer_and_limits_report_what_the_transport_was_built_with() {
        let clock = FakeClock::default();
        let wanted = limits(8, 16);
        let default_peer = FakeTransport::new(Script::default(), clock.clone(), wanted);
        assert_eq!(
            default_peer.peer(),
            &Endpoint::Custom("fake://transport".to_owned()),
            "default peer"
        );
        assert_eq!(default_peer.limits(), &wanted, "limits");
        let named = Endpoint::Custom("fake://plc".to_owned());
        let transport = default_peer.with_peer(named.clone());
        assert_eq!(transport.peer(), &named, "named peer");
    }

    #[test]
    fn errors_carry_transport_diagnostics_without_an_invented_operation() {
        let error = failure(ErrorKind::Timeout, None, "fake receive", "late");
        assert_eq!(error.to_string(), "fake receive: late", "message");
        assert_eq!(error.diagnostics().operation, None, "no operation");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
    }
}
