use std::time::{Instant, SystemTime};

use crate::{Clock, OpFuture};

/// The `std`-only clock of the simulator: real time that never waits.
///
/// `now()` is [`Instant::now`] and `wall_now()` is [`SystemTime::now`], so a
/// [`Deadline`](crate::Deadline) an application builds from `Instant::now()`
/// agrees with the connection's clock (D9). `sleep_until` resolves on its first poll
/// without moving time: the simulator is not a timer. In real-time use a scripted
/// `Delay` step is therefore a no-op, and a scripted `Timeout` step returns
/// `ErrorKind::Timeout` directly instead of waiting for a deadline to pass. Tests that
/// need time to pass inject the testkit's `FakeClock` instead, and derive their deadlines
/// from it (ADR-0007, ADR-0012): deadlines are always compared against the connection's
/// own clock.
///
/// The type lives here rather than in core because core ships no `Clock` implementation
/// (ADR-0007, decision 3), and here rather than in the transport crate because it needs
/// neither Tokio nor a timer.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab::Clock;
/// use stonecrab::sim::RealtimeClock;
///
/// let clock: Arc<dyn Clock> = Arc::new(RealtimeClock::new());
/// let before = clock.now();
/// assert!(clock.now() >= before);
/// assert!(clock.wall_now() > std::time::SystemTime::UNIX_EPOCH);
/// ```
///
/// # Compatibility: docs/parity/sim.md#SIM-REALTIME-CLOCK
/// Stonecrab-owned; PLC4X has no equivalent, and the row records that no parity is claimed.
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct RealtimeClock;

impl RealtimeClock {
    /// The real-time clock. Equivalent to [`RealtimeClock::default`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Clock;
    /// use stonecrab::sim::RealtimeClock;
    ///
    /// let clock = RealtimeClock::new();
    /// assert!(clock.now() <= clock.now());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Clock for RealtimeClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }

    /// Resolves with `Ok(())` on the first poll, whatever `deadline` is. Time is not
    /// moved and nothing waits.
    ///
    /// # Cancel safety
    /// Cancel-safe. The future holds no state; dropping it, before or after a poll, has no
    /// effect. The deadline source is the `deadline` argument, which is ignored.
    ///
    /// # Wire effects
    /// None. Sleeping touches no transport.
    fn sleep_until(&self, _deadline: Instant) -> OpFuture<'_, ()> {
        Box::pin(std::future::ready(Ok(())))
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests {
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::Duration;

    use crate::testkit::poll_once;

    use super::*;

    // covers: ADR-0007 decision 3 (RealtimeClock is a std-only Clock implementation)
    #[test]
    fn it_is_object_safe_and_shareable_as_arc_dyn_clock() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<RealtimeClock>();
        let clock: Arc<dyn Clock> = Arc::new(RealtimeClock::new());
        let shared = Arc::clone(&clock);
        let before = Instant::now();
        let reading = shared.now();
        assert!(
            reading >= before,
            "now() is not behind a reading taken earlier"
        );
        assert!(clock.now() >= reading, "now() never moves backwards");
    }

    #[test]
    fn now_is_the_monotonic_clock_and_wall_now_the_system_clock() {
        let clock = RealtimeClock::default();
        let (before, wall_before) = (Instant::now(), SystemTime::now());
        let (reading, wall) = (clock.now(), clock.wall_now());
        let (after, wall_after) = (Instant::now(), SystemTime::now());
        assert!(
            before <= reading && reading <= after,
            "now() lies between two Instant readings"
        );
        // The wall clock may be stepped by the host, so only a sanity bound is asserted.
        assert!(wall > SystemTime::UNIX_EPOCH, "wall_now() is after 1970");
        assert!(
            wall_before <= wall_after,
            "the system clock did not step backwards during the test"
        );
    }

    #[test]
    fn sleep_until_is_ready_on_the_first_poll_for_any_deadline() {
        let clock = RealtimeClock::new();
        let now = clock.now();
        let far = now
            .checked_add(Duration::from_secs(3_600))
            .expect("an hour ahead is representable");
        for deadline in [now, far] {
            let mut sleep = clock.sleep_until(deadline);
            assert!(
                matches!(poll_once(&mut sleep), Poll::Ready(Ok(()))),
                "sleep_until resolves immediately"
            );
        }
        assert!(
            clock.now() < far,
            "sleeping did not move time to the deadline"
        );
    }

    #[test]
    fn a_dropped_sleep_has_no_effect() {
        let clock = RealtimeClock::new();
        drop(clock.sleep_until(clock.now()));
        let mut again = clock.sleep_until(clock.now());
        assert!(
            matches!(poll_once(&mut again), Poll::Ready(Ok(()))),
            "still ready"
        );
    }
}
