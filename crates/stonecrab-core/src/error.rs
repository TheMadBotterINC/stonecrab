use std::error::Error;
use std::time::{Duration, Instant};

use crate::Operation;

/// Workspace-wide fallible operation result.
pub type Result<T> = std::result::Result<T, PlcError>;

/// Stable failure classification; messages are not intended for programmatic matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ErrorKind {
    /// Invalid configuration or a value incompatible with the requested type.
    InvalidConfig,
    /// A malformed or ambiguous protocol address.
    InvalidAddress,
    /// The named operation is not implemented.
    Unsupported(Operation),
    /// The bounded operation deadline elapsed.
    Timeout,
    /// The caller cancelled the operation.
    Cancelled,
    /// Connectivity was lost.
    Disconnected,
    /// Credentials were rejected.
    Authentication,
    /// Peer identity or certificate validation failed.
    Trust,
    /// The received data violates its declared format.
    MalformedData,
    /// The device returned a protocol exception.
    DeviceException,
    /// A configured resource bound was exceeded.
    ResourceLimit,
    /// A bounded producer could not proceed.
    Backpressure,
    /// An internal invariant could not be maintained.
    Internal,
}

/// Observable connection lifecycle, without an executor dependency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConnectionState {
    /// No connection has been attempted.
    Disconnected,
    /// Opening a transport or completing a handshake.
    Connecting,
    /// Available for operations.
    Connected,
    /// Waiting to retry opening the transport.
    Reconnecting {
        /// One-based reconnect attempt.
        attempt: u32,
        /// Monotonic time at which the attempt may begin.
        next_at: Instant,
    },
    /// Releasing pending work and resources.
    Closing,
    /// Terminal explicit shutdown.
    Closed,
    /// Terminal failure.
    Failed(ErrorKind),
}

/// Broad diagnostic category, separate from protocol-specific error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FailureCategory {
    /// Rejected before any I/O.
    Validation,
    /// Transport or connection lifecycle failure.
    Transport,
    /// Invalid protocol data or a device exception.
    Protocol,
    /// Internal implementation failure.
    Internal,
}

/// Context captured at the failure site.
///
/// Optional fields stay absent until known; a core validation error must not
/// invent a connection state or operation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Diagnostics {
    /// Operation being attempted, when known.
    pub operation: Option<Operation>,
    /// Connection state at the failure site, when known.
    pub connection_state: Option<ConnectionState>,
    /// Elapsed monotonic duration, when measured.
    pub latency: Option<Duration>,
    /// Number of retries already performed.
    pub retries: u32,
    /// Broad failure category, when known.
    pub category: Option<FailureCategory>,
}

/// Typed failure with a small boxed representation and preserved source chain.
///
/// Messages and sources supplied by callers must already be redacted: this type
/// cannot identify secrets inside arbitrary third-party errors.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[error(transparent)]
pub struct PlcError {
    inner: Box<ErrorInner>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
struct ErrorInner {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
    diagnostics: Diagnostics,
}

impl PlcError {
    /// Construct a failure without inventing operation context.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{ErrorKind, PlcError};
    /// let error = PlcError::new(ErrorKind::Timeout, "request deadline elapsed");
    /// assert_eq!(error.kind(), ErrorKind::Timeout);
    /// assert_eq!(error.to_string(), "request deadline elapsed");
    /// ```
    #[must_use]
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            inner: Box::new(ErrorInner {
                kind,
                message: message.into(),
                source: None,
                diagnostics: Diagnostics::default(),
            }),
        }
    }

    /// Attach an already-redacted original failure, preserving its type.
    #[must_use]
    pub fn with_source(mut self, source: impl Error + Send + Sync + 'static) -> Self {
        self.inner.source = Some(Box::new(source));
        self
    }

    /// Attach context collected where the failure occurred.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Diagnostics) -> Self {
        self.inner.diagnostics = diagnostics;
        self
    }

    /// Failure discriminator for programmatic matching.
    #[must_use]
    pub const fn kind(&self) -> ErrorKind {
        self.inner.kind
    }

    /// Context captured by the operation.
    #[must_use]
    pub const fn diagnostics(&self) -> &Diagnostics {
        &self.inner.diagnostics
    }

    /// Whether a read retry policy may consider the failure transient.
    ///
    /// This does not authorize retries and must never be used to replay writes.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self.kind(),
            ErrorKind::Timeout | ErrorKind::Disconnected | ErrorKind::Backpressure
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: ALPHA_PLAN D13 / additive Ord on ErrorKind (BTreeSet<ErrorKind>)
    #[test]
    fn error_kinds_sort_in_declaration_order_and_round_trip_through_a_set() {
        let declared = [
            ErrorKind::InvalidConfig,
            ErrorKind::InvalidAddress,
            ErrorKind::Unsupported(Operation::Connect),
            ErrorKind::Unsupported(Operation::Close),
            ErrorKind::Timeout,
            ErrorKind::Cancelled,
            ErrorKind::Disconnected,
            ErrorKind::Authentication,
            ErrorKind::Trust,
            ErrorKind::MalformedData,
            ErrorKind::DeviceException,
            ErrorKind::ResourceLimit,
            ErrorKind::Backpressure,
            ErrorKind::Internal,
        ];
        let mut reversed = declared;
        reversed.reverse();
        let mut sorted = reversed.to_vec();
        sorted.sort();
        assert_eq!(
            sorted, declared,
            "variants by declaration, payload by Operation order"
        );
        for (left, right) in declared.iter().zip(declared.iter().skip(1)) {
            assert!(left < right, "strictly increasing");
            assert_eq!(left.partial_cmp(right), Some(std::cmp::Ordering::Less));
        }
        let set: std::collections::BTreeSet<_> = reversed.into_iter().collect();
        assert_eq!(set.len(), declared.len(), "no duplicates or losses");
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            declared,
            "set iterates sorted"
        );
        let retry: std::collections::BTreeSet<_> = [
            ErrorKind::Backpressure,
            ErrorKind::Timeout,
            ErrorKind::Disconnected,
        ]
        .into_iter()
        .collect();
        assert_eq!(
            retry.into_iter().collect::<Vec<_>>(),
            [
                ErrorKind::Timeout,
                ErrorKind::Disconnected,
                ErrorKind::Backpressure
            ],
            "the default retry set is stable"
        );
    }

    #[test]
    fn error_preserves_message_context_and_original_source() {
        let diagnostics = Diagnostics {
            operation: Some(Operation::Read),
            connection_state: Some(ConnectionState::Connected),
            latency: Some(Duration::from_millis(10)),
            retries: 2,
            category: Some(FailureCategory::Transport),
        };
        let error = PlcError::new(ErrorKind::Timeout, "read deadline elapsed")
            .with_source(std::io::Error::from(std::io::ErrorKind::TimedOut))
            .with_diagnostics(diagnostics.clone());
        assert_eq!(error.kind(), ErrorKind::Timeout, "classification survives");
        assert_eq!(
            error.to_string(),
            "read deadline elapsed",
            "message survives"
        );
        assert_eq!(error.diagnostics(), &diagnostics, "diagnostics survive");
        let source = error.source().expect("source is retained");
        assert!(
            source.downcast_ref::<std::io::Error>().is_some(),
            "source retains concrete type"
        );
        assert!(
            format!("{error:?}").contains("Timeout"),
            "debug identifies the error kind"
        );
    }

    #[test]
    fn errors_without_sources_have_no_source_and_no_fabricated_context() {
        let error = PlcError::new(ErrorKind::InvalidConfig, "invalid configuration");
        assert!(error.source().is_none(), "no fabricated source");
        assert_eq!(
            error.diagnostics(),
            &Diagnostics::default(),
            "no fabricated context"
        );
        assert!(
            size_of::<PlcError>() <= 2 * size_of::<usize>(),
            "error remains small"
        );
    }

    #[test]
    fn only_declared_transient_errors_are_retry_candidates() {
        for kind in [
            ErrorKind::Timeout,
            ErrorKind::Disconnected,
            ErrorKind::Backpressure,
        ] {
            assert!(
                PlcError::new(kind, "transient failure").is_retryable(),
                "{kind:?} is eligible for read policy"
            );
        }
        for kind in [
            ErrorKind::InvalidConfig,
            ErrorKind::InvalidAddress,
            ErrorKind::Unsupported(Operation::Write),
            ErrorKind::Cancelled,
            ErrorKind::Authentication,
            ErrorKind::Trust,
            ErrorKind::MalformedData,
            ErrorKind::DeviceException,
            ErrorKind::ResourceLimit,
            ErrorKind::Internal,
        ] {
            assert!(
                !PlcError::new(kind, "permanent failure").is_retryable(),
                "{kind:?} is not automatically retryable"
            );
        }
    }
}
