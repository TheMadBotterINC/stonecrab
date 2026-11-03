//! Discovery service building block and pure endpoint/token selection.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-ENDPOINT-MATCH
//! # Compatibility: docs/parity/opcua.md#OPCUA-ENDPOINT-TOKEN-COMPAT
//! # Compatibility: docs/parity/opcua.md#OPCUA-POLICY-ENDPOINT-URI
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-ENDPOINT-URL

use crate::opcua::codec::{NodeId, UaString};
use crate::opcua::generated::endpoint_description::EndpointDescription;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::user_token_policy::UserTokenPolicy;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::lane::Lane;
use crate::opcua::options::{MessageSecurity, OpcuaOptions, SecurityPolicy, authority_parts};
use crate::opcua::service::status_error;
use crate::{CancellationToken, Deadline, ErrorKind, PlcError, Result};

const TCP_PROFILE: &str = "http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary";

/// Runs on the caller's channel and connect deadline, never an automatic None fallback.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "secure-connect discovery wiring is S106")
)]
pub(super) async fn get_endpoints(
    lane: &Lane,
    options: &OpcuaOptions,
    deadline: Deadline,
    cancel: &CancellationToken,
) -> Result<Vec<EndpointDescription>> {
    let response = lane
        .exchange_until(
            &NodeId::numeric(0, 0),
            428,
            |request_header| Service::GetEndpointsRequest {
                request_header,
                endpoint_url: UaString::new(&options.endpoint_url),
                locale_ids: None,
                profile_uris: None,
            },
            deadline,
            cancel,
        )
        .await?;
    let Service::GetEndpointsResponse {
        response_header,
        endpoints,
    } = response
    else {
        return Err(PlcError::new(
            ErrorKind::MalformedData,
            "OPC UA expected GetEndpointsResponse",
        ));
    };
    if response_header.service_result.0 & 0xC000_0000 != 0 {
        return Err(status_error(response_header.service_result));
    }
    Ok(endpoints.unwrap_or_default())
}

/// The strongest compatible pair; no policy or mode downgrade and no DNS resolution.
pub(super) fn select<'a>(
    endpoints: &'a [EndpointDescription],
    options: &OpcuaOptions,
    token_type: UserTokenType,
) -> Result<(&'a EndpointDescription, &'a UserTokenPolicy)> {
    let expected_mode = if options.security_policy.is_secure() {
        match options.message_security {
            MessageSecurity::None => MessageSecurityMode::MessageSecurityModeNone,
            MessageSecurity::Sign => MessageSecurityMode::MessageSecurityModeSign,
            MessageSecurity::SignEncrypt => MessageSecurityMode::MessageSecurityModeSignAndEncrypt,
        }
    } else {
        MessageSecurityMode::MessageSecurityModeNone
    };
    let candidates = endpoints
        .iter()
        .filter(|endpoint| {
            endpoint.security_policy_uri == UaString::new(options.security_policy.uri())
                && endpoint.security_mode == expected_mode
                && endpoint.transport_profile_uri == UaString::new(TCP_PROFILE)
                && url_matches(&endpoint.endpoint_url, options)
        })
        .flat_map(|endpoint| {
            endpoint
                .user_identity_tokens
                .iter()
                .flatten()
                .filter(move |token| {
                    token.token_type == token_type
                        && token.policy_id.as_bytes().is_some_and(|id| !id.is_empty())
                })
                .map(move |token| (endpoint, token))
        });
    candidates.max_by_key(|(endpoint, token)| (endpoint.security_level, protects_token(endpoint, token)))
        .ok_or_else(|| {
            let advertised = endpoints.iter().take(16).map(|endpoint| display_url(&endpoint.endpoint_url)).collect::<Vec<_>>().join(", ");
            PlcError::new(ErrorKind::Authentication, format!(
                "OPC UA no compatible endpoint/token policy; advertised endpoints: [{advertised}]; check endpoint-host and endpoint-port"
            ))
        })
}

/// Null/empty token policy inherits the channel; this ranks claims, not trust.
fn protects_token(endpoint: &EndpointDescription, token: &UserTokenPolicy) -> bool {
    let policy = token
        .security_policy_uri
        .as_bytes()
        .filter(|uri| !uri.is_empty())
        .unwrap_or_else(|| endpoint.security_policy_uri.as_bytes().unwrap_or_default());
    policy != SecurityPolicy::None.uri().as_bytes()
}

/// Borrowed components after the same checked authority grammar used by configuration.
fn url_parts(url: &str) -> Option<(&str, u16, Option<&str>)> {
    if url
        .chars()
        .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '@' | '?' | '#'))
    {
        return None;
    }
    let (scheme, authority) = url.split_once("://")?;
    if !scheme.eq_ignore_ascii_case("opc.tcp") {
        return None;
    }
    authority_parts(authority).ok()
}

fn host_matches(left: &str, right: &str) -> bool {
    match (
        left.parse::<std::net::Ipv6Addr>(),
        right.parse::<std::net::Ipv6Addr>(),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.eq_ignore_ascii_case(right),
    }
}

fn url_matches(advertised: &UaString, options: &OpcuaOptions) -> bool {
    let Some((host, port, path)) = advertised
        .text()
        .and_then(core::result::Result::ok)
        .and_then(url_parts)
    else {
        return false;
    };
    let Some((original_host, original_port, original_path)) = url_parts(&options.endpoint_url)
    else {
        return false;
    };
    path == original_path
        && ((host_matches(host, original_host) && port == original_port)
            || (host_matches(
                host,
                options.endpoint_host.as_deref().unwrap_or(original_host),
            ) && port == options.endpoint_port.unwrap_or(original_port)))
}

/// Never repeat credentials/query/fragment/control text supplied by a peer.
pub(super) fn display_url(url: &UaString) -> String {
    let Some(text) = url
        .text()
        .and_then(core::result::Result::ok)
        .filter(|url| url_parts(url).is_some())
    else {
        return "<invalid endpoint URL>".to_owned();
    };
    let mut chars = text.chars();
    let mut shown: String = chars.by_ref().take(256).collect();
    if chars.next().is_some() {
        shown.push_str("...");
    }
    shown
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
