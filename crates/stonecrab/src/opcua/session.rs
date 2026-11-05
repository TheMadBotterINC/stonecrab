//! Session setup bound to the channel policy and provisioned server identity.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-MODE-NONE
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-CREATE-SESSION
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-CLOSE-SESSION
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIMIT-CLOSE-HINT

use std::sync::Mutex;
use std::time::Duration;

use crate::opcua::codec::{LocalizedText, NodeId, UaByteString, UaString};
use crate::opcua::generated::application_description::ApplicationDescription;
use crate::opcua::generated::application_type::ApplicationType;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::response_header::ResponseHeader;
use crate::opcua::generated::signature_data::SignatureData;
use crate::opcua::identity::{Identity, activation_error};
use crate::opcua::lane::Lane;
use crate::opcua::options::OpcuaOptions;
use crate::opcua::service::status_error;
use crate::{CancellationToken, Deadline, ErrorKind, PlcError, Result};

pub(super) struct Session {
    pub(super) lane: Lane,
    pub(super) authentication_token: NodeId,
    pub(super) revised_timeout: Duration,
    pub(super) selected_endpoint: String,
    pub(super) attributes: Mutex<Vec<(NodeId, crate::opcua::attributes::Attributes)>>,
    pub(super) continuations: crate::opcua::browse::Continuations,
}

impl Session {
    pub(super) async fn open(
        lane: Lane,
        options: &OpcuaOptions,
        identity: Identity<'_>,
        nonce: [u8; 32],
        deadline: Deadline,
        cancel: &CancellationToken,
        security: Option<&crate::opcua::crypto::Security>,
    ) -> Result<Self> {
        if let Some(security) = security
            && options.discovery
        {
            let endpoints =
                crate::opcua::endpoint::get_endpoints(&lane, options, deadline, cancel).await?;
            let (endpoint, _token) =
                crate::opcua::endpoint::select(&endpoints, options, identity.token_type())?;
            security.verify(
                endpoint.server_certificate.as_bytes().unwrap_or_default(),
                lane.wall_now(),
            )?;
        }
        let reply = lane
            .exchange_until(
                &NodeId::numeric(0, 0),
                461,
                |header| Service::CreateSessionRequest {
                    request_header: header,
                    client_description: ApplicationDescription {
                        application_uri: UaString::new(security.map_or(
                            "urn:stonecrab:client",
                            crate::opcua::crypto::Security::application_uri,
                        )),
                        product_uri: UaString::new("urn:stonecrab"),
                        application_name: LocalizedText {
                            locale: None,
                            text: Some(UaString::new("Stonecrab")),
                        },
                        application_type: ApplicationType::ApplicationTypeClient,
                        gateway_server_uri: UaString::null(),
                        discovery_profile_uri: UaString::null(),
                        discovery_urls: None,
                    },
                    server_uri: UaString::null(),
                    endpoint_url: UaString::new(&options.endpoint_url),
                    session_name: UaString::new("Stonecrab"),
                    client_nonce: UaByteString::new(nonce.to_vec()),
                    client_certificate: security.map_or_else(UaByteString::null, |security| {
                        UaByteString::new(security.certificate().to_vec())
                    }),
                    requested_session_timeout: options.session_timeout.as_secs_f64() * 1000.0,
                    max_response_message_size: 0,
                },
                deadline,
                cancel,
            )
            .await?;
        let Service::CreateSessionResponse {
            response_header,
            authentication_token,
            revised_session_timeout,
            server_endpoints,
            server_certificate,
            server_nonce,
            server_signature,
            ..
        } = reply
        else {
            return Err(malformed("expected CreateSessionResponse"));
        };
        check_status(&response_header)?;
        if authentication_token == NodeId::numeric(0, 0) {
            return Err(malformed("null authentication token"));
        }
        let revised_timeout = Duration::try_from_secs_f64(revised_session_timeout / 1000.0)
            .ok()
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| malformed("invalid revised session timeout"))?;
        let client_signature = if let Some(security) = security {
            security.check_server_signature(&server_signature, &nonce, lane.wall_now())?;
            let certificate = server_certificate.as_bytes().unwrap_or_default();
            security.verify(certificate, lane.wall_now())?;
            let server = secure_nonce(&server_nonce, &nonce)?;
            security.client_signature(certificate, server)?
        } else {
            empty_signature()
        };
        let endpoints = server_endpoints.unwrap_or_default();
        let (endpoint, policy) =
            crate::opcua::endpoint::select(&endpoints, options, identity.token_type())?;
        let selected_endpoint = crate::opcua::endpoint::display_url(&endpoint.endpoint_url);
        let effective = identity.policy(policy, &endpoint.security_policy_uri)?;
        if identity.requires_server(effective) {
            secure_nonce(&server_nonce, &nonce)?;
            if security.is_none() {
                let (peer, verifier) = crate::opcua::crypto::Verifier::prepare(options, effective)?;
                let peer = peer.ok_or_else(|| {
                    PlcError::new(
                        ErrorKind::InvalidConfig,
                        "OPC UA protected identity requires server-certificate-file",
                    )
                })?;
                verifier.verify_bound(
                    server_certificate.as_bytes().unwrap_or_default(),
                    &peer,
                    effective,
                    lane.wall_now(),
                )?;
            }
        }
        let (user_identity_token, user_token_signature) = identity.token(
            policy,
            effective,
            server_certificate.as_bytes().unwrap_or_default(),
            server_nonce.as_bytes().unwrap_or_default(),
            options,
        )?;
        let reply = lane
            .exchange_until(
                &authentication_token,
                467,
                |header| Service::ActivateSessionRequest {
                    request_header: header,
                    client_signature,
                    client_software_certificates: None,
                    locale_ids: None,
                    user_identity_token,
                    user_token_signature,
                },
                deadline,
                cancel,
            )
            .await
            .map_err(activation_error)?;
        let Service::ActivateSessionResponse {
            response_header,
            server_nonce: activated_nonce,
            ..
        } = reply
        else {
            return Err(malformed("expected ActivateSessionResponse"));
        };
        check_status(&response_header).map_err(activation_error)?;
        if security.is_some() || identity.requires_server(effective) {
            secure_nonce(
                &activated_nonce,
                server_nonce.as_bytes().unwrap_or_default(),
            )?;
        }
        Ok(Self {
            lane,
            authentication_token,
            revised_timeout,
            selected_endpoint,
            attributes: Mutex::new(Vec::new()),
            continuations: crate::opcua::browse::Continuations::default(),
        })
    }

    pub(super) async fn close(&self, deadline: Deadline, cancel: &CancellationToken) -> Result<()> {
        let result = self
            .lane
            .exchange_until(
                &self.authentication_token,
                473,
                |mut header| {
                    header.timeout_hint = 50000;
                    Service::CloseSessionRequest {
                        request_header: header,
                        delete_subscriptions: true,
                    }
                },
                deadline,
                cancel,
            )
            .await
            .and_then(|reply| {
                let Service::CloseSessionResponse { response_header } = reply else {
                    return Err(malformed("expected CloseSessionResponse"));
                };
                check_status(&response_header)
            });
        let close = self.lane.close(deadline, cancel).await;
        result.and(close)
    }
}

fn secure_nonce<'a>(nonce: &'a UaByteString, previous: &[u8]) -> Result<&'a [u8]> {
    nonce
        .as_bytes()
        .filter(|bytes| {
            (32..=128).contains(&bytes.len())
                && bytes.iter().any(|byte| *byte != 0)
                && *bytes != previous
        })
        .ok_or_else(|| {
            PlcError::new(
                ErrorKind::Authentication,
                "OPC UA invalid session server nonce",
            )
        })
}

const fn empty_signature() -> SignatureData {
    SignatureData {
        algorithm: UaString::null(),
        signature: UaByteString::null(),
    }
}

fn check_status(header: &ResponseHeader) -> Result<()> {
    if header.service_result.0 & 0xC000_0000 != 0 {
        return Err(status_error(header.service_result));
    }
    Ok(())
}

fn malformed(message: &str) -> PlcError {
    PlcError::new(
        ErrorKind::MalformedData,
        format!("OPC UA session: {message}"),
    )
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
