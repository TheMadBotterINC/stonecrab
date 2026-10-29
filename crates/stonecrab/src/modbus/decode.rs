//! Typed decoding of register and coil data into [`PlcValue`]s, and the inverse for tests
//! and the future write path.
//!
//! The layout rules are stated once, on [`TypedDecoder`]; every function here applies
//! them through the bounds-checked primitives of `stonecrab::codec` (ADR-0013).

use std::num::{NonZeroU16, NonZeroUsize};

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
/// - Text (`CHAR`, `WCHAR`, `STRING`, `WSTRING`, ADR-0025) is a run of bytes, not a
///   number: the registers are read in order and never reversed, and the two bytes of each
///   register are taken high byte first, or low byte first under the two byte-swap modes,
///   where the byte order and the word order disagree (`BIG_ENDIAN_BYTE_SWAP` and
///   `LITTLE_ENDIAN_BYTE_SWAP`). A `CHAR` is the first of those bytes, one byte of UTF-8;
///   a `WCHAR` one UTF-16BE code unit; a `STRING` is UTF-8 and a `WSTRING` UTF-16BE up to
///   the first NUL. Bytes that are not valid UTF-8 or UTF-16 fail the read with the bytes
///   kept; nothing is replaced. These functions treat `STRING` and `WSTRING` as
///   `STRING(1)` and `WSTRING(1)`, the size [`ModbusDataType::register_size`] gives; a
///   read or write through an address applies its declared length
///   ([`ModbusAddress::string_length`](crate::modbus::ModbusAddress::string_length)).
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
    /// for `SINT`, `INT`, `DINT` and `LINT`, `F32` for `REAL`, `F64` for `LREAL`,
    /// `Bool` for `BOOL`, `Char` for `CHAR` and `WCHAR` and `String` for `STRING` and
    /// `WSTRING`, the same as [`ModbusDataType::value_type`].
    ///
    /// # Errors
    /// Returns an [`ItemError`] of kind [`ErrorKind::MalformedData`] when the number of
    /// registers is not the size of `data_type`, or when the bytes of a text type are not
    /// valid UTF-8 (`CHAR`: a byte above 0x7F) or UTF-16 (an unpaired surrogate). Its
    /// [`ProtocolStatus`] is protocol
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
    /// to docs/parity/modbus.md#MB-TYPE-LREAL and docs/parity/modbus.md#MB-TYPE-CHAR to
    /// docs/parity/modbus.md#MB-TYPE-WSTRING. Upstream decodes a whole read through
    /// its generated `DataItem`; Stonecrab rejects a wrong register count instead of
    /// reading what is there, and keeps the received bytes (decision D14, ADR-0006). Text
    /// that does not decode is an error, where upstream substitutes U+FFFD
    /// (docs/parity/modbus.md#MB-TYPE-STRING).
    pub fn decode_registers(
        registers: &[u16],
        data_type: ModbusDataType,
        byte_order: ByteOrder,
        word_order: WordOrder,
        raw_budget: NonZeroUsize,
    ) -> Result<PlcValue, ItemError> {
        let needed = usize::from(data_type.register_size());
        if registers.len() != needed {
            return Err(wrong_register_count(
                registers, data_type, needed, raw_budget,
            ));
        }
        let ordered = most_significant_first(registers, word_order);
        read_value(&ordered, byte_order, word_order, data_type)
            .map_err(|error| with_registers(error, registers, raw_budget))
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
    /// with zero padding bits for a lone 8-bit value or `BOOL`, and a zero byte after a
    /// `CHAR` or a `STRING(1)`. The result holds exactly
    /// [`ModbusDataType::register_size`] registers.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when the variant of `value` is not the one
    /// `data_type` decodes to (`PlcValue::U32` for `DWORD`, for example); nothing is
    /// converted or widened. A text value is refused the same way when it does not fit:
    /// a `CHAR` takes U+0000 to U+007F, a `WCHAR` one UTF-16 code unit (not a character
    /// outside the Basic Multilingual Plane), and a `STRING` or `WSTRING` refuses a NUL
    /// character, which would end it when it is read back.
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
    /// connection uses it (docs/parity/modbus.md#MB-TCP-WRITE). Text:
    /// docs/parity/modbus.md#MB-TYPE-CHAR to docs/parity/modbus.md#MB-TYPE-WSTRING.
    pub fn encode_registers(
        value: &PlcValue,
        data_type: ModbusDataType,
        byte_order: ByteOrder,
        word_order: WordOrder,
    ) -> Result<Vec<u16>, PlcError> {
        if data_type.is_text() {
            return encode_text(
                std::slice::from_ref(value),
                data_type,
                None,
                byte_order,
                word_order,
            );
        }
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

/// The `MalformedData` item error for a register slice of the wrong length, `needed`
/// being the right one.
fn wrong_register_count(
    registers: &[u16],
    data_type: ModbusDataType,
    needed: usize,
    raw_budget: NonZeroUsize,
) -> ItemError {
    let error = PlcError::new(
        ErrorKind::MalformedData,
        format!(
            "{} needs {needed} register(s) but {} were received",
            data_type.upstream_name(),
            registers.len()
        ),
    );
    with_registers(error, registers, raw_budget)
}

/// `error` as an item error whose status keeps `registers` as the bytes received (high
/// byte first) when they fit `raw_budget`.
fn with_registers(error: PlcError, registers: &[u16], raw_budget: NonZeroUsize) -> ItemError {
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

/// `error` as an item error whose status keeps `raw` (a reply as received) when there is
/// some and it fits `raw_budget`.
pub(super) fn with_raw_reply(
    error: PlcError,
    raw: Option<&[u8]>,
    raw_budget: NonZeroUsize,
) -> ItemError {
    let status = decode_status();
    ItemError::new(error)
        .with_status(raw.map_or(status.clone(), |raw| keep_raw(status, raw, raw_budget)))
}

/// Whether text takes the two bytes of each register low byte first: under the two
/// byte-swap modes, where the byte order and the word order disagree, as upstream swaps
/// the payload of `BIG_ENDIAN_BYTE_SWAP` and `LITTLE_ENDIAN_BYTE_SWAP` and reads strings
/// in buffer order otherwise (`Conn:498-505`, `Opt:165-187`).
const fn swaps_text(byte_order: ByteOrder, word_order: WordOrder) -> bool {
    matches!(
        (byte_order, word_order),
        (ByteOrder::LittleEndian, WordOrder::HighWordFirst)
            | (ByteOrder::BigEndian, WordOrder::LowWordFirst)
    )
}

/// The bytes one element of a text type occupies, `length` being the declared length of
/// a string (1 when `None`).
fn text_width(data_type: ModbusDataType, length: Option<NonZeroU16>) -> usize {
    usize::try_from(data_type.element_bytes(length.map_or(1, NonZeroU16::get)))
        .unwrap_or(usize::MAX)
}

/// Decode a text value of `data_type` from `registers`, which start at the value and may
/// run past it: one element, or `elements` of them packed one after another into a
/// [`PlcValue::List`]. A register run too short for the value is the wrong register count;
/// bytes that do not decode fail the whole value, keeping the registers it occupies.
pub(super) fn decode_text(
    registers: &[u16],
    data_type: ModbusDataType,
    length: Option<NonZeroU16>,
    elements: Option<NonZeroU16>,
    byte_order: ByteOrder,
    word_order: WordOrder,
    raw_budget: NonZeroUsize,
) -> Result<PlcValue, ItemError> {
    let width = text_width(data_type, length);
    let count = elements.map_or(1, |count| usize::from(count.get()));
    let needed = width.saturating_mul(count).div_ceil(2);
    let Some(words) = registers.get(..needed) else {
        return Err(wrong_register_count(
            registers, data_type, needed, raw_budget,
        ));
    };
    let bytes = text_order(words, byte_order, word_order);
    let decoded = match elements {
        None => text_value(bytes.get(..width).unwrap_or(&bytes), data_type),
        Some(_) => bytes
            .chunks(width)
            .take(count)
            .map(|element| text_value(element, data_type))
            .collect::<Result<Vec<_>, _>>()
            .map(PlcValue::List),
    };
    decoded.map_err(|error| with_registers(error, words, raw_budget))
}

/// The bytes of text registers in reading order: each register high byte first, or low
/// byte first when [`swaps_text`].
fn text_order(registers: &[u16], byte_order: ByteOrder, word_order: WordOrder) -> Vec<u8> {
    let swap = swaps_text(byte_order, word_order);
    registers
        .iter()
        .flat_map(|word| {
            if swap {
                word.to_le_bytes()
            } else {
                word.to_be_bytes()
            }
        })
        .collect()
}

/// The value of one text element from its bytes, already in reading order.
fn text_value(bytes: &[u8], data_type: ModbusDataType) -> Result<PlcValue, PlcError> {
    let character = matches!(data_type, ModbusDataType::Char | ModbusDataType::WChar);
    let text = if matches!(data_type, ModbusDataType::WChar | ModbusDataType::WString) {
        // A string ends at its first NUL code unit; a character is its one unit.
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(u16::from_be_bytes)
            .take_while(|unit| character || *unit != 0)
            .collect();
        char::decode_utf16(units)
            .collect::<Result<String, _>>()
            .map_err(|error| {
                malformed(format!(
                    "{} holds the unpaired UTF-16 surrogate 0x{:04X}",
                    data_type.upstream_name(),
                    error.unpaired_surrogate()
                ))
            })?
    } else {
        // A string ends at its first NUL byte; a character is its one byte.
        let content = if character {
            bytes
        } else {
            bytes.split(|byte| *byte == 0).next().unwrap_or(bytes)
        };
        std::str::from_utf8(content)
            .map_err(|error| {
                malformed(format!(
                    "{} is not valid UTF-8 from byte {}",
                    data_type.upstream_name(),
                    error.valid_up_to()
                ))
            })?
            .to_owned()
    };
    Ok(if character {
        PlcValue::Char(text.chars().next().unwrap_or_default())
    } else {
        PlcValue::String(text)
    })
}

/// Encode `values`, one text value per element, packed one after another and padded with
/// zero bytes: each element to its declared width, the run to a whole register. A value of
/// the wrong variant, one that does not fit its element and a string holding a NUL are
/// refused.
pub(super) fn encode_text(
    values: &[PlcValue],
    data_type: ModbusDataType,
    length: Option<NonZeroU16>,
    byte_order: ByteOrder,
    word_order: WordOrder,
) -> Result<Vec<u16>, PlcError> {
    let width = text_width(data_type, length);
    let mut bytes = Vec::with_capacity(width.saturating_mul(values.len()).saturating_add(1));
    for value in values {
        let mut element = text_bytes(value, data_type, width)?;
        element.resize(width, 0);
        bytes.append(&mut element);
    }
    bytes.resize(bytes.len().div_ceil(2).saturating_mul(2), 0);
    let swap = swaps_text(byte_order, word_order);
    Ok(bytes
        .as_chunks::<2>()
        .0
        .iter()
        .copied()
        .map(|pair| {
            if swap {
                u16::from_le_bytes(pair)
            } else {
                u16::from_be_bytes(pair)
            }
        })
        .collect())
}

/// The UTF-8 or UTF-16BE bytes of one text value, at most `width` of them.
fn text_bytes(
    value: &PlcValue,
    data_type: ModbusDataType,
    width: usize,
) -> Result<Vec<u8>, PlcError> {
    let character = matches!(data_type, ModbusDataType::Char | ModbusDataType::WChar);
    let text = match (character, value) {
        (true, PlcValue::Char(c)) => c.to_string(),
        (false, PlcValue::String(text)) if !text.contains('\0') => text.clone(),
        (false, PlcValue::String(_)) => {
            return Err(refused(format!(
                "the text holds a NUL character, which would end the {} when it is read back",
                data_type.upstream_name()
            )));
        }
        _ => return Err(mismatch(data_type)),
    };
    let (encoded, encoding): (Vec<u8>, _) =
        if matches!(data_type, ModbusDataType::WChar | ModbusDataType::WString) {
            (
                text.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                "UTF-16",
            )
        } else {
            (text.into_bytes(), "UTF-8")
        };
    if encoded.len() > width {
        return Err(refused(format!(
            "the value takes {} bytes of {encoding}, more than the {width} of one {} element",
            encoded.len(),
            data_type.upstream_name()
        )));
    }
    Ok(encoded)
}

/// A `MalformedData` error of a value that came off the wire.
fn malformed(message: String) -> PlcError {
    PlcError::new(ErrorKind::MalformedData, message)
}

/// An `InvalidConfig` error of a value that cannot be laid out for its type.
fn refused(message: String) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
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
/// the last byte of its 16-bit value (the first byte is padding). Text is one register
/// here, read byte by byte as [`decode_text`] reads it.
fn read_value(
    ordered: &[u16],
    byte_order: ByteOrder,
    word_order: WordOrder,
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
        ModbusDataType::Char
        | ModbusDataType::WChar
        | ModbusDataType::String
        | ModbusDataType::WString => {
            let bytes = text_order(ordered, byte_order, word_order);
            let width = text_width(data_type, None);
            return text_value(bytes.get(..width).unwrap_or(&bytes), data_type);
        }
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
        // Text never reaches here: `encode_registers` hands it to `encode_text`.
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const BUDGET: NonZeroUsize = NonZeroUsize::MIN.saturating_add(63);

    /// The four (byte order, word order) pairs, the upstream modes.
    const ORDERS: [(ByteOrder, WordOrder); 4] = [
        (ByteOrder::BigEndian, WordOrder::HighWordFirst),
        (ByteOrder::LittleEndian, WordOrder::LowWordFirst),
        (ByteOrder::LittleEndian, WordOrder::HighWordFirst),
        (ByteOrder::BigEndian, WordOrder::LowWordFirst),
    ];

    // covers: MB-TYPE-STRING / a run of registers too short for the declared length is the
    // wrong register count, with the registers that arrived kept
    #[test]
    fn a_run_too_short_for_the_text_is_a_wrong_register_count() {
        let length = NonZeroU16::new(4);
        let error = decode_text(
            &[0x4142],
            ModbusDataType::String,
            length,
            None,
            ByteOrder::BigEndian,
            WordOrder::HighWordFirst,
            BUDGET,
        )
        .err();
        assert_eq!(
            error.as_ref().map(ItemError::kind),
            Some(ErrorKind::MalformedData)
        );
        assert!(
            error
                .as_ref()
                .is_some_and(|error| error.to_string().contains("needs 2 register(s) but 1")),
            "{error:?}"
        );
        assert_eq!(
            error
                .as_ref()
                .and_then(ItemError::status)
                .and_then(ProtocolStatus::raw),
            Some(&[0x41, 0x42][..]),
            "the bytes that arrived"
        );
    }

    // covers: MB-TYPE-STRING, MB-TYPE-WSTRING, MB-TYPE-CHAR / SPEC mandatory-failure-cases /
    // F7 malformed data: bytes that are not UTF-8 or UTF-16 are an error, never replaced,
    // and a bit past the bytes received is one too
    #[test]
    fn text_that_does_not_decode_is_malformed_data_with_the_registers_kept() {
        for (data_type, length, registers, needle) in [
            (
                ModbusDataType::String,
                4,
                [0x61FF, 0x6200],
                "UTF-8 from byte 1",
            ),
            (ModbusDataType::WString, 2, [0xD800, 0x0041], "0xD800"),
        ] {
            let error = decode_text(
                &registers,
                data_type,
                NonZeroU16::new(length),
                None,
                ByteOrder::BigEndian,
                WordOrder::HighWordFirst,
                BUDGET,
            )
            .err();
            assert_eq!(
                error.as_ref().map(ItemError::kind),
                Some(ErrorKind::MalformedData),
                "{data_type:?}"
            );
            assert!(
                error
                    .as_ref()
                    .is_some_and(|error| error.to_string().contains(needle)),
                "{data_type:?}: {error:?}"
            );
            let raw: Vec<u8> = registers
                .iter()
                .flat_map(|word| word.to_be_bytes())
                .collect();
            assert_eq!(
                error
                    .as_ref()
                    .and_then(ItemError::status)
                    .and_then(ProtocolStatus::raw),
                Some(raw.as_slice()),
                "{data_type:?}: the registers as received"
            );
        }
        let bit = TypedDecoder::decode_bits(&[0x01], 8, ModbusDataType::Bool, BUDGET);
        assert_eq!(
            bit.map_err(|error| error.kind()),
            Err(ErrorKind::MalformedData)
        );
    }

    // covers: MB-TYPE-STRING, MB-TYPE-CHAR / SPEC mandatory-failure-cases / F7 values that
    // do not fit are refused: a NUL inside a string, a value of another variant
    #[test]
    fn a_nul_in_a_string_and_a_value_of_another_variant_are_refused() {
        let big = (ByteOrder::BigEndian, WordOrder::HighWordFirst);
        for (value, data_type, needle) in [
            (
                PlcValue::String("a\0b".to_owned()),
                ModbusDataType::String,
                "NUL",
            ),
            (
                PlcValue::String("a".to_owned()),
                ModbusDataType::Char,
                "not a CHAR",
            ),
            (
                PlcValue::Char('a'),
                ModbusDataType::WString,
                "not a WSTRING",
            ),
        ] {
            let refused = encode_text(&[value], data_type, NonZeroU16::new(4), big.0, big.1);
            assert!(
                refused
                    .as_ref()
                    .is_err_and(|error| error.kind() == ErrorKind::InvalidConfig
                        && error.to_string().contains(needle)),
                "{data_type:?}: {refused:?}"
            );
        }
    }

    /// A text type, a declared length, an element count or none, and texts that fit.
    fn text_case() -> impl Strategy<Value = (ModbusDataType, u16, Option<u16>, Vec<String>)> {
        (
            prop::sample::select(vec![
                ModbusDataType::Char,
                ModbusDataType::WChar,
                ModbusDataType::String,
                ModbusDataType::WString,
            ]),
            1_u16..=24,
            prop::option::of(1_u16..=5),
        )
            .prop_flat_map(|(data_type, length, count)| {
                let element = match data_type {
                    ModbusDataType::Char => "[\\x00-\\x7F]",
                    ModbusDataType::WChar => "[\\x00-\\u{D7FF}\\u{E000}-\\u{FFFF}]",
                    ModbusDataType::String | ModbusDataType::WString | _ => "[^\\x00]{0,24}",
                };
                let elements = usize::from(count.unwrap_or(1));
                (
                    Just(data_type),
                    Just(length),
                    Just(count),
                    prop::collection::vec(element, elements),
                )
            })
    }

    proptest! {
        // covers: MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING,
        // MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP / every
        // text that fits its element is written and read back unchanged, in every mode, alone
        // and in a list; one that does not fit is refused
        #[test]
        fn prop_text_round_trips_in_every_mode(case in text_case(), mode in 0_usize..4) {
            let (data_type, length, count, texts) = case;
            let (byte_order, word_order) = ORDERS.get(mode).copied().unwrap_or(ORDERS[0]);
            let character = matches!(data_type, ModbusDataType::Char | ModbusDataType::WChar);
            let length = if character { None } else { NonZeroU16::new(length) };
            let values: Vec<PlcValue> = texts
                .iter()
                .map(|text| {
                    if character {
                        PlcValue::Char(text.chars().next().unwrap_or_default())
                    } else {
                        PlcValue::String(text.clone())
                    }
                })
                .collect();
            let width = text_width(data_type, length);
            let fits = values.iter().all(|value| {
                text_bytes(value, data_type, usize::MAX).is_ok_and(|bytes| bytes.len() <= width)
            });
            let written = encode_text(&values, data_type, length, byte_order, word_order);
            prop_assert_eq!(written.is_ok(), fits);
            if let Ok(registers) = written {
                prop_assert_eq!(registers.len(), width.saturating_mul(values.len()).div_ceil(2));
                let elements = count.and_then(NonZeroU16::new);
                let read = decode_text(
                    &registers, data_type, length, elements, byte_order, word_order, BUDGET,
                );
                let expected = match elements {
                    Some(_) => PlcValue::List(values.clone()),
                    None => values.first().cloned().unwrap_or(PlcValue::Null),
                };
                prop_assert_eq!(read.ok(), Some(expected));
            }
        }
    }
}
