//! # Compatibility: docs/parity/opcua.md#OPCUA-CODEC-SERVICE-SUBSET
//!
//! First-party service-body vectors from the pinned model's field order and OPC UA Part 6.
//! Counts, nulls, enum values and little-endian fields are written independently of the
//! generated codec. Every enabled case has a complete body and every truncated prefix.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{
    Depth, ExpandedNodeId, ExtensionBody, ExtensionObject, Identifier, MAX_STRING_BYTES, NodeId,
    UaByteString, UaString,
};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition;

const NULL: [u8; 4] = [0xFF; 4];
const EMPTY_OBJECT: [u8; 3] = [0, 0, 0];

fn request_header() -> Vec<u8> {
    [
        vec![0, 0],
        vec![0; 8],
        42_u32.to_le_bytes().to_vec(),
        vec![0; 4],
        NULL.to_vec(),
        5000_u32.to_le_bytes().to_vec(),
        EMPTY_OBJECT.to_vec(),
    ]
    .concat()
}
fn response_header() -> Vec<u8> {
    [
        vec![0; 8],
        42_u32.to_le_bytes().to_vec(),
        vec![0; 4],
        vec![0],
        NULL.to_vec(),
        EMPTY_OBJECT.to_vec(),
    ]
    .concat()
}
fn application() -> Vec<u8> {
    [
        NULL.to_vec(),
        NULL.to_vec(),
        vec![0],
        1_u32.to_le_bytes().to_vec(),
        NULL.to_vec(),
        NULL.to_vec(),
        NULL.to_vec(),
    ]
    .concat()
}
fn token() -> Vec<u8> {
    [
        1_u32.to_le_bytes().to_vec(),
        2_u32.to_le_bytes().to_vec(),
        vec![0; 8],
        60_000_u32.to_le_bytes().to_vec(),
    ]
    .concat()
}
fn signature() -> Vec<u8> {
    [NULL, NULL].concat()
}
fn endpoint() -> Vec<u8> {
    [
        NULL.to_vec(),
        application(),
        NULL.to_vec(),
        1_u32.to_le_bytes().to_vec(),
        NULL.to_vec(),
        NULL.to_vec(),
        NULL.to_vec(),
        vec![0],
    ]
    .concat()
}
fn read_value() -> Vec<u8> {
    [
        vec![0, 0],
        13_u32.to_le_bytes().to_vec(),
        NULL.to_vec(),
        vec![0, 0],
        NULL.to_vec(),
    ]
    .concat()
}
fn envelope(id: u16, body: &[u8]) -> Vec<u8> {
    [
        vec![1, 0],
        id.to_le_bytes().to_vec(),
        vec![1],
        i32::try_from(body.len()).unwrap().to_le_bytes().to_vec(),
        body.to_vec(),
    ]
    .concat()
}

fn vectors() -> Vec<(&'static str, u16, Vec<u8>)> {
    vec![
        (
            "WriteValue",
            670,
            [
                vec![0, 0],
                13_u32.to_le_bytes().to_vec(),
                NULL.to_vec(),
                vec![1, 6, 42, 0, 0, 0],
            ]
            .concat(),
        ),
        (
            "WriteRequest",
            673,
            [request_header(), NULL.to_vec()].concat(),
        ),
        (
            "WriteResponse",
            676,
            [response_header(), NULL.to_vec(), NULL.to_vec()].concat(),
        ),
        ("RequestHeader", 391, request_header()),
        ("ResponseHeader", 394, response_header()),
        ("ServiceFault", 397, response_header()),
        (
            "GetEndpointsRequest",
            428,
            [
                request_header(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
            ]
            .concat(),
        ),
        (
            "GetEndpointsResponse",
            431,
            [response_header(), 1_i32.to_le_bytes().to_vec(), endpoint()].concat(),
        ),
        ("ChannelSecurityToken", 443, token()),
        (
            "OpenSecureChannelRequest",
            446,
            [
                request_header(),
                vec![0; 4],
                vec![0; 4],
                1_u32.to_le_bytes().to_vec(),
                NULL.to_vec(),
                60_000_u32.to_le_bytes().to_vec(),
            ]
            .concat(),
        ),
        (
            "OpenSecureChannelResponse",
            449,
            [response_header(), vec![0; 4], token(), NULL.to_vec()].concat(),
        ),
        ("CloseSecureChannelRequest", 452, request_header()),
        ("ApplicationDescription", 310, application()),
        (
            "UserTokenPolicy",
            306,
            [
                NULL.to_vec(),
                vec![0; 4],
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
            ]
            .concat(),
        ),
        ("EndpointDescription", 314, endpoint()),
        ("SignedSoftwareCertificate", 346, signature()),
        ("SignatureData", 458, signature()),
        (
            "CreateSessionRequest",
            461,
            [
                request_header(),
                application(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                60_000_f64.to_le_bytes().to_vec(),
                vec![0; 4],
            ]
            .concat(),
        ),
        (
            "CreateSessionResponse",
            464,
            [
                response_header(),
                vec![0, 0],
                vec![0, 0],
                60_000_f64.to_le_bytes().to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
                signature(),
                vec![0; 4],
            ]
            .concat(),
        ),
        ("AnonymousIdentityToken", 321, NULL.to_vec()),
        (
            "X509IdentityToken",
            327,
            [NULL.to_vec(), NULL.to_vec()].concat(),
        ),
        (
            "UserNameIdentityToken",
            324,
            [NULL.to_vec(), NULL.to_vec(), NULL.to_vec(), NULL.to_vec()].concat(),
        ),
        (
            "ActivateSessionRequest",
            467,
            [
                request_header(),
                signature(),
                NULL.to_vec(),
                NULL.to_vec(),
                envelope(321, &NULL),
                signature(),
            ]
            .concat(),
        ),
        (
            "ActivateSessionResponse",
            470,
            [
                response_header(),
                NULL.to_vec(),
                NULL.to_vec(),
                NULL.to_vec(),
            ]
            .concat(),
        ),
        (
            "CloseSessionRequest",
            473,
            [request_header(), vec![1]].concat(),
        ),
        ("CloseSessionResponse", 476, response_header()),
        ("ReadValueId", 628, read_value()),
        (
            "ReadRequest",
            631,
            [
                request_header(),
                0_f64.to_le_bytes().to_vec(),
                2_u32.to_le_bytes().to_vec(),
                1_i32.to_le_bytes().to_vec(),
                read_value(),
            ]
            .concat(),
        ),
        (
            "ReadResponse",
            634,
            [
                response_header(),
                1_i32.to_le_bytes().to_vec(),
                vec![1, 6, 42, 0, 0, 0],
                NULL.to_vec(),
            ]
            .concat(),
        ),
    ]
}

fn fixture(name: &str, negative: bool) -> Vec<u8> {
    let suffix = if negative { "-truncated" } else { "" };
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/opcua/services")
        .join(format!("{name}{suffix}.hex"));
    std::fs::read_to_string(path)
        .expect("first-party fixture")
        .split_whitespace()
        .map(|byte| u8::from_str_radix(byte, 16).expect("hex byte"))
        .collect()
}

fn parse(bytes: &[u8]) -> Result<ExtensionObject, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    let value = ExtensionObject::parse(&mut reader, Depth::new())?;
    reader.expect_eof()?;
    Ok(value)
}
fn encode(value: &ExtensionObject, depth: Depth) -> Result<Vec<u8>, ParseError> {
    let mut writer = BoundedWriter::new(MAX_STRING_BYTES);
    value.serialize(&mut writer, depth)?;
    Ok(writer.into_bytes())
}
fn object(type_id: ExpandedNodeId, body: Vec<u8>) -> ExtensionObject {
    ExtensionObject {
        type_id,
        body: ExtensionBody::Raw(UaByteString::new(body)),
    }
}
fn local(id: u32) -> ExpandedNodeId {
    ExpandedNodeId::local(NodeId::numeric(0, id))
}

#[test]
fn opcua_every_generated_service_has_positive_and_negative_vectors() {
    let all = vectors();
    assert_eq!(all.len(), 29, "S108 connection/read/write/identity closure");
    for (name, id, body) in all {
        assert_eq!(
            fixture(name, false),
            body,
            "retained independent fixture for {name}"
        );
        assert!(
            ExtensionObjectDefinition::parse(
                &mut BoundedReader::new(&fixture(name, true)),
                Depth::new(),
                i32::from(id)
            )
            .is_err(),
            "retained negative fixture for {name}"
        );
        let mut reader = BoundedReader::new(&body);
        let service = ExtensionObjectDefinition::parse(&mut reader, Depth::new(), i32::from(id))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        reader.expect_eof().unwrap();
        assert!(
            format!("{service:?}").starts_with(name),
            "correct generated case for {id}"
        );
        let mut writer = BoundedWriter::new(body.len());
        service.serialize(&mut writer, Depth::new()).unwrap();
        assert_eq!(
            writer.as_slice(),
            body,
            "independent wire vector for {name}"
        );
        for cut in 0..body.len() {
            let prefix = body.get(..cut).unwrap();
            assert!(
                ExtensionObjectDefinition::parse(
                    &mut BoundedReader::new(prefix),
                    Depth::new(),
                    i32::from(id)
                )
                .is_err(),
                "{name} prefix {cut} must fail"
            );
        }
        assert!(
            service
                .serialize(
                    &mut BoundedWriter::new(body.len().saturating_sub(1)),
                    Depth::new()
                )
                .is_err(),
            "{name} bounded writer"
        );
        let wire = envelope(id, &body);
        let typed = parse(&wire).unwrap();
        assert!(
            matches!(typed.body, ExtensionBody::Known(_)),
            "{name} is decoded"
        );
        assert_eq!(
            typed.length_in_bytes(Depth::new()),
            Ok(wire.len()),
            "{name} measured size"
        );
        assert_eq!(
            encode(&typed, Depth::new()).unwrap(),
            wire,
            "{name} envelope round trip"
        );
    }
}

#[test]
fn opcua_disabled_and_unknown_bodies_remain_raw_and_preserve_identity() {
    for id in [0_u32, 940, 100_000, u32::MAX] {
        let raw = object(local(id), vec![0xAA, 0xBB]);
        let wire = encode(&raw, Depth::new()).unwrap();
        assert_eq!(parse(&wire).unwrap(), raw, "unsupported id {id} survives");
    }
    let id = NodeId::numeric(0, 391);
    for type_id in [
        ExpandedNodeId::local(NodeId::numeric(2, 391)),
        ExpandedNodeId {
            node_id: id.clone(),
            namespace_uri: Some(UaString::new("urn:custom")),
            server_index: None,
        },
        ExpandedNodeId {
            node_id: id,
            namespace_uri: None,
            server_index: Some(2),
        },
        ExpandedNodeId::local(NodeId {
            namespace: 0,
            identifier: Identifier::String(UaString::new("391")),
        }),
    ] {
        let raw = object(type_id, vec![0xFF]);
        assert_eq!(
            parse(&encode(&raw, Depth::new()).unwrap()).unwrap(),
            raw,
            "custom identity never uses the namespace-zero table"
        );
    }
    for bytes in [UaByteString::null(), UaByteString::new(Vec::new())] {
        let raw = ExtensionObject {
            type_id: local(940),
            body: ExtensionBody::Raw(bytes),
        };
        assert_eq!(
            parse(&encode(&raw, Depth::new()).unwrap()).unwrap(),
            raw,
            "disabled body retains null/empty distinction"
        );
    }
}

#[test]
fn opcua_recognized_malformed_bodies_never_fall_back_to_raw() {
    assert!(
        matches!(
            parse(&envelope(391, &[])),
            Err(ParseError::Truncated { .. })
        ),
        "known empty body is malformed"
    );
    let mut trailing = request_header();
    trailing.push(0xAA);
    assert_eq!(
        parse(&envelope(391, &trailing)),
        Err(ParseError::TrailingBytes { remaining: 1 })
    );
    // ApplicationType 99 is an invalid enum inside a recognized ApplicationDescription.
    let invalid = [
        NULL.to_vec(),
        NULL.to_vec(),
        vec![0],
        99_u32.to_le_bytes().to_vec(),
    ]
    .concat();
    assert!(
        matches!(
            parse(&envelope(310, &invalid)),
            Err(ParseError::InvalidDiscriminator {
                field: "ApplicationType",
                ..
            })
        ),
        "an inner discriminator error is not an unknown body"
    );
    let enormous = [
        request_header(),
        vec![0; 8],
        vec![2, 0, 0, 0],
        i32::MAX.to_le_bytes().to_vec(),
    ]
    .concat();
    assert!(
        matches!(
            parse(&envelope(631, &enormous)),
            Err(ParseError::CountLimit { .. })
        ),
        "declared count is bounded before allocation"
    );
}

#[test]
fn opcua_known_body_recursion_uses_one_shared_budget() {
    let prefix = request_header();
    let prefix = prefix
        .get(..prefix.len().saturating_sub(EMPTY_OBJECT.len()))
        .unwrap();
    let mut body = request_header();
    for _ in 0..Depth::LIMIT {
        body = [prefix.to_vec(), envelope(391, &body)].concat();
    }
    assert!(
        matches!(
            parse(&envelope(391, &body)),
            Err(ParseError::RecursionLimit { .. })
        ),
        "generated-to-external recursion stays bounded"
    );
    let known = parse(&envelope(391, &request_header())).unwrap();
    let mut depth = Depth::new();
    for _ in 1..Depth::LIMIT {
        depth = depth.enter().unwrap();
    }
    assert!(
        matches!(
            known.length_in_bytes(depth),
            Err(ParseError::RecursionLimit { .. })
        ),
        "length shares the budget"
    );
    assert!(
        matches!(
            encode(&known, depth),
            Err(ParseError::RecursionLimit { .. })
        ),
        "encode shares the budget"
    );
}

#[test]
fn opcua_known_body_cannot_exceed_the_binary_body_budget() {
    let large = ExtensionObject {
        type_id: local(321),
        body: ExtensionBody::Known(Box::new(
            ExtensionObjectDefinition::AnonymousIdentityToken {
                policy_id: UaString::new("x".repeat(MAX_STRING_BYTES)),
            },
        )),
    };
    assert!(
        matches!(
            large.length_in_bytes(Depth::new()),
            Err(ParseError::OversizedLength { .. })
        ),
        "length must fit the bounded encoded body"
    );
    assert!(
        matches!(
            encode(&large, Depth::new()),
            Err(ParseError::OversizedLength { .. })
        ),
        "write uses the same body budget"
    );
}

#[test]
fn opcua_generated_external_recursion_fits_a_two_mib_stack() {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let prefix = request_header();
            let prefix = prefix
                .get(..prefix.len().saturating_sub(EMPTY_OBJECT.len()))
                .unwrap();
            let mut body = request_header();
            for _ in 2..Depth::LIMIT {
                body = [prefix.to_vec(), envelope(391, &body)].concat();
            }
            let wire = envelope(391, &body);
            let parsed = parse(&wire).expect("the complete shared recursion budget fits");
            assert_eq!(
                encode(&parsed, Depth::new()).unwrap(),
                wire,
                "deep known bodies round trip"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
