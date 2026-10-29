use std::collections::BTreeMap;

use serde::Deserialize;

use crate::sensors::ProfileError;
use crate::sensors::{MeasurementDef, ProfileId, SchemaVersion};

/// The device a profile applies to, where known.
///
/// Every field is optional: a synthetic profile describes no real device. Absent fields
/// mean "unknown", not "any".
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::DeviceApplicability;
///
/// let device: DeviceApplicability = toml::from_str("model = \"M1\"")?;
/// assert_eq!(device.model.as_deref(), Some("M1"));
/// assert_eq!(device.manufacturer, None);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[non_exhaustive]
#[serde(default, deny_unknown_fields)]
pub struct DeviceApplicability {
    /// Manufacturer name as the documentation spells it.
    pub manufacturer: Option<String>,
    /// Model designation.
    pub model: Option<String>,
    /// Firmware version or range the register map was written for.
    pub firmware: Option<String>,
}

/// What a connection must look like for a profile's addresses to mean what it says.
///
/// `scheme` names the connection-string scheme whose address grammar the profile's
/// addresses use (for example `modbus-tcp` or `sim`); a profile never mixes schemes
/// (decision D17). `address_convention` is the explicit address convention (for example
/// `plc4x`): a profile declares it instead of leaving a bare number such as `40001`
/// ambiguous. Whether it is required for a scheme is checked by
/// [`SensorProfile::validate`], not by the loader.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::ConnectionRequirements;
///
/// let connection: ConnectionRequirements = toml::from_str(
///     "scheme = \"modbus-tcp\"\naddress_convention = \"plc4x\"\n[options]\ndefault-unit-identifier = \"1\"",
/// )?;
/// assert_eq!(connection.scheme, "modbus-tcp");
/// assert_eq!(connection.address_convention.as_deref(), Some("plc4x"));
/// assert_eq!(connection.options["default-unit-identifier"], "1");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ConnectionRequirements {
    /// Connection-string scheme the profile's addresses belong to.
    pub scheme: String,
    /// Explicit address convention; `None` when the profile declares none.
    #[serde(default)]
    pub address_convention: Option<String>,
    /// Connection options (the keys of the connection string's query) the profile needs.
    #[serde(default)]
    pub options: BTreeMap<String, String>,
}

/// Where a profile's content comes from.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::Provenance;
///
/// let provenance: Provenance = toml::from_str("source = \"Synthetic reference profile\"")?;
/// assert_eq!(provenance.source, "Synthetic reference profile");
/// assert!(provenance.fixtures.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// The document the register map was taken from, or a statement that none exists.
    pub source: String,
    /// Names of the retained fixtures that exercise the profile.
    #[serde(default)]
    pub fixtures: Vec<String>,
}

/// How far a profile has been checked, using the evidence labels of the specification.
///
/// The TOML values are `"synthetic"`, `"documented"`, `"fixture_backed"` and
/// `"hardware_verified"`. A profile never claims more than the evidence behind it: a
/// synthetic reference profile says `synthetic`.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::ValidationStatus;
///
/// #[derive(serde::Deserialize)]
/// struct Doc { validation: ValidationStatus }
///
/// let doc: Doc = toml::from_str(r#"validation = "fixture_backed""#)?;
/// assert_eq!(doc.validation, ValidationStatus::FixtureBacked);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ValidationStatus {
    /// Invented for tests and examples; no real device or documentation stands behind it.
    Synthetic,
    /// Written from a manufacturer's documentation, with no retained fixture.
    Documented,
    /// Documented and exercised against retained fixtures.
    FixtureBacked,
    /// Verified against a physical device, with a hardware record.
    HardwareVerified,
}

/// A declarative sensor profile: a device-independent description of measurements.
///
/// A profile maps protocol addresses to named measurements with a data type, byte and
/// word order, a scaling pipeline, a unit and quality rules. It contains no executable
/// code and is never written by this library, so the types implement `Deserialize` and
/// not `Serialize`.
///
/// [`SensorProfile::from_toml`] loads and shape-checks a profile. It does not validate the
/// profile semantically: call [`SensorProfile::validate`] for the unit table, duplicate
/// names, status-bit overlap and the rule that `modbus-tcp` profiles declare an address
/// convention.
///
/// The TOML top level holds `id`, `schema_version` (which must be 1 or 2; the
/// `value_rules` key of a measurement needs 2), `validation`, the
/// tables `[device]`, `[connection]` and `[provenance]`, and an array of
/// `[[measurements]]`. `id`, `schema_version`, `connection`, `provenance` and
/// `validation` are required; `device` and `measurements` default to empty. Unknown keys
/// anywhere are errors.
///
/// ```toml
/// id = "synthetic_temperature"
/// schema_version = 1
/// validation = "synthetic"
///
/// [connection]
/// scheme = "modbus-tcp"
/// address_convention = "plc4x"
///
/// [provenance]
/// source = "Synthetic reference profile; no real device"
///
/// [[measurements]]
/// name = "boiler_temperature"
/// address = "holding-register:1:INT"
/// data_type = "INT"
/// unit = "Cel"
/// scaling = [{ divide = 10.0 }]
/// ```
///
/// Profiles declare their address convention and there is no named device profile yet:
/// every profile that ships with Stonecrab is labeled [`ValidationStatus::Synthetic`].
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
/// Upstream PLC4X has no sensor profile concept; this is a Stonecrab type with no
/// upstream parity claim.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct SensorProfile {
    /// Stable identifier of the profile.
    pub id: ProfileId,
    /// Schema version the document is written against: 1, or 2 for a profile that uses
    /// value-matched quality rules.
    pub schema_version: SchemaVersion,
    /// The device the profile applies to, where known.
    #[serde(default)]
    pub device: DeviceApplicability,
    /// What the connection must look like.
    pub connection: ConnectionRequirements,
    /// The measurements, in document order.
    #[serde(default)]
    pub measurements: Vec<MeasurementDef>,
    /// Documentary source and fixtures.
    pub provenance: Provenance,
    /// How far the profile has been checked.
    pub validation: ValidationStatus,
}

impl SensorProfile {
    /// Largest document, in bytes, that [`SensorProfile::from_toml`] parses.
    pub const MAX_TOML_BYTES: usize = 64 * 1024;

    /// Load a profile from TOML text.
    ///
    /// The size bound is checked before any parsing. The schema version is read first, so
    /// a document written for another version is reported as such even when it also
    /// contains keys this version does not know. The whole document is then parsed with
    /// unknown keys denied. No part of the document is echoed into an error.
    ///
    /// This is the only entry point that reads a file's text; reading the file is the
    /// caller's job, so the loader performs no I/O.
    ///
    /// # Errors
    /// Returns [`ProfileError::Oversized`] for a document over
    /// [`SensorProfile::MAX_TOML_BYTES`], [`ProfileError::IncompatibleSchema`] when
    /// `schema_version` is present as an integer in the `u32` range and is not 1 or 2, and
    /// [`ProfileError::Syntax`] for invalid TOML, an unknown or missing key, a value of
    /// the wrong type (including a `schema_version` that is not an integer in the `u32`
    /// range) or an invalid identifier. Identifier failures raised while deserialising
    /// are folded into `Syntax`; [`ProfileError::InvalidIdentifier`] is returned only by
    /// the identifier constructors. All of them convert into a [`PlcError`](crate::PlcError) of
    /// kind `InvalidConfig`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::{SensorProfile, ValidationStatus};
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_temperature"
    ///     schema_version = 1
    ///     validation = "synthetic"
    ///
    ///     [connection]
    ///     scheme = "sim"
    ///
    ///     [provenance]
    ///     source = "Synthetic reference profile; no real device"
    ///
    ///     [[measurements]]
    ///     name = "boiler_temperature"
    ///     address = "holding-register:1:INT"
    ///     data_type = "INT"
    ///     unit = "Cel"
    ///     scaling = [{ divide = 10.0 }]
    /// "#)?;
    /// assert_eq!(profile.id.as_str(), "synthetic_temperature");
    /// assert_eq!(profile.validation, ValidationStatus::Synthetic);
    /// assert_eq!(profile.measurements.len(), 1);
    /// # Ok::<(), stonecrab::sensors::ProfileError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-SCHEMA-V1
    /// Upstream PLC4X has no sensor profile concept; this is a Stonecrab operation with
    /// no upstream parity claim.
    #[must_use = "load errors must be checked"]
    pub fn from_toml(text: &str) -> Result<Self, ProfileError> {
        if text.len() > Self::MAX_TOML_BYTES {
            return Err(ProfileError::Oversized {
                len: text.len(),
                max: Self::MAX_TOML_BYTES,
            });
        }
        let probe: VersionProbe = toml::from_str(text).map_err(|error| syntax(text, &error))?;
        if let Some(found) = probe.schema_version {
            SchemaVersion::new(found)?;
        }
        toml::from_str(text).map_err(|error| syntax(text, &error))
    }
}

/// Reads only `schema_version`, ignoring every other key.
#[derive(Deserialize)]
struct VersionProbe {
    schema_version: Option<u32>,
}

fn syntax(text: &str, error: &toml::de::Error) -> ProfileError {
    let offset = error.span().map_or(0, |span| span.start);
    let (line, column) = position(text, offset);
    ProfileError::Syntax { line, column }
}

/// The 1-based line and column of a byte offset, counting columns in characters.
///
/// An offset past the end or inside a character is clamped to the end of the text, and
/// the result never depends on what the text says.
fn position(text: &str, offset: usize) -> (usize, usize) {
    let before = text.get(..offset).unwrap_or(text);
    let line = before
        .bytes()
        .filter(|byte| *byte == b'\n')
        .count()
        .saturating_add(1);
    let column = before
        .chars()
        .rev()
        .take_while(|c| *c != '\n')
        .count()
        .saturating_add(1);
    (line, column)
}

#[cfg(test)]
mod tests {
    use super::position;

    #[test]
    fn positions_are_one_based_and_count_characters() {
        let text = "ab\ncd\u{e9}\nf";
        assert_eq!(position(text, 0), (1, 1), "start");
        assert_eq!(position(text, 2), (1, 3), "the newline itself");
        assert_eq!(position(text, 3), (2, 1), "start of line two");
        assert_eq!(
            position(text, 7),
            (2, 4),
            "the newline after a two-byte character"
        );
        assert_eq!(position(text, 8), (3, 1), "start of line three");
        assert_eq!(position(text, 9), (3, 2), "end of text");
    }

    #[test]
    fn an_offset_outside_the_text_is_clamped_to_the_end() {
        let text = "ab\ncd\u{e9}";
        assert_eq!(position(text, 1_000), (2, 4), "past the end");
        assert_eq!(position(text, 6), (2, 4), "inside the two-byte character");
        assert_eq!(position("", 0), (1, 1), "empty text");
    }
}
