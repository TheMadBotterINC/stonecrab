use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::task::Poll;
use std::time::Instant;

use stonecrab_core::{
    CancellationToken, Clock, ConnectionConfig, ConnectionState, Deadline, Diagnostics, Endpoint,
    ErrorKind, FailureCategory, OpFuture, Operation, PlcError, ReconnectPolicy, ResourceLimits,
    Result, StateCell, StateWatch, Timeouts,
};
use tokio::sync::{Mutex as SlotMutex, Semaphore};

use crate::{FramedTransport, Framer, Jitter, TransportFactory};

/// What a [`ConnectionGuard`] needs to know about its connection, by value.
///
/// It is the part of a `ConnectionConfig` that a transport-level owner uses: where to
/// connect, how long to wait, how to back off and which budgets to enforce. It holds no
/// credentials, so a protocol session that authenticates keeps those itself. The fields
/// are public for assignment; [`GuardConfig::new`] supplies the defaults of
/// [`Timeouts`] and [`ReconnectPolicy`].
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
/// Upstream configures Netty bootstraps per driver; this type is a Stonecrab contract
/// with no upstream parity claim.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab_core::{Endpoint, ResourceLimits};
/// use stonecrab_transport::GuardConfig;
///
/// let limits = ResourceLimits::builder().max_pending_requests(4).build()?;
/// let mut config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
/// config.timeouts.request = Duration::from_millis(500);
/// assert_eq!(config.timeouts.connect, Duration::from_secs(5));
/// assert_eq!(config.limits.max_pending_requests().get(), 4);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct GuardConfig {
    /// The peer the guard opens, and reopens after a failure.
    pub endpoint: Endpoint,
    /// `connect` bounds every open; `request` is the default-deadline source and the
    /// bound on `close` (decision D15, ADR-0007).
    pub timeouts: Timeouts,
    /// The backoff between reopen attempts and the budget of attempts.
    pub reconnect: ReconnectPolicy,
    /// `max_pending_requests` bounds the requests the guard owns (running plus queued);
    /// the frame and buffer bounds go to every transport it opens.
    pub limits: ResourceLimits,
}

impl GuardConfig {
    /// A configuration for `endpoint` with default timeouts and reconnect policy.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{Endpoint, ReconnectPolicy, ResourceLimits, Timeouts};
    /// use stonecrab_transport::GuardConfig;
    ///
    /// let limits = ResourceLimits::builder().build()?;
    /// let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
    /// assert_eq!(config.timeouts, Timeouts::default());
    /// assert_eq!(config.reconnect, ReconnectPolicy::default());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn new(endpoint: Endpoint, limits: ResourceLimits) -> Self {
        Self {
            endpoint,
            timeouts: Timeouts::default(),
            reconnect: ReconnectPolicy::default(),
            limits,
        }
    }
}

/// Copies the endpoint, timeouts, reconnect policy and limits; the credentials and the
/// read retry policy stay with the caller.
impl From<&ConnectionConfig> for GuardConfig {
    fn from(config: &ConnectionConfig) -> Self {
        Self {
            endpoint: config.endpoint().clone(),
            timeouts: *config.timeouts(),
            reconnect: *config.reconnect(),
            limits: *config.limits(),
        }
    }
}

/// Counters a [`ConnectionGuard`] keeps for tests and diagnostics.
///
/// A snapshot from [`ConnectionGuard::stats`]; the counters only grow.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
/// A Stonecrab diagnostic with no upstream counterpart.
///
/// # Examples
/// ```
/// use stonecrab_transport::GuardStats;
///
/// let stats = GuardStats::default();
/// assert_eq!((stats.reconnects, stats.resyncs, stats.requests, stats.timeouts), (0, 0, 0, 0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct GuardStats {
    /// Transports opened after the first one: successful request-driven reopens.
    pub reconnects: u64,
    /// Times the guard ran `FramedTransport::resync` before a request, because bytes the
    /// stream could not account for were still buffered.
    pub resyncs: u64,
    /// Requests whose frame reached the transport (a request refused before I/O is not
    /// counted).
    pub requests: u64,
    /// Requests that failed with [`ErrorKind::Timeout`], whatever phase they were in.
    pub timeouts: u64,
}

#[derive(Debug, Default)]
struct Counters {
    reconnects: AtomicU64,
    resyncs: AtomicU64,
    requests: AtomicU64,
    timeouts: AtomicU64,
}

fn bump(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// The private sub-states of `Connected` (ARCHITECTURE.md section 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing is owed by the peer.
    Idle,
    /// A frame went out and its reply is awaited. Seen at the start of a request only
    /// when the request that set it was dropped.
    InFlight,
    /// A request was abandoned (timeout, cancel, framing error): its reply, or part of
    /// it, may still arrive.
    Resyncing,
}

/// Everything the single in-flight slot protects.
struct Link<F, R> {
    transport: Option<FramedTransport<F, R>>,
    book: Book,
}

/// The slot's bookkeeping, apart from the transport so that both can be borrowed at once.
struct Book {
    phase: Phase,
    /// Reconnect attempts scheduled since the last received reply.
    failures: u32,
    /// When the pending reconnect attempt may begin; `Some` exactly while `Reconnecting`.
    next_at: Option<Instant>,
    /// Whether any transport has been opened, so a later open counts as a reconnect.
    opened_before: bool,
}

/// A failure inside the slot, and whether the transport must be dropped because of it.
struct Fault {
    error: PlcError,
    lose: bool,
}

impl Fault {
    const fn keep(error: PlcError) -> Self {
        Self { error, lose: false }
    }

    const fn lose(error: PlcError) -> Self {
        Self { error, lose: true }
    }
}

/// What cut a wait short.
#[derive(Debug, Clone, Copy)]
enum Cut {
    Cancelled,
    Closing,
    Elapsed,
}

/// The context every error of one call is stamped with.
#[derive(Clone, Copy)]
struct Site {
    started: Instant,
    operation: Option<Operation>,
}

/// Puts the state back to `to` when an open is abandoned, unless something else (a
/// `close`) has moved it on from `Connecting` since.
struct Restore<'a> {
    cell: &'a StateCell,
    to: ConnectionState,
    armed: bool,
}

impl Restore<'_> {
    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        if self.armed && self.cell.current() == ConnectionState::Connecting {
            self.cell.set(self.to);
        }
    }
}

/// Owner of one connection: the lifecycle state machine, one in-flight request and a
/// bounded FIFO behind it, and request-driven reconnect with backoff.
///
/// The guard holds a [`FramedTransport`] (made by an [`TransportFactory`] and a framer
/// constructor) and offers one operation on it, [`request`](Self::request): one frame out
/// and one frame back. A protocol session (the Modbus `ModbusSession`) wraps the guard,
/// builds the frames and correlates the replies.
///
/// # The machine
/// `state()` reports core's `ConnectionState`:
///
/// | From | Event | To |
/// | --- | --- | --- |
/// | `Disconnected` | `connect` | `Connecting`, then `Connected` |
/// | `Connected` | peer closed, I/O error, send or resync failed | `Reconnecting { attempt, next_at }`, or `Failed(Disconnected)` when `is_exhausted` |
/// | `Connected` | request timed out, was cancelled or hit a framing error | stays `Connected`; the next request resyncs first |
/// | `Reconnecting` | next request or `connect` | waits until `next_at`, then `Connecting`, then `Connected` |
/// | `Connecting` | open fails with `Timeout` or `Disconnected` | `Reconnecting` with the next attempt, or `Failed(Disconnected)` when exhausted |
/// | `Connecting` | open fails with any other kind (for example `InvalidConfig`) | `Failed(kind)` |
/// | `Connecting` | the caller's token fires | back to the state before the open |
/// | any non-terminal | `close` | `Closing`, then `Closed` |
///
/// `Closed` and `Failed` are terminal: a new connection needs a new guard, and every
/// later request fails with `Disconnected` before any I/O. Inside `Connected` the guard
/// keeps three private sub-states, `Idle`, `InFlight` and `Resyncing`.
///
/// # Reconnect is request-driven (decision D15)
/// There is no background task and nothing runs between calls: a dead peer is noticed by
/// the next request. After a failure the state is `Reconnecting { attempt, next_at }`
/// with `next_at = clock.now() + reconnect.next_delay(attempt - 1, sample)` (`attempt`
/// is one-based; the sample comes from [`Jitter`] unless
/// [`with_jitter_source`](Self::with_jitter_source) replaces it). The next request (or
/// `connect`) sleeps until `next_at` on the clock, cancellably, bounded by its own
/// deadline and token, then opens the transport within `timeouts.connect` and, if that
/// works, sends. One request makes one open attempt; if the open fails the request fails
/// with the open's error and the following request waits out the next, longer backoff.
/// `reconnect.max_attempts` retries are allowed after the first failure; when the next
/// one would exceed them the state becomes `Failed(Disconnected)`. Only a received reply
/// resets the attempt count, not an open: a peer that accepts and then closes keeps
/// spending the budget. An open cut short by the caller's own earlier deadline does not
/// count as a failed attempt.
///
/// # Queue and backpressure
/// At most `limits.max_pending_requests()` requests are owned at once, the running one
/// included (a counting admission gate). One more fails with
/// [`ErrorKind::Backpressure`] before any I/O. Admitted requests take the single
/// in-flight slot in arrival order (a fair `tokio::sync::Mutex`); a queued request that
/// is cancelled, whose deadline passes, or that is overtaken by `close` leaves the queue
/// at once and frees its place.
///
/// # Deadlines
/// Every wait has a bound and a single source. `request` takes the deadline from the
/// caller; [`request_with_default_deadline`](Self::request_with_default_deadline) builds
/// `Deadline::after(clock.now(), timeouts.request)` (ADR-0007). Queueing, the backoff
/// sleep, the open (also limited by `timeouts.connect`), the send, the resync and the
/// wait for the reply all end at that deadline. `close` is bounded by
/// `Deadline::after(clock.now(), timeouts.request)`.
///
/// # Late replies
/// A request that times out, is cancelled after the frame went out, or hits a framing
/// error leaves the guard `Connected` and marks the stream as possibly owing bytes. Before
/// the next request, if the receive buffer holds bytes the guard cannot account for (half
/// a reply, garbage, an unsolicited extra frame), it runs
/// [`FramedTransport::resync`] within that request's deadline and discards the frame the
/// resync recognised; a resync that finds no boundary in time is `Disconnected` and the
/// guard reconnects. A reply that arrives whole after the guard has moved on is not
/// seen by the guard: it comes out of the next `next_frame` as that request's reply, and
/// discarding it is the session's job, by correlation (the transaction id).
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
/// Upstream has no equivalent: its Netty channel reconnect logic is outside the
/// inventoried Modbus files. This is a Stonecrab contract; see also
/// `docs/parity/transport.md#TRANSPORT-RECONNECT` and
/// `docs/parity/transport.md#TRANSPORT-QUEUE`.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab_core::{Clock, ConnectionState, Deadline, Endpoint, ResourceLimits};
/// use stonecrab_testkit::{
///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, Step, block_on_with,
/// };
/// use stonecrab_transport::{ConnectionGuard, GuardConfig};
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::strict(vec![
///     Step::Expect(vec![0, 4, b'p', b'i', b'n', b'g']),
///     Step::Reply(vec![0, 4, b'p', b'o', b'n', b'g']),
/// ]));
/// let limits = ResourceLimits::builder().build()?;
/// let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
/// let guard = ConnectionGuard::new(
///     config,
///     Arc::new(factory),
///     Box::new(|| LengthPrefixedFramer::new(64)),
///     Arc::new(clock.clone()),
/// );
///
/// block_on_with(&clock, guard.connect())?;
/// assert_eq!(guard.state(), ConnectionState::Connected);
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
/// let reply = block_on_with(&clock, guard.request(b"ping".to_vec(), deadline, None))?;
/// assert_eq!(reply, b"pong");
/// block_on_with(&clock, guard.close())?;
/// assert_eq!(guard.state(), ConnectionState::Closed);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
pub struct ConnectionGuard<F, R> {
    config: GuardConfig,
    factory: Arc<dyn TransportFactory>,
    new_framer: Box<dyn Fn() -> R + Send + Sync>,
    clock: Arc<dyn Clock>,
    jitter: Mutex<Box<dyn FnMut() -> f64 + Send>>,
    state: StateCell,
    /// Cancelled by `close`: wakes every wait the guard owns.
    closing: CancellationToken,
    /// One permit per request the guard may own, running or queued.
    admission: Semaphore,
    /// The single in-flight slot; `tokio`'s mutex queues its waiters in arrival order.
    link: SlotMutex<Link<F, R>>,
    counters: Counters,
}

/// Shows the endpoint, the state and, when the slot is free, the private sub-state.
impl<F, R> fmt::Debug for ConnectionGuard<F, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut shown = f.debug_struct("ConnectionGuard");
        shown
            .field("endpoint", &self.config.endpoint)
            .field("state", &self.state.current());
        match self.link.try_lock() {
            Ok(link) => shown.field("phase", &link.book.phase),
            Err(_busy) => shown.field("phase", &"slot in use"),
        };
        shown.finish_non_exhaustive()
    }
}

impl<F: Send, R: Framer<F>> ConnectionGuard<F, R> {
    /// A guard in the `Disconnected` state; nothing is opened until
    /// [`connect`](Self::connect).
    ///
    /// `new_framer` is called once per transport the guard opens, so every connection
    /// starts with a fresh framer. `clock` is the one source of time for deadlines,
    /// backoff and latency (use `SystemClock` in production). Backoff jitter comes from
    /// a [`Jitter`] seeded from the operating system.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim (see the type).
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, LengthPrefixedFramer};
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(guard.state(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn new(
        config: GuardConfig,
        factory: Arc<dyn TransportFactory>,
        new_framer: Box<dyn Fn() -> R + Send + Sync>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (state, _watch) = StateCell::new(ConnectionState::Disconnected);
        let mut jitter = Jitter::new();
        let admission = Semaphore::new(config.limits.max_pending_requests().get());
        Self {
            config,
            factory,
            new_framer,
            clock,
            jitter: Mutex::new(Box::new(move || jitter.next_unit())),
            state,
            closing: CancellationToken::new(),
            admission,
            link: SlotMutex::new(Link {
                transport: None,
                book: Book {
                    phase: Phase::Idle,
                    failures: 0,
                    next_at: None,
                    opened_before: false,
                },
            }),
            counters: Counters::default(),
        }
    }

    /// Draw backoff jitter samples from `source` instead of the default [`Jitter`].
    ///
    /// Each backoff takes one call, expected in `[0, 1]` (`ReconnectPolicy::next_delay`
    /// clamps it). Tests pass a constant such as `|| 0.5` for an exact schedule.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-RECONNECT
    /// A Stonecrab test seam (decision D13); upstream has no reconnect backoff.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, LengthPrefixedFramer};
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock),
    /// )
    /// .with_jitter_source(|| 0.5);
    /// # let _ = guard;
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn with_jitter_source(mut self, source: impl FnMut() -> f64 + Send + 'static) -> Self {
        self.jitter = Mutex::new(Box::new(source));
        self
    }

    /// The current lifecycle state.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, LengthPrefixedFramer};
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(guard.state(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn state(&self) -> ConnectionState {
        self.state.current()
    }

    /// An observer of the lifecycle state, with the current state counted as seen.
    ///
    /// A `StateWatch` keeps the latest value only, so transitions that happen between two
    /// polls of `changed()` are coalesced (ADR-0007); the terminal states `Closed` and
    /// `Failed` end the stream. Tests that need every transition register a waker that
    /// reads the state on each change.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, LengthPrefixedFramer};
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(guard.watch_state().current(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn watch_state(&self) -> StateWatch {
        self.state.watch()
    }

    /// A snapshot of the guard's counters.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, LengthPrefixedFramer};
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig, GuardStats};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(guard.stats(), GuardStats::default());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn stats(&self) -> GuardStats {
        GuardStats {
            reconnects: self.counters.reconnects.load(Ordering::Relaxed),
            resyncs: self.counters.resyncs.load(Ordering::Relaxed),
            requests: self.counters.requests.load(Ordering::Relaxed),
            timeouts: self.counters.timeouts.load(Ordering::Relaxed),
        }
    }

    /// Open the transport: `Disconnected` to `Connecting` to `Connected`.
    ///
    /// The open is bounded by `timeouts.connect`. Called while `Connected` it does
    /// nothing. Called while `Reconnecting` it waits out the backoff first and then opens,
    /// like a request does. A failed open with `Timeout` or `Disconnected` leaves the
    /// guard `Reconnecting` (or `Failed(Disconnected)` when the retry budget is spent);
    /// any other kind leaves it `Failed(kind)`. The deadline source is
    /// `timeouts.connect` (the backoff wait is the policy's own, and `close` interrupts
    /// it).
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-RECONNECT
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if the open did not finish within `timeouts.connect`.
    /// - [`ErrorKind::Disconnected`] if the peer refused or the guard is `Closing`,
    ///   `Closed` or `Failed`.
    /// - [`ErrorKind::InvalidConfig`] if `timeouts.connect` does not fit the clock's range,
    ///   and any other kind the factory reports (`Authentication`, ...).
    ///
    /// Failures carry `Diagnostics` with `operation = Connect`, the state at the failure,
    /// the latency on the guard's clock and the reconnect attempt as `retries`.
    ///
    /// # Cancel safety
    /// Cancel-safe. A dropped call leaves no transport behind and puts the state back to
    /// what it was before the open began.
    ///
    /// # Wire effects
    /// Opens a connection and sends no application bytes. The open is bounded by the
    /// smaller of `timeouts.connect` and the connect deadline.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{
    ///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, block_on_with,
    /// };
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(factory),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, guard.connect())?;
    /// assert_eq!(guard.state(), ConnectionState::Connected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn connect(&self) -> OpFuture<'_, ()> {
        Box::pin(self.open_link())
    }

    /// Send one frame and return the next frame the peer sends.
    ///
    /// This is the only exchange the guard offers; the caller (the protocol session)
    /// builds the frame and checks that the reply answers it. The steps: refuse a
    /// terminal state, a cancelled token or an elapsed deadline before any I/O; claim a
    /// place (a full guard is `Backpressure`); wait in line for the in-flight slot; if the
    /// transport is gone wait out the backoff and reopen it; resync if the previous
    /// request left bytes behind; send the frame and wait for the reply, all within
    /// `deadline`.
    ///
    /// The deadline source is the `deadline` argument: build it from the guard's clock, or
    /// use [`request_with_default_deadline`](Self::request_with_default_deadline). The
    /// token is honoured while queued, during the backoff, during the open and while
    /// waiting for the reply; a frame that is being written is written to the end (a
    /// half-written frame would poison the transport), and a token that fired meanwhile
    /// takes effect right after it.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// - [`ErrorKind::Backpressure`] if `max_pending_requests` requests are already owned;
    ///   nothing was sent.
    /// - [`ErrorKind::Cancelled`] if `token` fired: nothing was sent when it fired while
    ///   queued, during the backoff, the open or before the send; the frame may have
    ///   reached the peer when it fired while waiting for the reply.
    /// - [`ErrorKind::Timeout`] if `deadline` had passed or passed while queued, in the
    ///   backoff, the open, the send or the wait for the reply. After the frame went out
    ///   the reply may still come; the next request resyncs as described on the type.
    /// - [`ErrorKind::Disconnected`] if the guard is not open (never connected, `Closing`,
    ///   `Closed`, `Failed`), the peer closed, the send or the resync failed, or the open
    ///   was refused. The guard is then `Reconnecting`, or `Failed(Disconnected)` when
    ///   the retry budget is spent.
    /// - [`ErrorKind::ResourceLimit`] or [`ErrorKind::MalformedData`] if the frame cannot
    ///   be encoded within the frame limit (nothing sent), or the reply violates the
    ///   framer (the next request resyncs).
    ///
    /// Every error carries `Diagnostics` with the state at the failure, the latency on
    /// the guard's clock, the reconnect attempt as `retries` and a category; `operation`
    /// is `Connect` for a failed open and is left unset otherwise, because the guard
    /// cannot tell a read from a write (the session sets it).
    ///
    /// # Cancel safety
    /// Cancel-safe in the guard's own state: a dropped future releases the slot and the
    /// queue place, and the next request treats the stream as possibly owing a reply. If
    /// the future is dropped while the frame is being written, the transport is poisoned
    /// and the next request fails with `Disconnected` and reconnects.
    ///
    /// # Wire effects
    /// Bounded by the caller's `Deadline`. Transmits one frame once it holds the slot. The frame is never sent twice, and a
    /// request that fails after the send is an unknown outcome for a write: the guard
    /// does not retry or replay (coding standards section 6.8).
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use stonecrab_core::{Clock, Deadline, Endpoint, ErrorKind, ResourceLimits};
    /// use stonecrab_testkit::{
    ///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, block_on_with,
    /// };
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard: ConnectionGuard<Vec<u8>, LengthPrefixedFramer> = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock.clone()),
    /// );
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
    /// // Not connected yet: refused before any I/O.
    /// let error = block_on_with(&clock, guard.request(b"ping".to_vec(), deadline, None))
    ///     .unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::Disconnected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn request<'a>(
        &'a self,
        frame: F,
        deadline: Deadline,
        token: Option<&'a CancellationToken>,
    ) -> OpFuture<'a, F> {
        Box::pin(async move {
            let result = self.exchange(Some(frame), deadline, token).await;
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == ErrorKind::Timeout)
            {
                bump(&self.counters.timeouts);
            }
            result
        })
    }

    /// Read one further frame from the peer without sending anything.
    ///
    /// For a session that got a reply to an earlier exchange (a late reply after a
    /// timeout or a cancel) and must drop it by correlation: it calls this until the
    /// frame it wants arrives, all within one deadline. It takes the in-flight slot like
    /// [`request`](Self::request) does, never reopens the connection (a reply owed by a
    /// connection that is gone will not come) and does not count as a request.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// As [`request`](Self::request), except that nothing is ever sent and a lost
    /// transport is [`ErrorKind::Disconnected`] without a reopen. A timeout or a
    /// framing error leaves the guard `Connected` and the next request resyncs.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped future releases the slot and the queue place and loses no
    /// buffered frame.
    ///
    /// # Wire effects
    /// None: the call only reads.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use stonecrab_core::{Clock, Deadline, Endpoint, ErrorKind, ResourceLimits};
    /// use stonecrab_testkit::{
    ///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, block_on_with,
    /// };
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard: ConnectionGuard<Vec<u8>, LengthPrefixedFramer> = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(factory),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, guard.connect())?;
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
    /// // The peer sends nothing: the read times out.
    /// let error = block_on_with(&clock, guard.next_reply(deadline, None)).unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::Timeout);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn next_reply<'a>(
        &'a self,
        deadline: Deadline,
        token: Option<&'a CancellationToken>,
    ) -> OpFuture<'a, F> {
        Box::pin(async move {
            let result = self.exchange(None, deadline, token).await;
            if result
                .as_ref()
                .is_err_and(|error| error.kind() == ErrorKind::Timeout)
            {
                bump(&self.counters.timeouts);
            }
            result
        })
    }

    /// [`request`](Self::request) with the default deadline: `timeouts.request` from now
    /// on the guard's clock (ADR-0007).
    ///
    /// The deadline is fixed when the returned future is first polled, so the time spent
    /// before that is not counted.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// As [`request`](Self::request), plus [`ErrorKind::InvalidConfig`] if
    /// `timeouts.request` does not fit the clock's range.
    ///
    /// # Cancel safety
    /// As [`request`](Self::request).
    ///
    /// # Wire effects
    /// As [`request`](Self::request), bounded by
    /// `Deadline::after(clock.now(), timeouts.request)`.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{Clock, Endpoint, ErrorKind, ResourceLimits};
    /// use stonecrab_testkit::{
    ///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, Step, block_on_with,
    /// };
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::new(vec![Step::Expect(vec![0, 1, b'x'])])); // never answers
    /// let limits = ResourceLimits::builder().build()?;
    /// let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
    /// let request_timeout = config.timeouts.request;
    /// let guard = ConnectionGuard::new(
    ///     config,
    ///     Arc::new(factory),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, guard.connect())?;
    /// let started = clock.now();
    /// let error = block_on_with(&clock, guard.request_with_default_deadline(b"x".to_vec(), None))
    ///     .unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::Timeout);
    /// assert_eq!(clock.now() - started, request_timeout);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn request_with_default_deadline<'a>(
        &'a self,
        frame: F,
        token: Option<&'a CancellationToken>,
    ) -> OpFuture<'a, F> {
        Box::pin(async move {
            let site = Site {
                started: self.clock.now(),
                operation: None,
            };
            let Ok(deadline) = Deadline::after(site.started, self.config.timeouts.request) else {
                return Err(self.error(
                    ErrorKind::InvalidConfig,
                    "timeouts.request does not fit the clock's range",
                    &site,
                    self.attempt_hint(),
                    FailureCategory::Validation,
                ));
            };
            self.request(frame, deadline, token).await
        })
    }

    /// Close the connection: fail what is waiting, shut the transport down, end `Closed`.
    ///
    /// The state becomes `Closing` at once and every wait the guard owns ends: the
    /// in-flight request, the queued ones, a backoff sleep and an open in progress all
    /// fail with `Disconnected`. Then the guard takes the slot, shuts the transport down
    /// (a half-close; no application bytes) and sets `Closed`. A resync that was running
    /// is one of those waits and is abandoned. Everything is bounded by
    /// `Deadline::after(clock.now(), timeouts.request)`: if the slot is not free or the
    /// shutdown not done by then, the state is `Closed` anyway and the call reports
    /// `Timeout`; whoever still holds the transport drops it. Closing a guard that is
    /// already `Closed` or `Failed` does nothing, so the call is idempotent. A
    /// `timeouts.request` the clock cannot represent gives a bound that has already
    /// passed, so the shutdown is attempted once and `close` never waits.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim (see the type).
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if the slot or the shutdown was not finished by the
    ///   deadline. The guard is `Closed` regardless.
    ///
    /// A peer that already went away is not an error. Failures carry `Diagnostics` with
    /// `operation = Close`.
    ///
    /// # Cancel safety
    /// Cancel-safe in the sense that the state is already `Closing`; a repeat call
    /// finishes the job.
    ///
    /// # Wire effects
    /// Half-closes the transport (a FIN for TCP). A request that was in flight is an
    /// unknown outcome if it was a write.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab_testkit::{
    ///     FakeClock, FakeTransportFactory, LengthPrefixedFramer, Script, block_on_with,
    /// };
    /// use stonecrab_transport::{ConnectionGuard, GuardConfig};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let probes = factory.clone();
    /// let limits = ResourceLimits::builder().build()?;
    /// let guard = ConnectionGuard::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(factory),
    ///     Box::new(|| LengthPrefixedFramer::new(64)),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, guard.connect())?;
    /// block_on_with(&clock, guard.close())?;
    /// block_on_with(&clock, guard.close())?; // idempotent
    /// assert_eq!(guard.state(), ConnectionState::Closed);
    /// assert!(probes.probes()[0].is_shut_down());
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(self.shut_down())
    }
}

// The lifecycle itself.
impl<F: Send, R: Framer<F>> ConnectionGuard<F, R> {
    /// `connect`: take the slot and open the transport if there is none.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the slot guard is the in-flight slot and is held until the call ends"
    )]
    async fn open_link(&self) -> Result<()> {
        let site = Site {
            started: self.clock.now(),
            operation: Some(Operation::Connect),
        };
        self.refuse_terminal(&site)?;
        let mut guard = self.take_slot(&site, None, None).await?;
        let link = &mut *guard;
        self.refuse_terminal(&site)?;
        if link.transport.is_none() {
            let opened = self.reopen(&mut link.book, &site, None, None).await?;
            link.transport = Some(opened);
        }
        Ok(())
    }

    /// `request`: validate, queue, (re)open, then drive the exchange.
    #[expect(
        clippy::significant_drop_tightening,
        reason = "the slot guard is the in-flight slot and is held until the request ends"
    )]
    async fn exchange(
        &self,
        frame: Option<F>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
    ) -> Result<F> {
        let site = Site {
            started: self.clock.now(),
            operation: None,
        };
        self.gate(&site, deadline, token)?;
        let Ok(_place) = self.admission.try_acquire() else {
            return Err(self.error(
                ErrorKind::Backpressure,
                "the guard already owns max_pending_requests requests",
                &site,
                self.attempt_hint(),
                FailureCategory::Validation,
            ));
        };
        let mut guard = self.take_slot(&site, Some(deadline), token).await?;
        let link = &mut *guard;
        // Time passed in the queue: look again.
        self.gate(&site, deadline, token)?;
        let framed = match link.transport.take() {
            Some(framed) => framed,
            // A further reply belongs to the connection that is gone.
            None if frame.is_none() => {
                return Err(self.error(
                    ErrorKind::Disconnected,
                    "the connection that owed the reply is gone",
                    &site,
                    self.attempt_hint(),
                    FailureCategory::Transport,
                ));
            }
            None => {
                self.reopen(&mut link.book, &site, Some(deadline), token)
                    .await?
            }
        };
        let framed = link.transport.insert(framed);
        let driven = self
            .drive(framed, &mut link.book, frame, deadline, token, &site)
            .await;
        driven.map_err(|fault| {
            if fault.lose {
                link.transport = None;
                self.schedule_reconnect(&mut link.book);
            }
            fault.error
        })
    }

    /// Resync if the stream owes bytes, send the frame, wait for the reply.
    async fn drive(
        &self,
        framed: &mut FramedTransport<F, R>,
        book: &mut Book,
        frame: Option<F>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
        site: &Site,
    ) -> std::result::Result<F, Fault> {
        if let Some(frame) = frame {
            self.realign(framed, book, deadline, token, site).await?;
            self.transmit(framed, book, frame, deadline, site).await?;
        }
        self.await_reply(framed, book, deadline, token, site).await
    }

    /// Before a request: if bytes of an abandoned exchange are buffered, find the next
    /// frame boundary and drop the frame found there.
    async fn realign(
        &self,
        framed: &mut FramedTransport<F, R>,
        book: &mut Book,
        deadline: Deadline,
        token: Option<&CancellationToken>,
        site: &Site,
    ) -> std::result::Result<(), Fault> {
        if framed.buffered_len() > 0 {
            bump(&self.counters.resyncs);
            match self.until(framed.resync(deadline), token, None, true).await {
                Ok(Ok(())) => {
                    // `resync` holds the frame it recognised, so this returns it at once
                    // even with an elapsed deadline; it is the abandoned reply or debris.
                    drop(framed.next_frame(Deadline::at(self.clock.now())).await);
                }
                Ok(Err(error)) => {
                    return Err(Fault::lose(self.annotate(error, site, book.failures)));
                }
                Err(cut) => {
                    let error = self.cut(cut, site, book.failures, FailureCategory::Transport);
                    return Err(Fault::keep(error));
                }
            }
        }
        book.phase = Phase::Idle;
        Ok(())
    }

    /// Send the frame within the deadline. The token is not consulted while the frame is
    /// written, because a half-written frame poisons the transport.
    async fn transmit(
        &self,
        framed: &mut FramedTransport<F, R>,
        book: &mut Book,
        frame: F,
        deadline: Deadline,
        site: &Site,
    ) -> std::result::Result<(), Fault> {
        // Nothing is written after the deadline: the stream stays intact and trusted.
        if deadline.is_elapsed(self.clock.now()) {
            book.phase = Phase::Idle;
            return Err(Fault::keep(self.error(
                ErrorKind::Timeout,
                "the request deadline passed before the frame was sent",
                site,
                book.failures,
                FailureCategory::Validation,
            )));
        }
        // Encoding happens here, so the future below does not borrow the frame.
        let sending = framed.send_frame(&frame, deadline);
        drop(frame);
        book.phase = Phase::InFlight;
        match self.until(sending, None, None, true).await {
            Ok(Ok(())) => {
                bump(&self.counters.requests);
                Ok(())
            }
            Ok(Err(error)) => {
                // A frame the framer refused never reached the transport; any other
                // failure may have left half a frame on the wire.
                let refused = matches!(
                    error.kind(),
                    ErrorKind::ResourceLimit | ErrorKind::MalformedData
                );
                let error = self.annotate(error, site, book.failures);
                if refused {
                    book.phase = Phase::Idle;
                    Err(Fault::keep(error))
                } else {
                    Err(Fault::lose(error))
                }
            }
            Err(cut) => Err(self.stopped(cut, book, site)),
        }
    }

    /// Wait for the reply frame, within the deadline and unless the token fires. A token
    /// that fired while the frame was being written is seen first, before the wait.
    async fn await_reply(
        &self,
        framed: &mut FramedTransport<F, R>,
        book: &mut Book,
        deadline: Deadline,
        token: Option<&CancellationToken>,
        site: &Site,
    ) -> std::result::Result<F, Fault> {
        match self
            .until(framed.next_frame(deadline), token, None, true)
            .await
        {
            Ok(Ok(reply)) => {
                // Only a completed exchange proves the link: an open that is followed by
                // a close must keep spending the retry budget.
                book.phase = Phase::Idle;
                book.failures = 0;
                Ok(reply)
            }
            Ok(Err(error)) => {
                let abandoned = matches!(
                    error.kind(),
                    ErrorKind::Timeout | ErrorKind::ResourceLimit | ErrorKind::MalformedData
                );
                let error = self.annotate(error, site, book.failures);
                if abandoned {
                    // The stream is intact, but a reply (or the rest of one) may follow.
                    book.phase = Phase::Resyncing;
                    Err(Fault::keep(error))
                } else {
                    Err(Fault::lose(error))
                }
            }
            Err(cut) => Err(self.stopped(cut, book, site)),
        }
    }

    /// A send or a reply wait that was cut short: the frame is on the wire (or may be), so
    /// the stream may owe a reply, but it is intact.
    fn stopped(&self, cut: Cut, book: &mut Book, site: &Site) -> Fault {
        if matches!(cut, Cut::Cancelled) {
            book.phase = Phase::Resyncing;
        }
        Fault::keep(self.cut(cut, site, book.failures, FailureCategory::Transport))
    }

    /// Wait out the backoff, then open a transport within `timeouts.connect` (and the
    /// caller's deadline). The state ends `Connected`, `Reconnecting`, `Failed`, or as it
    /// was before when the caller gave up.
    async fn reopen(
        &self,
        book: &mut Book,
        site: &Site,
        deadline: Option<Deadline>,
        token: Option<&CancellationToken>,
    ) -> Result<FramedTransport<F, R>> {
        if let Some(next_at) = book.next_at
            && self.clock.now() < next_at
        {
            let nap = self.clock.sleep_until(next_at);
            if let Err(cut) = self.until(nap, token, deadline, true).await {
                return Err(self.cut(cut, site, book.failures, FailureCategory::Transport));
            }
        }
        let bound = Deadline::after(self.clock.now(), self.config.timeouts.connect);
        let Ok(limit) = bound else {
            // A connect timeout the clock cannot represent can never work: no retry helps.
            let error = self.error(
                ErrorKind::InvalidConfig,
                "timeouts.connect does not fit the clock's range",
                site,
                book.failures,
                FailureCategory::Validation,
            );
            book.next_at = None;
            self.state
                .set(ConnectionState::Failed(ErrorKind::InvalidConfig));
            return Err(error);
        };
        // A timeout that the caller's own earlier deadline caused says nothing about the
        // peer, so it does not spend the reconnect budget.
        let caller_bound = deadline.is_some_and(|caller| caller < limit);
        let limit = deadline.map_or(limit, |caller| limit.min(caller));
        let mut restore = Restore {
            cell: &self.state,
            to: self.state.current(),
            armed: true,
        };
        self.state.set(ConnectionState::Connecting);
        let opening = self
            .factory
            .open(&self.config.endpoint, self.config.limits, limit);
        match self.until(opening, token, None, true).await {
            Ok(Ok(transport)) => {
                book.phase = Phase::Idle;
                book.next_at = None;
                if book.opened_before {
                    bump(&self.counters.reconnects);
                }
                book.opened_before = true;
                restore.disarm();
                self.state.set(ConnectionState::Connected);
                Ok(FramedTransport::new(transport, (self.new_framer)()))
            }
            Ok(Err(error)) => {
                let error = self.annotate(error, site, book.failures);
                if caller_bound && error.kind() == ErrorKind::Timeout {
                    return Err(error);
                }
                restore.disarm();
                if matches!(error.kind(), ErrorKind::Timeout | ErrorKind::Disconnected) {
                    self.schedule_reconnect(book);
                } else {
                    book.next_at = None;
                    self.state.set(ConnectionState::Failed(error.kind()));
                }
                Err(error)
            }
            Err(cut) => Err(self.cut(cut, site, book.failures, FailureCategory::Transport)),
        }
    }

    /// The transport is gone: `Reconnecting` with the next backoff, or `Failed` when the
    /// retry budget is spent.
    fn schedule_reconnect(&self, book: &mut Book) {
        let retry = book.failures;
        if self.config.reconnect.is_exhausted(retry) {
            book.next_at = None;
            self.state
                .set(ConnectionState::Failed(ErrorKind::Disconnected));
            return;
        }
        let delay = self.config.reconnect.next_delay(retry, self.sample());
        let now = self.clock.now();
        let next_at = now.checked_add(delay).unwrap_or(now);
        book.failures = retry.saturating_add(1);
        book.next_at = Some(next_at);
        self.state.set(ConnectionState::Reconnecting {
            attempt: book.failures,
            next_at,
        });
    }

    /// `close`: see [`close`](Self::close).
    async fn shut_down(&self) -> Result<()> {
        let site = Site {
            started: self.clock.now(),
            operation: Some(Operation::Close),
        };
        if matches!(
            self.state.current(),
            ConnectionState::Closed | ConnectionState::Failed(_)
        ) {
            return Ok(());
        }
        self.closing.cancel();
        if self.state.current() != ConnectionState::Closing {
            self.state.set(ConnectionState::Closing);
        }
        let deadline = Deadline::after(site.started, self.config.timeouts.request)
            .unwrap_or(Deadline::at(site.started));
        // The slot and the shutdown share one bound, so a holder that never lets go and a
        // transport that never finishes are both cut off at the deadline.
        let outcome = match self
            .until(self.finish(deadline, &site), None, Some(deadline), false)
            .await
        {
            Ok(done) => done,
            Err(cut) => Err(self.cut(cut, &site, self.attempt_hint(), FailureCategory::Transport)),
        };
        if self.state.current() != ConnectionState::Closed {
            self.state.set(ConnectionState::Closed);
        }
        outcome
    }

    /// Take the slot and shut the transport down. A peer that is already gone is not an
    /// error.
    async fn finish(&self, deadline: Deadline, site: &Site) -> Result<()> {
        let mut guard = self.link.lock().await;
        guard.book.next_at = None;
        let Some(framed) = guard.transport.take() else {
            return Ok(());
        };
        let mut transport = framed.into_inner();
        match transport.shutdown(deadline).await {
            Err(error) if error.kind() != ErrorKind::Disconnected => {
                Err(self.annotate(error, site, guard.book.failures))
            }
            Ok(()) | Err(_) => Ok(()),
        }
    }

    /// Wait for the in-flight slot, in line, unless the token fires, the deadline passes or
    /// the guard closes.
    async fn take_slot(
        &self,
        site: &Site,
        deadline: Option<Deadline>,
        token: Option<&CancellationToken>,
    ) -> Result<tokio::sync::MutexGuard<'_, Link<F, R>>> {
        self.until(self.link.lock(), token, deadline, true)
            .await
            .map_err(|cut| self.cut(cut, site, self.attempt_hint(), FailureCategory::Validation))
    }

    /// Drive `work`, but give up when the guard starts closing (if `honor_close`), the
    /// token fires or the deadline passes on the guard's clock. The signals are polled
    /// before the work and the timer after it, so a cancellation wins a tie and finished
    /// work wins against the deadline.
    async fn until<T>(
        &self,
        work: impl Future<Output = T>,
        token: Option<&CancellationToken>,
        deadline: Option<Deadline>,
        honor_close: bool,
    ) -> std::result::Result<T, Cut> {
        let mut work = pin!(work);
        let mut closing = honor_close.then(|| self.closing.cancelled());
        let mut cancelled = token.map(CancellationToken::cancelled);
        let mut timer = deadline.map(|limit| self.clock.sleep_until(limit.instant()));
        poll_fn(|cx| {
            if let Some(closing) = closing.as_mut()
                && closing.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(Err(Cut::Closing));
            }
            if let Some(cancelled) = cancelled.as_mut()
                && cancelled.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(Err(Cut::Cancelled));
            }
            if let Poll::Ready(done) = work.as_mut().poll(cx) {
                return Poll::Ready(Ok(done));
            }
            // A clock whose sleep fails could only end the wait early, so the result is
            // not read.
            if let Some(timer) = timer.as_mut()
                && timer.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(Err(Cut::Elapsed));
            }
            Poll::Pending
        })
        .await
    }

    /// The next jitter sample.
    fn sample(&self) -> f64 {
        let mut source = self.jitter.lock().unwrap_or_else(PoisonError::into_inner);
        (*source)()
    }

    /// The checks before every request: open for business, not cancelled, in time.
    fn gate(
        &self,
        site: &Site,
        deadline: Deadline,
        token: Option<&CancellationToken>,
    ) -> Result<()> {
        self.refuse_terminal(site)?;
        if self.state.current() == ConnectionState::Disconnected {
            return Err(self.error(
                ErrorKind::Disconnected,
                "the connection is not open; call connect first",
                site,
                0,
                FailureCategory::Transport,
            ));
        }
        if token.is_some_and(CancellationToken::is_cancelled) {
            return Err(self.error(
                ErrorKind::Cancelled,
                "the request was cancelled before it started",
                site,
                self.attempt_hint(),
                FailureCategory::Validation,
            ));
        }
        if deadline.is_elapsed(self.clock.now()) {
            return Err(self.error(
                ErrorKind::Timeout,
                "the request deadline had already elapsed",
                site,
                self.attempt_hint(),
                FailureCategory::Validation,
            ));
        }
        Ok(())
    }

    /// Fail with `Disconnected` when the state is terminal or ending.
    fn refuse_terminal(&self, site: &Site) -> Result<()> {
        let why = match self.state.current() {
            ConnectionState::Closing => "the connection is closing",
            ConnectionState::Closed => "the connection is closed",
            ConnectionState::Failed(_) => "the connection has failed and cannot be reused",
            ConnectionState::Disconnected
            | ConnectionState::Connecting
            | ConnectionState::Connected
            | ConnectionState::Reconnecting { .. }
            | _ => return Ok(()),
        };
        Err(self.error(
            ErrorKind::Disconnected,
            why,
            site,
            self.attempt_hint(),
            FailureCategory::Transport,
        ))
    }

    /// The reconnect attempt the state names, for errors raised outside the slot.
    fn attempt_hint(&self) -> u32 {
        if let ConnectionState::Reconnecting { attempt, .. } = self.state.current() {
            attempt
        } else {
            0
        }
    }

    /// The error for a wait that was cut short.
    fn cut(&self, cut: Cut, site: &Site, retries: u32, category: FailureCategory) -> PlcError {
        match cut {
            Cut::Cancelled => self.error(
                ErrorKind::Cancelled,
                "the call was cancelled while it waited",
                site,
                retries,
                category,
            ),
            Cut::Closing => self.error(
                ErrorKind::Disconnected,
                "the connection is closing",
                site,
                retries,
                FailureCategory::Transport,
            ),
            Cut::Elapsed => self.error(
                ErrorKind::Timeout,
                "the deadline passed while the call waited",
                site,
                retries,
                category,
            ),
        }
    }

    /// A new error stamped with the diagnostics of this call.
    fn error(
        &self,
        kind: ErrorKind,
        detail: &str,
        site: &Site,
        retries: u32,
        category: FailureCategory,
    ) -> PlcError {
        PlcError::new(kind, format!("connection guard: {detail}")).with_diagnostics(self.stamp(
            Diagnostics::default(),
            site,
            retries,
            category,
        ))
    }

    /// An error from below (transport, framing), keeping its kind, message and source and
    /// completing its diagnostics.
    fn annotate(&self, error: PlcError, site: &Site, retries: u32) -> PlcError {
        let base = error.diagnostics().clone();
        error.with_diagnostics(self.stamp(base, site, retries, FailureCategory::Transport))
    }

    /// Fill in what the guard knows: the operation if the error lacks one, the state now,
    /// the latency on the guard's clock and the reconnect attempt. A category already
    /// set is kept.
    fn stamp(
        &self,
        mut diagnostics: Diagnostics,
        site: &Site,
        retries: u32,
        category: FailureCategory,
    ) -> Diagnostics {
        diagnostics.operation = diagnostics.operation.or(site.operation);
        diagnostics.connection_state = Some(self.state.current());
        diagnostics.latency = Some(self.clock.now().saturating_duration_since(site.started));
        diagnostics.retries = retries;
        diagnostics.category = diagnostics.category.or(Some(category));
        diagnostics
    }
}
