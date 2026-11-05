# S109 caller-driven OPC UA renewal and idle expiry

Date: 2026-11-05 UTC (implementation started 2026-11-04 local).
ADR-0047 precedes implementation. Product implementation: 8d94757 and 040eb9b;
final fuzz-invariant correction: 35aa5d2.

## Behavior and boundaries

Operations renew a due token on the existing socket, correlation table and continued
request/sequence counters. OPN retains asymmetric protection alongside symmetric MSG/CLO;
SIGN still encrypts OPN. There is no spawned task, dependency, feature, normal public API,
version or tag change. General transport-loss reconnect/backoff remains S110.

channel-lifetime-ms defaults to 3600000 and min-channel-lifetime-ms to 5000, both positive
UInt32 milliseconds. Zero or larger server revisions fall back to the requested lifetime;
negative configuration is rejected (the wire field is unsigned). The effective scheduling
minimum is bounded by the request. Renewal uses integer ceil(75 percent), at least one
millisecond. Scheduling does not extend actual token expiry. The previous token keeps
only its original validity plus 25 percent receive grace and retires on the first
authenticated new-token MSG. Only one previous token is retained.

Concurrent callers share one admitted renewal. Backpressure before admission is nonfatal;
any admitted renewal error, including a dropped future, fails the lane. New keys install
while the receiver owns Wire, before releasing it. Certificate/trust, response handles,
channel and fresh token ids, fresh nonzero/unreflected client/server nonces and response
protocol are checked. Interleaved pending MSG replies retain normal dispatch. Shared
ServiceFault decoding preserves its typed error. No old-epoch request is replayed.

Known idle channel/session expiry repeats the complete existing handshake under a bounded
preparation lock. Each operation retains one session snapshot, including metadata and
staged writes; the new session has an empty metadata cache. OPN does not update session
activity. Caller cancellation/deadlines and connect bounds cover preparation. A dropped
recovery leaves Reconnecting visible and a later caller may retry. Close cancels recovery
and prevents publication after closure, including the activation/publication race.
Already-sent old writes remain Unknown and are never replayed. Old-session failure cannot
poison a successfully published session. No caller means no idle renewal or recovery.

## Native evidence and limits

The pinned open62541 v1.5.9 gate selects exactly 20 tests: the existing 17 plus three
lifecycle tests. All modern policies under SIGN_ENCRYPT survive more than three short
lifetimes; a test-only factory counts exactly one real socket open. None renewal also
retains one socket. Known channel/session idle expiry under None and Basic256Sha256 opens
exactly one replacement session/socket on the next call, with no work during idle time.
The image self-check and finite example pass. Final container and exported PKI are removed.

Deterministic tests additionally exercise exact lifetime/grace boundaries, previous-token
retirement/id reuse, interleaved replies, full admission tables, invalid/refused/tampered
renewal, cancellation/drop/provider failures, concurrent callers, idle cache replacement,
close races and ambiguous writes. Independent forced previous-token delay and sequence
wrap remain UNVERIFIED. The S108 protected-None/application-certificate limitation is
unchanged. No physical hardware was used.

## Local validation

- Full strict check passes: 2574 nextest cases and threaded cargo test; 588 merged plus
  five other doctests. Formatting, strict Clippy, rustdoc and no-default build pass.
- Standalone OPC UA strict Clippy and 568 library cases pass, including 394 OPC UA cases.
- Core 6978/6978, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 10866/10866 lines are 100%. Transport 2880/2900
  (99.31%) and testkit 5271/5394 (97.72%) pass unchanged floors. No changed exclusions.
- Focused mutations: 15 tested, 14 caught, one unviable, zero survivors/timeouts.
  mutants.toml, explicit gitignore=true, four jobs, debug profiles off, 120-second timeout.
  An explicit source-range diff scopes due/expired/previous-grace predicates and lifetime
  option arms. The pure lifetime module joins the existing mutation scope, without new
  exclusions. Artifacts: /tmp/stonecrab-s109-mutants/mutants.out/.
- All 59 isolated feature combinations plus no-default pass; manifests/lock restored.
  Regen/self-tests, dependencies/D23 trees, machete, examples, semver (202 passed/58
  skipped), parity/self-tests, lint attributes and local DCO pass.
- Fresh Cargo-cache package verification and reviewed 235-file inventory pass; three new
  files are channel/lifetime.rs, lane/renewal.rs and lifecycle_tests.rs. No fixtures/testdata
  ship. Unpacked all-feature and standalone OPC UA builds pass. Packaging follows doctests.
- Five lifetime URI corpus seeds cover short/max/zero/negative/overflow inputs. They
  exposed a stale accepted-key assertion in the fuzz harness. The correction adds the
  accepted keys and independently checks positive UInt32 values, defaults and parsed
  values; the saved failing seed replays successfully. Initial failure evidence is retained
  locally under /tmp/stonecrab-s109-fuzz-regression/. Fuzz-only inspection fields do not
  alter the normal public API.

Five parity rows close: 249 total, 167 implemented, 44 planned, 38 deferred. The general
no-background-tasks row remains planned until the Publish work in S116.

All 18 fuzz targets pass at least 60 seconds with zero crash artifacts in the final run:
[report](fuzz/2026-11-05-s109.md). Fuzz cfg(fuzzing) strict Clippy, deny and root/fuzz
policy comparison pass. The fuzz report's uncommitted changes were the root-only evidence
draft; its product and fuzz inputs match 35aa5d2 and the hosted source below.

## Hosted and merge evidence

[CI 41970120172](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41970120172)
on 9eef6871dfbd84678b2ee0541c70a1696d3736c8 passes all 32 executed jobs: both Rust
versions on Windows/macOS/Linux, coverage, cross-target builds, native OPC UA and other
interop, package verification and full serial fuzz smoke included. Scheduled and
first-branch DCO skip by workflow conditions; local DCO verifies the entire commit range.

The final evidence/handoff changes are root-only. Product, crate documentation,
generator, manifest/lock, script, interop, packaging and fuzz inputs match the hosted
source exactly. Merge uses --no-ff --no-squash --signoff, preserving every plan,
implementation, test and evidence commit, and pushes master. Final checks require a clean
master synced with origin, both merge parents and feature ancestry, and matching remote
HEAD. No release/tag/version change; next is S110 request-driven reconnect.
