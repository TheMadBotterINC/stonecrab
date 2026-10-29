use std::time::Duration;

use crate::{ByteOrder, TagId, WordOrder};
use serde::Deserialize;
use serde::de::{Deserializer, Error as _};

use crate::sensors::{ProfileError, ProfileId};

mod pipeline;
mod scale;

pub use pipeline::{Measurement, Quality};

/// Name of the byte order inside each 16-bit word, as written in a profile.
///
/// The TOML values are `"big_endian"` and `"little_endian"`. A profile that omits the
/// key leaves the order unset; it is never inferred (SPEC "Sensor profile contract").
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::ByteOrder;
/// use stonecrab::sensors::ByteOrderName;
///
/// #[derive(serde::Deserialize)]
/// struct Doc { byte_order: ByteOrderName }
///
/// let doc: Doc = toml::from_str(r#"byte_order = "little_endian""#)?;
/// assert_eq!(ByteOrder::from(doc.byte_order), ByteOrder::LittleEndian);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum ByteOrderName {
    /// The most significant byte of a word comes first.
    BigEndian,
    /// The least significant byte of a word comes first.
    LittleEndian,
}

impl From<ByteOrderName> for ByteOrder {
    fn from(name: ByteOrderName) -> Self {
        match name {
            ByteOrderName::BigEndian => Self::BigEndian,
            ByteOrderName::LittleEndian => Self::LittleEndian,
        }
    }
}

/// Name of the order of 16-bit words inside a multi-word value, as written in a profile.
///
/// The TOML values are `"high_word_first"` and `"low_word_first"`. A profile that omits
/// the key leaves the order unset; it is never inferred from plausible-looking values.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::WordOrder;
/// use stonecrab::sensors::WordOrderName;
///
/// #[derive(serde::Deserialize)]
/// struct Doc { word_order: WordOrderName }
///
/// let doc: Doc = toml::from_str(r#"word_order = "low_word_first""#)?;
/// assert_eq!(WordOrder::from(doc.word_order), WordOrder::LowWordFirst);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum WordOrderName {
    /// The most significant 16-bit word comes first.
    HighWordFirst,
    /// The least significant 16-bit word comes first.
    LowWordFirst,
}

impl From<WordOrderName> for WordOrder {
    fn from(name: WordOrderName) -> Self {
        match name {
            WordOrderName::HighWordFirst => Self::HighWordFirst,
            WordOrderName::LowWordFirst => Self::LowWordFirst,
        }
    }
}

/// One step of a measurement's scaling pipeline.
///
/// Steps run in the order written. In TOML each step is an inline table with one key
/// naming the step; the value is the constant, or a table for the two steps that take
/// more than one number:
///
/// ```toml
/// scaling = [
///     { divide = 10.0 },
///     { offset = -40.0 },
///     { clamp = { min = -40.0, max = 215.0 } },
/// ]
/// ```
///
/// The keys are `multiply`, `divide`, `offset`, `clamp` (with `min` and `max`) and
/// `bit_field` (with `shift` and `width`, for example `{ bit_field = { shift = 4, width = 2 } }`).
///
/// The loader checks the shape only. Whether a constant is finite, whether a divisor is
/// zero, whether a clamp is ordered and whether a bit field fits in 64 bits are checked
/// by [`SensorProfile::validate`](crate::sensors::SensorProfile::validate), not by the loader.
/// Whether a step suits the data type is left to the measurement pipeline.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-MEASUREMENTS
/// Upstream PLC4X has no scaling pipeline; this is a Stonecrab type with no upstream
/// parity claim.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::ScaleStep;
///
/// assert_eq!(toml::from_str::<ScaleStep>("multiply = 0.5")?, ScaleStep::Multiply(0.5));
/// assert_eq!(toml::from_str::<ScaleStep>("divide = 10")?, ScaleStep::Divide(10.0));
/// assert_eq!(toml::from_str::<ScaleStep>("offset = -40.0")?, ScaleStep::Offset(-40.0));
/// assert_eq!(
///     toml::from_str::<ScaleStep>("clamp = { min = 0.0, max = 100.0 }")?,
///     ScaleStep::Clamp { min: 0.0, max: 100.0 },
/// );
/// assert_eq!(
///     toml::from_str::<ScaleStep>("bit_field = { shift = 4, width = 2 }")?,
///     ScaleStep::BitField { shift: 4, width: 2 },
/// );
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ScaleStep {
    /// Multiply the value by the constant.
    Multiply(f64),
    /// Divide the value by the constant.
    Divide(f64),
    /// Add the constant to the value.
    Offset(f64),
    /// Limit the value to the inclusive range `min..=max`.
    Clamp {
        /// Lower limit.
        min: f64,
        /// Upper limit.
        max: f64,
    },
    /// Extract `width` bits starting at bit `shift` (bit 0 is the least significant).
    BitField {
        /// Index of the lowest extracted bit.
        shift: u8,
        /// Number of extracted bits.
        width: u8,
    },
}

/// Inclusive plausibility range of a scaled measurement.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::ValueRange;
///
/// let range: ValueRange = toml::from_str("min = -40.0\nmax = 215.0")?;
/// assert_eq!((range.min, range.max), (-40.0, 215.0));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct ValueRange {
    /// Lowest plausible scaled value.
    pub min: f64,
    /// Highest plausible scaled value.
    pub max: f64,
}

/// One bit of a status word that qualifies a measurement.
///
/// A quality mapping ([`QualityMapping`]) refers to status fields by `name`.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::StatusField;
///
/// let field: StatusField = toml::from_str(
///     "name = \"open_circuit\"\naddress = \"holding-register:20:WORD\"\nbit = 3\ninvert = true",
/// )?;
/// assert_eq!((field.name.as_str(), field.bit, field.invert), ("open_circuit", 3, true));
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct StatusField {
    /// Name that a [`QualityMapping`] uses to refer to this field (`snake_case` ASCII).
    #[serde(deserialize_with = "deserialize_snake_case")]
    pub name: String,
    /// Protocol address text of the status word, in the grammar of the profile's driver.
    pub address: String,
    /// Index of the bit inside the status word; bit 0 is the least significant.
    pub bit: u8,
    /// When true the bit is read as set when the wire value is clear.
    #[serde(default)]
    pub invert: bool,
}

/// Which status fields degrade the quality of a measurement.
///
/// Both lists hold [`StatusField::name`] values. Whether every name refers to a declared
/// field is checked by [`SensorProfile::validate`](crate::sensors::SensorProfile::validate), not by
/// the loader.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use stonecrab::sensors::QualityMapping;
///
/// let map: QualityMapping = toml::from_str("bad_when = [\"open_circuit\"]")?;
/// assert_eq!(map.bad_when, ["open_circuit"]);
/// assert!(map.uncertain_when.is_empty());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[non_exhaustive]
#[serde(default, deny_unknown_fields)]
pub struct QualityMapping {
    /// Names of status fields that, when set, make the value bad.
    pub bad_when: Vec<String>,
    /// Names of status fields that, when set, make the value uncertain.
    pub uncertain_when: Vec<String>,
}

/// One named measurement of a sensor profile: where to read it and how to interpret it.
///
/// The TOML keys are the field names, with two exceptions: `min_interval` is written
/// `min_interval_ms` and `stale_after` is written `stale_after_ms`, both as whole
/// milliseconds. Fields without a stated default are required.
///
/// `data_type` is the driver's own type name (for the Modbus driver, one of the upper-case
/// names such as `INT` or `REAL`); the loader passes it through and does not interpret
/// it. `address` is likewise text in the grammar of the profile's connection scheme.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-MEASUREMENTS
/// Upstream PLC4X has no measurement definition; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// # fn main() -> Result<(), toml::de::Error> {
/// use std::time::Duration;
/// use stonecrab::sensors::{MeasurementDef, ScaleStep};
///
/// let measurement: MeasurementDef = toml::from_str(r#"
///     name = "boiler_temperature"
///     address = "holding-register:1:INT"
///     data_type = "INT"
///     unit = "Cel"
///     scaling = [{ divide = 10.0 }]
///     min_interval_ms = 1000
/// "#)?;
/// assert_eq!(measurement.name.as_str(), "boiler_temperature");
/// assert_eq!(measurement.count, 1);
/// assert_eq!(measurement.scaling, [ScaleStep::Divide(10.0)]);
/// assert_eq!(measurement.min_interval, Some(Duration::from_secs(1)));
/// assert!(!measurement.keep_raw);
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[non_exhaustive]
#[serde(deny_unknown_fields)]
pub struct MeasurementDef {
    /// Measurement name: `snake_case` ASCII of 1 to 64 bytes. It doubles as the
    /// [`TagId`] of the tag built from this definition.
    #[serde(deserialize_with = "deserialize_tag_name")]
    pub name: TagId,
    /// Protocol address text, in the grammar of the profile's connection scheme.
    pub address: String,
    /// The driver's type name for the value, passed through unchanged.
    pub data_type: String,
    /// Number of elements to read; defaults to 1.
    #[serde(default = "default_count")]
    pub count: u32,
    /// Byte order inside each word; `None` when the profile leaves it unset.
    #[serde(default)]
    pub byte_order: Option<ByteOrderName>,
    /// Word order inside a multi-word value; `None` when the profile leaves it unset.
    #[serde(default)]
    pub word_order: Option<WordOrderName>,
    /// Scaling pipeline, applied in the order written; empty keeps the typed value.
    #[serde(default)]
    pub scaling: Vec<ScaleStep>,
    /// Engineering unit of the scaled value, for example `"Cel"`; `None` when the unit is
    /// unknown (the TOML key is absent). An empty or blank unit is not the same as an
    /// absent one: [`SensorProfile::validate`](crate::sensors::SensorProfile::validate)
    /// refuses it.
    #[serde(default)]
    pub unit: Option<String>,
    /// Optional plausibility range of the scaled value.
    #[serde(default)]
    pub range: Option<ValueRange>,
    /// Status bits that qualify this measurement.
    #[serde(default)]
    pub status_fields: Vec<StatusField>,
    /// How the status fields map to quality; `None` when quality is not derived.
    #[serde(default)]
    pub quality_map: Option<QualityMapping>,
    /// Shortest recommended interval between acquisitions (TOML key `min_interval_ms`).
    #[serde(
        default,
        rename = "min_interval_ms",
        deserialize_with = "deserialize_millis"
    )]
    pub min_interval: Option<Duration>,
    /// Age after which the last value counts as stale (TOML key `stale_after_ms`).
    #[serde(
        default,
        rename = "stale_after_ms",
        deserialize_with = "deserialize_millis"
    )]
    pub stale_after: Option<Duration>,
    /// Keep the unscaled typed value next to the scaled one for diagnosis.
    ///
    /// This is the unscaled typed value, not wire bytes. Defaults to false.
    #[serde(default)]
    pub keep_raw: bool,
}

impl MeasurementDef {
    /// Longest measurement name in bytes.
    pub const MAX_NAME_BYTES: usize = ProfileId::MAX_BYTES;
}

const fn default_count() -> u32 {
    1
}

/// Read a string and apply the identifier rule, naming `field` and never echoing the text.
fn identifier<'de, D: Deserializer<'de>>(
    deserializer: D,
    field: &'static str,
) -> Result<String, D::Error> {
    let text = String::deserialize(deserializer)?;
    if ProfileId::is_valid(&text) {
        Ok(text)
    } else {
        Err(D::Error::custom(ProfileError::InvalidIdentifier { field }))
    }
}

fn deserialize_snake_case<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    identifier(deserializer, "status field name")
}

fn deserialize_tag_name<'de, D: Deserializer<'de>>(deserializer: D) -> Result<TagId, D::Error> {
    let text = identifier(deserializer, "measurement name")?;
    TagId::new(&text).map_err(D::Error::custom)
}

fn deserialize_millis<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Duration>, D::Error> {
    Ok(Option::<u64>::deserialize(deserializer)?.map(Duration::from_millis))
}
