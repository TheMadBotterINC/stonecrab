//! Caller-driven service exchanges on one established policy-None channel.
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
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;

use crate::codec::{BoundedReader, BoundedWriter};
use crate::future::{WakerSlots, wake_all};
use crate::opcua::codec::{
    DateTime, Depth, ExpandedNodeId, ExtensionBody, ExtensionObject, Identifier, NodeId,
    StatusCode, UaString,
};
use crate::opcua::frame::{Body, ChunkType, Frame, OpcuaFramer, SecureChunk, SequenceHeader};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::request_header::RequestHeader;
use crate::transport::FramedTransport;
use crate::{CancellationToken, Clock, Deadline, ErrorKind, OpFuture, PlcError, Result};

type Framed = FramedTransport<Frame, OpcuaFramer>;

struct Pending {
    queued: bool,
    reply: Option<Result<Service>>,
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
}

/// A bounded correlation table and a single interruptible transport owner.
pub(super) struct Lane {
    state: Mutex<State>,
    clock: Arc<dyn Clock>,
    stopped: CancellationToken,
    request_timeout: Duration,
    max_pending: usize,
    max_frame: usize,
    channel_id: u32,
    token_id: u32,
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
            }),
            clock,
            stopped: CancellationToken::new(),
            request_timeout,
            max_pending: limits.max_pending_requests().get(),
            max_frame,
            channel_id,
            token_id,
        }
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
            self.check(deadline, cancel)?;
            let request = self.reserve()?;
            let header = self.header(authentication_token, request.id, deadline)?;
            let mut writer = BoundedWriter::new(self.max_frame.saturating_sub(24));
            NodeId::numeric(0, encoding_id).serialize(&mut writer, Depth::new())?;
            build(header).serialize(&mut writer, Depth::new())?;
            let body = Bytes::from(writer.into_bytes());
            self.exchange_body(request.id, body, deadline, cancel).await
        })
    }

    async fn exchange_body(
        &self,
        request_id: u32,
        body: Bytes,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Service> {
        let mut lease = match self.acquire(request_id, true, deadline, cancel).await? {
            Acquired::Transport(lease) => lease,
            Acquired::Reply(reply) => return *reply,
        };
        let sequence_number = self.next_sequence()?;
        let frame = Frame::single(Body::Message(SecureChunk {
            channel_id: self.channel_id,
            token_id: self.token_id,
            sequence: SequenceHeader {
                sequence_number,
                request_id,
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
        drop(lease);
        self.receive(request_id, deadline, cancel).await
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Critical sections move ownership and update counters; no I/O or wake runs here.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn reserve(&self) -> Result<Request<'_>> {
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
                },
            );
            (id, state.receiver.clone())
        };
        if let Some(receiver) = receiver {
            receiver.cancel();
        }
        Ok(Request { lane: self, id })
    }

    fn header(&self, token: &NodeId, id: u32, deadline: Deadline) -> Result<RequestHeader> {
        let timestamp = DateTime::from_system_time(self.clock.wall_now())
            .ok_or_else(|| error(ErrorKind::InvalidConfig, "clock timestamp is out of range"))?;
        let remaining = deadline
            .instant()
            .saturating_duration_since(self.clock.now());
        Ok(RequestHeader {
            authentication_token: token.clone(),
            timestamp: timestamp.0,
            request_handle: id,
            return_diagnostics: 0,
            audit_entry_id: UaString::null(),
            timeout_hint: u32::try_from(remaining.as_millis())
                .unwrap_or(u32::MAX)
                .max(1),
            additional_header: ExtensionObject {
                type_id: ExpandedNodeId::local(NodeId::numeric(0, 0)),
                body: ExtensionBody::Null,
            },
        })
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
                    return Poll::Ready(Err(error(ErrorKind::Internal, "missing request slot")));
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
            let read = {
                let interrupt = lease.interrupt.clone();
                let mut interrupted = pin!(interrupt.cancelled());
                let mut incoming = lease.framed()?.next_frame(deadline);
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
                    if let Err(problem) = self.dispatch(frame) {
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

    fn dispatch(&self, frame: Frame) -> Result<()> {
        if let Body::Error(server) = frame.body {
            return Err(status_error(server.error));
        }
        let Body::Message(chunk) = frame.body else {
            return Err(error(
                ErrorKind::MalformedData,
                "expected a service message",
            ));
        };
        if frame.chunk != ChunkType::Final {
            return Err(error(
                ErrorKind::MalformedData,
                "multi-chunk services require S94",
            ));
        }
        if chunk.channel_id != self.channel_id || chunk.token_id != self.token_id {
            return Err(error(
                ErrorKind::MalformedData,
                "foreign channel or token id",
            ));
        }
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
        pending.reply = Some(decode_service(&chunk.body));
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

    fn fail(&self, problem: PlcError) {
        let problem = Arc::new(problem);
        let (framed, wakers) = {
            let mut state = self.lock();
            state.failure.get_or_insert_with(|| Arc::clone(&problem));
            (state.framed.take(), state.waiters.drain())
        };
        drop(framed);
        self.stopped.cancel();
        wake_all(wakers);
    }
}

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

fn decode_service(body: &[u8]) -> Result<Service> {
    let mut reader = BoundedReader::new(body);
    let id = NodeId::parse(&mut reader, Depth::new())?;
    let Identifier::Numeric(id_value) = id.identifier else {
        return Err(error(
            ErrorKind::MalformedData,
            "non-numeric service encoding id",
        ));
    };
    if id.namespace != 0 {
        return Err(error(
            ErrorKind::MalformedData,
            "non-zero service namespace",
        ));
    }
    let id = i32::try_from(id_value)
        .map_err(|_range| error(ErrorKind::MalformedData, "service encoding id out of range"))?;
    let service = Service::parse(&mut reader, Depth::new(), id)?;
    reader.expect_eof()?;
    if let Service::ServiceFault { response_header } = &service {
        return Err(status_error(response_header.service_result));
    }
    Ok(service)
}

/// Exact device status, also retained through a channel-wide failure's source chain.
#[derive(Debug, thiserror::Error)]
#[error("OPC UA status 0x{code:08X} ({name})")]
pub(super) struct StatusFault {
    pub(super) code: u32,
    pub(super) name: &'static str,
}

fn status_error(status: StatusCode) -> PlcError {
    let source = StatusFault {
        code: status.0,
        name: status.name().unwrap_or("unknown"),
    };
    PlcError::new(ErrorKind::DeviceException, source.to_string()).with_source(source)
}

fn failure(state: &State) -> Result<()> {
    state.failure.as_ref().map_or(Ok(()), |problem| {
        Err(PlcError::new(problem.kind(), problem.to_string()).with_source(Arc::clone(problem)))
    })
}

fn error(kind: ErrorKind, message: &str) -> PlcError {
    PlcError::new(kind, format!("OPC UA lane: {message}"))
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
