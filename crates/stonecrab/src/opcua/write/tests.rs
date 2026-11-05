use super::*;
use crate::opcua::codec::{Depth, NodeId, Scalar, VariantValue};
use crate::opcua::responder::{Device, write_response};
use crate::opcua::service::decode_service;
use crate::opcua::{OpcuaAddress, OpcuaDriver};
use crate::testkit::{
    FakeClock, FakeTransportFactory, FixtureAddress, Reaction, Script, Step, block_on_with,
};
use crate::{
    CancellationToken, Clock, Connection, ConnectionConfig, ConnectionState, Driver, PlcValue,
    ResourceLimits, Tag,
};
use std::sync::Arc;
use std::task::{Context, Waker};

const URI: &str =
    "opcua://fake:4840/path?security-policy=NONE&request-timeout-ms=25&access=read-write";
struct Rig {
    driver: OpcuaDriver,
    clock: FakeClock,
    factory: FakeTransportFactory,
    device: Device,
}
impl Rig {
    fn new() -> Self {
        Self::mapped(|_, reaction| reaction)
    }
    fn mapped(mut map: impl FnMut(Service, Reaction) -> Reaction + Send + 'static) -> Self {
        let clock = FakeClock::default();
        let device = Device::default();
        let mut reactor = device.reactor();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![Step::react(move |bytes| {
            let reaction = reactor(bytes);
            if bytes.starts_with(b"MSG") {
                map(payload(bytes), reaction)
            } else {
                reaction
            }
        })]));
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
        self.connect_uri(URI, ResourceLimits::builder().build().unwrap())
    }
    fn connect_uri(&self, uri: &str, limits: ResourceLimits) -> Box<dyn Connection> {
        let config = self.driver.parse_config(uri).unwrap();
        let config = ConnectionConfig::builder(config.scheme(), config.endpoint().clone())
            .access(config.access())
            .timeouts(*config.timeouts())
            .limits(limits)
            .driver_options(Arc::clone(config.driver_options().unwrap()))
            .build()
            .unwrap();
        block_on_with(&self.clock, self.driver.connect(config)).unwrap()
    }
    fn sent(&self) -> Vec<Service> {
        self.factory
            .probes()
            .into_iter()
            .flat_map(|probe| probe.sent())
            .filter(|bytes| bytes.starts_with(b"MSG"))
            .map(|bytes| payload(&bytes))
            .collect()
    }
    fn writes(&self) -> usize {
        self.sent()
            .into_iter()
            .filter(|s| matches!(s, Service::WriteRequest { .. }))
            .count()
    }
    fn metadata(&self) -> usize {
        self.sent().into_iter().filter(|s| matches!(s,Service::ReadRequest { nodes_to_read: Some(nodes), .. } if nodes.iter().any(|node| node.attribute_id !=13))).count()
    }
}
fn payload(bytes: &[u8]) -> Service {
    decode_service(bytes.get(24..).unwrap()).unwrap()
}
fn tag(name: &str, address: &str) -> Tag {
    Tag::new(
        TagId::new(name).unwrap(),
        Arc::new(OpcuaAddress::parse(address).unwrap()),
    )
}
fn request(connection: &dyn Connection, address: &str, value: PlcValue) -> WriteRequest {
    WriteRequest::builder(connection.limits())
        .item(tag("setpoint", address), value)
        .build()
        .unwrap()
}
fn replace(reaction: Reaction, body: Vec<u8>) -> Reaction {
    let Reaction::Reply(mut bytes) = reaction else {
        panic!("reply")
    };
    bytes.truncate(24);
    bytes.extend(body);
    let size = u32::try_from(bytes.len()).unwrap().to_le_bytes();
    bytes.get_mut(4..8).unwrap().copy_from_slice(&size);
    Reaction::Reply(bytes)
}

#[test]
fn opcua_write_of_an_int32_node_sends_one_write_request() {
    let rig = Rig::new();
    let connection = rig.connect();
    let response = block_on_with(
        &rig.clock,
        connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(-7))),
    )
    .unwrap();
    assert_eq!(response.confirmed_count(), 1);
    assert_eq!(
        response.results().first().unwrap().status().unwrap().code(),
        0
    );
    assert_eq!(rig.writes(), 1);
    assert_eq!(rig.metadata(), 0);
    let sent = rig.sent();
    let Service::WriteRequest {
        nodes_to_write: Some(nodes),
        ..
    } = sent.last().unwrap()
    else {
        panic!("write")
    };
    let node = nodes.first().unwrap();
    assert_eq!(node.node_id, NodeId::numeric(2, 42));
    assert_eq!(node.attribute_id, 13);
    assert!(node.index_range.as_bytes().is_none());
    assert_eq!(
        node.value.value.as_ref().unwrap().value,
        VariantValue::Scalar(Scalar::Int32(-7))
    );
    assert!(node.value.status.is_none());
    assert!(node.value.source_timestamp.is_none());
    assert!(node.value.server_timestamp.is_none());
    assert!(node.value.source_picoseconds.is_none());
    assert!(node.value.server_picoseconds.is_none());
    block_on_with(&rig.clock, connection.close()).unwrap();
}
// covers: OPCUA-WRITE-REQUEST
#[test]
fn opcua_write_value_carries_attribute_and_index_range() {
    let item = WriteItem::new(
        tag("array", "ns=2;i=42[2];DINT"),
        PlcValue::List(vec![PlcValue::I32(3)]),
    );
    let node = node(
        &item,
        crate::opcua::read::node(item.tag()).unwrap(),
        &Attributes::default(),
        &mut value::Budget::new(1024),
    )
    .unwrap();
    assert_eq!(node.index_range.as_bytes(), Some(b"2".as_slice()));
    assert_eq!(node.attribute_id, 13);
}
#[test]
fn opcua_write_sends_value_only_without_status_or_timestamps() {
    let item = WriteItem::new(tag("one", "ns=2;i=42;DINT"), PlcValue::I32(3));
    let node = node(
        &item,
        crate::opcua::read::node(item.tag()).unwrap(),
        &Attributes::default(),
        &mut value::Budget::new(1024),
    )
    .unwrap();
    let mut writer = crate::codec::BoundedWriter::new(1024);
    node.value.serialize(&mut writer, Depth::new()).unwrap();
    assert_eq!(writer.into_bytes(), [1, 6, 3, 0, 0, 0]);
}
#[test]
fn opcua_server_attributes_are_cached_per_session() {
    let rig = Rig::new();
    let connection = rig.connect();
    for value in [1, 2] {
        assert_eq!(
            block_on_with(
                &rig.clock,
                connection.write(request(&*connection, "ns=2;i=42", PlcValue::I32(value)))
            )
            .unwrap()
            .confirmed_count(),
            1
        );
    }
    assert_eq!(rig.metadata(), 1);
    assert_eq!(rig.writes(), 2);
    block_on_with(&rig.clock, connection.close()).unwrap();
    rig.factory
        .push_script(Script::strict(vec![Step::react(rig.device.reactor())]));
    let second = rig.connect();
    let response = block_on_with(
        &rig.clock,
        second.write(request(&*second, "ns=2;i=42", PlcValue::I32(3))),
    )
    .unwrap();
    assert_eq!(response.confirmed_count(), 1);
    assert_eq!(rig.metadata(), 2);
    block_on_with(&rig.clock, second.close()).unwrap();
}
#[test]
fn opcua_metadata_cache_and_requests_obey_the_session_item_bound() {
    let rig = Rig::new();
    let connection = rig.connect_uri(
        URI,
        ResourceLimits::builder()
            .max_items_per_request(1)
            .build()
            .unwrap(),
    );
    for node in [42, 43, 42] {
        let response = block_on_with(
            &rig.clock,
            connection.write(request(
                &*connection,
                &format!("ns=2;i={node}"),
                PlcValue::I32(3),
            )),
        )
        .unwrap();
        assert_eq!(response.confirmed_count(), 1);
    }
    assert_eq!(rig.metadata(), 12);
    assert_eq!(rig.writes(), 3);
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_write_reply_lost_is_outcome_unknown_and_not_replayed() {
    let rig = Rig::new();
    let connection = rig.connect();
    rig.device.unknown_after_transmit();
    let response = block_on_with(
        &rig.clock,
        connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(77))),
    )
    .unwrap();
    assert_eq!(response.unknown_count(), 1);
    assert!(matches!(
        response.results().first().unwrap().outcome(),
        WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit)
    ));
    assert_eq!(rig.writes(), 1);
    assert_eq!(connection.state(), ConnectionState::Connected);
    let read = crate::ReadRequest::builder(connection.limits())
        .tag(tag("check", "ns=2;i=42"))
        .build()
        .unwrap();
    let response = block_on_with(&rig.clock, connection.read(read)).unwrap();
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap(),
        &PlcValue::I32(77)
    );
    assert_eq!(rig.writes(), 1);
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_write_cancellation_distinguishes_before_and_after_transmission() {
    let rig = Rig::new();
    let connection = rig.connect();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let make = |cancel: CancellationToken| {
        WriteRequest::builder(connection.limits())
            .item(tag("one", "ns=2;i=42;DINT"), PlcValue::I32(1))
            .cancel(cancel)
            .build()
            .unwrap()
    };
    assert_eq!(
        block_on_with(&rig.clock, connection.write(make(cancel)))
            .unwrap_err()
            .kind(),
        ErrorKind::Cancelled
    );
    assert_eq!(rig.writes(), 0);
    let cancel = CancellationToken::new();
    rig.device.unknown_after_transmit();
    let mut future = connection.write(make(cancel.clone()));
    assert!(
        future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(rig.writes(), 1);
    cancel.cancel();
    let response = block_on_with(&rig.clock, future).unwrap();
    assert!(matches!(
        response.results().first().unwrap().outcome(),
        WriteOutcome::Unknown(UnknownReason::CancelledAfterTransmit)
    ));
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_write_disconnect_is_unknown_and_never_replayed() {
    let rig = Rig::mapped(|service, reaction| {
        if matches!(service, Service::WriteRequest { .. }) {
            Reaction::Close
        } else {
            reaction
        }
    });
    let connection = rig.connect();
    let response = block_on_with(
        &rig.clock,
        connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1))),
    )
    .unwrap();
    assert!(matches!(
        response.results().first().unwrap().outcome(),
        WriteOutcome::Unknown(UnknownReason::DisconnectedAfterTransmit)
    ));
    assert_eq!(rig.writes(), 1);
    assert!(matches!(
        connection.state(),
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
    assert_eq!(
        block_on_with(&rig.clock, connection.close())
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(connection.state(), ConnectionState::Closed);
}
#[test]
fn opcua_bad_not_writable_is_access_denied_on_write() {
    let rig = Rig::mapped(|service, reaction| {
        if let Service::WriteRequest { request_header, .. } = service {
            replace(
                reaction,
                write_response(request_header.request_handle, &[0x803B_0000]),
            )
        } else {
            reaction
        }
    });
    let connection = rig.connect();
    let response = block_on_with(
        &rig.clock,
        connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1))),
    )
    .unwrap();
    let result = response.results().first().unwrap();
    let WriteOutcome::Rejected(error) = result.outcome() else {
        panic!("rejection")
    };
    assert_eq!(error.kind(), ErrorKind::AccessDenied);
    assert_eq!(result.status().unwrap().code(), 0x803B_0000);
    assert!(std::error::Error::source(error.error()).is_some());
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_any_other_bad_write_status_keeps_its_code() {
    for (status, expected) in [
        (0x8074_0000, ErrorKind::InvalidDataType),
        (0x8034_0000, ErrorKind::NotFound),
        (0x8001_0000, ErrorKind::DeviceException),
        (0xDEAD_BEEF, ErrorKind::Internal),
    ] {
        let rig = Rig::mapped(move |service, reaction| {
            if let Service::WriteRequest { request_header, .. } = service {
                replace(
                    reaction,
                    write_response(request_header.request_handle, &[status]),
                )
            } else {
                reaction
            }
        });
        let connection = rig.connect();
        let response = block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1))),
        )
        .unwrap();
        let result = response.results().first().unwrap();
        let WriteOutcome::Rejected(error) = result.outcome() else {
            panic!("rejection")
        };
        assert_eq!(error.kind(), expected);
        assert_eq!(result.status().unwrap().code(), status);
        block_on_with(&rig.clock, connection.close()).unwrap();
    }
}
#[test]
fn opcua_write_count_mismatch_is_malformed_and_is_not_replayed() {
    let rig = Rig::mapped(|service, reaction| {
        if let Service::WriteRequest { request_header, .. } = service {
            replace(reaction, write_response(request_header.request_handle, &[]))
        } else {
            reaction
        }
    });
    let connection = rig.connect();
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1)))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::MalformedData
    );
    assert_eq!(rig.writes(), 1);
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_write_access_limits_and_address_errors_send_nothing() {
    let rig = Rig::new();
    let connection = rig.connect_uri(
        &URI.replace("&access=read-write", ""),
        ResourceLimits::builder().build().unwrap(),
    );
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42", PlcValue::I32(1)))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Unsupported(Operation::Write)
    );
    assert_eq!(rig.writes() + rig.metadata(), 0);
    block_on_with(&rig.clock, connection.close()).unwrap();
    rig.factory
        .push_script(Script::strict(vec![Step::react(rig.device.reactor())]));
    let connection = rig.connect_uri(
        URI,
        ResourceLimits::builder()
            .max_items_per_request(1)
            .build()
            .unwrap(),
    );
    let foreign = Tag::new(
        TagId::new("foreign").unwrap(),
        Arc::new(FixtureAddress::new("elsewhere", None)),
    );
    let batch = WriteRequest::builder(connection.limits())
        .item(foreign, PlcValue::I32(1))
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.write(batch))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidAddress
    );
    let batch = WriteRequest::builder(ResourceLimits::builder().build().unwrap())
        .item(tag("one", "ns=2;i=42;DINT"), PlcValue::I32(1))
        .item(tag("two", "ns=2;i=43;DINT"), PlcValue::I32(1))
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.write(batch))
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.writes() + rig.metadata(), 0);
    block_on_with(&rig.clock, connection.close()).unwrap();
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1)))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Disconnected
    );
}

fn modify(reaction: Reaction, id: u32, mutate: impl FnOnce(&mut Service)) -> Reaction {
    let Reaction::Reply(bytes) = &reaction else {
        panic!("reply")
    };
    let mut service = payload(bytes);
    mutate(&mut service);
    replace(
        reaction,
        crate::opcua::service::encode_service(id, &service, 262_144)
            .unwrap()
            .to_vec(),
    )
}
#[test]
fn opcua_metadata_errors_are_rejected_without_guessing_a_type() {
    for mode in 0..4 {
        let rig = Rig::mapped(move |service, reaction| {
            if let Service::ReadRequest { request_header, .. } = service {
                if mode == 0 {
                    return replace(
                        reaction,
                        crate::opcua::responder::close_response(request_header.request_handle, 0),
                    );
                }
                modify(reaction, 634, |service| {
                    let Service::ReadResponse {
                        response_header,
                        results,
                        ..
                    } = service
                    else {
                        panic!("read")
                    };
                    match mode {
                        1 => *results = None,
                        2 => {
                            response_header.service_result =
                                crate::opcua::codec::StatusCode(0x8001_0000);
                        }
                        3 => {
                            results.as_mut().unwrap().first_mut().unwrap().status =
                                Some(crate::opcua::codec::StatusCode(0x801F_0000));
                        }
                        _ => panic!("mode"),
                    }
                })
            } else {
                reaction
            }
        });
        let connection = rig.connect();
        for _ in 0..2 {
            let response = block_on_with(
                &rig.clock,
                connection.write(request(&*connection, "ns=2;i=42", PlcValue::I32(1))),
            )
            .unwrap();
            assert_eq!(response.rejected_count(), 1);
            assert_eq!(response.confirmed_count(), 0);
        }
        assert_eq!(rig.metadata(), 2, "failed metadata is never cached");
        assert_eq!(rig.writes(), 0);
        block_on_with(&rig.clock, connection.close()).unwrap();
    }
}
#[test]
fn opcua_metadata_timeout_stops_the_batch_before_any_write() {
    let rig = Rig::mapped(|service, reaction| {
        if matches!(service, Service::ReadRequest { .. }) {
            Reaction::Silence
        } else {
            reaction
        }
    });
    let connection = rig.connect();
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42", PlcValue::I32(1)))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(rig.writes(), 0);
    // The test peer assigned a sequence to its suppressed reply; close may reject the gap.
    let closed = block_on_with(&rig.clock, connection.close());
    assert!(closed.is_ok() || closed.unwrap_err().kind() == ErrorKind::MalformedData);
    assert_eq!(connection.state(), ConnectionState::Closed);
}
#[test]
fn opcua_local_write_failures_remain_per_item_and_keep_request_order() {
    let rig = Rig::new();
    let connection = rig.connect();
    let batch = WriteRequest::builder(connection.limits())
        .item(
            tag("attribute", "ns=2;i=42;a=DisplayName;DINT"),
            PlcValue::I32(1),
        )
        .item(
            tag("range", "ns=2;i=42[0..1];DINT"),
            PlcValue::List(vec![PlcValue::I32(1)]),
        )
        .item(tag("good", "ns=2;i=42;DINT"), PlcValue::I32(2))
        .build()
        .unwrap();
    let response = block_on_with(&rig.clock, connection.write(batch)).unwrap();
    assert_eq!(response.rejected_count(), 2);
    assert_eq!(response.confirmed_count(), 1);
    assert_eq!(
        response
            .results()
            .iter()
            .map(|r| r.tag().as_str())
            .collect::<Vec<_>>(),
        ["attribute", "range", "good"]
    );
    assert_eq!(rig.writes(), 1);
    assert_eq!(rig.metadata(), 0);
    let response = block_on_with(
        &rig.clock,
        connection.write(request(
            &*connection,
            "ns=2;i=42;a=DisplayName;DINT",
            PlcValue::I32(1),
        )),
    )
    .unwrap();
    assert_eq!(response.rejected_count(), 1);
    assert_eq!(rig.writes(), 1);
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_write_deadline_and_encoded_size_failures_precede_transmission() {
    use crate::{Clock, Deadline};
    let rig = Rig::new();
    let connection = rig.connect();
    let batch = WriteRequest::builder(connection.limits())
        .item(tag("one", "ns=2;i=42;DINT"), PlcValue::I32(1))
        .deadline(Deadline::at(rig.clock.now()))
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, connection.write(batch))
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    let value = PlcValue::String("x".repeat(connection.limits().max_buffered_bytes().get()));
    assert_eq!(
        block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42;STRING", value))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(rig.writes() + rig.metadata(), 0);
    block_on_with(&rig.clock, connection.close()).unwrap();
}
#[test]
fn opcua_wrong_write_reply_is_unknown_and_service_failure_is_typed() {
    for mode in 0..2 {
        let rig = Rig::mapped(move |service, reaction| {
            if let Service::WriteRequest { request_header, .. } = service {
                if mode == 0 {
                    return replace(
                        reaction,
                        crate::opcua::responder::close_response(request_header.request_handle, 0),
                    );
                }
                modify(reaction, 676, |service| {
                    let Service::WriteResponse {
                        response_header, ..
                    } = service
                    else {
                        panic!("write")
                    };
                    response_header.service_result = crate::opcua::codec::StatusCode(0x8001_0000);
                })
            } else {
                reaction
            }
        });
        let connection = rig.connect();
        let result = block_on_with(
            &rig.clock,
            connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1))),
        );
        if mode == 0 {
            assert_eq!(result.unwrap().unknown_count(), 1);
        } else {
            assert_eq!(result.unwrap_err().kind(), ErrorKind::DeviceException);
        }
        assert_eq!(rig.writes(), 1);
        block_on_with(&rig.clock, connection.close()).unwrap();
    }
}
#[test]
fn opcua_close_interrupts_a_pending_write_with_its_delivery_state_preserved() {
    let rig = Rig::new();
    let connection = rig.connect();
    rig.device.unknown_after_transmit();
    let mut writing = connection.write(request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1)));
    assert!(
        writing
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut closing = connection.close();
    assert!(
        closing
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let response = block_on_with(&rig.clock, writing).unwrap();
    assert_eq!(response.unknown_count(), 1);
    block_on_with(&rig.clock, closing).unwrap();
    assert_eq!(connection.state(), ConnectionState::Closed);
}

#[test]
fn opcua_write_per_tag_status_is_reported_per_tag() {
    let rig = Rig::mapped(|service, reaction| {
        if let Service::WriteRequest { request_header, .. } = service {
            replace(
                reaction,
                write_response(
                    request_header.request_handle,
                    &[0, 0x803B_0000, 0x4000_0000, 0x002E_0000],
                ),
            )
        } else {
            reaction
        }
    });
    let connection = rig.connect();
    let batch = (42..46)
        .fold(
            WriteRequest::builder(connection.limits()),
            |builder, number| {
                builder.item(
                    tag(&number.to_string(), &format!("ns=2;i={number};DINT")),
                    PlcValue::I32(1),
                )
            },
        )
        .build()
        .unwrap();
    let response = block_on_with(&rig.clock, connection.write(batch)).unwrap();
    assert_eq!(response.confirmed_count(), 2);
    assert_eq!(response.rejected_count(), 1);
    assert_eq!(response.unknown_count(), 1);
    assert_eq!(
        response
            .results()
            .iter()
            .map(|r| r.tag().as_str())
            .collect::<Vec<_>>(),
        ["42", "43", "44", "45"]
    );
    assert_eq!(
        response
            .results()
            .iter()
            .map(|r| r.status().unwrap().code())
            .collect::<Vec<_>>(),
        [0, 0x803B_0000, 0x4000_0000, 0x002E_0000]
    );
    assert_eq!(rig.writes(), 1);
    block_on_with(&rig.clock, connection.close()).unwrap();
}

#[test]
fn opcua_deep_caller_values_are_rejected_and_abandoned_without_recursive_drop() {
    let rig = Rig::new();
    let connection = rig.connect();
    // The constrained stack measures deep write/disposal, independently of handshake size.
    std::thread::Builder::new()
        .name("opcua-deep-write".to_owned())
        .stack_size(128 * 1024)
        .spawn(move || {
            for abandon in [false, true] {
                let mut value = PlcValue::I32(1);
                for _ in 0..100_000 {
                    value = PlcValue::List(vec![value]);
                }
                let future = connection.write(request(&*connection, "ns=2;i=42", value));
                if abandon {
                    drop(future);
                } else {
                    let response = block_on_with(&rig.clock, future).unwrap();
                    assert_eq!(response.rejected_count(), 1);
                    let WriteOutcome::Rejected(error) =
                        response.results().first().unwrap().outcome()
                    else {
                        panic!("rejection")
                    };
                    assert_eq!(error.kind(), ErrorKind::ResourceLimit);
                }
            }
            assert_eq!(rig.writes(), 0);
            block_on_with(&rig.clock, connection.close()).unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn opcua_write_mapper_refuses_a_non_write_service() {
    let rig = Rig::new();
    let connection = rig.connect();
    let request = request(&*connection, "ns=2;i=42;DINT", PlcValue::I32(1));
    let reply = decode_service(&crate::opcua::responder::close_response(1, 0)).unwrap();
    assert_eq!(
        response(
            &request,
            vec![TagId::new("setpoint").unwrap()],
            vec![],
            reply,
            rig.clock.now(),
            rig.clock.now()
        )
        .unwrap_err()
        .kind(),
        ErrorKind::MalformedData
    );
    block_on_with(&rig.clock, connection.close()).unwrap();
}
