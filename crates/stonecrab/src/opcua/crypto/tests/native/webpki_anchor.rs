//! A self-signed application certificate as the trust anchor, validated through rustls-webpki
//! built with no crypto provider feature: the signature algorithm is the first-party
//! `RSA_PKCS1_SHA256` of this crate over aws-lc-rs.

use super::pki;

use std::time::Duration;

use crate::opcua::crypto::certificate::PKCS1_SHA256;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::RSA_PKCS1_SHA256;
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use webpki::{EndEntityCert, KeyUsage, anchor_from_trusted_cert};

fn at(seconds: u64) -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(seconds))
}

#[test]
fn a_self_signed_application_certificate_validates_against_itself() {
    let pair = pki::key_pair();
    let spec = pki::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(pki::certificate(&spec, &pair, &pair));
    let anchors = [anchor_from_trusted_cert(&der).expect("anchor")];
    let cert = EndEntityCert::try_from(&der).expect("parse");
    let path = cert
        .verify_for_usage(
            &[&PKCS1_SHA256],
            &anchors,
            &[],
            at(pki::NOW),
            KeyUsage::server_auth(),
            None,
            None,
        )
        .expect("a self-signed certificate that is its own anchor");
    assert_eq!(path.anchor().subject.as_ref(), anchors[0].subject.as_ref());
    cert.verify_is_valid_for_subject_name(&ServerName::try_from("plc.example").expect("name"))
        .expect("dNSName matches");
    assert!(
        cert.verify_is_valid_for_subject_name(
            &ServerName::try_from("other.example").expect("name")
        )
        .is_err()
    );
}

#[test]
fn a_different_anchor_is_unknown_issuer() {
    let pair = pki::key_pair();
    let other = pki::other_key_pair();
    let ee_spec = pki::CertSpec::application("probe application", "probe application", 1);
    let other_spec = pki::CertSpec::application("someone else", "someone else", 2);
    let ee = CertificateDer::from(pki::certificate(&ee_spec, &pair, &pair));
    let stranger = CertificateDer::from(pki::certificate(&other_spec, &other, &other));
    let anchor = anchor_from_trusted_cert(&stranger).expect("anchor");
    let cert = EndEntityCert::try_from(&ee).expect("parse");
    let error = cert
        .verify_for_usage(
            &[&PKCS1_SHA256],
            &[anchor],
            &[],
            at(pki::NOW),
            KeyUsage::server_auth(),
            None,
            None,
        )
        .err()
        .expect("not trusted");
    assert!(matches!(error, webpki::Error::UnknownIssuer), "{error:?}");
}

#[test]
fn an_expired_certificate_is_refused() {
    let pair = pki::key_pair();
    let mut spec = pki::CertSpec::application("probe application", "probe application", 1);
    spec.not_before = "010101000000Z";
    spec.not_after = "020101000000Z";
    let der = CertificateDer::from(pki::certificate(&spec, &pair, &pair));
    let anchor = anchor_from_trusted_cert(&der).expect("anchor");
    let cert = EndEntityCert::try_from(&der).expect("parse");
    let error = cert
        .verify_for_usage(
            &[&PKCS1_SHA256],
            &[anchor],
            &[],
            at(pki::NOW),
            KeyUsage::server_auth(),
            None,
            None,
        )
        .err()
        .expect("expired");
    assert!(
        matches!(error, webpki::Error::CertExpired { .. }),
        "{error:?}"
    );
}

#[test]
fn a_ca_issued_application_certificate_validates_through_the_ca_anchor() {
    let ca_key = pki::other_key_pair();
    let app_key = pki::key_pair();
    let mut ca_spec = pki::CertSpec::application("probe ca", "probe ca", 1);
    ca_spec.is_ca = true;
    let app_spec = pki::CertSpec::application("probe application", "probe ca", 2);
    let ca = CertificateDer::from(pki::certificate(&ca_spec, &ca_key, &ca_key));
    let app = CertificateDer::from(pki::certificate(&app_spec, &app_key, &ca_key));
    let anchor = anchor_from_trusted_cert(&ca).expect("anchor");
    let cert = EndEntityCert::try_from(&app).expect("parse");
    cert.verify_for_usage(
        &[&PKCS1_SHA256],
        &[anchor],
        &[],
        at(pki::NOW),
        KeyUsage::client_auth(),
        None,
        None,
    )
    .expect("chain to the CA");
}

#[test]
fn without_the_algorithm_nothing_validates() {
    // webpki built with no provider feature trusts no signature algorithm of its own.
    let pair = pki::key_pair();
    let spec = pki::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(pki::certificate(&spec, &pair, &pair));
    let anchor = anchor_from_trusted_cert(&der).expect("anchor");
    let cert = EndEntityCert::try_from(&der).expect("parse");
    assert!(
        cert.verify_for_usage(
            &[],
            &[anchor],
            &[],
            at(pki::NOW),
            KeyUsage::server_auth(),
            None,
            None,
        )
        .is_err()
    );
}

#[test]
fn a_signature_by_the_certificates_key_verifies_through_the_certificate() {
    // The OPC UA use: the peer signed a chunk, the verifier holds only the certificate.
    let pair = pki::key_pair();
    let spec = pki::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(pki::certificate(&spec, &pair, &pair));
    let cert = EndEntityCert::try_from(&der).expect("parse");
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        b"chunk",
        &mut signature,
    )
    .expect("sign");
    cert.verify_signature(&PKCS1_SHA256, b"chunk", &signature)
        .expect("verify");
    assert!(
        cert.verify_signature(&PKCS1_SHA256, b"chunks", &signature)
            .is_err()
    );
}
