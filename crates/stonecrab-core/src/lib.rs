#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

mod capability;
mod error;

pub use capability::{Capabilities, Operation, SubscriptionMode};
pub use error::{ConnectionState, Diagnostics, ErrorKind, FailureCategory, PlcError, Result};
