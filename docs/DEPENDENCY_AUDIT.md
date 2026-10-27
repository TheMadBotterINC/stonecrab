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

## proptest 1.12.0 (dev-dependency of stonecrab-codec and stonecrab-core)
- Purpose: property tests of the codec (`prop_round_trip_exact_integers`,
  `prop_length_checked_before_allocation`) and of the connection URI parser in
  `stonecrab-core` (`crates/stonecrab-core/tests/uri_properties.rs`), with shrinking and the tracked
  `proptest-regressions/` files of CODING_STANDARDS 13.3.
- License: MIT OR Apache-2.0.
- Features: `std` only; default features are disabled in `[workspace.dependencies]`
  (ADR-0009). The `fork` and `timeout` features stay off: they pull `rusty-fork` and
  `tempfile`, and through them `once_cell`. In 1.12.0 the `std` feature enables only
  `rand/std`, `rand/sys_rng`, `regex-syntax` and `num-traits/std`; it does not enable
  `lazy_static`. The D23 check (`cargo tree -e normal,dev -i once_cell -i lazy_static -i log
  -i time -i chrono`) matches no package, so no `deny.toml` wrapper is needed (ADR-0013).
- Reaches only dev builds of `stonecrab-codec` and `stonecrab-core`: it is in neither
  crate's `[dependencies]`, not in the packaged manifests' runtime graph, and absent from the fuzz workspace lock.
- Resolved transitive crates in `Cargo.lock`, all MIT OR Apache-2.0: `rand` 0.10.3,
  `rand_core` 0.10.1, `rand_xorshift` 0.5.0, `chacha20` 0.10.2, `getrandom` 0.4.3,
  `cfg-if`, `cpufeatures` and `core_detect` (CPU feature probes), `libc` and `r-efi`
  (platform bindings), `num-traits`, `regex-syntax`, `bitflags`, `unarray` and `autocfg`
  (build script).
- Unsafe: `proptest` itself has no `forbid(unsafe_code)` and contains a handful of
  `unsafe` uses (9 occurrences of the keyword in its sources on 2026-10-27). `unarray`,
  `chacha20`, `getrandom` and `libc`-level code carry the larger share, as expected for
  array initialization, SIMD and OS entropy. This is test-only code; first-party code
  stays `forbid(unsafe_code)`. No native code beyond `libc` bindings is built.
- Alternatives: `quickcheck` (smaller, weaker shrinking and no regression files),
  hand-written exhaustive loops (kept in addition for boundaries), and `cargo fuzz` (used
  for the parser, but it does not shrink or run in `cargo test`).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.
