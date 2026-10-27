use std::num::NonZeroU32;
use std::time::Instant;

use crate::{ErrorKind, PlcError, ReadItem, ReadRequest, Result, TagId};

/// Complete correlated batch, including each individual failure.
///
/// Items are immutable and restored to original request order. Sampling times
/// remain per item; splitting a batch is not an atomic snapshot.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct ReadResponse {
    items: Vec<ReadItem>,
    started_at: Instant,
    finished_at: Instant,
    split_into: NonZeroU32,
}

impl ReadResponse {
    /// Assemble results for exactly one validated request.
    ///
    /// The builder records an invalid operation interval as a sticky error;
    /// all validation occurs before any I/O can be performed by these types.
    ///
    /// # Examples
    /// ```
    /// use std::time::Instant;
    /// use stonecrab_core::{ReadRequest, ReadResponse, ReadResponseBuilder};
    /// fn begin_response(request: &ReadRequest, start: Instant, finish: Instant) -> ReadResponseBuilder {
    ///     ReadResponse::builder(request, start, finish)
    /// }
    /// ```
    #[must_use = "assemble and validate the correlated results"]
    pub fn builder(
        request: &ReadRequest,
        started_at: Instant,
        finished_at: Instant,
    ) -> ReadResponseBuilder {
        ReadResponseBuilder {
            expected: request.items().iter().map(|tag| tag.id().clone()).collect(),
            items: Vec::new(),
            started_at,
            finished_at,
            split_into: NonZeroU32::MIN,
            error: (finished_at < started_at).then(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    "read response finishes before it starts",
                )
            }),
        }
    }

    /// All results, including failures, in original request order.
    #[must_use]
    pub fn items(&self) -> &[ReadItem] {
        &self.items
    }

    /// Find an item by stable caller identity, not by device address.
    #[must_use]
    pub fn get(&self, id: &TagId) -> Option<&ReadItem> {
        self.items.iter().find(|item| item.tag() == id)
    }

    /// Successful items without discarding their correlation or timestamps.
    pub fn iter_ok(&self) -> impl Iterator<Item = &ReadItem> {
        self.items.iter().filter(|item| item.outcome().is_ok())
    }

    /// Failed items without turning partial failure into a batch failure.
    pub fn iter_failed(&self) -> impl Iterator<Item = &ReadItem> {
        self.items.iter().filter(|item| item.outcome().is_err())
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

    /// Number of wire batches used; greater than one is explicitly disclosed.
    #[must_use]
    pub const fn split_into(&self) -> NonZeroU32 {
        self.split_into
    }

    /// Take every result, preserving original request order.
    #[must_use]
    pub fn into_items(self) -> Vec<ReadItem> {
        self.items
    }
}

/// Bounded correlation validator for a read response.
///
/// Defaults to one wire batch. Result count cannot exceed the validated request;
/// unexpected identities, duplicates and sampling times outside the operation
/// interval fail before insertion. The first error is sticky. Failed item outcomes
/// are valid data, including a batch where every item failed.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct ReadResponseBuilder {
    expected: Vec<TagId>,
    items: Vec<(usize, ReadItem)>,
    started_at: Instant,
    finished_at: Instant,
    split_into: NonZeroU32,
    error: Option<PlcError>,
}

impl ReadResponseBuilder {
    /// Retain one item after validating identity, uniqueness and local time.
    #[must_use = "builder changes must be retained"]
    pub fn item(mut self, item: ReadItem) -> Self {
        if self.error.is_some() {
            return self;
        }
        let Some(position) = self.expected.iter().position(|id| id == item.tag()) else {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "read response contains an unexpected tag identifier",
            ));
            return self;
        };
        if self.items.iter().any(|(existing, _)| *existing == position) {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "read response repeats a tag identifier",
            ));
        } else if item.sampled_at() < self.started_at || item.sampled_at() > self.finished_at {
            self.error = Some(PlcError::new(
                ErrorKind::MalformedData,
                "read sampling time is outside the operation interval",
            ));
        } else {
            self.items.push((position, item));
        }
        self
    }

    /// Disclose how many protocol batches were used for acquisition.
    #[must_use = "builder changes must be retained"]
    pub const fn split_into(mut self, split_into: NonZeroU32) -> Self {
        self.split_into = split_into;
        self
    }

    /// Require exactly one result per requested identity and restore caller order.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for a reversed operation interval,
    /// or [`ErrorKind::MalformedData`] for missing, unexpected or duplicate items
    /// and invalid sampling times. Individual item errors do not fail this method.
    #[must_use = "response correlation validation must be checked"]
    pub fn build(mut self) -> Result<ReadResponse> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.items.len() != self.expected.len() {
            return Err(PlcError::new(
                ErrorKind::MalformedData,
                "read response is missing requested items",
            ));
        }
        self.items.sort_unstable_by_key(|(position, _)| *position);
        Ok(ReadResponse {
            items: self.items.into_iter().map(|(_, item)| item).collect(),
            started_at: self.started_at,
            finished_at: self.finished_at,
            split_into: self.split_into,
        })
    }
}
