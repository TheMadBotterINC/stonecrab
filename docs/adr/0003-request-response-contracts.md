# 0003: Bounded request and correlated response contracts
## Status
Amended by ADR-0018: the testkit named below is no longer a separate crate, so the
dev-dependency cycle between the core and the testkit no longer exists; it is the
`testkit` feature of `stonecrab`.
Accepted for the next unreleased core slice.
## Design
Protocol implementations own validated addresses behind `ProtocolAddress`; a `Tag`
adds caller identity, an optional scalar type and a positive element count. Requests
own immutable tags, reject duplicate identifiers and enforce a finite item budget
before growing their internal collection.
Read results retain every requested identifier, partial failures, sampling times
and original protocol status. Validated responses restore request order and reject
missing, extra or duplicate results instead of silently losing correlation.
Write results distinguish confirmed, rejected and unknown-after-transmit outcomes;
no retry or I/O behavior belongs in these data types.
## Files and boundaries
- Extend `tag.rs` with `ProtocolAddress` and `Tag`; reuse existing `TagId`.
- Add `request.rs` for a bounded read builder and explicit deadlines.
- Add `response.rs` for protocol status, item errors, read results and write outcomes.
- Keep batch correlation in `read_response.rs`, separate from individual outcomes.
- Add `tools/stonecrab-testkit` with a fixture address used by consumer contract tests.
  It is unpublished and never a runtime dependency of a published crate.
  Core integration tests using that path-only development dependency stay in the
  workspace rather than the published archive. Packaged core unit tests and
  documentation remain self-contained. Package verification uses a fresh Cargo
  cache to avoid stale same-version unpublished sibling archives.
  `cargo-deny` permits wildcard path dependencies for this unpublished development
  testkit; wildcard registry versions remain denied. Published runtime dependencies
  still require versioned paths, enforced by Cargo packaging. This is the narrow
  local-tooling exception to the version policy in coding standards §15.
- Use `bytes::Bytes` for retained protocol-status bytes, matching the specified
  shared-buffer representation rather than inventing a second byte container.
- Remove no existing public API; this is additive development work.
## Decisions
Use `Arc<dyn ProtocolAddress>` so cloned requests share immutable validated
addresses. Require `Any + Debug + Send + Sync`; downcasting uses stable trait
upcasting, not an `as_any` shim. The testkit is the sanctioned test double, not an
ad hoc mock in each module. The public address trait is required by the architecture,
not an executor or driver trait.
Start with explicit absolute deadlines. Cancellation tokens and relative-deadline
clock ownership remain deferred until a connection implementation can enforce them.
Builders return validation errors before any transport exists.
Item errors use a boxed body like connection errors, keeping result and outcome
enums compact while retaining status and source chains. The narrow
`clippy::enum_variant_names` expectation on `UnknownReason` preserves the
architecture's deliberate `AfterTransmit` names: wire consequences must remain
visible at call sites rather than being inferred from a generic timeout name.
The narrow `variant_size_differences` expectation on `WriteOutcome` permits a
boxed item error beside a one-byte reason. Do not allocate a second box solely
to make the variants appear equally sized.
Retain raw status bytes only within an explicit byte budget. `bytes` is used with
default features disabled and `std` enabled; its unsafe implementation is a
third-party dependency, never permission for first-party unsafe.
## Consequences
These contracts do not implement Modbus parsing, driver lookup, connections or
subscriptions. No wire compatibility is claimed from fixture addresses.
