# 0017: Release process and evidence
## Status
Amended by ADR-0018 (2026-10-27): Stonecrab is one crate, so decisions 1 to 7 below are
rewritten for the single package `stonecrab` (one version, one publish, one tag, one
yank). The original seven-crate text (publish order core to facade, per-crate tags,
the `=` pins and the facade-plus-sibling yank rule) is replaced, not kept; the git history
holds it. Decisions 8 to 11 are unchanged except where they name a crate.

Accepted by the sole maintainer on 2026-10-27, as slice S32 of docs/ALPHA_PLAN.md. It
implements decision D1 (versioning, itself superseded by ADR-0018) and D20 (release
tooling and review), builds on ADR-0005 (gates, coverage thresholds and the
single-maintainer waiver) and ADR-0009 (the fuzz policy), and records the evidence the
alpha release checklist asks for. It completes CODING_STANDARDS section 20 for the alpha.
Nothing described here has been run against crates.io: no crate is published, no tag
exists and none is pushed by this slice.

## Context
Only `stonecrab` 0.1.0, a specification-stage placeholder, is on crates.io. The alpha is
one crate, `stonecrab` 0.2.0, with the former sibling crates as modules (ADR-0018). A
published version cannot be replaced. CODING_STANDARDS 20.2 asks for signed tags and a
human-reviewed changelog, 20.3 for a release-candidate checklist (several of whose items
the alpha does not meet, see decision 11), and 20.5 for a 24-hour yank rule. The plan's
alpha release checklist (items 9 to 13) adds the evidence file, the SBOM,
prepared-but-unpushed tags and the yank commitment. There is one maintainer, so ADR-0005
decision 6 (the waiver of the two-approval rule of 18.6, which names release tooling)
applies to every release change.

## Decision
1. **Version (D1, superseded by ADR-0018).** The alpha is `stonecrab` 0.2.0, a plain
   pre-1.0 version. `-alpha.N` is rejected: the published 0.1.0 placeholder of `stonecrab`
   would win resolution over a pre-release, and the minor bump records the removal of
   `placeholder::status`. The manifest already carries the release version, so the alpha
   needs no version bump. There are no sibling crates and so no `=` pins.
2. **Tooling and pins (D20).** `cargo-release` is the release tool, with the pins
   `cargo-release` 1.1.7, `cargo-mutants` 27.1.0 and `cargo-cyclonedx` 0.5.9 in
   `mise.toml` (`cargo:` entries, which mise installs with `cargo install --locked`).
   `release-plz` is not used. `release.toml` sets `publish = false`, `push = false` and
   `tag = false`, so cargo-release can neither publish, push nor create a tag. It also
   sets `sign-commit`, `sign-tag`, `allow-branch = ["master"]`, one release commit and
   the tag name `v{{version}}`. A `pre-release-hook` runs `cargo package --locked --list`
   for the crate; the full archive build is `mise run package`.
   - Use only the steps that change files: `cargo release version <level>` (a dry run
     unless `--execute` is given; it prints the planned bump), then `replace` and `hook`.
     Do not run the all-in-one `cargo release <level>`: its commit step cannot add the
     `Signed-off-by` trailer the `dco` job requires. The maintainer commits with
     `git commit -s -S`.
   - The `tag`, `push` and `publish` steps run pre-flight checks even as a dry run
     (`git fetch` of the remote and crates.io index queries), so they are not used for
     verification. With `publish = false` the publish step publishes nothing even with
     `--execute`: cargo-release 1.1.7 filters the packages on that setting and returns
     before it would call `cargo publish` (read in the 1.1.7 source, not run).
3. **Publish commands.** There is one crate, so there is no order. Nothing is published
   without the maintainer's separate approval, which is a message of the maintainer for
   that version and is not given by this ADR, by a green CI run or by any agent.
   - Pre-flight, from a clean checkout of the release commit on `master` whose hosted CI
     run is green: `git status --porcelain` prints nothing; `mise run check`, `mise run
     coverage`, `mise run package` (the archive built with a fresh `CARGO_HOME` and
     compared with `packaging/stonecrab.txt`), `mise run interop` and `mise run sbom`.
   - Then, with the credentials of the maintainer's own `cargo login` (a token is never
     written to the repository, a log or a commit):

     ```sh
     cargo package --locked -p stonecrab            # the archive builds
     cargo publish --locked --dry-run -p stonecrab
     cargo publish --locked -p stonecrab            # only with the separate approval
     ```

     `cargo publish` waits until the version is in the index. Stop at the first failure.
     The archive holds the `testkit` feature's code but not the repository files its own
     tests read (`testdata/`, `fixtures/`); the full suite runs from the checkout of the
     release commit, which is what the pre-flight does.
4. **Tag.** One annotated tag, `v0.2.0`, on the release commit. It is signed when a
   signing key is configured; whether to sign is the maintainer's decision, and the
   `sign-tag` setting of `release.toml` is a safeguard for a hand-run `cargo release tag`,
   not a requirement on this procedure. Only the maintainer creates and pushes it:

   ```sh
   # sign the tag (use -a instead of -s when no key is configured)
   git tag -s v0.2.0 -m "stonecrab 0.2.0" <release-commit>
   # must report a good signature when signed
   git tag -v v0.2.0
   # only after crates.io accepted the crate
   git push origin v0.2.0
   ```

   The tag is pushed by name (never `--tags`) and only after crates.io accepted the
   crate. A tag is never moved or deleted after the push; a wrong release is yanked and
   superseded.
5. **Yank procedure (CODING_STANDARDS 20.5).** A published version with a confirmed
   soundness or safety defect (unbounded allocation, a panic on input, write replay) is
   handled in this order, and the 24 hours run from the maintainer's recorded
   confirmation time:
   1. `cargo yank --vers <version> stonecrab` within 24 hours. It is one crate: there is
      no sibling to yank with it and no pin to follow. `cargo yank --undo --vers <version>
      stonecrab` reverses a mistaken yank. Yanking does not remove the version from
      existing lock files.
   2. An advisory pull request to `rustsec/advisory-db` for `stonecrab`
      (`crates/stonecrab/RUSTSEC-0000-0000.md`, the identifier assigned by the database),
      stating the affected and the patched versions, and a GitHub security advisory on
      the repository when a reporter is involved. The maintainer opens it; the
      description contains no credentials or private device data.
   3. A fixed patch release of `stonecrab` (a fix to any module is a release of the whole
      crate) with a `CHANGELOG.md` entry, a new tag under decision 4 and the same
      pre-flight. A patch release is not waited for before the yank.
   4. `SECURITY.md` is updated to name the supported versions after the fix.
   Review of the release change is the self-review of the ADR-0005 waiver until a
   second maintainer with merge rights exists.
6. **SBOM.** `mise run sbom` runs `scripts/sbom.sh`: `cargo cyclonedx --format json
   --all-features --spec-version 1.5` for the workspace, the one SBOM file moved to
   `target/release-evidence/sbom/`, and `cargo package --locked -p stonecrab --list`
   beside it. The `release-evidence` workflow
   (`.github/workflows/release-evidence.yml`, `workflow_dispatch` only, read-only
   permissions) runs the same script and uploads the directory as the `release-evidence`
   artifact, to be attached to the draft pre-release. The script replaces the checkout
   path in the SBOM with `.` and fails if an absolute home or temp path remains. The SBOM
   lists dependencies and is not signed; it makes no vulnerability claim.
7. **Mutation evidence.** `mutants.toml` scopes `cargo mutants` to the modules named in
   CODING_STANDARDS 13.9 and the plan: the core's `value` and `uri`; `sim` `address` and
   `driver` (its configuration parser); `modbus` `address`, `pdu`, `framing`, `decode`,
   `batch`, `driver` and `options` (the configuration parser and its parameters);
   `sensors` `measurement/scale`. It uses nextest with `--fail-fast` for the test phase
   (the nextest profile keeps running after a failure, which turned caught mutants into
   timeouts), `--all-features` and `--locked`, runs in a scratch copy of the tree (never
   `--in-place`) and tests the one package. `mise run mutants` runs `scripts/mutants.sh`,
   which stores only `outcomes.json` under `docs/evidence/mutants/`. A surviving mutant
   is killed by a test or listed in `exclude_re` of `mutants.toml` with a reason a
   reviewer can check (cargo-mutants 27.1.0 has no `[exclusions]` table; `exclude_re` is
   its mechanism). The kill rate per file is recorded in `docs/evidence/alpha.md`. One
   recorded run is the alpha evidence; the 80% mutation figure of the plan is not a gate
   at the alpha and the measured rates replace it (decision 11). The weekly `scheduled`
   job of `ci.yml` keeps its no-op mutation step until the maintainer enables
   `cargo mutants --config mutants.toml` there, which is a CI change under the waiver.
8. **Fuzz evidence.** The seven targets run at least 30 minutes each, concurrently, one
   single-threaded libFuzzer process per target, with `scripts/fuzz-evidence.sh` (the
   command per target is `cd fuzz && env -u RUSTUP_TOOLCHAIN cargo fuzz run --target
   <host triple> <target> <scratch corpus> -- -max_total_time=1800`). Each target gets
   its own copy of its seed corpus under `fuzz/target/`, as `scripts/fuzz-smoke.sh`
   does, so libFuzzer's new inputs never enter the committed `fuzz/corpus/`. The run's
   durations, final run counts, final coverage counters and the number of crash
   artifacts are tabulated in `docs/evidence/alpha.md`, not the raw logs. A crash is a
   finding: it is reproduced, its input committed as a corpus seed with a regression test
   named `fuzz_<target>_<short-hash>` in the crate, and reported (CONTRIBUTING,
   "Fuzz findings"). CI keeps the 60-second `fuzz-smoke` job and the weekly 30-minute run.
9. **Upstream baseline evidence.** `upstream/manifest.toml` records the verification of
   the Apache PLC4X 1.0.0 source-release signature (GnuPG, a temporary keyring holding
   only the project KEYS file, the signing-key fingerprint, the archive name and its
   SHA-512 as published and as computed) and sets `signature_verified = true` only on a
   good signature. That proves the archive matches a key the project publishes; the
   archive's contents were not inventoried. The six drafted upstream issues of decision
   10 are drafted in `docs/evidence/upstream-issues/` and are not filed.
10. **Upstream issues.** The parity record's quirks a, b, c, e, f and g are written up
    as drafts, one file each, in `docs/evidence/upstream-issues/`. They were found by
    reading the pinned source and are not reproduced against a running upstream. They
    are not filed: opening an issue on the Apache tracker speaks for the project and
    needs the maintainer's approval, so the "Divergences from upstream" table of
    `docs/parity/modbus.md` says "drafted, not filed: needs maintainer approval" for
    them. The other rows that said "to be filed before S32" are not among the six; the
    table records that no issue was drafted for them and why.
11. **What the alpha release does not claim.** Not met, with the measured substitute in
    `docs/evidence/alpha.md`: the 8-hour fuzz run of CODING_STANDARDS 20.3 (30 minutes
    per target), the 24-hour soak and memory stability, benchmarks and the 10% gate
    (no baseline, no performance claim), differential tests against PLC4J, branch
    coverage (region coverage is reported), a mutation threshold as a gate (one run, the
    rates stated), hardware verification (no device, no hardware record), a `semver`
    baseline (blocking only after the first tag), the upstream little-endian vectors
    (P4), the M0 inventory beyond the Modbus TCP read path, and `xtask release-check`
    (there is no xtask; `docs/evidence/alpha.md` carries the checklist instead). The
    alpha claims no support for writes, subscriptions, browse, discovery, RTU, OPC UA
    or any server other than pymodbus 3.8.6.

## Alternatives considered
- **release-plz.** Generates versions and changelogs from commits and publishes from CI.
  Rejected for the alpha: it would put a publish token in CI, and publication needs a
  separate human approval that no workflow can give.
- **cargo-release with publish enabled.** One command could do everything, but it would
  also commit without a `Signed-off-by` trailer, tag, push and publish in one step.
  Rejected: every irreversible step stays a separate maintainer action.
- **`-alpha.N` versions.** Rejected under D1.
- **Storing the whole `mutants.out` directory.** Rejected: the per-mutant logs hold paths
  of the machine that ran them and add nothing a reviewer can check beyond the outcomes.
- **Excluding survivors with `#[mutants::skip]`.** Rejected: it needs a dependency on the
  `mutants` crate and hides the reason inside the code; the list in `mutants.toml` is
  reviewed in one place.
- **A single committed SBOM.** Rejected: a SBOM belongs to a release commit, so the
  workflow and `mise run sbom` produce it per release and it is attached, not stored.

## Consequences
The release is a sequence of explicit maintainer commands with no automation that can
publish or tag, at the cost of manual steps and the risk of a typing mistake; the ordered
checklist and the signed-tag verification are the mitigation. The waiver means no second
person reviews a release until a second maintainer exists; the self-review stays
mandatory. The evidence files are a snapshot of one commit and one machine (a 12-core
macOS machine for the fuzz and mutation runs); CI re-runs the cheap parts.

## Compliance
`mise run mutants`, `mise run sbom` and `scripts/fuzz-evidence.sh` reproduce the
evidence. `release.toml`, `mutants.toml`, the three pins in `mise.toml` and the
`release-evidence` workflow are the configuration. SECURITY.md points here for the yank
mechanics, CONTRIBUTING.md points here for the release process, and
`docs/evidence/alpha.md` carries the release checklist state. This ADR does not change
any gate, coverage tier or CI job of ADR-0005.

## Addendum (S62, 2026-10-29): the soak and the 8-hour fuzz run as release evidence
Two long runs belong to the evidence of a release and are run by the maintainer on the
maintainer's own machine, because they tie it up for a day and a hosted job cannot hold
them (a GitHub-hosted job ends after six hours, and the shared runners are too noisy for a
memory figure). Neither is a CI job and neither can publish or tag. Slice S62 made them
commands, and proved each with a short run (`docs/evidence/soak/`, `docs/evidence/fuzz/`);
the long runs were not started, because they need the maintainer's approval first.

**The 24-hour soak.** `scripts/soak.sh` (`mise run soak -- TARGET SECONDS`) builds the
`soak` example in release mode and runs it for SECONDS (default 86400) against one target,
reading four tags in a loop through the public API (`DriverRegistry`, `Connection::read`)
and sampling the resident memory of that process every 10 s with `ps -o rss=`, which
behaves the same on macOS and Linux. The two targets are `sim` (the simulator in the same
process, read flat out, with a lost link, a lost reply and an undecodable reply pushed in
turn every 5 s, so the link is reopened about every 15 s) and `pymodbus` (the interop
peer in a container of its own on port 5021, read every 10 ms, the container restarted
every 120 s so the driver's own reconnect runs 720 times in 24 hours). A CSV
(`elapsed_s,rss_kib,reads,errors,reconnects`), the example's log and a Markdown report
are written to `target/soak/TARGET/`. The commands the maintainer runs, in two shells (the
two targets do not disturb each other), and the record:

```sh
caffeinate -i mise run soak -- sim 86400
caffeinate -i mise run soak -- pymodbus 86400
cp -R target/soak/sim docs/evidence/soak/24h-YYYY-MM-DD-sim      # then the same for pymodbus
```

Pass criteria. The first 20% of the run is a warm-up and is not judged. Growth is the
median RSS of the last quarter of the remaining samples minus the median of the first
quarter, so one spike does not move it. The run passes when all of these hold: growth is
at most 4096 KiB (`--max-growth-kib`); the example exited 0; in the last quarter of the run
there were more reads than errors (the link works at the end); and, for pymodbus, at
least as many reconnects as container restarts. Fewer than 12 samples is inconclusive and
fails. The bound is 4 MiB because the 10-minute runs drifted by at most 112 KiB after the
warm-up (the allocator moves in 16 KiB pages), and a leak that matters is far above it
over 24 hours. The growth is the difference of two quarter medians that lie about 0.6 of
the run apart (the middle of the first and of the last quarter of the judged 80%), so the
bound is reached by a steady leak of about 0.28 MiB an hour over 24 hours (4 MiB over
14.4 hours), and by about 40 MiB an hour over 10 minutes. Leaks of that size are caught:
two bytes per read on pymodbus (about 61 reads a second, as measured, so about 5.2 million
reads in 24 hours), or 12 KiB leaked per container restart (720 restarts); a leak of one
byte per read or of 6 KiB per restart passes, and so does anything smaller. On the
simulator, which reads about a million times a second, one byte per read is caught within
a 10-minute run. The self-test checks a 24-hour series at half and at twice the
detectable rate. The report records the least-squares slope as information. The soak measures the Stonecrab process only; it says
nothing about file descriptors or task counts (SPEC.md lists them for the soak), which
`ps` does not give portably; the reconnect counts show that the link is reopened and the
flat RSS that nothing piles up.

**The 8-hour fuzz run.** `scripts/fuzz-evidence.sh 28800` (`mise run fuzz-evidence --
28800`) runs every fuzz target concurrently for 8 hours on the nightly pinned in
`fuzz/rust-toolchain.toml`, and writes `fuzz/target/evidence-logs/report.md`: the run's
facts, then per target the seed inputs, runs, coverage counters, features, corpus,
executions per second, seconds, exit status and crash artifacts. It passes when every
target exits 0 after the full time and leaves no artifact. The record is
`cp fuzz/target/evidence-logs/report.md docs/evidence/fuzz/8h-YYYY-MM-DD.md`. The script
existed since S03b with a table printed to the terminal; S62 added the report file, the
seed and executions-per-second columns and the pass criteria, and left the way the
targets run alone.

**When.** Both runs, with their reports committed, are part of the checklist of a release
that changes `src/transport`, `src/modbus` or `src/sim`, on the commit that will be tagged
or a descendant that changes none of those directories. A release that changes none of them
may cite the last reports. The single-maintainer waiver of ADR-0005 applies as it does to
the other evidence: the maintainer may waive a run, and says so in the release notes, as
the alpha did for the gates it disclosed. The weekly
`scheduled` job keeps running the 30-minute fuzz.

**Not done.** The 24-hour soak and the 8-hour fuzz run have not been run: S62 recorded 10
minutes of each soak target and 5 minutes of fuzzing. Neither is a claim about 24 hours.
