# S91 public anonymous OPC UA session evidence

Measured 2026-11-03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch
`feat/opcua-session-connect`, baseline master `3cb5274` (S90).

Public `OpcuaDriver` follows the existing clock/factory injection pattern and explicit
`DriverRegistry` registration. An explicit policy None connection performs TCP, HEL/ACK,
OPN Issue, CreateSession and anonymous ActivateSession under one connect deadline.
Connected is returned only after successful activation. The secure default and supplied
credentials fail before opening the transport. This slice advertises no data operations;
Read remains S92, full endpoint selection/discovery S96, security/renewal later slices.

CreateSession sends the Stonecrab Client ApplicationDescription and a fresh 32-byte OS
nonce, separate from the policy-None channel nonce. Optional getrandom 0.4.3 uses no default
features and is enabled only by opcua; std has no portable secure entropy API, and jitter
is unsuitable. Its unsafe OS boundary and dependency justification are reviewed in
DEPENDENCY_AUDIT.md and ADR-0034. Tests inject the nonce bytes at the private session
boundary, without adding a one-implementation RNG trait. A fill error has no fallback.

Handshake timeout defaults to 60000 ms and maps to Timeouts.connect, covering the entire
attempt; session timeout defaults to 120000 ms. The authentication token and positive,
finite, representable revised timeout are adopted. Activation uses an anonymous policy id
advertised on a None/None UA TCP endpoint. More detailed endpoint matching and overrides
remain S96. Session setup uses the absolute connect bound rather than the ordinary lane
request-timeout cap. Supported response handles are checked against the wire request id.

CloseSession uses the authentication token, deleteSubscriptions=true and timeoutHint
50000, then CLO continues the same lane counters with a null token and no response wait.
The whole shutdown uses request timeout. Guards release the lane and publish Closed on
success, failure, timeout or drop. Concurrent/repeated closes are tested; a second close
also has a deadline if the first future is parked. The obsolete Channel.close path is
removed and its tests use Lane.close. Reads/writes return typed Unsupported while open,
Disconnected after close, and capability flags remain false.

## Local checks

| Check | Result |
| --- | --- |
| `mise run check` | Passed: fmt, strict all-feature Clippy, nextest/tests/doctests, warnings-denied docs and no-default-feature build |
| Nextest | 2211 passed; 76 external-peer tests skipped in the normal suite |
| Doctests | 584 merged + 5 separate passed |
| Coverage | OPC UA 6259/6259 (100%); core 6657/6657, codec 245/245, sim 2303/2303, Modbus 6699/6699, sensors 1840/1840, boards 511/511; transport 2877/2897 (99.31%), testkit 5265/5391 (97.66%); floors unchanged |
| Features | All 59 combinations and no-default-feature check passed; run separately from other Cargo work because cargo-hack temporarily rewrites the manifest |
| Default / fuzz-only Clippy | Passed, including `RUSTFLAGS='--cfg fuzzing'` all-feature/all-target checks |
| Semver | Default and all features against published 0.3.0 passed; no update required |
| Dependencies / machete | Advisory, license, source and ban checks passed; no unused dependency |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Fuzz smoke | All 14 targets passed, 10-second budget each (11 seconds reported by libFuzzer); the updated connection-string target completed 751977 runs |
| Mutation campaign | Configuration/options, nonce error mapping and session status check: 26 mutants, 23 caught, 3 unviable, no survivors or timeouts; baseline passed |
| Parity / lint attributes | Passed, including parity self-tests; 275 implemented/automated-verified rows resolve |
| Archive | Verified 189-file package and reviewed list; 774748 compressed bytes at `2d55df3`; unpacked archive also builds with all features |

Deterministic tests cover ordered handshake fields, fixed injected nonce, token/timeout
adoption, bad revisions, absent/incompatible anonymous policies, bad response types and
handles, exact refusal status, silent peers at each handshake stage, cancellation/drop
while creating or activating, invalid configuration before I/O, close message order,
silent/idempotent/concurrent/dropped close, and state watcher notification. Resource/clock
encoding errors fail before sending a close. No first-party unsafe, spawned task, new
public trait, crypto backend, or data-operation support is introduced.

The first fuzz invocations failed before running because the new optional direct
getrandom edge was absent from fuzz/Cargo.lock. An offline metadata resolution added only
that edge; there was no version upgrade. The parser oracle accepts and checks both new
timeout options and a retained seed exercises them together. The final run passed all
14 targets without a crash; version and advisory checks passed in the fuzz workspace.

## Independent peer

The existing image's in-container checks passed. Stonecrab then selected exactly two
ignored tests and both passed:

- `opcua_interop_peer_answers_hello_and_opens_a_none_channel`: channel HEL/ACK/OPN/CLO,
  counter continuity and the expected session-required refusal.
- `opcua_interop_connects_and_closes_with_policy_none`: public driver registered in
  DriverRegistry, anonymous session activation, Connected, clean close, Closed.

Peer: open62541 v1.5.9, source `05c2b389b65cc35884eb40d800301af1f3d130cf`, image
`sha256:f4ebdd0a43ba35d5eb6f888c23de0a56adebaf2c878e07ec7fc84ae61c200d62`.
Evidence: `target/interop-evidence/open62541/{image-digest.txt,check.txt,nextest.txt,server.log}`.
The Docker peer was stopped and host test PKI removed. Its own secure-policy checks
validate the scaffold, not Stonecrab's future secure client. No physical device was used,
no secrets were committed/uploaded, and no crate or tag was published.

Self-review under ADR-0005's single-maintainer waiver covers the code and interop filter:
one transport owner per phase, continued identifiers, separate session nonce, bounded
close, truthful capability flags, no silent downgrade, exact-count selection expanded
from one test to two, and unchanged cleanup/evidence destinations. CI workflow comments
now describe public session verification; the existing steps run the expanded script.
Hosted CI is triggered on push; this evidence assumes no hosted result.

Local logs: `/tmp/stonecrab-s91-{quality3,coverage3,features,dependencies,semver,default,
fuzz3,fuzz-clippy,regen,package,packaged-opcua,mutants,interop-final,peer-check}.log`.
Mutation outcomes: `/tmp/stonecrab-s91-mutants/mutants.out/outcomes.json`.
