use std::collections::BTreeMap;

use crate::sensors::{
    ConnectionRequirements, MeasurementDef, ProfileError, ScaleStep, SensorProfile, Unit,
};

/// Modbus data type names that occupy more than 16 bits on the wire (32- and 64-bit
/// integers, bit strings and floats), spelled as the Modbus driver spells them.
const WIDE_MODBUS_TYPES: [&str; 8] = [
    "DINT", "UDINT", "DWORD", "LINT", "ULINT", "LWORD", "REAL", "LREAL",
];

/// The one address convention that exists at alpha.
const PLC4X_CONVENTION: &str = "plc4x";

/// The connection scheme whose profiles must declare an address convention (decision D17).
const MODBUS_TCP_SCHEME: &str = "modbus-tcp";

/// Largest bit position plus one that a `BitField` step may reach (values are 64 bits).
const VALUE_BITS: u16 = 64;

impl SensorProfile {
    /// Check the meaning of a loaded profile and report every problem at once.
    ///
    /// [`SensorProfile::from_toml`] checks the shape of a document; this checks what it
    /// says. The check never stops at the first problem: the `Err` holds every error, in
    /// a fixed order, so an author fixes a profile in one pass. The order is the
    /// connection first, then each measurement in document order, and within a
    /// measurement the fields in the order of [`MeasurementDef`]: name, `count`,
    /// `byte_order` and `word_order`, scaling steps, unit, status fields, quality
    /// mapping. A measurement is named in an error by its 0-based index; errors never
    /// contain any text of the profile.
    ///
    /// Rules:
    /// - the address convention, when set, is `plc4x`, and a `modbus-tcp` connection must
    ///   set it (`UnknownAddressConvention`, `MissingAddressConvention`);
    /// - measurement names are unique (`DuplicateMeasurementName`; the later one is
    ///   reported). Names are `snake_case` already, because the loader refuses anything
    ///   else;
    /// - `count` is 1 (`UnsupportedCount`);
    /// - a data type of 32 or 64 bits has both a `byte_order` and a `word_order`
    ///   (`MissingByteOrder`, `MissingWordOrder`). The width comes from the data type
    ///   name by the Modbus rule: `DINT`, `UDINT`, `DWORD`, `LINT`, `ULINT`, `LWORD`,
    ///   `REAL` and `LREAL`, in upper case, are wider than 16 bits. Every other name is
    ///   passed through without an order requirement, because the width of another
    ///   driver's type is that driver's to say. The rule is Modbus-name based and not
    ///   universal;
    /// - every scaling step can be evaluated: constants are finite, a divisor is not
    ///   zero, a clamp has `min <= max`, a bit field has a width of at least 1 and
    ///   `shift + width <= 64` (`InvalidScaleStep`);
    /// - the unit is not empty and is in the [`Unit`] table (`EmptyUnit`, `UnknownUnit`);
    /// - no two status fields of one measurement name the same address text and bit
    ///   (`OverlappingStatusBits`; two measurements may share a status word);
    /// - every name in the quality mapping is a status field of the same measurement
    ///   (`UnknownQualityReference`).
    ///
    /// Uniqueness of status-field names within one measurement is not enforced: a quality
    /// reference resolves to any status field of that name, so duplicates are ambiguous
    /// but accepted.
    ///
    /// Validation is independent of any driver. Whether `address` and `data_type` are
    /// meaningful to the driver, and whether status fields name real addresses, is the
    /// driver's `parse_address` to say when tags are built (slice S29); this method only
    /// compares address text for equality.
    ///
    /// # Errors
    /// Returns every [`ProfileError`] found, never an empty list: the validation variants
    /// are `DuplicateMeasurementName`, `UnknownUnit`, `EmptyUnit`, `MissingByteOrder`,
    /// `MissingWordOrder`, `OverlappingStatusBits`, `UnsupportedCount`,
    /// `InvalidScaleStep`, `UnknownQualityReference`, `UnknownAddressConvention` and
    /// `MissingAddressConvention`. Each converts into a [`PlcError`](crate::PlcError)
    /// of kind `InvalidConfig`.
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
}

fn check_connection(connection: &ConnectionRequirements, errors: &mut Vec<ProfileError>) {
    match connection.address_convention.as_deref() {
        Some(PLC4X_CONVENTION) => {}
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
    if WIDE_MODBUS_TYPES.contains(&measurement.data_type.as_str()) {
        if measurement.byte_order.is_none() {
            errors.push(ProfileError::MissingByteOrder { measurement: index });
        }
        if measurement.word_order.is_none() {
            errors.push(ProfileError::MissingWordOrder { measurement: index });
        }
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
    for (position, field) in measurement.status_fields.iter().enumerate() {
        let repeats = measurement
            .status_fields
            .iter()
            .take(position)
            .any(|earlier| earlier.address == field.address && earlier.bit == field.bit);
        if repeats {
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
        StatusField, ValidationStatus, WordOrderName,
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
            0..3u8,
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
        let tail = (
            prop::collection::vec(arb_step(), 0..4),
            prop::sample::select(vec!["Cel", "kPa", "1", "", "furlong"]),
            prop::collection::vec(arb_status_field(), 0..4),
            arb_quality(),
        );
        (head, tail).prop_map(
            |((name, data_type, count, byte_order, word_order), (scaling, unit, status, map))| {
                MeasurementDef {
                    name: TagId::new(name).expect("valid tag id"),
                    address: "a".to_owned(),
                    data_type: data_type.to_owned(),
                    count,
                    byte_order,
                    word_order,
                    scaling,
                    unit: unit.to_owned(),
                    range: None,
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
            prop::option::of(prop::sample::select(vec!["plc4x", "other", ""])),
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
            Some(text) => text == "plc4x",
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
        let wide = matches!(
            m.data_type.as_str(),
            "DINT" | "UDINT" | "DWORD" | "LINT" | "ULINT" | "LWORD" | "REAL" | "LREAL"
        );
        let pairs: BTreeSet<(&str, u8)> = m
            .status_fields
            .iter()
            .map(|f| (f.address.as_str(), f.bit))
            .collect();
        let declared: BTreeSet<&str> = m.status_fields.iter().map(|f| f.name.as_str()).collect();
        let steps_ok = m.scaling.iter().all(|step| match *step {
            ScaleStep::Multiply(c) | ScaleStep::Offset(c) => c.is_finite(),
            ScaleStep::Divide(c) => c.is_finite() && c.abs() > 0.0,
            ScaleStep::Clamp { min, max } => min.is_finite() && max.is_finite() && min <= max,
            ScaleStep::BitField { shift, width } => {
                (1..=64).contains(&width) && u32::from(shift).saturating_add(u32::from(width)) <= 64
            }
        });
        let quality_ok = m.quality_map.as_ref().is_none_or(|map| {
            map.bad_when
                .iter()
                .chain(&map.uncertain_when)
                .all(|name| declared.contains(name.as_str()))
        });
        let clauses = [
            m.count == 1,
            !wide || (m.byte_order.is_some() && m.word_order.is_some()),
            steps_ok,
            ["Cel", "kPa", "1"].contains(&m.unit.as_str()),
            pairs.len() == m.status_fields.len(),
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
}
