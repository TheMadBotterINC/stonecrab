//! The Modbus protocol data unit (PDU) codec for the read function codes 01 to 04.
//!
//! A PDU is the function code and its data, without the MBAP header or any serial
//! framing (decision D5). The alpha transcribes the four read requests, their normal
//! responses and the exception response; write PDUs are not transcribed (decision D3), so
//! their function codes decode as [`FunctionCode::Unknown`] and every decode function
//! rejects them with a precise error. The types are written by hand from the Modbus
//! Application Protocol Specification V1.1b and the pinned `modbus.mspec` (decision D6);
//! the golden vectors in `fixtures/modbus/tcp/` are the contract a future generator would
//! have to meet.
//!
//! Every length on the wire is compared with its bound before anything is allocated: a
//! read quantity of 1 to 2000 bits or 1 to 125 registers, a response byte count of 1 to
//! 250 and a whole PDU of at most 253 bytes. All integers are big-endian, as the
//! specification requires; coil bits are packed least significant bit first.

mod code;
mod codec;
mod model;

pub use code::{ExceptionCode, FunctionCode};
pub use codec::{decode_request, decode_response, encode_request, encode_response};
pub use model::ModbusPdu;

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

/// Number of distinct wire addresses, 0 to 65535: a request's `start + quantity` may not
/// pass it.
const ADDRESS_SPACE: usize = 65_536;

/// Largest data length of a response: 2000 bits take 250 bytes and 125 registers take 250
/// bytes, so both fit the 253-byte PDU with the function code and the byte count.
const MAX_RESPONSE_DATA_BYTES: usize = 250;
