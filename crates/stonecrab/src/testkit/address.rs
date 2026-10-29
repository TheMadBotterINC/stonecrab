use crate::{PlcValueType, ProtocolAddress};

/// Deterministic address fixture; never interpreted as a real protocol address.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FixtureAddress {
    canonical: String,
    hint: Option<PlcValueType>,
    multiple_words: bool,
}

impl FixtureAddress {
    /// Retain test text and an optional fixed scalar type without parsing.
    #[must_use]
    pub fn new(canonical: &str, hint: Option<PlcValueType>) -> Self {
        Self {
            canonical: canonical.to_owned(),
            hint,
            multiple_words: false,
        }
    }

    /// Make the address declare that its value spans more than one protocol word, as a
    /// wide type of a second driver would ([`ProtocolAddress::spans_multiple_words`]).
    #[must_use]
    pub const fn spanning_multiple_words(mut self) -> Self {
        self.multiple_words = true;
        self
    }
}

impl ProtocolAddress for FixtureAddress {
    fn protocol(&self) -> &'static str {
        "fixture"
    }

    fn canonical(&self) -> String {
        self.canonical.clone()
    }

    fn value_type_hint(&self) -> Option<PlcValueType> {
        self.hint
    }

    fn spans_multiple_words(&self) -> bool {
        self.multiple_words
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_address_retains_text_and_optional_type_hint() {
        for hint in [None, Some(PlcValueType::I16)] {
            let address = FixtureAddress::new("counter", hint);
            assert_eq!(address.protocol(), "fixture", "never impersonate a driver");
            assert_eq!(address.canonical(), "counter", "text retained");
            assert_eq!(address.value_type_hint(), hint, "hint retained");
            assert_eq!(address.clone(), address, "value semantics");
            assert!(!address.spans_multiple_words(), "single word by default");
            assert!(
                address.spanning_multiple_words().spans_multiple_words(),
                "the fixture can declare a wide value"
            );
        }
    }
}
