#![doc = include_str!("../docs/components/sim.md")]
//!
//! # Registering the simulator
//!
//! The `sim` scheme is registered by the application: build a [`DriverRegistry`](crate::DriverRegistry), register a
//! [`SimDriver`] holding the [`SimDevice`]s you want, and connect with `sim://<device>`.
//!
//! ```
//! use std::sync::Arc;
//! use stonecrab::sim::{SimDevice, SimDriver, SimTag};
//! use stonecrab::{DriverRegistry, PlcValue};
//!
//! let device = SimDevice::new();
//! device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
//! let mut registry = DriverRegistry::new();
//! registry.register(Arc::new(SimDriver::new().with_device("bench", device)?))?;
//! assert_eq!(registry.schemes(), ["sim"]);
//! # Ok::<(), stonecrab::PlcError>(())
//! ```

mod address;
mod clock;
mod connection;
mod device;
mod driver;
mod script;

pub use address::{SimAddress, SimArea};
pub use clock::RealtimeClock;
pub use connection::SimConnection;
pub use device::{SimDevice, SimTag};
pub use driver::SimDriver;
pub use script::{FailureScript, FailureStep};
