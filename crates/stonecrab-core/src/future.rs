use std::future::Future;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::pin::Pin;
use std::task::Waker;

use crate::Result;

/// The future returned by every object-safe asynchronous operation in Stonecrab.
///
/// `OpFuture<'a, T>` is a pinned, boxed, `Send` future that resolves to
/// [`Result<T>`](crate::Result). Public traits such as `Clock` return it instead of
/// using `async fn`, for three reasons:
///
/// - **Object safety.** A trait method returning `impl Future` or an `async fn` cannot
///   be called through `dyn Trait`. Drivers, connections and clocks are chosen at run
///   time (`Arc<dyn Clock>`, `Box<dyn Connection>`), so their methods return a concrete,
///   nameable type. A boxed future is that type.
/// - **A visible `Send` bound.** Hand-written `Box::pin(async move { ... })` bodies
///   state `Send` in the type, so a future that accidentally holds a non-`Send` value
///   across an await point fails to compile at the implementation, not at a distant
///   caller. The `async-trait` macro is not used because it hides that bound and the
///   allocation (ADR-0001, ADR-0002; coding standards 8.2).
/// - **No executor.** The alias names only `std` types, so `stonecrab-core` stays
///   independent of any runtime.
///
/// The lifetime `'a` lets an implementation borrow from `&self` or from arguments for
/// as long as the future lives. The price is one heap allocation per operation, which is
/// small beside a network round trip and is accepted deliberately.
///
/// # Examples
/// ```
/// use std::task::{Context, Poll, Waker};
/// use stonecrab_core::OpFuture;
///
/// // An implementation borrows its input and returns a boxed, `Send` future.
/// fn length_of<'a>(text: &'a str) -> OpFuture<'a, usize> {
///     Box::pin(async move { Ok(text.len()) })
/// }
///
/// fn assert_send<T: Send>(_: &T) {}
///
/// let mut future = length_of("abc");
/// assert_send(&future);
/// let mut cx = Context::from_waker(Waker::noop());
/// assert!(matches!(future.as_mut().poll(&mut cx), Poll::Ready(Ok(3))));
/// ```
pub type OpFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

/// Parked task wakers, one slot per waiting future, shared by the token and the watch.
///
/// A waiting future keeps the slot id it was given. Polling again with the same waker
/// (`Waker::will_wake`) changes nothing; polling with a different waker replaces the
/// parked one in place, so repeated polls never grow the list. The owner drains the
/// list and wakes each waker outside its lock.
#[derive(Default)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct WakerSlots {
    next: u64,
    entries: Vec<(u64, Waker)>,
}

impl WakerSlots {
    /// Park `waker` in the slot held by `slot`, allocating a slot on first use or when
    /// the previous slot was drained by a wake-up.
    ///
    /// Returns the waker this call displaced, if any. The caller must drop it only after
    /// releasing its lock, because a waker's `Drop` may call back into the token or cell.
    #[must_use = "drop the displaced waker after unlocking"]
    pub(crate) fn register(&mut self, slot: &mut Option<u64>, waker: &Waker) -> Option<Waker> {
        let existing = slot.and_then(|id| self.entries.iter_mut().find(|(entry, _)| *entry == id));
        if let Some((_, parked)) = existing {
            if parked.will_wake(waker) {
                None
            } else {
                Some(std::mem::replace(parked, waker.clone()))
            }
        } else {
            let id = self.next;
            self.next = self.next.wrapping_add(1);
            self.entries.push((id, waker.clone()));
            *slot = Some(id);
            None
        }
    }

    /// Forget the slot of a future that is being dropped. Unknown ids are ignored.
    ///
    /// Returns the removed waker, if any; the caller drops it after releasing its lock.
    #[must_use = "drop the removed waker after unlocking"]
    pub(crate) fn release(&mut self, slot: u64) -> Option<Waker> {
        let index = self.entries.iter().position(|(entry, _)| *entry == slot)?;
        Some(self.entries.remove(index).1)
    }

    /// Remove and return every parked waker so the caller can wake them after unlocking.
    pub(crate) fn drain(&mut self) -> Vec<Waker> {
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|(_, waker)| waker)
            .collect()
    }

    /// Number of parked wakers.
    #[cfg(test)]
    pub(crate) const fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Wake every waker, even if some of them panic.
///
/// Each waker is called once. A panic from one waker does not stop the rest from being
/// woken; the first panic is re-raised after the last waker has run.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn wake_all(wakers: Vec<Waker>) {
    let mut first_panic = None;
    for waker in wakers {
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| waker.wake())) {
            first_panic.get_or_insert(payload);
        }
    }
    if let Some(payload) = first_panic {
        resume_unwind(payload);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    use super::*;

    struct Counter(AtomicUsize);

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

    #[test]
    fn same_waker_keeps_one_slot_and_a_new_waker_replaces_it_in_place() {
        let (first, first_waker) = counting();
        let (second, second_waker) = counting();
        let mut slots = WakerSlots::default();
        let mut slot = None;
        drop(slots.register(&mut slot, &first_waker));
        drop(slots.register(&mut slot, &first_waker.clone()));
        assert_eq!(slots.len(), 1, "same waker registers one entry");
        drop(slots.register(&mut slot, &second_waker));
        assert_eq!(slots.len(), 1, "a new waker replaces the parked one");
        for waker in slots.drain() {
            waker.wake();
        }
        assert_eq!(
            first.0.load(Ordering::SeqCst),
            0,
            "replaced waker not woken"
        );
        assert_eq!(second.0.load(Ordering::SeqCst), 1, "replacement woken once");
        assert_eq!(slots.len(), 0, "drain empties the list");
    }

    struct Panicker;

    impl Wake for Panicker {
        fn wake(self: Arc<Self>) {
            std::panic::panic_any("waker panic");
        }
    }

    #[test]
    fn wake_all_wakes_every_waker_then_reraises_the_first_panic() {
        let (before, before_waker) = counting();
        let (after, after_waker) = counting();
        let wakers = vec![before_waker, Waker::from(Arc::new(Panicker)), after_waker];
        let result = catch_unwind(AssertUnwindSafe(|| wake_all(wakers)));
        assert!(result.is_err(), "the panic is re-raised");
        assert_eq!(before.0.load(Ordering::SeqCst), 1, "earlier waker woken");
        assert_eq!(after.0.load(Ordering::SeqCst), 1, "later waker woken");
        wake_all(Vec::new());
    }

    #[test]
    fn drained_slot_registers_again_and_release_forgets_it() {
        let (_counter, waker) = counting();
        let mut slots = WakerSlots::default();
        let mut slot = None;
        drop(slots.register(&mut slot, &waker));
        let first = slot.expect("slot allocated");
        drop(slots.drain());
        drop(slots.register(&mut slot, &waker));
        assert_ne!(slot, Some(first), "a drained slot gets a fresh id");
        assert_eq!(slots.len(), 1, "re-registered after drain");
        drop(slots.release(first));
        assert_eq!(slots.len(), 1, "unknown id is ignored");
        drop(slots.release(slot.expect("slot present")));
        assert_eq!(slots.len(), 0, "release removes the entry");
    }
}
