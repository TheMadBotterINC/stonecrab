//! OPN signatures and blockwise RSA-OAEP; no sequence or service parsing before authentication.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-ASYMMETRIC
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-PADDING
//! # Compatibility: docs/parity/opcua.md#OPCUA-MODE-OPN-PROTECTED

use std::fmt;

use aws_lc_rs::constant_time::verify_slices_are_equal;
use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::{
    OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepAlgorithm, OaepPrivateDecryptingKey,
    OaepPublicEncryptingKey,
};
use aws_lc_rs::signature::{RSA_PKCS1_SHA256, RSA_PSS_SHA256, RsaEncoding};
use bytes::Bytes;
use rustls_pki_types::CertificateDer;

use super::certificate::{self, PKCS1_SHA256, PSS_SHA256};
use super::{KeyMaterial, invalid};
use crate::codec::{BoundedReader, BoundedWriter};
use crate::opcua::chunk::SecuritySizes;
use crate::opcua::codec::{LITTLE, UaByteString, UaString};
use crate::opcua::frame::{AsymmetricHeader, Body, Frame, FrameHeader, MessageType};
use crate::opcua::options::SecurityPolicy;
use crate::{ErrorKind, PlcError, Result, Secret};

fn rejected() -> PlcError {
    PlcError::new(
        ErrorKind::Authentication,
        "OPC UA asymmetric message security verification failed",
    )
}

fn limited() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "OPC UA protected OPN chunk exceeds its byte limit",
    )
}

pub(in crate::opcua) struct Asymmetric {
    pub(super) policy: SecurityPolicy,
    pub(super) local: KeyMaterial,
    pub(super) peer: CertificateDer<'static>,
    encrypting: OaepPublicEncryptingKey,
    decrypting: Secret<OaepPrivateDecryptingKey>,
    sizes: SecuritySizes,
}

impl fmt::Debug for Asymmetric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Asymmetric")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

impl Asymmetric {
    pub(in crate::opcua) fn new(
        policy: SecurityPolicy,
        local: KeyMaterial,
        peer: Option<CertificateDer<'static>>,
    ) -> Result<Self> {
        if !policy.is_secure() {
            return Err(invalid());
        }
        let peer = peer.ok_or_else(|| {
            PlcError::new(
                ErrorKind::InvalidConfig,
                "OPC UA protected OPN requires an RSA server-certificate-file before I/O",
            )
        })?;
        let encrypting = OaepPublicEncryptingKey::new(certificate::server_key(&peer, policy)?)
            .ok()
            .ok_or_else(invalid)?;
        let sizes = policy.descriptor().asymmetric_sizes(
            local.signing.expose().public_modulus_len(),
            encrypting.key_size_bytes(),
        )?;
        let decrypting = OaepPrivateDecryptingKey::new(local.decrypting.expose().clone())
            .ok()
            .ok_or_else(invalid)?;
        Ok(Self {
            policy,
            local,
            peer,
            encrypting,
            decrypting: Secret::new(decrypting),
            sizes,
        })
    }

    pub(in crate::opcua) fn header(&self) -> AsymmetricHeader {
        AsymmetricHeader {
            security_policy_uri: UaString::new(self.policy.descriptor().uri),
            sender_certificate: UaByteString::new(self.local.sender_certificate.clone()),
            receiver_thumbprint: UaByteString::new(
                digest(&SHA1_FOR_LEGACY_USE_ONLY, self.peer.as_ref())
                    .as_ref()
                    .to_vec(),
            ),
        }
    }

    pub(in crate::opcua) fn max_body(&self, maximum: usize) -> Result<usize> {
        self.sizes
            .max_body(maximum, self.header().length_in_bytes()?)
    }

    fn oaep(&self) -> &'static OaepAlgorithm {
        if self.policy == SecurityPolicy::Aes256Sha256RsaPss {
            &OAEP_SHA256_MGF1SHA256
        } else {
            &OAEP_SHA1_MGF1SHA1
        }
    }

    fn encoding(&self) -> &'static dyn RsaEncoding {
        if self.policy == SecurityPolicy::Aes256Sha256RsaPss {
            &RSA_PSS_SHA256
        } else {
            &RSA_PKCS1_SHA256
        }
    }

    /// Every secure OPN is signed and encrypted, including a channel in SIGN mode.
    pub(in crate::opcua) fn seal(&self, frame: &Frame, maximum: usize) -> Result<Bytes> {
        let Body::Open(chunk) = &frame.body else {
            return Err(invalid());
        };
        if chunk.security != self.header() {
            return Err(invalid());
        }
        if chunk.body.len() > self.max_body(maximum)? {
            return Err(limited());
        }
        let clear = chunk
            .security
            .length_in_bytes()?
            .checked_add(12)
            .ok_or_else(limited)?;
        let plain = frame.length_in_bytes()?;
        let signature_size = self.local.signing.expose().public_modulus_len();
        let block = self.encrypting.max_plaintext_size(self.oaep());
        let overhead = padding_overhead(self.encrypting.key_size_bytes());
        let count = plain
            .checked_sub(clear)
            .and_then(|size| size.checked_add(signature_size))
            .and_then(|size| size.checked_add(overhead))
            .ok_or_else(limited)?;
        let extra = block
            .checked_sub(count.checked_rem(block).ok_or_else(invalid)?)
            .and_then(|size| size.checked_rem(block))
            .ok_or_else(invalid)?;
        let padded = count.checked_add(extra).ok_or_else(limited)?;
        let total = padded
            .checked_div(block)
            .and_then(|blocks| blocks.checked_mul(self.encrypting.key_size_bytes()))
            .and_then(|size| size.checked_add(clear))
            .filter(|size| *size <= maximum)
            .ok_or_else(limited)?;
        let size = u32::try_from(total)
            .ok()
            .filter(|size| *size <= 0x7fff_ffff)
            .ok_or_else(limited)?;
        let mut writer = BoundedWriter::new(total);
        frame.serialize(size, &mut writer)?;
        let [low, high] = u16::try_from(extra).ok().ok_or_else(invalid)?.to_le_bytes();
        for _ in 0..extra.saturating_add(1) {
            writer.write_u8(low)?;
        }
        if overhead == 2 {
            writer.write_u8(high)?;
        }
        let signature = self.sign(writer.as_slice())?;
        writer.write_bytes(&signature)?;
        let plaintext = writer.as_slice().get(clear..).ok_or_else(invalid)?;
        let mut out = BoundedWriter::new(total);
        out.write_bytes(writer.as_slice().get(..clear).ok_or_else(invalid)?)?;
        let mut ciphertext = vec![0; self.encrypting.ciphertext_size()];
        for part in plaintext.chunks_exact(block) {
            let encrypted = self
                .encrypting
                .encrypt(self.oaep(), part, &mut ciphertext, None)
                .ok()
                .ok_or_else(rejected)?;
            out.write_bytes(encrypted)?;
        }
        Ok(Bytes::from(out.into_bytes()))
    }

    pub(super) fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        let mut signature = vec![0; self.local.signing.expose().public_modulus_len()];
        self.local
            .signing
            .expose()
            .sign(
                self.encoding(),
                &SystemRandom::new(),
                message,
                &mut signature,
            )
            .ok()
            .ok_or_else(rejected)?;
        Ok(signature)
    }

    pub(in crate::opcua) fn open(&self, bytes: &[u8], maximum: usize) -> Result<Frame> {
        if bytes.len() > maximum {
            return Err(limited());
        }
        self.authenticate(bytes).map_err(|_error| rejected())
    }

    fn authenticate(&self, bytes: &[u8]) -> Result<Frame> {
        let mut reader = BoundedReader::new(bytes);
        let header = FrameHeader::parse(&mut reader)?;
        if header.message_type != MessageType::Open
            || usize::try_from(header.size).ok() != Some(bytes.len())
        {
            return Err(rejected());
        }
        let _channel_id = reader.read_u32(LITTLE)?;
        let security = AsymmetricHeader::parse(&mut reader)?;
        if security.security_policy_uri.as_bytes() != Some(self.policy.descriptor().uri.as_bytes())
        {
            return Err(rejected());
        }
        verify_slices_are_equal(
            security
                .receiver_thumbprint
                .as_bytes()
                .ok_or_else(rejected)?,
            &self.local.thumbprint,
        )
        .ok()
        .ok_or_else(rejected)?;
        let chain = security
            .sender_certificate
            .as_bytes()
            .ok_or_else(rejected)?;
        let leaf = first_certificate(chain)?;
        verify_slices_are_equal(leaf, self.peer.as_ref())
            .ok()
            .ok_or_else(rejected)?;
        let clear = bytes.len().saturating_sub(reader.remaining());
        let ciphertext = bytes.get(clear..).ok_or_else(rejected)?;
        let key = self.decrypting.expose();
        if !ciphertext.len().is_multiple_of(key.key_size_bytes()) {
            return Err(rejected());
        }
        let mut plain = bytes.get(..clear).ok_or_else(rejected)?.to_vec();
        let mut scratch = vec![0; key.min_output_size()];
        let block = key
            .key_size_bytes()
            .saturating_sub(self.policy.descriptor().oaep_overhead);
        for part in ciphertext.chunks_exact(key.key_size_bytes()) {
            let decrypted = key
                .decrypt(self.oaep(), part, &mut scratch, None)
                .ok()
                .ok_or_else(rejected)?;
            if decrypted.len() != block {
                return Err(rejected());
            }
            plain.extend_from_slice(decrypted);
        }
        let auth_end = plain
            .len()
            .checked_sub(self.encrypting.key_size_bytes())
            .filter(|end| *end >= clear.saturating_add(8))
            .ok_or_else(rejected)?;
        let message = plain.get(..auth_end).ok_or_else(rejected)?;
        let signature = plain.get(auth_end..).ok_or_else(rejected)?;
        self.verify(message, signature)?;
        let end = unpad(message, clear, key.key_size_bytes())?;
        let body = Body::parse(MessageType::Open, plain.get(8..end).ok_or_else(rejected)?)?;
        Ok(Frame {
            chunk: header.chunk,
            body,
        })
    }

    pub(super) fn verify(&self, message: &[u8], signature: &[u8]) -> Result<()> {
        let algorithm = if self.policy == SecurityPolicy::Aes256Sha256RsaPss {
            &PSS_SHA256
        } else {
            &PKCS1_SHA256
        };
        webpki::EndEntityCert::try_from(&self.peer)
            .ok()
            .ok_or_else(rejected)?
            .verify_signature(algorithm, message, signature)
            .map_err(|_error| rejected())
    }
}

const fn padding_overhead(modulus: usize) -> usize {
    if modulus > 256 { 2 } else { 1 }
}

/// Only the first, bounded DER certificate selects the peer key; issuers cannot replace it.
fn first_certificate(bytes: &[u8]) -> Result<&[u8]> {
    let mut reader = BoundedReader::new(bytes);
    let (tag, _value) = certificate::tlv(&mut reader).ok_or_else(rejected)?;
    if tag != 0x30 {
        return Err(rejected());
    }
    bytes
        .get(..bytes.len().saturating_sub(reader.remaining()))
        .ok_or_else(rejected)
}

fn unpad(message: &[u8], clear: usize, modulus: usize) -> Result<usize> {
    let overhead = padding_overhead(modulus);
    let (low, high) = if overhead == 2 {
        let last = message.len().checked_sub(2).ok_or_else(rejected)?;
        let pair: [u8; 2] = message
            .get(last..)
            .ok_or_else(rejected)?
            .try_into()
            .ok()
            .ok_or_else(rejected)?;
        let [low, high] = pair;
        (low, high)
    } else {
        (*message.last().ok_or_else(rejected)?, 0)
    };
    let extra = usize::from(u16::from_le_bytes([low, high]));
    let end = message
        .len()
        .checked_sub(extra.saturating_add(overhead))
        .filter(|end| *end >= clear.saturating_add(8))
        .ok_or_else(rejected)?;
    let low_end = message.len().saturating_sub(overhead.saturating_sub(1));
    if !message
        .get(end..low_end)
        .ok_or_else(rejected)?
        .iter()
        .all(|byte| *byte == low)
    {
        return Err(rejected());
    }
    Ok(end)
}

#[cfg(test)]
mod tests;

#[cfg(fuzzing)]
pub(super) mod fuzzing;
