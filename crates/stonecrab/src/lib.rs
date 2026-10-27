#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

pub use stonecrab_core::{
    AccessPolicy, AddressOptions, AddressOptionsBuilder, ByteOrder, CancellationToken,
    Capabilities, Clock, Connection, ConnectionConfig, ConnectionConfigBuilder, ConnectionState,
    ConnectionUri, Credentials, Deadline, Diagnostics, Driver, DriverRegistry, Endpoint, ErrorKind,
    FailureCategory, ItemError, OpFuture, Operation, PlcError, PlcValue, PlcValueType,
    ProtocolAddress, ProtocolStatus, ReadItem, ReadRequest, ReadRequestBuilder, ReadResponse,
    ReadResponseBuilder, ReconnectPolicy, ResourceLimits, ResourceLimitsBuilder, Result,
    RetryPolicy, Secret, StateCell, StateWatch, SubscriptionMode, Tag, TagId, Timeouts,
    UnknownReason, WordOrder, WriteItem, WriteOutcome, WriteRequest, WriteRequestBuilder,
    WriteResponse, WriteResponseBuilder, WriteResult,
};
