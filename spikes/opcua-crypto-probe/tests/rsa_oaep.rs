//! RSA-OAEP with SHA-1 (Basic256Sha256 and Aes128_Sha256_RsaOaep) and SHA-256
//! (Aes256_Sha256_RsaPss), encrypted with the public key of a certificate and decrypted with
//! the private key.

mod common;

use aws_lc_rs::encoding::{AsDer, PublicKeyX509Der};
use aws_lc_rs::rsa::{
    OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepAlgorithm, OaepPrivateDecryptingKey,
    OaepPublicEncryptingKey, PrivateDecryptingKey, PublicEncryptingKey,
};
use aws_lc_rs::signature::KeyPair;

fn keys() -> (OaepPublicEncryptingKey, OaepPrivateDecryptingKey) {
    let private = PrivateDecryptingKey::from_pkcs8(common::pkcs8_2048()).expect("PKCS #8");
    // The public key a peer has is the SubjectPublicKeyInfo of its certificate.
    let spki: PublicKeyX509Der<'_> = common::key_pair().public_key().as_der().expect("SPKI");
    let public = PublicEncryptingKey::from_der(spki.as_ref()).expect("public key");
    (
        OaepPublicEncryptingKey::new(public).expect("oaep public"),
        OaepPrivateDecryptingKey::new(private).expect("oaep private"),
    )
}

fn round_trip(algorithm: &'static OaepAlgorithm, max_plaintext: usize) {
    let (public, private) = keys();
    assert_eq!(public.key_size_bytes(), 256);
    // k - 2 hLen - 2: 214 for SHA-1, 190 for SHA-256 at 2048 bits.
    assert_eq!(public.max_plaintext_size(algorithm), max_plaintext);
    let plaintext: Vec<u8> = (0..max_plaintext).map(|i| i as u8).collect();
    let mut ciphertext = vec![0u8; public.ciphertext_size()];
    let ciphertext = public
        .encrypt(algorithm, &plaintext, &mut ciphertext, None)
        .expect("encrypt")
        .to_vec();
    assert_eq!(ciphertext.len(), 256);
    let mut out = vec![0u8; private.min_output_size()];
    let decrypted = private
        .decrypt(algorithm, &ciphertext, &mut out, None)
        .expect("decrypt");
    assert_eq!(decrypted, plaintext.as_slice());

    // One plaintext byte more than the key can carry is refused.
    let mut too_long = vec![0u8; public.ciphertext_size()];
    assert!(
        public
            .encrypt(
                algorithm,
                &vec![0u8; max_plaintext + 1],
                &mut too_long,
                None
            )
            .is_err()
    );

    // A flipped ciphertext bit does not decrypt.
    let mut tampered = ciphertext.clone();
    tampered[100] ^= 0x01;
    let mut out = vec![0u8; private.min_output_size()];
    assert!(
        private
            .decrypt(algorithm, &tampered, &mut out, None)
            .is_err()
    );
}

#[test]
fn oaep_sha1_round_trip() {
    round_trip(&OAEP_SHA1_MGF1SHA1, 214);
}

#[test]
fn oaep_sha256_round_trip() {
    round_trip(&OAEP_SHA256_MGF1SHA256, 190);
}

#[test]
fn oaep_hash_mismatch_is_refused() {
    let (public, private) = keys();
    let mut ciphertext = vec![0u8; public.ciphertext_size()];
    let ciphertext = public
        .encrypt(&OAEP_SHA256_MGF1SHA256, b"secret", &mut ciphertext, None)
        .expect("encrypt")
        .to_vec();
    let mut out = vec![0u8; private.min_output_size()];
    assert!(
        private
            .decrypt(&OAEP_SHA1_MGF1SHA1, &ciphertext, &mut out, None)
            .is_err()
    );
}

#[test]
fn oaep_is_randomised() {
    // OAEP seeds each encryption: two encryptions of one plaintext differ.
    let (public, _) = keys();
    let mut a = vec![0u8; 256];
    let mut b = vec![0u8; 256];
    let a = public
        .encrypt(&OAEP_SHA1_MGF1SHA1, b"same", &mut a, None)
        .expect("encrypt")
        .to_vec();
    let b = public
        .encrypt(&OAEP_SHA1_MGF1SHA1, b"same", &mut b, None)
        .expect("encrypt")
        .to_vec();
    assert_ne!(a, b);
}

#[test]
fn a_wrong_key_does_not_decrypt() {
    let (public, _) = keys();
    let other = common::other_key_pair();
    let other_pkcs8: aws_lc_rs::encoding::Pkcs8V1Der<'_> = other.as_der().expect("PKCS #8");
    let other_private = OaepPrivateDecryptingKey::new(
        PrivateDecryptingKey::from_pkcs8(other_pkcs8.as_ref()).expect("PKCS #8"),
    )
    .expect("oaep private");
    let mut ciphertext = vec![0u8; 256];
    let ciphertext = public
        .encrypt(&OAEP_SHA256_MGF1SHA256, b"secret", &mut ciphertext, None)
        .expect("encrypt")
        .to_vec();
    let mut out = vec![0u8; 256];
    assert!(
        other_private
            .decrypt(&OAEP_SHA256_MGF1SHA256, &ciphertext, &mut out, None)
            .is_err()
    );
}
