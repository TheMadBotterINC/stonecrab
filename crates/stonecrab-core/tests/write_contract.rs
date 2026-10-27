//! Consumer-side write batch and response contracts using the shared testkit.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab_core::{
        CancellationToken, Deadline, ErrorKind, ItemError, PlcError, PlcValue, PlcValueType,
        ProtocolStatus, ResourceLimits, Tag, TagId, UnknownReason, WriteItem, WriteOutcome,
        WriteRequest, WriteResponse, WriteResult,
    };
    use stonecrab_testkit::FixtureAddress;

    fn id(name: &str) -> TagId {
        TagId::new(name).expect("fixture identity")
    }

    fn tag(name: &str, hint: Option<PlcValueType>) -> Tag {
        Tag::new(id(name), Arc::new(FixtureAddress::new(name, hint)))
    }

    fn limits(items: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_items_per_request(items)
            .build()
            .expect("valid budgets")
    }

    fn request(names: &[&str]) -> WriteRequest {
        names
            .iter()
            .fold(
                WriteRequest::builder(limits(names.len())),
                |builder, name| builder.item(tag(name, None), PlcValue::I16(1)),
            )
            .build()
            .expect("nonempty unique fixture request")
    }

    fn confirmed(name: &str) -> WriteResult {
        WriteResult::new(id(name), WriteOutcome::Confirmed)
    }

    fn rejected(name: &str) -> WriteResult {
        WriteResult::new(
            id(name),
            WriteOutcome::Rejected(
                ItemError::new(PlcError::new(
                    ErrorKind::DeviceException,
                    "device rejected the write",
                ))
                .with_status(ProtocolStatus::new("modbus", 2)),
            ),
        )
    }

    fn unknown(name: &str, reason: UnknownReason) -> WriteResult {
        WriteResult::new(id(name), WriteOutcome::Unknown(reason))
    }

    #[test]
    fn bounded_requests_preserve_order_values_and_explicit_deadlines() {
        let now = Instant::now();
        let request = WriteRequest::builder(limits(2))
            .item(tag("first", None), PlcValue::Bool(true))
            .item(tag("second", Some(PlcValueType::U64)), PlcValue::U64(9))
            .deadline(Deadline::at(now))
            .build()
            .expect("valid batch");
        let pairs: Vec<_> = request
            .items()
            .iter()
            .map(|item| (item.tag().id().as_str(), item.value().clone()))
            .collect();
        assert_eq!(
            pairs,
            [
                ("first", PlcValue::Bool(true)),
                ("second", PlcValue::U64(9))
            ],
            "caller order and values retained"
        );
        assert_eq!(
            request.deadline(),
            Some(Deadline::at(now)),
            "expired cutoff is not silently extended"
        );
        let implicit = WriteRequest::builder(limits(1))
            .item(tag("first", None), PlcValue::Null)
            .build()
            .expect("valid without explicit cutoff");
        assert_eq!(implicit.deadline(), None, "connection supplies the default");
        let item = WriteItem::new(tag("loose", None), PlcValue::I16(3));
        assert_eq!(item.tag().id().as_str(), "loose", "item retains its tag");
    }

    #[test]
    fn a_value_type_that_contradicts_the_tag_hint_is_invalid_config() {
        let error = WriteRequest::builder(limits(2))
            .item(tag("typed", Some(PlcValueType::U16)), PlcValue::I16(1))
            .build()
            .expect_err("hint mismatch rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "typed conflict");
        let explicit = tag("explicit", None)
            .with_type(PlcValueType::I16)
            .expect("no conflicting hint");
        let error = WriteRequest::builder(limits(2))
            .item(explicit, PlcValue::U16(1))
            .build()
            .expect_err("explicit type also binds the value");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "explicit conflict");
    }

    // covers: SPEC mandatory-failure-cases / resource limits and local validation
    #[test]
    fn empty_duplicate_and_oversized_requests_fail_before_io() {
        let error = WriteRequest::builder(limits(2))
            .build()
            .expect_err("empty rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "empty batch");
        let error = WriteRequest::builder(limits(3))
            .item(tag("same", None), PlcValue::I16(1))
            .item(tag("same", None), PlcValue::I16(2))
            .item(tag("later", None), PlcValue::I16(3))
            .build()
            .expect_err("duplicate remains an error");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "sticky duplicate");
        let error = WriteRequest::builder(limits(1))
            .item(tag("one", None), PlcValue::I16(1))
            .item(tag("two", None), PlcValue::I16(2))
            .item(tag("three", None), PlcValue::I16(3))
            .build()
            .expect_err("over budget remains an error");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "sticky limit");
    }

    // covers: SPEC errors / ambiguous write never replayed
    #[test]
    fn mixed_outcomes_restore_request_order_and_keep_unknown_unchanged() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["first", "bad", "lost"]);
        let lost = unknown("lost", UnknownReason::DisconnectedAfterTransmit)
            .with_status(ProtocolStatus::new("modbus", 0xff));
        let response = WriteResponse::builder(&request, start, finish)
            .result(lost)
            .result(rejected("bad"))
            .result(confirmed("first"))
            .build()
            .expect("rejection and unknown are valid data");
        let ids: Vec<_> = response
            .results()
            .iter()
            .map(|result| result.tag().as_str())
            .collect();
        assert_eq!(ids, ["first", "bad", "lost"], "request order restored");
        assert_eq!(response.started_at(), start, "start retained");
        assert_eq!(response.finished_at(), finish, "finish retained");
        assert_eq!(response.confirmed_count(), 1, "confirmed counted");
        assert_eq!(response.rejected_count(), 1, "rejected counted");
        assert_eq!(response.unknown_count(), 1, "unknown counted");
        let bad = response.get(&id("bad")).expect("identity retained");
        let WriteOutcome::Rejected(error) = bad.outcome() else {
            panic!("rejection must stay a rejection");
        };
        assert_eq!(error.status().map(ProtocolStatus::code), Some(2), "status");
        let lost = response.get(&id("lost")).expect("unknown is never dropped");
        assert!(
            matches!(
                lost.outcome(),
                WriteOutcome::Unknown(UnknownReason::DisconnectedAfterTransmit)
            ),
            "unknown keeps its reason and is not collapsed into a rejection"
        );
        assert_eq!(
            lost.status().map(ProtocolStatus::code),
            Some(0xff),
            "optional status survives beside the unknown outcome"
        );
        assert!(
            response
                .get(&id("first"))
                .is_some_and(|r| r.status().is_none()),
            "no invented status"
        );
        assert_eq!(response.iter_confirmed().count(), 1, "confirmed iterator");
        assert_eq!(response.iter_rejected().count(), 1, "rejected iterator");
        assert_eq!(response.iter_unknown().count(), 1, "unknown iterator");
        assert_eq!(response.into_results().len(), 3, "consuming keeps all");
    }

    // covers: SPEC errors / ambiguous write never replayed
    #[test]
    fn every_unknown_reason_survives_correlation_unchanged() {
        let now = Instant::now();
        let request = request(&["a", "b", "c"]);
        let response = WriteResponse::builder(&request, now, now)
            .result(unknown("a", UnknownReason::TimeoutAfterTransmit))
            .result(unknown("b", UnknownReason::DisconnectedAfterTransmit))
            .result(unknown("c", UnknownReason::CancelledAfterTransmit))
            .build()
            .expect("an all-unknown batch is valid data");
        let reasons: Vec<_> = response
            .iter_unknown()
            .map(|result| {
                if let WriteOutcome::Unknown(reason) = result.outcome() {
                    Some(*reason)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            reasons,
            [
                Some(UnknownReason::TimeoutAfterTransmit),
                Some(UnknownReason::DisconnectedAfterTransmit),
                Some(UnknownReason::CancelledAfterTransmit),
            ],
            "reasons retained in request order"
        );
        assert_eq!(response.iter_rejected().count(), 0, "none collapsed");
        assert_eq!(response.iter_confirmed().count(), 0, "none promoted");
    }

    #[test]
    fn an_all_rejected_batch_is_still_a_valid_correlated_response() {
        let now = Instant::now();
        let request = request(&["one", "two"]);
        let response = WriteResponse::builder(&request, now, now)
            .result(rejected("two"))
            .result(rejected("one"))
            .build()
            .expect("all per-item rejections retained");
        assert_eq!(response.confirmed_count(), 0, "no invented success");
        assert_eq!(response.rejected_count(), 2, "rejections retained");
        assert_eq!(response.unknown_count(), 0, "no invented ambiguity");
    }

    // covers: SPEC mandatory-failure-cases / incorrect correlation
    #[test]
    fn response_correlation_rejects_missing_unexpected_and_duplicate_results() {
        let now = Instant::now();
        let request = request(&["one", "two"]);
        let missing = WriteResponse::builder(&request, now, now)
            .result(confirmed("one"))
            .build()
            .expect_err("missing result");
        assert_eq!(missing.kind(), ErrorKind::MalformedData, "incomplete");
        let unexpected = WriteResponse::builder(&request, now, now)
            .result(confirmed("other"))
            .result(confirmed("one"))
            .build()
            .expect_err("unexpected identity sticky");
        assert_eq!(unexpected.kind(), ErrorKind::MalformedData, "unrequested");
        let duplicate = WriteResponse::builder(&request, now, now)
            .result(confirmed("one"))
            .result(confirmed("one"))
            .build()
            .expect_err("duplicate result");
        assert_eq!(duplicate.kind(), ErrorKind::MalformedData, "duplicate");
    }

    #[test]
    fn response_time_validation_accepts_equal_edges_and_rejects_reversal() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["one"]);
        let reversed = WriteResponse::builder(&request, finish, start)
            .result(confirmed("one"))
            .build()
            .expect_err("reversed operation interval");
        assert_eq!(reversed.kind(), ErrorKind::InvalidConfig, "bad interval");
        for (from, to) in [(start, start), (start, finish)] {
            let response = WriteResponse::builder(&request, from, to)
                .result(confirmed("one"))
                .build()
                .expect("boundary interval accepted");
            assert_eq!(response.started_at(), from, "start retained");
            assert_eq!(response.finished_at(), to, "finish retained");
        }
    }

    #[test]
    fn every_three_item_permutation_preserves_identity_and_its_distinct_outcome() {
        let now = Instant::now();
        let request = request(&["a", "b", "c"]);
        let result = |name: &str| match name {
            "a" => confirmed("a"),
            "b" => rejected("b"),
            _ => unknown("c", UnknownReason::CancelledAfterTransmit),
        };
        for order in [
            ["a", "b", "c"],
            ["a", "c", "b"],
            ["b", "a", "c"],
            ["b", "c", "a"],
            ["c", "a", "b"],
            ["c", "b", "a"],
        ] {
            let response = order
                .into_iter()
                .fold(
                    WriteResponse::builder(&request, now, now),
                    |builder, name| builder.result(result(name)),
                )
                .build()
                .expect("complete permutation");
            let confirmed: Vec<_> = response
                .iter_confirmed()
                .map(|r| r.tag().as_str())
                .collect();
            let rejected: Vec<_> = response.iter_rejected().map(|r| r.tag().as_str()).collect();
            let unknown: Vec<_> = response.iter_unknown().map(|r| r.tag().as_str()).collect();
            assert_eq!(confirmed, ["a"], "sorting must not move a confirmation");
            assert_eq!(rejected, ["b"], "sorting must not move a rejection");
            assert_eq!(unknown, ["c"], "sorting must not move an unknown outcome");
            let ids: Vec<_> = response
                .results()
                .iter()
                .map(|r| r.tag().as_str())
                .collect();
            assert_eq!(ids, ["a", "b", "c"], "request order restored");
        }
    }

    // covers: SPEC mandatory-failure-cases / F4 cancellation (request slot)
    #[test]
    fn the_cancel_slot_is_absent_by_default_and_shares_the_callers_token() {
        let plain = request(&["a"]);
        assert!(
            plain.cancel().is_none(),
            "no token unless the caller sets one"
        );

        let token = CancellationToken::new();
        let first = CancellationToken::new();
        let with_token = WriteRequest::builder(limits(2))
            .item(tag("a", None), PlcValue::Bool(true))
            .cancel(first.clone())
            .cancel(token.clone())
            .item(tag("b", None), PlcValue::Bool(false))
            .build()
            .expect("valid batch");
        let carried = with_token.cancel().expect("token set");
        assert!(!carried.is_cancelled(), "live token stays live");
        token.cancel();
        assert!(carried.is_cancelled(), "the slot shares the caller's flag");
        assert!(
            !first.is_cancelled(),
            "a second call replaced the first token"
        );
        assert!(
            with_token
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
        let cancelled = WriteRequest::builder(limits(1))
            .item(tag("a", None), PlcValue::Bool(true))
            .cancel(token)
            .build()
            .expect("cancelled tokens are not a request error");
        assert!(
            cancelled
                .cancel()
                .is_some_and(CancellationToken::is_cancelled),
            "kept as given"
        );
    }
}
