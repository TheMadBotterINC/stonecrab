//! Continuously acquire a profile for a finite duration, then stop and close.
//!
//! Defaults: `sim://bench`, the synthetic temperature profile, 1000 ms polls and 5000 ms
//! duration. Override `STONECRAB_URL`, `STONECRAB_PROFILE`, `STONECRAB_INTERVAL_MS` and
//! `STONECRAB_DURATION_MS`. `--expect-good` fails on any read failure, non-good observation,
//! or missing measurement. The existing acquisition loop is driven by the caller.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::sensors::{Acquisition, Quality, SensorProfile};
use stonecrab::sim::SimDriver;
use stonecrab::transport::SystemClock;
use stonecrab::{Clock, Deadline, DriverRegistry, ErrorKind, PlcError};

#[path = "support/bench.rs"]
mod bench;

async fn run(
    profile: SensorProfile,
    url: &str,
    interval: Duration,
    duration: Duration,
) -> Result<bool, PlcError> {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
    let stop_at = Deadline::after(clock.now(), duration)?;
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(
        SimDriver::new()
            .with_clock(Arc::clone(&clock))
            .with_device("bench", bench::bench_device()?)?,
    ))?;
    registry.register(Arc::new(
        ModbusTcpDriver::new().with_clock(Arc::clone(&clock)),
    ))?;
    let (driver, config) = registry.parse(url)?;
    let options = config.address_options();
    if options.convention().is_some() && options != profile.address_options()? {
        return Err(PlcError::new(
            ErrorKind::InvalidConfig,
            "connection and profile address conventions differ",
        ));
    }
    let defined = profile.measurements.len();
    let connection = driver.connect(config).await?;
    let result = async {
        let mut acquisition = Acquisition::new(
            &profile,
            driver.as_ref(),
            connection.as_ref(),
            clock.as_ref(),
            interval,
        )?;
        poll(&mut acquisition, defined, stop_at).await
    }
    .await;
    let closed = connection.close().await;
    let good = result?;
    closed?;
    Ok(good)
}

async fn poll(
    acquisition: &mut Acquisition<'_>,
    defined: usize,
    stop_at: Deadline,
) -> Result<bool, PlcError> {
    let mut seen = BTreeSet::new();
    let mut observations = 0_u64;
    let mut not_good = 0_u64;
    let mut failures = 0_u64;
    let end = tokio::time::sleep_until(tokio::time::Instant::from_std(stop_at.instant()));
    tokio::pin!(end);
    loop {
        let event = tokio::select! {
            biased;
            () = &mut end => break,
            event = acquisition.recv() => event,
        };
        match event {
            Ok(Some(value)) => {
                seen.insert(value.tag.clone());
                observations = observations.saturating_add(1);
                if value.quality != Quality::Good {
                    not_good = not_good.saturating_add(1);
                }
                tracing::info!(name = %value.name, value = ?value.value, unit = ?value.unit,
                    quality = %value.quality, origin = ?value.origin, raw = ?value.raw,
                    received_at = ?value.received_at, source_time = ?value.source_time,
                    "profile observation");
            }
            Ok(None) => break,
            Err(error) => {
                failures = failures.saturating_add(1);
                tracing::warn!(error = %error, kind = ?error.kind(), "profile poll failed");
            }
        }
    }
    tracing::info!(
        measurements = seen.len(),
        defined,
        observations,
        not_good,
        failures,
        "acquisition complete"
    );
    Ok(seen.len() == defined && not_good == 0 && failures == 0)
}

fn milliseconds(name: &str, default: u64) -> Result<Duration, PlcError> {
    let value = match std::env::var(name) {
        Ok(value) => value.parse::<u64>().ok().filter(|value| *value > 0),
        Err(std::env::VarError::NotPresent) => Some(default),
        Err(std::env::VarError::NotUnicode(_)) => None,
    };
    value.map(Duration::from_millis).ok_or_else(|| {
        PlcError::new(
            ErrorKind::InvalidConfig,
            format!("{name} must be positive milliseconds"),
        )
    })
}

fn main() -> ExitCode {
    if tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .without_time()
        .try_init()
        .is_err()
    {
        return ExitCode::FAILURE;
    }
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    let expect_good = match arguments.as_slice() {
        [] => false,
        [argument] if argument == "--expect-good" => true,
        _ => {
            tracing::error!("only --expect-good is supported");
            return ExitCode::FAILURE;
        }
    };
    let result = (|| {
        let interval = milliseconds("STONECRAB_INTERVAL_MS", 1000)?;
        let duration = milliseconds("STONECRAB_DURATION_MS", 5000)?;
        let path = std::env::var("STONECRAB_PROFILE")
            .unwrap_or_else(|_| "profiles/synthetic/sim/temperature_transmitter.toml".to_owned());
        let text = std::fs::read_to_string(path).map_err(|error| {
            PlcError::new(
                ErrorKind::InvalidConfig,
                format!("cannot read profile: {}", error.kind()),
            )
        })?;
        let profile = SensorProfile::from_toml(&text)?;
        let url = std::env::var("STONECRAB_URL").unwrap_or_else(|_| "sim://bench".to_owned());
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    format!("cannot start runtime: {}", error.kind()),
                )
            })?;
        runtime.block_on(run(profile, &url, interval, duration))
    })();
    match result {
        Ok(good) if good || !expect_good => ExitCode::SUCCESS,
        Ok(_) => {
            tracing::error!("--expect-good: missing, non-good or lost observations");
            ExitCode::FAILURE
        }
        Err(error) => {
            tracing::error!(kind = ?error.kind(), error = %error, "profile acquisition failed");
            ExitCode::FAILURE
        }
    }
}
