//! Read one tag from an OPC UA server through the driver registry.
//!
//! Run it with `cargo run -p stonecrab --example opcua_read --features opcua`.
//! `STONECRAB_URL` is the connection string (default `opcua://127.0.0.1:4840?security-policy=NONE`) and
//! `STONECRAB_ADDRESS` the address read (default `ns=1;s=scalar/Int32`). The result
//! leaves through `tracing::info!` with structured fields on a `tracing-subscriber` fmt
//! layer installed in `main` (ADR-0011); nothing is printed with `println!`.
//!
//! The example needs a reachable OPC UA server. The repository's CI runs it
//! through interop/open62541/interop.sh tests. The explicit None policy is unsigned and
//! unencrypted; the library default remains secure and is refused until implemented.
#![forbid(unsafe_code)]

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use stonecrab::opcua::OpcuaDriver;
use stonecrab::{Deadline, DriverRegistry, ErrorKind, PlcError, ReadRequest, Tag, TagId};

/// The connection string used when `STONECRAB_URL` is not set.
const DEFAULT_URL: &str = "opcua://127.0.0.1:4840?security-policy=NONE";

/// The address read when `STONECRAB_ADDRESS` is not set.
const DEFAULT_ADDRESS: &str = "ns=1;s=scalar/Int32";

/// How long the read may take, counted from the moment it is built.
const READ_DEADLINE: Duration = Duration::from_secs(2);

/// Parse the configuration, connect, read one tag and close.
async fn run(url: &str, address_text: &str) -> Result<(), PlcError> {
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(OpcuaDriver::new()))?;

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
            tracing::error!(kind = ?error.kind(), error = %error, "opcua_read failed");
            ExitCode::FAILURE
        }
    }
}
