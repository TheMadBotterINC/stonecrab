# Stonecrab Transport
Deadline-bounded byte transports for the Stonecrab protocol drivers. A `Transport` sends
and receives bytes to one peer and does nothing else: it does not frame, parse, retry or
reconnect. The crate also holds the `TransportFactory` that opens transports, the
`ConnectionGuard` that owns a connection's lifecycle, the Tokio-backed `SystemClock`, and
a small jitter source for reconnect backoff. It depends on
`stonecrab-core` (plus `bytes` and a scoped set of Tokio features) and not on the codec
crate; framing errors are reported with the crate's own `FrameError`.

No upstream parity claim is made: Apache PLC4X builds its transports on Netty, and these
types are Stonecrab's own contract. The behavior that is specified is recorded in the
[transport record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/transport.md).

| Capability | Support level |
| --- | --- |
| `Transport` and `TransportFactory` traits, `FrameError` | build-tested |
| `TcpTransport` and `TcpFactory` (feature `tcp`) | interoperability-tested |
| `Framer` trait and `FramedTransport` (`next_frame`, `send_frame`) | interoperability-tested |
| `FramedTransport::resync` | simulator-tested |
| `ConnectionGuard` connect, close, lifecycle and one in-flight request, `GuardConfig`, `GuardStats` | interoperability-tested |
| `ConnectionGuard` request-driven reconnect, bounded queue, resync after a timeout | simulator-tested |
| `SystemClock` (Tokio timer) | interoperability-tested |
| `Jitter` (xorshift64* sample source for `ReconnectPolicy::next_delay`) | build-tested |
| Serial, UDP and TLS transports | planned |
| Hardware verification | planned |

How the levels were reached. The TCP transport has byte-level tests over real loopback
sockets (connect refusal, fragmented and coalesced delivery, peer close, buffer and frame
bounds, shutdown) and, through `stonecrab-modbus`, carries every read of its `interop`
job against pymodbus 3.8.6 (the Modbus MBAP framer runs over `FramedTransport` and the
guard there). `interoperability-tested` therefore means those paths ran against an
independent server; it is a statement about the crate used by that driver, not a claim
about any other protocol. Deadline expiry, resync, reconnect, the queue and late replies
are tested on the in-memory `FakeTransport` and a fake clock (decision D21: Tokio paused
time does not control socket readiness), and pymodbus has no way to drop a link or delay a
reply on demand, so those rows are `simulator-tested` and no further. The framing layer
ships no protocol framer of its own; the only one is the Modbus one. No physical device has
been tried.

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

## Framing
A `Framer<F>` turns bytes into frames (`decode`, `encode`, `max_frame_bytes`) and does no
I/O. `FramedTransport<F, R>` joins a `Box<dyn Transport>` with a framer `R` and owns the
receive buffer, bounded by `max_buffered_bytes`:

- `next_frame(deadline)` receives until the framer yields a whole frame. Fragmented
  frames are assembled, coalesced frames come out of successive calls, a frame over the
  framer's maximum fails with `ResourceLimit` as soon as its header is in, and a peer that
  closes mid-frame is `Disconnected`.
- `send_frame(&frame, deadline)` encodes into a scratch buffer and sends one transport
  frame; a frame over the smaller of the framer's and the transport's frame limit fails
  with `ResourceLimit` before anything is sent.
- `resync(deadline)` realigns after a framing error or an abandoned request: it discards
  bytes from the front, one at a time, while the framer rejects them, and keeps
  receiving. It fails with `Disconnected` when the deadline passes without a frame
  boundary. Its deadline is the remaining deadline of the request whose timeout triggered
  it, or `Deadline::after(clock.now(), timeouts.request)` from close or recovery.

## The connection guard
`ConnectionGuard<F, R>` owns one connection: the lifecycle (`Disconnected`, `Connecting`,
`Connected`, `Reconnecting { attempt, next_at }`, `Closing`, `Closed`, `Failed`), a
`FramedTransport` made by a `TransportFactory`, and one operation on it,
`request(frame, deadline, token)`: one frame out, one frame back. A protocol session wraps
it and does the correlation. Observe it with `state()` and `watch_state()`, count with
`stats()`, and end it with `close()`.

- **Reconnect is request-driven (D15).** There is no background task. After a transport
  failure the state is `Reconnecting` with `next_at = now + next_delay(attempt - 1,
  sample)`; the next request sleeps until `next_at` on the clock (cancellably, within its
  own deadline), reopens within `timeouts.connect` and carries on. When
  `ReconnectPolicy::is_exhausted` says so the state becomes `Failed(Disconnected)` and
  every later request fails with `Disconnected` before any I/O.
- **One in flight, a bounded line behind it.** At most `max_pending_requests` requests
  are owned at once, the running one included; one more is `Backpressure` before any
  I/O. The rest wait in arrival order on `tokio::sync` primitives and leave the line when
  cancelled, timed out or overtaken by `close`.
- **Deadlines.** `request` takes the deadline from the caller;
  `request_with_default_deadline` uses `timeouts.request`. `close` is bounded by
  `timeouts.request` too and always ends `Closed`.
- **Late replies.** A request that times out or is cancelled after its frame went out
  leaves the guard `Connected`. If bytes of its reply are buffered, the next request first
  runs `resync` within its own deadline and drops the frame found; if none is found in
  time that request fails with `Disconnected` and the guard reconnects. A reply that
  arrives whole later is the next request's reply, and the session discards it by
  correlation, so a session must loop on `next_reply(deadline, token)` (which reads a
  further frame without sending) until the correlated frame arrives.
- **Reconnect budget.** Only a received reply resets the attempt count; an open alone
  does not, so a peer that accepts and then closes ends in `Failed(Disconnected)`.

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
use stonecrab::{Deadline, Endpoint, ResourceLimits};
use stonecrab::transport::{TcpFactory, TransportFactory};
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
