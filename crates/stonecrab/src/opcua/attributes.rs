//! Variable metadata shared by staged writes and later browse enrichment (ADR-0038).
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-WRITE-TYPE-SELECT

use std::num::NonZeroUsize;
use std::sync::{MutexGuard, PoisonError};

use crate::opcua::codec::{Array, DataValue, Identifier, NodeId, Scalar, UaString, VariantValue};
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::generated::timestamps_to_return::TimestampsToReturn;
use crate::opcua::session::Session;
use crate::{CancellationToken, Deadline, ErrorKind, Operation, PlcError, PlcStruct, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Attributes {
    pub(super) data_type: Option<u32>,
    pub(super) rank: Option<i32>,
    pub(super) dimensions: Vec<u32>,
    pub(super) access: Option<u8>,
}

pub(super) async fn resolve(
    session: &Session,
    node: &ReadValueId,
    capacity: NonZeroUsize,
    deadline: Deadline,
    cancel: &CancellationToken,
) -> Result<Attributes> {
    let cached = lock(session)
        .iter()
        .find(|(id, _)| id == &node.node_id)
        .map(|(_, attributes)| attributes.clone());
    if let Some(attributes) = cached {
        return Ok(attributes);
    }
    let queries = [14, 15, 16, 17].map(|attribute_id| ReadValueId {
        node_id: node.node_id.clone(),
        attribute_id,
        index_range: UaString::null(),
        data_encoding: node.data_encoding.clone(),
    });
    let mut values = Vec::new();
    for queries in queries.chunks(capacity.get()) {
        let reply = session
            .lane
            .exchange_until(
                &session.authentication_token,
                631,
                |header| Service::ReadRequest {
                    request_header: header,
                    max_age: 0.0,
                    timestamps_to_return: TimestampsToReturn::TimestampsToReturnNeither,
                    nodes_to_read: Some(queries.to_vec()),
                },
                deadline,
                cancel,
            )
            .await?;
        let Service::ReadResponse {
            response_header,
            results,
            ..
        } = reply
        else {
            return Err(malformed());
        };
        if response_header.service_result.0 & 0xC000_0000 != 0 {
            return Err(crate::opcua::service::status_error(
                response_header.service_result,
            ));
        }
        let results = results.unwrap_or_default();
        if results.len() != queries.len() {
            return Err(malformed());
        }
        values.extend(results);
    }
    let attributes = decode(values)?;
    let mut cache = lock(session);
    cache.retain(|(id, _)| id != &node.node_id);
    if cache.len() >= capacity.get() {
        cache.remove(0);
    }
    cache.push((node.node_id.clone(), attributes.clone()));
    drop(cache);
    Ok(attributes)
}

fn lock(session: &Session) -> MutexGuard<'_, Vec<(NodeId, Attributes)>> {
    session
        .attributes
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "each metadata attribute accepts only its specified wire type; other Variant kinds are malformed"
)]
fn decode(values: Vec<DataValue>) -> Result<Attributes> {
    let [data_type, rank, dimensions, access]: [DataValue; 4] =
        values.try_into().map_err(|_value| malformed())?;
    let data_type = match value(data_type)? {
        VariantValue::Null => None,
        VariantValue::Scalar(Scalar::NodeId(NodeId {
            namespace: 0,
            identifier: Identifier::Numeric(id),
        })) => Some(id),
        VariantValue::Scalar(Scalar::NodeId(_)) => {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                "OPC UA custom datatype resolution is not implemented",
            ));
        }
        _ => return Err(malformed()),
    };
    let rank = match value(rank)? {
        VariantValue::Null => None,
        VariantValue::Scalar(Scalar::Int32(rank))
            if rank >= -3 && rank <= i32::try_from(PlcStruct::MAX_DEPTH).unwrap_or(i32::MAX) =>
        {
            Some(rank)
        }
        _ => return Err(malformed()),
    };
    let dimensions = match value(dimensions)? {
        VariantValue::Null | VariantValue::Array(Array::UInt32(None)) => Vec::new(),
        VariantValue::Array(Array::UInt32(Some(dimensions)))
            if dimensions.len() <= PlcStruct::MAX_DEPTH =>
        {
            dimensions
        }
        _ => return Err(malformed()),
    };
    let access = match value(access)? {
        VariantValue::Null => None,
        VariantValue::Scalar(Scalar::Byte(access)) => Some(access),
        _ => return Err(malformed()),
    };
    Ok(Attributes {
        data_type,
        rank,
        dimensions,
        access,
    })
}

fn value(data: DataValue) -> Result<VariantValue> {
    let status = data.status.unwrap_or_default();
    match status.0 {
        // An absent optional attribute permits explicit value inference, not a schema guess.
        0x8035_0000 | 0x803D_0000 => Ok(VariantValue::Null),
        _ if status.0 & 0xC000_0000 != 0 => Err(PlcError::new(
            crate::opcua::read::classify(status),
            "OPC UA variable metadata read failed",
        )
        .with_source(crate::opcua::service::status_error(status))),
        _ => Ok(data.value.unwrap_or_default().value),
    }
}

fn malformed() -> PlcError {
    PlcError::new(
        ErrorKind::MalformedData,
        "OPC UA invalid variable metadata response",
    )
}

#[cfg(test)]
mod tests;
