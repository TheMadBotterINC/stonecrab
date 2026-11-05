//! Separate credential resolution and identity token construction (ADR-0037).
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-AUTH-RESOLVE
//! # Compatibility: docs/parity/opcua.md#OPCUA-AUTH-TYPES
//! # Compatibility: docs/parity/opcua.md#OPCUA-AUTH-INSECURE
//! # Compatibility: docs/parity/opcua.md#OPCUA-AUTH-TOKEN-POLICY
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-PASSWORD-WITHOUT-USER

use std::error::Error;

use crate::opcua::codec::{
    ExpandedNodeId, ExtensionBody, ExtensionObject, NodeId, UaByteString, UaString,
};
use crate::opcua::crypto::UserKey;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::signature_data::SignatureData;
use crate::opcua::generated::user_token_policy::UserTokenPolicy;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::options::{OpcuaOptions, SecurityPolicy};
use crate::opcua::service::StatusFault;
use crate::{CredentialIdentity, Credentials, ErrorKind, Operation, PlcError, Result};

pub(super) enum Identity<'a> {
    Anonymous,
    UserName(&'a Credentials),
    X509(Box<UserKey>),
}

impl<'a> Identity<'a> {
    pub(super) fn resolve(
        credentials: Option<&'a Credentials>,
        options: &OpcuaOptions,
    ) -> Result<Self> {
        let Some(credentials) = credentials else {
            return Ok(Self::Anonymous);
        };
        match credentials.identity() {
            CredentialIdentity::X509 {
                certificate_file,
                private_key_file,
            } => {
                return Ok(Self::X509(Box::new(UserKey::load(
                    certificate_file,
                    private_key_file,
                )?)));
            }
            CredentialIdentity::UserNamePassword => {}
        }
        if credentials.username.is_empty() {
            return if credentials.password.expose().is_empty() {
                Ok(Self::Anonymous)
            } else {
                Err(PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA password requires a nonempty username",
                ))
            };
        }
        if !options.allow_insecure_credentials
            && !options.security_policy.is_secure()
            && options.server_certificate_file.is_none()
        {
            return Err(PlcError::new(
                ErrorKind::Authentication,
                "OPC UA username credentials over None require allow-insecure-credentials=true",
            ));
        }
        Ok(Self::UserName(credentials))
    }

    pub(super) const fn token_type(&self) -> UserTokenType {
        match self {
            Self::Anonymous => UserTokenType::UserTokenTypeAnonymous,
            Self::UserName(_) => UserTokenType::UserTokenTypeUserName,
            Self::X509(_) => UserTokenType::UserTokenTypeCertificate,
        }
    }

    pub(super) fn policy(
        &self,
        token: &UserTokenPolicy,
        endpoint: &UaString,
    ) -> Result<SecurityPolicy> {
        if matches!(self, Self::Anonymous) {
            return Ok(SecurityPolicy::None);
        }
        let uri = token
            .security_policy_uri
            .as_bytes()
            .filter(|uri| !uri.is_empty())
            .unwrap_or_else(|| endpoint.as_bytes().unwrap_or_default());
        [
            SecurityPolicy::None,
            SecurityPolicy::Basic256Sha256,
            SecurityPolicy::Aes128Sha256RsaOaep,
            SecurityPolicy::Aes256Sha256RsaPss,
        ]
        .into_iter()
        .find(|policy| policy.uri().as_bytes() == uri)
        .ok_or_else(|| {
            PlcError::new(
                ErrorKind::Unsupported(Operation::Connect),
                "OPC UA user token security policy is unsupported",
            )
        })
    }

    pub(super) const fn requires_server(&self, policy: SecurityPolicy) -> bool {
        !matches!(self, Self::Anonymous) && policy.is_secure()
    }

    pub(super) fn token(
        &self,
        policy: &UserTokenPolicy,
        effective: SecurityPolicy,
        certificate: &[u8],
        nonce: &[u8],
        options: &OpcuaOptions,
    ) -> Result<(ExtensionObject, SignatureData)> {
        let mut signature = empty_signature();
        let (id, body) = match self {
            Self::Anonymous => (
                321,
                Service::AnonymousIdentityToken {
                    policy_id: policy.policy_id.clone(),
                },
            ),
            Self::UserName(credentials) => {
                let (password, algorithm) = if effective.is_secure() {
                    (
                        crate::opcua::crypto::token::encrypt(
                            effective,
                            certificate,
                            credentials.password.expose(),
                            nonce,
                        )?,
                        UaString::new(effective.descriptor().encryption_uri),
                    )
                } else {
                    if !options.allow_insecure_credentials {
                        return Err(PlcError::new(
                            ErrorKind::Authentication,
                            "OPC UA plain username tokens require allow-insecure-credentials=true",
                        ));
                    }
                    if options.security_policy.is_secure()
                        && options.message_security == crate::opcua::options::MessageSecurity::Sign
                    {
                        return Err(PlcError::new(
                            ErrorKind::Authentication,
                            "OPC UA SIGN requires a protected username token policy",
                        ));
                    }
                    tracing::warn!(
                        "OPC UA sending plain username credentials: allow-insecure-credentials=true"
                    );
                    (
                        credentials.password.expose().as_bytes().to_vec(),
                        UaString::null(),
                    )
                };
                (
                    324,
                    Service::UserNameIdentityToken {
                        policy_id: policy.policy_id.clone(),
                        user_name: UaString::new(&credentials.username),
                        password: UaByteString::new(password),
                        encryption_algorithm: algorithm,
                    },
                )
            }
            Self::X509(key) => {
                if !effective.is_secure() {
                    return Err(PlcError::new(
                        ErrorKind::Authentication,
                        "OPC UA X509 identity requires a protected token policy",
                    ));
                }
                signature = SignatureData {
                    algorithm: UaString::new(effective.descriptor().signature_uri),
                    signature: UaByteString::new(
                        key.sign(effective, &[certificate, nonce].concat())?,
                    ),
                };
                (
                    327,
                    Service::X509IdentityToken {
                        policy_id: policy.policy_id.clone(),
                        certificate_data: UaByteString::new(key.certificate().to_vec()),
                    },
                )
            }
        };
        Ok((
            ExtensionObject {
                type_id: ExpandedNodeId::local(NodeId::numeric(0, id)),
                body: ExtensionBody::Known(Box::new(body)),
            },
            signature,
        ))
    }
}

const fn empty_signature() -> SignatureData {
    SignatureData {
        algorithm: UaString::null(),
        signature: UaByteString::null(),
    }
}

pub(super) fn activation_error(error: PlcError) -> PlcError {
    let mut source: Option<&(dyn Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(status) = current.downcast_ref::<StatusFault>()
            && matches!(
                status.code,
                0x801F_0000 | 0x8020_0000 | 0x8021_0000 | 0x8057_0000
            )
        {
            return PlcError::new(
                ErrorKind::Authentication,
                "OPC UA identity authentication refused",
            )
            .with_source(error);
        }
        source = current.source();
    }
    error
}
