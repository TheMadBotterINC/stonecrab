use std::num::NonZeroUsize;
use std::time::Instant;

use crate::{BrowseNode, BrowseRequest, ErrorKind, ItemError, PlcError, Result, TagId};

/// A single named query's flat node list or typed refusal, with explicit truncation.
#[derive(Debug)]
#[non_exhaustive]
pub struct BrowseResult {
    query: TagId,
    outcome: std::result::Result<Vec<BrowseNode>, ItemError>,
    truncated: bool,
}

impl BrowseResult {
    /// Pair a caller identity with nodes or an item error; response assembly validates it.
    #[must_use]
    pub const fn new(
        query: TagId,
        outcome: std::result::Result<Vec<BrowseNode>, ItemError>,
    ) -> Self {
        Self {
            query,
            outcome,
            truncated: false,
        }
    }

    /// Disclose a successful walk stopped by a budget. An error cannot also be truncated.
    #[must_use]
    pub const fn with_truncated(mut self, truncated: bool) -> Self {
        self.truncated = truncated;
        self
    }

    /// Stable caller query identity.
    #[must_use]
    pub const fn query(&self) -> &TagId {
        &self.query
    }
    /// Flat nodes or the original typed item failure/status.
    #[must_use]
    pub const fn outcome(&self) -> &std::result::Result<Vec<BrowseNode>, ItemError> {
        &self.outcome
    }
    /// Whether a successful walk stopped before visiting all reachable nodes.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }
}

/// Complete browse results restored to original query order.
///
/// Parent indices refer to the node list within each query, never another query.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct BrowseResponse {
    items: Vec<BrowseResult>,
    started_at: Instant,
    finished_at: Instant,
}

impl BrowseResponse {
    /// Assemble exactly one result per query under the request's total node bound.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab::{BrowseRequest, BrowseResponse, BrowseResult, ResourceLimits, TagId};
    /// let id = TagId::new("objects")?;
    /// let request = BrowseRequest::builder(ResourceLimits::builder().build()?)
    ///     .query(id.clone(), "").build()?;
    /// let now = Instant::now();
    /// let response = BrowseResponse::builder(&request, now, now)
    ///     .item(BrowseResult::new(id.clone(), Ok(Vec::new())).with_truncated(true))
    ///     .build()?;
    /// assert!(response.get(&id).unwrap().truncated());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "build and validate the retained changes"]
    pub fn builder(
        request: &BrowseRequest,
        started_at: Instant,
        finished_at: Instant,
    ) -> BrowseResponseBuilder {
        BrowseResponseBuilder {
            expected: request
                .queries()
                .iter()
                .map(|query| query.id().clone())
                .collect(),
            items: Vec::new(),
            nodes: 0,
            max_nodes: request.max_nodes(),
            started_at,
            finished_at,
            error: (finished_at < started_at).then(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    "browse response finishes before it starts",
                )
            }),
        }
    }

    /// All query results in original caller order, including errors.
    #[must_use]
    pub fn items(&self) -> &[BrowseResult] {
        &self.items
    }
    /// Find a result by caller identity, not by node name/address.
    #[must_use]
    pub fn get(&self, query: &TagId) -> Option<&BrowseResult> {
        self.items.iter().find(|item| item.query() == query)
    }
    /// Successful queries, including successful empty and truncated walks.
    pub fn iter_ok(&self) -> impl Iterator<Item = &BrowseResult> {
        self.items.iter().filter(|item| item.outcome().is_ok())
    }
    /// Failed queries; partial failure is data rather than a batch error.
    pub fn iter_failed(&self) -> impl Iterator<Item = &BrowseResult> {
        self.items.iter().filter(|item| item.outcome().is_err())
    }
    /// Operation start on the connection's monotonic clock.
    #[must_use]
    pub const fn started_at(&self) -> Instant {
        self.started_at
    }
    /// Operation completion on the same monotonic clock.
    #[must_use]
    pub const fn finished_at(&self) -> Instant {
        self.finished_at
    }
    /// Take correlated results without copying the node lists.
    #[must_use]
    pub fn into_items(self) -> Vec<BrowseResult> {
        self.items
    }
}

/// Sticky validation of correlation, parent forests, operation time and total nodes.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct BrowseResponseBuilder {
    expected: Vec<TagId>,
    items: Vec<(usize, BrowseResult)>,
    nodes: usize,
    max_nodes: NonZeroUsize,
    started_at: Instant,
    finished_at: Instant,
    error: Option<PlcError>,
}

impl BrowseResponseBuilder {
    /// Validate one query result before inserting it; preserve the first error.
    /// Node counts are checked before inspecting parent links.
    #[must_use = "build and validate the retained changes"]
    pub fn item(mut self, item: BrowseResult) -> Self {
        if self.error.is_some() {
            return self;
        }
        let Some(position) = self.expected.iter().position(|id| id == item.query()) else {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "unexpected browse query identifier",
            ));
            return self;
        };
        if self.items.iter().any(|(existing, _)| *existing == position) {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "duplicate browse query result",
            ));
            return self;
        }
        let count = item.outcome().as_ref().map_or(0, Vec::len);
        let Some(total) = self
            .nodes
            .checked_add(count)
            .filter(|total| *total <= self.max_nodes.get())
        else {
            self.error = Some(PlcError::new(
                ErrorKind::ResourceLimit,
                "browse response exceeds total node budget",
            ));
            return self;
        };
        match item.outcome() {
            Ok(nodes) => {
                if nodes
                    .iter()
                    .enumerate()
                    .any(|(index, node)| node.parent().is_some_and(|parent| parent >= index))
                {
                    self.error = Some(PlcError::new(
                        ErrorKind::MalformedData,
                        "browse parent must precede its child",
                    ));
                    return self;
                }
            }
            Err(_) if item.truncated() => {
                self.error = Some(PlcError::new(
                    ErrorKind::MalformedData,
                    "failed browse query cannot be truncated",
                ));
                return self;
            }
            Err(_) => {}
        }
        self.nodes = total;
        self.items.push((position, item));
        self
    }

    /// Require all named results and restore caller order without merging equal names.
    ///
    /// # Errors
    /// `InvalidConfig` for reversed time; `MalformedData` for missing/duplicate/foreign
    /// identities, non-backward parents or a truncated error; `ResourceLimit` for excess nodes.
    /// Per-query errors are valid response data, including a batch where every query failed.
    pub fn build(mut self) -> Result<BrowseResponse> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.items.len() != self.expected.len() {
            return Err(PlcError::new(
                ErrorKind::MalformedData,
                "browse response is missing query results",
            ));
        }
        self.items.sort_unstable_by_key(|(position, _)| *position);
        Ok(BrowseResponse {
            items: self.items.into_iter().map(|(_, item)| item).collect(),
            started_at: self.started_at,
            finished_at: self.finished_at,
        })
    }
}
