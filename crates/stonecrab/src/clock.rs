use std::time::{Duration, Instant, SystemTime};

use crate::OpFuture;

/// Source of monotonic time, wall-clock time and sleeping, injected into connections.
///
/// Deadlines and freshness are evaluated against [`Clock::now`], which is monotonic and
/// never moves backwards. [`Clock::wall_now`] is only for receipt timestamps shown to
/// people; nothing may compute a timeout from it. [`Clock::sleep_until`] is the single
/// place a connection waits for time to pass (reconnect backoff, request deadlines), so
/// a test clock can advance time without waiting.
///
/// # No implementation in core
/// The crate root ships no `Clock` implementation. It has no timer and may not sleep
/// (`clippy.toml` disallows `std::thread::sleep`). The implementations live where their
/// dependencies are allowed (ADR-0007):
///
/// - `SystemClock` in `stonecrab::transport`, backed by the Tokio timer.
/// - `FakeClock` in the testkit, advanced by hand for deterministic tests.
/// - `RealtimeClock` in the simulator driver, a `std`-only clock whose `sleep_until`
///   resolves immediately.
///
/// A caller who injects a clock derives its [`Deadline`](crate::Deadline)s from that
/// same clock; mixing readings from two clocks is a logic error.
///
/// # Compatibility: docs/parity/core.md#CORE-CLOCK
/// Upstream reads the system clock directly; the injectable trait is a Stonecrab addition.
///
/// # Examples
/// A clock implementation returns a boxed future from `sleep_until`. This one never
/// waits, which is enough to show the shape and that the trait is object safe.
/// ```
/// use std::task::{Context, Poll, Waker};
/// use std::time::{Instant, SystemTime};
/// use stonecrab::{Clock, OpFuture};
///
/// struct FrozenClock {
///     start: Instant,
/// }
///
/// impl Clock for FrozenClock {
///     fn now(&self) -> Instant {
///         self.start
///     }
///     fn wall_now(&self) -> SystemTime {
///         SystemTime::UNIX_EPOCH
///     }
///     fn sleep_until(&self, _deadline: Instant) -> OpFuture<'_, ()> {
///         Box::pin(std::future::ready(Ok(())))
///     }
/// }
///
/// let clock = FrozenClock { start: Instant::now() };
/// let dynamic: &dyn Clock = &clock;
/// let mut sleep = dynamic.sleep_until(dynamic.now());
/// let mut cx = Context::from_waker(Waker::noop());
/// assert!(matches!(sleep.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
/// assert_eq!(dynamic.wall_now(), SystemTime::UNIX_EPOCH);
/// ```
pub trait Clock: Send + Sync {
    /// Current monotonic reading. Never decreases; unrelated to the wall clock.
    fn now(&self) -> Instant;

    /// Current wall-clock time, for receipt timestamps only.
    ///
    /// The wall clock can jump (NTP, manual changes), so it must not feed a deadline.
    fn wall_now(&self) -> SystemTime;

    /// A future that resolves with `Ok(())` once [`Clock::now`] reaches `deadline`.
    ///
    /// A `deadline` at or before `now()` resolves on the first poll. The future never
    /// resolves with an error from a correct implementation; the `Result` is part of the
    /// uniform [`OpFuture`] shape.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future cancels the sleep and has no side effects: no
    /// timer entry outlives it and nothing was sent to a device.
    ///
    /// # Wire effects
    /// None. Sleeping touches no transport.
    fn sleep_until(&self, deadline: Instant) -> OpFuture<'_, ()>;
}

/// How many ticks of `interval` lie after `from`, up to and including `now`, and how far
/// `now` is past the last of them (past `from` when there is none).
///
/// The grid arithmetic of the polling subscriptions (ADR-0020, addendum S55) and the event
/// pump (ADR-0031): ticks lie on a grid of `interval` from `from`, and a zero interval has
/// none.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn ticks(from: Instant, now: Instant, interval: Duration) -> (u64, Duration) {
    let elapsed = now.saturating_duration_since(from).as_nanos();
    let step = interval.as_nanos();
    let whole = elapsed.checked_div(step).unwrap_or(0);
    let rest = elapsed.checked_rem(step).unwrap_or(0);
    (
        u64::try_from(whole).unwrap_or(u64::MAX),
        Duration::from_nanos(u64::try_from(rest).unwrap_or(u64::MAX)),
    )
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    use super::*;

    struct StepClock {
        start: Instant,
    }

    impl Clock for StepClock {
        fn now(&self) -> Instant {
            self.start
        }
        fn wall_now(&self) -> SystemTime {
            SystemTime::UNIX_EPOCH
        }
        fn sleep_until(&self, _deadline: Instant) -> OpFuture<'_, ()> {
            Box::pin(std::future::ready(Ok(())))
        }
    }

    #[test]
    fn clock_is_object_safe_shareable_and_send() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<dyn Clock>();
        let start = Instant::now();
        let clock: Arc<dyn Clock> = Arc::new(StepClock { start });
        let shared = Arc::clone(&clock);
        assert_eq!(shared.now(), start, "monotonic reading through dyn");
        assert_eq!(
            clock.wall_now(),
            SystemTime::UNIX_EPOCH,
            "wall reading through dyn"
        );
        let mut sleep = clock.sleep_until(start);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(
            matches!(sleep.as_mut().poll(&mut cx), Poll::Ready(Ok(()))),
            "an elapsed deadline resolves on the first poll"
        );
    }

    #[test]
    fn ticks_count_the_grid_points_after_the_start_up_to_now() {
        const TICK: Duration = Duration::from_millis(250);
        let start = Instant::now();
        assert_eq!(ticks(start, start, TICK), (0, Duration::ZERO), "none yet");
        assert_eq!(
            ticks(start, start + TICK * 5 / 2, TICK),
            (2, TICK / 2),
            "two and a half"
        );
        assert_eq!(
            ticks(start, start + TICK, TICK),
            (1, Duration::ZERO),
            "the end counts"
        );
        assert_eq!(
            ticks(start + TICK, start, TICK),
            (0, Duration::ZERO),
            "a now before the start"
        );
        assert_eq!(
            ticks(start, start + TICK, Duration::ZERO),
            (0, Duration::ZERO),
            "a zero interval has no ticks"
        );
    }
}
