//! Loader tests: the section 13.7 list for `SensorProfile::from_toml`, one test per
//! mandatory case, plus the shape of every schema type from the fixtures under
//! `tests/profiles/` (which ship nowhere: the package include list excludes tests).
#![cfg(feature = "sensors")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use stonecrab::sensors::{
        ByteOrderName, MeasurementDef, ProfileError, ProfileId, Quality, QualityMapping, ScaleStep,
        SchemaVersion, SensorProfile, StatusField, ValidationStatus, ValueRange, WordOrderName,
    };
    use stonecrab::{ByteOrder, ErrorKind, PlcError, WordOrder};

    const MINIMAL: &str = include_str!("profiles/minimal.toml");
    const FULL: &str = include_str!("profiles/full.toml");
    const UNKNOWN_FIELD: &str = include_str!("profiles/unknown_field.toml");
    const SCHEMA_V3: &str = include_str!("profiles/schema_v3.toml");
    const WRONG_TYPE: &str = include_str!("profiles/wrong_type.toml");
    const MISSING_NAME: &str = include_str!("profiles/missing_name.toml");

    fn load(text: &str) -> SensorProfile {
        SensorProfile::from_toml(text).expect("fixture loads")
    }

    fn syntax_of(text: &str) -> (usize, usize) {
        match SensorProfile::from_toml(text) {
            Err(ProfileError::Syntax { line, column }) => (line, column),
            other => panic!("expected a syntax error, got {other:?}"),
        }
    }

    /// The minimal profile with one measurement whose extra lines are given.
    fn one_measurement(extra: &str) -> String {
        format!(
            "{MINIMAL}\n[[measurements]]\nname = \"level\"\naddress = \"a\"\ndata_type = \"INT\"\nunit = \"m\"\n{extra}\n"
        )
    }

    fn only_measurement(text: &str) -> MeasurementDef {
        load(text)
            .measurements
            .into_iter()
            .next()
            .expect("one measurement")
    }

    // covers: CODING_STANDARDS 13.7 / zero length
    #[test]
    fn an_empty_document_is_a_syntax_error_without_echo() {
        assert_eq!(
            SensorProfile::from_toml(""),
            Err(ProfileError::Syntax { line: 1, column: 1 })
        );
        assert_eq!(
            syntax_of("   \n\n"),
            (1, 1),
            "whitespace is no better than empty"
        );
    }

    // covers: CODING_STANDARDS 13.7 / maximum-plus-one length
    #[test]
    fn the_size_bound_is_checked_before_parsing() {
        let max = SensorProfile::MAX_TOML_BYTES;
        assert_eq!(max, 65_536, "the documented bound");

        let padding = |len: usize| {
            let mut text = String::from(MINIMAL);
            text.push_str("# ");
            while text.len() < len {
                text.push('x');
            }
            text
        };
        let at_bound = padding(max);
        assert_eq!(at_bound.len(), max);
        assert_eq!(
            load(&at_bound).id.as_str(),
            "minimal",
            "exactly at the bound loads"
        );

        let over = padding(max + 1);
        assert_eq!(over.len(), max + 1);
        assert_eq!(
            SensorProfile::from_toml(&over),
            Err(ProfileError::Oversized { len: max + 1, max }),
            "one byte over is refused"
        );
        // The bound applies before parsing: text that is not TOML at all is still Oversized.
        let garbage = "\u{0}".repeat(max + 1);
        assert_eq!(
            SensorProfile::from_toml(&garbage),
            Err(ProfileError::Oversized { len: max + 1, max }),
            "oversized garbage is not parsed"
        );
    }

    #[test]
    fn the_minimal_profile_loads_with_defaults() {
        let profile = load(MINIMAL);
        assert_eq!(profile.id, ProfileId::new("minimal").expect("valid"));
        assert_eq!(profile.schema_version, SchemaVersion::V1);
        assert_eq!(profile.validation, ValidationStatus::Synthetic);
        assert_eq!(profile.device.manufacturer, None);
        assert_eq!(profile.device.model, None);
        assert_eq!(profile.device.firmware, None);
        assert_eq!(profile.connection.scheme, "sim");
        assert_eq!(
            profile.connection.address_convention, None,
            "never defaulted"
        );
        assert_eq!(profile.connection.options.len(), 0);
        assert_eq!(profile.measurements.len(), 0);
        assert_eq!(
            profile.provenance.source,
            "Synthetic test fixture; no real device"
        );
        assert_eq!(profile.provenance.fixtures.len(), 0);
    }

    // covers: CODING_STANDARDS 13.7 / invalid discriminator (unknown field)
    #[test]
    fn an_unknown_field_is_rejected_at_every_level() {
        assert_eq!(syntax_of(UNKNOWN_FIELD), (4, 1), "top level, at the key");
        let cases = [
            "[device]\nserial = \"x\"\n",
            "[provenance]\nreviewer = \"x\"\n",
            "[connection.extra]\nk = \"v\"\n",
        ];
        for tail in cases {
            let text = format!("{MINIMAL}\n{tail}");
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{tail}"
            );
        }
        for extra in [
            "gain = 2.0",
            "range = { min = 0.0, max = 1.0, step = 0.1 }",
            "scaling = [{ clamp = { min = 0.0, max = 1.0, soft = true } }]",
            "scaling = [{ bit_field = { shift = 1, width = 1, signed = true } }]",
            "status_fields = [{ name = \"f\", address = \"a\", bit = 0, mask = 1 }]",
            "quality_map = { good_when = [\"f\"] }",
        ] {
            let text = one_measurement(extra);
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{extra}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 incompatible profile version
    #[test]
    fn schema_version_3_is_rejected_as_incompatible() {
        assert_eq!(
            SensorProfile::from_toml(SCHEMA_V3),
            Err(ProfileError::IncompatibleSchema { found: 3 }),
            "reported even though the document has a key version 2 does not know"
        );
        for found in [0_u32, 4, 100, u32::MAX] {
            let text = MINIMAL.replace("schema_version = 1", &format!("schema_version = {found}"));
            assert_eq!(
                SensorProfile::from_toml(&text),
                Err(ProfileError::IncompatibleSchema { found }),
                "version {found}"
            );
        }
        let error = PlcError::from(ProfileError::IncompatibleSchema { found: 3 });
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "conversion kind");
    }

    // covers: SENSORS-SCHEMA-V2 / version 2 loads, and a version 1 document loads unchanged
    #[test]
    fn schema_version_2_loads_and_version_1_documents_load_as_they_did() {
        let v2 = MINIMAL.replace("schema_version = 1", "schema_version = 2");
        let profile = load(&v2);
        assert_eq!(profile.schema_version, SchemaVersion::V2);
        assert_eq!(profile.schema_version, SchemaVersion::CURRENT);
        assert_eq!(
            profile.validate(),
            Ok(()),
            "version 2 adds a key, not a rule"
        );
        for text in [MINIMAL, FULL] {
            let profile = load(text);
            assert_eq!(
                profile.schema_version,
                SchemaVersion::V1,
                "version 1 stays 1"
            );
            assert_eq!(profile.validate(), Ok(()));
            assert!(
                profile
                    .measurements
                    .iter()
                    .all(|m| m.value_rules.is_empty()),
                "a version 1 document has no value rules"
            );
        }
        // The same measurement, read under either version, differs by nothing the key adds.
        let one = |version: u32| {
            only_measurement(
                &one_measurement("")
                    .replace("schema_version = 1", &format!("schema_version = {version}")),
            )
        };
        assert_eq!(one(1), one(2), "the version alone changes no measurement");
    }

    // covers: SENSORS-VALUE-RULES / the value_rules key loads in full, defaults and refusals
    #[test]
    fn value_rules_load_with_values_quality_and_an_optional_reason() {
        let text = one_measurement(
            "value_rules = [\n  { values = [-32766, -32765], quality = \"bad\", reason = \"not acquired\" },\n  { values = [7], quality = \"uncertain\" },\n]",
        )
        .replace("schema_version = 1", "schema_version = 2");
        let measurement = only_measurement(&text);
        let rules: Vec<(&[i64], Quality, Option<&str>)> = measurement
            .value_rules
            .iter()
            .map(|rule| (rule.values.as_slice(), rule.quality, rule.reason.as_deref()))
            .collect();
        assert_eq!(
            rules,
            [
                (&[-32766, -32765][..], Quality::Bad, Some("not acquired")),
                (&[7][..], Quality::Uncertain, None),
            ]
        );
        assert!(
            only_measurement(&one_measurement(""))
                .value_rules
                .is_empty(),
            "no key, no rules"
        );
        for bad in [
            "value_rules = [{ values = [1], quality = \"good\" }]",
            "value_rules = [{ values = [1], quality = \"stale\" }]",
            "value_rules = [{ values = [1], quality = \"Bad\" }]",
            "value_rules = [{ values = [1] }]",
            "value_rules = [{ quality = \"bad\" }]",
            "value_rules = [{ values = [1.5], quality = \"bad\" }]",
            "value_rules = [{ values = [\"1\"], quality = \"bad\" }]",
            "value_rules = [{ values = [1], quality = \"bad\", range = [1, 2] }]",
            "value_rules = [{ values = [18446744073709551615], quality = \"bad\" }]",
            "value_rules = { values = [1], quality = \"bad\" }",
        ] {
            let text = one_measurement(bad).replace("schema_version = 1", "schema_version = 2");
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_missing_or_mistyped_schema_version_is_a_syntax_error() {
        let missing = MINIMAL.replace("schema_version = 1\n", "");
        assert!(matches!(
            SensorProfile::from_toml(&missing),
            Err(ProfileError::Syntax { .. })
        ));
        for value in ["\"1\"", "1.0", "-1", "4294967296", "true"] {
            let text = MINIMAL.replace("schema_version = 1", &format!("schema_version = {value}"));
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "schema_version = {value}"
            );
        }
    }

    // covers: CODING_STANDARDS 13.7 / invalid discriminator (wrong-typed field)
    #[test]
    fn a_wrong_typed_field_is_a_syntax_error_at_its_position() {
        assert_eq!(
            syntax_of(WRONG_TYPE),
            (15, 9),
            "count = \"two\" points at the value"
        );
        for (key, value) in [
            ("id", "5"),
            ("validation", "\"verified\""),
            ("validation", "1"),
        ] {
            let text = MINIMAL
                .lines()
                .map(|line| {
                    if line.starts_with(key) {
                        format!("{key} = {value}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{key} = {value}"
            );
        }
    }

    // covers: CODING_STANDARDS 13.7 / missing required field (a measurement without a name)
    #[test]
    fn a_measurement_without_a_name_is_rejected() {
        let (line, _) = syntax_of(MISSING_NAME);
        assert_eq!(line, 11, "the measurement table that lacks the name");
    }

    #[test]
    fn missing_required_keys_are_rejected() {
        for key in [
            "id = ",
            "validation = ",
            "[connection]",
            "[provenance]",
            "scheme = ",
            "source = ",
        ] {
            let text: String = MINIMAL
                .lines()
                .filter(|line| !line.starts_with(key))
                .flat_map(|line| [line, "\n"])
                .collect();
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{key}"
            );
        }
        for drop in ["address = ", "data_type = "] {
            let text: String = one_measurement("")
                .lines()
                .filter(|line| !line.starts_with(drop))
                .flat_map(|line| [line, "\n"])
                .collect();
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{drop}"
            );
        }
    }

    // covers: SPEC sensors / unit when known (G06: an absent unit is unknown, not an error)
    #[test]
    fn a_measurement_without_a_unit_loads_with_no_unit() {
        let text: String = one_measurement("")
            .lines()
            .filter(|line| !line.starts_with("unit = "))
            .flat_map(|line| [line, "\n"])
            .collect();
        let measurement = only_measurement(&text);
        assert_eq!(measurement.unit, None, "unknown, not empty");
        assert_eq!(
            only_measurement(&one_measurement("")).unit.as_deref(),
            Some("m"),
            "a stated unit is kept"
        );
        assert_eq!(
            only_measurement(&one_measurement("").replace("unit = \"m\"", "unit = \"\""))
                .unit
                .as_deref(),
            Some(""),
            "the loader checks shape only; an empty unit is the validator's to refuse"
        );
    }

    // covers: CODING_STANDARDS 13.7 / hostile nesting stays inside the size bound
    #[test]
    fn hostile_nesting_is_rejected_without_exhausting_the_stack() {
        let depth = 16_384; // four bytes per level fits in the 64 KiB bound
        let arrays = format!("id = {}{}", "[".repeat(depth), "]".repeat(depth));
        assert!(matches!(
            SensorProfile::from_toml(&arrays),
            Err(ProfileError::Syntax { .. })
        ));
        let inline_tables = format!("id = {}", "{a=".repeat(depth));
        assert!(matches!(
            SensorProfile::from_toml(&inline_tables),
            Err(ProfileError::Syntax { .. })
        ));
        let dotted = format!("{} = 1", vec!["a"; depth].join("."));
        assert!(matches!(
            SensorProfile::from_toml(&dotted),
            Err(ProfileError::Syntax { .. })
        ));
    }

    #[test]
    fn invalid_toml_is_a_syntax_error_with_a_position() {
        assert_eq!(syntax_of("id = \n"), (1, 6));
        assert_eq!(
            syntax_of("id = \"a\"\nid = \"b\"\n"),
            (2, 1),
            "duplicate key"
        );
        assert_eq!(syntax_of("[connection\n"), (1, 12));
    }

    // covers: SPEC mandatory-failure-cases / invalid configuration (no echo of the document)
    #[test]
    fn errors_never_echo_the_document() {
        let canary = "CANARY_TOKEN_9f3a";
        let documents = [
            format!("{MINIMAL}\n{canary} = 1\n"),
            format!("{MINIMAL}\nid = \"{canary}\"\n"),
            MINIMAL.replace("synthetic", canary),
            MINIMAL.replace(
                "schema_version = 1",
                &format!("schema_version = \"{canary}\""),
            ),
            one_measurement(&format!("name = \"{canary}\"")),
        ];
        for document in documents {
            let error = SensorProfile::from_toml(&document).expect_err("each document is invalid");
            let shown = format!("{error} {error:?} {}", PlcError::from(error));
            assert!(!shown.contains(canary), "echoed into: {shown}");
        }
    }

    #[test]
    fn the_full_profile_loads_every_field() {
        let profile = load(FULL);
        assert_eq!(profile.id.as_str(), "synthetic_full");
        assert_eq!(profile.schema_version.get(), 1);
        assert_eq!(profile.validation, ValidationStatus::Synthetic);
        assert_eq!(
            profile.device.manufacturer.as_deref(),
            Some("Synthetic Manufacturer")
        );
        assert_eq!(profile.device.model.as_deref(), Some("Synthetic Model"));
        assert_eq!(profile.device.firmware.as_deref(), Some("1.0"));
        assert_eq!(profile.connection.scheme, "modbus-tcp");
        assert_eq!(
            profile.connection.address_convention.as_deref(),
            Some("plc4x")
        );
        assert_eq!(profile.connection.options.len(), 2);
        assert_eq!(
            profile
                .connection
                .options
                .get("default-unit-identifier")
                .map(String::as_str),
            Some("1")
        );
        assert_eq!(
            profile
                .connection
                .options
                .get("request-timeout")
                .map(String::as_str),
            Some("500")
        );
        assert_eq!(
            profile.provenance.fixtures,
            ["synthetic_full_read_01", "synthetic_full_read_02"]
        );
        assert_eq!(profile.measurements.len(), 3);

        let [temperature, pressure, state] = profile.measurements.as_slice() else {
            panic!("three measurements");
        };
        assert_eq!(temperature.name.as_str(), "boiler_temperature");
        assert_eq!(temperature.address, "holding-register:1:INT");
        assert_eq!(temperature.data_type, "INT");
        assert_eq!(temperature.count, 1);
        assert_eq!(temperature.byte_order, Some(ByteOrderName::BigEndian));
        assert_eq!(temperature.word_order, None);
        assert_eq!(temperature.unit.as_deref(), Some("Cel"));
        assert_eq!(
            temperature.scaling,
            [
                ScaleStep::Divide(10.0),
                ScaleStep::Offset(-40.0),
                ScaleStep::Clamp {
                    min: -40.0,
                    max: 215.0
                },
            ]
        );
        let range = temperature.range.expect("range");
        assert_eq!((range.min, range.max), (-40.0, 215.0));
        assert_eq!(temperature.min_interval, Some(Duration::from_millis(1000)));
        assert_eq!(temperature.stale_after, Some(Duration::from_secs(5)));
        assert!(temperature.keep_raw);
        assert_eq!(temperature.status_fields.len(), 2);
        let [open, out_of_range] = temperature.status_fields.as_slice() else {
            panic!("two status fields");
        };
        assert_eq!(
            (
                open.name.as_str(),
                open.address.as_str(),
                open.bit,
                open.invert
            ),
            ("open_circuit", "holding-register:20:WORD", 3, true)
        );
        assert!(!out_of_range.invert, "invert defaults to false");
        let quality = temperature.quality_map.as_ref().expect("quality map");
        assert_eq!(quality.bad_when, ["open_circuit"]);
        assert_eq!(quality.uncertain_when, ["out_of_range"]);

        assert_eq!(pressure.byte_order, Some(ByteOrderName::LittleEndian));
        assert_eq!(pressure.word_order, Some(WordOrderName::LowWordFirst));
        assert_eq!(
            pressure.scaling,
            [ScaleStep::Multiply(2.0)],
            "an integer constant is accepted"
        );
        assert_eq!((pressure.min_interval, pressure.stale_after), (None, None));
        assert!(!pressure.keep_raw, "keep_raw defaults to false");
        assert_eq!(pressure.range, None);
        assert_eq!(pressure.quality_map, None);
        assert_eq!(pressure.status_fields.len(), 0);

        assert_eq!(state.scaling, [ScaleStep::BitField { shift: 4, width: 1 }]);
    }

    #[test]
    fn the_loaded_profile_clones_and_compares_equal() {
        let profile = load(FULL);
        assert_eq!(profile.clone(), profile);
        assert_ne!(profile, load(MINIMAL));
    }

    // covers: CODING_STANDARDS 13.7 / invalid discriminator (identifier boundaries)
    #[test]
    fn profile_id_boundaries() {
        let longest = "a".repeat(ProfileId::MAX_BYTES);
        let too_long = "a".repeat(ProfileId::MAX_BYTES + 1);
        for text in [
            "a",
            "a0",
            "a_b",
            "synthetic_temperature_2",
            longest.as_str(),
        ] {
            let id = ProfileId::new(text).expect("accepted");
            assert_eq!(id.as_str(), text);
            assert_eq!(id.to_string(), text);
            assert_eq!(ProfileId::try_from(text), Ok(id.clone()));
            assert_eq!(ProfileId::try_from(text.to_owned()), Ok(id));
        }
        for text in [
            "",
            "A",
            "aB",
            "0a",
            "_a",
            "a-b",
            "a b",
            "a.b",
            "é",
            "a\u{e9}",
            " a",
            "a\n",
            too_long.as_str(),
        ] {
            let error = ProfileId::new(text).expect_err("rejected");
            assert_eq!(
                error,
                ProfileError::InvalidIdentifier {
                    field: "profile id"
                },
                "{text:?}"
            );
            assert_eq!(ProfileId::try_from(text.to_owned()), Err(error));
        }
        let (a, b) = (
            ProfileId::new("a").expect("valid"),
            ProfileId::new("b").expect("valid"),
        );
        assert!(a < b, "ordered by text");
    }

    #[test]
    fn profile_id_boundaries_apply_in_documents() {
        let longest = "a".repeat(ProfileId::MAX_BYTES);
        let ok = MINIMAL.replace("id = \"minimal\"", &format!("id = \"{longest}\""));
        assert_eq!(load(&ok).id.as_str(), longest);
        for bad in ["", "Minimal", "mini mal", "0minimal", &"a".repeat(65)] {
            let text = MINIMAL.replace("id = \"minimal\"", &format!("id = \"{bad}\""));
            assert_eq!(syntax_of(&text), (2, 6), "id = {bad:?} points at the value");
        }
    }

    #[test]
    fn measurement_names_double_as_tag_ids() {
        let longest = "n".repeat(MeasurementDef::MAX_NAME_BYTES);
        for name in ["level", "level_2", "l", longest.as_str()] {
            let measurement = only_measurement(
                &one_measurement(&format!("# {name}"))
                    .replace("name = \"level\"", &format!("name = \"{name}\"")),
            );
            assert_eq!(measurement.name.as_str(), name);
            assert_eq!(measurement.name.to_string(), name, "usable as a TagId");
        }
        let too_long = "n".repeat(MeasurementDef::MAX_NAME_BYTES + 1);
        for name in [
            "",
            "Level",
            "level one",
            "1level",
            "_level",
            "level-1",
            "niveau\u{e9}",
            too_long.as_str(),
        ] {
            let text =
                one_measurement("").replace("name = \"level\"", &format!("name = \"{name}\""));
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "name {name:?}"
            );
        }
    }

    #[test]
    fn status_field_names_follow_the_same_rule() {
        let field = |name: &str| {
            one_measurement(&format!(
                "status_fields = [{{ name = \"{name}\", address = \"a\", bit = 0 }}]"
            ))
        };
        let field_ok = only_measurement(&field("open_circuit")).status_fields;
        assert_eq!(field_ok.len(), 1);
        for name in ["", "Open", "open circuit", "0open"] {
            assert!(
                matches!(
                    SensorProfile::from_toml(&field(name)),
                    Err(ProfileError::Syntax { .. })
                ),
                "{name:?}"
            );
        }
    }

    #[test]
    fn durations_are_whole_milliseconds() {
        let measurement = only_measurement(&one_measurement(
            "min_interval_ms = 0\nstale_after_ms = 1500",
        ));
        assert_eq!(
            measurement.min_interval,
            Some(Duration::ZERO),
            "zero is carried, not rejected"
        );
        assert_eq!(measurement.stale_after, Some(Duration::from_millis(1500)));
        let absent = only_measurement(&one_measurement(""));
        assert_eq!((absent.min_interval, absent.stale_after), (None, None));
        for bad in [
            "min_interval_ms = -1",
            "min_interval_ms = 1.5",
            "min_interval_ms = \"1s\"",
            "stale_after_ms = -1",
            "stale_after_ms = true",
        ] {
            assert!(
                matches!(
                    SensorProfile::from_toml(&one_measurement(bad)),
                    Err(ProfileError::Syntax { .. })
                ),
                "{bad}"
            );
        }
        for unknown in ["min_interval = 1000", "stale_after = 1000"] {
            assert!(
                matches!(
                    SensorProfile::from_toml(&one_measurement(unknown)),
                    Err(ProfileError::Syntax { .. })
                ),
                "{unknown}"
            );
        }
    }

    #[test]
    fn every_scale_step_form_loads() {
        let measurement = only_measurement(&one_measurement(
            "scaling = [{ multiply = 0.5 }, { divide = 4 }, { offset = -1.5 }, { clamp = { min = 0, max = 10.5 } }, { bit_field = { shift = 2, width = 3 } }]",
        ));
        assert_eq!(
            measurement.scaling,
            [
                ScaleStep::Multiply(0.5),
                ScaleStep::Divide(4.0),
                ScaleStep::Offset(-1.5),
                ScaleStep::Clamp {
                    min: 0.0,
                    max: 10.5
                },
                ScaleStep::BitField { shift: 2, width: 3 },
            ]
        );
        let empty = only_measurement(&one_measurement("scaling = []"));
        assert_eq!(empty.scaling.len(), 0);
    }

    #[test]
    fn malformed_scale_steps_are_rejected() {
        for bad in [
            "scaling = [\"multiply\"]",
            "scaling = [{ scale = 2.0 }]",
            "scaling = [{ multiply = \"2\" }]",
            "scaling = [{ multiply = 2.0, offset = 1.0 }]",
            "scaling = [{}]",
            "scaling = [{ clamp = { min = 0.0 } }]",
            "scaling = [{ clamp = 5.0 }]",
            "scaling = [{ bit_field = { shift = 256, width = 1 } }]",
            "scaling = [{ bit_field = { shift = -1, width = 1 } }]",
            "scaling = [{ bit_field = { shift = 1 } }]",
            "scaling = { multiply = 2.0 }",
        ] {
            assert!(
                matches!(
                    SensorProfile::from_toml(&one_measurement(bad)),
                    Err(ProfileError::Syntax { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn non_finite_constants_are_carried_for_the_validator() {
        // TOML spells them `nan` and `inf`. Rejecting them is validation (slice S28b).
        let measurement = only_measurement(&one_measurement(
            "scaling = [{ multiply = inf }, { offset = nan }]",
        ));
        let [ScaleStep::Multiply(multiplier), ScaleStep::Offset(offset)] =
            measurement.scaling.as_slice()
        else {
            panic!("two steps");
        };
        assert!(multiplier.is_infinite() && offset.is_nan());
    }

    #[test]
    fn byte_and_word_order_names_map_to_core() {
        let cases = [
            ("byte_order = \"big_endian\"", ByteOrder::BigEndian),
            ("byte_order = \"little_endian\"", ByteOrder::LittleEndian),
        ];
        for (line, expected) in cases {
            let name = only_measurement(&one_measurement(line))
                .byte_order
                .expect("set");
            assert_eq!(ByteOrder::from(name), expected, "{line}");
        }
        let cases = [
            ("word_order = \"high_word_first\"", WordOrder::HighWordFirst),
            ("word_order = \"low_word_first\"", WordOrder::LowWordFirst),
        ];
        for (line, expected) in cases {
            let name = only_measurement(&one_measurement(line))
                .word_order
                .expect("set");
            assert_eq!(WordOrder::from(name), expected, "{line}");
        }
        let unset = only_measurement(&one_measurement(""));
        assert_eq!(
            (unset.byte_order, unset.word_order),
            (None, None),
            "never inferred"
        );
        for bad in [
            "byte_order = \"BIG_ENDIAN\"",
            "byte_order = \"big\"",
            "word_order = \"little_endian\"",
            "word_order = 1",
        ] {
            assert!(
                matches!(
                    SensorProfile::from_toml(&one_measurement(bad)),
                    Err(ProfileError::Syntax { .. })
                ),
                "{bad}"
            );
        }
    }

    #[test]
    fn every_validation_status_loads() {
        let cases = [
            ("synthetic", ValidationStatus::Synthetic),
            ("documented", ValidationStatus::Documented),
            ("fixture_backed", ValidationStatus::FixtureBacked),
            ("hardware_verified", ValidationStatus::HardwareVerified),
        ];
        for (name, expected) in cases {
            let text = MINIMAL.replace(
                "validation = \"synthetic\"",
                &format!("validation = \"{name}\""),
            );
            assert_eq!(load(&text).validation, expected, "{name}");
        }
        for bad in ["Synthetic", "fixturebacked", "verified", ""] {
            let text = MINIMAL.replace(
                "validation = \"synthetic\"",
                &format!("validation = \"{bad}\""),
            );
            assert!(
                matches!(
                    SensorProfile::from_toml(&text),
                    Err(ProfileError::Syntax { .. })
                ),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn schema_version_validates_directly() {
        assert_eq!(SchemaVersion::new(1), Ok(SchemaVersion::V1));
        assert_eq!(SchemaVersion::new(2), Ok(SchemaVersion::V2));
        assert_eq!(SchemaVersion::try_from(1), Ok(SchemaVersion::V1));
        assert_eq!(SchemaVersion::try_from(2), Ok(SchemaVersion::CURRENT));
        assert_eq!(
            SchemaVersion::try_from(7),
            Err(ProfileError::IncompatibleSchema { found: 7 })
        );
        assert_eq!(
            SchemaVersion::new(3),
            Err(ProfileError::IncompatibleSchema { found: 3 })
        );
        assert!(SchemaVersion::V1 < SchemaVersion::V2, "ordered by number");
        assert_eq!(SchemaVersion::V1.to_string(), "1");
        assert_eq!(SchemaVersion::CURRENT.to_string(), "2");
    }

    #[test]
    fn schema_types_deserialize_on_their_own() {
        let step: ScaleStep = toml::from_str("clamp = { min = 1, max = 2 }").expect("clamp");
        assert_eq!(step, ScaleStep::Clamp { min: 1.0, max: 2.0 });
        let range: ValueRange = toml::from_str("min = 1\nmax = 2").expect("range");
        assert_eq!((range.min, range.max), (1.0, 2.0));
        let field: StatusField =
            toml::from_str("name = \"f\"\naddress = \"a\"\nbit = 7").expect("field");
        assert_eq!((field.bit, field.invert), (7, false));
        let map: QualityMapping = toml::from_str("").expect("all lists default to empty");
        assert!(map.bad_when.is_empty() && map.uncertain_when.is_empty());
        assert!(
            toml::from_str::<SchemaVersion>("x = 1").is_err(),
            "a bare table is not a version"
        );
    }

    #[test]
    fn profile_errors_convert_to_invalid_config() {
        let cases = [
            ProfileError::Oversized { len: 2, max: 1 },
            ProfileError::Syntax { line: 1, column: 1 },
            ProfileError::IncompatibleSchema { found: 3 },
            ProfileError::InvalidIdentifier {
                field: "profile id",
            },
        ];
        for error in cases {
            let plc = PlcError::from(error);
            assert_eq!(plc.kind(), ErrorKind::InvalidConfig, "{error:?}");
            assert!(
                std::error::Error::source(&plc).is_some(),
                "source kept for {error:?}"
            );
            assert!(!plc.is_retryable(), "a bad profile is not retryable");
        }
    }
}
