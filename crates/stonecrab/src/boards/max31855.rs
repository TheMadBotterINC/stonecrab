//! The MAX31855 thermocouple converter on SPI (ADR-0026).

use std::fmt;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use super::{SpiBus, failure, measurement};
use crate::sensors::{Measurement, Quality};
use crate::{Clock, Deadline, ErrorKind, FailureCategory, OpFuture, PlcValue, Result};

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
    /// Every measurement has `origin`
    /// [`ValidationStatus::Documented`](crate::sensors::ValidationStatus::Documented) (the
    /// decoding follows the datasheet; no hardware record exists), `received_mono` the time of
    /// the transfer, `received_at` the wall time of it, no source time (the converter stamps
    /// none) and no protocol status. A read that failed produces no reading and so no
    /// measurement, which is how [`StaleTracker`](crate::sensors::StaleTracker) lets an
    /// earlier value age; a `Bad` measurement without a value is not retained there either.
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
        .map(|(name, unit, value, raw, quality)| {
            measurement(
                name,
                unit,
                value,
                raw,
                quality,
                self.sampled_at,
                self.received_at,
            )
        })
        .collect()
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
