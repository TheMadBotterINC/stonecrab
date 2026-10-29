use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use bytes::Bytes;

use crate::boards::I2cBus;
use crate::testkit::FakeClock;
use crate::testkit::fake_transport::failure;
use crate::{Clock, Deadline, ErrorKind, OpFuture, Operation};

/// What the next transaction on a [`FakeI2cBus`] does.
///
/// A device that does not acknowledge (absent, unpowered, at another address) is
/// `Fail(ErrorKind::Disconnected)`, which is what [`I2cBus`] specifies for a NACK.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::ErrorKind;
/// use stonecrab::testkit::I2cStep;
///
/// // A register answers, a write is acknowledged 5 ms late, the device stops acknowledging,
/// // then it stops answering at all.
/// let script = vec![
///     I2cStep::Reply(vec![0xE5]),
///     I2cStep::Delay(Duration::from_millis(5)),
///     I2cStep::Ack,
///     I2cStep::Fail(ErrorKind::Disconnected),
///     I2cStep::Silence,
/// ];
/// assert_eq!(script.len(), 5);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum I2cStep {
    /// The device acknowledged: a write completes, and a write-read returns no bytes.
    Ack,
    /// A write-read returns these bytes whatever length was asked for, so a script can also
    /// play a short or a long answer; a write given this step completes and ignores them.
    Reply(Vec<u8>),
    /// Fail with an error of this kind, as a bus that cannot carry the transaction does.
    Fail(ErrorKind),
    /// Never answer: the transaction waits out its deadline on the fake clock and times out.
    Silence,
    /// Pause before the next step: the transaction waits this long on the fake clock, then
    /// plays the step after this one, as a host or a bus that stalled would. A transaction
    /// whose deadline passes first times out at the deadline.
    Delay(Duration),
}

/// One transaction a [`FakeI2cBus`] was asked for, as it recorded it.
///
/// # Examples
/// ```
/// use stonecrab::testkit::I2cTransfer;
///
/// // A read of one register: its address written, one byte read back.
/// let read = I2cTransfer { address: 0x53, write: vec![0x00], read: 1 };
/// // A write of one register.
/// let write = I2cTransfer { address: 0x53, write: vec![0x2D, 0x08], read: 0 };
/// assert_ne!(read, write);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I2cTransfer {
    /// The 7-bit address the transaction was sent to.
    pub address: u8,
    /// The bytes written: for a register device, the register address and any values.
    pub write: Vec<u8>,
    /// The bytes asked for after the repeated start; 0 for a plain write.
    pub read: usize,
}

#[derive(Debug)]
struct State {
    steps: VecDeque<I2cStep>,
    transfers: Vec<I2cTransfer>,
}

/// An in-memory [`I2cBus`] that plays a script of [`I2cStep`]s on a [`FakeClock`] and records
/// every transaction.
///
/// Each transaction takes the next step; a script that has run out behaves like
/// [`I2cStep::Silence`], as a device that stopped answering would. A transaction whose
/// deadline has already passed fails with `Timeout` before it takes a step and is not
/// recorded. Nothing sleeps on real time: a silent transaction sleeps on the fake clock until
/// its deadline, so [`block_on_with`](crate::testkit::block_on_with) drives it. The bus does
/// not check the address or the lengths; it plays the script.
///
/// Clones share one script and one record, so a test keeps a clone to inspect
/// [`transfers`](Self::transfers) after the other has been boxed into an adapter.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use bytes::Bytes;
/// use stonecrab::{Clock, Deadline, ErrorKind};
/// use stonecrab::boards::I2cBus;
/// use stonecrab::testkit::{FakeClock, FakeI2cBus, I2cStep, I2cTransfer, block_on_with};
///
/// let clock = FakeClock::default();
/// let mut bus = FakeI2cBus::new(vec![I2cStep::Reply(vec![0xE5]), I2cStep::Ack], clock.clone());
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
///
/// let id = block_on_with(&clock, bus.write_read(0x53, Bytes::from_static(&[0x00]), 1, deadline))?;
/// assert_eq!(&id[..], &[0xE5]);
/// block_on_with(&clock, bus.write(0x53, Bytes::from_static(&[0x2D, 0x08]), deadline))?;
/// // The script is spent, so the bus is silent and the transaction times out at the deadline.
/// let error = block_on_with(&clock, bus.write(0x53, Bytes::from_static(&[0x2D, 0x00]), deadline))
///     .expect_err("silent");
/// assert_eq!(error.kind(), ErrorKind::Timeout);
/// assert_eq!(bus.transfers()[1], I2cTransfer { address: 0x53, write: vec![0x2D, 0x08], read: 0 });
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone)]
pub struct FakeI2cBus {
    state: Arc<Mutex<State>>,
    clock: FakeClock,
}

impl FakeI2cBus {
    /// A bus that plays `steps` in order, timing its waits on `clock`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeI2cBus};
    ///
    /// let bus = FakeI2cBus::new(Vec::new(), FakeClock::default());
    /// assert!(bus.transfers().is_empty());
    /// ```
    #[must_use]
    pub fn new(steps: Vec<I2cStep>, clock: FakeClock) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                steps: steps.into(),
                transfers: Vec::new(),
            })),
            clock,
        }
    }

    /// Every transaction that took a step, in order.
    ///
    /// A transaction refused for an elapsed deadline took no step and is not listed.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeI2cBus};
    ///
    /// assert!(FakeI2cBus::new(Vec::new(), FakeClock::default()).transfers().is_empty());
    /// ```
    #[must_use]
    pub fn transfers(&self) -> Vec<I2cTransfer> {
        self.with(|state| state.transfers.clone())
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }

    /// Records the transaction and plays the next step.
    async fn play(&self, transfer: I2cTransfer, deadline: Deadline) -> crate::Result<Bytes> {
        const SITE: &str = "fake i2c";
        let operation = Some(if transfer.read == 0 {
            Operation::Write
        } else {
            Operation::Read
        });
        if deadline.is_elapsed(self.clock.now()) {
            return Err(failure(
                ErrorKind::Timeout,
                operation,
                SITE,
                "deadline already elapsed",
            ));
        }
        let mut step = self.with(|state| {
            state.transfers.push(transfer);
            state.steps.pop_front()
        });
        while let Some(I2cStep::Delay(by)) = step {
            let resume = self
                .clock
                .now()
                .checked_add(by)
                .map_or(deadline.instant(), |at| at.min(deadline.instant()));
            self.clock.sleep_until(resume).await?;
            step = if deadline.is_elapsed(self.clock.now()) {
                Some(I2cStep::Silence)
            } else {
                self.with(|state| state.steps.pop_front())
            };
        }
        match step {
            Some(I2cStep::Ack) => Ok(Bytes::new()),
            Some(I2cStep::Reply(bytes)) => Ok(Bytes::from(bytes)),
            Some(I2cStep::Fail(kind)) => Err(failure(kind, operation, SITE, "scripted failure")),
            Some(I2cStep::Silence | I2cStep::Delay(_)) | None => {
                self.clock.sleep_until(deadline.instant()).await?;
                Err(failure(
                    ErrorKind::Timeout,
                    operation,
                    SITE,
                    "deadline elapsed",
                ))
            }
        }
    }
}

impl I2cBus for FakeI2cBus {
    /// Plays the next step as a write, within `deadline`.
    ///
    /// # Cancel safety
    /// The step is taken and the transaction recorded only when the future is polled; a
    /// silent transaction takes its step before it waits, so a dropped one has used it.
    ///
    /// # Wire effects
    /// None; the transaction is appended to [`FakeI2cBus::transfers`].
    fn write(&mut self, address: u8, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            let transfer = I2cTransfer {
                address,
                write: bytes.to_vec(),
                read: 0,
            };
            self.play(transfer, deadline).await.map(drop)
        })
    }

    /// Plays the next step as a write-read, within `deadline`.
    ///
    /// # Cancel safety
    /// As [`write`](Self::write).
    ///
    /// # Wire effects
    /// None; the transaction is appended to [`FakeI2cBus::transfers`].
    fn write_read(
        &mut self,
        address: u8,
        bytes: Bytes,
        len: usize,
        deadline: Deadline,
    ) -> OpFuture<'_, Bytes> {
        Box::pin(async move {
            let transfer = I2cTransfer {
                address,
                write: bytes.to_vec(),
                read: len,
            };
            self.play(transfer, deadline).await
        })
    }
}
