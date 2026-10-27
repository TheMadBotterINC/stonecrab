//! `SimConnection`: one simulated link to a [`SimDevice`], on the connection's own clock.

use std::any::Any;
use std::fmt;
use std::future::poll_fn;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::Poll;
use std::time::{Duration, Instant};

use stonecrab_core::{
    AccessPolicy, CancellationToken, Capabilities, Clock, Connection, ConnectionConfig,
    ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, ItemError, OpFuture,
    Operation, PlcError, PlcValue, ProtocolStatus, ReadItem, ReadRequest, ReadResponse,
    ReadResponseBuilder, ResourceLimits, Result, RetryPolicy, StateCell, StateWatch,
    SubscriptionMode, Tag, TagId, Timeouts, UnknownReason, WriteItem, WriteOutcome, WriteRequest,
    WriteResponse, WriteResponseBuilder, WriteResult,
};
use tracing::Instrument;

use crate::{FailureStep, SimAddress, SimArea, SimDevice};

/// The status code of a simulated device that has no tag of the requested name (the
/// Modbus "illegal data address" code, which is the closest thing to it).
const UNSET_TAG_CODE: u32 = 2;

/// The status code a `PartialFailure` step gives the items it names.
const PARTIAL_FAILURE_CODE: u32 = 4;

/// What a `SimConnection` is attached to, resolved by the driver.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct Target {
    /// The registered device name, used for tracing and nothing else.
    pub(crate) name: String,
    pub(crate) device: Arc<SimDevice>,
    /// The per-operation delay: the `latency-ms` option, or the device's own setting.
    pub(crate) latency: Duration,
    /// The `seed` option; applied to the device when the connection becomes `Connected`.
    pub(crate) seed: Option<u64>,
}

/// One simulated link to one [`SimDevice`].
///
/// A `SimConnection` is what `SimDriver::connect` returns (as a `Box<dyn Connection>`). It
/// reads and writes the device's tags through the [`Connection`] contract and owns, on top
/// of the device:
///
/// - **A lifecycle** observed through [`Connection::watch_state`]:
///   `Disconnected` -> `Connecting` -> `Connected` -> `Closing` -> `Closed`. A scripted
///   [`FailureStep::Disconnect`] moves a connected connection to
///   `Failed(ErrorKind::Disconnected)`, from which every operation fails with
///   `Disconnected` and `close` still reaches `Closed`.
/// - **One in-flight operation** (`Capabilities.concurrency` is 1). A second read or write
///   that starts while another is in flight fails at once with
///   [`ErrorKind::Backpressure`], before any device access. There is no wait queue: the
///   queue length is zero. A rejected operation never retries.
/// - **Time on the injected clock.** Every operation waits once, on the connection's
///   [`Clock`], for the device latency (zero unless the device or the connection string
///   sets one), racing the wait against the request's cancellation token, its deadline
///   and the connection being closed. With a `RealtimeClock` the wait resolves at once;
///   with the testkit's `FakeClock` the test decides when time passes.
/// - **The default-deadline rule.** A request without a deadline gets
///   `Deadline::after(clock.now(), timeouts.request)`.
/// - **A failure script.** The device's [`FailureScript`](crate::FailureScript) is
///   consumed one [`FailureStep`] per connect, read or write, at the point where the
///   operation would touch the device (after every pre-I/O check, so a rejected request
///   leaves its step queued). `close` ignores the script. The steps are the simulator's
///   fault injection; their semantics are documented on [`FailureStep`].
/// - **Diagnostics on every error**: the operation, the connection state at the failure,
///   the elapsed time on the clock, the retries made and a category (`Validation` for a
///   rejection before the exchange begins, `Protocol` for what the device refused,
///   `Transport` for `Timeout` and `Disconnected`, `Internal` for the rest, a
///   cancellation while waiting among them).
/// - **`tracing` spans** named `connect`, `read`, `write` and `close` with the fields
///   `scheme`, `endpoint` (the device name; nothing else is in it), `op` and `request_id`
///   (a per-connection counter that starts at 0 for `connect` and increases by one per
///   span).
///
/// # Reads
/// Each tag is served on its own: `STATE` reads call [`SimDevice::serve_read`], `RANDOM`
/// reads call [`SimDevice::random`], and a `SINK` read is a per-item
/// [`ErrorKind::Unsupported`] error because `SINK` is write-only. A `STATE` name that was
/// never set is a per-item [`ErrorKind::InvalidAddress`] error whose status is
/// `ProtocolStatus::new("sim", 2)`. The call fails as a whole only when the request is
/// rejected or the exchange is interrupted; `sampled_at` is the clock reading taken for
/// each item, `source_time` is always `None` (the simulator has no device timestamp) and
/// `split_into` is 1 (the simulator never splits a batch: a request over the device's
/// item limit is rejected with [`ErrorKind::ResourceLimit`] instead).
///
/// A read is retried as a whole, never per item, under the configuration's
/// [`RetryPolicy`] and only for kinds in its retry set. The default policy makes one
/// attempt. An explicit request deadline is shared by all attempts; a request without one
/// gets `timeouts.request` afresh for each attempt (the first attempt's deadline is the
/// default-deadline rule's). Each attempt takes its own step of the failure script, so a
/// script of `Timeout` steps drives the retry loop.
///
/// # Writes
/// A connection whose access policy is read-only refuses every write before anything
/// else. Otherwise a `SINK` write is counted (see [`SimDevice::sink_writes`]) and
/// discarded and is `Confirmed`; a `STATE` write to a writable tag replaces its value and is
/// `Confirmed`; a `STATE` write to a tag that is not writable, to an unset name or with a
/// value whose type differs from the stored one, and any `RANDOM` write, is `Rejected` with
/// an item error. A write is never retried. The device is changed only after the latency
/// has elapsed, so a timeout, a cancellation or a close during the wait means nothing was
/// applied. The `Unknown` outcome that follows a transmitted write comes from the scripted
/// [`FailureStep::UnknownAfterTransmit`].
///
/// # Compatibility: docs/parity/sim.md#SIM-CONNECTION
/// Stonecrab-owned. The upstream simulated driver has no lifecycle, deadline, cancellation
/// or concurrency model to compare with.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{ConnectionState, DriverRegistry, PlcValue, ReadRequest, Tag, TagId};
/// use stonecrab_driver_sim::{SimAddress, SimDevice, SimDriver, SimTag};
/// use stonecrab_testkit::{FakeClock, block_on_with};
///
/// let clock = Arc::new(FakeClock::default());
/// let device = SimDevice::new();
/// device.ramp("count", PlcValue::U16(0), PlcValue::U16(1))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new().with_clock(Arc::<FakeClock>::clone(&clock)).with_device("bench", device)?,
/// ))?;
///
/// let connection = block_on_with(&clock, registry.connect("sim://bench"))?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
///
/// let tag = Tag::new(TagId::new("count")?, Arc::new(SimAddress::parse("STATE/count:u16")?));
/// let request = ReadRequest::builder(connection.limits()).tag(tag).build()?;
/// let response = block_on_with(&clock, connection.read(request))?;
/// let value = response.items()[0].outcome().as_ref().ok().cloned();
/// assert_eq!(value, Some(PlcValue::U16(0)));
///
/// block_on_with(&clock, connection.close())?;
/// assert_eq!(connection.state(), ConnectionState::Closed);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
pub struct SimConnection {
    clock: Arc<dyn Clock>,
    device: Arc<SimDevice>,
    name: String,
    latency: Duration,
    seed: Option<u64>,
    timeouts: Timeouts,
    retry: RetryPolicy,
    access: AccessPolicy,
    limits: ResourceLimits,
    capabilities: Capabilities,
    state: StateCell,
    /// Cancelled when `close` starts: ends an operation that is waiting.
    shutdown: CancellationToken,
    /// True while a read or write is in flight; see [`Slot`].
    in_flight: AtomicBool,
    next_request_id: AtomicU64,
}

impl fmt::Debug for SimConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimConnection")
            .field("device", &self.name)
            .field("state", &self.state.current())
            .field("latency", &self.latency)
            .finish_non_exhaustive()
    }
}

impl SimConnection {
    /// A connection in the `Disconnected` state; [`establish`](Self::establish) opens it.
    pub(crate) fn new(clock: Arc<dyn Clock>, target: Target, config: &ConnectionConfig) -> Self {
        let mut capabilities = Capabilities::default();
        capabilities.read = true;
        capabilities.write = true;
        capabilities.subscribe = SubscriptionMode::Unsupported;
        capabilities.concurrency = NonZeroUsize::MIN;
        capabilities.max_items_per_request = target.device.max_items();
        let (state, _watch) = StateCell::new(ConnectionState::Disconnected);
        Self {
            clock,
            device: target.device,
            name: target.name,
            latency: target.latency,
            seed: target.seed,
            timeouts: *config.timeouts(),
            retry: config.retry().clone(),
            access: config.access(),
            limits: *config.limits(),
            capabilities,
            state,
            shutdown: CancellationToken::new(),
            in_flight: AtomicBool::new(false),
            next_request_id: AtomicU64::new(0),
        }
    }

    /// Open the connection: `Disconnected` -> `Connecting` -> (latency) -> `Connected`.
    ///
    /// The wait is bounded by `timeouts.connect`; a latency that reaches it is a
    /// `Timeout` and the connection stays in `Connecting`. The connect takes one step of
    /// the device's failure script: `Delay` waits first, `Timeout` waits for the connect
    /// deadline and fails, `Disconnect` leaves the connection in `Failed(Disconnected)`,
    /// and any other step is consumed with a warning and has no effect.
    pub(crate) fn establish(&self) -> OpFuture<'_, ()> {
        let request_id = self.next_request_id();
        let span = tracing::info_span!(
            "connect",
            scheme = "sim",
            endpoint = %self.name,
            op = "connect",
            request_id
        );
        Box::pin(self.establish_inner().instrument(span))
    }

    async fn establish_inner(&self) -> Result<()> {
        let op = self.op(Operation::Connect);
        let deadline = op.after(op.started, self.timeouts.connect)?;
        self.state.set(ConnectionState::Connecting);
        match self.next_step() {
            None => {}
            Some(FailureStep::Delay(pause)) => self.wait(&op, deadline, None, pause).await?,
            Some(FailureStep::Timeout) => return Err(self.lose(&op, deadline, None).await),
            Some(FailureStep::Disconnect) => return Err(self.fail_link(&op)),
            Some(
                ignored @ (FailureStep::DeviceException(_)
                | FailureStep::Malformed
                | FailureStep::PartialFailure(_)
                | FailureStep::Backpressure
                | FailureStep::UnknownAfterTransmit),
            ) => {
                tracing::warn!(
                    step = step_name(&ignored),
                    "a failure step that means nothing to a connect was consumed and ignored"
                );
            }
        }
        self.delay(&op, deadline, None).await?;
        if let Some(seed) = self.seed {
            self.device.reseed(seed);
        }
        self.state.set(ConnectionState::Connected);
        tracing::info!("sim connection established");
        Ok(())
    }

    fn next_request_id(&self) -> u64 {
        self.next_request_id.fetch_add(1, Ordering::Relaxed)
    }

    fn op(&self, operation: Operation) -> Op<'_> {
        Op {
            connection: self,
            operation,
            started: self.clock.now(),
            retries: 0,
        }
    }

    /// Claim the single in-flight slot or fail with `Backpressure`.
    fn acquire(&self, op: &Op<'_>) -> Result<Slot<'_>> {
        // Acquire on success pairs with the Release store in `Slot::drop`, so the next
        // holder sees everything the previous operation did to the device before it let go.
        self.in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Slot(&self.in_flight))
            .map_err(|_busy| {
                op.reject(
                    ErrorKind::Backpressure,
                    "another operation is in flight and the simulator has no wait queue",
                )
            })
    }

    /// The checks that precede every attempt: open, not cancelled, deadline not elapsed.
    fn gate(
        &self,
        op: &Op<'_>,
        token: Option<&CancellationToken>,
        deadline: Deadline,
    ) -> Result<()> {
        if self.state.current() != ConnectionState::Connected {
            return Err(op.error(
                ErrorKind::Disconnected,
                "the connection is not open",
                FailureCategory::Transport,
            ));
        }
        if token.is_some_and(CancellationToken::is_cancelled) {
            return Err(op.reject(ErrorKind::Cancelled, "the request was cancelled"));
        }
        if deadline.is_elapsed(self.clock.now()) {
            return Err(op.error(
                ErrorKind::Timeout,
                "the request deadline had already elapsed",
                FailureCategory::Transport,
            ));
        }
        Ok(())
    }

    /// Wait out the device latency on the connection's clock, racing the request's token,
    /// its deadline and the connection being closed; see [`wait`](Self::wait).
    async fn delay(
        &self,
        op: &Op<'_>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
    ) -> Result<()> {
        self.wait(op, deadline, token, self.latency).await
    }

    /// Wait `span` on the connection's clock, racing the request's token, its deadline and
    /// the connection being closed.
    ///
    /// The wait ends at the earlier of the reply time and the deadline. A reply time at or
    /// after the deadline is a `Timeout` once the deadline is reached (equality means the
    /// deadline elapsed), so the caller really waits for the deadline, as it would for a
    /// device that is too slow. If the token or the shutdown fires first, the wait ends
    /// early with `Cancelled` or `Disconnected`. A token is polled before the shutdown and
    /// both before the timer, so a wake-up that coincides with the timer reports the
    /// cancellation.
    async fn wait(
        &self,
        op: &Op<'_>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
        span: Duration,
    ) -> Result<()> {
        let reply = op.after(self.clock.now(), span)?;
        let too_slow = deadline.is_elapsed(reply.instant());
        let target = if too_slow { deadline } else { reply };
        match self.race(target.instant(), token).await {
            None if too_slow => Err(op.error(
                ErrorKind::Timeout,
                "the request deadline passed before the device replied",
                FailureCategory::Transport,
            )),
            None => Ok(()),
            Some(interrupt) => Err(op.interrupted(interrupt)),
        }
    }

    /// Sleep until `target` on the connection's clock unless the token or the shutdown
    /// fires first; `None` means the sleep elapsed.
    async fn race(&self, target: Instant, token: Option<&CancellationToken>) -> Option<Interrupt> {
        let mut sleep = self.clock.sleep_until(target);
        let mut cancelled = token.map(CancellationToken::cancelled);
        let mut closing = self.shutdown.cancelled();
        poll_fn(|cx| {
            if let Some(cancelled) = cancelled.as_mut()
                && cancelled.as_mut().poll(cx).is_ready()
            {
                return Poll::Ready(Some(Interrupt::Cancelled));
            }
            if closing.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Some(Interrupt::Closed));
            }
            // `Clock::sleep_until` never fails for a correct clock (its contract), and a
            // broken one that did could only end the wait early, so the result is not read.
            sleep.as_mut().poll(cx).map(|_slept| None)
        })
        .await
    }

    /// Take the next step of the device's failure script for this operation, if any.
    fn next_step(&self) -> Option<FailureStep> {
        let step = self.device.take_failure();
        if let Some(step) = &step {
            tracing::debug!(step = step_name(step), "consumed a failure step");
        }
        step
    }

    /// The scripted loss of a reply: wait for the deadline (or an interruption) and fail.
    async fn lose(
        &self,
        op: &Op<'_>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
    ) -> PlcError {
        match self.race(deadline.instant(), token).await {
            None => op.error(
                ErrorKind::Timeout,
                "the device's reply was lost and the request deadline passed",
                FailureCategory::Transport,
            ),
            Some(interrupt) => op.interrupted(interrupt),
        }
    }

    /// The scripted loss of the link: the connection is `Failed(Disconnected)` for good.
    fn fail_link(&self, op: &Op<'_>) -> PlcError {
        self.state
            .set(ConnectionState::Failed(ErrorKind::Disconnected));
        op.error(
            ErrorKind::Disconnected,
            "the link to the device was lost",
            FailureCategory::Transport,
        )
    }

    /// Apply the scripted step (if any) that acts before the device replies, then wait out
    /// the latency, and say what the reply is to be.
    async fn arrive(
        &self,
        op: &Op<'_>,
        step: Option<FailureStep>,
        deadline: Deadline,
        token: Option<&CancellationToken>,
    ) -> Result<Reply> {
        let reply = match step {
            None => Reply::Serve,
            Some(FailureStep::Delay(pause)) => {
                self.wait(op, deadline, token, pause).await?;
                Reply::Serve
            }
            Some(FailureStep::Timeout) => Reply::Lose,
            Some(FailureStep::Disconnect) => return Err(self.fail_link(op)),
            Some(FailureStep::DeviceException(status)) => Reply::Refuse(status),
            Some(FailureStep::Malformed) => Reply::Garble,
            Some(FailureStep::PartialFailure(ids)) => Reply::Partial(ids),
            Some(FailureStep::Backpressure) => {
                return Err(op.reject(
                    ErrorKind::Backpressure,
                    "the simulator was scripted to refuse the request",
                ));
            }
            Some(FailureStep::UnknownAfterTransmit) => Reply::Unknown,
        };
        self.delay(op, deadline, token).await?;
        Ok(reply)
    }

    /// Resolve every tag to a `SimAddress`, rejecting what the simulator cannot serve.
    fn resolve<'a>(
        &self,
        op: &Op<'_>,
        tags: impl ExactSizeIterator<Item = &'a Tag>,
    ) -> Result<Vec<&'a SimAddress>> {
        if let Some(max) = self.capabilities.max_items_per_request
            && tags.len() > max.get()
        {
            return Err(op.reject(
                ErrorKind::ResourceLimit,
                "the request has more items than the simulated device accepts",
            ));
        }
        tags.map(|tag| {
            let address: &dyn Any = tag.address();
            match address.downcast_ref::<SimAddress>() {
                Some(address) if tag.count() == NonZeroU32::MIN => Ok(address),
                Some(_) => Err(op.reject(
                    ErrorKind::InvalidAddress,
                    "the simulator serves one element per tag",
                )),
                None => Err(op.reject(
                    ErrorKind::InvalidAddress,
                    "the tag's address does not belong to the sim driver",
                )),
            }
        })
        .collect()
    }

    /// The deadline the request carries, or the connection's default for one without.
    fn deadline_for(&self, op: &Op<'_>, requested: Option<Deadline>) -> Result<Deadline> {
        requested.map_or_else(|| op.after(op.started, self.timeouts.request), Ok)
    }

    async fn read_inner(&self, request: ReadRequest) -> Result<ReadResponse> {
        let mut op = self.op(Operation::Read);
        let addresses = self.resolve(&op, request.items().iter())?;
        let first = self.deadline_for(&op, request.deadline())?;
        let _slot = self.acquire(&op)?;
        let items = loop {
            // An explicit deadline is shared by every attempt; a request without one gets
            // `timeouts.request` afresh for each attempt, so a retry has time to run.
            let deadline = if request.deadline().is_none() && op.retries > 0 {
                op.after(self.clock.now(), self.timeouts.request)?
            } else {
                first
            };
            match self.read_attempt(&op, &request, &addresses, deadline).await {
                Ok(items) => break items,
                Err(error)
                    if self
                        .retry
                        .should_retry(error.kind(), op.retries.saturating_add(1)) =>
                {
                    tracing::debug!(retries = op.retries, "retrying the whole read");
                    op.retries = op.retries.saturating_add(1);
                }
                Err(error) => return Err(error),
            }
        };
        let finished = self.clock.now();
        let failed = items.iter().filter(|item| item.outcome().is_err()).count();
        let built = items
            .into_iter()
            .fold(
                ReadResponse::builder(&request, op.started, finished),
                ReadResponseBuilder::item,
            )
            .build();
        let response = op.check(built, FailureCategory::Internal)?;
        tracing::debug!(items = response.items().len(), failed, "read complete");
        Ok(response)
    }

    /// One attempt: gate, take a scripted step, wait for the device, then answer.
    async fn read_attempt(
        &self,
        op: &Op<'_>,
        request: &ReadRequest,
        addresses: &[&SimAddress],
        deadline: Deadline,
    ) -> Result<Vec<ReadItem>> {
        self.gate(op, request.cancel(), deadline)?;
        let step = self.next_step();
        let reply = self.arrive(op, step, deadline, request.cancel()).await?;
        match reply {
            Reply::Serve => Ok(self.sample_all(op, request, addresses, &[])),
            Reply::Partial(failing) => Ok(self.sample_all(op, request, addresses, &failing)),
            Reply::Refuse(status) => Ok(self.fail_all(
                op,
                request,
                ErrorKind::DeviceException,
                "the device refused the request",
                Some(&status),
            )),
            Reply::Garble => Ok(self.fail_all(
                op,
                request,
                ErrorKind::MalformedData,
                "the device's reply could not be decoded",
                None,
            )),
            Reply::Lose | Reply::Unknown => {
                // The device answered; the reply never arrived.
                drop(self.sample_all(op, request, addresses, &[]));
                Err(self.lose(op, deadline, request.cancel()).await)
            }
        }
    }

    /// Serve every tag, except those named in `failing`, which fail as scripted.
    fn sample_all(
        &self,
        op: &Op<'_>,
        request: &ReadRequest,
        addresses: &[&SimAddress],
        failing: &[TagId],
    ) -> Vec<ReadItem> {
        let status = partial_failure_status();
        request
            .items()
            .iter()
            .zip(addresses)
            .map(|(tag, address)| {
                if failing.contains(tag.id()) {
                    let refusal = op.scripted_item(
                        ErrorKind::DeviceException,
                        "the device failed this item",
                        Some(&status),
                    );
                    ReadItem::new(tag.id().clone(), Err(refusal), self.clock.now())
                } else {
                    self.sample(op, tag, address)
                }
            })
            .collect()
    }

    /// Fail every item without touching the device.
    fn fail_all(
        &self,
        op: &Op<'_>,
        request: &ReadRequest,
        kind: ErrorKind,
        message: &'static str,
        status: Option<&ProtocolStatus>,
    ) -> Vec<ReadItem> {
        request
            .items()
            .iter()
            .map(|tag| {
                ReadItem::new(
                    tag.id().clone(),
                    Err(op.scripted_item(kind, message, status)),
                    self.clock.now(),
                )
            })
            .collect()
    }

    fn sample(&self, op: &Op<'_>, tag: &Tag, address: &SimAddress) -> ReadItem {
        let sampled_at = self.clock.now();
        let served = match address.area() {
            SimArea::State => self
                .device
                .serve_read(address.name())
                .and_then(|value| expect_type(value, address)),
            SimArea::Random => self.device.random(address.value_type()),
            SimArea::Sink => Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Read),
                "a SINK address is write-only",
            )),
        };
        ReadItem::new(
            tag.id().clone(),
            served.map_err(|error| op.item_error(error)),
            sampled_at,
        )
    }

    async fn write_inner(&self, request: WriteRequest) -> Result<WriteResponse> {
        let op = self.op(Operation::Write);
        if self.access == AccessPolicy::ReadOnly {
            return Err(op.reject(
                ErrorKind::Unsupported(Operation::Write),
                "the connection's access policy is read-only",
            ));
        }
        let addresses = self.resolve(&op, request.items().iter().map(WriteItem::tag))?;
        let deadline = self.deadline_for(&op, request.deadline())?;
        let _slot = self.acquire(&op)?;
        self.gate(&op, request.cancel(), deadline)?;
        let step = self.next_step();
        let reply = self.arrive(&op, step, deadline, request.cancel()).await?;
        let results = match reply {
            Reply::Serve => self.apply_all(&op, &request, &addresses, &[]),
            Reply::Partial(failing) => self.apply_all(&op, &request, &addresses, &failing),
            Reply::Refuse(status) => request
                .items()
                .iter()
                .map(|item| {
                    let refusal = op.scripted_item(
                        ErrorKind::DeviceException,
                        "the device refused the request",
                        Some(&status),
                    );
                    WriteResult::new(item.tag().id().clone(), WriteOutcome::Rejected(refusal))
                })
                .collect(),
            Reply::Garble => {
                return Err(op.error(
                    ErrorKind::MalformedData,
                    "the device's confirmation could not be decoded",
                    FailureCategory::Protocol,
                ));
            }
            Reply::Lose => {
                // The device applied the write; the confirmation never arrived.
                drop(self.apply_all(&op, &request, &addresses, &[]));
                return Err(self.lose(&op, deadline, request.cancel()).await);
            }
            Reply::Unknown => {
                // Applied exactly once; whether it was is all the caller can know.
                drop(self.apply_all(&op, &request, &addresses, &[]));
                request
                    .items()
                    .iter()
                    .map(|item| {
                        WriteResult::new(
                            item.tag().id().clone(),
                            WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit),
                        )
                    })
                    .collect()
            }
        };
        let finished = self.clock.now();
        let rejected = results
            .iter()
            .filter(|result| matches!(result.outcome(), WriteOutcome::Rejected(_)))
            .count();
        let built = results
            .into_iter()
            .fold(
                WriteResponse::builder(&request, op.started, finished),
                WriteResponseBuilder::result,
            )
            .build();
        let response = op.check(built, FailureCategory::Internal)?;
        tracing::debug!(items = response.results().len(), rejected, "write complete");
        Ok(response)
    }

    /// Apply every item, except those named in `failing`, which are rejected as scripted.
    fn apply_all(
        &self,
        op: &Op<'_>,
        request: &WriteRequest,
        addresses: &[&SimAddress],
        failing: &[TagId],
    ) -> Vec<WriteResult> {
        request
            .items()
            .iter()
            .zip(addresses)
            .map(|(item, address)| {
                if failing.contains(item.tag().id()) {
                    let status = partial_failure_status();
                    let refusal = op.scripted_item(
                        ErrorKind::DeviceException,
                        "the device failed this item",
                        Some(&status),
                    );
                    WriteResult::new(item.tag().id().clone(), WriteOutcome::Rejected(refusal))
                } else {
                    self.apply(op, item, address)
                }
            })
            .collect()
    }

    fn apply(&self, op: &Op<'_>, item: &WriteItem, address: &SimAddress) -> WriteResult {
        let applied = match address.area() {
            SimArea::State => self.device.write(address.name(), item.value().clone()),
            SimArea::Sink => {
                self.device.record_sink_write();
                Ok(())
            }
            SimArea::Random => Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                "a RANDOM address is read-only",
            )),
        };
        let outcome = match applied {
            Ok(()) => WriteOutcome::Confirmed,
            Err(error) => WriteOutcome::Rejected(op.item_error(error)),
        };
        WriteResult::new(item.tag().id().clone(), outcome)
    }

    async fn close_inner(&self) -> Result<()> {
        if self.state.current() == ConnectionState::Closed {
            return Ok(());
        }
        let op = self.op(Operation::Close);
        let bound = op.after(op.started, self.timeouts.request)?;
        let goodbye = op.after(op.started, self.latency)?;
        self.state.set(ConnectionState::Closing);
        self.shutdown.cancel();
        // As in `delay`, the result of a clock's sleep is not read: it cannot fail.
        drop(
            self.clock
                .sleep_until(goodbye.instant().min(bound.instant()))
                .await,
        );
        self.state.set(ConnectionState::Closed);
        tracing::info!("sim connection closed");
        Ok(())
    }
}

/// A `STATE` value must have the type its address names.
fn expect_type(value: PlcValue, address: &SimAddress) -> Result<PlcValue> {
    if value.value_type() == address.value_type() {
        Ok(value)
    } else {
        Err(PlcError::new(
            ErrorKind::InvalidConfig,
            "the stored value's type differs from the address's type",
        ))
    }
}

/// The status of an item a `PartialFailure` step names.
const fn partial_failure_status() -> ProtocolStatus {
    ProtocolStatus::new("sim", PARTIAL_FAILURE_CODE)
}

/// The name of a step, for tracing.
const fn step_name(step: &FailureStep) -> &'static str {
    match step {
        FailureStep::Delay(_) => "delay",
        FailureStep::Timeout => "timeout",
        FailureStep::Disconnect => "disconnect",
        FailureStep::DeviceException(_) => "device-exception",
        FailureStep::Malformed => "malformed",
        FailureStep::PartialFailure(_) => "partial-failure",
        FailureStep::Backpressure => "backpressure",
        FailureStep::UnknownAfterTransmit => "unknown-after-transmit",
    }
}

impl Connection for SimConnection {
    fn state(&self) -> ConnectionState {
        self.state.current()
    }

    fn watch_state(&self) -> StateWatch {
        self.state.watch()
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn limits(&self) -> ResourceLimits {
        self.limits
    }

    /// Read every tag of `request`; see the type documentation for the per-area rules.
    ///
    /// # Cancel safety
    /// Cancel-safe. The simulator transmits nothing; the device is read only after the
    /// latency wait ends, so dropping the future earlier leaves the device untouched and
    /// releases the single in-flight slot. Cancelling the request's token during the wait
    /// resolves the call with [`ErrorKind::Cancelled`]. The deadline source is the request's
    /// deadline, or `timeouts.request` of the connection. A scripted
    /// [`FailureStep::Timeout`] serves the read first and then waits for the deadline, so
    /// dropping the future during that last wait does not undo the device read.
    ///
    /// # Wire effects
    /// None. There is no wire; a read never changes the device except that a ramp steps and
    /// a read counter counts.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        let request_id = self.next_request_id();
        let span = tracing::debug_span!(
            "read",
            scheme = "sim",
            endpoint = %self.name,
            op = "read",
            request_id
        );
        Box::pin(self.read_inner(request).instrument(span))
    }

    /// Write every item of `request`; see the type documentation for the per-area rules.
    ///
    /// # Cancel safety
    /// Cancel-safe. The device is changed only after the latency wait ends, in one synchronous
    /// step, so dropping the future or cancelling the token earlier leaves the device exactly
    /// as it was. The deadline source is the request's deadline, or `timeouts.request`. A
    /// scripted [`FailureStep::Timeout`] applies the write first and then waits for the
    /// deadline, so dropping the future during that last wait does not undo the write.
    ///
    /// # Wire effects
    /// None. There is no wire; a confirmed `STATE` write replaces the stored value and a
    /// confirmed `SINK` write is counted and discarded.
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        let request_id = self.next_request_id();
        let span = tracing::debug_span!(
            "write",
            scheme = "sim",
            endpoint = %self.name,
            op = "write",
            request_id
        );
        Box::pin(self.write_inner(request).instrument(span))
    }

    /// Close the connection: `Closing`, a wait of the device latency (at most
    /// `timeouts.request`), then `Closed`.
    ///
    /// An operation that is waiting when `close` starts ends with
    /// [`ErrorKind::Disconnected`]. Closing a closed connection returns at once.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future leaves the state at `Closing`; call `close` again
    /// to reach `Closed`. The deadline source is `timeouts.request`.
    ///
    /// # Wire effects
    /// None. There is no wire.
    fn close(&self) -> OpFuture<'_, ()> {
        let request_id = self.next_request_id();
        let span = tracing::info_span!(
            "close",
            scheme = "sim",
            endpoint = %self.name,
            op = "close",
            request_id
        );
        Box::pin(self.close_inner().instrument(span))
    }
}

/// What cut a wait short.
#[derive(Clone, Copy)]
enum Interrupt {
    /// The request's token was cancelled.
    Cancelled,
    /// `close` started.
    Closed,
}

/// What the device does with a request that reached it, decided by the script.
enum Reply {
    /// Answer normally.
    Serve,
    /// Answer normally, but fail the named items.
    Partial(Vec<TagId>),
    /// Refuse every item with this status.
    Refuse(ProtocolStatus),
    /// Answer with data that cannot be decoded.
    Garble,
    /// Act on the request, then lose the reply.
    Lose,
    /// Act on the request and report that its outcome is unknown.
    Unknown,
}

/// Ownership of the single in-flight slot; dropping it, even when the future is dropped
/// mid-wait, releases the slot.
struct Slot<'a>(&'a AtomicBool);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The context every error of one operation is stamped with.
#[derive(Clone, Copy)]
struct Op<'a> {
    connection: &'a SimConnection,
    operation: Operation,
    /// The clock reading when the operation began, the base of every latency figure.
    started: Instant,
    retries: u32,
}

impl Op<'_> {
    fn diagnostics(&self, category: FailureCategory) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        diagnostics.operation = Some(self.operation);
        diagnostics.connection_state = Some(self.connection.state.current());
        diagnostics.latency = Some(
            self.connection
                .clock
                .now()
                .saturating_duration_since(self.started),
        );
        diagnostics.retries = self.retries;
        diagnostics.category = Some(category);
        diagnostics
    }

    fn error(&self, kind: ErrorKind, message: &'static str, category: FailureCategory) -> PlcError {
        PlcError::new(kind, message).with_diagnostics(self.diagnostics(category))
    }

    /// The error of a wait that was cut short.
    fn interrupted(&self, interrupt: Interrupt) -> PlcError {
        match interrupt {
            Interrupt::Cancelled => self.error(
                ErrorKind::Cancelled,
                "the request was cancelled while waiting for the device",
                FailureCategory::Internal,
            ),
            Interrupt::Closed => self.error(
                ErrorKind::Disconnected,
                "the connection was closed while the request was waiting",
                FailureCategory::Transport,
            ),
        }
    }

    /// A rejection before the simulated exchange begins.
    fn reject(&self, kind: ErrorKind, message: &'static str) -> PlcError {
        self.error(kind, message, FailureCategory::Validation)
    }

    /// `Deadline::after`, with this operation's diagnostics on an overflow.
    fn after(&self, from: Instant, span: Duration) -> Result<Deadline> {
        self.check(Deadline::after(from, span), FailureCategory::Validation)
    }

    /// Pass a result from core through, stamping a failure with this operation's
    /// diagnostics. For a response builder's `build` this arm cannot be taken for the
    /// items the connection assembles; it is `Internal` rather than ignored.
    fn check<T>(&self, result: Result<T>, category: FailureCategory) -> Result<T> {
        result.map_err(|error| self.wrap(error, category))
    }

    /// Re-wrap an error from core or the device with this operation's diagnostics.
    fn wrap(&self, error: PlcError, category: FailureCategory) -> PlcError {
        PlcError::new(error.kind(), error.to_string())
            .with_diagnostics(self.diagnostics(category))
            .with_source(error)
    }

    /// A per-item failure a script step produced: `Protocol`, with the status if any.
    fn scripted_item(
        &self,
        kind: ErrorKind,
        message: &'static str,
        status: Option<&ProtocolStatus>,
    ) -> ItemError {
        let item =
            ItemError::new(self.wrap(PlcError::new(kind, message), FailureCategory::Protocol));
        match status {
            Some(status) => item.with_status(status.clone()),
            None => item,
        }
    }

    /// A per-item failure the device reported (`Protocol`). An unset name carries the
    /// device status `("sim", 2)`.
    fn item_error(&self, error: PlcError) -> ItemError {
        let unset = error.kind() == ErrorKind::InvalidAddress;
        let item = ItemError::new(self.wrap(error, FailureCategory::Protocol));
        if unset {
            item.with_status(ProtocolStatus::new("sim", UNSET_TAG_CODE))
        } else {
            item
        }
    }
}

#[cfg(test)]
mod tests {
    use std::task::Poll;

    use stonecrab_core::Endpoint;
    use stonecrab_testkit::{FakeClock, block_on_with, poll_once};

    use super::*;

    fn connection(clock: &Arc<FakeClock>, latency: Duration) -> SimConnection {
        connection_on(clock, Arc::new(SimDevice::new()), latency)
    }

    fn connection_on(
        clock: &Arc<FakeClock>,
        device: Arc<SimDevice>,
        latency: Duration,
    ) -> SimConnection {
        let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .build()
            .expect("config");
        let target = Target {
            name: "bench".to_owned(),
            device,
            latency,
            seed: None,
        };
        SimConnection::new(Arc::<FakeClock>::clone(clock), target, &config)
    }

    // covers: ALPHA_PLAN S11b-2 / lifecycle Disconnected -> Connecting -> Connected ->
    // Closing -> Closed, each state observed by value
    #[test]
    fn the_lifecycle_runs_through_every_state_in_order() {
        let clock = Arc::new(FakeClock::default());
        let connection = connection(&clock, Duration::from_millis(5));
        let watch = connection.watch_state();
        let mut seen = vec![watch.current()];

        let mut establishing = connection.establish();
        assert!(
            poll_once(&mut establishing).is_pending(),
            "the connect waits"
        );
        seen.push(watch.current());
        assert!(clock.advance_to_next_sleeper().is_some(), "latency passes");
        assert!(
            matches!(poll_once(&mut establishing), Poll::Ready(Ok(()))),
            "the connect completes"
        );
        seen.push(watch.current());

        let mut closing = connection.close();
        assert!(poll_once(&mut closing).is_pending(), "the goodbye waits");
        seen.push(watch.current());
        assert!(clock.advance_to_next_sleeper().is_some(), "latency passes");
        assert!(
            matches!(poll_once(&mut closing), Poll::Ready(Ok(()))),
            "the close completes"
        );
        seen.push(watch.current());

        assert_eq!(
            seen,
            [
                ConnectionState::Disconnected,
                ConnectionState::Connecting,
                ConnectionState::Connected,
                ConnectionState::Closing,
                ConnectionState::Closed,
            ],
            "the documented lifecycle, in order"
        );
    }

    // covers: ALPHA_PLAN S12 / a scripted Disconnect on connect leaves the connection Failed
    #[test]
    fn a_scripted_disconnect_on_connect_leaves_the_connection_failed() {
        let clock = Arc::new(FakeClock::default());
        let device = Arc::new(SimDevice::new());
        device
            .push_failure(FailureStep::Disconnect)
            .expect("room in script");
        let connection = connection_on(&clock, Arc::clone(&device), Duration::ZERO);
        let error = block_on_with(&clock, connection.establish())
            .expect_err("a scripted disconnect fails the connect");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "kind");
        assert_eq!(
            connection.state(),
            ConnectionState::Failed(ErrorKind::Disconnected),
            "the dropped connection is Failed"
        );
        assert_eq!(device.pending_failures(), 0, "the step was consumed");
    }

    #[test]
    fn debug_names_the_device_and_the_state() {
        let clock = Arc::new(FakeClock::default());
        let connection = connection(&clock, Duration::ZERO);
        let shown = format!("{connection:?}");
        assert!(shown.contains("bench"), "device: {shown}");
        assert!(shown.contains("Disconnected"), "state: {shown}");
    }

    #[test]
    fn a_connection_that_never_connected_still_closes() {
        let clock = Arc::new(FakeClock::default());
        let connection = connection(&clock, Duration::ZERO);
        let mut closing = connection.close();
        assert!(
            matches!(poll_once(&mut closing), Poll::Ready(Ok(()))),
            "close needs no wait without latency"
        );
        assert_eq!(connection.state(), ConnectionState::Closed, "closed");
    }
}
