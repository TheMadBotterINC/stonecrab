# Alpha release evidence
Recorded for slice S32 on 2026-10-27 (ADR-0017). This file is the evidence the alpha
release checklist of `docs/ALPHA_PLAN.md` asks for. Every number below was measured on the
date and tree stated next to it; nothing is estimated, and what was not run is listed under
"Gates not run". Nothing here is published: no crate is on crates.io beyond the `stonecrab`
0.1.0 placeholder, no tag exists and nothing is pushed by this slice.

Tree measured: `8f5c3d0` plus the S32 changes (the S32 commit adds documentation,
configuration, scripts and the evidence files, plus four test additions that kill mutation
survivors; no library behavior changes).
Machine: Apple silicon macOS, 12 cores; toolchain Rust 1.99.1 (development), fuzz nightly
`nightly-2026-10-01`.

## Pinned tools

| Tool | Version | Where pinned |
| --- | --- | --- |
| cargo-mutants | 27.1.0 | `mise.toml` (`cargo:cargo-mutants`) |
| cargo-cyclonedx | 0.5.9 | `mise.toml`, `.github/workflows/release-evidence.yml` |
| cargo-release | 1.1.7 | `mise.toml` |
| cargo-fuzz | 0.13.2 | `mise.toml` |
| cargo-llvm-cov | 0.9.1 | `mise.toml` |
| cargo-nextest | 0.9.148 | `mise.toml` |

The three S32 tools were installed with `cargo install --locked` (mise's `cargo:` backend
runs exactly that).

## Upstream baseline

| Item | Value |
| --- | --- |
| Project | Apache PLC4X |
| Version / tag | 1.0.0 / `v1.0.0` |
| Annotated tag object | `32a524591841ab56d26b032914ce06918148fbb6` |
| Implementation commit | `ea229918f5d07700e04d7d69ab1645da679cb700` |
| Inventory scope | Modbus TCP read path only (`[inventory.modbus_tcp]` in `upstream/manifest.toml`); `inventory_complete = false`, nine items still pending, none gates the alpha |
| Source-release signature | verified in S32: `signature_verified = true`; see below |

Source-release signature (recorded in `upstream/manifest.toml`, `[signature]`):
`apache-plc4x-1.0.0-source-release.zip` and its detached `.asc` were downloaded from
`downloads.apache.org` together with the project `KEYS` file into a scratch directory
outside the repository. The KEYS file was imported into a temporary GnuPG home (not any
personal keyring), and `gpg --verify` exited 0 with a good signature by the RSA key with
fingerprint `F156813FF315007E36BA6C13089127C15C60D6B9`, made 2026-09-03. The SHA-512 of the
archive as published
(`f654ef368d494bc2c344ff010f5d0db78ff9d1eee9a0588f3fef4ce16597920852f8bd424808f5c2ed02e76b951929fe1736fed34d4c210ad2f4a27b18303cc0`)
equals the one computed locally. Limits: gpg reported the key as not certified (no trust
path in the temporary keyring), so this shows the archive matches a key the Apache PLC4X
project publishes; it does not establish who holds the key. The archive's contents were
not inventoried or compared with the pinned commit, and no archive content is stored in
the repository.

## Interoperability evidence

| Item | Value |
| --- | --- |
| CI job | `interop` in `.github/workflows/ci.yml` (ubuntu-latest, every push and pull request); `mise run interop` runs the same script locally (needs Docker) |
| Artifact | `interop-evidence`: `image-digest.txt`, `pip-freeze.txt`, `server.log`, `contract-report.txt`, `nextest.txt`, `example.txt` (ADR-0015 decision 16) |
| Peer | pymodbus 3.8.6 (`interop/pymodbus/requirements.txt`), first-party `server.py` over `fixtures/modbus/interop/register-map.toml` |
| Image | `python:3.12-slim@sha256:12316debbfb86890f6b2d825820612741ba527ad5a44e3902759141d572bfe8f` (the `FROM` line of `interop/pymodbus/Dockerfile`) |
| Hosted runs | S30 `f19be4c`: https://github.com/TheMadBotterINC/stonecrab/actions/runs/40548582722 ; S27 `0429da3`: https://github.com/TheMadBotterINC/stonecrab/actions/runs/40543567845 |
| Interop tests | ten `interop_` tests plus the `modbus_tcp_read` example and `profile_read --expect-good` for each `modbus-tcp` profile |

Contract suite (`ContractSuite`, 23 cases) per fixture, as recorded in
`docs/COMPATIBILITY.md` and ADR-0015:

| Fixture | Passed | Skipped | Failed |
| --- | --- | --- | --- |
| `FakeTransport` with the first-party MBAP responder and `FakeClock` | 21 | 2 (the write cases) | 0 |
| loopback `ScriptedPeer` over real sockets | 18 | 5 | 0 |
| pymodbus 3.8.6 over a real `TcpFactory` | 14 | 9 | 0 |

The S32 slice did not re-run `interop` (it needs Docker and a hosted-run comparison); the
numbers above are the S27 and S30 records.

## Hosted CI matrix

From `STATUS.md` ("Hosted CI and the per-OS matrix"); no new run id is invented here. The
run for the S32 commit is not yet recorded.

| Run | Commit | Result |
| --- | --- | --- |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/40548582722 | `f19be4c` (S30) | completed successfully; every job green except the report-only `semver` |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/40543567845 | `0429da3` (S27) | earlier run, recorded in `STATUS.md` |
| (S31 and S32 commits) | `8f5c3d0`, S32 | not yet recorded; the S31 commit adds the `parity` job |

Jobs of the S30 run: `test` on ubuntu, macOS and Windows each with Rust 1.97.1 and 1.99.1
(six cells), `quality`, `cross`, `dependencies`, `package`, `dco`, `features`, `machete`,
`lint-attrs`, `geiger`, `beta` (report only), `fuzz-smoke` (seven targets), `examples`,
`interop`: green. `semver` (report only) fails until the sibling crates exist on
crates.io.

## Fuzzing

One recorded run, all seven targets concurrently (one single-threaded libFuzzer process
each, 12-core machine), started 2026-10-27 18:19:51 UTC and finished 18:50:06 UTC, with
`scripts/fuzz-evidence.sh 1800`, which builds once and then runs for each target

```sh
cd fuzz && env -u RUSTUP_TOOLCHAIN cargo fuzz run --target aarch64-apple-darwin <target> \
  target/evidence-corpus/<target> -- -max_total_time=1800
```

Each target ran on its own copy of its committed seed inputs under `fuzz/target/`, so
`fuzz/corpus/` is untouched (`git status --porcelain fuzz` printed nothing after the run).
Nightly `nightly-2026-10-01`, cargo-fuzz 0.13.2, libFuzzer defaults (4096-byte input
limit, 2048 MB RSS limit). No target crashed, hung or ran out of memory: every process
exited 0 and `fuzz/artifacts/` holds no file.

| Target | Seeds | Duration | Runs | Final coverage counter (`cov`) | Artifacts |
| --- | --- | --- | --- | --- | --- |
| `fuzz_codec_bounded_reader` | 5 | 1802 s | 141,115,168 | 191 | 0 |
| `fuzz_core_connection_string` | 5 | 1801 s | 143,338,475 | 449 | 0 |
| `fuzz_modbus_address` | 6 | 1801 s | 85,956,280 | 711 | 0 |
| `fuzz_modbus_connection_string` | 15 | 1802 s | 50,778,734 | 1,546 | 0 |
| `fuzz_modbus_mbap` | 13 | 1802 s | 63,894,402 | 116 | 0 |
| `fuzz_modbus_pdu` | 6 | 1802 s | 100,470,025 | 311 | 0 |
| `fuzz_sensors_profile_toml` | 6 | 1802 s | 32,175,834 | 4,715 | 0 |

Durations are the script's wall-clock seconds per target (1,801 or 1,802; the whole run
finished after 1,803 s), not libFuzzer's own "Done N runs" figure. The 30-minute figure
meets the plan's minimum; the 8-hour release run of CODING_STANDARDS 20.3 was not done.
The per-target libFuzzer logs (`fuzz/target/evidence-logs/`) were not retained and cannot
be re-checked: the figures are the summary the script printed, reproduced verbatim below
(runs, final `cov`, seconds, exit status, artifact count), as recorded by the implementer
of this slice. A 10-second spot run by a reviewer reproduced the same `cov` plateaus for
three targets, exit 0 and no artifact. No mid-run plateau figures are claimed. The seed
counts match `fuzz/corpus/`.

```text
target                             runs         cov        seconds  exit     artifacts
fuzz_codec_bounded_reader          141115168    191        1802     0        0
fuzz_core_connection_string        143338475    449        1801     0        0
fuzz_modbus_address                85956280     711        1801     0        0
fuzz_modbus_connection_string      50778734     1546       1802     0        0
fuzz_modbus_mbap                   63894402     116        1802     0        0
fuzz_modbus_pdu                    100470025    311        1802     0        0
fuzz_sensors_profile_toml          32175834     4715       1802     0        0
```

## Mutation testing

One recorded run of cargo-mutants 27.1.0 (`mise run mutants`, `mutants.toml`, nextest,
3 jobs), 2026-10-27, 23 minutes. Scope, exclusions with reasons, the tests added to kill
survivors and the earlier runs are in `docs/evidence/mutants/README.md`; the outcomes are
in `docs/evidence/mutants/outcomes.json`.

**569 mutants run: 462 caught, 0 missed, 0 timeouts, 107 unviable; 3 more were excluded
as equivalent. Kill rate 100% (462 / 462); 99.4% (462 / 465) if the three excluded
mutants are counted as survivors.** The plan's 80% mutation target is therefore met by
the measurement, but it was not a gate and one run is the whole evidence.

| File | Mutants | Caught | Missed | Excluded | Timeout | Unviable | Kill rate |
| --- | --- | --- | --- | --- | --- | --- | --- |
| core `value.rs` | 15 | 14 | 0 | 0 | 0 | 1 | 100.0% |
| core `uri.rs` | 35 | 33 | 0 | 0 | 0 | 2 | 100.0% |
| driver-sim `address.rs` | 23 | 18 | 0 | 0 | 0 | 5 | 100.0% |
| driver-sim `driver.rs` (configuration parser) | 34 | 25 | 0 | 0 | 0 | 9 | 100.0% |
| modbus `address.rs` | 119 | 87 | 0 | 0 | 0 | 32 | 100.0% |
| modbus `pdu/code.rs` | 12 | 10 | 0 | 0 | 0 | 2 | 100.0% |
| modbus `pdu/codec.rs` | 41 | 38 | 0 | 0 | 0 | 3 | 100.0% |
| modbus `pdu/model.rs` | 23 | 21 | 0 | 0 | 0 | 2 | 100.0% |
| modbus `framing/mbap.rs` | 44 | 35 | 0 | 0 | 0 | 9 | 100.0% |
| modbus `decode.rs` | 36 | 29 | 0 | 0 | 0 | 7 | 100.0% |
| modbus `batch.rs` | 31 | 23 | 0 | 0 | 0 | 8 | 100.0% |
| modbus `driver.rs` (configuration parser) | 35 | 26 | 0 | 0 | 0 | 9 | 100.0% |
| modbus `options.rs` (configuration parameters) | 20 | 14 | 0 | 0 | 0 | 6 | 100.0% |
| sensors `measurement/scale.rs` | 104 | 89 | 0 | 3 | 0 | 12 | 100.0% |
| Total | 572 | 462 | 0 | 3 | 0 | 107 | 100.0% |

The first run found 7 survivors. Four were killed by tests added in S32 (named in the
README: the modbus address prefix guard, the limit-parameter digits message, the sensors
integer-zero sign and the `I32` own-type arm). Three are equivalent mutants listed in
`exclude_re` of `mutants.toml` with their reasons (`own_type` `I64` arm, `integral` `|`
versus `^`, `bits_of` `<` versus `<=`). No library code changed. The stored run also
changed one setting: `--fail-fast` for the test phase, because 32 mutants that failed
assertions at once were reported as timeouts only for lack of it.

## Coverage

Fresh `mise run coverage` on the S32 tree (exit 0; 1,182 tests passed, 10 skipped; the
gates are unchanged). Line coverage is the gated figure; region coverage is reported by
`cargo llvm-cov` and is not gated and is not branch coverage (the `Branches` column of the
summary is empty on stable). Source of the numbers: `target/coverage-summary.txt` and the
gate output of the run.

| Crate | Lines hit / total | Line coverage | Gate | Regions hit / total | Region coverage | Functions |
| --- | --- | --- | --- | --- | --- | --- |
| `stonecrab-core` | 3577 / 3577 | 100.00% | 100% | 5710 / 5725 | 99.74% | 424 / 424 |
| `stonecrab-codec` | 181 / 181 | 100.00% | 100% | 258 / 259 | 99.61% | 32 / 32 |
| `stonecrab-driver-sim` | 2051 / 2051 | 100.00% | 100% | 3467 / 3479 | 99.66% | 256 / 256 |
| `stonecrab-modbus` | 2758 / 2758 | 100.00% | 100% | 4203 / 4236 | 99.22% | 323 / 323 |
| `stonecrab-sensors` | 1411 / 1411 | 100.00% | 100% | 2185 / 2200 | 99.32% | 162 / 162 |
| `stonecrab-transport` | 1216 / 1220 | 99.67% | 95% | 1819 / 1840 | 98.86% | 147 / 148 |
| `stonecrab-testkit` (unpublished) | 4606 / 4679 | 98.44% | 90% | 8489 / 8700 | 97.57% | 584 / 589 |
| `stonecrab` (facade) | n/a | n/a | n/a (no executable lines) | n/a | n/a | n/a |
| Workspace total | 15800 / 15877 | 99.52% | none | 26131 / 26439 | 98.84% | 1928 / 1934 |

The four uncovered transport lines are the real-socket deadline-expiry and
resolver-failure arms named in ADR-0005 and ADR-0014. The sensors line total rose from
1398 to 1411 because S32 added two unit tests in `scale.rs`; every added line is executed.
The other crates equal the figures in `STATUS.md`.

## Local gates

Run from the worktree of the S32 tree on Rust 1.99.1. Exit statuses are the recorded
ones; the orchestrator re-runs them on the commit.

| Command | Exit | Result |
| --- | --- | --- |
| `mise run check` | 0 | fmt, strict Clippy, nextest (1,182 passed, 10 skipped), `cargo test` with doctests, docs with warnings denied, no-default-features |
| `mise run coverage` | 0 | per-crate gates pass (above) |
| `mise run lint-attrs` | 0 | 8 `lib.rs` files checked |
| `mise run parity` | 0 | the check and its self-test |
| `mise run dependencies` | 0 | advisories, bans, licenses and sources ok (unmatched-license-allowance warnings, as before) |
| `mise run machete` | 0 | no unused dependencies |
| `mise run mutants` | 0 | the stored run (above); runs 1 and 2 exited 3 |
| `scripts/fuzz-evidence.sh 1800` | 0 | seven targets, no artifact |
| `mise run package` | 101 | refused to package a tree with uncommitted changes (`scale.rs`), as designed; the same workspace `cargo package --locked --workspace --exclude stonecrab-testkit --allow-dirty` with a fresh `CARGO_HOME` exited 0 and every crate's `--list` equals `packaging/*.txt`; re-run on the clean commit |
| `mise run sbom` | 101 / 0 | 101 on the dirty tree for the same reason; with `SBOM_ALLOW_DIRTY=1` exit 0: 7 CycloneDX 1.5 JSON files, `package-list.txt` (121 lines) and `package-lists/` per crate |
| `cargo release --version` | 0 | `cargo-release 1.1.7` |
| `cargo release version patch --workspace --exclude stonecrab-testkit` | 0 | dry run (no `--execute`): prints the planned 0.1.0 to 0.1.1 and 0.2.0 to 0.2.1 bumps and the `=` pin rewrites, then "aborting release due to dry run"; no file changed |
| YAML parse of `release-evidence.yml` | 0 | PyYAML 6.0.3 `safe_load` |
| `git status --porcelain` | 0 | only the files of the slice; `fuzz/corpus/` unchanged |
| `ls fuzz/artifacts` | 0 | empty directories only |

`cargo tree --locked -e normal,dev --workspace --all-features -i <package>` for `once_cell`,
`lazy_static`, `log`, `time` and `chrono` in turn shows `once_cell` only through
`tracing-core` (wrapped), `lazy_static` only through `sharded-slab` under the
`tracing-subscriber` dev-dependency (wrapped), and no `log`, `time` or `chrono`.

SBOM output (`mise run sbom` and the `release-evidence` workflow), in
`target/release-evidence/`: `sbom/<crate>.cdx.json` for `stonecrab`, `stonecrab-core`,
`stonecrab-codec`, `stonecrab-transport`, `stonecrab-driver-sim`, `stonecrab-modbus` and
`stonecrab-sensors`, plus `package-list.txt` and `package-lists/<crate>.txt`.
The SBOM files have the checkout path replaced by `.` and the script fails if an absolute
`/Users`, `/home`, `/private`, `/var` or `/tmp` path remains, so a local run and the
workflow run are both safe to attach. The workflow has not been dispatched: it needs the
commit on GitHub.

## Divergence table and upstream issues

The divergence table is "Divergences from upstream" in `docs/parity/modbus.md`. The
issues for quirks a, b, c, e, f and g are drafted in `docs/evidence/upstream-issues/` and
**not filed**: filing on the Apache tracker needs the maintainer's approval (ADR-0017
decision 10). They were found by reading the v1.0.0 source and were not reproduced against
a running upstream.

| Quirk | Parity rows | Draft | Status |
| --- | --- | --- | --- |
| a: UDP configuration class tested against a BouncyCastle `UDPTransport` | `MB-QUIRK-A` | `upstream-issues/quirk-a.md` | drafted, not filed: needs maintainer approval |
| b: split FC20 read drops the second item | `MB-QUIRK-B` | `upstream-issues/quirk-b.md` | drafted, not filed: needs maintainer approval |
| c: dead FC05 echo check | `MB-QUIRK-C` | `upstream-issues/quirk-c.md` | drafted, not filed: needs maintainer approval |
| e: replies matched on transaction id only | `MB-QUIRK-E`, `MB-READ-MATCH` | `upstream-issues/quirk-e.md` | drafted, not filed: needs maintainer approval |
| f: test suite passes undeclared `unit-identifier` | `MB-QUIRK-F`, `MB-CFG-UNKNOWN` | `upstream-issues/quirk-f.md` | drafted, not filed: needs maintainer approval |
| g: `4x:INT` reaches `Integer.parseInt(null)` | `MB-QUIRK-G`, `MB-ADDR-SHORT-4X` | `upstream-issues/quirk-g.md` | drafted, not filed: needs maintainer approval |

The other divergence rows that had said "upstream issue to be filed before S32"
(`MB-CFG-UNIT-ID`, `MB-EXC-05`, `MB-TYPE-TIME`, `MB-LIMIT-ADU`) now state
that no issue was drafted and why. The divergence rows of `MB-QUIRK-D`, `MB-QUIRK-I` and
`MB-QUIRK-L` were removed (the quirk rows remain): each is outside
the six quirks the plan names, is a Stonecrab inference, or rests on an UNVERIFIED
consequence.

## Gates not run

Not met for the alpha, as disclosed in the plan and in `CHANGELOG.md`, with the measured
substitute where one exists:

| Gate | State at the alpha |
| --- | --- |
| Branch coverage >= 80% | not run; line coverage is gated per crate and region coverage is reported (above); `--branch` on stable is UNVERIFIED |
| Mutation >= 80% | not a gate; one recorded run, rates above |
| Release fuzzing 8 h per target | not run; 30 minutes per target, table above |
| 24 h soak and memory stability | not run (a stable-driver-release gate) |
| Benchmarks and the 10% gate | not run; no baseline, no performance claim |
| Differential tests against PLC4J | not run; no Java harness |
| Hardware verification | not run; no Modbus device on the bench, template only (`docs/hardware/TEMPLATE.md`) |
| Real-clock timing and fault injection against a peer | not run; stock pymodbus has no fault hooks (ADR-0015 decision 16) |
| `cargo semver-checks` | report-only; no baseline on crates.io for the sibling crates, and the 0.1.0 API was replaced |
| `cargo publish --dry-run` in dependency order | not run in S32 (forbidden against the registry in this slice; a dependent crate cannot dry-run before its siblings are published); the workspace `cargo package` form passed |
| `regen`, RTU, OPC UA, direct-sensor cases | out of scope for the alpha |
| Upstream vectors (P4) | not redistributed; every vector is first-party, so the four byte-order rows stay at `implemented` |
| Stale-value enforcement, `Timeouts.idle`, queued cancellation on the simulator, real-socket connect-timeout arm | deferred or uncovered as named in the plan and ADR-0014 |

## Alpha release checklist

Items of "Alpha release checklist" in `docs/ALPHA_PLAN.md`. `[x]` means done and evidenced
here; `[ ]` says what blocks it.

1. `[ ]` P1 to P4 done. P5 (the waiver) is done and recorded in ADR-0005 and `STATUS.md`.
   **Blocked on the maintainer:** P1 (private vulnerability reporting, verified with a test
   report), P2 (secret scanning and push protection), P3 (the `master` ruleset), P4 (written
   approval to redistribute the PLC4X vectors, which also blocks S04a and S04b).
2. `[ ]` Local gates green on 1.99.1. Green: `mise run check`, `lint-attrs`, `coverage`,
   `dependencies`, `machete`, `parity` (table above), and `cargo tree -i` shows only the
   recorded wrapped paths. The 60-second `mise run fuzz` smoke was not re-run; the
   30-minute seven-target run above stands in for it. **Open:** `mise run package`
   refuses the uncommitted tree and `mise run interop` needs Docker (last green in S30);
   both are re-run on the clean commit. The MSRV cell is green in hosted run
   40548582722.
3. `[ ]` Hosted CI green on `master` for the candidate commit, including `interop`,
   `fuzz-smoke` via `workflow_dispatch`, `features`, `examples`, `dco`, `lint-attrs`, run
   URL recorded in `STATUS.md`. **Blocked:** the S31 and S32 commits are not pushed; the
   last recorded green run is for S30 (`f19be4c`).
4. `[x]` Both example runs pass: `profile_read` against pymodbus and against `sim` returned
   success in the S30 `interop` and `examples` jobs (run 40548582722). Re-confirm on the
   candidate commit with item 3.
5. `[x]` `grep -rn "covers: " crates/ tools/` lists F1 to F7 with the counts of
   `docs/COMPATIBILITY.md` (50, 71, 69, 44, 32, 4, 36); `scripts/parity-check.sh` passes.
6. `[x]` `docs/parity/*.md`: the parity check enforces a test id on every `implemented` or
   `automated-verified` row; deferred rows are visible; every divergence row cites a
   test and a protocol section, a draft or the stated reason no issue was drafted (quirks
   d, i and l are not divergences and have no row); the four byte-order rows
   stay below `automated-verified` until P4 (S23's little-endian vectors).
7. `[x]` README matrix, crate READMEs and `Capabilities` agree (the README and matrix tests
   pass in `mise run check`): `sim` read and write, `modbus-tcp` read only.
8. `[ ]` `fixtures/` listed in `PROVENANCE.toml` with the adapter's exchange count; NOTICE.
   **Partly blocked by P4:** `fixtures/modbus/PROVENANCE.toml` lists every first-party
   fixture and `NOTICE` says no upstream vectors are redistributed; the upstream copies,
   the driver-suite adapter (S04a, S04b) and its exchange count do not exist.
9. `[x]` This file is complete: fuzz durations, mutation rates with the exclusions,
   region coverage, matrix results, pymodbus version and digest, upstream baseline,
   gates not run, divergence references. The upstream issues are drafted and not filed
   (reason above).
10. `[x]` CHANGELOG release sections carry the four SPEC items (upstream baseline,
    supported operations, hardware evidence, breaking changes); `SECURITY.md` names the
    alpha as the supported line.
11. `[ ]` `cargo package --list` reviewed per crate (done: equal to `packaging/*.txt`);
    SBOM generated (done locally; **attach to the draft pre-release: maintainer**);
    `cargo publish --dry-run` in dependency order with a fresh `CARGO_HOME`
    (**not run**: forbidden against the registry in S32; run by the maintainer per
    ADR-0017 decision 3); `cargo semver-checks` report (**not run**: no baseline for the
    siblings, the facade's 0.1.0 API was replaced).
12. `[ ]` Signed tags prepared but not pushed. **Prepared as commands only**
    (ADR-0017 decision 4): no tag exists. Creating and pushing them, `cargo publish` and
    crates.io registration need the maintainer's separate approval.
13. `[x]` `SECURITY.md` carries the yank and RUSTSEC commitment and points to ADR-0017
    decision 5 for the procedure.
