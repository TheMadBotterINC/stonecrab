#![doc = include_str!("../docs/components/codec.md")]

mod bits;
mod error;
mod reader;
mod writer;

use crate::ByteOrder;

pub use bits::{coil_byte_count, pack_bits_lsb_first, unpack_bits_lsb_first};
pub use error::ParseError;
pub use reader::BoundedReader;
pub use writer::BoundedWriter;

/// Whether `order` lays the bytes of a word out least significant byte first.
///
/// `ByteOrder` is exhaustive (ADR-0006 declares its two-variant shape final), so this
/// test is total: the only other value is `BigEndian`.
const fn is_little(order: ByteOrder) -> bool {
    match order {
        ByteOrder::LittleEndian => true,
        ByteOrder::BigEndian => false,
    }
}
