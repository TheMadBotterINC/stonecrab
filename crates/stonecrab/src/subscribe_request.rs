use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::time::Duration;

use crate::{CancellationToken, Deadline, ErrorKind, PlcError, ResourceLimits, Result, Tag, TagId};

/// What makes a subscription deliver an event: the upstream subscription types.
///
/// One kind applies to every tag of a [`SubscribeRequest`] (ADR-0020 decision 3); a caller
/// who needs two kinds makes two subscriptions.
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// Corresponds to the upstream `PlcSubscriptionType` (`CYCLIC`, `CHANGE_OF_STATE`,
/// `EVENT`). Upstream sets the type per tag; Stonecrab sets it per request. The optional
/// minimum interval of an upstream change-of-state tag has no counterpart: the polling
/// helper upstream ignores it, and a polling driver discloses the interval it uses through
/// [`Subscription::interval`](crate::Subscription::interval).
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::SubscriptionKind;
/// let every_second = SubscriptionKind::Cyclic(Duration::from_secs(1));
/// assert_ne!(every_second, SubscriptionKind::ChangeOfState);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SubscriptionKind {
    /// An event for every tag at this interval, whether or not the value changed. The
    /// interval must not be zero.
    Cyclic(Duration),
    /// An event for a tag when its value changes, and once for its first value.
    ChangeOfState,
    /// An event when the device raises one; only a native subscription can deliver it.
    Event,
}

/// What a subscription does when its consumer falls behind and its bounded queue is full.
///
/// The queue holds at most [`ResourceLimits::max_queued_events`] events. Every policy that
/// gives up an event counts it in [`LossCounters`](crate::LossCounters): silent loss is
/// prohibited (SPEC.md 'Connection and resource behavior'). There is no default; a request
/// names its policy (ADR-0020 decision 2).
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// A Stonecrab addition: upstream calls its consumers on the polling thread, with no queue
/// and no loss counter.
///
/// # Examples
/// ```
/// use stonecrab::QueuePolicy;
/// let lossless = QueuePolicy::Backpressure;
/// assert_ne!(lossless, QueuePolicy::DropOldest);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum QueuePolicy {
    /// Lose nothing: a full queue makes the producer wait until the consumer takes an
    /// event, so a polling producer stops reading the device meanwhile.
    Backpressure,
    /// Keep at most one pending event per tag, the latest: a newer event for a tag
    /// replaces the pending one in place and counts in
    /// [`LossCounters::coalesced`](crate::LossCounters::coalesced).
    CoalesceLatest,
    /// Make room by discarding the oldest pending event, counted in
    /// [`LossCounters::dropped`](crate::LossCounters::dropped).
    DropOldest,
}

/// Immutable bounded subscription request, validated without performing I/O.
///
/// The deadline and the cancellation token bound the
/// [`subscribe`](crate::Connection::subscribe) call, exactly as they bound a read: no
/// deadline means the connection supplies `Deadline::after(clock.now(), timeouts.request)`
/// from its own clock (ADR-0007), and an elapsed deadline or a cancelled token is rejected
/// before anything is transmitted. Neither bounds the lifetime of the
/// [`Subscription`](crate::Subscription), which ends when it is unsubscribed or dropped, or
/// when the connection closes.
///
/// # Wire effects
/// None; this is data. Nothing is transmitted until a connection subscribes.
///
/// # Compatibility: docs/parity/core.md#CORE-SUBSCRIPTION
/// Corresponds to the upstream `PlcSubscriptionRequest`, with one kind for the whole
/// request and a declared queue policy.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SubscribeRequest {
    items: Vec<Tag>,
    kind: SubscriptionKind,
    queue: QueuePolicy,
    deadline: Option<Deadline>,
    cancel: Option<CancellationToken>,
}

impl SubscribeRequest {
    /// Begin a request with an explicit finite item budget, its kind and its queue policy.
    ///
    /// Defaults and error behavior are documented on [`SubscribeRequestBuilder`].
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab::{ErrorKind, QueuePolicy, ResourceLimits, SubscribeRequest, SubscriptionKind};
    /// let limits = ResourceLimits::builder().build()?;
    /// let kind = SubscriptionKind::Cyclic(Duration::from_secs(1));
    /// let error = SubscribeRequest::builder(limits, kind, QueuePolicy::DropOldest)
    ///     .build()
    ///     .expect_err("empty batch");
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "build and validate the request before use"]
    pub const fn builder(
        limits: ResourceLimits,
        kind: SubscriptionKind,
        queue: QueuePolicy,
    ) -> SubscribeRequestBuilder {
        SubscribeRequestBuilder {
            items: Vec::new(),
            identifiers: BTreeSet::new(),
            max_items: limits.max_items_per_request(),
            max_queued: limits.max_queued_events(),
            kind,
            queue,
            deadline: None,
            cancel: None,
            error: None,
        }
    }

    /// Subscribed tags in original caller order.
    #[must_use]
    pub fn items(&self) -> &[Tag] {
        &self.items
    }

    /// What makes the subscription deliver an event.
    #[must_use]
    pub const fn kind(&self) -> SubscriptionKind {
        self.kind
    }

    /// What the subscription does when its consumer falls behind.
    #[must_use]
    pub const fn queue(&self) -> QueuePolicy {
        self.queue
    }

    /// Explicit cutoff for the subscribe call, or absence requiring the connection's
    /// bounded default.
    #[must_use]
    pub const fn deadline(&self) -> Option<Deadline> {
        self.deadline
    }

    /// The token the caller can cancel to abandon the subscribe call, if one was supplied.
    #[must_use]
    pub const fn cancel(&self) -> Option<&CancellationToken> {
        self.cancel.as_ref()
    }
}

/// A bounded subscription request under construction.
///
/// Starts empty with no explicit deadline and no cancel token. `tag` checks the item budget
/// and duplicates before growing internal collections, and the first error is sticky, as on
/// [`ReadRequestBuilder`](crate::ReadRequestBuilder). `build` also rejects an empty
/// request, a zero cyclic interval, and a [`QueuePolicy::CoalesceLatest`] request with more
/// tags than [`ResourceLimits::max_queued_events`], whose queue could not hold one event per
/// tag. No I/O is performed.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct SubscribeRequestBuilder {
    items: Vec<Tag>,
    identifiers: BTreeSet<TagId>,
    max_items: NonZeroUsize,
    max_queued: NonZeroUsize,
    kind: SubscriptionKind,
    queue: QueuePolicy,
    deadline: Option<Deadline>,
    cancel: Option<CancellationToken>,
    error: Option<PlcError>,
}

impl SubscribeRequestBuilder {
    /// Add one tag in caller order, rejecting overflow and duplicate identity.
    #[must_use = "builder changes must be retained"]
    pub fn tag(mut self, tag: Tag) -> Self {
        if self.error.is_some() {
            return self;
        }
        if self.items.len() >= self.max_items.get() {
            self.error = Some(PlcError::new(
                ErrorKind::ResourceLimit,
                "subscribe request exceeds item budget",
            ));
        } else if !self.identifiers.insert(tag.id().clone()) {
            self.error = Some(PlcError::new(
                ErrorKind::InvalidConfig,
                "duplicate tag identifier in subscribe request",
            ));
        } else {
            self.items.push(tag);
        }
        self
    }

    /// Supply an absolute cutoff for the subscribe call; an elapsed deadline is retained so
    /// the connection can reject it against its own clock without transmitting.
    #[must_use = "builder changes must be retained"]
    pub const fn deadline(mut self, deadline: Deadline) -> Self {
        self.deadline = Some(deadline);
        self
    }

    /// Attach a token the caller can cancel to abandon the subscribe call.
    ///
    /// The token is retained as given and a cancelled one is kept for the connection to
    /// reject; a second call replaces the first.
    #[must_use = "builder changes must be retained"]
    pub fn cancel(mut self, token: CancellationToken) -> Self {
        self.cancel = Some(token);
        self
    }

    /// Finish validation and retain original order without performing I/O.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for an empty request, duplicate identifiers or
    /// a zero cyclic interval, and [`ErrorKind::ResourceLimit`] for an exceeded item budget
    /// or a coalescing request with more tags than the queue holds.
    #[must_use = "subscribe request validation must be checked"]
    pub fn build(self) -> Result<SubscribeRequest> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if self.items.is_empty() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "subscribe request must contain at least one tag",
            ));
        }
        if self.kind == SubscriptionKind::Cyclic(Duration::ZERO) {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "a cyclic subscription needs a non-zero interval",
            ));
        }
        if self.queue == QueuePolicy::CoalesceLatest && self.items.len() > self.max_queued.get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "a coalescing subscription has more tags than max_queued_events",
            ));
        }
        Ok(SubscribeRequest {
            items: self.items,
            kind: self.kind,
            queue: self.queue,
            deadline: self.deadline,
            cancel: self.cancel,
        })
    }
}
