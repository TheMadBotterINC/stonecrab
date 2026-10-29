use std::collections::VecDeque;
use std::fmt;
use std::num::NonZeroUsize;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll};
use std::time::Duration;

use crate::future::{WakerSlots, wake_all};
use crate::{
    ErrorKind, OpFuture, PlcError, QueuePolicy, ReadItem, ResourceLimits, Result, SubscriptionMode,
};

/// Events a subscription gave up, by cause, since it started.
///
/// A snapshot: the counters only grow while the subscription lives, and
/// [`Subscription::unsubscribe`] returns the final values. Which counter a policy feeds is
/// documented on [`QueuePolicy`]; [`QueuePolicy::Backpressure`] loses nothing and leaves
/// both at zero.
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// A Stonecrab addition: upstream counts no lost events.
///
/// # Examples
/// ```
/// use stonecrab::LossCounters;
/// let none = LossCounters::default();
/// assert_eq!((none.dropped, none.coalesced), (0, 0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct LossCounters {
    /// Events discarded to make room in a full queue.
    pub dropped: u64,
    /// Pending events replaced by a newer event for the same tag.
    pub coalesced: u64,
}

/// The queue both ends share, guarded by one mutex (the only lock of this module).
struct Queue {
    items: VecDeque<ReadItem>,
    capacity: NonZeroUsize,
    policy: QueuePolicy,
    loss: LossCounters,
    sender_gone: bool,
    receiver_gone: bool,
    /// Wakers of `recv` futures waiting for an event or the end.
    consumers: WakerSlots,
    /// Wakers of `send` futures waiting for room under backpressure.
    producers: WakerSlots,
}

impl Queue {
    fn is_full(&self) -> bool {
        self.items.len() >= self.capacity.get()
    }

    fn drop_oldest(&mut self) {
        drop(self.items.pop_front());
        self.loss.dropped = self.loss.dropped.saturating_add(1);
    }

    /// Queue `item` under the policy, or hand it back when backpressure says wait.
    fn offer(&mut self, item: ReadItem) -> Option<ReadItem> {
        match self.policy {
            QueuePolicy::Backpressure => {
                if self.is_full() {
                    return Some(item);
                }
            }
            QueuePolicy::CoalesceLatest => {
                if let Some(pending) = self
                    .items
                    .iter_mut()
                    .find(|pending| pending.tag() == item.tag())
                {
                    *pending = item;
                    self.loss.coalesced = self.loss.coalesced.saturating_add(1);
                    return None;
                }
                if self.is_full() {
                    self.drop_oldest();
                }
            }
            QueuePolicy::DropOldest => {
                if self.is_full() {
                    self.drop_oldest();
                }
            }
        }
        self.items.push_back(item);
        None
    }
}

struct Shared {
    queue: Mutex<Queue>,
}

impl Shared {
    /// Lock the queue. A poisoned lock is recovered: the critical sections move items,
    /// bump counters and move wakers, so a panic elsewhere leaves nothing half-written.
    fn lock(&self) -> MutexGuard<'_, Queue> {
        self.queue.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A live subscription: the handle that ends it and the bounded stream of its events.
///
/// Events are [`ReadItem`]s, one per tag and sample, carrying the value or a per-item
/// error, the local sampling instant and the device time when the device supplied one.
/// They wait in a queue of at most [`ResourceLimits::max_queued_events`] events, governed
/// by the request's [`QueuePolicy`]; what the policy gave up is in [`loss`](Self::loss).
///
/// # Lifetime
/// The subscription lives until it is unsubscribed or dropped, or until its connection
/// closes or fails. [`unsubscribe`](Self::unsubscribe) and `drop` end it at once; events
/// still queued are discarded by that choice and are not counted as loss. When the
/// connection ends it, the producer stops and [`recv`](Self::recv) still delivers every
/// queued event before it reports the end with `None`; the reason is the connection's
/// state. A reconnect does not end it (ADR-0020 decision 6).
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// Corresponds to the upstream `PlcSubscriptionResponse`, its handles and the
/// `PlcSubscriptionEvent`s their consumers receive: one handle for the whole request
/// instead of one per tag, and a pull-based queue instead of consumer callbacks.
///
/// # Examples
/// A polling subscription discloses that it polls and its sampling interval, and delivers
/// what was queued before its producer ended.
/// ```
/// use std::task::{Context, Poll, Waker};
/// use std::time::{Duration, Instant};
/// use stonecrab::{
///     PlcValue, QueuePolicy, ReadItem, ResourceLimits, SubscriptionMode, SubscriptionSender,
///     TagId,
/// };
///
/// let limits = ResourceLimits::builder().build()?;
/// let interval = Duration::from_secs(1);
/// let (sender, subscription) = SubscriptionSender::polling(interval, QueuePolicy::DropOldest, limits);
/// assert_eq!(subscription.mode(), SubscriptionMode::Polling);
/// assert_eq!(subscription.interval(), Some(interval));
///
/// let mut cx = Context::from_waker(Waker::noop());
/// let item = ReadItem::new(TagId::new("level")?, Ok(PlcValue::U16(7)), Instant::now());
/// assert!(sender.send(item).as_mut().poll(&mut cx).is_ready());
/// drop(sender);
/// let mut next = subscription.recv();
/// assert!(matches!(next.as_mut().poll(&mut cx), Poll::Ready(Ok(Some(_)))));
/// drop(next);
/// assert!(matches!(subscription.recv().as_mut().poll(&mut cx), Poll::Ready(Ok(None))));
/// assert_eq!(subscription.unsubscribe().dropped, 0);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[must_use = "a subscription ends when it is dropped"]
pub struct Subscription {
    shared: Arc<Shared>,
    mode: SubscriptionMode,
    interval: Option<Duration>,
}

impl Subscription {
    /// How the events are obtained: [`SubscriptionMode::Polling`] for scheduled reads,
    /// [`SubscriptionMode::Native`] for the device's own mechanism.
    #[must_use]
    pub const fn mode(&self) -> SubscriptionMode {
        self.mode
    }

    /// The sampling interval the driver actually uses; always present for a polling
    /// subscription.
    #[must_use]
    pub const fn interval(&self) -> Option<Duration> {
        self.interval
    }

    /// The events given up so far.
    #[must_use]
    pub fn loss(&self) -> LossCounters {
        self.shared.lock().loss
    }

    /// The next event, waiting while the queue is empty.
    ///
    /// Resolves to `Some(event)` in queue order, and to `None` once the producer has ended
    /// (its connection closed or failed) and every queued event has been delivered; after
    /// that every call resolves to `None` at once. The future never resolves with an error.
    /// Taking an event makes room for a producer that waits under
    /// [`QueuePolicy::Backpressure`].
    ///
    /// # Cancel safety
    /// Cancel-safe. An event is taken from the queue only when the future returns it, so
    /// dropping a pending future loses nothing.
    ///
    /// # Wire effects
    /// None. Receiving touches no transport; the producer acquires the events.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{Result, Subscription};
    ///
    /// async fn count(subscription: &Subscription) -> Result<usize> {
    ///     let mut events = 0;
    ///     while subscription.recv().await?.is_some() {
    ///         events += 1;
    ///     }
    ///     Ok(events)
    /// }
    /// ```
    pub fn recv(&self) -> OpFuture<'_, Option<ReadItem>> {
        Box::pin(Recv {
            shared: &self.shared,
            slot: None,
        })
    }

    /// End the subscription now and return the final loss counters.
    ///
    /// Equivalent to dropping it, except that the counters are read after the producer can
    /// no longer add to them. Events still queued are discarded and not counted as loss.
    pub fn unsubscribe(self) -> LossCounters {
        self.close()
    }

    /// Mark the consumer gone, discard the queue and wake every waiting producer.
    fn close(&self) -> LossCounters {
        let (loss, discarded, wakers) = {
            let mut queue = self.shared.lock();
            queue.receiver_gone = true;
            (
                queue.loss,
                std::mem::take(&mut queue.items),
                queue.producers.drain(),
            )
        };
        // Dropped and woken after the lock: a waker may call back into the queue.
        drop(discarded);
        wake_all(wakers);
        loss
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.close();
    }
}

impl fmt::Debug for Subscription {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let queue = self.shared.lock();
        f.debug_struct("Subscription")
            .field("mode", &self.mode)
            .field("interval", &self.interval)
            .field("policy", &queue.policy)
            .field("capacity", &queue.capacity)
            .field("queued", &queue.items.len())
            .field("loss", &queue.loss)
            .finish()
    }
}

/// The producer end of a [`Subscription`]: what a driver holds to deliver events.
///
/// A driver that implements [`Connection::subscribe`](crate::Connection::subscribe) makes
/// the pair with [`polling`](Self::polling), returns the [`Subscription`] and keeps the
/// sender in the task that acquires the events. Dropping the sender ends the stream: the
/// consumer receives every queued event, then `None`. A connection drops the senders of
/// its subscriptions when it closes or fails.
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// Corresponds to the delivery side of the upstream polling helper, which hands each event
/// to the registered consumers; here the event goes into the bounded queue.
pub struct SubscriptionSender {
    shared: Arc<Shared>,
}

impl SubscriptionSender {
    /// Make a polling subscription that samples every `interval` and queues at most
    /// `limits.max_queued_events()` events under `queue`.
    ///
    /// The subscription reports [`SubscriptionMode::Polling`] and `interval`, which must be
    /// the interval the driver actually polls at. Nothing is allocated for the events until
    /// they arrive.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab::{QueuePolicy, ResourceLimits, SubscriptionSender};
    /// let limits = ResourceLimits::builder().max_queued_events(8).build()?;
    /// let (sender, subscription) =
    ///     SubscriptionSender::polling(Duration::from_millis(500), QueuePolicy::Backpressure, limits);
    /// assert!(!sender.is_closed());
    /// drop(subscription);
    /// assert!(sender.is_closed());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn polling(
        interval: Duration,
        queue: QueuePolicy,
        limits: ResourceLimits,
    ) -> (Self, Subscription) {
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                items: VecDeque::new(),
                capacity: limits.max_queued_events(),
                policy: queue,
                loss: LossCounters::default(),
                sender_gone: false,
                receiver_gone: false,
                consumers: WakerSlots::default(),
                producers: WakerSlots::default(),
            }),
        });
        let subscription = Subscription {
            shared: Arc::clone(&shared),
            mode: SubscriptionMode::Polling,
            interval: Some(interval),
        };
        (Self { shared }, subscription)
    }

    /// Queue one event under the subscription's [`QueuePolicy`].
    ///
    /// Under [`QueuePolicy::CoalesceLatest`] and [`QueuePolicy::DropOldest`] the future
    /// resolves at once, after counting what the policy gave up. Under
    /// [`QueuePolicy::Backpressure`] it waits while the queue is full and resolves when the
    /// consumer has made room.
    ///
    /// # Errors
    /// Returns [`ErrorKind::Cancelled`] when the subscription was unsubscribed or dropped;
    /// the event is discarded and the producer should stop.
    ///
    /// # Cancel safety
    /// Cancel-safe for the queue: dropping the future before it resolves leaves the event
    /// unqueued, and a producer that does so accounts for the sample itself (a polling
    /// producer counts it as a skipped tick).
    ///
    /// # Wire effects
    /// None. Queueing touches no transport.
    ///
    /// # Examples
    /// ```
    /// use std::time::{Duration, Instant};
    /// use stonecrab::{ErrorKind, PlcValue, QueuePolicy, ReadItem, ResourceLimits, SubscriptionSender, TagId};
    ///
    /// async fn deliver(sender: &SubscriptionSender, value: u16) -> stonecrab::Result<bool> {
    ///     let item = ReadItem::new(TagId::new("level")?, Ok(PlcValue::U16(value)), Instant::now());
    ///     match sender.send(item).await {
    ///         Ok(()) => Ok(true),
    ///         Err(error) if error.kind() == ErrorKind::Cancelled => Ok(false),
    ///         Err(error) => Err(error),
    ///     }
    /// }
    /// ```
    pub fn send(&self, item: ReadItem) -> OpFuture<'_, ()> {
        Box::pin(Sending {
            shared: &self.shared,
            item: Some(item),
            slot: None,
        })
    }

    /// Whether the subscription was unsubscribed or dropped, so the producer can stop
    /// acquiring events it would never deliver.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shared.lock().receiver_gone
    }
}

impl Drop for SubscriptionSender {
    fn drop(&mut self) {
        let wakers = {
            let mut queue = self.shared.lock();
            queue.sender_gone = true;
            queue.consumers.drain()
        };
        wake_all(wakers);
    }
}

impl fmt::Debug for SubscriptionSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let queue = self.shared.lock();
        f.debug_struct("SubscriptionSender")
            .field("policy", &queue.policy)
            .field("capacity", &queue.capacity)
            .field("queued", &queue.items.len())
            .field("closed", &queue.receiver_gone)
            .finish()
    }
}

/// Future behind [`Subscription::recv`]; `slot` is its parked-waker id.
struct Recv<'a> {
    shared: &'a Shared,
    slot: Option<u64>,
}

impl Future for Recv<'_> {
    type Output = Result<Option<ReadItem>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let mut queue = this.shared.lock();
        let (outcome, wakers, displaced) = if let Some(item) = queue.items.pop_front() {
            (Poll::Ready(Ok(Some(item))), queue.producers.drain(), None)
        } else if queue.sender_gone {
            (Poll::Ready(Ok(None)), Vec::new(), None)
        } else {
            let displaced = queue.consumers.register(&mut this.slot, cx.waker());
            (Poll::Pending, Vec::new(), displaced)
        };
        // Dropped and woken after the lock: a waker may call back into the queue.
        drop(queue);
        drop(displaced);
        wake_all(wakers);
        outcome
    }
}

impl Drop for Recv<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            let removed = self.shared.lock().consumers.release(slot);
            drop(removed);
        }
    }
}

/// Future behind [`SubscriptionSender::send`]; `item` is the event until it is queued.
struct Sending<'a> {
    shared: &'a Shared,
    item: Option<ReadItem>,
    slot: Option<u64>,
}

impl Future for Sending<'_> {
    type Output = Result<()>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = &mut *self;
        let mut queue = this.shared.lock();
        if queue.receiver_gone {
            drop(queue);
            return Poll::Ready(Err(PlcError::new(
                ErrorKind::Cancelled,
                "the subscription was unsubscribed or dropped",
            )));
        }
        // A future polled again after it resolved has nothing left to queue.
        let Some(item) = this.item.take() else {
            return Poll::Ready(Ok(()));
        };
        let (outcome, wakers, displaced) = match queue.offer(item) {
            None => (Poll::Ready(Ok(())), queue.consumers.drain(), None),
            Some(item) => {
                this.item = Some(item);
                let displaced = queue.producers.register(&mut this.slot, cx.waker());
                (Poll::Pending, Vec::new(), displaced)
            }
        };
        // Dropped and woken after the lock: a waker may call back into the queue.
        drop(queue);
        drop(displaced);
        wake_all(wakers);
        outcome
    }
}

impl Drop for Sending<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot {
            let removed = self.shared.lock().producers.release(slot);
            drop(removed);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::{Wake, Waker};
    use std::time::Instant;

    use proptest::prelude::*;

    use super::*;
    use crate::{PlcValue, TagId};

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

    fn pair(policy: QueuePolicy, capacity: usize) -> (SubscriptionSender, Subscription) {
        let limits = ResourceLimits::builder()
            .max_queued_events(capacity)
            .build()
            .expect("valid budgets");
        SubscriptionSender::polling(Duration::from_millis(250), policy, limits)
    }

    fn event(tag: &str, value: u32) -> ReadItem {
        ReadItem::new(
            TagId::new(tag).expect("valid identity"),
            Ok(PlcValue::U32(value)),
            Instant::now(),
        )
    }

    fn value(item: &ReadItem) -> (String, u32) {
        let number = item
            .outcome()
            .as_ref()
            .ok()
            .and_then(PlcValue::as_u64)
            .and_then(|wide| u32::try_from(wide).ok())
            .expect("a test event carries a u32");
        (item.tag().as_str().to_owned(), number)
    }

    fn poll<T>(future: &mut OpFuture<'_, T>, waker: &Waker) -> Poll<Result<T>> {
        future.as_mut().poll(&mut Context::from_waker(waker))
    }

    /// Queue `item`, which must not wait.
    fn sent(sender: &SubscriptionSender, item: ReadItem) {
        let outcome = poll(&mut sender.send(item), Waker::noop());
        assert!(matches!(outcome, Poll::Ready(Ok(()))), "queued at once");
    }

    /// The next event when one is ready; `Ready(None)` is the end of the stream (`recv`
    /// never fails, so no error can hide behind it).
    fn received(subscription: &Subscription) -> Poll<Option<(String, u32)>> {
        poll(&mut subscription.recv(), Waker::noop())
            .map(|outcome| outcome.ok().flatten().as_ref().map(value))
    }

    /// Take the next event, which must be ready, into `delivered`.
    fn take(subscription: &Subscription, delivered: &mut Vec<(String, u32)>) {
        let next = received(subscription);
        assert!(matches!(next, Poll::Ready(Some(_))), "an event is ready");
        if let Poll::Ready(Some(item)) = next {
            delivered.push(item);
        }
    }

    fn queued(subscription: &Subscription) -> Vec<(String, u32)> {
        subscription.shared.lock().items.iter().map(value).collect()
    }

    fn entry(tag: &str, value: u32) -> (String, u32) {
        (tag.to_owned(), value)
    }

    // covers: SPEC mandatory-failure-cases / F6 queue exhaustion, slow consumer
    #[test]
    fn backpressure_holds_a_full_queue_until_the_consumer_takes_an_event() {
        let (sender, subscription) = pair(QueuePolicy::Backpressure, 2);
        sent(&sender, event("a", 1));
        sent(&sender, event("a", 2));
        let (wakes, waker) = counting();
        let mut third = sender.send(event("a", 3));
        assert!(poll(&mut third, &waker).is_pending(), "a full queue waits");
        assert!(poll(&mut third, &waker).is_pending(), "and keeps waiting");
        assert_eq!(queued(&subscription).len(), 2, "never above capacity");
        assert_eq!(
            received(&subscription),
            Poll::Ready(Some(entry("a", 1))),
            "oldest first"
        );
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "room wakes the producer");
        assert!(
            matches!(poll(&mut third, &waker), Poll::Ready(Ok(()))),
            "the waiting event is queued"
        );
        assert!(
            matches!(poll(&mut third, &waker), Poll::Ready(Ok(()))),
            "a resolved send has nothing left to queue"
        );
        drop(third);
        assert_eq!(
            queued(&subscription),
            [entry("a", 2), entry("a", 3)],
            "nothing lost, order kept"
        );
        assert_eq!(subscription.loss(), LossCounters::default(), "no loss");
    }

    #[test]
    fn a_producer_waiting_on_backpressure_learns_that_the_subscription_ended() {
        let (sender, subscription) = pair(QueuePolicy::Backpressure, 1);
        sent(&sender, event("a", 1));
        let (wakes, waker) = counting();
        let mut waiting = sender.send(event("a", 2));
        assert!(poll(&mut waiting, &waker).is_pending(), "full");
        drop(subscription);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "the drop wakes it");
        assert!(
            matches!(
                poll(&mut waiting, &waker),
                Poll::Ready(Err(ref error)) if error.kind() == ErrorKind::Cancelled
            ),
            "the waiting send fails with Cancelled"
        );
        assert!(sender.is_closed(), "the producer can stop");
    }

    #[test]
    fn dropping_a_waiting_send_or_recv_releases_its_waker_slot() {
        let (sender, subscription) = pair(QueuePolicy::Backpressure, 1);
        let mut waiting = subscription.recv();
        assert!(poll(&mut waiting, Waker::noop()).is_pending(), "empty");
        assert_eq!(subscription.shared.lock().consumers.len(), 1, "parked");
        drop(waiting);
        assert_eq!(subscription.shared.lock().consumers.len(), 0, "released");
        sent(&sender, event("a", 1));
        let mut blocked = sender.send(event("a", 2));
        assert!(poll(&mut blocked, Waker::noop()).is_pending(), "full");
        assert_eq!(subscription.shared.lock().producers.len(), 1, "parked");
        drop(blocked);
        assert_eq!(subscription.shared.lock().producers.len(), 0, "released");
        assert_eq!(
            queued(&subscription),
            [entry("a", 1)],
            "the dropped send queued nothing"
        );
    }

    // covers: SPEC mandatory-failure-cases / F6 slow consumer (latest-value coalescing)
    #[test]
    fn coalescing_replaces_the_pending_event_of_a_tag_in_place_and_counts_it() {
        let (sender, subscription) = pair(QueuePolicy::CoalesceLatest, 4);
        sent(&sender, event("a", 1));
        sent(&sender, event("b", 1));
        sent(&sender, event("a", 2));
        sent(&sender, event("a", 3));
        assert_eq!(
            queued(&subscription),
            [entry("a", 3), entry("b", 1)],
            "the latest value per tag, in first-pending order"
        );
        assert_eq!(subscription.loss().coalesced, 2, "two replaced values");
        assert_eq!(subscription.loss().dropped, 0, "nothing dropped");
        assert_eq!(
            received(&subscription),
            Poll::Ready(Some(entry("a", 3))),
            "latest a"
        );
        sent(&sender, event("a", 4));
        assert_eq!(
            queued(&subscription),
            [entry("b", 1), entry("a", 4)],
            "a delivered tag queues again at the back"
        );
    }

    // covers: SPEC mandatory-failure-cases / F6 dropped events (coalescing with too many tags)
    #[test]
    fn coalescing_with_more_tags_than_room_drops_the_oldest_and_counts_it() {
        let (sender, subscription) = pair(QueuePolicy::CoalesceLatest, 2);
        sent(&sender, event("a", 1));
        sent(&sender, event("b", 1));
        sent(&sender, event("c", 1));
        assert_eq!(
            queued(&subscription),
            [entry("b", 1), entry("c", 1)],
            "the oldest went"
        );
        assert_eq!(subscription.loss().dropped, 1, "and was counted");
    }

    // covers: SPEC mandatory-failure-cases / F6 dropped events
    #[test]
    fn dropping_discards_the_oldest_event_and_counts_each_one() {
        let (sender, subscription) = pair(QueuePolicy::DropOldest, 2);
        for number in 1..=5 {
            sent(&sender, event("a", number));
        }
        assert_eq!(
            queued(&subscription),
            [entry("a", 4), entry("a", 5)],
            "the newest stay"
        );
        assert_eq!(
            subscription.loss(),
            LossCounters {
                dropped: 3,
                coalesced: 0
            },
            "every discarded event is counted"
        );
    }

    #[test]
    fn an_event_and_the_end_of_the_producer_each_wake_a_waiting_consumer() {
        let (sender, subscription) = pair(QueuePolicy::DropOldest, 4);
        let (wakes, waker) = counting();
        let mut waiting = subscription.recv();
        assert!(poll(&mut waiting, &waker).is_pending(), "nothing yet");
        sent(&sender, event("a", 1));
        assert_eq!(wakes.0.load(Ordering::SeqCst), 1, "an event wakes it");
        assert!(
            matches!(poll(&mut waiting, &waker), Poll::Ready(Ok(Some(_)))),
            "the event"
        );
        drop(waiting);
        let mut waiting = subscription.recv();
        assert!(poll(&mut waiting, &waker).is_pending(), "empty again");
        drop(sender);
        assert_eq!(wakes.0.load(Ordering::SeqCst), 2, "the end wakes it");
        assert!(
            matches!(poll(&mut waiting, &waker), Poll::Ready(Ok(None))),
            "the end"
        );
    }

    #[test]
    fn the_end_of_the_producer_delivers_the_queue_then_none_forever() {
        let (sender, subscription) = pair(QueuePolicy::DropOldest, 4);
        sent(&sender, event("a", 1));
        sent(&sender, event("b", 2));
        drop(sender);
        assert_eq!(
            received(&subscription),
            Poll::Ready(Some(entry("a", 1))),
            "queued first"
        );
        assert_eq!(
            received(&subscription),
            Poll::Ready(Some(entry("b", 2))),
            "queued second"
        );
        assert_eq!(received(&subscription), Poll::Ready(None), "then the end");
        assert_eq!(
            received(&subscription),
            Poll::Ready(None),
            "and it stays ended"
        );
    }

    #[test]
    fn unsubscribe_returns_the_final_counts_discards_the_queue_and_refuses_the_producer() {
        let (sender, subscription) = pair(QueuePolicy::DropOldest, 1);
        sent(&sender, event("a", 1));
        sent(&sender, event("a", 2));
        assert!(!sender.is_closed(), "open");
        let loss = subscription.unsubscribe();
        assert_eq!(loss.dropped, 1, "final counts");
        assert!(sender.is_closed(), "closed");
        assert!(
            sender.shared.lock().items.is_empty(),
            "queued events are discarded, not counted"
        );
        assert!(
            matches!(
                poll(&mut sender.send(event("a", 3)), Waker::noop()),
                Poll::Ready(Err(ref error)) if error.kind() == ErrorKind::Cancelled
            ),
            "a send after unsubscribe fails with Cancelled"
        );
    }

    #[test]
    fn both_ends_disclose_their_shape_in_debug() {
        let (sender, subscription) = pair(QueuePolicy::CoalesceLatest, 3);
        sent(&sender, event("a", 1));
        assert_eq!(subscription.mode(), SubscriptionMode::Polling, "polling");
        assert_eq!(
            subscription.interval(),
            Some(Duration::from_millis(250)),
            "interval"
        );
        let text = format!("{subscription:?}");
        for field in [
            "Polling",
            "250ms",
            "CoalesceLatest",
            "capacity: 3",
            "queued: 1",
            "loss",
        ] {
            assert!(text.contains(field), "{field} in {text}");
        }
        let text = format!("{sender:?}");
        for field in ["CoalesceLatest", "queued: 1", "closed: false"] {
            assert!(text.contains(field), "{field} in {text}");
        }
    }

    #[test]
    fn the_handles_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<Subscription>();
        assert_send_sync::<SubscriptionSender>();
        assert_send_sync::<LossCounters>();
    }

    #[derive(Debug, Clone)]
    enum Op {
        Send(u8),
        Recv,
    }

    fn ops(tags: u8) -> impl Strategy<Value = Vec<Op>> {
        prop::collection::vec(
            prop_oneof![
                3 => (0..tags).prop_map(Op::Send),
                1 => Just(Op::Recv),
            ],
            0..64,
        )
    }

    /// Run `ops` with sequence-numbered events, checking the bound after every step, then
    /// end the producer and drain. Returns every delivered event and the final counters.
    fn run(
        policy: QueuePolicy,
        capacity: usize,
        ops: &[Op],
    ) -> (Vec<(String, u32)>, LossCounters, u64) {
        let (sender, subscription) = pair(policy, capacity);
        let mut delivered = Vec::new();
        let mut sends = 0_u32;
        for op in ops {
            match op {
                Op::Send(tag) => {
                    sends = sends.saturating_add(1);
                    let mut send = sender.send(event(&format!("t{tag}"), sends));
                    if poll(&mut send, Waker::noop()).is_pending() {
                        // Backpressure: the consumer makes room, then the event goes in.
                        assert_eq!(policy, QueuePolicy::Backpressure, "only backpressure waits");
                        take(&subscription, &mut delivered);
                        assert!(
                            matches!(poll(&mut send, Waker::noop()), Poll::Ready(Ok(()))),
                            "room lets the waiting event in"
                        );
                    }
                }
                Op::Recv => {
                    if !subscription.shared.lock().items.is_empty() {
                        take(&subscription, &mut delivered);
                    }
                }
            }
            assert!(
                subscription.shared.lock().items.len() <= capacity,
                "the queue never exceeds max_queued_events"
            );
        }
        drop(sender);
        while let Poll::Ready(Some(item)) = received(&subscription) {
            delivered.push(item);
        }
        assert_eq!(
            received(&subscription),
            Poll::Ready(None),
            "drained to the end"
        );
        (delivered, subscription.unsubscribe(), u64::from(sends))
    }

    fn total(delivered: &[(String, u32)], loss: LossCounters) -> u64 {
        u64::try_from(delivered.len())
            .unwrap_or(u64::MAX)
            .saturating_add(loss.dropped)
            .saturating_add(loss.coalesced)
    }

    fn increasing(delivered: &[(String, u32)]) -> bool {
        delivered
            .iter()
            .zip(delivered.iter().skip(1))
            .all(|(earlier, later)| earlier.1 < later.1)
    }

    proptest! {
        // covers: SPEC mandatory-failure-cases / F6 queue exhaustion, slow consumer
        #[test]
        fn backpressure_delivers_every_event_in_order(capacity in 1_usize..5, ops in ops(4)) {
            let (delivered, loss, sends) = run(QueuePolicy::Backpressure, capacity, &ops);
            prop_assert_eq!(loss, LossCounters::default());
            prop_assert_eq!(u64::try_from(delivered.len()).unwrap_or(u64::MAX), sends);
            prop_assert!(increasing(&delivered), "order kept: {:?}", delivered);
        }

        // covers: SPEC mandatory-failure-cases / F6 dropped events
        #[test]
        fn dropping_accounts_for_every_event_and_keeps_order(
            capacity in 1_usize..5,
            ops in ops(4),
        ) {
            let (delivered, loss, sends) = run(QueuePolicy::DropOldest, capacity, &ops);
            prop_assert_eq!(loss.coalesced, 0);
            prop_assert_eq!(total(&delivered, loss), sends);
            prop_assert!(increasing(&delivered), "order kept: {:?}", delivered);
        }

        // covers: SPEC mandatory-failure-cases / F6 slow consumer (latest-value coalescing)
        #[test]
        fn coalescing_keeps_the_latest_value_per_tag_and_accounts_for_every_event(
            capacity in 4_usize..8,
            ops in ops(4),
        ) {
            let (sender, subscription) = pair(QueuePolicy::CoalesceLatest, capacity);
            let mut latest = BTreeMap::new();
            let mut sends = 0_u32;
            let mut delivered = Vec::new();
            for op in &ops {
                match op {
                    Op::Send(tag) => {
                        sends = sends.saturating_add(1);
                        let name = format!("t{tag}");
                        latest.insert(name.clone(), sends);
                        sent(&sender, event(&name, sends));
                    }
                    Op::Recv => {
                        if !subscription.shared.lock().items.is_empty() {
                            take(&subscription, &mut delivered);
                        }
                    }
                }
                let pending = queued(&subscription);
                prop_assert!(pending.len() <= capacity, "bounded");
                for (tag, number) in &pending {
                    prop_assert_eq!(latest.get(tag), Some(number), "the latest value of {}", tag);
                    prop_assert_eq!(
                        pending.iter().filter(|(other, _)| other == tag).count(),
                        1,
                        "one pending event per tag"
                    );
                }
            }
            drop(sender);
            while let Poll::Ready(Some(item)) = received(&subscription) {
                delivered.push(item);
            }
            let loss = subscription.unsubscribe();
            prop_assert_eq!(loss.dropped, 0, "every tag fits");
            prop_assert_eq!(total(&delivered, loss), u64::from(sends));
        }
    }
}
