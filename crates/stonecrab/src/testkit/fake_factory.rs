use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use crate::transport::{Transport, TransportFactory};
use crate::{Clock, Deadline, Endpoint, ErrorKind, OpFuture, Operation, ResourceLimits};

use crate::testkit::fake_transport::failure;
use crate::testkit::{FakeClock, FakeTransport, FakeTransportProbe, Script};

const SITE: &str = "fake connect";

/// What the next `open` does.
#[derive(Debug)]
enum Plan {
    Script(Script),
    Park,
    Refuse,
}

/// One recorded call of [`FakeTransportFactory::open`].
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline, Endpoint, ResourceLimits};
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
/// use stonecrab::transport::TransportFactory;
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::default());
/// let endpoint = Endpoint::Custom("fake://plc".to_owned());
/// let limits = ResourceLimits::builder().build()?;
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
/// block_on_with(&clock, factory.open(&endpoint, limits, deadline))?;
///
/// let call = factory.opens().remove(0);
/// assert_eq!((call.endpoint, call.limits, call.deadline), (endpoint, limits, deadline));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCall {
    /// The endpoint `open` was asked for.
    pub endpoint: Endpoint,
    /// The limits `open` was given.
    pub limits: ResourceLimits,
    /// The connect deadline `open` was given.
    pub deadline: Deadline,
}

#[derive(Debug, Default)]
struct State {
    plans: VecDeque<Plan>,
    opens: Vec<OpenCall>,
    probes: Vec<FakeTransportProbe>,
}

/// A [`TransportFactory`] that hands out [`FakeTransport`]s from a queue of scripts.
///
/// Queue what each `open` should do, in order: [`push_script`](Self::push_script) opens
/// a transport running that script, [`push_connect_timeout`](Self::push_connect_timeout)
/// parks the call until the [`FakeClock`] passes the connect deadline and then fails with
/// `Timeout`, and [`push_refusal`](Self::push_refusal) fails at once with
/// `Disconnected`. An `open` with nothing queued fails with `Internal` so a test that
/// opens more often than it planned fails loudly. Every call is recorded (see
/// [`opens`](Self::opens)) and so is every transport handed out
/// ([`probes`](Self::probes)), whether the test kept the `Box<dyn Transport>` or not.
///
/// The factory accepts any `Endpoint`; opened transports report the endpoint and limits
/// they were opened with. Clones share one queue and one record, so a test keeps a clone
/// while the code under test owns an `Arc<dyn TransportFactory>`.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-TCP-CONNECT
/// A Stonecrab test double with no upstream counterpart; failures carry
/// `Diagnostics.operation == Some(Operation::Connect)` like `TcpFactory`'s.
///
/// # Examples
/// The connect-timeout path, driven without waiting:
/// ```
/// use std::time::Duration;
/// use stonecrab::{Clock, Deadline, Endpoint, ErrorKind, Operation, ResourceLimits};
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, block_on_with};
/// use stonecrab::transport::TransportFactory;
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_connect_timeout();
/// let endpoint = Endpoint::Custom("fake://plc".to_owned());
/// let limits = ResourceLimits::builder().build()?;
/// let deadline = Deadline::after(clock.now(), Duration::from_secs(5))?;
///
/// let error = block_on_with(&clock, factory.open(&endpoint, limits, deadline))
///     .err()
///     .expect("the open never connects");
/// assert_eq!(error.kind(), ErrorKind::Timeout);
/// assert_eq!(error.diagnostics().operation, Some(Operation::Connect));
/// assert_eq!(clock.now(), deadline.instant(), "the fake clock stopped at the deadline");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone)]
pub struct FakeTransportFactory {
    clock: FakeClock,
    state: Arc<Mutex<State>>,
}

impl FakeTransportFactory {
    /// A factory with an empty queue, whose transports and parked opens use `clock`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// let factory = FakeTransportFactory::new(FakeClock::default());
    /// assert_eq!(factory.pending(), 0);
    /// ```
    #[must_use]
    pub fn new(clock: FakeClock) -> Self {
        Self {
            clock,
            state: Arc::new(Mutex::new(State::default())),
        }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut state)
    }

    fn push(&self, plan: Plan) {
        self.with(|state| state.plans.push_back(plan));
    }

    /// Queue an `open` that succeeds with a transport running `script`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script};
    ///
    /// let factory = FakeTransportFactory::new(FakeClock::default());
    /// factory.push_script(Script::default());
    /// assert_eq!(factory.pending(), 1);
    /// ```
    pub fn push_script(&self, script: Script) {
        self.push(Plan::Script(script));
    }

    /// Queue an `open` that parks until the clock reaches the connect deadline, then
    /// fails with `Timeout` and `Diagnostics.operation == Some(Operation::Connect)`.
    ///
    /// The parked call sleeps on the [`FakeClock`], so [`block_on_with`](crate::testkit::block_on_with)
    /// moves time to the deadline and the test finishes at once.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// let factory = FakeTransportFactory::new(FakeClock::default());
    /// factory.push_connect_timeout();
    /// assert_eq!(factory.pending(), 1);
    /// ```
    pub fn push_connect_timeout(&self) {
        self.push(Plan::Park);
    }

    /// Queue an `open` that fails at once with `Disconnected` (a refused connection).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// let factory = FakeTransportFactory::new(FakeClock::default());
    /// factory.push_refusal();
    /// assert_eq!(factory.pending(), 1);
    /// ```
    pub fn push_refusal(&self) {
        self.push(Plan::Refuse);
    }

    /// How many queued opens have not been used yet.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// assert_eq!(FakeTransportFactory::new(FakeClock::default()).pending(), 0);
    /// ```
    #[must_use]
    pub fn pending(&self) -> usize {
        self.with(|state| state.plans.len())
    }

    /// Every `open` call so far, oldest first, including failed ones.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// assert!(FakeTransportFactory::new(FakeClock::default()).opens().is_empty());
    /// ```
    #[must_use]
    pub fn opens(&self) -> Vec<OpenCall> {
        self.with(|state| state.opens.clone())
    }

    /// A probe for every transport handed out so far, oldest first.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory};
    ///
    /// assert!(FakeTransportFactory::new(FakeClock::default()).probes().is_empty());
    /// ```
    #[must_use]
    pub fn probes(&self) -> Vec<FakeTransportProbe> {
        self.with(|state| state.probes.clone())
    }
}

impl TransportFactory for FakeTransportFactory {
    /// Does what the next queued plan says, within `deadline`.
    ///
    /// The call is recorded first. A deadline that has already passed fails with
    /// `Timeout` without using up a queued plan; then the next plan runs: a script opens
    /// a [`FakeTransport`] (sharing this factory's clock, reporting `endpoint` and
    /// `limits`), a refusal fails with `Disconnected`, a connect timeout waits on the
    /// `FakeClock` for `deadline` and fails with `Timeout`. An empty queue fails with
    /// `Internal`. Every failure carries `Diagnostics.operation == Some(Operation::Connect)`.
    /// The deadline is the `deadline` argument.
    ///
    /// # Cancel safety
    /// Cancel-safe in the sense that no transport is left behind; a dropped call that
    /// had already taken its plan has used it up.
    ///
    /// # Wire effects
    /// None.
    fn open<'a>(
        &'a self,
        endpoint: &'a Endpoint,
        limits: ResourceLimits,
        deadline: Deadline,
    ) -> OpFuture<'a, Box<dyn Transport>> {
        Box::pin(async move {
            let connect = Some(Operation::Connect);
            self.with(|state| {
                state.opens.push(OpenCall {
                    endpoint: endpoint.clone(),
                    limits,
                    deadline,
                });
            });
            if deadline.is_elapsed(self.clock.now()) {
                return Err(failure(
                    ErrorKind::Timeout,
                    connect,
                    SITE,
                    "deadline already elapsed",
                ));
            }
            match self.with(|state| state.plans.pop_front()) {
                Some(Plan::Script(script)) => {
                    let transport = FakeTransport::new(script, self.clock.clone(), limits)
                        .with_peer(endpoint.clone());
                    let probe = transport.probe();
                    self.with(|state| state.probes.push(probe));
                    let boxed: Box<dyn Transport> = Box::new(transport);
                    Ok(boxed)
                }
                Some(Plan::Park) => {
                    self.clock.sleep_until(deadline.instant()).await?;
                    Err(failure(
                        ErrorKind::Timeout,
                        connect,
                        SITE,
                        "deadline elapsed",
                    ))
                }
                Some(Plan::Refuse) => Err(failure(
                    ErrorKind::Disconnected,
                    connect,
                    SITE,
                    "connection refused",
                )),
                None => Err(failure(
                    ErrorKind::Internal,
                    connect,
                    SITE,
                    "no scripted open is queued",
                )),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crate::{FailureCategory, PlcError};
    use bytes::{Bytes, BytesMut};

    use super::*;
    use crate::testkit::{Step, block_on_with, poll_once};

    fn endpoint() -> Endpoint {
        Endpoint::Custom("fake://plc".to_owned())
    }

    fn limits() -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(32)
            .build()
            .expect("limits")
    }

    fn deadline(clock: &FakeClock, seconds: u64) -> Deadline {
        Deadline::after(clock.now(), Duration::from_secs(seconds)).expect("representable")
    }

    fn open_error(clock: &FakeClock, factory: &FakeTransportFactory, seconds: u64) -> PlcError {
        let target = endpoint();
        let limit = deadline(clock, seconds);
        block_on_with(clock, factory.open(&target, limits(), limit))
            .err()
            .expect("open fails")
    }

    #[test]
    fn the_factory_is_send_sync_and_usable_as_a_trait_object() {
        fn assert_send_sync<T: Send + Sync + ?Sized>() {}
        assert_send_sync::<FakeTransportFactory>();
        assert_send_sync::<dyn TransportFactory>();
        let shared: Arc<dyn TransportFactory> =
            Arc::new(FakeTransportFactory::new(FakeClock::default()));
        assert_eq!(Arc::strong_count(&shared), 1, "constructed");
    }

    #[test]
    fn scripts_are_handed_out_in_order_and_every_open_is_recorded() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::new(vec![Step::Reply(b"one".to_vec())]));
        factory.push_script(Script::new(vec![Step::Reply(b"two".to_vec())]));
        assert_eq!(factory.pending(), 2, "queued");

        let first_deadline = deadline(&clock, 5);
        let mut first = block_on_with(&clock, factory.open(&endpoint(), limits(), first_deadline))
            .expect("first open");
        let second_deadline = deadline(&clock, 9);
        let mut second =
            block_on_with(&clock, factory.open(&endpoint(), limits(), second_deadline))
                .expect("second open");
        assert_eq!(first.peer(), &endpoint(), "peer is the endpoint opened");
        assert_eq!(first.limits(), &limits(), "limits are the ones passed");
        assert_eq!(factory.pending(), 0, "queue used up");

        let mut buffer = BytesMut::new();
        block_on_with(&clock, second.recv(&mut buffer, deadline(&clock, 1))).expect("recv");
        assert_eq!(
            &buffer[..],
            b"two",
            "the second transport runs the second script"
        );
        buffer.clear();
        block_on_with(&clock, first.recv(&mut buffer, deadline(&clock, 1))).expect("recv");
        assert_eq!(
            &buffer[..],
            b"one",
            "the first transport runs the first script"
        );

        let calls = factory.opens();
        assert_eq!(calls.len(), 2, "two calls");
        assert_eq!(
            calls
                .first()
                .map(|call| (&call.endpoint, call.limits, call.deadline)),
            Some((&endpoint(), limits(), first_deadline)),
            "first call recorded"
        );
        assert_eq!(
            calls.get(1).map(|call| call.deadline),
            Some(second_deadline),
            "second call"
        );
        assert_eq!(factory.probes().len(), 2, "a probe per transport");
    }

    #[test]
    fn probes_see_frames_sent_through_a_boxed_transport() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let mut transport = block_on_with(
            &clock,
            factory.open(&endpoint(), limits(), deadline(&clock, 5)),
        )
        .expect("open");
        block_on_with(
            &clock,
            transport.send(Bytes::from_static(b"frame"), deadline(&clock, 5)),
        )
        .expect("send");
        let probes = factory.clone().probes();
        assert_eq!(
            probes.first().map(FakeTransportProbe::sent),
            Some(vec![Bytes::from_static(b"frame")]),
            "the factory's probe sees the boxed transport's log"
        );
    }

    // covers: SPEC mandatory-failure-cases / F2 transport (connect timeout)
    #[test]
    fn a_scripted_connect_timeout_parks_until_the_clock_passes_the_deadline() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_connect_timeout();
        let limit = deadline(&clock, 5);
        let target = endpoint();
        let mut parked = factory.open(&target, limits(), limit);
        assert!(
            poll_once(&mut parked).is_pending(),
            "parks before the deadline"
        );
        assert_eq!(clock.sleeper_count(), 1, "sleeping on the fake clock");
        let error = block_on_with(&clock, parked).err().expect("times out");
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "connect operation"
        );
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Transport),
            "category"
        );
        assert_eq!(
            clock.now(),
            limit.instant(),
            "the fake clock stopped at the deadline"
        );
        assert!(factory.probes().is_empty(), "no transport was made");
    }

    #[test]
    fn an_elapsed_deadline_times_out_without_using_up_a_queued_open() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let target = endpoint();
        let error = block_on_with(
            &clock,
            factory.open(&target, limits(), Deadline::at(clock.now())),
        )
        .err()
        .expect("elapsed");
        assert_eq!(error.kind(), ErrorKind::Timeout, "timeout");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "connect operation"
        );
        assert_eq!(factory.pending(), 1, "the plan is still queued");
        assert_eq!(factory.opens().len(), 1, "the call was recorded");
    }

    #[test]
    fn a_refusal_is_a_retryable_disconnect_and_an_empty_queue_is_an_internal_error() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_refusal();
        let error = open_error(&clock, &factory, 5);
        assert_eq!(error.kind(), ErrorKind::Disconnected, "refused");
        assert!(error.is_retryable(), "a refused connect may be retried");
        assert_eq!(
            error.diagnostics().operation,
            Some(Operation::Connect),
            "connect operation"
        );
        let error = open_error(&clock, &factory, 5);
        assert_eq!(error.kind(), ErrorKind::Internal, "nothing queued");
        assert_eq!(factory.opens().len(), 2, "both calls recorded");
    }

    #[test]
    fn outage_then_recovery_is_scriptable() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_refusal();
        factory.push_connect_timeout();
        factory.push_script(Script::new(vec![Step::Reply(b"up".to_vec())]));
        assert_eq!(
            open_error(&clock, &factory, 5).kind(),
            ErrorKind::Disconnected,
            "down"
        );
        assert_eq!(
            open_error(&clock, &factory, 5).kind(),
            ErrorKind::Timeout,
            "hung"
        );
        let mut transport = block_on_with(
            &clock,
            factory.open(&endpoint(), limits(), deadline(&clock, 5)),
        )
        .expect("recovered");
        let mut buffer = BytesMut::new();
        block_on_with(&clock, transport.recv(&mut buffer, deadline(&clock, 1))).expect("recv");
        assert_eq!(&buffer[..], b"up", "the recovered link works");
    }
}
