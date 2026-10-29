//! The Modbus protocol data unit (PDU) codec for the read function codes 01 to 04, the
//! write function codes 06, 15 and 16 and the file record function codes 20 and 21.
//!
//! A PDU is the function code and its data, without the MBAP header or any serial
//! framing (decision D5). The codec transcribes the four read requests, their normal
//! responses, the three write requests upstream emits (Write Multiple Coils for every coil
//! write, Write Single Register for a 2-byte holding-register payload and Write Multiple
//! Registers otherwise), their responses and the exception response. Read File Record (20)
//! and Write File Record (21) carry the extended registers: a request holds up to 35 read
//! sub-requests or up to 251 bytes of write sub-requests, each naming a file, a record
//! (0 to 9999) and a length, and a write is answered by an echo of the whole request. Write
//! Single Coil (05) is not emitted by upstream and stays [`FunctionCode::Unknown`]: every
//! decode function rejects it with a precise error. The types are
//! written by hand from the Modbus Application Protocol Specification V1.1b and the
//! pinned `modbus.mspec` (decision D6); the golden vectors in `fixtures/modbus/tcp/` are
//! the contract a future generator would have to meet.
//!
//! Every length on the wire is compared with its bound before anything is allocated: a
//! read quantity of 1 to 2000 bits or 1 to 125 registers, a write quantity of 1 to 1968
//! coils or 1 to 123 registers with a byte count that fits it, a response byte count of 1
//! to 250, a file record request byte count of 7 to 245 (a multiple of 7 for a read) or 9
//! to 251 (write), a file record read response of at most 245 data bytes and a whole PDU of
//! at most 253 bytes. All integers are big-endian, as the specification requires; coil
//! bits are packed least significant bit first.

mod code;
mod codec;
mod model;

pub use code::{ExceptionCode, FunctionCode};
pub use codec::{decode_request, decode_response, encode_request, encode_response};
pub use model::{FileRecord, FileRecordRead, ModbusPdu};

/// Largest PDU in bytes (V1.1b section 4.1: the 256-byte serial ADU less the address and
/// the CRC). Larger input is refused before it is parsed.
///
/// # Compatibility: docs/parity/modbus.md#MB-LIMIT-PDU
/// Upstream has no explicit PDU size check in the inventoried read path; the bound is
/// V1.1b section 4.1.
pub const MAX_PDU_BYTES: usize = 253;

/// Most bits one read request may ask for (function codes 01 and 02, V1.1b sections 6.1
/// and 6.2).
///
/// # Compatibility: docs/parity/modbus.md#MB-LIMIT-COILS
/// Matches the upstream configurable block bound `max-coils-per-request` (default 2000).
pub const MAX_READ_BITS: u16 = 2_000;

/// Most registers one read request may ask for (function codes 03 and 04, V1.1b sections
/// 6.3 and 6.4).
///
/// # Compatibility: docs/parity/modbus.md#MB-LIMIT-REGISTERS
/// Matches the upstream configurable block bound `max-registers-per-request` (default
/// 125).
pub const MAX_READ_REGISTERS: u16 = 125;

/// Most coils one Write Multiple Coils request may carry (function code 15, V1.1b section
/// 6.11: 1968 bits are 246 data bytes, the most that fit the 253-byte PDU).
const MAX_WRITE_COILS: u16 = 1_968;

/// Most registers one Write Multiple Registers request may carry (function code 16, V1.1b
/// section 6.12: 123 registers are 246 data bytes, the most that fit the 253-byte PDU).
const MAX_WRITE_REGISTERS: u16 = 123;

/// Most registers one Read File Record request may ask for, in all its sub-requests
/// together, when it is made for [`ModbusPdu::read_file_record_request`] (function code
/// 20, V1.1b section 6.14). The response data may not pass 245 bytes (0xF5) and every
/// sub-response costs 2 bytes beside its registers. A span of 120 registers is cut at the
/// 10000-record end of a file into at most two sub-requests, so it needs at most
/// `2 * 2 + 2 * 120 = 244` bytes; 121 registers would fit one file but not two.
///
/// # Compatibility: docs/parity/modbus.md#MB-LIMIT-REGISTERS
/// Upstream applies the 125 registers of `max-registers-per-request` to extended registers
/// too, so a full block of 125 asks for a 254-byte response that no device may send; the
/// bound here is the divergence recorded under `MB-ADDR-EXTENDED`.
pub const MAX_READ_EXTENDED_REGISTERS: u16 = 120;

/// The reference type of every file record sub-request (V1.1b sections 6.14 and 6.15).
const FILE_REFERENCE_TYPE: u8 = 6;

/// Records in one file: record numbers run from 0 to 9999 (0x270F).
const FILE_RECORDS: u16 = 10_000;

/// The highest record number of a file, 0x270F.
const MAX_RECORD_NUMBER: u16 = 9_999;

/// Bytes of one Read File Record sub-request: reference type, file, record and length.
const FILE_READ_ITEM_BYTES: usize = 7;

/// Bytes of a Write File Record sub-request before its registers: reference type, file,
/// record and length.
const FILE_WRITE_HEADER_BYTES: usize = 7;

/// Largest data length of a Read File Record request or response (V1.1b section 6.14,
/// 0xF5).
const MAX_FILE_READ_BYTES: usize = 245;

/// Largest data length of a Write File Record request or response (V1.1b section 6.15,
/// 0xFB: the 253-byte PDU less the function code and the length).
const MAX_FILE_WRITE_BYTES: usize = 251;

/// Smallest data length of a Write File Record request: one sub-request of one register.
const MIN_FILE_WRITE_BYTES: usize = FILE_WRITE_HEADER_BYTES + 2;

/// Number of distinct wire addresses, 0 to 65535: a request's `start + quantity` may not
/// pass it.
const ADDRESS_SPACE: usize = 65_536;

/// Largest data length of a response: 2000 bits take 250 bytes and 125 registers take 250
/// bytes, so both fit the 253-byte PDU with the function code and the byte count.
const MAX_RESPONSE_DATA_BYTES: usize = 250;
