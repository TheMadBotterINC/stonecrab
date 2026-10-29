//! The event pump: named batches of tags read at intervals and delivered to one consumer
//! (ADR-0031).
//!
//! Each [`TagBatch`] reads its [`ReadRequest`] through a connection, or through a lease
//! from a [`ConnectionCache`] taken for each fetch, on an interval with an initial delay.
//! Nothing is spawned: every started batch is driven while the caller awaits
//! [`EventPump::recv`], on the injected [`Clock`], as a polling subscription's producer is
//! driven by `Subscription::recv`.

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use crate::clock::ticks;
use crate::{
    Clock, Connection, ConnectionCache, ErrorKind, OpFuture, PlcError, ReadRequest, ReadResponse,
    Result,
};

/// Upstream's `DEFAULT_INITIAL_BACKOFF_MS`: the first pause after a failed fetch, one second.
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
/// Upstream's `DEFAULT_MAX_BACKOFF_MS`: the longest pause after failed fetches, one minute.
const MAX_BACKOFF: Duration = Duration::from_secs(60);

/// Where a batch reads.
enum Source<'a> {
    Connection(&'a dyn Connection),
    /// A lease of this connection string, taken for each fetch and given back after it.
    Cache(&'a ConnectionCache, String),
}

/// A named set of tags read together at an interval: one batch of an [`EventPump`].
///
/// A batch reads its [`ReadRequest`] in one [`Connection::read`] per fetch, either on a
/// connection it borrows ([`on_connection`](Self::on_connection)) or on a lease it takes
/// from a [`ConnectionCache`] for each fetch and gives back right after it
/// ([`on_cache`](Self::on_cache)). Its trigger is an interval with an initial delay: once
/// the batch is started, its fetches fall due on a grid that begins `initial_delay` after
/// the start and has one point every `interval`.
///
/// # Compatibility: docs/parity/tools.md#TOOL-EP-READ
/// Upstream's `TagBatch` with a `TimerTrigger`. The tags are a built [`ReadRequest`]
/// instead of name and address strings, and there is no listener: the batch's results
/// come out of [`EventPump::recv`].
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::testkit::FixtureAddress;
/// use stonecrab::{ConnectionCache, DriverRegistry, ReadRequest, ResourceLimits, Tag, TagBatch, TagId};
/// # use std::time::Instant;
/// # use stonecrab::{Clock, OpFuture};
/// # struct Frozen(Instant);
/// # impl Clock for Frozen {
/// #     fn now(&self) -> Instant { self.0 }
/// #     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
/// #     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
/// # }
///
/// let cache = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(Frozen(Instant::now()))).build()?;
/// let tag = Tag::new(TagId::new("level")?, Arc::new(FixtureAddress::new("level", None)));
/// let request = ReadRequest::builder(ResourceLimits::builder().build()?).tag(tag).build()?;
/// let batch = TagBatch::on_cache("tank", &cache, "sim://bench", request, Duration::from_secs(1))?
///     .initial_delay(Duration::from_millis(500));
/// assert_eq!(batch.name(), "tank");
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct TagBatch<'a> {
    name: String,
    source: Source<'a>,
    request: ReadRequest,
    interval: Duration,
    initial_delay: Duration,
}

impl<'a> TagBatch<'a> {
    /// A batch named `name` that reads `request` on `connection` every `interval`, with no
    /// initial delay.
    ///
    /// The request is read as it is at every fetch, so it must not carry a deadline: each
    /// read gets the connection's default deadline (`timeouts.request`). A cancellation
    /// token on it cancels every later fetch.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] for a blank name, a zero interval or a request with a
    /// deadline.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-MULTI
    /// Upstream leases each fetch's connection from a `PlcConnectionFactory`; a borrowed
    /// connection is the case of a factory that always returns the same one.
    ///
    /// # Examples
    /// See [`EventPump`].
    pub fn on_connection(
        name: &str,
        connection: &'a dyn Connection,
        request: ReadRequest,
        interval: Duration,
    ) -> Result<Self> {
        Self::new(name, Source::Connection(connection), request, interval)
    }

    /// A batch named `name` that reads `request` every `interval`, with no initial delay,
    /// on a lease of `connection_string` from `cache`.
    ///
    /// Each fetch waits for its lease ([`ConnectionCache::get`]), reads, and gives the lease
    /// back, so the connection is shared with the cache's other users between fetches. A
    /// fetch whose lease cannot be had fails like a read that failed. The request rules are
    /// those of [`on_connection`](Self::on_connection).
    ///
    /// # Errors
    /// See [`on_connection`](Self::on_connection).
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-MULTI
    /// Upstream's `withConnectionFactory` and `withConnectionString` with the connection
    /// cache as the factory.
    ///
    /// # Examples
    /// See [`TagBatch`].
    pub fn on_cache(
        name: &str,
        cache: &'a ConnectionCache,
        connection_string: &str,
        request: ReadRequest,
        interval: Duration,
    ) -> Result<Self> {
        let source = Source::Cache(cache, connection_string.to_owned());
        Self::new(name, source, request, interval)
    }

    fn new(
        name: &str,
        source: Source<'a>,
        request: ReadRequest,
        interval: Duration,
    ) -> Result<Self> {
        if name.trim().is_empty() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "a batch needs a name that is not blank",
            ));
        }
        if interval.is_zero() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "a batch's interval must be greater than zero",
            ));
        }
        if request.deadline().is_some() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "a batch reads its request again at every fetch, so the request must not carry \
                 a deadline; each read takes the connection's default (timeouts.request)",
            ));
        }
        Ok(Self {
            name: name.to_owned(),
            source,
            request,
            interval,
            initial_delay: Duration::ZERO,
        })
    }

    /// The same batch, whose first fetch falls due `delay` after it is started instead of
    /// at once.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-TIMER
    /// Upstream's `TimerTrigger(interval, initialDelay, unit)`.
    ///
    /// # Examples
    /// See [`TagBatch`].
    #[must_use]
    pub const fn initial_delay(mut self, delay: Duration) -> Self {
        self.initial_delay = delay;
        self
    }

    /// The batch's name, unique within its pump.
    ///
    /// # Examples
    /// See [`TagBatch`].
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// One fetch: a read of the request, through a lease taken for it when the batch reads
    /// from a cache.
    fn fetch(&self) -> OpFuture<'a, ReadResponse> {
        let request = self.request.clone();
        match &self.source {
            Source::Connection(connection) => {
                let connection: &'a dyn Connection = *connection;
                connection.read(request)
            }
            Source::Cache(cache, connection_string) => {
                let cache: &'a ConnectionCache = cache;
                let connection_string = connection_string.clone();
                Box::pin(async move {
                    let lease = cache.get(&connection_string).await?;
                    lease.read(request).await
                })
            }
        }
    }
}

impl fmt::Debug for TagBatch<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("TagBatch");
        debug.field("name", &self.name);
        match &self.source {
            Source::Connection(_) => debug.field("source", &"connection"),
            Source::Cache(_, connection_string) => debug.field("cache", connection_string),
        };
        debug
            .field("tags", &self.request.items().len())
            .field("interval", &self.interval)
            .field("initial_delay", &self.initial_delay)
            .finish_non_exhaustive()
    }
}

/// The result of one fetch of a batch, delivered by [`EventPump::recv`].
///
/// # Compatibility: docs/parity/tools.md#TOOL-EP-READ
/// Upstream calls `onTagsFetched(batch, response)` or `onError(batch, error)` on the batch's
/// listener, and `onFetchSkipped` for each skipped trigger; here the outcome and the count
/// of skipped ticks arrive together.
///
/// # Examples
/// See [`EventPump`].
#[derive(Debug)]
#[non_exhaustive]
pub struct BatchEvent {
    /// The name of the batch that was fetched.
    pub batch: String,
    /// The response of the batch's read, or why the fetch failed as a whole: the read's
    /// error, or for a batch on a cache the error of [`ConnectionCache::get`]. A response
    /// may still carry item errors.
    pub outcome: Result<ReadResponse>,
    /// The ticks of this batch that passed without a fetch since its previous event: a
    /// tick that fell due while a fetch ran or while the consumer was away, or during the
    /// pause after a failed fetch. Stopping a batch resets the count.
    pub skipped: u64,
}

/// What a started batch is doing.
struct Running<'a> {
    /// The next tick; `None` once it would lie past the end of the clock's time.
    due: Option<Instant>,
    /// The fetch in flight and the tick it serves.
    fetch: Option<(OpFuture<'a, ReadResponse>, Instant)>,
    /// Ticks skipped since the batch's last event.
    skipped: u64,
    /// The current pause after failed fetches: zero after a success.
    backoff: Duration,
    /// No fetch starts before this instant (the end of the pause), if set.
    not_before: Option<Instant>,
}

/// A batch of a pump, stopped or started.
struct Batch<'a> {
    spec: TagBatch<'a>,
    running: Option<Running<'a>>,
}

impl Batch<'_> {
    /// Start the batch at `now`: its first tick lies `initial_delay` later.
    fn start(&mut self, now: Instant) {
        self.running = Some(Running {
            due: now.checked_add(self.spec.initial_delay),
            fetch: None,
            skipped: 0,
            backoff: Duration::ZERO,
            not_before: None,
        });
    }

    /// Start the fetch of the latest tick at or before `now`, if one fell due and no fetch
    /// runs; the earlier ticks are skipped, and so is the latest one during a pause.
    fn fetch_if_due(&mut self, now: Instant) {
        let Some(running) = &mut self.running else {
            return;
        };
        let Some(due) = running.due.filter(|due| *due <= now) else {
            return;
        };
        if running.fetch.is_some() {
            return;
        }
        let interval = self.spec.interval;
        let (late, past) = ticks(due, now, interval);
        let tick = now.checked_sub(past).unwrap_or(due);
        running.skipped = running.skipped.saturating_add(late);
        running.due = tick.checked_add(interval);
        if running.not_before.is_some_and(|until| now < until) {
            running.skipped = running.skipped.saturating_add(1);
        } else {
            running.fetch = Some((self.spec.fetch(), tick));
        }
    }

    /// When this batch next needs the clock: its next tick, unless a fetch runs (the fetch
    /// wakes the pump instead).
    fn waiting_until(&self) -> Option<Instant> {
        self.running
            .as_ref()
            .filter(|running| running.fetch.is_none())
            .and_then(|running| running.due)
    }

    /// Whether a fetch of this batch is in flight.
    fn fetching(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.fetch.is_some())
    }

    /// Poll the fetch in flight; its event once it has finished at the clock's `now`.
    ///
    /// The ticks that fell due while it ran are skipped and the next fetch waits for the
    /// next tick. A failed fetch starts or doubles the pause (upstream's backoff); a fetch
    /// that succeeded ends it.
    fn poll_fetch(&mut self, cx: &mut Context<'_>, clock: &dyn Clock) -> Option<BatchEvent> {
        let running = self.running.as_mut()?;
        let (fetch, tick) = running.fetch.as_mut()?;
        let Poll::Ready(outcome) = fetch.as_mut().poll(cx) else {
            return None;
        };
        let tick = *tick;
        running.fetch = None;
        let now = clock.now();
        let (overrun, past) = ticks(tick, now, self.spec.interval);
        running.skipped = running.skipped.saturating_add(overrun);
        running.due = now.checked_add(self.spec.interval.saturating_sub(past));
        if outcome.is_ok() {
            running.backoff = Duration::ZERO;
            running.not_before = None;
        } else {
            running.backoff = running
                .backoff
                .saturating_mul(2)
                .clamp(INITIAL_BACKOFF, MAX_BACKOFF);
            running.not_before = now.checked_add(running.backoff);
        }
        Some(BatchEvent {
            batch: self.spec.name.clone(),
            outcome,
            skipped: std::mem::take(&mut running.skipped),
        })
    }
}

/// Named batches of tags, each read at its own interval, delivered to one consumer.
///
/// A pump holds [`TagBatch`]es by name. A batch is added stopped; [`start_batch`] starts
/// one and [`start_all`] every stopped one, [`stop_batch`] and [`stop_all`] stop them, and
/// [`close`] stops and forgets them all. The consumer awaits [`recv`], which runs every
/// started batch while it waits and resolves to the next [`BatchEvent`].
///
/// # Ticks and slow fetches
/// A started batch's ticks lie on a grid: the first `initial_delay` after the start, then
/// one every `interval`. At a tick the batch's request is read once. A tick that falls due
/// while the batch's previous fetch is still running is skipped, and the next fetch waits
/// for the next tick, so a slow device never gets a burst of reads. After a fetch that
/// failed, the batch pauses before it fetches again: one second, doubled after each
/// further failure up to one minute, and ended by a fetch that succeeds. The ticks in the
/// pause are skipped too. Each event counts the ticks its batch skipped since its previous
/// event ([`BatchEvent::skipped`]).
///
/// # The consumer
/// Nothing runs between calls of [`recv`]: no task is spawned. A tick that fell due while
/// the consumer was away is fetched late, once, when it calls again, and the earlier ones
/// are skipped. The queue of events holds at most one per batch, because fetches start only
/// when the queue is empty, so a slow consumer costs skipped ticks, never memory. Batches
/// on different connections run independently: while one batch's read waits on its device,
/// [`recv`] still delivers the other batches' events, and a read still in flight when
/// [`recv`] returns goes on at the next call.
///
/// The clock must be the one the connections sample with, and it must sleep: on a clock
/// whose sleep resolves before its deadline (the simulator's `RealtimeClock`) the pump
/// cannot pace anything and [`recv`] refuses to wait.
///
/// [`start_batch`]: Self::start_batch
/// [`start_all`]: Self::start_all
/// [`stop_batch`]: Self::stop_batch
/// [`stop_all`]: Self::stop_all
/// [`close`]: Self::close
/// [`recv`]: Self::recv
///
/// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
/// Upstream's `EventPump` with timer-triggered `TagBatch`es. Upstream calls each batch's
/// listener from its own threads; here one consumer receives every batch's events. See
/// `docs/adr/0031-event-pump.md` for the differences.
///
/// # Examples
/// A level read every second, from the start.
/// ```
/// # #[cfg(all(feature = "sim", feature = "testkit"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::sim::{SimAddress, SimDevice, SimDriver, SimTag};
/// use stonecrab::testkit::{FakeClock, block_on_with};
/// use stonecrab::{
///     Clock, Connection, DriverRegistry, EventPump, PlcValue, ReadRequest, Tag, TagBatch, TagId,
/// };
///
/// let clock = FakeClock::default();
/// let device = SimDevice::new();
/// device.set("level", SimTag::new(PlcValue::U16(840)))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new().with_clock(Arc::new(clock.clone())).with_device("bench", device)?,
/// ))?;
/// let connection = block_on_with(&clock, registry.connect("sim://bench"))?;
/// let level = Tag::new(TagId::new("level")?, Arc::new(SimAddress::parse("STATE/level:u16")?));
/// let request = ReadRequest::builder(connection.limits()).tag(level).build()?;
///
/// let mut pump = EventPump::new(&clock);
/// pump.add_batch(TagBatch::on_connection("tank", &*connection, request, Duration::from_secs(1))?)?;
/// pump.start_batch("tank")?;
/// let start = clock.now();
/// for second in 0..3 {
///     let event = block_on_with(&clock, pump.recv())?.ok_or("the batch runs")?;
///     assert_eq!((event.batch.as_str(), event.skipped), ("tank", 0));
///     assert_eq!(clock.now() - start, Duration::from_secs(second));
///     let response = event.outcome?;
///     let value = response.items().first().and_then(|item| item.outcome().as_ref().ok());
///     assert_eq!(value, Some(&PlcValue::U16(840)));
/// }
/// pump.close();
/// assert!(block_on_with(&clock, pump.recv())?.is_none());
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sim", feature = "testkit")))]
/// # fn main() {}
/// ```
pub struct EventPump<'a> {
    clock: &'a dyn Clock,
    /// In the order they were added, which orders events that finish at the same poll.
    batches: Vec<Batch<'a>>,
    /// At most one event per batch: fetches start only when it is empty.
    queue: VecDeque<BatchEvent>,
    closed: bool,
}

impl<'a> EventPump<'a> {
    /// An empty pump that reads time from `clock`.
    ///
    /// # Examples
    /// See [`EventPump`].
    #[must_use]
    pub fn new(clock: &'a dyn Clock) -> Self {
        Self {
            clock,
            batches: Vec::new(),
            queue: VecDeque::new(),
            closed: false,
        }
    }

    /// Add `batch`, stopped.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if the pump has a batch of the same name, and
    /// [`ErrorKind::Disconnected`] if it is closed.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `addBatch`.
    ///
    /// # Examples
    /// See [`EventPump`].
    pub fn add_batch(&mut self, batch: TagBatch<'a>) -> Result<()> {
        self.refuse_when_closed()?;
        if self.is_started(&batch.name).is_some() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                format!("the event pump already has a batch named {:?}", batch.name),
            ));
        }
        self.batches.push(Batch {
            spec: batch,
            running: None,
        });
        Ok(())
    }

    /// Stop and remove the batch named `name`, and give it back; `None` if there is none.
    /// A fetch in flight is dropped; an event of the batch already queued is still
    /// delivered.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `removeBatch`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::EventPump;
    /// # use std::time::Instant;
    /// # use stonecrab::{Clock, OpFuture};
    /// # struct Frozen(Instant);
    /// # impl Clock for Frozen {
    /// #     fn now(&self) -> Instant { self.0 }
    /// #     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
    /// #     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
    /// # }
    ///
    /// let clock = Frozen(Instant::now());
    /// let mut pump = EventPump::new(&clock);
    /// assert!(pump.remove_batch("tank").is_none());
    /// ```
    pub fn remove_batch(&mut self, name: &str) -> Option<TagBatch<'a>> {
        let index = self
            .batches
            .iter()
            .position(|batch| batch.spec.name == name)?;
        Some(self.batches.remove(index).spec)
    }

    /// Start the batch named `name`: its first tick lies its initial delay from now.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if there is no such batch or it is already started,
    /// and [`ErrorKind::Disconnected`] if the pump is closed.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `startBatch`, which also refuses a batch that is already started.
    ///
    /// # Examples
    /// See [`EventPump`].
    pub fn start_batch(&mut self, name: &str) -> Result<()> {
        self.refuse_when_closed()?;
        let now = self.clock.now();
        let batch = self.batch(name)?;
        if batch.running.is_some() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                format!("the batch {name:?} is already started"),
            ));
        }
        batch.start(now);
        Ok(())
    }

    /// Stop the batch named `name`; stopping a stopped batch does nothing. A fetch in
    /// flight is dropped; an event of the batch already queued is still delivered.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if there is no such batch.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `stopBatch`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, EventPump};
    /// # use std::time::Instant;
    /// # use stonecrab::{Clock, OpFuture};
    /// # struct Frozen(Instant);
    /// # impl Clock for Frozen {
    /// #     fn now(&self) -> Instant { self.0 }
    /// #     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
    /// #     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
    /// # }
    ///
    /// let clock = Frozen(Instant::now());
    /// let mut pump = EventPump::new(&clock);
    /// let refused = pump.stop_batch("tank").err().map(|error| error.kind());
    /// assert_eq!(refused, Some(ErrorKind::InvalidConfig));
    /// ```
    pub fn stop_batch(&mut self, name: &str) -> Result<()> {
        self.batch(name)?.running = None;
        Ok(())
    }

    /// Start every stopped batch, each with its first tick its initial delay from now.
    ///
    /// # Errors
    /// [`ErrorKind::Disconnected`] if the pump is closed.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `startAll`, which skips the started batches.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::EventPump;
    /// # use std::time::Instant;
    /// # use stonecrab::{Clock, OpFuture};
    /// # struct Frozen(Instant);
    /// # impl Clock for Frozen {
    /// #     fn now(&self) -> Instant { self.0 }
    /// #     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
    /// #     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
    /// # }
    ///
    /// let clock = Frozen(Instant::now());
    /// let mut pump = EventPump::new(&clock);
    /// pump.start_all()?;
    /// pump.stop_all();
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn start_all(&mut self) -> Result<()> {
        self.refuse_when_closed()?;
        let now = self.clock.now();
        for batch in &mut self.batches {
            if batch.running.is_none() {
                batch.start(now);
            }
        }
        Ok(())
    }

    /// Stop every batch, dropping the fetches in flight.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `stopAll`.
    ///
    /// # Examples
    /// See [`start_all`](Self::start_all).
    pub fn stop_all(&mut self) {
        for batch in &mut self.batches {
            batch.running = None;
        }
    }

    /// Whether the batch named `name` is started; `None` if there is no such batch.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `getBatch(id).isStarted()`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::EventPump;
    /// # use std::time::Instant;
    /// # use stonecrab::{Clock, OpFuture};
    /// # struct Frozen(Instant);
    /// # impl Clock for Frozen {
    /// #     fn now(&self) -> Instant { self.0 }
    /// #     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
    /// #     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
    /// # }
    ///
    /// let clock = Frozen(Instant::now());
    /// assert_eq!(EventPump::new(&clock).is_started("tank"), None);
    /// ```
    #[must_use]
    pub fn is_started(&self, name: &str) -> Option<bool> {
        self.batches
            .iter()
            .find(|batch| batch.spec.name == name)
            .map(|batch| batch.running.is_some())
    }

    /// Stop and forget every batch and every queued event, and refuse every later
    /// [`add_batch`](Self::add_batch), [`start_batch`](Self::start_batch) and
    /// [`start_all`](Self::start_all) with [`ErrorKind::Disconnected`]; [`recv`](Self::recv)
    /// then resolves to `None`. Closing a closed pump does nothing. The connections and
    /// the cache the batches read from stay open: the pump only borrows them.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-PUMP
    /// Upstream's `close`, which closes every batch and its trigger.
    ///
    /// # Examples
    /// See [`EventPump`].
    pub fn close(&mut self) {
        self.closed = true;
        self.batches.clear();
        self.queue.clear();
    }

    /// The next event of a started batch, reading and waiting as needed.
    ///
    /// Resolves to `Some(event)` in the order the fetches finished (batches whose fetches
    /// finish at the same poll in the order they were added), and to `None` when no event
    /// is queued, no fetch is in flight and no started batch has a tick left: when every
    /// batch is stopped, or the pump is closed.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] when the clock's sleep resolved before its deadline, so
    /// the clock cannot pace the pump. A failed fetch is not an error of `recv`: it is the
    /// [`BatchEvent::outcome`] of its batch, and the batch goes on.
    ///
    /// # Cancel safety
    /// Cancel-safe (no bytes sent on drop): dropping the future loses no event and abandons
    /// no read. An event leaves the queue only when it is returned, and a read in flight
    /// stays with its batch and goes on at the next call. Between calls such a read holds
    /// its connection's in-flight slot, or its lease from the cache, and its deadline keeps
    /// running; stop the batch to drop it. The deadline source of each read is the
    /// connection's `timeouts.request`, and of a lease the cache's maximum wait time.
    ///
    /// # Wire effects
    /// One read per batch fetch, through the connection's read path and so its in-flight
    /// slot, deadline and retry policy; for a batch on a cache, what
    /// [`ConnectionCache::get`] sends to open or ping the connection first. Waiting touches
    /// no transport.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-EP-TIMER
    /// Upstream's `TimerTrigger` fires on a `java.util.Timer` with a fixed delay and hands
    /// each firing to the batch, which skips it while a fetch is in progress; here the
    /// ticks are evaluated while the consumer waits.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{EventPump, Result};
    ///
    /// async fn failures(pump: &mut EventPump<'_>) -> Result<usize> {
    ///     let mut failed = 0;
    ///     while let Some(event) = pump.recv().await? {
    ///         if event.outcome.is_err() {
    ///             failed += 1;
    ///         }
    ///     }
    ///     Ok(failed)
    /// }
    /// ```
    pub fn recv(&mut self) -> OpFuture<'_, Option<BatchEvent>> {
        Box::pin(async move {
            loop {
                if let Some(event) = self.queue.pop_front() {
                    return Ok(Some(event));
                }
                let now = self.clock.now();
                for batch in &mut self.batches {
                    batch.fetch_if_due(now);
                }
                let wake = self.batches.iter().filter_map(Batch::waiting_until).min();
                if wake.is_none() && !self.batches.iter().any(Batch::fetching) {
                    return Ok(None);
                }
                let clock = self.clock;
                let mut sleep = wake.map(|at| clock.sleep_until(at));
                let (batches, queue) = (&mut self.batches, &mut self.queue);
                let slept = poll_fn(|cx| {
                    queue.extend(
                        batches
                            .iter_mut()
                            .filter_map(|batch| batch.poll_fetch(cx, clock)),
                    );
                    if !queue.is_empty() {
                        return Poll::Ready(false);
                    }
                    // `Clock::sleep_until` never fails for a correct clock (its contract); one
                    // that resolves early is caught below, so the result is not read.
                    sleep.as_mut().map_or(Poll::Pending, |sleep| {
                        sleep.as_mut().poll(cx).map(|_slept| true)
                    })
                })
                .await;
                if slept && wake.is_some_and(|at| clock.now() < at) {
                    return Err(PlcError::new(
                        ErrorKind::InvalidConfig,
                        "the clock resolved a sleep before its deadline, so it cannot pace an \
                         event pump (the simulator's RealtimeClock never sleeps); pass the \
                         clock the connections sample with, one that sleeps",
                    ));
                }
            }
        })
    }

    fn batch(&mut self, name: &str) -> Result<&mut Batch<'a>> {
        self.batches
            .iter_mut()
            .find(|batch| batch.spec.name == name)
            .ok_or_else(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    format!("the event pump has no batch named {name:?}"),
                )
            })
    }

    fn refuse_when_closed(&self) -> Result<()> {
        if self.closed {
            return Err(PlcError::new(
                ErrorKind::Disconnected,
                "the event pump is closed",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for EventPump<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let started = self
            .batches
            .iter()
            .filter(|batch| batch.running.is_some())
            .count();
        f.debug_struct("EventPump")
            .field("batches", &self.batches.len())
            .field("started", &started)
            .field("queued", &self.queue.len())
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}
