//! Read request assembly and value/status mapping, without I/O.
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
use std::time::{Duration, Instant, SystemTime};

use crate::opcua::codec::{
    DataValue, DateTime, Guid, Identifier, NodeId, QualifiedName, StatusCode, UaByteString,
    UaString,
};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::service::status_error;
use crate::opcua::{OpcuaAddress, OpcuaDataType, OpcuaIdentifier};
use crate::{
    ErrorKind, ItemError, Operation, PlcError, ProtocolStatus, ReadItem, ReadRequest, ReadResponse,
    Result, Tag, TagId,
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
    let mut budget = crate::opcua::value::MAX_VALUE_NODES;
    for (tag, data) in request.items().iter().zip(results) {
        response = response.item(item(
            tag.id().clone(),
            data,
            sampled,
            address(tag)?.data_type(),
            &mut budget,
        ));
    }
    response.build()
}

pub(super) fn nodes(request: &ReadRequest) -> Result<Vec<ReadValueId>> {
    request.items().iter().map(node).collect()
}

pub(super) fn node(tag: &Tag) -> Result<ReadValueId> {
    let address = address(tag)?;
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
}

pub(super) fn address(tag: &Tag) -> Result<&OpcuaAddress> {
    let address: &dyn Any = tag.address();
    address.downcast_ref::<OpcuaAddress>().ok_or_else(|| {
        PlcError::new(
            ErrorKind::InvalidAddress,
            "OPC UA read requires OPC UA addresses",
        )
    })
}

fn item(
    tag: TagId,
    data: DataValue,
    sampled: Instant,
    target: OpcuaDataType,
    budget: &mut usize,
) -> ReadItem {
    let status = data.status.unwrap_or_default();
    let outcome =
        if status.0 & 0x8000_0000 != 0 || (data.value.is_none() && status.0 & 0x4000_0000 != 0) {
            {
                let source = status_error(status);
                Err(PlcError::new(classify(status), source.to_string()).with_source(source))
            }
        } else {
            crate::opcua::value::decode(data.value.unwrap_or_default(), target, budget)
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
    if let Some(time) = timestamp(data.source_timestamp, data.source_picoseconds) {
        item = item.with_source_time(time);
    }
    if let Some(time) = timestamp(data.server_timestamp, data.server_picoseconds) {
        item = item.with_server_time(time);
    }
    item
}

/// `SystemTime` cannot represent the remaining fraction below one nanosecond.
fn timestamp(time: Option<DateTime>, picoseconds: Option<u16>) -> Option<SystemTime> {
    let time = time.filter(|time| time.0 != 0)?.to_system_time()?;
    let nanos = u64::from(picoseconds.unwrap_or(0).min(9999)).div_euclid(100);
    time.checked_add(Duration::from_nanos(nanos))
}

pub(super) const fn protocol_status(status: StatusCode) -> ProtocolStatus {
    let source = ProtocolStatus::new("opcua", status.0);
    if let Some(name) = status.name() {
        source.with_name(name)
    } else {
        source
    }
}

pub(super) const fn classify(status: StatusCode) -> ErrorKind {
    match status.0 {
        0x8034_0000 => ErrorKind::NotFound,
        0x8074_0000 => ErrorKind::InvalidDataType,
        0x803B_0000 | 0x801F_0000 => ErrorKind::AccessDenied,
        0x8035_0000 | 0x8037_0000 => ErrorKind::InvalidAddress,
        _ if status.name().is_none() => ErrorKind::Internal,
        _ => ErrorKind::DeviceException,
    }
}

#[cfg(test)]
mod tests;
