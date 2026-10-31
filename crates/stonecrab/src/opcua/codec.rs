//! The hand-written OPC UA built-in types of the binary encoding (OPC UA Part 6, 5.2.2):
//! String, `ByteString`, Guid, `DateTime`, `StatusCode`, `NodeId` in its six encodings,
//! `ExpandedNodeId`, `QualifiedName` and `LocalizedText`.
//!
//! Each type implements the external-type signature of the generator (ADR-0034, S79
//! addendum): `parse(reader, depth)`, `serialize(&self, writer, depth)` and
//! `length_in_bytes(&self, depth)`, each returning `Result<_, ParseError>`. The built-ins
//! have a fixed shape and never recurse, so none spends the [`Depth`] budget; the recursive
//! types of slice S86 do. Everything is little-endian and goes through `BoundedReader` and
//! `BoundedWriter`. A failed `parse` or `serialize` may have consumed or written a prefix, so
//! the caller drops the reader or writer.

mod date_time;
mod guid;
mod name;
mod node_id;
#[cfg(test)]
mod properties;
mod status_code;
mod string;
#[cfg(test)]
mod support;
#[cfg(test)]
#[cfg(feature = "testkit")]
mod vectors;

pub(super) use date_time::DateTime;
pub(super) use guid::Guid;
pub(super) use name::{LocalizedText, QualifiedName};
pub(super) use node_id::{ExpandedNodeId, Identifier, NodeId};
pub(super) use status_code::StatusCode;
pub(super) use string::{UaByteString, UaString};

use crate::ByteOrder;

/// Every multi-byte number of the OPC UA binary encoding is little-endian.
const LITTLE: ByteOrder = ByteOrder::LittleEndian;

/// The longest String or `ByteString` this codec reads or writes, in bytes.
///
/// A length is also bounded by the bytes that remain, so this only caps what a frame large
/// enough to hold the bytes may declare. 16 MiB is above the 4 MiB message size most servers
/// offer and far below `i32::MAX`.
pub(super) const MAX_STRING_BYTES: usize = 16 * 1024 * 1024;

/// The recursion budget that the generated and the hand-written types pass on by value.
///
/// The generator treats it as opaque (ADR-0019, S79 addendum). Only the recursive types spend
/// it: slice S86 adds the entry that takes one unit and fails with
/// `ParseError::RecursionLimit` at zero. A built-in has a fixed shape and ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Depth {
    /// Entries into recursive types still allowed.
    remaining: u8,
}

impl Depth {
    /// The number of recursive entries a message may nest (ADR-0034 decision 3): above the 29
    /// contexts of the deepest message upstream tests, far below the 2000 it refuses.
    pub(super) const LIMIT: u8 = 64;

    /// A full budget.
    pub(super) const fn new() -> Self {
        Self {
            remaining: Self::LIMIT,
        }
    }

    /// The entries still allowed.
    pub(super) const fn remaining(self) -> u8 {
        self.remaining
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcua_codec_depth_starts_full_and_is_copy() {
        let budget = Depth::new();
        let copy = budget;
        assert_eq!(budget.remaining(), Depth::LIMIT);
        assert_eq!(copy, budget, "the budget is passed by value");
        assert_eq!(Depth::LIMIT, 64, "ADR-0034 decision 3");
    }

    #[test]
    fn opcua_codec_string_maximum_is_16_mib() {
        assert_eq!(MAX_STRING_BYTES, 16_777_216, "ADR-0034, S85 addendum");
    }
}
