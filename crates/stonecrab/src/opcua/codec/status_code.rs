//! `StatusCode`: a `UInt32` (OPC UA Part 6, 5.2.2.11).

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE};

/// The encoded size of a `StatusCode`.
const SIZE: usize = 4;

/// A `StatusCode`: the severity in the top two bits, the rest as the specification assigns.
/// The codec keeps the number as it is and decides nothing about its meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(in crate::opcua) struct StatusCode(pub(in crate::opcua) u32);

#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::unused_self,
    reason = "the external-type signature takes `&self` and the budget, which a built-in ignores"
)]
impl StatusCode {
    /// Reads a `StatusCode`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when fewer than 4 bytes remain.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        _depth: Depth,
    ) -> Result<Self, ParseError> {
        reader.read_u32(LITTLE).map(Self)
    }

    /// Writes the `StatusCode`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the writer's capacity is reached.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        _depth: Depth,
    ) -> Result<(), ParseError> {
        writer.write_u32(self.0, LITTLE)
    }

    /// The encoded size, always 4 bytes.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the external-type signature returns a Result"
    )]
    pub(in crate::opcua) const fn length_in_bytes(
        &self,
        _depth: Depth,
    ) -> Result<usize, ParseError> {
        Ok(SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{assert_cut_off, assert_vector};

    #[test]
    fn opcua_codec_status_code_is_a_little_endian_u32() {
        assert_vector(&StatusCode(0), &[0, 0, 0, 0]);
        // BadUnexpectedError.
        assert_vector(&StatusCode(0x8001_0000), &[0x00, 0x00, 0x01, 0x80]);
        assert_vector(&StatusCode(u32::MAX), &[0xFF; 4]);
        assert_cut_off(&StatusCode(0x8034_0000));
        assert_eq!(StatusCode::default(), StatusCode(0), "Good");
    }
}
