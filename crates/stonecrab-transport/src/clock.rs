use std::time::{Instant, SystemTime};

use stonecrab_core::{Clock, OpFuture};

/// The real clock for production connections, backed by the Tokio timer.
///
/// `now()` is `tokio::time::Instant::now().into_std()`, so a test that pauses Tokio time
/// (`tokio::time::pause`) moves this clock and every [`Deadline`](stonecrab_core::Deadline)
/// built from it together. Outside paused time that value is the ordinary
/// `std::time::Instant::now()`. `wall_now()` is `SystemTime::now()`, for receipt
/// timestamps only (ADR-0007). `sleep_until` is `tokio::time::sleep_until`.
///
/// `SystemClock` lives in this crate rather than in core because core has no timer
/// (ADR-0007, decision 3). It must be polled inside a Tokio runtime with the time driver
/// enabled; `sleep_until` panics inside Tokio, not here, if there is none. Sockets in
/// [`TcpTransport`](crate::TcpTransport) compare deadlines against the same Tokio clock.
///
/// # Examples
/// ```
/// use stonecrab_core::Clock;
/// use stonecrab_transport::SystemClock;
///
/// let clock = SystemClock::default();
/// let before = clock.now();
/// assert!(clock.now() >= before);
/// assert!(clock.wall_now() > std::time::SystemTime::UNIX_EPOCH);
/// ```
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        tokio::time::Instant::now().into_std()
    }

    fn wall_now(&self) -> SystemTime {
        SystemTime::now()
    }

    /// Resolves once Tokio's clock reaches `deadline`; a `deadline` at or before `now()`
    /// resolves on the first poll.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future cancels the Tokio timer entry and has no side
    /// effects; nothing was sent to a device. The deadline source is the `deadline`
    /// argument.
    ///
    /// # Wire effects
    /// None. Sleeping touches no transport.
    fn sleep_until(&self, deadline: Instant) -> OpFuture<'_, ()> {
        Box::pin(async move {
            tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
            Ok(())
        })
    }
}
