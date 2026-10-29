//! The connection cache: shared connections leased one holder at a time (ADR-0030).
//!
//! The cache keeps one connection per connection string, opened through a
//! [`DriverRegistry`], and hands it out as a [`LeasedConnection`] to one holder at a time.
//! Other callers for the same string queue in arrival order. All timing is read from the
//! injected [`Clock`] when the cache or a lease is called; nothing is spawned and nothing
//! runs between calls, so an expired lease or an idle connection is noticed by the next
//! call that looks at it.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::future::poll_fn;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::{Poll, Waker};
use std::time::{Duration, Instant};

use crate::future::{WakerSlots, wake_all};
use crate::{
    Capabilities, Clock, Connection, ConnectionState, Diagnostics, DriverRegistry, ErrorKind,
    FailureCategory, OpFuture, Operation, PlcError, ReadRequest, ReadResponse, ResourceLimits,
    Result, StateWatch, SubscribeRequest, Subscription, WriteRequest, WriteResponse,
};

/// Upstream's `DEFAULT_MAX_IDLE_TIME_MS`: five minutes.
const DEFAULT_MAX_IDLE_TIME: Duration = Duration::from_secs(300);
/// Upstream's `DEFAULT_MAX_LEASE_TIME_MS`: one minute.
const DEFAULT_MAX_LEASE_TIME: Duration = Duration::from_secs(60);
/// Upstream's `DEFAULT_MAX_WAIT_TIME_MS`: thirty seconds.
const DEFAULT_MAX_WAIT_TIME: Duration = Duration::from_secs(30);
/// Upstream's `DEFAULT_PING_TIMEOUT_MS`: five seconds.
const DEFAULT_PING_TIMEOUT: Duration = Duration::from_secs(5);
/// Upstream's `DEFAULT_IDLE_PING_THRESHOLD_MS`: thirty seconds.
const DEFAULT_IDLE_PING_THRESHOLD: Duration = Duration::from_secs(30);
/// Upstream's `DEFAULT_CLOSE_TIMEOUT_MS`: five seconds.
const DEFAULT_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

/// A cache of connections shared by connection string, leased to one holder at a time.
///
/// Opening a connection to a controller can take several round trips, and a controller
/// accepts few connections. The cache opens one connection per connection string through
/// its [`DriverRegistry`] and lends it out: [`get`](Self::get) returns a
/// [`LeasedConnection`], which is itself a [`Connection`], and dropping the lease gives the
/// connection back instead of closing it. The cache composes with every [`Driver`](crate::Driver)
/// and adds none; it performs no operation of its own.
///
/// # Leases and the queue
/// A connection string has at most one lease at a time. A caller that asks while the
/// connection is leased waits, behind the callers that asked before it, until the lease
/// comes back, its maximum lease time passes, or its own maximum wait time passes
/// ([`ErrorKind::Timeout`]). At most [`ResourceLimits::max_pending_requests`] callers hold
/// or wait for one string, the holder included; one more is refused with
/// [`ErrorKind::Backpressure`] before anything is opened. Dropping a waiting `get` future
/// leaves the queue at once.
///
/// # Timeouts
/// Every bound is measured on the injected [`Clock`] and evaluated when the cache or a lease
/// is called, never by a task of its own:
///
/// - **Lease time** (default one minute): a lease older than this fails every later call
///   with [`ErrorKind::Timeout`], and a waiter takes the connection over.
/// - **Wait time** (default 30 s): the longest a `get` waits in the queue.
/// - **Idle time** (default five minutes): a connection nobody leased for this long is
///   closed by the next [`get`](Self::get), for any string.
/// - **Idle ping threshold** (default 30 s) and **ping timeout** (default 5 s): a connection
///   that was idle this long, or whose last lease expired, is pinged before it is lent again
///   and replaced if the ping fails or outlasts the ping timeout. A connection whose
///   [`Capabilities::ping`] is false is lent without a ping; a lease whose call fails with
///   [`ErrorKind::Disconnected`] or [`ErrorKind::Timeout`] makes the cache replace the
///   connection before the next lease.
/// - **Close time** (default 5 s): the longest the cache waits for a connection to close;
///   a close that takes longer is abandoned and the connection dropped.
///
/// Connecting is bounded by the driver, by `timeouts.connect` of the connection string
/// ([`Driver::connect`](crate::Driver::connect)). A clock whose sleeps resolve at once, such
/// as the simulator's `RealtimeClock`, cannot pace the bounds: a waiter then notices its wait
/// time only when it is woken, and a ping or a close is never cut short.
///
/// # Compatibility: docs/parity/tools.md#TOOL-CC-CACHE
/// Corresponds to upstream's `PlcConnectionCache`, built over a `PlcConnectionFactory`. The
/// cache has no scheduler of its own; see `docs/adr/0030-connection-cache.md` for the
/// differences.
///
/// # Examples
/// ```
/// # #[cfg(all(feature = "sim", feature = "testkit"))]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::sim::{SimDevice, SimDriver};
/// use stonecrab::testkit::{FakeClock, block_on_with};
/// use stonecrab::{Clock, Connection, ConnectionCache, ConnectionState, DriverRegistry};
///
/// let clock = FakeClock::default();
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new()
///         .with_clock(Arc::new(clock.clone()))
///         .with_device("bench", SimDevice::new())?,
/// ))?;
/// let cache = ConnectionCache::builder(Arc::new(registry), Arc::new(clock.clone())).build()?;
///
/// let lease = block_on_with(&clock, cache.get("sim://bench"))?;
/// assert_eq!(lease.state(), ConnectionState::Connected);
/// assert_eq!(cache.active_leases(), 1);
/// drop(lease);
/// assert_eq!((cache.len(), cache.active_leases()), (1, 0));
/// block_on_with(&clock, cache.close())?;
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sim", feature = "testkit")))]
/// # fn main() {}
/// ```
pub struct ConnectionCache {
    inner: Arc<Inner>,
}

impl ConnectionCache {
    /// A builder for a cache that opens connections through `registry` and reads time from
    /// `clock`, with upstream's default settings.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-BUILDER
    /// Upstream's `withConnectionFactory` and `withScheduler` are the two arguments.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use std::time::{Duration, Instant};
    /// use stonecrab::{Clock, ConnectionCache, DriverRegistry, OpFuture};
    ///
    /// struct Frozen(Instant);
    /// impl Clock for Frozen {
    ///     fn now(&self) -> Instant { self.0 }
    ///     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
    ///     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
    /// }
    ///
    /// let cache = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(Frozen(Instant::now())))
    ///     .max_lease_time(Duration::from_secs(10))
    ///     .build()?;
    /// assert!(cache.is_empty());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn builder(registry: Arc<DriverRegistry>, clock: Arc<dyn Clock>) -> ConnectionCacheBuilder {
        ConnectionCacheBuilder {
            registry,
            clock,
            settings: Settings {
                max_idle_time: DEFAULT_MAX_IDLE_TIME,
                max_lease_time: DEFAULT_MAX_LEASE_TIME,
                max_wait_time: DEFAULT_MAX_WAIT_TIME,
                ping_timeout: DEFAULT_PING_TIMEOUT,
                idle_ping_threshold: DEFAULT_IDLE_PING_THRESHOLD,
                close_timeout: DEFAULT_CLOSE_TIMEOUT,
            },
            limits: None,
        }
    }

    /// Lease the connection of `connection_string`, opening it first if the cache holds none.
    ///
    /// The string is the cache key exactly as given; two spellings of one endpoint are two
    /// connections. The call first closes every connection that has been idle longer than
    /// the maximum idle time, then joins the queue of the string and waits for its turn
    /// (see the type documentation). On its turn it reuses the cached connection, pings it
    /// first if it was idle past the idle ping threshold, or replaces it if it failed: the
    /// old connection is closed and a new one opened with
    /// [`DriverRegistry::connect`].
    ///
    /// # Errors
    /// Every error the cache makes carries diagnostics with `operation = Connect`.
    /// - [`ErrorKind::Disconnected`] if the cache is closed, or is closed or the string
    ///   removed while the call waits or connects.
    /// - [`ErrorKind::Backpressure`] if [`ResourceLimits::max_pending_requests`] callers
    ///   already hold or wait for the string.
    /// - [`ErrorKind::Timeout`] if the maximum wait time passes in the queue.
    /// - The kinds of [`DriverRegistry::connect`] when a connection has to be opened.
    ///
    /// # Cancel safety
    /// Cancel-safe for the queue: dropping the future while it waits leaves the queue at
    /// once, and dropping it after its turn came lets the next caller in. Dropping it while
    /// it opens a connection abandons the attempt as [`Driver::connect`](crate::Driver::connect)
    /// describes; while it pings, the ping is abandoned (a ping may have been transmitted);
    /// while it closes an idle or failed connection, that connection is dropped without
    /// finishing its close. The deadline sources are the maximum wait time, the ping timeout,
    /// the close timeout and the connection string's `timeouts.connect`.
    ///
    /// Poll the future to completion or drop it. The turn is taken when the future is
    /// polled, so a future kept at the front of the queue but never polled again holds up
    /// the string, and the callers behind it fail with [`ErrorKind::Timeout`].
    ///
    /// # Wire effects
    /// None for a connection that is reused without a ping. Otherwise what the driver's
    /// `close`, `connect` and `ping` send.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-QUEUE
    /// Upstream's `getConnection`. Upstream blocks the calling thread; this future waits.
    ///
    /// # Examples
    /// A string no registered driver serves fails like [`DriverRegistry::connect`] does.
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::testkit::{FakeClock, block_on_with};
    /// use stonecrab::{ConnectionCache, DriverRegistry, ErrorKind, Operation};
    ///
    /// let clock = FakeClock::default();
    /// let cache = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(clock.clone())).build()?;
    /// let refused = block_on_with(&clock, cache.get("sim://bench")).err().map(|error| error.kind());
    /// assert_eq!(refused, Some(ErrorKind::Unsupported(Operation::Connect)));
    /// assert!(cache.is_empty(), "a failed connect leaves no entry");
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn get(&self, connection_string: &str) -> OpFuture<'_, LeasedConnection> {
        let key = connection_string.to_owned();
        Box::pin(async move {
            let idle = self
                .inner
                .with(|state, now, _| state.take_idle(now, &self.inner.settings));
            for connection in idle {
                self.inner.close_bounded(connection).await;
            }
            let (claim, deadline) = self.inner.enqueue(key)?;
            loop {
                let step = poll_fn(|cx| Poll::Ready(self.inner.step(&claim, deadline, cx.waker())));
                let Some(until) = step.await? else { break };
                // Park until the queue wakes this caller or the clock reaches `until`, then
                // look again. A clock whose sleep ends early (it does not sleep) leaves only
                // the queue's wake-up, so the caller never spins.
                let clock = &*self.inner.clock;
                let mut sleep = clock.sleep_until(until);
                let mut parked = false;
                poll_fn(|cx| {
                    let first = !std::mem::replace(&mut parked, true);
                    if first && (sleep.as_mut().poll(cx).is_pending() || clock.now() < until) {
                        return Poll::Pending;
                    }
                    Poll::Ready(())
                })
                .await;
            }
            self.inner.acquire(claim).await
        })
    }

    /// Close every cached connection and refuse every later [`get`](Self::get).
    ///
    /// Waiting callers fail with [`ErrorKind::Disconnected`], and so does every later call
    /// through a lease that is still out. Each connection is closed in turn, each bounded by
    /// the close timeout. Closing a closed cache does nothing.
    ///
    /// # Errors
    /// None: a connection that fails to close, or takes longer than the close timeout, is
    /// dropped. The `Result` is the uniform [`OpFuture`] shape.
    ///
    /// # Cancel safety
    /// Cancel-safe for the cache, which is closed and emptied before the first connection is
    /// closed; dropping the future drops the connections not yet closed without finishing
    /// their close. The deadline source is the close timeout.
    ///
    /// # Wire effects
    /// What each driver's `close` sends.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-CLOSED
    /// Upstream's `close()`; a later `getConnection` throws
    /// `PlcConnectionCacheClosedException`.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::testkit::{FakeClock, block_on_with};
    /// use stonecrab::{ConnectionCache, DriverRegistry, ErrorKind};
    ///
    /// let clock = FakeClock::default();
    /// let cache = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(clock.clone())).build()?;
    /// block_on_with(&clock, cache.close())?;
    /// let refused = block_on_with(&clock, cache.get("sim://bench")).err().map(|error| error.kind());
    /// assert_eq!(refused, Some(ErrorKind::Disconnected));
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(async move {
            let connections = self.inner.with(|state, _, deferred| {
                state.closed = true;
                let entries = std::mem::take(&mut state.entries);
                entries
                    .into_values()
                    .filter_map(|mut entry| entry.dissolve(deferred))
                    .collect::<Vec<_>>()
            });
            for connection in connections {
                self.inner.close_bounded(connection).await;
            }
            Ok(())
        })
    }

    /// Evict the connection of `connection_string` and close it, even while it is leased.
    ///
    /// The string leaves the cache before the connection is closed, so the next
    /// [`get`](Self::get) opens a new one. A lease of the evicted connection fails every later
    /// call with [`ErrorKind::Disconnected`], and so do the callers that were waiting for it.
    /// Returns whether the cache held the string.
    ///
    /// # Errors
    /// None; see [`close`](Self::close).
    ///
    /// # Cancel safety
    /// Cancel-safe for the cache, which forgets the string before the close starts; dropping
    /// the future drops the connection without finishing its close. The deadline source is
    /// the close timeout.
    ///
    /// # Wire effects
    /// What the driver's `close` sends.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-ADMIN
    /// Upstream's `removeCachedConnection`.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::testkit::{FakeClock, block_on_with};
    /// use stonecrab::{ConnectionCache, DriverRegistry};
    ///
    /// let clock = FakeClock::default();
    /// let cache = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(clock.clone())).build()?;
    /// assert!(!block_on_with(&clock, cache.remove("sim://bench"))?, "nothing was cached");
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn remove(&self, connection_string: &str) -> OpFuture<'_, bool> {
        let key = connection_string.to_owned();
        Box::pin(async move {
            let removed = self.inner.with(|state, _, deferred| {
                state
                    .entries
                    .remove(&key)
                    .map(|mut entry| entry.dissolve(deferred))
            });
            let Some(connection) = removed else {
                return Ok(false);
            };
            if let Some(connection) = connection {
                self.inner.close_bounded(connection).await;
            }
            Ok(true)
        })
    }

    /// The number of connection strings the cache holds: those with a connection, a lease,
    /// or a caller waiting or connecting. An idle connection past its idle time counts
    /// until the next [`get`](Self::get) closes it.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-ADMIN
    /// Upstream's `getCachedConnectionCount`.
    ///
    /// # Examples
    /// See [`ConnectionCache`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.with(|state, _, _| state.entries.len())
    }

    /// Whether the cache holds no connection string; see [`len`](Self::len).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The number of leases out, one at most per connection string. A lease past its
    /// maximum lease time counts until it is dropped or a waiter takes its connection over.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-ADMIN
    /// Upstream's `getActiveLeaseCount`.
    ///
    /// # Examples
    /// See [`ConnectionCache`].
    #[must_use]
    pub fn active_leases(&self) -> usize {
        self.inner.with(|state, _, _| {
            state
                .entries
                .values()
                .filter(|entry| entry.holder.is_some_and(|holder| holder.lease.is_some()))
                .count()
        })
    }

    /// The connection strings the cache holds (see [`len`](Self::len)), sorted. The list is
    /// a snapshot: the cache may change as soon as it is returned.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-CC-ADMIN
    /// Upstream's `getCachedConnections`, which returns an unordered set.
    ///
    /// # Examples
    /// See [`ConnectionCache::builder`]; a new cache holds nothing.
    #[must_use]
    pub fn connection_strings(&self) -> Vec<String> {
        self.inner
            .with(|state, _, _| state.entries.keys().cloned().collect())
    }
}

impl fmt::Debug for ConnectionCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionCache")
            .field("settings", &self.inner.settings)
            .field("max_pending", &self.inner.max_pending)
            .field("connection_strings", &self.connection_strings())
            .finish_non_exhaustive()
    }
}

/// The settings of a [`ConnectionCache`] before it is built.
///
/// Every duration must be representable on the clock from now, and the maximum idle time,
/// the maximum lease time, the ping timeout and the close timeout must be positive. A zero
/// maximum wait time means "do not wait": a `get` for a leased string fails with
/// [`ErrorKind::Timeout`] at once. A zero idle ping threshold pings before every reuse.
///
/// # Compatibility: docs/parity/tools.md#TOOL-CC-BUILDER
/// Upstream's `PlcConnectionCache.Builder`, whose `0` disables the lease and close bounds;
/// here every bound stays finite.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use std::time::{Duration, Instant};
/// use stonecrab::{Clock, ConnectionCache, DriverRegistry, ErrorKind, OpFuture};
///
/// struct Frozen(Instant);
/// impl Clock for Frozen {
///     fn now(&self) -> Instant { self.0 }
///     fn wall_now(&self) -> std::time::SystemTime { std::time::SystemTime::UNIX_EPOCH }
///     fn sleep_until(&self, _: Instant) -> OpFuture<'_, ()> { Box::pin(std::future::ready(Ok(()))) }
/// }
///
/// let refused = ConnectionCache::builder(Arc::new(DriverRegistry::new()), Arc::new(Frozen(Instant::now())))
///     .max_lease_time(Duration::ZERO)
///     .build()
///     .err()
///     .map(|error| error.kind());
/// assert_eq!(refused, Some(ErrorKind::InvalidConfig));
/// ```
#[must_use]
pub struct ConnectionCacheBuilder {
    registry: Arc<DriverRegistry>,
    clock: Arc<dyn Clock>,
    settings: Settings,
    limits: Option<ResourceLimits>,
}

impl ConnectionCacheBuilder {
    /// Close a connection nobody leased for this long (default five minutes; upstream's
    /// `withMaxIdleTime`).
    pub const fn max_idle_time(mut self, value: Duration) -> Self {
        self.settings.max_idle_time = value;
        self
    }

    /// Invalidate a lease held this long (default one minute; upstream's
    /// `withMaxLeaseTime`).
    pub const fn max_lease_time(mut self, value: Duration) -> Self {
        self.settings.max_lease_time = value;
        self
    }

    /// Give up waiting for a lease after this long (default 30 s; upstream's
    /// `withMaxWaitTime`).
    pub const fn max_wait_time(mut self, value: Duration) -> Self {
        self.settings.max_wait_time = value;
        self
    }

    /// Replace a connection whose idle ping takes longer than this (default 5 s; upstream's
    /// `withPingTimeout`).
    pub const fn ping_timeout(mut self, value: Duration) -> Self {
        self.settings.ping_timeout = value;
        self
    }

    /// Ping a connection idle this long before lending it again (default 30 s; upstream's
    /// `withIdlePingThreshold`).
    pub const fn idle_ping_threshold(mut self, value: Duration) -> Self {
        self.settings.idle_ping_threshold = value;
        self
    }

    /// Abandon a close that takes longer than this (default 5 s; upstream's
    /// `withCloseTimeout`).
    pub const fn close_timeout(mut self, value: Duration) -> Self {
        self.settings.close_timeout = value;
        self
    }

    /// Bound each connection string's queue by `limits.max_pending_requests()`, the holder
    /// included (default: [`ResourceLimits::builder`]'s, 16).
    pub const fn limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Validate the settings and make the cache.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConfig`] if the maximum idle time, the maximum lease time, the
    /// ping timeout or the close timeout is zero, or a duration does not fit the clock.
    ///
    /// # Examples
    /// See [`ConnectionCacheBuilder`].
    pub fn build(self) -> Result<ConnectionCache> {
        let settings = self.settings;
        let positive = [
            ("max_idle_time", settings.max_idle_time),
            ("max_lease_time", settings.max_lease_time),
            ("ping_timeout", settings.ping_timeout),
            ("close_timeout", settings.close_timeout),
        ];
        let any = [
            ("max_wait_time", settings.max_wait_time),
            ("idle_ping_threshold", settings.idle_ping_threshold),
        ];
        let now = self.clock.now();
        for (name, value) in positive {
            if value.is_zero() {
                return Err(invalid(format!("{name} must be greater than zero")));
            }
        }
        for (name, value) in positive.into_iter().chain(any) {
            if now.checked_add(value).is_none() {
                return Err(invalid(format!("{name} does not fit the clock")));
            }
        }
        let limits = match self.limits {
            Some(limits) => limits,
            None => ResourceLimits::builder().build()?,
        };
        Ok(ConnectionCache {
            inner: Arc::new(Inner {
                registry: self.registry,
                clock: self.clock,
                settings,
                max_pending: limits.max_pending_requests().get(),
                state: Mutex::new(State::default()),
            }),
        })
    }
}

impl fmt::Debug for ConnectionCacheBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectionCacheBuilder")
            .field("registry", &self.registry)
            .field("settings", &self.settings)
            .field("limits", &self.limits)
            .finish_non_exhaustive()
    }
}

/// A connection leased from a [`ConnectionCache`]; dropping it gives the connection back.
///
/// The lease is a [`Connection`]: `read`, `write`, `ping` and `subscribe` go to the cached
/// connection after the lease checks that it is still valid, and `state`, `watch_state`,
/// `capabilities` and `limits` describe the cached connection. A lease is invalid once its
/// maximum lease time has passed ([`ErrorKind::Timeout`]), or once it was given back with
/// `close`, its string removed or the cache closed ([`ErrorKind::Disconnected`]); a call
/// through an invalid lease fails before it reaches the connection. `close` does not close
/// the connection: it gives the lease back early, as dropping it does.
///
/// A call that fails with [`ErrorKind::Disconnected`] or [`ErrorKind::Timeout`] marks the
/// connection as failed, and the cache replaces it before the next lease. A subscription made
/// through a lease belongs to the cached connection: it outlives the lease and is not
/// restored on a replacement connection (ADR-0030).
///
/// # Compatibility: docs/parity/tools.md#TOOL-CC-LEASE
/// Upstream's `LeasedPlcConnection`, whose `close()` returns the connection to the cache.
///
/// # Examples
/// See [`ConnectionCache`].
pub struct LeasedConnection {
    claim: Claim,
    connection: Arc<dyn Connection>,
    expires: Instant,
}

impl LeasedConnection {
    /// Run `call` on the cached connection if the lease is still valid, and mark the
    /// connection failed when the call fails with a kind that says the link is gone.
    fn through<'a, T: Send + 'a>(
        &'a self,
        operation: Operation,
        call: impl FnOnce(&'a dyn Connection) -> OpFuture<'a, T> + Send + 'a,
    ) -> OpFuture<'a, T> {
        Box::pin(async move {
            self.claim.check(operation, self.expires)?;
            let outcome = call(&*self.connection).await;
            self.claim.observe(outcome.as_ref().err());
            outcome
        })
    }
}

impl Connection for LeasedConnection {
    fn state(&self) -> ConnectionState {
        self.connection.state()
    }

    fn watch_state(&self) -> StateWatch {
        self.connection.watch_state()
    }

    fn capabilities(&self) -> Capabilities {
        self.connection.capabilities()
    }

    fn limits(&self) -> ResourceLimits {
        self.connection.limits()
    }

    /// Read through the cached connection; see [`LeasedConnection`] for when the lease
    /// refuses.
    ///
    /// # Cancel safety
    /// As the cached connection's `read`.
    ///
    /// # Wire effects
    /// None when the lease refuses; otherwise the cached connection's.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        self.through(Operation::Read, |connection| connection.read(request))
    }

    /// Write through the cached connection; see [`LeasedConnection`] for when the lease
    /// refuses.
    ///
    /// # Cancel safety
    /// As the cached connection's `write`.
    ///
    /// # Wire effects
    /// None when the lease refuses; otherwise the cached connection's.
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        self.through(Operation::Write, |connection| connection.write(request))
    }

    /// Give the lease back early; the cached connection stays open.
    ///
    /// # Cancel safety
    /// Cancel-safe: the lease is given back on the first poll, with no I/O.
    ///
    /// # Wire effects
    /// None.
    fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(async move {
            self.claim.release();
            Ok(())
        })
    }

    /// Ping through the cached connection; see [`LeasedConnection`] for when the lease
    /// refuses.
    ///
    /// # Cancel safety
    /// As the cached connection's `ping`.
    ///
    /// # Wire effects
    /// None when the lease refuses; otherwise the cached connection's.
    fn ping(&self) -> OpFuture<'_, ()> {
        self.through(Operation::Ping, |connection| connection.ping())
    }

    /// Subscribe through the cached connection; see [`LeasedConnection`] for when the lease
    /// refuses and for the life of the subscription.
    ///
    /// # Cancel safety
    /// As the cached connection's `subscribe`.
    ///
    /// # Wire effects
    /// None when the lease refuses; otherwise the cached connection's.
    fn subscribe(&self, request: SubscribeRequest) -> OpFuture<'_, Subscription> {
        self.through(Operation::Subscribe, |connection| {
            connection.subscribe(request)
        })
    }
}

impl fmt::Debug for LeasedConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LeasedConnection")
            .field("connection_string", &self.claim.key)
            .field("expires", &self.expires)
            .finish_non_exhaustive()
    }
}

/// The durations a builder collects.
#[derive(Debug, Clone, Copy)]
struct Settings {
    max_idle_time: Duration,
    max_lease_time: Duration,
    max_wait_time: Duration,
    ping_timeout: Duration,
    idle_ping_threshold: Duration,
    close_timeout: Duration,
}

/// The cache's shared state; leases keep it alive.
struct Inner {
    registry: Arc<DriverRegistry>,
    clock: Arc<dyn Clock>,
    settings: Settings,
    /// `max_pending_requests` of the builder's limits.
    max_pending: usize,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    /// Identifies a caller across every string, so one that outlived its entry (the string
    /// was removed and asked for again) never matches the new entry.
    next_id: u64,
    entries: BTreeMap<String, Entry>,
}

/// One connection string: its connection, its holder and its queue.
struct Entry {
    connection: Option<Arc<dyn Connection>>,
    holder: Option<Holder>,
    waiters: VecDeque<Waiter>,
    wakers: WakerSlots,
    /// When the connection was last known good: opened or given back in time.
    last_good: Instant,
    /// The last lease expired: ping before the next one, as upstream forces validation.
    suspect: bool,
    /// A call through a lease failed with a link kind: replace before the next lease.
    broken: bool,
}

/// The caller whose turn it is. `lease` is `None` while it connects or pings.
#[derive(Debug, Clone, Copy)]
struct Holder {
    id: u64,
    lease: Option<Instant>,
}

/// A caller waiting for its turn, with the slot of its parked waker.
struct Waiter {
    id: u64,
    slot: Option<u64>,
}

/// Wakers taken out under the lock, woken or dropped once it is released (a waker's `Drop`
/// may call back into the cache).
#[derive(Default)]
struct Deferred {
    wake: Vec<Waker>,
    discard: Vec<Waker>,
}

/// A caller's place for one string, as a waiter, the holder or a lease; dropping it gives
/// the place up.
struct Claim {
    inner: Arc<Inner>,
    key: String,
    id: u64,
}

/// What the holder does with the cached connection before it becomes a lease.
enum Prepare {
    /// Lent as it is; the lease has begun and ends at the instant.
    Lent(Arc<dyn Connection>, Instant),
    /// Ping it first.
    Ping(Arc<dyn Connection>),
    /// Close it and open a new one.
    Replace(Arc<dyn Connection>),
    /// There is none: open one.
    Connect,
}

impl Inner {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Run `f` on the state with the clock's reading, then release the lock and handle the
    /// wakers `f` set aside.
    fn with<R>(&self, f: impl FnOnce(&mut State, Instant, &mut Deferred) -> R) -> R {
        let now = self.clock.now();
        let mut deferred = Deferred::default();
        let result = {
            let mut state = self.lock();
            f(&mut state, now, &mut deferred)
        };
        drop(deferred.discard);
        wake_all(deferred.wake);
        result
    }

    /// Join the queue of `key`, refusing a closed cache or a full queue before any I/O.
    fn enqueue(self: &Arc<Self>, key: String) -> Result<(Claim, Instant)> {
        self.with(|state, now, _| {
            if state.closed {
                return Err(closed());
            }
            let id = state.next_id;
            state.next_id = id.wrapping_add(1);
            let entry = state.entries.entry(key.clone()).or_insert_with(|| Entry {
                connection: None,
                holder: None,
                waiters: VecDeque::new(),
                wakers: WakerSlots::default(),
                last_good: now,
                suspect: false,
                broken: false,
            });
            let present = entry
                .waiters
                .len()
                .saturating_add(usize::from(entry.holder.is_some()));
            if present >= self.max_pending {
                return Err(refusal(
                    ErrorKind::Backpressure,
                    Operation::Connect,
                    format!(
                        "{present} callers already hold or wait for this connection string \
                         (max_pending_requests)"
                    ),
                ));
            }
            entry.waiters.push_back(Waiter { id, slot: None });
            let claim = Claim {
                inner: Arc::clone(self),
                key,
                id,
            };
            Ok((claim, after(now, self.settings.max_wait_time)))
        })
    }

    /// Take the turn if it has come, fail if the wait is over, or park `waker` and return
    /// the instant to look again at (`Ok(None)`: the turn is taken).
    fn step(&self, claim: &Claim, deadline: Instant, waker: &Waker) -> Result<Option<Instant>> {
        self.with(|state, now, deferred| {
            if state.closed {
                return Err(closed());
            }
            let Some(entry) = state.entries.get_mut(&claim.key) else {
                return Err(removed());
            };
            if entry
                .holder
                .is_some_and(|holder| holder.lease.is_some_and(|until| until <= now))
            {
                // The lease outlived its maximum time: take the connection back from it.
                entry.holder = None;
                entry.suspect = true;
                entry.notify(deferred);
            }
            if !entry.waiters.iter().any(|waiter| waiter.id == claim.id) {
                return Err(removed());
            }
            if entry.holder.is_none()
                && entry
                    .waiters
                    .front()
                    .is_some_and(|front| front.id == claim.id)
            {
                entry.leave(claim.id, deferred);
                entry.holder = Some(Holder {
                    id: claim.id,
                    lease: None,
                });
                return Ok(None);
            }
            if now >= deadline {
                entry.leave(claim.id, deferred);
                return Err(refusal(
                    ErrorKind::Timeout,
                    Operation::Connect,
                    "no lease of this connection string within the maximum wait time",
                ));
            }
            if let Some(waiter) = entry
                .waiters
                .iter_mut()
                .find(|waiter| waiter.id == claim.id)
            {
                deferred
                    .discard
                    .extend(entry.wakers.register(&mut waiter.slot, waker));
            }
            let lease = entry.holder.and_then(|holder| holder.lease);
            Ok(Some(lease.map_or(deadline, |until| until.min(deadline))))
        })
    }

    /// The holder's turn: make the cached connection a lease, pinging, replacing or opening
    /// it first as needed. Dropping `claim` on an error lets the next caller in.
    async fn acquire(&self, claim: Claim) -> Result<LeasedConnection> {
        let mut pinged = false;
        loop {
            let (connection, expires) = match self.prepare(&claim, pinged)? {
                Prepare::Lent(connection, expires) => (connection, expires),
                Prepare::Ping(connection) => {
                    let until = after(self.clock.now(), self.settings.ping_timeout);
                    pinged = matches!(
                        within(&*self.clock, until, connection.ping()).await,
                        Some(Ok(()))
                    );
                    if !pinged {
                        self.mark_broken(&claim);
                    }
                    continue;
                }
                Prepare::Replace(old) => {
                    // Look again before opening: the string may have been removed or the
                    // cache closed during the close.
                    self.close_bounded(old).await;
                    continue;
                }
                Prepare::Connect => self.open(&claim).await?,
            };
            return Ok(LeasedConnection {
                claim,
                connection,
                expires,
            });
        }
    }

    /// Decide what the holder does with the cached connection. `pinged`: it answered a ping
    /// just now.
    fn prepare(&self, claim: &Claim, pinged: bool) -> Result<Prepare> {
        self.with(|state, now, deferred| {
            let Some(entry) = state.held(claim) else {
                return Err(removed());
            };
            let Some(connection) = entry.connection.clone() else {
                return Ok(Prepare::Connect);
            };
            if entry.broken || connection.state() != ConnectionState::Connected {
                entry.connection = None;
                entry.broken = false;
                return Ok(Prepare::Replace(connection));
            }
            let idle = now.saturating_duration_since(entry.last_good);
            let stale = entry.suspect || idle >= self.settings.idle_ping_threshold;
            if stale && !pinged && connection.capabilities().ping {
                return Ok(Prepare::Ping(connection));
            }
            entry.suspect = false;
            let expires = entry.lend(claim.id, now, self.settings.max_lease_time, deferred);
            Ok(Prepare::Lent(connection, expires))
        })
    }

    /// Open a new connection for the holder and lend it, or close it again if the string
    /// was removed or the cache closed meanwhile.
    async fn open(&self, claim: &Claim) -> Result<(Arc<dyn Connection>, Instant)> {
        let connection: Arc<dyn Connection> = Arc::from(self.registry.connect(&claim.key).await?);
        let lent = self.with(|state, now, deferred| {
            let entry = state.held(claim)?;
            entry.connection = Some(Arc::clone(&connection));
            entry.last_good = now;
            entry.suspect = false;
            Some(entry.lend(claim.id, now, self.settings.max_lease_time, deferred))
        });
        if let Some(expires) = lent {
            return Ok((connection, expires));
        }
        self.close_bounded(connection).await;
        Err(removed())
    }

    /// Have the connection `claim` holds replaced before the next lease.
    fn mark_broken(&self, claim: &Claim) {
        self.with(|state, _, _| {
            if let Some(entry) = state.held(claim) {
                entry.broken = true;
            }
        });
    }

    /// Close `connection`, giving up after the close timeout; the outcome is not reported.
    async fn close_bounded(&self, connection: Arc<dyn Connection>) {
        let until = after(self.clock.now(), self.settings.close_timeout);
        drop(within(&*self.clock, until, connection.close()).await);
    }
}

impl State {
    /// The entry of `claim` while `claim` holds it.
    fn held(&mut self, claim: &Claim) -> Option<&mut Entry> {
        self.entries
            .get_mut(&claim.key)
            .filter(|entry| entry.holder.is_some_and(|holder| holder.id == claim.id))
    }

    /// Remove every entry whose connection nobody has leased for the maximum idle time and
    /// return their connections to close.
    fn take_idle(&mut self, now: Instant, settings: &Settings) -> Vec<Arc<dyn Connection>> {
        let idle: Vec<String> = self
            .entries
            .iter()
            .filter(|(_, entry)| {
                entry.holder.is_none()
                    && entry.waiters.is_empty()
                    && now.saturating_duration_since(entry.last_good) >= settings.max_idle_time
            })
            .map(|(key, _)| key.clone())
            .collect();
        idle.iter()
            .filter_map(|key| self.entries.remove(key))
            .filter_map(|entry| entry.connection)
            .collect()
    }
}

impl Entry {
    /// Wake every waiter so it looks at the entry again.
    fn notify(&mut self, deferred: &mut Deferred) {
        deferred.wake.extend(self.wakers.drain());
    }

    /// Take `id` out of the queue, if it is there.
    fn leave(&mut self, id: u64, deferred: &mut Deferred) {
        if let Some(index) = self.waiters.iter().position(|waiter| waiter.id == id)
            && let Some(Waiter {
                slot: Some(slot), ..
            }) = self.waiters.remove(index)
        {
            deferred.discard.extend(self.wakers.release(slot));
        }
    }

    /// Start the lease of holder `id` and return when it expires; the waiters look again,
    /// because the instant they wait for has changed.
    fn lend(
        &mut self,
        id: u64,
        now: Instant,
        max_lease_time: Duration,
        deferred: &mut Deferred,
    ) -> Instant {
        let expires = after(now, max_lease_time);
        self.holder = Some(Holder {
            id,
            lease: Some(expires),
        });
        self.notify(deferred);
        expires
    }

    /// The entry leaves the cache: its waiters look again (and find it gone), and its
    /// connection is returned for closing.
    fn dissolve(&mut self, deferred: &mut Deferred) -> Option<Arc<dyn Connection>> {
        self.notify(deferred);
        self.connection.take()
    }
}

impl Claim {
    /// Fail with the reason the lease that expires at `expires` is no longer valid, if it
    /// is not.
    fn check(&self, operation: Operation, expires: Instant) -> Result<()> {
        self.inner.with(|state, now, _| {
            if now >= expires {
                return Err(refusal(
                    ErrorKind::Timeout,
                    operation,
                    "this lease passed the connection cache's maximum lease time",
                ));
            }
            if state.closed || state.held(self).is_none() {
                return Err(refusal(
                    ErrorKind::Disconnected,
                    operation,
                    "this lease was given back, its connection string removed or the connection \
                     cache closed",
                ));
            }
            Ok(())
        })
    }

    /// Have the connection replaced before the next lease if `error` says the link is gone.
    fn observe(&self, error: Option<&PlcError>) {
        if error.is_some_and(|error| {
            matches!(error.kind(), ErrorKind::Timeout | ErrorKind::Disconnected)
        }) {
            self.inner.mark_broken(self);
        }
    }

    /// Give the place up: as the holder, hand the connection back (good, or suspect when
    /// its lease expired); as a waiter, leave the queue. The waiters look again.
    fn release(&self) {
        self.inner.with(|state, now, deferred| {
            let Some(entry) = state.entries.get_mut(&self.key) else {
                return;
            };
            match entry.holder {
                Some(holder) if holder.id == self.id => {
                    match holder.lease {
                        Some(until) if until <= now => entry.suspect = true,
                        Some(_) => entry.last_good = now,
                        None => {}
                    }
                    entry.holder = None;
                }
                Some(_) | None => entry.leave(self.id, deferred),
            }
            entry.notify(deferred);
            if entry.holder.is_none() && entry.waiters.is_empty() && entry.connection.is_none() {
                state.entries.remove(&self.key);
            }
        });
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.release();
    }
}

/// Run `operation` until it finishes or `clock` reaches `until`, whichever is first;
/// `None` if the clock got there first. A clock whose sleep resolves before `until` cannot
/// end the race.
async fn within(
    clock: &dyn Clock,
    until: Instant,
    mut operation: OpFuture<'_, ()>,
) -> Option<Result<()>> {
    let mut sleep = Some(clock.sleep_until(until));
    poll_fn(|cx| {
        if let Poll::Ready(outcome) = operation.as_mut().poll(cx) {
            return Poll::Ready(Some(outcome));
        }
        if sleep
            .as_mut()
            .is_some_and(|sleep| sleep.as_mut().poll(cx).is_ready())
        {
            sleep = None;
        }
        if sleep.is_none() && clock.now() >= until {
            return Poll::Ready(None);
        }
        Poll::Pending
    })
    .await
}

/// `now` plus `duration`. `build` checked that every setting fits the clock, so the
/// fallback (now: the bound has passed) would only be reached by a process that ran for
/// most of the clock's range.
fn after(now: Instant, duration: Duration) -> Instant {
    now.checked_add(duration).unwrap_or(now)
}

/// A refusal before any I/O, with the operation it refused.
fn refusal(kind: ErrorKind, operation: Operation, message: impl Into<String>) -> PlcError {
    PlcError::new(kind, message).with_diagnostics(Diagnostics {
        operation: Some(operation),
        category: Some(FailureCategory::Validation),
        ..Diagnostics::default()
    })
}

fn closed() -> PlcError {
    refusal(
        ErrorKind::Disconnected,
        Operation::Connect,
        "the connection cache is closed",
    )
}

fn removed() -> PlcError {
    refusal(
        ErrorKind::Disconnected,
        Operation::Connect,
        "the connection string was removed from the connection cache, or the cache closed, \
         while the call waited",
    )
}

fn invalid(message: String) -> PlcError {
    refusal(ErrorKind::InvalidConfig, Operation::Connect, message)
}
