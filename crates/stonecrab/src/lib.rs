#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

pub use stonecrab_core::{
    ByteOrder, CancellationToken, Capabilities, Clock, ConnectionState, Deadline, Diagnostics,
    ErrorKind, FailureCategory, ItemError, OpFuture, Operation, PlcError, PlcValue, PlcValueType,
    ProtocolAddress, ProtocolStatus, ReadItem, ReadRequest, ReadRequestBuilder, ReadResponse,
    ReadResponseBuilder, ResourceLimits, ResourceLimitsBuilder, Result, Secret, StateCell,
    StateWatch, SubscriptionMode, Tag, TagId, UnknownReason, WordOrder, WriteOutcome,
};
