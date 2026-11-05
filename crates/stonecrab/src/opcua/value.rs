//! Pure OPC UA Variant mapping and checked temporal reinterpretation.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-READ-VALUE-MAPPING
//! # Compatibility: docs/parity/opcua.md#OPCUA-READ-TYPE-OVERRIDE
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-TEMPORAL-OVERRIDE
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-DATETIME-PRECISION
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-DATETIME
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-GUID
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-GUID-TEXT
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-GUID-TEXT
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-NODE-ID
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-QUALIFIED-NAME
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-STATUS-CODE
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-UNMAPPED
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-SCALAR-ARRAY
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-ONE-ELEMENT-ARRAY
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-ONE-ELEMENT-ARRAY

use crate::opcua::address::guid_text;
use crate::opcua::codec::{
    Array, Depth, Guid, Identifier, NodeId, Scalar, UaString, Variant, VariantValue,
};
use crate::opcua::{OpcuaAddress, OpcuaDataType, OpcuaIdentifier};
use crate::{
    ErrorKind, Operation, PlcDate, PlcDateTime, PlcError, PlcSignedDuration, PlcStruct,
    PlcTimeOfDay, PlcValue, Result,
};

/// Aggregate values and List containers in one Read response, independent of wire length.
pub(super) const MAX_VALUE_NODES: usize = 1_048_576;
const NANOS_PER_SECOND: i128 = 1_000_000_000;
const NANOS_PER_DAY: i128 = 86_400_000_000_000;
const UA_UNIX_TICKS: i128 = 116_444_736_000_000_000;
const IEC_UNIX_DAYS: i128 = 7305;
const IEC_UNIX_SECONDS: i128 = 631_152_000;

pub(super) fn decode(
    variant: Variant,
    target: OpcuaDataType,
    budget: &mut usize,
) -> Result<PlcValue> {
    let (value, used) = match variant.value {
        VariantValue::Null => (PlcValue::Null, 1),
        VariantValue::Scalar(value) => {
            let used = if matches!(&value, Scalar::LocalizedText(_)) {
                3
            } else {
                1
            };
            if used > *budget {
                return Err(resource());
            }
            (scalar(value, target)?, used)
        }
        VariantValue::Array(array) => {
            let (partitions, used) = shape(variant.dimensions.as_deref(), array.len(), *budget)?;
            let extra = if let Array::LocalizedText(Some(values)) = &array {
                if !values.is_empty()
                    && variant
                        .dimensions
                        .as_ref()
                        .is_some_and(|dims| dims.len() >= PlcStruct::MAX_DEPTH)
                {
                    return Err(resource());
                }
                values.len().checked_mul(2).ok_or_else(resource)?
            } else {
                0
            };
            let used = used
                .checked_add(extra)
                .filter(|used| *used <= *budget)
                .ok_or_else(resource)?;
            let value = array_values(array, target)?;
            let Some(mut values) = value else {
                charge(budget, 1)?;
                return Ok(PlcValue::Null);
            };
            for (groups, width) in partitions.into_iter().rev() {
                let mut source = values.into_iter();
                let mut outer = Vec::new();
                for _ in 0..groups {
                    outer.push(PlcValue::List(source.by_ref().take(width).collect()));
                }
                values = outer;
            }
            (PlcValue::List(values), used)
        }
    };
    charge(budget, used)?;
    Ok(value)
}

fn charge(budget: &mut usize, count: usize) -> Result<()> {
    *budget = budget.checked_sub(count).ok_or_else(resource)?;
    Ok(())
}

/// Each partition is (number of groups, elements per group), outer to inner.
fn shape(
    dimensions: Option<&[i32]>,
    count: usize,
    budget: usize,
) -> Result<(Vec<(usize, usize)>, usize)> {
    let mut nodes = count
        .checked_add(1)
        .filter(|nodes| *nodes <= budget)
        .ok_or_else(resource)?;
    let Some(dimensions) = dimensions else {
        return Ok((Vec::new(), nodes));
    };
    if dimensions.is_empty() {
        return Err(malformed("empty array dimensions"));
    }
    if dimensions.len() > usize::from(Depth::LIMIT) {
        return Err(resource());
    }
    let mut product = 1_usize;
    let mut partitions = Vec::new();
    for (axis, &dimension) in dimensions.iter().enumerate() {
        let width = usize::try_from(dimension)
            .map_err(|_negative| malformed("negative array dimension"))?;
        let groups = product;
        product = product
            .checked_mul(width)
            .filter(|product| *product <= MAX_VALUE_NODES)
            .ok_or_else(resource)?;
        if axis != 0 {
            nodes = nodes
                .checked_add(groups)
                .filter(|nodes| *nodes <= budget)
                .ok_or_else(resource)?;
            if count == 0 && nodes > usize::from(Depth::LIMIT) {
                return Err(resource());
            }
            partitions.push((groups, width));
        }
    }
    if product != count {
        return Err(malformed("array dimensions do not match element count"));
    }
    Ok((partitions, nodes))
}

fn array_values(array: Array, target: OpcuaDataType) -> Result<Option<Vec<PlcValue>>> {
    macro_rules! arrays {
        (mapped: $($kind:ident),+; refused: $($refused:ident),+) => {
            match array {
                $(Array::$kind(values) => values.map(|values| values.into_iter().map(|value| scalar(Scalar::$kind(value), target)).collect()).transpose(),)+
                $(Array::$refused(_) => Err(unsupported(stringify!($refused))),)+
            }
        };
    }
    arrays! {
        mapped: Boolean, SByte, Byte, Int16, UInt16, Int32, UInt32, Int64, UInt64,
        Float, Double, String, DateTime, Guid, XmlElement, NodeId, StatusCode, QualifiedName, ByteString, LocalizedText;
        refused: ExpandedNodeId, ExtensionObject, DataValue, Variant, DiagnosticInfo
    }
}

fn scalar(value: Scalar, target: OpcuaDataType) -> Result<PlcValue> {
    let value = match value {
        Scalar::Boolean(value) => PlcValue::Bool(value),
        Scalar::SByte(value) => PlcValue::I8(value),
        Scalar::Byte(value) => PlcValue::U8(value),
        Scalar::Int16(value) => PlcValue::I16(value),
        Scalar::UInt16(value) => PlcValue::U16(value),
        Scalar::Int32(value) => PlcValue::I32(value),
        Scalar::UInt32(value) => PlcValue::U32(value),
        Scalar::Int64(value) => PlcValue::I64(value),
        Scalar::UInt64(value) => PlcValue::U64(value),
        Scalar::Float(value) => PlcValue::F32(value),
        Scalar::Double(value) => PlcValue::F64(value),
        Scalar::String(value) => text(value)?.map_or(PlcValue::Null, PlcValue::String),
        Scalar::XmlElement(value) => text(value.0)?.map_or(PlcValue::Null, PlcValue::String),
        Scalar::DateTime(value) => {
            let unix_nanos = i128::from(value.0)
                .saturating_sub(UA_UNIX_TICKS)
                .saturating_mul(100);
            PlcValue::DateTime(date_time(unix_nanos)?)
        }
        Scalar::Guid(value) => PlcValue::String(guid_text(guid_bytes(value))),
        Scalar::NodeId(value) => PlcValue::String(node_text(value)?),
        Scalar::QualifiedName(value) => match text(value.name)? {
            Some(name) => PlcValue::String(format!("ns={};s={name}", value.namespace_index)),
            None => PlcValue::Null,
        },
        Scalar::StatusCode(value) => PlcValue::String(format!(
            "{} (0x{:08X})",
            value.name().unwrap_or("unknown"),
            value.0
        )),
        Scalar::ByteString(value) => value.0.map_or(PlcValue::Null, PlcValue::Bytes),
        Scalar::ExpandedNodeId(_) => return Err(unsupported("ExpandedNodeId")),
        Scalar::LocalizedText(value) => {
            let fields = [("text", value.text), ("locale", value.locale)]
                .into_iter()
                .map(|(name, value)| {
                    Ok((
                        name.to_owned(),
                        text(value.unwrap_or_default())?.map_or(PlcValue::Null, PlcValue::String),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            PlcValue::Struct(PlcStruct::new(fields)?)
        }
        Scalar::ExtensionObject(_) => return Err(unsupported("ExtensionObject")),
        Scalar::DataValue(_) => return Err(unsupported("DataValue")),
        Scalar::Variant(_) => return Err(unsupported("Variant")),
        Scalar::DiagnosticInfo(_) => return Err(unsupported("DiagnosticInfo")),
    };
    temporal(value, target)
}

const fn guid_bytes(value: Guid) -> [u8; 16] {
    let [a0, a1, a2, a3] = value.data1.to_le_bytes();
    let [b0, b1] = value.data2.to_le_bytes();
    let [c0, c1] = value.data3.to_le_bytes();
    let [d0, d1, d2, d3, d4, d5, d6, d7] = value.data4;
    [
        a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, d2, d3, d4, d5, d6, d7,
    ]
}

pub(super) fn node_text(value: NodeId) -> Result<String> {
    let identifier = match value.identifier {
        Identifier::Numeric(value) => OpcuaIdentifier::Numeric(value),
        Identifier::String(value) => {
            OpcuaIdentifier::String(text(value)?.ok_or_else(invalid_type)?)
        }
        Identifier::Guid(value) => OpcuaIdentifier::Guid(guid_bytes(value)),
        Identifier::ByteString(value) => {
            OpcuaIdentifier::ByteString(value.0.ok_or_else(invalid_type)?)
        }
    };
    OpcuaAddress::node_text(value.namespace, identifier).map_err(|_address| invalid_type())
}

fn text(value: UaString) -> Result<Option<String>> {
    value
        .0
        .map(|bytes| String::from_utf8(bytes).map_err(|_utf8| malformed("String is not UTF-8")))
        .transpose()
}

fn temporal(value: PlcValue, target: OpcuaDataType) -> Result<PlcValue> {
    if matches!(
        value,
        PlcValue::Null
            | PlcValue::SignedDuration(_)
            | PlcValue::Date(_)
            | PlcValue::TimeOfDay(_)
            | PlcValue::DateTime(_)
    ) {
        return Ok(value);
    }
    let raw = || value.as_i64().map(i128::from).ok_or_else(invalid_type);
    match target {
        OpcuaDataType::Time | OpcuaDataType::LTime => {
            let nanos = raw()?.saturating_mul(if target == OpcuaDataType::Time {
                1_000_000
            } else {
                1
            });
            Ok(PlcValue::SignedDuration(PlcSignedDuration::from_nanos(
                i64::try_from(nanos).map_err(|_range| invalid_type())?,
            )))
        }
        OpcuaDataType::Date => Ok(PlcValue::Date(date(raw()?.saturating_add(IEC_UNIX_DAYS))?)),
        OpcuaDataType::LDate => Ok(PlcValue::Date(date(
            raw()?.saturating_add(IEC_UNIX_SECONDS).div_euclid(86_400),
        )?)),
        OpcuaDataType::TimeOfDay | OpcuaDataType::LTimeOfDay => {
            Ok(PlcValue::TimeOfDay(time(raw()?.saturating_mul(1_000_000))?))
        }
        OpcuaDataType::DateAndTime => Ok(PlcValue::DateTime(date_time(
            raw()?
                .saturating_add(IEC_UNIX_SECONDS)
                .saturating_mul(NANOS_PER_SECOND),
        )?)),
        OpcuaDataType::LDateAndTime => Ok(PlcValue::DateTime(date_time(
            raw()?
                .saturating_add(631_152_000_000)
                .saturating_mul(1_000_000),
        )?)),
        OpcuaDataType::Null
        | OpcuaDataType::Bool
        | OpcuaDataType::Byte
        | OpcuaDataType::SInt
        | OpcuaDataType::Int
        | OpcuaDataType::DInt
        | OpcuaDataType::LInt
        | OpcuaDataType::USInt
        | OpcuaDataType::UInt
        | OpcuaDataType::UDInt
        | OpcuaDataType::ULInt
        | OpcuaDataType::Real
        | OpcuaDataType::LReal
        | OpcuaDataType::Char
        | OpcuaDataType::WChar
        | OpcuaDataType::String => Ok(value),
    }
}

fn date(days: i128) -> Result<PlcDate> {
    i32::try_from(days)
        .map(PlcDate::from_days_since_epoch)
        .map_err(|_range| invalid_type())
}
fn time(nanos: i128) -> Result<PlcTimeOfDay> {
    PlcTimeOfDay::from_nanos_since_midnight(u64::try_from(nanos).map_err(|_range| invalid_type())?)
        .map_err(|_range| invalid_type())
}
fn date_time(nanos: i128) -> Result<PlcDateTime> {
    Ok(PlcDateTime::new(
        date(nanos.div_euclid(NANOS_PER_DAY))?,
        time(nanos.rem_euclid(NANOS_PER_DAY))?,
    ))
}
fn unsupported(kind: &str) -> PlcError {
    PlcError::new(
        ErrorKind::Unsupported(Operation::Read),
        format!("OPC UA {kind} value mapping is not implemented"),
    )
}
fn malformed(reason: &str) -> PlcError {
    PlcError::new(ErrorKind::MalformedData, format!("OPC UA {reason}"))
}
fn resource() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "OPC UA value depth/node budget exceeded",
    )
}
fn invalid_type() -> PlcError {
    PlcError::new(
        ErrorKind::InvalidDataType,
        "OPC UA value cannot be represented by the requested type",
    )
}

#[cfg(test)]
mod tests;
