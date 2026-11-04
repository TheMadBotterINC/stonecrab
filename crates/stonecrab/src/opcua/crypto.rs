//! Private cryptographic effects and policy-bound key material.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-KEYDERIVATION
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-NONCE

mod asymmetric;
mod backend;
pub(super) mod certificate;
mod derive;
mod keystore;
mod policy;
mod symmetric;
mod trust;
pub(super) use asymmetric::Asymmetric;
pub(super) use symmetric::Symmetric;

#[cfg(test)]
pub(super) use backend::Hash;
pub(super) use backend::{Backend, Native};
pub(super) use derive::derive_keys;
pub(super) use keystore::KeyMaterial;
pub(super) use policy::Policy;
pub(super) use trust::Verifier;

use crate::opcua::codec::UaByteString;
use crate::opcua::options::SecurityPolicy;
use crate::{ErrorKind, PlcError, Result};

/// A provisioned peer and client identity shared throughout one connection handshake.
#[derive(Debug)]
pub(super) struct Security {
    pub(super) asymmetric: std::sync::Arc<Asymmetric>,
    verifier: Verifier,
}

impl Security {
    pub(super) fn prepare(
        options: &crate::opcua::options::OpcuaOptions,
        now: std::time::SystemTime,
    ) -> Result<Option<Self>> {
        if options.security_policy == SecurityPolicy::None {
            return Ok(None);
        }
        if options.server_certificate_file.is_none() {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "OPC UA secure policy requires server-certificate-file before I/O; tls.trust-store or tls.verify=false alone cannot bootstrap it; select security-policy=NONE explicitly for an unprotected connection",
            ));
        }
        let material = KeyMaterial::load(options.keystore.as_deref())?;
        let (server, verifier) = Verifier::prepare(options)?;
        let asymmetric = Asymmetric::new(options.security_policy, material, server)?;
        if options.security_policy != SecurityPolicy::Basic256Sha256 {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "OPC UA this secure policy is not implemented; use Basic256Sha256",
            ));
        }
        let security = Self {
            asymmetric: std::sync::Arc::new(asymmetric),
            verifier,
        };
        security.verify(security.asymmetric.peer.as_ref(), now)?;
        Ok(Some(security))
    }

    pub(super) fn policy(&self) -> SecurityPolicy {
        self.asymmetric.policy
    }

    pub(super) fn application_uri(&self) -> &str {
        &self.asymmetric.local.application_uri
    }
    pub(super) fn certificate(&self) -> &[u8] {
        &self.asymmetric.local.sender_certificate
    }

    pub(super) fn verify(&self, bytes: &[u8], now: std::time::SystemTime) -> Result<()> {
        let chain = certificate::chain(bytes)?;
        self.verifier.verify(&chain, self.asymmetric.policy, now)?;
        let leaf = chain.first().ok_or_else(untrusted)?;
        aws_lc_rs::constant_time::verify_slices_are_equal(
            leaf.as_ref(),
            self.asymmetric.peer.as_ref(),
        )
        .ok()
        .ok_or_else(untrusted)
    }

    pub(super) fn check_server_signature(
        &self,
        signature: &crate::opcua::generated::signature_data::SignatureData,
        nonce: &[u8],
        now: std::time::SystemTime,
    ) -> Result<()> {
        self.verify(self.asymmetric.peer.as_ref(), now)?;
        if signature.algorithm.as_bytes()
            != Some(self.asymmetric.policy.descriptor().signature_uri.as_bytes())
        {
            return Err(PlcError::new(
                ErrorKind::Authentication,
                "OPC UA invalid server signature algorithm",
            ));
        }
        let signature = signature.signature.as_bytes().ok_or_else(|| {
            PlcError::new(ErrorKind::Authentication, "OPC UA missing server signature")
        })?;
        self.asymmetric
            .verify(&[self.certificate(), nonce].concat(), signature)
    }

    pub(super) fn client_signature(
        &self,
        certificate: &[u8],
        nonce: &[u8],
    ) -> Result<crate::opcua::generated::signature_data::SignatureData> {
        Ok(crate::opcua::generated::signature_data::SignatureData {
            algorithm: crate::opcua::codec::UaString::new(
                self.asymmetric.policy.descriptor().signature_uri,
            ),
            signature: UaByteString::new(self.asymmetric.sign(&[certificate, nonce].concat())?),
        })
    }
}

fn invalid() -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        "OPC UA cryptographic operation failed",
    )
}

fn untrusted() -> PlcError {
    PlcError::new(
        ErrorKind::Trust,
        "OPC UA server certificate is not trusted; configure server-certificate-file or tls.trust-store, select security-policy=NONE, or explicitly set tls.verify=false",
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
pub(super) mod tests;

#[cfg(fuzzing)]
pub use asymmetric::fuzzing::OpnProbe;
