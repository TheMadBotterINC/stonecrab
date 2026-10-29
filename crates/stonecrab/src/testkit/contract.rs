//! The driver contract suite: one set of cases that every driver must pass.
//!
//! [`ContractSuite::run`] connects through a driver, runs every case of the suite in
//! order against what the driver and its [`ContractFixture`] can do, and returns a
//! [`ContractReport`]. The suite never panics on a driver failure: every failed assertion
//! becomes [`CaseOutcome::Failed`] with a reason, so the report says which case broke. A
//! case the driver or the fixture cannot exercise is [`CaseOutcome::Skipped`] with a
//! reason, never silently omitted.

use std::fmt;
use std::sync::Arc;

use crate::{Clock, Driver, OpFuture, PlcValue, ProtocolAddress};

mod cases;
#[cfg(test)]
mod tests;

type Probe = Box<dyn Fn() -> u64 + Send + Sync>;
type Counter = Box<dyn Fn() -> u32 + Send + Sync>;
type Hook = Box<dyn Fn() + Send + Sync>;

/// A fault a fixture can arm for the next operation.
///
/// # Examples
/// ```
/// use stonecrab::testkit::Fault;
///
/// assert_eq!(Fault::Timeout.to_string(), "timeout");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fault {
    /// The link is lost during the next operation.
    Disconnect,
    /// The device's reply to the next operation never arrives.
    Timeout,
    /// The device refuses the next operation with an exception status.
    DeviceException,
    /// The next write is applied and its confirmation is lost.
    UnknownAfterTransmit,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disconnect => "disconnect",
            Self::Timeout => "timeout",
            Self::DeviceException => "device exception",
            Self::UnknownAfterTransmit => "unknown-after-transmit",
        })
    }
}

/// What a case needs before it can run; an unmet need makes the case skipped.
///
/// # Examples
/// ```
/// use stonecrab::testkit::{Fault, Requires};
///
/// assert_eq!(Requires::Fault(Fault::Disconnect).to_string(), "fixture fault: disconnect");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Requires {
    /// The driver reports `Capabilities.read`.
    Read,
    /// The driver reports `Capabilities.write`.
    Write,
    /// The driver reports that writes are unsupported.
    WriteUnsupported,
    /// The driver reports `Capabilities.ping`.
    Ping,
    /// The fixture names a writable address.
    WritableAddress,
    /// The fixture has the hook that arms this fault.
    Fault(Fault),
}

impl fmt::Display for Requires {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => f.write_str("driver supports read"),
            Self::Write => f.write_str("driver supports write"),
            Self::WriteUnsupported => f.write_str("driver reports write unsupported"),
            Self::Ping => f.write_str("driver supports ping"),
            Self::WritableAddress => f.write_str("fixture names a writable address"),
            Self::Fault(fault) => write!(f, "fixture fault: {fault}"),
        }
    }
}

/// One case of the suite: a sentence, the mandatory case it covers and what it needs.
///
/// # Examples
/// ```
/// use stonecrab::testkit::{ContractCase, Requires};
///
/// let case = ContractCase::new("a read succeeds", "F3 device", &[Requires::Read]);
/// assert_eq!(case.requires, [Requires::Read]);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ContractCase {
    /// The case as a sentence.
    pub name: &'static str,
    /// The `SPEC.md` or `ALPHA_PLAN.md` mandatory case it covers (the `covers:` mapping).
    pub covers: &'static str,
    /// What the driver and the fixture must offer; empty when the case always runs.
    pub requires: &'static [Requires],
}

impl ContractCase {
    /// A case with its sentence, coverage mapping and requirements.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::ContractCase;
    ///
    /// let case = ContractCase::new("close is terminal", "F5 shutdown", &[]);
    /// assert_eq!(case.name, "close is terminal");
    /// ```
    #[must_use]
    pub const fn new(
        name: &'static str,
        covers: &'static str,
        requires: &'static [Requires],
    ) -> Self {
        Self {
            name,
            covers,
            requires,
        }
    }
}

/// How one case ended.
///
/// # Examples
/// ```
/// use stonecrab::testkit::CaseOutcome;
///
/// assert_eq!(CaseOutcome::Skipped("fixture cannot inject timeout".to_owned()).label(), "SKIP");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaseOutcome {
    /// Every assertion held.
    Passed,
    /// An assertion failed; the text says which and what was observed.
    Failed(String),
    /// The case could not run; the text says why.
    Skipped(String),
}

impl CaseOutcome {
    /// The four-letter label the report prints: `PASS`, `FAIL` or `SKIP`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::CaseOutcome;
    ///
    /// assert_eq!(CaseOutcome::Passed.label(), "PASS");
    /// assert_eq!(CaseOutcome::Failed("why".to_owned()).label(), "FAIL");
    /// ```
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Passed => "PASS",
            Self::Failed(_) => "FAIL",
            Self::Skipped(_) => "SKIP",
        }
    }
}

/// The result of one run: every case, in order, with its outcome.
///
/// `Display` lists every case with its outcome, its reason and the case it covers.
///
/// # Examples
/// ```
/// use stonecrab::testkit::{CaseOutcome, ContractCase, ContractReport};
///
/// let case = ContractCase::new("close is terminal", "F5 shutdown", &[]);
/// let report = ContractReport::new(vec![(case, CaseOutcome::Passed)]);
/// assert!(report.is_green());
/// assert_eq!((report.passed(), report.failed(), report.skipped()), (1, 0, 0));
/// assert!(report.to_string().contains("PASS  close is terminal"));
/// ```
#[derive(Debug)]
pub struct ContractReport {
    /// Every case with its outcome, in the order the suite ran them.
    pub cases: Vec<(ContractCase, CaseOutcome)>,
}

impl ContractReport {
    /// A report of the given cases.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::ContractReport;
    ///
    /// assert!(!ContractReport::new(Vec::new()).is_green(), "an empty run proves nothing");
    /// ```
    #[must_use]
    pub const fn new(cases: Vec<(ContractCase, CaseOutcome)>) -> Self {
        Self { cases }
    }

    /// How many cases passed.
    #[must_use]
    pub fn passed(&self) -> usize {
        self.count(|outcome| matches!(outcome, CaseOutcome::Passed))
    }

    /// How many cases failed.
    #[must_use]
    pub fn failed(&self) -> usize {
        self.count(|outcome| matches!(outcome, CaseOutcome::Failed(_)))
    }

    /// How many cases were skipped.
    #[must_use]
    pub fn skipped(&self) -> usize {
        self.count(|outcome| matches!(outcome, CaseOutcome::Skipped(_)))
    }

    /// True when no case failed and at least one passed. A run that skipped everything
    /// exercised nothing and is not green.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::ContractReport;
    ///
    /// let report = ContractReport::new(Vec::new());
    /// assert_eq!(report.passed() + report.failed() + report.skipped(), 0);
    /// assert!(!report.is_green());
    /// ```
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.failed() == 0 && self.passed() > 0
    }

    /// The outcome of the case with this sentence, if the report has it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::{CaseOutcome, ContractCase, ContractReport};
    ///
    /// let case = ContractCase::new("close is terminal", "F5 shutdown", &[]);
    /// let report = ContractReport::new(vec![(case, CaseOutcome::Passed)]);
    /// assert_eq!(report.outcome("close is terminal"), Some(&CaseOutcome::Passed));
    /// assert_eq!(report.outcome("no such case"), None);
    /// ```
    #[must_use]
    pub fn outcome(&self, name: &str) -> Option<&CaseOutcome> {
        self.cases
            .iter()
            .find(|(case, _)| case.name == name)
            .map(|(_, outcome)| outcome)
    }

    fn count(&self, wanted: fn(&CaseOutcome) -> bool) -> usize {
        self.cases
            .iter()
            .filter(|(_, outcome)| wanted(outcome))
            .count()
    }
}

impl fmt::Display for ContractReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "contract suite: {} passed, {} failed, {} skipped",
            self.passed(),
            self.failed(),
            self.skipped()
        )?;
        for (case, outcome) in &self.cases {
            writeln!(
                f,
                "{}  {}  [covers: {}]",
                outcome.label(),
                case.name,
                case.covers
            )?;
            if let CaseOutcome::Failed(reason) | CaseOutcome::Skipped(reason) = outcome {
                writeln!(f, "      reason: {reason}")?;
            }
        }
        Ok(())
    }
}

/// What the suite needs from the driver under test: addresses, probes and fault hooks.
///
/// The fixture is the suite's only window into the device. `io_probe` is a counter that
/// must not move when the suite says "no I/O" (for the simulator, device reads plus sink
/// writes). `device_reads` counts reads that reached the device. A hook arms one failure
/// for the next operation that reaches the device; a hook the device cannot provide stays
/// `None` and the cases that need it are skipped with the reason
/// "fixture cannot inject `<fault>`". The `clock` must be the clock the driver's
/// connections use, and it must be deterministic (a `FakeClock`), because the suite builds
/// deadlines from it and measures the time a timeout takes.
///
/// `Debug` prints names and which hooks are present, never closures or values.
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use std::sync::atomic::{AtomicU32, Ordering};
/// use stonecrab::PlcValue;
/// use stonecrab::testkit::{ContractFixture, FakeClock, FixtureAddress};
///
/// let reads = Arc::new(AtomicU32::new(0));
/// let probe = Arc::clone(&reads);
/// let counter = Arc::clone(&reads);
/// let fixture = ContractFixture::new(
///     Arc::new(FakeClock::default()),
///     "STATE/level:u16",
///     "STATE/missing:u16",
///     Arc::new(FixtureAddress::new("elsewhere", None)),
///     move || u64::from(probe.load(Ordering::Relaxed)),
///     move || counter.load(Ordering::Relaxed),
/// )
/// .with_writable("SINK/out:u16", PlcValue::U16(1))
/// .with_inject_timeout(|| ());
/// assert!(fixture.inject_timeout.is_some() && fixture.inject_disconnect.is_none());
/// assert!(format!("{fixture:?}").contains("STATE/level:u16"));
/// ```
#[non_exhaustive]
pub struct ContractFixture {
    /// The clock the driver's connections use.
    pub clock: Arc<dyn Clock>,
    /// Address text the device reads successfully.
    pub readable: String,
    /// Address text that parses but that the device refuses to read.
    pub rejected: String,
    /// An address that is not the driver's own (for example a `FixtureAddress`).
    pub foreign: Arc<dyn ProtocolAddress>,
    /// Address text and a matching value for a write the device accepts, if any. Name it
    /// for a driver without write too: the "unsupported write performs no I/O" case and
    /// the write half of the capabilities case need it. The uri given to `run` must grant
    /// write access for a driver that supports write.
    pub writable: Option<(String, PlcValue)>,
    /// True when the driver reports device time, so `source_time` is expected.
    pub declares_device_time: bool,
    /// A counter that must not move when the suite says "no I/O".
    pub io_probe: Probe,
    /// A counter of reads that reached the device.
    pub device_reads: Counter,
    /// Arms a lost link for the next operation.
    pub inject_disconnect: Option<Hook>,
    /// Arms a lost reply for the next operation.
    pub inject_timeout: Option<Hook>,
    /// Arms a device exception for the next operation.
    pub inject_device_exception: Option<Hook>,
    /// Arms an applied write with a lost confirmation for the next write.
    pub inject_unknown_after_transmit: Option<Hook>,
}

impl ContractFixture {
    /// A fixture with the required parts and no optional ones.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::testkit::{ContractFixture, FakeClock, FixtureAddress};
    ///
    /// let fixture = ContractFixture::new(
    ///     Arc::new(FakeClock::default()),
    ///     "a",
    ///     "b",
    ///     Arc::new(FixtureAddress::new("c", None)),
    ///     || 0,
    ///     || 0,
    /// );
    /// assert_eq!((fixture.io_probe)(), 0);
    /// assert!(fixture.writable.is_none());
    /// ```
    #[must_use]
    pub fn new(
        clock: Arc<dyn Clock>,
        readable: impl Into<String>,
        rejected: impl Into<String>,
        foreign: Arc<dyn ProtocolAddress>,
        io_probe: impl Fn() -> u64 + Send + Sync + 'static,
        device_reads: impl Fn() -> u32 + Send + Sync + 'static,
    ) -> Self {
        Self {
            clock,
            readable: readable.into(),
            rejected: rejected.into(),
            foreign,
            writable: None,
            declares_device_time: false,
            io_probe: Box::new(io_probe),
            device_reads: Box::new(device_reads),
            inject_disconnect: None,
            inject_timeout: None,
            inject_device_exception: None,
            inject_unknown_after_transmit: None,
        }
    }

    /// Name an address the device accepts writes to, with a value of the right type.
    #[must_use]
    pub fn with_writable(mut self, address: impl Into<String>, value: PlcValue) -> Self {
        self.writable = Some((address.into(), value));
        self
    }

    /// Declare that the driver reports device time, so reads must carry `source_time`.
    #[must_use]
    pub const fn with_device_time(mut self) -> Self {
        self.declares_device_time = true;
        self
    }

    /// Provide the hook that arms a lost link.
    #[must_use]
    pub fn with_inject_disconnect(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.inject_disconnect = Some(Box::new(hook));
        self
    }

    /// Provide the hook that arms a lost reply.
    #[must_use]
    pub fn with_inject_timeout(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.inject_timeout = Some(Box::new(hook));
        self
    }

    /// Provide the hook that arms a device exception.
    #[must_use]
    pub fn with_inject_device_exception(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.inject_device_exception = Some(Box::new(hook));
        self
    }

    /// Provide the hook that arms an applied write with a lost confirmation.
    #[must_use]
    pub fn with_inject_unknown_after_transmit(
        mut self,
        hook: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        self.inject_unknown_after_transmit = Some(Box::new(hook));
        self
    }
}

impl fmt::Debug for ContractFixture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ContractFixture")
            .field("readable", &self.readable)
            .field("rejected", &self.rejected)
            .field("foreign_protocol", &self.foreign.protocol())
            .field("writable", &self.writable.as_ref().map(|(text, _)| text))
            .field("declares_device_time", &self.declares_device_time)
            .field("inject_disconnect", &self.inject_disconnect.is_some())
            .field("inject_timeout", &self.inject_timeout.is_some())
            .field(
                "inject_device_exception",
                &self.inject_device_exception.is_some(),
            )
            .field(
                "inject_unknown_after_transmit",
                &self.inject_unknown_after_transmit.is_some(),
            )
            .finish_non_exhaustive()
    }
}

/// The suite itself: a list of cases and the function that runs them.
///
/// The queued-cancellation case (F4 queued) is not a suite case. A queue exists only in
/// drivers that have one, so it is covered on `ConnectionGuard` (slice S19) and the
/// simulator, which has no wait queue, does not exercise it.
///
/// # Examples
/// ```
/// use stonecrab::testkit::ContractSuite;
///
/// assert!(ContractSuite::cases().len() > 20);
/// ```
#[derive(Debug, Clone, Copy)]
pub struct ContractSuite;

impl ContractSuite {
    /// Every case of the suite, in the order [`run`](Self::run) executes them.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::ContractSuite;
    ///
    /// let cases = ContractSuite::cases();
    /// assert_eq!(cases.len(), 26);
    /// let mut names: Vec<_> = cases.iter().map(|case| case.name).collect();
    /// names.sort_unstable();
    /// names.dedup();
    /// assert_eq!(names.len(), 26, "case names are unique");
    /// ```
    #[must_use]
    pub fn cases() -> Vec<ContractCase> {
        cases::Id::ALL.iter().map(|id| id.case()).collect()
    }

    /// Run every case against `driver` and report each outcome.
    ///
    /// The suite parses `uri` with `driver.parse_config` and connects with
    /// `driver.connect`, once per case (a case that leaves a connection unusable never
    /// shares it). Resolution always succeeds: a failure of the driver is a
    /// [`CaseOutcome::Failed`] entry in the report, never an `Err` or a panic. Drive the
    /// future with `block_on_with` on the fixture's clock.
    ///
    /// The `uri` must grant write access whenever the driver supports write and the
    /// fixture names a writable address: core's default access policy is read-only, so a
    /// driver whose `parse_config` does not override it (the simulator does) must be
    /// given a uri that does. It must also not raise the read retry above one attempt: the cases that need a
    /// different retry policy or access policy rebuild the parsed configuration through
    /// `ConnectionConfig::builder`, and are skipped when the configuration carries
    /// credentials, which cannot be copied.
    ///
    /// # Cancel safety
    /// Not cancel-safe in the sense of resumable: dropping the future abandons the run
    /// and leaves any fault a case armed in the device. Build a fresh device to run again.
    /// The deadline source is each case's own request, which takes `timeouts.request` of
    /// the parsed configuration unless the case sets one.
    ///
    /// # Wire effects
    /// Whatever the driver's reads, writes, connects and closes do, plus one write per
    /// write case to the fixture's writable address. Nothing is sent by the suite itself.
    ///
    /// # Examples
    /// The full run against the simulator is `crates/stonecrab/tests/sim_contract.rs`;
    /// the testkit cannot depend on the simulator, so this example is compiled but not run.
    /// ```no_run
    /// use stonecrab::Driver;
    /// use stonecrab::testkit::{ContractFixture, ContractSuite, FakeClock, block_on_with};
    ///
    /// fn check(driver: &dyn Driver, fixture: &ContractFixture, clock: &FakeClock) {
    ///     let report = block_on_with(clock, ContractSuite::run(driver, "sim://bench", fixture));
    ///     let report = report.expect("the run always resolves");
    ///     assert!(report.is_green(), "{report}");
    /// }
    /// ```
    pub fn run<'a>(
        driver: &'a dyn Driver,
        uri: &'a str,
        fixture: &'a ContractFixture,
    ) -> OpFuture<'a, ContractReport> {
        Box::pin(async move { Ok(cases::run_all(driver, uri, fixture).await) })
    }
}
