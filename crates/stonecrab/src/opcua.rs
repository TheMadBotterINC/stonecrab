#![doc = include_str!("../docs/components/opcua.md")]

#[cfg(test)]
#[cfg(feature = "testkit")]
use crate as library;

mod address;
mod attributes;
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "S113 enables public typed browse")
)]
mod browse;
mod channel;
mod chunk;
// The types of the binary encoding (the built-ins and the recursive types) are written and
// used by generated service bodies; public connection/value mapping arrives in S91/S92.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "the connection and value layer consume the remaining codec entry points in S91/S92"
    )
)]
mod codec;
mod connection;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "S105/S106 consume the remaining private asymmetric and symmetric channel entry points"
    )
)]
mod crypto;
mod driver;
mod endpoint;
#[expect(
    dead_code,
    reason = "generated metadata and service constructors are consumed incrementally by S89–S92"
)]
mod generated;
mod identity;
mod lane;
#[cfg(test)]
#[cfg(feature = "testkit")]
mod lifecycle_tests;
#[cfg(test)]
#[cfg(feature = "testkit")]
#[path = "../tests/support/opcua_responder.rs"]
mod responder;
#[cfg(test)]
#[cfg(feature = "testkit")]
#[path = "../tests/support/opcua_secure.rs"]
mod secure_responder;
#[cfg(test)]
#[cfg(feature = "testkit")]
mod secure_tests;
mod service;
#[cfg(test)]
mod service_tests;
mod session;
mod wire;
mod write;
// The framing is written and tested here; the secure channel of slice S90 is its first caller.
// The fuzz entry point reaches the framer but not the constructors the channel will call.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "symmetric framer constructors are used only by fixtures/fuzzing; runtime negotiates buffers"
    )
)]
mod frame;
#[cfg(fuzzing)]
pub mod fuzzing;
mod options;
#[cfg(test)]
#[path = "../tests/support/opcua_pki.rs"]
mod pki;
mod read;
mod value;

pub use driver::OpcuaDriver;

pub use address::{OpcuaAddress, OpcuaAttribute, OpcuaDataType, OpcuaIdentifier};

#[cfg(test)]
#[cfg(feature = "testkit")]
#[path = "../tests/support/opcua_upstream.rs"]
mod upstream_vectors;
