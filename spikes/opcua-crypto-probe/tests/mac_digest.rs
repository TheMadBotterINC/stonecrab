//! HMAC-SHA1 (Basic256 and the legacy channel keys), HMAC-SHA256 (Basic256Sha256 and the
//! Aes policies), and the SHA-1 thumbprint of a certificate (an OPC UA `ThumbprintId`).

mod common;

use aws_lc_rs::digest::{SHA1_FOR_LEGACY_USE_ONLY, SHA256, digest};
use aws_lc_rs::hmac::{self, HMAC_SHA1_FOR_LEGACY_USE_ONLY, HMAC_SHA256};
use aws_lc_rs::signature::KeyPair;

#[test]
fn hmac_sha1_matches_rfc_2202_case_1() {
    let key = hmac::Key::new(HMAC_SHA1_FOR_LEGACY_USE_ONLY, &[0x0b; 20]);
    let tag = hmac::sign(&key, b"Hi There");
    assert_eq!(
        common::hex(tag.as_ref()),
        "b617318655057264e28bc0b6fb378c8ef146be00"
    );
    hmac::verify(&key, b"Hi There", tag.as_ref()).expect("verify");
    assert!(hmac::verify(&key, b"Hi there", tag.as_ref()).is_err());
}

#[test]
fn hmac_sha256_matches_rfc_4231_case_1() {
    let key = hmac::Key::new(HMAC_SHA256, &[0x0b; 20]);
    let tag = hmac::sign(&key, b"Hi There");
    assert_eq!(
        common::hex(tag.as_ref()),
        "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
    );
    hmac::verify(&key, b"Hi There", tag.as_ref()).expect("verify");
    let mut bad = tag.as_ref().to_vec();
    bad[0] ^= 1;
    assert!(hmac::verify(&key, b"Hi There", &bad).is_err());
}

#[test]
fn hmac_streams_in_pieces() {
    // P_SHA key derivation feeds the HMAC piecewise.
    let key = hmac::Key::new(HMAC_SHA256, b"secret");
    let mut context = hmac::Context::with_key(&key);
    context.update(b"seed");
    context.update(b"label");
    assert_eq!(
        context.sign().as_ref(),
        hmac::sign(&key, b"seedlabel").as_ref()
    );
}

#[test]
fn sha1_digest_matches_fips_180_abc() {
    assert_eq!(
        common::hex(digest(&SHA1_FOR_LEGACY_USE_ONLY, b"abc").as_ref()),
        "a9993e364706816aba3e25717850c26c9cd0d89d"
    );
    assert_eq!(
        common::hex(digest(&SHA256, b"abc").as_ref()),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn the_thumbprint_is_the_sha1_of_the_certificate_der() {
    let pair = common::key_pair();
    let spec = common::CertSpec::application("probe", "probe", 1);
    let der = common::certificate(&spec, &pair, &pair);
    let thumbprint = digest(&SHA1_FOR_LEGACY_USE_ONLY, &der);
    assert_eq!(thumbprint.as_ref().len(), 20);
    // Stable for the same bytes, different for any other certificate.
    assert_eq!(
        thumbprint.as_ref(),
        digest(&SHA1_FOR_LEGACY_USE_ONLY, &der).as_ref()
    );
    let other = common::CertSpec::application("probe", "probe", 2);
    let other_der = common::certificate(&other, &pair, &pair);
    assert_ne!(
        thumbprint.as_ref(),
        digest(&SHA1_FOR_LEGACY_USE_ONLY, &other_der).as_ref()
    );
    let _ = pair.public_key();
}
