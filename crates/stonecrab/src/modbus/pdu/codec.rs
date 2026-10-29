//! Encoding and decoding of the read and write PDUs.

use crate::ByteOrder;
use crate::codec::{BoundedReader, BoundedWriter, ParseError, coil_byte_count};

use super::model::{check_length, discriminator};
use super::{
    ADDRESS_SPACE, ExceptionCode, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS,
    MAX_RESPONSE_DATA_BYTES, MAX_WRITE_COILS, MAX_WRITE_REGISTERS, ModbusPdu,
};

/// Builds a request variant from its `(start, quantity)`.
type RequestBuilder = fn(u16, u16) -> ModbusPdu;

/// The field name of a function code that is not the expected one or not a supported one.
const FUNCTION_FIELD: &str = "function code";

/// Checks the quantity of a read or write request against `max` and the end of the address
/// space.
pub(super) fn check_request(start: u16, quantity: u16, max: u16) -> Result<(), ParseError> {
    if quantity == 0 {
        return Err(ParseError::InvalidDiscriminator {
            field: "quantity",
            value: 0,
        });
    }
    if quantity > max {
        return Err(ParseError::OversizedLength {
            length: usize::from(quantity),
            max: usize::from(max),
        });
    }
    let end = usize::from(start).saturating_add(usize::from(quantity));
    if end > ADDRESS_SPACE {
        return Err(ParseError::OversizedLength {
            length: end,
            max: ADDRESS_SPACE,
        });
    }
    Ok(())
}

/// The five bytes of a PDU that is a function code and two big-endian words: a read
/// request or a write multiple (start, quantity), a write single register request or
/// response (address, value).
const fn function_and_words(function: FunctionCode, first: u16, second: u16) -> [u8; 5] {
    let [first_hi, first_lo] = first.to_be_bytes();
    let [second_hi, second_lo] = second.to_be_bytes();
    [function.as_u8(), first_hi, first_lo, second_hi, second_lo]
}

/// Checks a response byte count: 1 to 250 and, for registers, even. Returns the count as
/// the single byte it is on the wire.
fn check_byte_count(len: usize, whole_registers: bool) -> Result<u8, ParseError> {
    let count = u8::try_from(len)
        .ok()
        .filter(|count| usize::from(*count) <= MAX_RESPONSE_DATA_BYTES);
    match count {
        None => Err(ParseError::OversizedLength {
            length: len,
            max: MAX_RESPONSE_DATA_BYTES,
        }),
        Some(0) => Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: 0,
        }),
        Some(count) if whole_registers && !len.is_multiple_of(2) => {
            Err(ParseError::InvalidDiscriminator {
                field: "register byte count",
                value: u64::from(count),
            })
        }
        Some(count) => Ok(count),
    }
}

/// Encodes a request: the function code and its fields, big-endian.
///
/// A read request is the wire address and the quantity; Write Single Register is the
/// address and the value; Write Multiple Coils and Write Multiple Registers are the start
/// address, the quantity, the byte count and the data. The request is validated as a
/// decoder would validate it (quantity 1 to 2000 bits or 1 to 125 registers to read, 1 to
/// 1968 coils or 1 to 123 registers to write, `start + quantity` at most 65536, a coil
/// write holding exactly `ceil(quantity / 8)` bytes), so a request this function accepts
/// is one [`decode_request`] accepts. Nothing is written when it fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is not a request or the quantity is 0;
/// [`ParseError::OversizedLength`] when the quantity or the end of the span is too large,
/// or the writer cannot hold the PDU; [`ParseError::Truncated`] or
/// [`ParseError::TrailingBytes`] when the data of a coil write is not
/// `ceil(quantity / 8)` bytes.
///
/// # Examples
/// ```
/// use stonecrab::codec::BoundedWriter;
/// use stonecrab::modbus::{ModbusPdu, encode_request};
///
/// let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
/// let mut writer = BoundedWriter::new(253);
/// encode_request(&request, &mut writer)?;
/// assert_eq!(writer.as_slice(), [0x03, 0x00, 0x6B, 0x00, 0x03]);
///
/// let write = ModbusPdu::WriteMultipleRegisters { start: 1, registers: vec![10, 258] };
/// let mut writer = BoundedWriter::new(253);
/// encode_request(&write, &mut writer)?;
/// assert_eq!(
///     writer.as_slice(),
///     [0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02]
/// );
/// # Ok::<(), stonecrab::codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The read bytes are those of upstream's `ModbusPDUReadHoldingRegistersRequest` and its
/// three siblings (`MB-TCP-READ-COIL`, `MB-TCP-READ-DI`, `MB-TCP-READ-IR`); the quantity
/// bounds are `MB-LIMIT-COILS` and `MB-LIMIT-REGISTERS` and the span bound is
/// `MB-LIMIT-SPAN`. The write bytes are those of upstream's write requests
/// (`MB-TCP-WRITE`); the write bounds are V1.1b sections 6.6, 6.11 and 6.12.
pub fn encode_request(pdu: &ModbusPdu, writer: &mut BoundedWriter) -> Result<(), ParseError> {
    let function = pdu.function();
    let mut out = Vec::new();
    match pdu {
        ModbusPdu::ReadCoils { start, quantity }
        | ModbusPdu::ReadDiscreteInputs { start, quantity } => {
            check_request(*start, *quantity, MAX_READ_BITS)?;
            out.extend_from_slice(&function_and_words(function, *start, *quantity));
        }
        ModbusPdu::ReadHoldingRegisters { start, quantity }
        | ModbusPdu::ReadInputRegisters { start, quantity } => {
            check_request(*start, *quantity, MAX_READ_REGISTERS)?;
            out.extend_from_slice(&function_and_words(function, *start, *quantity));
        }
        ModbusPdu::WriteSingleRegister { address, value } => {
            out.extend_from_slice(&function_and_words(function, *address, *value));
        }
        ModbusPdu::WriteMultipleCoils {
            start,
            quantity,
            bytes,
        } => {
            check_request(*start, *quantity, MAX_WRITE_COILS)?;
            check_length(coil_byte_count(usize::from(*quantity)), bytes.len())?;
            out.extend_from_slice(&function_and_words(function, *start, *quantity));
            out.push(check_byte_count(bytes.len(), false)?);
            out.extend_from_slice(bytes);
        }
        ModbusPdu::WriteMultipleRegisters { start, registers } => {
            let Ok(quantity) = u16::try_from(registers.len()) else {
                return Err(ParseError::OversizedLength {
                    length: registers.len(),
                    max: usize::from(MAX_WRITE_REGISTERS),
                });
            };
            check_request(*start, quantity, MAX_WRITE_REGISTERS)?;
            out.extend_from_slice(&function_and_words(function, *start, quantity));
            out.push(check_byte_count(registers.len().saturating_mul(2), true)?);
            out.extend(registers.iter().flat_map(|register| register.to_be_bytes()));
        }
        ModbusPdu::ReadBitsResponse { .. }
        | ModbusPdu::ReadRegistersResponse { .. }
        | ModbusPdu::WriteSingleRegisterResponse { .. }
        | ModbusPdu::WriteMultipleResponse { .. }
        | ModbusPdu::Exception { .. } => {
            return Err(discriminator("request function code", function));
        }
    }
    writer.write_bytes(&out)
}

/// Decodes a request, the form a responder or a fuzz target sees.
///
/// A read request must be exactly the function code, the start address and the quantity,
/// with the quantity in 1..=2000 for function codes 01 and 02 or 1..=125 for 03 and 04. A
/// Write Single Register request (06) is the function code, the address and the value. A
/// Write Multiple Coils (15) or Write Multiple Registers (16) request is the function
/// code, the start address, the quantity (1..=1968 coils or 1..=123 registers), a byte
/// count and that many data bytes; the byte count must be `ceil(quantity / 8)` for coils
/// and `2 * quantity` for registers, and is compared with the quantity before the data is
/// read. `start + quantity` is at most 65536 for all of them. Every other function code,
/// including Write Single Coil (05), is refused. The checks run in this order: PDU size,
/// function, truncation of the fixed fields, trailing bytes (reads and function 06),
/// quantity, span, byte count, truncation of the data, trailing bytes (function 15 and 16).
///
/// # Errors
/// [`ParseError::OversizedLength`] for a PDU over 253 bytes, a quantity over its maximum or
/// a span past the address space; [`ParseError::InvalidDiscriminator`] for an unsupported
/// function code (field `function code`), a quantity of 0, a write byte count that does
/// not fit the quantity (field `byte count`) or an odd register byte count (field
/// `register byte count`); [`ParseError::Truncated`] and [`ParseError::TrailingBytes`]
/// when the input is not exactly the length its fields imply.
///
/// # Examples
/// ```
/// use stonecrab::codec::BoundedReader;
/// use stonecrab::modbus::{ModbusPdu, decode_request};
///
/// let bytes = [0x01, 0x00, 0x13, 0x00, 0x13];
/// let request = decode_request(&mut BoundedReader::new(&bytes))?;
/// assert_eq!(request, ModbusPdu::ReadCoils { start: 19, quantity: 19 });
///
/// // V1.1b section 6.11: ten coils from address 19.
/// let bytes = [0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01];
/// let request = decode_request(&mut BoundedReader::new(&bytes))?;
/// assert_eq!(
///     request,
///     ModbusPdu::WriteMultipleCoils { start: 19, quantity: 10, bytes: vec![0xCD, 0x01] }
/// );
/// # Ok::<(), stonecrab::codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-COIL
/// The layouts are upstream's four read request PDUs (`MB-TCP-READ-COIL`,
/// `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-TCP-READ-IR`) and its three write request PDUs
/// (`MB-TCP-WRITE`), with the bounds `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`,
/// `MB-LIMIT-SPAN` and `MB-LIMIT-PDU`. Function 05 is not decoded (`MB-QUIRK-C`).
pub fn decode_request(reader: &mut BoundedReader<'_>) -> Result<ModbusPdu, ParseError> {
    check_pdu_size(reader)?;
    let function = FunctionCode::from_u8(reader.read_u8()?);
    let (build, max): (RequestBuilder, u16) = match function {
        FunctionCode::ReadCoils => (
            |start, quantity| ModbusPdu::ReadCoils { start, quantity },
            MAX_READ_BITS,
        ),
        FunctionCode::ReadDiscreteInputs => (
            |start, quantity| ModbusPdu::ReadDiscreteInputs { start, quantity },
            MAX_READ_BITS,
        ),
        FunctionCode::ReadHoldingRegisters => (
            |start, quantity| ModbusPdu::ReadHoldingRegisters { start, quantity },
            MAX_READ_REGISTERS,
        ),
        FunctionCode::ReadInputRegisters => (
            |start, quantity| ModbusPdu::ReadInputRegisters { start, quantity },
            MAX_READ_REGISTERS,
        ),
        FunctionCode::WriteSingleRegister => {
            let address = reader.read_u16(ByteOrder::BigEndian)?;
            let value = reader.read_u16(ByteOrder::BigEndian)?;
            reader.expect_eof()?;
            return Ok(ModbusPdu::WriteSingleRegister { address, value });
        }
        FunctionCode::WriteMultipleCoils => return decode_write_coils(reader),
        FunctionCode::WriteMultipleRegisters => return decode_write_registers(reader),
        FunctionCode::Unknown(_) => return Err(discriminator(FUNCTION_FIELD, function)),
    };
    let start = reader.read_u16(ByteOrder::BigEndian)?;
    let quantity = reader.read_u16(ByteOrder::BigEndian)?;
    reader.expect_eof()?;
    check_request(start, quantity, max)?;
    Ok(build(start, quantity))
}

/// Reads the start address, the quantity and the byte count of a Write Multiple request
/// and checks the quantity against `max` and the address space.
fn read_write_header(
    reader: &mut BoundedReader<'_>,
    max: u16,
) -> Result<(u16, u16, u8), ParseError> {
    let start = reader.read_u16(ByteOrder::BigEndian)?;
    let quantity = reader.read_u16(ByteOrder::BigEndian)?;
    let count = reader.read_u8()?;
    check_request(start, quantity, max)?;
    Ok((start, quantity, count))
}

/// Decodes the fields after the function code of a Write Multiple Coils request. The byte
/// count is proved against the quantity before the data is read or copied.
fn decode_write_coils(reader: &mut BoundedReader<'_>) -> Result<ModbusPdu, ParseError> {
    let (start, quantity, count) = read_write_header(reader, MAX_WRITE_COILS)?;
    if usize::from(count) != coil_byte_count(usize::from(quantity)) {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: u64::from(count),
        });
    }
    let bytes = reader
        .read_bytes(usize::from(count), MAX_PDU_BYTES)?
        .to_vec();
    reader.expect_eof()?;
    Ok(ModbusPdu::WriteMultipleCoils {
        start,
        quantity,
        bytes,
    })
}

/// Decodes the fields after the function code of a Write Multiple Registers request. The
/// byte count is proved against the quantity before the data is read or copied.
fn decode_write_registers(reader: &mut BoundedReader<'_>) -> Result<ModbusPdu, ParseError> {
    let (start, quantity, count) = read_write_header(reader, MAX_WRITE_REGISTERS)?;
    if !count.is_multiple_of(2) {
        return Err(ParseError::InvalidDiscriminator {
            field: "register byte count",
            value: u64::from(count),
        });
    }
    if usize::from(count) != usize::from(quantity).saturating_mul(2) {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: u64::from(count),
        });
    }
    let registers = reader
        .read_bytes(usize::from(count), MAX_PDU_BYTES)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_be_bytes(*pair))
        .collect();
    reader.expect_eof()?;
    Ok(ModbusPdu::WriteMultipleRegisters { start, registers })
}

/// Refuses input longer than a PDU can be, before anything is parsed.
const fn check_pdu_size(reader: &BoundedReader<'_>) -> Result<(), ParseError> {
    if reader.remaining() > MAX_PDU_BYTES {
        return Err(ParseError::OversizedLength {
            length: reader.remaining(),
            max: MAX_PDU_BYTES,
        });
    }
    Ok(())
}

/// Decodes the response to a request for `expected`: the matching normal response or the
/// matching exception response.
///
/// A normal read response is the function code, a byte count and that many data bytes: 1
/// to 250 bytes of packed bits for function codes 01 and 02, or a whole number of registers
/// (an even byte count of 2 to 250) for 03 and 04. A write response echoes the request: the
/// address and value for function code 06, the start address and quantity (1..=1968 coils
/// or 1..=123 registers, within the address space) for 15 and 16. An exception response is
/// the function code with the high bit set and one of the ten exception codes of V1.1b
/// section 7. The input must hold exactly one PDU. The checks run in this order: PDU size
/// (at most 253 bytes), function, byte count (zero, over 250, odd for registers),
/// truncation, trailing bytes, then for function codes 15 and 16 the quantity and the
/// span; every length is compared with its bound before the result is allocated.
///
/// The response does not carry the request's quantity, so a bit response is accepted
/// whatever its byte count within 1..=250. Pass the result to
/// [`ModbusPdu::check_reply_to`] with the request to prove the byte count against the
/// quantity, or the echo of a write against what was sent.
///
/// An exception code outside the ten defined values (0, 9, 12 and above) is refused rather
/// than carried: a reply the codec cannot name is not trusted to be an exception at all.
///
/// # Errors
/// - [`ParseError::OversizedLength`]: a PDU over 253 bytes, a byte count over 250, or an
///   echoed quantity over its maximum or a span past the address space.
/// - [`ParseError::InvalidDiscriminator`]: field `function code` when the function (with or
///   without the high bit) is not `expected`, `unsupported function code` when it is
///   `expected` but not a supported code (05 and the file record codes), `byte count` for
///   a count of 0, `register byte count` for an odd count, `quantity` for an echoed
///   quantity of 0 and `exception code` for a code outside the ten defined ones.
/// - [`ParseError::Truncated`]: the input ends before the function code, the byte count,
///   the exception code or the declared data.
/// - [`ParseError::TrailingBytes`]: bytes remain after the PDU.
///
/// # Examples
/// ```
/// use stonecrab::codec::BoundedReader;
/// use stonecrab::modbus::{FunctionCode, ModbusPdu, decode_response};
///
/// let bytes = [0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
/// let response = decode_response(
///     &mut BoundedReader::new(&bytes),
///     FunctionCode::ReadHoldingRegisters,
/// )?;
/// assert_eq!(
///     response,
///     ModbusPdu::ReadRegistersResponse {
///         function: FunctionCode::ReadHoldingRegisters,
///         registers: vec![555, 0, 100],
///     }
/// );
///
/// // V1.1b section 6.12: two registers written from address 1.
/// let bytes = [0x10, 0x00, 0x01, 0x00, 0x02];
/// let echo = decode_response(
///     &mut BoundedReader::new(&bytes),
///     FunctionCode::WriteMultipleRegisters,
/// )?;
/// assert_eq!(
///     echo,
///     ModbusPdu::WriteMultipleResponse {
///         function: FunctionCode::WriteMultipleRegisters,
///         start: 1,
///         quantity: 2,
///     }
/// );
/// # Ok::<(), stonecrab::codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The layouts are the responses of upstream's four read PDUs (`MB-TCP-READ-COIL`,
/// `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-TCP-READ-IR`), its three write PDUs
/// (`MB-TCP-WRITE`) and its error PDU, with the bounds `MB-LIMIT-COILS`,
/// `MB-LIMIT-REGISTERS` and `MB-LIMIT-PDU`. The ten exception codes are `MB-EXC-01` to
/// `MB-EXC-0B`; refusing any other code is Stonecrab's reading of `MB-EXC-UNKNOWN`.
pub fn decode_response(
    reader: &mut BoundedReader<'_>,
    expected: FunctionCode,
) -> Result<ModbusPdu, ParseError> {
    check_pdu_size(reader)?;
    let first = reader.read_u8()?;
    let function = FunctionCode::from_u8(first & 0x7F);
    if function != expected {
        return Err(ParseError::InvalidDiscriminator {
            field: FUNCTION_FIELD,
            value: u64::from(first),
        });
    }
    let exception = first & 0x80 != 0;
    match function {
        FunctionCode::ReadCoils | FunctionCode::ReadDiscreteInputs if !exception => {
            let bytes = read_counted(reader, false)?.to_vec();
            reader.expect_eof()?;
            Ok(ModbusPdu::ReadBitsResponse { function, bytes })
        }
        FunctionCode::ReadHoldingRegisters | FunctionCode::ReadInputRegisters if !exception => {
            let registers = read_counted(reader, true)?
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes(*pair))
                .collect();
            reader.expect_eof()?;
            Ok(ModbusPdu::ReadRegistersResponse {
                function,
                registers,
            })
        }
        FunctionCode::WriteSingleRegister if !exception => {
            let address = reader.read_u16(ByteOrder::BigEndian)?;
            let value = reader.read_u16(ByteOrder::BigEndian)?;
            reader.expect_eof()?;
            Ok(ModbusPdu::WriteSingleRegisterResponse { address, value })
        }
        FunctionCode::WriteMultipleCoils | FunctionCode::WriteMultipleRegisters if !exception => {
            let start = reader.read_u16(ByteOrder::BigEndian)?;
            let quantity = reader.read_u16(ByteOrder::BigEndian)?;
            reader.expect_eof()?;
            let max = if function == FunctionCode::WriteMultipleCoils {
                MAX_WRITE_COILS
            } else {
                MAX_WRITE_REGISTERS
            };
            check_request(start, quantity, max)?;
            Ok(ModbusPdu::WriteMultipleResponse {
                function,
                start,
                quantity,
            })
        }
        FunctionCode::ReadCoils
        | FunctionCode::ReadDiscreteInputs
        | FunctionCode::ReadHoldingRegisters
        | FunctionCode::ReadInputRegisters
        | FunctionCode::WriteSingleRegister
        | FunctionCode::WriteMultipleCoils
        | FunctionCode::WriteMultipleRegisters => decode_exception(reader, function),
        FunctionCode::Unknown(_) => Err(ParseError::InvalidDiscriminator {
            field: "unsupported function code",
            value: u64::from(first),
        }),
    }
}

/// Reads a byte count and that many bytes, checking the count before the bytes.
fn read_counted<'a>(
    reader: &mut BoundedReader<'a>,
    whole_registers: bool,
) -> Result<&'a [u8], ParseError> {
    let count = check_byte_count(usize::from(reader.read_u8()?), whole_registers)?;
    reader.read_bytes(usize::from(count), MAX_RESPONSE_DATA_BYTES)
}

/// Decodes the exception code after the function byte of an exception response.
fn decode_exception(
    reader: &mut BoundedReader<'_>,
    function: FunctionCode,
) -> Result<ModbusPdu, ParseError> {
    let raw = reader.read_u8()?;
    let code = ExceptionCode::from_u8(raw);
    if matches!(code, ExceptionCode::Unknown(_)) {
        return Err(ParseError::InvalidDiscriminator {
            field: "exception code",
            value: u64::from(raw),
        });
    }
    reader.expect_eof()?;
    Ok(ModbusPdu::Exception { function, code })
}

/// Encodes a response PDU: a normal read or write response or an exception response.
///
/// The response is validated as [`decode_response`] validates it (1 to 250 bytes of bits,
/// 1 to 125 registers, an echoed quantity of 1 to 1968 coils or 1 to 123 registers, a
/// function that matches the variant, one of the ten defined exception codes), so a value
/// this function accepts decodes back to itself. It is meant for tests and for the
/// responder of the contract suite. Nothing is written when it fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is a request, when the function does not
/// belong to the variant (or is not a supported function, for an exception), when the data
/// is empty, when the echoed quantity is 0 or when the exception code is
/// `ExceptionCode::Unknown`; [`ParseError::OversizedLength`] when the data is longer than
/// 250 bytes, the echoed quantity or span is too large or the writer cannot hold the PDU.
///
/// # Examples
/// ```
/// use stonecrab::codec::BoundedWriter;
/// use stonecrab::modbus::{ExceptionCode, FunctionCode, ModbusPdu, encode_response};
///
/// let refusal = ModbusPdu::Exception {
///     function: FunctionCode::ReadHoldingRegisters,
///     code: ExceptionCode::IllegalDataAddress,
/// };
/// let mut writer = BoundedWriter::new(253);
/// encode_response(&refusal, &mut writer)?;
/// assert_eq!(writer.as_slice(), [0x83, 0x02]);
/// # Ok::<(), stonecrab::codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The same response layouts and bounds as [`decode_response`]; the rows are the same.
pub fn encode_response(pdu: &ModbusPdu, writer: &mut BoundedWriter) -> Result<(), ParseError> {
    let mut out = Vec::new();
    match pdu {
        ModbusPdu::ReadBitsResponse { function, bytes } => {
            if !matches!(
                function,
                FunctionCode::ReadCoils | FunctionCode::ReadDiscreteInputs
            ) {
                return Err(discriminator("response function code", *function));
            }
            out.push(function.as_u8());
            out.push(check_byte_count(bytes.len(), false)?);
            out.extend_from_slice(bytes);
        }
        ModbusPdu::ReadRegistersResponse {
            function,
            registers,
        } => {
            if !matches!(
                function,
                FunctionCode::ReadHoldingRegisters | FunctionCode::ReadInputRegisters
            ) {
                return Err(discriminator("response function code", *function));
            }
            out.push(function.as_u8());
            out.push(check_byte_count(registers.len().saturating_mul(2), true)?);
            out.extend(registers.iter().flat_map(|register| register.to_be_bytes()));
        }
        ModbusPdu::WriteSingleRegisterResponse { address, value } => {
            out.extend_from_slice(&function_and_words(
                FunctionCode::WriteSingleRegister,
                *address,
                *value,
            ));
        }
        ModbusPdu::WriteMultipleResponse {
            function,
            start,
            quantity,
        } => {
            let max = match function {
                FunctionCode::WriteMultipleCoils => MAX_WRITE_COILS,
                FunctionCode::WriteMultipleRegisters => MAX_WRITE_REGISTERS,
                FunctionCode::ReadCoils
                | FunctionCode::ReadDiscreteInputs
                | FunctionCode::ReadHoldingRegisters
                | FunctionCode::ReadInputRegisters
                | FunctionCode::WriteSingleRegister
                | FunctionCode::Unknown(_) => {
                    return Err(discriminator("response function code", *function));
                }
            };
            check_request(*start, *quantity, max)?;
            out.extend_from_slice(&function_and_words(*function, *start, *quantity));
        }
        ModbusPdu::Exception { function, code } => {
            if matches!(function, FunctionCode::Unknown(_)) {
                return Err(discriminator(FUNCTION_FIELD, *function));
            }
            let raw = code.as_u8();
            if matches!(code, ExceptionCode::Unknown(_)) {
                return Err(ParseError::InvalidDiscriminator {
                    field: "exception code",
                    value: u64::from(raw),
                });
            }
            out.extend_from_slice(&[function.exception_code(), raw]);
        }
        ModbusPdu::ReadCoils { .. }
        | ModbusPdu::ReadDiscreteInputs { .. }
        | ModbusPdu::ReadHoldingRegisters { .. }
        | ModbusPdu::ReadInputRegisters { .. }
        | ModbusPdu::WriteMultipleCoils { .. }
        | ModbusPdu::WriteSingleRegister { .. }
        | ModbusPdu::WriteMultipleRegisters { .. } => {
            return Err(discriminator("response function code", pdu.function()));
        }
    }
    writer.write_bytes(&out)
}
