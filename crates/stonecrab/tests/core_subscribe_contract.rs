//! Consumer-side subscription request contracts (slice S54, ADR-0020) using the shared
//! testkit address; the queue behind a subscription is tested in its module.
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab::testkit::FixtureAddress;
    use stonecrab::{
        CancellationToken, Deadline, ErrorKind, QueuePolicy, ResourceLimits, SubscribeRequest,
        SubscribeRequestBuilder, SubscriptionKind, Tag, TagId,
    };

    const SECOND: SubscriptionKind = SubscriptionKind::Cyclic(Duration::from_secs(1));

    fn tag(name: &str) -> Tag {
        Tag::new(
            TagId::new(name).expect("fixture identity"),
            Arc::new(FixtureAddress::new(name, None)),
        )
    }

    fn limits(items: usize, events: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_items_per_request(items)
            .max_queued_events(events)
            .build()
            .expect("valid budgets")
    }

    fn with(builder: SubscribeRequestBuilder, names: &[&str]) -> SubscribeRequestBuilder {
        names
            .iter()
            .fold(builder, |builder, name| builder.tag(tag(name)))
    }

    #[test]
    fn requests_keep_caller_order_kind_policy_deadline_and_token() {
        let token = CancellationToken::new();
        let deadline = Deadline::at(Instant::now());
        let request = with(
            SubscribeRequest::builder(limits(4, 4), SECOND, QueuePolicy::DropOldest),
            &["second", "first"],
        )
        .deadline(deadline)
        .cancel(token.clone())
        .build()
        .expect("valid request");
        let names: Vec<&str> = request
            .items()
            .iter()
            .map(|tag| tag.id().as_str())
            .collect();
        assert_eq!(names, ["second", "first"], "caller order");
        assert_eq!(request.kind(), SECOND, "kind");
        assert_eq!(request.queue(), QueuePolicy::DropOldest, "policy");
        assert_eq!(request.deadline(), Some(deadline), "explicit deadline");
        token.cancel();
        assert!(
            request
                .cancel()
                .is_some_and(CancellationToken::is_cancelled),
            "the token is shared with the caller"
        );
        let plain = with(
            SubscribeRequest::builder(
                limits(4, 4),
                SubscriptionKind::Event,
                QueuePolicy::Backpressure,
            ),
            &["only"],
        )
        .build()
        .expect("valid request");
        assert_eq!(plain.deadline(), None, "no deadline by default");
        assert!(plain.cancel().is_none(), "no token by default");
    }

    #[test]
    fn empty_duplicate_and_over_budget_requests_are_rejected_with_the_first_error() {
        let empty = SubscribeRequest::builder(limits(2, 2), SECOND, QueuePolicy::DropOldest)
            .build()
            .expect_err("empty");
        assert_eq!(empty.kind(), ErrorKind::InvalidConfig, "empty request");
        let duplicate = with(
            SubscribeRequest::builder(limits(4, 4), SECOND, QueuePolicy::DropOldest),
            &["a", "a", "b", "c", "d", "e"],
        )
        .build()
        .expect_err("duplicate");
        assert_eq!(
            duplicate.kind(),
            ErrorKind::InvalidConfig,
            "the duplicate comes first and sticks"
        );
        let over = with(
            SubscribeRequest::builder(limits(2, 8), SECOND, QueuePolicy::DropOldest),
            &["a", "b", "c", "a"],
        )
        .build()
        .expect_err("over budget");
        assert_eq!(over.kind(), ErrorKind::ResourceLimit, "item budget");
    }

    #[test]
    fn a_zero_cyclic_interval_is_invalid_config() {
        let error = with(
            SubscribeRequest::builder(
                limits(2, 2),
                SubscriptionKind::Cyclic(Duration::ZERO),
                QueuePolicy::Backpressure,
            ),
            &["a"],
        )
        .build()
        .expect_err("zero interval");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "zero interval");
    }

    // covers: SPEC mandatory-failure-cases / F6 queue exhaustion (G10: max_queued_events)
    #[test]
    fn coalescing_needs_room_for_one_event_per_tag() {
        let builder = || {
            SubscribeRequest::builder(
                limits(8, 2),
                SubscriptionKind::ChangeOfState,
                QueuePolicy::CoalesceLatest,
            )
        };
        let fits = with(builder(), &["a", "b"]).build().expect("two tags fit");
        assert_eq!(fits.items().len(), 2, "every tag has a slot");
        let error = with(builder(), &["a", "b", "c"])
            .build()
            .expect_err("three tags do not fit");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "queue too small");
        let dropping = with(
            SubscribeRequest::builder(limits(8, 2), SECOND, QueuePolicy::DropOldest),
            &["a", "b", "c"],
        )
        .build()
        .expect("dropping counts its losses instead");
        assert_eq!(dropping.items().len(), 3, "accepted");
    }
}
