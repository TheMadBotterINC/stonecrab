use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::{Deadline, ErrorKind, PlcError, PlcValue, ResourceLimits, Result, Tag, TagId};

/// One value to write to one tag.
///
/// The pair is plain data: building it validates nothing against a device, and
/// the batch builder ([`WriteRequestBuilder::item`]) checks the value's type
/// against the tag's type hint. Cloning copies the value and shares the tag's
/// address.
///
/// # Wire effects
/// None; this is data. Nothing is transmitted until a connection executes a
/// [`WriteRequest`].
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{PlcValue, ProtocolAddress, Tag, TagId, WriteItem};
/// fn pair(address: Arc<dyn ProtocolAddress>) -> stonecrab_core::Result<WriteItem> {
///     let tag = Tag::new(TagId::new("setpoint")?, address);
///     let item = WriteItem::new(tag, PlcValue::I16(25));
///     assert_eq!(item.tag().id().as_str(), "setpoint");
///     assert_eq!(item.value(), &PlcValue::I16(25));
///     Ok(item)
/// }
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-WRITE-REQUEST
/// Corresponds to one tag and value of an upstream `PlcWriteRequest`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WriteItem {
    tag: Tag,
    value: PlcValue,
}

impl WriteItem {
    /// Pair a tag with the value to write to it, without validating either.
    #[must_use]
    pub const fn new(tag: Tag, value: PlcValue) -> Self {
        Self { tag, value }
    }

    /// The tag to write: caller identity, address and requested type.
    #[must_use]
    pub const fn tag(&self) -> &Tag {
        &self.tag
    }

    /// The value to write, exactly as the caller supplied it.
    #[must_use]
    pub const fn value(&self) -> &PlcValue {
        &self.value
    }

    /// Take the tag and value apart without cloning either.
    #[must_use]
    pub fn into_parts(self) -> (Tag, PlcValue) {
        (self.tag, self.value)
    }
}

/// Immutable bounded write batch, validated without performing I/O.
///
/// A write is never retried and never replayed after an unknown outcome
/// (ADR-0003, ADR-0010): if a transmitted write is not confirmed, the response
/// carries [`crate::WriteOutcome::Unknown`] and the application decides what to
/// do. This type holds no retry state and offers no way to resubmit itself;
/// reusing the same request is a new, explicit caller decision.
///
/// No deadline means the future connection must supply its configured deadline
/// before transmission, not that unbounded I/O is permitted. The cancel slot
/// arrives with the connection contract, together with the read one.
///
/// # Wire effects
/// None; this is data. The request is only a description of what a connection
/// would transmit.
///
/// # Compatibility: docs/parity/core.md#CORE-WRITE-REQUEST
/// Corresponds to the upstream `PlcWriteRequest`: an ordered batch of tags and
/// values.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct WriteRequest {
    items: Vec<WriteItem>,
    deadline: Option<Deadline>,
}

impl WriteRequest {
    /// Begin a request with an explicit finite item budget.
    ///
    /// Defaults and error behavior are documented on [`WriteRequestBuilder`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{ErrorKind, ResourceLimits, WriteRequest};
    /// let limits = ResourceLimits::builder().build()?;
    /// let error = WriteRequest::builder(limits).build().expect_err("empty batch");
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use = "build and validate the request before use"]
    pub const fn builder(limits: ResourceLimits) -> WriteRequestBuilder {
        WriteRequestBuilder {
            items: Vec::new(),
            identifiers: BTreeSet::new(),
            max_items: limits.max_items_per_request(),
            deadline: None,
            error: None,
        }
    }

    /// Items to write in original caller order.
    #[must_use]
    pub fn items(&self) -> &[WriteItem] {
        &self.items
    }

    /// Explicit cutoff, or absence requiring the connection's bounded default.
    #[must_use]
    pub const fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }
}

/// A bounded write batch under construction.
///
/// Starts empty with no explicit deadline. `item` checks the item budget, the
/// value type against the tag's type hint and duplicate identity before growing
/// internal collections. The first validation error is sticky: subsequent items
/// are discarded without allocation, and `build` returns that error. Empty
/// requests are rejected. No I/O is performed.
///
/// # Wire effects
/// None; this is data.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct WriteRequestBuilder {
    items: Vec<WriteItem>,
    identifiers: BTreeSet<TagId>,
    max_items: NonZeroUsize,
    deadline: Option<Deadline>,
    error: Option<PlcError>,
}

impl WriteRequestBuilder {
    /// Add one tag and value in caller order.
    ///
    /// Rejects overflow of the item budget, a value whose type differs from a
    /// type the tag already carries, and a repeated tag identifier. A tag with no
    /// type hint accepts any value type; the driver validates the value against
    /// the device before transmission.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{PlcValue, Tag, WriteRequestBuilder};
    /// fn add(builder: WriteRequestBuilder, tag: Tag) -> WriteRequestBuilder {
    ///     builder.item(tag, PlcValue::Bool(true))
    /// }
    /// ```
    #[must_use = "builder changes must be retained"]
    pub fn item(mut self, tag: Tag, value: PlcValue) -> Self {
        if self.error.is_some() {
            return self;
        }
        if self.items.len() >= self.max_items.get() {
            self.error = Some(PlcError::new(
                ErrorKind::ResourceLimit,
                "write request exceeds item budget",
            ));
        } else if tag
            .value_type()
            .is_some_and(|expected| expected != value.value_type())
        {
            self.error = Some(PlcError::new(
                ErrorKind::InvalidConfig,
                "write value type conflicts with the tag's value type",
            ));
        } else if !self.identifiers.insert(tag.id().clone()) {
            self.error = Some(PlcError::new(
                ErrorKind::InvalidConfig,
                "duplicate tag identifier in write request",
            ));
        } else {
            self.items.push(WriteItem::new(tag, value));
        }
        self
    }

    /// Supply an absolute cutoff; an already elapsed deadline is retained so the
    /// connection can reject it against its own clock without transmitting.
    #[must_use = "builder changes must be retained"]
    pub const fn deadline(mut self, deadline: Deadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Finish validation and retain original order without performing I/O.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for empty requests, duplicate
    /// identifiers or a value type that conflicts with its tag, or
    /// [`ErrorKind::ResourceLimit`] for an exceeded item budget.
    #[must_use = "write request validation must be checked"]
    pub fn build(self) -> Result<WriteRequest> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.items.is_empty() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "write request must contain at least one item",
            ));
        }
        Ok(WriteRequest {
            items: self.items,
            deadline: self.deadline,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Instant;

    use super::*;
    use crate::{PlcValueType, ProtocolAddress};

    #[derive(Debug)]
    struct Probe(Option<PlcValueType>);

    impl ProtocolAddress for Probe {
        fn protocol(&self) -> &'static str {
            "probe"
        }

        fn canonical(&self) -> String {
            "probe".to_owned()
        }

        fn value_type_hint(&self) -> Option<PlcValueType> {
            self.0
        }
    }

    fn tag(name: &str, hint: Option<PlcValueType>) -> Tag {
        Tag::new(
            TagId::new(name).expect("valid identity"),
            Arc::new(Probe(hint)),
        )
    }

    fn limits(items: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_items_per_request(items)
            .build()
            .expect("valid budgets")
    }

    #[test]
    fn address_probe_reports_its_protocol_and_canonical_text() {
        let probe = tag("probe", None);
        assert_eq!(probe.address().protocol(), "probe", "protocol");
        assert_eq!(probe.address().canonical(), "probe", "canonical");
    }

    #[test]
    fn items_pair_a_tag_with_its_value_and_come_apart_without_loss() {
        let item = WriteItem::new(tag("setpoint", None), PlcValue::I16(25));
        assert_eq!(item.tag().id().as_str(), "setpoint", "tag retained");
        assert_eq!(item.value(), &PlcValue::I16(25), "value retained");
        let copy = item.clone();
        let (tag, value) = item.into_parts();
        assert_eq!(tag.id().as_str(), "setpoint", "tag handed back");
        assert_eq!(value, PlcValue::I16(25), "value handed back");
        assert_eq!(copy.value(), &PlcValue::I16(25), "clone is independent");
    }

    #[test]
    fn requests_keep_caller_order_deadline_and_untyped_tags_accept_any_value() {
        let now = Instant::now();
        let request = WriteRequest::builder(limits(3))
            .item(tag("first", None), PlcValue::Bool(true))
            .item(tag("second", Some(PlcValueType::U64)), PlcValue::U64(7))
            .item(tag("third", None), PlcValue::I16(-1))
            .deadline(Deadline::at(now))
            .build()
            .expect("valid batch");
        let ids: Vec<_> = request
            .items()
            .iter()
            .map(|item| item.tag().id().as_str())
            .collect();
        assert_eq!(ids, ["first", "second", "third"], "caller order retained");
        assert_eq!(request.deadline(), Some(Deadline::at(now)), "deadline kept");
        assert_eq!(request.clone().items().len(), 3, "request is reusable");
        let implicit = WriteRequest::builder(limits(1))
            .item(tag("only", None), PlcValue::Null)
            .build()
            .expect("valid without explicit cutoff");
        assert_eq!(implicit.deadline(), None, "no invented deadline");
    }

    #[test]
    fn empty_batches_are_rejected() {
        let error = WriteRequest::builder(limits(2))
            .build()
            .expect_err("empty rejected");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "empty batch");
    }

    #[test]
    fn a_value_type_that_contradicts_the_tag_hint_is_invalid_config() {
        let error = WriteRequest::builder(limits(3))
            .item(tag("typed", Some(PlcValueType::U16)), PlcValue::I16(1))
            .item(tag("later", None), PlcValue::I16(2))
            .build()
            .expect_err("mismatch is sticky");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "type conflict");
        let error = WriteRequest::builder(limits(1))
            .item(tag("typed", Some(PlcValueType::U16)), PlcValue::Null)
            .build()
            .expect_err("Null is a distinct type");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "null conflict");
        WriteRequest::builder(limits(1))
            .item(tag("typed", Some(PlcValueType::U16)), PlcValue::U16(1))
            .build()
            .expect("matching type accepted");
    }

    #[test]
    fn duplicate_identities_are_rejected_and_the_first_error_wins() {
        let error = WriteRequest::builder(limits(4))
            .item(tag("same", None), PlcValue::I16(1))
            .item(tag("same", None), PlcValue::I16(2))
            .item(tag("later", Some(PlcValueType::U16)), PlcValue::I16(3))
            .build()
            .expect_err("duplicate remains the error");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "sticky duplicate");
        assert!(
            error.to_string().contains("duplicate"),
            "first error is retained"
        );
    }

    #[test]
    fn over_budget_batches_fail_before_the_collections_grow() {
        let builder = WriteRequest::builder(limits(1))
            .item(tag("one", None), PlcValue::I16(1))
            .item(tag("two", None), PlcValue::I16(2))
            .item(tag("three", None), PlcValue::I16(3));
        assert_eq!(builder.items.len(), 1, "no item added past the budget");
        assert_eq!(builder.identifiers.len(), 1, "no identity added past it");
        let error = builder.build().expect_err("over budget");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit, "limit failure");
    }
}
