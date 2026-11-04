use std::error::Error as _;
use std::io::{self, Cursor};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::opcua::pki::{self, CertSpec, Scratch};

fn at() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(pki::NOW))
        .unwrap()
}
fn policy() -> SecurityPolicy {
    SecurityPolicy::Basic256Sha256
}
fn cert(spec: &CertSpec<'_>) -> CertificateDer<'static> {
    CertificateDer::from(pki::certificate(spec, &pki::key_pair(), &pki::key_pair()))
}
fn leaf() -> CertificateDer<'static> {
    cert(&CertSpec::application("server", "server", 1))
}
fn options(parameters: &[(&str, &str)]) -> OpcuaOptions {
    OpcuaOptions::from_parameters(
        "opc.tcp://plc.example:4840".to_owned(),
        &parameters
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect::<Vec<_>>(),
    )
    .unwrap()
}
fn assert_trust(error: &PlcError) {
    assert_eq!(error.kind(), ErrorKind::Trust);
    assert!(error.source().is_none());
    for key in [
        "server-certificate-file",
        "tls.trust-store",
        "security-policy=NONE",
        "tls.verify=false",
    ] {
        assert!(error.to_string().contains(key));
    }
}

// covers: OPCUA-CERT-VERIFY-ORDER
// covers: OPCUA-CERT-VERIFY-PERMISSIVE
#[test]
fn opcua_verify_false_beats_trust_store_and_pinning() {
    let scratch = Scratch::new();
    let pin = scratch.write("server.pem", &pki::pem("CERTIFICATE", leaf().as_ref()));
    let (server, verifier) = Verifier::prepare(&options(&[
        ("server-certificate-file", pin.to_str().unwrap()),
        ("tls.trust-store", "missing-sensitive.pem"),
        ("tls.verify", "false"),
    ]))
    .unwrap();
    assert_eq!(server.unwrap(), leaf());
    assert_eq!(verifier.name(), "permissive");
    verifier.verify(&[], policy(), at()).unwrap();
    verifier
        .verify(&[CertificateDer::from(vec![0xff])], policy(), at())
        .unwrap();
}

#[test]
fn opcua_tls_verify_false_accepts_a_self_signed_server_certificate() {
    let (_, verifier) = Verifier::prepare(&options(&[("tls.verify", "false")])).unwrap();
    verifier.verify(&[leaf()], policy(), at()).unwrap();
}

#[test]
fn opcua_trust_bypass_does_not_supply_a_non_rsa_bootstrap_key() {
    use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
    let key = Ed25519KeyPair::generate().unwrap();
    let spki = pki::seq(&[
        &[0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70],
        &pki::tlv(3, &[&[0], key.public_key().as_ref()].concat()),
    ]);
    let der = pki::certificate_with_spki(
        &CertSpec::application("non-RSA", "issuer", 1),
        &spki,
        &pki::key_pair(),
    );
    let scratch = Scratch::new();
    let path = scratch.write("server.pem", &pki::pem("CERTIFICATE", &der));
    let error = Verifier::prepare(&options(&[
        ("server-certificate-file", path.to_str().unwrap()),
        ("tls.verify", "false"),
    ]))
    .unwrap_err();
    assert_trust(&error);
}

// covers: OPCUA-CERT-VERIFY-PINNED
#[test]
fn opcua_pinned_certificate_accepts_the_identical_der() {
    let certificate = leaf();
    let verifier = Verifier::Pinned(certificate.clone());
    verifier
        .verify(std::slice::from_ref(&certificate), policy(), at())
        .unwrap();
    verifier
        .verify(
            &[certificate.clone(), CertificateDer::from(vec![0xff])],
            policy(),
            at(),
        )
        .unwrap();
    assert_trust(&verifier.verify(&[], policy(), at()).unwrap_err());
    assert_trust(
        &verifier
            .verify(
                &[CertificateDer::from(vec![0xff]), certificate],
                policy(),
                at(),
            )
            .unwrap_err(),
    );
}

#[test]
fn opcua_pinned_certificate_rejects_a_one_byte_difference() {
    let certificate = leaf();
    let verifier = Verifier::Pinned(certificate.clone());
    for position in 0..certificate.len() {
        let mut changed = certificate.as_ref().to_vec();
        *changed.get_mut(position).unwrap() ^= 1;
        assert_trust(
            &verifier
                .verify(&[CertificateDer::from(changed)], policy(), at())
                .unwrap_err(),
        );
    }
    let mut longer = certificate.as_ref().to_vec();
    longer.push(0);
    assert_trust(
        &verifier
            .verify(&[CertificateDer::from(longer)], policy(), at())
            .unwrap_err(),
    );
}

#[test]
fn opcua_an_expired_pinned_certificate_is_rejected() {
    let mut spec = CertSpec::application("server", "server", 2);
    spec.not_before = "010101000000Z";
    spec.not_after = "020101000000Z";
    let expired = cert(&spec);
    assert_trust(
        &Verifier::Pinned(expired.clone())
            .verify(&[expired], policy(), at())
            .unwrap_err(),
    );
    spec.not_before = "350101000000Z";
    spec.not_after = "360101000000Z";
    let future = cert(&spec);
    assert_trust(
        &Verifier::Pinned(future.clone())
            .verify(&[future], policy(), at())
            .unwrap_err(),
    );
}

// covers: OPCUA-DIV-FAIL-CLOSED
#[test]
fn opcua_default_verifier_rejects_every_server_certificate() {
    let (server, verifier) = Verifier::prepare(&options(&[])).unwrap();
    assert_eq!(server, None);
    assert_eq!(verifier.name(), "reject");
    for chain in [vec![], vec![leaf()], vec![CertificateDer::from(vec![0xff])]] {
        assert_trust(&verifier.verify(&chain, policy(), at()).unwrap_err());
    }
}

fn chain(signature: pki::Signature) -> [CertificateDer<'static>; 3] {
    let root = pki::other_key_pair();
    let issuer = pki::other_key_pair();
    let mut root_spec = CertSpec::application("root", "root", 1);
    root_spec.is_ca = true;
    let mut issuer_spec = CertSpec::application("issuer", "root", 2);
    issuer_spec.is_ca = true;
    issuer_spec.signature = signature;
    let mut server_spec = CertSpec::application("server", "issuer", 3);
    server_spec.signature = signature;
    [
        CertificateDer::from(pki::certificate(&root_spec, &root, &root)),
        CertificateDer::from(pki::certificate(&issuer_spec, &issuer, &root)),
        CertificateDer::from(pki::certificate(&server_spec, &pki::key_pair(), &issuer)),
    ]
}

// covers: OPCUA-CERT-VERIFY-TRUSTSTORE
#[test]
fn opcua_trust_store_accepts_a_chain_to_a_known_anchor() {
    for signature in [pki::Signature::Pkcs1, pki::Signature::Pss] {
        let [root, issuer, server] = chain(signature);
        let verifier = Verifier::Store(vec![root.clone()]);
        verifier
            .verify(&[server.clone(), issuer.clone()], policy(), at())
            .unwrap();
        verifier
            .verify(
                &[server.clone(), issuer.clone(), root.clone()],
                policy(),
                at(),
            )
            .unwrap();
        assert_trust(
            &verifier
                .verify(std::slice::from_ref(&server), policy(), at())
                .unwrap_err(),
        );
        let mut tampered = server.as_ref().to_vec();
        *tampered.last_mut().unwrap() ^= 1;
        assert_trust(
            &verifier
                .verify(&[CertificateDer::from(tampered), issuer], policy(), at())
                .unwrap_err(),
        );
        assert_trust(
            &Verifier::Store(vec![])
                .verify(&[server, root], policy(), at())
                .unwrap_err(),
        );
    }
}

#[test]
fn opcua_trust_store_rejects_an_unknown_issuer() {
    let [root, issuer, server] = chain(pki::Signature::Pkcs1);
    assert_trust(
        &Verifier::Store(vec![leaf()])
            .verify(&[server, issuer, root], policy(), at())
            .unwrap_err(),
    );
    assert_trust(
        &Verifier::Store(vec![leaf()])
            .verify(&[], policy(), at())
            .unwrap_err(),
    );
    assert_trust(
        &Verifier::Store(vec![])
            .verify(&[CertificateDer::from(vec![0xff])], policy(), at())
            .unwrap_err(),
    );
}

#[test]
fn opcua_a_certificate_from_discovery_is_not_an_anchor() {
    let [root, issuer, server] = chain(pki::Signature::Pkcs1);
    let (_, verifier) = Verifier::prepare(&options(&[])).unwrap();
    assert_trust(
        &verifier
            .verify(&[server, issuer, root], policy(), at())
            .unwrap_err(),
    );
}

#[test]
fn opcua_a_stored_certificate_with_mismatched_signature_algorithms_is_not_an_anchor() {
    let certificate = leaf();
    let mut malformed = certificate.as_ref().to_vec();
    let algorithm = rustls_pki_types::alg_id::RSA_PKCS1_SHA256;
    let start = malformed
        .windows(algorithm.as_ref().len())
        .rposition(|window| window == algorithm.as_ref())
        .unwrap();
    let hash = malformed.get_mut(start.checked_add(10).unwrap()).unwrap();
    assert_eq!(*hash, 0x0b);
    *hash = 0x0c;
    let malformed = CertificateDer::from(malformed);
    assert!(certificate::current(&malformed, at()));
    assert_trust(
        &Verifier::Store(vec![malformed])
            .verify(&[certificate], policy(), at())
            .unwrap_err(),
    );
}

#[test]
fn opcua_trust_store_rejects_an_expired_certificate() {
    let mut spec = CertSpec::application("server", "server", 4);
    let current = leaf();
    Verifier::Store(vec![current.clone()])
        .verify(std::slice::from_ref(&current), policy(), at())
        .unwrap();
    spec.not_before = "010101000000Z";
    spec.not_after = "020101000000Z";
    let expired = cert(&spec);
    assert_trust(
        &Verifier::Store(vec![expired.clone()])
            .verify(&[expired], policy(), at())
            .unwrap_err(),
    );
    let [root, issuer, server] = chain(pki::Signature::Pkcs1);
    let after_expiry = UNIX_EPOCH
        .checked_add(Duration::from_secs(2_114_380_801))
        .unwrap();
    assert_trust(
        &Verifier::Store(vec![root])
            .verify(&[server, issuer], policy(), after_expiry)
            .unwrap_err(),
    );
}

#[test]
fn opcua_trust_store_checks_anchor_and_intermediate_dates_independently() {
    let root_key = pki::other_key_pair();
    let issuer_key = pki::other_key_pair();
    let mut root_spec = CertSpec::application("root", "root", 1);
    root_spec.is_ca = true;
    let mut issuer_spec = CertSpec::application("issuer", "root", 2);
    issuer_spec.is_ca = true;
    let server_spec = CertSpec::application("server", "issuer", 3);
    let server = CertificateDer::from(pki::certificate(
        &server_spec,
        &pki::key_pair(),
        &issuer_key,
    ));
    let issuer = CertificateDer::from(pki::certificate(&issuer_spec, &issuer_key, &root_key));
    for (before, after) in [
        ("010101000000Z", "020101000000Z"),
        ("350101000000Z", "360101000000Z"),
    ] {
        root_spec.not_before = before;
        root_spec.not_after = after;
        let root = CertificateDer::from(pki::certificate(&root_spec, &root_key, &root_key));
        assert_trust(
            &Verifier::Store(vec![root])
                .verify(&[server.clone(), issuer.clone()], policy(), at())
                .unwrap_err(),
        );
    }
    root_spec.not_before = "260101000000Z";
    root_spec.not_after = "360101000000Z";
    let root = CertificateDer::from(pki::certificate(&root_spec, &root_key, &root_key));
    Verifier::Store(vec![root.clone()])
        .verify(&[server.clone(), issuer], policy(), at())
        .unwrap();
    issuer_spec.not_before = "010101000000Z";
    issuer_spec.not_after = "020101000000Z";
    let expired_issuer =
        CertificateDer::from(pki::certificate(&issuer_spec, &issuer_key, &root_key));
    assert_trust(
        &Verifier::Store(vec![root])
            .verify(&[server, expired_issuer], policy(), at())
            .unwrap_err(),
    );
}

#[test]
fn opcua_chain_verification_with_a_pre_epoch_clock_is_a_typed_trust_error() {
    let key = pki::other_key_pair();
    let mut root_spec = CertSpec::application("root", "root", 1);
    root_spec.is_ca = true;
    root_spec.not_before = "500101000000Z";
    root_spec.not_after = "600101000000Z";
    let mut server_spec = CertSpec::application("server", "root", 2);
    server_spec.not_before = "500101000000Z";
    server_spec.not_after = "600101000000Z";
    let root = CertificateDer::from(pki::certificate(&root_spec, &key, &key));
    let server = CertificateDer::from(pki::certificate(&server_spec, &pki::key_pair(), &key));
    let now = certificate::time(0x17, b"550101000000Z").unwrap();
    assert_trust(
        &Verifier::Store(vec![root])
            .verify(&[server], policy(), now)
            .unwrap_err(),
    );
}

#[test]
fn opcua_trust_store_beats_pinning() {
    let scratch = Scratch::new();
    let [root, issuer, server] = chain(pki::Signature::Pkcs1);
    let pin = scratch.write("pin.pem", &pki::pem("CERTIFICATE", leaf().as_ref()));
    let store = scratch.write("trust.pem", &pki::pem("CERTIFICATE", root.as_ref()));
    let (_, verifier) = Verifier::prepare(&options(&[
        ("server-certificate-file", pin.to_str().unwrap()),
        ("tls.trust-store", store.to_str().unwrap()),
    ]))
    .unwrap();
    assert_eq!(verifier.name(), "trust-store");
    verifier.verify(&[server, issuer], policy(), at()).unwrap();
    assert_trust(&verifier.verify(&[leaf()], policy(), at()).unwrap_err());
}

#[test]
fn opcua_pinning_alone_is_used_when_nothing_else_is_set() {
    let scratch = Scratch::new();
    let certificate = leaf();
    let pin = scratch.write("pin.pem", &pki::pem("CERTIFICATE", certificate.as_ref()));
    let (_, verifier) = Verifier::prepare(&options(&[(
        "server-certificate-file",
        pin.to_str().unwrap(),
    )]))
    .unwrap();
    assert_eq!(verifier.name(), "pinned");
    verifier.verify(&[certificate], policy(), at()).unwrap();
}

// covers: OPCUA-CFG-SERVER-CERTIFICATE-FILE
#[test]
fn opcua_server_certificate_file_accepts_der_and_pem() {
    let scratch = Scratch::new();
    let certificate = leaf();
    let der = scratch.path().join("server.der");
    std::fs::write(&der, certificate.as_ref()).unwrap();
    let pem = scratch.write("server.pem", &pki::pem("CERTIFICATE", certificate.as_ref()));
    for path in [der, pem] {
        let (bootstrap, verifier) = Verifier::prepare(&options(&[(
            "server-certificate-file",
            path.to_str().unwrap(),
        )]))
        .unwrap();
        assert_eq!(bootstrap.unwrap(), certificate);
        verifier
            .verify(std::slice::from_ref(&certificate), policy(), at())
            .unwrap();
        std::fs::remove_file(path).unwrap();
        verifier
            .verify(std::slice::from_ref(&certificate), policy(), at())
            .unwrap();
    }
}

#[test]
fn opcua_server_certificate_auto_detection_uses_structure_not_embedded_text() {
    let certificate = cert(&CertSpec::application(
        "-----BEGIN CERTIFICATE-----",
        "issuer",
        1,
    ));
    assert_eq!(
        parse(certificate.as_ref(), None).unwrap(),
        vec![certificate.clone()]
    );
    let text = format!(
        "0 comment before the certificate\r\n{}",
        pki::pem("CERTIFICATE", certificate.as_ref())
    );
    assert_eq!(parse(text.as_bytes(), None).unwrap(), vec![certificate]);
}

#[test]
fn opcua_trust_chain_count_and_byte_limits_accept_the_exact_boundary() {
    let certificate = leaf();
    for verifier in [
        Verifier::Pinned(certificate.clone()),
        Verifier::Store(vec![certificate.clone()]),
    ] {
        verifier
            .verify(&vec![certificate.clone(); 16], policy(), at())
            .unwrap();
        assert_trust(
            &verifier
                .verify(&vec![certificate.clone(); 17], policy(), at())
                .unwrap_err(),
        );
        let remaining = MAX_CERT_BYTES.checked_sub(certificate.len()).unwrap();
        verifier
            .verify(
                &[
                    certificate.clone(),
                    CertificateDer::from(vec![0; remaining]),
                ],
                policy(),
                at(),
            )
            .unwrap();
        assert_trust(
            &verifier
                .verify(
                    &[
                        certificate.clone(),
                        CertificateDer::from(vec![0; remaining.checked_add(1).unwrap()]),
                    ],
                    policy(),
                    at(),
                )
                .unwrap_err(),
        );
    }
}

#[test]
fn opcua_certificate_file_size_limit_includes_pem_padding() {
    let certificate = leaf();
    let mut exact = pki::pem("CERTIFICATE", certificate.as_ref()).into_bytes();
    exact.resize(MAX_CERT_BYTES, b' ');
    assert_eq!(
        parse(&exact, Some(CertificateFormat::Pem)).unwrap(),
        vec![certificate]
    );
    exact.push(b' ');
    assert!(parse(&exact, Some(CertificateFormat::Pem)).is_none());
}

#[test]
fn opcua_certificate_file_rejects_non_certificate_sections_among_certificates() {
    let certificate = pki::pem("CERTIFICATE", leaf().as_ref());
    for label in [
        "PRIVATE KEY",
        "ENCRYPTED PRIVATE KEY",
        "PUBLIC KEY",
        "UNKNOWN",
    ] {
        for newline in ["\n", "\r\n", "\r"] {
            let text = format!("{certificate}{}", pki::pem(label, b"unused test data"))
                .replace('\n', newline);
            assert!(parse(text.as_bytes(), Some(CertificateFormat::Pem)).is_none());
            assert!(parse(text.as_bytes(), None).is_none());
        }
    }
}

// covers: OPCUA-CFG-TRUST-STORE
// covers: OPCUA-DIV-JVM-KEYSTORES
#[test]
fn opcua_trust_store_pem_and_der_load_the_anchors() {
    let scratch = Scratch::new();
    let certificate = leaf();
    let der = scratch.path().join("trust.der");
    std::fs::write(&der, certificate.as_ref()).unwrap();
    let pem = scratch.write(
        "trust.pem",
        &format!(
            "{}{}",
            pki::pem("CERTIFICATE", certificate.as_ref()),
            pki::pem("CERTIFICATE", certificate.as_ref())
        ),
    );
    for (path, format) in [(der, "der"), (pem, "pem")] {
        let (_, verifier) = Verifier::prepare(&options(&[
            ("tls.trust-store", path.to_str().unwrap()),
            ("tls.trust-store-type", format),
        ]))
        .unwrap();
        verifier
            .verify(std::slice::from_ref(&certificate), policy(), at())
            .unwrap();
    }
}

#[test]
fn opcua_certificate_file_errors_are_redacted_and_bounded() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("sensitive-path"))
        }
    }
    let scratch = Scratch::new();
    let error = Verifier::prepare(&options(&[(
        "server-certificate-file",
        scratch.path().to_str().unwrap(),
    )]))
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(error.source().is_none());
    let malformed = scratch.write("sensitive.pem", "sensitive content");
    let duplicate = scratch.write(
        "two.pem",
        &pki::pem("CERTIFICATE", leaf().as_ref()).repeat(2),
    );
    for path in [
        malformed,
        duplicate,
        scratch.path().join("sensitive-missing.pem"),
    ] {
        let error = Verifier::prepare(&options(&[(
            "server-certificate-file",
            path.to_str().unwrap(),
        )]))
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.source().is_none());
        assert!(!format!("{error:?}").contains("sensitive"));
    }
    let error =
        Verifier::prepare(&options(&[("tls.trust-store", "sensitive-missing.pem")])).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(!format!("{error:?}").contains("sensitive"));
    let error = read(&mut Broken, "tls.trust-store").unwrap_err();
    assert!(error.source().is_none());
    assert!(!format!("{error:?}").contains("sensitive"));
    let mut overflow = Cursor::new(vec![0; MAX_CERT_BYTES.checked_add(10).unwrap()]);
    assert!(read(&mut overflow, "tls.trust-store").is_err());
    assert_eq!(overflow.position(), 1_048_577);
    assert_eq!(
        read(&mut vec![0; MAX_CERT_BYTES].as_slice(), "tls.trust-store")
            .unwrap()
            .len(),
        MAX_CERT_BYTES
    );
    assert!(parse(&vec![0; MAX_CERT_BYTES.checked_add(1).unwrap()], None).is_none());
    for bytes in [
        b"".as_slice(),
        b"-----BEGIN CERTIFICATE-----\n!\n-----END CERTIFICATE-----",
        b"-----BEGIN CERTIFICATE----",
        b"-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----",
    ] {
        assert!(parse(bytes, Some(CertificateFormat::Pem)).is_none());
    }
    assert!(parse(&[0x30, 0], Some(CertificateFormat::Der)).is_none());
    for verifier in [Verifier::Store(vec![leaf()]), Verifier::Pinned(leaf())] {
        assert_trust(
            &verifier
                .verify(&vec![leaf(); 17], policy(), at())
                .unwrap_err(),
        );
        assert_trust(
            &verifier
                .verify(
                    &[CertificateDer::from(vec![
                        0;
                        MAX_CERT_BYTES
                            .checked_add(1)
                            .unwrap()
                    ])],
                    policy(),
                    at(),
                )
                .unwrap_err(),
        );
    }
    assert_eq!(
        format!("{:?}", Verifier::Pinned(leaf())),
        "Verifier { strategy: \"pinned\" }"
    );
}

struct Captured(Arc<Mutex<Vec<u8>>>);
impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
    type Writer = tracing_subscriber::fmt::writer::MutexGuardWriter<'a, Vec<u8>>;
    fn make_writer(&'a self) -> Self::Writer {
        tracing_subscriber::fmt::MakeWriter::make_writer(self.0.as_ref())
    }
}

// covers: OPCUA-CFG-VERIFY
#[test]
fn opcua_tls_verify_false_is_recorded_in_diagnostics() {
    // tracing-core#3611: keep registration off the single-dispatch TLS fast path.
    let _registered = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
    let output = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(Captured(Arc::clone(&output)))
        .finish();
    let opts = options(&[
        ("tls.verify", "false"),
        ("tls.trust-store", "sensitive-unused-file.pem"),
    ]);
    assert!(format!("{opts:?}").contains("verify: false"));
    tracing::subscriber::with_default(subscriber, || {
        std::thread::scope(|scope| {
            scope
                .spawn(|| Verifier::prepare(&opts).unwrap())
                .join()
                .unwrap();
        });
        Verifier::prepare(&opts).unwrap();
    });
    let text = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for value in ["WARN", "tls_verify=false", "tls.verify=false", "permissive"] {
        assert!(text.contains(value), "{text}");
    }
    assert!(!text.contains("sensitive"));
}
