# Stonecrab Modbus
Native Modbus TCP driver for Stonecrab (alpha). The alpha scope is read-only: function
codes 01 (read coils), 02 (read discrete inputs), 03 (read holding registers) and 04
(read input registers) over TCP with MBAP framing. RTU, ASCII, TLS, UDP, writes, extended
registers, string types, subscriptions, browse, discovery and ping are out of scope for
the alpha and are listed under "Not in the alpha" below. The driver has been checked
against an independent server, pymodbus 3.8.6, and no physical device.

This crate offers `ModbusTcpDriver`, the `Driver` of the `modbus-tcp` scheme, with
`ModbusOptions`, the parsed connection parameters; `ModbusConnection` with
`ConnectionDefaults`, the `Connection` that reads tags in batches over a session;
`ModbusSession` with `SessionStats` and `exception_item_error`, which send one read
request over a connection and return the reply that answers it; and the layers
underneath: `UnitId`, `ModbusArea`, `ModbusDataType`, `AddressConvention` and
`ModbusAddress` (built with its builder or parsed from text with `ModbusAddress::parse`),
`Batcher`, `BatchLimits`, `Chunk` and `Member`, the PDU types `FunctionCode`,
`ExceptionCode` and `ModbusPdu` with `encode_request`, `decode_request`, `decode_response`
and `encode_response`, the framing types `MbapHeader`, `TransactionId`, `ModbusAdu` and
`MbapFramer`, and `TypedDecoder`, which turns registers or coil bits into `PlcValue`s.
`SystemClock`, the Tokio-backed clock the driver uses by default, is re-exported from the
transport crate. Through the facade the driver is `stonecrab::modbus` behind the `modbus`
feature.

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
- `capabilities` is the value every connection reports: read only, no subscriptions,
  browse or discovery, one operation at a time, PDUs up to 253 bytes.

### Connection strings
`modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]`, followed by an optional
`?key=value&key=value`. The host is a name, an IPv4 address or an IPv6 address in brackets
(`[::1]:502`); the port is 1 to 65535 and defaults to 502; `tcp` is the only transport code
(`udp`, `tls` and the rest are `InvalidConfig`). The text may not contain whitespace,
control characters or `%`, nor a `user:password@` part, and is at most 4096 bytes.

The six upstream parameters and the shared `address-convention` key are the only keys. A
key may appear once, a value may not be empty, and every other key is `InvalidConfig`
(upstream warns and ignores it; `MB-CFG-UNKNOWN`). That includes the misspelt
`unit-identifier` that the upstream manual test passes. An error about a parameter value
or an unknown key names the key, and no error repeats the connection string.

| Key | Default | Accepted | Row |
| --- | --- | --- | --- |
| `request-timeout-ms` | 5000 | whole number of milliseconds above 0; also the config's `Timeouts.request` | `MB-CFG-REQUEST-TIMEOUT` |
| `default-unit-identifier` | 1 | 0 to 255 (upstream has no bound; divergence) | `MB-CFG-UNIT-ID` |
| `ping-address` | `4x00001:BOOL` | an address of the grammar below, read under the `plc4x` convention; recorded, unused at alpha | `MB-CFG-PING-ADDRESS` |
| `default-payload-byte-order` | `BIG_ENDIAN` | `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP` | `MB-CFG-BYTE-ORDER` |
| `max-coils-per-request` | 2000 | 1 to 2000 | `MB-CFG-MAX-COILS` |
| `max-registers-per-request` | 125 | 1 to 125 | `MB-CFG-MAX-REGISTERS` |
| `address-convention` | none | `plc4x` only | `MB-ADDR-BARE-40001` |

Numbers are decimal digits only: no sign, no space, no `0x`. `address-convention=plc4x`
is the explicit-convention rule of the address grammar below: the bare short forms such
as `40001` parse only under it, and the driver hands it to every `parse_address` call
through `ConnectionConfig::address_options()`. Without it only the long forms
(`holding-register:1`) and the prefixed short forms (`4x00001`) parse.

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
passes in, so the crate's tests run on the in-memory transport of the repository's
unpublished testkit and, for the driver, over loopback sockets to a scripted peer. The
common driver contract suite passes against the driver over both: over the in-memory
transport with the two write cases skipped by name, and over loopback sockets with those two
and the three timing cases skipped by name (the loopback fixture cannot swallow a reply on a
deterministic clock). The crate depends on `stonecrab-core`, `stonecrab-codec`,
`stonecrab-transport` (with its `tcp` feature, for `TcpFactory`, and the `Framer` trait,
`ConnectionGuard` and `TransportFactory`), `bytes` and `tracing`; default features are
empty.

Upstream behavior is the Apache PLC4X Modbus TCP driver at the commit pinned in
`upstream/manifest.toml`. Every claim about it, and every divergence, is recorded with a
stable row id in the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md).

## Support levels
The labels are those of the project's hardware-evidence policy: `build-tested`,
`simulator-tested`, `interoperability-tested` and `hardware-verified`, plus `planned` and
`not supported`. `simulator-tested` covers the in-memory transport and the loopback
scripted peer. The `interop` CI job (slice S27) runs the driver against an independent
server, pymodbus 3.8.6, on every push (see "Interoperability" below); the rows it exercises
read `interoperability-tested`. No physical device has been tried.

| Capability | Support level |
| --- | --- |
| Read: coils, discrete inputs, holding registers and input registers (function codes 01 to 04) as the 15 numeric and bit types | interoperability-tested |
| Write | not supported |
| Subscribe, browse and discover | not supported |
| Connection string and parameters (`parse_config`) | simulator-tested |
| Connect, close and connection state | interoperability-tested |
| Request-driven reconnect, read retry, cancellation, timeout and late-reply discard | simulator-tested |
| Batched reads with split disclosure | interoperability-tested |
| Interoperability against an independent server | interoperability-tested |
| Hardware verification | planned |

The layers underneath, with their evidence:

| Layer | Evidence |
| --- | --- |
| Address types: `UnitId`, `ModbusArea`, `ModbusDataType`, `ModbusAddress` | unit-tested; property-tested; fuzzed |
| Address text grammar and parser (`ModbusAddress::parse`) | unit-tested; fuzzed |
| Connection-string parser (`ModbusTcpDriver::parse_config`) | unit-tested; property-tested; fuzzed |
| PDU codec for function codes 01 to 04 and the exception PDU | unit-tested; vector-tested; property-tested; fuzzed |
| MBAP framing (`MbapFramer`, `MbapHeader`, `ModbusAdu`, `TransactionId`) | unit-tested; vector-tested; fuzzed |
| Session: transaction ids, reply correlation by transaction id, unit id and function code, exception mapping (`ModbusSession`, `exception_item_error`) | unit-tested on the in-memory transport |
| Typed register decoding and byte order (`TypedDecoder`) | unit-tested; vector-tested; property-tested; the little-endian modes are derived from the documented layouts, not yet confirmed by upstream vectors |
| Read batching (`Batcher`) | unit-tested; property-tested |
| Connection and driver (`ModbusConnection`, `ModbusTcpDriver`) | unit-tested on the in-memory transport; contract-suite-tested on the in-memory transport and over loopback sockets; one read run over loopback; ten reads and the contract suite against pymodbus 3.8.6 |

## Not in the alpha
Each item is a deferred row of the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md)
with its owner. Where the parser or driver can see the request it rejects it with a typed
error rather than ignoring it; operations that have no API at all do not exist:

- writes (`MB-TCP-WRITE`): `write` is `Unsupported(Operation::Write)`, sends nothing, and
  `Capabilities.write` is false; write PDUs, the upstream FC06-versus-FC16 choice and
  `WriteOutcome::Unknown` on the wire come after the alpha;
- subscriptions, browse, discovery and ping (`MB-TCP-SUBSCRIBE`, `MB-TCP-BROWSE`,
  `MB-TCP-DISCOVER`, `MB-TCP-PING`): these operations do not exist in the API; the
  `ping-address` key is validated and unused;
- extended registers (FC20 and FC21, `extended-register`, `6x`), `STRING`, `WSTRING`,
  `CHAR`, `WCHAR`, the `TIME` and `DATE` family, array selections and the legacy array
  form (`MB-ADDR-*`, `MB-TYPE-*`);
- UDP, TLS and TLS-PSK transports (`MB-TCP-TRANSPORTS-*`); RTU and ASCII framing are
  milestone M2 (no row yet);
- the upstream gap-filling read optimizer: Stonecrab merges adjacent or overlapping tags
  only (`MB-READ-GAP`), and a background reconnect task: reconnect is request-driven;
- the upstream test vectors, which wait for maintainer approval to redistribute them
  (prerequisite P4): the repository's vectors are first-party, and the byte-order rows
  stay below `automated-verified` for that reason.

## Interoperability
The `interop` job of the repository's CI builds `interop/pymodbus/` (a first-party Modbus
TCP server on pymodbus 3.8.6, `python:3.12-slim` pinned by digest, serving the register
map of `fixtures/modbus/interop/register-map.toml`), starts it on `127.0.0.1:5020` and
runs ten tests of `tests/interop_pymodbus.rs` plus the `modbus_tcp_read` example against
it. The tests are `#[ignore]`d and run only when `STONECRAB_INTEROP_MODBUS` names the
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
- The bare `40001` spelling is refused without `address-convention=plc4x` and reads with it.
- The common driver contract suite passes through `ModbusTcpDriver::new()`: 14 cases pass
  and 9 are skipped by name (the timing, disconnect and device-exception cases that need
  a fault hook a stock pymodbus does not have, and the two write cases).

Not verified there: timing over a real clock, dropped links and device exceptions on
demand, and unit-id correlation (pymodbus answers every unit id). The decisions behind
the run are in ADR-0015, decision 16.

## Address grammar
`ModbusAddress::parse(text, &AddressOptions)` reads the grammar of the upstream PLC4X Modbus
driver, with the divergences listed below. Everything is case-sensitive and decimal, and
the number is the one-based logical address: the wire offset is the number minus one, so
`holding-register:16` reads wire address 15 and the first register is `holding-register:1`.

| Form | Examples | Accepted |
| --- | --- | --- |
| long: `AREA:N` or `AREA:N:TYPE`, N has 1 to 9 digits | `holding-register:16:INT`, `coil:1` | always |
| prefixed short: `0x`, `1x`, `3x` or `4x` plus 4 or 5 digits | `4x00016:INT`, `0x0010` | always |
| bare short: prefix digit `0`, `1`, `3` or `4` plus 4 or 5 digits | `40001`, `400001`, `10001` | only with `address-convention=plc4x` |

`AREA` is `coil`, `discrete-input`, `holding-register` or `input-register`, and the prefixes
are `0x` coil, `1x` discrete input, `3x` input register and `4x` holding register; there is
no `2x`. `0x0010` is coil 10, a decimal number that happens to start with the coil prefix,
not hexadecimal. When no type is written it is `BOOL` for the bit areas (coil, discrete
input) and `INT` for the register areas. `TYPE` is one of the 15 upper-case names `BOOL`,
`BYTE`, `WORD`, `DWORD`, `LWORD`, `SINT`, `USINT`, `INT`, `UINT`, `DINT`, `UDINT`, `LINT`,
`ULINT`, `REAL`, `LREAL`; the bit areas accept `BOOL` only, and a type whose registers pass
wire address 65535 is refused (`LREAL` at `holding-register:65534`). The last address is
`holding-register:65536`, wire address 65535.

The explicit-convention rule: a bare number such as `40001` could be holding register 1
under the PLC4X convention or a zero-based wire address, so the parser accepts the bare
forms only when the caller named the convention (`AddressOptions` with convention `plc4x`,
from the connection option `address-convention=plc4x` or a profile's `address_convention`).
Without it the error names the ambiguity and the two spellings that are always accepted:
`holding-register:1` and `4x00001`. Any other convention name is an error, even for the
unambiguous spellings. The text `canonical()` renders is always the long form, which parses
back to an equal address under every option.

An address may end with a config block, directly after the type with no space before the
brace: `{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`. The two keys are `unit-id`, a decimal
integer from 0 to 255 (the one-byte MBAP field; upstream has no range check, so this is a
documented divergence), and `byte-order`, a quoted name (single or double quotes) from
`BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP` and `LITTLE_ENDIAN_BYTE_SWAP`. An
unquoted name is rejected: the upstream value pattern only matches quoted text, and
Stonecrab turns what would be a silently dropped pair into an error. Either key, both in
any order, or an empty block `{}` are accepted; spaces around keys, colons, values and
commas are ignored; unknown or repeated keys are errors.

Rejected with `ErrorKind::InvalidAddress`, each message naming the reason:

- empty input, input over 256 bytes, control characters, and any text no form matches;
- a bare short form without the explicit convention, and an unknown convention name;
- logical address 0, above 65536, hexadecimal digits (`holding-register:0x0010`), more
  than 9 digits in the long form, other than 4 or 5 in a short form, `4x:INT` with no digits;
- lowercase or unknown type names (`holding-register:16:i16`);
- a type other than `BOOL` on a coil or discrete input (upstream fails these only at read
  time);
- a span past wire address 65535;
- the features deferred past the alpha: array selections `[n]` and `[a..b]`, `STRING`,
  `WSTRING`, `CHAR`, `WCHAR` and `STRING(n)`, the `TIME` and `DATE` family,
  `extended-register`, `6x` and the bare `6` prefix, and the legacy `AREA:N:INT[4]` form
  (the message gives the replacement `AREA:N[0..3]:INT`);
- a malformed config block: unterminated, unknown or repeated key, `unit-id` outside
  0..=255, unquoted or unknown `byte-order`.

Error messages echo at most 32 control-free characters of the input. The row ids behind
each rule are in the parity record, `MB-ADDR-*`, `MB-LIMIT-ADDR`, `MB-LIMIT-SPAN`,
`MB-LIMIT-DIGITS` and `MB-CFG-UNIT-ID`.

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
# Ok::<(), stonecrab::PlcError>(())
```

## PDU codec
A PDU is the function code and its data, without the MBAP header. `ModbusPdu` has the four
read requests (`ReadCoils`, `ReadDiscreteInputs`, `ReadHoldingRegisters`,
`ReadInputRegisters`, each `{ start, quantity }` with the zero-based wire address), the two
normal responses (`ReadBitsResponse` holding the packed bytes exactly as received, and
`ReadRegistersResponse` holding the registers) and `Exception { function, code }`. Write
PDUs are not transcribed: function codes 05, 06, 15, 16, 20 and 21 are
`FunctionCode::Unknown` and every decoder refuses them.

A request is the function code, the start address and the quantity (5 bytes), with a
quantity of 1 to 2000 bits or 1 to 125 registers and `start + quantity` at most 65536
(`decode_request`, `encode_request`). A response is the function code, a byte count of 1 to
250 (even for registers) and the data, or the function code plus 0x80 and one of the ten
exception codes (`decode_response`, `encode_response`).

Every length is compared with its bound before anything is allocated, a PDU is at most 253
bytes, and input must hold exactly one PDU. A response does not repeat the request's
quantity, so `decode_response` cannot know whether the byte count fits it:
`ModbusPdu::check_reply_to(&request)` proves that, and the matching function, once the
session knows the request. Exception codes outside the ten that V1.1b section 7 defines are
refused as `InvalidDiscriminator` rather than carried (`ExceptionCode::Unknown` exists so
the number can still be named). The rows are `MB-TCP-READ-*`, `MB-EXC-*` and
`MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`, `MB-LIMIT-PDU` in the parity record.

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

The golden vectors in `fixtures/modbus/tcp/` are written from the examples of the Modbus
Application Protocol Specification V1.1b or built to break the decoder; they are
repository fixtures, not part of the published crate.

## MBAP framing
On Modbus TCP every PDU travels behind a 7 byte MBAP header: the transaction id, the
protocol id (always 0), a length that counts the unit id plus the PDU, and the unit id, all
big-endian. `MbapFramer` implements the transport crate's `Framer` for `ModbusAdu`, an
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
`ConnectionGuard<ModbusAdu, MbapFramer>` (the lifecycle, request-driven reconnect and
the single in-flight slot of the transport crate) and the transaction counter.
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
`stonecrab::Connection`; the capabilities are read only, no subscription, no browse,
no discovery, 253-byte PDUs and one operation at a time.

`read` checks the request before any I/O, in this order: the connection is not `Closing`,
`Closed` or `Failed` (`Disconnected`), the token is not cancelled (`Cancelled`), every
tag's address is a `ModbusAddress` (`InvalidAddress`, naming the tag and never the
address), no tag has an element count above one (`InvalidConfig`: array selections are
deferred) and the request fits `max_items_per_request` (`ResourceLimit`). Then one read runs
at a time (a concurrent one is `Backpressure`) and an expired request deadline is
`Timeout`. The tags are planned into chunks by `Batcher::plan` and the chunks run one
after another:

- each chunk is one read request (function code 01 to 04) to the unit of its addresses, or
  the default unit, under the deadline `min(request deadline, now + timeouts.request)`
  taken when the attempt starts;
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
in `Closed`, also during the backoff. `write` is `Unsupported(Operation::Write)` and sends
nothing. A debug span `read` carries `scheme`, `endpoint`, `op`, `items` and `chunks`.

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
pymodbus register map agree with it, but the upstream little-endian vectors are not in the
repository until the maintainer approval P4 is granted, so the byte-order rows of the
parity record are `implemented`, not `automated-verified`.

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
decoder; they are repository fixtures, not part of the published crate.

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
