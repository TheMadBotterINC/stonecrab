//! Consumer-side address and bounded request contracts using the shared testkit.
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::num::NonZeroU32;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab::testkit::FixtureAddress;
    use stonecrab::{
        CancellationToken, Deadline, ErrorKind, ItemError, PlcError, PlcValue, PlcValueType,
        ProtocolAddress, ProtocolStatus, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Tag,
        TagId,
    };

    fn tag(name: &str, hint: Option<PlcValueType>) -> Tag {
        Tag::new(
            TagId::new(name).expect("fixture identity"),
            Arc::new(FixtureAddress::new(name, hint)),
        )
    }

    fn limits(items: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_items_per_request(items)
            .build()
            .expect("valid budgets")
    }

    #[test]
    fn tags_retain_identity_share_addresses_and_use_stable_trait_upcasting() {
        let address: Arc<dyn ProtocolAddress> =
            Arc::new(FixtureAddress::new("counter", Some(PlcValueType::U64)));
        let tagged = Tag::new(TagId::new("counter").expect("valid"), Arc::clone(&address));
        assert_eq!(tagged.id().as_str(), "counter", "identity retained");
        assert_eq!(tagged.address().protocol(), "fixture", "driver identity");
        assert_eq!(tagged.address().canonical(), "counter", "canonical text");
        assert_eq!(
            tagged.value_type(),
            Some(PlcValueType::U64),
            "address hint used"
        );
        assert_eq!(tagged.count(), NonZeroU32::MIN, "one element default");
        let concrete: &dyn Any = tagged.address();
        assert!(
            concrete.downcast_ref::<FixtureAddress>().is_some(),
            "stable trait upcasting"
        );
        assert_eq!(Arc::strong_count(&address), 2, "constructor shares address");
        let cloned = tagged.clone();
        assert_eq!(cloned.id(), tagged.id(), "clone preserves correlation");
        assert_eq!(Arc::strong_count(&address), 3, "clone shares address");
    }

    #[test]
    fn requested_types_and_counts_are_explicit_and_validated() {
        let untyped = tag("untyped", None);
        assert_eq!(untyped.value_type(), None, "type is not invented");
        let typed = untyped
            .with_type(PlcValueType::I16)
            .expect("no conflicting hint")
            .with_count(NonZeroU32::new(3).expect("positive"));
        assert_eq!(
            typed.value_type(),
            Some(PlcValueType::I16),
            "explicit requested type"
        );
        assert_eq!(typed.count().get(), 3, "explicit positive count");
        let matching = tag("matching", Some(PlcValueType::I16))
            .with_type(PlcValueType::I16)
            .expect("matching type");
        assert_eq!(
            matching.value_type(),
            Some(PlcValueType::I16),
            "matching hint accepted"
        );
        let error = tag("conflict", Some(PlcValueType::U64))
            .with_type(PlcValueType::I16)
            .expect_err("mismatch rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "typed conflict");
    }

    #[test]
    fn bounded_requests_preserve_order_and_explicit_deadlines() {
        let now = Instant::now();
        let request = ReadRequest::builder(limits(2))
            .tag(tag("first", None))
            .tag(tag("second", Some(PlcValueType::U64)))
            .deadline(Deadline::at(now))
            .build()
            .expect("valid batch");
        let ids: Vec<_> = request
            .items()
            .iter()
            .map(|tag| tag.id().as_str())
            .collect();
        assert_eq!(ids, ["first", "second"], "original order retained");
        assert_eq!(
            request.deadline(),
            Some(Deadline::at(now)),
            "expired cutoff is not silently extended"
        );
        assert_eq!(
            request.clone().items().len(),
            2,
            "request is reusable without reparsing"
        );
        let implicit = ReadRequest::builder(limits(1))
            .tag(tag("first", None))
            .build()
            .expect("valid without explicit cutoff");
        assert_eq!(
            implicit.deadline(),
            None,
            "connection must supply bounded default later"
        );
    }

    // covers: SPEC mandatory-failure-cases / resource limits and local validation
    #[test]
    fn empty_duplicate_and_oversized_requests_fail_before_io() {
        let error = ReadRequest::builder(limits(2))
            .build()
            .expect_err("empty rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "empty batch");
        let error = ReadRequest::builder(limits(3))
            .tag(tag("same", None))
            .tag(tag("same", None))
            .tag(tag("later", None))
            .build()
            .expect_err("duplicate remains an error");
        assert_eq!(
            error.kind(),
            ErrorKind::InvalidConfig,
            "sticky duplicate failure"
        );
        let error = ReadRequest::builder(limits(1))
            .tag(tag("one", None))
            .tag(tag("two", None))
            .tag(tag("three", None))
            .build()
            .expect_err("over budget remains an error");
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "sticky limit failure"
        );
    }

    fn request(names: &[&str]) -> ReadRequest {
        names
            .iter()
            .fold(
                ReadRequest::builder(limits(names.len())),
                |builder, name| builder.tag(tag(name, None)),
            )
            .build()
            .expect("nonempty unique fixture request")
    }

    fn reading(name: &str, sampled_at: Instant) -> ReadItem {
        ReadItem::new(
            TagId::new(name).expect("fixture identity"),
            Ok(PlcValue::I16(42)),
            sampled_at,
        )
    }

    // covers: SPEC mandatory-failure-cases / mixed batch success and timestamp absence
    #[test]
    fn split_read_responses_restore_request_order_and_keep_partial_failures() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["first", "failed", "last"]);
        let failed = ReadItem::new(
            TagId::new("failed").expect("valid"),
            Err(ItemError::new(PlcError::new(
                ErrorKind::DeviceException,
                "invalid register",
            ))
            .with_status(ProtocolStatus::new("modbus", 2))),
            finish,
        );
        let response = ReadResponse::builder(&request, start, finish)
            .item(reading("last", finish))
            .item(failed)
            .item(reading("first", start))
            .split_into(NonZeroU32::new(2).expect("positive"))
            .build()
            .expect("partial failure is valid data");
        let ids: Vec<_> = response
            .items()
            .iter()
            .map(|item| item.tag().as_str())
            .collect();
        assert_eq!(ids, ["first", "failed", "last"], "request order restored");
        assert_eq!(response.started_at(), start, "start retained");
        assert_eq!(response.finished_at(), finish, "finish retained");
        assert_eq!(
            response.split_into().get(),
            2,
            "split acquisition disclosed"
        );
        assert_eq!(response.iter_ok().count(), 2, "successful values retained");
        assert_eq!(response.iter_failed().count(), 1, "item failure retained");
        let failed = response
            .get(&TagId::new("failed").expect("valid"))
            .expect("original identity retained");
        assert_eq!(
            failed.status().expect("status retained").code(),
            2,
            "device exception retained"
        );
        assert!(
            failed.source_time().is_none(),
            "no invented source timestamp"
        );
        assert!(
            response
                .get(&TagId::new("absent").expect("valid"))
                .is_none(),
            "unknown identity not fabricated"
        );
        assert_eq!(
            response.into_items().len(),
            3,
            "consuming retains every item"
        );
    }

    #[test]
    fn an_all_failed_batch_is_still_a_valid_correlated_response() {
        let now = Instant::now();
        let request = request(&["failed"]);
        let item = ReadItem::new(
            TagId::new("failed").expect("valid"),
            Err(ItemError::new(PlcError::new(
                ErrorKind::DeviceException,
                "device rejected request",
            ))),
            now,
        );
        let response = ReadResponse::builder(&request, now, now)
            .item(item)
            .build()
            .expect("all per-item failures retained");
        assert_eq!(response.iter_ok().count(), 0, "no invented success");
        assert_eq!(response.iter_failed().count(), 1, "failure retained");
        assert_eq!(
            response.split_into(),
            NonZeroU32::MIN,
            "single batch default"
        );
    }

    // covers: SPEC mandatory-failure-cases / incorrect correlation
    #[test]
    fn response_correlation_rejects_missing_unexpected_and_duplicate_results() {
        let now = Instant::now();
        let request = request(&["one", "two"]);
        let missing = ReadResponse::builder(&request, now, now)
            .item(reading("one", now))
            .build()
            .expect_err("missing item");
        assert_eq!(
            missing.kind(),
            ErrorKind::MalformedData,
            "incomplete results rejected"
        );
        let unexpected = ReadResponse::builder(&request, now, now)
            .item(reading("other", now))
            .item(reading("one", now))
            .build()
            .expect_err("unexpected identity sticky");
        assert_eq!(
            unexpected.kind(),
            ErrorKind::MalformedData,
            "unrequested result rejected"
        );
        let duplicate = ReadResponse::builder(&request, now, now)
            .item(reading("one", now))
            .item(reading("one", now))
            .build()
            .expect_err("duplicate result");
        assert_eq!(
            duplicate.kind(),
            ErrorKind::MalformedData,
            "duplicate does not replace another result"
        );
    }

    #[test]
    fn response_time_validation_checks_both_interval_edges_and_reversed_intervals() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["one"]);
        let reversed = ReadResponse::builder(&request, finish, start)
            .item(reading("one", start))
            .build()
            .expect_err("reversed operation interval");
        assert_eq!(
            reversed.kind(),
            ErrorKind::InvalidConfig,
            "bad interval classified"
        );
        for sampled in [
            start
                .checked_sub(Duration::from_nanos(1))
                .expect("representable"),
            finish
                .checked_add(Duration::from_nanos(1))
                .expect("representable"),
        ] {
            let error = ReadResponse::builder(&request, start, finish)
                .item(reading("one", sampled))
                .build()
                .expect_err("out-of-interval sample");
            assert_eq!(
                error.kind(),
                ErrorKind::MalformedData,
                "sample must belong to operation"
            );
        }
    }

    #[test]
    fn every_three_item_permutation_preserves_identity_and_its_distinct_value() {
        let now = Instant::now();
        let request = request(&["a", "b", "c"]);
        for order in [
            [("a", 1), ("b", 2), ("c", 3)],
            [("a", 1), ("c", 3), ("b", 2)],
            [("b", 2), ("a", 1), ("c", 3)],
            [("b", 2), ("c", 3), ("a", 1)],
            [("c", 3), ("a", 1), ("b", 2)],
            [("c", 3), ("b", 2), ("a", 1)],
        ] {
            let response = order
                .into_iter()
                .fold(
                    ReadResponse::builder(&request, now, now),
                    |builder, (name, value)| {
                        builder.item(ReadItem::new(
                            TagId::new(name).expect("fixture"),
                            Ok(PlcValue::I16(value)),
                            now,
                        ))
                    },
                )
                .build()
                .expect("complete permutation");
            let pairs: Vec<_> = response
                .items()
                .iter()
                .map(|item| {
                    (
                        item.tag().as_str(),
                        item.outcome().as_ref().expect("success").as_i64(),
                    )
                })
                .collect();
            assert_eq!(
                pairs,
                [("a", Some(1)), ("b", Some(2)), ("c", Some(3))],
                "sorting must not separate values from their identities"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (request slot)
    #[test]
    fn the_cancel_slot_is_absent_by_default_and_shares_the_callers_token() {
        let plain = ReadRequest::builder(limits(2))
            .tag(tag("a", None))
            .build()
            .expect("valid batch");
        assert!(
            plain.cancel().is_none(),
            "no token unless the caller sets one"
        );

        let token = CancellationToken::new();
        let first = CancellationToken::new();
        let request = ReadRequest::builder(limits(2))
            .tag(tag("a", None))
            .cancel(first.clone())
            .cancel(token.clone())
            .tag(tag("b", None))
            .build()
            .expect("valid batch");
        let carried = request.cancel().expect("token set");
        assert!(!carried.is_cancelled(), "live token stays live");
        token.cancel();
        assert!(carried.is_cancelled(), "the slot shares the caller's flag");
        assert!(
            !first.is_cancelled(),
            "a second call replaced the first token"
        );
        assert!(
            request
                .clone()
                .cancel()
                .is_some_and(CancellationToken::is_cancelled),
            "clones of the request carry the same token"
        );
    }

    #[test]
    fn an_already_cancelled_token_is_retained_for_the_connection_to_reject() {
        let token = CancellationToken::new();
        token.cancel();
        let request = ReadRequest::builder(limits(1))
            .tag(tag("a", None))
            .cancel(token)
            .build()
            .expect("cancelled tokens are not a request error");
        assert!(
            request
                .cancel()
                .is_some_and(CancellationToken::is_cancelled),
            "kept as given"
        );
    }
}
