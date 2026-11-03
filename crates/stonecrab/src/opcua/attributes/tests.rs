use super::*;
use crate::opcua::codec::{StatusCode, Variant};

fn data(value: VariantValue) -> DataValue {
    DataValue {
        value: Some(Variant {
            value,
            dimensions: None,
        }),
        ..DataValue::default()
    }
}
fn scalar(value: Scalar) -> DataValue {
    data(VariantValue::Scalar(value))
}
fn fields() -> Vec<DataValue> {
    vec![
        scalar(Scalar::NodeId(NodeId::numeric(0, 6))),
        scalar(Scalar::Int32(-1)),
        data(VariantValue::Array(Array::UInt32(Some(vec![])))),
        scalar(Scalar::Byte(3)),
    ]
}
#[test]
fn opcua_variable_attributes_decode_only_their_declared_wire_shapes() {
    assert_eq!(
        decode(fields()).unwrap(),
        Attributes {
            data_type: Some(6),
            rank: Some(-1),
            dimensions: vec![],
            access: Some(3)
        }
    );
    assert_eq!(
        decode(vec![DataValue::default(); 4]).unwrap(),
        Attributes::default()
    );
    for (index, value) in [
        (0, scalar(Scalar::Int32(6))),
        (1, scalar(Scalar::Byte(1))),
        (1, scalar(Scalar::Int32(-4))),
        (1, scalar(Scalar::Int32(65))),
        (2, scalar(Scalar::Int32(1))),
        (
            2,
            data(VariantValue::Array(Array::UInt32(Some(vec![1; 65])))),
        ),
        (3, scalar(Scalar::Int32(3))),
    ] {
        let mut values = fields();
        *values.get_mut(index).unwrap() = value;
        assert_eq!(decode(values).unwrap_err().kind(), ErrorKind::MalformedData);
    }
    assert_eq!(decode(vec![]).unwrap_err().kind(), ErrorKind::MalformedData);
    let mut values = fields();
    *values.get_mut(2).unwrap() = data(VariantValue::Array(Array::UInt32(None)));
    assert_eq!(decode(values).unwrap().dimensions, Vec::<u32>::new());
    for id in [
        NodeId::numeric(2, 6),
        NodeId {
            namespace: 0,
            identifier: Identifier::String(UaString::new("custom")),
        },
    ] {
        let mut values = fields();
        *values.first_mut().unwrap() = scalar(Scalar::NodeId(id));
        assert_eq!(
            decode(values).unwrap_err().kind(),
            ErrorKind::Unsupported(Operation::Write)
        );
    }
}
#[test]
fn opcua_only_absent_or_unsupported_metadata_permits_inference() {
    for status in [0x8035_0000, 0x803D_0000] {
        let values = vec![
            DataValue {
                status: Some(StatusCode(status)),
                ..DataValue::default()
            };
            4
        ];
        assert_eq!(decode(values).unwrap(), Attributes::default());
    }
    for status in [0x8034_0000, 0x801F_0000, 0x4000_0000, 0xDEAD_BEEF] {
        let mut values = fields();
        values.first_mut().unwrap().status = Some(StatusCode(status));
        let error = decode(values).unwrap_err();
        assert_eq!(
            error.kind(),
            crate::opcua::read::classify(StatusCode(status))
        );
        assert!(std::error::Error::source(&error).is_some());
    }
}
