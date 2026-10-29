//! Property tests and exhaustive sweeps of the MAX31855 frame decode (volume set by
//! `PROPTEST_CASES`).
//!
//! The reference below decodes a frame with different arithmetic from the library's (bit
//! tests and widened integers instead of byte halves and arithmetic shifts), and the sweeps
//! walk every value of one half of the frame against representative values of the other.
//! Nothing here touches a device.
#![cfg(feature = "boards")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::{Instant, SystemTime};

    use proptest::prelude::*;
    use stonecrab::boards::{ThermocoupleFault, ThermocoupleReading};
    use stonecrab::sensors::Quality;
    use stonecrab::{ErrorKind, PlcValue};

    /// What a frame is, by the datasheet's layout.
    #[derive(Debug, PartialEq, Eq)]
    enum Class {
        Unavailable,
        Malformed,
        Reading {
            thermocouple: i64,
            cold: i64,
            flags: u8,
        },
    }

    fn bit(raw: u32, index: u32) -> bool {
        (raw >> index) & 1 == 1
    }

    /// A two's complement field of `bits` bits (`full` is `2^bits`, `half` is `2^(bits-1)`).
    fn signed(field: u32, full: i64, half: i64) -> i64 {
        let value = i64::from(field).rem_euclid(full);
        if value >= half {
            value.wrapping_sub(full)
        } else {
            value
        }
    }

    fn reference(raw: u32) -> Class {
        if raw == 0 || raw == u32::MAX {
            return Class::Unavailable;
        }
        if bit(raw, 17) || bit(raw, 3) {
            return Class::Malformed;
        }
        let flags =
            u8::from(bit(raw, 0)) | (u8::from(bit(raw, 1)) << 1) | (u8::from(bit(raw, 2)) << 2);
        if bit(raw, 16) != (flags != 0) {
            return Class::Malformed;
        }
        Class::Reading {
            thermocouple: signed(raw >> 18, 1 << 14, 1 << 13),
            cold: signed((raw >> 4) & 0xFFF, 1 << 12, 1 << 11),
            flags,
        }
    }

    /// Decode `raw` and check the outcome against the reference. Returns the reading if any.
    fn check(raw: u32) -> Option<ThermocoupleReading> {
        let result = ThermocoupleReading::from_raw(raw, Instant::now(), SystemTime::UNIX_EPOCH);
        match (reference(raw), result) {
            (Class::Unavailable, Err(error)) => {
                assert_eq!(error.kind(), ErrorKind::Disconnected, "{raw:#010x}");
                None
            }
            (Class::Malformed, Err(error)) => {
                assert_eq!(error.kind(), ErrorKind::MalformedData, "{raw:#010x}");
                None
            }
            (
                Class::Reading {
                    thermocouple,
                    cold,
                    flags,
                },
                Ok(reading),
            ) => {
                assert_eq!(reading.raw, raw, "frame kept {raw:#010x}");
                assert_eq!(
                    i64::from(reading.thermocouple_quarters),
                    thermocouple,
                    "{raw:#010x}"
                );
                assert_eq!(
                    i64::from(reading.cold_junction_sixteenths),
                    cold,
                    "{raw:#010x}"
                );
                assert_eq!(
                    reading.fault.map_or(0, ThermocoupleFault::code),
                    flags,
                    "{raw:#010x}"
                );
                assert_eq!(reading.fault.is_some(), flags != 0, "{raw:#010x}");
                Some(reading)
            }
            (class, result) => panic!("{raw:#010x}: reference {class:?}, library {result:?}"),
        }
    }

    /// The reading's invariants that hold for every accepted frame.
    fn check_reading(reading: &ThermocoupleReading) {
        let raw = reading.raw;
        assert!(
            (-8192..=8191).contains(&reading.thermocouple_quarters),
            "{raw:#010x}: 14 bits"
        );
        assert!(
            (-2048..=2047).contains(&reading.cold_junction_sixteenths),
            "{raw:#010x}: 12 bits"
        );
        assert_eq!(
            reading.cold_junction_celsius(),
            f64::from(reading.cold_junction_sixteenths) / 16.0,
            "{raw:#010x}: exact"
        );
        match reading.thermocouple_celsius() {
            Some(celsius) => {
                assert!(
                    reading.fault.is_none(),
                    "{raw:#010x}: a temperature only without a fault"
                );
                assert_eq!(
                    celsius,
                    f64::from(reading.thermocouple_quarters) / 4.0,
                    "exact"
                );
            }
            None => assert!(
                reading.fault.is_some(),
                "{raw:#010x}: no temperature means a fault"
            ),
        }
        let measurements = reading.measurements().expect("measurements");
        let [thermocouple, cold, fault] = measurements.as_slice() else {
            panic!(
                "{raw:#010x}: three measurements, not {}",
                measurements.len()
            );
        };
        let code = reading.fault.map_or(0, ThermocoupleFault::code);
        assert_eq!(thermocouple.name, "thermocouple_temperature", "order");
        assert_eq!(
            thermocouple.quality,
            if code == 0 {
                Quality::Good
            } else {
                Quality::Bad
            },
            "{raw:#010x}: a fault is bad"
        );
        assert_eq!(
            thermocouple.value.is_some(),
            code == 0,
            "{raw:#010x}: value only when good"
        );
        assert_eq!(cold.name, "cold_junction_temperature", "order");
        assert_eq!(
            cold.quality,
            Quality::Good,
            "{raw:#010x}: internal stays good"
        );
        assert_eq!(
            fault.value,
            Some(PlcValue::U8(code)),
            "{raw:#010x}: fault kept"
        );
        for measurement in &measurements {
            if let Some(PlcValue::F64(value)) = &measurement.value {
                assert!(value.is_finite(), "{raw:#010x}: finite");
            }
        }
    }

    /// Frame bits as the datasheet lays them out, from fields.
    fn encode(thermocouple: i16, cold: i16, flags: u8) -> u32 {
        let field14 = u32::from(u16::from_ne_bytes(thermocouple.to_ne_bytes())) & 0x3FFF;
        let field12 = u32::from(u16::from_ne_bytes(cold.to_ne_bytes())) & 0x0FFF;
        let fault = if flags == 0 { 0 } else { 1 << 16 };
        (field14 << 18) | fault | (field12 << 4) | u32::from(flags)
    }

    proptest! {
        // covers: SPEC mandatory-failure-cases / F10 direct sensors (every frame is classified)
        #[test]
        fn every_frame_is_classified_like_the_reference(raw in any::<u32>()) {
            if let Some(reading) = check(raw) {
                check_reading(&reading);
            }
        }

        // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (round trip)
        #[test]
        fn a_frame_built_from_fields_decodes_to_those_fields(
            thermocouple in -8192_i16..=8191,
            cold in -2048_i16..=2047,
            flags in 0_u8..8,
        ) {
            let raw = encode(thermocouple, cold, flags);
            prop_assume!(raw != 0);
            let reading = check(raw).expect("a frame built from valid fields is accepted");
            prop_assert_eq!(reading.thermocouple_quarters, thermocouple);
            prop_assert_eq!(reading.cold_junction_sixteenths, cold);
            check_reading(&reading);
        }

        #[test]
        fn setting_a_reserved_bit_always_refuses_the_frame(
            thermocouple in -8192_i16..=8191,
            cold in -2048_i16..=2047,
            flags in 0_u8..8,
            reserved in prop_oneof![Just(1_u32 << 17), Just(1_u32 << 3), Just((1_u32 << 17) | (1 << 3))],
        ) {
            let raw = encode(thermocouple, cold, flags) | reserved;
            prop_assume!(raw != u32::MAX);
            let error = ThermocoupleReading::from_raw(raw, Instant::now(), SystemTime::UNIX_EPOCH)
                .expect_err("a reserved bit is set");
            prop_assert_eq!(error.kind(), ErrorKind::MalformedData);
        }
    }

    /// Representative values of one half of a frame: edges of every field and each flag
    /// combination.
    const HALVES: [u32; 12] = [
        0x0000, 0x0001, 0x0002, 0x0003, 0x0004, 0x0007, 0x1900, 0x7FFF, 0x8000, 0xE700, 0xFFFE,
        0xFFFF,
    ];

    #[test]
    fn every_low_half_with_representative_high_halves_is_classified_like_the_reference() {
        for high in HALVES {
            for low in 0..=0xFFFF_u32 {
                if let Some(reading) = check((high << 16) | low) {
                    check_reading(&reading);
                }
            }
        }
    }

    #[test]
    fn every_high_half_with_representative_low_halves_is_classified_like_the_reference() {
        for low in HALVES {
            for high in 0..=0xFFFF_u32 {
                if let Some(reading) = check((high << 16) | low) {
                    check_reading(&reading);
                }
            }
        }
    }

    #[test]
    fn exactly_the_two_empty_frames_and_the_inconsistent_or_reserved_ones_are_refused() {
        let mut accepted = 0_u32;
        for high in 0..=0xFFFF_u32 {
            for low in [0x0000_u32, 0x0001, 0x0010, 0x1900, 0x0F0F, 0xFFF0] {
                let raw = (high << 16) | low;
                if ThermocoupleReading::from_raw(raw, Instant::now(), SystemTime::UNIX_EPOCH)
                    .is_ok()
                {
                    accepted = accepted.saturating_add(1);
                    assert!(
                        raw != 0 && raw != u32::MAX,
                        "{raw:#010x}: never the empty frames"
                    );
                    assert!(
                        !bit(raw, 17) && !bit(raw, 3),
                        "{raw:#010x}: reserved bits are clear"
                    );
                }
            }
        }
        assert!(accepted > 0, "some frames are accepted");
    }
}
