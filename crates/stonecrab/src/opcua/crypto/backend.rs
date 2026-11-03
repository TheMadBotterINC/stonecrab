//! Effects used by derivation; failures never substitute deterministic bytes.

use aws_lc_rs::cipher::{
    self, AES_128, AES_256, DecryptingKey, DecryptionContext, EncryptingKey, EncryptionContext,
    UnboundCipherKey,
};
use aws_lc_rs::hmac;
use aws_lc_rs::iv::FixedLength;

use super::invalid;
use crate::Result;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::opcua) enum Hash {
    Sha1,
    Sha256,
}

impl Hash {
    pub(in crate::opcua) const fn length(self) -> usize {
        match self {
            Self::Sha1 => 20,
            Self::Sha256 => 32,
        }
    }
    const fn algorithm(self) -> hmac::Algorithm {
        match self {
            Self::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Self::Sha256 => hmac::HMAC_SHA256,
        }
    }
}

pub(in crate::opcua) trait Backend: Send + Sync {
    fn fill_random(&self, output: &mut [u8]) -> Result<()>;
    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>>;
    fn verify_hmac(&self, hash: Hash, secret: &[u8], message: &[u8], tag: &[u8]) -> Result<()>;
    fn encrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()>;
    fn decrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()>;
}

#[derive(Debug)]
pub(in crate::opcua) struct Native;

impl Backend for Native {
    fn fill_random(&self, output: &mut [u8]) -> Result<()> {
        entropy_result(getrandom::fill(output))
    }

    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>> {
        let key = hmac::Key::new(hash.algorithm(), secret);
        let mut context = hmac::Context::with_key(&key);
        for piece in pieces {
            context.update(piece);
        }
        Ok(context.sign().as_ref().to_vec())
    }
    fn verify_hmac(&self, hash: Hash, secret: &[u8], message: &[u8], tag: &[u8]) -> Result<()> {
        hmac::verify(&hmac::Key::new(hash.algorithm(), secret), message, tag)
            .map_err(primitive_error)
    }

    fn encrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        let key = EncryptingKey::cbc(cipher_key(key)?).map_err(primitive_error)?;
        key.less_safe_encrypt(bytes, EncryptionContext::Iv128(FixedLength::from(*iv)))
            .map_err(primitive_error)?;
        Ok(())
    }

    fn decrypt(&self, key: &[u8], iv: &[u8; 16], bytes: &mut [u8]) -> Result<()> {
        let key = DecryptingKey::cbc(cipher_key(key)?).map_err(primitive_error)?;
        let _plain = key
            .decrypt(bytes, DecryptionContext::Iv128(FixedLength::from(*iv)))
            .map_err(primitive_error)?;
        Ok(())
    }
}

fn cipher_key(key: &[u8]) -> Result<UnboundCipherKey> {
    let algorithm: &cipher::Algorithm = match key.len() {
        16 => &AES_128,
        32 => &AES_256,
        _ => return Err(invalid()),
    };
    UnboundCipherKey::new(algorithm, key).map_err(primitive_error)
}

fn primitive_error(_error: aws_lc_rs::error::Unspecified) -> crate::PlcError {
    invalid()
}

fn entropy_result(result: std::result::Result<(), getrandom::Error>) -> Result<()> {
    result.map_err(|_error| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcua_nonce_generation_failure_has_no_fallback() {
        assert_eq!(
            entropy_result(Err(getrandom::Error::UNSUPPORTED))
                .unwrap_err()
                .kind(),
            crate::ErrorKind::InvalidConfig
        );
        let mut nonce = [0; 32];
        Native.fill_random(&mut nonce).unwrap();
        assert_ne!(nonce, [0; 32]);
    }
}
