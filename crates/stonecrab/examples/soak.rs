//! Read continuously for a set time and count what happened, for the soak run of
//! `scripts/soak.sh` (ADR-0017 addendum S62).
//!
//! Run it with `cargo run -p stonecrab --example soak --features full`. It reads a small
//! fixed set of tags in a loop through the driver registry, so the soak exercises the
//! public API only, and ends after `STONECRAB_SOAK_SECONDS` (default 60). The settings are
//! environment variables, as in the other examples:
//!
//! - `STONECRAB_URL`: the connection string (default `sim://soak`, served in-process by a
//!   `SimDevice`; a `modbus-tcp://` URL reads the pymodbus interop peer's register map).
//! - `STONECRAB_SOAK_SECONDS`: how long to run.
//! - `STONECRAB_SOAK_PERIOD_MS`: the pause after each read, default 0 (read flat out).
//! - `STONECRAB_SOAK_FAULT_SECS`: against the simulator only, push one failure step (a lost
//!   link, a lost reply, an undecodable reply, in turn) every this many seconds, default 5;
//!   0 pushes none. A real peer is faulted from outside, by restarting it.
//!
//! Every second a `tracing::info!` line carries the counters `elapsed_s`, `reads` (read
//! calls), `errors` (read calls that failed or had a failed item, and failed attempts to
//! reopen) and `reconnects` (the link lost and back: a connection that went through
//! `Reconnecting` and returned to `Connected`, or one that ended `Failed` and was reopened);
//! a last line `soak finished` carries the totals. `scripts/soak.sh` samples the resident
//! memory of this process beside those lines. Nothing is printed with `println!`.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::sim::{FailureStep, SimDevice, SimDriver, SimTag};
use stonecrab::{
    ConnectionState, Deadline, DriverRegistry, ErrorKind, PlcError, PlcValue, ReadRequest, Tag,
    TagId,
};

/// The connection string used when `STONECRAB_URL` is not set.
const DEFAULT_URL: &str = "sim://soak";

/// How long one read may take, counted from the moment it is built. A lost reply of the
/// simulator waits this long, so it is also the cost of that fault.
const READ_DEADLINE: Duration = Duration::from_millis(500);

/// How often the counters are reported.
const REPORT_EVERY: Duration = Duration::from_secs(1);

/// The pause between two attempts to reopen a connection that ended.
const REOPEN_PAUSE: Duration = Duration::from_millis(500);

/// The tags read from the simulator, as `(tag identifier, address text)`.
const SIM_ADDRESSES: [(&str, &str); 4] = [
    ("temperature", "STATE/temperature:f32"),
    ("counter", "STATE/counter:u32"),
    ("level", "STATE/level:u16"),
    ("running", "STATE/running:bool"),
];

/// The tags read from the pymodbus interop peer (`fixtures/modbus/interop/register-map.toml`).
const MODBUS_ADDRESSES: [(&str, &str); 4] = [
    ("level", "holding-register:13:UINT"),
    ("pressure", "holding-register:12:INT"),
    ("flow", "holding-register:16:UDINT"),
    ("running", "coil:1:BOOL"),
];

/// What the run counted.
#[derive(Debug, Default, Clone, Copy)]
struct Counters {
    /// Read calls made.
    reads: u64,
    /// Read calls that failed or had a failed item, and failed attempts to reopen.
    errors: u64,
    /// Times the link was lost and came back.
    reconnects: u64,
}

/// The settings of a run.
struct Settings {
    url: String,
    duration: Duration,
    period: Duration,
    fault_every: Option<Duration>,
}

/// Add one to a counter without wrapping.
const fn bump(counter: &mut u64) {
    *counter = counter.saturating_add(1);
}

/// A whole number of the environment, or `default` when the variable is not set.
fn env_number(name: &str, default: u64) -> Result<u64, PlcError> {
    match std::env::var(name) {
        Ok(text) => text.parse().map_err(|error: std::num::ParseIntError| {
            PlcError::new(
                ErrorKind::InvalidConfig,
                format!("{name} must be a whole number: {error}"),
            )
        }),
        Err(_) => Ok(default),
    }
}

impl Settings {
    /// Read the settings from the environment.
    fn from_env() -> Result<Self, PlcError> {
        let faults = env_number("STONECRAB_SOAK_FAULT_SECS", 5)?;
        Ok(Self {
            url: std::env::var("STONECRAB_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned()),
            duration: Duration::from_secs(env_number("STONECRAB_SOAK_SECONDS", 60)?),
            period: Duration::from_millis(env_number("STONECRAB_SOAK_PERIOD_MS", 0)?),
            fault_every: (faults > 0).then(|| Duration::from_secs(faults)),
        })
    }
}

/// The failure step to push for the `n`th fault: the link, the reply, then the decoding.
const fn fault(n: u64) -> FailureStep {
    match n % 3 {
        0 => FailureStep::Disconnect,
        1 => FailureStep::Timeout,
        _ => FailureStep::Malformed,
    }
}

/// The simulated `soak` device: a few readable values and a counter that moves.
fn soak_device() -> Result<SimDevice, PlcError> {
    let device = SimDevice::new();
    device.set("temperature", SimTag::new(PlcValue::F32(21.5)))?;
    device.ramp("counter", PlcValue::U32(0), PlcValue::U32(1))?;
    device.set("level", SimTag::new(PlcValue::U16(42_424)))?;
    device.set("running", SimTag::new(PlcValue::Bool(true)))?;
    Ok(device)
}

/// Connect and read until the duration has passed.
async fn run(settings: &Settings) -> Result<Counters, PlcError> {
    let sim = Arc::new(SimDriver::new().with_device("soak", soak_device()?)?);
    let mut registry = DriverRegistry::new();
    registry.register(Arc::<SimDriver>::clone(&sim))?;
    registry.register(Arc::new(ModbusTcpDriver::new()))?;

    let (driver, config) = registry.parse(&settings.url)?;
    let options = config.address_options();
    let addresses = if settings.url.starts_with("sim:") {
        SIM_ADDRESSES
    } else {
        MODBUS_ADDRESSES
    };
    let mut tags = Vec::new();
    for (name, text) in addresses {
        tags.push(Tag::new(
            TagId::new(name)?,
            driver.parse_address(text, &options)?,
        ));
    }
    // Faults are pushed to the simulator only; a real peer is faulted from outside.
    let device = if settings.url.starts_with("sim:") {
        sim.device("soak")
    } else {
        None
    };

    let mut connection = registry.connect(&settings.url).await?;
    let started = Instant::now();
    let mut counters = Counters::default();
    // Times of the next report and the next fault, as time since `started`.
    let mut next_report = REPORT_EVERY;
    let mut next_fault = settings.fault_every;
    let mut faults = 0_u64;
    let mut link_down = false;

    while started.elapsed() < settings.duration {
        let elapsed = started.elapsed();
        if let (Some(device), Some(due), Some(every)) =
            (device.as_ref(), next_fault, settings.fault_every)
            && elapsed >= due
        {
            device.push_failure(fault(faults))?;
            faults = faults.saturating_add(1);
            next_fault = Some(elapsed.saturating_add(every));
        }

        let mut request = ReadRequest::builder(connection.limits());
        for tag in &tags {
            request = request.tag(tag.clone());
        }
        let request = request
            .deadline(Deadline::after(Instant::now(), READ_DEADLINE)?)
            .build()?;
        bump(&mut counters.reads);
        match connection.read(request).await {
            Ok(response) if response.iter_failed().next().is_none() => {}
            Ok(_) | Err(_) => bump(&mut counters.errors),
        }

        let state = connection.state();
        if matches!(
            state,
            ConnectionState::Failed(_) | ConnectionState::Closed | ConnectionState::Closing
        ) {
            drop(connection.close().await);
            // Reopen it, trying until it works or the run is over.
            connection = loop {
                match registry.connect(&settings.url).await {
                    Ok(next) => break next,
                    Err(_) => bump(&mut counters.errors),
                }
                if started.elapsed() >= settings.duration {
                    return Ok(counters);
                }
                tokio::time::sleep(REOPEN_PAUSE).await;
            };
            bump(&mut counters.reconnects);
            link_down = false;
        } else if state == ConnectionState::Connected {
            if link_down {
                bump(&mut counters.reconnects);
                link_down = false;
            }
        } else {
            link_down = true;
        }

        if started.elapsed() >= next_report {
            next_report = next_report.saturating_add(REPORT_EVERY);
            report("sample", started, counters);
        }
        if !settings.period.is_zero() {
            tokio::time::sleep(settings.period).await;
        }
    }
    drop(connection.close().await);
    Ok(counters)
}

/// Emit the counters as one structured line.
fn report(message: &str, started: Instant, counters: Counters) {
    tracing::info!(
        elapsed_s = started.elapsed().as_secs(),
        reads = counters.reads,
        errors = counters.errors,
        reconnects = counters.reconnects,
        "{message}"
    );
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let installed = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_writer(std::io::stdout)
        .without_time()
        .try_init();
    // Only fails when a global subscriber already exists, which a fresh process has not.
    if installed.is_err() {
        return ExitCode::FAILURE;
    }
    let started = Instant::now();
    let outcome = match Settings::from_env() {
        Ok(settings) => run(&settings).await,
        Err(error) => Err(error),
    };
    match outcome {
        Ok(counters) => {
            report("soak finished", started, counters);
            ExitCode::SUCCESS
        }
        Err(error) => {
            tracing::error!(kind = ?error.kind(), error = %error, "soak failed");
            ExitCode::FAILURE
        }
    }
}
