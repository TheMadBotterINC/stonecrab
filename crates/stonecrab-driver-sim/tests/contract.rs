//! The common driver contract suite (slice S13, row `SIM-CONTRACT-SUITE` of
//! `docs/parity/sim.md`) run against the simulator.
//!
//! The suite lives in the testkit; the simulator supplies the device, the fault hooks
//! (each one queues a `FailureStep` on the device) and the counters. Everything runs on
//! the testkit's `FakeClock`, so no case waits on real time (`CODING_STANDARDS` 13.4).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::num::NonZeroUsize;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use stonecrab_core::PlcValue;
    use stonecrab_driver_sim::{FailureStep, SimDevice, SimDriver, SimTag};
    use stonecrab_testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, FakeClock, FixtureAddress,
        block_on_with,
    };

    /// A simulator on a fake clock and the fixture that wires the suite to it.
    struct Bench {
        clock: Arc<FakeClock>,
        driver: SimDriver,
        fixture: ContractFixture,
    }

    /// Queue `step` on the device when the hook fires.
    fn hook(device: &Arc<SimDevice>, step: FailureStep) -> impl Fn() + Send + Sync + 'static {
        let device = Arc::clone(device);
        move || {
            device
                .push_failure(step.clone())
                .expect("room in the failure script");
        }
    }

    impl Bench {
        fn new() -> Self {
            let clock = Arc::new(FakeClock::default());
            let device = SimDevice::new().with_max_items(NonZeroUsize::new(4).expect("four"));
            // The readable address is a ramp with a read counter; `SINK/out` needs no tag.
            let (level, reads) = SimTag::new(PlcValue::U16(0)).with_read_counter();
            device.set("level", level).expect("level");
            device
                .ramp("level", PlcValue::U16(0), PlcValue::U16(1))
                .expect("ramp");
            let driver = SimDriver::new()
                .with_clock(Arc::<FakeClock>::clone(&clock))
                .with_device("bench", device)
                .expect("device");
            let device = driver.device("bench").expect("bench device");
            let probe_device = Arc::clone(&device);
            let probe_reads = Arc::clone(&reads);
            let fixture = ContractFixture::new(
                Arc::<FakeClock>::clone(&clock),
                "STATE/level:u16",
                "STATE/missing:u16",
                Arc::new(FixtureAddress::new("elsewhere", None)),
                move || {
                    u64::from(probe_reads.load(Ordering::Relaxed))
                        .saturating_add(probe_device.sink_writes())
                },
                move || reads.load(Ordering::Relaxed),
            )
            .with_writable("SINK/out:u16", PlcValue::U16(7))
            .with_inject_disconnect(hook(&device, FailureStep::Disconnect))
            .with_inject_timeout(hook(&device, FailureStep::Timeout))
            .with_inject_device_exception(hook(
                &device,
                FailureStep::DeviceException(stonecrab_core::ProtocolStatus::new("sim", 9)),
            ))
            .with_inject_unknown_after_transmit(hook(&device, FailureStep::UnknownAfterTransmit));
            Self {
                clock,
                driver,
                fixture,
            }
        }

        fn run(&self, uri: &str) -> ContractReport {
            let report = block_on_with(
                &self.clock,
                ContractSuite::run(&self.driver, uri, &self.fixture),
            )
            .expect("the suite always resolves");
            writeln!(std::io::stdout().lock(), "{uri}\n{report}").expect("stdout");
            report
        }
    }

    const UNSUPPORTED_WRITE: &str = "an unsupported write performs no I/O";

    fn assert_clean(report: &ContractReport) {
        assert!(report.is_green(), "the suite is green:\n{report}");
        assert_eq!(report.failed(), 0, "no failures:\n{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "every case has an outcome"
        );
        assert_eq!(report.skipped(), 1, "only one case is skipped:\n{report}");
        assert!(
            matches!(
                report.outcome(UNSUPPORTED_WRITE),
                Some(CaseOutcome::Skipped(reason)) if reason.contains("supported")
            ),
            "the sim supports write, so the unsupported-write case is skipped:\n{report}"
        );
    }

    // covers: ALPHA_PLAN S13 / exit: suite green against sim under block_on_with
    #[test]
    fn the_suite_is_green_against_the_simulator() {
        let report = Bench::new().run("sim://bench");
        assert_clean(&report);
        let names: Vec<&str> = report.cases.iter().map(|(case, _)| case.name).collect();
        assert!(
            names.contains(&"a read-only policy rejects a write with no I/O"),
            "the read-only case is in the suite and ran: {names:?}"
        );
        assert_eq!(
            report.outcome("a read-only policy rejects a write with no I/O"),
            Some(&CaseOutcome::Passed),
            "the read-only case ran against the sim:\n{report}"
        );
    }

    // covers: SPEC 'Connection and resource behavior' / lifecycle with a goodbye in progress
    #[test]
    fn the_suite_is_green_when_every_operation_takes_device_latency() {
        let report = Bench::new().run("sim://bench?latency-ms=5");
        assert_clean(&report);
    }

    #[test]
    fn a_fixture_without_hooks_skips_the_fault_cases_and_names_the_reason() {
        let bench = Bench::new();
        let clock = Arc::<FakeClock>::clone(&bench.clock);
        let bare = ContractFixture::new(
            clock,
            "STATE/level:u16",
            "STATE/missing:u16",
            Arc::new(FixtureAddress::new("elsewhere", None)),
            || 0,
            || 0,
        );
        let report = block_on_with(
            &bench.clock,
            ContractSuite::run(&bench.driver, "sim://bench", &bare),
        )
        .expect("resolves");
        assert!(
            report.is_green(),
            "the cases that need no hook pass:\n{report}"
        );
        for name in [
            "a disconnect during a read yields Disconnected",
            "a timeout carries diagnostics naming operation, state and category",
            "read retry stops after the configured maximum attempts",
        ] {
            assert!(
                matches!(
                    report.outcome(name),
                    Some(CaseOutcome::Skipped(reason)) if reason.starts_with("fixture cannot inject")
                ),
                "{name} is skipped for the missing hook:\n{report}"
            );
        }
        assert_eq!(
            report.outcome("an unknown write outcome is never replayed"),
            Some(&CaseOutcome::Skipped(
                "fixture has no writable address".to_owned()
            )),
            "the write cases name the missing address:\n{report}"
        );
    }

    #[test]
    fn a_hook_that_arms_nothing_fails_the_unknown_outcome_case_instead_of_passing() {
        // A hook that arms nothing and a probe that never moves: the write is confirmed
        // and the case must notice that it is not Unknown.
        let bench = Bench::new();
        let lying = ContractFixture::new(
            Arc::<FakeClock>::clone(&bench.clock),
            "STATE/level:u16",
            "STATE/missing:u16",
            Arc::new(FixtureAddress::new("elsewhere", None)),
            || 0,
            || 0,
        )
        .with_writable("SINK/out:u16", PlcValue::U16(7))
        .with_inject_unknown_after_transmit(|| ());
        let report = block_on_with(
            &bench.clock,
            ContractSuite::run(&bench.driver, "sim://bench", &lying),
        )
        .expect("resolves");
        assert!(
            matches!(
                report.outcome("an unknown write outcome is never replayed"),
                Some(CaseOutcome::Failed(_))
            ),
            "a hook that arms nothing and a probe that never moves are failures, not passes:\n{report}"
        );
        assert!(!report.is_green(), "the report is not green:\n{report}");
    }

    #[test]
    fn the_suite_reports_a_connection_string_the_driver_rejects_as_failures() {
        let bench = Bench::new();
        let report = bench.run("sim://nowhere");
        assert_eq!(
            report.passed(),
            0,
            "nothing can pass without a device:\n{report}"
        );
        assert!(
            report.failed() > 0,
            "the failures are reported, not raised:\n{report}"
        );
        assert!(!report.is_green(), "the report is not green");
    }

    /// Run the suite on a bench whose fixture was changed to lie, and return the reason the
    /// named case failed (the case must fail: a suite that cannot be broken proves nothing).
    fn failure_of(mut bench: Bench, name: &str, lie: impl FnOnce(&mut ContractFixture)) -> String {
        lie(&mut bench.fixture);
        let report = bench.run("sim://bench");
        assert!(
            !report.is_green(),
            "a lying fixture is not green:\n{report}"
        );
        match report.outcome(name) {
            Some(CaseOutcome::Failed(reason)) => reason.clone(),
            other => panic!("{name} must fail, got {other:?}:\n{report}"),
        }
    }

    #[test]
    fn declaring_device_time_the_simulator_does_not_provide_fails_the_source_time_case() {
        let reason = failure_of(
            Bench::new(),
            "source time is absent unless the driver declares device time",
            |fixture| fixture.declares_device_time = true,
        );
        assert!(
            reason.contains("source_time"),
            "the reason names the field: {reason}"
        );
    }

    #[test]
    fn a_probe_that_moves_on_its_own_fails_the_no_io_cases() {
        let ticks = Arc::new(AtomicU64::new(0));
        let reason = failure_of(
            Bench::new(),
            "a foreign address is rejected with no I/O",
            |fixture| {
                fixture.io_probe = Box::new(move || ticks.fetch_add(1, Ordering::Relaxed));
            },
        );
        assert!(
            reason.contains("the I/O probe"),
            "the reason names the probe: {reason}"
        );
    }

    #[test]
    fn a_read_counter_that_never_moves_fails_the_retry_case() {
        let reason = failure_of(
            Bench::new(),
            "read retry stops after the configured maximum attempts",
            |fixture| fixture.device_reads = Box::new(|| 0),
        );
        assert!(
            reason.contains("device reads for one request"),
            "the reason names the counter: {reason}"
        );
    }

    #[test]
    fn a_rejected_address_the_device_actually_reads_fails_the_partial_failure_case() {
        let reason = failure_of(
            Bench::new(),
            "a partial failure keeps the successes",
            |fixture| fixture.rejected = fixture.readable.clone(),
        );
        assert!(
            reason.contains("per-item outcomes"),
            "the reason says what differed: {reason}"
        );
    }
}
