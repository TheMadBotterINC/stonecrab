//! Write value construction and correlated dispositions, without transport I/O.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-WRITE
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-REQUEST
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-TYPES
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-TIME
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-DATETIME
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-STATUS
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-UNSUPPORTED
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-DATETIME-WRITE
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-DATETIME-WRITE

use std::time::Instant;

use crate::opcua::attributes::Attributes;
use crate::opcua::codec::DataValue;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::generated::write_value::WriteValue;
use crate::opcua::read::{address, classify, protocol_status};
use crate::{
    ErrorKind, ItemError, Operation, PlcError, Result, TagId, UnknownReason, WriteItem,
    WriteOutcome, WriteRequest, WriteResponse, WriteResult,
};

pub(super) mod value;

pub(super) fn node(
    item: &WriteItem,
    node: ReadValueId,
    attributes: &Attributes,
    budget: &mut value::Budget,
) -> Result<WriteValue> {
    if node.attribute_id != 13 {
        return Err(PlcError::new(
            ErrorKind::Unsupported(Operation::Write),
            "OPC UA writes support only the Value attribute",
        ));
    }
    Ok(WriteValue {
        node_id: node.node_id,
        attribute_id: node.attribute_id,
        index_range: node.index_range,
        value: DataValue {
            value: Some(value::encode(
                item.value(),
                address(item.tag())?,
                attributes,
                budget,
            )?),
            ..DataValue::default()
        },
    })
}

pub(super) fn rejected(tag: TagId, error: PlcError) -> WriteResult {
    WriteResult::new(tag, WriteOutcome::Rejected(ItemError::new(error)))
}

pub(super) fn response(
    request: &WriteRequest,
    tags: Vec<TagId>,
    mut local: Vec<WriteResult>,
    reply: Service,
    started: Instant,
    finished: Instant,
) -> Result<WriteResponse> {
    let Service::WriteResponse {
        response_header,
        results,
        ..
    } = reply
    else {
        return Err(PlcError::new(
            ErrorKind::MalformedData,
            "OPC UA expected WriteResponse",
        ));
    };
    if response_header.service_result.0 & 0xC000_0000 != 0 {
        return Err(crate::opcua::service::status_error(
            response_header.service_result,
        ));
    }
    let results = results.unwrap_or_default();
    if results.len() != tags.len() {
        return Err(PlcError::new(
            ErrorKind::MalformedData,
            "OPC UA Write result count differs from the sent item count",
        ));
    }
    for (tag, status) in tags.into_iter().zip(results) {
        let result = if status.0 & 0xC000_0000 == 0 {
            WriteResult::new(tag, WriteOutcome::Confirmed)
        } else {
            let source = crate::opcua::service::status_error(status);
            rejected(
                tag,
                PlcError::new(classify(status), source.to_string()).with_source(source),
            )
        };
        local.push(result.with_status(protocol_status(status)));
    }
    finish(request, local, started, finished)
}

pub(super) fn unknown(tags: Vec<TagId>, kind: ErrorKind) -> Vec<WriteResult> {
    let reason = match kind {
        ErrorKind::Cancelled => UnknownReason::CancelledAfterTransmit,
        ErrorKind::Disconnected => UnknownReason::DisconnectedAfterTransmit,
        ErrorKind::Timeout
        | ErrorKind::InvalidConfig
        | ErrorKind::InvalidAddress
        | ErrorKind::Unsupported(_)
        | ErrorKind::Authentication
        | ErrorKind::Trust
        | ErrorKind::MalformedData
        | ErrorKind::DeviceException
        | ErrorKind::ResourceLimit
        | ErrorKind::Backpressure
        | ErrorKind::Internal
        | ErrorKind::NotFound
        | ErrorKind::InvalidDataType
        | ErrorKind::AccessDenied => UnknownReason::TimeoutAfterTransmit,
    };
    tags.into_iter()
        .map(|tag| WriteResult::new(tag, WriteOutcome::Unknown(reason)))
        .collect()
}

pub(super) fn finish(
    request: &WriteRequest,
    results: Vec<WriteResult>,
    started: Instant,
    finished: Instant,
) -> Result<WriteResponse> {
    results
        .into_iter()
        .fold(
            WriteResponse::builder(request, started, finished),
            crate::WriteResponseBuilder::result,
        )
        .build()
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
