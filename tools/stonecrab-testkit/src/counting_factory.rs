use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use bytes::{Bytes, BytesMut};
use stonecrab_core::{Deadline, Endpoint, OpFuture, ResourceLimits};
use stonecrab_transport::{Transport, TransportFactory};

/// What the factory and every transport it handed out have seen so far.
#[derive(Debug, Default)]
struct Counts {
    opens: AtomicU64,
    frames: AtomicU64,
    bytes: AtomicU64,
}

/// A [`TransportFactory`] that wraps another factory and counts what passes through the
/// transports it hands out.
///
/// The contract suite's `io_probe` must not move when the suite says "no I/O". Against a
/// device whose socket the test does not own (a real server in a container), the only
/// place to observe that is the transport boundary: this factory counts every
/// [`open`](TransportFactory::open) call and, for every transport it returned, every
/// [`send`](Transport::send) call and the bytes handed to it. [`io`](Self::io) adds the
/// opens and the frames into the one number a probe wants.
///
/// A call is counted when it is made, whether or not it succeeds, so a probe that does not
/// move proves that the driver never reached the transport at all. Bytes are counted from
/// the length of the frame given to `send`. `recv` and `shutdown` are delegated and not
/// counted: they transmit no application bytes. Clones share one set of counters, so a
/// test keeps a clone while the driver owns an `Arc<dyn TransportFactory>`.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// A Stonecrab test double with no upstream counterpart; the wrapped factory's behavior,
/// errors and endpoint handling are passed through unchanged.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use bytes::Bytes;
/// use stonecrab_core::{Clock, Deadline, Endpoint, ResourceLimits};
/// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory, Script, block_on_with};
/// use stonecrab_transport::TransportFactory;
///
/// let clock = FakeClock::default();
/// let inner = FakeTransportFactory::new(clock.clone());
/// inner.push_script(Script::default());
/// let factory = CountingFactory::new(Arc::new(inner));
///
/// let endpoint = Endpoint::Custom("fake://plc".to_owned());
/// let limits = ResourceLimits::builder().build()?;
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
/// let mut transport = block_on_with(&clock, factory.open(&endpoint, limits, deadline))?;
/// block_on_with(&clock, transport.send(Bytes::from_static(b"frame"), deadline))?;
///
/// assert_eq!((factory.opens(), factory.frames(), factory.bytes()), (1, 1, 5));
/// assert_eq!(factory.io(), 2, "one open and one frame");
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Clone)]
pub struct CountingFactory {
    inner: Arc<dyn TransportFactory>,
    counts: Arc<Counts>,
}

impl fmt::Debug for CountingFactory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CountingFactory")
            .field("opens", &self.opens())
            .field("frames", &self.frames())
            .field("bytes", &self.bytes())
            .finish_non_exhaustive()
    }
}

impl CountingFactory {
    /// A factory that opens transports through `inner` and counts what passes.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory};
    ///
    /// let factory = CountingFactory::new(Arc::new(FakeTransportFactory::new(FakeClock::default())));
    /// assert_eq!(factory.io(), 0);
    /// ```
    #[must_use]
    pub fn new(inner: Arc<dyn TransportFactory>) -> Self {
        Self {
            inner,
            counts: Arc::new(Counts::default()),
        }
    }

    /// How many times `open` was called, failed calls included.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory};
    ///
    /// let factory = CountingFactory::new(Arc::new(FakeTransportFactory::new(FakeClock::default())));
    /// assert_eq!(factory.opens(), 0);
    /// ```
    #[must_use]
    pub fn opens(&self) -> u64 {
        self.counts.opens.load(Ordering::Relaxed)
    }

    /// How many times `send` was called on a transport this factory returned.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory};
    ///
    /// let factory = CountingFactory::new(Arc::new(FakeTransportFactory::new(FakeClock::default())));
    /// assert_eq!(factory.frames(), 0);
    /// ```
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.counts.frames.load(Ordering::Relaxed)
    }

    /// The total length of the frames given to `send`.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory};
    ///
    /// let factory = CountingFactory::new(Arc::new(FakeTransportFactory::new(FakeClock::default())));
    /// assert_eq!(factory.bytes(), 0);
    /// ```
    #[must_use]
    pub fn bytes(&self) -> u64 {
        self.counts.bytes.load(Ordering::Relaxed)
    }

    /// Opens plus frames: the one counter a contract fixture's `io_probe` wants.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_testkit::{CountingFactory, FakeClock, FakeTransportFactory};
    ///
    /// let factory = CountingFactory::new(Arc::new(FakeTransportFactory::new(FakeClock::default())));
    /// assert_eq!(factory.io(), 0);
    /// ```
    #[must_use]
    pub fn io(&self) -> u64 {
        self.opens().saturating_add(self.frames())
    }
}

impl TransportFactory for CountingFactory {
    /// Counts the call, then opens through the wrapped factory and wraps the transport.
    ///
    /// Errors, the deadline and the endpoint are the wrapped factory's; the returned
    /// transport reports the same `peer` and `limits` as the one it wraps.
    ///
    /// # Cancel safety
    /// As the wrapped factory's `open`; the call stays counted when the future is dropped.
    ///
    /// # Wire effects
    /// As the wrapped factory's `open`. Counting transmits nothing.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        self.counts.opens.fetch_add(1, Ordering::Relaxed);
        Box::pin(async move {
            let inner = self.inner.open(endpoint, limits, deadline).await?;
            let counted: Box<dyn Transport> = Box::new(CountingTransport {
                inner,
                counts: Arc::clone(&self.counts),
            });
            Ok(counted)
        })
    }
}

/// A transport that counts the frames given to `send` and delegates everything.
struct CountingTransport {
    inner: Box<dyn Transport>,
    counts: Arc<Counts>,
}

impl Transport for CountingTransport {
    fn send(&mut self, bytes: Bytes, deadline: Deadline) -> OpFuture<'_, ()> {
        self.counts.frames.fetch_add(1, Ordering::Relaxed);
        let length = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        self.counts.bytes.fetch_add(length, Ordering::Relaxed);
        self.inner.send(bytes, deadline)
    }

    fn recv<'a>(&'a mut self, buffer: &'a mut BytesMut, deadline: Deadline) -> OpFuture<'a, usize> {
        self.inner.recv(buffer, deadline)
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
mod tests {
    use std::time::Duration;

    use stonecrab_core::{Clock, ErrorKind};

    use super::*;
    use crate::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};

    fn endpoint() -> Endpoint {
        Endpoint::Custom("fake://plc".to_owned())
    }

    fn limits() -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(8)
            .build()
            .expect("limits")
    }

    fn deadline(clock: &FakeClock) -> Deadline {
        Deadline::after(clock.now(), Duration::from_secs(5)).expect("representable")
    }

    fn bench() -> (FakeClock, FakeTransportFactory, CountingFactory) {
        let clock = FakeClock::default();
        let inner = FakeTransportFactory::new(clock.clone());
        let counting = CountingFactory::new(Arc::new(inner.clone()));
        (clock, inner, counting)
    }

    #[test]
    fn the_factory_is_send_sync_and_usable_as_a_trait_object() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<CountingFactory>();
        let (_, _, counting) = bench();
        let shared: Arc<dyn TransportFactory> = Arc::new(counting);
        assert_eq!(Arc::strong_count(&shared), 1, "constructed");
    }

    #[test]
    fn opens_frames_and_bytes_are_counted_across_every_transport() {
        let (clock, inner, factory) = bench();
        inner.push_script(Script::new(vec![Step::Reply(b"ok".to_vec())]));
        inner.push_script(Script::default());
        let target = endpoint();
        let mut first = block_on_with(&clock, factory.open(&target, limits(), deadline(&clock)))
            .expect("first open");
        let mut second = block_on_with(&clock, factory.open(&target, limits(), deadline(&clock)))
            .expect("second open");
        assert_eq!(
            (factory.opens(), factory.io()),
            (2, 2),
            "two opens, no frame"
        );

        block_on_with(
            &clock,
            first.send(Bytes::from_static(b"abc"), deadline(&clock)),
        )
        .expect("send");
        block_on_with(
            &clock,
            first.send(Bytes::from_static(b"de"), deadline(&clock)),
        )
        .expect("send");
        block_on_with(
            &clock,
            second.send(Bytes::from_static(b"f"), deadline(&clock)),
        )
        .expect("send");
        assert_eq!(factory.frames(), 3, "three send calls on two transports");
        assert_eq!(factory.bytes(), 6, "3 + 2 + 1 bytes");
        assert_eq!(factory.io(), 5, "opens plus frames");
    }

    #[test]
    fn receiving_and_shutting_down_are_delegated_and_not_counted() {
        let (clock, inner, factory) = bench();
        inner.push_script(Script::new(vec![Step::Reply(b"ok".to_vec())]));
        let target = endpoint();
        let mut transport =
            block_on_with(&clock, factory.open(&target, limits(), deadline(&clock))).expect("open");
        let mut buffer = BytesMut::new();
        let read =
            block_on_with(&clock, transport.recv(&mut buffer, deadline(&clock))).expect("recv");
        assert_eq!(
            (read, &buffer[..]),
            (2, &b"ok"[..]),
            "the reply came through"
        );
        block_on_with(&clock, transport.shutdown(deadline(&clock))).expect("shutdown");
        assert_eq!(factory.io(), 1, "only the open counts");
        assert_eq!(factory.bytes(), 0, "nothing was sent");
    }

    #[test]
    fn the_wrapper_reports_the_peer_and_limits_of_the_transport_it_wraps() {
        let (clock, inner, factory) = bench();
        inner.push_script(Script::default());
        let target = endpoint();
        let transport =
            block_on_with(&clock, factory.open(&target, limits(), deadline(&clock))).expect("open");
        assert_eq!(transport.peer(), &target, "peer");
        assert_eq!(transport.limits(), &limits(), "limits");
    }

    #[test]
    fn a_failed_open_and_a_refused_send_are_still_counted() {
        let (clock, inner, factory) = bench();
        let target = endpoint();
        let error = block_on_with(&clock, factory.open(&target, limits(), deadline(&clock)))
            .err()
            .expect("nothing is queued");
        assert_eq!(
            error.kind(),
            ErrorKind::Internal,
            "the wrapped factory's error"
        );
        assert_eq!(factory.opens(), 1, "the failed call is counted");

        inner.push_script(Script::default());
        let mut transport =
            block_on_with(&clock, factory.open(&target, limits(), deadline(&clock))).expect("open");
        let oversized = Bytes::from_static(b"123456789");
        let refused = block_on_with(&clock, transport.send(oversized, deadline(&clock)))
            .expect_err("longer than max_frame_bytes");
        assert_eq!(refused.kind(), ErrorKind::ResourceLimit, "the frame limit");
        assert_eq!(factory.frames(), 1, "the attempt is counted");
        assert_eq!(factory.bytes(), 9, "with its length");
    }

    #[test]
    fn clones_share_the_counters_and_debug_shows_them() {
        let (clock, inner, factory) = bench();
        inner.push_script(Script::default());
        let observer = factory.clone();
        let target = endpoint();
        drop(
            block_on_with(&clock, factory.open(&target, limits(), deadline(&clock))).expect("open"),
        );
        assert_eq!(observer.opens(), 1, "the clone sees the open");
        assert!(
            format!("{observer:?}").contains("opens: 1"),
            "debug shows the counters"
        );
    }
}
