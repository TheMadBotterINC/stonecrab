# Dependency review
Initial review: 2026-10-27. Versions are resolved in `Cargo.lock`.

## thiserror 2.0.21
- Purpose: error derives and source chaining required by the coding standards.
- License: MIT OR Apache-2.0.
- Features: `std`; default features disabled.
- First-party error code remains safe; the derive implementation and its compiler
  dependencies execute at build time. Treat the procedural macro chain as trusted
  build tooling, not as a reason to relax first-party unsafe prohibition.
- `cargo-deny` 0.20.2 passed advisory, license, ban and registry-source checks on
  2026-10-27. Repeat at release; this does not certify memory safety of the tree.
- Alternatives: handwritten Error/Display implementations are possible, but the
  normative standards specify `thiserror` 2.
