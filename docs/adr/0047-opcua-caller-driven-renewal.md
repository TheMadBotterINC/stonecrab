# 0047: Caller-driven OPC UA renewal and idle expiry

## Status

Accepted S109 design, 2026-11-04 local, under E19 and ADR-0034. No dependency,
feature, public API, background task, release or version change. S110 owns general
transport-loss reconnect/backoff; S109 recovers known idle channel/session expiry.

## Design

Before service transmission, renew a due token through the existing lane and preserve
its socket, correlation table and monotonically increasing counters. Before a public
operation uses an expired channel/session, repeat the existing driver handshake once
under a bounded preparation lock and publish a fresh session snapshot.

- Add channel-lifetime-ms (3600000) and min-channel-lifetime-ms (5000), positive UInt32
  milliseconds. Reuse numeric parsing. A wire lifetime is UInt32 and cannot be negative;
  negative configuration is rejected. Zero/larger server revisions fall back to requested
  lifetime for compatibility, and the effective scheduling lifetime is raised to the
  configured minimum bounded by the request, with a static warning. Pure Lifetime arithmetic
  uses monotonic elapsed durations and integer 75 percent scheduling rounded up to a
  positive millisecond, matching the pinned upstream without float precision loss. A minimum does not extend a server token's
  actual validity; retain a separate actual expiry and old-token 25 percent grace limit.
- Add concrete Renewal metadata in channel.rs (policy/mode, requested/minimum, token,
  client/server nonces, retained Arc<Security> and Lifetime). Channel transfers this single
  metadata value to lane State instead of duplicating token/nonce ownership. Lifetime lives
  in channel/lifetime.rs to isolate arithmetic and its boundary tests. Security preparation
  returns Arc<Security> for channel/session sharing; no new verifier/backend interface.
- Wire retains asymmetric and symmetric protection together and chooses by frame type.
  OPN remains signed/encrypted even on SIGN. MSG/CLO retain their existing symmetric path.
  Reuse Symmetric.install/retire_previous; add bounded open-body sizing and installation
  entry points. Authentication still precedes service/sequence parsing. No protection toggle
  may make an interleaved MSG use asymmetric keys or an OPN use symmetric keys.
- Lane retains the current channel metadata and one async renewal lock using the already
  available Tokio Mutex (usable with manually polled injected-clock futures). Each normal
  exchange checks renewal before reserving its request id. Recheck under the lock so
  concurrent callers send one Renew. Renew itself uses the same reservation/transport
  lease/correlation/receive machinery, a null authentication token, the configured mode and
  requested lifetime, fresh policy-sized nonce and continued request/sequence counters.
  Pending OPN records its nonce and send time. No request is replayed.
- Decode/verify a Renew response while its receiver owns Wire, then install new keys before
  releasing that owner. Check final OPN, response handle/id/protocol/channel, new nonzero
  token id, exact provisioned certificate/trust and fresh/nonzero/unreflected nonce.
  Interleaved MSG replies continue through normal dispatch. Keep at most one prior token,
  bounded by its original lifetime plus 25 percent grace, and retire it on the first
  authenticated new-token MSG. Reject old/foreign/expired ids and drop retired keys.
  All renewal errors fail the lane with the original typed error; a dropped renewal also
  fails immediately because its epoch is uncertain. Cancellation while merely waiting
  for the renewal lock does not fail another caller's channel.
- Record session activity only when an authenticated session service is sent; OPN renewal
  does not keep a session alive. Session expiry uses the server-adopted timeout and injected
  monotonic time. Connection retains Arc<Session> snapshots behind a short std mutex and
  uses a Tokio Mutex only around idle recovery. Reads/writes keep one snapshot for their
  complete operation, including metadata and staged write; ordinary requests stay pipelined.
  Extract the existing driver handshake into a concrete open_session method and retain the
  driver/config on the connection. Reopen with existing credentials/trust/options and the
  original operation deadline/cancellation bounded by handshake timeout. A new Session
  starts with an empty metadata cache. Close cancels preparation and prevents resurrection.
  Dropping a recovery releases its lock, leaves Reconnecting visible, and permits the
  next call to retry known expiry.
- Keep non-expiry transport failure behavior until S110. Already-sent operations are never
  retried across recovery; writes preserve Unknown after transmission and ordinary errors
  before transmission. Reconnecting/Connected transitions describe idle recovery; no work
  occurs without a caller. Invalid local requests must still perform no network I/O.
- Extend deterministic responders for Renew/new tokens/nonces, overlap, interleaved replies,
  malformed/refused/dropped renewal, concurrency, cancellation, partial sends, exact 75
  percent boundaries and idle channel/session recovery. Native exact-count tests exercise
  three short lifetimes on modern secure policies, None renewal and idle session recovery.
  Existing 17 native cases remain. No generator closure or upstream edit is needed.

## Validation and delivery

Commit this plan first, then coherent implementation/test increments. Retain 100 percent
handwritten core/OPC UA lines and existing coverage/mutation exclusions. Run full check,
standalone OPC UA Clippy/tests, coverage, targeted mutations, all 18 fuzz targets >=60 s
plus fuzz Clippy/deny/policy-copy comparison, isolated 59-feature matrix, regen/self-tests,
dependencies/D23/machete, examples, semver, parity/attributes/DCO, reviewed fresh package
and unpacked builds, native build/self-check/tests/cleanup, and hosted CI. Feature tooling
runs in isolation; fresh package follows doctests. Final root-only evidence/handoff notes
preserve hosted-tested inputs, followed by normal --no-ff --no-squash --signoff merge and
push to master. Do not start S110 in this turn.

## References

The pinned PLC4X SecureChannel.java renewal/lifetime behavior is a comparison, not copied
source. OPC Foundation [OpenSecureChannel](https://reference.opcfoundation.org/specs/OPC-10000-4/5.6.2),
[parameters](https://reference.opcfoundation.org/specs/OPC-10000-4/5.6.2.2) and
[UA TCP mapping](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.4) were checked:
renew at 75 percent, retain prior-token grace, keep sequence numbers across tokens, verify
certificates/nonces again, and renew None channels too. No independent sequence-wrap or
hardware claim is added.
