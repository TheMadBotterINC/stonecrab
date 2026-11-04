//! Token-bound MSG/CLO authentication and AES-CBC protection before payload parsing.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-CRYPTO-SYMMETRIC

use bytes::Bytes;

use super::backend::Backend;
use super::derive::{KeyPair, Keys, derive_keys};
use super::{Policy, invalid};
use crate::codec::{BoundedReader, BoundedWriter};
use crate::opcua::codec::LITTLE;
use crate::opcua::frame::{Body, Frame, FrameHeader, MessageType};
use crate::opcua::options::{MessageSecurity, SecurityPolicy};
use crate::{ErrorKind, PlcError, Result};

const CLEAR_HEADER: usize = 16;
const SEQUENCE_END: usize = 24;

fn rejected() -> PlcError {
    PlcError::new(
        ErrorKind::Authentication,
        "OPC UA message security verification failed",
    )
}

fn limited() -> PlcError {
    PlcError::new(
        ErrorKind::ResourceLimit,
        "OPC UA protected chunk exceeds its byte limit",
    )
}

#[derive(Debug)]
struct Token {
    id: u32,
    keys: KeyPair,
}

/// A client sends with client keys and receives with server keys. Expiry is the caller's job.
#[derive(Debug)]
pub(in crate::opcua) struct Symmetric {
    policy: SecurityPolicy,
    encrypted: bool,
    channel_id: u32,
    current: Token,
    previous: Option<Token>,
}

impl Symmetric {
    #[cfg(all(test, feature = "testkit"))]
    pub(in crate::opcua) const fn into_server(mut self) -> Self {
        std::mem::swap(&mut self.current.keys.client, &mut self.current.keys.server);
        self
    }

    pub(in crate::opcua) fn new(
        backend: &dyn Backend,
        policy: SecurityPolicy,
        mode: MessageSecurity,
        channel_id: u32,
        token_id: u32,
        client_nonce: &[u8],
        server_nonce: &[u8],
    ) -> Result<Self> {
        if !policy.is_secure() || mode == MessageSecurity::None || channel_id == 0 || token_id == 0
        {
            return Err(invalid());
        }
        Ok(Self {
            policy,
            encrypted: mode == MessageSecurity::SignEncrypt,
            channel_id,
            current: Token {
                id: token_id,
                keys: derive_keys(backend, policy, client_nonce, server_nonce)?,
            },
            previous: None,
        })
    }

    pub(in crate::opcua) fn max_body(&self, maximum: usize) -> Result<usize> {
        self.policy
            .descriptor()
            .symmetric_sizes(self.encrypted)
            .max_body(maximum, 4)
    }

    pub(in crate::opcua) const fn token_id(&self) -> u32 {
        self.current.id
    }

    /// Derive before replacing either active set; failures leave both intact.
    pub(in crate::opcua) fn install(
        &mut self,
        backend: &dyn Backend,
        token_id: u32,
        client_nonce: &[u8],
        server_nonce: &[u8],
    ) -> Result<()> {
        if token_id == 0 || self.token(token_id).is_some() {
            return Err(invalid());
        }
        let token = Token {
            id: token_id,
            keys: derive_keys(backend, self.policy, client_nonce, server_nonce)?,
        };
        self.previous = Some(std::mem::replace(&mut self.current, token));
        Ok(())
    }

    pub(in crate::opcua) fn retire_previous(&mut self) {
        self.previous = None;
    }

    fn token(&self, id: u32) -> Option<&Token> {
        if self.current.id == id {
            Some(&self.current)
        } else {
            self.previous.as_ref().filter(|token| token.id == id)
        }
    }

    pub(in crate::opcua) fn seal(
        &self,
        backend: &dyn Backend,
        frame: &Frame,
        maximum: usize,
    ) -> Result<Bytes> {
        let chunk = match &frame.body {
            Body::Message(chunk) | Body::Close(chunk) => chunk,
            Body::Hello(_) | Body::Acknowledge(_) | Body::Error(_) | Body::Open(_) => {
                return Err(invalid());
            }
        };
        if chunk.channel_id != self.channel_id || chunk.token_id != self.current.id {
            return Err(invalid());
        }
        let policy = self.policy.descriptor();
        let plain = frame.length_in_bytes()?;
        let padding = if self.encrypted {
            padding(plain, policy)?
        } else {
            0
        };
        let total = plain
            .checked_add(padding)
            .and_then(|size| size.checked_add(policy.signature))
            .filter(|size| *size <= maximum)
            .ok_or_else(limited)?;
        let size = u32::try_from(total)
            .ok()
            .filter(|size| *size <= 0x7fff_ffff)
            .ok_or_else(limited)?;
        let mut writer = BoundedWriter::new(total);
        frame.serialize(size, &mut writer)?;
        if self.encrypted {
            let byte = u8::try_from(padding.saturating_sub(1))
                .ok()
                .ok_or_else(invalid)?;
            for _ in 0..padding {
                writer.write_u8(byte)?;
            }
        }
        let keys = &self.current.keys.client;
        let tag = backend
            .hmac(policy.hash, keys.signing.expose(), &[writer.as_slice()])
            .map_err(|_error| rejected())?;
        if tag.len() != policy.signature {
            return Err(rejected());
        }
        writer.write_bytes(&tag)?;
        let mut bytes = writer.into_bytes();
        if self.encrypted {
            let body = bytes.get_mut(CLEAR_HEADER..).ok_or_else(rejected)?;
            backend
                .encrypt(keys.encryption.expose(), iv(keys)?, body)
                .map_err(|_error| rejected())?;
        }
        Ok(Bytes::from(bytes))
    }

    /// No sequence header or service byte is parsed before authentication succeeds.
    pub(in crate::opcua) fn open(
        &self,
        backend: &dyn Backend,
        bytes: &[u8],
        maximum: usize,
    ) -> Result<Frame> {
        if bytes.len() > maximum {
            return Err(limited());
        }
        self.authenticate(backend, bytes)
            .map_err(|_error| rejected())
    }

    fn authenticate(&self, backend: &dyn Backend, bytes: &[u8]) -> Result<Frame> {
        let mut reader = BoundedReader::new(bytes);
        let header = FrameHeader::parse(&mut reader)?;
        if !matches!(
            header.message_type,
            MessageType::Message | MessageType::Close
        ) || usize::try_from(header.size).ok() != Some(bytes.len())
            || reader.read_u32(LITTLE)? != self.channel_id
        {
            return Err(rejected());
        }
        let token_id = reader.read_u32(LITTLE)?;
        let keys = &self.token(token_id).ok_or_else(rejected)?.keys.server;
        let policy = self.policy.descriptor();
        let auth_end = bytes
            .len()
            .checked_sub(policy.signature)
            .filter(|end| *end >= SEQUENCE_END)
            .ok_or_else(rejected)?;
        let mut plain = bytes.to_vec();
        if self.encrypted {
            let body = plain.get_mut(CLEAR_HEADER..).ok_or_else(rejected)?;
            backend.decrypt(keys.encryption.expose(), iv(keys)?, body)?;
        }
        let message = plain.get(..auth_end).ok_or_else(rejected)?;
        let tag = plain.get(auth_end..).ok_or_else(rejected)?;
        backend.verify_hmac(policy.hash, keys.signing.expose(), message, tag)?;
        let body_end = if self.encrypted {
            unpad(message)?
        } else {
            auth_end
        };
        let body = Body::parse(
            header.message_type,
            plain.get(8..body_end).ok_or_else(rejected)?,
        )
        .ok()
        .ok_or_else(rejected)?;
        Ok(Frame {
            chunk: header.chunk,
            body,
        })
    }
}

fn iv(keys: &Keys) -> Result<&[u8; 16]> {
    keys.iv
        .expose()
        .as_slice()
        .try_into()
        .map_err(|_error| invalid())
}

/// The size byte itself is present even when the additional padding count is zero.
fn padding(plain: usize, policy: Policy) -> Result<usize> {
    let unpadded = plain
        .checked_sub(CLEAR_HEADER)
        .and_then(|size| size.checked_add(policy.signature))
        .and_then(|size| size.checked_add(1))
        .ok_or_else(limited)?;
    let remainder = unpadded.checked_rem(policy.block).ok_or_else(invalid)?;
    let extra = policy
        .block
        .checked_sub(remainder)
        .and_then(|size| size.checked_rem(policy.block))
        .ok_or_else(invalid)?;
    extra.checked_add(1).ok_or_else(limited)
}

fn unpad(message: &[u8]) -> Result<usize> {
    let count = *message.last().ok_or_else(rejected)?;
    let length = usize::from(count).checked_add(1).ok_or_else(rejected)?;
    let end = message
        .len()
        .checked_sub(length)
        .filter(|end| *end >= SEQUENCE_END)
        .ok_or_else(rejected)?;
    if !message
        .get(end..)
        .ok_or_else(rejected)?
        .iter()
        .all(|byte| *byte == count)
    {
        return Err(rejected());
    }
    Ok(end)
}

#[cfg(test)]
mod tests;
