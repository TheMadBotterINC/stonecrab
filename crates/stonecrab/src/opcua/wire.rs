//! Bounded wire chunks authenticated before sequence or service parsing.

use std::sync::Arc;

use bytes::{Bytes, BytesMut};

use crate::opcua::chunk::SecuritySizes;
use crate::opcua::crypto::{Asymmetric, Backend, Symmetric};
use crate::opcua::frame::{Frame, FrameHeader, MessageType, OpcuaFramer, RawFramer};
use crate::transport::{FramedTransport, Framer, Transport};
use crate::{Deadline, Result};

enum Protection {
    Plain,
    Asymmetric(Arc<Asymmetric>),
    Symmetric(Box<Symmetric>),
}

pub(super) struct Wire {
    raw: FramedTransport<Bytes, RawFramer>,
    protection: Protection,
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
            protection: Protection::Plain,
            backend,
        }
    }

    pub(super) const fn is_protected(&self) -> bool {
        !matches!(self.protection, Protection::Plain)
    }

    pub(super) fn asymmetric(&mut self, asymmetric: Arc<Asymmetric>) {
        self.protection = Protection::Asymmetric(asymmetric);
    }

    pub(super) fn symmetric(&mut self, symmetric: Symmetric) {
        self.protection = Protection::Symmetric(Box::new(symmetric));
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
        match &self.protection {
            Protection::Plain => SecuritySizes::NONE.max_body(maximum, 4),
            Protection::Asymmetric(crypto) => crypto.max_body(maximum),
            Protection::Symmetric(crypto) => crypto.max_body(maximum),
        }
    }

    pub(super) fn replace_framer(&mut self, framer: OpcuaFramer) {
        self.raw.replace_framer(RawFramer(framer));
    }

    pub(super) async fn send_frame(&mut self, frame: &Frame, deadline: Deadline) -> Result<()> {
        let limit = self.raw.framer().0.send_limit();
        let bytes = match &self.protection {
            Protection::Plain => {
                let mut out = BytesMut::new();
                let mut framer = self.raw.framer().0;
                framer.encode(frame, &mut out)?;
                out.freeze()
            }
            Protection::Asymmetric(crypto) => crypto.seal(frame, limit)?,
            Protection::Symmetric(crypto) => crypto.seal(self.backend.as_ref(), frame, limit)?,
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
        match &self.protection {
            Protection::Plain => Ok(Frame::parse(bytes)?),
            Protection::Asymmetric(crypto) => crypto.open(bytes, limit),
            Protection::Symmetric(crypto) => crypto.open(self.backend.as_ref(), bytes, limit),
        }
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
