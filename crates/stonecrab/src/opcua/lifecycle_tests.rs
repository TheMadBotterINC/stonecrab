//! Public-driver lifecycle tests on injected monotonic time and independent native TCP.

use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use super::codec::{NodeId, UaByteString};
use super::frame::{Body, Frame};
use super::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use super::generated::security_token_request_type::SecurityTokenRequestType;
use super::options::{MessageSecurity, SecurityPolicy};
use super::secure_responder::Fixture;
use super::{OpcuaAddress, OpcuaDriver};
use crate::testkit::{FakeClock, FakeTransportFactory, Reaction, Script, Step, block_on_with};
use crate::{Clock, Connection, Driver, ErrorKind, PlcValue, ReadRequest, Tag, TagId};

fn tag() -> Tag {
    Tag::new(
        TagId::new("value").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;i=42;DINT").unwrap()),
    )
}

fn read(connection: &dyn Connection) -> ReadRequest {
    ReadRequest::builder(connection.limits())
        .tag(tag())
        .build()
        .unwrap()
}

fn setup() -> (OpcuaDriver, FakeClock, FakeTransportFactory) {
    let clock = FakeClock::default();
    clock.set_wall_now(
        UNIX_EPOCH
            .checked_add(Duration::from_secs(super::pki::NOW))
            .unwrap(),
    );
    let factory = FakeTransportFactory::new(clock.clone());
    let driver = OpcuaDriver::new()
        .with_clock(Arc::new(clock.clone()))
        .with_transport_factory(Arc::new(factory.clone()));
    (driver, clock, factory)
}

fn short_uri(fixture: &Fixture, policy: SecurityPolicy, mode: MessageSecurity) -> String {
    format!(
        "{}&channel-lifetime-ms=1000&min-channel-lifetime-ms=1",
        fixture.uri(policy, mode, false)
    )
}

#[test]
fn opcua_renewal_is_due_at_75_percent_and_preserves_session_socket_and_counters() {
    for policy in [
        SecurityPolicy::Basic256Sha256,
        SecurityPolicy::Aes128Sha256RsaOaep,
        SecurityPolicy::Aes256Sha256RsaPss,
    ] {
        let fixture = Fixture::new();
        for mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
            let (driver, clock, factory) = setup();
            let peer = fixture.peer(policy, mode);
            let probe = peer.probe.clone();
            factory.push_script(Script::strict(vec![Step::react(peer.reactor(
                |_, reply| {
                    if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
                        security_token.revised_lifetime = 1000;
                    }
                },
            ))]));
            let connection = block_on_with(
                &clock,
                driver.connect(
                    driver
                        .parse_config(&short_uri(&fixture, policy, mode))
                        .unwrap(),
                ),
            )
            .unwrap();
            clock.advance(Duration::from_millis(749));
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            assert_eq!(
                probe
                    .requests()
                    .iter()
                    .filter(|r| matches!(r, Service::OpenSecureChannelRequest { .. }))
                    .count(),
                1
            );
            clock.advance(Duration::from_millis(1));
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            clock.advance(Duration::from_millis(750));
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            let requests = probe.requests();
            let opens: Vec<_> = requests
                .iter()
                .filter_map(|r| {
                    if let Service::OpenSecureChannelRequest {
                        request_header,
                        request_type,
                        client_nonce,
                        requested_lifetime,
                        ..
                    } = r
                    {
                        Some((
                            request_header,
                            request_type,
                            client_nonce,
                            requested_lifetime,
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(opens.len(), 3);
            for (index, (header, kind, nonce, lifetime)) in opens.iter().enumerate() {
                assert_eq!(
                    **kind,
                    if index == 0 {
                        SecurityTokenRequestType::SecurityTokenRequestTypeIssue
                    } else {
                        SecurityTokenRequestType::SecurityTokenRequestTypeRenew
                    }
                );
                assert_eq!(header.authentication_token, NodeId::numeric(0, 0));
                assert_eq!(**lifetime, 1000);
                assert_eq!(nonce.as_bytes().unwrap().len(), 32);
            }
            assert_ne!(opens.first().unwrap().2, opens.get(1).unwrap().2);
            assert_ne!(opens.get(1).unwrap().2, opens.get(2).unwrap().2);
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| matches!(r, Service::CreateSessionRequest { .. }))
                    .count(),
                1
            );
            assert_eq!(factory.opens().len(), 1);
            block_on_with(&clock, connection.close()).unwrap();
        }
    }
}

#[test]
fn opcua_idle_channel_and_session_expiry_repeat_the_handshake_before_the_next_read() {
    let fixture = Fixture::new();
    for channel_expired in [false, true] {
        let (driver, clock, factory) = setup();
        let peers = [
            fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt),
            fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt),
        ];
        let mut probes = Vec::new();
        for (index, peer) in peers.into_iter().enumerate() {
            probes.push(peer.probe.clone());
            factory.push_script(Script::strict(vec![Step::react(peer.reactor(
                move |_, reply| {
                    if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
                        security_token.revised_lifetime =
                            if channel_expired { 1000 } else { 10_000 };
                    }
                    if let Service::CreateSessionResponse {
                        authentication_token,
                        revised_session_timeout,
                        ..
                    } = reply
                    {
                        *authentication_token =
                            NodeId::numeric(0, if index == 0 { 77 } else { 88 });
                        *revised_session_timeout = if channel_expired { 10_000.0 } else { 1000.0 };
                    }
                },
            ))]));
        }
        let uri = format!(
            "{}&channel-lifetime-ms={}&min-channel-lifetime-ms=1",
            fixture.uri(
                SecurityPolicy::Basic256Sha256,
                MessageSecurity::SignEncrypt,
                false
            ),
            if channel_expired { 1000 } else { 10_000 }
        );
        let connection =
            block_on_with(&clock, driver.connect(driver.parse_config(&uri).unwrap())).unwrap();
        clock.advance(Duration::from_millis(1000));
        assert_eq!(
            factory.opens().len(),
            1,
            "idle time causes no background work"
        );
        let result = block_on_with(&clock, connection.read(read(&*connection))).unwrap();
        assert_eq!(result.iter_ok().count(), 1);
        assert_eq!(factory.opens().len(), 2);
        let requests = probes.get(1).unwrap().requests();
        let Service::ReadRequest { request_header, .. } = requests
            .iter()
            .find(|r| matches!(r, Service::ReadRequest { .. }))
            .unwrap()
        else {
            panic!("read")
        };
        assert_eq!(request_header.authentication_token, NodeId::numeric(0, 88));
        assert_eq!(
            probes
                .first()
                .unwrap()
                .requests()
                .iter()
                .filter(|r| matches!(r, Service::ReadRequest { .. }))
                .count(),
            0
        );
        block_on_with(&clock, connection.close()).unwrap();
    }
}

#[test]
fn opcua_failed_renewal_fails_the_channel_before_sending_the_user_request() {
    use super::codec::StatusCode;
    let fixture = Fixture::new();
    for fault in 0..11 {
        let (driver, clock, factory) = setup();
        let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
        let probe = peer.probe.clone();
        factory.push_script(Script::strict(vec![Step::react(peer.reactor(
            move |request, reply| {
                let Service::OpenSecureChannelRequest {
                    request_type,
                    client_nonce,
                    ..
                } = request
                else {
                    return;
                };
                let Service::OpenSecureChannelResponse {
                    response_header,
                    server_protocol_version,
                    security_token,
                    server_nonce,
                } = reply
                else {
                    panic!("open")
                };
                security_token.revised_lifetime = 1000;
                if *request_type != SecurityTokenRequestType::SecurityTokenRequestTypeRenew {
                    return;
                }
                match fault {
                    0 => response_header.service_result = StatusCode(0x8013_0000),
                    1 => security_token.token_id = 0,
                    2 => security_token.token_id = 29,
                    3 => security_token.channel_id = 18,
                    4 => *server_nonce = UaByteString::new(vec![1; 31]),
                    5 => *server_nonce = UaByteString::new(vec![0x55; 32]),
                    6 => *server_nonce = client_nonce.clone(),
                    7 => *server_nonce = UaByteString::new(vec![0; 32]),
                    8 => response_header.request_handle = 100,
                    _ => *server_protocol_version = 1,
                }
            },
        ))]));
        let connection = block_on_with(
            &clock,
            driver.connect(
                driver
                    .parse_config(&short_uri(
                        &fixture,
                        SecurityPolicy::Basic256Sha256,
                        MessageSecurity::SignEncrypt,
                    ))
                    .unwrap(),
            ),
        )
        .unwrap();
        clock.advance(Duration::from_millis(750));
        if fault == 10 {
            clock.set_wall_now(
                UNIX_EPOCH
                    .checked_add(Duration::from_secs(4_000_000_000))
                    .unwrap(),
            );
        }
        let error = block_on_with(&clock, connection.read(read(&*connection))).unwrap_err();
        let kind = if fault == 10 {
            ErrorKind::Trust
        } else if fault == 0 {
            ErrorKind::DeviceException
        } else if (4..=7).contains(&fault) {
            ErrorKind::Authentication
        } else {
            ErrorKind::MalformedData
        };
        assert_eq!(error.kind(), kind, "fault {fault}: {error}");
        assert_eq!(connection.state(), crate::ConnectionState::Failed(kind));
        assert_eq!(
            probe
                .requests()
                .iter()
                .filter(|r| matches!(r, Service::ReadRequest { .. }))
                .count(),
            0
        );
        let before = factory.probes().first().unwrap().sent().len();
        assert_eq!(
            block_on_with(&clock, connection.read(read(&*connection)))
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
        assert_eq!(factory.probes().first().unwrap().sent().len(), before);
    }
}

#[test]
fn opcua_dropping_an_inflight_renewal_never_resumes_with_the_old_token() {
    use std::task::{Context, Waker};
    let fixture = Fixture::new();
    let (driver, clock, factory) = setup();
    let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let probe = peer.probe.clone();
    let observed = probe.clone();
    let mut reactor = peer.reactor(|_, reply| {
        if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
            security_token.revised_lifetime = 1000;
        }
    });
    factory.push_script(Script::strict(vec![Step::react(move |wire| {
        let reply = reactor(wire);
        if matches!(
            observed.requests().last(),
            Some(Service::OpenSecureChannelRequest {
                request_type: SecurityTokenRequestType::SecurityTokenRequestTypeRenew,
                ..
            })
        ) {
            Reaction::Silence
        } else {
            reply
        }
    })]));
    let connection = block_on_with(
        &clock,
        driver.connect(
            driver
                .parse_config(&short_uri(
                    &fixture,
                    SecurityPolicy::Basic256Sha256,
                    MessageSecurity::SignEncrypt,
                ))
                .unwrap(),
        ),
    )
    .unwrap();
    clock.advance(Duration::from_millis(750));
    let mut pending = connection.read(read(&*connection));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(pending);
    let before = factory.probes().first().unwrap().sent().len();
    assert_eq!(
        block_on_with(&clock, connection.read(read(&*connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(factory.probes().first().unwrap().sent().len(), before);
    assert_eq!(
        probe
            .requests()
            .iter()
            .filter(|r| matches!(r, Service::ReadRequest { .. }))
            .count(),
        0
    );
}

#[derive(Default)]
struct NativeFactory {
    tcp: crate::transport::TcpFactory,
    opens: std::sync::atomic::AtomicUsize,
}

impl crate::transport::TransportFactory for NativeFactory {
    fn open<'a>(
        &'a self,
        endpoint: &'a crate::Endpoint,
        limits: crate::ResourceLimits,
        deadline: crate::Deadline,
    ) -> crate::OpFuture<'a, Box<dyn crate::transport::Transport>> {
        self.opens
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.tcp.open(endpoint, limits, deadline)
    }
}

fn native_uri(policy: &str, lifetime: u32, session: u32) -> (super::pki::Scratch, String) {
    let base = std::env::var("STONECRAB_INTEROP_OPEN62541").unwrap();
    let base = base.split('?').next().unwrap();
    let pki =
        std::path::PathBuf::from(std::env::var("STONECRAB_INTEROP_OPEN62541_PKI_DIR").unwrap());
    let scratch = super::pki::Scratch::new();
    let pem = format!(
        "{}{}",
        std::fs::read_to_string(pki.join("client.cert.pem")).unwrap(),
        std::fs::read_to_string(pki.join("client.key.pem")).unwrap()
    );
    let key = scratch.write("application.pem", &pem);
    let uri = format!(
        "{base}?security-policy={policy}&message-security=SIGN_ENCRYPT&discovery=false&channel-lifetime-ms={lifetime}&min-channel-lifetime-ms=1&session-timeout-ms={session}&tls.keystore={}&server-certificate-file={}",
        key.display(),
        pki.join("server.cert.der").display()
    );
    (scratch, uri)
}

fn native_read(connection: &dyn Connection) -> ReadRequest {
    ReadRequest::builder(connection.limits())
        .tag(Tag::new(
            TagId::new("counter").unwrap(),
            Arc::new(OpcuaAddress::parse("ns=1;s=counter;UDINT").unwrap()),
        ))
        .build()
        .unwrap()
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_renewal_survives_three_short_lifetimes_under_sign_encrypt() {
    for policy in [
        "Basic256Sha256",
        "Aes128_Sha256_RsaOaep",
        "Aes256_Sha256_RsaPss",
    ] {
        let (_scratch, uri) = native_uri(policy, 1500, 120_000);
        let factory = Arc::new(NativeFactory::default());
        let clock = Arc::new(crate::transport::SystemClock::default());
        let driver = OpcuaDriver::new()
            .with_transport_factory(Arc::<NativeFactory>::clone(&factory))
            .with_clock(Arc::<crate::transport::SystemClock>::clone(&clock));
        let connection = driver
            .connect(driver.parse_config(&uri).unwrap())
            .await
            .unwrap();
        let until = clock
            .now()
            .checked_add(Duration::from_millis(4800))
            .unwrap();
        while clock.now() < until {
            assert_eq!(
                connection
                    .read(native_read(&*connection))
                    .await
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            clock
                .sleep_until(clock.now().checked_add(Duration::from_millis(150)).unwrap())
                .await
                .unwrap();
        }
        assert_eq!(
            factory.opens.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "renewal must preserve the socket"
        );
        connection.close().await.unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_none_channel_renews_its_token_on_the_same_socket() {
    let (_scratch, uri) = native_uri("NONE", 1000, 120_000);
    let factory = Arc::new(NativeFactory::default());
    let clock = Arc::new(crate::transport::SystemClock::default());
    let driver = OpcuaDriver::new()
        .with_transport_factory(Arc::<NativeFactory>::clone(&factory))
        .with_clock(Arc::<crate::transport::SystemClock>::clone(&clock));
    let connection = driver
        .connect(driver.parse_config(&uri).unwrap())
        .await
        .unwrap();
    for _ in 0..5 {
        clock
            .sleep_until(clock.now().checked_add(Duration::from_millis(800)).unwrap())
            .await
            .unwrap();
        assert_eq!(
            connection
                .read(native_read(&*connection))
                .await
                .unwrap()
                .iter_ok()
                .count(),
            1
        );
    }
    assert_eq!(factory.opens.load(std::sync::atomic::Ordering::Relaxed), 1);
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_idle_channel_or_session_is_reopened_before_the_next_read() {
    for policy in ["NONE", "Basic256Sha256"] {
        for channel in [false, true] {
            let (_scratch, uri) = native_uri(
                policy,
                if channel { 1000 } else { 60_000 },
                if channel { 120_000 } else { 1000 },
            );
            let factory = Arc::new(NativeFactory::default());
            let clock = Arc::new(crate::transport::SystemClock::default());
            let driver = OpcuaDriver::new()
                .with_transport_factory(Arc::<NativeFactory>::clone(&factory))
                .with_clock(Arc::<crate::transport::SystemClock>::clone(&clock));
            let connection = driver
                .connect(driver.parse_config(&uri).unwrap())
                .await
                .unwrap();
            clock
                .sleep_until(
                    clock
                        .now()
                        .checked_add(Duration::from_millis(1500))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                factory.opens.load(std::sync::atomic::Ordering::Relaxed),
                1,
                "no work while idle"
            );
            assert_eq!(
                connection
                    .read(native_read(&*connection))
                    .await
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            assert_eq!(factory.opens.load(std::sync::atomic::Ordering::Relaxed), 2);
            connection.close().await.unwrap();
        }
    }
}

#[test]
fn opcua_previous_token_is_accepted_only_during_overlap_and_before_the_first_new_token_message() {
    use super::crypto::{Native, Symmetric};
    use super::frame::{SecureChunk, SequenceHeader};
    let fixture = Fixture::new();
    for expired in [false, true] {
        let (driver, clock, factory) = setup();
        let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
        let probe = peer.probe.clone();
        let mut reactor = peer.reactor(|_, reply| {
            if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
                security_token.revised_lifetime = 1000;
            }
        });
        let mut reads = 0_u32;
        let advance = clock.clone();
        factory.push_script(Script::strict(vec![Step::react(move |wire| {
            let reply = reactor(wire);
            let requests = probe.requests();
            let Some(Service::ReadRequest { request_header, .. }) = requests.last() else {
                return reply;
            };
            reads = reads.checked_add(1).unwrap();
            if reads != 1 && reads != 3 {
                return reply;
            }
            if expired {
                advance.advance(Duration::from_millis(500));
            }
            let Service::OpenSecureChannelRequest { client_nonce, .. } = requests.first().unwrap()
            else {
                panic!("open")
            };
            let old = Symmetric::new(
                &Native,
                SecurityPolicy::Basic256Sha256,
                MessageSecurity::SignEncrypt,
                17,
                29,
                client_nonce.as_bytes().unwrap(),
                &[0x55; 32],
            )
            .unwrap()
            .into_server();
            let frame = Frame::single(Body::Message(SecureChunk {
                channel_id: 17,
                token_id: 29,
                sequence: SequenceHeader {
                    sequence_number: 44_u32.checked_add(reads).unwrap(),
                    request_id: request_header.request_handle,
                },
                body: bytes::Bytes::from(super::responder::read_response(
                    request_header.request_handle,
                    101,
                )),
            }));
            Reaction::Reply(old.seal(&Native, &frame, 8192).unwrap().to_vec())
        })]));
        let connection = block_on_with(
            &clock,
            driver.connect(
                driver
                    .parse_config(&short_uri(
                        &fixture,
                        SecurityPolicy::Basic256Sha256,
                        MessageSecurity::SignEncrypt,
                    ))
                    .unwrap(),
            ),
        )
        .unwrap();
        clock.advance(Duration::from_millis(750));
        if !expired {
            let result = block_on_with(&clock, connection.read(read(&*connection))).unwrap();
            assert_eq!(
                result.items().first().unwrap().outcome().as_ref().unwrap(),
                &PlcValue::I32(101)
            );
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
        }
        assert_eq!(
            block_on_with(&clock, connection.read(read(&*connection)))
                .unwrap_err()
                .kind(),
            ErrorKind::Authentication
        );
        assert_eq!(factory.opens().len(), 1);
    }
}

#[test]
fn opcua_cancelled_dropped_or_timed_out_idle_recovery_can_be_retried_and_close_stops_it() {
    use std::task::{Context, Waker};
    let fixture = Fixture::new();
    for action in 0..4 {
        let (driver, clock, factory) = setup();
        for index in 0..2 {
            if index == 1 {
                factory.push_connect_timeout();
            }
            let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
            factory.push_script(Script::strict(vec![Step::react(peer.reactor(
                |_, reply| {
                    if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
                        security_token.revised_lifetime = 1000;
                    }
                },
            ))]));
        }
        let connection = block_on_with(
            &clock,
            driver.connect(
                driver
                    .parse_config(&short_uri(
                        &fixture,
                        SecurityPolicy::Basic256Sha256,
                        MessageSecurity::SignEncrypt,
                    ))
                    .unwrap(),
            ),
        )
        .unwrap();
        clock.advance(Duration::from_millis(1000));
        let cancel = crate::CancellationToken::new();
        let deadline = crate::Deadline::after(clock.now(), Duration::from_millis(10)).unwrap();
        let request = ReadRequest::builder(connection.limits())
            .tag(tag())
            .deadline(deadline)
            .cancel(cancel.clone())
            .build()
            .unwrap();
        let mut pending = connection.read(request);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        assert!(matches!(
            connection.state(),
            crate::ConnectionState::Reconnecting { .. }
        ));
        match action {
            0 => drop(pending),
            1 => {
                cancel.cancel();
                assert_eq!(
                    block_on_with(&clock, pending).unwrap_err().kind(),
                    ErrorKind::Cancelled
                );
            }
            2 => {
                clock.advance(Duration::from_millis(10));
                assert_eq!(
                    block_on_with(&clock, pending).unwrap_err().kind(),
                    ErrorKind::Timeout
                );
            }
            _ => {
                let _closed = block_on_with(&clock, connection.close());
                assert_eq!(
                    block_on_with(&clock, pending).unwrap_err().kind(),
                    ErrorKind::Disconnected
                );
                assert_eq!(connection.state(), crate::ConnectionState::Closed);
            }
        }
        assert_eq!(factory.opens().len(), 2);
        if action != 3 {
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            assert_eq!(factory.opens().len(), 3);
            assert_eq!(connection.state(), crate::ConnectionState::Connected);
            block_on_with(&clock, connection.close()).unwrap();
        }
    }
}

#[test]
fn opcua_idle_recovery_clears_metadata_and_never_replays_an_inflight_write() {
    use crate::WriteRequest;
    use std::task::{Context, Waker};
    let fixture = Fixture::new();
    for in_flight in [false, true] {
        let (driver, clock, factory) = setup();
        let mut probes = Vec::new();
        for index in 0..2 {
            let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
            let probe = peer.probe.clone();
            probes.push(probe.clone());
            let mut reactor = peer.reactor(move |_, reply| {
                if let Service::OpenSecureChannelResponse { security_token, .. } = reply {
                    security_token.revised_lifetime = 1000;
                }
                if index == 1
                    && let Service::CreateSessionResponse {
                        authentication_token,
                        ..
                    } = reply
                {
                    *authentication_token = NodeId::numeric(0, 88);
                }
            });
            factory.push_script(Script::strict(vec![Step::react(move |wire| {
                let reply = reactor(wire);
                if index == 0
                    && in_flight
                    && matches!(probe.requests().last(), Some(Service::WriteRequest { .. }))
                {
                    Reaction::Silence
                } else {
                    reply
                }
            })]));
        }
        let connection = block_on_with(
            &clock,
            driver.connect(
                driver
                    .parse_config(&short_uri(
                        &fixture,
                        SecurityPolicy::Basic256Sha256,
                        MessageSecurity::SignEncrypt,
                    ))
                    .unwrap(),
            ),
        )
        .unwrap();
        let write = |value| {
            WriteRequest::builder(connection.limits())
                .item(
                    Tag::new(
                        TagId::new("setpoint").unwrap(),
                        Arc::new(OpcuaAddress::parse("ns=1;i=42").unwrap()),
                    ),
                    PlcValue::I32(value),
                )
                .build()
                .unwrap()
        };
        if in_flight {
            let mut pending = connection.write(write(501));
            assert!(
                pending
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            clock.advance(Duration::from_millis(1000));
            assert_eq!(
                block_on_with(&clock, connection.read(read(&*connection)))
                    .unwrap()
                    .iter_ok()
                    .count(),
                1
            );
            assert_eq!(block_on_with(&clock, pending).unwrap().unknown_count(), 1);
            assert_eq!(
                probes
                    .get(1)
                    .unwrap()
                    .requests()
                    .iter()
                    .filter(|r| matches!(r, Service::WriteRequest { .. }))
                    .count(),
                0
            );
            assert_eq!(connection.state(), crate::ConnectionState::Connected);
        } else {
            for value in [1, 2] {
                assert_eq!(
                    block_on_with(&clock, connection.write(write(value)))
                        .unwrap()
                        .confirmed_count(),
                    1
                );
            }
            clock.advance(Duration::from_millis(1000));
            assert_eq!(
                block_on_with(&clock, connection.write(write(3)))
                    .unwrap()
                    .confirmed_count(),
                1
            );
            for probe in &probes {
                assert_eq!(probe.requests().iter().filter(|r| matches!(r, Service::ReadRequest { nodes_to_read: Some(nodes), .. } if nodes.iter().any(|node| node.attribute_id != 13))).count(), 1);
            }
            let requests = probes.get(1).unwrap().requests();
            let Service::WriteRequest { request_header, .. } = requests
                .iter()
                .find(|r| matches!(r, Service::WriteRequest { .. }))
                .unwrap()
            else {
                panic!("write")
            };
            assert_eq!(request_header.authentication_token, NodeId::numeric(0, 88));
        }
        assert_eq!(factory.opens().len(), 2);
        block_on_with(&clock, connection.close()).unwrap();
    }
}

#[test]
fn opcua_a_metadata_renewal_failure_is_fatal_before_any_write_is_transmitted() {
    let fixture = Fixture::new();
    let (driver, clock, factory) = setup();
    let peer = fixture.peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let probe = peer.probe.clone();
    factory.push_script(Script::strict(vec![Step::react(peer.reactor(
        |request, reply| {
            if let Service::OpenSecureChannelResponse {
                security_token,
                server_nonce,
                ..
            } = reply
            {
                security_token.revised_lifetime = 1000;
                if matches!(
                    request,
                    Service::OpenSecureChannelRequest {
                        request_type: SecurityTokenRequestType::SecurityTokenRequestTypeRenew,
                        ..
                    }
                ) {
                    *server_nonce = UaByteString::new(vec![0x55; 32]);
                }
            }
        },
    ))]));
    let connection = block_on_with(
        &clock,
        driver.connect(
            driver
                .parse_config(&short_uri(
                    &fixture,
                    SecurityPolicy::Basic256Sha256,
                    MessageSecurity::SignEncrypt,
                ))
                .unwrap(),
        ),
    )
    .unwrap();
    clock.advance(Duration::from_millis(750));
    let write = crate::WriteRequest::builder(connection.limits())
        .item(
            Tag::new(
                TagId::new("setpoint").unwrap(),
                Arc::new(OpcuaAddress::parse("ns=1;i=42").unwrap()),
            ),
            PlcValue::I32(501),
        )
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&clock, connection.write(write))
            .unwrap_err()
            .kind(),
        ErrorKind::Authentication
    );
    assert_eq!(
        connection.state(),
        crate::ConnectionState::Failed(ErrorKind::Authentication)
    );
    assert_eq!(
        probe
            .requests()
            .iter()
            .filter(|r| matches!(r, Service::WriteRequest { .. }))
            .count(),
        0
    );
}
