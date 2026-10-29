//! Polling subscriptions: the producer that a driver without a native subscription hands
//! its [`Subscription`] (ADR-0020, addendum S55).
//!
//! A driver makes a [`Plan`] from the request before any I/O: an event subscription is
//! refused, and the read every tick performs is built against the connection's own limits
//! so the driver can check its addresses. [`Plan::start`] makes the bounded queue and the
//! producer, which the subscription drives from `recv` (no task is spawned: the core has no
//! executor, `CODING_STANDARDS` 8.1). Each tick reads every tag through the driver's own
//! read path, so a polling read takes the connection's single in-flight slot, deadline and
//! retry policy like any other read.

use std::future::poll_fn;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::subscription::Gate;
use crate::{
    Clock, ConnectionState, ErrorKind, ItemError, OpFuture, Operation, PlcError, PlcValue,
    QueuePolicy, ReadItem, ReadRequest, ReadRequestBuilder, ReadResponse, ResourceLimits, Result,
    StateCell, StateWatch, SubscribeRequest, Subscription, SubscriptionKind, SubscriptionSender,
};

/// The interval a change-of-state subscription polls at: upstream's default polling
/// interval of `PollingSubscriptionConnectionBase` (1000 ms).
const CHANGE_OF_STATE_INTERVAL: Duration = Duration::from_millis(1000);

/// A driver's read path on the state it shares with its subscriptions.
type ReadPath<L> = for<'a> fn(&'a L, ReadRequest) -> OpFuture<'a, ReadResponse>;

/// Tells the polling subscriptions of a connection that the connection is gone.
///
/// The state a driver shares with its subscriptions owns one; when that state is dropped
/// (the last handle went away without `close`), the lifeline reports `Closed` to every
/// producer, so a consumer waiting for a tick hears the end at once instead of at the
/// next tick.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct Lifeline {
    cell: StateCell,
}

impl Lifeline {
    /// A lifeline of a connection that is still there.
    pub(crate) fn new() -> Self {
        let (cell, _watch) = StateCell::new(ConnectionState::Connected);
        Self { cell }
    }
}

impl Drop for Lifeline {
    fn drop(&mut self) {
        self.cell.set(ConnectionState::Closed);
    }
}

/// Whether `clock` really waits: a sleep until one `interval` from now stays pending on its
/// first poll, or resolves only once the clock has reached it. The probe is dropped at once,
/// which a [`Clock`] cancels with no side effect.
fn sleeps(clock: &dyn Clock, interval: Duration) -> bool {
    clock.now().checked_add(interval).is_none_or(|target| {
        let mut probe = clock.sleep_until(target);
        let pending = probe
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending();
        drop(probe);
        pending || clock.now() >= target
    })
}

/// A polling subscription that passed every check that needs no I/O.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct Plan {
    /// What every tick reads: the request's tags, in order, with no deadline or token, so
    /// each read gets the connection's default deadline.
    read: ReadRequest,
    interval: Duration,
    /// Change-of-state: deliver a tag's outcome only when it differs from the last one.
    changes_only: bool,
    queue: QueuePolicy,
    limits: ResourceLimits,
}

impl Plan {
    /// Plan `request` for a connection with `limits` that polls on `clock`.
    ///
    /// A cyclic subscription polls at its own interval and delivers every reading; a
    /// change-of-state one polls at upstream's default of 1000 ms and delivers a tag's first
    /// outcome and then each change.
    ///
    /// # Errors
    /// [`ErrorKind::Unsupported`] for [`Operation::Subscribe`] when the request is an event
    /// subscription, which only a device can raise, or a change-of-state subscription on a
    /// clock whose sleeps resolve at once (the simulator's `RealtimeClock`): with no change
    /// to deliver, its producer would read again at every poll and wake itself without end.
    /// Then what [`ReadRequestBuilder::build`] refuses for the tags under `limits`.
    pub(crate) fn new(
        request: &SubscribeRequest,
        limits: ResourceLimits,
        clock: &dyn Clock,
    ) -> Result<Self> {
        let (interval, changes_only) = match request.kind() {
            SubscriptionKind::Cyclic(interval) => (interval, false),
            SubscriptionKind::ChangeOfState => (CHANGE_OF_STATE_INTERVAL, true),
            SubscriptionKind::Event => {
                return Err(PlcError::new(
                    ErrorKind::Unsupported(Operation::Subscribe),
                    "an event subscription needs the device's own mechanism; polling cannot \
                     deliver it",
                ));
            }
        };
        if changes_only && !sleeps(clock, interval) {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Subscribe),
                "a change-of-state subscription needs a clock that sleeps; this connection's \
                 clock resolves every sleep at once, so polling for a change would never \
                 pause (inject a clock that sleeps, such as a runtime's or the testkit's \
                 FakeClock)",
            ));
        }
        let read = request
            .items()
            .iter()
            .cloned()
            .fold(ReadRequest::builder(limits), ReadRequestBuilder::tag)
            .build()?;
        Ok(Self {
            read,
            interval,
            changes_only,
            queue: request.queue(),
            limits,
        })
    }

    /// The read every tick performs, for the driver's address checks.
    pub(crate) const fn read(&self) -> &ReadRequest {
        &self.read
    }

    /// The subscription, driving a producer that reads `link` through `read` on `clock`.
    ///
    /// The producer holds `link` weakly, so it never keeps the connection alive, and ends
    /// when the connection is gone or `watch` reports it closing, closed or failed.
    /// `lifeline` is the one `link` owns: when `link` is dropped, it ends a wait for the next
    /// tick at once.
    pub(crate) fn start<L: Send + Sync + 'static>(
        self,
        clock: Arc<dyn Clock>,
        watch: StateWatch,
        lifeline: &Lifeline,
        link: Weak<L>,
        read: ReadPath<L>,
    ) -> Subscription {
        let (sender, subscription) =
            SubscriptionSender::polling(self.interval, self.queue, self.limits);
        let gate = Gate::new();
        let producer = Producer {
            plan: self,
            sender,
            gate: Arc::clone(&gate),
            suspended: AtomicBool::new(false),
            clock,
            watch,
            gone: lifeline.cell.watch(),
            link,
            read,
        };
        subscription.driven_by(
            Box::pin(async move {
                producer.run().await.unwrap_or_default();
            }),
            gate,
        )
    }
}

/// The last outcome a change-of-state subscription delivered for a tag.
#[derive(PartialEq)]
enum Seen {
    Value(PlcValue),
    Failure(ErrorKind),
}

/// Whether `outcome` differs from `last`, remembering it when it does. Values compare
/// with `PlcValue`'s equality, as upstream compares with `Objects.equals`; failures compare
/// by `ErrorKind`, so a tag that keeps failing the same way reports it once and a change
/// of cause is delivered.
fn changed(last: &mut Option<Seen>, outcome: &std::result::Result<PlcValue, ItemError>) -> bool {
    let now = match outcome {
        Ok(value) => Seen::Value(value.clone()),
        Err(error) => Seen::Failure(error.kind()),
    };
    if last.as_ref() == Some(&now) {
        false
    } else {
        *last = Some(now);
        true
    }
}

/// The connection will not serve another read: the subscription ends.
const fn ended(state: ConnectionState) -> bool {
    matches!(
        state,
        ConnectionState::Closing | ConnectionState::Closed | ConnectionState::Failed(_)
    )
}

/// How many ticks of `interval` lie after `from`, up to and including `now`, and how far
/// `now` is past the last of them (past `from` when there is none).
fn ticks(from: Instant, now: Instant, interval: Duration) -> (u64, Duration) {
    let elapsed = now.saturating_duration_since(from).as_nanos();
    let step = interval.as_nanos();
    let whole = elapsed.checked_div(step).unwrap_or(0);
    let rest = elapsed.checked_rem(step).unwrap_or(0);
    (
        u64::try_from(whole).unwrap_or(u64::MAX),
        Duration::from_nanos(u64::try_from(rest).unwrap_or(u64::MAX)),
    )
}

/// The producer future of one subscription.
struct Producer<L> {
    plan: Plan,
    sender: SubscriptionSender,
    /// Tells the producer that the `recv` driving its read was dropped.
    gate: Arc<Gate>,
    /// Whether the current tick returned `Pending` at least once, so its poller had control.
    /// Only the producer's own poll reads and writes it (an atomic only so the future stays
    /// `Send`), so `Relaxed` suffices.
    suspended: AtomicBool,
    clock: Arc<dyn Clock>,
    watch: StateWatch,
    /// The lifeline of `link`: `Closed` once the connection was dropped.
    gone: StateWatch,
    link: Weak<L>,
    read: ReadPath<L>,
}

/// Return `Pending` once, waking the task, so that whoever polls gets control back. Only
/// for a poller that ran a whole tick without ever giving it back; a `Pending` that waits
/// on nothing would also make the testkit's executor jump the fake clock ahead.
async fn yield_now() {
    let mut yielded = false;
    poll_fn(|cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await;
}

impl<L: Send + Sync> Producer<L> {
    /// Sample at every tick until the connection ends.
    ///
    /// Ticks lie on a grid of the interval from the start. A tick that came due while the
    /// consumer was not waiting is sampled late, once, and the earlier ones are skipped; the
    /// ticks that came due while a read ran are skipped and the next read waits for the
    /// next tick, so a stall never ends in a burst. A read the connection refused because
    /// its single in-flight slot was taken (`Backpressure`) or because the link is being
    /// re-established (`Disconnected` while the state is not terminal) skips its tick; any
    /// other failure of the whole read becomes an item error for every tag. A read in
    /// flight when the `recv` that drives it is dropped is cancelled and its tick skipped,
    /// so no read outlives its consumer's wait. A tick that never returned `Pending` (its
    /// wait and its read both resolved at once) ends with one yield, so a clock whose sleeps
    /// resolve at once (the simulator's `RealtimeClock`) gives one sample per poll instead
    /// of sampling without end; a cyclic tick always queues its sample, so each `recv` then
    /// resolves after one read, and `Plan::new` refuses change-of-state on such a clock.
    ///
    /// # Errors
    /// [`ErrorKind::Cancelled`] once the subscription is gone, which ends the producer.
    async fn run(self) -> Result<()> {
        let interval = self.plan.interval;
        let mut seen: Vec<Option<Seen>> = self.plan.read.items().iter().map(|_| None).collect();
        let mut due = self.clock.now();
        while self.parked(self.wait(due)).await {
            let now = self.clock.now();
            let (late, past) = ticks(due, now, interval);
            self.sender.count_skipped(late);
            let taken = now.checked_sub(past).unwrap_or(due);
            let Some(link) = self.link.upgrade() else {
                return Ok(());
            };
            let outcome = self.parked(self.sample(&link)).await;
            drop(link);
            let items = match outcome {
                Some(Ok(response)) => response.into_items(),
                Some(Err(_)) if ended(self.watch.current()) => return Ok(()),
                Some(Err(error))
                    if matches!(
                        error.kind(),
                        ErrorKind::Backpressure | ErrorKind::Disconnected
                    ) =>
                {
                    self.sender.count_skipped(1);
                    Vec::new()
                }
                // The consumer's `recv` was dropped mid-read: the tick is skipped.
                None => {
                    self.sender.count_skipped(1);
                    // Stop at the yield below, whatever the read did before it was dropped.
                    self.suspended.store(false, Ordering::Relaxed);
                    Vec::new()
                }
                Some(Err(error)) => self.failed(&error),
            };
            for (item, last) in items.into_iter().zip(seen.iter_mut()) {
                if !self.plan.changes_only || changed(last, item.outcome()) {
                    self.parked(self.sender.send(item)).await?;
                }
            }
            let now = self.clock.now();
            let (overrun, past) = ticks(taken, now, interval);
            self.sender.count_skipped(overrun);
            // An interval that reaches past the end of time has no next tick.
            let Some(next) = now.checked_add(interval.saturating_sub(past)) else {
                return Ok(());
            };
            due = next;
            if !self.suspended.swap(false, Ordering::Relaxed) {
                yield_now().await;
            }
        }
        Ok(())
    }

    /// Await `step`, noting whether it returned `Pending`.
    async fn parked<T>(&self, step: impl Future<Output = T>) -> T {
        let mut step = pin!(step);
        poll_fn(|cx| {
            let polled = step.as_mut().poll(cx);
            if polled.is_pending() {
                self.suspended.store(true, Ordering::Relaxed);
            }
            polled
        })
        .await
    }

    /// One tick's read, or `None` when the `recv` driving it was dropped meanwhile: the
    /// read is dropped with its in-flight slot, so a user's read is never refused for it.
    async fn sample(&self, link: &L) -> Option<Result<ReadResponse>> {
        let mut read = (self.read)(link, self.plan.read.clone());
        self.gate.begin();
        let outcome = poll_fn(|cx| {
            if self.gate.abandoned() {
                return Poll::Ready(None);
            }
            read.as_mut().poll(cx).map(Some)
        })
        .await;
        self.gate.end();
        outcome
    }

    /// Wait for `due` on the clock; false once the connection ended or was dropped.
    async fn wait(&self, due: Instant) -> bool {
        let mut change = self.watch.changed();
        let mut dropped = self.gone.changed();
        let mut sleep = (self.clock.now() < due).then(|| self.clock.sleep_until(due));
        poll_fn(|cx| {
            while change.as_mut().poll(cx).is_ready() {
                change = self.watch.changed();
            }
            while dropped.as_mut().poll(cx).is_ready() {
                dropped = self.gone.changed();
            }
            if ended(self.watch.current()) || ended(self.gone.current()) {
                return Poll::Ready(false);
            }
            // `Clock::sleep_until` never fails for a correct clock (its contract), and a
            // broken one that did could only end the wait early, so the result is not read.
            sleep.as_mut().map_or(Poll::Ready(true), |sleep| {
                sleep.as_mut().poll(cx).map(|_slept| true)
            })
        })
        .await
    }

    /// The item error every tag gets when the whole read failed with `error`.
    fn failed(&self, error: &PlcError) -> Vec<ReadItem> {
        let now = self.clock.now();
        self.plan
            .read
            .items()
            .iter()
            .map(|tag| {
                let copy = PlcError::new(error.kind(), error.to_string())
                    .with_diagnostics(error.diagnostics().clone());
                ReadItem::new(tag.id().clone(), Err(ItemError::new(copy)), now)
            })
            .collect()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests {
    use std::collections::VecDeque;
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::SystemTime;

    use super::*;
    use crate::testkit::{
        CountingWaker, FakeClock, FixtureAddress, block_on_with, poll_once, poll_once_with,
    };
    use crate::{LossCounters, SubscriptionMode, Tag, TagId};

    const TICK: Duration = Duration::from_millis(250);

    /// What the scripted read path answers next.
    enum Reply {
        Value(u16),
        /// The value, after this long on the clock.
        Slow(Duration, u16),
        /// The whole read fails with this kind.
        Fail(ErrorKind),
        /// The connection closes during the read, which fails.
        Close,
        /// The read is pending once, then panics: a broken clock, layer or driver.
        Panic,
    }

    /// A clock whose sleeps resolve at once, like the simulator's `RealtimeClock`.
    struct NeverSleeps;

    impl Clock for NeverSleeps {
        fn now(&self) -> Instant {
            Instant::now()
        }

        fn wall_now(&self) -> SystemTime {
            SystemTime::now()
        }

        fn sleep_until(&self, _deadline: Instant) -> OpFuture<'_, ()> {
            Box::pin(std::future::ready(Ok(())))
        }
    }

    /// A read path that answers from a script, on the fake clock, with a state cell.
    struct Device {
        clock: Arc<FakeClock>,
        state: StateCell,
        replies: Mutex<VecDeque<Reply>>,
        reads: AtomicU32,
        /// Reads begun and not yet finished or dropped: the connection's in-flight slot.
        in_flight: AtomicU32,
        lifeline: Lifeline,
    }

    /// Releases the slot when the read finishes or is dropped.
    struct Slot<'a>(&'a AtomicU32);

    impl Drop for Slot<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::Relaxed);
        }
    }

    impl Device {
        fn new(replies: Vec<Reply>) -> Arc<Self> {
            let (state, _watch) = StateCell::new(ConnectionState::Connected);
            Arc::new(Self {
                clock: Arc::new(FakeClock::default()),
                state,
                replies: Mutex::new(replies.into()),
                reads: AtomicU32::new(0),
                in_flight: AtomicU32::new(0),
                lifeline: Lifeline::new(),
            })
        }

        fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
            Box::pin(async move {
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.in_flight.fetch_add(1, Ordering::Relaxed);
                let _slot = Slot(&self.in_flight);
                let next = self.replies.lock().expect("script").pop_front();
                let started = self.clock.now();
                let value = match next.unwrap_or(Reply::Value(0)) {
                    Reply::Value(value) => value,
                    Reply::Slow(pause, value) => {
                        let until = started.checked_add(pause).expect("a short pause");
                        self.clock.sleep_until(until).await?;
                        value
                    }
                    Reply::Fail(kind) => return Err(PlcError::new(kind, "scripted failure")),
                    Reply::Close => {
                        self.state.set(ConnectionState::Closing);
                        return Err(PlcError::new(ErrorKind::Disconnected, "closed"));
                    }
                    Reply::Panic => {
                        let mut polled = false;
                        poll_fn(|_| {
                            assert!(!polled, "a broken read panics on its second poll");
                            polled = true;
                            Poll::Pending
                        })
                        .await
                    }
                };
                let now = self.clock.now();
                request
                    .items()
                    .iter()
                    .fold(
                        ReadResponse::builder(&request, started, now),
                        |built, tag| {
                            built.item(ReadItem::new(
                                tag.id().clone(),
                                Ok(PlcValue::U16(value)),
                                now,
                            ))
                        },
                    )
                    .build()
            })
        }

        fn reads(&self) -> u32 {
            self.reads.load(Ordering::Relaxed)
        }

        fn in_flight(&self) -> u32 {
            self.in_flight.load(Ordering::Relaxed)
        }

        fn subscribe(self: &Arc<Self>, kind: SubscriptionKind, tags: &[&str]) -> Subscription {
            let limits = ResourceLimits::builder().build().expect("limits");
            self.subscribe_queued(kind, tags, QueuePolicy::Backpressure, limits)
        }

        fn subscribe_queued(
            self: &Arc<Self>,
            kind: SubscriptionKind,
            tags: &[&str],
            policy: QueuePolicy,
            limits: ResourceLimits,
        ) -> Subscription {
            let request = tags
                .iter()
                .fold(
                    SubscribeRequest::builder(limits, kind, policy),
                    |built, id| {
                        built.tag(Tag::new(
                            TagId::new(id).expect("id"),
                            Arc::new(FixtureAddress::new(id, None)),
                        ))
                    },
                )
                .build()
                .expect("request");
            Plan::new(&request, limits, &*self.clock)
                .expect("plan")
                .start(
                    Arc::<FakeClock>::clone(&self.clock),
                    self.state.watch(),
                    &self.lifeline,
                    Arc::downgrade(self),
                    Self::read,
                )
        }

        fn since(&self, start: Instant) -> Duration {
            self.clock.now().saturating_duration_since(start)
        }
    }

    /// The next event: its tag, its value (`None` for an item error) and its sampling time.
    fn next(device: &Device, subscription: &mut Subscription) -> (String, Option<u16>, Instant) {
        let event = block_on_with(&device.clock, subscription.recv())
            .expect("recv never fails")
            .expect("an event");
        let value = event
            .outcome()
            .as_ref()
            .ok()
            .and_then(PlcValue::as_u64)
            .and_then(|wide| u16::try_from(wide).ok());
        (event.tag().as_str().to_owned(), value, event.sampled_at())
    }

    fn ended(device: &Device, subscription: &mut Subscription) -> bool {
        matches!(block_on_with(&device.clock, subscription.recv()), Ok(None))
    }

    // covers: SPEC 'Connection and resource behavior' / polling interval disclosed
    #[test]
    fn a_plan_takes_the_interval_of_its_kind_and_refuses_an_event() {
        let limits = ResourceLimits::builder().build().expect("limits");
        let tag = Tag::new(
            TagId::new("a").expect("id"),
            Arc::new(FixtureAddress::new("a", None)),
        );
        let request = |kind| {
            SubscribeRequest::builder(limits, kind, QueuePolicy::DropOldest)
                .tag(tag.clone())
                .build()
                .expect("request")
        };
        let clock = FakeClock::default();
        let cyclic =
            Plan::new(&request(SubscriptionKind::Cyclic(TICK)), limits, &clock).expect("plan");
        assert_eq!(
            (cyclic.interval, cyclic.changes_only),
            (TICK, false),
            "cyclic"
        );
        assert_eq!(cyclic.read().items().len(), 1, "the tick reads the tag");
        let changes =
            Plan::new(&request(SubscriptionKind::ChangeOfState), limits, &clock).expect("plan");
        assert_eq!(
            (changes.interval, changes.changes_only),
            (Duration::from_secs(1), true),
            "change-of-state polls at upstream's default"
        );
        let refused = Plan::new(&request(SubscriptionKind::Event), limits, &clock).err();
        assert_eq!(
            refused.map(|error| error.kind()),
            Some(ErrorKind::Unsupported(Operation::Subscribe)),
            "an event subscription cannot be polled"
        );
        let wide = ResourceLimits::builder()
            .max_items_per_request(2)
            .build()
            .expect("limits");
        let narrow = ResourceLimits::builder()
            .max_items_per_request(1)
            .build()
            .expect("limits");
        let two = SubscribeRequest::builder(
            wide,
            SubscriptionKind::Cyclic(TICK),
            QueuePolicy::DropOldest,
        )
        .tag(tag.clone())
        .tag(Tag::new(
            TagId::new("b").expect("id"),
            Arc::new(FixtureAddress::new("b", None)),
        ))
        .build()
        .expect("request");
        assert_eq!(
            Plan::new(&two, narrow, &clock)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::ResourceLimit),
            "the connection's own limits apply"
        );
    }

    #[test]
    fn a_cyclic_subscription_delivers_every_reading_one_tick_apart() {
        let device = Device::new(vec![Reply::Value(1), Reply::Value(1), Reply::Value(2)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        assert_eq!(subscription.mode(), SubscriptionMode::Polling, "mode");
        assert_eq!(subscription.interval(), Some(TICK), "interval");
        let start = device.clock.now();
        for (tick, value) in [(0, 1), (1, 1), (2, 2)] {
            let (tag, got, sampled) = next(&device, &mut subscription);
            assert_eq!((tag.as_str(), got), ("a", Some(value)), "tick {tick}");
            assert_eq!(
                sampled.saturating_duration_since(start),
                TICK * tick,
                "tick {tick} is on the grid, the first at once"
            );
        }
        assert_eq!(device.reads(), 3, "one read per tick");
        assert_eq!(subscription.loss(), LossCounters::default(), "nothing lost");
        assert!(
            format!("{subscription:?}").contains("polling: true"),
            "{subscription:?}"
        );
    }

    // covers: SPEC 'Public API contract' / change-of-state delivers only changes
    #[test]
    fn change_of_state_delivers_the_first_outcome_each_change_and_a_failure_once() {
        let device = Device::new(vec![
            Reply::Value(5),
            Reply::Value(5),
            Reply::Value(7),
            Reply::Fail(ErrorKind::Timeout),
            Reply::Fail(ErrorKind::Timeout),
            Reply::Value(7),
        ]);
        let mut subscription = device.subscribe(SubscriptionKind::ChangeOfState, &["a", "b"]);
        let start = device.clock.now();
        let second = Duration::from_secs(1);
        for (tag, value, at) in [
            ("a", Some(5), 0),
            ("b", Some(5), 0),
            ("a", Some(7), 2),
            ("b", Some(7), 2),
            ("a", None, 3),
            ("b", None, 3),
            ("a", Some(7), 5),
            ("b", Some(7), 5),
        ] {
            let (got, outcome, sampled) = next(&device, &mut subscription);
            assert_eq!((got.as_str(), outcome), (tag, value), "at {at} s");
            assert_eq!(
                sampled.saturating_duration_since(start),
                second * at,
                "{tag} at {at} s"
            );
        }
        assert_eq!(device.reads(), 6, "a read every second");
    }

    // covers: SPEC 'Public API contract' / change-of-state delivers only changes
    #[test]
    fn change_of_state_delivers_a_failure_again_when_its_kind_changes() {
        let device = Device::new(vec![
            Reply::Fail(ErrorKind::Timeout),
            Reply::Fail(ErrorKind::Timeout),
            Reply::Fail(ErrorKind::Internal),
            Reply::Value(7),
        ]);
        let mut subscription = device.subscribe(SubscriptionKind::ChangeOfState, &["a"]);
        let start = device.clock.now();
        let second = Duration::from_secs(1);
        for (value, at) in [(None, 0), (None, 2), (Some(7), 3)] {
            let (_, got, sampled) = next(&device, &mut subscription);
            assert_eq!(got, value, "at {at} s");
            assert_eq!(sampled.saturating_duration_since(start), second * at);
        }
        assert_eq!(device.reads(), 4, "a read every second");
    }

    // covers: SPEC 'Connection and resource behavior' / skip missed ticks, no burst after a stall
    #[test]
    fn a_slow_read_skips_the_ticks_it_overran_and_the_next_read_waits_for_its_tick() {
        let device = Device::new(vec![
            Reply::Slow(TICK * 5 / 2, 1),
            Reply::Slow(TICK, 2),
            Reply::Value(3),
        ]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let start = device.clock.now();
        let (_, value, _) = next(&device, &mut subscription);
        assert_eq!(value, Some(1), "the slow reading");
        assert_eq!(
            device.since(start),
            TICK * 5 / 2,
            "it took two and a half ticks"
        );
        assert_eq!(subscription.loss().skipped, 2, "the two ticks it overran");
        let (_, value, _) = next(&device, &mut subscription);
        assert_eq!(value, Some(2), "the next reading");
        assert_eq!(
            device.since(start),
            TICK * 4,
            "it started at the next tick (3) and took one interval"
        );
        assert_eq!(
            subscription.loss().skipped,
            3,
            "a read that takes exactly an interval overran the tick at its end"
        );
        let (_, value, _) = next(&device, &mut subscription);
        assert_eq!(value, Some(3), "the reading after it");
        assert_eq!(device.since(start), TICK * 5, "at the next tick");
        assert_eq!(device.reads(), 3, "no read was made up");
    }

    #[test]
    fn a_consumer_that_comes_late_gets_one_sample_and_the_missed_ticks_are_counted() {
        let device = Device::new(vec![Reply::Value(1), Reply::Value(2), Reply::Value(3)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let start = device.clock.now();
        next(&device, &mut subscription);
        device.clock.advance(TICK * 7 / 2);
        let (_, value, sampled) = next(&device, &mut subscription);
        assert_eq!(value, Some(2), "one sample");
        assert_eq!(
            sampled.saturating_duration_since(start),
            TICK * 7 / 2,
            "taken at once"
        );
        assert_eq!(subscription.loss().skipped, 2, "ticks 1 and 2 were missed");
        let (_, value, sampled) = next(&device, &mut subscription);
        assert_eq!(value, Some(3), "the next sample");
        assert_eq!(
            sampled.saturating_duration_since(start),
            TICK * 4,
            "on the grid again"
        );
    }

    // covers: SPEC mandatory-failure-cases / F4 queue full (single in-flight slot), F5 reconnect
    #[test]
    fn a_read_refused_for_the_busy_slot_or_a_link_being_reopened_skips_its_tick() {
        let device = Device::new(vec![
            Reply::Fail(ErrorKind::Backpressure),
            Reply::Fail(ErrorKind::Disconnected),
            Reply::Value(3),
        ]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let start = device.clock.now();
        let (_, value, sampled) = next(&device, &mut subscription);
        assert_eq!(value, Some(3), "the first reading that worked");
        assert_eq!(
            sampled.saturating_duration_since(start),
            TICK * 2,
            "two ticks were skipped, not delivered as errors"
        );
        assert_eq!(subscription.loss().skipped, 2, "and counted");
    }

    // covers: SPEC mandatory-failure-cases / F5 explicit shutdown ends the subscription
    #[test]
    fn the_end_of_the_connection_ends_the_subscription_after_the_queued_events() {
        let device = Device::new(vec![Reply::Value(1)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a", "b"]);
        assert_eq!(next(&device, &mut subscription).0, "a", "the first event");
        device.state.set(ConnectionState::Closed);
        assert_eq!(next(&device, &mut subscription).0, "b", "the queued event");
        assert!(ended(&device, &mut subscription), "then the end");
        assert!(ended(&device, &mut subscription), "for good");
        assert_eq!(device.reads(), 1, "no read after the close");

        // Waiting for a tick: a reconnect goes on waiting, the close ends the wait.
        let device = Device::new(vec![Reply::Value(1)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        next(&device, &mut subscription);
        let mut waiting = subscription.recv();
        assert!(poll_once(&mut waiting).is_pending(), "waiting for tick 1");
        device.state.set(ConnectionState::Reconnecting {
            attempt: 1,
            next_at: device.clock.now(),
        });
        assert!(
            poll_once(&mut waiting).is_pending(),
            "a reconnect does not end it"
        );
        device.state.set(ConnectionState::Closing);
        assert!(
            matches!(poll_once(&mut waiting), Poll::Ready(Ok(None))),
            "closing ends it without waiting for the tick"
        );
        assert_eq!(device.reads(), 1, "no read after the close");

        // A read that fails because the connection closed is no error event.
        let device = Device::new(vec![Reply::Close]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        assert!(ended(&device, &mut subscription), "the subscription ends");
    }

    #[test]
    fn a_dropped_connection_or_an_endless_interval_ends_the_subscription() {
        let device = Device::new(vec![Reply::Value(1)]);
        let clock = Arc::clone(&device.clock);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        next(&device, &mut subscription);
        assert_eq!(
            Arc::weak_count(&device),
            1,
            "the producer holds the link weakly"
        );
        let wakes = CountingWaker::new();
        let mut waiting = subscription.recv();
        assert!(
            poll_once_with(&mut waiting, &wakes.waker()).is_pending(),
            "waiting for tick 1"
        );
        let before = clock.now();
        drop(device);
        assert_eq!(wakes.count(), 1, "the drop wakes the waiting consumer");
        assert!(
            matches!(poll_once(&mut waiting), Poll::Ready(Ok(None))),
            "and ends the subscription"
        );
        assert_eq!(clock.now(), before, "without waiting for the tick");
        drop(waiting);

        let device = Device::new(vec![Reply::Value(1)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(Duration::MAX), &["a"]);
        assert_eq!(
            next(&device, &mut subscription).1,
            Some(1),
            "the first tick"
        );
        assert!(
            ended(&device, &mut subscription),
            "no tick after the end of time"
        );
    }

    // covers: ADR-0020 addendum S55 / change-of-state needs a clock that sleeps
    #[test]
    fn change_of_state_is_refused_on_a_clock_that_never_sleeps() {
        let limits = ResourceLimits::builder().build().expect("limits");
        let request = |kind| {
            SubscribeRequest::builder(limits, kind, QueuePolicy::DropOldest)
                .tag(Tag::new(
                    TagId::new("a").expect("id"),
                    Arc::new(FixtureAddress::new("a", None)),
                ))
                .build()
                .expect("request")
        };
        let refused = Plan::new(
            &request(SubscriptionKind::ChangeOfState),
            limits,
            &NeverSleeps,
        )
        .err()
        .map(|error| error.kind());
        assert_eq!(
            refused,
            Some(ErrorKind::Unsupported(Operation::Subscribe)),
            "a change-of-state producer would read at every poll without a pause"
        );
        let cyclic = Plan::new(
            &request(SubscriptionKind::Cyclic(TICK)),
            limits,
            &NeverSleeps,
        );
        assert!(
            cyclic.is_ok(),
            "a cyclic tick always delivers, so it is allowed"
        );
        assert!(
            NeverSleeps.wall_now() > SystemTime::UNIX_EPOCH,
            "the stand-in reads the system clock"
        );
        assert!(
            sleeps(&FakeClock::default(), Duration::MAX),
            "an interval past the end of time needs no probe"
        );
    }

    // covers: ADR-0020 addendum S55 / a producer that panicked is never polled again
    #[test]
    fn a_read_that_panics_ends_the_subscription_and_no_drop_panics_again() {
        // The pending recv is dropped while the panic unwinds through its owner.
        let device = Device::new(vec![Reply::Panic]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let mut waiting = subscription.recv();
        assert!(
            poll_once(&mut waiting).is_pending(),
            "the read is in flight"
        );
        let unwound = catch_unwind(AssertUnwindSafe(move || poll_once(&mut waiting).is_ready()));
        assert!(unwound.is_err(), "the read's panic reaches the caller");
        assert_eq!(device.in_flight(), 0, "the read was dropped with its slot");
        assert!(ended(&device, &mut subscription), "the subscription ended");

        // The recv outlives the caught panic and is dropped afterwards.
        let device = Device::new(vec![Reply::Panic]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let mut waiting = subscription.recv();
        assert!(
            poll_once(&mut waiting).is_pending(),
            "the read is in flight"
        );
        let unwound = catch_unwind(AssertUnwindSafe(|| poll_once(&mut waiting).is_ready()));
        assert!(unwound.is_err(), "the read's panic reaches the caller");
        drop(waiting);
        assert!(ended(&device, &mut subscription), "the subscription ended");
        assert_eq!(
            device.reads(),
            1,
            "the panicked producer was never polled again"
        );

        // A panic elsewhere unwinds through a recv whose read is in flight: the producer is
        // dropped, not polled during the unwinding.
        let device = Device::new(vec![Reply::Slow(TICK, 1)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let mut waiting = subscription.recv();
        assert!(
            poll_once(&mut waiting).is_pending(),
            "the read is in flight"
        );
        let unwound = catch_unwind(AssertUnwindSafe(move || {
            let _held = waiting;
            panic!("the consumer panics while its recv is pending");
        }));
        assert!(unwound.is_err(), "the consumer's panic");
        assert_eq!(device.in_flight(), 0, "the read was dropped with its slot");
        assert!(ended(&device, &mut subscription), "the subscription ended");
    }

    // A producer whose link is gone while its lifeline is not still ends at its next tick.
    #[test]
    fn a_link_gone_while_its_lifeline_lives_ends_the_subscription_at_the_next_tick() {
        let device = Device::new(vec![Reply::Value(1)]);
        let clock = Arc::clone(&device.clock);
        let limits = ResourceLimits::builder().build().expect("limits");
        let request = SubscribeRequest::builder(
            limits,
            SubscriptionKind::Cyclic(TICK),
            QueuePolicy::DropOldest,
        )
        .tag(Tag::new(
            TagId::new("a").expect("id"),
            Arc::new(FixtureAddress::new("a", None)),
        ))
        .build()
        .expect("request");
        let lifeline = Lifeline::new();
        let mut subscription = Plan::new(&request, limits, &*clock).expect("plan").start(
            Arc::<FakeClock>::clone(&clock),
            device.state.watch(),
            &lifeline,
            Arc::downgrade(&device),
            Device::read,
        );
        drop(device);
        assert!(
            matches!(block_on_with(&clock, subscription.recv()), Ok(None)),
            "the first tick finds the link gone"
        );
        drop(lifeline);
    }

    // covers: ADR-0020 addendum S55 / a dropped recv cancels the read it left in flight
    #[test]
    fn dropping_a_recv_mid_read_cancels_the_read_and_frees_the_slot() {
        let device = Device::new(vec![Reply::Slow(TICK * 2, 1), Reply::Value(2)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        let mut waiting = subscription.recv();
        assert!(poll_once(&mut waiting).is_pending(), "the read is running");
        assert_eq!(device.in_flight(), 1, "it holds the slot");
        drop(waiting);
        assert_eq!(device.in_flight(), 0, "the slot is free for a user's read");
        assert_eq!(subscription.loss().skipped, 1, "the tick was skipped");
        assert_eq!(device.reads(), 1, "no other read was started");
        assert_eq!(
            next(&device, &mut subscription).1,
            Some(2),
            "polling goes on"
        );

        // A recv dropped while the poller waits for its tick has no read to cancel.
        let mut waiting = subscription.recv();
        assert!(poll_once(&mut waiting).is_pending(), "waiting for a tick");
        drop(waiting);
        assert_eq!(subscription.loss().skipped, 1, "nothing was cancelled");
        assert_eq!(device.reads(), 2, "and nothing read");
    }

    // covers: ADR-0020 addendum S55 / what a slow consumer observes under each queue policy
    #[test]
    fn a_slow_consumer_meets_the_queue_policy_only_when_a_sample_outgrows_the_queue() {
        let tags = ["a", "b", "c"];
        let cyclic = SubscriptionKind::Cyclic(TICK);
        let capacity = |events| {
            ResourceLimits::builder()
                .max_queued_events(events)
                .build()
                .expect("limits")
        };

        // Dropping: the sample of three overflows a queue of two, the oldest goes.
        let device = Device::new(vec![Reply::Value(1)]);
        let mut subscription =
            device.subscribe_queued(cyclic, &tags, QueuePolicy::DropOldest, capacity(2));
        assert_eq!(next(&device, &mut subscription).0, "b", "a was dropped");
        assert_eq!(next(&device, &mut subscription).0, "c", "then the rest");
        let loss = subscription.loss();
        assert_eq!(
            (loss.dropped, loss.coalesced, loss.skipped),
            (1, 0, 0),
            "dropped"
        );

        // Backpressure: the producer waits for room, reads nothing meanwhile, and the
        // ticks that came due are skipped.
        let device = Device::new(vec![Reply::Value(1), Reply::Value(2)]);
        let mut subscription =
            device.subscribe_queued(cyclic, &tags, QueuePolicy::Backpressure, capacity(2));
        assert_eq!(next(&device, &mut subscription).0, "a", "the first event");
        assert_eq!(next(&device, &mut subscription).0, "b", "the second");
        device.clock.advance(TICK * 3);
        assert_eq!(next(&device, &mut subscription).0, "c", "the stalled third");
        let loss = subscription.loss();
        assert_eq!(
            (loss.dropped, loss.skipped),
            (0, 3),
            "nothing lost, ticks skipped"
        );
        assert_eq!(device.reads(), 1, "no read while the producer waited");

        // Coalescing: the queue is empty whenever a sample starts, so nothing coalesces; a
        // slow consumer gets the latest sample and the ticks it missed are skipped.
        let device = Device::new(vec![Reply::Value(1), Reply::Value(2)]);
        let mut subscription =
            device.subscribe_queued(cyclic, &tags, QueuePolicy::CoalesceLatest, capacity(3));
        for id in tags {
            assert_eq!(next(&device, &mut subscription).0, id, "the first sample");
        }
        device.clock.advance(TICK * 5);
        assert_eq!(
            (
                next(&device, &mut subscription).0,
                next(&device, &mut subscription).1
            ),
            ("a".to_owned(), Some(2)),
            "the latest sample"
        );
        let loss = subscription.loss();
        assert_eq!(
            (loss.coalesced, loss.dropped, loss.skipped),
            (0, 0, 4),
            "skipped"
        );
    }

    #[test]
    fn unsubscribing_drops_the_producer_and_stops_the_reads() {
        let device = Device::new(vec![Reply::Value(1)]);
        let mut subscription = device.subscribe(SubscriptionKind::Cyclic(TICK), &["a"]);
        next(&device, &mut subscription);
        assert_eq!(subscription.unsubscribe().skipped, 0, "the final counters");
        assert_eq!(Arc::weak_count(&device), 0, "the producer is gone");
        device.clock.advance(TICK * 4);
        assert_eq!(device.reads(), 1, "no read after unsubscribe");
    }

    #[test]
    fn ticks_count_the_grid_points_after_the_start_up_to_now() {
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
