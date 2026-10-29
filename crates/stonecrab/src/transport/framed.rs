use std::fmt;

use crate::{Deadline, Diagnostics, ErrorKind, FailureCategory, OpFuture, PlcError, Result};
use bytes::{Bytes, BytesMut};

use crate::transport::{FrameError, Framer, Transport};

/// A [`Transport`] plus a [`Framer`]: whole frames in, whole frames out.
///
/// `FramedTransport` owns the boxed transport, the framer, and the receive buffer, which
/// it keeps within `limits().max_buffered_bytes()`. [`next_frame`](Self::next_frame)
/// reads from the transport until the framer yields a frame, so callers see neither
/// fragmentation (a frame arriving in pieces) nor coalescing (two frames arriving
/// together: the second waits in the buffer for the next call).
/// [`send_frame`](Self::send_frame) encodes a frame and sends it as one `Transport::send`.
/// [`resync`](Self::resync) realigns the stream after a framing error or an abandoned
/// request.
///
/// # Generic or boxed
/// The framer is a value of type `R` and the frame type is `F`; a driver holds a
/// concrete `FramedTransport<Adu, MbapFramer>`. Only the byte pipe is a trait object
/// (`Box<dyn Transport>`), so the same framer works over TCP, a serial port or a test
/// double. The type is `Send` when `F` and `R` are.
///
/// # One owner
/// Every method takes `&mut self`, so requests on one connection are serialized by
/// construction (coding standards section 8.9). Whoever owns the value decides when a
/// request is in flight; `ConnectionGuard` is that owner.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
/// Upstream frames inside Netty pipelines; this layer is a Stonecrab contract with no
/// upstream parity claim.
///
/// # Examples
/// Echo-style exchange over the in-memory transport double with a length-prefixed test
/// framer (a 2-byte big-endian length, then the payload):
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline, ResourceLimits};
/// use stonecrab::testkit::{
///     FakeClock, FakeTransport, LengthPrefixedFramer, Script, Step, block_on_with,
/// };
/// use stonecrab::transport::FramedTransport;
///
/// let clock = FakeClock::default();
/// let script = Script::strict(vec![
///     Step::Expect(vec![0, 4, b'p', b'i', b'n', b'g']),
///     Step::ReplyFragmented(vec![vec![0, 4, b'p'], vec![b'o', b'n', b'g']]),
/// ]);
/// let limits = ResourceLimits::builder().build()?;
/// let transport = FakeTransport::new(script, clock.clone(), limits);
/// let mut framed = FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
///
/// block_on_with(&clock, framed.send_frame(&b"ping".to_vec(), deadline))?;
/// let reply = block_on_with(&clock, framed.next_frame(deadline))?;
/// assert_eq!(reply, b"pong");
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct FramedTransport<F, R> {
    transport: Box<dyn Transport>,
    framer: R,
    /// Bytes received and not yet decoded; never longer than `max_buffered_bytes`.
    buffer: BytesMut,
    /// Reused by `send_frame` to encode into.
    scratch: BytesMut,
    /// The frame `resync` recognised, handed out by the next `next_frame`.
    ready: Option<F>,
}

/// Shows the peer and how many bytes are buffered, never the bytes themselves.
impl<F, R> fmt::Debug for FramedTransport<F, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FramedTransport")
            .field("peer", self.transport.peer())
            .field("buffered", &self.buffer.len())
            .field("frame_ready", &self.ready.is_some())
            .finish_non_exhaustive()
    }
}

impl<F: Send, R: Framer<F>> FramedTransport<F, R> {
    /// Frame `transport` with `framer`.
    ///
    /// The receive buffer starts empty and never grows past the transport's
    /// `limits().max_buffered_bytes()`. Nothing is read or sent.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use stonecrab::ResourceLimits;
    /// use stonecrab::testkit::{FakeClock, FakeTransport, LengthPrefixedFramer, Script};
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
    /// let framed: FramedTransport<Vec<u8>, _> =
    ///     FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// assert_eq!(framed.buffered_len(), 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn new(transport: Box<dyn Transport>, framer: R) -> Self {
        Self {
            transport,
            framer,
            buffer: BytesMut::new(),
            scratch: BytesMut::new(),
            ready: None,
        }
    }

    /// The transport underneath, for its `peer()` and `limits()`.
    ///
    /// Shared access only: reading or sending through the transport directly would
    /// bypass this layer's buffer and desynchronize the framing.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use stonecrab::ResourceLimits;
    /// use stonecrab::testkit::{FakeClock, FakeTransport, LengthPrefixedFramer, Script};
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
    /// let framed: FramedTransport<Vec<u8>, _> =
    ///     FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// assert_eq!(framed.transport().limits(), &limits);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn transport(&self) -> &dyn Transport {
        &*self.transport
    }

    /// Bytes received and not yet decoded into a frame.
    ///
    /// Never more than the transport's `max_buffered_bytes`. A frame that
    /// [`resync`](Self::resync) recognised and is holding for the next
    /// [`next_frame`](Self::next_frame) is not counted.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use stonecrab::ResourceLimits;
    /// use stonecrab::testkit::{FakeClock, FakeTransport, LengthPrefixedFramer, Script};
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
    /// let framed: FramedTransport<Vec<u8>, _> =
    ///     FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// assert_eq!(framed.buffered_len(), 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Hold `frame` for the next [`next_frame`](Self::next_frame), which returns it before
    /// anything else. The guard's supervisor uses it to keep a frame it read while nobody
    /// had asked for one.
    pub(crate) fn park(&mut self, frame: F) {
        self.ready = Some(frame);
    }

    /// Whether a frame is held for the next [`next_frame`](Self::next_frame).
    pub(crate) const fn parked(&self) -> bool {
        self.ready.is_some()
    }

    /// Drop the frame held for the next [`next_frame`](Self::next_frame), if any: one the
    /// guard's supervisor kept, or the one [`resync`](Self::resync) recognised. True if a
    /// frame was dropped, so the guard can count it.
    pub(crate) fn unpark(&mut self) -> bool {
        self.ready.take().is_some()
    }

    /// Shut the transport down where it is, keeping the buffered bytes and a held frame.
    ///
    /// The guard's idle bound uses it so that the transport stays in the guard's in-flight
    /// slot until its shutdown has returned; [`into_inner`](Self::into_inner) followed by
    /// `shutdown` would take it out first. Errors and cancel safety are the transport's
    /// [`Transport::shutdown`].
    pub(crate) fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        self.transport.shutdown(deadline)
    }

    /// Receive once more into the buffer, without decoding, within `deadline`. The guard's
    /// supervisor uses it behind a parked frame, to notice a peer close.
    ///
    /// Fails `Disconnected` at end of stream and `ResourceLimit` when the buffer is
    /// already at its bound, as a read for a frame does.
    pub(crate) async fn listen(&mut self, deadline: Deadline) -> Result<()> {
        self.fill(deadline).await
    }

    /// Give back the transport, dropping the framer, the buffered bytes and any frame
    /// held for the next `next_frame`.
    ///
    /// Use it to `shutdown` the transport, or to reopen with a different framer. Bytes
    /// that were received but not yet decoded are lost.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use stonecrab::ResourceLimits;
    /// use stonecrab::testkit::{FakeClock, FakeTransport, LengthPrefixedFramer, Script};
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), FakeClock::default(), limits);
    /// let framed: FramedTransport<Vec<u8>, _> =
    ///     FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// let transport = framed.into_inner();
    /// assert_eq!(transport.limits(), &limits);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn into_inner(self) -> Box<dyn Transport> {
        self.transport
    }

    /// Receive until the framer yields one whole frame, and return it.
    ///
    /// Frames already in the buffer are returned first without any I/O, even if
    /// `deadline` has passed (a coalesced delivery: the second frame costs no read). Only
    /// when the buffer holds no whole frame does the call read, appending to the buffer
    /// until the framer is satisfied. The deadline is the `deadline` argument: build it
    /// from the request's remaining deadline.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if `deadline` has passed (or passes) before a whole frame
    ///   is buffered; the bytes received so far stay buffered, so a later call with a
    ///   new deadline continues the same frame.
    /// - [`ErrorKind::Disconnected`] if the peer closed (a 0-byte read), whether the
    ///   buffer held a partial frame or nothing, or the transport is unusable.
    /// - [`ErrorKind::ResourceLimit`] if the framer declares a frame over its maximum
    ///   (checked as soon as the header is in, before the payload is awaited), or if the
    ///   buffer reaches `max_buffered_bytes` without completing a frame. Nothing is
    ///   discarded; call [`resync`](Self::resync) to move past the bytes.
    /// - [`ErrorKind::MalformedData`] if the framer rejects the header. Nothing is
    ///   discarded and the same error is returned until [`resync`](Self::resync).
    ///
    /// # Cancel safety
    /// Cancel-safe. A dropped or timed-out call loses nothing: received bytes are already
    /// in the internal buffer and the underlying `recv` is cancel-safe, so the next call
    /// carries on with the same frame.
    ///
    /// # Wire effects
    /// None. Receiving transmits nothing.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::time::Duration;
    /// use stonecrab::{Clock, Deadline, ResourceLimits};
    /// use stonecrab::testkit::{
    ///     FakeClock, FakeTransport, LengthPrefixedFramer, Script, Step, block_on_with,
    /// };
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let clock = FakeClock::default();
    /// let script = Script::new(vec![
    ///     Step::ReplyCoalescedWithNext,
    ///     Step::Reply(vec![0, 1, b'a']),
    ///     Step::Reply(vec![0, 1, b'b']),
    /// ]);
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(script, clock.clone(), limits);
    /// let mut framed = FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
    ///
    /// // Both frames arrived in one read; the second call needs no I/O.
    /// assert_eq!(block_on_with(&clock, framed.next_frame(deadline))?, b"a");
    /// assert_eq!(block_on_with(&clock, framed.next_frame(deadline))?, b"b");
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn next_frame(&mut self, deadline: Deadline) -> OpFuture<'_, F> {
        Box::pin(async move {
            if let Some(frame) = self.ready.take() {
                return Ok(frame);
            }
            loop {
                if let Some(frame) = self
                    .framer
                    .decode(&mut self.buffer)
                    .map_err(frame_failure)?
                {
                    return Ok(frame);
                }
                self.fill(deadline).await?;
            }
        })
    }

    /// Encode `frame` and send it as one transport frame.
    ///
    /// The frame is encoded into a scratch buffer when this method is called (pure, no
    /// I/O) and the bytes are sent when the returned future is polled. The encoded size
    /// must fit the smaller of the framer's `max_frame_bytes` and the transport's
    /// `limits().max_frame_bytes()`; otherwise nothing is sent. The deadline is the
    /// `deadline` argument: build it from the request's remaining deadline.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// - [`ErrorKind::ResourceLimit`] if the frame encodes to more than the frame limit
    ///   (nothing is sent), or if the framer reports it oversized.
    /// - [`ErrorKind::MalformedData`] if the framer cannot encode `frame`.
    /// - [`ErrorKind::Timeout`] if `deadline` has already passed (nothing is sent) or
    ///   passes while writing.
    /// - [`ErrorKind::Disconnected`] if the transport is unusable or the write fails.
    ///
    /// # Cancel safety
    /// Not cancel-safe, as `Transport::send` is not: a dropped or timed-out future may
    /// have put a prefix of the frame on the wire, after which the transport is poisoned
    /// and every later call fails with `Disconnected`. Reopen the connection (decision
    /// D15); do not `resync`, which only realigns the receive side.
    ///
    /// # Wire effects
    /// Transmits the encoded frame in one `Transport::send`. On timeout or cancellation
    /// the peer may have received a prefix of it, up to all of it, so a write in flight
    /// is an unknown outcome.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::time::Duration;
    /// use stonecrab::{Clock, Deadline, ResourceLimits};
    /// use stonecrab::testkit::{
    ///     FakeClock, FakeTransport, LengthPrefixedFramer, Script, block_on_with,
    /// };
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(Script::default(), clock.clone(), limits);
    /// let probe = transport.probe();
    /// let mut framed = FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(64));
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
    ///
    /// block_on_with(&clock, framed.send_frame(&b"hi".to_vec(), deadline))?;
    /// assert_eq!(probe.sent()[0].as_ref(), &[0, 2, b'h', b'i']);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn send_frame(&mut self, frame: &F, deadline: Deadline) -> OpFuture<'_, ()> {
        let encoded = self.encode(frame);
        Box::pin(async move { self.transport.send(encoded?, deadline).await })
    }

    /// Realign the receive stream on a frame boundary.
    ///
    /// Call it after a framing error from [`next_frame`](Self::next_frame) (`Malformed`,
    /// `Oversized`, a full buffer) or when a request was abandoned and its reply may
    /// still be on the way. `resync` discards received bytes from the front of the
    /// buffer, one byte at a time, for as long as the framer rejects what is there, and
    /// keeps receiving when the buffer runs out.
    ///
    /// # The boundary rule
    /// Each step asks the framer to `decode` the front of the buffer:
    /// - `Ok(Some(frame))`: a boundary. The frame is recognised and held for the next
    ///   [`next_frame`](Self::next_frame), which returns it without I/O; `resync` returns.
    /// - `Ok(None)` with room left in the buffer: the front is a header the framer
    ///   accepts, or too short to judge; the frame is not complete yet. Receive more and
    ///   ask again. A prefix the framer accepts is kept, not discarded, so a frame that
    ///   was half received when its request timed out is completed rather than skipped.
    /// - `Err(_)`, or `Ok(None)` with a full buffer: not a frame start. Discard one byte
    ///   and ask again.
    ///
    /// A framer whose [`Framer::resync_discards`] is true (Modbus RTU, where only silence
    /// separates frames) is not searched: every buffered byte is dropped, nothing is
    /// received and the call returns at once with no frame held, so the next
    /// [`next_frame`](Self::next_frame) reads what arrives after it.
    ///
    /// A frame held by an earlier `resync` is dropped first. A framer with no sync marker
    /// (a bare length prefix) can lock onto a false header in garbage; `resync` is best
    /// effort for such protocols, and a caller that gets a wrong frame afterwards (a
    /// transaction id that does not match) reopens the connection. A late reply that is
    /// itself a whole valid frame is delivered by the next `next_frame`; the owner discards
    /// it by correlation.
    ///
    /// # The deadline
    /// `resync` receives, so it needs a deadline. The `deadline` argument is the only
    /// source, and the caller chooses it by context (decision D15):
    /// - when a request's timeout triggered the resync, the remaining deadline of that
    ///   request;
    /// - when it runs from `ConnectionGuard::close` or from recovery, with no request
    ///   deadline, `Deadline::after(clock.now(), timeouts.request)`.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-RESYNC
    /// No upstream parity claim: upstream's Netty decoders drop the connection on a
    /// framing error rather than resynchronize.
    ///
    /// # Errors
    /// - [`ErrorKind::Disconnected`] if the deadline passes (or has passed) without a
    ///   frame boundary, with the transport's `Timeout` as the error's source: the stream
    ///   cannot be trusted and the owner reopens the connection. Also if the peer closes
    ///   or the transport is unusable.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future leaves the buffer holding only bytes the framer
    /// has not rejected, so calling `resync` again continues. Bytes already discarded are
    /// not restored.
    ///
    /// # Wire effects
    /// None. Receiving transmits nothing; the discarded bytes are never forwarded.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::time::Duration;
    /// use stonecrab::{Clock, Deadline, ErrorKind, ResourceLimits};
    /// use stonecrab::testkit::{
    ///     FakeClock, FakeTransport, LengthPrefixedFramer, Script, Step, block_on_with,
    /// };
    /// use stonecrab::transport::FramedTransport;
    ///
    /// let clock = FakeClock::default();
    /// // Four bytes of garbage (a length of 65535 is over the 16 byte frame maximum),
    /// // then a good frame.
    /// let script = Script::new(vec![
    ///     Step::Reply(vec![0xFF, 0xFF, 0xFF, 0xFF]),
    ///     Step::Reply(vec![0, 2, b'o', b'k']),
    /// ]);
    /// let limits = ResourceLimits::builder().build()?;
    /// let transport = FakeTransport::new(script, clock.clone(), limits);
    /// let mut framed = FramedTransport::new(Box::new(transport), LengthPrefixedFramer::new(16));
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
    ///
    /// let error = block_on_with(&clock, framed.next_frame(deadline)).unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    /// block_on_with(&clock, framed.resync(deadline))?;
    /// assert_eq!(block_on_with(&clock, framed.next_frame(deadline))?, b"ok");
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn resync(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            self.ready = None;
            if self.framer.resync_discards() {
                self.buffer.clear();
                return Ok(());
            }
            loop {
                let bound = self.transport.limits().max_buffered_bytes().get();
                let discarded = match self.framer.decode(&mut self.buffer) {
                    Ok(Some(frame)) => {
                        self.ready = Some(frame);
                        return Ok(());
                    }
                    Ok(None) if self.buffer.len() < bound => false,
                    Ok(None) | Err(_) => self.discard_front(),
                };
                if !discarded {
                    self.fill(deadline).await.map_err(no_boundary)?;
                }
            }
        })
    }

    /// Append one `recv` to the buffer, refusing to grow it past the bound.
    async fn fill(&mut self, deadline: Deadline) -> Result<()> {
        let bound = self.transport.limits().max_buffered_bytes().get();
        if self.buffer.len() >= bound {
            return Err(failure(
                ErrorKind::ResourceLimit,
                &format!("receive buffer holds {bound} bytes without a complete frame"),
            ));
        }
        if self.transport.recv(&mut self.buffer, deadline).await? == 0 {
            return Err(failure(
                ErrorKind::Disconnected,
                &format!(
                    "peer closed the connection with {} undecoded bytes buffered",
                    self.buffer.len()
                ),
            ));
        }
        Ok(())
    }

    /// Drop the first buffered byte; false if there was none.
    fn discard_front(&mut self) -> bool {
        let dropped = self.buffer.len().min(1);
        drop(self.buffer.split_to(dropped));
        dropped > 0
    }

    /// Encode `frame` and check it against the frame limits.
    fn encode(&mut self, frame: &F) -> Result<Bytes> {
        self.scratch.clear();
        self.framer
            .encode(frame, &mut self.scratch)
            .map_err(frame_failure)?;
        let limit = self
            .transport
            .limits()
            .max_frame_bytes()
            .get()
            .min(self.framer.max_frame_bytes());
        if self.scratch.len() > limit {
            return Err(frame_failure(FrameError::Oversized(self.scratch.len())));
        }
        Ok(self.scratch.split().freeze())
    }
}

/// A transport-category failure raised by the framing layer itself.
fn failure(kind: ErrorKind, detail: &str) -> PlcError {
    let diagnostics = Diagnostics {
        category: Some(FailureCategory::Transport),
        ..Diagnostics::default()
    };
    PlcError::new(kind, format!("framed transport: {detail}")).with_diagnostics(diagnostics)
}

/// A framer's complaint as a typed error: `Oversized` is a transport-category
/// `ResourceLimit`, the rest a protocol-category `MalformedData`.
fn frame_failure(error: FrameError) -> PlcError {
    let category = match error {
        FrameError::Oversized(_) => FailureCategory::Transport,
        FrameError::Truncated | FrameError::Malformed => FailureCategory::Protocol,
    };
    let diagnostics = Diagnostics {
        category: Some(category),
        ..Diagnostics::default()
    };
    PlcError::from(error).with_diagnostics(diagnostics)
}

/// A resync that ran out of time has found no boundary: the stream is lost.
fn no_boundary(error: PlcError) -> PlcError {
    if error.kind() != ErrorKind::Timeout {
        return error;
    }
    let diagnostics = Diagnostics {
        category: Some(FailureCategory::Transport),
        ..Diagnostics::default()
    };
    PlcError::new(
        ErrorKind::Disconnected,
        "framed transport: no frame boundary before the deadline; reopen the connection",
    )
    .with_diagnostics(diagnostics)
    .with_source(error)
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: SPEC mandatory-failure-cases / F2 transport (framing error mapping)
    #[test]
    fn framer_complaints_map_to_kinds_and_categories() {
        let cases = [
            (
                FrameError::Oversized(9),
                ErrorKind::ResourceLimit,
                FailureCategory::Transport,
            ),
            (
                FrameError::Malformed,
                ErrorKind::MalformedData,
                FailureCategory::Protocol,
            ),
            (
                FrameError::Truncated,
                ErrorKind::MalformedData,
                FailureCategory::Protocol,
            ),
        ];
        for (frame_error, kind, category) in cases {
            let error = frame_failure(frame_error);
            assert_eq!(error.kind(), kind, "kind for {frame_error:?}");
            assert_eq!(
                error.diagnostics().category,
                Some(category),
                "category for {frame_error:?}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 resynchronization
    #[test]
    fn only_a_timeout_becomes_the_resync_failure() {
        let other = no_boundary(PlcError::new(ErrorKind::Disconnected, "peer reset"));
        assert_eq!(other.kind(), ErrorKind::Disconnected, "kept");
        assert_eq!(other.to_string(), "peer reset", "untouched message");
    }
}
