//! Encoding and decoding of the read, write and file record PDUs.

use crate::ByteOrder;
use crate::codec::{BoundedReader, BoundedWriter, ParseError, coil_byte_count};

use super::model::{check_length, coil_word, discriminator};
use super::{
    ADDRESS_SPACE, COIL_ON, ExceptionCode, FILE_READ_ITEM_BYTES, FILE_RECORDS, FILE_REFERENCE_TYPE,
    FILE_WRITE_HEADER_BYTES, FileRecord, FileRecordRead, FunctionCode, MAX_FILE_READ_BYTES,
    MAX_FILE_WRITE_BYTES, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS, MAX_RECORD_NUMBER,
    MAX_RESPONSE_DATA_BYTES, MAX_WRITE_COILS, MAX_WRITE_REGISTERS, MIN_FILE_WRITE_BYTES, ModbusPdu,
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

/// Checks one run of a file: `length` registers from `record`, at least one, starting at a
/// record number of the file (0 to 9999) and ending at or before its last record.
pub(super) fn check_file_span(record: u16, length: u16) -> Result<(), ParseError> {
    if length == 0 {
        return Err(ParseError::InvalidDiscriminator {
            field: "record length",
            value: 0,
        });
    }
    if record > MAX_RECORD_NUMBER {
        return Err(ParseError::OversizedLength {
            length: usize::from(record),
            max: usize::from(MAX_RECORD_NUMBER),
        });
    }
    let end = usize::from(record).saturating_add(usize::from(length));
    if end > usize::from(FILE_RECORDS) {
        return Err(ParseError::OversizedLength {
            length: end,
            max: usize::from(FILE_RECORDS),
        });
    }
    Ok(())
}

/// The (file, record, length) runs `quantity` registers from `record` of `file` are cut
/// into at the end of each file: the first run goes to record 9999 and each next run
/// starts at record 0 of the next file number.
pub(super) fn split_span(
    file: u16,
    record: u16,
    quantity: u16,
) -> Result<Vec<(u16, u16, u16)>, ParseError> {
    if quantity == 0 {
        return Err(ParseError::InvalidDiscriminator {
            field: "quantity",
            value: 0,
        });
    }
    if record > MAX_RECORD_NUMBER {
        return Err(ParseError::OversizedLength {
            length: usize::from(record),
            max: usize::from(MAX_RECORD_NUMBER),
        });
    }
    let mut runs = Vec::new();
    let (mut file, mut record, mut left) = (file, record, quantity);
    loop {
        let take = left.min(FILE_RECORDS.saturating_sub(record));
        runs.push((file, record, take));
        left = left.saturating_sub(take);
        if left == 0 {
            return Ok(runs);
        }
        file = file.checked_add(1).ok_or(ParseError::OversizedLength {
            length: usize::from(u16::MAX).saturating_add(1),
            max: usize::from(u16::MAX),
        })?;
        record = 0;
    }
}

/// Checks the sub-requests of a Read File Record request: 1 to 35 of them (the 7 to 245
/// request bytes), each a legal run, and an answer that fits the 245 data bytes of a
/// response, 2 bytes beside the registers of each.
pub(super) fn check_read_records(records: &[FileRecordRead]) -> Result<(), ParseError> {
    if records.is_empty() {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: 0,
        });
    }
    let request_bytes = records.len().saturating_mul(FILE_READ_ITEM_BYTES);
    if request_bytes > MAX_FILE_READ_BYTES {
        return Err(ParseError::OversizedLength {
            length: request_bytes,
            max: MAX_FILE_READ_BYTES,
        });
    }
    let mut answer_bytes = 0_usize;
    for record in records {
        check_file_span(record.record, record.length)?;
        answer_bytes = answer_bytes
            .saturating_add(2)
            .saturating_add(usize::from(record.length).saturating_mul(2));
    }
    if answer_bytes > MAX_FILE_READ_BYTES {
        return Err(ParseError::OversizedLength {
            length: answer_bytes,
            max: MAX_FILE_READ_BYTES,
        });
    }
    Ok(())
}

/// Checks the sub-requests of a Write File Record request or of its echo: at least one,
/// each a legal run, 251 data bytes at most together (7 bytes and 2 per register each).
pub(super) fn check_write_records(records: &[FileRecord]) -> Result<(), ParseError> {
    if records.is_empty() {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: 0,
        });
    }
    let mut data_bytes = 0_usize;
    for record in records {
        let length = u16::try_from(record.registers.len()).map_err(|_too_many| {
            ParseError::OversizedLength {
                length: record.registers.len(),
                max: usize::from(u16::MAX),
            }
        })?;
        check_file_span(record.record, length)?;
        data_bytes = data_bytes
            .saturating_add(FILE_WRITE_HEADER_BYTES)
            .saturating_add(usize::from(length).saturating_mul(2));
    }
    if data_bytes > MAX_FILE_WRITE_BYTES {
        return Err(ParseError::OversizedLength {
            length: data_bytes,
            max: MAX_FILE_WRITE_BYTES,
        });
    }
    Ok(())
}

/// A validated length as the single byte it is on the wire.
fn length_byte(len: usize) -> u8 {
    u8::try_from(len).unwrap_or(u8::MAX)
}

/// The bytes of a file record write request or of its echo after the function code: the
/// data length, then per sub-request the reference type, the file, the record, the length
/// and the registers.
fn write_records_body(records: &[FileRecord], out: &mut Vec<u8>) {
    let data_bytes = records.iter().fold(0_usize, |sum, record| {
        sum.saturating_add(FILE_WRITE_HEADER_BYTES)
            .saturating_add(record.registers.len().saturating_mul(2))
    });
    out.push(length_byte(data_bytes));
    for record in records {
        out.push(FILE_REFERENCE_TYPE);
        out.extend_from_slice(&record.file.to_be_bytes());
        out.extend_from_slice(&record.record.to_be_bytes());
        let length = u16::try_from(record.registers.len()).unwrap_or(u16::MAX);
        out.extend_from_slice(&length.to_be_bytes());
        out.extend(
            record
                .registers
                .iter()
                .flat_map(|register| register.to_be_bytes()),
        );
    }
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
/// address and the value; Write Single Coil is the address and `0xFF00` for on or `0x0000`
/// for off; Write Multiple Coils and Write Multiple Registers are the start
/// address, the quantity, the byte count and the data; Read File Record is the byte count
/// and a reference type, file, record and length per sub-request; Write File Record is
/// the byte count and a reference type, file, record, length and registers per
/// sub-request. The request is validated as a decoder would validate it (quantity 1 to
/// 2000 bits or 1 to 125 registers to read, 1 to 1968 coils or 1 to 123 registers to
/// write, `start + quantity` at most 65536, a coil write holding exactly `ceil(quantity /
/// 8)` bytes, a file record run of at least one register inside records 0 to 9999, a
/// read of at most 35 sub-requests whose answer fits 245 bytes, a write of at most 251
/// data bytes), so a request this function accepts is one [`decode_request`] accepts, except
/// Write Single Coil, which only a client sends and [`decode_request`] refuses. Nothing is
/// written when it fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is not a request, the quantity is 0, or a
/// file record request has no sub-request or a run of 0 registers; [`ParseError::OversizedLength`]
/// when the quantity or the end of the span is too large, a record number is over 9999, a
/// run passes the end of its file, the file record request is longer than its bound, or the
/// writer cannot hold the PDU; [`ParseError::Truncated`] or [`ParseError::TrailingBytes`]
/// when the data of a coil write is not `ceil(quantity / 8)` bytes.
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
/// (`MB-TCP-WRITE`); the write bounds are V1.1b sections 6.6, 6.11 and 6.12. The file
/// record bytes are those of upstream's `ModbusPDUReadFileRecordRequest` and
/// `ModbusPDUWriteFileRecordRequest` (`MB-ADDR-EXTENDED`), bounded by V1.1b sections 6.14
/// and 6.15.
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
        ModbusPdu::WriteSingleCoil { address, value } => {
            out.extend_from_slice(&function_and_words(function, *address, coil_word(*value)));
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
        ModbusPdu::ReadFileRecord { records } => {
            check_read_records(records)?;
            out.push(function.as_u8());
            out.push(length_byte(
                records.len().saturating_mul(FILE_READ_ITEM_BYTES),
            ));
            for record in records {
                out.push(FILE_REFERENCE_TYPE);
                out.extend_from_slice(&record.file.to_be_bytes());
                out.extend_from_slice(&record.record.to_be_bytes());
                out.extend_from_slice(&record.length.to_be_bytes());
            }
        }
        ModbusPdu::WriteFileRecord { records } => {
            check_write_records(records)?;
            out.push(function.as_u8());
            write_records_body(records, &mut out);
        }
        ModbusPdu::ReadBitsResponse { .. }
        | ModbusPdu::ReadRegistersResponse { .. }
        | ModbusPdu::WriteSingleRegisterResponse { .. }
        | ModbusPdu::WriteSingleCoilResponse { .. }
        | ModbusPdu::WriteMultipleResponse { .. }
        | ModbusPdu::ReadFileRecordResponse { .. }
        | ModbusPdu::WriteFileRecordResponse { .. }
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
/// read. `start + quantity` is at most 65536 for all of them. A Read File Record request
/// (20) is the byte count, 7 to 245 and a multiple of 7, and that many bytes of
/// sub-requests: the reference type 6, the file number, the record number (0 to 9999) and
/// the record length (at least 1, and the run ends inside the file), where the registers
/// answered to all of them must fit the 245 data bytes of a response. A Write File Record
/// request (21) is the byte count, 9 to 251, and sub-requests of the same header followed
/// by `2 * length` bytes of registers that fill the byte count exactly. Every other
/// function code, including Write Single Coil (05), is refused. The checks run in this
/// order: PDU size, function, truncation of the fixed fields, trailing bytes (reads and
/// function 06), quantity, span, byte count, truncation of the data, trailing bytes
/// (function 15 and 16); for function 20 and 21 the byte count, truncation, trailing
/// bytes and then the sub-requests in order (reference type, record length, record number,
/// end of the run, and for a read the size of the answer).
///
/// # Errors
/// [`ParseError::OversizedLength`] for a PDU over 253 bytes, a quantity over its maximum, a
/// span past the address space, a file record byte count over its bound, a record number
/// over 9999, a run past the last record of its file or a read that asks for more than a
/// response can hold; [`ParseError::InvalidDiscriminator`] for an unsupported function
/// code (field `function code`), a quantity of 0, a write byte count that does not fit the
/// quantity (field `byte count`), an odd register byte count (field `register byte
/// count`), a file record byte count of 0, below 9 (write) or not a multiple of 7 (read),
/// a reference type other than 6 (field `reference type`) or a record length of 0 (field
/// `record length`); [`ParseError::Truncated`] and [`ParseError::TrailingBytes`] when the
/// input is not exactly the length its fields imply.
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
///
/// // V1.1b section 6.14: two groups of records, file 4 records 1 and 2, file 3 records 9
/// // and 10.
/// let bytes = [
///     0x14, 0x0E, 0x06, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x06, 0x00, 0x03, 0x00, 0x09,
///     0x00, 0x02,
/// ];
/// let request = decode_request(&mut BoundedReader::new(&bytes))?;
/// assert_eq!(request.function().as_u8(), 0x14);
/// # Ok::<(), stonecrab::codec::ParseError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-READ-COIL
/// The layouts are upstream's four read request PDUs (`MB-TCP-READ-COIL`,
/// `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-TCP-READ-IR`) and its three write request PDUs
/// (`MB-TCP-WRITE`), with the bounds `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`,
/// `MB-LIMIT-SPAN` and `MB-LIMIT-PDU`. Function 05 is not decoded as a request
/// (`MB-QUIRK-C`, `MB-CFG-SINGLE-COIL`). The file record layouts are upstream's
/// `ModbusPDUReadFileRecordRequest` and `ModbusPDUWriteFileRecordRequest`
/// (`MB-ADDR-EXTENDED`); the bounds on top of upstream's are V1.1b sections 6.14 and 6.15.
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
        FunctionCode::ReadFileRecord => return decode_read_file_request(reader),
        FunctionCode::WriteFileRecord => {
            return decode_file_records(reader)
                .map(|records| ModbusPdu::WriteFileRecord { records });
        }
        // A client sends function 05 and never parses it as a request.
        FunctionCode::WriteSingleCoil | FunctionCode::Unknown(_) => {
            return Err(discriminator(FUNCTION_FIELD, function));
        }
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

/// Decodes the fields after the function code of a Read File Record request. The byte count
/// is proved to be whole sub-requests of 7 bytes, at most 245, before the data is read.
fn decode_read_file_request(reader: &mut BoundedReader<'_>) -> Result<ModbusPdu, ParseError> {
    let count = usize::from(reader.read_u8()?);
    if count > MAX_FILE_READ_BYTES {
        return Err(ParseError::OversizedLength {
            length: count,
            max: MAX_FILE_READ_BYTES,
        });
    }
    if count == 0 || !count.is_multiple_of(FILE_READ_ITEM_BYTES) {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: u64::try_from(count).unwrap_or(u64::MAX),
        });
    }
    let data = reader.read_bytes(count, MAX_FILE_READ_BYTES)?;
    reader.expect_eof()?;
    let mut records = Vec::new();
    for item in data.as_chunks::<FILE_READ_ITEM_BYTES>().0 {
        let [
            kind,
            file_hi,
            file_lo,
            record_hi,
            record_lo,
            length_hi,
            length_lo,
        ] = *item;
        check_reference_type(kind)?;
        records.push(FileRecordRead {
            file: u16::from_be_bytes([file_hi, file_lo]),
            record: u16::from_be_bytes([record_hi, record_lo]),
            length: u16::from_be_bytes([length_hi, length_lo]),
        });
    }
    check_read_records(&records)?;
    Ok(ModbusPdu::ReadFileRecord { records })
}

/// Refuses a reference type other than 6.
fn check_reference_type(kind: u8) -> Result<(), ParseError> {
    if kind == FILE_REFERENCE_TYPE {
        Ok(())
    } else {
        Err(ParseError::InvalidDiscriminator {
            field: "reference type",
            value: u64::from(kind),
        })
    }
}

/// Decodes the fields after the function code of a Write File Record request, or of its
/// echo, which has the same layout: the data length (9 to 251), proved before the data is
/// read, then sub-requests of a header and as many registers as the header says.
fn decode_file_records(reader: &mut BoundedReader<'_>) -> Result<Vec<FileRecord>, ParseError> {
    let count = usize::from(reader.read_u8()?);
    if count > MAX_FILE_WRITE_BYTES {
        return Err(ParseError::OversizedLength {
            length: count,
            max: MAX_FILE_WRITE_BYTES,
        });
    }
    if count < MIN_FILE_WRITE_BYTES {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: u64::try_from(count).unwrap_or(u64::MAX),
        });
    }
    let data = reader.read_bytes(count, MAX_FILE_WRITE_BYTES)?;
    reader.expect_eof()?;
    let mut items = BoundedReader::new(data);
    let mut records = Vec::new();
    while items.remaining() > 0 {
        check_reference_type(items.read_u8()?)?;
        let file = items.read_u16(ByteOrder::BigEndian)?;
        let record = items.read_u16(ByteOrder::BigEndian)?;
        let length = items.read_u16(ByteOrder::BigEndian)?;
        check_file_span(record, length)?;
        let registers = items
            .read_bytes(usize::from(length).saturating_mul(2), MAX_FILE_WRITE_BYTES)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        records.push(FileRecord {
            file,
            record,
            registers,
        });
    }
    Ok(records)
}

/// Decodes the fields after the function code of a Read File Record response: the data
/// length (1 to 245), then per sub-response a length byte (odd, 3 or more: the reference
/// type and whole registers), the reference type and the registers.
fn decode_read_file_response(reader: &mut BoundedReader<'_>) -> Result<ModbusPdu, ParseError> {
    let count = usize::from(reader.read_u8()?);
    if count == 0 {
        return Err(ParseError::InvalidDiscriminator {
            field: "byte count",
            value: 0,
        });
    }
    let data = reader.read_bytes(count, MAX_FILE_READ_BYTES)?;
    reader.expect_eof()?;
    let mut items = BoundedReader::new(data);
    let mut records = Vec::new();
    while items.remaining() > 0 {
        let length = items.read_u8()?;
        if length < 3 || length.is_multiple_of(2) {
            return Err(ParseError::InvalidDiscriminator {
                field: "file response length",
                value: u64::from(length),
            });
        }
        check_reference_type(items.read_u8()?)?;
        let registers = items
            .read_bytes(usize::from(length).saturating_sub(1), MAX_FILE_READ_BYTES)?
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_be_bytes(*pair))
            .collect();
        records.push(registers);
    }
    Ok(ModbusPdu::ReadFileRecordResponse { records })
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
/// address and value for function code 06, the address and `0xFF00` or `0x0000` for function
/// code 05, the start address and quantity (1..=1968 coils or 1..=123 registers, within the
/// address space) for 15 and 16, and the whole request for function code 21. A Read File
/// Record response (20) is the data length (1 to 245)
/// and per sub-response a length byte (odd, 3 or more), the reference type 6 and whole
/// registers. An exception response is
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
///   `expected` but not a supported code, `coil value` for an echoed coil state other than
///   `0xFF00` or `0x0000`, `byte count` for a count of 0 or a Write File
///   Record count below 9, `register byte count` for an odd count, `quantity` for an echoed
///   quantity of 0, `file response length` for a Read File Record sub-response that is
///   even or below 3, `reference type` for a type other than 6, `record length` for a
///   record length of 0 and `exception code` for a code outside the ten defined ones.
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
/// `MB-EXC-0B`; refusing any other code is Stonecrab's reading of `MB-EXC-UNKNOWN`. The
/// file record responses are upstream's `ModbusPDUReadFileRecordResponse` and
/// `ModbusPDUWriteFileRecordResponse` (`MB-ADDR-EXTENDED`), except that the echo of a write
/// carries `2 * length` bytes of registers per sub-request as the specification says, where
/// the upstream mspec reads `length` bytes.
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
        FunctionCode::WriteSingleCoil if !exception => {
            let address = reader.read_u16(ByteOrder::BigEndian)?;
            let word = reader.read_u16(ByteOrder::BigEndian)?;
            reader.expect_eof()?;
            let value = match word {
                COIL_ON => true,
                0 => false,
                other => {
                    return Err(ParseError::InvalidDiscriminator {
                        field: "coil value",
                        value: u64::from(other),
                    });
                }
            };
            Ok(ModbusPdu::WriteSingleCoilResponse { address, value })
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
        FunctionCode::ReadFileRecord if !exception => decode_read_file_response(reader),
        FunctionCode::WriteFileRecord if !exception => decode_file_records(reader)
            .map(|records| ModbusPdu::WriteFileRecordResponse { records }),
        FunctionCode::ReadCoils
        | FunctionCode::ReadDiscreteInputs
        | FunctionCode::ReadHoldingRegisters
        | FunctionCode::ReadInputRegisters
        | FunctionCode::WriteSingleRegister
        | FunctionCode::WriteSingleCoil
        | FunctionCode::WriteMultipleCoils
        | FunctionCode::WriteMultipleRegisters
        | FunctionCode::ReadFileRecord
        | FunctionCode::WriteFileRecord => decode_exception(reader, function),
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
/// file record read of 1 to 245 data bytes with a register in each sub-response, a file
/// record echo that a request could be, a function that matches the variant, one of the
/// ten defined exception codes), so a value this function accepts decodes back to itself. It is meant for tests and for the
/// responder of the contract suite. Nothing is written when it fails.
///
/// # Errors
/// [`ParseError::InvalidDiscriminator`] when `pdu` is a request, when the function does not
/// belong to the variant (or is not a supported function, for an exception), when the data
/// is empty, when the echoed quantity is 0, when a file record response has no
/// sub-response or one of no register, or when the exception code is
/// `ExceptionCode::Unknown`; [`ParseError::OversizedLength`] when the data is longer than
/// 250 bytes (245 for a file record read), the echoed quantity or span is too large, a
/// file record run leaves its file or the writer cannot hold the PDU.
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
        ModbusPdu::WriteSingleCoilResponse { address, value } => {
            out.extend_from_slice(&function_and_words(
                FunctionCode::WriteSingleCoil,
                *address,
                coil_word(*value),
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
                | FunctionCode::WriteSingleCoil
                | FunctionCode::ReadFileRecord
                | FunctionCode::WriteFileRecord
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
        ModbusPdu::ReadFileRecordResponse { records } => {
            if records.is_empty() {
                return Err(ParseError::InvalidDiscriminator {
                    field: "byte count",
                    value: 0,
                });
            }
            let mut data_bytes = 0_usize;
            for registers in records {
                if registers.is_empty() {
                    return Err(ParseError::InvalidDiscriminator {
                        field: "record length",
                        value: 0,
                    });
                }
                data_bytes = data_bytes
                    .saturating_add(2)
                    .saturating_add(registers.len().saturating_mul(2));
            }
            if data_bytes > MAX_FILE_READ_BYTES {
                return Err(ParseError::OversizedLength {
                    length: data_bytes,
                    max: MAX_FILE_READ_BYTES,
                });
            }
            out.push(FunctionCode::ReadFileRecord.as_u8());
            out.push(length_byte(data_bytes));
            for registers in records {
                out.push(length_byte(
                    registers.len().saturating_mul(2).saturating_add(1),
                ));
                out.push(FILE_REFERENCE_TYPE);
                out.extend(registers.iter().flat_map(|register| register.to_be_bytes()));
            }
        }
        ModbusPdu::WriteFileRecordResponse { records } => {
            check_write_records(records)?;
            out.push(FunctionCode::WriteFileRecord.as_u8());
            write_records_body(records, &mut out);
        }
        ModbusPdu::ReadCoils { .. }
        | ModbusPdu::ReadDiscreteInputs { .. }
        | ModbusPdu::ReadHoldingRegisters { .. }
        | ModbusPdu::ReadInputRegisters { .. }
        | ModbusPdu::WriteMultipleCoils { .. }
        | ModbusPdu::WriteSingleRegister { .. }
        | ModbusPdu::WriteSingleCoil { .. }
        | ModbusPdu::WriteMultipleRegisters { .. }
        | ModbusPdu::ReadFileRecord { .. }
        | ModbusPdu::WriteFileRecord { .. } => {
            return Err(discriminator("response function code", pdu.function()));
        }
    }
    writer.write_bytes(&out)
}
