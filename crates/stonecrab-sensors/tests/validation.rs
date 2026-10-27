//! Validation tests: one test per `ProfileError` the validator returns, the full error
//! list in document order, and a valid profile that validates clean. Profiles are loaded
//! from TOML, the way applications build them.
#![forbid(unsafe_code)]
#![cfg(feature = "toml")]

#[cfg(test)]
mod tests {
    use stonecrab_core::{ErrorKind, PlcError};
    use stonecrab_sensors::{ProfileError, SensorProfile};

    const FULL: &str = include_str!("profiles/full.toml");

    /// A profile whose connection is `connection` (TOML lines under `[connection]`) and
    /// whose measurements are the given `[[measurements]]` bodies.
    fn profile(connection: &str, measurements: &[&str]) -> SensorProfile {
        let mut text = format!(
            "id = \"synthetic_validation\"\nschema_version = 1\nvalidation = \"synthetic\"\n\
             [connection]\n{connection}\n[provenance]\nsource = \"Synthetic test profile\"\n"
        );
        for body in measurements {
            text.push_str("[[measurements]]\n");
            text.push_str(body);
            text.push('\n');
        }
        SensorProfile::from_toml(&text).expect("test document loads")
    }

    /// A measurement body with the given overrides appended to a valid base. A repeated
    /// key would be a TOML error, so the base holds only the keys no test overrides.
    fn measurement(name: &str, extra: &str) -> String {
        format!("name = \"{name}\"\naddress = \"a\"\n{extra}")
    }

    /// A valid `INT` measurement in degrees Celsius with `extra` lines.
    fn int_cel(name: &str, extra: &str) -> String {
        measurement(
            name,
            &format!("data_type = \"INT\"\nunit = \"Cel\"\n{extra}"),
        )
    }

    fn errors_of(connection: &str, bodies: &[String]) -> Vec<ProfileError> {
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        profile(connection, &refs)
            .validate()
            .expect_err("the profile is invalid")
    }

    fn one(extra: &str) -> Vec<ProfileError> {
        errors_of("scheme = \"sim\"", &[int_cel("level", extra)])
    }

    #[test]
    fn a_valid_synthetic_profile_validates_clean() {
        let text = format!("{FULL}\n");
        let profile = SensorProfile::from_toml(&text).expect("loads");
        assert_eq!(profile.validate(), Ok(()), "the full fixture is valid");
        assert_eq!(
            profile.measurements.len(),
            3,
            "the fixture covers narrow and wide types"
        );
    }

    #[test]
    fn a_profile_with_no_measurements_is_valid() {
        assert_eq!(profile("scheme = \"sim\"", &[]).validate(), Ok(()));
    }

    #[test]
    fn a_sim_profile_needs_no_address_convention() {
        let valid = profile("scheme = \"sim\"", &[&int_cel("level", "")]);
        assert_eq!(valid.validate(), Ok(()));
    }

    #[test]
    fn duplicate_measurement_names_report_the_later_one() {
        let bodies = [
            int_cel("level", ""),
            int_cel("flow", ""),
            int_cel("level", ""),
        ];
        assert_eq!(
            errors_of("scheme = \"sim\"", &bodies),
            [ProfileError::DuplicateMeasurementName {
                measurement: 2,
                first: 0
            }],
        );
        let thrice = [
            int_cel("level", ""),
            int_cel("level", ""),
            int_cel("level", ""),
        ];
        assert_eq!(
            errors_of("scheme = \"sim\"", &thrice),
            [
                ProfileError::DuplicateMeasurementName {
                    measurement: 1,
                    first: 0
                },
                ProfileError::DuplicateMeasurementName {
                    measurement: 2,
                    first: 0
                },
            ],
            "every repeat points at the first"
        );
    }

    #[test]
    fn an_unknown_unit_is_reported() {
        let body = measurement("level", "data_type = \"INT\"\nunit = \"celsius\"");
        assert_eq!(
            errors_of("scheme = \"sim\"", &[body]),
            [ProfileError::UnknownUnit { measurement: 0 }],
        );
        let wrong_case = measurement("level", "data_type = \"INT\"\nunit = \"cel\"");
        assert_eq!(
            errors_of("scheme = \"sim\"", &[wrong_case]),
            [ProfileError::UnknownUnit { measurement: 0 }],
            "units are case-sensitive"
        );
    }

    #[test]
    fn an_empty_unit_is_reported_as_empty_and_not_unknown() {
        let body = measurement("level", "data_type = \"INT\"\nunit = \"\"");
        assert_eq!(
            errors_of("scheme = \"sim\"", &[body]),
            [ProfileError::EmptyUnit { measurement: 0 }],
        );
    }

    #[test]
    fn wide_modbus_types_need_both_orders() {
        for data_type in [
            "DINT", "UDINT", "DWORD", "LINT", "ULINT", "LWORD", "REAL", "LREAL",
        ] {
            let body = measurement(
                "flow",
                &format!("data_type = \"{data_type}\"\nunit = \"L\""),
            );
            assert_eq!(
                errors_of("scheme = \"sim\"", &[body]),
                [
                    ProfileError::MissingByteOrder { measurement: 0 },
                    ProfileError::MissingWordOrder { measurement: 0 },
                ],
                "{data_type} is wider than 16 bits"
            );
        }
    }

    #[test]
    fn one_missing_order_is_reported_alone() {
        let only_word = measurement(
            "flow",
            "data_type = \"REAL\"\nunit = \"L\"\nbyte_order = \"big_endian\"",
        );
        let only_byte = measurement(
            "flow",
            "data_type = \"REAL\"\nunit = \"L\"\nword_order = \"high_word_first\"",
        );
        assert_eq!(
            errors_of("scheme = \"sim\"", &[only_word]),
            [ProfileError::MissingWordOrder { measurement: 0 }],
        );
        assert_eq!(
            errors_of("scheme = \"sim\"", &[only_byte]),
            [ProfileError::MissingByteOrder { measurement: 0 }],
        );
    }

    #[test]
    fn narrow_and_foreign_type_names_need_no_order() {
        let narrow = [
            "INT", "UINT", "WORD", "BOOL", "SINT", "BYTE", "real", "Float64", "i32",
        ];
        let bodies: Vec<String> = narrow
            .iter()
            .enumerate()
            .map(|(index, data_type)| {
                measurement(
                    &format!("value_{index}"),
                    &format!("data_type = \"{data_type}\"\nunit = \"1\""),
                )
            })
            .collect();
        let refs: Vec<&str> = bodies.iter().map(String::as_str).collect();
        assert_eq!(
            profile("scheme = \"sim\"", &refs).validate(),
            Ok(()),
            "the width rule is Modbus-name based; other names pass through"
        );
    }

    #[test]
    fn overlapping_status_bits_on_one_address_are_reported() {
        let overlap = int_cel(
            "level",
            "status_fields = [\n\
             { name = \"first\", address = \"w:1\", bit = 3 },\n\
             { name = \"other_bit\", address = \"w:1\", bit = 4 },\n\
             { name = \"other_word\", address = \"w:2\", bit = 3 },\n\
             { name = \"again\", address = \"w:1\", bit = 3, invert = true },\n\
             ]",
        );
        assert_eq!(
            errors_of("scheme = \"sim\"", &[overlap]),
            [ProfileError::OverlappingStatusBits {
                measurement: 0,
                status_field: 3
            }],
            "same address and bit; other bits and other words are fine"
        );
    }

    #[test]
    fn measurements_may_share_a_status_word() {
        let status = "status_fields = [{ name = \"fault\", address = \"w:1\", bit = 0 }]";
        let bodies = [int_cel("level", status), int_cel("flow", status)];
        let refs = [bodies[0].as_str(), bodies[1].as_str()];
        assert_eq!(profile("scheme = \"sim\"", &refs).validate(), Ok(()));
    }

    #[test]
    fn a_count_other_than_one_is_unsupported() {
        for count in [0_u32, 2, 100] {
            assert_eq!(
                one(&format!("count = {count}")),
                [ProfileError::UnsupportedCount {
                    measurement: 0,
                    count
                }],
            );
        }
    }

    #[test]
    fn invalid_scale_steps_are_reported_by_position() {
        let steps = [
            "{ divide = 0.0 }",
            "{ divide = -0.0 }",
            "{ multiply = inf }",
            "{ multiply = nan }",
            "{ divide = inf }",
            "{ offset = -inf }",
            "{ clamp = { min = 5.0, max = 1.0 } }",
            "{ clamp = { min = nan, max = 1.0 } }",
            "{ clamp = { min = 0.0, max = inf } }",
            "{ bit_field = { shift = 0, width = 0 } }",
            "{ bit_field = { shift = 60, width = 5 } }",
            "{ bit_field = { shift = 255, width = 255 } }",
        ];
        for (position, step) in steps.iter().enumerate() {
            let pipeline = format!("scaling = [{{ multiply = 2.0 }}, {step}]");
            assert_eq!(
                one(&pipeline),
                [ProfileError::InvalidScaleStep {
                    measurement: 0,
                    step: 1
                }],
                "step {position}: {step}"
            );
        }
    }

    #[test]
    fn boundary_scale_steps_are_valid() {
        let pipeline = "scaling = [{ divide = 1e-300 }, { multiply = 0.0 }, { offset = 0.0 },\n\
             { clamp = { min = 2.0, max = 2.0 } },\n\
             { bit_field = { shift = 63, width = 1 } },\n\
             { bit_field = { shift = 0, width = 64 } }]";
        let valid = profile("scheme = \"sim\"", &[&int_cel("level", pipeline)]);
        assert_eq!(valid.validate(), Ok(()));
    }

    #[test]
    fn quality_references_must_name_declared_status_fields() {
        let extra = "status_fields = [{ name = \"fault\", address = \"w:1\", bit = 0 }]\n\
             quality_map = { bad_when = [\"fault\", \"ghost\"], uncertain_when = [\"fault\", \"phantom\"] }";
        assert_eq!(
            one(extra),
            [
                ProfileError::UnknownQualityReference {
                    measurement: 0,
                    entry: 1
                },
                ProfileError::UnknownQualityReference {
                    measurement: 0,
                    entry: 3
                },
            ],
            "entries count bad_when first, then uncertain_when"
        );
    }

    #[test]
    fn a_status_field_of_another_measurement_is_not_a_quality_reference() {
        let first = int_cel(
            "level",
            "status_fields = [{ name = \"fault\", address = \"w:1\", bit = 0 }]",
        );
        let second = int_cel("flow", "quality_map = { bad_when = [\"fault\"] }");
        assert_eq!(
            errors_of("scheme = \"sim\"", &[first, second]),
            [ProfileError::UnknownQualityReference {
                measurement: 1,
                entry: 0
            }],
        );
    }

    #[test]
    fn an_unknown_address_convention_is_reported() {
        for scheme in ["modbus-tcp", "sim"] {
            let connection = format!("scheme = \"{scheme}\"\naddress_convention = \"zero_based\"");
            assert_eq!(
                errors_of(&connection, &[]),
                [ProfileError::UnknownAddressConvention],
                "scheme {scheme}"
            );
        }
        let empty = "scheme = \"sim\"\naddress_convention = \"\"";
        assert_eq!(
            errors_of(empty, &[]),
            [ProfileError::UnknownAddressConvention],
            "an empty convention is a convention, and an unknown one"
        );
    }

    #[test]
    fn modbus_tcp_requires_an_address_convention() {
        assert_eq!(
            errors_of("scheme = \"modbus-tcp\"", &[]),
            [ProfileError::MissingAddressConvention],
        );
        let declared = profile(
            "scheme = \"modbus-tcp\"\naddress_convention = \"plc4x\"",
            &[],
        );
        assert_eq!(declared.validate(), Ok(()));
    }

    // covers: SENSORS-VALIDATION / every error is returned, in document order
    #[test]
    fn a_profile_with_several_faults_returns_every_error_in_document_order() {
        let bodies = [
            measurement(
                "first",
                "data_type = \"LREAL\"\ncount = 4\nunit = \"furlong\"\n\
                 scaling = [{ divide = 0.0 }, { offset = 1.0 }, { clamp = { min = 2.0, max = 1.0 } }]",
            ),
            measurement("second", "data_type = \"INT\"\nunit = \"\""),
            measurement(
                "first",
                "data_type = \"DINT\"\nbyte_order = \"big_endian\"\nunit = \"Cel\"\n\
                 status_fields = [\n\
                 { name = \"a\", address = \"w\", bit = 1 },\n\
                 { name = \"b\", address = \"w\", bit = 1 },\n\
                 ]\n\
                 quality_map = { bad_when = [\"c\"] }",
            ),
        ];
        assert_eq!(
            errors_of("scheme = \"modbus-tcp\"", &bodies),
            [
                ProfileError::MissingAddressConvention,
                ProfileError::UnsupportedCount {
                    measurement: 0,
                    count: 4
                },
                ProfileError::MissingByteOrder { measurement: 0 },
                ProfileError::MissingWordOrder { measurement: 0 },
                ProfileError::InvalidScaleStep {
                    measurement: 0,
                    step: 0
                },
                ProfileError::InvalidScaleStep {
                    measurement: 0,
                    step: 2
                },
                ProfileError::UnknownUnit { measurement: 0 },
                ProfileError::EmptyUnit { measurement: 1 },
                ProfileError::DuplicateMeasurementName {
                    measurement: 2,
                    first: 0
                },
                ProfileError::MissingWordOrder { measurement: 2 },
                ProfileError::OverlappingStatusBits {
                    measurement: 2,
                    status_field: 1
                },
                ProfileError::UnknownQualityReference {
                    measurement: 2,
                    entry: 0
                },
            ],
        );
    }

    #[test]
    fn validation_errors_convert_to_invalid_config_and_never_echo_the_profile() {
        let canary = "canary_unit_text";
        let body = measurement(
            "level",
            &format!("data_type = \"INT\"\nunit = \"{canary}\""),
        );
        for error in errors_of("scheme = \"sim\"", &[body]) {
            assert!(!error.to_string().contains(canary), "display: {error}");
            assert!(!format!("{error:?}").contains(canary), "debug: {error:?}");
            let plc = PlcError::from(error);
            assert_eq!(plc.kind(), ErrorKind::InvalidConfig, "kind");
        }
    }
}
