# 0005: CI gates, per-crate coverage tiers and the single-maintainer waiver
## Status
Amended by ADR-0018: the per-crate coverage tiers below are enforced per module of the
single crate `stonecrab`, with the same thresholds (`scripts/coverage-gate.sh`); the
text is the original decision.
Accepted by the sole maintainer on 2026-10-27, with plan P5.
It affects `.github/workflows/ci.yml`, `mise.toml`, `scripts/` and `.config/nextest.toml`,
amends ADR-0001's coverage clause, CODING_STANDARDS 5.1, 13.2, 13.10, 18.6, 19 and
Appendix A, and implements docs/ALPHA_PLAN.md decisions D7 and D21.

## Context
ADR-0001 required 100% executable handwritten line coverage for the initial slice and
said to "measure each crate independently as soon as multiple crates contain executable
logic". The workspace now has three crates and CI still runs one workspace aggregate
(`cargo llvm-cov ... --exclude-from-report stonecrab-testkit --fail-under-lines 100`).
That command cannot express the tiers the alpha needs: socket code in
`stonecrab-transport` and the non-pure modules of `stonecrab-modbus` have OS-dependent
error arms (the real-socket connect-timeout arm is the first) that no portable test
reaches. It also hides which crate regressed. CODING_STANDARDS 13.10 excludes `tools/`
from coverage and names `xtask` helpers that do not exist; 19 lists CI jobs that are not
yet defined. The repository has one maintainer, so the two-approval rule of 18.6 cannot
be met.

## Decision
1. **Per-crate measurement.** `scripts/coverage-gate.sh` makes one instrumented
   `cargo llvm-cov nextest --locked --workspace --all-features --no-report` run, then
   reads it with `cargo llvm-cov report`: one `--lcov` export to `target/coverage.lcov`,
   one text summary (region coverage is reported there and not gated), and one
   `report --fail-under-lines N` pass per crate from a threshold table in the script.
   A pass selects its crate by adding every other crate's directory to
   `--ignore-filename-regex` (the Rust regex engine has no look-ahead). Every pass
   excludes `(^|/)(examples|benches|fuzz|generated)/`. This replaces the workspace
   aggregate command in ci.yml and mise.toml. `cargo test --locked --workspace
   --all-features` stays as a second step because nextest does not run doctests.
2. **Thresholds (lines).**

   | Scope | Minimum |
   | --- | --- |
   | core, codec, driver-sim and sensors crates (`stonecrab-` prefix) | 100% |
   | `stonecrab-modbus` modules `address`, `pdu`, `framing`, `batch`, `decode` | 100% |
   | `stonecrab-transport`, whole `stonecrab-modbus` crate | >= 95% |
   | `stonecrab-testkit` | >= 90% |
   | a crate with no executable lines (the facade) | `n/a`, skipped |

   Today the script holds `stonecrab-core` 100, `stonecrab-testkit` 90 and
   `stonecrab` n/a; later slices add rows when their crates land. On the current tree
   core is 969/969 lines and the testkit 23/23. The 95% tier supersedes ADR-0001's
   blanket 100% requirement for `stonecrab-transport` and the non-pure modules of
   `stonecrab-modbus`: every uncovered line must be named in the PR as an OS-dependent
   error arm, the connect-timeout arm of the real socket being the first named one.
   The testkit's 90% is a recorded override of 13.10's exclusion of `tools/`, because
   its doubles drive the correctness of every other crate's tests. Error paths are never
   excluded from coverage and `// covers:` tags stay required regardless of the number.
   The gate runs on Linux only. Changed-lines >= 95% is a review rule applied to the
   `target/coverage.lcov` artifact, not a script check. Region coverage is reported and
   not gated; `--branch` on stable 1.99.1 is unverified and not enabled.
3. **Empty-set behavior (verified on this tree with cargo-llvm-cov 0.9.1).** The facade
   holds only `pub use`, so it has no instrumented lines: it is absent from the full
   report and its lcov export is empty. `cargo llvm-cov report --fail-under-lines 100`
   with an ignore regex that leaves no files prints an empty table (`TOTAL` all zero)
   and exits 1 with nothing on stderr; the same command without `--fail-under-lines`
   exits 0. A gated pass over an empty set therefore fails closed, which is right for a
   crate whose tests did not run but wrong for the facade. The script computes per-crate
   executable lines from the lcov file, lists the facade as `n/a` and never passes it to
   `report --fail-under-lines`. It fails if an `n/a` crate gains executable lines, so
   a future logic line in the facade forces a threshold decision, and it fails on any
   crate directory without a table row.
4. **nextest.** `.config/nextest.toml` is Appendix A verbatim (`default` and `ci`
   profiles, 60 s slow-timeout with `terminate-after = 2`, no retries); it parses and
   runs under the pinned cargo-nextest 0.9.148. CI uses nextest as its runner with the
   `ci` profile, and `cargo test` must also pass (13.2).
5. **Timing and OS cells (D21).** Deadline, late-reply, connect-timeout, resync and
   reconnect timing are tested on `FakeTransport`, `FakeTransportFactory` and
   `FakeClock`. Loopback `ScriptedPeer` tests cover bytes only (fragmentation,
   coalescing, truncation, close, refused connect), because Tokio paused time does not
   control socket readiness and a listening loopback socket completes the handshake in
   the kernel backlog. The macOS and Windows test cells stay blocking. A cell with
   socket tests may become `continue-on-error` only with an issue link and a STATUS.md
   note.
6. **Single-maintainer waiver (P5).** CODING_STANDARDS 18.6 requires two approvals for
   changes to CI workflows, the hardware runner, release tooling and bench-executed
   code, and forbids authors approving their own PRs. While the project has one
   maintainer that rule is waived: the maintainer merges their own PRs. The waiver
   expires when a second maintainer with merge rights exists; from that day 18.6
   applies unchanged and this ADR is superseded on this point. Until then every PR
   carries a written self-review against the Appendix C reviewer checklist in its
   description, and STATUS.md carries a matching line. The waiver does not relax any
   automated gate, DCO sign-off, or the hardware-bench review that SPEC.md requires.
7. **lint-attrs.** `scripts/lint-attrs.sh` (run as `mise run lint-attrs` and by the
   `lint-attrs` job) replaces the `xtask lint-attrs` that 5.1 names; there is no xtask
   crate. It is table-driven over `crates/*/src/lib.rs` and `tools/*/src/lib.rs` and
   fails on any `allow`, `warn`, `deny`, `forbid` or `expect` crate attribute other than
   `#![forbid(unsafe_code)]`, `#![cfg_attr(docsrs, ...)]` and the crate's permitted
   `#![deny(...)]` promotions: core `missing_docs`, `as_conversions`; codec, driver-sim
   and modbus all four of `missing_docs`, `indexing_slicing`, `arithmetic_side_effects`,
   `as_conversions`; transport `missing_docs`, `indexing_slicing`; facade and sensors
   `missing_docs`; testkit none. A present crate without a row fails. The script has a
   `--self-test` mode over fixture trees in `scripts/testdata/lint-attrs/`.
8. **geiger.** cargo-geiger 0.14.0 builds and runs on the pinned 1.99.1 toolchain, but
   it cannot gate: it exits 1 whenever any dependency contains `unsafe` (`bytes` does)
   and it rejects a virtual workspace manifest. The blocking `geiger` job is therefore
   the substitute grep, which fails on any `unsafe` word under `crates/` and `tools/`
   outside `forbid(unsafe_code)` lines; cargo-geiger runs as a non-blocking report of
   the facade's dependency tree.
9. **Other jobs and tools.** ci.yml adds `concurrency` (per ref, cancel in progress),
   `workflow_dispatch`, a weekly `schedule`, `dtolnay/rust-toolchain` pinned to the
   commit behind its `v1` tag (that repository's `1.x.y` refs are branches, not tags),
   and the jobs `dco`, `features`, `machete`, `lint-attrs`, `beta` and `semver`
   (the last two report only) and a `scheduled` skeleton whose steps are no-ops until
   fuzz targets and `mutants.toml` exist. `dco` checks out full history and runs
   `scripts/dco-check.sh` over the pull request base..head range, or on push over
   before..sha, skipping an all-zero `before`; root commit 97a5a2d is not rewritten and
   never falls in a range. A push whose `before` commit is missing from the clone (a
   force-push) is skipped with a notice instead of failing. Third-party actions float on
   a major tag, and the report-only `semver` job installs cargo-semver-checks unpinned;
   both are accepted until the first tag. mise.toml pins cargo-nextest 0.9.148,
   cargo-hack 0.6.45 and cargo-machete 0.9.2 beside the existing tools.
   The `clippy::wildcard_enum_match_arm` question is recorded when S11b-1 lands.

## Consequences
Each crate's regression is visible by name and the transport and Modbus socket arms no
longer force coverage exclusions, at the cost of a reviewer-enforced list of named
uncovered arms. The script owns the table, so a new crate cannot merge without a row in
the coverage and lint-attribute tables. The features pass cannot use `--locked` because
`cargo hack --no-dev-deps` edits manifests while it runs. cargo-hack and cargo-machete are
not in mise's aqua registry and mise builds them from source; CI installs prebuilt
binaries with `taiki-e/install-action`. Single-maintainer merges carry no independent
review until the waiver expires, which is accepted and recorded rather than hidden.

## Alternatives considered
- Keep the workspace aggregate at 100%: rejected, it forces coverage exclusions on
  OS-dependent error arms, which 13.10 forbids.
- Use a flat 90% everywhere (13.10 as written): rejected, it lets the pure core and
  codec modules slip below the 100% they have today.
- Pass the facade to `report --fail-under-lines` and tolerate exit 1: rejected, it
  cannot distinguish "no logic" from "tests did not run".
- Write `xtask` crates for `coverage-gate` and `lint-attrs`: rejected, shell scripts
  have no dependencies and keep the workspace to the crates it ships.
- Gate on cargo-geiger output: rejected, see Decision 8.

## Compliance
`mise run coverage` and the CI `quality` job run `scripts/coverage-gate.sh`; the
`lint-attrs`, `dco`, `features`, `machete` and `geiger` jobs enforce Decisions 7 to 9;
reviewers apply the changed-lines rule and the named-arm rule from the lcov artifact.
This ADR overrides ADR-0001's blanket coverage clause for transport and modbus,
CODING_STANDARDS 13.10's `tools/` exclusion and `xtask` names, 5.1's `xtask lint-attrs`
and 18.6's two-approval rule while the waiver stands.

Addendum (S11b-1, 2026-10-27): `clippy::wildcard_enum_match_arm` (warn, root `Cargo.toml`)
does fire on a `_` arm over a foreign `#[non_exhaustive]` enum (`PlcValueType`, `PlcValue`
from `stonecrab-core`) under the pinned 1.99.1 toolchain when the `_` stands for known
variants. It does not fire when every known variant is named and `_` is only the catch-all
for future variants, nor on a tuple scrutinee such as `match (value, step)`. Sibling crates
therefore write one final or-pattern arm that names the unsupported known variants and ends
with `| _` and returns a typed error (`Null | Char | String | Duration | _ => Err(..)`), and
a test per named variant executes that arm, so the 100% line gate holds with no
`#[expect]`. Details and the probe results are in ADR-0012, decision 7.

Addendum (S31, 2026-10-27): the parity gate. `scripts/parity-check.sh` (`mise run parity`,
the CI job `parity`, and `--self-test` over the fixture trees in
`scripts/testdata/parity-check/`, in the style of `lint-attrs.sh`) joins `lint-attrs`,
`dco` and `features` as a blocking job. It replaces the parity half of the `xtask
release-check` that CODING_STANDARDS 20.3 names ("parity records have no `implemented`
status without a test id"), for the same reason as Decision 7. A row id is the first cell
of a table row in `docs/parity/*.md`, an upper-case id of two or more hyphen-joined groups
(`MB-TCP-CONNECT`); the files use no other form, so headings are not ids. The script fails,
printing file, line and id for every finding, when:

- (a) a `docs/parity/<file>.md#<ROW-ID>` reference in `crates/*/src/**/*.rs`, which
  covers every `# Compatibility:` rustdoc section and any prose cross-reference of the
  same shape, names a file that does not exist or an id that no row of that file carries;
- (b) a row whose status starts with `implemented` or `automated-verified` is named
  nowhere, as a bare id, in `crates/*/src/**/*.rs`, `crates/*/tests/**/*.rs` or
  `tools/stonecrab-testkit/src/**/*.rs`;
- (c) such a row's test-id column holds no backticked test id (the S31 exit criterion "no
  `implemented` row without a test id"), or a table row has a different number of cells
  than its header.

Divergence tables carry no status column and are not subject to (b) and (c); their ids
repeat the ids of the behavior rows they qualify, so (a) resolves them. Rows that are
`deferred` or `not reproduced` are exempt from (b). The first run found 11 `implemented`
rows that nothing named (`CORE-BYTEORDER-MODEL`, `CORE-VALUE-ORD`, `CORE-OPFUTURE`,
`CORE-CLOCK`, `CORE-CANCEL-TOKEN`, `CORE-STATE-WATCH`, `MB-INTEROP-HR-02`, `-03`, `-06`,
`SIM-CONTRACT-SUITE`, `TRANSPORT-STREAM-DELIVERY`); each was fixed by naming the row in the
rustdoc or the test it describes (comment-only edits), and the check was not changed. What
the gate does not do: it does not prove that a named test exists or passes (the `test`
jobs run the tests, and S31 checked once, by a throw-away script, that every backticked
snake_case test id in the records exists as a `fn`); and a name in a comment satisfies
(b), so review still judges whether the cited test verifies the row.

Addendum (S35, 2026-10-28): the `semver` job is blocking. Decision 9 accepted it as report
only, with cargo-semver-checks unpinned, until the first tag; `stonecrab` 0.2.0 is now
published on crates.io and is the baseline. The job drops `continue-on-error` and pins
cargo-semver-checks 0.51.0, so a new lint release cannot fail an unchanged tree. With the
`version` in `Cargo.toml` unchanged the tool checks a minor release (its message is "no
change; assume minor"), so additive changes pass and a breaking change fails until the
version bump that announces it, as CODING_STANDARDS 9.10 requires. The first blocking run
is recorded in STATUS.md. Third-party actions still float on a major tag.
