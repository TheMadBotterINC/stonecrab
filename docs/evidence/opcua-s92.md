# S92 public OPC UA scalar Read evidence

Measured 2026-11-03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch `feat/opcua-read`,
baseline master `2a22999` (S91). Final product implementation `38b896c`.

Connection.read validates the whole request before I/O and sends one Read service per
batch through the existing session/lane: maxAge zero, Both timestamps, all four NodeId
identifier forms, configured attribute and IndexRange, null data encoding. Results keep
caller order. Null/short/long result lists fail the whole call with MalformedData; bad
statuses, invalid UTF-8 and not-yet-supported mappings remain per-item errors.

The pure read module owns response/count validation and mapping. Boolean, eight exact
integer widths/signs, Float/Double (including NaN payloads/signed zero), String/XML and Null
are supported. Text retains NUL, whitespace and BOM; null strings map Null. Good and
Uncertain values retain original status. A missing Good value is Null; missing Uncertain
is an item error. The six assigned Bad codes map NotFound, InvalidDataType, AccessDenied
or InvalidAddress; other known failures are DeviceException and unknown failures Internal,
with exact code/name retained. Source time is attached only when supplied and convertible;
a null zero timestamp stays absent. Extended metadata/type mappings remain S95/S98.

The existing non-exhaustive ErrorKind gains NotFound, InvalidDataType and AccessDenied,
appended so old relative ordering is unchanged. Ordinary reads use the earlier of the
explicit deadline and per-attempt request timeout. Default one attempt sends once; an
explicit RetryPolicy allows bounded safe read attempts while the same lane is usable.
No reconnect, background task, public trait, new dependency or write support is added.
The connection closes pending reads with Disconnected; cancel-safe receives retain
buffered bytes, and interrupted sends fail the lane. Conditional StateCell updates use
its existing mutex and wake outside it, preventing late failures overwriting Closed.
An unpolled read can retain its transport lease until resumed/dropped; close remains
bounded and may fail its clean protocol goodbye at that deadline.

## Validation

| Check | Final result |
| --- | --- |
| `mise run check` | Passed: format, strict all-feature/all-target Clippy, tests/doctests, warnings-denied docs and no-default-feature build |
| Nextest | 2232 passed; 77 external-peer tests skipped in the normal suite |
| Doctests | 584 merged + 5 separate passed |
| Coverage | OPC UA 6544/6544 (100%); core 6671/6671, codec 245/245, sim 2303/2303, Modbus 6699/6699, sensors 1840/1840, boards 511/511; transport 2877/2897 (99.31%), testkit 5277/5394 (97.83%); all floors unchanged |
| Features | All 59 combinations and no-default-feature check passed, separately from other Cargo metadata work |
| Contract suite | 23 passed, 0 failed; exact 10 named skips for write-dependent cases, supported-ping cases and six polling/subscription cases; unsupported operations themselves passed |
| Semver | Default and all features against published 0.3.0 passed |
| Default / fuzz-only Clippy | Passed, including all-feature/all-target `RUSTFLAGS='--cfg fuzzing'` |
| Fuzz | All 14 targets passed, 10-second budget each (11 seconds reported); no crashes |
| Dependencies / machete | Advisory, license, source and ban checks passed; no unused dependencies |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Mutations | 38 total: 30 caught, 8 unviable, 0 survivors/timeouts; unmutated baseline passed |
| Parity / lint attributes | Passed with self-tests; 287 implemented/automated-verified rows referenced |
| Package | Verified 192-file archive and reviewed list, 784836 compressed bytes at 38b896c; unpacked archive also builds with all features |

Deterministic tests cover batch order/one-request behavior, all supported scalar types,
full NodeId forms and attribute/index fields, status classifications, partial invalid
UTF-8, nulls, explicit/default deadlines, cancellation/drop with a subsequent read,
connection-budget/foreign-address/count rejection before I/O, bad response type/status,
result count mismatch, bounded retries, disconnect/diagnostics, pending-read close,
terminal state watchers and a clock failure. The contract suite now preserves parsed
Arc driver options when rebuilding a configuration; it exercises the actual OPC UA
connection, including configured retry limits, rather than substituting default options.

The first mutation invocation inherited the configuration's wider examine globs (1683
mutants) and was interrupted; it is not a completed campaign. An explicit name filter
then selected the read campaign. That initial focused run found two surviving deletions
of per-item diagnostic state/category. Both missing assertions were added. Review also
identified the source-timestamp null sentinel. Its new regression caught an unapplied
edit, which was corrected before the final full tests/coverage/mutation/package/native
runs. Final mutation outcomes contain 32 read-module mutations (24 caught, 8 unviable)
and 6 additional configuration struct-field mutations emitted outside the name filter,
all caught. No exclusions were added to hide the survivors. Pure read.rs is now in the
normal mutation scope.

## Independent peer

The open62541 v1.5.9 in-container checks passed, then the final exact-count native run
selected three tests and all passed: HEL/OPN/CLO plus continued counters, public anonymous
session connect/close, and public Read of Boolean, Int32, Double, String plus an unknown
node in one batch. The four values match the peer's constants; the unknown node returns
NotFound with BadNodeIdUnknown (0x80340000). Clean close passed. The finite opcua_read
example also returned I32(-123456789) from ns=1;s=scalar/Int32 and exited successfully.

Source `05c2b389b65cc35884eb40d800301af1f3d130cf`; image
`sha256:f4ebdd0a43ba35d5eb6f888c23de0a56adebaf2c878e07ec7fc84ae61c200d62`.
Evidence: `target/interop-evidence/open62541/{image-digest.txt,check.txt,nextest.txt,example.txt,server.log}`.
The Docker peer was stopped and host test PKI removed. Its own secure-policy checks
validate the scaffold, not Stonecrab's future secure client. No hardware was available,
no keys were committed/uploaded and no crate or tag was published.

Self-review under ADR-0005's single-maintainer waiver includes the expanded exact-count
filter and example in the existing interop job, with unchanged pinned tools and cleanup.
The example evidence file is cleared with the other artifacts before a full run. Scope
remains explicit anonymous policy None, basic scalars, no chunking or secure policy.
Attribute selection is implemented; LocalizedText DisplayName conversion remains S98.
All integrations retain signed-off commits with ordinary merges. Hosted CI is triggered
on push; no hosted result is assumed here.

Logs: `/tmp/stonecrab-s92-{quality-complete,coverage-complete,features,semver,fuzz,
default-clippy,fuzz-clippy,dependencies,regen,package-complete,packaged-opcua-complete,
interop-complete,peer-check,read-mutants-complete}.log`.
Mutation outcomes: `/tmp/stonecrab-s92-read-mutants-complete/mutants.out/outcomes.json`.
