//! Tests of the typed decoder (slice S23): the golden vectors `fixtures/modbus/tcp/typed_*`,
//! the upstream driver-suite exchanges `fixtures/modbus/tcp/adu_upstream-*` (slice S04b), the
//! interop register map, the layouts of the four upstream byte-order modes, the
//! single-register padding rules, the register-count and bit-area rejections and the
//! round-trip properties (volume set by `PROPTEST_CASES`).
//!
//! The expected files of the `typed_*` vectors are written by hand from the documented
//! layouts (docs/parity/modbus.md, "Byte order"), never by this decoder. Decode and encode
//! are asserted separately and each against the file, so a decoder bug and an encoder bug
//! cannot cancel out. The upstream exchanges are serialized from the XML models of the
//! retained PLC4X driver suites by `tests/modbus_driver_suite_adapter.rs` (not by this
//! decoder, and not by Stonecrab's codec); their registers are decoded here with the type and
//! the payload byte order the XML case declares and compared with the value the XML reports.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::num::NonZeroUsize;
    use std::path::Path;
    use std::time::Duration;

    use proptest::prelude::*;
    use stonecrab::codec::pack_bits_lsb_first;
    use stonecrab::modbus::{ModbusAddress, ModbusDataType, TypedDecoder};
    use stonecrab::testkit::{GoldenVector, InteropMap, VectorSource, load_dir};
    use stonecrab::{AddressOptions, ByteOrder, ErrorKind, ItemError, PlcValue, WordOrder};

    type Mode = (ByteOrder, WordOrder);

    const BIG_ENDIAN: Mode = (ByteOrder::BigEndian, WordOrder::HighWordFirst);
    const LITTLE_ENDIAN: Mode = (ByteOrder::LittleEndian, WordOrder::LowWordFirst);
    const BIG_ENDIAN_BYTE_SWAP: Mode = (ByteOrder::LittleEndian, WordOrder::HighWordFirst);
    const LITTLE_ENDIAN_BYTE_SWAP: Mode = (ByteOrder::BigEndian, WordOrder::LowWordFirst);

    /// The four upstream mode names and the pair each maps to (ADR-0006 decision 4).
    const MODES: [(&str, Mode); 4] = [
        ("BIG_ENDIAN", BIG_ENDIAN),
        ("LITTLE_ENDIAN", LITTLE_ENDIAN),
        ("BIG_ENDIAN_BYTE_SWAP", BIG_ENDIAN_BYTE_SWAP),
        ("LITTLE_ENDIAN_BYTE_SWAP", LITTLE_ENDIAN_BYTE_SWAP),
    ];

    const ALL_TYPES: [ModbusDataType; 15] = [
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

    const NARROW: [ModbusDataType; 4] = [
        ModbusDataType::Bool,
        ModbusDataType::Byte,
        ModbusDataType::SInt,
        ModbusDataType::USInt,
    ];

    /// A REAL from its decimal text, for constants that resemble `PI` or `E`.
    fn real(text: &str) -> PlcValue {
        PlcValue::F32(text.parse().expect("an f32 literal"))
    }

    /// An LREAL from its decimal text.
    fn lreal(text: &str) -> PlcValue {
        PlcValue::F64(text.parse().expect("an f64 literal"))
    }

    fn budget(bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new(bytes).expect("a non-zero budget")
    }

    fn decode(registers: &[u16], data_type: ModbusDataType, mode: Mode) -> PlcValue {
        TypedDecoder::decode_registers(registers, data_type, mode.0, mode.1, budget(64))
            .unwrap_or_else(|e| panic!("{registers:04X?} as {data_type:?} under {mode:?}: {e}"))
    }

    fn encode(value: &PlcValue, data_type: ModbusDataType, mode: Mode) -> Vec<u16> {
        TypedDecoder::encode_registers(value, data_type, mode.0, mode.1)
            .unwrap_or_else(|e| panic!("{value:?} as {data_type:?} under {mode:?}: {e}"))
    }

    fn failure(registers: &[u16], data_type: ModbusDataType, limit: usize) -> ItemError {
        TypedDecoder::decode_registers(
            registers,
            data_type,
            ByteOrder::BigEndian,
            WordOrder::HighWordFirst,
            budget(limit),
        )
        .expect_err("a wrong register count is rejected")
    }

    /// Value equality that compares float bit patterns (a NaN is equal to itself).
    fn same(got: &PlcValue, want: &PlcValue) -> bool {
        match (got, want) {
            (PlcValue::F32(a), PlcValue::F32(b)) => a.to_bits() == b.to_bits(),
            (PlcValue::F64(a), PlcValue::F64(b)) => a.to_bits() == b.to_bits(),
            _ => got == want,
        }
    }

    /// The bit pattern of an `F32`, or `None` for any other value.
    fn single_bits(value: &PlcValue) -> Option<u32> {
        if let PlcValue::F32(float) = value {
            Some(float.to_bits())
        } else {
            None
        }
    }

    /// The bit pattern of an `F64`, or `None` for any other value.
    fn double_bits(value: &PlcValue) -> Option<u64> {
        if let PlcValue::F64(float) = value {
            Some(float.to_bits())
        } else {
            None
        }
    }

    fn assert_same(got: &PlcValue, want: &PlcValue, context: &str) {
        assert!(same(got, want), "{context}: got {got:?}, want {want:?}");
    }

    fn registers_of(bytes: &[u8]) -> Vec<u16> {
        assert!(
            bytes.len().is_multiple_of(2),
            "whole registers: {bytes:02X?}"
        );
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[high, low]| u16::from_be_bytes([*high, *low]))
            .collect()
    }

    fn wire_of(registers: &[u16]) -> Vec<u8> {
        registers.iter().flat_map(|r| r.to_be_bytes()).collect()
    }

    fn type_named(name: &str) -> ModbusDataType {
        ModbusDataType::from_upstream_name(name).expect("one of the 15 type names")
    }

    fn mode_named(name: &str) -> Mode {
        MODES
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map_or_else(
                || panic!("unknown byte order name {name}"),
                |(_, mode)| *mode,
            )
    }

    /// The mode the address parser maps `name` to.
    fn parsed_mode(name: &str) -> Mode {
        let text = format!("holding-register:1:DWORD{{byte-order: '{name}'}}");
        ModbusAddress::parse(&text, &AddressOptions::default())
            .expect("the address parses")
            .byte_order()
            .expect("the byte order is set")
    }

    // ---- the golden vectors ---------------------------------------------------------

    /// The `typed_*` vectors (the `PROVENANCE.toml` is one level up, at `fixtures/modbus`).
    fn vectors() -> Vec<GoldenVector> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        load_dir(&dir)
            .expect("fixtures/modbus loads: every file listed, every listed vector present")
            .into_iter()
            .filter(|vector| vector.name.starts_with("tcp/typed_"))
            .collect()
    }

    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "serde_json is not a dependency of this crate, so its Value type cannot be named"
    )]
    fn unsigned(vector: &GoldenVector, key: &str) -> u64 {
        vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get(key))
            .and_then(|value| value.as_u64())
            .unwrap_or_else(|| panic!("{}: `{key}` is not an unsigned number", vector.name))
    }

    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "serde_json is not a dependency of this crate, so its Value type cannot be named"
    )]
    fn signed(vector: &GoldenVector, key: &str) -> i64 {
        vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get(key))
            .and_then(|value| value.as_i64())
            .unwrap_or_else(|| panic!("{}: `{key}` is not a signed number", vector.name))
    }

    #[expect(
        clippy::redundant_closure_for_method_calls,
        reason = "serde_json is not a dependency of this crate, so its Value type cannot be named"
    )]
    fn flag(vector: &GoldenVector, key: &str) -> bool {
        vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get(key))
            .and_then(|value| value.as_bool())
            .unwrap_or_else(|| panic!("{}: `{key}` is not a Boolean", vector.name))
    }

    fn text<'a>(vector: &'a GoldenVector, key: &str) -> &'a str {
        vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get(key))
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| panic!("{}: `{key}` is not text", vector.name))
    }

    fn bits_of(vector: &GoldenVector) -> u64 {
        let digits = text(vector, "bits").trim_start_matches("0x");
        u64::from_str_radix(digits, 16).expect("hexadecimal bits")
    }

    /// The value the expected file describes, built without the decoder.
    fn expected_value(vector: &GoldenVector) -> PlcValue {
        let single = || u32::try_from(bits_of(vector)).expect("a u32 bit pattern");
        match text(vector, "type") {
            "Bool" => PlcValue::Bool(flag(vector, "value")),
            "U8" => PlcValue::U8(u8::try_from(unsigned(vector, "value")).expect("a u8")),
            "U16" => PlcValue::U16(u16::try_from(unsigned(vector, "value")).expect("a u16")),
            "U32" => PlcValue::U32(u32::try_from(unsigned(vector, "value")).expect("a u32")),
            "U64" => PlcValue::U64(unsigned(vector, "value")),
            "I8" => PlcValue::I8(i8::try_from(signed(vector, "value")).expect("an i8")),
            "I16" => PlcValue::I16(i16::try_from(signed(vector, "value")).expect("an i16")),
            "I32" => PlcValue::I32(i32::try_from(signed(vector, "value")).expect("an i32")),
            "I64" => PlcValue::I64(signed(vector, "value")),
            "F32" => PlcValue::F32(f32::from_bits(single())),
            "F64" => PlcValue::F64(f64::from_bits(bits_of(vector))),
            other => panic!("{}: unknown value type {other}", vector.name),
        }
    }

    fn vector_mode(vector: &GoldenVector) -> Mode {
        mode_named(text(vector, "byte_order"))
    }

    fn vector_type(vector: &GoldenVector) -> ModbusDataType {
        type_named(text(vector, "data_type"))
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, swapped words, integer boundaries
    #[test]
    fn typed_vectors_cover_every_type_in_every_mode() {
        let all = vectors();
        for data_type in ALL_TYPES {
            for (name, _) in MODES {
                let want = format!(
                    "tcp/typed_{}_{}",
                    data_type.upstream_name().to_lowercase(),
                    name.to_lowercase()
                );
                let vector = all
                    .iter()
                    .find(|v| v.name == want)
                    .unwrap_or_else(|| panic!("{want}: no such vector"));
                assert!(!vector.must_fail, "{want}: an accepted vector");
                assert_eq!(
                    text(vector, "data_type"),
                    data_type.upstream_name(),
                    "{want}"
                );
                assert_eq!(text(vector, "byte_order"), name, "{want}");
            }
        }
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let provenance = std::fs::read_to_string(dir.join("PROVENANCE.toml")).expect("readable");
        let listed = provenance
            .lines()
            .filter(|line| line.starts_with("name = \"tcp/typed_"))
            .count();
        assert_eq!(
            all.len(),
            listed,
            "typed vectors loaded vs listed in PROVENANCE.toml"
        );
        assert!(all.len() >= 80, "{} typed vectors loaded", all.len());
        for vector in &all {
            assert!(
                matches!(
                    vector.source,
                    VectorSource::ProtocolSpec | VectorSource::Adversarial
                ),
                "{}: until P4 is granted no vector is upstream or captured",
                vector.name
            );
            assert_eq!(
                vector.must_fail,
                vector.source == VectorSource::Adversarial,
                "{}: adversarial vectors are the must_fail ones",
                vector.name
            );
            assert!(
                vector.expected.is_some(),
                "{}: has an expected file",
                vector.name
            );
        }
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, swapped words, integer boundaries
    #[test]
    fn typed_vectors_decode_to_the_expected_values() {
        let mut checked = 0_usize;
        for vector in vectors().iter().filter(|v| !v.must_fail) {
            let got = decode(
                &registers_of(&vector.bytes),
                vector_type(vector),
                vector_mode(vector),
            );
            assert_same(&got, &expected_value(vector), &vector.name);
            checked = checked.saturating_add(1);
        }
        assert!(checked >= 74, "{checked} accepted vectors decoded");
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
    #[test]
    fn typed_vectors_encode_to_the_register_bytes() {
        for vector in vectors().iter().filter(|v| !v.must_fail) {
            let registers = encode(
                &expected_value(vector),
                vector_type(vector),
                vector_mode(vector),
            );
            assert_eq!(
                wire_of(&registers),
                vector.bytes,
                "{}: encoded register bytes",
                vector.name
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
    #[test]
    fn typed_must_fail_vectors_are_rejected_with_raw_bytes_attached() {
        let rejected: Vec<GoldenVector> = vectors().into_iter().filter(|v| v.must_fail).collect();
        assert!(rejected.len() >= 5, "{} must_fail vectors", rejected.len());
        for vector in &rejected {
            let registers = registers_of(&vector.bytes);
            let count = unsigned(vector, "register_count");
            assert_eq!(
                u64::try_from(registers.len()).expect("a count"),
                count,
                "{}: the file counts its registers",
                vector.name
            );
            let mode = vector_mode(vector);
            let error = TypedDecoder::decode_registers(
                &registers,
                vector_type(vector),
                mode.0,
                mode.1,
                budget(64),
            )
            .expect_err(&vector.name);
            assert_eq!(error.kind(), ErrorKind::MalformedData, "{}", vector.name);
            assert_eq!(text(vector, "error"), "MalformedData", "{}", vector.name);
            let status = error.status().expect("a status");
            assert_eq!(status.protocol(), "modbus", "{}", vector.name);
            assert_eq!(status.code(), 0, "{}", vector.name);
            assert_eq!(status.name(), Some("decode"), "{}", vector.name);
            let raw = status.raw().expect("raw bytes attached");
            assert_eq!(
                u64::try_from(raw.len()).expect("a length"),
                unsigned(vector, "raw_len"),
                "{}: raw length",
                vector.name
            );
            assert_eq!(raw, vector.bytes, "{}: the bytes as received", vector.name);
        }
    }

    // ---- the upstream driver-suite exchanges (S04b) ----------------------------------

    /// The read response vectors the adapter serialized from the case models of `file`; the
    /// write exchanges are tested in `modbus_write_vectors.rs`.
    fn upstream_responses(file: &str) -> Vec<GoldenVector> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        load_dir(&dir)
            .expect("fixtures/modbus loads: every file listed, every listed vector present")
            .into_iter()
            .filter(|vector| {
                text_at(vector, &["upstream", "file"]) == Some(file)
                    && text_at(vector, &["upstream", "operation"]) != Some("write")
                    && text_at(vector, &["pdu", "direction"]) == Some("response")
            })
            .collect()
    }

    fn value_at<'a>(vector: &'a GoldenVector, path: &[&str]) -> Option<&'a serde_json::Value> {
        path.iter()
            .try_fold(vector.expected.as_ref()?, |value, key| value.get(*key))
    }

    fn text_at<'a>(vector: &'a GoldenVector, path: &[&str]) -> Option<&'a str> {
        value_at(vector, path)?.as_str()
    }

    /// Whether `got` is the value the XML reports as `text`: exact for integers, `BOOL` and
    /// `REAL` (the text is the shortest decimal of the `f32`), within half a unit of the last
    /// written digit for `LREAL` (the text `2.71828182846` is a rounded `e`).
    fn matches_upstream(got: &PlcValue, data_type: ModbusDataType, text: &str) -> bool {
        if let PlcValue::F64(float) = got {
            let want: f64 = text.parse().expect("a decimal");
            let places = text
                .split_once('.')
                .map_or(0, |(_, fraction)| fraction.len());
            let unit = 10_f64.powi(
                i32::try_from(places)
                    .expect("a few places")
                    .saturating_neg(),
            );
            return (float - want).abs() <= unit / 2.0;
        }
        let decimal = text.strip_prefix("0x").map_or_else(
            || text.to_owned(),
            |hex| {
                u64::from_str_radix(hex, 16)
                    .expect("hex digits")
                    .to_string()
            },
        );
        same(got, &named_value(data_type, &decimal))
    }

    /// Decode every response of `file` under `mode_name` (the order the file declares) and
    /// compare each element with the value the case reports. Returns the types seen.
    fn check_upstream_file(file: &str, mode_name: &str) -> BTreeSet<&'static str> {
        let mut seen = BTreeSet::new();
        for vector in upstream_responses(file) {
            let at = &vector.name;
            assert_eq!(
                text_at(&vector, &["upstream", "byte_order"]),
                Some(mode_name),
                "{at}: the order the XML declares"
            );
            let data_type = type_named(
                text_at(&vector, &["upstream", "tag", "data_type"]).expect("the declared type"),
            );
            // The ADU is the 7 byte header, the function code, the byte count, then registers.
            let registers = registers_of(vector.bytes.get(9..).expect("an ADU with a payload"));
            let listed = value_at(&vector, &["pdu", "registers"])
                .and_then(serde_json::Value::as_array)
                .expect("the register list");
            assert_eq!(registers.len(), listed.len(), "{at}: register count");
            let values = value_at(&vector, &["upstream", "values"])
                .and_then(serde_json::Value::as_array)
                .expect("the reported values");
            let width = usize::from(data_type.register_size());
            assert_eq!(
                registers.len(),
                values.len().saturating_mul(width),
                "{at}: registers per element"
            );
            for (chunk, value) in registers.chunks(width).zip(values) {
                let want = value
                    .get("text")
                    .and_then(serde_json::Value::as_str)
                    .expect("text");
                assert_eq!(
                    value.get("type").and_then(serde_json::Value::as_str),
                    Some(data_type.upstream_name()),
                    "{at}: the reported value has the tag's type"
                );
                let got = decode(chunk, data_type, mode_named(mode_name));
                assert!(
                    matches_upstream(&got, data_type, want),
                    "{at}: {chunk:04X?} decode to {got:?}, the XML reports {want}"
                );
            }
            seen.insert(data_type.upstream_name());
        }
        seen
    }

    // covers: MB-BYTEORDER-BE
    #[test]
    fn upstream_big_endian_exchanges_decode_to_the_values_the_xml_reports() {
        let driver_suite = check_upstream_file("DriverTestsuite.xml", "BIG_ENDIAN");
        assert_eq!(
            driver_suite,
            BTreeSet::from(["REAL"]),
            "the four driver-suite reads"
        );
        assert_eq!(
            upstream_responses("DriverTestsuite.xml").len(),
            4,
            "read exchanges"
        );
        let all = check_upstream_file("Modbus-all-datatypes.xml", "BIG_ENDIAN");
        assert_eq!(
            all.len(),
            14,
            "every type but LWORD, which the file does not read"
        );
        assert_eq!(
            upstream_responses("Modbus-all-datatypes.xml").len(),
            14,
            "read exchanges"
        );
    }

    // covers: MB-BYTEORDER-LE
    #[test]
    fn upstream_little_endian_exchanges_decode_to_the_values_the_xml_reports() {
        let file = "Modbus-all-datatypes-little-endian.xml";
        let seen = check_upstream_file(file, "LITTLE_ENDIAN");
        assert_eq!(
            seen.len(),
            14,
            "every type but LWORD, which the file does not read"
        );
        assert_eq!(upstream_responses(file).len(), 14, "read exchanges");
    }

    // covers: MB-BYTEORDER-LE
    #[test]
    fn upstream_little_endian_registers_are_not_the_big_endian_values() {
        let file = "Modbus-all-datatypes-little-endian.xml";
        for vector in upstream_responses(file) {
            let data_type = type_named(
                text_at(&vector, &["upstream", "tag", "data_type"]).expect("the declared type"),
            );
            let registers = registers_of(vector.bytes.get(9..).expect("an ADU with a payload"));
            let reported = value_at(&vector, &["upstream", "values"])
                .and_then(serde_json::Value::as_array)
                .and_then(|values| values.first())
                .and_then(|value| value.get("text"))
                .and_then(serde_json::Value::as_str)
                .expect("the reported value");
            let misread = TypedDecoder::decode_registers(
                &registers,
                data_type,
                BIG_ENDIAN.0,
                BIG_ENDIAN.1,
                budget(64),
            );
            assert!(
                !matches!(&misread, Ok(got) if matches_upstream(got, data_type, reported)),
                "{}: the vector would pass under the wrong order",
                vector.name
            );
        }
    }

    // ---- the four upstream byte-order modes ------------------------------------------

    // covers: MB-BYTEORDER-BE
    #[test]
    fn byte_order_big_endian_maps_big_high() {
        assert_eq!(parsed_mode("BIG_ENDIAN"), BIG_ENDIAN);
        assert_eq!(
            decode(&[0x0102, 0x0304], ModbusDataType::DWord, BIG_ENDIAN),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-LE
    #[test]
    fn byte_order_little_endian_maps_little_low() {
        assert_eq!(parsed_mode("LITTLE_ENDIAN"), LITTLE_ENDIAN);
        assert_eq!(
            decode(&[0x0403, 0x0201], ModbusDataType::DWord, LITTLE_ENDIAN),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-BE-SWAP
    #[test]
    fn byte_order_big_endian_byte_swap_maps_little_high() {
        assert_eq!(parsed_mode("BIG_ENDIAN_BYTE_SWAP"), BIG_ENDIAN_BYTE_SWAP);
        assert_eq!(
            decode(
                &[0x0201, 0x0403],
                ModbusDataType::DWord,
                BIG_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-LE-SWAP
    #[test]
    fn byte_order_little_endian_byte_swap_maps_big_low() {
        assert_eq!(
            parsed_mode("LITTLE_ENDIAN_BYTE_SWAP"),
            LITTLE_ENDIAN_BYTE_SWAP
        );
        assert_eq!(
            decode(
                &[0x0304, 0x0102],
                ModbusDataType::DWord,
                LITTLE_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-BE, MB-BYTEORDER-LE
    #[test]
    fn decode_dword_layout_1234_and_4321() {
        let wire = |bytes: [u8; 4]| registers_of(&bytes);
        let value = PlcValue::U32(0x0102_0304);
        assert_eq!(
            decode(&wire([1, 2, 3, 4]), ModbusDataType::DWord, BIG_ENDIAN),
            value
        );
        assert_eq!(
            decode(&wire([4, 3, 2, 1]), ModbusDataType::DWord, LITTLE_ENDIAN),
            value
        );
    }

    // covers: MB-BYTEORDER-BE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn decode_dword_layout_2143() {
        assert_eq!(
            decode(
                &registers_of(&[2, 1, 4, 3]),
                ModbusDataType::DWord,
                BIG_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-LE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn decode_dword_layout_3412() {
        assert_eq!(
            decode(
                &registers_of(&[3, 4, 1, 2]),
                ModbusDataType::DWord,
                LITTLE_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U32(0x0102_0304)
        );
    }

    // covers: MB-BYTEORDER-BE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn decode_lword_layout_21436587() {
        assert_eq!(
            decode(
                &registers_of(&[2, 1, 4, 3, 6, 5, 8, 7]),
                ModbusDataType::LWord,
                BIG_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U64(0x0102_0304_0506_0708)
        );
    }

    // covers: MB-BYTEORDER-LE-SWAP
    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn decode_lword_layout_78563412() {
        assert_eq!(
            decode(
                &registers_of(&[7, 8, 5, 6, 3, 4, 1, 2]),
                ModbusDataType::LWord,
                LITTLE_ENDIAN_BYTE_SWAP
            ),
            PlcValue::U64(0x0102_0304_0506_0708)
        );
    }

    // covers: MB-BYTEORDER-LE
    #[test]
    fn decode_lword_layout_87654321() {
        assert_eq!(
            decode(
                &registers_of(&[8, 7, 6, 5, 4, 3, 2, 1]),
                ModbusDataType::LWord,
                LITTLE_ENDIAN
            ),
            PlcValue::U64(0x0102_0304_0506_0708)
        );
    }

    // covers: SPEC mandatory-failure-cases / F7 swapped words
    #[test]
    fn the_two_axes_are_independent() {
        let high = PlcValue::U32(0xAABB_CCDD);
        // Word order alone: the registers of the same bytes swap places.
        for byte_order in [ByteOrder::BigEndian, ByteOrder::LittleEndian] {
            let first = TypedDecoder::encode_registers(
                &high,
                ModbusDataType::UDInt,
                byte_order,
                WordOrder::HighWordFirst,
            )
            .expect("encodes");
            let second = TypedDecoder::encode_registers(
                &high,
                ModbusDataType::UDInt,
                byte_order,
                WordOrder::LowWordFirst,
            )
            .expect("encodes");
            let reversed: Vec<u16> = second.iter().rev().copied().collect();
            assert_eq!(
                first, reversed,
                "{byte_order:?}: word order reverses registers"
            );
        }
        // Byte order alone: every register swaps its bytes.
        for word_order in [WordOrder::HighWordFirst, WordOrder::LowWordFirst] {
            let big = TypedDecoder::encode_registers(
                &high,
                ModbusDataType::UDInt,
                ByteOrder::BigEndian,
                word_order,
            )
            .expect("encodes");
            let little = TypedDecoder::encode_registers(
                &high,
                ModbusDataType::UDInt,
                ByteOrder::LittleEndian,
                word_order,
            )
            .expect("encodes");
            let swapped: Vec<u16> = big.iter().map(|r| r.swap_bytes()).collect();
            assert_eq!(little, swapped, "{word_order:?}: byte order swaps bytes");
        }
    }

    // ---- the types, with the interop constants ------------------------------------------

    // covers: MB-TYPE-BOOL
    #[test]
    fn decode_bool_register_value_1_is_true() {
        assert_eq!(
            decode(&[0x0001], ModbusDataType::Bool, BIG_ENDIAN),
            PlcValue::Bool(true)
        );
        assert_eq!(
            decode(&[0x0000], ModbusDataType::Bool, BIG_ENDIAN),
            PlcValue::Bool(false)
        );
    }

    // covers: MB-TYPE-BOOL
    #[test]
    fn decode_bool_coil_bit() {
        let budget = budget(8);
        // Interop coils 1 to 10 packed least significant bit first.
        let coils = [
            true, false, true, true, false, true, false, false, true, false,
        ];
        let packed = pack_bits_lsb_first(&coils);
        for (index, want) in coils.iter().enumerate() {
            let got = TypedDecoder::decode_bits(&packed, index, ModbusDataType::Bool, budget);
            assert_eq!(got.expect("decodes"), PlcValue::Bool(*want), "coil {index}");
        }
    }

    // covers: MB-TYPE-BYTE
    #[test]
    fn decode_byte_42_from_register() {
        assert_eq!(
            decode(&[0x002A], ModbusDataType::Byte, BIG_ENDIAN),
            PlcValue::U8(42)
        );
    }

    // covers: MB-TYPE-WORD
    #[test]
    fn decode_word_42424() {
        assert_eq!(
            decode(&[0xA5B8], ModbusDataType::Word, BIG_ENDIAN),
            PlcValue::U16(42_424)
        );
    }

    // covers: MB-TYPE-DWORD, MB-BYTEORDER-BE
    #[test]
    fn decode_dword_4242442424_big_endian() {
        assert_eq!(
            decode(&[0xFCDE, 0x88B8], ModbusDataType::DWord, BIG_ENDIAN),
            PlcValue::U32(4_242_442_424)
        );
    }

    // covers: MB-TYPE-LWORD
    #[test]
    fn decode_lword_u64_exact() {
        assert_eq!(
            decode(
                &[0x3AE0, 0x2EE8, 0x4D04, 0x49B2],
                ModbusDataType::LWord,
                BIG_ENDIAN
            ),
            PlcValue::U64(4_242_442_424_242_424_242)
        );
        // Above 2^53, where a float would round: every low bit survives.
        assert_eq!(
            decode(
                &[0x0020, 0x0000, 0x0000, 0x0001],
                ModbusDataType::LWord,
                BIG_ENDIAN
            ),
            PlcValue::U64((1 << 53) + 1)
        );
    }

    // covers: MB-TYPE-SINT
    #[test]
    fn decode_sint_minus_42() {
        assert_eq!(
            decode(&[0xFFD6], ModbusDataType::SInt, BIG_ENDIAN),
            PlcValue::I8(-42),
            "the interop register 0xFFD6: the pad byte is ignored"
        );
        assert_eq!(
            decode(&[0x00D6], ModbusDataType::SInt, BIG_ENDIAN),
            PlcValue::I8(-42)
        );
        assert_eq!(
            decode(&[0xD600], ModbusDataType::SInt, LITTLE_ENDIAN),
            PlcValue::I8(-42)
        );
    }

    // covers: MB-TYPE-INT
    #[test]
    fn decode_int_minus_2424() {
        assert_eq!(
            decode(&[0xF688], ModbusDataType::Int, BIG_ENDIAN),
            PlcValue::I16(-2424)
        );
    }

    // covers: MB-TYPE-INT
    // covers: SPEC mandatory-failure-cases / F7 integer boundaries
    #[test]
    fn decode_int_boundaries() {
        for mode in MODES.map(|(_, mode)| mode) {
            for value in [i16::MIN, -1, 0, 1, i16::MAX] {
                let registers = encode(&PlcValue::I16(value), ModbusDataType::Int, mode);
                assert_eq!(registers.len(), 1, "one register");
                assert_eq!(
                    decode(&registers, ModbusDataType::Int, mode),
                    PlcValue::I16(value),
                    "{value} under {mode:?}"
                );
            }
        }
        assert_eq!(
            decode(&[0x8000], ModbusDataType::Int, BIG_ENDIAN),
            PlcValue::I16(i16::MIN)
        );
        assert_eq!(
            decode(&[0x7FFF], ModbusDataType::Int, BIG_ENDIAN),
            PlcValue::I16(i16::MAX)
        );
        assert_eq!(
            decode(&[0x0080], ModbusDataType::Int, LITTLE_ENDIAN),
            PlcValue::I16(i16::MIN),
            "swapped bytes: 0x0080 is 0x8000"
        );
    }

    // covers: MB-TYPE-DINT
    #[test]
    fn decode_dint_minus_242442424() {
        assert_eq!(
            decode(&[0xF18C, 0x9F48], ModbusDataType::DInt, BIG_ENDIAN),
            PlcValue::I32(-242_442_424)
        );
    }

    // covers: MB-TYPE-LINT
    // covers: SPEC mandatory-failure-cases / F7 integer boundaries
    #[test]
    fn decode_lint_i64_boundaries() {
        for mode in MODES.map(|(_, mode)| mode) {
            for value in [i64::MIN, i64::MIN + 1, -1, 0, i64::MAX - 1, i64::MAX] {
                let registers = encode(&PlcValue::I64(value), ModbusDataType::LInt, mode);
                assert_eq!(registers.len(), 4, "four registers");
                assert_eq!(
                    decode(&registers, ModbusDataType::LInt, mode),
                    PlcValue::I64(value),
                    "{value} under {mode:?}"
                );
            }
        }
        assert_eq!(
            decode(
                &[0xC51F, 0xD117, 0xB2FB, 0xB64E],
                ModbusDataType::LInt,
                BIG_ENDIAN
            ),
            PlcValue::I64(-4_242_442_424_242_424_242)
        );
        assert_eq!(
            decode(&[0x8000, 0, 0, 0], ModbusDataType::LInt, BIG_ENDIAN),
            PlcValue::I64(i64::MIN)
        );
    }

    // covers: MB-TYPE-USINT
    #[test]
    fn decode_usint_42() {
        assert_eq!(
            decode(&[0x002A], ModbusDataType::USInt, BIG_ENDIAN),
            PlcValue::U8(42)
        );
        assert_eq!(
            decode(&[0x2A00], ModbusDataType::USInt, LITTLE_ENDIAN),
            PlcValue::U8(42)
        );
    }

    // covers: MB-TYPE-UINT
    #[test]
    fn decode_uint_42424() {
        assert_eq!(
            decode(&[0xA5B8], ModbusDataType::UInt, BIG_ENDIAN),
            PlcValue::U16(42_424)
        );
    }

    // covers: MB-TYPE-UDINT
    #[test]
    fn decode_udint_4242442424() {
        assert_eq!(
            decode(&[0xFCDE, 0x88B8], ModbusDataType::UDInt, BIG_ENDIAN),
            PlcValue::U32(4_242_442_424)
        );
    }

    // covers: MB-TYPE-ULINT
    // covers: SPEC mandatory-failure-cases / F7 integer boundaries
    #[test]
    fn decode_ulint_u64_max_exact() {
        for mode in MODES.map(|(_, mode)| mode) {
            let registers = encode(&PlcValue::U64(u64::MAX), ModbusDataType::ULInt, mode);
            assert_eq!(
                registers, [0xFFFF; 4],
                "{mode:?}: all bits set in any order"
            );
            assert_eq!(
                decode(&registers, ModbusDataType::ULInt, mode),
                PlcValue::U64(u64::MAX)
            );
        }
        assert_eq!(
            decode(&[0, 0, 0, 0], ModbusDataType::ULInt, BIG_ENDIAN),
            PlcValue::U64(0)
        );
        assert_eq!(
            decode(
                &[0x3AE0, 0x2EE8, 0x4D04, 0x49B2],
                ModbusDataType::ULInt,
                BIG_ENDIAN
            ),
            PlcValue::U64(4_242_442_424_242_424_242)
        );
    }

    // covers: MB-TYPE-REAL, MB-BYTEORDER-BE
    #[test]
    fn decode_real_3_141593_big_endian() {
        assert_same(
            &decode(&[0x4049, 0x0FDC], ModbusDataType::Real, BIG_ENDIAN),
            &real("3.141593"),
            "interop holding registers 26 and 27",
        );
    }

    // covers: MB-TYPE-REAL, MB-BYTEORDER-LE
    #[test]
    fn decode_real_little_endian_region() {
        // The Stonecrab-only region of the interop map: holding registers 300 and 301.
        assert_same(
            &decode(&[0xDC0F, 0x4940], ModbusDataType::Real, LITTLE_ENDIAN),
            &real("3.141593"),
            "REAL 3.141593 under LITTLE_ENDIAN",
        );
        // The same registers read as BIG_ENDIAN are a different number: the mode is never guessed.
        assert!(
            !same(
                &decode(&[0xDC0F, 0x4940], ModbusDataType::Real, BIG_ENDIAN),
                &real("3.141593")
            ),
            "no inference of the order from plausibility"
        );
    }

    // covers: MB-TYPE-REAL
    // covers: SPEC mandatory-failure-cases / F7 NaN and infinity preserved for the sensors to judge
    #[test]
    fn decode_real_nan_and_infinity_preserved() {
        let patterns = [
            0x7FC0_0001_u32, // quiet NaN with a payload
            0xFFC1_2345,     // negative quiet NaN with a payload
            0x7F80_0001,     // signalling NaN
            0x7F80_0000,     // +infinity
            0xFF80_0000,     // -infinity
            0x8000_0000,     // -0.0
            0x0000_0001,     // the smallest subnormal
        ];
        for mode in MODES.map(|(_, mode)| mode) {
            for bits in patterns {
                let value = PlcValue::F32(f32::from_bits(bits));
                let registers = encode(&value, ModbusDataType::Real, mode);
                let got = decode(&registers, ModbusDataType::Real, mode);
                assert_eq!(single_bits(&got), Some(bits), "{bits:#010x} under {mode:?}");
            }
        }
        assert!(
            matches!(
                decode(&[0x7F80, 0x0000], ModbusDataType::Real, BIG_ENDIAN),
                PlcValue::F32(f) if f.is_infinite() && f.is_sign_positive()
            ),
            "+infinity"
        );
    }

    // covers: MB-TYPE-LREAL
    // covers: SPEC mandatory-failure-cases / F7 NaN and infinity preserved for the sensors to judge
    #[test]
    fn decode_lreal_nan_and_infinity_preserved() {
        let patterns = [
            0x7FF8_0000_0000_0001_u64,
            0xFFF8_0000_DEAD_BEEF,
            0x7FF0_0000_0000_0001,
            0x7FF0_0000_0000_0000,
            0xFFF0_0000_0000_0000,
            0x8000_0000_0000_0000,
            0x0000_0000_0000_0001,
        ];
        for mode in MODES.map(|(_, mode)| mode) {
            for bits in patterns {
                let value = PlcValue::F64(f64::from_bits(bits));
                let registers = encode(&value, ModbusDataType::LReal, mode);
                assert_eq!(
                    double_bits(&decode(&registers, ModbusDataType::LReal, mode)),
                    Some(bits),
                    "{bits:#018x} under {mode:?}"
                );
            }
        }
    }

    // covers: MB-TYPE-LREAL
    #[test]
    fn decode_lreal_2_71828182846() {
        assert_same(
            &decode(
                &[0x4005, 0xBF0A, 0x8B14, 0x5FCF],
                ModbusDataType::LReal,
                BIG_ENDIAN,
            ),
            &lreal("2.71828182846"),
            "interop holding registers 28 to 31",
        );
    }

    // covers: MB-TYPE-REAL, MB-TYPE-LREAL
    #[test]
    fn signed_zero_is_preserved() {
        for mode in MODES.map(|(_, mode)| mode) {
            let registers = encode(&PlcValue::F32(-0.0), ModbusDataType::Real, mode);
            assert_same(
                &decode(&registers, ModbusDataType::Real, mode),
                &PlcValue::F32(-0.0),
                "REAL -0.0",
            );
            assert!(
                !same(
                    &decode(&registers, ModbusDataType::Real, mode),
                    &PlcValue::F32(0.0)
                ),
                "REAL -0.0 is not +0.0"
            );
            let registers = encode(&PlcValue::F64(-0.0), ModbusDataType::LReal, mode);
            assert_same(
                &decode(&registers, ModbusDataType::LReal, mode),
                &PlcValue::F64(-0.0),
                "LREAL -0.0",
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values
    #[test]
    fn negative_values_round_trip_under_every_mode() {
        let cases = [
            (ModbusDataType::SInt, PlcValue::I8(-128)),
            (ModbusDataType::SInt, PlcValue::I8(-1)),
            (ModbusDataType::Int, PlcValue::I16(-2424)),
            (ModbusDataType::DInt, PlcValue::I32(-242_442_424)),
            (ModbusDataType::DInt, PlcValue::I32(i32::MIN)),
            (
                ModbusDataType::LInt,
                PlcValue::I64(-4_242_442_424_242_424_242),
            ),
            (ModbusDataType::Real, PlcValue::F32(-118.625)),
            (ModbusDataType::LReal, lreal("-2.71828182846")),
        ];
        for mode in MODES.map(|(_, mode)| mode) {
            for (data_type, value) in &cases {
                let registers = encode(value, *data_type, mode);
                assert_same(
                    &decode(&registers, *data_type, mode),
                    value,
                    &format!("{value:?} under {mode:?}"),
                );
            }
        }
    }

    // ---- single-register padding ----------------------------------------------------

    // covers: MB-TYPE-BYTE, MB-TYPE-SINT, MB-TYPE-USINT, MB-BYTEORDER-BE, MB-BYTEORDER-LE
    #[test]
    fn a_lone_eight_bit_value_is_the_low_byte_under_big_and_the_high_byte_under_little() {
        for data_type in [
            ModbusDataType::Byte,
            ModbusDataType::USInt,
            ModbusDataType::SInt,
        ] {
            for (name, mode) in MODES {
                let value = if data_type == ModbusDataType::SInt {
                    PlcValue::I8(-91)
                } else {
                    PlcValue::U8(0xA5)
                };
                let register = match mode.0 {
                    ByteOrder::BigEndian => 0x00A5,
                    ByteOrder::LittleEndian => 0xA500,
                };
                assert_eq!(
                    encode(&value, data_type, mode),
                    [register],
                    "{data_type:?} under {name}"
                );
                assert_same(&decode(&[register], data_type, mode), &value, name);
            }
        }
    }

    // covers: MB-TYPE-BOOL, MB-BYTEORDER-BE, MB-BYTEORDER-LE
    #[test]
    fn a_lone_bool_is_bit_0_under_big_and_bit_8_under_little() {
        for (name, mode) in MODES {
            let (set, clear) = match mode.0 {
                ByteOrder::BigEndian => (0x0001, 0x0000),
                ByteOrder::LittleEndian => (0x0100, 0x0000),
            };
            assert_eq!(
                encode(&PlcValue::Bool(true), ModbusDataType::Bool, mode),
                [set],
                "true under {name}"
            );
            assert_eq!(
                encode(&PlcValue::Bool(false), ModbusDataType::Bool, mode),
                [clear],
                "false under {name}"
            );
            assert_eq!(
                decode(&[set], ModbusDataType::Bool, mode),
                PlcValue::Bool(true)
            );
            assert_eq!(
                decode(&[clear], ModbusDataType::Bool, mode),
                PlcValue::Bool(false)
            );
        }
    }

    // covers: MB-TYPE-BOOL, MB-TYPE-BYTE
    #[test]
    fn padding_bits_are_ignored_when_decoding() {
        // BIG_ENDIAN: only bit 0 of the register is the BOOL; the byte is the low byte.
        assert_eq!(
            decode(&[0xFFFE], ModbusDataType::Bool, BIG_ENDIAN),
            PlcValue::Bool(false)
        );
        assert_eq!(
            decode(&[0x0002], ModbusDataType::Bool, BIG_ENDIAN),
            PlcValue::Bool(false)
        );
        assert_eq!(
            decode(&[0xFFFF], ModbusDataType::Bool, BIG_ENDIAN),
            PlcValue::Bool(true)
        );
        assert_eq!(
            decode(&[0x1234], ModbusDataType::Byte, BIG_ENDIAN),
            PlcValue::U8(0x34)
        );
        // LITTLE_ENDIAN: bit 8 is the BOOL, the high byte is the byte.
        assert_eq!(
            decode(&[0xFEFF], ModbusDataType::Bool, LITTLE_ENDIAN),
            PlcValue::Bool(false)
        );
        assert_eq!(
            decode(&[0x0101], ModbusDataType::Bool, LITTLE_ENDIAN),
            PlcValue::Bool(true)
        );
        assert_eq!(
            decode(&[0x1234], ModbusDataType::Byte, LITTLE_ENDIAN),
            PlcValue::U8(0x12)
        );
    }

    // ---- wrong register counts ------------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
    #[test]
    fn wrong_register_counts_are_rejected_with_the_raw_bytes_attached() {
        for data_type in ALL_TYPES {
            let size = usize::from(data_type.register_size());
            for count in 0_usize..=6 {
                let registers: Vec<u16> = (0_u16..).take(count).map(|n| 0x1100 | n).collect();
                let outcome = TypedDecoder::decode_registers(
                    &registers,
                    data_type,
                    ByteOrder::BigEndian,
                    WordOrder::HighWordFirst,
                    budget(64),
                );
                if count == size {
                    assert!(outcome.is_ok(), "{data_type:?} with {count} registers");
                    continue;
                }
                let error = outcome.expect_err("the wrong count is rejected");
                assert_eq!(
                    error.kind(),
                    ErrorKind::MalformedData,
                    "{data_type:?} with {count} registers"
                );
                let status = error.status().expect("a status");
                assert_eq!(status.protocol(), "modbus");
                assert_eq!(status.code(), 0);
                assert_eq!(status.name(), Some("decode"));
                let raw = status.raw().expect("raw bytes attached");
                assert_eq!(raw.len(), count.saturating_mul(2), "raw length");
                assert_eq!(raw, wire_of(&registers), "the bytes as received");
                let message = error.to_string();
                assert!(
                    message.contains(data_type.upstream_name()),
                    "the message names the type: {message}"
                );
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
    #[test]
    fn an_empty_register_slice_keeps_an_empty_raw_distinct_from_absent() {
        let error = failure(&[], ModbusDataType::Int, 8);
        let status = error.status().expect("a status");
        assert_eq!(status.raw(), Some(&[][..]), "empty raw, not absent");
    }

    // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
    #[test]
    fn raw_bytes_are_attached_only_within_the_budget() {
        let registers = [0x0102, 0x0304, 0x0506];
        let at_limit = failure(&registers, ModbusDataType::Real, 6);
        let status = at_limit.status().expect("a status");
        assert_eq!(status.raw(), Some(&[1, 2, 3, 4, 5, 6][..]), "6 bytes fit 6");

        let over = failure(&registers, ModbusDataType::Real, 5);
        assert_eq!(over.kind(), ErrorKind::MalformedData, "still the same kind");
        let status = over.status().expect("a status survives");
        assert_eq!(status.name(), Some("decode"));
        assert_eq!(status.code(), 0);
        assert_eq!(status.raw(), None, "no truncated copy: none at all");

        let one = failure(&registers, ModbusDataType::Real, 1);
        assert_eq!(
            one.status().and_then(|s| s.raw()),
            None,
            "budget of one byte"
        );
    }

    // ---- bit areas ------------------------------------------------------------------

    // covers: MB-TYPE-BOOL
    #[test]
    fn the_interop_coils_and_discrete_inputs_decode_bit_by_bit() {
        let map = interop_map();
        for run in [&map.coils, &map.discrete_inputs] {
            let packed = pack_bits_lsb_first(&run.values);
            assert_eq!(run.values.len(), 10, "{}: ten bits", run.row);
            for (index, want) in run.values.iter().enumerate() {
                let got =
                    TypedDecoder::decode_bits(&packed, index, ModbusDataType::Bool, budget(8))
                        .expect("decodes");
                assert_eq!(got, PlcValue::Bool(*want), "{} bit {index}", run.row);
            }
        }
    }

    // covers: MB-ADDR-BIT-TYPES
    #[test]
    fn a_non_bool_type_on_a_bit_area_is_rejected() {
        for data_type in ALL_TYPES {
            let outcome = TypedDecoder::decode_bits(&[0xFF], 0, data_type, budget(8));
            if data_type == ModbusDataType::Bool {
                assert_eq!(outcome.expect("BOOL decodes"), PlcValue::Bool(true));
                continue;
            }
            let error = outcome.expect_err("only BOOL is a bit");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{data_type:?}");
            let status = error.status().expect("a status");
            assert_eq!(status.name(), Some("decode"), "{data_type:?}");
            assert_eq!(status.raw(), None, "{data_type:?}: no register bytes here");
            assert!(
                error.to_string().contains(data_type.upstream_name()),
                "the message names the type"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
    #[test]
    fn a_bit_index_beyond_the_received_bytes_is_malformed_data_with_the_bytes_kept() {
        for (packed, index) in [
            (vec![0xAA_u8], 8_usize),
            (vec![0xAA, 0x55], 16),
            (vec![], 0),
            (vec![0xAA], usize::MAX),
        ] {
            let error = TypedDecoder::decode_bits(&packed, index, ModbusDataType::Bool, budget(8))
                .expect_err("out of range");
            assert_eq!(error.kind(), ErrorKind::MalformedData, "index {index}");
            let status = error.status().expect("a status");
            assert_eq!(status.name(), Some("decode"));
            assert_eq!(status.raw(), Some(packed.as_slice()), "index {index}");
        }
        // The last bit of the last byte is in range.
        assert_eq!(
            TypedDecoder::decode_bits(&[0x80], 7, ModbusDataType::Bool, budget(8))
                .expect("in range"),
            PlcValue::Bool(true)
        );
        // Over the budget the error keeps no raw bytes.
        let over = TypedDecoder::decode_bits(&[1, 2, 3], 24, ModbusDataType::Bool, budget(2))
            .expect_err("out of range");
        assert_eq!(over.status().and_then(|s| s.raw()), None);
    }

    // ---- encoding -------------------------------------------------------------------

    fn sample(data_type: ModbusDataType) -> PlcValue {
        match data_type {
            ModbusDataType::Bool => PlcValue::Bool(true),
            ModbusDataType::Byte | ModbusDataType::USInt => PlcValue::U8(7),
            ModbusDataType::Word | ModbusDataType::UInt => PlcValue::U16(7),
            ModbusDataType::DWord | ModbusDataType::UDInt => PlcValue::U32(7),
            ModbusDataType::LWord | ModbusDataType::ULInt => PlcValue::U64(7),
            ModbusDataType::SInt => PlcValue::I8(-7),
            ModbusDataType::Int => PlcValue::I16(-7),
            ModbusDataType::DInt => PlcValue::I32(-7),
            ModbusDataType::LInt => PlcValue::I64(-7),
            ModbusDataType::Real => PlcValue::F32(7.5),
            ModbusDataType::LReal => PlcValue::F64(7.5),
            _ => panic!("the 15 types of the alpha"),
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 values of the wrong type are refused, not converted
    #[test]
    fn encode_refuses_a_value_of_another_type() {
        let foreign = [
            PlcValue::Null,
            PlcValue::Char('x'),
            PlcValue::String("7".to_owned()),
            PlcValue::Duration(Duration::from_secs(7)),
        ];
        for data_type in ALL_TYPES {
            let own = sample(data_type);
            for other in ALL_TYPES {
                let value = sample(other);
                let outcome = TypedDecoder::encode_registers(
                    &value,
                    data_type,
                    ByteOrder::BigEndian,
                    WordOrder::HighWordFirst,
                );
                if value.value_type() == own.value_type() {
                    let registers = outcome.expect("the matching type encodes");
                    assert_eq!(
                        registers.len(),
                        usize::from(data_type.register_size()),
                        "{data_type:?}: register count"
                    );
                } else {
                    let error = outcome.expect_err("a value of another type is refused");
                    assert_eq!(
                        error.kind(),
                        ErrorKind::InvalidConfig,
                        "{value:?} as {data_type:?}"
                    );
                }
            }
            for value in &foreign {
                let outcome = TypedDecoder::encode_registers(
                    value,
                    data_type,
                    ByteOrder::BigEndian,
                    WordOrder::HighWordFirst,
                );
                assert!(outcome.is_err(), "{value:?} as {data_type:?}");
            }
        }
    }

    // covers: MB-TYPE-BOOL ... MB-TYPE-LREAL
    #[test]
    fn encode_writes_zero_padding_and_the_documented_registers() {
        for mode in MODES.map(|(_, mode)| mode) {
            for data_type in NARROW {
                let registers = encode(&sample(data_type), data_type, mode);
                let padding_clear = match mode.0 {
                    ByteOrder::BigEndian => registers.iter().all(|r| r & 0xFF00 == 0),
                    ByteOrder::LittleEndian => registers.iter().all(|r| r.trailing_zeros() >= 8),
                };
                assert!(
                    padding_clear,
                    "{data_type:?} under {mode:?}: {registers:04X?}"
                );
            }
        }
        assert_eq!(
            encode(
                &PlcValue::U32(4_242_442_424),
                ModbusDataType::DWord,
                BIG_ENDIAN
            ),
            [0xFCDE, 0x88B8]
        );
        assert_eq!(
            encode(&real("3.141593"), ModbusDataType::Real, LITTLE_ENDIAN),
            [0xDC0F, 0x4940]
        );
    }

    // ---- the interop register map --------------------------------------------------

    fn interop_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the register map loads")
    }

    /// The value a register-map row names, from its `value` text.
    fn named_value(data_type: ModbusDataType, text: &str) -> PlcValue {
        match data_type {
            ModbusDataType::Bool => PlcValue::Bool(text == "true"),
            ModbusDataType::Byte | ModbusDataType::USInt => PlcValue::U8(text.parse().expect("u8")),
            ModbusDataType::Word | ModbusDataType::UInt => {
                PlcValue::U16(text.parse().expect("u16"))
            }
            ModbusDataType::DWord | ModbusDataType::UDInt => {
                PlcValue::U32(text.parse().expect("u32"))
            }
            ModbusDataType::LWord | ModbusDataType::ULInt => {
                PlcValue::U64(text.parse().expect("u64"))
            }
            ModbusDataType::SInt => PlcValue::I8(text.parse().expect("i8")),
            ModbusDataType::Int => PlcValue::I16(text.parse().expect("i16")),
            ModbusDataType::DInt => PlcValue::I32(text.parse().expect("i32")),
            ModbusDataType::LInt => PlcValue::I64(text.parse().expect("i64")),
            ModbusDataType::Real => PlcValue::F32(text.parse().expect("f32")),
            ModbusDataType::LReal => PlcValue::F64(text.parse().expect("f64")),
            _ => panic!("the 15 types of the alpha"),
        }
    }

    // covers: MB-TYPE-BOOL ... MB-TYPE-LREAL, MB-BYTEORDER-BE, MB-BYTEORDER-LE
    #[test]
    fn register_map_decodes_every_entry_to_the_row_value() {
        let map = interop_map();
        let mut types = BTreeSet::new();
        let mut checked = 0_usize;
        for entry in map
            .holding_registers
            .iter()
            .chain(&map.holding_extension)
            .chain(&map.input_registers)
        {
            let data_type = type_named(&entry.data_type);
            let mode = entry.byte_order.as_deref().map_or(BIG_ENDIAN, mode_named);
            let want = named_value(data_type, &entry.value);
            let context = format!("{} at {} ({})", entry.row, entry.address, entry.data_type);
            assert_same(&decode(&entry.words, data_type, mode), &want, &context);
            let encoded = encode(&want, data_type, mode);
            if NARROW.contains(&data_type) {
                // The map's SINT -42 is the sign-extended register 0xFFD6; the encoder pads
                // with zero, so only the decoded value has to agree.
                assert_same(&decode(&encoded, data_type, mode), &want, &context);
            } else {
                assert_eq!(
                    encoded, entry.words,
                    "{context}: encodes back to the words of the map"
                );
            }
            if map.holding_registers.contains(entry) {
                types.insert(entry.data_type.clone());
            }
            checked = checked.saturating_add(1);
        }
        assert_eq!(
            types.len(),
            15,
            "all fifteen types on the holding registers"
        );
        assert_eq!(
            checked, 21,
            "15 holding + 1 little-endian + 5 input entries"
        );
        let little = map
            .holding_extension
            .iter()
            .find(|e| e.byte_order.as_deref() == Some("LITTLE_ENDIAN"))
            .expect("the little-endian region");
        assert_eq!(little.address, 300, "holding registers 300 and 301");
    }

    // ---- properties -----------------------------------------------------------------

    fn mode_strategy() -> impl Strategy<Value = Mode> {
        proptest::sample::select(MODES.map(|(_, mode)| mode).to_vec())
    }

    fn integer_case() -> impl Strategy<Value = (ModbusDataType, PlcValue)> {
        prop_oneof![
            any::<bool>().prop_map(|v| (ModbusDataType::Bool, PlcValue::Bool(v))),
            any::<u8>().prop_map(|v| (ModbusDataType::Byte, PlcValue::U8(v))),
            any::<u8>().prop_map(|v| (ModbusDataType::USInt, PlcValue::U8(v))),
            any::<i8>().prop_map(|v| (ModbusDataType::SInt, PlcValue::I8(v))),
            any::<u16>().prop_map(|v| (ModbusDataType::Word, PlcValue::U16(v))),
            any::<u16>().prop_map(|v| (ModbusDataType::UInt, PlcValue::U16(v))),
            any::<i16>().prop_map(|v| (ModbusDataType::Int, PlcValue::I16(v))),
            any::<u32>().prop_map(|v| (ModbusDataType::DWord, PlcValue::U32(v))),
            any::<u32>().prop_map(|v| (ModbusDataType::UDInt, PlcValue::U32(v))),
            any::<i32>().prop_map(|v| (ModbusDataType::DInt, PlcValue::I32(v))),
            any::<u64>().prop_map(|v| (ModbusDataType::LWord, PlcValue::U64(v))),
            any::<u64>().prop_map(|v| (ModbusDataType::ULInt, PlcValue::U64(v))),
            any::<i64>().prop_map(|v| (ModbusDataType::LInt, PlcValue::I64(v))),
        ]
    }

    proptest! {
        // covers: MB-TYPE-BOOL ... MB-TYPE-ULINT, MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
        // covers: SPEC mandatory-failure-cases / F7 integer boundaries
        #[test]
        fn prop_encode_decode_exact_integers(
            (data_type, value) in integer_case(),
            mode in mode_strategy(),
        ) {
            let registers = TypedDecoder::encode_registers(&value, data_type, mode.0, mode.1)
                .expect("a matching value encodes");
            prop_assert_eq!(registers.len(), usize::from(data_type.register_size()));
            let back = TypedDecoder::decode_registers(&registers, data_type, mode.0, mode.1, budget(16))
                .expect("what was encoded decodes");
            prop_assert_eq!(back, value);
        }

        // covers: MB-TYPE-REAL, MB-TYPE-LREAL, MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-BYTEORDER-BE-SWAP, MB-BYTEORDER-LE-SWAP
        // covers: SPEC mandatory-failure-cases / F7 NaN and infinity preserved for the sensors to judge
        #[test]
        fn prop_encode_decode_floats_bitwise(
            single in any::<u32>(),
            double in any::<u64>(),
            mode in mode_strategy(),
        ) {
            let real = PlcValue::F32(f32::from_bits(single));
            let registers = TypedDecoder::encode_registers(&real, ModbusDataType::Real, mode.0, mode.1)
                .expect("encodes");
            let back = TypedDecoder::decode_registers(&registers, ModbusDataType::Real, mode.0, mode.1, budget(16))
                .expect("decodes");
            prop_assert_eq!(single_bits(&back), Some(single));
            let lreal = PlcValue::F64(f64::from_bits(double));
            let registers = TypedDecoder::encode_registers(&lreal, ModbusDataType::LReal, mode.0, mode.1)
                .expect("encodes");
            let back = TypedDecoder::decode_registers(&registers, ModbusDataType::LReal, mode.0, mode.1, budget(16))
                .expect("decodes");
            prop_assert_eq!(double_bits(&back), Some(double));
        }

        // covers: MB-TYPE-BOOL ... MB-TYPE-LREAL
        #[test]
        fn prop_decode_then_encode_normalises_only_the_padding(
            data_type in proptest::sample::select(ALL_TYPES.to_vec()),
            mode in mode_strategy(),
            seed in proptest::collection::vec(any::<u16>(), 4),
        ) {
            let size = usize::from(data_type.register_size());
            let registers: Vec<u16> = seed.iter().copied().take(size).collect();
            let value = TypedDecoder::decode_registers(&registers, data_type, mode.0, mode.1, budget(16))
                .expect("the right count always decodes");
            let again = TypedDecoder::encode_registers(&value, data_type, mode.0, mode.1)
                .expect("a decoded value encodes");
            if !NARROW.contains(&data_type) {
                prop_assert_eq!(&again, &registers, "full width values re-encode exactly");
            }
            let twice = TypedDecoder::decode_registers(&again, data_type, mode.0, mode.1, budget(16))
                .expect("decodes");
            prop_assert!(same(&twice, &value), "{:?} then {:?}", value, twice);
        }

        // covers: SPEC mandatory-failure-cases / F7 odd or wrong register counts rejected with raw bytes
        #[test]
        fn prop_register_count_decides_acceptance_and_errors_keep_the_bytes(
            data_type in proptest::sample::select(ALL_TYPES.to_vec()),
            registers in proptest::collection::vec(any::<u16>(), 0..12),
            limit in 1_usize..40,
        ) {
            let outcome = TypedDecoder::decode_registers(
                &registers,
                data_type,
                ByteOrder::BigEndian,
                WordOrder::HighWordFirst,
                budget(limit),
            );
            let right = registers.len() == usize::from(data_type.register_size());
            prop_assert_eq!(outcome.is_ok(), right);
            if let Err(error) = outcome {
                prop_assert_eq!(error.kind(), ErrorKind::MalformedData);
                let raw = error.status().and_then(|s| s.raw());
                let wire = wire_of(&registers);
                if wire.len() <= limit {
                    prop_assert_eq!(raw, Some(wire.as_slice()));
                } else {
                    prop_assert_eq!(raw, None);
                }
            }
        }
    }
}
