//! Property tests over the built-in types and the recursive types: any value written is read
//! back equal and measures as long as it is written, and no bytes make a parser panic or accept
//! what it cannot write back.

use proptest::prelude::*;

use crate::codec::BoundedReader;
use crate::opcua::codec::support::{Builtin, decode, encode};
use crate::opcua::codec::{
    Array, DataValue, DateTime, Depth, DiagnosticInfo, ExpandedNodeId, ExtensionBody,
    ExtensionObject, Guid, Identifier, LocalizedText, NodeId, QualifiedName, Scalar, StatusCode,
    UaByteString, UaString, Variant, XmlElement,
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

fn extension_object() -> impl Strategy<Value = ExtensionObject> {
    let body = prop_oneof![
        Just(ExtensionBody::Null),
        ua_byte_string().prop_map(ExtensionBody::Raw),
        ua_string().prop_map(ExtensionBody::Xml),
    ];
    (expanded_node_id(), body).prop_map(|(type_id, body)| ExtensionObject { type_id, body })
}

fn diagnostic_info() -> impl Strategy<Value = DiagnosticInfo> {
    let leaf = (
        prop::option::of(any::<i32>()),
        prop::option::of(any::<i32>()),
        prop::option::of(any::<i32>()),
        prop::option::of(any::<i32>()),
        prop::option::of(ua_string()),
        prop::option::of(any::<u32>().prop_map(StatusCode)),
    )
        .prop_map(
            |(
                symbolic_id,
                namespace_uri,
                locale,
                localized_text,
                additional_info,
                inner_status,
            )| {
                DiagnosticInfo {
                    symbolic_id,
                    namespace_uri,
                    locale,
                    localized_text,
                    additional_info,
                    inner_status,
                    inner: None,
                }
            },
        );
    leaf.prop_recursive(5, 16, 1, |inner| {
        (inner.clone(), inner).prop_map(|(mut outer, cause)| {
            outer.inner = Some(Box::new(cause));
            outer
        })
    })
}

/// A Variant of values that compare equal to themselves (no NaN).
fn variant() -> impl Strategy<Value = Variant> {
    let scalar = prop_oneof![
        any::<bool>().prop_map(Scalar::Boolean),
        any::<i32>().prop_map(Scalar::Int32),
        any::<u64>().prop_map(Scalar::UInt64),
        any::<f64>()
            .prop_filter("NaN is not equal to itself", |value| !value.is_nan())
            .prop_map(Scalar::Double),
        ua_string().prop_map(Scalar::String),
        ua_string().prop_map(|text| Scalar::XmlElement(XmlElement(text))),
        guid().prop_map(Scalar::Guid),
        ua_byte_string().prop_map(Scalar::ByteString),
        node_id().prop_map(Scalar::NodeId),
        any::<u32>().prop_map(|code| Scalar::StatusCode(StatusCode(code))),
        localized_text().prop_map(Scalar::LocalizedText),
        extension_object().prop_map(Scalar::ExtensionObject),
        diagnostic_info().prop_map(Scalar::DiagnosticInfo),
    ];
    let array = prop_oneof![
        prop::option::of(prop::collection::vec(any::<i16>(), 0..5)).prop_map(Array::Int16),
        prop::option::of(prop::collection::vec(ua_string(), 0..4)).prop_map(Array::String),
        prop::option::of(prop::collection::vec(expanded_node_id(), 0..3))
            .prop_map(Array::ExpandedNodeId),
    ];
    let leaf = prop_oneof![
        Just(Variant::default()),
        scalar.prop_map(Variant::scalar),
        array.prop_map(|values| Variant {
            value: crate::opcua::codec::VariantValue::Array(values),
            dimensions: None,
        }),
    ];
    leaf.prop_recursive(4, 24, 3, |inner| {
        prop_oneof![
            inner
                .clone()
                .prop_map(|value| Variant::scalar(Scalar::Variant(Box::new(value)))),
            data_value(inner.clone())
                .prop_map(|value| Variant::scalar(Scalar::DataValue(Box::new(value)))),
            prop::collection::vec(inner, 0..3).prop_map(|values| Variant {
                value: crate::opcua::codec::VariantValue::Array(Array::Variant(Some(values))),
                dimensions: None,
            }),
        ]
    })
}

fn data_value(value: impl Strategy<Value = Variant>) -> impl Strategy<Value = DataValue> {
    (
        prop::option::of(value),
        prop::option::of(any::<u32>().prop_map(StatusCode)),
        prop::option::of(any::<i64>().prop_map(DateTime)),
        prop::option::of(any::<u16>()),
        prop::option::of(any::<i64>().prop_map(DateTime)),
        prop::option::of(any::<u16>()),
    )
        .prop_map(
            |(
                value,
                status,
                source_timestamp,
                source_picoseconds,
                server_timestamp,
                server_picoseconds,
            )| DataValue {
                value,
                status,
                source_timestamp,
                source_picoseconds,
                server_timestamp,
                server_picoseconds,
            },
        )
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

/// What `bytes` parse to, if anything, must write back to at most the bytes it consumed, in a
/// form that parses to something that writes the same bytes again. (Equality of the values
/// is not asked for: a float may be NaN.)
fn parse_writes_back_stably<T: Builtin>(bytes: &[u8]) -> Result<(), TestCaseError> {
    let mut reader = BoundedReader::new(bytes);
    if let Ok(value) = T::parse_with(&mut reader, Depth::new()) {
        let consumed = bytes.len().saturating_sub(reader.remaining());
        let written = encode(&value);
        prop_assert!(
            written.len() <= consumed,
            "a value never grows when written back"
        );
        prop_assert_eq!(value.length_with(Depth::new()), Ok(written.len()));
        let again = decode::<T>(&written);
        prop_assert!(again.is_ok(), "what is written parses: {:?}", again);
        prop_assert_eq!(again.map(|value| encode(&value)), Ok(written));
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

proptest! {
    // covers: SPEC mandatory-failure-cases / encode then decode returns the value
    #[test]
    fn opcua_prop_recursive_types_round_trip(
        variant in variant(),
        data in data_value(variant()),
        info in diagnostic_info(),
        object in extension_object(),
    ) {
        round_trips(&variant)?;
        round_trips(&data)?;
        round_trips(&info)?;
        round_trips(&object)?;
    }

    // covers: SPEC mandatory-failure-cases / arbitrary bytes never panic a parser
    #[test]
    fn opcua_prop_recursive_types_parse_never_panics_and_is_stable(
        bytes in prop::collection::vec(any::<u8>(), 0..96),
    ) {
        parse_writes_back_stably::<Variant>(&bytes)?;
        parse_writes_back_stably::<DataValue>(&bytes)?;
        parse_writes_back_stably::<DiagnosticInfo>(&bytes)?;
        parse_writes_back_stably::<ExtensionObject>(&bytes)?;
    }

    // covers: SPEC mandatory-failure-cases / truncated and corrupted frames are refused
    #[test]
    fn opcua_prop_recursive_types_survive_a_cut_or_flipped_byte(
        variant in variant(),
        position in any::<prop::sample::Index>(),
        flip in 1..=u8::MAX,
        cut in any::<bool>(),
    ) {
        let mut bytes = encode(&variant);
        let at = position.index(bytes.len());
        if cut {
            bytes.truncate(at);
        } else if let Some(byte) = bytes.get_mut(at) {
            *byte ^= flip;
        }
        parse_writes_back_stably::<Variant>(&bytes)?;
        parse_writes_back_stably::<DataValue>(&bytes)?;
    }
}
