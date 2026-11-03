use super::*;
use crate::codec::BoundedWriter;
use crate::opcua::codec::Depth;
use crate::{PlcDate, PlcDateTime, PlcSignedDuration, PlcTimeOfDay};
use std::time::Duration;

fn date(days: i32, nanos: u64) -> PlcValue {
    PlcValue::DateTime(PlcDateTime::new(
        PlcDate::from_days_since_epoch(days),
        PlcTimeOfDay::from_nanos_since_midnight(nanos).unwrap(),
    ))
}
fn convert(value: &PlcValue, suffix: &str, attributes: &Attributes) -> Result<Variant> {
    encode(
        value,
        &OpcuaAddress::parse(&format!("ns=2;i=42{suffix}"))?,
        attributes,
        &mut Budget::new(262_144),
    )
}
fn kind(value: &Variant) -> u8 {
    let mut writer = BoundedWriter::new(262_144);
    value.serialize(&mut writer, Depth::new()).unwrap();
    *writer.into_bytes().first().unwrap()
}

// covers: OPCUA-WRITE-TYPES / OPCUA-WRITE-REQUEST
#[test]
fn opcua_each_writable_type_encodes_as_its_built_in_variant() {
    for (suffix, value, expected, id) in [
        (";BOOL", PlcValue::Bool(true), Scalar::Boolean(true), 1),
        (";SINT", PlcValue::I8(-2), Scalar::SByte(-2), 2),
        (";BYTE", PlcValue::U8(255), Scalar::Byte(255), 3),
        (";USINT", PlcValue::U8(2), Scalar::Byte(2), 3),
        (";INT", PlcValue::I16(-234), Scalar::Int16(-234), 4),
        (";UINT", PlcValue::U16(54_321), Scalar::UInt16(54_321), 5),
        (";DINT", PlcValue::I32(i32::MIN), Scalar::Int32(i32::MIN), 6),
        (
            ";UDINT",
            PlcValue::U32(u32::MAX),
            Scalar::UInt32(u32::MAX),
            7,
        ),
        (";LINT", PlcValue::I64(i64::MIN), Scalar::Int64(i64::MIN), 8),
        (
            ";ULINT",
            PlcValue::U64(u64::MAX),
            Scalar::UInt64(u64::MAX),
            9,
        ),
        (";REAL", PlcValue::F32(1.5), Scalar::Float(1.5), 10),
        (";LREAL", PlcValue::F64(2.5), Scalar::Double(2.5), 11),
        (
            ";CHAR",
            PlcValue::Char('é'),
            Scalar::String(UaString::new("é")),
            12,
        ),
        (
            ";WCHAR",
            PlcValue::Char('水'),
            Scalar::String(UaString::new("水")),
            12,
        ),
        (
            ";STRING",
            PlcValue::String("text\0value".to_owned()),
            Scalar::String(UaString::new("text\0value")),
            12,
        ),
        (
            ";DATE_AND_TIME",
            date(0, 0),
            Scalar::DateTime(DateTime(116_444_736_000_000_000)),
            13,
        ),
        (
            "",
            PlcValue::Bytes(vec![0, 255]),
            Scalar::ByteString(UaByteString::new(vec![0, 255])),
            15,
        ),
    ] {
        let actual = convert(&value, suffix, &Attributes::default()).unwrap();
        assert_eq!(actual.value, VariantValue::Scalar(expected));
        assert_eq!(kind(&actual), id);
        let array = convert(
            &PlcValue::List(vec![value.clone(), value]),
            suffix,
            &Attributes::default(),
        )
        .unwrap();
        assert_eq!(kind(&array), id | 0x80);
        assert!(array.dimensions.is_none());
    }
}

// covers: OPCUA-WRITE-TYPE-SELECT
#[test]
fn opcua_the_tag_suffix_beats_the_server_declared_type() {
    let attributes = Attributes {
        data_type: Some(12),
        ..Attributes::default()
    };
    assert_eq!(
        convert(&PlcValue::I64(42), ";DINT", &attributes)
            .unwrap()
            .value,
        VariantValue::Scalar(Scalar::Int32(42))
    );
    assert!(convert(&PlcValue::I64(42), "", &attributes).is_err());
}
#[test]
fn opcua_the_server_declared_type_is_used_without_a_suffix() {
    let attributes = Attributes {
        data_type: Some(4),
        ..Attributes::default()
    };
    assert_eq!(
        convert(&PlcValue::I64(42), "", &attributes).unwrap().value,
        VariantValue::Scalar(Scalar::Int16(42))
    );
    assert_eq!(
        convert(&PlcValue::I64(32_768), "", &attributes)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidDataType
    );
    for value in [
        PlcValue::Null,
        PlcValue::Bool(false),
        PlcValue::I8(1),
        PlcValue::U8(1),
        PlcValue::I16(1),
        PlcValue::U16(1),
        PlcValue::I32(1),
        PlcValue::U32(1),
        PlcValue::I64(1),
        PlcValue::U64(1),
        PlcValue::F32(1.5),
        PlcValue::F64(1.5),
        PlcValue::Char('x'),
        PlcValue::String("x".into()),
        date(0, 0),
        PlcValue::Bytes(vec![]),
        PlcValue::SignedDuration(PlcSignedDuration::from_nanos(0)),
        PlcValue::Duration(Duration::ZERO),
    ] {
        convert(&value, "", &Attributes::default()).unwrap();
    }
}
#[test]
fn opcua_an_abstract_uinteger_node_picks_an_unsigned_type_sized_to_the_value() {
    for (value, id) in [(255, 3), (256, 5), (65_536, 7), (u64::MAX, 9)] {
        let attributes = Attributes {
            data_type: Some(28),
            ..Attributes::default()
        };
        assert_eq!(
            kind(
                &convert(
                    &PlcValue::List(vec![PlcValue::U8(0), PlcValue::U64(value)]),
                    "",
                    &attributes
                )
                .unwrap()
            ),
            id | 0x80
        );
        assert_eq!(
            kind(&convert(&PlcValue::U64(value), "", &attributes).unwrap()),
            id
        );
    }
    assert!(
        convert(
            &PlcValue::I8(-1),
            "",
            &Attributes {
                data_type: Some(28),
                ..Attributes::default()
            }
        )
        .is_err()
    );
    for declared in [None, Some(0), Some(24), Some(26), Some(27)] {
        convert(
            &PlcValue::I32(1),
            "",
            &Attributes {
                data_type: declared,
                ..Attributes::default()
            },
        )
        .unwrap();
    }
    assert!(
        convert(
            &PlcValue::Bool(true),
            "",
            &Attributes {
                data_type: Some(26),
                ..Attributes::default()
            }
        )
        .is_err()
    );
}

// covers: OPCUA-WRITE-TIME
#[test]
fn opcua_time_is_written_as_int32_milliseconds() {
    for value in [i32::MIN, -1, 0, i32::MAX] {
        let value =
            PlcValue::SignedDuration(PlcSignedDuration::from_millis(i64::from(value)).unwrap());
        assert!(matches!(
            convert(&value, ";TIME", &Attributes::default())
                .unwrap()
                .value,
            VariantValue::Scalar(Scalar::Int32(_))
        ));
        convert(
            &PlcValue::List(vec![value]),
            ";TIME",
            &Attributes::default(),
        )
        .unwrap();
    }
    convert(
        &PlcValue::Duration(Duration::from_millis(42)),
        ";TIME",
        &Attributes::default(),
    )
    .unwrap();
}
#[test]
fn opcua_a_time_that_overflows_int32_is_rejected() {
    for value in [
        PlcValue::SignedDuration(
            PlcSignedDuration::from_millis(i64::from(i32::MAX).saturating_add(1)).unwrap(),
        ),
        PlcValue::SignedDuration(PlcSignedDuration::from_nanos(-1)),
        PlcValue::Duration(Duration::MAX),
        PlcValue::I32(1),
    ] {
        assert_eq!(
            convert(&value, ";TIME", &Attributes::default())
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidDataType
        );
    }
}
// covers: OPCUA-WRITE-DATETIME / OPCUA-DIV-DATETIME-WRITE / OPCUA-QUIRK-DATETIME-WRITE
#[test]
fn opcua_a_date_time_written_then_read_returns_the_same_instant() {
    for value in [
        date(-134_774, 0),
        date(-1, 86_399_999_999_900),
        date(0, 100),
        date(20_759, 123_456_700),
    ] {
        let variant = convert(&value, ";DATE_AND_TIME", &Attributes::default()).unwrap();
        let mut budget = crate::opcua::value::MAX_VALUE_NODES;
        assert_eq!(
            crate::opcua::value::decode(variant, OpcuaDataType::Null, &mut budget).unwrap(),
            value
        );
        convert(&value, ";LDATE_AND_TIME", &Attributes::default()).unwrap();
    }
    for value in [date(0, 1), date(i32::MAX, 0), PlcValue::I64(1)] {
        assert!(convert(&value, ";DATE_AND_TIME", &Attributes::default()).is_err());
    }
}

// covers: OPCUA-WRITE-REQUEST
#[test]
fn opcua_an_index_range_write_needs_an_array_value() {
    assert!(convert(&PlcValue::I32(1), "[0];DINT", &Attributes::default()).is_err());
    let one = convert(
        &PlcValue::List(vec![PlcValue::I32(1)]),
        "[2];DINT",
        &Attributes::default(),
    )
    .unwrap();
    assert_eq!(kind(&one), 0x86);
    assert!(
        convert(
            &PlcValue::List(vec![PlcValue::I32(1)]),
            "[0..1];DINT",
            &Attributes::default()
        )
        .is_err()
    );
}
#[test]
fn opcua_an_array_write_encodes_a_variant_array() {
    let value = PlcValue::List(vec![
        PlcValue::List(vec![PlcValue::I32(1), PlcValue::I32(2)]),
        PlcValue::List(vec![PlcValue::I32(3), PlcValue::I32(4)]),
    ]);
    let attributes = Attributes {
        data_type: Some(6),
        rank: Some(2),
        dimensions: vec![2, 2],
        access: Some(3),
    };
    let variant = convert(&value, "", &attributes).unwrap();
    assert_eq!(variant.dimensions, Some(vec![2, 2]));
    assert_eq!(
        variant.value,
        VariantValue::Array(Array::Int32(Some(vec![1, 2, 3, 4])))
    );
    convert(&value, "[0..1][1..2]", &attributes).unwrap();
    for dimensions in [vec![1, 2], vec![2], vec![0, 1]] {
        assert!(
            convert(
                &value,
                "",
                &Attributes {
                    dimensions,
                    ..attributes.clone()
                }
            )
            .is_err()
        );
    }
    for rank in [-3, -1, 0, 1, 2, 3] {
        assert_eq!(
            convert(
                &value,
                "",
                &Attributes {
                    rank: Some(rank),
                    dimensions: vec![],
                    ..attributes.clone()
                }
            )
            .is_ok(),
            matches!(rank, 0 | 2)
        );
    }
    for value in [
        PlcValue::List(vec![PlcValue::List(vec![]), PlcValue::I32(1)]),
        PlcValue::List(vec![PlcValue::I32(1), PlcValue::List(vec![])]),
        PlcValue::List(vec![
            PlcValue::List(vec![]),
            PlcValue::List(vec![PlcValue::I32(1)]),
        ]),
    ] {
        assert!(convert(&value, ";DINT", &Attributes::default()).is_err());
    }
    assert!(convert(&PlcValue::List(vec![]), "", &Attributes::default()).is_err());
    convert(&PlcValue::List(vec![]), ";DINT", &Attributes::default()).unwrap();
    convert(
        &PlcValue::List(vec![PlcValue::List(vec![])]),
        ";DINT",
        &Attributes::default(),
    )
    .unwrap();
}
#[test]
fn opcua_write_conversions_reject_loss_and_preserve_explicit_nonfinite_floats() {
    for (value, suffix) in [
        (PlcValue::I64(128), ";SINT"),
        (PlcValue::U64(256), ";USINT"),
        (PlcValue::U64(65_536), ";UINT"),
        (PlcValue::U64(u64::MAX), ";DINT"),
        (PlcValue::I64(-1), ";UDINT"),
        (PlcValue::U64(u64::MAX), ";LINT"),
        (PlcValue::I64(-1), ";ULINT"),
        (PlcValue::F64(0.1), ";REAL"),
        (PlcValue::I32(1), ";REAL"),
        (PlcValue::I32(1), ";LREAL"),
        (PlcValue::Bool(false), ";STRING"),
        (PlcValue::I32(1), ";BOOL"),
    ] {
        assert!(convert(&value, suffix, &Attributes::default()).is_err());
    }
    for value in [
        0.0_f64,
        -0.0,
        1.5,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ] {
        let variant = convert(&PlcValue::F64(value), ";REAL", &Attributes::default()).unwrap();
        assert!(matches!(
            variant.value,
            VariantValue::Scalar(Scalar::Float(_))
        ));
    }
    convert(&PlcValue::F32(1.5), ";LREAL", &Attributes::default()).unwrap();
    assert!(
        convert(
            &PlcValue::I32(1),
            "",
            &Attributes {
                data_type: Some(15),
                ..Attributes::default()
            }
        )
        .is_err()
    );
}
// covers: OPCUA-WRITE-UNSUPPORTED
#[test]
fn opcua_an_unsupported_write_type_is_a_typed_error_not_a_panic() {
    for suffix in [";LTIME", ";DATE", ";LDATE", ";TIME_OF_DAY", ";LTIME_OF_DAY"] {
        assert_eq!(
            convert(&PlcValue::I32(1), suffix, &Attributes::default())
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported(Operation::Write)
        );
    }
    for value in [
        PlcValue::Struct(PlcStruct::new(vec![]).unwrap()),
        PlcValue::Date(PlcDate::from_days_since_epoch(0)),
        PlcValue::TimeOfDay(PlcTimeOfDay::from_nanos_since_midnight(0).unwrap()),
    ] {
        assert_eq!(
            convert(&value, "", &Attributes::default())
                .unwrap_err()
                .kind(),
            ErrorKind::Unsupported(Operation::Write)
        );
    }
    for id in [14, 16, 21, 22, 999] {
        assert_eq!(
            convert(
                &PlcValue::Null,
                "",
                &Attributes {
                    data_type: Some(id),
                    ..Attributes::default()
                }
            )
            .unwrap_err()
            .kind(),
            ErrorKind::Unsupported(Operation::Write)
        );
    }
}
#[test]
fn opcua_write_budget_is_transactional_and_bounds_depth_and_pending_nodes() {
    let address = OpcuaAddress::parse("ns=2;i=42").unwrap();
    let mut budget = Budget::new(2);
    assert!(
        encode(
            &PlcValue::String("long".into()),
            &address,
            &Attributes::default(),
            &mut budget
        )
        .is_err()
    );
    assert_eq!(budget.bytes, 2);
    assert_eq!(budget.nodes, crate::opcua::value::MAX_VALUE_NODES);
    let mut value = PlcValue::I32(1);
    for _ in 0..64 {
        value = PlcValue::List(vec![value]);
    }
    convert(&value, ";DINT", &Attributes::default()).unwrap();
    value = PlcValue::List(vec![value]);
    assert_eq!(
        convert(&value, ";DINT", &Attributes::default())
            .unwrap_err()
            .kind(),
        ErrorKind::ResourceLimit
    );
    budget.nodes = 1;
    assert!(
        encode(
            &PlcValue::List(vec![PlcValue::I32(1)]),
            &address,
            &Attributes::default(),
            &mut budget
        )
        .is_err()
    );
    budget.nodes = 0;
    assert!(
        encode(
            &PlcValue::I32(1),
            &address,
            &Attributes::default(),
            &mut budget
        )
        .is_err()
    );
}

#[test]
fn opcua_date_time_is_encoded_as_ticks_since_1601() {
    let value = convert(&date(0, 100), ";DATE_AND_TIME", &Attributes::default()).unwrap();
    assert_eq!(
        value.value,
        VariantValue::Scalar(Scalar::DateTime(DateTime(116_444_736_000_000_001)))
    );
}
#[test]
fn opcua_the_four_string_types_are_sent_as_opc_string() {
    for (value, suffix) in [
        (PlcValue::Char('x'), ";CHAR"),
        (PlcValue::Char('水'), ";WCHAR"),
        (PlcValue::String("ascii".into()), ";STRING"),
        (PlcValue::String("wide 水".into()), ""),
    ] {
        let variant = convert(
            &value,
            suffix,
            &Attributes {
                data_type: Some(12),
                ..Attributes::default()
            },
        )
        .unwrap();
        assert_eq!(kind(&variant), 12);
    }
    assert!(OpcuaDataType::from_name("WSTRING").is_none());
}

#[test]
fn opcua_metadata_access_and_range_rank_rejections_leave_the_budget_unchanged() {
    assert_eq!(
        convert(
            &PlcValue::I32(1),
            "",
            &Attributes {
                access: Some(1),
                ..Attributes::default()
            }
        )
        .unwrap_err()
        .kind(),
        ErrorKind::AccessDenied
    );
    assert!(
        convert(
            &PlcValue::List(vec![PlcValue::I32(1)]),
            "[0];DINT",
            &Attributes {
                rank: Some(2),
                ..Attributes::default()
            }
        )
        .is_err()
    );
    let ragged = PlcValue::List(vec![
        PlcValue::List(vec![PlcValue::I32(1)]),
        PlcValue::I32(2),
    ]);
    assert!(convert(&ragged, ";DINT", &Attributes::default()).is_err());
    assert!(scalar(&PlcValue::I32(1), Encoding::BuiltIn(0)).is_err());
    assert!(
        convert(
            &PlcValue::List(vec![PlcValue::Null]),
            "",
            &Attributes::default()
        )
        .is_err()
    );
}

#[test]
fn opcua_rank_zero_accepts_arrays_and_ranges_but_never_a_scalar() {
    let attributes = Attributes {
        rank: Some(0),
        ..Attributes::default()
    };
    assert!(convert(&PlcValue::I32(1), ";DINT", &attributes).is_err());
    convert(
        &PlcValue::List(vec![PlcValue::I32(1)]),
        ";DINT",
        &attributes,
    )
    .unwrap();
    convert(
        &PlcValue::List(vec![PlcValue::I32(1)]),
        "[0];DINT",
        &attributes,
    )
    .unwrap();
    let mut budget = Budget {
        nodes: 3,
        bytes: 100,
    };
    let address = OpcuaAddress::parse("ns=2;i=42;DINT").unwrap();
    encode(
        &PlcValue::List(vec![PlcValue::I32(1), PlcValue::I32(2)]),
        &address,
        &Attributes::default(),
        &mut budget,
    )
    .unwrap();
    assert_eq!(budget.nodes, 0);
    let mut budget = Budget {
        nodes: 2,
        bytes: 100,
    };
    assert!(
        encode(
            &PlcValue::List(vec![PlcValue::I32(1), PlcValue::I32(2)]),
            &address,
            &Attributes::default(),
            &mut budget
        )
        .is_err()
    );
    assert_eq!(budget.nodes, 2);
}
