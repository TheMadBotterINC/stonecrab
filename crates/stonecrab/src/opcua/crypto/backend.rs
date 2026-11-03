//! Effects used by derivation; failures never substitute deterministic bytes.

use aws_lc_rs::hmac;

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
}

pub(in crate::opcua) trait Backend: Send + Sync {
    fn fill_random(&self, output: &mut [u8]) -> Result<()>;
    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>>;
}

#[derive(Debug)]
pub(in crate::opcua) struct Native;

impl Backend for Native {
    fn fill_random(&self, output: &mut [u8]) -> Result<()> {
        entropy_result(getrandom::fill(output))
    }

    fn hmac(&self, hash: Hash, secret: &[u8], pieces: &[&[u8]]) -> Result<Vec<u8>> {
        let algorithm = match hash {
            Hash::Sha1 => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
            Hash::Sha256 => hmac::HMAC_SHA256,
        };
        let key = hmac::Key::new(algorithm, secret);
        let mut context = hmac::Context::with_key(&key);
        for piece in pieces {
            context.update(piece);
        }
        Ok(context.sign().as_ref().to_vec())
    }
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
