# Contributing
Read `docs/SPEC.md`, `docs/ARCHITECTURE.md` and `docs/CODING_STANDARDS.md` before
changing public contracts. The retained `docs/PROJECT_BRIEF.md` describes the target,
not shipped functionality.

Use `mise install`, then `mise run check` and `mise run coverage`.
Unit tests and doctests must also pass with ordinary `cargo test --workspace`.
Tests must not access devices or external networks. Use explicit fake time rather
than sleeps, and bind loopback integration listeners to an ephemeral port.

Keep changes reviewable. Add regression tests for fixes and boundary/failure tests
for new behavior. Do not exclude error paths from coverage.
Use Conventional Commits and DCO sign-off (`git commit -s`).
ADRs document new crates, public traits and dependency decisions.
Never place credentials in source, fixtures, logs or commit messages.
