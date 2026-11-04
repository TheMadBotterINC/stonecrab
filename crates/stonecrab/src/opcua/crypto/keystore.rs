//! Bounded application-instance PEM loading; parsing never establishes certificate trust.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CFG-KEYSTORE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CFG-KEYSTORE-TYPE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-KEYSTORE
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-CHAIN
//! # Compatibility: docs/parity/opcua.md#OPCUA-CERT-APPLICATION-URI

use std::fmt;
use std::fs::File;
use std::io::Read;

use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use aws_lc_rs::encoding::{AsDer, PublicKeyX509Der};
use aws_lc_rs::rsa::PrivateDecryptingKey;
use aws_lc_rs::signature::{KeyPair, RsaKeyPair};
use rustls_pki_types::CertificateDer;
use rustls_pki_types::pem::{PemObject, SectionKind};

use crate::{ErrorKind, PlcError, Result, Secret};

const MAX_PEM_BYTES: u64 = 1 << 20;

/// Native handles own the key; neither diagnostics nor errors format their provider data.
pub(in crate::opcua) struct KeyMaterial {
    pub(super) leaf: CertificateDer<'static>,
    pub(super) sender_certificate: Vec<u8>,
    pub(super) thumbprint: Vec<u8>,
    pub(super) application_uri: String,
    pub(super) signing: Secret<RsaKeyPair>,
    pub(super) decrypting: Secret<PrivateDecryptingKey>,
}

impl fmt::Debug for KeyMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KeyMaterial([REDACTED])")
    }
}

fn invalid(reason: &str) -> PlcError {
    PlcError::new(
        ErrorKind::InvalidConfig,
        format!("OPC UA tls.keystore {reason}"),
    )
}

fn unreadable() -> PlcError {
    invalid("could not be read")
}

fn malformed() -> PlcError {
    invalid("must contain valid PEM certificates and one unencrypted RSA PKCS#8 private key")
}

fn read_pem(reader: impl Read) -> Result<Secret<Vec<u8>>> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_PEM_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()
        .ok_or_else(unreadable)?;
    if u64::try_from(bytes.len())
        .ok()
        .is_none_or(|len| len > MAX_PEM_BYTES)
    {
        return Err(invalid("exceeds the 1 MiB file limit"));
    }
    Ok(Secret::new(bytes))
}

impl KeyMaterial {
    pub(in crate::opcua) fn load(path: Option<&str>) -> Result<Self> {
        let path = path.ok_or_else(|| {
            invalid(
                "is required for a secure policy; generated client certificates are not offered",
            )
        })?;
        // Do not attach the I/O source: OS errors may contain a configured path.
        let file = File::open(path).ok().ok_or_else(unreadable)?;
        let bytes = read_pem(file)?;
        Self::from_pem(bytes.expose())
    }

    fn from_pem(bytes: &[u8]) -> Result<Self> {
        // The general PEM reader skips unknown sections, including encrypted keys.
        // A key store must not silently accept one beside an otherwise usable key.
        for line in bytes.split(|byte| *byte == b'\n' || *byte == b'\r') {
            if line.starts_with(b"-----BEGIN ")
                && line != b"-----BEGIN CERTIFICATE-----"
                && line != b"-----BEGIN PRIVATE KEY-----"
            {
                return Err(malformed());
            }
        }
        let mut certificates = Vec::new();
        let mut private_key = None;
        for section in <(SectionKind, Vec<u8>)>::pem_slice_iter(bytes) {
            let (kind, der) = section.ok().ok_or_else(malformed)?;
            if kind == SectionKind::Certificate {
                let certificate = CertificateDer::from(der);
                webpki::EndEntityCert::try_from(&certificate)
                    .ok()
                    .ok_or_else(malformed)?;
                certificates.push(certificate);
            } else if kind == SectionKind::PrivateKey && private_key.is_none() {
                private_key = Some(Secret::new(der));
            } else {
                return Err(malformed());
            }
        }
        let private_key = private_key.ok_or_else(malformed)?;
        let signing = RsaKeyPair::from_pkcs8(private_key.expose())
            .ok()
            .ok_or_else(malformed)?;
        let decrypting = PrivateDecryptingKey::from_pkcs8(private_key.expose())
            .ok()
            .ok_or_else(malformed)?;
        let sender_certificate = certificates
            .iter()
            .flat_map(AsRef::as_ref)
            .copied()
            .collect();
        let leaf = certificates.into_iter().next().ok_or_else(malformed)?;
        let certificate = webpki::EndEntityCert::try_from(&leaf)
            .ok()
            .ok_or_else(malformed)?;
        let public_key = AsDer::<PublicKeyX509Der<'_>>::as_der(signing.public_key())
            .ok()
            .ok_or_else(malformed)?;
        if public_key.as_ref() != certificate.subject_public_key_info().as_ref() {
            return Err(invalid(
                "requires the leaf certificate first and a matching RSA private key",
            ));
        }
        let application_uri = {
            let mut uris = certificate.valid_uri_names();
            let application_uri = uris.next().filter(|uri| absolute_uri(uri)).ok_or_else(|| {
                invalid("leaf certificate requires one absolute application URI in subjectAltName")
            })?;
            if uris.next().is_some() {
                return Err(invalid(
                    "leaf certificate must have exactly one application URI in subjectAltName",
                ));
            }
            application_uri.to_owned()
        };
        let thumbprint = digest(&SHA1_FOR_LEGACY_USE_ONLY, leaf.as_ref())
            .as_ref()
            .to_vec();
        Ok(Self {
            leaf,
            sender_certificate,
            thumbprint,
            application_uri,
            signing: Secret::new(signing),
            decrypting: Secret::new(decrypting),
        })
    }
}

fn absolute_uri(uri: &str) -> bool {
    let Some((scheme, rest)) = uri.split_once(':') else {
        return false;
    };
    !rest.is_empty()
        && scheme.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && scheme
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+.-".contains(&byte))
        && uri.bytes().all(|byte| byte.is_ascii_graphic())
}

#[cfg(test)]
mod tests;
