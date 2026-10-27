//! Read one tag from a Modbus TCP server through the driver registry.
//!
//! Run it with `cargo run -p stonecrab --example modbus_tcp_read --features modbus`.
//! `STONECRAB_URL` is the connection string (default `modbus-tcp://127.0.0.1:5020`) and
//! `STONECRAB_ADDRESS` the address read (default `holding-register:13:UINT`). The result
//! leaves through `tracing::info!` with structured fields on a `tracing-subscriber` fmt
//! layer installed in `main` (ADR-0011); nothing is printed with `println!`.
//!
//! The example needs a reachable Modbus TCP server. The repository's CI does not run it
//! until the interoperability job exists.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::{Deadline, DriverRegistry, ErrorKind, PlcError, ReadRequest, Tag, TagId};

/// The connection string used when `STONECRAB_URL` is not set.
const DEFAULT_URL: &str = "modbus-tcp://127.0.0.1:5020";

/// The address read when `STONECRAB_ADDRESS` is not set.
const DEFAULT_ADDRESS: &str = "holding-register:13:UINT";

/// How long the read may take, counted from the moment it is built.
const READ_DEADLINE: Duration = Duration::from_secs(2);

/// Parse the configuration, connect, read one tag and close.
async fn run(url: &str, address_text: &str) -> Result<(), PlcError> {
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(ModbusTcpDriver::new()))?;

    // `connect` takes the configuration by value, so the address options are taken first.
    // They carry `address-convention=plc4x` when the connection string names it, which is
    // what lets the bare `40001` spellings parse.
    let (driver, config) = registry.parse(url)?;
    let options = config.address_options();
    let address = driver.parse_address(address_text, &options)?;
    let connection = driver.connect(config).await?;

    let tag = Tag::new(TagId::new("tag")?, address);
    // The driver's SystemClock reads tokio::time::Instant::now(), which is the ordinary
    // Instant::now() outside paused test time, so a deadline built from Instant::now()
    // agrees with the connection's clock.
    let started = Instant::now();
    let deadline = Deadline::after(started, READ_DEADLINE)?;
    let request = ReadRequest::builder(connection.limits())
        .tag(tag)
        .deadline(deadline)
        .build()?;
    let response = connection.read(request).await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            // Release the connection even when the read failed; the read error is reported.
            drop(connection.close().await);
            return Err(error);
        }
    };

    let mut failed = 0_usize;
    for item in response.items() {
        // Microseconds since the read was built; u128 is a native tracing field type.
        let sampled_at = item
            .sampled_at()
            .saturating_duration_since(started)
            .as_micros();
        match item.outcome() {
            Ok(value) => {
                tracing::info!(name = %item.tag(), value = ?value, outcome = "ok", sampled_at);
            }
            Err(error) => {
                failed = failed.saturating_add(1);
                tracing::error!(
                    name = %item.tag(), outcome = ?error.kind(), sampled_at, "item failed"
                );
            }
        }
    }
    connection.close().await?;
    if failed > 0 {
        return Err(PlcError::new(
            ErrorKind::Internal,
            "at least one item of the read failed",
        ));
    }
    Ok(())
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
    let url = std::env::var("STONECRAB_URL").unwrap_or_else(|_| DEFAULT_URL.to_owned());
    let address = std::env::var("STONECRAB_ADDRESS").unwrap_or_else(|_| DEFAULT_ADDRESS.to_owned());
    match run(&url, &address).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(kind = ?error.kind(), error = %error, "modbus_tcp_read failed");
            ExitCode::FAILURE
        }
    }
}
