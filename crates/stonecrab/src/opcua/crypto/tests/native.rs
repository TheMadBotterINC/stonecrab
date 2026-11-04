//! Adoption checks from the S82 probe; adapters and certificates are test-only.
use super::{hex, unhex};
mod aes_cbc;
mod mac_digest;
use crate::opcua::pki;
mod rsa_oaep;
mod rsa_sign;
mod webpki_anchor;

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
pub(super) struct RsaPkcs1Sha256;

/// The value to pass in the `supported_sig_algs` list of `EndEntityCert::verify_for_usage`.
static SHA256_RSA: RsaPkcs1Sha256 = RsaPkcs1Sha256;

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
            .map_err(|_error| InvalidSignature)
    }

    fn public_key_alg_id(&self) -> AlgorithmIdentifier {
        RSA_ENCRYPTION
    }

    fn signature_alg_id(&self) -> AlgorithmIdentifier {
        SHA256_WITH_RSA_ENCRYPTION
    }
}
