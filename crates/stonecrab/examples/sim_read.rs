//! Read three tags from an in-process simulated device through the driver registry.
//!
//! Run it with `cargo run -p stonecrab --example sim_read --features sim`. Results leave
//! through `tracing::info!` with structured fields on a `tracing-subscriber` fmt layer
//! installed in `main` (ADR-0011); nothing is printed with `println!`.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stonecrab::sim::{SimDevice, SimDriver, SimTag};
use stonecrab::{
    AddressOptions, Deadline, DriverRegistry, PlcError, PlcValue, ReadRequest, Tag, TagId,
};

/// The addresses read, as `(tag identifier, simulator address text)`.
const ADDRESSES: [(&str, &str); 3] = [
    ("temperature", "STATE/temperature:f32"),
    ("counter", "STATE/counter:u32"),
    ("running", "STATE/running:bool"),
];

/// Build the registry, connect to `sim://bench`, read the tags and close.
async fn run() -> Result<(), PlcError> {
    let device = SimDevice::new();
    device.set("temperature", SimTag::new(PlcValue::F32(21.5)))?;
    device.ramp("counter", PlcValue::U32(100), PlcValue::U32(5))?;
    device.set("running", SimTag::new(PlcValue::Bool(true)))?;

    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(SimDriver::new().with_device("bench", device)?))?;
    let connection = registry.connect("sim://bench").await?;

    let driver = registry.driver("sim").ok_or_else(|| {
        PlcError::new(
            stonecrab::ErrorKind::InvalidConfig,
            "the sim driver is not registered",
        )
    })?;
    let options = AddressOptions::default();
    let mut request = ReadRequest::builder(connection.limits());
    for (name, text) in ADDRESSES {
        let address = driver.parse_address(text, &options)?;
        request = request.tag(Tag::new(TagId::new(name)?, address));
    }

    // The example has no clock handle. The simulator's default RealtimeClock reads
    // Instant::now(), so a deadline built from Instant::now() agrees with it.
    let started = Instant::now();
    let deadline = Deadline::after(started, Duration::from_millis(500))?;
    let response = connection.read(request.deadline(deadline).build()?).await?;

    for item in response.items() {
        // Microseconds since the example started; u128 is a native tracing field type.
        let sampled_at = item
            .sampled_at()
            .saturating_duration_since(started)
            .as_micros();
        match item.outcome() {
            Ok(value) => {
                tracing::info!(name = %item.tag(), value = ?value, outcome = "ok", sampled_at);
            }
            Err(error) => {
                tracing::error!(
                    name = %item.tag(), outcome = ?error.kind(), sampled_at, "item failed"
                );
            }
        }
    }
    connection.close().await
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
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(kind = ?error.kind(), error = %error, "sim_read failed");
            ExitCode::FAILURE
        }
    }
}
