//! The MAX31855 adapter over the scripted SPI bus: frame decode, faults, refused frames,
//! the measurements a reading becomes, and the failures of the bus (ADR-0026).
//!
//! No bus, no converter and no host device is touched. The temperature frames follow the
//! datasheet's data formats (14-bit thermocouple field at 0.25 C a count and 12-bit internal
//! field at 0.0625 C a count, both two's complement); the bit patterns below are written out
//! by hand (the rows of the datasheet's Tables 4 and 5 as binary) so the decoder is not its
//! own oracle. Rows that are not in the datasheet are named as hand-computed counts.
//! Every wait runs on the fake clock.
#![cfg(all(feature = "boards", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime};

    use stonecrab::boards::{Max31855, SpiBus, ThermocoupleFault, ThermocoupleReading};
    use stonecrab::sensors::{Measurement, Quality, ValidationStatus};
    use stonecrab::testkit::{FakeClock, FakeSpiBus, SpiStep, block_on_with};
    use stonecrab::{
        Clock, Deadline, ErrorKind, FailureCategory, Operation, PlcError, PlcValue, Result,
    };

    /// The internal temperature field of +25.0 C, positioned in a frame (bits 15 to 4).
    const INTERNAL_25: u32 = 0x1900;

    fn decode(raw: u32) -> Result<ThermocoupleReading> {
        ThermocoupleReading::from_raw(raw, Instant::now(), SystemTime::UNIX_EPOCH)
    }

    fn decoded(raw: u32) -> ThermocoupleReading {
        decode(raw).expect("a valid frame")
    }

    fn refused(raw: u32) -> PlcError {
        decode(raw).expect_err("a refused frame")
    }

    /// A thermocouple field (14 bits) positioned in a frame (bits 31 to 18).
    const fn thermocouple_field(bits: u32) -> u32 {
        bits << 18
    }

    /// An internal temperature field (12 bits) positioned in a frame (bits 15 to 4).
    const fn internal_field(bits: u32) -> u32 {
        bits << 4
    }

    /// Decodes each (14-bit field, quarters, degrees C) with the internal temperature at 25 C.
    fn check_thermocouple_examples(examples: &[(u32, i16, f64)]) {
        for &(field, quarters, celsius) in examples {
            let reading = decoded(thermocouple_field(field) | INTERNAL_25);
            assert_eq!(
                reading.thermocouple_quarters, quarters,
                "count of {field:#06x}"
            );
            assert_eq!(
                reading.thermocouple_celsius(),
                Some(celsius),
                "degrees of {field:#06x}"
            );
            assert_eq!(reading.cold_junction_celsius(), 25.0, "internal stays 25 C");
            assert!(reading.fault.is_none(), "no fault in {field:#06x}");
        }
    }

    /// Decodes each (12-bit field, sixteenths, degrees C) with the thermocouple at 25 C.
    fn check_internal_examples(examples: &[(u32, i16, f64)]) {
        for &(field, sixteenths, celsius) in examples {
            // 25.0 C at the thermocouple, so a zero internal field is not an all-zeros frame.
            let reading = decoded(thermocouple_field(0x0064) | internal_field(field));
            assert_eq!(
                reading.cold_junction_sixteenths, sixteenths,
                "count of {field:#05x}"
            );
            assert_eq!(
                reading.cold_junction_celsius(),
                celsius,
                "degrees of {field:#05x}"
            );
            assert_eq!(
                reading.thermocouple_celsius(),
                Some(25.0),
                "thermocouple stays 25 C"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (thermocouple)
    #[test]
    fn the_datasheet_thermocouple_examples_decode_exactly() {
        // Table 4 of the datasheet, D[31:18]: (14-bit field, quarters, degrees C).
        check_thermocouple_examples(&[
            (0b01_1001_0000_0000, 6400, 1600.0),
            (0b00_1111_1010_0000, 4000, 1000.0),
            (0b00_0001_1001_0011, 403, 100.75),
            (0b00_0000_0110_0100, 100, 25.0),
            (0b00_0000_0000_0000, 0, 0.0),
            (0b11_1111_1111_1111, -1, -0.25),
            (0b11_1111_1111_1100, -4, -1.0),
            (0b11_1100_0001_1000, -1000, -250.0),
        ]);
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (cold junction)
    #[test]
    fn the_datasheet_internal_examples_decode_exactly() {
        // Table 5 of the datasheet, D[15:4]: (12-bit field, sixteenths, degrees C).
        check_internal_examples(&[
            (0b0111_1111_0000, 2032, 127.0),
            (0b0110_0100_1001, 1609, 100.5625),
            (0b0001_1001_0000, 400, 25.0),
            (0b0000_0000_0000, 0, 0.0),
            (0b1111_1111_1111, -1, -0.0625),
            (0b1111_1111_0000, -16, -1.0),
            (0b1110_1100_0000, -320, -20.0),
            (0b1100_1001_0000, -880, -55.0),
        ]);
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (thermocouple extremes)
    #[test]
    fn further_hand_computed_thermocouple_counts_and_the_extremes_decode_exactly() {
        // Not rows of the datasheet: counts worked out by hand, the extremes of the field
        // (0x2000 is the most negative, 0x1FFF the most positive) included.
        check_thermocouple_examples(&[
            (0x2000, -8192, -2048.0),
            (0x1FFF, 8191, 2047.75),
            (0x0FA3, 4003, 1000.75),
            (0x0191, 401, 100.25),
            (0x0001, 1, 0.25),
            (0x3BC8, -1080, -270.0),
        ]);
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (cold junction extremes)
    #[test]
    fn further_hand_computed_internal_counts_and_the_extremes_decode_exactly() {
        // Not rows of the datasheet: counts worked out by hand, the extremes of the field
        // (0x800 is the most negative, 0x7FF the most positive) included.
        check_internal_examples(&[
            (0x800, -2048, -128.0),
            (0x7FF, 2047, 127.9375),
            (0x001, 1, 0.0625),
            (0xE70, -400, -25.0),
        ]);
    }

    #[test]
    fn the_frame_is_kept_and_the_stamps_are_the_ones_given() {
        let (sampled_at, received_at) = (
            Instant::now(),
            SystemTime::UNIX_EPOCH + Duration::from_secs(9),
        );
        let reading = ThermocoupleReading::from_raw(0x3E8C_1900, sampled_at, received_at)
            .expect("a valid frame");
        assert_eq!(reading.raw, 0x3E8C_1900, "the frame as sent");
        assert_eq!(reading.sampled_at, sampled_at, "monotonic stamp");
        assert_eq!(reading.received_at, received_at, "wall stamp");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (open/short thermocouple flags)
    #[test]
    fn each_fault_flag_is_decoded_alone_and_the_internal_temperature_survives() {
        // (frame, open, short to ground, short to vcc, code)
        let cases = [
            (0x0001_1901_u32, true, false, false, 0b001_u8),
            (0x0001_1902, false, true, false, 0b010),
            (0x0001_1904, false, false, true, 0b100),
            (0x0001_1903, true, true, false, 0b011),
            (0x0001_1907, true, true, true, 0b111),
        ];
        for (frame, open, ground, vcc, code) in cases {
            let reading = decoded(frame);
            let fault = reading.fault.expect("the fault bit is set");
            assert_eq!(
                (fault.open, fault.short_to_ground, fault.short_to_vcc),
                (open, ground, vcc),
                "flags of {frame:#010x}"
            );
            assert_eq!(fault.code(), code, "code of {frame:#010x}");
            assert_eq!(
                reading.thermocouple_celsius(),
                None,
                "no temperature with a fault"
            );
            assert_eq!(reading.thermocouple_quarters, 0, "the bits are kept");
            assert_eq!(reading.cold_junction_celsius(), 25.0, "internal survives");
        }
    }

    #[test]
    fn the_thermocouple_bits_are_kept_under_a_fault_even_when_negative() {
        let reading = decoded(thermocouple_field(0x3FFF) | 0x0001_0000 | INTERNAL_25 | 0b001);
        assert_eq!(
            reading.thermocouple_quarters, -1,
            "bits kept, sign extended"
        );
        assert_eq!(
            reading.thermocouple_celsius(),
            None,
            "but not offered as a temperature"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid frame)
    #[test]
    fn a_reserved_bit_set_is_malformed_data_with_protocol_diagnostics() {
        for frame in [
            0x0002_1900_u32,
            0x0000_1908,
            0x0002_190C,
            0x0003_0000 | 0x1900,
        ] {
            let error = refused(frame);
            assert_eq!(
                error.kind(),
                ErrorKind::MalformedData,
                "frame {frame:#010x}"
            );
            let diagnostics = error.diagnostics();
            assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
            assert_eq!(
                diagnostics.category,
                Some(FailureCategory::Protocol),
                "category"
            );
            assert!(
                error.to_string().contains(&format!("{frame:#010x}")),
                "the message names the frame: {error}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid frame)
    #[test]
    fn a_fault_bit_that_disagrees_with_the_flags_is_malformed_data() {
        for (frame, what) in [
            (0x0001_1900_u32, "fault bit with no flag"),
            (0x0000_1901, "open flag without the fault bit"),
            (0x0000_1902, "ground flag without the fault bit"),
            (0x0000_1904, "vcc flag without the fault bit"),
        ] {
            let error = refused(frame);
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{what}");
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (open/short thermocouple flags)
    #[test]
    fn an_open_circuit_frame_as_the_datasheet_describes_it_is_a_bad_reading() {
        // T+ and T- unconnected: D31 is 0, D[30:18] are 1 (field 0x1FFF), the fault bit and OC
        // are set, and the internal temperature is still measured (25 C).
        let frame = 0x7FFD_1901;
        assert_eq!(frame >> 18, 0x1FFF, "the pattern of the datasheet");
        let reading = decoded(frame);
        assert_eq!(reading.thermocouple_quarters, 8191, "the bits are kept");
        assert_eq!(reading.thermocouple_celsius(), None, "not a temperature");
        assert_eq!(reading.fault.map(ThermocoupleFault::code), Some(1), "OC");
        assert_eq!(reading.cold_junction_celsius(), 25.0, "internal survives");

        let measurements = reading.measurements().expect("measurements");
        let thermocouple = by_name(&measurements, "thermocouple_temperature");
        assert_eq!(thermocouple.quality, Quality::Bad, "bad");
        assert_eq!(thermocouple.value, None, "no value to misread");
        assert_eq!(thermocouple.raw, Some(PlcValue::I16(8191)), "the bits");
        let fault = by_name(&measurements, "thermocouple_fault");
        assert_eq!(fault.value, Some(PlcValue::U8(1)), "fault code 1");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (unavailable device)
    #[test]
    fn all_zeros_and_all_ones_are_a_device_that_did_not_answer() {
        for frame in [0x0000_0000_u32, 0xFFFF_FFFF] {
            let error = refused(frame);
            assert_eq!(error.kind(), ErrorKind::Disconnected, "frame {frame:#010x}");
            let diagnostics = error.diagnostics();
            assert_eq!(
                diagnostics.category,
                Some(FailureCategory::Transport),
                "category"
            );
            assert_eq!(diagnostics.operation, Some(Operation::Read), "operation");
        }
    }

    #[test]
    fn a_frame_that_is_nearly_all_zeros_or_all_ones_is_still_read() {
        // 0.25 C and 0.0625 C: the smallest non-zero frame.
        let smallest = decoded(0x0004_0010);
        assert_eq!(smallest.thermocouple_celsius(), Some(0.25), "thermocouple");
        assert_eq!(smallest.cold_junction_celsius(), 0.0625, "cold junction");
        // -0.25 C and -0.0625 C with every defined bit set but the reserved ones.
        let all_defined = decoded(0xFFFD_FFF7);
        assert_eq!(all_defined.thermocouple_quarters, -1, "thermocouple count");
        assert_eq!(
            all_defined.cold_junction_sixteenths, -1,
            "cold junction count"
        );
        assert_eq!(
            all_defined.fault.map(ThermocoupleFault::code),
            Some(0b111),
            "flags"
        );
    }

    fn by_name<'a>(measurements: &'a [Measurement], name: &str) -> &'a Measurement {
        measurements
            .iter()
            .find(|measurement| measurement.name == name)
            .expect("the measurement is there")
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (measurements)
    #[test]
    fn a_good_reading_becomes_three_good_measurements_with_units_and_counts() {
        let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mono = Instant::now();
        // -250.00 C at the thermocouple, -25.0 C inside the chip.
        let reading = ThermocoupleReading::from_raw(0xF060_E700, mono, wall).expect("valid");
        let measurements = reading.measurements().expect("measurements");
        let names: Vec<&str> = measurements.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "thermocouple_temperature",
                "cold_junction_temperature",
                "thermocouple_fault"
            ],
            "names and order"
        );

        let [thermocouple, cold, fault] = measurements.as_slice() else {
            panic!("three measurements, not {}", measurements.len());
        };
        assert_eq!(
            thermocouple.value,
            Some(PlcValue::F64(-250.0)),
            "negative preserved"
        );
        assert_eq!(thermocouple.raw, Some(PlcValue::I16(-1000)), "quarters");
        assert_eq!(
            (thermocouple.unit.as_deref(), thermocouple.quality),
            (Some("Cel"), Quality::Good)
        );

        assert_eq!(cold.value, Some(PlcValue::F64(-25.0)), "negative preserved");
        assert_eq!(cold.raw, Some(PlcValue::I16(-400)), "sixteenths");
        assert_eq!(
            (cold.unit.as_deref(), cold.quality),
            (Some("Cel"), Quality::Good)
        );

        assert_eq!(fault.value, Some(PlcValue::U8(0)), "no fault is explicit");
        assert_eq!(
            (fault.unit.as_deref(), fault.quality),
            (Some("1"), Quality::Good)
        );

        for measurement in &measurements {
            assert_eq!(
                measurement.tag.as_str(),
                measurement.name,
                "tag is the name"
            );
            assert_eq!(measurement.origin, ValidationStatus::Documented, "origin");
            assert_eq!(measurement.received_mono, mono, "monotonic stamp");
            assert_eq!(measurement.received_at, wall, "wall stamp");
            assert!(
                measurement.source_time.is_none(),
                "the converter stamps nothing"
            );
            assert!(measurement.protocol_status.is_none(), "no protocol status");
            assert!(measurement.quality_before_stale.is_none(), "not stale");
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (open/short thermocouple flags)
    #[test]
    fn a_fault_makes_the_thermocouple_bad_and_keeps_the_fault_and_the_cold_junction() {
        // (frame, code)
        for (frame, code) in [(0x0001_1901_u32, 1_u8), (0x0001_1902, 2), (0x0001_1904, 4)] {
            let measurements = decoded(frame).measurements().expect("measurements");
            let thermocouple = by_name(&measurements, "thermocouple_temperature");
            assert_eq!(thermocouple.quality, Quality::Bad, "bad with {frame:#010x}");
            assert_eq!(thermocouple.value, None, "no value to misread");
            assert_eq!(
                thermocouple.raw,
                Some(PlcValue::I16(0)),
                "the bits are kept"
            );
            let cold = by_name(&measurements, "cold_junction_temperature");
            assert_eq!(cold.quality, Quality::Good, "internal stays usable");
            assert_eq!(cold.value, Some(PlcValue::F64(25.0)), "internal value");
            let fault = by_name(&measurements, "thermocouple_fault");
            assert_eq!(
                fault.value,
                Some(PlcValue::U8(code)),
                "the fault is visible"
            );
            assert_eq!(fault.raw, Some(PlcValue::U8(code)), "raw code");
            assert_eq!(fault.quality, Quality::Good, "the report itself is sound");
        }
    }

    struct Rig {
        clock: FakeClock,
        bus: FakeSpiBus,
        sensor: Max31855,
    }

    fn rig(steps: Vec<SpiStep>) -> Rig {
        let clock = FakeClock::default();
        let bus = FakeSpiBus::new(steps, clock.clone());
        let sensor = Max31855::new(Box::new(bus.clone()), Arc::new(clock.clone()));
        Rig { clock, bus, sensor }
    }

    impl Rig {
        fn deadline(&self) -> Deadline {
            Deadline::after(self.clock.now(), Duration::from_secs(1)).expect("representable")
        }

        fn read(&mut self) -> Result<ThermocoupleReading> {
            let deadline = self.deadline();
            block_on_with(&self.clock, self.sensor.read(deadline))
        }
    }

    #[test]
    fn a_read_asks_for_four_bytes_and_stamps_the_reading_from_the_clock() {
        let mut rig = rig(vec![SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00])]);
        rig.clock.advance(Duration::from_secs(5));
        let reading = rig.read().expect("a reading");
        assert_eq!(reading.raw, 0x3E8C_1900, "big-endian frame");
        assert_eq!(reading.thermocouple_celsius(), Some(1000.75), "temperature");
        assert_eq!(reading.sampled_at, rig.clock.now(), "monotonic stamp");
        assert_eq!(reading.received_at, rig.clock.wall_now(), "wall stamp");
        assert_eq!(rig.bus.reads(), [4], "one four-byte transfer");
    }

    #[test]
    fn a_fault_is_a_reading_not_an_error() {
        let mut rig = rig(vec![SpiStep::Reply(vec![0x00, 0x01, 0x19, 0x04])]);
        let reading = rig.read().expect("a reading");
        assert!(
            reading.fault.is_some_and(|fault| fault.short_to_vcc),
            "short to vcc"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI failure)
    #[test]
    fn a_bus_failure_keeps_its_kind_and_the_next_read_goes_on() {
        let mut rig = rig(vec![
            SpiStep::Fail(ErrorKind::Disconnected),
            SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00]),
        ]);
        let error = rig.read().expect_err("the bus failed");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind kept");
        assert!(rig.read().is_ok(), "the next frame reads");
        assert_eq!(rig.bus.reads(), [4, 4], "two transfers");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI timeout)
    #[test]
    fn a_silent_bus_times_out_at_the_deadline_and_a_later_frame_still_reads() {
        let mut rig = rig(vec![
            SpiStep::Silence,
            SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00]),
        ]);
        let started = rig.clock.now();
        let deadline = rig.deadline();
        let error = block_on_with(&rig.clock, rig.sensor.read(deadline)).expect_err("silent");
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        assert_eq!(
            rig.clock.now(),
            deadline.instant(),
            "waited exactly to the deadline"
        );
        assert_eq!(
            rig.clock.now() - started,
            Duration::from_secs(1),
            "one second of fake time"
        );
        assert!(rig.read().is_ok(), "the next frame reads");
    }

    #[test]
    fn an_elapsed_deadline_times_out_before_the_bus_is_touched() {
        let mut rig = rig(vec![SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00])]);
        let deadline = Deadline::at(rig.clock.now());
        let error = block_on_with(&rig.clock, rig.sensor.read(deadline)).expect_err("elapsed");
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        assert!(rig.bus.reads().is_empty(), "no transfer was started");
        assert!(rig.read().is_ok(), "and the frame is still there");
    }

    #[test]
    fn a_spent_script_is_a_bus_that_no_longer_answers() {
        let mut rig = rig(Vec::new());
        let error = rig.read().expect_err("nothing scripted");
        assert_eq!(error.kind(), ErrorKind::Timeout, "silence is a timeout");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (SPI failure, short frame)
    #[test]
    fn a_transfer_of_the_wrong_length_is_malformed_data() {
        for bytes in [
            vec![],
            vec![0x3E, 0x8C, 0x19],
            vec![0x3E, 0x8C, 0x19, 0x00, 0x00],
        ] {
            let length = bytes.len();
            let mut rig = rig(vec![SpiStep::Reply(bytes)]);
            let error = rig.read().expect_err("wrong length");
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{length} bytes");
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Protocol),
                "category"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (unavailable device)
    #[test]
    fn a_bus_with_no_device_on_it_is_disconnected_whatever_it_floats_to() {
        for bytes in [vec![0xFF; 4], vec![0x00; 4]] {
            let mut rig = rig(vec![SpiStep::Reply(bytes)]);
            let error = rig.read().expect_err("no device");
            assert_eq!(error.kind(), ErrorKind::Disconnected, "no answer");
        }
    }

    #[test]
    fn the_adapter_and_its_readings_move_between_threads_and_print() {
        fn send<T: Send>() {}
        send::<Max31855>();
        send::<ThermocoupleReading>();
        send::<Box<dyn SpiBus>>();
        let rig = rig(Vec::new());
        assert_eq!(
            format!("{:?}", rig.sensor),
            "Max31855 { .. }",
            "no bus internals shown"
        );
    }
}
