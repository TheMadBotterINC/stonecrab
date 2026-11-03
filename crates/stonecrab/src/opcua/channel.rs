//! Policy-None channel establishment before ownership transfers to the request lane.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-HELLO
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-OPEN
//! # Compatibility: docs/parity/opcua.md#OPCUA-POLICY-NONE
//! # Compatibility: docs/parity/opcua.md#OPCUA-MODE-POLICY-NONE
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIMIT-RECEIVE-BUFFER
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-ERR

use std::fmt;
use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;

use crate::opcua::codec::{NodeId, UaByteString, UaString};
use crate::opcua::frame::{
    AsymmetricHeader, Body, ChunkType, Frame, Hello, OpcuaFramer, OpenChunk, SecureChunk,
    SequenceHeader, TransportLimits,
};
use crate::opcua::generated::channel_security_token::ChannelSecurityToken;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::security_token_request_type::SecurityTokenRequestType;
use crate::opcua::options::{MessageSecurity, OpcuaOptions, SecurityPolicy};
use crate::opcua::service::{decode_service, encode_service, request_header, status_error};
use crate::transport::{FramedTransport, Transport};
use crate::{CancellationToken, Clock, Deadline, ErrorKind, PlcError, ResourceLimits, Result};

const DEFAULT_BUFFER: u32 = 65535;
const MIN_BUFFER: u32 = 8192;
const DEFAULT_LIFETIME: u32 = 3_600_000;

pub(super) struct Channel {
    pub(super) framed: FramedTransport<Frame, OpcuaFramer>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) limits: TransportLimits,
    pub(super) token: ChannelSecurityToken,
    pub(super) server_nonce: UaByteString,
    pub(super) received_sequence: u32,
    configured_mode: MessageSecurity,
}

impl Channel {
    pub(super) async fn open_none(
        transport: Box<dyn Transport>,
        options: &OpcuaOptions,
        clock: Arc<dyn Clock>,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        if options.security_policy != SecurityPolicy::None {
            return Err(error(
                ErrorKind::InvalidConfig,
                "policy None must be selected explicitly",
            ));
        }
        let resources = *transport.limits();
        let proposed = proposed_limits(resources, options.receive_buffer_size)?;
        let framer = OpcuaFramer::with_buffers(
            proposed.receive_buffer_size,
            proposed.send_buffer_size,
            resources,
        );
        let mut framed = FramedTransport::new(transport, framer);
        let hello = Frame::single(Body::Hello(Hello {
            protocol_version: 0,
            limits: proposed,
            endpoint_url: UaString::new(&options.endpoint_url),
        }));
        within(
            clock.as_ref(),
            deadline,
            cancel,
            framed.send_frame(&hello, deadline),
        )
        .await?;
        let reply = within(
            clock.as_ref(),
            deadline,
            cancel,
            framed.next_frame(deadline),
        )
        .await?;
        check_frame(&reply)?;
        let Body::Acknowledge(ack) = reply.body else {
            return Err(error(ErrorKind::MalformedData, "expected Acknowledge"));
        };
        if ack.protocol_version != 0 {
            return Err(error(
                ErrorKind::MalformedData,
                "unsupported protocol version",
            ));
        }
        let limits = negotiate(proposed, ack.limits)?;
        framed.replace_framer(OpcuaFramer::with_buffers(
            limits.receive_buffer_size,
            limits.send_buffer_size,
            resources,
        ));
        let security = AsymmetricHeader::none();
        let overhead = security.length_in_bytes()?.saturating_add(20);
        let header = request_header(clock.as_ref(), &NodeId::numeric(0, 0), 1, deadline)?;
        let request = Service::OpenSecureChannelRequest {
            request_header: header,
            client_protocol_version: 0,
            request_type: SecurityTokenRequestType::SecurityTokenRequestTypeIssue,
            security_mode: MessageSecurityMode::MessageSecurityModeNone,
            client_nonce: UaByteString::null(),
            requested_lifetime: DEFAULT_LIFETIME,
        };
        let body = encode_service(446, &request, body_budget(limits, overhead))?;
        let open = Frame::single(Body::Open(OpenChunk {
            channel_id: 0,
            security,
            sequence: SequenceHeader {
                sequence_number: 1,
                request_id: 1,
            },
            body,
        }));
        within(
            clock.as_ref(),
            deadline,
            cancel,
            framed.send_frame(&open, deadline),
        )
        .await?;
        let reply = within(
            clock.as_ref(),
            deadline,
            cancel,
            framed.next_frame(deadline),
        )
        .await?;
        check_frame(&reply)?;
        let Body::Open(chunk) = reply.body else {
            return Err(error(
                ErrorKind::MalformedData,
                "expected OpenSecureChannel",
            ));
        };
        if chunk.security != AsymmetricHeader::none()
            || chunk.sequence.request_id != 1
            || chunk.sequence.sequence_number == 0
        {
            return Err(error(
                ErrorKind::MalformedData,
                "unexpected OpenSecureChannel header",
            ));
        }
        if limits.max_message_size != 0
            && chunk.body.len() > usize::try_from(limits.max_message_size).unwrap_or(usize::MAX)
        {
            return Err(error(
                ErrorKind::ResourceLimit,
                "response exceeds negotiated message size",
            ));
        }
        let Service::OpenSecureChannelResponse {
            response_header,
            server_protocol_version,
            security_token,
            server_nonce,
        } = decode_service(&chunk.body)?
        else {
            return Err(error(
                ErrorKind::MalformedData,
                "unexpected channel service response",
            ));
        };
        if response_header.service_result.0 & 0xC000_0000 != 0 {
            return Err(status_error(response_header.service_result));
        }
        if response_header.request_handle != 1
            || server_protocol_version != 0
            || chunk.channel_id == 0
            || security_token.channel_id != chunk.channel_id
            || security_token.token_id == 0
            || security_token.revised_lifetime == 0
        {
            return Err(error(
                ErrorKind::MalformedData,
                "invalid channel response parameters",
            ));
        }
        Ok(Self {
            framed,
            clock,
            limits,
            token: security_token,
            server_nonce,
            received_sequence: chunk.sequence.sequence_number,
            configured_mode: options.message_security,
        })
    }

    /// CLO is sent once; no response is awaited. Dropping the owner also closes its pipe.
    pub(super) async fn close(
        mut self,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<()> {
        let header = request_header(self.clock.as_ref(), &NodeId::numeric(0, 0), 2, deadline)?;
        let request = Service::CloseSecureChannelRequest {
            request_header: header,
        };
        let body = encode_service(452, &request, body_budget(self.limits, 24))?;
        let close = Frame::single(Body::Close(SecureChunk {
            channel_id: self.token.channel_id,
            token_id: self.token.token_id,
            sequence: SequenceHeader {
                sequence_number: 2,
                request_id: 2,
            },
            body,
        }));
        within(
            self.clock.as_ref(),
            deadline,
            cancel,
            self.framed.send_frame(&close, deadline),
        )
        .await?;
        within(
            self.clock.as_ref(),
            deadline,
            cancel,
            self.framed.shutdown(deadline),
        )
        .await
    }
}

fn proposed_limits(resources: ResourceLimits, receive: u32) -> Result<TransportLimits> {
    let cap = u32::try_from(resources.max_frame_bytes().get())
        .unwrap_or(u32::MAX)
        .min(0x7FFF_FFFF);
    let buffer = DEFAULT_BUFFER.min(cap);
    let receive = receive.min(cap);
    if buffer < MIN_BUFFER || receive < MIN_BUFFER {
        return Err(error(
            ErrorKind::InvalidConfig,
            "frame budget must allow an 8192-byte channel buffer",
        ));
    }
    Ok(TransportLimits {
        receive_buffer_size: receive,
        send_buffer_size: buffer,
        max_message_size: 2_097_152,
        max_chunk_count: 64,
    })
}

fn negotiate(client: TransportLimits, server: TransportLimits) -> Result<TransportLimits> {
    if server.receive_buffer_size < MIN_BUFFER || server.send_buffer_size < MIN_BUFFER {
        return Err(error(
            ErrorKind::MalformedData,
            "server channel buffers are below 8192 bytes",
        ));
    }
    Ok(TransportLimits {
        receive_buffer_size: client.receive_buffer_size.min(server.send_buffer_size),
        send_buffer_size: client.send_buffer_size.min(server.receive_buffer_size),
        max_message_size: smaller_limit(client.max_message_size, server.max_message_size),
        max_chunk_count: smaller_limit(client.max_chunk_count, server.max_chunk_count),
    })
}

fn smaller_limit(client: u32, server: u32) -> u32 {
    match (client, server) {
        (0, value) | (value, 0) => value,
        (client, server) => client.min(server),
    }
}

fn body_budget(limits: TransportLimits, overhead: usize) -> usize {
    let buffer = usize::try_from(limits.send_buffer_size)
        .unwrap_or(usize::MAX)
        .saturating_sub(overhead);
    if limits.max_message_size == 0 {
        buffer
    } else {
        buffer.min(usize::try_from(limits.max_message_size).unwrap_or(usize::MAX))
    }
}

fn check_frame(frame: &Frame) -> Result<()> {
    if let Body::Error(server) = &frame.body {
        return Err(status_error(server.error));
    }
    if frame.chunk != ChunkType::Final {
        return Err(error(
            ErrorKind::MalformedData,
            "handshake response is not a final chunk",
        ));
    }
    Ok(())
}

async fn within<T>(
    clock: &dyn Clock,
    deadline: Deadline,
    cancel: &CancellationToken,
    io: impl Future<Output = Result<T>>,
) -> Result<T> {
    let mut io = pin!(io);
    let mut cancelled = pin!(cancel.cancelled());
    let mut timer = pin!(clock.sleep_until(deadline.instant()));
    poll_fn(|cx| {
        let _cancelled = cancelled.as_mut().poll(cx);
        if cancel.is_cancelled() {
            return Poll::Ready(Err(error(ErrorKind::Cancelled, "handshake cancelled")));
        }
        if deadline.is_elapsed(clock.now()) {
            return Poll::Ready(Err(error(ErrorKind::Timeout, "handshake deadline elapsed")));
        }
        if let Poll::Ready(result) = timer.as_mut().poll(cx) {
            result?;
            return Poll::Ready(Err(error(
                ErrorKind::InvalidConfig,
                "clock woke before handshake deadline",
            )));
        }
        io.as_mut().poll(cx)
    })
    .await
}

fn error(kind: ErrorKind, message: &str) -> PlcError {
    PlcError::new(kind, format!("OPC UA channel: {message}"))
}

impl fmt::Debug for Channel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Channel")
            .field("limits", &self.limits)
            .field("configured_message_security", &self.configured_mode)
            .field("effective_message_security", &"NONE")
            .field("channel_id", &self.token.channel_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
