//! One policy descriptor shared by wire URIs, derivation and chunk sizing.

use super::backend::Hash;
use super::invalid;
use crate::Result;
use crate::opcua::chunk::SecuritySizes;
use crate::opcua::options::SecurityPolicy;

#[derive(Debug, Clone, Copy)]
pub(in crate::opcua) struct Policy {
    pub(in crate::opcua) uri: &'static str,
    pub(in crate::opcua) signature_uri: &'static str,
    pub(in crate::opcua) encryption_uri: &'static str,
    pub(in crate::opcua) hash: Hash,
    pub(in crate::opcua) signature: usize,
    pub(in crate::opcua) signing_key: usize,
    pub(in crate::opcua) encryption_key: usize,
    pub(in crate::opcua) block: usize,
    pub(in crate::opcua) nonce: usize,
    pub(in crate::opcua) oaep_overhead: usize,
}

impl SecurityPolicy {
    pub(in crate::opcua) fn oaep(self) -> Result<&'static aws_lc_rs::rsa::OaepAlgorithm> {
        match self {
            Self::None => Err(invalid()),
            Self::Basic256Sha256 | Self::Aes128Sha256RsaOaep => {
                Ok(&aws_lc_rs::rsa::OAEP_SHA1_MGF1SHA1)
            }
            Self::Aes256Sha256RsaPss => Ok(&aws_lc_rs::rsa::OAEP_SHA256_MGF1SHA256),
        }
    }

    pub(in crate::opcua) fn encoding(
        self,
    ) -> Result<&'static dyn aws_lc_rs::signature::RsaEncoding> {
        match self {
            Self::None => Err(invalid()),
            Self::Basic256Sha256 | Self::Aes128Sha256RsaOaep => {
                Ok(&aws_lc_rs::signature::RSA_PKCS1_SHA256)
            }
            Self::Aes256Sha256RsaPss => Ok(&aws_lc_rs::signature::RSA_PSS_SHA256),
        }
    }

    pub(in crate::opcua) const fn descriptor(self) -> Policy {
        let mut policy = Policy {
            uri: "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256",
            signature_uri: "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256",
            encryption_uri: "http://www.w3.org/2001/04/xmlenc#rsa-oaep",
            hash: Hash::Sha256,
            signature: 32,
            signing_key: 32,
            encryption_key: 32,
            block: 16,
            nonce: 32,
            oaep_overhead: 42,
        };
        match self {
            Self::None => {
                policy.uri = "http://opcfoundation.org/UA/SecurityPolicy#None";
                policy.signature_uri = "";
                policy.encryption_uri = "";
                policy.signature = 0;
                policy.signing_key = 0;
                policy.encryption_key = 0;
                policy.block = 1;
                policy.nonce = 0;
                policy.oaep_overhead = 0;
            }
            Self::Basic256Sha256 => {}
            Self::Aes128Sha256RsaOaep => {
                policy.uri = "http://opcfoundation.org/UA/SecurityPolicy#Aes128_Sha256_RsaOaep";
                policy.encryption_key = 16;
            }
            Self::Aes256Sha256RsaPss => {
                policy.uri = "http://opcfoundation.org/UA/SecurityPolicy#Aes256_Sha256_RsaPss";
                policy.signature_uri = "http://opcfoundation.org/UA/security/rsa-pss-sha2-256";
                policy.encryption_uri = "http://opcfoundation.org/UA/security/rsa-oaep-sha2-256";
                policy.oaep_overhead = 66;
            }
        }
        policy
    }
}

impl Policy {
    /// RSA2048 is native-tested for all modern policies; other boundary sizes remain UNVERIFIED.
    pub(in crate::opcua) fn accepts_rsa_key(self, bytes: usize) -> bool {
        self.nonce != 0 && (256..=512).contains(&bytes)
    }
    pub(in crate::opcua) fn key_length(self) -> Result<usize> {
        self.signing_key
            .checked_add(self.encryption_key)
            .and_then(|length| length.checked_add(self.block))
            .ok_or_else(invalid)
    }

    pub(in crate::opcua) const fn symmetric_sizes(self, encrypt: bool) -> SecuritySizes {
        if self.nonce == 0 {
            SecuritySizes::NONE
        } else if encrypt {
            SecuritySizes::new(self.signature, self.block, self.block, 1)
        } else {
            SecuritySizes::new(self.signature, 1, 1, 0)
        }
    }

    pub(in crate::opcua) fn asymmetric_sizes(
        self,
        signing_modulus: usize,
        encryption_modulus: usize,
    ) -> Result<SecuritySizes> {
        if self.nonce == 0 {
            return Ok(SecuritySizes::NONE);
        }
        // Accepted RSA policies require at least a 2048-bit key; the maximum is 4096 bits.
        if !self.accepts_rsa_key(signing_modulus) || !self.accepts_rsa_key(encryption_modulus) {
            return Err(invalid());
        }
        let plain = encryption_modulus
            .checked_sub(self.oaep_overhead)
            .ok_or_else(invalid)?;
        let padding = if encryption_modulus > 256 { 2 } else { 1 };
        Ok(SecuritySizes::new(
            signing_modulus,
            plain,
            encryption_modulus,
            padding,
        ))
    }
}
