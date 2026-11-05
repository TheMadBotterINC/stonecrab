use super::*;
use crate::{ReconnectPolicy, Timeouts};

fn policy() -> ReconnectPolicy {
    ReconnectPolicy {
        initial: Duration::from_millis(5),
        max: Duration::from_millis(20),
        jitter: 0.0,
        max_attempts: NonZeroU32::new(2),
        ..ReconnectPolicy::default()
    }
}

fn connect(rig: &Rig, policy: ReconnectPolicy, connect_timeout: Duration) -> OpcuaConnection {
    let parsed = rig.driver.parse_config(URI).unwrap();
    let config = crate::ConnectionConfig::builder("opcua", parsed.endpoint().clone())
        .driver_options(Arc::clone(parsed.driver_options().unwrap()))
        .access(crate::AccessPolicy::ReadWrite)
        .reconnect(policy)
        .timeouts(Timeouts {
            connect: connect_timeout,
            ..*parsed.timeouts()
        })
        .build()
        .unwrap();
    let bound = Deadline::after(rig.clock.now(), config.timeouts().connect).unwrap();
    let session = block_on_with(
        &rig.clock,
        rig.driver
            .open_session(&config, bound, &CancellationToken::new()),
    )
    .unwrap();
    OpcuaConnection::new(
        session,
        OpcuaDriver::new()
            .with_clock(Arc::new(rig.clock.clone()))
            .with_transport_factory(Arc::new(rig.factory.clone())),
        config,
        Arc::new(rig.clock.clone()),
    )
}

fn lose(rig: &Rig, connection: &OpcuaConnection) {
    rig.device.disconnect();
    let error = block_on_with(
        &rig.clock,
        connection.read(request(connection, vec![tag("lost", 1)])),
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Disconnected);
    assert_eq!(error.diagnostics().operation, Some(Operation::Read));
    assert_eq!(
        error.diagnostics().category,
        Some(crate::FailureCategory::Transport)
    );
}

fn replacement(rig: &Rig) {
    rig.factory
        .push_script(Script::strict(vec![Step::react(rig.device.reactor())]));
}

fn next_read(connection: &dyn Connection) -> ReadRequest {
    request(connection, vec![tag("next", 42)])
}

#[test]
fn opcua_transport_loss_sets_reconnecting_and_the_next_read_repeats_the_handshake() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    let original = connection.current();
    replacement(&rig);
    let failed_at = rig.clock.now();
    lose(&rig, &connection);
    assert_eq!(
        connection.state(),
        ConnectionState::Reconnecting {
            attempt: 1,
            next_at: failed_at + Duration::from_millis(5)
        }
    );
    connection.observe_failure();
    assert_eq!(rig.factory.opens().len(), 1);
    let mut next = connection.read(next_read(&connection));
    assert!(
        next.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    rig.clock.advance(Duration::from_millis(4));
    assert!(
        next.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(rig.factory.opens().len(), 1);
    rig.clock.advance(Duration::from_millis(1));
    assert_eq!(
        block_on_with(&rig.clock, next).unwrap().iter_ok().count(),
        1
    );
    assert_eq!(connection.state(), ConnectionState::Connected);
    assert!(!Arc::ptr_eq(&original, &connection.current()));
    assert_eq!(rig.factory.opens().len(), 2);
    for probe in rig.factory.probes() {
        let sent = probe.sent();
        assert_eq!(
            sent.iter()
                .filter(|frame| frame.starts_with(b"HEL"))
                .count(),
            1
        );
        assert_eq!(
            sent.iter()
                .filter(|frame| frame.starts_with(b"OPN"))
                .count(),
            1
        );
        for expected in [461, 467, 631] {
            assert_eq!(
                sent.iter()
                    .filter(|frame| frame.starts_with(b"MSG")
                        && matches!(
                            (expected, body(frame)),
                            (461, Service::CreateSessionRequest { .. })
                                | (467, Service::ActivateSessionRequest { .. })
                                | (631, Service::ReadRequest { .. })
                        ))
                    .count(),
                1
            );
        }
    }
    block_on_with(&rig.clock, connection.close()).unwrap();
}

#[test]
fn opcua_reconnect_backoff_advances_once_per_failed_handshake_and_exhausts() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    lose(&rig, &connection);
    for (attempt, delay) in [(2, 10), (3, 0)] {
        rig.factory.push_refusal();
        let error = block_on_with(&rig.clock, connection.read(next_read(&connection))).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert_eq!(rig.factory.opens().len(), attempt);
        if attempt == 2 {
            assert_eq!(
                connection.state(),
                ConnectionState::Reconnecting {
                    attempt: 2,
                    next_at: rig.clock.now() + Duration::from_millis(delay)
                }
            );
        } else {
            assert_eq!(
                connection.state(),
                ConnectionState::Failed(ErrorKind::Disconnected)
            );
        }
    }
    let before = rig.factory.opens().len();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(rig.factory.opens().len(), before);
}

#[test]
fn opcua_reconnect_connect_timeout_counts_but_caller_interruption_does_not() {
    for action in 0..5 {
        let rig = Rig::new();
        let connection = connect(&rig, policy(), Duration::from_millis(10));
        lose(&rig, &connection);
        rig.clock.advance(Duration::from_millis(5));
        rig.factory.push_connect_timeout();
        replacement(&rig);
        let cancel = CancellationToken::new();
        let request = ReadRequest::builder(connection.limits())
            .tag(tag("pending", 1))
            .cancel(cancel.clone())
            .deadline(
                Deadline::after(
                    rig.clock.now(),
                    Duration::from_millis(if action == 4 {
                        10
                    } else if action == 3 {
                        25
                    } else {
                        2
                    }),
                )
                .unwrap(),
            )
            .build()
            .unwrap();
        let mut pending = connection.read(request);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        match action {
            0 => drop(pending),
            1 => {
                cancel.cancel();
                assert_eq!(
                    block_on_with(&rig.clock, pending).unwrap_err().kind(),
                    ErrorKind::Cancelled
                );
            }
            _ => {
                assert_eq!(
                    block_on_with(&rig.clock, pending).unwrap_err().kind(),
                    ErrorKind::Timeout
                );
            }
        }
        assert!(
            matches!(connection.state(), ConnectionState::Reconnecting { attempt, .. } if attempt == if action >= 3 { 2 } else { 1 })
        );
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap()
                .iter_ok()
                .count(),
            1
        );
        assert_eq!(rig.factory.opens().len(), 3);
        assert_eq!(connection.state(), ConnectionState::Connected);
    }
}

#[test]
fn opcua_reconnect_never_replays_a_write_or_pending_read() {
    let clock = FakeClock::default();
    let factory = FakeTransportFactory::new(clock.clone());
    let device = Device::default();
    let mut peer = device.reactor();
    factory.push_script(Script::strict(vec![Step::react(move |wire| {
        let reply = peer(wire);
        if wire.starts_with(b"MSG") && matches!(body(wire), Service::WriteRequest { .. }) {
            Reaction::Close
        } else {
            reply
        }
    })]));
    let rig = Rig {
        driver: OpcuaDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone())),
        clock,
        factory,
        device,
    };
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    rig.device.timeout();
    let mut pending = connection.read(next_read(&connection));
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let write = WriteRequest::builder(connection.limits())
        .item(
            Tag::new(
                TagId::new("set").unwrap(),
                Arc::new(OpcuaAddress::parse("ns=2;i=42;DINT").unwrap()),
            ),
            PlcValue::I32(99),
        )
        .build()
        .unwrap();
    let mut writing = connection.write(write);
    assert!(
        writing
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(
        pending
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let result = block_on_with(&rig.clock, writing).unwrap();
    assert_eq!(result.unknown_count(), 1);
    assert!(matches!(
        connection.state(),
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
    replacement(&rig);
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap()
            .iter_ok()
            .count(),
        1
    );
    assert_eq!(
        block_on_with(&rig.clock, pending).unwrap_err().kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(connection.state(), ConnectionState::Connected);
    let probes = rig.factory.probes();
    assert_eq!(
        probes
            .first()
            .unwrap()
            .sent()
            .iter()
            .filter(|frame| frame.starts_with(b"MSG")
                && matches!(body(frame), Service::WriteRequest { .. }))
            .count(),
        1
    );
    assert_eq!(
        probes
            .get(1)
            .unwrap()
            .sent()
            .iter()
            .filter(|frame| frame.starts_with(b"MSG")
                && matches!(body(frame), Service::WriteRequest { .. }))
            .count(),
        0
    );
    assert_eq!(
        probes
            .get(1)
            .unwrap()
            .sent()
            .iter()
            .filter(|frame| frame.starts_with(b"MSG")
                && matches!(body(frame), Service::ReadRequest { .. }))
            .count(),
        1
    );
    block_on_with(&rig.clock, connection.close()).unwrap();
}

#[test]
fn opcua_concurrent_reconnect_callers_share_one_fresh_session_and_reset_the_budget() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    lose(&rig, &connection);
    rig.factory.push_refusal();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    replacement(&rig);
    let mut first = connection.read(next_read(&connection));
    let mut second = connection.read(next_read(&connection));
    for pending in [&mut first, &mut second] {
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(
        block_on_with(&rig.clock, first).unwrap().iter_ok().count(),
        1
    );
    assert_eq!(
        block_on_with(&rig.clock, second).unwrap().iter_ok().count(),
        1
    );
    assert_eq!(rig.factory.opens().len(), 3);
    lose(&rig, &connection);
    assert!(matches!(
        connection.state(),
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
}

#[test]
fn opcua_close_interrupts_reconnect_backoff_and_handshake_without_resurrection() {
    for in_handshake in [false, true] {
        let rig = Rig::new();
        let connection = connect(&rig, policy(), Duration::from_millis(10));
        lose(&rig, &connection);
        if in_handshake {
            rig.clock.advance(Duration::from_millis(5));
            rig.factory.push_connect_timeout();
        }
        let mut pending = connection.read(next_read(&connection));
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        let before = rig.factory.opens().len();
        let _closed = block_on_with(&rig.clock, connection.close());
        assert_eq!(
            block_on_with(&rig.clock, pending).unwrap_err().kind(),
            ErrorKind::Disconnected
        );
        assert_eq!(connection.state(), ConnectionState::Closed);
        replacement(&rig);
        rig.clock.advance(Duration::from_secs(10));
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
        assert_eq!(rig.factory.opens().len(), before);
    }
}

#[test]
fn opcua_non_transport_lane_failures_remain_terminal_even_after_idle_expiry() {
    for kind in [
        ErrorKind::Trust,
        ErrorKind::Authentication,
        ErrorKind::MalformedData,
        ErrorKind::Internal,
    ] {
        let rig = Rig::new();
        let connection = connect(&rig, policy(), Duration::from_millis(10));
        connection
            .current()
            .lane
            .fail(PlcError::new(kind, "injected terminal failure"));
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
        assert_eq!(connection.state(), ConnectionState::Failed(kind));
        rig.clock.advance(Duration::from_secs(120));
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap_err()
                .kind(),
            ErrorKind::Disconnected
        );
        assert_eq!(rig.factory.opens().len(), 1);
    }
}

#[test]
fn opcua_interruption_during_backoff_does_not_open_or_spend_an_attempt() {
    for action in 0..3 {
        let rig = Rig::new();
        let connection = connect(&rig, policy(), Duration::from_millis(10));
        lose(&rig, &connection);
        let scheduled = connection.state();
        let cancel = CancellationToken::new();
        let request = ReadRequest::builder(connection.limits())
            .tag(tag("interrupted", 1))
            .cancel(cancel.clone())
            .deadline(Deadline::after(rig.clock.now(), Duration::from_millis(2)).unwrap())
            .build()
            .unwrap();
        let mut pending = connection.read(request);
        assert!(
            pending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
        match action {
            0 => drop(pending),
            1 => {
                cancel.cancel();
                assert_eq!(
                    block_on_with(&rig.clock, pending).unwrap_err().kind(),
                    ErrorKind::Cancelled
                );
            }
            _ => {
                assert_eq!(
                    block_on_with(&rig.clock, pending).unwrap_err().kind(),
                    ErrorKind::Timeout
                );
            }
        }
        assert_eq!(connection.state(), scheduled);
        assert_eq!(rig.factory.opens().len(), 1);
        replacement(&rig);
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap()
                .iter_ok()
                .count(),
            1
        );
        assert_eq!(rig.factory.opens().len(), 2);
    }
}

#[test]
fn opcua_an_open_socket_without_session_replies_does_not_reset_reconnect_attempts() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    lose(&rig, &connection);
    rig.factory
        .push_script(Script::strict(vec![Step::react(|_| Reaction::Close)]));
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    assert!(matches!(
        connection.state(),
        ConnectionState::Reconnecting { attempt: 2, .. }
    ));
    assert_eq!(rig.factory.opens().len(), 2);
    rig.factory.push_refusal();
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::Disconnected
    );
    assert_eq!(
        connection.state(),
        ConnectionState::Failed(ErrorKind::Disconnected)
    );
    assert_eq!(rig.factory.opens().len(), 3);
}

#[test]
fn opcua_reconnect_state_watchers_wake_after_releasing_the_current_session_lock() {
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;
    struct Watcher {
        connection: Arc<OpcuaConnection>,
        wakes: AtomicUsize,
    }
    impl Wake for Watcher {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }
        fn wake_by_ref(self: &Arc<Self>) {
            assert!(self.connection.session.try_lock().is_ok());
            self.wakes.fetch_add(1, Ordering::SeqCst);
        }
    }
    let rig = Rig::new();
    let connection = Arc::new(connect(&rig, policy(), Duration::from_millis(10)));
    let watcher = Arc::new(Watcher {
        connection: Arc::clone(&connection),
        wakes: AtomicUsize::new(0),
    });
    let waker = Waker::from(Arc::clone(&watcher));
    let watch = connection.watch_state();
    let mut changed = watch.changed();
    assert!(
        changed
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    lose(&rig, &connection);
    assert_eq!(watcher.wakes.load(Ordering::SeqCst), 1);
    block_on_with(&rig.clock, changed).unwrap();
    let mut changed = watch.changed();
    assert!(
        changed
            .as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );
    replacement(&rig);
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&*connection)))
            .unwrap()
            .iter_ok()
            .count(),
        1
    );
    assert_eq!(watcher.wakes.load(Ordering::SeqCst), 2);
    block_on_with(&rig.clock, changed).unwrap();
    assert_eq!(watch.current(), ConnectionState::Connected);
}

#[test]
fn opcua_a_clock_that_wakes_before_backoff_cannot_open_a_socket() {
    struct Early(FakeClock);
    impl Clock for Early {
        fn now(&self) -> std::time::Instant {
            self.0.now()
        }
        fn wall_now(&self) -> std::time::SystemTime {
            self.0.wall_now()
        }
        fn sleep_until(&self, deadline: std::time::Instant) -> OpFuture<'_, ()> {
            if deadline.saturating_duration_since(self.now()) == Duration::from_millis(5) {
                Box::pin(async { Ok(()) })
            } else {
                self.0.sleep_until(deadline)
            }
        }
    }
    let rig = Rig::new();
    let mut connection = connect(&rig, policy(), Duration::from_millis(10));
    connection.clock = Arc::new(Early(rig.clock.clone()));
    lose(&rig, &connection);
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidConfig
    );
    assert_eq!(rig.factory.opens().len(), 1);
    assert!(matches!(
        connection.state(),
        ConnectionState::Reconnecting { attempt: 1, .. }
    ));
}

#[test]
fn opcua_pending_requests_fail_disconnected_and_are_not_replayed_after_reconnect() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    rig.device.timeout();
    let mut receiving = connection.read(next_read(&connection));
    assert!(
        receiving
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    let mut queued = connection.read(next_read(&connection));
    assert!(
        queued
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    connection
        .current()
        .lane
        .fail(PlcError::new(ErrorKind::Disconnected, "injected link loss"));
    replacement(&rig);
    assert_eq!(
        block_on_with(&rig.clock, connection.read(next_read(&connection)))
            .unwrap()
            .iter_ok()
            .count(),
        1
    );
    for pending in [receiving, queued] {
        assert_eq!(
            block_on_with(&rig.clock, pending).unwrap_err().kind(),
            ErrorKind::Disconnected
        );
    }
    assert_eq!(rig.factory.opens().len(), 2);
    for probe in rig.factory.probes() {
        assert_eq!(
            probe
                .sent()
                .iter()
                .filter(|frame| frame.starts_with(b"MSG")
                    && matches!(body(frame), Service::ReadRequest { .. }))
                .count(),
            1
        );
    }
    assert_eq!(connection.state(), ConnectionState::Connected);
}

#[test]
fn opcua_failed_renewal_timeout_recovers_but_a_healthy_request_timeout_does_not() {
    for failed_lane in [false, true] {
        let rig = Rig::new();
        let connection = connect(&rig, policy(), Duration::from_millis(10));
        if failed_lane {
            connection
                .current()
                .lane
                .fail(PlcError::new(ErrorKind::Timeout, "renewal timed out"));
            replacement(&rig);
        } else {
            rig.device.timeout();
            assert_eq!(
                block_on_with(&rig.clock, connection.read(next_read(&connection)))
                    .unwrap_err()
                    .kind(),
                ErrorKind::Timeout
            );
        }
        assert_eq!(
            block_on_with(&rig.clock, connection.read(next_read(&connection)))
                .unwrap()
                .iter_ok()
                .count(),
            1
        );
        assert_eq!(rig.factory.opens().len(), if failed_lane { 2 } else { 1 });
        assert_eq!(connection.state(), ConnectionState::Connected);
    }
}

#[test]
fn opcua_a_late_lifecycle_update_cannot_overwrite_a_terminal_state() {
    let rig = Rig::new();
    let connection = connect(&rig, policy(), Duration::from_millis(10));
    block_on_with(&rig.clock, connection.close()).unwrap();
    assert!(
        connection
            .state
            .replace_if(
                ConnectionState::Reconnecting {
                    attempt: 1,
                    next_at: rig.clock.now()
                },
                ConnectionState::Connected
            )
            .is_empty()
    );
    assert_eq!(connection.state(), ConnectionState::Closed);
}
