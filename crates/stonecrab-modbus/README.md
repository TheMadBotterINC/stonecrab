# Stonecrab Modbus
Native Modbus TCP driver for Stonecrab. The alpha scope is read-only: function codes 01
(read coils), 02 (read discrete inputs), 03 (read holding registers) and 04 (read input
registers) over TCP with MBAP framing. RTU, ASCII, TLS, UDP, writes, extended registers
and string types are out of scope for the alpha.

Nothing is implemented beyond the address types, their text grammar, read batching, the
PDU codec, MBAP framing and the typed decoder. This crate currently offers `UnitId`, `ModbusArea`,
`ModbusDataType`, `AddressConvention`, `ModbusAddress` (built with its builder or parsed
from text with `ModbusAddress::parse`) and `ProtocolAddress` for it, plus `Batcher`,
`BatchLimits`, `Chunk` and `Member`, the PDU types `FunctionCode`, `ExceptionCode` and
`ModbusPdu` with `encode_request`, `decode_request`, `decode_response` and
`encode_response`, the framing types `MbapHeader`, `TransactionId`, `ModbusAdu` and
`MbapFramer`, and `TypedDecoder`, which turns registers or coil bits into `PlcValue`s.
There is no session, connection or driver yet, and no I/O of any kind. The crate
depends on `stonecrab-core`, `stonecrab-codec`, `stonecrab-transport` (without its
`tcp` feature, for the `Framer` trait only) and `bytes`; default features are empty.

Upstream behavior is the Apache PLC4X Modbus TCP driver at the commit pinned in
`upstream/manifest.toml`. Every claim about it, and every divergence, is recorded with a
stable row id in the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md).

| Capability | Support level |
| --- | --- |
| Address types: `UnitId`, `ModbusArea`, `ModbusDataType`, `ModbusAddress` | build-tested |
| Address text grammar and parser (`ModbusAddress::parse`) | unit-tested; fuzzed |
| PDU codec for function codes 01 to 04 and the exception PDU | unit-tested; vector-tested; property-tested; fuzzed |
| MBAP framing (`MbapFramer`, `MbapHeader`, `ModbusAdu`, `TransactionId`) | unit-tested; vector-tested; fuzzed |
| Transaction correlation | planned |
| Typed register decoding and byte order (`TypedDecoder`) | unit-tested; vector-tested; property-tested; the little-endian modes are derived from the documented layouts, not yet confirmed by upstream vectors |
| Read batching (`Batcher`) | unit-tested; property-tested |
| Session, connection and `modbus-tcp` driver | planned |
| Interoperability against an independent server | planned |
| Hardware verification | planned |

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
use stonecrab_core::{AddressOptions, ProtocolAddress};
use stonecrab_modbus::ModbusAddress;

let options = AddressOptions::builder().convention("plc4x").build()?;
let address = ModbusAddress::parse("40016:REAL{unit-id: 10}", &options)?;
assert_eq!(address.offset(), 15);
assert_eq!(address.canonical(), "holding-register:16:REAL{unit-id: 10}");

// Without the convention the bare form is ambiguous and refused.
assert!(ModbusAddress::parse("40016", &AddressOptions::default()).is_err());
assert!(ModbusAddress::parse("4x00016", &AddressOptions::default()).is_ok());
# Ok::<(), stonecrab_core::PlcError>(())
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
use stonecrab_codec::{BoundedReader, BoundedWriter};
use stonecrab_modbus::{
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
# Ok::<(), stonecrab_codec::ParseError>(())
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
use stonecrab_modbus::{MbapFramer, TransactionId};
use stonecrab_transport::Framer;

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
# Ok::<(), stonecrab_transport::FrameError>(())
```

The golden ADU vectors in `fixtures/modbus/tcp/adu_*` wrap the V1.1b example PDUs in the
header layout of the Modbus Messaging on TCP/IP Implementation Guide V1.0b, and add
the adversarial cases (lengths 255, 1 and 0, protocol id 1, a truncated header, a body that
has not arrived, a 261 byte ADU and a 1 KiB stream of seven coalesced ADUs).
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
use stonecrab_core::{ByteOrder, ErrorKind, PlcValue, WordOrder};
use stonecrab_modbus::{ModbusDataType, TypedDecoder};

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
The six connection parameters (request timeout, default unit identifier, ping address,
default payload byte order, maximum coils and registers per request) land with the driver
in slice S26.

## Building an address
The builder offers the same checks without text (the parser ends in it):

```rust
use stonecrab_core::{ByteOrder, ProtocolAddress, WordOrder};
use stonecrab_modbus::{ModbusArea, ModbusDataType, ModbusAddress, UnitId};

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
# Ok::<(), stonecrab_core::PlcError>(())
```
