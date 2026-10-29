//! MBAP framing: the golden ADU vectors of `fixtures/modbus/tcp/adu_*`, the `CODING_STANDARDS`
//! section 13.7 list for `MbapFramer::decode`, the transaction counter and the framer over
//! the in-memory transport double.
//!
//! Most vectors are written by hand from the Modbus Messaging on TCP/IP Implementation
//! Guide V1.0b and the V1.1b examples (section 13.6), never produced by this framer; the
//! six `adu_upstream-*` vectors are the frames of the Apache PLC4X parser suite.
//! Decode and encode are asserted separately, each against the ADUs the expected file
//! lists, so a framer bug and an encoder bug cannot cancel out. Timing is on the fake clock
//! only: nothing here sleeps.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;
    use std::time::Duration;

    use bytes::{Bytes, BytesMut};
    use stonecrab::modbus::{
        MAX_PDU_BYTES, MbapFramer, MbapHeader, ModbusAdu, TransactionId, UnitId,
    };
    use stonecrab::testkit::{
        FakeClock, FakeTransport, GoldenVector, Script, Step, VectorSource, block_on_with, load_dir,
    };
    use stonecrab::transport::{FrameError, FramedTransport, Framer};
    use stonecrab::{Clock, Deadline, ErrorKind, ResourceLimits};

    /// The ADU vectors (the `PROVENANCE.toml` is one level up, at `fixtures/modbus`).
    fn vectors() -> Vec<GoldenVector> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        load_dir(&dir)
            .expect("fixtures/modbus loads: every file listed, every listed vector present")
            .into_iter()
            .filter(|vector| vector.name.starts_with("tcp/adu_"))
            .collect()
    }

    fn vector(name: &str) -> GoldenVector {
        vectors()
            .into_iter()
            .find(|vector| vector.name == format!("tcp/adu_{name}"))
            .unwrap_or_else(|| panic!("vector adu_{name} is listed"))
    }

    fn text<'a>(vector: &'a GoldenVector, key: &str) -> Option<&'a str> {
        vector.expected.as_ref()?.get(key)?.as_str()
    }

    fn number(value: &serde_json::Value, key: &str) -> u64 {
        value
            .get(key)
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("expected.json has no number `{key}`"))
    }

    fn hex_bytes(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|pair| u8::from_str_radix(pair, 16).expect("a hex byte"))
            .collect()
    }

    /// One ADU as the expected file spells it.
    struct Expected {
        transaction_id: u16,
        protocol_id: u16,
        length: u16,
        unit_id: u8,
        pdu: Vec<u8>,
    }

    fn small<T: TryFrom<u64>>(value: &serde_json::Value, key: &str) -> T {
        T::try_from(number(value, key)).unwrap_or_else(|_| panic!("`{key}` is out of range"))
    }

    fn expected_adus(vector: &GoldenVector) -> Vec<Expected> {
        let list = vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get("adus"))
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("{}: an accepted vector lists `adus`", vector.name));
        list.iter()
            .map(|adu| Expected {
                transaction_id: small(adu, "transaction_id"),
                protocol_id: small(adu, "protocol_id"),
                length: small(adu, "length"),
                unit_id: small(adu, "unit_id"),
                pdu: hex_bytes(
                    adu.get("pdu")
                        .and_then(serde_json::Value::as_str)
                        .expect("a pdu string"),
                ),
            })
            .collect()
    }

    fn leftover(vector: &GoldenVector) -> usize {
        let value = vector
            .expected
            .as_ref()
            .and_then(|expected| expected.get("leftover"))
            .expect("an accepted vector states `leftover`");
        usize::try_from(value.as_u64().expect("a number")).expect("a size")
    }

    /// The framer cuts the ADUs the expected file lists. (`must_fail` is not the test: a
    /// vector can be framed and still fail at the PDU layer.)
    fn frames(vector: &GoldenVector) -> bool {
        vector
            .expected
            .as_ref()
            .is_some_and(|expected| expected.get("adus").is_some())
    }

    fn accepted() -> Vec<GoldenVector> {
        vectors().into_iter().filter(frames).collect()
    }

    fn rejected() -> Vec<GoldenVector> {
        vectors().into_iter().filter(|v| !frames(v)).collect()
    }

    fn upto(bytes: &[u8], end: usize) -> &[u8] {
        bytes.get(..end).expect("end is within the bytes")
    }

    fn from(bytes: &[u8], start: usize) -> &[u8] {
        bytes.get(start..).expect("start is within the bytes")
    }

    fn between(bytes: &[u8], start: usize, end: usize) -> &[u8] {
        bytes.get(start..end).expect("range is within the bytes")
    }

    fn buffer(bytes: &[u8]) -> BytesMut {
        BytesMut::from(bytes)
    }

    fn decode(bytes: &[u8]) -> (Result<Option<ModbusAdu>, FrameError>, BytesMut) {
        let mut buffered = buffer(bytes);
        let result = MbapFramer::new().decode(&mut buffered);
        (result, buffered)
    }

    /// Every ADU the framer cuts from `bytes`, then the bytes left over.
    fn drain(bytes: &[u8]) -> (Vec<ModbusAdu>, usize) {
        let mut framer = MbapFramer::new();
        let mut buffered = buffer(bytes);
        let mut adus = Vec::new();
        while let Some(adu) = framer.decode(&mut buffered).expect("an accepted vector") {
            adus.push(adu);
        }
        (adus, buffered.len())
    }

    /// An ADU wire image: header fields and PDU, written out by the test.
    fn wire(
        transaction_id: u16,
        protocol_id: u16,
        length: u16,
        unit_id: u8,
        pdu: &[u8],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&transaction_id.to_be_bytes());
        bytes.extend_from_slice(&protocol_id.to_be_bytes());
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes.push(unit_id);
        bytes.extend_from_slice(pdu);
        bytes
    }

    /// The V1.1b section 6.1 request PDU: read coils 20 to 38.
    const READ_COILS: [u8; 5] = [0x01, 0x00, 0x13, 0x00, 0x13];

    fn read_coils_adu() -> Vec<u8> {
        wire(1, 0, 6, 0x11, &READ_COILS)
    }

    #[test]
    fn the_adu_vectors_are_loaded_with_provenance() {
        let all = vectors();
        assert!(all.len() >= 25, "{} vectors loaded", all.len());
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let provenance = std::fs::read_to_string(dir.join("PROVENANCE.toml")).expect("readable");
        let listed = provenance
            .lines()
            .filter(|line| line.starts_with("name = \"tcp/adu_"))
            .count();
        assert_eq!(listed, all.len(), "one [[vector]] entry per adu vector");
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
            let hex = std::fs::read_to_string(dir.join(format!("{}.hex", vector.name)))
                .expect("readable");
            assert!(
                hex.lines()
                    .next()
                    .is_some_and(|l| l.contains("7 byte MBAP header")),
                "{}: the first comment says the file holds a whole ADU",
                vector.name
            );
            if vector.source == VectorSource::ProtocolSpec {
                assert!(
                    hex.contains("Modbus Messaging on TCP/IP Implementation Guide V1.0b")
                        && hex.contains("Modbus Application Protocol V1.1b section"),
                    "{}: cites both documents",
                    vector.name
                );
            }
            let pdu_error = vector
                .expected
                .as_ref()
                .and_then(|expected| expected.get("pdu")?.get("error"))
                .is_some();
            assert_eq!(
                vector.must_fail,
                text(vector, "error").is_some() || text(vector, "outcome").is_some() || pdu_error,
                "{}: only a must_fail vector names an error, an outcome or a PDU error",
                vector.name
            );
        }
        let names: BTreeSet<String> = all.into_iter().map(|v| v.name).collect();
        for name in [
            "length-255-header-only",
            "length-1",
            "length-0",
            "protocol-id-1",
            "truncated-header-6-bytes",
            "length-exceeds-bytes-present",
            "stream-coalesced-1024",
            "261-bytes-full",
        ] {
            assert!(
                names.contains(&format!("tcp/adu_{name}")),
                "adu_{name} is listed"
            );
        }
    }

    #[test]
    fn decode_compare_every_accepted_vector_matches_its_expected_file() {
        let all = accepted();
        assert!(all.len() >= 15, "{} accepted vectors", all.len());
        for vector in &all {
            let want = expected_adus(vector);
            let (adus, rest) = drain(&vector.bytes);
            assert_eq!(adus.len(), want.len(), "{}: number of ADUs", vector.name);
            assert_eq!(
                rest,
                leftover(vector),
                "{}: bytes left in the buffer",
                vector.name
            );
            for (index, (got, want)) in adus.iter().zip(&want).enumerate() {
                let header = got.header();
                let at = format!("{} adu {index}", vector.name);
                assert_eq!(
                    header.transaction_id, want.transaction_id,
                    "{at}: transaction id"
                );
                assert_eq!(header.protocol_id, want.protocol_id, "{at}: protocol id");
                assert_eq!(header.length, want.length, "{at}: length");
                assert_eq!(header.unit_id, UnitId::from(want.unit_id), "{at}: unit id");
                assert_eq!(&got.pdu()[..], want.pdu.as_slice(), "{at}: pdu");
                assert_eq!(got.wire_len(), want.pdu.len() + 7, "{at}: wire length");
            }
        }
    }

    #[test]
    fn re_encode_compare_every_expected_adu_encodes_to_the_vector_bytes() {
        for vector in accepted() {
            let mut framer = MbapFramer::new();
            let mut out = BytesMut::new();
            for want in expected_adus(&vector) {
                let adu = ModbusAdu::new(
                    want.transaction_id,
                    UnitId::from(want.unit_id),
                    Bytes::from(want.pdu.clone()),
                )
                .unwrap_or_else(|e| panic!("{}: the expected ADU builds: {e}", vector.name));
                assert_eq!(adu.header().length, want.length, "{}: length", vector.name);
                assert_eq!(
                    adu.header().protocol_id,
                    want.protocol_id,
                    "{}: protocol",
                    vector.name
                );
                assert_eq!(
                    framer.encode(&adu, &mut out),
                    Ok(()),
                    "{}: encodes",
                    vector.name
                );
            }
            let framed_len = vector.bytes.len() - leftover(&vector);
            assert_eq!(
                &out[..],
                upto(&vector.bytes, framed_len),
                "{}: encoded bytes against the .hex",
                vector.name
            );
        }
    }

    #[test]
    fn decode_then_encode_is_the_identity_on_every_accepted_vector() {
        for vector in accepted() {
            let (adus, rest) = drain(&vector.bytes);
            let mut framer = MbapFramer::new();
            let mut out = BytesMut::new();
            for adu in &adus {
                framer.encode(adu, &mut out).expect("a decoded ADU encodes");
            }
            assert_eq!(
                &out[..],
                upto(&vector.bytes, vector.bytes.len() - rest),
                "{}: bytes",
                vector.name
            );
        }
    }

    #[test]
    fn every_accepted_vector_decodes_the_same_when_fed_one_byte_at_a_time() {
        for vector in accepted() {
            let (want, _) = drain(&vector.bytes);
            let mut framer = MbapFramer::new();
            let mut buffered = BytesMut::new();
            let mut got = Vec::new();
            for byte in &vector.bytes {
                buffered.extend_from_slice(&[*byte]);
                while let Some(adu) = framer.decode(&mut buffered).expect("accepted") {
                    got.push(adu);
                }
            }
            assert_eq!(got, want, "{}: ADUs from a byte-wise feed", vector.name);
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn every_must_fail_vector_fails_or_waits_as_its_expected_file_says() {
        let all = rejected();
        assert!(all.len() >= 10, "{} must_fail vectors", all.len());
        for vector in &all {
            let (result, after) = decode(&vector.bytes);
            assert_eq!(
                &after[..],
                &vector.bytes[..],
                "{}: buffer untouched",
                vector.name
            );
            let note = vector.note.as_deref().unwrap_or_default();
            match (text(vector, "error"), text(vector, "outcome")) {
                (Some("Malformed"), None) => {
                    assert_eq!(result, Err(FrameError::Malformed), "{}", vector.name);
                    assert!(
                        note.contains("FrameError::Malformed"),
                        "{}: {note}",
                        vector.name
                    );
                }
                (Some("Oversized"), None) => {
                    let size = small::<usize>(vector.expected.as_ref().expect("expected"), "size");
                    assert_eq!(result, Err(FrameError::Oversized(size)), "{}", vector.name);
                    assert!(
                        note.contains("FrameError::Oversized"),
                        "{}: {note}",
                        vector.name
                    );
                }
                (None, Some("incomplete")) => {
                    assert_eq!(result, Ok(None), "{}: waits for more bytes", vector.name);
                    assert!(note.contains("Ok(None)"), "{}: {note}", vector.name);
                    if vector
                        .expected
                        .as_ref()
                        .is_some_and(|e| e.get("adu_len").is_some())
                    {
                        let header = MbapHeader::decode(&vector.bytes).expect("a complete header");
                        let declared =
                            small::<usize>(vector.expected.as_ref().expect("e"), "adu_len");
                        assert_eq!(header.adu_len(), declared, "{}: declared ADU", vector.name);
                        assert!(
                            vector.bytes.len() < declared,
                            "{}: body is short",
                            vector.name
                        );
                    }
                }
                other => panic!("{}: unexpected expectation {other:?}", vector.name),
            }
            // The same bytes as a bare header, where there are enough of them.
            let header = MbapHeader::decode(&vector.bytes);
            match text(vector, "header_error") {
                Some("Truncated") => {
                    assert_eq!(header, Err(FrameError::Truncated), "{}", vector.name);
                }
                Some(other) => panic!("{}: unknown header_error {other}", vector.name),
                None => assert_eq!(
                    header.err(),
                    result.err(),
                    "{}: the header alone gives the same verdict",
                    vector.name
                ),
            }
        }
    }

    #[test]
    fn the_upstream_adus_are_cut_whole_as_the_parser_suite_describes() {
        // Header fields and PDU lengths as written in ParserSerializerTestsuite.xml: the
        // frames of the FC04 pair and of the four function 0x14 and 0x15 frames.
        let expected: [(&str, u16, u16, u8, usize, usize); 6] = [
            (
                "upstream-read-input-registers-request",
                0x0000,
                6,
                0xFF,
                5,
                12,
            ),
            (
                "upstream-read-input-registers-response",
                0x7CFE,
                201,
                0xFF,
                200,
                207,
            ),
            (
                "upstream-read-extended-registers-request-split-file-record",
                0x000A,
                17,
                1,
                16,
                23,
            ),
            (
                "upstream-read-extended-registers-response-split-file-record",
                0x000A,
                27,
                1,
                26,
                33,
            ),
            (
                "upstream-write-extended-registers-request-file-record",
                0x000A,
                12,
                1,
                11,
                18,
            ),
            (
                "upstream-write-extended-registers-request-split-file-record",
                0x000A,
                21,
                1,
                20,
                27,
            ),
        ];
        for (name, transaction_id, length, unit_id, pdu_len, wire_len) in expected {
            let vector = vector(name);
            assert_eq!(vector.source, VectorSource::UpstreamFixture, "{name}");
            assert_eq!(vector.bytes.len(), wire_len, "{name}: bytes");
            let (adus, rest) = drain(&vector.bytes);
            assert_eq!((adus.len(), rest), (1, 0), "{name}: one whole ADU");
            let adu = adus.first().expect("one ADU");
            let header = adu.header();
            assert_eq!(header.transaction_id, transaction_id, "{name}: transaction");
            assert_eq!(header.protocol_id, 0, "{name}: protocol");
            assert_eq!(header.length, length, "{name}: length");
            assert_eq!(header.unit_id, UnitId::from(unit_id), "{name}: unit");
            assert_eq!(adu.pdu().len(), pdu_len, "{name}: pdu length");
            assert!(
                !vector.must_fail,
                "{name}: framed, and decoded by the PDU layer"
            );
        }
    }

    #[test]
    fn the_stream_vector_is_one_kibibyte_of_seven_adus() {
        let stream = vector("stream-coalesced-1024");
        assert_eq!(stream.bytes.len(), 1024);
        let (adus, rest) = drain(&stream.bytes);
        assert_eq!(adus.len(), 7);
        assert_eq!(rest, 0);
        let sizes: Vec<usize> = adus.iter().map(ModbusAdu::wire_len).collect();
        assert_eq!(sizes, [12, 259, 9, 12, 259, 259, 214]);
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    // CODING_STANDARDS 13.7: zero length.
    #[test]
    fn mbap_empty_buffer_waits() {
        let (result, after) = decode(&[]);
        assert_eq!(result, Ok(None));
        assert!(after.is_empty());
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_truncated_header_waits() {
        let adu = read_coils_adu();
        for have in 0..MbapHeader::ENCODED_LEN {
            let (result, after) = decode(upto(&adu, have));
            assert_eq!(result, Ok(None), "{have} header bytes");
            assert_eq!(
                MbapHeader::decode(upto(&adu, have)),
                Err(FrameError::Truncated),
                "{have} header bytes read as a whole header"
            );
            assert_eq!(
                &after[..],
                upto(&adu, have),
                "{have} header bytes: untouched"
            );
        }
        assert_eq!(
            MbapHeader::decode(upto(&adu, 6)),
            Err(FrameError::Truncated)
        );
        assert_eq!(MbapHeader::decode(&[]), Err(FrameError::Truncated));
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    // CODING_STANDARDS 13.7: length one byte short.
    #[test]
    fn mbap_seven_bytes_with_the_body_one_byte_short_waits() {
        let (result, after) = decode(&wire(1, 0, 2, 0x11, &[]));
        assert_eq!(result, Ok(None));
        assert_eq!(after.len(), 7, "nothing consumed");
        let adu = read_coils_adu();
        let (result, after) = decode(upto(&adu, adu.len() - 1));
        assert_eq!(result, Ok(None), "one byte short of a whole ADU");
        assert_eq!(after.len(), adu.len() - 1);
    }

    #[test]
    fn mbap_minimal_adu_is_decoded() {
        let (result, after) = decode(&wire(8, 0, 2, 1, &[0x03]));
        let adu = result.expect("valid").expect("a whole ADU");
        assert_eq!(adu.transaction_id(), 8);
        assert_eq!(adu.unit_id(), UnitId::from(1));
        assert_eq!(&adu.pdu()[..], &[0x03]);
        assert_eq!(adu.wire_len(), 8);
        assert!(after.is_empty(), "exactly the ADU is consumed");
    }

    // CODING_STANDARDS 13.7: exact length, and length one byte long.
    #[test]
    fn mbap_exact_adu_is_consumed_and_one_extra_byte_is_left() {
        let mut bytes = read_coils_adu();
        let (result, after) = decode(&bytes);
        assert_eq!(
            &result.expect("valid").expect("whole").pdu()[..],
            &READ_COILS
        );
        assert!(after.is_empty());
        bytes.push(0xAA);
        let (result, after) = decode(&bytes);
        assert!(result.expect("valid").is_some());
        assert_eq!(&after[..], &[0xAA], "the extra byte stays in the buffer");
    }

    #[test]
    fn mbap_coalesced_adus_are_split() {
        let mut bytes = read_coils_adu();
        bytes.extend_from_slice(&wire(2, 0, 3, 0x11, &[0x81, 0x02]));
        let mut framer = MbapFramer::new();
        let mut buffered = buffer(&bytes);
        let first = framer.decode(&mut buffered).expect("valid").expect("first");
        assert_eq!(first.transaction_id(), 1);
        assert_eq!(buffered.len(), 9, "the second ADU is untouched");
        let second = framer
            .decode(&mut buffered)
            .expect("valid")
            .expect("second");
        assert_eq!(second.transaction_id(), 2);
        assert_eq!(&second.pdu()[..], &[0x81, 0x02]);
        assert_eq!(framer.decode(&mut buffered), Ok(None));
        assert!(buffered.is_empty());
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    // CODING_STANDARDS 13.7: maximum length accepted, maximum plus one rejected.
    #[test]
    fn mbap_length_254_with_the_full_body_is_accepted_at_260_bytes() {
        let pdu = vec![0x5A; MAX_PDU_BYTES];
        let bytes = wire(7, 0, 254, 1, &pdu);
        assert_eq!(bytes.len(), 260);
        let (result, after) = decode(&bytes);
        let adu = result.expect("valid").expect("whole");
        assert_eq!(adu.pdu().len(), 253);
        assert_eq!(adu.wire_len(), 260);
        assert!(after.is_empty());
        assert_eq!(MbapFramer::new().max_frame_bytes(), 260);
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_length_255_is_rejected() {
        let (result, after) = decode(&wire(1, 0, 255, 1, &[]));
        assert_eq!(result, Err(FrameError::Oversized(261)));
        assert_eq!(after.len(), 7, "untouched");
        let (result, _) = decode(&wire(1, 0, 255, 1, &[0; 254]));
        assert_eq!(
            result,
            Err(FrameError::Oversized(261)),
            "a whole 261 byte ADU too"
        );
        let (result, _) = decode(&wire(1, 0, 0xFFFF, 1, &[]));
        assert_eq!(result, Err(FrameError::Oversized(65_541)));
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_length_1_is_rejected() {
        let (result, after) = decode(&wire(1, 0, 1, 1, &READ_COILS));
        assert_eq!(result, Err(FrameError::Malformed));
        assert_eq!(after.len(), 12, "untouched");
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_length_0_is_rejected() {
        let (result, _) = decode(&wire(1, 0, 0, 1, &READ_COILS));
        assert_eq!(result, Err(FrameError::Malformed));
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_protocol_id_1_is_rejected() {
        for protocol in [1, 0x0100, 0xFFFF] {
            let (result, after) = decode(&wire(1, protocol, 6, 0x11, &READ_COILS));
            assert_eq!(
                result,
                Err(FrameError::Malformed),
                "protocol id {protocol:#06x}"
            );
            assert_eq!(after.len(), 12, "untouched");
        }
        let (result, _) = decode(&wire(1, 1, 255, 1, &[]));
        assert_eq!(
            result,
            Err(FrameError::Malformed),
            "protocol id is judged before length"
        );
    }

    #[test]
    fn mbap_header_is_judged_before_the_body_is_awaited() {
        // Only the seven header bytes are here; a length of 255 is refused at once
        // instead of waiting for 254 bytes that may never come.
        let (result, _) = decode(&wire(1, 0, 255, 1, &[]));
        assert!(result.is_err());
        let (result, _) = decode(&wire(1, 0, 254, 1, &[]));
        assert_eq!(result, Ok(None), "the largest legal length waits");
    }

    #[test]
    fn mbap_header_matches_spec_vector() {
        // Messaging guide layout around the V1.1b section 6.3 example: transaction 3,
        // protocol 0, length 6, unit 17.
        let header =
            MbapHeader::decode(&[0x00, 0x03, 0x00, 0x00, 0x00, 0x06, 0x11]).expect("valid");
        assert_eq!(
            header,
            MbapHeader {
                transaction_id: 3,
                protocol_id: 0,
                length: 6,
                unit_id: UnitId::from(17),
            }
        );
        assert_eq!(header.adu_len(), 12);
        let mut out = BytesMut::from(&b"x"[..]);
        header.encode(&mut out).expect("valid");
        assert_eq!(
            &out[..],
            &[b'x', 0x00, 0x03, 0x00, 0x00, 0x00, 0x06, 0x11],
            "appends"
        );
        assert_eq!(MbapHeader::ENCODED_LEN, 7);
        assert_eq!((MbapHeader::MIN_LENGTH, MbapHeader::MAX_LENGTH), (2, 254));
    }

    #[test]
    fn mbap_header_encode_refuses_what_decode_refuses_and_appends_nothing() {
        let valid = MbapHeader {
            transaction_id: 1,
            protocol_id: 0,
            length: 6,
            unit_id: UnitId::default(),
        };
        let cases = [
            (
                MbapHeader {
                    protocol_id: 1,
                    ..valid
                },
                FrameError::Malformed,
            ),
            (MbapHeader { length: 1, ..valid }, FrameError::Malformed),
            (MbapHeader { length: 0, ..valid }, FrameError::Malformed),
            (
                MbapHeader {
                    length: 255,
                    ..valid
                },
                FrameError::Oversized(261),
            ),
        ];
        for (header, error) in cases {
            let mut out = BytesMut::new();
            assert_eq!(header.encode(&mut out), Err(error), "{header:?}");
            assert!(out.is_empty(), "{header:?}: nothing appended");
        }
        let mut out = BytesMut::new();
        assert_eq!(MbapHeader { length: 2, ..valid }.encode(&mut out), Ok(()));
        assert_eq!(
            MbapHeader {
                length: 254,
                ..valid
            }
            .encode(&mut out),
            Ok(())
        );
    }

    #[test]
    fn adu_constructor_validates_the_pdu_length() {
        let unit = UnitId::from(5);
        assert_eq!(
            ModbusAdu::new(1, unit, Bytes::new()),
            Err(FrameError::Malformed),
            "an empty PDU"
        );
        let smallest = ModbusAdu::new(1, unit, Bytes::from_static(&[0x03])).expect("1 byte");
        assert_eq!((smallest.header().length, smallest.wire_len()), (2, 8));
        let largest = ModbusAdu::new(1, unit, Bytes::from(vec![0; MAX_PDU_BYTES])).expect("253");
        assert_eq!((largest.header().length, largest.wire_len()), (254, 260));
        assert_eq!(largest.header().protocol_id, 0);
        assert_eq!(
            ModbusAdu::new(1, unit, Bytes::from(vec![0; MAX_PDU_BYTES + 1])),
            Err(FrameError::Oversized(261)),
            "a 254 byte PDU"
        );
        assert_eq!(
            ModbusAdu::new(1, unit, Bytes::from(vec![0; 70_000])),
            Err(FrameError::Oversized(70_007))
        );
    }

    #[test]
    fn adu_accessors_and_debug_never_show_pdu_bytes() {
        let adu =
            ModbusAdu::new(9, UnitId::from(2), Bytes::from_static(&[0xDE, 0xAD])).expect("ok");
        assert_eq!(adu.transaction_id(), 9);
        assert_eq!(adu.unit_id(), UnitId::from(2));
        assert_eq!(adu.pdu(), &Bytes::from_static(&[0xDE, 0xAD]));
        let shown = format!("{adu:?}");
        assert!(shown.contains("pdu_len: 2"), "{shown}");
        assert!(shown.contains("transaction_id: 9"), "{shown}");
        assert!(!shown.contains("222") && !shown.contains("DE"), "{shown}");
        assert_eq!(adu.clone().into_pdu(), Bytes::from_static(&[0xDE, 0xAD]));
    }

    #[test]
    fn mbap_encode_appends_the_header_then_the_pdu_to_a_used_buffer() {
        let adu =
            ModbusAdu::new(1, UnitId::from(0x11), Bytes::copy_from_slice(&READ_COILS)).expect("ok");
        let mut out = BytesMut::from(&b"prefix"[..]);
        MbapFramer::new().encode(&adu, &mut out).expect("encodes");
        assert_eq!(upto(&out, 6), b"prefix");
        assert_eq!(from(&out, 6), read_coils_adu().as_slice());
        assert_eq!(MbapFramer::new(), MbapFramer);
    }

    #[test]
    fn txn_ids_start_at_1() {
        let mut ids = TransactionId::default();
        assert_eq!(ids, TransactionId::FIRST);
        assert_eq!(ids.next().get(), 1);
        assert_eq!(ids.next().get(), 2);
        assert_eq!(ids.next().get(), 3);
    }

    // covers: MB-READ-TXN (wrap before 0xFFFF)
    #[test]
    fn txn_ids_wrap_from_fffe_to_1() {
        let mut ids = TransactionId::LAST;
        assert_eq!(ids.get(), 0xFFFE);
        assert_eq!(ids.next().get(), 0xFFFE);
        assert_eq!(ids.next().get(), 1);
        assert_eq!(ids.next().get(), 2);
    }

    #[test]
    fn txn_ids_never_yield_0_or_ffff_and_cycle_through_65534_values() {
        let mut ids = TransactionId::default();
        let mut seen = BTreeSet::new();
        for _ in 0..(2 * 65_534) {
            let id = ids.next().get();
            assert!(id != 0 && id != 0xFFFF, "yielded {id:#06x}");
            seen.insert(id);
        }
        assert_eq!(seen.len(), 65_534);
        assert_eq!(ids, TransactionId::FIRST, "two full cycles are back at 1");
        assert_eq!(u16::from(TransactionId::LAST), 0xFFFE);
        assert_eq!(TransactionId::LAST.to_string(), "65534");
    }

    // The framer over the in-memory transport double: the fake clock is the only clock.
    struct Rig {
        clock: FakeClock,
        framed: FramedTransport<ModbusAdu, MbapFramer>,
    }

    impl Rig {
        fn new(steps: Vec<Step>) -> Self {
            let clock = FakeClock::default();
            let limits = ResourceLimits::builder().build().expect("valid limits");
            let transport = FakeTransport::new(Script::new(steps), clock.clone(), limits);
            let framed = FramedTransport::new(Box::new(transport), MbapFramer::new());
            Self { clock, framed }
        }

        fn deadline(&self) -> Deadline {
            Deadline::after(self.clock.now(), Duration::from_secs(5)).expect("representable")
        }

        fn next(&mut self) -> stonecrab::Result<ModbusAdu> {
            let deadline = self.deadline();
            block_on_with(&self.clock, self.framed.next_frame(deadline))
        }
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn mbap_fragmented_adu_is_reassembled() {
        let response = vector("read-holding-registers-response").bytes;
        let pieces = vec![
            upto(&response, 3).to_vec(),
            between(&response, 3, 7).to_vec(),
            between(&response, 7, 9).to_vec(),
            from(&response, 9).to_vec(),
        ];
        let mut rig = Rig::new(vec![Step::ReplyFragmented(pieces)]);
        let adu = rig.next().expect("the whole ADU arrives");
        assert_eq!(adu.transaction_id(), 3);
        assert_eq!(&adu.pdu()[..], from(&response, 7));
        assert_eq!(rig.framed.buffered_len(), 0);
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn mbap_coalesced_adus_are_delivered_one_at_a_time() {
        let first = vector("read-coils-request").bytes;
        let second = vector("read-coils-exception").bytes;
        let mut rig = Rig::new(vec![
            Step::ReplyCoalescedWithNext,
            Step::Reply(first),
            Step::Reply(second),
        ]);
        assert_eq!(rig.next().expect("first").transaction_id(), 1);
        assert_eq!(
            rig.framed.buffered_len(),
            9,
            "the second ADU waits in the buffer"
        );
        let second = rig.next().expect("second, with no further read");
        assert_eq!(second.transaction_id(), 5);
        assert_eq!(&second.pdu()[..], &[0x81, 0x02]);
    }

    #[test]
    fn mbap_coalesced_kibibyte_stream_is_delivered_adu_by_adu() {
        let stream = vector("stream-coalesced-1024");
        let mut rig = Rig::new(vec![Step::Reply(stream.bytes.clone())]);
        let ids: Vec<u16> = (0..7)
            .map(|_| rig.next().expect("an ADU").transaction_id())
            .collect();
        assert_eq!(ids, [1, 1, 2, 3, 3, 4, 5]);
        assert_eq!(rig.framed.buffered_len(), 0);
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing, peer close at each framing boundary
    #[test]
    fn mbap_truncated_adu_then_close_is_disconnected() {
        for cut in [3, 6, 7, 11] {
            let mut rig = Rig::new(vec![Step::CloseAt(cut), Step::Reply(read_coils_adu())]);
            let error = rig.next().expect_err("the peer closed inside the ADU");
            assert_eq!(error.kind(), ErrorKind::Disconnected, "cut at {cut}");
            assert_eq!(
                rig.framed.buffered_len(),
                cut,
                "cut at {cut}: partial ADU kept"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated headers, oversized lengths
    #[test]
    fn mbap_bad_headers_surface_as_typed_errors_through_the_transport() {
        let mut rig = Rig::new(vec![Step::Reply(wire(1, 1, 6, 0x11, &READ_COILS))]);
        let error = rig.next().expect_err("protocol id 1");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        let mut rig = Rig::new(vec![Step::Reply(wire(1, 0, 255, 0x11, &[]))]);
        let error = rig.next().expect_err("length 255");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
    }

    #[test]
    fn mbap_resync_skips_a_bad_header_and_finds_the_next_adu() {
        let mut garbage = wire(1, 1, 6, 0x11, &[]);
        garbage.truncate(7);
        let mut rig = Rig::new(vec![Step::Reply(garbage), Step::Reply(read_coils_adu())]);
        assert!(rig.next().is_err(), "the bad header is reported");
        let deadline = rig.deadline();
        block_on_with(&rig.clock, rig.framed.resync(deadline)).expect("realigned");
        let adu = rig.next().expect("the good ADU");
        assert_eq!(adu.transaction_id(), 1);
        assert_eq!(&adu.pdu()[..], &READ_COILS);
    }

    #[test]
    fn mbap_send_frame_puts_header_then_pdu_on_the_wire() {
        let clock = FakeClock::default();
        let limits = ResourceLimits::builder().build().expect("valid limits");
        let transport = FakeTransport::new(Script::default(), clock.clone(), limits);
        let probe = transport.probe();
        let mut framed = FramedTransport::new(Box::new(transport), MbapFramer::new());
        let deadline = Deadline::after(clock.now(), Duration::from_secs(5)).expect("deadline");
        let adu =
            ModbusAdu::new(1, UnitId::from(0x11), Bytes::copy_from_slice(&READ_COILS)).expect("ok");
        block_on_with(&clock, framed.send_frame(&adu, deadline)).expect("sent");
        assert_eq!(
            probe.sent().first().expect("one send").as_ref(),
            read_coils_adu().as_slice()
        );
    }
}
