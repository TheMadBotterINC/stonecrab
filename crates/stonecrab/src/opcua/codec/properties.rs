//! Property tests over the built-in types: any value written is read back equal and measures
//! as long as it is written, and no bytes make a parser panic or accept what it cannot write
//! back.

use proptest::prelude::*;

use crate::codec::BoundedReader;
use crate::opcua::codec::support::{Builtin, decode, encode};
use crate::opcua::codec::{
    DateTime, Depth, ExpandedNodeId, Guid, Identifier, LocalizedText, NodeId, QualifiedName,
    StatusCode, UaByteString, UaString,
};

fn ua_string() -> impl Strategy<Value = UaString> {
    prop::option::of(prop_oneof![
        any::<String>().prop_map(String::into_bytes),
        prop::collection::vec(any::<u8>(), 0..64),
    ])
    .prop_map(UaString)
}

fn ua_byte_string() -> impl Strategy<Value = UaByteString> {
    prop::option::of(prop::collection::vec(any::<u8>(), 0..64)).prop_map(UaByteString)
}

fn guid() -> impl Strategy<Value = Guid> {
    (any::<u32>(), any::<u16>(), any::<u16>(), any::<[u8; 8]>()).prop_map(
        |(data1, data2, data3, data4)| Guid {
            data1,
            data2,
            data3,
            data4,
        },
    )
}

/// Numbers that sit on both sides of the limits of the compact encodings.
fn edge_u32() -> impl Strategy<Value = u32> {
    prop_oneof![
        0..4_u32,
        253..259_u32,
        65_533..65_539_u32,
        Just(u32::MAX),
        any::<u32>()
    ]
}

fn node_id() -> impl Strategy<Value = NodeId> {
    let namespace =
        edge_u32().prop_map(|number| u16::try_from(number % 70_000).unwrap_or(u16::MAX));
    let identifier = prop_oneof![
        edge_u32().prop_map(Identifier::Numeric),
        ua_string().prop_map(Identifier::String),
        guid().prop_map(Identifier::Guid),
        ua_byte_string().prop_map(Identifier::ByteString),
    ];
    (namespace, identifier).prop_map(|(namespace, identifier)| NodeId {
        namespace,
        identifier,
    })
}

fn expanded_node_id() -> impl Strategy<Value = ExpandedNodeId> {
    (
        node_id(),
        prop::option::of(ua_string()),
        prop::option::of(any::<u32>()),
    )
        .prop_map(|(node_id, namespace_uri, server_index)| ExpandedNodeId {
            node_id,
            namespace_uri,
            server_index,
        })
}

fn qualified_name() -> impl Strategy<Value = QualifiedName> {
    (any::<u16>(), ua_string()).prop_map(|(namespace_index, name)| QualifiedName {
        namespace_index,
        name,
    })
}

fn localized_text() -> impl Strategy<Value = LocalizedText> {
    (prop::option::of(ua_string()), prop::option::of(ua_string()))
        .prop_map(|(locale, text)| LocalizedText { locale, text })
}

/// Writes `value`, reads it back, and checks the measured length.
fn round_trips<T: Builtin>(value: &T) -> Result<(), TestCaseError> {
    let bytes = encode(value);
    prop_assert_eq!(value.length_with(Depth::new()), Ok(bytes.len()));
    let decoded = decode::<T>(&bytes);
    prop_assert_eq!(decoded.as_ref(), Ok(value));
    Ok(())
}

/// What `bytes` parse to, if anything, must write back to the bytes it consumed, or to a
/// compact form of them that parses to the same value.
fn parse_is_stable<T: Builtin>(bytes: &[u8]) -> Result<(), TestCaseError> {
    let mut reader = BoundedReader::new(bytes);
    if let Ok(value) = T::parse_with(&mut reader, Depth::new()) {
        let consumed = bytes.len().saturating_sub(reader.remaining());
        let written = encode(&value);
        prop_assert!(
            written.len() <= consumed,
            "a value never grows when written back"
        );
        let decoded = decode::<T>(&written);
        prop_assert_eq!(decoded.as_ref(), Ok(&value));
    }
    Ok(())
}

proptest! {
    // covers: SPEC mandatory-failure-cases / encode then decode returns the value
    #[test]
    fn opcua_prop_builtin_round_trip(
        string in ua_string(),
        byte_string in ua_byte_string(),
        guid in guid(),
        ticks in any::<i64>(),
        status in any::<u32>(),
        node in node_id(),
        expanded in expanded_node_id(),
        name in qualified_name(),
        text in localized_text(),
    ) {
        round_trips(&string)?;
        round_trips(&byte_string)?;
        round_trips(&guid)?;
        round_trips(&DateTime(ticks))?;
        round_trips(&StatusCode(status))?;
        round_trips(&node)?;
        round_trips(&expanded)?;
        round_trips(&name)?;
        round_trips(&text)?;
    }

    // covers: SPEC mandatory-failure-cases / arbitrary bytes never panic a parser
    #[test]
    fn opcua_prop_builtin_parse_never_panics_and_is_stable(
        bytes in prop::collection::vec(any::<u8>(), 0..96),
    ) {
        parse_is_stable::<UaString>(&bytes)?;
        parse_is_stable::<UaByteString>(&bytes)?;
        parse_is_stable::<Guid>(&bytes)?;
        parse_is_stable::<DateTime>(&bytes)?;
        parse_is_stable::<StatusCode>(&bytes)?;
        parse_is_stable::<NodeId>(&bytes)?;
        parse_is_stable::<ExpandedNodeId>(&bytes)?;
        parse_is_stable::<QualifiedName>(&bytes)?;
        parse_is_stable::<LocalizedText>(&bytes)?;
    }
}
