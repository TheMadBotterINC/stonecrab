use std::collections::BTreeMap;
use std::fmt;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant, SystemTime};

use stonecrab_core::{Clock, OpFuture, Result};

/// A hand-advanced [`Clock`] for deterministic tests.
///
/// Time only moves when a test calls [`advance`](Self::advance), [`set`](Self::set) or
/// [`advance_to_next_sleeper`](Self::advance_to_next_sleeper); nothing ever waits on real
/// time. Clones share one timeline, so a `FakeClock` (or an `Arc<FakeClock>`, which
/// coerces to `Arc<dyn Clock>`) can be handed to the code under test while the test keeps
/// a handle to drive it.
///
/// # Sleepers
/// A future from [`Clock::sleep_until`] registers a sleeper, keyed by its deadline, the
/// first time it is polled and still pending. Moving the clock to or past a deadline
/// removes the sleeper and wakes its task exactly once; sleepers are woken in deadline
/// order, and in registration order among equal deadlines. Dropping a sleep future
/// removes its sleeper without waking anything. Wakers are called after the internal lock
/// is released, so a waker may call back into the clock.
///
/// # Wall clock
/// [`Clock::wall_now`] starts at [`SystemTime::UNIX_EPOCH`] and follows the fake elapsed
/// time. [`set_wall_now`](Self::set_wall_now) rebases it without touching the monotonic
/// reading, which models a wall-clock jump.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab_core::Clock;
/// use stonecrab_testkit::{FakeClock, poll_once};
///
/// let clock = FakeClock::default();
/// let deadline = clock.now() + Duration::from_secs(5);
/// let mut sleep = clock.sleep_until(deadline);
/// assert!(poll_once(&mut sleep).is_pending());
/// clock.advance(Duration::from_secs(5));
/// assert!(poll_once(&mut sleep).is_ready());
/// ```
#[derive(Clone)]
pub struct FakeClock {
    shared: Arc<Mutex<State>>,
}

struct State {
    now: Instant,
    wall_anchor: (Instant, SystemTime),
    next_id: u64,
    sleepers: BTreeMap<(Instant, u64), Waker>,
}

impl State {
    /// Move time forward to `target` (never back) and take every due sleeper's waker.
    fn advance_to(&mut self, target: Instant) -> Vec<Waker> {
        if target > self.now {
            self.now = target;
        }
        let mut due = Vec::new();
        while let Some(entry) = self.sleepers.first_entry() {
            if entry.key().0 > self.now {
                break;
            }
            due.push(entry.remove());
        }
        due
    }
}

fn wake_all(due: Vec<Waker>) {
    for waker in due {
        waker.wake();
    }
}

impl FakeClock {
    /// A clock whose monotonic reading starts at `start`, with no sleepers.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab_core::Clock;
    /// use stonecrab_testkit::FakeClock;
    ///
    /// let start = Instant::now();
    /// assert_eq!(FakeClock::new(start).now(), start);
    /// ```
    #[must_use]
    pub fn new(start: Instant) -> Self {
        Self {
            shared: Arc::new(Mutex::new(State {
                now: start,
                wall_anchor: (start, SystemTime::UNIX_EPOCH),
                next_id: 0,
                sleepers: BTreeMap::new(),
            })),
        }
    }

    fn with_state<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.shared.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }

    /// Move the clock forward by `by`, waking every sleeper whose deadline is reached.
    ///
    /// If the sum is not representable as an [`Instant`] the call is ignored and the clock
    /// is unchanged.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab_core::Clock;
    /// use stonecrab_testkit::FakeClock;
    ///
    /// let clock = FakeClock::default();
    /// let before = clock.now();
    /// clock.advance(Duration::from_millis(250));
    /// assert_eq!(clock.now() - before, Duration::from_millis(250));
    /// ```
    pub fn advance(&self, by: Duration) {
        let due = self.with_state(|state| {
            state
                .now
                .checked_add(by)
                .map_or_else(Vec::new, |target| state.advance_to(target))
        });
        wake_all(due);
    }

    /// Move the clock forward to `instant`, waking every sleeper whose deadline is reached.
    ///
    /// The monotonic reading never decreases. Asking for an `instant` before the current
    /// reading is ignored: the clock is unchanged, nothing is woken and `false` is
    /// returned. Otherwise (including `instant` equal to the reading) it returns `true`.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab_core::Clock;
    /// use stonecrab_testkit::FakeClock;
    ///
    /// let clock = FakeClock::default();
    /// let start = clock.now();
    /// assert!(clock.set(start + Duration::from_secs(1)));
    /// assert!(!clock.set(start), "backwards is ignored");
    /// assert_eq!(clock.now(), start + Duration::from_secs(1));
    /// ```
    pub fn set(&self, instant: Instant) -> bool {
        let (moved, due) = self.with_state(|state| {
            if instant < state.now {
                (false, Vec::new())
            } else {
                (true, state.advance_to(instant))
            }
        });
        wake_all(due);
        moved
    }

    /// Move the clock to the earliest pending sleeper's deadline and wake it.
    ///
    /// Returns that deadline, which is also the new reading, or `None` (clock unchanged)
    /// when no sleeper is registered. Sleepers sharing that exact deadline are all due and
    /// all wake, in registration order; later deadlines stay pending.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab_core::Clock;
    /// use stonecrab_testkit::{FakeClock, poll_once};
    ///
    /// let clock = FakeClock::default();
    /// assert_eq!(clock.advance_to_next_sleeper(), None);
    /// let deadline = clock.now() + Duration::from_secs(9);
    /// let mut sleep = clock.sleep_until(deadline);
    /// assert!(poll_once(&mut sleep).is_pending());
    /// assert_eq!(clock.advance_to_next_sleeper(), Some(deadline));
    /// assert_eq!(clock.now(), deadline);
    /// ```
    pub fn advance_to_next_sleeper(&self) -> Option<Instant> {
        let (deadline, due) = self.with_state(|state| {
            let next = state.sleepers.keys().next().map(|key| key.0)?;
            Some((next, state.advance_to(next)))
        })?;
        wake_all(due);
        Some(deadline)
    }

    /// Rebase the wall clock so [`Clock::wall_now`] reads `wall` now.
    ///
    /// The wall reading keeps following the fake elapsed time from there. The monotonic
    /// reading and the sleepers are untouched.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Duration, SystemTime};
    /// use stonecrab_core::Clock;
    /// use stonecrab_testkit::FakeClock;
    ///
    /// let clock = FakeClock::default();
    /// let wall = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    /// clock.set_wall_now(wall);
    /// clock.advance(Duration::from_secs(2));
    /// assert_eq!(clock.wall_now(), wall + Duration::from_secs(2));
    /// ```
    pub fn set_wall_now(&self, wall: SystemTime) {
        self.with_state(|state| state.wall_anchor = (state.now, wall));
    }

    /// Number of sleepers currently registered (polled, pending, not yet woken or dropped).
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::FakeClock;
    ///
    /// assert_eq!(FakeClock::default().sleeper_count(), 0);
    /// ```
    #[must_use]
    pub fn sleeper_count(&self) -> usize {
        self.with_state(|state| state.sleepers.len())
    }
}

impl Default for FakeClock {
    /// A clock starting at the [`Instant`] current when it is constructed.
    fn default() -> Self {
        Self::new(Instant::now())
    }
}

impl fmt::Debug for FakeClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (now, sleepers) = self.with_state(|state| (state.now, state.sleepers.len()));
        f.debug_struct("FakeClock")
            .field("now", &now)
            .field("sleepers", &sleepers)
            .finish()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        self.with_state(|state| state.now)
    }

    fn wall_now(&self) -> SystemTime {
        self.with_state(|state| {
            let (anchor, wall) = state.wall_anchor;
            let elapsed = state.now.saturating_duration_since(anchor);
            wall.checked_add(elapsed).unwrap_or(wall)
        })
    }

    /// Resolves once the fake reading reaches `deadline`; immediately if it already has.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future removes its sleeper and wakes nothing.
    ///
    /// # Wire effects
    /// None.
    fn sleep_until(&self, deadline: Instant) -> OpFuture<'_, ()> {
        Box::pin(Sleep {
            clock: self.clone(),
            deadline,
            id: None,
        })
    }
}

/// The future behind [`FakeClock::sleep_until`].
struct Sleep {
    clock: FakeClock,
    deadline: Instant,
    /// Set once a pending poll has registered a sleeper under `(deadline, id)`.
    id: Option<u64>,
}

impl Future for Sleep {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let (outcome, stale) = this.clock.with_state(|state| {
            if this.deadline <= state.now {
                let stale = this
                    .id
                    .take()
                    .and_then(|id| state.sleepers.remove(&(this.deadline, id)));
                return (Poll::Ready(Ok(())), stale);
            }
            let id = this.id.unwrap_or(state.next_id);
            if this.id.is_none() {
                state.next_id = id.wrapping_add(1);
                this.id = Some(id);
            }
            let stale = state
                .sleepers
                .insert((this.deadline, id), cx.waker().clone());
            (Poll::Pending, stale)
        });
        drop(stale);
        outcome
    }
}

impl Drop for Sleep {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let stale = self
                .clock
                .with_state(|state| state.sleepers.remove(&(self.deadline, id)));
            drop(stale);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CountingWaker, block_on_with, poll_once, poll_once_with};

    const SECOND: Duration = Duration::from_secs(1);

    fn assert_send_sync<T: Send + Sync>() {}

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn sleep_until_at_or_before_now_is_ready_on_first_poll() {
        let clock = FakeClock::default();
        let now = clock.now();
        assert!(
            matches!(poll_once(&mut clock.sleep_until(now)), Poll::Ready(Ok(()))),
            "deadline equal to now"
        );
        clock.advance(secs(10));
        assert!(
            matches!(poll_once(&mut clock.sleep_until(now)), Poll::Ready(Ok(()))),
            "deadline in the past"
        );
        assert_eq!(clock.sleeper_count(), 0, "no sleeper for ready sleeps");
    }

    #[test]
    fn sleep_until_pends_then_resolves_after_advancing_past_the_deadline() {
        let clock = FakeClock::default();
        let counter = CountingWaker::new();
        let waker = counter.waker();
        let mut sleep = clock.sleep_until(clock.now() + secs(5));
        assert!(
            poll_once_with(&mut sleep, &waker).is_pending(),
            "before the deadline"
        );
        assert_eq!(clock.sleeper_count(), 1, "registered");
        clock.advance(secs(4));
        assert_eq!(counter.count(), 0, "not woken early");
        assert!(
            poll_once_with(&mut sleep, &waker).is_pending(),
            "still early"
        );
        assert_eq!(clock.sleeper_count(), 1, "re-poll keeps one sleeper");
        clock.advance(secs(2));
        assert_eq!(counter.count(), 1, "woken once");
        assert_eq!(clock.sleeper_count(), 0, "woken sleeper is removed");
        assert!(
            matches!(poll_once_with(&mut sleep, &waker), Poll::Ready(Ok(()))),
            "ready after the deadline"
        );
        clock.advance(secs(5));
        assert_eq!(counter.count(), 1, "never woken a second time");
    }

    #[test]
    fn sleepers_wake_in_deadline_order_each_exactly_once() {
        let clock = FakeClock::default();
        let start = clock.now();
        let late = CountingWaker::new();
        let early = CountingWaker::new();
        let mut late_sleep = clock.sleep_until(start + secs(8));
        let mut early_sleep = clock.sleep_until(start + secs(3));
        assert!(
            poll_once_with(&mut late_sleep, &late.waker()).is_pending(),
            "late pends"
        );
        assert!(
            poll_once_with(&mut early_sleep, &early.waker()).is_pending(),
            "early pends"
        );
        clock.advance(secs(3));
        assert_eq!(
            (early.count(), late.count()),
            (1, 0),
            "only the early sleeper"
        );
        clock.advance(secs(100));
        assert_eq!(
            (early.count(), late.count()),
            (1, 1),
            "late wakes, early not again"
        );
        assert!(poll_once(&mut early_sleep).is_ready(), "early ready");
        assert!(poll_once(&mut late_sleep).is_ready(), "late ready");
    }

    struct LogWake {
        label: u8,
        log: Arc<Mutex<Vec<u8>>>,
    }

    impl std::task::Wake for LogWake {
        fn wake(self: Arc<Self>) {
            self.log.lock().unwrap().push(self.label);
        }
    }

    #[test]
    fn equal_deadlines_wake_in_registration_order() {
        let clock = FakeClock::default();
        let deadline = clock.now() + secs(2);
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut sleeps = Vec::new();
        for label in 0..3_u8 {
            let log = Arc::clone(&order);
            let waker = Waker::from(Arc::new(LogWake { label, log }));
            let mut sleep = clock.sleep_until(deadline);
            assert!(poll_once_with(&mut sleep, &waker).is_pending(), "pends");
            sleeps.push(sleep);
        }
        assert_eq!(
            clock.advance_to_next_sleeper(),
            Some(deadline),
            "all due together"
        );
        assert_eq!(*order.lock().unwrap(), [0, 1, 2], "registration order");
        assert_eq!(clock.sleeper_count(), 0, "all removed");
    }

    #[test]
    fn advance_to_next_sleeper_moves_to_the_earliest_deadline_and_wakes_only_it() {
        let clock = FakeClock::default();
        let start = clock.now();
        assert_eq!(clock.advance_to_next_sleeper(), None, "no sleepers");
        assert_eq!(clock.now(), start, "unchanged without sleepers");
        let first = CountingWaker::new();
        let second = CountingWaker::new();
        let mut a = clock.sleep_until(start + secs(7));
        let mut b = clock.sleep_until(start + secs(2));
        assert!(
            poll_once_with(&mut a, &first.waker()).is_pending(),
            "a pends"
        );
        assert!(
            poll_once_with(&mut b, &second.waker()).is_pending(),
            "b pends"
        );
        assert_eq!(
            clock.advance_to_next_sleeper(),
            Some(start + secs(2)),
            "earliest"
        );
        assert_eq!(clock.now(), start + secs(2), "time moved to it");
        assert_eq!(
            (first.count(), second.count()),
            (0, 1),
            "only the earliest woke"
        );
        assert_eq!(clock.sleeper_count(), 1, "the other stays registered");
        assert_eq!(
            clock.advance_to_next_sleeper(),
            Some(start + secs(7)),
            "then the next"
        );
        assert_eq!(first.count(), 1, "now the other woke");
        assert_eq!(clock.advance_to_next_sleeper(), None, "drained");
    }

    #[test]
    fn dropping_a_sleep_future_removes_its_sleeper_without_waking() {
        let clock = FakeClock::default();
        let counter = CountingWaker::new();
        let mut sleep = clock.sleep_until(clock.now() + secs(5));
        assert!(
            poll_once_with(&mut sleep, &counter.waker()).is_pending(),
            "pends"
        );
        assert_eq!(clock.sleeper_count(), 1, "registered");
        drop(sleep);
        assert_eq!(clock.sleeper_count(), 0, "removed on drop");
        clock.advance(secs(10));
        assert_eq!(counter.count(), 0, "never woken");
        drop(clock.sleep_until(clock.now() + SECOND));
        assert_eq!(clock.sleeper_count(), 0, "unpolled drop is a no-op");
    }

    #[test]
    fn repolling_with_a_new_waker_replaces_the_old_one() {
        let clock = FakeClock::default();
        let old = CountingWaker::new();
        let new = CountingWaker::new();
        let mut sleep = clock.sleep_until(clock.now() + SECOND);
        assert!(
            poll_once_with(&mut sleep, &old.waker()).is_pending(),
            "first poll"
        );
        assert!(
            poll_once_with(&mut sleep, &new.waker()).is_pending(),
            "second poll"
        );
        assert_eq!(clock.sleeper_count(), 1, "one sleeper per future");
        clock.advance(SECOND);
        assert_eq!((old.count(), new.count()), (0, 1), "latest waker is woken");
    }

    /// A waker that calls back into the clock from both `wake` and `drop`.
    struct ReenterWake {
        clock: FakeClock,
        calls: Arc<Mutex<u32>>,
    }

    impl ReenterWake {
        fn touch(&self) {
            assert!(self.clock.now() >= self.clock.now(), "now is monotonic");
            assert!(
                self.clock.sleeper_count() < 100,
                "sleeper count is readable"
            );
            self.clock.advance(Duration::ZERO);
            let mut calls = self.calls.lock().unwrap();
            *calls = calls.saturating_add(1);
        }
    }

    impl std::task::Wake for ReenterWake {
        fn wake(self: Arc<Self>) {
            self.touch();
        }
    }

    impl Drop for ReenterWake {
        fn drop(&mut self) {
            self.touch();
        }
    }

    fn reentrant_waker(clock: &FakeClock, calls: &Arc<Mutex<u32>>) -> Waker {
        Waker::from(Arc::new(ReenterWake {
            clock: clock.clone(),
            calls: Arc::clone(calls),
        }))
    }

    #[test]
    fn wakers_may_reenter_the_clock_from_wake_and_drop() {
        let clock = FakeClock::default();
        let calls = Arc::new(Mutex::new(0_u32));
        let start = clock.now();

        // advance, set and advance_to_next_sleeper all wake outside the lock.
        let mut sleeps = Vec::new();
        for offset in 1..=3_u64 {
            let mut sleep = clock.sleep_until(start + secs(offset));
            let waker = reentrant_waker(&clock, &calls);
            assert!(poll_once_with(&mut sleep, &waker).is_pending(), "pends");
            sleeps.push(sleep);
        }
        clock.advance(SECOND);
        assert!(clock.set(start + secs(2)), "set forward");
        assert_eq!(
            clock.advance_to_next_sleeper(),
            Some(start + secs(3)),
            "next sleeper"
        );
        assert_eq!(clock.sleeper_count(), 0, "all woken");
        assert!(*calls.lock().unwrap() >= 6, "wake and drop each re-entered");

        // Replacing a stored waker drops the stale one outside the lock.
        let mut sleep = clock.sleep_until(start + secs(10));
        assert!(
            poll_once_with(&mut sleep, &reentrant_waker(&clock, &calls)).is_pending(),
            "first poll"
        );
        assert!(
            poll_once_with(&mut sleep, &reentrant_waker(&clock, &calls)).is_pending(),
            "second poll drops the stale waker"
        );

        // Dropping a registered sleep drops its waker outside the lock.
        drop(sleep);
        assert_eq!(clock.sleeper_count(), 0, "sleeper removed");
    }
    #[test]
    fn set_moves_forward_and_ignores_backwards() {
        let clock = FakeClock::default();
        let start = clock.now();
        let counter = CountingWaker::new();
        let mut sleep = clock.sleep_until(start + secs(5));
        assert!(
            poll_once_with(&mut sleep, &counter.waker()).is_pending(),
            "pends"
        );
        clock.advance(secs(3));
        assert!(!clock.set(start), "backwards is refused");
        assert_eq!(clock.now(), start + secs(3), "clock unchanged");
        assert!(clock.set(start + secs(3)), "equal is accepted as a no-op");
        assert_eq!(counter.count(), 0, "nothing woken yet");
        assert!(clock.set(start + secs(9)), "forward moves");
        assert_eq!(clock.now(), start + secs(9), "forward reading");
        assert_eq!(counter.count(), 1, "set wakes due sleepers");
    }

    #[test]
    fn advance_that_overflows_instant_is_ignored() {
        let clock = FakeClock::default();
        let now = clock.now();
        clock.advance(Duration::MAX);
        assert_eq!(clock.now(), now, "unchanged");
    }

    #[test]
    fn wall_clock_follows_fake_time_and_can_be_rebased() {
        let clock = FakeClock::default();
        assert_eq!(clock.wall_now(), SystemTime::UNIX_EPOCH, "default epoch");
        clock.advance(secs(10));
        assert_eq!(
            clock.wall_now(),
            SystemTime::UNIX_EPOCH + secs(10),
            "follows elapsed"
        );
        let jump = SystemTime::UNIX_EPOCH + secs(5_000);
        let before = clock.now();
        clock.set_wall_now(jump);
        assert_eq!(clock.now(), before, "monotonic reading untouched");
        assert_eq!(clock.wall_now(), jump, "rebased");
        clock.advance(secs(1));
        assert_eq!(clock.wall_now(), jump + SECOND, "keeps following");
    }

    #[test]
    fn clones_share_one_timeline_and_arc_works_as_dyn_clock() {
        let clock = FakeClock::default();
        let shared: Arc<dyn Clock> = Arc::new(clock.clone());
        let start = shared.now();
        let mut sleep = shared.sleep_until(start + secs(4));
        assert!(
            poll_once(&mut sleep).is_pending(),
            "pends through the trait object"
        );
        assert_eq!(clock.sleeper_count(), 1, "visible through the other handle");
        clock.advance(secs(4));
        assert_eq!(shared.now(), start + secs(4), "time shared");
        assert!(
            matches!(poll_once(&mut sleep), Poll::Ready(Ok(()))),
            "ready"
        );
        assert_send_sync::<FakeClock>();
    }

    #[test]
    fn debug_shows_now_and_sleeper_count() {
        let clock = FakeClock::default();
        let mut sleep = clock.sleep_until(clock.now() + SECOND);
        assert!(poll_once(&mut sleep).is_pending(), "pends");
        let text = format!("{clock:?}");
        assert!(text.starts_with("FakeClock { now: "), "{text}");
        assert!(text.contains("sleepers: 1"), "{text}");
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let clock = FakeClock::default();
        let poisoner = clock.clone();
        let result = std::thread::spawn(move || {
            poisoner.with_state(|_| std::panic::panic_any("poison the clock lock"));
        })
        .join();
        assert!(result.is_err(), "the thread panicked");
        clock.advance(SECOND);
        assert_eq!(clock.sleeper_count(), 0, "still usable");
    }

    #[test]
    fn sleeps_chain_under_block_on_with() {
        let clock = FakeClock::default();
        let start = clock.now();
        let outcome = block_on_with(&clock, async {
            for step in 1..=3 {
                clock.sleep_until(start + secs(step)).await?;
            }
            Ok::<_, stonecrab_core::PlcError>(clock.now())
        });
        assert_eq!(
            outcome.unwrap(),
            start + secs(3),
            "three sleeps, no real waiting"
        );
    }
}
