//! Bounded forward hierarchical traversal on one activated Session.
//!
//! # Compatibility: docs/parity/opcua.md#OPCUA-BROWSE-REFERENCES
//! # Compatibility: docs/parity/opcua.md#OPCUA-LIMIT-BROWSE
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-BROWSE-SIBLINGS
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-BROWSE-NODE-IDS
//! # Compatibility: docs/parity/opcua.md#OPCUA-DIV-BROWSE-STATUS

use std::collections::{BTreeSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use crate::opcua::codec::{ExpandedNodeId, NodeId, StatusCode, UaByteString};
use crate::opcua::generated::browse_description::BrowseDescription;
use crate::opcua::generated::browse_direction::BrowseDirection;
use crate::opcua::generated::browse_result::BrowseResult as Page;
use crate::opcua::generated::extension_object_definition::ExtensionObjectDefinition as Service;
use crate::opcua::generated::reference_description::ReferenceDescription;
use crate::opcua::generated::view_description::ViewDescription;
use crate::opcua::options::OpcuaOptions;
use crate::opcua::session::Session;
use crate::opcua::{OpcuaAddress, read, value};
use crate::{
    BrowseNode, CancellationToken, Clock, Deadline, ErrorKind, ItemError, Operation, PlcError,
    Result,
};

#[derive(Default)]
pub(super) struct Continuations {
    pending: Mutex<Vec<UaByteString>>,
    drain: tokio::sync::Mutex<()>,
}

/// Own every known point until it is consumed or acknowledged as released.
struct Points<'a> {
    session: &'a Session,
    known: Vec<UaByteString>,
    transmitted: AtomicBool,
}

impl<'a> Points<'a> {
    const fn new(session: &'a Session) -> Self {
        Self {
            session,
            known: Vec::new(),
            transmitted: AtomicBool::new(false),
        }
    }

    async fn release(&mut self, deadline: Deadline, cancel: &CancellationToken) -> Result<()> {
        while let Some(point) = self.known.last() {
            let reply = self
                .session
                .lane
                .exchange_until(
                    &self.session.authentication_token,
                    533,
                    |request_header| Service::BrowseNextRequest {
                        request_header,
                        release_continuation_points: true,
                        continuation_points: Some(vec![point.clone()]),
                    },
                    deadline,
                    cancel,
                )
                .await?;
            let Service::BrowseNextResponse {
                response_header,
                results,
                diagnostic_infos,
            } = reply
            else {
                return Err(malformed("expected continuation release response"));
            };
            good(response_header.service_result)?;
            if results.is_some_and(|v| !v.is_empty())
                || diagnostic_infos.is_some_and(|v| !v.is_empty())
            {
                return Err(malformed("continuation release response must be empty"));
            }
            self.known.pop();
        }
        Ok(())
    }

    async fn page(
        &mut self,
        node: &NodeId,
        max: u32,
        deadline: Deadline,
        cancel: &CancellationToken,
    ) -> Result<Page> {
        self.session.lane.check(deadline, cancel)?;
        let next = self.known.first().cloned();
        self.transmitted.store(false, Ordering::Release);
        let reply = if let Some(point) = next.as_ref() {
            self.session
                .lane
                .exchange_tracked(
                    &self.session.authentication_token,
                    533,
                    |request_header| Service::BrowseNextRequest {
                        request_header,
                        release_continuation_points: false,
                        continuation_points: Some(vec![point.clone()]),
                    },
                    deadline,
                    cancel,
                    Some(&self.transmitted),
                )
                .await
        } else {
            self.session
                .lane
                .exchange_tracked(
                    &self.session.authentication_token,
                    527,
                    |request_header| Service::BrowseRequest {
                        request_header,
                        view: ViewDescription {
                            view_id: NodeId::numeric(0, 0),
                            timestamp: 0,
                            view_version: 0,
                        },
                        requested_max_references_per_node: max,
                        nodes_to_browse: Some(vec![BrowseDescription {
                            node_id: node.clone(),
                            browse_direction: BrowseDirection::BrowseDirectionForward,
                            reference_type_id: NodeId::numeric(0, 33),
                            include_subtypes: true,
                            node_class_mask: 0,
                            result_mask: 0x3f,
                        }]),
                    },
                    deadline,
                    cancel,
                    Some(&self.transmitted),
                )
                .await
        };
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                // A decoded ServiceFault cannot allocate an unseen continuation point.
                if error.kind() == ErrorKind::DeviceException {
                    self.transmitted.store(false, Ordering::Release);
                }
                return Err(error);
            }
        };
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "all other service kinds are refused here"
        )]
        let (header, results) = match reply {
            Service::BrowseResponse {
                response_header,
                results,
                ..
            } if next.is_none() => (response_header, results),
            Service::BrowseNextResponse {
                response_header,
                results,
                ..
            } if next.is_some() => (response_header, results),
            _ => return Err(malformed("unexpected browse response")),
        };
        self.transmitted.store(false, Ordering::Release);
        let mut results = results.unwrap_or_default();
        if header.service_result.0 & 0xc000_0000 == 0 && results.len() == 1 {
            self.known.clear();
        }
        // Capture points even in a malformed cardinality response; never silently leak them.
        for result in &mut results {
            let point = std::mem::replace(&mut result.continuation_point, UaByteString::null());
            if point.as_bytes().is_some_and(|bytes| !bytes.is_empty()) {
                self.known.push(point);
            }
        }
        good(header.service_result)?;
        match <[Page; 1]>::try_from(results) {
            Ok([page]) => Ok(page),
            Err(_results) => Err(malformed("browse result count differs from request")),
        }
    }
}

impl Drop for Points<'_> {
    fn drop(&mut self) {
        if self.transmitted.load(Ordering::Acquire) {
            self.session.lane.fail(PlcError::new(
                ErrorKind::Disconnected,
                "OPC UA interrupted browse may own an unseen continuation",
            ));
            return;
        }
        if self.session.lane.is_failed() || self.known.is_empty() {
            return;
        }
        let (count, bytes) = self.session.lane.continuation_budget();
        let mut pending = self
            .session
            .continuations
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let used = pending
            .iter()
            .chain(&self.known)
            .fold(0_usize, |total, point| {
                total.saturating_add(point.as_bytes().map_or(0, <[u8]>::len))
            });
        if pending.len().saturating_add(self.known.len()) > count || used > bytes {
            drop(pending);
            self.session.lane.fail(PlcError::new(
                ErrorKind::ResourceLimit,
                "OPC UA continuation cleanup budget exceeded",
            ));
        } else {
            pending.append(&mut self.known);
        }
    }
}

pub(super) async fn release_pending(
    session: &Session,
    clock: &dyn Clock,
    deadline: Deadline,
    cancel: &CancellationToken,
) -> Result<()> {
    let _guard = crate::opcua::channel::within(clock, deadline, cancel, async {
        Ok(session.continuations.drain.lock().await)
    })
    .await?;
    let mut points = Points::new(session);
    points.known = std::mem::take(
        &mut *session
            .continuations
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner),
    );
    match points.release(deadline, cancel).await {
        Err(error)
            if !matches!(
                error.kind(),
                ErrorKind::Cancelled | ErrorKind::Timeout | ErrorKind::Backpressure
            ) =>
        {
            session.lane.fail(PlcError::new(
                error.kind(),
                "OPC UA continuation cleanup failed",
            ));
            Err(error)
        }
        result => result,
    }
}

struct Work {
    node: NodeId,
    index: Option<usize>,
    depth: u32,
}

struct Walk {
    nodes: Vec<BrowseNode>,
    pending: VecDeque<Work>,
    visited: BTreeSet<String>,
    max_nodes: usize,
    max_total: u32,
    max_depth: u32,
    truncated: bool,
}

impl Walk {
    fn new(root: NodeId, options: &OpcuaOptions, max_nodes: usize) -> Result<Self> {
        Ok(Self {
            visited: BTreeSet::from([value::node_text(root.clone())?]),
            pending: VecDeque::from([Work {
                node: root,
                index: None,
                depth: 0,
            }]),
            nodes: Vec::new(),
            max_nodes,
            max_total: options.browse_max_total_nodes,
            max_depth: options.browse_max_depth,
            truncated: false,
        })
    }

    fn reference(&mut self, reference: ReferenceDescription, work: &Work) -> Result<bool> {
        let ExpandedNodeId {
            node_id,
            namespace_uri,
            server_index,
        } = reference.node_id;
        if namespace_uri.is_some() || server_index.is_some_and(|index| index != 0) {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Browse),
                "OPC UA remote expanded browse identifier",
            ));
        }
        let address = value::node_text(node_id.clone())?;
        if self.visited.contains(&address) {
            return Ok(true);
        }
        if self.nodes.len() >= self.max_nodes
            || self.max_total != 0
                && self.visited.len() >= usize::try_from(self.max_total).unwrap_or(usize::MAX)
        {
            self.truncated = true;
            return Ok(false);
        }
        let name = reference
            .display_name
            .text
            .as_ref()
            .or(Some(&reference.browse_name.name))
            .and_then(crate::opcua::codec::UaString::text)
            .transpose()
            .map_err(|_utf8| malformed("browse name is not UTF-8"))?
            .unwrap_or_default();
        let mut node = BrowseNode::new(Arc::new(OpcuaAddress::parse(&address)?), name)?;
        if let Some(parent) = work.index {
            node = node.with_parent(parent);
        }
        let index = self.nodes.len();
        self.nodes.push(node);
        self.visited.insert(address);
        let depth = work.depth.saturating_add(1);
        if self.max_depth != 0 && depth >= self.max_depth {
            self.truncated = true;
        } else {
            self.pending.push_back(Work {
                node: node_id,
                index: Some(index),
                depth,
            });
        }
        Ok(true)
    }

    fn failure(&mut self, work: &Work, error: ItemError) -> std::result::Result<(), ItemError> {
        if let Some(index) = work.index {
            if let Some(node) = self.nodes.get_mut(index) {
                *node = node.clone().with_error(error);
            }
            Ok(())
        } else {
            Err(error)
        }
    }
}

pub(super) async fn walk(
    session: &Session,
    root: NodeId,
    options: &OpcuaOptions,
    max_nodes: usize,
    deadline: Deadline,
    cancel: &CancellationToken,
) -> Result<(std::result::Result<Vec<BrowseNode>, ItemError>, bool)> {
    let mut walk = Walk::new(root, options, max_nodes)?;
    while let Some(work) = walk.pending.pop_front() {
        let mut points = Points::new(session);
        let mut references = 0_u32;
        loop {
            let page = match points
                .page(
                    &work.node,
                    options.browse_max_references_per_node,
                    deadline,
                    cancel,
                )
                .await
            {
                Ok(page) => page,
                Err(error) if error.kind() == ErrorKind::DeviceException => {
                    let error = service_item(error);
                    points.release(deadline, cancel).await?;
                    if let Err(error) = walk.failure(&work, error) {
                        return Ok((Err(error), false));
                    }
                    break;
                }
                Err(error) => return Err(error),
            };
            if page.status_code.0 & 0xc000_0000 != 0 {
                points.release(deadline, cancel).await?;
                if let Err(error) = walk.failure(&work, status_item(page.status_code)) {
                    return Ok((Err(error), false));
                }
                break;
            }
            let mut stop = false;
            for reference in page.references.unwrap_or_default() {
                session.lane.check(deadline, cancel)?;
                if options.browse_max_references_per_node != 0
                    && references >= options.browse_max_references_per_node
                {
                    walk.truncated = true;
                    stop = true;
                    break;
                }
                references = references.saturating_add(1);
                if !walk.reference(reference, &work)? {
                    stop = true;
                    break;
                }
            }
            if stop
                || !points.known.is_empty()
                    && options.browse_max_references_per_node != 0
                    && references >= options.browse_max_references_per_node
            {
                walk.truncated = true;
                points.release(deadline, cancel).await?;
                break;
            }
            if points.known.is_empty() {
                break;
            }
        }
    }
    Ok((Ok(walk.nodes), walk.truncated))
}

fn service_status(error: &PlcError) -> Option<StatusCode> {
    let mut source: &(dyn std::error::Error + 'static) = error;
    loop {
        if let Some(status) = source.downcast_ref::<crate::opcua::service::StatusFault>() {
            return Some(StatusCode(status.code));
        }
        source = source.source()?;
    }
}

fn service_item(error: PlcError) -> ItemError {
    match service_status(&error) {
        Some(status) => status_item(status),
        None => ItemError::new(error),
    }
}

fn status_item(status: StatusCode) -> ItemError {
    ItemError::new(
        PlcError::new(read::classify(status), "OPC UA browse node refused")
            .with_source(crate::opcua::service::status_error(status)),
    )
    .with_status(read::protocol_status(status))
}

fn good(status: StatusCode) -> Result<()> {
    if status.0 & 0xc000_0000 == 0 {
        Ok(())
    } else {
        Err(crate::opcua::service::status_error(status))
    }
}

fn malformed(message: &str) -> PlcError {
    PlcError::new(
        ErrorKind::MalformedData,
        format!("OPC UA browse: {message}"),
    )
}

#[cfg(test)]
#[cfg(feature = "testkit")]
mod tests;
