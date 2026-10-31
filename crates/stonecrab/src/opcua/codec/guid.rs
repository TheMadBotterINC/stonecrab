//! Guid: a `UInt32`, two `UInt16`s and eight bytes (OPC UA Part 6, 5.2.2.6).

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE};

/// The encoded size of a Guid.
const SIZE: usize = 16;

/// A 128-bit Guid in the groups of its text form (`data1-data2-data3-data4`).
///
/// The first three groups are little-endian on the wire and the last eight bytes are as
/// written, so the wire bytes of the value `72962B91-FA75-4AE6-8D28-B404DC7DAF63` begin
/// `91 2B 96 72 75 FA E6 4A`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(in crate::opcua) struct Guid {
    /// The first group, eight hexadecimal digits.
    pub(in crate::opcua) data1: u32,
    /// The second group, four digits.
    pub(in crate::opcua) data2: u16,
    /// The third group, four digits.
    pub(in crate::opcua) data3: u16,
    /// The last two groups, four and twelve digits, as eight bytes.
    pub(in crate::opcua) data4: [u8; 8],
}

#[expect(
    clippy::unused_self,
    reason = "the external-type signature takes `&self` and the budget, which a built-in ignores"
)]
impl Guid {
    /// Reads a Guid.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when fewer than 16 bytes remain.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        _depth: Depth,
    ) -> Result<Self, ParseError> {
        let data1 = reader.read_u32(LITTLE)?;
        let data2 = reader.read_u16(LITTLE)?;
        let data3 = reader.read_u16(LITTLE)?;
        let mut data4 = [0; 8];
        for byte in &mut data4 {
            *byte = reader.read_u8()?;
        }
        Ok(Self {
            data1,
            data2,
            data3,
            data4,
        })
    }

    /// Writes the Guid.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the writer's capacity is reached.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        _depth: Depth,
    ) -> Result<(), ParseError> {
        writer.write_u32(self.data1, LITTLE)?;
        writer.write_u16(self.data2, LITTLE)?;
        writer.write_u16(self.data3, LITTLE)?;
        writer.write_bytes(&self.data4)
    }

    /// The encoded size, always 16 bytes.
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
    fn opcua_codec_guid_first_three_groups_are_little_endian() {
        // 72962B91-FA75-4AE6-8D28-B404DC7DAF63, the worked example of Part 6.
        let guid = Guid {
            data1: 0x7296_2B91,
            data2: 0xFA75,
            data3: 0x4AE6,
            data4: [0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D, 0xAF, 0x63],
        };
        assert_vector(
            &guid,
            &[
                0x91, 0x2B, 0x96, 0x72, 0x75, 0xFA, 0xE6, 0x4A, 0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D,
                0xAF, 0x63,
            ],
        );
        assert_cut_off(&guid);
        assert_vector(&Guid::default(), &[0; 16]);
    }
}
