//! The parse failure type and its conversion to the workspace error.

use std::error::Error;
use std::fmt;

use crate::{ErrorKind, PlcError};

/// A failure to read or write bounded bytes.
///
/// The variants carry sizes and field names only, never payload bytes, so a value can be
/// logged without leaking device data. Protocol crates convert it into their own error
/// type or into [`PlcError`] with `From`.
///
/// `InvalidChecksum` and `RecursionLimit` arrive with the serial framing and nested
/// structures that need them; the enum is `#[non_exhaustive]` for that reason.
///
/// # Examples
/// ```
/// use stonecrab::codec::{BoundedReader, ParseError};
///
/// let mut reader = BoundedReader::new(&[0x01]);
/// let error = reader.read_u8().and_then(|_| reader.read_u8()).unwrap_err();
/// assert_eq!(error, ParseError::Truncated { needed: 1, available: 0 });
/// assert_eq!(error.to_string(), "truncated input: needed 1 bytes, 0 available");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseError {
    /// Fewer bytes were available than the read needs.
    Truncated {
        /// Bytes the read needs.
        needed: usize,
        /// Bytes that were available.
        available: usize,
    },
    /// A length exceeds the maximum the caller allows, or a write would exceed the
    /// writer's capacity. Detected before any allocation or copy.
    OversizedLength {
        /// The offending length (for a write, the total length after the write).
        length: usize,
        /// The maximum that applied.
        max: usize,
    },
    /// A field holds a value that names no known variant. Raised by the protocol crates
    /// that decode enumerations; this crate only defines the shape.
    InvalidDiscriminator {
        /// Name of the field that was being decoded.
        field: &'static str,
        /// The value that was found.
        value: u64,
    },
    /// Bytes remain after the last field was read.
    TrailingBytes {
        /// Number of unread bytes.
        remaining: usize,
    },
    /// A length computation overflowed `usize`.
    Overflow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { needed, available } => {
                write!(
                    f,
                    "truncated input: needed {needed} bytes, {available} available"
                )
            }
            Self::OversizedLength { length, max } => {
                write!(f, "length {length} exceeds the maximum of {max}")
            }
            Self::InvalidDiscriminator { field, value } => {
                write!(f, "invalid {field} discriminator {value}")
            }
            Self::TrailingBytes { remaining } => {
                write!(f, "{remaining} trailing bytes after the last field")
            }
            Self::Overflow => f.write_str("length computation overflowed"),
        }
    }
}

impl Error for ParseError {}

impl From<ParseError> for PlcError {
    /// Maps [`ParseError::OversizedLength`] to [`ErrorKind::ResourceLimit`] and every
    /// other variant to [`ErrorKind::MalformedData`]. The message is the `Display` text
    /// and the `ParseError` is kept as the source.
    ///
    /// # Examples
    /// ```
    /// use std::error::Error;
    /// use stonecrab::codec::ParseError;
    /// use stonecrab::{ErrorKind, PlcError};
    ///
    /// let error = PlcError::from(ParseError::TrailingBytes { remaining: 2 });
    /// assert_eq!(error.kind(), ErrorKind::MalformedData);
    /// assert!(error.source().is_some());
    ///
    /// let error = PlcError::from(ParseError::OversizedLength { length: 9, max: 8 });
    /// assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    /// ```
    fn from(error: ParseError) -> Self {
        let kind = match error {
            ParseError::OversizedLength { .. } => ErrorKind::ResourceLimit,
            ParseError::Truncated { .. }
            | ParseError::InvalidDiscriminator { .. }
            | ParseError::TrailingBytes { .. }
            | ParseError::Overflow => ErrorKind::MalformedData,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}
