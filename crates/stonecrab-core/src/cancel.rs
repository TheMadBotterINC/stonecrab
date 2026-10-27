use std::fmt;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll};

use crate::future::{WakerSlots, wake_all};
use crate::{OpFuture, Result};

#[derive(Default)]
struct State {
    cancelled: bool,
    waiters: WakerSlots,
    children: Vec<Weak<Inner>>,
}

#[derive(Default)]
struct Inner {
    state: Mutex<State>,
}

impl Inner {
    /// Lock the state. A poisoned lock is recovered: every critical section only flips a
    /// flag or moves vectors, so a panic elsewhere cannot leave the state inconsistent.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Cooperative cancellation signal that can be cloned, observed and handed to children.
///
/// All clones share one flag. [`cancel`](Self::cancel) is idempotent and wakes every
/// task currently waiting in [`cancelled`](Self::cancelled) exactly once. A token made
/// with [`child`](Self::child) is cancelled when its parent is; cancelling a child never
/// cancels the parent or its siblings.
///
/// The token is a plain `std::sync::Mutex` over a flag, a list of parked wakers and
/// weak references to children. It needs no runtime, so core stays Tokio-free
/// (ADR-0007). Parents hold children by `Weak`, so a child that nobody references any
/// more is freed and is not woken or counted.
///
/// A token is a request, not a guarantee: code that observes it decides how to unwind.
/// Whether a transmitted request was applied is reported by the operation's own outcome,
/// never inferred from the token.
///
/// # Cancel safety
/// Every method is cancel-safe. Dropping a [`cancelled`](Self::cancelled) future removes
/// its parked waker and leaves the token untouched.
///
/// # Examples
/// ```
/// use std::task::{Context, Poll, Waker};
/// use stonecrab_core::CancellationToken;
///
/// let token = CancellationToken::new();
/// let child = token.child();
/// let mut cx = Context::from_waker(Waker::noop());
/// let mut wait = child.cancelled();
/// assert!(wait.as_mut().poll(&mut cx).is_pending());
///
/// token.cancel();
/// assert!(child.is_cancelled());
/// assert!(matches!(wait.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
/// ```
#[must_use]
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

impl CancellationToken {
    /// Create a token that is not cancelled and has no parent.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::CancellationToken;
    /// let token = CancellationToken::new();
    /// assert!(!token.is_cancelled());
    /// ```
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner::default()),
        }
    }

    /// Create a child token.
    ///
    /// The child is cancelled when this token is, including when this token is already
    /// cancelled. Cancelling the child leaves this token and its other children alone.
    /// The parent keeps only a weak reference, so dropping every handle to the child
    /// frees it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::CancellationToken;
    /// let parent = CancellationToken::new();
    /// let child = parent.child();
    /// child.cancel();
    /// assert!(child.is_cancelled());
    /// assert!(!parent.is_cancelled());
    /// ```
    pub fn child(&self) -> Self {
        let inner = Arc::new(Inner::default());
        {
            let mut parent = self.inner.lock();
            if parent.cancelled {
                inner.lock().cancelled = true;
            } else {
                parent.children.retain(|weak| weak.strong_count() > 0);
                parent.children.push(Arc::downgrade(&inner));
            }
        }
        Self { inner }
    }

    /// Cancel this token and every live descendant.
    ///
    /// Idempotent: only the first call (per token) changes anything. Each parked waker is
    /// woken exactly once, after the internal lock is released, so a waker may call back
    /// into the token. Descendants are visited iteratively, so a deep chain of children
    /// cannot overflow the stack. If a waker panics, the rest are still woken and the
    /// first panic is re-raised afterwards; the whole tree is already cancelled by then.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::CancellationToken;
    /// let token = CancellationToken::new();
    /// token.cancel();
    /// token.cancel();
    /// assert!(token.is_cancelled());
    /// ```
    pub fn cancel(&self) {
        let mut pending = vec![Arc::clone(&self.inner)];
        let mut wakers = Vec::new();
        while let Some(inner) = pending.pop() {
            let children = {
                let mut state = inner.lock();
                if state.cancelled {
                    continue;
                }
                state.cancelled = true;
                wakers.extend(state.waiters.drain());
                std::mem::take(&mut state.children)
            };
            pending.extend(children.iter().filter_map(Weak::upgrade));
        }
        // Every descendant is marked before any user code runs, so a panicking waker
        // cannot strand part of the tree.
        wake_all(wakers);
    }

    /// Whether this token, or any ancestor, has been cancelled.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::CancellationToken;
    /// let token = CancellationToken::new();
    /// assert!(!token.is_cancelled());
    /// token.cancel();
    /// assert!(token.is_cancelled());
    /// ```
    pub fn is_cancelled(&self) -> bool {
        self.inner.lock().cancelled
    }

    /// A future that resolves with `Ok(())` once this token is cancelled.
    ///
    /// It is `Pending` while the token is live and `Ready` on the first poll if the token
    /// is already cancelled. Each poll re-registers the task's waker, replacing the one
    /// from the previous poll; polling again with a waker for which `Waker::will_wake`
    /// holds does nothing. The future never resolves with an error.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future removes its waker; the token is unchanged.
    ///
    /// # Wire effects
    /// None. Observing a token touches no transport.
    ///
    /// # Examples
    /// ```
    /// use std::task::{Context, Poll, Waker};
    /// use stonecrab_core::CancellationToken;
    ///
    /// let token = CancellationToken::new();
    /// let mut cx = Context::from_waker(Waker::noop());
    /// let mut wait = token.cancelled();
    /// assert!(wait.as_mut().poll(&mut cx).is_pending());
    /// token.cancel();
    /// assert!(matches!(wait.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
    /// ```
    pub fn cancelled(&self) -> OpFuture<'_, ()> {
        Box::pin(Cancelled {
            token: self,
            slot: None,
        })
    }

    /// Number of children that are still referenced somewhere.
    fn live_children(&self) -> usize {
        self.inner
            .lock()
            .children
            .iter()
            .filter(|weak| weak.strong_count() > 0)
            .count()
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .field("children", &self.live_children())
            .finish()
    }
}

/// Future behind [`CancellationToken::cancelled`]; `slot` is its parked-waker id.
struct Cancelled<'a> {
    token: &'a CancellationToken,
    slot: Option<u64>,
}

impl Future for Cancelled<'_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let displaced = {
            let mut state = this.token.inner.lock();
            if state.cancelled {
                return Poll::Ready(Ok(()));
            }
            state.waiters.register(&mut this.slot, cx.waker())
        };
        // Dropped after the lock: a waker's Drop may call back into the token.
        drop(displaced);
        Poll::Pending
    }
}

impl Drop for Cancelled<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            let removed = self.token.inner.lock().waiters.release(slot);
            // Dropped after the lock: a waker's Drop may call back into the token.
            drop(removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Wake, Waker};

    use super::*;

    struct Counter(AtomicUsize);

    impl Counter {
        fn wakes(&self) -> usize {
            self.0.load(Ordering::SeqCst)
        }
    }

    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn counting() -> (Arc<Counter>, Waker) {
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(Arc::clone(&counter));
        (counter, waker)
    }

    fn poll_with(future: &mut OpFuture<'_, ()>, waker: &Waker) -> Poll<Result<()>> {
        future.as_mut().poll(&mut Context::from_waker(waker))
    }

    fn waiters(token: &CancellationToken) -> usize {
        token.inner.lock().waiters.len()
    }

    /// A waker whose `Drop` calls back into the token, as a task owning a token handle would.
    struct Reentrant {
        token: CancellationToken,
        drops: Arc<AtomicUsize>,
    }

    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            std::hint::black_box(self.token.is_cancelled());
        }
    }

    impl Drop for Reentrant {
        fn drop(&mut self) {
            std::hint::black_box(self.token.is_cancelled());
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn reentrant(token: &CancellationToken) -> (Arc<AtomicUsize>, Waker) {
        let drops = Arc::new(AtomicUsize::new(0));
        let waker = Waker::from(Arc::new(Reentrant {
            token: token.clone(),
            drops: Arc::clone(&drops),
        }));
        (drops, waker)
    }

    #[test]
    fn a_waker_whose_drop_touches_the_token_cannot_deadlock_on_replace_or_release() {
        let token = CancellationToken::new();
        let (replaced_drops, replaced) = reentrant(&token);
        let (_, replacement) = counting();
        let mut wait = token.cancelled();
        assert!(poll_with(&mut wait, &replaced).is_pending(), "pends");
        replaced.wake_by_ref();
        drop(replaced);
        assert!(
            poll_with(&mut wait, &replacement).is_pending(),
            "repoll pends"
        );
        assert_eq!(
            replaced_drops.load(Ordering::SeqCst),
            1,
            "displaced waker dropped"
        );
        drop(wait);

        let (released_drops, released) = reentrant(&token);
        let mut other = token.cancelled();
        assert!(poll_with(&mut other, &released).is_pending(), "pends");
        released.wake_by_ref();
        drop(released);
        drop(other);
        assert_eq!(
            released_drops.load(Ordering::SeqCst),
            1,
            "released waker dropped"
        );
        assert_eq!(waiters(&token), 0, "nothing left parked");
    }

    struct Panicker;

    impl Wake for Panicker {
        fn wake(self: Arc<Self>) {
            std::panic::panic_any("waker panic");
        }
    }

    #[test]
    fn a_panicking_waker_cannot_strand_descendants_or_other_waiters() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let grand = child.child();
        let (counter, waker) = counting();
        let mut bad = parent.cancelled();
        let mut good = parent.cancelled();
        let panicker = Waker::from(Arc::new(Panicker));
        assert!(
            poll_with(&mut bad, &panicker).is_pending(),
            "parks panicker"
        );
        assert!(poll_with(&mut good, &waker).is_pending(), "parks counter");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| parent.cancel()));
        assert!(result.is_err(), "the waker panic is re-raised");
        assert!(child.is_cancelled(), "child cancelled");
        assert!(grand.is_cancelled(), "grandchild cancelled");
        assert_eq!(counter.wakes(), 1, "later waiter still woken");
    }

    #[test]
    fn cancelling_on_another_thread_never_loses_a_wake_up() {
        for _ in 0..200 {
            let token = CancellationToken::new();
            let (counter, waker) = counting();
            let remote = token.clone();
            let canceller = std::thread::spawn(move || remote.cancel());
            let mut wait = token.cancelled();
            let mut woken_or_ready = false;
            while !woken_or_ready {
                woken_or_ready = poll_with(&mut wait, &waker).is_ready() || counter.wakes() > 0;
                std::thread::yield_now();
            }
            assert!(canceller.join().is_ok(), "canceller thread finished");
            assert!(
                poll_with(&mut wait, &waker).is_ready(),
                "ready after cancel"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn cancel_wakes_each_waiter_exactly_once_and_a_second_cancel_wakes_nothing() {
        let token = CancellationToken::new();
        let (first, first_waker) = counting();
        let (second, second_waker) = counting();
        let mut one = token.cancelled();
        let mut two = token.cancelled();
        assert!(
            poll_with(&mut one, &first_waker).is_pending(),
            "live token pends"
        );
        assert!(
            poll_with(&mut two, &second_waker).is_pending(),
            "live token pends"
        );
        assert!(!token.is_cancelled(), "not cancelled before cancel");
        token.cancel();
        assert!(token.is_cancelled(), "cancelled after cancel");
        assert_eq!(first.wakes(), 1, "first waiter woken once");
        assert_eq!(second.wakes(), 1, "second waiter woken once");
        token.cancel();
        assert_eq!(
            first.wakes(),
            1,
            "second cancel does not wake the first waiter"
        );
        assert_eq!(
            second.wakes(),
            1,
            "second cancel does not wake the second waiter"
        );
        assert!(matches!(
            poll_with(&mut one, &first_waker),
            Poll::Ready(Ok(()))
        ));
        assert!(matches!(
            poll_with(&mut two, &second_waker),
            Poll::Ready(Ok(()))
        ));
        assert_eq!(waiters(&token), 0, "no waker retained after cancel");
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn cancelled_on_an_already_cancelled_token_is_ready_on_the_first_poll() {
        let token = CancellationToken::new();
        token.cancel();
        let (counter, waker) = counting();
        let mut wait = token.cancelled();
        assert!(
            matches!(poll_with(&mut wait, &waker), Poll::Ready(Ok(()))),
            "already cancelled resolves immediately"
        );
        assert_eq!(waiters(&token), 0, "nothing registered");
        assert_eq!(counter.wakes(), 0, "no wake needed");
        let mut again = token.cancelled();
        assert!(matches!(
            poll_with(&mut again, Waker::noop()),
            Poll::Ready(Ok(()))
        ));
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn child_follows_parent_but_parent_does_not_follow_child() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let grandchild = child.child();
        let sibling = parent.child();
        child.cancel();
        assert!(child.is_cancelled(), "child cancelled");
        assert!(grandchild.is_cancelled(), "grandchild follows child");
        assert!(!parent.is_cancelled(), "parent unaffected by child");
        assert!(!sibling.is_cancelled(), "sibling unaffected by child");
        parent.cancel();
        assert!(sibling.is_cancelled(), "sibling follows parent");
        let late = parent.child();
        assert!(
            late.is_cancelled(),
            "child of a cancelled parent starts cancelled"
        );
        assert_eq!(
            parent.live_children(),
            0,
            "cancelled parent keeps no children"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn parent_cancel_wakes_child_waiters_once_and_skips_independently_cancelled_children() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let done = parent.child();
        done.cancel();
        let (counter, waker) = counting();
        let mut wait = child.cancelled();
        assert!(poll_with(&mut wait, &waker).is_pending(), "child pends");
        parent.cancel();
        assert_eq!(counter.wakes(), 1, "child waiter woken once by parent");
        parent.cancel();
        assert_eq!(counter.wakes(), 1, "repeat cancel is a no-op");
        assert!(matches!(poll_with(&mut wait, &waker), Poll::Ready(Ok(()))));
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn dropped_child_is_not_retained_by_its_parent() {
        let parent = CancellationToken::new();
        let child = parent.child();
        let weak = Arc::downgrade(&child.inner);
        assert_eq!(parent.live_children(), 1, "child counted while referenced");
        drop(child);
        assert!(weak.upgrade().is_none(), "parent holds no strong reference");
        assert_eq!(parent.live_children(), 0, "dropped child no longer counted");
        parent.cancel();
        assert!(parent.is_cancelled(), "cancel tolerates a dead child entry");
        let other = CancellationToken::new();
        drop(other.child());
        let kept = other.child();
        assert_eq!(
            other.inner.lock().children.len(),
            1,
            "creating a child prunes dead entries"
        );
        assert_eq!(other.live_children(), 1, "only the kept child is live");
        drop(kept);
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (token semantics)
    #[test]
    fn polling_again_replaces_the_waker_and_the_same_waker_registers_one_entry() {
        let token = CancellationToken::new();
        let (first, first_waker) = counting();
        let (second, second_waker) = counting();
        let mut wait = token.cancelled();
        assert!(
            poll_with(&mut wait, &first_waker).is_pending(),
            "first poll pends"
        );
        assert!(
            poll_with(&mut wait, &first_waker).is_pending(),
            "repoll pends"
        );
        assert!(
            poll_with(&mut wait, &first_waker.clone()).is_pending(),
            "clone pends"
        );
        assert_eq!(waiters(&token), 1, "will_wake dedup keeps one entry");
        assert!(
            poll_with(&mut wait, &second_waker).is_pending(),
            "new waker pends"
        );
        assert_eq!(waiters(&token), 1, "the new waker replaced the old one");
        token.cancel();
        assert_eq!(first.wakes(), 0, "replaced waker is not woken");
        assert_eq!(second.wakes(), 1, "current waker is woken once");
    }

    #[test]
    fn dropping_a_pending_future_removes_its_waker_without_cancelling() {
        let token = CancellationToken::new();
        let (counter, waker) = counting();
        let mut kept = token.cancelled();
        let mut dropped = token.cancelled();
        assert!(poll_with(&mut kept, &waker).is_pending(), "pends");
        assert!(poll_with(&mut dropped, &waker).is_pending(), "pends");
        assert_eq!(waiters(&token), 2, "one slot per future");
        drop(dropped);
        assert_eq!(waiters(&token), 1, "dropped future released its slot");
        assert!(!token.is_cancelled(), "dropping a waiter does not cancel");
        drop(token.cancelled());
        token.cancel();
        assert_eq!(counter.wakes(), 1, "only the live waiter is woken");
        drop(kept);
    }

    #[test]
    fn a_waker_woken_by_cancel_may_call_back_into_the_token_without_deadlock() {
        struct Reentrant(CancellationToken);
        impl Wake for Reentrant {
            fn wake(self: Arc<Self>) {
                assert!(self.0.is_cancelled(), "flag visible inside wake");
            }
        }
        let token = CancellationToken::new();
        let waker = Waker::from(Arc::new(Reentrant(token.clone())));
        let mut wait = token.cancelled();
        assert!(poll_with(&mut wait, &waker).is_pending(), "pends");
        token.cancel();
        assert!(
            poll_with(&mut wait, &waker).is_ready(),
            "ready after the wake"
        );
    }

    #[test]
    fn clones_share_state_and_default_is_a_live_token() {
        let token = CancellationToken::default();
        let clone = token.clone();
        assert!(!clone.is_cancelled(), "default token is live");
        clone.cancel();
        assert!(token.is_cancelled(), "clones share the flag");
    }

    #[test]
    fn deep_child_chains_cancel_without_recursion() {
        let root = CancellationToken::new();
        let mut chain = vec![root.clone()];
        for _ in 0..10_000 {
            let next = chain.last().expect("chain starts non-empty").child();
            chain.push(next);
        }
        root.cancel();
        assert!(
            chain.last().expect("chain is non-empty").is_cancelled(),
            "deepest descendant cancelled"
        );
    }

    #[test]
    fn debug_shows_the_flag_and_child_count_but_no_wakers() {
        let token = CancellationToken::new();
        let _child = token.child();
        let (_counter, waker) = counting();
        let mut wait = token.cancelled();
        assert!(poll_with(&mut wait, &waker).is_pending(), "pends");
        assert_eq!(
            format!("{token:?}"),
            "CancellationToken { cancelled: false, children: 1 }",
            "debug shape before cancel"
        );
        token.cancel();
        assert_eq!(
            format!("{token:?}"),
            "CancellationToken { cancelled: true, children: 0 }",
            "debug shape after cancel"
        );
    }

    #[test]
    fn a_poisoned_lock_is_recovered() {
        let token = CancellationToken::new();
        let inner = Arc::clone(&token.inner);
        let result = std::thread::spawn(move || {
            let _guard = inner.state.lock();
            panic!("poison the token lock");
        })
        .join();
        assert!(result.is_err(), "the helper thread panicked");
        assert!(token.inner.state.is_poisoned(), "lock is poisoned");
        assert!(!token.is_cancelled(), "state readable after poisoning");
        token.cancel();
        assert!(token.is_cancelled(), "state writable after poisoning");
    }
}
