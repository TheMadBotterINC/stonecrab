//! Tests of the report machinery, the assertion helpers and the skip logic.
//!
//! The testkit cannot depend on the simulator (dependency cycle, ADR-0003), so the full
//! run is in `crates/stonecrab/tests/sim_contract.rs`. Here a tiny driver that
//! supports nothing stands in (the testkit is a sanctioned home for doubles, 13.5).

use std::sync::Arc;
use std::time::Duration;

use crate::{
    Capabilities, Clock, Connection, ConnectionConfig, ConnectionState, Deadline, Diagnostics,
    Driver, Endpoint, ErrorKind, FailureCategory, ItemError, OpFuture, Operation, PlcError,
    PlcValue, ProtocolAddress, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Result,
    StateCell, StateWatch, Tag, TagId, WriteRequest, WriteResponse,
};

use super::cases::{Must, Stop, each_exception, ensure, ensure_eq, fail, names, refused, shown};
use super::*;
use crate::testkit::{FakeClock, FixtureAddress, block_on_with};

/// A driver that supports nothing: every read and write is `Unsupported`, close works.
struct Nothing {
    clock: Arc<FakeClock>,
    refuse_connect: bool,
    slow_close: bool,
}

struct NothingConnection {
    clock: Arc<FakeClock>,
    slow_close: bool,
    state: StateCell,
}

impl Driver for Nothing {
    fn scheme(&self) -> &'static str {
        "nothing"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        if uri == "bad" {
            return Err(PlcError::new(ErrorKind::InvalidConfig, "no such device"));
        }
        ConnectionConfig::builder("nothing", Endpoint::Custom(uri.to_owned())).build()
    }

    fn parse_address(
        &self,
        text: &str,
        _options: &crate::AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(FixtureAddress::new(text, None)))
    }

    fn connect(&self, _config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        Box::pin(async move {
            if self.refuse_connect {
                return Err(PlcError::new(ErrorKind::Disconnected, "refused"));
            }
            let (state, _watch) = StateCell::new(ConnectionState::Connected);
            let connection: Box<dyn Connection> = Box::new(NothingConnection {
                clock: Arc::clone(&self.clock),
                slow_close: self.slow_close,
                state,
            });
            Ok(connection)
        })
    }
}

impl Connection for NothingConnection {
    fn state(&self) -> ConnectionState {
        self.state.current()
    }

    fn watch_state(&self) -> StateWatch {
        self.state.watch()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn limits(&self) -> ResourceLimits {
        ResourceLimits::builder().build().expect("default limits")
    }

    fn read(&self, _request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        Box::pin(async { Err(PlcError::new(ErrorKind::Unsupported(Operation::Read), "no")) })
    }

    fn write(&self, _request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        Box::pin(async {
            Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                "no",
            ))
        })
    }

    fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(async move {
            if self.state.current() == ConnectionState::Closed {
                return Ok(());
            }
            self.state.set(ConnectionState::Closing);
            if self.slow_close {
                let until = Deadline::after(self.clock.now(), Duration::from_millis(5))?;
                self.clock.sleep_until(until.instant()).await?;
            }
            self.state.set(ConnectionState::Closed);
            Ok(())
        })
    }
}

fn nothing(refuse_connect: bool, slow_close: bool) -> (Arc<FakeClock>, Nothing) {
    let clock = Arc::new(FakeClock::default());
    let driver = Nothing {
        clock: Arc::clone(&clock),
        refuse_connect,
        slow_close,
    };
    (clock, driver)
}

fn fixture(clock: &Arc<FakeClock>) -> ContractFixture {
    ContractFixture::new(
        Arc::<FakeClock>::clone(clock),
        "readable",
        "rejected",
        Arc::new(FixtureAddress::new("elsewhere", None)),
        || 0,
        || 0,
    )
}

fn run(
    clock: &Arc<FakeClock>,
    driver: &Nothing,
    uri: &str,
    fx: &ContractFixture,
) -> ContractReport {
    block_on_with(clock, ContractSuite::run(driver, uri, fx)).expect("the suite always resolves")
}

fn case(name: &'static str) -> ContractCase {
    ContractCase::new(name, "F3 test", &[])
}

#[test]
fn the_suite_names_thirty_five_distinct_cases_each_with_a_coverage_mapping() {
    let cases = ContractSuite::cases();
    assert_eq!(cases.len(), 35, "the case list");
    let mut names: Vec<&str> = cases.iter().map(|case| case.name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 35, "every sentence is distinct");
    assert!(
        cases.iter().all(|case| !case.covers.is_empty()),
        "every case maps to a mandatory case"
    );
}

#[test]
fn a_report_counts_its_outcomes_and_is_green_only_without_failures_and_with_a_pass() {
    let passed = (case("one"), CaseOutcome::Passed);
    let skipped = (case("two"), CaseOutcome::Skipped("because".to_owned()));
    let failed = (case("three"), CaseOutcome::Failed("broken".to_owned()));
    let green = ContractReport::new(vec![passed.clone(), skipped.clone()]);
    let counts = (green.passed(), green.failed(), green.skipped());
    assert_eq!(counts, (1, 0, 1), "counts");
    assert!(green.is_green(), "a pass and a skip is green");
    let red = ContractReport::new(vec![passed, skipped.clone(), failed]);
    let counts = (red.passed(), red.failed(), red.skipped());
    assert_eq!(counts, (1, 1, 1), "counts");
    assert!(!red.is_green(), "a failure is not green");
    assert!(
        !ContractReport::new(vec![skipped]).is_green(),
        "all skipped exercised nothing"
    );
    assert_eq!(
        red.outcome("two"),
        Some(&CaseOutcome::Skipped("because".to_owned())),
        "lookup"
    );
    assert_eq!(red.outcome("four"), None, "no such case");
}

#[test]
fn display_lists_every_case_with_its_label_reason_and_coverage() {
    let report = ContractReport::new(vec![
        (case("one"), CaseOutcome::Passed),
        (case("two"), CaseOutcome::Skipped("because".to_owned())),
        (case("three"), CaseOutcome::Failed("broken".to_owned())),
    ]);
    let expected = "contract suite: 1 passed, 1 failed, 1 skipped\n\
        PASS  one  [covers: F3 test]\n\
        SKIP  two  [covers: F3 test]\n      reason: because\n\
        FAIL  three  [covers: F3 test]\n      reason: broken\n";
    assert_eq!(report.to_string(), expected, "the exact report text");
}

#[test]
fn faults_and_requirements_print_the_words_the_skip_reasons_use() {
    assert_eq!(Fault::Disconnect.to_string(), "disconnect", "disconnect");
    assert_eq!(Fault::Timeout.to_string(), "timeout", "timeout");
    assert_eq!(
        Fault::DeviceException.to_string(),
        "device exception",
        "exception"
    );
    assert_eq!(
        Fault::UnknownAfterTransmit.to_string(),
        "unknown-after-transmit",
        "unknown"
    );
    for (need, text) in [
        (Requires::Read, "driver supports read"),
        (Requires::Write, "driver supports write"),
        (Requires::Ping, "driver supports ping"),
        (
            Requires::WriteUnsupported,
            "driver reports write unsupported",
        ),
        (
            Requires::WritableAddress,
            "fixture names a writable address",
        ),
        (Requires::Fault(Fault::Timeout), "fixture fault: timeout"),
        (Requires::Subscribe, "driver supports subscribe"),
    ] {
        assert_eq!(need.to_string(), text, "requirement text");
    }
}

#[test]
fn the_fixture_builder_sets_each_optional_part_and_debug_prints_names_only() {
    let clock = Arc::new(FakeClock::default());
    let bare = fixture(&clock);
    assert!(
        bare.writable.is_none() && !bare.declares_device_time,
        "defaults"
    );
    assert!(
        bare.inject_disconnect.is_none()
            && bare.inject_timeout.is_none()
            && bare.inject_device_exception.is_none()
            && bare.inject_unknown_after_transmit.is_none(),
        "no hooks by default"
    );
    let full = fixture(&clock)
        .with_writable("out", PlcValue::U16(9))
        .with_device_time()
        .with_inject_disconnect(|| ())
        .with_inject_timeout(|| ())
        .with_inject_device_exception(|| ())
        .with_inject_unknown_after_transmit(|| ());
    assert!(full.declares_device_time, "device time declared");
    assert!(
        full.inject_disconnect.is_some()
            && full.inject_timeout.is_some()
            && full.inject_device_exception.is_some()
            && full.inject_unknown_after_transmit.is_some(),
        "every hook set"
    );
    let text = format!("{full:?}");
    for name in [
        "readable",
        "rejected",
        "fixture",
        "\"out\"",
        "inject_timeout: true",
    ] {
        assert!(text.contains(name), "Debug names {name}: {text}");
    }
    assert!(
        !text.contains("U16"),
        "Debug never prints the value: {text}"
    );
    assert!(
        !format!("{bare:?}").contains("inject_timeout: true"),
        "bare has no hook"
    );
}

#[test]
fn the_assertion_helpers_pass_what_holds_and_fail_with_a_reason_that_names_the_values() {
    let reason = |step: std::result::Result<(), Stop>| match step {
        Err(Stop::Fail(text)) => text,
        Err(Stop::Skip(text)) => format!("skipped: {text}"),
        Ok(()) => "passed".to_owned(),
    };
    assert_eq!(reason(ensure(true, "holds")), "passed", "ensure holds");
    assert_eq!(reason(ensure(false, "broken")), "broken", "ensure fails");
    assert_eq!(reason(ensure_eq(1, 1, "same")), "passed", "ensure_eq holds");
    assert_eq!(
        reason(ensure_eq(1, 2, "count")),
        "count: expected 2, got 1",
        "ensure_eq fails"
    );
    assert_eq!(shown("x"), "\"x\"", "shown is Debug");
    let slow = Err::<(), _>(PlcError::new(ErrorKind::Timeout, "slow"));
    assert_eq!(
        reason(slow.must("a call")),
        "a call: unexpected Timeout error: slow",
        "must names the error"
    );
    assert_eq!(
        Ok::<_, PlcError>(5).must("fine").ok(),
        Some(5),
        "must passes success"
    );
    let wanted = ErrorKind::Cancelled;
    let wrong = refused::<()>(
        Err(PlcError::new(ErrorKind::Timeout, "slow")),
        wanted,
        "read",
    );
    assert_eq!(
        reason(wrong.map(|_| ())),
        "read: expected a Cancelled error, got Timeout: slow",
        "a wrong kind"
    );
    let success = refused(Ok(1), wanted, "read");
    assert_eq!(
        reason(success.map(|_| ())),
        "read: expected a Cancelled error, got success",
        "success where an error is due"
    );
    let right = refused::<()>(Err(PlcError::new(wanted, "stop")), wanted, "read");
    assert_eq!(
        right.ok().map(|error| error.kind()),
        Some(wanted),
        "the expected error comes back"
    );
    assert_eq!(reason(fail::<()>("no")), "no", "fail");
}

#[test]
fn diagnostics_must_name_the_read_a_state_and_the_category() {
    let complete = |operation, state, category| {
        let found = Diagnostics {
            operation,
            connection_state: state,
            category,
            ..Diagnostics::default()
        };
        PlcError::new(ErrorKind::Timeout, "slow").with_diagnostics(found)
    };
    let (read, open, transport) = (
        Some(Operation::Read),
        Some(ConnectionState::Connected),
        Some(FailureCategory::Transport),
    );
    let good = complete(read, open, transport);
    assert!(
        names(&good, FailureCategory::Transport, "t").is_ok(),
        "complete diagnostics"
    );
    let cases = [
        (complete(None, open, transport), "operation"),
        (complete(read, None, transport), "connection_state"),
        (complete(read, open, None), "category"),
        (
            complete(Some(Operation::Write), open, transport),
            "operation",
        ),
        (
            complete(read, open, Some(FailureCategory::Protocol)),
            "category",
        ),
    ];
    for (error, field) in cases {
        let step = names(&error, FailureCategory::Transport, "t");
        assert!(
            matches!(&step, Err(Stop::Fail(text)) if text.contains(field)),
            "{field} is named in the failure"
        );
    }
}

#[test]
fn each_exception_checks_whole_call_errors_and_every_item_and_rejects_a_value() {
    let boom = || PlcError::new(ErrorKind::DeviceException, "refused");
    let mut seen = 0;
    let whole = each_exception(Err(boom()), |_| {
        seen += 1;
        Ok(())
    });
    assert!(
        whole.is_ok() && seen == 1,
        "a whole-call error is checked once"
    );

    let now = FakeClock::default().now();
    let tag = |id: &str| {
        Tag::new(
            TagId::new(id).expect("id"),
            Arc::new(FixtureAddress::new(id, None)),
        )
    };
    let (a, b) = (tag("a"), tag("b"));
    let request = ReadRequest::builder(ResourceLimits::builder().build().expect("limits"))
        .tag(a.clone())
        .tag(b.clone())
        .build()
        .expect("request");
    let response = |first: std::result::Result<PlcValue, ItemError>| {
        ReadResponse::builder(&request, now, now)
            .item(ReadItem::new(a.id().clone(), first, now))
            .item(ReadItem::new(
                b.id().clone(),
                Err(ItemError::new(boom())),
                now,
            ))
            .build()
            .expect("response")
    };
    let mut items = 0;
    let all = each_exception(Ok(response(Err(ItemError::new(boom())))), |error| {
        items += 1;
        ensure_eq(error.kind(), ErrorKind::DeviceException, "kind")
    });
    assert!(all.is_ok() && items == 2, "every item is checked");
    let value = each_exception(Ok(response(Ok(PlcValue::U16(1)))), |_| Ok(()));
    assert!(
        matches!(value, Err(Stop::Fail(_))),
        "a value read despite the exception fails"
    );
}

// covers: SPEC 'Contract suite' / a case the driver cannot run is skipped with a stated reason
#[test]
fn a_driver_with_nothing_passes_the_cases_it_can_and_skips_the_rest_with_reasons() {
    let (clock, driver) = nothing(false, true);
    let fx = fixture(&clock).with_writable("out", PlcValue::U16(1));
    let report = run(&clock, &driver, "x", &fx);
    assert!(report.is_green(), "no failures:\n{report}");
    for name in [
        "capabilities match behavior and unsupported operations perform no I/O",
        "connect and close walk the lifecycle states observed through the state watch",
        "close is terminal and a second close succeeds",
        "an unsupported write performs no I/O",
        "an unsupported ping performs no I/O and changes no state",
        "an unsupported subscribe performs no I/O and changes no state",
        "an unsupported browse performs no I/O and changes no state",
    ] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}:\n{report}"
        );
    }
    assert_eq!(report.passed(), 7, "exactly the seven:\n{report}");
    assert_eq!(
        report.skipped(),
        28,
        "everything else is skipped:\n{report}"
    );
    for name in [
        "a polling driver refuses an event subscription with no I/O",
        "closing the connection ends its subscriptions after the queued events",
    ] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Skipped(
                "driver reports subscribe unsupported".to_owned()
            )),
            "subscription cases:\n{report}"
        );
    }
    assert_eq!(
        report.outcome("a partial failure keeps the successes"),
        Some(&CaseOutcome::Skipped(
            "driver reports read unsupported".to_owned()
        )),
        "read cases:\n{report}"
    );
    assert_eq!(
        report.outcome("a read-only policy rejects a write with no I/O"),
        Some(&CaseOutcome::Skipped(
            "driver reports write unsupported".to_owned()
        )),
        "write cases:\n{report}"
    );
}

struct Browsing {
    inner: Nothing,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    wrong_query: bool,
}

struct BrowsingConnection {
    inner: Box<dyn Connection>,
    clock: Arc<FakeClock>,
    calls: Arc<std::sync::atomic::AtomicUsize>,
    wrong_query: bool,
}

impl Driver for Browsing {
    fn scheme(&self) -> &'static str {
        self.inner.scheme()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            browse: true,
            ..Capabilities::default()
        }
    }
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        self.inner.parse_config(uri)
    }
    fn parse_address(
        &self,
        text: &str,
        options: &crate::AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        self.inner.parse_address(text, options)
    }
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        Box::pin(async move {
            let connection: Box<dyn Connection> = Box::new(BrowsingConnection {
                inner: self.inner.connect(config).await?,
                clock: Arc::clone(&self.inner.clock),
                calls: Arc::clone(&self.calls),
                wrong_query: self.wrong_query,
            });
            Ok(connection)
        })
    }
}

impl Connection for BrowsingConnection {
    fn state(&self) -> ConnectionState {
        self.inner.state()
    }
    fn watch_state(&self) -> StateWatch {
        self.inner.watch_state()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            browse: true,
            ..self.inner.capabilities()
        }
    }
    fn limits(&self) -> ResourceLimits {
        self.inner.limits()
    }
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        self.inner.read(request)
    }
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        self.inner.write(request)
    }
    fn close(&self) -> OpFuture<'_, ()> {
        self.inner.close()
    }
    fn browse(&self, request: crate::BrowseRequest) -> OpFuture<'_, crate::BrowseResponse> {
        Box::pin(async move {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let request = if self.wrong_query {
                crate::BrowseRequest::builder(self.limits())
                    .query(TagId::new("foreign")?, "")
                    .build()?
            } else {
                request
            };
            let now = self.clock.now();
            let mut response = crate::BrowseResponse::builder(&request, now, now);
            for query in request.queries() {
                let node = crate::BrowseNode::new(
                    Arc::new(FixtureAddress::new("nothing", None)),
                    query.text(),
                )?;
                response =
                    response.item(crate::BrowseResult::new(query.id().clone(), Ok(vec![node])));
            }
            response.build()
        })
    }
}

#[test]
fn browse_contract_requirements_and_supported_correlation_are_exercised() {
    for wrong_query in [false, true] {
        let (clock, inner) = nothing(false, false);
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let driver = Browsing {
            inner,
            calls: Arc::clone(&calls),
            wrong_query,
        };
        let missing =
            block_on_with(&clock, ContractSuite::run(&driver, "x", &fixture(&clock))).unwrap();
        assert_eq!(
            missing.outcome("a browse correlates queries and respects the node bound"),
            Some(&CaseOutcome::Skipped(
                "fixture has no browsable query".into()
            ))
        );
        assert_eq!(
            missing.outcome("an unsupported browse performs no I/O and changes no state"),
            Some(&CaseOutcome::Skipped(
                "driver reports browse supported".into()
            ))
        );
        let fx = fixture(&clock).with_browsable("opaque fixture query");
        assert_eq!(fx.browsable.as_deref(), Some("opaque fixture query"));
        assert!(!format!("{fx:?}").contains("opaque fixture query"));
        assert_eq!(Requires::Browse.to_string(), "driver supports browse");
        assert_eq!(
            Requires::BrowsableQuery.to_string(),
            "fixture names a browsable query"
        );
        let report = block_on_with(&clock, ContractSuite::run(&driver, "x", &fx)).unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(report.is_green(), !wrong_query, "{report}");
        if wrong_query {
            assert!(
                matches!(report.outcome("a browse correlates queries and respects the node bound"), Some(CaseOutcome::Failed(reason)) if reason.contains("identity"))
            );
        } else {
            assert_eq!(
                report.outcome("a browse correlates queries and respects the node bound"),
                Some(&CaseOutcome::Passed)
            );
        }
    }
}

#[test]
fn cached_browse_forwards_the_override_and_a_released_lease_does_not_call_it() {
    let (clock, inner) = nothing(false, false);
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let driver = Browsing {
        inner,
        calls: Arc::clone(&calls),
        wrong_query: false,
    };
    let mut registry = crate::DriverRegistry::new();
    registry.register(Arc::new(driver)).unwrap();
    let cache =
        crate::ConnectionCache::builder(Arc::new(registry), Arc::<FakeClock>::clone(&clock))
            .build()
            .unwrap();
    let lease = block_on_with(&clock, cache.get("nothing://peer")).unwrap();
    let request = || {
        crate::BrowseRequest::builder(lease.limits())
            .query(TagId::new("root").unwrap(), "start")
            .build()
            .unwrap()
    };
    let response = block_on_with(&clock, lease.browse(request())).unwrap();
    assert_eq!(response.items().len(), 1);
    assert_eq!(
        response
            .items()
            .first()
            .unwrap()
            .outcome()
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .name(),
        "start"
    );
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    block_on_with(&clock, lease.close()).unwrap();
    let error = block_on_with(&clock, lease.browse(request())).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Disconnected);
    assert_eq!(error.diagnostics().operation, Some(Operation::Browse));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

// covers: SPEC 'Contract suite' / a skip names the missing fixture requirement
#[test]
fn without_a_writable_address_the_unsupported_write_case_is_skipped_by_name() {
    let (clock, driver) = nothing(false, false);
    let report = run(&clock, &driver, "x", &fixture(&clock));
    assert_eq!(
        report.outcome("an unsupported write performs no I/O"),
        Some(&CaseOutcome::Skipped(
            "fixture has no writable address".to_owned()
        )),
        "the missing address is named:\n{report}"
    );
}

// covers: SPEC 'Contract suite' / a driver failure becomes Failed, never a panic
#[test]
fn a_driver_whose_connect_fails_reports_failures_instead_of_panicking() {
    let (clock, driver) = nothing(true, false);
    let report = run(&clock, &driver, "x", &fixture(&clock));
    assert!(!report.is_green(), "not green:\n{report}");
    assert_eq!(
        report.passed(),
        0,
        "nothing passes without a connection:\n{report}"
    );
    assert!(
        matches!(
            report.outcome("close is terminal and a second close succeeds"),
            Some(CaseOutcome::Failed(reason))
                if reason.starts_with("connecting: unexpected Disconnected")
        ),
        "the reason names the connect failure:\n{report}"
    );
}

// covers: SPEC 'Contract suite' / a driver failure becomes Failed, never a panic
#[test]
fn a_connection_string_the_driver_rejects_fails_the_cases_that_need_it() {
    let (clock, driver) = nothing(false, false);
    let report = run(&clock, &driver, "bad", &fixture(&clock));
    assert!(
        matches!(
            report.outcome("close is terminal and a second close succeeds"),
            Some(CaseOutcome::Failed(reason))
                if reason.starts_with("parsing the connection string")
        ),
        "a parse failure is a case failure:\n{report}"
    );
}

#[test]
fn the_public_types_are_send_sync_and_static() {
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_send_sync::<ContractFixture>();
    assert_send_sync::<ContractReport>();
    assert_send_sync::<ContractCase>();
    assert_send_sync::<CaseOutcome>();
    assert_send_sync::<ContractSuite>();
    assert_send_sync::<Fault>();
    assert_send_sync::<Requires>();
}
