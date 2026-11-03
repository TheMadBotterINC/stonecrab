//! Pinned upstream XML -> independent bytes -> Stonecrab equivalence (S93).
//! The adapter follows annotated XML leaf order, not generated service constructors.
//! Set `STONECRAB_REGENERATE_OPCUA_VECTORS=1` to rewrite the reviewed derived corpus.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use bytes::BytesMut;
use roxmltree::{Document, Node};
use serde_json::{Value, json};

use crate::ResourceLimits;
use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, ExpandedNodeId, ExtensionBody, ExtensionObject, NodeId};
use crate::opcua::frame::{Body, OpcuaFramer};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::testkit::{load_dir, load_hex, load_provenance};
use crate::transport::Framer;

type Result<T> = std::result::Result<T, String>;
const COMMIT: &str = "ea229918f5d07700e04d7d69ab1645da679cb700";
const SOURCE: &str = "protocols/opcua/src/test/resources/protocols/opcua";
const REGENERATE: &str = "STONECRAB_REGENERATE_OPCUA_VECTORS";
const SUITES: [(&str, &str, &str); 2] = [
    (
        "ParserSerializerTestsuite.xml",
        "8b6e045af9d12f2b6602935f0aa0055934d74c70",
        "78dceebbcc0fa94eee6a7ec0dd14d749ef7f1b843313983ff4f33a18d7ccdde0",
    ),
    (
        "DriverTestsuite.xml",
        "73e62eba94a8190005e48f1a327a5e0eb30da9cf",
        "a879927e60a7395af8b5b8316f9c11eb3795f98ee6270f48c90406ba0a119b3f",
    ),
];

// S123 must empty this exact list as the owning services are generated.
const REFUSALS: [(&str, i32, &str); 5] = [
    (
        "raw/opc-ua-publish-response-payload-data-change",
        829,
        "PublishResponse",
    ),
    (
        "raw/opc-ua-publish-response-payload-event-notification",
        829,
        "PublishResponse",
    ),
    (
        "raw/opc-ua-notification-message",
        805,
        "NotificationMessage",
    ),
    (
        "raw/opc-ua-monitored-item-notification",
        808,
        "MonitoredItemNotification",
    ),
    (
        "raw/opc-ua-monitoring-parameters",
        742,
        "MonitoringParameters",
    ),
];
const CANONICAL_READ: &str = "driver/send-read-request";
// This envelope round-trips opaquely; it is not evidence of DataChangeNotification support.
const OPAQUE: &str = "raw/opc-ua-extension-object";

fn directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/opcua/upstream")
}
fn read(path: impl AsRef<Path>) -> String {
    fs::read_to_string(path).expect("fixture text")
}
fn hex(bytes: &[u8]) -> String {
    let mut out = String::new();
    for byte in bytes {
        write!(&mut out, "{byte:02x}").expect("String write");
    }
    out
}
fn slug(text: &str) -> String {
    text.to_ascii_lowercase()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}
fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Result<Node<'a, 'input>> {
    let mut found = node
        .children()
        .filter(|child| child.is_element() && child.tag_name().name() == name);
    let value = found.next().ok_or_else(|| format!("missing {name}"))?;
    if found.next().is_some() {
        return Err(format!("duplicate {name}"));
    }
    Ok(value)
}
fn number(node: Node<'_, '_>) -> Result<i128> {
    node.text()
        .unwrap_or_default()
        .trim()
        .parse()
        .map_err(|_error| format!("invalid integer {}", node.tag_name().name()))
}
fn width(node: Node<'_, '_>) -> Result<usize> {
    node.attribute("bitLength")
        .ok_or_else(|| "missing bitLength".to_owned())?
        .parse()
        .map_err(|_error| "invalid bitLength".to_owned())
}

#[derive(Debug, Default)]
struct Encoder {
    bytes: Vec<u8>,
    partial: u8,
    used: usize,
    adjustments: Vec<String>,
}
impl Encoder {
    fn adjustment(&mut self, field: &str, declared: i128, actual: i128) {
        if declared != actual {
            self.adjustments
                .push(format!("{field}: {declared} -> {actual}"));
        }
    }
    fn append(&mut self, bytes: &[u8]) -> Result<()> {
        if self.used != 0 {
            return Err("unaligned byte field".to_owned());
        }
        self.bytes.extend_from_slice(bytes);
        Ok(())
    }
    #[expect(
        clippy::unwrap_in_result,
        reason = "width is checked before bounded conversions and slices"
    )]
    fn integer(&mut self, value: i128, bits: usize, signed: bool) -> Result<()> {
        if bits == 0 || bits > 64 {
            return Err("integer width must be 1 through 64".to_owned());
        }
        let bound = 1_i128
            .checked_shl(
                u32::try_from(if signed { bits.saturating_sub(1) } else { bits }).expect("width"),
            )
            .expect("bounded shift");
        let minimum = if signed {
            bound.checked_neg().expect("bounded signed width")
        } else {
            0
        };
        let maximum = bound.saturating_sub(1);
        if !(minimum..=maximum).contains(&value) {
            return Err("integer outside declared width".to_owned());
        }
        if bits < 8 {
            if self.used.saturating_add(bits) > 8 {
                return Err("bit field crosses byte boundary".to_owned());
            }
            let mask = 1_i128
                .checked_shl(u32::try_from(bits).expect("width"))
                .expect("shift")
                .saturating_sub(1);
            self.partial = self
                .partial
                .checked_shl(u32::try_from(bits).expect("width"))
                .expect("shift is below eight")
                | u8::try_from(value & mask).expect("masked byte");
            self.used = self.used.saturating_add(bits);
            if self.used == 8 {
                self.bytes.push(self.partial);
                self.partial = 0;
                self.used = 0;
            }
            Ok(())
        } else if matches!(bits, 8 | 16 | 32 | 64) {
            self.append(
                value
                    .to_le_bytes()
                    .get(..bits.checked_div(8).expect("nonzero"))
                    .expect("integer bytes"),
            )
        } else {
            Err("unsupported integer width".to_owned())
        }
    }
    #[expect(
        clippy::unwrap_in_result,
        reason = "fixture UTF-8 byte lengths fit the platform-independent i128 bound"
    )]
    fn string(&mut self, node: Node<'_, '_>) -> Result<()> {
        let length = child(node, "sLength")?;
        if length.attribute("dataType") != Some("int") || width(length)? != 32 {
            return Err("invalid PascalString length shape".to_owned());
        }
        let declared = number(length)?;
        if declared < -1 {
            return Err("invalid PascalString length".to_owned());
        }
        let value = node
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "stringValue");
        let text = value.and_then(|value| value.text()).unwrap_or_default();
        if declared == -1 && !text.is_empty() {
            return Err("null String with content".to_owned());
        }
        let actual = if declared == -1 {
            -1
        } else {
            i128::try_from(text.len()).expect("String length")
        };
        self.adjustment("sLength", declared, actual);
        if let Some(value) = value {
            if value.attribute("dataType") != Some("string")
                || value.attribute("encoding") != Some("UTF-8")
            {
                return Err("unsupported String shape".to_owned());
            }
            self.adjustment(
                "bitLength",
                i128::try_from(width(value)?).expect("width"),
                i128::try_from(text.len().saturating_mul(8)).expect("width"),
            );
        }
        self.integer(actual, 32, true)?;
        self.append(text.as_bytes())
    }
    fn leaf(&mut self, node: Node<'_, '_>, kind: &str) -> Result<()> {
        let bits = width(node)?;
        let text = node.text().unwrap_or_default();
        match kind {
            "bit" => {
                if bits != 1 {
                    return Err("bit width must be one".to_owned());
                }
                self.integer(
                    match text {
                        "true" => 1,
                        "false" => 0,
                        _ => return Err("invalid bit".to_owned()),
                    },
                    bits,
                    false,
                )
            }
            "int" | "uint" => self.integer(number(node)?, bits, kind == "int"),
            "string" => {
                if node.attribute("encoding") != Some("UTF-8")
                    || bits != text.len().saturating_mul(8)
                {
                    return Err("String bitLength mismatch".to_owned());
                }
                self.append(text.as_bytes())
            }
            "byte" => {
                let raw = text
                    .strip_prefix("0x")
                    .ok_or_else(|| "byte payload needs 0x".to_owned())?;
                let bytes = if raw.is_empty() {
                    Vec::new()
                } else {
                    load_hex(raw).map_err(|error| error.to_string())?
                };
                if bits != bytes.len().saturating_mul(8) {
                    return Err("byte bitLength mismatch".to_owned());
                }
                self.append(&bytes)
            }
            "float" if bits == 64 => {
                let value = text
                    .parse::<f64>()
                    .map_err(|_error| "invalid float".to_owned())?;
                self.append(&value.to_le_bytes())
            }
            _ => Err(format!("unsupported XML leaf {kind}/{bits}")),
        }
    }
    #[expect(
        clippy::unwrap_in_result,
        reason = "fixture message sizes fit i128 before checked u32 conversion"
    )]
    fn node(&mut self, node: Node<'_, '_>) -> Result<()> {
        if node.tag_name().name() == "PascalString" {
            return self.string(node);
        }
        if let Some(kind) = node.attribute("dataType") {
            return self.leaf(node, kind);
        }
        let mut elements = 0_usize;
        for child in node.children().filter(Node::is_element) {
            elements = elements.saturating_add(1);
            self.node(child)?;
        }
        if elements == 0 && node.text().is_some_and(|text| !text.trim().is_empty()) {
            return Err("untyped text leaf".to_owned());
        }
        if node.tag_name().name() == "MessagePDU" {
            let length = child(node, "totalLength")?;
            let actual = self.bytes.len();
            self.adjustment(
                "totalLength",
                number(length)?,
                i128::try_from(actual).expect("frame length"),
            );
            let size = u32::try_from(actual).map_err(|_error| "frame size overflow".to_owned())?;
            self.bytes
                .get_mut(4..8)
                .ok_or_else(|| "missing frame header".to_owned())?
                .copy_from_slice(&size.to_le_bytes());
        }
        Ok(())
    }
}
fn encode(node: Node<'_, '_>) -> Result<Encoder> {
    let mut encoder = Encoder::default();
    encoder.node(node)?;
    if encoder.used != 0 {
        return Err("unfinished bit field".to_owned());
    }
    Ok(encoder)
}

struct Vector {
    name: String,
    bytes: Vec<u8>,
    expected: Value,
    origin: String,
}
fn corpus() -> Vec<Vector> {
    let mut vectors = Vec::new();
    for (index, (file, _, _)) in SUITES.iter().enumerate() {
        let text = read(directory().join(file));
        let document = Document::parse(&text).expect("upstream XML");
        if index == 0 {
            for case in document
                .root_element()
                .children()
                .filter(|node| node.has_tag_name("testcase"))
            {
                let name = child(case, "name").unwrap().text().unwrap();
                let raw = load_hex(child(case, "raw").unwrap().text().unwrap()).unwrap();
                let root = child(case, "xml")
                    .unwrap()
                    .children()
                    .find(Node::is_element)
                    .unwrap();
                let encoded = encode(root).unwrap();
                assert_eq!(
                    encoded.bytes, raw,
                    "independent XML serialization of {name}"
                );
                assert_eq!(encoded.adjustments, Vec::<String>::new());
                let id = case
                    .descendants()
                    .find(|node| node.has_tag_name("extensionId"))
                    .map(|node| i32::try_from(number(node).unwrap()).unwrap());
                vectors.push(Vector {
                    name: format!("raw/{}", slug(name)),
                    bytes: raw,
                    expected: json!({
                        "case": name,
                        "root_type": child(case, "root-type").unwrap().text().unwrap(),
                        "encoding_id": id,
                        "adjustments": []
                    }),
                    origin: format!("{SOURCE}/{file}@{COMMIT}, testcase {name}/raw"),
                });
            }
        } else {
            for case in document.descendants().filter(|node| {
                node.has_tag_name("outgoing-plc-message")
                    || node.has_tag_name("incoming-plc-message")
            }) {
                let name = case.attribute("name").unwrap();
                let root = child(case, "MessagePDU").unwrap();
                let encoded = encode(root).unwrap();
                let id = root
                    .descendants()
                    .find(|node| node.has_tag_name("ExtensionObject"))
                    .map(|node| {
                        i32::try_from(
                            number(
                                child(node, "typeId")
                                    .unwrap()
                                    .descendants()
                                    .find(|node| node.has_tag_name("id"))
                                    .unwrap(),
                            )
                            .unwrap(),
                        )
                        .unwrap()
                    });
                vectors.push(Vector {
                    name: format!("driver/{}", slug(name)),
                    bytes: encoded.bytes,
                    expected: json!({
                        "case": name,
                        "root_type": "MessagePDU",
                        "encoding_id": id,
                        "adjustments": encoded.adjustments
                    }),
                    origin: format!(
                        "{SOURCE}/{file}@{COMMIT}, {} {name}/MessagePDU",
                        case.tag_name().name()
                    ),
                });
            }
        }
    }
    assert_eq!(vectors.len(), 23);
    vectors
}
fn canonical_read(bytes: &[u8]) -> Vec<u8> {
    let wide = [2, 0, 0, 0xD2, 0x08, 0, 0];
    let positions = bytes
        .windows(wide.len())
        .enumerate()
        .filter(|(_, window)| *window == wide)
        .map(|(at, _)| at)
        .collect::<Vec<_>>();
    assert_eq!(
        positions.len(),
        1,
        "exact namespace-0 numeric 2258 in the read fixture"
    );
    let at = *positions.first().expect("one identifier");
    let end = at.checked_add(wide.len()).expect("bounded offset");
    let mut out = bytes.get(..at).expect("prefix").to_vec();
    out.extend_from_slice(&[1, 0, 0xD2, 0x08]);
    out.extend_from_slice(bytes.get(end..).expect("suffix"));
    let size = u32::try_from(out.len()).expect("bounded frame");
    out.get_mut(4..8)
        .expect("header")
        .copy_from_slice(&size.to_le_bytes());
    out
}

fn rendered(vectors: &[Vector]) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut provenance="# S93: pinned Apache PLC4X suites. ASF headers retained.\n# Original capture origin attributed to Milo: UNVERIFIED-capture.\n# Raw vectors copied verbatim; driver bytes derive implicit lengths independently.\n# No live credentials/private keys identified; anonymous historical test data.\n".to_owned();
    for (file, blob, digest) in SUITES {
        writeln!(&mut provenance,"\n[[file]]\npath = \"{file}\"\norigin = \"{SOURCE}/{file}@{COMMIT}\"\nlicense = \"Apache-2.0\"\nnote = \"git blob {blob}; SHA-256 {digest}; UNVERIFIED-capture Milo origin\"").unwrap();
    }
    for vector in vectors {
        let mut expected = vector.expected.clone();
        if vector.name == CANONICAL_READ {
            expected.as_object_mut().expect("metadata object").insert(
                "reencodes_as".to_owned(),
                json!(hex(&canonical_read(&vector.bytes))),
            );
        }
        files.insert(
            format!("{}.hex", vector.name),
            format!("# {}\n{}\n", vector.origin, hex(&vector.bytes)),
        );
        files.insert(
            format!("{}.expected.json", vector.name),
            format!("{}\n", serde_json::to_string_pretty(&expected).unwrap()),
        );
        writeln!(&mut provenance,"\n[[vector]]\nname = {}\nsource = \"upstream_fixture\"\norigin = {}\nlicense = \"Apache-2.0\"\nnote = \"UNVERIFIED-capture; derived XML bytes use serializer-computed implicit lengths; inspect expected.json adjustments. Disabled services are listed in opcua_upstream.rs and must be enabled by S123.\"",serde_json::to_string(&vector.name).expect("quoted TOML name"),serde_json::to_string(&vector.origin).expect("quoted TOML origin")).unwrap();
    }
    files.insert("PROVENANCE.toml".to_owned(), provenance);
    files
}

#[test]
fn opcua_upstream_adapter_reproduces_the_complete_reviewed_corpus() {
    let vectors = corpus();
    let files = rendered(&vectors);
    let dir = directory();
    if std::env::var_os(REGENERATE).is_some() {
        for (name, text) in &files {
            let path = dir.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
    }
    let mut paths = vec![dir.clone()];
    let mut actual = BTreeSet::new();
    while let Some(path) = paths.pop() {
        for entry in fs::read_dir(path).expect("fixture directory") {
            let path = entry.expect("fixture entry").path();
            if path.is_dir() {
                paths.push(path);
            } else {
                actual.insert(
                    path.strip_prefix(&dir)
                        .expect("relative path")
                        .to_path_buf(),
                );
            }
        }
    }
    let expected = files
        .keys()
        .map(PathBuf::from)
        .chain(SUITES.iter().map(|(file, _, _)| PathBuf::from(file)))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual, expected,
        "exact reviewed corpus; remove stale files"
    );
    for (name, text) in &files {
        assert_eq!(
            &read(dir.join(name)),
            text,
            "regenerate {name} deliberately with {REGENERATE}"
        );
    }
    let loaded = load_dir(&dir).unwrap();
    assert_eq!(loaded.len(), 23);
    assert_eq!(
        loaded
            .iter()
            .map(|vector| vector.name.as_str())
            .collect::<BTreeSet<_>>(),
        vectors.iter().map(|vector| vector.name.as_str()).collect()
    );
    let provenance = load_provenance(&dir).unwrap();
    assert_eq!(provenance.file.len(), 2);
    for vector in &provenance.vector {
        assert!(vector.note.as_ref().unwrap().contains("UNVERIFIED-capture"));
    }
}

#[test]
fn opcua_upstream_retained_suites_match_the_pinned_fingerprints() {
    let manifest: toml::Value = toml::from_str(&read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../upstream/manifest.toml"),
    ))
    .unwrap();
    for (file, blob, digest) in SUITES {
        let bytes = fs::read(directory().join(file)).unwrap();
        assert_eq!(
            hex(ring::digest::digest(&ring::digest::SHA256, &bytes).as_ref()),
            digest
        );
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .contains("Licensed to the Apache Software Foundation")
        );
        let fixture = manifest
            .get("fixture")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| {
                entry.get("vendored_path").unwrap().as_str()
                    == Some(format!("fixtures/opcua/upstream/{file}").as_str())
            })
            .unwrap();
        assert_eq!(fixture.get("git_blob").unwrap().as_str(), Some(blob));
        assert_eq!(fixture.get("sha256").unwrap().as_str(), Some(digest));
        assert_eq!(fixture.get("commit").unwrap().as_str(), Some(COMMIT));
        assert_eq!(fixture.get("protocol").unwrap().as_str(), Some("opcua"));
        assert_eq!(fixture.get("license").unwrap().as_str(), Some("Apache-2.0"));
        assert_eq!(
            fixture.get("upstream_path").unwrap().as_str(),
            Some(format!("{SOURCE}/{file}").as_str())
        );
    }
}

#[expect(
    clippy::unwrap_in_result,
    reason = "encoding ids come from the pinned positive fixture metadata"
)]
fn service(
    bytes: &[u8],
    expected_id: i32,
    includes_id: bool,
) -> std::result::Result<Vec<u8>, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    if includes_id {
        let id = NodeId::parse(&mut reader, Depth::new())?;
        assert_eq!(id, NodeId::numeric(0, u32::try_from(expected_id).unwrap()));
    }
    let service = Service::parse(&mut reader, Depth::new(), expected_id)?;
    reader.expect_eof()?;
    let mut writer = BoundedWriter::new(65535);
    if includes_id {
        NodeId::numeric(0, u32::try_from(expected_id).unwrap())
            .serialize(&mut writer, Depth::new())?;
    }
    service.serialize(&mut writer, Depth::new())?;
    Ok(writer.into_bytes())
}
#[expect(
    clippy::unwrap_in_result,
    reason = "carrier frames must be valid; only disabled generated-body errors enter the refusal list"
)]
fn round_trip(
    bytes: &[u8],
    expected: &Value,
    name: &str,
) -> std::result::Result<Vec<u8>, ParseError> {
    let id = expected
        .get("encoding_id")
        .unwrap()
        .as_i64()
        .map(|id| i32::try_from(id).unwrap());
    let mut reader = BoundedReader::new(bytes);
    let mut writer = BoundedWriter::new(65535);
    match expected.get("root_type").unwrap().as_str().unwrap() {
        "ExpandedNodeId" => {
            let value = ExpandedNodeId::parse(&mut reader, Depth::new())?;
            reader.expect_eof()?;
            value.serialize(&mut writer, Depth::new())?;
        }
        "ExtensionObjectEncodingMask" => {
            let mask = reader.read_u8()?;
            reader.expect_eof()?;
            assert_eq!(mask, 1);
            writer.write_u8(mask)?;
        }
        "ExtensionObject" => {
            let value = ExtensionObject::parse(&mut reader, Depth::new())?;
            reader.expect_eof()?;
            assert_eq!(matches!(value.body, ExtensionBody::Raw(_)), name == OPAQUE);
            value.serialize(&mut writer, Depth::new())?;
        }
        "ExtensionObjectDefinition" => return service(bytes, id.unwrap(), false),
        "MessagePDU" => {
            let mut framer = OpcuaFramer::new(65535, ResourceLimits::builder().build().unwrap());
            let mut input = BytesMut::from(bytes);
            let mut frame = framer.decode(&mut input).unwrap().unwrap();
            assert!(input.is_empty());
            let mut encoded = BytesMut::new();
            framer.encode(&frame, &mut encoded).unwrap();
            assert_eq!(encoded.as_ref(), bytes, "framer {name}");
            let body = match &mut frame.body {
                Body::Open(chunk) => Some(&mut chunk.body),
                Body::Message(chunk) | Body::Close(chunk) => Some(&mut chunk.body),
                Body::Hello(_) | Body::Acknowledge(_) | Body::Error(_) => None,
            };
            if let Some(body) = body {
                *body = service(body, id.unwrap(), true)?.into();
            }
            encoded.clear();
            framer.encode(&frame, &mut encoded).unwrap();
            return Ok(encoded.to_vec());
        }
        kind => panic!("unhandled root type {kind}"),
    }
    Ok(writer.into_bytes())
}

#[test]
fn opcua_upstream_vectors_round_trip_or_match_the_exact_disabled_service_list() {
    let loaded = load_dir(&directory()).unwrap();
    let mut refused = BTreeSet::new();
    let mut supported = 0;
    for vector in loaded {
        match round_trip(
            &vector.bytes,
            vector.expected.as_ref().unwrap(),
            &vector.name,
        ) {
            Ok(bytes) => {
                if vector.name == CANONICAL_READ {
                    let expected = vector
                        .expected
                        .as_ref()
                        .expect("metadata")
                        .get("reencodes_as")
                        .expect("named canonical form")
                        .as_str()
                        .expect("hex");
                    assert_eq!(bytes, load_hex(expected).expect("canonical bytes"));
                    assert_eq!(bytes, canonical_read(&vector.bytes));
                } else {
                    assert_eq!(bytes, vector.bytes, "{}", vector.name);
                }
                assert!(
                    !REFUSALS.iter().any(|(name, _, _)| *name == vector.name),
                    "remove newly enabled service from REFUSALS: {}",
                    vector.name
                );
                supported += 1;
            }
            Err(ParseError::InvalidDiscriminator { field, value }) => {
                let (_, id, kind) = REFUSALS
                    .iter()
                    .find(|(name, _, _)| *name == vector.name)
                    .unwrap_or_else(|| panic!("unlisted refusal {}: {field}/{value}", vector.name));
                assert_eq!(
                    field,
                    format!("ExtensionObjectDefinition.{kind} (disabled)")
                );
                assert_eq!(value, u64::try_from(*id).unwrap());
                refused.insert(vector.name);
            }
            Err(error) => panic!("unexpected refusal {}: {error}", vector.name),
        }
    }
    assert_eq!(supported, 18);
    assert_eq!(
        refused,
        REFUSALS
            .iter()
            .map(|(name, _, _)| (*name).to_owned())
            .collect()
    );
}

// # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-HELLO-ACK
#[test]
fn opcua_upstream_vector_hello_acknowledge_decodes() {
    let loaded = load_dir(&directory()).unwrap();
    for name in [
        "driver/send-opcua-hello-packet",
        "driver/receive-acknowledgement-response",
    ] {
        let vector = loaded.iter().find(|vector| vector.name == name).unwrap();
        assert_eq!(
            round_trip(&vector.bytes, vector.expected.as_ref().unwrap(), name).unwrap(),
            vector.bytes
        );
    }
}

#[test]
fn opcua_upstream_adapter_rejects_invalid_annotations_and_partial_fields() {
    for (xml, error) in [
        (r#"<n dataType="uint" bitLength="0">0</n>"#, "integer width"),
        (
            r#"<n dataType="uint" bitLength="9">0</n>"#,
            "unsupported integer width",
        ),
        (
            r#"<n dataType="uint" bitLength="8">-1</n>"#,
            "outside declared width",
        ),
        (
            r#"<n dataType="int" bitLength="64">9223372036854775808</n>"#,
            "outside declared width",
        ),
        (r#"<n dataType="bit" bitLength="2">true</n>"#, "bit width"),
        (
            r#"<n dataType="bit" bitLength="1">maybe</n>"#,
            "invalid bit",
        ),
        (
            r#"<n dataType="uint" bitLength="8">x</n>"#,
            "invalid integer",
        ),
        (r#"<n dataType="uint">0</n>"#, "missing bitLength"),
        (
            r#"<n dataType="uint" bitLength="x">0</n>"#,
            "invalid bitLength",
        ),
        (
            r#"<n dataType="string" bitLength="16" encoding="UTF-8">a</n>"#,
            "String bitLength mismatch",
        ),
        (r#"<n dataType="byte" bitLength="8">FF</n>"#, "needs 0x"),
        (
            r#"<n dataType="byte" bitLength="16">0xFF</n>"#,
            "byte bitLength mismatch",
        ),
        (r#"<n dataType="byte" bitLength="8">0xGG</n>"#, "hex"),
        (
            r#"<n dataType="float" bitLength="32">1.0</n>"#,
            "unsupported XML leaf",
        ),
        (
            r#"<n dataType="float" bitLength="64">wrong</n>"#,
            "invalid float",
        ),
        (
            r#"<n dataType="unknown" bitLength="8">0</n>"#,
            "unsupported XML leaf",
        ),
        (r"<n>untyped</n>", "untyped text leaf"),
        (
            r#"<n dataType="bit" bitLength="1">true</n>"#,
            "unfinished bit field",
        ),
        (
            r#"<n><a dataType="uint" bitLength="7">1</a><b dataType="uint" bitLength="2">1</b></n>"#,
            "crosses byte boundary",
        ),
        (
            r#"<n><a dataType="bit" bitLength="1">true</a><b dataType="uint" bitLength="8">1</b></n>"#,
            "unaligned byte field",
        ),
        (r"<PascalString/>", "missing sLength"),
        (
            r"<PascalString><sLength/><sLength/></PascalString>",
            "duplicate sLength",
        ),
        (
            r#"<PascalString><sLength dataType="int" bitLength="16">0</sLength></PascalString>"#,
            "length shape",
        ),
        (
            r#"<PascalString><sLength dataType="int" bitLength="32">-2</sLength></PascalString>"#,
            "invalid PascalString length",
        ),
        (
            r#"<PascalString><sLength dataType="int" bitLength="32">-1</sLength><stringValue dataType="string" bitLength="8" encoding="UTF-8">x</stringValue></PascalString>"#,
            "null String with content",
        ),
        (
            r#"<PascalString><sLength dataType="int" bitLength="32">1</sLength><stringValue dataType="byte" bitLength="8">x</stringValue></PascalString>"#,
            "unsupported String shape",
        ),
        (
            r#"<MessagePDU><totalLength dataType="uint" bitLength="32">4</totalLength></MessagePDU>"#,
            "missing frame header",
        ),
    ] {
        let document = Document::parse(xml).expect("valid XML syntax");
        assert!(
            encode(document.root_element())
                .expect_err("invalid annotated model")
                .contains(error),
            "{xml}: expected {error}"
        );
    }
}

#[test]
fn opcua_upstream_implicit_length_adjustments_are_an_exact_closed_list() {
    let actual = corpus()
        .into_iter()
        .filter_map(|vector| {
            let adjustments = vector
                .expected
                .get("adjustments")
                .expect("metadata")
                .as_array()
                .expect("array");
            if adjustments.is_empty() {
                None
            } else {
                Some((
                    vector.name,
                    adjustments
                        .iter()
                        .map(|value| value.as_str().expect("adjustment").to_owned())
                        .collect::<Vec<_>>(),
                ))
            }
        })
        .collect::<BTreeMap<_, _>>();
    let expected = [
        (
            "driver/receive-acknowledgement-response",
            vec!["totalLength: 24 -> 28"],
        ),
        (
            "driver/send-create-session-request",
            vec![
                "sLength: 78 -> 61",
                "bitLength: 624 -> 488",
                "totalLength: 310 -> 293",
            ],
        ),
        (
            "driver/receive-create-session-response",
            vec![
                "sLength: 25 -> 14",
                "bitLength: 200 -> 112",
                "totalLength: 6450 -> 1867",
            ],
        ),
    ]
    .into_iter()
    .map(|(name, adjustments)| {
        (
            name.to_owned(),
            adjustments.into_iter().map(str::to_owned).collect(),
        )
    })
    .collect();
    assert_eq!(actual, expected);
}

// # Compatibility: docs/parity/opcua.md#OPCUA-DIV-NUMERIC-NODE-ID-ENCODING
#[test]
fn opcua_upstream_read_numeric_node_id_has_the_documented_canonical_form() {
    let loaded = load_dir(&directory()).expect("corpus");
    let vector = loaded
        .iter()
        .find(|vector| vector.name == CANONICAL_READ)
        .expect("read request");
    let canonical = round_trip(
        &vector.bytes,
        vector.expected.as_ref().expect("metadata"),
        &vector.name,
    )
    .expect("read decodes");
    assert_eq!(vector.bytes.len(), 131);
    assert_eq!(canonical.len(), 128);
    let mut framer = OpcuaFramer::new(65535, ResourceLimits::builder().build().expect("limits"));
    for bytes in [&vector.bytes, &canonical] {
        let frame = framer
            .decode(&mut BytesMut::from(bytes.as_slice()))
            .expect("frame")
            .expect("complete");
        let Body::Message(chunk) = frame.body else {
            panic!("MSG")
        };
        let mut reader = BoundedReader::new(&chunk.body);
        assert_eq!(
            NodeId::parse(&mut reader, Depth::new()).expect("service id"),
            NodeId::numeric(0, 631)
        );
        let Service::ReadRequest {
            nodes_to_read,
            max_age,
            ..
        } = Service::parse(&mut reader, Depth::new(), 631).expect("read")
        else {
            panic!("ReadRequest")
        };
        let nodes = nodes_to_read.expect("nodes");
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes.first().expect("one node").node_id,
            NodeId::numeric(0, 2258)
        );
        assert_eq!(max_age, 0.0);
        reader.expect_eof().expect("complete service");
    }
    assert_eq!(
        round_trip(
            &canonical,
            vector.expected.as_ref().expect("metadata"),
            &vector.name
        )
        .expect("canonical read"),
        canonical
    );
}
