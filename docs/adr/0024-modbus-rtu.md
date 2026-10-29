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

## Addendum (S60, 2026-10-29): Modbus ASCII
### Status
Accepted (2026-10-29), as slice S60 of docs/POST_ALPHA_PLAN.md ("Modbus ASCII: framing and
LRC"; exit "Upstream ASCII vectors pass"). The decisions above stand and are not reopened;
this addendum adds a second serial framing to the same session, connection and PDU codec.
Every change to the public API is additive to the published `stonecrab` 0.2.0. No ADR of its
own is needed: nothing here changes a decision of another ADR.

### Context
Upstream's `ModbusAsciiDriver` (protocol code `modbus-ascii`) is the RTU driver with another
frame: the same carriers (`serial` by default, also `tcp`, `tls`, `tls-psk`, `udp`, `test`),
the same parameters, the same connection logic, a diffed copy. Its codec
(`ModbusAsciiMessageCodec`) frames on the first CR LF, gives up (DESYNCHRONIZED) when more
than 513 bytes are buffered without one, hex-decodes the text between the colon and the CR
LF (either case; an even number of digits, at least six) and lets the generated parser check
the LRC. It has no timing logic. The Go port (`MessageCodecAscii`) adds a resynchronization
that skips to the next colon. The two disagree, and the base class of the Java codec
(`MessageCodecBase`) differs from what the parity record assumed: a DESYNCHRONIZED result
drops the front byte and frames again (it is a resynchronization, byte-wise); a frame is
consumed before it is parsed, so a frame that fails to parse (no colon in front, a bad digit,
a wrong LRC) is lost and raises `MessageCodecException`; and the noise in front of a colon,
up to the first CR LF, is consumed together with the frame that follows it, so a good frame
behind noise on the same line is lost as well. Go keeps such a frame.

### Decision
1. **The driver and its carriers.** `stonecrab::modbus::ModbusAsciiDriver`, scheme
   `modbus-ascii`. `modbus-ascii://<port>` and `modbus-ascii:serial://<port>` open a serial
   port (the authority passed through as written); `modbus-ascii:tcp://host[:port]` carries
   ASCII frames over TCP to a serial-to-IP gateway, default port 502 as Go injects it (the
   Java driver has none: the divergence of decision 10 above). `tls`, `tls-psk`, `udp`,
   `test` and every other code are `InvalidConfig`. An injected transport factory replaces the
   carrier and the pacer still applies. The parameters are those of `modbus-rtu` (decision
   11 above), including unit 0 refused and the `serial.` keys on the serial carrier only,
   with one difference: `serial.data-bits` is 8 (the default, as upstream) or 7, which ASCII
   devices commonly use with even parity (the SOLO SLM1-C of HW2 ships as ASCII 7E1:
   `serial.data-bits=7&serial.parity=even`). Five and six are `InvalidConfig` (upstream hands
   any value to the port; the serial transport takes five to eight, but an ASCII character is
   seven bits and a Modbus byte two of them).
2. **One more wire, nothing duplicated.** `Wire` (crate-private) gains `Ascii`, placed after
   `Rtu`; it picks `AsciiFramer`, the scheme `modbus-ascii` and the 513-byte frame limit, and
   correlates and refuses broadcast as `Rtu` does (`wire != Tcp`). The session, the
   connection, the PDU codec and every path above them are unchanged for TCP and RTU (their
   tests stay as they were). The driver functions that parse and open a serial connection are
   shared by both serial drivers; the crate-private options type is renamed `SerialOptions`
   (it was `RtuOptions`). The public additions are `AsciiAdu`, `AsciiFramer` and
   `ModbusAsciiDriver`; no feature and no dependency is added (`modbus` contains the driver).
3. **The frame.** A colon, then the address, the PDU and the LRC as two hex characters per
   byte, then CR LF. The framer writes upper-case digits and reads either case (Java's
   `Character.digit` and Go's `encoding/hex` do; Go's comment says the protocol is specified with
   upper case). A frame ends at the first CR LF after its colon; a lone CR or LF is not a
   terminator, and a CR not followed by LF is a character that is no hex digit. A frame is at
   least 9 bytes (a colon, three bytes of hex and CR LF: the address, a function code, the
   LRC) and at most 513 (`AsciiAdu::MAX_BYTES`: the colon, a 253-byte PDU with the address and
   the LRC at two characters each, CR LF). `AsciiAdu` is the address and the PDU; the LRC is
   checked on receive and computed on send, as the CRC is for RTU.
4. **The LRC.** The two's complement of the sum of the address and the PDU bytes modulo 256,
   so the sum of every byte of the frame, the LRC included, is 0. Upstream's definition is
   `StaticHelper.asciiLrcCheck` (Java, with Go's port): add the bytes into an 8-bit
   accumulator, negate. A frame with a wrong LRC is not a frame.
5. **The size bound.** Past 513 bytes without a CR LF there is no frame: `decode` reports
   `Oversized` for a colon whose hex has run to the 513th byte without ending, and for a
   buffer of more than 513 bytes that holds no colon at all (upstream's DESYNCHRONIZED, which
   waits for the 514th byte; a frame cannot be completed after the 513th, so the framer says so
   one byte earlier, a recorded divergence). Up to 512 bytes of an unterminated run are
   waited for. A run of hex that is only long and has its CR LF inside the bound is judged
   whole.
6. **Resynchronization (MBS-ASCII-RESYNC).** Decided: Go's rule inside one buffer, Stonecrab's
   fail-fast between requests, as RTU. `decode` judges each colon of the buffer in turn. A
   colon that starts a whole frame with a right LRC gives that frame, and everything before the
   colon (noise, the debris of a frame that never finished) leaves the buffer with it, so noise
   and a broken frame in front of a good one never lose it (Java loses it). A colon that starts
   something that cannot be a frame (a character that is no hex digit, a second colon before the
   CR LF, an odd number of digits, fewer than three bytes, a wrong LRC) is skipped and the next
   colon is judged: the next frame start is the colon, which cannot appear inside a frame. A
   colon that starts a frame still arriving (no CR LF yet, and no later colon) is waited for,
   whatever came before it. When no colon holds a frame and none is still arriving, the first
   colon's verdict decides: `Malformed`, or `Oversized`; nothing is removed (the `Framer`
   contract). The request then fails at once with `MalformedData` (a wrong LRC, a bad digit; a
   read may be retried, a write whose echo failed is `Unknown` and never replayed) or
   `ResourceLimit` (an unterminated run), where Go skips a byte and waits for the deadline
   (decision 3 above). Between requests `resync_discards` is true, as for RTU: the tail of a
   frame the owner gave up on carries no colon to search from, and a byte-wise search would
   wait for a frame that never comes and close the port (decision 4 above); the buffer is
   dropped and counted in `GuardStats::resyncs`. The search for a colon is therefore a property
   of `decode`, not of the resynchronization.
7. **Timing (MBS-ASCII-TIMING).** Upstream has no inter-character timeout and no inter-frame
   timing for ASCII, and neither has this driver: a frame is bounded by its colon and its CR LF,
   so slow characters are accepted until the request's deadline (the serial line specification
   is remembered to allow up to a second between characters, UNVERIFIED as it is not checked
   in; nothing enforces it), and the t3.5 silence of RTU
   does not apply. The line is not watched before a request unless `serial.interframe-delay`
   asks for a gap (milliseconds, default 0), which for ASCII is the whole gap, not a floor under
   t3.5.
8. **Late replies (decision 6 above applies).** The drain stays. A reply that came too late
   has the right unit and function, and nothing in an ASCII frame tells it from the answer to
   the next request (there is no transaction id): its colon and CR LF only show that it is a
   whole frame, which is also what the answer looks like. After a send that nothing answered,
   the next request watches the line for the gap plus the time of the largest frame, 513
   characters instead of 256 (534 ms at 9600 8N1: a character is a start bit, the data bits
   (7 or 8), the parity bit and the stop bits), dropping what arrives; the watch is part of the
   request's deadline. Over the `tcp` carrier the line is not known and the drain is that of
   9600 8N1. The residual risk of decision 6 above is unchanged.
9. **Correlation, bus, broadcast, capabilities.** As RTU (decisions 7, 8, 9 and 12 above): a
   reply is accepted when its address and function code match, one request on the line, a port
   is not shared between connections, unit 0 is refused, and the capabilities are those of
   `modbus-tcp`.
10. **Evidence level.** `simulator-tested`: the fake transport on the fake clock, the upstream
    vectors (the parser suite's two ADUs, the codec test's frame and buffers, the Go codec
    test's frames, garbage and cases, transcribed with provenance), the contract suite over the
    fake transport, ASCII over a loopback TCP socket and a fuzz target
    (`fuzz_modbus_ascii`). No independent ASCII server exists yet (a socat pair with libmodbus
    or modbus-tk in ASCII mode is a later slice, as for RTU) and no hardware has been read
    (HW2).

### Consequences
- A reply with a wrong LRC or a bad digit fails its request where upstream (Go) waits; a read
  retry covers it. Noise or a broken frame in front of a good reply no longer loses it.
- The drain after an unanswered request is up to 534 ms at 9600 baud (4.3 s at 1200 baud): the
  default request timeout of 5000 ms leaves room for it and a short reply down to about 1200
  baud; slower lines need a longer `request-timeout-ms`, as for RTU.
- A noise-only buffer longer than 513 bytes fails the request with `ResourceLimit` and is
  dropped before the next one.
- With no gap by default, a frame that arrives while the line is idle is not dropped before
  the next request, except by the guard's supervisor inside a Tokio runtime (ADR-0024
  decision 6 above): a complete unsolicited frame from the right unit with the right function code would
  answer the next request. `serial.interframe-delay` makes the driver watch the line first,
  as RTU always does; the default stays upstream's 0.
- `Wire` and the options type are crate-private; the renamed `SerialOptions` is no API change.

### Alternatives considered
- **Following Java: byte-wise resynchronization and the frame lost with its noise.**
  Rejected: it loses a good reply behind noise on the same line, which Go's colon rule and
  the specification's start marker avoid.
- **Searching the buffer for a colon in `FramedTransport::resync`.** Rejected, as for RTU
  (decision 4): the tail of an abandoned frame carries no colon, and a frame begun by a colon
  would be waited for until the deadline, which closes the port.
- **Skipping a broken frame and waiting for the deadline (Go).** Rejected for the request
  path, as for RTU (decision 3): nothing retransmits on Modbus.
- **A character timeout of one second.** Rejected: upstream has none, and the request
  deadline already bounds the wait; a timeout that ends a frame mid-way would only add a
  second kind of failure.
- **No drain for ASCII because its frames have markers.** Rejected: the markers tell a frame's
  extent, not whose answer it is.
- **A separate ASCII session and connection, or a generic over the framer.** Rejected, as for
  RTU: the `Wire` choice is a few lines, the rest is shared.

### Compliance
- `crates/stonecrab/src/modbus/framing/ascii.rs` (the LRC, `AsciiAdu`, `AsciiFramer`),
  `crates/stonecrab/src/modbus/rtu/ascii.rs` (the driver), `rtu/driver.rs` (the parsing and
  the connecting both serial drivers share), `rtu/line.rs` (the parameters and the silence),
  `session.rs` (`Wire::Ascii`).
- Tests: `crates/stonecrab/tests/modbus_ascii.rs` (the upstream vectors, an LRC error, bad
  digits, a missing CR or LF, lower-case hex, a reply split across reads and two in one, noise
  before the colon, the size bound, a reply from another unit, a late reply and the drain,
  slow characters, a lost link, the contract suite, ASCII over a TCP gateway), the unit tests
  of the framer and of the parameters, and the fuzz target `fuzz_modbus_ascii`.
- `docs/parity/modbus-serial.md` records the ASCII rows as implemented, with the divergences
  of decisions 5 and 6 and the LRC fail-fast.

## Addendum (S59b, 2026-10-29): RTU against an independent peer
### Status
Accepted (2026-10-29), as slice S59b of docs/POST_ALPHA_PLAN.md. It amends decision 14 above
(evidence level, "no interop peer exists yet") and changes no other decision. No code under
`crates/stonecrab/src` changes.

### Decision
1. **The peer.** The diagslave 3.6 image of S61 (ADR-0027) is run a second way: `diagslave -m
   enc -p 1503 -a 1`, Modbus RTU encapsulated in TCP, which is the stream of the `tcp`
   carrier (decision 10 above). `serve.sh` takes the mode; `interop.sh` reads
   `INTEROP_DIAGSLAVE_MODE`; the container, the host port (TCP 1503), the test file and the
   evidence directory are the mode's own, so the UDP job is unchanged and the two never
   collide. The CI job is `interop-rtu`; `mise run interop-rtu` runs it and `mise run
   interop` includes it, because TCP works where UDP does not forward.
2. **One unit, `-a 1`.** Without `-a` diagslave answers every unit id and the
   request-to-an-unserved-unit case cannot be observed. With `-a 1` a frame for another unit
   (and one with a wrong CRC) makes diagslave close the connection with no reply: the driver
   reports `Disconnected`. That is recorded as the peer's behavior, not as a rule of the
   driver; a unit that is merely silent is still `Timeout` (decision 7 and the fake-transport
   tests). The other tests need one unit only.
3. **Evidence level.** Over the `tcp` carrier, reads of every area, writes read back (the
   driver sends FC06 or FC16 for registers and FC15 for coils; diagslave's log does not tell
   05 from 15 or 06 from 16), a range selection, requests and replies of 255 bytes, ping,
   exception 02 (for function codes 20 and 21, which diagslave refuses always) and an
   unserved unit are `interoperability-tested`, against a closed-source peer whose RTU
   framing shares no code with Stonecrab. The first run was local (colima, 9 of 9, 2026-10-29);
   the first hosted run is the next evidence. The serial carrier, the silence and the drain
   (a gateway has no line), a bad CRC from the slave and ASCII are not touched: they stay
   `simulator-tested`. The input registers and discrete inputs of diagslave alias its holding
   registers and coils, so the tests read them at cells written through the other area.

### Compliance
- `crates/stonecrab/tests/modbus_interop_diagslave_rtu.rs`, `interop/diagslave/`,
  `docs/parity/modbus-serial.md` (rows `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING`,
  `MBS-TRANS-TCP`, `MBS-RTU-CRC` and `MBS-RTU-WIRE-MODEL` carry `automated-verified` for the
  RTU half over a gateway).

## Addendum (2026-10-29): the first hosted run of `interop-rtu`

The hosted job ran with commit 5858730 (CI run 40838776577) on `ubuntu-latest`: the diagslave
image in its encapsulated mode (`-m enc -p 1503 -a 1`) and all nine `interop_rtu_*` tests
passed. The `automated-verified` statuses of the S59b addendum and the
`interoperability-tested` level of RTU over a TCP gateway now rest on that run as well as
on the first local one.

## Addendum (S60b, 2026-10-29): ASCII, and RTU on a serial device, against an independent peer
### Status
Accepted (2026-10-29), as slice S60b of docs/POST_ALPHA_PLAN.md. It amends the evidence level
of the ASCII driver (addendum S60, "simulator-tested only") and extends addendum S59b; it
changes no other decision. No code under `crates/stonecrab/src` changes.

### Decision
1. **The peer, on a serial device.** The diagslave 3.6 image of S61 and S59b gets two more
   modes, `ascii` (`diagslave -m ascii -a 1 -b 19200 -d 7 -s 1 -p even`) and `rtu-pty`
   (`diagslave -m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20`). In both, socat (Debian
   package 1.8.0.3-1+deb13u1, by exact version, like iproute2) makes a pseudo-terminal pair
   inside the container, diagslave opens one end as its serial port and socat bridges the
   other end to TCP port 1504. The drivers' `tcp` carrier (decision 10 of the base decision)
   therefore reaches the framing diagslave uses on a line: the colon, hex and LRC of ASCII,
   and RTU frames cut by the gap between characters (`-f 20`, the largest tolerance diagslave
   takes; with its default of 0 a request written in pieces 10 ms apart gets no reply, a
   request written as one frame is answered). `-m enc` stays the peer of S59b.
2. **One master.** The bridge serves one TCP connection at a time (`max-children=1`): a
   serial line has one master, and two connections would share one reply stream. The tests
   that need two clients in sequence close the first before opening the next.
3. **The unserved unit is silence on a line.** With `-a 1` diagslave closes the connection
   on a frame for another unit in `-m enc` (addendum S59b), and answers nothing at all on a
   serial device, in `-m ascii` and `-m rtu` alike: the driver reports `Timeout` at the
   deadline, which is what decision 7 says of silence. Each of the two RTU runs has its own
   test of the case and `interop.sh` leaves the other out.
4. **Evidence level.** Over the `tcp` carrier, reads of every area, writes read back, a range
   selection, requests and replies of 511 characters in ASCII (255 bytes in RTU), ping,
   exception 02 and an unserved unit are `interoperability-tested` for ASCII (job
   `interop-ascii`) and for RTU on a serial device (job `interop-rtu-pty`), as they are for
   RTU on `-m enc`. The driver's own serial port, the line settings, the silence before an
   RTU request, the reopen of a lost port and a device stay `simulator-tested` (a pseudo-terminal
   is also not a serial port of the driver: the carrier is `tcp`). The first runs were local
   (colima, 9 of 9 each, 2026-10-29); the first hosted runs are the next evidence.

### Compliance
- `crates/stonecrab/tests/modbus_interop_diagslave_ascii.rs`,
  `crates/stonecrab/tests/modbus_interop_diagslave_rtu.rs`, `interop/diagslave/`,
  `docs/parity/modbus-serial.md` (rows `MBS-ASCII-SEND`, `MBS-ASCII-LRC` and
  `MBS-ASCII-WIRE-MODEL` carry `automated-verified`; `MBS-OP-READ`, `MBS-OP-WRITE`,
  `MBS-OP-PING` and `MBS-TRANS-TCP` carry it for the ASCII half too).

## Addendum (2026-10-29): the first hosted runs of `interop-ascii` and `interop-rtu-pty`

Both jobs of the S60b matrix ran with commit fc30f8c (CI run 40843548924) on
`ubuntu-latest` and passed nine of nine each: diagslave behind socat as an ASCII slave and as
a gap-framed RTU slave, reached through the drivers' TCP gateway carrier. The evidence the
S60b addendum called the next step now exists; a serial port of the drivers is still the
open item.
