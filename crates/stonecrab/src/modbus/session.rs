//! The Modbus session: one request in flight, a reply accepted only when it answers it.

use std::collections::VecDeque;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::transport::{ConnectionGuard, GuardConfig, GuardStats, TransportFactory};
use crate::{
    CancellationToken, Clock, ConnectionState, Deadline, ErrorKind, FailureCategory, ItemError,
    OpFuture, Operation, PlcError, ProtocolStatus, Result, StateWatch,
};
use bytes::Bytes;
use tracing::Instrument;

use crate::modbus::{
    ExceptionCode, FunctionCode, MAX_PDU_BYTES, MbapFramer, ModbusAdu, ModbusPdu, TransactionId,
    UnitId, decode_response, encode_request,
};

/// How many abandoned transaction ids the session remembers, to tell a late reply from a
/// stray one. A peer answers at most one reply per request and the guard resynchronizes
/// before the next send, so a handful is plenty; the bound keeps the memory fixed.
const ABANDONED_CAP: usize = 32;

/// The exception code `Unsupported` stands for (ILLEGAL FUNCTION).
const ILLEGAL_FUNCTION: u8 = 0x01;

/// The exception code `InvalidAddress` stands for (ILLEGAL DATA ADDRESS).
const ILLEGAL_DATA_ADDRESS: u8 = 0x02;

/// The protocol name every `ProtocolStatus` of this crate carries.
const PROTOCOL: &str = "modbus";

/// Counters a [`ModbusSession`] keeps for tests and diagnostics.
///
/// A snapshot from [`ModbusSession::stats`]; the counters only grow. `late_replies` and
/// `mismatched_replies` count the replies the session discarded without accepting: a
/// late reply carries the transaction id of a request the session had already given up
/// on (timeout, cancellation, lost connection), a mismatched one carries any other
/// transaction id, or the current id with the wrong unit id or function code. A reply the
/// guard drops before the session sees it (with a supervisor, a late reply that arrives
/// between two requests) is counted in
/// [`guard.discarded_frames`](GuardStats::discarded_frames) instead.
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
/// Upstream drops an unknown transaction id with a log line and keeps no counter; the
/// counters are Stonecrab's, part of the divergence recorded in that row.
///
/// # Examples
/// ```
/// use stonecrab::modbus::SessionStats;
///
/// let stats = SessionStats::default();
/// assert_eq!((stats.late_replies, stats.mismatched_replies), (0, 0));
/// assert_eq!(stats.guard.requests, 0);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct SessionStats {
    /// Replies discarded because their transaction id belongs to a request the session
    /// gave up on.
    pub late_replies: u64,
    /// Replies discarded because they do not answer the pending request and are not late:
    /// another transaction id, or the right id with the wrong unit id or function code.
    pub mismatched_replies: u64,
    /// The counters of the connection guard underneath (requests sent, timeouts,
    /// resyncs, reconnects, frames discarded between exchanges).
    pub guard: GuardStats,
}

/// The transaction counter and the ids the session has stopped waiting for.
struct Exchanges {
    next: TransactionId,
    abandoned: VecDeque<u16>,
}

/// Holds the session's single exchange slot until dropped, so a dropped request future
/// frees it.
struct Busy<'a>(&'a AtomicBool);

impl<'a> Busy<'a> {
    /// Takes the slot, or `None` if an exchange is running.
    fn acquire(flag: &'a AtomicBool) -> Option<Self> {
        (!flag.swap(true, Ordering::Acquire)).then_some(Self(flag))
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// A request whose reply the session still waits for. If it is dropped unsettled after its
/// frame reached the transport (an error, a timeout, a cancellation, a dropped future), the
/// session remembers its transaction id as given up on.
struct Outstanding<'a> {
    session: &'a ModbusSession,
    txn: TransactionId,
    /// The guard's `requests` counter before the call; a rise means the frame went out.
    sent_before: u64,
    settled: bool,
}

impl Drop for Outstanding<'_> {
    fn drop(&mut self) {
        if !self.settled && self.session.guard.stats().requests > self.sent_before {
            self.session.abandon(self.txn);
        }
    }
}

/// A Modbus TCP session: transaction ids, request/reply correlation and the mapping of
/// device exceptions, over a [`ConnectionGuard`] that owns the connection.
///
/// The session sends one request at a time ([`request`](Self::request), a read or a
/// write) and returns the reply PDU that answers it. A reply is accepted only when its
/// transaction id, its unit id and its function code (the reply's function with the exception bit cleared)
/// all match the request. Anything else is discarded, counted ([`SessionStats`]) and
/// logged at `warn` level with the expected and the received transaction id, and the
/// session waits for the next reply within the same deadline. A device exception is an
/// answer, not a failure of the exchange: it comes back as `Ok(ModbusPdu::Exception)`
/// with the connection still `Connected`, and [`exception_item_error`] maps it per
/// member (decision D18).
///
/// # One request in flight
/// The session owns the whole exchange, including the wait for the next reply after a
/// discard, so it runs one request at a time (`MB-READ-INFLIGHT`): a request made while
/// another is running fails with [`ErrorKind::Backpressure`] before any I/O. The bounded
/// wait queue of decision D15 belongs to the caller that owns the connection (S25), which
/// serializes its chunks; the guard's own queue is not used by the session, because a
/// request queued behind a discard could otherwise take the reply the first one waits for.
///
/// # Transaction ids and late replies
/// Ids come from a [`TransactionId`] counter that belongs to the session, so they
/// continue across a reconnect: 1 to 0xFFFE, then 1 again. When a request is given up
/// after its frame went out (timeout, cancellation, a dropped future, a lost connection, a
/// reply the guard could not frame) the session remembers its id, 32 at most, oldest
/// forgotten first. A reply that arrives whole afterwards is the first frame of a later
/// exchange (the guard hands it out as that request's reply); the session recognizes it by the remembered id,
/// counts it in `late_replies` and reads on. Replies the guard cannot frame (half a
/// reply, debris) are the guard's to resynchronize before the next request, as its
/// documentation describes.
///
/// # Resynchronization
/// The guard offers no resynchronization entry point to its caller. It resynchronizes
/// by itself, within the next request's deadline, after a timeout, a cancellation or a
/// framing error, and reconnects when that fails. The session does not need more: the
/// MBAP framer cuts exactly one ADU from the stream by the length field, so a reply whose
/// PDU does not decode or does not fit the request ([`ErrorKind::MalformedData`]) leaves
/// the stream aligned, and nothing stays buffered that the next request must discard.
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
/// Upstream matches a reply to its request by transaction id alone, one request in flight
/// (`MB-READ-INFLIGHT`), and times a request out after `request-timeout-ms`
/// (`MB-READ-TIMEOUT`); writes go through the same exchange (`MB-TCP-WRITE`). Checking the
/// unit id and function code as well is a divergence (Modbus Application Protocol V1.1b
/// section 7: a server echoes the unit identifier and the function code); the transaction
/// counter is `MB-READ-TXN`.
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline, Endpoint, ResourceLimits};
/// use stonecrab::modbus::{FunctionCode, ModbusPdu, ModbusSession, UnitId};
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
/// use stonecrab::transport::GuardConfig;
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// // Modbus Application Protocol V1.1b section 6.3: read registers 108 to 110 of unit 17.
/// factory.push_script(Script::strict(vec![
///     Step::Expect(vec![0, 1, 0, 0, 0, 6, 0x11, 3, 0x00, 0x6B, 0x00, 0x03]),
///     Step::Reply(vec![0, 1, 0, 0, 0, 9, 0x11, 3, 6, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]),
/// ]));
/// let limits = ResourceLimits::builder().build()?;
/// let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
/// let session = ModbusSession::new(config, Arc::new(factory), Arc::new(clock.clone()));
///
/// block_on_with(&clock, session.connect())?;
/// let request = ModbusPdu::ReadHoldingRegisters { start: 107, quantity: 3 };
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
/// let reply = block_on_with(&clock, session.request(&request, UnitId::from(17), deadline, None))?;
/// assert_eq!(
///     reply,
///     ModbusPdu::ReadRegistersResponse {
///         function: FunctionCode::ReadHoldingRegisters,
///         registers: vec![555, 0, 100],
///     }
/// );
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct ModbusSession {
    guard: ConnectionGuard<ModbusAdu, MbapFramer>,
    clock: Arc<dyn Clock>,
    /// `host:port`, for the span; the endpoint carries no credentials.
    endpoint: String,
    exchanges: Mutex<Exchanges>,
    busy: AtomicBool,
    late: AtomicU64,
    mismatched: AtomicU64,
}

/// Shows the endpoint, the state and the counters.
impl fmt::Debug for ModbusSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusSession")
            .field("endpoint", &self.endpoint)
            .field("state", &self.guard.state())
            .field("late_replies", &self.late.load(Ordering::Relaxed))
            .field(
                "mismatched_replies",
                &self.mismatched.load(Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl ModbusSession {
    /// A session in the `Disconnected` state; nothing is opened until
    /// [`connect`](Self::connect).
    ///
    /// `config` says where to connect and with which timeouts, reconnect policy and
    /// limits (build it from a `ConnectionConfig` with `GuardConfig::from`); `factory`
    /// opens the transports (the real TCP factory in production, the testkit's
    /// `FakeTransportFactory` in tests); `clock` is the one source of time (decision D13).
    /// The `limits.max_frame_bytes` should be at least `ModbusAdu::MAX_BYTES` (260).
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-TXN
    /// The first transaction id is 1, as upstream's counter.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab::modbus::ModbusSession;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(session.state(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn new(
        config: GuardConfig,
        factory: Arc<dyn TransportFactory>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let endpoint = config.endpoint.to_string();
        let guard = ConnectionGuard::new(
            config,
            factory,
            Box::new(MbapFramer::new),
            Arc::clone(&clock),
        );
        Self {
            guard,
            clock,
            endpoint,
            exchanges: Mutex::new(Exchanges {
                next: TransactionId::FIRST,
                abandoned: VecDeque::new(),
            }),
            busy: AtomicBool::new(false),
            late: AtomicU64::new(0),
            mismatched: AtomicU64::new(0),
        }
    }

    /// Start the transaction counter at `first` instead of 1.
    ///
    /// For a driver that resumes a counter, and for tests that exercise the wrap from
    /// 0xFFFE to 1 without sending 65534 requests.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-TXN
    /// The counter wraps from 0xFFFE to 1 as upstream's does ([`TransactionId::next`]).
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{Endpoint, ResourceLimits};
    /// use stonecrab::modbus::{ModbusSession, TransactionId};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Arc::new(clock),
    /// )
    /// .starting_at(TransactionId::LAST);
    /// # let _ = session;
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn starting_at(self, first: TransactionId) -> Self {
        self.lock().next = first;
        self
    }

    /// The current lifecycle state of the connection underneath.
    ///
    /// A device exception leaves it `Connected`; a lost connection shows as
    /// `Reconnecting`, a spent retry budget as `Failed`.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab::modbus::ModbusSession;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(session.state(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn state(&self) -> ConnectionState {
        self.guard.state()
    }

    /// An observer of the lifecycle state, with the current state counted as seen.
    ///
    /// Latest value only: transitions between two polls coalesce (ADR-0007).
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab::modbus::ModbusSession;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(session.watch_state().current(), ConnectionState::Disconnected);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn watch_state(&self) -> StateWatch {
        self.guard.watch_state()
    }

    /// A snapshot of the session's counters and the guard's.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{Endpoint, ResourceLimits};
    /// use stonecrab::modbus::{ModbusSession, SessionStats};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(FakeTransportFactory::new(clock.clone())),
    ///     Arc::new(clock),
    /// );
    /// assert_eq!(session.stats(), SessionStats::default());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn stats(&self) -> SessionStats {
        SessionStats {
            late_replies: self.late.load(Ordering::Relaxed),
            mismatched_replies: self.mismatched.load(Ordering::Relaxed),
            guard: self.guard.stats(),
        }
    }

    /// Open the connection: `Disconnected` to `Connecting` to `Connected`.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-RECONNECT
    /// No upstream parity claim; this is the guard's `connect`.
    ///
    /// # Errors
    /// As `ConnectionGuard::connect`: [`ErrorKind::Timeout`] if the open did not finish
    /// within `timeouts.connect`, [`ErrorKind::Disconnected`] if the peer refused or the
    /// session is closed or failed, and any other kind the factory reports. The failure
    /// carries `Diagnostics` with `operation = Connect`.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped call leaves no transport behind and restores the state.
    ///
    /// # Wire effects
    /// Opens a connection and sends no application bytes, bounded by `timeouts.connect`.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab::modbus::ModbusSession;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(factory),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, session.connect())?;
    /// assert_eq!(session.state(), ConnectionState::Connected);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn connect(&self) -> OpFuture<'_, ()> {
        self.guard.connect()
    }

    /// Start the guard's supervisor (`ConnectionGuard::supervise`): background reconnect
    /// and the idle bound. False outside a Tokio runtime, where nothing is started.
    pub(crate) fn supervise(&self) -> bool {
        self.guard.supervise()
    }

    /// Close the connection: `Closing`, then `Closed`.
    ///
    /// A running request fails with [`ErrorKind::Disconnected`]. The call is idempotent
    /// and bounded by `timeouts.request`.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-GUARD-LIFECYCLE
    /// No upstream parity claim; this is the guard's `close`.
    ///
    /// # Errors
    /// [`ErrorKind::Timeout`] if the shutdown did not finish by the bound; the state is
    /// `Closed` regardless. The failure carries `Diagnostics` with `operation = Close`.
    ///
    /// # Cancel safety
    /// Cancel-safe: the state is already `Closing`, and a repeat call finishes the job.
    ///
    /// # Wire effects
    /// Half-closes the transport (a FIN for TCP).
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Endpoint, ResourceLimits};
    /// use stonecrab::modbus::ModbusSession;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let limits = ResourceLimits::builder().build()?;
    /// let session = ModbusSession::new(
    ///     GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits),
    ///     Arc::new(factory),
    ///     Arc::new(clock.clone()),
    /// );
    /// block_on_with(&clock, session.connect())?;
    /// block_on_with(&clock, session.close())?;
    /// assert_eq!(session.state(), ConnectionState::Closed);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn close(&self) -> OpFuture<'_, ()> {
        self.guard.close()
    }

    /// Send one request, a read or a write, to `unit_id` and return the reply PDU that
    /// answers it.
    ///
    /// The request is encoded with [`encode_request`] into a writer bounded by
    /// [`MAX_PDU_BYTES`], framed with the next transaction id and sent through the guard.
    /// A reply is accepted when its transaction id, unit id and function code match; a
    /// reply that does not match is discarded, counted and logged (see the type) and the
    /// session waits for the next one within the same `deadline`. The accepted PDU is
    /// decoded with [`decode_response`] and proved against the request with
    /// [`ModbusPdu::check_reply_to`] (byte count against quantity for a read, the echo of
    /// address and quantity or value for a write). The result is a
    /// [`ModbusPdu::ReadBitsResponse`], a [`ModbusPdu::ReadRegistersResponse`] or the echo
    /// of a write ([`ModbusPdu::WriteSingleRegisterResponse`] or
    /// [`ModbusPdu::WriteMultipleResponse`]), or `Ok(ModbusPdu::Exception { .. })` when the
    /// device refused the request: the connection stays `Connected` and
    /// [`exception_item_error`] gives the per-member error.
    ///
    /// The deadline source is the `deadline` argument and nothing else: queueing, the
    /// backoff, the open, the send, the resync and every wait for a reply (the first and
    /// each after a discard) end at it. `token` is honoured as `ConnectionGuard::request`
    /// documents. A span `request` (debug level) covers the call, with the fields
    /// `scheme = "modbus-tcp"`, `endpoint` (host and port), `op` (`"read"`, or `"write"`
    /// for a write request) and `txn`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MATCH
    /// Upstream matches by transaction id only (`MB-READ-MATCH`), keeps one request in
    /// flight (`MB-READ-INFLIGHT`) and times out after `request-timeout-ms`
    /// (`MB-READ-TIMEOUT`); the unit id and function checks, the counters and the
    /// byte-count proof are Stonecrab's. Exceptions map per `MB-EXC-01` and its siblings.
    ///
    /// # Errors
    /// Every error carries `Diagnostics` with `operation = Read` (the connection re-stamps
    /// it for a write), the connection state at the failure, the latency on the session's
    /// clock, the reconnect attempt as `retries` and a category.
    /// - [`ErrorKind::Backpressure`] if another request of this session is running;
    ///   nothing was sent. The guard also reports it when its own queue is full.
    /// - [`ErrorKind::ResourceLimit`] or [`ErrorKind::MalformedData`] if `pdu` is not a
    ///   request or is out of bounds (quantity 0, too large, past the address space);
    ///   nothing was sent. The same kinds come from the guard for a reply that
    ///   violates the framing, after which the guard resynchronizes before the next
    ///   request.
    /// - [`ErrorKind::MalformedData`] (category `Protocol`) if the matching reply's PDU
    ///   does not decode, or its byte count or echo does not fit the request. The stream
    ///   stays aligned.
    /// - [`ErrorKind::Cancelled`] if `token` fired: nothing was sent when it fired before
    ///   the flush; the frame may have reached the peer when it fired later.
    /// - [`ErrorKind::Timeout`] if `deadline` passed, in any phase. After the flush the
    ///   reply may still come; the session counts it as late when it does.
    /// - [`ErrorKind::Disconnected`] if the session is not open, the peer closed, or the
    ///   send failed. The state is then `Reconnecting` (the next request reopens), or
    ///   `Failed` when the retry budget is spent.
    ///
    /// # Cancel safety
    /// Cancel-safe in the session's own state: a dropped future frees the exchange slot.
    /// If the frame had gone out, its reply may still arrive; the session records the id
    /// when the future is dropped and the next request discards the reply as late, after
    /// the guard has resynchronized. A request whose future is dropped while the frame is
    /// being written poisons the transport, as the guard documents.
    ///
    /// # Wire effects
    /// Transmits one request frame, once; the session never replays a request (retries of a
    /// read are the connection's policy, S25; a write is never retried). Receiving
    /// transmits nothing.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use stonecrab::{Clock, Deadline, Endpoint, ErrorKind, ResourceLimits};
    /// use stonecrab::modbus::{ExceptionCode, FunctionCode, ModbusPdu, ModbusSession, UnitId};
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
    /// use stonecrab::transport::GuardConfig;
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// // The device refuses: exception 02, ILLEGAL DATA ADDRESS, for function 03.
    /// factory.push_script(Script::new(vec![
    ///     Step::Expect(vec![0, 1, 0, 0, 0, 6, 1, 3, 0xFF, 0xFF, 0, 1]),
    ///     Step::Reply(vec![0, 1, 0, 0, 0, 3, 1, 0x83, 2]),
    /// ]));
    /// let limits = ResourceLimits::builder().build()?;
    /// let config = GuardConfig::new(Endpoint::Custom("fake://plc".to_owned()), limits);
    /// let session = ModbusSession::new(config, Arc::new(factory), Arc::new(clock.clone()));
    /// block_on_with(&clock, session.connect())?;
    ///
    /// let request = ModbusPdu::ReadHoldingRegisters { start: 0xFFFF, quantity: 1 };
    /// let deadline = Deadline::after(clock.now(), Duration::from_secs(1))?;
    /// let reply = block_on_with(&clock, session.request(&request, UnitId::from(1), deadline, None))?;
    /// assert_eq!(
    ///     reply,
    ///     ModbusPdu::Exception {
    ///         function: FunctionCode::ReadHoldingRegisters,
    ///         code: ExceptionCode::IllegalDataAddress,
    ///     }
    /// );
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn request<'a>(
        &'a self,
        pdu: &'a ModbusPdu,
        unit_id: UnitId,
        deadline: Deadline,
        token: Option<&'a CancellationToken>,
    ) -> OpFuture<'a, ModbusPdu> {
        let span = tracing::debug_span!(
            "request",
            scheme = "modbus-tcp",
            endpoint = %self.endpoint,
            op = operation_name(pdu.function()),
            txn = tracing::field::Empty
        );
        Box::pin(
            self.exchange(pdu, unit_id, deadline, token, span.clone())
                .instrument(span),
        )
    }

    /// The whole exchange: claim the slot, send, accept or discard replies, decode.
    async fn exchange(
        &self,
        request: &ModbusPdu,
        unit_id: UnitId,
        deadline: Deadline,
        token: Option<&CancellationToken>,
        span: tracing::Span,
    ) -> Result<ModbusPdu> {
        let started = self.clock.now();
        let Some(_slot) = Busy::acquire(&self.busy) else {
            return Err(self.stamp(
                PlcError::new(
                    ErrorKind::Backpressure,
                    "another request of this session is running",
                ),
                started,
                FailureCategory::Validation,
            ));
        };
        let txn = self.issue();
        span.record("txn", txn.get());
        let frame = Self::frame(request, unit_id, txn)
            .map_err(|error| self.stamp(error, started, FailureCategory::Validation))?;
        let mut outstanding = Outstanding {
            session: self,
            txn,
            sent_before: self.guard.stats().requests,
            settled: false,
        };
        // The guard keeps the in-flight slot from the send to the reply that answers, so
        // a supervisor never comes between two reads of the correlation.
        let mut answers = |reply: &ModbusAdu| {
            let answered = Self::answers(reply, txn, unit_id, request.function());
            if !answered {
                self.discard(reply, txn);
            }
            answered
        };
        let reply = self
            .guard
            .request_matching(frame, deadline, token, &mut answers)
            .await
            .map_err(|error| self.stamp(error, started, FailureCategory::Transport))?;
        outstanding.settled = true;
        self.interpret(request, &reply, started)
    }

    /// The request PDU as an ADU with the next transaction id.
    fn frame(request: &ModbusPdu, unit_id: UnitId, txn: TransactionId) -> Result<ModbusAdu> {
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_request(request, &mut writer).map_err(PlcError::from)?;
        ModbusAdu::new(txn.get(), unit_id, Bytes::from(writer.into_bytes())).map_err(PlcError::from)
    }

    /// Whether `reply` answers the request: transaction id, unit id and function code (the
    /// reply's function with the exception bit cleared) all match.
    fn answers(
        reply: &ModbusAdu,
        txn: TransactionId,
        unit_id: UnitId,
        function: FunctionCode,
    ) -> bool {
        reply.transaction_id() == txn.get()
            && reply.unit_id() == unit_id
            && reply
                .pdu()
                .first()
                .is_some_and(|byte| FunctionCode::from_u8(byte & 0x7F) == function)
    }

    /// Decode the matching reply and prove it against the request. A failure leaves the
    /// stream aligned (the framer cut exactly one ADU), so nothing is abandoned or
    /// resynchronized.
    fn interpret(
        &self,
        request: &ModbusPdu,
        reply: &ModbusAdu,
        started: Instant,
    ) -> Result<ModbusPdu> {
        let mut reader = BoundedReader::new(reply.pdu());
        decode_response(&mut reader, request.function())
            .and_then(|pdu| pdu.check_reply_to(request).map(|()| pdu))
            .map_err(|error| self.malformed(&error, started))
    }

    /// The error for a matching reply whose PDU is not an answer to the request.
    fn malformed(&self, error: &ParseError, started: Instant) -> PlcError {
        self.stamp(
            PlcError::new(
                ErrorKind::MalformedData,
                format!("the reply PDU does not answer the request: {error}"),
            )
            .with_source(error.clone()),
            started,
            FailureCategory::Protocol,
        )
    }

    /// Count a reply that does not answer the request and say so at `warn` level.
    fn discard(&self, reply: &ModbusAdu, expected: TransactionId) {
        let late = self.lock().abandoned.contains(&reply.transaction_id());
        let counter = if late { &self.late } else { &self.mismatched };
        counter.fetch_add(1, Ordering::Relaxed);
        tracing::warn!(
            expected_txn = expected.get(),
            received_txn = reply.transaction_id(),
            received_unit = reply.unit_id().get(),
            late,
            "discarded a reply that does not answer the pending request"
        );
    }

    /// The next transaction id; an id the counter wraps back to is no longer abandoned.
    fn issue(&self) -> TransactionId {
        let mut exchanges = self.lock();
        let txn = exchanges.next.next();
        exchanges.abandoned.retain(|old| *old != txn.get());
        txn
    }

    /// Remember that the session stopped waiting for `txn`, forgetting the oldest id past
    /// the bound.
    fn abandon(&self, txn: TransactionId) {
        let mut exchanges = self.lock();
        if exchanges.abandoned.len() >= ABANDONED_CAP {
            exchanges.abandoned.pop_front();
        }
        exchanges.abandoned.push_back(txn.get());
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Exchanges> {
        self.exchanges
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// `error` with the diagnostics of a read filled in where the failure site left them
    /// out: the operation always, the state, the latency and the category when absent. The
    /// guard's own values (state at the failure, latency, reconnect attempt) are kept.
    fn stamp(&self, error: PlcError, started: Instant, category: FailureCategory) -> PlcError {
        let mut diagnostics = error.diagnostics().clone();
        diagnostics.operation = Some(Operation::Read);
        diagnostics
            .connection_state
            .get_or_insert_with(|| self.guard.state());
        diagnostics
            .latency
            .get_or_insert_with(|| self.clock.now().saturating_duration_since(started));
        diagnostics.category.get_or_insert(category);
        error.with_diagnostics(diagnostics)
    }
}

/// The span name of a request: a write function is a `"write"`, every other a `"read"`.
const fn operation_name(function: FunctionCode) -> &'static str {
    match function {
        FunctionCode::WriteSingleRegister
        | FunctionCode::WriteMultipleCoils
        | FunctionCode::WriteMultipleRegisters => "write",
        FunctionCode::ReadCoils
        | FunctionCode::ReadDiscreteInputs
        | FunctionCode::ReadHoldingRegisters
        | FunctionCode::ReadInputRegisters
        | FunctionCode::Unknown(_) => "read",
    }
}

/// The per-member error for a device exception, by decision D18.
///
/// Code 1 (ILLEGAL FUNCTION) is [`ErrorKind::Unsupported`]`(`[`Operation::Read`]`)`, code
/// 2 (ILLEGAL DATA ADDRESS) is [`ErrorKind::InvalidAddress`], and every other code,
/// including 3 and 5, and a number outside the ten defined codes, is
/// [`ErrorKind::DeviceException`]. The error's [`ProtocolStatus`] is always
/// `ProtocolStatus::new("modbus", code)` with the V1.1b name of the code
/// ([`ExceptionCode::name`]), so the raw number and the name are never lost. The
/// `Diagnostics` carry `operation = Read` and the `Protocol` category; the connection
/// state is left absent because the function knows none (the connection slice, which
/// applies the error per member, sets what it knows).
///
/// An acknowledge (code 5) is never success: upstream's mapping to OK is a documented
/// deviation (`MB-EXC-05`).
///
/// # Compatibility: docs/parity/modbus.md#MB-EXC-01
/// The mapping rows are `MB-EXC-01` to `MB-EXC-0B`; code 3 (`MB-EXC-03`, upstream
/// `INVALID_DATA`) and code 5 (`MB-EXC-05`, upstream OK) are deviations, with Modbus
/// Application Protocol V1.1b section 7 as the reference. `MB-EXC-UNKNOWN` covers a code
/// the wire decoder refuses; the function still maps one that a caller constructs.
///
/// # Examples
/// ```
/// use stonecrab::{ErrorKind, Operation};
/// use stonecrab::modbus::{ExceptionCode, FunctionCode, exception_item_error};
///
/// let refused = exception_item_error(
///     FunctionCode::ReadHoldingRegisters,
///     ExceptionCode::IllegalDataAddress,
/// );
/// assert_eq!(refused.kind(), ErrorKind::InvalidAddress);
/// let status = refused.status().ok_or("status")?;
/// assert_eq!((status.protocol(), status.code(), status.name()), ("modbus", 2, Some("ILLEGAL DATA ADDRESS")));
///
/// let busy = exception_item_error(FunctionCode::ReadCoils, ExceptionCode::ServerDeviceBusy);
/// assert_eq!(busy.kind(), ErrorKind::DeviceException);
/// let unsupported = exception_item_error(FunctionCode::ReadCoils, ExceptionCode::IllegalFunction);
/// assert_eq!(unsupported.kind(), ErrorKind::Unsupported(Operation::Read));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[must_use]
pub fn exception_item_error(function: FunctionCode, code: ExceptionCode) -> ItemError {
    exception_error(Operation::Read, function, code)
}

/// The error [`exception_item_error`] builds, for the operation that was refused: code 1
/// is `Unsupported(operation)`, so a device that does not implement a write function is
/// `Unsupported(Operation::Write)`, and the diagnostics name `operation`.
pub(super) fn exception_error(
    operation: Operation,
    function: FunctionCode,
    code: ExceptionCode,
) -> ItemError {
    let kind = match code.as_u8() {
        ILLEGAL_FUNCTION => ErrorKind::Unsupported(operation),
        ILLEGAL_DATA_ADDRESS => ErrorKind::InvalidAddress,
        _ => ErrorKind::DeviceException,
    };
    let diagnostics = crate::Diagnostics {
        operation: Some(operation),
        category: Some(FailureCategory::Protocol),
        ..crate::Diagnostics::default()
    };
    let error = PlcError::new(
        kind,
        format!(
            "modbus exception {} ({}) for function {:#04x}",
            code.as_u8(),
            code.name(),
            function.as_u8()
        ),
    )
    .with_diagnostics(diagnostics);
    ItemError::new(error)
        .with_status(ProtocolStatus::new(PROTOCOL, u32::from(code.as_u8())).with_name(code.name()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: MB-TCP-WRITE / the request span says what the frame does
    #[test]
    fn the_request_span_names_a_write_function_a_write() {
        for function in [
            FunctionCode::WriteSingleRegister,
            FunctionCode::WriteMultipleCoils,
            FunctionCode::WriteMultipleRegisters,
        ] {
            assert_eq!(operation_name(function), "write", "{function:?}");
        }
        for function in [
            FunctionCode::ReadCoils,
            FunctionCode::ReadDiscreteInputs,
            FunctionCode::ReadHoldingRegisters,
            FunctionCode::ReadInputRegisters,
            FunctionCode::Unknown(0x2B),
        ] {
            assert_eq!(operation_name(function), "read", "{function:?}");
        }
    }

    // covers: MB-EXC-01 / ILLEGAL FUNCTION is unsupported for the operation that was refused
    #[test]
    fn illegal_function_is_unsupported_for_the_operation_that_was_refused() {
        for operation in [Operation::Read, Operation::Write] {
            let error = exception_error(
                operation,
                FunctionCode::ReadCoils,
                ExceptionCode::IllegalFunction,
            );
            assert_eq!(error.kind(), ErrorKind::Unsupported(operation));
            assert_eq!(error.error().diagnostics().operation, Some(operation));
        }
        let read = exception_item_error(FunctionCode::ReadCoils, ExceptionCode::IllegalFunction);
        assert_eq!(read.kind(), ErrorKind::Unsupported(Operation::Read));
    }
}
