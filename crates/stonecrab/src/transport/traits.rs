use crate::{Deadline, Endpoint, OpFuture, ResourceLimits};
use bytes::{Bytes, BytesMut};

/// A connected, deadline-bounded byte pipe to one peer.
///
/// A transport moves bytes and nothing else: it does not frame, parse or retry. Framers
/// and sessions are built on top of it (the Modbus `MbapFramer`, `ConnectionGuard`).
/// Every method that waits takes the `Deadline` it must finish by; the transport never
/// substitutes a default, so there is no unbounded wait (coding standards section 8.5). A
/// deadline that has already passed fails with [`ErrorKind::Timeout`] before any I/O.
///
/// The trait is object safe and `Send`: a [`Box<dyn Transport>`](TransportFactory::open)
/// can be moved between threads, and one transport has exactly one owner (`&mut self`),
/// so requests on it are serialized by construction (coding standards section 8.9).
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-DEADLINES
/// Upstream transports are Netty channels with a read timeout handler; this trait is a
/// Stonecrab contract with no upstream parity claim.
///
/// # Examples
/// A function over any transport; [`TcpFactory`](crate::transport::TcpFactory) shows one running.
/// ```
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::{Deadline, Result};
/// use stonecrab::transport::Transport;
///
/// /// Send one request and read whatever the peer answers with in one `recv`.
/// async fn ask(
///     transport: &mut dyn Transport,
///     request: Bytes,
///     deadline: Deadline,
/// ) -> Result<BytesMut> {
///     transport.send(request, deadline).await?;
///     let mut reply = BytesMut::new();
///     transport.recv(&mut reply, deadline).await?;
///     Ok(reply)
/// }
/// # let _ = ask;
/// ```
///
/// [`ErrorKind::Timeout`]: crate::ErrorKind::Timeout
pub trait Transport: Send {
    /// Write one frame to the peer.
    ///
    /// `bytes` is one frame: a payload longer than `limits().max_frame_bytes()` is refused
    /// without sending anything. The frame is written completely or the transport is
    /// poisoned (see below). The deadline is the `deadline` argument.
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) if `deadline` has
    ///   already passed (nothing is sent) or passes while writing.
    /// - [`ErrorKind::ResourceLimit`](crate::ErrorKind::ResourceLimit) if `bytes`
    ///   is longer than the frame limit (nothing is sent).
    /// - [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) if the
    ///   transport was shut down, saw the peer close, or was poisoned by an earlier
    ///   failure, or if the write fails (reset, broken pipe).
    ///
    /// # Cancel safety
    /// Not cancel-safe: part of the frame may already have been transmitted when the
    /// future is dropped or the deadline passes, so the byte stream is out of step with
    /// the peer's framing. The transport marks itself unusable at that point and every
    /// later call fails with `Disconnected`; the owner must reopen the connection
    /// (decision D15) rather than resynchronize.
    ///
    /// # Wire effects
    /// Transmits `bytes` in one or more TCP segments. If the call times out or is
    /// cancelled the peer may have received a prefix of the frame, up to all of it; the
    /// caller cannot tell which, so a write request in flight is an unknown outcome.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()>;

    /// Append received bytes to `buffer` and return how many were appended.
    ///
    /// At most `limits().max_buffered_bytes()` minus `buffer.len()` bytes are read, so
    /// `buffer` never grows past the bound. Whatever has arrived (at least one byte, at
    /// most the free space) is returned by one call: a frame may take several calls
    /// (fragmentation) and one call may return several frames (coalescing). A return of
    /// `0` means the peer closed its side; the transport is then unusable and the next
    /// call fails with `Disconnected`. The deadline is the `deadline` argument.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-BUFFER-BOUND
    /// Upstream bounds frames in its Netty decoders; the buffered-bytes bound here is a
    /// Stonecrab contract.
    ///
    /// # Errors
    /// - [`ErrorKind::ResourceLimit`](crate::ErrorKind::ResourceLimit) if
    ///   `buffer` already holds `max_buffered_bytes` or more; nothing is read.
    /// - [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) if `deadline` has
    ///   already passed or passes before any byte arrives.
    /// - [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) if the
    ///   transport is unusable (shut down, peer closed, poisoned) or the read fails.
    ///
    /// # Cancel safety
    /// Cancel-safe. A dropped or timed-out `recv` consumes nothing from the socket and
    /// leaves `buffer` as it was; the transport stays usable.
    ///
    /// # Wire effects
    /// None. Receiving transmits nothing.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize>;

    /// Close the transport: stop sending and tell the peer.
    ///
    /// After this call every `send` and `recv` fails with `Disconnected`; calling
    /// `shutdown` again succeeds without doing anything. Bytes the peer already sent and
    /// that were not read are discarded when the transport is dropped. The deadline is
    /// the `deadline` argument.
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) if `deadline` has
    ///   already passed (the transport is left open) or passes while closing.
    /// - [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) if the
    ///   operating system reports the connection already gone.
    ///
    /// # Cancel safety
    /// Cancel-safe. Closing sends no application bytes, so no request can be cut short;
    /// once started the transport is considered closed whether or not the future
    /// completes.
    ///
    /// # Wire effects
    /// Half-closes the TCP connection (a FIN), which the peer sees as end of stream. No
    /// application bytes are sent.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()>;

    /// The endpoint this transport was opened for, as given to the factory.
    fn peer(&self) -> &Endpoint;

    /// The budgets this transport enforces: `max_frame_bytes` on `send` and
    /// `max_buffered_bytes` on `recv`.
    fn limits(&self) -> &ResourceLimits;

    /// Whether every `recv` appends exactly one whole datagram, so that bytes a datagram
    /// leaves after its last whole frame can never be completed by the next one.
    ///
    /// The default, `false`, is a byte stream (TCP, a serial line): a frame may arrive over
    /// several `recv` calls. [`UdpTransport`](crate::transport::UdpTransport) returns `true`.
    /// [`FramedTransport`](crate::transport::FramedTransport) then never reads past the end
    /// of a datagram that held part of a frame: it drops those bytes and fails the read with
    /// `MalformedData` instead of joining them to the next datagram, and its `resync` drops
    /// every buffered byte instead of searching them.
    ///
    /// # Compatibility: docs/parity/transport.md#UTR-UDP-RING
    /// Upstream's UDP transport appends datagrams to one byte ring and loses their
    /// boundaries; keeping them is a recorded difference.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Duration, Instant};
    /// use stonecrab::{Deadline, Endpoint, ResourceLimits};
    /// use stonecrab::transport::{TransportFactory, UdpFactory};
    /// use tokio::net::UdpSocket;
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let device = UdpSocket::bind("127.0.0.1:0").await?;
    ///     let endpoint = Endpoint::Custom(device.local_addr()?.to_string());
    ///     let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;
    ///     let limits = ResourceLimits::builder().build()?;
    ///     let transport = UdpFactory::default().open(&endpoint, limits, deadline).await?;
    ///     assert!(transport.delivers_datagrams());
    ///     Ok::<(), Box<dyn std::error::Error>>(())
    /// })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    fn delivers_datagrams(&self) -> bool {
        false
    }
}

/// Opens [`Transport`]s for endpoints of one kind.
///
/// A driver holds an `Arc<dyn TransportFactory>` and calls [`open`](Self::open) for the
/// first connection and again for every request-driven reconnect (decision D15); the
/// factory keeps no connection state, which is why it is `Send + Sync`.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// Upstream builds a Netty bootstrap per connection; no upstream parity claim is made.
///
/// # Examples
/// A reconnect helper over any factory; [`TcpFactory`](crate::transport::TcpFactory) shows one
/// running.
/// ```
/// use stonecrab::{Deadline, Endpoint, ResourceLimits, Result};
/// use stonecrab::transport::{Transport, TransportFactory};
///
/// /// Open a fresh transport, as a request-driven reconnect does.
/// async fn reopen(
///     factory: &dyn TransportFactory,
///     endpoint: &Endpoint,
///     limits: ResourceLimits,
///     deadline: Deadline,
/// ) -> Result<Box<dyn Transport>> {
///     factory.open(endpoint, limits, deadline).await
/// }
/// # let _ = reopen;
/// ```
pub trait TransportFactory: Send + Sync {
    /// Resolve and connect to `endpoint`, then return the open transport.
    ///
    /// `limits` becomes the transport's [`Transport::limits`]. The deadline bounds the
    /// whole open, including name resolution, and is the `deadline` argument; callers
    /// usually build it from `Timeouts::connect`.
    ///
    /// # Errors
    /// - [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) if the
    ///   factory does not handle this kind of endpoint.
    /// - [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) if `deadline` has
    ///   already passed or passes before the connection is established.
    /// - [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) if the peer
    ///   refuses, resets or cannot be reached, or the name does not resolve.
    ///
    /// Failures carry `Diagnostics` with `operation` set to `Connect`.
    ///
    /// # Cancel safety
    /// Cancel-safe. A dropped `open` leaves no transport behind: a half-open socket is
    /// closed when the future is dropped and nothing was sent to the peer.
    ///
    /// # Wire effects
    /// Opens a connection (a TCP handshake for the TCP factory) and sends no application
    /// bytes. Resolving a host name may query the network through the system resolver.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>>;
}
