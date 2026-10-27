use std::num::NonZeroUsize;

use crate::{ErrorKind, PlcError, Result};

/// Finite per-connection budgets, validated before a driver allocates resources.
///
/// These are library budgets, not claims about a device's advertised capabilities.
/// All fields are private so the buffer/frame relationship remains valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ResourceLimits {
    frame_bytes: NonZeroUsize,
    buffered_bytes: NonZeroUsize,
    pending_requests: NonZeroUsize,
    queued_events: NonZeroUsize,
    items_per_request: NonZeroUsize,
}

impl ResourceLimits {
    /// Configure finite budgets without performing I/O or allocating the buffers.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ResourceLimits;
    /// let limits = ResourceLimits::builder()
    ///     .max_frame_bytes(260)
    ///     .max_buffered_bytes(520)
    ///     .build()?;
    /// assert_eq!(limits.max_frame_bytes().get(), 260);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "configure and validate the resource budgets before use"]
    pub fn builder() -> ResourceLimitsBuilder {
        ResourceLimitsBuilder::default()
    }

    /// Maximum bytes in any individual frame.
    #[must_use]
    pub const fn max_frame_bytes(self) -> NonZeroUsize {
        self.frame_bytes
    }

    /// Maximum bytes retained in the connection's receive buffer.
    #[must_use]
    pub const fn max_buffered_bytes(self) -> NonZeroUsize {
        self.buffered_bytes
    }

    /// Maximum requests owned by the connection at once.
    #[must_use]
    pub const fn max_pending_requests(self) -> NonZeroUsize {
        self.pending_requests
    }

    /// Maximum events retained for a bounded subscription.
    #[must_use]
    pub const fn max_queued_events(self) -> NonZeroUsize {
        self.queued_events
    }

    /// Maximum tags accepted in a single request.
    #[must_use]
    pub const fn max_items_per_request(self) -> NonZeroUsize {
        self.items_per_request
    }
}

/// Resource-budget configuration.
///
/// Defaults: 65,536 frame bytes, 262,144 buffered bytes, 16 pending requests,
/// 256 queued events and 1,024 items per request. Every bound must be positive,
/// and the receive buffer must accommodate a complete frame. No allocation
/// occurs until a future transport/driver uses the validated limits.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
#[must_use]
pub struct ResourceLimitsBuilder {
    frame_bytes: usize,
    buffered_bytes: usize,
    pending_requests: usize,
    queued_events: usize,
    items_per_request: usize,
}

impl Default for ResourceLimitsBuilder {
    fn default() -> Self {
        Self {
            frame_bytes: 65_536,
            buffered_bytes: 262_144,
            pending_requests: 16,
            queued_events: 256,
            items_per_request: 1_024,
        }
    }
}

impl ResourceLimitsBuilder {
    /// Set the maximum individual frame size in bytes.
    #[must_use = "builder changes must be retained"]
    pub const fn max_frame_bytes(mut self, value: usize) -> Self {
        self.frame_bytes = value;
        self
    }

    /// Set the maximum accumulated receive-buffer size in bytes.
    #[must_use = "builder changes must be retained"]
    pub const fn max_buffered_bytes(mut self, value: usize) -> Self {
        self.buffered_bytes = value;
        self
    }

    /// Set the maximum number of requests owned concurrently.
    #[must_use = "builder changes must be retained"]
    pub const fn max_pending_requests(mut self, value: usize) -> Self {
        self.pending_requests = value;
        self
    }

    /// Set the maximum number of retained subscription events.
    #[must_use = "builder changes must be retained"]
    pub const fn max_queued_events(mut self, value: usize) -> Self {
        self.queued_events = value;
        self
    }

    /// Set the maximum tag count in a request.
    #[must_use = "builder changes must be retained"]
    pub const fn max_items_per_request(mut self, value: usize) -> Self {
        self.items_per_request = value;
        self
    }

    /// Validate all budgets without allocating resources.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] if any budget is zero or the receive
    /// buffer is smaller than the maximum frame.
    #[must_use = "resource configuration validation must be checked"]
    pub fn build(self) -> Result<ResourceLimits> {
        let limits = ResourceLimits {
            frame_bytes: positive("max_frame_bytes", self.frame_bytes)?,
            buffered_bytes: positive("max_buffered_bytes", self.buffered_bytes)?,
            pending_requests: positive("max_pending_requests", self.pending_requests)?,
            queued_events: positive("max_queued_events", self.queued_events)?,
            items_per_request: positive("max_items_per_request", self.items_per_request)?,
        };
        if limits.buffered_bytes < limits.frame_bytes {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "max_buffered_bytes must be at least max_frame_bytes",
            ));
        }
        Ok(limits)
    }
}

fn positive(name: &str, value: usize) -> Result<NonZeroUsize> {
    NonZeroUsize::new(value).ok_or_else(|| {
        PlcError::new(
            ErrorKind::InvalidConfig,
            format!("{name} must be greater than zero"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_budgets_are_finite_and_consistent() {
        let limits = ResourceLimits::builder().build().expect("valid defaults");
        assert_eq!(limits.max_frame_bytes().get(), 65_536, "frame default");
        assert_eq!(limits.max_buffered_bytes().get(), 262_144, "buffer default");
        assert_eq!(limits.max_pending_requests().get(), 16, "request default");
        assert_eq!(limits.max_queued_events().get(), 256, "event default");
        assert_eq!(limits.max_items_per_request().get(), 1_024, "item default");
    }

    #[test]
    fn custom_budgets_and_equal_frame_buffer_sizes_are_preserved() {
        let limits = ResourceLimits::builder()
            .max_frame_bytes(260)
            .max_buffered_bytes(260)
            .max_pending_requests(1)
            .max_queued_events(2)
            .max_items_per_request(3)
            .build()
            .expect("equal frame and buffer bounds are valid");
        assert_eq!(limits.max_frame_bytes().get(), 260, "frame retained");
        assert_eq!(limits.max_buffered_bytes().get(), 260, "buffer retained");
        assert_eq!(
            limits.max_pending_requests().get(),
            1,
            "single pending request"
        );
        assert_eq!(limits.max_queued_events().get(), 2, "queue retained");
        assert_eq!(
            limits.max_items_per_request().get(),
            3,
            "item bound retained"
        );
    }

    // covers: SPEC mandatory-failure-cases / resource limits
    #[test]
    fn zero_budgets_and_undersized_buffers_fail_before_allocation() {
        for (field, builder) in [
            (
                "max_frame_bytes",
                ResourceLimits::builder().max_frame_bytes(0),
            ),
            (
                "max_buffered_bytes",
                ResourceLimits::builder().max_buffered_bytes(0),
            ),
            (
                "max_pending_requests",
                ResourceLimits::builder().max_pending_requests(0),
            ),
            (
                "max_queued_events",
                ResourceLimits::builder().max_queued_events(0),
            ),
            (
                "max_items_per_request",
                ResourceLimits::builder().max_items_per_request(0),
            ),
        ] {
            let error = builder.build().expect_err("zero budget invalid");
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidConfig,
                "typed invalid budget"
            );
            assert!(
                error.to_string().contains(field),
                "identifies invalid budget {field}"
            );
        }
        let error = ResourceLimits::builder()
            .max_buffered_bytes(1)
            .build()
            .expect_err("buffer must hold frame");
        assert_eq!(
            error.kind(),
            ErrorKind::InvalidConfig,
            "typed inconsistent budgets"
        );
    }
}
