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

## bytes 1.12.1
- Purpose: retain bounded original protocol-status bytes in the representation
  specified by the architecture, with cheap shared immutable clones.
- License: MIT.
- Features: `std`; default features disabled.
- The crate encapsulates unsafe buffer/reference-counting machinery. First-party
  code only uses checked length validation, `Bytes::copy_from_slice`, borrowing
  and cloning. No raw pointer or native-code interface is introduced.
- Alternatives: an owned standard-library slice could store the bytes, but would
  introduce a separate representation from the specified shared-buffer layer.
- Review date: 2026-10-27. Dependency policy is checked with `cargo-deny`; this
  records the trust boundary, not a memory-safety certification.

## libfuzzer-sys 0.4.13 (fuzz workspace only)
- Purpose: the `fuzz_target!` entry point and the bundled LLVM libFuzzer runtime used by
  `cargo fuzz`.
- License: `(MIT OR Apache-2.0) AND NCSA` (NCSA, an OSI-approved permissive license,
  covers the vendored libFuzzer sources). NCSA is therefore added to the allow list of
  `fuzz/deny.toml` only; the root `deny.toml` is unchanged.
- Declared in `fuzz/Cargo.toml` only (ADR-0009), never in the root `[workspace.dependencies]`
  and never in a published crate or its dev-dependencies. The fuzz workspace is excluded
  from the root workspace and has its own `Cargo.lock` and `fuzz/deny.toml`.
- Native code: its build script compiles the vendored libFuzzer C++ sources with `cc`
  (needs a C++ compiler) and the crate links them with unsafe FFI. It requires a nightly
  toolchain for the sanitizer coverage flags, pinned in `fuzz/rust-toolchain.toml`.
  First-party code in `fuzz/` remains `forbid(unsafe_code)`.
- Build-time transitive crates in `fuzz/Cargo.lock`: `cc`, `jobserver`, `libc`, `shlex`,
  `find-msvc-tools`, `getrandom`, `cfg-if`, `r-efi` (platform-gated). All pass the fuzz
  `cargo deny` check; none of them reach the root lock.
- Alternatives: `honggfuzz` and `afl` need external drivers or runtimes; `cargo fuzz`
  with libFuzzer is the tool the coding standards name (section 13.8).
- Review date: 2026-10-27. The deny check does not certify memory safety.

## arbitrary 1.5.0 (fuzz workspace only)
- Purpose: derive structured inputs from fuzzer bytes (`Arbitrary`, `Unstructured`).
- License: MIT OR Apache-2.0.
- Declared in `fuzz/Cargo.toml` only (ADR-0009); no published crate depends on it. The
  `derive` feature is not enabled; targets that need it enable it at the slice that adds
  them, with a note here.
- No native code. The crate is dependency-free in this configuration.
- Alternatives: hand-parsing `&[u8]` in each target; rejected for structured inputs.
- Review date: 2026-10-27.
