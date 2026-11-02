//! Read a synthetic sensor profile through the driver registry and report its measurements.
//!
//! Run it from the repository root with
//! `cargo run -p stonecrab --example profile_read --features full`, so that the default
//! profile path resolves. `STONECRAB_URL` is the connection string (default `sim://bench`)
//! and `STONECRAB_PROFILE` the profile file (default
//! `profiles/synthetic/sim/temperature_transmitter.toml`, read from the current directory).
//! With `--expect-good` the exit status is 1 when any measurement is not of quality `Good`
//! or the profile yields fewer measurements than it defines; every measurement is still
//! reported first.
//!
//! The registry holds two drivers. `sim://bench` is served in-process by a `SimDevice`
//! whose `STATE` tags carry the values of the Modbus interop register map
//! (`fixtures/modbus/interop/register-map.toml`), so the `profiles/synthetic/sim/` profiles
//! and the `profiles/synthetic/modbus-tcp/` profiles produce the same measurements. A
//! `modbus-tcp://` URL, such as `modbus-tcp://127.0.0.1:5020?address-convention=plc4x`
//! against the pymodbus container of `interop/pymodbus/interop.sh`, needs the `modbus-tcp`
//! profile files.
//!
//! The connection comes from `STONECRAB_URL` only: the `[connection.options]` table of a
//! profile is informational and is not applied, and only the profile's address convention
//! is checked against the URL.
//!
//! The result leaves through `tracing::info!` with structured fields (`name`, `value`,
//! `unit`, `quality`, `origin`, `sampled_at` and, when the profile keeps it, `raw`) on a
//! `tracing-subscriber` fmt layer installed in `main` (ADR-0011); nothing is printed with
//! `println!`. A measurement has no `source_time` here because neither driver reports one.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::sensors::{Quality, SensorProfile};
use stonecrab::sim::SimDriver;
use stonecrab::{Deadline, DriverRegistry, ErrorKind, PlcError, ReadRequest};

#[path = "support/bench.rs"]
mod bench;

use bench::bench_device;

/// The connection string used when `STONECRAB_URL` is not set.
const DEFAULT_URL: &str = "sim://bench";

/// The profile read when `STONECRAB_PROFILE` is not set, relative to the current directory.
const DEFAULT_PROFILE: &str = "profiles/synthetic/sim/temperature_transmitter.toml";

/// How long the read may take, counted from the moment it is built.
const READ_DEADLINE: Duration = Duration::from_secs(2);

/// What a run found, for the exit status.
struct Outcome {
    /// How many measurements the profile defines.
    defined: usize,
    /// How many measurements the response covered.
    reported: usize,
    /// How many of those are not of quality `Good`.
    not_good: usize,
}

/// Load and validate the profile, connect, read its tags once, report and close.
async fn run(url: &str, profile_path: &str) -> Result<Outcome, PlcError> {
    let text = std::fs::read_to_string(profile_path).map_err(|error| {
        PlcError::new(
            ErrorKind::InvalidConfig,
            format!(
                "cannot read the profile file {profile_path}: {}",
                error.kind()
            ),
        )
    })?;
    let profile = SensorProfile::from_toml(&text)?;
    if let Err(errors) = profile.validate() {
        for error in &errors {
            tracing::error!(error = %error, "profile validation failed");
        }
        return Err(PlcError::new(
            ErrorKind::InvalidConfig,
            "the profile does not validate",
        ));
    }

    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(
        SimDriver::new().with_device("bench", bench_device()?)?,
    ))?;
    registry.register(Arc::new(ModbusTcpDriver::new()))?;

    // `connect` takes the configuration by value, so its address options are taken first. A
    // connection string that names an address convention must agree with the profile's.
    let (driver, config) = registry.parse(url)?;
    let connection_options = config.address_options();
    if connection_options.convention().is_some()
        && connection_options != profile.address_options()?
    {
        return Err(PlcError::new(
            ErrorKind::InvalidConfig,
            "the connection string and the profile name different address conventions",
        ));
    }
    let tags = profile.tags(driver.as_ref())?;
    let connection = driver.connect(config).await?;

    let mut request = ReadRequest::builder(connection.limits());
    for tag in tags {
        request = request.tag(tag);
    }
    // The drivers' clocks read Instant::now() (the simulator's RealtimeClock, the Modbus
    // SystemClock through tokio::time::Instant outside paused test time), so a deadline
    // built from Instant::now() agrees with the connection's clock.
    let started = Instant::now();
    let deadline = Deadline::after(started, READ_DEADLINE)?;
    let response = match request.deadline(deadline).build() {
        Ok(request) => connection.read(request).await,
        Err(error) => Err(error),
    };
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // Release the connection even when the read failed; the read error is reported.
            drop(connection.close().await);
            return Err(error);
        }
    };

    let measurements = profile.measurements(&response, SystemTime::now());
    let mut not_good = 0_usize;
    for measurement in &measurements {
        if measurement.quality != Quality::Good {
            not_good = not_good.saturating_add(1);
        }
        // Microseconds since the read was built; u128 is a native tracing field type.
        let sampled_at = measurement
            .received_mono
            .saturating_duration_since(started)
            .as_micros();
        // An Option of a field value records nothing when it is None, so `raw` appears only
        // when the profile keeps it.
        tracing::info!(
            name = %measurement.name,
            value = measurement.value.as_ref().map(tracing::field::debug),
            unit = measurement.unit.as_deref(),
            quality = %measurement.quality,
            origin = ?measurement.origin,
            sampled_at,
            raw = measurement.raw.as_ref().map(tracing::field::debug),
        );
    }
    tracing::info!(
        profile = %profile.id,
        measurements = measurements.len(),
        split_into = response.split_into().get(),
        "read complete"
    );
    connection.close().await?;
    Ok(Outcome {
        defined: profile.measurements.len(),
        reported: measurements.len(),
        not_good,
    })
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
    let mut expect_good = false;
    for argument in std::env::args().skip(1) {
        if argument == "--expect-good" {
            expect_good = true;
        } else {
            tracing::error!(argument = %argument, "unknown argument; the only option is --expect-good");
            return ExitCode::FAILURE;
        }
    }
    let url = std::env::var("STONECRAB_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
    let profile = std::env::var("STONECRAB_PROFILE").unwrap_or_else(|_| DEFAULT_PROFILE.to_owned());
    match run(&url, &profile).await {
        Ok(outcome) if expect_good && outcome.not_good > 0 => {
            tracing::error!(
                not_good = outcome.not_good,
                "--expect-good: a measurement is not of quality good"
            );
            ExitCode::FAILURE
        }
        Ok(outcome) if expect_good && outcome.reported != outcome.defined => {
            tracing::error!(
                defined = outcome.defined,
                reported = outcome.reported,
                "--expect-good: the response does not cover every measurement"
            );
            ExitCode::FAILURE
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(kind = ?error.kind(), error = %error, "profile_read failed");
            ExitCode::FAILURE
        }
    }
}
