# 0050: Caller-driven OPC UA browse walk

## Status

Accepted S112 design, 2026-11-05, under E18 and ADR-0034. No dependency,
feature, release or version change. Public browse capability remains disabled
until S113 supplies typed variable metadata.

## Design

A private connection operation snapshots one activated Session and walks forward
hierarchical references breadth first, assembling the S111 flat response within
the request, connection and protocol budgets. BrowseNext stays on that Session;
known continuation points are released and uncertain interrupted exchanges retire
the lane rather than replaying a walk on a replacement session.

- Extend the existing generator profile with BrowseDirection, NodeClass and the
  eight Browse/BrowseNext support and service cases. Regenerate from pinned mspec;
  extend the existing service decoder, handle correlation and vector tests. No
  handwritten wire types or new transport abstraction.
- Add opcua/browse.rs for the concrete service exchange and iterative traversal.
  Reuse read::node and value::node_text for all identifier forms, status
  classification, the lane's bounded exchange and the existing response builder.
  Query parse failures and root refusals are per-query errors. Add optional
  ItemError to BrowseNode for a refused child; continue its siblings and preserve
  the exact protocol status. Same-name siblings retain distinct node identities.
- Start blank/** at Objects (ns=0;i=85), otherwise parse the address. Mark the root
  visited before traversal. Request one node per service with Forward,
  HierarchicalReferences (ns=0;i=33), subtypes included, all node classes and result
  mask 0x3f. Reject nonlocal expanded identifiers rather than aliasing them.
- Reuse numeric option parsing for browse-max-references-per-node (65536),
  browse-max-total-nodes (1000000) and browse-max-depth (64). Zero disables that
  protocol limit only. Retain the finite core output bound across queries, count
  rejected/duplicate references against work budgets, check deadline/cancellation
  throughout and report truncation. Root depth is zero and root counts as visited.
- Keep continuation ownership scoped to the Session. A small bounded pending
  release queue permits cancellation before a new exchange without spawning work;
  subsequent callers drain it before ordinary operations. A dropped/failed exchange
  that could have allocated an unseen point fails the lane, and S110 recovery
  supplies a fresh Session on a later call. Never migrate points or retry a walk.
  CloseSession releases remaining server resources; an unseen point after transport
  interruption can only be reclaimed by server session expiry. Do not claim immediate
  cleanup in that case. Empty pages with continuation are legal and remain cancellable.
- Add a private connection entry point for operation diagnostics, one deadline and
  one Session snapshot. Keep Connection::browse's default Unsupported and
  Capabilities.browse=false through S112, preserving the shared contract until S113.
  No generic service framework, no public staging API and no recursive async tasks.

No existing implementation is replaced; shared identifier formatting is exposed
within opcua instead of duplicated. Metadata reads, access/type enrichment and
filtering stay in S113.

## Validation and delivery

Commit this sketch before code, then commit green generated/service and walk/test
increments. Test exact wire parameters and response cardinality/handles, continuation
drain/release/drop/cancellation, empty pages, malformed replies and service faults,
cycles, sibling identities, identifier forms, limits/zero and per-node failures.
Update URI fuzz acceptance/seeds and codec vectors. Add the independently built
open62541 namespace walk and preserve the exact native test-count gate.

Run full strict checks and coverage (retain 100 percent core/handwritten OPC UA),
standalone OPC UA, feature matrix, generator/dependency/parity/attribute/DCO gates,
focused mutations, all fuzz targets, examples, semver and a fresh reviewed package
with unpacked builds. Record limitations honestly in parity/evidence, run hosted CI,
then normal signed-off merge and push; no squash and no S113 work.

The service semantics follow [OPC UA Browse](https://reference.opcfoundation.org/specs/OPC-10000-4/5.9.2)
and [BrowseNext](https://reference.opcfoundation.org/specs/OPC-10000-4/5.9.3): empty pages
may carry continuation, points belong to a Session, and release replies have empty
results and diagnostic arrays.
