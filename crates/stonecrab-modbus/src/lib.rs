#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::as_conversions)]

mod address;
mod batch;
mod pdu;

pub use address::{
    AddressConvention, ModbusAddress, ModbusAddressBuilder, ModbusArea, ModbusDataType, UnitId,
};
pub use batch::{BatchLimits, Batcher, Chunk, Member};
pub use pdu::{
    ExceptionCode, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS, ModbusPdu,
    decode_request, decode_response, encode_request, encode_response,
};
