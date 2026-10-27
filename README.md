# Stonecrab
**Apache PLC4X, carcinized.** An independent native Rust port, built for sensor acquisition first.
Copyright 2026 The Mad Botter INC and the Stonecrab contributors. Apache-2.0.

## Current status
See [`STATUS.md`](STATUS.md) for the current engineering handoff, verification
evidence, known gaps and continuation steps.
The published 0.1.0 crate is specification-stage only. The unreleased 0.2.0 facade
and 0.1.0 core establish the initial workspace. No protocol drivers, network I/O,
sensor profiles or hardware support are implemented. Upstream compatibility is not claimed.

The original [project brief](docs/PROJECT_BRIEF.md), [specification](docs/SPEC.md),
[architecture](docs/ARCHITECTURE.md), and [coding standards](docs/CODING_STANDARDS.md)
are retained as source documents. Their API examples and diagrams describe targets,
not shipped functionality. See [current scope](docs/parity/core.md).

## Development
Install [mise](https://mise.jdx.dev/), then run `mise install`.
Use `mise run check` for formatting, strict Clippy, tests, doctests and docs;
`mise run coverage` enforces the per-crate line-coverage thresholds of ADR-0005 (core
100%, testkit 90%); `lint-attrs`, `features`, `machete` and `dco` run the other CI gates.
The pinned development compiler is Rust 1.99.1, edition 2024; MSRV is 1.97.

The facade is in `crates/stonecrab`, pure contracts in `crates/stonecrab-core`.
Both have empty default features. No executor is required by core.
See [contributing](CONTRIBUTING.md), [security](SECURITY.md),
[license](LICENSE), [notices](NOTICE), and [changelog](CHANGELOG.md).
Stonecrab is not affiliated with the Apache Software Foundation or Apache PLC4X.
