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
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::user_token_policy::UserTokenPolicy;
use crate::opcua::generated::user_token_type::UserTokenType;
use crate::opcua::options::{OpcuaOptions, SecurityPolicy};
use crate::opcua::service::StatusFault;
use crate::{CredentialIdentity, Credentials, ErrorKind, Operation, PlcError, Result};

#[derive(Clone, Copy)]
pub(super) enum Identity<'a> {
    Anonymous,
    UserName(&'a Credentials),
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
            CredentialIdentity::X509 { .. } => {
                return Err(PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA certificate identity activation is not implemented",
                ));
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
        if !options.allow_insecure_credentials {
            return Err(PlcError::new(
                ErrorKind::Authentication,
                "OPC UA username credentials over None require allow-insecure-credentials=true",
            ));
        }
        Ok(Self::UserName(credentials))
    }

    pub(super) const fn token_type(self) -> UserTokenType {
        match self {
            Self::Anonymous => UserTokenType::UserTokenTypeAnonymous,
            Self::UserName(_) => UserTokenType::UserTokenTypeUserName,
        }
    }

    pub(super) fn token(
        self,
        policy: &UserTokenPolicy,
        endpoint_policy: &UaString,
    ) -> Result<ExtensionObject> {
        let (id, body) = match self {
            Self::Anonymous => (
                321,
                Service::AnonymousIdentityToken {
                    policy_id: policy.policy_id.clone(),
                },
            ),
            Self::UserName(credentials) => {
                let effective = policy
                    .security_policy_uri
                    .as_bytes()
                    .filter(|uri| !uri.is_empty())
                    .unwrap_or_else(|| endpoint_policy.as_bytes().unwrap_or_default());
                if effective != SecurityPolicy::None.uri().as_bytes() {
                    return Err(PlcError::new(
                        ErrorKind::Unsupported(Operation::Connect),
                        "OPC UA protected username tokens are not implemented",
                    ));
                }
                (
                    324,
                    Service::UserNameIdentityToken {
                        policy_id: policy.policy_id.clone(),
                        user_name: UaString::new(&credentials.username),
                        password: UaByteString::new(
                            credentials.password.expose().as_bytes().to_vec(),
                        ),
                        encryption_algorithm: UaString::null(),
                    },
                )
            }
        };
        Ok(ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, id)),
            body: ExtensionBody::Known(Box::new(body)),
        })
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
