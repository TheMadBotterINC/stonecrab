# S100 staged OPC UA writes

Date: 2026-11-03. [ADR-0038](../adr/0038-staged-opcua-writes.md), under accepted ADR-0034
and core write ADR-0010. stonecrab stays 0.3.0; opcua remains opt-in outside default/full.
No dependency, feature name, public operation/outcome type or background task is added.

## Behavior and boundaries

ConnectionConfig access is authoritative. Default read-only refuses writes before all
other checks or metadata I/O; URI access=read-write and programmatic ReadWrite enable
writes. State, request/item/count/address, cancellation and absolute deadline bounds are
validated. A suffix fixes the element encoding; otherwise DataType, ValueRank,
ArrayDimensions and AccessLevel are read and successfully resolved metadata cached per
session. Each cold node needs a four-attribute Read, chunked by the same item limit.
Capacity is max_items_per_request with FIFO replacement. No lock spans I/O, a new session
starts empty, and failed metadata is never cached. Only missing/unsupported attributes
permit explicit inference; other failures never authorize a schema guess.

Eligible items are sent once in one WriteRequest, with local rejections retained in
request order. DataValue has value only. Scalars, arrays, rectangular matrices and ranges
use checked conversions; UInteger chooses the smallest fitting unsigned type. Ranges
require List, including one element; ranged tags now advertise that hint. Core's exact
hint contract is unchanged. Whole-array writes without a range use metadata/inference.
TIME is exact Int32 milliseconds, DateTime checked 1601 ticks aligned to 100 ns. Values
outside those representations reject rather than truncate. Character/Unicode strings
use OPC String; unavailable WORD/DWORD/LWORD/WSTRING suffixes are not added. ByteString
is additive. Other IEC calendar/time suffix writes and custom schema writes remain typed
Unsupported; custom definition resolution and structure writes remain S119/S120.

The lane reuses its exchange path with a private per-request transmission marker. Queue,
size, cancellation and deadline checks precede polling send I/O. A partial/failed send
remains channel-fatal, with a conservative Unknown for that write. Read retry policy is
never applied to writes. Close races the tracked write without losing its delivery state.
Pre-send failures remain errors. Lost/unusable exchanges and unexpected correlated
service types are Unknown; Uncertain Write results are also Unknown with their device
status retained. Core's existing TimeoutAfterTransmit fallback reason represents unusable
confirmations too; no new public reason is introduced. Bad results remain typed rejections
with exact status/source. A verified WriteResponse's wrong count is the roadmap's explicit
MalformedData error and never authorizes replay.

Pure traversal reserves children before growing its pending stack, enforcing the node
budget once. Owned WriteRequest disposal reuses S98's iterative tree disposer, including
rejected values and futures dropped before polling. A 100000-level caller List is rejected
or abandoned on a 128 KiB thread without recursive drop. Builder and explicit Clone
behavior are unchanged. Ten protocol rows close: 249 total, 125 implemented, 86 planned,
38 deferred. The independent 23-vector upstream corpus now has 17 exact round-trips,
one established NodeId canonicalization and five exact disabled-service refusals.

## Verification

- Deterministic suite: 2393 tests pass, including supported write/unknown injection in
  the public ContractSuite (24 passed, nine explicitly skipped unsupported cases).
- Coverage: core 6978/6978, codec 245/245, sim 2307/2307, modbus 6699/6699,
  sensors 1840/1840, boards 511/511, handwritten OPC UA 8130/8130, all 100%.
  Transport 2877/2897 (99.31%, floor 95), testkit 5272/5394 (97.74%, floor 90).
  No threshold or generated exclusion changed. Impossible error-mapper closures were
  replaced with existing error constructors, retaining checked conversions.
- Configured mutation campaign: 153 tested, 136 caught/17 unviable, exit 0. It covers
  write mapping/conversion, metadata decoding, and core request/tree disposal. Final
  shape campaign: 38 caught, exit 0. Final status campaign: 27 tested, 22 caught/five
  unviable, exit 0. No survivors, timeouts or new exclusions in these clean campaigns.
  All use --config mutants.toml, --gitignore true, jobs 4 and debug info disabled;
  regexes scope changed pure functions, with the emitted existing struct-field cases.
- The first campaign found four survivors: redundant prospective node checks and missing
  rank-zero boundaries. Accounting now reserves nodes once on enqueue; exact/over node
  and rank-zero tests enforce the behavior. Later scalar-or-one-dimensional range and
  uncertain/unexpected-confirmation checks received the focused final campaigns above.
- Regeneration/self-test pass for both protocols: generator checks, pinned hashes,
  deterministic output and unchanged generated trees. The three Write cases were enabled
  and regenerated in a separate gen commit. Each has independent complete/truncated
  service vectors and fuzz seeds. The generated dispatch has 28 cases.
- All 15 fuzz targets pass at 10 seconds each on the pinned nightly. The connection
  parser seed/assertions include access; the address target now checks List hints on
  ranges. Fuzz exercises parsers/codecs; write delivery/conversion semantics are covered
  by deterministic and mutation tests. Default/all-feature semver, dependency policy,
  machete and default/fuzz Clippy pass. Parity and attribute checks pass.
- Final full check, isolated feature matrix and fresh package/unpacked archive identity
  are recorded in the completion note after those gates finish.

## Independent peer

Pinned open62541 v1.5.9, commit 05c2b389b65cc35884eb40d800301af1f3d130cf;
linux/amd64 image sha256:387753eda7c2280d6d6b221943ed2e7b9af7b0d9f62c2b134c55b0bd327d124c.
All in-image checks, exactly nine native tests and the finite public example pass.
The new test writes/reads Boolean, eight integer widths/signs, Float/Double, Unicode/NUL
String, 100 ns DateTime, ByteString, character-string forms, TIME, a variable-length Int32
array, a 2x3 matrix, and one selected array element. It uses only the disposable writable
namespace; existing readonly nodes remain unchanged. A native request-builder failure
exposed the old scalar hint on ranged tags; fixing the hint preserved the core builder's
exact validation instead of weakening it. The stale address doctest was updated too.
Docker is stopped and copied host PKI removed; no key/password is in evidence. Native
software-peer evidence does not claim hardware, secure writes or custom structure support.

## Review and handoff

Self-review uses ADR-0005's single-maintainer waiver. The design was committed before
code. Shrink review retains pure write encoding/mapping, bounded session metadata and a
per-exchange marker at the existing I/O boundary. Existing codecs, read node/status
helpers, cache/session ownership, core response builders and iterative disposal are reused.
The old blanket Unsupported write path is removed; no manager/provider trait, runtime,
compatibility alias or hidden I/O is introduced. Numeric narrowing uses checked/native
conversions and exact floating comparison without as casts. Normal signed-off merge
preserves all commits. No physical device, release/tag or external messaging.

Next: S101, the accepted cryptography backend/policy/key-schedule slice. Private-key and
trust loading, secure discovery/activation and custom schema support remain their owners.

Logs: /tmp/stonecrab-s100-{check-complete,coverage-complete,clippy,interop-final,
mutants-final,shape-mutants-final,status-mutants-final,regen,fuzz,semver,semver-all,
dependencies,machete,default-clippy,fuzz-clippy,parity,lint-attrs}.log.
Mutation outcomes are in /tmp/stonecrab-s100-{mutants-final,shape-mutants-final,
status-mutants-final}/mutants.out/outcomes.json. Native evidence is under
 target/interop-evidence/open62541/ (no private keys).
