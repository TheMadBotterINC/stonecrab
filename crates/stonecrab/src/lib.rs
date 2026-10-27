#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use stonecrab_core::{
    Capabilities, ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, ItemError,
    Operation, PlcError, PlcValue, PlcValueType, ProtocolAddress, ProtocolStatus, ReadItem,
    ReadRequest, ReadRequestBuilder, ReadResponse, ReadResponseBuilder, ResourceLimits,
    ResourceLimitsBuilder, Result, Secret, SubscriptionMode, Tag, TagId, UnknownReason,
    WriteOutcome,
};
