//! Checked message-body limits and accumulation, independent of transport ownership.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CHUNK-SIZE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CHUNK-SEND-LIMITS
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-SEND-LIMITS
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-SEND-LIMITS
//! # Compatibility: docs/parity/opcua.md#OPCUA-CHUNK-RECEIVE

use bytes::Bytes;

use crate::codec::BoundedReader;
use crate::opcua::codec::{Depth, StatusCode, UaString};
use crate::opcua::service::status_error;
use crate::{ErrorKind, PlcError, Result};

/// Sizes of the active policy's block transformation and footer.
#[derive(Debug, Clone, Copy)]
pub(super) struct SecuritySizes {
    signature: usize,
    plain_block: usize,
    cipher_block: usize,
    padding: usize,
}

impl SecuritySizes {
    pub(super) const NONE: Self = Self {
        signature: 0,
        plain_block: 1,
        cipher_block: 1,
        padding: 0,
    };

    pub(super) const fn new(
        signature: usize,
        plain_block: usize,
        cipher_block: usize,
        padding: usize,
    ) -> Self {
        Self {
            signature,
            plain_block,
            cipher_block,
            padding,
        }
    }

    /// UASC has a 12-byte message header and an 8-byte sequence header.
    pub(super) fn max_body(self, buffer: usize, security_header: usize) -> Result<usize> {
        buffer
            .checked_sub(12)
            .and_then(|size| size.checked_sub(security_header))
            .and_then(|size| size.checked_div(self.cipher_block))
            .and_then(|blocks| blocks.checked_mul(self.plain_block))
            .and_then(|size| size.checked_sub(8))
            .and_then(|size| size.checked_sub(self.signature))
            .and_then(|size| size.checked_sub(self.padding))
            .filter(|size| *size > 0)
            .ok_or_else(|| limited("send buffer cannot hold a message body"))
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct MessageLimits {
    pub(super) bytes: usize,
    pub(super) chunks: u32,
}

impl MessageLimits {
    pub(super) fn check(self, bytes: usize, chunks: u32) -> Result<()> {
        if bytes > self.bytes {
            return Err(limited("message size exceeds negotiated limit"));
        }
        if chunks > self.chunks {
            return Err(limited("chunk count exceeds negotiated limit"));
        }
        Ok(())
    }

    /// Preflight the whole body before a caller emits any chunk or consumes sequences.
    pub(super) fn plan(self, bytes: usize, buffer: usize) -> Result<(usize, u32)> {
        let capacity = SecuritySizes::NONE.max_body(buffer, 4)?;
        let count = bytes.div_ceil(capacity);
        let count = u32::try_from(count).map_err(|_range| limited("chunk count overflow"))?;
        self.check(bytes, count)?;
        Ok((capacity, count))
    }
}

#[derive(Debug, Default)]
pub(super) struct Assembly {
    body: Vec<u8>,
    chunks: u32,
}

impl Assembly {
    pub(super) const fn len(&self) -> usize {
        self.body.len()
    }

    fn reserve(&mut self, additional: usize) -> Result<()> {
        self.body
            .try_reserve_exact(additional)
            .map_err(|_allocation| limited("message allocation failed"))
    }

    /// Checks both bounds before reserving; failure leaves the partial message untouched.
    pub(super) fn push(
        &mut self,
        bytes: &[u8],
        final_chunk: bool,
        limits: MessageLimits,
    ) -> Result<Option<Bytes>> {
        let count = self
            .chunks
            .checked_add(1)
            .ok_or_else(|| limited("chunk count overflow"))?;
        let length = check_growth(self.body.len(), bytes.len(), limits.bytes)?;
        limits.check(length, count)?;
        self.reserve(bytes.len())?;
        self.body.extend_from_slice(bytes);
        self.chunks = count;
        if final_chunk {
            self.chunks = 0;
            Ok(Some(Bytes::from(std::mem::take(&mut self.body))))
        } else {
            Ok(None)
        }
    }
}

/// Shared by per-message and aggregate receive budgets; rejects overflow before allocation.
pub(super) fn check_growth(current: usize, additional: usize, maximum: usize) -> Result<usize> {
    current
        .checked_add(additional)
        .filter(|length| *length <= maximum)
        .ok_or_else(|| limited("receive byte budget exceeded"))
}

/// A peer ABORT is request-local and retains its exact status, never its reason text.
pub(super) fn abort(body: &[u8]) -> Result<PlcError> {
    let mut reader = BoundedReader::new(body);
    let status = StatusCode::parse(&mut reader, Depth::new())?;
    let _reason = UaString::parse(&mut reader, Depth::new())?;
    reader.expect_eof()?;
    Ok(status_error(status))
}

pub(super) fn limited(message: &str) -> PlcError {
    PlcError::new(ErrorKind::ResourceLimit, format!("OPC UA chunk: {message}"))
}

#[cfg(test)]
mod tests;
