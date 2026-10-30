//! A self-signed application certificate as the trust anchor, validated through rustls-webpki
//! built with no crypto provider feature: the signature algorithm is the first-party
//! `RSA_PKCS1_SHA256` of this crate over aws-lc-rs.

mod common;

use std::time::Duration;

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::RSA_PKCS1_SHA256;
use opcua_crypto_probe::RSA_PKCS1_SHA256 as SHA256_RSA;
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};
use webpki::{EndEntityCert, KeyUsage, anchor_from_trusted_cert};

fn at(seconds: u64) -> UnixTime {
    UnixTime::since_unix_epoch(Duration::from_secs(seconds))
}

#[test]
fn a_self_signed_application_certificate_validates_against_itself() {
    let pair = common::key_pair();
    let spec = common::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(common::certificate(&spec, &pair, &pair));
    let anchors = [anchor_from_trusted_cert(&der).expect("anchor")];
    let cert = EndEntityCert::try_from(&der).expect("parse");
    let path = cert
        .verify_for_usage(
            &[&SHA256_RSA],
            &anchors,
            &[],
            at(common::NOW),
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
    let pair = common::key_pair();
    let other = common::other_key_pair();
    let ee_spec = common::CertSpec::application("probe application", "probe application", 1);
    let other_spec = common::CertSpec::application("someone else", "someone else", 2);
    let ee = CertificateDer::from(common::certificate(&ee_spec, &pair, &pair));
    let stranger = CertificateDer::from(common::certificate(&other_spec, &other, &other));
    let anchor = anchor_from_trusted_cert(&stranger).expect("anchor");
    let cert = EndEntityCert::try_from(&ee).expect("parse");
    let error = cert
        .verify_for_usage(
            &[&SHA256_RSA],
            &[anchor],
            &[],
            at(common::NOW),
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
    let pair = common::key_pair();
    let spec = common::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(common::certificate(&spec, &pair, &pair));
    let anchor = anchor_from_trusted_cert(&der).expect("anchor");
    let cert = EndEntityCert::try_from(&der).expect("parse");
    // 2037-01-01, a year after notAfter.
    let error = cert
        .verify_for_usage(
            &[&SHA256_RSA],
            &[anchor],
            &[],
            at(2_114_380_800),
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
    let ca_key = common::other_key_pair();
    let app_key = common::key_pair();
    let mut ca_spec = common::CertSpec::application("probe ca", "probe ca", 1);
    ca_spec.is_ca = true;
    let app_spec = common::CertSpec::application("probe application", "probe ca", 2);
    let ca = CertificateDer::from(common::certificate(&ca_spec, &ca_key, &ca_key));
    let app = CertificateDer::from(common::certificate(&app_spec, &app_key, &ca_key));
    let anchor = anchor_from_trusted_cert(&ca).expect("anchor");
    let cert = EndEntityCert::try_from(&app).expect("parse");
    cert.verify_for_usage(
        &[&SHA256_RSA],
        &[anchor],
        &[],
        at(common::NOW),
        KeyUsage::client_auth(),
        None,
        None,
    )
    .expect("chain to the CA");
}

#[test]
fn without_the_algorithm_nothing_validates() {
    // webpki built with no provider feature trusts no signature algorithm of its own.
    let pair = common::key_pair();
    let spec = common::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(common::certificate(&spec, &pair, &pair));
    let anchor = anchor_from_trusted_cert(&der).expect("anchor");
    let cert = EndEntityCert::try_from(&der).expect("parse");
    assert!(
        cert.verify_for_usage(
            &[],
            &[anchor],
            &[],
            at(common::NOW),
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
    let pair = common::key_pair();
    let spec = common::CertSpec::application("probe application", "probe application", 1);
    let der = CertificateDer::from(common::certificate(&spec, &pair, &pair));
    let cert = EndEntityCert::try_from(&der).expect("parse");
    let mut signature = vec![0u8; pair.public_modulus_len()];
    pair.sign(
        &RSA_PKCS1_SHA256,
        &SystemRandom::new(),
        b"chunk",
        &mut signature,
    )
    .expect("sign");
    cert.verify_signature(&SHA256_RSA, b"chunk", &signature)
        .expect("verify");
    assert!(
        cert.verify_signature(&SHA256_RSA, b"chunks", &signature)
            .is_err()
    );
}
