# Stonecrab Transport
Deadline-bounded byte transports for the Stonecrab protocol drivers. A `Transport` sends
and receives bytes to one peer and does nothing else: it does not frame, parse, retry or
reconnect. The crate also holds the `TransportFactory` that opens transports, the
Tokio-backed `SystemClock`, and a small jitter source for reconnect backoff. It depends on
`stonecrab-core` (plus `bytes` and a scoped set of Tokio features) and not on the codec
crate; framing errors are reported with the crate's own `FrameError`.

No upstream parity claim is made: Apache PLC4X builds its transports on Netty, and these
types are Stonecrab's own contract. The behavior that is specified is recorded in the
[transport record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/transport.md).

| Capability | Support level |
| --- | --- |
| `Transport` and `TransportFactory` traits, `FrameError` | build-tested |
| `TcpTransport` and `TcpFactory` (feature `tcp`) | build-tested |
| `SystemClock` (Tokio timer) | build-tested |
| `Jitter` (xorshift64* sample source for `ReconnectPolicy::next_delay`) | build-tested |
| Serial, UDP and TLS transports | planned |

The TCP transport has byte-level tests over real loopback sockets (connect refusal,
fragmented and coalesced delivery, peer close, buffer and frame bounds, shutdown). Those
tests move bytes only; they exercise no protocol, no simulator and no device, so the
support level stays build-tested. Deadline expiry is tested on paused Tokio time against a
future that never completes, not on a socket. Simulator, interoperability and hardware
verification are not claimed.

## Features
- `tcp` (default): `TcpTransport` and `TcpFactory`, and Tokio's `net` feature. Without it
  the crate still builds and exposes `Transport`, `TransportFactory`, `FrameError`,
  `SystemClock` and `Jitter`.

Tokio is used with `rt`, `time`, `sync` and `io-util` always on and `net` behind `tcp`;
the `full` feature is banned by `deny.toml`. Nothing here spawns a task, and a transport
does no work between calls (decision D15 of the alpha plan).

## The deadline rule
Every call that waits takes the `Deadline` it must finish by, and that argument is the
only deadline source: there is no default and no hidden timeout. A deadline that has
already passed fails with `ErrorKind::Timeout` before any I/O. Deadlines are compared with
Tokio's clock, which is the ordinary `std::time::Instant` clock unless a test pauses time;
build them from `SystemClock::now()` and they agree in both cases.

## Limits
`recv` appends only up to `ResourceLimits::max_buffered_bytes` and fails with
`ErrorKind::ResourceLimit` when the buffer is already full. `send` carries one frame and
refuses more than `ResourceLimits::max_frame_bytes`. A `send` that is cut short may have
put a prefix of the frame on the wire, so the transport poisons itself and the owner
reopens it.

```rust
# #[cfg(feature = "tcp")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use std::time::{Duration, Instant};
use bytes::{Bytes, BytesMut};
use stonecrab_core::{Deadline, Endpoint, ResourceLimits};
use stonecrab_transport::{TcpFactory, TransportFactory};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
runtime.block_on(async {
    // A loopback "device" that answers `ping` with `pong`.
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let endpoint = Endpoint::Tcp {
        host: "127.0.0.1".to_owned(),
        port: listener.local_addr()?.port(),
    };
    let deadline = Deadline::after(Instant::now(), Duration::from_secs(5))?;

    let limits = ResourceLimits::builder().build()?;
    let mut transport = TcpFactory::default().open(&endpoint, limits, deadline).await?;
    let (mut device, _) = listener.accept().await?;

    transport.send(Bytes::from_static(b"ping"), deadline).await?;
    let mut request = [0_u8; 4];
    device.read_exact(&mut request).await?;
    device.write_all(b"pong").await?;

    let mut reply = BytesMut::new();
    transport.recv(&mut reply, deadline).await?;
    assert_eq!(&reply[..], b"pong");
    transport.shutdown(deadline).await?;
    Ok::<(), Box<dyn std::error::Error>>(())
})?;
# Ok(())
# }
# #[cfg(not(feature = "tcp"))]
# fn main() {}
```
