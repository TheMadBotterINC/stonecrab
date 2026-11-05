use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroUsize;

use crate::{CancellationToken, Deadline, ErrorKind, PlcError, ResourceLimits, Result, TagId};

/// A named opaque protocol query. Blank text requests the driver's default starting node.
///
/// Core bounds the text but leaves syntax validation to the driver, per query. Debug
/// reports only identity and byte length because the text has not been parsed/redacted.
#[derive(Clone)]
#[non_exhaustive]
pub struct BrowseQuery {
    id: TagId,
    text: String,
}

impl BrowseQuery {
    /// Maximum query text size in UTF-8 bytes.
    pub const MAX_TEXT_BYTES: usize = 8192;

    /// Retain a named query after checking its byte bound, without parsing or I/O.
    ///
    /// # Errors
    /// Returns [`ErrorKind::ResourceLimit`] before copying oversized text.
    pub fn new(id: TagId, text: &str) -> Result<Self> {
        if text.len() > Self::MAX_TEXT_BYTES {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "browse query exceeds text budget",
            ));
        }
        Ok(Self {
            id,
            text: text.to_owned(),
        })
    }

    /// Caller identity, independent of the starting address or node name.
    #[must_use]
    pub const fn id(&self) -> &TagId {
        &self.id
    }

    /// Exact unparsed text, including an empty default-start query.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

impl fmt::Debug for BrowseQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BrowseQuery")
            .field("id", &self.id)
            .field("text_bytes", &self.text.len())
            .finish()
    }
}

/// Immutable named browse queries, bounded before any driver performs I/O.
///
/// No explicit deadline means the connection supplies its request timeout. An override
/// must enforce its own connection limits even when a caller builds with looser limits.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BrowseRequest {
    queries: Vec<BrowseQuery>,
    max_nodes: NonZeroUsize,
    deadline: Option<Deadline>,
    cancel: Option<CancellationToken>,
}

impl BrowseRequest {
    /// Begin a finite browse batch using the connection's resource limits.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{BrowseRequest, ResourceLimits, TagId};
    /// let request = BrowseRequest::builder(ResourceLimits::builder().build()?)
    ///     .query(TagId::new("objects")?, "")
    ///     .build()?;
    /// assert_eq!(request.queries()[0].text(), "");
    /// assert_eq!(request.max_nodes().get(), 10000);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "build and validate the retained changes"]
    pub const fn builder(limits: ResourceLimits) -> BrowseRequestBuilder {
        BrowseRequestBuilder {
            queries: Vec::new(),
            identifiers: BTreeSet::new(),
            max_queries: limits.max_items_per_request(),
            max_nodes: limits.max_browse_nodes(),
            deadline: None,
            cancel: None,
            error: None,
        }
    }

    /// Named queries in original caller order.
    #[must_use]
    pub fn queries(&self) -> &[BrowseQuery] {
        &self.queries
    }

    /// Total nodes allowed across all query results.
    #[must_use]
    pub const fn max_nodes(&self) -> NonZeroUsize {
        self.max_nodes
    }

    /// Caller cutoff, or absence requiring the connection's bounded default.
    #[must_use]
    pub const fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// Caller cancellation token, retained even when already cancelled.
    #[must_use]
    pub const fn cancel(&self) -> Option<&CancellationToken> {
        self.cancel.as_ref()
    }
}

/// Sticky validation of unique named queries and finite result limits.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct BrowseRequestBuilder {
    queries: Vec<BrowseQuery>,
    identifiers: BTreeSet<TagId>,
    max_queries: NonZeroUsize,
    max_nodes: NonZeroUsize,
    deadline: Option<Deadline>,
    cancel: Option<CancellationToken>,
    error: Option<PlcError>,
}

impl BrowseRequestBuilder {
    /// Add one query; validate count and identity before copying its bounded text.
    #[must_use = "build and validate the retained changes"]
    pub fn query(mut self, id: TagId, text: &str) -> Self {
        if self.error.is_some() {
            return self;
        }
        if self.queries.len() >= self.max_queries.get() {
            self.error = Some(PlcError::new(
                ErrorKind::ResourceLimit,
                "browse request exceeds query budget",
            ));
        } else if self.identifiers.contains(&id) {
            self.error = Some(PlcError::new(
                ErrorKind::InvalidConfig,
                "duplicate browse query identifier",
            ));
        } else {
            match BrowseQuery::new(id, text) {
                Ok(query) => {
                    self.identifiers.insert(query.id().clone());
                    self.queries.push(query);
                }
                Err(error) => self.error = Some(error),
            }
        }
        self
    }

    /// Retain the absolute cutoff for the connection to check on its own clock.
    #[must_use = "build and validate the retained changes"]
    pub const fn deadline(mut self, deadline: Deadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Attach or replace a token; clones share the caller's cancellation flag.
    #[must_use = "build and validate the retained changes"]
    pub fn cancel(mut self, cancel: CancellationToken) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Require at least one query and return the first validation error, if any.
    ///
    /// # Errors
    /// `InvalidConfig` for an empty batch or duplicate identity; `ResourceLimit` for excess
    /// queries or query text. Query syntax is deliberately left to a driver's per-query result.
    pub fn build(self) -> Result<BrowseRequest> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.queries.is_empty() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "browse request must contain a query",
            ));
        }
        Ok(BrowseRequest {
            queries: self.queries,
            max_nodes: self.max_nodes,
            deadline: self.deadline,
            cancel: self.cancel,
        })
    }
}
