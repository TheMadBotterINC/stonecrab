//! Caller-driven service exchanges on one established policy-bound channel.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-REQUEST-MATCH
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-SEQUENCE
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-REQUEST-HEADER
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-SERVICE-FAULT
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-ERR-MESSAGE
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-ERR-UNHANDLED
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-ERR

use std::collections::BTreeMap;
use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::{Duration, Instant};

use bytes::Bytes;

use crate::future::{WakerSlots, wake_all};
use crate::opcua::channel::{Channel, Renewal};
use crate::opcua::chunk::{Assembly, MessageLimits, abort, check_growth};
use crate::opcua::codec::{NodeId, UaByteString};
use crate::opcua::frame::{Body, ChunkType, Frame, SecureChunk, SequenceHeader};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::request_header::RequestHeader;
use crate::opcua::service::{decode_service, encode_service, request_header, status_error};
use crate::opcua::wire::Wire;
use crate::{CancellationToken, Clock, Deadline, ErrorKind, OpFuture, PlcError, Result};

type Framed = Wire;

struct Pending {
    queued: bool,
    reply: Option<Result<Service>>,
    assembly: Assembly,
    open: Option<(UaByteString, Instant)>,
}

struct State {
    framed: Option<Framed>,
    receiver: Option<CancellationToken>,
    pending: BTreeMap<u32, Pending>,
    waiters: WakerSlots,
    failure: Option<Arc<PlcError>>,
    request_id: u32,
    sent_sequence: u32,
    received_sequence: Option<u32>,
    discarded: u64,
    token_id: u32,
    renewal: Option<Renewal>,
    last_activity: Instant,
}

/// A bounded correlation table and a single interruptible transport owner.
pub(super) struct Lane {
    state: Mutex<State>,
    clock: Arc<dyn Clock>,
    stopped: CancellationToken,
    request_timeout: Duration,
    max_pending: usize,
    max_frame: usize,
    message_limits: MessageLimits,
    max_buffered: usize,
    channel_id: u32,
    renewal_lock: tokio::sync::Mutex<()>,
}

impl Lane {
    pub(super) fn new(
        framed: Framed,
        clock: Arc<dyn Clock>,
        request_timeout: Duration,
        channel_id: u32,
        token_id: u32,
    ) -> Self {
        let limits = *framed.transport().limits();
        let max_frame = framed.max_frame_bytes();
        Self {
            state: Mutex::new(State {
                framed: Some(framed),
                receiver: None,
                pending: BTreeMap::new(),
                waiters: WakerSlots::default(),
                failure: None,
                request_id: 0,
                sent_sequence: 0,
                received_sequence: None,
                discarded: 0,
                token_id,
                renewal: None,
                last_activity: clock.now(),
            }),
            clock,
            stopped: CancellationToken::new(),
            request_timeout,
            max_pending: limits.max_pending_requests().get(),
            max_frame,
            message_limits: MessageLimits {
                bytes: limits.max_buffered_bytes().get(),
                chunks: 64,
            },
            max_buffered: limits.max_buffered_bytes().get(),
            channel_id,
            renewal_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Continue the handshake's identifiers and apply its negotiated outbound ceiling.
    pub(super) fn from_channel(channel: Channel, request_timeout: Duration) -> Self {
        let mut lane = Self::new(
            channel.framed,
            channel.clock,
            request_timeout,
            channel.renewal.token.channel_id,
            channel.renewal.token.token_id,
        );
        lane.max_frame = lane
            .max_frame
            .min(usize::try_from(channel.limits.send_buffer_size).unwrap_or(usize::MAX));
        if channel.limits.max_message_size != 0 {
            lane.message_limits.bytes = lane
                .message_limits
                .bytes
                .min(usize::try_from(channel.limits.max_message_size).unwrap_or(usize::MAX));
        }
        if channel.limits.max_chunk_count != 0 {
            lane.message_limits.chunks = channel.limits.max_chunk_count;
        }
        let state = lane.state.get_mut().unwrap_or_else(PoisonError::into_inner);
        state.request_id = 1;
        state.sent_sequence = 1;
        state.received_sequence = Some(channel.received_sequence);
        state.renewal = Some(channel.renewal);
        lane
    }

    /// Sends once; receiving, cancelling or dropping never replays a request.
    pub(super) fn exchange<'a>(
        &'a self,
        authentication_token: &'a NodeId,
        encoding_id: u32,
        build: impl FnOnce(RequestHeader) -> Service + Send + 'a,
        deadline: Option<Deadline>,
        cancel: &'a CancellationToken,
    ) -> OpFuture<'a, Service> {
        Box::pin(async move {
            let now = self.clock.now();
            let default = Deadline::after(now, self.request_timeout)?;
            let deadline = deadline.unwrap_or(default).min(default);
            self.exchange_until(authentication_token, encoding_id, build, deadline, cancel)
                .await
        })
    }

    /// Session setup shares the whole connect bound, independently of request timeouts.
    pub(super) fn exchange_until<'a>(
        &'a self,
        authentication_token: &'a NodeId,
        encoding_id: u32,
        build: impl FnOnce(RequestHeader) -> Service + Send + 'a,
        deadline: Deadline,
        cancel: &'a CancellationToken,
    ) -> OpFuture<'a, Service> {
        self.exchange_tracked(
            authentication_token,
            encoding_id,
            build,
            deadline,
            cancel,
            None,
        )
    }

    pub(super) async fn write(
        &self,
        authentication_token: &NodeId,
        nodes: Vec<crate::opcua::generated::write_value::WriteValue>,
        deadline: Deadline,
        cancel: &CancellationToken,
        closing: &CancellationToken,
    ) -> (Result<Service>, bool) {
        let transmitted = AtomicBool::new(false);
        let mut exchange = pin!(self.exchange_tracked(
            authentication_token,
            673,
            |header| Service::WriteRequest {
                request_header: header,
                nodes_to_write: Some(nodes),
            },
            deadline,
            cancel,
            Some(&transmitted),
        ));
        let mut closing = pin!(closing.cancelled());
        let result = poll_fn(|cx| {
            if closing.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(error(ErrorKind::Disconnected, "connection is closing")));
            }
            exchange.as_mut().poll(cx)
        })
        .await;
        (result, transmitted.load(Ordering::Acquire))
    }

    fn exchange_tracked<'a>(
        &'a self,
        authentication_token: &'a NodeId,
        encoding_id: u32,
        build: impl FnOnce(RequestHeader) -> Service + Send + 'a,
        deadline: Deadline,
        cancel: &'a CancellationToken,
        transmitted: Option<&'a AtomicBool>,
    ) -> OpFuture<'a, Service> {
        Box::pin(async move {
            self.check(deadline, cancel)?;
            self.prepare(deadline, cancel).await?;
            let request = self.reserve(None)?;
            let header = request_header(
                self.clock.as_ref(),
                authentication_token,
                request.id,
                deadline,
            )?;
            let body = encode_service(encoding_id, &build(header), self.message_limits.bytes)?;
            self.exchange_body(
                request.id,
                body,
                deadline,
                cancel,
                transmitted,
                authentication_token != &NodeId::numeric(0, 0),
            )
            .await
        })
    }

    async fn exchange_body(
        &self,
        request_id: u32,
        body: Bytes,
        deadline: Deadline,
        cancel: &CancellationToken,
        transmitted: Option<&AtomicBool>,
        session_activity: bool,
    ) -> Result<Service> {
        let mut lease = match self.acquire(request_id, true, deadline, cancel).await? {
            Acquired::Transport(lease) => lease,
            Acquired::Reply(reply) => return *reply,
        };
        let capacity = lease.framed()?.max_body(self.max_frame)?;
        let (capacity, count) = self.message_limits.plan(body.len(), capacity)?;
        let first = {
            let mut state = self.lock();
            let first = state.sent_sequence.checked_add(1).ok_or_else(exhausted)?;
            let last = state
                .sent_sequence
                .checked_add(count)
                .ok_or_else(exhausted)?;
            state.sent_sequence = last;
            first
        };
        let mut sequence_number = first;
        let mut parts = body.chunks(capacity).peekable();
        lease.sending = true;
        while let Some(part) = parts.next() {
            let frame = Frame {
                chunk: if parts.peek().is_some() {
                    ChunkType::Intermediate
                } else {
                    ChunkType::Final
                },
                body: Body::Message(SecureChunk {
                    channel_id: self.channel_id,
                    token_id: self.token_id(),
                    sequence: SequenceHeader {
                        sequence_number,
                        request_id,
                    },
                    body: Bytes::copy_from_slice(part),
                }),
            };
            self.until(deadline, cancel, async {
                if let Some(transmitted) = transmitted {
                    transmitted.store(true, Ordering::Release);
                }
                lease.framed()?.send_frame(&frame, deadline).await
            })
            .await?;
            sequence_number = sequence_number.saturating_add(1);
        }
        lease.sending = false;
        if session_activity {
            self.lock().last_activity = self.clock.now();
        }
        drop(lease);
        self.receive(request_id, deadline, cancel).await
    }

    pub(super) fn wall_now(&self) -> std::time::SystemTime {
        self.clock.wall_now()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Critical sections move ownership and update counters; no I/O or wake runs here.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reserve(&self, open: Option<(UaByteString, Instant)>) -> Result<Request<'_>> {
        let (id, receiver) = {
            let mut state = self.lock();
            failure(&state)?;
            if state.pending.len() >= self.max_pending {
                return Err(error(
                    ErrorKind::Backpressure,
                    "pending request limit reached",
                ));
            }
            let id = state.request_id.checked_add(1).ok_or_else(exhausted)?;
            state.request_id = id;
            state.pending.insert(
                id,
                Pending {
                    queued: true,
                    reply: None,
                    assembly: Assembly::default(),
                    open,
                },
            );
            (id, state.receiver.clone())
        };
        if let Some(receiver) = receiver {
            receiver.cancel();
        }
        Ok(Request { lane: self, id })
    }

    fn token_id(&self) -> u32 {
        self.lock().token_id
    }

    fn next_sequence(&self) -> Result<u32> {
        let mut state = self.lock();
        let next = state.sent_sequence.checked_add(1).ok_or_else(exhausted)?;
        state.sent_sequence = next;
        drop(state);
        Ok(next)
    }

    async fn acquire(
        &self,
        id: u32,
        sending: bool,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Acquired<'_>> {
        let mut waiter = Waiter {
            lane: self,
            slot: None,
        };
        let acquire = poll_fn(|cx| {
            let (outcome, displaced) = {
                let mut state = self.lock();
                let writers = state.pending.values().any(|pending| pending.queued);
                let State {
                    pending,
                    framed,
                    receiver,
                    waiters,
                    ..
                } = &mut *state;
                let Some(pending) = pending.get_mut(&id) else {
                    return Poll::Ready(Err(missing_request()));
                };
                if let Some(reply) = pending.reply.take() {
                    return Poll::Ready(Ok(Acquired::Reply(Box::new(reply))));
                }
                let transport = if sending || !writers {
                    framed.take()
                } else {
                    None
                };
                let outcome = transport.map(|framed| {
                    let interrupt = CancellationToken::new();
                    if sending {
                        pending.queued = false;
                    } else {
                        *receiver = Some(interrupt.clone());
                    }
                    Acquired::Transport(Box::new(TransportLease {
                        lane: self,
                        framed: Some(framed),
                        interrupt,
                        sending: false,
                    }))
                });
                let displaced = if outcome.is_none() {
                    waiters.register(&mut waiter.slot, cx.waker())
                } else {
                    None
                };
                drop(state);
                (outcome, displaced)
            };
            drop(displaced);
            outcome.map_or(Poll::Pending, |outcome| Poll::Ready(Ok(outcome)))
        });
        self.until(deadline, cancel, acquire).await
    }

    async fn receive(
        &self,
        id: u32,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Service> {
        loop {
            let mut lease = match self.acquire(id, false, deadline, cancel).await? {
                Acquired::Reply(reply) => return *reply,
                Acquired::Transport(lease) => lease,
            };
            self.expire_keys(lease.framed()?);
            let read = {
                let interrupt = lease.interrupt.clone();
                let mut interrupted = pin!(interrupt.cancelled());
                let mut incoming = pin!(lease.framed()?.next_frame(deadline));
                let read = poll_fn(|cx| {
                    if interrupted.as_mut().poll(cx).is_ready() {
                        return Poll::Ready(Ok(None));
                    }
                    incoming.as_mut().poll(cx).map(|reply| reply.map(Some))
                });
                self.until(deadline, cancel, read).await
            };
            match read {
                Ok(Some(frame)) => {
                    let result = self
                        .accept_open(&frame, lease.framed()?)
                        .and_then(|()| self.dispatch(frame));
                    self.expire_keys(lease.framed()?);
                    if let Err(problem) = result {
                        self.fail(problem);
                    }
                }
                Ok(None) => {}
                Err(problem)
                    if matches!(problem.kind(), ErrorKind::Timeout | ErrorKind::Cancelled) =>
                {
                    return Err(problem);
                }
                Err(problem) => self.fail(problem),
            }
        }
    }

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "validate headers of supported session/data responses; callers reject wrong service types"
    )]
    fn dispatch(&self, frame: Frame) -> Result<()> {
        if let Body::Error(server) = frame.body {
            return Err(status_error(server.error));
        }
        let chunk = match frame.body {
            Body::Message(chunk) => {
                let mut state = self.lock();
                self.accept_token(&mut state, chunk.channel_id, chunk.token_id)?;
                chunk
            }
            Body::Open(chunk) => SecureChunk {
                channel_id: chunk.channel_id,
                token_id: self.token_id(),
                sequence: chunk.sequence,
                body: chunk.body,
            },
            _ => {
                return Err(error(
                    ErrorKind::MalformedData,
                    "expected a service message",
                ));
            }
        };
        let mut state = self.lock();
        let sequence = chunk.sequence.sequence_number;
        if sequence == 0
            || state
                .received_sequence
                .is_some_and(|previous| previous.checked_add(1) != Some(sequence))
        {
            return Err(error(
                ErrorKind::MalformedData,
                "unexpected sequence number",
            ));
        }
        state.received_sequence = Some(sequence);
        let retained = state.pending.values().fold(0_usize, |total, pending| {
            total.saturating_add(pending.assembly.len())
        });
        let Some(pending) = state.pending.get_mut(&chunk.sequence.request_id) else {
            state.discarded = state.discarded.saturating_add(1);
            drop(state);
            return Ok(());
        };
        if pending.queued || pending.reply.is_some() {
            return Err(error(
                ErrorKind::MalformedData,
                "unsolicited or duplicate response",
            ));
        }
        if frame.chunk == ChunkType::Abort {
            let problem = abort(&chunk.body)?;
            pending.assembly = Assembly::default();
            pending.reply = Some(Err(problem));
            return Ok(());
        }
        check_growth(retained, chunk.body.len(), self.max_buffered)?;
        let Some(body) = pending.assembly.push(
            &chunk.body,
            frame.chunk == ChunkType::Final,
            self.message_limits,
        )?
        else {
            return Ok(());
        };
        pending.reply = Some(decode_service(&body).and_then(|service| {
            let header = match &service {
                Service::OpenSecureChannelResponse {
                    response_header, ..
                }
                | Service::GetEndpointsResponse {
                    response_header, ..
                }
                | Service::CreateSessionResponse {
                    response_header, ..
                }
                | Service::ActivateSessionResponse {
                    response_header, ..
                }
                | Service::CloseSessionResponse { response_header }
                | Service::ReadResponse {
                    response_header, ..
                }
                | Service::WriteResponse {
                    response_header, ..
                } => Some(response_header),
                _ => None,
            };
            if header.is_some_and(|header| header.request_handle != chunk.sequence.request_id) {
                return Err(error(ErrorKind::MalformedData, "foreign response handle"));
            }
            Ok(service)
        }));
        drop(state);
        Ok(())
    }

    fn check(&self, deadline: Deadline, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            return Err(error(ErrorKind::Cancelled, "request cancelled"));
        }
        failure(&self.lock())?;
        if deadline.is_elapsed(self.clock.now()) {
            return Err(error(ErrorKind::Timeout, "request deadline elapsed"));
        }
        Ok(())
    }

    async fn until<T>(
        &self,
        deadline: Deadline,
        cancel: &CancellationToken,
        io: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut io = pin!(io);
        let mut cancelled = pin!(cancel.cancelled());
        let mut stopped = pin!(self.stopped.cancelled());
        let mut timer = pin!(self.clock.sleep_until(deadline.instant()));
        poll_fn(|cx| {
            let _cancelled = cancelled.as_mut().poll(cx);
            let _stopped = stopped.as_mut().poll(cx);
            self.check(deadline, cancel)?;
            if let Poll::Ready(result) = timer.as_mut().poll(cx) {
                result?;
                self.check(deadline, cancel)?;
                return Poll::Ready(Err(error(
                    ErrorKind::InvalidConfig,
                    "clock woke before deadline",
                )));
            }
            io.as_mut().poll(cx)
        })
        .await
    }

    pub(super) fn fail(&self, problem: PlcError) {
        let problem = Arc::new(problem);
        let (framed, wakers) = {
            let mut state = self.lock();
            state.failure.get_or_insert_with(|| Arc::clone(&problem));
            for pending in state.pending.values_mut() {
                pending.assembly = Assembly::default();
            }
            (state.framed.take(), state.waiters.drain())
        };
        drop(framed);
        self.stopped.cancel();
        wake_all(wakers);
    }

    pub(super) fn is_failed(&self) -> bool {
        self.lock().failure.is_some()
    }

    /// Continue the lane's counters for CLO and shut down without awaiting a reply.
    pub(super) async fn close(&self, deadline: Deadline, cancel: &CancellationToken) -> Result<()> {
        self.check(deadline, cancel)?;
        let request = self.reserve(None)?;
        let mut lease = match self.acquire(request.id, true, deadline, cancel).await? {
            Acquired::Transport(lease) => lease,
            Acquired::Reply(_) => {
                return Err(error(ErrorKind::MalformedData, "unsolicited close reply"));
            }
        };
        let header = request_header(
            self.clock.as_ref(),
            &NodeId::numeric(0, 0),
            request.id,
            deadline,
        )?;
        let body = encode_service(
            452,
            &Service::CloseSecureChannelRequest {
                request_header: header,
            },
            lease
                .framed()?
                .max_body(self.max_frame)?
                .min(self.message_limits.bytes),
        )?;
        let frame = Frame::single(Body::Close(SecureChunk {
            channel_id: self.channel_id,
            token_id: self.token_id(),
            sequence: SequenceHeader {
                sequence_number: self.next_sequence()?,
                request_id: request.id,
            },
            body,
        }));
        lease.sending = true;
        self.until(
            deadline,
            cancel,
            lease.framed()?.send_frame(&frame, deadline),
        )
        .await?;
        lease.sending = false;
        self.until(deadline, cancel, lease.framed()?.shutdown(deadline))
            .await
    }
}

mod renewal;

enum Acquired<'a> {
    Transport(Box<TransportLease<'a>>),
    Reply(Box<Result<Service>>),
}

struct Request<'a> {
    lane: &'a Lane,
    id: u32,
}

impl Drop for Request<'_> {
    fn drop(&mut self) {
        let (pending, wakers) = {
            let mut state = self.lane.lock();
            (state.pending.remove(&self.id), state.waiters.drain())
        };
        drop(pending);
        wake_all(wakers);
    }
}

struct Waiter<'a> {
    lane: &'a Lane,
    slot: Option<u64>,
}

impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        let waker = self
            .slot
            .and_then(|slot| self.lane.lock().waiters.release(slot));
        drop(waker);
    }
}

struct TransportLease<'a> {
    lane: &'a Lane,
    framed: Option<Framed>,
    interrupt: CancellationToken,
    sending: bool,
}

impl TransportLease<'_> {
    fn framed(&mut self) -> Result<&mut Framed> {
        self.framed
            .as_mut()
            .ok_or_else(|| error(ErrorKind::Internal, "missing transport lease"))
    }
}

impl Drop for TransportLease<'_> {
    fn drop(&mut self) {
        if self.sending {
            self.lane
                .fail(error(ErrorKind::Disconnected, "send did not complete"));
        }
        let wakers = {
            let mut state = self.lane.lock();
            state.receiver = None;
            if state.failure.is_none() {
                state.framed = self.framed.take();
            }
            state.waiters.drain()
        };
        wake_all(wakers);
    }
}

fn failure(state: &State) -> Result<()> {
    state.failure.as_ref().map_or(Ok(()), |problem| {
        Err(PlcError::new(problem.kind(), problem.to_string()).with_source(Arc::clone(problem)))
    })
}

fn error(kind: ErrorKind, message: &str) -> PlcError {
    PlcError::new(kind, format!("OPC UA lane: {message}"))
}

fn missing_request() -> PlcError {
    error(ErrorKind::Internal, "missing request slot")
}

fn exhausted() -> PlcError {
    error(
        ErrorKind::ResourceLimit,
        "identifier exhausted; open a new channel",
    )
}

impl fmt::Debug for Lane {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.lock();
        f.debug_struct("Lane")
            .field("pending", &state.pending.len())
            .field("discarded", &state.discarded)
            .field("failed", &state.failure.is_some())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
