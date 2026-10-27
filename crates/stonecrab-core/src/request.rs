use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::{Deadline, ErrorKind, PlcError, ResourceLimits, Result, Tag, TagId};

/// Immutable bounded read batch, validated without performing I/O.
///
/// No deadline means the future connection must supply its configured deadline
/// before transmission, not that unbounded I/O is permitted.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct ReadRequest {
    items: Vec<Tag>,
    deadline: Option<Deadline>,
}

impl ReadRequest {
    /// Begin a request with an explicit finite item budget.
    ///
    /// Defaults and error behavior are documented on [`ReadRequestBuilder`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{ErrorKind, ReadRequest, ResourceLimits};
    /// let limits = ResourceLimits::builder().build()?;
    /// let error = ReadRequest::builder(limits).build().expect_err("empty batch");
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use = "build and validate the request before use"]
    pub const fn builder(limits: ResourceLimits) -> ReadRequestBuilder {
        ReadRequestBuilder {
            items: Vec::new(),
            identifiers: BTreeSet::new(),
            max_items: limits.max_items_per_request(),
            deadline: None,
            error: None,
        }
    }

    /// Requested tags in original caller order.
    #[must_use]
    pub fn items(&self) -> &[Tag] {
        &self.items
    }

    /// Explicit cutoff, or absence requiring the connection's bounded default.
    #[must_use]
    pub const fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }
}

/// A bounded read batch under construction.
///
/// Starts empty with no explicit deadline. `tag` checks duplicates and the item
/// budget before growing internal collections. The first validation error is
/// sticky: subsequent tags are discarded without allocation, and `build` returns
/// that error. Empty requests are rejected. No I/O is performed.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct ReadRequestBuilder {
    items: Vec<Tag>,
    identifiers: BTreeSet<TagId>,
    max_items: NonZeroUsize,
    deadline: Option<Deadline>,
    error: Option<PlcError>,
}

impl ReadRequestBuilder {
    /// Add one tag in caller order, rejecting overflow and duplicate identity.
    #[must_use = "builder changes must be retained"]
    pub fn tag(mut self, tag: Tag) -> Self {
        if self.error.is_some() {
            return self;
        }
        if self.items.len() >= self.max_items.get() {
            self.error = Some(PlcError::new(
                ErrorKind::ResourceLimit,
                "read request exceeds item budget",
            ));
        } else if !self.identifiers.insert(tag.id().clone()) {
            self.error = Some(PlcError::new(
                ErrorKind::InvalidConfig,
                "duplicate tag identifier in read request",
            ));
        } else {
            self.items.push(tag);
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
    /// Returns [`ErrorKind::InvalidConfig`] for empty requests or duplicate
    /// identifiers, or [`ErrorKind::ResourceLimit`] for an exceeded item budget.
    #[must_use = "read request validation must be checked"]
    pub fn build(self) -> Result<ReadRequest> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.items.is_empty() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "read request must contain at least one tag",
            ));
        }
        Ok(ReadRequest {
            items: self.items,
            deadline: self.deadline,
        })
    }
}
