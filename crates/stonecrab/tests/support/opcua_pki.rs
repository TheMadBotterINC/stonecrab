//! Test support: a 2048-bit RSA key made once per test binary, and a minimal X.509 writer.
//! `rcgen` would do this but depends on the `time` crate that the root `deny.toml` bans, so the
//! few DER structures webpki reads are written by hand, as ADR-0032 decision 11 does.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};

use aws_lc_rs::encoding::AsDer;
use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::KeySize;
use aws_lc_rs::signature::{KeyPair, RSA_PKCS1_SHA256, RsaKeyPair};

/// 2026-10-30T00:00:00Z, the date of the slice, as seconds since the epoch.
pub(super) const NOW: u64 = 1_793_318_400;

pub(super) fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let len = content.len();
    if len < 0x80 {
        out.push(u8::try_from(len).unwrap());
    } else if len <= 0xff {
        out.extend_from_slice(&[0x81, u8::try_from(len).unwrap()]);
    } else {
        out.extend_from_slice(&[
            0x82,
            u8::try_from(len >> 8).unwrap(),
            u8::try_from(len & 0xff).unwrap(),
        ]);
    }
    out.extend_from_slice(content);
    out
}

pub(super) fn seq(parts: &[&[u8]]) -> Vec<u8> {
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
    seq(&[
        &set(&[&seq(&[&oid(&[0x55, 0x04, 0x03]), &utf8(common_name)])]),
        &set(&[&seq(&[
            &oid(&[0x55, 0x04, 0x0a]),
            &utf8("Stonecrab test PKI"),
        ])]),
    ])
}

const SHA256_WITH_RSA: [u8; 15] = [
    0x30, 0x0d, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x01, 0x0b, 0x05, 0x00,
];

/// What a certificate is for.
pub(super) struct CertSpec<'a> {
    pub(super) subject: &'a str,
    pub(super) issuer: &'a str,
    pub(super) serial: u8,
    pub(super) is_ca: bool,
    pub(super) dns_name: &'a str,
    pub(super) application_uris: &'a [&'a str],
    /// `YYMMDDHHMMSSZ` `UTCTime` strings.
    pub(super) not_before: &'a str,
    pub(super) not_after: &'a str,
}

impl<'a> CertSpec<'a> {
    pub(super) fn application(subject: &'a str, issuer: &'a str, serial: u8) -> Self {
        CertSpec {
            subject,
            issuer,
            serial,
            is_ca: false,
            dns_name: "plc.example",
            application_uris: &["urn:stonecrab:probe"],
            not_before: "260101000000Z",
            not_after: "360101000000Z",
        }
    }
}

/// Writes an X.509 v3 certificate for `subject_key`, signed by `signer` with PKCS #1 SHA-256.
pub(super) fn certificate(
    spec: &CertSpec<'_>,
    subject_key: &RsaKeyPair,
    signer: &RsaKeyPair,
) -> Vec<u8> {
    let spki = AsDer::<aws_lc_rs::encoding::PublicKeyX509Der<'_>>::as_der(subject_key.public_key())
        .expect("SubjectPublicKeyInfo");
    certificate_with_spki(spec, spki.as_ref(), signer)
}

/// Also allows non-RSA subject keys for negative tests.
pub(super) fn certificate_with_spki(
    spec: &CertSpec<'_>,
    spki: &[u8],
    signer: &RsaKeyPair,
) -> Vec<u8> {
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
    let (usage_byte, unused_bits) = if spec.is_ca || spec.subject == spec.issuer {
        (0xf4, 2)
    } else {
        (0xf0, 4)
    };
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
                &spec
                    .application_uris
                    .iter()
                    .flat_map(|uri| tlv(0x86, uri.as_bytes()))
                    .collect::<Vec<_>>(),
                &tlv(0x82, spec.dns_name.as_bytes()),
            ]),
        ),
    ]);
    // Stable key identifiers bind the CA-issued test chain (RFC 5280 4.2.1.1/2).
    let key_id = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY, spki);
    let issuer_spki: aws_lc_rs::encoding::PublicKeyX509Der<'_> =
        signer.public_key().as_der().unwrap();
    let issuer_id = aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY,
        issuer_spki.as_ref(),
    );
    let subject_id = seq(&[
        &oid(&[0x55, 0x1d, 0x0e]),
        &tlv(0x04, &tlv(0x04, key_id.as_ref())),
    ]);
    let authority_id = seq(&[
        &oid(&[0x55, 0x1d, 0x23]),
        &tlv(0x04, &seq(&[&tlv(0x80, issuer_id.as_ref())])),
    ]);
    let extensions = tlv(
        0xa3,
        &seq(&[&basic, &usage, &eku, &san, &subject_id, &authority_id]),
    );
    let tbs = seq(&[
        &version,
        &serial,
        &SHA256_WITH_RSA,
        &name(spec.issuer),
        &validity,
        &name(spec.subject),
        spki,
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
pub(super) fn pkcs8_2048() -> &'static [u8] {
    static KEY: OnceLock<Vec<u8>> = OnceLock::new();
    KEY.get_or_init(|| {
        let pair = RsaKeyPair::generate(KeySize::Rsa2048).expect("generate RSA-2048");
        AsDer::<aws_lc_rs::encoding::Pkcs8V1Der<'_>>::as_der(&pair)
            .expect("PKCS #8")
            .as_ref()
            .to_vec()
    })
}

pub(super) fn key_pair() -> RsaKeyPair {
    RsaKeyPair::from_pkcs8(pkcs8_2048()).expect("PKCS #8 key")
}

/// A second, unrelated key.
pub(super) fn other_key_pair() -> RsaKeyPair {
    RsaKeyPair::generate(KeySize::Rsa2048).expect("generate RSA-2048")
}

/// Temporary PEM files, removed when dropped.
#[derive(Debug)]
pub(super) struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    /// A new, empty directory under the system temporary directory.
    pub(super) fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "stonecrab-opcua-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create the scratch directory");
        Self { dir }
    }

    /// Write `content` to the file `name` in the directory and return its path.
    pub(super) fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, content).expect("write a scratch file");
        path
    }

    /// The directory.
    pub(super) fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A directory that cannot be removed is left to the system's temporary cleanup.
        let _removed = fs::remove_dir_all(&self.dir);
    }
}

pub(super) fn pem(label: &str, der: &[u8]) -> String {
    let encoded = base64(der);
    let mut text = format!("-----BEGIN {label}-----\n");
    for line in encoded.as_bytes().chunks(64) {
        text.push_str(std::str::from_utf8(line).expect("base64 is ASCII"));
        text.push('\n');
    }
    let _infallible = writeln!(text, "-----END {label}-----");
    text
}

/// Standard base64 with padding (RFC 4648, section 4).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing,
    reason = "bit arithmetic on a 24-bit group and table lookups by a six-bit index"
)]
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let mut group = [0_u8; 3];
        for (slot, byte) in group.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        let value = u32::from_be_bytes([0, group[0], group[1], group[2]]);
        for index in 0..4 {
            if index <= chunk.len() {
                let sextet = (value >> (18 - 6 * index)) & 0x3F;
                text.push(char::from(
                    ALPHABET[usize::try_from(sextet).expect("six bits")],
                ));
            } else {
                text.push('=');
            }
        }
    }
    text
}
