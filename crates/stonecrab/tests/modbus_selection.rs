//! Array selections in the Modbus address grammar (slice S48, ADR-0022): `[index]`,
//! `[lo..hi]` and `;base`, the wire offsets they resolve to, the limits a whole selection
//! must fit, the legacy forms and their hints, and the upstream test cases of the array
//! notation, transcribed from the pinned Apache PLC4X sources (v1.0.0, commit ea229918)
//! with the upstream file named on each case. Everything here is parsing: no I/O.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU16;

    use proptest::prelude::*;
    use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType};
    use stonecrab::{AddressOptions, ErrorKind, PlcValueType, ProtocolAddress};

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

    fn none() -> AddressOptions {
        AddressOptions::default()
    }

    fn named(convention: &str) -> AddressOptions {
        AddressOptions::builder()
            .convention(convention)
            .build()
            .expect("a valid convention name")
    }

    fn accept_with(text: &str, options: &AddressOptions) -> ModbusAddress {
        ModbusAddress::parse(text, options).unwrap_or_else(|e| panic!("{text:?} rejected: {e}"))
    }

    fn accept(text: &str) -> ModbusAddress {
        accept_with(text, &none())
    }

    /// The message of the `InvalidAddress` error for `text`, after checking the kind.
    fn reject(text: &str) -> String {
        match ModbusAddress::parse(text, &none()) {
            Ok(address) => panic!("{text:?} should be rejected, got {address:?}"),
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::InvalidAddress, "kind of {text:?}");
                error.to_string()
            }
        }
    }

    fn assert_rejected_naming(text: &str, needle: &str) {
        let message = reject(text);
        assert!(
            message.contains(needle),
            "message for {text:?} should contain {needle:?}: {message}"
        );
    }

    /// (offset, element count) of an accepted address.
    fn shape(address: &ModbusAddress) -> (u16, Option<u16>) {
        (address.offset(), address.elements().map(NonZeroU16::get))
    }

    /// Bits or registers one element occupies: what a selection steps by.
    fn width(area: ModbusArea, data_type: ModbusDataType) -> u32 {
        if area.is_bit_area() {
            1
        } else {
            u32::from(data_type.register_size())
        }
    }

    /// The type an area accepts for `listed`: `BOOL` on a bit area.
    fn fitting(area: ModbusArea, listed: ModbusDataType) -> ModbusDataType {
        if area.is_bit_area() {
            ModbusDataType::Bool
        } else {
            listed
        }
    }

    /// The replacement address a legacy-form message offers, if it offers one.
    fn hint_of(message: &str) -> Option<&str> {
        message
            .split_once("now written '")
            .and_then(|(_, after)| after.split_once('\''))
            .map(|(hint, _)| hint)
    }

    // covers: MB-ADDR-SELECTION / an index is a scalar, a range a list, even of one
    #[test]
    fn an_index_is_one_element_and_a_range_is_a_list_even_of_one() {
        let index = accept("holding-register:1[4]:INT");
        assert_eq!(shape(&index), (4, None), "the fifth INT, alone");
        assert_eq!(index.value_type_hint(), Some(PlcValueType::I16), "a scalar");
        let one = accept("holding-register:1[4..4]:INT");
        assert_eq!(shape(&one), (4, Some(1)), "a list of one");
        assert_eq!(one.value_type_hint(), Some(PlcValueType::List), "a list");
        assert_ne!(index, one, "same element, different values");
        let eight = accept("holding-register:1[0..7]:INT");
        assert_eq!(shape(&eight), (0, Some(8)), "inclusive bounds");
        assert!(!eight.spans_multiple_words(), "word order is per element");
        let plain = accept("holding-register:1:INT");
        assert_eq!(shape(&plain), (0, None), "no selection");
        assert_eq!(
            accept("holding-register:1[0]:INT"),
            plain,
            "index 0 is the address itself"
        );
        assert_eq!(
            shape(&accept("holding-register:1[0..1]")),
            (0, Some(2)),
            "the type may be omitted"
        );
        assert_eq!(
            shape(&accept("coil:1[0..7]{unit-id: 3}")),
            (0, Some(8)),
            "the block follows the type, or the selection when the type is omitted"
        );
    }

    // covers: MB-ADDR-SELECTION / offsets count elements of the type from the declared base
    #[test]
    fn element_offsets_scale_by_the_element_width_from_the_declared_base() {
        for (text, offset, elements) in [
            ("holding-register:1[4]:DINT", 8, None),
            ("holding-register:1[2]:LINT", 8, None),
            ("input-register:1[4]:DINT", 8, None),
            ("holding-register:1[4..7]:REAL", 8, Some(4)),
            ("holding-register:1[4..7;1]:INT", 3, Some(4)),
            ("holding-register:1[4..7;1]:REAL", 6, Some(4)),
            ("holding-register:10[1;1]:DINT", 9, None),
            ("holding-register:10[3..3;3]:LREAL", 9, Some(1)),
            ("coil:1[4]:BOOL", 4, None),
            ("coil:9[0..15]", 8, Some(16)),
            ("discrete-input:1[4..7;1]", 3, Some(4)),
            ("4x00001[0..1]:REAL", 0, Some(2)),
            ("3x00011[2]:UDINT", 14, None),
            ("0x00001[0..0]", 0, Some(1)),
            ("1x00005[1..2;1]", 4, Some(2)),
        ] {
            assert_eq!(shape(&accept(text)), (offset, elements), "{text}");
        }
        assert_eq!(
            shape(&accept_with("40001[0..1]:REAL", &named("plc4x"))),
            (0, Some(2)),
            "the bare form takes a selection under a convention"
        );
        assert_eq!(
            shape(&accept_with(
                "holding-register:0[1..2]:REAL",
                &named("zero-based")
            )),
            (2, Some(2)),
            "zero-based: the number is the wire offset of element 0"
        );
    }

    // covers: MB-ADDR-SELECTION / the canonical text keeps a range and drops a resolved index
    #[test]
    fn canonical_writes_a_range_after_the_number_and_reads_back() {
        for (text, canonical) in [
            ("4x00001[0..1]:REAL", "holding-register:1[0..1]:REAL"),
            (
                "holding-register:1[4..7;1]:INT{unit-id: 2}",
                "holding-register:4[0..3]:INT{unit-id: 2}",
            ),
            (
                "holding-register:1[4..4]:INT",
                "holding-register:5[0..0]:INT",
            ),
            ("holding-register:1[4]:DINT", "holding-register:9:DINT"),
            ("coil:1[0..1999]", "coil:1[0..1999]:BOOL"),
            (
                "input-register:3[0..2]:WORD{byte-order: 'LITTLE_ENDIAN'}",
                "input-register:3[0..2]:WORD{byte-order: 'LITTLE_ENDIAN'}",
            ),
        ] {
            let address = accept(text);
            assert_eq!(address.canonical(), canonical, "{text}");
            assert_eq!(accept(canonical), address, "{canonical} reads back");
        }
    }

    // covers: MB-ADDR-SELECTION / malformed selections are typed rejections at parse time
    #[test]
    fn malformed_selections_are_invalid_addresses() {
        for text in [
            "holding-register:1[",
            "holding-register:1[0..3",
            "holding-register:1[]",
            "holding-register:1[..3]",
            "holding-register:1[1..]",
            "holding-register:1[a]",
            "holding-register:1[1..x]",
            "holding-register:1[1..2..3]",
            "holding-register:1[1;]",
            "holding-register:1[;1]",
            "holding-register:1[1..2;]",
            "holding-register:1[1;2;3]",
            "holding-register:1[ 1]",
            "holding-register:1[1 ]",
            "holding-register:1[-1]",
            "holding-register:1[+1]",
            "holding-register:1[0x1]",
            "holding-register:1[1.5]",
            "holding-register:1[1...2]",
        ] {
            assert_rejected_naming(text, "invalid selection");
        }
        assert_rejected_naming("holding-register:1[0..1]x", "unexpected text 'x'");
        assert_rejected_naming("holding-register:1[0..1]]:INT", "unexpected text ']:INT'");
        assert_rejected_naming("holding-register:1[0..1] :INT", "unexpected text");
    }

    // covers: MB-ADDR-SELECTION / Modbus areas are linear: one dimension
    #[test]
    fn a_second_dimension_is_refused() {
        for text in [
            "holding-register:1[0..1][2]",
            "holding-register:1[1][2]:INT",
            "holding-register:1[1..2,3..4]:INT",
            "holding-register:1[0,1]",
            "coil:1[0..1][0..1]",
        ] {
            assert_rejected_naming(text, "more than one dimension");
        }
    }

    // covers: MB-ADDR-SELECTION / bounds are checked before any I/O
    #[test]
    fn a_reversed_range_and_an_index_below_the_base_are_refused() {
        assert_rejected_naming(
            "holding-register:1[7..4]:INT",
            "upper bound 4 is below its lower bound 7",
        );
        assert_rejected_naming(
            "holding-register:1[0;1]:INT",
            "index 0 lies below the declared lower bound 1",
        );
        assert_rejected_naming(
            "holding-register:1[2..9;3]:INT",
            "index 2 lies below the declared lower bound 3",
        );
        assert_eq!(
            shape(&accept("holding-register:1[3..9;3]:INT")),
            (0, Some(7)),
            "an index at the base is element 0"
        );
    }

    // covers: MB-LIMIT-DIGITS / selection numbers are bounded by value, not by digit count
    #[test]
    fn a_selection_number_beyond_u32_is_refused_and_leading_zeros_are_not() {
        assert_eq!(
            shape(&accept("holding-register:1[0000000004]:INT")),
            (4, None),
            "ten digits with leading zeros are the number 4"
        );
        for text in [
            "holding-register:1[4294967296]",
            "holding-register:1[0..4294967296]",
            "holding-register:1[1;4294967296]",
            "holding-register:1[99999999999999999999]",
        ] {
            let message = reject(text);
            assert!(message.contains("too large"), "{text}: {message}");
            assert!(
                !message.contains("4294967296") && !message.contains("9999999999"),
                "{text}: the digits are not echoed: {message}"
            );
        }
        assert_rejected_naming(
            "holding-register:1[4294967295]:INT",
            "past the last address",
        );
    }

    // covers: MB-LIMIT-SPAN / the whole selection ends at or before wire address 65535
    #[test]
    fn the_whole_selection_must_end_at_or_before_the_last_wire_address() {
        for (text, offset, elements) in [
            ("holding-register:65535[0..1]:INT", 65_534, Some(2)),
            ("holding-register:65536[0..0]:INT", 65_535, Some(1)),
            ("holding-register:65533[0..1]:REAL", 65_532, Some(2)),
            ("holding-register:1[32767]:DINT", 65_534, None),
            ("coil:65536[0..0]", 65_535, Some(1)),
            ("coil:1[65535]", 65_535, None),
            ("discrete-input:63537[0..1999]", 63_536, Some(2_000)),
        ] {
            assert_eq!(shape(&accept(text)), (offset, elements), "{text}: fits");
        }
        for text in [
            "holding-register:65536[0..1]:INT",
            "holding-register:65534[0..1]:REAL",
            "holding-register:65530[0..1]:LREAL",
            "discrete-input:63538[0..1999]",
            "coil:65536[1..1]",
        ] {
            assert_rejected_naming(text, "MB-LIMIT-SPAN");
        }
        assert_rejected_naming("coil:65536[1]", "starts at wire offset 65536");
        assert_rejected_naming(
            "holding-register:1[32768]:DINT",
            "starts at wire offset 65536",
        );
        assert_rejected_naming("coil:1[0..70000]", "holds 70001 elements");
        assert_rejected_naming(
            "holding-register:65536[0..1]:INT",
            "a list of 2 INT at wire offset 65535 spans 2 register(s) and passes the last address",
        );
    }

    // covers: MB-LIMIT-COILS / a bit selection is one read of at most 2000 bits
    #[test]
    fn a_bit_selection_holds_at_most_2000_bits() {
        for area in ["coil", "discrete-input"] {
            assert_eq!(
                shape(&accept(&format!("{area}:1[0..1999]"))),
                (0, Some(2_000)),
                "{area}: 2000 bits"
            );
            assert_rejected_naming(
                &format!("{area}:1[0..2000]"),
                "a list of 2001 BOOL spans 2001 bits, more than the 2000 one read carries \
                 (docs/parity/modbus.md#MB-LIMIT-COILS)",
            );
        }
    }

    // covers: MB-LIMIT-REGISTERS / a register selection is measured in registers, not elements
    #[test]
    fn a_register_selection_holds_at_most_125_registers() {
        for (text, elements) in [
            ("holding-register:1[0..124]:INT", 125),
            ("input-register:1[0..124]:WORD", 125),
            ("holding-register:1[0..61]:DINT", 62),
            ("input-register:1[0..30]:LREAL", 31),
            ("holding-register:1[0..124]:BOOL", 125),
        ] {
            assert_eq!(shape(&accept(text)).1, Some(elements), "{text}: fits");
        }
        for (text, registers) in [
            ("holding-register:1[0..125]:INT", 126),
            ("holding-register:1[0..62]:DINT", 126),
            ("input-register:1[0..31]:LREAL", 128),
            ("input-register:1[0..125]:SINT", 126),
        ] {
            assert_rejected_naming(
                text,
                &format!(
                    "spans {registers} registers, more than the 125 one read carries \
                     (docs/parity/modbus.md#MB-LIMIT-REGISTERS)"
                ),
            );
        }
    }

    // covers: MB-ADDR-SELECTION / the builder applies the same limits as the parser
    #[test]
    fn the_builder_makes_a_list_under_the_same_limits() {
        let count = |value: u16| NonZeroU16::new(value).expect("non-zero");
        let built = ModbusAddress::builder(ModbusArea::HoldingRegister, 0)
            .data_type(ModbusDataType::Real)
            .elements(count(2))
            .build()
            .expect("two REALs");
        assert_eq!(
            built,
            accept("holding-register:1[0..1]:REAL"),
            "same address"
        );
        for (area, offset, data_type, elements) in [
            (ModbusArea::Coil, 0, ModbusDataType::Bool, 2_001),
            (ModbusArea::InputRegister, 0, ModbusDataType::DInt, 63),
            (ModbusArea::HoldingRegister, 65_535, ModbusDataType::Int, 2),
            (
                ModbusArea::DiscreteInput,
                65_535,
                ModbusDataType::Bool,
                u16::MAX,
            ),
        ] {
            let refused = ModbusAddress::builder(area, offset)
                .data_type(data_type)
                .elements(count(elements))
                .build()
                .map_err(|e| e.kind());
            assert_eq!(
                refused,
                Err(ErrorKind::InvalidAddress),
                "{area:?} {offset} x{elements}"
            );
        }
    }

    // covers: MB-ADDR-LEGACY-ARRAY / every replacement the message offers parses
    #[test]
    fn every_legacy_hint_parses_to_the_selection_it_names() {
        for area in AREAS {
            for listed in TYPES {
                let data_type = fitting(area, listed);
                let name = data_type.upstream_name();
                let element = width(area, data_type);
                let limit: u32 = if area.is_bit_area() { 2_000 } else { 125 };
                let most = limit.div_euclid(element);
                for count in [0, 1, 2, 3, most] {
                    for legacy in [
                        format!("{}:7:{name}[{count}]", area.long_name()),
                        format!("{}:7:{name}:{count}", area.long_name()),
                        format!("{}x00007:{name}[{count}]", area.prefix_digit()),
                    ] {
                        let message = reject(&legacy);
                        let hint = hint_of(&message)
                            .unwrap_or_else(|| panic!("{legacy}: a replacement: {message}"));
                        let parsed = accept(hint);
                        let expected = if count <= 1 {
                            (6, None)
                        } else {
                            (6, u16::try_from(count).ok())
                        };
                        assert_eq!(shape(&parsed), expected, "{legacy} -> {hint}");
                        assert_eq!(parsed.data_type(), data_type, "{legacy} -> {hint}");
                    }
                }
                // One element more than a read carries: the range would be refused, so
                // the message gives the general example instead.
                let wide = format!("{}:7:{name}[{}]", area.long_name(), most + 1);
                let message = reject(&wide);
                assert_eq!(hint_of(&message), None, "{wide}: {message}");
                assert!(message.contains("for example"), "{wide}: {message}");
            }
        }
    }

    // covers: MB-ADDR-LEGACY-ARRAY / no replacement past the end of the map
    #[test]
    fn a_legacy_hint_is_withheld_when_the_range_passes_the_last_address() {
        assert_eq!(
            hint_of(&reject("holding-register:65535:INT[2]")),
            Some("holding-register:65535[0..1]:INT"),
            "logical 65535 and 65536 are wire 65534 and 65535"
        );
        for legacy in [
            "holding-register:65536:INT[2]",
            "holding-register:65536:DINT[1]",
            "coil:65536:BOOL:2",
            "holding-register:1:INT[999999999]",
            "coil:1:INT[4]",
        ] {
            let message = reject(legacy);
            assert_eq!(hint_of(&message), None, "{legacy}: {message}");
            assert!(
                message.contains("MB-ADDR-LEGACY-ARRAY"),
                "{legacy}: {message}"
            );
        }
        // The replacement is checked under the caller's convention.
        let zero_based =
            ModbusAddress::parse("holding-register:65535:INT[2]", &named("zero-based"))
                .map_err(|e| e.to_string());
        let message = zero_based.expect_err("the legacy form");
        assert_eq!(hint_of(&message), None, "{message}");
    }

    // ---- upstream cases, transcribed ----------------------------------------------------

    // covers: MB-ADDR-SELECTION / transcribed from upstream
    // plc4j/spi/drivers/src/test/java/org/apache/plc4x/java/spi/drivers/model/ArrayNotationParserTest.java
    // (singleDimensionResolvesToTheDocumentedOffsets, aBareIndexIsNotTheSameAsAOneElementRange,
    // malformedExpressionsAreRejected, moreDimensionsThanTheProtocolCarriesIsRejected,
    // theCommaSpellingIsTheSameSelection under Modbus's single dimension,
    // aMalformedCommaListIsRejected, refusesARangeThatCannotBeCounted), on a coil, where an
    // element is one address
    #[test]
    fn upstream_array_notation_parser_cases_hold_on_a_modbus_address() {
        for (selection, size, first) in [
            ("[4]", 1, 4),
            ("[0..7]", 8, 0),
            ("[4;1]", 1, 3),
            ("[4..7;1]", 4, 3),
            ("[0]", 1, 0),
            ("[7..7]", 1, 7),
        ] {
            let address = accept(&format!("coil:1{selection}"));
            assert_eq!(address.offset(), first, "{selection}: first offset");
            let elements = address.elements().map_or(1, NonZeroU16::get);
            assert_eq!(elements, size, "{selection}: size");
            assert_eq!(
                address.elements().is_none(),
                !selection.contains(".."),
                "{selection}: a bare index is a scalar, a range is not"
            );
        }
        for selection in [
            "[]", "[7..4]", "[0;1]", "[-1]", "[1..-2]", "[a]", "[1..x]", "[1..2;]", "[1..]",
            "[..2]",
        ] {
            let message = reject(&format!("coil:1{selection}"));
            assert!(
                message.contains(selection.trim_end_matches(']').trim_start_matches('['))
                    || message.contains("below"),
                "{selection}: the message names the selection or the bound: {message}"
            );
        }
        for selection in [
            "[1][2]",
            "[0,1]",
            "[1..2,3..4]",
            "[1..2;1,3..4;1]",
            "[0,1,2]",
            "[1..2,3]",
            "[0,1][2]",
            "[0,]",
            "[,1]",
            "[0,,1]",
            "[0, 1]",
        ] {
            assert_rejected_naming(&format!("coil:1{selection}"), "more than one dimension");
        }
        assert_rejected_naming("coil:1[0..2147483647]", "holds 2147483648 elements");
    }

    // covers: MB-ADDR-SELECTION / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusArrayParityTest.java
    #[test]
    fn upstream_array_parity_cases() {
        let range = accept("holding-register:1[0..7]:INT");
        assert_eq!(shape(&range), (0, Some(8)), "aRangeOfEightStartingAtZero");
        assert_eq!(
            accept("holding-register:1[4]:INT").elements(),
            None,
            "aBareIndexIsAScalar"
        );
        assert_eq!(
            accept("holding-register:1:INT").elements(),
            None,
            "anOmittedSelectionIsAScalar"
        );
    }

    // covers: MB-ADDR-SELECTION / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/base/tag/ModbusSelectionOffsetTest.java
    #[test]
    fn upstream_selection_offset_cases() {
        // oneRegisterPerElement, twoRegistersPerElement, fourRegistersPerElement
        assert_eq!(accept("holding-register:1[4]:INT").offset(), 4);
        assert_eq!(accept("holding-register:1[4]:DINT").offset(), 8);
        assert_eq!(accept("holding-register:1[2]:LINT").offset(), 8);
        // theOtherRegisterAreasScaleToo (its extended-register half is in
        // an_extended_selection_moves_the_register_address_by_whole_elements)
        assert_eq!(accept("input-register:1[4]:DINT").offset(), 8);
        // bitAreasAreNotScaled
        assert_eq!(accept("coil:1[4]:BOOL").offset(), 4);
        assert_eq!(accept("discrete-input:1[4]:BOOL").offset(), 4);
        // limitsAreCountedInRegisters
        assert_rejected_naming("holding-register:1[0..62]:DINT", "MB-LIMIT-REGISTERS");
        assert_eq!(
            accept("holding-register:1[0..61]:DINT").elements(),
            NonZeroU16::new(62)
        );
        // aSelectionStartingInsideARegisterIsRejected and stringsAdvanceByTheirDeclaredLength
        // name CHAR and STRING, which stay deferred (MB-ADDR-STRING): refused whatever the
        // selection, before the odd byte offset could matter.
        for text in [
            "holding-register:1[1..2]:CHAR",
            "holding-register:1[2..3]:CHAR",
            "holding-register:1[2]:STRING(20)",
        ] {
            assert_rejected_naming(text, "string and character types");
        }
        // aRangeIsAnArrayEvenWhenItSpansOneElement, noSelectionIsAScalar,
        // aRangeReportsItsElements
        assert_eq!(accept("holding-register:1[4]:INT").elements(), None);
        assert_eq!(
            accept("holding-register:1[4..4]:INT").elements(),
            NonZeroU16::new(1)
        );
        assert_eq!(accept("holding-register:1:INT").elements(), None);
        assert_eq!(
            accept("holding-register:1[0..3]:INT").elements(),
            NonZeroU16::new(4)
        );
    }

    // covers: MB-ADDR-SELECTION, MB-LIMIT-COILS, MB-LIMIT-REGISTERS / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusTagTest.java
    // (verifyModbusTag for the coil, discrete-input, holding and input ranges; the extended
    // range is upstream_extended_register_ranges_agree_across_forms_up_to_120): every count
    // up to the limit parses the same in all four spellings, the next one is refused, and the
    // canonical text reads back equal
    #[test]
    fn upstream_modbus_tag_test_ranges_agree_across_forms_up_to_the_limit() {
        let plc4x = named("plc4x");
        for (forms, limit) in [
            (
                [
                    "coil:1[0..%d]:BOOL",
                    "00001[0..%d]:BOOL",
                    "000001[0..%d]:BOOL",
                    "0x00001[0..%d]:BOOL",
                ],
                2_000_u16,
            ),
            (
                [
                    "discrete-input:1[0..%d]:BOOL",
                    "10001[0..%d]:BOOL",
                    "100001[0..%d]:BOOL",
                    "1x00001[0..%d]:BOOL",
                ],
                2_000,
            ),
            (
                [
                    "holding-register:1[0..%d]:INT",
                    "40001[0..%d]:INT",
                    "400001[0..%d]:INT",
                    "4x00001[0..%d]:INT",
                ],
                125,
            ),
            (
                [
                    "input-register:1[0..%d]:INT",
                    "30001[0..%d]:INT",
                    "300001[0..%d]:INT",
                    "3x00001[0..%d]:INT",
                ],
                125,
            ),
        ] {
            let text = |form: &str, last: u16| form.replace("%d", &last.to_string());
            for count in 1..=limit {
                let first = accept_with(&text(forms[0], count - 1), &plc4x);
                assert_eq!(first.elements(), NonZeroU16::new(count), "{}", forms[0]);
                assert_eq!(first.logical_address(), 1, "{}", forms[0]);
                for form in &forms[1..] {
                    assert_eq!(
                        accept_with(&text(form, count - 1), &plc4x),
                        first,
                        "{form} x{count}"
                    );
                }
            }
            for form in forms {
                let refused = ModbusAddress::parse(&text(form, limit), &plc4x);
                assert_eq!(
                    refused.map_err(|e| e.kind()),
                    Err(ErrorKind::InvalidAddress),
                    "{form}: one past the limit"
                );
                let address = accept_with(&text(form, 42), &plc4x);
                assert_eq!(
                    accept_with(&address.canonical(), &plc4x),
                    address,
                    "{form}: the canonical text"
                );
            }
        }
        // aCountTooWideToBeANumberIsAnInvalidTagNotANumberFormatError
        assert_rejected_naming("holding-register:1[0..99999999998]:INT", "too large");
    }

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED, MB-LIMIT-REGISTERS / transcribed from
    // upstream plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusTagTest.java
    // (testExtended_INT_ARRAY_RANGE, Apache PLC4X v1.0.0 commit ea229918), with the selection
    // that the_four_spellings_of_extended_register_1_are_one_address in tests/modbus_parse.rs
    // leaves out: every count up to the limit parses the same in all four spellings, at
    // logical and register address 1 ("Addresses for extended memory start at address 0
    // instead of 1"), the next count is refused, and the canonical text reads back equal.
    // Upstream's allowedMax is 125 (TagExt:106-108); Stonecrab's is 120, the most one file
    // record read carries (ADR-0022, addendum of 2026-10-29), so the counts 121 to 125 that
    // upstream accepts are refused here.
    #[test]
    fn upstream_extended_register_ranges_agree_across_forms_up_to_120() {
        let plc4x = named("plc4x");
        let forms = [
            "extended-register:1[0..%d]:INT",
            "60001[0..%d]:INT",
            "600001[0..%d]:INT",
            "6x00001[0..%d]:INT",
        ];
        let text = |form: &str, last: u16| form.replace("%d", &last.to_string());
        for count in 1..=120_u16 {
            let first = accept_with(&text(forms[0], count - 1), &plc4x);
            assert_eq!(first.area(), ModbusArea::ExtendedRegister, "x{count}");
            assert_eq!(first.elements(), NonZeroU16::new(count), "x{count}");
            assert_eq!(first.logical_address(), 1, "x{count}: logical address");
            assert_eq!(
                (first.file_number().map(NonZeroU16::get), first.offset()),
                (Some(1), 1),
                "x{count}: record 1 of file 1"
            );
            for form in &forms[1..] {
                assert_eq!(
                    accept_with(&text(form, count - 1), &plc4x),
                    first,
                    "{form} x{count}"
                );
            }
        }
        for form in forms {
            for last in 120..=125_u16 {
                let refused = ModbusAddress::parse(&text(form, last), &plc4x);
                let message = refused.map_or_else(|error| error.to_string(), |_| String::new());
                assert!(
                    message.contains("more than the 120 one read carries"),
                    "{form} with {} registers: {message}",
                    last + 1
                );
            }
            let address = accept_with(&text(form, 42), &plc4x);
            assert_eq!(
                accept_with(&address.canonical(), &plc4x),
                address,
                "{form}: the canonical text"
            );
        }
    }

    // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED / an element offset on the extended
    // registers counts registers from the register address, through the end of a file, and
    // an index is the element's own address; the first case is transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/base/tag/ModbusSelectionOffsetTest.java
    // (theOtherRegisterAreasScaleToo: an extended address "is not shifted by the protocol
    // offset the way the others are, so its base stays 1 and the eight-register advance lands
    // on 9")
    #[test]
    fn an_extended_selection_moves_the_register_address_by_whole_elements() {
        let placed = |address: &ModbusAddress| {
            (
                address.logical_address(),
                address.file_number().map(NonZeroU16::get),
                address.offset(),
                address.elements().map(NonZeroU16::get),
            )
        };
        for (text, expected) in [
            ("extended-register:1[4]:DINT", (9, Some(1), 9, None)),
            ("extended-register:1[4..7;1]:INT", (4, Some(1), 4, Some(4))),
            ("6x00001[2]:LREAL", (9, Some(1), 9, None)),
            // An index or a range may start in the next file, or run into it.
            ("extended-register:9998[1]:DINT", (10_000, Some(2), 0, None)),
            (
                "extended-register:9997[1..2]:DINT",
                (9_999, Some(1), 9_999, Some(2)),
            ),
            (
                "extended-register:29998[0..3]:INT",
                (29_998, Some(3), 9_998, Some(4)),
            ),
        ] {
            assert_eq!(placed(&accept(text)), expected, "{text}");
        }
        for (indexed, direct) in [
            ("extended-register:1[4]:DINT", "extended-register:9:DINT"),
            (
                "extended-register:9997[1]:DINT",
                "extended-register:9999:DINT",
            ),
            ("6x09990[6;1]:DINT", "extended-register:10000:DINT"),
        ] {
            assert_eq!(accept(indexed), accept(direct), "{indexed} is {direct}");
        }
        assert_eq!(
            accept("extended-register:9999[0..0]:DINT").elements(),
            NonZeroU16::new(1),
            "a range of one is a list of one"
        );
        // The canonical text writes the range after the address of the first element.
        let range = accept("6x00001[4..7;1]:DINT");
        assert_eq!(
            range.canonical(),
            "extended-register:7[0..3]:DINT",
            "canonical"
        );
        assert_eq!(accept(&range.canonical()), range, "reads back");
        // The number is checked before a selection moves it: address 0 does not exist.
        for text in [
            "extended-register:0[1]:INT",
            "extended-register:0[0..1]",
            "6x00000[2]",
        ] {
            assert_rejected_naming(text, "address 0 does not exist");
        }
        // No convention shifts an extended register, a selection included.
        for convention in ["plc4x", "one-based-40001", "zero-based"] {
            for text in ["extended-register:1[4]:DINT", "600001[4]:DINT"] {
                assert_eq!(
                    accept_with(text, &named(convention)),
                    accept("extended-register:9:DINT"),
                    "{text} under {convention}"
                );
            }
        }
    }

    // covers: MB-LIMIT-EXT, MB-LIMIT-REGISTERS, MB-ADDR-EXTENDED / the whole selected span
    // ends at or before extended address 655349999, and a list fits one file record read, 120
    // registers where upstream allows 125 (ADR-0022, addendum of 2026-10-29); at and past
    // each bound, by the parser and by the builder
    #[test]
    fn an_extended_selection_ends_at_or_before_655349999_and_fits_one_file_record_read() {
        for (text, first, elements) in [
            (
                "extended-register:655349880[0..119]:INT",
                655_349_880,
                Some(120),
            ),
            (
                "extended-register:655349998[0..1]:INT",
                655_349_998,
                Some(2),
            ),
            (
                "extended-register:655349999[0..0]:INT",
                655_349_999,
                Some(1),
            ),
            ("extended-register:655349998[1]:INT", 655_349_999, None),
            ("extended-register:655349988[2]:LREAL", 655_349_996, None),
            ("extended-register:1[0..119]:INT", 1, Some(120)),
            ("extended-register:1[0..59]:DINT", 1, Some(60)),
            ("extended-register:1[0..29]:LREAL", 1, Some(30)),
            ("extended-register:9950[0..119]:INT", 9_950, Some(120)),
        ] {
            let address = accept(text);
            assert_eq!(
                (
                    address.logical_address(),
                    address.elements().map(NonZeroU16::get)
                ),
                (first, elements),
                "{text}: fits"
            );
        }
        for (text, needle) in [
            (
                "extended-register:655349881[0..119]:INT",
                "a list of 120 INT at extended-register address 655349881 spans 120 \
                 register(s) and passes the last address 655349999",
            ),
            (
                "extended-register:655349999[0..1]:INT",
                "passes the last address 655349999",
            ),
            (
                "extended-register:655349989[2]:LREAL",
                "passes the last address 655349999",
            ),
            (
                "extended-register:655349999[1]:INT",
                "the selection starts at extended-register address 655350000, past the last \
                 address 655349999 (docs/parity/modbus.md#MB-LIMIT-EXT)",
            ),
            (
                "extended-register:999999999[0]:INT",
                "starts at extended-register address 999999999",
            ),
            (
                "extended-register:1[0..120]:INT",
                "a list of 121 INT spans 121 registers, more than the 120 one read carries \
                 (docs/parity/modbus.md#MB-LIMIT-REGISTERS)",
            ),
            (
                "extended-register:1[0..124]:INT",
                "spans 125 registers, more than the 120",
            ),
            (
                "extended-register:1[0..60]:DINT",
                "spans 122 registers, more than the 120",
            ),
            (
                "extended-register:1[0..30]:LREAL",
                "spans 124 registers, more than the 120",
            ),
            ("extended-register:1[0..70000]:INT", "holds 70001 elements"),
        ] {
            assert_rejected_naming(text, needle);
        }
        let count = |value: u16| NonZeroU16::new(value).expect("non-zero");
        let built = ModbusAddress::extended(655_349_880)
            .elements(count(120))
            .build()
            .expect("the last 120 registers");
        assert_eq!(
            built,
            accept("extended-register:655349880[0..119]:INT"),
            "the builder makes the parser's address"
        );
        for (address, elements) in [(655_349_881, 120), (1, 121), (0, 1)] {
            let refused = ModbusAddress::extended(address)
                .elements(count(elements))
                .build();
            assert_eq!(
                refused.map_err(|e| e.kind()),
                Err(ErrorKind::InvalidAddress),
                "{address} x{elements}"
            );
        }
    }

    // covers: MB-ADDR-LEGACY-ARRAY, MB-ADDR-EXTENDED / the legacy count forms are refused on
    // the extended registers too, with the replacement only when it parses there
    #[test]
    fn a_legacy_count_on_an_extended_register_is_hinted_only_when_the_range_fits() {
        assert_eq!(
            hint_of(&reject("extended-register:1:INT[4]")),
            Some("extended-register:1[0..3]:INT"),
            "a count of four"
        );
        assert_eq!(
            hint_of(&reject("6x09999:DINT:2")),
            Some("6x09999[0..1]:DINT"),
            "two DINTs across the end of file 1"
        );
        // 121 registers do not fit one file record read; the files end at 655349999.
        for text in [
            "extended-register:1:INT[121]",
            "extended-register:655349999:INT[2]",
        ] {
            let message = reject(text);
            assert_eq!(hint_of(&message), None, "{text}: {message}");
            assert!(
                message.contains("for example 'holding-register:1[0..3]:INT'"),
                "{text}: {message}"
            );
        }
    }

    // covers: MB-ADDR-LEGACY-ARRAY / transcribed from upstream
    // plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusLegacyAddressTest.java
    // and the Modbus-shaped rows of anAddressWrittenBeforeTheMigrationIsRewritten in
    // ArrayNotationParserTest.java
    #[test]
    fn upstream_legacy_address_cases() {
        // legacyForm0IsRejected, legacyForm1IsRejected, theErrorNamesTheReplacementAddress
        let message = reject("holding-register:1:INT[4]");
        assert_eq!(hint_of(&message), Some("holding-register:1[0..3]:INT"));
        assert_eq!(
            hint_of(&reject("coil:1:BOOL[8]")),
            Some("coil:1[0..7]:BOOL")
        );
        // theReplacementFormParses
        assert_eq!(
            accept("holding-register:1[0..3]:INT").elements(),
            NonZeroU16::new(4)
        );
        // 'myTag:DINT:8' -> 'myTag[0..7]:DINT' and 'foo:INT[1]' -> 'foo[0]:INT'
        assert_eq!(
            hint_of(&reject("holding-register:5:DINT:8")),
            Some("holding-register:5[0..7]:DINT")
        );
        assert_eq!(
            hint_of(&reject("holding-register:5:INT[1]")),
            Some("holding-register:5[0]:INT")
        );
        // anAddressThatIsNotInTheOldShapeGetsNoRewrite
        assert!(hint_of(&reject("nonsense")).is_none());
    }

    proptest! {
        // covers: MB-ADDR-SELECTION / a list's canonical text reads back to an equal address
        #[test]
        fn prop_list_canonical_round_trips(
            area in proptest::sample::select(AREAS.to_vec()),
            offset in prop_oneof![any::<u16>(), 63_000_u16..=65_535],
            listed in proptest::sample::select(TYPES.to_vec()),
            count in 1_u16..=2_000,
        ) {
            let built = ModbusAddress::builder(area, offset)
                .data_type(fitting(area, listed))
                .elements(NonZeroU16::new(count).expect("non-zero"))
                .build();
            // A list past the end of the map or wider than one read is not an address.
            let Ok(address) = built else {
                return Ok(());
            };
            let canonical = address.canonical();
            prop_assert_eq!(
                ModbusAddress::parse(&canonical, &none()).ok(),
                Some(address),
                "{}",
                canonical
            );
        }

        // covers: MB-ADDR-SELECTION, MB-LIMIT-SPAN, MB-LIMIT-COILS, MB-LIMIT-REGISTERS / a
        // selection is the address plus (index - base) elements, and fits or is refused
        #[test]
        fn prop_selection_resolves_to_the_base_plus_index_times_width(
            area in proptest::sample::select(AREAS.to_vec()),
            number in 1_u32..=65_536,
            listed in proptest::sample::select(TYPES.to_vec()),
            base in 0_u32..=100,
            ahead in prop_oneof![0_u32..=200, 0_u32..=70_000],
            extra in proptest::option::of(prop_oneof![0_u32..=130, 0_u32..=2_100]),
        ) {
            let data_type = fitting(area, listed);
            let lo = base.saturating_add(ahead);
            let selection = match extra {
                Some(extra) => format!("[{lo}..{};{base}]", lo.saturating_add(extra)),
                None => format!("[{lo};{base}]"),
            };
            let text = format!(
                "{}:{number}{selection}:{}",
                area.long_name(),
                data_type.upstream_name()
            );
            let element = u64::from(width(area, data_type));
            let start = u64::from(number.saturating_sub(1))
                .saturating_add(u64::from(ahead).saturating_mul(element));
            let count = extra.map_or(1, |extra| u64::from(extra).saturating_add(1));
            let span = count.saturating_mul(element);
            let limit = if area.is_bit_area() { 2_000 } else { 125 };
            let fits = start <= 65_535 && start.saturating_add(span) <= 65_536 && span <= limit;
            match ModbusAddress::parse(&text, &none()) {
                Ok(address) => {
                    prop_assert!(fits, "{} was accepted", text);
                    prop_assert_eq!(u64::from(address.offset()), start, "{}", text);
                    prop_assert_eq!(
                        address.elements().map(|count| u64::from(count.get())),
                        extra.map(|_| count),
                        "{}",
                        text
                    );
                }
                Err(error) => {
                    prop_assert!(!fits, "{} was refused: {}", text, error);
                    prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
                }
            }
        }

        // covers: MB-ADDR-SELECTION / an index is the element's own address
        #[test]
        fn prop_an_index_is_the_address_of_its_element(
            area in proptest::sample::select(AREAS.to_vec()),
            number in 1_u32..=65_536,
            listed in proptest::sample::select(TYPES.to_vec()),
            index in 0_u32..=40_000,
        ) {
            let data_type = fitting(area, listed);
            let name = data_type.upstream_name();
            let indexed = format!("{}:{number}[{index}]:{name}", area.long_name());
            let element_number = u64::from(number)
                .saturating_add(u64::from(index).saturating_mul(u64::from(width(area, data_type))));
            let direct = format!("{}:{element_number}:{name}", area.long_name());
            let (left, right) = (
                ModbusAddress::parse(&indexed, &none()),
                ModbusAddress::parse(&direct, &none()),
            );
            prop_assert_eq!(left.as_ref().ok(), right.as_ref().ok(), "{} vs {}", indexed, direct);
            prop_assert_eq!(left.is_err(), right.is_err(), "{} vs {}", indexed, direct);
        }

        // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED, MB-LIMIT-EXT, MB-LIMIT-REGISTERS / on
        // the extended registers a selection is the register address plus (index - base)
        // elements, in the file and record of that address, an index is the element's own
        // address, and the span fits (at most 120 registers, ending at or before 655349999)
        // or is refused
        #[test]
        fn prop_an_extended_selection_is_the_address_plus_index_times_width(
            number in prop_oneof![
                1_u32..=655_349_999,
                9_900_u32..=10_100,
                655_349_800_u32..=655_349_999
            ],
            listed in proptest::sample::select(TYPES.to_vec()),
            base in 0_u32..=100,
            ahead in prop_oneof![0_u32..=200, 0_u32..=5_000_000],
            extra in proptest::option::of(prop_oneof![0_u32..=130, 0_u32..=2_100]),
        ) {
            let name = listed.upstream_name();
            let lo = base.saturating_add(ahead);
            let selection = match extra {
                Some(extra) => format!("[{lo}..{};{base}]", lo.saturating_add(extra)),
                None => format!("[{lo};{base}]"),
            };
            let text = format!("extended-register:{number}{selection}:{name}");
            let element = u64::from(listed.register_size());
            let start = u64::from(number)
                .saturating_add(u64::from(ahead).saturating_mul(element));
            let count = extra.map_or(1, |extra| u64::from(extra).saturating_add(1));
            let span = count.saturating_mul(element);
            let fits = start.saturating_add(span) <= 655_350_000 && span <= 120;
            match ModbusAddress::parse(&text, &none()) {
                Ok(address) => {
                    prop_assert!(fits, "{} was accepted", text);
                    prop_assert_eq!(u64::from(address.logical_address()), start, "{}", text);
                    prop_assert_eq!(
                        address.file_number().map(|file| u64::from(file.get())),
                        start.checked_div(10_000).map(|files| files.saturating_add(1)),
                        "{}",
                        text
                    );
                    prop_assert_eq!(
                        Some(u64::from(address.offset())),
                        start.checked_rem(10_000),
                        "{}",
                        text
                    );
                    prop_assert_eq!(
                        address.elements().map(|count| u64::from(count.get())),
                        extra.map(|_| count),
                        "{}",
                        text
                    );
                    if extra.is_none() {
                        let direct = format!("extended-register:{start}:{name}");
                        prop_assert_eq!(
                            ModbusAddress::parse(&direct, &none()).ok(),
                            Some(address),
                            "{} vs {}",
                            text,
                            direct
                        );
                    }
                }
                Err(error) => {
                    prop_assert!(!fits, "{} was refused: {}", text, error);
                    prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
                }
            }
        }

        // covers: MB-ADDR-SELECTION, MB-ADDR-EXTENDED / an extended list's canonical text
        // reads back to an equal address
        #[test]
        fn prop_an_extended_list_canonical_round_trips(
            address in prop_oneof![1_u32..=655_349_999, 655_349_800_u32..=655_349_999],
            listed in proptest::sample::select(TYPES.to_vec()),
            count in 1_u16..=130,
        ) {
            let built = ModbusAddress::extended(address)
                .data_type(listed)
                .elements(NonZeroU16::new(count).expect("non-zero"))
                .build();
            // A list past the last address or wider than one read is not an address.
            let Ok(built) = built else {
                return Ok(());
            };
            let canonical = built.canonical();
            prop_assert_eq!(
                ModbusAddress::parse(&canonical, &none()).ok(),
                Some(built),
                "{}",
                canonical
            );
        }

        // covers: SPEC mandatory-failure-cases / F3 invalid register (selection text never
        // panics and is never anything but InvalidAddress when refused)
        #[test]
        fn prop_selection_text_never_panics(selection in "\\[[0-9.;,\\[\\] a-z-]{0,24}") {
            for area in ["holding-register:1", "coil:1", "4x00001", "extended-register:1"] {
                let text = format!("{area}{selection}:INT");
                if let Err(error) = ModbusAddress::parse(&text, &none()) {
                    prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{}", text);
                }
            }
        }
    }
}
