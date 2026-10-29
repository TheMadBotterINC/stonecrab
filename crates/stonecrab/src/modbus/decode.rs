//! Typed decoding of register and coil data into [`PlcValue`]s, and the inverse for tests
//! and the future write path.
//!
//! The layout rules are stated once, on [`TypedDecoder`]; every function here applies
//! them through the bounds-checked primitives of `stonecrab::codec` (ADR-0013).

use std::num::NonZeroUsize;

use crate::codec::{BoundedReader, BoundedWriter, unpack_bits_lsb_first};
use crate::{ByteOrder, ErrorKind, ItemError, PlcError, PlcValue, ProtocolStatus, WordOrder};

use crate::modbus::ModbusDataType;

/// The widest value is an `LREAL`, `LWORD`, `LINT` or `ULINT`: four registers, 8 bytes.
const MAX_VALUE_BYTES: usize = 8;

/// The protocol name and the status name every decode failure carries.
const STATUS_PROTOCOL: &str = "modbus";
const STATUS_NAME: &str = "decode";

/// Turns the registers or coil bits of a Modbus read into a typed [`PlcValue`], and a
/// value back into registers.
///
/// This is a unit struct; every operation is an associated function and nothing is
/// stored. The layout is a function of the type, the [`ByteOrder`] and the [`WordOrder`]:
///
/// - A register is 16 bits and arrives as two bytes, high byte first. Under
///   [`ByteOrder::BigEndian`] the two bytes of a register are the two bytes of the value
///   in that order; under [`ByteOrder::LittleEndian`] they are swapped within the
///   register.
/// - A value of two or four registers puts its most significant register first under
///   [`WordOrder::HighWordFirst`] and last under [`WordOrder::LowWordFirst`].
/// - A lone `BOOL`, `BYTE`, `SINT` or `USINT` occupies one register and is the low-order
///   part of the 16-bit value that the two rules above lay out: the low byte of the
///   register under `BigEndian`, the high byte under `LittleEndian`; a `BOOL` is bit 0 of
///   the register under `BigEndian` and bit 8 under `LittleEndian`. The other bits of the
///   register are padding, written as zero and ignored when decoding.
/// - Integers are exact. `REAL` and `LREAL` go through `from_bits` and `to_bits`, so NaN
///   payloads, infinities and the sign of zero are preserved bit for bit.
///
/// The four upstream modes (docs/parity/modbus.md, "Byte order") are these pairs, and lay
/// the value `0x01020304` out as the bytes shown:
///
/// | Upstream mode | `ByteOrder` | `WordOrder` | Registers | Bytes on the wire |
/// | --- | --- | --- | --- | --- |
/// | `BIG_ENDIAN` | `BigEndian` | `HighWordFirst` | `[0x0102, 0x0304]` | `[1,2,3,4]` |
/// | `LITTLE_ENDIAN` | `LittleEndian` | `LowWordFirst` | `[0x0403, 0x0201]` | `[4,3,2,1]` |
/// | `BIG_ENDIAN_BYTE_SWAP` | `LittleEndian` | `HighWordFirst` | `[0x0201, 0x0403]` | `[2,1,4,3]` |
/// | `LITTLE_ENDIAN_BYTE_SWAP` | `BigEndian` | `LowWordFirst` | `[0x0304, 0x0102]` | `[3,4,1,2]` |
///
/// # Examples
/// ```
/// use std::num::NonZeroUsize;
/// use stonecrab::{ByteOrder, PlcValue, WordOrder};
/// use stonecrab::modbus::{ModbusDataType, TypedDecoder};
///
/// let budget = NonZeroUsize::new(16).ok_or("budget")?;
/// let value = PlcValue::U32(0x0102_0304);
/// let modes = [
///     (ByteOrder::BigEndian, WordOrder::HighWordFirst, [1, 2, 3, 4]),
///     (ByteOrder::LittleEndian, WordOrder::LowWordFirst, [4, 3, 2, 1]),
///     (ByteOrder::LittleEndian, WordOrder::HighWordFirst, [2, 1, 4, 3]),
///     (ByteOrder::BigEndian, WordOrder::LowWordFirst, [3, 4, 1, 2]),
/// ];
/// for (byte_order, word_order, wire) in modes {
///     let registers =
///         TypedDecoder::encode_registers(&value, ModbusDataType::DWord, byte_order, word_order)?;
///     let bytes: Vec<u8> = registers.iter().flat_map(|r| r.to_be_bytes()).collect();
///     assert_eq!(bytes, wire);
///     let back = TypedDecoder::decode_registers(
///         &registers,
///         ModbusDataType::DWord,
///         byte_order,
///         word_order,
///         budget,
///     )?;
///     assert_eq!(back, value);
/// }
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-BE
/// The mapping of the four upstream modes to a byte order and a word order is rows
/// `MB-BYTEORDER-BE` (`BIG_ENDIAN`), `MB-BYTEORDER-LE` (`LITTLE_ENDIAN`),
/// `MB-BYTEORDER-BE-SWAP` (`BIG_ENDIAN_BYTE_SWAP`) and `MB-BYTEORDER-LE-SWAP`
/// (`LITTLE_ENDIAN_BYTE_SWAP`), derived from the documented 4-byte layouts and not yet
/// confirmed by the upstream little-endian vectors (docs/parity/modbus.md#MB-BYTEORDER-LE).
/// The single-register padding of the 8-bit types and `BOOL` follows the upstream
/// `ModbusRegisterCodec` as recorded in docs/parity/modbus.md#MB-TYPE-BOOL,
/// docs/parity/modbus.md#MB-TYPE-BYTE, docs/parity/modbus.md#MB-TYPE-SINT and
/// docs/parity/modbus.md#MB-TYPE-USINT; the record states it for `BIG_ENDIAN` and
/// `LITTLE_ENDIAN` only, so the two byte-swap modes follow from the pair mapping
/// (INFERENCE). The value mapping of each type is the "Registers and `DataItem` encoding"
/// column of the data type table (docs/parity/modbus.md#MB-TYPE-WORD,
/// docs/parity/modbus.md#MB-TYPE-DWORD, docs/parity/modbus.md#MB-TYPE-LWORD,
/// docs/parity/modbus.md#MB-TYPE-INT, docs/parity/modbus.md#MB-TYPE-DINT,
/// docs/parity/modbus.md#MB-TYPE-LINT, docs/parity/modbus.md#MB-TYPE-UINT,
/// docs/parity/modbus.md#MB-TYPE-UDINT, docs/parity/modbus.md#MB-TYPE-ULINT,
/// docs/parity/modbus.md#MB-TYPE-REAL and docs/parity/modbus.md#MB-TYPE-LREAL).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TypedDecoder;

impl TypedDecoder {
    /// Decode the registers of one element into a typed value.
    ///
    /// `registers` must hold exactly [`ModbusDataType::register_size`] registers, as read
    /// from a holding or input register area. The layout is described on
    /// [`TypedDecoder`]; the padding bits of a lone 8-bit value or `BOOL` are ignored.
    ///
    /// The result is `U8` for `BYTE` and `USINT`, `U16` for `WORD` and `UINT`, `U32` for
    /// `DWORD` and `UDINT`, `U64` for `LWORD` and `ULINT`, `I8`, `I16`, `I32` and `I64`
    /// for `SINT`, `INT`, `DINT` and `LINT`, `F32` for `REAL`, `F64` for `LREAL` and
    /// `Bool` for `BOOL`, the same as [`ModbusDataType::value_type`].
    ///
    /// # Errors
    /// Returns an [`ItemError`] of kind [`ErrorKind::MalformedData`] when the number of
    /// registers is not the size of `data_type`. Its [`ProtocolStatus`] is protocol
    /// `"modbus"`, code 0, name `"decode"`, and keeps the received registers as bytes
    /// (each register high byte first, as on the wire) through
    /// [`ProtocolStatus::with_raw`]. The bytes are attached only when they fit
    /// `raw_budget`; an over-budget input yields the same error with no raw bytes
    /// (`ProtocolStatus::raw` is `None`), never a truncated copy.
    ///
    /// # Examples
    /// ```
    /// use std::num::NonZeroUsize;
    /// use stonecrab::{ByteOrder, ErrorKind, PlcValue, WordOrder};
    /// use stonecrab::modbus::{ModbusDataType, TypedDecoder};
    ///
    /// let budget = NonZeroUsize::new(16).ok_or("budget")?;
    /// // Interop holding registers 26 and 27: REAL 3.141593, BIG_ENDIAN.
    /// let real = TypedDecoder::decode_registers(
    ///     &[0x4049, 0x0FDC],
    ///     ModbusDataType::Real,
    ///     ByteOrder::BigEndian,
    ///     WordOrder::HighWordFirst,
    ///     budget,
    /// )?;
    /// assert_eq!(real, PlcValue::F32(3.141_593));
    ///
    /// // One register is not a REAL: the bytes stay inspectable on the error.
    /// let error = TypedDecoder::decode_registers(
    ///     &[0x4049],
    ///     ModbusDataType::Real,
    ///     ByteOrder::BigEndian,
    ///     WordOrder::HighWordFirst,
    ///     budget,
    /// )
    /// .unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::MalformedData);
    /// let status = error.status().ok_or("status")?;
    /// assert_eq!(status.name(), Some("decode"));
    /// assert_eq!(status.raw(), Some(&[0x40, 0x49][..]));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-BE
    /// Byte and word order: docs/parity/modbus.md#MB-BYTEORDER-BE,
    /// docs/parity/modbus.md#MB-BYTEORDER-LE, docs/parity/modbus.md#MB-BYTEORDER-BE-SWAP
    /// and docs/parity/modbus.md#MB-BYTEORDER-LE-SWAP. Types: docs/parity/modbus.md#MB-TYPE-BOOL
    /// to docs/parity/modbus.md#MB-TYPE-LREAL. Upstream decodes a whole read through
    /// its generated `DataItem`; Stonecrab rejects a wrong register count instead of
    /// reading what is there, and keeps the received bytes (decision D14, ADR-0006).
    pub fn decode_registers(
        registers: &[u16],
        data_type: ModbusDataType,
        byte_order: ByteOrder,
        word_order: WordOrder,
        raw_budget: NonZeroUsize,
    ) -> Result<PlcValue, ItemError> {
        if registers.len() != usize::from(data_type.register_size()) {
            return Err(wrong_register_count(registers, data_type, raw_budget));
        }
        let ordered = most_significant_first(registers, word_order);
        read_value(&ordered, byte_order, data_type).map_err(ItemError::new)
    }

    /// Decode one bit of a coil or discrete input read.
    ///
    /// `packed` is the data of a bit response exactly as received, 8 bits per byte least
    /// significant bit first, and `index` the zero-based position of the element in it.
    /// Bit areas hold `BOOL` only.
    ///
    /// # Errors
    /// Returns an [`ItemError`] of kind [`ErrorKind::InvalidConfig`] when `data_type` is
    /// not [`ModbusDataType::Bool`] (its status is `"modbus"`, 0, `"decode"` without raw
    /// bytes), and of kind [`ErrorKind::MalformedData`] when `index` lies beyond the
    /// `packed` bytes (its status keeps `packed` as raw bytes when they fit `raw_budget`,
    /// as for [`decode_registers`](Self::decode_registers)).
    ///
    /// # Examples
    /// ```
    /// use std::num::NonZeroUsize;
    /// use stonecrab::{ErrorKind, PlcValue};
    /// use stonecrab::modbus::{ModbusDataType, TypedDecoder};
    ///
    /// let budget = NonZeroUsize::new(16).ok_or("budget")?;
    /// // Interop coils 1 to 10: true, false, true, true, false, true, false, false, true, false.
    /// let packed = [0b0010_1101, 0b0000_0001];
    /// let coil_9 = TypedDecoder::decode_bits(&packed, 8, ModbusDataType::Bool, budget)?;
    /// assert_eq!(coil_9, PlcValue::Bool(true));
    /// let coil_2 = TypedDecoder::decode_bits(&packed, 1, ModbusDataType::Bool, budget)?;
    /// assert_eq!(coil_2, PlcValue::Bool(false));
    ///
    /// let refused = TypedDecoder::decode_bits(&packed, 0, ModbusDataType::Int, budget);
    /// assert_eq!(refused.unwrap_err().kind(), ErrorKind::InvalidConfig);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
    /// A bit area reads one bit per `BOOL` (docs/parity/modbus.md#MB-TYPE-BOOL). Upstream
    /// parses a typed coil and fails at read time; Stonecrab refuses non-`BOOL` types on
    /// bit areas at parse time and again here (docs/parity/modbus.md#MB-ADDR-BIT-TYPES).
    pub fn decode_bits(
        packed: &[u8],
        index: usize,
        data_type: ModbusDataType,
        raw_budget: NonZeroUsize,
    ) -> Result<PlcValue, ItemError> {
        if data_type != ModbusDataType::Bool {
            let error = PlcError::new(
                ErrorKind::InvalidConfig,
                format!(
                    "{} cannot be read from a coil or discrete input; bit areas hold BOOL only",
                    data_type.upstream_name()
                ),
            );
            return Err(ItemError::new(error).with_status(decode_status()));
        }
        let bit = index
            .checked_add(1)
            .and_then(|count| unpack_bits_lsb_first(packed, count).ok())
            .and_then(|bits| bits.get(index).copied());
        bit.map(PlcValue::Bool).ok_or_else(|| {
            let error = PlcError::new(
                ErrorKind::MalformedData,
                format!(
                    "bit {index} lies outside the {} byte(s) received",
                    packed.len()
                ),
            );
            let status = keep_raw(decode_status(), packed, raw_budget);
            ItemError::new(error).with_status(status)
        })
    }

    /// Encode a typed value as the registers of one element.
    ///
    /// The inverse of [`decode_registers`](Self::decode_registers) for tests and the
    /// future write path: the registers are laid out as described on [`TypedDecoder`],
    /// with zero padding bits for a lone 8-bit value or `BOOL`. The result holds exactly
    /// [`ModbusDataType::register_size`] registers.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when the variant of `value` is not the one
    /// `data_type` decodes to (`PlcValue::U32` for `DWORD`, for example); nothing is
    /// converted or widened.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ByteOrder, PlcValue, WordOrder};
    /// use stonecrab::modbus::{ModbusDataType, TypedDecoder};
    ///
    /// // Interop holding registers 4 and 5: DWORD 4242442424, BIG_ENDIAN.
    /// let registers = TypedDecoder::encode_registers(
    ///     &PlcValue::U32(4_242_442_424),
    ///     ModbusDataType::DWord,
    ///     ByteOrder::BigEndian,
    ///     WordOrder::HighWordFirst,
    /// )?;
    /// assert_eq!(registers, [0xFCDE, 0x88B8]);
    ///
    /// // A value of another type is refused, not converted.
    /// let refused = TypedDecoder::encode_registers(
    ///     &PlcValue::I32(1),
    ///     ModbusDataType::DWord,
    ///     ByteOrder::BigEndian,
    ///     WordOrder::HighWordFirst,
    /// );
    /// assert!(refused.is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-BE
    /// The same layouts as decoding: docs/parity/modbus.md#MB-BYTEORDER-BE,
    /// docs/parity/modbus.md#MB-BYTEORDER-LE, docs/parity/modbus.md#MB-BYTEORDER-BE-SWAP
    /// and docs/parity/modbus.md#MB-BYTEORDER-LE-SWAP. The write path of the
    /// connection uses it (docs/parity/modbus.md#MB-TCP-WRITE).
    pub fn encode_registers(
        value: &PlcValue,
        data_type: ModbusDataType,
        byte_order: ByteOrder,
        word_order: WordOrder,
    ) -> Result<Vec<u16>, PlcError> {
        let bytes = value_bytes(value, data_type)?;
        let mut reader = BoundedReader::new(&bytes);
        let mut words = Vec::new();
        while reader.remaining() > 0 {
            words.push(reader.read_u16(byte_order)?);
        }
        Ok(most_significant_first(&words, word_order))
    }
}

/// The status every decode failure carries: protocol `modbus`, code 0, name `decode`.
const fn decode_status() -> ProtocolStatus {
    ProtocolStatus::new(STATUS_PROTOCOL, 0).with_name(STATUS_NAME)
}

/// Attach `raw` to `status`, or return `status` unchanged when `raw` exceeds `budget`.
fn keep_raw(status: ProtocolStatus, raw: &[u8], budget: NonZeroUsize) -> ProtocolStatus {
    status.clone().with_raw(raw, budget).unwrap_or(status)
}

/// The `MalformedData` item error for a register slice of the wrong length.
fn wrong_register_count(
    registers: &[u16],
    data_type: ModbusDataType,
    raw_budget: NonZeroUsize,
) -> ItemError {
    let error = PlcError::new(
        ErrorKind::MalformedData,
        format!(
            "{} needs {} register(s) but {} were received",
            data_type.upstream_name(),
            data_type.register_size(),
            registers.len()
        ),
    );
    let status = decode_status();
    // The copy is made only when it fits, so an oversized input is never duplicated.
    let status = if registers.len().saturating_mul(2) <= raw_budget.get() {
        let wire: Vec<u8> = registers.iter().flat_map(|r| r.to_be_bytes()).collect();
        keep_raw(status, &wire, raw_budget)
    } else {
        status
    };
    ItemError::new(error).with_status(status)
}

/// The registers with the most significant one first, whichever way they were laid out.
///
/// Reversing is its own inverse, so the same function orders registers for decoding and
/// for encoding.
fn most_significant_first(registers: &[u16], word_order: WordOrder) -> Vec<u16> {
    let mut ordered = registers.to_vec();
    match word_order {
        WordOrder::HighWordFirst => {}
        WordOrder::LowWordFirst => ordered.reverse(),
    }
    ordered
}

/// Read the value from registers already ordered most significant first.
///
/// Each register is first turned into its two bytes in `byte_order`, which yields the
/// value's big-endian bytes; the value is then read big-endian. A lone narrow value is
/// the last byte of its 16-bit value (the first byte is padding).
fn read_value(
    ordered: &[u16],
    byte_order: ByteOrder,
    data_type: ModbusDataType,
) -> Result<PlcValue, PlcError> {
    let mut writer = BoundedWriter::new(MAX_VALUE_BYTES);
    for register in ordered {
        writer.write_u16(*register, byte_order)?;
    }
    let mut reader = BoundedReader::new(writer.as_slice());
    let big = ByteOrder::BigEndian;
    Ok(match data_type {
        ModbusDataType::Bool => {
            reader.read_u8()?;
            PlcValue::Bool((reader.read_u8()? & 1) == 1)
        }
        ModbusDataType::Byte | ModbusDataType::USInt => {
            reader.read_u8()?;
            PlcValue::U8(reader.read_u8()?)
        }
        ModbusDataType::SInt => {
            reader.read_u8()?;
            PlcValue::I8(reader.read_i8()?)
        }
        ModbusDataType::Word | ModbusDataType::UInt => PlcValue::U16(reader.read_u16(big)?),
        ModbusDataType::Int => PlcValue::I16(reader.read_i16(big)?),
        ModbusDataType::DWord | ModbusDataType::UDInt => PlcValue::U32(reader.read_u32(big)?),
        ModbusDataType::DInt => PlcValue::I32(reader.read_i32(big)?),
        ModbusDataType::LWord | ModbusDataType::ULInt => PlcValue::U64(reader.read_u64(big)?),
        ModbusDataType::LInt => PlcValue::I64(reader.read_i64(big)?),
        ModbusDataType::Real => PlcValue::F32(f32::from_bits(reader.read_u32(big)?)),
        ModbusDataType::LReal => PlcValue::F64(f64::from_bits(reader.read_u64(big)?)),
    })
}

/// The big-endian bytes of `value`, a lone narrow value padded to 16 bits, or an error
/// when the variant is not the one `data_type` decodes to.
///
/// The writer cannot overflow (the widest value is `MAX_VALUE_BYTES`), so the `?` on its
/// writes never fires; it converts a `ParseError` rather than ignoring it.
fn value_bytes(value: &PlcValue, data_type: ModbusDataType) -> Result<Vec<u8>, PlcError> {
    let mut writer = BoundedWriter::new(MAX_VALUE_BYTES);
    let big = ByteOrder::BigEndian;
    match (data_type, value) {
        (ModbusDataType::Bool, PlcValue::Bool(v)) => {
            writer.write_u8(0)?;
            writer.write_u8(u8::from(*v))?;
        }
        (ModbusDataType::Byte | ModbusDataType::USInt, PlcValue::U8(v)) => {
            writer.write_u8(0)?;
            writer.write_u8(*v)?;
        }
        (ModbusDataType::SInt, PlcValue::I8(v)) => {
            writer.write_u8(0)?;
            writer.write_i8(*v)?;
        }
        (ModbusDataType::Word | ModbusDataType::UInt, PlcValue::U16(v)) => {
            writer.write_u16(*v, big)?;
        }
        (ModbusDataType::Int, PlcValue::I16(v)) => writer.write_i16(*v, big)?,
        (ModbusDataType::DWord | ModbusDataType::UDInt, PlcValue::U32(v)) => {
            writer.write_u32(*v, big)?;
        }
        (ModbusDataType::DInt, PlcValue::I32(v)) => writer.write_i32(*v, big)?,
        (ModbusDataType::LWord | ModbusDataType::ULInt, PlcValue::U64(v)) => {
            writer.write_u64(*v, big)?;
        }
        (ModbusDataType::LInt, PlcValue::I64(v)) => writer.write_i64(*v, big)?,
        (ModbusDataType::Real, PlcValue::F32(v)) => writer.write_u32(v.to_bits(), big)?,
        (ModbusDataType::LReal, PlcValue::F64(v)) => writer.write_u64(v.to_bits(), big)?,
        _ => return Err(mismatch(data_type)),
    }
    Ok(writer.into_bytes())
}

/// The error for a value whose variant is not the one `data_type` decodes to.
fn mismatch(data_type: ModbusDataType) -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        format!(
            "the value is not a {} value; nothing is converted or widened",
            data_type.upstream_name()
        ),
    )
}
