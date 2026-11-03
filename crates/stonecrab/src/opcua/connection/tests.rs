use super::*;
use crate::opcua::codec::NodeId;
use crate::opcua::frame::{Body, OpcuaFramer};
use crate::opcua::responder::{Device, close_response, int32_value, message, read_results};
use crate::opcua::service::decode_service;
use crate::opcua::{OpcuaAddress, OpcuaDriver};
use crate::testkit::{
    FakeClock, FakeTransportFactory, FixtureAddress, Reaction, Script, Step, block_on_with,
};
use crate::transport::Framer;
use crate::{Driver, PlcValue, ResourceLimits, RetryPolicy, Tag, TagId};
use bytes::BytesMut;
use std::num::NonZeroU32;
use std::task::{Context, Waker};

const URI: &str = "opcua://fake:4840/path?security-policy=NONE&request-timeout-ms=25";

struct Rig {
    driver: OpcuaDriver,
    clock: FakeClock,
    factory: FakeTransportFactory,
    device: Device,
}
impl Rig {
    fn new() -> Self {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let device = Device::default();
        factory.push_script(Script::strict(vec![Step::react(device.reactor())]));
        let driver = OpcuaDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        Self {
            driver,
            clock,
            factory,
            device,
        }
    }
    fn connect(&self) -> Box<dyn Connection> {
        block_on_with(
            &self.clock,
            self.driver
                .connect(self.driver.parse_config(URI).expect("config")),
        )
        .expect("connect")
    }
    fn frames(&self) -> usize {
        self.factory
            .probes()
            .iter()
            .map(|probe| probe.sent().len())
            .sum()
    }
}
fn tag(id: &str, number: u32) -> Tag {
    Tag::new(
        TagId::new(id).unwrap(),
        Arc::new(OpcuaAddress::parse(&format!("ns=2;i={number}")).unwrap()),
    )
}
fn request(connection: &dyn Connection, tags: Vec<Tag>) -> ReadRequest {
    tags.into_iter()
        .fold(
            ReadRequest::builder(connection.limits()),
            crate::ReadRequestBuilder::tag,
        )
        .build()
        .unwrap()
}
fn body(bytes: &[u8]) -> Service {
    let frame = OpcuaFramer::new(65535, ResourceLimits::builder().build().unwrap())
        .decode(&mut BytesMut::from(bytes))
        .unwrap()
        .unwrap();
    let Body::Message(chunk) = frame.body else {
        panic!("message")
    };
    decode_service(&chunk.body).unwrap()
}

#[test]
fn opcua_read_batches_all_tags_into_one_request_and_keeps_request_order() {
    let rig = Rig::new();
    let connection = rig.connect();
    let before = rig.frames();
    let response = block_on_with(
        &rig.clock,
        connection.read(request(
            &*connection,
            vec![tag("third", 3), tag("first", 1), tag("second", 2)],
        )),
    )
    .unwrap();
    assert_eq!(rig.frames(), before + 1);
    assert_eq!(rig.device.reads(), 1);
    assert_eq!(response.split_into(), NonZeroU32::MIN);
    assert_eq!(
        response
            .items()
            .iter()
            .map(|item| item.tag().as_str())
            .collect::<Vec<_>>(),
        ["third", "first", "second"]
    );
    assert_eq!(
        response
            .items()
            .iter()
            .map(|item| item.outcome().as_ref().unwrap())
            .collect::<Vec<_>>(),
        [&PlcValue::I32(3), &PlcValue::I32(1), &PlcValue::I32(2)]
    );
    let sent = rig.factory.probes().remove(0).sent();
    let Service::ReadRequest {
        request_header,
        max_age,
        timestamps_to_return,
        nodes_to_read,
    } = body(sent.last().unwrap())
    else {
        panic!("read")
    };
    assert_eq!(request_header.authentication_token, NodeId::numeric(0, 77));
    assert_eq!(request_header.timeout_hint, 25);
    assert_eq!(max_age, 0.0);
    assert_eq!(
        timestamps_to_return,
        TimestampsToReturn::TimestampsToReturnBoth
    );
    assert_eq!(nodes_to_read.unwrap().len(), 3);
    block_on_with(&rig.clock, connection.close()).unwrap();
}

#[test]
fn opcua_bad_node_id_unknown_is_not_found_without_losing_other_items() {
    let rig = Rig::new();
    let connection = rig.connect();
    let response = block_on_with(
        &rig.clock,
        connection.read(request(
            &*connection,
            vec![tag("good", 42), tag("bad", 65535)],
        )),
    )
    .unwrap();
    assert_eq!(response.iter_ok().count(), 1);
    let error = response
        .iter_failed()
        .next()
        .unwrap()
        .outcome()
        .as_ref()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(error.status().unwrap().code(), 0x8034_0000);
    assert_eq!(error.status().unwrap().name(), Some("BadNodeIdUnknown"));
}

#[test]
fn opcua_read_rejects_foreign_addresses_counts_and_connection_item_budget_before_io() {
    let rig = Rig::new();
    let connection = rig.connect();
    let before = rig.frames();
    let foreign = Tag::new(
        TagId::new("foreign").unwrap(),
        Arc::new(FixtureAddress::new("other", None)),
    );
    for (tag, kind) in [
        (foreign, ErrorKind::InvalidAddress),
        (
            tag("count", 1).with_count(NonZeroU32::new(2).unwrap()),
            ErrorKind::InvalidConfig,
        ),
    ] {
        let error = block_on_with(
            &rig.clock,
            connection.read(request(&*connection, vec![tag])),
        )
        .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(
            error.diagnostics().category,
            Some(crate::FailureCategory::Validation)
        );
    }
    assert_eq!(rig.frames(), before);
    drop(connection);
    let rig = Rig::new();
    let parsed = rig.driver.parse_config(URI).unwrap();
    let config = crate::ConnectionConfig::builder("opcua", parsed.endpoint().clone())
        .driver_options(Arc::clone(parsed.driver_options().unwrap()))
        .limits(
            ResourceLimits::builder()
                .max_items_per_request(1)
                .build()
                .unwrap(),
        )
        .build()
        .unwrap();
    let connection = block_on_with(&rig.clock, rig.driver.connect(config)).unwrap();
    let oversized = vec![tag("a", 1), tag("b", 2)]
        .into_iter()
        .fold(
            ReadRequest::builder(ResourceLimits::builder().build().unwrap()),
            crate::ReadRequestBuilder::tag,
        )
        .build()
        .unwrap();
    let before = rig.frames();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(oversized))
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.frames(), before);
}

#[test]
fn opcua_read_deadlines_cancellation_and_drop_preserve_the_next_read() {
    let rig = Rig::new();
    let connection = rig.connect();
    for cancel_after_send in [false, true] {
        let token = CancellationToken::new();
        let read = ReadRequest::builder(connection.limits())
            .tag(tag("a", 1))
            .cancel(token.clone())
            .build()
            .unwrap();
        let before = rig.frames();
        if cancel_after_send {
            rig.device.timeout();
        } else {
            token.cancel();
        }
        let mut future = connection.read(read);
        if cancel_after_send {
            assert!(
                future
                    .as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            token.cancel();
        }
        assert_eq!(
            block_on_with(&rig.clock, future).unwrap_err().kind(),
            ErrorKind::Cancelled
        );
        assert_eq!(rig.frames(), before + usize::from(cancel_after_send));
    }
    let before = rig.frames();
    let expired = ReadRequest::builder(connection.limits())
        .tag(tag("a", 1))
        .deadline(Deadline::at(rig.clock.now()))
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(expired))
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(rig.frames(), before);
    rig.device.timeout();
    let mut future = connection.read(request(&*connection, vec![tag("a", 1)]));
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(future);
    let response = block_on_with(
        &rig.clock,
        connection.read(request(&*connection, vec![tag("b", 2)])),
    )
    .unwrap();
    assert_eq!(response.iter_ok().count(), 1);
}

#[test]
fn opcua_close_fails_pending_read_with_disconnected() {
    let rig = Rig::new();
    let connection = rig.connect();
    rig.device.timeout();
    let mut read = connection.read(request(&*connection, vec![tag("a", 1)]));
    let mut cx = Context::from_waker(Waker::noop());
    assert!(read.as_mut().poll(&mut cx).is_pending());
    let mut close = connection.close();
    assert!(close.as_mut().poll(&mut cx).is_pending());
    assert_eq!(
        block_on_with(&rig.clock, read).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
    block_on_with(&rig.clock, close).unwrap();
    assert_eq!(connection.state(), ConnectionState::Closed);
}

#[test]
fn opcua_disconnect_fails_the_connection_and_device_fault_preserves_its_status() {
    let rig = Rig::new();
    let connection = rig.connect();
    rig.device.exception();
    let error = block_on_with(
        &rig.clock,
        connection.read(request(&*connection, vec![tag("a", 1)])),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::DeviceException);
    assert_eq!(
        error.diagnostics().category,
        Some(crate::FailureCategory::Protocol)
    );
    assert_eq!(connection.state(), ConnectionState::Connected);
    rig.device.disconnect();
    let watch = connection.watch_state();
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.read(request(&*connection, vec![tag("a", 1)]))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(
        watch.current(),
        ConnectionState::Failed(ErrorKind::Disconnected)
    );
    let _closed = block_on_with(&rig.clock, connection.close());
    assert_eq!(connection.state(), ConnectionState::Closed);
}

fn altered_read(reply: Vec<u8>) -> Rig {
    let clock = FakeClock::default();
    let factory = FakeTransportFactory::new(clock.clone());
    let device = Device::default();
    let mut reactor = device.reactor();
    factory.push_script(Script::strict(vec![Step::react(move |bytes| {
        let normal = reactor(bytes);
        if bytes.starts_with(b"MSG") && matches!(body(bytes), Service::ReadRequest { .. }) {
            Reaction::Reply(message(44, 4, 17, 29, &reply))
        } else {
            normal
        }
    })]));
    let driver = OpcuaDriver::new()
        .with_clock(Arc::new(clock.clone()))
        .with_transport_factory(Arc::new(factory.clone()));
    Rig {
        driver,
        clock,
        factory,
        device,
    }
}

#[test]
fn opcua_a_short_long_or_null_result_list_is_malformed_data() {
    for values in [vec![], vec![int32_value(1, 0), int32_value(2, 0)]] {
        let rig = altered_read(read_results(4, &values));
        let connection = rig.connect();
        assert_eq!(
            block_on_with(
                &rig.clock,
                connection.read(request(&*connection, vec![tag("one", 1)]))
            )
            .unwrap_err()
            .kind(),
            ErrorKind::MalformedData
        );
    }
    let mut null = read_results(4, &[]);
    null.get_mut(28..32).unwrap().fill(0xFF);
    let rig = altered_read(null);
    let connection = rig.connect();
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.read(request(&*connection, vec![tag("one", 1)]))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::MalformedData
    );
}

#[test]
fn opcua_wrong_read_service_and_service_result_are_operation_errors() {
    let mut refused = read_results(4, &[int32_value(1, 0)]);
    refused
        .get_mut(16..20)
        .unwrap()
        .copy_from_slice(&0x8001_0000_u32.to_le_bytes());
    for (reply, kind) in [
        (close_response(4, 0), ErrorKind::MalformedData),
        (refused, ErrorKind::DeviceException),
    ] {
        let rig = altered_read(reply);
        let connection = rig.connect();
        let error = block_on_with(
            &rig.clock,
            connection.read(request(&*connection, vec![tag("one", 1)])),
        )
        .unwrap_err();
        assert_eq!(error.kind(), kind);
        assert_eq!(error.diagnostics().operation, Some(Operation::Read));
    }
}

#[test]
fn opcua_invalid_utf8_is_only_one_item_error_in_a_successful_batch() {
    let rig = altered_read(read_results(
        4,
        &[vec![1, 12, 1, 0, 0, 0, 0xFF], int32_value(42, 0)],
    ));
    let connection = rig.connect();
    let response = block_on_with(
        &rig.clock,
        connection.read(request(
            &*connection,
            vec![tag("text", 1), tag("number", 2)],
        )),
    )
    .unwrap();
    assert_eq!(response.iter_failed().count(), 1);
    assert_eq!(response.iter_ok().count(), 1);
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap_err()
            .kind(),
        ErrorKind::MalformedData
    );
}

#[test]
fn opcua_read_timeout_and_bounded_retries_use_the_fake_clock() {
    for retries in [1, 3] {
        let rig = Rig::new();
        let parsed = rig.driver.parse_config(URI).unwrap();
        let config = crate::ConnectionConfig::builder("opcua", parsed.endpoint().clone())
            .timeouts(*parsed.timeouts())
            .driver_options(Arc::clone(parsed.driver_options().unwrap()))
            .retry(RetryPolicy {
                max_attempts: NonZeroU32::new(retries).unwrap(),
                ..RetryPolicy::default()
            })
            .build()
            .unwrap();
        let connection = block_on_with(&rig.clock, rig.driver.connect(config)).unwrap();
        for _ in 0..retries {
            rig.device.timeout();
        }
        let start = rig.clock.now();
        let error = block_on_with(
            &rig.clock,
            connection.read(request(&*connection, vec![tag("one", 1)])),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Timeout);
        assert_eq!(error.diagnostics().retries, retries - 1);
        assert_eq!(rig.device.reads(), retries);
        assert_eq!(
            rig.clock.now().duration_since(start),
            Duration::from_millis(25 * u64::from(retries))
        );
        let _closed = block_on_with(&rig.clock, connection.close());
    }
    let rig = Rig::new();
    let parsed = rig.driver.parse_config(URI).unwrap();
    let config = crate::ConnectionConfig::builder("opcua", parsed.endpoint().clone())
        .timeouts(*parsed.timeouts())
        .driver_options(Arc::clone(parsed.driver_options().unwrap()))
        .retry(RetryPolicy {
            max_attempts: NonZeroU32::new(3).unwrap(),
            ..RetryPolicy::default()
        })
        .build()
        .unwrap();
    let connection = block_on_with(&rig.clock, rig.driver.connect(config)).unwrap();
    rig.device.timeout();
    rig.device.timeout();
    let deadline = Deadline::after(rig.clock.now(), Duration::from_millis(30)).unwrap();
    let read = ReadRequest::builder(connection.limits())
        .tag(tag("one", 1))
        .deadline(deadline)
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(read))
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(rig.device.reads(), 2);
    assert_eq!(rig.clock.now(), deadline.instant());
}

#[tokio::test]
#[ignore = "requires open62541; interop/open62541/interop.sh tests"]
#[expect(
    clippy::disallowed_macros,
    reason = "interop evidence records verified read facts"
)]
async fn opcua_interop_reads_boolean_int32_double_and_string_with_policy_none() {
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("URI");
    let driver = OpcuaDriver::new();
    let connection = driver
        .connect(driver.parse_config(&uri).unwrap())
        .await
        .unwrap();
    let mut request = ReadRequest::builder(connection.limits());
    for name in ["Boolean", "Int32", "Double", "String", "unknown"] {
        let address = OpcuaAddress::parse(&format!("ns=1;s=scalar/{name}")).unwrap();
        request = request.tag(Tag::new(TagId::new(name).unwrap(), Arc::new(address)));
    }
    let response = connection.read(request.build().unwrap()).await.unwrap();
    assert_eq!(response.items().len(), 5);
    assert_eq!(response.iter_ok().count(), 4);
    let expected = [
        PlcValue::Bool(true),
        PlcValue::I32(-123_456_789),
        PlcValue::F64(std::f64::consts::E),
        PlcValue::String("stonecrab".to_owned()),
    ];
    for (item, expected) in response.iter_ok().zip(expected) {
        assert_eq!(item.outcome().as_ref().unwrap(), &expected);
    }
    let error = response
        .iter_failed()
        .next()
        .unwrap()
        .outcome()
        .as_ref()
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::NotFound);
    assert_eq!(error.status().unwrap().code(), 0x8034_0000);
    connection.close().await.unwrap();
    println!(
        "public Read: Boolean, Int32, Double, String exact; unknown node NotFound (0x80340000); close passed"
    );
}

struct FaultClock {
    clock: FakeClock,
    fail: Arc<AtomicBool>,
}
impl Clock for FaultClock {
    fn now(&self) -> std::time::Instant {
        self.clock.now()
    }
    fn wall_now(&self) -> std::time::SystemTime {
        self.clock.wall_now()
    }
    fn sleep_until(&self, deadline: std::time::Instant) -> OpFuture<'_, ()> {
        if self.fail.load(Ordering::Acquire) {
            Box::pin(async { Err(PlcError::new(ErrorKind::Internal, "injected clock failure")) })
        } else {
            self.clock.sleep_until(deadline)
        }
    }
}

#[test]
fn opcua_a_clock_failure_has_internal_read_diagnostics_without_sending() {
    let rig = Rig::new();
    let fail = Arc::new(AtomicBool::new(false));
    let driver = rig.driver.with_clock(Arc::new(FaultClock {
        clock: rig.clock.clone(),
        fail: Arc::clone(&fail),
    }));
    let connection = block_on_with(
        &rig.clock,
        driver.connect(driver.parse_config(URI).unwrap()),
    )
    .unwrap();
    let before = rig.device.reads();
    fail.store(true, Ordering::Release);
    let error = block_on_with(
        &rig.clock,
        connection.read(request(&*connection, vec![tag("one", 1)])),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Internal);
    assert_eq!(
        error.diagnostics().category,
        Some(crate::FailureCategory::Internal)
    );
    assert_eq!(rig.device.reads(), before);
}
