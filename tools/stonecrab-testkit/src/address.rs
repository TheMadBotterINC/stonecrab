use stonecrab_core::{PlcValueType, ProtocolAddress};

/// Deterministic address fixture; never interpreted as a real protocol address.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FixtureAddress {
    canonical: String,
    hint: Option<PlcValueType>,
}

impl FixtureAddress {
    /// Retain test text and an optional fixed scalar type without parsing.
    #[must_use]
    pub fn new(canonical: &str, hint: Option<PlcValueType>) -> Self {
        Self {
            canonical: canonical.to_owned(),
            hint,
        }
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
        }
    }
}
