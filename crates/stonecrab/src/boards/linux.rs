//! The Linux SPI backend: a `/dev/spidev` node read on a worker thread (ADR-0026).

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
                // A send to a stopped worker hands the command back and drops it, and `done`
                // with it, so the reply below reports a stopped worker either way: before the
                // command was queued or while it waited.
                drop(self.commands.send(Command { len, done }).await);
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
