use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll, Wake, Waker};

use crate::testkit::FakeClock;

/// Most polls `block_on` and `block_on_with` make before declaring a future stuck.
const POLL_BUDGET: usize = 10_000;

/// Most consecutive pending polls `block_on_with` makes with no sleeper to advance to.
const IDLE_POLL_BUDGET: usize = 1_000;

#[expect(
    clippy::panic,
    reason = "a stuck future must fail the test loudly instead of hanging it"
)]
fn stuck(who: &str, why: &str) -> ! {
    panic!("{who}: {why}; the future is stuck (nothing it waits on can happen in a test)")
}

/// Drive `future` to completion on the calling thread with a no-op waker.
///
/// The future is polled in a loop, so it must make progress by itself (ready values,
/// futures that finish after a bounded number of polls). It gets at most 10 000 polls; a
/// future still pending after that is a stuck test, and the call panics with a message
/// rather than hanging. Use [`block_on_with`] for futures that wait on a [`FakeClock`].
///
/// # Panics
/// Panics when the future is still pending after 10 000 polls.
///
/// # Examples
/// ```
/// use stonecrab::testkit::block_on;
///
/// assert_eq!(block_on(async { 6 * 7 }), 42);
/// ```
///
/// A future that never completes fails the test instead of hanging it.
/// ```should_panic
/// use stonecrab::testkit::block_on;
///
/// block_on(std::future::pending::<()>());
/// ```
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    for _ in 0..POLL_BUDGET {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
    }
    stuck("block_on", "still pending after 10000 polls")
}

/// Drive `future` to completion, advancing `clock` to the next sleeper whenever it pends.
///
/// Each time the future is pending the clock moves to the earliest registered sleeper
/// (see [`FakeClock::advance_to_next_sleeper`]) and the future is polled again, so a chain
/// of sleeps completes without real waiting. A future that is pending while no sleeper is
/// registered is waiting on something other than time; after 1 000 such consecutive polls
/// the call panics with a message instead of hanging. The total is capped at 10 000 polls.
///
/// # Panics
/// Panics when no sleeper is registered for 1 000 consecutive pending polls, or when the
/// future is still pending after 10 000 polls in total.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::Clock;
/// use stonecrab::testkit::{FakeClock, block_on_with};
///
/// let clock = FakeClock::default();
/// let deadline = clock.now() + Duration::from_secs(30);
/// block_on_with(&clock, clock.sleep_until(deadline))?;
/// assert_eq!(clock.now(), deadline);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// A future waiting on something time cannot deliver fails the test.
/// ```should_panic
/// use stonecrab::CancellationToken;
/// use stonecrab::testkit::{FakeClock, block_on_with};
///
/// let token = CancellationToken::new();
/// let _ = block_on_with(&FakeClock::default(), token.cancelled());
/// ```
pub fn block_on_with<T>(clock: &FakeClock, future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    let mut idle = 0_usize;
    for _ in 0..POLL_BUDGET {
        if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
            return value;
        }
        if clock.advance_to_next_sleeper().is_some() {
            idle = 0;
        } else {
            idle = idle.saturating_add(1);
            if idle >= IDLE_POLL_BUDGET {
                stuck(
                    "block_on_with",
                    "pending for 1000 consecutive polls with no sleeper registered",
                );
            }
        }
    }
    stuck("block_on_with", "still pending after 10000 polls")
}

/// Poll `future` exactly once with a no-op waker.
///
/// The future stays usable afterwards. An `Unpin` future (every `OpFuture`) is passed as
/// `&mut`; pin an `async` block first with `Box::pin`.
///
/// # Examples
/// ```
/// use std::task::Poll;
/// use stonecrab::testkit::poll_once;
///
/// assert_eq!(poll_once(&mut Box::pin(async { 1 })), Poll::Ready(1));
/// assert!(poll_once(&mut Box::pin(std::future::pending::<()>())).is_pending());
/// ```
pub fn poll_once<F>(future: &mut F) -> Poll<F::Output>
where
    F: Future + Unpin + ?Sized,
{
    poll_once_with(future, Waker::noop())
}

/// Poll `future` exactly once with `waker`, for tests that observe wake-ups.
///
/// # Examples
/// ```
/// use std::task::Poll;
/// use stonecrab::testkit::{CountingWaker, poll_once_with};
///
/// let counter = CountingWaker::new();
/// let waker = counter.waker();
/// assert_eq!(poll_once_with(&mut Box::pin(async { "done" }), &waker), Poll::Ready("done"));
/// assert_eq!(counter.count(), 0);
/// ```
pub fn poll_once_with<F>(future: &mut F, waker: &Waker) -> Poll<F::Output>
where
    F: Future + Unpin + ?Sized,
{
    std::pin::Pin::new(future).poll(&mut Context::from_waker(waker))
}

/// A waker that counts how many times it has been woken.
///
/// Clones of the `CountingWaker` and every [`Waker`] from [`waker`](Self::waker) share one
/// counter.
///
/// # Examples
/// ```
/// use stonecrab::testkit::CountingWaker;
///
/// let counter = CountingWaker::new();
/// let waker = counter.waker();
/// waker.wake_by_ref();
/// waker.wake();
/// assert_eq!(counter.count(), 2);
/// ```
#[derive(Debug, Clone, Default)]
pub struct CountingWaker {
    wakes: Arc<WakeCount>,
}

#[derive(Debug, Default)]
struct WakeCount(AtomicUsize);

impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

impl CountingWaker {
    /// A counter at zero.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::CountingWaker;
    ///
    /// assert_eq!(CountingWaker::new().count(), 0);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A [`Waker`] that increments this counter each time it is woken.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::CountingWaker;
    ///
    /// let counter = CountingWaker::new();
    /// counter.waker().wake();
    /// assert_eq!(counter.count(), 1);
    /// ```
    #[must_use]
    pub fn waker(&self) -> Waker {
        Waker::from(Arc::clone(&self.wakes))
    }

    /// Number of wake calls so far.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::CountingWaker;
    ///
    /// assert_eq!(CountingWaker::new().count(), 0);
    /// ```
    #[must_use]
    pub fn count(&self) -> usize {
        self.wakes.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{CancellationToken, Clock, PlcError};

    use super::*;

    #[test]
    fn block_on_returns_ready_and_multi_poll_values() {
        assert_eq!(block_on(async { 7 }), 7, "ready value");
        let mut yields = 3;
        let value = block_on(std::future::poll_fn(|cx| {
            if yields == 0 {
                Poll::Ready("done")
            } else {
                yields -= 1;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }));
        assert_eq!(value, "done", "a future that finishes within the budget");
    }

    #[test]
    #[should_panic(expected = "block_on: still pending after 10000 polls")]
    fn block_on_fails_fast_on_a_never_ready_future() {
        block_on(std::future::pending::<()>());
    }

    #[test]
    fn block_on_with_advances_through_a_chain_of_sleeps() {
        let clock = FakeClock::default();
        let start = clock.now();
        let result = block_on_with(&clock, async {
            clock.sleep_until(start + Duration::from_secs(1)).await?;
            clock.sleep_until(start + Duration::from_secs(60)).await?;
            Ok::<_, PlcError>(7)
        });
        assert_eq!(result.unwrap(), 7, "completes");
        assert_eq!(
            clock.now(),
            start + Duration::from_secs(60),
            "time advanced to last"
        );
    }

    #[test]
    fn block_on_with_tolerates_a_bounded_run_of_idle_polls() {
        let clock = FakeClock::default();
        let mut idle = 10;
        let value = block_on_with(
            &clock,
            std::future::poll_fn(|cx| {
                if idle == 0 {
                    Poll::Ready(1)
                } else {
                    idle -= 1;
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            }),
        );
        assert_eq!(value, 1, "self-waking future completes");
    }

    #[test]
    #[should_panic(expected = "no sleeper registered")]
    fn block_on_with_fails_fast_on_a_future_waiting_for_an_uncancelled_token() {
        let token = CancellationToken::new();
        block_on_with(&FakeClock::default(), token.cancelled()).unwrap();
    }

    #[test]
    #[should_panic(expected = "block_on_with: still pending after 10000 polls")]
    fn block_on_with_caps_total_polls_even_when_sleepers_keep_appearing() {
        let clock = FakeClock::default();
        block_on_with(&clock, async {
            loop {
                let next = clock.now() + Duration::from_secs(1);
                clock.sleep_until(next).await.unwrap();
            }
        });
    }

    #[test]
    fn poll_once_reports_ready_and_pending_and_keeps_the_future_usable() {
        let token = CancellationToken::new();
        let mut wait = token.cancelled();
        assert!(poll_once(&mut wait).is_pending(), "pending while live");
        token.cancel();
        assert!(
            matches!(poll_once(&mut wait), Poll::Ready(Ok(()))),
            "ready after cancel"
        );
    }

    #[test]
    fn counting_waker_counts_wake_and_wake_by_ref_across_clones() {
        let counter = CountingWaker::new();
        let clone = counter.clone();
        let waker = counter.waker();
        assert_eq!(counter.count(), 0, "starts at zero");
        waker.wake_by_ref();
        clone.waker().wake();
        assert_eq!(counter.count(), 2, "every wake counted, shared by clones");
        assert!(format!("{counter:?}").contains("CountingWaker"), "debug");
    }

    #[test]
    fn arc_fake_clock_works_as_arc_dyn_clock_under_block_on_with() {
        let fake = Arc::new(FakeClock::default());
        let shared = Arc::clone(&fake);
        let clock: Arc<dyn Clock> = shared;
        let deadline = clock.now() + Duration::from_secs(3);
        let result = block_on_with(&fake, clock.sleep_until(deadline));
        assert!(result.is_ok(), "sleep resolved");
        assert_eq!(clock.now(), deadline, "through the trait object");
    }
}
