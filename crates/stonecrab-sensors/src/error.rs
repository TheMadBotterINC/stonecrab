use std::error::Error;
use std::fmt;

use stonecrab_core::{ErrorKind, PlcError};

/// Why a sensor profile could not be loaded.
///
/// Every variant carries sizes, positions and fixed field labels only. None of them
/// echoes any part of the document, because a profile may come from application
/// configuration that the library must not repeat into logs.
///
/// The loader checks the schema only (decision D17, slice S28a). Semantic validation of a
/// parsed profile (the units table, duplicate names, a missing address convention for
/// `modbus-tcp`) arrives with slice S28b and adds variants; the enum is
/// `#[non_exhaustive]` for that reason.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
/// Upstream PLC4X has no sensor profile concept; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// use stonecrab_core::{ErrorKind, PlcError};
/// use stonecrab_sensors::{ProfileError, SchemaVersion};
///
/// let error = SchemaVersion::new(2).expect_err("only version 1 exists");
/// assert_eq!(error, ProfileError::IncompatibleSchema { found: 2 });
/// assert_eq!(PlcError::from(error).kind(), ErrorKind::InvalidConfig);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProfileError {
    /// The document is larger than [`SensorProfile::MAX_TOML_BYTES`](crate::SensorProfile::MAX_TOML_BYTES);
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
    /// [`SensorProfile::from_toml`]: crate::SensorProfile::from_toml
    InvalidIdentifier {
        /// Which identifier was rejected, for example `"profile id"`.
        field: &'static str,
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
    fn display_names_sizes_and_positions_only() {
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
