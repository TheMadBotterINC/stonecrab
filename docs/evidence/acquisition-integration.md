# Acquisition branch integration evidence

Measured 2026-11-02, Linux x86_64, Rust 1.99.1. Source branch:
`feat/sensors-continuous-acquisition` at `f8aedbf`; integration baseline:
master `0f6eb88` (0.3.0, through S87).

## Conflict resolution

The branch started from the earlier 0.2.0 tree. Current master independently implements
Acquisition/StaleTracker (ADR-0023), native subscription delivery and polling
(ADR-0020/0021), and EventPump (ADR-0031), along with newer protocol work.
All published library sources remain byte-for-byte unchanged from the integration
baseline. Current version metadata, feature declarations, parity records, architectural
decisions and reviewed package list are retained. The branch's competing sensor types,
conflicting ADR-0019 and obsolete evidence/status claims are discarded from the squash.
Its original six signed-off commits remain on the original feature branch locally.

The finite `profile_poll` example is adapted to the existing `Acquisition::recv` API and
optional units. It shares the existing synthetic bench with `profile_read`. The new
virtual-day test reuses the existing Rig in `tests/sensors_acquisition.rs`. CI and mise
exercise every synthetic simulator profile for 2200 ms, and the pymodbus profile step
runs the same finite example against each synthetic Modbus profile.

No API, dependency, version bump or alternate runtime layer is added. A finite deadline
can drop a pending receive with the published cancellation consequences. Profile
acquisition remains caller-driven and exposes observations/errors, rather than the
original branch's separate queue-policy and missed-tick counters. Stale notification
while an in-flight read is pending is not claimed; master's existing loop awaits the
read before delivering subsequent events. Original stale-threshold and interval
semantics are retained.

## Local verification

| Check | Result |
| --- | --- |
| `mise run check` | Passed: format, strict Clippy for all targets/features, nextest, tests/doctests, warnings-denied docs and minimal build |
| Nextest | 2115 passed, 74 external-peer tests skipped in the normal suite |
| Doctests | 583 merged and 5 separately executed passed |
| `scripts/coverage-gate.sh` | All existing component thresholds passed unchanged |
| Sensors line coverage | 1840/1840, 100% |
| Other line coverage | Core, codec, sim, Modbus, boards, OPC UA 100%; transport 99.31%; testkit 97.66% |
| `mise run examples` | All original examples and all three finite continuous simulator profiles passed |
| `interop/pymodbus/interop.sh run` | 16/16 tests passed; contract suite 17 passed, 16 explicitly skipped; all one-shot and finite continuous profiles passed |
| Continuous pymodbus measurements | Discrete: 24 observations / 8 names; process: 9 / 3; temperature: 6 / 2; no non-good observations or read failures |
| Parity and lint attributes | Passed; 252 implemented/automated-verified rows resolve |
| Package file list | Matches the existing reviewed list; no library source or packaged path added |
| Interop shell syntax / diff whitespace | Passed |

The virtual-day regression makes 86,401 one-second reads of each of two measurements
(172,802 delivered observations including time zero), checks exact read counts and
monotonic times, and verifies no timer is retained after each receive or shutdown.
It is a deterministic scheduling/resource test, not a real-time or physical endurance run.

Local output: `/tmp/stonecrab-merge-check.log`,
`/tmp/stonecrab-merge-coverage.log`, `/tmp/stonecrab-merge-examples.log`,
`/tmp/stonecrab-merge-interop.log`, and `target/coverage-summary.txt` /
`target/coverage.lcov`. Independent-server output, peer identity and server logs are
under `target/interop-evidence/`; finite output is `profiles-poll.txt`.
These are local artifacts. Hosted CI is triggered by pushing the integration commit.
No physical equipment or manufacturer profile is verified, and no package is published.

Self-review under ADR-0005's single-maintainer waiver: current contracts and lower-level
state machines are reused; no competing public API survives the reconciliation; the
shared fixture has two real consumers; existing read cancellation and explicit close
remain the ownership boundary; the new example/test and CI commands passed locally.
Coverage policy and protocol support claims are unchanged. Hardware commissioning is a
procedure with an empty evidence requirement, not a hardware-support claim.
