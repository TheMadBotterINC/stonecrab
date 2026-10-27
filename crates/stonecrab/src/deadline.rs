use std::time::{Duration, Instant};

use crate::{ErrorKind, PlcError, Result};

/// Monotonic cutoff; equality with the cutoff means the deadline has elapsed.
///
/// Callers supply the clock reading explicitly so tests and future clock adapters
/// use identical semantics without sleeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
#[must_use]
pub struct Deadline(Instant);

impl Deadline {
    /// Create an absolute cutoff, including one already in the past.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab::Deadline;
    /// let now = Instant::now();
    /// assert!(Deadline::at(now).is_elapsed(now));
    /// ```
    pub const fn at(instant: Instant) -> Self {
        Self(instant)
    }

    /// Compute a cutoff relative to an explicit monotonic clock reading.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Duration, Instant};
    /// use stonecrab::Deadline;
    /// let now = Instant::now();
    /// let deadline = Deadline::after(now, Duration::from_secs(1))?;
    /// assert_eq!(deadline.remaining(now), Some(Duration::from_secs(1)));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] if the duration exceeds the
    /// platform's representable monotonic clock range.
    pub fn after(now: Instant, duration: Duration) -> Result<Self> {
        now.checked_add(duration).map(Self).ok_or_else(|| {
            PlcError::new(
                ErrorKind::InvalidConfig,
                "deadline exceeds monotonic clock range",
            )
        })
    }

    /// The absolute monotonic cutoff.
    pub const fn instant(self) -> Instant {
        self.0
    }

    /// Positive time remaining, or `None` at and after the cutoff.
    pub fn remaining(self, now: Instant) -> Option<Duration> {
        self.0
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
    }

    /// Whether the cutoff is at or before the supplied monotonic reading.
    pub fn is_elapsed(self, now: Instant) -> bool {
        now >= self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: SPEC mandatory-failure-cases / deadline boundaries
    #[test]
    fn deadlines_cover_before_exactly_at_and_after_cutoff() {
        let start = Instant::now();
        let duration = Duration::from_secs(5);
        let cutoff = start
            .checked_add(duration)
            .expect("five seconds representable");
        let deadline = Deadline::after(start, duration).expect("valid deadline");
        assert_eq!(
            deadline,
            Deadline::at(cutoff),
            "absolute and relative construction agree"
        );
        assert_eq!(deadline.instant(), cutoff, "cutoff retained exactly");
        assert_eq!(
            deadline.remaining(start),
            Some(duration),
            "time remaining before cutoff"
        );
        assert!(!deadline.is_elapsed(start), "not expired before cutoff");
        assert_eq!(deadline.remaining(cutoff), None, "no time left at cutoff");
        assert!(deadline.is_elapsed(cutoff), "expired at cutoff");
        let later = cutoff
            .checked_add(Duration::from_nanos(1))
            .expect("one nanosecond representable");
        assert_eq!(deadline.remaining(later), None, "no time left after cutoff");
        assert!(deadline.is_elapsed(later), "expired after cutoff");
        assert!(
            Deadline::after(start, Duration::ZERO)
                .expect("zero duration")
                .is_elapsed(start),
            "zero duration expires immediately"
        );
    }

    #[test]
    fn deadline_overflow_is_a_typed_error_not_a_panic() {
        let error = Deadline::after(Instant::now(), Duration::MAX)
            .expect_err("duration cannot fit monotonic clock");
        assert_eq!(
            error.kind(),
            ErrorKind::InvalidConfig,
            "overflow is a configuration failure"
        );
    }
}
