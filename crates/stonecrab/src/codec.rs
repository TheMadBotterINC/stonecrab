#![doc = include_str!("../docs/components/codec.md")]

mod bits;
mod error;
#[cfg(any(feature = "opcua", all(test, feature = "modbus", feature = "testkit")))]
mod generated_bits;
mod reader;
mod writer;

use crate::ByteOrder;

pub use bits::{coil_byte_count, pack_bits_lsb_first, unpack_bits_lsb_first};
pub use error::ParseError;
#[cfg(any(feature = "opcua", all(test, feature = "modbus", feature = "testkit")))]
pub(crate) use generated_bits::{join_bits, split_bits};
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
