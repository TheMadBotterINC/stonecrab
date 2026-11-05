use std::sync::atomic::{AtomicUsize, Ordering};

use super::derive::expand;
use super::*;

const POLICIES: [SecurityPolicy; 3] = [
    SecurityPolicy::Basic256Sha256,
    SecurityPolicy::Aes128Sha256RsaOaep,
    SecurityPolicy::Aes256Sha256RsaPss,
];

// covers: OPCUA-POLICY-AES128-SHA256-RSAOAEP
#[test]
fn opcua_aes128_sha256_rsaoaep_derives_the_documented_key_sizes() {
    assert_aes_keys(SecurityPolicy::Aes128Sha256RsaOaep, 16);
}

// covers: OPCUA-POLICY-AES256-SHA256-RSAPSS
#[test]
fn opcua_aes256_sha256_rsapss_derives_the_documented_key_sizes() {
    assert_aes_keys(SecurityPolicy::Aes256Sha256RsaPss, 32);
}

fn assert_aes_keys(policy: SecurityPolicy, encryption_bytes: usize) {
    let client: Vec<u8> = (0..32).collect();
    let server: Vec<u8> = (32..64).collect();
    let keys = derive_keys(&Native, policy, &client, &server).unwrap();
    // Independent Python hmac/hashlib P_SHA256 streams, server/client nonce as secret/seed
    // for the client direction, reversed for the server direction (same vector as S101).
    let client_stream = concat!(
        "dd585db0c102dd1a4c1ed4dd195606dec3f7a1c789afca78f9479ed3a5d668af",
        "ce49cb8f1c65a827f412c48e71c9f9cb3b5c2ee2fc2e4b3bd46d4098b5e45475",
        "a77832c6215b6e7ab85f2e668be7aeff"
    );
    let server_stream = concat!(
        "b72593c43fee5fafa0256cd6bb904ff40c066a225db95f66dd744e20858a2220",
        "ddf75067e3d76ac714c08e24eabd85ff425d7f5fb25e6e083b94b174e29db89b",
        "c513e9172274d5ed54e52a3552901ae0"
    );
    for (keys, expected) in [(&keys.client, client_stream), (&keys.server, server_stream)] {
        assert_eq!(
            (
                keys.signing.expose().len(),
                keys.encryption.expose().len(),
                keys.iv.expose().len()
            ),
            (32, encryption_bytes, 16)
        );
        let joined = [
            keys.signing.expose().as_slice(),
            keys.encryption.expose(),
            keys.iv.expose(),
        ]
        .concat();
        assert_eq!(
            hex(&joined),
            expected
                .get(..joined.len().checked_mul(2).unwrap())
                .unwrap()
        );
    }
}

#[test]
fn opcua_aes256_sha256_rsapss_plaintext_block_is_key_bytes_minus_66() {
    let descriptor = SecurityPolicy::Aes256Sha256RsaPss.descriptor();
    assert_eq!(descriptor.oaep_overhead, 66);
    assert_eq!(
        descriptor.signature_uri,
        "http://opcfoundation.org/UA/security/rsa-pss-sha2-256"
    );
    assert_eq!(
        descriptor.encryption_uri,
        "http://opcfoundation.org/UA/security/rsa-oaep-sha2-256"
    );
    let sizes = descriptor.asymmetric_sizes(256, 256).unwrap();
    assert!(sizes.max_body(527, 4).is_err());
    assert_eq!(sizes.max_body(528, 4).unwrap(), 115);
    assert_eq!(sizes.max_body(784, 4).unwrap(), 305);
    assert_eq!(
        descriptor
            .asymmetric_sizes(512, 512)
            .unwrap()
            .max_body(1040, 4)
            .unwrap(),
        370
    );
}

pub(in crate::opcua) struct Injected {
    calls: AtomicUsize,
    fail_at: usize,
    digest_length: Option<usize>,
}

impl Injected {
    pub(in crate::opcua) fn new(fail_at: usize, digest_length: Option<usize>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail_at,
            digest_length,
        }
    }

    fn call(&self) -> Result<()> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == self.fail_at {
            Err(invalid())
        } else {
            Ok(())
        }
    }
}

impl Backend for Injected {
    fn fill_random(&self, output: &mut [u8]) -> Result<()> {
        self.call()?;
        output.fill(0xa5);
        Ok(())
    }

    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>> {
        self.call()?;
        self.digest_length.map_or_else(
            || Native.hmac(hash, secret, pieces),
            |length| Ok(vec![0; length]),
        )
    }
    fn verify_hmac(&self, hash: Hash, secret: &[u8], message: &[u8], tag: &[u8]) -> Result<()> {
        self.call()?;
        Native.verify_hmac(hash, secret, message, tag)
    }
    fn encrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        self.call()?;
        Native.encrypt(key, iv, bytes)
    }
    fn decrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        self.call()?;
        Native.decrypt(key, iv, bytes)
    }
}

pub(super) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut output, byte| {
        write!(output, "{byte:02x}").unwrap();
        output
    })
}

pub(super) fn unhex(text: &str) -> Vec<u8> {
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}

#[test]
fn opcua_key_derivation_matches_the_published_vector_for_basic256sha256() {
    // NIST master-secret vector, OpenSSL 3.5.4 evpkdf_tls12_prf.txt (SHA-256).
    let secret = unhex(
        "f8938ecc9edebc5030c0c6a441e213cd24e6f770a50dda07876f8d55da062bcadb386b411fd4fe4313a604fce6c17fbc",
    );
    let mut seed = b"master secret".to_vec();
    seed.extend(unhex(
        "36c129d01a3200894b9179faac589d9835d58775f9b5ea3587cb8fd0364cae8c",
    ));
    seed.extend(unhex(
        "f6c9575ed7ddd73e1f7d16eca115415812a43c2b747daaaae043abfb50053fce",
    ));
    assert_eq!(
        hex(expand(&Native, Hash::Sha256, &secret, &seed, 48)
            .unwrap()
            .expose()),
        "202c88c00f84a17a20027079604787461176455539e705be730890602c289a5001e34eeb3a043e5d52a65e66125188bf"
    );
}

#[test]
fn opcua_p_sha1_and_sha256_match_independent_multiblock_vectors() {
    // Python stdlib hmac/hashlib, secret=b"secret", seed=b"seed", first 104 bytes.
    for (hash, expected) in [
        (
            Hash::Sha1,
            concat!(
                "5d55432bdfe9b93a813acc932adf0f297815d1c83166a983ea042d410665dfa1",
                "9268d99774fd6f3fc5433f0465e765c49b8590d51e1e9988be2167fd8456dcb",
                "37305f0284b8b5879706d1b3e6fc73edcd7f623d5f27478f97dad60aeebc1d4",
                "f0641770f88d2e437d"
            ),
        ),
        (
            Hash::Sha256,
            concat!(
                "8e4d932530d765a0aae974c304735ecc1202a819f80adbd5ad09c1a34fc06918",
                "e3d07795214d94c6a1976caea5a0b644c5b04d1ad3e09c6111c37afc00df0b6",
                "d134279f973813359ad8be59625d3ba60e38ad6d93a8315525a6c6e5db141537",
                "2732e0b54cf3cf9bb"
            ),
        ),
    ] {
        assert_eq!(
            hex(expand(&Native, hash, b"secret", b"seed", 104)
                .unwrap()
                .expose()),
            expected
        );
    }
}

#[test]
fn opcua_key_derivation_yields_distinct_keys_per_direction() {
    let client: Vec<u8> = (0..32).collect();
    let server: Vec<u8> = (32..64).collect();
    let keys = derive_keys(&Native, SecurityPolicy::Basic256Sha256, &client, &server).unwrap();
    assert_eq!(
        hex(keys.client.signing.expose()),
        "dd585db0c102dd1a4c1ed4dd195606dec3f7a1c789afca78f9479ed3a5d668af"
    );
    assert_eq!(
        hex(keys.client.encryption.expose()),
        "ce49cb8f1c65a827f412c48e71c9f9cb3b5c2ee2fc2e4b3bd46d4098b5e45475"
    );
    assert_eq!(
        hex(keys.client.iv.expose()),
        "a77832c6215b6e7ab85f2e668be7aeff"
    );
    assert_eq!(
        hex(keys.server.signing.expose()),
        "b72593c43fee5fafa0256cd6bb904ff40c066a225db95f66dd744e20858a2220"
    );
    assert_eq!(
        hex(keys.server.encryption.expose()),
        "ddf75067e3d76ac714c08e24eabd85ff425d7f5fb25e6e083b94b174e29db89b"
    );
    assert_eq!(
        hex(keys.server.iv.expose()),
        "c513e9172274d5ed54e52a3552901ae0"
    );
    let reversed = derive_keys(&Native, SecurityPolicy::Basic256Sha256, &server, &client).unwrap();
    assert_eq!(
        keys.client.signing.expose(),
        reversed.server.signing.expose()
    );
    assert_eq!(
        keys.server.encryption.expose(),
        reversed.client.encryption.expose()
    );
    assert_ne!(keys.client.iv.expose(), keys.server.iv.expose());
}

#[test]
fn opcua_key_derivation_uses_the_policy_hmac_and_sizes() {
    for policy in POLICIES {
        let descriptor = policy.descriptor();
        let keys = derive_keys(&Native, policy, &[1; 32], &[2; 32]).unwrap();
        let expected = expand(
            &Native,
            Hash::Sha256,
            &[2; 32],
            &[1; 32],
            descriptor.key_length().unwrap(),
        )
        .unwrap();
        let joined = [
            keys.client.signing.expose().as_slice(),
            keys.client.encryption.expose(),
            keys.client.iv.expose(),
        ]
        .concat();
        assert_eq!(&joined, expected.expose());
        assert_eq!(keys.client.signing.expose().len(), 32);
        assert_eq!(
            keys.client.encryption.expose().len(),
            if policy == SecurityPolicy::Aes128Sha256RsaOaep {
                16
            } else {
                32
            }
        );
        assert_eq!(keys.client.iv.expose().len(), 16);
        assert_eq!(keys.server.signing.expose().len(), 32);
        assert_eq!(
            keys.server.encryption.expose().len(),
            descriptor.encryption_key
        );
        assert_eq!(keys.server.iv.expose().len(), 16);
    }
}

#[test]
fn opcua_open_secure_channel_nonce_has_the_policy_length() {
    let backend = Injected::new(usize::MAX, None);
    assert_eq!(
        channel_nonce(&backend, SecurityPolicy::None)
            .unwrap()
            .as_bytes(),
        None
    );
    assert_eq!(backend.calls.load(Ordering::SeqCst), 0);
    for policy in POLICIES {
        let nonce = channel_nonce(&backend, policy).unwrap();
        assert_eq!(nonce.as_bytes().unwrap().len(), policy.descriptor().nonce);
        assert_eq!(nonce.as_bytes().unwrap(), &[0xa5; 32]);
    }
    assert_eq!(backend.calls.load(Ordering::SeqCst), 3);
}

#[test]
fn opcua_create_session_nonce_is_32_bytes() {
    assert_eq!(
        session_nonce(&Injected::new(usize::MAX, None)).unwrap(),
        [0xa5; 32]
    );
}

#[test]
fn opcua_nonces_come_from_the_injected_random_source() {
    for policy in POLICIES {
        assert_eq!(
            channel_nonce(&Injected::new(0, None), policy)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
    }
    assert_eq!(
        session_nonce(&Injected::new(0, None)).unwrap_err().kind(),
        ErrorKind::InvalidConfig
    );
}

#[test]
fn opcua_derivation_is_bounded_and_refuses_malformed_mac_output() {
    let unused = Injected::new(0, None);
    assert_eq!(
        expand(&unused, Hash::Sha256, b"secret", b"seed", 0)
            .unwrap()
            .expose()
            .as_slice(),
        [0u8; 0]
    );
    assert!(expand(&unused, Hash::Sha256, b"secret", b"seed", 257).is_err());
    assert_eq!(unused.calls.load(Ordering::SeqCst), 0);
    for hash in [Hash::Sha1, Hash::Sha256] {
        assert_eq!(
            expand(&Native, hash, b"", b"", 256).unwrap().expose().len(),
            256
        );
        for length in [0, 19, 21, 31, 33] {
            assert!(
                expand(
                    &Injected::new(usize::MAX, Some(length)),
                    hash,
                    b"secret",
                    b"seed",
                    1
                )
                .is_err()
            );
        }
    }
    // Six HMAC calls per direction for the 80-byte schedule; no partial result escapes.
    for step in 0..12 {
        let backend = Injected::new(step, None);
        assert!(derive_keys(&backend, SecurityPolicy::Basic256Sha256, &[1; 32], &[2; 32]).is_err());
        assert_eq!(
            backend.calls.load(Ordering::SeqCst),
            step.checked_add(1).unwrap()
        );
    }
    for policy in POLICIES.into_iter().chain([SecurityPolicy::None]) {
        for length in [0, 31, 33, 256] {
            assert!(derive_keys(&unused, policy, &vec![1; length], &[2; 32]).is_err());
            assert!(derive_keys(&unused, policy, &[1; 32], &vec![2; length]).is_err());
        }
    }
}

#[test]
fn opcua_key_formatting_is_redacted() {
    let keys = derive_keys(&Native, SecurityPolicy::Basic256Sha256, &[1; 32], &[2; 32]).unwrap();
    let debug = format!("{keys:?}");
    assert_eq!(debug.matches("[REDACTED]").count(), 6);
    assert!(!debug.contains(&hex(keys.client.signing.expose())));
}

#[test]
fn opcua_policy_descriptors_drive_one_chunk_size_formula() {
    let none = SecurityPolicy::None.descriptor();
    assert_eq!(
        (
            none.signature,
            none.signing_key,
            none.encryption_key,
            none.block,
            none.nonce,
            none.oaep_overhead
        ),
        (0, 0, 0, 1, 0, 0)
    );
    assert_eq!((none.signature_uri, none.encryption_uri), ("", ""));
    assert_eq!(none.symmetric_sizes(true).max_body(8192, 4).unwrap(), 8168);
    assert_eq!(none.symmetric_sizes(false).max_body(8192, 4).unwrap(), 8168);
    assert_eq!(
        none.asymmetric_sizes(0, 0)
            .unwrap()
            .max_body(8192, 4)
            .unwrap(),
        8168
    );
    for policy in POLICIES {
        let descriptor = policy.descriptor();
        assert_eq!(descriptor.hash, Hash::Sha256);
        assert_eq!(descriptor.signature, 32);
        assert_eq!(
            descriptor.symmetric_sizes(true).max_body(8192, 4).unwrap(),
            8135
        );
        assert_eq!(
            descriptor.symmetric_sizes(false).max_body(8192, 4).unwrap(),
            8136
        );
        for modulus in [256usize, 257, 298, 299, 322, 323, 384, 512] {
            let plain = modulus.checked_sub(descriptor.oaep_overhead).unwrap();
            let padding = if modulus > 256 { 2 } else { 1 };
            let expected = 8121usize
                .checked_div(modulus)
                .unwrap()
                .checked_mul(plain)
                .unwrap()
                .checked_sub(8)
                .unwrap()
                .checked_sub(modulus)
                .unwrap()
                .checked_sub(padding)
                .unwrap();
            assert_eq!(
                descriptor
                    .asymmetric_sizes(modulus, modulus)
                    .unwrap()
                    .max_body(8192, 59)
                    .unwrap(),
                expected
            );
        }
        for modulus in [0, 255, 513, usize::MAX] {
            assert!(descriptor.asymmetric_sizes(256, modulus).is_err());
            assert!(descriptor.asymmetric_sizes(modulus, 256).is_err());
        }
        // Signature size follows the sender key; cipher blocks follow the receiver key.
        assert_eq!(
            descriptor
                .asymmetric_sizes(512, 256)
                .unwrap()
                .max_body(8192, 59)
                .unwrap(),
            descriptor
                .asymmetric_sizes(256, 256)
                .unwrap()
                .max_body(8192, 59)
                .unwrap()
                .checked_sub(256)
                .unwrap()
        );
        if policy == SecurityPolicy::Aes256Sha256RsaPss {
            assert_eq!(
                descriptor.signature_uri,
                "http://opcfoundation.org/UA/security/rsa-pss-sha2-256"
            );
            assert_eq!(
                descriptor.encryption_uri,
                "http://opcfoundation.org/UA/security/rsa-oaep-sha2-256"
            );
            assert_eq!(descriptor.oaep_overhead, 66);
        } else {
            assert_eq!(
                descriptor.signature_uri,
                "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256"
            );
            assert_eq!(
                descriptor.encryption_uri,
                "http://www.w3.org/2001/04/xmlenc#rsa-oaep"
            );
            assert_eq!(descriptor.oaep_overhead, 42);
        }
    }
    for (signing_key, encryption_key, block) in [(usize::MAX, 1, 0), (1, 0, usize::MAX)] {
        assert!(
            Policy {
                signing_key,
                encryption_key,
                block,
                ..none
            }
            .key_length()
            .is_err()
        );
    }
}

mod native;

#[test]
fn injected_backend_symmetric_methods_preserve_native_results_and_failures() {
    let backend = Injected::new(usize::MAX, None);
    let tag = backend
        .hmac(Hash::Sha256, b"secret", &[b"message"])
        .unwrap();
    backend
        .verify_hmac(Hash::Sha256, b"secret", b"message", &tag)
        .unwrap();
    let mut block = [0; 16];
    backend.encrypt(&[1; 16], &[2; 16], &mut block).unwrap();
    backend.decrypt(&[1; 16], &[2; 16], &mut block).unwrap();
    assert_eq!(block, [0; 16]);
    for operation in 0..3 {
        let failing = Injected::new(0, None);
        let result = match operation {
            0 => failing.verify_hmac(Hash::Sha256, b"secret", b"message", &tag),
            1 => failing.encrypt(&[1; 16], &[2; 16], &mut block),
            _ => failing.decrypt(&[1; 16], &[2; 16], &mut block),
        };
        assert!(result.is_err());
    }
}
