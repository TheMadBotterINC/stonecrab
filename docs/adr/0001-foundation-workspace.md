# 0001: Runtime-independent foundation
## Status
Superseded in part by ADR-0018: the workspace crate graph below (separate crates for the
core, drivers, transport, codec and testkit) is replaced by one crate, `stonecrab`, with
the former crates as modules; the text is the original decision.
Accepted for the initial development slice.
## Context
The published package contains only a placeholder. The supplied architecture directs
dependencies downward and requires core contracts to remain executor-independent.
The source documents are retained unchanged under `docs/`; their diagrams are targets.
## Decision
Create only `crates/stonecrab` and `crates/stonecrab-core` initially. The facade
re-exports core; the old root library and placeholder are deleted, without a shim.
Use `error.rs`, `capability.rs`, `tag.rs`, `deadline.rs`, `secret.rs` and `value.rs`
for distinct pure contracts. Add drivers, transport, codec and testkit only with
working implementations and applicable tests. No public driver trait is introduced
until the simulator can exercise it.
Use the specified Rust 1.99.1 toolchain through mise and rustup, with MSRV 1.97.
Require 100% executable handwritten line coverage for this initial slice; measure
each crate independently as soon as multiple crates contain executable logic.
## Dependencies
`thiserror` 2, Apache-2.0 OR MIT, with default features disabled and `std` enabled,
implements the mandated typed error derives and source chaining. Its macro tooling
executes at build time only; no async runtime is introduced. Record lockfile versions
and dependency review separately.
## Consequences
Facade 0.2.0 is an unreleased breaking change from the published placeholder API.
Calendar types, recursive collections, cancellation and connection contracts remain
design work rather than empty APIs. No parity or hardware claim follows from tests
of these primitives. Full M0 source inventory remains a separate gate.
