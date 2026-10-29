//! The ADXL345 three-axis accelerometer on I2C (ADR-0029).

use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use bytes::Bytes;

use super::{I2cBus, failure, measurement};
use crate::sensors::{Measurement, Quality};
use crate::{Clock, Deadline, ErrorKind, FailureCategory, OpFuture, PlcValue, Result};

/// `DEVID` (0x00): the fixed device identifier.
const DEVID: u8 = 0x00;
/// The `DEVID` of every ADXL345 (octal 345).
const DEVICE_ID: u8 = 0xE5;
/// `OFSX` (0x1E), followed by `OFSY` and `OFSZ`: offsets the device adds to every sample.
const OFSX: u8 = 0x1E;
/// `BW_RATE` (0x2C): the output rate in bits 3 to 0, the low-power bit 4.
const BW_RATE: u8 = 0x2C;
/// `POWER_CTL` (0x2D): link, auto-sleep, measure (bit 3), sleep and wakeup.
const POWER_CTL: u8 = 0x2D;
/// `INT_SOURCE` (0x30): `DATA_READY` in bit 7, the overrun flag in bit 0.
const INT_SOURCE: u8 = 0x30;
/// `DATA_FORMAT` (0x31): self-test, SPI, interrupt polarity, full resolution (bit 3), justify
/// (bit 2) and range (bits 1 and 0).
const DATA_FORMAT: u8 = 0x31;
/// `DATAX0` (0x32): the first of the six data registers, x, y and z, each low byte first.
const DATAX0: u8 = 0x32;
/// `FIFO_CTL` (0x38): the mode in bits 7 and 6, trigger in bit 5, samples in bits 4 to 0.
const FIFO_CTL: u8 = 0x38;
/// `FIFO_STATUS` (0x39): the trigger flag in bit 7, bit 6 reads 0, the entries in bits 5 to 0.
const FIFO_STATUS: u8 = 0x39;

/// `POWER_CTL` with only the measure bit set: measuring, no sleep, no link.
const MEASURE: u8 = 0x08;
/// `DATA_FORMAT` bit 3, `FULL_RES`.
const FULL_RES: u8 = 0x08;
/// `INT_SOURCE` bit 7, `DATA_READY`: the data registers hold a sample not read yet. Reading
/// them clears it.
const DATA_READY: u8 = 0x80;
/// `INT_SOURCE` bit 0: new data replaced unread data, or the FIFO filled.
const OVERRUN: u8 = 0x01;
/// `FIFO_STATUS` bits 5 to 0: the entries the FIFO holds.
const ENTRIES: u8 = 0x3F;
/// The entries of a full FIFO; the device sets the overrun flag when the FIFO fills.
const FIFO_DEPTH: u8 = 32;
/// `FIFO_STATUS` bit 6, which the device always reads as 0.
const STATUS_ZERO: u8 = 0x40;
/// The most entries there can be: 32 in the FIFO and one at the output.
const MAX_ENTRIES: u8 = 33;
/// The bytes of one sample: x, y and z, each a little-endian pair.
const SAMPLE_BYTES: usize = 6;

/// Names of the three acceleration measurements of a single reading.
const AXES: [&str; 3] = ["acceleration_x", "acceleration_y", "acceleration_z"];
/// The UCUM unit of the measurements: the standard acceleration of free fall.
const STANDARD_GRAVITY: &str = "[g]";

/// The measurement range of an ADXL345.
///
/// # Examples
/// ```
/// use stonecrab::boards::{AccelRange, Adxl345Config};
///
/// let mut config = Adxl345Config::default();
/// config.range = AccelRange::G16;
/// assert_eq!(config.range, AccelRange::G16);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum AccelRange {
    /// Plus or minus 2 g (`DATA_FORMAT` range bits 00).
    G2,
    /// Plus or minus 4 g (range bits 01).
    G4,
    /// Plus or minus 8 g (range bits 10).
    G8,
    /// Plus or minus 16 g (range bits 11).
    G16,
}

impl AccelRange {
    /// The range bits of `DATA_FORMAT`, the counts per g in 10-bit mode (the datasheet's typical
    /// sensitivity), and the largest count at full resolution (10, 11, 12 or 13 bits).
    const fn layout(self) -> (u8, u16, i16) {
        match self {
            Self::G2 => (0b00, 256, 511),
            Self::G4 => (0b01, 128, 1023),
            Self::G8 => (0b10, 64, 2047),
            Self::G16 => (0b11, 32, 4095),
        }
    }
}

/// The output data rate of an ADXL345: the rates of the datasheet's table of rate codes, from
/// 3200 Hz (code 1111) halving down to 6.25 Hz (code 0110).
///
/// Each rate's interval is exact in nanoseconds: 1/3200 s is 312 500 ns, and every lower rate
/// doubles it.
///
/// # Examples
/// ```
/// use stonecrab::boards::{Adxl345Config, OutputRate};
///
/// let mut config = Adxl345Config::default();
/// config.rate = OutputRate::Hz400;
/// assert_eq!(config.rate, OutputRate::Hz400);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OutputRate {
    /// 3200 Hz (rate code 1111).
    Hz3200,
    /// 1600 Hz (1110).
    Hz1600,
    /// 800 Hz (1101).
    Hz800,
    /// 400 Hz (1100).
    Hz400,
    /// 200 Hz (1011).
    Hz200,
    /// 100 Hz (1010), the device's reset value.
    Hz100,
    /// 50 Hz (1001).
    Hz50,
    /// 25 Hz (1000).
    Hz25,
    /// 12.5 Hz (0111).
    Hz12_5,
    /// 6.25 Hz (0110).
    Hz6_25,
}

impl OutputRate {
    /// The rate code of `BW_RATE` and the time between two samples, in nanoseconds.
    const fn layout(self) -> (u8, u64) {
        match self {
            Self::Hz3200 => (0b1111, 312_500),
            Self::Hz1600 => (0b1110, 625_000),
            Self::Hz800 => (0b1101, 1_250_000),
            Self::Hz400 => (0b1100, 2_500_000),
            Self::Hz200 => (0b1011, 5_000_000),
            Self::Hz100 => (0b1010, 10_000_000),
            Self::Hz50 => (0b1001, 20_000_000),
            Self::Hz25 => (0b1000, 40_000_000),
            Self::Hz12_5 => (0b0111, 80_000_000),
            Self::Hz6_25 => (0b0110, 160_000_000),
        }
    }
}

/// How an ADXL345 keeps samples: no FIFO, or one of the two FIFO modes the adapter drains.
///
/// With [`Bypass`](Self::Bypass) a host reads the latest sample with
/// [`Adxl345::read_sample`]; with [`Fifo`](Self::Fifo) or [`Stream`](Self::Stream) it drains
/// the FIFO with [`Adxl345::read_fifo`]. The device's trigger mode needs an interrupt line and
/// is not offered.
///
/// # Examples
/// ```
/// use stonecrab::boards::{Adxl345Config, FifoMode};
///
/// assert_eq!(Adxl345Config::default().fifo, FifoMode::Stream);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FifoMode {
    /// The FIFO is bypassed (`FIFO_CTL` mode bits 00): the data registers hold the latest sample.
    Bypass,
    /// The FIFO collects up to 32 samples and then stops (01): when it is full, new samples
    /// are dropped, so the samples kept are the oldest.
    Fifo,
    /// The FIFO holds the latest 32 samples (10): when it is full, the oldest is overwritten,
    /// so the samples kept are the newest.
    Stream,
}

impl FifoMode {
    /// The mode bits of `FIFO_CTL`, in place (bits 7 and 6).
    const fn bits(self) -> u8 {
        match self {
            Self::Bypass => 0x00,
            Self::Fifo => 0x40,
            Self::Stream => 0x80,
        }
    }
}

/// The configuration an [`Adxl345`] writes at [`init`](Adxl345::init).
///
/// These are device configuration, not values the application controls: the adapter writes
/// them, in a fixed order, every time it initializes the device, and offers no other way to
/// write a register. The default is the device at address 0x53 (SDO/ALT ADDRESS grounded, as
/// on most breakout boards), plus or minus 2 g at full resolution, 100 Hz and the FIFO in
/// stream mode.
///
/// # Examples
/// ```
/// use stonecrab::boards::{AccelRange, Adxl345Config, FifoMode, OutputRate};
///
/// let mut config = Adxl345Config::default();
/// config.address = 0x1D; // SDO/ALT ADDRESS high
/// config.range = AccelRange::G8;
/// config.rate = OutputRate::Hz800;
/// config.fifo = FifoMode::Fifo;
/// config.full_resolution = false; // 10 bits at every range
/// assert_eq!(config.address, 0x1D);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Adxl345Config {
    /// The 7-bit I2C address: 0x53 with SDO/ALT ADDRESS low, 0x1D with it high.
    pub address: u8,
    /// The measurement range.
    pub range: AccelRange,
    /// The output data rate.
    pub rate: OutputRate,
    /// How the device keeps samples.
    pub fifo: FifoMode,
    /// Full resolution (10 to 13 bits, 256 counts per g at every range) when `true`; 10 bits
    /// at every range (256, 128, 64 or 32 counts per g) when `false`.
    pub full_resolution: bool,
}

impl Default for Adxl345Config {
    fn default() -> Self {
        Self {
            address: 0x53,
            range: AccelRange::G2,
            rate: OutputRate::Hz100,
            fifo: FifoMode::Stream,
            full_resolution: true,
        }
    }
}

impl Adxl345Config {
    /// Counts per g of the samples (the datasheet's typical sensitivity).
    const fn counts_per_g(self) -> u16 {
        let (_, counts_per_g, _) = self.range.layout();
        if self.full_resolution {
            256
        } else {
            counts_per_g
        }
    }

    /// The largest count a sample can hold: 511 in 10-bit mode, up to 4095 at full resolution.
    const fn largest_count(self) -> i16 {
        let (_, _, largest) = self.range.layout();
        if self.full_resolution { largest } else { 511 }
    }

    /// `DATA_FORMAT`: full resolution and range; no self-test, right-justified, interrupts
    /// active high, 4-wire SPI (meaningless on I2C).
    const fn data_format(self) -> u8 {
        let (range, _, _) = self.range.layout();
        if self.full_resolution {
            FULL_RES | range
        } else {
            range
        }
    }

    /// The interval between two samples, exact.
    const fn interval(self) -> Duration {
        let (_, nanos) = self.rate.layout();
        Duration::from_nanos(nanos)
    }
}

/// One three-axis sample of an ADXL345, as the counts the device sent.
///
/// The counts are right-justified and sign-extended, so a negative acceleration is a negative
/// count. Divide by the counts per g of the configuration (carried by [`AccelReading`] and
/// [`SampleBlock`]) for g; nothing is rounded here.
///
/// # Examples
/// ```
/// use stonecrab::boards::{AccelSample, Adxl345Config};
///
/// // x = +1 g, y = -1 g, z = 0 at 256 counts per g.
/// let sample = AccelSample::from_registers([0x00, 0x01, 0x00, 0xFF, 0x00, 0x00], Adxl345Config::default())?;
/// assert_eq!((sample.x, sample.y, sample.z), (256, -256, 0));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct AccelSample {
    /// The x-axis count (`DATAX1` and `DATAX0`).
    pub x: i16,
    /// The y-axis count (`DATAY1` and `DATAY0`).
    pub y: i16,
    /// The z-axis count (`DATAZ1` and `DATAZ0`).
    pub z: i16,
}

impl AccelSample {
    /// Decode the six data registers (`DATAX0` to `DATAZ1`) as `config` lays them out.
    ///
    /// Each axis is a little-endian pair, low byte first, right-justified with sign extension
    /// (the only justification the adapter configures). The count must fit the resolution of
    /// `config`: 10 bits (-512 to 511) in 10-bit mode, and 10, 11, 12 or 13 bits at full
    /// resolution for plus or minus 2, 4, 8 or 16 g. At plus or minus 2, 4 and 8 g the device
    /// clips to its range, so a count outside it is not one the device sent. At plus or minus
    /// 16 g the datasheet does not promise clipping: an acceleration beyond the range can roll
    /// over inside the field to a count that fits, which no check here can tell from a true
    /// one.
    ///
    /// # Errors
    /// [`ErrorKind::MalformedData`] if a count does not fit the resolution; the message names
    /// the axis and the count.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::boards::{AccelRange, AccelSample, Adxl345Config};
    ///
    /// let mut config = Adxl345Config::default();
    /// config.range = AccelRange::G16; // 13 bits at full resolution
    /// let sample = AccelSample::from_registers([0xFF, 0x0F, 0x00, 0xF0, 0x01, 0x00], config)?;
    /// assert_eq!((sample.x, sample.y, sample.z), (4095, -4096, 1));
    ///
    /// config.full_resolution = false; // 10 bits: 4095 cannot be a count
    /// let error = AccelSample::from_registers([0xFF, 0x0F, 0, 0, 0, 0], config).expect_err("too wide");
    /// assert_eq!(error.kind(), ErrorKind::MalformedData);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn from_registers(data: [u8; SAMPLE_BYTES], config: Adxl345Config) -> Result<Self> {
        let [x0, x1, y0, y1, z0, z1] = data;
        let sample = Self {
            x: i16::from_le_bytes([x0, x1]),
            y: i16::from_le_bytes([y0, y1]),
            z: i16::from_le_bytes([z0, z1]),
        };
        let largest = config.largest_count();
        // `!largest` is the most negative count of the same width (-512 for 511).
        for (axis, count) in [("x", sample.x), ("y", sample.y), ("z", sample.z)] {
            if !(!largest..=largest).contains(&count) {
                return Err(failure(
                    ErrorKind::MalformedData,
                    FailureCategory::Protocol,
                    format!(
                        "adxl345: {axis} count {count} is outside {}..={largest}, the resolution configured",
                        !largest
                    ),
                ));
            }
        }
        Ok(sample)
    }
}

/// One sample read from the data registers, with the time it was read.
///
/// [`Adxl345::read_sample`] returns one only for a sample the device flagged as new
/// (`DATA_READY`), so a reading is never the reset zeros of a device that has not produced a
/// sample yet, nor a sample returned before.
///
/// # Examples
/// ```
/// use std::time::{Instant, SystemTime};
/// use stonecrab::boards::{AccelReading, AccelSample, Adxl345Config};
///
/// let sample = AccelSample::from_registers([0, 0, 0, 0, 0x00, 0x01], Adxl345Config::default())?;
/// let reading = AccelReading::new(sample, 256, Instant::now(), SystemTime::UNIX_EPOCH);
/// assert_eq!(reading.sample.z, 256);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct AccelReading {
    /// The counts of the three axes.
    pub sample: AccelSample,
    /// Counts per g for the configuration the sample was read with: 256 at full resolution,
    /// 256, 128, 64 or 32 in 10-bit mode for plus or minus 2, 4, 8 or 16 g.
    pub counts_per_g: u16,
    /// The monotonic time, from the adapter's clock, at which the read completed.
    pub sampled_at: Instant,
    /// The wall-clock time, from the adapter's clock, at which the read completed.
    pub received_at: SystemTime,
}

impl AccelReading {
    /// A reading of `sample` at `counts_per_g`, stamped with the time it was read.
    ///
    /// [`Adxl345::read_sample`] builds one; this constructor is for code that decodes data
    /// registers it read some other way.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::boards::{AccelReading, AccelSample, Adxl345Config};
    ///
    /// let sample = AccelSample::from_registers([0; 6], Adxl345Config::default())?;
    /// let reading = AccelReading::new(sample, 256, Instant::now(), SystemTime::UNIX_EPOCH);
    /// assert_eq!(reading.counts_per_g, 256);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub const fn new(
        sample: AccelSample,
        counts_per_g: u16,
        sampled_at: Instant,
        received_at: SystemTime,
    ) -> Self {
        Self {
            sample,
            counts_per_g,
            sampled_at,
            received_at,
        }
    }

    /// The reading as three measurements of the sensors envelope, in this order:
    /// `acceleration_x`, `acceleration_y` and `acceleration_z`.
    ///
    /// Each is in `[g]` (the UCUM unit of the standard acceleration of free fall) as `F64`,
    /// the count divided by [`counts_per_g`](Self::counts_per_g), which is a power of two, so
    /// the division is exact. Each keeps its count in `raw` (`I16`), has quality `Good` (the
    /// sample was read and fits the configured resolution; that is not a claim of
    /// calibration, and at plus or minus 16 g it does not rule out a rollover, see
    /// [`AccelSample::from_registers`]), `origin`
    /// [`ValidationStatus::Documented`](crate::sensors::ValidationStatus::Documented), the
    /// reading's stamps, no source time and no protocol status.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if a measurement name were not a valid tag identifier;
    /// the names are constants, so this does not happen.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::PlcValue;
    /// use stonecrab::boards::{AccelReading, AccelSample, Adxl345Config};
    ///
    /// // z = +1 g, x = -0.5 g at 256 counts per g.
    /// let sample = AccelSample::from_registers([0x80, 0xFF, 0, 0, 0x00, 0x01], Adxl345Config::default())?;
    /// let measurements = AccelReading::new(sample, 256, Instant::now(), SystemTime::UNIX_EPOCH)
    ///     .measurements()?;
    /// assert_eq!(measurements[0].name, "acceleration_x");
    /// assert_eq!(measurements[0].value, Some(PlcValue::F64(-0.5)));
    /// assert_eq!(measurements[2].value, Some(PlcValue::F64(1.0)));
    /// assert_eq!(measurements[2].unit.as_deref(), Some("[g]"));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn measurements(&self) -> Result<Vec<Measurement>> {
        let counts_per_g = f64::from(self.counts_per_g);
        AXES.into_iter()
            .zip([self.sample.x, self.sample.y, self.sample.z])
            .map(|(name, count)| {
                measurement(
                    name,
                    STANDARD_GRAVITY,
                    Some(PlcValue::F64(f64::from(count) / counts_per_g)),
                    PlcValue::I16(count),
                    Quality::Good,
                    self.sampled_at,
                    self.received_at,
                )
            })
            .collect()
    }
}

/// The samples one drain of the ADXL345's FIFO delivered, with their spacing, their place in
/// the stream and what was lost before them.
///
/// A block is raw waveform data, not a set of measurements: its samples are equally spaced
/// by [`interval`](Self::interval), oldest first, each with the three axes in the fixed
/// layout of [`AccelSample`]. It is kept as one value, so a consumer never mistakes a
/// waveform for unrelated scalars, and it says what it does not hold:
///
/// - [`sequence`](Self::sequence) counts the blocks since [`Adxl345::init`]: 0, 1, 2 and so
///   on, with no gap, so a consumer that drops or misses a block can tell. Another `init`
///   starts again at 0, and samples between the two streams belong to neither.
/// - [`overflowed`](Self::overflowed) is the device's overrun flag, read before the drain, or
///   a level of 32 entries or more (a full FIFO, which sets the flag), or a FIFO that the
///   adapter's clock says filled during the drain: samples may have been lost. In stream mode
///   the oldest were overwritten, so the gap is before this block's first sample or, when the
///   FIFO filled during the drain, between two of its samples; in FIFO mode new samples were
///   dropped, after this block's last one.
/// - [`lost`](Self::lost) is the host's count of the samples lost, when `overflowed` is set;
///   it is 0 otherwise. The device counts nothing it drops, and a pop clears its flag, so this
///   is an estimate from the adapter's clock: with the flag or a full level, the samples
///   produced since the previous drain less the entries delivered; during the drain, after
///   each pop, what the entries not popped before it and the samples produced since the level
///   was read add up to beyond the 33 the device keeps. It can be off by a sample or two of
///   bus latency.
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline};
/// use stonecrab::boards::{Adxl345, Adxl345Config};
/// use stonecrab::testkit::{FakeClock, FakeI2cBus, I2cStep, block_on_with};
///
/// let clock = FakeClock::default();
/// let mut script = vec![I2cStep::Reply(vec![0xE5]), I2cStep::Ack, I2cStep::Ack]; // `DEVID`, standby, bypass
/// script.push(I2cStep::Reply(vec![0; 6])); // the data registers, read once to clear the flags
/// script.extend(std::iter::repeat_n(I2cStep::Ack, 5)); // the rest of the configuration
/// script.push(I2cStep::Reply(vec![0x00])); // `INT_SOURCE`: no overrun
/// script.push(I2cStep::Reply(vec![0x01])); // `FIFO_STATUS`: one entry
/// script.push(I2cStep::Reply(vec![0, 0, 0, 0, 0x00, 0x01])); // x = 0, y = 0, z = +1 g
/// let bus = FakeI2cBus::new(script, clock.clone());
/// let mut sensor = Adxl345::new(Box::new(bus), Arc::new(clock.clone()), Adxl345Config::default());
///
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
/// block_on_with(&clock, sensor.init(deadline))?;
/// let block = block_on_with(&clock, sensor.read_fifo(deadline))?;
/// assert_eq!((block.sequence, block.samples.len(), block.lost), (0, 1, 0));
/// assert_eq!(block.interval, Duration::from_millis(10));
/// assert_eq!(block.samples[0].z, 256);
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SampleBlock {
    /// The place of this block in the stream since `init`: 0 for the first, then one more for
    /// each block.
    pub sequence: u64,
    /// The time between two samples at the configured output rate, exact.
    pub interval: Duration,
    /// Counts per g of every sample in the block (see [`AccelReading::counts_per_g`]).
    pub counts_per_g: u16,
    /// The samples, oldest first. Empty when the FIFO was empty. Equally spaced unless
    /// `overflowed` is set.
    pub samples: Vec<AccelSample>,
    /// The device's overrun flag at the start of the drain, a full FIFO (32 entries or more)
    /// at the level read, or a FIFO that filled during the drain by the host's clock.
    pub overflowed: bool,
    /// Samples lost before or within this block, estimated from the host's clock; 0 unless
    /// `overflowed` is set.
    pub lost: u64,
    /// The monotonic time, from the adapter's clock, at which the FIFO's level was read. The
    /// samples were taken before it.
    pub sampled_at: Instant,
    /// The wall-clock time, from the adapter's clock, at which the FIFO's level was read.
    pub received_at: SystemTime,
}

/// An ADXL345 accelerometer on an I2C bus.
///
/// The adapter owns the bus, the clock it stamps samples with and the configuration it writes.
/// [`init`](Self::init) checks the device's identity and writes the configuration; then
/// [`read_sample`](Self::read_sample) (FIFO bypassed) reads a sample not read before, or
/// [`read_fifo`](Self::read_fifo) (FIFO or stream mode) drains the FIFO into a
/// [`SampleBlock`]. The configuration writes are the only writes the adapter makes, and there
/// is no public way to write a register.
///
/// A read that fails or is dropped leaves the stream in doubt (a data read pops the FIFO, a
/// device that lost power is back at its defaults), so the adapter refuses further reads until
/// `init` runs again and starts a new stream. Reading faster than the bus and the host allow is
/// not promised: what was lost is reported, not prevented.
///
/// The tests behind this type decode the datasheet's formats and drive the bus through a
/// scripted double; none establishes calibration accuracy, and no hardware record exists yet
/// (`docs/hardware/adxl345-i2c.md` is the runbook for one).
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline};
/// use stonecrab::boards::{Adxl345, Adxl345Config, FifoMode};
/// use stonecrab::testkit::{FakeClock, FakeI2cBus, I2cStep, block_on_with};
///
/// let clock = FakeClock::default();
/// let mut config = Adxl345Config::default();
/// config.fifo = FifoMode::Bypass;
/// let mut script = vec![I2cStep::Reply(vec![0xE5]), I2cStep::Ack, I2cStep::Ack]; // `DEVID`, standby, bypass
/// script.push(I2cStep::Reply(vec![0; 6])); // the data registers, read once to clear the flags
/// script.extend(std::iter::repeat_n(I2cStep::Ack, 5)); // the rest of the configuration
/// script.push(I2cStep::Reply(vec![0x00])); // `INT_SOURCE`: no sample yet
/// script.push(I2cStep::Reply(vec![0x80])); // `INT_SOURCE`: `DATA_READY`
/// script.push(I2cStep::Reply(vec![0, 0, 0, 0, 0x00, 0x01])); // z = +1 g
/// let bus = FakeI2cBus::new(script, clock.clone());
/// let mut sensor = Adxl345::new(Box::new(bus), Arc::new(clock.clone()), config);
///
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
/// block_on_with(&clock, sensor.init(deadline))?;
/// assert!(block_on_with(&clock, sensor.read_sample(deadline))?.is_none()); // turning on
/// let reading = block_on_with(&clock, sensor.read_sample(deadline))?.expect("a new sample");
/// assert_eq!((reading.sample.z, reading.counts_per_g), (256, 256));
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct Adxl345 {
    bus: Box<dyn I2cBus>,
    clock: Arc<dyn Clock>,
    config: Adxl345Config,
    /// `init` succeeded and no read failed or was dropped since.
    ready: bool,
    /// The sequence number of the next block.
    sequence: u64,
    /// When the FIFO's level was last read (or `init` finished), moved on past the samples
    /// that drain already counted lost.
    drained_at: Instant,
}

impl fmt::Debug for Adxl345 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Adxl345")
            .field("config", &self.config)
            .field("ready", &self.ready)
            .field("sequence", &self.sequence)
            .finish_non_exhaustive()
    }
}

impl Adxl345 {
    /// An accelerometer on `bus`, stamping its samples with `clock`, to be configured as
    /// `config`. Nothing is sent until [`init`](Self::init).
    ///
    /// Derive the deadlines passed to the reads from the same clock.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::Clock;
    /// use stonecrab::boards::{Adxl345, Adxl345Config, I2cBus};
    ///
    /// fn adapter(bus: Box<dyn I2cBus>, clock: Arc<dyn Clock>) -> Adxl345 {
    ///     Adxl345::new(bus, clock, Adxl345Config::default())
    /// }
    /// # let _ = adapter;
    /// ```
    #[must_use]
    pub fn new(bus: Box<dyn I2cBus>, clock: Arc<dyn Clock>, config: Adxl345Config) -> Self {
        let drained_at = clock.now();
        Self {
            bus,
            clock,
            config,
            ready: false,
            sequence: 0,
            drained_at,
        }
    }

    /// Read `DEVID` and check that the device is an ADXL345 (0xE5).
    ///
    /// The deadline is the `deadline` argument. This changes nothing on the device and does
    /// not start or end a stream.
    ///
    /// # Errors
    /// - The bus's own errors, unchanged: [`ErrorKind::Timeout`] when the deadline passes and
    ///   [`ErrorKind::Disconnected`] when the device does not answer or the bus fails.
    /// - [`ErrorKind::MalformedData`] if the bus returned other than one byte.
    /// - [`ErrorKind::Trust`] if `DEVID` is not 0xE5: the device at the address is not an
    ///   ADXL345. The message names the byte read.
    ///
    /// # Cancel safety
    /// Cancel-safe: reading `DEVID` has no effect on the device.
    ///
    /// # Wire effects
    /// One register read: the register address 0x00, then one byte.
    pub fn verify_identity(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            let [id] = self.registers::<1>(DEVID, deadline).await?;
            if id == DEVICE_ID {
                Ok(())
            } else {
                Err(failure(
                    ErrorKind::Trust,
                    FailureCategory::Protocol,
                    format!(
                        "adxl345: DEVID of the device at {:#04x} is {id:#04x}, not {DEVICE_ID:#04x}",
                        self.config.address
                    ),
                ))
            }
        })
    }

    /// Check the identity, write the configuration and start a new stream.
    ///
    /// After [`verify_identity`](Self::verify_identity), seven writes and one read, in this
    /// order:
    /// 1. `POWER_CTL` 0x00: standby, as the datasheet asks while configuring.
    /// 2. `FIFO_CTL` 0x00: bypass, which empties the FIFO of an earlier stream.
    /// 3. A six-byte read of `DATAX0` to `DATAZ1`, discarded: it clears `DATA_READY` and the
    ///    overrun flag an earlier stream left, so neither is reported for the new one.
    /// 4. `OFSX`, `OFSY` and `OFSZ` 0, in one write: no offset left by another program shifts the
    ///    samples.
    /// 5. `DATA_FORMAT`: the range and full resolution of the configuration, right-justified, no
    ///    self-test.
    /// 6. `BW_RATE`: the output rate, normal power.
    /// 7. `FIFO_CTL`: the FIFO mode, trigger and samples bits 0. The adapter polls and uses no
    ///    interrupt; `INT_ENABLE` and `INT_MAP` are left as found, and a samples value of 0
    ///    sets the watermark flag at once, so a watermark interrupt another program enabled
    ///    keeps its INT pin asserted.
    /// 8. `POWER_CTL` 0x08: measuring.
    ///
    /// The next block has sequence 0, and loss is counted from the end of `init`. The first
    /// sample follows after the datasheet's turn-on time (about one output period plus
    /// 1.1 ms); until then [`read_sample`](Self::read_sample) returns `None`. Call `init`
    /// again to recover after a failed read or a device that lost power. The deadline is the
    /// `deadline` argument and bounds all nine transactions.
    ///
    /// # Errors
    /// The errors of [`verify_identity`](Self::verify_identity) (nothing is written to a
    /// device that is not an ADXL345), the bus's own errors, and [`ErrorKind::MalformedData`]
    /// if the read of the data registers returned other than six bytes. After an error the
    /// device may be partly configured, and reads are refused until an `init` succeeds.
    ///
    /// # Cancel safety
    /// Not cancel-safe: a dropped call may leave the device partly configured; reads are then
    /// refused until an `init` succeeds.
    ///
    /// # Wire effects
    /// Two register reads and seven register writes, as listed above. They configure the
    /// device; they change no measured quantity.
    pub fn init(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            self.ready = false;
            self.verify_identity(deadline).await?;
            let config = self.config;
            for bytes in [vec![POWER_CTL, 0x00], vec![FIFO_CTL, 0x00]] {
                self.bus
                    .write(config.address, Bytes::from(bytes), deadline)
                    .await?;
            }
            self.registers::<SAMPLE_BYTES>(DATAX0, deadline).await?;
            let writes = [
                vec![OFSX, 0x00, 0x00, 0x00],
                vec![DATA_FORMAT, config.data_format()],
                vec![BW_RATE, config.rate.layout().0],
                vec![FIFO_CTL, config.fifo.bits()],
                vec![POWER_CTL, MEASURE],
            ];
            for bytes in writes {
                self.bus
                    .write(config.address, Bytes::from(bytes), deadline)
                    .await?;
            }
            self.sequence = 0;
            self.drained_at = self.clock.now();
            self.ready = true;
            Ok(())
        })
    }

    /// Read the sample the data registers hold, with the FIFO bypassed, if it was not read
    /// before.
    ///
    /// Reads `INT_SOURCE` first. With `DATA_READY` clear there is no sample this adapter has
    /// not returned already: straight after [`init`](Self::init), during the datasheet's
    /// turn-on time (about one output period plus 1.1 ms), the registers hold reset zeros,
    /// and a read faster than the output rate would repeat the previous sample. Then the data
    /// registers are not read, the result is `Ok(None)` and the stream goes on. Otherwise the
    /// six data registers are read, which clears `DATA_READY`, and the reading is stamped
    /// after that read completes. Samples the host did not read before the next one replaced
    /// them are not counted; [`read_fifo`](Self::read_fifo) is the way to keep every sample.
    /// The deadline is the `deadline` argument.
    ///
    /// # Errors
    /// - [`ErrorKind::Disconnected`] before any I/O if [`init`](Self::init) has not succeeded
    ///   or a read failed or was dropped since.
    /// - [`ErrorKind::InvalidConfig`] before any I/O if the configuration uses the FIFO (use
    ///   [`read_fifo`](Self::read_fifo)).
    /// - The bus's own errors, unchanged ([`ErrorKind::Timeout`], [`ErrorKind::Disconnected`]).
    /// - [`ErrorKind::MalformedData`] if the bus returned a wrong number of bytes or a count
    ///   does not fit the resolution ([`AccelSample::from_registers`]).
    ///
    /// Every error but the first two ends the stream: reads are refused until `init` runs.
    ///
    /// # Cancel safety
    /// A dropped call ends the stream as an error does.
    ///
    /// # Wire effects
    /// One one-byte register read of 0x30 (`INT_SOURCE`), then, with `DATA_READY` set, one
    /// six-byte read of 0x32 (`DATAX0` to `DATAZ1`).
    pub fn read_sample(&mut self, deadline: Deadline) -> OpFuture<'_, Option<AccelReading>> {
        Box::pin(async move {
            self.start(false)?;
            let [source] = self.registers::<1>(INT_SOURCE, deadline).await?;
            if source & DATA_READY == 0 {
                self.ready = true;
                return Ok(None);
            }
            let data = self.registers::<SAMPLE_BYTES>(DATAX0, deadline).await?;
            let sampled_at = self.clock.now();
            let received_at = self.clock.wall_now();
            let sample = AccelSample::from_registers(data, self.config)?;
            self.ready = true;
            Ok(Some(AccelReading::new(
                sample,
                self.config.counts_per_g(),
                sampled_at,
                received_at,
            )))
        })
    }

    /// Drain the FIFO into the next [`SampleBlock`], in FIFO or stream mode.
    ///
    /// Reads the overrun flag (`INT_SOURCE`), then the FIFO's level (`FIFO_STATUS`), then each
    /// entry the level reported, oldest first, each in one six-byte read as the datasheet
    /// requires. Samples that arrive during the drain stay for the next block. The block is
    /// stamped when the level is read; its loss accounting is described on [`SampleBlock`].
    /// A level of 32 or more counts as an overflow even with the flag clear, because the FIFO
    /// may have filled between the two reads. Every pop clears the flag, so a FIFO that fills
    /// during the drain (a host stall between two pops) leaves no trace on the device: after
    /// each pop the adapter checks on its clock whether the entries not popped before it and
    /// the samples produced since the level was read exceed the 33 the device keeps, and
    /// counts the excess as lost. The deadline is the `deadline` argument and bounds the whole
    /// drain.
    ///
    /// # Errors
    /// - [`ErrorKind::Disconnected`] before any I/O if [`init`](Self::init) has not succeeded
    ///   or a read failed or was dropped since.
    /// - [`ErrorKind::InvalidConfig`] before any I/O if the FIFO is bypassed (use
    ///   [`read_sample`](Self::read_sample)).
    /// - The bus's own errors, unchanged ([`ErrorKind::Timeout`], [`ErrorKind::Disconnected`]).
    /// - [`ErrorKind::MalformedData`] if the bus returned a wrong number of bytes, `FIFO_STATUS`
    ///   has bit 6 set or more than 33 entries, or a count does not fit the resolution.
    ///
    /// Every error but the first two ends the stream: the entries already read are gone from
    /// the device, so reads are refused until `init` starts a new stream.
    ///
    /// # Cancel safety
    /// A dropped call ends the stream as an error does.
    ///
    /// # Wire effects
    /// Two one-byte register reads (0x30, 0x39), then one six-byte read of 0x32 per entry,
    /// each of which removes that entry from the FIFO.
    pub fn read_fifo(&mut self, deadline: Deadline) -> OpFuture<'_, SampleBlock> {
        Box::pin(async move {
            self.start(true)?;
            let [source] = self.registers::<1>(INT_SOURCE, deadline).await?;
            let [status] = self.registers::<1>(FIFO_STATUS, deadline).await?;
            let sampled_at = self.clock.now();
            let received_at = self.clock.wall_now();
            let entries = status & ENTRIES;
            if status & STATUS_ZERO != 0 || entries > MAX_ENTRIES {
                return Err(failure(
                    ErrorKind::MalformedData,
                    FailureCategory::Protocol,
                    format!(
                        "adxl345: FIFO_STATUS {status:#04x} is not a FIFO level (bit 6 set or more than {MAX_ENTRIES} entries)"
                    ),
                ));
            }
            let interval = self.config.interval();
            let mut samples = Vec::with_capacity(usize::from(entries));
            // Each pop clears the overrun flag, so the samples lost during the drain are counted
            // on the clock: just before the pop that ended now, the device held the entries not
            // popped before it and the samples produced since the level was read, and what was
            // beyond the MAX_ENTRIES it keeps was lost.
            let mut lost_draining = 0;
            for popped in 0..entries {
                let data = self.registers::<SAMPLE_BYTES>(DATAX0, deadline).await?;
                samples.push(AccelSample::from_registers(data, self.config)?);
                let held = produced(sampled_at, self.clock.now(), interval)
                    .saturating_add(u64::from(entries.saturating_sub(popped)));
                lost_draining = lost_draining.max(held.saturating_sub(u64::from(MAX_ENTRIES)));
            }
            let flagged = source & OVERRUN != 0 || entries >= FIFO_DEPTH;
            let lost_before = if flagged {
                produced(self.drained_at, sampled_at, interval).saturating_sub(u64::from(entries))
            } else {
                0
            };
            let overflowed = flagged || lost_draining > 0;
            let lost = lost_before.saturating_add(lost_draining);
            let block = SampleBlock {
                sequence: self.sequence,
                interval,
                counts_per_g: self.config.counts_per_g(),
                samples,
                overflowed,
                lost,
                sampled_at,
                received_at,
            };
            self.sequence = self.sequence.wrapping_add(1);
            // As many samples produced after the level read as were lost during the drain are
            // accounted for already (popped into this block in stream mode, dropped in FIFO
            // mode): the next block's estimate starts after them.
            self.drained_at = u32::try_from(lost_draining)
                .ok()
                .and_then(|count| interval.checked_mul(count))
                .and_then(|counted| sampled_at.checked_add(counted))
                .unwrap_or(sampled_at);
            self.ready = true;
            Ok(block)
        })
    }

    /// Admits a read: refused before any I/O if the stream has not started or the FIFO mode
    /// does not suit it; otherwise the stream is in doubt until the read succeeds.
    fn start(&mut self, drains_fifo: bool) -> Result<()> {
        if !self.ready {
            return Err(failure(
                ErrorKind::Disconnected,
                FailureCategory::Validation,
                "adxl345: no stream; init has not succeeded, or a read failed or was dropped since",
            ));
        }
        if drains_fifo == (self.config.fifo == FifoMode::Bypass) {
            let message = if drains_fifo {
                "adxl345: read_fifo needs FIFO or stream mode; the FIFO is bypassed, use read_sample"
            } else {
                "adxl345: read_sample needs the FIFO bypassed; use read_fifo"
            };
            return Err(failure(
                ErrorKind::InvalidConfig,
                FailureCategory::Validation,
                message,
            ));
        }
        self.ready = false;
        Ok(())
    }

    /// Reads `N` registers from `first` in one transaction.
    async fn registers<const N: usize>(
        &mut self,
        first: u8,
        deadline: Deadline,
    ) -> Result<[u8; N]> {
        let request = Bytes::copy_from_slice(&[first]);
        let bytes = self
            .bus
            .write_read(self.config.address, request, N, deadline)
            .await?;
        <[u8; N]>::try_from(bytes.as_ref()).map_err(|_wrong_length| {
            failure(
                ErrorKind::MalformedData,
                FailureCategory::Protocol,
                format!(
                    "adxl345: the bus returned {} bytes from register {first:#04x}, not {N}",
                    bytes.len()
                ),
            )
        })
    }
}

/// The samples a device at `interval` produced from `since` to `until`, by the adapter's clock,
/// rounded down.
fn produced(since: Instant, until: Instant, interval: Duration) -> u64 {
    let produced = until
        .saturating_duration_since(since)
        .as_nanos()
        .checked_div(interval.as_nanos())
        .unwrap_or_default();
    u64::try_from(produced).unwrap_or(u64::MAX)
}
