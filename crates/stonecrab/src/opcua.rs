#![doc = include_str!("../docs/components/opcua.md")]

mod address;
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
// Service bodies are decoded by ExtensionObject now; channel/session code will also use
// the separately generated request and response structs in S89–S92.
#[expect(
    dead_code,
    reason = "generated metadata and service constructors are consumed incrementally by S89–S92"
)]
mod generated;
#[cfg_attr(
    not(all(test, feature = "testkit")),
    expect(
        dead_code,
        reason = "the channel/session layers of S90/S91 own the lane"
    )
)]
mod lane;
#[cfg(test)]
mod service_tests;
// The connection string and its options are parsed here but nothing calls them until the
// driver arrives (slice S91, which removes these two expectations).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the driver of slice S91 is the first caller")
)]
mod driver;
// The framing is written and tested here; the secure channel of slice S90 is its first caller.
// The fuzz entry point reaches the framer but not the constructors the channel will call.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the channel of slice S90 is the first caller")
)]
mod frame;
#[cfg(fuzzing)]
pub mod fuzzing;
// The fuzz entry point reaches all of it, so under `fuzzing` nothing here is dead.
#[cfg_attr(
    not(any(test, fuzzing)),
    expect(dead_code, reason = "the driver of slice S91 is the first caller")
)]
mod options;

pub use address::{OpcuaAddress, OpcuaAttribute, OpcuaDataType, OpcuaIdentifier};
