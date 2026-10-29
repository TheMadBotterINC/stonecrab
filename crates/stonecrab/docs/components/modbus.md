# Stonecrab Modbus
Native Modbus TCP and Modbus RTU drivers for Stonecrab (alpha), `stonecrab::modbus`, behind
the `modbus` feature (on by default). The alpha scope is the function codes 01 (read coils),
02 (read discrete inputs), 03 (read holding registers) and 04 (read input registers) over
TCP with MBAP framing, writes of coils and holding registers (function codes 15, 06 and 16,
refused unless the connection says `access=read-write`), the extended registers through the
file record function codes 20 (read) and 21 (write) since S50, a ping that reads one
address, and polling subscriptions (cyclic and change-of-state, since S55). Since S59 the
same operations run over Modbus RTU on a serial line or a serial-to-IP gateway (see "Modbus
RTU" below). ASCII, TLS, UDP, string types, browse and discovery are out of scope for the
alpha and are listed under "Not in the alpha" below.
The TCP driver has been checked against an independent server, pymodbus 3.8.6, and no
physical device; the RTU driver on the in-memory transport and over a loopback socket only.
No independent server serves the file record function codes, so the extended registers are
checked against the specification examples, the upstream PLC4X frames and a first-party
device on the in-memory transport only.

This module offers `ModbusTcpDriver`, the `Driver` of the `modbus-tcp` scheme, with
`ModbusOptions`, the parsed connection parameters; `ModbusConnection` with
`ConnectionDefaults`, the `Connection` that reads tags in batches over a session;
`ModbusSession` with `SessionStats` and `exception_item_error`, which send one read
request over a connection and return the reply that answers it; and the layers
underneath: `UnitId`, `ModbusArea`, `ModbusDataType`, `AddressConvention` and
`ModbusAddress` (built with its builder or parsed from text with `ModbusAddress::parse`),
`Batcher`, `BatchLimits`, `Chunk` and `Member`, the PDU types `FunctionCode`,
`ExceptionCode`, `ModbusPdu`, `FileRecordRead` and `FileRecord` with `encode_request`,
`decode_request`, `decode_response` and `encode_response`, the framing types `MbapHeader`,
`TransactionId`, `ModbusAdu` and `MbapFramer`, the RTU framing types `RtuAdu` and
`RtuFramer`, `ModbusRtuDriver`, the `Driver` of the `modbus-rtu` scheme, and `TypedDecoder`,
which turns registers or coil bits into `PlcValue`s. `SystemClock`, the Tokio-backed clock
the drivers use by default, is re-exported from `stonecrab::transport`. The `modbus` feature
turns on the `transport` feature (and with it Tokio), the `serial` feature (the serial
transport, for RTU) and `tracing`; the codec module is always built.

## Driver

`ModbusTcpDriver::new()` uses the real TCP transport and the Tokio-backed clock, so
connections must be driven inside a Tokio runtime with the time driver enabled;
`with_clock` and `with_transport_factory` replace them, which is how the repository's
tests run the driver over an in-memory transport and over loopback sockets.

- `parse_config` reads the connection string below into a `ConnectionConfig` that carries
  the parsed `ModbusOptions` in its driver-options slot; `connect` reads them back.
- `parse_address` is `ModbusAddress::parse` with the address convention of the options.
  Take the options from the configuration with `config.address_options()` before
  `connect` consumes it.
- `capabilities` is the value every connection reports: read, write, ping and polling
  subscriptions, no browse or discovery, one operation at a time, PDUs up to 253 bytes.
  Whether a connection may write is its access policy, read-only unless the connection
  string says `access=read-write`.

### Connection strings
`modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]`, followed by an optional
`?key=value&key=value`. The host is a name, an IPv4 address or an IPv6 address in brackets
(`[::1]:502`); the port is 1 to 65535 and defaults to 502; `tcp` is the only transport code
(`udp`, `tls` and the rest are `InvalidConfig`). The text may not contain whitespace,
control characters or `%`, nor a `user:password@` part, and is at most 4096 bytes.

The six upstream parameters, the shared `address-convention` key and Stonecrab's `access`
key are the only keys. A
key may appear once, a value may not be empty, and every other key is `InvalidConfig`
(upstream warns and ignores it; `MB-CFG-UNKNOWN`). That includes the misspelt
`unit-identifier` that the upstream manual test passes. An error about a parameter value
or an unknown key names the key, and no error repeats the connection string.

| Key | Default | Accepted | Row |
| --- | --- | --- | --- |
| `request-timeout-ms` | 5000 | whole number of milliseconds above 0; also the config's `Timeouts.request` | `MB-CFG-REQUEST-TIMEOUT` |
| `default-unit-identifier` | 1 | 0 to 255 (upstream has no bound; divergence) | `MB-CFG-UNIT-ID` |
| `ping-address` | `4x00001:BOOL` | an address of the grammar below, read under the `plc4x` convention; the address `Connection::ping` reads once | `MB-CFG-PING-ADDRESS` |
| `default-payload-byte-order` | `BIG_ENDIAN` | `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP` | `MB-CFG-BYTE-ORDER` |
| `max-coils-per-request` | 2000 | 1 to 2000 | `MB-CFG-MAX-COILS` |
| `max-registers-per-request` | 125 | 1 to 125 | `MB-CFG-MAX-REGISTERS` |
| `address-convention` | none | `plc4x`, `one-based-40001` or `zero-based` | `MB-ADDR-BARE-40001`, `MB-ADDR-CONVENTIONS` |
| `access` | `read-only` | `read-only` or `read-write`; `read-write` is what lets `write` reach the wire (a Stonecrab parameter, upstream has no access policy) | `MB-CFG-ACCESS` |

Numbers are decimal digits only: no sign, no space, no `0x`. `address-convention` is the
explicit-convention rule of the address grammar below: the bare short forms such as
`40001` parse only under one of the three names, and the driver hands the name to every
`parse_address` call through `ConnectionConfig::address_options()`. Without it only the
long forms (`holding-register:1`) and the prefixed short forms (`4x00001`) parse, read
one-based as under `plc4x`.

### Byte order names
The four upstream names set the byte and word order a value uses when neither its tag nor
its address says (ADR-0006). The tag's own order wins over the address's `byte-order`
block, which wins over this parameter.

| `default-payload-byte-order` | Byte order | Word order | 4-byte layout |
| --- | --- | --- | --- |
| `BIG_ENDIAN` | big | high word first | `[1,2,3,4]` |
| `LITTLE_ENDIAN` | little | low word first | `[4,3,2,1]` |
| `BIG_ENDIAN_BYTE_SWAP` | little | high word first | `[2,1,4,3]` |
| `LITTLE_ENDIAN_BYTE_SWAP` | big | low word first | `[3,4,1,2]` |

```rust
use stonecrab::{ByteOrder, Driver, WordOrder};
use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver, UnitId};

let driver = ModbusTcpDriver::new();
let config = driver.parse_config(
    "modbus-tcp://192.0.2.10:1502?default-unit-identifier=7&request-timeout-ms=800\
     &default-payload-byte-order=LITTLE_ENDIAN&address-convention=plc4x",
)?;
let options = config.driver_options_as::<ModbusOptions>().expect("attached by parse_config");
assert_eq!(options.unit_id, UnitId::from(7));
assert_eq!(
    (options.byte_order, options.word_order),
    (ByteOrder::LittleEndian, WordOrder::LowWordFirst)
);

// The address options come from the configuration, before `connect` consumes it.
let address = driver.parse_address("40001:UINT", &config.address_options())?;
assert_eq!(stonecrab::ProtocolAddress::canonical(&*address), "holding-register:1:UINT");

// A misspelt key is an error, not a silent default.
assert!(driver.parse_config("modbus-tcp://192.0.2.10?unit-identifier=7").is_err());
# Ok::<(), stonecrab::PlcError>(())
```

`parse_config` starts from the defaults given to `ModbusTcpDriver::with_defaults` (the
upstream defaults on `ModbusTcpDriver::new()`) and the connection-string parameters
override them; the full rule is driver defaults, then the connection string, then the
address block, then the tag. A configuration built by hand with `ConnectionConfig::builder`
carries no options, and `connect` then uses the driver's defaults.

The session and the connection do I/O only through a transport factory the caller
passes in, so the module's tests run on the in-memory transport of the `testkit` feature
and, for the driver, over loopback sockets to a scripted peer. The
common driver contract suite passes against the driver over both: over the in-memory
transport with the unsupported-write and unsupported-ping cases skipped by name (the driver
writes and pings), and over loopback sockets with those two and the four timing cases
skipped by name (the loopback fixture cannot swallow a reply on a deterministic clock). The module builds on the core, `stonecrab::codec` and
`stonecrab::transport` (`TcpFactory`, the `Framer` trait, `ConnectionGuard` and
`TransportFactory`), `bytes` and `tracing`.

Upstream behavior is the Apache PLC4X Modbus TCP driver at the commit pinned in
`upstream/manifest.toml`. Every claim about it, and every divergence, is recorded with a
stable row id in the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md).

## Modbus RTU
`ModbusRtuDriver`, scheme `modbus-rtu` (ADR-0024), runs the connection, the session, the PDU
codec, the address grammar and the parameters of `modbus-tcp` over RTU framing: the
address byte, the PDU and a CRC-16 (polynomial 0xA001 reflected, initial value 0xFFFF, low
byte first). What differs:

- **Carriers.** `modbus-rtu://<port>` or `modbus-rtu:serial://<port>` opens the serial port
  named by the text after `://`, as written (`modbus-rtu:///dev/ttyUSB0`,
  `modbus-rtu://COM3`); `modbus-rtu:tcp://host[:port]` sends RTU frames over TCP to a
  serial-to-IP gateway, default port 502. `tls`, `tls-psk`, `udp` and `test` are
  `InvalidConfig`.
- **Line parameters**, on the serial carrier only, under the `serial.` prefix:
  `serial.baud-rate` (default 9600), `serial.data-bits` (8, the only value),
  `serial.stop-bits` (1 or 2), `serial.parity` (`none`, `odd`, `even`),
  `serial.flow-control` (`none`, `rts-cts`, `xon-xoff`), `serial.interframe-delay`
  (milliseconds, default 0) and `serial.reuse-port` (`false` only). Value names are
  matched without regard to case and with `_` read as `-`. `default-unit-identifier`
  takes 1 to 255.
- **Framing.** A reply is sized by its function code and checked by its CRC. A reply with
  a wrong CRC fails its request with `MalformedData` at once (a read may be retried by the
  retry policy, a write is `Unknown`), and the bytes are dropped before the next request.
  Noise in front of a whole reply is skipped.
- **Correlation.** There is no transaction id: a reply is accepted when its unit and
  function code match the request, and is then proved against it as for TCP. A reply from
  another unit is discarded and the wait goes on.
- **Silence.** Before each request the line is watched until it has been quiet for 3.5
  character times (1.75 ms above 19200 baud) or `serial.interframe-delay`, whichever is
  longer; what arrives meanwhile is dropped. After a request nothing answered, the watch
  is longer by the time of a 256-byte frame, so a late reply is heard out and dropped
  instead of answering the next request. The watch is part of the request's deadline.
  Over a gateway the line is not known: there is no gap, and the drain after an
  unanswered request is that of a 9600 8N1 line (270 ms).
- **One connection per port, no broadcast.** The connection runs one operation at a time;
  a port is not shared between connections. Unit 0 is the broadcast address, which no
  server answers: it is refused before any I/O.

```
use stonecrab::modbus::ModbusRtuDriver;
use stonecrab::{Driver, Endpoint};

let driver = ModbusRtuDriver::new();
let serial = driver
    .parse_config("modbus-rtu:///dev/ttyUSB0?serial.baud-rate=19200&serial.parity=even")?;
assert_eq!(serial.endpoint(), &Endpoint::Custom("/dev/ttyUSB0".to_owned()));

let gateway = driver.parse_config("modbus-rtu:tcp://192.0.2.20")?;
assert_eq!(
    gateway.endpoint(),
    &Endpoint::Tcp { host: "192.0.2.20".to_owned(), port: 502 }
);
assert!(driver.parse_config("modbus-rtu://COM3?default-unit-identifier=0").is_err());
# Ok::<(), stonecrab::PlcError>(())
```

| RTU capability | Support level |
| --- | --- |
| Read, write, ping and subscribe by polling over RTU framing, with the operations and limits of `modbus-tcp` | simulator-tested |
| CRC errors, the silence before a request, the drain of a late reply, a lost serial link and its reopen, replies from another unit | simulator-tested |
| RTU over a serial-to-IP gateway (`modbus-rtu:tcp://`) | simulator-tested (a loopback socket) |
| Interoperability against an independent RTU server | planned (a virtual serial pair with libmodbus or modbus-tk) |
| Hardware verification on RS-485 | planned (HW2) |

The RTU rows of the
[serial parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus-serial.md)
record every behavior and divergence with its test ids.

## Support levels
The labels are those of the project's hardware-evidence policy: `build-tested`,
`simulator-tested`, `interoperability-tested` and `hardware-verified`, plus `planned` and
`not supported`. `simulator-tested` covers the in-memory transport and the loopback
scripted peer. The `interop` CI job (slice S27) runs the driver against an independent
server, pymodbus 3.8.6, on every push, and the `interop-libmodbus` job (slice S52a) against a
second one, libmodbus 3.1.11's `unit-test-server` (see "Interoperability" below); the rows
they exercise read `interoperability-tested`. The `interop-toxiproxy` job (slice S52b) adds
fault injection (Toxiproxy, `docker pause` and `kill`) against both peers. No physical
device has been tried.

| Capability | Support level |
| --- | --- |
| Read: coils, discrete inputs, holding registers and input registers (function codes 01 to 04) as the 15 numeric and bit types | interoperability-tested |
| Array selections `[i]`, `[lo..hi]` and `;base`: a range read and written as one `PlcValue::List`, on every area (the extended registers included) | simulator-tested |
| Ping: one read of `ping-address` (`Connection::ping`) | interoperability-tested |
| Write: coils and holding registers (function codes 15, 06 and 16), `Unknown` after a request that went out unanswered | interoperability-tested |
| Extended registers: read by function code 20 and write by function code 21, cut at the end of each 10000-register file (`extended-register:N`, `6x`, bare `6`) | simulator-tested |
| Subscribe by polling: cyclic and change-of-state, event refused (`Connection::subscribe`) | simulator-tested |
| Browse and discover | not supported |
| Connection string and parameters (`parse_config`) | simulator-tested |
| Connect, close and connection state | interoperability-tested |
| Reconnect (in the background when opened in a Tokio runtime, else request-driven), the idle bound, read retry, cancellation, timeout and late-reply discard | simulator-tested |
| Batched reads with split disclosure | interoperability-tested |
| Interoperability against an independent server | interoperability-tested |
| Hardware verification | planned |

The layers underneath, with their evidence:

| Layer | Evidence |
| --- | --- |
| Address types: `UnitId`, `ModbusArea`, `ModbusDataType`, `ModbusAddress` | unit-tested; property-tested; fuzzed |
| Address text grammar and parser (`ModbusAddress::parse`) | unit-tested; fuzzed |
| Connection-string parser (`ModbusTcpDriver::parse_config`) | unit-tested; property-tested; fuzzed |
| PDU codec for function codes 01 to 04, 06, 15, 16, 20 and 21 and the exception PDU | unit-tested; vector-tested; property-tested; fuzzed |
| MBAP framing (`MbapFramer`, `MbapHeader`, `ModbusAdu`, `TransactionId`) | unit-tested; vector-tested; fuzzed |
| RTU framing and CRC-16 (`RtuFramer`, `RtuAdu`) | unit-tested; vector-tested (the upstream RTU parser vectors and the specification's CRC examples); property-tested; fuzzed |
| Session: transaction ids, reply correlation by transaction id, unit id and function code, exception mapping (`ModbusSession`, `exception_item_error`) | unit-tested on the in-memory transport |
| Typed register decoding and byte order (`TypedDecoder`) | unit-tested; vector-tested; property-tested; `BIG_ENDIAN` and `LITTLE_ENDIAN` confirmed by the retained upstream PLC4X vectors; the two byte-swap modes are derived from the documented layouts |
| Read batching (`Batcher`) | unit-tested; property-tested |
| Connection and driver (`ModbusConnection`, `ModbusTcpDriver`) | unit-tested on the in-memory transport; contract-suite-tested on the in-memory transport and over loopback sockets; one read run over loopback; ten reads, two pings and the contract suite against pymodbus 3.8.6; ten tests against libmodbus 3.1.11 |

## Not in the alpha
Each item is a deferred row of the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md)
with its owner. Where the parser or driver can see the request it rejects it with a typed
error rather than ignoring it; operations that have no API at all do not exist:

- a write of a coil with function code 05 (upstream never sends it, `MB-QUIRK-C`); the
  other writes, extended registers included, are in the driver (see "Connection");
- browse and discovery (`MB-TCP-BROWSE`, `MB-TCP-DISCOVER`): these operations do not
  exist in the API; subscriptions poll since S55 (`MB-TCP-SUBSCRIBE`, see "Connection");
- `STRING`, `WSTRING`, `CHAR`, `WCHAR`, the `TIME` and `DATE` family, a selection of more
  than one dimension and the legacy array form (`MB-ADDR-*`, `MB-TYPE-*`);
- the extended registers on any independent server: none of pymodbus 3.8.6, libmodbus
  3.1.11 and modbus-tk serves function codes 20 and 21 (`MB-ADDR-EXTENDED` is
  `implemented`, not `automated-verified`);
- UDP, TLS and TLS-PSK transports (`MB-TCP-TRANSPORTS-*`); Modbus ASCII is slice S60, and
  the RTU items that are deferred (broadcast, shared ports, the serial read and write
  timeouts, DTR and RTS) are listed in the serial parity record;
- the upstream gap-filling read optimizer, by decision (S51, ADR-0015 addendum): Stonecrab
  merges adjacent or overlapping tags only (`MB-READ-GAP`), because reading through a hole
  in a device's map is refused with exception 02 for every tag of the request; and
  upstream's lack of reconnect: Stonecrab reconnects in the background when the connection
  is opened inside a Tokio runtime (S57, `MB-READ-RECONNECT`);
- the two byte-swap modes (`BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP`) as upstream
  vectors: the retained PLC4X files configure only `BIG_ENDIAN` and `LITTLE_ENDIAN`, so
  those two rows stay `implemented`, not `automated-verified`;
- a write whose confirmation is lost on demand against an independent server: the fault
  is injected on the in-memory transport and over loopback, and a stock pymodbus cannot
  lose a confirmation.

## Interoperability
The `interop` job of the repository's CI builds `interop/pymodbus/` (a first-party Modbus
TCP server on pymodbus 3.8.6, `python:3.12-slim` pinned by digest, serving the register
map of `fixtures/modbus/interop/register-map.toml`), starts it on `127.0.0.1:5020` and
runs fifteen tests of `tests/modbus_interop_pymodbus.rs` plus the `modbus_tcp_read` example
against it. The tests are `#[ignore]`d and run only when `STONECRAB_INTEROP_MODBUS` names the
server, so `cargo test` needs no container and no network.

To run it locally you need Docker; from the repository root:

```sh
mise run interop
```

It builds the image, starts the container, waits for the port, runs the tests and the
example, writes the evidence to `target/interop-evidence/` (image id, `pip freeze`, server
log, the contract report, the test and example output) and removes the container. Without
`docker` on the path it stops with a message saying so. `interop/pymodbus/README.md` has
the manual commands.

What the run verifies against the independent server:

- All 15 holding-register types decode to the documented values (registers 1 to 31), plus
  coils 1 to 10, discrete inputs 1 to 10 and input registers 1 to 7, with the wire
  address one below the logical address.
- The little-endian REAL at registers 300 and 301, by address block and by connection
  parameter.
- A read past the map yields a device exception (input register 22: exception 02, status
  code 2 on the item), and a good tag in another chunk still succeeds.
- Batches split at 125 registers and 2000 coils and say so in `split_into`; 125 registers
  and 2000 coils are one request each.
- The bare `40001` spelling is refused without an `address-convention` and reads with
  `plc4x`.
- A holding register (function 06) and a coil (function 15) are written and read back, the
  multi-register types (DINT, REAL, LREAL, and a REAL in the little-endian layout) land in
  the register words the map documents, and a write the server refuses (exception 02) is
  `Rejected` while the writes around it are confirmed.
- The common driver contract suite passes through `ModbusTcpDriver::new()`: 15 cases pass
  and 11 are skipped by name (the timing, disconnect and device-exception cases that need
  a fault hook a stock pymodbus does not have, the write whose confirmation is lost, and
  the two cases of an unsupported write and an unsupported ping, which a driver that
  writes and pings has nothing to prove for).

Not verified there: timing over a real clock, dropped links and device exceptions on
demand, and unit-id correlation (pymodbus answers every unit id). The decisions behind
the run are in ADR-0015, decision 16.

### A second peer: libmodbus 3.1.11
The `interop-libmodbus` job (slice S52a) builds `interop/libmodbus/`, a `debian:trixie-slim`
image pinned by digest that compiles the `unit-test-server` shipped in Debian's `libmodbus-dev`
3.1.11-2 (the C stack of the libmodbus test suite: it shares no code with Stonecrab or
pymodbus), starts it on `127.0.0.1:1502` and runs the ten tests of
`tests/modbus_interop_libmodbus.rs` against it, one at a time. The server serves one
connection and exits, and the image starts it again, so each test opens one connection and
what it writes is gone when it closes. `mise run interop` runs this peer after pymodbus and
`mise run interop-libmodbus` runs it alone; the evidence (image id, package version, server
log, test output) is in `target/interop-evidence/libmodbus/`. The expected values are the
constants of libmodbus's `unit-test.h`, transcribed in the test file with their source and
license.

What the run verifies against it, with unit id 17:

- The 22 input bits and the input register of `unit-test.h`; the coils and holding
  registers start at zero, take the 37 coils and three registers of `unit-test.h` by
  function codes 15 and 06 and read them back; a REAL written by function code 16 lands in
  the words `unit-test.h` lists for each of the four byte orders.
- Exception 06 at address 0x170 is a `DeviceException` item error with status 6, and the
  next read works.
- A reply with the wrong transaction id and unit id (address 0x171) is discarded and the read
  ends in `Timeout`; a 500 ms reply (0x172) is read inside a 2 s deadline, and past a 200 ms
  deadline it is a `Timeout`, its late reply is discarded and the next read gets its own;
  a reply sent one byte at a time (0x173, unit 255) is reassembled.
- A reply that holds one register for a two-register read, which the server sends for every
  such read, fails the read as `MalformedData` (category `Protocol`) and the connection
  goes on. The failure carries no raw bytes: the byte count is proved against the quantity
  before the registers are decoded.

Not verified: dropped links, a server that refuses a unit id, and anything the server does
not do (it never receives function code 07, which makes 3.1.11 exit without a reply).

## Address grammar
`ModbusAddress::parse(text, &AddressOptions)` reads the grammar of the upstream PLC4X Modbus
driver, with the divergences listed below. Everything is case-sensitive and decimal, and
under the default reading (no convention, `plc4x`, `one-based-40001`) the number is the
one-based logical address: the wire offset is the number minus one, so
`holding-register:16` reads wire address 15 and the first register is `holding-register:1`.
Under `zero-based` the number is the wire offset itself, see the conventions below.

| Form | Examples | Accepted |
| --- | --- | --- |
| long: `AREA:N` or `AREA:N:TYPE`, N has 1 to 9 digits | `holding-register:16:INT`, `coil:1` | always |
| prefixed short: `0x`, `1x`, `3x`, `4x` or `6x` plus 4 or 5 digits | `4x00016:INT`, `0x0010` | always |
| bare short: prefix digit `0`, `1`, `3`, `4` or `6` plus 4 or 5 digits | `40001`, `400001`, `10001` | only with an `address-convention` |

`AREA` is `coil`, `discrete-input`, `holding-register`, `input-register` or
`extended-register`, and the prefixes are `0x` coil, `1x` discrete input, `3x` input
register, `4x` holding register and `6x` extended register; there is no `2x`. `0x0010` is coil 10, a decimal number that happens to start with the coil prefix,
not hexadecimal. When no type is written it is `BOOL` for the bit areas (coil, discrete
input) and `INT` for the register areas. `TYPE` is one of the 15 upper-case names `BOOL`,
`BYTE`, `WORD`, `DWORD`, `LWORD`, `SINT`, `USINT`, `INT`, `UINT`, `DINT`, `UDINT`, `LINT`,
`ULINT`, `REAL`, `LREAL`; the bit areas accept `BOOL` only, and a type whose registers pass
wire address 65535 is refused (`LREAL` at `holding-register:65534`). The last address is
`holding-register:65536`, wire address 65535.

The explicit-convention rule: a bare number such as `40001` could be holding register 1
under the PLC4X convention or a zero-based wire address, so the parser accepts the bare
forms only when the caller named the convention (`AddressOptions` with a convention name,
from the connection option `address-convention` or a profile's `address_convention`).
Without it the error names the ambiguity and the two spellings that are always accepted:
`holding-register:1` and `4x00001`. A name other than the three below is an error, even
for the unambiguous spellings.

| `address-convention` | `AddressConvention` | The number counts from | Numbers | Forms |
| --- | --- | --- | --- | --- |
| none | | 1 | 1 to 65536 | long and prefixed short; bare refused |
| `plc4x` | `Plc4xCompatible` | 1 | 1 to 65536 | long, prefixed short, bare |
| `one-based-40001` | `OneBased40001` | 1 | 1 to 65536 | long, prefixed short, bare |
| `zero-based` | `ZeroBased` | 0 | 0 to 65535 | long, prefixed short, bare |

The convention governs every number in the text, whatever the form: the digits of
`AREA:N`, of `Nx` plus 4 or 5 digits, and the digits after the prefix digit of a bare
form. The prefix digit still selects the area (`0` coil, `1` discrete input, `3` input
register, `4` holding register). Counting from 1, the wire offset is the number minus one;
counting from 0 it is the number. So the first holding register is `holding-register:1`,
`4x00001` and `40001` under the first three rows and `holding-register:0`, `4x00000` and
`40000` under `zero-based`, and `holding-register:1` is the second register there. The
boundaries are the same in every form: 0 is refused except under `zero-based`, 65536 is
accepted except under `zero-based` and the digit group is 4 or 5 digits in the short forms
(so the number 10000 needs the five-digit group, `4x10000` or `410000`). `plc4x` and
`one-based-40001` read every text alike; they differ in what they say, `plc4x` that the
text follows the PLC4X grammar and `one-based-40001` only that the numbers are the
one-based Modicon numbers of a device manual. A profile or connection that uses a
zero-based map should select `zero-based` rather than subtract one by hand.

The extended registers are the one area that ignores the convention's offset. The number
is the register address, counted through all the files of the device, 10000 to a file:
`extended-register:1`, `6x00001` and (with a convention) `60001` are address 1, address 0
does not exist (upstream rejects it too) and the last address is 655349999, record 9999 of
file 65535. Address `A` is record `A % 10000` of file `A / 10000 + 1` (files count from 1 on
the wire), so a value that starts in the last registers of a file continues at record 0 of
the next, and one read or write carries it as one request of a sub-request per file. A chunk
of tags is at most 120 registers, not 125, so that the response fits the 245 data bytes of a
file record reply. The 15 types of the holding registers are accepted, `INT` by default.

```rust
use stonecrab::AddressOptions;
use stonecrab::modbus::ModbusAddress;

let address = ModbusAddress::parse("extended-register:10000:DINT", &AddressOptions::default())?;
assert_eq!(address.file_number().map(|file| file.get()), Some(2));
assert_eq!(address.offset(), 0, "the record in file 2");
assert_eq!(address.logical_address(), 10_000);
assert!(ModbusAddress::parse("extended-register:0", &AddressOptions::default()).is_err());
# Ok::<(), stonecrab::PlcError>(())
```

An address is the wire offset it names, so one register read under two conventions
compares equal and carries `AddressConvention::Plc4xCompatible`. The text `canonical()`
renders is always the one-based long form, which parses back to an equal address with no
convention, `plc4x` or `one-based-40001`; under `zero-based` the same text is one register
higher.

An address may end with a config block, directly after the type with no space before the
brace: `{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`. The two keys are `unit-id`, a decimal
integer from 0 to 255 (the one-byte MBAP field; upstream has no range check, so this is a
documented divergence), and `byte-order`, a quoted name (single or double quotes) from
`BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP` and `LITTLE_ENDIAN_BYTE_SWAP`. An
unquoted name is rejected: the upstream value pattern only matches quoted text, and
Stonecrab turns what would be a silently dropped pair into an error. Either key, both in
any order, or an empty block `{}` are accepted; spaces around keys, colons, values and
commas are ignored; unknown or repeated keys are errors.

Every form may carry a selection between the number and the type, in the upstream array
notation (`MB-ADDR-SELECTION`, ADR-0022): `[i]` names element `i` and reads a scalar,
`[lo..hi]` names elements `lo` to `hi` inclusive and reads one `PlcValue::List`, even
`[4..4]`, and `;base` inside either states the lower bound the indices count from
(default 0, so `[4..7;1]` is the elements at offsets 3 to 6). Element `k` starts `k` bits
after the address on a coil or discrete input and `k` times the type's register count
after it on a register area: `holding-register:1[4]:DINT` is wire offset 8 and the same
address as `holding-register:9:DINT`. A range is one address whose `elements()` is its
count; it renders canonically as `holding-register:1[0..1]:REAL`, the logical number of
its first element followed by `[0..N-1]`. The whole range must end at or before wire
address 65535 and fit one read, 2000 bits or 125 registers (63 `DINT`s are 126 registers
and are refused). Each element has the layout it has alone, so an 8-bit or `BOOL` element
on a register area takes a register of its own (upstream packs them, a recorded
divergence), and byte and word order apply to each element. A range is one tag to the
batcher: it merges with adjacent tags, counts whole against `max-registers-per-request` or
`max-coils-per-request` and gives one read item; a write takes a `PlcValue::List` of
exactly its count (see "Connection").

On the extended registers the number is the register address and a selection counts
registers from it the same way, through the end of a file (ADR-0022, addendum of
2026-10-29): `extended-register:1[4]:DINT` is address 9 and
`extended-register:9998[1]:DINT` is address 10000, record 0 of file 2. The number is
checked before the selection moves it, so `extended-register:0[1]` is refused. A range
must end at or before 655349999 and fit one file record read, 120 registers (upstream
allows 125, whose answer no device may send). It may cross the end of a file: it is read
by one function 20 request whose sub-responses are joined in order, so an element may take
its registers from two files, and written by one function 21 request cut the same way,
which carries 251 data bytes: every list inside one file fits, and up to 118 registers
across the end of a file.

Rejected with `ErrorKind::InvalidAddress`, each message naming the reason:

- empty input, input over 256 bytes, control characters, and any text no form matches;
- a bare short form without the explicit convention, and an unknown convention name;
- logical address 0, above 65536 (except the extended registers, see above), hexadecimal digits (`holding-register:0x0010`), more
  than 9 digits in the long form, other than 4 or 5 in a short form, `4x:INT` with no digits;
- lowercase or unknown type names (`holding-register:16:i16`);
- a type other than `BOOL` on a coil or discrete input (upstream fails these only at read
  time);
- a span past wire address 65535, a selection that starts or ends past it, and a range
  wider than one read (2001 bits, 126 registers, or 121 on the extended registers);
- extended-register address 0 (a selection does not move it to 1), above 655349999, or
  with a type or selection that passes it (upstream accepts up to 655359999, but the last
  10000 addresses would be file 65536, which the wire cannot name);
- a selection that is not `[i]` or `[lo..hi]` with an optional `;base` in decimal digits,
  has a second dimension (`[0..1][2]`, `[1,2]`), an upper bound below its lower bound, an
  index below its base or a number beyond `u32`;
- the legacy `AREA:N:INT[4]` and `AREA:N:INT:4` forms (the message gives the replacement
  `AREA:N[0..3]:INT`, `[0]` for a count of 1, and only when that replacement parses;
  otherwise the general example);
- the features deferred past the alpha: `STRING`, `WSTRING`, `CHAR`, `WCHAR` and
  `STRING(n)`, and the `TIME` and `DATE` family;
- a malformed config block: unterminated, unknown or repeated key, `unit-id` outside
  0..=255, unquoted or unknown `byte-order`.

Error messages echo at most 32 control-free characters of the input. The row ids behind
each rule are in the parity record, `MB-ADDR-*`, `MB-LIMIT-ADDR`, `MB-LIMIT-SPAN`,
`MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`, `MB-LIMIT-DIGITS` and `MB-CFG-UNIT-ID`.

```rust
use stonecrab::{AddressOptions, ProtocolAddress};
use stonecrab::modbus::ModbusAddress;

let options = AddressOptions::builder().convention("plc4x").build()?;
let address = ModbusAddress::parse("40016:REAL{unit-id: 10}", &options)?;
assert_eq!(address.offset(), 15);
assert_eq!(address.canonical(), "holding-register:16:REAL{unit-id: 10}");

// Without the convention the bare form is ambiguous and refused.
assert!(ModbusAddress::parse("40016", &AddressOptions::default()).is_err());
assert!(ModbusAddress::parse("4x00016", &AddressOptions::default()).is_ok());

// Under `zero-based` the same number is the wire offset: register 0 is the first.
let zero_based = AddressOptions::builder().convention("zero-based").build()?;
let first = ModbusAddress::parse("40000", &zero_based)?;
assert_eq!(first.offset(), 0);
assert_eq!(first, ModbusAddress::parse("40001", &options)?);
assert!(ModbusAddress::parse("40000", &options).is_err());
# Ok::<(), stonecrab::PlcError>(())
```

A `ModbusAddress` declares `ProtocolAddress::spans_multiple_words` from its own data type:
true for every type wider than one 16-bit register (`DWORD`, `DINT`, `UDINT`, `REAL` and
the 64-bit types), false for the 16-bit types and for a coil. A sensor profile's
`SensorProfile::validate_for` uses it to require a declared `byte_order` and `word_order`.

## PDU codec
A PDU is the function code and its data, without the MBAP header. `ModbusPdu` has the four
read requests (`ReadCoils`, `ReadDiscreteInputs`, `ReadHoldingRegisters`,
`ReadInputRegisters`, each `{ start, quantity }` with the zero-based wire address), the two
normal read responses (`ReadBitsResponse` holding the packed bytes exactly as received, and
`ReadRegistersResponse` holding the registers), the three write requests upstream emits
(`WriteMultipleCoils`, function 15; `WriteSingleRegister`, function 06;
`WriteMultipleRegisters`, function 16) with their responses (`WriteSingleRegisterResponse`
and `WriteMultipleResponse`, which echo the request) and `Exception { function, code }`.
Write Single Coil (05) is not a variant, because upstream never sends it
(`MB-QUIRK-C`): function codes 05, 20 and 21 are `FunctionCode::Unknown` and every decoder
refuses them. The codec only builds and reads PDUs; the connection puts the write requests
on the wire (see "Connection").

`ModbusPdu::write_request(area, start, quantity, payload)` picks the request for an
encoded payload the way upstream does: a coil is always function 15 (even one coil), a
holding-register payload of exactly 2 bytes is function 06 and any other is function 16;
discrete inputs and input registers cannot be written.

A read request is the function code, the start address and the quantity (5 bytes), with a
quantity of 1 to 2000 bits or 1 to 125 registers and `start + quantity` at most 65536
(`decode_request`, `encode_request`). A write request is the function code and the address
and value (function 06), or the start address, the quantity of 1 to 1968 coils or 1 to 123
registers, a byte count (`ceil(quantity / 8)` for coils, `2 * quantity` for registers, so
an odd register count is refused) and the data (functions 15 and 16). A read response is the
function code, a byte count of 1 to 250 (even for registers) and the data; a write response
echoes the request (the address and value, or the start address and quantity); an
exception response is the function code plus 0x80 and one of the ten exception codes
(`decode_response`, `encode_response`).

Every length is compared with its bound before anything is allocated, a PDU is at most 253
bytes, and input must hold exactly one PDU. A response does not repeat the request's
quantity, so `decode_response` cannot know whether the byte count fits it:
`ModbusPdu::check_reply_to(&request)` proves that, and the matching function, once the
session knows the request. For a write it proves that the echo repeats what was sent; a
mismatch is a typed `InvalidDiscriminator` naming the field (`echoed start address`,
`echoed quantity` or `echoed value`). Exception codes outside the ten that V1.1b
section 7 defines are refused as `InvalidDiscriminator` rather than carried
(`ExceptionCode::Unknown` exists so the number can still be named). The rows are
`MB-TCP-READ-*`, `MB-TCP-WRITE`, `MB-EXC-*` and `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`,
`MB-LIMIT-PDU` in the parity record.

```rust
use stonecrab::codec::{BoundedReader, BoundedWriter};
use stonecrab::modbus::{
    FunctionCode, ModbusPdu, decode_response, encode_request,
};

// Request registers 108 to 110 (wire address 107): V1.1b section 6.3.
let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
let mut writer = BoundedWriter::new(253);
encode_request(&request, &mut writer)?;
assert_eq!(writer.as_slice(), [0x03, 0x00, 0x6B, 0x00, 0x03]);

// The reply of the same section, checked against the request.
let reply = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
let response = decode_response(
    &mut BoundedReader::new(&reply),
    FunctionCode::ReadHoldingRegisters,
)?;
response.check_reply_to(&request)?;
assert_eq!(
    response,
    ModbusPdu::ReadRegistersResponse {
        function: FunctionCode::ReadHoldingRegisters,
        registers: vec![555, 0, 100],
    }
);
# Ok::<(), stonecrab::codec::ParseError>(())
```

```rust
use stonecrab::codec::{BoundedReader, BoundedWriter};
use stonecrab::modbus::{
    FunctionCode, ModbusArea, ModbusPdu, decode_response, encode_request,
};

// Write 000A and 0102 to two holding registers from wire address 1: V1.1b section 6.12.
// The 4-byte payload is not exactly 2 bytes, so the request is function 16.
let request = ModbusPdu::write_request(
    ModbusArea::HoldingRegister,
    1,
    2,
    &[0x00, 0x0A, 0x01, 0x02],
)?;
let mut writer = BoundedWriter::new(253);
encode_request(&request, &mut writer)?;
assert_eq!(
    writer.as_slice(),
    [0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02]
);

// The device echoes the start address and the quantity.
let reply = [0x10, 0x00, 0x01, 0x00, 0x02];
let response = decode_response(
    &mut BoundedReader::new(&reply),
    FunctionCode::WriteMultipleRegisters,
)?;
response.check_reply_to(&request)?;
# Ok::<(), stonecrab::codec::ParseError>(())
```

The golden vectors in `fixtures/modbus/tcp/` are written from the examples of the Modbus
Application Protocol Specification V1.1b or built to break the decoder; they are
repository fixtures, not part of the published package.

## MBAP framing
On Modbus TCP every PDU travels behind a 7 byte MBAP header: the transaction id, the
protocol id (always 0), a length that counts the unit id plus the PDU, and the unit id, all
big-endian. `MbapFramer` implements the transport module's `Framer` for `ModbusAdu`, an
`MbapHeader` plus the PDU as opaque `Bytes`; the PDU is decoded after the session has
matched the reply to its request. `decode` follows these rules:

- fewer than 7 bytes buffered is `Ok(None)`: wait for the header;
- the header is judged as soon as it is complete, before anything is allocated or
  awaited: a protocol id other than 0 or a length of 0 or 1 is `FrameError::Malformed`
  (the protocol id is checked first), and a length above 254, which would make the ADU
  larger than 260 bytes, is `FrameError::Oversized` carrying the declared ADU size;
- an incomplete body is `Ok(None)` and leaves the buffer untouched, so a fragmented ADU is
  reassembled by the caller feeding more bytes;
- a whole ADU is removed from the front of the buffer, exactly, and bytes behind it stay
  for the next call, so a coalesced read yields one ADU per call.

`encode` writes the header then the PDU. `ModbusAdu::new` is the only constructor and keeps
the halves consistent: a PDU of 1 to 253 bytes, protocol id 0 and the matching length, so
an ADU is 8 to 260 bytes and always encodes. `TransactionId` is the request counter: it
starts at 1, wraps from 0xFFFE to 1 as upstream does, and never yields 0 or 0xFFFF;
a reply's id is whatever the peer sent and is not restricted. The rows are
`MB-READ-FRAMING`, `MB-LIMIT-ADU` and `MB-READ-TXN` in the parity record.

```rust
use bytes::BytesMut;
use stonecrab::modbus::{MbapFramer, TransactionId};
use stonecrab::transport::Framer;

// A reply arrives in two pieces; the first 9 bytes are not a whole ADU yet.
let mut framer = MbapFramer::new();
let mut buffer = BytesMut::from(&[0x00, 0x03, 0x00, 0x00, 0x00, 0x09, 0x11, 0x03, 0x06][..]);
assert_eq!(framer.decode(&mut buffer)?, None);
buffer.extend_from_slice(&[0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]);

let adu = framer.decode(&mut buffer)?.expect("a whole ADU");
assert_eq!(adu.transaction_id(), 3);
assert_eq!(adu.pdu().len(), 8);
assert!(buffer.is_empty());

let mut ids = TransactionId::default();
assert_eq!(ids.next().get(), 1);
# Ok::<(), stonecrab::transport::FrameError>(())
```

The golden ADU vectors in `fixtures/modbus/tcp/adu_*` wrap the V1.1b example PDUs in the
header layout of the Modbus Messaging on TCP/IP Implementation Guide V1.0b, and add
the adversarial cases (lengths 255, 1 and 0, protocol id 1, a truncated header, a body that
has not arrived, a 261 byte ADU and a 1 KiB stream of seven coalesced ADUs).
## Session
`ModbusSession::new(config, factory, clock)` owns a
`ConnectionGuard<ModbusAdu, MbapFramer>` (the lifecycle, reconnect, supervised when
opened in a Tokio runtime, and the single in-flight slot of the transport module) and the transaction counter.
`connect`, `close`, `state`, `watch_state` and `stats` pass through to the guard. `request(&pdu, unit_id, deadline, token)` encodes a read
request, sends it with the next transaction id (1 to 0xFFFE, continuing across a
reconnect) and returns the PDU that answers it:

- a reply is accepted only when its transaction id, its unit id and its function code (the
  reply's function with the exception bit cleared) all match the request; upstream
  compares the transaction id alone (`MB-READ-MATCH`, a divergence);
- any other reply is discarded, counted in `SessionStats` and logged at `warn` level with
  `expected_txn` and `received_txn`; the session then reads on within the same deadline.
  `late_replies` counts a reply whose transaction id belongs to a request the session gave
  up on (a timeout, a cancellation or a dropped request after the frame went out, a lost
  connection; the last 32 ids are remembered); `mismatched_replies` counts every other
  discarded reply;
- the accepted PDU is decoded with `decode_response` and proved against the request with
  `ModbusPdu::check_reply_to`; one that does not decode or whose byte count does not fit
  the quantity is `ErrorKind::MalformedData` with the `Protocol` category, and the stream
  stays aligned because the framer cut exactly one ADU;
- a device exception is not an error of the exchange: it is `Ok(ModbusPdu::Exception)`,
  the connection stays `Connected`, and `exception_item_error(function, code)` gives the
  per-member `ItemError` (code 1 `Unsupported(Operation::Read)`, code 2 `InvalidAddress`,
  every other code `DeviceException`; the raw code and the V1.1b name are always in the
  `ProtocolStatus`);
- every error carries `Diagnostics` with `operation = Read`, the connection state, the
  latency and the reconnect attempt, and the request has a `debug` span with the fields
  `scheme = "modbus-tcp"`, `endpoint` (host and port), `op = "read"` and `txn`.

A session runs one request at a time; a second one made meanwhile fails with
`ErrorKind::Backpressure` before any I/O. Resynchronization after a timeout, a
cancellation or a framing error is the guard's: it discards the unaccounted bytes within
the next request's deadline and reconnects if that fails.

```rust
# #[cfg(feature = "testkit")]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use std::time::Duration;
use stonecrab::{Clock, Deadline, Endpoint, ResourceLimits};
use stonecrab::modbus::{ModbusPdu, ModbusSession, UnitId};
use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
use stonecrab::transport::GuardConfig;

// The V1.1b section 6.3 exchange, against the repository's in-memory test transport.
let clock = FakeClock::default();
let factory = FakeTransportFactory::new(clock.clone());
factory.push_script(Script::strict(vec![
    Step::Expect(vec![0, 1, 0, 0, 0, 6, 0x11, 3, 0x00, 0x6B, 0x00, 0x03]),
    // A reply for the wrong unit comes first and is discarded.
    Step::Reply(vec![0, 1, 0, 0, 0, 9, 0x12, 3, 6, 0, 1, 0, 2, 0, 3]),
    Step::Reply(vec![0, 1, 0, 0, 0, 9, 0x11, 3, 6, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]),
]));
let limits = ResourceLimits::builder().build()?;
let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
let session = ModbusSession::new(config, Arc::new(factory), Arc::new(clock.clone()));
block_on_with(&clock, session.connect())?;

let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
let reply = block_on_with(&clock, session.request(&request, UnitId::from(17), deadline, None))?;
assert!(matches!(reply, ModbusPdu::ReadRegistersResponse { .. }));
assert_eq!(session.stats().mismatched_replies, 1);
# Ok::<(), stonecrab::PlcError>(())
# }
# #[cfg(not(feature = "testkit"))]
# fn main() {}
```

## Connection
`ModbusConnection::open(config, factory, clock, defaults)` builds the connection from a
`ConnectionConfig` for the scheme `modbus-tcp` (any other scheme is `InvalidConfig` before
a transport is opened), opens it and returns it `Connected`. It implements
`stonecrab::Connection`; the capabilities are read, write, ping and polling subscriptions,
no browse, no discovery, 253-byte PDUs and one operation at a time.

`subscribe` (S55, `MB-TCP-SUBSCRIBE`, ADR-0020 addendum) polls. It refuses
`SubscriptionKind::Event` with `Unsupported(Subscribe)` and then checks the request as a
read is checked, before any I/O. The `Subscription` reports `SubscriptionMode::Polling`
and its interval (the cyclic interval, or 1000 ms for change-of-state, upstream's default).
Each tick is one `read` of the subscription's tags, with the read's batching, deadline,
retry and span; the first tick is due at once, the next ones on a grid of the interval,
and the polling runs while `Subscription::recv` waits. Cyclic delivers every item;
change-of-state a tag's first outcome and then each change. A polling read takes the one
in-flight slot: a user read that starts meanwhile gets `Backpressure`, and a tick that finds
the slot taken is skipped, as are ticks that come due while a read runs or while nobody
waits, and a tick that meets the link being re-established. Every skipped tick counts in
`LossCounters::skipped`. Any other failure of a tick's read reaches the consumer as an item
error for every tag. `close`, or a terminal failure, ends the subscription after its queued
events.

`read` checks the request before any I/O, in this order: the connection is not `Closing`,
`Closed` or `Failed` (`Disconnected`), the token is not cancelled (`Cancelled`), every
tag's address is a `ModbusAddress` (`InvalidAddress`, naming the tag and never the
address), no tag has an element count above one (`InvalidConfig`: several elements are a
range selection in the address) and the request fits `max_items_per_request`
(`ResourceLimit`). Then one read runs
at a time (a concurrent read or ping is `Backpressure`) and an expired request deadline is
`Timeout`. The tags are planned into chunks by `Batcher::plan` and the chunks run one
after another:

- each chunk is one read request (function code 01 to 04, or 20 for extended registers) to
  the unit of its addresses, or the default unit, under the deadline `min(request deadline,
  now + timeouts.request)` taken when the attempt starts; an extended-register chunk that
  runs past record 9999 of a file is one function 20 request with a sub-request per file,
  the reply is proved to answer each of them with the registers asked for, and the
  registers of all of them are joined in order (`MB-ADDR-EXTENDED`);
- a device exception fails every member of that chunk with `exception_item_error` and the
  next chunk still runs; a reply that does not decode fails the member alone;
- a chunk whose exchange fails is retried under the `RetryPolicy` of the configuration
  (one attempt by default), only for kinds in its retry set and only while the request
  deadline has not passed; when the policy gives up, the whole read fails with that error,
  whose `Diagnostics` name the operation, the state, the latency and the retries performed;
- the response has one item per tag in request order, `split_into` is the number of chunks,
  each item's `sampled_at` is the instant its chunk was answered and `source_time` is
  `None`.

A tag's own byte and word order beat its address's, which beat `ConnectionDefaults`
(unit 1 and `BIG_ENDIAN` unless set). A lost peer shows as `Reconnecting` in
`watch_state`; the next read waits out the backoff and reopens, and the request that was in
flight when the peer closed fails with `Disconnected` and is never replayed. `close` ends
in `Closed`, also during the backoff. A debug span `read` carries `scheme`, `endpoint`,
`op`, `items` and `chunks`.

`write` (`MB-TCP-WRITE`) sends one request per item, one after another, as the upstream
driver does, through the same session, single in-flight slot and deadline rule as a read.
A connection whose access policy is read-only, which is the default, refuses the call
first, with `Unsupported(Operation::Write)`, before the connection state, the request or
the wire is looked at (`MB-WRITE-ACCESS`); `access=read-write` in the connection string
grants it. After that the call fails, with nothing sent, for a closed connection, a
cancelled token, a tag that is not a Modbus address, an element count above one, a
deadline that had passed and a read, write or ping in flight. Otherwise every item gets an
outcome:

- the value is encoded for the data type of its address in the byte and word order of the
  tag, else of the address block, else of `ConnectionDefaults`; a coil takes a `BOOL`; the
  request is function 15 for a coil (even one), 06 for a holding-register payload of
  exactly 2 bytes and 16 for any other, and function 21 for an extended register, cut at
  the end of each file into one sub-request per file of the same request;
- a range selection takes a `PlcValue::List` of exactly its element count, each element
  encoded as it would be alone, in one request (function 15 for coils, 16 for holding
  registers, 06 for a one-element list of a one-register type, 21 for extended registers,
  cut at the end of each file); a scalar written to a range, or a list to one element, is
  refused by the request builder with `InvalidConfig`;
- a discrete input or an input register, a value that does not fit its address (a list
  of another length or with an element of another representation included, with
  `InvalidConfig`), and a list wider than one write carries (124 registers, 1969 coils, or
  on the extended registers more than 118 registers across the end of a file, with
  `ResourceLimit`) are `Rejected` with no request, and the other items go on;
- an echo that repeats the request is `Confirmed` (for function 21, every sub-request with
  its registers); a device exception is `Rejected`, with the exception as the item error
  and its code as the protocol status;
- a request that went out and got no usable answer (the deadline passed, the link was
  lost, the token fired, or the reply did not decode or did not echo the request) is
  `WriteOutcome::Unknown` and is never retried or replayed, whatever the `RetryPolicy`
  says. The items after it are not sent: each is `Rejected` with `ErrorKind::Cancelled`
  and can be submitted again once the caller has read the value back
  (`MB-WRITE-UNKNOWN`, `MB-WRITE-HALT`). The reply must echo the function code of the
  request, so an FC16 response to an FC06 request, as the upstream all-datatypes files
  pair them, is not a confirmation (`MB-WRITE-ECHO`);
- a request that never went out (the link cannot be reopened, the deadline had passed, the
  token had fired) is that item's `Rejected`, with the error that stopped it.

Dropping the future of a write that went out loses the outcome: cancel through the
request's token to get `Unknown(CancelledAfterTransmit)`. A debug span `write` carries
`scheme`, `endpoint`, `op` and `items`; the session's `request` span inside it says
`op = "write"`.

```rust
# #[cfg(feature = "testkit")]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use stonecrab::{
    AccessPolicy, AddressOptions, Connection, ConnectionConfig, Endpoint, PlcValue, Tag, TagId,
    WriteRequest,
};
use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};

// V1.1b section 6.6: write 0x0003 to register 2 (wire address 1); the device echoes it.
let clock = FakeClock::default();
let factory = FakeTransportFactory::new(clock.clone());
factory.push_script(Script::strict(vec![
    Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 6, 0x00, 0x01, 0x00, 0x03]),
    Step::Reply(vec![0, 1, 0, 0, 0, 6, 1, 6, 0x00, 0x01, 0x00, 0x03]),
]));
let config = ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
    .access(AccessPolicy::ReadWrite)
    .build()?;
let connection = block_on_with(
    &clock,
    ModbusConnection::open(
        config,
        Arc::new(factory),
        Arc::new(clock.clone()),
        ConnectionDefaults::default(),
    ),
)?;

let address = ModbusAddress::parse("holding-register:2:UINT", &AddressOptions::default())?;
let tag = Tag::new(TagId::new("setpoint")?, Arc::new(address));
let request = WriteRequest::builder(connection.limits())
    .item(tag, PlcValue::U16(3))
    .build()?;
let response = block_on_with(&clock, connection.write(request))?;
assert_eq!(response.confirmed_count(), 1);
# Ok::<(), stonecrab::PlcError>(())
# }
# #[cfg(not(feature = "testkit"))]
# fn main() {}
```

`ping` (`MB-TCP-PING`) sends one read of the `ping-address` (default `4x00001:BOOL`) in the
unit the address names, else the default unit, through the same session, admission rules
and single in-flight slot as a read: a `Closing`, `Closed` or `Failed` connection is
`Disconnected` with nothing sent, a read or ping in flight is `Backpressure`, and a peer
that does not answer within `timeouts.request` is `Timeout`. It never retries, and
dropping the future frees the slot and discards a late reply. Any reply that answers the
request is success, an exception reply included (`MB-QUIRK-D`: upstream's `onPing` does the
same, and a reply of any kind shows that the peer answers); a transport failure keeps its
kind. A debug span `ping` carries `scheme`, `endpoint` and `op`.

```rust
# #[cfg(feature = "testkit")]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use stonecrab::{
    AddressOptions, Connection, ConnectionConfig, Endpoint, PlcValue, ReadRequest, Tag, TagId,
};
use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};

// Registers 108 and 109 of the V1.1b section 6.3 exchange, on the in-memory test transport.
let clock = FakeClock::default();
let factory = FakeTransportFactory::new(clock.clone());
factory.push_script(Script::strict(vec![
    Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0x00, 0x6B, 0x00, 0x02]),
    Step::Reply(vec![0, 1, 0, 0, 0, 7, 1, 3, 4, 0x02, 0x2B, 0x00, 0x00]),
]));
let config = ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
    .build()?;
let connection = block_on_with(
    &clock,
    ModbusConnection::open(config, Arc::new(factory), Arc::new(clock.clone()), ConnectionDefaults::default()),
)?;

let options = AddressOptions::default();
let mut request = ReadRequest::builder(connection.limits());
for (id, address) in [("a", "holding-register:108"), ("b", "holding-register:109")] {
    let address = ModbusAddress::parse(address, &options)?;
    request = request.tag(Tag::new(TagId::new(id)?, Arc::new(address)));
}
let response = block_on_with(&clock, connection.read(request.build()?))?;
assert_eq!(response.split_into().get(), 1);
assert_eq!(response.items()[0].outcome().as_ref().ok(), Some(&PlcValue::I16(555)));
assert_eq!(response.items()[1].outcome().as_ref().ok(), Some(&PlcValue::I16(0)));
# Ok::<(), stonecrab::PlcError>(())
# }
# #[cfg(not(feature = "testkit"))]
# fn main() {}
```

## Decoding values
`TypedDecoder` turns the registers of a holding or input register read into a typed
`PlcValue`, and one bit of a coil or discrete-input read into a `Bool`. The layout is two
independent axes, the core `ByteOrder` and `WordOrder`; the four upstream byte-order
names are four pairs of them (ADR-0006) and lay the value `0x01020304` out as shown:

| Upstream mode | `ByteOrder` | `WordOrder` | Bytes on the wire |
| --- | --- | --- | --- |
| `BIG_ENDIAN` | `BigEndian` | `HighWordFirst` | `[1,2,3,4]` |
| `LITTLE_ENDIAN` | `LittleEndian` | `LowWordFirst` | `[4,3,2,1]` |
| `BIG_ENDIAN_BYTE_SWAP` | `LittleEndian` | `HighWordFirst` | `[2,1,4,3]` |
| `LITTLE_ENDIAN_BYTE_SWAP` | `BigEndian` | `LowWordFirst` | `[3,4,1,2]` |

The mapping is derived from the documented layouts. The first-party vectors and the
pymodbus register map agree with it, and since S04b so do the retained upstream PLC4X
exchanges: the read responses of `DriverTestsuite.xml` and `Modbus-all-datatypes.xml`
decode under `BIG_ENDIAN`, and those of `Modbus-all-datatypes-little-endian.xml` under
`LITTLE_ENDIAN`, to the value each XML case reports (the vectors are serialized from the
XML models by a test-only adapter, not by Stonecrab's codec). The rows
`MB-BYTEORDER-BE` and `MB-BYTEORDER-LE` of the parity record are therefore
`automated-verified`; no upstream file configures the two byte-swap modes, so their rows
are `implemented`.

A lone `BOOL`, `BYTE`, `SINT` or `USINT` occupies one register: the low byte (and, for a
`BOOL`, bit 0) under `BigEndian`, the high byte (bit 8) under `LittleEndian`. The other
bits are padding, ignored when decoding and written as zero. Integers are exact; `REAL`
and `LREAL` keep NaN payloads, infinities and the sign of zero bit for bit, and judging
them is the sensor layer's job. The wrong number of registers for the type is an
`ErrorKind::MalformedData` item error whose `ProtocolStatus` (protocol `modbus`, code 0,
name `decode`) keeps the received registers as bytes, high byte first, when they fit the
caller's budget. A type other than `BOOL` on a bit area is `ErrorKind::InvalidConfig`.

```rust
use std::num::NonZeroUsize;
use stonecrab::{ByteOrder, ErrorKind, PlcValue, WordOrder};
use stonecrab::modbus::{ModbusDataType, TypedDecoder};

let budget = NonZeroUsize::new(250).ok_or("budget")?;

// REAL 3.141593 as the pymodbus map stores it (holding registers 26 and 27)...
let big = TypedDecoder::decode_registers(
    &[0x4049, 0x0FDC],
    ModbusDataType::Real,
    ByteOrder::BigEndian,
    WordOrder::HighWordFirst,
    budget,
)?;
// ...and under LITTLE_ENDIAN (registers 300 and 301 of the map).
let little = TypedDecoder::decode_registers(
    &[0xDC0F, 0x4940],
    ModbusDataType::Real,
    ByteOrder::LittleEndian,
    WordOrder::LowWordFirst,
    budget,
)?;
assert_eq!(big, little);

// Two registers are not an INT; the bytes stay inspectable.
let error = TypedDecoder::decode_registers(
    &[0x0001, 0x0002],
    ModbusDataType::Int,
    ByteOrder::BigEndian,
    WordOrder::HighWordFirst,
    budget,
)
.unwrap_err();
assert_eq!(error.kind(), ErrorKind::MalformedData);
assert_eq!(error.status().and_then(|s| s.raw()), Some(&[0, 1, 0, 2][..]));

// Coil 3 of a bit response packed as 0b0000_0101.
let coil = TypedDecoder::decode_bits(&[0b0000_0101], 2, ModbusDataType::Bool, budget)?;
assert_eq!(coil, PlcValue::Bool(true));
# Ok::<(), Box<dyn std::error::Error>>(())
```

The typed vectors in `fixtures/modbus/tcp/typed_*` hold register bytes for every type in
every mode, written by byte permutation of the documented layouts and kept apart from the
decoder; they are repository fixtures, not part of the published package.

## Configuration
The six connection parameters and the `address-convention` key are described under
"Connection strings" above. The connection takes the request timeout from
`Timeouts.request` and the default unit, byte order and request-size limits from
`ConnectionDefaults`, which the driver fills from `ModbusOptions`.

## Building an address
The builder offers the same checks without text (the parser ends in it):

```rust
use stonecrab::{ByteOrder, ProtocolAddress, WordOrder};
use stonecrab::modbus::{ModbusArea, ModbusDataType, ModbusAddress, UnitId};

// Holding register 16 (one-based) is wire offset 15.
let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 15)
    .data_type(ModbusDataType::Real)
    .unit_id(UnitId::from(10))
    .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
    .build()?;
assert_eq!(
    address.canonical(),
    "holding-register:16:REAL{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}"
);

// A 16-bit integer cannot be read from a coil; the builder says so.
let refused = ModbusAddress::builder(ModbusArea::Coil, 0)
    .data_type(ModbusDataType::Int)
    .build();
assert!(refused.is_err());
# Ok::<(), stonecrab::PlcError>(())
```
