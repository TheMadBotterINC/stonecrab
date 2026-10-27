use std::time::Instant;

use crate::{ErrorKind, PlcError, ProtocolStatus, Result, TagId, WriteOutcome, WriteRequest};

/// The disposition of one requested write, correlated to its tag.
///
/// The outcome is carried unchanged: a confirmed write, a rejection with its item
/// error, or an unknown outcome with its [`crate::UnknownReason`]. An unknown outcome is
/// never collapsed into a rejection and never dropped. `status` is the device's
/// own status for this write, kept apart from the outcome so a confirmation can
/// still carry the code the device returned.
///
/// # Wire effects
/// None; this is data describing what already happened.
///
/// # Examples
/// ```
/// use stonecrab::{ProtocolStatus, TagId, UnknownReason, WriteOutcome, WriteResult};
/// let result = WriteResult::new(
///     TagId::new("setpoint")?,
///     WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit),
/// )
/// .with_status(ProtocolStatus::new("modbus", 0));
/// assert!(matches!(
///     result.outcome(),
///     WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit)
/// ));
/// assert_eq!(result.status().map(ProtocolStatus::code), Some(0));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-WRITE-RESPONSE
/// Corresponds to one tag's response code in an upstream `PlcWriteResponse`;
/// the unknown outcome has no upstream counterpart.
#[derive(Debug)]
#[non_exhaustive]
pub struct WriteResult {
    tag: TagId,
    outcome: WriteOutcome,
    status: Option<ProtocolStatus>,
}

impl WriteResult {
    /// Record one write disposition with no device status.
    #[must_use]
    pub const fn new(tag: TagId, outcome: WriteOutcome) -> Self {
        Self {
            tag,
            outcome,
            status: None,
        }
    }

    /// Attach the device's original status for this write.
    ///
    /// The status is independent of the outcome and of any status already held by
    /// a rejection's item error.
    #[must_use]
    pub fn with_status(mut self, status: ProtocolStatus) -> Self {
        self.status = Some(status);
        self
    }

    /// Original caller-supplied identity.
    #[must_use]
    pub const fn tag(&self) -> &TagId {
        &self.tag
    }

    /// Confirmed, rejected or unknown, exactly as the driver reported it.
    #[must_use]
    pub const fn outcome(&self) -> &WriteOutcome {
        &self.outcome
    }

    /// Original device status, absent when the driver supplied none.
    #[must_use]
    pub const fn status(&self) -> Option<&ProtocolStatus> {
        self.status.as_ref()
    }
}

/// Complete correlated write batch, including each rejection and unknown outcome.
///
/// Results are immutable and restored to original request order. An unknown
/// outcome is first-class data: it is never collapsed into a rejection, never
/// dropped, and never triggers a replay (ADR-0003, ADR-0010). The application
/// decides what to do with it, for example read the value back and compare.
///
/// # Wire effects
/// None; this is data. Building or inspecting a response transmits nothing.
///
/// # Examples
/// ```
/// use stonecrab::WriteResponse;
/// fn tally(response: &WriteResponse) -> usize {
///     response.confirmed_count() + response.rejected_count() + response.unknown_count()
/// }
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-WRITE-RESPONSE
/// Corresponds to the upstream `PlcWriteResponse`; `WriteOutcome::Unknown` is a
/// Stonecrab addition for ambiguous timeouts (ARCHITECTURE section 3.3).
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct WriteResponse {
    results: Vec<WriteResult>,
    started_at: Instant,
    finished_at: Instant,
}

impl WriteResponse {
    /// Assemble results for exactly one validated request.
    ///
    /// The builder records a reversed operation interval as a sticky error; all
    /// validation occurs before any I/O can be performed by these types.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab::{WriteRequest, WriteResponse, WriteResponseBuilder};
    /// fn begin(request: &WriteRequest, start: Instant, finish: Instant) -> WriteResponseBuilder {
    ///     WriteResponse::builder(request, start, finish)
    /// }
    /// ```
    #[must_use = "assemble and validate the correlated results"]
    pub fn builder(
        request: &WriteRequest,
        started_at: Instant,
        finished_at: Instant,
    ) -> WriteResponseBuilder {
        WriteResponseBuilder {
            expected: request
                .items()
                .iter()
                .map(|item| item.tag().id().clone())
                .collect(),
            results: Vec::new(),
            started_at,
            finished_at,
            error: (finished_at < started_at).then(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    "write response finishes before it starts",
                )
            }),
        }
    }

    /// All results, including rejections and unknown outcomes, in request order.
    #[must_use]
    pub fn results(&self) -> &[WriteResult] {
        &self.results
    }

    /// Find a result by stable caller identity, not by device address.
    #[must_use]
    pub fn get(&self, id: &TagId) -> Option<&WriteResult> {
        self.results.iter().find(|result| result.tag() == id)
    }

    /// Writes the device explicitly confirmed.
    pub fn iter_confirmed(&self) -> impl Iterator<Item = &WriteResult> {
        self.results
            .iter()
            .filter(|result| matches!(result.outcome(), WriteOutcome::Confirmed))
    }

    /// Writes the device or local validation explicitly rejected.
    pub fn iter_rejected(&self) -> impl Iterator<Item = &WriteResult> {
        self.results
            .iter()
            .filter(|result| matches!(result.outcome(), WriteOutcome::Rejected(_)))
    }

    /// Writes that may have been applied; never replay these automatically.
    pub fn iter_unknown(&self) -> impl Iterator<Item = &WriteResult> {
        self.results
            .iter()
            .filter(|result| matches!(result.outcome(), WriteOutcome::Unknown(_)))
    }

    /// Number of confirmed writes.
    #[must_use]
    pub fn confirmed_count(&self) -> usize {
        self.iter_confirmed().count()
    }

    /// Number of rejected writes.
    #[must_use]
    pub fn rejected_count(&self) -> usize {
        self.iter_rejected().count()
    }

    /// Number of writes whose delivery is unknown.
    #[must_use]
    pub fn unknown_count(&self) -> usize {
        self.iter_unknown().count()
    }

    /// Monotonic operation start.
    #[must_use]
    pub const fn started_at(&self) -> Instant {
        self.started_at
    }

    /// Monotonic operation completion.
    #[must_use]
    pub const fn finished_at(&self) -> Instant {
        self.finished_at
    }

    /// Take every result, preserving original request order.
    #[must_use]
    pub fn into_results(self) -> Vec<WriteResult> {
        self.results
    }
}

/// Bounded correlation validator for a write response.
///
/// Result count cannot exceed the validated request; unexpected identities and
/// duplicates fail before insertion, and the first error is sticky. Rejected and
/// unknown outcomes are valid data, including a batch where every write was
/// rejected, and are retained exactly as supplied.
///
/// # Wire effects
/// None; this is data.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct WriteResponseBuilder {
    expected: Vec<TagId>,
    results: Vec<(usize, WriteResult)>,
    started_at: Instant,
    finished_at: Instant,
    error: Option<PlcError>,
}

impl WriteResponseBuilder {
    /// Retain one result after validating identity and uniqueness.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{WriteResponseBuilder, WriteResult};
    /// fn add(builder: WriteResponseBuilder, result: WriteResult) -> WriteResponseBuilder {
    ///     builder.result(result)
    /// }
    /// ```
    #[must_use = "builder changes must be retained"]
    pub fn result(mut self, result: WriteResult) -> Self {
        if self.error.is_some() {
            return self;
        }
        let Some(position) = self.expected.iter().position(|id| id == result.tag()) else {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "write response contains an unexpected tag identifier",
            ));
            return self;
        };
        if self
            .results
            .iter()
            .any(|(existing, _)| *existing == position)
        {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "write response repeats a tag identifier",
            ));
        } else {
            self.results.push((position, result));
        }
        self
    }

    /// Require exactly one result per requested identity and restore caller order.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for a reversed operation interval, or
    /// [`ErrorKind::MalformedData`] for missing, unexpected or duplicate results.
    /// Individual rejected or unknown outcomes do not fail this method.
    #[must_use = "response correlation validation must be checked"]
    pub fn build(mut self) -> Result<WriteResponse> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.results.len() != self.expected.len() {
            return Err(PlcError::new(
                ErrorKind::MalformedData,
                "write response is missing requested items",
            ));
        }
        self.results.sort_unstable_by_key(|(position, _)| *position);
        Ok(WriteResponse {
            results: self.results.into_iter().map(|(_, result)| result).collect(),
            started_at: self.started_at,
            finished_at: self.finished_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::{ItemError, PlcValue, PlcValueType, ProtocolAddress, Tag, UnknownReason};

    #[derive(Debug)]
    struct Probe;

    impl ProtocolAddress for Probe {
        fn protocol(&self) -> &'static str {
            "probe"
        }

        fn canonical(&self) -> String {
            "probe".to_owned()
        }

        fn value_type_hint(&self) -> Option<PlcValueType> {
            None
        }
    }

    fn id(name: &str) -> TagId {
        TagId::new(name).expect("valid identity")
    }

    fn request(names: &[&str]) -> WriteRequest {
        let limits = crate::ResourceLimits::builder()
            .max_items_per_request(names.len())
            .build()
            .expect("valid budgets");
        names
            .iter()
            .fold(WriteRequest::builder(limits), |builder, name| {
                builder.item(Tag::new(id(name), Arc::new(Probe)), PlcValue::I16(1))
            })
            .build()
            .expect("nonempty unique request")
    }

    fn confirmed(name: &str) -> WriteResult {
        WriteResult::new(id(name), WriteOutcome::Confirmed)
    }

    fn rejected(name: &str) -> WriteResult {
        WriteResult::new(
            id(name),
            WriteOutcome::Rejected(ItemError::new(PlcError::new(
                ErrorKind::DeviceException,
                "device rejected the write",
            ))),
        )
    }

    fn unknown(name: &str) -> WriteResult {
        WriteResult::new(
            id(name),
            WriteOutcome::Unknown(UnknownReason::TimeoutAfterTransmit),
        )
    }

    #[test]
    fn the_probe_address_reports_its_protocol_and_canonical_text() {
        let tag = Tag::new(id("probe"), Arc::new(Probe));
        assert_eq!(tag.address().protocol(), "probe", "protocol");
        assert_eq!(tag.address().canonical(), "probe", "canonical");
    }

    #[test]
    fn results_carry_identity_outcome_and_an_independent_status() {
        let bare = confirmed("one");
        assert_eq!(bare.tag(), &id("one"), "identity retained");
        assert!(matches!(bare.outcome(), WriteOutcome::Confirmed), "outcome");
        assert!(bare.status().is_none(), "no invented status");
        let with = unknown("two").with_status(ProtocolStatus::new("probe", 7));
        assert_eq!(with.status().map(ProtocolStatus::code), Some(7), "status");
        let reason = UnknownReason::TimeoutAfterTransmit;
        let kept = matches!(with.outcome(), WriteOutcome::Unknown(r) if *r == reason);
        assert!(kept, "status does not change the outcome");
    }

    #[test]
    fn mixed_outcomes_are_restored_to_request_order_and_counted() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["a", "b", "c"]);
        let response = WriteResponse::builder(&request, start, finish)
            .result(unknown("c"))
            .result(confirmed("a"))
            .result(rejected("b"))
            .build()
            .expect("mixed outcomes are valid data");
        let ids: Vec<_> = response
            .results()
            .iter()
            .map(|result| result.tag().as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"], "request order restored");
        assert_eq!(response.started_at(), start, "start retained");
        assert_eq!(response.finished_at(), finish, "finish retained");
        assert_eq!(response.confirmed_count(), 1, "confirmed counted");
        assert_eq!(response.rejected_count(), 1, "rejected counted");
        assert_eq!(response.unknown_count(), 1, "unknown counted");
        assert_eq!(response.iter_confirmed().count(), 1, "confirmed iterator");
        assert_eq!(response.iter_rejected().count(), 1, "rejected iterator");
        let kept: Vec<_> = response.iter_unknown().map(|r| r.tag().as_str()).collect();
        assert_eq!(kept, ["c"], "unknown iterator");
        assert!(response.get(&id("b")).is_some(), "lookup by identity");
        assert!(response.get(&id("absent")).is_none(), "nothing fabricated");
        assert_eq!(response.into_results().len(), 3, "consuming keeps all");
    }

    #[test]
    fn correlation_rejects_missing_unexpected_and_duplicate_results() {
        let now = Instant::now();
        let request = request(&["one", "two"]);
        let missing = WriteResponse::builder(&request, now, now)
            .result(confirmed("one"))
            .build()
            .expect_err("missing result");
        assert_eq!(missing.kind(), ErrorKind::MalformedData, "missing");
        let unexpected = WriteResponse::builder(&request, now, now)
            .result(confirmed("other"))
            .result(confirmed("one"))
            .result(confirmed("two"))
            .build()
            .expect_err("unexpected identity is sticky");
        assert_eq!(unexpected.kind(), ErrorKind::MalformedData, "unexpected");
        let duplicate = WriteResponse::builder(&request, now, now)
            .result(confirmed("one"))
            .result(unknown("one"))
            .result(confirmed("two"))
            .build()
            .expect_err("duplicate result");
        assert_eq!(duplicate.kind(), ErrorKind::MalformedData, "duplicate");
        assert!(
            duplicate.to_string().contains("repeats"),
            "first error is retained"
        );
    }

    #[test]
    fn a_reversed_interval_is_invalid_and_an_empty_one_is_valid() {
        let start = Instant::now();
        let finish = start
            .checked_add(Duration::from_secs(1))
            .expect("representable");
        let request = request(&["one"]);
        let reversed = WriteResponse::builder(&request, finish, start)
            .result(confirmed("one"))
            .build()
            .expect_err("reversed interval");
        assert_eq!(reversed.kind(), ErrorKind::InvalidConfig, "bad interval");
        let instant = WriteResponse::builder(&request, start, start)
            .result(confirmed("one"))
            .build()
            .expect("zero-length interval");
        assert_eq!(instant.started_at(), instant.finished_at(), "equal edges");
    }
}
