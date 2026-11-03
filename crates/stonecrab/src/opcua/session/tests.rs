use std::sync::Arc;
use std::task::{Context, Waker};

use bytes::BytesMut;

use super::*;
use crate::opcua::OpcuaDriver;
use crate::opcua::channel::Channel;
use crate::opcua::connection::OpcuaConnection;
use crate::opcua::frame::{Body, Frame, OpcuaFramer};
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
    assert_eq!(connection.capabilities(), crate::Capabilities::default());
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
        assert_eq!(problem.kind(), ErrorKind::DeviceException);
        assert!(problem.to_string().contains("0x801F0000"));
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
fn opcua_data_operations_are_unsupported_until_implemented_and_disconnected_after_close() {
    let (driver, clock, _) = driver(Script::strict(vec![Step::respond(reply)]));
    assert_eq!(driver.scheme(), "opcua");
    assert_eq!(driver.capabilities(), crate::Capabilities::default());
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
        let expected_read = if closed {
            ErrorKind::Disconnected
        } else {
            ErrorKind::Unsupported(Operation::Read)
        };
        let expected_write = if closed {
            ErrorKind::Disconnected
        } else {
            ErrorKind::Unsupported(Operation::Write)
        };
        assert_eq!(
            block_on_with(&clock, connection.read(read))
                .unwrap_err()
                .kind(),
            expected_read
        );
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
        Channel::open_none(
            Box::new(transport),
            options,
            Arc::new(clock.clone()),
            deadline,
            &cancel,
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
            [0xA5; 32],
            deadline,
            &cancel,
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
    );
    let shown = format!("{connection:?}");
    assert!(shown.contains("54.321s"));
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
