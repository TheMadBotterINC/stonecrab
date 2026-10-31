//! The first-party byte vectors of `fixtures/opcua/codec` (`CODING_STANDARDS` 13.6).
//!
//! The bytes and the expected files were written from OPC UA Part 6 by a script that shares no
//! code with the codec. Parsing and re-encoding are asserted separately, so a pair of
//! matching mistakes cannot cancel out. The shapes of the expected files are those the
//! `expected_*` functions read: a `type`, then the fields of that built-in; an `error` names
//! the `ParseError` variant of a vector that must fail; `reencodes_as` is the canonical form
//! of a vector that is accepted in a wider or flagged form.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde_json::Value;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{
    DateTime, Depth, ExpandedNodeId, Guid, Identifier, LocalizedText, NodeId, QualifiedName,
    StatusCode, UaByteString, UaString,
};
use crate::testkit::{GoldenVector, load_dir};

fn vectors() -> Vec<GoldenVector> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/opcua/codec");
    load_dir(&directory)
        .expect("fixtures/opcua/codec loads: every file listed, every listed vector present")
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    let found = value.get(key);
    assert!(found.is_some(), "the expected file has `{key}`: {value}");
    found.expect("checked above")
}

fn number(value: &Value, key: &str) -> u64 {
    field(value, key).as_u64().expect("an unsigned number")
}

fn small<T: TryFrom<u64>>(value: &Value, key: &str) -> T
where
    T::Error: Debug,
{
    T::try_from(number(value, key)).expect("the number fits its type")
}

fn text(value: &Value, key: &str) -> Option<String> {
    field(value, key).as_str().map(str::to_owned)
}

fn byte_list(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .expect("a list of bytes")
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().expect("a byte")).expect("fits a byte"))
        .collect()
}

fn byte_string(value: &Value) -> UaByteString {
    if value.is_null() {
        UaByteString::null()
    } else {
        UaByteString::new(byte_list(value))
    }
}

fn ua_string(value: &Value, key: &str) -> UaString {
    text(value, key).map_or_else(UaString::null, UaString::new)
}

fn guid(value: &Value) -> Guid {
    let data4: [u8; 8] = byte_list(field(value, "data4"))
        .try_into()
        .expect("eight bytes");
    Guid {
        data1: small(value, "data1"),
        data2: small(value, "data2"),
        data3: small(value, "data3"),
        data4,
    }
}

fn node_id(value: &Value) -> NodeId {
    let identifier = field(value, "identifier");
    let kind = field(identifier, "kind").as_str().expect("a kind");
    let identifier = match kind {
        "numeric" => Identifier::Numeric(small(identifier, "value")),
        "string" => Identifier::String(ua_string(identifier, "value")),
        "guid" => Identifier::Guid(guid(identifier)),
        _ => {
            assert_eq!(kind, "byte_string", "an identifier kind");
            Identifier::ByteString(byte_string(field(identifier, "value")))
        }
    };
    NodeId {
        namespace: small(value, "namespace"),
        identifier,
    }
}

fn expanded_node_id(value: &Value) -> ExpandedNodeId {
    ExpandedNodeId {
        node_id: node_id(value.get("node_id").expect("a node id")),
        namespace_uri: field(value, "namespace_uri_present")
            .as_bool()
            .expect("a flag")
            .then(|| ua_string(value, "namespace_uri")),
        server_index: field(value, "server_index")
            .as_u64()
            .map(|index| u32::try_from(index).expect("a u32")),
    }
}

fn localized_text(value: &Value) -> LocalizedText {
    let part = |present: &str, key: &str| {
        field(value, present)
            .as_bool()
            .expect("a flag")
            .then(|| ua_string(value, key))
    };
    LocalizedText {
        locale: part("locale_present", "locale"),
        text: part("text_present", "text"),
    }
}

/// The name of a `ParseError` variant.
fn variant(error: &ParseError) -> String {
    format!("{error:?}")
        .split([' ', '{'])
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The bytes a vector must re-encode to: its own, unless its expected file names a canonical
/// form (a hex string).
fn canonical_bytes(vector: &GoldenVector) -> Vec<u8> {
    let Some(hex) = vector
        .expected
        .as_ref()
        .and_then(|json| json.get("reencodes_as"))
    else {
        return vector.bytes.clone();
    };
    hex.as_str()
        .expect("hex text")
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let digits = std::str::from_utf8(pair).expect("hex digits");
            u8::from_str_radix(digits, 16).expect("hex digits")
        })
        .collect()
}

/// Parses, re-encodes and measures one vector against its expectation.
fn check<T: Debug + PartialEq>(
    vector: &GoldenVector,
    expected: impl FnOnce(&Value) -> T,
    parse: fn(&mut BoundedReader<'_>, Depth) -> Result<T, ParseError>,
    serialize: fn(&T, &mut BoundedWriter, Depth) -> Result<(), ParseError>,
    length: fn(&T, Depth) -> Result<usize, ParseError>,
) {
    let name = &vector.name;
    let json = vector
        .expected
        .as_ref()
        .expect("every vector has an expected file");
    let depth = Depth::new();
    let mut reader = BoundedReader::new(&vector.bytes);
    let parsed = parse(&mut reader, depth);
    if vector.must_fail {
        let error = parsed.expect_err(&format!("{name} must be refused"));
        assert_eq!(
            variant(&error),
            field(json, "error").as_str().expect("an error name"),
            "{name}: the refusal"
        );
        return;
    }
    let expected = expected(json);
    assert_eq!(
        parsed.as_ref().ok(),
        Some(&expected),
        "{name}: decoded value ({parsed:?})"
    );
    assert_eq!(reader.remaining(), 0, "{name}: every byte is consumed");

    let canonical = canonical_bytes(vector);
    let mut writer = BoundedWriter::new(canonical.len());
    let written = serialize(&expected, &mut writer, depth);
    assert_eq!(written, Ok(()), "{name}: serialize");
    assert_eq!(writer.into_bytes(), canonical, "{name}: re-encoded bytes");
    assert_eq!(
        length(&expected, depth),
        Ok(canonical.len()),
        "{name}: length_in_bytes"
    );
}

// covers: SPEC mandatory-failure-cases / truncated headers, invalid discriminators and
// oversized lengths of the OPC UA built-ins
#[test]
fn opcua_codec_first_party_vectors_parse_and_reencode() {
    // Per built-in: the vectors that are accepted and the ones that must be refused.
    let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for vector in vectors() {
        let json = vector.expected.as_ref().expect("an expected file");
        let kind = field(json, "type").as_str().expect("a type").to_owned();
        match kind.as_str() {
            "String" => check(
                &vector,
                |json| {
                    json.get("bytes").map_or_else(
                        || ua_string(json, "value"),
                        |bytes| UaString::from_bytes(byte_list(bytes)),
                    )
                },
                UaString::parse,
                UaString::serialize,
                UaString::length_in_bytes,
            ),
            "ByteString" => check(
                &vector,
                |json| byte_string(field(json, "value")),
                UaByteString::parse,
                UaByteString::serialize,
                UaByteString::length_in_bytes,
            ),
            "Guid" => check(
                &vector,
                guid,
                Guid::parse,
                Guid::serialize,
                Guid::length_in_bytes,
            ),
            "DateTime" => {
                check(
                    &vector,
                    |json| DateTime(field(json, "ticks").as_i64().expect("ticks")),
                    DateTime::parse,
                    DateTime::serialize,
                    DateTime::length_in_bytes,
                );
                if let Some(seconds) = json.get("unix_seconds") {
                    assert_eq!(
                        seconds.as_u64(),
                        Some(0),
                        "only the epoch has a seconds field"
                    );
                    let time = DateTime(field(json, "ticks").as_i64().expect("ticks"));
                    assert_eq!(time.to_system_time(), Some(UNIX_EPOCH), "{}", vector.name);
                }
            }
            "StatusCode" => check(
                &vector,
                |json| StatusCode(small(json, "code")),
                StatusCode::parse,
                StatusCode::serialize,
                StatusCode::length_in_bytes,
            ),
            "NodeId" => check(
                &vector,
                node_id,
                NodeId::parse,
                NodeId::serialize,
                NodeId::length_in_bytes,
            ),
            "ExpandedNodeId" => check(
                &vector,
                expanded_node_id,
                ExpandedNodeId::parse,
                ExpandedNodeId::serialize,
                ExpandedNodeId::length_in_bytes,
            ),
            "QualifiedName" => check(
                &vector,
                |json| QualifiedName {
                    namespace_index: small(json, "namespace_index"),
                    name: ua_string(json, "name"),
                },
                QualifiedName::parse,
                QualifiedName::serialize,
                QualifiedName::length_in_bytes,
            ),
            _ => {
                assert_eq!(kind, "LocalizedText", "{}: a built-in type", vector.name);
                check(
                    &vector,
                    localized_text,
                    LocalizedText::parse,
                    LocalizedText::serialize,
                    LocalizedText::length_in_bytes,
                );
            }
        }
        let counts = seen.entry(kind).or_default();
        if vector.must_fail {
            counts.1 += 1;
        } else {
            counts.0 += 1;
        }
    }
    assert_eq!(seen.len(), 9, "every built-in has vectors: {seen:?}");
    assert!(
        seen.values()
            .all(|(valid, refused)| *valid >= 1 && *refused >= 1),
        "every built-in has a valid and a refused vector: {seen:?}"
    );
}
