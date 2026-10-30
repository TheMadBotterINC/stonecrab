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

## proptest 1.12.0 (dev-dependency of stonecrab)
- Purpose: property tests of the codec (`prop_round_trip_exact_integers`,
  `prop_length_checked_before_allocation`) and of the connection URI parser in
  core (`crates/stonecrab/tests/core_uri_properties.rs`), with shrinking and the tracked
  `proptest-regressions/` files of CODING_STANDARDS 13.3.
- License: MIT OR Apache-2.0.
- Features: `std` only; default features are disabled in `[workspace.dependencies]`
  (ADR-0009). The `fork` and `timeout` features stay off: they pull `rusty-fork` and
  `tempfile`, and through them `once_cell`. In 1.12.0 the `std` feature enables only
  `rand/std`, `rand/sys_rng`, `regex-syntax` and `num-traits/std`; it does not enable
  `lazy_static`. The D23 check (`cargo tree -e normal,dev -i once_cell -i lazy_static -i log
  -i time -i chrono`) matches no package, so no `deny.toml` wrapper is needed (ADR-0013).
- Reaches only dev builds of `stonecrab`: it is not in the crate's `[dependencies]`, not
  in the packaged manifest's runtime graph, and absent from the fuzz workspace lock.
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

## tokio 1.54.1 (stonecrab, feature `transport`)
- Purpose: the async runtime pieces the transport needs: `TcpStream` and `lookup_host`
  (`net`), `timeout_at` and `sleep_until` (`time`), `read_buf` and `write_all`
  (`io-util`), and the runtime driver (`rt`). `std` has no async sockets or timers, and
  blocking sockets on a thread cannot honour a deadline (CODING_STANDARDS 8.4, ADR-0014).
- License: MIT.
- Features: `default-features = false` in `[workspace.dependencies]`
  (`tokio = "1.54"`). `stonecrab` enables `rt`, `net`, `time`, `sync` and `io-util`
  behind its `transport` feature (the `testkit` feature adds `macros`); dev-dependencies
  add `macros`, `test-util` and `rt-multi-thread`. `sync` has no consumer in this slice
  (it adds no crate; the connection guard of S19 uses it). `full` is banned by `[[bans.features]]` in
  `deny.toml` and the ban was verified to fire (ADR-0014).
- Unsafe: tokio is not `forbid(unsafe_code)` and uses `unsafe` heavily in the task
  harness, the I/O driver and the timer wheel (about 1,100 occurrences of the keyword in
  its sources on 2026-10-27); `mio` (about 175), `socket2` (about 240) and `pin-project-lite`
  (13) add more, as expected for readiness polling, raw sockets and pin projection.
  First-party code stays `forbid(unsafe_code)`. No native code is compiled: the only
  build script in the tree is `libc`'s, which probes the target and links nothing.
- Transitive crates in this configuration (normal graph, `transport` on): `bytes` 1.12.1
  (already audited above), `libc` 0.2.191 (MIT OR Apache-2.0), `mio` 1.2.4 (MIT),
  `socket2` 0.6.5 (MIT OR Apache-2.0), `pin-project-lite` 0.2.17 (Apache-2.0 OR MIT);
  `windows-sys` 0.61.2 and `windows-link` 0.2.1 (MIT OR Apache-2.0) on Windows targets and
  `wasi` 0.11.1 (Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT) on WASI. Dev builds
  add `tokio-macros` 2.7.2 (MIT; `proc-macro2`, `quote` and `syn` are already present
  through `thiserror`). With `--no-default-features` (or any feature set without
  `transport`) none of them is in the graph.
- Banned crates: the D23 check (`cargo tree -e normal,dev -i once_cell -i lazy_static -i
  log -i time -i chrono`, and each name alone) matches no package, so no `deny.toml`
  wrapper is needed (ADR-0014).
- Alternatives: `async-std` and `smol` (smaller ecosystems and not the runtime named in
  section 15.8), `mio` alone (would re-implement the timer and the resolver pool) and
  blocking `std::net` on a worker thread (cannot be cancelled by a deadline).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## tracing 0.1.44 (stonecrab, features `sim` and `modbus`)
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
  for the one crate that would depend on both. Nothing in the simulator needs
  `#[instrument]` (manual spans and `Instrument` are in `tracing` itself). Adopt `attributes` later with
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
- Role in `deny.toml`: a wrapper allowed to depend on `once_cell`
  (`wrappers = ["tracing-core", "rustls"]`; `rustls` since S71, entry below).
- Alternatives: none separate from `tracing`; see above.
- Review date: 2026-10-27.

## once_cell 1.21.4 (via tracing-core and, with feature `tls`, rustls; banned for first-party use)
- Purpose: lazy and one-time initialization inside `tracing-core`'s callsite registry, and,
  since S71, inside `rustls` (its process-wide default crypto provider, through
  `once_cell::race`). Reached only through `tracing-core/std` and `rustls`; no first-party
  crate depends on it.
- License: MIT OR Apache-2.0.
- Features: the resolved set on 2026-10-27 is `alloc`, `default`, `race` and `std` (from
  `cargo tree -f '{p} {f}'`); nothing in this workspace selects them.
- Policy: `deny.toml` keeps the ban with `reason = "use std lazy initialization in
  first-party code"` and `wrappers = ["tracing-core", "rustls"]` (decision D23; `rustls`
  added by S71 on 2026-10-29: without it `cargo deny check` fails with `crate 'once_cell =
  1.21.4' is explicitly banned`, which was verified). Removing the
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

## tracing-subscriber 0.3.23 (stonecrab, dev-dependency)
- Purpose: the `fmt` subscriber that `examples/sim_read.rs` installs to print structured
  events (ADR-0011 decision 8, ADR-0014 addendum for S14). Examples and integration
  tests only: it is a `[dev-dependencies]` entry, so it never reaches the packaged runtime graph, and
  library crates still never install a subscriber (ADR-0012).
- License: MIT.
- Features: `[workspace.dependencies]` has `tracing-subscriber = { version = "0.3",
  default-features = false, features = ["fmt", "std"] }` (the 0.3 series is the minor-level
  range of 0.3.23, the version resolved on the review date). The resolved set is `alloc`,
  `fmt`, `registry`, `sharded-slab`, `std` and `thread_local`. `tracing-log` (and therefore
  `log`, which is banned), `ansi` (`nu-ansi-term`), `smallvec`, `env-filter` (`regex`),
  `json` (`serde`), `chrono`, `time` and `local-time` stay off.
- Unsafe: not `forbid(unsafe_code)`; about 28 occurrences of the keyword in its sources.
  Third-party code, outside the first-party prohibition. No native code, no build script.
- Transitive crates added to `Cargo.lock`: `sharded-slab` 0.1.7, `lazy_static` 1.5.1 and
  `thread_local` 1.1.10 (entries below); `tracing-core`, `cfg-if` and `tracing` were
  already present.
- Banned crates: the D23 check finds `lazy_static` through `sharded-slab` (wrapped, below)
  and the existing `once_cell` through `tracing-core`; `log`, `time` and `chrono` match no
  package.
- Alternatives: a hand-written `Subscriber` (more first-party code to audit for no gain),
  and `println!` in the example (denied by `clippy.toml`, ADR-0011).
- Review date: 2026-10-27. `cargo-deny` passes with the `lazy_static` wrapper; this records
  the trust boundary, not a memory-safety certification.

## sharded-slab 0.1.7 (via tracing-subscriber)
- Purpose: the lock-free slab behind `tracing-subscriber`'s `registry` (span storage), which
  the `fmt` feature requires.
- License: MIT.
- Features: none selected.
- Unsafe: not `forbid(unsafe_code)`; about 74 occurrences of the keyword in its sources
  (the slab's atomic page and slot handling). Third-party code, reached from dev builds of
  the integration tests and examples only.
- Role in `deny.toml`: the only wrapper allowed to depend on `lazy_static`
  (`wrappers = ["sharded-slab"]`).
- Alternatives: none separate from `tracing-subscriber`'s `fmt`; dropping `fmt` would
  also drop the formatter the example uses.
- Review date: 2026-10-27.

## lazy_static 1.5.1 (via sharded-slab, banned for first-party use)
- Purpose: lazy statics inside `sharded-slab`. Reached only through `sharded-slab`; no
  first-party crate depends on it.
- License: MIT OR Apache-2.0.
- Features: none (`spin_no_std` is off).
- Policy: `deny.toml` keeps the ban with `reason = "use std lazy initialization in
  first-party code"` and `wrappers = ["sharded-slab"]` (decision D23). Removing the wrapper
  makes `cargo deny check bans` fail with `crate 'lazy_static = 1.5.1' is explicitly
  banned`; that was verified on 2026-10-27 and the wrapper restored. Any other crate that
  starts depending on `lazy_static` fails the same check and needs its own wrapper entry
  and audit entry.
- Unsafe: about 2 occurrences of the keyword in its sources. Third-party code.
- Alternatives: `std::sync::LazyLock` replaces it in first-party code (the reason for the
  ban).
- Review date: 2026-10-27.

## thread_local 1.1.10 (via tracing-subscriber, not banned)
- Purpose: per-thread storage for `tracing-subscriber`'s `registry` (the current span
  stack). The plan named a `thread_local` path as possible and UNVERIFIED; it exists.
- License: MIT OR Apache-2.0.
- Dependencies: `cfg-if` only (already audited via `tokio` and `tracing`). It does not
  depend on `once_cell`, so no further banned crate comes with it, and `deny.toml` bans
  nothing named `thread_local`, so no wrapper entry is needed.
- Unsafe: not `forbid(unsafe_code)`; about 27 occurrences of the keyword in its sources.
  Third-party code.
- Alternatives: `std::thread_local!` for first-party needs; this crate is reached only
  through `tracing-subscriber`.
- Review date: 2026-10-27.

## serde 1.0.229 (stonecrab, feature `sensors`)
- Purpose: the `Deserialize` derive for the sensor profile schema types (ADR-0016). Profiles
  are read, never written, so `Serialize` is not derived and no format crate other than
  `toml` is involved.
- License: MIT OR Apache-2.0.
- Features: `[workspace.dependencies]` has `serde = { version = "1.0", default-features =
  false, features = ["std"] }`; `stonecrab` enables `derive`. The `rc` and `alloc`
  features stay off.
- Transitive crates, all MIT OR Apache-2.0: `serde_core` 1.0.229 (the trait definitions that
  `serde` re-exports; `toml` depends on it directly) and `serde_derive` 1.0.229, a procedural
  macro that runs at build time with `syn` 3.0.9, `quote` 1.0.47, `proc-macro2` 1.0.107 and
  `unicode-ident` 1.0.27. The last four were already in the tree through `thiserror-impl`, so
  they add no new code; treat the macro chain as trusted build tooling, as for `thiserror`.
- Unsafe: first-party code stays `forbid(unsafe_code)`. `serde` and `serde_core` mention the
  keyword twice each in their sources (on 2026-10-27); `serde_derive` not at all. Both
  `serde` and `serde_core` have a `build.rs` that probes the compiler version and links
  nothing.
- Alternatives: a hand-written reader for the schema (rejected: it would re-implement
  `deny_unknown_fields`, defaults and enum tagging for every field), and `miniserde` (no
  `deny_unknown_fields` and no `toml` support).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## toml 1.1.8 (stonecrab, features `sensors` and `testkit`)
- Purpose: the TOML reader behind `SensorProfile::from_toml`. TOML is the format the
  specification names for profiles ("initially TOML or an equivalently validated
  declarative format"). A hand-written reader would be the attack surface that this
  dependency removes (CODING_STANDARDS 21.1).
- License: MIT OR Apache-2.0.
- Features: `[workspace.dependencies]` has `toml = { version = "1.1", default-features =
  false, features = ["parse", "serde", "std"] }`, all three names verified against the
  1.1.8 manifest. `parse` adds the parser (`toml_parser`, `winnow`), `serde` adds
  `toml::from_str` for derived types, `std` is the standard-library build. `display` (the
  serializer, `toml_writer`) stays off because the library never writes a profile;
  `preserve_order` (`indexmap`), `fast_hash`, `debug` and `unbounded` stay off. The
  crate is optional in `stonecrab`, enabled by the `sensors` feature (on by default) and
  by `testkit`.
- Transitive crates in this configuration: `toml_parser` 1.1.5 (MIT OR Apache-2.0),
  `toml_datetime` 1.1.2 (MIT OR Apache-2.0), `serde_spanned` 1.1.2 (MIT OR Apache-2.0),
  `winnow` 1.0.4 (MIT) and `serde_core` (above). `toml_writer`, `indexmap`, `hashbrown`
  and `equivalent` appear in `Cargo.lock` because Cargo records optional dependencies, but
  no enabled feature builds them. `toml_datetime` pulls neither `time` nor `chrono`: its
  `Datetime` is its own type.
- One `winnow`: `toml` 1.1.8 and `toml_parser` 1.1.5 both use `winnow` 1.0, so
  `clippy::multiple_crate_versions` (a `cargo` group lint, denied by `-D warnings`) stays
  quiet without a lockfile pin. Until 2026-10-29 the crate used `toml` 0.9.12, whose
  `winnow` 0.7 met `toml_parser` 1.1's `winnow` 1.0, and the locks pinned `toml_parser` at
  1.0.9 to keep one copy (ADR-0016). The move to `toml` 1.1 (Dependabot PR #3) dropped the
  pin from `Cargo.lock`, `fuzz/Cargo.lock` and the codegen tool's lock. The one breaking
  change of `toml` 1.0 (`Time` seconds and nanoseconds became `Option`) touches no Stonecrab
  code, no `toml` type is in the public API, and cargo-semver-checks reports no change; the
  MSRV of `toml` 1.1 is Rust 1.85, below the crate's 1.97 (ADR-0016 addendum).
- Unsafe: `toml`, `toml_datetime` and `serde_spanned` are `forbid(unsafe_code)`;
  `toml_parser` is `forbid(unsafe_code)` unless its `unsafe` feature is on (it is off);
  `winnow` 1.0.4 contains `unsafe` (47 occurrences of the keyword in its sources on
  2026-10-29, in its stream and byte-slice helpers; 0.7.15 had 49). No crate compiles native code or has a
  build script. First-party code stays `forbid(unsafe_code)`.
- Resource bounds: the library refuses a document over 64 KiB before the parser sees it
  (`ProfileError::Oversized`), and `toml` 1.1 limits nesting depth by default (the
  `unbounded` feature, which lifts the limit, is off).
- Banned crates: the D23 check (`cargo tree -e normal,dev -i once_cell -i lazy_static -i log
  -i time -i chrono`, and each name alone) matches no package, so no `deny.toml` wrapper is
  needed (ADR-0016).
- Alternatives: `serde_json` or RON profiles (no comments and no readable tables for
  hand-edited register maps), `toml_edit` (preserves formatting, which a read-only loader does
  not need, and is larger), and a hand-written reader (rejected above).
- Review date: 2026-10-27, revisited 2026-10-29 for `toml` 1.1. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## serde_json 1.0.154 (stonecrab, feature `testkit`)
- Purpose: parse the `<name>.expected.json` files of the golden-vector loader into
  `serde_json::Value` (CODING_STANDARDS 13.6, ADR-0009). The files are authored by hand
  from the protocol specification or a capture, and the loader only parses them. The
  crate is declared in `[workspace.dependencies]` and used only by the
  testkit behind the off-by-default feature `testkit`; no default feature enables it
  (ADR-0009 decision 2, ADR-0018).
- License: MIT OR Apache-2.0.
- Features: `[workspace.dependencies]` has `serde_json = { version = "1.0",
  default-features = false, features = ["std"] }`. `std` enables `memchr/std` and
  `serde_core/std`. `preserve_order` (`indexmap`, `foldhash`), `arbitrary_precision`,
  `float_roundtrip`, `raw_value` and `alloc` stay off. The testkit reads JSON only, so
  nothing serializes through `serde_json`.
- Transitive crates in this configuration: `itoa` 1.0.18 (MIT OR Apache-2.0), `memchr`
  2.8.3 (Unlicense OR MIT), `zmij` 1.0.23 (MIT; the float formatter that replaced `ryu`)
  and `serde_core` 1.0.229 (already audited under `serde`).
- Unsafe: `serde_json` is not `forbid(unsafe_code)`; its sources mention the keyword 16
  times (UTF-8 and slice shortcuts in the reader and number formatting), `itoa` 13, `zmij`
  70 (SIMD and `transmute` helpers) and `memchr` 340 (vectorized search). `serde_json` and
  `zmij` have a `build.rs` that only emits `cargo:rustc-cfg` lines (limb width, compiler
  version and `opt_level` probes); neither compiles native code or touches the network.
  First-party code stays `forbid(unsafe_code)`.
- Banned crates: `cargo tree -e normal,dev -i once_cell` finds the one existing path,
  `once_cell` through `tracing-core` (wrapped in `deny.toml`, ADR-0012), and
  the same result holds with `--workspace --all-features -e normal,dev,build`, so
  `serde_json`, `itoa`, `memchr`, `zmij` and the `testkit` feature add no path to it.
  `lazy_static`, `log`, `time` and `chrono` match no package (ADR-0009 addendum).
- Alternatives: `toml` for the expected files (rejected: the expected values are
  structured trees that other tools produce as JSON, and the plan names JSON), `miniserde`
  and `json` (smaller, but no `deny_unknown_fields`-style strictness and not the format
  crate ADR-0009 sanctions), and a hand-written JSON reader (rejected: it would be a
  parser written only to avoid a dependency that never reaches a published crate).
- Review date: 2026-10-27. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## roxmltree 0.21.2 (stonecrab, dev-dependency)
- Purpose: parse the retained PLC4X driver-suite XML files in the test-only adapter
  `crates/stonecrab/tests/modbus_driver_suite_adapter.rs` (slice S04b, ADR-0009 addendum).
  The files are repository fixtures, not network input. The crate is a dev-dependency of
  `stonecrab`, so it is not in the published manifest's dependencies and
  `cargo tree -e normal --all-features -p stonecrab` does not list it.
- License: MIT OR Apache-2.0.
- Features: `[workspace.dependencies]` has `roxmltree = { version = "0.21",
  default-features = false, features = ["std"] }`. `std` enables `memchr/std`;
  `positions` (node positions in the text) stays off.
- Transitive crates in this configuration: `memchr` 2.8.3 only, already audited under
  `serde_json` (`default-features = false`; its own `std` feature is enabled by
  `roxmltree/std`). `roxmltree` is not dependency-free: its manifest declares `memchr`.
- Unsafe: `roxmltree` is `#![forbid(unsafe_code)]` (`src/lib.rs:17`) and its sources do not
  mention the keyword elsewhere (on 2026-10-28); it has no `build.rs`, no proc macro and no
  native code. `memchr` is as recorded under `serde_json`. First-party code stays
  `forbid(unsafe_code)`.
- Resource bounds: `ParsingOptions::default()` refuses DTDs (`allow_dtd: false`), so no
  entity expansion happens; the node count limit is left at its default because the
  parsed files are the three retained fixtures.
- Banned crates: `cargo tree --workspace --all-features -e normal,dev,build -i <name>` for
  each of `once_cell`, `lazy_static`, `log`, `time` and `chrono`: the two existing paths
  (`once_cell` through `tracing-core`, `lazy_static` through `sharded-slab`, both wrapped in
  `deny.toml`) are unchanged, and the other three match no package. `roxmltree` and
  `memchr` add no path, so no `deny.toml` change is needed.
- Alternatives: a string scan of the XML (S04a does this for the `<raw>` elements of the
  parser suite, but the driver suites need element structure, attributes and nesting), and
  `quick-xml` or `xmltree` (larger or with more dependencies; `roxmltree` is the
  read-only tree reader the plan names).
- Review date: 2026-10-28. `cargo-deny` passes advisory, license, ban and source checks;
  this records the trust boundary, not a memory-safety certification.

## criterion 0.8.2 (stonecrab, dev-dependency)
- Purpose: the benchmark harness of slice S52 (`crates/stonecrab/benches/modbus_wire.rs` and
  `modbus_loopback.rs`, ADR-0009 addendum "S52"). The crate is a dev-dependency of `stonecrab`
  only, so it is not in the published manifest's dependencies and
  `cargo tree -e normal --all-features -p stonecrab` does not list it or any crate below
  (checked on 2026-10-28). `scripts/bench-compare.sh` reads the `estimates.json` files it writes.
- License: Apache-2.0 OR MIT.
- Features: `[workspace.dependencies]` has `criterion = { version = "0.8", default-features =
  false }` with no feature. The default features are `rayon` (parallel statistics), `plotters`
  (SVG plots) and `cargo_bench_support`, which is an empty marker in 0.8.2. Without
  `plotters` there are no HTML reports (the gate and the evidence need none); without `rayon`
  the analysis is single-threaded, which also keeps it off the cores a benchmark measures.
  The `async_*` features are off: the loopback benchmark drives its own current-thread Tokio
  runtime with `block_on`.
- Transitive crates this adds to `Cargo.lock` (all dev-only): `criterion-plot`, `anes`,
  `cast`, `ciborium` with `ciborium-io` and `ciborium-ll`, `half` with `zerocopy` and
  `crunchy`, `clap` with `clap_builder`, `clap_lex` and `anstyle`, `itertools` with
  `either`, `oorandom`, `page_size`, `regex` with `regex-automata`, `regex-syntax` and
  `aho-corasick`, `tinytemplate`, `walkdir` with `same-file` (and `winapi-util` on Windows),
  `alloca` with its build dependencies `cc`, `shlex` and `find-msvc-tools`, a second `syn`
  (2.0.119, through `zerocopy-derive`; the tree already had 3.0.9), and the Windows-only
  `winapi` family. Already audited and shared: `serde`, `serde_json`, `num-traits`, `memchr`, `libc`.
  Licenses are MIT or Apache-2.0 or both (`ciborium` is Apache-2.0 only; `alloca` MIT;
  `zerocopy` BSD-2-Clause OR Apache-2.0 OR MIT; `same-file` and `walkdir` Unlicense/MIT, of
  which MIT is taken; `unicode-ident` already carries Unicode-3.0). `cargo deny` accepts all
  of them under the existing allow list, so `deny.toml` is unchanged.
- Unsafe and native code: none of these crates is `forbid(unsafe_code)` except `clap`,
  `clap_builder` and `regex-syntax`; `aho-corasick`, `regex-automata`, `half` and `zerocopy`
  carry the most (the keyword appears on 227, 57, 84 and 554 lines of source and comments).
  They run only inside the bench binaries, never in the library or its tests. The one native
  step is `alloca`'s `build.rs`, which compiles a short C file with the `cc` crate (a C
  compiler is needed to build the benches; every CI runner and the development machines
  have one). `crunchy`, `zerocopy` and `num-traits` have build scripts that only emit cfg
  lines or generate Rust. First-party code stays `forbid(unsafe_code)`: the CI geiger grep
  finds the keyword only in the attribute of the two bench files.
- Banned crates: `cargo tree --workspace --all-features -e normal,dev,build -i <name>` for
  each of `once_cell`, `lazy_static`, `log`, `time` and `chrono`: the two existing paths
  (`once_cell` through `tracing-core`, `lazy_static` through `sharded-slab`, both wrapped in
  `deny.toml`) are unchanged and the other three match no package. Criterion adds no path to
  any of them, so no `deny.toml` change. `multiple-versions` is `warn`: `syn` 2.0.119 and
  3.0.9 now coexist, both as build-time proc-macro dependencies.
- Alternatives: `divan` (named beside criterion in CODING_STANDARDS 15.8) was not evaluated
  beyond its feature list, because criterion passes the policy: its defaults pull
  `clap` with `wrap_help` and it has no saved-baseline files with confidence intervals for
  the gate to read. A hand-written timing loop would replace the statistics (outlier
  handling, confidence intervals, saved baselines) that the gate depends on.
- Review date: 2026-10-28. `cargo-deny` 0.20.2 passes advisory, license, ban and source
  checks (`mise run dependencies`, exit 0); this records the trust boundary, not a
  memory-safety certification.

## serial2 0.2.39 (stonecrab, feature `serial`, which `modbus` enables since S59; dev-dependency with `unix`)
- Purpose: open a serial port by name, apply baud rate, character size, parity, stop bits
  and flow control, and give `Read` and `Write` with a read and a write timeout. The serial
  transport (`stonecrab::transport::SerialTransport`, slice S58, ADR-0014 addendum) runs it
  on a dedicated worker thread (CODING_STANDARDS 8.4). The standards name a serial library
  (sections 4.5 and 15.8). Its `SerialPort::pair`, behind the `unix` feature, makes the
  pseudo-terminal pair the integration tests use.
- License: BSD-2-Clause OR Apache-2.0. Both are in the `deny.toml` allow list, so
  `deny.toml` is unchanged. This is why it replaces `serialport` (MPL-2.0), which would have
  needed a license exception the maintainer has not granted.
- Features: `default-features = false` in `[workspace.dependencies]` (`serial2 = "0.2.39"`;
  the crate has no default features). Stonecrab enables it through `serial = ["transport",
  "dep:serial2"]`, and since S59 the `modbus` feature enables `serial`, because Modbus RTU
  needs the serial transport (ADR-0024 decision 13, ADR-0018 addendum S59); so the
  `default` and `full` features include `serial2`, and only a build without `modbus` and
  `serial` leaves it out. The dev-dependency entry adds the
  `unix` feature (it only exposes `SerialPort::pair` and the raw `termios` type), so test
  builds always compile the crate; it is not in the packaged manifest's dependencies. The
  `serde` feature (which would pull `serde`), `rs4xx` and `windows` stay off.
- Transitive crates in this configuration (`cargo tree -e normal --target all`): none new.
  `cfg-if` 1.0.5 and `libc` 0.2.191 (Unix; both MIT OR Apache-2.0) and `windows-sys` 0.61.2
  with `windows-link` 0.2.1 (Windows; MIT OR Apache-2.0) are the versions Tokio already
  resolves, so `Cargo.lock` gains only the `serial2` package. On Windows the crate asks
  `windows-sys` for the `Win32_Devices_Communication`, `Win32_System_Registry`,
  `Win32_System_IO`, `Win32_System_Threading`, `Win32_Storage_FileSystem`,
  `Win32_System_WindowsProgramming` and `Win32_Security` features, which enlarge the binding
  surface of a crate that is already linked there; no second `windows-sys` appears and
  `cargo deny` reports no new duplicate.
- Unsafe: `serial2` is not `forbid(unsafe_code)`; its sources contain the keyword 51 times
  (termios and ioctl calls, `poll`, `read`, `write` and `close` on Unix, overlapped
  `ReadFile` and `WriteFile` on Windows, and `from_raw_fd`) on 2026-10-28. That is the
  expected surface for a crate whose job is system calls; CODING_STANDARDS 4.5 puts it behind
  one audited crate. `cfg-if` has none; `libc` and `windows-sys` are bindings. First-party
  code stays `forbid(unsafe_code)` and does not mention the keyword.
- Native code: none. No build script, no proc macro.
- Behavior to know: on Unix the port is opened non-blocking (`O_NONBLOCK`, `O_NOCTTY`) and
  each read or write first `poll`s for readiness for at most the port's timeout (3 seconds
  unless set; Stonecrab sets 20 ms), so a call never blocks past its timeout, which is what
  bounds a write into a stalled line. On Windows the timeouts are the `COMMTIMEOUTS` of the
  handle, and a timeout is reported as a transfer of zero bytes, which the crate turns into
  `TimedOut`. It takes no exclusive lock: a second open of the same device succeeds. On
  macOS every settings call ends in the `IOSSIOSPEED` ioctl, which a pseudo-terminal
  refuses with `ENOTTY`, so `SerialPort::pair` and opening a pseudo-terminal by name fail
  there; the pair tests and the by-name tests are skipped on macOS and Windows (ADR-0014
  addendum S58). It has no mark or space parity and no 1.5 stop bits.
- Banned crates: `cargo tree --locked -p stonecrab --all-features -e normal,dev,build -i
  <name>` for each of `once_cell`, `lazy_static`, `log`, `time` and `chrono`: the two
  existing paths (`once_cell` through `tracing-core`, `lazy_static` through `sharded-slab`,
  both wrapped in `deny.toml`) are unchanged and the other three match no package. Neither
  `serial2` nor anything it uses is on those paths, so the wrappers need no change.
- Alternatives: `serialport` (the first attempt at S58; MPL-2.0, which `deny.toml` does not
  allow, and with a larger tree: `nix`, a second `bitflags`, `core-foundation`, `io-kit-sys`,
  `mach2` and a second `windows-sys`); `mio-serial` and `tokio-serial` (they wrap
  `serialport` and put its descriptor behind a reactor; section 8.4 asks for a dedicated
  thread instead); `serial2-tokio` (the same, over `serial2`); writing termios and Win32
  serial code (needs `unsafe`, which first-party code forbids).
- Maintenance: 0.2.39 is the current release as resolved on the review date, and
  `cargo-deny` reports no open RUSTSEC advisory against it.
- Review date: 2026-10-28. `cargo-deny` passes advisory, license, ban and source checks with
  `deny.toml` unchanged; this records the trust boundary, not a memory-safety certification.

## sha2 0.11.1 (tools/stonecrab-codegen workspace only)
- Purpose: the SHA-256 of each vendored mspec, which the generator compares with
  `upstream/manifest.toml` before it generates anything and writes into every provenance
  header and `generated/MANIFEST.toml` (ADR-0019 decisions 2, 6 and 7, slice S66). A
  first-party SHA-256 would be cryptographic code the project would have to own and test;
  the standard library has none.
- License: MIT OR Apache-2.0, as are all its dependencies below, so `deny.toml` and the
  workspace's copy of it (`tools/stonecrab-codegen/deny.toml`) are unchanged.
- Declared in `tools/stonecrab-codegen/Cargo.toml` only, with `default-features = false`
  (`sha2 = { version = "0.11", default-features = false }`): the defaults `alloc` and `oid`
  (OID constants for ASN.1) are not needed, and `zeroize` stays off. The generator workspace
  is excluded from the root workspace (ADR-0019 decision 2, the layout of ADR-0009), so
  nothing here reaches the root `Cargo.lock`, the published crate or its dev-dependencies.
- Transitive crates in this configuration (`cargo tree --locked -e normal,build` in the tool
  workspace): `digest` 0.11.3, `block-buffer` 0.12.1, `crypto-common` 0.2.2, `hybrid-array`
  0.4.15, `typenum` 1.20.1, `cpufeatures` 0.3.1 with `libc` 0.2.191 (CPU feature detection),
  and `cfg-if` 1.0.5. No proc macro. `libc` has a build script that only sets `cfg` flags; no
  crate compiles native code. 0.11 was chosen over 0.10.9 because it drops `generic-array`
  and its `version_check` build dependency for `hybrid-array`.
- Unsafe: `sha2` 0.11.1 has the keyword on 39 lines of its sources on 2026-10-29, in
  its architecture back ends (`x86_sha`, `x86_avx2`, `aarch64_sha2`, `riscv_zknh`,
  `loongarch64_asm`, `wasm32_simd128`: SIMD intrinsics and inline assembly behind runtime
  feature detection); `cpufeatures` reads CPUID or the OS feature flags; `block-buffer`,
  `hybrid-array`, `digest`, `crypto-common` and `typenum` use it for fixed-size arrays. That
  is the expected surface of a hash crate; ADR-0019 decision 2 chose it. The tool's own code is
  `forbid(unsafe_code)`, and nothing of this runs in a library build or a test of the crate:
  only `mise run generate` and the `regen` gate run the tool.
- The other dependency of the tool is `toml` 1.1.8, the version and features audited above
  for the crate (`parse`, `serde`, `std`), with the same single `winnow` 1.0.4 in the tool's
  `Cargo.lock`, so `clippy::multiple_crate_versions` stays quiet there too.
- Banned crates: the D23 check in the tool workspace (`cargo tree --locked -e
  normal,dev,build -i <name>` for each of `once_cell`, `lazy_static`, `log`, `time` and
  `chrono`) matches no package, so no wrapper is needed. The root policy's two wrappers
  (`tracing-core`, `sharded-slab`) are reported as unused there, a warning.
- Maintenance: 0.11.1 is the newest release resolved on the review date, and `cargo deny
  --frozen check` in the tool workspace reports no RUSTSEC advisory, yanked crate, banned
  crate or unknown source (advisories, bans, licenses and sources ok).
- Alternatives: `ring` or `aws-lc-rs` (native code and a C toolchain for one hash), a
  first-party SHA-256 (rejected above), shelling out to `shasum` or `sha256sum` (differs per
  platform; the gate still uses whichever exists for its own step 3).
- Review date: 2026-10-29. This records the trust boundary, not a memory-safety
  certification.

## rustls 0.23.46 (stonecrab, feature `tls`)
- Purpose: the TLS 1.3 and 1.2 client of the TLS transport (`stonecrab::transport::TlsFactory`,
  slice S71, ADR-0032; decision E11 of 2026-10-29 chose rustls with the ring provider). It
  verifies the server's certificate against the caller's trust anchors and the host name
  (through `rustls-webpki`), presents the client certificate for mutual TLS, and parses the
  PEM files (through `rustls-pki-types`). The standard library has no TLS.
- License: Apache-2.0 OR ISC OR MIT. All in the `deny.toml` allow list.
- Features: `default-features = false` in `[workspace.dependencies]` (`rustls = "0.23"`),
  with `ring` (the provider), `std` (file-backed PEM parsing, the session cache) and `tls12`.
  The defaults `aws_lc_rs`, `prefer-post-quantum` and `logging` stay off: `logging` would
  bring `log`, which `deny.toml` bans, and aws-lc-rs is the provider E11 did not choose.
  Enabled only by the `tls` feature (`tls = ["transport", "dep:rustls", "dep:tokio-rustls"]`),
  which no other feature turns on, so the default build has none of the crates of this and
  the next entries.
- Unsafe: `rustls` is `#![forbid(unsafe_code)]`. The keyword's remaining occurrences in its
  sources (5 on 2026-10-29) are that attribute and documentation.
- Banned crates: it depends on `once_cell` unconditionally (its `std` feature only adds
  `once_cell/std`), so `deny.toml` names it as a second wrapper of that ban (entry above).
  The D23 check (`cargo tree --locked -p stonecrab --all-features -e normal,dev,build -i
  <name>`) finds `once_cell` through `tracing-core` and `rustls` and `lazy_static` through
  `sharded-slab`; `log`, `time` and `chrono` match no package.
- Transitive crates (`cargo tree -e normal --target all --features tls`): `ring`,
  `rustls-webpki`, `rustls-pki-types`, `subtle`, `zeroize` and `once_cell` (entries below and
  above); `Cargo.lock` gains `getrandom` 0.2.17, `ring` 0.17.14, `rustls` 0.23.46,
  `rustls-pki-types` 1.15.1, `rustls-webpki` 0.103.16, `subtle` 2.6.1, `tokio-rustls` 0.26.6,
  `untrusted` 0.9.0, `zeroize` 1.9.1 and, for Windows targets only, `windows-sys` 0.52.0 with
  `windows-targets` 0.52.6 and its eight `windows_*` 0.52.6 import-library crates.
- Behavior to know: the client configuration keeps the rustls defaults that matter here:
  no key log (`NoKeyLog`), SNI for DNS names, an in-memory session cache of 256 entries
  (a reconnect may resume), no ALPN, and revocation is not checked.
- Alternatives: `native-tls` and `openssl` (banned in `deny.toml`; platform-dependent
  behavior), `aws-lc-rs` as the provider (CMake and NASM in the build; E11 chose ring),
  `boring` (BoringSSL bindings, the same objections).
- Maintenance: 0.23.46 is the newest 0.23 release on the review date (0.24 is a pre-release);
  `cargo deny` reports no RUSTSEC advisory.
- Review date: 2026-10-29. `cargo-deny` passes advisory, license, ban and source checks with
  the `rustls` wrapper; this records the trust boundary, not a memory-safety certification.

## tokio-rustls 0.26.6 (stonecrab, feature `tls`)
- Purpose: drives a rustls client session over a Tokio `TcpStream` (`TlsConnector`,
  `client::TlsStream` with `AsyncRead` and `AsyncWrite`), which the TLS transport wraps in its
  deadlines. The tests also use its server side (`TlsAcceptor`) as the loopback peer.
- License: MIT OR Apache-2.0.
- Features: `default-features = false` (`tokio-rustls = "0.26"`) with `ring` and `tls12`; the
  defaults `logging` and `aws_lc_rs` stay off for the reasons of the rustls entry.
- Unsafe: none in its sources on 2026-10-29 (the keyword does not occur), although the crate
  does not declare `forbid(unsafe_code)`. No build script.
- Alternatives: driving `rustls::ClientConnection` by hand over the socket (the read and
  write state machine tokio-rustls already is), `async-tls` (unmaintained).
- Review date: 2026-10-29.

## ring 0.17.14 (via rustls with feature `tls`; dev-dependency of stonecrab)
- Purpose: the cryptography of the rustls provider: AEAD ciphers, X25519 and ECDH key
  exchange, ECDSA, Ed25519 and RSA signature verification and signing, HKDF and HMAC, and
  the system random generator. As a dev-dependency (`ring = { workspace = true }` in
  `[dev-dependencies]`, `default-features = false` in `[workspace.dependencies]`) it makes
  the Ed25519 keys and signatures of the test certificate authorities
  (`crates/stonecrab/tests/support/tls_pki.rs`); the same version as the normal graph, so the
  lock gains nothing for it.
- License: Apache-2.0 AND ISC (its own code under ISC-style terms, parts derived from
  BoringSSL under Apache-2.0 and ISC). Both are in the allow list.
- Features: through `rustls/ring` the defaults `alloc` and `dev_urandom_fallback`; the
  dev-dependency adds none.
- Native code: its build script compiles C and assembly (from BoringSSL) with `cc` 1.10.0,
  which was already in `Cargo.lock` (with `shlex` and `find-msvc-tools`), so a build with
  `tls` needs a C compiler, and the targets ring supports are the targets the feature
  supports. The crate is not `forbid(unsafe_code)`: about 230 lines carry the keyword on
  2026-10-29 (the FFI to the C and assembly primitives, CPU feature detection, constant-time
  helpers). That is the expected surface of a cryptographic library, and the reason E11 put
  it behind an opt-in feature.
- Transitive crates: `cfg-if` and `libc` (already present), `untrusted` 0.9.0 (ISC; a
  panic-free DER and byte reader, no `unsafe`), `getrandom` 0.2.17 (MIT OR Apache-2.0; 58
  lines with the keyword: the operating-system random calls) and, on Windows,
  `windows-sys` 0.52.0 (bindings).
- Duplicates (CODING_STANDARDS 10.7): `getrandom` 0.2.17 beside the 0.4.3 that `proptest`
  brings to dev builds, and, on Windows, `windows-sys` 0.52.0 beside the 0.61.2 of Tokio and
  `serial2`. `ring` 0.17 pins both older majors and has no release on the newer ones; they
  are justified here and stay until a ring release moves. `cargo deny` reports the
  `windows-sys` pair as a warning (`multiple-versions = "warn"`), and Clippy's
  `multiple_crate_versions` stays quiet under `-D warnings`.
- Maintenance: 0.17.14 is past RUSTSEC-2025-0009 (fixed in 0.17.12); `cargo deny` reports no
  advisory on the review date.
- Review date: 2026-10-29.

## rustls-webpki 0.103.16 (via rustls with feature `tls`)
- Purpose: X.509 path building and verification for rustls: chain to a trust anchor,
  validity dates, basic constraints, extended key usage and the subject alternative names
  the server name must match.
- License: ISC.
- Features: `alloc`, `ring` and `std`, selected by rustls.
- Unsafe: none in its sources on 2026-10-29. No build script.
- Review date: 2026-10-29.

## rustls-pki-types 1.15.1 (via rustls with feature `tls`)
- Purpose: the shared certificate, private-key and server-name types of the rustls crates,
  and the PEM parser (`PemObject`) the TLS transport reads the trust store and the keystore
  with, which replaces the deprecated `rustls-pemfile`.
- License: MIT OR Apache-2.0.
- Features: `alloc`, `default` and `std` (file and slice PEM parsing).
- Unsafe: one line with the keyword on 2026-10-29. Its private-key types implement
  `zeroize::Zeroize` for an owner that calls it; they are not cleared on drop by themselves,
  which is why ADR-0032 claims no zeroization beyond what rustls does.
- Review date: 2026-10-29.

## subtle 2.6.1 and zeroize 1.9.1 (via rustls with feature `tls`)
- Purpose: `subtle` gives rustls constant-time comparisons (BSD-3-Clause; 2 lines with the
  keyword); `zeroize` is how rustls clears its own key material, and the `Zeroize` impls of
  `rustls-pki-types` (Apache-2.0 OR MIT; 20 lines with the keyword, volatile writes and
  fences, which is its purpose).
- Features: `zeroize` with `alloc`; `subtle` with none. No build script in either.
- Review date: 2026-10-29.

## Drafts for S101 (OPC UA cryptography, ADR-0035): not adopted

The four entries below were written by slice S82 from the probe on the unmerged branch
`spike/s82-crypto-probe` (ADR-0035, status Proposed). None of these crates is in `Cargo.toml`
or `Cargo.lock` of this repository for the `opcua` feature: that happens in slice S101, which
turns each draft into a dated entry (or corrects it) in the same change that adds the
dependency. Until then they are drafts, review dates are the probe's, and nothing here
changes the entries above.

### aws-lc-rs 1.19.0 (DRAFT for S101: stonecrab, feature `opcua`)
- Purpose: the cryptography of the OPC UA security policies (decision E15, ADR-0035): RSA-OAEP
  with SHA-1 and SHA-256, RSA PKCS #1 v1.5 and PSS signatures with SHA-256, AES-CBC with a
  caller IV and no padding, HMAC-SHA1 and HMAC-SHA256, SHA-1 and SHA-256 digests, and the RSA
  signature verification `rustls-webpki` needs for certificate chains. `ring` 0.17 has no RSA
  encryption and no AES-CBC, and the standard library has none of this.
- License: ISC AND (Apache-2.0 OR ISC). Both in the `deny.toml` allow list.
- Features: `default-features = false` with `aws-lc-sys` and `alloc`. The defaults `ring-io` and
  `ring-sig-verify` (which pull `untrusted`), `fips`, `bindgen`, `asan`, `prebuilt-nasm` and
  `legacy-des` stay off.
- Unsafe: not `forbid(unsafe_code)`. 381 lines with the keyword in `src` on 2026-10-30 (FFI
  calls into AWS-LC, `Send` and `Sync` implementations for `EVP_PKEY` holders). First-party
  code keeps `forbid(unsafe_code)`; the probe forbids it too.
- Build script: `build.rs` only declares `cfg` checks and two test switches
  (`AWS_LC_RS_DISABLE_SLOW_TESTS`, `AWS_LC_RS_DEV_TESTS_ONLY`); it compiles no native code. The
  native build is `aws-lc-sys`'s. `links = "aws_lc_rs_1_19_0_sys"`.
- Transitive crates: `aws-lc-sys` (next entry) and `zeroize` 1.9.1, which `rustls` already
  brings under `tls`.
- API facts that shape the design (all from the 1.19.0 sources, tested in the probe): RSA
  signing encodings are PKCS #1 SHA-256, SHA-384, SHA-512 and PSS with the same hashes; SHA-1
  exists only for verification (`..._SHA1_FOR_LEGACY_USE_ONLY`), and `RsaEncoding` is sealed, so
  SHA1withRSA cannot be signed (E16); `PrivateDecryptingKey` loads PKCS #8 only
  (`from_pkcs8`), while `RsaKeyPair::from_der` reads PKCS #1 and can re-export PKCS #8; AES-CBC
  without padding is `EncryptingKey::cbc` with `less_safe_encrypt` and a caller
  `EncryptionContext::Iv128`.
- Alternatives: RustCrypto `rsa` with `aes`, `cbc`, `hmac`, `sha1` and `sha2` (pure Rust, but
  RUSTSEC-2023-0071 has no patched version and would need a `deny.toml` ignore); the `openssl`
  crate (banned, ADR-0032); policy None only.
- Maintenance: 1.19.0 was published 2026-10-28; 1.18.2 (2026-10-12, with `aws-lc-sys` 0.46.0)
  is the recorded fallback. `cargo deny` with the root `deny.toml` reports no advisory for the
  probe's graph on 2026-10-30.
- Review date: 2026-10-30 (draft).

### aws-lc-sys 0.47.0 (DRAFT for S101: stonecrab, feature `opcua`, native code)
- Purpose: the FFI bindings and the build of AWS-LC, the C cryptographic library under
  `aws-lc-rs`. A direct dependency is declared only to set the feature `disable-prebuilt-nasm`.
- License: `ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND
  (Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)`. Every term can be satisfied
  from the allow list; the `OpenSSL` licence does not appear.
- Features: `default-features = false` and `disable-prebuilt-nasm`, so the Windows x86_64
  build assembles with NASM and never with prebuilt objects. `prebuilt-nasm` (which
  `rustls-webpki`'s own `aws-lc-rs` feature would switch on), `bindgen`, `fips`, `ssl`, `asan`
  and `unstable` stay off, and so does its default `all-bindings` (`default-features = false`).
- Native code: the crate package is 9.7 MB (53 MB of AWS-LC C source unpacked), `links =
  "aws_lc_0_47_0"`. Its build script (`builder/main.rs`) compiles AWS-LC with the `cc` crate
  and falls back to CMake when the `cc` builder's dependency check fails. A build needs a C
  compiler; Windows x86_64 also needs NASM (the probe used NASM 2.16.03). The script adopts a
  system AWS-LC found through `OPENSSL_DIR`, `OPENSSL_INCLUDE_DIR`, `OPENSSL_LIB_DIR` or
  pkg-config unless `AWS_LC_SYS_USE_SYSTEM=0`; CI sets that for every job, and downstream users
  need it documented (it never adopts plain OpenSSL, `builder/system_detect.rs`). The clean
  release build of the library took 29 s on a CI runner and the static archive is 7.3 MB
  (`libaws_lc_0_47_0_crypto.a`); the probe's release example is 3.2 MB (2.9 MB stripped).
- Unsafe: not `forbid(unsafe_code)`; 11,254 lines with the keyword in `src` on 2026-10-30,
  mostly generated `unsafe extern` declarations, plus 31 in `builder`. That is the surface of
  a C-library binding and the reason the dependency sits behind an opt-in feature.
- Build dependencies: `cc` 1.10.0 (already in `Cargo.lock` for `ring`; with `shlex`,
  `find-msvc-tools`, `jobserver` and `libc`, and on non-host targets the `jobserver` crates
  `cfg-if` 1.0.5, `getrandom` 0.4.3 and `r-efi` 6.0.0, whose licence has an
  `LGPL-2.1-or-later` alternative that `deny.toml` does not need), `cmake` 0.1.58, `dunce` 1.0.5, `fs_extra`
  1.3.0 and, on Unix, `pkg-config` 0.3.34, all MIT or Apache-2.0 (`dunce`: CC0-1.0 OR MIT-0
  OR Apache-2.0).
- Banned crates: the D23 check (`cargo tree --locked --all-features --target all -e
  normal,dev,build -i <name>`) matches no package for `once_cell`, `lazy_static`, `log`, `time`
  and `chrono` in the probe's graph, with no new `deny.toml` wrapper. S101 repeats it on
  `-p stonecrab --all-features` and pastes the output.
- Advisories: RUSTSEC-2026-0044 to 0048 exist for `aws-lc-sys` (checked in the cargo-deny
  advisory database on 2026-10-30): 0045 (AES-CCM tag timing), 0046 and 0047 (`PKCS7_verify`
  bypasses) are patched from 0.38.0, 0044 (name constraints) and 0048 (CRL scope) from 0.39.0.
  0.47.0 is past every patched version, and `cargo deny` reports none for it.
- Alternatives: as for `aws-lc-rs`; the `aws-lc-fips-sys` FIPS build (needs Go and CMake,
  out of scope).
- Review date: 2026-10-30 (draft).

### rustls-webpki 0.103.16 (DRAFT for S101: addendum for feature `opcua`; entry above stands)
- Purpose under `opcua`: validating the server's application certificate against the operator's
  trust list (S104), and verifying a signature with a certificate's public key
  (`EndEntityCert::verify_signature`), with no TLS involved.
- Features: `alloc` and, when `tls` is also on, rustls's `ring` and `std`. No provider feature
  of its own under `opcua`: `rustls-webpki`'s `aws-lc-rs` feature depends on
  `aws-lc-rs/prebuilt-nasm`, which the Windows build must not turn on, so the signature
  algorithms are first-party: one `SignatureVerificationAlgorithm` implementation for RSA
  PKCS #1 SHA-256 over `aws_lc_rs::signature::UnparsedPublicKey` (about 25 lines in the
  probe), more as S104 needs them. Without any algorithm in the list webpki validates nothing.
- Details found by the probe: the library is named `webpki` (not `rustls_webpki`);
  `AlgorithmIdentifier::from_slice` takes the contents of the DER SEQUENCE, not the header; a
  self-signed certificate without the CA flag validates against itself as its own trust anchor
  in 0.103.16 although the documentation of `anchor_from_trusted_cert` says self-signed
  certificates are unsupported, so S104 keeps the probe's test; `cargo machete` needs
  `rustls-webpki` in `ignored` where the package is imported as `webpki`.
- Maintenance: 0.103.16 was published 2026-10-20. RUSTSEC-2026-0382 (distrusted intermediates
  may be accepted; dated 2026-10-20, checked in the cargo-deny advisory database on 2026-10-30)
  is patched from 0.103.16, so this is the first release without it and the pin must not go
  lower.
- Review date: 2026-10-30 (draft).

### rustls-pki-types 1.15.1 (DRAFT for S101: addendum for feature `opcua`; entry above stands)
- Purpose under `opcua`: the `CertificateDer`, `TrustAnchor`, `UnixTime` and `AlgorithmIdentifier`
  types webpki takes, `SignatureVerificationAlgorithm` (the trait the first-party algorithm
  implements), and the PEM parser that classifies a key store by label (`PRIVATE KEY` to
  PKCS #8, `RSA PRIVATE KEY` to PKCS #1; `ENCRYPTED PRIVATE KEY` is not accepted), replacing the
  deprecated `rustls-pemfile`.
- Features: `alloc` is enough for the in-memory PEM parsing the probe tests; `std` adds the
  file-reading helpers that `tls` already enables.
- Unsafe: one line with the keyword on 2026-10-30; the `Zeroize` implementations of its key
  types clear them only when the owner calls `zeroize` (see the `tls` entry).
- Review date: 2026-10-30 (draft).
