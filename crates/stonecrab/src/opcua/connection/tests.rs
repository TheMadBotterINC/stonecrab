use super::*;
use crate::opcua::codec::NodeId;
use crate::opcua::frame::{Body, OpcuaFramer};
use crate::opcua::responder::{Device, close_response, int32_value, message, read_results};
use crate::opcua::service::decode_service;
use crate::opcua::{OpcuaAddress, OpcuaDataType, OpcuaDriver};
use crate::testkit::{
    FakeClock, FakeTransportFactory, FixtureAddress, Reaction, Script, Step, block_on_with,
};
use crate::transport::Framer;
use crate::{Driver, PlcValue, ResourceLimits, RetryPolicy, Tag, TagId};
use bytes::BytesMut;
use std::num::NonZeroU32;
use std::task::{Context, Waker};

const URI: &str = "opcua://fake:4840/path?security-policy=NONE&request-timeout-ms=25";

#[tokio::test]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_browse_walks_the_test_namespace() {
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").unwrap();
    let driver = OpcuaDriver::new();
    let config = driver.parse_config(&uri).unwrap();
    let clock = Arc::new(crate::transport::SystemClock);
    let session = driver
        .open_session(
            &config,
            Deadline::after(clock.now(), Duration::from_secs(30)).unwrap(),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    let connection = OpcuaConnection::new(session, driver, config, clock);
    let request = crate::BrowseRequest::builder(connection.limits())
        .query(TagId::new("interop").unwrap(), "ns=1;s=Interop")
        .build()
        .unwrap();
    let response = connection.browse_walk(request).await.unwrap();
    let item = response.items().first().unwrap();
    assert!(!item.truncated());
    let nodes = item.outcome().as_ref().unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node.address().canonical() == "ns=1;s=counter")
    );
    assert!(
        nodes
            .iter()
            .any(|node| node.address().canonical() == "ns=1;s=scalar/Int32")
    );
    assert!(nodes.iter().all(|node| node.error().is_none()));
    let request = crate::BrowseRequest::builder(
        ResourceLimits::builder()
            .max_browse_nodes(1)
            .build()
            .unwrap(),
    )
    .query(TagId::new("objects").unwrap(), "")
    .build()
    .unwrap();
    let response = connection.browse_walk(request).await.unwrap();
    assert!(response.items().first().unwrap().truncated());
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap()
            .len(),
        1
    );
    connection.close().await.unwrap();
}

struct Rig {
    driver: OpcuaDriver,
    clock: FakeClock,
    factory: FakeTransportFactory,
    device: Device,
}

#[test]
fn opcua_browse_missing_attached_options_is_an_internal_error() {
    let rig = Rig::new();
    let parsed = rig.driver.parse_config(URI).unwrap();
    let session = block_on_with(
        &rig.clock,
        rig.driver.open_session(
            &parsed,
            Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap(),
            &CancellationToken::new(),
        ),
    )
    .unwrap();
    let invalid = crate::ConnectionConfig::builder("opcua", parsed.endpoint().clone())
        .build()
        .unwrap();
    let connection =
        OpcuaConnection::new(session, rig.driver, invalid, Arc::new(rig.clock.clone()));
    let request = crate::BrowseRequest::builder(connection.limits())
        .query(TagId::new("root").unwrap(), "")
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.browse_walk(request))
            .unwrap_err()
            .kind(),
        ErrorKind::Internal
    );
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
fn opcua_disconnect_schedules_reconnect_and_device_fault_preserves_its_status() {
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
    assert!(matches!(
        watch.current(),
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
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

#[test]
fn opcua_temporal_suffixes_are_applied_per_tag_in_one_read_batch() {
    let rig = Rig::new();
    let connection = rig.connect();
    let tags = [
        "TIME",
        "LTIME",
        "DATE",
        "LDATE",
        "TIME_OF_DAY",
        "LTIME_OF_DAY",
        "DATE_AND_TIME",
        "LDATE_AND_TIME",
    ]
    .into_iter()
    .map(|suffix| {
        Tag::new(
            TagId::new(suffix).unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=2;i=42;{suffix}")).unwrap()),
        )
    })
    .collect();
    let reply = block_on_with(&rig.clock, connection.read(request(&*connection, tags))).unwrap();
    assert_eq!(reply.items().len(), 8);
    assert_eq!(rig.device.reads(), 1);
    for (index, suffix) in [
        OpcuaDataType::Time,
        OpcuaDataType::LTime,
        OpcuaDataType::Date,
        OpcuaDataType::LDate,
        OpcuaDataType::TimeOfDay,
        OpcuaDataType::LTimeOfDay,
        OpcuaDataType::DateAndTime,
        OpcuaDataType::LDateAndTime,
    ]
    .into_iter()
    .enumerate()
    {
        let mut budget = crate::opcua::value::MAX_VALUE_NODES;
        let expected = crate::opcua::value::decode(
            crate::opcua::codec::Variant::scalar(crate::opcua::codec::Scalar::Int32(42)),
            suffix,
            &mut budget,
        )
        .unwrap();
        assert_eq!(
            reply
                .items()
                .get(index)
                .unwrap()
                .outcome()
                .as_ref()
                .unwrap(),
            &expected
        );
    }
}

#[test]
fn opcua_an_unmapped_variant_is_a_per_item_unsupported_error() {
    let rig = altered_read(read_results(4, &[vec![1, 23, 0], int32_value(42, 0)]));
    let connection = rig.connect();
    let reply = block_on_with(
        &rig.clock,
        connection.read(request(
            &*connection,
            vec![tag("unmapped", 1), tag("ok", 2)],
        )),
    )
    .unwrap();
    let problem = reply
        .items()
        .first()
        .unwrap()
        .outcome()
        .as_ref()
        .unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::Unsupported(Operation::Read));
    assert!(problem.to_string().contains("DataValue"));
    assert_eq!(
        reply.items().get(1).unwrap().outcome().as_ref().unwrap(),
        &PlcValue::I32(42)
    );
    assert_eq!(reply.items().get(1).unwrap().status().unwrap().code(), 0);
}

#[test]
fn opcua_array_shapes_and_bad_node_text_are_isolated_to_their_items() {
    let matrix = [
        vec![1, 0xC6],
        6_i32.to_le_bytes().to_vec(),
        (1_i32..=6).flat_map(i32::to_le_bytes).collect(),
        2_i32.to_le_bytes().to_vec(),
        2_i32.to_le_bytes().to_vec(),
        3_i32.to_le_bytes().to_vec(),
    ]
    .concat();
    let rig = altered_read(read_results(
        4,
        &[
            matrix,
            vec![1, 17, 3, 2, 0, 1, 0, 0, 0, 0xFF],
            [vec![1, 0x86], (-1_i32).to_le_bytes().to_vec()].concat(),
            [vec![1, 0x86], 0_i32.to_le_bytes().to_vec()].concat(),
            [
                vec![1, 0x86],
                1_i32.to_le_bytes().to_vec(),
                42_i32.to_le_bytes().to_vec(),
            ]
            .concat(),
        ],
    ));
    let connection = rig.connect();
    let reply = block_on_with(
        &rig.clock,
        connection.read(request(
            &*connection,
            (0..5).map(|n| tag(&n.to_string(), n)).collect(),
        )),
    )
    .unwrap();
    let ints = |values: &[i32]| PlcValue::List(values.iter().copied().map(PlcValue::I32).collect());
    assert_eq!(
        reply.items().first().unwrap().outcome().as_ref().unwrap(),
        &PlcValue::List(vec![ints(&[1, 2, 3]), ints(&[4, 5, 6])])
    );
    assert_eq!(
        reply
            .items()
            .get(1)
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap_err()
            .kind(),
        ErrorKind::MalformedData
    );
    assert_eq!(
        reply.items().get(2).unwrap().outcome().as_ref().unwrap(),
        &PlcValue::Null
    );
    assert_eq!(
        reply.items().get(3).unwrap().outcome().as_ref().unwrap(),
        &ints(&[])
    );
    assert_eq!(
        reply.items().get(4).unwrap().outcome().as_ref().unwrap(),
        &ints(&[42])
    );
    assert_eq!(reply.iter_ok().count(), 4);
}

#[tokio::test]
#[ignore = "requires open62541; interop/open62541/interop.sh tests"]
#[expect(
    clippy::disallowed_macros,
    reason = "interop evidence records mapped types and explicit refusals"
)]
async fn opcua_interop_reads_every_builtin_scalar_array_and_matrix_node() {
    use crate::{PlcDate, PlcDateTime, PlcTimeOfDay};
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("URI");
    let driver = OpcuaDriver::new();
    let connection = driver
        .connect(
            driver
                .parse_config(&format!(
                    "{uri}&encoding.receive-buffer-size=8192&encoding.send-buffer-size=8192"
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let ints = |values: &[i32]| PlcValue::List(values.iter().copied().map(PlcValue::I32).collect());
    let doubles =
        |values: &[f64]| PlcValue::List(values.iter().copied().map(PlcValue::F64).collect());
    let expected = vec![
        ("scalar/Boolean", PlcValue::Bool(true)),
        ("scalar/SByte", PlcValue::I8(-12)),
        ("scalar/Byte", PlcValue::U8(200)),
        ("scalar/Int16", PlcValue::I16(-1234)),
        ("scalar/UInt16", PlcValue::U16(54321)),
        ("scalar/Int32", PlcValue::I32(-123_456_789)),
        ("scalar/UInt32", PlcValue::U32(3_123_456_789)),
        ("scalar/Int64", PlcValue::I64(-1_234_567_890_123)),
        ("scalar/UInt64", PlcValue::U64(12_345_678_901_234_567_890)),
        ("scalar/Float", PlcValue::F32(1.5)),
        ("scalar/Double", PlcValue::F64(std::f64::consts::E)),
        ("scalar/String", PlcValue::String("stonecrab".to_owned())),
        ("scalar/ByteString", PlcValue::Bytes(vec![0, 1, 254, 255])),
        (
            "scalar/LocalizedText",
            PlcValue::Struct(
                crate::PlcStruct::new(vec![
                    ("text".to_owned(), PlcValue::String("text".to_owned())),
                    ("locale".to_owned(), PlcValue::String("en-US".to_owned())),
                ])
                .unwrap(),
            ),
        ),
        (
            "scalar/DateTime",
            PlcValue::DateTime(PlcDateTime::new(
                PlcDate::from_ymd(2026, 10, 30).unwrap(),
                PlcTimeOfDay::from_nanos_since_midnight(0).unwrap(),
            )),
        ),
        (
            "scalar/Guid",
            PlcValue::String("72962b91-fa75-4ae6-8d28-b404dc7daf63".to_owned()),
        ),
        ("scalar/XmlElement", PlcValue::String("<a>1</a>".to_owned())),
        ("scalar/NodeId", PlcValue::String("ns=0;i=2253".to_owned())),
        (
            "scalar/StatusCode",
            PlcValue::String("BadTimeout (0x800A0000)".to_owned()),
        ),
        (
            "scalar/QualifiedName",
            PlcValue::String("ns=1;s=qname".to_owned()),
        ),
        (
            "array/Boolean",
            PlcValue::List(vec![
                PlcValue::Bool(true),
                PlcValue::Bool(false),
                PlcValue::Bool(true),
            ]),
        ),
        ("array/Int32", ints(&[1, -2, 3, -4, 5])),
        ("array/Double", doubles(&[0.5, 1.5, 2.5])),
        (
            "array/String",
            PlcValue::List(
                ["alpha", "beta", "gamma"]
                    .into_iter()
                    .map(|text| PlcValue::String(text.to_owned()))
                    .collect(),
            ),
        ),
        (
            "matrix/Int32_2x3",
            PlcValue::List(vec![ints(&[1, 2, 3]), ints(&[4, 5, 6])]),
        ),
        (
            "matrix/Double_2x2x2",
            PlcValue::List(vec![
                PlcValue::List(vec![doubles(&[1.0, 2.0]), doubles(&[3.0, 4.0])]),
                PlcValue::List(vec![doubles(&[5.0, 6.0]), doubles(&[7.0, 8.0])]),
            ]),
        ),
    ];
    let refused = [
        ("scalar/ExpandedNodeId", "ExpandedNodeId"),
        ("scalar/DataValue", "DataValue"),
        ("scalar/DiagnosticInfo", "DiagnosticInfo"),
        ("structure", "ExtensionObject"),
    ];
    let large = [
        ("large/Int32_4000", 4000_usize),
        ("large/Int32_10000", 10000_usize),
    ];
    let mut request = ReadRequest::builder(connection.limits());
    for name in expected
        .iter()
        .map(|(name, _)| *name)
        .chain(refused.iter().map(|(name, _)| *name))
        .chain(large.iter().map(|(name, _)| *name))
    {
        request = request.tag(Tag::new(
            TagId::new(name).unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=1;s={name}")).unwrap()),
        ));
    }
    let reply = connection.read(request.build().unwrap()).await.unwrap();
    assert_eq!(reply.items().len(), 32);
    assert_eq!(reply.iter_ok().count(), 28);
    assert_eq!(reply.iter_failed().count(), 4);
    for ((name, expected), item) in expected.iter().zip(reply.items()) {
        assert_eq!(item.tag().as_str(), *name);
        assert_eq!(item.outcome().as_ref().unwrap(), expected, "{name}");
        assert_eq!(
            item.status().unwrap().code(),
            0,
            "value StatusCode never replaces item quality"
        );
    }
    for ((name, kind), item) in refused
        .iter()
        .zip(reply.items().iter().skip(expected.len()))
    {
        assert_eq!(item.tag().as_str(), *name);
        let error = item.outcome().as_ref().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported(Operation::Read));
        assert!(error.to_string().contains(kind));
    }
    for ((name, count), item) in large
        .iter()
        .zip(reply.items().iter().skip(expected.len() + refused.len()))
    {
        assert_eq!(item.tag().as_str(), *name);
        let PlcValue::List(values) = item.outcome().as_ref().unwrap() else {
            panic!("array")
        };
        assert_eq!(values.len(), *count);
        for (index, value) in values.iter().enumerate() {
            assert_eq!(*value, PlcValue::I32(i32::try_from(index).unwrap()));
        }
    }
    connection.close().await.unwrap();
    println!(
        "public Read: 18 mapped scalars, four arrays, 2D/3D matrices and 4000/10000-element arrays exact; six named deferred-kind errors isolated; 8192-byte buffers; close passed"
    );
}

#[tokio::test]
#[expect(
    clippy::disallowed_macros,
    reason = "ignored independent peer test records non-secret value/metadata checks"
)]
#[ignore = "needs the independent open62541 peer; interop/open62541/interop.sh tests"]
async fn opcua_interop_reads_bytes_localized_text_and_both_timestamps() {
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("URI");
    let driver = OpcuaDriver::new();
    let connection = driver
        .connect(driver.parse_config(&uri).unwrap())
        .await
        .unwrap();
    let mut request = ReadRequest::builder(connection.limits());
    for name in ["ByteString", "LocalizedText"] {
        request = request.tag(Tag::new(
            TagId::new(name).unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=1;s=scalar/{name}")).unwrap()),
        ));
    }
    let reply = connection.read(request.build().unwrap()).await.unwrap();
    assert_eq!(reply.items().len(), 2);
    assert_eq!(
        reply.items().first().unwrap().outcome().as_ref().unwrap(),
        &PlcValue::Bytes(vec![0, 1, 254, 255])
    );
    let PlcValue::Struct(text) = reply.items().get(1).unwrap().outcome().as_ref().unwrap() else {
        panic!("localized text structure")
    };
    assert_eq!(text.get("text"), Some(&PlcValue::String("text".to_owned())));
    assert_eq!(
        text.get("locale"),
        Some(&PlcValue::String("en-US".to_owned()))
    );
    for item in reply.items() {
        assert_eq!(item.status().unwrap().code(), 0);
        assert!(
            item.source_time()
                .is_some_and(|time| time > std::time::UNIX_EPOCH)
        );
        assert!(
            item.server_time()
                .is_some_and(|time| time > std::time::UNIX_EPOCH)
        );
    }
    println!(
        "ByteString exact octets, LocalizedText text/locale and both typed timestamps verified through public Read"
    );
    connection.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires the disposable open62541 peer"]
async fn opcua_interop_write_then_read_round_trips_every_writable_type() {
    use crate::{PlcDate, PlcDateTime, PlcSignedDuration, PlcTimeOfDay};
    let uri = std::env::var("STONECRAB_INTEROP_OPEN62541").expect("URI");
    let driver = OpcuaDriver::new();
    let connection = driver
        .connect(
            driver
                .parse_config(&format!("{uri}&access=read-write"))
                .unwrap(),
        )
        .await
        .unwrap();
    let datetime = PlcValue::DateTime(PlcDateTime::new(
        PlcDate::from_days_since_epoch(10957),
        PlcTimeOfDay::from_hms_nano(1, 2, 3, 456_789_100).unwrap(),
    ));
    let values = vec![
        ("Boolean", PlcValue::Bool(false)),
        ("SByte", PlcValue::I8(i8::MIN)),
        ("Byte", PlcValue::U8(u8::MAX)),
        ("Int16", PlcValue::I16(i16::MIN)),
        ("UInt16", PlcValue::U16(u16::MAX)),
        ("Int32", PlcValue::I32(i32::MIN)),
        ("UInt32", PlcValue::U32(u32::MAX)),
        ("Int64", PlcValue::I64(i64::MIN)),
        ("UInt64", PlcValue::U64(u64::MAX)),
        ("Float", PlcValue::F32(1.5)),
        ("Double", PlcValue::F64(-2.5)),
        ("String", PlcValue::String("write 水\0text".into())),
        ("DateTime", datetime),
        ("ByteString", PlcValue::Bytes(vec![0, 255, 1, 254])),
    ];
    let mut write = WriteRequest::builder(connection.limits());
    let mut read = ReadRequest::builder(connection.limits());
    for (name, value) in &values {
        let tag = Tag::new(
            TagId::new(name).unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=1;s=writable/{name}")).unwrap()),
        );
        write = write.item(tag.clone(), value.clone());
        read = read.tag(tag);
    }
    let response = connection.write(write.build().unwrap()).await.unwrap();
    assert_eq!(response.confirmed_count(), values.len(), "{response:?}");
    let response = connection.read(read.build().unwrap()).await.unwrap();
    for (item, (_, expected)) in response.items().iter().zip(&values) {
        assert_eq!(item.outcome().as_ref().unwrap(), expected);
    }
    // CHAR/WCHAR use String on the wire; WSTRING has the same UTF-8 representation.
    for (suffix, value, expected) in [
        ("CHAR", PlcValue::Char('Z'), PlcValue::String("Z".into())),
        ("WCHAR", PlcValue::Char('水'), PlcValue::String("水".into())),
        (
            "STRING",
            PlcValue::String("wide 水".into()),
            PlcValue::String("wide 水".into()),
        ),
    ] {
        let tag = Tag::new(
            TagId::new("text").unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=1;s=writable/String;{suffix}")).unwrap()),
        );
        let response = connection
            .write(
                WriteRequest::builder(connection.limits())
                    .item(tag, value)
                    .build()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.confirmed_count(), 1);
        let response = connection
            .read(request(
                &*connection,
                vec![Tag::new(
                    TagId::new("text").unwrap(),
                    Arc::new(OpcuaAddress::parse("ns=1;s=writable/String").unwrap()),
                )],
            ))
            .await
            .unwrap();
        assert_eq!(
            response
                .items()
                .first()
                .unwrap()
                .outcome()
                .as_ref()
                .unwrap(),
            &expected
        );
    }
    let duration = PlcValue::SignedDuration(PlcSignedDuration::from_millis(-123).unwrap());
    let tag = Tag::new(
        TagId::new("time").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;s=writable/Int32;TIME").unwrap()),
    );
    let response = connection
        .write(
            WriteRequest::builder(connection.limits())
                .item(tag.clone(), duration.clone())
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.confirmed_count(), 1);
    let response = connection
        .read(request(&*connection, vec![tag]))
        .await
        .unwrap();
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap(),
        &duration
    );
    let array = PlcValue::List(vec![
        PlcValue::I32(10),
        PlcValue::I32(20),
        PlcValue::I32(30),
    ]);
    let matrix = PlcValue::List(vec![
        PlcValue::List(vec![PlcValue::I32(1), PlcValue::I32(2), PlcValue::I32(3)]),
        PlcValue::List(vec![PlcValue::I32(4), PlcValue::I32(5), PlcValue::I32(6)]),
    ]);
    for (name, value) in [("Int32Array", array), ("Int32Matrix", matrix)] {
        let tag = Tag::new(
            TagId::new(name).unwrap(),
            Arc::new(OpcuaAddress::parse(&format!("ns=1;s=writable/{name}")).unwrap()),
        );
        let response = connection
            .write(
                WriteRequest::builder(connection.limits())
                    .item(tag.clone(), value.clone())
                    .build()
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.confirmed_count(), 1, "{response:?}");
        let response = connection
            .read(request(&*connection, vec![tag]))
            .await
            .unwrap();
        assert_eq!(
            response
                .items()
                .first()
                .unwrap()
                .outcome()
                .as_ref()
                .unwrap(),
            &value
        );
    }
    let tag = Tag::new(
        TagId::new("selected").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;s=writable/Int32Array[1];DINT").unwrap()),
    );
    let response = connection
        .write(
            WriteRequest::builder(connection.limits())
                .item(tag, PlcValue::List(vec![PlcValue::I32(-99)]))
                .build()
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.confirmed_count(), 1);
    let tag = Tag::new(
        TagId::new("check").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;s=writable/Int32Array").unwrap()),
    );
    let response = connection
        .read(request(&*connection, vec![tag]))
        .await
        .unwrap();
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap(),
        &PlcValue::List(vec![
            PlcValue::I32(10),
            PlcValue::I32(-99),
            PlcValue::I32(30)
        ])
    );
    connection.close().await.unwrap();
}

fn concrete(rig: Rig) -> (OpcuaConnection, FakeClock, FakeTransportFactory) {
    let config = rig.driver.parse_config(URI).unwrap();
    let deadline = Deadline::after(rig.clock.now(), config.timeouts().connect).unwrap();
    let session = block_on_with(
        &rig.clock,
        rig.driver
            .open_session(&config, deadline, &CancellationToken::new()),
    )
    .unwrap();
    let connection = OpcuaConnection::new(session, rig.driver, config, Arc::new(rig.clock.clone()));
    (connection, rig.clock, rig.factory)
}

#[test]
fn opcua_concurrent_idle_callers_share_one_fresh_session() {
    let (connection, clock, factory) = concrete(Rig::new());
    factory.push_script(Script::strict(vec![Step::react(
        Device::default().reactor(),
    )]));
    clock.advance(Duration::from_secs(120));
    let guard = block_on_with(&clock, connection.preparation.lock());
    let mut first = connection.read(request(&connection, vec![tag("first", 1)]));
    let mut second = connection.read(request(&connection, vec![tag("second", 2)]));
    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(
        second
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    drop(guard);
    assert_eq!(block_on_with(&clock, first).unwrap().iter_ok().count(), 1);
    assert_eq!(block_on_with(&clock, second).unwrap().iter_ok().count(), 1);
    assert_eq!(factory.opens().len(), 2);
    block_on_with(&clock, connection.close()).unwrap();
}

#[test]
fn opcua_idle_reopen_failure_and_close_during_activation_do_not_publish_a_session() {
    for closing in [false, true] {
        let (connection, clock, factory) = concrete(Rig::new());
        let original = connection.current();
        if closing {
            let stop = connection.operations_stopped.clone();
            let mut reactor = Device::default().reactor();
            factory.push_script(Script::strict(vec![Step::react(move |wire| {
                let reply = reactor(wire);
                if wire.starts_with(b"MSG")
                    && matches!(body(wire), Service::ActivateSessionRequest { .. })
                {
                    stop.cancel();
                }
                reply
            })]));
        } else {
            factory.push_refusal();
        }
        clock.advance(Duration::from_secs(120));
        let error = block_on_with(
            &clock,
            connection.read(request(&connection, vec![tag("value", 1)])),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert!(Arc::ptr_eq(&original, &connection.current()));
        assert_eq!(factory.opens().len(), 2);
        if !closing {
            assert!(matches!(
                connection.state(),
                ConnectionState::Reconnecting { attempt: 1, .. }
            ));
        }
        let _closed = block_on_with(&clock, connection.close());
        assert_eq!(connection.state(), ConnectionState::Closed);
    }
}

#[test]
fn opcua_close_admission_prevents_an_expired_session_from_starting_recovery() {
    let (connection, clock, factory) = concrete(Rig::new());
    clock.advance(Duration::from_secs(120));
    connection.state.set(ConnectionState::Closing);
    let deadline = Deadline::after(clock.now(), Duration::from_secs(1)).unwrap();
    let error = block_on_with(
        &clock,
        connection.ready(deadline, &CancellationToken::new()),
    )
    .err()
    .unwrap();
    assert_eq!(error.kind(), ErrorKind::Disconnected);
    assert_eq!(factory.opens().len(), 1);
    assert_eq!(connection.state(), ConnectionState::Closing);
    let _closed = block_on_with(&clock, connection.close());
    assert_eq!(connection.state(), ConnectionState::Closed);
}

#[path = "reconnect_tests.rs"]
mod reconnect_tests;
