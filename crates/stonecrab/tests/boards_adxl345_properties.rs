//! Property tests and exhaustive sweeps of the ADXL345 sample decode (volume set by
//! `PROPTEST_CASES`).
//!
//! The reference below decodes a pair with different arithmetic from the library's (a
//! widened unsigned value and a subtraction instead of `i16::from_le_bytes`) and states each
//! resolution as a bit count rather than a largest count. Nothing here touches a device.
#![cfg(feature = "boards")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime};

    use proptest::prelude::*;
    use stonecrab::boards::{AccelRange, AccelReading, AccelSample, Adxl345Config};
    use stonecrab::{ErrorKind, PlcValue};

    const RANGES: [AccelRange; 4] = [
        AccelRange::G2,
        AccelRange::G4,
        AccelRange::G8,
        AccelRange::G16,
    ];

    fn config(range: AccelRange, full_resolution: bool) -> Adxl345Config {
        let mut config = Adxl345Config::default();
        config.range = range;
        config.full_resolution = full_resolution;
        config
    }

    /// The resolution in bits and the counts per g: the bits and the 10-bit sensitivities
    /// from the datasheet's Table 1, 256 at full resolution from its `FULL_RES` description.
    fn resolution(range: AccelRange, full_resolution: bool) -> (u32, u16) {
        let (bits, counts_per_g) = match range {
            AccelRange::G2 => (10, 256),
            AccelRange::G4 => (11, 128),
            AccelRange::G8 => (12, 64),
            AccelRange::G16 | _ => (13, 32),
        };
        if full_resolution {
            (bits, 256)
        } else {
            (10, counts_per_g)
        }
    }

    /// Half the span of a `bits`-bit two's complement count: 2 to the power `bits - 1`.
    fn half(bits: u32) -> i64 {
        (1_i64 << bits) >> 1
    }

    /// A little-endian pair as a two's complement count.
    fn reference_count(low: u8, high: u8) -> i64 {
        let value = i64::from(high)
            .wrapping_mul(256)
            .wrapping_add(i64::from(low));
        if value >= 32_768 {
            value.wrapping_sub(65_536)
        } else {
            value
        }
    }

    /// The counts of the three pairs, or `None` when one does not fit `bits` bits.
    fn reference(data: [u8; 6], bits: u32) -> Option<[i64; 3]> {
        let [x0, x1, y0, y1, z0, z1] = data;
        let counts = [
            reference_count(x0, x1),
            reference_count(y0, y1),
            reference_count(z0, z1),
        ];
        let half = half(bits);
        counts
            .iter()
            .all(|count| half.wrapping_neg() <= *count && *count < half)
            .then_some(counts)
    }

    fn check(data: [u8; 6], range: AccelRange, full_resolution: bool) {
        let (bits, counts_per_g) = resolution(range, full_resolution);
        let decoded = AccelSample::from_registers(data, config(range, full_resolution));
        match (reference(data, bits), decoded) {
            (Some(counts), Ok(sample)) => {
                let got = [sample.x, sample.y, sample.z].map(i64::from);
                assert_eq!(got, counts, "{data:02x?} {range:?} {full_resolution}");
                let reading =
                    AccelReading::new(sample, counts_per_g, Instant::now(), SystemTime::UNIX_EPOCH);
                for (measurement, count) in reading
                    .measurements()
                    .expect("measurements")
                    .iter()
                    .zip(counts)
                {
                    let Some(PlcValue::F64(g)) = measurement.value else {
                        panic!("a value in g");
                    };
                    // The scale is a power of two, so g times the scale is the count, exactly.
                    assert_eq!(
                        g * f64::from(counts_per_g),
                        f64::from(i32::try_from(count).expect("small"))
                    );
                }
            }
            (None, Err(error)) => {
                assert_eq!(error.kind(), ErrorKind::MalformedData, "{data:02x?}");
            }
            (expected, got) => panic!(
                "{data:02x?} {range:?} {full_resolution}: reference {expected:?}, library {got:?}"
            ),
        }
    }

    proptest! {
        #[test]
        fn every_register_block_is_decoded_like_the_reference(
            data in any::<[u8; 6]>(),
            range in prop::sample::select(RANGES.to_vec()),
            full_resolution in any::<bool>(),
        ) {
            check(data, range, full_resolution);
        }

        // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (round trip)
        #[test]
        fn counts_within_the_resolution_round_trip(
            x in -4096_i16..=4095,
            y in -4096_i16..=4095,
            z in -4096_i16..=4095,
            range in prop::sample::select(RANGES.to_vec()),
            full_resolution in any::<bool>(),
        ) {
            let (bits, _) = resolution(range, full_resolution);
            let half = i16::try_from(half(bits)).expect("at most 4096");
            let fit = |count: i16| count.clamp(half.wrapping_neg(), half.wrapping_sub(1));
            let (x, y, z) = (fit(x), fit(y), fit(z));
            let [x0, x1] = x.to_le_bytes();
            let [y0, y1] = y.to_le_bytes();
            let [z0, z1] = z.to_le_bytes();
            let sample = AccelSample::from_registers([x0, x1, y0, y1, z0, z1], config(range, full_resolution))
                .expect("a count within the resolution");
            prop_assert_eq!((sample.x, sample.y, sample.z), (x, y, z));
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (every sample is classified)
    #[test]
    fn every_pair_on_each_axis_is_classified_like_the_reference_at_every_resolution() {
        for range in RANGES {
            for full_resolution in [false, true] {
                for value in 0..=u16::MAX {
                    let [low, high] = value.to_le_bytes();
                    check([low, high, 0, 0, 0, 0], range, full_resolution);
                    check([0, 0, low, high, 0, 0], range, full_resolution);
                    check([0, 0, 0, 0, low, high], range, full_resolution);
                }
            }
        }
    }
}
