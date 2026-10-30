//! A small program that links every primitive the probe tests use, so that CI can record the
//! size of a release binary that carries the E15 stack (the footprint job of the S82 workflow).

use std::hint::black_box;

use aws_lc_rs::cipher::{
    AES_128, DecryptingKey, DecryptionContext, EncryptingKey, EncryptionContext, UnboundCipherKey,
};
use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, digest};
use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der};
use aws_lc_rs::hmac::{self, HMAC_SHA256};
use aws_lc_rs::iv::FixedLength;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::{
    KeySize, OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey,
    OaepPublicEncryptingKey, PrivateDecryptingKey, PublicEncryptingKey,
};
use aws_lc_rs::signature::{
    KeyPair, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, RSA_PSS_2048_8192_SHA256,
    RSA_PSS_SHA256, RsaKeyPair, UnparsedPublicKey,
};

fn main() {
    let pair = RsaKeyPair::generate(KeySize::Rsa2048).expect("key");
    let pkcs8: Pkcs8V1Der<'_> = pair.as_der().expect("pkcs8");
    let spki: PublicKeyX509Der<'_> = pair.public_key().as_der().expect("spki");
    let private = OaepPrivateDecryptingKey::new(
        PrivateDecryptingKey::from_pkcs8(pkcs8.as_ref()).expect("key"),
    )
    .expect("oaep");
    let public =
        OaepPublicEncryptingKey::new(PublicEncryptingKey::from_der(spki.as_ref()).expect("key"))
            .expect("oaep");
    for algorithm in [&OAEP_SHA1_MGF1SHA1, &OAEP_SHA256_MGF1SHA256] {
        let mut ciphertext = vec![0u8; 256];
        let ciphertext = public
            .encrypt(algorithm, black_box(b"nonce"), &mut ciphertext, None)
            .expect("encrypt")
            .to_vec();
        let mut out = vec![0u8; 256];
        black_box(
            private
                .decrypt(algorithm, &ciphertext, &mut out, None)
                .expect("decrypt")
                .len(),
        );
    }
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        b"chunk",
        &mut signature,
    )
    .expect("sign");
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, pair.public_key().as_ref())
        .verify(b"chunk", &signature)
        .expect("verify");
    pair.sign(
        &RSA_PSS_SHA256,
        &SystemRandom::new(),
        b"chunk",
        &mut signature,
    )
    .expect("sign");
    UnparsedPublicKey::new(&RSA_PSS_2048_8192_SHA256, pair.public_key().as_ref())
        .verify(b"chunk", &signature)
        .expect("verify");
    let key = [7u8; 16];
    let mut block = [1u8; 32];
    let encrypting =
        EncryptingKey::cbc(UnboundCipherKey::new(&AES_128, &key).expect("key")).expect("cbc");
    encrypting
        .less_safe_encrypt(
            &mut block,
            EncryptionContext::Iv128(FixedLength::from([2u8; 16])),
        )
        .expect("encrypt");
    let decrypting =
        DecryptingKey::cbc(UnboundCipherKey::new(&AES_128, &key).expect("key")).expect("cbc");
    decrypting
        .decrypt(
            &mut block,
            DecryptionContext::Iv128(FixedLength::from([2u8; 16])),
        )
        .expect("decrypt");
    black_box(hmac::sign(&hmac::Key::new(HMAC_SHA256, &key), &block));
    black_box(digest(&SHA1_FOR_LEGACY_USE_ONLY, &block));
}
