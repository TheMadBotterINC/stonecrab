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
#[cfg(feature = "transport")]
#[deny(clippy::indexing_slicing, clippy::as_conversions)]
pub mod transport;
#[allow(clippy::unwrap_used)]
pub mod sensors;
