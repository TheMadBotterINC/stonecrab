#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::as_conversions)]

mod address;
mod batch;

pub use address::{
    AddressConvention, ModbusAddress, ModbusAddressBuilder, ModbusArea, ModbusDataType, UnitId,
};
pub use batch::{BatchLimits, Batcher, Chunk, Member};
