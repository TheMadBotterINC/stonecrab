#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]

mod error;
mod ident;
mod measurement;
mod profile;
mod tags;
mod units;
mod validate;

pub use error::ProfileError;
pub use ident::{ProfileId, SchemaVersion};
pub use measurement::{
    ByteOrderName, Measurement, MeasurementDef, Quality, QualityMapping, ScaleStep, StatusField,
    ValueRange, WordOrderName,
};
pub use profile::{
    ConnectionRequirements, DeviceApplicability, Provenance, SensorProfile, ValidationStatus,
};
pub use units::Unit;
