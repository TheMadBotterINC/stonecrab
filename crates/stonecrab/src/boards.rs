#![doc = include_str!("../docs/components/boards.md")]

use std::fmt;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use bytes::Bytes;

use crate::sensors::{Measurement, Quality, ValidationStatus};
use crate::{
    Clock, Deadline, Diagnostics, ErrorKind, FailureCategory, OpFuture, Operation, PlcError,
    PlcValue, Result, TagId,
};

/// Bytes in one MAX31855 frame: the 32 bits the converter shifts out, most significant first.
const FRAME_BYTES: usize = 4;

/// Bit 17 of the frame: reserved, always read as 0 by the converter.
const RESERVED_HIGH: u32 = 1 << 17;
/// Bit 16 of the frame: set when any of the three fault flags is set.
const FAULT: u32 = 1 << 16;
/// Bit 3 of the frame: reserved, always read as 0 by the converter.
const RESERVED_LOW: u32 = 1 << 3;
/// Bit 2: the thermocouple is shorted to Vcc.
const SHORT_TO_VCC: u32 = 1 << 2;
/// Bit 1: the thermocouple is shorted to ground.
const SHORT_TO_GROUND: u32 = 1 << 1;
/// Bit 0: the thermocouple is open.
const OPEN: u32 = 1;

/// Name of the thermocouple temperature measurement.
const THERMOCOUPLE: &str = "thermocouple_temperature";
/// Name of the internal (cold-junction) temperature measurement.
const COLD_JUNCTION: &str = "cold_junction_temperature";
/// Name of the fault measurement.
const FAULT_CODE: &str = "thermocouple_fault";

/// A failure on a board adapter: a read of the sensor, with the category of what went wrong.
fn failure(kind: ErrorKind, category: FailureCategory, message: impl Into<String>) -> PlcError {
    let diagnostics = Diagnostics {
        operation: Some(Operation::Read),
        category: Some(category),
        ..Diagnostics::default()
    };
    PlcError::new(kind, message).with_diagnostics(diagnostics)
}

/// A bus that clocks bytes out of one SPI device, within a deadline.
///
/// The trait is the whole contract between a board adapter and the hardware under it:
/// [`Max31855`] asks for a frame, a backend ([`LinuxSpiDev`] on a Linux host, the testkit's
/// `FakeSpiBus` in tests) delivers the bytes or a typed error. A backend moves bytes and
/// nothing else: it does not interpret, retry or reconnect. One chip select is asserted for
/// the whole transfer, so a device that frames its answer by chip select (as the MAX31855
/// does) gets one frame per call.
///
/// The trait is object safe and `Send`: a `Box<dyn SpiBus>` can move between threads, and
/// one bus has exactly one owner (`&mut self`), so transfers on it are serialized by
/// construction. Only a receive exists, because the only adapter shipped so far, the
/// MAX31855, sends nothing; a method that also transmits arrives, with a default that
/// refuses, together with the first device that needs it.
///
/// # Examples
/// A function over any bus; [`Max31855`] shows one running.
/// ```
/// use bytes::Bytes;
/// use stonecrab::{Deadline, Result};
/// use stonecrab::boards::SpiBus;
///
/// /// Ask a device for its four-byte answer.
/// async fn four_bytes(bus: &mut dyn SpiBus, deadline: Deadline) -> Result<Bytes> {
///     bus.read(4, deadline).await
/// }
/// # let _ = four_bytes;
/// ```
pub trait SpiBus: Send {
    /// Clock `len` bytes out of the device and return them, first byte first.
    ///
    /// The answer is exactly `len` bytes or an error; a backend never returns a short
    /// frame. The deadline is the `deadline` argument and bounds the whole call, including
    /// any wait for a transfer already in progress.
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if `deadline` has already passed (nothing is clocked) or
    ///   passes before the transfer completes.
    /// - [`ErrorKind::Disconnected`] if the bus or device cannot be used: it was closed,
    ///   removed, or the transfer failed.
    /// - [`ErrorKind::InvalidConfig`] if `len` is zero or more than the backend can carry
    ///   in one transfer.
    ///
    /// # Cancel safety
    /// Cancel-safe in effect: a read transmits nothing the device acts on, so a dropped or
    /// timed-out call leaves no state behind. A transfer already started may still finish
    /// on the bus; its bytes are discarded and never returned to a later call.
    ///
    /// # Wire effects
    /// Asserts chip select and clocks `len` bytes in. The data line toward the device
    /// carries no meaningful bytes.
    fn read(&mut self, len: usize, deadline: Deadline) -> OpFuture<'_, Bytes>;
}

/// The fault flags of one MAX31855 frame.
///
/// The converter sets one flag for each fault it sees and sets the frame's fault bit when any
/// flag is set; the datasheet does not say the flags exclude one another, so all three are kept
/// as the converter reported them. A [`ThermocoupleReading`] carries one only when at least
/// one flag is set.
///
/// # Examples
/// ```
/// use stonecrab::boards::ThermocoupleFault;
///
/// let fault = ThermocoupleFault::default();
/// assert_eq!(fault.code(), 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub struct ThermocoupleFault {
    /// The thermocouple is not connected (flag OC, bit 0).
    pub open: bool,
    /// The thermocouple is shorted to ground (flag SCG, bit 1).
    pub short_to_ground: bool,
    /// The thermocouple is shorted to the supply (flag SCV, bit 2).
    pub short_to_vcc: bool,
}

impl ThermocoupleFault {
    /// The three flags as the converter laid them out: bit 0 open, bit 1 short to ground,
    /// bit 2 short to Vcc. This is the value of the `thermocouple_fault` measurement.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::boards::ThermocoupleReading;
    /// use std::time::{Instant, SystemTime};
    ///
    /// // Frame bit 16 (fault) and bit 1 (short to ground).
    /// let reading = ThermocoupleReading::from_raw(0x0001_0002, Instant::now(), SystemTime::UNIX_EPOCH)?;
    /// let fault = reading.fault.expect("a fault");
    /// assert!(fault.short_to_ground && !fault.open && !fault.short_to_vcc);
    /// assert_eq!(fault.code(), 0b010);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn code(self) -> u8 {
        u8::from(self.open)
            | (u8::from(self.short_to_ground) << 1)
            | (u8::from(self.short_to_vcc) << 2)
    }
}

/// One decoded MAX31855 frame, with the time it was read.
///
/// The temperatures are kept as the integer counts the converter sent, so nothing is
/// rounded: [`thermocouple_quarters`](Self::thermocouple_quarters) counts quarters of a
/// degree Celsius (14 bits, two's complement) and
/// [`cold_junction_sixteenths`](Self::cold_junction_sixteenths) counts sixteenths of one (12
/// bits, two's complement). Both are sign-extended, so a negative temperature is a negative
/// count. [`thermocouple_celsius`](Self::thermocouple_celsius) and
/// [`cold_junction_celsius`](Self::cold_junction_celsius) convert them exactly; the
/// converter's accuracy is the datasheet's business, not this type's.
///
/// A frame is accepted only if it could have come from a working converter: both reserved
/// bits read 0, the fault bit agrees with the three fault flags, and the frame is neither all
/// zeros nor all ones (see [`ThermocoupleReading::from_raw`]).
///
/// # Examples
/// ```
/// use std::time::{Instant, SystemTime};
/// use stonecrab::boards::ThermocoupleReading;
///
/// // -0.25 C at the thermocouple, +25.0 C inside the chip.
/// let reading = ThermocoupleReading::from_raw(0xFFFC_1900, Instant::now(), SystemTime::UNIX_EPOCH)?;
/// assert_eq!(reading.thermocouple_quarters, -1);
/// assert_eq!(reading.thermocouple_celsius(), Some(-0.25));
/// assert_eq!(reading.cold_junction_celsius(), 25.0);
/// assert!(reading.fault.is_none());
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ThermocoupleReading {
    /// The 32-bit frame as the converter sent it, first bit shifted out in bit 31.
    pub raw: u32,
    /// The thermocouple temperature in quarters of a degree Celsius (bits 31 to 18).
    pub thermocouple_quarters: i16,
    /// The internal (cold-junction) temperature in sixteenths of a degree Celsius (bits 15
    /// to 4).
    pub cold_junction_sixteenths: i16,
    /// The faults the converter flagged, or `None` when its fault bit was clear.
    pub fault: Option<ThermocoupleFault>,
    /// The monotonic time, from the adapter's clock, at which the transfer completed.
    pub sampled_at: Instant,
    /// The wall-clock time, from the adapter's clock, at which the transfer completed.
    pub received_at: SystemTime,
}

impl ThermocoupleReading {
    /// Decode a 32-bit frame, stamping it with the time it was read.
    ///
    /// The layout is the MAX31855's: bits 31 to 18 the thermocouple temperature (0.25 C per
    /// count), bit 17 reserved, bit 16 the fault bit, bits 15 to 4 the internal temperature
    /// (0.0625 C per count), bit 3 reserved, bit 2 short to Vcc, bit 1 short to ground, bit 0
    /// open. A frame is refused, with no reading, when:
    /// - it is all zeros or all ones: nothing answered. A floating data line reads as all
    ///   ones and a line held low as all zeros, and the converter can send neither (all ones
    ///   sets both reserved bits). The price is that a real reading of exactly 0 C at both
    ///   junctions is refused too; the die would have to sit at exactly 0.0000 C. The error is
    ///   [`ErrorKind::Disconnected`].
    /// - a reserved bit is set: the bus is corrupt or the device is not a MAX31855. The error
    ///   is [`ErrorKind::MalformedData`].
    /// - the fault bit disagrees with the three flags (set with none, or clear with one): the
    ///   converter sets the bit exactly when a flag is set, so the frame is corrupt. The
    ///   error is [`ErrorKind::MalformedData`].
    ///
    /// # Errors
    /// [`ErrorKind::Disconnected`] for an all-zeros or all-ones frame and
    /// [`ErrorKind::MalformedData`] for the other two cases. The message names the frame in
    /// hexadecimal.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::ErrorKind;
    /// use stonecrab::boards::ThermocoupleReading;
    ///
    /// let (now, wall) = (Instant::now(), SystemTime::UNIX_EPOCH);
    /// // +1000.75 C at the thermocouple, +25.0 C inside the chip.
    /// let reading = ThermocoupleReading::from_raw(0x3E8C_1900, now, wall)?;
    /// assert_eq!(reading.thermocouple_celsius(), Some(1000.75));
    /// // A floating data line reads as all ones.
    /// let error = ThermocoupleReading::from_raw(u32::MAX, now, wall).expect_err("no device");
    /// assert_eq!(error.kind(), ErrorKind::Disconnected);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn from_raw(raw: u32, sampled_at: Instant, received_at: SystemTime) -> Result<Self> {
        if raw == 0 || raw == u32::MAX {
            return Err(failure(
                ErrorKind::Disconnected,
                FailureCategory::Transport,
                format!(
                    "max31855: frame {raw:#010x} is all zeros or all ones, so no device answered"
                ),
            ));
        }
        if raw & (RESERVED_HIGH | RESERVED_LOW) != 0 {
            return Err(failure(
                ErrorKind::MalformedData,
                FailureCategory::Protocol,
                format!("max31855: frame {raw:#010x} has a reserved bit set"),
            ));
        }
        let flags = ThermocoupleFault {
            open: raw & OPEN != 0,
            short_to_ground: raw & SHORT_TO_GROUND != 0,
            short_to_vcc: raw & SHORT_TO_VCC != 0,
        };
        let flagged = flags.code() != 0;
        if flagged != (raw & FAULT != 0) {
            return Err(failure(
                ErrorKind::MalformedData,
                FailureCategory::Protocol,
                format!(
                    "max31855: frame {raw:#010x} has a fault bit that disagrees with its fault flags"
                ),
            ));
        }
        // The frame is big-endian. Reading the top and bottom halves as signed 16-bit numbers
        // and shifting right arithmetically sign-extends the 14-bit and 12-bit fields.
        let [a, b, c, d] = raw.to_be_bytes();
        Ok(Self {
            raw,
            thermocouple_quarters: i16::from_be_bytes([a, b]) >> 2,
            cold_junction_sixteenths: i16::from_be_bytes([c, d]) >> 4,
            fault: flagged.then_some(flags),
            sampled_at,
            received_at,
        })
    }

    /// The thermocouple temperature in degrees Celsius, or `None` while a fault is flagged.
    ///
    /// With a fault the converter's temperature bits do not describe a measurement, so none
    /// is offered; [`thermocouple_quarters`](Self::thermocouple_quarters) still holds the bits.
    /// The conversion is exact: a count of quarters is a multiple of 0.25.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::boards::ThermocoupleReading;
    ///
    /// let (now, wall) = (Instant::now(), SystemTime::UNIX_EPOCH);
    /// let cold = ThermocoupleReading::from_raw(0xF060_1900, now, wall)?;
    /// assert_eq!(cold.thermocouple_celsius(), Some(-250.0));
    /// // The open-circuit flag and the fault bit.
    /// let open = ThermocoupleReading::from_raw(0x0001_1901, now, wall)?;
    /// assert_eq!(open.thermocouple_celsius(), None);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn thermocouple_celsius(&self) -> Option<f64> {
        self.fault
            .is_none()
            .then(|| f64::from(self.thermocouple_quarters) * 0.25)
    }

    /// The internal (cold-junction) temperature in degrees Celsius, exact.
    ///
    /// A fault on the thermocouple does not touch the internal sensor, so this is available
    /// whenever the frame was accepted.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::boards::ThermocoupleReading;
    ///
    /// // +25.0 C at the thermocouple, -25.0 C inside the chip.
    /// let reading = ThermocoupleReading::from_raw(0x0190_E700, Instant::now(), SystemTime::UNIX_EPOCH)?;
    /// assert_eq!(reading.cold_junction_celsius(), -25.0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn cold_junction_celsius(&self) -> f64 {
        f64::from(self.cold_junction_sixteenths) * 0.0625
    }

    /// The reading as three measurements of the sensors envelope, in this order:
    /// `thermocouple_temperature`, `cold_junction_temperature` and `thermocouple_fault`.
    ///
    /// - **`thermocouple_temperature`** (`Cel`, `F64`): quality `Good`; with a fault flagged
    ///   its quality is `Bad` and its value `None`, so a consumer that ignores quality
    ///   still cannot read a temperature that is not one. Its `raw` holds the `I16` count of
    ///   quarter degrees in both cases.
    /// - **`cold_junction_temperature`** (`Cel`, `F64`): quality `Good`, with or without a
    ///   thermocouple fault. Its `raw` holds the `I16` count of sixteenths.
    /// - **`thermocouple_fault`** (`1`, `U8`): the flags of
    ///   [`ThermocoupleFault::code`], 0 when none is set. Its quality is `Good`: it reports the
    ///   state faithfully, and a value other than 0 is the fault, kept visible beside the
    ///   `Bad` temperature.
    ///
    /// Every measurement has `origin` [`ValidationStatus::Documented`] (the decoding follows
    /// the datasheet; no hardware record exists), `received_mono` the time of the transfer,
    /// `received_at` the wall time of it, no source time (the converter stamps none) and no
    /// protocol status. A read that failed produces no reading and so no measurement, which
    /// is how [`StaleTracker`](crate::sensors::StaleTracker) lets an earlier value age; a
    /// `Bad` measurement without a value is not retained there either.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if a measurement name were not a valid tag identifier;
    /// the names are constants, so this does not happen.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab::PlcValue;
    /// use stonecrab::boards::ThermocoupleReading;
    /// use stonecrab::sensors::Quality;
    ///
    /// // An open thermocouple on a chip at 25.0 C.
    /// let reading = ThermocoupleReading::from_raw(0x0001_1901, Instant::now(), SystemTime::UNIX_EPOCH)?;
    /// let measurements = reading.measurements()?;
    /// assert_eq!(measurements[0].name, "thermocouple_temperature");
    /// assert_eq!((measurements[0].quality, &measurements[0].value), (Quality::Bad, &None));
    /// assert_eq!(measurements[1].value, Some(PlcValue::F64(25.0)));
    /// assert_eq!(measurements[2].value, Some(PlcValue::U8(0b001)));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn measurements(&self) -> Result<Vec<Measurement>> {
        let code = self.fault.map_or(0, ThermocoupleFault::code);
        let thermocouple_quality = if self.fault.is_some() {
            Quality::Bad
        } else {
            Quality::Good
        };
        [
            (
                THERMOCOUPLE,
                "Cel",
                self.thermocouple_celsius().map(PlcValue::F64),
                PlcValue::I16(self.thermocouple_quarters),
                thermocouple_quality,
            ),
            (
                COLD_JUNCTION,
                "Cel",
                Some(PlcValue::F64(self.cold_junction_celsius())),
                PlcValue::I16(self.cold_junction_sixteenths),
                Quality::Good,
            ),
            (
                FAULT_CODE,
                "1",
                Some(PlcValue::U8(code)),
                PlcValue::U8(code),
                Quality::Good,
            ),
        ]
        .into_iter()
        .map(|(name, unit, value, raw, quality)| self.measurement(name, unit, value, raw, quality))
        .collect()
    }

    fn measurement(
        &self,
        name: &str,
        unit: &str,
        value: Option<PlcValue>,
        raw: PlcValue,
        quality: Quality,
    ) -> Result<Measurement> {
        Ok(Measurement {
            tag: TagId::new(name)?,
            name: name.to_owned(),
            value,
            raw: Some(raw),
            unit: unit.to_owned(),
            quality,
            origin: ValidationStatus::Documented,
            received_at: self.received_at,
            received_mono: self.sampled_at,
            source_time: None,
            protocol_status: None,
            quality_before_stale: None,
        })
    }
}

/// A MAX31855 thermocouple converter on an SPI bus.
///
/// The adapter owns the bus and the clock it stamps readings with. [`read`](Self::read)
/// clocks one 32-bit frame out of the converter and decodes it; the result converts to
/// measurements with [`ThermocoupleReading::measurements`]. The converter has no
/// configuration and no registers, so the adapter writes nothing: every call is a read. The
/// chip converts continuously (a conversion takes on the order of 100 ms, per the datasheet),
/// so reading faster than that returns the same value again.
///
/// The tests behind this type decode the datasheet's examples and drive the bus through a
/// scripted double; none of them establishes calibration accuracy, and no hardware record
/// exists yet (`docs/hardware/max31855-spi.md` is the runbook for one).
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline};
/// use stonecrab::boards::Max31855;
/// use stonecrab::testkit::{FakeClock, FakeSpiBus, SpiStep, block_on_with};
///
/// let clock = FakeClock::default();
/// // +1000.75 C at the thermocouple, +25.0 C inside the chip.
/// let bus = FakeSpiBus::new(vec![SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00])], clock.clone());
/// let mut sensor = Max31855::new(Box::new(bus), Arc::new(clock.clone()));
///
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
/// let reading = block_on_with(&clock, sensor.read(deadline))?;
/// assert_eq!(reading.thermocouple_celsius(), Some(1000.75));
/// assert_eq!(reading.measurements()?.len(), 3);
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct Max31855 {
    bus: Box<dyn SpiBus>,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for Max31855 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Max31855").finish_non_exhaustive()
    }
}

impl Max31855 {
    /// A converter on `bus`, stamping its readings with `clock`.
    ///
    /// Derive the deadlines passed to [`read`](Self::read) from the same clock.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::boards::{Max31855, SpiBus};
    /// use stonecrab::Clock;
    ///
    /// fn adapter(bus: Box<dyn SpiBus>, clock: Arc<dyn Clock>) -> Max31855 {
    ///     Max31855::new(bus, clock)
    /// }
    /// # let _ = adapter;
    /// ```
    #[must_use]
    pub fn new(bus: Box<dyn SpiBus>, clock: Arc<dyn Clock>) -> Self {
        Self { bus, clock }
    }

    /// Read and decode one frame, within `deadline`.
    ///
    /// The reading is stamped after the transfer completes. The deadline is the `deadline`
    /// argument.
    ///
    /// # Errors
    /// - The bus's own errors, unchanged: [`ErrorKind::Timeout`] when the deadline passes
    ///   (the bus is read at most once, so a late bus never yields a stale frame later) and
    ///   [`ErrorKind::Disconnected`] when it fails.
    /// - [`ErrorKind::MalformedData`] if the bus returned other than four bytes.
    /// - The errors of [`ThermocoupleReading::from_raw`]: [`ErrorKind::Disconnected`] for an
    ///   all-zeros or all-ones frame (no device answered) and [`ErrorKind::MalformedData`] for
    ///   a reserved bit or an inconsistent fault bit.
    ///
    /// A converter fault (open or short) is not an error: it is a reading with `fault` set.
    ///
    /// # Cancel safety
    /// Cancel-safe: see [`SpiBus::read`]. A dropped call returns nothing, and the next call
    /// reads a fresh frame.
    ///
    /// # Wire effects
    /// One four-byte transfer on the bus. The converter is read-only and unaffected.
    pub fn read(&mut self, deadline: Deadline) -> OpFuture<'_, ThermocoupleReading> {
        Box::pin(async move {
            let bytes = self.bus.read(FRAME_BYTES, deadline).await?;
            let sampled_at = self.clock.now();
            let received_at = self.clock.wall_now();
            let Ok(frame) = <[u8; FRAME_BYTES]>::try_from(bytes.as_ref()) else {
                return Err(failure(
                    ErrorKind::MalformedData,
                    FailureCategory::Protocol,
                    format!(
                        "max31855: the bus returned {} bytes, not {FRAME_BYTES}",
                        bytes.len()
                    ),
                ));
            };
            ThermocoupleReading::from_raw(u32::from_be_bytes(frame), sampled_at, received_at)
        })
    }
}

#[cfg(feature = "adapters-linux")]
mod linux {
    use std::fs::File;
    use std::io::{self, Read};
    use std::path::Path;
    use std::thread;

    use bytes::Bytes;
    use tokio::sync::{mpsc, oneshot};

    use super::{SpiBus, failure};
    use crate::{Deadline, ErrorKind, FailureCategory, OpFuture, PlcError, Result};

    /// The most bytes one transfer carries: the default buffer of the Linux `spidev` driver
    /// (`bufsiz`, one page), beyond which it refuses a read.
    const MAX_READ_BYTES: usize = 4096;

    /// Commands queued ahead of the worker. A bus has one owner and runs one call at a time, so
    /// the queue holds the live command plus one abandoned by a call that timed out; a full queue
    /// makes the next call wait inside its deadline.
    const COMMAND_CAPACITY: usize = 2;

    struct Command {
        len: usize,
        done: oneshot::Sender<io::Result<Vec<u8>>>,
    }

    /// An SPI device node of a Linux host, read from a worker thread.
    ///
    /// `/dev/spidevB.C` is read with plain `read(2)`: the kernel asserts chip select, clocks
    /// the bytes in and releases it, as one transfer. That needs no `ioctl` and so no
    /// dependency and no code outside the standard library, and it is all a receive-only
    /// device such as the MAX31855 needs. The cost is that nothing here configures the bus: the clock mode and the speed
    /// are whatever the device tree and the driver gave the node (mode 0 and a rate no higher
    /// than the converter's 5 MHz for the MAX31855; `docs/hardware/max31855-spi.md`).
    ///
    /// A thread owns the file and blocks in the read, because a read of a device node blocks
    /// (coding standards 8.4); the async side sends it a command and races the answer against
    /// the deadline. The thread ends when the `LinuxSpiDev` is dropped, after the transfer it
    /// is in; nothing waits for it. A call that timed out leaves its command to be skipped if
    /// the worker has not started it, or finished and discarded if it has.
    ///
    /// The type is built from any [`Read`], so the tests run it without a device;
    /// [`open`](Self::open) is the Linux-specific part.
    ///
    /// # Examples
    /// ```no_run
    /// use stonecrab::boards::{LinuxSpiDev, Max31855};
    /// use stonecrab::transport::SystemClock;
    /// use std::sync::Arc;
    ///
    /// let bus = LinuxSpiDev::open("/dev/spidev0.0")?;
    /// let sensor = Max31855::new(Box::new(bus), Arc::new(SystemClock::default()));
    /// # let _ = sensor;
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[derive(Debug)]
    pub struct LinuxSpiDev {
        commands: mpsc::Sender<Command>,
    }

    fn io_failure(error: io::Error) -> PlcError {
        let message = format!("spi device: {error}");
        failure(ErrorKind::Disconnected, FailureCategory::Transport, message).with_source(error)
    }

    fn gone() -> PlcError {
        failure(
            ErrorKind::Disconnected,
            FailureCategory::Transport,
            "spi device: the worker has stopped",
        )
    }

    fn timed_out(detail: &str) -> PlcError {
        failure(
            ErrorKind::Timeout,
            FailureCategory::Transport,
            format!("spi device: {detail}"),
        )
    }

    impl LinuxSpiDev {
        /// Open the device node at `path` for reading, for example `/dev/spidev0.0`.
        ///
        /// Opens the file (one `open(2)`, which returns at once for a device node) and starts
        /// the worker thread. Call it from setup code, not from a latency-sensitive task.
        ///
        /// # Errors
        /// [`ErrorKind::Disconnected`] if the node cannot be opened (it does not exist, the
        /// process may not read it) or the thread cannot be started; the operating system's error
        /// is kept as the source.
        ///
        /// # Examples
        /// ```no_run
        /// use stonecrab::boards::LinuxSpiDev;
        ///
        /// let bus = LinuxSpiDev::open("/dev/spidev0.0")?;
        /// # let _ = bus;
        /// # Ok::<(), stonecrab::PlcError>(())
        /// ```
        pub fn open(path: impl AsRef<Path>) -> Result<Self> {
            Self::from_device(File::open(path).map_err(io_failure)?)
        }

        /// Wrap an already open device, or any reader that behaves like one.
        ///
        /// Each `read` of `device` must be one whole transfer: it fills the buffer it is given or
        /// fails. A [`File`] on a `spidev` node does; the tests pass an in-memory reader.
        ///
        /// # Errors
        /// [`ErrorKind::Disconnected`] if the worker thread cannot be started.
        ///
        /// # Examples
        /// ```
        /// use stonecrab::boards::LinuxSpiDev;
        ///
        /// let bus = LinuxSpiDev::from_device(std::io::Cursor::new(vec![0x3E, 0x8C, 0x19, 0x00]))?;
        /// # let _ = bus;
        /// # Ok::<(), stonecrab::PlcError>(())
        /// ```
        pub fn from_device<R>(device: R) -> Result<Self>
        where
            R: Read + Send + 'static,
        {
            let (commands, receiver) = mpsc::channel(COMMAND_CAPACITY);
            thread::Builder::new()
                .name("stonecrab-spi".to_owned())
                .spawn(move || run(device, receiver))
                .map_err(io_failure)?;
            Ok(Self { commands })
        }
    }

    /// The worker loop: one transfer per command until the bus is dropped.
    fn run<R: Read>(mut device: R, mut commands: mpsc::Receiver<Command>) {
        while let Some(Command { len, done }) = commands.blocking_recv() {
            if done.is_closed() {
                continue;
            }
            let mut buffer = vec![0_u8; len];
            let result = device.read_exact(&mut buffer).map(|()| buffer);
            drop(done.send(result));
        }
    }

    impl SpiBus for LinuxSpiDev {
        /// Reads `len` bytes in one transfer, within `deadline`.
        ///
        /// `len` is at most 4096. The deadline is judged on Tokio's clock and is the `deadline`
        /// argument; the future needs a Tokio runtime with the time driver.
        ///
        /// # Cancel safety
        /// Cancel-safe in effect; a transfer already in the kernel finishes and its bytes are
        /// dropped.
        ///
        /// # Wire effects
        /// One `read(2)` of the device node, which the kernel turns into one chip-select cycle.
        fn read(&mut self, len: usize, deadline: Deadline) -> OpFuture<'_, Bytes> {
            Box::pin(async move {
                if len == 0 || len > MAX_READ_BYTES {
                    return Err(failure(
                        ErrorKind::InvalidConfig,
                        FailureCategory::Validation,
                        format!("spi device: a transfer is 1 to {MAX_READ_BYTES} bytes, not {len}"),
                    ));
                }
                if deadline.is_elapsed(tokio::time::Instant::now().into_std()) {
                    return Err(timed_out("deadline already elapsed"));
                }
                let transfer = async {
                    let (done, reply) = oneshot::channel();
                    self.commands
                        .send(Command { len, done })
                        .await
                        .map_err(|_closed| gone())?;
                    reply.await.map_err(|_closed| gone())?.map_err(io_failure)
                };
                let limit = tokio::time::Instant::from_std(deadline.instant());
                match tokio::time::timeout_at(limit, transfer).await {
                    Ok(done) => done.map(Bytes::from),
                    Err(_elapsed) => Err(timed_out("deadline elapsed")),
                }
            })
        }
    }
}

#[cfg(feature = "adapters-linux")]
#[cfg_attr(docsrs, doc(cfg(feature = "adapters-linux")))]
pub use linux::LinuxSpiDev;
