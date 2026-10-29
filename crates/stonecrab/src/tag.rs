use std::any::Any;
use std::fmt;
use std::num::NonZeroU32;
use std::str::FromStr;
use std::sync::Arc;

use crate::{ByteOrder, ErrorKind, PlcError, PlcValueType, Result, WordOrder};

/// Driver-owned validated protocol address, independent of caller identity.
///
/// Implementors own parsing, canonical syntax and validation. The string methods
/// and Debug implementation must not expose credentials. Trait upcasting to
/// [`Any`] allows a driver to downcast only its own concrete address type.
/// Methods must be pure and return stable metadata for the address's lifetime.
pub trait ProtocolAddress: Any + fmt::Debug + Send + Sync {
    /// Stable driver protocol identifier, not the connection endpoint.
    fn protocol(&self) -> &'static str;

    /// Canonical address text without credentials.
    fn canonical(&self) -> String;

    /// Scalar type fixed by the parsed address, or `None` when not specified.
    fn value_type_hint(&self) -> Option<PlcValueType>;

    /// Whether the value at this address occupies more than one of the protocol's words.
    ///
    /// When it does, its word order must be declared and is never inferred (docs/SPEC.md);
    /// `SensorProfile::validate_for` (feature `sensors`) refuses a measurement at such an
    /// address that sets no `byte_order` or `word_order`. The default is `false`: an
    /// address is single-word unless its driver says otherwise.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{PlcValueType, ProtocolAddress};
    ///
    /// #[derive(Debug)]
    /// struct Counter;
    ///
    /// impl ProtocolAddress for Counter {
    ///     fn protocol(&self) -> &'static str {
    ///         "counter"
    ///     }
    ///     fn canonical(&self) -> String {
    ///         "counter".to_owned()
    ///     }
    ///     fn value_type_hint(&self) -> Option<PlcValueType> {
    ///         Some(PlcValueType::I16)
    ///     }
    /// }
    ///
    /// assert!(!Counter.spans_multiple_words());
    /// ```
    fn spans_multiple_words(&self) -> bool {
        false
    }
}

/// Immutable caller identity, validated address and positive element count.
///
/// Clones share an address through [`Arc`]; no parser, socket or device is invoked.
/// Byte and word order are optional per-tag settings that stay absent unless the
/// caller sets them; see [`ByteOrder`] for the precedence a driver applies.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Tag {
    id: TagId,
    address: Arc<dyn ProtocolAddress>,
    value_type: Option<PlcValueType>,
    count: NonZeroU32,
    byte_order: Option<ByteOrder>,
    word_order: Option<WordOrder>,
}

impl Tag {
    /// Retain a validated driver address, using its type hint and a count of one.
    ///
    /// Address implementations must satisfy [`ProtocolAddress`]'s validation
    /// contract before reaching this constructor.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ProtocolAddress, Tag, TagId};
    /// fn tag_address(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<Tag> {
    ///     Ok(Tag::new(TagId::new("counter")?, address))
    /// }
    /// ```
    #[must_use]
    pub fn new(id: TagId, address: Arc<dyn ProtocolAddress>) -> Self {
        let value_type = address.value_type_hint();
        Self {
            id,
            address,
            value_type,
            count: NonZeroU32::MIN,
            byte_order: None,
            word_order: None,
        }
    }

    /// Select a requested type without contradicting an address's explicit type.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when a driver-supplied type hint
    /// conflicts with the requested type.
    #[must_use = "requested type validation must be checked"]
    pub fn with_type(mut self, value_type: PlcValueType) -> Result<Self> {
        if self
            .address
            .value_type_hint()
            .is_some_and(|hint| hint != value_type)
        {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "requested value type conflicts with protocol address",
            ));
        }
        self.value_type = Some(value_type);
        Ok(self)
    }

    /// Set a positive number of elements; protocol-specific range limits are
    /// validated by the driver before transmission.
    #[must_use]
    pub const fn with_count(mut self, count: NonZeroU32) -> Self {
        self.count = count;
        self
    }

    /// Set the byte order within each protocol word for this tag.
    ///
    /// Infallible and returning `Self`, unlike [`Tag::with_type`], because core has no
    /// address-specific rule to contradict; a driver that reads an explicit order from
    /// its address syntax applies the precedence documented on [`ByteOrder`] (this
    /// field first) and validates conflicts itself. The order is never inferred.
    ///
    /// The example below is compile-checked; the executed assertions live in
    /// `tests/tag_order.rs`, because core has no in-crate address fixture.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ByteOrder, ProtocolAddress, Tag, TagId};
    /// fn little(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<Tag> {
    ///     let tag = Tag::new(TagId::new("counter")?, address)
    ///         .with_byte_order(ByteOrder::LittleEndian);
    ///     assert_eq!(tag.byte_order(), Some(ByteOrder::LittleEndian));
    ///     Ok(tag)
    /// }
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
    /// The tag field is the highest-precedence source of the upstream byte order.
    #[must_use]
    pub const fn with_byte_order(mut self, byte_order: ByteOrder) -> Self {
        self.byte_order = Some(byte_order);
        self
    }

    /// Set the order of 16-bit words within a multi-word value for this tag.
    ///
    /// Infallible and returning `Self` for the same reason as [`Tag::with_byte_order`].
    /// The order is never inferred.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ProtocolAddress, Tag, TagId, WordOrder};
    /// fn low_first(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<Tag> {
    ///     let tag = Tag::new(TagId::new("counter")?, address)
    ///         .with_word_order(WordOrder::LowWordFirst);
    ///     assert_eq!(tag.word_order(), Some(WordOrder::LowWordFirst));
    ///     Ok(tag)
    /// }
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
    /// The tag field is the highest-precedence source of the upstream byte order.
    #[must_use]
    pub const fn with_word_order(mut self, word_order: WordOrder) -> Self {
        self.word_order = Some(word_order);
        self
    }

    /// Caller-supplied stable correlation identity.
    #[must_use]
    pub const fn id(&self) -> &TagId {
        &self.id
    }

    /// Borrow the original validated driver address.
    #[must_use]
    pub fn address(&self) -> &dyn ProtocolAddress {
        self.address.as_ref()
    }

    /// Explicit requested type or the address hint, absent when neither exists.
    #[must_use]
    pub const fn value_type(&self) -> Option<PlcValueType> {
        self.value_type
    }

    /// Positive number of requested elements.
    #[must_use]
    pub const fn count(&self) -> NonZeroU32 {
        self.count
    }

    /// Explicit byte order set on this tag, `None` when the caller set none.
    ///
    /// `None` means "defer to the address block, then the driver default"; it is not
    /// a big-endian default.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ProtocolAddress, Tag, TagId};
    /// fn unset(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<()> {
    ///     let tag = Tag::new(TagId::new("counter")?, address);
    ///     assert_eq!(tag.byte_order(), None);
    ///     Ok(())
    /// }
    /// ```
    #[must_use]
    pub const fn byte_order(&self) -> Option<ByteOrder> {
        self.byte_order
    }

    /// Explicit word order set on this tag, `None` when the caller set none.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ProtocolAddress, Tag, TagId};
    /// fn unset(address: Arc<dyn ProtocolAddress>) -> stonecrab::Result<()> {
    ///     let tag = Tag::new(TagId::new("counter")?, address);
    ///     assert_eq!(tag.word_order(), None);
    ///     Ok(())
    /// }
    /// ```
    #[must_use]
    pub const fn word_order(&self) -> Option<WordOrder> {
        self.word_order
    }
}

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
    /// use stonecrab::TagId;
    /// let tag = TagId::new("boiler_temp")?;
    /// assert_eq!(tag.as_str(), "boiler_temp");
    /// # Ok::<(), stonecrab::PlcError>(())
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
