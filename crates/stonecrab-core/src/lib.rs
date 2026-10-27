#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

mod cancel;
mod capability;
mod clock;
mod deadline;
mod error;
mod future;
mod limits;
mod order;
mod read_response;
mod request;
mod response;
mod secret;
mod state;
mod tag;
mod value;

pub use cancel::CancellationToken;
pub use capability::{Capabilities, Operation, SubscriptionMode};
pub use clock::Clock;
pub use deadline::Deadline;
pub use error::{ConnectionState, Diagnostics, ErrorKind, FailureCategory, PlcError, Result};
pub use future::OpFuture;
pub use limits::{ResourceLimits, ResourceLimitsBuilder};
pub use order::{ByteOrder, WordOrder};
pub use read_response::{ReadResponse, ReadResponseBuilder};
pub use request::{ReadRequest, ReadRequestBuilder};
pub use response::{ItemError, ProtocolStatus, ReadItem, UnknownReason, WriteOutcome};
pub use secret::Secret;
pub use state::{StateCell, StateWatch};
pub use tag::{ProtocolAddress, Tag, TagId};
pub use value::{PlcValue, PlcValueType};
