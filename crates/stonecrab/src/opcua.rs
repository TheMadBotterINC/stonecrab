#![doc = include_str!("../docs/components/opcua.md")]

mod address;
// The built-in types of the binary encoding are written and tested here, but nothing calls
// them until the recursive types (slice S86) and the generated service layer (slice S88).
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        unused_imports,
        reason = "the recursive types of slice S86 are the first callers"
    )
)]
mod codec;
// The connection string and its options are parsed here but nothing calls them until the
// driver arrives (slice S91, which removes these two expectations).
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the driver of slice S91 is the first caller")
)]
mod driver;
#[cfg(fuzzing)]
pub mod fuzzing;
// The fuzz entry point reaches all of it, so under `fuzzing` nothing here is dead.
#[cfg_attr(
    not(any(test, fuzzing)),
    expect(dead_code, reason = "the driver of slice S91 is the first caller")
)]
mod options;

pub use address::{OpcuaAddress, OpcuaAttribute, OpcuaDataType, OpcuaIdentifier};
