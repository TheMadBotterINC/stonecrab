# S111 bounded core browse contract

Date: 2026-11-05. ADR-0049 (91e4dbc) precedes implementation (af79d47), contract tests
(561d7b2), validation-order hardening (b4cb982) and documentation (e147161).

## Behavior and support boundary

The facade adds BrowseQuery/Request/Builder, BrowseNode, BrowseResult and
BrowseResponse/Builder, a positive ResourceLimits::max_browse_nodes and a provided
Connection::browse. These are additive APIs under E18, with no dependency, feature,
version, tag or release change. The simulator, Modbus and OPC UA still report browse
unsupported. The OPC UA walk and metadata remain S112/S113.

Named queries reuse TagId and retain opaque text, including a blank default start,
deadline and cancellation. Core rejects empty batches, duplicate identities, excess query
count and text above 8192 UTF-8 bytes. The first builder error is sticky. Query Debug
shows identity/byte length, never raw unvalidated text. Drivers own syntax validation
and per-query refusals. The request retains its total response node budget, default
10000; a driver must also enforce its actual connection's limits.

Nodes carry reusable Arc<dyn ProtocolAddress>, display name, access flags, optional
PlcValueType/array dimensions, immutable string metadata and local parent index. Same-name
siblings and repeated addresses stay distinct. None shape is unknown, empty dimensions
scalar and zero extents unknown length; metadata never allocates value arrays or infers
counts. Names/values are bounded to 8192 bytes, option keys to 256 bytes, options and rank
to 32. Empty/control-bearing option keys and contradictory address type hints refuse.
Metadata key bounds precede character scanning. Total node bounds precede parent scans.

Response assembly requires one result per query and restores caller order. Every parent
must point to an earlier node within its own query, rejecting cycles/dangling links without
recursive ownership/destruction. Operation time is validated. Typed ItemError/status is
query data; all-failed batches are valid. Successful truncation is explicit and cannot
accompany an error. Node overflow is ResourceLimit rather than silent dropping.

The provided browse method returns Unsupported(Operation::Browse), zero latency,
Validation category and current state with no I/O, regardless of deadline/cancellation.
Overrides own bounded deadlines, cancellation and continuation cleanup. Cached leases
forward through the existing through gate and refuse after release without invoking the
underlying override. No current driver capability or protocol traffic is added.

ContractFixture gains optional browsable query text and a setter; Requires gains Browse
and BrowsableQuery. The shared suite grows from 33 to 35 cases. Current drivers exercise
the named unsupported/no-I/O case and skip supported browse by a precise capability
reason. A dedicated test-only browsing driver exercises positive correlation, missing
fixture skips, deliberately wrong result identity and cached forwarding; it is not a
simulator/Modbus/OPC UA capability claim.

## Local validation

- Full strict check passes: 2602 nextest cases and threaded cargo test; 592 merged plus
  five other doctests. Formatting, strict Clippy, rustdoc and no-default build pass.
- All nine public browse API tests pass with no default features. Standalone OPC UA
  strict Clippy and 568 library tests (394 OPC UA cases) pass.
- Core 7338/7338, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 10952/10952 lines are 100%. Transport 2878/2900
  (99.24%) and testkit 5373/5511 (97.50%) pass unchanged floors. No new exclusions.
- Focused mutations on the final validation order: 18 tested, all 18 caught, zero
  survivors/unviable/timeouts. mutants.toml, explicit gitignore=true, four jobs, debug
  profiles off, 120-second timeout. A source-range diff restricts selection to the three
  new browse modules; regex selects relational/boolean validation operators. Guards
  cover type contradiction, shape/text/options limits, query count, time, backward parents,
  cumulative nodes and result count. New pure modules join the existing examination scope;
  exclusions are unchanged. Artifacts: /tmp/stonecrab-s111-mutants-final/mutants.out/.
  An earlier run also caught all 18; final rerun follows early-budget hardening.
- All 18 fuzz targets pass at least 60 seconds without crash artifacts:
  [report](fuzz/2026-11-05-s111.md). Its dirty-tree marker records documentation edits;
  runtime browse validation matches b4cb982. Fuzz cfg(fuzzing) strict Clippy, deny and
  root/fuzz policy comparison pass. No new wire parser or corpus format is introduced.
- All 59 isolated feature combinations plus no-default pass; manifests/lock restored.
  Regen/self-tests, dependencies/D23 trees, machete, examples, semver (202 passed/58
  skipped), parity/self-tests, attributes and local DCO pass.
- The native open62541 v1.5.9 gate passes its existing 20 tests, image self-check and
  finite example. These are regression checks, not browse interoperability evidence.
  Final native container and exported PKI are removed. No physical hardware was used.
- Fresh Cargo-cache package verification passes the reviewed 239-file inventory; only
  browse_node.rs, browse_request.rs and browse_response.rs are added. No fixtures/testdata
  ship. Packaging follows completed doctests.

CORE-CONNECTION-TRAIT and architecture/support documentation describe the additive API
without claiming device browse support. No new parity row closes: OPC UA stays at
249 total, 168 implemented, 43 planned and 38 deferred.

Unpacked all-feature and standalone OPC UA builds pass.

## Hosted and merge evidence

[CI 41984563228](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41984563228)
on 2c3f17cfb892b02ccddf359acca7832c0d63305c passes all 32 executed jobs: both Rust
versions on Windows/macOS/Linux, coverage, cross-target builds, native OPC UA and other
interop, package verification and full serial fuzz smoke included. Scheduled and
first-branch DCO skip by workflow conditions; local DCO verifies the entire commit range.

The final evidence/handoff changes are root-only. Product, crate documentation,
generator, manifest/lock, script, interop, packaging and fuzz inputs match the hosted
source exactly. Merge uses --no-ff --no-squash --signoff, preserving every plan,
implementation, test and evidence commit, and pushes master. Final checks require a clean
master synced with origin, both merge parents and feature ancestry, and matching remote
HEAD. No release/tag/version change; next is S112 OPC UA browse walk.
