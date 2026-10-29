use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use crate::{Deadline, Endpoint, ErrorKind, OpFuture, PlcError, ResourceLimits, Result};
use bytes::{BufMut, Bytes, BytesMut};
use tokio::net::{UdpSocket, lookup_host};

use crate::transport::deadline::{Site, ensure_before, within};
use crate::transport::{Transport, TransportFactory};

/// The largest datagram `send` offers: the largest UDP payload over IPv4, which is also
/// upstream's `max-packet-size` default.
const SEND_BOUND: usize = 65_507;

/// No UDP payload is longer: the IP and UDP length fields are 16 bits.
const LARGEST_DATAGRAM: usize = 65_535;

/// A [`Transport`] over one connected UDP socket: one datagram per call.
///
/// Made by [`UdpFactory::open`]; there is no public constructor. The socket is connected to
/// the one peer the endpoint names, so the operating system delivers datagrams from that
/// address and port only: a datagram from any other address is never seen. `send` sends one
/// datagram and `recv` appends one whole datagram, so a boundary is never lost and two
/// datagrams are never merged ([`Transport::delivers_datagrams`] is true). A datagram longer
/// than the free space of the receive buffer is dropped and reported as `ResourceLimit`,
/// never cut short. An empty datagram carries nothing and is skipped, so `recv` never
/// returns `0`: a UDP peer cannot close.
///
/// There is no connection: nothing is sent when the socket opens, and a peer that is not
/// listening is noticed only when the operating system reports an ICMP port-unreachable
/// message for an earlier datagram, as a `Disconnected` failure of a later `send` or `recv`
/// (where ICMP is filtered, the request times out instead). The transport has one owner,
/// performs no background work and spawns no task. Dropping it closes the socket.
///
/// # Compatibility: docs/parity/transport.md#UTR-UDP
/// Upstream's UDP transport accepts datagrams from its configured remote address only, as
/// here; it appends them to a byte ring, which loses the boundaries
/// (docs/parity/transport.md#UTR-UDP-RING), and drops one that does not fit with only a log
/// warning.
///
/// # Examples
/// See [`UdpFactory`] for a complete exchange with a loopback peer.
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
    peer: Endpoint,
    limits: ResourceLimits,
    /// False once a send or a receive failed.
    usable: bool,
    /// True once `shutdown` has been called, so a repeat is a no-op.
    shut_down: bool,
}

impl UdpTransport {
    const fn new(socket: UdpSocket, peer: Endpoint, limits: ResourceLimits) -> Self {
        Self {
            socket,
            peer,
            limits,
            usable: true,
            shut_down: false,
        }
    }

    fn ensure_usable(&self, site: Site) -> Result<()> {
        if self.usable {
            Ok(())
        } else {
            Err(site.error(ErrorKind::Disconnected, "transport is no longer usable"))
        }
    }
}

/// Windows reports a datagram longer than the buffer as an error (`WSAEMSGSIZE`) where
/// other systems cut it to the buffer: both mean that it did not fit.
#[cfg(windows)]
fn too_long(error: &io::Error) -> bool {
    error.raw_os_error() == Some(10_040)
}

/// Windows reports a datagram longer than the buffer as an error (`WSAEMSGSIZE`) where
/// other systems cut it to the buffer: both mean that it did not fit.
#[cfg(not(windows))]
const fn too_long(_error: &io::Error) -> bool {
    false
}

impl Transport for UdpTransport {
    /// Sends `bytes` as one datagram within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `bytes` fits in `max_frame_bytes` and in the 65 507 bytes of the
    /// largest UDP payload over IPv4. The deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: a datagram is sent whole or not at all, so a dropped or timed-out call
    /// leaves nothing half sent and the transport stays usable.
    ///
    /// # Wire effects
    /// Transmits one datagram; whether it arrives is not known.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            ensure_before(deadline, Site::UDP_SEND)?;
            self.ensure_usable(Site::UDP_SEND)?;
            let limit = self.limits.max_frame_bytes().get().min(SEND_BOUND);
            if bytes.len() > limit {
                let detail = format!(
                    "datagram of {} bytes exceeds the {limit} byte limit",
                    bytes.len()
                );
                return Err(Site::UDP_SEND.error(ErrorKind::ResourceLimit, &detail));
            }
            let sent = within(deadline, Site::UDP_SEND, self.socket.send(&bytes)).await;
            // A deadline expiry sent nothing; any other failure is the link's. A datagram goes
            // whole or not at all, so the count is the length.
            self.usable = sent
                .as_ref()
                .map_or_else(|error| error.kind() == ErrorKind::Timeout, |_| true);
            sent.map(|_sent| ())
        })
    }

    /// Appends the next datagram from the peer to `buffer`, within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `buffer` is below `max_buffered_bytes`. One call appends exactly
    /// one datagram, whole. A datagram longer than the free space (`max_buffered_bytes`
    /// minus `buffer.len()`) is consumed, nothing is appended and the call fails with
    /// `ResourceLimit`; the transport stays usable. An empty datagram is skipped. The
    /// deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: the socket hands over a datagram only when the call completes, so a
    /// timeout or drop loses none and the transport stays usable.
    ///
    /// # Wire effects
    /// None.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            ensure_before(deadline, Site::UDP_RECV)?;
            self.ensure_usable(Site::UDP_RECV)?;
            let bound = self.limits.max_buffered_bytes().get();
            let free = bound.saturating_sub(buffer.len());
            if free == 0 {
                let detail = format!("receive buffer already holds the {bound} byte limit");
                return Err(Site::UDP_RECV.error(ErrorKind::ResourceLimit, &detail));
            }
            // One byte more than fits, so that a datagram that fills the window is known
            // not to fit; the space is reserved because the socket cuts a datagram to the
            // space it is given.
            let window = free.min(LARGEST_DATAGRAM).saturating_add(1);
            buffer.reserve(window);
            let start = buffer.len();
            loop {
                let received = {
                    let mut space = (&mut *buffer).limit(window);
                    let datagram = async {
                        self.socket
                            .recv_buf(&mut space)
                            .await
                            .or_else(|error| too_long(&error).then_some(window).ok_or(error))
                    };
                    within(deadline, Site::UDP_RECV, datagram).await
                };
                match received {
                    Ok(0) => {}
                    Ok(read) if read > free => {
                        buffer.truncate(start);
                        let detail = format!(
                            "a datagram longer than the {free} free bytes of the receive buffer \
                             was dropped"
                        );
                        return Err(Site::UDP_RECV.error(ErrorKind::ResourceLimit, &detail));
                    }
                    Ok(read) => return Ok(read),
                    Err(error) => {
                        // A deadline expiry is the only `Timeout` here and loses nothing.
                        self.usable &= error.kind() == ErrorKind::Timeout;
                        return Err(error);
                    }
                }
            }
        })
    }

    /// Closes the transport within `deadline`; nothing is sent, because UDP has no close.
    ///
    /// The transport is closed from the moment this is polled; a repeat call returns
    /// `Ok(())`, even with an elapsed deadline. The socket itself is closed when the
    /// transport is dropped. The deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: nothing is sent, and the transport is closed from the first poll.
    ///
    /// # Wire effects
    /// None.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.shut_down {
                return Ok(());
            }
            ensure_before(deadline, Site::UDP_SHUTDOWN)?;
            self.shut_down = true;
            self.usable = false;
            Ok(())
        })
    }

    fn peer(&self) -> &Endpoint {
        &self.peer
    }

    fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    fn delivers_datagrams(&self) -> bool {
        true
    }
}

/// Opens [`UdpTransport`]s.
///
/// `open` takes an `Endpoint::Custom` whose text is `host:port`, an IPv6 host in brackets
/// (`192.0.2.10:502`, `plc.example:502`, `[2001:db8::10]:502`). The name is resolved with
/// `tokio::net::lookup_host` and the first address is the peer: UDP has no handshake that
/// could refuse an address and move on to the next. A socket of that address's family is
/// bound to an ephemeral port on every interface and connected to the peer, so it accepts
/// datagrams from the peer only. Any other endpoint, or text that is not `host:port`, is
/// `InvalidConfig`; a name that does not resolve is `Disconnected`; a deadline that passes
/// is `Timeout`. Every open makes a new socket with a new port, so a reply sent to an
/// earlier socket never reaches a later one.
///
/// Upstream's options (`local-address`, `local-port`, `max-packet-size`, the buffer sizes,
/// `broadcast`, `reuse-address`, `share-socket`, `multicast-ttl`, `read-timeout-ms`) are not
/// offered: the receive bound is `ResourceLimits::max_buffered_bytes`, the send bound
/// `ResourceLimits::max_frame_bytes`, and every call takes its own `Deadline`. The struct
/// is `#[non_exhaustive]`; build one with `UdpFactory::default()`.
///
/// # Compatibility: docs/parity/transport.md#UTR-UDP-OPTIONS
/// Which upstream options are refused, and why, is recorded there and in ADR-0027.
///
/// # Examples
/// A complete exchange with a loopback peer, bounded by a deadline.
/// ```
/// use std::time::{Duration, Instant};
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::{Deadline, Endpoint, ResourceLimits};
/// use stonecrab::transport::{TransportFactory, UdpFactory};
/// use tokio::net::UdpSocket;
///
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(async {
///     let device = UdpSocket::bind("127.0.0.1:0").await?;
///     let endpoint = Endpoint::Custom(device.local_addr()?.to_string());
///     let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;
///
///     let factory = UdpFactory::default();
///     let mut transport = factory.open(&endpoint, ResourceLimits::builder().build()?, deadline).await?;
///
///     transport.send(Bytes::from_static(b"ping"), deadline).await?;
///     let mut request = [0_u8; 16];
///     let (length, client) = device.recv_from(&mut request).await?;
///     assert_eq!(&request[..length], b"ping");
///
///     device.send_to(b"pong", client).await?;
///     let mut reply = BytesMut::new();
///     let read = transport.recv(&mut reply, deadline).await?;
///     assert_eq!((read, &reply[..]), (4, &b"pong"[..]));
///
///     transport.shutdown(deadline).await?;
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct UdpFactory;

/// An open failure: text that is not `host:port` is a configuration error, anything else
/// a peer that cannot be reached.
fn open_error(error: io::Error) -> PlcError {
    if error.kind() == io::ErrorKind::InvalidInput {
        let detail = error.to_string();
        Site::UDP_CONNECT
            .error(ErrorKind::InvalidConfig, &detail)
            .with_source(error)
    } else {
        Site::UDP_CONNECT.io_error(error)
    }
}

/// Resolve `host:port`, then bind a socket of the first address's family and connect it.
async fn resolve_and_connect(authority: &str) -> io::Result<UdpSocket> {
    let peer = lookup_host(authority)
        .await?
        .next()
        .ok_or(io::ErrorKind::NotFound)?;
    let local = match peer {
        SocketAddr::V4(_) => SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)),
        SocketAddr::V6(_) => SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)),
    };
    let socket = UdpSocket::bind(local).await?;
    socket.connect(peer).await?;
    Ok(socket)
}

impl UdpFactory {
    /// The body of `open`, returning the concrete transport.
    async fn open_udp(
        self,
        endpoint: &Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> Result<UdpTransport> {
        let Endpoint::Custom(authority) = endpoint else {
            return Err(Site::UDP_CONNECT.error(
                ErrorKind::InvalidConfig,
                "the udp transport needs an Endpoint::Custom holding host:port",
            ));
        };
        let connected = within(deadline, Site::UDP_CONNECT, async {
            Ok(resolve_and_connect(authority).await)
        })
        .await?;
        let socket = connected.map_err(open_error)?;
        Ok(UdpTransport::new(socket, endpoint.clone(), limits))
    }
}

impl TransportFactory for UdpFactory {
    /// Resolves the peer and connects a new UDP socket to it within `deadline`, then
    /// returns a [`UdpTransport`].
    ///
    /// Checks run in this order: the endpoint is `Endpoint::Custom` (else `InvalidConfig`),
    /// the deadline has not passed, then resolution and the socket setup run under the
    /// deadline, which is the `deadline` argument. Name resolution runs on Tokio's blocking
    /// pool; if the deadline passes during it the call returns `Timeout` at once and the
    /// resolver thread finishes in the background without effect.
    ///
    /// # Cancel safety
    /// Cancel-safe: a socket made before the drop is closed with the future.
    ///
    /// # Wire effects
    /// None: connecting a UDP socket sends nothing. Resolving a host name may query the
    /// network through the system resolver.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let transport: Box<dyn Transport> =
                Box::new(self.open_udp(endpoint, limits, deadline).await?);
            Ok(transport)
        })
    }
}
