//! `ModbusConnection`: the `Connection` of the `modbus-tcp` scheme, over a [`ModbusSession`].

use std::any::Any;
use std::fmt;
use std::num::{NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use stonecrab_core::{
    ByteOrder, CancellationToken, Capabilities, Clock, Connection, ConnectionConfig,
    ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, ItemError, OpFuture,
    Operation, PlcError, PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Result,
    RetryPolicy, StateWatch, SubscriptionMode, Timeouts, WordOrder, WriteRequest, WriteResponse,
};
use stonecrab_transport::{GuardConfig, TransportFactory};
use tracing::Instrument;

use crate::{
    BatchLimits, Batcher, Chunk, MAX_PDU_BYTES, Member, ModbusAddress, ModbusAdu, ModbusArea,
    ModbusPdu, ModbusSession, TypedDecoder, UnitId, exception_item_error,
};

/// The only scheme this connection serves.
const SCHEME: &str = "modbus-tcp";

/// The most register or coil bytes an item error keeps for inspection (the PDU bound).
const RAW_BUDGET: NonZeroUsize = match NonZeroUsize::new(MAX_PDU_BYTES) {
    Some(budget) => budget,
    None => NonZeroUsize::MIN,
};

/// The capabilities of the alpha driver and of every connection it opens: read only, no
/// subscriptions, browse or discovery, one operation at a time, PDUs up to 253 bytes
/// (`MB-TCP-CAPABILITIES`). The driver reports the same value before any connection exists.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn alpha_capabilities() -> Capabilities {
    let mut capabilities = Capabilities::default();
    capabilities.read = true;
    capabilities.subscribe = SubscriptionMode::Unsupported;
    capabilities.max_pdu_bytes = NonZeroUsize::new(MAX_PDU_BYTES);
    capabilities.concurrency = NonZeroUsize::MIN;
    capabilities
}

/// The connection-wide defaults a tag or an address may override: which unit answers when an
/// address names none, and how multi-byte values are laid out when neither the tag nor its
/// address says.
///
/// The three fields are the connection parameters `default-unit-identifier` and
/// `default-payload-byte-order` of the upstream driver. The byte order is the pair
/// ([`ByteOrder`], [`WordOrder`]) that the four upstream names map to (ADR-0006), applied
/// field by field: a tag's own `byte_order` or `word_order` wins over the address block's,
/// which wins over the value here. The `Default` is the upstream default: unit 1 and
/// `BIG_ENDIAN` (big-endian bytes, high word first).
///
/// The struct is `#[non_exhaustive]`; start from [`Default`] and assign the fields. The
/// `modbus-tcp` driver fills it from the connection string (slice S26).
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
/// Tag field over address block over `default-payload-byte-order`
/// (docs/parity/modbus.md#MB-CFG-BYTE-ORDER); the unit follows
/// docs/parity/modbus.md#MB-CFG-UNIT-ID.
///
/// # Examples
/// ```
/// use stonecrab_core::{ByteOrder, WordOrder};
/// use stonecrab_modbus::{ConnectionDefaults, UnitId};
///
/// let mut defaults = ConnectionDefaults::default();
/// assert_eq!(defaults.unit_id, UnitId::DEFAULT);
/// assert_eq!(
///     (defaults.byte_order, defaults.word_order),
///     (ByteOrder::BigEndian, WordOrder::HighWordFirst)
/// );
///
/// // LITTLE_ENDIAN, upstream's second mode.
/// defaults.byte_order = ByteOrder::LittleEndian;
/// defaults.word_order = WordOrder::LowWordFirst;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectionDefaults {
    /// The unit a read addresses when the tag's address names none.
    pub unit_id: UnitId,
    /// The byte order of a value whose tag and address name none.
    pub byte_order: ByteOrder,
    /// The word order of a value whose tag and address name none.
    pub word_order: WordOrder,
}

impl Default for ConnectionDefaults {
    fn default() -> Self {
        Self {
            unit_id: UnitId::DEFAULT,
            byte_order: ByteOrder::BigEndian,
            word_order: WordOrder::HighWordFirst,
        }
    }
}

/// A live Modbus TCP connection: reads coils, discrete inputs, holding registers and input
/// registers as typed values, in correlated batches.
///
/// `ModbusConnection` is what the `modbus-tcp` driver hands out as a `dyn Connection`
/// (slice S26). It owns a [`ModbusSession`] (transaction ids, reply correlation, the
/// request-driven reconnect of the connection guard underneath) and adds the read
/// semantics of the [`Connection`] contract:
///
/// - **Batching.** A read request is planned by [`Batcher::plan`] into chunks, one wire
///   request each (function codes 01 to 04, at most 125 registers or 2000 bits), and the
///   chunks run one after another. `ReadResponse::split_into` is the number of chunks.
///   Every item is decoded from its chunk's reply with [`TypedDecoder`], with the byte and
///   word order of the tag, else of its address, else of [`ConnectionDefaults`].
/// - **Failure scope.** A device exception answers a whole chunk: every member of that
///   chunk gets [`exception_item_error`] and the next chunk still runs. A reply whose data
///   does not decode fails only that member (`MalformedData`). A chunk whose exchange fails
///   (timeout, lost connection, cancellation, malformed frame) fails the whole read with
///   that error, because the items of the other chunks would be a partial snapshot the
///   caller did not ask for.
/// - **Time.** A request without a deadline gets `timeouts.request` per chunk; one with a
///   deadline is bounded by it. Each attempt of each chunk runs under
///   `min(request deadline, now + timeouts.request)` computed when the attempt starts, so a
///   long batch does not starve its last chunk and a retry has time to run. Item
///   `sampled_at` is the clock reading when the item's chunk was answered; `source_time`
///   is always `None` because Modbus carries no device timestamp.
/// - **Retry.** A failed chunk is retried under the configuration's [`RetryPolicy`] (one
///   attempt unless the caller raised `max_attempts`), only for the kinds in its
///   `retry_on` set, and only while the request's own deadline has not passed. A device
///   exception is an answer, not a failure, and is never retried. `write` does no I/O and
///   so never retries.
/// - **One read in flight.** `Capabilities.concurrency` is 1: a read that starts while
///   another runs fails with [`ErrorKind::Backpressure`] before any I/O, as the simulator's
///   does. The session underneath is therefore never asked for two exchanges at once, and
///   the chunks of one read are the only requests the device sees.
/// - **Lifecycle.** `Connected` after [`open`](Self::open); a transport failure shows as
///   `Reconnecting { attempt, next_at }` and the next read waits out the backoff and
///   reopens (decision D15); `close` ends in `Closed`. A read on a `Closing`, `Closed` or
///   `Failed` connection fails with `Disconnected` before any I/O.
/// - **Tracing.** A `debug` span `read` covers each read with `scheme = "modbus-tcp"`,
///   `endpoint` (host and port, never credentials), `op = "read"`, `items` and `chunks`;
///   the session's `request` span nests inside it with the `txn` of the chunk in flight.
///   `open` and `close` have `info` spans of the same shape.
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
/// The counterpart of upstream `ModbusTcpConnection.onRead`: blocks run one after another
/// (`MB-READ-MERGE`), one request is in flight (`MB-READ-INFLIGHT`), a request times out
/// after `request-timeout-ms` (`MB-READ-TIMEOUT`). Reconnect (`MB-READ-RECONNECT`) and read
/// retry (`MB-READ-RETRY`) are Stonecrab extensions; the default policy sends once, as
/// upstream does.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{
///     AddressOptions, Connection, ConnectionConfig, ConnectionState, Endpoint, PlcValue,
///     ReadRequest, Tag, TagId,
/// };
/// use stonecrab_modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
/// use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
///
/// // Modbus Application Protocol V1.1b section 6.3: holding register 108 is 555.
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::strict(vec![
///     Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0x00, 0x6B, 0x00, 0x01]),
///     Step::Reply(vec![0, 1, 0, 0, 0, 5, 1, 3, 2, 0x02, 0x2B]),
/// ]));
/// let config = ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
///     .build()?;
/// let connection = block_on_with(
///     &clock,
///     ModbusConnection::open(
///         config,
///         Arc::new(factory),
///         Arc::new(clock.clone()),
///         ConnectionDefaults::default(),
///     ),
/// )?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
///
/// let address = ModbusAddress::parse("holding-register:108", &AddressOptions::default())?;
/// let tag = Tag::new(TagId::new("level")?, Arc::new(address));
/// let request = ReadRequest::builder(connection.limits()).tag(tag).build()?;
/// let response = block_on_with(&clock, connection.read(request))?;
/// assert_eq!(response.items()[0].outcome().as_ref().ok(), Some(&PlcValue::I16(555)));
/// assert_eq!(response.split_into().get(), 1);
///
/// block_on_with(&clock, connection.close())?;
/// assert_eq!(connection.state(), ConnectionState::Closed);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
pub struct ModbusConnection {
    session: ModbusSession,
    clock: Arc<dyn Clock>,
    /// `host:port` of the endpoint, for spans; it carries no credentials.
    endpoint: String,
    timeouts: Timeouts,
    retry: RetryPolicy,
    limits: ResourceLimits,
    batch: BatchLimits,
    defaults: ConnectionDefaults,
    capabilities: Capabilities,
    /// True while a read is in flight; see [`Slot`].
    in_flight: AtomicBool,
}

/// Shows the endpoint and the state, never a payload.
impl fmt::Debug for ModbusConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusConnection")
            .field("endpoint", &self.endpoint)
            .field("state", &self.session.state())
            .finish_non_exhaustive()
    }
}

impl ModbusConnection {
    /// Build the connection from `config` and open it: `Disconnected` to `Connecting` to
    /// `Connected`.
    ///
    /// `config` must be for the scheme `modbus-tcp`; its endpoint, timeouts, reconnect
    /// policy, retry policy and limits are used, its access policy and credentials are
    /// not (Modbus TCP has no authentication, and a write is refused whatever the policy).
    /// `factory` opens the transports: the real TCP factory in production, the testkit's
    /// `FakeTransportFactory` in tests. `clock` is the one source of time. `defaults`
    /// carries the connection-wide unit and byte order (see [`ConnectionDefaults`]).
    ///
    /// The deadline source is `timeouts.connect`, applied by the guard to the open.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-CFG-REQUEST-TIMEOUT
    /// The request timeout of the configuration becomes the per-chunk deadline bound.
    ///
    /// # Errors
    /// - [`ErrorKind::InvalidConfig`] if the scheme is not `modbus-tcp`, or
    ///   `limits.max_frame_bytes` is below the 260 bytes of the largest Modbus TCP frame;
    ///   nothing is opened.
    /// - [`ErrorKind::Timeout`] if the open did not finish within `timeouts.connect`.
    /// - [`ErrorKind::Disconnected`] if the peer refused the connection.
    /// - Any other kind the transport factory reports.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped call leaves no transport behind and no connection to close.
    ///
    /// # Wire effects
    /// Opens a transport and sends no application bytes, bounded by `timeouts.connect`.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{Connection, ConnectionConfig, ConnectionState, Endpoint, ErrorKind};
    /// use stonecrab_modbus::{ConnectionDefaults, ModbusConnection};
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let endpoint = Endpoint::Custom("fake://plc".to_owned());
    ///
    /// // Any scheme but modbus-tcp is refused before a transport is opened.
    /// let wrong = ConnectionConfig::builder("sim", endpoint.clone()).build()?;
    /// let refused = block_on_with(
    ///     &clock,
    ///     ModbusConnection::open(
    ///         wrong,
    ///         Arc::new(factory.clone()),
    ///         Arc::new(clock.clone()),
    ///         ConnectionDefaults::default(),
    ///     ),
    /// )
    /// .err()
    /// .map(|error| error.kind());
    /// assert_eq!(refused, Some(ErrorKind::InvalidConfig));
    /// assert!(factory.opens().is_empty());
    ///
    /// let config = ConnectionConfig::builder("modbus-tcp", endpoint).build()?;
    /// let connection = block_on_with(
    ///     &clock,
    ///     ModbusConnection::open(
    ///         config,
    ///         Arc::new(factory),
    ///         Arc::new(clock.clone()),
    ///         ConnectionDefaults::default(),
    ///     ),
    /// )?;
    /// assert_eq!(connection.state(), ConnectionState::Connected);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub fn open(
        config: ConnectionConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
        defaults: ConnectionDefaults,
    ) -> OpFuture<'static, Self> {
        let span = tracing::info_span!(
            "connect",
            scheme = SCHEME,
            endpoint = %config.endpoint(),
            op = "connect"
        );
        Box::pin(
            async move {
                let connection = Self::build(&config, factory, clock, defaults)?;
                connection.session.connect().await?;
                tracing::info!("modbus connection established");
                Ok(connection)
            }
            .instrument(span),
        )
    }

    /// The connection in the `Disconnected` state, after checking the configuration.
    fn build(
        config: &ConnectionConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
        defaults: ConnectionDefaults,
    ) -> Result<Self> {
        if config.scheme() != SCHEME {
            return Err(invalid_config("the connection scheme must be modbus-tcp"));
        }
        let limits = *config.limits();
        if limits.max_frame_bytes().get() < ModbusAdu::MAX_BYTES {
            return Err(invalid_config(
                "max_frame_bytes must be at least 260, the largest Modbus TCP frame",
            ));
        }
        let wire = BatchLimits::default();
        let (registers, coils) = (wire.max_registers(), wire.max_coils());
        // The protocol maxima are valid bounds, so the constructor cannot refuse them; the
        // default stands in for the impossible refusal.
        let batch =
            BatchLimits::new(registers, coils, limits.max_items_per_request()).unwrap_or(wire);
        let session = ModbusSession::new(GuardConfig::from(config), factory, Arc::clone(&clock));
        Ok(Self {
            session,
            clock,
            endpoint: config.endpoint().to_string(),
            timeouts: *config.timeouts(),
            retry: config.retry().clone(),
            limits,
            batch,
            defaults,
            capabilities: alpha_capabilities(),
            in_flight: AtomicBool::new(false),
        })
    }

    /// Claim the single in-flight slot or fail with `Backpressure`.
    fn acquire(&self, op: &Op<'_>) -> Result<Slot<'_>> {
        // Acquire on success pairs with the Release store in `Slot::drop`, so the next
        // holder sees everything the previous read did to the session before it let go.
        self.in_flight
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| Slot(&self.in_flight))
            .map_err(|_busy| {
                op.reject(
                    ErrorKind::Backpressure,
                    "another read is in flight and the connection runs one at a time",
                )
            })
    }

    /// The checks that need no tag: the connection is open and the request not cancelled.
    fn gate(&self, op: &Op<'_>, request: &ReadRequest) -> Result<()> {
        let state = self.session.state();
        if matches!(
            state,
            ConnectionState::Closing | ConnectionState::Closed | ConnectionState::Failed(_)
        ) {
            return Err(op.error(
                ErrorKind::Disconnected,
                "the connection is closed or failed",
                FailureCategory::Transport,
            ));
        }
        if request
            .cancel()
            .is_some_and(CancellationToken::is_cancelled)
        {
            return Err(op.reject(ErrorKind::Cancelled, "the request was cancelled"));
        }
        Ok(())
    }

    /// Validate every tag and split the request into chunks, before any I/O.
    fn plan(&self, op: &Op<'_>, request: &ReadRequest) -> Result<Vec<Chunk>> {
        for tag in request.items() {
            let address: &dyn Any = tag.address();
            if !address.is::<ModbusAddress>() {
                return Err(op.reject(
                    ErrorKind::InvalidAddress,
                    format!("tag {} does not carry a Modbus address", tag.id()),
                ));
            }
        }
        for tag in request.items() {
            if tag.count() != NonZeroU32::MIN {
                return Err(op.reject(
                    ErrorKind::InvalidConfig,
                    format!(
                        "tag {} has element count {}: Modbus element counts above one need \
                         array selections, which are not supported yet",
                        tag.id(),
                        tag.count()
                    ),
                ));
            }
        }
        // The batcher refuses more tags than `max_items_per_request` before it plans, which
        // is the re-validation against the connection's own limits.
        Batcher::plan(request.items(), &self.batch)
            .map_err(|error| op.wrap(error, FailureCategory::Validation))
    }

    async fn read_inner(&self, request: ReadRequest, span: tracing::Span) -> Result<ReadResponse> {
        let op = Op {
            connection: self,
            started: self.clock.now(),
        };
        self.gate(&op, &request)?;
        let chunks = self.plan(&op, &request)?;
        span.record("chunks", chunks.len());
        let _slot = self.acquire(&op)?;
        if request
            .deadline()
            .is_some_and(|deadline| deadline.is_elapsed(self.clock.now()))
        {
            return Err(op.error(
                ErrorKind::Timeout,
                "the request deadline had already elapsed",
                FailureCategory::Transport,
            ));
        }
        let mut items = Vec::with_capacity(request.items().len());
        for (index, chunk) in chunks.iter().enumerate() {
            let answer = self
                .exchange(&op, &request, index, chunks.len(), chunk)
                .await?;
            items.extend(self.items_of(&op, chunk, &answer));
        }
        self.respond(&op, &request, items, chunks.len())
    }

    /// The correlated response for `items`, one per tag of `request`, answered by `chunks`
    /// wire requests.
    fn respond(
        &self,
        op: &Op<'_>,
        request: &ReadRequest,
        items: Vec<ReadItem>,
        chunks: usize,
    ) -> Result<ReadResponse> {
        let finished = self.clock.now();
        let split = u32::try_from(chunks)
            .ok()
            .and_then(NonZeroU32::new)
            .unwrap_or(NonZeroU32::MAX);
        let built = items
            .into_iter()
            .fold(
                ReadResponse::builder(request, op.started, finished),
                stonecrab_core::ReadResponseBuilder::item,
            )
            .split_into(split)
            .build();
        let response = built.map_err(|error| op.wrap(error, FailureCategory::Internal))?;
        let (items, failed) = (response.items().len(), response.iter_failed().count());
        tracing::debug!(items, failed, "read complete");
        Ok(response)
    }

    /// One chunk: send its read request, retrying under the policy, and say how it was
    /// answered.
    async fn exchange(
        &self,
        op: &Op<'_>,
        request: &ReadRequest,
        index: usize,
        total: usize,
        chunk: &Chunk,
    ) -> Result<Answer> {
        let pdu = read_pdu(chunk);
        let unit_id = chunk.unit_id.unwrap_or(self.defaults.unit_id);
        let mut attempts: u32 = 0;
        loop {
            attempts = attempts.saturating_add(1);
            let deadline = self.chunk_deadline(op, request.deadline())?;
            let outcome = self
                .session
                .request(&pdu, unit_id, deadline, request.cancel())
                .await;
            match outcome {
                Ok(reply) => {
                    return Ok(Answer {
                        reply,
                        arrived: self.clock.now(),
                        retries: attempts.saturating_sub(1),
                    });
                }
                Err(error) if self.may_retry(request, &error, attempts) => {
                    tracing::debug!(chunk = index, attempts, kind = ?error.kind(), "retrying the chunk");
                }
                Err(error) => return Err(self.chunk_failure(op, error, index, total, attempts)),
            }
        }
    }

    /// Whether the policy allows another attempt after `attempts` attempts failed with
    /// `error`, and the request's own deadline leaves room for it.
    fn may_retry(&self, request: &ReadRequest, error: &PlcError, attempts: u32) -> bool {
        self.retry.should_retry(error.kind(), attempts)
            && request
                .deadline()
                .is_none_or(|deadline| !deadline.is_elapsed(self.clock.now()))
    }

    /// `min(request deadline, now + timeouts.request)`, taken when an attempt starts.
    fn chunk_deadline(&self, op: &Op<'_>, requested: Option<Deadline>) -> Result<Deadline> {
        let bound = Deadline::after(self.clock.now(), self.timeouts.request)
            .map_err(|error| op.wrap(error, FailureCategory::Validation))?;
        Ok(requested.map_or(bound, |deadline| deadline.min(bound)))
    }

    /// The error of a chunk whose exchange failed, with the read's diagnostics.
    fn chunk_failure(
        &self,
        op: &Op<'_>,
        error: PlcError,
        index: usize,
        total: usize,
        attempts: u32,
    ) -> PlcError {
        let mut diagnostics = error.diagnostics().clone();
        diagnostics.operation = Some(Operation::Read);
        // The state now, not the guard's at the instant of the failure: a peer that closed
        // leaves the guard `Reconnecting` by the time the read reports it.
        diagnostics.connection_state = Some(self.session.state());
        diagnostics.latency = Some(op.elapsed());
        diagnostics.retries = attempts.saturating_sub(1);
        diagnostics
            .category
            .get_or_insert(FailureCategory::Transport);
        PlcError::new(
            error.kind(),
            format!("chunk {} of {total}: {error}", index.saturating_add(1)),
        )
        .with_diagnostics(diagnostics)
        .with_source(error)
    }

    /// One item per member of `chunk`, decoded from the chunk's answer.
    fn items_of(&self, op: &Op<'_>, chunk: &Chunk, answer: &Answer) -> Vec<ReadItem> {
        chunk
            .members
            .iter()
            .map(|member| {
                let outcome = decode_member(&answer.reply, chunk.area, member, self.defaults)
                    .map_err(|error| self.stamp_item(op, error, answer));
                ReadItem::new(member.tag.clone(), outcome, answer.arrived)
            })
            .collect()
    }

    /// `item` with the diagnostics a member error should carry: the operation, the state
    /// now, the time to the chunk's answer and the retries it took, over what the decoder
    /// or the exception mapping already said. The original stays as the source.
    fn stamp_item(&self, op: &Op<'_>, item: ItemError, answer: &Answer) -> ItemError {
        let mut diagnostics = item.error().diagnostics().clone();
        diagnostics.operation = Some(Operation::Read);
        diagnostics.connection_state = Some(self.session.state());
        diagnostics.latency = Some(answer.arrived.saturating_duration_since(op.started));
        diagnostics.retries = answer.retries;
        diagnostics
            .category
            .get_or_insert(FailureCategory::Protocol);
        let status = item.status().cloned();
        let kind = item.kind();
        let message = item.to_string();
        let stamped = ItemError::new(
            PlcError::new(kind, message)
                .with_diagnostics(diagnostics)
                .with_source(item),
        );
        status.into_iter().fold(stamped, ItemError::with_status)
    }
}

/// How a chunk was answered: the reply PDU, when it arrived and how many retries it took.
struct Answer {
    reply: ModbusPdu,
    arrived: Instant,
    retries: u32,
}

/// The read request PDU for `chunk`: function code by area, wire start and quantity.
const fn read_pdu(chunk: &Chunk) -> ModbusPdu {
    let (start, quantity) = (chunk.start, chunk.quantity);
    match chunk.area {
        ModbusArea::Coil => ModbusPdu::ReadCoils { start, quantity },
        ModbusArea::DiscreteInput => ModbusPdu::ReadDiscreteInputs { start, quantity },
        ModbusArea::HoldingRegister => ModbusPdu::ReadHoldingRegisters { start, quantity },
        ModbusArea::InputRegister => ModbusPdu::ReadInputRegisters { start, quantity },
    }
}

/// The value of one member in a chunk's reply, or the error that is the member's alone.
///
/// An exception reply fails every member the same way ([`exception_item_error`]). A data
/// reply is cut at the member's offset and decoded with the member's type and the order it
/// resolved to, `defaults` standing in for what the tag and address left unset. A reply of
/// the wrong shape for the area cannot come out of the session, which proves every reply
/// against its request; it is `MalformedData` here rather than a panic.
fn decode_member(
    reply: &ModbusPdu,
    area: ModbusArea,
    member: &Member,
    defaults: ConnectionDefaults,
) -> std::result::Result<PlcValue, ItemError> {
    let mismatch = || {
        ItemError::new(PlcError::new(
            ErrorKind::MalformedData,
            "the reply does not have the shape of the area that was read",
        ))
    };
    let offset = usize::from(member.offset);
    match reply {
        ModbusPdu::Exception { function, code } => Err(exception_item_error(*function, *code)),
        ModbusPdu::ReadBitsResponse { bytes, .. } if area.is_bit_area() => {
            TypedDecoder::decode_bits(bytes, offset, member.data_type, RAW_BUDGET)
        }
        ModbusPdu::ReadRegistersResponse { registers, .. } if !area.is_bit_area() => {
            let end = offset.saturating_add(usize::from(member.data_type.register_size()));
            // A reply too short for the member decodes as the wrong register count, which
            // keeps the bytes that did arrive.
            let words = registers.get(offset..end).unwrap_or(&[]);
            TypedDecoder::decode_registers(
                words,
                member.data_type,
                member.byte_order.unwrap_or(defaults.byte_order),
                member.word_order.unwrap_or(defaults.word_order),
                RAW_BUDGET,
            )
        }
        ModbusPdu::ReadBitsResponse { .. }
        | ModbusPdu::ReadRegistersResponse { .. }
        | ModbusPdu::ReadCoils { .. }
        | ModbusPdu::ReadDiscreteInputs { .. }
        | ModbusPdu::ReadHoldingRegisters { .. }
        | ModbusPdu::ReadInputRegisters { .. } => Err(mismatch()),
    }
}

/// An `InvalidConfig` error of opening the connection.
fn invalid_config(message: &'static str) -> PlcError {
    let mut diagnostics = Diagnostics::default();
    diagnostics.operation = Some(Operation::Connect);
    diagnostics.category = Some(FailureCategory::Validation);
    PlcError::new(ErrorKind::InvalidConfig, message).with_diagnostics(diagnostics)
}

/// Ownership of the single in-flight slot; dropping it, even when the future is dropped
/// mid-exchange, releases the slot.
struct Slot<'a>(&'a AtomicBool);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// The context every error of one read is stamped with.
struct Op<'a> {
    connection: &'a ModbusConnection,
    /// The clock reading when the read began, the base of every latency figure.
    started: Instant,
}

impl Op<'_> {
    fn elapsed(&self) -> Duration {
        self.connection
            .clock
            .now()
            .saturating_duration_since(self.started)
    }

    fn diagnostics(&self, category: FailureCategory) -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        diagnostics.operation = Some(Operation::Read);
        diagnostics.connection_state = Some(self.connection.session.state());
        diagnostics.latency = Some(self.elapsed());
        diagnostics.category = Some(category);
        diagnostics
    }

    fn error(
        &self,
        kind: ErrorKind,
        message: impl Into<String>,
        category: FailureCategory,
    ) -> PlcError {
        PlcError::new(kind, message).with_diagnostics(self.diagnostics(category))
    }

    /// A rejection before the exchange begins.
    fn reject(&self, kind: ErrorKind, message: impl Into<String>) -> PlcError {
        self.error(kind, message, FailureCategory::Validation)
    }

    /// Re-wrap an error from core or the batcher with this read's diagnostics.
    fn wrap(&self, error: PlcError, category: FailureCategory) -> PlcError {
        PlcError::new(error.kind(), error.to_string())
            .with_diagnostics(self.diagnostics(category))
            .with_source(error)
    }
}

impl Connection for ModbusConnection {
    fn state(&self) -> ConnectionState {
        self.session.state()
    }

    fn watch_state(&self) -> StateWatch {
        self.session.watch_state()
    }

    fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    fn limits(&self) -> ResourceLimits {
        self.limits
    }

    /// Read every tag of `request` in as few wire requests as the batcher allows.
    ///
    /// The request is checked before any I/O, in this order: the connection is open, the
    /// token is not cancelled, every tag carries a [`ModbusAddress`], no tag has an element
    /// count above one, the request is within `max_items_per_request` and the chunk limits.
    /// Then one read runs at a time, the request deadline is checked, and the chunks run in
    /// order. See the type documentation for batching, failure scope, deadlines and retry.
    ///
    /// The deadline source is the request's deadline, or `timeouts.request` of the
    /// configuration, bounded per chunk attempt as the type documentation describes.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
    /// Results come back correlated and in request order with `split_into` disclosure
    /// (`MB-READ-MERGE`); chunks run one after another; a chunk exception fails only its
    /// members (`MB-EXC-01`); one request is in flight (`MB-READ-INFLIGHT`).
    ///
    /// # Errors
    /// Every error carries `Diagnostics` with `operation = Read`, the connection state, the
    /// latency, the retries performed and a category. A failure of a chunk's exchange names
    /// the chunk in its message.
    /// - [`ErrorKind::Disconnected`] if the connection is `Closing`, `Closed` or `Failed`
    ///   (nothing sent), or the peer closed during the read.
    /// - [`ErrorKind::Cancelled`] if the request's token fired: nothing was sent when it
    ///   fired before the first flush.
    /// - [`ErrorKind::InvalidAddress`] if a tag's address is not a Modbus address.
    /// - [`ErrorKind::InvalidConfig`] if a tag's element count is not 1.
    /// - [`ErrorKind::ResourceLimit`] if the request has more items than
    ///   `max_items_per_request`.
    /// - [`ErrorKind::Backpressure`] if another read is in flight; nothing was sent.
    /// - [`ErrorKind::Timeout`] if the request deadline had passed, or a chunk's deadline
    ///   passed and the retry policy gave up.
    /// - [`ErrorKind::MalformedData`] if a reply violated the framing or did not answer
    ///   its request. A reply whose data does not decode is an item error instead.
    ///
    /// # Cancel safety
    /// Cancel-safe in the connection's own state: a dropped future releases the single
    /// in-flight slot. If a chunk's request frame had gone out, its reply may still
    /// arrive; the session discards it as late and the guard resynchronizes before the
    /// next request. A read is a pure query, so dropping it changes nothing on the
    /// device. Cancelling the request's token resolves the call with
    /// [`ErrorKind::Cancelled`].
    ///
    /// # Wire effects
    /// Transmits one read request frame (function code 01, 02, 03 or 04) per chunk, and
    /// again per retry. Nothing is ever written to the device.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        let span = tracing::debug_span!(
            "read",
            scheme = SCHEME,
            endpoint = %self.endpoint,
            op = "read",
            items = request.items().len(),
            chunks = tracing::field::Empty
        );
        Box::pin(self.read_inner(request, span.clone()).instrument(span))
    }

    /// Refuse the write: `modbus-tcp` is read-only at alpha.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CAPABILITIES
    /// Upstream implements writes; Stonecrab defers them (decision D3), and
    /// `Capabilities.write` is false.
    ///
    /// # Errors
    /// - [`ErrorKind::Unsupported`]`(`[`Operation::Write`]`)` always, with `Diagnostics`
    ///   `operation = Write` and the `Validation` category.
    ///
    /// # Cancel safety
    /// Cancel-safe: the call does no I/O and resolves at once.
    ///
    /// # Wire effects
    /// None. Nothing is sent, so the outcome is never unknown.
    fn write(&self, _request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        let span = tracing::debug_span!(
            "write",
            scheme = SCHEME,
            endpoint = %self.endpoint,
            op = "write"
        );
        Box::pin(
            async move {
                let mut diagnostics = Diagnostics::default();
                diagnostics.operation = Some(Operation::Write);
                diagnostics.connection_state = Some(self.session.state());
                diagnostics.latency = Some(Duration::ZERO);
                diagnostics.category = Some(FailureCategory::Validation);
                Err(PlcError::new(
                    ErrorKind::Unsupported(Operation::Write),
                    "the modbus-tcp driver is read-only at alpha",
                )
                .with_diagnostics(diagnostics))
            }
            .instrument(span),
        )
    }

    /// Close the connection: `Closing`, then `Closed`.
    ///
    /// A read in flight fails with [`ErrorKind::Disconnected`], and so does a read that is
    /// waiting out a reconnect backoff. The call is idempotent. The deadline source is
    /// `timeouts.request`, applied by the guard to the shutdown.
    ///
    /// # Errors
    /// - [`ErrorKind::Timeout`] if the shutdown did not finish within `timeouts.request`;
    ///   the state is `Closed` regardless. The failure carries `Diagnostics` with
    ///   `operation = Close`.
    ///
    /// # Cancel safety
    /// Cancel-safe: the state is already `Closing`, and a repeat call finishes the job.
    ///
    /// # Wire effects
    /// Half-closes the transport (a FIN for TCP); no application bytes are sent.
    fn close(&self) -> OpFuture<'_, ()> {
        let span = tracing::info_span!(
            "close",
            scheme = SCHEME,
            endpoint = %self.endpoint,
            op = "close"
        );
        Box::pin(self.session.close().instrument(span))
    }
}

#[cfg(test)]
mod tests {
    use stonecrab_core::{AddressOptions, Endpoint, Tag, TagId};
    use stonecrab_testkit::{FakeClock, FakeTransportFactory};

    use super::*;
    use crate::ModbusDataType;

    fn member(data_type: ModbusDataType, offset: u16) -> Member {
        Member {
            tag: TagId::new("m").expect("tag id"),
            offset,
            data_type,
            byte_order: None,
            word_order: None,
        }
    }

    // covers: ALPHA_PLAN S25 / a reply of the wrong shape is MalformedData, not a panic
    #[test]
    fn a_reply_of_the_wrong_shape_for_the_area_fails_the_member() {
        let defaults = ConnectionDefaults::default();
        let registers = ModbusPdu::ReadRegistersResponse {
            function: crate::FunctionCode::ReadHoldingRegisters,
            registers: vec![1],
        };
        let bits = ModbusPdu::ReadBitsResponse {
            function: crate::FunctionCode::ReadCoils,
            bytes: vec![1],
        };
        let request = ModbusPdu::ReadCoils {
            start: 0,
            quantity: 1,
        };
        let int = member(ModbusDataType::Int, 0);
        let bool_member = member(ModbusDataType::Bool, 0);
        for (reply, area, member) in [
            (&registers, ModbusArea::Coil, &bool_member),
            (&bits, ModbusArea::HoldingRegister, &int),
            (&request, ModbusArea::Coil, &bool_member),
        ] {
            let error = decode_member(reply, area, member, defaults).expect_err("wrong shape");
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{area:?}");
        }
        let value = decode_member(&registers, ModbusArea::HoldingRegister, &int, defaults)
            .expect("the right shape decodes");
        assert_eq!(value, PlcValue::I16(1), "value");
        let value =
            decode_member(&bits, ModbusArea::Coil, &bool_member, defaults).expect("a bit decodes");
        assert_eq!(value, PlcValue::Bool(true), "bit");
        let refused = ModbusPdu::Exception {
            function: crate::FunctionCode::ReadCoils,
            code: crate::ExceptionCode::IllegalDataAddress,
        };
        let error = decode_member(&refused, ModbusArea::Coil, &bool_member, defaults)
            .expect_err("an exception fails the member");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
    }

    // covers: ALPHA_PLAN S25 / a reply too short for a member keeps the bytes that arrived
    #[test]
    fn a_reply_too_short_for_the_member_is_a_wrong_register_count() {
        let reply = ModbusPdu::ReadRegistersResponse {
            function: crate::FunctionCode::ReadHoldingRegisters,
            registers: vec![0x0001],
        };
        let dint = member(ModbusDataType::DInt, 0);
        let error = decode_member(
            &reply,
            ModbusArea::HoldingRegister,
            &dint,
            ConnectionDefaults::default(),
        )
        .expect_err("one register is not a DINT");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "kind");
        let past_the_end = member(ModbusDataType::Int, 3);
        let error = decode_member(
            &reply,
            ModbusArea::HoldingRegister,
            &past_the_end,
            ConnectionDefaults::default(),
        )
        .expect_err("offset past the reply");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "past the end");
    }

    // covers: ALPHA_PLAN S25 / a response the builder refuses is an Internal error
    #[test]
    fn a_response_the_builder_refuses_is_an_internal_error() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let config =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .build()
                .expect("config");
        let connection = ModbusConnection::build(
            &config,
            Arc::new(factory),
            Arc::new(clock.clone()),
            ConnectionDefaults::default(),
        )
        .expect("connection");
        let address = ModbusAddress::parse("holding-register:1", &AddressOptions::default())
            .expect("address");
        let tag = Tag::new(TagId::new("a").expect("tag id"), Arc::new(address));
        let request = ReadRequest::builder(connection.limits())
            .tag(tag)
            .build()
            .expect("request");
        let op = Op {
            connection: &connection,
            started: clock.now(),
        };
        // No item for the one requested tag: the response is incomplete.
        let error = connection
            .respond(&op, &request, Vec::new(), 1)
            .expect_err("an incomplete response");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Internal),
            "category"
        );

        // With its item, the same request gets its response.
        let item = ReadItem::new(
            TagId::new("a").expect("tag id"),
            Ok(PlcValue::I16(1)),
            clock.now(),
        );
        let response = connection
            .respond(&op, &request, vec![item], 1)
            .expect("a complete response");
        assert_eq!(response.items().len(), 1, "one item");
        assert_eq!(response.split_into().get(), 1, "one chunk");
    }
}
