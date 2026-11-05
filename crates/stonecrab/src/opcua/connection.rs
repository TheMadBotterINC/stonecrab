//! The public Connection contract over an activated OPC UA session.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-CONNECT
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-READ
//! # Compatibility: docs/parity/opcua.md#OPCUA-OP-CLOSE
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-NO-REACTIVATE

use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::future::wake_all;

use crate::opcua::driver::OpcuaDriver;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::read_value_id::ReadValueId;
use crate::opcua::generated::timestamps_to_return::TimestampsToReturn;
use crate::opcua::session::Session;
use crate::{
    CancellationToken, Capabilities, Clock, Connection, ConnectionState, Deadline, ErrorKind,
    OpFuture, Operation, PlcError, ReadRequest, ReadResponse, ResourceLimits, Result, StateCell,
    StateWatch, WriteRequest, WriteResponse,
};

struct Current {
    session: Arc<Session>,
    attempts: u32,
    jitter: crate::transport::Jitter,
}

pub(super) struct OpcuaConnection {
    session: Mutex<Current>,
    preparation: tokio::sync::Mutex<()>,
    driver: OpcuaDriver,
    config: crate::ConnectionConfig,
    clock: Arc<dyn Clock>,
    request_timeout: Duration,
    limits: ResourceLimits,
    state: StateCell,
    closing: AtomicBool,
    operations_stopped: CancellationToken,
    retry: crate::RetryPolicy,
    access: crate::AccessPolicy,
}

impl OpcuaConnection {
    pub(super) fn new(
        session: Session,
        driver: OpcuaDriver,
        config: crate::ConnectionConfig,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            session: Mutex::new(Current {
                session: Arc::new(session),
                attempts: 0,
                jitter: crate::transport::Jitter::new(),
            }),
            preparation: tokio::sync::Mutex::new(()),
            driver,
            request_timeout: config.timeouts().request,
            limits: *config.limits(),
            retry: config.retry().clone(),
            access: config.access(),
            config,
            clock,
            state: StateCell::new(ConnectionState::Connected).0,
            closing: AtomicBool::new(false),
            operations_stopped: CancellationToken::new(),
        }
    }

    pub(super) fn current(&self) -> Arc<Session> {
        Arc::clone(
            &self
                .session
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .session,
        )
    }

    fn observe_failure(&self) {
        let wakers = {
            let mut current = self.session.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(kind) = current.session.lane.failure_kind() else {
                return;
            };
            if self.state.current() != ConnectionState::Connected {
                return;
            }
            let wakers = self.failed(&mut current, ConnectionState::Connected, kind);
            drop(current);
            wakers
        };
        wake_all(wakers);
    }

    fn failed(
        &self,
        current: &mut Current,
        previous: ConnectionState,
        kind: ErrorKind,
    ) -> Vec<std::task::Waker> {
        let next = if matches!(kind, ErrorKind::Disconnected | ErrorKind::Timeout) {
            let retry = current.attempts;
            if self.config.reconnect().is_exhausted(retry) {
                ConnectionState::Failed(ErrorKind::Disconnected)
            } else {
                let delay = self
                    .config
                    .reconnect()
                    .next_delay(retry, current.jitter.next_unit());
                let now = self.clock.now();
                current.attempts = retry.saturating_add(1);
                ConnectionState::Reconnecting {
                    attempt: current.attempts,
                    next_at: now.checked_add(delay).unwrap_or(now),
                }
            }
        } else {
            ConnectionState::Failed(kind)
        };
        self.state.replace_if(previous, next)
    }

    async fn ready(&self, deadline: Deadline, cancel: &CancellationToken) -> Result<Arc<Session>> {
        let preparation = async {
            crate::opcua::channel::within(self.clock.as_ref(), deadline, cancel, async { Ok(()) })
                .await?;
            self.observe_failure();
            let session = self.current();
            if self.state.current() == ConnectionState::Connected
                && !session.lane.expired(session.revised_timeout)
            {
                crate::opcua::browse::release_pending(
                    &session,
                    self.clock.as_ref(),
                    deadline,
                    cancel,
                )
                .await?;
                return Ok(session);
            }
            let _guard =
                crate::opcua::channel::within(self.clock.as_ref(), deadline, cancel, async {
                    Ok(self.preparation.lock().await)
                })
                .await?;
            self.observe_failure();
            let session = self.current();
            let previous = self.state.current();
            if previous == ConnectionState::Connected {
                if !session.lane.expired(session.revised_timeout) {
                    crate::opcua::browse::release_pending(
                        &session,
                        self.clock.as_ref(),
                        deadline,
                        cancel,
                    )
                    .await?;
                    return Ok(session);
                }
                let wakers = {
                    let _current = self.session.lock().unwrap_or_else(PoisonError::into_inner);
                    self.state.replace_if(
                        previous,
                        ConnectionState::Reconnecting {
                            attempt: 1,
                            next_at: self.clock.now(),
                        },
                    )
                };
                wake_all(wakers);
                session.lane.fail(PlcError::new(
                    ErrorKind::Disconnected,
                    "OPC UA idle channel or session expired",
                ));
            }
            let reconnecting = self.state.current();
            let ConnectionState::Reconnecting { next_at, .. } = reconnecting else {
                return Err(stopped_error());
            };
            if self.clock.now() < next_at {
                crate::opcua::channel::within(self.clock.as_ref(), deadline, cancel, async {
                    self.clock.sleep_until(next_at).await?;
                    if self.clock.now() < next_at {
                        return Err(PlcError::new(
                            ErrorKind::InvalidConfig,
                            "clock woke before reconnect backoff",
                        ));
                    }
                    Ok(())
                })
                .await?;
            }
            let connect_bound = Deadline::after(self.clock.now(), self.config.timeouts().connect);
            let caller_bound = connect_bound.as_ref().is_ok_and(|bound| deadline < *bound);
            let result = async {
                self.driver
                    .open_session(&self.config, connect_bound?.min(deadline), cancel)
                    .await
            }
            .await;
            match result {
                Ok(session) => {
                    let session = Arc::new(session);
                    let wakers = {
                        let mut current =
                            self.session.lock().unwrap_or_else(PoisonError::into_inner);
                        if self.operations_stopped.is_cancelled() {
                            return Err(stopped_error());
                        }
                        current.session = Arc::clone(&session);
                        current.attempts = 0;
                        let wakers = self
                            .state
                            .replace_if(reconnecting, ConnectionState::Connected);
                        drop(current);
                        wakers
                    };
                    wake_all(wakers);
                    Ok(session)
                }
                Err(error) => {
                    if !(error.kind() == ErrorKind::Cancelled
                        || caller_bound && error.kind() == ErrorKind::Timeout)
                    {
                        let wakers = {
                            let mut current =
                                self.session.lock().unwrap_or_else(PoisonError::into_inner);
                            let wakers = self.failed(&mut current, reconnecting, error.kind());
                            drop(current);
                            wakers
                        };
                        wake_all(wakers);
                    }
                    Err(error)
                }
            }
        };
        let mut preparation = pin!(preparation);
        let mut stopped = pin!(self.operations_stopped.cancelled());
        poll_fn(|cx| {
            if stopped.as_mut().poll(cx).is_ready() {
                return std::task::Poll::Ready(Err(stopped_error()));
            }
            preparation.as_mut().poll(cx)
        })
        .await
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "S113 enables public typed browse")
    )]
    pub(super) async fn browse_walk(
        &self,
        request: crate::BrowseRequest,
    ) -> Result<crate::BrowseResponse> {
        let started = self.clock.now();
        let result = async {
            let mut stopped = pin!(self.operations_stopped.cancelled());
            let mut browse = pin!(self.browse_batch(&request, started));
            poll_fn(|cx| {
                if stopped.as_mut().poll(cx).is_ready() {
                    return std::task::Poll::Ready(Err(stopped_error()));
                }
                browse.as_mut().poll(cx)
            })
            .await
        }
        .await;
        result.map_err(|error| self.operation_error(Operation::Browse, started, error))
    }

    async fn browse_batch(
        &self,
        request: &crate::BrowseRequest,
        started: std::time::Instant,
    ) -> Result<crate::BrowseResponse> {
        if request.queries().len() > self.limits.max_items_per_request().get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "OPC UA browse exceeds connection query budget",
            ));
        }
        let cancel = request.cancel().cloned().unwrap_or_default();
        let default = Deadline::after(started, self.request_timeout)?;
        let deadline = request
            .deadline()
            .map_or(default, |requested| requested.min(default));
        crate::opcua::channel::within(self.clock.as_ref(), deadline, &cancel, async { Ok(()) })
            .await?;
        let options = self
            .config
            .driver_options_as::<crate::opcua::options::OpcuaOptions>()
            .ok_or_else(|| {
                PlcError::new(ErrorKind::Internal, "OPC UA connection options absent")
            })?;
        let mut session = None;
        let mut budget = request
            .max_nodes()
            .get()
            .min(self.limits.max_browse_nodes().get());
        let mut results = Vec::with_capacity(request.queries().len());
        for query in request.queries() {
            let parsed =
                crate::opcua::OpcuaAddress::parse_query(query.text()).and_then(|address| {
                    crate::opcua::read::node(&crate::Tag::new(
                        query.id().clone(),
                        Arc::new(address),
                    ))
                    .map(|value| value.node_id)
                });
            let (outcome, truncated) = match parsed {
                Err(error) => (Err(crate::ItemError::new(error)), false),
                Ok(root) => {
                    let session = match &session {
                        Some(session) => session,
                        None => session.insert(self.ready(deadline, &cancel).await?),
                    };
                    let (outcome, truncated) = crate::opcua::browse::walk(
                        session, root, options, budget, deadline, &cancel,
                    )
                    .await?;
                    if let Ok(nodes) = &outcome {
                        budget = budget.saturating_sub(nodes.len());
                    }
                    (outcome, truncated)
                }
            };
            results.push(
                crate::BrowseResult::new(query.id().clone(), outcome).with_truncated(truncated),
            );
        }
        results
            .into_iter()
            .fold(
                crate::BrowseResponse::builder(request, started, self.clock.now()),
                crate::BrowseResponseBuilder::item,
            )
            .build()
    }

    async fn read_inner(&self, request: ReadRequest) -> Result<ReadResponse> {
        let started = self.clock.now();
        let result = self.read_batch(&request).await;
        result.map_err(|error| self.operation_error(Operation::Read, started, error))
    }

    fn operation_error(
        &self,
        operation: Operation,
        started: std::time::Instant,
        error: PlcError,
    ) -> PlcError {
        self.observe_failure();
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
            operation: Some(operation),
            connection_state: Some(self.state()),
            latency: Some(self.clock.now().saturating_duration_since(started)),
            category: Some(category),
            retries,
            ..crate::Diagnostics::default()
        })
    }

    async fn read_batch(&self, request: &ReadRequest) -> Result<ReadResponse> {
        let started = self.clock.now();
        if !matches!(
            self.state(),
            ConnectionState::Connected | ConnectionState::Reconnecting { .. }
        ) {
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
        let first_deadline = Deadline::after(started, self.request_timeout)?;
        let first_deadline = request
            .deadline()
            .map_or(first_deadline, |requested| first_deadline.min(requested));
        let session = self.ready(first_deadline, &cancel).await?;
        let mut attempt = 0_u32;
        let reply = loop {
            attempt = attempt.saturating_add(1);
            let deadline = if attempt == 1 {
                first_deadline
            } else {
                let default = Deadline::after(self.clock.now(), self.request_timeout)?;
                request
                    .deadline()
                    .map_or(default, |requested| default.min(requested))
            };
            let result = self.read_once(&session, &nodes, deadline, &cancel).await;
            match result {
                Err(error)
                    if self.retry.should_retry(error.kind(), attempt)
                        && !session.lane.is_failed()
                        && !self.operations_stopped.is_cancelled()
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
        session: &Session,
        nodes: &[ReadValueId],
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Service> {
        let mut stopped = pin!(self.operations_stopped.cancelled());
        let mut exchange = session.lane.exchange(
            &session.authentication_token,
            631,
            |header| Service::ReadRequest {
                request_header: header,
                max_age: 0.0,
                timestamps_to_return: TimestampsToReturn::TimestampsToReturnBoth,
                nodes_to_read: Some(nodes.to_vec()),
            },
            Some(deadline),
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

    async fn write_inner(&self, request: WriteRequest) -> Result<WriteResponse> {
        let started = self.clock.now();
        self.write_batch(&request, started)
            .await
            .map_err(|error| self.operation_error(Operation::Write, started, error))
    }

    async fn write_batch(
        &self,
        request: &WriteRequest,
        started: std::time::Instant,
    ) -> Result<WriteResponse> {
        use crate::opcua::{OpcuaDataType, attributes, read, write};
        if self.access == crate::AccessPolicy::ReadOnly {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Write),
                "OPC UA connection is read-only",
            ));
        }
        if !matches!(
            self.state(),
            ConnectionState::Connected | ConnectionState::Reconnecting { .. }
        ) {
            return Err(PlcError::new(
                ErrorKind::Disconnected,
                "OPC UA connection is closed",
            ));
        }
        if request.items().len() > self.limits.max_items_per_request().get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                "OPC UA write exceeds the connection item budget",
            ));
        }
        let deadline = Deadline::after(started, self.request_timeout)?;
        let deadline = request
            .deadline()
            .map_or(deadline, |requested| deadline.min(requested));
        let cancel = request.cancel().cloned().unwrap_or_default();
        if cancel.is_cancelled() {
            return Err(PlcError::new(
                ErrorKind::Cancelled,
                "OPC UA write cancelled before transmission",
            ));
        }
        if deadline.is_elapsed(self.clock.now()) {
            return Err(PlcError::new(
                ErrorKind::Timeout,
                "OPC UA write deadline elapsed before transmission",
            ));
        }
        let nodes = request
            .items()
            .iter()
            .map(|item| read::node(item.tag()))
            .collect::<Result<Vec<_>>>()?;
        let session = self.ready(deadline, &cancel).await?;
        let mut local = Vec::new();
        let mut tags = Vec::new();
        let mut prepared = Vec::new();
        let mut budget = write::value::Budget::new(self.limits.max_buffered_bytes().get());
        for (item, node) in request.items().iter().zip(nodes) {
            let attributes = if node.attribute_id == 13
                && read::address(item.tag())?.data_type() == OpcuaDataType::Null
            {
                match attributes::resolve(
                    &session,
                    &node,
                    self.limits.max_items_per_request(),
                    deadline,
                    &cancel,
                )
                .await
                {
                    Ok(attributes) => attributes,
                    Err(error)
                        if session.lane.is_failed()
                            || matches!(
                                error.kind(),
                                ErrorKind::Timeout
                                    | ErrorKind::Cancelled
                                    | ErrorKind::Disconnected
                                    | ErrorKind::Backpressure
                                    | ErrorKind::ResourceLimit
                                    | ErrorKind::Internal
                            ) =>
                    {
                        return Err(error);
                    }
                    Err(error) => {
                        local.push(write::rejected(item.tag().id().clone(), error));
                        continue;
                    }
                }
            } else {
                attributes::Attributes::default()
            };
            match write::node(item, node, &attributes, &mut budget) {
                Ok(node) => {
                    tags.push(item.tag().id().clone());
                    prepared.push(node);
                }
                Err(error) => local.push(write::rejected(item.tag().id().clone(), error)),
            }
        }
        if prepared.is_empty() {
            return write::finish(request, local, started, self.clock.now());
        }
        let (reply, transmitted) = session
            .lane
            .write(
                &session.authentication_token,
                prepared,
                deadline,
                &cancel,
                &self.operations_stopped,
            )
            .await;
        match reply {
            Ok(reply @ Service::WriteResponse { .. }) => {
                write::response(request, tags, local, reply, started, self.clock.now())
            }
            Ok(_) => {
                local.extend(write::unknown(tags, ErrorKind::MalformedData));
                write::finish(request, local, started, self.clock.now())
            }
            Err(error) if transmitted => {
                self.observe_failure();
                local.extend(write::unknown(tags, error.kind()));
                write::finish(request, local, started, self.clock.now())
            }
            Err(error) => Err(error),
        }
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
        wake_all(self.state.replace_if(previous, ConnectionState::Closing));
        self.operations_stopped.cancel();
        self.current()
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
    fn write(&self, request: WriteRequest) -> OpFuture<'_, WriteResponse> {
        Box::pin(self.write_inner(request))
    }
    fn close(&self) -> OpFuture<'_, ()> {
        Box::pin(self.close_inner())
    }
}

fn stopped_error() -> PlcError {
    PlcError::new(ErrorKind::Disconnected, "OPC UA connection is closing")
}

struct Release<'a>(&'a OpcuaConnection);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.current().lane.fail(PlcError::new(
            ErrorKind::Disconnected,
            "OPC UA connection closed",
        ));
        self.0.state.set(ConnectionState::Closed);
    }
}

impl Drop for OpcuaConnection {
    fn drop(&mut self) {
        self.current().lane.fail(PlcError::new(
            ErrorKind::Disconnected,
            "OPC UA connection dropped",
        ));
        self.state.set(ConnectionState::Closed);
    }
}

impl std::fmt::Debug for OpcuaConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let session = self.current();
        let options = self
            .config
            .driver_options_as::<crate::opcua::options::OpcuaOptions>();
        f.debug_struct("OpcuaConnection")
            .field("state", &self.state())
            .field(
                "configured_policy",
                &options.map(|options| options.security_policy),
            )
            .field(
                "configured_mode",
                &options.map(|options| options.message_security),
            )
            .field("selected_endpoint", &session.selected_endpoint)
            .field("revised_session_timeout", &session.revised_timeout)
            .finish_non_exhaustive()
    }
}

pub(super) fn capabilities() -> Capabilities {
    Capabilities {
        read: true,
        write: true,
        ..Capabilities::default()
    }
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
