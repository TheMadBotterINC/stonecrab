//! The address text grammar: accepted forms, defaults, offsets and every rejection.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab_core::{
        AddressOptions, ByteOrder, ErrorKind, PlcError, ProtocolAddress, WordOrder,
    };
    use stonecrab_modbus::{AddressConvention, ModbusAddress, ModbusArea, ModbusDataType, UnitId};

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
                message.contains("address-convention=plc4x"),
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
        for name in ["PLC4X", "plc4x2", "zero-based", "one-based", "x"] {
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

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn selection_range_is_rejected_at_parse_time() {
        for text in [
            "holding-register:1[0..1]:REAL",
            "holding-register:1[0..7]:SINT{unit-id: 10}",
            "4x00001[0..1]:REAL",
            "40001[0..1]",
            "coil:1[1..3]",
        ] {
            let error = reject_with(text, &plc4x());
            assert!(
                error.to_string().contains("array selections"),
                "{text}: {error}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn selection_index_is_rejected_at_parse_time() {
        for text in [
            "holding-register:1[3]:INT",
            "holding-register:1[3]",
            "coil:1[0]",
            "4x00001[0]",
        ] {
            assert_rejected_naming(text, "array selections");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn string_type_is_rejected_at_parse_time() {
        assert_rejected_naming("holding-register:1:STRING", "string and character types");
        assert_rejected_naming(
            "holding-register:1:STRING(20)",
            "string and character types",
        );
        assert_rejected_naming("holding-register:1[0..2]:STRING(20)", "array selections");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn wstring_type_is_rejected_at_parse_time() {
        assert_rejected_naming("holding-register:1:WSTRING", "WSTRING");
        assert_rejected_naming("input-register:1:WSTRING(8)", "WSTRING");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn char_type_is_rejected_at_parse_time() {
        assert_rejected_naming("holding-register:1:CHAR", "CHAR");
        assert_rejected_naming("coil:1:CHAR", "CHAR");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn wchar_type_is_rejected_at_parse_time() {
        assert_rejected_naming("holding-register:1:WCHAR", "WCHAR");
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn parenthesised_length_is_rejected() {
        assert_rejected_naming("holding-register:1:INT(2)", "parenthesised length");
        assert_rejected_naming("coil:1:BOOL(1)", "parenthesised length");
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

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn extended_register_is_rejected_at_parse_time() {
        for text in [
            "extended-register:1",
            "extended-register:1:INT",
            "extended-register",
            "extended-registers:1",
        ] {
            assert_rejected_naming(text, "extended registers");
        }
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn six_x_prefix_is_rejected_at_parse_time() {
        for text in ["6x00001", "6x", "6x00001:INT"] {
            assert_rejected_naming(text, "extended registers");
        }
        // The bare 6 prefix is the same area, with or without the convention.
        assert_rejected_naming("60001", "extended registers");
        assert!(
            reject_with("60001", &plc4x())
                .to_string()
                .contains("extended registers"),
            "bare 6 with the convention"
        );
        assert_rejected_naming("2x00001", "no area uses prefix digit 2");
        assert_rejected_naming("5x00001", "no area uses prefix digit 5");
        assert_rejected_naming("9x00001", "no area uses prefix digit 9");
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
        assert_rejected_naming("4x00001:REAL[1]", "4x00001[0..0]:REAL");
        assert_rejected_naming("coil:5:BOOL[16]", "coil:5[0..15]:BOOL");
        // Counts the hint cannot compute fall back to a generic spelling.
        assert_rejected_naming("holding-register:1:INT[0]", "[0..N-1]:INT");
        assert_rejected_naming("holding-register:1:INT[x]", "[0..N-1]:INT");
        assert_rejected_naming("holding-register:1:INT[0..3]", "[0..N-1]:INT");
        assert_rejected_naming("holding-register:1:INT[12345678901]", "[0..N-1]:INT");
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
}
