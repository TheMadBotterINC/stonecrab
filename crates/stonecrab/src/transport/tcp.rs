use std::io;
use std::net::SocketAddr;

use crate::{Deadline, Endpoint, ErrorKind, OpFuture, ResourceLimits, Result};
use bytes::{BufMut, Bytes, BytesMut};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, lookup_host};

use crate::transport::deadline::{Site, ensure_before, within};
use crate::transport::{Transport, TransportFactory};

/// A [`Transport`] over one Tokio TCP connection.
///
/// Made by [`TcpFactory::open`]; there is no public constructor. The transport owns the
/// socket (one owner, no clones) and performs no background work: nothing runs between
/// calls, a peer that closes is noticed by the next `recv` or `send`, and no task is
/// spawned (decision D15). Dropping it closes the socket without a graceful shutdown.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-DEADLINES
/// Every call takes a `Deadline` and races it against the Tokio timer. No upstream parity
/// claim is made (upstream uses Netty).
///
/// # Examples
/// See [`TcpFactory`] for a complete exchange over a loopback listener.
#[derive(Debug)]
pub struct TcpTransport {
    stream: TcpStream,
    peer: Endpoint,
    limits: ResourceLimits,
    /// False once the peer closed, a write was cut short or a read failed.
    usable: bool,
    /// True once `shutdown` has been called, so a repeat is a no-op.
    shut_down: bool,
}

impl TcpTransport {
    const fn new(stream: TcpStream, peer: Endpoint, limits: ResourceLimits) -> Self {
        Self {
            stream,
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

impl Transport for TcpTransport {
    /// Writes `bytes` to the socket within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `bytes` fits in `max_frame_bytes`. The deadline is the
    /// `deadline` argument.
    ///
    /// # Cancel safety
    /// Not cancel-safe: a prefix of `bytes` may have been transmitted when the future is
    /// dropped or the deadline passes, so the transport marks itself unusable first and
    /// only restores that once the whole frame was written.
    ///
    /// # Wire effects
    /// Transmits `bytes`; on timeout or cancellation the peer may hold any prefix of it.
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            ensure_before(deadline, Site::SEND)?;
            self.ensure_usable(Site::SEND)?;
            let limit = self.limits.max_frame_bytes().get();
            if bytes.len() > limit {
                let detail = format!(
                    "frame of {} bytes exceeds the {limit} byte frame limit",
                    bytes.len()
                );
                return Err(Site::SEND.error(ErrorKind::ResourceLimit, &detail));
            }
            self.usable = false;
            within(deadline, Site::SEND, self.stream.write_all(&bytes)).await?;
            self.usable = true;
            Ok(())
        })
    }

    /// Reads whatever has arrived into the free space of `buffer`, within `deadline`.
    ///
    /// Checks run in this order, each before any I/O: the deadline has not passed, the
    /// transport is usable, `buffer` is below `max_buffered_bytes`. The deadline is the
    /// `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: `read_buf` consumes nothing from the socket until it completes, so a
    /// timeout or drop loses no bytes and the transport stays usable.
    ///
    /// # Wire effects
    /// None.
    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            ensure_before(deadline, Site::RECV)?;
            self.ensure_usable(Site::RECV)?;
            let bound = self.limits.max_buffered_bytes().get();
            let free = bound.saturating_sub(buffer.len());
            if free == 0 {
                let detail = format!("receive buffer already holds the {bound} byte limit");
                return Err(Site::RECV.error(ErrorKind::ResourceLimit, &detail));
            }
            // `Limit` caps the writable window at the free space; `BytesMut` alone may
            // offer more capacity than was asked for.
            let mut window = (&mut *buffer).limit(free);
            match within(deadline, Site::RECV, self.stream.read_buf(&mut window)).await {
                Ok(0) => {
                    self.usable = false;
                    Ok(0)
                }
                Ok(read) => Ok(read),
                Err(error) => {
                    // A deadline expiry is the only `Timeout` here and loses nothing.
                    self.usable &= error.kind() == ErrorKind::Timeout;
                    Err(error)
                }
            }
        })
    }

    /// Half-closes the socket within `deadline`.
    ///
    /// The transport is closed from the moment this is polled, whether or not the
    /// half-close completes; a repeat call returns `Ok(())` without I/O, even with an elapsed
    /// deadline. The deadline is
    /// the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe: nothing application-level is sent, and the transport is closed from
    /// the first poll.
    ///
    /// # Wire effects
    /// Sends a TCP FIN; no application bytes.
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.shut_down {
                return Ok(());
            }
            ensure_before(deadline, Site::SHUTDOWN)?;
            self.shut_down = true;
            self.usable = false;
            within(deadline, Site::SHUTDOWN, self.stream.shutdown()).await
        })
    }

    fn peer(&self) -> &Endpoint {
        &self.peer
    }

    fn limits(&self) -> &ResourceLimits {
        &self.limits
    }
}

/// Opens [`TcpTransport`]s.
///
/// `open` resolves `Endpoint::Tcp { host, port }` with `tokio::net::lookup_host` and tries
/// the resolved addresses in order, all inside the one connect deadline, so a host with
/// several addresses cannot take longer than the deadline in total. `Endpoint::Custom` is
/// rejected. A refused or reset connection is `Disconnected`; a deadline that passes is
/// `Timeout`.
///
/// `nodelay` sets `TCP_NODELAY` on every opened socket. It defaults to `true`: Modbus
/// requests are small and the transport never batches writes, so Nagle's algorithm would
/// only add latency. Build one with `TcpFactory::default()` and assign the field; the
/// struct is `#[non_exhaustive]`.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// Upstream opens its TCP channel through Netty with `TCP_NODELAY` on; there is no upstream
/// parity claim for resolution order or error mapping.
///
/// # Examples
/// A complete exchange against a loopback listener, bounded by a deadline.
/// ```
/// use std::time::{Duration, Instant};
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::{Deadline, Endpoint, ResourceLimits};
/// use stonecrab::transport::{TcpFactory, TransportFactory};
/// use tokio::io::{AsyncReadExt, AsyncWriteExt};
/// use tokio::net::TcpListener;
///
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(async {
///     let listener = TcpListener::bind("127.0.0.1:0").await?;
///     let endpoint = Endpoint::Tcp {
///         host: "127.0.0.1".to_owned(),
///         port: listener.local_addr()?.port(),
///     };
///     let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;
///
///     let factory = TcpFactory::default();
///     let mut transport = factory.open(&endpoint, ResourceLimits::builder().build()?, deadline).await?;
///     let (mut device, _) = listener.accept().await?;
///
///     transport.send(Bytes::from_static(b"ping"), deadline).await?;
///     let mut request = [0_u8; 4];
///     device.read_exact(&mut request).await?;
///     assert_eq!(&request, b"ping");
///
///     device.write_all(b"pong").await?;
///     let mut reply = BytesMut::new();
///     let read = transport.recv(&mut reply, deadline).await?;
///     assert_eq!((read, &reply[..]), (4, &b"pong"[..]));
///
///     transport.shutdown(deadline).await?;
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct TcpFactory {
    /// Whether to set `TCP_NODELAY` on opened sockets. Default `true`.
    pub nodelay: bool,
}

impl Default for TcpFactory {
    fn default() -> Self {
        Self { nodelay: true }
    }
}

/// Connect to the first address that accepts, trying them in order.
///
/// Returns the last failure if every address fails.
async fn connect_first(addresses: impl IntoIterator<Item = SocketAddr>) -> io::Result<TcpStream> {
    let mut last = None;
    for address in addresses {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) => last = Some(error),
        }
    }
    Err(last
        .unwrap_or_else(|| io::Error::new(io::ErrorKind::NotFound, "name resolved to no address")))
}

/// Resolve `host:port` and connect, as one future so a single deadline covers both.
async fn resolve_and_connect(host: &str, port: u16) -> io::Result<TcpStream> {
    connect_first(lookup_host((host, port)).await?).await
}

impl TcpFactory {
    /// The body of `open`, returning the concrete transport so tests can inspect it.
    async fn open_tcp(
        &self,
        endpoint: &Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> Result<TcpTransport> {
        let Endpoint::Tcp { host, port } = endpoint else {
            return Err(Site::CONNECT.error(
                ErrorKind::InvalidConfig,
                "the tcp transport needs an Endpoint::Tcp",
            ));
        };
        let stream = within(deadline, Site::CONNECT, resolve_and_connect(host, *port)).await?;
        stream
            .set_nodelay(self.nodelay)
            .map_err(|error| Site::CONNECT.io_error(error))?;
        Ok(TcpTransport::new(stream, endpoint.clone(), limits))
    }
}

impl TransportFactory for TcpFactory {
    /// Resolves and connects within `deadline`, then returns a [`TcpTransport`].
    ///
    /// Checks run in this order: the endpoint is `Endpoint::Tcp` (else `InvalidConfig`),
    /// the deadline has not passed, then resolution and connection run under the
    /// deadline, which is the `deadline` argument. Name resolution runs on Tokio's
    /// blocking pool; if the deadline passes during it the call returns `Timeout` at once
    /// and the resolver thread finishes in the background without effect.
    ///
    /// # Cancel safety
    /// Cancel-safe: a half-open socket is closed when the future is dropped.
    ///
    /// # Wire effects
    /// A TCP handshake with each address tried; no application bytes.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let transport: Box<dyn Transport> =
                Box::new(self.open_tcp(endpoint, limits, deadline).await?);
            Ok(transport)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::net::TcpListener;

    use super::*;
    use crate::transport::deadline::now;

    fn soon() -> Deadline {
        Deadline::after(now(), Duration::from_secs(5)).expect("representable")
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (refused connect, address order)
    #[tokio::test]
    async fn connect_first_skips_a_refused_address_and_takes_the_next() {
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
            listener.local_addr().expect("address")
        };
        let live = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let live_address = live.local_addr().expect("address");
        let stream = within(soon(), Site::CONNECT, connect_first([closed, live_address]))
            .await
            .expect("second address accepts");
        assert_eq!(
            stream.peer_addr().expect("peer"),
            live_address,
            "connected to the live address"
        );

        let error = within(soon(), Site::CONNECT, connect_first([closed]))
            .await
            .expect_err("only a closed port");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "refused connect");

        let error = within(soon(), Site::CONNECT, connect_first([]))
            .await
            .expect_err("no addresses");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "empty resolution");
    }

    #[tokio::test]
    async fn nodelay_follows_the_factory_setting() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let endpoint = Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: listener.local_addr().expect("address").port(),
        };
        let limits = ResourceLimits::builder().build().expect("default limits");
        assert!(TcpFactory::default().nodelay, "nodelay defaults to true");
        for nodelay in [true, false] {
            let factory = TcpFactory { nodelay };
            let transport = factory
                .open_tcp(&endpoint, limits, soon())
                .await
                .expect("open");
            assert_eq!(
                transport.stream.nodelay().expect("option"),
                nodelay,
                "TCP_NODELAY"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (reset while sending)
    #[tokio::test]
    async fn a_write_failing_on_a_reset_connection_is_disconnected_and_poisons() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let endpoint = Endpoint::Tcp {
            host: "127.0.0.1".to_owned(),
            port: listener.local_addr().expect("address").port(),
        };
        let limits = ResourceLimits::builder().build().expect("default limits");
        let mut transport = TcpFactory::default()
            .open_tcp(&endpoint, limits, soon())
            .await
            .expect("open");
        let (device, _) = listener.accept().await.expect("accept");
        device.set_zero_linger().expect("linger 0");
        drop(device);
        // The reset has reached the client socket once it reports readable; only then is
        // the failing write deterministic.
        transport.stream.readable().await.expect("reset observed");

        let error = transport
            .send(Bytes::from_static(b"after the reset"), soon())
            .await
            .expect_err("the write fails");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "reset while writing");
        let error = transport
            .send(Bytes::from_static(b"again"), soon())
            .await
            .expect_err("poisoned");
        assert_eq!(
            error.kind(),
            ErrorKind::Disconnected,
            "unusable after a failed write"
        );
    }
}
