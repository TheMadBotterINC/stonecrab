#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use stonecrab_core::{
    Capabilities, ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, ItemError,
    Operation, PlcError, PlcValue, PlcValueType, ProtocolStatus, ReadItem, ResourceLimits,
    ResourceLimitsBuilder, Result, Secret, SubscriptionMode, TagId, UnknownReason, WriteOutcome,
};
