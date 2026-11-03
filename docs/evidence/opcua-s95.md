# S95 extended OPC UA value evidence

Measured 2026-11-03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch
`feat/opcua-extended-values`, baseline master `c30b8c0` (S94). Initial mapper `a9b91c5`,
final item/budget correction `519b5e7`; packaged source/docs `b4eed94`.

The pure Variant mapper now lives in opcua/value.rs; read.rs retains request, status and
item assembly and supplies each tag's suffix. The old scalar-only decoding path was
removed. No public value variant, dependency, feature, version or generator output changed.

Eighteen scalar kinds and their arrays map: Boolean, eight integer widths/signs,
Float/Double, String/XML, DateTime, Guid, NodeId, QualifiedName and StatusCode. Null and
empty supported arrays remain distinct; one-element arrays remain Lists. Matrices use
row-major nested Lists, preserving singleton axes; zero extents produce empty Lists.
The mapper checks dimensions against materialized count and bounds rank to 64, all items
of a response to 1048576 value/List nodes, and empty matrices to 64 List containers before
building containers. Construction is iterative and moves elements without cloning them.
Failed mappings do not consume the remaining shared budget. Good absent and explicit Null
values both consume one node; Bad status wins without consuming a mapped value.

DateTime uses i128 calendar arithmetic, preserving all 100 ns ticks, including zero,
negative ticks and i64 extrema, without depending on host SystemTime range. The UTC fields
use existing PlcDateTime; the core type itself remains zone-free. Source timestamp zero
still means absent. Guid uses canonical lower-case mixed-endian text. NodeId uses the
existing bounded tag formatter/grammar and parses back with the same identifier; invalid
UTF-8 or unrepresentable tag text fails only its item. QualifiedName is ns=<n>;s=<name>,
with null name mapped Null. StatusCode text keeps its name (or unknown) and full hex code;
a StatusCode value does not replace the item's quality/status.

Temporal overrides apply per mapped leaf. TIME is milliseconds and LTIME nanoseconds;
DATE is days since 1990, LDATE and DATE_AND_TIME seconds since 1990, LDATE_AND_TIME
milliseconds since 1990. Both time-of-day suffixes use milliseconds, following the pinned
LTIME_OF_DAY constructor rather than inferring its unit from its name. Null and temporal
values stay unchanged; other suffixes retain the server type. Counts use the existing
lossless integer contract. Floats, booleans, text, unsigned values beyond i64 and range /
overflow failures return per-item InvalidDataType rather than Java truncation/wrapping.

ByteString/LocalizedText remain S98; ExpandedNodeId, DataValue, Variant, DiagnosticInfo and
ExtensionObject/structure mapping are named Unsupported errors here, including their
null/empty typed arrays. A deferred kind does not fail the rest of the batch. Thirteen
assigned parity rows are implemented, plus explicit DateTime-precision and checked-override
divergences. Remaining type/metadata/structure rows retain their later owners.

The pinned Apache PLC4X source and SPI constructor review is recorded in
[ADR-0034 S95](../adr/0034-opcua-client-design.md#s95-implementation-sketch-2026-11-03).
Six OpcuaVariantToPlcValueTest behaviors are adapted with Rust values. StaticHelperTest
namespace/URI/nonnumeric cases exercise existing ExtensionObject dispatch; compact
numeric encodings remain covered by the built-in vectors. A missing ExpandedNodeId is
not representable in the Rust envelope. Original Java sources were read outside the
repository, not vendored; root/crate NOTICE record Apache attribution.

## Validation

| Check | Final result |
| --- | --- |
| `mise run check` | Passed: format, strict all-feature/all-target Clippy, nextest, cargo test/doctests, warnings-denied docs and no-default-feature build |
| Nextest | 2296 passed; 79 external-peer tests skipped in the ordinary suite |
| Doctests | 584 merged + 5 separate passed |
| Coverage | OPC UA 6975/6975 (100%); core 6671/6671, codec 245/245, sim 2303/2303, Modbus 6699/6699, sensors 1840/1840, boards 511/511; transport 2877/2897 (99.31%), testkit 5278/5394 (97.85%); all floors/exclusions unchanged |
| Features | All 59 combinations and no-default-feature build passed, isolated from other Cargo operations |
| Default / fuzz-only Clippy | Passed, including all-target/all-feature `RUSTFLAGS='--cfg fuzzing'` |
| Fuzz | All 15 targets passed with a 10-second budget (11 reported), including the extended decode/value target; no crashes |
| Mutations | Full mapper/read scope: 81 total, 64 caught, 17 unviable, no survivors/timeouts. After the absent-value fix, read rerun: 36 total, 30 caught, 6 unviable, none survived; both unmutated baselines passed |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Semver | Default and all features against published 0.3.0 passed |
| Dependencies / machete | Advisory/license/ban/source checks passed; no unused dependencies |
| Parity / lint attributes | Passed with self-tests; 315 implemented/automated-verified rows referenced |
| Package | Verified reviewed 196-file list, 803252 compressed bytes at b4eed94; upstream fixtures/XML/adapter absent; unpacked archive builds with all features |

Tests cover mapped kinds and array null/empty/one-element cases, integer widths/float bits,
2D/3D and singleton/zero axes, inconsistent/negative/hostile dimensions, exact rank/node
budgets, UTC epoch/tick extrema, Guid padding/layout, all NodeId forms/escaping and bad
text, QualifiedName and StatusCode forms, all eight suffix units/epochs/negative/range
cases, already-temporal/Null behavior, per-tag override plumbing and per-item isolation.
Eight first-party seeds extend the existing decode fuzz harness; accepted value trees
are traversed iteratively to check node and List-depth bounds. No new fuzz target or
harness dependency was required.

The first line report flagged the closing line of the multiline DateTime calendar
expression. Naming its Unix-nanosecond intermediate made that conversion clearer; i64
extrema and checked calendar failure regressions already existed. Coverage reached 100
without exclusions. Final review then caught Good absent values bypassing the new node
budget. The correction routes both absent and explicit Null through the same decoder;
new exact-depletion/error-precedence regressions pass, followed by full checks, coverage,
independent peer and affected-read mutation reruns. The 81-mutant campaign contains 45
value mutations (34 caught, 11 unviable), 30 read mutations (24 caught, 6 unviable) and six
configuration struct-field mutations emitted outside the name filter, all caught. The
final read rerun includes those six extra mutations too. No survivor exclusions were added.

## Independent peer and limits

The open62541 in-image checks passed. The final native filter selected exactly five tests,
all passed: channel/counter handoff, public session lifecycle, basic public scalar Read,
chunked private-lane Read, and the new extended public Read. The extended test sends one
32-tag batch under 8192-byte send/receive buffers. It verifies 18 mapped scalars, four
arrays, 2D/3D matrices and every element of the 4000/10000-element Int32 arrays. All 26
mapped items succeed; six deferred-kind errors are named and isolated. Close succeeds.
The finite public opcua_read example also passes. The peer recipe/namespace is unchanged.

Source `05c2b389b65cc35884eb40d800301af1f3d130cf` (v1.5.9); final image
`sha256:8f99f54e55f8372a595e5ab97700b6ba4404cb8ef2722ce69d17ed7b457a3590`.
Evidence: `target/interop-evidence/open62541/{image-digest.txt,check.txt,nextest.txt,example.txt,server.log}`.
The peer was stopped and host PKI removed. Its own secure-policy tests validate the
scaffold, not Stonecrab secure client execution. Portable epoch tests run locally;
Windows/other hosted matrix outcomes are pending CI, not claimed as local evidence.
No physical hardware, key commits/uploads, crate publication or tag publication.

Self-review under ADR-0005's single-maintainer waiver: one pure mapping module replaces
the old path; existing calendar types/formatter/codec boundaries are reused; no speculative
trait/task/dependency/public variant was added. Checked shape/node preflight bounds tree
construction; unsupported/invalid values preserve other items and statuses; temporal
range/precision differences and deferred kinds are explicit. The exact-count peer filter,
package contents, semver and coverage were checked. Normal signed-off merges preserve
commits. Hosted CI is triggered on push; no hosted result is assumed here.

Logs: `/tmp/stonecrab-s95-{focused-complete,quality-final,coverage-final,features,
default-clippy-final,fuzz-clippy-final,fuzz,mutants,read-mutants-final,regen,semver,
semver-all,dependencies,machete,parity,lint-attrs,package,packaged-all,interop-final}.log`.
Mutation outcomes: `/tmp/stonecrab-s95-{mutants,read-mutants-final}/mutants.out/outcomes.json`.
