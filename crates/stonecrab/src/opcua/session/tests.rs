use std::sync::Arc;
use std::task::{Context, Waker};

use bytes::BytesMut;

use super::*;
use crate::opcua::OpcuaDriver;
use crate::opcua::channel::Channel;
use crate::opcua::codec::{ExpandedNodeId, ExtensionBody};
use crate::opcua::connection::OpcuaConnection;
use crate::opcua::frame::{Body, Frame, OpcuaFramer};
use crate::opcua::generated::message_security_mode::MessageSecurityMode;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::responder::{
    acknowledge, activate_response, close_response, create_response, message, open_response,
    service_fault,
};
use crate::opcua::service::decode_service;
use crate::testkit::{FakeClock, FakeTransport, FakeTransportFactory, Script, Step, block_on_with};
use crate::transport::Framer;
use crate::{
    AddressOptions, Clock, Connection, ConnectionConfig, ConnectionState, Driver, DriverRegistry,
    Endpoint, Operation, ReadRequest, ResourceLimits, WriteRequest,
};

const URI: &str = "opcua://fake:4840/path?security-policy=NONE";

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_basic256sha256_refuses_untrusted_expired_and_empty_trust_peers() {
    use crate::opcua::pki::Scratch;
    let base = std::env::var("STONECRAB_INTEROP_OPEN62541").unwrap();
    let base = base.split('?').next().unwrap();
    let empty = std::env::var("STONECRAB_INTEROP_OPEN62541_EMPTY_TRUST").unwrap();
    let expired = std::env::var("STONECRAB_INTEROP_OPEN62541_EXPIRED_SERVER").unwrap();
    let none = std::env::var("STONECRAB_INTEROP_OPEN62541_NONE_ONLY").unwrap();
    let pki =
        std::path::PathBuf::from(std::env::var("STONECRAB_INTEROP_OPEN62541_PKI_DIR").unwrap());
    let scratch = Scratch::new();
    for policy in [
        "Basic256Sha256",
        "Aes128_Sha256_RsaOaep",
        "Aes256_Sha256_RsaPss",
    ] {
        for (index, (endpoint, client, certificate, trust)) in [
            (base, "untrusted", "server", false),
            (base, "expired-client", "server", false),
            (empty.as_str(), "client", "server", false),
            (base, "client", "server", true),
            (base, "client", "untrusted", true),
            (expired.as_str(), "client", "expired-server", false),
            (none.as_str(), "client", "server", false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut pem = std::fs::read_to_string(pki.join(format!("{client}.cert.pem"))).unwrap();
            pem.push_str(&std::fs::read_to_string(pki.join(format!("{client}.key.pem"))).unwrap());
            let key = scratch.write("client.pem", &pem);
            let mut uri = format!(
                "{endpoint}?security-policy={policy}&message-security=SIGN_ENCRYPT&tls.keystore={}&server-certificate-file={}",
                key.display(),
                pki.join(format!("{certificate}.cert.der")).display()
            );
            if trust {
                uri = format!(
                    "{uri}&tls.trust-store={}",
                    pki.join(if index == 3 {
                        "untrusted.cert.pem"
                    } else {
                        "server.cert.pem"
                    })
                    .display()
                );
            }
            let driver = OpcuaDriver::new();
            let error = driver
                .connect(driver.parse_config(&uri).unwrap())
                .await
                .err()
                .unwrap();
            assert_eq!(
                error.kind(),
                if index < 3 || index == 6 {
                    ErrorKind::DeviceException
                } else {
                    ErrorKind::Trust
                },
                "case {index}: {error}"
            );
            if index < 3 || index == 6 {
                let mut source: &(dyn std::error::Error + 'static) = &error;
                let status = loop {
                    if let Some(status) =
                        source.downcast_ref::<crate::opcua::service::StatusFault>()
                    {
                        break status.code;
                    }
                    source = source.source().expect("native certificate status");
                };
                assert_eq!(
                    status,
                    match index {
                        1 => 0x8014_0000,
                        6 => 0x8055_0000,
                        _ => 0x8013_0000,
                    },
                    "case {index}"
                );
            }
        }
    }
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_basic256sha256_public_sessions_read_write_and_close_in_both_modes() {
    assert_secure_native_session("Basic256Sha256").await;
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_aes128_sha256_rsaoaep_public_sessions_read_write_and_close_in_both_modes() {
    assert_secure_native_session("Aes128_Sha256_RsaOaep").await;
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_aes256_sha256_rsapss_public_sessions_read_write_and_close_in_both_modes() {
    assert_secure_native_session("Aes256_Sha256_RsaPss").await;
}

#[expect(
    clippy::disallowed_macros,
    reason = "the ignored interop tests record non-secret secure session results"
)]
async fn assert_secure_native_session(policy: &str) {
    use crate::opcua::OpcuaAddress;
    use crate::opcua::pki::Scratch;
    use crate::{PlcValue, Tag, TagId};
    let base = std::env::var("STONECRAB_INTEROP_OPEN62541").unwrap();
    let base = base.split('?').next().unwrap();
    let pki =
        std::path::PathBuf::from(std::env::var("STONECRAB_INTEROP_OPEN62541_PKI_DIR").unwrap());
    let mut pem = std::fs::read_to_string(pki.join("client.cert.pem")).unwrap();
    pem.push_str(&std::fs::read_to_string(pki.join("client.key.pem")).unwrap());
    let scratch = Scratch::new();
    let key = scratch.write("client.pem", &pem);
    for mode in ["SIGN", "SIGN_ENCRYPT"] {
        for discovery in [false, true] {
            let driver = OpcuaDriver::new();
            let uri = format!(
                "{base}?security-policy={policy}&message-security={mode}&discovery={discovery}&tls.keystore={}&server-certificate-file={}&access=read-write&encoding.receive-buffer-size=8192&encoding.send-buffer-size=8192",
                key.display(),
                pki.join("server.cert.der").display()
            );
            let connection = driver
                .connect(driver.parse_config(&uri).unwrap())
                .await
                .unwrap();
            let tag = |name: &str, address: &str| {
                Tag::new(
                    TagId::new(name).unwrap(),
                    Arc::new(OpcuaAddress::parse(address).unwrap()),
                )
            };
            let request = ReadRequest::builder(connection.limits())
                .tag(tag("scalar", "ns=1;s=scalar/Int32"))
                .tag(tag("large", "ns=1;s=large/Int32_10000"))
                .build()
                .unwrap();
            let read = connection.read(request).await.unwrap();
            assert_eq!(read.iter_ok().count(), 2);
            assert_eq!(
                read.items().first().unwrap().outcome().as_ref().unwrap(),
                &PlcValue::I32(-123_456_789)
            );
            let PlcValue::List(values) = read.items().get(1).unwrap().outcome().as_ref().unwrap()
            else {
                panic!("array")
            };
            assert_eq!(values.len(), 10000);
            assert_eq!(values.last(), Some(&PlcValue::I32(9999)));
            let write = WriteRequest::builder(connection.limits())
                .item(
                    tag("setpoint", "ns=1;s=writable/Int32;DINT"),
                    PlcValue::I32(87654),
                )
                .build()
                .unwrap();
            assert_eq!(connection.write(write).await.unwrap().confirmed_count(), 1);
            let read = ReadRequest::builder(connection.limits())
                .tag(tag("setpoint", "ns=1;s=writable/Int32"))
                .build()
                .unwrap();
            assert_eq!(
                connection
                    .read(read)
                    .await
                    .unwrap()
                    .items()
                    .first()
                    .unwrap()
                    .outcome()
                    .as_ref()
                    .unwrap(),
                &PlcValue::I32(87654)
            );
            connection.close().await.unwrap();
            println!(
                "{policy} {mode}, discovery={discovery}: public connect, chunked Read, Write/read-back and close passed"
            );
        }
    }
}

fn frame(bytes: &[u8]) -> Frame {
    OpcuaFramer::new(65535, ResourceLimits::builder().build().unwrap())
        .decode(&mut BytesMut::from(bytes))
        .unwrap()
        .unwrap()
}

fn payload(bytes: &[u8]) -> Service {
    match frame(bytes).body {
        Body::Message(chunk) | Body::Close(chunk) => decode_service(&chunk.body).unwrap(),
        other @ (Body::Hello(_) | Body::Acknowledge(_) | Body::Error(_) | Body::Open(_)) => {
            panic!("not a service message: {other:?}")
        }
    }
}

#[expect(
    clippy::wildcard_enum_match_arm,
    clippy::unwrap_in_result,
    reason = "fixture only handles the three session requests"
)]
fn reply(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.starts_with(b"HEL") {
        return Some(acknowledge(65535, 65535, 0, 0));
    }
    if bytes.starts_with(b"OPN") {
        return Some(open_response(41, 1, 17, 29, 120_000, 0));
    }
    if !bytes.starts_with(b"MSG") {
        return None;
    }
    let Body::Message(chunk) = frame(bytes).body else {
        panic!("unexpected fixture variant")
    };
    let body = match decode_service(&chunk.body).unwrap() {
        Service::CreateSessionRequest { .. } => {
            create_response(chunk.sequence.request_id, 54321.0, 0)
        }
        Service::ActivateSessionRequest { .. } => activate_response(chunk.sequence.request_id, 0),
        Service::CloseSessionRequest { .. } => close_response(chunk.sequence.request_id, 0),
        _ => panic!("unexpected request"),
    };
    Some(message(
        chunk.sequence.sequence_number.checked_add(40).unwrap(),
        chunk.sequence.request_id,
        17,
        29,
        &body,
    ))
}

fn driver(script: Script) -> (OpcuaDriver, FakeClock, FakeTransportFactory) {
    let clock = FakeClock::default();
    let factory = FakeTransportFactory::new(clock.clone());
    factory.push_script(script);
    let driver = OpcuaDriver::new()
        .with_clock(Arc::new(clock.clone()))
        .with_transport_factory(Arc::new(factory.clone()));
    (driver, clock, factory)
}

fn connect(driver: &OpcuaDriver, clock: &FakeClock) -> Result<Box<dyn Connection>> {
    block_on_with(clock, driver.connect(Driver::parse_config(driver, URI)?))
}

#[test]
fn opcua_connect_runs_hello_open_create_activate_in_order_against_the_scripted_peer() {
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(reply)]));
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(driver)).unwrap();
    let connection = block_on_with(&clock, registry.connect(URI)).unwrap();
    assert_eq!(connection.state(), ConnectionState::Connected);
    assert_eq!(
        connection.capabilities(),
        crate::opcua::connection::capabilities()
    );
    assert_eq!(
        connection.limits(),
        ResourceLimits::builder().build().unwrap()
    );
    let probe = factory.probes().remove(0);
    let sent = probe.sent();
    assert_eq!(sent.len(), 4);
    assert!(sent.first().unwrap().starts_with(b"HELF"));
    assert!(sent.get(1).unwrap().starts_with(b"OPNF"));
    let Service::CreateSessionRequest {
        request_header,
        client_description,
        client_nonce,
        endpoint_url,
        requested_session_timeout,
        ..
    } = payload(sent.get(2).unwrap())
    else {
        panic!()
    };
    assert_eq!(client_nonce.as_bytes().unwrap().len(), 32);
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 0));
    assert_eq!(request_header.request_handle, 2);
    assert_eq!(
        client_description.application_type,
        ApplicationType::ApplicationTypeClient
    );
    assert_eq!(
        client_description.application_uri,
        UaString::new("urn:stonecrab:client")
    );
    assert_eq!(endpoint_url, UaString::new("opc.tcp://fake:4840/path"));
    assert_eq!(requested_session_timeout, 120_000.0);
    let Service::ActivateSessionRequest {
        request_header,
        user_identity_token,
        client_signature,
        user_token_signature,
        ..
    } = payload(sent.get(3).unwrap())
    else {
        panic!()
    };
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 77));
    assert_eq!(request_header.request_handle, 3);
    assert_eq!(client_signature, empty_signature());
    assert_eq!(user_token_signature, empty_signature());
    assert_eq!(
        user_identity_token.type_id,
        ExpandedNodeId::local(NodeId::numeric(0, 321))
    );
    assert_eq!(
        user_identity_token.body,
        ExtensionBody::Known(Box::new(Service::AnonymousIdentityToken {
            policy_id: UaString::new("anonymous-policy")
        }))
    );
    block_on_with(&clock, connection.close()).unwrap();
    assert!(probe.is_shut_down());
}

#[test]
fn opcua_close_sends_close_session_then_close_secure_channel() {
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(reply)]));
    let connection = connect(&driver, &clock).unwrap();
    let watch = connection.watch_state();
    block_on_with(&clock, connection.close()).unwrap();
    assert_eq!(connection.state(), ConnectionState::Closed);
    assert_eq!(
        block_on_with(&clock, watch.changed()).unwrap(),
        ConnectionState::Closed
    );
    block_on_with(&clock, connection.close()).unwrap();
    let sent = factory.probes().remove(0).sent();
    assert_eq!(sent.len(), 6);
    let Service::CloseSessionRequest {
        request_header,
        delete_subscriptions,
    } = payload(sent.get(4).unwrap())
    else {
        panic!()
    };
    assert!(delete_subscriptions);
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 77));
    assert_eq!(request_header.timeout_hint, 50000);
    let Body::Close(chunk) = frame(sent.get(5).unwrap()).body else {
        panic!()
    };
    assert_eq!(chunk.sequence.sequence_number, 5);
    assert_eq!(chunk.sequence.request_id, 5);
    let Service::CloseSecureChannelRequest { request_header } = payload(sent.get(5).unwrap())
    else {
        panic!()
    };
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 0));
}

#[test]
fn opcua_connect_is_connected_only_after_activate_session_succeeds() {
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"MSG")
            && matches!(payload(bytes), Service::ActivateSessionRequest { .. })
        {
            None
        } else {
            reply(bytes)
        }
    })]));
    let mut connecting = driver.connect(Driver::parse_config(&driver, URI).unwrap());
    assert!(
        connecting
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(factory.probes().remove(0).sent().len(), 4);
    drop(connecting);
    assert_eq!(clock.sleeper_count(), 0);
}

#[test]
fn opcua_connect_fails_when_activate_session_is_refused() {
    for fault in [false, true] {
        let (driver, clock, _) = driver(Script::strict(vec![Step::respond(move |bytes| {
            if bytes.starts_with(b"MSG")
                && matches!(payload(bytes), Service::ActivateSessionRequest { .. })
            {
                let body = if fault {
                    service_fault(3, 0x801F_0000)
                } else {
                    activate_response(3, 0x801F_0000)
                };
                Some(message(43, 3, 17, 29, &body))
            } else {
                reply(bytes)
            }
        })]));
        let problem = connect(&driver, &clock).err().unwrap();
        assert_eq!(problem.kind(), ErrorKind::Authentication);
        assert!(
            std::error::Error::source(&problem)
                .unwrap()
                .to_string()
                .contains("0x801F0000")
        );
    }
}

#[test]
fn opcua_close_with_a_silent_server_still_closes_the_transport_within_the_request_timeout() {
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"MSG")
            && matches!(payload(bytes), Service::CloseSessionRequest { .. })
        {
            None
        } else {
            reply(bytes)
        }
    })]));
    let connection = connect(&driver, &clock).unwrap();
    let start = clock.now();
    assert_eq!(
        block_on_with(&clock, connection.close())
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(clock.now().duration_since(start), Duration::from_secs(30));
    assert_eq!(connection.state(), ConnectionState::Closed);
    assert_eq!(factory.probes().remove(0).sent().len(), 5);
    block_on_with(&clock, connection.close()).unwrap();
}

#[test]
fn opcua_dropping_close_is_terminal_and_wakes_a_concurrent_close() {
    let (driver, clock, _) = driver(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"MSG")
            && matches!(payload(bytes), Service::CloseSessionRequest { .. })
        {
            None
        } else {
            reply(bytes)
        }
    })]));
    let connection = connect(&driver, &clock).unwrap();
    let watch = connection.watch_state();
    let mut first = connection.close();
    let mut second = connection.close();
    let mut cx = Context::from_waker(Waker::noop());
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert_eq!(watch.current(), ConnectionState::Closing);
    assert!(second.as_mut().poll(&mut cx).is_pending());
    drop(first);
    block_on_with(&clock, second).unwrap();
    assert_eq!(watch.current(), ConnectionState::Closed);
    assert_eq!(clock.sleeper_count(), 0);
}

#[test]
fn opcua_read_only_writes_are_refused_and_read_fails_after_close() {
    let (driver, clock, _) = driver(Script::strict(vec![Step::respond(reply)]));
    assert_eq!(driver.scheme(), "opcua");
    assert_eq!(
        driver.capabilities(),
        crate::opcua::connection::capabilities()
    );
    assert_eq!(
        Driver::parse_address(&driver, "ns=2;i=1", &AddressOptions::default())
            .unwrap()
            .protocol(),
        "opcua"
    );
    assert!(!format!("{driver:?}").contains("fake"));
    let connection = connect(&driver, &clock).unwrap();
    let tag = crate::Tag::new(
        crate::TagId::new("test").unwrap(),
        Arc::new(crate::opcua::OpcuaAddress::parse("ns=2;i=1").unwrap()),
    );
    for closed in [false, true] {
        if closed {
            block_on_with(&clock, connection.close()).unwrap();
        }
        let read = ReadRequest::builder(connection.limits())
            .tag(tag.clone())
            .build()
            .unwrap();
        let write = WriteRequest::builder(connection.limits())
            .item(tag.clone(), crate::PlcValue::Bool(true))
            .build()
            .unwrap();
        let expected_write = ErrorKind::Unsupported(Operation::Write);
        if closed {
            assert_eq!(
                block_on_with(&clock, connection.read(read))
                    .unwrap_err()
                    .kind(),
                ErrorKind::Disconnected
            );
        }
        assert_eq!(
            block_on_with(&clock, connection.write(write))
                .unwrap_err()
                .kind(),
            expected_write
        );
    }
}

#[tokio::test]
#[ignore = "requires the independent open62541 peer; interop/open62541/interop.sh tests"]
#[expect(
    clippy::disallowed_macros,
    reason = "interop evidence records public connection facts"
)]
async fn opcua_interop_connects_and_closes_with_policy_none() {
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("interop URI");
    let driver = OpcuaDriver::new();
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(driver)).unwrap();
    let connection = registry.connect(&uri).await.unwrap();
    assert_eq!(connection.state(), ConnectionState::Connected);
    connection.close().await.unwrap();
    assert_eq!(connection.state(), ConnectionState::Closed);
    println!("Stonecrab public anonymous None connect/close: Connected -> Closed");
}

fn scripted_response(request_id: u32, body: Vec<u8>) -> Script {
    Script::strict(vec![Step::respond(move |bytes| {
        if bytes.starts_with(b"MSG") {
            let Body::Message(chunk) = frame(bytes).body else {
                panic!("unexpected fixture variant")
            };
            if chunk.sequence.request_id == request_id {
                return Some(message(
                    chunk.sequence.sequence_number.checked_add(40).unwrap(),
                    request_id,
                    17,
                    29,
                    &body,
                ));
            }
        }
        reply(bytes)
    })])
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "fixture encodes only two setup responses"
)]
fn encode(service: &Service) -> Vec<u8> {
    let id = match service {
        Service::CreateSessionResponse { .. } => 464,
        Service::ActivateSessionResponse { .. } => 470,
        _ => panic!("response"),
    };
    crate::opcua::service::encode_service(id, service, 65535)
        .unwrap()
        .to_vec()
}

#[test]
fn opcua_create_session_adopts_the_authentication_token_and_revised_timeout() {
    let clock = FakeClock::default();
    let transport = FakeTransport::new(
        Script::strict(vec![Step::respond(reply)]),
        clock.clone(),
        ResourceLimits::builder().build().unwrap(),
    );
    let probe = transport.probe();
    let config = OpcuaDriver::configuration(URI).unwrap();
    let options = config.driver_options_as::<OpcuaOptions>().unwrap();
    let cancel = CancellationToken::new();
    let deadline = Deadline::after(clock.now(), Duration::from_secs(60)).unwrap();
    let channel = block_on_with(
        &clock,
        Channel::open(
            Box::new(transport),
            options,
            Arc::new(clock.clone()),
            deadline,
            &cancel,
            Arc::new(crate::opcua::crypto::Native),
            None,
        ),
    )
    .unwrap();
    assert_eq!(
        channel.server_nonce.as_bytes(),
        Some(b"server-nonce".as_slice())
    );
    let session = block_on_with(
        &clock,
        Session::open(
            Lane::from_channel(channel, Duration::from_millis(1)),
            options,
            Identity::Anonymous,
            [0xA5; 32],
            deadline,
            &cancel,
            None,
        ),
    )
    .unwrap();
    assert_eq!(session.authentication_token, NodeId::numeric(0, 77));
    assert_eq!(session.revised_timeout, Duration::from_millis(54321));
    let Service::CreateSessionRequest { client_nonce, .. } = payload(probe.sent().get(2).unwrap())
    else {
        panic!()
    };
    assert_eq!(client_nonce.as_bytes(), Some([0xA5; 32].as_slice()));
    let connection = OpcuaConnection::new(
        session,
        Arc::new(clock.clone()),
        Duration::from_secs(30),
        ResourceLimits::builder().build().unwrap(),
        crate::RetryPolicy::default(),
        crate::AccessPolicy::ReadOnly,
    );
    let shown = format!("{connection:?}");
    assert!(shown.contains("54.321s"));
    assert!(shown.contains("selected_endpoint: \"opc.tcp://fake:4840/path\""));
    assert!(!shown.contains("nonce"));
    let watch = connection.watch_state();
    drop(connection);
    assert_eq!(watch.current(), ConnectionState::Closed);
}

#[test]
fn opcua_create_session_rejects_null_token_invalid_revisions_and_incompatible_policies() {
    let base = decode_service(&create_response(2, 54321.0, 0)).unwrap();
    let mut cases = Vec::new();
    for timeout in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::MAX, 0.000_000_01] {
        let mut response = base.clone();
        let Service::CreateSessionResponse {
            revised_session_timeout,
            ..
        } = &mut response
        else {
            panic!("unexpected fixture variant")
        };
        *revised_session_timeout = timeout;
        cases.push((response, ErrorKind::MalformedData));
    }
    let mut response = base.clone();
    let Service::CreateSessionResponse {
        authentication_token,
        ..
    } = &mut response
    else {
        panic!("unexpected fixture variant")
    };
    *authentication_token = NodeId::numeric(0, 0);
    cases.push((response, ErrorKind::MalformedData));
    for choice in 0..9 {
        let mut response = base.clone();
        let Service::CreateSessionResponse {
            server_endpoints, ..
        } = &mut response
        else {
            panic!("unexpected fixture variant")
        };
        if choice == 0 {
            *server_endpoints = None;
        } else {
            let endpoint = server_endpoints.as_mut().unwrap().first_mut().unwrap();
            match choice {
                1 => endpoint.security_mode = MessageSecurityMode::MessageSecurityModeSign,
                2 => endpoint.security_policy_uri = UaString::new("wrong"),
                3 => endpoint.transport_profile_uri = UaString::new("wrong"),
                4 => endpoint.user_identity_tokens = None,
                5 => endpoint.user_identity_tokens.as_mut().unwrap().clear(),
                _ => {
                    let policy = endpoint
                        .user_identity_tokens
                        .as_mut()
                        .unwrap()
                        .first_mut()
                        .unwrap();
                    match choice {
                        6 => policy.token_type = UserTokenType::UserTokenTypeUserName,
                        7 => policy.policy_id = UaString::null(),
                        _ => policy.policy_id = UaString::new(""),
                    }
                }
            }
        }
        cases.push((response, ErrorKind::Authentication));
    }
    for (response, expected) in cases {
        let (driver, clock, factory) = driver(scripted_response(2, encode(&response)));
        assert_eq!(connect(&driver, &clock).err().unwrap().kind(), expected);
        assert_eq!(factory.probes().remove(0).sent().len(), 3);
        assert_eq!(clock.sleeper_count(), 0);
    }
}

#[test]
fn opcua_session_rejects_wrong_response_types_handles_and_service_statuses() {
    for (stage, response, expected) in [
        (2, activate_response(2, 0), ErrorKind::MalformedData),
        (2, create_response(9, 54321.0, 0), ErrorKind::MalformedData),
        (
            2,
            create_response(2, 54321.0, 0x8001_0000),
            ErrorKind::DeviceException,
        ),
        (3, create_response(3, 54321.0, 0), ErrorKind::MalformedData),
        (3, activate_response(9, 0), ErrorKind::MalformedData),
    ] {
        let (driver, clock, _) = driver(scripted_response(stage, response));
        assert_eq!(connect(&driver, &clock).err().unwrap().kind(), expected);
    }
    for response in [
        activate_response(4, 0),
        close_response(4, 0x8001_0000),
        close_response(9, 0),
    ] {
        let (driver, clock, factory) = driver(scripted_response(4, response));
        let connection = connect(&driver, &clock).unwrap();
        assert!(block_on_with(&clock, connection.close()).is_err());
        assert_eq!(connection.state(), ConnectionState::Closed);
        assert_eq!(
            factory.probes().remove(0).sent().len(),
            6,
            "CLO still attempted"
        );
    }
}

#[test]
fn opcua_each_handshake_stage_uses_the_connect_deadline() {
    for stage in 0..4 {
        let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(move |bytes| {
            let stop = match stage {
                0 => bytes.starts_with(b"HEL"),
                1 => bytes.starts_with(b"OPN"),
                2 => {
                    bytes.starts_with(b"MSG")
                        && matches!(payload(bytes), Service::CreateSessionRequest { .. })
                }
                _ => {
                    bytes.starts_with(b"MSG")
                        && matches!(payload(bytes), Service::ActivateSessionRequest { .. })
                }
            };
            if stop { None } else { reply(bytes) }
        })]));
        let config = Driver::parse_config(
            &driver,
            &format!("{URI}&handshake-timeout-ms=75&request-timeout-ms=1"),
        )
        .unwrap();
        let start = clock.now();
        assert_eq!(
            block_on_with(&clock, driver.connect(config))
                .err()
                .unwrap()
                .kind(),
            ErrorKind::Timeout
        );
        assert_eq!(clock.now().duration_since(start), Duration::from_millis(75));
        assert_eq!(factory.probes().remove(0).sent().len(), stage + 1);
    }
}

#[test]
fn opcua_connect_refused_yields_disconnected_error_and_invalid_configuration_opens_nothing() {
    let (driver, clock, factory) = driver(Script::default());
    let configs = [
        ConnectionConfig::builder(
            "other",
            Endpoint::Tcp {
                host: "fake".to_owned(),
                port: 4840,
            },
        )
        .build()
        .unwrap(),
        ConnectionConfig::builder("opcua", Endpoint::Custom("fake".to_owned()))
            .build()
            .unwrap(),
        ConnectionConfig::builder(
            "opcua",
            Endpoint::Tcp {
                host: "fake".to_owned(),
                port: 4840,
            },
        )
        .build()
        .unwrap(),
        Driver::parse_config(&driver, "opcua://fake:4840").unwrap(),
    ];
    for config in configs {
        assert_eq!(
            block_on_with(&clock, driver.connect(config))
                .err()
                .unwrap()
                .kind(),
            ErrorKind::InvalidConfig
        );
    }
    let options = OpcuaOptions::from_parameters(
        "opc.tcp://fake:4840".to_owned(),
        &[("security-policy".to_owned(), "NONE".to_owned())],
    )
    .unwrap();
    let config = ConnectionConfig::builder(
        "opcua",
        Endpoint::Tcp {
            host: "fake".to_owned(),
            port: 4840,
        },
    )
    .driver_options(Arc::new(options))
    .credentials(crate::Credentials::new("operator", "do-not-show"))
    .build()
    .unwrap();
    let problem = block_on_with(&clock, driver.connect(config)).err().unwrap();
    assert_eq!(problem.kind(), ErrorKind::Authentication);
    assert!(!problem.to_string().contains("do-not-show"));
    assert_eq!(factory.opens().len(), 0);
    // The first queued script is silent, proving the TCP open has a single deadline too.
    assert_eq!(
        connect(&driver, &clock).err().unwrap().kind(),
        ErrorKind::Timeout
    );
    factory.push_refusal();
    assert_eq!(
        connect(&driver, &clock).err().unwrap().kind(),
        ErrorKind::Disconnected
    );
    factory.push_connect_timeout();
    assert_eq!(
        connect(&driver, &clock).err().unwrap().kind(),
        ErrorKind::Timeout
    );
}

#[test]
fn opcua_a_concurrent_close_is_bounded_if_the_first_future_is_not_driven() {
    let (driver, clock, _) = driver(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"MSG")
            && matches!(payload(bytes), Service::CloseSessionRequest { .. })
        {
            None
        } else {
            reply(bytes)
        }
    })]));
    let connection = connect(&driver, &clock).unwrap();
    let mut first = connection.close();
    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let start = clock.now();
    assert_eq!(
        block_on_with(&clock, connection.close())
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(clock.now().duration_since(start), Duration::from_secs(30));
    assert_eq!(connection.state(), ConnectionState::Closed);
    assert_eq!(
        block_on_with(&clock, first).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
}

#[test]
fn opcua_config_handshake_timeout_default_is_60000() {
    let config = OpcuaDriver::configuration(URI).unwrap();
    assert_eq!(config.timeouts().connect, Duration::from_secs(60));
    assert_eq!(
        config
            .driver_options_as::<OpcuaOptions>()
            .unwrap()
            .handshake_timeout,
        config.timeouts().connect
    );
    let config = OpcuaDriver::configuration(&format!("{URI}&handshake-timeout-ms=123")).unwrap();
    assert_eq!(config.timeouts().connect, Duration::from_millis(123));
}

#[test]
fn opcua_config_session_timeout_default_is_120000() {
    let config = OpcuaDriver::configuration(URI).unwrap();
    assert_eq!(
        config
            .driver_options_as::<OpcuaOptions>()
            .unwrap()
            .session_timeout,
        Duration::from_secs(120)
    );
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(reply)]));
    let config = OpcuaDriver::configuration(&format!("{URI}&session-timeout-ms=87654")).unwrap();
    let connection = block_on_with(&clock, driver.connect(config)).unwrap();
    let sent = factory.probes().remove(0).sent();
    let Service::CreateSessionRequest {
        requested_session_timeout,
        ..
    } = payload(sent.get(2).unwrap())
    else {
        panic!("create")
    };
    assert_eq!(requested_session_timeout, 87654.0);
    drop(connection);
    for key in ["session-timeout-ms", "handshake-timeout-ms"] {
        for value in ["0", "-1", "+1", "a", "1.5", "18446744073709551616"] {
            assert_eq!(
                OpcuaDriver::configuration(&format!("{URI}&{key}={value}"))
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidConfig
            );
        }
    }
}

#[test]
fn opcua_session_cancellation_and_drop_never_activate_a_half_open_session() {
    for stage in [2, 3] {
        for dropped in [false, true] {
            let clock = FakeClock::default();
            let transport = FakeTransport::new(
                Script::strict(vec![Step::respond(move |bytes| {
                    if bytes.starts_with(b"MSG") {
                        let Body::Message(chunk) = frame(bytes).body else {
                            panic!("message")
                        };
                        if chunk.sequence.request_id == stage {
                            return None;
                        }
                    }
                    reply(bytes)
                })]),
                clock.clone(),
                ResourceLimits::builder().build().unwrap(),
            );
            let probe = transport.probe();
            let config = OpcuaDriver::configuration(URI).unwrap();
            let options = config.driver_options_as::<OpcuaOptions>().unwrap();
            let cancel = CancellationToken::new();
            let deadline = Deadline::after(clock.now(), Duration::from_secs(60)).unwrap();
            let channel = block_on_with(
                &clock,
                Channel::open(
                    Box::new(transport),
                    options,
                    Arc::new(clock.clone()),
                    deadline,
                    &cancel,
                    Arc::new(crate::opcua::crypto::Native),
                    None,
                ),
            )
            .unwrap();
            let mut session = Box::pin(Session::open(
                Lane::from_channel(channel, Duration::from_secs(30)),
                options,
                Identity::Anonymous,
                [7; 32],
                deadline,
                &cancel,
                None,
            ));
            assert!(
                session
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            if dropped {
                drop(session);
            } else {
                cancel.cancel();
                assert_eq!(
                    block_on_with(&clock, session).err().unwrap().kind(),
                    ErrorKind::Cancelled
                );
            }
            assert_eq!(probe.sent().len(), usize::try_from(stage).unwrap() + 1);
            assert_eq!(clock.sleeper_count(), 0);
        }
    }
}

// covers: OPCUA-LIFE-ACTIVATE
#[test]
fn opcua_activate_session_uses_the_selected_endpoint_token_policy() {
    let mut response = decode_service(&create_response(2, 54321.0, 0)).unwrap();
    let Service::CreateSessionResponse {
        server_endpoints, ..
    } = &mut response
    else {
        panic!("fixture")
    };
    let mut stronger = server_endpoints.as_ref().unwrap().first().unwrap().clone();
    stronger.security_level = 255;
    stronger
        .user_identity_tokens
        .as_mut()
        .unwrap()
        .first_mut()
        .unwrap()
        .policy_id = UaString::new("selected-anonymous");
    server_endpoints.as_mut().unwrap().push(stronger);
    let (driver, clock, factory) = driver(scripted_response(2, encode(&response)));
    let connection = connect(&driver, &clock).unwrap();
    let sent = factory.probes().remove(0).sent();
    let Service::ActivateSessionRequest {
        user_identity_token,
        ..
    } = payload(sent.get(3).unwrap())
    else {
        panic!("activate")
    };
    assert_eq!(
        user_identity_token.body,
        ExtensionBody::Known(Box::new(Service::AnonymousIdentityToken {
            policy_id: UaString::new("selected-anonymous")
        }))
    );
    block_on_with(&clock, connection.close()).unwrap();
}

#[test]
fn opcua_anonymous_session_activates_with_an_anonymous_identity_token() {
    let (driver, clock, factory) = driver(Script::strict(vec![
        Step::respond(reply),
        Step::respond(reply),
        Step::respond(reply),
        Step::respond(reply),
        Step::respond(reply),
        Step::respond(reply),
    ]));
    let config =
        OpcuaDriver::configuration("opcua://fake:4840/path?security-policy=NONE&discovery=false")
            .unwrap();
    let connection = block_on_with(&clock, driver.connect(config)).unwrap();
    let sent = factory.probes().remove(0).sent();
    assert_eq!(
        sent.len(),
        4,
        "None connect skips GetEndpoints even with explicit discovery option"
    );
    let Service::ActivateSessionRequest {
        user_identity_token,
        ..
    } = payload(sent.get(3).unwrap())
    else {
        panic!("activate")
    };
    assert_eq!(
        user_identity_token.body,
        ExtensionBody::Known(Box::new(Service::AnonymousIdentityToken {
            policy_id: UaString::new("anonymous-policy")
        }))
    );
    block_on_with(&clock, connection.close()).unwrap();
}

fn user_response(policy_uri: UaString) -> Vec<u8> {
    let mut response = decode_service(&create_response(2, 54321.0, 0)).unwrap();
    let Service::CreateSessionResponse {
        server_endpoints, ..
    } = &mut response
    else {
        panic!("fixture")
    };
    let policies = server_endpoints
        .as_mut()
        .unwrap()
        .first_mut()
        .unwrap()
        .user_identity_tokens
        .as_mut()
        .unwrap();
    let mut policy = policies.first().unwrap().clone();
    policy.policy_id = UaString::new("selected-user");
    policy.token_type = UserTokenType::UserTokenTypeUserName;
    policy.security_policy_uri = policy_uri;
    policies.push(policy);
    encode(&response)
}

fn registry(driver: OpcuaDriver) -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(driver)).unwrap();
    registry
}

// covers: OPCUA-CFG-PASSWORD / OPCUA-CFG-USERNAME
#[test]
fn opcua_config_password_is_redacted_in_display_and_debug() {
    let config =
        OpcuaDriver::configuration(URI)
            .unwrap()
            .with_credentials(crate::Credentials::new(
                "operator-sentinel",
                "password-sentinel",
            ));
    for text in [format!("{config:?}"), config.redacted_display()] {
        assert!(!text.contains("operator-sentinel"));
        assert!(!text.contains("password-sentinel"));
    }
    for key in ["username", "password"] {
        let (driver, clock, factory) = driver(Script::strict(vec![]));
        let registry = registry(driver);
        let error = block_on_with(
            &clock,
            registry.connect_with(
                &format!("{URI}&{key}=password-sentinel"),
                crate::Credentials::new("operator-sentinel", "password-sentinel"),
            ),
        )
        .err()
        .unwrap();
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(!format!("{error:?}").contains("password-sentinel"));
        assert_eq!(factory.opens().len(), 0);
    }
}

// covers: OPCUA-AUTH-RESOLVE
#[test]
fn opcua_credentials_come_only_from_the_config() {
    for token_uri in [
        UaString::null(),
        UaString::new(""),
        UaString::new(crate::opcua::options::SecurityPolicy::None.uri()),
    ] {
        let (driver, clock, factory) = driver(scripted_response(2, user_response(token_uri)));
        let registry = registry(driver);
        let connection = block_on_with(
            &clock,
            registry.connect_with(
                &format!("{URI}&allow-insecure-credentials=true"),
                crate::Credentials::new("operator-sentinel", "пароль\0sentinel"),
            ),
        )
        .unwrap();
        let sent = factory.probes().remove(0).sent();
        let Service::ActivateSessionRequest {
            user_identity_token,
            ..
        } = payload(sent.get(3).unwrap())
        else {
            panic!("activate")
        };
        assert_eq!(
            user_identity_token.type_id,
            ExpandedNodeId::local(NodeId::numeric(0, 324))
        );
        let ExtensionBody::Known(body) = user_identity_token.body else {
            panic!("known token")
        };
        assert_eq!(
            *body,
            Service::UserNameIdentityToken {
                policy_id: UaString::new("selected-user"),
                user_name: UaString::new("operator-sentinel"),
                password: UaByteString::new("пароль\0sentinel".as_bytes().to_vec()),
                encryption_algorithm: UaString::null()
            }
        );
        block_on_with(&clock, connection.close()).unwrap();
    }
}

#[test]
fn opcua_no_credentials_means_anonymous() {
    for credentials in [None, Some(crate::Credentials::new("", ""))] {
        let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(reply)]));
        let registry = registry(driver);
        let connection = if let Some(credentials) = credentials {
            block_on_with(&clock, registry.connect_with(URI, credentials)).unwrap()
        } else {
            block_on_with(&clock, registry.connect(URI)).unwrap()
        };
        let sent = factory.probes().remove(0).sent();
        let Service::ActivateSessionRequest {
            user_identity_token,
            ..
        } = payload(sent.get(3).unwrap())
        else {
            panic!("activate")
        };
        assert_eq!(
            user_identity_token.type_id,
            ExpandedNodeId::local(NodeId::numeric(0, 321))
        );
        block_on_with(&clock, connection.close()).unwrap();
    }
}

// covers: OPCUA-AUTH-INSECURE / OPCUA-CFG-ALLOW-INSECURE-CREDENTIALS
#[test]
fn opcua_credentials_over_an_unprotected_token_policy_fail_before_sending() {
    for flag in ["", "&allow-insecure-credentials=false"] {
        let (driver, clock, factory) = driver(Script::strict(vec![]));
        let registry = registry(driver);
        let problem = block_on_with(
            &clock,
            registry.connect_with(
                &format!("{URI}{flag}"),
                crate::Credentials::new("operator", "password-sentinel"),
            ),
        )
        .err()
        .unwrap();
        assert_eq!(problem.kind(), ErrorKind::Authentication);
        assert!(!problem.to_string().contains("password-sentinel"));
        assert_eq!(factory.opens().len(), 0);
    }
}

struct CapturedLog(Arc<Mutex<Vec<u8>>>);
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = tracing_subscriber::fmt::writer::MutexGuardWriter<'a, Vec<u8>>;
    fn make_writer(&'a self) -> Self::Writer {
        tracing_subscriber::fmt::MakeWriter::make_writer(self.0.as_ref())
    }
}

#[test]
fn opcua_allow_insecure_credentials_sends_the_user_name_and_warns() {
    // tracing-core#3611: keep registration off the single-dispatch TLS fast path.
    let _registered = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(CapturedLog(Arc::clone(&output)))
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        let (driver, clock, _) = driver(scripted_response(2, user_response(UaString::null())));
        let registry = registry(driver);
        let connection = block_on_with(
            &clock,
            registry.connect_with(
                &format!("{URI}&allow-insecure-credentials=true"),
                crate::Credentials::new("operator-sentinel", "password-sentinel"),
            ),
        )
        .unwrap();
        block_on_with(&clock, connection.close()).unwrap();
    });
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    assert!(text.contains("WARN"));
    assert!(text.contains("allow-insecure-credentials=true"));
    assert!(!text.contains("operator-sentinel"));
    assert!(!text.contains("password-sentinel"));
}

// covers: OPCUA-AUTH-TOKEN-POLICY
#[test]
fn opcua_an_endpoint_without_a_matching_user_token_policy_is_not_selected() {
    let (driver, clock, factory) = driver(Script::strict(vec![Step::respond(reply)]));
    let registry = registry(driver);
    let problem = block_on_with(
        &clock,
        registry.connect_with(
            &format!("{URI}&allow-insecure-credentials=true"),
            crate::Credentials::new("operator", "password-sentinel"),
        ),
    )
    .err()
    .unwrap();
    assert_eq!(problem.kind(), ErrorKind::Authentication);
    assert_eq!(factory.probes().remove(0).sent().len(), 3);
    assert!(!problem.to_string().contains("password-sentinel"));
}

#[test]
fn opcua_protected_username_policy_is_never_downgraded_to_plain() {
    for token_uri in [
        "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256",
        "unrecognized-protected-policy",
    ] {
        let (driver, clock, factory) = driver(scripted_response(
            2,
            user_response(UaString::new(token_uri)),
        ));
        let registry = registry(driver);
        let problem = block_on_with(
            &clock,
            registry.connect_with(
                &format!("{URI}&allow-insecure-credentials=true"),
                crate::Credentials::new("operator", "password-sentinel"),
            ),
        )
        .err()
        .unwrap();
        assert_eq!(problem.kind(), ErrorKind::Unsupported(Operation::Connect));
        assert_eq!(factory.probes().remove(0).sent().len(), 3);
        assert!(!problem.to_string().contains("password-sentinel"));
    }
}

// covers: OPCUA-DIV-PASSWORD-WITHOUT-USER / OPCUA-AUTH-TYPES
#[test]
fn opcua_a_password_without_a_user_name_is_invalid_config() {
    let (driver, clock, factory) = driver(Script::strict(vec![]));
    let registry = registry(driver);
    let error = block_on_with(
        &clock,
        registry.connect_with(URI, crate::Credentials::new("", "password-sentinel")),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(!error.to_string().contains("password-sentinel"));
    assert_eq!(factory.opens().len(), 0);
}
#[test]
fn opcua_an_unsupported_credential_kind_is_invalid_config() {
    let (driver, clock, factory) = driver(Script::strict(vec![]));
    let registry = registry(driver);
    let error = block_on_with(
        &clock,
        registry.connect_with(
            URI,
            crate::Credentials::x509("certificate-sentinel", "key-sentinel"),
        ),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(!format!("{error:?}").contains("sentinel"));
    assert_eq!(factory.opens().len(), 0);
}

#[test]
fn opcua_identity_refusals_are_authentication_with_the_device_source_preserved() {
    use crate::opcua::codec::StatusCode;
    use crate::opcua::service::StatusFault;
    use std::error::Error;
    for code in [0x801F_0000, 0x8020_0000, 0x8021_0000, 0x8057_0000] {
        for wrapped in [false, true] {
            let device = status_error(StatusCode(code));
            let problem = if wrapped {
                PlcError::new(ErrorKind::Disconnected, "wrapped source").with_source(device)
            } else {
                device
            };
            let error = activation_error(problem);
            assert_eq!(error.kind(), ErrorKind::Authentication);
            let mut source = error.source();
            let mut found = None;
            while let Some(current) = source {
                if let Some(status) = current.downcast_ref::<StatusFault>() {
                    found = Some(status.code);
                }
                source = current.source();
            }
            assert_eq!(found, Some(code));
        }
    }
    assert_eq!(
        activation_error(PlcError::new(ErrorKind::Timeout, "deadline")).kind(),
        ErrorKind::Timeout
    );
    assert_eq!(
        activation_error(status_error(StatusCode(0x8001_0000))).kind(),
        ErrorKind::DeviceException
    );
    for reply in [
        activate_response(3, 0x801F_0000),
        service_fault(3, 0x8020_0000),
    ] {
        let (driver, clock, _) = driver(scripted_response(3, reply));
        assert_eq!(
            connect(&driver, &clock).err().unwrap().kind(),
            ErrorKind::Authentication
        );
    }
}

#[tokio::test]
#[expect(
    clippy::disallowed_macros,
    reason = "ignored independent peer test records only nonsensitive login/refusal outcomes"
)]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_username_over_none_requires_opt_in_and_wrong_password_is_authentication() {
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("endpoint");
    let file =
        std::env::var("STONECRAB_INTEROP_OPEN62541_PASSWORD_FILE").expect("runtime password file");
    let password = std::fs::read_to_string(file).expect("runtime PKI password");
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(OpcuaDriver::new())).unwrap();
    assert_eq!(
        registry
            .connect_with(&uri, crate::Credentials::new("stonecrab", password.clone()))
            .await
            .err()
            .unwrap()
            .kind(),
        ErrorKind::Authentication
    );
    let uri = format!("{uri}&allow-insecure-credentials=true");
    let connection = registry
        .connect_with(&uri, crate::Credentials::new("stonecrab", password.clone()))
        .await
        .unwrap();
    assert_eq!(connection.state(), ConnectionState::Connected);
    connection.close().await.unwrap();
    let error = registry
        .connect_with(
            &uri,
            crate::Credentials::new("stonecrab", format!("{password}\0wrong")),
        )
        .await
        .err()
        .unwrap();
    assert_eq!(error.kind(), ErrorKind::Authentication);
    assert!(!format!("{error:?}").contains(&password));
    println!(
        "public username/None opt-in connect/close passed; missing opt-in and wrong password refused without credential disclosure"
    );
}
