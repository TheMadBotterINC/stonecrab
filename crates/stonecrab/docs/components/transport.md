# Stonecrab Transport
Deadline-bounded byte transports for the Stonecrab protocol drivers
(`stonecrab::transport`, behind the `transport` feature). A `Transport` sends and
receives bytes to one peer and does nothing else: it does not frame, parse, retry or
reconnect. The module also holds the `TransportFactory` that opens transports, the
`ConnectionGuard` that owns a connection's lifecycle, the Tokio-backed `SystemClock`, and
a small jitter source for reconnect backoff. It uses the core and a scoped set of Tokio
features and not the codec module; framing errors are reported with the module's own
`FrameError`.

No upstream parity claim is made: Apache PLC4X builds its transports on Netty, and these
types are Stonecrab's own contract. The behavior that is specified is recorded in the
[transport record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/transport.md).

| Capability | Support level |
| --- | --- |
| `Transport` and `TransportFactory` traits, `FrameError` | build-tested |
| `TcpTransport` and `TcpFactory` | interoperability-tested |
| `Framer` trait and `FramedTransport` (`next_frame`, `send_frame`) | interoperability-tested |
| `FramedTransport::resync` | simulator-tested |
| `ConnectionGuard` connect, close, lifecycle and one in-flight request, `GuardConfig`, `GuardStats` | interoperability-tested |
| `ConnectionGuard` request-driven reconnect, bounded queue, resync after a timeout | simulator-tested |
| `ConnectionGuard::supervise`: background reconnect and the `Timeouts.idle` bound | simulator-tested |
| `SystemClock` (Tokio timer) | interoperability-tested |
| `Jitter` (xorshift64* sample source for `ReconnectPolicy::next_delay`) | build-tested |
| `SerialTransport` and `SerialFactory` (feature `serial`) | simulator-tested |
| `UdpTransport` and `UdpFactory`, `Transport::delivers_datagrams` | simulator-tested; interoperability-tested once the first hosted `interop-udp` run passes |
| TLS transport | planned |
| Hardware verification | planned |

How the levels were reached. The TCP transport has byte-level tests over real loopback
sockets (connect refusal, fragmented and coalesced delivery, peer close, buffer and frame
bounds, shutdown) and, through `stonecrab::modbus`, carries every read of its `interop`
job against pymodbus 3.8.6 (the Modbus MBAP framer runs over `FramedTransport` and the
guard there). `interoperability-tested` therefore means those paths ran against an
independent server; it is a statement about the module as used by that driver, not a claim
about any other protocol. Deadline expiry, resync, reconnect, the queue and late replies
are tested on the in-memory `FakeTransport` and a fake clock (decision D21: Tokio paused
time does not control socket readiness), and pymodbus has no way to drop a link or delay a
reply on demand, so those rows are `simulator-tested` and no further. The framing layer
ships no protocol framer of its own; the only one is the Modbus one. No physical device has
been tried.

The serial transport is `simulator-tested`: its byte-level tests run over a pseudo-terminal
pair (the virtual serial pair of the `serial2` crate), which exercises the operating
system's terminal layer but no UART, RS-485 adapter or device, and no protocol framer runs
over it yet. Its worker, retry, cancellation and error paths run on an in-memory port. The
pair tests run on Unix other than macOS: `serial2` builds its pair with a baud-rate ioctl
that a macOS pseudo-terminal refuses, and Windows has no pseudo-terminals. Where they do not
run, the in-memory tests still do.

The UDP transport (S61, ADR-0027) has datagram tests over real loopback sockets (the source
filter, kept boundaries, an oversize datagram, the send bound, a closed port, timeouts) and,
through `stonecrab::modbus` (`modbus-tcp:udp`), carries the reads, writes and ping of its
`interop-udp` job against diagslave 3.6, an independent Modbus UDP slave, with `tc netem`
losing, delaying, duplicating and reordering the replies on a real clock. Its first local run
went through a UDP-over-TCP relay, because colima's default port forwarder does not forward
UDP; the hosted job publishes the UDP port directly, and its first run is pending as of
2026-10-29.

## Feature
The module exists only with the `transport` feature, which is the only thing that brings
Tokio into the crate: `rt`, `net`, `time`, `sync` and `io-util` are scoped on, and the
`full` feature of Tokio is banned by `deny.toml`. The `modbus` and `testkit` features
turn `transport` on; `sim`, `sensors` and `default-features = false` do not, so the
core stays free of Tokio. A transport does no work between calls. The one task in the
module is the connection guard's optional supervisor, which `ConnectionGuard::supervise`
starts on the current Tokio runtime and the guard owns: `close` joins it and dropping the
guard aborts it (ADR-0014, addendum S57).

The `serial` feature adds `SerialTransport` and `SerialFactory` to the module and turns on
`transport` and the `serial2` crate. No other feature enables it, so a build without it has
no `serial2` in its dependency graph. See [Serial](#serial). The UDP transport needs nothing
beyond Tokio's `net`, so it is part of `transport` itself. See [UDP](#udp).

## The deadline rule
Every call that waits takes the `Deadline` it must finish by, and that argument is the
only deadline source: there is no default and no hidden timeout. A deadline that has
already passed fails with `ErrorKind::Timeout` before any I/O. Deadlines are compared with
Tokio's clock, which is the ordinary `std::time::Instant` clock unless a test pauses time;
build them from `SystemClock::now()` and they agree in both cases.

## Serial
`SerialFactory::open` opens the port named by an `Endpoint::Custom` (`/dev/ttyUSB0`,
`COM3`) with the factory's settings, which are upstream's option names and defaults: 9600
baud, 8 data bits, no parity, 1 stop bit, no flow control. It returns a `SerialTransport`
that follows the same `Transport` rules as the TCP one. `SerialTransport::from_port` wraps a
port you opened yourself (any `Read + Write` that gives up a call after a short timeout; a
`serial2::SerialPort` with its read and write timeouts set to about 20 ms).

Serial ports have blocking reads and writes, so one dedicated thread owns each port (the
worker) and the async side talks to it over a bounded channel (coding standards 8.4 and
8.8; ADR-0014, addendum S58). The worker is a thread, not a Tokio task, and it reads the
port only while a `recv` is waiting: between calls it is blocked on the channel, so the
no-background-work rule of decision D15 holds here too.

- **Deadlines are the caller's.** The worker reads no clock. Every call races its reply
  against its `Deadline` on the Tokio clock, exactly as the TCP transport does, so an
  elapsed deadline fails before any I/O and a deadline that passes mid-call is `Timeout`.
- **Writes are bounded by the deadline.** The factory gives the port a 20 ms write timeout,
  so a write call returns once the line has taken some bytes or the timeout has passed, and
  the worker offers a frame in chunks of at most 256 bytes and looks for a departed caller
  before each. A `send` into a stalled or flow-controlled line therefore ends at its
  deadline, and the worker is free again within one slice of it. A write that returns
  nothing (`Ok(0)`, which some platforms use for a timeout) is not an error: it is tried
  again until the frame is out or the caller has gone.
- **Cancellation.** A dropped or timed-out call is noticed within a slice. A dropped `recv`
  loses nothing, even if the worker had already answered it: bytes the worker read for it
  are kept in a stash on the worker's side and returned, in order, by the next `recv`. A
  cut-short `send` poisons the transport, as for TCP.
- **Close.** `shutdown` stops the worker, waits for it (within the deadline) and so releases
  the device; a repeat call is a no-op. Dropping the transport without it also stops the
  worker, without waiting.
- **Errors.** A port error is `Disconnected` with the cause as the source; a closed peer
  (an end of stream or a broken pipe) is `recv` returning `0`, after which every call is
  `Disconnected`; a missing or unreadable port is `Disconnected`; a wrong endpoint, an empty
  name, a zero baud rate or a rate the platform has no setting for is `InvalidConfig`.
- **Not offered yet.** `read-timeout-ms`, `write-timeout-ms` (replaced by the deadlines),
  `dtr`, `rts`, `reuse-port` and `interframe-delay`; mark and space parity and 1.5 stop
  bits. The Modbus RTU driver (S59, ADR-0024) owns inter-frame timing
  (`serial.interframe-delay`) and refuses a shared port (`serial.reuse-port=true`). `send` returns when
  the bytes are handed to the operating system, not when the last bit has left the line.
  The port is not opened exclusively: keep one transport per port.

## UDP
`UdpFactory::open` takes an `Endpoint::Custom` holding `host:port` (an IPv6 host in brackets),
resolves it, and connects a new socket on an ephemeral port to the first address (ADR-0027).
The operating system then delivers datagrams from that address only; a datagram from any
other address is never seen, as upstream. Every open makes a new socket, so a reply sent to
an earlier one never reaches a later one.

- **One datagram per call.** `send` sends one datagram; one above `max_frame_bytes` or
  65 507 bytes (the largest UDP payload over IPv4) is `ResourceLimit` before any I/O. `recv`
  appends exactly one whole datagram, so boundaries are kept; one longer than the free space
  of the buffer is consumed and reported as `ResourceLimit` with nothing appended, where
  upstream drops it and only logs a warning. An empty datagram is skipped, so `recv` never
  returns `0`. `Transport::delivers_datagrams` is true, which tells `FramedTransport` where
  a datagram ends (see [Framing](#framing)).
- **No connection.** Opening and `shutdown` send nothing. A peer that does not listen is
  noticed only when the operating system reports an ICMP port-unreachable message for an
  earlier datagram, as `Disconnected` on a later call (the transport is then unusable and
  the guard opens a new socket); where ICMP is filtered the request times out. A send cut
  short by its deadline leaves the transport usable, since a datagram goes whole or not at
  all. Nothing retransmits: a lost datagram is a request that times out.
- **No options.** Upstream's `udp.` options are not offered; why each is refused or
  deferred is recorded in ADR-0027 and the transport record (`UTR-UDP-OPTIONS`).

## Framing
A `Framer<F>` turns bytes into frames (`decode`, `encode`, `max_frame_bytes`) and does no
I/O. `FramedTransport<F, R>` joins a `Box<dyn Transport>` with a framer `R` and owns the
receive buffer, bounded by `max_buffered_bytes`:

- `next_frame(deadline)` receives until the framer yields a whole frame. Fragmented
  frames are assembled, coalesced frames come out of successive calls, a frame over the
  framer's maximum fails with `ResourceLimit` as soon as its header is in, and a peer that
  closes mid-frame is `Disconnected`. Over a transport that delivers datagrams (UDP), a
  datagram that ends inside a frame is never joined to the next one: its bytes are dropped
  and the read fails with `MalformedData`.
- `send_frame(&frame, deadline)` encodes into a scratch buffer and sends one transport
  frame; a frame over the smaller of the framer's and the transport's frame limit fails
  with `ResourceLimit` before anything is sent.
- `resync(deadline)` realigns after a framing error or an abandoned request: it discards
  bytes from the front, one at a time, while the framer rejects them, and keeps
  receiving. It fails with `Disconnected` when the deadline passes without a frame
  boundary. A framer whose `Framer::resync_discards` is true (Modbus RTU since S59, where
  a byte-wise search could wait for a frame tail that never comes) has the whole buffer
  dropped instead, counted in `GuardStats::resyncs`, and so does a transport that delivers
  datagrams, whose buffered bytes no later datagram continues. Its deadline is the remaining deadline of the request whose timeout triggered
  it, or `Deadline::after(clock.now(), timeouts.request)` from close or recovery.

## The connection guard
`ConnectionGuard<F, R>` owns one connection: the lifecycle (`Disconnected`, `Connecting`,
`Connected`, `Reconnecting { attempt, next_at }`, `Closing`, `Closed`, `Failed`), a
`FramedTransport` made by a `TransportFactory`, and one operation on it,
`request(frame, deadline, token)`: one frame out, one frame back. A protocol session wraps
it and does the correlation. Observe it with `state()` and `watch_state()`, count with
`stats()`, and end it with `close()`.

- **Reconnect.** After a transport failure the state is `Reconnecting` with
  `next_at = now + next_delay(attempt - 1, sample)`; the next request sleeps until
  `next_at` on the clock (cancellably, within its own deadline), reopens within
  `timeouts.connect` and carries on. When `ReconnectPolicy::is_exhausted` says so the
  state becomes `Failed(Disconnected)` and every later request fails with `Disconnected`
  before any I/O.
- **The supervisor.** `supervise()` starts a background task on the current Tokio
  runtime (outside one, or once the guard is closing, closed or failed, it returns
  `false` and reconnect stays request-driven; the task lives on that runtime and is
  aborted, not joined, when the guard is dropped). It reads the stream (in every
  sub-state, so a link that dropped after a request timed out is noticed too), so a peer
  that closes is noticed with no request; in `Reconnecting` it reopens the transport as
  soon as `next_at` passes, attempt after attempt, until the guard is `Connected` or
  `Failed`. Every state it reaches goes through `watch_state()`. A request takes the
  slot from its reader at once, even on a multi-thread runtime; one that arrives while
  it is opening a transport waits in line, within its own deadline. A frame the reader
  finds after a timed-out or cancelled request is that request's late reply and is
  dropped. A frame it finds while nothing is owed is kept for the next `next_reply` (a
  caller may be between two guard calls) and dropped by the next request; behind it the
  reader only buffers what arrives, so a peer close is still noticed. The Modbus TCP
  connection starts it when it is opened inside a Tokio runtime.
- **The idle bound.** With a supervisor, `timeouts.idle` is enforced, a kept frame or
  not: when no exchange has run for that long the transport is shut down (with whatever
  it buffered), the state stays `Connected`, and the next request opens a new transport
  without backoff, while `next_reply` fails `Disconnected`. A zero bound closes the
  transport after every exchange. Without a supervisor `idle` is ignored. The transport
  leaves the guard only once its shutdown has returned, so a `close` that overtakes an
  idle close shuts it down itself; only a transport lost to a failure (a peer close, an
  I/O error, a send cut short, a failed resync) is dropped without a shutdown.
- **One in flight, a bounded line behind it.** At most `max_pending_requests` requests
  are owned at once, the running one included; one more is `Backpressure` before any
  I/O. The rest wait in arrival order on `tokio::sync` primitives and leave the line when
  cancelled, timed out or overtaken by `close`.
- **Deadlines.** `request` takes the deadline from the caller;
  `request_with_default_deadline` uses `timeouts.request`. `close` is bounded by
  `timeouts.request` too, joins the supervisor within that bound (aborting it if it is
  still running then) and always ends `Closed`.
- **Late replies.** A request that times out or is cancelled after its frame went out
  leaves the guard `Connected`. If bytes of its reply are buffered, the next request first
  runs `resync` within its own deadline and drops the frame found; if none is found in
  time that request fails with `Disconnected` and the guard reconnects. A reply that
  arrives whole later is the next request's reply (with a supervisor, one that arrives
  between two requests is read and dropped by the supervisor instead, and counted in
  `stats().discarded_frames` with every other frame the guard drops between exchanges),
  and the session discards it by correlation. A session does that with
  `request_matching(frame, deadline, token, accept)`, which keeps the in-flight slot from
  the send to the first frame `accept` takes, so neither the supervisor nor the idle bound
  comes between its reads; `next_reply(deadline, token)` reads one further frame without
  sending, and under a supervisor the slot is free between two such calls.
- **Reconnect budget.** Only a received reply resets the attempt count; an open alone
  does not, so a peer that accepts and then closes ends in `Failed(Disconnected)`. With a
  supervisor, a reconnected link that stays up for a whole read (`idle` after its last
  activity, else `timeouts.request`) also resets it.

## Limits
`recv` appends only up to `ResourceLimits::max_buffered_bytes` and fails with
`ErrorKind::ResourceLimit` when the buffer is already full. `send` carries one frame and
refuses more than `ResourceLimits::max_frame_bytes`. A `send` that is cut short may have
put a prefix of the frame on the wire, so the transport poisons itself and the owner
reopens it; a datagram cannot be cut short, so a UDP `send` cut short leaves the transport
usable.

```rust
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
```
