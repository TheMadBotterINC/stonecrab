//! The bit rules of `SensorProfile::validate_for`: a status bit lies inside the width of its
//! data, and no bit is claimed twice, whatever the spelling of its address. Covered with
//! the Modbus driver, whose grammar has several spellings of one register, and the
//! simulator, whose addresses carry their own types.
#![cfg(all(feature = "modbus", feature = "sim", feature = "sensors"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::sensors::{ProfileError, SensorProfile};
    use stonecrab::sim::SimDriver;
    use stonecrab::{ErrorKind, PlcError};

    /// Four spellings of holding register 1 as a `WORD`: long, short with four and with
    /// five digits, and bare (which needs the plc4x convention the helper declares).
    const REGISTER_1: [&str; 4] = [
        "holding-register:1:WORD",
        "4x0001:WORD",
        "4x00001:WORD",
        "400001:WORD",
    ];

    /// A profile for `scheme` (with the plc4x convention when it is Modbus) whose
    /// measurements are the given `[[measurements]]` bodies.
    fn profile(scheme: &str, measurements: &[String]) -> SensorProfile {
        let convention = if scheme == "modbus-tcp" {
            "address_convention = \"plc4x\"\n"
        } else {
            ""
        };
        let mut text = format!(
            "id = \"synthetic_overlap\"\nschema_version = 1\nvalidation = \"synthetic\"\n\
             [connection]\nscheme = \"{scheme}\"\n{convention}\
             [provenance]\nsource = \"Synthetic test profile\"\n"
        );
        for body in measurements {
            text.push_str("[[measurements]]\n");
            text.push_str(body);
            text.push('\n');
        }
        SensorProfile::from_toml(&text).expect("test document loads")
    }

    /// A `WORD` measurement at `address` with `extra` lines and these status fields, each
    /// `(address, bit)`; the field names are `f0`, `f1` and so on.
    fn measurement(address: &str, extra: &str, fields: &[(&str, u8)]) -> String {
        let list: Vec<String> = fields
            .iter()
            .enumerate()
            .map(|(position, (field, bit))| {
                format!("{{ name = \"f{position}\", address = \"{field}\", bit = {bit} }}")
            })
            .collect();
        format!(
            "name = \"level\"\naddress = \"{address}\"\ndata_type = \"WORD\"\nunit = \"1\"\n\
             {extra}\nstatus_fields = [{}]",
            list.join(", ")
        )
    }

    /// The `validate_for` errors of a Modbus profile with one measurement.
    fn modbus_errors(address: &str, extra: &str, fields: &[(&str, u8)]) -> Vec<ProfileError> {
        let profile = profile("modbus-tcp", &[measurement(address, extra, fields)]);
        profile
            .validate_for(&ModbusTcpDriver::new())
            .err()
            .unwrap_or_default()
    }

    const VALUE: &str = "holding-register:30:WORD";

    fn overlap(status_field: usize) -> ProfileError {
        ProfileError::OverlappingStatusBits {
            measurement: 0,
            status_field,
        }
    }

    fn out_of_range(status_field: usize) -> ProfileError {
        ProfileError::StatusBitOutOfRange {
            measurement: 0,
            status_field,
        }
    }

    fn in_bit_field(status_field: usize) -> ProfileError {
        ProfileError::StatusBitInBitField {
            measurement: 0,
            status_field,
        }
    }

    // covers: SENSORS-VALIDATION / two spellings of one register are one address
    #[test]
    fn every_spelling_of_one_register_collides_with_every_other() {
        for first in REGISTER_1 {
            for second in REGISTER_1 {
                let fields = [(first, 3), (second, 3)];
                let profile = profile("modbus-tcp", &[measurement(VALUE, "", &fields)]);
                let driver = ModbusTcpDriver::new();
                // The driver-free pass sees equal text only; validate_for reports each pair
                // once, whichever pass found it.
                let by_text = if first == second {
                    Err(vec![overlap(1)])
                } else {
                    Ok(())
                };
                assert_eq!(profile.validate(), by_text, "{first} then {second}");
                assert_eq!(
                    profile.validate_for(&driver),
                    Err(vec![overlap(1)]),
                    "{first} then {second}"
                );
            }
        }
    }

    // covers: SENSORS-VALIDATION / other bits and other registers do not collide
    #[test]
    fn other_bits_and_other_registers_do_not_collide() {
        let fields = [
            ("holding-register:1:WORD", 3),
            ("4x00001:WORD", 4),
            ("4x00002:WORD", 3),
            ("input-register:1:WORD", 3),
        ];
        assert_eq!(modbus_errors(VALUE, "", &fields), []);
    }

    // covers: SENSORS-VALIDATION / the later field of a spelled-differently pair is reported
    #[test]
    fn a_third_spelling_is_reported_once_and_the_first_is_never_blamed() {
        let fields = [
            ("holding-register:1:WORD", 2),
            ("4x00001:WORD", 2),
            ("400001:WORD", 2),
        ];
        assert_eq!(modbus_errors(VALUE, "", &fields), [overlap(1), overlap(2)]);
    }

    // covers: SENSORS-VALIDATION / bit addresses have aliases too
    #[test]
    fn spellings_of_one_coil_and_one_discrete_input_collide() {
        let coils = [
            ("coil:3", 0),
            ("0x00003", 0),
            ("00003", 0),
            ("0x0003:BOOL", 0),
        ];
        assert_eq!(
            modbus_errors(VALUE, "", &coils),
            [overlap(1), overlap(2), overlap(3)]
        );
        let inputs = [("discrete-input:3", 0), ("1x00003", 0), ("10003", 0)];
        assert_eq!(modbus_errors(VALUE, "", &inputs), [overlap(1), overlap(2)]);
        let different_areas = [("coil:3", 0), ("discrete-input:3", 0)];
        assert_eq!(modbus_errors(VALUE, "", &different_areas), []);
    }

    // covers: SENSORS-VALIDATION / canonical equality, not register identity
    #[test]
    fn one_register_under_another_type_or_unit_id_is_another_address() {
        let fields = [
            ("holding-register:1:WORD", 3),
            ("holding-register:1:UINT", 3),
            ("holding-register:1:WORD{unit-id: 2}", 3),
        ];
        assert_eq!(modbus_errors(VALUE, "", &fields), []);
    }

    // covers: SENSORS-VALIDATION / measurements may share a status word
    #[test]
    fn measurements_may_share_a_status_word_under_any_spelling() {
        let first = measurement(VALUE, "", &[("holding-register:1:WORD", 0)]);
        let second = measurement("holding-register:31:WORD", "", &[("4x00001:WORD", 0)])
            .replace("level", "other");
        let profile = profile("modbus-tcp", &[first, second]);
        assert_eq!(profile.validate_for(&ModbusTcpDriver::new()), Ok(()));
    }

    // covers: SENSORS-VALIDATION / a status bit inside the measurement's bit field
    #[test]
    fn a_status_bit_inside_the_bit_field_of_its_own_word_is_reported() {
        let bit_field = "scaling = [{ bit_field = { shift = 4, width = 2 } }]";
        let fields = [
            ("holding-register:30:WORD", 3),
            ("holding-register:30:WORD", 4),
            ("4x00030:WORD", 5),
            ("400030:WORD", 6),
            ("holding-register:31:WORD", 4),
            ("holding-register:30:UINT", 4),
        ];
        assert_eq!(
            modbus_errors(VALUE, bit_field, &fields),
            [in_bit_field(1), in_bit_field(2)],
            "bits 4 and 5 of the same word, under any spelling of it; bits 3 and 6, \
             another word and another type are not in the range"
        );
        assert_eq!(
            modbus_errors("4x00030:WORD", bit_field, &fields),
            [in_bit_field(1), in_bit_field(2)],
            "the measurement's own address may be spelled differently"
        );
    }

    // covers: SENSORS-VALIDATION / the bit field range is half open and may reach bit 15
    #[test]
    fn the_bit_field_range_includes_its_first_bit_and_excludes_its_end() {
        let top = "scaling = [{ bit_field = { shift = 8, width = 8 } }]";
        let edges = [(VALUE, 7), (VALUE, 8), (VALUE, 15)];
        assert_eq!(
            modbus_errors(VALUE, top, &edges),
            [in_bit_field(1), in_bit_field(2)]
        );
        let none = "scaling = [{ bit_field = { shift = 0, width = 0 } }]";
        let invalid = modbus_errors(VALUE, none, &[(VALUE, 0)]);
        assert_eq!(
            invalid,
            [ProfileError::InvalidScaleStep {
                measurement: 0,
                step: 0
            }],
            "an empty range claims nothing"
        );
    }

    // covers: SENSORS-VALIDATION / only a bit field on the raw word claims bits
    #[test]
    fn a_bit_field_after_other_steps_or_on_another_word_claims_nothing() {
        let later = "scaling = [{ offset = 0 }, { bit_field = { shift = 0, width = 16 } }]";
        assert_eq!(modbus_errors(VALUE, later, &[(VALUE, 3)]), []);
        let plain = "scaling = [{ multiply = 2 }]";
        assert_eq!(modbus_errors(VALUE, plain, &[(VALUE, 3)]), []);
        assert_eq!(
            modbus_errors(VALUE, "", &[(VALUE, 3)]),
            [],
            "no steps at all"
        );
        let other_word = "scaling = [{ bit_field = { shift = 0, width = 16 } }]";
        assert_eq!(
            modbus_errors(VALUE, other_word, &[("holding-register:31:WORD", 3)]),
            []
        );
    }

    // covers: SENSORS-VALIDATION / two bit fields compose, they do not overlap
    #[test]
    fn chained_bit_fields_are_one_extraction_and_raise_no_overlap() {
        let chain = "scaling = [{ bit_field = { shift = 4, width = 8 } }, \
                     { bit_field = { shift = 2, width = 4 } }]";
        assert_eq!(
            modbus_errors(VALUE, chain, &[("holding-register:31:WORD", 0)]),
            []
        );
        assert_eq!(
            modbus_errors(VALUE, chain, &[(VALUE, 4), (VALUE, 12)]),
            [in_bit_field(0)],
            "the first step's range is the claim: bits 4 to 11 of the word"
        );
    }

    // covers: SENSORS-VALIDATION / status bit width follows the parsed type
    #[test]
    fn a_status_bit_must_lie_inside_the_width_of_its_data() {
        let cases = [
            ("coil:1", 1_u8),
            ("discrete-input:1", 1),
            ("holding-register:1:SINT", 8),
            ("holding-register:1:USINT", 8),
            ("holding-register:1:WORD", 16),
            ("holding-register:1:INT", 16),
            ("holding-register:1:UINT", 16),
            ("holding-register:1:DWORD", 32),
            ("holding-register:1:DINT", 32),
            ("holding-register:1:UDINT", 32),
            ("holding-register:1:LWORD", 64),
            ("holding-register:1:LINT", 64),
            ("holding-register:1:ULINT", 64),
        ];
        for (address, width) in cases {
            let last = width - 1;
            assert_eq!(
                modbus_errors(VALUE, "", &[(address, last)]),
                [],
                "{address} bit {last}"
            );
            if width < 64 {
                assert_eq!(
                    modbus_errors(VALUE, "", &[(address, width)]),
                    [out_of_range(0)],
                    "{address} bit {width}"
                );
            }
            assert_eq!(
                modbus_errors(VALUE, "", &[(address, 64)]),
                [out_of_range(0)],
                "{address} bit 64 is reported once, by whichever rule sees it first"
            );
            assert_eq!(
                modbus_errors(VALUE, "", &[(address, u8::MAX)]),
                [out_of_range(0)],
                "{address} bit 255"
            );
        }
    }

    // covers: SENSORS-VALIDATION / a coil is one bit, so only bit 0 exists
    #[test]
    fn a_coil_status_address_has_only_bit_zero() {
        let fields = [("coil:2", 0), ("coil:3", 1), ("0x00004", 7)];
        assert_eq!(
            modbus_errors(VALUE, "", &fields),
            [out_of_range(1), out_of_range(2)]
        );
    }

    // covers: SENSORS-VALIDATION / the driver-free pass bounds bits by 64
    #[test]
    fn validate_alone_reports_bit_64_and_leaves_narrower_widths_to_the_driver() {
        let profile = profile(
            "modbus-tcp",
            &[measurement(
                VALUE,
                "",
                &[("holding-register:1:WORD", 16), ("coil:1", 64)],
            )],
        );
        assert_eq!(profile.validate(), Err(vec![out_of_range(1)]));
        assert_eq!(
            profile.validate_for(&ModbusTcpDriver::new()),
            Err(vec![out_of_range(1), out_of_range(0)]),
            "the driver-free error first, then the width error"
        );
    }

    // covers: SENSORS-VALIDATION / widths come from the driver's type, not from a driver
    #[test]
    fn the_simulator_types_bound_status_bits_and_other_types_are_not_checked() {
        let body = "name = \"level\"\naddress = \"STATE/level:i16\"\ndata_type = \"INT\"\n\
                    unit = \"1\"\nstatus_fields = [\n\
                    { name = \"narrow\", address = \"STATE/narrow:u8\", bit = 8 },\n\
                    { name = \"word\", address = \"STATE/word:u16\", bit = 15 },\n\
                    { name = \"wide\", address = \"STATE/wide:i32\", bit = 32 },\n\
                    { name = \"flag\", address = \"STATE/flag:bool\", bit = 1 },\n\
                    { name = \"real\", address = \"STATE/real:f32\", bit = 40 },\n\
                    ]";
        let profile = profile("sim", &[body.to_owned()]);
        assert_eq!(
            profile.validate_for(&SimDriver::new()),
            Err(vec![out_of_range(0), out_of_range(2), out_of_range(3)]),
            "u8, i32 and bool are too narrow; a float has no bit width to check"
        );
    }

    // covers: SENSORS-VALIDATION / a refused status address is reported by validate_for
    #[test]
    fn a_refused_status_address_is_reported_once_and_in_document_order() {
        let fields = [("holding-register:1:WORD", 1), ("canary-status-text", 0)];
        let profile = profile("modbus-tcp", &[measurement(VALUE, "", &fields)]);
        let driver = ModbusTcpDriver::new();
        let rejected = ProfileError::StatusAddressRejected {
            measurement: 0,
            status_field: 1,
        };
        assert_eq!(profile.validate_for(&driver), Err(vec![rejected]));
        assert_eq!(profile.tags(&driver).err(), Some(rejected));
        assert!(!rejected.to_string().contains("canary"), "no address text");
    }

    // covers: SENSORS-VALIDATION / errors keep document order and never echo the document
    #[test]
    fn every_bit_error_is_reported_in_field_order_and_converts_to_invalid_config() {
        let bit_field = "scaling = [{ bit_field = { shift = 0, width = 4 } }]";
        let fields = [
            ("canary-status-text", 0),
            ("holding-register:31:WORD", 16),
            ("4x00031:WORD", 16),
            ("4x00030:WORD", 2),
        ];
        let errors = modbus_errors(VALUE, bit_field, &fields);
        assert_eq!(
            errors,
            [
                ProfileError::StatusAddressRejected {
                    measurement: 0,
                    status_field: 0
                },
                out_of_range(1),
                out_of_range(2),
                overlap(2),
                in_bit_field(3),
            ]
        );
        for error in &errors {
            let text = error.to_string();
            for echo in ["canary", "4x000", "holding-register"] {
                assert!(!text.contains(echo), "{text} echoes {echo}");
            }
            assert_eq!(PlcError::from(*error).kind(), ErrorKind::InvalidConfig);
        }
        let profile = profile("modbus-tcp", &[measurement(VALUE, bit_field, &fields)]);
        assert_eq!(
            profile.tags(&ModbusTcpDriver::new()).err(),
            errors.first().copied(),
            "tags returns the first"
        );
    }

    // covers: SENSORS-VALIDATION / a refused measurement address does not hide status errors
    #[test]
    fn a_refused_measurement_address_does_not_hide_status_errors() {
        let bit_field = "scaling = [{ bit_field = { shift = 0, width = 4 } }]";
        assert_eq!(
            modbus_errors("canary-address", bit_field, &[("coil:1", 1), (VALUE, 1)]),
            [
                ProfileError::AddressRejected { measurement: 0 },
                out_of_range(0),
            ],
            "the value's canonical address is unknown, so no bit field claim is made"
        );
    }

    // covers: SENSORS-VALIDATION / the overlap rule agrees with an independent restatement
    proptest! {
        #[test]
        fn overlaps_across_spellings_match_an_independent_oracle(
            picks in prop::collection::vec((1..3usize, 0..4usize, 0..4u8), 0..8),
        ) {
            let spelling = |register: usize, form: usize| -> String {
                let number = register;
                match form {
                    0 => format!("holding-register:{number}:WORD"),
                    1 => format!("4x{number:04}:WORD"),
                    2 => format!("4x{number:05}:WORD"),
                    _ => format!("4{number:05}:WORD"),
                }
            };
            let rows: Vec<(usize, u8, String)> = picks
                .iter()
                .map(|(register, form, bit)| (*register, *bit, spelling(*register, *form)))
                .collect();
            let fields: Vec<(&str, u8)> =
                rows.iter().map(|(_, bit, text)| (text.as_str(), *bit)).collect();
            // Equal text is the driver-free pass's, so those come first; the rest follow.
            let (mut by_text, mut by_alias) = (Vec::new(), Vec::new());
            for (position, row) in rows.iter().enumerate() {
                let same_bit = |earlier: &&(usize, u8, String)| earlier.0 == row.0 && earlier.1 == row.1;
                let earlier = rows.iter().take(position).filter(same_bit);
                if earlier.clone().any(|other| other.2 == row.2) {
                    by_text.push(overlap(position));
                } else if earlier.clone().next().is_some() {
                    by_alias.push(overlap(position));
                }
            }
            by_text.extend(by_alias);
            let expected = by_text;
            prop_assert_eq!(modbus_errors(VALUE, "", &fields), expected);
        }
    }
}
