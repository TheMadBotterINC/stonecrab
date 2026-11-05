# 0049: Bounded core browse contract

## Status

Accepted S111 design, 2026-11-05, under E18 and ADR-0034. Additive public types and a
provided Connection method; no dependency, feature, release or version change. No driver
gains browse support here. The OPC UA walk and metadata remain S112/S113.

## Design

A limits-bound request retains named protocol query strings, deadline and cancellation;
the provided Connection::browse returns typed Unsupported without I/O. Drivers assemble
one correlated result per query, each with either a flat bounded node list and truncation
flag or a typed item error, using the same sticky builders as reads and writes.

- Add browse_request.rs (BrowseQuery, BrowseRequest and builder), browse_node.rs
  (BrowseNode), and browse_response.rs (BrowseResult, BrowseResponse and builder), exported
  by the facade. Separate request validation, node metadata and result correlation without
  a new generic request layer or protocol interface. Reuse TagId for caller query identity,
  Arc<dyn ProtocolAddress>, PlcValueType and ItemError. Do not invent a second identifier
  or recursively owned tree. Query text remains opaque, including blank starts; a driver
  parses it and reports bad syntax per query. Limit text to 8192 UTF-8 bytes before copying;
  query Debug reports identity and byte length, never raw unvalidated text.
- ResourceLimits gains positive max_browse_nodes, default 10000, a total response budget
  across queries. It is a library safety bound, distinct from S112's protocol walk options;
  callers can raise it explicitly. Queries use max_items_per_request and reject duplicates,
  emptiness and overflow before insertion. Requests retain their node budget so response
  assembly cannot silently use looser limits. Runtime drivers must also enforce the
  actual connection limits, as with read/write requests.
- BrowseNode owns an address, display name, readable/writable flags, optional scalar type,
  optional array dimensions, metadata options and optional parent index. Defaults are no
  access flags, the address's type hint, unknown shape and no parent/options. Names and
  option values are at most 8192 bytes; at most 32 options with keys at most 256 bytes;
  at most 32 array dimensions. Empty/device Unicode names remain data. None shape is
  unknown, empty dimensions scalar, zero extents unknown-length dimensions. Do not infer
  element counts or allocate arrays from metadata. Type overrides cannot contradict an
  address hint. Setters validate before copying and keep fields private/immutable.
- Parent indices are local to a query's flat result and must point strictly backwards.
  This rules out dangling parents, cycles and recursive destruction while preserving
  same-name siblings and repeated addresses. Response assembly validates total nodes,
  exact query identities/count, uniqueness and operation interval; restores caller query
  order; and keeps per-query ItemError/status as data. A truncated successful result is
  explicit; limit overflow during assembly is ResourceLimit, not silent truncation.
- Extend Connection with a provided browse method using the existing unsupported
  diagnostics helper: Unsupported(Operation::Browse), Validation, zero latency and current
  state, regardless of query/deadline/token. Document default and overriding deadline,
  cancellation and continuation cleanup duties. Forward browse through LeasedConnection's
  existing through gate so lease expiry/closure still applies. No current capabilities
  change, and no network traffic is added.
- Extend ContractFixture with optional browsable query text and with_browsable; append
  Requires::Browse and BrowsableQuery. Add unsupported_browse_performs_no_io for every
  non-browsing driver and a supported browse correlation/bounds case for future drivers.
  Test both capability/fixture skip paths using existing test doubles, and update exact
  suite counts/skip lists without hiding failures. Supported behavior is exercised by
  a small explicit test driver, not by pretending sim/Modbus/OPC UA can browse.

## Validation and delivery

Commit the plan first, then green API/test and contract/forwarding increments. Test all
builder boundaries and sticky errors, redaction, parent forests, partial errors and
truncation, facade use and Send/Sync/object safety, cached forwarding and lease refusal,
default no-I/O behavior even for expired/cancelled requests, and contract suite outcomes.
Retain core and handwritten OPC UA 100 percent line coverage and existing exclusions.
Add the pure browse validation modules to the existing mutation scope and run focused
mutations. Run strict full check, standalone OPC UA, isolated feature matrix, regen,
dependencies/D23/machete, examples, semver, parity/attributes/DCO, all 18 fuzz targets
and fuzz Clippy/deny, fresh reviewed package/unpacked builds after doctests, existing
20-test native OPC UA gate/self-check/example/cleanup and hosted CI.

Update CORE-CONNECTION-TRAIT and architecture/support documentation without claiming a
driver browse capability. Final root-only handoff notes preserve hosted-tested build
inputs; normal --no-ff --no-squash --signoff merge and push to master. Do not start S112.
