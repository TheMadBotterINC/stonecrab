//! The acquisition loop: a profile's measurements read at their intervals (ADR-0023).

use std::collections::VecDeque;
use std::fmt;
use std::future::poll_fn;
use std::task::Poll;
use std::time::{Duration, Instant};

use crate::sensors::{Measurement, SensorProfile, StaleTracker};
use crate::{
    Clock, Connection, ConnectionState, Driver, ErrorKind, OpFuture, PlcError, ReadRequest,
    ReadRequestBuilder, ResourceLimits, Result, StateWatch, Tag,
};

/// Reads the measurements of a profile through a connection, each at its own interval, and
/// reports fresh measurements and stale transitions.
///
/// **Intervals.** Every measurement is read every `interval`, the rate the caller asks for,
/// unless its definition sets a longer `min_interval`: then it is read every `min_interval`.
/// A faster rate is raised, never refused, so one rate serves a profile whose measurements
/// recommend different intervals. Successive reads of a measurement start at least its
/// interval apart: after a read starts, the measurement is next due one interval later.
/// A read that comes due while the previous one still runs waits for it and is not made up
/// later.
///
/// **Reads.** The first [`recv`](Self::recv) reads every measurement. After that, the
/// measurements that are due at the same instant are read in one [`ReadRequest`] (each
/// with its status-field tags), sent through [`Connection::read`] with the connection's
/// default deadline and retry policy, and the response becomes measurements through
/// [`SensorProfile::measurements`] with the clock's `wall_now`. Each one is delivered, in
/// document order, and recorded in a [`StaleTracker`].
///
/// **Staleness.** Between reads the loop also wakes when a retained value passes its
/// `stale_after` and delivers the transition: the retained measurement with
/// [`Quality::Stale`](crate::sensors::Quality::Stale) (see [`StaleTracker`]). A
/// measurement read fresh is never `Stale`, so the quality tells the two apart. A failed
/// read does not erase a retained value: an item that failed is delivered as a `Bad`
/// measurement without a value, a read that failed as a whole is returned as the error,
/// and in both cases the value read before keeps aging.
///
/// **The end.** The loop ends when the connection reports `Closing`, `Closed` or `Failed`,
/// seen while it waits or after a read that failed: `recv` then delivers what was queued
/// and resolves to `None` from then on.
///
/// No task is spawned (`CODING_STANDARDS` 8.7): the loop runs while the caller awaits
/// `recv`, as [`Subscription::recv`](crate::Subscription::recv) does. Its queue is bounded
/// by the profile: it is refilled only when empty, by the measurements of one read or the
/// transitions found at one instant, so it never holds more than one event per measurement.
/// The clock must be the one the connection samples with, and it must sleep: on a clock
/// whose sleep resolves before its deadline (the simulator's `RealtimeClock`) the loop
/// cannot pace anything and `recv` refuses to wait.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-ACQUISITION
/// Upstream PLC4X has no sensor acquisition loop; its polling subscription helper is the
/// nearest relative (docs/parity/core.md#CORE-SUBSCRIPTION). This is a Stonecrab type with
/// no upstream parity claim.
///
/// # Examples
/// A level read every second (the profile's `min_interval` raises the 500 ms asked for)
/// whose next two reads fail: the value read first goes stale 2.5 s after it was read.
/// ```
/// # #[cfg(all(feature = "sim", feature = "testkit"))]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::{Clock, DriverRegistry, PlcValue, TagId};
/// use stonecrab::sensors::{Acquisition, Quality, SensorProfile};
/// use stonecrab::sim::{FailureStep, SimDevice, SimDriver, SimTag};
/// use stonecrab::testkit::{FakeClock, block_on_with};
///
/// let profile = SensorProfile::from_toml(r#"
///     id = "synthetic_level"
///     schema_version = 1
///     validation = "synthetic"
///
///     [connection]
///     scheme = "sim"
///
///     [provenance]
///     source = "Synthetic reference profile; no real device"
///
///     [[measurements]]
///     name = "tank_level"
///     address = "STATE/level:u16"
///     data_type = "UINT"
///     unit = "mm"
///     min_interval_ms = 1000
///     stale_after_ms = 2500
/// "#)?;
/// let device = SimDevice::new();
/// device.set("level", SimTag::new(PlcValue::U16(840)))?;
/// let clock = Arc::new(FakeClock::default());
/// let driver = Arc::new(
///     SimDriver::new().with_clock(Arc::<FakeClock>::clone(&clock)).with_device("bench", device)?,
/// );
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::<SimDriver>::clone(&driver))?;
/// let connection = block_on_with(&clock, registry.connect("sim://bench"))?;
/// let start = clock.now();
///
/// let mut acquisition =
///     Acquisition::new(&profile, &*driver, &*connection, &*clock, Duration::from_millis(500))?;
/// let first = block_on_with(&clock, acquisition.recv())?.ok_or("ended")?;
/// assert_eq!((first.quality, first.value), (Quality::Good, Some(PlcValue::U16(840))));
///
/// let failing = vec![TagId::new("tank_level")?];
/// let bench = driver.device("bench").ok_or("no device")?;
/// bench.push_failure(FailureStep::PartialFailure(failing.clone()))?;
/// bench.push_failure(FailureStep::PartialFailure(failing))?;
/// for second in [1, 2] {
///     let failed = block_on_with(&clock, acquisition.recv())?.ok_or("ended")?;
///     assert_eq!((failed.quality, failed.value), (Quality::Bad, None));
///     assert_eq!(clock.now() - start, Duration::from_secs(second));
/// }
/// let stale = block_on_with(&clock, acquisition.recv())?.ok_or("ended")?;
/// assert_eq!(stale.quality, Quality::Stale);
/// assert_eq!(stale.quality_before_stale, Some(Quality::Good));
/// assert_eq!(stale.value, Some(PlcValue::U16(840)));
/// assert_eq!(clock.now() - start, Duration::from_millis(2500) + Duration::from_nanos(1));
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sim", feature = "testkit")))]
/// # fn main() {}
/// ```
pub struct Acquisition<'a> {
    profile: &'a SensorProfile,
    connection: &'a dyn Connection,
    clock: &'a dyn Clock,
    watch: StateWatch,
    limits: ResourceLimits,
    /// One slot per definition of the profile, in document order.
    slots: Vec<Slot>,
    tracker: StaleTracker,
    queue: VecDeque<Measurement>,
    ended: bool,
}

/// When one measurement is read next, and what reading it takes.
struct Slot {
    /// The measurement's tag and its status-field tags.
    tags: Vec<Tag>,
    interval: Duration,
    /// `None` once the next read would lie past the end of time.
    due: Option<Instant>,
}

/// The connection will not serve another read: the acquisition ends.
const fn ended(state: ConnectionState) -> bool {
    matches!(
        state,
        ConnectionState::Closing | ConnectionState::Closed | ConnectionState::Failed(_)
    )
}

/// One read request of `tags`, built against `limits`.
fn request(limits: ResourceLimits, tags: impl IntoIterator<Item = Tag>) -> Result<ReadRequest> {
    tags.into_iter()
        .fold(ReadRequest::builder(limits), ReadRequestBuilder::tag)
        .build()
}

impl<'a> Acquisition<'a> {
    /// An acquisition of `profile`'s measurements through `connection`, read every
    /// `interval` or every `min_interval` of a measurement that sets a longer one.
    ///
    /// `driver` parses the profile's addresses ([`SensorProfile::tags`]); `clock` is the
    /// clock the connection samples with. Every measurement is due at once: the first
    /// [`recv`](Self::recv) reads them all. Nothing is read here.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] for a zero `interval`, for a profile the driver cannot
    /// make tags of (the [`ProfileError`](crate::sensors::ProfileError) of
    /// [`SensorProfile::tags`]) and for a profile without measurements; then what
    /// [`ReadRequestBuilder::build`] refuses when every tag of the profile does not fit one
    /// read under the connection's limits.
    ///
    /// # Examples
    /// See [`Acquisition`].
    pub fn new(
        profile: &'a SensorProfile,
        driver: &dyn Driver,
        connection: &'a dyn Connection,
        clock: &'a dyn Clock,
        interval: Duration,
    ) -> Result<Self> {
        if interval.is_zero() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "the acquisition interval must be greater than zero",
            ));
        }
        let tags = profile.tags(driver)?;
        let limits = connection.limits();
        drop(request(limits, tags.iter().cloned())?);
        let now = clock.now();
        let mut tags = tags.into_iter();
        let slots = profile
            .measurements
            .iter()
            .map(|definition| Slot {
                tags: tags
                    .by_ref()
                    .take(definition.status_fields.len().saturating_add(1))
                    .collect(),
                interval: definition.min_interval.unwrap_or_default().max(interval),
                due: Some(now),
            })
            .collect();
        Ok(Self {
            profile,
            connection,
            clock,
            watch: connection.watch_state(),
            limits,
            slots,
            tracker: StaleTracker::new(profile),
            queue: VecDeque::new(),
            ended: false,
        })
    }

    /// The next fresh measurement or stale transition, reading and waiting as needed.
    ///
    /// Resolves to `Some(measurement)` in the order the loop produced them, and to `None`
    /// once the connection has ended and the queue is empty; after that every call resolves
    /// to `None` at once.
    ///
    /// # Errors
    /// The error of a read that failed as a whole while the connection is still open (a
    /// timeout, a busy in-flight slot, a link being re-established): the measurements of
    /// that read are not delivered, their retained values keep aging, and the next call
    /// carries on. [`ErrorKind::InvalidConfig`] when the clock's sleep resolved before its
    /// deadline, so the clock cannot pace the acquisition.
    ///
    /// # Cancel safety
    /// Not cancel-safe: request may have been transmitted; outcome is Unknown. Dropping the
    /// future during a read abandons that read, with the consequence [`Connection::read`]
    /// documents (a read changes nothing on the device and the connection stays usable);
    /// the measurements of that read are lost and are next due one interval after it
    /// started. Dropping the future while it waits sends nothing and loses nothing: a
    /// measurement is taken from the queue only when the future returns it, and every
    /// due time and stale threshold stays as it was.
    ///
    /// # Wire effects
    /// Sends one read request per instant at which measurements are due, through the
    /// connection's read path and so its in-flight slot, deadline and retry policy. Waiting
    /// and stale transitions touch no transport.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Result;
    /// use stonecrab::sensors::{Acquisition, Quality};
    ///
    /// async fn count_stale(acquisition: &mut Acquisition<'_>) -> Result<usize> {
    ///     let mut stale = 0;
    ///     while let Some(measurement) = acquisition.recv().await? {
    ///         if measurement.quality == Quality::Stale {
    ///             stale += 1;
    ///         }
    ///     }
    ///     Ok(stale)
    /// }
    /// ```
    pub fn recv(&mut self) -> OpFuture<'_, Option<Measurement>> {
        Box::pin(async move {
            loop {
                if let Some(measurement) = self.queue.pop_front() {
                    return Ok(Some(measurement));
                }
                if self.ended {
                    return Ok(None);
                }
                let now = self.clock.now();
                self.queue.extend(self.tracker.expire(now));
                if !self.queue.is_empty() {
                    continue;
                }
                if let Some(request) = self.due(now)? {
                    match self.connection.read(request).await {
                        Ok(response) => {
                            let wall_now = self.clock.wall_now();
                            for measurement in self.profile.measurements(&response, wall_now) {
                                self.tracker.record(measurement.clone());
                                self.queue.push_back(measurement);
                            }
                        }
                        Err(_) if ended(self.connection.state()) => self.ended = true,
                        Err(error) => return Err(error),
                    }
                    continue;
                }
                let wake = self
                    .slots
                    .iter()
                    .filter_map(|slot| slot.due)
                    .chain(self.tracker.next_expiry())
                    .min();
                self.ended = !self.wait(wake).await?;
            }
        })
    }

    /// The read of every measurement due at `now`, each then due one interval later, or
    /// `None` when none is due.
    fn due(&mut self, now: Instant) -> Result<Option<ReadRequest>> {
        let mut tags = Vec::new();
        for slot in &mut self.slots {
            if slot.due.is_some_and(|due| due <= now) {
                slot.due = now.checked_add(slot.interval);
                tags.extend(slot.tags.iter().cloned());
            }
        }
        if tags.is_empty() {
            return Ok(None);
        }
        request(self.limits, tags).map(Some)
    }

    /// Wait until `wake` (forever when `None`); false once the connection has ended.
    async fn wait(&self, wake: Option<Instant>) -> Result<bool> {
        let mut change = self.watch.changed();
        let mut sleep = wake.map(|at| self.clock.sleep_until(at));
        let slept = poll_fn(|cx| {
            while change.as_mut().poll(cx).is_ready() {
                change = self.watch.changed();
            }
            if ended(self.watch.current()) {
                return Poll::Ready(false);
            }
            // `Clock::sleep_until` never fails for a correct clock (its contract); one that
            // resolves early is caught below, so the result is not read.
            sleep.as_mut().map_or(Poll::Pending, |sleep| {
                sleep.as_mut().poll(cx).map(|_slept| true)
            })
        })
        .await;
        if slept && wake.is_some_and(|at| self.clock.now() < at) {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "the clock resolved a sleep before its deadline, so it cannot pace an \
                 acquisition (the simulator's RealtimeClock never sleeps); pass the clock \
                 the connection samples with, one that sleeps",
            ));
        }
        Ok(slept)
    }
}

impl fmt::Debug for Acquisition<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Acquisition")
            .field("profile", &self.profile.id)
            .field("measurements", &self.slots.len())
            .field("queued", &self.queue.len())
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}
