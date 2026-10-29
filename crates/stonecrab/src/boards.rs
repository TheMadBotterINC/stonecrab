#![doc = include_str!("../docs/components/boards.md")]

use std::time::{Instant, SystemTime};

use bytes::Bytes;

use crate::sensors::{Measurement, Quality, ValidationStatus};
use crate::{
    Deadline, Diagnostics, ErrorKind, FailureCategory, OpFuture, Operation, PlcError, PlcValue,
    Result, TagId,
};

mod adxl345;
#[cfg(feature = "adapters-linux")]
mod linux;
mod max31855;

pub use adxl345::{
    AccelRange, AccelReading, AccelSample, Adxl345, Adxl345Config, FifoMode, OutputRate,
    SampleBlock,
};
#[cfg(feature = "adapters-linux")]
#[cfg_attr(docsrs, doc(cfg(feature = "adapters-linux")))]
pub use linux::LinuxSpiDev;
pub use max31855::{Max31855, ThermocoupleFault, ThermocoupleReading};

/// A failure on a board adapter: a read of the sensor, with the category of what went wrong.
fn failure(kind: ErrorKind, category: FailureCategory, message: impl Into<String>) -> PlcError {
    let diagnostics = Diagnostics {
        operation: Some(Operation::Read),
        category: Some(category),
        ..Diagnostics::default()
    };
    PlcError::new(kind, message).with_diagnostics(diagnostics)
}

/// One measurement of a board reading in the sensors envelope: origin `Documented` (written
/// from a datasheet, no hardware record), no source time (the chips stamp none) and no
/// protocol status.
fn measurement(
    name: &str,
    unit: &str,
    value: Option<PlcValue>,
    raw: PlcValue,
    quality: Quality,
    received_mono: Instant,
    received_at: SystemTime,
) -> Result<Measurement> {
    Ok(Measurement {
        tag: TagId::new(name)?,
        name: name.to_owned(),
        value,
        raw: Some(raw),
        unit: Some(unit.to_owned()),
        quality,
        origin: ValidationStatus::Documented,
        received_at,
        received_mono,
        source_time: None,
        protocol_status: None,
        quality_before_stale: None,
    })
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
/// A bus that carries register transactions to the devices on one I2C bus, within a
/// deadline.
///
/// The trait is the contract between a board adapter and the I2C controller under it:
/// [`Adxl345`] names a device by its 7-bit address and writes or reads its registers, a
/// backend (the testkit's `FakeI2cBus` in tests; no host backend ships yet, see ADR-0029)
/// carries the transaction or returns a typed error. A backend moves bytes and nothing else:
/// it does not interpret, retry or reconnect. The two methods are the two transactions a
/// register device needs: a write (the register address, then the values) and a write
/// followed by a read under one repeated start (the register address, then the bytes it
/// answers with).
///
/// The trait is object safe and `Send`, and one bus has exactly one owner (`&mut self`), so
/// transactions on it are serialized by construction.
///
/// # Examples
/// A function over any bus; [`Adxl345`] shows one running.
/// ```
/// use bytes::Bytes;
/// use stonecrab::{Deadline, Result};
/// use stonecrab::boards::I2cBus;
///
/// /// Read one register of the device at `address`.
/// async fn register(bus: &mut dyn I2cBus, address: u8, register: u8, deadline: Deadline)
///     -> Result<Bytes>
/// {
///     bus.write_read(address, Bytes::copy_from_slice(&[register]), 1, deadline).await
/// }
/// # let _ = register;
/// ```
pub trait I2cBus: Send {
    /// Write `bytes` to the device at the 7-bit `address` in one transaction.
    ///
    /// The deadline is the `deadline` argument and bounds the whole call, including any wait
    /// for a transaction already in progress.
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if `deadline` has already passed (nothing is sent) or passes
    ///   before the transaction completes.
    /// - [`ErrorKind::Disconnected`] if the device does not acknowledge its address or a byte
    ///   (absent, unpowered, at another address) or the bus cannot be used (closed, removed,
    ///   arbitration lost, a line held low).
    /// - [`ErrorKind::InvalidConfig`] if `address` is above `0x7F`, `bytes` is empty or longer
    ///   than the backend carries in one transaction.
    ///
    /// # Cancel safety
    /// Not cancel-safe: a dropped or timed-out call may or may not have reached the device,
    /// and a register it wrote stays written.
    ///
    /// # Wire effects
    /// Start, the address with the write bit, `bytes`, stop.
    fn write(&mut self, address: u8, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()>;

    /// Write `bytes` to the device at the 7-bit `address`, then read `len` bytes from it, in
    /// one transaction joined by a repeated start.
    ///
    /// The answer is exactly `len` bytes or an error; a backend never returns a short read.
    /// The deadline is the `deadline` argument and bounds the whole call.
    ///
    /// # Errors
    /// As [`write`](Self::write), and [`ErrorKind::InvalidConfig`] also if `len` is zero or
    /// more than the backend carries in one transaction.
    ///
    /// # Cancel safety
    /// Not cancel-safe: reading some registers changes the device (reading the ADXL345's data
    /// registers pops its FIFO), so a dropped call may have consumed what it would have
    /// returned.
    ///
    /// # Wire effects
    /// Start, the address with the write bit, `bytes`, a repeated start, the address with the
    /// read bit, `len` bytes clocked in, stop.
    fn write_read(
        &mut self,
        address: u8,
        bytes: Bytes,
        len: usize,
        deadline: Deadline,
    ) -> OpFuture<'_, Bytes>;
}
