# Contributing
Read `docs/SPEC.md`, `docs/ARCHITECTURE.md` and `docs/CODING_STANDARDS.md` before
changing public contracts. The retained `docs/PROJECT_BRIEF.md` describes the target,
not shipped functionality.

Use `mise install`, then `mise run check` and `mise run coverage`.
Use `mise run dependencies` to check advisories, licenses, bans and sources.
Unit tests and doctests must also pass with ordinary `cargo test --workspace`.
Tests must not access devices or external networks. Use explicit fake time rather
than sleeps, and bind loopback integration listeners to an ephemeral port.

Keep changes reviewable. Add regression tests for fixes and boundary/failure tests
for new behavior. Do not exclude error paths from coverage.
Use Conventional Commits and DCO sign-off (`git commit -s`).
ADRs document new crates, public traits and dependency decisions.
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
- Merge (18.7): squash-merge with the PR title as the subject; no merge commits or
  force-pushes to the trunk.
- New drivers (18.8): capability metadata, address documentation, fixtures with
  provenance, mandatory failure-case tests, contract-suite run and support-level
  evidence, or the PR stays a draft.
- Bug fixes (18.9): carry a regression test named after the issue, or a documented
  reason it can only be shown on hardware, with the hardware record linked.

The `interop` and `fuzz` mise tasks arrive in later slices of `docs/ALPHA_PLAN.md`;
until then the PR template items for them are marked n/a.

## Changing the upstream baseline

Follow `docs/SPEC.md` "Code generation and upstream maintenance". A baseline change is
its own PR and contains:

1. The `upstream/manifest.toml` change, alone, with no logic changes.
2. A parity diff over `docs/parity` showing every capability whose status changes.
3. A full regression run, including interop and fuzz-smoke once those exist.
4. A migration note in `CHANGELOG.md` under `Unreleased`.

## Fuzz findings

A crash or hang found by fuzzing is handled per `docs/CODING_STANDARDS.md` section
13.8. Commit the reproducing input under `fuzz/corpus/<target>/` and add a regression
test named `fuzz_<target>_<short-hash>` in the owning crate. That test is merged before
or together with the fix. Never commit inputs that contain credentials or private device
data.
