//! `ModbusConnection`: the `Connection` of the `modbus-tcp`, `modbus-rtu` and `modbus-ascii`
//! schemes, over a [`ModbusSession`].

use std::any::Any;
use std::fmt;
use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::codec::{BoundedWriter, ParseError, pack_bits_lsb_first};
use crate::transport::{GuardConfig, TransportFactory};
use crate::{
    AccessPolicy, ByteOrder, CancellationToken, Capabilities, Clock, Connection, ConnectionConfig,
    ConnectionState, Deadline, Diagnostics, ErrorKind, FailureCategory, ItemError, OpFuture,
    Operation, PlcError, PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Result,
    RetryPolicy, StateWatch, SubscribeRequest, Subscription, SubscriptionMode, Tag, Timeouts,
    UnknownReason, WordOrder, WriteItem, WriteOutcome, WriteRequest, WriteResponse,
    WriteResponseBuilder, WriteResult,
};
use tracing::Instrument;

use crate::modbus::address::element_width;
use crate::modbus::batch::resolve_order;
use crate::modbus::decode::{decode_text, encode_text, with_raw_reply};
use crate::modbus::session::{Wire, exception_error};
use crate::modbus::{
    BatchLimits, Batcher, Chunk, MAX_PDU_BYTES, Member, ModbusAddress, ModbusArea, ModbusOptions,
    ModbusPdu, ModbusSession, SessionStats, TypedDecoder, UnitId, encode_response,
    exception_item_error,
};
use crate::polling::{Lifeline, Plan};

/// The most register or coil bytes an item error keeps for inspection (the PDU bound).
const RAW_BUDGET: NonZeroUsize = match NonZeroUsize::new(MAX_PDU_BYTES) {
    Some(budget) => budget,
    None => NonZeroUsize::MIN,
};

/// The capabilities of the driver and of every connection it opens: read, write, ping and
/// polling subscriptions, no browse or discovery, one operation at a time, PDUs up to 253
/// bytes (`MB-TCP-CAPABILITIES`). The driver reports the same value before any connection
/// exists.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn driver_capabilities() -> Capabilities {
    Capabilities {
        read: true,
        write: true,
        ping: true,
        subscribe: SubscriptionMode::Polling,
        max_pdu_bytes: NonZeroUsize::new(MAX_PDU_BYTES),
        concurrency: NonZeroUsize::MIN,
        ..Capabilities::default()
    }
}

/// The connection-wide defaults a tag or an address may override, and the request-size
/// limits of the batcher: which unit answers when an address names none, how multi-byte
/// values are laid out when neither the tag nor its address says, and how many registers
/// or coils one wire request may carry.
///
/// The fields are the connection parameters `default-unit-identifier`,
/// `default-payload-byte-order`, `max-registers-per-request` and `max-coils-per-request`
/// of the upstream driver. The byte order is the pair ([`ByteOrder`], [`WordOrder`]) that
/// the four upstream names map to (ADR-0006), applied field by field: a tag's own
/// `byte_order` or `word_order` wins over the address block's, which wins over the value
/// here. The limits feed [`BatchLimits`] (the chunk bounds of the batcher); a value above
/// the protocol maximum of 125 registers or 2000 coils is refused when the connection
/// opens. The `Default` is the upstream default: unit 1, `BIG_ENDIAN` (big-endian bytes,
/// high word first), 125 registers and 2000 coils, and a single coil written with function
/// 15.
///
/// `single_coil_fc05` is a Stonecrab extension with no upstream counterpart, the connection
/// parameter `single-coil-write` (`MB-CFG-SINGLE-COIL`): when true, a write of exactly one
/// coil goes out as function 05 where upstream sends function 15. It is off by default.
///
/// The struct is `#[non_exhaustive]`; start from [`Default`] and assign the fields. The
/// `modbus-tcp` driver fills it from the connection string through
/// [`ModbusOptions`](crate::modbus::ModbusOptions).
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
/// Tag field over address block over `default-payload-byte-order`
/// (docs/parity/modbus.md#MB-CFG-BYTE-ORDER); the unit follows
/// docs/parity/modbus.md#MB-CFG-UNIT-ID and the limits follow
/// docs/parity/modbus.md#MB-CFG-MAX-REGISTERS and
/// docs/parity/modbus.md#MB-CFG-MAX-COILS.
///
/// # Examples
/// ```
/// use stonecrab::{ByteOrder, WordOrder};
/// use stonecrab::modbus::{ConnectionDefaults, UnitId};
///
/// let mut defaults = ConnectionDefaults::default();
/// assert_eq!(defaults.unit_id, UnitId::DEFAULT);
/// assert!(!defaults.single_coil_fc05, "a single coil is function 15, as upstream sends it");
/// assert_eq!(
///     (defaults.byte_order, defaults.word_order),
///     (ByteOrder::BigEndian, WordOrder::HighWordFirst)
/// );
/// assert_eq!((defaults.max_registers.get(), defaults.max_coils.get()), (125, 2000));
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
    /// The most coils or discrete inputs one wire request carries (at most 2000).
    pub max_coils: NonZeroU16,
    /// The most registers one wire request carries (at most 125).
    pub max_registers: NonZeroU16,
    /// Write a single coil with function 05 instead of function 15 (`single-coil-write`).
    pub single_coil_fc05: bool,
}

impl Default for ConnectionDefaults {
    fn default() -> Self {
        let wire = BatchLimits::default();
        Self {
            unit_id: UnitId::DEFAULT,
            byte_order: ByteOrder::BigEndian,
            word_order: WordOrder::HighWordFirst,
            max_coils: wire.max_coils(),
            max_registers: wire.max_registers(),
            single_coil_fc05: false,
        }
    }
}

/// A live Modbus TCP connection: reads coils, discrete inputs, holding registers and input
/// registers as typed values, in correlated batches, and writes coils and holding registers.
///
/// `ModbusConnection` is what the `modbus-tcp` driver hands out as a `dyn Connection`
/// (slice S26), and what the `modbus-rtu` and `modbus-ascii` drivers hand out too, over RTU
/// and ASCII framing ([`ModbusRtuDriver`](crate::modbus::ModbusRtuDriver),
/// [`ModbusAsciiDriver`](crate::modbus::ModbusAsciiDriver), ADR-0024): everything below
/// applies to all of them, with the serial differences those drivers document (no
/// transaction ids, the silence before each RTU request, no unit 0). It owns a
/// [`ModbusSession`] (transaction ids, reply correlation, the reconnect of the connection
/// guard underneath, in the background when the connection is opened inside a Tokio
/// runtime) and adds the read semantics of the [`Connection`] contract:
///
/// - **Batching.** A read request is planned by [`Batcher::plan`] into chunks, one wire
///   request each (function codes 01 to 04, at most 125 registers or 2000 bits; function
///   code 20 for extended registers, at most 120, cut at the end of each 10000-register
///   file into one sub-request per file whose data is joined in order), and the
///   chunks run one after another. `ReadResponse::split_into` is the number of chunks.
///   Every item is decoded from its chunk's reply with [`TypedDecoder`], with the byte and
///   word order of the tag, else of its address, else of [`ConnectionDefaults`].
/// - **Failure scope.** A device exception answers a whole chunk: every member of that
///   chunk gets [`exception_item_error`] and the next chunk still runs. So does a reply that
///   decodes but does not fit the request (a short reply: a byte count other than the
///   quantity needs): every member of the chunk fails with `MalformedData`, and the status
///   of each keeps the reply PDU as received
///   ([`ProtocolStatus::raw`](crate::ProtocolStatus::raw)). A reply whose data does not
///   decode fails only that member (`MalformedData`). A chunk whose exchange fails (timeout,
///   lost connection, cancellation, a frame or a PDU that cannot be decoded) fails the whole
///   read with that error, because the items of the other chunks would be a partial snapshot
///   the caller did not ask for. A reply that does not fit is not retried: it is an answer,
///   as an exception is.
/// - **Time.** A request without a deadline gets `timeouts.request` per chunk; one with a
///   deadline is bounded by it. Each attempt of each chunk runs under
///   `min(request deadline, now + timeouts.request)` computed when the attempt starts, so a
///   long batch does not starve its last chunk and a retry has time to run. Item
///   `sampled_at` is the clock reading when the item's chunk was answered; `source_time`
///   is always `None` because Modbus carries no device timestamp.
/// - **Retry.** A failed chunk is retried under the configuration's [`RetryPolicy`] (one
///   attempt unless the caller raised `max_attempts`), only for the kinds in its
///   `retry_on` set, and only while the request's own deadline has not passed. A device
///   exception is an answer, not a failure, and is never retried. A write is never
///   retried, whatever the policy says.
/// - **Writes.** [`write`](Connection::write) sends one request per item (function 15 for
///   a coil, 06 or 16 for holding registers, 21 for extended registers), one after another,
///   and gives each item its own outcome: `Confirmed` for an echo, `Rejected` for a device
///   exception or an item that cannot be written, `Unknown` for a request that went out and
///   got no usable answer. It is refused first while the access policy is read-only, which is the
///   default; `access=read-write` in the connection string, or
///   `ConnectionConfig::builder(..).access(AccessPolicy::ReadWrite)`, grants it.
/// - **One operation in flight.** `Capabilities.concurrency` is 1: a read, write or ping
///   that starts while another runs fails with [`ErrorKind::Backpressure`] before any I/O,
///   as the simulator's does. The session underneath is therefore never asked for two
///   exchanges at once, and the chunks of one read (or the items of one write) are the
///   only requests the device sees.
/// - **Lifecycle.** `Connected` after [`open`](Self::open); a transport failure shows as
///   `Reconnecting { attempt, next_at }` and the next read waits out the backoff and
///   reopens (decision D15); `close` ends in `Closed`. A read on a `Closing`, `Closed` or
///   `Failed` connection fails with `Disconnected` before any I/O.
/// - **Ping.** [`ping`](Connection::ping) sends one read of the configured ping address
///   (`ping-address` of [`ModbusOptions`], default `4x00001:BOOL`) through the same session,
///   the same admission rules and the same in-flight slot as a read, bounded by
///   `timeouts.request`, and never retries. Any reply that answers the request proves the
///   peer is alive, an exception reply included (`MB-QUIRK-D`); a silent peer is
///   [`ErrorKind::Timeout`].
/// - **Tracing.** A `debug` span `read` covers each read with `scheme = "modbus-tcp"`,
///   `endpoint` (host and port, never credentials), `op = "read"`, `items` and `chunks`;
///   the session's `request` span nests inside it with the `txn` of the chunk in flight.
///   `ping` has a `ping` span with `op = "ping"`; its nested `request` span says
///   `op = "read"` because a ping is a read on the wire.
///   `open` and `close` have `info` spans of the same shape, and `subscribe` a `debug`
///   span with `op = "subscribe"`; each polling read has its own `read` span.
/// - **Subscriptions.** [`subscribe`](Connection::subscribe) polls (ADR-0020, addendum
///   S55): every tick reads the subscription's tags through [`read`](Connection::read), with
///   its batching, deadline and retry, so a polling read takes the single in-flight slot
///   and a user read that starts meanwhile gets [`ErrorKind::Backpressure`], as before. A
///   tick that finds the slot taken is skipped, never queued.
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
/// The counterpart of upstream `ModbusTcpConnection.onRead`: blocks run one after another
/// (`MB-READ-MERGE`), one request is in flight (`MB-READ-INFLIGHT`), a request times out
/// after `request-timeout-ms` (`MB-READ-TIMEOUT`). Reconnect (`MB-READ-RECONNECT`) and read
/// retry (`MB-READ-RETRY`) are Stonecrab extensions; the default policy sends once, as
/// upstream does. Ping is the counterpart of `onPing` (`MB-TCP-PING`) and write of `onWrite`
/// (`MB-TCP-WRITE`).
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::{
///     AddressOptions, Connection, ConnectionConfig, ConnectionState, Endpoint, PlcValue,
///     ReadRequest, Tag, TagId,
/// };
/// use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
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
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct ModbusConnection {
    /// Everything the connection is, shared with the polling task of each subscription.
    link: Arc<Link>,
}

/// The state of one connection: what a [`ModbusConnection`] owns, and what its polling
/// subscriptions read through, holding it weakly so they never keep it alive.
struct Link {
    session: ModbusSession,
    /// The framing, and so the scheme the spans name.
    wire: Wire,
    clock: Arc<dyn Clock>,
    /// `host:port` of the endpoint, for spans; it carries no credentials.
    endpoint: String,
    timeouts: Timeouts,
    retry: RetryPolicy,
    limits: ResourceLimits,
    batch: BatchLimits,
    defaults: ConnectionDefaults,
    /// What `ping` reads once: the `ping-address` of the options in the configuration.
    ping_address: ModbusAddress,
    capabilities: Capabilities,
    /// What the configuration permits: a write on a read-only connection is refused first.
    access: AccessPolicy,
    /// True while a read, write or ping is in flight; see [`Slot`].
    in_flight: AtomicBool,
    /// Ends the polling subscriptions' wait for a tick when the connection is dropped.
    lifeline: Lifeline,
}

/// Shows the endpoint and the state, never a payload.
impl fmt::Debug for ModbusConnection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusConnection")
            .field("endpoint", &self.link.endpoint)
            .field("state", &self.link.session.state())
            .finish_non_exhaustive()
    }
}

impl ModbusConnection {
    /// Build the connection from `config` and open it: `Disconnected` to `Connecting` to
    /// `Connected`.
    ///
    /// `config` must be for the scheme `modbus-tcp`; its endpoint, timeouts, reconnect
    /// policy, retry policy, access policy and limits are used, its credentials are not
    /// (Modbus TCP has no authentication). The access policy is read-only unless the
    /// configuration says otherwise, and a read-only connection refuses every write.
    /// `factory` opens the transports: the real TCP factory in production, the testkit's
    /// `FakeTransportFactory` in tests. `clock` is the one source of time. `defaults`
    /// carries the connection-wide unit, byte order and request-size limits (see
    /// [`ConnectionDefaults`]). The address [`ping`](Connection::ping) reads is the
    /// `ping_address` of the [`ModbusOptions`] in the driver options of `config`, else the
    /// upstream default `4x00001:BOOL`.
    ///
    /// The deadline source is `timeouts.connect`, applied by the guard to the open.
    ///
    /// Opened inside a Tokio runtime, the connection starts the guard's supervisor
    /// (`ConnectionGuard::supervise`): a dropped link is reopened in the background
    /// following the reconnect policy, without waiting for a request, and `timeouts.idle`,
    /// when set, closes a link no request has used for that long (the next request reopens
    /// it). [`close`](Connection::close) stops and joins the task. Opened elsewhere (the
    /// testkit's executor), reconnect is driven by the next request instead.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-CFG-REQUEST-TIMEOUT
    /// The request timeout of the configuration becomes the per-chunk deadline bound.
    ///
    /// # Errors
    /// - [`ErrorKind::InvalidConfig`] if the scheme is not `modbus-tcp`, or
    ///   `limits.max_frame_bytes` is below the 260 bytes of the largest Modbus TCP frame, or
    ///   `defaults` names more than 125 registers or 2000 coils per request; nothing is
    ///   opened.
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
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{Connection, ConnectionConfig, ConnectionState, Endpoint, ErrorKind};
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusConnection};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
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
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn open(
        config: ConnectionConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
        defaults: ConnectionDefaults,
    ) -> OpFuture<'static, Self> {
        let ping_address = config
            .driver_options_as::<ModbusOptions>()
            .map_or(ModbusAddress::DEFAULT_PING, |options| {
                options.ping_address.clone()
            });
        Self::open_on(Wire::Tcp, config, factory, clock, defaults, ping_address)
    }

    /// A snapshot of the connection's reply counters, the session's and the guard's.
    ///
    /// [`late_replies`](SessionStats::late_replies) and
    /// [`mismatched_replies`](SessionStats::mismatched_replies) count the replies the session
    /// discarded while it waited for the answer to a request, and
    /// [`guard.discarded_frames`](crate::transport::GuardStats::discarded_frames) the frames
    /// the guard dropped between two exchanges (with the background reader of a connection
    /// opened inside a Tokio runtime, a late reply is usually dropped there, before any
    /// request sees it).
    /// The counters only grow, and a read or a write that fails does not reset them.
    ///
    /// The method is on the concrete type because the [`Connection`] trait has no
    /// statistics and no way back to the concrete type: a `Box<dyn Connection>` from
    /// [`DriverRegistry::connect`](crate::DriverRegistry::connect) or from a driver's
    /// [`connect`](crate::Driver::connect) cannot reach it. A caller that wants the
    /// counters opens the link as the concrete type: [`open`](Self::open) for Modbus TCP,
    /// [`ModbusRtuDriver::open`](crate::modbus::ModbusRtuDriver::open) and
    /// [`ModbusAsciiDriver::open`](crate::modbus::ModbusAsciiDriver::open) for the serial
    /// framings.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
    /// Stonecrab's own counters, part of the divergence recorded in that row: upstream drops
    /// an unknown transaction id with a log line and counts nothing.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{
    ///     AddressOptions, Connection, ConnectionConfig, Endpoint, ReadRequest, Tag, TagId,
    /// };
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// // A reply to another transaction comes first; the peer then answers the read.
    /// factory.push_script(Script::strict(vec![
    ///     Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0x00, 0x6B, 0x00, 0x01]),
    ///     Step::Reply(vec![0x03, 0xE8, 0, 0, 0, 5, 1, 3, 2, 0x00, 0x07]),
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
    /// assert_eq!(connection.stats().mismatched_replies, 0);
    ///
    /// let address = ModbusAddress::parse("holding-register:108", &AddressOptions::default())?;
    /// let tag = Tag::new(TagId::new("level")?, Arc::new(address));
    /// let request = ReadRequest::builder(connection.limits()).tag(tag).build()?;
    /// block_on_with(&clock, connection.read(request))?;
    ///
    /// let stats = connection.stats();
    /// assert_eq!((stats.late_replies, stats.mismatched_replies), (0, 1));
    /// assert_eq!(stats.guard.requests, 1);
    /// assert_eq!(stats.guard.discarded_frames, 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn stats(&self) -> SessionStats {
        self.link.session.stats()
    }

    /// [`open`](Self::open) for the framing `wire`, with the address `ping` reads; the
    /// serial drivers open their connections here.
    pub(crate) fn open_on(
        wire: Wire,
        config: ConnectionConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
        defaults: ConnectionDefaults,
        ping_address: ModbusAddress,
    ) -> OpFuture<'static, Self> {
        let span = tracing::info_span!(
            "connect",
            scheme = wire.scheme(),
            endpoint = %config.endpoint(),
            op = "connect"
        );
        Box::pin(
            async move {
                let link = Link::build(wire, &config, factory, clock, defaults, ping_address)?;
                link.session.connect().await?;
                let supervised = link.session.supervise();
                tracing::info!(supervised, "modbus connection established");
                Ok(Self {
                    link: Arc::new(link),
                })
            }
            .instrument(span),
        )
    }
}

impl Link {
    /// The connection in the `Disconnected` state, after checking the configuration.
    fn build(
        wire: Wire,
        config: &ConnectionConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
        defaults: ConnectionDefaults,
        ping_address: ModbusAddress,
    ) -> Result<Self> {
        let scheme = wire.scheme();
        if config.scheme() != scheme {
            return Err(invalid_config(format!(
                "the connection scheme must be {scheme}"
            )));
        }
        let limits = *config.limits();
        let largest = wire.max_frame_bytes();
        if limits.max_frame_bytes().get() < largest {
            return Err(invalid_config(format!(
                "max_frame_bytes must be at least {largest}, the largest {scheme} frame"
            )));
        }
        let batch = BatchLimits::new(
            defaults.max_registers,
            defaults.max_coils,
            limits.max_items_per_request(),
        )?;
        let session =
            ModbusSession::on_wire(wire, GuardConfig::from(config), factory, Arc::clone(&clock));
        Ok(Self {
            session,
            wire,
            clock,
            endpoint: config.endpoint().to_string(),
            timeouts: *config.timeouts(),
            retry: config.retry().clone(),
            limits,
            batch,
            defaults,
            ping_address,
            capabilities: driver_capabilities(),
            access: config.access(),
            in_flight: AtomicBool::new(false),
            lifeline: Lifeline::new(),
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
                    "another operation is in flight and the connection runs one at a time",
                )
            })
    }

    /// The connection is open: not `Closing`, `Closed` or `Failed`.
    fn ensure_open(&self, op: &Op<'_>) -> Result<()> {
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
        Ok(())
    }

    /// The checks that need no tag: the connection is open and the request not cancelled.
    fn gate(&self, op: &Op<'_>, cancel: Option<&CancellationToken>) -> Result<()> {
        self.ensure_open(op)?;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(op.reject(ErrorKind::Cancelled, "the request was cancelled"));
        }
        Ok(())
    }

    /// Validate every tag and split the request into chunks, before any I/O.
    fn plan(&self, op: &Op<'_>, request: &ReadRequest) -> Result<Vec<Chunk>> {
        let tags: Vec<&Tag> = request.items().iter().collect();
        addresses_of(op, &tags)?;
        // The batcher refuses more tags than `max_items_per_request` before it plans, which
        // is the re-validation against the connection's own limits.
        Batcher::plan(request.items(), &self.batch)
            .map_err(|error| op.wrap(error, FailureCategory::Validation))
    }

    /// Fail with `Timeout` if the request's own deadline had already elapsed.
    fn ensure_time_left(&self, op: &Op<'_>, deadline: Option<Deadline>) -> Result<()> {
        if deadline.is_some_and(|deadline| deadline.is_elapsed(self.clock.now())) {
            return Err(op.error(
                ErrorKind::Timeout,
                "the request deadline had already elapsed",
                FailureCategory::Transport,
            ));
        }
        Ok(())
    }

    /// The read path of [`Connection::read`], and of every polling tick.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        let span = tracing::debug_span!(
            "read",
            scheme = self.wire.scheme(),
            endpoint = %self.endpoint,
            op = "read",
            items = request.items().len(),
            chunks = tracing::field::Empty
        );
        Box::pin(self.read_inner(request, span.clone()).instrument(span))
    }

    /// Check a subscription as a read is checked, before any I/O, and plan its polling.
    fn plan_subscription(&self, request: &SubscribeRequest) -> Result<Plan> {
        let op = Op {
            connection: self,
            operation: Operation::Subscribe,
            started: self.clock.now(),
        };
        let plan = Plan::new(request, self.limits, &*self.clock)
            .map_err(|error| op.wrap(error, FailureCategory::Validation))?;
        self.gate(&op, request.cancel())?;
        self.plan(&op, plan.read()).map(drop)?;
        self.ensure_time_left(&op, request.deadline())?;
        Ok(plan)
    }

    async fn read_inner(&self, request: ReadRequest, span: tracing::Span) -> Result<ReadResponse> {
        let op = Op {
            connection: self,
            operation: Operation::Read,
            started: self.clock.now(),
        };
        self.gate(&op, request.cancel())?;
        let chunks = self.plan(&op, &request)?;
        span.record("chunks", chunks.len());
        let _slot = self.acquire(&op)?;
        self.ensure_time_left(&op, request.deadline())?;
        let mut items = Vec::with_capacity(request.items().len());
        for (index, chunk) in chunks.iter().enumerate() {
            let answer = self
                .exchange(&op, &request, index, chunks.len(), chunk)
                .await?;
            items.extend(self.items_of(&op, chunk, &answer));
        }
        self.respond(&op, &request, items, chunks.len())
    }

    /// One read of the ping address, answered by any reply that correlates with it.
    async fn ping_inner(&self) -> Result<()> {
        let op = Op {
            connection: self,
            operation: Operation::Ping,
            started: self.clock.now(),
        };
        self.ensure_open(&op)?;
        let _slot = self.acquire(&op)?;
        let address = &self.ping_address;
        let read = read_pdu(
            address.area(),
            address.file_number(),
            address.offset(),
            address.width(),
        );
        let pdu = read?;
        let unit_id = address.unit_id().unwrap_or(self.defaults.unit_id);
        let deadline = self.chunk_deadline(&op, None)?;
        // The session has proved the reply against the request, so a data reply and an
        // exception reply both show that the peer answers (`MB-QUIRK-D`).
        self.session
            .request(&pdu, unit_id, deadline, None)
            .await
            .map(|_reply| ())
            .map_err(|error| {
                let message = error.to_string();
                self.failure(&op, error, 1, message)
            })
    }

    async fn write_inner(&self, request: WriteRequest) -> Result<WriteResponse> {
        let op = Op {
            connection: self,
            operation: Operation::Write,
            started: self.clock.now(),
        };
        // The policy is the first word: a read-only connection touches neither the wire nor
        // the request.
        if self.access == AccessPolicy::ReadOnly {
            return Err(op.reject(
                ErrorKind::Unsupported(Operation::Write),
                "the connection's access policy is read-only",
            ));
        }
        self.gate(&op, request.cancel())?;
        let tags: Vec<&Tag> = request.items().iter().map(WriteItem::tag).collect();
        let addresses = addresses_of(&op, &tags)?;
        let plans: Vec<_> = request
            .items()
            .iter()
            .zip(addresses)
            .map(|(item, address)| self.prepare(&op, item, address))
            .collect();
        let _slot = self.acquire(&op)?;
        self.ensure_time_left(&op, request.deadline())?;
        let results = self.write_all(&op, &request, plans).await;
        self.respond_write(&op, &request, results)
    }

    /// The wire request of one item, or the refusal that is the item's alone: its area is
    /// read-only, or its value cannot be laid out for its type.
    fn prepare(
        &self,
        op: &Op<'_>,
        item: &WriteItem,
        address: &ModbusAddress,
    ) -> std::result::Result<Planned, ItemError> {
        let refuse = |error: PlcError| ItemError::new(op.wrap(error, FailureCategory::Validation));
        let area = address.area();
        if !area.is_writable() {
            return Err(refuse(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                format!(
                    "tag {} is a {}, which is read-only; only coils, holding registers and \
                     extended registers are written",
                    item.tag().id(),
                    area.long_name()
                ),
            )));
        }
        // A coil is a BOOL: bit 0 of the register the encoder lays out under the default order.
        let (byte_order, word_order) = if area.is_bit_area() {
            (ByteOrder::BigEndian, WordOrder::HighWordFirst)
        } else {
            let (byte_order, word_order) = resolve_order(item.tag(), address);
            (
                byte_order.unwrap_or(self.defaults.byte_order),
                word_order.unwrap_or(self.defaults.word_order),
            )
        };
        // A list goes out element after element, each laid out as it would be alone; text
        // packs its elements byte after byte into one run of registers.
        let elements = match (address.elements(), item.value()) {
            (None, value) => std::slice::from_ref(value),
            (Some(count), PlcValue::List(elements))
                if elements.len() == usize::from(count.get()) =>
            {
                elements.as_slice()
            }
            (Some(count), _) => {
                return Err(refuse(PlcError::new(
                    ErrorKind::InvalidConfig,
                    format!(
                        "tag {} selects a list of {count} elements; the value must be a list of \
                         exactly {count} (docs/parity/modbus.md#MB-ADDR-SELECTION)",
                        item.tag().id()
                    ),
                )));
            }
        };
        let registers = if address.data_type().is_text() {
            encode_text(
                elements,
                address.data_type(),
                address.string_length(),
                byte_order,
                word_order,
            )
            .map(|run| vec![run])
        } else {
            elements
                .iter()
                .map(|value| {
                    TypedDecoder::encode_registers(
                        value,
                        address.data_type(),
                        byte_order,
                        word_order,
                    )
                })
                .collect::<Result<Vec<_>>>()
        }
        .map_err(refuse)?;
        let built = if let Some(file) = address.file_number() {
            // One request carries the whole value, the elements of a list in order, cut at
            // the end of each file; one wider than a request carries is refused unsent.
            let run: Vec<u16> = registers.into_iter().flatten().collect();
            ModbusPdu::write_file_record_request(file.get(), address.offset(), &run)
        } else {
            let payload = if area.is_bit_area() {
                let bits: Vec<bool> = registers
                    .iter()
                    .map(|element| element.first().is_some_and(|word| word & 1 == 1))
                    .collect();
                pack_bits_lsb_first(&bits)
            } else {
                registers
                    .iter()
                    .flatten()
                    .copied()
                    .flat_map(u16::to_be_bytes)
                    .collect()
            };
            ModbusPdu::write_request(area, address.offset(), address.width(), &payload)
        };
        built
            .map(|pdu| {
                // The opt-in `single-coil-write=fc05`: one coil goes out as function 05.
                let pdu = if self.defaults.single_coil_fc05
                    && let ModbusPdu::WriteMultipleCoils {
                        start,
                        quantity: 1,
                        bytes,
                    } = &pdu
                {
                    ModbusPdu::WriteSingleCoil {
                        address: *start,
                        value: bytes.first().is_some_and(|byte| byte & 1 == 1),
                    }
                } else {
                    pdu
                };
                Planned {
                    unit_id: address.unit_id().unwrap_or(self.defaults.unit_id),
                    pdu,
                }
            })
            .map_err(PlcError::from)
            .map_err(refuse)
    }

    /// Send the planned writes one after another and say how each ended.
    ///
    /// Every item gets a result. A write whose request never left (the link was down, the
    /// deadline had passed, the token had fired) is that item's `Rejected`, with the error
    /// that stopped it. After an `Unknown` the rest of the planned writes are not sent: the
    /// link and the device are in a state nobody knows, and a later write may depend on the
    /// one that is unknown.
    async fn write_all(
        &self,
        op: &Op<'_>,
        request: &WriteRequest,
        plans: Vec<std::result::Result<Planned, ItemError>>,
    ) -> Vec<WriteResult> {
        let mut results = Vec::with_capacity(plans.len());
        let mut halted = false;
        for (item, plan) in request.items().iter().zip(plans) {
            let id = item.tag().id();
            let outcome = match plan {
                Err(refusal) => WriteOutcome::Rejected(refusal),
                Ok(_) if halted => WriteOutcome::Rejected(ItemError::new(op.reject(
                    ErrorKind::Cancelled,
                    format!("write of {id} was not sent: an earlier write has an unknown outcome"),
                ))),
                Ok(plan) => match self.write_one(op, request, &plan).await {
                    Written::Answered(outcome) => {
                        halted = matches!(outcome, WriteOutcome::Unknown(_));
                        outcome
                    }
                    Written::Unsent(error) => {
                        let message = format!("write of {id}: {error}");
                        WriteOutcome::Rejected(ItemError::new(self.failure(op, error, 1, message)))
                    }
                },
            };
            results.push(WriteResult::new(id.clone(), outcome));
        }
        results
    }

    /// One write request, sent once: the outcome the device's answer gives, `Unknown` if
    /// the frame went out and no usable answer came back, or the error of a request that
    /// never went out.
    ///
    /// The frame counts as sent once the guard has handed it whole to the transport (its
    /// `requests` counter rose): a server cannot act on half an MBAP frame, so a send that
    /// failed before that point changed nothing. The counter is a lower bound: a send that
    /// reports a failure (a deadline, a lost link) after the whole frame was written is
    /// `Rejected`, not `Unknown`. For a frame of at most 20 bytes the window is small, and
    /// it is recorded as `MB-WRITE-UNKNOWN`.
    async fn write_one(&self, op: &Op<'_>, request: &WriteRequest, plan: &Planned) -> Written {
        let deadline = match self.chunk_deadline(op, request.deadline()) {
            Ok(deadline) => deadline,
            Err(error) => return Written::Unsent(error),
        };
        let sent_before = self.session.stats().guard.requests;
        let outcome = self
            .session
            .request(&plan.pdu, plan.unit_id, deadline, request.cancel())
            .await;
        match outcome {
            Ok(reply) => {
                let answer = Answer {
                    // The session proved the echo against the request.
                    misfit: None,
                    reply,
                    arrived: self.clock.now(),
                    retries: 0,
                };
                Written::Answered(self.outcome_of(op, &answer))
            }
            Err(error) if self.session.stats().guard.requests > sent_before => {
                tracing::debug!(kind = ?error.kind(), "a write was sent and not confirmed");
                Written::Answered(WriteOutcome::Unknown(unknown_reason(error.kind())))
            }
            Err(error) => Written::Unsent(error),
        }
    }

    /// What the device's answer to a write says: the echo confirms it (the session has
    /// proved it repeats the request), an exception rejects it.
    fn outcome_of(&self, op: &Op<'_>, answer: &Answer) -> WriteOutcome {
        match &answer.reply {
            ModbusPdu::Exception { function, code } => {
                let refusal = exception_error(Operation::Write, *function, *code);
                WriteOutcome::Rejected(self.stamp_item(op, refusal, answer))
            }
            ModbusPdu::WriteSingleRegisterResponse { .. }
            | ModbusPdu::WriteSingleCoilResponse { .. }
            | ModbusPdu::WriteMultipleResponse { .. }
            | ModbusPdu::WriteFileRecordResponse { .. } => WriteOutcome::Confirmed,
            // The session proves every reply against its request, so a read reply to a
            // write cannot come out of it; it is not a confirmation if it ever does.
            ModbusPdu::ReadBitsResponse { .. }
            | ModbusPdu::ReadRegistersResponse { .. }
            | ModbusPdu::ReadCoils { .. }
            | ModbusPdu::ReadDiscreteInputs { .. }
            | ModbusPdu::ReadHoldingRegisters { .. }
            | ModbusPdu::ReadInputRegisters { .. }
            | ModbusPdu::WriteMultipleCoils { .. }
            | ModbusPdu::WriteSingleCoil { .. }
            | ModbusPdu::WriteSingleRegister { .. }
            | ModbusPdu::WriteMultipleRegisters { .. }
            | ModbusPdu::ReadFileRecordResponse { .. }
            | ModbusPdu::ReadFileRecord { .. }
            | ModbusPdu::WriteFileRecord { .. } => WriteOutcome::Unknown(UNCONFIRMED),
        }
    }

    /// The correlated response for `results`, one per item of `request`.
    fn respond_write(
        &self,
        op: &Op<'_>,
        request: &WriteRequest,
        results: Vec<WriteResult>,
    ) -> Result<WriteResponse> {
        let finished = self.clock.now();
        let built = results
            .into_iter()
            .fold(
                WriteResponse::builder(request, op.started, finished),
                WriteResponseBuilder::result,
            )
            .build();
        let response = built.map_err(|error| op.wrap(error, FailureCategory::Internal))?;
        tracing::debug!(
            items = response.results().len(),
            confirmed = response.confirmed_count(),
            rejected = response.rejected_count(),
            unknown = response.unknown_count(),
            "write complete"
        );
        Ok(response)
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
                crate::ReadResponseBuilder::item,
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
        let pdu = read_pdu(chunk.area, chunk.file, chunk.start, chunk.quantity)?;
        let unit_id = chunk.unit_id.unwrap_or(self.defaults.unit_id);
        let mut attempts: u32 = 0;
        loop {
            attempts = attempts.saturating_add(1);
            let deadline = self.chunk_deadline(op, request.deadline())?;
            let outcome = self
                .session
                .request_decoded(&pdu, unit_id, deadline, request.cancel())
                .await;
            match outcome {
                Ok(reply) => {
                    return Ok(Answer {
                        misfit: reply.check_reply_to(&pdu).err(),
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
        let message = format!("chunk {} of {total}: {error}", index.saturating_add(1));
        self.failure(op, error, attempts, message)
    }

    /// `error` as the operation's own, saying `message`, with its diagnostics.
    fn failure(&self, op: &Op<'_>, error: PlcError, attempts: u32, message: String) -> PlcError {
        let mut diagnostics = error.diagnostics().clone();
        diagnostics.operation = Some(op.operation);
        // The state now, not the guard's at the instant of the failure: a peer that closed
        // leaves the guard `Reconnecting` by the time the operation reports it.
        diagnostics.connection_state = Some(self.session.state());
        diagnostics.latency = Some(op.elapsed());
        diagnostics.retries = attempts.saturating_sub(1);
        diagnostics
            .category
            .get_or_insert(FailureCategory::Transport);
        PlcError::new(error.kind(), message)
            .with_diagnostics(diagnostics)
            .with_source(error)
    }

    /// One item per member of `chunk`, decoded from the chunk's answer.
    fn items_of(&self, op: &Op<'_>, chunk: &Chunk, answer: &Answer) -> Vec<ReadItem> {
        chunk
            .members
            .iter()
            .map(|member| {
                let outcome = answer
                    .misfit
                    .as_ref()
                    .map_or_else(
                        || decode_member(&answer.reply, chunk.area, member, self.defaults),
                        |error| Err(misfit_item(error, &answer.reply)),
                    )
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
        diagnostics.operation = Some(op.operation);
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
///
/// `misfit` is why the reply does not answer the request although it decodes (its data is not
/// the length the request asked for): every member of the chunk fails with it, and no member
/// is decoded from data that cannot be trusted to be the data asked for.
struct Answer {
    misfit: Option<ParseError>,
    reply: ModbusPdu,
    arrived: Instant,
    retries: u32,
}

/// The Modbus address of every tag, in order, or the first tag that has none or asks
/// for more than one element.
fn addresses_of<'t>(op: &Op<'_>, tags: &[&'t Tag]) -> Result<Vec<&'t ModbusAddress>> {
    let addresses = tags
        .iter()
        .map(|tag| {
            let address: &dyn Any = tag.address();
            address.downcast_ref::<ModbusAddress>().ok_or_else(|| {
                op.reject(
                    ErrorKind::InvalidAddress,
                    format!("tag {} does not carry a Modbus address", tag.id()),
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    for tag in tags {
        if tag.count() != NonZeroU32::MIN {
            return Err(op.reject(
                ErrorKind::InvalidConfig,
                format!(
                    "tag {} has element count {}: Modbus reads several elements through a \
                     range selection in the address, for example holding-register:1[0..3]:INT",
                    tag.id(),
                    tag.count()
                ),
            ));
        }
    }
    Ok(addresses)
}

/// The reason of an `Unknown` whose reply did not confirm the write (it does not decode, or
/// does not echo the request): the confirmation did not arrive.
const UNCONFIRMED: UnknownReason = UnknownReason::TimeoutAfterTransmit;

/// One item of a write request, turned into the request that carries it.
#[derive(Debug)]
struct Planned {
    unit_id: UnitId,
    pdu: ModbusPdu,
}

/// How one write request ended.
enum Written {
    /// The frame went out; this is what is known of the write.
    Answered(WriteOutcome),
    /// The frame never went out, for this reason.
    Unsent(PlcError),
}

/// Why a write that went out is not confirmed, from the failure that ended the wait: the
/// deadline, the loss of the link or the caller's cancellation. A reply that does not
/// answer the write (it does not decode, or echoes something else) is a confirmation that
/// did not arrive, and counts as the first.
const fn unknown_reason(kind: ErrorKind) -> UnknownReason {
    match kind {
        ErrorKind::Disconnected => UnknownReason::DisconnectedAfterTransmit,
        ErrorKind::Cancelled => UnknownReason::CancelledAfterTransmit,
        ErrorKind::Timeout
        | ErrorKind::InvalidConfig
        | ErrorKind::InvalidAddress
        | ErrorKind::Unsupported(_)
        | ErrorKind::Authentication
        | ErrorKind::Trust
        | ErrorKind::MalformedData
        | ErrorKind::DeviceException
        | ErrorKind::ResourceLimit
        | ErrorKind::Backpressure
        | ErrorKind::Internal => UNCONFIRMED,
    }
}

/// The read request PDU: function code by area, wire start and quantity. An extended
/// register is read by Read File Record at `file` and the record `start`, cut at the end
/// of each file into one sub-request per file.
fn read_pdu(
    area: ModbusArea,
    file: Option<NonZeroU16>,
    start: u16,
    quantity: u16,
) -> Result<ModbusPdu> {
    Ok(match area {
        ModbusArea::Coil => ModbusPdu::ReadCoils { start, quantity },
        ModbusArea::DiscreteInput => ModbusPdu::ReadDiscreteInputs { start, quantity },
        ModbusArea::HoldingRegister => ModbusPdu::ReadHoldingRegisters { start, quantity },
        ModbusArea::InputRegister => ModbusPdu::ReadInputRegisters { start, quantity },
        ModbusArea::ExtendedRegister => {
            let Some(file) = file else {
                return Err(PlcError::new(
                    ErrorKind::Internal,
                    "an extended register without a file number",
                ));
            };
            ModbusPdu::read_file_record_request(file.get(), start, quantity)
                .map_err(PlcError::from)?
        }
    })
}

/// The error of a member whose chunk's reply decodes but does not fit the request (a byte
/// count other than the request's quantity asks for): `MalformedData` whose status keeps the
/// reply PDU as received, function code and byte count included, when it fits `RAW_BUDGET`.
///
/// The bytes are the decoded PDU written again with `encode_response`, not a copy of the
/// frame. They equal the received bytes because `decode_response` is strictly canonical (it
/// refuses a byte count that disagrees with the data, trailing bytes, a zero or odd count and
/// a wrong reference type), so encoding what it accepts gives back what it was given; a
/// more lenient decode would have to hand the received bytes up instead.
fn misfit_item(error: &ParseError, reply: &ModbusPdu) -> ItemError {
    let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
    let received = encode_response(reply, &mut writer)
        .ok()
        .map(|()| writer.into_bytes());
    with_raw_reply(
        PlcError::new(
            ErrorKind::MalformedData,
            format!("the reply PDU does not answer the request: {error}"),
        )
        .with_source(error.clone()),
        received.as_deref(),
        RAW_BUDGET,
    )
}

/// The value of one member in a chunk's reply, or the error that is the member's alone.
///
/// An exception reply fails every member the same way ([`exception_item_error`]). A data
/// reply is cut at the member's offset and decoded with the member's type and the order it
/// resolved to, `defaults` standing in for what the tag and address left unset; the reply
/// of a file record read is the registers of its sub-requests joined in order. A text
/// member, a list included, is decoded in one piece from its packed bytes. A reply of
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
    // A file record reply is the registers of its sub-responses, joined in order, so an
    // element may take its registers from two files.
    let joined: Vec<u16> = if let ModbusPdu::ReadFileRecordResponse { records } = reply {
        records.iter().flatten().copied().collect()
    } else {
        Vec::new()
    };
    // The registers of a register reply.
    let words = || match reply {
        ModbusPdu::Exception { function, code } => Err(exception_item_error(*function, *code)),
        ModbusPdu::ReadRegistersResponse { registers, .. }
            if !area.is_bit_area() && area != ModbusArea::ExtendedRegister =>
        {
            Ok(registers.as_slice())
        }
        ModbusPdu::ReadFileRecordResponse { .. } if area == ModbusArea::ExtendedRegister => {
            Ok(joined.as_slice())
        }
        ModbusPdu::ReadBitsResponse { .. }
        | ModbusPdu::ReadRegistersResponse { .. }
        | ModbusPdu::ReadFileRecordResponse { .. }
        | ModbusPdu::ReadFileRecord { .. }
        | ModbusPdu::WriteFileRecord { .. }
        | ModbusPdu::WriteFileRecordResponse { .. }
        | ModbusPdu::ReadCoils { .. }
        | ModbusPdu::ReadDiscreteInputs { .. }
        | ModbusPdu::ReadHoldingRegisters { .. }
        | ModbusPdu::ReadInputRegisters { .. }
        | ModbusPdu::WriteMultipleCoils { .. }
        | ModbusPdu::WriteSingleCoil { .. }
        | ModbusPdu::WriteSingleRegister { .. }
        | ModbusPdu::WriteMultipleRegisters { .. }
        | ModbusPdu::WriteSingleCoilResponse { .. }
        | ModbusPdu::WriteSingleRegisterResponse { .. }
        | ModbusPdu::WriteMultipleResponse { .. } => Err(mismatch()),
    };
    // The value of the element at `offset` registers or bits into the reply.
    let element = |offset: usize| {
        if let (ModbusPdu::ReadBitsResponse { bytes, .. }, true) = (reply, area.is_bit_area()) {
            TypedDecoder::decode_bits(bytes, offset, member.data_type, RAW_BUDGET)
        } else {
            words().and_then(|registers| decode_registers_at(registers, offset, member, defaults))
        }
    };
    let first = usize::from(member.offset);
    if member.data_type.is_text() {
        return words().and_then(|registers| {
            decode_text(
                registers.get(first..).unwrap_or_default(),
                member.data_type,
                member.string_length,
                member.elements,
                member.byte_order.unwrap_or(defaults.byte_order),
                member.word_order.unwrap_or(defaults.word_order),
                RAW_BUDGET,
            )
        });
    }
    match member.elements {
        None => element(first),
        // A list is its elements one width apart, each decoded as it would be alone; the
        // first element that fails fails the item.
        Some(count) => {
            let width = usize::from(element_width(area, member.data_type));
            (0..usize::from(count.get()))
                .map(|index| element(first.saturating_add(index.saturating_mul(width))))
                .collect::<std::result::Result<Vec<_>, _>>()
                .map(PlcValue::List)
        }
    }
}

/// The value of the element of `member` that starts `offset` registers into the `registers`
/// of a chunk's reply. A reply too short for the element decodes as the wrong register
/// count, which keeps the bytes that did arrive.
fn decode_registers_at(
    registers: &[u16],
    offset: usize,
    member: &Member,
    defaults: ConnectionDefaults,
) -> std::result::Result<PlcValue, ItemError> {
    let end = offset.saturating_add(usize::from(member.data_type.register_size()));
    let words = registers.get(offset..end).unwrap_or(&[]);
    TypedDecoder::decode_registers(
        words,
        member.data_type,
        member.byte_order.unwrap_or(defaults.byte_order),
        member.word_order.unwrap_or(defaults.word_order),
        RAW_BUDGET,
    )
}

/// An `InvalidConfig` error of opening the connection.
fn invalid_config(message: String) -> PlcError {
    let diagnostics = Diagnostics {
        operation: Some(Operation::Connect),
        category: Some(FailureCategory::Validation),
        ..Diagnostics::default()
    };
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

/// The context every error of one read, write or ping is stamped with.
struct Op<'a> {
    connection: &'a Link,
    /// What the diagnostics name: `Read`, `Write`, `Ping` or `Subscribe`.
    operation: Operation,
    /// The clock reading when the operation began, the base of every latency figure.
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
        Diagnostics {
            operation: Some(self.operation),
            connection_state: Some(self.connection.session.state()),
            latency: Some(self.elapsed()),
            category: Some(category),
            ..Diagnostics::default()
        }
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

    /// Re-wrap an error from core or the batcher with this operation's diagnostics.
    fn wrap(&self, error: PlcError, category: FailureCategory) -> PlcError {
        PlcError::new(error.kind(), error.to_string())
            .with_diagnostics(self.diagnostics(category))
            .with_source(error)
    }
}

impl Connection for ModbusConnection {
    fn state(&self) -> ConnectionState {
        self.link.session.state()
    }

    fn watch_state(&self) -> StateWatch {
        self.link.session.watch_state()
    }

    fn capabilities(&self) -> Capabilities {
        self.link.capabilities
    }

    fn limits(&self) -> ResourceLimits {
        self.link.limits
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
    ///   `max_items_per_request`, or one tag (a range selection, a list of
    ///   [`ModbusAddress::elements`]) is wider than `max-registers-per-request` or
    ///   `max-coils-per-request`.
    /// - [`ErrorKind::Backpressure`] if another read or a ping is in flight; nothing was
    ///   sent.
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
    /// Transmits one read request frame (function code 01, 02, 03 or 04, or 20 for the
    /// extended registers) per chunk, and again per retry. Nothing is ever written to the
    /// device.
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        self.link.read(request)
    }

    /// Write every item of `request`, one wire request each, one after another.
    ///
    /// A connection whose access policy is read-only refuses the call first, before the
    /// connection state, the request or the wire is looked at. Otherwise the connection
    /// must be open and the token not cancelled, every tag must carry a [`ModbusAddress`]
    /// with an element count of one, and only then is one item laid out as a wire request:
    /// the value is encoded with the data type of its address and the byte and word order
    /// of the tag, else of the address, else of [`ConnectionDefaults`]; a coil takes a
    /// `BOOL`; the request is function 15 for a coil (even one), function 06 for a holding
    /// register payload of exactly 2 bytes and function 16 for any other
    /// ([`ModbusPdu::write_request`]), and function 21 for an extended register, cut at the
    /// end of each file into one sub-request per file
    /// ([`ModbusPdu::write_file_record_request`]). A range selection
    /// ([`ModbusAddress::elements`]) takes a [`PlcValue::List`] of exactly its element count,
    /// each element laid out as it would be alone, in one request, on the extended registers
    /// too. An item that cannot be laid out (a discrete input or an input register, a value
    /// or a list that does not fit its address, which is `InvalidConfig`, or a list wider
    /// than one request carries, which is `ResourceLimit`: 124 registers or 1969 coils, and
    /// on the extended registers more than 118 registers across the end of a file) is
    /// `Rejected` without a request and does not stop the others. Then one operation runs at a time, the request deadline is checked, and
    /// the requests go out in order, each under `min(request deadline, now +
    /// timeouts.request)` taken when it starts.
    ///
    /// Each answer decides its item and nothing else:
    ///
    /// - an echo that repeats the request is `Confirmed`;
    /// - a device exception is `Rejected`, with the exception as the item error and its
    ///   code as the protocol status (code 1 is `Unsupported(Operation::Write)`, code 2
    ///   `InvalidAddress`, every other `DeviceException`);
    /// - a request that went out and got no usable answer (the deadline passed, the link
    ///   was lost, the token fired, or the reply did not answer the request) is
    ///   `Unknown`, with the matching [`UnknownReason`]. Such a write is never retried and
    ///   never replayed, whatever the retry policy says, and the writes after it in the
    ///   request are not sent: they are `Rejected` with [`ErrorKind::Cancelled`] and can be
    ///   resubmitted once the caller has read the value back.
    ///
    /// A request that never went out (the link could not be reopened, the deadline had
    /// passed, the token had fired) is that item's `Rejected`, with the error as the item
    /// error: once the call has begun, every item has an outcome and no `Confirmed` or
    /// `Unknown` result is lost to an error of the call.
    ///
    /// The deadline source is the request's deadline, or `timeouts.request` of the
    /// configuration, per wire request.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-WRITE
    /// The counterpart of upstream `ModbusTcpConnection.onWrite`: one PDU per tag, chained
    /// sequentially, coils through function 15, a 2-byte holding-register payload through
    /// function 06 and any other through function 16, discrete inputs and input registers
    /// refused (`MB-TCP-WRITE`), extended registers through function 21 (Conn:467-487). A
    /// list is written element by element as upstream does, except that an element narrower
    /// than a register keeps a register of its own
    /// (docs/parity/modbus.md#MB-ADDR-SELECTION). Stonecrab refuses a write while the access policy is
    /// read-only (`MB-WRITE-ACCESS`), reports a write with no usable answer as `Unknown`
    /// where upstream reports an error code and goes on (`MB-WRITE-UNKNOWN`), and requires
    /// the reply to echo the function code of the request (`MB-WRITE-ECHO`).
    ///
    /// # Errors
    /// Every error carries `Diagnostics` with `operation = Write`, the connection state, the
    /// latency and a category.
    /// - [`ErrorKind::Unsupported`]`(`[`Operation::Write`]`)` if the access policy is
    ///   read-only; nothing else was looked at and nothing was sent.
    /// - [`ErrorKind::Disconnected`] if the connection is `Closing`, `Closed` or `Failed`
    ///   (nothing sent).
    /// - [`ErrorKind::Cancelled`] if the request's token had fired before the call.
    /// - [`ErrorKind::InvalidAddress`] if a tag's address is not a Modbus address.
    /// - [`ErrorKind::InvalidConfig`] if a tag's element count is not 1.
    /// - [`ErrorKind::Backpressure`] if another read, write or ping is in flight; nothing
    ///   was sent.
    /// - [`ErrorKind::Timeout`] if the request deadline had already passed.
    ///
    /// A value that does not fit its address, a discrete input or an input register, a
    /// request that could not go out, and everything the device answers are not errors of
    /// the call: they are the item's outcome.
    ///
    /// # Cancel safety
    /// Cancel-safe in the connection's own state: a dropped future releases the single
    /// in-flight slot. It is not safe for the write: if a request frame had gone out, the
    /// device may apply it and the caller never learns the outcome, because the future that
    /// would have reported `Unknown` is gone. Cancel through the request's token instead,
    /// which resolves the call with `Unknown(CancelledAfterTransmit)` for a write that went
    /// out. The session discards a late reply and the guard resynchronizes before the next
    /// request.
    ///
    /// # Wire effects
    /// Transmits one request frame (function code 06, 15 or 16) per item that can be laid
    /// out, in request order, and stops after the first `Unknown`. A frame is never sent
    /// twice. Writes the device.
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        let span = tracing::debug_span!(
            "write",
            scheme = self.link.wire.scheme(),
            endpoint = %self.link.endpoint,
            op = "write",
            items = request.items().len()
        );
        Box::pin(self.link.write_inner(request).instrument(span))
    }

    /// Close the connection: `Closing`, then `Closed`.
    ///
    /// A read in flight fails with [`ErrorKind::Disconnected`], and so does a read that is
    /// waiting out a reconnect backoff. A background reconnect in progress stops, and its
    /// task is joined. The call is idempotent. The deadline source is
    /// `timeouts.request`, applied by the guard to the shutdown and the join.
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
            scheme = self.link.wire.scheme(),
            endpoint = %self.link.endpoint,
            op = "close"
        );
        Box::pin(self.link.session.close().instrument(span))
    }

    /// Check that the peer answers: one read of `ping-address` through the session.
    ///
    /// The read goes through the same admission rules and the same single in-flight slot
    /// as [`read`](Self::read), so a ping while a read (or another ping) runs fails with
    /// [`ErrorKind::Backpressure`] before any I/O. It is never retried. The address is the
    /// `ping-address` of the connection string (`4x00001:BOOL` unless it says otherwise),
    /// read in the unit it names, else in the default unit, as one coil, one discrete input
    /// or as many registers as its data type spans. The value of the reply is not looked
    /// at.
    ///
    /// The deadline source is `timeouts.request`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-PING
    /// The counterpart of upstream `onPing`, including its quirk (d): any reply that
    /// answers the request is success, an exception reply too, because a correlated reply
    /// shows that the peer answers (docs/parity/modbus.md#MB-QUIRK-D).
    ///
    /// # Errors
    /// Every error carries `Diagnostics` with `operation = Ping`, the connection state, the
    /// latency and a category.
    /// - [`ErrorKind::Disconnected`] if the connection is `Closing`, `Closed` or `Failed`
    ///   (nothing sent), or the peer closed before it answered.
    /// - [`ErrorKind::Backpressure`] if a read or another ping is in flight; nothing was
    ///   sent.
    /// - [`ErrorKind::Timeout`] if no reply arrived within `timeouts.request`.
    /// - [`ErrorKind::MalformedData`] if a reply violated the framing or did not answer
    ///   the request.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped future releases the single in-flight slot. If the request
    /// frame had gone out, its reply may still arrive; the session discards it as late and
    /// the guard resynchronizes before the next request. A ping reads and so changes
    /// nothing on the device.
    ///
    /// # Wire effects
    /// Transmits one read request frame (function code 01, 02, 03 or 04) for the ping
    /// address. Nothing is ever written to the device.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{Connection, ConnectionConfig, Endpoint};
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusConnection};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    ///
    /// // The default ping address, holding register 1, is wire address 0 of unit 1.
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::strict(vec![
    ///     Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0, 0, 0, 1]),
    ///     Step::Reply(vec![0, 1, 0, 0, 0, 5, 1, 3, 2, 0, 0]),
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
    /// assert!(connection.capabilities().ping);
    /// block_on_with(&clock, connection.ping())?;
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    fn ping(&self) -> OpFuture<'_, ()> {
        let span = tracing::debug_span!(
            "ping",
            scheme = self.link.wire.scheme(),
            endpoint = %self.link.endpoint,
            op = "ping"
        );
        Box::pin(self.link.ping_inner().instrument(span))
    }

    /// Subscribe by polling: every tick reads the request's tags through
    /// [`read`](Self::read).
    ///
    /// The request is checked before any I/O and in this order: an
    /// [`SubscriptionKind::Event`](crate::SubscriptionKind::Event) request is refused (a
    /// Modbus device raises no events), so is a change-of-state request on a clock whose
    /// sleeps resolve at once (its polling would never pause), the request's tags fit the
    /// connection's resource limits, the connection is open, the token is not cancelled,
    /// every tag carries a [`ModbusAddress`] with an element count of one, the tags fit
    /// `max_items_per_request` and the chunk limits, and the deadline has not elapsed. The
    /// deadline and the token bound this call only; the subscription lives until it is
    /// unsubscribed or dropped, or the connection closes or fails.
    ///
    /// The [`Subscription`] reports [`SubscriptionMode::Polling`] and its interval: the
    /// cyclic interval of the request, or 1000 ms for a change-of-state request. The first
    /// tick is due at once and the next ones on a grid of the interval; the polling runs
    /// while [`Subscription::recv`] waits (see its `# Polling` section). A cyclic
    /// subscription delivers every reading, a change-of-state one a tag's first outcome and
    /// then each change. A tick that comes due while a read runs, or while the consumer is
    /// not waiting, is skipped and counted in
    /// [`LossCounters::skipped`](crate::LossCounters::skipped); so is a tick whose read
    /// finds the single in-flight slot taken by another operation, or the link down while
    /// it is re-established. Any other failure of a polling read reaches the consumer as an
    /// item error for every tag, and an exception reply as the per-item errors a read
    /// returns. Closing the connection, or its failure, ends the subscription: `recv`
    /// delivers what is queued, then `None`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-SUBSCRIBE
    /// The counterpart of upstream's polling subscriptions (`PollingSubscriptionConnectionBase`,
    /// which `ModbusTcpConnection` extends): cyclic at the request's interval,
    /// change-of-state at the 1000 ms default, event refused. The first tick is due at once,
    /// as upstream's timer has no initial delay.
    ///
    /// # Errors
    /// Every error carries `Diagnostics` with `operation = Subscribe`, the connection state,
    /// the latency and the `Validation` category, except `Disconnected` and `Timeout`, which
    /// are `Transport`.
    /// - [`ErrorKind::Unsupported`] for [`Operation::Subscribe`] for an event subscription,
    ///   or for a change-of-state subscription on a clock whose sleeps resolve at once.
    /// - [`ErrorKind::Disconnected`] if the connection is `Closing`, `Closed` or `Failed`.
    /// - [`ErrorKind::Cancelled`] if the request's token was cancelled.
    /// - [`ErrorKind::InvalidAddress`] if a tag's address is not a Modbus address.
    /// - [`ErrorKind::InvalidConfig`] if a tag's element count is not 1.
    /// - [`ErrorKind::ResourceLimit`] if the request has more items than
    ///   `max_items_per_request`.
    /// - [`ErrorKind::Timeout`] if the request's deadline had already elapsed.
    ///
    /// # Cancel safety
    /// Cancel-safe: the call performs no I/O, so a dropped future leaves nothing behind.
    /// The deadline source is the request's deadline; with none, nothing in the call waits.
    ///
    /// # Wire effects
    /// The call sends nothing. The subscription then sends one read per tick while its
    /// consumer waits, as [`read`](Self::read) does: one read request frame (function code
    /// 01, 02, 03 or 04) per chunk, and again per retry. Nothing is ever written.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use stonecrab::{
    ///     AddressOptions, Connection, ConnectionConfig, Endpoint, PlcValue, QueuePolicy,
    ///     SubscribeRequest, SubscriptionKind, SubscriptionMode, Tag, TagId,
    /// };
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    ///
    /// // The first tick reads holding register 108 at once: 555.
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
    /// let address = ModbusAddress::parse("holding-register:108", &AddressOptions::default())?;
    /// let tag = Tag::new(TagId::new("level")?, Arc::new(address));
    /// let kind = SubscriptionKind::Cyclic(Duration::from_secs(1));
    /// let request = SubscribeRequest::builder(connection.limits(), kind, QueuePolicy::DropOldest)
    ///     .tag(tag)
    ///     .build()?;
    /// let mut subscription = block_on_with(&clock, connection.subscribe(request))?;
    /// assert_eq!(subscription.mode(), SubscriptionMode::Polling);
    /// assert_eq!(subscription.interval(), Some(Duration::from_secs(1)));
    /// let event = block_on_with(&clock, subscription.recv())?.expect("the first tick");
    /// assert_eq!(event.outcome().as_ref().ok(), Some(&PlcValue::I16(555)));
    /// assert_eq!(subscription.unsubscribe().skipped, 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    fn subscribe(&self, request: SubscribeRequest) -> OpFuture<'_, Subscription> {
        let span = tracing::debug_span!(
            "subscribe",
            scheme = self.link.wire.scheme(),
            endpoint = %self.link.endpoint,
            op = "subscribe",
            items = request.items().len()
        );
        Box::pin(
            async move {
                let plan = self.link.plan_subscription(&request)?;
                tracing::debug!("polling subscription started");
                Ok(plan.start(
                    Arc::clone(&self.link.clock),
                    self.link.session.watch_state(),
                    &self.link.lifeline,
                    Arc::downgrade(&self.link),
                    Link::read,
                ))
            }
            .instrument(span),
        )
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests {
    use crate::testkit::{FakeClock, FakeTransportFactory, block_on_with};
    use crate::{AddressOptions, Endpoint, TagId};

    use super::*;
    use crate::modbus::ModbusDataType;

    fn member(data_type: ModbusDataType, offset: u16) -> Member {
        Member {
            tag: TagId::new("m").expect("tag id"),
            offset,
            data_type,
            byte_order: None,
            word_order: None,
            elements: None,
            string_length: None,
        }
    }

    // covers: ALPHA_PLAN S25 / a reply of the wrong shape is MalformedData, not a panic
    #[test]
    fn a_reply_of_the_wrong_shape_for_the_area_fails_the_member() {
        let defaults = ConnectionDefaults::default();
        let registers = ModbusPdu::ReadRegistersResponse {
            function: crate::modbus::FunctionCode::ReadHoldingRegisters,
            registers: vec![1],
        };
        let bits = ModbusPdu::ReadBitsResponse {
            function: crate::modbus::FunctionCode::ReadCoils,
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
            function: crate::modbus::FunctionCode::ReadCoils,
            code: crate::modbus::ExceptionCode::IllegalDataAddress,
        };
        let error = decode_member(&refused, ModbusArea::Coil, &bool_member, defaults)
            .expect_err("an exception fails the member");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "exception 02");
    }

    // covers: MB-ADDR-EXTENDED / the reply of a file record read is the registers of its
    // sub-responses in order, and only an extended register takes it
    #[test]
    fn a_file_record_reply_is_joined_in_order_for_an_extended_member() {
        let defaults = ConnectionDefaults::default();
        let file_reply = ModbusPdu::ReadFileRecordResponse {
            records: vec![vec![0x0001], vec![0x0002, 0x0003]],
        };
        let registers = ModbusPdu::ReadRegistersResponse {
            function: crate::modbus::FunctionCode::ReadHoldingRegisters,
            registers: vec![1],
        };
        let dint = member(ModbusDataType::DInt, 0);
        let across = member(ModbusDataType::DInt, 1);
        // Registers 1 and 2 are the last of one sub-response and the first of the next.
        assert_eq!(
            decode_member(&file_reply, ModbusArea::ExtendedRegister, &dint, defaults)
                .expect("two registers"),
            PlcValue::I32(0x0001_0002)
        );
        assert_eq!(
            decode_member(&file_reply, ModbusArea::ExtendedRegister, &across, defaults)
                .expect("two registers"),
            PlcValue::I32(0x0002_0003)
        );
        for (reply, area) in [
            (&file_reply, ModbusArea::HoldingRegister),
            (&registers, ModbusArea::ExtendedRegister),
        ] {
            let error = decode_member(reply, area, &dint, defaults).expect_err("wrong shape");
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{area:?}");
        }
    }

    // covers: MB-TCP-READ-COIL, MB-TCP-READ-DI, MB-TCP-READ-HR, MB-TCP-READ-IR, MB-ADDR-EXTENDED
    // / the read request of each area, and of an extended register that has no file, which no
    // address can make
    #[test]
    fn the_read_request_follows_the_area() {
        let file = NonZeroU16::new(2);
        let (start, quantity) = (7, 2);
        for (area, expected) in [
            (ModbusArea::Coil, ModbusPdu::ReadCoils { start, quantity }),
            (
                ModbusArea::DiscreteInput,
                ModbusPdu::ReadDiscreteInputs { start, quantity },
            ),
            (
                ModbusArea::HoldingRegister,
                ModbusPdu::ReadHoldingRegisters { start, quantity },
            ),
            (
                ModbusArea::InputRegister,
                ModbusPdu::ReadInputRegisters { start, quantity },
            ),
        ] {
            assert_eq!(
                read_pdu(area, None, start, quantity).expect("a request"),
                expected,
                "{area:?}"
            );
        }
        assert_eq!(
            read_pdu(ModbusArea::ExtendedRegister, file, 9_999, 2).expect("a request"),
            ModbusPdu::read_file_record_request(2, 9_999, 2).expect("the same request")
        );
        let error = read_pdu(ModbusArea::ExtendedRegister, None, 0, 1).expect_err("no file");
        assert_eq!(error.kind(), ErrorKind::Internal);
        let error = read_pdu(ModbusArea::ExtendedRegister, file, 0, 121).expect_err("too long");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    // covers: ALPHA_PLAN S25 / a reply too short for a member keeps the bytes that arrived
    #[test]
    fn a_reply_too_short_for_the_member_is_a_wrong_register_count() {
        let reply = ModbusPdu::ReadRegistersResponse {
            function: crate::modbus::FunctionCode::ReadHoldingRegisters,
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

    // covers: MB-ADDR-EXTENDED / a chunk the batcher could not have planned is refused before
    // anything is sent
    #[test]
    fn an_extended_chunk_that_cannot_be_encoded_is_refused_without_a_request() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let config =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .build()
                .expect("config");
        let connection = Link::build(
            Wire::Tcp,
            &config,
            Arc::new(factory.clone()),
            Arc::new(clock.clone()),
            ConnectionDefaults::default(),
            ModbusAddress::DEFAULT_PING,
        )
        .expect("connection");
        let address =
            ModbusAddress::parse("extended-register:1", &AddressOptions::default()).expect("ok");
        let tag = Tag::new(TagId::new("a").expect("tag id"), Arc::new(address));
        let request = ReadRequest::builder(connection.limits)
            .tag(tag)
            .build()
            .expect("request");
        let op = Op {
            connection: &connection,
            operation: Operation::Read,
            started: clock.now(),
        };
        let chunk = Chunk {
            unit_id: None,
            area: ModbusArea::ExtendedRegister,
            start: 0,
            file: NonZeroU16::new(1),
            quantity: 121,
            members: Vec::new(),
        };
        let error = block_on_with(&clock, connection.exchange(&op, &request, 0, 1, &chunk))
            .err()
            .expect("121 registers answer in more than 245 bytes");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        assert!(factory.opens().is_empty(), "no transport was needed");
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
        let connection = Link::build(
            Wire::Tcp,
            &config,
            Arc::new(factory),
            Arc::new(clock.clone()),
            ConnectionDefaults::default(),
            ModbusAddress::DEFAULT_PING,
        )
        .expect("connection");
        let address = ModbusAddress::parse("holding-register:1", &AddressOptions::default())
            .expect("address");
        let tag = Tag::new(TagId::new("a").expect("tag id"), Arc::new(address));
        let request = ReadRequest::builder(connection.limits)
            .tag(tag)
            .build()
            .expect("request");
        let op = Op {
            connection: &connection,
            operation: Operation::Read,
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

    fn quiet_connection() -> (FakeClock, Link) {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let config =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .build()
                .expect("config");
        let connection = Link::build(
            Wire::Tcp,
            &config,
            Arc::new(factory),
            Arc::new(clock.clone()),
            ConnectionDefaults::default(),
            ModbusAddress::DEFAULT_PING,
        )
        .expect("connection");
        (clock, connection)
    }

    fn item(text: &str, value: PlcValue) -> (WriteItem, ModbusAddress) {
        let address = ModbusAddress::parse(text, &AddressOptions::default()).expect("address");
        let tag = Tag::new(TagId::new("w").expect("tag id"), Arc::new(address.clone()));
        (WriteItem::new(tag, value), address)
    }

    // covers: MB-TCP-WRITE / an item that cannot be laid out is refused with its own reason
    #[test]
    fn an_item_that_cannot_be_laid_out_is_refused_without_a_request() {
        let (clock, connection) = quiet_connection();
        let op = Op {
            connection: &connection,
            operation: Operation::Write,
            started: clock.now(),
        };
        let prepare = |text: &str, value: PlcValue| {
            let (item, address) = item(text, value);
            connection.prepare(&op, &item, &address)
        };

        // The request builder checks a value against its tag, so these reach `prepare` only
        // through a hand-made item: a coil takes a BOOL, a register the type of its address.
        for (text, value) in [
            ("coil:1", PlcValue::U16(1)),
            ("holding-register:1:UINT", PlcValue::I16(1)),
            ("holding-register:1:DWORD", PlcValue::U16(1)),
        ] {
            let error = prepare(text, value).expect_err("refused");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{text}");
            assert_eq!(
                error.error().diagnostics().category,
                Some(FailureCategory::Validation),
                "{text}"
            );
        }

        // The areas that cannot be written never reach the encoder.
        for text in ["discrete-input:1", "input-register:1:UINT"] {
            let value = if text.starts_with("discrete") {
                PlcValue::Bool(true)
            } else {
                PlcValue::U16(1)
            };
            let error = prepare(text, value).expect_err("read-only");
            assert_eq!(
                error.kind(),
                ErrorKind::Unsupported(Operation::Write),
                "{text}"
            );
        }

        let planned = prepare("coil:20", PlcValue::Bool(true)).expect("a coil");
        assert_eq!(
            planned.pdu,
            ModbusPdu::WriteMultipleCoils {
                start: 19,
                quantity: 1,
                bytes: vec![1]
            }
        );
        assert_eq!(
            planned.unit_id,
            UnitId::DEFAULT,
            "the connection default unit"
        );
    }

    // covers: MB-TCP-WRITE / only an echo confirms; a reply of any other shape is not a
    // confirmation
    #[test]
    fn a_reply_that_is_not_an_echo_or_an_exception_is_not_a_confirmation() {
        let (clock, connection) = quiet_connection();
        let op = Op {
            connection: &connection,
            operation: Operation::Write,
            started: clock.now(),
        };
        let outcome = |reply: ModbusPdu| {
            let answer = Answer {
                misfit: None,
                reply,
                arrived: clock.now(),
                retries: 0,
            };
            connection.outcome_of(&op, &answer)
        };
        let shown = |reply: ModbusPdu| format!("{:?}", outcome(reply));
        assert_eq!(
            shown(ModbusPdu::WriteSingleRegisterResponse {
                address: 1,
                value: 2
            }),
            "Confirmed"
        );
        assert_eq!(
            shown(ModbusPdu::WriteMultipleResponse {
                function: crate::modbus::FunctionCode::WriteMultipleCoils,
                start: 1,
                quantity: 2
            }),
            "Confirmed"
        );
        let read = ModbusPdu::ReadRegistersResponse {
            function: crate::modbus::FunctionCode::ReadHoldingRegisters,
            registers: vec![1],
        };
        let request = ModbusPdu::ReadCoils {
            start: 0,
            quantity: 1,
        };
        for reply in [read, request] {
            assert_eq!(
                shown(reply),
                "Unknown(TimeoutAfterTransmit)",
                "never a confirmation"
            );
        }
        let refusal = shown(ModbusPdu::Exception {
            function: crate::modbus::FunctionCode::WriteSingleRegister,
            code: crate::modbus::ExceptionCode::IllegalFunction,
        });
        assert!(
            refusal.starts_with("Rejected("),
            "an exception rejects: {refusal}"
        );
        assert!(refusal.contains("kind: Unsupported(Write)"), "{refusal}");
    }

    // covers: SPEC errors / ambiguous write never replayed
    #[test]
    fn the_reason_of_an_unknown_follows_what_ended_the_wait() {
        for (kind, reason) in [
            (ErrorKind::Timeout, UnknownReason::TimeoutAfterTransmit),
            (
                ErrorKind::Disconnected,
                UnknownReason::DisconnectedAfterTransmit,
            ),
            (ErrorKind::Cancelled, UnknownReason::CancelledAfterTransmit),
            (
                ErrorKind::MalformedData,
                UnknownReason::TimeoutAfterTransmit,
            ),
            (
                ErrorKind::ResourceLimit,
                UnknownReason::TimeoutAfterTransmit,
            ),
            (ErrorKind::Internal, UnknownReason::TimeoutAfterTransmit),
        ] {
            assert_eq!(unknown_reason(kind), reason, "{kind:?}");
        }
    }

    // covers: ALPHA_PLAN S25 / a response the builder refuses is an Internal error (write)
    #[test]
    fn a_write_response_the_builder_refuses_is_an_internal_error() {
        let (clock, connection) = quiet_connection();
        let (item, _) = item("holding-register:1:UINT", PlcValue::U16(1));
        let request = WriteRequest::builder(connection.limits)
            .item(item.tag().clone(), PlcValue::U16(1))
            .build()
            .expect("request");
        let op = Op {
            connection: &connection,
            operation: Operation::Write,
            started: clock.now(),
        };
        let error = connection
            .respond_write(&op, &request, Vec::new())
            .expect_err("no result for the one item");
        assert_eq!(error.kind(), ErrorKind::MalformedData, "kind");
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Internal),
            "category"
        );
        let result = WriteResult::new(item.tag().id().clone(), WriteOutcome::Confirmed);
        let response = connection
            .respond_write(&op, &request, vec![result])
            .expect("a complete response");
        assert_eq!(response.confirmed_count(), 1);
    }
}
