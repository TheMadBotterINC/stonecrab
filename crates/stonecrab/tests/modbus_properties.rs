//! Property tests of the address grammar (volume set by `PROPTEST_CASES`).
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType, UnitId};
    use stonecrab::{AddressOptions, ByteOrder, ErrorKind, ProtocolAddress, WordOrder};

    const AREAS: [ModbusArea; 4] = [
        ModbusArea::Coil,
        ModbusArea::DiscreteInput,
        ModbusArea::HoldingRegister,
        ModbusArea::InputRegister,
    ];

    const TYPES: [ModbusDataType; 15] = [
        ModbusDataType::Bool,
        ModbusDataType::Byte,
        ModbusDataType::Word,
        ModbusDataType::DWord,
        ModbusDataType::LWord,
        ModbusDataType::SInt,
        ModbusDataType::USInt,
        ModbusDataType::Int,
        ModbusDataType::UInt,
        ModbusDataType::DInt,
        ModbusDataType::UDInt,
        ModbusDataType::LInt,
        ModbusDataType::ULInt,
        ModbusDataType::Real,
        ModbusDataType::LReal,
    ];

    fn named(convention: &str) -> AddressOptions {
        AddressOptions::builder()
            .convention(convention)
            .build()
            .expect("a valid convention name")
    }

    fn plc4x() -> AddressOptions {
        named("plc4x")
    }

    /// No convention, then each of the three.
    fn every_option() -> [AddressOptions; 4] {
        [
            AddressOptions::default(),
            plc4x(),
            named("one-based-40001"),
            named("zero-based"),
        ]
    }

    /// The options under which the one-based canonical text reads back unchanged.
    fn one_based_options() -> [AddressOptions; 3] {
        [AddressOptions::default(), plc4x(), named("one-based-40001")]
    }

    fn orders() -> impl Strategy<Value = Option<(ByteOrder, WordOrder)>> {
        prop_oneof![
            Just(None),
            Just(Some((ByteOrder::BigEndian, WordOrder::HighWordFirst))),
            Just(Some((ByteOrder::LittleEndian, WordOrder::LowWordFirst))),
            Just(Some((ByteOrder::LittleEndian, WordOrder::HighWordFirst))),
            Just(Some((ByteOrder::BigEndian, WordOrder::LowWordFirst))),
        ]
    }

    /// Text built from the characters the grammar cares about, so the fuzz-like input
    /// reaches deep into the scanner.
    fn grammar_soup() -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop_oneof![
                Just("holding-register".to_owned()),
                Just("coil".to_owned()),
                Just("extended-register".to_owned()),
                Just("unit-id".to_owned()),
                Just("byte-order".to_owned()),
                Just("LITTLE_ENDIAN".to_owned()),
                Just("INT".to_owned()),
                Just("0x".to_owned()),
                Just("4x".to_owned()),
                "[0-9]{1,6}",
                "[:{}\\[\\]()',.\" \\-_xX]{1,3}",
                any::<char>().prop_map(String::from),
            ],
            0..12,
        )
        .prop_map(|parts| parts.concat())
    }

    proptest! {
        // covers: SPEC mandatory-failure-cases / F3 invalid register
        #[test]
        fn prop_parse_never_panics(
            arbitrary in any::<String>(),
            soup in grammar_soup(),
            long in "\\PC{0,400}",
        ) {
            for text in [&arbitrary, &soup, &long] {
                for options in every_option() {
                    if let Err(error) = ModbusAddress::parse(text, &options) {
                        prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
                        prop_assert!(!error.to_string().chars().any(char::is_control));
                    }
                }
            }
        }

        // covers: MB-ADDR-LONG
        #[test]
        fn prop_canonical_round_trips(
            area in proptest::sample::select(AREAS.to_vec()),
            offset in prop_oneof![any::<u16>(), 65_500_u16..=65_535],
            listed_type in proptest::sample::select(TYPES.to_vec()),
            unit in proptest::option::of(any::<u8>()),
            order in orders(),
        ) {
            let data_type = if area.is_bit_area() {
                ModbusDataType::Bool
            } else {
                listed_type
            };
            let mut builder = ModbusAddress::builder(area, offset).data_type(data_type);
            if let Some(unit) = unit {
                builder = builder.unit_id(UnitId::from(unit));
            }
            if let Some((byte, word)) = order {
                builder = builder.byte_order(byte, word);
            }
            // A register type that runs past the end of the map is not an address.
            let Ok(address) = builder.build() else {
                return Ok(());
            };
            let canonical = address.canonical();
            for options in one_based_options() {
                let parsed = ModbusAddress::parse(&canonical, &options);
                prop_assert_eq!(parsed.as_ref().ok(), Some(&address), "{}", canonical);
            }
        }

        // covers: SPEC mandatory-failure-cases / F3 invalid register
        #[test]
        fn prop_parse_success_is_stable_under_canonicalisation(soup in grammar_soup()) {
            // The canonical text is one-based whatever convention read the soup.
            for options in every_option() {
                if let Ok(address) = ModbusAddress::parse(&soup, &options) {
                    for again in one_based_options() {
                        let reread = ModbusAddress::parse(&address.canonical(), &again);
                        prop_assert_eq!(reread.as_ref().ok(), Some(&address));
                    }
                }
            }
        }

        // covers: MB-ADDR-SHORT-4X
        #[test]
        fn prop_short_and_long_forms_agree(
            area in proptest::sample::select(AREAS.to_vec()),
            logical in 1_u32..=65_536,
        ) {
            let long = format!("{}:{}", area.long_name(), logical);
            let short = format!("{}x{:05}", area.prefix_digit(), logical);
            let bare = format!("{}{:05}", area.prefix_digit(), logical);
            let long = ModbusAddress::parse(&long, &AddressOptions::default()).ok();
            prop_assert!(long.is_some());
            prop_assert_eq!(&long, &ModbusAddress::parse(&short, &AddressOptions::default()).ok());
            prop_assert_eq!(&long, &ModbusAddress::parse(&bare, &plc4x()).ok());
            prop_assert!(ModbusAddress::parse(&bare, &AddressOptions::default()).is_err());
            prop_assert_eq!(&long, &ModbusAddress::parse(&bare, &named("one-based-40001")).ok());
        }

        // covers: MB-ADDR-CONVENTIONS
        #[test]
        fn prop_zero_based_forms_agree_and_sit_one_below_the_one_based_number(
            area in proptest::sample::select(AREAS.to_vec()),
            number in 0_u32..=65_535,
        ) {
            let zero = named("zero-based");
            let long = format!("{}:{}", area.long_name(), number);
            let short = format!("{}x{:05}", area.prefix_digit(), number);
            let bare = format!("{}{:05}", area.prefix_digit(), number);
            let long = ModbusAddress::parse(&long, &zero).ok();
            prop_assert!(long.is_some());
            prop_assert_eq!(long.as_ref().map(|a| u32::from(a.offset())), Some(number));
            prop_assert_eq!(&long, &ModbusAddress::parse(&short, &zero).ok());
            prop_assert_eq!(&long, &ModbusAddress::parse(&bare, &zero).ok());
            // The same register in the one-based spelling has the number plus one.
            let one_based = format!("{}:{}", area.long_name(), number.saturating_add(1));
            prop_assert_eq!(&long, &ModbusAddress::parse(&one_based, &plc4x()).ok());
        }
    }
}
