#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

// The component documents of docs/components/ are included by the modules they describe.
// The core contracts live at the crate root, so their document is compiled here, for its
// code blocks only (they run as doctests; nothing is rendered).
#[cfg(doctest)]
#[doc = include_str!("../docs/components/core.md")]
struct CoreComponentDoc;

mod address_options;
mod cancel;
mod capability;
mod clock;
mod config;
mod connection;
mod connection_cache;
mod deadline;
mod driver;
mod entity;
mod entity_manager;
mod error;
mod event_pump;
mod future;
mod limits;
mod order;
// The polling producer is shared by the drivers that subscribe by polling (ADR-0020,
// addendum S55); without one of them it has no caller.
#[cfg(any(feature = "sim", feature = "modbus"))]
mod polling;
mod read_response;
mod registry;
mod request;
mod response;
mod secret;
mod state;
mod subscribe_request;
mod subscription;
mod tag;
mod uri;
mod value;
mod write_request;
mod write_response;

#[cfg(feature = "boards")]
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
#[cfg_attr(docsrs, doc(cfg(feature = "boards")))]
pub mod boards;
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
pub mod codec;
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
#[cfg(feature = "sensors")]
#[deny(clippy::indexing_slicing)]
#[cfg_attr(docsrs, doc(cfg(feature = "sensors")))]
pub mod sensors;
#[cfg(feature = "sim")]
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
#[cfg_attr(docsrs, doc(cfg(feature = "sim")))]
pub mod sim;
#[cfg(feature = "testkit")]
#[cfg_attr(docsrs, doc(cfg(feature = "testkit")))]
pub mod testkit;
#[cfg(feature = "transport")]
#[deny(clippy::indexing_slicing, clippy::as_conversions)]
#[cfg_attr(docsrs, doc(cfg(feature = "transport")))]
pub mod transport;

pub use address_options::{AddressOptions, AddressOptionsBuilder};
pub use cancel::CancellationToken;
pub use capability::{Capabilities, Operation, SubscriptionMode};
pub use clock::Clock;
pub use config::{
    AccessPolicy, ConnectionConfig, ConnectionConfigBuilder, Credentials, DriverOptions, Endpoint,
    ReconnectPolicy, RetryPolicy, Timeouts,
};
pub use connection::Connection;
pub use connection_cache::{ConnectionCache, ConnectionCacheBuilder, LeasedConnection};
pub use deadline::Deadline;
pub use driver::Driver;
pub use entity::{EntityField, FieldValue, PlcEntity};
pub use entity_manager::{ConnectedEntity, EntityManager};
pub use error::{ConnectionState, Diagnostics, ErrorKind, FailureCategory, PlcError, Result};
pub use event_pump::{BatchEvent, EventPump, TagBatch};
pub use future::OpFuture;
pub use limits::{ResourceLimits, ResourceLimitsBuilder};
pub use order::{ByteOrder, WordOrder};
pub use read_response::{ReadResponse, ReadResponseBuilder};
pub use registry::DriverRegistry;
pub use request::{ReadRequest, ReadRequestBuilder};
pub use response::{ItemError, ProtocolStatus, ReadItem, UnknownReason, WriteOutcome};
pub use secret::Secret;
pub use state::{StateCell, StateWatch};
pub use subscribe_request::{
    QueuePolicy, SubscribeRequest, SubscribeRequestBuilder, SubscriptionKind,
};
pub use subscription::{LossCounters, Subscription, SubscriptionSender};
pub use tag::{ProtocolAddress, Tag, TagId};
pub use uri::ConnectionUri;
pub use value::{PlcDate, PlcDateTime, PlcSignedDuration, PlcTimeOfDay, PlcValue, PlcValueType};
pub use write_request::{WriteItem, WriteRequest, WriteRequestBuilder};
pub use write_response::{WriteResponse, WriteResponseBuilder, WriteResult};
