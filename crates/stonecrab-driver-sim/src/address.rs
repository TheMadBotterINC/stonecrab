//! The simulator's address text grammar: `AREA/<name>:<type>`.
//!
//! The scanner never indexes, slices with `[]`, casts or does unchecked arithmetic, and
//! every rejection is an [`ErrorKind::InvalidAddress`] whose message names the reason and
//! echoes at most a bounded excerpt of the input (control characters are rejected before
//! anything is echoed).

use std::fmt;

use stonecrab_core::{ErrorKind, PlcError, PlcValueType, ProtocolAddress, Result, TagId};

/// Longest address text `SimAddress::parse` looks at, in bytes.
const MAX_TEXT_BYTES: usize = 256;

/// Longest unrecognised piece of the input echoed back in an error message, in characters.
const ECHO_LIMIT: usize = 32;

/// The eleven scalar types the grammar accepts, by lowercase name.
const SCALARS: [(&str, PlcValueType); 11] = [
    ("bool", PlcValueType::Bool),
    ("i8", PlcValueType::I8),
    ("i16", PlcValueType::I16),
    ("i32", PlcValueType::I32),
    ("i64", PlcValueType::I64),
    ("u8", PlcValueType::U8),
    ("u16", PlcValueType::U16),
    ("u32", PlcValueType::U32),
    ("u64", PlcValueType::U64),
    ("f32", PlcValueType::F32),
    ("f64", PlcValueType::F64),
];

/// The three areas, in the order the grammar documents them.
const AREAS: [SimArea; 3] = [SimArea::State, SimArea::Random, SimArea::Sink];

/// Which behavior the simulator gives an address: a stored value, a random value or a sink.
///
/// Upstream's simulated driver names the three areas `STATE`, `RANDOM` and `STDOUT`.
/// Stonecrab renames the last one `SINK`: library code may not print (the print macros are
/// denied, coding standards section 14), so a write to it is counted and discarded rather than
/// written to standard output.
///
/// # Examples
/// ```
/// use stonecrab_driver_sim::SimArea;
///
/// assert_eq!(SimArea::Sink.name(), "SINK");
/// ```
///
/// # Compatibility: docs/parity/sim.md#SIM-ADDRESS-GRAMMAR
/// The `STDOUT` to `SINK` rename is recorded as a divergence in that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum SimArea {
    /// A named value held by the device: read back after it was set or written.
    State,
    /// A fresh value of the requested type on every read, from the device's seeded
    /// generator. Not writable.
    Random,
    /// A write-only area: writes are counted and discarded. Not readable.
    Sink,
}

impl SimArea {
    /// The upper-case text of the area in an address, `STATE`, `RANDOM` or `SINK`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_driver_sim::SimArea;
    ///
    /// assert_eq!(SimArea::State.name(), "STATE");
    /// assert_eq!(SimArea::Random.name(), "RANDOM");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::State => "STATE",
            Self::Random => "RANDOM",
            Self::Sink => "SINK",
        }
    }
}

/// A validated simulator address: an area, a tag name and a scalar type.
///
/// The text form is `STATE|RANDOM|SINK/<name>:<type>`, case-sensitive. `<name>` obeys the
/// [`TagId`] rules (1 to 256 bytes, no control characters, no leading or trailing
/// whitespace) and may itself contain `/` and `:`: the area ends at the first `/` and the
/// type begins after the last `:`. `<type>` is one of the lowercase scalar names `bool`,
/// `i8`, `i16`, `i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32` and `f64`. The whole text
/// is at most [`SimAddress::MAX_TEXT_BYTES`] (256) bytes, so through `parse` a name is at
/// most 247 bytes (`STATE/` and `:i8` take 9); [`SimAddress::new`] allows the full 256.
/// Examples are `STATE/boiler:f32` and `RANDOM/noise:u16`.
///
/// Upstream's simulated driver spells types in upper case (`INT`, `REAL`), accepts
/// `STDOUT` and documents a legacy array form in its Javadoc. Stonecrab uses the lowercase
/// Rust scalar names, renames `STDOUT` to [`SimArea::Sink`] and rejects the array form
/// (`STATE/name:i16[4]`) with a message that says so. This is not a Modbus address:
/// nothing here models registers or function codes.
///
/// `canonical()` renders the same text, so `parse(canonical(a))` equals `a`.
///
/// # Examples
/// ```
/// use stonecrab_core::{PlcValueType, ProtocolAddress};
/// use stonecrab_driver_sim::{SimAddress, SimArea};
///
/// let address = SimAddress::parse("STATE/boiler temp:f32")?;
/// assert_eq!(address.area(), SimArea::State);
/// assert_eq!(address.name(), "boiler temp");
/// assert_eq!(address.value_type(), PlcValueType::F32);
/// assert_eq!(address.protocol(), "sim");
/// assert_eq!(address.canonical(), "STATE/boiler temp:f32");
/// assert_eq!(address.value_type_hint(), Some(PlcValueType::F32));
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/sim.md#SIM-ADDRESS-GRAMMAR
/// Shape mirrors the upstream simulated driver's `AREA/name:TYPE`; the lowercase names,
/// the `SINK` rename and the rejected array form are divergences recorded in that row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimAddress {
    area: SimArea,
    name: TagId,
    value_type: PlcValueType,
    type_name: &'static str,
}

impl SimAddress {
    /// Longest address text accepted by [`parse`](Self::parse), in bytes.
    pub const MAX_TEXT_BYTES: usize = MAX_TEXT_BYTES;

    /// Assemble an address from its parts, with the same validation as the text form.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] when `name` is not a valid [`TagId`] (empty,
    /// over 256 bytes, control characters or edge whitespace) or when `value_type` is not
    /// one of the eleven scalar types of the grammar (`Null`, `Char`, `String` and
    /// `Duration` have no simulator spelling). The name is not echoed.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::{ErrorKind, PlcValueType};
    /// use stonecrab_driver_sim::{SimAddress, SimArea};
    ///
    /// let address = SimAddress::new(SimArea::Random, "noise", PlcValueType::U16)?;
    /// assert_eq!(address.to_string(), "RANDOM/noise:u16");
    ///
    /// let refused = SimAddress::new(SimArea::State, "label", PlcValueType::String);
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-ADDRESS-GRAMMAR
    /// The scalar subset is a Stonecrab choice; upstream accepts its full type list.
    pub fn new(area: SimArea, name: &str, value_type: PlcValueType) -> Result<Self> {
        let Some(&(type_name, _)) = SCALARS.iter().find(|(_, scalar)| *scalar == value_type) else {
            return Err(invalid_address(
                "the type has no simulator spelling: use bool, i8, i16, i32, i64, u8, u16, \
                 u32, u64, f32 or f64"
                    .to_owned(),
            ));
        };
        let name = TagId::new(name).map_err(|source| {
            invalid_address(
                "invalid name: require 1..=256 UTF-8 bytes without control characters or \
                 edge whitespace"
                    .to_owned(),
            )
            .with_source(source)
        })?;
        Ok(Self {
            area,
            name,
            value_type,
            type_name,
        })
    }

    /// Parse the text form `STATE|RANDOM|SINK/<name>:<type>`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`], with a message naming the reason, for:
    /// empty input; input over [`MAX_TEXT_BYTES`](Self::MAX_TEXT_BYTES) bytes; a control
    /// character anywhere; a missing `/`; an area other than `STATE`, `RANDOM` or `SINK`
    /// (including upstream's `STDOUT`); a missing `:<type>`; a type other than the eleven
    /// lowercase scalar names (upper-case upstream spellings included); the legacy array
    /// form (`:i16[4]`); and a name that is not a valid [`TagId`] (empty, control
    /// characters, edge whitespace).
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ErrorKind;
    /// use stonecrab_driver_sim::SimAddress;
    ///
    /// assert!(SimAddress::parse("SINK/log:bool").is_ok());
    /// for refused in ["", "STATE", "STATE/x", "STDOUT/x:i16", "STATE/x:INT", "STATE/x:i16[4]"] {
    ///     assert_eq!(
    ///         SimAddress::parse(refused).map_err(|e| e.kind()),
    ///         Err(ErrorKind::InvalidAddress),
    ///         "{refused:?}"
    ///     );
    /// }
    /// ```
    ///
    /// # Compatibility: docs/parity/sim.md#SIM-ADDRESS-GRAMMAR
    /// Divergences from the upstream grammar (lowercase types, `SINK`, no array form,
    /// 256-byte bound) are listed in that row.
    pub fn parse(text: &str) -> Result<Self> {
        check_text(text)?;
        let Some((area_text, rest)) = text.split_once('/') else {
            return Err(invalid_address(
                "address needs the form AREA/name:type, for example STATE/boiler:f32; the '/' \
                 after the area is missing"
                    .to_owned(),
            ));
        };
        let Some(&area) = AREAS.iter().find(|area| area.name() == area_text) else {
            return Err(invalid_address(format!(
                "unknown area '{}': use STATE, RANDOM or SINK (upstream's STDOUT is renamed \
                 SINK)",
                excerpt(area_text)
            )));
        };
        let Some((name, type_text)) = rest.rsplit_once(':') else {
            return Err(invalid_address(
                "address needs a ':type' after the name, for example STATE/boiler:f32".to_owned(),
            ));
        };
        if type_text.contains('[') {
            return Err(invalid_address(
                "array types such as ':i16[4]' are not accepted; the simulator addresses one \
                 scalar per name"
                    .to_owned(),
            ));
        }
        let Some(&(_, value_type)) = SCALARS.iter().find(|(spelling, _)| *spelling == type_text)
        else {
            return Err(invalid_address(format!(
                "unknown type '{}': use the lowercase names bool, i8, i16, i32, i64, u8, u16, \
                 u32, u64, f32 or f64",
                excerpt(type_text)
            )));
        };
        Self::new(area, name, value_type)
    }

    /// The area of the address.
    #[must_use]
    pub const fn area(&self) -> SimArea {
        self.area
    }

    /// The tag name, without the area or the type.
    #[must_use]
    pub fn name(&self) -> &str {
        self.name.as_str()
    }

    /// The scalar type the address names.
    #[must_use]
    pub const fn value_type(&self) -> PlcValueType {
        self.value_type
    }
}

impl ProtocolAddress for SimAddress {
    fn protocol(&self) -> &'static str {
        "sim"
    }

    fn canonical(&self) -> String {
        self.to_string()
    }

    fn value_type_hint(&self) -> Option<PlcValueType> {
        Some(self.value_type)
    }
}

impl fmt::Display for SimAddress {
    /// The canonical text, the same as [`ProtocolAddress::canonical`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}:{}", self.area.name(), self.name, self.type_name)
    }
}

fn invalid_address(message: String) -> PlcError {
    PlcError::new(ErrorKind::InvalidAddress, message)
}

/// A bounded prefix of an unrecognised piece of the (control-free) input.
fn excerpt(text: &str) -> String {
    text.chars().take(ECHO_LIMIT).collect()
}

/// Empty, overlong and control-bearing input are rejected before any scanning.
fn check_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(invalid_address("address is empty".to_owned()));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(invalid_address(format!(
            "address is {} bytes; at most {MAX_TEXT_BYTES} are accepted",
            text.len()
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid_address(
            "address contains a control character".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(text: &str) -> Option<ErrorKind> {
        SimAddress::parse(text).err().map(|error| error.kind())
    }

    fn message(text: &str) -> String {
        SimAddress::parse(text)
            .err()
            .map(|error| error.to_string())
            .unwrap_or_default()
    }

    // covers: SIM-ADDRESS-GRAMMAR, CODING_STANDARDS 13.7 (valid forms)
    #[test]
    fn every_area_and_type_parses_and_renders_back() {
        for area in AREAS {
            for (spelling, value_type) in SCALARS {
                let text = format!("{}/tag:{spelling}", area.name());
                let address = SimAddress::parse(&text).ok();
                assert_eq!(
                    address.as_ref().map(SimAddress::area),
                    Some(area),
                    "{text}: area"
                );
                assert_eq!(
                    address.as_ref().map(SimAddress::value_type),
                    Some(value_type),
                    "{text}: type"
                );
                assert_eq!(
                    address.as_ref().map(SimAddress::canonical),
                    Some(text.clone()),
                    "{text}: canonical"
                );
                assert_eq!(
                    address.as_ref().map(ToString::to_string),
                    Some(text.clone()),
                    "{text}: display"
                );
            }
        }
    }

    #[test]
    fn the_address_reports_the_sim_protocol_and_its_type_hint() {
        let address = SimAddress::parse("STATE/a:u8").ok();
        assert_eq!(
            address.as_ref().map(SimAddress::protocol),
            Some("sim"),
            "protocol"
        );
        assert_eq!(
            address.as_ref().and_then(SimAddress::value_type_hint),
            Some(PlcValueType::U8),
            "hint"
        );
        assert_eq!(address.as_ref().map(SimAddress::name), Some("a"), "name");
    }

    #[test]
    fn a_name_may_contain_slashes_colons_and_interior_spaces() {
        let address = SimAddress::parse("STATE/line 1/zone:a:u32").ok();
        assert_eq!(
            address.as_ref().map(SimAddress::name),
            Some("line 1/zone:a"),
            "the area ends at the first '/', the type starts after the last ':'"
        );
        let canonical = address
            .as_ref()
            .map(SimAddress::canonical)
            .unwrap_or_default();
        assert_eq!(SimAddress::parse(&canonical).ok(), address, "round trip");
    }

    // covers: CODING_STANDARDS 13.7 (zero length)
    #[test]
    fn empty_input_is_rejected() {
        assert_eq!(kind(""), Some(ErrorKind::InvalidAddress), "empty");
        assert!(message("").contains("empty"), "message names the reason");
    }

    // covers: CODING_STANDARDS 13.7 (missing discriminator)
    #[test]
    fn a_missing_slash_is_rejected() {
        assert_eq!(kind("STATE"), Some(ErrorKind::InvalidAddress), "area only");
        assert_eq!(
            kind("STATE:i16"),
            Some(ErrorKind::InvalidAddress),
            "colon but no slash"
        );
        assert!(
            message("STATE:i16").contains("'/'"),
            "message names the missing slash"
        );
    }

    // covers: CODING_STANDARDS 13.7 (missing discriminator)
    #[test]
    fn a_missing_type_is_rejected() {
        assert_eq!(
            kind("STATE/boiler"),
            Some(ErrorKind::InvalidAddress),
            "no colon"
        );
        assert!(
            message("STATE/boiler").contains("':type'"),
            "message names the missing type"
        );
        assert_eq!(
            kind("STATE/boiler:"),
            Some(ErrorKind::InvalidAddress),
            "empty type"
        );
    }

    // covers: CODING_STANDARDS 13.7 (every invalid discriminator)
    #[test]
    fn unknown_areas_are_rejected_including_upstream_stdout_and_lowercase() {
        for text in [
            "STDOUT/x:i16",
            "state/x:i16",
            "Random/x:i16",
            "/x:i16",
            "OTHER/x:i16",
        ] {
            assert_eq!(kind(text), Some(ErrorKind::InvalidAddress), "{text}");
        }
        assert!(
            message("STDOUT/x:i16").contains("renamed SINK"),
            "the message points at the rename"
        );
    }

    // covers: CODING_STANDARDS 13.7 (every invalid discriminator)
    #[test]
    fn unknown_and_upper_case_types_are_rejected() {
        for text in [
            "STATE/x:INT",
            "STATE/x:REAL",
            "STATE/x:I16",
            "STATE/x:string",
            "STATE/x:char",
            "STATE/x:i128",
            "STATE/x:f16",
            "STATE/x: i16",
        ] {
            assert_eq!(kind(text), Some(ErrorKind::InvalidAddress), "{text}");
        }
        assert!(
            message("STATE/x:INT").contains("lowercase"),
            "message says lowercase"
        );
    }

    // covers: SIM-ADDRESS-GRAMMAR (legacy array form is not accepted)
    #[test]
    fn the_legacy_array_form_is_rejected_with_its_own_message() {
        for text in ["STATE/x:i16[4]", "RANDOM/x:u8[2]", "STATE/x:i16[]"] {
            assert_eq!(kind(text), Some(ErrorKind::InvalidAddress), "{text}");
            assert!(
                message(text).contains("array"),
                "{text}: message names the array form"
            );
        }
    }

    // covers: CODING_STANDARDS 13.7 (maximum-plus-one length of the name)
    #[test]
    fn a_name_over_the_tag_id_bound_is_rejected() {
        let name = "n".repeat(TagId::MAX_BYTES + 1);
        assert_eq!(
            SimAddress::new(SimArea::State, &name, PlcValueType::I16)
                .err()
                .map(|e| e.kind()),
            Some(ErrorKind::InvalidAddress),
            "the constructor applies the TagId bound"
        );
        assert_eq!(
            kind(&format!("STATE/{name}:i16")),
            Some(ErrorKind::InvalidAddress),
            "the text form rejects it too"
        );
    }

    // covers: CODING_STANDARDS 13.7 (length one byte short, one byte long, maximum plus one)
    #[test]
    fn the_text_bound_is_exact() {
        // "STATE/" + name + ":i16" is 10 bytes plus the name.
        let at_limit = format!("STATE/{}:i16", "n".repeat(MAX_TEXT_BYTES - 10));
        assert_eq!(at_limit.len(), MAX_TEXT_BYTES, "constructed at the bound");
        assert!(
            SimAddress::parse(&at_limit).is_ok(),
            "exactly 256 bytes is accepted"
        );
        let short = format!("STATE/{}:i16", "n".repeat(MAX_TEXT_BYTES - 11));
        assert!(
            SimAddress::parse(&short).is_ok(),
            "one byte short is accepted"
        );
        let long = format!("STATE/{}:i16", "n".repeat(MAX_TEXT_BYTES - 9));
        assert_eq!(long.len(), MAX_TEXT_BYTES + 1, "constructed one byte over");
        assert_eq!(
            kind(&long),
            Some(ErrorKind::InvalidAddress),
            "257 bytes is rejected"
        );
        assert!(
            message(&long).contains("257 bytes"),
            "message states the length"
        );
        let huge = "x".repeat(100_000);
        assert_eq!(
            kind(&huge),
            Some(ErrorKind::InvalidAddress),
            "very long input is rejected"
        );
    }

    // covers: CODING_STANDARDS 13.7 (one byte short and one byte long of a minimal form)
    #[test]
    fn the_minimal_form_is_exact() {
        assert!(SimAddress::parse("STATE/a:u8").is_ok(), "minimal form");
        assert_eq!(
            kind("STATE/a:u"),
            Some(ErrorKind::InvalidAddress),
            "type one byte short"
        );
        assert_eq!(
            kind("STATE/:u8"),
            Some(ErrorKind::InvalidAddress),
            "name one byte short"
        );
        assert_eq!(
            kind("STATE/a:u8x"),
            Some(ErrorKind::InvalidAddress),
            "type one byte long"
        );
        assert_eq!(
            kind("STATE/a:u88"),
            Some(ErrorKind::InvalidAddress),
            "type digit long"
        );
        assert_eq!(
            kind("STATE/a:u8:"),
            Some(ErrorKind::InvalidAddress),
            "trailing colon"
        );
        assert_eq!(
            kind("STATE//a:u8:i8"),
            None,
            "a slash and colons in the name are valid"
        );
    }

    // covers: CODING_STANDARDS 13.7 (control characters)
    #[test]
    fn control_characters_are_rejected_anywhere() {
        for text in [
            "STATE/a\n:u8",
            "STATE/a:u8\n",
            "\tSTATE/a:u8",
            "STA\0TE/a:u8",
            "STATE/a\u{7f}:u8",
            "STATE/a\u{85}:u8",
        ] {
            assert_eq!(kind(text), Some(ErrorKind::InvalidAddress), "{text:?}");
        }
        assert!(
            message("STATE/a\n:u8").contains("control"),
            "message names control characters"
        );
    }

    // covers: CODING_STANDARDS 13.7 (name rules shared with TagId)
    #[test]
    fn edge_whitespace_and_empty_names_are_rejected_by_the_tag_id_rules() {
        for text in ["STATE/ a:u8", "STATE/a :u8", "STATE/:u8", "STATE/ :u8"] {
            assert_eq!(kind(text), Some(ErrorKind::InvalidAddress), "{text:?}");
        }
        assert!(
            message("STATE/ a:u8").contains("name"),
            "message names the name"
        );
        assert!(
            SimAddress::parse("STATE/a b:u8").is_ok(),
            "interior space is allowed"
        );
        assert!(
            SimAddress::parse("STATE/\u{e9}t\u{e9}:f64").is_ok(),
            "non-ASCII is allowed"
        );
    }

    #[test]
    fn the_name_error_keeps_the_tag_id_failure_as_its_source() {
        let error = SimAddress::parse("STATE/ a:u8").err();
        let source = error.as_ref().and_then(std::error::Error::source);
        assert!(source.is_some(), "the TagId failure is kept as the source");
    }

    #[test]
    fn errors_echo_a_bounded_excerpt_only() {
        let long_area = "A".repeat(200);
        let rendered = message(&format!("{long_area}/x:u8"));
        assert!(
            rendered.contains(&"A".repeat(ECHO_LIMIT)),
            "excerpt is echoed"
        );
        assert!(
            !rendered.contains(&"A".repeat(ECHO_LIMIT + 1)),
            "but only {ECHO_LIMIT} characters"
        );
        let long_type = message(&format!("STATE/x:{}", "t".repeat(100)));
        assert!(
            !long_type.contains(&"t".repeat(ECHO_LIMIT + 1)),
            "the type excerpt is bounded"
        );
    }

    #[test]
    fn new_refuses_types_without_a_spelling_and_accepts_every_scalar() {
        for refused in [
            PlcValueType::Null,
            PlcValueType::Char,
            PlcValueType::String,
            PlcValueType::Duration,
        ] {
            assert_eq!(
                SimAddress::new(SimArea::State, "x", refused)
                    .err()
                    .map(|e| e.kind()),
                Some(ErrorKind::InvalidAddress),
                "non-scalar type is refused"
            );
        }
        for (spelling, value_type) in SCALARS {
            let address = SimAddress::new(SimArea::Sink, "x", value_type).ok();
            assert_eq!(
                address.as_ref().map(SimAddress::canonical),
                Some(format!("SINK/x:{spelling}")),
                "{spelling}"
            );
        }
    }

    #[test]
    fn areas_are_named_and_ordered() {
        assert_eq!(
            AREAS.map(SimArea::name),
            ["STATE", "RANDOM", "SINK"],
            "names in documentation order"
        );
        assert!(
            SimArea::State < SimArea::Random && SimArea::Random < SimArea::Sink,
            "order"
        );
    }
}
