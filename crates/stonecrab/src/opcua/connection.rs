//! The public Connection contract over an activated OPC UA session.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-CONNECT
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-READ
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-CLOSE

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::generated::timestamps_to_return::TimestampsToReturn;
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
    reads_stopped: CancellationToken,
    retry: crate::RetryPolicy,
}

impl OpcuaConnection {
    pub(super) fn new(
        session: Session,
        clock: Arc<dyn Clock>,
        request_timeout: Duration,
        limits: ResourceLimits,
        retry: crate::RetryPolicy,
    ) -> Self {
        Self {
            session,
            clock,
            request_timeout,
            limits,
            state: StateCell::new(ConnectionState::Connected).0,
            closing: AtomicBool::new(false),
            reads_stopped: CancellationToken::new(),
            retry,
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

    async fn read_inner(&self, request: ReadRequest) -> Result<ReadResponse> {
        let started = self.clock.now();
        let result = self.read_batch(&request).await;
        result.map_err(|error| {
            if self.session.lane.is_failed() {
                self.state.set_if(
                    ConnectionState::Connected,
                    ConnectionState::Failed(error.kind()),
                );
            }
            let category = match error.kind() {
                ErrorKind::InvalidAddress
                | ErrorKind::InvalidConfig
                | ErrorKind::ResourceLimit
                | ErrorKind::Unsupported(_) => crate::FailureCategory::Validation,
                ErrorKind::MalformedData
                | ErrorKind::DeviceException
                | ErrorKind::NotFound
                | ErrorKind::InvalidDataType
                | ErrorKind::AccessDenied
                | ErrorKind::Authentication
                | ErrorKind::Trust => crate::FailureCategory::Protocol,
                ErrorKind::Internal => crate::FailureCategory::Internal,
                ErrorKind::Timeout
                | ErrorKind::Cancelled
                | ErrorKind::Disconnected
                | ErrorKind::Backpressure => crate::FailureCategory::Transport,
            };
            let retries = error.diagnostics().retries;
            error.with_diagnostics(crate::Diagnostics {
                operation: Some(Operation::Read),
                connection_state: Some(self.state()),
                latency: Some(self.clock.now().saturating_duration_since(started)),
                category: Some(category),
                retries,
                ..crate::Diagnostics::default()
            })
        })
    }

    async fn read_batch(&self, request: &ReadRequest) -> Result<ReadResponse> {
        let started = self.clock.now();
        if self.state() != ConnectionState::Connected {
            return Err(PlcError::new(
                ErrorKind::Disconnected,
                "OPC UA connection is closed",
            ));
        }
        if request.items().len() > self.limits.max_items_per_request().get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "OPC UA read exceeds the connection item budget",
            ));
        }
        let nodes = crate::opcua::read::nodes(request)?;
        let cancel = request.cancel().cloned().unwrap_or_default();
        let mut attempt = 0_u32;
        let reply = loop {
            attempt = attempt.saturating_add(1);
            let result = self.read_once(&nodes, request.deadline(), &cancel).await;
            match result {
                Err(error)
                    if self.retry.should_retry(error.kind(), attempt)
                        && !self.session.lane.is_failed()
                        && !self.reads_stopped.is_cancelled()
                        && !request
                            .deadline()
                            .is_some_and(|deadline| deadline.is_elapsed(self.clock.now())) => {}
                result => break result,
            }
        };
        reply
            .and_then(|reply| {
                crate::opcua::read::response(request, reply, started, self.clock.now())
            })
            .map_err(|error| {
                let mut diagnostics = error.diagnostics().clone();
                diagnostics.retries = attempt.saturating_sub(1);
                error.with_diagnostics(diagnostics)
            })
    }

    async fn read_once(
        &self,
        nodes: &[ReadValueId],
        deadline: Option<Deadline>,
        cancel: &CancellationToken,
    ) -> Result<Service> {
        let mut stopped = pin!(self.reads_stopped.cancelled());
        let mut exchange = self.session.lane.exchange(
            &self.session.authentication_token,
            631,
            |header| Service::ReadRequest {
                request_header: header,
                max_age: 0.0,
                timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
                nodes_to_read: Some(nodes.to_vec()),
            },
            deadline,
            cancel,
        );
        poll_fn(|cx| {
            if stopped.as_mut().poll(cx).is_ready() {
                return std::task::Poll::Ready(Err(PlcError::new(
                    ErrorKind::Disconnected,
                    "OPC UA connection is closing",
                )));
            }
            exchange.as_mut().poll(cx)
        })
        .await
    }

    async fn close_inner(&self) -> Result<()> {
        let previous = self.state.current();
        if previous == ConnectionState::Closed {
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
        self.state.set_if(previous, ConnectionState::Closing);
        self.reads_stopped.cancel();
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
        capabilities()
    }
    fn limits(&self) -> ResourceLimits {
        self.limits
    }
    fn read(&self, request: ReadRequest) -> OpFuture<'_, ReadResponse> {
        Box::pin(self.read_inner(request))
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

pub(super) fn capabilities() -> Capabilities {
    Capabilities {
        read: true,
        ..Capabilities::default()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
