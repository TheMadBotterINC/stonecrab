//! Pure service-body encoding, common headers and exact device status sources.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-REQUEST-HEADER
//! # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-SERVICE-FAULT

use crate::codec::{BoundedReader, BoundedWriter};
use crate::opcua::codec::{
    DateTime, Depth, ExpandedNodeId, ExtensionBody, ExtensionObject, Identifier, NodeId,
    StatusCode, UaString,
};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::request_header::RequestHeader;
use crate::{Clock, Deadline, ErrorKind, PlcError, Result};
use bytes::Bytes;

pub(super) fn request_header(
    clock: &dyn Clock,
    token: &NodeId,
    id: u32,
    deadline: Deadline,
) -> Result<RequestHeader> {
    let timestamp = DateTime::from_system_time(clock.wall_now())
        .ok_or_else(|| error(ErrorKind::InvalidConfig, "clock timestamp is out of range"))?;
    let remaining = deadline.instant().saturating_duration_since(clock.now());
    Ok(RequestHeader {
        authentication_token: token.clone(),
        timestamp: timestamp.0,
        request_handle: id,
        return_diagnostics: 0,
        audit_entry_id: UaString::null(),
        timeout_hint: u32::try_from(remaining.as_millis())
            .unwrap_or(u32::MAX)
            .max(1),
        additional_header: ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, 0)),
            body: ExtensionBody::Null,
        },
    })
}

pub(super) fn encode_service(id: u32, service: &Service, max: usize) -> Result<Bytes> {
    let mut writer = BoundedWriter::new(max);
    NodeId::numeric(0, id).serialize(&mut writer, Depth::new())?;
    service.serialize(&mut writer, Depth::new())?;
    Ok(Bytes::from(writer.into_bytes()))
}

pub(super) fn decode_service(body: &[u8]) -> Result<Service> {
    let mut reader = BoundedReader::new(body);
    let id = NodeId::parse(&mut reader, Depth::new())?;
    let Identifier::Numeric(id_value) = id.identifier else {
        return Err(error(
            ErrorKind::MalformedData,
            "non-numeric service encoding id",
        ));
    };
    if id.namespace != 0 {
        return Err(error(
            ErrorKind::MalformedData,
            "non-zero service namespace",
        ));
    }
    let id = i32::try_from(id_value)
        .map_err(|_range| error(ErrorKind::MalformedData, "service encoding id out of range"))?;
    let service = Service::parse(&mut reader, Depth::new(), id)?;
    reader.expect_eof()?;
    if let Service::ServiceFault { response_header } = &service {
        return Err(status_error(response_header.service_result));
    }
    Ok(service)
}

/// Exact device status, also retained through a channel-wide failure's source chain.
#[derive(Debug, thiserror::Error)]
#[error("OPC UA status 0x{code:08X} ({name})")]
pub(super) struct StatusFault {
    pub(super) code: u32,
    pub(super) name: &'static str,
}

pub(super) fn status_error(status: StatusCode) -> PlcError {
    let source = StatusFault {
        code: status.0,
        name: status.name().unwrap_or("unknown"),
    };
    PlcError::new(ErrorKind::DeviceException, source.to_string()).with_source(source)
}

fn error(kind: ErrorKind, message: &str) -> PlcError {
    PlcError::new(kind, format!("OPC UA service: {message}"))
}
