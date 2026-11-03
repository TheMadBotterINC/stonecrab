use std::error::Error;
use std::sync::Mutex;

use super::*;
use crate::opcua::crypto::Native;
use crate::opcua::crypto::Symmetric;
use crate::opcua::crypto::backend::Hash;
use crate::opcua::crypto::tests::{hex, unhex};
use crate::opcua::frame::{ChunkType, SecureChunk, SequenceHeader};

const POLICIES: [SecurityPolicy; 3] = [
    SecurityPolicy::Basic256Sha256,
    SecurityPolicy::Aes128Sha256RsaOaep,
    SecurityPolicy::Aes256Sha256RsaPss,
];
const MODES: [MessageSecurity; 2] = [MessageSecurity::Sign, MessageSecurity::SignEncrypt];

fn endpoint(policy: SecurityPolicy, mode: MessageSecurity) -> Symmetric {
    Symmetric::new(
        &Native,
        policy,
        mode,
        17,
        29,
        &(0..32).collect::<Vec<u8>>(),
        &(32..64).collect::<Vec<u8>>(),
    )
    .unwrap()
}

fn peer(policy: SecurityPolicy, mode: MessageSecurity) -> Symmetric {
    let mut peer = endpoint(policy, mode);
    swap(&mut peer);
    peer
}

fn swap(endpoint: &mut Symmetric) {
    std::mem::swap(
        &mut endpoint.current.keys.client,
        &mut endpoint.current.keys.server,
    );
}

fn frame(body: &[u8], token_id: u32) -> Frame {
    Frame::single(Body::Message(SecureChunk {
        channel_id: 17,
        token_id,
        sequence: SequenceHeader {
            sequence_number: 7,
            request_id: 9,
        },
        body: Bytes::copy_from_slice(body),
    }))
}

#[derive(Default)]
struct Fault {
    fail: &'static str,
    bad_mac: bool,
    calls: Mutex<Vec<&'static str>>,
}

impl Fault {
    fn check(&self, operation: &'static str) -> Result<()> {
        self.calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(operation);
        if self.fail == operation {
            Err(PlcError::new(
                ErrorKind::Trust,
                "DO_NOT_LEAK provider details",
            ))
        } else {
            Ok(())
        }
    }
}

impl Backend for Fault {
    fn fill_random(&self, output: &mut [u8]) -> Result<()> {
        self.check("random")?;
        Native.fill_random(output)
    }
    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>> {
        self.check("mac")?;
        if self.bad_mac {
            Ok(vec![0; 31])
        } else {
            Native.hmac(hash, secret, pieces)
        }
    }
    fn verify_hmac(&self, hash: Hash, secret: &[u8], message: &[u8], tag: &[u8]) -> Result<()> {
        self.check("verify")?;
        Native.verify_hmac(hash, secret, message, tag)
    }
    fn encrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        self.check("encrypt")?;
        Native.encrypt(key, iv, bytes)
    }
    fn decrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        self.check("decrypt")?;
        Native.decrypt(key, iv, bytes)
    }
}

fn uniform(error: &PlcError) {
    assert_eq!(error.kind(), ErrorKind::Authentication);
    assert_eq!(
        error.to_string(),
        "OPC UA message security verification failed"
    );
    assert!(error.source().is_none());
}

#[test]
fn opcua_mac_comparison_is_constant_time_by_construction() {
    // Native invokes AWS-LC hmac::verify, not a first-party byte equality operation.
    for hash in [Hash::Sha1, Hash::Sha256] {
        let tag = Native.hmac(hash, b"secret", &[b"mes", b"sage"]).unwrap();
        Native
            .verify_hmac(hash, b"secret", b"message", &tag)
            .unwrap();
        for index in 0..tag.len() {
            let mut bad = tag.clone();
            *bad.get_mut(index).unwrap() ^= 1;
            assert!(
                Native
                    .verify_hmac(hash, b"secret", b"message", &bad)
                    .is_err()
            );
        }
        for length in [0, 1, 19, 20, 21, 31, 32, 33, 64]
            .into_iter()
            .filter(|length| *length != tag.len())
        {
            assert!(
                Native
                    .verify_hmac(hash, b"secret", b"message", &vec![0; length])
                    .is_err()
            );
        }
        assert!(
            Native
                .verify_hmac(hash, b"wrong", b"message", &tag)
                .is_err()
        );
        assert!(
            Native
                .verify_hmac(hash, b"secret", b"changed", &tag)
                .is_err()
        );
    }
}

#[test]
fn opcua_aes_cbc_native_wrappers_match_nist_without_implicit_padding() {
    let input = unhex("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e51");
    let iv: [u8; 16] = unhex("000102030405060708090a0b0c0d0e0f")
        .try_into()
        .unwrap();
    for (key, expected) in [
        (
            "2b7e151628aed2a6abf7158809cf4f3c",
            "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b2",
        ),
        (
            "603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4",
            "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d",
        ),
    ] {
        let key = unhex(key);
        let mut actual = input.clone();
        Native.encrypt(&key, &iv, &mut actual).unwrap();
        assert_eq!(hex(&actual), expected);
        assert_eq!(actual.len(), input.len());
        Native.decrypt(&key, &iv, &mut actual).unwrap();
        assert_eq!(actual, input);
        for length in [1, 15, 17, 31] {
            assert!(Native.encrypt(&key, &iv, &mut vec![0; length]).is_err());
            assert!(Native.decrypt(&key, &iv, &mut vec![0; length]).is_err());
        }
    }
    for length in [0, 1, 15, 17, 24, 31, 33] {
        assert!(Native.encrypt(&vec![0; length], &iv, &mut [0; 16]).is_err());
        assert!(Native.decrypt(&vec![0; length], &iv, &mut [0; 16]).is_err());
    }
}

#[test]
fn opcua_symmetric_chunks_match_independent_python_openssl_vectors() {
    // Python hmac/struct PRF and openssl enc -aes-{128,256}-cbc -nopad; ADR-0040/evidence.
    for policy in POLICIES {
        for (mode, expected) in [
            (
                MessageSecurity::Sign,
                "4d5347463d000000110000001d0000000700000009000000010203040557968e1cc7dd7dff61044bf01da42a0e3c329e0dcc3f2b9087d689f03ec23195",
            ),
            (
                MessageSecurity::SignEncrypt,
                if policy == SecurityPolicy::Aes128Sha256RsaOaep {
                    "4d53474640000000110000001d0000003dde1962eab6a3e9e287441c4f8b296938a4a874afe6541c6f0131706d3198cf544289f32e5c6c192738b179ac45bf48"
                } else {
                    "4d53474640000000110000001d000000b5f1508d4745c899ea14202d0808f20a7ac01e8937d22c1fa1ca7feb7338523c98effd1746954f9bca26c427962ff392"
                },
            ),
        ] {
            let client = endpoint(policy, mode);
            let server = peer(policy, mode);
            let frame = frame(&[1, 2, 3, 4, 5], 29);
            let bytes = client.seal(&Native, &frame, 8192).unwrap();
            assert_eq!(hex(&bytes), expected);
            assert_eq!(server.open(&Native, &unhex(expected), 8192).unwrap(), frame);
            // Direction keys are separate; a client must reject its reflected request.
            uniform(&client.open(&Native, &bytes, 8192).unwrap_err());
            let response = server.seal(&Native, &frame, 8192).unwrap();
            assert_eq!(client.open(&Native, &response, 8192).unwrap(), frame);
        }
    }
}

#[test]
fn opcua_aes_cbc_chunk_round_trips_with_the_policy_key_sizes() {
    for policy in POLICIES {
        for mode in MODES {
            let client = endpoint(policy, mode);
            let server = peer(policy, mode);
            for length in 0..65 {
                for flag in [ChunkType::Intermediate, ChunkType::Final, ChunkType::Abort] {
                    let mut frame = frame(&vec![0x5a; length], 29);
                    frame.chunk = flag;
                    for close in [false, true] {
                        if close {
                            let Body::Message(chunk) = frame.body else {
                                panic!("message");
                            };
                            frame.body = Body::Close(chunk);
                        }
                        let bytes = client.seal(&Native, &frame, 8192).unwrap();
                        assert_eq!(server.open(&Native, &bytes, bytes.len()).unwrap(), frame);
                        if mode == MessageSecurity::SignEncrypt {
                            assert_eq!(
                                bytes.len().checked_sub(16).unwrap().checked_rem(16),
                                Some(0)
                            );
                        } else {
                            assert_eq!(bytes.len(), length.checked_add(56).unwrap());
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn opcua_protected_header_body_mac_and_ciphertext_tampering_share_one_error() {
    for policy in POLICIES {
        for mode in MODES {
            let client = endpoint(policy, mode);
            let server = peer(policy, mode);
            let bytes = client
                .seal(&Native, &frame(b"secret body", 29), 8192)
                .unwrap();
            for index in 0..bytes.len() {
                let mut bad = bytes.to_vec();
                *bad.get_mut(index).unwrap() ^= 1;
                uniform(&server.open(&Native, &bad, 8192).unwrap_err());
            }
            for length in 0..bytes.len() {
                uniform(
                    &server
                        .open(&Native, bytes.get(..length).unwrap(), 8192)
                        .unwrap_err(),
                );
            }
            let mut extra = bytes.to_vec();
            extra.push(0);
            uniform(&server.open(&Native, &extra, 8192).unwrap_err());
        }
    }
}

fn authenticated(issuer: &Symmetric, mut plain: Vec<u8>) -> Vec<u8> {
    let keys = &issuer.current.keys.client;
    let end = plain.len().checked_sub(32).unwrap();
    let tag = Native
        .hmac(
            Hash::Sha256,
            keys.signing.expose(),
            &[plain.get(..end).unwrap()],
        )
        .unwrap();
    plain.get_mut(end..).unwrap().copy_from_slice(&tag);
    Native
        .encrypt(
            keys.encryption.expose(),
            iv(keys).unwrap(),
            plain.get_mut(16..).unwrap(),
        )
        .unwrap();
    plain
}

#[test]
fn opcua_authenticated_padding_is_checked_only_after_mac_verification() {
    let policy = SecurityPolicy::Basic256Sha256;
    let client = endpoint(policy, MessageSecurity::SignEncrypt);
    let server = peer(policy, MessageSecurity::SignEncrypt);
    let bytes = client
        .seal(&Native, &frame(&[1, 2, 3, 4, 5], 29), 8192)
        .unwrap();
    let mut clear = bytes.to_vec();
    let keys = &client.current.keys.client;
    Native
        .decrypt(
            keys.encryption.expose(),
            iv(keys).unwrap(),
            clear.get_mut(16..).unwrap(),
        )
        .unwrap();
    for (index, value) in [(30, 3), (31, 255), (31, 1)] {
        let mut bad = clear.clone();
        *bad.get_mut(index).unwrap() = value;
        let backend = Fault::default();
        uniform(
            &server
                .open(&backend, &authenticated(&client, bad), 8192)
                .unwrap_err(),
        );
        assert_eq!(*backend.calls.lock().unwrap(), ["decrypt", "verify"]);
    }
    let backend = Fault {
        fail: "verify",
        ..Fault::default()
    };
    let bad = authenticated(&client, clear);
    uniform(&server.open(&backend, &bad, 8192).unwrap_err());
    assert_eq!(*backend.calls.lock().unwrap(), ["decrypt", "verify"]);
}

#[test]
fn opcua_padding_makes_the_chunk_a_multiple_of_the_cipher_block() {
    let policy = SecurityPolicy::Basic256Sha256.descriptor();
    for payload in 0usize..64 {
        let plain = payload.checked_add(24).unwrap();
        let count = padding(plain, policy).unwrap();
        assert!((1..=16).contains(&count));
        assert_eq!(
            plain
                .checked_sub(16)
                .unwrap()
                .checked_add(count)
                .unwrap()
                .checked_add(32)
                .unwrap()
                .checked_rem(16),
            Some(0)
        );
    }
    assert_eq!(padding(31, policy).unwrap(), 1);
    for plain in [0, 15, usize::MAX] {
        assert!(padding(plain, policy).is_err());
    }
    assert!(padding(24, Policy { block: 0, ..policy }).is_err());
    assert!(unpad(&[]).is_err());
    assert!(unpad(&[0; 24]).is_err());
    assert!(unpad(&[255; 25]).is_err());
    let mut minimal = vec![0; 24];
    minimal.push(0);
    assert_eq!(unpad(&minimal).unwrap(), 24);
    let mut inconsistent = vec![0; 24];
    inconsistent.extend([0, 1]);
    assert!(unpad(&inconsistent).is_err());
}

#[test]
fn opcua_authenticated_extra_padding_blocks_are_accepted() {
    let client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let server = peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let frame = frame(&[7; 8], 29);
    let mut writer = BoundedWriter::new(320);
    frame.serialize(320, &mut writer).unwrap();
    for _ in 0..256 {
        writer.write_u8(255).unwrap();
    }
    writer.write_bytes(&[0; 32]).unwrap();
    let bytes = authenticated(&client, writer.into_bytes());
    assert_eq!(server.open(&Native, &bytes, 320).unwrap(), frame);
}

#[test]
fn opcua_keys_are_regenerated_after_a_renewal_changes_the_nonce() {
    for changed_client in [false, true] {
        let mut client = Symmetric::new(
            &Native,
            SecurityPolicy::Basic256Sha256,
            MessageSecurity::SignEncrypt,
            17,
            29,
            &[1; 32],
            &[2; 32],
        )
        .unwrap();
        let mut server = Symmetric::new(
            &Native,
            SecurityPolicy::Basic256Sha256,
            MessageSecurity::SignEncrypt,
            17,
            29,
            &[1; 32],
            &[2; 32],
        )
        .unwrap();
        swap(&mut server);
        let old = client.seal(&Native, &frame(b"old", 29), 8192).unwrap();
        let previous_key = client.current.keys.client.signing.expose().clone();
        let (client_nonce, server_nonce) = if changed_client {
            ([3; 32], [2; 32])
        } else {
            ([1; 32], [4; 32])
        };
        client
            .install(&Native, 31, &client_nonce, &server_nonce)
            .unwrap();
        server
            .install(&Native, 31, &client_nonce, &server_nonce)
            .unwrap();
        swap(&mut server);
        assert_eq!(client.token_id(), 31);
        assert_ne!(client.current.keys.client.signing.expose(), &previous_key);
        let fresh = frame(b"new", 31);
        let wire = client.seal(&Native, &fresh, 8192).unwrap();
        assert_eq!(server.open(&Native, &wire, 8192).unwrap(), fresh);
        assert_eq!(server.open(&Native, &old, 8192).unwrap(), frame(b"old", 29));
        server.retire_previous();
        uniform(&server.open(&Native, &old, 8192).unwrap_err());
        assert!(client.seal(&Native, &frame(b"old", 29), 8192).is_err());
        // A third token keeps only its direct predecessor.
        client.install(&Native, 33, &[5; 32], &[6; 32]).unwrap();
        server.install(&Native, 33, &[5; 32], &[6; 32]).unwrap();
        swap(&mut server);
        assert_eq!(server.open(&Native, &wire, 8192).unwrap(), fresh);
        uniform(&client.open(&Native, &old, 8192).unwrap_err());
    }
}

#[test]
fn opcua_failed_token_install_keeps_both_key_sets_and_rejects_active_id_reuse() {
    let mut client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    client.install(&Native, 31, &[1; 32], &[2; 32]).unwrap();
    let before = client.seal(&Native, &frame(b"body", 31), 8192).unwrap();
    let backend = Fault {
        fail: "mac",
        ..Fault::default()
    };
    for id in [0, 29, 31] {
        assert!(client.install(&backend, id, &[3; 32], &[4; 32]).is_err());
    }
    assert_eq!(backend.calls.lock().unwrap().len(), 0);
    assert!(client.install(&backend, 33, &[3; 32], &[4; 32]).is_err());
    assert!(client.install(&Native, 33, &[3; 31], &[4; 32]).is_err());
    assert_eq!(client.token_id(), 31);
    assert_eq!(client.previous.as_ref().unwrap().id, 29);
    assert_eq!(
        client.seal(&Native, &frame(b"body", 31), 8192).unwrap(),
        before
    );
    client.retire_previous();
    client.retire_previous();
    assert!(client.previous.is_none());
}

#[test]
fn opcua_symmetric_modes_policies_and_ids_are_validated_before_derivation() {
    let backend = Fault {
        fail: "mac",
        ..Fault::default()
    };
    for (policy, mode, channel, token) in [
        (SecurityPolicy::None, MessageSecurity::Sign, 17, 29),
        (
            SecurityPolicy::Basic256Sha256,
            MessageSecurity::None,
            17,
            29,
        ),
        (SecurityPolicy::Basic256Sha256, MessageSecurity::Sign, 0, 29),
        (SecurityPolicy::Basic256Sha256, MessageSecurity::Sign, 17, 0),
    ] {
        assert!(
            Symmetric::new(&backend, policy, mode, channel, token, &[1; 32], &[2; 32]).is_err()
        );
    }
    assert_eq!(backend.calls.lock().unwrap().len(), 0);
    assert!(
        Symmetric::new(
            &backend,
            SecurityPolicy::Basic256Sha256,
            MessageSecurity::Sign,
            17,
            29,
            &[1; 32],
            &[2; 32]
        )
        .is_err()
    );
}

#[test]
fn opcua_symmetric_bounds_precede_allocation_and_provider_calls() {
    for mode in MODES {
        let client = endpoint(SecurityPolicy::Basic256Sha256, mode);
        let server = peer(SecurityPolicy::Basic256Sha256, mode);
        let frame = frame(&[1; 7], 29);
        let bytes = client.seal(&Native, &frame, 8192).unwrap();
        let backend = Fault {
            fail: "mac",
            ..Fault::default()
        };
        assert_eq!(
            client
                .seal(&backend, &frame, bytes.len().checked_sub(1).unwrap())
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(
            server
                .open(&backend, &bytes, bytes.len().checked_sub(1).unwrap())
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
        assert_eq!(backend.calls.lock().unwrap().len(), 0);
        assert_eq!(client.seal(&Native, &frame, bytes.len()).unwrap(), bytes);
        assert_eq!(server.open(&Native, &bytes, bytes.len()).unwrap(), frame);
        assert!(client.seal(&Native, &frame, 0).is_err());
    }
}

#[test]
fn opcua_symmetric_backend_failures_have_no_secret_source_or_fallback() {
    for mode in MODES {
        let client = endpoint(SecurityPolicy::Basic256Sha256, mode);
        let server = peer(SecurityPolicy::Basic256Sha256, mode);
        let frame = frame(b"body", 29);
        let bytes = client.seal(&Native, &frame, 8192).unwrap();
        for operation in ["mac", "encrypt"] {
            let backend = Fault {
                fail: operation,
                ..Fault::default()
            };
            if operation == "mac" || mode == MessageSecurity::SignEncrypt {
                uniform(&client.seal(&backend, &frame, 8192).unwrap_err());
            }
        }
        for operation in ["verify", "decrypt"] {
            let backend = Fault {
                fail: operation,
                ..Fault::default()
            };
            if operation == "verify" || mode == MessageSecurity::SignEncrypt {
                uniform(&server.open(&backend, &bytes, 8192).unwrap_err());
            }
        }
        uniform(
            &client
                .seal(
                    &Fault {
                        bad_mac: true,
                        ..Fault::default()
                    },
                    &frame,
                    8192,
                )
                .unwrap_err(),
        );
        let backend = Fault::default();
        let incoming = client.seal(&backend, &frame, 8192).unwrap();
        server.open(&backend, &incoming, 8192).unwrap();
        backend.fill_random(&mut []).unwrap();
    }
    assert!(
        Fault {
            fail: "random",
            ..Fault::default()
        }
        .fill_random(&mut [])
        .is_err()
    );
}

#[test]
fn opcua_symmetric_rejects_local_header_misuse_and_redacts_both_token_key_sets() {
    let mut client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let mut wrong = frame(b"body", 29);
    let Body::Message(chunk) = &mut wrong.body else {
        panic!("message");
    };
    chunk.channel_id = 18;
    assert_eq!(
        client.seal(&Native, &wrong, 8192).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
    let unsupported = Frame::single(Body::Acknowledge(crate::opcua::frame::Acknowledge {
        protocol_version: 0,
        limits: crate::opcua::frame::TransportLimits {
            receive_buffer_size: 0,
            send_buffer_size: 0,
            max_message_size: 0,
            max_chunk_count: 0,
        },
    }));
    assert_eq!(
        client.seal(&Native, &unsupported, 8192).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
    client.install(&Native, 31, &[3; 32], &[4; 32]).unwrap();
    let formatted = format!("{client:?}");
    assert_eq!(formatted.matches("[REDACTED]").count(), 12);
    assert!(!formatted.contains(&hex(client.current.keys.client.signing.expose())));
    client.current.keys.client.iv = crate::Secret::new(vec![0; 15]);
    assert_eq!(
        client
            .seal(&Native, &frame(b"body", 31), 8192)
            .unwrap_err()
            .kind(),
        ErrorKind::InvalidConfig
    );
}

#[test]
fn opcua_plaintext_block_sizes_match_the_documented_values() {
    for policy in POLICIES {
        assert_eq!(policy.descriptor().block, 16);
        assert_eq!(
            policy
                .descriptor()
                .symmetric_sizes(true)
                .max_body(8192, 4)
                .unwrap(),
            8135
        );
        for (modulus, overhead) in [
            (256usize, policy.descriptor().oaep_overhead),
            (512, policy.descriptor().oaep_overhead),
        ] {
            let plain = modulus.checked_sub(overhead).unwrap();
            assert!(plain < modulus);
            assert_eq!(
                overhead,
                if policy == SecurityPolicy::Aes256Sha256RsaPss {
                    66
                } else {
                    42
                }
            );
        }
    }
}

#[test]
fn opcua_failed_install_at_each_derivation_step_preserves_the_active_tokens() {
    use crate::opcua::crypto::tests::Injected;
    let mut client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    client.install(&Native, 31, &[1; 32], &[2; 32]).unwrap();
    let old = client.seal(&Native, &frame(b"body", 31), 8192).unwrap();
    for step in 0..12 {
        assert!(
            client
                .install(&Injected::new(step, None), 33, &[3; 32], &[4; 32])
                .is_err()
        );
        assert_eq!(client.token_id(), 31);
        assert_eq!(client.previous.as_ref().unwrap().id, 29);
        assert_eq!(
            client.seal(&Native, &frame(b"body", 31), 8192).unwrap(),
            old
        );
    }
}

#[test]
fn opcua_retokening_keeps_unchanged_nonce_keys_but_authenticates_the_new_id() {
    let mut client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    let mut server = peer(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    let original = client.current.keys.client.signing.expose().clone();
    let old = client.seal(&Native, &frame(b"body", 29), 8192).unwrap();
    let client_nonce: Vec<u8> = (0..32).collect();
    let server_nonce: Vec<u8> = (32..64).collect();
    client
        .install(&Native, 31, &client_nonce, &server_nonce)
        .unwrap();
    server
        .install(&Native, 31, &client_nonce, &server_nonce)
        .unwrap();
    swap(&mut server);
    assert_eq!(client.current.keys.client.signing.expose(), &original);
    let mut renamed = old.to_vec();
    renamed
        .get_mut(12..16)
        .unwrap()
        .copy_from_slice(&31u32.to_le_bytes());
    uniform(&server.open(&Native, &renamed, 8192).unwrap_err());
    let fresh = client.seal(&Native, &frame(b"body", 31), 8192).unwrap();
    assert_eq!(
        server.open(&Native, &fresh, 8192).unwrap(),
        frame(b"body", 31)
    );
}

#[test]
fn opcua_third_token_install_discards_only_the_oldest_key_set() {
    let mut client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let mut server = peer(SecurityPolicy::Basic256Sha256, MessageSecurity::SignEncrypt);
    let oldest = client.seal(&Native, &frame(b"first", 29), 8192).unwrap();
    client.install(&Native, 31, &[3; 32], &[4; 32]).unwrap();
    server.install(&Native, 31, &[3; 32], &[4; 32]).unwrap();
    swap(&mut server);
    let previous = client.seal(&Native, &frame(b"second", 31), 8192).unwrap();
    client.install(&Native, 33, &[5; 32], &[6; 32]).unwrap();
    server.install(&Native, 33, &[5; 32], &[6; 32]).unwrap();
    swap(&mut server);
    let current = client.seal(&Native, &frame(b"third", 33), 8192).unwrap();
    uniform(&server.open(&Native, &oldest, 8192).unwrap_err());
    assert_eq!(
        server.open(&Native, &previous, 8192).unwrap(),
        frame(b"second", 31)
    );
    assert_eq!(
        server.open(&Native, &current, 8192).unwrap(),
        frame(b"third", 33)
    );
    assert_eq!(server.current.id, 33);
    assert_eq!(server.previous.as_ref().unwrap().id, 31);
}

#[test]
fn opcua_protected_decoder_refuses_other_known_message_types_without_crypto_work() {
    let receiver = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    for code in [b"HEL", b"ACK", b"ERR", b"OPN"] {
        let mut wire = vec![0; 64];
        wire.get_mut(..3).unwrap().copy_from_slice(code);
        *wire.get_mut(3).unwrap() = b'F';
        wire.get_mut(4..8)
            .unwrap()
            .copy_from_slice(&64u32.to_le_bytes());
        let backend = Fault::default();
        uniform(&receiver.open(&backend, &wire, 8192).unwrap_err());
        assert_eq!(backend.calls.lock().unwrap().len(), 0);
    }
}

#[test]
fn opcua_empty_symmetric_body_fits_its_exact_protected_ceiling() {
    for mode in MODES {
        let client = endpoint(SecurityPolicy::Basic256Sha256, mode);
        let server = peer(SecurityPolicy::Basic256Sha256, mode);
        let frame = frame(&[], 29);
        let maximum = if mode == MessageSecurity::Sign {
            56usize
        } else {
            64
        };
        let wire = client.seal(&Native, &frame, maximum).unwrap();
        assert_eq!(wire.len(), maximum);
        assert_eq!(server.open(&Native, &wire, maximum).unwrap(), frame);
        assert_eq!(
            client
                .seal(&Native, &frame, maximum.checked_sub(1).unwrap())
                .unwrap_err()
                .kind(),
            ErrorKind::ResourceLimit
        );
    }
}

#[test]
fn opcua_valid_mac_does_not_authorize_wrong_chunk_type_size_channel_or_token() {
    let client = endpoint(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    let server = peer(SecurityPolicy::Basic256Sha256, MessageSecurity::Sign);
    let bytes = client.seal(&Native, &frame(&[0; 4], 29), 8192).unwrap();
    for (offset, replacement) in [
        (0usize, b"ACK".to_vec()),
        (4, 61u32.to_le_bytes().to_vec()),
        (8, 18u32.to_le_bytes().to_vec()),
        (12, 30u32.to_le_bytes().to_vec()),
    ] {
        let mut authenticated = bytes.to_vec();
        let end = offset.checked_add(replacement.len()).unwrap();
        authenticated
            .get_mut(offset..end)
            .unwrap()
            .copy_from_slice(&replacement);
        let signature = authenticated.len().checked_sub(32).unwrap();
        let tag = Native
            .hmac(
                Hash::Sha256,
                client.current.keys.client.signing.expose(),
                &[authenticated.get(..signature).unwrap()],
            )
            .unwrap();
        authenticated
            .get_mut(signature..)
            .unwrap()
            .copy_from_slice(&tag);
        let backend = Fault::default();
        uniform(&server.open(&backend, &authenticated, 8192).unwrap_err());
        assert_eq!(backend.calls.lock().unwrap().len(), 0);
    }
}
