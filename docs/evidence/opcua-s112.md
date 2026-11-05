# OPC UA S112 browse walk evidence

2026-11-05; ADR-0050, E18 and ADR-0034. No dependency, feature, release or
version change. No physical hardware was available or used.

## Scope and boundary

The generated service closure grows from 29 to 37 cases, adding BrowseDirection,
NodeClass and the Browse/BrowseNext structures/services from the pinned mspec.
Independent positive/truncated byte fixtures and matching service fuzz seeds cover
every enabled case; response handles are checked by the existing request lane.

The private connection path snapshots one Session for an iterative breadth-first
walk: blank/** starts at Objects, invalid starts are per-query errors, root is
previsited, and requests ask for forward HierarchicalReferences with subtypes, all
node classes and mask 0x3f. One node/continuation per request bounds fanout. Same-name
siblings, GUID/ByteString identities, backward parents and exact per-node failure
status survive traversal. Root refusals remain query errors; transport failures,
deadline and cancellation fail the operation.

The three UInt32 URI ceilings default to 65536 references per node, 1000000 visited
nodes and depth 64. Zero removes only that protocol ceiling. The finite core budget
is the smaller of request and connection limits and spans queries. Root counts as
visited, duplicate references count toward the per-node work limit, boundary children
are retained without expansion, and limits set truncation. Empty continuation pages
are legal and remain deadline/cancellation bounded.

Known continuation points are consumed or released on the original Session. Dropping
before a send retains known points in a count/byte-bounded queue; ready drains it
before later operations. Cleanup Drop never performs I/O or spawns a task. The lane's
existing write transmission marker distinguishes an unsent admission refusal from a
request that may have allocated or rotated an unseen point. The latter retires the
lane; S110 recovers on a later call without replaying the walk. Unseen points after
interruption require server session expiry; cleanup failure/queue overflow can also
retire the lane. CloseSession releases remaining session resources when reachable.
Do not read this as a guarantee of immediate server-side cleanup after transport loss.

Public Connection::browse remains the S111 Unsupported default and browse capability
remains false. S113 supplies typed addresses, variable attributes/access/shape,
metadata and filtering before enabling that capability. OPCUA-OP-BROWSE, ENTRY and
the combined WALK/attribute row stay planned. Eight precise config/reference/limit/
divergence rows close; OPC UA is 249 total, 176 implemented, 35 planned, 38 deferred.

## Local validation

- Full strict check passes: 2624 nextest tests, threaded cargo test, 592 merged and
  five other doctests, formatting, strict Clippy, rustdoc and no-default build.
- Core 7346/7346, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 11442/11442 lines are 100%. Transport
  2878/2900 (99.24%) and testkit 5374/5511 (97.51%) meet unchanged floors. No coverage
  exclusion or mutation exclusion was added.
- Standalone OPC UA strict all-target Clippy and 569 library tests pass without
  default features. With testkit enabled, the deterministic service/connection walk
  cases run in the full suite. All ten core-only browse contract tests pass.
- All 59 feature combinations plus no-default pass in an isolated copy with restored
  manifests. An initial main-checkout matrix was stopped and its manifests verified
  restored; the subsequent full check and final isolated matrix both pass.
- Thirteen focused relational/boolean/status mutations are all caught, with zero
  survivors, unviable mutants or timeouts. mutants.toml, explicit gitignore=true,
  four jobs, debug profiles disabled, timeout 120 seconds; in-diff limits selection
  to browse.rs and regex to Walk::reference/good operators. Artifacts:
  /tmp/stonecrab-s112-mutants/mutants.out/. The selected pure logic is unchanged by
  the final admission-tracking hardening; its regression test proves Backpressure
  sends no browse frame, preserves a known point and leaves the lane healthy.
- All 18 fuzz targets pass at least 60 seconds, no crash artifacts:
  [report](fuzz/2026-11-05-s112.md). New URI acceptance/invariants and 21 numeric
  boundary seeds accompany 16 new complete/truncated service seeds. The report's
  dirty marker precedes final commits; the fuzzed codec/parser behavior is unchanged
  by later traversal admission tracking. Fuzz cfg(fuzzing) strict Clippy and deny pass.
- Generator/offline/self-tests, dependencies/D23 trees, machete, semver (202 pass,
  58 skip), examples, parity/self-tests, attribute lint and local DCO pass.
- Independent open62541 v1.5.9 gate passes all 21 tests, image self-check and finite
  example. The new namespace walk checks counter/scalar nodes and a truncated Objects
  walk through the private connection path. Paging/release are deterministic evidence;
  this native case is namespace interoperability, not a public browse capability claim.
- Fresh Cargo-cache package verification passes the reviewed 247-file inventory.
  Eight added files are browse.rs, its tests and six generated support modules; no
  fixtures/testdata ship. Packaging follows completed doctests. Unpacked all-feature
  and standalone OPC UA builds are checked separately.

## Hosted and merge evidence

Hosted CI and final cleanup/merge checks are pending. Final handoff changes will be
root-only, preserving the hosted-tested product, generator, scripts, interop, package
and fuzz inputs. Merge uses --no-ff --no-squash --signoff and pushes master. Next is
S113 typed browse metadata; it is not part of S112.
