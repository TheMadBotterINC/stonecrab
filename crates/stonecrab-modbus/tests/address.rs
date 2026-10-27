//! Address type tables: unit id, area, data type, builder checks and canonical text.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::HashSet;
    use std::sync::Arc;

    use stonecrab_core::{
        ByteOrder, ErrorKind, PlcError, PlcValueType, ProtocolAddress, WordOrder,
    };
    use stonecrab_modbus::{AddressConvention, ModbusAddress, ModbusArea, ModbusDataType, UnitId};

    const ALL_AREAS: [ModbusArea; 4] = [
        ModbusArea::Coil,
        ModbusArea::DiscreteInput,
        ModbusArea::HoldingRegister,
        ModbusArea::InputRegister,
    ];

    /// Variant, upstream name, registers, core type: one row per accepted type.
    const TYPE_TABLE: [(ModbusDataType, &str, u16, PlcValueType); 15] = [
        (ModbusDataType::Bool, "BOOL", 1, PlcValueType::Bool),
        (ModbusDataType::Byte, "BYTE", 1, PlcValueType::U8),
        (ModbusDataType::Word, "WORD", 1, PlcValueType::U16),
        (ModbusDataType::DWord, "DWORD", 2, PlcValueType::U32),
        (ModbusDataType::LWord, "LWORD", 4, PlcValueType::U64),
        (ModbusDataType::SInt, "SINT", 1, PlcValueType::I8),
        (ModbusDataType::USInt, "USINT", 1, PlcValueType::U8),
        (ModbusDataType::Int, "INT", 1, PlcValueType::I16),
        (ModbusDataType::UInt, "UINT", 1, PlcValueType::U16),
        (ModbusDataType::DInt, "DINT", 2, PlcValueType::I32),
        (ModbusDataType::UDInt, "UDINT", 2, PlcValueType::U32),
        (ModbusDataType::LInt, "LINT", 4, PlcValueType::I64),
        (ModbusDataType::ULInt, "ULINT", 4, PlcValueType::U64),
        (ModbusDataType::Real, "REAL", 2, PlcValueType::F32),
        (ModbusDataType::LReal, "LREAL", 4, PlcValueType::F64),
    ];

    const DEFERRED_STRING_NAMES: [&str; 4] = ["STRING", "WSTRING", "CHAR", "WCHAR"];

    const DEFERRED_CALENDAR_NAMES: [&str; 8] = [
        "TIME",
        "LTIME",
        "DATE",
        "LDATE",
        "TIME_OF_DAY",
        "LTIME_OF_DAY",
        "DATE_AND_TIME",
        "LDATE_AND_TIME",
    ];

    fn build(
        area: ModbusArea,
        offset: u16,
        data_type: ModbusDataType,
    ) -> Result<ModbusAddress, PlcError> {
        ModbusAddress::builder(area, offset)
            .data_type(data_type)
            .build()
    }

    fn rejection(result: Result<ModbusAddress, PlcError>) -> PlcError {
        match result {
            Ok(address) => panic!("expected a rejection, got {address:?}"),
            Err(error) => error,
        }
    }

    // covers: MB-CFG-UNIT-ID
    #[test]
    fn unit_id_converts_both_ways_and_defaults_to_one() {
        assert_eq!(
            UnitId::default().get(),
            1,
            "upstream default-unit-identifier"
        );
        assert_eq!(
            UnitId::DEFAULT,
            UnitId::default(),
            "constant equals default"
        );
        for value in [0_u8, 1, 10, 254, 255] {
            let unit = UnitId::from(value);
            assert_eq!(unit.get(), value, "get returns the wrapped byte");
            assert_eq!(u8::from(unit), value, "into u8 returns the wrapped byte");
            assert_eq!(unit.to_string(), value.to_string(), "display is decimal");
        }
    }

    // covers: MB-CFG-UNIT-ID
    #[test]
    fn unit_id_zero_and_255_are_documented_not_rejected() {
        assert_eq!(UnitId::BROADCAST.get(), 0, "broadcast constant");
        assert_eq!(UnitId::TCP_NO_UNIT.get(), 255, "no-unit constant");
        for unit in [UnitId::BROADCAST, UnitId::TCP_NO_UNIT] {
            let address = ModbusAddress::builder(ModbusArea::Coil, 0)
                .unit_id(unit)
                .build()
                .expect("0 and 255 are accepted");
            assert_eq!(address.unit_id(), Some(unit), "carried unchanged");
        }
        assert!(UnitId::BROADCAST < UnitId::DEFAULT, "ordered by byte value");
    }

    // covers: MB-ADDR-LONG, MB-ADDR-SHORT-4X
    #[test]
    fn area_table_covers_every_method() {
        // area, is_bit_area, is_writable, prefix digit, long name, default type
        let table = [
            (
                ModbusArea::Coil,
                true,
                true,
                0_u8,
                "coil",
                ModbusDataType::Bool,
            ),
            (
                ModbusArea::DiscreteInput,
                true,
                false,
                1,
                "discrete-input",
                ModbusDataType::Bool,
            ),
            (
                ModbusArea::HoldingRegister,
                false,
                true,
                4,
                "holding-register",
                ModbusDataType::Int,
            ),
            (
                ModbusArea::InputRegister,
                false,
                false,
                3,
                "input-register",
                ModbusDataType::Int,
            ),
        ];
        for (area, bit, writable, digit, name, default_type) in table {
            assert_eq!(area.is_bit_area(), bit, "{name}: is_bit_area");
            assert_eq!(area.is_writable(), writable, "{name}: is_writable");
            assert_eq!(area.prefix_digit(), digit, "{name}: prefix digit");
            assert_eq!(area.long_name(), name, "{name}: long name");
            assert_eq!(
                area.default_data_type(),
                default_type,
                "{name}: default type"
            );
        }
        let digits: HashSet<u8> = ALL_AREAS.iter().map(|a| a.prefix_digit()).collect();
        assert_eq!(
            digits.len(),
            4,
            "prefix digits are distinct (there is no 2x area)"
        );
    }

    // covers: MB-TYPE-BOOL
    #[test]
    fn data_type_table_covers_all_fifteen_types() {
        let names: HashSet<&str> = TYPE_TABLE.iter().map(|row| row.1).collect();
        assert_eq!(names.len(), 15, "fifteen distinct names");
        for (kind, name, size, value_type) in TYPE_TABLE {
            assert_eq!(kind.upstream_name(), name, "{name}: upstream name");
            assert_eq!(kind.register_size(), size, "{name}: register size");
            assert_eq!(kind.value_type(), value_type, "{name}: core type");
            assert_eq!(
                ModbusDataType::from_upstream_name(name).expect("accepted name"),
                kind,
                "{name}: round trip"
            );
        }
    }

    // covers: MB-ADDR-STRING, MB-TYPE-CHAR, MB-TYPE-WCHAR, MB-TYPE-STRING, MB-TYPE-WSTRING
    #[test]
    fn string_and_character_names_are_recognised_and_refused_as_deferred() {
        for name in DEFERRED_STRING_NAMES {
            let error = rejection_of_name(name);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{name}: kind");
            let text = error.to_string();
            assert!(
                text.contains(name),
                "{name}: message names the type: {text}"
            );
            assert!(
                text.contains("deferred"),
                "{name}: message names the deferral: {text}"
            );
            assert!(
                text.contains("MB-ADDR-STRING"),
                "{name}: message cites the row: {text}"
            );
        }
    }

    // covers: MB-TYPE-TIME, MB-TYPE-LTIME, MB-TYPE-DATE, MB-TYPE-LDATE
    #[test]
    fn calendar_names_are_recognised_and_refused_as_unsupported() {
        for name in DEFERRED_CALENDAR_NAMES {
            let error = rejection_of_name(name);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{name}: kind");
            let text = error.to_string();
            assert!(
                text.contains(name),
                "{name}: message names the type: {text}"
            );
            assert!(
                text.contains("calendar"),
                "{name}: message says why: {text}"
            );
            assert!(
                text.contains("MB-TYPE-TIME"),
                "{name}: message cites the row: {text}"
            );
        }
    }

    #[test]
    fn the_twenty_seven_upstream_names_split_fifteen_and_twelve() {
        let accepted = TYPE_TABLE.len();
        let deferred = DEFERRED_STRING_NAMES.len() + DEFERRED_CALENDAR_NAMES.len();
        assert_eq!(accepted + deferred, 27, "upstream defines 27 names");
        assert_eq!(accepted, 15, "fifteen accepted");
        assert_eq!(deferred, 12, "twelve deferred");
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn unknown_and_miscased_names_are_refused() {
        for name in [
            "int", "i16", "Int", "", " INT", "INT ", "FLOAT", "BOOL[4]", "INT\n",
        ] {
            let error = rejection_of_name(name);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{name:?}: kind");
            assert!(
                error.to_string().contains("unknown Modbus data type"),
                "{name:?}: message says unknown"
            );
        }
    }

    #[test]
    fn unknown_name_echo_is_bounded_and_free_of_control_characters() {
        let hostile = format!("\u{1b}[31m{}\n", "X".repeat(10_000));
        let text = rejection_of_name(&hostile).to_string();
        assert!(!text.contains('\u{1b}'), "no escape character echoed");
        assert!(!text.contains('\n'), "no newline echoed");
        assert!(
            text.len() < 200,
            "echo is clipped, message is {} bytes",
            text.len()
        );
    }

    fn rejection_of_name(name: &str) -> PlcError {
        match ModbusDataType::from_upstream_name(name) {
            Ok(kind) => panic!("{name:?} should be refused, got {kind:?}"),
            Err(error) => error,
        }
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn builder_defaults_follow_the_area() {
        for area in ALL_AREAS {
            let address = ModbusAddress::builder(area, 0)
                .build()
                .expect("default is valid");
            assert_eq!(
                address.data_type(),
                area.default_data_type(),
                "{area:?}: type"
            );
            assert_eq!(address.unit_id(), None, "{area:?}: no unit id");
            assert_eq!(address.byte_order(), None, "{area:?}: no byte order");
            assert_eq!(
                address.convention(),
                AddressConvention::Plc4xCompatible,
                "{area:?}: convention"
            );
            assert_eq!(address.area(), area, "{area:?}: area");
        }
    }

    // covers: MB-ADDR-LONG, MB-LIMIT-ADDR
    #[test]
    fn offset_is_the_wire_address_and_logical_is_one_more() {
        let address = build(ModbusArea::HoldingRegister, 15, ModbusDataType::Int).expect("valid");
        assert_eq!(address.offset(), 15, "wire offset");
        assert_eq!(address.logical_address(), 16, "logical address");
        let first = build(ModbusArea::Coil, 0, ModbusDataType::Bool).expect("valid");
        assert_eq!(first.logical_address(), 1, "logical address starts at one");
        let last = build(ModbusArea::DiscreteInput, 65_535, ModbusDataType::Bool).expect("valid");
        assert_eq!(last.logical_address(), 65_536, "highest logical address");
    }

    // covers: MB-ADDR-BIT-TYPES
    #[test]
    fn bit_areas_accept_bool_only() {
        for area in [ModbusArea::Coil, ModbusArea::DiscreteInput] {
            for (kind, name, _, _) in TYPE_TABLE {
                let result = build(area, 0, kind);
                if kind == ModbusDataType::Bool {
                    assert!(result.is_ok(), "{name} on {area:?} must be accepted");
                } else {
                    let error = rejection(result);
                    assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{name}: kind");
                    let text = error.to_string();
                    assert!(text.contains(name), "{name}: names the type: {text}");
                    assert!(
                        text.contains(area.long_name()),
                        "{name}: names the area: {text}"
                    );
                    assert!(
                        text.contains("MB-ADDR-BIT-TYPES"),
                        "{name}: cites the row: {text}"
                    );
                }
            }
        }
    }

    #[test]
    fn int_on_a_coil_is_rejected() {
        let error = rejection(build(ModbusArea::Coil, 3, ModbusDataType::Int));
        assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind");
        assert_eq!(
            error.to_string(),
            "data type INT is not valid on a coil: bit areas read BOOL only \
             (docs/parity/modbus.md#MB-ADDR-BIT-TYPES)",
            "exact message"
        );
    }

    #[test]
    fn register_areas_accept_all_fifteen_types() {
        for area in [ModbusArea::HoldingRegister, ModbusArea::InputRegister] {
            for (kind, name, _, _) in TYPE_TABLE {
                let address =
                    build(area, 100, kind).unwrap_or_else(|e| panic!("{name} on {area:?}: {e}"));
                assert_eq!(address.data_type(), kind, "{name}: type kept");
            }
        }
    }

    // covers: MB-LIMIT-SPAN
    #[test]
    fn span_check_table_at_the_top_of_the_map() {
        // area, offset, type, accepted
        let table = [
            (
                ModbusArea::DiscreteInput,
                65_535_u16,
                ModbusDataType::Bool,
                true,
            ),
            (ModbusArea::Coil, 65_535, ModbusDataType::Bool, true),
            (
                ModbusArea::HoldingRegister,
                65_535,
                ModbusDataType::Int,
                true,
            ),
            (
                ModbusArea::HoldingRegister,
                65_535,
                ModbusDataType::Real,
                false,
            ),
            (
                ModbusArea::HoldingRegister,
                65_534,
                ModbusDataType::Real,
                true,
            ),
            (
                ModbusArea::HoldingRegister,
                65_534,
                ModbusDataType::DInt,
                true,
            ),
            (
                ModbusArea::InputRegister,
                65_534,
                ModbusDataType::LReal,
                false,
            ),
            (
                ModbusArea::InputRegister,
                65_533,
                ModbusDataType::LReal,
                false,
            ),
            (
                ModbusArea::InputRegister,
                65_532,
                ModbusDataType::LReal,
                true,
            ),
            (
                ModbusArea::HoldingRegister,
                65_533,
                ModbusDataType::LWord,
                false,
            ),
            (
                ModbusArea::HoldingRegister,
                65_532,
                ModbusDataType::ULInt,
                true,
            ),
        ];
        for (area, offset, kind, accepted) in table {
            let label = format!("{} at {offset}", kind.upstream_name());
            let result = build(area, offset, kind);
            if accepted {
                assert!(result.is_ok(), "{label} on {area:?} must be accepted");
            } else {
                let error = rejection(result);
                assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{label}: kind");
                let text = error.to_string();
                assert!(
                    text.contains("MB-LIMIT-SPAN"),
                    "{label}: cites the row: {text}"
                );
                assert!(
                    text.contains(&offset.to_string()),
                    "{label}: names the offset"
                );
            }
        }
    }

    #[test]
    fn real_at_the_last_wire_address_is_rejected_with_an_exact_message() {
        let error = rejection(build(
            ModbusArea::HoldingRegister,
            65_535,
            ModbusDataType::Real,
        ));
        assert_eq!(
            error.to_string(),
            "REAL at wire offset 65535 spans 2 register(s) and passes the last address \
             65535 (docs/parity/modbus.md#MB-LIMIT-SPAN)",
            "exact message"
        );
    }

    #[test]
    fn setters_are_order_independent_and_the_last_call_wins() {
        let first = ModbusAddress::builder(ModbusArea::HoldingRegister, 8)
            .data_type(ModbusDataType::Int)
            .unit_id(UnitId::from(2))
            .byte_order(ByteOrder::BigEndian, WordOrder::HighWordFirst)
            .convention(AddressConvention::Plc4xCompatible)
            .data_type(ModbusDataType::Real)
            .unit_id(UnitId::from(3))
            .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
            .build()
            .expect("valid");
        let second = ModbusAddress::builder(ModbusArea::HoldingRegister, 8)
            .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
            .unit_id(UnitId::from(3))
            .data_type(ModbusDataType::Real)
            .build()
            .expect("valid");
        assert_eq!(first, second, "same final settings, same address");
        assert_eq!(first.data_type(), ModbusDataType::Real, "last type");
        assert_eq!(first.unit_id(), Some(UnitId::from(3)), "last unit");
        assert_eq!(
            first.byte_order(),
            Some((ByteOrder::LittleEndian, WordOrder::LowWordFirst)),
            "last order"
        );
    }

    // covers: MB-QUIRK-L
    #[test]
    fn addresses_differing_only_in_byte_order_are_not_equal() {
        let plain = build(ModbusArea::HoldingRegister, 0, ModbusDataType::Real).expect("valid");
        let little = ModbusAddress::builder(ModbusArea::HoldingRegister, 0)
            .data_type(ModbusDataType::Real)
            .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
            .build()
            .expect("valid");
        assert_ne!(plain, little, "byte order participates in equality");
        let set: HashSet<ModbusAddress> = [plain.clone(), little, plain].into_iter().collect();
        assert_eq!(set.len(), 2, "hash agrees with equality");
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn canonical_renders_long_form_without_a_block() {
        let table = [
            (ModbusArea::Coil, 0_u16, ModbusDataType::Bool, "coil:1:BOOL"),
            (
                ModbusArea::DiscreteInput,
                9,
                ModbusDataType::Bool,
                "discrete-input:10:BOOL",
            ),
            (
                ModbusArea::HoldingRegister,
                15,
                ModbusDataType::Int,
                "holding-register:16:INT",
            ),
            (
                ModbusArea::InputRegister,
                65_534,
                ModbusDataType::UInt,
                "input-register:65535:UINT",
            ),
            (
                ModbusArea::HoldingRegister,
                65_535,
                ModbusDataType::Word,
                "holding-register:65536:WORD",
            ),
        ];
        for (area, offset, kind, text) in table {
            let address = build(area, offset, kind).expect("valid");
            assert_eq!(address.canonical(), text, "canonical text");
        }
    }

    // covers: MB-ADDR-CONFIG-BLOCK, MB-BYTEORDER-PRECEDENCE
    #[test]
    fn canonical_appends_the_block_only_for_fields_that_are_set() {
        let unit_only = ModbusAddress::builder(ModbusArea::HoldingRegister, 7)
            .data_type(ModbusDataType::SInt)
            .unit_id(UnitId::from(10))
            .build()
            .expect("valid");
        assert_eq!(
            unit_only.canonical(),
            "holding-register:8:SINT{unit-id: 10}",
            "unit only"
        );

        let order_only = ModbusAddress::builder(ModbusArea::HoldingRegister, 0)
            .byte_order(ByteOrder::BigEndian, WordOrder::HighWordFirst)
            .build()
            .expect("valid");
        assert_eq!(
            order_only.canonical(),
            "holding-register:1:INT{byte-order: 'BIG_ENDIAN'}",
            "byte order only"
        );

        let both = ModbusAddress::builder(ModbusArea::InputRegister, 299)
            .data_type(ModbusDataType::Real)
            .unit_id(UnitId::BROADCAST)
            .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
            .build()
            .expect("valid");
        assert_eq!(
            both.canonical(),
            "input-register:300:REAL{unit-id: 0, byte-order: 'LITTLE_ENDIAN'}",
            "both fields"
        );
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
    #[test]
    fn canonical_names_all_four_upstream_byte_orders() {
        let table = [
            (ByteOrder::BigEndian, WordOrder::HighWordFirst, "BIG_ENDIAN"),
            (
                ByteOrder::LittleEndian,
                WordOrder::LowWordFirst,
                "LITTLE_ENDIAN",
            ),
            (
                ByteOrder::LittleEndian,
                WordOrder::HighWordFirst,
                "BIG_ENDIAN_BYTE_SWAP",
            ),
            (
                ByteOrder::BigEndian,
                WordOrder::LowWordFirst,
                "LITTLE_ENDIAN_BYTE_SWAP",
            ),
        ];
        for (byte, word, name) in table {
            let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 0)
                .data_type(ModbusDataType::DWord)
                .byte_order(byte, word)
                .build()
                .expect("valid");
            assert_eq!(
                address.canonical(),
                format!("holding-register:1:DWORD{{byte-order: '{name}'}}"),
                "{name}: canonical"
            );
            assert_eq!(address.byte_order(), Some((byte, word)), "{name}: accessor");
        }
    }

    // covers: MB-ADDR-LONG
    #[test]
    fn protocol_address_downcasts_through_a_shared_trait_object() {
        let address = ModbusAddress::builder(ModbusArea::InputRegister, 4)
            .data_type(ModbusDataType::Real)
            .unit_id(UnitId::from(5))
            .build()
            .expect("valid");
        let shared: Arc<dyn ProtocolAddress> = Arc::new(address.clone());
        assert_eq!(shared.protocol(), "modbus", "protocol identifier");
        assert_eq!(
            shared.canonical(),
            "input-register:5:REAL{unit-id: 5}",
            "canonical through the trait object"
        );
        assert_eq!(
            shared.value_type_hint(),
            Some(PlcValueType::F32),
            "hint is the data type's core type"
        );
        let any: &dyn Any = &*shared;
        let back = any
            .downcast_ref::<ModbusAddress>()
            .expect("a driver recovers its own address type");
        assert_eq!(back, &address, "downcast value equals the original");
    }

    #[test]
    fn value_type_hint_is_always_present_and_matches_the_table() {
        for (kind, name, _, value_type) in TYPE_TABLE {
            let address = build(ModbusArea::HoldingRegister, 0, kind).expect("valid");
            assert_eq!(address.value_type_hint(), Some(value_type), "{name}: hint");
        }
    }

    #[test]
    fn debug_text_shows_every_field_and_no_secret() {
        let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 15)
            .data_type(ModbusDataType::Int)
            .unit_id(UnitId::from(10))
            .byte_order(ByteOrder::LittleEndian, WordOrder::LowWordFirst)
            .build()
            .expect("valid");
        assert_eq!(
            format!("{address:?}"),
            "ModbusAddress { unit_id: Some(UnitId(10)), area: HoldingRegister, offset: 15, \
             data_type: Int, byte_order: Some((LittleEndian, LowWordFirst)), \
             convention: Plc4xCompatible }",
            "derived Debug text"
        );
        let bare = ModbusAddress::builder(ModbusArea::Coil, 0)
            .build()
            .expect("valid");
        assert!(
            format!("{bare:?}").contains("unit_id: None, area: Coil, offset: 0"),
            "absent unit id prints as None"
        );
    }

    #[test]
    fn convention_defaults_to_plc4x_compatible() {
        assert_eq!(
            AddressConvention::default(),
            AddressConvention::Plc4xCompatible,
            "only variant is the default"
        );
    }
}
