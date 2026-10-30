//! RSA signatures: PKCS #1 v1.5 with SHA-256 (Basic256Sha256) and PSS with SHA-256
//! (Aes128_Sha256_RsaOaep is PKCS #1 SHA-256; Aes256_Sha256_RsaPss is PSS), signed with the
//! key pair and verified with the public key bytes of the peer's certificate.

mod common;

use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{
    KeyPair, RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY, RSA_PKCS1_2048_8192_SHA256,
    RSA_PKCS1_SHA256, RSA_PSS_2048_8192_SHA256, RSA_PSS_SHA256, RsaEncoding, RsaPssSaltLength,
    UnparsedPublicKey,
};

const MESSAGE: &[u8] = b"OpenSecureChannelRequest chunk to sign";

fn sign(encoding: &'static dyn RsaEncoding, message: &[u8]) -> Vec<u8> {
    let pair = common::key_pair();
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(encoding, &SystemRandom::new(), message, &mut signature)
        .expect("sign");
    signature
}

#[test]
fn pkcs1_sha256_signs_and_verifies() {
    let pair = common::key_pair();
    let signature = sign(&RSA_PKCS1_SHA256, MESSAGE);
    assert_eq!(signature.len(), 256);
    // PKCS #1 v1.5 is deterministic.
    assert_eq!(signature, sign(&RSA_PKCS1_SHA256, MESSAGE));
    let public = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, pair.public_key().as_ref());
    public.verify(MESSAGE, &signature).expect("verify");
    assert!(public.verify(b"another message", &signature).is_err());
    let mut tampered = signature.clone();
    tampered[10] ^= 1;
    assert!(public.verify(MESSAGE, &tampered).is_err());
}

#[test]
fn pss_sha256_signs_and_verifies() {
    let pair = common::key_pair();
    let signature = sign(&RSA_PSS_SHA256, MESSAGE);
    assert_eq!(signature.len(), 256);
    // PSS is salted: two signatures of one message differ.
    assert_ne!(signature, sign(&RSA_PSS_SHA256, MESSAGE));
    let public = UnparsedPublicKey::new(&RSA_PSS_2048_8192_SHA256, pair.public_key().as_ref());
    public.verify(MESSAGE, &signature).expect("verify");
    assert!(public.verify(b"another message", &signature).is_err());
}

#[test]
fn the_two_paddings_do_not_cross_verify() {
    let pair = common::key_pair();
    let pkcs1 = sign(&RSA_PKCS1_SHA256, MESSAGE);
    let pss = sign(&RSA_PSS_SHA256, MESSAGE);
    let as_pss = UnparsedPublicKey::new(&RSA_PSS_2048_8192_SHA256, pair.public_key().as_ref());
    let as_pkcs1 = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, pair.public_key().as_ref());
    assert!(as_pss.verify(MESSAGE, &pkcs1).is_err());
    assert!(as_pkcs1.verify(MESSAGE, &pss).is_err());
}

#[test]
fn pss_salt_length_is_the_digest_length_by_default() {
    // OPC UA RSA-PSS uses a salt as long as the digest. A signature made with a different salt
    // length is refused by a verifier that expects the digest length (the default).
    let pair = common::key_pair();
    let pss_salt_20 = RSA_PSS_SHA256.with_pss_salt_length(RsaPssSaltLength::Custom(20));
    let pss_salt_20: &'static dyn RsaEncoding = Box::leak(Box::new(pss_salt_20));
    let signature = sign(pss_salt_20, MESSAGE);
    let public = UnparsedPublicKey::new(&RSA_PSS_2048_8192_SHA256, pair.public_key().as_ref());
    assert!(public.verify(MESSAGE, &signature).is_err());
}

#[test]
fn a_foreign_key_does_not_verify() {
    let signature = sign(&RSA_PKCS1_SHA256, MESSAGE);
    let other = common::other_key_pair();
    let public = UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, other.public_key().as_ref());
    assert!(public.verify(MESSAGE, &signature).is_err());
}

#[test]
fn sha1_with_rsa_cannot_be_signed() {
    // The gap behind decision E16: aws-lc-rs 1.19.0 has no SHA-1 signing encoding. Its RSA
    // signing encodings are PKCS #1 SHA-256, SHA-384, SHA-512 and PSS SHA-256, SHA-384,
    // SHA-512 (signature.rs, `RSA_PKCS1_SHA*` and `RSA_PSS_SHA*`), so a SHA-1 digest handed to
    // a signing encoding is refused as a digest mismatch. SHA-1 appears only on the
    // verification side, as `RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY`, which this test
    // names so that a removal would fail the build.
    let pair = common::key_pair();
    let sha1 = digest(&SHA1_FOR_LEGACY_USE_ONLY, MESSAGE);
    let mut signature = vec![0u8; pair.public_modulus_len()];
    assert!(
        pair.sign_digest(&RSA_PKCS1_SHA256, &sha1, &mut signature)
            .is_err()
    );
    let _legacy_verifier = UnparsedPublicKey::new(
        &RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY,
        pair.public_key(),
    );
}
