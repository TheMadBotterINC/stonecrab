//! Encoding and decoding of the read PDUs.

use stonecrab_codec::{BoundedReader, BoundedWriter, ParseError};
use stonecrab_core::ByteOrder;

use super::model::discriminator;
use super::{
    ADDRESS_SPACE, ExceptionCode, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS,
    MAX_RESPONSE_DATA_BYTES, ModbusPdu,
};

/// Builds a request variant from its `(start, quantity)`.
type RequestBuilder = fn(u16, u16) -> ModbusPdu;

/// The field name of a function code that is not the expected one or not a read function.
const FUNCTION_FIELD: &str = "function code";

/// Checks the quantity of a read request against `max` and the end of the address space.
fn check_request(start: u16, quantity: u16, max: u16) -> Result<(), ParseError> {
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

/// Encodes a read request: the function code, the wire address and the quantity.
///
/// The request is validated as a decoder would validate it (quantity 1 to 2000 bits or 1
/// to 125 registers, `start + quantity` at most 65536), so a request this function accepts
/// is one [`decode_request`] accepts. Nothing is written when it fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is not one of the four request
/// variants or the quantity is 0; [`ParseError::OversizedLength`] when the quantity or the
/// end of the span is too large, or the writer cannot hold the five bytes.
///
/// # Examples
/// ```
/// use stonecrab_codec::BoundedWriter;
/// use stonecrab_modbus::{ModbusPdu, encode_request};
///
/// let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
/// let mut writer = BoundedWriter::new(253);
/// encode_request(&request, &mut writer)?;
/// assert_eq!(writer.as_slice(), [0x03, 0x00, 0x6B, 0x00, 0x03]);
/// # Ok::<(), stonecrab_codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The bytes are those of upstream's `ModbusPDUReadHoldingRegistersRequest` and its three
/// siblings (`MB-TCP-READ-COIL`, `MB-TCP-READ-DI`, `MB-TCP-READ-IR`); the quantity bounds
/// are `MB-LIMIT-COILS` and `MB-LIMIT-REGISTERS` and the span bound is `MB-LIMIT-SPAN`.
pub fn encode_request(pdu: &ModbusPdu, writer: &mut BoundedWriter) -> Result<(), ParseError> {
    let (start, quantity, max) = match pdu {
        ModbusPdu::ReadCoils { start, quantity }
        | ModbusPdu::ReadDiscreteInputs { start, quantity } => (*start, *quantity, MAX_READ_BITS),
        ModbusPdu::ReadHoldingRegisters { start, quantity }
        | ModbusPdu::ReadInputRegisters { start, quantity } => {
            (*start, *quantity, MAX_READ_REGISTERS)
        }
        ModbusPdu::ReadBitsResponse { .. }
        | ModbusPdu::ReadRegistersResponse { .. }
        | ModbusPdu::Exception { .. } => {
            return Err(discriminator("request function code", pdu.function()));
        }
    };
    check_request(start, quantity, max)?;
    let [start_hi, start_lo] = start.to_be_bytes();
    let [quantity_hi, quantity_lo] = quantity.to_be_bytes();
    writer.write_bytes(&[
        pdu.function().as_u8(),
        start_hi,
        start_lo,
        quantity_hi,
        quantity_lo,
    ])
}

/// Decodes a read request, the form a responder or a fuzz target sees.
///
/// The PDU must be exactly the function code, the start address and the quantity, with the
/// quantity in 1..=2000 for function codes 01 and 02 or 1..=125 for 03 and 04 and
/// `start + quantity` at most 65536. Every other function code, including the write codes,
/// is refused. The checks run in this order: PDU size, function, truncation, trailing
/// bytes, quantity, span.
///
/// # Errors
/// [`ParseError::OversizedLength`] for a PDU over 253 bytes, a quantity over its maximum or
/// a span past the address space; [`ParseError::InvalidDiscriminator`] for an unsupported
/// function code (field `function code`) or a quantity of 0; [`ParseError::Truncated`] and
/// [`ParseError::TrailingBytes`] when the input is not exactly five bytes.
///
/// # Examples
/// ```
/// use stonecrab_codec::BoundedReader;
/// use stonecrab_modbus::{ModbusPdu, decode_request};
///
/// let bytes = [0x01, 0x00, 0x13, 0x00, 0x13];
/// let request = decode_request(&mut BoundedReader::new(&bytes))?;
/// assert_eq!(request, ModbusPdu::ReadCoils { start: 19, quantity: 19 });
/// # Ok::<(), stonecrab_codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-COIL
/// The layouts are upstream's four read request PDUs (`MB-TCP-READ-COIL`,
/// `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-TCP-READ-IR`) and the bounds `MB-LIMIT-COILS`,
/// `MB-LIMIT-REGISTERS`, `MB-LIMIT-SPAN` and `MB-LIMIT-PDU`.
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
        FunctionCode::Unknown(_) => return Err(discriminator(FUNCTION_FIELD, function)),
    };
    let start = reader.read_u16(ByteOrder::BigEndian)?;
    let quantity = reader.read_u16(ByteOrder::BigEndian)?;
    reader.expect_eof()?;
    check_request(start, quantity, max)?;
    Ok(build(start, quantity))
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
/// A normal response is the function code, a byte count and that many data bytes: 1 to 250
/// bytes of packed bits for function codes 01 and 02, or a whole number of registers (an
/// even byte count of 2 to 250) for 03 and 04. An exception response is the function code
/// with the high bit set and one of the ten exception codes of V1.1b section 7. The input
/// must hold exactly one PDU. The checks run in this order: PDU size (at most 253 bytes),
/// function, byte count (zero, over 250, odd for registers), truncation, trailing bytes;
/// every length is compared with its bound before the result is allocated.
///
/// The response does not carry the request's quantity, so a bit response is accepted
/// whatever its byte count within 1..=250. Pass the result to
/// [`ModbusPdu::check_reply_to`] with the request to prove the byte count against the
/// quantity.
///
/// An exception code outside the ten defined values (0, 9, 12 and above) is refused rather
/// than carried: a reply the codec cannot name is not trusted to be an exception at all.
///
/// # Errors
/// - [`ParseError::OversizedLength`]: a PDU over 253 bytes or a byte count over 250.
/// - [`ParseError::InvalidDiscriminator`]: field `function code` when the function (with or
///   without the high bit) is not `expected`, `unsupported function code` when it is
///   `expected` but not one of 01 to 04 (the write and file record codes), `byte count` for
///   a count of 0, `register byte count` for an odd count and `exception code` for a code
///   outside the ten defined ones.
/// - [`ParseError::Truncated`]: the input ends before the function code, the byte count,
///   the exception code or the declared data.
/// - [`ParseError::TrailingBytes`]: bytes remain after the PDU.
///
/// # Examples
/// ```
/// use stonecrab_codec::BoundedReader;
/// use stonecrab_modbus::{FunctionCode, ModbusPdu, decode_response};
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
/// # Ok::<(), stonecrab_codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-HR
/// The layouts are the responses of upstream's four read PDUs (`MB-TCP-READ-COIL`,
/// `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-TCP-READ-IR`) and its error PDU, with the
/// bounds `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS` and `MB-LIMIT-PDU`. The ten exception
/// codes are `MB-EXC-01` to `MB-EXC-0B`; refusing any other code is Stonecrab's reading of
/// `MB-EXC-UNKNOWN`.
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
        FunctionCode::ReadCoils
        | FunctionCode::ReadDiscreteInputs
        | FunctionCode::ReadHoldingRegisters
        | FunctionCode::ReadInputRegisters => decode_exception(reader, function),
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

/// Encodes a response PDU: a normal read response or an exception response.
///
/// The response is validated as [`decode_response`] validates it (1 to 250 bytes of bits,
/// 1 to 125 registers, a function that matches the variant, one of the ten defined
/// exception codes), so a value this function accepts decodes back to itself. It is meant
/// for tests and for the responder of the contract suite. Nothing is written when it
/// fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is a request, when the function does not
/// belong to the variant (or is not one of the four read functions, for an exception), when the data is
/// empty or when the exception code is `ExceptionCode::Unknown`;
/// [`ParseError::OversizedLength`] when the data is longer than 250 bytes or the writer
/// cannot hold the PDU.
///
/// # Examples
/// ```
/// use stonecrab_codec::BoundedWriter;
/// use stonecrab_modbus::{ExceptionCode, FunctionCode, ModbusPdu, encode_response};
///
/// let refusal = ModbusPdu::Exception {
///     function: FunctionCode::ReadHoldingRegisters,
///     code: ExceptionCode::IllegalDataAddress,
/// };
/// let mut writer = BoundedWriter::new(253);
/// encode_response(&refusal, &mut writer)?;
/// assert_eq!(writer.as_slice(), [0x83, 0x02]);
/// # Ok::<(), stonecrab_codec::ParseError>(())
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
        | ModbusPdu::ReadInputRegisters { .. } => {
            return Err(discriminator("response function code", pdu.function()));
        }
    }
    writer.write_bytes(&out)
}
