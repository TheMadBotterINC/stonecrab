//! The one place a deadline meets Tokio's timer (coding standards section 8.5).
//!
//! Every transport I/O call goes through [`within`], which races the I/O future against
//! `tokio::time::timeout_at(deadline)`. The deadline is always the `Deadline` argument of
//! the public method; no method falls back to a default. Nothing here spawns a task.

use std::future::Future;
use std::io;
use std::time::Instant;

use crate::{Deadline, Diagnostics, ErrorKind, FailureCategory, Operation, PlcError, Result};

/// Which transport call an error belongs to: a short name for the message and the
/// `Diagnostics::operation`, when the core `Operation` set has a matching entry.
#[derive(Debug, Clone, Copy)]
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) struct Site {
    name: &'static str,
    operation: Option<Operation>,
}

impl Site {
    /// Opening the connection.
    pub(crate) const CONNECT: Self = Self {
        name: "tcp connect",
        operation: Some(Operation::Connect),
    };
    /// Sending one frame; the core `Operation` set has no per-frame entry.
    pub(crate) const SEND: Self = Self {
        name: "tcp send",
        operation: None,
    };
    /// Receiving bytes; the core `Operation` set has no per-read entry.
    pub(crate) const RECV: Self = Self {
        name: "tcp receive",
        operation: None,
    };
    /// Closing the connection.
    pub(crate) const SHUTDOWN: Self = Self {
        name: "tcp shutdown",
        operation: Some(Operation::Close),
    };

    /// A failure of this site's kind, with transport diagnostics attached.
    pub(crate) fn error(self, kind: ErrorKind, detail: &str) -> PlcError {
        let diagnostics = Diagnostics {
            operation: self.operation,
            category: Some(FailureCategory::Transport),
            ..Diagnostics::default()
        };
        PlcError::new(kind, format!("{}: {detail}", self.name)).with_diagnostics(diagnostics)
    }

    /// Map an operating-system failure to a typed error that keeps it as the source.
    ///
    /// `TimedOut` while connecting is a connect timeout; on an established connection it
    /// means the operating system gave up retransmitting, which is a lost connection.
    /// Refused, reset, aborted, broken-pipe, unreachable and every other failure is
    /// `Disconnected`: the transport cannot be used and a reconnect decides what happens
    /// next.
    pub(crate) fn io_error(self, error: io::Error) -> PlcError {
        let connecting = self.operation == Some(Operation::Connect);
        let kind = if connecting && error.kind() == io::ErrorKind::TimedOut {
            ErrorKind::Timeout
        } else {
            ErrorKind::Disconnected
        };
        let detail = error.to_string();
        self.error(kind, &detail).with_source(error)
    }
}

/// The serial transport's sites, in their own block so the feature gate sits on one item.
#[cfg(feature = "serial")]
impl Site {
    /// Opening a serial port.
    pub(crate) const SERIAL_OPEN: Self = Self {
        name: "serial open",
        operation: Some(Operation::Connect),
    };
    /// Sending one frame to a serial port.
    pub(crate) const SERIAL_SEND: Self = Self {
        name: "serial send",
        operation: None,
    };
    /// Receiving bytes from a serial port.
    pub(crate) const SERIAL_RECV: Self = Self {
        name: "serial receive",
        operation: None,
    };
    /// Closing a serial port.
    pub(crate) const SERIAL_SHUTDOWN: Self = Self {
        name: "serial shutdown",
        operation: Some(Operation::Close),
    };
}

/// The UDP transport's sites, so that a failure names the datagram transport.
impl Site {
    /// Opening a UDP socket and connecting it to its peer.
    pub(crate) const UDP_CONNECT: Self = Self {
        name: "udp connect",
        operation: Some(Operation::Connect),
    };
    /// Sending one datagram.
    pub(crate) const UDP_SEND: Self = Self {
        name: "udp send",
        operation: None,
    };
    /// Receiving one datagram.
    pub(crate) const UDP_RECV: Self = Self {
        name: "udp receive",
        operation: None,
    };
    /// Closing a UDP socket.
    pub(crate) const UDP_SHUTDOWN: Self = Self {
        name: "udp shutdown",
        operation: Some(Operation::Close),
    };
}

/// The monotonic reading deadlines are judged against: Tokio's clock, which is the plain
/// `std::time::Instant::now()` unless a test has paused time.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn now() -> Instant {
    tokio::time::Instant::now().into_std()
}

/// Fail with `Timeout` if `deadline` has already elapsed, before any I/O is attempted.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn ensure_before(deadline: Deadline, site: Site) -> Result<()> {
    if deadline.is_elapsed(now()) {
        Err(site.error(ErrorKind::Timeout, "deadline already elapsed"))
    } else {
        Ok(())
    }
}

/// Run `io` to completion or until `deadline`, whichever comes first.
///
/// An elapsed deadline fails with `Timeout` before `io` is polled; an I/O failure is
/// mapped with [`Site::io_error`]; a deadline that expires while `io` is pending drops
/// `io` and fails with `Timeout`. Dropping the future returned here drops `io`, so its
/// cancel safety is the cancel safety of `io`.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) async fn within<T>(
    deadline: Deadline,
    site: Site,
    io: impl Future<Output = io::Result<T>>,
) -> Result<T> {
    ensure_before(deadline, site)?;
    let limit = tokio::time::Instant::from_std(deadline.instant());
    match tokio::time::timeout_at(limit, io).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(site.io_error(error)),
        Err(_elapsed) => Err(site.error(ErrorKind::Timeout, "deadline elapsed")),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn after(duration: Duration) -> Deadline {
        Deadline::after(now(), duration).expect("representable")
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (deadline expiry)
    #[tokio::test(start_paused = true)]
    async fn a_pending_io_future_fails_with_timeout_when_the_deadline_passes() {
        let started = tokio::time::Instant::now();
        let error = within(
            after(Duration::from_secs(3)),
            Site::RECV,
            std::future::pending::<io::Result<()>>(),
        )
        .await
        .expect_err("never completes");
        assert_eq!(error.kind(), ErrorKind::Timeout, "deadline expiry");
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(3),
            "paused time advanced exactly to the deadline"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn an_elapsed_deadline_fails_even_if_the_io_future_is_ready() {
        // `timeout_at` alone would let a ready future win, so this proves the pre-check.
        let error = within(Deadline::at(now()), Site::SEND, std::future::ready(Ok(())))
            .await
            .expect_err("already elapsed");
        assert_eq!(error.kind(), ErrorKind::Timeout, "elapsed deadline");
    }

    #[tokio::test(start_paused = true)]
    async fn a_completed_future_returns_its_value_and_a_failed_one_maps_the_error() {
        let value = within(after(Duration::from_secs(1)), Site::SEND, async {
            Ok(7_u8)
        })
        .await
        .expect("completes");
        assert_eq!(value, 7, "value passes through");
        let error = within(after(Duration::from_secs(1)), Site::SEND, async {
            Err::<(), _>(io::Error::from(io::ErrorKind::BrokenPipe))
        })
        .await
        .expect_err("io failure");
        assert_eq!(error.kind(), ErrorKind::Disconnected, "broken pipe");
    }

    #[test]
    fn io_failures_map_to_disconnected_except_a_connect_timeout() {
        let cases = [
            (
                Site::CONNECT,
                io::ErrorKind::ConnectionRefused,
                ErrorKind::Disconnected,
            ),
            (Site::CONNECT, io::ErrorKind::TimedOut, ErrorKind::Timeout),
            (
                Site::RECV,
                io::ErrorKind::ConnectionReset,
                ErrorKind::Disconnected,
            ),
            (Site::RECV, io::ErrorKind::TimedOut, ErrorKind::Disconnected),
            (
                Site::SEND,
                io::ErrorKind::ConnectionAborted,
                ErrorKind::Disconnected,
            ),
            (
                Site::SHUTDOWN,
                io::ErrorKind::NotConnected,
                ErrorKind::Disconnected,
            ),
            (
                Site::CONNECT,
                io::ErrorKind::HostUnreachable,
                ErrorKind::Disconnected,
            ),
        ];
        for (site, io_kind, want) in cases {
            let error = site.io_error(io::Error::from(io_kind));
            assert_eq!(error.kind(), want, "{} with {io_kind:?}", site.name);
        }
    }

    #[test]
    fn errors_carry_transport_diagnostics_and_the_io_source() {
        let connect = Site::CONNECT.io_error(io::Error::from(io::ErrorKind::ConnectionRefused));
        let diagnostics = connect.diagnostics();
        assert_eq!(
            diagnostics.operation,
            Some(Operation::Connect),
            "connect operation"
        );
        assert_eq!(
            diagnostics.category,
            Some(FailureCategory::Transport),
            "category"
        );
        let source = std::error::Error::source(&connect).expect("io source kept");
        assert!(
            source.downcast_ref::<io::Error>().is_some(),
            "source is the io error"
        );
        let close = Site::SHUTDOWN.error(ErrorKind::Timeout, "x");
        assert_eq!(
            close.diagnostics().operation,
            Some(Operation::Close),
            "close operation"
        );
        let send = Site::SEND.error(ErrorKind::Timeout, "x");
        assert_eq!(send.diagnostics().operation, None, "no invented operation");
        assert_eq!(
            Site::RECV.error(ErrorKind::Timeout, "late").to_string(),
            "tcp receive: late",
            "message shape"
        );
    }
}
