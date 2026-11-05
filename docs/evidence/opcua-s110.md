# S110 request-driven OPC UA reconnect

Date: 2026-11-05. ADR-0048 (8948ef4) precedes implementation (8f8202f).
Documentation and expanded secure discovery tests: 0292f4e.

## Behavior and boundaries

An observed failed lane with Disconnected/Timeout schedules the existing reconnect
policy. The next read/write waits for backoff on the injected clock and makes at most
one complete handshake attempt. Concurrent callers share preparation and recheck the
current session. No operation means no reconnect work. There is no dependency, feature,
public API, task, release or version change.

Reuse ReconnectPolicy, transport::Jitter, the S109 preparation lock and open_session.
Current-session ownership, scheduled attempt accounting and state changes share the
existing session mutex. Private conditional StateCell replacement returns drained wakers,
which wake after releasing that mutex. This avoids callbacks under the lock and late
old-session errors overwriting a fresh Connected state.

Failed handshakes advance backoff/attempts; exhaustion becomes Failed(Disconnected).
Opening a socket alone never resets the budget; a completed authenticated session does,
because it contains valid replies. Cancellation/drop or an earlier caller deadline
preserves the scheduled attempt. A handshake timeout at an equal caller/connect bound
still counts correctly. Known idle expiry retains immediate S109 preparation; failed
transport recovery then enters the same backoff policy. Close interrupts lock/backoff/
handshake waits and prevents publication after closure.

Every in-flight operation keeps its original session snapshot. Pending/queued old reads
fail Disconnected after transport loss without replay; transmitted writes stay Unknown.
A new operation may write on the replacement, which has a new authentication token,
fresh nonces/crypto preparation and empty metadata cache. Credential/key/trust files are
resolved again; missing key material fails before any replacement socket opens, with
redacted diagnostics. Healthy request timeouts and ordinary service refusals do not force reconnect. Failed
lanes with trust, authentication, configuration or malformed/protocol errors remain
terminal; handshake validation failures are terminal too.

No session re-activation, TransferSubscriptions or subscription re-creation is added.
Typed subscription termination remains S117; real network-fault recovery evidence is
S121. Existing S108 protected-None and independent sequence-wrap/old-token-delay limits
are unchanged. No physical hardware was used.

## Validation

- Full strict check: 2591 nextest cases and threaded cargo test pass; 588 merged plus
  five other doctests pass. Formatting, strict Clippy, rustdoc and no-default build pass.
  Final documentation Clippy/rustdoc and expanded secure reconnect tests also pass.
- Standalone OPC UA strict Clippy and 568 library cases pass, including 394 OPC UA cases.
- Core 6980/6980, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 10952/10952 lines are 100%. Transport 2878/2900
  (99.24%) and testkit 5272/5394 (97.74%) pass unchanged thresholds. No new exclusions.
- Focused reconnect cases cover exact backoff boundaries, one attempt per call, refusal
  and incomplete handshake, exhaustion/reset, equal versus earlier caller deadline,
  cancellation/drop during backoff/handshake, shared preparation, pending/queued requests,
  Unknown writes, closure and reentrant state-watch wakeups. A faulty clock cannot skip
  backoff. Late lifecycle updates cannot overwrite Closed or a healthy replacement.
- Secure reconnect tests cover all modern policies in SIGN/SIGN_ENCRYPT with discovery
  on/off, repeated discovery/session activation, a fresh token/nonce and cleared metadata.
  Public integration verifies lost-read recovery; the shared 33-case contract suite is
  green (24 pass, nine named skips), including lost-read and ambiguous-write cases.
- All 18 fuzz targets pass at least 60 seconds with zero crash artifacts:
  [report](fuzz/2026-11-05-s110.md). Its dirty-tree marker records documentation/test edits
  in progress; runtime reconnect/state code is unchanged after 8f8202f. Fuzz cfg(fuzzing)
  strict Clippy, deny and root/fuzz policy comparison pass. No new parser or corpus format.
- All 59 isolated feature combinations plus no-default pass; manifests/lock restored.
  Regen/self-tests, dependencies/D23 trees, machete, examples, semver (202 passed/58
  skipped), parity/self-tests, lint attributes and local DCO pass.
- The native open62541 v1.5.9 gate passes all existing 20 tests, image self-check and
  finite example. Final container and exported PKI are removed. These are regression
  checks of existing native behavior; request-driven transport-loss reconnect is
  simulator-tested, with real network faults explicitly left to S121.
- S110 changes connection/lane I/O and private state publication outside the existing
  pure-code mutation scope. No new mutation run or mutation-score improvement is claimed;
  the existing mutation configuration and exclusions are unchanged.

One parity row closes: 249 total, 168 implemented, 43 planned, 38 deferred.
OPCUA-OP-RECONNECT and OPCUA-DIV-RECONNECT document the implemented handshake portion
but remain planned overall until S117's subscription termination.

Fresh package/unpacked and hosted results will be recorded when complete.
