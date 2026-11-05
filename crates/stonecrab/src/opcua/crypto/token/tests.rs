use super::*;
use crate::opcua::pki::{self, CertSpec};
use aws_lc_rs::rsa::{
    OAEP_SHA1_MGF1SHA1, OAEP_SHA256_MGF1SHA256, OaepPrivateDecryptingKey, PrivateDecryptingKey,
};

#[test]
fn opcua_user_name_token_encrypts_password_and_server_nonce_with_the_token_policy() {
    let pair = pki::key_pair();
    let certificate = pki::certificate(&CertSpec::application("server", "server", 1), &pair, &pair);
    let key =
        OaepPrivateDecryptingKey::new(PrivateDecryptingKey::from_pkcs8(pki::pkcs8_2048()).unwrap())
            .unwrap();
    for policy in [
        SecurityPolicy::Basic256Sha256,
        SecurityPolicy::Aes128Sha256RsaOaep,
        SecurityPolicy::Aes256Sha256RsaPss,
    ] {
        for nonce in [vec![0x55; 32], vec![0x55; 128]] {
            for password in ["", "пароль\0secret", &"x".repeat(64)] {
                let encrypted = encrypt(policy, &certificate, password, &nonce).unwrap();
                assert_ne!(
                    encrypted,
                    encrypt(policy, &certificate, password, &nonce).unwrap()
                );
                let algorithm = if policy == SecurityPolicy::Aes256Sha256RsaPss {
                    &OAEP_SHA256_MGF1SHA256
                } else {
                    &OAEP_SHA1_MGF1SHA1
                };
                let mut plain = Vec::new();
                let mut block = vec![0; key.min_output_size()];
                for part in encrypted.as_chunks::<256>().0 {
                    plain
                        .extend_from_slice(key.decrypt(algorithm, part, &mut block, None).unwrap());
                }
                let length = u32::try_from(password.len().saturating_add(nonce.len())).unwrap();
                assert_eq!(
                    plain,
                    [
                        length.to_le_bytes().as_slice(),
                        password.as_bytes(),
                        nonce.as_slice()
                    ]
                    .concat()
                );
            }
        }
    }
}

#[test]
fn opcua_identity_encryption_refuses_none_invalid_nonce_key_and_extended_passwords() {
    let pair = pki::key_pair();
    let certificate = pki::certificate(&CertSpec::application("server", "server", 1), &pair, &pair);
    assert!(SecurityPolicy::None.encoding().is_err());
    assert!(encrypt(SecurityPolicy::None, &certificate, "p", &[1; 32]).is_err());
    assert_eq!(
        encrypt(
            SecurityPolicy::Basic256Sha256,
            &certificate,
            &"x".repeat(65),
            &[1; 32]
        )
        .unwrap_err()
        .kind(),
        ErrorKind::ResourceLimit
    );
    for nonce in [vec![], vec![1; 31], vec![0; 32], vec![1; 129]] {
        assert_eq!(
            encrypt(SecurityPolicy::Basic256Sha256, &certificate, "p", &nonce)
                .unwrap_err()
                .kind(),
            ErrorKind::Authentication
        );
    }
    assert_eq!(
        encrypt(SecurityPolicy::Basic256Sha256, &[], "p", &[1; 32])
            .unwrap_err()
            .kind(),
        ErrorKind::Trust
    );
}
