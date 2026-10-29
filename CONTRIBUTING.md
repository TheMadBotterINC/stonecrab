# Contributing
Read `docs/SPEC.md`, `docs/ARCHITECTURE.md` and `docs/CODING_STANDARDS.md` before
changing public contracts. The retained `docs/PROJECT_BRIEF.md` describes the target,
not shipped functionality.

Stonecrab is one crate, `crates/stonecrab`, whose parts are modules behind Cargo features
(ADR-0018). Use `mise install`, then `mise run check` and `mise run coverage`.
Use `mise run dependencies` to check advisories, licenses, bans and sources.
`mise run lint-attrs`, `features`, `machete`, `parity` and `dco -- <range>` run the CI jobs
of the same names; `mise run coverage` applies the per-module thresholds of ADR-0005.
`mise run parity` (`scripts/parity-check.sh`) fails when a `# Compatibility:
docs/parity/<file>.md#<ROW-ID>` section in `crates/*/src` names a row that does not exist,
or when a row marked `implemented` or `automated-verified` in `docs/parity/*.md` has no
test id or is named by no `# Compatibility` section or test, or when such a row (or a
divergence row) cites a test name that no Rust source, example or fuzz target contains;
a change to a parity row and to the code it describes goes in the same PR, and so does a
test rename and the records that cite the test.
Unit tests and doctests must also pass with `cargo test --all-features` (what `mise run
check` runs). A plain `cargo test` builds only the default features, leaves out the
`testkit` feature and skips the test files that need it, so it runs the reduced suite; the
test kit's own tests read repository files and so must run from a checkout.
A new module takes its row in `scripts/lint-attrs.sh` and `scripts/coverage-gate.sh` in the
same PR.
Tests must not access devices or external networks. Use explicit fake time rather
than sleeps, and bind loopback integration listeners to an ephemeral port.

Keep changes reviewable. Add regression tests for fixes and boundary/failure tests
for new behavior. Do not exclude error paths from coverage.
Use Conventional Commits and DCO sign-off (`git commit -s`).
The CI `geiger` job greps first-party Rust sources for the word `unsafe`; it may
appear only in the `forbid(unsafe_code)` attribute, so keep it out of comments,
strings and test names too.
ADRs document new modules and features, public traits and dependency decisions.
Never place credentials in source, fixtures, logs or commit messages.

## Workflow

The rules below mirror `docs/CODING_STANDARDS.md` section 18; the standards are
normative and are not duplicated here. Per ADR-0004 the trunk is `master`.

- Branching (18.1): short-lived branches named `<type>/<scope>-<slug>`; no long-lived
  feature branches.
- Commits (18.2): Conventional Commits with the scopes listed in the standards.
- DCO (18.3): every commit carries `Signed-off-by:`; there is no CLA.
- One concern per PR (18.4): split regenerations from logic changes and reformatting
  from fixes; keep PRs reviewable in one sitting.
- PR requirements (18.5): fill in `.github/PULL_REQUEST_TEMPLATE.md`; the PR title
  follows the commit format.
- Review (18.6): one maintainer approval, two for CI, release tooling or hardware bench
  changes; authors do not approve their own PRs.
  ADR-0005 waives the two-approval rule while there is one maintainer; each PR then
  carries a self-review against the Appendix C checklist.
- Merge (18.7): squash-merge with the PR title as the subject; no merge commits or
  force-pushes to the trunk.
- New drivers (18.8): capability metadata, address documentation, fixtures with
  provenance, mandatory failure-case tests, contract-suite run and support-level
  evidence, or the PR stays a draft.
- Bug fixes (18.9): carry a regression test named after the issue, or a documented
  reason it can only be shown on hardware, with the hardware record linked.

`mise run fuzz` runs the fuzz smoke check (see `fuzz/README.md`), 60 seconds per target.
`mise run interop` runs the pymodbus interoperability job and needs Docker; its PR template
item may be marked n/a when the change cannot affect a driver. Commit any
`proptest-regressions/` directories that property tests create.

## Changing the upstream baseline

Follow `docs/SPEC.md` "Code generation and upstream maintenance". A baseline change is
its own PR and contains:

1. The `upstream/manifest.toml` change, alone, with no logic changes.
2. A parity diff over `docs/parity` showing every capability whose status changes.
3. A full regression run, including interop and fuzz-smoke.
4. A migration note in `CHANGELOG.md` under `Unreleased`.

## Releases

Releases, signed tags, the yank procedure and the release evidence are described in
`docs/adr/0017-release-process-and-evidence.md`. Only the maintainer publishes the crate or
pushes the tag, each with a separate approval.

## Fuzz findings

A crash or hang found by fuzzing is handled per `docs/CODING_STANDARDS.md` section
13.8. Commit the reproducing input under `fuzz/corpus/<target>/` and add a regression
test named `fuzz_<target>_<short-hash>` in the crate. That test is merged before
or together with the fix. Never commit inputs that contain credentials or private device
data.
