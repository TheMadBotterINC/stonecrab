#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub use stonecrab_core::{
    Capabilities, ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, Operation,
    PlcError, Result, Secret, SubscriptionMode, TagId,
};
