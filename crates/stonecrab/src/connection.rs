//! The connection boundary: one live link to one device.

use std::time::Duration;

use crate::{
    Capabilities, ConnectionState, Diagnostics, ErrorKind, FailureCategory, OpFuture, Operation,
    PlcError, ReadRequest, ReadResponse, ResourceLimits, StateWatch, SubscribeRequest,
    Subscription, WriteRequest, WriteResponse,
};

/// One live link to one device: state observation, reads, writes and an explicit close.
///
/// A [`Driver`](crate::Driver) returns a `Box<dyn Connection>` from `connect`. The trait
/// is object safe and `Send + Sync`; every asynchronous method returns an [`OpFuture`], a
/// boxed `Send` future, so a connection can be shared between tasks and driven by any
/// executor. All per-connection state (the transport, the lifecycle state, the in-flight
/// request) lives behind it.
///
/// # Narrow surface
/// The surface is the seven required methods below (decision D2) and two provided methods
/// that return [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) until a driver
/// overrides them: [`ping`](Self::ping), which takes no request (ADR-0011, addendum of
/// slice S40), and [`subscribe`](Self::subscribe), which takes a [`SubscribeRequest`]
/// (ADR-0020). `browse` and `discover` are not here: their request and response types do
/// not exist yet (ADR-0001 forbids empty scaffolds). They will be added as provided
/// methods in the same way, so adding them breaks no implementor.
///
/// # The default-deadline rule
/// Every operation is bounded (`CODING_STANDARDS` 8.5). A request that carries an explicit
/// [`Deadline`](crate::Deadline) keeps it, and one that does not receives
/// `Deadline::after(clock.now(), timeouts.request)`, computed by the connection from its
/// own clock and owned by the connection (ADR-0007). A request whose deadline has already
/// elapsed on that clock is rejected with [`ErrorKind::Timeout`](crate::ErrorKind::Timeout)
/// before anything is transmitted. A caller who injects a clock derives its deadlines from
/// the same clock.
///
/// # Cancellation
/// A request may carry a [`CancellationToken`](crate::CancellationToken). A token that is
/// already cancelled when the operation starts is rejected with
/// [`ErrorKind::Cancelled`](crate::ErrorKind::Cancelled) before anything is transmitted;
/// one cancelled while the operation waits ends it with the same kind. Dropping the
/// returned future is the other way to cancel; the `# Cancel safety` section of each
/// method says what that costs.
///
/// # Compatibility: docs/parity/core.md#CORE-CONNECTION-TRAIT
/// Corresponds to the upstream `PlcConnection`: read and write requests execute against
/// one connection, and capability support is queryable. Upstream's `ping` and subscription
/// requests are provided methods here that return `Unsupported` until a driver overrides
/// them. See ADR-0011 for every difference from the ARCHITECTURE.md 3.1 diagram.
///
/// # Examples
/// Code that is written against the trait works with any driver's connection. This
/// function waits for a connection to leave its opening states.
/// ```
/// use stonecrab::{Connection, ConnectionState, Result};
///
/// async fn wait_until_open(connection: &dyn Connection) -> Result<ConnectionState> {
///     let watch = connection.watch_state();
///     let mut state = watch.current();
///     while matches!(state, ConnectionState::Disconnected | ConnectionState::Connecting) {
///         state = watch.changed().await?;
///     }
///     Ok(state)
/// }
/// ```
pub trait Connection: Send + Sync {
    /// The current lifecycle state.
    ///
    /// This is a snapshot: the state may change before the caller acts on it. Use
    /// [`watch_state`](Self::watch_state) to observe changes.
    fn state(&self) -> ConnectionState;

    /// A new observer of the lifecycle state.
    ///
    /// The watch has latest-value semantics: a transition that is overwritten before the
    /// observer polls is coalesced (see [`StateWatch`]). The terminal states
    /// [`Closed`](ConnectionState::Closed) and [`Failed`](ConnectionState::Failed) are the
    /// end of the stream.
    fn watch_state(&self) -> StateWatch;

    /// What this connection can do.
    ///
    /// Returned by value: [`Capabilities`] is `Copy`. It may be narrower than the
    /// driver's, because it reflects the device the connection is attached to (for
    /// example its item limit).
    fn capabilities(&self) -> Capabilities;

    /// The resource budgets this connection enforces.
    ///
    /// Returned by value: [`ResourceLimits`] is `Copy`. A caller builds requests with
    /// `ReadRequest::builder(connection.limits())` so the item budget in the builder is
    /// the connection's own.
    fn limits(&self) -> ResourceLimits;

    /// Read every tag of `request` and correlate the results.
    ///
    /// Partial failure is data: a tag that cannot be read yields a per-item error in the
    /// response, not an error of the whole call. The call fails as a whole when the
    /// request is rejected before any I/O, when the deadline passes, when the token is
    /// cancelled, or when the link is lost. The deadline is the request's own, or
    /// `Deadline::after(clock.now(), timeouts.request)` when it has none (see the
    /// default-deadline rule on the trait).
    ///
    /// A read may be retried by the connection under its
    /// [`RetryPolicy`](crate::RetryPolicy), only for retryable error kinds and only as a
    /// whole request.
    ///
    /// # Errors
    /// Returns [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) when the
    /// connection is not open, [`ErrorKind::InvalidAddress`](crate::ErrorKind::InvalidAddress)
    /// for a tag whose address belongs to another driver,
    /// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) when reads are not
    /// supported, [`ErrorKind::Backpressure`](crate::ErrorKind::Backpressure) when the
    /// connection's concurrency limit is reached, [`ErrorKind::Cancelled`](crate::ErrorKind::Cancelled)
    /// for a cancelled token, and [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) for an
    /// elapsed or exceeded deadline. Each error carries [`Diagnostics`](crate::Diagnostics).
    ///
    /// # Cancel safety
    /// A rejection before transmission (cancelled token, elapsed deadline, bad address,
    /// limit reached) sends nothing. Dropping the future after the driver transmitted
    /// abandons the read: reads do not change the device, so the only consequences are a
    /// reply the connection discards and, on a transport with ordered replies, a
    /// resynchronisation the driver performs itself before the next request (never a
    /// lost sample). The connection stays usable. The deadline source is the request's
    /// deadline or `timeouts.request`.
    ///
    /// # Wire effects
    /// Transmits one or more read requests (a driver may split a batch and discloses it
    /// in [`ReadResponse::split_into`]). If the call is cancelled or times out after
    /// transmission, the device has seen the requests and its replies are discarded.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse>;

    /// Write every item of `request` and correlate the outcomes.
    ///
    /// A write is never retried and never replayed. An item whose delivery cannot be
    /// confirmed after transmission is reported as
    /// [`WriteOutcome::Unknown`](crate::WriteOutcome::Unknown), not as a rejection, and
    /// the application decides what to do (SPEC.md 'Errors', ADR-0010). A connection
    /// whose [`AccessPolicy`](crate::AccessPolicy) is read-only refuses the whole request
    /// before anything is transmitted. The deadline rule is the same as for
    /// [`read`](Self::read).
    ///
    /// # Errors
    /// Returns [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) for
    /// [`Operation::Write`](crate::Operation::Write) when writes are not supported or the
    /// access policy is read-only, and the same kinds as [`read`](Self::read) for a closed
    /// connection, a foreign address, a reached concurrency limit, a cancelled token and
    /// an elapsed deadline. Item-level refusals are
    /// [`WriteOutcome::Rejected`](crate::WriteOutcome::Rejected) entries in the response.
    ///
    /// # Cancel safety
    /// Not cancel-safe once the driver has transmitted: the device may have applied the
    /// write. Cancelling through the request's token after transmission still resolves
    /// the future, with [`WriteOutcome::Unknown`](crate::WriteOutcome::Unknown); dropping
    /// the future instead loses that outcome, so a caller that needs it cancels through
    /// the token. Before transmission, cancelling sends nothing. The deadline source is
    /// the request's deadline or `timeouts.request`.
    ///
    /// # Wire effects
    /// Transmits write requests. If the call is cancelled or times out after
    /// transmission, the device may or may not have applied the values; the response says
    /// `Unknown` and the library never replays the write.
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse>;

    /// Release the connection: pending work ends, the transport is shut down and the
    /// state becomes [`Closed`](ConnectionState::Closed).
    ///
    /// Idempotent: closing a closed connection succeeds. Operations started afterwards
    /// fail with [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected). The whole
    /// shutdown is bounded by `timeouts.request`; when it passes the connection is
    /// released without a clean protocol goodbye and the call still ends in `Closed`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) only when a driver
    /// reports that the clean shutdown did not finish in time. The connection is
    /// [`Closed`](ConnectionState::Closed) in either case.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future leaves the connection in
    /// [`Closing`](ConnectionState::Closing) or [`Closed`](ConnectionState::Closed); call
    /// `close` again to finish. Nothing is lost, because `close` is idempotent.
    ///
    /// # Wire effects
    /// May send a protocol goodbye and shuts the transport down. Pending requests are not
    /// replayed.
    fn close(&self) -> OpFuture<'_, ()>;

    /// Check that the peer answers, without reading or writing tagged values.
    ///
    /// The default implementation performs no I/O and fails with
    /// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) for
    /// [`Operation::Ping`]; a driver that can ping overrides it and sets
    /// [`Capabilities::ping`]. Like [`close`](Self::close) the call takes no request, so it
    /// has no deadline slot of its own: an override bounds its attempt by
    /// `timeouts.request`, and the way to cancel it is to drop the future.
    ///
    /// # Errors
    /// The default returns [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported)
    /// for [`Operation::Ping`], with `Diagnostics` naming the operation, the connection
    /// state and the `Validation` category. An override may also return
    /// [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) when the connection is
    /// not open and [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) when the peer does
    /// not answer within `timeouts.request`.
    ///
    /// # Cancel safety
    /// The default is cancel-safe: it does no I/O and resolves at once. An override must
    /// stay cancel-safe, because dropping the future abandons the attempt and a ping
    /// changes nothing on the device.
    ///
    /// # Wire effects
    /// The default sends nothing. An override sends one protocol-level probe and never a
    /// read or write of a tagged value.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-CONNECTION-TRAIT
    /// Corresponds to the upstream `PlcConnection.ping()`, which also takes no argument.
    /// The default reports `Unsupported(Operation::Ping)`; see the parity row for the
    /// Modbus driver.
    ///
    /// # Examples
    /// A caller can tell "no ping" from "no answer" by the error kind.
    /// ```
    /// use stonecrab::{Connection, ErrorKind, Operation, Result};
    ///
    /// async fn answers(connection: &dyn Connection) -> Result<Option<bool>> {
    ///     match connection.ping().await {
    ///         Ok(()) => Ok(Some(true)),
    ///         Err(error) if error.kind() == ErrorKind::Unsupported(Operation::Ping) => Ok(None),
    ///         Err(error) if error.kind() == ErrorKind::Timeout => Ok(Some(false)),
    ///         Err(error) => Err(error),
    ///     }
    /// }
    /// ```
    fn ping(&self) -> OpFuture<'_, ()> {
        Box::pin(async move {
            Err(unsupported(
                Operation::Ping,
                self.state(),
                "this connection does not implement ping",
            ))
        })
    }

    /// Start delivering events for every tag of `request` into a bounded queue.
    ///
    /// The default implementation performs no I/O and fails with
    /// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) for
    /// [`Operation::Subscribe`], whatever the request holds; a driver that can subscribe
    /// overrides it, sets [`Capabilities::subscribe`] and returns the [`Subscription`]
    /// made with [`SubscriptionSender`](crate::SubscriptionSender). An override follows the
    /// default-deadline and cancellation rules of [`read`](Self::read) for the call itself;
    /// the subscription then lives until it is unsubscribed or dropped, or until the
    /// connection closes or fails (ADR-0020). A polling override discloses its sampling
    /// interval through [`Subscription::interval`] and refuses
    /// [`SubscriptionKind::Event`](crate::SubscriptionKind::Event).
    ///
    /// # Errors
    /// The default returns [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported) for
    /// [`Operation::Subscribe`], with `Diagnostics` naming the operation, the connection
    /// state and the `Validation` category. An override may also return the kinds of
    /// [`read`](Self::read) for a closed connection, a foreign address, a cancelled token
    /// and an elapsed deadline, and `Unsupported(Operation::Subscribe)` for a kind it
    /// cannot deliver.
    ///
    /// # Cancel safety
    /// The default is cancel-safe: it does no I/O and resolves at once. Dropping an
    /// override's future before it resolves abandons the subscription; an override that
    /// already registered it on the device removes it again.
    ///
    /// # Wire effects
    /// The default sends nothing. A polling override sends nothing until its first tick
    /// and then reads at the disclosed interval; a native override sends the protocol's
    /// subscription request.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-CONNECTION-TRAIT
    /// Corresponds to the upstream `PlcConnection.subscriptionRequestBuilder()` and the
    /// execution of the request it builds. The default reports
    /// `Unsupported(Operation::Subscribe)`; the request and the subscription are
    /// `docs/parity/core.md#CORE-SUBSCRIPTION`.
    ///
    /// # Examples
    /// A caller falls back to reading when the connection cannot subscribe.
    /// ```
    /// use stonecrab::{Connection, ErrorKind, Operation, Result, SubscribeRequest, Subscription};
    ///
    /// async fn try_subscribe(
    ///     connection: &dyn Connection,
    ///     request: SubscribeRequest,
    /// ) -> Result<Option<Subscription>> {
    ///     match connection.subscribe(request).await {
    ///         Ok(subscription) => Ok(Some(subscription)),
    ///         Err(error) if error.kind() == ErrorKind::Unsupported(Operation::Subscribe) => Ok(None),
    ///         Err(error) => Err(error),
    ///     }
    /// }
    /// ```
    fn subscribe(&self, request: SubscribeRequest) -> OpFuture<'_, Subscription> {
        drop(request);
        Box::pin(async move {
            Err(unsupported(
                Operation::Subscribe,
                self.state(),
                "this connection does not implement subscribe",
            ))
        })
    }
}

/// The error of a provided method a connection does not override: no I/O happened, so the
/// latency is zero and the category is `Validation`.
fn unsupported(operation: Operation, state: ConnectionState, message: &'static str) -> PlcError {
    let diagnostics = Diagnostics {
        operation: Some(operation),
        connection_state: Some(state),
        latency: Some(Duration::ZERO),
        category: Some(FailureCategory::Validation),
        ..Diagnostics::default()
    };
    PlcError::new(ErrorKind::Unsupported(operation), message).with_diagnostics(diagnostics)
}
