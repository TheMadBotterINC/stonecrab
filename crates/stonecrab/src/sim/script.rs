//! The failure script: a bounded queue of scripted faults a [`SimDevice`](crate::sim::SimDevice)
//! hands to its connections, one per operation.

use std::collections::VecDeque;
use std::time::Duration;

use crate::{ErrorKind, PlcError, ProtocolStatus, Result, TagId};

/// One scripted fault, consumed by exactly one connect, read or write.
///
/// A connection pops at most one step per operation, at the point where the operation
/// would touch the device. Everything that is rejected before I/O (a foreign address, a
/// batch over the item limit, an already cancelled token, an elapsed deadline, a read-only
/// access policy, the single in-flight slot, a connection that is not open) happens before
/// a step is taken, so the step stays queued for the next operation that does reach the
/// device. `close` never consumes a step. Each attempt of a retried read pops its own step.
///
/// The semantics below hold for reads and writes; "the device is touched" means that reads
/// are served (ramps step, read counters count) and writes are applied. For `connect` see
/// [`Timeout`](Self::Timeout), [`Disconnect`](Self::Disconnect) and
/// [`Delay`](Self::Delay); every other step is consumed without effect on a connect, with a
/// `tracing` warning.
///
/// The enum is `#[non_exhaustive]`: a later slice may add steps.
///
/// # Compatibility: docs/parity/sim.md#SIM-FAILURE-SCRIPT
/// Stonecrab-owned. The upstream simulated driver has no failure injection.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::ProtocolStatus;
/// use stonecrab::sim::FailureStep;
///
/// let step = FailureStep::DeviceException(ProtocolStatus::new("sim", 4));
/// assert_ne!(step, FailureStep::Delay(Duration::from_millis(5)));
/// assert_eq!(step.clone(), step);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureStep {
    /// Wait this long on the connection's clock, then proceed normally.
    ///
    /// The wait is raced against the request's cancellation token, the connection being
    /// closed and the deadline exactly like the device latency, and comes before the
    /// latency wait. A wait that reaches the deadline is [`ErrorKind::Timeout`] with no
    /// device access; a cancelled token is [`ErrorKind::Cancelled`]. On `connect` there is
    /// no request token, so the connect deadline (`timeouts.connect`) is the only bound.
    Delay(Duration),
    /// The device is touched and then the reply is lost: "timeout after transmit".
    ///
    /// A read is served (counters count, ramps step) and a write is applied, then the
    /// connection waits on its clock until the deadline and fails the whole operation with
    /// [`ErrorKind::Timeout`] (category `Transport`) and no response. The connection stays
    /// `Connected`. A cancelled token ends the wait early with [`ErrorKind::Cancelled`].
    /// On `connect` the device is not touched: the connect waits for `timeouts.connect` and
    /// fails with `Timeout`, and the `seed` option is not applied.
    ///
    /// A read that is retried (a `RetryPolicy` that allows `Timeout`) runs its next
    /// attempt after this wait. A request without a deadline gets a fresh
    /// `timeouts.request` for every attempt, so each attempt can wait out its own step; an
    /// explicit deadline is shared by all attempts, so once it has passed no further
    /// attempt reaches the device.
    Timeout,
    /// The link is lost before the device is reached.
    ///
    /// The device is not touched. The connection enters
    /// [`ConnectionState::Failed`](crate::ConnectionState::Failed) with
    /// [`ErrorKind::Disconnected`] (before the error is built, so its diagnostics report
    /// that state) and the operation fails with [`ErrorKind::Disconnected`] (category
    /// `Transport`). Every later read or write fails with `Disconnected` before touching
    /// the device and consumes no step; `close` still moves the state to `Closed`. On
    /// `connect` the connection is dropped in `Failed(Disconnected)`.
    Disconnect,
    /// The device refuses the request with this status.
    ///
    /// The device is not touched. Every item of a read is an
    /// [`ItemError`](crate::ItemError) of kind [`ErrorKind::DeviceException`]
    /// carrying a clone of the status, and every item of a write is
    /// [`WriteOutcome::Rejected`](crate::WriteOutcome::Rejected) with that status.
    /// The call itself succeeds and the connection stays `Connected`.
    DeviceException(ProtocolStatus),
    /// The reply cannot be decoded.
    ///
    /// The device is not touched (the simulator replaces the reply before the device is
    /// consulted). Every item of a read is an item error of kind
    /// [`ErrorKind::MalformedData`] (category `Protocol`). A write fails as a whole with
    /// [`ErrorKind::MalformedData`], because a corrupted confirmation cannot be attributed
    /// to items; use [`UnknownAfterTransmit`](Self::UnknownAfterTransmit) to model a write
    /// that was applied but not confirmed.
    Malformed,
    /// The named items fail and the others are served normally.
    ///
    /// A named item of a read is an item error of kind [`ErrorKind::DeviceException`] with
    /// the status `ProtocolStatus::new("sim", 4)` and is not served (no counter counts, no
    /// ramp steps); a named item of a write is `Rejected` with that status and not
    /// applied. All other items behave as they would without a step. An identifier that is
    /// not in the request is ignored, and a step that names every item fails every item
    /// without failing the call.
    PartialFailure(Vec<TagId>),
    /// The simulator reports that it cannot accept the request.
    ///
    /// The device is not touched. The operation fails with [`ErrorKind::Backpressure`]
    /// (category `Validation`), the connection stays `Connected` and the in-flight slot is
    /// released. The kind is in the default retry set, so a retried read pops the next step
    /// on its next attempt.
    Backpressure,
    /// A write is applied but its outcome is unknown.
    ///
    /// For a write the device is touched exactly once: every item is applied (a refused
    /// item stays unchanged) and every item's outcome is
    /// [`WriteOutcome::Unknown`](crate::WriteOutcome::Unknown) with
    /// [`UnknownReason::TimeoutAfterTransmit`](crate::UnknownReason::TimeoutAfterTransmit).
    /// The call succeeds without waiting for the deadline, the connection stays
    /// `Connected` and nothing is retried: a write is never replayed. For a read the step
    /// behaves exactly like [`Timeout`](Self::Timeout).
    UnknownAfterTransmit,
}

/// A bounded first-in first-out queue of [`FailureStep`]s.
///
/// A [`SimDevice`](crate::sim::SimDevice) owns one; push steps with
/// [`SimDevice::push_failure`](crate::sim::SimDevice::push_failure) and read how many remain with
/// [`SimDevice::pending_failures`](crate::sim::SimDevice::pending_failures). The script is part of
/// the device, so it is shared by every connection to it: steps are consumed in the order
/// the operations reach the device, whichever connection runs them.
///
/// At most [`FailureScript::MAX_STEPS`] steps are held.
///
/// # Compatibility: docs/parity/sim.md#SIM-FAILURE-SCRIPT
/// Stonecrab-owned. The upstream simulated driver has no failure injection.
///
/// # Examples
/// ```
/// use stonecrab::sim::{FailureScript, FailureStep};
///
/// let mut script = FailureScript::default();
/// assert!(script.is_empty());
/// script.push(FailureStep::Timeout)?;
/// script.push_all([FailureStep::Disconnect, FailureStep::Malformed])?;
/// assert_eq!(script.len(), 3);
/// assert_eq!(script.pop(), Some(FailureStep::Timeout));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone)]
pub struct FailureScript {
    steps: VecDeque<FailureStep>,
}

impl Default for FailureScript {
    fn default() -> Self {
        Self::new()
    }
}

impl FailureScript {
    /// Most steps a script holds.
    pub const MAX_STEPS: usize = 1024;

    /// An empty script.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::FailureScript;
    ///
    /// assert!(FailureScript::new().is_empty());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            steps: VecDeque::new(),
        }
    }

    /// Queue `step` behind the others.
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] when the script already holds
    /// [`MAX_STEPS`](Self::MAX_STEPS) steps; the step is not queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureScript, FailureStep};
    ///
    /// let mut script = FailureScript::default();
    /// script.push(FailureStep::Backpressure)?;
    /// assert_eq!(script.len(), 1);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn push(&mut self, step: FailureStep) -> Result<()> {
        if self.steps.len() >= Self::MAX_STEPS {
            return Err(full());
        }
        self.steps.push_back(step);
        Ok(())
    }

    /// Queue every step of `steps` in order, or none of them.
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] when the steps would not fit under
    /// [`MAX_STEPS`](Self::MAX_STEPS); nothing is queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureScript, FailureStep};
    ///
    /// let mut script = FailureScript::default();
    /// script.push_all([FailureStep::Timeout, FailureStep::Timeout, FailureStep::Timeout])?;
    /// assert_eq!(script.len(), 3);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn push_all(&mut self, steps: impl IntoIterator<Item = FailureStep>) -> Result<()> {
        let room = Self::MAX_STEPS.saturating_sub(self.steps.len());
        let incoming: Vec<FailureStep> = steps.into_iter().take(room.saturating_add(1)).collect();
        if incoming.len() > room {
            return Err(full());
        }
        self.steps.extend(incoming);
        Ok(())
    }

    /// Remove and return the oldest step, if any.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureScript, FailureStep};
    ///
    /// let mut script = FailureScript::default();
    /// assert_eq!(script.pop(), None);
    /// script.push(FailureStep::Malformed)?;
    /// assert_eq!(script.pop(), Some(FailureStep::Malformed));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn pop(&mut self) -> Option<FailureStep> {
        self.steps.pop_front()
    }

    /// Drop every queued step.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::{FailureScript, FailureStep};
    ///
    /// let mut script = FailureScript::default();
    /// script.push(FailureStep::Timeout)?;
    /// script.clear();
    /// assert!(script.is_empty());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn clear(&mut self) {
        self.steps.clear();
    }

    /// How many steps are queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::FailureScript;
    ///
    /// assert_eq!(FailureScript::default().len(), 0);
    /// ```
    #[must_use]
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether no step is queued.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sim::FailureScript;
    ///
    /// assert!(FailureScript::default().is_empty());
    /// ```
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

fn full() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "the failure script holds the maximum number of steps",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind<T>(result: Result<T>) -> Option<ErrorKind> {
        result.err().map(|error| error.kind())
    }

    #[test]
    fn a_script_is_a_fifo_queue() {
        let mut script = FailureScript::default();
        assert!(script.is_empty(), "starts empty");
        assert_eq!(script.pop(), None, "nothing to pop");
        script.push(FailureStep::Timeout).expect("room");
        script
            .push_all([FailureStep::Disconnect, FailureStep::Malformed])
            .expect("room");
        assert_eq!(script.len(), 3, "three queued");
        assert_eq!(script.pop(), Some(FailureStep::Timeout), "oldest first");
        assert_eq!(script.pop(), Some(FailureStep::Disconnect), "then the next");
        script.clear();
        assert!(script.is_empty(), "clear drops the rest");
        assert_eq!(script.pop(), None, "and nothing is left to pop");
    }

    #[test]
    fn the_script_is_bounded_and_push_all_is_all_or_nothing() {
        let mut script = FailureScript::default();
        script
            .push_all(vec![
                FailureStep::Backpressure;
                FailureScript::MAX_STEPS - 1
            ])
            .expect("fits exactly");
        assert_eq!(
            kind(script.push_all([FailureStep::Timeout, FailureStep::Timeout])),
            Some(ErrorKind::ResourceLimit),
            "two do not fit in one place"
        );
        assert_eq!(
            script.len(),
            FailureScript::MAX_STEPS - 1,
            "none was queued"
        );
        script.push(FailureStep::Timeout).expect("the last place");
        assert_eq!(
            kind(script.push(FailureStep::Timeout)),
            Some(ErrorKind::ResourceLimit),
            "full"
        );
        assert_eq!(
            kind(script.push_all(std::iter::empty())),
            None,
            "an empty batch always fits"
        );
        assert_eq!(script.len(), FailureScript::MAX_STEPS, "still full");
        assert_eq!(
            kind(script.push_all(std::iter::repeat(FailureStep::Timeout))),
            Some(ErrorKind::ResourceLimit),
            "an endless iterator is cut off at the bound"
        );
    }

    #[test]
    fn steps_compare_and_clone() {
        let id = TagId::new("a").expect("identity");
        let step = FailureStep::PartialFailure(vec![id]);
        assert_eq!(step.clone(), step, "clone equals");
        assert_ne!(step, FailureStep::Malformed, "variants differ");
        let shown = format!("{step:?}");
        assert!(shown.contains("PartialFailure"), "Debug names it: {shown}");
    }
}
