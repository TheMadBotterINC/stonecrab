// covers: OPCUA-POLICY-BASIC256SHA256
// covers: OPCUA-MODE-SIGN
// covers: OPCUA-MODE-SIGN-ENCRYPT
// covers: OPCUA-MODE-DETERMINE
// covers: OPCUA-CERT-VERIFY-DEFAULT
// covers: OPCUA-CERT-VERIFY-POINTS
// covers: OPCUA-CERT-VERIFY-DISCOVERY
// covers: OPCUA-DISC-GETENDPOINTS
// covers: OPCUA-DISC-NONE-POLICY

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use super::codec::{UaByteString, UaString};
use super::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use super::options::MessageSecurity;
use super::secure_responder::{Fixture, Probe};
use super::{OpcuaAddress, OpcuaDriver};
use crate::testkit::{FakeClock, FakeTransportFactory, Reaction, Script, Step, block_on_with};
use crate::{Connection, Driver, ErrorKind, PlcValue, ReadRequest, Tag, TagId, WriteRequest};

fn driver(
    reactor: impl FnMut(&[u8]) -> Reaction + Send + 'static,
) -> (OpcuaDriver, FakeClock, FakeTransportFactory) {
    let clock = FakeClock::default();
    clock.set_wall_now(
        UNIX_EPOCH
            .checked_add(Duration::from_secs(super::pki::NOW))
            .unwrap(),
    );
    let factory = FakeTransportFactory::new(clock.clone());
    factory.push_script(Script::strict(vec![Step::react(reactor)]));
    let driver = OpcuaDriver::new()
        .with_clock(Arc::new(clock.clone()))
        .with_transport_factory(Arc::new(factory.clone()));
    (driver, clock, factory)
}

fn connect(
    driver: &OpcuaDriver,
    clock: &FakeClock,
    uri: &str,
) -> crate::Result<Box<dyn Connection>> {
    block_on_with(clock, driver.connect(driver.parse_config(uri)?))
}

fn tag() -> Tag {
    Tag::new(
        TagId::new("value").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;i=42;DINT").unwrap()),
    )
}

fn activated(probe: &Probe) -> bool {
    probe
        .requests()
        .iter()
        .any(|request| matches!(request, Service::ActivateSessionRequest { .. }))
}

#[test]
fn opcua_basic256sha256_public_driver_authenticates_sessions_and_reads_writes_closes() {
    let fixture = Fixture::new();
    for mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
        for discovery in [false, true] {
            let peer = fixture.peer(mode);
            let probe = peer.probe.clone();
            let (driver, clock, factory) = driver(peer.reactor(|_, _| {}));
            let connection = connect(&driver, &clock, &fixture.uri(mode, discovery)).unwrap();
            let write = WriteRequest::builder(connection.limits())
                .item(tag(), PlcValue::I32(731))
                .build()
                .unwrap();
            assert_eq!(
                block_on_with(&clock, connection.write(write))
                    .unwrap()
                    .confirmed_count(),
                1
            );
            let read = ReadRequest::builder(connection.limits())
                .tag(tag())
                .build()
                .unwrap();
            let read = block_on_with(&clock, connection.read(read)).unwrap();
            assert_eq!(
                read.items().first().unwrap().outcome().as_ref().unwrap(),
                &PlcValue::I32(731)
            );
            block_on_with(&clock, connection.close()).unwrap();
            assert!(activated(&probe));
            assert_eq!(factory.opens().len(), 1);
            let requests = probe.requests();
            assert_eq!(
                requests
                    .iter()
                    .filter(|request| matches!(request, Service::GetEndpointsRequest { .. }))
                    .count(),
                usize::from(discovery)
            );
            let create = requests
                .iter()
                .find(|request| matches!(request, Service::CreateSessionRequest { .. }))
                .unwrap();
            let Service::CreateSessionRequest {
                client_certificate,
                client_nonce,
                client_description,
                ..
            } = create
            else {
                panic!("create")
            };
            assert_eq!(
                client_certificate.as_bytes(),
                Some(fixture.client_certificate.as_slice())
            );
            assert_eq!(client_nonce.as_bytes().unwrap().len(), 32);
            assert_eq!(
                client_description.application_uri,
                UaString::new("urn:stonecrab:probe")
            );
            assert!(matches!(
                requests.last(),
                Some(Service::CloseSecureChannelRequest { .. })
            ));
            assert!(probe.incoming().iter().any(|wire| wire.starts_with(b"CLO")));
            assert!(probe.outgoing().iter().any(|wire| wire.starts_with(b"OPN")));
        }
    }
}

#[test]
fn opcua_secure_session_rejects_invalid_server_proofs_before_activation() {
    let fixture = Fixture::new();
    for mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
        for fault in 0..10 {
            let peer = fixture.peer(mode);
            let probe = peer.probe.clone();
            let substitute = fixture.client_certificate.clone();
            let (driver, clock, factory) = driver(peer.reactor(move |request, response| {
                if let Service::CreateSessionResponse {
                    server_signature,
                    server_nonce,
                    server_certificate,
                    ..
                } = response
                {
                    let Service::CreateSessionRequest { client_nonce, .. } = request else {
                        panic!("create")
                    };
                    match fault {
                        0 => server_signature.algorithm = UaString::null(),
                        1 => server_signature.algorithm = UaString::new("wrong"),
                        2 => server_signature.signature = UaByteString::null(),
                        3 => server_signature.signature = UaByteString::new(vec![0; 256]),
                        4 => *server_nonce = UaByteString::null(),
                        5 => *server_nonce = UaByteString::new(vec![1; 31]),
                        6 => *server_nonce = UaByteString::new(vec![1; 129]),
                        7 => *server_nonce = UaByteString::new(vec![0; 32]),
                        8 => *server_nonce = client_nonce.clone(),
                        _ => *server_certificate = UaByteString::new(substitute.clone()),
                    }
                }
            }));
            let error = connect(&driver, &clock, &fixture.uri(mode, false))
                .err()
                .unwrap();
            assert_eq!(
                error.kind(),
                if fault == 9 {
                    ErrorKind::Trust
                } else {
                    ErrorKind::Authentication
                },
                "fault {fault}: {error}"
            );
            assert!(!activated(&probe));
            assert_eq!(factory.opens().len(), 1);
        }
    }
}

#[test]
fn opcua_secure_discovery_certificate_remains_bound_when_verification_is_disabled() {
    let fixture = Fixture::new();
    for verify in [true, false] {
        let peer = fixture.peer(MessageSecurity::SignEncrypt);
        let probe = peer.probe.clone();
        let substitute = fixture.client_certificate.clone();
        let (driver, clock, factory) = driver(peer.reactor(move |_, response| {
            if let Service::GetEndpointsResponse { endpoints, .. } = response {
                endpoints
                    .as_mut()
                    .unwrap()
                    .first_mut()
                    .unwrap()
                    .server_certificate = UaByteString::new(substitute.clone());
            }
        }));
        let uri = format!(
            "{}&tls.verify={verify}",
            fixture.uri(MessageSecurity::SignEncrypt, true)
        );
        assert_eq!(
            connect(&driver, &clock, &uri).err().unwrap().kind(),
            ErrorKind::Trust
        );
        assert!(
            !probe
                .requests()
                .iter()
                .any(|request| matches!(request, Service::CreateSessionRequest { .. }))
        );
        assert_eq!(factory.opens().len(), 1);
    }
}

#[test]
fn opcua_secure_activation_requires_a_fresh_valid_nonce() {
    let fixture = Fixture::new();
    for nonce in [
        UaByteString::null(),
        UaByteString::new(vec![0; 32]),
        UaByteString::new(vec![0x66; 32]),
        UaByteString::new(vec![1; 31]),
    ] {
        let peer = fixture.peer(MessageSecurity::Sign);
        let (driver, clock, _) = driver(peer.reactor(move |_, response| {
            if let Service::ActivateSessionResponse { server_nonce, .. } = response {
                *server_nonce = nonce.clone();
            }
        }));
        assert_eq!(
            connect(&driver, &clock, &fixture.uri(MessageSecurity::Sign, false))
                .err()
                .unwrap()
                .kind(),
            ErrorKind::Authentication
        );
    }
}

#[test]
fn opcua_secure_open_requires_an_independent_policy_sized_nonce() {
    let fixture = Fixture::new();
    for fault in 0..4 {
        let peer = fixture.peer(MessageSecurity::SignEncrypt);
        let probe = peer.probe.clone();
        let (driver, clock, _) = driver(peer.reactor(move |request, response| {
            if let Service::OpenSecureChannelResponse { server_nonce, .. } = response {
                let Service::OpenSecureChannelRequest { client_nonce, .. } = request else {
                    panic!("open")
                };
                *server_nonce = match fault {
                    0 => UaByteString::null(),
                    1 => UaByteString::new(vec![1; 31]),
                    2 => UaByteString::new(vec![0; 32]),
                    _ => client_nonce.clone(),
                };
            }
        }));
        assert_eq!(
            connect(
                &driver,
                &clock,
                &fixture.uri(MessageSecurity::SignEncrypt, false)
            )
            .err()
            .unwrap()
            .kind(),
            ErrorKind::Authentication
        );
        assert_eq!(probe.requests().len(), 1);
    }
}

#[test]
fn opcua_secure_wire_corruption_has_one_authentication_error_and_no_fallback() {
    let fixture = Fixture::new();
    for mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
        for target in [b"OPN", b"MSG"] {
            for fault in 0..3 {
                let peer = fixture.peer(mode);
                let mut reactor = peer.reactor(|_, _| {});
                let (driver, clock, factory) = driver(move |request| {
                    let mut reaction = reactor(request);
                    if request.starts_with(target) {
                        let Reaction::Reply(bytes) = &mut reaction else {
                            panic!("reply")
                        };
                        match fault {
                            0 => *bytes.last_mut().unwrap() ^= 0x80,
                            1 => *bytes.first_mut().unwrap() = b'X',
                            _ => *bytes.get_mut(8).unwrap() ^= 1,
                        }
                    }
                    reaction
                });
                let error = connect(&driver, &clock, &fixture.uri(mode, false))
                    .err()
                    .unwrap();
                assert_eq!(error.kind(), ErrorKind::Authentication, "{error}");
                assert_eq!(
                    error.to_string(),
                    "OPC UA message security verification failed"
                );
                assert_eq!(factory.opens().len(), 1);
            }
        }
    }
}

#[test]
fn opcua_secure_bootstrap_rejects_untrusted_and_expired_peers_before_network() {
    let fixture = Fixture::new();
    let untrusted = fixture.scratch.write(
        "untrusted.pem",
        &super::pki::pem("CERTIFICATE", &fixture.client_certificate),
    );
    for expired in [false, true] {
        let (driver, clock, factory) =
            driver(fixture.peer(MessageSecurity::Sign).reactor(|_, _| {}));
        let mut uri = fixture.uri(MessageSecurity::Sign, false);
        if expired {
            clock.set_wall_now(
                UNIX_EPOCH
                    .checked_add(Duration::from_mins(35_000_000))
                    .unwrap(),
            );
        } else {
            uri = format!("{uri}&tls.trust-store={}", untrusted.display());
        }
        assert_eq!(
            connect(&driver, &clock, &uri).err().unwrap().kind(),
            ErrorKind::Trust
        );
        assert_eq!(factory.opens().len(), 0);
    }
}

#[test]
fn opcua_secure_chunked_requests_and_coalesced_responses_obey_negotiated_buffers() {
    let fixture = Fixture::new();
    for mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
        let peer = fixture.peer(mode);
        let probe = peer.probe.clone();
        let (driver, clock, _) = driver(peer.reactor(|_, _| {}));
        let uri = format!(
            "{}&encoding.receive-buffer-size=8192&encoding.send-buffer-size=8192",
            fixture.uri(mode, false)
        );
        let connection = connect(&driver, &clock, &uri).unwrap();
        let tags = (0..1000)
            .map(|index| {
                Tag::new(
                    TagId::new(&format!("value{index}")).unwrap(),
                    Arc::new(OpcuaAddress::parse(&format!("ns=1;i={index};DINT")).unwrap()),
                )
            })
            .collect::<Vec<_>>();
        let mut write = WriteRequest::builder(connection.limits());
        for tag in &tags {
            write = write.item(tag.clone(), PlcValue::I32(731));
        }
        assert_eq!(
            block_on_with(&clock, connection.write(write.build().unwrap()))
                .unwrap()
                .confirmed_count(),
            1000
        );
        let mut read = ReadRequest::builder(connection.limits());
        for tag in tags {
            read = read.tag(tag);
        }
        let values = block_on_with(&clock, connection.read(read.build().unwrap())).unwrap();
        assert_eq!(values.items().len(), 1000);
        assert!(
            values
                .iter_ok()
                .all(|item| item.outcome().as_ref().unwrap() == &PlcValue::I32(731))
        );
        assert!(
            probe
                .incoming()
                .iter()
                .any(|wire| wire.starts_with(b"MSGC"))
        );
        assert!(probe.incoming().iter().all(|wire| wire.len() <= 8192));
        assert!(probe.outgoing().iter().any(|wire| wire.len() > 8192));
        block_on_with(&clock, connection.close()).unwrap();
    }
}

#[test]
fn opcua_secure_channel_checks_policy_trust_time_and_crypto_failures() {
    use super::channel::Channel;
    use super::crypto::{Native, Security, tests::Injected};
    use super::options::{OpcuaOptions, SecurityPolicy};
    use crate::testkit::FakeTransport;
    use crate::{CancellationToken, Clock, Deadline, ResourceLimits};
    let fixture = Fixture::new();
    for fault in 0..6 {
        let (driver, clock, _) = driver(fixture.peer(MessageSecurity::Sign).reactor(|_, _| {}));
        let config = driver
            .parse_config(&fixture.uri(MessageSecurity::Sign, false))
            .unwrap();
        let mut options = config.driver_options_as::<OpcuaOptions>().unwrap().clone();
        let security = Security::prepare(&options, clock.wall_now())
            .unwrap()
            .unwrap();
        let advanced = clock.clone();
        let mut reactor = fixture.peer(MessageSecurity::Sign).reactor(|_, _| {});
        let transport = FakeTransport::new(
            Script::strict(vec![Step::react(move |request| {
                let mut response = reactor(request);
                if fault == 1 && request.starts_with(b"OPN") {
                    advanced.set_wall_now(
                        UNIX_EPOCH
                            .checked_add(Duration::from_mins(35_000_000))
                            .unwrap(),
                    );
                }
                if fault == 0 && request.starts_with(b"HEL") {
                    response = Reaction::Reply(super::responder::acknowledge(8192, 8192, 0, 0));
                }
                response
            })]),
            clock.clone(),
            ResourceLimits::builder().build().unwrap(),
        );
        if fault == 0 {
            options.max_message_size = 0;
        }
        if fault == 3 {
            options.message_security = MessageSecurity::None;
        }
        if fault == 4 {
            options.security_policy = SecurityPolicy::Aes128Sha256RsaOaep;
        }
        if fault == 5 {
            options.security_policy = SecurityPolicy::Aes128Sha256RsaOaep;
            assert_eq!(
                Security::prepare(&options, clock.wall_now())
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidConfig
            );
            options.security_policy = SecurityPolicy::Aes256Sha256RsaPss;
            assert_eq!(
                Security::prepare(&options, clock.wall_now())
                    .unwrap_err()
                    .kind(),
                ErrorKind::InvalidConfig
            );
            continue;
        }
        let backend: Arc<dyn super::crypto::Backend> = if fault == 2 {
            Arc::new(Injected::new(1, None))
        } else {
            Arc::new(Native)
        };
        let result = block_on_with(
            &clock,
            Channel::open(
                Box::new(transport),
                &options,
                Arc::new(clock.clone()),
                Deadline::after(clock.now(), Duration::from_secs(5)).unwrap(),
                &CancellationToken::new(),
                backend,
                Some(&security),
            ),
        );
        if fault == 0 {
            let channel = result.unwrap();
            assert!(format!("{channel:?}").contains("SIGN"));
            let debug = format!("{:?}", channel.framed);
            assert!(debug.contains("Wire"));
            assert!(!debug.contains("PRIVATE KEY"));
        } else {
            assert_eq!(
                result.unwrap_err().kind(),
                if fault == 1 {
                    ErrorKind::Trust
                } else {
                    ErrorKind::InvalidConfig
                }
            );
        }
    }
}

#[test]
fn opcua_secure_transport_limits_and_eof_keep_their_error_categories() {
    let fixture = Fixture::new();
    for fault in 0..2 {
        let mut reactor = fixture.peer(MessageSecurity::Sign).reactor(|_, _| {});
        let (driver, clock, _) = driver(move |request| {
            let response = reactor(request);
            if request.starts_with(b"OPN") {
                if fault == 0 {
                    Reaction::Close
                } else {
                    Reaction::Reply([b"OPNF".as_slice(), &100_000_u32.to_le_bytes()].concat())
                }
            } else {
                response
            }
        });
        let error = connect(&driver, &clock, &fixture.uri(MessageSecurity::Sign, false))
            .err()
            .unwrap();
        assert_eq!(
            error.kind(),
            if fault == 0 {
                ErrorKind::Disconnected
            } else {
                ErrorKind::ResourceLimit
            }
        );
    }
}

#[test]
fn opcua_secure_session_proofs_cover_the_exact_client_certificate_chain() {
    let mut fixture = Fixture::new();
    let mut pem = super::pki::pem("CERTIFICATE", &fixture.client_certificate);
    pem.push_str(&super::pki::pem("CERTIFICATE", &fixture.server_certificate));
    pem.push_str(&super::pki::pem("PRIVATE KEY", super::pki::pkcs8_2048()));
    fixture.client_store = fixture.scratch.write("chain.pem", &pem);
    let peer = fixture.peer(MessageSecurity::SignEncrypt);
    let probe = peer.probe.clone();
    let (driver, clock, _) = driver(peer.reactor(|_, _| {}));
    let connection = connect(
        &driver,
        &clock,
        &fixture.uri(MessageSecurity::SignEncrypt, true),
    )
    .unwrap();
    let requests = probe.requests();
    let Service::CreateSessionRequest {
        client_certificate, ..
    } = requests
        .iter()
        .find(|request| matches!(request, Service::CreateSessionRequest { .. }))
        .unwrap()
    else {
        panic!("create")
    };
    assert_eq!(
        client_certificate.as_bytes().unwrap(),
        [fixture.client_certificate, fixture.server_certificate].concat()
    );
    assert!(activated(&probe));
    block_on_with(&clock, connection.close()).unwrap();
}
