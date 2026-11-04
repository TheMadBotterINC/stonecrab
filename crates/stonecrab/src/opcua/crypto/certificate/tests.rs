use super::*;
use crate::opcua::pki::{self, CertSpec};

fn now() -> SystemTime {
    UNIX_EPOCH
        .checked_add(Duration::from_secs(pki::NOW))
        .unwrap()
}

#[test]
fn opcua_signature_algorithm_debug_contains_only_the_algorithm_name() {
    assert_eq!(format!("{PKCS1_SHA256:?}"), "Pkcs1");
    assert_eq!(format!("{PSS_SHA256:?}"), "Pss");
}

fn application(spec: &CertSpec<'_>) -> CertificateDer<'static> {
    CertificateDer::from(pki::certificate(spec, &pki::key_pair(), &pki::key_pair()))
}

#[test]
fn opcua_direct_certificate_validity_and_server_eku_are_required() {
    let mut spec = CertSpec::application("server", "server", 1);
    let cert = application(&spec);
    for policy in [
        SecurityPolicy::Basic256Sha256,
        SecurityPolicy::Aes128Sha256RsaOaep,
        SecurityPolicy::Aes256Sha256RsaPss,
    ] {
        check_leaf(&cert, policy, now()).unwrap();
        assert_eq!(server_key(&cert, policy).unwrap().key_size_bytes(), 256);
    }
    assert!(server_key(&cert, SecurityPolicy::None).is_err());
    spec.eku = None;
    check_leaf(&application(&spec), SecurityPolicy::Basic256Sha256, now()).unwrap();
    let client = [pki::CLIENT_AUTH];
    spec.eku = Some(&client);
    assert_eq!(
        check_leaf(&application(&spec), SecurityPolicy::Basic256Sha256, now())
            .err()
            .unwrap()
            .kind(),
        crate::ErrorKind::Trust
    );
    spec.eku = Some(&[]);
    assert!(check_leaf(&application(&spec), SecurityPolicy::Basic256Sha256, now()).is_err());
}

#[test]
fn opcua_validity_is_inclusive_and_uses_calendar_dates() {
    let cert = application(&CertSpec::application("server", "server", 2));
    let properties = Properties::parse(cert.as_ref()).unwrap();
    assert!(current(&cert, properties.not_before));
    assert!(current(&cert, properties.not_after));
    assert!(!current(
        &cert,
        properties
            .not_before
            .checked_sub(Duration::from_nanos(1))
            .unwrap()
    ));
    assert!(!current(
        &cert,
        properties
            .not_after
            .checked_add(Duration::from_nanos(1))
            .unwrap()
    ));
    for text in [
        "260230000000Z",
        "260101240000Z",
        "260101006000Z",
        "260101000060Z",
        "260100000000Z",
        "261301000000Z",
        "260101000000+0000",
        "260101000000Zx",
        "26010100000Z",
        "2x0101000000Z",
    ] {
        assert!(time(0x17, text.as_bytes()).is_none(), "{text}");
    }
    assert!(time(0x17, b"000229000000Z").is_some());
    assert_eq!(
        time(0x17, b"261101123456Z")
            .unwrap()
            .duration_since(time(0x17, b"261101000000Z").unwrap())
            .unwrap(),
        Duration::from_secs(45296)
    );
    assert!(time(0x17, b"010229000000Z").is_none());
    assert_eq!(time(0x17, b"700101000000Z"), Some(UNIX_EPOCH));
    assert!(time(0x17, b"500101000000Z").unwrap() < UNIX_EPOCH);
    assert!(time(0x17, b"490101000000Z").unwrap() > now());
    assert!(time(0x18, b"20500101000000Z").is_some());
    assert!(time(0x18, b"20490101000000Z").is_none());
    assert!(time(0x18, b"20500230000000Z").is_none());
    assert!(time(0x16, b"260101000000Z").is_none());
}

#[test]
fn opcua_generalized_time_and_future_or_inverted_validity_are_checked() {
    let mut spec = CertSpec::application("server", "server", 3);
    spec.not_before = "20500101000000Z";
    spec.not_after = "20600101000000Z";
    let cert = application(&spec);
    assert!(!current(&cert, now()));
    let at = time(0x18, b"20550101000000Z").unwrap();
    check_leaf(&cert, SecurityPolicy::Basic256Sha256, at).unwrap();
    spec.not_before = "360101000000Z";
    spec.not_after = "260101000000Z";
    assert!(!current(&application(&spec), now()));
    spec.not_before = "260101000000Z";
    spec.not_after = "361332000000Z";
    let malformed_date = application(&spec);
    webpki::EndEntityCert::try_from(&malformed_date).unwrap();
    assert_eq!(
        check_leaf(&malformed_date, SecurityPolicy::Basic256Sha256, now())
            .err()
            .unwrap()
            .kind(),
        crate::ErrorKind::Trust
    );
}

#[test]
fn opcua_der_reader_rejects_noncanonical_lengths_and_truncation() {
    for bytes in [
        vec![],
        vec![0x30],
        vec![0x1f, 0],
        vec![0x30, 0x80],
        vec![0x30, 0x85],
        vec![0x30, 0x81, 0],
        vec![0x30, 0x81, 127],
        vec![0x30, 0x82, 0, 128],
        vec![0x30, 0x82, 1],
        vec![0x30, 0xff],
        vec![0x30, 0x84, 0xff, 0xff, 0xff, 0xff],
    ] {
        assert!(tlv(&mut BoundedReader::new(&bytes)).is_none(), "{bytes:?}");
    }
    for header in [[0x30, 0x80].as_slice(), [0x30, 0x81, 127].as_slice()] {
        let bytes = [header, vec![42; 128].as_slice()].concat();
        assert!(tlv(&mut BoundedReader::new(&bytes)).is_none());
    }
    let exact = pki::tlv(0x04, &[42; 128]);
    assert_eq!(
        tlv(&mut BoundedReader::new(&exact)),
        Some((4, vec![42; 128].as_slice()))
    );
    let short = pki::tlv(0x04, &[42; 127]);
    assert_eq!(
        tlv(&mut BoundedReader::new(&short)),
        Some((4, vec![42; 127].as_slice()))
    );
    let bytes = pki::tlv(0x04, &vec![42; 256]);
    assert_eq!(
        tlv(&mut BoundedReader::new(&bytes)),
        Some((4, vec![42; 256].as_slice()))
    );
    assert!(element(&mut BoundedReader::new(&[4, 0]), 0x30).is_none());
    assert!(Properties::parse(&vec![0; MAX_CERT_BYTES.checked_add(1).unwrap()]).is_none());
    assert!(!current(&CertificateDer::from(vec![0]), now()));
    assert!(
        server_key(
            &CertificateDer::from(vec![0; MAX_CERT_BYTES.checked_add(1).unwrap()]),
            SecurityPolicy::Basic256Sha256
        )
        .is_err()
    );
    assert!(
        server_key(
            &CertificateDer::from(vec![0]),
            SecurityPolicy::Basic256Sha256
        )
        .is_err()
    );
}

fn skeleton(version: &[u8], extra: &[u8]) -> Vec<u8> {
    let validity = pki::seq(&[
        &pki::tlv(0x17, b"260101000000Z"),
        &pki::tlv(0x17, b"360101000000Z"),
    ]);
    let tbs = pki::seq(&[
        version,
        &[2, 1, 1],
        &[0x30, 0],
        &[0x30, 0],
        &validity,
        &[0x30, 0],
        &[0x30, 0],
        extra,
    ]);
    pki::seq(&[&tbs, &[0x30, 0], &[3, 1, 0]])
}

#[test]
fn opcua_property_reader_checks_extensions_and_critical_eku_without_trusting_them() {
    let version = [0xa0, 3, 2, 1, 2];
    assert!(Properties::parse(&skeleton(&[0xa0, 3, 2, 1, 1], &[])).is_none());
    let oid = pki::tlv(6, &[0x55, 0x1d, 0x25]);
    let eku = pki::seq(&[&pki::tlv(6, pki::SERVER_AUTH)]);
    let good = pki::seq(&[&oid, &[1, 1, 0xff], &pki::tlv(4, &eku)]);
    let ext = pki::tlv(0xa3, &pki::seq(&[&good]));
    assert!(
        Properties::parse(&skeleton(&version, &ext))
            .unwrap()
            .server_auth
    );
    let ids = [&[0x81, 1, 0, 0x82, 1, 0], ext.as_slice()].concat();
    assert!(
        Properties::parse(&skeleton(&version, &ids))
            .unwrap()
            .server_auth
    );
    let bad = pki::seq(&[&oid, &[2, 1, 0]]);
    assert!(Properties::parse(&skeleton(&version, &pki::tlv(0xa3, &pki::seq(&[&bad])))).is_none());
    for bytes in [&[][..], &[0x30, 1, 0], &[0x30, 0, 0], &[0x30, 3, 2, 1, 0]] {
        assert!(server_eku(bytes).is_none());
    }
    assert_eq!(server_eku(&[0x30, 0]), Some(false));
}

#[test]
fn opcua_certificate_rsa_bounds_are_typed_trust_errors() {
    for size in [128_usize, 513, 1024] {
        let mut modulus = vec![0x55; size];
        *modulus.first_mut().unwrap() = 0x80;
        let integer = [&[0], modulus.as_slice()].concat();
        let public = pki::seq(&[&pki::tlv(2, &integer), &pki::tlv(2, &[1, 0, 1])]);
        let spki = pki::seq(&[
            &pki::tlv(0x30, alg_id::RSA_ENCRYPTION.as_ref()),
            &pki::tlv(3, &[&[0], public.as_slice()].concat()),
        ]);
        let cert = CertificateDer::from(pki::certificate_with_spki(
            &CertSpec::application("server", "server", 4),
            &spki,
            &pki::key_pair(),
        ));
        let error = server_key(&cert, SecurityPolicy::Basic256Sha256).unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::Trust);
        assert_eq!(
            check_leaf(&cert, SecurityPolicy::Basic256Sha256, now())
                .err()
                .unwrap()
                .kind(),
            crate::ErrorKind::Trust
        );
    }
    for (size, accepted) in [(255, false), (256, true), (512, true), (513, false)] {
        assert_eq!(
            SecurityPolicy::Basic256Sha256
                .descriptor()
                .accepts_rsa_key(size),
            accepted
        );
    }
}

#[test]
fn opcua_property_reader_enforces_its_size_limit_with_structurally_valid_der() {
    let oversized_name = "X".repeat(MAX_CERT_BYTES);
    let mut spec = CertSpec::application(&oversized_name, "issuer", 1);
    let oversized = application(&spec);
    assert!(oversized.len() > MAX_CERT_BYTES);
    assert!(server_key(&oversized, SecurityPolicy::Basic256Sha256).is_err());
    assert!(Properties::parse(oversized.as_ref()).is_none());
    let overhead = oversized.len().checked_sub(oversized_name.len()).unwrap();
    let exact_name = "X".repeat(MAX_CERT_BYTES.checked_sub(overhead).unwrap());
    spec.subject = &exact_name;
    let exact = application(&spec);
    assert_eq!(exact.len(), MAX_CERT_BYTES);
    assert!(server_key(&exact, SecurityPolicy::Basic256Sha256).is_err());
    assert!(Properties::parse(exact.as_ref()).unwrap().current(now()));
}
