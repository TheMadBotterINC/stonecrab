# Stonecrab Codec
Bounded, panic-free byte-level primitives shared by the Stonecrab protocol crates.
Every length that comes off the wire is checked against a caller-supplied maximum and
against the bytes actually present before anything is allocated or copied, and no
function in this crate panics on any input. The crate has no network, executor or
filesystem dependency; it depends only on `stonecrab-core` for `ByteOrder` and
`PlcError`. Default features are empty.

No upstream parity claim is made: these are Stonecrab's own helpers. The Modbus layouts
that consume them are tracked in the
[Modbus parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/modbus.md).

| Capability | Support level |
| --- | --- |
| `BoundedReader`: bounds-checked integer, float, byte-run and bit reads | build-tested |
| `BoundedWriter`: capacity-bounded writes of the same types | build-tested |
| `ParseError` and its conversion to `PlcError` | build-tested |
| Coil bit packing helpers (least significant bit first) | build-tested |
| Checksums (`InvalidChecksum`) and nesting limits (`RecursionLimit`) | planned |
| Hardware verification | planned |

The primitives are unit-tested, property-tested and fuzzed, and the Modbus PDU codec built
on them runs against pymodbus 3.8.6 in the repository's `interop` job; that is evidence
for the Modbus read path, and the primitives carry no support level above `build-tested`
of their own. Checksums arrive with Modbus RTU (milestone M2). Byte order is always passed
explicitly as a `ByteOrder`; nothing is inferred from the data.

API:

- `BoundedReader`: `new`, `remaining`, `read_u8`, `read_i8`, `read_u16`, `read_i16`,
  `read_u32`, `read_i32`, `read_u64`, `read_i64`, `read_f32`, `read_f64`, `read_bytes`,
  `read_bits_lsb_first` and `expect_eof`.
- `BoundedWriter`: `new`, `capacity`, `remaining_capacity`, the matching `write_*`
  methods, `write_bytes`, `write_bits_lsb_first`, `as_slice` and `into_bytes`.
- `ParseError`: `Truncated`, `OversizedLength`, `InvalidDiscriminator`, `TrailingBytes`
  and `Overflow`, with `From<ParseError> for PlcError` (`MalformedData`, except
  `OversizedLength` which maps to `ResourceLimit`).
- `coil_byte_count`, `pack_bits_lsb_first` and `unpack_bits_lsb_first`.

```rust
use stonecrab::codec::{BoundedReader, BoundedWriter, ParseError};
use stonecrab::ByteOrder;

// Write one register and three packed coils, then read them back.
let mut writer = BoundedWriter::new(16);
writer.write_u16(0x1234, ByteOrder::BigEndian)?;
writer.write_bits_lsb_first(&[true, false, true])?;
let wire = writer.into_bytes();
assert_eq!(wire, [0x12, 0x34, 0b0000_0101]);

let mut reader = BoundedReader::new(&wire);
assert_eq!(reader.read_u16(ByteOrder::BigEndian)?, 0x1234);
assert_eq!(reader.read_bits_lsb_first(3, 2000)?, [true, false, true]);
reader.expect_eof()?;

// A hostile length is refused before the data is touched.
let mut hostile = BoundedReader::new(&[0xFF; 4]);
let refused = hostile.read_bytes(1_073_741_824, 253);
assert!(matches!(refused, Err(ParseError::OversizedLength { .. })));
# Ok::<(), ParseError>(())
```
