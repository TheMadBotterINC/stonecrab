# 0048: Request-driven OPC UA reconnect

## Status

Accepted S110 design, 2026-11-05, under E19 and ADR-0034. No dependency, feature,
public API, spawned task, release or version change. Subscription termination remains
S117 and real network-fault evidence remains S121.

## Design

An observed transport failure schedules the existing reconnect policy, and the next
operation waits on the injected clock before repeating the full S109 handshake once.
The operation that lost its session and every pending operation keep that session's
outcome; none is replayed, and an already-transmitted write stays Unknown.

- Extend the existing connection session mutex to own a concrete Current value: the
  Arc<Session>, scheduled retry count and existing transport::Jitter. This keeps session
  publication, attempt accounting and failure observation atomic without a second book
  or generation registry. Extend Lane with a private failure-kind inspection. Only a
  failed lane with Timeout/Disconnected is eligible for transport reconnect; validation,
  trust, authentication and malformed/protocol failures remain terminal. A normal request
  timeout/cancellation on a healthy lane does not force reconnect.
- Reuse ReconnectPolicy::next_delay/is_exhausted, Clock::sleep_until and S109's existing
  preparation lock/open_session path. State stays Reconnecting { attempt, next_at }
  throughout backoff and handshake. One public call makes at most one handshake attempt;
  concurrent callers share the preparation lock and recheck the current session. Failed
  retry handshakes advance the policy; exhausted attempts become Failed(Disconnected).
  A complete authenticated handshake resets attempts because it includes valid replies;
  merely opening a socket does not. Known idle expiry retains immediate S109 recovery;
  its failed handshake then enters the same transport backoff policy.
- Caller cancellation, dropped futures, and a caller deadline earlier than the handshake
  bound leave the scheduled attempt available. Backoff/lock waits use the operation
  deadline/token; handshake uses min(operation deadline, connect bound). Keep the stop
  race around preparation, but let the handshake's own timer classify its timeout so an
  equal connect/caller bound cannot bypass attempt accounting. All waits remain bounded.
- Replace private StateCell::set_if with a private conditional replacement that returns
  drained wakers. Callers wake after releasing the session mutex. This permits state and
  current-session changes in the same critical section without invoking arbitrary wakers
  there; it also prevents a late old-session failure from overwriting Connected after
  publication. Public StateCell/StateWatch APIs do not change.
- Replace the duplicated failed-lane state updates in read error diagnostics and Unknown
  writes with one current-session failure observation. Recovery creates fresh session
  identifiers, crypto/identity/trust preparation and an empty metadata cache. Pending old
  reads fail Disconnected on transport loss; old writes remain Unknown and cannot poison
  the replacement. Close stops backoff/handshake and prevents session publication, keeping
  Closed terminal. No re-activation, TransferSubscriptions or background work is added.
- Add focused connection reconnect tests in connection/reconnect_tests.rs, keeping this
  lifecycle concern separate from the existing large read/write and S109 lifecycle test
  files. Cover one/full handshake, fresh authentication tokens/cache, concurrent callers,
  pending/queued operations, Unknown writes, precise backoff/limits/reset, failed open and
  handshake, caller interruption/drop, close, redaction and non-retryable failure. Add
  secure fake-peer recovery to the existing lifecycle tests and exercise the public
  contract fixture's lost-read and ambiguous-write paths followed by a fresh operation.
  Update existing terminal-transport assertions to the new scheduled state.

## Validation and delivery

Commit this plan before implementation, then coherent green implementation/test slices.
Retain 100 percent core/handwritten OPC UA line coverage and existing thresholds and
exclusions. Run full strict check, standalone OPC UA, coverage, feature matrix in
isolation, regeneration/self-tests, dependencies/D23/machete, examples, semver,
parity/self-tests, attributes/DCO, all 18 fuzz targets >=60 seconds and fuzz Clippy/deny,
fresh reviewed package/unpacked builds after doctests, existing 20-test native OPC UA
gate/self-check/example/cleanup, and hosted CI. Mutation selection stays in the existing
pure-code scope; report honestly if S110 has no changed pure-code predicates there.

OPCUA-LIFE-NO-REACTIVATE can close with S110 evidence. OPCUA-OP-RECONNECT and
OPCUA-DIV-RECONNECT retain their planned overall status until the S117 subscription half
is implemented; describe the completed handshake portion explicitly. Final root-only
evidence/handoff changes preserve hosted-tested build inputs. Merge normally with
--no-ff --no-squash --signoff and push master. Do not begin S111 in this turn.
