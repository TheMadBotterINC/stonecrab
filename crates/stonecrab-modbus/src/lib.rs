#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::as_conversions)]

mod address;
mod batch;
mod connection;
mod decode;
mod framing;
mod pdu;
mod session;

pub use address::{
    AddressConvention, ModbusAddress, ModbusAddressBuilder, ModbusArea, ModbusDataType, UnitId,
};
pub use batch::{BatchLimits, Batcher, Chunk, Member};
pub use connection::{ConnectionDefaults, ModbusConnection};
pub use decode::TypedDecoder;
pub use framing::{MbapFramer, MbapHeader, ModbusAdu, TransactionId};
pub use pdu::{
    ExceptionCode, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS, ModbusPdu,
    decode_request, decode_response, encode_request, encode_response,
};
pub use session::{ModbusSession, SessionStats, exception_item_error};
