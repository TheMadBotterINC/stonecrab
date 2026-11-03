//! Shared most-significant-bit field runs used by generated protocol codecs.

use crate::codec::ParseError;

/// Bits in the one-byte runs the generator emits.
const RUN_BITS: u32 = 8;

/// The right shift that brings the field at `offset .. offset + width` (counted from the
/// most significant bit) to the low bits, or `Overflow` when it does not lie in the byte.
fn shift(offset: u32, width: u32) -> Result<u32, ParseError> {
    RUN_BITS
        .checked_sub(offset)
        .and_then(|rest| rest.checked_sub(width))
        .ok_or(ParseError::Overflow)
}

/// A mask of the `width` low bits, `width` from 1 to 8.
fn mask(width: u32) -> Result<u8, ParseError> {
    RUN_BITS
        .checked_sub(width)
        .filter(|_| width > 0)
        .and_then(|unused| u8::MAX.checked_shr(unused))
        .ok_or(ParseError::Overflow)
}

/// The field at bits `offset .. offset + width` of `run`, counted from the most
/// significant bit, as the low bits of the result.
///
/// # Errors
/// [`ParseError::Overflow`] when the field does not lie within the byte or is empty.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn split_bits(run: u8, offset: u32, width: u32) -> Result<u8, ParseError> {
    let shift = shift(offset, width)?;
    let mask = mask(width)?;
    Ok(run.checked_shr(shift).ok_or(ParseError::Overflow)? & mask)
}

/// `run` with the field at bits `offset .. offset + width` (from the most significant bit)
/// replaced by `value`.
///
/// # Errors
/// [`ParseError::Overflow`] when the field does not lie within the byte or is empty, or when
/// `value` does not fit in `width` bits: a value is never truncated (a `functionFlag` above
/// 127 is refused, not cut to seven bits).
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn join_bits(run: u8, offset: u32, width: u32, value: u8) -> Result<u8, ParseError> {
    let shift = shift(offset, width)?;
    let mask = mask(width)?;
    if value & !mask != 0 {
        return Err(ParseError::Overflow);
    }
    let field = mask.checked_shl(shift).ok_or(ParseError::Overflow)?;
    let placed = value.checked_shl(shift).ok_or(ParseError::Overflow)?;
    Ok((run & !field) | placed)
}

#[cfg(test)]
mod tests {
    use super::{join_bits, split_bits};
    use crate::codec::ParseError;

    #[test]
    fn a_run_splits_most_significant_bit_first() {
        assert_eq!(split_bits(0x83, 0, 1), Ok(1), "the error flag of 0x83");
        assert_eq!(split_bits(0x83, 1, 7), Ok(3), "the function of 0x83");
        assert_eq!(split_bits(0x03, 0, 1), Ok(0), "no error flag");
        assert_eq!(split_bits(0xA5, 0, 8), Ok(0xA5), "the whole byte");
        assert_eq!(split_bits(0b0110_0000, 1, 2), Ok(0b11), "a middle field");
    }

    #[test]
    fn a_run_joins_back_to_the_same_byte() {
        for byte in 0..=u8::MAX {
            let flag = split_bits(byte, 0, 1).expect("one bit");
            let function = split_bits(byte, 1, 7).expect("seven bits");
            let joined = join_bits(0, 0, 1, flag).and_then(|run| join_bits(run, 1, 7, function));
            assert_eq!(joined, Ok(byte), "{byte:#04x} round-trips");
        }
        assert_eq!(join_bits(0xFF, 1, 7, 0), Ok(0x80), "the field is replaced");
    }

    #[test]
    fn a_field_outside_the_byte_or_a_value_too_wide_is_an_overflow() {
        assert_eq!(
            split_bits(0, 4, 5),
            Err(ParseError::Overflow),
            "past the byte"
        );
        assert_eq!(
            split_bits(0, 9, 0),
            Err(ParseError::Overflow),
            "offset past the byte"
        );
        assert_eq!(
            split_bits(0, 0, 0),
            Err(ParseError::Overflow),
            "an empty field"
        );
        assert_eq!(
            join_bits(0, 1, 7, 0x80),
            Err(ParseError::Overflow),
            "128 needs 8 bits"
        );
        assert_eq!(
            join_bits(0, 7, 2, 1),
            Err(ParseError::Overflow),
            "past the byte"
        );
        assert_eq!(
            join_bits(0, 0, 0, 0),
            Err(ParseError::Overflow),
            "an empty field"
        );
    }
}
