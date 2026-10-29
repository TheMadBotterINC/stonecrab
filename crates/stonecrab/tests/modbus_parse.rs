//! The address text grammar: accepted forms, defaults, offsets and every rejection.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::modbus::{AddressConvention, ModbusAddress, ModbusArea, ModbusDataType, UnitId};
    use stonecrab::{AddressOptions, ByteOrder, ErrorKind, PlcError, ProtocolAddress, WordOrder};

    fn none() -> AddressOptions {
        AddressOptions::default()
    }

    fn plc4x() -> AddressOptions {
        AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("plc4x is a valid convention name")
    }

    fn accept(text: &str) -> ModbusAddress {
        ModbusAddress::parse(text, &none()).unwrap_or_else(|e| panic!("{text:?} rejected: {e}"))
    }

    fn accept_plc4x(text: &str) -> ModbusAddress {
        ModbusAddress::parse(text, &plc4x()).unwrap_or_else(|e| panic!("{text:?} rejected: {e}"))
    }

    /// The error of `text` under `options`, after checking it is `InvalidAddress`.
    fn reject_with(text: &str, options: &AddressOptions) -> PlcError {
        match ModbusAddress::parse(text, options) {
            Ok(address) => panic!("{text:?} should be rejected, got {address:?}"),
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind of {text:?}");
                error
            }
        }
    }

    /// The message of the `InvalidAddress` error for `text` with no convention.
    fn reject(text: &str) -> String {
        reject_with(text, &none()).to_string()
    }

    fn assert_rejected_naming(text: &str, needle: &str) {
        let message = reject(text);
        assert!(
            message.contains(needle),
            "message for {text:?} should contain {needle:?}: {message}"
        );
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn holding_register_16_int_is_wire_offset_15() {
        let address = accept("holding-register:16:INT");
        assert_eq!(address.area(), ModbusArea::HoldingRegister, "area");
        assert_eq!(address.offset(), 15, "wire offset is logical minus one");
        assert_eq!(address.logical_address(), 16, "logical address");
        assert_eq!(address.data_type(), ModbusDataType::Int, "type");
        assert_eq!(address.unit_id(), None, "no unit written");
        assert_eq!(address.byte_order(), None, "no byte order written");
        assert_eq!(
            address.convention(),
            AddressConvention::Plc4xCompatible,
            "convention"
        );
        assert_eq!(
            accept_plc4x("holding-register:16:INT"),
            address,
            "the long form means the same with an explicit convention"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn holding_register_0x0010_is_rejected() {
        let message = reject("holding-register:0x0010");
        assert!(
            message.contains("hexadecimal"),
            "message names hex digits: {message}"
        );
        assert_rejected_naming("holding-register:0x0010:INT", "hexadecimal");
        assert_rejected_naming("holding-register:00FF", "hexadecimal");
        // The project brief's example is not upstream syntax at all.
        assert_rejected_naming("holding:0x0010:i16", "unrecognised address");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn lowercase_i16_is_rejected() {
        let message = reject("holding-register:16:i16");
        assert!(
            message.contains("upper case") && message.contains("i16"),
            "message says names are upper case: {message}"
        );
        assert_rejected_naming("holding-register:16:int", "upper case");
        assert_rejected_naming("holding-register:16:INT16", "unknown Modbus data type");
        assert_rejected_naming("holding-register:16:", "missing data type");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn logical_address_zero_is_rejected() {
        for text in [
            "holding-register:0",
            "coil:000000000",
            "discrete-input:0:BOOL",
            "input-register:0",
            "4x00000",
            "0x0000",
        ] {
            assert_rejected_naming(text, "logical address 0");
        }
        let error = reject_with("40000", &plc4x());
        assert!(
            error.to_string().contains("logical address 0"),
            "bare form too: {error}"
        );
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn default_types_are_bool_for_bits_and_int_for_registers() {
        for (text, area, data_type) in [
            ("coil:1", ModbusArea::Coil, ModbusDataType::Bool),
            (
                "discrete-input:1",
                ModbusArea::DiscreteInput,
                ModbusDataType::Bool,
            ),
            (
                "holding-register:1",
                ModbusArea::HoldingRegister,
                ModbusDataType::Int,
            ),
            (
                "input-register:1",
                ModbusArea::InputRegister,
                ModbusDataType::Int,
            ),
            ("0x0001", ModbusArea::Coil, ModbusDataType::Bool),
            ("1x0001", ModbusArea::DiscreteInput, ModbusDataType::Bool),
            ("4x0001", ModbusArea::HoldingRegister, ModbusDataType::Int),
            ("3x0001", ModbusArea::InputRegister, ModbusDataType::Int),
        ] {
            let address = accept(text);
            assert_eq!(address.area(), area, "area of {text}");
            assert_eq!(address.data_type(), data_type, "default type of {text}");
            assert_eq!(address.offset(), 0, "offset of {text}");
        }
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn every_accepted_type_name_parses_on_a_register_area() {
        for name in [
            "BOOL", "BYTE", "WORD", "DWORD", "LWORD", "SINT", "USINT", "INT", "UINT", "DINT",
            "UDINT", "LINT", "ULINT", "REAL", "LREAL",
        ] {
            let address = accept(&format!("input-register:100:{name}"));
            assert_eq!(address.data_type().upstream_name(), name, "type {name}");
        }
    }

    // covers: MB-ADDR-SHORT-4X
    #[test]
    fn four_x_00016_is_logical_16() {
        let address = accept("4x00016:INT");
        assert_eq!(address.logical_address(), 16, "logical");
        assert_eq!(address.offset(), 15, "wire");
        assert_eq!(
            address,
            accept("holding-register:16:INT"),
            "same as long form"
        );
        assert_eq!(accept("4x0016"), address, "four digits mean the same");
        assert_eq!(accept("3x00016").area(), ModbusArea::InputRegister, "3x");
        assert_eq!(accept("1x00016").area(), ModbusArea::DiscreteInput, "1x");
        assert_eq!(accept("4x65536").offset(), 65_535, "last register");
    }

    // covers: MB-ADDR-SHORT-4X
    #[test]
    fn zero_x_0010_is_a_decimal_coil_address_not_hex() {
        let address = accept("0x0010");
        assert_eq!(address.area(), ModbusArea::Coil, "0x is the coil prefix");
        assert_eq!(address.logical_address(), 10, "decimal ten, not sixteen");
        assert_eq!(address.offset(), 9, "wire offset nine");
        assert_eq!(address.data_type(), ModbusDataType::Bool, "coil default");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn short_forms_require_four_or_five_digits() {
        for prefix in ["0x", "1x", "3x", "4x"] {
            for accepted in ["0001", "00001", "9999", "12345"] {
                let text = format!("{prefix}{accepted}");
                assert_eq!(
                    accept(&text).logical_address(),
                    accepted.parse::<u32>().expect("digits"),
                    "{text} is accepted"
                );
            }
            for rejected in ["", "1", "001", "000001", "1234567890123456789012"] {
                let text = format!("{prefix}{rejected}");
                let message = reject(&text);
                assert!(
                    message.contains("4 or 5 digits"),
                    "{text} names the digit rule: {message}"
                );
            }
        }
        // Uppercase X is not the prefix.
        assert_rejected_naming("4X00016", "unrecognised address");
        // A non-digit before the lowercase x is not a prefix either (not "prefix digit N").
        assert_rejected_naming("ax00016", "unrecognised address");
        assert_rejected_naming("xx00016", "unrecognised address");
        assert_rejected_naming("4x00016 ", "unexpected text");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn four_x_without_digits_is_rejected() {
        assert_rejected_naming("4x:INT", "needs 4 or 5 digits");
        assert_rejected_naming("0x", "needs 4 or 5 digits");
        assert_rejected_naming("3x{unit-id: 1}", "needs 4 or 5 digits");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn bare_40001_is_rejected_unless_convention_is_explicit() {
        for text in ["40001", "400001", "00001", "000001", "10001", "30001:INT"] {
            let message = reject(text);
            assert!(
                message.contains("ambiguous"),
                "{text} names the ambiguity: {message}"
            );
            assert!(
                message.contains("-register:N")
                    || message.contains("coil:N")
                    || message.contains("discrete-input:N"),
                "{text} names the long spelling: {message}"
            );
            assert!(
                message.contains("xNNNNN") && message.contains("x00001"),
                "{text} names the prefixed spelling: {message}"
            );
            assert!(
                message.contains("address-convention to plc4x, one-based-40001 or zero-based"),
                "{text} says how to opt in: {message}"
            );
        }
        assert_eq!(
            ModbusAddress::parse("40001", &AddressOptions::default()).map_err(|e| e.kind()),
            Err(ErrorKind::InvalidAddress),
            "an absent convention is not an explicit one"
        );
        // The unambiguous spellings of the same register stay accepted.
        assert_eq!(accept("4x00001").offset(), 0, "prefixed form");
        assert_eq!(accept("holding-register:1").offset(), 0, "long form");
    }

    // covers: MB-ADDR-BARE-40001
    #[test]
    fn bare_40001_with_plc4x_convention_is_logical_1() {
        for (text, area, logical) in [
            ("40001", ModbusArea::HoldingRegister, 1),
            ("400001", ModbusArea::HoldingRegister, 1),
            ("49999", ModbusArea::HoldingRegister, 9_999),
            ("00001", ModbusArea::Coil, 1),
            ("000001", ModbusArea::Coil, 1),
            ("10001", ModbusArea::DiscreteInput, 1),
            ("30001", ModbusArea::InputRegister, 1),
        ] {
            let address = accept_plc4x(text);
            assert_eq!(address.area(), area, "area of {text}");
            assert_eq!(address.logical_address(), logical, "logical of {text}");
        }
        let typed = accept_plc4x("40001:REAL{unit-id: 2}");
        assert_eq!(typed.data_type(), ModbusDataType::Real, "type after bare");
        assert_eq!(typed.unit_id(), Some(UnitId::from(2)), "block after bare");
        // Wrong lengths and unknown prefixes stay rejected with the convention.
        for text in [
            "4001", "4000001", "2000", "20001", "50001", "70001", "90001",
        ] {
            reject_with(text, &plc4x());
        }
        assert!(
            reject_with("20001", &plc4x())
                .to_string()
                .contains("no area uses prefix digit 2"),
            "unknown prefix named"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn unknown_convention_name_is_rejected() {
        for name in ["PLC4X", "plc4x2", "zero_based", "one-based", "x"] {
            let options = AddressOptions::builder()
                .convention(name)
                .build()
                .expect("a valid carrier");
            // Even an unambiguous spelling is refused, so a typo never goes unnoticed.
            for text in ["holding-register:1", "4x00001", "40001", ""] {
                let error = reject_with(text, &options);
                assert!(
                    error.to_string().contains("unknown address convention"),
                    "{name} with {text:?}: {error}"
                );
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn typed_coil_address_is_rejected_at_parse_time() {
        for name in ["INT", "WORD", "REAL", "LREAL", "BYTE", "UDINT"] {
            let text = format!("coil:1:{name}");
            assert_rejected_naming(&text, "bit areas read BOOL only");
            assert_rejected_naming(&text, "coil");
        }
        assert_rejected_naming("0x0001:INT", "bit areas read BOOL only");
        assert_eq!(
            accept("coil:1:BOOL").data_type(),
            ModbusDataType::Bool,
            "BOOL"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn typed_discrete_input_address_is_rejected_at_parse_time() {
        for name in ["INT", "DINT", "REAL"] {
            let text = format!("discrete-input:1:{name}");
            assert_rejected_naming(&text, "bit areas read BOOL only");
            assert_rejected_naming(&text, "discrete-input");
        }
        assert_rejected_naming("1x00001:UINT", "bit areas read BOOL only");
        assert_eq!(
            accept("discrete-input:1:BOOL").data_type(),
            ModbusDataType::Bool,
            "BOOL"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn address_above_65535_is_rejected() {
        // Wire offset 65535 is logical 65536; the next logical address has no wire offset.
        for text in [
            "holding-register:65537",
            "coil:100000",
            "input-register:999999999",
            "4x65537",
            "4x99999",
            "1x70000",
        ] {
            assert_rejected_naming(text, "above 65536");
        }
        let message = reject("holding-register:65537");
        assert!(
            message.contains("65535"),
            "names the last wire offset: {message}"
        );
        let bare = reject_with("465537", &plc4x()).to_string();
        assert!(bare.contains("above 65536"), "bare form too: {bare}");
        assert_eq!(
            accept_plc4x("465536").offset(),
            65_535,
            "bare form at the top"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn span_past_65535_is_rejected() {
        for text in [
            "holding-register:65536:DINT",
            "holding-register:65536:REAL",
            "input-register:65536:UDINT",
            "holding-register:65536:DWORD",
            "holding-register:65534:LINT",
            "4x65535:LINT",
        ] {
            assert_rejected_naming(text, "passes the last address 65535");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn lreal_at_top_of_map_is_rejected() {
        for text in [
            "holding-register:65534:LREAL",
            "holding-register:65535:LREAL",
            "holding-register:65536:LREAL",
            "input-register:65534:ULINT",
        ] {
            assert_rejected_naming(text, "passes the last address 65535");
        }
        let top = accept("holding-register:65533:LREAL");
        assert_eq!(top.offset(), 65_532, "the last LREAL starts at wire 65532");
    }

    // covers: MB-LIMIT-SPAN
    #[test]
    fn last_valid_register_is_accepted() {
        let last = accept("holding-register:65536");
        assert_eq!(last.offset(), 65_535, "wire address 65535 is reachable");
        assert_eq!(last.logical_address(), 65_536, "logical 65536");
        assert_eq!(accept("coil:65536").offset(), 65_535, "last coil");
        assert_eq!(
            accept("input-register:65535:DINT").offset(),
            65_534,
            "last DINT"
        );
        assert_eq!(
            accept("input-register:65533:LINT").offset(),
            65_532,
            "last LINT"
        );
        assert_eq!(accept("4x65536:INT"), last, "short form agrees");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn overlong_digit_group_is_rejected() {
        // Nine digits are within the regex bound; the value decides.
        assert_eq!(
            accept("coil:000000001").logical_address(),
            1,
            "nine digits with leading zeros"
        );
        assert_rejected_naming("coil:999999999", "above 65536");
        for text in [
            "coil:0000000001",
            "holding-register:1234567890",
            "input-register:99999999999999999999999999999999",
            "discrete-input:00000000000:BOOL",
        ] {
            assert_rejected_naming(text, "takes 1 to 9");
        }
        // Short and bare forms count 4 or 5 digits.
        assert_rejected_naming("4x123456", "4 or 5 digits");
        assert_rejected_naming("4x1234567890123456", "4 or 5 digits");
        reject_with("4000001", &plc4x());
        reject_with("40000000000000000000", &plc4x());
        // Nothing echoes a long digit run back.
        let message = reject("holding-register:99999999999999999999999999999999");
        assert!(!message.contains("99999999999"), "no digit echo: {message}");
    }

    // covers: MB-ADDR-CONFIG-BLOCK
    #[test]
    fn unit_id_block_sets_unit() {
        for (text, unit) in [
            ("holding-register:8:SINT{unit-id: 10}", 10),
            ("holding-register:8:SINT{unit-id:10}", 10),
            ("holding-register:8:SINT{ unit-id : 10 }", 10),
            ("holding-register:8:SINT{unit-id: 0}", 0),
            ("holding-register:8:SINT{unit-id: 255}", 255),
            ("holding-register:8:SINT{unit-id: 007}", 7),
            ("4x00008:SINT{unit-id: 10}", 10),
            ("holding-register:8{unit-id: 10}", 10),
        ] {
            let address = accept(text);
            assert_eq!(address.unit_id(), Some(UnitId::from(unit)), "{text}");
            assert_eq!(address.offset(), 7, "offset of {text}");
        }
        assert_eq!(
            accept("coil:1{unit-id: 3}").data_type(),
            ModbusDataType::Bool,
            "default type"
        );
        let empty = accept("coil:1{}");
        assert_eq!(empty, accept("coil:1"), "an empty block sets nothing");
        assert_eq!(accept("coil:1{   }"), empty, "a blank block sets nothing");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn unit_id_256_is_rejected() {
        assert_rejected_naming("holding-register:1{unit-id: 256}", "above 255");
        assert_rejected_naming("holding-register:1{unit-id: 65536}", "above 255");
        assert_rejected_naming(
            "holding-register:1{unit-id: 99999999999999999999999999999999}",
            "above 255",
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn negative_unit_id_is_rejected() {
        assert_rejected_naming("holding-register:1{unit-id: -1}", "is negative");
        assert_rejected_naming("holding-register:1{unit-id: -0}", "is negative");
        for value in ["", "abc", "1.5", "'10'", "\"10\"", "+1", "0x10", "1 2"] {
            let text = format!("holding-register:1{{unit-id: {value}}}");
            assert_rejected_naming(&text, "is not a decimal integer");
        }
    }

    // covers: MB-ADDR-CONFIG-BLOCK
    #[test]
    fn byte_order_names_map_per_adr_0006() {
        for (name, byte, word) in [
            ("BIG_ENDIAN", ByteOrder::BigEndian, WordOrder::HighWordFirst),
            (
                "LITTLE_ENDIAN",
                ByteOrder::LittleEndian,
                WordOrder::LowWordFirst,
            ),
            (
                "BIG_ENDIAN_BYTE_SWAP",
                ByteOrder::LittleEndian,
                WordOrder::HighWordFirst,
            ),
            (
                "LITTLE_ENDIAN_BYTE_SWAP",
                ByteOrder::BigEndian,
                WordOrder::LowWordFirst,
            ),
        ] {
            for quoted in [format!("'{name}'"), format!("\"{name}\"")] {
                let text = format!("holding-register:1:REAL{{byte-order: {quoted}}}");
                assert_eq!(accept(&text).byte_order(), Some((byte, word)), "{text}");
            }
            let text = format!("holding-register:1:REAL{{byte-order: '{name}'}}");
            assert!(
                accept(&text).canonical().contains(name),
                "canonical text names {name}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn byte_order_block_requires_quoted_name() {
        for value in [
            "LITTLE_ENDIAN",
            "BIG_ENDIAN_BYTE_SWAP",
            "",
            "'LITTLE_ENDIAN",
            "LITTLE_ENDIAN'",
            "'LITTLE_ENDIAN\"",
        ] {
            let text = format!("holding-register:1:REAL{{byte-order: {value}}}");
            assert_rejected_naming(&text, "must be quoted");
        }
        for value in ["'little_endian'", "'MIDDLE_ENDIAN'", "''", "'BIG_ENDIAN '"] {
            let text = format!("holding-register:1:REAL{{byte-order: {value}}}");
            assert_rejected_naming(&text, "unknown byte-order");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn unknown_byte_order_name_is_rejected() {
        for name in [
            "BIG",
            "big_endian",
            "MIDDLE_ENDIAN",
            "BIG_ENDIAN_WORD_SWAP",
            " ",
        ] {
            let text = format!("holding-register:1:REAL{{byte-order: '{name}'}}");
            assert_rejected_naming(&text, "unknown byte-order");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn unknown_config_key_is_rejected() {
        assert_rejected_naming(
            "holding-register:1{unitid: 1}",
            "unknown address block key 'unitid'",
        );
        assert_rejected_naming(
            "holding-register:1{Unit-Id: 1}",
            "unknown address block key",
        );
        assert_rejected_naming(
            "holding-register:1{unit-identifier: 1}",
            "unknown address block key",
        );
        assert_rejected_naming("holding-register:1{: 1}", "unknown address block key ''");
        assert_rejected_naming(
            "holding-register:1{scale: 2.5}",
            "unknown address block key",
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn malformed_config_block_is_rejected() {
        assert_rejected_naming("holding-register:1{unit-id: 1", "not terminated");
        assert_rejected_naming("holding-register:1{unit-id}", "not 'key: value'");
        assert_rejected_naming("holding-register:1{unit-id: 1,}", "not 'key: value'");
        assert_rejected_naming("holding-register:1{,unit-id: 1}", "not 'key: value'");
        assert_rejected_naming(
            "holding-register:1{unit-id: 1, unit-id: 2}",
            "more than once",
        );
        assert_rejected_naming(
            "holding-register:1{byte-order: 'BIG_ENDIAN', byte-order: 'BIG_ENDIAN'}",
            "more than once",
        );
        assert_rejected_naming("holding-register:1{unit-id: 1}}", "unexpected '}'");
        assert_rejected_naming("{}", "address before the config block is empty");
        assert_rejected_naming("holding-register:1:INT ", "(whitespace)");
        assert_rejected_naming("holding-register:1 {unit-id: 1}", "unexpected text");
        assert_rejected_naming("holding-register:1{unit-id: 1}x", "not terminated");
        assert_rejected_naming("{unit-id: 1}", "address before the config block is empty");
        assert_rejected_naming("holding-register:1}", "unexpected text");
    }

    // covers: MB-ADDR-CONFIG-BLOCK
    #[test]
    fn both_keys_combine_in_any_order() {
        let a = accept("holding-register:1:REAL{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}");
        let b = accept("holding-register:1:REAL{byte-order:'LITTLE_ENDIAN',unit-id:10}");
        assert_eq!(a, b, "order of keys and spacing do not matter");
        assert_eq!(a.unit_id(), Some(UnitId::from(10)), "unit");
        assert_eq!(
            a.byte_order(),
            Some((ByteOrder::LittleEndian, WordOrder::LowWordFirst)),
            "order"
        );
        assert_eq!(
            a.canonical(),
            "holding-register:1:REAL{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}",
            "canonical order"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register; MB-TYPE-STRING,
    // MB-TYPE-WSTRING (upstream ModbusStringTest.refusesAStringWithoutALength)
    #[test]
    fn a_string_without_a_length_is_rejected_at_parse_time() {
        assert_rejected_naming("holding-register:8:STRING", "STRING(20)");
        assert_rejected_naming("input-register:1:WSTRING", "WSTRING(20)");
        assert_rejected_naming("holding-register:1[0..2]:STRING", "requires the length");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register; MB-TYPE-STRING (upstream
    // ModbusTagTest.aStringLengthTooWideToBeANumberIsAlsoAnInvalidTag and the `\d{1,5}` of
    // the tag pattern)
    #[test]
    fn a_string_length_is_one_to_five_digits_from_1_to_65535() {
        assert_eq!(
            accept("holding-register:1:STRING(1)")
                .string_length()
                .map(std::num::NonZeroU16::get),
            Some(1)
        );
        assert_eq!(
            accept("holding-register:1:WSTRING(00125)")
                .string_length()
                .map(std::num::NonZeroU16::get),
            Some(125),
            "leading zeros, as upstream's Integer.parseInt reads them"
        );
        for (text, needle) in [
            ("holding-register:1:STRING(0)", "1 to 65535"),
            ("holding-register:1:STRING(99999)", "1 to 65535"),
            (
                "holding-register:1:STRING(99999999999)",
                "1 to 5 decimal digits",
            ),
            ("holding-register:1:STRING()", "1 to 5 decimal digits"),
            ("holding-register:1:STRING(20", "1 to 5 decimal digits"),
            ("holding-register:1:STRING(x)", "1 to 5 decimal digits"),
            ("holding-register:1:STRING(-1)", "1 to 5 decimal digits"),
            ("holding-register:1:STRING(20)(4)", "unexpected text"),
            ("holding-register:1:STRING(251)", "MB-LIMIT-REGISTERS"),
            ("holding-register:1:WSTRING(126)", "MB-LIMIT-REGISTERS"),
            ("extended-register:1:STRING(241)", "MB-LIMIT-REGISTERS"),
        ] {
            assert_rejected_naming(text, needle);
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register; MB-ADDR-BIT-TYPES,
    // MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING
    #[test]
    fn text_types_on_a_bit_area_are_rejected_at_parse_time() {
        for text in [
            "coil:1:CHAR",
            "discrete-input:1:WCHAR",
            "coil:1:STRING(2)",
            "1x00001:WSTRING(2)",
        ] {
            assert_rejected_naming(text, "bit areas read BOOL only");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register; MB-TYPE-STRING (upstream
    // ModbusStringTest.refusesALengthOnANonStringType)
    #[test]
    fn parenthesised_length_is_rejected() {
        assert_rejected_naming("holding-register:8:INT(20)", "only supported for STRING");
        assert_rejected_naming("holding-register:1:CHAR(2)", "not for CHAR");
        assert_rejected_naming("coil:1:BOOL(1)", "only supported for STRING");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn time_family_names_are_rejected_at_parse_time() {
        for name in [
            "TIME",
            "LTIME",
            "DATE",
            "LDATE",
            "TIME_OF_DAY",
            "LTIME_OF_DAY",
            "DATE_AND_TIME",
            "LDATE_AND_TIME",
        ] {
            let text = format!("holding-register:1:{name}");
            assert_rejected_naming(&text, "calendar types cannot be encoded");
        }
    }

    // The four spellings of extended register 1 of ModbusTagTest.testExtended_INT_ARRAY_RANGE
    // (plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusTagTest.java,
    // Apache PLC4X v1.0.0 commit ea229918), here without the array selection: "All forms of
    // defining the tag MUST result in an identical modbus tag", and logical address 1 is wire
    // address 1 ("Addresses for extended memory start at address 0 instead of 1"). The test
    // with its selection is `upstream_extended_register_ranges_agree_across_forms_up_to_120`
    // in tests/modbus_selection.rs (MB-ADDR-SELECTION).
    // covers: MB-ADDR-EXTENDED
    #[test]
    fn the_four_spellings_of_extended_register_1_are_one_address() {
        let long = accept("extended-register:1:INT");
        for text in ["extended-register:1", "6x00001", "6x00001:INT", "6x0001"] {
            assert_eq!(accept(text), long, "{text}");
        }
        for text in ["60001", "600001", "60001:INT"] {
            assert_eq!(accept_plc4x(text), long, "{text} with the plc4x convention");
        }
        assert_eq!(long.area(), ModbusArea::ExtendedRegister, "area");
        assert_eq!(long.logical_address(), 1, "logical address");
        assert_eq!(long.offset(), 1, "record 1 ...");
        assert_eq!(
            long.file_number().map(std::num::NonZeroU16::get),
            Some(1),
            "... of file 1"
        );
        assert_eq!(
            long.canonical(),
            "extended-register:1:INT",
            "canonical text"
        );
        assert_eq!(accept(&long.canonical()), long, "reads back");
    }

    // covers: MB-ADDR-EXTENDED, MB-ADDR-BARE-40001
    #[test]
    fn the_bare_6_form_needs_an_explicit_convention_like_the_other_areas() {
        for text in ["60001", "600001", "60001:INT"] {
            let message = reject(text);
            assert!(
                message.contains("ambiguous") && message.contains("extended-register"),
                "{text}: {message}"
            );
        }
        for convention in ["plc4x", "one-based-40001", "zero-based"] {
            assert_eq!(
                ModbusAddress::parse("60001", &with(convention))
                    .expect("accepted")
                    .logical_address(),
                1,
                "{convention}"
            );
        }
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn an_extended_address_is_the_register_address_under_every_convention() {
        for convention in ["plc4x", "one-based-40001", "zero-based"] {
            let options = with(convention);
            for (text, number) in [
                ("extended-register:1", 1_u32),
                ("extended-register:9999", 9_999),
                ("extended-register:10000", 10_000),
                ("extended-register:29998:DINT", 29_998),
                ("6x65535", 65_535),
                ("6x65536", 65_536),
                ("665536", 65_536),
                ("extended-register:655349999", 655_349_999),
            ] {
                let address = ModbusAddress::parse(text, &options)
                    .unwrap_or_else(|e| panic!("{text} under {convention}: {e}"));
                assert_eq!(
                    address.logical_address(),
                    number,
                    "{text} under {convention}"
                );
            }
        }
        // File numbers are 1-based on the wire: address 10000 is record 0 of file 2.
        let boundary = accept("extended-register:10000");
        assert_eq!(
            boundary.file_number().map(std::num::NonZeroU16::get),
            Some(2)
        );
        assert_eq!(boundary.offset(), 0);
    }

    // Upstream rejects logical address 0 (ModbusTag.java:195-197, "address must be greater
    // than zero") and addresses over 655359999 (ModbusTagExtendedRegister.java:36, 86-88).
    // covers: MB-ADDR-EXTENDED, MB-LIMIT-EXT
    #[test]
    fn extended_address_0_and_the_end_of_the_files_are_rejected() {
        for convention in ["plc4x", "one-based-40001", "zero-based"] {
            let options = with(convention);
            for text in ["extended-register:0", "6x00000", "60000", "6x0000"] {
                let error = reject_with(text, &options).to_string();
                assert!(
                    error.contains("address 0 does not exist"),
                    "{text}: {error}"
                );
            }
        }
        for text in [
            "extended-register:655350000",
            "extended-register:655359999",
            "extended-register:999999999",
            "extended-register:655349999:DINT",
            "extended-register:655349997:LREAL",
        ] {
            let error = reject(text);
            assert!(error.contains("passes the last address"), "{text}: {error}");
        }
        assert_eq!(
            accept("extended-register:655349998:DINT").logical_address(),
            655_349_998,
            "the last DINT"
        );
        assert!(
            reject("extended-register:1000000000").contains("digits"),
            "ten digits"
        );
        assert!(reject("extended-register:").contains("missing address number"));
        assert!(reject("extended-register").contains("expected ':'"));
        assert!(reject("extended-registers:1").contains("expected ':'"));
        assert!(reject("6x").contains("4 or 5 digits"));
        assert!(reject("6x123").contains("4 or 5 digits"));
    }

    // Upstream ModbusTagDataTypeTest.testExtended_DataType reads INT, UINT, DINT and REAL at
    // extended-register:1 (widths 1, 1, 2, 2); the other types are those of the holding
    // registers.
    // covers: MB-ADDR-EXTENDED
    #[test]
    fn an_extended_register_takes_the_types_a_holding_register_takes() {
        for (name, kind) in [
            ("BOOL", ModbusDataType::Bool),
            ("BYTE", ModbusDataType::Byte),
            ("WORD", ModbusDataType::Word),
            ("DWORD", ModbusDataType::DWord),
            ("LWORD", ModbusDataType::LWord),
            ("SINT", ModbusDataType::SInt),
            ("USINT", ModbusDataType::USInt),
            ("INT", ModbusDataType::Int),
            ("UINT", ModbusDataType::UInt),
            ("DINT", ModbusDataType::DInt),
            ("UDINT", ModbusDataType::UDInt),
            ("LINT", ModbusDataType::LInt),
            ("ULINT", ModbusDataType::ULInt),
            ("REAL", ModbusDataType::Real),
            ("LREAL", ModbusDataType::LReal),
        ] {
            let extended = accept(&format!("extended-register:1:{name}"));
            let holding = accept(&format!("holding-register:1:{name}"));
            assert_eq!(extended.data_type(), kind, "{name}");
            assert_eq!(
                extended.data_type().register_size(),
                holding.data_type().register_size(),
                "{name} width"
            );
            assert_eq!(
                extended.spans_multiple_words(),
                holding.spans_multiple_words(),
                "{name} spans"
            );
        }
        assert_eq!(
            accept("extended-register:1").data_type(),
            ModbusDataType::Int
        );
        assert_eq!(
            accept("extended-register:7:DINT{unit-id: 9, byte-order: 'LITTLE_ENDIAN'}").canonical(),
            "extended-register:7:DINT{unit-id: 9, byte-order: 'LITTLE_ENDIAN'}"
        );
        assert_rejected_naming("extended-register:1:STRING", "STRING");
        // Since the merge with S48 the extended registers take the selection the other areas
        // take (ADR-0022, addendum of 2026-10-29); before it, this was a typed rejection.
        let range = accept("extended-register:1[0..1]:INT");
        assert_eq!(
            range.elements().map(std::num::NonZeroU16::get),
            Some(2),
            "a list of two INTs"
        );
        assert_eq!(range.canonical(), "extended-register:1[0..1]:INT");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn prefix_digits_without_an_area_are_rejected() {
        assert_rejected_naming("2x00001", "no area uses prefix digit 2");
        assert_rejected_naming("5x00001", "no area uses prefix digit 5");
        assert_rejected_naming("7x00001", "no area uses prefix digit 7");
        assert_rejected_naming("9x00001", "no area uses prefix digit 9");
        assert_rejected_naming("2x00001", "6 (extended-register)");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn legacy_array_form_is_rejected_with_hint() {
        let error = reject("holding-register:1:INT[4]");
        assert!(
            error.contains("holding-register:1[0..3]:INT"),
            "message gives the replacement: {error}"
        );
        assert!(
            error.contains("legacy array form"),
            "names the form: {error}"
        );
        assert_rejected_naming("holding-register:1:INT:4", "holding-register:1[0..3]:INT");
        // A count of one or none is the index [0], as upstream writes it (ANP:120-123).
        assert_rejected_naming("4x00001:REAL[1]", "4x00001[0]:REAL");
        assert_rejected_naming("holding-register:1:INT[0]", "holding-register:1[0]:INT");
        assert_rejected_naming("coil:5:BOOL[16]", "coil:5[0..15]:BOOL");
        // Counts the hint cannot compute get the general example, which parses.
        for text in [
            "holding-register:1:INT[x]",
            "holding-register:1:INT[0..3]",
            "holding-register:1:INT[12345678901]",
        ] {
            let message = reject(text);
            assert!(
                message.contains("for example 'holding-register:1[0..3]:INT'"),
                "{text}: {message}"
            );
            assert!(!message.contains("now written"), "{text}: no replacement");
        }
        // After a selection, a count behind the type is not the legacy form.
        assert_rejected_naming("holding-register:1[0..3]:INT[4]", "unexpected text '[4]'");
        assert_rejected_naming("holding-register:1[0]:INT:4", "unexpected text ':4'");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn malformed_shapes_are_rejected() {
        assert_rejected_naming("holding-register", "expected ':'");
        assert_rejected_naming("holding-registers:1", "expected ':'");
        assert_rejected_naming("holding-register:", "missing address number");
        assert_rejected_naming("holding-register:abc", "missing address number");
        assert_rejected_naming("holding-register:-1", "missing address number");
        assert_rejected_naming("holding-register:1:INT:", "unexpected text");
        assert_rejected_naming("holding-register:1:INT:X", "unexpected text");
        assert_rejected_naming("holding-register:1:INT.", "unexpected text");
        assert_rejected_naming("holding-register:1.5", "unexpected text");
        assert_rejected_naming("Holding-Register:1", "unrecognised address");
        assert_rejected_naming("HOLDING-REGISTER:1", "unrecognised address");
        assert_rejected_naming(" holding-register:1", "unrecognised address");
        assert_rejected_naming("holding-register:1 ", "unexpected text");
        assert_rejected_naming("holding", "unrecognised address");
        assert_rejected_naming("x", "unrecognised address");
        assert_rejected_naming(":", "unrecognised address");
        assert_rejected_naming("holding-register:\u{ff11}", "missing address number");
        assert_rejected_naming("\u{ff14}x00001", "unrecognised address");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn empty_input_is_rejected() {
        assert_rejected_naming("", "empty");
        reject_with("", &plc4x());
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn text_length_boundary_is_exact() {
        // Spaces inside the block are allowed, so valid text can be padded to any length.
        let pad = |total: usize| {
            let fixed = "coil:1{}".len();
            format!("coil:1{{{}}}", " ".repeat(total - fixed))
        };
        let short = pad(ModbusAddress::MAX_TEXT_BYTES - 1);
        let exact = pad(ModbusAddress::MAX_TEXT_BYTES);
        let long = pad(ModbusAddress::MAX_TEXT_BYTES + 1);
        assert_eq!(short.len(), 255, "one byte short");
        assert_eq!(exact.len(), 256, "exact");
        assert_eq!(long.len(), 257, "one byte long");
        assert_eq!(accept(&short), accept("coil:1"), "255 bytes parse");
        assert_eq!(accept(&exact), accept("coil:1"), "256 bytes parse");
        assert_rejected_naming(&long, "257 bytes, over the 256 byte limit");
        let huge = "a".repeat(1_000_000);
        assert_rejected_naming(&huge, "over the 256 byte limit");
        let message = reject(&huge);
        assert!(
            message.len() < 200,
            "no echo of the input: {}",
            message.len()
        );
        assert_eq!(ModbusAddress::MAX_TEXT_BYTES, 256, "documented bound");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn control_characters_are_rejected_and_never_echoed() {
        for text in [
            "coil:1\n",
            "coil:1\r\n",
            "co\til:1",
            "coil:1\u{0}",
            "coil:\u{1b}[31m1",
            "coil:1\u{7f}",
            "coil:1\u{85}",
            "\u{202e}coil:1\u{9c}",
        ] {
            let message = reject(text);
            assert!(
                message.contains("control character"),
                "{text:?} names the reason: {message}"
            );
            assert!(
                !message.chars().any(char::is_control),
                "{text:?} message has no control character: {message:?}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn echoed_input_is_bounded_and_control_stripped() {
        let long_key = format!("holding-register:1{{{}: 1}}", "k".repeat(200));
        let message = reject(&long_key);
        assert!(message.contains("..."), "truncation is visible: {message}");
        assert!(message.len() < 300, "bounded echo: {}", message.len());
        let long_area = "z".repeat(200);
        let message = reject(&long_area);
        assert!(message.len() < 400, "bounded echo: {}", message.len());
        assert!(
            !message.contains(&"z".repeat(40)),
            "at most 32 characters echoed"
        );
        let long_name = AddressOptions::builder()
            .convention(&"c".repeat(AddressOptions::MAX_CONVENTION_BYTES))
            .build()
            .expect("64 bytes is the maximum name");
        let message = reject_with("coil:1", &long_name).to_string();
        assert!(
            message.contains("..."),
            "convention echo truncated: {message}"
        );
        // A multibyte key is cut on a character boundary.
        let wide = format!("coil:1{{{}: 1}}", "\u{1f980}".repeat(40));
        let message = reject(&wide);
        assert!(
            message.contains("..."),
            "wide characters truncated: {message}"
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn every_invalid_area_discriminator_is_rejected() {
        for text in [
            "register:1",
            "holding:1",
            "input:1",
            "discrete:1",
            "2x00001",
            "5x00001",
            "7x00001",
            "8x00001",
            "9x00001",
            "Coil:1",
            "coils:1",
        ] {
            reject(text);
        }
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn canonical_renders_the_long_form() {
        for (text, canonical) in [
            ("4x00016:INT", "holding-register:16:INT"),
            ("holding-register:16", "holding-register:16:INT"),
            ("0x0010", "coil:10:BOOL"),
            ("1x00001", "discrete-input:1:BOOL"),
            ("3x00300:REAL", "input-register:300:REAL"),
            (
                "4x00001:REAL{unit-id: 1}",
                "holding-register:1:REAL{unit-id: 1}",
            ),
            (
                "holding-register:1:REAL{ byte-order : \"BIG_ENDIAN_BYTE_SWAP\" }",
                "holding-register:1:REAL{byte-order: 'BIG_ENDIAN_BYTE_SWAP'}",
            ),
            ("coil:000000007", "coil:7:BOOL"),
        ] {
            let address = accept(text);
            assert_eq!(address.canonical(), canonical, "canonical of {text}");
            assert_eq!(accept(canonical), address, "{canonical} parses back");
            assert_eq!(
                accept_plc4x(canonical),
                address,
                "{canonical} parses back under the convention"
            );
        }
        assert_eq!(
            accept_plc4x("40016").canonical(),
            "holding-register:16:INT",
            "the bare form is never rendered"
        );
    }

    fn with(name: &str) -> AddressOptions {
        AddressOptions::builder()
            .convention(name)
            .build()
            .expect("a valid convention name")
    }

    /// The conventions that read numbers from 1, then the one that reads from 0.
    const ONE_BASED: [&str; 2] = ["plc4x", "one-based-40001"];
    const ZERO_BASED: &str = "zero-based";

    /// Every area with its long name and prefix digit.
    const AREAS: [(ModbusArea, &str, u8); 4] = [
        (ModbusArea::Coil, "coil", 0),
        (ModbusArea::DiscreteInput, "discrete-input", 1),
        (ModbusArea::InputRegister, "input-register", 3),
        (ModbusArea::HoldingRegister, "holding-register", 4),
    ];

    /// The numbers at the edges of the digit groups and of the map.
    const BOUNDARIES: [u32; 6] = [0, 1, 9_999, 10_000, 65_535, 65_536];

    /// Every spelling of `number` on an area: long, prefixed short and the two bare
    /// lengths (a 4-digit group only while the number has at most 4 digits).
    fn spellings(name: &str, prefix: u8, number: u32) -> Vec<(&'static str, String)> {
        let mut texts = vec![
            ("long", format!("{name}:{number}")),
            ("prefixed", format!("{prefix}x{number:05}")),
            ("bare 6 digits", format!("{prefix}{number:05}")),
        ];
        if number <= 9_999 {
            texts.push(("prefixed 4 digits", format!("{prefix}x{number:04}")));
            texts.push(("bare 5 digits", format!("{prefix}{number:04}")));
        }
        texts
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn each_convention_reads_every_form_on_every_area_at_the_boundaries() {
        for (area, name, prefix) in AREAS {
            for number in BOUNDARIES {
                for (form, text) in spellings(name, prefix, number) {
                    let bare = form.starts_with("bare");
                    // One-based: the number minus one, 1 to 65536.
                    for convention in ONE_BASED {
                        let result = ModbusAddress::parse(&text, &with(convention));
                        if number == 0 {
                            let message = result.expect_err("0 is not one-based").to_string();
                            assert!(message.contains("logical address 0"), "{text}: {message}");
                        } else {
                            let address = result
                                .unwrap_or_else(|e| panic!("{convention} {form} {text}: {e}"));
                            assert_eq!(address.area(), area, "{convention} {text}");
                            assert_eq!(
                                u32::from(address.offset()),
                                number - 1,
                                "{convention} {text}"
                            );
                        }
                    }
                    // Zero-based: the number itself, 0 to 65535.
                    let result = ModbusAddress::parse(&text, &with(ZERO_BASED));
                    if number <= 65_535 {
                        let address =
                            result.unwrap_or_else(|e| panic!("zero-based {form} {text}: {e}"));
                        assert_eq!(address.area(), area, "zero-based {text}");
                        assert_eq!(u32::from(address.offset()), number, "zero-based {text}");
                    } else {
                        let message = result.expect_err("65536 is past the map").to_string();
                        assert!(
                            message.contains("zero-based address 65536 is above 65535"),
                            "{text}: {message}"
                        );
                    }
                    // No convention: the PLC4X reading of the unambiguous forms, and the
                    // bare forms refused whatever their number.
                    let plain = ModbusAddress::parse(&text, &none());
                    if bare {
                        assert!(plain.is_err(), "{text} needs a convention");
                    } else if (1..=65_536).contains(&number) {
                        assert_eq!(
                            u32::from(plain.expect("unambiguous form").offset()),
                            number - 1,
                            "no convention, {text}"
                        );
                    } else {
                        assert!(plain.is_err(), "no convention, {text}");
                    }
                }
            }
        }
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn the_same_text_is_one_register_higher_under_zero_based() {
        for text in ["holding-register:16", "4x00016", "40016"] {
            let one = ModbusAddress::parse(text, &with("one-based-40001")).expect("one-based");
            let zero = ModbusAddress::parse(text, &with("zero-based")).expect("zero-based");
            assert_eq!(one.offset(), 15, "{text} one-based");
            assert_eq!(zero.offset(), 16, "{text} zero-based");
        }
        // The first register has the number 1 or 0 by convention, and the same wire offset.
        let first_one = ModbusAddress::parse("40001", &with("one-based-40001")).expect("first");
        let first_zero = ModbusAddress::parse("40000", &with("zero-based")).expect("first");
        assert_eq!(first_one, first_zero, "one register, two spellings");
        assert_eq!(first_zero.offset(), 0);
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn the_two_one_based_names_read_every_spelling_alike() {
        for text in [
            "holding-register:16:REAL{unit-id: 2}",
            "4x00016:INT",
            "40016:UINT{byte-order: 'LITTLE_ENDIAN'}",
            "00001",
            "165536",
            "30001:LREAL",
        ] {
            assert_eq!(
                ModbusAddress::parse(text, &with("plc4x")).ok(),
                ModbusAddress::parse(text, &with("one-based-40001")).ok(),
                "{text}"
            );
        }
        assert_eq!(accept_plc4x("40016"), accept("4x00016"), "as before");
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn an_address_is_the_wire_target_whatever_the_convention_that_read_it() {
        let zero = ModbusAddress::parse("holding-register:0:REAL{unit-id: 3}", &with("zero-based"))
            .expect("zero-based");
        let one = accept("holding-register:1:REAL{unit-id: 3}");
        assert_eq!(zero, one, "equal wire targets compare equal");
        assert_eq!(
            zero.convention(),
            AddressConvention::Plc4xCompatible,
            "an address records no reading"
        );
        assert_eq!(
            zero.canonical(),
            "holding-register:1:REAL{unit-id: 3}",
            "canonical text is one-based"
        );
        assert_eq!(accept(&zero.canonical()), zero, "and reads back with none");
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn zero_based_spans_end_at_wire_offset_65535() {
        let options = with("zero-based");
        let last = ModbusAddress::parse("holding-register:65532:LREAL", &options).expect("fits");
        assert_eq!(last.offset(), 65_532);
        let edge = ModbusAddress::parse("holding-register:65535", &options).expect("one INT");
        assert_eq!(edge.offset(), 65_535);
        let past = reject_with("holding-register:65533:LREAL", &options).to_string();
        assert!(past.contains("passes the last address"), "{past}");
        // Four digits are enough for a bare number only up to 9999; five give the rest.
        assert_eq!(
            ModbusAddress::parse("465535:INT", &options)
                .expect("top")
                .offset(),
            65_535
        );
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn digit_counts_and_prefixes_are_checked_under_every_convention() {
        for convention in ["plc4x", "one-based-40001", "zero-based"] {
            let options = with(convention);
            for text in [
                "4001",
                "4000001",
                "20001",
                "4x001",
                "4x000001",
                "2x00001",
                "6001",
                "6000001",
                "6x001",
                "6x000001",
                "holding-register:",
                "holding-register:1234567890",
                "holding-register:0x10",
            ] {
                reject_with(text, &options);
            }
        }
    }

    // covers: MB-ADDR-CONVENTIONS
    #[test]
    fn the_names_are_exact_and_round_trip() {
        for convention in [
            AddressConvention::Plc4xCompatible,
            AddressConvention::ZeroBased,
            AddressConvention::OneBased40001,
        ] {
            assert_eq!(
                AddressConvention::from_name(convention.name()),
                Some(convention)
            );
        }
        assert_eq!(AddressConvention::ZeroBased.name(), "zero-based");
        assert_eq!(AddressConvention::OneBased40001.name(), "one-based-40001");
        for name in [
            "",
            "ZeroBased",
            "zero_based",
            "Zero-Based",
            " zero-based",
            "one-based",
        ] {
            assert_eq!(AddressConvention::from_name(name), None, "{name:?}");
        }
        let message = reject_with("40001", &with("one-based")).to_string();
        assert!(
            message.contains("plc4x, one-based-40001 and zero-based"),
            "the refusal lists the names: {message}"
        );
    }
}
