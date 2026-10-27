//! Property tests of the bounded codec (volume set by `PROPTEST_CASES`).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use stonecrab::ByteOrder;
    use stonecrab::codec::{
        BoundedReader, BoundedWriter, ParseError, coil_byte_count, pack_bits_lsb_first,
        unpack_bits_lsb_first,
    };

    fn order(little: bool) -> ByteOrder {
        if little {
            ByteOrder::LittleEndian
        } else {
            ByteOrder::BigEndian
        }
    }

    proptest! {
        // covers: SPEC mandatory-failure-cases / F1 exact integers survive the codec
        #[test]
        fn prop_round_trip_exact_integers(
            a in any::<u8>(),
            b in any::<i8>(),
            c in any::<u16>(),
            d in any::<i16>(),
            e in any::<u32>(),
            f in any::<i32>(),
            g in any::<u64>(),
            h in any::<i64>(),
            little in any::<bool>(),
        ) {
            let order = order(little);
            let mut writer = BoundedWriter::new(30);
            writer.write_u8(a).unwrap();
            writer.write_i8(b).unwrap();
            writer.write_u16(c, order).unwrap();
            writer.write_i16(d, order).unwrap();
            writer.write_u32(e, order).unwrap();
            writer.write_i32(f, order).unwrap();
            writer.write_u64(g, order).unwrap();
            writer.write_i64(h, order).unwrap();
            let bytes = writer.into_bytes();
            prop_assert_eq!(bytes.len(), 30);

            let mut reader = BoundedReader::new(&bytes);
            prop_assert_eq!(reader.read_u8().unwrap(), a);
            prop_assert_eq!(reader.read_i8().unwrap(), b);
            prop_assert_eq!(reader.read_u16(order).unwrap(), c);
            prop_assert_eq!(reader.read_i16(order).unwrap(), d);
            prop_assert_eq!(reader.read_u32(order).unwrap(), e);
            prop_assert_eq!(reader.read_i32(order).unwrap(), f);
            prop_assert_eq!(reader.read_u64(order).unwrap(), g);
            prop_assert_eq!(reader.read_i64(order).unwrap(), h);
            prop_assert_eq!(reader.expect_eof(), Ok(()));
        }

        #[test]
        fn prop_round_trip_floats_keeps_every_bit(
            bits32 in any::<u32>(),
            bits64 in any::<u64>(),
            little in any::<bool>(),
        ) {
            let order = order(little);
            let mut writer = BoundedWriter::new(12);
            writer.write_f32(f32::from_bits(bits32), order).unwrap();
            writer.write_f64(f64::from_bits(bits64), order).unwrap();
            let bytes = writer.into_bytes();
            let mut reader = BoundedReader::new(&bytes);
            prop_assert_eq!(reader.read_f32(order).unwrap().to_bits(), bits32);
            prop_assert_eq!(reader.read_f64(order).unwrap().to_bits(), bits64);
        }

        // covers: SPEC mandatory-failure-cases / F1 oversized lengths
        #[test]
        fn prop_length_checked_before_allocation(
            max in any::<usize>(),
            excess in 1_usize..,
            data in vec(any::<u8>(), 0..16),
        ) {
            let len = max.saturating_add(excess);
            prop_assume!(len > max);
            let mut reader = BoundedReader::new(&data);
            prop_assert_eq!(
                reader.read_bytes(len, max),
                Err(ParseError::OversizedLength { length: len, max })
            );
            prop_assert_eq!(
                reader.read_bits_lsb_first(len, max),
                Err(ParseError::OversizedLength { length: len, max })
            );
            prop_assert_eq!(reader.remaining(), data.len(), "nothing consumed");
        }

        #[test]
        fn prop_reads_within_max_never_exceed_the_input(
            len in any::<usize>(),
            data in vec(any::<u8>(), 0..16),
        ) {
            let mut reader = BoundedReader::new(&data);
            match reader.read_bytes(len, usize::MAX) {
                Ok(got) => prop_assert_eq!(Some(got), data.get(..len)),
                Err(error) => prop_assert_eq!(
                    error,
                    ParseError::Truncated { needed: len, available: data.len() }
                ),
            }
            let mut reader = BoundedReader::new(&data);
            match reader.read_bits_lsb_first(len, usize::MAX) {
                Ok(bits) => prop_assert_eq!(bits.len(), len),
                Err(error) => {
                    let truncated = matches!(error, ParseError::Truncated { .. });
                    prop_assert!(truncated);
                }
            }
        }

        #[test]
        fn prop_bits_round_trip(bits in vec(any::<bool>(), 0..200)) {
            let packed = pack_bits_lsb_first(&bits);
            prop_assert_eq!(packed.len(), coil_byte_count(bits.len()));
            prop_assert_eq!(unpack_bits_lsb_first(&packed, bits.len()).unwrap(), bits.clone());
            let mut reader = BoundedReader::new(&packed);
            prop_assert_eq!(reader.read_bits_lsb_first(bits.len(), bits.len()).unwrap(), bits);
            prop_assert_eq!(reader.expect_eof(), Ok(()));
        }

        #[test]
        fn prop_writer_never_exceeds_its_capacity(
            capacity in 0_usize..32,
            chunks in vec(vec(any::<u8>(), 0..12), 0..8),
        ) {
            let mut writer = BoundedWriter::new(capacity);
            for chunk in &chunks {
                let before = writer.as_slice().len();
                let expected = if writer.write_bytes(chunk).is_ok() {
                    before.saturating_add(chunk.len())
                } else {
                    before
                };
                prop_assert_eq!(writer.as_slice().len(), expected);
                prop_assert!(writer.as_slice().len() <= capacity);
            }
        }

        #[test]
        fn prop_reader_never_panics_on_arbitrary_calls(
            data in vec(any::<u8>(), 0..64),
            script in vec((0_u8..14, any::<usize>(), any::<usize>()), 0..24),
        ) {
            let mut reader = BoundedReader::new(&data);
            for (op, a, b) in script {
                let before = reader.remaining();
                let failed = match op {
                    0 => reader.read_u8().is_err(),
                    1 => reader.read_i8().is_err(),
                    2 => reader.read_u16(order(a % 2 == 0)).is_err(),
                    3 => reader.read_i16(order(a % 2 == 0)).is_err(),
                    4 => reader.read_u32(order(a % 2 == 0)).is_err(),
                    5 => reader.read_i32(order(a % 2 == 0)).is_err(),
                    6 => reader.read_u64(order(a % 2 == 0)).is_err(),
                    7 => reader.read_i64(order(a % 2 == 0)).is_err(),
                    8 => reader.read_f32(order(a % 2 == 0)).is_err(),
                    9 => reader.read_f64(order(a % 2 == 0)).is_err(),
                    10 => reader.read_bytes(a, b).is_err(),
                    11 => reader.read_bits_lsb_first(a, b).is_err(),
                    _ => reader.expect_eof().is_err(),
                };
                if failed && op != 12 {
                    prop_assert_eq!(reader.remaining(), before, "failed reads do not consume");
                }
                prop_assert!(reader.remaining() <= before);
            }
        }
    }
}
