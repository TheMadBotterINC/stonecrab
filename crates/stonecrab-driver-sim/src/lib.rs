#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::indexing_slicing)]
#![deny(clippy::arithmetic_side_effects)]
#![deny(clippy::as_conversions)]

mod address;
mod clock;
mod device;

pub use address::{SimAddress, SimArea};
pub use clock::RealtimeClock;
pub use device::{SimDevice, SimTag};
