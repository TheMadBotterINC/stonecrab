//! Certificate-file preparation and explicit trust strategies; no discovery trust.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CFG-SERVER-CERTIFICATE-FILE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CFG-TRUST-STORE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CFG-VERIFY
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-VERIFY-ORDER
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-VERIFY-PERMISSIVE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-VERIFY-TRUSTSTORE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-VERIFY-PINNED
//! # Compatibility: docs/parity/opcua.md#OPCUA-POLICY-DEFAULT
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-NONE-FALLBACK
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-FAIL-CLOSED
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-JVM-KEYSTORES

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::constant_time::verify_slices_are_equal;
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, UnixTime};
use webpki::{EndEntityCert, KeyUsage, anchor_from_trusted_cert};

use super::certificate::{self, MAX_CERT_BYTES, PKCS1_SHA256, PSS_SHA256};
use super::untrusted;
use crate::opcua::options::{CertificateFormat, OpcuaOptions, SecurityPolicy};
use crate::{ErrorKind, PlcError, Result};

pub(in crate::opcua) enum Verifier {
    Permissive,
    Store(Vec<CertificateDer<'static>>),
    Pinned(CertificateDer<'static>),
    Reject,
}

impl fmt::Debug for Verifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Verifier")
            .field("strategy", &self.name())
            .finish()
    }
}

impl Verifier {
    const fn name(&self) -> &'static str {
        match self {
            Self::Permissive => "permissive",
            Self::Store(_) => "trust-store",
            Self::Pinned(_) => "pinned",
            Self::Reject => "reject",
        }
    }

    pub(in crate::opcua) fn prepare(
        options: &OpcuaOptions,
        policy: SecurityPolicy,
    ) -> Result<(Option<CertificateDer<'static>>, Self)> {
        let bootstrap = options
            .server_certificate_file
            .as_deref()
            .map(|path| -> Result<_> {
                let [certificate]: [CertificateDer<'static>; 1] =
                    load(path, "server-certificate-file", None)?
                        .try_into()
                        .map_err(|_certificates| invalid("server-certificate-file"))?;
                Ok(certificate)
            })
            .transpose()?;
        if let Some(der) = &bootstrap {
            certificate::server_key(der, policy)?;
        }
        let verifier = if !options.verify {
            Self::Permissive
        } else if let Some(path) = &options.trust_store {
            Self::Store(load(
                path,
                "tls.trust-store",
                Some(options.trust_store_type),
            )?)
        } else if let Some(der) = &bootstrap {
            Self::Pinned(der.clone())
        } else {
            Self::Reject
        };
        tracing::debug!(
            certificate_verifier = verifier.name(),
            tls_verify = options.verify,
            "OPC UA certificate verifier configured"
        );
        if !options.verify {
            tracing::warn!(
                certificate_verifier = verifier.name(),
                tls_verify = false,
                "OPC UA certificate trust verification disabled by tls.verify=false"
            );
        }
        Ok((bootstrap, verifier))
    }

    pub(in crate::opcua) fn verify(
        &self,
        chain: &[CertificateDer<'_>],
        policy: SecurityPolicy,
        now: SystemTime,
    ) -> Result<()> {
        match self {
            Self::Permissive => Ok(()),
            Self::Reject => Err(untrusted()),
            Self::Pinned(pin) => {
                bounded_chain(chain)?;
                let leaf = chain.first().ok_or_else(untrusted)?;
                verify_slices_are_equal(pin.as_ref(), leaf.as_ref())
                    .ok()
                    .ok_or_else(untrusted)?;
                certificate::check_leaf(leaf, policy, now).map(|_certificate| ())
            }
            Self::Store(store) => {
                bounded_chain(chain)?;
                let (leaf, intermediates) = chain.split_first().ok_or_else(untrusted)?;
                let certificate = certificate::check_leaf(leaf, policy, now)?;
                if store
                    .iter()
                    .any(|anchor| verify_slices_are_equal(anchor.as_ref(), leaf.as_ref()).is_ok())
                {
                    return Ok(());
                }
                let anchors = store
                    .iter()
                    .filter(|der| certificate::current(der, now))
                    .map(anchor_from_trusted_cert)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .ok()
                    .ok_or_else(untrusted)?;
                let time = UnixTime::since_unix_epoch(
                    now.duration_since(UNIX_EPOCH).ok().ok_or_else(untrusted)?,
                );
                certificate
                    .verify_for_usage(
                        &[&PKCS1_SHA256, &PSS_SHA256],
                        &anchors,
                        intermediates,
                        time,
                        KeyUsage::server_auth(),
                        None,
                        None,
                    )
                    .ok()
                    .ok_or_else(untrusted)?;
                Ok(())
            }
        }
    }

    pub(in crate::opcua) fn verify_bound(
        &self,
        bytes: &[u8],
        peer: &CertificateDer<'_>,
        policy: SecurityPolicy,
        now: SystemTime,
    ) -> Result<()> {
        let chain = certificate::chain(bytes)?;
        self.verify(&chain, policy, now)?;
        let leaf = chain.first().ok_or_else(untrusted)?;
        verify_slices_are_equal(leaf.as_ref(), peer.as_ref())
            .ok()
            .ok_or_else(untrusted)
    }
}

fn bounded_chain(chain: &[CertificateDer<'_>]) -> Result<()> {
    if chain.len() > 16
        || chain
            .iter()
            .try_fold(0_usize, |sum, cert| sum.checked_add(cert.len()))
            .is_none_or(|bytes| bytes > MAX_CERT_BYTES)
    {
        return Err(untrusted());
    }
    Ok(())
}

fn invalid(option: &str) -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        format!(
            "OPC UA {option} requires a readable certificate file within 1 MiB, in the selected DER/PEM format"
        ),
    )
}

fn read(reader: &mut dyn Read, option: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    if reader.take(1_048_577).read_to_end(&mut bytes).is_err() {
        return Err(invalid(option));
    }
    if bytes.len() > MAX_CERT_BYTES {
        return Err(invalid(option));
    }
    Ok(bytes)
}

fn load(
    path: &str,
    option: &str,
    format: Option<CertificateFormat>,
) -> Result<Vec<CertificateDer<'static>>> {
    let mut file = File::open(path).ok().ok_or_else(|| invalid(option))?;
    parse(&read(&mut file, option)?, format).ok_or_else(|| invalid(option))
}

pub(super) fn parse(
    bytes: &[u8],
    format: Option<CertificateFormat>,
) -> Option<Vec<CertificateDer<'static>>> {
    if bytes.len() > MAX_CERT_BYTES {
        return None;
    }
    let Some(format) = format else {
        return parse(bytes, Some(CertificateFormat::Der))
            .or_else(|| parse(bytes, Some(CertificateFormat::Pem)));
    };
    let certificates = if format == CertificateFormat::Pem {
        for line in bytes.split(|byte| *byte == b'\r' || *byte == b'\n') {
            if line.starts_with(b"-----BEGIN ") && line != b"-----BEGIN CERTIFICATE-----" {
                return None;
            }
        }
        CertificateDer::pem_slice_iter(bytes)
            .collect::<std::result::Result<Vec<_>, _>>()
            .ok()?
    } else {
        vec![CertificateDer::from(bytes.to_vec())]
    };
    if certificates.is_empty() {
        return None;
    }
    for cert in &certificates {
        EndEntityCert::try_from(cert).ok()?;
    }
    Some(certificates)
}

#[cfg(test)]
mod tests;
