//! Bounded legacy RSA identity secrets; trust and certificate binding precede this call.

use aws_lc_rs::rsa::OaepPublicEncryptingKey;
use rustls_pki_types::CertificateDer;

use super::{certificate, invalid};
use crate::opcua::options::SecurityPolicy;
use crate::{ErrorKind, PlcError, Result, Secret};

fn limited() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "OPC UA encrypted passwords above 64 UTF-8 bytes require the unsupported extended secret format",
    )
}

pub(in crate::opcua) fn encrypt(
    policy: SecurityPolicy,
    certificate: &[u8],
    password: &str,
    nonce: &[u8],
) -> Result<Vec<u8>> {
    let algorithm = policy.oaep()?;
    if password.len() > 64 {
        return Err(limited());
    }
    if !(32..=128).contains(&nonce.len()) || !nonce.iter().any(|byte| *byte != 0) {
        return Err(PlcError::new(
            ErrorKind::Authentication,
            "OPC UA invalid user token server nonce",
        ));
    }
    let length = u32::try_from(password.len().saturating_add(nonce.len()))
        .ok()
        .ok_or_else(limited)?;
    let secret =
        Secret::new([length.to_le_bytes().as_slice(), password.as_bytes(), nonce].concat());
    let key = OaepPublicEncryptingKey::new(certificate::server_key(
        &CertificateDer::from(certificate),
        policy,
    )?)
    .ok()
    .ok_or_else(invalid)?;
    let mut out = Vec::new();
    let mut block = vec![0; key.ciphertext_size()];
    for plain in secret.expose().chunks(key.max_plaintext_size(algorithm)) {
        out.extend_from_slice(
            key.encrypt(algorithm, plain, &mut block, None)
                .ok()
                .ok_or_else(invalid)?,
        );
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
