use std::collections::BTreeMap;

use crate::Driver;
use crate::sensors::{
    ConnectionRequirements, MeasurementDef, ProfileError, ScaleStep, SensorProfile, StatusField,
    Unit,
};
use crate::{AddressOptions, PlcValueType};

/// The address conventions a profile may declare: the names of the Modbus conventions
/// (`stonecrab::modbus::AddressConvention`; this module does not depend on the `modbus`
/// feature).
const CONVENTIONS: [&str; 3] = ["plc4x", "one-based-40001", "zero-based"];

/// The connection scheme whose profiles must declare an address convention (decision D17).
const MODBUS_TCP_SCHEME: &str = "modbus-tcp";

/// Largest bit position plus one that a `BitField` step or a status bit may reach (values
/// are 64 bits).
const VALUE_BITS: u16 = 64;

impl SensorProfile {
    /// Check the meaning of a loaded profile and report every problem at once.
    ///
    /// [`SensorProfile::from_toml`] checks the shape of a document; this checks what it
    /// says. The check never stops at the first problem: the `Err` holds every error, in
    /// a fixed order, so an author fixes a profile in one pass. The order is the
    /// connection first, then each measurement in document order, and within a
    /// measurement the fields in the order of [`MeasurementDef`]: name, `count`, scaling
    /// steps, unit, status fields, quality mapping. A measurement is named in an error by
    /// its 0-based index; errors never contain any text of the profile.
    ///
    /// Rules:
    /// - the address convention, when set, is `plc4x`, `one-based-40001` or `zero-based`,
    ///   and a `modbus-tcp` connection must set it (`UnknownAddressConvention`,
    ///   `MissingAddressConvention`);
    /// - measurement names are unique (`DuplicateMeasurementName`; the later one is
    ///   reported). Names are `snake_case` already, because the loader refuses anything
    ///   else;
    /// - `count` is 1 (`UnsupportedCount`);
    /// - every scaling step can be evaluated: constants are finite, a divisor is not
    ///   zero, a clamp has `min <= max`, a bit field has a width of at least 1 and
    ///   `shift + width <= 64` (`InvalidScaleStep`);
    /// - the unit is not empty and is in the [`Unit`] table (`EmptyUnit`, `UnknownUnit`);
    /// - a range, when present, has finite bounds and `min <= max`; equal bounds are valid
    ///   (`InvalidRange`);
    /// - no two status fields of one measurement name the same address text and bit
    ///   (`OverlappingStatusBits`; two measurements may share a status word), and no
    ///   status bit is 64 or more (`StatusBitOutOfRange`);
    /// - every name in the quality mapping is a status field of the same measurement
    ///   (`UnknownQualityReference`).
    ///
    /// Uniqueness of status-field names within one measurement is not enforced: a quality
    /// reference resolves to any status field of that name, so duplicates are ambiguous
    /// but accepted.
    ///
    /// Validation is independent of any driver. Whether `address` and `data_type` are
    /// meaningful to the driver, and whether status fields name real addresses, is the
    /// driver's `parse_address` to say; this method only compares address text for
    /// equality and bounds a status bit by 64. The rules that need the parsed address live
    /// in [`SensorProfile::validate_for`], which [`SensorProfile::tags`] runs: a value
    /// spanning several words needs a declared `byte_order` and `word_order`, a status
    /// bit lies inside the width of its data, and no bit is claimed twice under
    /// different spellings.
    ///
    /// # Errors
    /// Returns every [`ProfileError`] found, never an empty list: the validation variants
    /// are `DuplicateMeasurementName`, `UnknownUnit`, `EmptyUnit`,
    /// `OverlappingStatusBits`, `StatusBitOutOfRange`, `UnsupportedCount`,
    /// `InvalidScaleStep`, `InvalidRange`, `UnknownQualityReference`,
    /// `UnknownAddressConvention` and `MissingAddressConvention`. Each converts into a
    /// [`PlcError`](crate::PlcError) of kind `InvalidConfig`.
    ///
    /// # Examples
    /// ```
    /// # fn main() -> Result<(), stonecrab::sensors::ProfileError> {
    /// use stonecrab::sensors::{ProfileError, SensorProfile};
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_levels"
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
    ///     name = "tank_level"
    ///     address = "a"
    ///     data_type = "INT"
    ///     unit = "metres"
    ///     count = 2
    /// "#)?;
    /// assert_eq!(
    ///     profile.validate(),
    ///     Err(vec![
    ///         ProfileError::UnsupportedCount { measurement: 0, count: 2 },
    ///         ProfileError::UnknownUnit { measurement: 0 },
    ///     ]),
    /// );
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-VALIDATION
    /// Upstream PLC4X has no sensor profile concept; this is a Stonecrab operation with
    /// no upstream parity claim.
    #[must_use = "validation errors must be checked"]
    pub fn validate(&self) -> Result<(), Vec<ProfileError>> {
        let mut errors = Vec::new();
        check_connection(&self.connection, &mut errors);
        let mut first_with_name: BTreeMap<&str, usize> = BTreeMap::new();
        for (index, measurement) in self.measurements.iter().enumerate() {
            let first = *first_with_name
                .entry(measurement.name.as_str())
                .or_insert(index);
            if first != index {
                errors.push(ProfileError::DuplicateMeasurementName {
                    measurement: index,
                    first,
                });
            }
            check_measurement(index, measurement, &mut errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Check the meaning of a loaded profile against the driver that will read it.
    ///
    /// This runs every rule of [`SensorProfile::validate`], then parses each measurement's
    /// address and then its status field addresses with `driver` under
    /// [`SensorProfile::address_options`], exactly as [`SensorProfile::tags`] does, and
    /// appends, measurement by measurement in document order:
    /// - `AddressRejected` when the driver refuses the address;
    /// - `MissingByteOrder` and `MissingWordOrder` when the parsed address
    ///   [spans several words](crate::ProtocolAddress::spans_multiple_words) and the
    ///   definition does not set that order. Word order is never inferred (docs/SPEC.md),
    ///   and the driver, not the `data_type` text, says which values span words;
    /// - for each status field in order, `StatusAddressRejected` when the driver refuses
    ///   its address; otherwise `StatusBitOutOfRange` when the bit is at or beyond the
    ///   width of the parsed address's value type (1 for a bit, 8, 16, 32 or 64 for an
    ///   integer; a type with no such width is not checked), `OverlappingStatusBits` when
    ///   an earlier field has the same bit at an address with the same
    ///   [canonical](crate::ProtocolAddress::canonical) text but different source text
    ///   (`4x00001:WORD` and `holding-register:1:WORD`), and `StatusBitInBitField` when
    ///   the measurement's first scaling step is a `BitField` whose
    ///   `[shift, shift + width)` range holds the bit and the status address has the
    ///   measurement's canonical address.
    ///
    /// The errors of `validate` come first, so an overlap of two spellings is listed after
    /// an overlap of equal text even when its field is earlier. Addresses are compared as
    /// canonical text, so two spellings collide but one register
    /// read under two data types (`holding-register:1:INT` and `holding-register:1:WORD`)
    /// does not, and neither do two unit ids. When the declared address convention is
    /// unusable the addresses are not parsed, and `validate`'s `UnknownAddressConvention`
    /// stands alone. The connection scheme is not compared with the driver's, so pass the
    /// driver the profile names; `tags` checks the scheme first.
    ///
    /// # Errors
    /// Returns every [`ProfileError`] found, never an empty list: those of
    /// [`SensorProfile::validate`] plus `AddressRejected`, `StatusAddressRejected`,
    /// `MissingByteOrder`, `MissingWordOrder`, `StatusBitOutOfRange`,
    /// `OverlappingStatusBits` and `StatusBitInBitField`. Each converts into a
    /// [`PlcError`](crate::PlcError) of kind `InvalidConfig`.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "modbus")]
    /// # fn main() -> Result<(), stonecrab::sensors::ProfileError> {
    /// use stonecrab::modbus::ModbusTcpDriver;
    /// use stonecrab::sensors::{ProfileError, SensorProfile};
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_flow"
    ///     schema_version = 1
    ///     validation = "synthetic"
    ///
    ///     [connection]
    ///     scheme = "modbus-tcp"
    ///     address_convention = "plc4x"
    ///
    ///     [provenance]
    ///     source = "Synthetic reference profile; no real device"
    ///
    ///     [[measurements]]
    ///     name = "flow"
    ///     address = "holding-register:1:REAL"
    ///     data_type = "REAL"
    ///     unit = "L"
    /// "#)?;
    /// assert_eq!(profile.validate(), Ok(()));
    /// assert_eq!(
    ///     profile.validate_for(&ModbusTcpDriver::new()),
    ///     Err(vec![
    ///         ProfileError::MissingByteOrder { measurement: 0 },
    ///         ProfileError::MissingWordOrder { measurement: 0 },
    ///     ]),
    /// );
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "modbus"))]
    /// # fn main() {}
    /// ```
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-VALIDATION
    /// Upstream PLC4X has no sensor profile concept; this is a Stonecrab operation with
    /// no upstream parity claim.
    #[must_use = "validation errors must be checked"]
    pub fn validate_for(&self, driver: &dyn Driver) -> Result<(), Vec<ProfileError>> {
        let mut errors = self.validate().err().unwrap_or_default();
        if let Ok(options) = self.address_options() {
            for (index, measurement) in self.measurements.iter().enumerate() {
                let value = measurement.parse_address(index, driver, &options);
                match &value {
                    Ok(address) if address.spans_multiple_words() => {
                        if measurement.byte_order.is_none() {
                            errors.push(ProfileError::MissingByteOrder { measurement: index });
                        }
                        if measurement.word_order.is_none() {
                            errors.push(ProfileError::MissingWordOrder { measurement: index });
                        }
                    }
                    Ok(_) => {}
                    Err(error) => errors.push(*error),
                }
                let value = value.ok().map(|address| address.canonical());
                check_status_fields(
                    index,
                    measurement,
                    value.as_deref(),
                    driver,
                    &options,
                    &mut errors,
                );
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn check_connection(connection: &ConnectionRequirements, errors: &mut Vec<ProfileError>) {
    match connection.address_convention.as_deref() {
        Some(name) if CONVENTIONS.contains(&name) => {}
        Some(_) => errors.push(ProfileError::UnknownAddressConvention),
        None if connection.scheme == MODBUS_TCP_SCHEME => {
            errors.push(ProfileError::MissingAddressConvention);
        }
        None => {}
    }
}

fn check_measurement(index: usize, measurement: &MeasurementDef, errors: &mut Vec<ProfileError>) {
    if measurement.count != 1 {
        errors.push(ProfileError::UnsupportedCount {
            measurement: index,
            count: measurement.count,
        });
    }
    for (step, scale) in measurement.scaling.iter().enumerate() {
        if !step_is_valid(scale) {
            errors.push(ProfileError::InvalidScaleStep {
                measurement: index,
                step,
            });
        }
    }
    if measurement.unit.is_empty() {
        errors.push(ProfileError::EmptyUnit { measurement: index });
    } else if !Unit::is_known(&measurement.unit) {
        errors.push(ProfileError::UnknownUnit { measurement: index });
    }
    if let Some(range) = &measurement.range
        && !(range.min.is_finite() && range.max.is_finite() && range.min <= range.max)
    {
        errors.push(ProfileError::InvalidRange { measurement: index });
    }
    for (position, field) in measurement.status_fields.iter().enumerate() {
        if u16::from(field.bit) >= VALUE_BITS {
            errors.push(ProfileError::StatusBitOutOfRange {
                measurement: index,
                status_field: position,
            });
        }
        if repeats_earlier_text(&measurement.status_fields, position) {
            errors.push(ProfileError::OverlappingStatusBits {
                measurement: index,
                status_field: position,
            });
        }
    }
    if let Some(map) = &measurement.quality_map {
        let references = map.bad_when.iter().chain(&map.uncertain_when);
        for (entry, name) in references.enumerate() {
            if !measurement
                .status_fields
                .iter()
                .any(|field| field.name == *name)
            {
                errors.push(ProfileError::UnknownQualityReference {
                    measurement: index,
                    entry,
                });
            }
        }
    }
}

/// True when an earlier status field has the address text and the bit of field `position`.
fn repeats_earlier_text(fields: &[StatusField], position: usize) -> bool {
    fields.get(position).is_some_and(|field| {
        fields
            .iter()
            .take(position)
            .any(|earlier| earlier.address == field.address && earlier.bit == field.bit)
    })
}

/// The status-field rules that need the parsed addresses (`validate_for`). `value` is the
/// canonical text of the measurement's own address, when the driver accepted it.
fn check_status_fields(
    index: usize,
    measurement: &MeasurementDef,
    value: Option<&str>,
    driver: &dyn Driver,
    options: &AddressOptions,
    errors: &mut Vec<ProfileError>,
) {
    let extracted = match measurement.scaling.first() {
        Some(&ScaleStep::BitField { shift, width }) => {
            u16::from(shift)..u16::from(shift).saturating_add(u16::from(width))
        }
        _ => 0..0,
    };
    let mut claimed: Vec<(String, u8)> = Vec::new();
    for (position, field) in measurement.status_fields.iter().enumerate() {
        let address = match field.parse_address(index, position, driver, options) {
            Ok(address) => address,
            Err(error) => {
                errors.push(error);
                continue;
            }
        };
        let canonical = address.canonical();
        let narrow = address
            .value_type_hint()
            .and_then(bit_width)
            .is_some_and(|width| field.bit >= width);
        if narrow && u16::from(field.bit) < VALUE_BITS {
            errors.push(ProfileError::StatusBitOutOfRange {
                measurement: index,
                status_field: position,
            });
        }
        if !repeats_earlier_text(&measurement.status_fields, position)
            && claimed.contains(&(canonical.clone(), field.bit))
        {
            errors.push(ProfileError::OverlappingStatusBits {
                measurement: index,
                status_field: position,
            });
        }
        if value == Some(canonical.as_str()) && extracted.contains(&u16::from(field.bit)) {
            errors.push(ProfileError::StatusBitInBitField {
                measurement: index,
                status_field: position,
            });
        }
        claimed.push((canonical, field.bit));
    }
}

/// The number of bits a status word of this type has; `None` for a type that is not a bit
/// or an integer.
const fn bit_width(value_type: PlcValueType) -> Option<u8> {
    match value_type {
        PlcValueType::Bool => Some(1),
        PlcValueType::I8 | PlcValueType::U8 => Some(8),
        PlcValueType::I16 | PlcValueType::U16 => Some(16),
        PlcValueType::I32 | PlcValueType::U32 => Some(32),
        PlcValueType::I64 | PlcValueType::U64 => Some(64),
        PlcValueType::Null
        | PlcValueType::F32
        | PlcValueType::F64
        | PlcValueType::Char
        | PlcValueType::String
        | PlcValueType::Duration => None,
    }
}

/// True when the step can be evaluated: finite constants, a non-zero divisor, an ordered
/// clamp and a bit field that fits in 64 bits.
fn step_is_valid(step: &ScaleStep) -> bool {
    match *step {
        ScaleStep::Multiply(constant) | ScaleStep::Offset(constant) => constant.is_finite(),
        ScaleStep::Divide(divisor) => divisor.is_finite() && divisor != 0.0,
        ScaleStep::Clamp { min, max } => min.is_finite() && max.is_finite() && min <= max,
        ScaleStep::BitField { shift, width } => {
            width != 0 && u16::from(shift).saturating_add(u16::from(width)) <= VALUE_BITS
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use crate::TagId;
    use proptest::prelude::*;

    use super::*;
    use crate::sensors::{
        ByteOrderName, DeviceApplicability, ProfileId, Provenance, QualityMapping, SchemaVersion,
        StatusField, ValidationStatus, ValueRange, WordOrderName,
    };

    fn arb_step() -> impl Strategy<Value = ScaleStep> {
        let constant = || {
            prop_oneof![
                proptest::num::f64::ANY,
                Just(0.0),
                Just(-0.0),
                Just(f64::NAN),
                Just(f64::INFINITY),
                -10.0..10.0f64,
            ]
        };
        prop_oneof![
            constant().prop_map(ScaleStep::Multiply),
            constant().prop_map(ScaleStep::Divide),
            constant().prop_map(ScaleStep::Offset),
            (constant(), constant()).prop_map(|(min, max)| ScaleStep::Clamp { min, max }),
            (any::<u8>(), 0..70u8).prop_map(|(shift, width)| ScaleStep::BitField { shift, width }),
        ]
    }

    fn arb_status_field() -> impl Strategy<Value = StatusField> {
        (
            prop::sample::select(vec!["s0", "s1", "s2"]),
            prop::sample::select(vec!["w1", "w2"]),
            prop_oneof![0..3u8, 62..66u8, Just(255u8)],
            any::<bool>(),
        )
            .prop_map(|(name, address, bit, invert)| StatusField {
                name: name.to_owned(),
                address: address.to_owned(),
                bit,
                invert,
            })
    }

    fn arb_quality() -> impl Strategy<Value = Option<QualityMapping>> {
        let names = || prop::collection::vec(prop::sample::select(vec!["s0", "s1", "s9"]), 0..3);
        prop::option::of(
            (names(), names()).prop_map(|(bad, uncertain)| QualityMapping {
                bad_when: bad.into_iter().map(str::to_owned).collect(),
                uncertain_when: uncertain.into_iter().map(str::to_owned).collect(),
            }),
        )
    }

    fn arb_measurement() -> impl Strategy<Value = MeasurementDef> {
        let head = (
            prop::sample::select(vec!["m0", "m1", "m2"]),
            prop::sample::select(vec!["INT", "WORD", "REAL", "DINT", "LREAL", "Other", ""]),
            0..3u32,
            prop::option::of(Just(ByteOrderName::BigEndian)),
            prop::option::of(Just(WordOrderName::LowWordFirst)),
        );
        let bound = || {
            prop_oneof![
                -10.0..10.0f64,
                Just(f64::NAN),
                Just(f64::INFINITY),
                Just(f64::NEG_INFINITY),
            ]
        };
        let tail = (
            prop::collection::vec(arb_step(), 0..4),
            prop::sample::select(vec!["Cel", "kPa", "1", "", "furlong"]),
            prop::option::of((bound(), bound())),
            prop::collection::vec(arb_status_field(), 0..4),
            arb_quality(),
        );
        (head, tail).prop_map(
            |(
                (name, data_type, count, byte_order, word_order),
                (scaling, unit, bounds, status, map),
            )| {
                MeasurementDef {
                    name: TagId::new(name).expect("valid tag id"),
                    address: "a".to_owned(),
                    data_type: data_type.to_owned(),
                    count,
                    byte_order,
                    word_order,
                    scaling,
                    unit: unit.to_owned(),
                    range: bounds.map(|(min, max)| ValueRange { min, max }),
                    status_fields: status,
                    quality_map: map,
                    min_interval: None,
                    stale_after: None,
                    keep_raw: false,
                }
            },
        )
    }

    fn arb_profile() -> impl Strategy<Value = SensorProfile> {
        (
            prop::sample::select(vec!["modbus-tcp", "sim", ""]),
            prop::option::of(prop::sample::select(vec![
                "plc4x",
                "zero-based",
                "one-based-40001",
                "other",
                "",
            ])),
            prop::collection::vec(arb_measurement(), 0..5),
        )
            .prop_map(|(scheme, convention, measurements)| SensorProfile {
                id: ProfileId::new("generated").expect("valid id"),
                schema_version: SchemaVersion::CURRENT,
                device: DeviceApplicability::default(),
                connection: ConnectionRequirements {
                    scheme: scheme.to_owned(),
                    address_convention: convention.map(str::to_owned),
                    options: BTreeMap::new(),
                },
                measurements,
                provenance: Provenance {
                    source: "generated".to_owned(),
                    fixtures: Vec::new(),
                },
                validation: ValidationStatus::Synthetic,
            })
    }

    /// An independent restatement of "this profile is valid", written as one boolean.
    fn oracle_says_valid(profile: &SensorProfile) -> bool {
        let connection = &profile.connection;
        let convention_ok = match connection.address_convention.as_deref() {
            Some(text) => ["plc4x", "one-based-40001", "zero-based"].contains(&text),
            None => connection.scheme != "modbus-tcp",
        };
        let names: BTreeSet<&str> = profile
            .measurements
            .iter()
            .map(|m| m.name.as_str())
            .collect();
        convention_ok
            && names.len() == profile.measurements.len()
            && profile.measurements.iter().all(measurement_is_valid)
    }

    fn measurement_is_valid(m: &MeasurementDef) -> bool {
        let pairs: BTreeSet<(&str, u8)> = m
            .status_fields
            .iter()
            .map(|f| (f.address.as_str(), f.bit))
            .collect();
        let bits_exist = m.status_fields.iter().all(|f| f.bit < 64);
        let declared: BTreeSet<&str> = m.status_fields.iter().map(|f| f.name.as_str()).collect();
        let steps_ok = m.scaling.iter().all(|step| match *step {
            ScaleStep::Multiply(c) | ScaleStep::Offset(c) => c.is_finite(),
            ScaleStep::Divide(c) => c.is_finite() && c.abs() > 0.0,
            ScaleStep::Clamp { min, max } => min.is_finite() && max.is_finite() && min <= max,
            ScaleStep::BitField { shift, width } => {
                (1..=64).contains(&width) && u32::from(shift).saturating_add(u32::from(width)) <= 64
            }
        });
        let range_ok = m
            .range
            .is_none_or(|r| r.min.is_finite() && r.max.is_finite() && r.min <= r.max);
        let quality_ok = m.quality_map.as_ref().is_none_or(|map| {
            map.bad_when
                .iter()
                .chain(&map.uncertain_when)
                .all(|name| declared.contains(name.as_str()))
        });
        let clauses = [
            m.count == 1,
            steps_ok,
            ["Cel", "kPa", "1"].contains(&m.unit.as_str()),
            range_ok,
            pairs.len() == m.status_fields.len(),
            bits_exist,
            quality_ok,
        ];
        clauses.iter().all(|clause| *clause)
    }

    /// Profiles built to be valid, so the `Ok` side is exercised as often as the `Err` side.
    fn arb_valid_profile() -> impl Strategy<Value = SensorProfile> {
        let step = prop_oneof![
            (-10.0..10.0f64).prop_map(ScaleStep::Multiply),
            (1.0..10.0f64).prop_map(ScaleStep::Divide),
            (-10.0..10.0f64).prop_map(ScaleStep::Offset),
            (-5.0..0.0f64, 0.0..5.0f64).prop_map(|(min, max)| ScaleStep::Clamp { min, max }),
            (0..32u8, 1..32u8).prop_map(|(shift, width)| ScaleStep::BitField { shift, width }),
        ];
        let one = (
            prop::sample::select(vec!["INT", "REAL", "LREAL", "Other"]),
            prop::collection::vec(step, 0..4),
            0..4u8,
            any::<bool>(),
        );
        prop::collection::vec(one, 0..4).prop_map(|rows| {
            let mut profile = SensorProfile {
                id: ProfileId::new("generated").expect("valid id"),
                schema_version: SchemaVersion::CURRENT,
                device: DeviceApplicability::default(),
                connection: ConnectionRequirements {
                    scheme: "modbus-tcp".to_owned(),
                    address_convention: Some("plc4x".to_owned()),
                    options: BTreeMap::new(),
                },
                measurements: Vec::new(),
                provenance: Provenance {
                    source: "generated".to_owned(),
                    fixtures: Vec::new(),
                },
                validation: ValidationStatus::Synthetic,
            };
            for (index, (data_type, scaling, bits, with_quality)) in rows.into_iter().enumerate() {
                let status_fields: Vec<StatusField> = (0..bits)
                    .map(|bit| StatusField {
                        name: format!("s{bit}"),
                        address: "w".to_owned(),
                        bit,
                        invert: false,
                    })
                    .collect();
                let quality_map = (with_quality && bits > 0).then(|| QualityMapping {
                    bad_when: vec!["s0".to_owned()],
                    uncertain_when: Vec::new(),
                });
                profile.measurements.push(MeasurementDef {
                    name: TagId::new(&format!("m{index}")).expect("valid tag id"),
                    address: "a".to_owned(),
                    data_type: data_type.to_owned(),
                    count: 1,
                    byte_order: Some(ByteOrderName::BigEndian),
                    word_order: Some(WordOrderName::HighWordFirst),
                    scaling,
                    unit: "kPa".to_owned(),
                    range: None,
                    status_fields,
                    quality_map,
                    min_interval: None,
                    stale_after: None,
                    keep_raw: false,
                });
            }
            profile
        })
    }

    proptest! {
        #[test]
        fn profiles_built_to_be_valid_validate_clean(profile in arb_valid_profile()) {
            prop_assert_eq!(profile.validate(), Ok(()));
        }

        // covers: CODING_STANDARDS 13.3 / no schema instance panics validation
        #[test]
        fn validate_never_panics_and_agrees_with_an_independent_check(profile in arb_profile()) {
            let outcome = profile.validate();
            prop_assert_eq!(outcome.is_ok(), oracle_says_valid(&profile));
            if let Err(errors) = outcome {
                prop_assert!(!errors.is_empty(), "an Err is never empty");
                for error in &errors {
                    prop_assert!(!error.to_string().is_empty(), "every error displays");
                }
            }
        }
    }

    /// `CONVENTIONS` repeats the names of `AddressConvention`; this fails to compile when a
    /// variant is added (the match has no wildcard) and fails when the lists differ.
    #[cfg(feature = "modbus")]
    #[test]
    fn the_declarable_conventions_are_the_modbus_conventions() {
        use crate::modbus::AddressConvention;

        let all = [
            AddressConvention::Plc4xCompatible,
            AddressConvention::ZeroBased,
            AddressConvention::OneBased40001,
        ];
        for convention in all {
            match convention {
                AddressConvention::Plc4xCompatible
                | AddressConvention::ZeroBased
                | AddressConvention::OneBased40001 => {}
            }
            assert!(CONVENTIONS.contains(&convention.name()));
        }
        assert_eq!(CONVENTIONS.len(), all.len());
    }
}
