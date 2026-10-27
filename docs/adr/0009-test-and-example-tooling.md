# 0009: Test and example tooling
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S03b of docs/ALPHA_PLAN.md.
It records policy only: no dependency entry is added to the root manifest here. It
implements decision D23 for the test crates named below and builds on ADR-0005.

## Context
Several upcoming slices need test-only crates (property testing, golden-vector loading,
an XML reader for the upstream driver suite) and a fuzz workspace. The repository bans
`once_cell`, `lazy_static`, `log`, `time` and `chrono` (deny.toml, D23), and
`cargo-machete` runs in CI, so a dependency declared before its first consumer fails the
`machete` job. The choices and feature sets therefore need to be settled once, before
the first consumer lands.

## Decision
1. **Sanctioned test and example crates.** Each is added to the root
   `[workspace.dependencies]` as a minor-level caret range (CODING_STANDARDS 15.4), with
   its DEPENDENCY_AUDIT entry and the D23 `cargo tree -e normal,dev -i once_cell -i
   lazy_static -i log -i time -i chrono` check, in the slice that first consumes it. That
   keeps `machete` green. This ADR adds none of them.

   | Crate | Features | First consumer | Notes |
   | --- | --- | --- | --- |
   | `proptest` | `default-features = false`, `std` | S08 or S15, whichever lands first | `fork` and `timeout` are prohibited: they pull `tempfile` -> `once_cell`. Its `std` feature may enable `lazy_static`; UNVERIFIED for the version resolved at PR time, and the first consumer applies D23 and records the result. Slow property tests are bounded by nextest's slow-timeout, not by proptest's `timeout`. |
   | `serde` | `derive`, `std` | S04a | Unpublished testkit golden loader only. |
   | `serde_json` | `std` | S04a | Same. |
   | `roxmltree` | `default-features = false`, `std` | S04b | MIT OR Apache-2.0 per the defect report; its `forbid(unsafe_code)` status is recorded in the audit entry when it lands. Used only behind the testkit feature `fixtures`; never reaches a published crate. |

2. **Scope of the testkit.** `stonecrab-testkit` (under `tools/`) is unpublished and is
   the only crate that carries these loaders and adapters. Published crates may use
   `proptest` as a dev-dependency but never `serde`, `serde_json` or `roxmltree`. The
   testkit's line coverage threshold is 90% (ADR-0005), enforced by
   `scripts/coverage-gate.sh`; it stays excluded from `cargo package`.
3. **Property test volume.** CI sets `PROPTEST_CASES=256` as `env` on the `test` and
   `quality` jobs of `.github/workflows/ci.yml`. Regression files written by proptest are
   kept: `proptest-regressions/` directories are tracked in git, and `.gitignore` does
   not ignore them. CODING_STANDARDS 13.3 mentions 4096 cases on the scheduled run; the
   plan asks only for 256, and because the `test` and `quality` jobs also run on the
   schedule they get 256 there too. A 4096 override for the scheduled run is deferred.
4. **Fuzz layout.** `fuzz/` is its own Cargo workspace: the root `[workspace]` has
   `exclude = ["fuzz"]`, and the fuzz package (`stonecrab-fuzz`, `publish = false`) has
   its own `[workspace]` table, `Cargo.lock` (committed), `rust-toolchain.toml` and
   `deny.toml`. This keeps the nightly-only toolchain, the native libFuzzer build and
   their dependencies out of the root lock and the root `deny`, `machete` and `package`
   runs.
   - `libfuzzer-sys` (0.4) and `arbitrary` (1.5) are declared in `fuzz/Cargo.toml`, not
     in the root `[workspace.dependencies]`, with DEPENDENCY_AUDIT entries. The fuzz
     package depends on first-party crates by path; each slice that adds a target adds
     the path dependency it needs.
   - `fuzz/rust-toolchain.toml` pins the nightly by date: `nightly-2026-10-01`
     (rustc 1.101.0-nightly, 2026-09-30). CI passes the same date to
     `dtolnay/rust-toolchain`; bumping the date means changing both and amending this
     ADR. `cargo-fuzz` 0.13.2 is the pinned driver.
   - `fuzz/deny.toml` is a copy of the root `deny.toml` because `cargo-deny` 0.20 has no
     include mechanism. Its one permitted difference is `"NCSA"` appended to the license
     allow list, because `libfuzzer-sys` is licensed `(MIT OR Apache-2.0) AND NCSA`
     (vendored libFuzzer), so NCSA is required, not optional. `scripts/fuzz-smoke.sh`
     fails when the files differ in anything else, so the copies cannot drift unnoticed.
   - Targets are named `fuzz_<crate>_<parser>`; seed corpora live in
     `fuzz/corpus/<target>/` and crash inputs follow CONTRIBUTING "Fuzz findings".
     `fuzz/target` and `fuzz/artifacts` are ignored.
5. **Fuzz smoke job.** CI `fuzz-smoke` (also run on `workflow_dispatch`) and
   `mise run fuzz` both run `scripts/fuzz-smoke.sh`, which runs
   `cargo deny --manifest-path fuzz/Cargo.toml check` first, then every target for 60 s;
   with zero targets it prints that and exits 0. The weekly `scheduled` job runs
   `scripts/fuzz-smoke.sh 1800`. The plan's filter "only targets whose crate changed" is
   deferred until targets exist: every target runs on every invocation. For the same
   reason `workflow_dispatch` runs all targets, which equals a normal run today.
6. **Out of scope.** This ADR makes no statement about `tokio` or `tracing-subscriber`;
   ADR-0014 (S16, S14) decides them.

## Alternatives considered
- **honggfuzz or AFL instead of cargo-fuzz.** Rejected: libFuzzer through `cargo-fuzz` is
  the most widely used Rust setup and needs no extra system tooling beyond a C++ compiler.
- **Fuzz dependencies in the root workspace.** Rejected: they would put a nightly-only
  build and a native libFuzzer dependency into the root lock and the root `deny`,
  `machete` and `package` runs.
- **A per-crate `cargo-deny` license exception in the root `deny.toml`.** Rejected: the
  root run does not see the fuzz workspace, and the entry would loosen the root policy
  for a crate it never resolves. A separate policy file with a drift check keeps the
  exception next to the code that needs it.

## Compliance
The `fuzz-smoke` CI job and `mise run fuzz` run `scripts/fuzz-smoke.sh`, which fails when
`fuzz/deny.toml` differs from `deny.toml` in anything but the NCSA entry and runs
`cargo deny --locked` on the fuzz workspace.

## Consequences
- The root manifest changes by one line (`exclude`), and the root `Cargo.lock` does not
  change.
- The fuzz workspace needs a C++ compiler and `cargo-fuzz` locally; a missing nightly is
  installed by rustup on first use.
- The plan sentence "fuzz/deny.toml includes the root policy" holds only with the extra
  NCSA allowance; the plan wording should be amended to say so.
- Each consumer slice owns its own D23 check and audit entry.
