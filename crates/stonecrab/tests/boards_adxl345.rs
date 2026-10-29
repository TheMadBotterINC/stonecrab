//! The ADXL345 adapter over the scripted I2C bus: identity, the configuration writes, the
//! sample decode at every range and resolution, FIFO drains, overflow and loss accounting,
//! sequence numbers, and the failures of the bus (ADR-0029).
//!
//! No bus, no accelerometer and no host device is touched. The register addresses, bit
//! layouts and sensitivities follow the datasheet (Analog Devices ADXL345, Rev. 0, pinned by
//! sha256 in ADR-0029, not committed): registers 0x00 (`DEVID`, 0xE5), 0x1E to 0x20 (offsets),
//! 0x2C (`BW_RATE`), 0x2D (`POWER_CTL`), 0x30 (`INT_SOURCE`), 0x31 (`DATA_FORMAT`), 0x32 to 0x37
//! (data, low byte first), 0x38 (`FIFO_CTL`) and 0x39 (`FIFO_STATUS`); 256, 128, 64 and 32 LSB/g
//! in 10-bit mode at plus or minus 2, 4, 8 and 16 g (Table 1), and at full resolution 10 to 13
//! bits (Table 1) at 256 LSB/g (the `DATA_FORMAT` `FULL_RES` description, "4 mg/LSB"). The register bytes below are written out by hand so the decoder is not
//! its own oracle. Every wait runs on the fake clock.
#![cfg(all(feature = "boards", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::{Duration, SystemTime};

    use stonecrab::boards::{
        AccelRange, AccelReading, Adxl345, Adxl345Config, FifoMode, I2cBus, OutputRate, SampleBlock,
    };
    use stonecrab::sensors::{Quality, ValidationStatus};
    use stonecrab::testkit::{
        FakeClock, FakeI2cBus, I2cStep, I2cTransfer, block_on_with, poll_once,
    };
    use stonecrab::{Clock, Deadline, ErrorKind, FailureCategory, PlcError, PlcValue};

    /// The default address (SDO/ALT ADDRESS grounded).
    const ADDRESS: u8 = 0x53;
    /// The transactions of a good `init`.
    const INIT: usize = 9;

    /// A sensor on a fresh bus, with the bus handle and the clock.
    struct Bench {
        clock: FakeClock,
        bus: FakeI2cBus,
        sensor: Adxl345,
    }

    impl Bench {
        fn new(config: Adxl345Config, script: Vec<I2cStep>) -> Self {
            let clock = FakeClock::default();
            let bus = FakeI2cBus::new(script, clock.clone());
            let sensor = Adxl345::new(Box::new(bus.clone()), Arc::new(clock.clone()), config);
            Self { clock, bus, sensor }
        }

        /// A sensor whose script starts with a good `init` and goes on with `rest`, already
        /// initialized.
        fn ready(config: Adxl345Config, rest: Vec<I2cStep>) -> Self {
            let mut script = init_steps();
            script.extend(rest);
            let mut bench = Self::new(config, script);
            let deadline = bench.deadline();
            block_on_with(&bench.clock, bench.sensor.init(deadline)).expect("init");
            bench
        }

        fn deadline(&self) -> Deadline {
            Deadline::after(self.clock.now(), Duration::from_secs(1)).expect("deadline")
        }

        fn init(&mut self) -> Result<(), PlcError> {
            let deadline = self.deadline();
            block_on_with(&self.clock, self.sensor.init(deadline))
        }

        fn read_fifo(&mut self) -> Result<SampleBlock, PlcError> {
            let deadline = self.deadline();
            block_on_with(&self.clock, self.sensor.read_fifo(deadline))
        }

        fn read_sample(&mut self) -> Result<Option<AccelReading>, PlcError> {
            let deadline = self.deadline();
            block_on_with(&self.clock, self.sensor.read_sample(deadline))
        }

        fn transfers_since(&self, start: usize) -> Vec<I2cTransfer> {
            self.bus.transfers().split_off(start)
        }
    }

    /// The answers to a good `init`: `DEVID`, two acknowledged writes, the data registers
    /// (read and discarded; any bytes do), then five acknowledged writes.
    fn init_steps() -> Vec<I2cStep> {
        let mut steps = vec![
            I2cStep::Reply(vec![0xE5]),
            I2cStep::Ack,
            I2cStep::Ack,
            I2cStep::Reply(vec![0xFF, 0x7F, 0x00, 0x80, 0x12, 0x34]),
        ];
        steps.extend(std::iter::repeat_n(I2cStep::Ack, 5));
        steps
    }

    /// The answers to a `read_sample` that finds a new sample: `INT_SOURCE` with `DATA_READY`
    /// set, then the six data registers.
    fn new_sample(data: Vec<u8>) -> Vec<I2cStep> {
        vec![I2cStep::Reply(vec![0x80]), I2cStep::Reply(data)]
    }

    fn read(address: u8, register: u8, len: usize) -> I2cTransfer {
        I2cTransfer {
            address,
            write: vec![register],
            read: len,
        }
    }

    fn write(address: u8, bytes: &[u8]) -> I2cTransfer {
        I2cTransfer {
            address,
            write: bytes.to_vec(),
            read: 0,
        }
    }

    /// The transactions of `init`, given the three configuration bytes it writes.
    fn init_transfers(address: u8, data_format: u8, bw_rate: u8, fifo_ctl: u8) -> Vec<I2cTransfer> {
        vec![
            read(address, 0x00, 1),
            write(address, &[0x2D, 0x00]),
            write(address, &[0x38, 0x00]),
            read(address, 0x32, 6),
            write(address, &[0x1E, 0x00, 0x00, 0x00]),
            write(address, &[0x31, data_format]),
            write(address, &[0x2C, bw_rate]),
            write(address, &[0x38, fifo_ctl]),
            write(address, &[0x2D, 0x08]),
        ]
    }

    fn config(range: AccelRange, full_resolution: bool, fifo: FifoMode) -> Adxl345Config {
        let mut config = Adxl345Config::default();
        config.range = range;
        config.full_resolution = full_resolution;
        config.fifo = fifo;
        config
    }

    fn bypass() -> Adxl345Config {
        config(AccelRange::G2, true, FifoMode::Bypass)
    }

    fn assert_refused_without_io(bench: &mut Bench, kind: ErrorKind) {
        let before = bench.bus.transfers().len();
        let error = bench.read_fifo().expect_err("refused");
        assert_eq!(error.kind(), kind, "{error}");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation),
            "refused before any I/O"
        );
        assert_eq!(bench.bus.transfers().len(), before, "nothing was sent");
    }

    #[test]
    fn init_checks_the_identity_then_writes_the_default_configuration_in_order() {
        let mut bench = Bench::new(Adxl345Config::default(), init_steps());
        bench.init().expect("init");
        // Full resolution at 2 g (0x08), 100 Hz (0x0A), stream mode (0x80).
        assert_eq!(
            bench.bus.transfers(),
            init_transfers(ADDRESS, 0x08, 0x0A, 0x80)
        );
    }

    #[test]
    fn init_writes_every_range_resolution_rate_and_fifo_mode_as_configured() {
        let ranges = [
            (AccelRange::G2, 0b00),
            (AccelRange::G4, 0b01),
            (AccelRange::G8, 0b10),
            (AccelRange::G16, 0b11),
        ];
        let rates = [
            (OutputRate::Hz3200, 0x0F),
            (OutputRate::Hz1600, 0x0E),
            (OutputRate::Hz800, 0x0D),
            (OutputRate::Hz400, 0x0C),
            (OutputRate::Hz200, 0x0B),
            (OutputRate::Hz100, 0x0A),
            (OutputRate::Hz50, 0x09),
            (OutputRate::Hz25, 0x08),
            (OutputRate::Hz12_5, 0x07),
            (OutputRate::Hz6_25, 0x06),
        ];
        let modes = [
            (FifoMode::Bypass, 0x00),
            (FifoMode::Fifo, 0x40),
            (FifoMode::Stream, 0x80),
        ];
        for (range, range_bits) in ranges {
            for (full_resolution, full_res_bit) in [(false, 0x00), (true, 0x08)] {
                for (rate, rate_code) in rates {
                    for (fifo, fifo_bits) in modes {
                        let mut config = config(range, full_resolution, fifo);
                        config.rate = rate;
                        config.address = 0x1D;
                        let mut bench = Bench::new(config, init_steps());
                        bench.init().expect("init");
                        assert_eq!(
                            bench.bus.transfers(),
                            init_transfers(0x1D, full_res_bit | range_bits, rate_code, fifo_bits),
                            "{config:?}"
                        );
                    }
                }
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid identity)
    #[test]
    fn a_wrong_devid_is_an_invalid_identity_and_nothing_is_written() {
        for devid in [0x00, 0xE6, 0xFF, 0x1D] {
            let mut bench = Bench::new(Adxl345Config::default(), vec![I2cStep::Reply(vec![devid])]);
            let error = bench.init().expect_err("not an ADXL345");
            assert_eq!(error.kind(), ErrorKind::Trust, "{devid:#04x}");
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Protocol)
            );
            assert!(
                error.to_string().contains(&format!("{devid:#04x}")),
                "the message names the byte: {error}"
            );
            assert_eq!(
                bench.bus.transfers(),
                vec![read(ADDRESS, 0x00, 1)],
                "only DEVID"
            );
            assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid identity)
    #[test]
    fn verify_identity_alone_reads_devid_and_starts_no_stream() {
        let mut bench = Bench::new(
            Adxl345Config::default(),
            vec![I2cStep::Reply(vec![0xE5]), I2cStep::Reply(vec![0xE4])],
        );
        let deadline = bench.deadline();
        block_on_with(&bench.clock, bench.sensor.verify_identity(deadline)).expect("an ADXL345");
        let error = block_on_with(&bench.clock, bench.sensor.verify_identity(deadline))
            .expect_err("DEVID 0xe4");
        assert_eq!(error.kind(), ErrorKind::Trust);
        assert_eq!(
            bench.bus.transfers(),
            vec![read(ADDRESS, 0x00, 1), read(ADDRESS, 0x00, 1)]
        );
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (unavailable device)
    #[test]
    fn a_device_that_does_not_acknowledge_is_disconnected() {
        let mut bench = Bench::new(
            Adxl345Config::default(),
            vec![I2cStep::Fail(ErrorKind::Disconnected)],
        );
        let error = bench.init().expect_err("no acknowledge");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert_eq!(
            bench.bus.transfers().len(),
            1,
            "only the DEVID read was tried"
        );
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C failure)
    #[test]
    fn a_bus_error_during_the_configuration_writes_fails_init_until_a_new_init_succeeds() {
        let mut script = init_steps();
        script.truncate(5);
        script.push(I2cStep::Fail(ErrorKind::Disconnected));
        script.extend(init_steps());
        script.push(I2cStep::Reply(vec![0x00]));
        script.push(I2cStep::Reply(vec![0x00]));
        let mut bench = Bench::new(Adxl345Config::default(), script);
        let error = bench.init().expect_err("the fourth write fails");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert_eq!(
            bench.bus.transfers().len(),
            6,
            "DEVID, two writes, the data registers, the offsets, the failed one"
        );
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);

        bench.init().expect("the device answers again");
        let block = bench.read_fifo().expect("a block");
        assert_eq!((block.sequence, block.samples.len()), (0, 0));
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C timeout)
    #[test]
    fn a_silent_device_times_out_at_the_deadline() {
        let mut bench = Bench::new(Adxl345Config::default(), vec![I2cStep::Silence]);
        let start = bench.clock.now();
        let error = bench.init().expect_err("silent");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_eq!(
            bench.clock.now().duration_since(start),
            Duration::from_secs(1),
            "it waited out the deadline on the fake clock"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C timeout)
    #[test]
    fn an_elapsed_deadline_times_out_before_any_transaction() {
        let mut bench = Bench::new(Adxl345Config::default(), init_steps());
        let deadline =
            Deadline::after(bench.clock.now(), Duration::from_millis(5)).expect("deadline");
        bench.clock.advance(Duration::from_millis(5));
        let error = block_on_with(&bench.clock, bench.sensor.init(deadline)).expect_err("late");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert!(bench.bus.transfers().is_empty(), "nothing was sent");
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C failure, short reply)
    #[test]
    fn a_reply_of_the_wrong_length_is_malformed() {
        let mut bench = Bench::new(
            Adxl345Config::default(),
            vec![I2cStep::Reply(vec![0xE5, 0xE5])],
        );
        let error = bench.init().expect_err("two bytes for DEVID");
        assert_eq!(error.kind(), ErrorKind::MalformedData);

        let mut bench = Bench::new(
            Adxl345Config::default(),
            vec![
                I2cStep::Reply(vec![0xE5]),
                I2cStep::Ack,
                I2cStep::Ack,
                I2cStep::Reply(vec![0; 5]),
            ],
        );
        let error = bench
            .init()
            .expect_err("five bytes from the data registers");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        assert_eq!(bench.bus.transfers().len(), 4, "nothing written after it");
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);

        let mut bench = Bench::ready(bypass(), new_sample(vec![0; 5]));
        let error = bench.read_sample().expect_err("five data bytes");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        assert!(error.to_string().contains("returned 5 bytes"), "{error}");

        let mut bench = Bench::ready(bypass(), vec![I2cStep::Reply(vec![0x80, 0x80])]);
        let error = bench.read_sample().expect_err("two bytes for INT_SOURCE");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        assert_eq!(bench.transfers_since(INIT).len(), 1, "no data read");
    }

    #[test]
    fn reads_before_init_are_refused_without_io() {
        let mut bench = Bench::new(bypass(), Vec::new());
        let error = bench.read_sample().expect_err("no init");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert!(bench.bus.transfers().is_empty(), "nothing was sent");
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
    }

    #[test]
    fn each_read_is_refused_in_the_fifo_mode_that_does_not_suit_it_and_the_stream_goes_on() {
        let mut bench = Bench::ready(bypass(), new_sample(vec![0, 0, 0, 0, 0x00, 0x01]));
        assert_refused_without_io(&mut bench, ErrorKind::InvalidConfig);
        let reading = bench
            .read_sample()
            .expect("still ready")
            .expect("a new sample");
        assert_eq!(reading.sample.z, 256);

        let mut bench = Bench::ready(Adxl345Config::default(), Vec::new());
        let error = bench.read_sample().expect_err("the FIFO is on");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Validation)
        );
        assert_eq!(
            bench.bus.transfers().len(),
            INIT,
            "only init's transactions"
        );
    }

    /// (range, full resolution, data registers, counts, counts per g, g).
    type Example = (AccelRange, bool, [u8; 6], [i16; 3], u16, [f64; 3]);

    fn check_examples(examples: &[Example]) {
        for &(range, full_resolution, data, counts, counts_per_g, g) in examples {
            let config = config(range, full_resolution, FifoMode::Bypass);
            let mut bench = Bench::ready(config, new_sample(data.to_vec()));
            let reading = bench
                .read_sample()
                .unwrap_or_else(|error| panic!("{range:?} {full_resolution} {data:02x?}: {error}"))
                .expect("a new sample");
            let sample = reading.sample;
            assert_eq!(
                [sample.x, sample.y, sample.z],
                counts,
                "{range:?} {data:02x?}"
            );
            assert_eq!(
                reading.counts_per_g, counts_per_g,
                "{range:?} {full_resolution}"
            );
            let values: Vec<_> = reading
                .measurements()
                .expect("measurements")
                .into_iter()
                .map(|measurement| measurement.value)
                .collect();
            let expected: Vec<_> = g.iter().map(|&g| Some(PlcValue::F64(g))).collect();
            assert_eq!(values, expected, "{range:?} {full_resolution} {data:02x?}");
            assert_eq!(
                bench.transfers_since(INIT),
                vec![read(ADDRESS, 0x30, 1), read(ADDRESS, 0x32, 6)],
                "INT_SOURCE, then one six-byte read of DATAX0 to DATAZ1"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (10-bit mode)
    #[test]
    fn ten_bit_samples_decode_at_the_datasheet_sensitivity_of_each_range() {
        // Table 1: 256, 128, 64 and 32 LSB/g; +1 g on x, -1 g on y, the extremes of 10 bits
        // (511 and -512) on z.
        check_examples(&[
            (
                AccelRange::G2,
                false,
                [0x00, 0x01, 0x00, 0xFF, 0xFF, 0x01],
                [256, -256, 511],
                256,
                [1.0, -1.0, 1.996_093_75],
            ),
            (
                AccelRange::G4,
                false,
                [0x80, 0x00, 0x80, 0xFF, 0x00, 0xFE],
                [128, -128, -512],
                128,
                [1.0, -1.0, -4.0],
            ),
            (
                AccelRange::G8,
                false,
                [0x40, 0x00, 0xC0, 0xFF, 0xFF, 0x01],
                [64, -64, 511],
                64,
                [1.0, -1.0, 7.984_375],
            ),
            (
                AccelRange::G16,
                false,
                [0x20, 0x00, 0xE0, 0xFF, 0x00, 0xFE],
                [32, -32, -512],
                32,
                [1.0, -1.0, -16.0],
            ),
        ]);
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values (full resolution)
    #[test]
    fn full_resolution_samples_decode_at_256_per_g_with_the_width_of_each_range() {
        // Table 1: full resolution keeps 256 LSB/g with 10, 11, 12 and 13 bits; +1 g on x,
        // the largest count on y and the most negative on z.
        check_examples(&[
            (
                AccelRange::G2,
                true,
                [0x00, 0x01, 0xFF, 0x01, 0x00, 0xFE],
                [256, 511, -512],
                256,
                [1.0, 1.996_093_75, -2.0],
            ),
            (
                AccelRange::G4,
                true,
                [0x00, 0x01, 0xFF, 0x03, 0x00, 0xFC],
                [256, 1023, -1024],
                256,
                [1.0, 3.996_093_75, -4.0],
            ),
            (
                AccelRange::G8,
                true,
                [0x00, 0x01, 0xFF, 0x07, 0x00, 0xF8],
                [256, 2047, -2048],
                256,
                [1.0, 7.996_093_75, -8.0],
            ),
            (
                AccelRange::G16,
                true,
                [0x00, 0x01, 0xFF, 0x0F, 0x00, 0xF0],
                [256, 4095, -4096],
                256,
                [1.0, 15.996_093_75, -16.0],
            ),
        ]);
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid sample)
    #[test]
    fn a_count_one_past_the_resolution_is_malformed_and_ends_the_stream() {
        // (range, full resolution, a count one past each end, as the x pair).
        let cases = [
            (AccelRange::G2, false, [0x00, 0x02]),
            (AccelRange::G16, false, [0xFF, 0xFD]),
            (AccelRange::G2, true, [0xFF, 0xFD]),
            (AccelRange::G4, true, [0x00, 0x04]),
            (AccelRange::G4, true, [0xFF, 0xFB]),
            (AccelRange::G8, true, [0x00, 0x08]),
            (AccelRange::G8, true, [0xFF, 0xF7]),
            (AccelRange::G16, true, [0x00, 0x10]),
            (AccelRange::G16, true, [0xFF, 0xEF]),
        ];
        for (range, full_resolution, [low, high]) in cases {
            let config = config(range, full_resolution, FifoMode::Bypass);
            let mut bench = Bench::ready(config, new_sample(vec![0, 0, low, high, 0, 0]));
            let error = bench.read_sample().expect_err("outside the resolution");
            assert_eq!(
                error.kind(),
                ErrorKind::MalformedData,
                "{range:?} {full_resolution}"
            );
            assert!(
                error.to_string().contains("y count"),
                "names the axis: {error}"
            );
            let error = bench.read_sample().expect_err("the stream ended");
            assert_eq!(error.kind(), ErrorKind::Disconnected);
        }
    }

    #[test]
    fn a_reading_becomes_three_good_measurements_in_g_with_counts_and_stamps() {
        let mut bench = Bench::ready(
            config(AccelRange::G4, false, FifoMode::Bypass),
            // x = -0.5 g, y = +0.25 g, z = +1 g at 128 counts per g.
            new_sample(vec![0xC0, 0xFF, 0x20, 0x00, 0x80, 0x00]),
        );
        let wall = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
        bench.clock.advance(Duration::from_millis(7));
        bench.clock.set_wall_now(wall);
        let sampled = bench.clock.now();
        let reading = bench
            .read_sample()
            .expect("a reading")
            .expect("a new sample");
        assert_eq!((reading.sampled_at, reading.received_at), (sampled, wall));
        let measurements = reading.measurements().expect("measurements");
        let expected = [
            ("acceleration_x", -0.5, -64),
            ("acceleration_y", 0.25, 32),
            ("acceleration_z", 1.0, 128),
        ];
        assert_eq!(measurements.len(), 3);
        for (measurement, (name, g, count)) in measurements.iter().zip(expected) {
            assert_eq!(measurement.name, name);
            assert_eq!(measurement.tag.as_str(), name);
            assert_eq!(measurement.unit, "[g]");
            assert_eq!(measurement.value, Some(PlcValue::F64(g)), "{name}");
            assert_eq!(measurement.raw, Some(PlcValue::I16(count)), "{name}");
            assert_eq!(measurement.quality, Quality::Good);
            assert_eq!(measurement.origin, ValidationStatus::Documented);
            assert_eq!(measurement.received_mono, sampled);
            assert_eq!(measurement.received_at, wall);
            assert_eq!(measurement.source_time, None);
            assert_eq!(measurement.protocol_status, None);
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (no new sample)
    #[test]
    fn a_read_without_a_new_sample_returns_none_without_reading_the_data_and_the_stream_goes_on() {
        // Straight after `init` the device is still turning on (about one output period plus
        // 1.1 ms) and its data registers hold reset zeros; once a sample is read, the next read
        // before the next output period would repeat it. `DATA_READY` is clear in both.
        let mut script = vec![I2cStep::Reply(vec![0x00])];
        // `DATA_READY`, and the overrun flag, which in bypass mode only says that a sample
        // was replaced unread.
        script.push(I2cStep::Reply(vec![0x81]));
        script.push(I2cStep::Reply(vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01]));
        script.push(I2cStep::Reply(vec![0x02])); // the watermark bit alone: no new sample
        let mut bench = Bench::ready(bypass(), script);
        assert_eq!(
            bench.read_sample().expect("still ready"),
            None,
            "turning on"
        );
        let reading = bench
            .read_sample()
            .expect("still ready")
            .expect("a new sample");
        assert_eq!(reading.sample.z, 256);
        assert_eq!(bench.read_sample().expect("still ready"), None, "no repeat");
        assert_eq!(
            bench.transfers_since(INIT),
            vec![
                read(ADDRESS, 0x30, 1),
                read(ADDRESS, 0x30, 1),
                read(ADDRESS, 0x32, 6),
                read(ADDRESS, 0x30, 1),
            ],
            "the data registers are read only with DATA_READY set"
        );
    }

    #[test]
    fn a_drain_reads_the_overrun_flag_the_level_and_each_entry_in_one_six_byte_read() {
        let mut bench = Bench::ready(
            Adxl345Config::default(),
            vec![
                I2cStep::Reply(vec![0x82]), // DATA_READY and watermark, no overrun
                I2cStep::Reply(vec![0x03]), // three entries
                I2cStep::Reply(vec![0x00, 0x01, 0x00, 0x00, 0x00, 0x00]),
                I2cStep::Reply(vec![0x00, 0x00, 0x00, 0xFF, 0x00, 0x00]),
                I2cStep::Reply(vec![0x00, 0x00, 0x00, 0x00, 0x01, 0x00]),
            ],
        );
        bench.clock.advance(Duration::from_millis(30));
        let drained = bench.clock.now();
        let block = bench.read_fifo().expect("a block");
        assert_eq!(block.sequence, 0);
        assert_eq!(block.interval, Duration::from_millis(10));
        assert_eq!(block.counts_per_g, 256);
        let counts: Vec<_> = block.samples.iter().map(|s| [s.x, s.y, s.z]).collect();
        assert_eq!(counts, vec![[256, 0, 0], [0, -256, 0], [0, 0, 1]]);
        assert!(!block.overflowed);
        assert_eq!(block.lost, 0);
        assert_eq!(block.sampled_at, drained);
        assert_eq!(
            bench.transfers_since(INIT),
            vec![
                read(ADDRESS, 0x30, 1),
                read(ADDRESS, 0x39, 1),
                read(ADDRESS, 0x32, 6),
                read(ADDRESS, 0x32, 6),
                read(ADDRESS, 0x32, 6),
            ]
        );
    }

    #[test]
    fn an_empty_fifo_gives_an_empty_block_and_the_trigger_flag_is_ignored() {
        let mut bench = Bench::ready(
            Adxl345Config::default(),
            vec![
                I2cStep::Reply(vec![0x00]),
                I2cStep::Reply(vec![0x00]),
                I2cStep::Reply(vec![0x00]),
                I2cStep::Reply(vec![0x81]), // FIFO_TRIG and one entry
                I2cStep::Reply(vec![0x00, 0x01, 0x00, 0x01, 0x00, 0x01]),
            ],
        );
        let block = bench.read_fifo().expect("an empty block");
        assert_eq!((block.sequence, block.samples.len(), block.lost), (0, 0, 0));
        let block = bench.read_fifo().expect("one entry");
        assert_eq!((block.sequence, block.samples.len()), (1, 1));
    }

    /// The answers to a drain of `entries` samples of +1 g on z, with the overrun flag as given.
    fn drain(overrun: bool, entries: u8) -> Vec<I2cStep> {
        let mut steps = vec![
            I2cStep::Reply(vec![if overrun { 0x83 } else { 0x82 }]),
            I2cStep::Reply(vec![entries]),
        ];
        steps.extend(
            std::iter::repeat_n(
                vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
                usize::from(entries),
            )
            .map(I2cStep::Reply),
        );
        steps
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (FIFO overflow)
    #[test]
    fn a_stream_overflow_sets_the_overrun_flag_and_counts_the_samples_lost() {
        // 100 Hz in stream mode, drained after 1 s: 100 samples were produced and the FIFO
        // kept the latest 32, so 68 were overwritten.
        let mut bench = Bench::ready(Adxl345Config::default(), drain(true, 32));
        bench.clock.advance(Duration::from_secs(1));
        let block = bench.read_fifo().expect("a block");
        assert!(block.overflowed);
        assert_eq!(block.samples.len(), 32);
        assert_eq!(block.lost, 68);
        assert_eq!(
            bench.transfers_since(INIT).len(),
            34,
            "level, flag and 32 entries"
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (FIFO overflow)
    #[test]
    fn a_fifo_mode_overflow_counts_the_samples_dropped_after_the_full_fifo() {
        // 800 Hz in FIFO mode, drained after 100 ms: 80 samples were produced and 33 kept
        // (32 in the FIFO and one at the output), so 47 were dropped.
        let mut config = config(AccelRange::G2, true, FifoMode::Fifo);
        config.rate = OutputRate::Hz800;
        let mut bench = Bench::ready(config, drain(true, 33));
        bench.clock.advance(Duration::from_millis(100));
        let block = bench.read_fifo().expect("a block");
        assert_eq!(block.interval, Duration::from_micros(1250));
        assert!(block.overflowed);
        assert_eq!((block.samples.len(), block.lost), (33, 47));
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (FIFO overflow)
    #[test]
    fn loss_is_counted_from_the_previous_drain_and_only_when_the_device_flags_an_overrun() {
        let mut script = drain(true, 32); // at 1 s: 100 produced, 68 lost
        script.extend(drain(true, 32)); // at 2 s: 100 more, 68 lost, not 168
        script.extend(drain(false, 5)); // at 3 s, no overrun: late, but nothing flagged lost
        script.extend(drain(true, 32)); // 10 ms later: full, but no loss measurable
        let mut bench = Bench::ready(Adxl345Config::default(), script);
        let mut seen = Vec::new();
        for advance in [1000, 1000, 1000, 10] {
            bench.clock.advance(Duration::from_millis(advance));
            let block = bench.read_fifo().expect("a block");
            seen.push((block.sequence, block.overflowed, block.lost));
        }
        assert_eq!(
            seen,
            vec![(0, true, 68), (1, true, 68), (2, false, 0), (3, true, 0)]
        );
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (FIFO overflow)
    #[test]
    fn a_full_fifo_is_an_overflow_even_when_the_flag_read_before_it_was_clear() {
        // The FIFO filled between the read of INT_SOURCE and the read of FIFO_STATUS: the flag
        // read was clear, but a level of 32 or 33 means the FIFO filled, which sets it.
        // (mode, entries, lost of the 100 samples produced in 1 s at 100 Hz).
        for (fifo, entries, lost) in [
            (FifoMode::Stream, 32, 68),
            (FifoMode::Stream, 33, 67),
            (FifoMode::Fifo, 32, 68),
            (FifoMode::Fifo, 33, 67),
        ] {
            let mut bench = Bench::ready(config(AccelRange::G2, true, fifo), drain(false, entries));
            bench.clock.advance(Duration::from_secs(1));
            let block = bench.read_fifo().expect("a block");
            assert!(block.overflowed, "{fifo:?} {entries}");
            assert_eq!(block.lost, lost, "{fifo:?} {entries}");
        }
        let mut bench = Bench::ready(Adxl345Config::default(), drain(false, 31));
        bench.clock.advance(Duration::from_secs(1));
        let block = bench.read_fifo().expect("a block");
        assert_eq!((block.overflowed, block.lost), (false, 0), "31 is not full");
    }

    /// The answers to a drain of `entries` samples whose bus stalls for `stall` after
    /// `before` entries were popped.
    fn stalled_drain(entries: u8, before: usize, stall: Duration) -> Vec<I2cStep> {
        let mut steps = drain(false, entries);
        steps.insert(before.saturating_add(2), I2cStep::Delay(stall));
        steps
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (FIFO overflow)
    #[test]
    fn a_fifo_that_fills_during_the_drain_is_an_overflow_counted_once() {
        // 100 Hz in stream mode, 20 entries at the level read. A 100 ms stall after 5 pops
        // leaves 15 entries and 10 new samples: the FIFO did not fill.
        let mut bench = Bench::ready(
            Adxl345Config::default(),
            stalled_drain(20, 5, Duration::from_millis(100)),
        );
        let block = bench.read_fifo().expect("a block");
        assert_eq!((block.overflowed, block.lost), (false, 0));

        // A 400 ms stall instead: 15 entries and 40 new samples are 22 beyond the 33 the device
        // keeps, so 22 were overwritten, and the pops after the stall cleared the flag.
        let mut script = stalled_drain(20, 5, Duration::from_millis(400));
        script.extend(drain(true, 32));
        let mut bench = Bench::ready(Adxl345Config::default(), script);
        let level_read = bench.clock.now();
        let block = bench.read_fifo().expect("a block");
        assert!(
            block.overflowed,
            "no flag, but the clock says the FIFO filled"
        );
        assert_eq!((block.samples.len(), block.lost), (20, 22));
        assert_eq!(block.sampled_at, level_read);
        // At 1 s, full and flagged: of the 100 samples produced since the level read, 7 were
        // among the 22 lost already and 15 were in the first block, so 46 are lost, not 68.
        bench.clock.advance(Duration::from_millis(600));
        let block = bench.read_fifo().expect("a block");
        assert_eq!(
            (block.sequence, block.overflowed, block.lost),
            (1, true, 46)
        );
    }

    #[test]
    fn every_output_rate_has_its_exact_interval_and_scales_the_loss() {
        // The intervals written out by hand: 1/3200 s is 312.5 us, and each lower rate doubles
        // it, down to 160 ms at 6.25 Hz.
        let rates = [
            (OutputRate::Hz3200, Duration::from_nanos(312_500)),
            (OutputRate::Hz1600, Duration::from_micros(625)),
            (OutputRate::Hz800, Duration::from_micros(1250)),
            (OutputRate::Hz400, Duration::from_micros(2500)),
            (OutputRate::Hz200, Duration::from_millis(5)),
            (OutputRate::Hz100, Duration::from_millis(10)),
            (OutputRate::Hz50, Duration::from_millis(20)),
            (OutputRate::Hz25, Duration::from_millis(40)),
            (OutputRate::Hz12_5, Duration::from_millis(80)),
            (OutputRate::Hz6_25, Duration::from_millis(160)),
        ];
        for (rate, interval) in rates {
            let mut config = Adxl345Config::default();
            config.rate = rate;
            let mut script = drain(false, 0);
            script.extend(drain(true, 32));
            let mut bench = Bench::ready(config, script);
            let block = bench.read_fifo().expect("an empty block");
            assert_eq!(block.interval, interval, "{rate:?}");
            // 100 intervals later, one nanosecond short: 99 samples produced, 32 kept.
            let late = interval
                .checked_mul(100)
                .and_then(|late| late.checked_sub(Duration::from_nanos(1)))
                .expect("well within a Duration");
            bench.clock.advance(late);
            let block = bench.read_fifo().expect("a full block");
            assert_eq!(block.interval, interval, "{rate:?}");
            assert_eq!(block.lost, 67, "{rate:?}");
        }
    }

    #[test]
    fn sequence_numbers_count_blocks_without_gaps_and_restart_at_init() {
        let mut script = Vec::new();
        for _ in 0..3 {
            script.extend(drain(false, 1));
        }
        script.extend(init_steps());
        script.extend(drain(false, 0));
        let mut bench = Bench::ready(Adxl345Config::default(), script);
        let sequences: Vec<_> = (0..3)
            .map(|_| bench.read_fifo().expect("a block").sequence)
            .collect();
        assert_eq!(sequences, vec![0, 1, 2]);
        bench.init().expect("a new stream");
        assert_eq!(bench.read_fifo().expect("a block").sequence, 0);
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (invalid FIFO level)
    #[test]
    fn a_fifo_level_the_device_cannot_report_is_malformed() {
        for status in [0x40, 0x22, 0x3F, 0xFF] {
            let mut bench = Bench::ready(
                Adxl345Config::default(),
                vec![I2cStep::Reply(vec![0x00]), I2cStep::Reply(vec![status])],
            );
            let error = bench.read_fifo().expect_err("not a FIFO level");
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{status:#04x}");
            assert_eq!(bench.transfers_since(INIT).len(), 2, "no entry was read");
            assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
        }
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C failure)
    #[test]
    fn a_bus_failure_in_the_middle_of_a_drain_ends_the_stream_until_init() {
        let mut script = vec![
            I2cStep::Reply(vec![0x00]),
            I2cStep::Reply(vec![0x03]),
            I2cStep::Reply(vec![0, 0, 0, 0, 0, 1]),
            I2cStep::Fail(ErrorKind::Disconnected),
        ];
        script.extend(init_steps());
        script.extend(drain(false, 1));
        let mut bench = Bench::ready(Adxl345Config::default(), script);
        let error = bench.read_fifo().expect_err("the second entry fails");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
        bench.init().expect("recovered");
        let block = bench.read_fifo().expect("a new stream");
        assert_eq!((block.sequence, block.samples.len()), (0, 1));
    }

    // covers: SPEC mandatory-failure-cases / F10 direct sensors (I2C timeout)
    #[test]
    fn a_drain_that_times_out_or_is_dropped_ends_the_stream() {
        let mut bench = Bench::ready(
            Adxl345Config::default(),
            vec![I2cStep::Reply(vec![0x00]), I2cStep::Silence],
        );
        let error = bench.read_fifo().expect_err("FIFO_STATUS never answers");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);

        // A bus that stalls past the deadline times out at the deadline.
        let mut bench = Bench::ready(
            Adxl345Config::default(),
            vec![
                I2cStep::Reply(vec![0x00]),
                I2cStep::Delay(Duration::from_secs(2)),
                I2cStep::Reply(vec![0x00]),
            ],
        );
        let deadline = bench.deadline();
        let error = bench.read_fifo().expect_err("FIFO_STATUS answers too late");
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert!(deadline.is_elapsed(bench.clock.now()), "at the deadline");
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);

        let mut bench = Bench::ready(Adxl345Config::default(), vec![I2cStep::Silence]);
        let deadline = bench.deadline();
        let mut drain = bench.sensor.read_fifo(deadline);
        assert!(matches!(poll_once(&mut drain), Poll::Pending), "waiting");
        drop(drain);
        assert_refused_without_io(&mut bench, ErrorKind::Disconnected);
    }

    #[test]
    fn the_adapter_debug_names_its_configuration_and_state() {
        let bench = Bench::new(Adxl345Config::default(), Vec::new());
        let text = format!("{:?}", bench.sensor);
        assert!(text.starts_with("Adxl345"), "{text}");
        assert!(text.contains("ready: false"), "{text}");
        assert!(text.contains("address: 83"), "{text}");
    }

    #[test]
    fn the_bus_trait_is_object_safe_and_send() {
        fn assert_send<T: Send + ?Sized>() {}
        assert_send::<dyn I2cBus>();
        assert_send::<Adxl345>();
    }
}
