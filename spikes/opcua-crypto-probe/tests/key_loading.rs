//! PKCS #1 versus PKCS #8 private keys. aws-lc-rs 1.19.0 reads an RFC 8017 RSAPrivateKey with
//! `RsaKeyPair::from_der` (signing) but `PrivateDecryptingKey` (OAEP decryption) has
//! `from_pkcs8` only; OPC UA stacks and openssl commonly write the PKCS #1 form
//! ("BEGIN RSA PRIVATE KEY").

mod common;

use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
use aws_lc_rs::rsa::{
    OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey, OaepPublicEncryptingKey,
    PrivateDecryptingKey, PublicEncryptingKey,
};
use aws_lc_rs::signature::{KeyPair, RsaKeyPair};
use opcua_crypto_probe::pkcs1_to_pkcs8;
use rustls_pki_types::PrivateKeyDer;
use rustls_pki_types::pem::PemObject;

/// The RFC 8017 RSAPrivateKey inside the probe key's PKCS #8, by unwrapping the OCTET STRING
/// (the known layout of aws-lc-rs's `Pkcs8V1Der` for RSA: SEQUENCE, INTEGER 0, the 15-byte
/// AlgorithmIdentifier, OCTET STRING).
fn pkcs1_of(pkcs8: &[u8]) -> Vec<u8> {
    fn header(bytes: &[u8]) -> (usize, usize) {
        // (length of tag and length bytes, content length)
        match bytes[1] {
            n if n < 0x80 => (2, n as usize),
            0x81 => (3, bytes[2] as usize),
            0x82 => (4, ((bytes[2] as usize) << 8) | bytes[3] as usize),
            n => panic!("length form {n:#x}"),
        }
    }
    let (outer, _) = header(pkcs8);
    let inner = &pkcs8[outer..];
    assert_eq!(&inner[..3], &[0x02, 0x01, 0x00], "version 0");
    let inner = &inner[3 + 15..];
    assert_eq!(inner[0], 0x04, "OCTET STRING");
    let (skip, len) = header(inner);
    inner[skip..skip + len].to_vec()
}

fn base64(data: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in data.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn pem(label: &str, der: &[u8]) -> String {
    let body = base64(der);
    let mut out = format!("-----BEGIN {label}-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).expect("ascii"));
        out.push('\n');
    }
    out.push_str(&format!("-----END {label}-----\n"));
    out
}

fn decrypts(private: PrivateDecryptingKey) {
    let spki: aws_lc_rs::encoding::PublicKeyX509Der<'_> =
        common::key_pair().public_key().as_der().expect("SPKI");
    let public =
        OaepPublicEncryptingKey::new(PublicEncryptingKey::from_der(spki.as_ref()).expect("public"))
            .expect("oaep");
    let mut ciphertext = vec![0u8; 256];
    let ciphertext = public
        .encrypt(&OAEP_SHA256_MGF1SHA256, b"nonce", &mut ciphertext, None)
        .expect("encrypt")
        .to_vec();
    let private = OaepPrivateDecryptingKey::new(private).expect("oaep");
    let mut out = vec![0u8; 256];
    assert_eq!(
        private
            .decrypt(&OAEP_SHA256_MGF1SHA256, &ciphertext, &mut out, None)
            .expect("decrypt"),
        b"nonce"
    );
}

#[test]
fn the_signing_pair_reads_both_forms() {
    let pkcs8 = common::pkcs8_2048();
    let pkcs1 = pkcs1_of(pkcs8);
    let from_pkcs8 = RsaKeyPair::from_pkcs8(pkcs8).expect("pkcs8");
    let from_pkcs1 = RsaKeyPair::from_der(&pkcs1).expect("pkcs1");
    assert_eq!(
        from_pkcs8.public_key().as_ref(),
        from_pkcs1.public_key().as_ref()
    );
    // And the PKCS #1 reader refuses PKCS #8, the PKCS #8 reader refuses PKCS #1.
    assert!(RsaKeyPair::from_der(pkcs8).is_err());
    assert!(RsaKeyPair::from_pkcs8(&pkcs1).is_err());
}

#[test]
fn the_decrypting_key_reads_pkcs8_only() {
    let pkcs8 = common::pkcs8_2048();
    let pkcs1 = pkcs1_of(pkcs8);
    decrypts(PrivateDecryptingKey::from_pkcs8(pkcs8).expect("pkcs8"));
    // There is no PrivateDecryptingKey::from_der: handing it PKCS #1 as PKCS #8 is refused.
    assert!(PrivateDecryptingKey::from_pkcs8(&pkcs1).is_err());
}

#[test]
fn a_pkcs1_key_reaches_the_decrypting_key_by_either_conversion() {
    let pkcs1 = pkcs1_of(common::pkcs8_2048());
    // (1) first-party wrap of the PKCS #1 bytes into PKCS #8
    let wrapped = pkcs1_to_pkcs8(&pkcs1);
    assert_eq!(wrapped, common::pkcs8_2048());
    decrypts(PrivateDecryptingKey::from_pkcs8(&wrapped).expect("wrapped"));
    // (2) read as a signing pair, re-export as PKCS #8 (no first-party DER)
    let pair = RsaKeyPair::from_der(&pkcs1).expect("pkcs1");
    let exported: Pkcs8V1Der<'_> = pair.as_der().expect("export");
    decrypts(PrivateDecryptingKey::from_pkcs8(exported.as_ref()).expect("exported"));
}

#[test]
fn pem_labels_select_the_der_form() {
    let pkcs8 = common::pkcs8_2048();
    let pkcs1 = pkcs1_of(pkcs8);
    let pem8 = pem("PRIVATE KEY", pkcs8);
    let pem1 = pem("RSA PRIVATE KEY", &pkcs1);
    match PrivateKeyDer::from_pem_slice(pem8.as_bytes()).expect("pem8") {
        PrivateKeyDer::Pkcs8(der) => assert_eq!(der.secret_pkcs8_der(), pkcs8),
        other => panic!("expected PKCS #8, got {other:?}"),
    }
    match PrivateKeyDer::from_pem_slice(pem1.as_bytes()).expect("pem1") {
        PrivateKeyDer::Pkcs1(der) => assert_eq!(der.secret_pkcs1_der(), pkcs1.as_slice()),
        other => panic!("expected PKCS #1, got {other:?}"),
    }
    // An encrypted PEM key is not a key: no label of this parser matches.
    let encrypted = pem("ENCRYPTED PRIVATE KEY", pkcs8);
    assert!(PrivateKeyDer::from_pem_slice(encrypted.as_bytes()).is_err());
}
