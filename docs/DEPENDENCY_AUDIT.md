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
- Maintenance: 1.54.1 is the current 1.x release as resolved on the review date, and
  `cargo-deny` reports no open RUSTSEC advisory against it or its transitive crates.
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## tokio 1.54.1 (stonecrab-transport)
- Purpose: the async runtime pieces the transport needs: `TcpStream` and `lookup_host`
  (`net`), `timeout_at` and `sleep_until` (`time`), `read_buf` and `write_all`
  (`io-util`), and the runtime driver (`rt`). `std` has no async sockets or timers, and
  blocking sockets on a thread cannot honour a deadline (CODING_STANDARDS 8.4, ADR-0014).
- License: MIT.
- Features: `default-features = false` in `[workspace.dependencies]`
  (`tokio = "1.54"`). `stonecrab-transport` enables `rt`, `time`, `sync` and `io-util`,
  and `net` through its default `tcp` feature; dev-dependencies add `macros`, `test-util`
  and `rt-multi-thread`. `sync` has no consumer in this slice (it adds no crate; the
  connection guard of S19 uses it). `full` is banned by `[[bans.features]]` in
  `deny.toml` and the ban was verified to fire (ADR-0014).
- Unsafe: tokio is not `forbid(unsafe_code)` and uses `unsafe` heavily in the task
  harness, the I/O driver and the timer wheel (about 1,100 occurrences of the keyword in
  its sources on 2026-10-27); `mio` (about 175), `socket2` (about 240) and `pin-project-lite`
  (13) add more, as expected for readiness polling, raw sockets and pin projection.
  First-party code stays `forbid(unsafe_code)`. No native code is compiled: the only
  build script in the tree is `libc`'s, which probes the target and links nothing.
- Transitive crates in this configuration (normal graph, `tcp` on): `bytes` 1.12.1
  (already audited above), `libc` 0.2.191 (MIT OR Apache-2.0), `mio` 1.2.4 (MIT),
  `socket2` 0.6.5 (MIT OR Apache-2.0), `pin-project-lite` 0.2.17 (Apache-2.0 OR MIT);
  `windows-sys` 0.61.2 and `windows-link` 0.2.1 (MIT OR Apache-2.0) on Windows targets and
  `wasi` 0.11.1 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) on WASI. Dev builds
  add `tokio-macros` 2.7.2 (MIT; `proc-macro2`, `quote` and `syn` are already present
  through `thiserror`). With `--no-default-features` only `bytes` and `pin-project-lite`
  remain.
- Banned crates: the D23 check (`cargo tree -e normal,dev -i once_cell -i lazy_static -i
  log -i time -i chrono`, and each name alone) matches no package, so no `deny.toml`
  wrapper is needed (ADR-0014).
- Alternatives: `async-std` and `smol` (smaller ecosystems and not the runtime named in
  section 15.8), `mio` alone (would re-implement the timer and the resolver pool) and
  blocking `std::net` on a worker thread (cannot be cancelled by a deadline).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## tracing 0.1.44 (stonecrab-driver-sim, workspace-wide adoption)
- Purpose: structured, level-filtered diagnostics that cost nothing without a subscriber.
  The coding standards (section 14) name `tracing` as the one diagnostics API; the print
  macros are denied and `log` is banned. The simulator emits `debug` events when a tag is
  set, removed or made a ramp; S11b-2 adds the per-operation spans (`scheme`, redacted
  `endpoint`, `op`, `request_id`). Library crates never install a subscriber (ADR-0012).
- License: MIT.
- Features: `default-features = false`, `std` only, in `[workspace.dependencies]`
  (`tracing = "0.1"`: the 0.1 series has no minor bumps, and 0.1.44 resolved on the review
  date). The default `attributes` feature is deliberately off: it adds `tracing-attributes`
  and a second `syn` major version (2.0.119) beside the `syn` 3.0.9 that `thiserror-impl`
  uses, and `clippy::multiple_crate_versions` then fails the `-D warnings` Clippy gate
  for the one crate that depends on both. Nothing in the simulator needs `#[instrument]`
  (manual spans and `Instrument` are in `tracing` itself). Adopt `attributes` later with
  an `allowed-duplicate-crates = ["syn"]` entry in `clippy.toml` and a note here.
  The `log`, `valuable`, `async-await` and `release_max_level_*` features are off.
- Unsafe: `tracing` is not `forbid(unsafe_code)`; about 7 occurrences of the keyword in its
  sources on 2026-10-27 (callsite registration, `Sync` impls), and `tracing-core` about 33
  (callsite registry, dispatcher and `Dispatch` weak pointers). First-party code stays
  `forbid(unsafe_code)`. No native code and no build script.
- Transitive crates in this configuration (normal graph): `tracing-core` 0.1.36 (MIT),
  `pin-project-lite` 0.2.17 (Apache-2.0 OR MIT; already audited under tokio) and
  `once_cell` 1.21.4 (below). `tracing-attributes`, `syn` 2, `quote` and `proc-macro2`
  do not appear for `tracing` (they would with `attributes`).
- Banned crates: the D23 check finds exactly one path, `once_cell` through `tracing-core`
  (see the next two entries and ADR-0012); `lazy_static`, `log`, `time` and `chrono`
  match no package.
- Alternatives: `log` (banned; no spans, no structured fields), `slog` (own ecosystem,
  heavier, not the API the standards name) and no diagnostics (the standards require
  spans on every operation).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks
  with the `once_cell` wrapper; this records the trust boundary, not a memory-safety
  certification.

## tracing-core 0.1.36 (via tracing)
- Purpose: the callsite registry, `Dispatch` and `Subscriber` interfaces that `tracing`
  macros expand to. Pulled by `tracing` only.
- License: MIT.
- Features: `std` (enabled by `tracing/std`), which enables its optional `once_cell`
  dependency and the thread-safe callsite registry; `valuable` is off.
- Unsafe: not `forbid(unsafe_code)`; about 33 occurrences of the keyword in its sources.
  Third-party code, outside the first-party prohibition.
- Role in `deny.toml`: the only wrapper allowed to depend on `once_cell`
  (`wrappers = ["tracing-core"]`).
- Alternatives: none separate from `tracing`; see above.
- Review date: 2026-10-27.

## once_cell 1.21.4 (via tracing-core, banned for first-party use)
- Purpose: lazy and one-time initialization inside `tracing-core`'s callsite registry.
  Reached only through `tracing-core/std`; no first-party crate depends on it.
- License: MIT OR Apache-2.0.
- Features: the resolved set on 2026-10-27 is `alloc`, `default`, `race` and `std` (from
  `cargo tree -f '{p} {f}'`); nothing in this workspace selects them.
- Policy: `deny.toml` keeps the ban with `reason = "use std lazy initialization in
  first-party code"` and `wrappers = ["tracing-core"]` (decision D23). Removing the
  wrapper makes `cargo deny check` fail with `crate 'once_cell = 1.21.4' is explicitly
  banned`; that was verified on 2026-10-27 and the wrapper restored. Any other crate that
  starts depending on `once_cell` fails the same check and needs its own wrapper entry and
  audit entry.
- Unsafe: not `forbid(unsafe_code)`; about 51 occurrences of the keyword in its sources
  (the `unsync`/`sync` cell internals). Third-party code.
- Alternatives: `std::sync::OnceLock` and `LazyLock` replace it in first-party code (the
  reason for the ban); `tracing-core` without `std` would avoid the dependency but loses
  the thread-safe registry the simulator's shared device needs under a real subscriber.
- Review date: 2026-10-27.
