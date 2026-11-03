//! Bounded `P_HASH` expansion (OPC UA Part 6 §6.7.6, RFC 5246 §5).

use super::backend::{Backend, Hash};
use super::{Policy, invalid};
use crate::opcua::options::SecurityPolicy;
use crate::{Result, Secret};

const MAX_EXPANSION: usize = 256;

#[derive(Debug)]
pub(in crate::opcua) struct Keys {
    pub(in crate::opcua) signing: Secret<Vec<u8>>,
    pub(in crate::opcua) encryption: Secret<Vec<u8>>,
    pub(in crate::opcua) iv: Secret<Vec<u8>>,
}

#[derive(Debug)]
pub(in crate::opcua) struct KeyPair {
    pub(in crate::opcua) client: Keys,
    pub(in crate::opcua) server: Keys,
}

pub(in crate::opcua) fn expand(
    backend: &dyn Backend,
    hash: Hash,
    secret: &[u8],
    seed: &[u8],
    length: usize,
) -> Result<Secret<Vec<u8>>> {
    if length > MAX_EXPANSION {
        return Err(invalid());
    }
    let mut output = Vec::with_capacity(length);
    let mut a = Secret::new(seed.to_vec());
    for _ in 0..length.div_ceil(hash.length()) {
        a = mac(backend, hash, secret, &[a.expose()])?;
        let block = mac(backend, hash, secret, &[a.expose(), seed])?;
        let remaining = length.saturating_sub(output.len());
        output.extend(block.expose().iter().take(remaining));
    }
    Ok(Secret::new(output))
}

fn mac(
    backend: &dyn Backend,
    hash: Hash,
    secret: &[u8],
    pieces: &[&[u8]],
) -> Result<Secret<Vec<u8>>> {
    let output = backend.hmac(hash, secret, pieces)?;
    if output.len() != hash.length() {
        return Err(invalid());
    }
    Ok(Secret::new(output))
}

pub(in crate::opcua) fn derive_keys(
    backend: &dyn Backend,
    policy: SecurityPolicy,
    client_nonce: &[u8],
    server_nonce: &[u8],
) -> Result<KeyPair> {
    let policy = policy.descriptor();
    if policy.nonce == 0 || client_nonce.len() != policy.nonce || server_nonce.len() != policy.nonce
    {
        return Err(invalid());
    }
    Ok(KeyPair {
        client: direction(backend, policy, server_nonce, client_nonce)?,
        server: direction(backend, policy, client_nonce, server_nonce)?,
    })
}

fn direction(backend: &dyn Backend, policy: Policy, secret: &[u8], seed: &[u8]) -> Result<Keys> {
    let material = expand(backend, policy.hash, secret, seed, policy.key_length()?)?;
    let mut bytes = material.expose().iter().copied();
    let signing = bytes.by_ref().take(policy.signing_key).collect();
    let encryption = bytes.by_ref().take(policy.encryption_key).collect();
    let iv = bytes.collect();
    Ok(Keys {
        signing: Secret::new(signing),
        encryption: Secret::new(encryption),
        iv: Secret::new(iv),
    })
}
