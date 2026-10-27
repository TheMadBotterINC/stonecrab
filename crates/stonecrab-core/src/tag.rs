use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use crate::{ErrorKind, PlcError, Result};

/// Stable caller-owned correlation identifier, distinct from a protocol address.
///
/// Identifiers are nonempty UTF-8 strings, at most 256 bytes, without control
/// characters or leading/trailing whitespace. Interior spaces and Unicode are
/// retained verbatim. No normalization is performed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct TagId(Arc<str>);

impl TagId {
    /// Maximum UTF-8 byte length, checked before allocating an identifier.
    pub const MAX_BYTES: usize = 256;

    /// Validate and retain a caller's identifier.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::TagId;
    /// let tag = TagId::new("boiler_temp")?;
    /// assert_eq!(tag.as_str(), "boiler_temp");
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for an empty, oversized, control-bearing
    /// or whitespace-padded identifier. Rejected text is not included in the error.
    #[must_use = "identifier validation must be checked"]
    pub fn new(text: &str) -> Result<Self> {
        if text.is_empty()
            || text.len() > Self::MAX_BYTES
            || text.trim() != text
            || text.chars().any(char::is_control)
        {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "invalid tag identifier: require 1..=256 UTF-8 bytes without controls or edge whitespace",
            ));
        }
        Ok(Self(Arc::from(text)))
    }

    /// Borrow the original identifier without formatting or allocation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TagId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for TagId {
    type Err = PlcError;

    fn from_str(text: &str) -> Result<Self> {
        Self::new(text)
    }
}

impl TryFrom<&str> for TagId {
    type Error = PlcError;

    fn try_from(text: &str) -> Result<Self> {
        Self::new(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tag_identifiers_preserve_text_and_value_semantics() {
        for text in [
            "boiler_temp",
            "sensor α",
            &"a".repeat(TagId::MAX_BYTES),
            &"é".repeat(128),
        ] {
            let id = TagId::new(text).expect("valid identifier");
            assert_eq!(id.as_str(), text, "original bytes are retained");
            assert_eq!(id.to_string(), text, "display preserves identity");
            assert_eq!(
                id,
                text.parse::<TagId>().expect("FromStr validates"),
                "parse matches constructor"
            );
            assert_eq!(
                id,
                TagId::try_from(text).expect("TryFrom validates"),
                "conversion matches constructor"
            );
            assert_eq!(id.clone(), id, "clone preserves identity");
        }
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration
    #[test]
    fn invalid_tag_identifiers_are_rejected_without_echoing_input() {
        for text in [
            "",
            " leading",
            "trailing ",
            "tab\there",
            "line\nbreak",
            "nul\0byte",
            &"a".repeat(257),
            &"é".repeat(129),
        ] {
            let error = TagId::new(text).expect_err("invalid identifier must fail");
            assert_eq!(
                error.kind(),
                ErrorKind::InvalidConfig,
                "typed validation failure"
            );
            assert_eq!(
                error.to_string(),
                "invalid tag identifier: require 1..=256 UTF-8 bytes without controls or edge whitespace",
                "errors never echo arbitrary text"
            );
        }
    }
}
