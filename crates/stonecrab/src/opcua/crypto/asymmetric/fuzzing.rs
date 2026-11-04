//! Ephemeral keys and input-directed protected OPN probes; cargo-fuzz only.

use crate::Result;

/// Ephemeral self-peer key material for decrypt-then-parse fuzzing; no file I/O.
#[derive(Debug)]
pub struct OpnProbe {
    engines: Vec<crate::opcua::crypto::Asymmetric>,
}

impl OpnProbe {
    /// Replace a public certificate template's SPKI with a fresh key of the same size.
    /// Certificate trust is outside this crypto-only harness; no private fixture is stored.
    ///
    /// # Errors
    /// InvalidConfig if key generation or the fixed public template is unavailable.
    pub fn new(template: &[u8]) -> Result<Self> {
        use crate::Secret;
        use crate::opcua::crypto::{Asymmetric, KeyMaterial};
        use crate::opcua::options::SecurityPolicy;
        use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
        use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
        use aws_lc_rs::rsa::{KeySize, PrivateDecryptingKey};
        use aws_lc_rs::signature::{KeyPair, RsaKeyPair};
        use rustls_pki_types::CertificateDer;

        let invalid = || {
            crate::PlcError::new(
                crate::ErrorKind::InvalidConfig,
                "OPN fuzz key preparation failed",
            )
        };
        let pair = RsaKeyPair::generate(KeySize::Rsa2048).map_err(|_error| invalid())?;
        let new_spki: PublicKeyX509Der<'_> =
            pair.public_key().as_der().map_err(|_error| invalid())?;
        let der = CertificateDer::from(template);
        let certificate = webpki::EndEntityCert::try_from(&der).map_err(|_error| invalid())?;
        let old_spki = certificate.subject_public_key_info();
        let position = template
            .windows(old_spki.as_ref().len())
            .position(|bytes| bytes == old_spki.as_ref())
            .ok_or_else(invalid)?;
        let end = position
            .checked_add(old_spki.as_ref().len())
            .ok_or_else(invalid)?;
        let mut leaf = template.to_vec();
        let target = leaf
            .get_mut(position..end)
            .filter(|target| target.len() == new_spki.as_ref().len())
            .ok_or_else(invalid)?;
        target.copy_from_slice(new_spki.as_ref());
        let pkcs8: Pkcs8V1Der<'_> = pair.as_der().map_err(|_error| invalid())?;
        let mut engines = Vec::new();
        for policy in [
            SecurityPolicy::Basic256Sha256,
            SecurityPolicy::Aes128Sha256RsaOaep,
            SecurityPolicy::Aes256Sha256RsaPss,
        ] {
            let local = KeyMaterial {
                leaf: CertificateDer::from(leaf.clone()),
                sender_certificate: leaf.clone(),
                thumbprint: digest(&SHA1_FOR_LEGACY_USE_ONLY, &leaf).as_ref().to_vec(),
                application_uri: "urn:stonecrab:fuzz".to_owned(),
                signing: Secret::new(
                    RsaKeyPair::from_pkcs8(pkcs8.as_ref()).map_err(|_error| invalid())?,
                ),
                decrypting: Secret::new(
                    PrivateDecryptingKey::from_pkcs8(pkcs8.as_ref()).map_err(|_error| invalid())?,
                ),
            };
            engines.push(Asymmetric::new(
                policy,
                local,
                Some(CertificateDer::from(leaf.clone())),
            )?);
        }
        Ok(Self { engines })
    }

    /// Exercise raw ciphertext plus authenticated payloads and input-directed wire mutations.
    ///
    /// # Errors
    /// Internal for a round-trip mismatch, or a cryptographic preparation failure.
    pub fn inspect(&self, input: &[u8]) -> Result<()> {
        use crate::opcua::frame::{Body, Frame, OpenChunk, SequenceHeader};
        let selector = usize::from(input.first().copied().unwrap_or(0));
        let engine = self
            .engines
            .get(selector.checked_rem(self.engines.len()).unwrap_or(0))
            .ok_or_else(|| {
                crate::PlcError::new(crate::ErrorKind::Internal, "missing OPN fuzz engine")
            })?;
        let _raw = engine.open(input, 8192);
        let _small_padding = super::unpad(input, 0, 256);
        let _large_padding = super::unpad(input, 0, 512);
        let body = input.get(..input.len().min(2048)).unwrap_or_default();
        let frame = Frame::single(Body::Open(OpenChunk {
            channel_id: 17,
            security: engine.header(),
            sequence: SequenceHeader {
                sequence_number: 1,
                request_id: 1,
            },
            body: bytes::Bytes::copy_from_slice(body),
        }));
        let wire = engine.seal(&frame, 8192)?;
        if engine.open(&wire, 8192)? != frame {
            return Err(crate::PlcError::new(
                crate::ErrorKind::Internal,
                "OPN fuzz round trip changed the frame",
            ));
        }
        let mut changed = wire.to_vec();
        for pair in input.chunks_exact(2).take(32) {
            if let [position, value] = pair {
                let offset = usize::from(*position).saturating_mul(selector.saturating_add(1));
                let index = offset.checked_rem(changed.len()).unwrap_or(0);
                if let Some(byte) = changed.get_mut(index) {
                    *byte ^= value;
                }
            }
        }
        let _mutated = engine.open(&changed, 8192);
        Ok(())
    }
}
