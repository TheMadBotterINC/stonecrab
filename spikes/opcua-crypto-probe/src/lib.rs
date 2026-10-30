//! The cryptography probe of slice S82 (decision E15, OPC UA security policies).
//!
//! This crate exists to be built and tested on the CI matrix; it is not part of Stonecrab. Its
//! tests exercise the primitives the OPC UA security policies need through `aws-lc-rs`, and
//! the one piece of first-party glue they need: the signature verification algorithm that
//! `rustls-webpki` asks its caller for when it is built with no crypto provider feature.
#![forbid(unsafe_code)]

use aws_lc_rs::signature::{RSA_PKCS1_2048_8192_SHA256, UnparsedPublicKey};
use rustls_pki_types::{AlgorithmIdentifier, InvalidSignature, SignatureVerificationAlgorithm};

// An `AlgorithmIdentifier` of rustls-pki-types holds the contents of the DER SEQUENCE (the OID
// and the parameters), not the SEQUENCE header: a first attempt with the header was refused by
// webpki as an unsupported algorithm.

/// `rsaEncryption` (1.2.840.113549.1.1.1) with NULL parameters, as it sits in a certificate's
/// `SubjectPublicKeyInfo` for an RSA key.
const RSA_ENCRYPTION: AlgorithmIdentifier = AlgorithmIdentifier::from_slice(&[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x01, 0x05, 0x00,
]);

/// `sha256WithRSAEncryption` (1.2.840.113549.1.1.11) with NULL parameters, as it sits in a
/// certificate's `signatureAlgorithm`.
const SHA256_WITH_RSA_ENCRYPTION: AlgorithmIdentifier = AlgorithmIdentifier::from_slice(&[
    0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00,
]);

/// RSASSA-PKCS1-v1_5 with SHA-256 over a 2048 to 8192 bit key, verified by `aws-lc-rs`.
#[derive(Debug)]
pub struct RsaPkcs1Sha256;

/// The value to pass in the `supported_sig_algs` list of `EndEntityCert::verify_for_usage`.
pub static RSA_PKCS1_SHA256: RsaPkcs1Sha256 = RsaPkcs1Sha256;

impl SignatureVerificationAlgorithm for RsaPkcs1Sha256 {
    fn verify_signature(
        &self,
        public_key: &[u8],
        message: &[u8],
        signature: &[u8],
    ) -> Result<(), InvalidSignature> {
        // `public_key` is the `subjectPublicKey` bits, which for RSA is the RFC 8017
        // RSAPublicKey that aws-lc-rs verifies against.
        UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, public_key)
            .verify(message, signature)
            .map_err(|_| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        RSA_ENCRYPTION
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        SHA256_WITH_RSA_ENCRYPTION
    }
}

/// Wraps an RFC 8017 `RSAPrivateKey` (PKCS #1) in an RFC 5208 `PrivateKeyInfo` (PKCS #8): the
/// first-party wrap that decision E15 weighs against reading a PKCS #1 key through
/// `aws_lc_rs::signature::RsaKeyPair::from_der` and re-exporting it.
pub fn pkcs1_to_pkcs8(pkcs1: &[u8]) -> Vec<u8> {
    // SEQUENCE { INTEGER 0, SEQUENCE { rsaEncryption, NULL }, OCTET STRING { pkcs1 } }
    let mut body = vec![0x02, 0x01, 0x00];
    body.push(0x30);
    body.push(RSA_ENCRYPTION.as_ref().len() as u8);
    body.extend_from_slice(RSA_ENCRYPTION.as_ref());
    body.push(0x04);
    push_der_length(&mut body, pkcs1.len());
    body.extend_from_slice(pkcs1);
    let mut out = vec![0x30];
    push_der_length(&mut out, body.len());
    out.extend_from_slice(&body);
    out
}

fn push_der_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
}
