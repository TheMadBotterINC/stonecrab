use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::Clock;
use crate::ResourceLimits;
use crate::opcua::driver::OpcuaDriver;
use crate::opcua::frame::{Body, OpcuaFramer};
use crate::opcua::options::SecurityPolicy;
use crate::opcua::responder::{create_response, message, response_header};
use crate::opcua::service::decode_service;
use crate::testkit::{FakeClock, FakeTransport, Script, Step, block_on_with};
use crate::transport::{FramedTransport, Framer};

const URL: &str = "opc.tcp://fake:4840/path";

fn options(parameters: &[(&str, &str)]) -> OpcuaOptions {
    OpcuaOptions::from_parameters(
        URL.to_owned(),
        &parameters
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
fn none() -> OpcuaOptions {
    options(&[("security-policy", "NONE")])
}
fn endpoint() -> EndpointDescription {
    let Service::CreateSessionResponse {
        server_endpoints, ..
    } = decode_service(&create_response(1, 1000.0, 0)).unwrap()
    else {
        panic!("fixture")
    };
    server_endpoints.unwrap().remove(0)
}
fn token(endpoint: &mut EndpointDescription) -> &mut UserTokenPolicy {
    endpoint
        .user_identity_tokens
        .as_mut()
        .unwrap()
        .first_mut()
        .unwrap()
}
fn chosen(endpoints: &[EndpointDescription], options: &OpcuaOptions) -> String {
    select(endpoints, options, UserTokenType::UserTokenTypeAnonymous)
        .unwrap()
        .1
        .policy_id
        .text()
        .unwrap()
        .unwrap()
        .to_owned()
}
fn refused(endpoints: &[EndpointDescription], options: &OpcuaOptions) -> PlcError {
    select(endpoints, options, UserTokenType::UserTokenTypeAnonymous).unwrap_err()
}

#[test]
fn opcua_an_endpoint_matches_on_url_policy_uri_and_mode() {
    let good = endpoint();
    assert_eq!(
        chosen(std::slice::from_ref(&good), &none()),
        "anonymous-policy"
    );
    let mut bad = good;
    bad.transport_profile_uri = UaString::new("wrong");
    assert_eq!(refused(&[bad], &none()).kind(), ErrorKind::Authentication);
    for (policy, mode) in [
        (SecurityPolicy::None, MessageSecurity::None),
        (SecurityPolicy::Basic256Sha256, MessageSecurity::None),
        (SecurityPolicy::Basic256Sha256, MessageSecurity::Sign),
        (SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt),
        (
            SecurityPolicy::Aes128Sha256RsaOaep,
            MessageSecurity::SignEncrypt,
        ),
        (SecurityPolicy::Aes256Sha256RsaPss, MessageSecurity::Sign),
    ] {
        let mut options = none();
        options.security_policy = policy;
        options.message_security = mode;
        let mut good = endpoint();
        good.security_policy_uri = UaString::new(policy.uri());
        good.security_mode = match mode {
            MessageSecurity::None => MessageSecurityMode::MessageSecurityModeNone,
            MessageSecurity::Sign => MessageSecurityMode::MessageSecurityModeSign,
            MessageSecurity::SignEncrypt => MessageSecurityMode::MessageSecurityModeSignAndEncrypt,
        };
        assert_eq!(
            chosen(std::slice::from_ref(&good), &options),
            "anonymous-policy"
        );
        good.security_mode = MessageSecurityMode::MessageSecurityModeInvalid;
        assert_eq!(refused(&[good], &options).kind(), ErrorKind::Authentication);
    }
}

#[test]
fn opcua_an_endpoint_with_a_different_policy_uri_is_not_selected() {
    let mut bad = endpoint();
    bad.security_policy_uri = UaString::new(SecurityPolicy::Basic256Sha256.uri());
    assert_eq!(refused(&[bad], &none()).kind(), ErrorKind::Authentication);
}

#[test]
fn opcua_port_4840_does_not_match_48400() {
    for url in [
        "opc.tcp://fake:48400/path",
        "opc.tcp://fake:4840/paths",
        "opc.tcp://fake2:4840/path",
        "opc.tcp://fake:4840/Path",
        "opc.tcp://fake:4840",
        "opc.tcp://fake:4840/path/extra",
    ] {
        let mut bad = endpoint();
        bad.endpoint_url = UaString::new(url);
        assert_eq!(
            refused(&[bad], &none()).kind(),
            ErrorKind::Authentication,
            "{url}"
        );
    }
}

#[test]
fn opcua_endpoint_hosts_are_case_insensitive_and_ipv6_is_normalized() {
    let mut good = endpoint();
    good.endpoint_url = UaString::new("OPC.TCP://FAKE:04840/path");
    assert_eq!(
        chosen(std::slice::from_ref(&good), &none()),
        "anonymous-policy"
    );
    let mut options = none();
    options.endpoint_url = "opc.tcp://[::1]:4840/path".to_owned();
    good.endpoint_url = UaString::new("opc.tcp://[0:0:0:0:0:0:0:1]:4840/path");
    assert_eq!(
        chosen(std::slice::from_ref(&good), &options),
        "anonymous-policy"
    );
    good.endpoint_url = UaString::new("opc.tcp://[::2]:4840/path");
    assert_eq!(refused(&[good], &options).kind(), ErrorKind::Authentication);
}

// covers: OPCUA-CFG-ENDPOINT-HOST
#[test]
fn opcua_endpoint_host_overrides_the_host_used_to_match_endpoints() {
    let options = options(&[("security-policy", "NONE"), ("endpoint-host", "internal")]);
    let mut good = endpoint();
    good.endpoint_url = UaString::new("opc.tcp://internal:4840/path");
    assert_eq!(
        chosen(std::slice::from_ref(&good), &options),
        "anonymous-policy"
    );
    assert_eq!(
        chosen(&[endpoint()], &options),
        "anonymous-policy",
        "original remains accepted"
    );
    good.endpoint_url = UaString::new("opc.tcp://internal:4841/path");
    assert_eq!(refused(&[good], &options).kind(), ErrorKind::Authentication);
}

// covers: OPCUA-CFG-ENDPOINT-PORT
#[test]
fn opcua_endpoint_port_overrides_the_port_used_to_match_endpoints() {
    let options = options(&[("security-policy", "NONE"), ("endpoint-port", "4841")]);
    let mut good = endpoint();
    good.endpoint_url = UaString::new("opc.tcp://fake:4841/path");
    assert_eq!(
        chosen(std::slice::from_ref(&good), &options),
        "anonymous-policy"
    );
    good.endpoint_url = UaString::new("opc.tcp://internal:4841/path");
    assert_eq!(refused(&[good], &options).kind(), ErrorKind::Authentication);
}

#[test]
fn opcua_endpoint_host_and_port_override_the_url_match() {
    let options = options(&[
        ("security-policy", "NONE"),
        ("endpoint-host", "internal"),
        ("endpoint-port", "4841"),
    ]);
    let mut good = endpoint();
    good.endpoint_url = UaString::new("opc.tcp://internal:4841/path");
    assert_eq!(
        chosen(std::slice::from_ref(&good), &options),
        "anonymous-policy"
    );
    for url in [
        "opc.tcp://fake:4841/path",
        "opc.tcp://internal:4840/path",
        "opc.tcp://internal:4841/other",
    ] {
        good.endpoint_url = UaString::new(url);
        assert_eq!(
            refused(std::slice::from_ref(&good), &options).kind(),
            ErrorKind::Authentication
        );
    }
}

// Adapted from pinned PLC4X EndpointSelectionTest (Apache-2.0); see NOTICE.
#[test]
fn opcua_the_strongest_security_level_wins() {
    let mut low = endpoint();
    low.security_level = 1;
    token(&mut low).policy_id = UaString::new("low");
    let mut high = endpoint();
    high.security_level = 30;
    token(&mut high).policy_id = UaString::new("high");
    assert_eq!(chosen(&[low.clone(), high.clone()], &none()), "high");
    assert_eq!(chosen(&[high, low], &none()), "high");
}

#[test]
fn opcua_ties_prefer_a_token_protecting_policy() {
    let mut plain = endpoint();
    token(&mut plain).policy_id = UaString::new("plain");
    token(&mut plain).security_policy_uri = UaString::new(SecurityPolicy::None.uri());
    let mut protected = endpoint();
    token(&mut protected).policy_id = UaString::new("protected");
    token(&mut protected).security_policy_uri = UaString::new(SecurityPolicy::Basic256Sha256.uri());
    assert_eq!(
        chosen(&[plain.clone(), protected.clone()], &none()),
        "protected"
    );
    assert_eq!(chosen(&[protected, plain.clone()], &none()), "protected");
    let mut later = plain.clone();
    token(&mut later).policy_id = UaString::new("later");
    assert_eq!(chosen(&[plain, later], &none()), "later");
}

#[test]
fn opcua_null_and_empty_token_policies_inherit_the_channel() {
    for inherited in [UaString::null(), UaString::new("")] {
        let mut e = endpoint();
        token(&mut e).security_policy_uri = inherited;
        assert!(!protects_token(
            &e,
            e.user_identity_tokens.as_ref().unwrap().first().unwrap()
        ));
        e.security_policy_uri = UaString::new(SecurityPolicy::Basic256Sha256.uri());
        assert!(protects_token(
            &e,
            e.user_identity_tokens.as_ref().unwrap().first().unwrap()
        ));
    }
    let mut e = endpoint();
    e.security_policy_uri = UaString::null();
    assert!(protects_token(
        &e,
        e.user_identity_tokens.as_ref().unwrap().first().unwrap()
    ));
}

// covers: OPCUA-AUTH-ANONYMOUS
#[test]
fn opcua_anonymous_is_refused_when_the_endpoint_offers_no_anonymous_policy() {
    for kind in [
        UserTokenType::UserTokenTypeUserName,
        UserTokenType::UserTokenTypeCertificate,
        UserTokenType::UserTokenTypeIssuedToken,
    ] {
        let mut e = endpoint();
        token(&mut e).token_type = kind;
        assert_eq!(refused(&[e], &none()).kind(), ErrorKind::Authentication);
    }
}

#[test]
fn opcua_certificate_credentials_need_a_certificate_token_policy() {
    for kind in [
        UserTokenType::UserTokenTypeAnonymous,
        UserTokenType::UserTokenTypeUserName,
        UserTokenType::UserTokenTypeCertificate,
    ] {
        let mut e = endpoint();
        token(&mut e).token_type = kind;
        assert_eq!(
            select(&[e], &none(), UserTokenType::UserTokenTypeCertificate).is_ok(),
            kind == UserTokenType::UserTokenTypeCertificate
        );
    }
}

#[test]
fn opcua_selection_refuses_missing_endpoints_tokens_or_policy_ids() {
    assert_eq!(refused(&[], &none()).kind(), ErrorKind::Authentication);
    let mut e = endpoint();
    e.user_identity_tokens = None;
    assert_eq!(
        refused(std::slice::from_ref(&e), &none()).kind(),
        ErrorKind::Authentication
    );
    e.user_identity_tokens = Some(vec![]);
    assert_eq!(refused(&[e], &none()).kind(), ErrorKind::Authentication);
    for id in [UaString::null(), UaString::new("")] {
        let mut e = endpoint();
        token(&mut e).policy_id = id;
        assert_eq!(refused(&[e], &none()).kind(), ErrorKind::Authentication);
    }
}

#[test]
fn opcua_a_failed_endpoint_selection_lists_the_advertised_endpoints_and_the_two_options() {
    let mut e = endpoint();
    e.endpoint_url = UaString::new("opc.tcp://internal:4840/path");
    let error = refused(&[e], &none());
    let text = error.to_string();
    assert!(text.contains("opc.tcp://internal:4840/path"));
    assert!(text.contains("endpoint-host"));
    assert!(text.contains("endpoint-port"));
}

#[test]
fn opcua_endpoint_diagnostics_are_bounded_and_never_echo_credentials() {
    for url in [
        UaString::null(),
        UaString::from_bytes(vec![255]),
        UaString::new("garbage"),
        UaString::new("http://fake:4840/path"),
        UaString::new("opc.tcp://secret:password@fake:4840/path"),
        UaString::new("opc.tcp://fake:4840/path?password=secret"),
        UaString::new("opc.tcp://fake:4840/path#secret"),
        UaString::new("opc.tcp://fake:4840/\nsecret"),
        UaString::new("opc.tcp://fake:4840/\0secret"),
        UaString::new("opc.tcp://fake:4840/path@secret"),
        UaString::new("opc.tcp://fake:4840/ secret"),
        UaString::new("opc.tcp://fake:0/path"),
        UaString::new(format!("opc.tcp://fake:4840/{}", "x".repeat(4096))),
    ] {
        let mut e = endpoint();
        e.endpoint_url = url;
        assert_eq!(display_url(&e.endpoint_url), "<invalid endpoint URL>");
        let text = refused(&[e], &none()).to_string();
        assert!(!text.contains("secret"));
        assert!(!text.contains("password"));
    }
    let url = UaString::new(format!("{URL}{}", "é".repeat(300)));
    let shown = display_url(&url);
    assert_eq!(shown.chars().count(), 259);
    assert!(shown.ends_with("..."));
    let mut endpoints = vec![endpoint(); 17];
    endpoints.last_mut().unwrap().endpoint_url = UaString::new("opc.tcp://omitted:4840/path");
    let text = refused(&endpoints, &options(&[])).to_string();
    assert_eq!(text.matches(URL).count(), 16);
    assert!(!text.contains("omitted"));
    let mut opts = none();
    opts.endpoint_url = "invalid".to_owned();
    assert_eq!(
        refused(&[endpoint()], &opts).kind(),
        ErrorKind::Authentication
    );
}

fn response(handle: u32, status: u32, endpoints: bool) -> Vec<u8> {
    [
        vec![1, 0, 0xAF, 1],
        response_header(handle, status),
        if endpoints { vec![0; 4] } else { vec![255; 4] },
    ]
    .concat()
}
fn exchange_response(reply: Vec<u8>) -> Result<Vec<EndpointDescription>> {
    let clock = FakeClock::default();
    let limits = ResourceLimits::builder().build()?;
    let transport = FakeTransport::new(
        Script::strict(vec![Step::respond(move |bytes| {
            let frame = OpcuaFramer::new(65535, limits)
                .decode(&mut bytes::BytesMut::from(bytes))
                .unwrap()
                .unwrap();
            let Body::Message(chunk) = frame.body else {
                panic!("request")
            };
            let Service::GetEndpointsRequest {
                request_header,
                endpoint_url,
                locale_ids,
                profile_uris,
            } = decode_service(&chunk.body).unwrap()
            else {
                panic!("GetEndpoints request")
            };
            assert_eq!(request_header.authentication_token, NodeId::numeric(0, 0));
            assert_eq!(endpoint_url, UaString::new(URL));
            assert_eq!(locale_ids, None);
            assert_eq!(profile_uris, None);
            Some(message(1, 1, 17, 29, &reply))
        })]),
        clock.clone(),
        limits,
    );
    let lane = Lane::new(
        FramedTransport::new(Box::new(transport), OpcuaFramer::new(65535, limits)),
        Arc::new(clock.clone()),
        Duration::from_secs(30),
        17,
        29,
    );
    block_on_with(
        &clock,
        get_endpoints(
            &lane,
            &none(),
            Deadline::after(clock.now(), Duration::from_secs(5))?,
            &CancellationToken::new(),
        ),
    )
}

#[test]
fn opcua_get_endpoints_checks_service_type_status_and_handle() {
    for empty in [true, false] {
        assert_eq!(exchange_response(response(1, 0, empty)).unwrap(), []);
    }
    assert_eq!(
        exchange_response(response(1, 0x8001_0000, true))
            .unwrap_err()
            .kind(),
        ErrorKind::DeviceException
    );
    assert_eq!(
        exchange_response(response(1, 0x4000_0000, true))
            .unwrap_err()
            .kind(),
        ErrorKind::DeviceException
    );
    assert_eq!(
        exchange_response(response(2, 0, true)).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
    assert_eq!(
        exchange_response(create_response(1, 1000.0, 0))
            .unwrap_err()
            .kind(),
        ErrorKind::MalformedData
    );
    assert_eq!(
        exchange_response(vec![0, 0]).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
}

#[tokio::test]
#[expect(
    clippy::disallowed_macros,
    reason = "ignored interop records advertised policies without credentials"
)]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_get_endpoints_lists_the_independent_servers_policies() {
    use crate::opcua::channel::Channel;
    use crate::transport::{SystemClock, TcpFactory, TransportFactory};
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("interop endpoint");
    let config = OpcuaDriver::configuration(&uri).unwrap();
    let options = config.driver_options_as::<OpcuaOptions>().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
    let deadline = Deadline::after(clock.now(), Duration::from_secs(5)).unwrap();
    let cancel = CancellationToken::new();
    let transport = TcpFactory::default()
        .open(config.endpoint(), *config.limits(), deadline)
        .await
        .unwrap();
    let channel = Channel::open_none(transport, options, clock, deadline, &cancel)
        .await
        .unwrap();
    let lane = Lane::from_channel(channel, config.timeouts().request);
    let endpoints = get_endpoints(&lane, options, deadline, &cancel)
        .await
        .unwrap();
    assert_eq!(endpoints.len(), 7);
    for policy in [
        SecurityPolicy::None,
        SecurityPolicy::Basic256Sha256,
        SecurityPolicy::Aes128Sha256RsaOaep,
        SecurityPolicy::Aes256Sha256RsaPss,
    ] {
        let modes: &[MessageSecurityMode] = if policy.is_secure() {
            &[
                MessageSecurityMode::MessageSecurityModeSign,
                MessageSecurityMode::MessageSecurityModeSignAndEncrypt,
            ]
        } else {
            &[MessageSecurityMode::MessageSecurityModeNone]
        };
        for mode in modes {
            let matching: Vec<_> = endpoints
                .iter()
                .filter(|endpoint| {
                    endpoint.security_policy_uri == UaString::new(policy.uri())
                        && endpoint.security_mode == *mode
                })
                .collect();
            assert_eq!(matching.len(), 1, "{policy:?} {mode:?}");
            let endpoint = matching.first().unwrap();
            assert_eq!(endpoint.transport_profile_uri, UaString::new(TCP_PROFILE));
            let tokens = endpoint.user_identity_tokens.as_ref().unwrap();
            for kind in [
                UserTokenType::UserTokenTypeAnonymous,
                UserTokenType::UserTokenTypeUserName,
                UserTokenType::UserTokenTypeCertificate,
            ] {
                assert!(tokens.iter().any(|token| token.token_type == kind));
            }
            assert!(
                endpoint
                    .server_certificate
                    .as_bytes()
                    .is_some_and(|bytes| !bytes.is_empty())
            );
        }
    }
    println!(
        "GetEndpoints on an explicit None probe: seven endpoints, exact four policy URIs/modes and anonymous/username/certificate policies verified"
    );
    lane.close(deadline, &cancel).await.unwrap();
}

#[test]
fn opcua_get_endpoints_retains_the_advertised_descriptions() {
    let mut body: Vec<u8> =
        include_str!("../../../../../fixtures/opcua/services/GetEndpointsResponse.hex")
            .split_whitespace()
            .map(|byte| u8::from_str_radix(byte, 16).unwrap())
            .collect();
    body.get_mut(8..12)
        .unwrap()
        .copy_from_slice(&1_u32.to_le_bytes());
    let endpoints = exchange_response([vec![1, 0, 0xAF, 1], body].concat()).unwrap();
    assert_eq!(endpoints.len(), 1);
    let endpoint = endpoints.first().unwrap();
    assert_eq!(
        endpoint.security_mode,
        MessageSecurityMode::MessageSecurityModeNone
    );
    assert_eq!(endpoint.endpoint_url, UaString::null());
}
