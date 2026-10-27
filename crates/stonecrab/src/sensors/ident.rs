use std::fmt;

use serde::Deserialize;

use crate::sensors::ProfileError;

/// Stable identifier of a sensor profile.
///
/// An identifier is 1 to 64 bytes of `snake_case` ASCII: it starts with a lowercase letter
/// and continues with lowercase letters, digits or underscores. This is narrower than
/// [`TagId`](crate::TagId) on purpose, so a profile id is safe in file names,
/// log fields and metric labels. Rejected text is never echoed.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
/// Upstream PLC4X has no sensor profile concept; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// use stonecrab::sensors::ProfileId;
///
/// let id = ProfileId::new("synthetic_temperature")?;
/// assert_eq!(id.as_str(), "synthetic_temperature");
/// assert!(ProfileId::new("Synthetic Temperature").is_err());
/// # Ok::<(), stonecrab::sensors::ProfileError>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[non_exhaustive]
#[serde(try_from = "String")]
pub struct ProfileId(String);

impl ProfileId {
    /// Maximum length in bytes, checked before the text is stored.
    pub const MAX_BYTES: usize = 64;

    /// True when `text` is a valid identifier: 1 to [`ProfileId::MAX_BYTES`] bytes of ASCII
    /// that start with a lowercase letter and continue with lowercase letters, digits or
    /// underscores. Measurement names and status field names follow the same rule.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::ProfileId;
    /// assert!(ProfileId::is_valid("boiler_temp_2"));
    /// assert!(!ProfileId::is_valid("Boiler Temp"));
    /// ```
    #[must_use]
    pub fn is_valid(text: &str) -> bool {
        let mut bytes = text.bytes();
        text.len() <= Self::MAX_BYTES
            && matches!(bytes.next(), Some(b'a'..=b'z'))
            && bytes.all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
    }

    /// Validate and retain a profile identifier.
    ///
    /// # Errors
    /// Returns [`ProfileError::InvalidIdentifier`] for an empty, oversized or
    /// non-`snake_case` identifier. Rejected text is not included in the error.
    #[must_use = "identifier validation must be checked"]
    pub fn new(text: &str) -> Result<Self, ProfileError> {
        Self::try_from(text.to_owned())
    }

    /// Borrow the identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&str> for ProfileId {
    type Error = ProfileError;

    fn try_from(text: &str) -> Result<Self, ProfileError> {
        Self::new(text)
    }
}

impl TryFrom<String> for ProfileId {
    type Error = ProfileError;

    fn try_from(text: String) -> Result<Self, ProfileError> {
        if Self::is_valid(&text) {
            Ok(Self(text))
        } else {
            Err(ProfileError::InvalidIdentifier {
                field: "profile id",
            })
        }
    }
}

/// Version of the profile schema a document is written against.
///
/// Only version 1 exists. A document that declares any other version is rejected with
/// [`ProfileError::IncompatibleSchema`] rather than read on a best-effort basis, because a
/// profile decides how raw register values become engineering units.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
/// Upstream PLC4X has no sensor profile concept; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// use stonecrab::sensors::{ProfileError, SchemaVersion};
///
/// assert_eq!(SchemaVersion::new(1)?, SchemaVersion::CURRENT);
/// assert_eq!(SchemaVersion::CURRENT.get(), 1);
/// assert_eq!(SchemaVersion::new(2), Err(ProfileError::IncompatibleSchema { found: 2 }));
/// # Ok::<(), ProfileError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[non_exhaustive]
#[serde(try_from = "u32")]
pub struct SchemaVersion(u32);

impl SchemaVersion {
    /// The only schema version this library reads.
    pub const CURRENT: Self = Self(1);

    /// Accept version 1 and refuse every other value.
    ///
    /// # Errors
    /// Returns [`ProfileError::IncompatibleSchema`] carrying the rejected version.
    #[must_use = "version validation must be checked"]
    pub const fn new(version: u32) -> Result<Self, ProfileError> {
        if version == Self::CURRENT.0 {
            Ok(Self::CURRENT)
        } else {
            Err(ProfileError::IncompatibleSchema { found: version })
        }
    }

    /// The version number.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl fmt::Display for SchemaVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl TryFrom<u32> for SchemaVersion {
    type Error = ProfileError;

    fn try_from(version: u32) -> Result<Self, ProfileError> {
        Self::new(version)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snake_case_boundaries() {
        let longest = "a".repeat(ProfileId::MAX_BYTES);
        let too_long = "a".repeat(ProfileId::MAX_BYTES + 1);
        for text in ["a", "a1", "a_b", "a_", "temp_2_c", longest.as_str()] {
            assert!(ProfileId::is_valid(text), "accepted: {text}");
        }
        for text in [
            "",
            "A",
            "1a",
            "_a",
            "a-b",
            "a b",
            "a\u{e9}",
            "a.b",
            " a",
            "a\n",
            too_long.as_str(),
        ] {
            assert!(!ProfileId::is_valid(text), "rejected: {text:?}");
        }
    }
}
