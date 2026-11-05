#![expect(
    clippy::indexing_slicing,
    clippy::wildcard_enum_match_arm,
    clippy::unreachable,
    reason = "wire fixtures assert their exact shape and reject unexpected requests"
)]
use super::*;
use crate::codec::BoundedReader;
use crate::opcua::OpcuaDriver;
use crate::opcua::codec::{Depth, LocalizedText, QualifiedName, UaString};
use crate::opcua::connection::OpcuaConnection;
use crate::opcua::frame::{Body, OpcuaFramer};
use crate::opcua::generated::node_class::NodeClass;
use crate::opcua::generated::response_header::ResponseHeader;
use crate::opcua::responder::{Device, message, response_header, service_fault};
use crate::opcua::service::{decode_service, encode_service};
use crate::testkit::{FakeClock, FakeTransportFactory, Reaction, Script, Step, block_on_with};
use crate::transport::Framer;
use crate::{BrowseRequest, Connection, ConnectionConfig, Driver, ResourceLimits, TagId};
use bytes::BytesMut;
use std::future::Future;
use std::time::Duration;

const URI: &str = "opcua://fake:4840/path?security-policy=NONE&request-timeout-ms=25";

fn header(handle: u32, status: u32) -> ResponseHeader {
    ResponseHeader::parse(
        &mut BoundedReader::new(&response_header(handle, status)),
        Depth::new(),
        394,
    )
    .unwrap()
}
fn page(handle: u32, next: bool, results: Vec<Page>) -> Reaction {
    let response_header = header(handle, 0);
    let service = if next {
        Service::BrowseNextResponse {
            response_header,
            results: Some(results),
            diagnostic_infos: None,
        }
    } else {
        Service::BrowseResponse {
            response_header,
            results: Some(results),
            diagnostic_infos: None,
        }
    };
    Reaction::Reply(
        encode_service(if next { 536 } else { 530 }, &service, 1_000_000)
            .unwrap()
            .to_vec(),
    )
}
fn result(point: &[u8], references: Vec<ReferenceDescription>) -> Page {
    Page {
        status_code: StatusCode(0),
        continuation_point: UaByteString::new(point.to_vec()),
        references: Some(references),
    }
}
fn reference(node: NodeId, name: &str) -> ReferenceDescription {
    ReferenceDescription {
        reference_type_id: NodeId::numeric(0, 35),
        is_forward: true,
        node_id: ExpandedNodeId::local(node),
        browse_name: QualifiedName {
            namespace_index: 1,
            name: UaString::new(name),
        },
        display_name: LocalizedText {
            locale: None,
            text: Some(UaString::new(name)),
        },
        node_class: NodeClass::NodeClassObject,
        type_definition: ExpandedNodeId::local(NodeId::numeric(0, 0)),
    }
}
fn child(number: u32) -> ReferenceDescription {
    reference(NodeId::numeric(1, number), "same")
}

struct Rig {
    connection: OpcuaConnection,
    clock: FakeClock,
    sent: Arc<Mutex<Vec<Service>>>,
    factory: FakeTransportFactory,
}
impl Rig {
    fn new(options: &str, handler: impl FnMut(Service, u32) -> Reaction + Send + 'static) -> Self {
        Self::limited(options, ResourceLimits::builder().build().unwrap(), handler)
    }
    fn limited(
        options: &str,
        limits: ResourceLimits,
        mut handler: impl FnMut(Service, u32) -> Reaction + Send + 'static,
    ) -> Self {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::clone(&sent);
        let mut base = Device::default().reactor();
        let mut sequence = 41_u32;
        factory.push_script(Script::strict(vec![Step::react(move |bytes| {
            if !bytes.starts_with(b"MSG") {
                return base(bytes);
            }
            let frame = OpcuaFramer::new(65535, limits)
                .decode(&mut BytesMut::from(bytes))
                .unwrap()
                .unwrap();
            let Body::Message(chunk) = frame.body else {
                panic!("message")
            };
            let service = decode_service(&chunk.body).unwrap();
            log.lock().unwrap().push(service.clone());
            let reaction = match service {
                Service::BrowseRequest { .. } | Service::BrowseNextRequest { .. } => {
                    match handler(service, chunk.sequence.request_id) {
                        Reaction::Reply(body) => {
                            Reaction::Reply(message(0, chunk.sequence.request_id, 17, 29, &body))
                        }
                        other => other,
                    }
                }
                _ => base(bytes),
            };
            if let Reaction::Reply(mut bytes) = reaction {
                sequence = sequence.checked_add(1).unwrap();
                bytes[16..20].copy_from_slice(&sequence.to_le_bytes());
                Reaction::Reply(bytes)
            } else {
                reaction
            }
        })]));
        let driver = OpcuaDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let parsed = driver.parse_config(&format!("{URI}{options}")).unwrap();
        let config = ConnectionConfig::builder("opcua", parsed.endpoint().clone())
            .timeouts(*parsed.timeouts())
            .limits(limits)
            .driver_options(Arc::new(
                parsed.driver_options_as::<OpcuaOptions>().unwrap().clone(),
            ))
            .build()
            .unwrap();
        let session = block_on_with(
            &clock,
            driver.open_session(
                &config,
                Deadline::after(clock.now(), Duration::from_secs(1)).unwrap(),
                &CancellationToken::new(),
            ),
        )
        .unwrap();
        Self {
            connection: OpcuaConnection::new(session, driver, config, Arc::new(clock.clone())),
            clock,
            sent,
            factory,
        }
    }
    #[expect(
        clippy::unused_self,
        reason = "request construction belongs to the test rig"
    )]
    fn request(&self, queries: &[&str], max: usize) -> BrowseRequest {
        let limits = ResourceLimits::builder()
            .max_browse_nodes(max)
            .build()
            .unwrap();
        queries
            .iter()
            .enumerate()
            .fold(BrowseRequest::builder(limits), |builder, (i, text)| {
                builder.query(TagId::new(&i.to_string()).unwrap(), text)
            })
            .build()
            .unwrap()
    }
    fn browse(&self, query: &str) -> crate::BrowseResponse {
        block_on_with(
            &self.clock,
            self.connection.browse_walk(self.request(&[query], 100)),
        )
        .unwrap()
    }
    fn services(&self) -> Vec<Service> {
        self.sent.lock().unwrap().clone()
    }
}

#[test]
fn opcua_browse_walk_pages_cycles_siblings_and_child_status() {
    let mut calls = 0;
    let rig = Rig::new("", move |service, handle| {
        calls += 1;
        match (calls, service) {
            (
                1,
                Service::BrowseRequest {
                    nodes_to_browse,
                    requested_max_references_per_node,
                    view,
                    ..
                },
            ) => {
                assert_eq!(requested_max_references_per_node, 65536);
                assert_eq!(view.view_id, NodeId::numeric(0, 0));
                assert_eq!((view.timestamp, view.view_version), (0, 0));
                let nodes = nodes_to_browse.unwrap();
                assert_eq!(nodes.len(), 1);
                let description = &nodes[0];
                assert_eq!(description.node_id, NodeId::numeric(0, 85));
                assert_eq!(description.reference_type_id, NodeId::numeric(0, 33));
                assert_eq!(
                    description.browse_direction,
                    BrowseDirection::BrowseDirectionForward
                );
                assert!(description.include_subtypes);
                assert_eq!(
                    (description.node_class_mask, description.result_mask),
                    (0, 63)
                );
                page(
                    handle,
                    false,
                    vec![result(b"first", vec![child(1), child(2)])],
                )
            }
            (
                2,
                Service::BrowseNextRequest {
                    release_continuation_points,
                    continuation_points,
                    ..
                },
            ) => {
                assert!(!release_continuation_points);
                assert_eq!(
                    continuation_points.unwrap()[0].as_bytes(),
                    Some(b"first".as_slice())
                );
                page(handle, true, vec![result(b"second", vec![])])
            }
            (
                3,
                Service::BrowseNextRequest {
                    continuation_points,
                    ..
                },
            ) => {
                assert_eq!(
                    continuation_points.unwrap()[0].as_bytes(),
                    Some(b"second".as_slice())
                );
                page(
                    handle,
                    true,
                    vec![result(
                        b"",
                        vec![child(3), reference(NodeId::numeric(0, 85), "cycle")],
                    )],
                )
            }
            (4, Service::BrowseRequest { .. }) => page(
                handle,
                false,
                vec![Page {
                    status_code: StatusCode(0x8034_0000),
                    ..result(b"", vec![])
                }],
            ),
            (5, Service::BrowseRequest { .. }) => {
                page(handle, false, vec![result(b"", vec![child(4), child(1)])])
            }
            (6 | 7, Service::BrowseRequest { .. }) => {
                page(handle, false, vec![result(b"", vec![])])
            }
            _ => panic!("unexpected browse {calls}"),
        }
    });
    let response = rig.browse(" ** ");
    let item = &response.items()[0];
    assert!(!item.truncated());
    let nodes = item.outcome().as_ref().unwrap();
    assert_eq!(nodes.len(), 4);
    assert_eq!(
        nodes.iter().map(BrowseNode::name).collect::<Vec<_>>(),
        ["same"; 4]
    );
    assert_eq!(
        nodes.iter().map(BrowseNode::parent).collect::<Vec<_>>(),
        [None, None, None, Some(1)]
    );
    assert_eq!(nodes[0].error().unwrap().kind(), ErrorKind::NotFound);
    assert_eq!(
        nodes[0].error().unwrap().status().unwrap().code(),
        0x8034_0000
    );
    assert!(nodes[1].error().is_none());
    assert!(!rig.connection.capabilities().browse);
    block_on_with(&rig.clock, rig.connection.close()).unwrap();
}

#[test]
fn opcua_browse_limits_release_points_and_keep_already_discovered_siblings() {
    for options in [
        "&browse-max-references-per-node=1",
        "&browse-max-total-nodes=2",
    ] {
        let rig = Rig::new(options, |service, handle| match service {
            Service::BrowseRequest {
                nodes_to_browse, ..
            } if nodes_to_browse.as_ref().unwrap()[0].node_id == NodeId::numeric(0, 85) => page(
                handle,
                false,
                vec![result(b"remaining", vec![child(1), child(2)])],
            ),
            Service::BrowseRequest { .. } => page(handle, false, vec![result(b"", vec![])]),
            Service::BrowseNextRequest {
                release_continuation_points,
                continuation_points,
                ..
            } => {
                assert!(release_continuation_points);
                assert_eq!(
                    continuation_points.unwrap()[0].as_bytes(),
                    Some(b"remaining".as_slice())
                );
                page(handle, true, vec![])
            }
            _ => unreachable!(),
        });
        let response = rig.browse("");
        assert!(response.items()[0].truncated());
        assert_eq!(response.items()[0].outcome().as_ref().unwrap().len(), 1);
        assert_eq!(
            rig.services()
                .iter()
                .filter(|s| matches!(
                    s,
                    Service::BrowseNextRequest {
                        release_continuation_points: true,
                        ..
                    }
                ))
                .count(),
            1
        );
    }
    let rig = Rig::new("&browse-max-depth=1", |_service, handle| {
        page(handle, false, vec![result(b"", vec![child(1), child(2)])])
    });
    let response = rig.browse("");
    assert!(response.items()[0].truncated());
    assert_eq!(response.items()[0].outcome().as_ref().unwrap().len(), 2);
    assert_eq!(
        rig.services()
            .iter()
            .filter(|s| matches!(s, Service::BrowseRequest { .. }))
            .count(),
        1
    );
}

#[test]
fn opcua_browse_core_budget_is_shared_across_queries_and_connection_bound() {
    let limits = ResourceLimits::builder()
        .max_browse_nodes(1)
        .build()
        .unwrap();
    let rig = Rig::limited("", limits, |service, handle| match service {
        Service::BrowseRequest {
            nodes_to_browse, ..
        } if nodes_to_browse.as_ref().unwrap()[0].node_id == NodeId::numeric(0, 85) => {
            page(handle, false, vec![result(b"", vec![child(1), child(2)])])
        }
        _ => page(handle, false, vec![result(b"", vec![])]),
    });
    let response = block_on_with(
        &rig.clock,
        rig.connection.browse_walk(rig.request(&["", "**"], 10)),
    )
    .unwrap();
    assert_eq!(response.items()[0].outcome().as_ref().unwrap().len(), 1);
    assert!(response.items()[0].truncated());
    assert!(response.items()[1].outcome().as_ref().unwrap().is_empty());
    assert!(response.items()[1].truncated());
}

#[test]
fn opcua_browse_queries_keep_errors_and_status_without_discarding_siblings() {
    let rig = Rig::new("", |service, handle| match service {
        Service::BrowseRequest {
            nodes_to_browse, ..
        } if nodes_to_browse.as_ref().unwrap()[0].node_id == NodeId::numeric(1, 9) => {
            Reaction::Reply(service_fault(handle, 0x801F_0000))
        }
        _ => page(
            handle,
            false,
            vec![Page {
                status_code: StatusCode(0x8034_0000),
                ..result(b"", vec![])
            }],
        ),
    });
    let response = block_on_with(
        &rig.clock,
        rig.connection
            .browse_walk(rig.request(&["nonsense", "ns=1;i=9", ""], 100)),
    )
    .unwrap();
    assert_eq!(
        response.items()[0].outcome().as_ref().unwrap_err().kind(),
        ErrorKind::InvalidAddress
    );
    assert_eq!(
        response.items()[1].outcome().as_ref().unwrap_err().kind(),
        ErrorKind::AccessDenied
    );
    assert_eq!(
        response.items()[1]
            .outcome()
            .as_ref()
            .unwrap_err()
            .status()
            .unwrap()
            .code(),
        0x801F_0000
    );
    assert_eq!(
        response.items()[2].outcome().as_ref().unwrap_err().kind(),
        ErrorKind::NotFound
    );
}

#[test]
fn opcua_browse_zero_protocol_limits_keep_guid_bytestring_and_string_nodes() {
    let mut first = true;
    let rig = Rig::new(
        "&browse-max-references-per-node=0&browse-max-total-nodes=0&browse-max-depth=0",
        move |service, handle| {
            let Service::BrowseRequest {
                requested_max_references_per_node,
                ..
            } = service
            else {
                panic!("browse")
            };
            assert_eq!(requested_max_references_per_node, 0);
            let refs = if std::mem::take(&mut first) {
                [
                    "ns=2;g=00112233-4455-6677-8899-aabbccddeeff",
                    "ns=2;b=AQID",
                    "ns=2;s=string",
                ]
                .iter()
                .map(|address| {
                    let node = read::node(&crate::Tag::new(
                        TagId::new("node").unwrap(),
                        Arc::new(OpcuaAddress::parse(address).unwrap()),
                    ))
                    .unwrap()
                    .node_id;
                    let mut reference = reference(node, "fallback");
                    reference.display_name.text = None;
                    reference
                })
                .collect()
            } else {
                vec![]
            };
            page(handle, false, vec![result(b"", refs)])
        },
    );
    let response = rig.browse("");
    let nodes = response.items()[0].outcome().as_ref().unwrap();
    assert_eq!(
        nodes
            .iter()
            .map(|node| node.address().canonical())
            .collect::<Vec<_>>(),
        [
            "ns=2;g=00112233-4455-6677-8899-aabbccddeeff",
            "ns=2;b=AQID",
            "ns=2;s=string"
        ]
    );
    assert!(nodes.iter().all(|node| node.name() == "fallback"));
    assert!(!response.items()[0].truncated());
}

#[test]
fn opcua_browse_unknown_continuation_after_timeout_cancel_or_drop_retires_lane() {
    for cancel in [false, true] {
        let rig = Rig::new("", |_service, _handle| Reaction::Silence);
        let token = CancellationToken::new();
        let request = BrowseRequest::builder(rig.connection.limits())
            .query(TagId::new("root").unwrap(), "")
            .cancel(token.clone())
            .build()
            .unwrap();
        if cancel {
            let mut future = std::pin::pin!(rig.connection.browse_walk(request));
            let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(Future::poll(future.as_mut(), &mut cx).is_pending());
            token.cancel();
            assert_eq!(
                block_on_with(&rig.clock, future).unwrap_err().kind(),
                ErrorKind::Cancelled
            );
        } else {
            assert_eq!(
                block_on_with(&rig.clock, rig.connection.browse_walk(request))
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );
        }
        assert!(rig.connection.current().lane.is_failed());
    }
    let rig = Rig::new("", |_service, _handle| Reaction::Silence);
    let request = rig.request(&[""], 100);
    {
        let mut future = std::pin::pin!(rig.connection.browse_walk(request));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(Future::poll(future.as_mut(), &mut cx).is_pending());
    }
    assert!(rig.connection.current().lane.is_failed());
}

#[test]
fn opcua_browse_malformed_cardinality_retains_known_points_for_next_caller() {
    let rig = Rig::new("", |service, handle| match service {
        Service::BrowseRequest { .. } => page(
            handle,
            false,
            vec![result(b"first", vec![]), result(b"second", vec![])],
        ),
        Service::BrowseNextRequest {
            release_continuation_points,
            ..
        } => {
            assert!(release_continuation_points);
            page(handle, true, vec![])
        }
        _ => unreachable!(),
    });
    assert_eq!(
        block_on_with(
            &rig.clock,
            rig.connection.browse_walk(rig.request(&[""], 100))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::MalformedData
    );
    let session = rig.connection.current();
    assert_eq!(session.continuations.pending.lock().unwrap().len(), 2);
    let tag = crate::Tag::new(
        TagId::new("counter").unwrap(),
        Arc::new(OpcuaAddress::parse("ns=1;i=42").unwrap()),
    );
    let request = crate::ReadRequest::builder(rig.connection.limits())
        .tag(tag)
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, rig.connection.read(request))
            .unwrap()
            .iter_ok()
            .count(),
        1
    );
    assert!(session.continuations.pending.lock().unwrap().is_empty());
}

#[test]
fn opcua_browse_early_validation_does_no_io() {
    let rig = Rig::new("", |_service, _handle| panic!("no browse I/O"));
    let before = rig.factory.probes()[0].sent().len();
    assert!(rig.browse("invalid").items()[0].outcome().is_err());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let request = BrowseRequest::builder(rig.connection.limits())
        .query(TagId::new("root").unwrap(), "")
        .cancel(cancelled)
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, rig.connection.browse_walk(request))
            .unwrap_err()
            .kind(),
        ErrorKind::Cancelled
    );
    assert_eq!(rig.factory.probes()[0].sent().len(), before);
    block_on_with(&rig.clock, rig.connection.close()).unwrap();
    assert_eq!(
        block_on_with(
            &rig.clock,
            rig.connection.browse_walk(rig.request(&[""], 100))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::Disconnected
    );
}

#[test]
fn opcua_browse_release_refusals_and_malformed_replies_retire_cleanup_lane() {
    for case in 0..6 {
        let rig = Rig::new("", move |_service, handle| match case {
            0 => Reaction::Reply(
                encode_service(
                    634,
                    &Service::ReadResponse {
                        response_header: header(handle, 0),
                        results: None,
                        diagnostic_infos: None,
                    },
                    1000,
                )
                .unwrap()
                .to_vec(),
            ),
            1 => page(handle, true, vec![result(b"", vec![])]),
            2 => Reaction::Reply(service_fault(handle, 0x8001_0000)),
            3 => Reaction::Reply(
                encode_service(
                    536,
                    &Service::BrowseNextResponse {
                        response_header: header(handle, 0x8001_0000),
                        results: None,
                        diagnostic_infos: None,
                    },
                    1000,
                )
                .unwrap()
                .to_vec(),
            ),
            4 => Reaction::Reply(
                encode_service(
                    536,
                    &Service::BrowseNextResponse {
                        response_header: header(handle, 0),
                        results: None,
                        diagnostic_infos: Some(
                            vec![crate::opcua::codec::DiagnosticInfo::default()],
                        ),
                    },
                    1000,
                )
                .unwrap()
                .to_vec(),
            ),
            _ => Reaction::Silence,
        });
        let session = rig.connection.current();
        session
            .continuations
            .pending
            .lock()
            .unwrap()
            .push(UaByteString::new(vec![1]));
        let error = block_on_with(
            &rig.clock,
            release_pending(
                &session,
                &rig.clock,
                Deadline::after(rig.clock.now(), Duration::from_millis(10)).unwrap(),
                &CancellationToken::new(),
            ),
        )
        .unwrap_err();
        if case == 5 {
            assert_eq!(error.kind(), ErrorKind::Timeout);
            assert!(!session.lane.is_failed());
            assert_eq!(session.continuations.pending.lock().unwrap().len(), 1);
        } else {
            assert!(session.lane.is_failed());
        }
    }
}

#[test]
fn opcua_browse_pending_cleanup_is_bounded_by_count_and_bytes() {
    for bytes in [false, true] {
        let limits = ResourceLimits::builder()
            .max_pending_requests(1)
            .build()
            .unwrap();
        let rig = Rig::limited("", limits, |_service, _handle| panic!("no I/O"));
        let session = rig.connection.current();
        {
            let mut points = Points::new(&session);
            points.known = if bytes {
                vec![UaByteString::new(vec![
                    1;
                    limits
                        .max_buffered_bytes()
                        .get()
                        .checked_add(1)
                        .unwrap()
                ])]
            } else {
                vec![UaByteString::new(vec![1]), UaByteString::new(vec![2])]
            };
        }
        assert_eq!(session.lane.failure_kind(), Some(ErrorKind::ResourceLimit));
        assert!(session.continuations.pending.lock().unwrap().is_empty());
    }
}

#[test]
fn opcua_browse_rejects_foreign_services_and_handles() {
    for wrong_handle in [false, true] {
        let rig = Rig::new("", move |_service, handle| {
            if wrong_handle {
                page(
                    handle.checked_add(1).unwrap(),
                    false,
                    vec![result(b"", vec![])],
                )
            } else {
                page(handle, true, vec![result(b"", vec![])])
            }
        });
        let error = block_on_with(
            &rig.clock,
            rig.connection.browse_walk(rig.request(&[""], 100)),
        )
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        assert!(rig.connection.current().lane.is_failed());
    }
}

#[test]
fn opcua_browse_remote_identifiers_invalid_names_and_source_mapping_are_explicit() {
    let options = OpcuaOptions::from_parameters("opc.tcp://fake:4840".to_owned(), &[]).unwrap();
    for server in [false, true] {
        let mut walk = Walk::new(NodeId::numeric(0, 85), &options, 100).unwrap();
        let work = walk.pending.pop_front().unwrap();
        let mut child = child(1);
        if server {
            child.node_id.server_index = Some(1);
        } else {
            child.node_id.namespace_uri = Some(UaString::new("urn:remote"));
        }
        assert_eq!(
            walk.reference(child, &work).unwrap_err().kind(),
            ErrorKind::Unsupported(Operation::Browse)
        );
    }
    let mut walk = Walk::new(NodeId::numeric(0, 85), &options, 100).unwrap();
    let work = walk.pending.pop_front().unwrap();
    let mut invalid = child(1);
    invalid.display_name.text = Some(UaString::from_bytes(vec![0xff]));
    assert_eq!(
        walk.reference(invalid, &work).unwrap_err().kind(),
        ErrorKind::MalformedData
    );
    assert!(service_status(&PlcError::new(ErrorKind::DeviceException, "no source")).is_none());
    assert!(
        service_item(PlcError::new(ErrorKind::DeviceException, "no source"))
            .status()
            .is_none()
    );
    assert_eq!(
        service_status(
            &PlcError::new(ErrorKind::DeviceException, "wrapped")
                .with_source(crate::opcua::service::status_error(StatusCode(0x8034_0000)))
        ),
        Some(StatusCode(0x8034_0000))
    );
}

#[test]
fn opcua_browse_service_status_releases_known_point_and_marks_only_the_refused_node() {
    let mut calls = 0;
    let rig = Rig::new("", move |service, handle| {
        calls += 1;
        match (calls, service) {
            (1, _) => page(handle, false, vec![result(b"", vec![child(1), child(2)])]),
            (2, _) => page(handle, false, vec![result(b"live", vec![])]),
            (3, _) => Reaction::Reply(service_fault(handle, 0x801F_0000)),
            (
                4,
                Service::BrowseNextRequest {
                    release_continuation_points,
                    ..
                },
            ) => {
                assert!(release_continuation_points);
                page(handle, true, vec![])
            }
            (5, _) => page(handle, false, vec![result(b"", vec![])]),
            _ => panic!("unexpected call"),
        }
    });
    let response = rig.browse("");
    let nodes = response.items()[0].outcome().as_ref().unwrap();
    assert_eq!(nodes[0].error().unwrap().kind(), ErrorKind::AccessDenied);
    assert!(nodes[1].error().is_none());
}

#[test]
fn opcua_browse_bad_service_header_preserves_old_point_for_release() {
    let mut calls = 0;
    let rig = Rig::new("", move |_service, handle| {
        calls += 1;
        match calls {
            1 => page(handle, false, vec![result(b"original", vec![])]),
            2 => Reaction::Reply(
                encode_service(
                    536,
                    &Service::BrowseNextResponse {
                        response_header: header(handle, 0x8034_0000),
                        results: None,
                        diagnostic_infos: None,
                    },
                    1000,
                )
                .unwrap()
                .to_vec(),
            ),
            3 => page(handle, true, vec![]),
            _ => panic!("unexpected request"),
        }
    });
    let response = rig.browse("");
    assert_eq!(
        response.items()[0].outcome().as_ref().unwrap_err().kind(),
        ErrorKind::NotFound
    );
    assert!(
        rig.connection
            .current()
            .continuations
            .pending
            .lock()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        rig.services()
            .iter()
            .filter(|s| matches!(
                s,
                Service::BrowseNextRequest {
                    release_continuation_points: true,
                    ..
                }
            ))
            .count(),
        1
    );
}

#[test]
fn opcua_browse_actual_query_limit_and_elapsed_deadline_do_no_io() {
    let rig = Rig::limited(
        "",
        ResourceLimits::builder()
            .max_items_per_request(1)
            .build()
            .unwrap(),
        |_service, _handle| panic!("no browse I/O"),
    );
    let before = rig.factory.probes()[0].sent().len();
    assert_eq!(
        block_on_with(
            &rig.clock,
            rig.connection.browse_walk(rig.request(&["", "**"], 100))
        )
        .unwrap_err()
        .kind(),
        ErrorKind::ResourceLimit
    );
    let request = BrowseRequest::builder(rig.connection.limits())
        .query(TagId::new("root").unwrap(), "")
        .deadline(Deadline::at(rig.clock.now()))
        .build()
        .unwrap();
    assert_eq!(
        block_on_with(&rig.clock, rig.connection.browse_walk(request))
            .unwrap_err()
            .kind(),
        ErrorKind::Timeout
    );
    assert_eq!(rig.factory.probes()[0].sent().len(), before);
}

#[test]
fn opcua_browse_empty_continuation_pages_remain_cancellable_with_zero_protocol_limits() {
    let token = CancellationToken::new();
    let stop = token.clone();
    let mut pages = 0;
    let rig = Rig::new(
        "&browse-max-references-per-node=0&browse-max-total-nodes=0&browse-max-depth=0",
        move |service, handle| {
            pages += 1;
            let next = matches!(service, Service::BrowseNextRequest { .. });
            if pages == 4 {
                stop.cancel();
            }
            page(handle, next, vec![result(b"same-point", vec![])])
        },
    );
    let request = BrowseRequest::builder(rig.connection.limits())
        .query(TagId::new("root").unwrap(), "")
        .cancel(token)
        .build()
        .unwrap();
    let error = block_on_with(&rig.clock, rig.connection.browse_walk(request)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Cancelled);
    assert_eq!(
        rig.services()
            .iter()
            .filter(|s| matches!(
                s,
                Service::BrowseRequest { .. } | Service::BrowseNextRequest { .. }
            ))
            .count(),
        4
    );
}

#[test]
fn opcua_browse_dropping_a_pending_release_requeues_only_known_points() {
    let rig = Rig::new("", |_service, _handle| Reaction::Silence);
    let session = rig.connection.current();
    session
        .continuations
        .pending
        .lock()
        .unwrap()
        .push(UaByteString::new(vec![1]));
    let cancel = CancellationToken::new();
    {
        let mut release = std::pin::pin!(release_pending(
            &session,
            &rig.clock,
            Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap(),
            &cancel
        ));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(Future::poll(release.as_mut(), &mut cx).is_pending());
    }
    assert!(!session.lane.is_failed());
    assert_eq!(session.continuations.pending.lock().unwrap().len(), 1);
}

#[test]
fn opcua_browse_malformed_next_reply_and_pre_send_cancel_preserve_original_point() {
    for cancelled in [false, true] {
        let rig = Rig::new("", |_service, handle| page(handle, true, vec![]));
        let session = rig.connection.current();
        let cancel = CancellationToken::new();
        if cancelled {
            cancel.cancel();
        }
        {
            let mut points = Points::new(&session);
            points.known.push(UaByteString::new(b"original".to_vec()));
            let error = block_on_with(
                &rig.clock,
                points.page(
                    &NodeId::numeric(0, 85),
                    65536,
                    Deadline::after(rig.clock.now(), Duration::from_secs(1)).unwrap(),
                    &cancel,
                ),
            )
            .unwrap_err();
            assert_eq!(
                error.kind(),
                if cancelled {
                    ErrorKind::Cancelled
                } else {
                    ErrorKind::MalformedData
                }
            );
        }
        assert!(!session.lane.is_failed());
        let pending = session.continuations.pending.lock().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].as_bytes(), Some(b"original".as_slice()));
    }
}
