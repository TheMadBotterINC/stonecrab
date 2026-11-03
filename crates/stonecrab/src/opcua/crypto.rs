//! Private cryptographic effects and policy-bound key material.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-KEYDERIVATION
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-NONCE

mod backend;
mod derive;
mod policy;
mod symmetric;
pub(super) use symmetric::Symmetric;

#[cfg(test)]
pub(super) use backend::Hash;
pub(super) use backend::{Backend, Native};
pub(super) use derive::derive_keys;
pub(super) use policy::Policy;

use crate::opcua::codec::UaByteString;
use crate::opcua::options::SecurityPolicy;
use crate::{ErrorKind, PlcError, Result};

fn invalid() -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        "OPC UA cryptographic operation failed",
    )
}

/// `CreateSession` requires a fresh 32-byte nonce even on a None channel.
pub(super) fn session_nonce(backend: &dyn Backend) -> Result<[u8; 32]> {
    let mut bytes = [0; 32];
    backend.fill_random(&mut bytes)?;
    Ok(bytes)
}

/// None has no nonce and consumes no entropy; all accepted RSA policies use 32 bytes.
pub(super) fn channel_nonce(backend: &dyn Backend, policy: SecurityPolicy) -> Result<UaByteString> {
    if policy.descriptor().nonce == 0 {
        return Ok(UaByteString::null());
    }
    Ok(UaByteString::new(session_nonce(backend)?))
}

#[cfg(test)]
mod tests;
