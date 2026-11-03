use std::error::Error;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use super::*;
use crate::OpFuture;
use crate::codec::BoundedReader;
use crate::opcua::codec::Depth;
use crate::opcua::frame::SecureChunk;
use crate::opcua::generated::timestamps_to_return::TimestampsToReturn;
use crate::opcua::lane::Lane;
use crate::opcua::responder::{
    acknowledge, message, open, open_response, read_response, server_error, service_fault,
};
use crate::opcua::service::StatusFault;
use crate::testkit::{FakeClock, FakeTransport, FakeTransportProbe, Script, Step, block_on_with};
use crate::transport::Framer;
use bytes::Bytes;

struct Rig {
    transport: Option<Box<dyn Transport>>,
    clock: FakeClock,
    probe: FakeTransportProbe,
    options: OpcuaOptions,
    cancel: CancellationToken,
    deadline: Deadline,
}

impl Rig {
    fn new(script: Script) -> Self {
        let clock = FakeClock::default();
        let deadline = Deadline::after(clock.now(), Duration::from_secs(5)).unwrap();
        let transport = FakeTransport::new(
            script,
            clock.clone(),
            ResourceLimits::builder().build().unwrap(),
        );
        let probe = transport.probe();
        let options = OpcuaOptions::from_parameters(
            "opc.tcp://fake:4840/path".to_owned(),
            &[("security-policy".to_owned(), "NONE".to_owned())],
        )
        .unwrap();
        Self {
            transport: Some(Box::new(transport)),
            clock,
            probe,
            options,
            cancel: CancellationToken::new(),
            deadline,
        }
    }
    fn open(&mut self) -> Result<Channel> {
        block_on_with(
            &self.clock,
            Channel::open_none(
                self.transport.take().unwrap(),
                &self.options,
                Arc::new(self.clock.clone()),
                self.deadline,
                &self.cancel,
            ),
        )
    }
    fn healthy() -> Script {
        Script::strict(vec![Step::respond(|bytes| {
            if bytes.starts_with(b"HEL") {
                Some(acknowledge(16000, 24000, 100_000, 8))
            } else if bytes.starts_with(b"OPN") {
                Some(open_response(41, 1, 17, 29, 12345, 0))
            } else if bytes.starts_with(b"MSG") {
                Some(message(42, 2, 17, 29, &read_response(2, 42)))
            } else {
                None
            }
        })])
    }
}

fn frame(bytes: &[u8]) -> Frame {
    OpcuaFramer::new(65535, ResourceLimits::builder().build().unwrap())
        .decode(&mut bytes::BytesMut::from(bytes))
        .unwrap()
        .unwrap()
}
fn service(bytes: &[u8], id: i32) -> Service {
    let mut reader = BoundedReader::new(bytes);
    assert_eq!(
        NodeId::parse(&mut reader, Depth::new()).unwrap(),
        NodeId::numeric(0, u32::try_from(id).unwrap())
    );
    let result = Service::parse(&mut reader, Depth::new(), id).unwrap();
    reader.expect_eof().unwrap();
    result
}
fn read(header: crate::opcua::generated::request_header::RequestHeader) -> Service {
    Service::ReadRequest {
        request_header: header,
        max_age: 0.0,
        timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
        nodes_to_read: None,
    }
}
fn device_status(problem: &PlcError) -> u32 {
    let mut source: &(dyn Error + 'static) = problem;
    loop {
        if let Some(status) = source.downcast_ref::<StatusFault>() {
            return status.code;
        }
        source = source.source().expect("typed status source");
    }
}

#[test]
fn opcua_hello_limits_merge_to_the_smaller_of_client_and_server() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    assert_eq!(
        channel.limits,
        TransportLimits {
            receive_buffer_size: 24000,
            send_buffer_size: 16000,
            max_message_size: 100_000,
            max_chunk_count: 8
        }
    );
    assert_eq!(
        channel.framed.max_frame_bytes(),
        24000,
        "trait maximum spans both directions"
    );
}

#[test]
fn opcua_a_zero_server_limit_means_no_limit() {
    let client = proposed_limits(ResourceLimits::builder().build().unwrap(), 65535).unwrap();
    let server = TransportLimits {
        receive_buffer_size: 100_000,
        send_buffer_size: 100_000,
        max_message_size: 0,
        max_chunk_count: 0,
    };
    assert_eq!(negotiate(client, server).unwrap(), client);
    assert_eq!(smaller_limit(0, 50), 50);
    assert_eq!(smaller_limit(0, 0), 0);
    assert_eq!(body_budget(server, 24), 99976);
}

#[test]
fn opcua_hello_carries_the_endpoint_url() {
    let mut rig = Rig::new(Rig::healthy());
    rig.open().unwrap();
    let sent = rig.probe.sent();
    let Body::Hello(hello) = frame(sent.first().unwrap()).body else {
        panic!("HEL")
    };
    assert_eq!(
        hello.endpoint_url.text(),
        Some(Ok("opc.tcp://fake:4840/path"))
    );
    assert_eq!(hello.protocol_version, 0);
    assert_eq!(hello.limits.receive_buffer_size, 65535);
    assert_eq!(hello.limits.send_buffer_size, 65535);
}

#[test]
fn opcua_hello_advertises_the_configured_receive_buffer_within_the_resource_cap() {
    let mut rig = Rig::new(Rig::healthy());
    rig.options.receive_buffer_size = 8192;
    let channel = rig.open().unwrap();
    let sent = rig.probe.sent();
    let Body::Hello(hello) = frame(sent.first().unwrap()).body else {
        panic!("HEL")
    };
    assert_eq!(hello.limits.receive_buffer_size, 8192);
    assert_eq!(hello.limits.send_buffer_size, 65535);
    assert_eq!(channel.limits.receive_buffer_size, 8192);
    assert_eq!(channel.limits.send_buffer_size, 16000);
    let resources = ResourceLimits::builder()
        .max_frame_bytes(8192)
        .build()
        .unwrap();
    assert_eq!(
        proposed_limits(resources, 65535)
            .unwrap()
            .receive_buffer_size,
        8192
    );
}

#[test]
fn opcua_open_secure_channel_issue_request_carries_mode_nonce_and_lifetime() {
    let mut rig = Rig::new(Rig::healthy());
    rig.open().unwrap();
    let sent = rig.probe.sent();
    let Body::Open(chunk) = frame(sent.get(1).unwrap()).body else {
        panic!("OPN")
    };
    assert_eq!(chunk.channel_id, 0);
    assert_eq!(
        chunk.sequence,
        SequenceHeader {
            sequence_number: 1,
            request_id: 1
        }
    );
    assert_eq!(chunk.security, AsymmetricHeader::none());
    let Service::OpenSecureChannelRequest {
        request_header,
        client_protocol_version,
        request_type,
        security_mode,
        client_nonce,
        requested_lifetime,
    } = service(&chunk.body, 446)
    else {
        panic!("OPN Issue")
    };
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 0));
    assert_eq!(request_header.request_handle, 1);
    assert_eq!(request_header.timeout_hint, 5000);
    assert_eq!(client_protocol_version, 0);
    assert_eq!(
        request_type,
        SecurityTokenRequestType::SecurityTokenRequestTypeIssue
    );
    assert_eq!(security_mode, MessageSecurityMode::MessageSecurityModeNone);
    assert_eq!(client_nonce.as_bytes(), None, "null nonce on the wire");
    assert_eq!(requested_lifetime, 3_600_000);
}

#[test]
fn opcua_open_secure_channel_response_sets_channel_id_token_id_and_lifetime() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    assert_eq!(channel.token.channel_id, 17);
    assert_eq!(channel.token.token_id, 29);
    assert_eq!(channel.token.revised_lifetime, 12345);
    assert_eq!(channel.token.created_at, 1000);
    assert_eq!(
        channel.server_nonce.as_bytes(),
        Some(b"server-nonce".as_slice())
    );
    assert_eq!(channel.received_sequence, 41);
}

#[test]
fn opcua_policy_none_ignores_the_configured_message_security() {
    for mode in ["NONE", "SIGN", "SIGN_ENCRYPT"] {
        let mut rig = Rig::new(Rig::healthy());
        rig.options = OpcuaOptions::from_parameters(
            rig.options.endpoint_url.clone(),
            &[
                ("security-policy".to_owned(), "NONE".to_owned()),
                ("message-security".to_owned(), mode.to_owned()),
            ],
        )
        .unwrap();
        let channel = rig.open().unwrap();
        assert_eq!(channel.configured_mode.name(), mode);
        let shown = format!("{channel:?}");
        assert!(shown.contains("effective_message_security: \"NONE\""));
        assert!(!shown.contains("server-nonce"));
        let sent = rig.probe.sent();
        let Body::Open(chunk) = frame(sent.get(1).unwrap()).body else {
            panic!("OPN")
        };
        assert!(matches!(
            service(&chunk.body, 446),
            Service::OpenSecureChannelRequest {
                security_mode: MessageSecurityMode::MessageSecurityModeNone,
                ..
            }
        ));
    }
}

#[test]
fn opcua_policy_none_sends_unsigned_unencrypted_chunks() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    block_on_with(
        &rig.clock,
        Lane::from_channel(channel, Duration::from_secs(30)).close(rig.deadline, &rig.cancel),
    )
    .unwrap();
    let sent = rig.probe.sent();
    let Body::Close(chunk) = frame(sent.get(2).unwrap()).body else {
        panic!("CLO")
    };
    assert_eq!(chunk.channel_id, 17);
    assert_eq!(chunk.token_id, 29);
    assert_eq!(
        chunk.sequence,
        SequenceHeader {
            sequence_number: 2,
            request_id: 2
        }
    );
    let Service::CloseSecureChannelRequest { request_header } = service(&chunk.body, 452) else {
        panic!("close service")
    };
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 0));
    assert_eq!(request_header.request_handle, 2);
    assert_eq!(
        sent.get(2).unwrap().len(),
        24 + chunk.body.len(),
        "no signature or ciphertext padding"
    );
}

#[test]
fn opcua_close_secure_channel_expects_no_reply() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    let now = rig.clock.now();
    block_on_with(
        &rig.clock,
        Lane::from_channel(channel, Duration::from_secs(30)).close(rig.deadline, &rig.cancel),
    )
    .unwrap();
    assert_eq!(rig.clock.now(), now);
    assert!(rig.probe.is_shut_down());
    assert_eq!(rig.probe.sent().len(), 3);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_the_request_lane_continues_the_channel_counters() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    let lane = Lane::from_channel(channel, Duration::from_secs(30));
    let token = NodeId::numeric(0, 0);
    let reply = block_on_with(
        &rig.clock,
        lane.exchange(&token, 631, read, None, &rig.cancel),
    )
    .unwrap();
    assert!(matches!(reply, Service::ReadResponse { .. }));
    let sent = rig.probe.sent();
    let Body::Message(chunk) = frame(sent.get(2).unwrap()).body else {
        panic!("MSG")
    };
    assert_eq!(
        chunk.sequence,
        SequenceHeader {
            sequence_number: 2,
            request_id: 2
        }
    );
    assert_eq!(chunk.channel_id, 17);
    assert_eq!(chunk.token_id, 29);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_an_err_message_during_hello_fails_the_connect() {
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|_| {
        Some(server_error(0x8083_0000))
    })]));
    let problem = rig.open().unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::DeviceException);
    assert_eq!(device_status(&problem), 0x8083_0000);
    assert_eq!(rig.probe.sent().len(), 1);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_hello_times_out_at_the_handshake_timeout() {
    let mut rig = Rig::new(Script::default());
    let now = rig.clock.now();
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::Timeout);
    assert_eq!(rig.clock.now().duration_since(now), Duration::from_secs(5));
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_a_silent_open_secure_channel_times_out_on_the_same_deadline() {
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            Some(acknowledge(65535, 65535, 0, 0))
        } else {
            None
        }
    })]));
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::Timeout);
    assert_eq!(rig.probe.sent().len(), 2);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_a_delayed_ack_does_not_restart_the_handshake_deadline() {
    let mut rig = Rig::new(Script::strict(vec![
        Step::LateReply(Duration::from_secs(2)),
        Step::respond(|bytes| {
            if bytes.starts_with(b"HEL") {
                Some(acknowledge(65535, 65535, 0, 0))
            } else {
                None
            }
        }),
    ]));
    let start = rig.clock.now();
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::Timeout);
    assert_eq!(
        rig.clock.now().duration_since(start),
        Duration::from_secs(5)
    );
    let sent = rig.probe.sent();
    let Body::Open(chunk) = frame(sent.get(1).unwrap()).body else {
        panic!("OPN")
    };
    let Service::OpenSecureChannelRequest { request_header, .. } = service(&chunk.body, 446) else {
        panic!("Issue")
    };
    assert_eq!(request_header.timeout_hint, 3000);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_none_channel_never_falls_back_from_a_secure_policy() {
    let mut rig = Rig::new(Rig::healthy());
    rig.options.security_policy = SecurityPolicy::Basic256Sha256;
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::InvalidConfig);
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
}

#[test]
fn opcua_cancelled_and_expired_handshakes_send_nothing() {
    let mut rig = Rig::new(Rig::healthy());
    rig.cancel.cancel();
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::Cancelled);
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    let mut rig = Rig::new(Rig::healthy());
    rig.deadline = Deadline::at(rig.clock.now());
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::Timeout);
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
}

#[test]
fn opcua_cancelling_a_pending_handshake_releases_all_timers() {
    let mut rig = Rig::new(Script::default());
    let mut pending = Box::pin(Channel::open_none(
        rig.transport.take().unwrap(),
        &rig.options,
        Arc::new(rig.clock.clone()),
        rig.deadline,
        &rig.cancel,
    ));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    rig.cancel.cancel();
    assert!(
        matches!(pending.as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Ready(Err(problem)) if problem.kind() == ErrorKind::Cancelled)
    );
    drop(pending);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_negotiation_rejects_invalid_buffers_and_protocol_versions() {
    let proposed = proposed_limits(ResourceLimits::builder().build().unwrap(), 65535).unwrap();
    for (receive, send) in [(0, 8192), (8192, 0), (8191, 8192), (8192, 1)] {
        assert_eq!(
            negotiate(
                proposed,
                TransportLimits {
                    receive_buffer_size: receive,
                    send_buffer_size: send,
                    ..proposed
                }
            )
            .unwrap_err()
            .kind(),
            ErrorKind::MalformedData
        );
    }
    assert_eq!(
        proposed_limits(
            ResourceLimits::builder()
                .max_frame_bytes(8191)
                .build()
                .unwrap(),
            65535
        )
        .unwrap_err()
        .kind(),
        ErrorKind::InvalidConfig
    );
    let mut bad = acknowledge(65535, 65535, 0, 0);
    *bad.get_mut(8).unwrap() = 1;
    let mut rig = Rig::new(Script::strict(vec![Step::respond(move |_| {
        Some(bad.clone())
    })]));
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::MalformedData);
}

#[test]
fn opcua_a_channel_fault_preserves_its_status() {
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            Some(acknowledge(65535, 65535, 0, 0))
        } else {
            Some(open(1, 1, 0, &service_fault(1, 0x8001_0000)))
        }
    })]));
    let problem = rig.open().unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::DeviceException);
    assert_eq!(device_status(&problem), 0x8001_0000);
}

#[test]
fn opcua_invalid_handshake_message_types_and_chunk_flags_are_rejected() {
    let mut continued = acknowledge(65535, 65535, 0, 0);
    *continued.get_mut(3).unwrap() = b'C';
    for hello_reply in [continued, open_response(1, 1, 17, 29, 12345, 0)] {
        let mut rig = Rig::new(Script::strict(vec![Step::respond(move |_| {
            Some(hello_reply.clone())
        })]));
        assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::MalformedData);
    }
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|_| {
        Some(acknowledge(65535, 65535, 0, 0))
    })]));
    assert_eq!(
        rig.open().unwrap_err().kind(),
        ErrorKind::MalformedData,
        "a second ACK is not an OPN response"
    );
}

#[test]
fn opcua_an_err_during_open_secure_channel_keeps_its_status() {
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            Some(acknowledge(65535, 65535, 0, 0))
        } else {
            Some(server_error(0x8083_0000))
        }
    })]));
    assert_eq!(device_status(&rig.open().unwrap_err()), 0x8083_0000);
}

#[test]
fn opcua_open_secure_channel_rejects_foreign_ids_policy_and_zero_tokens() {
    let normal = open_response(41, 1, 17, 29, 12345, 0);
    let mut bad_policy = normal.clone();
    *bad_policy.get_mut(62).unwrap() = b'X';
    let mut bad_channel = normal.clone();
    *bad_channel.get_mut(8).unwrap() = 18;
    let mut bad_handle = normal.clone();
    *bad_handle.get_mut(91).unwrap() = 2;
    let mut bad_version = normal.clone();
    *bad_version.get_mut(107).unwrap() = 1;
    for response in [
        open_response(0, 1, 17, 29, 12345, 0),
        open_response(41, 9, 17, 29, 12345, 0),
        open_response(41, 1, 0, 29, 12345, 0),
        open_response(41, 1, 17, 0, 12345, 0),
        open_response(41, 1, 17, 29, 0, 0),
        bad_policy,
        bad_channel,
        bad_handle,
        bad_version,
    ] {
        let mut rig = Rig::new(Script::strict(vec![Step::respond(move |bytes| {
            if bytes.starts_with(b"HEL") {
                Some(acknowledge(65535, 65535, 0, 0))
            } else {
                Some(response.clone())
            }
        })]));
        assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::MalformedData);
        assert_eq!(rig.probe.sent().len(), 2);
        assert_eq!(rig.clock.sleeper_count(), 0);
    }
}

#[test]
fn opcua_refused_open_preserves_status_even_with_an_empty_token() {
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            Some(acknowledge(65535, 65535, 0, 0))
        } else {
            Some(open_response(41, 1, 0, 0, 0, 0x8001_0000))
        }
    })]));
    assert_eq!(device_status(&rig.open().unwrap_err()), 0x8001_0000);
}

#[test]
fn opcua_open_rejects_the_wrong_service_and_bounds_bodies_before_decoding() {
    for (response, max_message, kind) in [
        (
            open(41, 1, 17, &read_response(1, 5)),
            200,
            ErrorKind::MalformedData,
        ),
        (
            open_response(41, 1, 17, 29, 12345, 0),
            60,
            ErrorKind::ResourceLimit,
        ),
    ] {
        let mut rig = Rig::new(Script::strict(vec![Step::respond(move |bytes| {
            if bytes.starts_with(b"HEL") {
                Some(acknowledge(65535, 65535, max_message, 0))
            } else {
                Some(response.clone())
            }
        })]));
        assert_eq!(rig.open().unwrap_err().kind(), kind);
    }
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|_| {
        Some(acknowledge(65535, 65535, 1, 1))
    })]));
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::ResourceLimit);
    assert_eq!(rig.probe.sent().len(), 1, "oversized local OPN is not sent");
}

#[test]
fn opcua_negotiated_framing_keeps_buffered_bytes_and_applies_both_directions() {
    let resources = ResourceLimits::builder().build().unwrap();
    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            // Coalesced bytes beyond ACK must not disappear during framer replacement.
            Some(
                [
                    acknowledge(8192, 8192, 0, 0),
                    [b"OPNF".to_vec(), 8193_u32.to_le_bytes().to_vec()].concat(),
                ]
                .concat(),
            )
        } else {
            None
        }
    })]));
    assert_eq!(rig.open().unwrap_err().kind(), ErrorKind::ResourceLimit);
    let payload = Bytes::from(vec![0; 9000]);
    let frame = Frame::single(Body::Message(SecureChunk {
        channel_id: 17,
        token_id: 29,
        sequence: SequenceHeader {
            sequence_number: 1,
            request_id: 1,
        },
        body: payload,
    }));
    let mut framer = OpcuaFramer::with_buffers(8192, 16000, resources);
    let mut wire = bytes::BytesMut::new();
    framer.encode(&frame, &mut wire).unwrap();
    assert!(matches!(
        framer.decode(&mut wire),
        Err(crate::transport::FrameError::Oversized(9024))
    ));
    let mut framer = OpcuaFramer::with_buffers(16000, 8192, resources);
    assert!(matches!(
        framer.encode(&frame, &mut bytes::BytesMut::new()),
        Err(crate::transport::FrameError::Oversized(9024))
    ));
    assert_eq!(framer.decode(&mut wire).unwrap(), Some(frame));
}

#[test]
fn opcua_the_lane_applies_negotiated_request_and_response_message_sizes() {
    let mut rig = Rig::new(Rig::healthy());
    let mut channel = rig.open().unwrap();
    channel.limits.max_message_size = 30;
    let lane = Lane::from_channel(channel, Duration::from_secs(30));
    let token = NodeId::numeric(0, 0);
    assert_eq!(
        block_on_with(
            &rig.clock,
            lane.exchange(&token, 631, read, None, &rig.cancel)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent().len(), 2);

    let mut rig = Rig::new(Script::strict(vec![Step::respond(|bytes| {
        if bytes.starts_with(b"HEL") {
            Some(acknowledge(65535, 65535, 100_000, 8))
        } else if bytes.starts_with(b"OPN") {
            Some(open_response(41, 1, 17, 29, 12345, 0))
        } else {
            Some(message(42, 2, 17, 29, &[0; 61]))
        }
    })]));
    let mut channel = rig.open().unwrap();
    channel.limits.max_message_size = 60;
    let lane = Lane::from_channel(channel, Duration::from_secs(30));
    assert_eq!(
        block_on_with(
            &rig.clock,
            lane.exchange(&token, 631, read, None, &rig.cancel)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        rig.probe.sent().len(),
        3,
        "response size is rejected before service decoding"
    );

    let mut rig = Rig::new(Rig::healthy());
    let mut channel = rig.open().unwrap();
    channel.limits.max_message_size = 0;
    let lane = Lane::from_channel(channel, Duration::from_secs(30));
    assert!(
        block_on_with(
            &rig.clock,
            lane.exchange(&token, 631, read, None, &rig.cancel)
        )
        .is_ok()
    );
}

#[test]
fn opcua_policy_none_uri_matches_upstream() {
    let header = AsymmetricHeader::none();
    assert_eq!(
        header.security_policy_uri.text(),
        Some(Ok("http://opcfoundation.org/UA/SecurityPolicy#None"))
    );
}

#[test]
fn opcua_policy_none_does_not_open_the_keystore() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    let sent = rig.probe.sent();
    let Body::Open(chunk) = frame(sent.get(1).unwrap()).body else {
        panic!("OPN")
    };
    assert_eq!(chunk.security.sender_certificate.as_bytes(), None);
    assert_eq!(chunk.security.receiver_thumbprint.as_bytes(), None);
    assert_eq!(
        channel.token.revised_lifetime, 12345,
        "opening requires no certificate material"
    );
}

#[test]
fn opcua_a_cancelled_or_expired_close_never_sends_clo() {
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    rig.cancel.cancel();
    assert_eq!(
        block_on_with(
            &rig.clock,
            Lane::from_channel(channel, Duration::from_secs(30)).close(rig.deadline, &rig.cancel)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Cancelled
    );
    assert_eq!(rig.probe.sent().len(), 2);
    let mut rig = Rig::new(Rig::healthy());
    let channel = rig.open().unwrap();
    assert_eq!(
        block_on_with(
            &rig.clock,
            Lane::from_channel(channel, Duration::from_secs(30))
                .close(Deadline::at(rig.clock.now()), &rig.cancel)
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(rig.probe.sent().len(), 2);
}

struct EarlyClock {
    clock: FakeClock,
    fails: bool,
}
impl Clock for EarlyClock {
    fn now(&self) -> std::time::Instant {
        self.clock.now()
    }
    fn wall_now(&self) -> std::time::SystemTime {
        self.clock.wall_now()
    }
    fn sleep_until(&self, _at: std::time::Instant) -> OpFuture<'_, ()> {
        Box::pin(std::future::ready(if self.fails {
            Err(error(ErrorKind::Internal, "clock failed"))
        } else {
            Ok(())
        }))
    }
}

#[test]
fn opcua_channel_rejects_clocks_that_cannot_wait() {
    for fails in [false, true] {
        let mut rig = Rig::new(Script::default());
        let clock = Arc::new(EarlyClock {
            clock: rig.clock.clone(),
            fails,
        });
        let problem = block_on_with(
            &rig.clock,
            Channel::open_none(
                rig.transport.take().unwrap(),
                &rig.options,
                clock,
                rig.deadline,
                &rig.cancel,
            ),
        )
        .unwrap_err();
        assert_eq!(
            problem.kind(),
            if fails {
                ErrorKind::Internal
            } else {
                ErrorKind::InvalidConfig
            }
        );
        assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    }
}

#[tokio::test]
#[expect(
    clippy::disallowed_macros,
    reason = "the ignored interop test records non-secret negotiated limits for evidence"
)]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_peer_answers_hello_and_opens_a_none_channel() {
    use crate::opcua::driver::OpcuaDriver;
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
    let channel = Channel::open_none(transport, options, Arc::clone(&clock), deadline, &cancel)
        .await
        .unwrap();
    assert!(channel.token.channel_id > 0);
    assert!(channel.token.token_id > 0);
    assert!(channel.token.revised_lifetime > 0);
    println!(
        "policy None HEL/ACK and OPN Issue passed; lifetime={}ms receive={} send={}",
        channel.token.revised_lifetime,
        channel.limits.receive_buffer_size,
        channel.limits.send_buffer_size
    );
    Lane::from_channel(channel, Duration::from_secs(30))
        .close(deadline, &cancel)
        .await
        .unwrap();
    let transport = TcpFactory::default()
        .open(config.endpoint(), *config.limits(), deadline)
        .await
        .unwrap();
    let channel = Channel::open_none(transport, options, Arc::clone(&clock), deadline, &cancel)
        .await
        .unwrap();
    let lane = Lane::from_channel(channel, config.timeouts().request);
    let token = NodeId::numeric(0, 0);
    let problem = lane
        .exchange(&token, 631, read, Some(deadline), &cancel)
        .await
        .unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::DeviceException);
    assert_eq!(
        device_status(&problem),
        0x8025_0000,
        "request without a session is refused after counter handoff"
    );
    println!(
        "CLO completed without a reply; continued lane counters reached the session-required service check"
    );
}
