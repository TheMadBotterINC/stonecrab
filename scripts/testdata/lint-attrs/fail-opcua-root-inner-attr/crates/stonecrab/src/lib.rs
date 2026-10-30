#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

#[cfg(feature = "modbus")]
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
#[cfg_attr(docsrs, doc(cfg(feature = "modbus")))]
pub mod modbus;
#[cfg(feature = "opcua")]
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
#[cfg_attr(docsrs, doc(cfg(feature = "opcua")))]
pub mod opcua;
