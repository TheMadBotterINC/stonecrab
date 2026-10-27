use std::time::{Instant, SystemTime};

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
}
