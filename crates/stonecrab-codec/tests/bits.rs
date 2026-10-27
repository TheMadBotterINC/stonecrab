//! Coil bit packing helpers.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab_codec::{
        ParseError, coil_byte_count, pack_bits_lsb_first, unpack_bits_lsb_first,
    };

    /// `usize::MAX` divided by eight, rounded up, without arithmetic operators.
    fn top_byte_count() -> usize {
        (usize::MAX >> 3).saturating_add(1)
    }

    #[test]
    fn byte_count_rounds_up_and_cannot_overflow() {
        let table = [
            (0, 0),
            (1, 1),
            (7, 1),
            (8, 1),
            (9, 2),
            (16, 2),
            (17, 3),
            (2000, 250),
        ];
        for (count, bytes) in table {
            assert_eq!(coil_byte_count(count), bytes, "count {count}");
        }
        assert_eq!(
            coil_byte_count(usize::MAX),
            top_byte_count(),
            "no overflow at the top"
        );
    }

    #[test]
    fn packing_puts_bit_zero_of_byte_zero_first() {
        assert_eq!(pack_bits_lsb_first(&[]), Vec::<u8>::new(), "empty");
        assert_eq!(pack_bits_lsb_first(&[true]), [0x01]);
        assert_eq!(pack_bits_lsb_first(&[false, true]), [0x02]);
        assert_eq!(pack_bits_lsb_first(&[true, false, true]), [0x05]);
        let high_only = [false, false, false, false, false, false, false, true];
        assert_eq!(pack_bits_lsb_first(&high_only), [0x80]);
        assert_eq!(pack_bits_lsb_first(&[true; 8]), [0xFF]);
        assert_eq!(
            pack_bits_lsb_first(&[true; 9]),
            [0xFF, 0x01],
            "ninth bit starts byte 1"
        );
        assert_eq!(pack_bits_lsb_first(&[false; 16]), [0x00, 0x00]);
    }

    #[test]
    fn unpacking_reads_the_same_order_and_ignores_padding_and_extra_bytes() {
        assert_eq!(
            unpack_bits_lsb_first(&[0x05], 3).unwrap(),
            [true, false, true]
        );
        assert_eq!(
            unpack_bits_lsb_first(&[0xFF], 3).unwrap(),
            [true, true, true],
            "padding bits are ignored"
        );
        assert_eq!(
            unpack_bits_lsb_first(&[0x80, 0x01], 9).unwrap(),
            [false, false, false, false, false, false, false, true, true]
        );
        assert_eq!(
            unpack_bits_lsb_first(&[0x01, 0xFF], 1).unwrap(),
            [true],
            "bytes past the count are ignored"
        );
        assert!(
            unpack_bits_lsb_first(&[], 0).unwrap().is_empty(),
            "zero bits from nothing"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn unpacking_more_bits_than_the_input_holds_is_truncated() {
        assert_eq!(
            unpack_bits_lsb_first(&[], 1),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            })
        );
        assert_eq!(
            unpack_bits_lsb_first(&[0xFF], 9),
            Err(ParseError::Truncated {
                needed: 2,
                available: 1
            })
        );
        assert_eq!(
            unpack_bits_lsb_first(&[0xFF], usize::MAX),
            Err(ParseError::Truncated {
                needed: top_byte_count(),
                available: 1
            }),
            "refused before any allocation"
        );
    }

    #[test]
    fn pack_then_unpack_round_trips_every_length_up_to_forty() {
        for count in 0..=40_usize {
            let bits: Vec<bool> = (0..count).map(|i| i % 3 == 0 || i % 7 == 2).collect();
            let packed = pack_bits_lsb_first(&bits);
            assert_eq!(
                packed.len(),
                coil_byte_count(count),
                "count {count}: byte count"
            );
            assert_eq!(
                unpack_bits_lsb_first(&packed, count).unwrap(),
                bits,
                "count {count}"
            );
        }
    }
}
