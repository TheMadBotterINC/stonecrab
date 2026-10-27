# Stonecrab Modbus
Native Modbus TCP driver for Stonecrab. The alpha scope is read-only: function codes 01
(read coils), 02 (read discrete inputs), 03 (read holding registers) and 04 (read input
registers) over TCP with MBAP framing. RTU, ASCII, TLS, UDP, writes, extended registers
and string types are out of scope for the alpha.

Nothing is implemented beyond the address types. This crate currently offers `UnitId`,
`ModbusArea`, `ModbusDataType`, `AddressConvention`, `ModbusAddress` and its builder, and
`ModbusAddress` implements the core `ProtocolAddress` trait. There is no address text
parser, PDU codec, framing, session, connection or driver yet, and no I/O of any kind. The
crate depends only on `stonecrab-core`; default features are empty.

Upstream behavior is the Apache PLC4X Modbus TCP driver at the commit pinned in
`upstream/manifest.toml`. Every claim about it, and every divergence, is recorded with a
stable row id in the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md).

| Capability | Support level |
| --- | --- |
| Address types: `UnitId`, `ModbusArea`, `ModbusDataType`, `ModbusAddress` | build-tested |
| Address text grammar and parser | planned |
| PDU codec for function codes 01 to 04 and the exception PDU | planned |
| MBAP framing and transaction correlation | planned |
| Typed register decoding and byte order | planned |
| Read batching | planned |
| Session, connection and `modbus-tcp` driver | planned |
| Interoperability against an independent server | planned |
| Hardware verification | planned |

## Address grammar
The text grammar (`holding-register:16:INT`, `4x00016:INT{unit-id: 10}` and the other
forms) lands in slice S20b. Until then an address is built from its parts with
`ModbusAddress::builder`.

## Configuration
The six connection parameters (request timeout, default unit identifier, ping address,
default payload byte order, maximum coils and registers per request) land with the driver
in slice S26.

## Building an address
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
