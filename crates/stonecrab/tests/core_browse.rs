//! The public bounded browse data contract, independent of any driver or runtime.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
    use stonecrab::{
        BrowseNode, BrowseQuery, BrowseRequest, BrowseResponse, BrowseResult, CancellationToken,
        Deadline, ErrorKind, ItemError, PlcError, PlcValueType, ProtocolAddress, ProtocolStatus,
        ResourceLimits, Tag, TagId,
    };

    #[derive(Debug)]
    struct Address(Option<PlcValueType>);
    impl ProtocolAddress for Address {
        fn protocol(&self) -> &'static str {
            "browse-test"
        }
        fn canonical(&self) -> String {
            "node".to_owned()
        }
        fn value_type_hint(&self) -> Option<PlcValueType> {
            self.0
        }
    }

    fn id(name: &str) -> TagId {
        TagId::new(name).unwrap()
    }
    fn node(name: &str) -> BrowseNode {
        BrowseNode::new(Arc::new(Address(None)), name).unwrap()
    }
    fn limits(queries: usize, nodes: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_items_per_request(queries)
            .max_browse_nodes(nodes)
            .build()
            .unwrap()
    }
    fn request() -> BrowseRequest {
        BrowseRequest::builder(limits(3, 4))
            .query(id("first"), "")
            .query(id("second"), "ns=2;i=42")
            .query(id("third"), "invalid query is per-query data")
            .build()
            .unwrap()
    }
    fn failure(name: &str) -> BrowseResult {
        BrowseResult::new(
            id(name),
            Err(
                ItemError::new(PlcError::new(ErrorKind::NotFound, "node absent"))
                    .with_status(ProtocolStatus::new("opcua", 0x8034_0000)),
            ),
        )
    }

    #[test]
    fn browse_queries_keep_order_opaque_text_bounds_deadlines_and_shared_cancellation() {
        let token = CancellationToken::new();
        let ignored = CancellationToken::new();
        let deadline = Deadline::after(Instant::now(), Duration::from_secs(1)).unwrap();
        let request = BrowseRequest::builder(limits(2, 7))
            .cancel(ignored.clone())
            .cancel(token.clone())
            .deadline(deadline)
            .query(id("blank"), "")
            .query(id("opaque"), "\0\nnot yet a node")
            .build()
            .unwrap()
            .clone();
        assert_eq!(
            request
                .queries()
                .iter()
                .map(|q| q.id().as_str())
                .collect::<Vec<_>>(),
            ["blank", "opaque"]
        );
        assert_eq!(
            request.queries().get(1).unwrap().text(),
            "\0\nnot yet a node"
        );
        assert_eq!(request.deadline(), Some(deadline));
        assert_eq!(request.max_nodes().get(), 7);
        ignored.cancel();
        assert!(!request.cancel().unwrap().is_cancelled());
        token.cancel();
        assert!(request.cancel().unwrap().is_cancelled());
        let request = BrowseRequest::builder(limits(1, 1))
            .query(id("root"), "")
            .build()
            .unwrap();
        assert_eq!(request.deadline(), None);
        assert!(request.cancel().is_none());
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(
            BrowseRequest::builder(limits(1, 1))
                .query(id("root"), "")
                .cancel(cancelled)
                .build()
                .unwrap()
                .cancel()
                .unwrap()
                .is_cancelled()
        );
    }

    #[test]
    fn browse_query_validation_is_sticky_and_rejections_and_debug_do_not_echo_text() {
        let secret = "sensitive-query-value";
        let query = BrowseQuery::new(id("query"), secret).unwrap();
        assert!(!format!("{query:?}").contains(secret));
        assert!(format!("{query:?}").contains("text_bytes"));
        let maximum = "é".repeat(BrowseQuery::MAX_TEXT_BYTES.checked_div(2).unwrap());
        assert_eq!(
            BrowseQuery::new(id("max"), &maximum).unwrap().text(),
            maximum
        );
        let oversized = format!("{maximum}x");
        let error = BrowseQuery::new(id("large"), &oversized).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        assert!(!error.to_string().contains(&oversized));
        assert_eq!(
            BrowseRequest::builder(limits(2, 1))
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
        let duplicate = BrowseRequest::builder(limits(3, 1))
            .query(id("same"), "")
            .query(id("same"), "")
            .query(id("later"), &oversized)
            .build()
            .unwrap_err();
        assert_eq!(duplicate.kind(), ErrorKind::InvalidConfig);
        assert!(duplicate.to_string().contains("duplicate"));
        assert_eq!(
            BrowseRequest::builder(limits(1, 1))
                .query(id("first"), "")
                .query(id("second"), "")
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(
            BrowseRequest::builder(limits(2, 1))
                .query(id("large"), &oversized)
                .query(id("next"), "")
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
    }

    #[test]
    fn browse_nodes_keep_access_type_shape_options_and_reusable_addresses() {
        let address: Arc<dyn ProtocolAddress> = Arc::new(Address(Some(PlcValueType::I32)));
        let plain = BrowseNode::new(Arc::clone(&address), "温度").unwrap();
        assert!(!plain.readable() && !plain.writable());
        assert_eq!(plain.parent(), None);
        assert_eq!(plain.value_type(), Some(PlcValueType::I32));
        assert_eq!(plain.array_dimensions(), None);
        assert!(plain.options().is_empty());
        let item = plain
            .with_readable(true)
            .with_writable(true)
            .with_parent(0)
            .with_type(PlcValueType::I32)
            .unwrap()
            .with_array_dimensions(&[2, 0])
            .unwrap()
            .with_option("node-class", "Variable")
            .unwrap()
            .with_option("node-class", "updated")
            .unwrap()
            .clone();
        assert_eq!(item.name(), "温度");
        assert!(item.readable() && item.writable());
        assert_eq!(item.parent(), Some(0));
        assert_eq!(item.array_dimensions(), Some([2, 0].as_slice()));
        assert_eq!(item.options().len(), 1);
        assert_eq!(item.options().get("node-class").unwrap(), "updated");
        assert!(Arc::ptr_eq(item.address(), &address));
        let tag = Tag::new(id("value"), Arc::clone(item.address()));
        assert_eq!(tag.value_type(), Some(PlcValueType::I32));
        assert_eq!(
            node("").with_type(PlcValueType::Bool).unwrap().value_type(),
            Some(PlcValueType::Bool)
        );
        assert_eq!(
            node("")
                .with_array_dimensions(&[])
                .unwrap()
                .array_dimensions(),
            Some([].as_slice())
        );
        assert_eq!(node("").value_type(), None);
        assert_eq!(
            BrowseNode::new(address, "")
                .unwrap()
                .with_type(PlcValueType::Bool)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
    }

    #[test]
    fn browse_metadata_is_bounded_before_copying_and_full_options_allow_replacement() {
        let text = "x".repeat(BrowseNode::MAX_TEXT_BYTES);
        assert_eq!(node(&text).name().len(), BrowseNode::MAX_TEXT_BYTES);
        assert_eq!(
            BrowseNode::new(Arc::new(Address(None)), &format!("{text}x"))
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        let dimensions = [u32::MAX; BrowseNode::MAX_ARRAY_DIMENSIONS];
        assert_eq!(
            node("")
                .with_array_dimensions(&dimensions)
                .unwrap()
                .array_dimensions(),
            Some(dimensions.as_slice())
        );
        assert_eq!(
            node("")
                .with_array_dimensions(&[1; BrowseNode::MAX_ARRAY_DIMENSIONS + 1])
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        for key in ["", "bad\nkey"] {
            assert_eq!(
                node("").with_option(key, "").unwrap_err().kind(),
                ErrorKind::InvalidConfig
            );
        }
        let key = "k".repeat(BrowseNode::MAX_OPTION_KEY_BYTES);
        assert_eq!(
            node("")
                .with_option(&key, &text)
                .unwrap()
                .options()
                .get(&key)
                .unwrap(),
            &text
        );
        assert_eq!(
            node("")
                .with_option(&format!("{key}k"), "")
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(
            node("")
                .with_option("key", &format!("{text}x"))
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        let mut item = node("many");
        for index in 0..BrowseNode::MAX_OPTIONS {
            item = item.with_option(&format!("key{index}"), "value").unwrap();
        }
        let item = item.with_option("key0", "replacement").unwrap();
        assert_eq!(item.options().len(), BrowseNode::MAX_OPTIONS);
        assert_eq!(
            item.with_option("overflow", "secret").unwrap_err().kind(),
            ErrorKind::ResourceLimit
        );
    }

    #[test]
    fn browse_responses_preserve_query_order_partial_errors_truncation_and_same_name_siblings() {
        let request = request();
        let start = Instant::now();
        let finish = start + Duration::from_millis(5);
        let response = BrowseResponse::builder(&request, start, finish)
            .item(failure("third"))
            .item(BrowseResult::new(id("second"), Ok(Vec::new())))
            .item(
                BrowseResult::new(
                    id("first"),
                    Ok(vec![
                        node("root"),
                        node("same").with_parent(0),
                        node("same").with_parent(0),
                        node("child").with_parent(1),
                    ]),
                )
                .with_truncated(true),
            )
            .build()
            .unwrap();
        assert_eq!(
            response
                .items()
                .iter()
                .map(|r| r.query().as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "third"]
        );
        assert_eq!(response.iter_ok().count(), 2);
        assert_eq!(response.iter_failed().count(), 1);
        assert_eq!(response.started_at(), start);
        assert_eq!(response.finished_at(), finish);
        assert!(response.get(&id("missing")).is_none());
        let first = response.get(&id("first")).unwrap();
        assert!(first.truncated());
        assert_eq!(
            first.outcome().as_ref().unwrap().get(1).unwrap().name(),
            first.outcome().as_ref().unwrap().get(2).unwrap().name()
        );
        let error = response
            .get(&id("third"))
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert_eq!(error.status().unwrap().code(), 0x8034_0000);
        assert!(!response.get(&id("third")).unwrap().truncated());
        assert_eq!(response.into_items().len(), 3);
        let response = BrowseResponse::builder(&request, start, start)
            .item(failure("first"))
            .item(failure("second"))
            .item(failure("third"))
            .build()
            .unwrap();
        assert_eq!(response.iter_failed().count(), 3);
    }

    #[test]
    fn browse_response_rejects_bad_identity_intervals_parents_and_silent_overflow() {
        let request = request();
        let now = Instant::now();
        assert_eq!(
            BrowseResponse::builder(
                &request,
                now,
                now.checked_sub(Duration::from_nanos(1)).unwrap()
            )
            .item(failure("first"))
            .build()
            .unwrap_err()
            .kind(),
            ErrorKind::InvalidConfig
        );
        assert_eq!(
            BrowseResponse::builder(&request, now, now)
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedData
        );
        assert_eq!(
            BrowseResponse::builder(&request, now, now)
                .item(failure("unknown"))
                .item(failure("first"))
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedData
        );
        assert_eq!(
            BrowseResponse::builder(&request, now, now)
                .item(failure("first"))
                .item(failure("first"))
                .item(failure("second"))
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedData
        );
        assert_eq!(
            BrowseResponse::builder(&request, now, now)
                .item(failure("first").with_truncated(true))
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::MalformedData
        );
        for nodes in [
            vec![node("root").with_parent(0)],
            vec![node("root"), node("self").with_parent(1)],
            vec![node("forward").with_parent(1), node("back").with_parent(0)],
            vec![node("dangling").with_parent(usize::MAX)],
        ] {
            assert_eq!(
                BrowseResponse::builder(&request, now, now)
                    .item(BrowseResult::new(id("first"), Ok(nodes)))
                    .build()
                    .unwrap_err()
                    .kind(),
                ErrorKind::MalformedData
            );
        }
        let error = BrowseResponse::builder(&request, now, now)
            .item(BrowseResult::new(
                id("first"),
                Ok(vec![node("a"), node("b"), node("c")]),
            ))
            .item(BrowseResult::new(
                id("second"),
                Ok(vec![node("d"), node("e")]),
            ))
            .item(failure("unknown"))
            .build()
            .unwrap_err();
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "first error survives later invalid identities"
        );
    }

    #[test]
    fn browse_public_types_are_send_sync_and_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<BrowseQuery>();
        assert_bounds::<BrowseRequest>();
        assert_bounds::<stonecrab::BrowseRequestBuilder>();
        assert_bounds::<BrowseNode>();
        assert_bounds::<BrowseResult>();
        assert_bounds::<BrowseResponse>();
        assert_bounds::<stonecrab::BrowseResponseBuilder>();
        assert_eq!(
            ResourceLimits::builder()
                .build()
                .unwrap()
                .max_browse_nodes()
                .get(),
            10000
        );
        assert_eq!(
            ResourceLimits::builder()
                .max_browse_nodes(0)
                .build()
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
        assert_eq!(
            ResourceLimits::builder()
                .max_browse_nodes(usize::MAX)
                .build()
                .unwrap()
                .max_browse_nodes()
                .get(),
            usize::MAX
        );
    }

    struct DefaultBrowse {
        state: stonecrab::StateCell,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl stonecrab::Connection for DefaultBrowse {
        fn state(&self) -> stonecrab::ConnectionState {
            self.state.current()
        }
        fn watch_state(&self) -> stonecrab::StateWatch {
            self.state.watch()
        }
        fn capabilities(&self) -> stonecrab::Capabilities {
            stonecrab::Capabilities::default()
        }
        fn limits(&self) -> ResourceLimits {
            limits(1, 1)
        }
        fn read(
            &self,
            _: stonecrab::ReadRequest,
        ) -> stonecrab::OpFuture<'_, stonecrab::ReadResponse> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Err(PlcError::new(
                    ErrorKind::Unsupported(stonecrab::Operation::Read),
                    "no read",
                ))
            })
        }
        fn write(
            &self,
            _: stonecrab::WriteRequest,
        ) -> stonecrab::OpFuture<'_, stonecrab::WriteResponse> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(async {
                Err(PlcError::new(
                    ErrorKind::Unsupported(stonecrab::Operation::Write),
                    "no write",
                ))
            })
        }
        fn close(&self) -> stonecrab::OpFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn default_browse_is_object_safe_send_unsupported_without_io_even_if_cancelled_or_closed() {
        fn assert_send<T: Send>(_: &T) {}
        use std::task::{Context, Poll, Waker};
        use stonecrab::{Connection, ConnectionState, FailureCategory, Operation, StateCell};
        for state in [ConnectionState::Connected, ConnectionState::Closed] {
            let concrete = DefaultBrowse {
                state: StateCell::new(state).0,
                calls: std::sync::atomic::AtomicUsize::new(0),
            };
            let connection: &dyn Connection = &concrete;
            let cancel = CancellationToken::new();
            cancel.cancel();
            let request = BrowseRequest::builder(connection.limits())
                .query(id("root"), "unparsed!\0secret")
                .cancel(cancel)
                .deadline(
                    Deadline::after(
                        Instant::now().checked_sub(Duration::from_secs(1)).unwrap(),
                        Duration::from_nanos(1),
                    )
                    .unwrap(),
                )
                .build()
                .unwrap();
            let mut future = connection.browse(request.clone());
            assert_send(&future);
            let Poll::Ready(Err(error)) = future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
            else {
                panic!("default browse resolves immediately")
            };
            assert_eq!(error.kind(), ErrorKind::Unsupported(Operation::Browse));
            assert_eq!(error.diagnostics().operation, Some(Operation::Browse));
            assert_eq!(error.diagnostics().connection_state, Some(state));
            assert_eq!(error.diagnostics().latency, Some(Duration::ZERO));
            assert_eq!(
                error.diagnostics().category,
                Some(FailureCategory::Validation)
            );
            assert!(!error.to_string().contains("secret"));
            drop(connection.browse(request));
            assert_eq!(connection.state(), state);
            assert_eq!(concrete.calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        }
    }
}
