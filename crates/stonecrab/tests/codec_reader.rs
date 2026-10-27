//! Adversarial and golden checks of `BoundedReader` (section 13.7 of the coding standards).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::ByteOrder;
    use stonecrab::codec::{BoundedReader, ParseError};

    const BIG: ByteOrder = ByteOrder::BigEndian;
    const LITTLE: ByteOrder = ByteOrder::LittleEndian;

    fn reversed(bytes: &[u8]) -> Vec<u8> {
        bytes.iter().rev().copied().collect()
    }

    fn truncated(needed: usize, available: usize) -> ParseError {
        ParseError::Truncated { needed, available }
    }

    // One module per multi-byte read function. The vectors are written out by hand as
    // big-endian bytes and the little-endian form is the reversed bytes, so no case
    // depends on the standard library conversion the implementation uses.
    macro_rules! scalar_read_tests {
        ($module:ident, $read:ident, $ty:ty, $width:literal, [$(($bytes:expr, $value:expr)),+ $(,)?]) => {
            mod $module {
                use super::*;

                // covers: SPEC mandatory-failure-cases / F1 truncated payloads
                #[test]
                fn zero_length_input_is_truncated() {
                    for order in [BIG, LITTLE] {
                        let mut reader = BoundedReader::new(&[]);
                        assert_eq!(reader.$read(order), Err(truncated($width, 0)), "{order:?}");
                    }
                }

                // covers: SPEC mandatory-failure-cases / F1 truncated payloads
                #[test]
                fn one_byte_short_is_truncated_and_consumes_nothing() {
                    let short = [0x5A_u8; $width - 1];
                    for order in [BIG, LITTLE] {
                        let mut reader = BoundedReader::new(&short);
                        assert_eq!(
                            reader.$read(order),
                            Err(truncated($width, $width - 1)),
                            "{order:?}"
                        );
                        assert_eq!(reader.remaining(), $width - 1, "cursor did not move");
                    }
                }

                #[test]
                fn exact_length_reads_the_value_in_both_orders() {
                    $(
                        let big_bytes: &[u8] = &$bytes;
                        let value: $ty = $value;
                        let mut reader = BoundedReader::new(big_bytes);
                        assert_eq!(reader.$read(BIG).unwrap().to_bits_or_self(), value.to_bits_or_self(), "big {big_bytes:?}");
                        assert_eq!(reader.remaining(), 0, "big consumed all");
                        reader.expect_eof().unwrap();

                        let little_bytes = reversed(big_bytes);
                        let mut reader = BoundedReader::new(&little_bytes);
                        assert_eq!(reader.$read(LITTLE).unwrap().to_bits_or_self(), value.to_bits_or_self(), "little {little_bytes:?}");
                        reader.expect_eof().unwrap();

                        let mut reader = BoundedReader::new(big_bytes);
                        let swapped = reader.$read(LITTLE).unwrap();
                        let mut reader = BoundedReader::new(&little_bytes);
                        assert_eq!(swapped.to_bits_or_self(), reader.$read(BIG).unwrap().to_bits_or_self(), "orders are mirror images");
                    )+
                }

                // covers: SPEC mandatory-failure-cases / F1 truncated payloads
                #[test]
                fn one_byte_long_leaves_a_trailing_byte() {
                    $(
                        let mut input: Vec<u8> = $bytes.to_vec();
                        input.push(0xEE);
                        let mut reader = BoundedReader::new(&input);
                        reader.$read(BIG).unwrap();
                        assert_eq!(
                            reader.expect_eof(),
                            Err(ParseError::TrailingBytes { remaining: 1 }),
                            "trailing byte after {input:?}"
                        );
                    )+
                }

                #[test]
                fn consecutive_reads_advance_the_cursor() {
                    let first: Vec<u8> = vec![0x01; $width];
                    let second: Vec<u8> = vec![0x02; $width];
                    let input = [first.clone(), second.clone()].concat();
                    let mut reader = BoundedReader::new(&input);
                    let a = reader.$read(BIG).unwrap();
                    let b = reader.$read(BIG).unwrap();
                    assert_eq!(reader.remaining(), 0, "both consumed");
                    assert_eq!(a.to_bits_or_self(), BoundedReader::new(&first).$read(BIG).unwrap().to_bits_or_self());
                    assert_eq!(b.to_bits_or_self(), BoundedReader::new(&second).$read(BIG).unwrap().to_bits_or_self());
                }
            }
        };
    }

    /// Compares floats by bit pattern so NaN cases are exact; integers compare as is.
    trait BitsOrSelf: Copy {
        type Bits: PartialEq + std::fmt::Debug;
        fn to_bits_or_self(self) -> Self::Bits;
    }

    macro_rules! integer_bits {
        ($($ty:ty),*) => {$(
            impl BitsOrSelf for $ty {
                type Bits = $ty;
                fn to_bits_or_self(self) -> $ty { self }
            }
        )*};
    }
    integer_bits!(u16, i16, u32, i32, u64, i64);

    impl BitsOrSelf for f32 {
        type Bits = u32;
        fn to_bits_or_self(self) -> u32 {
            self.to_bits()
        }
    }

    impl BitsOrSelf for f64 {
        type Bits = u64;
        fn to_bits_or_self(self) -> u64 {
            self.to_bits()
        }
    }

    scalar_read_tests!(
        u16_reads,
        read_u16,
        u16,
        2,
        [
            ([0x00, 0x00], 0),
            ([0x12, 0x34], 0x1234),
            ([0xFF, 0xFF], u16::MAX),
            ([0x80, 0x01], 0x8001)
        ]
    );
    scalar_read_tests!(
        i16_reads,
        read_i16,
        i16,
        2,
        [
            ([0x00, 0x00], 0),
            ([0xFF, 0xFE], -2),
            ([0x80, 0x00], i16::MIN),
            ([0x7F, 0xFF], i16::MAX)
        ]
    );
    scalar_read_tests!(
        u32_reads,
        read_u32,
        u32,
        4,
        [
            ([0, 0, 0, 0], 0),
            ([0x12, 0x34, 0x56, 0x78], 0x1234_5678),
            ([0xFF; 4], u32::MAX)
        ]
    );
    scalar_read_tests!(
        i32_reads,
        read_i32,
        i32,
        4,
        [
            ([0xFF, 0xFF, 0xFF, 0xFE], -2),
            ([0x80, 0, 0, 0], i32::MIN),
            ([0x7F, 0xFF, 0xFF, 0xFF], i32::MAX),
            ([0, 0, 0, 1], 1)
        ]
    );
    scalar_read_tests!(
        u64_reads,
        read_u64,
        u64,
        8,
        [
            ([0; 8], 0),
            ([1, 2, 3, 4, 5, 6, 7, 8], 0x0102_0304_0506_0708),
            ([0xFF; 8], u64::MAX)
        ]
    );
    scalar_read_tests!(
        i64_reads,
        read_i64,
        i64,
        8,
        [
            ([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE], -2),
            ([0x80, 0, 0, 0, 0, 0, 0, 0], i64::MIN),
            ([0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], i64::MAX)
        ]
    );
    scalar_read_tests!(
        f32_reads,
        read_f32,
        f32,
        4,
        [
            ([0x3F, 0xC0, 0, 0], 1.5),
            ([0xC0, 0x49, 0x0F, 0xDB], -std::f32::consts::PI),
            ([0x80, 0, 0, 0], -0.0)
        ]
    );
    scalar_read_tests!(
        f64_reads,
        read_f64,
        f64,
        8,
        [
            ([0x3F, 0xF8, 0, 0, 0, 0, 0, 0], 1.5),
            (
                [0xC0, 0x09, 0x21, 0xFB, 0x54, 0x44, 0x2D, 0x18],
                -std::f64::consts::PI
            ),
            ([0x80, 0, 0, 0, 0, 0, 0, 0], -0.0)
        ]
    );

    // Anchor: the macro tests above are only as good as their helper, so check the
    // helper's own claim against a hand-computed value.
    #[test]
    fn little_endian_vectors_are_the_reversed_big_endian_bytes() {
        assert_eq!(reversed(&[1, 2, 3]), [3, 2, 1]);
        assert_eq!(
            BoundedReader::new(&[0x34, 0x12]).read_u16(LITTLE),
            Ok(0x1234)
        );
        assert_eq!(BoundedReader::new(&[0x34, 0x12]).read_u16(BIG), Ok(0x3412));
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn single_byte_reads_follow_the_same_boundaries() {
        assert_eq!(BoundedReader::new(&[]).read_u8(), Err(truncated(1, 0)));
        assert_eq!(BoundedReader::new(&[]).read_i8(), Err(truncated(1, 0)));
        let mut reader = BoundedReader::new(&[0xFE, 0x7F, 0x80]);
        assert_eq!(reader.read_u8(), Ok(0xFE));
        assert_eq!(reader.read_i8(), Ok(127));
        assert_eq!(reader.read_i8(), Ok(-128));
        assert_eq!(reader.remaining(), 0);
        assert_eq!(reader.expect_eof(), Ok(()));
        let mut long = BoundedReader::new(&[0x01, 0x02]);
        assert_eq!(long.read_u8(), Ok(1));
        assert_eq!(
            long.expect_eof(),
            Err(ParseError::TrailingBytes { remaining: 1 })
        );
        assert_eq!(BoundedReader::new(&[0xFF]).read_i8(), Ok(-1));
        assert_eq!(BoundedReader::new(&[0x00]).read_u8(), Ok(0));
    }

    #[test]
    fn float_special_values_keep_their_exact_bits_in_both_orders() {
        let f32_patterns: [u32; 8] = [
            0x7FC0_0000, // quiet NaN
            0x7FC0_0001, // quiet NaN with payload
            0xFFC0_0000, // negative quiet NaN
            0x7F80_0001, // signaling NaN
            0x7F80_0000, // +inf
            0xFF80_0000, // -inf
            0x0000_0001, // smallest subnormal
            0x8000_0000, // -0.0
        ];
        for bits in f32_patterns {
            let big = bits_to_be_u32(bits);
            let little = reversed(&big);
            let got = BoundedReader::new(&big).read_f32(BIG).unwrap();
            assert_eq!(got.to_bits(), bits, "f32 big {bits:#010x}");
            let got = BoundedReader::new(&little).read_f32(LITTLE).unwrap();
            assert_eq!(got.to_bits(), bits, "f32 little {bits:#010x}");
        }
        let f64_patterns: [u64; 8] = [
            0x7FF8_0000_0000_0000,
            0x7FF8_0000_0000_0001,
            0xFFF8_0000_0000_0000,
            0x7FF0_0000_0000_0001,
            0x7FF0_0000_0000_0000,
            0xFFF0_0000_0000_0000,
            0x0000_0000_0000_0001,
            0x8000_0000_0000_0000,
        ];
        for bits in f64_patterns {
            let big = bits_to_be_u64(bits);
            let little = reversed(&big);
            let got = BoundedReader::new(&big).read_f64(BIG).unwrap();
            assert_eq!(got.to_bits(), bits, "f64 big {bits:#018x}");
            let got = BoundedReader::new(&little).read_f64(LITTLE).unwrap();
            assert_eq!(got.to_bits(), bits, "f64 little {bits:#018x}");
        }
        assert!(
            BoundedReader::new(&[0x7F, 0xC0, 0, 0])
                .read_f32(BIG)
                .unwrap()
                .is_nan()
        );
        assert!(
            BoundedReader::new(&[0x7F, 0x80, 0, 0])
                .read_f32(BIG)
                .unwrap()
                .is_infinite()
        );
    }

    /// Splits a word into big-endian bytes with `u32::to_be_bytes`'s own definition
    /// spelled out by shifting, so the vectors above are not produced by the code under test.
    fn bits_to_be_u32(bits: u32) -> Vec<u8> {
        [24_u32, 16, 8, 0]
            .map(|shift| u8::try_from((bits >> shift) & 0xFF).unwrap())
            .to_vec()
    }

    fn bits_to_be_u64(bits: u64) -> Vec<u8> {
        [56_u32, 48, 40, 32, 24, 16, 8, 0]
            .map(|shift| u8::try_from((bits >> shift) & 0xFF).unwrap())
            .to_vec()
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths
    #[test]
    fn read_bytes_boundaries() {
        let data = [1_u8, 2, 3, 4];
        let mut reader = BoundedReader::new(&data);
        assert_eq!(
            reader.read_bytes(0, 0),
            Ok(&data[..0]),
            "zero length at zero max"
        );
        assert_eq!(reader.remaining(), 4, "nothing consumed");
        assert_eq!(
            reader.read_bytes(4, 4),
            Ok(&data[..]),
            "exact length at exact max"
        );
        assert_eq!(reader.remaining(), 0);
        assert_eq!(reader.read_bytes(0, 4), Ok(&data[..0]), "empty read at end");
        assert_eq!(
            reader.read_bytes(1, 4),
            Err(truncated(1, 0)),
            "zero bytes left"
        );

        let mut reader = BoundedReader::new(&data);
        assert_eq!(reader.read_bytes(3, 8), Ok(&data[..3]));
        assert_eq!(
            reader.read_bytes(2, 8),
            Err(truncated(2, 1)),
            "one byte short"
        );
        assert_eq!(
            reader.remaining(),
            1,
            "a failed read does not move the cursor"
        );
        assert_eq!(reader.read_bytes(1, 8), Ok(&data[3..]));
        assert_eq!(
            BoundedReader::new(&data).read_bytes(0, 0).map(<[u8]>::len),
            Ok(0)
        );

        let mut reader = BoundedReader::new(&data);
        reader.read_bytes(3, 8).unwrap();
        assert_eq!(
            reader.expect_eof(),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "one byte long"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn read_bytes_rejects_max_plus_one_even_when_the_bytes_are_present() {
        let data = [0_u8; 16];
        let mut reader = BoundedReader::new(&data);
        assert_eq!(
            reader.read_bytes(9, 8),
            Err(ParseError::OversizedLength { length: 9, max: 8 }),
            "max plus one"
        );
        assert_eq!(reader.remaining(), 16, "nothing consumed");
        assert_eq!(
            reader.read_bytes(8, 8).map(<[u8]>::len),
            Ok(8),
            "max itself is allowed"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn read_bytes_refuses_a_one_gibibyte_length_before_touching_the_data() {
        const GIB: usize = 1 << 30;
        let data = [0xAA_u8; 4];
        let mut reader = BoundedReader::new(&data);
        assert_eq!(
            reader.read_bytes(GIB, 253),
            Err(ParseError::OversizedLength {
                length: GIB,
                max: 253
            }),
            "over the maximum"
        );
        assert_eq!(
            reader.read_bytes(GIB, GIB),
            Err(truncated(GIB, 4)),
            "within the maximum but not present: no allocation, no copy"
        );
        assert_eq!(
            reader.read_bytes(usize::MAX, usize::MAX),
            Err(truncated(usize::MAX, 4)),
            "largest possible length"
        );
        assert_eq!(reader.remaining(), 4, "nothing consumed");
    }

    #[test]
    fn read_bytes_returns_a_view_of_the_input_not_a_copy() {
        let data = [9_u8, 8, 7, 6];
        let mut reader = BoundedReader::new(&data);
        reader.read_u8().unwrap();
        let view = reader.read_bytes(2, 2).unwrap();
        assert!(
            std::ptr::eq(view.as_ptr(), data[1..].as_ptr()),
            "same memory"
        );
        assert_eq!(view, [8, 7]);
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn read_bits_follows_coil_packing() {
        let mut reader = BoundedReader::new(&[0b0000_0101]);
        assert_eq!(
            reader.read_bits_lsb_first(3, 2000),
            Ok(vec![true, false, true])
        );
        assert_eq!(
            reader.expect_eof(),
            Ok(()),
            "three bits consume exactly one byte"
        );

        let mut reader = BoundedReader::new(&[0x01, 0x80, 0xFF]);
        let bits = reader.read_bits_lsb_first(16, 2000).unwrap();
        assert_eq!(bits.len(), 16);
        assert_eq!(
            bits.iter().position(|bit| *bit),
            Some(0),
            "bit 0 of byte 0 is first"
        );
        assert_eq!(
            bits.iter().rposition(|bit| *bit),
            Some(15),
            "bit 7 of byte 1 is last"
        );
        assert_eq!(bits.iter().filter(|bit| **bit).count(), 2);
        assert_eq!(reader.remaining(), 1, "two bytes consumed");

        let mut reader = BoundedReader::new(&[0xFF, 0x01]);
        assert_eq!(
            reader.read_bits_lsb_first(9, 9).unwrap(),
            vec![true; 9],
            "ninth bit in byte 1"
        );
        let mut reader = BoundedReader::new(&[0xFF]);
        assert_eq!(
            reader.read_bits_lsb_first(3, 8).unwrap(),
            vec![true; 3],
            "padding ignored"
        );
        assert_eq!(reader.remaining(), 0, "padding byte consumed");
    }

    #[test]
    fn read_bits_of_zero_count_consumes_nothing() {
        let mut reader = BoundedReader::new(&[0xFF]);
        assert_eq!(reader.read_bits_lsb_first(0, 0), Ok(Vec::new()));
        assert_eq!(reader.remaining(), 1);
        assert_eq!(
            BoundedReader::new(&[]).read_bits_lsb_first(0, 8),
            Ok(Vec::new())
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn read_bits_short_input_is_truncated_and_consumes_nothing() {
        assert_eq!(
            BoundedReader::new(&[]).read_bits_lsb_first(1, 8),
            Err(truncated(1, 0)),
            "zero length input"
        );
        let mut reader = BoundedReader::new(&[0xFF]);
        assert_eq!(
            reader.read_bits_lsb_first(9, 16),
            Err(truncated(2, 1)),
            "one byte short"
        );
        assert_eq!(reader.remaining(), 1);
        assert_eq!(
            reader.read_bits_lsb_first(usize::MAX, usize::MAX),
            Err(truncated(usize::MAX.div_ceil(8), 1)),
            "no allocation for an impossible count"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths
    #[test]
    fn read_bits_enforces_max_and_reports_one_byte_long_input() {
        let data = [0xFF_u8; 300];
        let mut reader = BoundedReader::new(&data);
        assert_eq!(
            reader.read_bits_lsb_first(2001, 2000),
            Err(ParseError::OversizedLength {
                length: 2001,
                max: 2000
            }),
            "max plus one"
        );
        assert_eq!(reader.remaining(), 300, "nothing consumed");
        assert_eq!(
            reader.read_bits_lsb_first(2000, 2000).unwrap().len(),
            2000,
            "max itself"
        );
        assert_eq!(reader.remaining(), 50, "250 bytes consumed");
        assert_eq!(
            reader.read_bits_lsb_first(1 << 30, 2000),
            Err(ParseError::OversizedLength {
                length: 1 << 30,
                max: 2000
            })
        );
        let mut reader = BoundedReader::new(&[0x01, 0x02]);
        reader.read_bits_lsb_first(8, 8).unwrap();
        assert_eq!(
            reader.expect_eof(),
            Err(ParseError::TrailingBytes { remaining: 1 })
        );
    }

    #[test]
    fn expect_eof_is_repeatable_and_does_not_consume() {
        let reader = BoundedReader::new(&[1, 2, 3]);
        let expected = Err(ParseError::TrailingBytes { remaining: 3 });
        assert_eq!(reader.expect_eof(), expected);
        assert_eq!(reader.expect_eof(), expected, "second call agrees");
        assert_eq!(reader.remaining(), 3);
        assert_eq!(BoundedReader::new(&[]).expect_eof(), Ok(()));
    }

    #[test]
    fn debug_shows_the_unread_count_and_never_the_bytes() {
        let reader = BoundedReader::new(&[0xDE, 0xAD, 0xBE]);
        let shown = format!("{reader:?}");
        assert_eq!(shown, "BoundedReader { remaining: 3 }");
        let copy = reader.clone();
        assert_eq!(
            copy.remaining(),
            reader.remaining(),
            "clone keeps the position"
        );
    }
}
