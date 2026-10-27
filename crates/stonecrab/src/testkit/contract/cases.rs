//! The cases of the contract suite, the assertion helpers they share and the runner.
//!
//! Every assertion is a call to a helper that returns `Err(Stop::Fail(reason))`, so a case
//! never panics: it ends at its first broken expectation and the runner records the reason.

use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::task::Poll;

use crate::{
    AccessPolicy, CancellationToken, Connection, ConnectionConfig, ConnectionState, Deadline,
    Driver, ErrorKind, FailureCategory, Operation, PlcError, ProtocolAddress, ReadRequest,
    ReadResponse, ResourceLimits, RetryPolicy, Tag, TagId, WriteRequest,
};

use super::{CaseOutcome, ContractCase, ContractFixture, ContractReport, Fault, Requires};
use crate::testkit::poll_once;

/// Reads of one request when the retry budget is spent: the suite's `max_attempts`.
const ATTEMPTS: NonZeroU32 = NonZeroU32::MIN.saturating_add(2);

/// Most items the over-budget case will build; a larger budget is skipped, not allocated.
const MAX_PROBE_ITEMS: usize = 8_192;

/// Reads the cleanup tries before it gives up on a fault that stays armed.
const SETTLE_TRIES: u32 = 6;

const A: &str = "contract-a";
const B: &str = "contract-b";
const C: &str = "contract-c";

/// Why a case stopped before the end.
pub(super) enum Stop {
    Fail(String),
    Skip(String),
}

type Check = Result<(), Stop>;
type Step<T> = Result<T, Stop>;

pub(super) fn fail<T>(why: impl Into<String>) -> Step<T> {
    Err(Stop::Fail(why.into()))
}

fn skip<T>(why: impl Into<String>) -> Step<T> {
    Err(Stop::Skip(why.into()))
}

pub(super) fn shown<T: fmt::Debug + ?Sized>(value: &T) -> String {
    format!("{value:?}")
}

pub(super) fn ensure(holds: bool, what: &str) -> Check {
    if holds { Ok(()) } else { fail(what) }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "call sites pass fresh temporaries; references would only add noise"
)]
pub(super) fn ensure_eq<T: PartialEq + fmt::Debug>(got: T, want: T, what: &str) -> Check {
    if got == want {
        Ok(())
    } else {
        fail(format!(
            "{what}: expected {}, got {}",
            shown(&want),
            shown(&got)
        ))
    }
}

/// Turn an error the case did not expect into a failure that names it.
pub(super) trait Must<T> {
    fn must(self, what: &str) -> Step<T>;
}

impl<T> Must<T> for Result<T, PlcError> {
    fn must(self, what: &str) -> Step<T> {
        self.or_else(|error| {
            fail(format!(
                "{what}: unexpected {} error: {error}",
                shown(&error.kind())
            ))
        })
    }
}

/// The error a call must fail with: success and every other kind are failures.
pub(super) fn refused<T>(
    outcome: Result<T, PlcError>,
    kind: ErrorKind,
    what: &str,
) -> Step<PlcError> {
    match outcome {
        Ok(_) => fail(format!(
            "{what}: expected a {} error, got success",
            shown(&kind)
        )),
        Err(error) if error.kind() == kind => Ok(error),
        Err(error) => fail(format!(
            "{what}: expected a {} error, got {}: {error}",
            shown(&kind),
            shown(&error.kind())
        )),
    }
}

/// The diagnostics every failure carries: the read, a connection state and a category.
pub(super) fn names(error: &PlcError, category: FailureCategory, what: &str) -> Check {
    let found = error.diagnostics();
    ensure_eq(
        found.operation,
        Some(Operation::Read),
        &format!("{what}: diagnostics.operation"),
    )?;
    ensure(
        found.connection_state.is_some(),
        &format!("{what}: diagnostics.connection_state is absent"),
    )?;
    ensure_eq(
        found.category,
        Some(category),
        &format!("{what}: diagnostics.category"),
    )
}

/// Run `check` on every error a device exception produced, whole-call or per item.
pub(super) fn each_exception(
    outcome: Result<ReadResponse, PlcError>,
    mut check: impl FnMut(&PlcError) -> Check,
) -> Check {
    match outcome {
        Err(error) => check(&error),
        Ok(response) => {
            for item in response.items() {
                match item.outcome() {
                    Ok(_) => {
                        return fail("an item was read although the device raised an exception");
                    }
                    Err(error) => check(error.error())?,
                }
            }
            Ok(())
        }
    }
}

fn tag(id: &str, address: &Arc<dyn ProtocolAddress>) -> Step<Tag> {
    Ok(Tag::new(
        TagId::new(id).must("a suite tag identifier")?,
        Arc::clone(address),
    ))
}

fn read_with(
    connection: &dyn Connection,
    tags: Vec<Tag>,
    deadline: Option<Deadline>,
    cancel: Option<CancellationToken>,
) -> Step<ReadRequest> {
    let mut builder = ReadRequest::builder(connection.limits());
    for tag in tags {
        builder = builder.tag(tag);
    }
    if let Some(deadline) = deadline {
        builder = builder.deadline(deadline);
    }
    if let Some(cancel) = cancel {
        builder = builder.cancel(cancel);
    }
    builder.build().must("building a read request")
}

async fn release(connection: Box<dyn Connection>) {
    drop(connection.close().await);
}

/// What a case works with: the driver, the connection string and the fixture.
pub(super) struct Env<'a> {
    pub(super) driver: &'a dyn Driver,
    pub(super) uri: &'a str,
    pub(super) fx: &'a ContractFixture,
}

impl Env<'_> {
    fn config(&self) -> Step<ConnectionConfig> {
        self.driver
            .parse_config(self.uri)
            .must("parsing the connection string")
    }

    /// The parsed configuration with another retry or access policy. A configuration
    /// with credentials cannot be copied (`Secret` is not `Clone`), so it skips.
    fn rebuilt(
        &self,
        retry: Option<RetryPolicy>,
        access: Option<AccessPolicy>,
    ) -> Step<ConnectionConfig> {
        let base = self.config()?;
        if base.credentials().is_some() {
            return skip("the parsed configuration carries credentials and cannot be rebuilt");
        }
        let mut builder = ConnectionConfig::builder(base.scheme(), base.endpoint().clone())
            .timeouts(*base.timeouts())
            .reconnect(*base.reconnect())
            .retry(retry.unwrap_or_else(|| base.retry().clone()))
            .access(access.unwrap_or_else(|| base.access()))
            .limits(*base.limits());
        if let Some(name) = base.address_options().convention() {
            builder = builder.address_convention(name);
        }
        builder.build().must("rebuilding the configuration")
    }

    async fn open(&self) -> Step<Box<dyn Connection>> {
        self.open_with(self.config()?).await
    }

    async fn open_with(&self, config: ConnectionConfig) -> Step<Box<dyn Connection>> {
        let connection = self.driver.connect(config).await.must("connecting")?;
        ensure_eq(
            connection.state(),
            ConnectionState::Connected,
            "the state of a new connection",
        )?;
        Ok(connection)
    }

    fn address(&self, text: &str) -> Step<Arc<dyn ProtocolAddress>> {
        let options = self.config()?.address_options();
        self.driver
            .parse_address(text, &options)
            .must("parsing a fixture address")
    }

    fn readable_tags(&self, ids: &[&str]) -> Step<Vec<Tag>> {
        let address = self.address(&self.fx.readable)?;
        ids.iter().map(|id| tag(id, &address)).collect()
    }

    fn readable_request(&self, connection: &dyn Connection, ids: &[&str]) -> Step<ReadRequest> {
        read_with(connection, self.readable_tags(ids)?, None, None)
    }

    fn write_request(&self, connection: &dyn Connection) -> Step<WriteRequest> {
        let Some((text, value)) = &self.fx.writable else {
            return skip("fixture has no writable address");
        };
        let target = tag(A, &self.address(text)?)?;
        WriteRequest::builder(connection.limits())
            .item(target, value.clone())
            .build()
            .must("building a write request")
    }

    fn probe(&self) -> u64 {
        (self.fx.io_probe)()
    }

    fn reads(&self) -> u32 {
        (self.fx.device_reads)()
    }

    fn no_io(&self, before: u64, what: &str) -> Check {
        ensure_eq(self.probe(), before, &format!("{what}: the I/O probe"))
    }

    fn inject(&self, fault: Fault) -> Check {
        let hook = match fault {
            Fault::Disconnect => &self.fx.inject_disconnect,
            Fault::Timeout => &self.fx.inject_timeout,
            Fault::DeviceException => &self.fx.inject_device_exception,
            Fault::UnknownAfterTransmit => &self.fx.inject_unknown_after_transmit,
        };
        match hook {
            Some(arm) => {
                arm();
                Ok(())
            }
            None => skip(format!("fixture cannot inject {fault}")),
        }
    }

    /// The reason a case cannot run, if its requirements are not met.
    fn unmet(&self, case: &ContractCase) -> Option<String> {
        let capabilities = self.driver.capabilities();
        case.requires.iter().find_map(|need| match need {
            Requires::Read if !capabilities.read => Some("driver reports read unsupported".into()),
            Requires::Write if !capabilities.write => {
                Some("driver reports write unsupported".into())
            }
            Requires::WriteUnsupported if capabilities.write => {
                Some("driver reports write supported; the case needs it unsupported".into())
            }
            Requires::WritableAddress if self.fx.writable.is_none() => {
                Some("fixture has no writable address".into())
            }
            Requires::Fault(fault) => self
                .fault_missing(*fault)
                .then(|| format!("fixture cannot inject {fault}")),
            Requires::Read
            | Requires::Write
            | Requires::WriteUnsupported
            | Requires::WritableAddress => None,
        })
    }

    const fn fault_missing(&self, fault: Fault) -> bool {
        match fault {
            Fault::Disconnect => self.fx.inject_disconnect.is_none(),
            Fault::Timeout => self.fx.inject_timeout.is_none(),
            Fault::DeviceException => self.fx.inject_device_exception.is_none(),
            Fault::UnknownAfterTransmit => self.fx.inject_unknown_after_transmit.is_none(),
        }
    }

    /// Drain faults a case left armed so the next case starts clean: connect and read
    /// until a read succeeds, which a leftover step cannot survive more than a few times.
    async fn settle(&self) -> Check {
        for _ in 0..SETTLE_TRIES {
            let Ok(connection) = self.open().await else {
                continue;
            };
            let clean = match self.readable_request(&*connection, &[A]) {
                Ok(request) => connection
                    .read(request)
                    .await
                    .is_ok_and(|response| response.iter_failed().next().is_none()),
                Err(_) => false,
            };
            release(connection).await;
            if clean {
                return Ok(());
            }
        }
        fail("a fault stayed armed after the case and could not be drained")
    }

    async fn execute(&self, id: Id) -> CaseOutcome {
        let outcome = match id.run(self).await {
            Ok(()) => CaseOutcome::Passed,
            Err(Stop::Fail(reason)) => CaseOutcome::Failed(reason),
            Err(Stop::Skip(reason)) => CaseOutcome::Skipped(reason),
        };
        let armed = id
            .case()
            .requires
            .iter()
            .any(|need| matches!(need, Requires::Fault(_)));
        match (armed, outcome) {
            (true, outcome) => match self.settle().await {
                Ok(()) => outcome,
                Err(Stop::Fail(reason) | Stop::Skip(reason)) => match outcome {
                    CaseOutcome::Passed => CaseOutcome::Failed(reason),
                    other @ (CaseOutcome::Failed(_) | CaseOutcome::Skipped(_)) => other,
                },
            },
            (false, outcome) => outcome,
        }
    }
}

pub(super) async fn run_all(
    driver: &dyn Driver,
    uri: &str,
    fixture: &ContractFixture,
) -> ContractReport {
    let env = Env {
        driver,
        uri,
        fx: fixture,
    };
    let mut done = Vec::with_capacity(Id::ALL.len());
    for id in Id::ALL {
        let case = id.case();
        let outcome = match env.unmet(&case) {
            Some(reason) => CaseOutcome::Skipped(reason),
            None => env.execute(id).await,
        };
        done.push((case, outcome));
    }
    ContractReport::new(done)
}

/// Every case, in execution order.
#[derive(Clone, Copy)]
pub(super) enum Id {
    Capabilities,
    Lifecycle,
    Correlation,
    Partial,
    Foreign,
    OverBudget,
    CancelledToken,
    ElapsedDeadline,
    AbsentDeadline,
    CancelBeforePoll,
    DisconnectDuringRead,
    ReadAfterClose,
    CloseTerminal,
    SourceTime,
    RejectedDiagnostics,
    TimeoutDiagnostics,
    DisconnectDiagnostics,
    ExceptionDiagnostics,
    RetryStops,
    RetrySkips,
    ReadOnlyPolicy,
    UnknownNotReplayed,
    UnsupportedWrite,
}

impl Id {
    pub(super) const ALL: [Self; 23] = [
        Self::Capabilities,
        Self::Lifecycle,
        Self::Correlation,
        Self::Partial,
        Self::Foreign,
        Self::OverBudget,
        Self::CancelledToken,
        Self::ElapsedDeadline,
        Self::AbsentDeadline,
        Self::CancelBeforePoll,
        Self::DisconnectDuringRead,
        Self::ReadAfterClose,
        Self::CloseTerminal,
        Self::SourceTime,
        Self::RejectedDiagnostics,
        Self::TimeoutDiagnostics,
        Self::DisconnectDiagnostics,
        Self::ExceptionDiagnostics,
        Self::RetryStops,
        Self::RetrySkips,
        Self::ReadOnlyPolicy,
        Self::UnknownNotReplayed,
        Self::UnsupportedWrite,
    ];

    pub(super) const fn case(self) -> ContractCase {
        const READ: &[Requires] = &[Requires::Read];
        match self {
            Self::Capabilities => ContractCase::new(
                "capabilities match behavior and unsupported operations perform no I/O",
                "F3 unsupported capability; SPEC 'Test layers' contract suite",
                &[],
            ),
            Self::Lifecycle => ContractCase::new(
                "connect and close walk the lifecycle states observed through the state watch",
                "F5 explicit shutdown; SPEC 'Connection and resource behavior'",
                &[],
            ),
            Self::Correlation => ContractCase::new(
                "a read correlates every tag with its result in request order",
                "SPEC 'Public API contract' read batch",
                READ,
            ),
            Self::Partial => ContractCase::new(
                "a partial failure keeps the successes",
                "F3 mixed success within a batch; SPEC 'Errors'",
                READ,
            ),
            Self::Foreign => ContractCase::new(
                "a foreign address is rejected with no I/O",
                "F3 invalid register or node",
                READ,
            ),
            Self::OverBudget => ContractCase::new(
                "an over-budget request is rejected with no I/O",
                "SPEC 'Connection and resource behavior' pending requests bounded",
                READ,
            ),
            Self::CancelledToken => ContractCase::new(
                "an already-cancelled token is rejected with no I/O",
                "F4 cancellation before transmit",
                READ,
            ),
            Self::ElapsedDeadline => ContractCase::new(
                "an elapsed deadline yields Timeout with no I/O",
                "SPEC 'Connection and resource behavior' bounded deadlines",
                READ,
            ),
            Self::AbsentDeadline => ContractCase::new(
                "an absent deadline uses the connection timeout",
                "SPEC 'Connection and resource behavior' bounded deadlines",
                &[Requires::Read, Requires::Fault(Fault::Timeout)],
            ),
            Self::CancelBeforePoll => ContractCase::new(
                "cancelling before the first poll yields Cancelled with nothing sent",
                "F4 cancellation while transmitting",
                READ,
            ),
            Self::DisconnectDuringRead => ContractCase::new(
                "a disconnect during a read yields Disconnected",
                "F5 network outage",
                &[Requires::Read, Requires::Fault(Fault::Disconnect)],
            ),
            Self::ReadAfterClose => ContractCase::new(
                "a read after close yields Disconnected",
                "F5 explicit shutdown",
                READ,
            ),
            Self::CloseTerminal => ContractCase::new(
                "close is terminal and a second close succeeds",
                "F5 explicit shutdown",
                &[],
            ),
            Self::SourceTime => ContractCase::new(
                "source time is absent unless the driver declares device time",
                "F6 timestamp absence",
                READ,
            ),
            Self::RejectedDiagnostics => ContractCase::new(
                "a request rejected before I/O carries diagnostics naming operation, state and category",
                "SPEC 'Connection and resource behavior' structured diagnostics",
                READ,
            ),
            Self::TimeoutDiagnostics => ContractCase::new(
                "a timeout carries diagnostics naming operation, state and category",
                "SPEC 'Connection and resource behavior' structured diagnostics",
                &[Requires::Read, Requires::Fault(Fault::Timeout)],
            ),
            Self::DisconnectDiagnostics => ContractCase::new(
                "a disconnect carries diagnostics naming operation, state and category",
                "SPEC 'Connection and resource behavior' structured diagnostics",
                &[Requires::Read, Requires::Fault(Fault::Disconnect)],
            ),
            Self::ExceptionDiagnostics => ContractCase::new(
                "a device exception carries diagnostics naming operation, state and category",
                "F3 device exception; SPEC 'Connection and resource behavior'",
                &[Requires::Read, Requires::Fault(Fault::DeviceException)],
            ),
            Self::RetryStops => ContractCase::new(
                "read retry stops after the configured maximum attempts",
                "F5 exhausted retry budget; SPEC 'Connection and resource behavior' bounded read retry",
                &[Requires::Read, Requires::Fault(Fault::Timeout)],
            ),
            Self::RetrySkips => ContractCase::new(
                "read retry skips non-retryable kinds",
                "F3 device exception; SPEC 'Connection and resource behavior' bounded read retry",
                &[Requires::Read, Requires::Fault(Fault::DeviceException)],
            ),
            Self::ReadOnlyPolicy => ContractCase::new(
                "a read-only policy rejects a write with no I/O",
                "SPEC 'Connection and resource behavior' read-only policy",
                &[Requires::Write, Requires::WritableAddress],
            ),
            Self::UnknownNotReplayed => ContractCase::new(
                "an unknown write outcome is never replayed",
                "SPEC 'Errors' ambiguous write",
                &[
                    Requires::Write,
                    Requires::WritableAddress,
                    Requires::Fault(Fault::UnknownAfterTransmit),
                ],
            ),
            Self::UnsupportedWrite => ContractCase::new(
                "an unsupported write performs no I/O",
                "F3 unsupported capability",
                &[Requires::WriteUnsupported, Requires::WritableAddress],
            ),
        }
    }

    async fn run(self, env: &Env<'_>) -> Check {
        match self {
            Self::Capabilities => capabilities_match(env).await,
            Self::Lifecycle => lifecycle(env).await,
            Self::Correlation => correlation(env).await,
            Self::Partial => partial_failure(env).await,
            Self::Foreign => foreign_address(env).await,
            Self::OverBudget => over_budget(env).await,
            Self::CancelledToken => cancelled_token(env).await,
            Self::ElapsedDeadline => elapsed_deadline(env).await,
            Self::AbsentDeadline => absent_deadline(env).await,
            Self::CancelBeforePoll => cancel_before_poll(env).await,
            Self::DisconnectDuringRead => disconnect_during_read(env).await,
            Self::ReadAfterClose => read_after_close(env).await,
            Self::CloseTerminal => close_terminal(env).await,
            Self::SourceTime => source_time(env).await,
            Self::RejectedDiagnostics => rejected_diagnostics(env).await,
            Self::TimeoutDiagnostics => timeout_diagnostics(env).await,
            Self::DisconnectDiagnostics => disconnect_diagnostics(env).await,
            Self::ExceptionDiagnostics => exception_diagnostics(env).await,
            Self::RetryStops => retry_stops(env).await,
            Self::RetrySkips => retry_skips(env).await,
            Self::ReadOnlyPolicy => read_only_policy(env).await,
            Self::UnknownNotReplayed => unknown_not_replayed(env).await,
            Self::UnsupportedWrite => unsupported_write(env).await,
        }
    }
}

async fn capabilities_match(env: &Env<'_>) -> Check {
    let declared = env.driver.capabilities();
    let connection = env.open().await?;
    let actual = connection.capabilities();
    ensure(
        !actual.read || declared.read,
        "the connection claims read; the driver does not",
    )?;
    ensure(
        !actual.write || declared.write,
        "the connection claims write; the driver does not",
    )?;
    ensure(
        !actual.browse || declared.browse,
        "the connection claims browse; the driver does not",
    )?;
    ensure(
        !actual.discover || declared.discover,
        "the connection claims discover; the driver does not",
    )?;
    ensure(
        !actual.supports(Operation::Subscribe) || declared.supports(Operation::Subscribe),
        "the connection claims subscribe; the driver does not",
    )?;
    let before = env.probe();
    let read = connection
        .read(env.readable_request(&*connection, &[A])?)
        .await;
    if actual.read {
        let response = read.must("a read the connection reports as supported")?;
        ensure_eq(
            response.iter_ok().count(),
            1,
            "values returned by that read",
        )?;
    } else {
        let what = "a read on a connection that reports read unsupported";
        refused(read, ErrorKind::Unsupported(Operation::Read), what)?;
        env.no_io(before, what)?;
    }
    if env.fx.writable.is_some() {
        let before = env.probe();
        let write = connection.write(env.write_request(&*connection)?).await;
        if actual.write {
            let response = write.must("a write the connection reports as supported")?;
            ensure_eq(response.confirmed_count(), 1, "confirmed writes")?;
        } else {
            let what = "a write on a connection that reports write unsupported";
            refused(write, ErrorKind::Unsupported(Operation::Write), what)?;
            env.no_io(before, what)?;
        }
    }
    release(connection).await;
    Ok(())
}

// Closing is asserted only when close pends on its first poll (StateWatch coalesces), so
// run the suite with latency where the driver allows it.
async fn lifecycle(env: &Env<'_>) -> Check {
    // `connect` returns only an open connection, so Connecting is not observable from the
    // outside; the open() helper has already asserted Connected by value.
    let connection = env.open().await?;
    let watch = connection.watch_state();
    ensure_eq(
        watch.current(),
        ConnectionState::Connected,
        "the watch of an open connection",
    )?;
    let mut closing = connection.close();
    match poll_once(&mut closing) {
        Poll::Ready(result) => result.must("closing the connection")?,
        Poll::Pending => {
            ensure_eq(
                connection.state(),
                ConnectionState::Closing,
                "the state while the close is in progress",
            )?;
            let change = poll_once(&mut watch.changed());
            ensure(
                matches!(change, Poll::Ready(Ok(ConnectionState::Closing))),
                "the watch reports Closing by value",
            )?;
            closing.await.must("closing the connection")?;
        }
    }
    ensure_eq(
        connection.state(),
        ConnectionState::Closed,
        "the state after close",
    )?;
    let change = poll_once(&mut watch.changed());
    ensure(
        matches!(change, Poll::Ready(Ok(ConnectionState::Closed))),
        "the watch reports Closed by value",
    )
}

// All tags share one address and value, so this checks tag order only, not that values
// are attached to the right tag.
async fn correlation(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let ids = [C, A, B];
    let request = env.readable_request(&*connection, &ids)?;
    let response = connection.read(request).await.must("a three-tag read")?;
    let got: Vec<&str> = response
        .items()
        .iter()
        .map(|item| item.tag().as_str())
        .collect();
    ensure_eq(got, ids.to_vec(), "result order against request order")?;
    ensure_eq(response.iter_failed().count(), 0, "failed items")?;
    release(connection).await;
    Ok(())
}

async fn partial_failure(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let readable = env.address(&env.fx.readable)?;
    let rejected = env.address(&env.fx.rejected)?;
    let tags = vec![tag(A, &readable)?, tag(B, &rejected)?, tag(C, &readable)?];
    let request = read_with(&*connection, tags, None, None)?;
    let what = "a request mixing readable and rejected addresses must not fail as a whole";
    let response = connection.read(request).await.must(what)?;
    let outcomes: Vec<&str> = response
        .items()
        .iter()
        .map(|item| {
            if item.outcome().is_ok() {
                "ok"
            } else {
                "failed"
            }
        })
        .collect();
    ensure_eq(outcomes, vec!["ok", "failed", "ok"], "per-item outcomes")?;
    release(connection).await;
    Ok(())
}

async fn foreign_address(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let before = env.probe();
    let foreign = tag(A, &env.fx.foreign)?;
    let request = read_with(&*connection, vec![foreign], None, None)?;
    refused(
        connection.read(request).await,
        ErrorKind::InvalidAddress,
        "a read of a foreign address",
    )?;
    env.no_io(before, "a foreign address")?;
    release(connection).await;
    Ok(())
}

async fn over_budget(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let limits = connection.limits().max_items_per_request().get();
    let budget = connection
        .capabilities()
        .max_items_per_request
        .map_or(limits, |advertised| advertised.get().min(limits));
    let wanted = budget.saturating_add(1);
    if wanted > MAX_PROBE_ITEMS {
        return skip("the connection's item budget is too large to exceed in a test");
    }
    let wide = ResourceLimits::builder()
        .max_items_per_request(wanted)
        .build()
        .must("building wider limits for the request")?;
    let address = env.address(&env.fx.readable)?;
    let mut builder = ReadRequest::builder(wide);
    for index in 0..wanted {
        builder = builder.tag(tag(&format!("contract-{index}"), &address)?);
    }
    let request = builder.build().must("building the over-budget request")?;
    let before = env.probe();
    refused(
        connection.read(request).await,
        ErrorKind::ResourceLimit,
        "a request one item over the budget",
    )?;
    env.no_io(before, "an over-budget request")?;
    release(connection).await;
    Ok(())
}

async fn cancelled_token(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let token = CancellationToken::new();
    token.cancel();
    let request = read_with(&*connection, env.readable_tags(&[A])?, None, Some(token))?;
    let before = env.probe();
    refused(
        connection.read(request).await,
        ErrorKind::Cancelled,
        "a read with a cancelled token",
    )?;
    env.no_io(before, "a cancelled token")?;
    release(connection).await;
    Ok(())
}

async fn elapsed_deadline(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let deadline = Deadline::at(env.fx.clock.now());
    let request = read_with(&*connection, env.readable_tags(&[A])?, Some(deadline), None)?;
    let before = env.probe();
    refused(
        connection.read(request).await,
        ErrorKind::Timeout,
        "a read whose deadline has elapsed",
    )?;
    env.no_io(before, "an elapsed deadline")?;
    release(connection).await;
    Ok(())
}

async fn absent_deadline(env: &Env<'_>) -> Check {
    let timeouts = *env.config()?.timeouts();
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::Timeout)?;
    let started = env.fx.clock.now();
    let outcome = connection.read(request).await;
    let elapsed = env.fx.clock.now().saturating_duration_since(started);
    refused(outcome, ErrorKind::Timeout, "a read whose reply is lost")?;
    ensure_eq(elapsed, timeouts.request, "time until the read failed")?;
    release(connection).await;
    Ok(())
}

async fn cancel_before_poll(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let token = CancellationToken::new();
    let request = read_with(
        &*connection,
        env.readable_tags(&[A])?,
        None,
        Some(token.clone()),
    )?;
    let before = env.probe();
    let pending = connection.read(request);
    token.cancel();
    refused(
        pending.await,
        ErrorKind::Cancelled,
        "a read cancelled before its first poll",
    )?;
    env.no_io(before, "a read cancelled before its first poll")?;
    release(connection).await;
    Ok(())
}

async fn disconnect_during_read(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::Disconnect)?;
    refused(
        connection.read(request).await,
        ErrorKind::Disconnected,
        "a read during a lost link",
    )?;
    release(connection).await;
    Ok(())
}

async fn read_after_close(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    connection.close().await.must("closing the connection")?;
    let request = env.readable_request(&*connection, &[A])?;
    let before = env.probe();
    refused(
        connection.read(request).await,
        ErrorKind::Disconnected,
        "a read after close",
    )?;
    env.no_io(before, "a read after close")
}

async fn close_terminal(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    connection.close().await.must("the first close")?;
    connection.close().await.must("the second close")?;
    ensure_eq(
        connection.state(),
        ConnectionState::Closed,
        "the state after two closes",
    )?;
    ensure_eq(
        connection.watch_state().current(),
        ConnectionState::Closed,
        "the watch after two closes",
    )
}

async fn source_time(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    let response = connection.read(request).await.must("a read")?;
    ensure(
        response.iter_ok().next().is_some(),
        "the read returned no value",
    )?;
    for item in response.iter_ok() {
        ensure_eq(
            item.source_time().is_some(),
            env.fx.declares_device_time,
            "whether source_time is present",
        )?;
    }
    release(connection).await;
    Ok(())
}

async fn rejected_diagnostics(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let foreign = tag(A, &env.fx.foreign)?;
    let request = read_with(&*connection, vec![foreign], None, None)?;
    let error = refused(
        connection.read(request).await,
        ErrorKind::InvalidAddress,
        "a read of a foreign address",
    )?;
    names(
        &error,
        FailureCategory::Validation,
        "a rejection before I/O",
    )?;
    release(connection).await;
    Ok(())
}

async fn timeout_diagnostics(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::Timeout)?;
    let error = refused(
        connection.read(request).await,
        ErrorKind::Timeout,
        "a read whose reply is lost",
    )?;
    names(&error, FailureCategory::Transport, "a timeout")?;
    release(connection).await;
    Ok(())
}

async fn disconnect_diagnostics(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::Disconnect)?;
    let error = refused(
        connection.read(request).await,
        ErrorKind::Disconnected,
        "a read during a lost link",
    )?;
    names(&error, FailureCategory::Transport, "a disconnect")?;
    release(connection).await;
    Ok(())
}

async fn exception_diagnostics(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::DeviceException)?;
    each_exception(connection.read(request).await, |error| {
        ensure_eq(error.kind(), ErrorKind::DeviceException, "the failure kind")?;
        names(error, FailureCategory::Protocol, "a device exception")
    })?;
    release(connection).await;
    Ok(())
}

fn retrying(attempts: NonZeroU32) -> RetryPolicy {
    RetryPolicy {
        max_attempts: attempts,
        ..RetryPolicy::default()
    }
}

async fn retry_stops(env: &Env<'_>) -> Check {
    let config = env.rebuilt(Some(retrying(ATTEMPTS)), None)?;
    let connection = env.open_with(config).await?;
    let request = env.readable_request(&*connection, &[A])?;
    for _ in 0..=ATTEMPTS.get() {
        env.inject(Fault::Timeout)?;
    }
    let before = env.reads();
    refused(
        connection.read(request).await,
        ErrorKind::Timeout,
        "a read whose every attempt times out",
    )?;
    ensure_eq(
        env.reads().saturating_sub(before),
        ATTEMPTS.get(),
        "device reads for one request (max_attempts)",
    )?;
    release(connection).await;
    Ok(())
}

// Against a device whose refusal is a per-item error the retry loop never sees, the
// counter delta is 0, so this case proves "surfaced, not masked" there and no more.
async fn retry_skips(env: &Env<'_>) -> Check {
    let config = env.rebuilt(Some(retrying(ATTEMPTS)), None)?;
    let connection = env.open_with(config).await?;
    let request = env.readable_request(&*connection, &[A])?;
    env.inject(Fault::DeviceException)?;
    let before = env.reads();
    let outcome = connection.read(request).await;
    // A device that refuses a request may or may not count it as a read; what matters is
    // that the refusal is surfaced once and never retried (a retry would move the counter
    // again or hide the refusal behind a later success).
    ensure(
        env.reads().saturating_sub(before) <= 1,
        "a device exception was retried: more than one device read",
    )?;
    each_exception(outcome, |error| {
        ensure_eq(error.kind(), ErrorKind::DeviceException, "the failure kind")
    })?;
    release(connection).await;
    Ok(())
}

async fn read_only_policy(env: &Env<'_>) -> Check {
    let config = env.rebuilt(None, Some(AccessPolicy::ReadOnly))?;
    let connection = env.open_with(config).await?;
    let request = env.write_request(&*connection)?;
    let before = env.probe();
    refused(
        connection.write(request).await,
        ErrorKind::Unsupported(Operation::Write),
        "a write on a read-only connection",
    )?;
    env.no_io(before, "a read-only policy")?;
    release(connection).await;
    Ok(())
}

async fn unknown_not_replayed(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    let request = env.write_request(&*connection)?;
    env.inject(Fault::UnknownAfterTransmit)?;
    let before = env.probe();
    let what = "a write whose confirmation is lost";
    let response = connection.write(request).await.must(what)?;
    ensure_eq(response.unknown_count(), 1, "Unknown outcomes")?;
    ensure_eq(
        env.probe().saturating_sub(before),
        1,
        "times the write was applied",
    )?;
    release(connection).await;
    Ok(())
}

async fn unsupported_write(env: &Env<'_>) -> Check {
    let connection = env.open().await?;
    ensure(
        !connection.capabilities().write,
        "the connection reports write although the driver does not",
    )?;
    let request = env.write_request(&*connection)?;
    let before = env.probe();
    refused(
        connection.write(request).await,
        ErrorKind::Unsupported(Operation::Write),
        "a write on a driver without write",
    )?;
    env.no_io(before, "an unsupported write")?;
    release(connection).await;
    Ok(())
}
