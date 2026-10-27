# 0017: Release process and evidence
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S32 of docs/ALPHA_PLAN.md. It
implements decision D1 (versioning) and D20 (release tooling and review), builds on
ADR-0005 (gates, coverage tiers and the single-maintainer waiver) and ADR-0009 (the fuzz
policy), and records the evidence the alpha release checklist asks for. It completes
CODING_STANDARDS section 20 for the alpha. Nothing described here has been run against
crates.io: no crate is published, no tag exists and none is pushed by this slice.

## Context
Only `stonecrab` 0.1.0, a specification-stage placeholder, is on crates.io. The alpha is
seven crates (facade 0.2.0; core, codec, transport, driver-sim, modbus and sensors
0.1.0) that depend on each other, so the order of publication matters, a published
version cannot be replaced, and a mistake in an `=` pin makes a fresh resolution fail.
CODING_STANDARDS 20.2 asks for signed `<crate>-v<version>` tags and a human-reviewed
changelog, 20.3 for a release-candidate checklist (several of whose items the alpha does
not meet, see decision 11), and 20.5 for a 24-hour yank rule. The plan's alpha release
checklist (items 9 to 13) adds the evidence file, the SBOM, prepared-but-unpushed tags
and the yank commitment. There is one maintainer, so ADR-0005 decision 6 (the waiver of
the two-approval rule of 18.6, which names release tooling) applies to every release
change.

## Decision
1. **Versions (D1).** The alpha is `stonecrab` 0.2.0 with `stonecrab-core`,
   `stonecrab-codec`, `stonecrab-transport`, `stonecrab-driver-sim`, `stonecrab-modbus`
   and `stonecrab-sensors` 0.1.0, plain pre-1.0 versions. `-alpha.N` is rejected: the
   published 0.1.0 placeholder of `stonecrab` would win resolution over a pre-release,
   and the facade's minor bump records the removal of `placeholder::status`. The Cargo
   manifests already carry the release versions, so the alpha needs no version bump.
   The facade requires its three optional siblings as `=0.1.0`; its requirement on
   `stonecrab-core` is the caret `0.1`, which is looser than the `=`-compatible ranges
   of CODING_STANDARDS 20.1. Tightening it is a manifest change outside this slice and
   is listed in STATUS.md as the follow-up.
2. **Tooling and pins (D20).** `cargo-release` is the release tool, with the pins
   `cargo-release` 1.1.7, `cargo-mutants` 27.1.0 and `cargo-cyclonedx` 0.5.9 in
   `mise.toml` (`cargo:` entries, which mise installs with `cargo install --locked`).
   `release-plz` is not used. `release.toml` sets `publish = false`, `push = false` and
   `tag = false`, so cargo-release can neither publish, push nor create a tag. It also
   sets `sign-commit`, `sign-tag`, `allow-branch = ["master"]`, per-crate commits and the
   tag name `{{crate_name}}-v{{version}}`, and `dependent-version = "fix"` so that a
   sibling requirement is rewritten only when it stops matching (the `=` pins follow a
   bumped sibling and are never loosened). A `pre-release-hook` runs `cargo package
   --locked --list` for the crate; the full archive build is `mise run package`.
   - Use only the steps that change files: `cargo release version <level> --workspace
     --exclude stonecrab-testkit` (a dry run unless `--execute` is given; it prints the
     planned bumps and the `=` pin rewrites), then `replace` and `hook`. Do not run the
     all-in-one `cargo release <level>`: its commit step cannot add the `Signed-off-by`
     trailer the `dco` job requires. The maintainer commits with `git commit -s -S`.
   - The `tag`, `push` and `publish` steps run pre-flight checks even as a dry run
     (`git fetch` of the remote and crates.io index queries), so they are not used for
     verification. With `publish = false` the publish step publishes nothing even with
     `--execute`: cargo-release 1.1.7 filters the packages on that setting and returns
     before it would call `cargo publish` (read in the 1.1.7 source, not run).
3. **Publish order and commands.** Order: `stonecrab-core`, `stonecrab-codec`,
   `stonecrab-transport`, `stonecrab-driver-sim`, `stonecrab-modbus`, `stonecrab-sensors`,
   `stonecrab`. It follows the normal dependencies (codec, transport and driver-sim need
   core; modbus needs core, codec and transport; sensors needs core; the facade needs
   core and the three optional siblings), and the dev-dependencies on `stonecrab-testkit`
   (path only, `publish = false`) are dropped from the archives by cargo. Nothing is
   published without the maintainer's separate approval, which is a message of the
   maintainer for that version and is not given by this ADR, by a green CI run or by any
   agent.
   - Pre-flight, from a clean checkout of the release commit on `master` whose hosted CI
     run is green: `git status --porcelain` prints nothing; `mise run check`, `mise run
     coverage`, `mise run package` (all seven archives built with a fresh `CARGO_HOME`
     and compared with `packaging/*.txt`), `mise run interop` and `mise run sbom`.
   - Per crate, in the order above, with the credentials of the maintainer's own
     `cargo login` (a token is never written to the repository, a log or a commit):

     ```sh
     cargo package --locked -p stonecrab-core         # the archive builds
     cargo publish --locked --dry-run -p stonecrab-core
     cargo publish --locked -p stonecrab-core         # only with the separate approval
     ```

     `cargo publish` waits until the version is in the index. For a dependent crate the
     per-crate `cargo package -p` and `--dry-run` resolve its siblings from crates.io, so
     they can only succeed after those are published; before that, the workspace form
     `mise run package` is the check (cargo overlays the unpublished siblings). Stop at
     the first failure; do not publish a later crate on top of a failed earlier one.
4. **Tags.** Annotated, signed, one per crate and version, named `<crate>-v<version>`:
   `stonecrab-core-v0.1.0`, `stonecrab-codec-v0.1.0`, `stonecrab-transport-v0.1.0`,
   `stonecrab-driver-sim-v0.1.0`, `stonecrab-modbus-v0.1.0`, `stonecrab-sensors-v0.1.0`
   and `stonecrab-v0.2.0`, all on the release commit. Only the maintainer creates and
   pushes them, with the maintainer's configured signing key:

   ```sh
   git tag -s stonecrab-core-v0.1.0 -m "stonecrab-core 0.1.0" <release-commit>
   git tag -v stonecrab-core-v0.1.0                  # must report a good signature
   git push origin stonecrab-core-v0.1.0             # after crates.io accepted that crate
   ```

   The same three lines repeat for the other six tags. Tags are pushed one by one, by
   name (never `--tags`), and each only after its crate was accepted. A tag is never
   moved or deleted after the push; a wrong release is yanked and superseded.
5. **Yank procedure (CODING_STANDARDS 20.5).** A published version with a confirmed
   soundness or safety defect (unbounded allocation, a panic on input, write replay) is
   handled in this order, and the 24 hours run from the maintainer's recorded
   confirmation time:
   1. `cargo yank --vers <version> <crate>` for each affected crate within 24 hours.
      A sibling the facade pins with `=` is yanked together with the facade version that
      pins it, because a fresh resolution of a facade whose pinned sibling is yanked
      fails. `cargo yank --undo --vers <version> <crate>` reverses a mistaken yank.
      Yanking does not remove the version from existing lock files.
   2. An advisory pull request to `rustsec/advisory-db` for each affected crate
      (`crates/<crate>/RUSTSEC-0000-0000.md`, the identifier assigned by the database),
      stating the affected and the patched versions, and a GitHub security advisory on
      the repository when a reporter is involved. The maintainer opens it; the
      description contains no credentials or private device data.
   3. A fixed patch release, in the publish order of decision 3, of every affected crate
      and of the facade when a pin moves, with a `CHANGELOG.md` entry, new signed tags
      under decision 4 and the same pre-flight. A patch release is not waited for before
      the yank.
   4. `SECURITY.md` is updated to name the supported versions after the fix.
   Review of the release change is the self-review of the ADR-0005 waiver until a
   second maintainer with merge rights exists.
6. **SBOM.** `mise run sbom` runs `scripts/sbom.sh`: `cargo cyclonedx --format json
   --all-features --spec-version 1.5` for the workspace, the files of the seven
   published crates moved to `target/release-evidence/sbom/` (the testkit's is
   discarded), and `cargo package --locked --workspace --exclude stonecrab-testkit
   --list` (plus one list per crate) beside them. The `release-evidence` workflow
   (`.github/workflows/release-evidence.yml`, `workflow_dispatch` only, read-only
   permissions) runs the same script and uploads the directory as the `release-evidence`
   artifact, to be attached to the draft pre-release. The script replaces the checkout path in the SBOM with `.` and fails if an absolute
   home or temp path remains. The SBOM lists dependencies and is
   not signed; it makes no vulnerability claim.
7. **Mutation evidence.** `mutants.toml` scopes `cargo mutants` to the modules named in
   CODING_STANDARDS 13.9 and the plan: core `value` and `uri`; driver-sim `address` and
   `driver` (its configuration parser); modbus `address`, `pdu`, `framing`, `decode`,
   `batch`, `driver` and `options` (the configuration parser and its parameters); sensors
   `measurement/scale`. It uses nextest with `--fail-fast` for the test phase (the nextest
   profile keeps running after a failure, which turned caught mutants into timeouts),
   `--all-features` and `--locked`, runs in a scratch copy of the tree (never
   `--in-place`) and tests only the mutated package. `mise run mutants` runs
   `scripts/mutants.sh`, which stores only `outcomes.json` under `docs/evidence/mutants/`.
   A surviving mutant is killed by a test or listed in `exclude_re` of `mutants.toml` with
   a reason a reviewer can check (cargo-mutants 27.1.0 has no `[exclusions]` table;
   `exclude_re` is its mechanism). The kill rate per file is recorded in
   `docs/evidence/alpha.md`. One recorded run is the alpha evidence; the 80% mutation
   figure of the plan is not a gate at the alpha and the measured rates replace it
   (decision 11). The weekly `scheduled` job of `ci.yml` keeps its no-op mutation step
   until the maintainer enables `cargo mutants --config mutants.toml` there, which is a CI
   change under the waiver.
8. **Fuzz evidence.** The seven targets run at least 30 minutes each, concurrently, one
   single-threaded libFuzzer process per target, with `scripts/fuzz-evidence.sh` (the
   command per target is `cd fuzz && env -u RUSTUP_TOOLCHAIN cargo fuzz run --target
   <host triple> <target> <scratch corpus> -- -max_total_time=1800`). Each target gets
   its own copy of its seed corpus under `fuzz/target/`, as `scripts/fuzz-smoke.sh`
   does, so libFuzzer's new inputs never enter the committed `fuzz/corpus/`. The run's
   durations, final run counts, final coverage counters and the number of crash
   artifacts are tabulated in `docs/evidence/alpha.md`, not the raw logs. A crash is a
   finding: it is reproduced, its input committed as a corpus seed with a regression test
   named `fuzz_<target>_<short-hash>` in the owning crate, and reported (CONTRIBUTING,
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
