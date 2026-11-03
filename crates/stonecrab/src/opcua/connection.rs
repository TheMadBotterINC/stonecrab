//! The public Connection contract over an activated OPC UA session.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-CONNECT

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::opcua::session::Session;
use crate::{
    CancellationToken, Capabilities, Clock, Connection, ConnectionState, Deadline, ErrorKind,
    OpFuture, Operation, PlcError, ReadRequest, ReadResponse, ResourceLimits, Result, StateCell,
    StateWatch, WriteRequest, WriteResponse,
};

pub(super) struct OpcuaConnection {
    session: Session,
    clock: Arc<dyn Clock>,
    request_timeout: Duration,
    limits: ResourceLimits,
    state: StateCell,
    closing: AtomicBool,
}

impl OpcuaConnection {
    pub(super) fn new(
        session: Session,
        clock: Arc<dyn Clock>,
        request_timeout: Duration,
        limits: ResourceLimits,
    ) -> Self {
        Self {
            session,
            clock,
            request_timeout,
            limits,
            state: StateCell::new(ConnectionState::Connected).0,
            closing: AtomicBool::new(false),
        }
    }

    fn unavailable(&self, operation: Operation) -> PlcError {
        if self.state() != ConnectionState::Connected {
            return PlcError::new(ErrorKind::Disconnected, "OPC UA connection is closed");
        }
        PlcError::new(
            ErrorKind::Unsupported(operation),
            "OPC UA operation is not implemented",
        )
        .with_diagnostics(crate::Diagnostics {
            operation: Some(operation),
            connection_state: Some(self.state()),
            category: Some(crate::FailureCategory::Validation),
            ..crate::Diagnostics::default()
        })
    }

    async fn close_inner(&self) -> Result<()> {
        if self.state.current() == ConnectionState::Closed {
            return Ok(());
        }
        let _release = Release(self);
        let deadline = Deadline::after(self.clock.now(), self.request_timeout)?;
        let watch = self.state.watch();
        if self.closing.swap(true, Ordering::AcqRel) {
            return crate::opcua::channel::within(
                self.clock.as_ref(),
                deadline,
                &CancellationToken::new(),
                async {
                    while watch.current() != ConnectionState::Closed {
                        watch.changed().await?;
                    }
                    Ok(())
                },
            )
            .await;
        }
        self.state.set(ConnectionState::Closing);
        self.session
            .close(deadline, &CancellationToken::new())
            .await
    }
}

impl Connection for OpcuaConnection {
    fn state(&self) -> ConnectionState {
        self.state.current()
    }
    fn watch_state(&self) -> StateWatch {
        self.state.watch()
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }
    fn limits(&self) -> ResourceLimits {
        self.limits
    }
    fn read(&self, _request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        Box::pin(async move { Err(self.unavailable(Operation::Read)) })
    }
    fn write(&self, _request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        Box::pin(async move { Err(self.unavailable(Operation::Write)) })
    }
    fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(self.close_inner())
    }
}

struct Release<'a>(&'a OpcuaConnection);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.session.lane.fail(PlcError::new(
            ErrorKind::Disconnected,
            "OPC UA connection closed",
        ));
        self.0.state.set(ConnectionState::Closed);
    }
}

impl Drop for OpcuaConnection {
    fn drop(&mut self) {
        self.session.lane.fail(PlcError::new(
            ErrorKind::Disconnected,
            "OPC UA connection dropped",
        ));
        self.state.set(ConnectionState::Closed);
    }
}

impl std::fmt::Debug for OpcuaConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpcuaConnection")
            .field("state", &self.state())
            .field("configured_policy", &"None")
            .field("effective_mode", &"None")
            .field("revised_session_timeout", &self.session.revised_timeout)
            .finish_non_exhaustive()
    }
}
