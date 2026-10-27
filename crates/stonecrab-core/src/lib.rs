#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

mod capability;
mod deadline;
mod error;
mod limits;
mod read_response;
mod request;
mod response;
mod secret;
mod tag;
mod value;

pub use capability::{Capabilities, Operation, SubscriptionMode};
pub use deadline::Deadline;
pub use error::{ConnectionState, Diagnostics, ErrorKind, FailureCategory, PlcError, Result};
pub use limits::{ResourceLimits, ResourceLimitsBuilder};
pub use read_response::{ReadResponse, ReadResponseBuilder};
pub use request::{ReadRequest, ReadRequestBuilder};
pub use response::{ItemError, ProtocolStatus, ReadItem, UnknownReason, WriteOutcome};
pub use secret::Secret;
pub use tag::{ProtocolAddress, Tag, TagId};
pub use value::{PlcValue, PlcValueType};
