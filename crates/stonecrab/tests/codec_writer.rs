//! Golden and boundary checks of `BoundedWriter`.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::ByteOrder;
    use stonecrab::codec::{BoundedReader, BoundedWriter, ParseError};

    const BIG: ByteOrder = ByteOrder::BigEndian;
    const LITTLE: ByteOrder = ByteOrder::LittleEndian;

    fn oversized(length: usize, max: usize) -> ParseError {
        ParseError::OversizedLength { length, max }
    }

    // Each case is written as hand-computed big-endian bytes; the little-endian form is
    // the same bytes reversed.
    macro_rules! scalar_write_tests {
        ($test:ident, $write:ident, [$(($value:expr, $bytes:expr)),+ $(,)?]) => {
            #[test]
            fn $test() {
                $(
                    let big: Vec<u8> = $bytes.to_vec();
                    let little: Vec<u8> = big.iter().rev().copied().collect();
                    let mut writer = BoundedWriter::new(big.len());
                    writer.$write($value, BIG).unwrap();
                    assert_eq!(writer.as_slice(), big, "big {:?}", $value);
                    assert_eq!(writer.remaining_capacity(), 0, "exact fit");
                    let mut writer = BoundedWriter::new(big.len());
                    writer.$write($value, LITTLE).unwrap();
                    assert_eq!(writer.as_slice(), little, "little {:?}", $value);

                    let mut short = BoundedWriter::new(big.len() - 1);
                    assert_eq!(
                        short.$write($value, BIG),
                        Err(oversized(big.len(), big.len() - 1)),
                        "one byte over the capacity"
                    );
                    assert!(short.as_slice().is_empty(), "failed write wrote nothing");
                )+
            }
        };
    }

    scalar_write_tests!(
        u16_writes,
        write_u16,
        [(0x1234_u16, [0x12, 0x34]), (u16::MAX, [0xFF, 0xFF])]
    );
    scalar_write_tests!(
        i16_writes,
        write_i16,
        [(-2_i16, [0xFF, 0xFE]), (i16::MIN, [0x80, 0x00])]
    );
    scalar_write_tests!(
        u32_writes,
        write_u32,
        [(0x1234_5678_u32, [0x12, 0x34, 0x56, 0x78])]
    );
    scalar_write_tests!(
        i32_writes,
        write_i32,
        [
            (-2_i32, [0xFF, 0xFF, 0xFF, 0xFE]),
            (i32::MIN, [0x80, 0, 0, 0])
        ]
    );
    scalar_write_tests!(
        u64_writes,
        write_u64,
        [(0x0102_0304_0506_0708_u64, [1, 2, 3, 4, 5, 6, 7, 8])]
    );
    scalar_write_tests!(
        i64_writes,
        write_i64,
        [
            (-2_i64, [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFE]),
            (i64::MIN, [0x80, 0, 0, 0, 0, 0, 0, 0])
        ]
    );
    scalar_write_tests!(
        f32_writes,
        write_f32,
        [(1.5_f32, [0x3F, 0xC0, 0, 0]), (-0.0_f32, [0x80, 0, 0, 0])]
    );
    scalar_write_tests!(
        f64_writes,
        write_f64,
        [
            (1.5_f64, [0x3F, 0xF8, 0, 0, 0, 0, 0, 0]),
            (-0.0_f64, [0x80, 0, 0, 0, 0, 0, 0, 0])
        ]
    );

    #[test]
    fn single_byte_writes_and_the_capacity_boundary() {
        let mut writer = BoundedWriter::new(2);
        assert_eq!(writer.capacity(), 2);
        assert_eq!(writer.remaining_capacity(), 2);
        writer.write_u8(0xFE).unwrap();
        writer.write_i8(-1).unwrap();
        assert_eq!(writer.remaining_capacity(), 0);
        assert_eq!(
            writer.write_u8(0),
            Err(oversized(3, 2)),
            "one past the capacity"
        );
        assert_eq!(writer.write_i8(0), Err(oversized(3, 2)));
        assert_eq!(
            writer.as_slice(),
            [0xFE, 0xFF],
            "failed writes changed nothing"
        );
        assert_eq!(writer.into_bytes(), vec![0xFE, 0xFF]);
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn zero_capacity_accepts_only_empty_writes() {
        let mut writer = BoundedWriter::new(0);
        assert_eq!(writer.write_u8(1), Err(oversized(1, 0)));
        assert_eq!(writer.write_bytes(&[]), Ok(()), "empty write fits");
        assert_eq!(writer.write_bits_lsb_first(&[]), Ok(()), "no bits fit");
        assert_eq!(writer.write_bits_lsb_first(&[true]), Err(oversized(1, 0)));
        assert_eq!(writer.as_slice().len(), 0, "nothing was written");
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn write_bytes_is_all_or_nothing_at_the_boundary() {
        let mut writer = BoundedWriter::new(4);
        writer.write_bytes(&[1, 2, 3]).unwrap();
        assert_eq!(
            writer.write_bytes(&[4, 5]),
            Err(oversized(5, 4)),
            "capacity plus one"
        );
        assert_eq!(writer.as_slice(), [1, 2, 3], "nothing partial was written");
        assert_eq!(writer.write_bytes(&[4]), Ok(()), "capacity exactly");
        assert_eq!(writer.as_slice(), [1, 2, 3, 4]);
        assert_eq!(writer.remaining_capacity(), 0);
    }

    #[test]
    fn huge_requests_are_refused_without_allocating() {
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(writer.capacity(), usize::MAX);
        writer.write_u8(1).unwrap();
        assert_eq!(writer.remaining_capacity(), usize::MAX - 1);
        let huge = BoundedWriter::new(usize::MAX);
        assert!(
            huge.as_slice().is_empty(),
            "a large capacity allocates nothing"
        );
        let mut limited = BoundedWriter::new(1 << 30);
        assert_eq!(limited.write_bits_lsb_first(&[]), Ok(()));
    }

    #[test]
    fn write_bits_packs_coils_least_significant_bit_first() {
        let mut writer = BoundedWriter::new(3);
        writer
            .write_bits_lsb_first(&[true, true, false, true])
            .unwrap();
        assert_eq!(writer.as_slice(), [0b0000_1011]);
        writer
            .write_bits_lsb_first(
                &[false; 7]
                    .iter()
                    .chain(&[true])
                    .copied()
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(writer.as_slice(), [0x0B, 0x80]);
        writer.write_bits_lsb_first(&[true; 9]).unwrap_err();
        assert_eq!(writer.as_slice(), [0x0B, 0x80], "refused before packing");
        assert_eq!(
            writer.write_bits_lsb_first(&[true; 9]),
            Err(oversized(4, 3))
        );
        writer.write_bits_lsb_first(&[true]).unwrap();
        assert_eq!(writer.into_bytes(), vec![0x0B, 0x80, 0x01]);
    }

    #[test]
    fn writer_output_reads_back_through_the_reader() {
        let mut writer = BoundedWriter::new(32);
        writer.write_i16(-300, LITTLE).unwrap();
        writer.write_f32(f32::NAN, BIG).unwrap();
        writer.write_bits_lsb_first(&[true, false]).unwrap();
        let bytes = writer.into_bytes();
        let mut reader = BoundedReader::new(&bytes);
        assert_eq!(reader.read_i16(LITTLE), Ok(-300));
        assert_eq!(reader.read_f32(BIG).unwrap().to_bits(), f32::NAN.to_bits());
        assert_eq!(reader.read_bits_lsb_first(2, 2), Ok(vec![true, false]));
        assert_eq!(reader.expect_eof(), Ok(()));
    }

    #[test]
    fn debug_shows_counts_and_never_the_bytes() {
        let mut writer = BoundedWriter::new(8);
        writer.write_bytes(&[0xDE, 0xAD]).unwrap();
        assert_eq!(
            format!("{writer:?}"),
            "BoundedWriter { written: 2, capacity: 8 }"
        );
        let copy = writer.clone();
        assert_eq!(
            copy.as_slice(),
            writer.as_slice(),
            "clone keeps the content"
        );
    }
}
