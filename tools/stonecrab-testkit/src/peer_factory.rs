use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use stonecrab_core::{Deadline, Endpoint, ErrorKind, OpFuture, PlcError, ResourceLimits};
use stonecrab_transport::{TcpFactory, Transport, TransportFactory};

use crate::{Script, ScriptReport, ScriptedPeer};

/// Builds the script of the next peer.
type ScriptBuilder = Box<dyn Fn() -> Script + Send + Sync>;

/// A [`TransportFactory`] that starts a fresh [`ScriptedPeer`] for every `open` and
/// connects to it with the real [`TcpFactory`].
///
/// A `ScriptedPeer` accepts one connection and refuses the next, while a driver connects
/// once per operation sequence and reconnects after a lost link. This factory gives each
/// `open` its own peer, running a script built by the closure passed to
/// [`new`](Self::new), so a driver test can keep a real clock and real loopback sockets
/// across reconnects. The endpoint passed to `open` is ignored: the connection goes to the
/// new peer's loopback address. No byte passes through the factory, so the driver sees the
/// same `TcpTransport` it sees in production.
///
/// Peers live until [`finish`](Self::finish) collects their reports (or the factory is
/// dropped, which aborts them).
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// A Stonecrab test double with no upstream counterpart; connect failures are the
/// `TcpFactory`'s own.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab_core::{Clock, Deadline, Endpoint, ResourceLimits};
/// use stonecrab_testkit::{Script, ScriptedPeerFactory};
/// use stonecrab_transport::{SystemClock, TransportFactory};
///
/// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
/// runtime.block_on(async {
///     let factory = ScriptedPeerFactory::new(Script::default);
///     let endpoint = Endpoint::Custom("ignored://host".to_owned());
///     let limits = ResourceLimits::builder().build()?;
///     let deadline = Deadline::after(SystemClock::default().now(), Duration::from_secs(5))?;
///     let transport = factory.open(&endpoint, limits, deadline).await?;
///     assert_eq!(factory.opens(), 1);
///     drop(transport);
///     assert_eq!(factory.finish().await?.len(), 1);
///     Ok::<(), Box<dyn std::error::Error>>(())
/// })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct ScriptedPeerFactory {
    script: ScriptBuilder,
    tcp: TcpFactory,
    peers: Mutex<Vec<ScriptedPeer>>,
    opens: AtomicU64,
}

impl fmt::Debug for ScriptedPeerFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptedPeerFactory")
            .field("opens", &self.opens())
            .finish_non_exhaustive()
    }
}

impl ScriptedPeerFactory {
    /// A factory whose peers each run a script built by `script`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeerFactory};
    ///
    /// let factory = ScriptedPeerFactory::new(Script::default);
    /// assert_eq!(factory.opens(), 0);
    /// ```
    pub fn new(script: impl Fn() -> Script + Send + Sync + 'static) -> Self {
        Self {
            script: Box::new(script),
            tcp: TcpFactory::default(),
            peers: Mutex::new(Vec::new()),
            opens: AtomicU64::new(0),
        }
    }

    /// How many peers `open` has started.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeerFactory};
    ///
    /// assert_eq!(ScriptedPeerFactory::new(Script::default).opens(), 0);
    /// ```
    #[must_use]
    pub fn opens(&self) -> u64 {
        self.opens.load(Ordering::Relaxed)
    }

    /// Stop every peer started so far and return their reports, in start order.
    ///
    /// # Errors
    /// `Internal` if a peer task panicked.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_testkit::{Script, ScriptedPeerFactory};
    ///
    /// let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    /// runtime.block_on(async {
    ///     let factory = ScriptedPeerFactory::new(Script::default);
    ///     assert!(factory.finish().await?.is_empty(), "no open, no peer");
    ///     Ok::<(), Box<dyn std::error::Error>>(())
    /// })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub async fn finish(&self) -> Result<Vec<ScriptReport>, PlcError> {
        let peers = std::mem::take(&mut *self.peers.lock().unwrap_or_else(PoisonError::into_inner));
        let mut reports = Vec::with_capacity(peers.len());
        for peer in peers {
            let report = peer.close().await.map_err(|error| {
                PlcError::new(ErrorKind::Internal, "a loopback peer task failed").with_source(error)
            })?;
            reports.push(report);
        }
        Ok(reports)
    }
}

impl TransportFactory for ScriptedPeerFactory {
    /// Starts a peer running a freshly built script and connects to it, within `deadline`.
    ///
    /// The `endpoint` is ignored. Fails with `Internal` if the peer cannot start (no Tokio
    /// runtime, or the loopback socket cannot be bound); connect failures are the
    /// `TcpFactory`'s.
    ///
    /// # Cancel safety
    /// Cancel-safe: a dropped call leaves at most a started peer, which `finish` or the
    /// drop of the factory stops.
    ///
    /// # Wire effects
    /// Opens one loopback connection to the new peer; nothing leaves the machine.
    fn open<'a>(
        &'a self,
        _endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let peer = ScriptedPeer::spawn((self.script)()).map_err(|error| {
                PlcError::new(ErrorKind::Internal, "the loopback peer did not start")
                    .with_source(error)
            })?;
            let endpoint = peer.endpoint();
            self.peers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(peer);
            self.opens.fetch_add(1, Ordering::Relaxed);
            self.tcp.open(&endpoint, limits, deadline).await
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stonecrab_core::{Clock, Deadline, Endpoint, ErrorKind, ResourceLimits};
    use stonecrab_transport::{SystemClock, TransportFactory};

    use super::*;
    use crate::{FakeClock, Step, block_on_with};

    fn deadline() -> Deadline {
        Deadline::after(SystemClock::default().now(), Duration::from_secs(5)).expect("deadline")
    }

    fn limits() -> ResourceLimits {
        ResourceLimits::builder().build().expect("limits")
    }

    #[tokio::test]
    async fn every_open_gets_its_own_peer_and_finish_reports_them_in_order() {
        let factory = ScriptedPeerFactory::new(|| Script::new(vec![Step::Reply(b"hi".to_vec())]));
        let endpoint = Endpoint::Custom("ignored://host".to_owned());
        let first = factory.open(&endpoint, limits(), deadline()).await;
        let second = factory.open(&endpoint, limits(), deadline()).await;
        assert!(first.is_ok() && second.is_ok(), "both opens connect");
        assert_eq!(factory.opens(), 2, "one peer per open");
        assert!(
            format!("{factory:?}").contains("opens: 2"),
            "debug shows opens"
        );
        let reports = factory.finish().await.expect("finish");
        assert_eq!(reports.len(), 2, "one report per peer");
        assert!(factory.finish().await.expect("again").is_empty(), "drained");
    }

    #[test]
    fn an_open_outside_a_runtime_fails_with_internal_and_starts_no_peer() {
        let factory = ScriptedPeerFactory::new(Script::default);
        let endpoint = Endpoint::Custom("ignored://host".to_owned());
        let clock = FakeClock::default();
        let error = block_on_with(&clock, factory.open(&endpoint, limits(), deadline()))
            .err()
            .expect("no runtime, no peer");
        assert_eq!(error.kind(), ErrorKind::Internal, "kind");
        assert_eq!(factory.opens(), 0, "nothing started");
    }
}
