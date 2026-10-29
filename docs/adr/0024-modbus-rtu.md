# 0024: Modbus RTU
## Status
Accepted (2026-10-28), as slice S59 of docs/POST_ALPHA_PLAN.md ("framing, CRC-16,
inter-frame timing, bus serialization, shared PDU"; exit "SPEC.md's RTU failure cases
pass"). It builds on the serial transport of S58 (ADR-0014, addendum S58) and amends
decision 2 of ADR-0018 (the feature table: `modbus` now enables `serial`). Every change to
the public API is additive to the published `stonecrab` 0.2.0. Modbus ASCII is S60 and is
not decided here.

## Context
SPEC.md requires that "Modbus TCP and RTU must share one PDU implementation", that "for
Modbus RTU, [requests are serialized] on a shared serial bus", that "late responses after a
timeout must not satisfy a newer request; close, drain or re-synchronize according to the
protocol", and lists the RTU failure cases: "CRC errors, timing boundaries, serial
disconnect/reopen and wrong unit replies". `docs/parity/modbus-serial.md` records 77 rows
from the upstream RTU and ASCII drivers (Apache PLC4X v1.0.0, commit ea229918).

Upstream's `ModbusRtuDriver` (protocol code `modbus-rtu`, default transport `serial`, also
`tcp`, `tls`, `tls-psk`, `udp` and `test`) frames responses only: it sizes a reply from its
function code (an exception is 5 bytes; function codes 01 to 04, 0x14, 0x15 and 0x17 are 5
plus the byte count; 05, 06, 0x0F and 0x10 are 8; any other code is a desynchronization),
lets the generated parser check the CRC, and on a failure skips one byte and tries again
(`ModbusRtuMessageCodec`). It has no 3.5-character silence detection; its `WritePacer`
spaces writes by `interframe-delay` (default 0) measured from the last write or read. The
connection allows one request at a time, matches a reply by unit id plus a function-code
check (there is no transaction id), and cannot tell a late reply of the same function from
the answer to the next request.

The Modbus TCP driver of the alpha already has everything above the framing: the address
grammar, the batcher, the PDU codec, the typed decoder, the exception mapping, the
`ModbusSession` over the transport `ConnectionGuard` (one exchange in flight, correlation,
counters) and the `ModbusConnection` (reads, writes, ping, polling subscriptions, retry,
reconnect). The `Framer` trait and `FramedTransport` (S18) turn bytes into frames; the
guard resynchronizes a stream an abandoned exchange may still owe bytes to before the next
request (`FramedTransport::resync`, which discards one byte at a time until a frame
decodes, and fails `Disconnected` when none does by the deadline). S58 added
`SerialFactory` and `SerialTransport` behind the off-by-default `serial` feature.

## Decision
1. **One PDU implementation, one session, one connection.** The `modbus-rtu` driver is
   `stonecrab::modbus::ModbusRtuDriver`. Its connections are `ModbusConnection`s and their
   sessions are `ModbusSession`s, built over a crate-private `Wire` (`Tcp` or `Rtu`) that
   picks the framer, the correlation rule and the scheme the spans name. Nothing above
   the framing is duplicated: reads, batching, writes, ping, subscriptions, retry,
   reconnect and the exception mapping are the code of Modbus TCP, unchanged for TCP
   (its public API, behavior and tests stay as they were). `ModbusConnection::open` and
   `ModbusSession::new` stay Modbus TCP; the RTU driver opens through crate-private
   constructors.
2. **Framing.** `RtuAdu` (the address byte and the PDU; at most 256 bytes on the wire,
   `RtuAdu::MAX_BYTES`) and `RtuFramer`, a `Framer<RtuAdu>`. The CRC is CRC-16/MODBUS:
   reflected polynomial 0xA001, initial value 0xFFFF, no final XOR, low byte first on the
   wire; it is appended on send and checked on receive. A response is sized by upstream's
   table. `decode` judges the front of the buffer: a size above 256 bytes is `Oversized`,
   judged from the header alone; a whole frame with the right CRC is the frame. When the
   front is anything else (incomplete, an unknown function code, or a wrong CRC), the
   framer takes the first later position that holds a whole frame with the right CRC,
   removing the noise together with the frame, so noise in front of a reply does not lose
   it. The search runs for an incomplete front too, because a stray byte in front of a
   reply reads as a header whose size the buffer does not hold yet (with unit 1, a write
   echo, an exception or a one-register read); a false match inside a frame still arriving
   needs a correct CRC-16. When no later position holds a frame, an incomplete front is
   `Ok(None)` and any other is `Malformed`, and nothing is removed (the `Framer`
   contract). Silence is not used to find frames, as upstream (the serial worker of S58
   does not time bytes; `MBS-QUIRK-RTU-NO-SILENCE`).
3. **A CRC error fails the request; the bytes are dropped before the next one.** A reply
   whose CRC is wrong is `MalformedData` (category `Protocol`) for its request at once,
   never a silent wait to the deadline. A read may be retried by the configuration's
   `RetryPolicy`; a write whose echo fails the CRC went out, so it is `Unknown` and never
   replayed (`MB-WRITE-UNKNOWN`). Before the next request the guard resynchronizes, which
   for RTU drops every buffered byte (decision 4); the drop is counted in
   `GuardStats::resyncs`. Upstream skips the byte and waits, so its request ends in a
   timeout; the fail-fast is a recorded divergence.
4. **Resynchronization drops the buffer.** A new provided method of the public `Framer`
   trait, `resync_discards(&self) -> bool` (default `false`), makes
   `FramedTransport::resync` drop every buffered byte and return at once instead of
   searching. `RtuFramer` returns `true`. An RTU stream has no marker to search for: the
   tail of a frame the owner gave up on looks like the start of a frame still arriving,
   and the byte-wise search would wait for bytes that never come and then fail
   `Disconnected`, closing the port. When the guard resynchronizes (before its next
   request) every buffered byte belongs to an exchange it abandoned. The method is
   additive (a provided method); `guard.rs` is unchanged.
5. **Silence before every request.** The driver wraps the carrier's transports in a
   crate-private pacer: before each send it watches the line until it has been quiet for
   the gap, drops every byte that arrives meanwhile (a master ignores what it receives
   while idle) and starts the watch again when one does. The gap is t3.5, 3.5 character
   times from the line settings (one start bit, eight data bits, the parity bit and the
   stop bits; 3.646 ms at 9600 8N1), fixed at 1.75 ms above 19200 baud as the Modbus over
   Serial Line specification recommends, or `serial.interframe-delay` when that is
   longer. Watching rather than measuring from the last activity is what flushes bytes
   nobody read since; it costs one gap per request. The watch is part of the request's
   deadline; a deadline that ends inside it is `Timeout`, and because the guard treats a
   failed send as a lost link, the port is let go and reopened by the next request. Over
   the `tcp` carrier there is no line, and the gap is `serial.interframe-delay` alone,
   which the `tcp` carrier does not accept, so it is zero; the drain of decision 6 still
   applies there.
6. **A late reply is drained.** After a send that nothing answered (no byte arrived until
   the next send: a timeout, a cancellation), the quiet time before the next request is
   the drain: the gap plus the time a 256-byte frame takes (270 ms at 9600 8N1, 24 ms at
   115200), so a reply that was late but on its way is heard out and dropped instead of
   answering the next request. Bytes left in the buffer are dropped by decision 4. A
   reply that starts after the next request is on the wire, from the same unit, with the
   same function code and a byte count or echo that fits the next request, cannot be told
   apart on a protocol without a transaction id; the drain bounds that window, and it is
   recorded with `MBS-BUS-LATE-REPLY`. Over the `tcp` carrier the gateway's line is not
   known, and the drain is that of 9600 8N1, upstream's default line (270 ms): the socket
   is watched and a late reply that reaches it within the drain is dropped, which the
   resync of decision 4 cannot do for bytes not yet read. A gateway on a slower line may
   pass a late reply on after the drain. Inside a Tokio runtime the guard's supervisor
   also reads the line between requests and drops a frame an abandoned exchange owed.
7. **Correlation.** A reply is accepted when its address equals the request's unit and its
   function code (with the exception bit cleared) equals the request's; it is then proved
   against the request as for TCP (the byte count against the quantity, the echo of a
   write). Any other frame is discarded, counted in `SessionStats::mismatched_replies` and
   logged, and the wait goes on to the deadline. An exception reply answers the request.
   `late_replies` counts transaction ids and stays 0 for RTU.
8. **One request on the line.** The connection runs one operation at a time
   (`Capabilities.concurrency` is 1, `Backpressure` otherwise), so a port carries one
   request at a time. A port is not shared between connections: `serial.reuse-port`
   accepts `false` only, `true` is `InvalidConfig` (upstream's shared ports,
   `MBS-BUS-SHARED-PORT`, stay deferred). The serial transport takes no exclusive lock on
   the device (ADR-0014, addendum S58), so the documentation says to open one connection
   per port.
9. **No broadcast.** Unit 0 is the broadcast address of a serial line and no server
   answers it; the guard has no send-only exchange. `default-unit-identifier=0` is
   `InvalidConfig` for `modbus-rtu`, and a request to unit 0 (an address block
   `{unit-id: 0}`) fails with `InvalidAddress` before any I/O. Upstream sends it and times
   out.
10. **Carriers and connection strings.** `modbus-rtu://<port>` and
    `modbus-rtu:serial://<port>`: the authority is the port name, passed through as
    written (`modbus-rtu:///dev/ttyUSB0`, `modbus-rtu://COM3`), as upstream passes the
    transport URL. `modbus-rtu:tcp://host[:port]` carries RTU frames over the TCP
    transport to a serial-to-IP gateway, default port 502 as the Go driver injects (the
    Java driver has no default and fails "No port defined": a divergence). `tls`,
    `tls-psk`, `udp`, `test` and every other code are `InvalidConfig`, as for
    `modbus-tcp`. An injected transport factory (`with_transport_factory`) replaces the
    carrier, and the pacer still applies.
11. **Parameters.** The parameters of `modbus-tcp`, with `default-unit-identifier` from 1
    to 255, and, for the serial carrier only, the line under the `serial.` prefix:
    `serial.baud-rate` (positive, default 9600), `serial.data-bits` (8 only: an RTU byte
    is eight data bits), `serial.stop-bits` (1 or 2, default 1), `serial.parity` (`none`,
    `odd`, `even`; default `none`), `serial.flow-control` (`none`, `rts-cts`, `xon-xoff`,
    `rtscts`, `xonxoff`; default `none`), `serial.interframe-delay` (milliseconds,
    default 0) and `serial.reuse-port` (`false`). Values are matched after upstream's
    normalisation (lower case, `_` as `-`), so `EVEN_PARITY` is refused as upstream fails
    it. Mark and space parity are refused (the serial transport has neither).
    `serial.read-timeout-ms`, `serial.write-timeout-ms`, `serial.dtr` and `serial.rts` are
    refused as unknown keys: every call takes a deadline (ADR-0014, addendum S58) and the
    modem lines cannot be tested without hardware. Every unknown key is `InvalidConfig`,
    a `serial.` key on the `tcp` carrier included. The parsed values travel in the
    configuration's driver options slot as a crate-private type; a configuration built by
    hand gets the defaults.
12. **Capabilities.** Those of `modbus-tcp`: read, write, ping, subscribe by polling, no
    browse or discovery, PDUs up to 253 bytes, concurrency 1.
13. **Feature.** `modbus = ["transport", "serial", "dep:tracing"]`: the default build now
    contains `serial2` (BSD-2-Clause OR Apache-2.0; it adds no other crate to the lock
    file), as the S58 addendum of ADR-0014 announced. No new feature is added; the
    feature table of ADR-0018 decision 2 reads `modbus` pulls in `transport`, `serial`
    and `tracing`.
14. **Evidence level.** The driver is `simulator-tested`: the fake transport on the fake
    clock, the contract suite over the fake transport, the upstream RTU parser vectors, a
    fuzz target (`fuzz_modbus_rtu`) and RTU over a loopback TCP socket. No interop peer
    exists yet (a socat pair with libmodbus RTU or modbus-tk `RtuServer` is a later
    slice) and no hardware has been read (HW2).

## Consequences
- RTU costs one gap of watching per request (1.75 to 4 ms at common rates) and, after an
  unanswered request, the drain (up to about 300 ms at 9600 baud), both inside the
  request's deadline. A request timeout shorter than the drain makes the request after a
  timeout fail too, and the port is reopened; the reopened port starts without a drain, so
  the request after that one is not affected. The drain grows as the baud rate falls (2.2 s
  at 1200 baud 8N1): the default 5000 ms leaves room for it and a short reply down to
  1200 baud, and below about 520 baud (8N1; about 620 baud with 12-bit characters) the
  drain alone is longer than it, so a configuration at such rates needs a longer
  `request-timeout-ms`. No parameter check refuses the short timeout.
- A noise byte that is no frame (an unknown function code) in front of a reply that
  arrives in two reads, with the noise and only part of the reply in the first, fails the
  request with `MalformedData` where upstream would lose the reply and time out; a read
  retry covers it. A noise byte that reads as an incomplete header waits for the reply.
- `Framer` gains a provided method; implementations outside the crate keep the search.
- The default build compiles `serial2` on every target that builds `modbus`.
- `ModbusConnection` and `ModbusSession` serve two schemes; their public constructors
  remain Modbus TCP.

## Alternatives considered
- **A separate RTU session and connection.** Rejected: it would duplicate correlation,
  retry, writes, ping and subscriptions, which SPEC.md forbids for the PDU and which the
  slice must not do for the rest.
- **The byte-wise search for RTU resynchronization.** Rejected: on a line without a frame
  marker the search waits for the rest of a candidate that never comes and then closes the
  port (decision 4). Upstream's resynchronizing codec stops waiting while it skips, which
  a `Framer` cannot do without consuming bytes outside a frame.
- **Skipping a bad CRC and waiting for a valid reply.** Upstream's behavior. Rejected for
  the request path: nothing retransmits on Modbus, so the wait always ends at the
  deadline; a typed `MalformedData` at once is more useful and the retry policy decides.
- **Silence-based framing on receive (a frame ends after 3.5 quiet characters).** The
  specification's model. Deferred to the hardware evidence: the serial worker hands over
  bytes in chunks without arrival times, and upstream does not do it either.
- **Measuring the gap from the last activity instead of watching.** Rejected: it cannot
  drop bytes that arrived unread, which is what keeps a late reply out.
- **A drain as long as the request timeout** (libmodbus's recovery mode sleeps the
  response timeout and flushes). Rejected: it would spend the whole budget of the next
  request.
- **Supporting broadcast.** Deferred: the guard has no send-only exchange, and a write
  without confirmation would always be `Unknown`.
- **Rejecting the `tcp` carrier.** Rejected because the TCP transport and the endpoint
  parser of `modbus-tcp` make it a few lines, and gateways are common.

## Compliance
- `crates/stonecrab/src/modbus/framing/rtu.rs` (CRC, `RtuAdu`, `RtuFramer`),
  `crates/stonecrab/src/modbus/rtu/` (the driver, the parameters and the silence, the
  pacer), `session.rs` (`Wire`), `connection.rs` (`open_on`), and the provided
  `Framer::resync_discards` used by `FramedTransport::resync`.
- Tests: `crates/stonecrab/tests/modbus_rtu.rs` (the SPEC.md RTU failure cases: a CRC
  error, the silence and the drain on the fake clock, a late reply that must not answer
  the next request, a lost serial link and its reopen, wrong-unit and wrong-function
  replies; the upstream vectors; the contract suite; RTU over loopback TCP), the unit tests
  of the framer, the parameters, the pacer and the session, and the fuzz target
  `fuzz_modbus_rtu`.
- `docs/parity/modbus-serial.md` records the RTU rows as implemented, deferred or
  divergent with their test ids; ASCII stays planned for S60.
