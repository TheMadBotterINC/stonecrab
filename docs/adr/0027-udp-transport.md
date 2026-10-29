# 0027: UDP transport
## Status
Accepted (2026-10-29), as slice S61 of docs/POST_ALPHA_PLAN.md ("UDP transport and
`modbus-tcp:udp`, against j2mod or diagslave with `tc netem` faults"; exit
"`MB-TCP-TRANSPORTS-UDP` implemented"). It adds a transport beside TCP (ADR-0014) and the
serial transport (ADR-0014, addendum S58), extends the `Transport` trait with one provided
method, and widens the transport codes of the `modbus-tcp` driver (ADR-0015). Every change to
the public API is additive to the published `stonecrab` 0.2.0. No Cargo feature is added:
the transport needs only Tokio's `net`, which the `transport` feature already enables
(ADR-0018).

## Context
Upstream's `transport_udp` (Apache PLC4X v1.0.0, commit ea229918) opens one NIO
`DatagramChannel` towards the configured remote address, ignores datagrams from any other
address, sends each write as one datagram after checking it against `max-packet-size`, and
appends received datagrams to a byte ring, so their boundaries are lost and a datagram that
does not fit is dropped whole with a warning (`docs/parity/transport.md`: `UTR-UDP`,
`UTR-UDP-RING`). It declares ten options, one of which (`multicast-ttl`) does nothing
(`UTR-UDP-OPTIONS`), and can share one channel between connections (`UTR-UDP-SHARED`).

Upstream's `modbus-tcp` driver lists `udp` among its transports and declares port 502 for
it, but picks its UDP configuration class with an `instanceof` test against BouncyCastle's
`UDPTransport`, which no PLC4X transport is (quirk `MB-QUIRK-A`). The branch never matches, so
the declared default never applies and `modbus-tcp:udp://host` without a port fails with "No
port defined" (`MB-TCP-TRANSPORTS-UDP`). Modbus over UDP is the Modbus TCP application data
unit, MBAP header included, one per datagram.

The transport layer of Stonecrab is a byte stream with deadlines: `Transport::recv` appends
whatever arrived, `FramedTransport` turns bytes into frames, and `ConnectionGuard` reconnects
and resynchronizes the stream (ADR-0014, addenda S18, S19 and S57). A stream framer reads
across `recv` calls until a frame is whole; on a datagram transport that would join the
rest of one datagram to the next one.

## Decision
1. **One connected socket per connection.** `stonecrab::transport::UdpFactory` and
   `UdpTransport`. The factory takes an `Endpoint::Custom` holding `host:port` (an IPv6 host in
   brackets), resolves it with `tokio::net::lookup_host`, binds a socket of the first
   address's family to an ephemeral port on every interface and connects it to that address.
   Only the first address is used: UDP has no handshake that could refuse one address and
   move on to the next. The operating system then delivers datagrams from that address and
   port only, which is upstream's source filter; a datagram from any other address is never
   seen. Every open makes a new socket with a new port, so a reply sent to an earlier socket
   never reaches a later one. Text that is not `host:port` is `InvalidConfig`, a name that does
   not resolve is `Disconnected`. The core `Endpoint` gains no `Udp` variant here: the serial
   transport of S58 set the precedent of a transport that reads its own `Endpoint::Custom`
   text, and `config.rs` is not part of this slice.
2. **One datagram per receive, boundaries kept.** `recv` appends exactly one whole datagram.
   A datagram longer than the free space of the receive buffer (`max_buffered_bytes` minus
   what the buffer holds) is consumed, nothing is appended, and the call fails with
   `ResourceLimit`; the transport stays usable. Upstream drops such a datagram and only logs
   a warning. The socket is offered one byte more than fits, so a datagram that fills the
   offer is known not to fit (Windows reports the same case as `WSAEMSGSIZE`, which is read
   the same way). The offer, at most 65 536 bytes, is reserved in the buffer before every
   receive, because the socket cuts a datagram to the space it is given: with the default
   `max_buffered_bytes` (262 144) a Modbus reply of a dozen bytes reserves 64 KiB of
   capacity, which `BytesMut` reuses once the previous frame split off the buffer has been
   dropped. The offer is not cut to `max_frame_bytes`, because the transport does not know
   how its caller frames. An empty datagram carries nothing and is skipped, so `recv` never
   returns `0`: a UDP peer cannot close.
3. **No frame across two datagrams.** `Transport` gains a provided method,
   `delivers_datagrams(&self) -> bool` (default `false`; `UdpTransport` returns `true`).
   `FramedTransport::next_frame` over such a transport does not read again while the buffer
   holds the rest of a datagram that ended inside a frame: it drops those bytes and fails the
   read with `MalformedData` (`FrameError::Truncated`), so a truncated or padded datagram can
   never be completed by bytes of the next one. `FramedTransport::resync` drops every buffered
   byte at once, as it does for a framer whose `resync_discards` is true (ADR-0024, decision
   4): what is left in the buffer before a request is the rest of a datagram no later datagram
   continues. A datagram that holds several whole frames still yields them one by one.
4. **The send bound.** `send` sends one datagram. It refuses, with `ResourceLimit` and before
   any I/O, a payload above `max_frame_bytes` or above 65 507 bytes, the largest UDP payload
   over IPv4 (upstream's `max-packet-size` default), whichever is smaller. A datagram is sent
   whole or not at all, so a `send` cut short by its deadline or dropped leaves nothing half
   sent and the transport stays usable; any other send failure makes it unusable.
5. **Connect, close, reconnect and the supervisor without a connection.** Opening sends
   nothing, so it succeeds whether or not anything listens. A peer that does not listen is
   noticed only when the operating system reports an ICMP port-unreachable message for an
   earlier datagram, as a failure of a later `send` or `recv` (`ECONNREFUSED`, or
   `WSAECONNRESET` on Windows), which is `Disconnected` and makes the transport unusable;
   where ICMP is filtered, the request times out at its deadline instead. `shutdown` sends
   nothing and marks the transport closed; dropping it closes the socket. The guard is
   unchanged: after `Disconnected` it reconnects with backoff, which here is a new socket.
   Its supervisor reads datagrams while no request runs, so a late or duplicated reply that
   arrives between requests is dropped and counted, but it cannot notice a peer that went
   away, because nothing tells it. The idle bound closes the socket and the next request
   opens a new one. Nothing retransmits at the transport: a lost datagram is a request that
   times out, read retries are the connection's `RetryPolicy` as for TCP, and a write whose
   confirmation is lost is `Unknown` and never replayed.
6. **Options.** None of upstream's `udp.` options is offered, and every `udp.` key stays
   `InvalidConfig` as an unknown key, as before. `max-packet-size` is the two bounds of
   decisions 2 and 4 (`ResourceLimits`); `read-timeout-ms` is the `Deadline` of every call;
   `multicast-ttl` does nothing upstream; `broadcast` cannot work with a connected socket,
   because replies to a broadcast come from unicast addresses, which the source filter drops;
   `reuse-address` and `share-socket` serve a shared socket, which is deferred
   (`UTR-UDP-SHARED`); `local-address`, `local-port` and the two buffer sizes have no user yet
   and are deferred until a device needs a fixed source port or larger buffers.
7. **`modbus-tcp:udp://host[:port]`.** `ModbusTcpDriver::parse_config` accepts the transport
   code `udp` beside `tcp`, with the same authority grammar and default port 502, and stores
   the peer as `Endpoint::Custom("host:port")`. Without an injected factory, `connect` opens
   `UdpFactory` for an `Endpoint::Custom` and `TcpFactory` for any other endpoint; an injected
   factory serves every transport code, as before. The connection, the session and the MBAP
   framer are those of TCP: one request in flight, and a reply is matched by transaction id,
   unit and function, which discards a duplicated reply and a reply that arrives after its
   request timed out. The TCP behavior is unchanged. Other codes (`tls`, `tls-psk`, `test`)
   stay refused, and so does the `udp` carrier of `modbus-rtu` (`MBS-TRANS-UDP`).
8. **`MB-QUIRK-A` stays a recorded divergence, not reproduced.** Stonecrab defaults `udp` to
   port 502, the port upstream's driver declares and the Modbus convention, where upstream
   fails for want of a port because its configuration-class branch never matches. The
   divergence row and the drafted upstream issue stay; the quirk row records that UDP is now
   implemented.
9. **Interop evidence.** diagslave 3.6 of proconX, a free Modbus slave simulator that serves
   Modbus UDP (`-m udp`), runs in a container built from `interop/diagslave/` on the pinned
   `debian:trixie-slim` digest of the libmodbus peer, with `iproute2` 6.15.0-1 pinned by
   version. Its archive is downloaded at build time from an unversioned URL and checked by
   SHA-256 (`01c87fe4...f2fe97e`); a replaced archive fails the build. Its license (proconX
   Free Software License for Binary Distributions, revision 5) allows use without charge and
   forbids redistributing anything but the whole unmodified distribution, so neither the
   archive nor an image holding it is committed or published. The tests apply `tc netem`
   inside the container (`NET_ADMIN`) for loss, delay, duplication and reordering of the
   replies, run through `interop/diagslave/interop.sh` with a checked count of six, as the
   CI job `interop-udp` and the mise task `interop-udp`. `mise run interop` leaves it out,
   because it needs a Docker that forwards published UDP ports (see Consequences).
   j2mod was the alternative; it was not needed.

## Consequences
- `modbus-tcp:udp` reads, writes, pings and polls like `modbus-tcp`; its failures are those
  of a lossy link: `Timeout` for a lost request or reply, `Disconnected` once the operating
  system reports the port unreachable, `MalformedData` for a datagram that is not a whole
  frame.
- A connection to a host where nothing listens opens without error; the first request fails.
  Callers that treated a successful `connect` as proof of a live device must not do so for
  UDP.
- Over UDP a truncated datagram fails its request at once with `MalformedData` and is
  dropped, and bytes after the last whole frame of a datagram are dropped before the next
  request; neither costs a request a wait, unlike the byte-wise search on a stream.
- `Transport` has one more provided method; implementations outside the crate keep stream
  behavior.
- Local runs of the interop job need a Docker that forwards UDP to the host. Colima's
  default ssh port forwarder does not, so `mise run interop-udp` fails its readiness probe
  there; the hosted job on Ubuntu is the evidence, and its first run is pending as of
  2026-10-29; until it passes, `docs/COMPATIBILITY.md` lists UDP as `simulator-tested`. The
  job loads the kernel's `sch_netem` module first, and `interop.sh start` checks that a
  `netem` discipline can be put on the container's interface, so a kernel without the module
  fails before any test runs.
- The default port 502 of a UDP connection string differs from upstream's effective behavior
  (an error); a string written for upstream with an explicit port behaves the same.

## Alternatives considered
- **An `Endpoint::Udp` variant.** The structured form would be clearer than `Custom` text,
  and the core's `Endpoint` documentation anticipates it. Not done in this slice, whose
  files exclude `config.rs`; a later change can add the variant and accept both forms.
- **Upstream's byte ring.** Rejected: it loses datagram boundaries and drops an oversize
  datagram with only a log warning, which the MBAP length can only partly repair.
- **A UDP-specific framer (a `Wire::Udp` with resynchronization by discard).** It would drop
  debris before a request, but could not stop the framer from reading the rest of a truncated
  datagram out of the next one; the transport has to say where a datagram ends, which is
  decision 3.
- **Refusing to append to a non-empty buffer in `UdpTransport::recv`.** Rejected: it would
  break the `Transport::recv` contract (append), and a caller collecting several datagrams
  into one buffer is legitimate outside the framing layer.
- **Trying every resolved address.** Rejected: without a handshake nothing can tell a dead
  address from a live one at open time.
- **Offering the socket options now.** Rejected for want of a user (CODING_STANDARDS design
  discipline); each refused key is recorded with its reason in decision 6.
- **j2mod as the peer.** Apache-2.0, but a JDK image and a jar; diagslave is one static
  binary and its license allows this use.

## Compliance
- `crates/stonecrab/src/transport/udp.rs` (`UdpFactory`, `UdpTransport`), the UDP sites of
  `transport/deadline.rs`, the provided `Transport::delivers_datagrams` (`traits.rs`) and its
  two uses in `transport/framed.rs`; `crates/stonecrab/src/modbus/driver.rs` (transport code
  `udp`, the factory choice).
- Tests: `crates/stonecrab/tests/transport_udp.rs` (round trip, kept boundaries, the source
  filter, the oversize datagram, the send bound, the empty datagram, a closed port, timeouts,
  shutdown, the endpoint, the framing over datagrams), `crates/stonecrab/tests/modbus_udp.rs`
  (reads and writes, a lost reply, a duplicated reply, a late reply, a reply split across two
  datagrams, the contract suite over UDP), the `udp` cases of `tests/modbus_config.rs` and of
  the driver's unit tests, and the ignored `crates/stonecrab/tests/modbus_interop_diagslave.rs`
  run by the `interop-udp` job.
  The loopback tests wait on no real deadline: a request or receive that must time out runs
  on Tokio's paused clock, and the contract run loses its write confirmation to a datagram
  that ends inside an MBAP header, not to a swallowed reply.
- `docs/parity/transport.md` (`UTR-UDP`, `UTR-UDP-OPTIONS`, `UTR-UDP-RING`, `UTR-UDP-SHARED`)
  and `docs/parity/modbus.md` (`MB-TCP-TRANSPORTS`, `MB-TCP-TRANSPORTS-UDP`,
  `MB-CFG-TRANSPORT-OPTIONS`, `MB-QUIRK-A`) record the rows with their test ids.

## Addendum (2026-10-29): the first hosted run of `interop-udp`

The hosted job ran with the commit that added this ADR (d208261, CI run 40830080420) on
`ubuntu-latest`: `sudo modprobe sch_netem`, the diagslave image built from the pinned base
and archive, the netem preflight passed, and all six `interop_udp_*` tests passed (write
and read back, loss of every reply, delay below and past the deadline, duplication,
reordering). The `interoperability-tested` level of the UDP rows in
`docs/COMPATIBILITY.md` and the `automated-verified` statuses of `UTR-UDP` and
`MB-TCP-TRANSPORTS-UDP` now rest on that run, as the Consequences above anticipated.
