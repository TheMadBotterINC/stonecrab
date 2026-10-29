//! Golden vectors of the PDU codec (coding standards section 13.6): `fixtures/modbus/tcp`.
//!
//! The expected files are written by hand from the Modbus Application Protocol
//! Specification V1.1b, never produced by this decoder. Decode and encode are asserted
//! separately and each against a PDU built from the expected file, so a decoder bug and an
//! encoder bug cannot cancel out. Every vector is PDU bytes only, without an MBAP header; the
//! whole-ADU vectors (`tcp/adu_*`) belong to `tests/framing.rs`, except that the PDU of each
//! upstream ADU (`tcp/adu_upstream-*`) is read here, from the bytes after the header and the
//! `pdu` object of its expected file.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use stonecrab::codec::{
        BoundedReader, BoundedWriter, ParseError, coil_byte_count, unpack_bits_lsb_first,
    };
    use stonecrab::modbus::{
        ExceptionCode, FunctionCode, MAX_PDU_BYTES, MbapHeader, ModbusArea, ModbusPdu,
        decode_request, decode_response, encode_request, encode_response,
    };
    use stonecrab::testkit::{GoldenVector, VectorSource, load_dir};

    /// The vector directory (the `PROVENANCE.toml` is one level up, at `fixtures/modbus`).
    fn vectors() -> Vec<GoldenVector> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        load_dir(&dir)
            .expect("fixtures/modbus loads: every file listed, every listed vector present")
            .into_iter()
            .filter_map(pdu_of)
            .collect()
    }

    /// The vector as PDU bytes, or `None` when it is not one. The adu_* vectors are
    /// framing-level ADUs (tests/framing.rs), except that an upstream ADU is seen here as
    /// the PDU after its header with the `pdu` object of its expected file; the typed_*
    /// vectors hold register bytes for the typed decoder (tests/decode.rs), not PDUs.
    fn pdu_of(mut vector: GoldenVector) -> Option<GoldenVector> {
        if vector.name.starts_with("tcp/adu_upstream-") {
            vector.bytes = vector.bytes.get(MbapHeader::ENCODED_LEN..)?.to_vec();
            vector.expected = vector.expected.as_ref()?.get("pdu").cloned();
            return Some(vector);
        }
        (vector.name.starts_with("tcp/")
            && !vector.name.starts_with("tcp/adu_")
            && !vector.name.starts_with("tcp/typed_"))
        .then_some(vector)
    }

    fn number(vector: &GoldenVector, key: &str) -> Option<u64> {
        vector.expected.as_ref()?.get(key)?.as_u64()
    }

    fn text<'a>(vector: &'a GoldenVector, key: &str) -> Option<&'a str> {
        vector.expected.as_ref()?.get(key)?.as_str()
    }

    fn small(vector: &GoldenVector, key: &str) -> u16 {
        let value = number(vector, key)
            .unwrap_or_else(|| panic!("{}: expected.json has no number `{key}`", vector.name));
        u16::try_from(value).unwrap_or_else(|_| panic!("{}: `{key}` is not a u16", vector.name))
    }

    fn list(vector: &GoldenVector, key: &str) -> Option<Vec<u64>> {
        let items = vector.expected.as_ref()?.get(key)?.as_array()?;
        Some(
            items
                .iter()
                .map(|item| item.as_u64().expect("a number in the list"))
                .collect(),
        )
    }

    fn function(vector: &GoldenVector) -> FunctionCode {
        FunctionCode::from_u8(u8::try_from(small(vector, "function")).expect("a function byte"))
    }

    fn bytes_of(values: &[u64]) -> Vec<u8> {
        values
            .iter()
            .map(|value| u8::try_from(*value).expect("a byte"))
            .collect()
    }

    fn registers_of(vector: &GoldenVector) -> Vec<u16> {
        list(vector, "registers")
            .expect("a register list")
            .iter()
            .map(|value| u16::try_from(*value).expect("a register"))
            .collect()
    }

    fn is_request(vector: &GoldenVector) -> bool {
        match text(vector, "direction") {
            Some("request") => true,
            Some("response") => false,
            other => panic!("{}: direction is {other:?}", vector.name),
        }
    }

    /// The read request the expected file describes.
    fn read_request(vector: &GoldenVector, function: FunctionCode) -> ModbusPdu {
        let (start, quantity) = (small(vector, "start"), small(vector, "quantity"));
        match function.as_u8() {
            1 => ModbusPdu::ReadCoils { start, quantity },
            2 => ModbusPdu::ReadDiscreteInputs { start, quantity },
            3 => ModbusPdu::ReadHoldingRegisters { start, quantity },
            4 => ModbusPdu::ReadInputRegisters { start, quantity },
            other => panic!("{}: read function {other}", vector.name),
        }
    }

    /// The PDU the expected file describes, built without the decoder.
    fn expected_pdu(vector: &GoldenVector) -> ModbusPdu {
        let function = function(vector);
        if is_request(vector) {
            return match function.as_u8() {
                1..=4 => read_request(vector, function),
                6 => ModbusPdu::WriteSingleRegister {
                    address: small(vector, "address"),
                    value: small(vector, "value"),
                },
                15 => ModbusPdu::WriteMultipleCoils {
                    start: small(vector, "start"),
                    quantity: small(vector, "quantity"),
                    bytes: bytes_of(&list(vector, "bytes").expect("a coil write lists its bytes")),
                },
                16 => ModbusPdu::WriteMultipleRegisters {
                    start: small(vector, "start"),
                    registers: registers_of(vector),
                },
                other => panic!("{}: request function {other}", vector.name),
            };
        }
        if let Some(code) = number(vector, "exception") {
            let code = ExceptionCode::from_u8(u8::try_from(code).expect("a code byte"));
            return ModbusPdu::Exception { function, code };
        }
        match function.as_u8() {
            6 => {
                return ModbusPdu::WriteSingleRegisterResponse {
                    address: small(vector, "address"),
                    value: small(vector, "value"),
                };
            }
            15 | 16 => {
                return ModbusPdu::WriteMultipleResponse {
                    function,
                    start: small(vector, "start"),
                    quantity: small(vector, "quantity"),
                };
            }
            _ => {}
        }
        if list(vector, "registers").is_some() {
            return ModbusPdu::ReadRegistersResponse {
                function,
                registers: registers_of(vector),
            };
        }
        let bytes = bytes_of(&list(vector, "bytes").expect("a bit response lists its bytes"));
        ModbusPdu::ReadBitsResponse { function, bytes }
    }

    /// Decodes the vector; a response that lists the `request` it answers must also pass
    /// the reply check against that request, which is where an echo mismatch fails.
    fn decode(vector: &GoldenVector) -> Result<ModbusPdu, ParseError> {
        let mut reader = BoundedReader::new(&vector.bytes);
        if is_request(vector) {
            return decode_request(&mut reader);
        }
        let reply = decode_response(&mut reader, function(vector))?;
        if let Some(sent) = list(vector, "request") {
            let sent = bytes_of(&sent);
            let request = decode_request(&mut BoundedReader::new(&sent))
                .unwrap_or_else(|error| panic!("{}: the request decodes: {error}", vector.name));
            reply.check_reply_to(&request)?;
        }
        Ok(reply)
    }

    fn accepted() -> Vec<GoldenVector> {
        vectors().into_iter().filter(|v| !v.must_fail).collect()
    }

    fn rejected() -> Vec<GoldenVector> {
        vectors().into_iter().filter(|v| v.must_fail).collect()
    }

    /// The name of a `ParseError` variant and its parameters as text.
    fn describe(error: &ParseError) -> (&'static str, Vec<(&'static str, String)>) {
        if let ParseError::Truncated { needed, available } = error {
            let params = vec![
                ("needed", needed.to_string()),
                ("available", available.to_string()),
            ];
            return ("Truncated", params);
        }
        if let ParseError::OversizedLength { length, max } = error {
            let params = vec![("length", length.to_string()), ("max", max.to_string())];
            return ("OversizedLength", params);
        }
        if let ParseError::InvalidDiscriminator { field, value } = error {
            let params = vec![("field", (*field).to_owned()), ("value", value.to_string())];
            return ("InvalidDiscriminator", params);
        }
        if let ParseError::TrailingBytes { remaining } = error {
            return ("TrailingBytes", vec![("remaining", remaining.to_string())]);
        }
        ("other", Vec::new())
    }

    #[test]
    fn the_tcp_directory_is_loaded_with_provenance() {
        let all = vectors();
        assert!(all.len() >= 50, "{} vectors loaded", all.len());
        for vector in &all {
            assert!(
                matches!(
                    vector.source,
                    VectorSource::ProtocolSpec
                        | VectorSource::Adversarial
                        | VectorSource::UpstreamFixture
                ),
                "{}: no vector is captured from a device yet",
                vector.name
            );
            assert!(
                vector.expected.is_some(),
                "{}: has an expected file",
                vector.name
            );
            assert!(
                vector.bytes.len() <= MAX_PDU_BYTES.saturating_add(1),
                "{}: {} bytes",
                vector.name,
                vector.bytes.len()
            );
            let accepted = !vector.must_fail;
            assert_eq!(
                accepted,
                text(vector, "error").is_none(),
                "{}: only a must_fail vector names an error",
                vector.name
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    // Exit criterion of S21: every enabled variant has a valid and a negative vector.
    #[test]
    fn every_function_has_a_valid_and_a_negative_vector() {
        let all = vectors();
        for function in [1_u64, 2, 3, 4, 6, 15, 16] {
            let of_function = |direction: &str, must_fail: bool| {
                all.iter().any(|vector| {
                    text(vector, "direction") == Some(direction)
                        && vector.must_fail == must_fail
                        && number(vector, "function").map_or_else(
                            || vector.bytes.first().map(|b| u64::from(*b)) == Some(function),
                            |f| f == function,
                        )
                })
            };
            assert!(
                of_function("request", false),
                "FC{function}: a valid request"
            );
            assert!(
                of_function("response", false),
                "FC{function}: a valid response"
            );
            assert!(
                of_function("response", true),
                "FC{function}: a negative response"
            );
            let exception = all.iter().any(|vector| {
                number(vector, "exception").is_some()
                    && number(vector, "function") == Some(function)
            });
            assert!(exception, "FC{function}: a valid exception response");
        }
        for function in [1_u8, 2, 3, 4, 6, 15, 16] {
            let negative_request = all.iter().any(|vector| {
                vector.must_fail
                    && text(vector, "direction") == Some("request")
                    && vector.bytes.first() == Some(&function)
            });
            assert!(negative_request, "FC{function}: a negative request");
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    #[test]
    fn the_unsupported_write_and_file_record_functions_have_vectors() {
        let names: BTreeSet<String> = rejected().into_iter().map(|v| v.name).collect();
        for name in [
            "tcp/request-write-single-coil",
            "tcp/request-read-file-record",
            "tcp/response-read-file-record",
            "tcp/request-function-zero",
            "tcp/response-function-zero",
            "tcp/adu_upstream-read-extended-registers-request-split-file-record",
            "tcp/adu_upstream-read-extended-registers-response-split-file-record",
            "tcp/adu_upstream-write-extended-registers-request-file-record",
            "tcp/adu_upstream-write-extended-registers-request-split-file-record",
        ] {
            assert!(names.contains(name), "{name} is listed");
        }
    }

    #[test]
    fn the_upstream_function_04_pair_decodes_and_the_file_record_frames_are_refused() {
        let all = vectors();
        // The driver-suite exchanges (S04b) are upstream vectors too; they are not these six.
        let upstream: Vec<&GoldenVector> = all
            .iter()
            .filter(|v| {
                v.source == VectorSource::UpstreamFixture
                    && !v.name.starts_with("tcp/adu_upstream-driver-suite-")
                    && !v.name.starts_with("tcp/adu_upstream-all-datatypes-")
            })
            .collect();
        assert_eq!(upstream.len(), 6, "the six frames of the parser suite");
        let named = |name: &str| {
            upstream
                .iter()
                .find(|v| v.name == format!("tcp/adu_upstream-{name}"))
                .unwrap_or_else(|| panic!("adu_upstream-{name} is listed"))
        };
        let request = named("read-input-registers-request");
        assert_eq!(
            decode(request),
            Ok(ModbusPdu::ReadInputRegisters {
                start: 0x08D2,
                quantity: 2
            })
        );
        let response = named("read-input-registers-response");
        let Ok(ModbusPdu::ReadRegistersResponse {
            function,
            registers,
        }) = decode(response)
        else {
            panic!("the function 0x04 response decodes to registers");
        };
        assert_eq!(function, FunctionCode::ReadInputRegisters);
        assert_eq!(registers.len(), 99, "byte count 198");
        for (index, value) in [(10, 1), (40, 4), (48, 475), (50, 470), (52, 19000)] {
            assert_eq!(registers.get(index), Some(&value), "register {index}");
        }
        for (name, function) in [
            ("read-extended-registers-request-split-file-record", 0x14),
            ("read-extended-registers-response-split-file-record", 0x14),
            ("write-extended-registers-request-file-record", 0x15),
            ("write-extended-registers-request-split-file-record", 0x15),
        ] {
            let vector = named(name);
            assert!(vector.must_fail, "{name}");
            assert_eq!(vector.bytes.first(), Some(&function), "{name}: function");
            assert!(
                matches!(
                    decode(vector),
                    Err(ParseError::InvalidDiscriminator { value, .. }) if value == u64::from(function)
                ),
                "{name}: the PDU codec refuses function {function:#04x}"
            );
        }
    }

    #[test]
    fn decode_compare_every_accepted_vector_matches_its_expected_file() {
        let all = accepted();
        assert!(all.len() >= 20, "{} accepted vectors", all.len());
        for vector in &all {
            let decoded = decode(vector);
            assert_eq!(
                decoded,
                Ok(expected_pdu(vector)),
                "{}: decoded PDU against expected.json",
                vector.name
            );
        }
    }

    #[test]
    fn decode_compare_bit_responses_unpack_to_the_hand_written_bits() {
        let mut checked = 0;
        for vector in accepted() {
            let Some(bytes) = list(&vector, "bytes") else {
                continue;
            };
            if vector
                .expected
                .as_ref()
                .and_then(|e| e.get("bits"))
                .is_none()
            {
                continue;
            }
            let quantity = usize::from(small(&vector, "quantity"));
            let Ok(ModbusPdu::ReadBitsResponse { bytes: decoded, .. }) = decode(&vector) else {
                panic!("{}: a bit response", vector.name);
            };
            assert_eq!(decoded, bytes_of(&bytes), "{}: data bytes", vector.name);
            assert_eq!(
                decoded.len(),
                coil_byte_count(quantity),
                "{}: byte count",
                vector.name
            );
            let unpacked = unpack_bits_lsb_first(&decoded, quantity).expect("enough bits");
            let want: Vec<bool> = vector
                .expected
                .as_ref()
                .and_then(|e| e.get("bits"))
                .and_then(|b| b.as_array())
                .expect("a bits list")
                .iter()
                .map(|bit| bit.as_bool().expect("a boolean"))
                .collect();
            assert_eq!(
                want.len(),
                quantity,
                "{}: one expected bit per coil",
                vector.name
            );
            assert_eq!(
                unpacked, want,
                "{}: bits, least significant bit first",
                vector.name
            );
            checked += 1;
        }
        assert_eq!(
            checked, 2,
            "the section 6.1 and 6.2 responses carry expected bits"
        );
    }

    #[test]
    fn re_encode_compare_every_accepted_expected_file_encodes_to_the_vector_bytes() {
        for vector in accepted() {
            let pdu = expected_pdu(&vector);
            let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
            let written = if is_request(&vector) {
                encode_request(&pdu, &mut writer)
            } else {
                encode_response(&pdu, &mut writer)
            };
            assert_eq!(written, Ok(()), "{}: encodes", vector.name);
            assert_eq!(
                writer.as_slice(),
                vector.bytes.as_slice(),
                "{}: encoded bytes against the .hex",
                vector.name
            );
        }
    }

    #[test]
    fn decode_then_encode_is_the_identity_on_every_accepted_vector() {
        for vector in accepted() {
            let pdu = decode(&vector).expect("accepted vector decodes");
            let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
            let written = if is_request(&vector) {
                encode_request(&pdu, &mut writer)
            } else {
                encode_response(&pdu, &mut writer)
            };
            assert_eq!(written, Ok(()), "{}: re-encodes", vector.name);
            assert_eq!(
                writer.as_slice(),
                vector.bytes.as_slice(),
                "{}: bytes",
                vector.name
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths
    #[test]
    fn every_must_fail_vector_fails_with_the_named_variant_and_parameters() {
        let all = rejected();
        assert!(all.len() >= 30, "{} must_fail vectors", all.len());
        for vector in &all {
            let error = decode(vector).expect_err(&format!("{}: must fail", vector.name));
            let (variant, params) = describe(&error);
            assert_eq!(
                Some(variant),
                text(vector, "error"),
                "{}: variant",
                vector.name
            );
            let note = vector.note.as_deref().unwrap_or_default();
            assert!(
                note.contains(&format!("ParseError::{variant}")),
                "{}: the provenance note names the variant: {note}",
                vector.name
            );
            let object = vector
                .expected
                .as_ref()
                .and_then(|e| e.as_object())
                .expect("expected.json is an object");
            let mut named = BTreeSet::new();
            for (key, value) in object {
                if matches!(key.as_str(), "direction" | "function" | "error" | "request") {
                    continue;
                }
                named.insert(key.as_str());
                let want = value
                    .as_u64()
                    .map(|n| n.to_string())
                    .or_else(|| value.as_str().map(str::to_owned))
                    .expect("a number or a string");
                let got = params
                    .iter()
                    .find(|(name, _)| name == key)
                    .map(|(_, v)| v.as_str());
                assert_eq!(got, Some(want.as_str()), "{}: `{key}`", vector.name);
            }
            let actual: BTreeSet<&str> = params.iter().map(|(name, _)| *name).collect();
            assert_eq!(
                actual, named,
                "{}: every parameter of the error is pinned",
                vector.name
            );
        }
    }

    #[test]
    fn spec_vectors_cite_a_section_and_adversarial_vectors_say_why() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let provenance = std::fs::read_to_string(dir.join("PROVENANCE.toml")).expect("readable");
        let adu_entries = provenance
            .lines()
            .filter(|l| l.starts_with("name = \"tcp/adu_"))
            .count();
        let typed = provenance
            .lines()
            .filter(|l| l.starts_with("name = \"tcp/typed_"))
            .count();
        let entries = provenance
            .lines()
            .filter(|l| *l == "[[vector]]")
            .count()
            .saturating_sub(adu_entries)
            .saturating_sub(typed);
        let upstream = vectors()
            .iter()
            .filter(|v| v.source == VectorSource::UpstreamFixture)
            .count();
        assert_eq!(
            entries,
            vectors().len() - upstream,
            "one [[vector]] entry per tcp vector"
        );
        // The upstream ADUs are seen here only through their PDU; their files hold the header.
        let own = vectors()
            .into_iter()
            .filter(|v| v.source != VectorSource::UpstreamFixture);
        for vector in own {
            let hex = std::fs::read_to_string(dir.join(format!("{}.hex", vector.name)))
                .expect("readable");
            assert!(
                hex.lines()
                    .next()
                    .is_some_and(|l| l.contains("no MBAP header")),
                "{}: the first comment says the file holds PDU bytes only",
                vector.name
            );
            if vector.source == VectorSource::ProtocolSpec {
                assert!(
                    hex.contains("Modbus Application Protocol V1.1b section"),
                    "{}: cites the specification section",
                    vector.name
                );
            }
        }
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn write_request_picks_the_pdu_of_every_accepted_write_request_vector() {
        let mut checked = 0;
        for vector in accepted()
            .iter()
            .filter(|v| is_request(v) && matches!(function(v).as_u8(), 6 | 15 | 16))
        {
            let pdu = expected_pdu(vector);
            let picked = match function(vector).as_u8() {
                15 => {
                    let bytes =
                        bytes_of(&list(vector, "bytes").expect("a coil write lists its bytes"));
                    let (start, quantity) = (small(vector, "start"), small(vector, "quantity"));
                    ModbusPdu::write_request(ModbusArea::Coil, start, quantity, &bytes)
                }
                6 => ModbusPdu::write_request(
                    ModbusArea::HoldingRegister,
                    small(vector, "address"),
                    1,
                    &small(vector, "value").to_be_bytes(),
                ),
                16 => {
                    let registers = registers_of(vector);
                    let payload: Vec<u8> = registers.iter().flat_map(|r| r.to_be_bytes()).collect();
                    let quantity = u16::try_from(registers.len()).expect("at most 123");
                    let start = small(vector, "start");
                    ModbusPdu::write_request(ModbusArea::HoldingRegister, start, quantity, &payload)
                }
                other => panic!("{}: not a write request: {other}", vector.name),
            };
            assert_eq!(picked, Ok(pdu), "{}: the selected request", vector.name);
            checked += 1;
        }
        assert_eq!(
            checked, 8,
            "two function 06, three 15 and three 16 requests"
        );
    }
}
