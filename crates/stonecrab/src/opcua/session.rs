//! Anonymous session setup on an explicitly selected None channel.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-CREATE-SESSION
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-CLOSE-SESSION
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIMIT-CLOSE-HINT

use std::time::Duration;

use crate::opcua::codec::{
    ExpandedNodeId, ExtensionBody, ExtensionObject, LocalizedText, NodeId, UaByteString, UaString,
};
use crate::opcua::generated::application_description::ApplicationDescription;
use crate::opcua::generated::application_type::ApplicationType;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::response_header::ResponseHeader;
use crate::opcua::generated::signature_data::SignatureData;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::lane::Lane;
use crate::opcua::options::OpcuaOptions;
use crate::opcua::service::status_error;
use crate::{CancellationToken, Deadline, ErrorKind, PlcError, Result};

pub(super) const NONE_POLICY: &str = "http://opcfoundation.org/UA/SecurityPolicy#None";
const TCP_PROFILE: &str = "http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary";

pub(super) struct Session {
    pub(super) lane: Lane,
    authentication_token: NodeId,
    pub(super) revised_timeout: Duration,
}

impl Session {
    pub(super) async fn open(
        lane: Lane,
        options: &OpcuaOptions,
        nonce: [u8; 32],
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Self> {
        let reply = lane
            .exchange_until(
                &NodeId::numeric(0, 0),
                461,
                |header| Service::CreateSessionRequest {
                    request_header: header,
                    client_description: ApplicationDescription {
                        application_uri: UaString::new("urn:stonecrab:client"),
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
                    client_certificate: UaByteString::null(),
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
        let policy_id = server_endpoints
            .into_iter()
            .flatten()
            .filter(|endpoint| {
                endpoint.security_mode == MessageSecurityMode::MessageSecurityModeNone
                    && endpoint.security_policy_uri == UaString::new(NONE_POLICY)
                    && endpoint.transport_profile_uri == UaString::new(TCP_PROFILE)
            })
            .flat_map(|endpoint| endpoint.user_identity_tokens.into_iter().flatten())
            .find(|policy| {
                policy.token_type == UserTokenType::UserTokenTypeAnonymous
                    && policy.policy_id.as_bytes().is_some_and(|id| !id.is_empty())
            })
            .map(|policy| policy.policy_id)
            .ok_or_else(|| {
                PlcError::new(
                    ErrorKind::Authentication,
                    "OPC UA server advertises no compatible anonymous policy",
                )
            })?;
        let reply = lane
            .exchange_until(
                &authentication_token,
                467,
                |header| Service::ActivateSessionRequest {
                    request_header: header,
                    client_signature: empty_signature(),
                    client_software_certificates: None,
                    locale_ids: None,
                    user_identity_token: ExtensionObject {
                        type_id: ExpandedNodeId::local(NodeId::numeric(0, 321)),
                        body: ExtensionBody::Known(Box::new(Service::AnonymousIdentityToken {
                            policy_id,
                        })),
                    },
                    user_token_signature: empty_signature(),
                },
                deadline,
                cancel,
            )
            .await?;
        let Service::ActivateSessionResponse {
            response_header, ..
        } = reply
        else {
            return Err(malformed("expected ActivateSessionResponse"));
        };
        check_status(&response_header)?;
        Ok(Self {
            lane,
            authentication_token,
            revised_timeout,
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
