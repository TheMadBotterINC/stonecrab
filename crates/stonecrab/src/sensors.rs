#![doc = include_str!("../docs/components/sensors.md")]
//!
//! # Loading a profile
//!
//! [`SensorProfile::from_toml`] is how an application reads a profile file.
//!
//! ```
//! use stonecrab::sensors::{SensorProfile, ValidationStatus};
//!
//! let profile = SensorProfile::from_toml(r#"
//!     id = "synthetic_example"
//!     schema_version = 1
//!     validation = "synthetic"
//!
//!     [connection]
//!     scheme = "sim"
//!
//!     [provenance]
//!     source = "Synthetic reference profile; no real device"
//! "#)?;
//! assert_eq!(profile.validation, ValidationStatus::Synthetic);
//! assert!(profile.validate().is_ok());
//! # Ok::<(), stonecrab::sensors::ProfileError>(())
//! ```

mod acquisition;
mod error;
mod ident;
mod measurement;
mod profile;
mod stale;
mod tags;
mod units;
mod validate;

pub use acquisition::Acquisition;
pub use error::ProfileError;
pub use ident::{ProfileId, SchemaVersion};
pub use measurement::{
    ByteOrderName, Measurement, MeasurementDef, Quality, QualityMapping, ScaleStep, StatusField,
    ValueRange, WordOrderName,
};
pub use profile::{
    ConnectionRequirements, DeviceApplicability, Provenance, SensorProfile, ValidationStatus,
};
pub use stale::StaleTracker;
pub use units::Unit;
