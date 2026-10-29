//! The silence before a request: a transport that watches the line, and drops what it
//! hears, before it lets a frame out.

use std::sync::Arc;

use crate::transport::{Transport, TransportFactory};
use crate::{
    Clock, Deadline, Diagnostics, Endpoint, ErrorKind, FailureCategory, OpFuture, PlcError,
    ResourceLimits, Result,
};
use bytes::{Bytes, BytesMut};

use crate::modbus::rtu::line::Silence;

/// Opens transports with `inner` and paces each one by `silence` on `clock`.
pub(super) struct PacedFactory {
    pub(super) inner: Arc<dyn TransportFactory>,
    pub(super) clock: Arc<dyn Clock>,
    pub(super) silence: Silence,
}

impl TransportFactory for PacedFactory {
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let inner = self.inner.open(endpoint, limits, deadline).await?;
            let paced: Box<dyn Transport> = Box::new(Paced {
                inner,
                clock: Arc::clone(&self.clock),
                silence: self.silence,
                answered: true,
            });
            Ok(paced)
        })
    }
}

/// A transport that, before every send, watches the line until it has been quiet for the
/// gap (the drain after a send that nothing answered) and drops what arrives meanwhile:
/// a master on a serial line ignores what it receives while idle, and must not talk over
/// a frame. Upstream's pacer only spaces writes (`WritePacer`); watching is Stonecrab's.
struct Paced {
    inner: Box<dyn Transport>,
    clock: Arc<dyn Clock>,
    silence: Silence,
    /// Whether a byte arrived since the last send (true before the first).
    answered: bool,
}

impl Paced {
    /// Watch the line until it has been quiet for the gap, or the drain when the last send
    /// was not answered, dropping every byte that arrives; a byte starts the watch again.
    /// The wait ends at `deadline` with `Timeout`.
    async fn settle(&mut self, deadline: Deadline) -> Result<()> {
        let mut quiet = if self.answered {
            self.silence.gap
        } else {
            self.silence.drain
        };
        let mut heard = BytesMut::new();
        while !quiet.is_zero() {
            let quiet_at = self
                .clock
                .now()
                .checked_add(quiet)
                .unwrap_or(deadline.instant());
            heard.clear();
            let watch = Deadline::at(quiet_at.min(deadline.instant()));
            match self.inner.recv(&mut heard, watch).await {
                Ok(0) => return Err(closed()),
                Ok(dropped) => {
                    tracing::debug!(dropped, "dropped bytes that arrived before a request");
                    self.answered = true;
                    quiet = self.silence.gap;
                }
                Err(error)
                    if error.kind() == ErrorKind::Timeout && self.clock.now() >= quiet_at =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

/// The peer closed while the line was being watched.
fn closed() -> PlcError {
    let diagnostics = Diagnostics {
        category: Some(FailureCategory::Transport),
        ..Diagnostics::default()
    };
    PlcError::new(
        ErrorKind::Disconnected,
        "the peer closed the connection while the line was watched before a request",
    )
    .with_diagnostics(diagnostics)
}

impl Transport for Paced {
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        Box::pin(async move {
            self.settle(deadline).await?;
            self.answered = false;
            self.inner.send(bytes, deadline).await
        })
    }

    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        Box::pin(async move {
            let read = self.inner.recv(buffer, deadline).await?;
            self.answered |= read > 0;
            Ok(read)
        })
    }

    fn shutdown(&mut self, deadline: Deadline) -> OpFuture<'_, ()> {
        self.inner.shutdown(deadline)
    }

    fn peer(&self) -> &Endpoint {
        self.inner.peer()
    }

    fn limits(&self) -> &ResourceLimits {
        self.inner.limits()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests {
    use std::time::Duration;

    use crate::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};

    use super::*;

    #[test]
    fn a_paced_transport_reports_the_peer_and_limits_of_the_one_it_wraps() {
        let clock = FakeClock::default();
        let inner = FakeTransportFactory::new(clock.clone());
        inner.push_script(Script::default());
        let factory = PacedFactory {
            inner: Arc::new(inner),
            clock: Arc::new(clock.clone()),
            silence: Silence {
                gap: Duration::ZERO,
                drain: Duration::ZERO,
            },
        };
        let endpoint = Endpoint::Custom("/dev/ttyS0".to_owned());
        let limits = ResourceLimits::builder().build().expect("limits");
        let deadline = Deadline::after(clock.now(), Duration::from_secs(1)).expect("deadline");
        let transport =
            block_on_with(&clock, factory.open(&endpoint, limits, deadline)).expect("open");
        assert_eq!(transport.peer(), &endpoint);
        assert_eq!(transport.limits(), &limits);
    }
}
