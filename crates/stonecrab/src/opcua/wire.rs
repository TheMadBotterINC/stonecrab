//! Bounded wire chunks authenticated before sequence or service parsing.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};

use crate::opcua::chunk::SecuritySizes;
use crate::opcua::crypto::{Asymmetric, Backend, Symmetric};
use crate::opcua::frame::{Frame, FrameHeader, MessageType, OpcuaFramer, RawFramer};
use crate::transport::{FramedTransport, Framer, Transport};
use crate::{Deadline, Result};

pub(super) struct Wire {
    raw: FramedTransport<Bytes, RawFramer>,
    asymmetric: Option<Arc<Asymmetric>>,
    symmetric: Option<Box<Symmetric>>,
    backend: Arc<dyn Backend>,
}

impl Wire {
    pub(super) fn new(
        transport: Box<dyn Transport>,
        framer: OpcuaFramer,
        backend: Arc<dyn Backend>,
    ) -> Self {
        Self {
            raw: FramedTransport::new(transport, RawFramer(framer)),
            asymmetric: None,
            symmetric: None,
            backend,
        }
    }

    pub(super) const fn is_protected(&self) -> bool {
        self.asymmetric.is_some() || self.symmetric.is_some()
    }

    pub(super) fn asymmetric(&mut self, asymmetric: Arc<Asymmetric>) {
        self.asymmetric = Some(asymmetric);
    }

    pub(super) fn symmetric(&mut self, symmetric: Symmetric) {
        self.symmetric = Some(Box::new(symmetric));
    }

    #[cfg(all(test, feature = "testkit"))]
    pub(super) fn into_inner(self) -> Box<dyn Transport> {
        self.raw.into_inner()
    }

    #[cfg(all(test, feature = "testkit"))]
    pub(super) fn buffered_len(&self) -> usize {
        self.raw.buffered_len()
    }

    pub(super) fn transport(&self) -> &dyn Transport {
        self.raw.transport()
    }

    pub(super) fn max_frame_bytes(&self) -> usize {
        self.raw.max_frame_bytes()
    }

    pub(super) fn max_body(&self, maximum: usize) -> Result<usize> {
        let maximum = maximum.min(self.raw.framer().0.send_limit());
        if let Some(crypto) = &self.symmetric {
            crypto.max_body(maximum)
        } else if let Some(crypto) = &self.asymmetric {
            crypto.max_body(maximum)
        } else {
            SecuritySizes::NONE.max_body(maximum, 4)
        }
    }

    pub(super) fn max_open_body(&self, maximum: usize) -> Result<usize> {
        let maximum = maximum.min(self.raw.framer().0.send_limit());
        if let Some(crypto) = &self.asymmetric {
            crypto.max_body(maximum)
        } else {
            SecuritySizes::NONE.max_body(
                maximum,
                crate::opcua::frame::AsymmetricHeader::none().length_in_bytes()?,
            )
        }
    }

    pub(super) fn install_token(&mut self, id: u32, client: &[u8], server: &[u8]) -> Result<()> {
        if let Some(crypto) = &mut self.symmetric {
            crypto.install(self.backend.as_ref(), id, client, server)?;
        }
        Ok(())
    }

    pub(super) fn retire_previous(&mut self) {
        if let Some(crypto) = &mut self.symmetric {
            crypto.retire_previous();
        }
    }

    pub(super) fn nonce(&self) -> Result<[u8; 32]> {
        crate::opcua::crypto::session_nonce(self.backend.as_ref())
    }

    pub(super) fn replace_framer(&mut self, framer: OpcuaFramer) {
        self.raw.replace_framer(RawFramer(framer));
    }

    pub(super) async fn send_frame(&mut self, frame: &Frame, deadline: Deadline) -> Result<()> {
        let limit = self.raw.framer().0.send_limit();
        let bytes = if matches!(frame.body, crate::opcua::frame::Body::Open(_)) {
            if let Some(crypto) = &self.asymmetric {
                crypto.seal(frame, limit)?
            } else {
                self.plain(frame)?
            }
        } else if let Some(crypto) = &self.symmetric {
            crypto.seal(self.backend.as_ref(), frame, limit)?
        } else if let Some(crypto) = &self.asymmetric {
            crypto.seal(frame, limit)?
        } else {
            self.plain(frame)?
        };
        self.raw.send_frame(&bytes, deadline).await
    }

    pub(super) async fn next_frame(&mut self, deadline: Deadline) -> Result<Frame> {
        let bytes = self
            .raw
            .next_frame(deadline)
            .await
            .map_err(|error| self.receive_error(error))?;
        self.decode(&bytes)
            .map_err(|error| self.receive_error(error))
    }

    fn decode(&self, bytes: &[u8]) -> Result<Frame> {
        let header = FrameHeader::parse(&mut crate::codec::BoundedReader::new(bytes))?;
        if header.message_type == MessageType::Error {
            return Ok(Frame::parse(bytes)?);
        }
        let limit = self.raw.framer().0.receive_limit();
        if header.message_type == MessageType::Open {
            if let Some(crypto) = &self.asymmetric {
                return crypto.open(bytes, limit);
            }
        } else if let Some(crypto) = &self.symmetric {
            return crypto.open(self.backend.as_ref(), bytes, limit);
        } else if let Some(crypto) = &self.asymmetric {
            return crypto.open(bytes, limit);
        }
        Ok(Frame::parse(bytes)?)
    }

    fn plain(&self, frame: &Frame) -> Result<Bytes> {
        let mut out = BytesMut::new();
        let mut framer = self.raw.framer().0;
        framer.encode(frame, &mut out)?;
        Ok(out.freeze())
    }

    fn receive_error(&self, error: crate::PlcError) -> crate::PlcError {
        if self.is_protected()
            && matches!(
                error.kind(),
                crate::ErrorKind::Authentication | crate::ErrorKind::MalformedData
            )
        {
            crate::PlcError::new(
                crate::ErrorKind::Authentication,
                "OPC UA message security verification failed",
            )
        } else {
            error
        }
    }

    pub(super) async fn shutdown(&mut self, deadline: Deadline) -> Result<()> {
        self.raw.shutdown(deadline).await
    }
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wire")
            .field("transport", &self.raw)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests {
    use super::*;
    use crate::opcua::frame::{Body, Frame, SecureChunk, SequenceHeader};
    use crate::opcua::{
        OpcuaDriver, crypto::Security, options::OpcuaOptions, secure_responder::Fixture,
    };
    use crate::testkit::{FakeClock, FakeTransport, Script, block_on_with};
    use crate::{Clock, ErrorKind, ResourceLimits};

    #[test]
    fn opcua_asymmetric_only_wire_refuses_service_messages_before_symmetric_keys_exist() {
        let fixture = Fixture::new();
        let clock = FakeClock::default();
        clock.set_wall_now(
            std::time::UNIX_EPOCH
                .checked_add(std::time::Duration::from_secs(crate::opcua::pki::NOW))
                .unwrap(),
        );
        let config = OpcuaDriver::configuration(&fixture.uri(
            crate::opcua::options::SecurityPolicy::Basic256Sha256,
            crate::opcua::options::MessageSecurity::SignEncrypt,
            false,
        ))
        .unwrap();
        let security = Security::prepare(
            config.driver_options_as::<OpcuaOptions>().unwrap(),
            clock.wall_now(),
        )
        .unwrap()
        .unwrap();
        let limits = ResourceLimits::builder().build().unwrap();
        let transport = FakeTransport::new(Script::default(), clock.clone(), limits);
        let probe = transport.probe();
        let mut wire = Wire::new(
            Box::new(transport),
            OpcuaFramer::new(65535, limits),
            Arc::new(crate::opcua::crypto::Native),
        );
        wire.asymmetric(Arc::clone(&security.asymmetric));
        let frame = Frame::single(Body::Message(SecureChunk {
            channel_id: 17,
            token_id: 29,
            sequence: SequenceHeader {
                sequence_number: 1,
                request_id: 1,
            },
            body: Bytes::from_static(b"secret"),
        }));
        let deadline = Deadline::after(clock.now(), std::time::Duration::from_secs(1)).unwrap();
        assert_eq!(
            block_on_with(&clock, wire.send_frame(&frame, deadline))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
        assert_eq!(probe.sent().len(), 0);
        assert!(
            wire.decode(&crate::opcua::responder::message(
                1,
                1,
                17,
                29,
                b"unprotected"
            ))
            .is_err()
        );
    }
}
