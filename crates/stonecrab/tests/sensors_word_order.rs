//! The word-order rule of `SensorProfile::validate_for`: a measurement whose parsed address
//! spans several words must declare both orders, and which addresses span words is the
//! driver's to say, not the `data_type` text's. Covered with the Modbus driver, the
//! simulator and the testkit's convention double standing in for a second driver with wide
//! types (coding standards 13.5).
#![cfg(all(
    feature = "modbus",
    feature = "sim",
    feature = "sensors",
    feature = "testkit"
))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::sensors::{ProfileError, SensorProfile};
    use stonecrab::sim::SimDriver;
    use stonecrab::testkit::ConventionDriver;

    const BOTH_ORDERS: &str = "byte_order = \"big_endian\"\nword_order = \"high_word_first\"";

    /// A profile for `scheme` (and the plc4x convention, when it is Modbus) with the given
    /// `[[measurements]]` bodies.
    fn profile(scheme: &str, measurements: &[String]) -> SensorProfile {
        let convention = if scheme == "modbus-tcp" {
            "address_convention = \"plc4x\"\n"
        } else {
            ""
        };
        let mut text = format!(
            "id = \"synthetic_word_order\"\nschema_version = 1\nvalidation = \"synthetic\"\n\
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

    /// A measurement named `name` at `address`, with `extra` lines.
    fn measurement(name: &str, address: &str, data_type: &str, extra: &str) -> String {
        format!(
            "name = \"{name}\"\naddress = \"{address}\"\ndata_type = \"{data_type}\"\n\
             unit = \"L\"\n{extra}"
        )
    }

    fn modbus_wide(name: &str, data_type: &str, extra: &str) -> String {
        measurement(
            name,
            &format!("holding-register:1:{data_type}"),
            data_type,
            extra,
        )
    }

    const BOTH_MISSING: [ProfileError; 2] = [
        ProfileError::MissingByteOrder { measurement: 0 },
        ProfileError::MissingWordOrder { measurement: 0 },
    ];

    // covers: SENSORS-VALIDATION / a Modbus value wider than one register needs both orders
    #[test]
    fn modbus_addresses_wider_than_one_register_need_both_orders() {
        let driver = ModbusTcpDriver::new();
        for data_type in [
            "REAL", "DINT", "LREAL", "UDINT", "DWORD", "LINT", "ULINT", "LWORD",
        ] {
            let profile = profile("modbus-tcp", &[modbus_wide("flow", data_type, "")]);
            assert_eq!(
                profile.validate_for(&driver),
                Err(BOTH_MISSING.to_vec()),
                "{data_type} validate_for"
            );
            assert_eq!(
                profile.tags(&driver).err(),
                Some(ProfileError::MissingByteOrder { measurement: 0 }),
                "{data_type} tags"
            );
        }
    }

    // covers: SENSORS-VALIDATION / one register or one bit needs no order
    #[test]
    fn modbus_single_register_and_bit_addresses_need_no_order() {
        let driver = ModbusTcpDriver::new();
        let bodies = [
            modbus_wide("integer", "INT", ""),
            modbus_wide("word", "WORD", ""),
            measurement("switch", "coil:3", "BOOL", ""),
        ];
        let profile = profile("modbus-tcp", &bodies);
        assert_eq!(profile.validate_for(&driver), Ok(()));
        assert_eq!(profile.tags(&driver).map(|tags| tags.len()), Ok(3));
    }

    // covers: SENSORS-VALIDATION / one missing order is reported alone
    #[test]
    fn one_missing_order_is_reported_alone() {
        let driver = ModbusTcpDriver::new();
        let only_byte = profile(
            "modbus-tcp",
            &[modbus_wide("flow", "REAL", "byte_order = \"big_endian\"")],
        );
        let only_word = profile(
            "modbus-tcp",
            &[modbus_wide(
                "flow",
                "REAL",
                "word_order = \"high_word_first\"",
            )],
        );
        assert_eq!(
            only_byte.validate_for(&driver),
            Err(vec![ProfileError::MissingWordOrder { measurement: 0 }]),
        );
        assert_eq!(
            only_word.validate_for(&driver),
            Err(vec![ProfileError::MissingByteOrder { measurement: 0 }]),
        );
    }

    // covers: SENSORS-VALIDATION / declared orders satisfy the rule
    #[test]
    fn declared_orders_satisfy_the_rule() {
        let driver = ModbusTcpDriver::new();
        let profile = profile("modbus-tcp", &[modbus_wide("flow", "REAL", BOTH_ORDERS)]);
        assert_eq!(profile.validate_for(&driver), Ok(()));
        assert_eq!(profile.tags(&driver).map(|tags| tags.len()), Ok(1));
    }

    // covers: SENSORS-VALIDATION / the data_type text does not decide
    #[test]
    fn a_data_type_text_of_real_on_a_single_word_address_needs_no_order() {
        let body = measurement("level", "STATE/level:i16", "REAL", "");
        let profile = profile("sim", &[body]);
        assert_eq!(profile.validate_for(&SimDriver::new()), Ok(()));
        assert_eq!(
            profile.tags(&SimDriver::new()).map(|tags| tags.len()),
            Ok(1)
        );
    }

    // covers: SENSORS-VALIDATION / a second driver declares its own wide addresses
    #[test]
    fn a_second_drivers_wide_address_needs_both_orders() {
        let narrow_driver = ConventionDriver::new("demo");
        let wide_driver = ConventionDriver::new("demo").with_wide_addresses();
        let profile = profile("demo", &[measurement("flow", "w", "INT", "")]);
        assert_eq!(profile.validate_for(&narrow_driver), Ok(()));
        assert_eq!(
            profile.validate_for(&wide_driver),
            Err(BOTH_MISSING.to_vec()),
            "the driver says the address is wide, whatever the data type text says"
        );
        assert_eq!(
            profile.tags(&wide_driver).err(),
            Some(ProfileError::MissingByteOrder { measurement: 0 }),
        );
        let declared = self::profile("demo", &[measurement("flow", "w", "INT", BOTH_ORDERS)]);
        assert_eq!(declared.validate_for(&wide_driver), Ok(()));
    }

    // covers: SENSORS-VALIDATION / driver errors follow validate's, in document order
    #[test]
    fn validate_for_reports_validate_errors_then_driver_errors_in_document_order() {
        let bodies = [
            measurement("bad_unit", "holding-register:1:INT", "INT", "")
                .replace("unit = \"L\"", "unit = \"furlong\""),
            measurement("refused", "not an address", "INT", ""),
            modbus_wide("flow", "REAL", ""),
        ];
        assert_eq!(
            profile("modbus-tcp", &bodies).validate_for(&ModbusTcpDriver::new()),
            Err(vec![
                ProfileError::UnknownUnit { measurement: 0 },
                ProfileError::AddressRejected { measurement: 1 },
                ProfileError::MissingByteOrder { measurement: 2 },
                ProfileError::MissingWordOrder { measurement: 2 },
            ]),
        );
    }

    // covers: SENSORS-VALIDATION / an unusable convention is reported once
    #[test]
    fn an_unusable_convention_is_reported_once_and_no_address_is_parsed() {
        let text = "id = \"synthetic_word_order\"\nschema_version = 1\n\
             validation = \"synthetic\"\n[connection]\nscheme = \"demo\"\n\
             address_convention = \"plc 4x\"\n\
             [provenance]\nsource = \"Synthetic test profile\"\n\
             [[measurements]]\nname = \"flow\"\naddress = \"\"\ndata_type = \"INT\"\nunit = \"L\"\n";
        let profile = SensorProfile::from_toml(text).expect("loads");
        assert_eq!(
            profile.validate_for(&ConventionDriver::new("demo")),
            Err(vec![ProfileError::UnknownAddressConvention]),
        );
    }
}
