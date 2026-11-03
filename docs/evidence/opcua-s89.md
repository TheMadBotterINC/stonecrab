# S89 exchange-lane evidence

Measured 2026-11-02, Linux x86_64, Rust 1.99.1. Branch
`feat/opcua-exchange-lane`, baseline master `dca3094` (S88).

The private lane owns one established policy-None channel above `FramedTransport`.
Request guards bound and release pending entries, including queued and completed
requests. A queued sender interrupts a cancel-safe receive; a transport lease restores
partial-frame bytes on receive cancellation. An armed send lease fails the lane if
interrupted. No task, transport split, dependency or automatic replay is introduced.
The existing effective frame bound is shared by the framed encoder and the lane, so a
local encoding refusal consumes no wire sequence number or transport write.

The 34 new lane tests use first-party byte constructions in
`tests/support/opcua_responder.rs`, FakeTransport and FakeClock. They cover out-of-order
and unknown replies, a second send while the first waits, reentrant wake-ups, queued and
in-flight cancellation, late abandoned replies, retained partial frames, partial-send
drop/timeout, resource limits, foreign channel/token ids, sequence gaps, malformed frames,
ServiceFault and ERR, exact timeout hints and clock failures. Two parser regressions
verify default/custom `request-timeout-ms` and invalid values. Sequence wrap is refused
explicitly pending S106; no request id can wrap into an outstanding slot.

| Check | Result |
| --- | --- |
| `mise run check` | Passed: formatting, strict Clippy, 2159 nextest tests, 74 external-peer tests skipped, 583 merged + 5 separate doctests, warnings-denied docs and minimal build |
| Coverage | OPC UA 5506/5506, 100%; core, codec, sim, Modbus, sensors and boards 100%; transport 2874/2894 (99.31%), testkit 5266/5391 (97.68%); floors unchanged |
| Feature powerset | All 59 combinations and no-default-feature check passed |
| Fuzz smoke | All 14 targets passed, 10 seconds per target, after updating the timeout invariant |
| Focused mutation campaign | Lane reserve/sequence/dispatch: 12 caught, 1 unviable; the tool also emitted 5 struct-field mutations outside the requested lane filter, all caught; 0 survivors/timeouts |
| Semver against published 0.3.0 | Default and all-features passed, no update required |
| Fuzz-only strict Clippy | Passed with `RUSTFLAGS='--cfg fuzzing'` |
| Regeneration | Both protocols byte-identical twice; edited/added/removed-output self-checks passed |
| Parity / lint attributes | Passed; 262 implemented/automated-verified rows resolve |
| Package | Verified archive build; reviewed 183-file list matches; 753,587 bytes compressed in the measured archive at `02cd884` (3.2 MiB unpacked); the external `tests/support` responder, fixtures and fuzz data are excluded |

The default mutation scope continues to exclude I/O. The focused command was
`cargo mutants --config mutants.toml --file crates/stonecrab/src/opcua/lane.rs --re
'^crates/stonecrab/src/opcua/lane.rs:.*Lane::(dispatch|reserve|next_sequence)' --jobs 2`.
Results are retained locally at `/tmp/stonecrab-s89-mutants/mutants.out/`.

The first fuzz smoke attempt exposed an obsolete harness assertion allowing only the
two S84 query keys; the new positive timeout seed triggered it immediately. The harness
now accepts `request-timeout-ms` and independently checks the configured duration and
positive-decimal invariant. Both positive/zero timeout seeds remain in the corpus.
This was a test-oracle failure, not a production-parser panic.

Local output is `/tmp/stonecrab-s89-{quality,coverage,features,mutants,semver,regen,fuzz,
package,fuzz-clippy}.log`; coverage artifacts are under `target/coverage*`.
No independent server or physical equipment was used, and no package was published.

Self-review under ADR-0005's single-maintainer waiver: the existing waker/cancellation
patterns and framed byte ownership are reused; no new generic lock or executor enters
the driver. The pure service encoding and non-generic exchange body are separate to keep
async code size and ownership clear. Wakers, removed replies and failed transports are
dropped outside the state lock. Device reason strings/session tokens are not echoed.
Seven implemented parity rows describe the internal lane only; the overall ERR row
still awaits Hello/connect handling in S90/S91. Public connect, session, Read, multi-chunk
reassembly, secure policies and reconnect remain their later slices.

Integration: normal merge `261b281` preserves the five signed-off branch commits. Its
tree matches the feature branch, and `mise run check` passed again on the merged tree
(`/tmp/stonecrab-s89-merge-check.log`). Hosted CI is triggered by the push; no hosted
result is assumed here.
