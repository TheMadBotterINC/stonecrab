#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

mod capability;
mod deadline;
mod error;
mod secret;
mod tag;

pub use capability::{Capabilities, Operation, SubscriptionMode};
pub use deadline::Deadline;
pub use error::{ConnectionState, Diagnostics, ErrorKind, FailureCategory, PlcError, Result};
pub use secret::Secret;
pub use tag::TagId;
