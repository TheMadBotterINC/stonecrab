//! Coil bit packing, least significant bit first.

use crate::ParseError;

/// Bit masks of one byte, least significant bit first.
const BIT_MASKS: [u8; 8] = [0x01, 0x02, 0x04, 0x08, 0x10, 0x20, 0x40, 0x80];

/// Number of bytes that carry `count` bits: `count` divided by eight, rounded up.
///
/// This is the Modbus coil byte count. It cannot overflow.
///
/// # Examples
/// ```
/// use stonecrab_codec::coil_byte_count;
/// assert_eq!(coil_byte_count(0), 0);
/// assert_eq!(coil_byte_count(8), 1);
/// assert_eq!(coil_byte_count(9), 2);
/// ```
#[must_use]
pub const fn coil_byte_count(count: usize) -> usize {
    count.div_ceil(8)
}

/// Packs `bits` into bytes, bit 0 of byte 0 first (Modbus coil packing).
///
/// Unused high bits of the last byte are zero. The result holds
/// [`coil_byte_count`]`(bits.len())` bytes.
///
/// # Examples
/// ```
/// use stonecrab_codec::pack_bits_lsb_first;
/// assert_eq!(pack_bits_lsb_first(&[true, false, true]), [0b0000_0101]);
/// assert_eq!(pack_bits_lsb_first(&[true; 9]), [0xFF, 0x01]);
/// ```
#[must_use]
pub fn pack_bits_lsb_first(bits: &[bool]) -> Vec<u8> {
    let mut packed = Vec::with_capacity(coil_byte_count(bits.len()));
    for chunk in bits.chunks(8) {
        let byte = chunk
            .iter()
            .zip(BIT_MASKS)
            .filter(|(bit, _)| **bit)
            .fold(0_u8, |acc, (_, mask)| acc | mask);
        packed.push(byte);
    }
    packed
}

/// Unpacks the first `count` bits of `bytes`, bit 0 of byte 0 first.
///
/// Padding bits after `count` and bytes beyond [`coil_byte_count`]`(count)` are ignored.
/// The output is allocated only after the input is proven to hold `count` bits, so its
/// size is bounded by eight times the input.
///
/// # Errors
/// [`ParseError::Truncated`] when `bytes` holds fewer than `count` bits.
///
/// # Examples
/// ```
/// use stonecrab_codec::{ParseError, unpack_bits_lsb_first};
/// assert_eq!(unpack_bits_lsb_first(&[0b0000_0101], 3)?, [true, false, true]);
/// assert!(unpack_bits_lsb_first(&[0xFF], 9).is_err());
/// # Ok::<(), ParseError>(())
/// ```
pub fn unpack_bits_lsb_first(bytes: &[u8], count: usize) -> Result<Vec<bool>, ParseError> {
    let needed = coil_byte_count(count);
    if bytes.len() < needed {
        return Err(ParseError::Truncated {
            needed,
            available: bytes.len(),
        });
    }
    let mut bits = Vec::with_capacity(count);
    bits.extend(
        bytes
            .iter()
            .flat_map(|byte| BIT_MASKS.iter().map(move |mask| byte & mask != 0))
            .take(count),
    );
    Ok(bits)
}
