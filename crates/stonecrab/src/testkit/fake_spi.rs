use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;

use crate::boards::SpiBus;
use crate::testkit::FakeClock;
use crate::testkit::fake_transport::failure;
use crate::{Clock, Deadline, ErrorKind, OpFuture, Operation};

/// What the next read of a [`FakeSpiBus`] does.
///
/// # Examples
/// ```
/// use stonecrab::ErrorKind;
/// use stonecrab::testkit::SpiStep;
///
/// // A good frame, a failing bus, then a device that stops answering.
/// let script = vec![
///     SpiStep::Reply(vec![0x3E, 0x8C, 0x19, 0x00]),
///     SpiStep::Fail(ErrorKind::Disconnected),
///     SpiStep::Silence,
/// ];
/// assert_eq!(script.len(), 3);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpiStep {
    /// Return these bytes whatever length was asked for, so a script can also play a short or
    /// a long answer.
    Reply(Vec<u8>),
    /// Fail with an error of this kind, as a bus that cannot carry the transfer does.
    Fail(ErrorKind),
    /// Never answer: the read waits out its deadline on the fake clock and times out.
    Silence,
}

#[derive(Debug)]
struct State {
    steps: VecDeque<SpiStep>,
    reads: Vec<usize>,
}

/// An in-memory [`SpiBus`] that plays a script of [`SpiStep`]s on a [`FakeClock`].
///
/// Each read takes the next step; a script that has run out behaves like
/// [`SpiStep::Silence`], as a device that stopped answering would. A read whose deadline has
/// already passed fails with `Timeout` before it takes a step. Nothing sleeps on real time:
/// a silent read sleeps on the fake clock until its deadline, so
/// [`block_on_with`](crate::testkit::block_on_with) drives it.
///
/// Clones share one script and one log, so a test keeps a clone to inspect
/// [`reads`](Self::reads) after the other has been boxed into an adapter.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline, ErrorKind};
/// use stonecrab::boards::SpiBus;
/// use stonecrab::testkit::{FakeClock, FakeSpiBus, SpiStep, block_on_with};
///
/// let clock = FakeClock::default();
/// let mut bus = FakeSpiBus::new(vec![SpiStep::Reply(vec![1, 2, 3, 4])], clock.clone());
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
///
/// let frame = block_on_with(&clock, bus.read(4, deadline))?;
/// assert_eq!(&frame[..], &[1, 2, 3, 4]);
/// // The script is spent, so the bus is silent and the read times out at the deadline.
/// let error = block_on_with(&clock, bus.read(4, deadline)).expect_err("silent");
/// assert_eq!(error.kind(), ErrorKind::Timeout);
/// assert_eq!(bus.reads(), vec![4, 4]);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone)]
pub struct FakeSpiBus {
    state: Arc<Mutex<State>>,
    clock: FakeClock,
}

impl FakeSpiBus {
    /// A bus that plays `steps` in order, timing its waits on `clock`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeSpiBus};
    ///
    /// let bus = FakeSpiBus::new(Vec::new(), FakeClock::default());
    /// assert!(bus.reads().is_empty());
    /// ```
    #[must_use]
    pub fn new(steps: Vec<SpiStep>, clock: FakeClock) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                steps: steps.into(),
                reads: Vec::new(),
            })),
            clock,
        }
    }

    /// The length asked for by each read that took a step, in order.
    ///
    /// A read refused for an elapsed deadline took no step and is not listed.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeSpiBus};
    ///
    /// assert_eq!(FakeSpiBus::new(Vec::new(), FakeClock::default()).reads(), Vec::<usize>::new());
    /// ```
    #[must_use]
    pub fn reads(&self) -> Vec<usize> {
        self.with(|state| state.reads.clone())
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }
}

impl SpiBus for FakeSpiBus {
    /// Plays the next step, within `deadline`.
    ///
    /// # Cancel safety
    /// Cancel-safe: the step is taken only when the future is polled and a silent read takes it
    /// before it waits, so a dropped silent read has already used its step.
    ///
    /// # Wire effects
    /// None; the length is appended to [`FakeSpiBus::reads`].
    fn read(&mut self, len: usize, deadline: Deadline) -> OpFuture<'_, Bytes> {
        Box::pin(async move {
            const SITE: &str = "fake spi";
            if deadline.is_elapsed(self.clock.now()) {
                return Err(failure(
                    ErrorKind::Timeout,
                    Some(Operation::Read),
                    SITE,
                    "deadline already elapsed",
                ));
            }
            let step = self.with(|state| {
                state.reads.push(len);
                state.steps.pop_front()
            });
            match step {
                Some(SpiStep::Reply(bytes)) => Ok(Bytes::from(bytes)),
                Some(SpiStep::Fail(kind)) => Err(failure(
                    kind,
                    Some(Operation::Read),
                    SITE,
                    "scripted failure",
                )),
                Some(SpiStep::Silence) | None => {
                    self.clock.sleep_until(deadline.instant()).await?;
                    Err(failure(
                        ErrorKind::Timeout,
                        Some(Operation::Read),
                        SITE,
                        "deadline elapsed",
                    ))
                }
            }
        })
    }
}
