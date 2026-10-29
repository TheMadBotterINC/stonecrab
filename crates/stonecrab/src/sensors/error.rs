use std::error::Error;
use std::fmt;

use crate::{ErrorKind, PlcError};

/// Why a sensor profile could not be loaded or is not valid.
///
/// Every variant carries sizes, positions, indices and fixed field labels only. None of
/// them echoes any part of the document, because a profile may come from application
/// configuration that the library must not repeat into logs.
///
/// The first four variants come from the loader, which checks the schema only (decision
/// D17, slice S28a). The next eleven check the meaning of a loaded profile: all but `MissingByteOrder` and
/// `MissingWordOrder` come from [`SensorProfile::validate`](crate::sensors::SensorProfile::validate)
/// (slice S28b), and those two come from
/// [`SensorProfile::validate_for`](crate::sensors::SensorProfile::validate_for), which asks a
/// driver which addresses span several words. The last four come from
/// [`SensorProfile::tags`](crate::sensors::SensorProfile::tags) (slice S29), which asks a driver to
/// parse the profile's addresses; `AddressRejected` also comes from `validate_for`. Validation errors name
/// the offending measurement by its 0-based index in
/// [`SensorProfile::measurements`](crate::sensors::SensorProfile::measurements), not by its name,
/// which keeps the type `Copy`; `profile.measurements[index].name` gives the name. The
/// enum is `#[non_exhaustive]`, so later slices add variants without a breaking change.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
/// Upstream PLC4X has no sensor profile concept; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// use stonecrab::{ErrorKind, PlcError};
/// use stonecrab::sensors::{ProfileError, SchemaVersion};
///
/// let error = SchemaVersion::new(2).expect_err("only version 1 exists");
/// assert_eq!(error, ProfileError::IncompatibleSchema { found: 2 });
/// assert_eq!(PlcError::from(error).kind(), ErrorKind::InvalidConfig);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// The document is larger than [`SensorProfile::MAX_TOML_BYTES`](crate::sensors::SensorProfile::MAX_TOML_BYTES);
    /// nothing was parsed.
    Oversized {
        /// Size of the rejected document in bytes.
        len: usize,
        /// The bound that applies.
        max: usize,
    },
    /// The document is not valid TOML, or does not match the profile schema: an unknown
    /// field, a missing required field, a value of the wrong type or an invalid name.
    ///
    /// Positions are 1-based and count characters; they point at the offending text.
    /// When the parser reports no position the error says line 1, column 1.
    Syntax {
        /// 1-based line of the offending text.
        line: usize,
        /// 1-based column of the offending text.
        column: usize,
    },
    /// The document declares a schema version this library does not read.
    IncompatibleSchema {
        /// The version the document declares.
        found: u32,
    },
    /// An identifier is empty, too long or not `snake_case` ASCII.
    ///
    /// Returned by the identifier constructors. [`SensorProfile::from_toml`] reports the
    /// same failure as [`ProfileError::Syntax`], because the deserialiser wraps it.
    ///
    /// [`SensorProfile::from_toml`]: crate::sensors::SensorProfile::from_toml
    InvalidIdentifier {
        /// Which identifier was rejected, for example `"profile id"`.
        field: &'static str,
    },
    /// Two measurements share a name (validation).
    DuplicateMeasurementName {
        /// Index of the later measurement, the one that repeats the name.
        measurement: usize,
        /// Index of the first measurement that has the name.
        first: usize,
    },
    /// A measurement's unit is not in the fixed unit table ([`Unit`](crate::sensors::Unit)).
    UnknownUnit {
        /// Index of the measurement.
        measurement: usize,
    },
    /// A measurement's unit is the empty string; the dimensionless unit is `"1"`.
    EmptyUnit {
        /// Index of the measurement.
        measurement: usize,
    },
    /// A value that spans several words has no `byte_order`.
    MissingByteOrder {
        /// Index of the measurement.
        measurement: usize,
    },
    /// A value that spans several words has no `word_order`.
    MissingWordOrder {
        /// Index of the measurement.
        measurement: usize,
    },
    /// Two status fields of one measurement name the same address and bit.
    OverlappingStatusBits {
        /// Index of the measurement.
        measurement: usize,
        /// Index of the later status field, the one that repeats the address and bit.
        status_field: usize,
    },
    /// A measurement's `count` is not 1; arrays are not supported at alpha.
    UnsupportedCount {
        /// Index of the measurement.
        measurement: usize,
        /// The declared count.
        count: u32,
    },
    /// A scaling step cannot be evaluated: a zero divisor, a non-finite constant, a clamp
    /// whose `min` exceeds its `max`, or a bit field of width 0 or beyond bit 63.
    InvalidScaleStep {
        /// Index of the measurement.
        measurement: usize,
        /// Index of the step in the measurement's scaling pipeline.
        step: usize,
    },
    /// A quality mapping names a status field the measurement does not declare.
    UnknownQualityReference {
        /// Index of the measurement.
        measurement: usize,
        /// Position of the reference in `bad_when` followed by `uncertain_when`.
        entry: usize,
    },
    /// The connection declares an address convention other than `plc4x`.
    UnknownAddressConvention,
    /// The scheme is `modbus-tcp` and the connection declares no address convention.
    MissingAddressConvention,
    /// The driver's scheme is not the profile's connection scheme ([`SensorProfile::tags`]).
    ///
    /// [`SensorProfile::tags`]: crate::sensors::SensorProfile::tags
    SchemeMismatch {
        /// The scheme of the driver that was offered.
        driver: &'static str,
    },
    /// The driver rejected a measurement's address text ([`SensorProfile::tags`]). The
    /// text is not repeated; `profile.measurements[measurement].address` holds it.
    ///
    /// [`SensorProfile::tags`]: crate::sensors::SensorProfile::tags
    AddressRejected {
        /// Index of the measurement.
        measurement: usize,
    },
    /// The driver rejected a status field's address text ([`SensorProfile::tags`]).
    ///
    /// [`SensorProfile::tags`]: crate::sensors::SensorProfile::tags
    StatusAddressRejected {
        /// Index of the measurement.
        measurement: usize,
        /// Index of the status field in the measurement's `status_fields`.
        status_field: usize,
    },
    /// Two status fields of one measurement share a name, so they would share a tag
    /// identity ([`SensorProfile::tags`]); validation accepts them, tag building cannot.
    ///
    /// [`SensorProfile::tags`]: crate::sensors::SensorProfile::tags
    DuplicateStatusFieldName {
        /// Index of the measurement.
        measurement: usize,
        /// Index of the later status field, the one that repeats the name.
        status_field: usize,
    },
    /// A measurement's `range` has a bound that is NaN or infinite, or `min` above `max`
    /// (equal bounds are valid).
    InvalidRange {
        /// Index of the measurement.
        measurement: usize,
    },
}

impl fmt::Display for ProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversized { len, max } => {
                write!(
                    f,
                    "sensor profile of {len} bytes exceeds the {max} byte limit"
                )
            }
            Self::Syntax { line, column } => {
                write!(
                    f,
                    "sensor profile is invalid at line {line}, column {column}"
                )
            }
            Self::IncompatibleSchema { found } => write!(
                f,
                "sensor profile schema version {found} is not supported; this library reads version 1"
            ),
            Self::InvalidIdentifier { field } => write!(
                f,
                "invalid {field}: require 1..=64 bytes of snake_case ASCII starting with a letter"
            ),
            Self::DuplicateMeasurementName { measurement, first } => write!(
                f,
                "measurement {measurement} repeats the name of measurement {first}"
            ),
            Self::UnknownUnit { measurement } => {
                write!(
                    f,
                    "measurement {measurement} has a unit outside the unit table"
                )
            }
            Self::EmptyUnit { measurement } => {
                write!(
                    f,
                    "measurement {measurement} has an empty unit; use \"1\" for none"
                )
            }
            Self::MissingByteOrder { measurement } => write!(
                f,
                "measurement {measurement} spans several words and has no byte_order"
            ),
            Self::MissingWordOrder { measurement } => write!(
                f,
                "measurement {measurement} spans several words and has no word_order"
            ),
            Self::OverlappingStatusBits {
                measurement,
                status_field,
            } => write!(
                f,
                "status field {status_field} of measurement {measurement} repeats an address and bit"
            ),
            Self::UnsupportedCount { measurement, count } => write!(
                f,
                "measurement {measurement} has count {count}; only count 1 is supported"
            ),
            Self::InvalidScaleStep { measurement, step } => write!(
                f,
                "scaling step {step} of measurement {measurement} cannot be evaluated"
            ),
            Self::InvalidRange { measurement } => write!(
                f,
                "measurement {measurement} has a range with a non-finite bound or min above max"
            ),
            Self::UnknownQualityReference { measurement, entry } => write!(
                f,
                "quality entry {entry} of measurement {measurement} names an undeclared status field"
            ),
            Self::UnknownAddressConvention => {
                f.write_str("the address convention is not supported; the only convention is plc4x")
            }
            Self::MissingAddressConvention => {
                f.write_str("a modbus-tcp profile must declare its address_convention")
            }
            Self::SchemeMismatch { driver } => write!(
                f,
                "the profile's connection scheme is not the scheme of the {driver} driver"
            ),
            Self::AddressRejected { measurement } => {
                write!(
                    f,
                    "the driver rejected the address of measurement {measurement}"
                )
            }
            Self::StatusAddressRejected {
                measurement,
                status_field,
            } => write!(
                f,
                "the driver rejected the address of status field {status_field} of measurement {measurement}"
            ),
            Self::DuplicateStatusFieldName {
                measurement,
                status_field,
            } => write!(
                f,
                "status field {status_field} of measurement {measurement} repeats the name of an earlier status field"
            ),
        }
    }
}

impl Error for ProfileError {}

/// Every variant maps to [`ErrorKind::InvalidConfig`]: a profile is configuration. The
/// [`ProfileError`] is kept as the error's `source()`.
impl From<ProfileError> for PlcError {
    fn from(error: ProfileError) -> Self {
        Self::new(ErrorKind::InvalidConfig, error.to_string()).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_sizes_positions_and_indices_only() {
        let cases = [
            (
                ProfileError::Oversized {
                    len: 70_000,
                    max: 65_536,
                },
                "sensor profile of 70000 bytes exceeds the 65536 byte limit",
            ),
            (
                ProfileError::Syntax { line: 3, column: 9 },
                "sensor profile is invalid at line 3, column 9",
            ),
            (
                ProfileError::IncompatibleSchema { found: 2 },
                "sensor profile schema version 2 is not supported; this library reads version 1",
            ),
            (
                ProfileError::InvalidIdentifier {
                    field: "profile id",
                },
                "invalid profile id: require 1..=64 bytes of snake_case ASCII starting with a letter",
            ),
            (
                ProfileError::DuplicateMeasurementName {
                    measurement: 3,
                    first: 1,
                },
                "measurement 3 repeats the name of measurement 1",
            ),
            (
                ProfileError::UnknownUnit { measurement: 2 },
                "measurement 2 has a unit outside the unit table",
            ),
            (
                ProfileError::EmptyUnit { measurement: 2 },
                "measurement 2 has an empty unit; use \"1\" for none",
            ),
            (
                ProfileError::MissingByteOrder { measurement: 4 },
                "measurement 4 spans several words and has no byte_order",
            ),
            (
                ProfileError::MissingWordOrder { measurement: 4 },
                "measurement 4 spans several words and has no word_order",
            ),
            (
                ProfileError::OverlappingStatusBits {
                    measurement: 0,
                    status_field: 2,
                },
                "status field 2 of measurement 0 repeats an address and bit",
            ),
            (
                ProfileError::UnsupportedCount {
                    measurement: 5,
                    count: 8,
                },
                "measurement 5 has count 8; only count 1 is supported",
            ),
            (
                ProfileError::InvalidScaleStep {
                    measurement: 1,
                    step: 2,
                },
                "scaling step 2 of measurement 1 cannot be evaluated",
            ),
            (
                ProfileError::InvalidRange { measurement: 2 },
                "measurement 2 has a range with a non-finite bound or min above max",
            ),
            (
                ProfileError::UnknownQualityReference {
                    measurement: 1,
                    entry: 3,
                },
                "quality entry 3 of measurement 1 names an undeclared status field",
            ),
            (
                ProfileError::UnknownAddressConvention,
                "the address convention is not supported; the only convention is plc4x",
            ),
            (
                ProfileError::MissingAddressConvention,
                "a modbus-tcp profile must declare its address_convention",
            ),
            (
                ProfileError::SchemeMismatch { driver: "sim" },
                "the profile's connection scheme is not the scheme of the sim driver",
            ),
            (
                ProfileError::AddressRejected { measurement: 2 },
                "the driver rejected the address of measurement 2",
            ),
            (
                ProfileError::StatusAddressRejected {
                    measurement: 2,
                    status_field: 1,
                },
                "the driver rejected the address of status field 1 of measurement 2",
            ),
            (
                ProfileError::DuplicateStatusFieldName {
                    measurement: 0,
                    status_field: 1,
                },
                "status field 1 of measurement 0 repeats the name of an earlier status field",
            ),
        ];
        for (error, text) in cases {
            assert_eq!(error.to_string(), text, "display for {error:?}");
        }
    }

    #[test]
    fn conversion_is_invalid_config_and_keeps_the_source() {
        let error = ProfileError::Syntax { line: 1, column: 1 };
        let plc = PlcError::from(error);
        assert_eq!(plc.kind(), ErrorKind::InvalidConfig, "kind");
        assert_eq!(plc.to_string(), error.to_string(), "message");
        let source = Error::source(&plc).expect("source retained");
        assert_eq!(source.to_string(), error.to_string(), "source");
    }
}
