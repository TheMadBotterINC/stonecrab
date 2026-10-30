//! Test support: a 2048-bit RSA key made once per test binary, and a minimal X.509 writer.
//! `rcgen` would do this but depends on the `time` crate that the root `deny.toml` bans, so the
//! few DER structures webpki reads are written by hand, as ADR-0032 decision 11 does.
#![allow(dead_code)]

use std::sync::OnceLock;

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair, RSA_PKCS1_SHA256, RsaKeyPair};

/// 2026-10-30T00:00:00Z, the date of the slice, as seconds since the epoch.
pub const NOW: u64 = 1_793_318_400;

pub fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(len as u8);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x81, len as u8]);
    } else {
        out.extend_from_slice(&[0x82, (len >> 8) as u8, len as u8]);
    }
    out.extend_from_slice(content);
    out
}

pub fn seq(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x30, &parts.concat())
}

fn set(parts: &[&[u8]]) -> Vec<u8> {
    tlv(0x31, &parts.concat())
}

fn oid(bytes: &[u8]) -> Vec<u8> {
    tlv(0x06, bytes)
}

fn utf8(text: &str) -> Vec<u8> {
    tlv(0x0c, text.as_bytes())
}

fn name(common_name: &str) -> Vec<u8> {
    // RDNSequence { SET { SEQUENCE { id-at-commonName, UTF8String } } }
    seq(&[&set(&[&seq(&[
        &oid(&[0x55, 0x04, 0x03]),
        &utf8(common_name),
    ])])])
}

const SHA256_WITH_RSA: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00,
];

/// What a certificate is for.
pub struct CertSpec<'a> {
    pub subject: &'a str,
    pub issuer: &'a str,
    pub serial: u8,
    pub is_ca: bool,
    pub dns_name: &'a str,
    pub application_uri: &'a str,
    /// `YYMMDDHHMMSSZ` UTCTime strings.
    pub not_before: &'a str,
    pub not_after: &'a str,
}

impl<'a> CertSpec<'a> {
    pub fn application(subject: &'a str, issuer: &'a str, serial: u8) -> Self {
        CertSpec {
            subject,
            issuer,
            serial,
            is_ca: false,
            dns_name: "plc.example",
            application_uri: "urn:stonecrab:probe",
            not_before: "260101000000Z",
            not_after: "360101000000Z",
        }
    }
}

/// Writes an X.509 v3 certificate for `subject_key`, signed by `signer` with PKCS #1 SHA-256.
pub fn certificate(spec: &CertSpec<'_>, subject_key: &RsaKeyPair, signer: &RsaKeyPair) -> Vec<u8> {
    let spki = AsDer::<aws_lc_rs::encoding::PublicKeyX509Der<'_>>::as_der(subject_key.public_key())
        .expect("SubjectPublicKeyInfo");
    let version = tlv(0xa0, &tlv(0x02, &[2]));
    let serial = tlv(0x02, &[spec.serial]);
    let validity = seq(&[
        &tlv(0x17, spec.not_before.as_bytes()),
        &tlv(0x17, spec.not_after.as_bytes()),
    ]);
    // basicConstraints (2.5.29.19), critical
    let basic = seq(&[
        &oid(&[0x55, 0x1d, 0x13]),
        &tlv(0x01, &[0xff]),
        &tlv(
            0x04,
            &seq(&[&if spec.is_ca {
                tlv(0x01, &[0xff])
            } else {
                Vec::new()
            }]),
        ),
    ]);
    // keyUsage (2.5.29.15), critical: digitalSignature, nonRepudiation, keyEncipherment,
    // dataEncipherment, keyCertSign (an application certificate's set, plus certSign for a CA)
    let (usage_byte, unused_bits) = if spec.is_ca { (0xf4, 2) } else { (0xf0, 4) };
    let usage = seq(&[
        &oid(&[0x55, 0x1d, 0x0f]),
        &tlv(0x01, &[0xff]),
        &tlv(0x04, &tlv(0x03, &[unused_bits, usage_byte])),
    ]);
    // extendedKeyUsage (2.5.29.37): serverAuth, clientAuth
    let eku = seq(&[
        &oid(&[0x55, 0x1d, 0x25]),
        &tlv(
            0x04,
            &seq(&[
                &oid(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01]),
                &oid(&[0x2b, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02]),
            ]),
        ),
    ]);
    // subjectAltName (2.5.29.17): uniformResourceIdentifier [6] and dNSName [2], as OPC UA
    // application instance certificates carry
    let san = seq(&[
        &oid(&[0x55, 0x1d, 0x11]),
        &tlv(
            0x04,
            &seq(&[
                &tlv(0x86, spec.application_uri.as_bytes()),
                &tlv(0x82, spec.dns_name.as_bytes()),
            ]),
        ),
    ]);
    let extensions = tlv(0xa3, &seq(&[&basic, &usage, &eku, &san]));
    let tbs = seq(&[
        &version,
        &serial,
        &SHA256_WITH_RSA,
        &name(spec.issuer),
        &validity,
        &name(spec.subject),
        spki.as_ref(),
        &extensions,
    ]);
    let mut signature = vec![0u8; signer.public_modulus_len()];
    signer
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            &tbs,
            &mut signature,
        )
        .expect("sign tbsCertificate");
    let mut bits = vec![0u8];
    bits.extend_from_slice(&signature);
    seq(&[&tbs, &SHA256_WITH_RSA, &tlv(0x03, &bits)])
}

/// The PKCS #8 encoding of a 2048-bit key generated once per test binary.
pub fn pkcs8_2048() -> &'static [u8] {
    static KEY: OnceLock<Vec<u8>> = OnceLock::new();
    KEY.get_or_init(|| {
        let pair = RsaKeyPair::generate(KeySize::Rsa2048).expect("generate RSA-2048");
        AsDer::<aws_lc_rs::encoding::Pkcs8V1Der<'_>>::as_der(&pair)
            .expect("PKCS #8")
            .as_ref()
            .to_vec()
    })
}

pub fn key_pair() -> RsaKeyPair {
    RsaKeyPair::from_pkcs8(pkcs8_2048()).expect("PKCS #8 key")
}

/// A second, unrelated key.
pub fn other_key_pair() -> RsaKeyPair {
    RsaKeyPair::generate(KeySize::Rsa2048).expect("generate RSA-2048")
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hex"))
        .collect()
}
