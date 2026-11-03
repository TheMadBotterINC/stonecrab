use std::error::Error;
use std::task::{Context, Poll, Waker};

use super::*;
use crate::ResourceLimits;
use crate::codec::BoundedReader;
use crate::opcua::codec::{DateTime, Depth, UaString};
use crate::opcua::codec::{Scalar, VariantValue};
use crate::opcua::generated::timestamps_to_return::TimestampsToReturn;
use crate::opcua::service::StatusFault;
use crate::testkit::{
    FakeClock, FakeTransport, FakeTransportProbe, Reaction, Script, Step, block_on_with,
};
use crate::transport::Framer;

use crate::opcua::responder::{message, read_response, server_error, service_fault};

struct Rig {
    lane: Lane,
    clock: FakeClock,
    probe: FakeTransportProbe,
    token: NodeId,
    cancel: CancellationToken,
}

impl Rig {
    fn new(script: Script, max_pending: usize) -> Self {
        let clock = FakeClock::default();
        let limits = ResourceLimits::builder()
            .max_pending_requests(max_pending)
            .build()
            .unwrap();
        let transport = FakeTransport::new(script, clock.clone(), limits);
        let probe = transport.probe();
        let framed = FramedTransport::new(Box::new(transport), OpcuaFramer::new(65535, limits));
        let lane = Lane::new(
            framed,
            Arc::new(clock.clone()),
            Duration::from_secs(30),
            17,
            29,
        );
        Self {
            lane,
            clock,
            probe,
            token: NodeId::numeric(2, 42),
            cancel: CancellationToken::new(),
        }
    }
    fn request(&self) -> OpFuture<'_, Service> {
        self.lane
            .exchange(&self.token, 631, read, None, &self.cancel)
    }
    fn reply(value: i32) -> Script {
        Script::strict(vec![Step::respond(move |_| {
            Some(message(1, 1, 17, 29, &read_response(1, value)))
        })])
    }
}

fn read(header: RequestHeader) -> Service {
    Service::ReadRequest {
        request_header: header,
        max_age: 0.0,
        timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
        nodes_to_read: None,
    }
}
fn poll<T>(future: &mut OpFuture<'_, T>) -> Poll<Result<T>> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}
fn ready<T>(future: &mut OpFuture<'_, T>) -> Result<T> {
    match poll(future) {
        Poll::Ready(result) => result,
        Poll::Pending => panic!("expected completion"),
    }
}
fn value(reply: Service) -> i32 {
    let Service::ReadResponse { results, .. } = reply else {
        panic!("read response")
    };
    let result = results.unwrap().remove(0);
    let variant = result.value.unwrap();
    let VariantValue::Scalar(Scalar::Int32(value)) = variant.value else {
        panic!("Int32 scalar")
    };
    value
}
fn status_source(problem: &PlcError) -> &StatusFault {
    let mut source: &(dyn Error + 'static) = problem;
    loop {
        if let Some(status) = source.downcast_ref::<StatusFault>() {
            return status;
        }
        source = source.source().expect("status retained in source chain");
    }
}

#[test]
fn opcua_a_response_is_matched_by_request_id() {
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::react(move |_| {
            seen += 1;
            if seen == 1 {
                Reaction::Silence
            } else {
                Reaction::Reply(
                    [
                        message(1, 2, 17, 29, &read_response(2, 200)),
                        message(2, 1, 17, 29, &read_response(1, 100)),
                    ]
                    .concat(),
                )
            }
        })]),
        2,
    );
    let mut first = rig.request();
    assert!(poll(&mut first).is_pending());
    let mut second = rig.request();
    assert!(
        poll(&mut second).is_pending(),
        "waiting read yields before another send"
    );
    assert!(poll(&mut first).is_pending());
    assert_eq!(value(ready(&mut second).unwrap()), 200);
    assert_eq!(value(ready(&mut first).unwrap()), 100);
    assert_eq!(rig.probe.sent().len(), 2);
    assert!(rig.lane.lock().pending.is_empty());
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_an_outstanding_request_does_not_delay_a_second_request() {
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::react(move |_| {
            seen += 1;
            if seen == 1 {
                Reaction::Silence
            } else {
                Reaction::Reply(message(1, 2, 17, 29, &read_response(2, 222)))
            }
        })]),
        2,
    );
    let mut first = rig.request();
    let mut second = rig.request();
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    assert!(poll(&mut first).is_pending());
    assert_eq!(value(ready(&mut second).unwrap()), 222);
    drop(first);
    assert!(rig.lane.lock().pending.is_empty());
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_request_header_carries_token_timestamp_handle_and_hint() {
    let rig = Rig::new(Rig::reply(5), 1);
    assert_eq!(value(block_on_with(&rig.clock, rig.request()).unwrap()), 5);
    let bytes = rig.probe.sent();
    let limits = ResourceLimits::builder().build().unwrap();
    let wire = OpcuaFramer::new(65535, limits)
        .decode(&mut bytes::BytesMut::from(bytes.first().unwrap().as_ref()))
        .unwrap()
        .unwrap();
    let Body::Message(chunk) = wire.body else {
        panic!("MSG request")
    };
    assert_eq!(
        chunk.sequence,
        SequenceHeader {
            sequence_number: 1,
            request_id: 1
        }
    );
    let mut reader = BoundedReader::new(&chunk.body);
    assert_eq!(
        NodeId::parse(&mut reader, Depth::new()).unwrap(),
        NodeId::numeric(0, 631)
    );
    let Service::ReadRequest { request_header, .. } =
        Service::parse(&mut reader, Depth::new(), 631).unwrap()
    else {
        panic!("ReadRequest")
    };
    reader.expect_eof().unwrap();
    assert_eq!(request_header.authentication_token, rig.token);
    assert_eq!(
        request_header.timestamp,
        DateTime::from_system_time(rig.clock.wall_now()).unwrap().0
    );
    assert_eq!(request_header.request_handle, 1);
    assert_eq!(request_header.timeout_hint, 30_000);
    assert_eq!(request_header.return_diagnostics, 0);
    assert_eq!(request_header.audit_entry_id, UaString::null());
}

#[test]
fn opcua_a_sequence_number_gap_fails_the_request() {
    let rig = Rig::new(
        Script::strict(vec![Step::respond(|_| {
            Some(
                [
                    message(7, 999, 17, 29, &[]),
                    message(9, 1, 17, 29, &read_response(1, 5)),
                ]
                .concat(),
            )
        })]),
        2,
    );
    let problem = block_on_with(&rig.clock, rig.request()).unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::MalformedData);
    assert!(problem.to_string().contains("sequence"));
    assert_eq!(rig.lane.lock().discarded, 1);
}

#[test]
fn opcua_a_response_with_an_unknown_request_id_is_discarded() {
    let rig = Rig::new(
        Script::strict(vec![Step::respond(|_| {
            Some(
                [
                    message(71, 123, 17, 29, &[0xAA]),
                    message(72, 1, 17, 29, &read_response(1, 7)),
                ]
                .concat(),
            )
        })]),
        1,
    );
    assert_eq!(value(block_on_with(&rig.clock, rig.request()).unwrap()), 7);
    assert_eq!(rig.lane.lock().discarded, 1);
}

#[test]
fn opcua_sequence_numbers_increase_by_one_per_chunk() {
    let mut id = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            id += 1;
            Some(message(id, id, 17, 29, &read_response(id, 1)))
        })]),
        1,
    );
    for expected in 1..=3 {
        assert_eq!(value(block_on_with(&rig.clock, rig.request()).unwrap()), 1);
        assert_eq!(rig.lane.lock().sent_sequence, expected);
    }
}

#[test]
fn opcua_a_chunk_with_a_foreign_channel_id_is_rejected() {
    for (channel, token) in [(99, 29), (17, 99)] {
        let rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| {
                Some(message(1, 1, channel, token, &read_response(1, 5)))
            })]),
            1,
        );
        assert_eq!(
            block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
            ErrorKind::MalformedData
        );
    }
}

#[test]
fn opcua_a_service_fault_is_a_device_error_with_the_status_name() {
    for code in [0x8034_0000, 0xDEAD_BEEF] {
        let rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| {
                Some(message(1, 1, 17, 29, &service_fault(1, code)))
            })]),
            1,
        );
        let problem = block_on_with(&rig.clock, rig.request()).unwrap_err();
        assert_eq!(problem.kind(), ErrorKind::DeviceException);
        let source = status_source(&problem);
        assert_eq!(source.code, code);
        assert_eq!(
            source.name,
            if code == 0x8034_0000 {
                "BadNodeIdUnknown"
            } else {
                "unknown"
            }
        );
        assert!(
            rig.lane.lock().failure.is_none(),
            "service faults leave the channel usable"
        );
    }
}

#[test]
fn opcua_a_service_fault_is_not_retried() {
    let rig = Rig::new(
        Script::strict(vec![Step::respond(|_| {
            Some(message(1, 1, 17, 29, &service_fault(1, 0x8001_0000)))
        })]),
        1,
    );
    assert_eq!(
        block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
        ErrorKind::DeviceException
    );
    assert_eq!(rig.probe.sent().len(), 1);
}

#[test]
fn opcua_a_server_err_message_fails_the_pending_request_with_its_status() {
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::react(move |_| {
            seen += 1;
            if seen == 1 {
                Reaction::Silence
            } else {
                Reaction::Reply(server_error(0x8001_0000))
            }
        })]),
        2,
    );
    let mut first = rig.request();
    let mut second = rig.request();
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    assert!(poll(&mut first).is_pending());
    let problem = ready(&mut second).unwrap_err();
    assert_eq!(status_source(&problem).code, 0x8001_0000);
    assert_eq!(
        ready(&mut first).unwrap_err().kind(),
        ErrorKind::DeviceException
    );
    assert_eq!(rig.clock.sleeper_count(), 0);
    assert!(rig.lane.lock().failure.is_some());
}

#[test]
fn opcua_read_times_out_after_request_timeout_on_fake_clock() {
    let rig = Rig::new(Script::default(), 1);
    let start = rig.clock.now();
    let problem = block_on_with(&rig.clock, rig.request()).unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::Timeout);
    assert_eq!(
        rig.clock.now().duration_since(start),
        Duration::from_secs(30)
    );
    assert!(rig.lane.lock().failure.is_none());
    assert_eq!(rig.clock.sleeper_count(), 0);
    assert!(rig.lane.lock().pending.is_empty());
}

#[test]
fn opcua_dropped_receive_restores_the_transport_and_releases_its_slot() {
    let rig = Rig::new(Script::default(), 1);
    let mut request = rig.request();
    assert!(poll(&mut request).is_pending());
    assert!(rig.clock.sleeper_count() > 0);
    drop(request);
    assert!(rig.lane.lock().framed.is_some());
    assert!(rig.lane.lock().pending.is_empty());
    assert_eq!(rig.clock.sleeper_count(), 0);
    assert!(rig.lane.lock().failure.is_none());
}

#[test]
fn opcua_full_pending_table_refuses_without_sending() {
    let rig = Rig::new(Script::default(), 1);
    let mut first = rig.request();
    assert!(poll(&mut first).is_pending());
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::Backpressure
    );
    assert_eq!(rig.probe.sent().len(), 1);
    drop(first);
    assert!(rig.lane.lock().pending.is_empty());
}

#[test]
fn opcua_cancelled_and_elapsed_requests_send_nothing() {
    let rig = Rig::new(Script::default(), 1);
    rig.cancel.cancel();
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::Cancelled
    );
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    let cancel = CancellationToken::new();
    let mut expired = rig.lane.exchange(
        &rig.token,
        631,
        read,
        Some(Deadline::at(rig.clock.now())),
        &cancel,
    );
    assert_eq!(ready(&mut expired).unwrap_err().kind(), ErrorKind::Timeout);
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
}

#[test]
fn opcua_cancelling_a_receive_keeps_other_requests_alive() {
    let rig = Rig::new(Script::default(), 2);
    let mut first = rig.request();
    assert!(poll(&mut first).is_pending());
    rig.cancel.cancel();
    assert_eq!(ready(&mut first).unwrap_err().kind(), ErrorKind::Cancelled);
    assert!(rig.lane.lock().failure.is_none());
    assert!(rig.lane.lock().pending.is_empty());
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_invalid_service_payloads_are_typed_and_never_replayed() {
    for body in [
        vec![],
        vec![3, 0, 0, 0, 0, 0, 0],
        vec![1, 2, 0, 0],
        vec![2, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF],
    ] {
        let rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| {
                Some(message(1, 1, 17, 29, &body))
            })]),
            1,
        );
        assert!(block_on_with(&rig.clock, rig.request()).is_err());
        assert_eq!(rig.probe.sent().len(), 1);
    }
}

#[test]
fn opcua_dropping_a_receive_preserves_a_partial_frame_for_the_next_caller() {
    let old = message(1, 1, 17, 29, &read_response(1, 111));
    let (prefix, suffix) = old.split_at(11);
    let prefix = prefix.to_vec();
    let suffix = suffix.to_vec();
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            seen += 1;
            Some(if seen == 1 {
                prefix.clone()
            } else {
                [
                    suffix.clone(),
                    message(2, 2, 17, 29, &read_response(2, 222)),
                ]
                .concat()
            })
        })]),
        1,
    );
    let mut first = rig.request();
    assert!(poll(&mut first).is_pending());
    drop(first);
    assert_eq!(rig.lane.lock().framed.as_ref().unwrap().buffered_len(), 11);
    assert_eq!(
        value(block_on_with(&rig.clock, rig.request()).unwrap()),
        222
    );
    assert_eq!(
        rig.lane.lock().discarded,
        1,
        "late abandoned response is never reassigned"
    );
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_a_queued_sender_can_be_cancelled_without_affecting_the_receiver() {
    let rig = Rig::new(Script::default(), 2);
    let mut first = rig.request();
    assert!(poll(&mut first).is_pending());
    let cancel = CancellationToken::new();
    let mut second = rig.lane.exchange(&rig.token, 631, read, None, &cancel);
    assert!(poll(&mut second).is_pending());
    cancel.cancel();
    assert_eq!(ready(&mut second).unwrap_err().kind(), ErrorKind::Cancelled);
    assert!(poll(&mut first).is_pending());
    assert_eq!(rig.probe.sent().len(), 1);
    assert_eq!(rig.lane.lock().pending.len(), 1);
    assert!(rig.lane.lock().failure.is_none());
    drop(first);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

struct SlowSend {
    inner: Box<dyn crate::transport::Transport>,
    clock: FakeClock,
}
impl crate::transport::Transport for SlowSend {
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            self.inner.send(bytes.slice(..8), deadline).await?;
            self.clock.sleep_until(deadline.instant()).await?;
            Err(error(ErrorKind::Timeout, "scripted partial send"))
        })
    }
    fn recv<'a>(
        &'a mut self,
        buffer: &'a mut bytes::BytesMut,
        deadline: Deadline,
    ) -> OpFuture<'a, usize> {
        self.inner.recv(buffer, deadline)
    }
    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        self.inner.shutdown(deadline)
    }
    fn peer(&self) -> &crate::Endpoint {
        self.inner.peer()
    }
    fn limits(&self) -> &ResourceLimits {
        self.inner.limits()
    }
}
fn slow_send(rig: &Rig) {
    let framed = rig.lane.lock().framed.take().unwrap();
    let limits = *framed.transport().limits();
    let slow = SlowSend {
        inner: framed.into_inner(),
        clock: rig.clock.clone(),
    };
    rig.lane.lock().framed = Some(FramedTransport::new(
        Box::new(slow),
        OpcuaFramer::new(65535, limits),
    ));
}

#[test]
fn opcua_dropping_a_partial_send_fails_every_pending_request() {
    let rig = Rig::new(Script::default(), 2);
    slow_send(&rig);
    let mut first = rig.request();
    let mut second = rig.request();
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    drop(first);
    assert_eq!(
        ready(&mut second).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(rig.probe.sent().first().unwrap().len(), 8);
    assert_eq!(rig.clock.sleeper_count(), 0);
    assert!(rig.lane.lock().pending.is_empty());
}

#[test]
fn opcua_a_partial_send_timeout_is_not_replayed_and_fails_the_channel() {
    let rig = Rig::new(Script::default(), 1);
    slow_send(&rig);
    assert_eq!(
        block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
        ErrorKind::Timeout
    );
    assert!(rig.lane.lock().failure.is_some());
    assert_eq!(rig.probe.sent().len(), 1);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_a_transport_loss_fails_the_lane_without_retry() {
    let rig = Rig::new(Script::strict(vec![Step::react(|_| Reaction::Close)]), 1);
    assert_eq!(
        block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(rig.probe.sent().len(), 1);
    assert!(rig.lane.lock().failure.is_some());
}

#[test]
fn opcua_invalid_chunk_shapes_are_rejected() {
    let acknowledge = [b"ACKF".to_vec(), 28_u32.to_le_bytes().to_vec(), vec![0; 20]].concat();
    for frame in [
        acknowledge,
        message(0, 1, 17, 29, &read_response(1, 1)),
        vec![0xFF; 8],
    ] {
        let rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| Some(frame.clone()))]),
            1,
        );
        assert_eq!(
            block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
            ErrorKind::MalformedData
        );
        assert!(rig.lane.lock().failure.is_some());
    }
}

#[test]
fn opcua_a_shorter_caller_deadline_limits_the_wire_hint() {
    let rig = Rig::new(Script::default(), 1);
    let start = rig.clock.now();
    let deadline = Deadline::after(start, Duration::from_millis(125)).unwrap();
    let request = rig.lane.exchange(
        &rig.token,
        631,
        |header| {
            assert_eq!(header.timeout_hint, 125);
            read(header)
        },
        Some(deadline),
        &rig.cancel,
    );
    assert_eq!(
        block_on_with(&rig.clock, request).unwrap_err().kind(),
        ErrorKind::Timeout
    );
    assert_eq!(
        rig.clock.now().duration_since(start),
        Duration::from_millis(125)
    );
}

#[test]
fn opcua_request_header_time_out_hint_carries_the_request_timeout() {
    let rig = Rig::new(Script::default(), 1);
    let start = rig.clock.now();
    let deadline = Deadline::after(start, Duration::from_secs(60)).unwrap();
    let request = rig.lane.exchange(
        &rig.token,
        631,
        |header| {
            assert_eq!(header.timeout_hint, 30_000);
            read(header)
        },
        Some(deadline),
        &rig.cancel,
    );
    assert_eq!(
        block_on_with(&rig.clock, request).unwrap_err().kind(),
        ErrorKind::Timeout
    );
    assert_eq!(
        rig.clock.now().duration_since(start),
        Duration::from_secs(30)
    );
}

#[test]
fn opcua_clock_and_encoding_failures_release_the_request_without_io() {
    let mut rig = Rig::new(Script::default(), 1);
    rig.lane.max_frame = 24;
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert!(rig.lane.lock().pending.is_empty());
    assert!(rig.lane.lock().failure.is_none());
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());

    let rig = Rig::new(Script::default(), 1);
    rig.clock.set_wall_now(
        std::time::SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_000_000_000_000))
            .unwrap(),
    );
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
    assert!(rig.lane.lock().pending.is_empty());
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
    fn sleep_until(&self, _deadline: std::time::Instant) -> OpFuture<'_, ()> {
        Box::pin(std::future::ready(if self.fails {
            Err(error(ErrorKind::InvalidConfig, "broken clock"))
        } else {
            Ok(())
        }))
    }
}

#[test]
fn opcua_a_clock_that_cannot_wait_is_rejected_before_sending() {
    for fails in [false, true] {
        let mut rig = Rig::new(Script::default(), 1);
        rig.lane.clock = Arc::new(EarlyClock {
            clock: rig.clock.clone(),
            fails,
        });
        assert_eq!(
            ready(&mut rig.request()).unwrap_err().kind(),
            ErrorKind::InvalidConfig
        );
        assert!(rig.lane.lock().pending.is_empty());
        assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    }
}

#[test]
fn opcua_identifier_exhaustion_never_wraps_or_reuses_a_pending_id() {
    let rig = Rig::new(Script::default(), 1);
    rig.lane.lock().request_id = u32::MAX;
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    rig.lane.lock().request_id = 0;
    rig.lane.lock().sent_sequence = u32::MAX;
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    assert!(rig.lane.lock().pending.is_empty());
    assert!(rig.lane.lock().framed.is_some());
}

#[test]
fn opcua_internal_lease_invariants_fail_with_typed_errors() {
    let rig = Rig::new(Script::default(), 1);
    let deadline = Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap();
    assert!(
        matches!(block_on_with(&rig.clock, rig.lane.acquire(10, false, deadline, &rig.cancel)), Err(problem) if problem.kind() == ErrorKind::Internal)
    );
    let mut empty = TransportLease {
        lane: &rig.lane,
        framed: None,
        interrupt: CancellationToken::new(),
        sending: false,
    };
    assert!(matches!(empty.framed(), Err(problem) if problem.kind() == ErrorKind::Internal));
}

#[test]
fn opcua_duplicate_and_unsolicited_responses_are_rejected() {
    let rig = Rig::new(Script::default(), 1);
    let _request = rig.lane.reserve().unwrap();
    let frame = |sequence| {
        Frame::single(Body::Message(SecureChunk {
            channel_id: 17,
            token_id: 29,
            sequence: SequenceHeader {
                sequence_number: sequence,
                request_id: 1,
            },
            body: Bytes::from(read_response(1, 42)),
        }))
    };
    assert_eq!(
        rig.lane.dispatch(frame(1)).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
    rig.lane.lock().pending.get_mut(&1).unwrap().queued = false;
    rig.lane.dispatch(frame(2)).unwrap();
    assert_eq!(
        rig.lane.dispatch(frame(3)).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
}

#[test]
fn opcua_a_completed_slot_does_not_transmit_another_request() {
    let rig = Rig::new(Script::default(), 1);
    let request = rig.lane.exchange(
        &rig.token,
        631,
        |header| {
            // Exercise the completion handoff before the caller acquires transport ownership.
            rig.lane
                .lock()
                .pending
                .get_mut(&header.request_handle)
                .unwrap()
                .reply = Some(decode_service(&read_response(header.request_handle, 42)));
            read(header)
        },
        None,
        &rig.cancel,
    );
    assert_eq!(value(block_on_with(&rig.clock, request).unwrap()), 42);
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    assert!(rig.lane.lock().pending.is_empty());
}

#[test]
fn opcua_lane_debug_reports_counts_and_never_session_tokens() {
    let rig = Rig::new(Script::default(), 1);
    let mut request = rig.request();
    assert!(poll(&mut request).is_pending());
    let rendered = format!("{:?}", rig.lane);
    assert!(rendered.contains("pending: 1"));
    assert!(rendered.contains("discarded: 0"));
    assert!(rendered.contains("failed: false"));
    assert!(!rendered.contains("42"));
}

struct Reenter {
    lane: std::sync::Weak<Lane>,
    wakes: Arc<std::sync::atomic::AtomicUsize>,
}
impl std::task::Wake for Reenter {
    fn wake(self: Arc<Self>) {
        self.wake_by_ref();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.wakes.fetch_add(1, Ordering::SeqCst);
        if let Some(lane) = self.lane.upgrade() {
            let _state = lane.lock();
        }
    }
}
impl Drop for Reenter {
    fn drop(&mut self) {
        if let Some(lane) = self.lane.upgrade() {
            let _state = lane.lock();
        }
    }
}

#[test]
fn opcua_send_and_reply_handoffs_wake_waiters_outside_the_lane_lock() {
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![Step::react(move |_| {
            seen += 1;
            if seen == 1 {
                Reaction::Silence
            } else {
                Reaction::Reply(
                    [
                        message(1, 2, 17, 29, &read_response(2, 2)),
                        message(2, 1, 17, 29, &read_response(1, 1)),
                    ]
                    .concat(),
                )
            }
        })]),
        2,
    );
    let lane = Arc::new(rig.lane);
    let wakes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let waker = Waker::from(Arc::new(Reenter {
        lane: Arc::downgrade(&lane),
        wakes: Arc::clone(&wakes),
    }));
    let mut cx = Context::from_waker(&waker);
    let mut first = lane.exchange(&rig.token, 631, read, None, &rig.cancel);
    let mut second = lane.exchange(&rig.token, 631, read, None, &rig.cancel);
    assert!(first.as_mut().poll(&mut cx).is_pending());
    assert!(second.as_mut().poll(&mut cx).is_pending());
    assert!(
        wakes.load(Ordering::SeqCst) > 0,
        "new sender wakes the receiver to yield"
    );
    assert!(first.as_mut().poll(&mut cx).is_pending());
    let Poll::Ready(Ok(reply)) = second.as_mut().poll(&mut cx) else {
        panic!("second reply is ready")
    };
    assert_eq!(value(reply), 2);
    let Poll::Ready(Ok(reply)) = first.as_mut().poll(&mut cx) else {
        panic!("first reply is ready")
    };
    assert_eq!(value(reply), 1);
    assert_eq!(lane.lock().waiters.len(), 0);
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_cancelling_one_request_does_not_cancel_another_pending_response() {
    let mut seen = 0;
    let rig = Rig::new(
        Script::strict(vec![
            Step::LateReply(Duration::from_millis(10)),
            Step::react(move |_| {
                seen += 1;
                if seen == 1 {
                    Reaction::Silence
                } else {
                    Reaction::Reply(message(1, 2, 17, 29, &read_response(2, 222)))
                }
            }),
        ]),
        2,
    );
    let other = CancellationToken::new();
    let mut first = rig.request();
    let mut second = rig.lane.exchange(&rig.token, 631, read, None, &other);
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    rig.cancel.cancel();
    assert_eq!(ready(&mut first).unwrap_err().kind(), ErrorKind::Cancelled);
    rig.clock.advance(Duration::from_millis(10));
    assert_eq!(value(ready(&mut second).unwrap()), 222);
    assert!(rig.lane.lock().failure.is_none());
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_an_oversized_local_request_does_not_consume_a_wire_sequence() {
    let rig = Rig::new(
        Script::strict(vec![Step::respond(|request| {
            let id = u32::from_le_bytes(request.get(20..24).unwrap().try_into().unwrap());
            Some(message(1, id, 17, 29, &read_response(id, 42)))
        })]),
        1,
    );
    assert_eq!(
        rig.lane.max_frame, 65535,
        "framer limit is smaller than transport limit"
    );
    let request = rig.lane.exchange(
        &rig.token,
        631,
        |mut header| {
            header.audit_entry_id = UaString::new("x".repeat(262_144));
            read(header)
        },
        None,
        &rig.cancel,
    );
    assert_eq!(
        block_on_with(&rig.clock, request).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.lane.lock().sent_sequence, 0);
    assert!(rig.lane.lock().failure.is_none());
    assert_eq!(rig.probe.sent(), Vec::<Bytes>::new());
    assert_eq!(value(block_on_with(&rig.clock, rig.request()).unwrap()), 42);
    assert_eq!(rig.lane.lock().sent_sequence, 1);
}

#[test]
fn opcua_lane_leaves_unexpected_service_types_for_the_session_to_reject() {
    let body = encode_service(
        458,
        &Service::SignatureData {
            algorithm: UaString::null(),
            signature: crate::opcua::codec::UaByteString::null(),
        },
        100,
    )
    .unwrap();
    let rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            Some(message(1, 1, 17, 29, &body))
        })]),
        1,
    );
    assert!(matches!(
        block_on_with(&rig.clock, rig.request()).unwrap(),
        Service::SignatureData { .. }
    ));
}

#[test]
fn opcua_lane_close_cannot_complete_from_an_unsolicited_reply() {
    let rig = Rig::new(Script::default(), 1);
    let framed = rig.lane.lock().framed.take();
    let deadline = Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap();
    let mut close = Box::pin(rig.lane.close(deadline, &rig.cancel));
    assert!(
        close
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut state = rig.lane.lock();
    let id = state.request_id;
    state.pending.get_mut(&id).unwrap().reply =
        Some(Err(error(ErrorKind::MalformedData, "unsolicited reply")));
    state.framed = framed;
    drop(state);
    assert_eq!(
        block_on_with(&rig.clock, close).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
    assert_eq!(rig.probe.sent().len(), 0);
}

#[test]
fn opcua_close_rejects_invalid_clock_and_undersized_body_before_sending() {
    let mut rig = Rig::new(Script::default(), 1);
    rig.clock.set_wall_now(
        std::time::SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_000_000_000_000))
            .unwrap(),
    );
    let deadline = Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap();
    assert!(block_on_with(&rig.clock, rig.lane.close(deadline, &rig.cancel)).is_err());
    assert_eq!(rig.probe.sent().len(), 0);
    rig.clock.set_wall_now(std::time::SystemTime::UNIX_EPOCH);
    rig.lane.max_frame = 24;
    assert_eq!(
        block_on_with(&rig.clock, rig.lane.close(deadline, &rig.cancel))
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent().len(), 0);
    assert_eq!(rig.lane.lock().pending.len(), 0);
}

fn chunk_wire(sequence: u32, request: u32, flag: u8, payload: &[u8]) -> Vec<u8> {
    let mut bytes = message(sequence, request, 17, 29, payload);
    *bytes.get_mut(3).unwrap() = flag;
    bytes
}
fn large_read(header: RequestHeader) -> Service {
    use crate::opcua::codec::QualifiedName;
    use crate::opcua::generated::read_value_id::ReadValueId;
    Service::ReadRequest {
        request_header: header,
        max_age: 0.0,
        timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
        nodes_to_read: Some(
            (0..1000)
                .map(|number| ReadValueId {
                    node_id: NodeId::numeric(2, 0xFFFF_0000_u32.checked_add(number).unwrap()),
                    attribute_id: 13,
                    index_range: UaString::null(),
                    data_encoding: QualifiedName {
                        namespace_index: 0,
                        name: UaString::null(),
                    },
                })
                .collect(),
        ),
    }
}

// covers: OPCUA-CHUNK-SEND / OPCUA-CHUNK-SIZE
#[test]
fn opcua_a_large_request_is_split_into_continue_chunks_and_one_final() {
    let mut rig = Rig::new(
        Script::strict(vec![Step::respond(|bytes| {
            bytes
                .starts_with(b"MSGF")
                .then(|| message(1, 1, 17, 29, &read_response(1, 42)))
        })]),
        1,
    );
    rig.lane.max_frame = 8192;
    let reply = block_on_with(
        &rig.clock,
        rig.lane
            .exchange(&rig.token, 631, large_read, None, &rig.cancel),
    )
    .unwrap();
    assert_eq!(value(reply), 42);
    let sent = rig.probe.sent();
    assert_eq!(sent.len(), 3);
    let mut body = Vec::new();
    for (index, bytes) in sent.iter().enumerate() {
        assert!(bytes.len() <= 8192);
        let frame = OpcuaFramer::new(8192, ResourceLimits::builder().build().unwrap())
            .decode(&mut bytes::BytesMut::from(bytes.as_ref()))
            .unwrap()
            .unwrap();
        assert_eq!(
            frame.chunk,
            if index == 2 {
                ChunkType::Final
            } else {
                ChunkType::Intermediate
            }
        );
        let Body::Message(chunk) = frame.body else {
            panic!("MSG")
        };
        assert_eq!(
            chunk.sequence.sequence_number,
            u32::try_from(index + 1).unwrap()
        );
        assert_eq!(chunk.sequence.request_id, 1);
        assert_eq!(chunk.channel_id, 17);
        assert_eq!(chunk.token_id, 29);
        body.extend_from_slice(&chunk.body);
    }
    let Service::ReadRequest { nodes_to_read, .. } = decode_service(&body).unwrap() else {
        panic!("ReadRequest")
    };
    let nodes = nodes_to_read.unwrap();
    assert_eq!(nodes.len(), 1000);
    for (index, node) in nodes.iter().enumerate() {
        assert_eq!(
            node.node_id,
            NodeId::numeric(2, 0xFFFF_0000 + u32::try_from(index).unwrap())
        );
    }
    assert_eq!(rig.lane.lock().sent_sequence, 3);
}

#[test]
fn opcua_a_request_over_the_negotiated_max_message_size_is_rejected_before_sending() {
    let mut rig = Rig::new(Script::default(), 1);
    rig.lane.message_limits.bytes = 10;
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent().len(), 0);
    assert_eq!(rig.lane.lock().sent_sequence, 0);
    assert!(rig.lane.lock().pending.is_empty());
    assert!(!rig.lane.is_failed());
}

#[test]
fn opcua_a_request_over_the_negotiated_max_chunk_count_is_rejected_before_sending() {
    let mut rig = Rig::new(Script::default(), 1);
    rig.lane.max_frame = 40;
    rig.lane.message_limits.chunks = 1;
    assert_eq!(
        ready(&mut rig.request()).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent().len(), 0);
    assert_eq!(rig.lane.lock().sent_sequence, 0);
    assert!(rig.lane.lock().pending.is_empty());
    assert!(!rig.lane.is_failed());
}

#[test]
fn opcua_chunked_responses_split_across_service_fields_decode_once_complete() {
    let body = read_response(1, -123_456);
    let replies = body
        .chunks(3)
        .enumerate()
        .map(|(index, part)| {
            chunk_wire(
                u32::try_from(index + 1).unwrap(),
                1,
                if (index + 1) * 3 >= body.len() {
                    b'F'
                } else {
                    b'C'
                },
                part,
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| Some(replies.clone()))]),
        1,
    );
    assert_eq!(
        value(block_on_with(&rig.clock, rig.request()).unwrap()),
        -123_456
    );
    assert!(rig.lane.lock().pending.is_empty());
}

// covers: OPCUA-DIV-ABORT-CHUNK / OPCUA-CHUNK-RECEIVE
#[test]
fn opcua_an_abort_chunk_fails_only_that_request() {
    let mut seen = 0;
    let abort = [0x8008_0000_u32.to_le_bytes().to_vec(), vec![0xFF; 4]].concat();
    let rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            seen += 1;
            Some(if seen == 1 {
                chunk_wire(1, 1, b'C', b"partial")
            } else {
                [
                    chunk_wire(2, 1, b'A', &abort),
                    message(3, 2, 17, 29, &read_response(2, 42)),
                ]
                .concat()
            })
        })]),
        2,
    );
    let mut first = rig.request();
    let mut second = rig.request();
    assert!(poll(&mut first).is_pending());
    assert_eq!(rig.lane.lock().pending.get(&1).unwrap().assembly.len(), 7);
    assert!(poll(&mut second).is_pending());
    assert!(poll(&mut first).is_pending());
    assert_eq!(value(ready(&mut second).unwrap()), 42);
    assert_eq!(
        status_source(&ready(&mut first).unwrap_err()).code,
        0x8008_0000
    );
    assert!(!rig.lane.is_failed());
    assert!(rig.lane.lock().pending.is_empty());
    assert_eq!(rig.clock.sleeper_count(), 0);
}

#[test]
fn opcua_abort_ignores_message_and_chunk_allowances_and_long_reason_text() {
    let abort = [
        0x8008_0000_u32.to_le_bytes().to_vec(),
        4097_i32.to_le_bytes().to_vec(),
        vec![b'x'; 4097],
    ]
    .concat();
    let mut rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            Some(
                [
                    chunk_wire(1, 1, b'C', b"partial"),
                    chunk_wire(2, 1, b'A', &abort),
                ]
                .concat(),
            )
        })]),
        1,
    );
    rig.lane.message_limits = MessageLimits {
        bytes: 128,
        chunks: 1,
    };
    let problem = block_on_with(&rig.clock, rig.request()).unwrap_err();
    assert_eq!(status_source(&problem).code, 0x8008_0000);
    assert!(!rig.lane.is_failed());
    assert!(rig.lane.lock().pending.is_empty());
}

#[test]
fn opcua_dropping_or_cancelling_a_partial_message_releases_only_its_assembly() {
    for cancel in [false, true] {
        let old = read_response(1, 111);
        let (prefix, suffix) = old.split_at(5);
        let first = chunk_wire(1, 1, b'C', prefix);
        let late = [
            chunk_wire(2, 1, b'F', suffix),
            message(3, 2, 17, 29, &read_response(2, 222)),
        ]
        .concat();
        let mut seen = 0;
        let rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| {
                seen += 1;
                Some(if seen == 1 {
                    first.clone()
                } else {
                    late.clone()
                })
            })]),
            1,
        );
        let token = CancellationToken::new();
        let mut first = rig.lane.exchange(&rig.token, 631, read, None, &token);
        assert!(poll(&mut first).is_pending());
        assert_eq!(rig.lane.lock().pending.get(&1).unwrap().assembly.len(), 5);
        if cancel {
            token.cancel();
            assert_eq!(ready(&mut first).unwrap_err().kind(), ErrorKind::Cancelled);
        }
        drop(first);
        assert!(rig.lane.lock().pending.is_empty());
        assert_eq!(
            value(block_on_with(&rig.clock, rig.request()).unwrap()),
            222
        );
        assert_eq!(rig.lane.lock().discarded, 1);
        assert!(!rig.lane.is_failed());
    }
}

#[test]
fn opcua_fragment_limits_fail_before_append_and_release_all_assemblies() {
    for (bytes, chunks) in [(128, 1), (60, 10)] {
        let reply = [
            chunk_wire(1, 1, b'C', &[0; 32]),
            chunk_wire(2, 1, b'F', &[0; 32]),
        ]
        .concat();
        let mut rig = Rig::new(
            Script::strict(vec![Step::respond(move |_| Some(reply.clone()))]),
            1,
        );
        rig.lane.message_limits = MessageLimits { bytes, chunks };
        assert_eq!(
            block_on_with(&rig.clock, rig.request()).unwrap_err().kind(),
            ErrorKind::ResourceLimit
        );
        assert!(rig.lane.is_failed());
        assert!(rig.lane.lock().pending.is_empty());
    }
}

#[test]
fn opcua_aggregate_partial_messages_stay_within_the_connection_receive_budget() {
    let mut seen = 0;
    let mut rig = Rig::new(
        Script::strict(vec![Step::respond(move |_| {
            seen += 1;
            Some(chunk_wire(seen, seen, b'C', b"123456"))
        })]),
        2,
    );
    rig.lane.max_buffered = 10;
    let mut first = rig.request();
    let mut second = rig.request();
    assert!(poll(&mut first).is_pending());
    assert!(poll(&mut second).is_pending());
    assert!(poll(&mut first).is_pending());
    assert_eq!(
        ready(&mut second).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.lane.lock().pending.get(&1).unwrap().assembly.len(), 0);
    assert_eq!(
        ready(&mut first).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert!(rig.lane.is_failed());
}

#[test]
fn opcua_sequence_exhaustion_for_a_whole_chunked_send_is_checked_before_io() {
    let mut rig = Rig::new(Script::default(), 1);
    rig.lane.max_frame = 8192;
    rig.lane.lock().sent_sequence = u32::MAX - 1;
    let request = rig
        .lane
        .exchange(&rig.token, 631, large_read, None, &rig.cancel);
    assert_eq!(
        block_on_with(&rig.clock, request).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.probe.sent().len(), 0);
    assert_eq!(rig.lane.lock().sent_sequence, u32::MAX - 1);
    assert!(!rig.lane.is_failed());
}

#[tokio::test]
#[ignore = "needs independent open62541; interop/open62541/interop.sh tests"]
#[expect(
    clippy::disallowed_macros,
    reason = "the interop test records non-secret byte/chunk evidence"
)]
async fn opcua_interop_large_array_read_chunks_under_none() {
    use crate::opcua::channel::Channel;
    use crate::opcua::codec::{Array, Identifier, QualifiedName};
    use crate::opcua::driver::OpcuaDriver;
    use crate::opcua::generated::read_value_id::ReadValueId;
    use crate::opcua::options::OpcuaOptions;
    use crate::opcua::session::Session;
    use crate::transport::{SystemClock, TcpFactory, TransportFactory};
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("interop URI");
    let config = OpcuaDriver::configuration(&format!("{uri}&encoding.receive-buffer-size=8192&encoding.send-buffer-size=8192&encoding.max-message-size=24576&encoding.max-chunk-count=4")).unwrap();
    let options = config.driver_options_as::<OpcuaOptions>().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
    let deadline = Deadline::after(clock.now(), Duration::from_secs(10)).unwrap();
    let cancel = CancellationToken::new();
    let transport = TcpFactory::default()
        .open(config.endpoint(), *config.limits(), deadline)
        .await
        .unwrap();
    let channel = Channel::open_none(
        transport,
        options,
        Arc::clone(&clock),
        deadline,
        &cancel,
        &crate::opcua::crypto::Native,
    )
    .await
    .unwrap();
    assert_eq!(channel.limits.receive_buffer_size, 8192);
    assert_eq!(channel.limits.send_buffer_size, 8192);
    assert_eq!(channel.limits.max_message_size, 24576);
    assert_eq!(channel.limits.max_chunk_count, 4);
    let session = Session::open(
        Lane::from_channel(channel, Duration::from_secs(5)),
        options,
        crate::opcua::identity::Identity::Anonymous,
        [7; 32],
        deadline,
        &cancel,
    )
    .await
    .unwrap();
    let make_node = |name: &str| ReadValueId {
        node_id: NodeId {
            namespace: 1,
            identifier: Identifier::String(UaString::new(name)),
        },
        attribute_id: 13,
        index_range: UaString::null(),
        data_encoding: QualifiedName {
            namespace_index: 0,
            name: UaString::null(),
        },
    };
    let before = session.lane.lock().received_sequence.unwrap();
    let reply = session
        .lane
        .exchange(
            &session.authentication_token,
            631,
            |header| Service::ReadRequest {
                request_header: header,
                max_age: 0.0,
                timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
                nodes_to_read: Some(vec![make_node("large/Int32_4000")]),
            },
            Some(deadline),
            &cancel,
        )
        .await
        .unwrap();
    let Service::ReadResponse {
        response_header,
        results,
        ..
    } = reply
    else {
        panic!("ReadResponse")
    };
    assert_eq!(response_header.service_result.0, 0);
    let results = results.unwrap();
    assert_eq!(results.len(), 1);
    let variant = results.first().unwrap().value.as_ref().unwrap();
    let VariantValue::Array(Array::Int32(Some(values))) = &variant.value else {
        panic!("Int32 array")
    };
    assert_eq!(values.len(), 4000);
    for (index, value) in values.iter().enumerate() {
        assert_eq!(*value, i32::try_from(index).unwrap());
    }
    let response_chunks = session
        .lane
        .lock()
        .received_sequence
        .unwrap()
        .checked_sub(before)
        .unwrap();
    assert!(response_chunks >= 2, "16 KB must span 8192-byte chunks");
    let before = session.lane.lock().sent_sequence;
    let reply = session
        .lane
        .exchange(
            &session.authentication_token,
            631,
            |header| Service::ReadRequest {
                request_header: header,
                max_age: 0.0,
                timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
                nodes_to_read: Some(vec![make_node("scalar/Int32"); 600]),
            },
            Some(deadline),
            &cancel,
        )
        .await
        .unwrap();
    let Service::ReadResponse {
        response_header,
        results,
        ..
    } = reply
    else {
        panic!("ReadResponse")
    };
    assert_eq!(response_header.service_result.0, 0);
    let results = results.unwrap();
    assert_eq!(results.len(), 600);
    for result in &results {
        assert_eq!(
            result.value.as_ref().unwrap().value,
            VariantValue::Scalar(Scalar::Int32(-123_456_789))
        );
    }
    let request_chunks = session
        .lane
        .lock()
        .sent_sequence
        .checked_sub(before)
        .unwrap();
    assert!(request_chunks >= 2, "600-node read request must be chunked");
    session.close(deadline, &cancel).await.unwrap();
    println!(
        "8192-byte negotiated chunks: 4000 Int32 values exact over {response_chunks} response chunks; 600-node Read exact over {request_chunks} request chunks; close passed"
    );
}
