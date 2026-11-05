use std::error::Error as _;
use std::io::{self, Cursor};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::rsa::{OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey, OaepPublicEncryptingKey};
use aws_lc_rs::signature::{
    Ed25519KeyPair, RSA_PKCS1_2048_8192_SHA256, RSA_PKCS1_SHA256, UnparsedPublicKey,
};

use super::*;
use crate::opcua::pki::{self, CertSpec, Scratch, pem};

fn leaf() -> Vec<u8> {
    pki::certificate(
        &CertSpec::application("client", "client", 1),
        &pki::key_pair(),
        &pki::key_pair(),
    )
}

fn store(certificates: &[Vec<u8>], key: &[u8]) -> String {
    let mut text = certificates
        .iter()
        .map(|cert| pem("CERTIFICATE", cert))
        .collect::<String>();
    text.push_str(&pem("PRIVATE KEY", key));
    text
}

fn refused(bytes: &[u8]) -> PlcError {
    let error = KeyMaterial::from_pem(bytes).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(error.source().is_none());
    error
}

// covers: OPCUA-CERT-KEYSTORE
#[test]
fn opcua_keystore_pem_loads_the_chain_and_the_key() {
    let certificate = leaf();
    let material = KeyMaterial::from_pem(
        store(std::slice::from_ref(&certificate), pki::pkcs8_2048()).as_bytes(),
    )
    .unwrap();
    assert_eq!(material.leaf.as_ref(), certificate);
    assert_eq!(material.sender_certificate, certificate);
    assert_eq!(material.application_uri, "urn:stonecrab:probe");
    let mut signature = vec![0; material.signing.expose().public_modulus_len()];
    material
        .signing
        .expose()
        .sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            b"key possession",
            &mut signature,
        )
        .unwrap();
    UnparsedPublicKey::new(
        &RSA_PKCS1_2048_8192_SHA256,
        pki::key_pair().public_key().as_ref(),
    )
    .verify(b"key possession", &signature)
    .unwrap();
    let public = OaepPublicEncryptingKey::new(material.decrypting.expose().public_key()).unwrap();
    let private = OaepPrivateDecryptingKey::new(material.decrypting.expose().clone()).unwrap();
    let mut ciphertext = vec![0; public.ciphertext_size()];
    let encrypted = public
        .encrypt(
            &OAEP_SHA256_MGF1SHA256,
            b"loaded key",
            &mut ciphertext,
            None,
        )
        .unwrap();
    let mut plaintext = vec![0; private.min_output_size()];
    assert_eq!(
        private
            .decrypt(&OAEP_SHA256_MGF1SHA256, encrypted, &mut plaintext, None)
            .unwrap(),
        b"loaded key"
    );
}

#[test]
fn opcua_keystore_without_a_private_key_is_rejected() {
    refused(pem("CERTIFICATE", &leaf()).as_bytes());
    refused(b"");
    refused(pem("PRIVATE KEY", pki::pkcs8_2048()).as_bytes());
}

#[test]
fn opcua_keystore_accepts_key_first_and_platform_line_endings() {
    let certificate = leaf();
    let text = format!(
        "{}{}",
        pem("PRIVATE KEY", pki::pkcs8_2048()),
        pem("CERTIFICATE", &certificate)
    );
    for newline in ["\n", "\r\n", "\r"] {
        let text = text.replace('\n', newline);
        let material = KeyMaterial::from_pem(text.as_bytes()).unwrap();
        assert_eq!(material.leaf.as_ref(), certificate);
        let extra = format!(
            "{text}{}",
            pem("ENCRYPTED PRIVATE KEY", b"not accepted").replace('\n', newline)
        );
        refused(extra.as_bytes());
    }
}

#[test]
fn opcua_keystore_with_a_non_rsa_key_is_rejected() {
    let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let pair = Ed25519KeyPair::from_pkcs8(key.as_ref()).unwrap();
    let spki = pki::seq(&[
        &[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70],
        &pki::tlv(0x03, &[&[0], pair.public_key().as_ref()].concat()),
    ]);
    let cert = pki::certificate_with_spki(
        &CertSpec::application("ed client", "CA", 2),
        &spki,
        &pki::key_pair(),
    );
    refused(store(&[cert], key.as_ref()).as_bytes());
    refused(store(&[leaf()], b"invalid PKCS8 bytes").as_bytes());
}

// covers: OPCUA-CERT-CHAIN
#[test]
fn opcua_keystore_keeps_the_certificate_chain() {
    let ca = pki::other_key_pair();
    let mut ca_spec = CertSpec::application("CA", "CA", 1);
    ca_spec.is_ca = true;
    let root = pki::certificate(&ca_spec, &ca, &ca);
    let client = pki::certificate(
        &CertSpec::application("client", "CA", 2),
        &pki::key_pair(),
        &ca,
    );
    let material =
        KeyMaterial::from_pem(store(&[client.clone(), root.clone()], pki::pkcs8_2048()).as_bytes())
            .unwrap();
    assert_eq!(material.leaf.as_ref(), client);
    assert_eq!(
        material.sender_certificate,
        [client.clone(), root.clone()].concat()
    );
    assert_eq!(material.thumbprint.len(), 20);
    assert_eq!(
        material.thumbprint,
        digest(&SHA1_FOR_LEGACY_USE_ONLY, &client).as_ref()
    );
    assert_ne!(
        material.thumbprint,
        digest(&SHA1_FOR_LEGACY_USE_ONLY, &root).as_ref()
    );
    refused(store(&[root, client], pki::pkcs8_2048()).as_bytes());
}

#[test]
fn opcua_keystore_rejects_a_mismatched_leaf_and_malformed_chain() {
    let wrong = pki::certificate(
        &CertSpec::application("wrong", "wrong", 3),
        &pki::other_key_pair(),
        &pki::key_pair(),
    );
    assert!(
        refused(store(&[wrong], pki::pkcs8_2048()).as_bytes())
            .to_string()
            .contains("matching RSA")
    );
    refused(store(&[vec![0x30, 0x00]], pki::pkcs8_2048()).as_bytes());
    refused(store(&[leaf(), vec![0x30, 0]], pki::pkcs8_2048()).as_bytes());
}

#[test]
fn opcua_keystore_rejects_extra_and_unsupported_key_sections() {
    let good = store(&[leaf()], pki::pkcs8_2048());
    for label in [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "RSA PRIVATE KEY",
        "EC PRIVATE KEY",
        "PUBLIC KEY",
        "UNKNOWN",
    ] {
        refused(format!("{good}{}", pem(label, pki::pkcs8_2048())).as_bytes());
        if label != "PRIVATE KEY" {
            refused(
                format!(
                    "{}{}",
                    pem("CERTIFICATE", &leaf()),
                    pem(label, pki::pkcs8_2048())
                )
                .as_bytes(),
            );
        }
    }
    for bytes in [
        b"-----BEGIN PRIVATE KEY-----\n!\n-----END PRIVATE KEY-----".as_slice(),
        b"-----BEGIN PRIVATE KEY-----\nAAA",
        b"-----BEGIN PRIVATE KEY----\nAAA",
        b"secret file data",
        &[0xff, 0xfe],
    ] {
        refused(bytes);
    }
}

// covers: OPCUA-CERT-APPLICATION-URI
#[test]
fn opcua_application_uri_comes_from_the_certificate_san() {
    for uri in [
        "urn:stonecrab:probe",
        "https://plc.example/client",
        "opc.tcp://plc.example:4840",
        "a+B.c-d:value",
    ] {
        let mut spec = CertSpec::application("client", "client", 4);
        let uris = [uri];
        spec.application_uris = &uris;
        let cert = pki::certificate(&spec, &pki::key_pair(), &pki::key_pair());
        let material = KeyMaterial::from_pem(store(&[cert], pki::pkcs8_2048()).as_bytes()).unwrap();
        assert_eq!(material.application_uri, uri);
    }
}

#[test]
fn opcua_secure_application_uri_does_not_fall_back_when_missing_or_invalid() {
    for uris in [
        vec![],
        vec![""],
        vec!["relative"],
        vec!["1bad:rest"],
        vec![":rest"],
        vec!["a_b:rest"],
        vec!["urn:"],
        vec!["urn:has space"],
        vec!["urn:has\0control"],
        vec!["urn:nonascii:é"],
        vec!["urn:first", "urn:second"],
    ] {
        let mut spec = CertSpec::application("urn:apache:plc4x:client", "client", 5);
        spec.application_uris = &uris;
        let cert = pki::certificate(&spec, &pki::key_pair(), &pki::key_pair());
        let error = refused(store(&[cert], pki::pkcs8_2048()).as_bytes());
        assert!(error.to_string().contains("application URI"));
        assert!(!error.to_string().contains("urn:apache"));
    }
}

#[test]
fn opcua_keystore_errors_and_debug_never_expose_contents_or_paths() {
    let scratch = Scratch::new();
    let path = scratch.write("sensitive-file.pem", "sensitive-file-contents");
    for path in [path, scratch.path().join("sensitive-missing-file.pem")] {
        let error = KeyMaterial::load(path.to_str()).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.source().is_none());
        for shown in [format!("{error}"), format!("{error:?}")] {
            assert!(!shown.contains("sensitive"));
            assert!(!shown.contains(scratch.path().to_str().unwrap()));
        }
    }
    let material = KeyMaterial::from_pem(store(&[leaf()], pki::pkcs8_2048()).as_bytes()).unwrap();
    assert_eq!(format!("{material:?}"), "KeyMaterial([REDACTED])");
    assert_eq!(format!("{material:#?}"), "KeyMaterial([REDACTED])");
    assert_eq!(format!("{:?}", material.signing), "[REDACTED]");
    assert_eq!(format!("{:?}", material.decrypting), "[REDACTED]");
}

// covers: OPCUA-CFG-KEYSTORE
#[test]
fn opcua_keystore_file_is_loaded_once_and_owned_after_removal() {
    let scratch = Scratch::new();
    let path = scratch.write("client.pem", &store(&[leaf()], pki::pkcs8_2048()));
    let material = KeyMaterial::load(path.to_str()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(material.application_uri, "urn:stonecrab:probe");
    assert_eq!(material.signing.expose().public_modulus_len(), 256);
}

#[test]
fn opcua_keystore_read_bounds_and_io_errors_are_enforced() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("sensitive source path or contents"))
        }
    }
    let limit = usize::try_from(MAX_PEM_BYTES).unwrap();
    let exact = vec![b' '; limit];
    assert_eq!(read_pem(exact.as_slice()).unwrap().expose().len(), limit);
    let mut overflow = Cursor::new(vec![b' '; limit.checked_add(100).unwrap()]);
    let error = read_pem(&mut overflow).unwrap_err();
    assert!(error.to_string().contains("1 MiB"));
    assert_eq!(overflow.position(), MAX_PEM_BYTES.checked_add(1).unwrap());
    let error = read_pem(Broken).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(error.source().is_none());
    assert!(!error.to_string().contains("sensitive"));
    let scratch = Scratch::new();
    let path = scratch.write("oversized.pem", &" ".repeat(limit.checked_add(1).unwrap()));
    assert!(
        KeyMaterial::load(path.to_str())
            .unwrap_err()
            .to_string()
            .contains("1 MiB")
    );
}

// covers: OPCUA-AUTH-X509
#[test]
fn opcua_user_key_accepts_der_or_pem_leaf_without_application_uri_and_signs_each_policy() {
    use crate::opcua::options::SecurityPolicy;
    use aws_lc_rs::signature::{RSA_PSS_2048_8192_SHA256, RsaPssSaltLength};
    let scratch = Scratch::new();
    let mut spec = CertSpec::application("user", "user", 19);
    spec.application_uris = &[];
    let leaf = pki::certificate(&spec, &pki::key_pair(), &pki::key_pair());
    let private = scratch.write("user-key.pem", &pem("PRIVATE KEY", pki::pkcs8_2048()));
    let certificate = scratch.path().join("user.der");
    for bytes in [
        leaf.clone(),
        pem("CERTIFICATE", &leaf).into_bytes(),
        format!("{}{}", pem("CERTIFICATE", &leaf), pem("CERTIFICATE", &leaf)).into_bytes(),
    ] {
        std::fs::write(&certificate, bytes).unwrap();
        let material = UserKey::load(&certificate, &private).unwrap();
        assert_eq!(material.certificate(), leaf);
        assert_eq!(format!("{material:?}"), "UserKey([REDACTED])");
        assert_eq!(format!("{material:#?}"), "UserKey([REDACTED])");
        assert_eq!(format!("{:?}", material.signing), "[REDACTED]");
        for policy in [
            SecurityPolicy::Basic256Sha256,
            SecurityPolicy::Aes128Sha256RsaOaep,
            SecurityPolicy::Aes256Sha256RsaPss,
        ] {
            let signature = material
                .sign(policy, b"server certificate and nonce")
                .unwrap();
            let key = pki::key_pair();
            if policy == SecurityPolicy::Aes256Sha256RsaPss {
                UnparsedPublicKey::new(
                    &const {
                        RSA_PSS_2048_8192_SHA256.with_pss_salt_length(RsaPssSaltLength::Custom(32))
                    },
                    key.public_key().as_ref(),
                )
                .verify(b"server certificate and nonce", &signature)
                .unwrap();
            } else {
                UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, key.public_key().as_ref())
                    .verify(b"server certificate and nonce", &signature)
                    .unwrap();
            }
        }
        assert!(material.sign(SecurityPolicy::None, b"refused").is_err());
    }
    let material = UserKey::load(&certificate, &private).unwrap();
    std::fs::remove_file(certificate).unwrap();
    std::fs::remove_file(private).unwrap();
    assert_eq!(material.certificate(), leaf);
}

#[test]
fn opcua_user_key_refuses_missing_malformed_oversized_and_mismatched_files_without_disclosure() {
    let scratch = Scratch::new();
    let certificate = scratch.write(
        "user-certificate-sentinel.pem",
        &pem("CERTIFICATE", &leaf()),
    );
    let private = scratch.write(
        "user-key-sentinel.pem",
        &pem("PRIVATE KEY", pki::pkcs8_2048()),
    );
    let missing = scratch.path().join("missing-sentinel");
    let bad = scratch.write("bad-sentinel.pem", "secret-content-sentinel");
    let oversized = scratch.write("oversized-sentinel.pem", &" ".repeat(1_048_577));
    let wrong = scratch.write(
        "wrong-sentinel.pem",
        &pem(
            "CERTIFICATE",
            &pki::certificate(
                &CertSpec::application("wrong", "wrong", 20),
                &pki::other_key_pair(),
                &pki::key_pair(),
            ),
        ),
    );
    let combined = scratch.write(
        "combined-sentinel.pem",
        &store(&[leaf()], pki::pkcs8_2048()),
    );
    let duplicate = scratch.write(
        "duplicate-sentinel.pem",
        &pem("PRIVATE KEY", pki::pkcs8_2048()).repeat(2),
    );
    let encrypted = scratch.write(
        "encrypted-sentinel.pem",
        &pem("ENCRYPTED PRIVATE KEY", pki::pkcs8_2048()),
    );
    let ed = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).unwrap();
    let non_rsa = scratch.write("ed-sentinel.pem", &pem("PRIVATE KEY", ed.as_ref()));
    for (cert, key) in [
        (&missing, &private),
        (&certificate, &missing),
        (&bad, &private),
        (&certificate, &bad),
        (&oversized, &private),
        (&certificate, &oversized),
        (&wrong, &private),
        (&certificate, &combined),
        (&certificate, &duplicate),
        (&certificate, &encrypted),
        (&certificate, &non_rsa),
    ] {
        let error = UserKey::load(cert, key).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.source().is_none());
        for text in [error.to_string(), format!("{error:?}")] {
            assert!(!text.contains("sentinel"));
            assert!(!text.contains(scratch.path().to_str().unwrap()));
        }
    }
}

#[test]
fn opcua_user_key_refuses_rsa_beyond_the_supported_policy_bounds() {
    use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
    use aws_lc_rs::rsa::KeySize;
    let scratch = Scratch::new();
    let pair = RsaKeyPair::generate(KeySize::Rsa8192).unwrap();
    let certificate = pki::certificate(
        &CertSpec::application("oversized", "oversized", 26),
        &pair,
        &pair,
    );
    let der: Pkcs8V1Der<'_> = pair.as_der().unwrap();
    let cert = scratch.write("user.pem", &pem("CERTIFICATE", &certificate));
    let key = scratch.write("user-key.pem", &pem("PRIVATE KEY", der.as_ref()));
    assert_eq!(
        UserKey::load(&cert, &key).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
}
