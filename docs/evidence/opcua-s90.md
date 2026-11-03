# S90 policy-None channel evidence

Measured 2026-11-02–03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch `feat/opcua-none-channel`,
baseline master `d5ef972` (S89).

The private channel owns HEL/ACK and OPN Issue, then either sends CLO without waiting
for a reply or hands the framed transport to the existing lane. It validates the policy,
protocol, request/handle, channel/token, sequence and lifetime; adopts the exact returned
token/lifetime/nonce; and carries counters forward. Selecting this path requires None
explicitly. Its wire mode is None, client nonce/certificate/thumbprint are null and no
signing/encryption or keystore is used. Public sessions/connect follow in S91, Read in
S92, reassembly in S94 and secure policies/renewal in their later slices.

Limits are directional: client receive merges with server send, and client send with
server receive. Zero means unlimited for message/chunk bounds; non-ECC buffers are at
least 8192 bytes. The receive-buffer option defaults to 65535 and is capped by resources
before HEL advertises it. Framer replacement preserves coalesced bytes and separate
encode/decode ceilings. The lane applies negotiated body bounds before encoding or
decoding. Shared pure service functions move from the lane into `opcua/service.rs`;
their old definitions are removed. No task, runtime dependency or public driver is added.

Thirty deterministic channel tests plus two parser tests cover wire fields, asymmetric
buffer limits, zero limits, counter handoff, malformed/refused/cancelled/expired setup,
coalesced input, status sources, local/message-size refusal, configured mode/receive
buffer and CLO without a response. A delayed ACK consumes two seconds of a five-second
deadline: OPN carries a 3000 ms hint and the silent peer times out at five seconds total.
Request handle and protocol-version mismatches are explicitly rejected.

| Check | Result |
| --- | --- |
| `mise run check` | Passed: format, strict all-feature Clippy, tests/doctests, warnings-denied docs and minimal build; final checked-tree totals recorded below |
| Nextest | 2191 passed; 75 external-peer tests skipped in the normal suite |
| Doctests | 583 merged + 5 separate passed |
| Coverage | OPC UA 5877/5877, 100%; core, codec, sim, Modbus, sensors and boards 100%; transport 2877/2897 (99.31%), testkit 5266/5391 (97.68%); floors unchanged |
| Feature powerset | All 59 combinations and no-default-feature check passed; default strict Clippy and transport-only check also passed |
| Fuzz smoke | All 14 targets passed, 10 seconds each; receive-buffer success/zero seeds retained and the connection-string oracle validates the configured value |
| Mutation campaign | Channel negotiation/budget helpers: 12 caught, 1 unviable; 5 additional struct-field mutations emitted elsewhere, all caught; no survivors/timeouts |
| Semver against published 0.3.0 | Default and all-features passed; no update required |
| Fuzz-only strict Clippy | Passed under `RUSTFLAGS='--cfg fuzzing'` |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Parity / lint attributes | Passed; 268 implemented/automated-verified rows resolve |
| Package | Verified archive build; reviewed 186-file list matches; 763,908 compressed bytes (3.2 MiB unpacked) at `3725efe`; external responder, fixtures and fuzz data excluded |

The first mutation baseline failed because it copied Cargo.toml while `mise run features`
temporarily removed dev dependencies. No mutants ran in that attempt. Features were
allowed to finish; the campaign was rerun against the restored tree with `--gitignore
true` and passed. Keep feature-matrix execution separate from metadata/package/mutation
work. Results: `/tmp/stonecrab-s90-mutants-stable/mutants.out/`. Package validation also
caught an unused helper in the default build; the negotiated-framer method is now gated
by `opcua`, and the default build is warning-free.

## Independent peer

`interop/open62541/interop.sh run` passed the image's own checks and the exact-count
Stonecrab test. The final `tests` run selected exactly one ignored unit test,
`opcua_interop_peer_answers_hello_and_opens_a_none_channel`, and passed. It exchanges
HEL/ACK and OPN Issue, sends CLO without awaiting a reply, then opens another channel
and hands it to the lane. A request without a session receives `BadSessionIdInvalid`
(0x80250000), proving that continued counters reach the server's session check.
The requested lifetime was 3600000 ms, revised to 600000 ms and retained exactly.

- Peer: open62541 v1.5.9, pinned source `05c2b389b65cc35884eb40d800301af1f3d130cf`.
- Image: `sha256:f4ebdd0a43ba35d5eb6f888c23de0a56adebaf2c878e07ec7fc84ae61c200d62`.
- Platform: linux/amd64; OpenSSL 3.5.7, Debian package `3.5.7-1~deb13u3` inside the image.
- Evidence: `target/interop-evidence/open62541/{image-digest.txt,check.txt,nextest.txt,server.log}`.
- The container was stopped and copied test PKI files removed. Nothing was uploaded or
  published by the local run. The recipe/provenance and CI now include the exact-count
  native test; the peer's own secure-policy checks describe its scaffold, not Stonecrab's
  future secure client.

Local logs: `/tmp/stonecrab-s90-{quality,coverage,features,fuzz,mutants,semver,regen,
package,default-clippy,fuzz-clippy,interop-full,interop-final}.log`.
No physical hardware was used and no crate was published. Hosted CI is triggered by the
push; no hosted result is assumed in this local evidence.

Self-review under ADR-0005's single-maintainer waiver includes the CI change: the job
uses existing pinned tool versions, selects exactly one test, keeps cleanup/evidence
steps on failure, and uploads only the established evidence directory. The channel and
lane have one transport owner per phase, no background task or replacement runtime.
Tests cover counter continuity, preserved buffers and directional bounds; configured
and effective security modes are shown without nonce/certificate bytes. Six additional
implemented parity rows describe private channel behavior. The public connection and
session/read capability claims remain planned.
