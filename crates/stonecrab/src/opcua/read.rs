//! Read request assembly and scalar value/status mapping, without I/O.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-READ-REQUEST
//! # Compatibility: docs/parity/opcua.md#OPCUA-READ-ATTRIBUTE-ID
//! # Compatibility: docs/parity/opcua.md#OPCUA-READ-STATUS
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-STATUS-CODE
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-STATUS-COLLAPSE
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-BOOLEAN
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-INTEGERS
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-FLOATS
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-STRING
//! # Compatibility: docs/parity/opcua.md#OPCUA-TYPE-NULL

use std::any::Any;
use std::num::NonZeroU32;
use std::time::Instant;

use crate::opcua::codec::{
    DataValue, DateTime, Guid, Identifier, NodeId, QualifiedName, Scalar, StatusCode, UaByteString,
    UaString, VariantValue,
};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::service::status_error;
use crate::opcua::{OpcuaAddress, OpcuaIdentifier};
use crate::{
    ErrorKind, ItemError, Operation, PlcError, PlcValue, ProtocolStatus, ReadItem, ReadRequest,
    ReadResponse, Result, TagId,
};

pub(super) fn response(
    request: &ReadRequest,
    reply: Service,
    started: Instant,
    sampled: Instant,
) -> Result<ReadResponse> {
    let Service::ReadResponse {
        response_header,
        results,
        ..
    } = reply
    else {
        return Err(PlcError::new(
            ErrorKind::MalformedData,
            "OPC UA expected ReadResponse",
        ));
    };
    if response_header.service_result.0 & 0xC000_0000 != 0 {
        return Err(status_error(response_header.service_result));
    }
    let results = results.unwrap_or_default();
    if results.len() != request.items().len() {
        return Err(PlcError::new(
            ErrorKind::MalformedData,
            "OPC UA Read result count differs from the tag count",
        ));
    }
    let mut response = ReadResponse::builder(request, started, sampled);
    for (tag, data) in request.items().iter().zip(results) {
        response = response.item(item(tag.id().clone(), data, sampled));
    }
    response.build()
}

pub(super) fn nodes(request: &ReadRequest) -> Result<Vec<ReadValueId>> {
    request
        .items()
        .iter()
        .map(|tag| {
            let address: &dyn Any = tag.address();
            let address = address.downcast_ref::<OpcuaAddress>().ok_or_else(|| {
                PlcError::new(
                    ErrorKind::InvalidAddress,
                    "OPC UA read requires OPC UA addresses",
                )
            })?;
            if tag.count() != NonZeroU32::MIN {
                return Err(PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA element selection belongs in the address IndexRange",
                ));
            }
            let identifier = match address.identifier() {
                OpcuaIdentifier::Numeric(number) => Identifier::Numeric(*number),
                OpcuaIdentifier::String(text) => Identifier::String(UaString::new(text)),
                OpcuaIdentifier::Guid(bytes) => {
                    let [
                        a0,
                        a1,
                        a2,
                        a3,
                        b0,
                        b1,
                        c0,
                        c1,
                        d0,
                        d1,
                        d2,
                        d3,
                        d4,
                        d5,
                        d6,
                        d7,
                    ] = *bytes;
                    Identifier::Guid(Guid {
                        data1: u32::from_le_bytes([a0, a1, a2, a3]),
                        data2: u16::from_le_bytes([b0, b1]),
                        data3: u16::from_le_bytes([c0, c1]),
                        data4: [d0, d1, d2, d3, d4, d5, d6, d7],
                    })
                }
                OpcuaIdentifier::ByteString(bytes) => {
                    Identifier::ByteString(UaByteString::new(bytes.clone()))
                }
            };
            Ok(ReadValueId {
                node_id: NodeId {
                    namespace: address.namespace(),
                    identifier,
                },
                attribute_id: address.attribute().number(),
                index_range: address
                    .index_range()
                    .map_or_else(UaString::null, UaString::new),
                data_encoding: QualifiedName {
                    namespace_index: 0,
                    name: UaString::null(),
                },
            })
        })
        .collect()
}

pub(super) fn item(tag: TagId, data: DataValue, sampled: Instant) -> ReadItem {
    let status = data.status.unwrap_or_default();
    let outcome =
        if status.0 & 0x8000_0000 != 0 || (data.value.is_none() && status.0 & 0x4000_0000 != 0) {
            {
                let source = status_error(status);
                Err(PlcError::new(classify(status), source.to_string()).with_source(source))
            }
        } else {
            data.value
                .map_or(Ok(PlcValue::Null), |value| decode(value.value))
        };
    let outcome = outcome.map_err(|error| {
        ItemError::new(error.with_diagnostics(crate::Diagnostics {
            operation: Some(Operation::Read),
            connection_state: Some(crate::ConnectionState::Connected),
            category: Some(crate::FailureCategory::Protocol),
            ..crate::Diagnostics::default()
        }))
    });
    let mut item = ReadItem::new(tag, outcome, sampled).with_status(protocol_status(status));
    if let Some(time) = data.source_timestamp.and_then(DateTime::to_system_time) {
        item = item.with_source_time(time);
    }
    item
}

const fn protocol_status(status: StatusCode) -> ProtocolStatus {
    let source = ProtocolStatus::new("opcua", status.0);
    if let Some(name) = status.name() {
        source.with_name(name)
    } else {
        source
    }
}

const fn classify(status: StatusCode) -> ErrorKind {
    match status.0 {
        0x8034_0000 => ErrorKind::NotFound,
        0x8074_0000 => ErrorKind::InvalidDataType,
        0x803B_0000 | 0x801F_0000 => ErrorKind::AccessDenied,
        0x8035_0000 | 0x8037_0000 => ErrorKind::InvalidAddress,
        _ if status.name().is_none() => ErrorKind::Internal,
        _ => ErrorKind::DeviceException,
    }
}

fn decode(value: VariantValue) -> Result<PlcValue> {
    let scalar = match value {
        VariantValue::Null => return Ok(PlcValue::Null),
        VariantValue::Scalar(scalar) => scalar,
        VariantValue::Array(_) => return Err(unsupported()),
    };
    match scalar {
        Scalar::Boolean(value) => Ok(PlcValue::Bool(value)),
        Scalar::SByte(value) => Ok(PlcValue::I8(value)),
        Scalar::Byte(value) => Ok(PlcValue::U8(value)),
        Scalar::Int16(value) => Ok(PlcValue::I16(value)),
        Scalar::UInt16(value) => Ok(PlcValue::U16(value)),
        Scalar::Int32(value) => Ok(PlcValue::I32(value)),
        Scalar::UInt32(value) => Ok(PlcValue::U32(value)),
        Scalar::Int64(value) => Ok(PlcValue::I64(value)),
        Scalar::UInt64(value) => Ok(PlcValue::U64(value)),
        Scalar::Float(value) => Ok(PlcValue::F32(value)),
        Scalar::Double(value) => Ok(PlcValue::F64(value)),
        Scalar::String(value) => text(value),
        Scalar::XmlElement(value) => text(value.0),
        Scalar::DateTime(_)
        | Scalar::Guid(_)
        | Scalar::ByteString(_)
        | Scalar::NodeId(_)
        | Scalar::ExpandedNodeId(_)
        | Scalar::StatusCode(_)
        | Scalar::QualifiedName(_)
        | Scalar::LocalizedText(_)
        | Scalar::ExtensionObject(_)
        | Scalar::DataValue(_)
        | Scalar::Variant(_)
        | Scalar::DiagnosticInfo(_) => Err(unsupported()),
    }
}

fn text(value: UaString) -> Result<PlcValue> {
    value.0.map_or(Ok(PlcValue::Null), |bytes| {
        String::from_utf8(bytes)
            .map(PlcValue::String)
            .map_err(|_error| PlcError::new(ErrorKind::MalformedData, "OPC UA String is not UTF-8"))
    })
}

fn unsupported() -> PlcError {
    PlcError::new(
        ErrorKind::Unsupported(Operation::Read),
        "OPC UA value mapping is not implemented for this type",
    )
}

#[cfg(test)]
mod tests;
