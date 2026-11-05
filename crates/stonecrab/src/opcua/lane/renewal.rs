//! Serialized OPN renewal on the same correlation and transport lane.

//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-RENEWAL
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-RENEWAL-FAILURE

use std::time::Duration;

use super::{Lane, Request, State, error, missing_request};
use crate::opcua::channel::Lifetime;
use crate::opcua::channel::Renewal;
use crate::opcua::codec::{NodeId, UaByteString};
use crate::opcua::frame::{AsymmetricHeader, OpenChunk};
use crate::opcua::frame::{Body, ChunkType, Frame, SequenceHeader};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::security_token_request_type::SecurityTokenRequestType;
use crate::opcua::options::MessageSecurity;
use crate::opcua::service::{decode_service, encode_service, request_header, status_error};
use crate::opcua::wire::Wire;
use crate::{CancellationToken, Deadline, ErrorKind, PlcError, Result};

impl Lane {
    pub(in crate::opcua) fn expired(&self, session_timeout: Duration) -> bool {
        let state = self.lock();
        let now = self.clock.now();
        state
            .renewal
            .as_ref()
            .is_some_and(|renewal| renewal.lifetime.expired(now))
            || now.saturating_duration_since(state.last_activity) >= session_timeout
    }

    pub(super) async fn prepare(
        &self,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<()> {
        if !self
            .lock()
            .renewal
            .as_ref()
            .is_some_and(|renewal| renewal.lifetime.due(self.clock.now()))
        {
            return Ok(());
        }
        let _guard = self
            .until(deadline, cancel, async {
                Ok(self.renewal_lock.lock().await)
            })
            .await?;
        let Some(renewal) = self
            .lock()
            .renewal
            .clone()
            .filter(|renewal| renewal.lifetime.due(self.clock.now()))
        else {
            return Ok(());
        };
        let request = self.reserve(Some((UaByteString::null(), self.clock.now())))?;
        let mut attempt = Attempt {
            lane: self,
            finished: false,
        };
        let result = self.renew(&request, renewal, deadline, cancel).await;
        if let Err(problem) = &result {
            self.fail(PlcError::new(problem.kind(), problem.to_string()));
        }
        attempt.finished = true;
        result
    }

    async fn renew(
        &self,
        request: &Request<'_>,
        renewal: Renewal,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<()> {
        self.check(deadline, cancel)?;
        if renewal.lifetime.expired(self.clock.now()) {
            return Err(error(
                ErrorKind::Disconnected,
                "channel token expired before renewal",
            ));
        }
        let mut lease = self.writer(request.id, deadline, cancel).await?;
        let nonce = if renewal.security.is_some() {
            let nonce = UaByteString::new(lease.framed()?.nonce()?.to_vec());
            if nonce == renewal.client_nonce
                || !nonce
                    .as_bytes()
                    .unwrap_or_default()
                    .iter()
                    .any(|byte| *byte != 0)
            {
                return Err(error(
                    ErrorKind::Authentication,
                    "repeated channel client nonce",
                ));
            }
            nonce
        } else {
            UaByteString::null()
        };
        let issued = self.clock.now();
        self.lock()
            .pending
            .get_mut(&request.id)
            .ok_or_else(missing_request)?
            .open = Some((nonce.clone(), issued));
        let header = request_header(
            self.clock.as_ref(),
            &NodeId::numeric(0, 0),
            request.id,
            deadline,
        )?;
        let body = encode_service(
            446,
            &Service::OpenSecureChannelRequest {
                request_header: header,
                client_protocol_version: 0,
                request_type: SecurityTokenRequestType::SecurityTokenRequestTypeRenew,
                security_mode: match renewal.security.as_ref().map(|_| renewal.mode) {
                    None | Some(MessageSecurity::None) => {
                        MessageSecurityMode::MessageSecurityModeNone
                    }
                    Some(MessageSecurity::Sign) => MessageSecurityMode::MessageSecurityModeSign,
                    Some(MessageSecurity::SignEncrypt) => {
                        MessageSecurityMode::MessageSecurityModeSignAndEncrypt
                    }
                },
                client_nonce: nonce,
                requested_lifetime: renewal.requested,
            },
            lease
                .framed()?
                .max_open_body(self.max_frame)?
                .min(self.message_limits.bytes),
        )?;
        let frame = Frame::single(Body::Open(OpenChunk {
            channel_id: self.channel_id,
            security: renewal
                .security
                .as_ref()
                .map_or_else(AsymmetricHeader::none, |security| {
                    security.asymmetric.header()
                }),
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
        drop(lease);
        self.receive(request.id, deadline, cancel).await?;
        Ok(())
    }

    pub(super) fn accept_open(&self, frame: &Frame, wire: &mut Wire) -> Result<()> {
        let Body::Open(chunk) = &frame.body else {
            return Ok(());
        };
        let (mut renewal, nonce, issued) = {
            let state = self.lock();
            let renewal = state.renewal.clone();
            let pending = state
                .pending
                .get(&chunk.sequence.request_id)
                .and_then(|pending| pending.open.clone());
            drop(state);
            match (renewal, pending) {
                (Some(renewal), Some((nonce, issued))) => (renewal, nonce, issued),
                _ => {
                    return Err(error(
                        ErrorKind::MalformedData,
                        "unsolicited channel response",
                    ));
                }
            }
        };
        if frame.chunk != ChunkType::Final || chunk.channel_id != self.channel_id {
            return Err(error(ErrorKind::MalformedData, "invalid renewal header"));
        }
        if chunk.body.len() > self.message_limits.bytes {
            return Err(error(
                ErrorKind::ResourceLimit,
                "renewal response exceeds the message budget",
            ));
        }
        if let Some(security) = &renewal.security {
            security.verify(
                chunk
                    .security
                    .sender_certificate
                    .as_bytes()
                    .unwrap_or_default(),
                self.wall_now(),
            )?;
        } else if chunk.security != AsymmetricHeader::none() {
            return Err(error(ErrorKind::MalformedData, "foreign renewal policy"));
        }
        let response = decode_service(&chunk.body)?;
        let Service::OpenSecureChannelResponse {
            response_header,
            server_protocol_version,
            security_token,
            server_nonce,
        } = response
        else {
            return Err(error(ErrorKind::MalformedData, "expected renewal response"));
        };
        if response_header.service_result.0 & 0xC000_0000 != 0 {
            return Err(status_error(response_header.service_result));
        }
        if response_header.request_handle != chunk.sequence.request_id
            || server_protocol_version != 0
            || security_token.channel_id != self.channel_id
            || security_token.token_id == 0
            || security_token.token_id == renewal.token.token_id
            || renewal
                .previous
                .is_some_and(|(id, _)| id == security_token.token_id)
        {
            return Err(error(
                ErrorKind::MalformedData,
                "invalid renewal parameters",
            ));
        }
        if renewal.security.is_some() {
            server_nonce
                .as_bytes()
                .filter(|bytes| {
                    bytes.len() == 32
                        && bytes.iter().any(|byte| *byte != 0)
                        && Some(*bytes) != nonce.as_bytes()
                        && server_nonce != renewal.server_nonce
                })
                .ok_or_else(|| error(ErrorKind::Authentication, "invalid renewed server nonce"))?;
        }
        wire.install_token(
            security_token.token_id,
            nonce.as_bytes().unwrap_or_default(),
            server_nonce.as_bytes().unwrap_or_default(),
        )?;
        renewal.previous = Some((renewal.token.token_id, renewal.lifetime));
        renewal.lifetime = Lifetime::new(
            renewal.requested,
            security_token.revised_lifetime,
            renewal.minimum,
            issued,
        );
        renewal.token = security_token;
        renewal.server_nonce = server_nonce;
        renewal.client_nonce = nonce;
        let mut state = self.lock();
        state.token_id = renewal.token.token_id;
        state.renewal = Some(renewal);
        drop(state);
        Ok(())
    }

    pub(super) fn accept_token(
        &self,
        state: &mut State,
        channel_id: u32,
        token_id: u32,
    ) -> Result<()> {
        if channel_id != self.channel_id {
            return Err(error(
                ErrorKind::MalformedData,
                "foreign channel or token id",
            ));
        }
        if let Some(renewal) = &mut state.renewal {
            let now = self.clock.now();
            if token_id == renewal.token.token_id && renewal.lifetime.accepts_previous(now) {
                renewal.previous = None;
                return Ok(());
            }
            if renewal
                .previous
                .is_some_and(|(id, lifetime)| id == token_id && lifetime.accepts_previous(now))
            {
                return Ok(());
            }
        } else if token_id == state.token_id {
            return Ok(());
        }
        Err(error(
            ErrorKind::MalformedData,
            "foreign channel or token id",
        ))
    }

    pub(super) fn expire_keys(&self, wire: &mut Wire) {
        let mut state = self.lock();
        if let Some(renewal) = &mut state.renewal {
            if renewal
                .previous
                .is_some_and(|(_, lifetime)| !lifetime.accepts_previous(self.clock.now()))
            {
                renewal.previous = None;
            }
            if renewal.previous.is_none() {
                wire.retire_previous();
            }
        }
    }
}

struct Attempt<'a> {
    lane: &'a Lane,
    finished: bool,
}
impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        if !self.finished {
            self.lane.fail(error(
                ErrorKind::Disconnected,
                "channel renewal did not complete",
            ));
        }
    }
}
