use super::*;
use crate::codec::BoundedReader;
use crate::opcua::codec::{
    DateTime, DiagnosticInfo, ExpandedNodeId, ExtensionBody, ExtensionObject, LocalizedText,
    QualifiedName, StatusCode, UaByteString, XmlElement,
};

fn fresh_mapping(value: Variant, target: OpcuaDataType) -> Result<PlcValue> {
    let mut budget = MAX_VALUE_NODES;
    decode(value, target, &mut budget)
}
fn mapped(value: Variant) -> PlcValue {
    fresh_mapping(value, OpcuaDataType::Null).unwrap()
}
fn array(array: Array, dimensions: Option<Vec<i32>>) -> Variant {
    Variant {
        value: VariantValue::Array(array),
        dimensions,
    }
}
fn ints(values: &[i32]) -> PlcValue {
    PlcValue::List(values.iter().copied().map(PlcValue::I32).collect())
}
fn overriden(raw: i64, target: OpcuaDataType) -> PlcValue {
    fresh_mapping(Variant::scalar(Scalar::Int64(raw)), target).unwrap()
}
fn datetime(
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
    nano: u32,
) -> PlcValue {
    PlcValue::DateTime(PlcDateTime::new(
        PlcDate::from_ymd(year, month, day).unwrap(),
        PlcTimeOfDay::from_hms_nano(hour, minute, second, nano).unwrap(),
    ))
}

#[test]
fn opcua_a_scalar_variant_decodes_to_a_scalar() {
    assert_eq!(
        mapped(Variant::scalar(Scalar::Int32(42))),
        PlcValue::I32(42)
    );
    assert_eq!(mapped(Variant::default()), PlcValue::Null);
}
#[test]
fn opcua_an_array_variant_decodes_to_a_list() {
    assert_eq!(
        mapped(array(Array::Int32(Some(vec![1, 2, 3])), None)),
        ints(&[1, 2, 3])
    );
}
#[test]
fn opcua_a_one_element_array_stays_a_list() {
    assert_eq!(
        mapped(array(Array::Int32(Some(vec![42])), None)),
        ints(&[42])
    );
    assert_eq!(
        mapped(array(Array::Int32(Some(vec![42])), Some(vec![1, 1]))),
        PlcValue::List(vec![ints(&[42])])
    );
}
#[test]
fn opcua_null_and_empty_mapped_arrays_remain_distinct() {
    let mut budget = 4;
    assert_eq!(
        decode(
            array(Array::Int32(None), None),
            OpcuaDataType::Null,
            &mut budget
        )
        .unwrap(),
        PlcValue::Null
    );
    assert_eq!(
        decode(
            array(Array::Int32(Some(vec![])), None),
            OpcuaDataType::Null,
            &mut budget
        )
        .unwrap(),
        ints(&[])
    );
    assert_eq!(budget, 2);
}
#[test]
fn opcua_a_two_dimensional_array_decodes_to_nested_lists() {
    assert_eq!(
        mapped(array(
            Array::Int32(Some(vec![1, 2, 3, 4, 5, 6])),
            Some(vec![2, 3])
        )),
        PlcValue::List(vec![ints(&[1, 2, 3]), ints(&[4, 5, 6])])
    );
}
#[test]
fn opcua_a_3_by_2_array_decodes_to_three_lists_of_two() {
    assert_eq!(
        mapped(array(
            Array::Int32(Some(vec![1, 2, 3, 4, 5, 6])),
            Some(vec![3, 2])
        )),
        PlcValue::List(vec![ints(&[1, 2]), ints(&[3, 4]), ints(&[5, 6])])
    );
}
#[test]
fn opcua_three_dimensional_matrices_and_zero_axes_keep_their_shape() {
    assert_eq!(
        mapped(array(
            Array::Int32(Some((1..=8).collect())),
            Some(vec![2, 2, 2])
        )),
        PlcValue::List(vec![
            PlcValue::List(vec![ints(&[1, 2]), ints(&[3, 4])]),
            PlcValue::List(vec![ints(&[5, 6]), ints(&[7, 8])])
        ])
    );
    assert_eq!(
        mapped(array(Array::Int32(Some(vec![])), Some(vec![3, 0]))),
        PlcValue::List(vec![ints(&[]), ints(&[]), ints(&[])])
    );
    assert_eq!(
        mapped(array(
            Array::Int32(Some(vec![])),
            Some(vec![0, i32::MAX, 0])
        )),
        ints(&[])
    );
}
#[test]
fn opcua_declared_dimensions_inconsistent_with_the_count_do_not_allocate_or_panic() {
    for dimensions in [
        vec![],
        vec![-1, 0],
        vec![0],
        vec![2, 2],
        vec![i32::MAX, 0],
        vec![1; 65],
    ] {
        let mut budget = MAX_VALUE_NODES;
        let error = decode(
            array(Array::Int32(Some(vec![1])), Some(dimensions)),
            OpcuaDataType::Null,
            &mut budget,
        )
        .unwrap_err();
        assert!(matches!(
            error.kind(),
            ErrorKind::MalformedData | ErrorKind::ResourceLimit
        ));
        assert_eq!(budget, MAX_VALUE_NODES);
    }
    assert_eq!(
        shape(Some(&[i32::MAX, 0]), 0, MAX_VALUE_NODES)
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        shape(Some(&[64, 0]), 0, MAX_VALUE_NODES)
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        shape(None, usize::MAX, usize::MAX).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        shape(None, 1, 1).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
}
#[test]
fn opcua_value_node_and_rank_budgets_are_exact_and_shared_across_items() {
    let mut budget = 4;
    assert_eq!(
        decode(
            array(Array::Int32(Some(vec![1, 2])), None),
            OpcuaDataType::Null,
            &mut budget
        )
        .unwrap(),
        ints(&[1, 2])
    );
    assert_eq!(budget, 1);
    assert_eq!(
        decode(Variant::default(), OpcuaDataType::Null, &mut budget).unwrap(),
        PlcValue::Null
    );
    assert_eq!(budget, 0);
    assert_eq!(
        decode(Variant::default(), OpcuaDataType::Null, &mut budget)
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(budget, 0);
    assert!(shape(Some(&[1; 64]), 1, 65).is_ok());
    assert!(shape(Some(&[1; 65]), 1, 100).is_err());
    assert!(shape(Some(&[3, 2]), 6, 9).is_err());
    assert_eq!(shape(Some(&[3, 2]), 6, 10).unwrap().1, 10);
    assert!(shape(Some(&[63, 0]), 0, 64).is_ok());
}

#[test]
fn opcua_date_time_variant_converts_ticks_to_a_utc_instant() {
    assert_eq!(
        mapped(Variant::scalar(Scalar::DateTime(DateTime(
            116_444_736_000_000_000
        )))),
        datetime(1970, 1, 1, 0, 0, 0, 0)
    );
    assert_eq!(
        mapped(Variant::scalar(Scalar::DateTime(DateTime(
            116_444_735_999_999_999
        )))),
        datetime(1969, 12, 31, 23, 59, 59, 999_999_900)
    );
    assert_eq!(
        mapped(Variant::scalar(Scalar::DateTime(DateTime(
            116_444_736_000_000_001
        )))),
        datetime(1970, 1, 1, 0, 0, 0, 100)
    );
}
#[test]
fn opcua_date_time_zero_and_max_do_not_overflow() {
    assert_eq!(
        mapped(Variant::scalar(Scalar::DateTime(DateTime(0)))),
        datetime(1601, 1, 1, 0, 0, 0, 0)
    );
    assert_eq!(
        mapped(Variant::scalar(Scalar::DateTime(DateTime(-1)))),
        datetime(1600, 12, 31, 23, 59, 59, 999_999_900)
    );
    for ticks in [i64::MIN, i64::MAX] {
        let PlcValue::DateTime(value) = mapped(Variant::scalar(Scalar::DateTime(DateTime(ticks))))
        else {
            panic!("datetime")
        };
        let nanos = i128::from(value.date().days_since_epoch())
            .saturating_mul(NANOS_PER_DAY)
            .saturating_add(i128::from(value.time().nanos_since_midnight()));
        assert_eq!(
            nanos.div_euclid(100).saturating_add(UA_UNIX_TICKS),
            i128::from(ticks)
        );
    }
}
#[test]
fn opcua_guid_variant_renders_the_canonical_text() {
    assert_eq!(
        mapped(Variant::scalar(Scalar::Guid(Guid::default()))),
        PlcValue::String("00000000-0000-0000-0000-000000000000".to_owned())
    );
}
#[test]
fn opcua_guid_fields_use_the_mixed_endian_layout() {
    let wire = [
        0x91, 0x2B, 0x96, 0x72, 0x75, 0xFA, 0xE6, 0x4A, 0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D, 0xAF,
        0x63,
    ];
    let value = Guid::parse(&mut BoundedReader::new(&wire), Depth::new()).unwrap();
    assert_eq!(guid_bytes(value), wire);
    assert_eq!(
        mapped(Variant::scalar(Scalar::Guid(value))),
        PlcValue::String("72962b91-fa75-4ae6-8d28-b404dc7daf63".to_owned())
    );
}
#[test]
fn opcua_node_id_variant_renders_an_address_that_parses_back() {
    for (node, identifier) in [
        (
            NodeId::numeric(0, u32::MAX),
            OpcuaIdentifier::Numeric(u32::MAX),
        ),
        (
            NodeId {
                namespace: 2,
                identifier: Identifier::String(UaString::new("plant;value[1]|a=b\\工場")),
            },
            OpcuaIdentifier::String("plant;value[1]|a=b\\工場".to_owned()),
        ),
        (
            NodeId {
                namespace: 2,
                identifier: Identifier::Guid(Guid::default()),
            },
            OpcuaIdentifier::Guid([0; 16]),
        ),
        (
            NodeId {
                namespace: 65535,
                identifier: Identifier::ByteString(UaByteString::new(vec![0, 1, 254, 255])),
            },
            OpcuaIdentifier::ByteString(vec![0, 1, 254, 255]),
        ),
    ] {
        let namespace = node.namespace;
        let PlcValue::String(text) = mapped(Variant::scalar(Scalar::NodeId(node))) else {
            panic!("text")
        };
        let address = OpcuaAddress::parse(&text).unwrap();
        assert_eq!(address.namespace(), namespace);
        assert_eq!(address.identifier(), &identifier);
        assert_eq!(address.to_string(), text);
    }
}
#[test]
fn opcua_unrepresentable_node_ids_and_invalid_utf8_fail_only_the_value() {
    for identifier in [
        Identifier::String(UaString::null()),
        Identifier::String(UaString::new("")),
        Identifier::String(UaString::new("a\nb")),
        Identifier::String(UaString::new("x".repeat(8192))),
        Identifier::String(UaString::from_bytes(vec![0xFF])),
        Identifier::ByteString(UaByteString::null()),
    ] {
        assert!(
            fresh_mapping(
                Variant::scalar(Scalar::NodeId(NodeId {
                    namespace: 2,
                    identifier
                })),
                OpcuaDataType::Null
            )
            .is_err()
        );
    }
    assert_eq!(
        fresh_mapping(
            Variant::scalar(Scalar::String(UaString::from_bytes(vec![0xFF]))),
            OpcuaDataType::Null
        )
        .unwrap_err()
        .kind(),
        ErrorKind::MalformedData
    );
}
#[test]
fn opcua_qualified_name_renders_ns_and_name() {
    assert_eq!(
        mapped(Variant::scalar(Scalar::QualifiedName(QualifiedName {
            namespace_index: 2,
            name: UaString::new("工場")
        }))),
        PlcValue::String("ns=2;s=工場".to_owned())
    );
    assert_eq!(
        mapped(Variant::scalar(Scalar::QualifiedName(
            QualifiedName::default()
        ))),
        PlcValue::Null
    );
}
#[test]
fn opcua_status_code_variant_renders_name_and_code() {
    for (code, text) in [
        (0, "Good (0x00000000)"),
        (0x800A_0000, "BadTimeout (0x800A0000)"),
        (0xDEAD_BEEF, "unknown (0xDEADBEEF)"),
    ] {
        assert_eq!(
            mapped(Variant::scalar(Scalar::StatusCode(StatusCode(code)))),
            PlcValue::String(text.to_owned())
        );
    }
}

#[test]
fn opcua_time_suffix_reinterprets_an_int_as_a_duration() {
    for raw in [0, 42, -42] {
        assert_eq!(
            overriden(raw, OpcuaDataType::Time),
            PlcValue::SignedDuration(PlcSignedDuration::from_millis(raw).unwrap())
        );
        assert_eq!(
            overriden(raw, OpcuaDataType::LTime),
            PlcValue::SignedDuration(PlcSignedDuration::from_nanos(raw))
        );
    }
}
#[test]
fn opcua_date_suffix_applies_the_iec_epoch_offset() {
    assert_eq!(
        overriden(0, OpcuaDataType::Date),
        PlcValue::Date(PlcDate::from_ymd(1990, 1, 1).unwrap())
    );
    assert_eq!(
        overriden(-1, OpcuaDataType::Date),
        PlcValue::Date(PlcDate::from_ymd(1989, 12, 31).unwrap())
    );
    assert_eq!(
        overriden(86_400, OpcuaDataType::LDate),
        PlcValue::Date(PlcDate::from_ymd(1990, 1, 2).unwrap())
    );
    assert_eq!(
        overriden(-1, OpcuaDataType::LDate),
        PlcValue::Date(PlcDate::from_ymd(1989, 12, 31).unwrap())
    );
    assert_eq!(
        overriden(1, OpcuaDataType::DateAndTime),
        datetime(1990, 1, 1, 0, 0, 1, 0)
    );
    assert_eq!(
        overriden(-1, OpcuaDataType::DateAndTime),
        datetime(1989, 12, 31, 23, 59, 59, 0)
    );
    assert_eq!(
        overriden(1001, OpcuaDataType::LDateAndTime),
        datetime(1990, 1, 1, 0, 0, 1, 1_000_000)
    );
    assert_eq!(
        overriden(-1, OpcuaDataType::LDateAndTime),
        datetime(1989, 12, 31, 23, 59, 59, 999_000_000)
    );
    for suffix in [OpcuaDataType::TimeOfDay, OpcuaDataType::LTimeOfDay] {
        assert_eq!(
            overriden(1001, suffix),
            PlcValue::TimeOfDay(PlcTimeOfDay::from_hms_nano(0, 0, 1, 1_000_000).unwrap())
        );
    }
}
#[test]
fn opcua_a_temporal_value_is_not_reinterpreted_twice() {
    let expected = datetime(1970, 1, 1, 0, 0, 0, 100);
    for value in [
        expected,
        PlcValue::SignedDuration(PlcSignedDuration::from_nanos(1)),
        PlcValue::Date(PlcDate::from_days_since_epoch(0)),
        PlcValue::TimeOfDay(PlcTimeOfDay::from_nanos_since_midnight(0).unwrap()),
    ] {
        assert_eq!(temporal(value.clone(), OpcuaDataType::Date).unwrap(), value);
    }
    let variant = Variant::scalar(Scalar::DateTime(DateTime(116_444_736_000_000_001)));
    assert_eq!(
        fresh_mapping(variant, OpcuaDataType::Time).unwrap(),
        datetime(1970, 1, 1, 0, 0, 0, 100)
    );
}
#[test]
fn opcua_an_empty_value_stays_empty_under_a_type_suffix() {
    assert_eq!(
        fresh_mapping(Variant::default(), OpcuaDataType::Time).unwrap(),
        PlcValue::Null
    );
    assert_eq!(
        fresh_mapping(array(Array::Int32(None), None), OpcuaDataType::Time).unwrap(),
        PlcValue::Null
    );
    assert_eq!(
        fresh_mapping(array(Array::Int32(Some(vec![])), None), OpcuaDataType::Time).unwrap(),
        ints(&[])
    );
    assert_eq!(
        temporal(PlcValue::Null, OpcuaDataType::Time).unwrap(),
        PlcValue::Null
    );
}
#[test]
fn opcua_temporal_overrides_apply_to_each_matrix_leaf_without_lossy_coercion() {
    let value = fresh_mapping(
        array(Array::Int32(Some(vec![1, 2])), Some(vec![2, 1])),
        OpcuaDataType::LTime,
    )
    .unwrap();
    assert_eq!(
        value,
        PlcValue::List(vec![
            PlcValue::List(vec![PlcValue::SignedDuration(
                PlcSignedDuration::from_nanos(1)
            )]),
            PlcValue::List(vec![PlcValue::SignedDuration(
                PlcSignedDuration::from_nanos(2)
            )])
        ])
    );
    for value in [
        PlcValue::F64(1.5),
        PlcValue::F64(1.0),
        PlcValue::F32(f32::NAN),
        PlcValue::Bool(true),
        PlcValue::String("42".to_owned()),
        PlcValue::U64(u64::MAX),
    ] {
        assert_eq!(
            temporal(value, OpcuaDataType::Time).unwrap_err().kind(),
            ErrorKind::InvalidDataType
        );
    }
    for (raw, suffix) in [
        (i64::MAX, OpcuaDataType::Time),
        (i64::MAX, OpcuaDataType::Date),
        (i64::MAX, OpcuaDataType::LDate),
        (i64::MAX, OpcuaDataType::DateAndTime),
        (i64::MAX, OpcuaDataType::LDateAndTime),
        (-1, OpcuaDataType::TimeOfDay),
        (86_400_000, OpcuaDataType::LTimeOfDay),
    ] {
        assert_eq!(
            temporal(PlcValue::I64(raw), suffix).unwrap_err().kind(),
            ErrorKind::InvalidDataType
        );
    }
    assert_eq!(
        overriden(i64::MIN, OpcuaDataType::LTime),
        PlcValue::SignedDuration(PlcSignedDuration::from_nanos(i64::MIN))
    );
    assert_eq!(
        time(i128::MAX).unwrap_err().kind(),
        ErrorKind::InvalidDataType
    );
}

#[test]
fn opcua_all_mapped_array_kinds_use_the_same_scalar_mapping_and_keep_nulls() {
    macro_rules! check {
        ($kind:ident, $value:expr) => {{
            let value = $value;
            let expected = mapped(Variant::scalar(Scalar::$kind(value.clone())));
            assert_eq!(
                mapped(array(Array::$kind(Some(vec![value])), None)),
                PlcValue::List(vec![expected])
            );
            assert_eq!(mapped(array(Array::$kind(None), None)), PlcValue::Null);
            assert_eq!(
                mapped(array(Array::$kind(Some(vec![])), None)),
                PlcValue::List(vec![])
            );
        }};
    }
    check!(Boolean, true);
    check!(SByte, i8::MIN);
    check!(Byte, u8::MAX);
    check!(Int16, i16::MIN);
    check!(UInt16, u16::MAX);
    check!(Int32, i32::MIN);
    check!(UInt32, u32::MAX);
    check!(Int64, i64::MIN);
    check!(UInt64, u64::MAX);
    check!(Float, 1.5_f32);
    check!(Double, -0.0_f64);
    check!(String, UaString::new("工場\0 "));
    check!(DateTime, DateTime(116_444_736_000_000_001));
    check!(Guid, Guid::default());
    check!(XmlElement, XmlElement(UaString::new("<a/>")));
    check!(NodeId, NodeId::numeric(2, 42));
    check!(StatusCode, StatusCode(0x800A_0000));
    check!(
        QualifiedName,
        QualifiedName {
            namespace_index: 1,
            name: UaString::new("name")
        }
    );
    for bits in [0x7FC1_2345, 0x8000_0000] {
        let PlcValue::List(list) =
            mapped(array(Array::Float(Some(vec![f32::from_bits(bits)])), None))
        else {
            panic!("list")
        };
        let Some(PlcValue::F32(value)) = list.first() else {
            panic!("f32")
        };
        assert_eq!(value.to_bits(), bits);
    }
}
#[test]
fn opcua_an_unmapped_variant_kind_is_a_per_item_unsupported_error_naming_the_kind() {
    macro_rules! refused {
        ($kind:ident, $value:expr) => {{
            for value in [
                Variant::scalar(Scalar::$kind($value)),
                array(Array::$kind(None), None),
                array(Array::$kind(Some(vec![])), None),
            ] {
                let mut budget = MAX_VALUE_NODES;
                let problem = decode(value, OpcuaDataType::Null, &mut budget).unwrap_err();
                assert_eq!(problem.kind(), ErrorKind::Unsupported(Operation::Read));
                assert!(problem.to_string().contains(stringify!($kind)));
                assert_eq!(budget, MAX_VALUE_NODES);
            }
        }};
    }
    refused!(ByteString, UaByteString::null());
    refused!(ExpandedNodeId, ExpandedNodeId::local(NodeId::numeric(0, 1)));
    refused!(LocalizedText, LocalizedText::default());
    refused!(
        ExtensionObject,
        ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, 0)),
            body: ExtensionBody::Null
        }
    );
    refused!(DataValue, Box::default());
    refused!(Variant, Box::default());
    refused!(DiagnosticInfo, DiagnosticInfo::default());
}

#[test]
fn opcua_upstream_variant_mapping_cases_retain_null_and_time_semantics() {
    // OpcuaVariantToPlcValueTest: null, suffix-null, no-tag null, Int32 42,
    // unsupported DataValue/DiagnosticInfo/Variant/ExpandedNodeId, and Int32 42;TIME.
    assert_eq!(mapped(Variant::default()), PlcValue::Null);
    assert_eq!(
        fresh_mapping(Variant::default(), OpcuaDataType::Time).unwrap(),
        PlcValue::Null
    );
    assert_eq!(
        mapped(Variant::scalar(Scalar::Int32(42))),
        PlcValue::I32(42)
    );
    assert_eq!(
        fresh_mapping(Variant::scalar(Scalar::Int32(42)), OpcuaDataType::Time).unwrap(),
        PlcValue::SignedDuration(PlcSignedDuration::from_nanos(42_000_000))
    );
    for scalar in [
        Scalar::DataValue(Box::default()),
        Scalar::DiagnosticInfo(DiagnosticInfo::default()),
        Scalar::Variant(Box::default()),
        Scalar::ExpandedNodeId(ExpandedNodeId::local(NodeId::numeric(0, 1))),
    ] {
        assert!(fresh_mapping(Variant::scalar(scalar), OpcuaDataType::Null).is_err());
    }
}
