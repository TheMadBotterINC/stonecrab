use std::error::Error as _;

use aws_lc_rs::encoding::{AsDer, Pkcs8V1Der};
use aws_lc_rs::rsa::{KeySize, PrivateDecryptingKey};
use aws_lc_rs::signature::RsaKeyPair;

use super::*;
use crate::opcua::frame::{ChunkType, OpenChunk, SequenceHeader};
use crate::opcua::options::MessageSecurity;
use crate::opcua::pki::{self, CertSpec};

const POLICIES: [SecurityPolicy; 3] = [
    SecurityPolicy::Basic256Sha256,
    SecurityPolicy::Aes128Sha256RsaOaep,
    SecurityPolicy::Aes256Sha256RsaPss,
];

fn material(pair: RsaKeyPair) -> KeyMaterial {
    let leaf = pki::certificate(
        &CertSpec::application("instance", "instance", 1),
        &pair,
        &pair,
    );
    let der: Pkcs8V1Der<'_> = pair.as_der().unwrap();
    let decrypting = PrivateDecryptingKey::from_pkcs8(der.as_ref()).unwrap();
    let thumbprint = digest(&SHA1_FOR_LEGACY_USE_ONLY, &leaf).as_ref().to_vec();
    KeyMaterial {
        sender_certificate: leaf.clone(),
        leaf: CertificateDer::from(leaf),
        thumbprint,
        application_uri: "urn:stonecrab:probe".to_owned(),
        signing: Secret::new(pair),
        decrypting: Secret::new(decrypting),
    }
}

fn endpoints(policy: SecurityPolicy) -> (Asymmetric, Asymmetric) {
    with_keys(policy, pki::key_pair(), pki::other_key_pair())
}

fn with_keys(policy: SecurityPolicy, a: RsaKeyPair, b: RsaKeyPair) -> (Asymmetric, Asymmetric) {
    let a = material(a);
    let b = material(b);
    let a_leaf = a.leaf.clone();
    let b_leaf = b.leaf.clone();
    (
        Asymmetric::new(policy, a, Some(b_leaf)).unwrap(),
        Asymmetric::new(policy, b, Some(a_leaf)).unwrap(),
    )
}

fn frame(endpoint: &Asymmetric, body: &[u8]) -> Frame {
    Frame::single(Body::Open(OpenChunk {
        channel_id: 17,
        security: endpoint.header(),
        sequence: SequenceHeader {
            sequence_number: 7,
            request_id: 9,
        },
        body: Bytes::copy_from_slice(body),
    }))
}

fn clear(bytes: &[u8]) -> usize {
    let mut reader = BoundedReader::new(bytes);
    FrameHeader::parse(&mut reader).unwrap();
    reader.read_u32(LITTLE).unwrap();
    AsymmetricHeader::parse(&mut reader).unwrap();
    bytes.len().checked_sub(reader.remaining()).unwrap()
}

/// Native block operations, separate from the engine's seal/open implementation.
fn decrypted(receiver: &Asymmetric, wire: &[u8]) -> Vec<u8> {
    let offset = clear(wire);
    let mut plain = wire.get(..offset).unwrap().to_vec();
    let key = receiver.decrypting.expose();
    let mut scratch = vec![0; key.min_output_size()];
    for block in wire
        .get(offset..)
        .unwrap()
        .chunks_exact(key.key_size_bytes())
    {
        plain.extend_from_slice(
            key.decrypt(receiver.policy.oaep().unwrap(), block, &mut scratch, None)
                .unwrap(),
        );
    }
    plain
}

fn encrypted(sender: &Asymmetric, plain: &[u8]) -> Vec<u8> {
    let offset = clear(plain);
    let mut out = plain.get(..offset).unwrap().to_vec();
    let key = &sender.encrypting;
    let mut scratch = vec![0; key.ciphertext_size()];
    assert!(
        plain
            .len()
            .checked_sub(offset)
            .unwrap()
            .is_multiple_of(key.max_plaintext_size(sender.policy.oaep().unwrap()))
    );
    for block in plain
        .get(offset..)
        .unwrap()
        .chunks_exact(key.max_plaintext_size(sender.policy.oaep().unwrap()))
    {
        out.extend_from_slice(
            key.encrypt(sender.policy.oaep().unwrap(), block, &mut scratch, None)
                .unwrap(),
        );
    }
    out
}

fn resign(sender: &Asymmetric, plain: &mut [u8]) {
    let auth_end = plain
        .len()
        .checked_sub(sender.local.signing.expose().public_modulus_len())
        .unwrap();
    let (message, signature) = plain.split_at_mut(auth_end);
    sender
        .local
        .signing
        .expose()
        .sign(
            sender.policy.encoding().unwrap(),
            &SystemRandom::new(),
            message,
            signature,
        )
        .unwrap();
}

fn refused(endpoint: &Asymmetric, bytes: &[u8]) -> PlcError {
    let error = endpoint.open(bytes, 65535).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Authentication);
    assert_eq!(error.to_string(), rejected().to_string());
    assert!(error.source().is_none());
    error
}

#[test]
fn opcua_opn_chunk_is_signed_and_encrypted_block_by_block() {
    for policy in POLICIES {
        let (a, b) = endpoints(policy);
        for flag in [ChunkType::Final, ChunkType::Intermediate, ChunkType::Abort] {
            let mut frame = frame(&a, &vec![0xab; 1024]);
            frame.chunk = flag;
            let wire = a.seal(&frame, 65535).unwrap();
            assert_eq!(b.open(&wire, 65535).unwrap(), frame);
            assert!(
                wire.len()
                    .checked_sub(clear(&wire))
                    .unwrap()
                    .is_multiple_of(256)
            );
            let plain = decrypted(&b, &wire);
            let end = plain.len().checked_sub(256).unwrap();
            let algorithm = if policy == SecurityPolicy::Aes256Sha256RsaPss {
                &PSS_SHA256
            } else {
                &PKCS1_SHA256
            };
            webpki::EndEntityCert::try_from(&a.local.leaf)
                .unwrap()
                .verify_signature(
                    algorithm,
                    plain.get(..end).unwrap(),
                    plain.get(end..).unwrap(),
                )
                .unwrap();
            assert_eq!(encrypted(&a, &plain).len(), wire.len());
            assert_ne!(
                wire,
                a.seal(&frame, 65535).unwrap(),
                "OAEP must use fresh randomness"
            );
            let response = frame_for_peer(&b);
            assert_eq!(
                a.open(&b.seal(&response, 65535).unwrap(), 65535).unwrap(),
                response
            );
        }
    }
}

fn frame_for_peer(peer: &Asymmetric) -> Frame {
    frame(peer, b"response")
}

#[test]
fn opcua_open_secure_channel_is_signed_and_encrypted_even_in_mode_sign() {
    for _mode in [MessageSecurity::Sign, MessageSecurity::SignEncrypt] {
        let (a, b) = endpoints(SecurityPolicy::Basic256Sha256);
        let request = frame(&a, b"secret channel nonce");
        let wire = a.seal(&request, 65535).unwrap();
        assert!(
            !wire
                .windows(20)
                .any(|bytes| bytes == b"secret channel nonce")
        );
        assert_eq!(b.open(&wire, 65535).unwrap(), request);
    }
}

#[test]
fn opcua_asymmetric_headers_carry_the_chain_and_leaf_thumbprint() {
    let (mut a, b) = endpoints(SecurityPolicy::Basic256Sha256);
    a.local
        .sender_certificate
        .extend_from_slice(b.local.leaf.as_ref());
    let request = frame(&a, b"chain");
    let header = a.header();
    assert_eq!(
        header.sender_certificate.as_bytes(),
        Some(a.local.sender_certificate.as_slice())
    );
    assert_eq!(
        header.receiver_thumbprint.as_bytes(),
        Some(b.local.thumbprint.as_slice())
    );
    assert_eq!(
        b.open(&a.seal(&request, 65535).unwrap(), 65535).unwrap(),
        request
    );
    assert_eq!(
        format!("{a:?}"),
        "Asymmetric { policy: Basic256Sha256, .. }"
    );
}

#[test]
fn opcua_asymmetric_padding_and_exact_chunk_limits_cover_every_residue() {
    for policy in POLICIES {
        let (a, b) = endpoints(policy);
        let block = a.encrypting.max_plaintext_size(a.policy.oaep().unwrap());
        for length in 0..block {
            let request = frame(&a, &vec![0x91; length]);
            let wire = a.seal(&request, 65535).unwrap();
            assert_eq!(b.open(&wire, wire.len()).unwrap(), request);
            assert_eq!(a.seal(&request, wire.len()).unwrap().len(), wire.len());
            assert_eq!(
                a.seal(&request, wire.len().checked_sub(1).unwrap())
                    .unwrap_err()
                    .kind(),
                ErrorKind::ResourceLimit
            );
            assert_eq!(
                b.open(&wire, wire.len().checked_sub(1).unwrap())
                    .unwrap_err()
                    .kind(),
                ErrorKind::ResourceLimit
            );
        }
        let capacity = a.max_body(8192).unwrap();
        let request = frame(&a, &vec![0; capacity]);
        assert!(a.seal(&request, 8192).unwrap().len() <= 8192);
        assert_eq!(
            a.seal(&frame(&a, &vec![0; capacity.checked_add(1).unwrap()]), 8192)
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(a.max_body(1).unwrap_err().kind(), ErrorKind::ResourceLimit);
    }
}

#[test]
fn opcua_asymmetric_padding_adds_two_bytes_for_a_4096_bit_key() {
    let large = || RsaKeyPair::generate(KeySize::Rsa4096).unwrap();
    for (left, right) in [(pki::key_pair(), large()), (large(), pki::key_pair())] {
        let (a, b) = with_keys(SecurityPolicy::Basic256Sha256, left, right);
        for (sender, receiver) in [(&a, &b), (&b, &a)] {
            // At least one count exceeds 255, exercising both padding-size bytes.
            let mut high_seen = false;
            for length in [0, 1, 40, 100, 213, 214, 300, 469, 470] {
                let request = frame(sender, &vec![0x49; length]);
                let wire = sender.seal(&request, 65535).unwrap();
                assert_eq!(receiver.open(&wire, 65535).unwrap(), request);
                let plain = decrypted(receiver, &wire);
                let end = plain
                    .len()
                    .checked_sub(sender.local.signing.expose().public_modulus_len())
                    .unwrap();
                let message = plain.get(..end).unwrap();
                if sender.encrypting.key_size_bytes() > 256 {
                    high_seen |= message.last() == Some(&1);
                    let mut corrupt = plain.clone();
                    *corrupt.get_mut(end.checked_sub(1).unwrap()).unwrap() = 0xff;
                    resign(sender, &mut corrupt);
                    refused(receiver, &encrypted(sender, &corrupt));
                }
            }
            assert_eq!(high_seen, sender.encrypting.key_size_bytes() > 256);
        }
    }
}

#[test]
fn opcua_a_tampered_opn_response_fails_with_one_uniform_error() {
    let (a, b) = endpoints(SecurityPolicy::Basic256Sha256);
    let wire = a.seal(&frame(&a, b"payload"), 65535).unwrap();
    for index in 0..wire.len() {
        let mut corrupt = wire.to_vec();
        *corrupt.get_mut(index).unwrap() ^= 1;
        refused(&b, &corrupt);
    }
    for length in 0..wire.len() {
        refused(&b, wire.get(..length).unwrap());
    }
    let mut trailing = wire.to_vec();
    trailing.push(0);
    refused(&b, &trailing);
    let mut invalid_signature = decrypted(&b, &wire);
    *invalid_signature.last_mut().unwrap() ^= 1;
    refused(&b, &encrypted(&a, &invalid_signature));
    let (c, _d) = endpoints(SecurityPolicy::Basic256Sha256);
    refused(&c, &wire);
}

#[test]
fn opcua_signed_bad_padding_and_short_rsa_blocks_are_refused() {
    for policy in POLICIES {
        let (a, b) = endpoints(policy);
        let wire = a.seal(&frame(&a, b"padding"), 65535).unwrap();
        let mut plain = decrypted(&b, &wire);
        let auth_end = plain.len().checked_sub(256).unwrap();
        let pad = auth_end.checked_sub(2).unwrap();
        *plain.get_mut(pad).unwrap() ^= 1;
        resign(&a, &mut plain);
        refused(&b, &encrypted(&a, &plain));

        // Valid OAEP can carry a short block. UASC requires exact plaintext blocks.
        let offset = clear(&wire);
        let mut short = wire.get(..offset).unwrap().to_vec();
        let mut scratch = vec![0; 256];
        short.extend_from_slice(
            a.encrypting
                .encrypt(a.policy.oaep().unwrap(), b"short", &mut scratch, None)
                .unwrap(),
        );
        let size = u32::try_from(short.len()).unwrap().to_le_bytes();
        short.get_mut(4..8).unwrap().copy_from_slice(&size);
        refused(&b, &short);
    }
    assert!(unpad(&[], 0, 512).is_err());
    assert!(unpad(&[], 0, 256).is_err());
    assert!(unpad(&[255], 0, 256).is_err());
    assert!(first_certificate(&[0x31, 0]).is_err());
    assert!(first_certificate(&[0x30, 0x80]).is_err());
}

#[test]
fn opcua_opn_encryption_fails_fast_without_a_server_certificate() {
    let error = Asymmetric::new(
        SecurityPolicy::Basic256Sha256,
        material(pki::key_pair()),
        None,
    )
    .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    assert!(error.to_string().contains("before I/O"));
    assert!(Asymmetric::new(SecurityPolicy::None, material(pki::key_pair()), None).is_err());
    assert!(
        Asymmetric::new(
            SecurityPolicy::Basic256Sha256,
            material(pki::key_pair()),
            Some(CertificateDer::from(vec![0]))
        )
        .is_err()
    );
    let (a, _b) = endpoints(SecurityPolicy::Basic256Sha256);
    let other = Frame::single(Body::Error(crate::opcua::frame::ErrorMessage {
        error: crate::opcua::codec::StatusCode(0),
        reason: UaString::null(),
    }));
    assert_eq!(
        a.seal(&other, 65535).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
    let mut wrong = frame(&a, b"");
    let Body::Open(chunk) = &mut wrong.body else {
        panic!("expected OPN")
    };
    chunk.security = AsymmetricHeader::none();
    assert_eq!(
        a.seal(&wrong, 65535).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
}

#[test]
fn opcua_local_rsa_key_outside_policy_bounds_is_refused() {
    let local = material(RsaKeyPair::generate(KeySize::Rsa8192).unwrap());
    let peer = material(pki::key_pair()).leaf;
    assert_eq!(
        Asymmetric::new(SecurityPolicy::Basic256Sha256, local, Some(peer))
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidConfig
    );
}

#[test]
fn opcua_protected_opn_rejects_a_partial_cipher_block_before_decryption() {
    let (a, b) = endpoints(SecurityPolicy::Basic256Sha256);
    let wire = a.seal(&frame(&a, b"body"), 65535).unwrap();
    for count in [0, 1, 255, 257] {
        let end = clear(&wire).checked_add(count).unwrap();
        let mut short = wire.get(..end).unwrap().to_vec();
        let size = u32::try_from(short.len()).unwrap().to_le_bytes();
        short.get_mut(4..8).unwrap().copy_from_slice(&size);
        refused(&b, &short);
    }
    // Correct signature over the declared size, followed by a partial trailing block.
    // A decoder silently dropping chunks_exact's remainder would accept this packet.
    let mut plain = decrypted(&b, &wire);
    let size = u32::try_from(wire.len() + 1).unwrap().to_le_bytes();
    plain.get_mut(4..8).unwrap().copy_from_slice(&size);
    resign(&a, &mut plain);
    let mut trailing = encrypted(&a, &plain);
    trailing.push(0);
    refused(&b, &trailing);
}

#[test]
fn opcua_opn_pkcs1_verification_matches_a_fixed_openssl_signature() {
    // S104's public OpenSSL self-signed certificate supplies fixed TBS/signature bytes.
    // No private key is retained; verification uses the same policy path as OPN.
    let der = include_bytes!("../../../../../../fuzz/corpus/fuzz_opcua_certificate/rsa-pkcs1.der");
    let engine = Asymmetric::new(
        SecurityPolicy::Basic256Sha256,
        material(pki::key_pair()),
        Some(CertificateDer::from(der.to_vec())),
    )
    .unwrap();
    let mut outer = BoundedReader::new(der);
    let (tag, cert) = certificate::tlv(&mut outer).unwrap();
    assert_eq!(tag, 0x30);
    let mut reader = BoundedReader::new(cert);
    certificate::tlv(&mut reader).unwrap();
    let tbs_end = cert.len().checked_sub(reader.remaining()).unwrap();
    let tbs = cert.get(..tbs_end).unwrap();
    certificate::tlv(&mut reader).unwrap();
    let (tag, bits) = certificate::tlv(&mut reader).unwrap();
    assert_eq!(tag, 3);
    assert_eq!(bits.first(), Some(&0));
    let signature = bits.get(1..).unwrap();
    engine.verify(tbs, signature).unwrap();
    assert!(engine.verify(b"different message", signature).is_err());
    let mut changed = signature.to_vec();
    *changed.last_mut().unwrap() ^= 1;
    assert!(engine.verify(tbs, &changed).is_err());
}

#[test]
fn opcua_authenticated_opn_headers_cannot_substitute_the_peer_or_policy() {
    let (a, b) = endpoints(SecurityPolicy::Basic256Sha256);
    let wire = a.seal(&frame(&a, b"bound identity"), 65535).unwrap();
    let plain = decrypted(&b, &wire);
    let policy_start = 16;
    let cert_start = policy_start + a.policy.descriptor().uri.len() + 4;
    let thumb_start = cert_start + a.local.sender_certificate.len() + 4;
    for position in [policy_start, cert_start, thumb_start] {
        let mut changed = plain.clone();
        *changed.get_mut(position).unwrap() ^= 1;
        resign(&a, &mut changed);
        refused(&b, &encrypted(&a, &changed));
    }
    // Structurally valid, same-key certificate with a different serial is not the pin.
    let alternate = pki::certificate(
        &CertSpec::application("instance", "instance", 2),
        a.local.signing.expose(),
        a.local.signing.expose(),
    );
    assert_eq!(alternate.len(), a.local.sender_certificate.len());
    let mut changed = plain.clone();
    let end = cert_start + alternate.len();
    changed
        .get_mut(cert_start..end)
        .unwrap()
        .copy_from_slice(&alternate);
    resign(&a, &mut changed);
    refused(&b, &encrypted(&a, &changed));
    // The wire size remains authenticated even when it is otherwise structurally legal.
    let mut changed = plain.clone();
    let size = u32::try_from(wire.len() + 1).unwrap();
    changed
        .get_mut(4..8)
        .unwrap()
        .copy_from_slice(&size.to_le_bytes());
    resign(&a, &mut changed);
    refused(&b, &encrypted(&a, &changed));
    let mut changed = plain;
    changed.get_mut(..3).unwrap().copy_from_slice(b"MSG");
    resign(&a, &mut changed);
    refused(&b, &encrypted(&a, &changed));
}
