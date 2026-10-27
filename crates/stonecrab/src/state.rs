use std::fmt;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};

use crate::future::{WakerSlots, wake_all};
use crate::{ConnectionState, OpFuture, Result};

struct Latest {
    state: ConnectionState,
    version: u64,
    waiters: WakerSlots,
}

struct Shared {
    latest: Mutex<Latest>,
}

impl Shared {
    /// Lock the latest value. A poisoned lock is recovered: the critical sections only
    /// store a value, bump a counter or move wakers, so a panic elsewhere leaves nothing
    /// half-written.
    fn lock(&self) -> MutexGuard<'_, Latest> {
        self.latest.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Owner side of an observable [`ConnectionState`].
///
/// The component that owns a connection keeps the cell and calls [`set`](Self::set) on
/// every lifecycle transition. Observers hold [`StateWatch`] handles made by
/// [`StateCell::new`] or [`watch`](Self::watch). Each `set` bumps a version counter and
/// wakes every parked watcher.
///
/// # Same-state writes
/// `set` always bumps the version, even when the new state equals the old one. The owner
/// is the single writer and decides what counts as a transition; for example a second
/// `Reconnecting` with a new `next_at` is worth reporting. Core does not compare states.
///
/// # Dropping the cell
/// Dropping the cell does not wake or fail its watchers; a pending
/// [`StateWatch::changed`] simply never resolves. Terminal states (`Closed`, `Failed`)
/// are the signal that no further change will come; observers that must not wait forever
/// combine `changed` with a deadline or a `CancellationToken`.
///
/// # Examples
/// ```
/// use stonecrab::{ConnectionState, StateCell};
///
/// let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
/// cell.set(ConnectionState::Connecting);
/// assert_eq!(cell.current(), ConnectionState::Connecting);
/// assert_eq!(watch.current(), ConnectionState::Connecting);
/// ```
pub struct StateCell {
    shared: Arc<Shared>,
}

impl StateCell {
    /// Create a cell holding `initial` and the first watcher.
    ///
    /// The returned watch treats `initial` as already seen: its
    /// [`changed`](StateWatch::changed) stays pending until the first [`set`](Self::set).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionState, StateCell};
    /// let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
    /// assert_eq!(cell.current(), watch.current());
    /// ```
    pub fn new(initial: ConnectionState) -> (Self, StateWatch) {
        let cell = Self {
            shared: Arc::new(Shared {
                latest: Mutex::new(Latest {
                    state: initial,
                    version: 0,
                    waiters: WakerSlots::default(),
                }),
            }),
        };
        let watch = cell.watch();
        (cell, watch)
    }

    /// Replace the state, bump the version and wake every parked watcher once.
    ///
    /// Wakers are called after the internal lock is released. If a waker panics, the
    /// remaining wakers are still woken and the first panic is re-raised afterwards.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionState, StateCell};
    /// let (cell, _watch) = StateCell::new(ConnectionState::Disconnected);
    /// cell.set(ConnectionState::Connected);
    /// assert_eq!(cell.current(), ConnectionState::Connected);
    /// ```
    pub fn set(&self, state: ConnectionState) {
        let wakers = {
            let mut latest = self.shared.lock();
            latest.state = state;
            latest.version = latest.version.wrapping_add(1);
            latest.waiters.drain()
        };
        wake_all(wakers);
    }

    /// The latest state.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionState, StateCell};
    /// let (cell, _watch) = StateCell::new(ConnectionState::Closed);
    /// assert_eq!(cell.current(), ConnectionState::Closed);
    /// ```
    pub fn current(&self) -> ConnectionState {
        self.shared.lock().state
    }

    /// Make another watcher that treats the current state as already seen.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionState, StateCell};
    /// let (cell, _first) = StateCell::new(ConnectionState::Disconnected);
    /// cell.set(ConnectionState::Connected);
    /// let late = cell.watch();
    /// assert_eq!(late.current(), ConnectionState::Connected);
    /// ```
    pub fn watch(&self) -> StateWatch {
        let version = self.shared.lock().version;
        StateWatch {
            shared: Arc::clone(&self.shared),
            seen: AtomicU64::new(version),
        }
    }
}

impl fmt::Debug for StateCell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let latest = self.shared.lock();
        f.debug_struct("StateCell")
            .field("state", &latest.state)
            .field("version", &latest.version)
            .finish()
    }
}

/// Observer of a [`StateCell`], with latest-value semantics.
///
/// A watch is not a queue. It remembers the version it last reported, and
/// [`changed`](Self::changed) resolves once per version change with the state at the
/// time it is polled. If the owner calls `set` several times before the watcher polls,
/// the intermediate states are overwritten and **coalesced**: the watcher sees one change
/// carrying only the latest state. A watcher that must see every transition cannot be
/// built on this type; the state machine records transitions in tracing events instead.
///
/// Clones are independent observers: a clone starts with the same "last seen" version as
/// its source and then tracks its own. All handles are `Send + Sync`.
///
/// # Compatibility: docs/parity/core.md#CORE-STATE-WATCH
/// Upstream has no state-watch API; the latest-value watch is a Stonecrab addition.
///
/// # Examples
/// ```
/// use std::task::{Context, Poll, Waker};
/// use stonecrab::{ConnectionState, StateCell};
///
/// let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
/// let mut cx = Context::from_waker(Waker::noop());
/// let mut change = watch.changed();
/// assert!(change.as_mut().poll(&mut cx).is_pending());
///
/// cell.set(ConnectionState::Connecting);
/// cell.set(ConnectionState::Connected); // coalesces with the line above
/// assert!(matches!(
///     change.as_mut().poll(&mut cx),
///     Poll::Ready(Ok(ConnectionState::Connected))
/// ));
/// ```
pub struct StateWatch {
    shared: Arc<Shared>,
    // Last version reported to this watcher. The mutex guards the state itself; this
    // counter only decides whether a change is news, so Relaxed ordering suffices.
    seen: AtomicU64,
}

impl StateWatch {
    /// The latest state, without consuming a pending change.
    ///
    /// A later [`changed`](Self::changed) still resolves for a version this watcher has
    /// not yet been told about.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionState, StateCell};
    /// let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
    /// cell.set(ConnectionState::Connected);
    /// assert_eq!(watch.current(), ConnectionState::Connected);
    /// ```
    pub fn current(&self) -> ConnectionState {
        self.shared.lock().state
    }

    /// A future that resolves with the latest state once the version differs from the
    /// one this watcher last reported.
    ///
    /// It is `Pending` until the owner calls [`StateCell::set`], then `Ready` exactly
    /// once per version: polling again without a further `set` is `Pending`. Several
    /// `set` calls before the poll resolve once with the latest value (coalescing, see
    /// the type documentation). Each poll re-registers the task's waker, replacing the
    /// previous one (`Waker::will_wake` deduplication). The future never resolves with an
    /// error.
    ///
    /// # Cancel safety
    /// Cancel-safe. The reported version is recorded only when the future returns
    /// `Ready`, so dropping a future after a wake-up but before it was polled loses no
    /// change; the next `changed` call reports it.
    ///
    /// # Wire effects
    /// None. Observing state touches no transport.
    ///
    /// # Examples
    /// ```
    /// use std::task::{Context, Poll, Waker};
    /// use stonecrab::{ConnectionState, StateCell};
    ///
    /// let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
    /// let mut cx = Context::from_waker(Waker::noop());
    /// let mut change = watch.changed();
    /// assert!(change.as_mut().poll(&mut cx).is_pending());
    /// cell.set(ConnectionState::Connected);
    /// assert!(matches!(
    ///     change.as_mut().poll(&mut cx),
    ///     Poll::Ready(Ok(ConnectionState::Connected))
    /// ));
    /// ```
    pub fn changed(&self) -> OpFuture<'_, ConnectionState> {
        Box::pin(Changed {
            watch: self,
            slot: None,
        })
    }
}

impl Clone for StateWatch {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            seen: AtomicU64::new(self.seen.load(Ordering::Relaxed)),
        }
    }
}

impl fmt::Debug for StateWatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let latest = self.shared.lock();
        f.debug_struct("StateWatch")
            .field("state", &latest.state)
            .field("version", &latest.version)
            .field("seen", &self.seen.load(Ordering::Relaxed))
            .finish()
    }
}

/// Future behind [`StateWatch::changed`]; `slot` is its parked-waker id.
struct Changed<'a> {
    watch: &'a StateWatch,
    slot: Option<u64>,
}

impl Future for Changed<'_> {
    type Output = Result<ConnectionState>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let displaced = {
            let mut latest = this.watch.shared.lock();
            if latest.version != this.watch.seen.load(Ordering::Relaxed) {
                this.watch.seen.store(latest.version, Ordering::Relaxed);
                return Poll::Ready(Ok(latest.state));
            }
            latest.waiters.register(&mut this.slot, cx.waker())
        };
        // Dropped after the lock: a waker's Drop may call back into the cell.
        drop(displaced);
        Poll::Pending
    }
}

impl Drop for Changed<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            let removed = self.watch.shared.lock().waiters.release(slot);
            // Dropped after the lock: a waker's Drop may call back into the cell.
            drop(removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::task::{Wake, Waker};
    use std::time::Instant;

    use crate::ErrorKind;

    use super::*;

    struct Counter(AtomicU64);

    impl Counter {
        fn wakes(&self) -> u64 {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counting() -> (Arc<Counter>, Waker) {
        let counter = Arc::new(Counter(AtomicU64::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        (counter, waker)
    }

    fn poll_with(
        future: &mut OpFuture<'_, ConnectionState>,
        waker: &Waker,
    ) -> Poll<Result<ConnectionState>> {
        future.as_mut().poll(&mut Context::from_waker(waker))
    }

    fn poll_state(
        future: &mut OpFuture<'_, ConnectionState>,
        waker: &Waker,
    ) -> Poll<Option<ConnectionState>> {
        poll_with(future, waker).map(Result::ok)
    }

    fn waiters(cell: &StateCell) -> usize {
        cell.shared.lock().waiters.len()
    }

    /// A waker whose `Drop` calls back into the cell and a watch, as a task owning them would.
    struct Reentrant {
        cell: StateWatch,
        drops: Arc<AtomicU64>,
    }

    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            std::hint::black_box(self.cell.current());
        }
    }

    impl Drop for Reentrant {
        fn drop(&mut self) {
            std::hint::black_box(self.cell.current());
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn reentrant(watch: &StateWatch) -> (Arc<AtomicU64>, Waker) {
        let drops = Arc::new(AtomicU64::new(0));
        let waker = Waker::from(Arc::new(Reentrant {
            cell: watch.clone(),
            drops: Arc::clone(&drops),
        }));
        (drops, waker)
    }

    #[test]
    fn a_waker_whose_drop_touches_the_cell_cannot_deadlock_on_replace_or_release() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let (replaced_drops, replaced) = reentrant(&watch);
        let (_, replacement) = counting();
        let mut change = watch.changed();
        assert!(poll_with(&mut change, &replaced).is_pending(), "pends");
        replaced.wake_by_ref();
        drop(replaced);
        assert!(
            poll_with(&mut change, &replacement).is_pending(),
            "repoll pends"
        );
        assert_eq!(
            replaced_drops.load(Ordering::SeqCst),
            1,
            "displaced waker dropped"
        );
        drop(change);

        let (released_drops, released) = reentrant(&watch);
        let mut other = watch.changed();
        assert!(poll_with(&mut other, &released).is_pending(), "pends");
        released.wake_by_ref();
        drop(released);
        drop(other);
        assert_eq!(
            released_drops.load(Ordering::SeqCst),
            1,
            "released waker dropped"
        );
        assert_eq!(waiters(&cell), 0, "nothing left parked");
    }

    #[test]
    fn current_reflects_set_on_the_cell_and_every_watch() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let other = cell.watch();
        assert_eq!(
            cell.current(),
            ConnectionState::Disconnected,
            "initial cell"
        );
        assert_eq!(
            watch.current(),
            ConnectionState::Disconnected,
            "initial watch"
        );
        cell.set(ConnectionState::Connecting);
        assert_eq!(
            cell.current(),
            ConnectionState::Connecting,
            "cell after set"
        );
        assert_eq!(
            watch.current(),
            ConnectionState::Connecting,
            "watch after set"
        );
        assert_eq!(other.current(), ConnectionState::Connecting, "second watch");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn changed_is_pending_until_set_then_ready_once_per_version() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let (counter, waker) = counting();
        let mut change = watch.changed();
        assert!(poll_with(&mut change, &waker).is_pending(), "no set yet");
        assert_eq!(counter.wakes(), 0, "nothing woke the watcher");
        cell.set(ConnectionState::Connected);
        assert_eq!(counter.wakes(), 1, "set woke the parked watcher once");
        assert_eq!(
            poll_state(&mut change, &waker),
            Poll::Ready(Some(ConnectionState::Connected)),
            "ready with the new value"
        );
        let mut next = watch.changed();
        assert!(
            poll_with(&mut next, &waker).is_pending(),
            "version already reported"
        );
        cell.set(ConnectionState::Closing);
        assert_eq!(counter.wakes(), 2, "next set wakes again");
        assert_eq!(
            poll_state(&mut next, &waker),
            Poll::Ready(Some(ConnectionState::Closing)),
            "second version reported"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn two_sets_before_a_poll_coalesce_into_one_change_with_the_latest_value() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        cell.set(ConnectionState::Connecting);
        cell.set(ConnectionState::Failed(ErrorKind::Timeout));
        let mut change = watch.changed();
        assert_eq!(
            poll_state(&mut change, Waker::noop()),
            Poll::Ready(Some(ConnectionState::Failed(ErrorKind::Timeout))),
            "latest value wins; Connecting was overwritten"
        );
        let mut next = watch.changed();
        assert!(
            poll_with(&mut next, Waker::noop()).is_pending(),
            "the coalesced changes count as one"
        );
    }

    #[test]
    fn setting_the_same_state_still_bumps_the_version() {
        let (cell, watch) = StateCell::new(ConnectionState::Connected);
        cell.set(ConnectionState::Connected);
        let mut change = watch.changed();
        assert_eq!(
            poll_state(&mut change, Waker::noop()),
            Poll::Ready(Some(ConnectionState::Connected)),
            "a same-state set is still a reported change"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn every_watcher_wakes_and_each_sees_the_change_independently() {
        let (cell, first) = StateCell::new(ConnectionState::Disconnected);
        let second = first.clone();
        let third = cell.watch();
        let (c1, w1) = counting();
        let (c2, w2) = counting();
        let (c3, w3) = counting();
        let (mut f1, mut f2, mut f3) = (first.changed(), second.changed(), third.changed());
        assert!(poll_with(&mut f1, &w1).is_pending(), "first pends");
        assert!(poll_with(&mut f2, &w2).is_pending(), "second pends");
        assert!(poll_with(&mut f3, &w3).is_pending(), "third pends");
        assert_eq!(waiters(&cell), 3, "one parked waker per watcher");
        cell.set(ConnectionState::Connected);
        assert_eq!(
            (c1.wakes(), c2.wakes(), c3.wakes()),
            (1, 1, 1),
            "all woken once"
        );
        assert_eq!(waiters(&cell), 0, "wakers drained by set");
        for future in [&mut f1, &mut f2, &mut f3] {
            assert_eq!(
                poll_state(future, Waker::noop()),
                Poll::Ready(Some(ConnectionState::Connected)),
                "each watcher reports the version"
            );
        }
    }

    #[test]
    fn a_clone_starts_from_the_source_seen_version_and_then_tracks_its_own() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        cell.set(ConnectionState::Connecting);
        let clone = watch.clone();
        let mut from_original = watch.changed();
        assert_eq!(
            poll_state(&mut from_original, Waker::noop()),
            Poll::Ready(Some(ConnectionState::Connecting)),
            "original consumes the change"
        );
        let mut from_clone = clone.changed();
        assert_eq!(
            poll_state(&mut from_clone, Waker::noop()),
            Poll::Ready(Some(ConnectionState::Connecting)),
            "clone made before the poll still has the change pending"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn repolling_replaces_the_waker_and_the_same_waker_registers_one_entry() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let (first, first_waker) = counting();
        let (second, second_waker) = counting();
        let mut change = watch.changed();
        assert!(poll_with(&mut change, &first_waker).is_pending(), "pends");
        assert!(
            poll_with(&mut change, &first_waker).is_pending(),
            "repoll pends"
        );
        assert!(
            poll_with(&mut change, &first_waker.clone()).is_pending(),
            "clone pends"
        );
        assert_eq!(waiters(&cell), 1, "will_wake dedup keeps one entry");
        assert!(
            poll_with(&mut change, &second_waker).is_pending(),
            "new waker pends"
        );
        assert_eq!(waiters(&cell), 1, "new waker replaced the old one");
        cell.set(ConnectionState::Connected);
        assert_eq!(first.wakes(), 0, "replaced waker not woken");
        assert_eq!(second.wakes(), 1, "current waker woken once");
    }

    #[test]
    fn a_dropped_future_loses_no_change_and_releases_its_waker() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let (counter, waker) = counting();
        let mut abandoned = watch.changed();
        assert!(poll_with(&mut abandoned, &waker).is_pending(), "pends");
        assert_eq!(waiters(&cell), 1, "parked");
        drop(abandoned);
        assert_eq!(waiters(&cell), 0, "slot released on drop");
        cell.set(ConnectionState::Connected);
        assert_eq!(counter.wakes(), 0, "dropped future is never woken");
        let mut again = watch.changed();
        assert_eq!(
            poll_state(&mut again, Waker::noop()),
            Poll::Ready(Some(ConnectionState::Connected)),
            "the change is still reported to a new future"
        );
        drop(watch.changed());
    }

    #[test]
    fn a_waker_woken_by_set_may_call_back_into_the_cell_without_deadlock() {
        struct Reentrant(Arc<Shared>);
        impl Wake for Reentrant {
            fn wake(self: Arc<Self>) {
                let seen = self.0.lock().state;
                assert_eq!(
                    seen,
                    ConnectionState::Connected,
                    "new state visible in wake"
                );
            }
        }
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let waker = Waker::from(Arc::new(Reentrant(Arc::clone(&cell.shared))));
        let mut change = watch.changed();
        assert!(poll_with(&mut change, &waker).is_pending(), "pends");
        cell.set(ConnectionState::Connected);
        assert!(
            poll_with(&mut change, &waker).is_ready(),
            "reported after the wake"
        );
    }

    #[test]
    fn debug_shows_state_and_versions_but_no_wakers() {
        let (cell, watch) = StateCell::new(ConnectionState::Closed);
        let (_counter, waker) = counting();
        let mut change = watch.changed();
        assert!(poll_with(&mut change, &waker).is_pending(), "pends");
        assert_eq!(
            format!("{cell:?}"),
            "StateCell { state: Closed, version: 0 }",
            "cell debug shape"
        );
        assert_eq!(
            format!("{watch:?}"),
            "StateWatch { state: Closed, version: 0, seen: 0 }",
            "watch debug shape"
        );
        cell.set(ConnectionState::Failed(ErrorKind::Disconnected));
        assert!(poll_with(&mut change, &waker).is_ready(), "change reported");
        assert_eq!(
            format!("{watch:?}"),
            "StateWatch { state: Failed(Disconnected), version: 1, seen: 1 }",
            "watch debug after a reported change"
        );
    }

    #[test]
    fn reconnecting_state_round_trips_with_its_deadline_data() {
        let next_at = Instant::now();
        let (cell, watch) = StateCell::new(ConnectionState::Connected);
        cell.set(ConnectionState::Reconnecting {
            attempt: 2,
            next_at,
        });
        assert_eq!(
            watch.current(),
            ConnectionState::Reconnecting {
                attempt: 2,
                next_at
            },
            "payload survives"
        );
    }

    struct Panicker;

    impl Wake for Panicker {
        fn wake(self: Arc<Self>) {
            std::panic::panic_any("waker panic");
        }
    }

    #[test]
    fn a_panicking_waker_does_not_stop_later_watchers_waking() {
        let (cell, first) = StateCell::new(ConnectionState::Disconnected);
        let second = cell.watch();
        let (counter, waker) = counting();
        let mut one = first.changed();
        let mut two = second.changed();
        assert!(
            poll_with(&mut one, &Waker::from(Arc::new(Panicker))).is_pending(),
            "parks panicker"
        );
        assert!(poll_with(&mut two, &waker).is_pending(), "parks counter");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            cell.set(ConnectionState::Connected);
        }));
        assert!(result.is_err(), "the waker panic is re-raised");
        assert_eq!(counter.wakes(), 1, "later watcher woken");
        assert_eq!(cell.current(), ConnectionState::Connected, "state stored");
    }

    #[test]
    fn setting_on_another_thread_never_loses_a_wake_up() {
        for _ in 0..200 {
            let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
            let (counter, waker) = counting();
            let setter = std::thread::spawn(move || cell.set(ConnectionState::Connected));
            let mut wait = watch.changed();
            let mut woken_or_ready = false;
            while !woken_or_ready {
                woken_or_ready = poll_with(&mut wait, &waker).is_ready() || counter.wakes() > 0;
                std::thread::yield_now();
            }
            assert!(setter.join().is_ok(), "setter thread finished");
            assert_eq!(watch.current(), ConnectionState::Connected, "set visible");
        }
    }

    #[test]
    fn changed_stays_pending_after_the_cell_is_dropped() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let (counter, waker) = counting();
        let mut wait = watch.changed();
        assert!(poll_with(&mut wait, &waker).is_pending(), "pending");
        drop(cell);
        assert!(poll_with(&mut wait, &waker).is_pending(), "still pending");
        assert_eq!(counter.wakes(), 0, "no wake on owner drop");
        assert_eq!(
            watch.current(),
            ConnectionState::Disconnected,
            "last state kept"
        );
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let (cell, watch) = StateCell::new(ConnectionState::Disconnected);
        let shared = Arc::clone(&cell.shared);
        let result = std::thread::spawn(move || {
            let _guard = shared.latest.lock();
            panic!("poison the state lock");
        })
        .join();
        assert!(result.is_err(), "the helper thread panicked");
        assert!(cell.shared.latest.is_poisoned(), "lock is poisoned");
        cell.set(ConnectionState::Connected);
        assert_eq!(
            watch.current(),
            ConnectionState::Connected,
            "usable after poisoning"
        );
    }
}
