use super::*;
use crate::opcua::codec::{
    DateTime, DiagnosticInfo, ExpandedNodeId, ExtensionBody, ExtensionObject, Scalar, Variant,
    XmlElement,
};
use crate::{PlcValue, ReadRequest, ResourceLimits, Tag};
use std::sync::Arc;
use std::time::Duration;

fn data(scalar: Scalar) -> DataValue {
    DataValue {
        value: Some(Variant::scalar(scalar)),
        ..DataValue::default()
    }
}
fn mapped(data: DataValue) -> ReadItem {
    let mut budget = crate::opcua::value::MAX_VALUE_NODES;
    item(
        TagId::new("sample").unwrap(),
        data,
        Instant::now(),
        OpcuaDataType::Null,
        &mut budget,
    )
}

#[test]
fn opcua_each_integer_variant_decodes_to_its_width_and_sign() {
    for (scalar, expected) in [
        (Scalar::SByte(i8::MIN), PlcValue::I8(i8::MIN)),
        (Scalar::Byte(u8::MAX), PlcValue::U8(u8::MAX)),
        (Scalar::Int16(i16::MIN), PlcValue::I16(i16::MIN)),
        (Scalar::UInt16(u16::MAX), PlcValue::U16(u16::MAX)),
        (Scalar::Int32(i32::MIN), PlcValue::I32(i32::MIN)),
        (Scalar::UInt32(u32::MAX), PlcValue::U32(u32::MAX)),
        (Scalar::Int64(i64::MIN), PlcValue::I64(i64::MIN)),
        (Scalar::UInt64(u64::MAX), PlcValue::U64(u64::MAX)),
    ] {
        assert_eq!(mapped(data(scalar)).outcome().as_ref().unwrap(), &expected);
    }
}

#[test]
fn opcua_boolean_variant_decodes_to_bool() {
    for value in [false, true] {
        assert_eq!(
            mapped(data(Scalar::Boolean(value)))
                .outcome()
                .as_ref()
                .unwrap(),
            &PlcValue::Bool(value)
        );
    }
}

#[test]
fn opcua_float_and_double_variants_decode_bit_exact() {
    for bits in [
        0,
        0x8000_0000,
        0x7F80_0000,
        0xFF80_0000,
        0x7FC1_2345,
        0xFFC1_2345,
        1,
    ] {
        let item = mapped(data(Scalar::Float(f32::from_bits(bits))));
        let PlcValue::F32(value) = item.outcome().as_ref().unwrap() else {
            panic!("float")
        };
        assert_eq!(value.to_bits(), bits);
    }
    for bits in [
        0,
        0x8000_0000_0000_0000,
        0x7FF0_0000_0000_0000,
        0xFFF0_0000_0000_0000,
        0x7FF8_1234_5678_90AB,
        1,
    ] {
        let item = mapped(data(Scalar::Double(f64::from_bits(bits))));
        let PlcValue::F64(value) = item.outcome().as_ref().unwrap() else {
            panic!("double")
        };
        assert_eq!(value.to_bits(), bits);
    }
}

#[test]
fn opcua_string_variant_decodes_utf8_without_trimming_and_null_is_null() {
    for text in ["", "\u{FEFF}工場\0 value  "] {
        for scalar in [
            Scalar::String(UaString::new(text)),
            Scalar::XmlElement(XmlElement(UaString::new(text))),
        ] {
            assert_eq!(
                mapped(data(scalar)).outcome().as_ref().unwrap(),
                &PlcValue::String(text.to_owned())
            );
        }
    }
    assert_eq!(
        mapped(data(Scalar::String(UaString::null())))
            .outcome()
            .as_ref()
            .unwrap(),
        &PlcValue::Null
    );
    assert_eq!(
        mapped(data(Scalar::String(UaString::from_bytes(vec![0xFF]))))
            .outcome()
            .as_ref()
            .unwrap_err()
            .kind(),
        ErrorKind::MalformedData
    );
}

#[test]
fn opcua_a_null_variant_decodes_to_a_null_value() {
    for value in [None, Some(Variant::default())] {
        assert_eq!(
            mapped(DataValue {
                value,
                ..DataValue::default()
            })
            .outcome()
            .as_ref()
            .unwrap(),
            &PlcValue::Null
        );
    }
}

#[test]
fn opcua_bad_statuses_keep_their_classification_and_exact_number() {
    for (code, expected) in [
        (0x8034_0000, ErrorKind::NotFound),
        (0x8074_0000, ErrorKind::InvalidDataType),
        (0x803B_0000, ErrorKind::AccessDenied),
        (0x801F_0000, ErrorKind::AccessDenied),
        (0x8035_0000, ErrorKind::InvalidAddress),
        (0x8037_0000, ErrorKind::InvalidAddress),
        (0x8001_0000, ErrorKind::DeviceException),
        (0xDEAD_BEEF, ErrorKind::Internal),
    ] {
        for value in [None, Some(Variant::scalar(Scalar::Int32(9)))] {
            let item = mapped(DataValue {
                value,
                status: Some(StatusCode(code)),
                ..DataValue::default()
            });
            let error = item.outcome().as_ref().unwrap_err();
            assert_eq!(error.kind(), expected);
            assert_eq!(error.status().unwrap().code(), code);
            assert_eq!(error.status().unwrap().name(), StatusCode(code).name());
            assert_eq!(error.error().diagnostics().operation, Some(Operation::Read));
            assert_eq!(
                error.error().diagnostics().connection_state,
                Some(crate::ConnectionState::Connected)
            );
            assert_eq!(
                error.error().diagnostics().category,
                Some(crate::FailureCategory::Protocol)
            );
            assert!(std::error::Error::source(error.error()).is_some());
        }
    }
}

#[test]
fn opcua_uncertain_with_a_value_is_ok_with_its_status_and_source_time() {
    let item = mapped(DataValue {
        status: Some(StatusCode(0x4000_0000)),
        source_timestamp: Some(DateTime(116_444_736_000_000_000)),
        ..data(Scalar::Int32(42))
    });
    assert_eq!(item.outcome().as_ref().unwrap(), &PlcValue::I32(42));
    assert_eq!(item.status().unwrap().code(), 0x4000_0000);
    assert_eq!(item.source_time(), Some(std::time::UNIX_EPOCH));
    assert_eq!(
        mapped(DataValue {
            status: Some(StatusCode(0x4000_0000)),
            ..DataValue::default()
        })
        .outcome()
        .as_ref()
        .unwrap_err()
        .kind(),
        ErrorKind::DeviceException
    );
    assert_eq!(
        mapped(DataValue {
            source_timestamp: Some(DateTime(0)),
            ..data(Scalar::Int32(42))
        })
        .source_time(),
        None
    );
}

#[test]
fn opcua_later_value_mappings_are_explicit_per_item_unsupported_errors() {
    let scalars = vec![
        Scalar::ExpandedNodeId(ExpandedNodeId::local(NodeId::numeric(0, 1))),
        Scalar::ExtensionObject(ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, 0)),
            body: ExtensionBody::Null,
        }),
        Scalar::DataValue(Box::default()),
        Scalar::Variant(Box::default()),
        Scalar::DiagnosticInfo(DiagnosticInfo::default()),
    ];
    for scalar in scalars {
        assert_eq!(
            mapped(data(scalar)).outcome().as_ref().unwrap_err().kind(),
            ErrorKind::Unsupported(Operation::Read)
        );
    }
}

#[test]
fn opcua_read_request_carries_node_id_attribute_and_index_range() {
    let mut request = ReadRequest::builder(ResourceLimits::builder().build().unwrap());
    for (id, text) in [
        "ns=65535;i=4294967295;a=17",
        "ns=2;s=plant.value[1..3,2];a=13",
        "ns=2;g=72962B91-FA75-4AE6-8D28-B404DC7DAF63",
        "ns=2;b=AQID",
    ]
    .into_iter()
    .enumerate()
    {
        request = request.tag(Tag::new(
            TagId::new(&id.to_string()).unwrap(),
            Arc::new(OpcuaAddress::parse(text).unwrap()),
        ));
    }
    let nodes = nodes(&request.build().unwrap()).unwrap();
    let numeric = nodes.first().unwrap();
    assert_eq!(numeric.node_id, NodeId::numeric(65535, u32::MAX));
    assert_eq!(numeric.attribute_id, 17);
    assert_eq!(numeric.index_range, UaString::null());
    assert_eq!(
        numeric.data_encoding,
        QualifiedName {
            namespace_index: 0,
            name: UaString::null()
        }
    );
    let string = nodes.get(1).unwrap();
    assert_eq!(
        string.node_id.identifier,
        Identifier::String(UaString::new("plant.value"))
    );
    assert_eq!(string.index_range, UaString::new("1:3,2"));
    let Identifier::Guid(guid) = &nodes.get(2).unwrap().node_id.identifier else {
        panic!("guid")
    };
    assert_eq!(guid.data1, 0x7296_2B91);
    assert_eq!(guid.data2, 0xFA75);
    assert_eq!(guid.data3, 0x4AE6);
    assert_eq!(guid.data4, [0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D, 0xAF, 0x63]);
    assert_eq!(
        nodes.get(3).unwrap().node_id.identifier,
        Identifier::ByteString(UaByteString::new(vec![1, 2, 3]))
    );
}

#[test]
fn opcua_good_absent_and_explicit_null_values_share_the_response_node_budget() {
    let mut budget = 2;
    for value in [None, Some(Variant::default())] {
        let data = DataValue {
            value,
            ..DataValue::default()
        };
        let result = item(
            TagId::new("null").unwrap(),
            data,
            Instant::now(),
            OpcuaDataType::Time,
            &mut budget,
        );
        assert_eq!(result.outcome().as_ref().unwrap(), &PlcValue::Null);
    }
    assert_eq!(budget, 0);
    for value in [None, Some(Variant::default())] {
        let data = DataValue {
            value,
            ..DataValue::default()
        };
        assert_eq!(
            item(
                TagId::new("over").unwrap(),
                data,
                Instant::now(),
                OpcuaDataType::Null,
                &mut budget
            )
            .outcome()
            .as_ref()
            .unwrap_err()
            .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(budget, 0);
    }
    let bad = DataValue {
        status: Some(StatusCode(0x8034_0000)),
        ..DataValue::default()
    };
    assert_eq!(
        item(
            TagId::new("bad").unwrap(),
            bad,
            Instant::now(),
            OpcuaDataType::Null,
            &mut budget
        )
        .outcome()
        .as_ref()
        .unwrap_err()
        .kind(),
        ErrorKind::NotFound
    );
    assert_eq!(budget, 0);
}

// covers: OPCUA-READ-METADATA / OPCUA-DIV-READ-METADATA
#[test]
fn opcua_read_item_carries_quality_and_both_timestamps() {
    for code in [0, 0x4000_0000, 0x8034_0000] {
        let result = mapped(DataValue {
            status: Some(StatusCode(code)),
            source_timestamp: Some(DateTime(116_444_736_000_000_001)),
            server_timestamp: Some(DateTime(116_444_736_000_000_002)),
            source_picoseconds: Some(123),
            server_picoseconds: Some(9999),
            ..data(Scalar::Int32(42))
        });
        assert_eq!(
            result.source_time(),
            Some(std::time::UNIX_EPOCH + Duration::from_nanos(101))
        );
        assert_eq!(
            result.server_time(),
            Some(std::time::UNIX_EPOCH + Duration::from_nanos(299))
        );
        assert_eq!(result.status().unwrap().code(), code);
        assert_eq!(result.status().unwrap().name(), StatusCode(code).name());
        if code == 0x8034_0000 {
            assert_eq!(
                result.outcome().as_ref().unwrap_err().kind(),
                ErrorKind::NotFound
            );
        } else {
            assert_eq!(result.outcome().as_ref().unwrap(), &PlcValue::I32(42));
        }
    }
}
#[test]
fn opcua_a_missing_timestamp_is_absent_not_zero() {
    for time in [None, Some(DateTime(0))] {
        let result = mapped(DataValue {
            source_timestamp: time,
            server_timestamp: time,
            source_picoseconds: Some(65535),
            server_picoseconds: Some(65535),
            ..data(Scalar::Int32(1))
        });
        assert_eq!(result.source_time(), None);
        assert_eq!(result.server_time(), None);
    }
    for (picos, nanos) in [
        (None, 0),
        (Some(0), 0),
        (Some(99), 0),
        (Some(100), 1),
        (Some(9999), 99),
        (Some(10000), 99),
        (Some(65535), 99),
    ] {
        let result = timestamp(Some(DateTime(116_444_736_000_000_000)), picos);
        assert_eq!(
            result,
            Some(std::time::UNIX_EPOCH + Duration::from_nanos(nanos))
        );
    }
    assert_eq!(
        timestamp(Some(DateTime(116_444_735_999_999_999)), Some(100)),
        Some(std::time::UNIX_EPOCH - Duration::from_nanos(99))
    );
}
