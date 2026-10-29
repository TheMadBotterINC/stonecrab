//! Throwaway certificate authorities for the TLS tests, made at run time (slice S71,
//! ADR-0032).
//!
//! Every key is a fresh Ed25519 key from ring's system random generator, so no private key
//! is committed and no two runs share one. The certificates are X.509 version 3, DER written
//! here field by field from RFC 5280 (sections 4.1 and 4.2) and RFC 8410 (the Ed25519
//! algorithm identifier and key), with only what rustls-webpki checks: a certificate
//! authority has a critical basic-constraints extension with `cA` true, an end entity has a
//! subject alternative name (a DNS name or an IPv4 address) and one extended key usage
//! (server or client authentication). Validity is a fixed span of UTC time, current from 2020
//! to 2049 or long past, so nothing depends on reading the clock. Nothing
//! is copied from another implementation, and `rcgen` is not used because it depends on the
//! `time` crate, which `deny.toml` bans.
//!
//! The PEM files go to a [`Scratch`] directory under the system temporary directory, which
//! is removed when it is dropped.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{Ed25519KeyPair, KeyPair};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// The DER of the Ed25519 algorithm identifier, `SEQUENCE { OID 1.3.101.112 }`.
const ED25519: [u8; 7] = [0x30, 0x05, 0x06, 0x03, 0x2B, 0x65, 0x70];

/// A name an end-entity certificate is issued for.
#[derive(Debug, Clone, Copy)]
pub enum SubjectName {
    /// A DNS name, `dNSName` in the subject alternative name.
    Dns(&'static str),
    /// An IPv4 address, `iPAddress` in the subject alternative name.
    Ip([u8; 4]),
}

/// What an end-entity certificate may be used for (its extended key usage).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// TLS server authentication.
    Server,
    /// TLS client authentication.
    Client,
}

/// When a certificate is valid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validity {
    /// From 2020-01-01 to 2049-12-31, the last year `UTCTime` can express.
    Current,
    /// During 2001.
    Expired,
}

/// A self-signed certificate authority with its key.
#[derive(Debug)]
pub struct Authority {
    name: Vec<u8>,
    key: Ed25519KeyPair,
    certificate: Vec<u8>,
}

/// An issued certificate (one certificate, the authority is not included) and its key.
#[derive(Debug)]
pub struct Identity {
    certificate: Vec<u8>,
    pkcs8: Vec<u8>,
}

/// A directory of PEM files, removed when dropped.
#[derive(Debug)]
pub struct Scratch {
    dir: PathBuf,
}

impl Authority {
    /// A new authority named `common_name`, valid from 2020 to 2049.
    pub fn new(common_name: &str) -> Self {
        let (key, _pkcs8) = generate();
        let name = name(common_name);
        let extensions = [extension(
            &[0x55, 0x1D, 0x13],
            true,
            &tlv(0x30, &tlv(0x01, &[0xFF])),
        )];
        let certificate = sign(
            &key,
            &name,
            &name,
            key.public_key().as_ref(),
            Validity::Current,
            &extensions,
        );
        Self {
            name,
            key,
            certificate,
        }
    }

    /// This authority's certificate in PEM, the content of a trust store.
    pub fn certificate_pem(&self) -> String {
        pem("CERTIFICATE", &self.certificate)
    }

    /// This authority's certificate in DER, a trust anchor for a rustls verifier.
    pub fn certificate_der(&self) -> CertificateDer<'static> {
        CertificateDer::from(self.certificate.clone())
    }

    /// Issue an end-entity certificate for `names`, signed by this authority.
    pub fn issue(&self, names: &[SubjectName], purpose: Purpose, validity: Validity) -> Identity {
        let (key, pkcs8) = generate();
        let alternative = names
            .iter()
            .flat_map(|subject| match subject {
                SubjectName::Dns(dns) => tlv(0x82, dns.as_bytes()),
                SubjectName::Ip(ip) => tlv(0x87, ip),
            })
            .collect::<Vec<_>>();
        let usage = match purpose {
            Purpose::Server => [0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x01],
            Purpose::Client => [0x2B, 0x06, 0x01, 0x05, 0x05, 0x07, 0x03, 0x02],
        };
        let extensions = [
            extension(&[0x55, 0x1D, 0x11], false, &tlv(0x30, &alternative)),
            extension(&[0x55, 0x1D, 0x25], false, &tlv(0x30, &tlv(0x06, &usage))),
        ];
        let certificate = sign(
            &self.key,
            &self.name,
            &name("stonecrab test end entity"),
            key.public_key().as_ref(),
            validity,
            &extensions,
        );
        Identity { certificate, pkcs8 }
    }
}

impl Identity {
    /// The certificate followed by the private key, in PEM: the content of a keystore.
    pub fn keystore_pem(&self) -> String {
        let mut text = pem("CERTIFICATE", &self.certificate);
        text.push_str(&pem("PRIVATE KEY", &self.pkcs8));
        text
    }

    /// The certificate in PEM alone.
    pub fn certificate_pem(&self) -> String {
        pem("CERTIFICATE", &self.certificate)
    }

    /// The private key in PEM alone.
    pub fn key_pem(&self) -> String {
        pem("PRIVATE KEY", &self.pkcs8)
    }

    /// The certificate chain (this one certificate) for a rustls configuration.
    pub fn chain(&self) -> Vec<CertificateDer<'static>> {
        vec![CertificateDer::from(self.certificate.clone())]
    }

    /// The private key for a rustls configuration.
    pub fn key(&self) -> PrivateKeyDer<'static> {
        PrivateKeyDer::from(PrivatePkcs8KeyDer::from(self.pkcs8.clone()))
    }
}

impl Scratch {
    /// A new, empty directory under the system temporary directory.
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "stonecrab-tls-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("create the scratch directory");
        Self { dir }
    }

    /// Write `content` to the file `name` in the directory and return its path.
    pub fn write(&self, name: &str, content: &str) -> PathBuf {
        let path = self.dir.join(name);
        fs::write(&path, content).expect("write a scratch file");
        path
    }

    /// The directory.
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        // A directory that cannot be removed is left to the system's temporary cleanup.
        let _removed = fs::remove_dir_all(&self.dir);
    }
}

/// A fresh Ed25519 key and its PKCS #8 document.
fn generate() -> (Ed25519KeyPair, Vec<u8>) {
    let random = SystemRandom::new();
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).expect("generate an Ed25519 key");
    let key = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("read the generated key");
    (key, pkcs8.as_ref().to_vec())
}

/// A signed certificate: `TBSCertificate`, the algorithm and the signature.
fn sign(
    signer: &Ed25519KeyPair,
    issuer: &[u8],
    subject: &[u8],
    public_key: &[u8],
    validity: Validity,
    extensions: &[Vec<u8>],
) -> Vec<u8> {
    let mut serial = [0_u8; 16];
    SystemRandom::new()
        .fill(&mut serial)
        .expect("random serial number");
    // Positive and minimally encoded: the first byte is 0x01 to 0x7F.
    if let Some(first) = serial.first_mut() {
        *first = (*first & 0x7F) | 0x01;
    }
    let (from, until) = match validity {
        Validity::Current => ("200101000000Z", "491231235959Z"),
        Validity::Expired => ("010101000000Z", "011231235959Z"),
    };
    let key_info = [ED25519.to_vec(), bit_string(public_key)].concat();
    let tbs = tlv(
        0x30,
        &[
            tlv(0xA0, &tlv(0x02, &[0x02])),
            tlv(0x02, &serial),
            ED25519.to_vec(),
            issuer.to_vec(),
            tlv(
                0x30,
                &[tlv(0x17, from.as_bytes()), tlv(0x17, until.as_bytes())].concat(),
            ),
            subject.to_vec(),
            tlv(0x30, &key_info),
            tlv(0xA3, &tlv(0x30, &extensions.concat())),
        ]
        .concat(),
    );
    let signature = signer.sign(&tbs);
    tlv(
        0x30,
        &[tbs, ED25519.to_vec(), bit_string(signature.as_ref())].concat(),
    )
}

/// `Name` with one common name: `SEQUENCE { SET { SEQUENCE { OID 2.5.4.3, UTF8String } } }`.
fn name(common_name: &str) -> Vec<u8> {
    let attribute = tlv(
        0x30,
        &[
            tlv(0x06, &[0x55, 0x04, 0x03]),
            tlv(0x0C, common_name.as_bytes()),
        ]
        .concat(),
    );
    tlv(0x30, &tlv(0x31, &attribute))
}

/// `Extension ::= SEQUENCE { extnID, critical BOOLEAN DEFAULT FALSE, extnValue OCTET STRING }`.
fn extension(oid: &[u8], critical: bool, value: &[u8]) -> Vec<u8> {
    let mut content = tlv(0x06, oid);
    if critical {
        content.extend(tlv(0x01, &[0xFF]));
    }
    content.extend(tlv(0x04, value));
    tlv(0x30, &content)
}

/// A BIT STRING of whole bytes.
fn bit_string(bytes: &[u8]) -> Vec<u8> {
    tlv(0x03, &[&[0_u8][..], bytes].concat())
}

/// A DER tag, length and value.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let length = content.len();
    if length < 0x80 {
        out.push(u8::try_from(length).expect("short form"));
    } else {
        let bytes = length.to_be_bytes();
        let significant: Vec<u8> = bytes
            .iter()
            .copied()
            .skip_while(|byte| *byte == 0)
            .collect();
        out.push(0x80 | u8::try_from(significant.len()).expect("a few length bytes"));
        out.extend(significant);
    }
    out.extend_from_slice(content);
    out
}

/// PEM: the label, base64 in lines of 64 characters, the end line.
fn pem(label: &str, der: &[u8]) -> String {
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
