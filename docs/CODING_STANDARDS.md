# Stonecrab Coding Standards

**Version:** 1.0 draft | **Applies to:** the `stonecrab` crate and everything else in the repository | **Owner:** maintainers | **Changes:** by pull request, with an ADR when a rule in §2, §4, §5, §8 or §15 changes.

This document is normative. **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT** and **MAY** are used as in RFC 2119. A rule marked **CI** is enforced mechanically; a rule marked **review** is enforced by code review. Where this document and the [engineering specification](./SPEC.md) disagree, the specification wins and this document gets fixed.

The standards exist for one reason: this library parses bytes from industrial networks and drives hardware on edge devices that nobody is watching. A panic, an unbounded allocation, a silently dropped sample or a replayed write is a production incident, not a bug report. Every rule below traces back to that.

---

## Contents

1. [Scope and precedence](#1-scope-and-precedence)
2. [Toolchain](#2-toolchain)
3. [Formatting](#3-formatting)
4. [`unsafe` policy](#4-unsafe-policy)
5. [Lints](#5-lints)
6. [Panics, errors, and failure](#6-panics-errors-and-failure)
7. [Numbers, bytes, and parsing](#7-numbers-bytes-and-parsing)
8. [Async and concurrency](#8-async-and-concurrency)
9. [API design](#9-api-design)
10. [Crate and module layout](#10-crate-and-module-layout)
11. [Naming](#11-naming)
12. [Documentation](#12-documentation)
13. [Testing](#13-testing)
14. [Logging and diagnostics](#14-logging-and-diagnostics)
15. [Dependencies and supply chain](#15-dependencies-and-supply-chain)
16. [Performance](#16-performance)
17. [Generated code](#17-generated-code)
18. [Version control and pull requests](#18-version-control-and-pull-requests)
19. [CI gates](#19-ci-gates)
20. [Releases and versioning](#20-releases-and-versioning)
21. [Security](#21-security)
- [Appendix A — Configuration files](#appendix-a--configuration-files)
- [Appendix B — Pull request template](#appendix-b--pull-request-template)
- [Appendix C — Reviewer checklist](#appendix-c--reviewer-checklist)
- [Appendix D — ADR template](#appendix-d--adr-template)

---

## 1. Scope and precedence

1.1. These standards apply to all first-party Rust in the workspace: the published crate under `crates/stonecrab/`, tooling under `tools/` when it exists, examples, benchmarks, fuzz targets and tests. Where a rule is relaxed for a category (tests may `unwrap`, binaries may use `anyhow`), the relaxation is stated explicitly; otherwise the rule applies everywhere.

1.2. Generated code (§17) is exempt from style and lint rules, is excluded from coverage thresholds, and is never edited by hand. It is **not** exempt from §4 (`unsafe`), §6 (panics) or §7 (bounded parsing): the generator must emit code that satisfies them.

1.3. Vendored or adapted upstream material (protocol definitions, fixtures) keeps its original form and license header. It is data, not first-party code.

1.4. Precedence when documents conflict: `SPEC.md` → this document → crate-level `README.md` → inline comments. An ADR under `docs/adr/` may override a rule here for a named crate or module and must say so in the rule's margin (see §1.5).

1.5. Exceptions to any **MUST** are recorded as an `#[expect(lint, reason = "ADR-NNNN: …")]` at the narrowest scope that works, plus a line in the ADR. There are no undocumented exceptions.

---

## 2. Toolchain

2.1. **Edition.** Every crate uses `edition = "2024"` via `workspace.package`. No crate pins an older edition. — **CI**

2.2. **Development toolchain.** `rust-toolchain.toml` at the workspace root pins the **current stable release by exact version** (today: `1.99.1`, released 15 Oct 2026). It is bumped by a pull request within **7 days** of each stable release. That PR runs the full CI matrix and fixes any new lint findings before merge; it does not suppress them. — **CI**

2.3. **MSRV.** The minimum supported Rust version is **stable − 2**, declared once as `workspace.package.rust-version` and inherited by every published crate (today: `1.97`). The toolchain-bump PR in §2.2 raises the MSRV in the same change. Dropping support for a Rust version is **not** a semver-breaking change for this project and is noted in the changelog. — **CI**

2.4. **CI matrix.** Every pull request builds and tests on:

| Job | Toolchain | Purpose |
| --- | --- | --- |
| `msrv` | `rust-version` from `Cargo.toml` | `cargo check --workspace --all-features` plus the test suite of every published crate |
| `stable` | `rust-toolchain.toml` | The full gate: fmt, clippy, tests, docs, coverage |
| `beta` | `beta` | `cargo check` + `cargo clippy`; failures are reported, not blocking, and must be fixed before the next toolchain bump |
| `nightly` | pinned nightly date | Fuzzing and Miri **only**. No first-party code may require nightly |

2.5. **Resolver.** The workspace sets `resolver = "3"` so Cargo's dependency resolution respects `rust-version` and the MSRV job cannot be broken by a transitive dependency that raised its own MSRV. — **CI**

2.6. **Components.** `rust-toolchain.toml` lists `rustfmt`, `clippy` and `llvm-tools-preview` (for `cargo-llvm-cov`). Cross targets are added as `targets` entries: `aarch64-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`, and later `x86_64-pc-windows-msvc` and `aarch64-apple-darwin`.

2.7. **Nightly features.** `#![feature(...)]` is prohibited in first-party code with exactly one exception: `#![cfg_attr(docsrs, feature(doc_cfg))]` so docs.rs labels feature-gated items. The `docsrs` cfg is only ever set by docs.rs or by a maintainer running `RUSTDOCFLAGS="--cfg docsrs" cargo +nightly doc`. — **CI**

2.8. **Edition 2024 specifics worth knowing.**

- `-> impl Trait` captures all in-scope lifetimes. Use precise capturing (`-> impl Iterator<Item = &T> + use<'_, T>`) when a return type must not capture a lifetime it does not need.
- `gen` is a reserved keyword. Nothing is named `gen`; the generator (a maintainer tool, `tools/stonecrab-codegen`) has the entry point `generate`.
- `std::env::set_var` is `unsafe` and therefore unavailable (§4). Tests needing environment configuration use constructor arguments or a `Config` builder instead.
- Temporaries in `if let` scrutinees and tail expressions drop earlier than in 2021. Bind lock guards and borrowed buffers to a named `let` and never rely on temporary lifetime extension.
- `let` chains (`if let Some(x) = a && x > 3`) are stable and preferred over nested `if let`.
- Trait upcasting is stable: a trait with an `Any` supertrait can be upcast with `as &dyn Any`. Do not write `fn as_any(&self) -> &dyn Any` shims.

---

## 3. Formatting

3.1. **Standard style, no customization.** Code is formatted by `rustfmt` with the default style for `style_edition = "2024"`. `rustfmt.toml` contains only that line (Appendix A). We do not tune widths, tabs, braces or import granularity; "what rustfmt produces" is the style. — **CI**

3.2. `cargo fmt --all --check` passes on every PR. Formatting-only changes go in their own commit (`style:` type, §18) so blame stays useful.

3.3. **Imports.** `rustfmt` on stable does not reorder import groups, so the following is a **review** rule: one `use` block per file, in three groups separated by a blank line — `std`/`core`/`alloc`, then external crates, then `crate`/`super`/`self`. Alphabetical within a group; `rustfmt` sorts within a group. No glob imports outside tests and the crate's own `prelude` module (if any). No `use` inside functions except to scope a test helper.

3.4. **Line endings and encoding.** LF only, UTF-8 only, no BOM, a trailing newline on every text file. `.gitattributes` enforces `* text=auto eol=lf`. Non-ASCII identifiers are denied (`non_ascii_idents`); non-ASCII is fine in strings, comments and docs (so "I²C" and "t3.5" are fine).

3.5. **Comments.** `//` for prose, `///` for item docs, `//!` for module docs. Comments say *why*, not *what*; a comment restating the next line is deleted in review. `TODO`/`FIXME` are permitted only with an issue link: `// TODO(#123): …`. `todo!()` and `unimplemented!()` in code are denied (§6).

3.6. **Attribute order.** `#[derive]` first, then `#[non_exhaustive]`, then `#[must_use]`, then `#[cfg]`/`#[cfg_attr]`, then lint attributes, then everything else. Doc comments precede all attributes.

3.7. **Derive order.** `Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default` in that order, followed by third-party derives (`serde::Serialize`, `thiserror::Error`). Derive the minimum that is meaningful: a type holding a `Box<dyn Error>` cannot derive `Clone` and should not pretend to.

---

## 4. `unsafe` policy

4.1. **Forbidden, everywhere, without exception.** Every crate's root module begins with `#![forbid(unsafe_code)]`, and `[workspace.lints.rust] unsafe_code = "forbid"` applies it again from the manifest. `forbid` cannot be overridden by `#[allow]`, so a stray `unsafe` block is a compile error, not a warning. — **CI**

4.2. **What this rules out, deliberately.**

- No FFI. There is no C in this workspace and no `extern "C"` block. A future stable C ABI is a *separate repository* with its own standards.
- No `#[no_mangle]`, no `#[link_section]`, no inline assembly, no `transmute`, no raw-pointer dereference, no `get_unchecked`, no `from_utf8_unchecked`, no `MaybeUninit` tricks, no hand-rolled `Send`/`Sync`.
- No `build.rs` that compiles or links native code. Build scripts are permitted only for embedding version metadata and must themselves be `#![forbid(unsafe_code)]`.
- No `static mut`. Use `std::sync::OnceLock`, `LazyLock`, atomics, or pass state explicitly.
- No `std::env::set_var` / `remove_var` (unsafe in edition 2024).

4.3. **Performance is not an exception.** If a benchmark shows a bounds check or a copy matters, the fix is a better algorithm, `bytes::Bytes` for zero-copy slicing, iterator chains the optimizer can elide, or `#[inline]` with a benchmark attached — never `unsafe`. If none of those suffice, the work goes to an ADR that proposes an *external, audited* crate that encapsulates the `unsafe`, and the ADR must show the measured gain.

4.4. **Dependencies.** Third-party crates may contain `unsafe`; the standard library does. The rule is that first-party code never does, and that we know what we are trusting:

- `cargo geiger --workspace --all-features` runs in CI and **fails if any first-party crate reports a non-zero `unsafe` count**. — **CI**
- Every direct dependency that uses `unsafe` (or ships C via a `-sys` crate) is listed in `docs/DEPENDENCY_AUDIT.md` with: the function it performs, why no `forbid(unsafe_code)` alternative was chosen, and the date of the last review. — **review**
- Adding a dependency that is not `#![forbid(unsafe_code)]` requires that entry in the same PR.
- Prefer, in order: `std` → a crate that forbids `unsafe` → a widely-used crate whose `unsafe` is well-audited (`tokio`, `bytes`, `rustls`) → anything else, with justification.

4.5. **Hardware access** goes through `std` file I/O and `ioctl`-wrapping crates (`spidev`, `i2cdev`, `serialport`), which encapsulate the `unsafe` syscalls. Our adapters wrap those behind the `SpiBus`/`I2cBus`/`SerialPort` traits in §8.7 so the `unsafe` surface is one audited crate per bus, not scattered.

4.6. **Miri.** Because there is no first-party `unsafe`, Miri is not a release gate. It runs on the nightly job against the test suites of the core and `stonecrab::codec` as a cheap detector of library misuse (uninitialized reads via a dependency, data races in tests), with findings filed as issues.

---

## 5. Lints

5.1. **Single source of truth.** All lint levels live in `[workspace.lints]` in the root `Cargo.toml` (Appendix A). The crate opts in with `[lints] workspace = true` (Cargo does not allow a crate to both inherit and override that table). Lint attributes in `lib.rs` are prohibited, with exactly two exceptions: the `#![forbid(unsafe_code)]` of §4.1, repeated in source so it survives the crate being built outside the workspace; and the **promotions** below, which raise a lint the workspace sets to `warn` up to `deny` where that code is never acceptable (source attributes take precedence over the `-W` flags Cargo passes, so this works). Stonecrab is one crate, so a promotion that holds for the whole crate is an inner attribute of `lib.rs`, and a promotion that holds for one module is an **outer attribute on that module's declaration** in `lib.rs` (`#[deny(...)] pub mod codec;`). Module root files (`src/codec.rs` and the like) carry no lint-level attribute of their own.

| Promotion | Where |
| --- | --- |
| `#![deny(missing_docs)]` | the crate (inner attribute in `lib.rs`) |
| `#![deny(clippy::as_conversions)]` | the crate (inner attribute in `lib.rs`), which covers the core and every module |
| `#[deny(clippy::indexing_slicing)]` | the module declarations of `codec`, `transport`, and every driver module (`sim`, `modbus`) |
| `#[deny(clippy::arithmetic_side_effects)]` | the module declarations of `codec` and every driver module |
| `#[deny(clippy::as_conversions)]` | also stated on the module declarations of `codec`, `transport` and every driver module |

The modules `sensors` and `testkit` carry no module-level promotion. A promotion may only *raise* a level; lowering one in source is prohibited. A new module that parses wire bytes or does arithmetic on them takes the promotions of `codec` in the pull request that adds it. **CI** (`scripts/lint-attrs.sh`, the `lint-attrs` job, checks `lib.rs` against its table of permitted crate-level and per-module promotions, and fails on any other lint attribute in `lib.rs` or in a module root file)

5.2. **Levels and promotion.** Locally, lints surface as **warnings** so a developer can iterate. In CI, `cargo clippy --workspace --all-targets --all-features -- -D warnings` promotes every warning to an error. A small set of lints is `deny` even locally because the code they flag is never acceptable to run (§6). — **CI**

5.3. **Groups.**

| Group | Level | Notes |
| --- | --- | --- |
| `rust::rust_2018_idioms`, `rust::rust_2024_compatibility` | warn | priority −1 |
| `clippy::all` | warn | priority −1; this is the default set and includes `correctness`, `suspicious`, `style`, `complexity`, `perf` |
| `clippy::pedantic` | warn | priority −1; individual noisy lints are allowed below |
| `clippy::cargo` | warn | priority −1; manifest hygiene |
| `clippy::nursery` | **not enabled as a group** | individual nursery lints are enabled by name where they have proven stable |
| `clippy::restriction` | **never enabled as a group** | the group is contradictory by design; individual lints are enabled by name |

5.4. **Rust lints enabled individually** (`warn` unless stated): `missing_docs` (promoted to **deny** in the crate per §5.1; see §12), `missing_debug_implementations`, `unreachable_pub`, `unused_qualifications`, `unused_import_braces`, `unused_lifetimes`, `single_use_lifetimes`, `trivial_casts`, `trivial_numeric_casts`, `let_underscore_drop`, `macro_use_extern_crate`, `meta_variable_misuse`, `variant_size_differences`, `non_ascii_idents` (**deny**), `unsafe_code` (**forbid**).

5.5. **Clippy restriction lints enabled individually.**

| Lint | Level | Why |
| --- | --- | --- |
| `unwrap_used`, `expect_used` | **deny** | §6.2. Allowed in tests via `clippy.toml` |
| `panic`, `unimplemented`, `todo`, `unreachable` | **deny** | §6.1 |
| `indexing_slicing` | warn; **promoted to deny** in the `codec`, `transport` and driver modules (§5.1) | Index panics are the classic parser bug |
| `string_slice` | **deny** | Byte-index slicing of `str` panics on a char boundary |
| `arithmetic_side_effects` | warn; **promoted to deny** in the `codec` and driver modules (§5.1) | §7.1 |
| `as_conversions` | warn; **promoted to deny** crate-wide (§5.1) | §7.2 |
| `integer_division` | warn | Review that truncation is intended |
| `float_cmp_const` | warn | Alongside pedantic `float_cmp` |
| `lossy_float_literal` | warn | |
| `mem_forget` | **deny** | Leaking a guard hides a bug |
| `exit` | **deny** | Libraries do not terminate the process |
| `dbg_macro`, `print_stdout`, `print_stderr`, `use_debug` | **deny** | §14; allowed in tests and binaries via `clippy.toml` |
| `allow_attributes`, `allow_attributes_without_reason` | **deny** | Forces `#[expect(…, reason = "…")]`, which fails when the exception becomes stale |
| `missing_assert_message` | warn | Tests explain what failed |
| `tests_outside_test_module` | warn | |
| `unwrap_in_result` | warn | A `Result`-returning fn that unwraps is lying |
| `map_err_ignore` | **deny** | Never discard an error's source |
| `let_underscore_must_use`, `let_underscore_untyped` | warn | |
| `clone_on_ref_ptr` | warn | Write `Arc::clone(&x)` so the cheap clone is visible |
| `rc_buffer` | warn | |
| `rest_pat_in_fully_bound_structs` | warn | |
| `same_name_method` | warn | |
| `wildcard_enum_match_arm` | warn | Non-exhaustive public enums still deserve a deliberate `_` |
| `str_to_string`, `string_add`, `format_push_string` | warn | |
| `mod_module_files` | **deny** | `foo.rs` + `foo/`, never `mod.rs` (§10.4) |
| `pub_without_shorthand` | warn | `pub(crate)` not `pub(in crate)` |
| `renamed_function_params` | warn | Trait impls keep the trait's parameter names |
| `large_include_file` | warn | Fixtures are loaded at runtime, not `include_bytes!`'d, above 64 KiB |
| `undocumented_unsafe_blocks`, `multiple_unsafe_ops_per_block` | n/a | Nothing to document; kept out of the list on purpose |

5.6. **Nursery lints enabled individually** (warn): `future_not_send`, `missing_const_for_fn`, `redundant_pub_crate`, `use_self`, `needless_pass_by_ref_mut`, `iter_on_single_items`, `significant_drop_in_scrutinee`, `significant_drop_tightening`, `branches_sharing_code`.

5.7. **Pedantic lints allowed** (because their signal-to-noise ratio is poor for this codebase): `module_name_repetitions` (`modbus::ModbusAddress` is deliberate), `must_use_candidate` (we add `#[must_use]` by hand where it matters, §9.6), `similar_names`, `too_many_lines` (handled by §10.5 instead), `struct_excessive_bools` (capability structs are bool-heavy by nature; replace with an `#[expect]` if the lint fires for a non-capability type).

5.8. **Suppressing a lint.** Use `#[expect(clippy::some_lint, reason = "…")]`, never `#[allow]`. `expect` fails to compile when the lint no longer fires, so suppressions cannot rot. The `reason` states *why the code is right*, not what the lint is. Scope the attribute to the smallest item that needs it: an expression or a function, never a module or crate.

```rust
// Right
#[expect(clippy::cast_possible_truncation, reason = "count is validated <= 125 by Batcher::plan")]
let byte_count = (count * 2) as u8;

// Wrong
#![allow(clippy::cast_possible_truncation)]
```

5.9. **`clippy.toml`** (Appendix A) sets `msrv` to the workspace MSRV so Clippy never suggests an API newer than we support, allows `unwrap`/`expect`/`dbg`/`print`/`panic` in tests, and lists `disallowed-methods` and `disallowed-types` for §8.

5.10. **rustdoc lints.** `rustdoc::all` is warn; `rustdoc::broken_intra_doc_links`, `rustdoc::private_intra_doc_links` and `rustdoc::bare_urls` are **deny**. `cargo doc --no-deps --all-features` runs with `RUSTDOCFLAGS="-D warnings"`. — **CI**

---

## 6. Panics, errors, and failure

6.1. **Library code does not panic on input.** Not on bytes from the wire, not on a connection string, not on a profile file, not on a value a caller passed. `panic!`, `unreachable!`, `todo!`, `unimplemented!`, `assert!` in non-test code, and the implicit panics of indexing, slicing, integer overflow (in debug) and `Option::unwrap` are all treated as bugs. `debug_assert!` is permitted for internal invariants that are *also* checked by the type system or a preceding validation step, and must never be the only check. — **CI** (`clippy::panic` etc. deny)

6.2. **`unwrap` and `expect`.** Denied in library crates. The one sanctioned pattern is a `const` evaluated at compile time:

```rust
const MAX_REGISTERS: NonZeroU16 = NonZeroU16::new(125).unwrap(); // const-evaluated: cannot fail at runtime
```

even this carries `#[expect(clippy::unwrap_used, reason = "const-evaluated")]` on the item. In tests, examples and binaries under `tools/`, `unwrap`/`expect` are allowed (`clippy.toml`), but `expect("…")` with a message describing the *assumption* is preferred.

6.3. **One error type per public boundary.** the core (the crate root) defines `PlcError`; every public function in the crate that can fail returns `Result<T, PlcError>` (`stonecrab::Result<T>`). Driver and codec modules may define internal error enums (`ParseError`, `ModbusFrameError`) for precision, but they convert into `PlcError` at the module boundary via `From`, preserving `source()`. Public functions never return `Box<dyn Error>`, `String`, `&'static str` or `anyhow::Error`.

6.4. **`PlcError` shape.** `ErrorKind` is the discriminator callers match on; the struct carries a message, an optional `source`, and `Diagnostics` (§14.4). The struct body lives behind a `Box` so `Result<T, PlcError>` stays two words wide and `clippy::result_large_err` never fires:

```rust
#[derive(Debug, thiserror::Error)]
#[error("{}", inner.message)]
pub struct PlcError { inner: Box<ErrorInner> }
```

6.5. **`thiserror` in libraries, `anyhow` in binaries.** Library crates use `thiserror` (2.x) derives. `anyhow`/`eyre` are permitted only in `tools/`, examples, benches and integration tests, and are denied as dependencies of published crates by `deny.toml`. — **CI**

6.6. **Error messages** are complete sentences without a trailing period, lowercase first letter unless starting with an identifier, and include the identifying value: `invalid Modbus address "40001": ambiguous convention; declare zero-based or one-based`. They never include credentials, full payloads, or anything `Secret<T>` wraps (§14.5).

6.7. **Partial failure is data, not an error.** A batch read with one failed item returns `Ok(ReadResponse)` whose items carry per-item `Result`s. `Err(PlcError)` from `read` means the *operation* failed (disconnected, timed out before any item resolved, rejected during validation). Code that returns `Err` because one of N items failed is wrong.

6.8. **Ambiguous outcomes are first-class.** A write whose reply is lost returns `WriteOutcome::Unknown(reason)`, not `Err(Timeout)`. Code that retries a write to "resolve" the ambiguity is rejected in review regardless of configuration flags.

6.9. **Error construction helpers.** Prefer `PlcError::invalid_address(addr, reason)` style constructors over `PlcError::new(ErrorKind::InvalidAddress, format!(...))` at call sites; the constructors live in `stonecrab::error` (the core) and are the only place messages are formatted. This keeps wording consistent and makes redaction (§14.5) a single code path.

6.10. **`Result` in `Drop`.** `Drop` cannot return errors. Types owning a connection expose an explicit `close()`; `Drop` only releases memory and, at most, emits a `tracing::warn!` that `close()` was not called. `Drop` never blocks, never spawns, never performs I/O.

---

## 7. Numbers, bytes, and parsing

7.1. **Arithmetic is explicit.** In codec and driver code, `+ - * / <<` on integers are denied (`clippy::arithmetic_side_effects`). Use `checked_*` when overflow is an input error, `saturating_*` when a clamp is the specified behavior, `wrapping_*` only for counters whose rollover is part of the protocol (Modbus transaction id, sequence numbers) and say so in a comment. Elsewhere the lint is warn; an `#[expect]` is acceptable with a reason that explains why overflow is impossible.

7.2. **No `as` casts** anywhere in the crate (the `as_conversions` promotion of §5.1). Widening uses `From`/`u32::from(x)`; narrowing uses `TryFrom` and returns `ParseError::Overflow` on failure. Float↔int conversion uses explicit functions that document rounding and range (`f64::from(i32)` is fine; `f64 as i64` is not).

7.3. **Exact integers stay exact.** A `u64` counter is a `PlcValue::U64`, never an `f64`. Scaling in `stonecrab::sensors` converts to `f64` only when a profile step requires it; integer-only pipelines (bit fields, lookups, offsets by integer) stay in integers.

7.4. **Endianness is always written.** `u16::from_be_bytes`, `to_le_bytes`, never a shift-and-or by hand, never a dependency on host order. Word order for multi-register values is a parameter (`WordOrder`), never a default.

7.5. **Bounded readers only.** All parsing goes through `BoundedReader`/`BoundedWriter` from `stonecrab::codec`. A length field is compared against the caller-supplied maximum *before* any `Vec::with_capacity`, `Bytes::copy_to_bytes` or `split_to`. Direct `&[u8]` indexing in parsers is denied (§5.5).

7.6. **Floats.** No `==`/`!=` on floats (`clippy::float_cmp`). Comparisons in scaling use an explicit tolerance or `total_cmp`. `NaN` and infinities are detected (`is_finite()`) after every scaling chain and turn the measurement `Quality::Bad`; they are never propagated as values.

7.7. **Bit manipulation** uses named helpers (`bit(byte, index)`, `set_bit`) or the `bitvec` crate's types, with an explicit bit-order parameter where the protocol defines one (Modbus coils are LSB-first within a byte). A magic shift in a parser is rejected in review.

7.8. **Strings from the wire** are decoded with a declared encoding; `String::from_utf8_lossy` is permitted only when the result is marked `Quality::Uncertain` and the raw bytes are retained in `RawValue`.

7.9. **Time.** Deadlines, freshness and durations use `std::time::Instant` (monotonic). Receipt timestamps use `std::time::SystemTime`. The core depends on neither `chrono`, `time` nor `jiff`; conversions to calendar types are the application's job (ADR pending for `PlcValue::DateTime`). Never derive a device timestamp from a local clock.

---

## 8. Async and concurrency

8.1. **Runtime boundary.** The core has no dependency on Tokio or any executor and compiles with `--no-default-features` on any target: Tokio enters the crate only through the `transport` feature (turned on by `modbus` and `testkit`) and the adapters, and is scoped to the features actually used (`rt`, `net`, `time`, `sync`, `macros`, `io-util`); `tokio = { features = ["full"] }` is prohibited in the published crate. **CI** (`cargo-deny` + feature job)

8.2. **Object-safe boundary = boxed futures.** The public traits `Driver` and `Connection` return `OpFuture<'_, T>` (`Pin<Box<dyn Future<Output = Result<T>> + Send + '_>>`). Implementations write `Box::pin(async move { … })`. The `async-trait` macro is not used; it hides the `Send` bound and the allocation we are deliberately paying for. Inside a crate, `async fn` and `impl Future` are used freely.

8.3. **Every future is `Send`.** `clippy::future_not_send` is enabled. A public future that is not `Send` is a bug. Holding a `std::sync::MutexGuard`, `RefCell` borrow or `Rc` across an `.await` is rejected (`clippy::await_holding_lock`, `await_holding_refcell_ref`).

8.4. **No blocking on the runtime.** `std::thread::sleep`, synchronous `std::net`/`std::fs` I/O, and blocking `serialport` calls are disallowed in async code (`clippy.toml` `disallowed-methods`). Serial, SPI and I²C run on a dedicated OS thread owned by `BlockingWorker`, which talks to the async side through bounded channels. `tokio::task::spawn_blocking` is acceptable for one-shot operations (opening a port); a long-lived device loop gets its own thread so it cannot starve the blocking pool.

8.5. **Every I/O call has a deadline.** `Transport::send`/`recv` take a `Deadline`. There is no `read().await` without `timeout`/deadline anywhere in the workspace. A function that performs I/O and takes no deadline parameter has a documented deadline source (the connection's `Timeouts`) in its rustdoc.

8.6. **Cancellation safety is documented.** Every public async fn's rustdoc has a `# Cancel safety` section stating one of: *cancel-safe (no bytes sent on drop)*, *not cancel-safe: request may have been transmitted; outcome is Unknown*, or *not cancel-safe: connection must be resynced*. `tokio::select!` branches may only contain cancel-safe futures or futures whose cancellation consequence is handled in the `else`/drop path.

8.7. **Structured tasks.** `tokio::spawn` is a disallowed method (`clippy.toml`), so every spawn site carries `#[expect(clippy::disallowed_methods, reason = "…")]` naming the owner that keeps the `JoinHandle` (or `JoinSet`) and aborts/awaits it in `close()`. Detached tasks that outlive their owner are prohibited; a `tokio::spawn(async move { … });` with the handle discarded is rejected in review even with the `expect`. Task-spawning types document their shutdown order.

8.8. **Channels are bounded.** `tokio::sync::mpsc::unbounded_channel`, `UnboundedSender` and `UnboundedReceiver` are disallowed types (`clippy.toml`). Every channel capacity is a named constant or a `ResourceLimits` field with a documented upper bound, and every producer's behavior on a full channel is one of the three `QueuePolicy` variants with its loss counter updated.

8.9. **One owner per connection.** Connection state lives in exactly one task or one `Mutex`-guarded state machine (`ConnectionGuard`). Requests are serialized through it; "just clone the stream" is not a pattern here. For Modbus RTU the serialization is per *bus*, not per connection, and the `SerialTransport` enforces it.

8.10. **Shared state.** Prefer message passing. When a lock is necessary: `std::sync::Mutex` for data never held across `.await`; `tokio::sync::Mutex` only when the critical section must await. Lock order is documented in the module docs when more than one lock exists. `parking_lot` is permitted for its poison-free guards; if used, it is used consistently within a crate.

8.11. **Atomics** are limited to counters (`AtomicU64` in `LossCounters`) and flags. Ordering is `Relaxed` for statistics, `AcqRel`/`Acquire`/`Release` for flags that gate reads of other data, with a comment naming the data being protected. Lock-free data structures are not written in this workspace.

8.12. **Timers in tests.** Timing-dependent tests use `#[tokio::test(start_paused = true)]` with `tokio::time::advance`, or the testkit `FakeClock`. A `sleep` in a test to "let things settle" is rejected (§13.5).

---

## 9. API design

9.1. **Follow the Rust API Guidelines.** The [checklist](https://rust-lang.github.io/api-guidelines/checklist.html) applies in full. The items below either go beyond it or are the ones most often missed.

9.2. **Public enums and structs are `#[non_exhaustive]`** unless an ADR says the shape is final. This includes `PlcValue`, `ErrorKind`, `Quality`, `ConnectionState`, `Capabilities` and every config struct. Config structs are constructed via builder or `Default` + field assignment, never by struct literal from outside the crate.

9.3. **Newtypes for identifiers and units.** `TagId`, `UnitId`, `TransactionId`, `SubscriptionId`, `ProfileId`, `Deadline`. A bare `u8` named `unit_id` in a public signature is rejected. Newtypes implement `Debug`, `Clone`, `Copy` where trivially possible, `PartialEq`, `Eq`, `Hash`, `Display`, and `From`/`TryFrom` for the underlying type (`TryFrom` when a range applies).

9.4. **Builders** for anything with more than three configurable fields: `ConnectionConfig::builder()`, `ReadRequest::builder()`. Builders are `#[must_use]`, consume `self`, validate in `build() -> Result<T>`, and never panic. A builder's defaults are documented on the builder type, not scattered over setter docs.

9.5. **Traits.** Public traits are object-safe unless named `*Ext`. Traits that external crates should not implement are sealed (`mod private { pub trait Sealed {} }`). A trait with one implementor and no test double is a struct. `Driver` and `Connection` are the only traits application code is expected to hold as `dyn`.

9.6. **`#[must_use]`** on: every builder method, every function returning `Result` whose `Ok` is not `()` (Rust already warns on unused `Result`), every method returning a new value from `self` (`with_type`), `Deadline`, `CancellationToken`, `Subscription`, and `WriteResponse`. Dropping a `WriteResponse` unexamined is how an `Unknown` outcome gets lost.

9.7. **Conversions and getters.** `as_` (cheap borrow), `to_` (expensive, borrowed), `into_` (consumes) per C-CONV. Getters have no `get_` prefix. `iter()`, `iter_mut()`, `into_iter()` for collections. `TryFrom` over `from_*` constructors that can fail.

9.8. **Generics vs. `dyn`.** Public boundaries that applications cross at runtime are `dyn` (the registry holds `Arc<dyn Driver>`). Internal codec and value code is generic or monomorphic; a `Box<dyn Trait>` inside a parser is a smell. Do not expose `impl Trait` in public *trait* method return types (not object-safe); `impl Trait` in inherent methods is fine.

9.9. **Feature flags are additive.** Enabling a feature never removes an item, changes a default, opens a socket or probes hardware. Feature-gated code is gated at the module boundary (`#[cfg(feature = "serial")] pub mod serial;`), not sprinkled across a module. A type that exists under two features has one definition. `cargo hack --feature-powerset` verifies every combination compiles. — **CI**

9.10. **Semver.** Pre-1.0: breaking changes bump the minor version and come with a migration note in `CHANGELOG.md`. `cargo semver-checks` runs on every PR against the last published version and its report is attached to the PR; a flagged break without a version bump blocks merge. — **CI**

9.11. **`Send + Sync + 'static`.** Every public type that can end up in a `dyn` object or a spawned task is `Send + Sync + 'static`, and there is a compile-time assertion for each in that crate's tests:

```rust
#[test]
fn public_types_are_send_sync() {
    fn assert<T: Send + Sync + 'static>() {}
    assert::<PlcError>();
    assert::<ReadResponse>();
}
```

9.12. **No `Box<dyn Any>` in public APIs.** `ProtocolAddress: Any` plus trait upcasting lets a driver downcast its own address type; nothing else needs dynamic typing.

9.13. **Stability of string formats.** Connection-string and address syntax are public API. Changes to either are breaking, get a parity note, and keep the previous form as a documented alias for at least one minor version.

---

## 10. Crate and module layout

10.1. **One responsibility per module**, matching `ARCHITECTURE.md` §1; Stonecrab ships as one crate (ADR-0018), and each former crate is a module behind a Cargo feature where it carries a dependency. A driver module contains exactly one protocol family. Shared code between two drivers moves down into `stonecrab::codec` or `stonecrab::transport`, never sideways.

10.2. **`lib.rs`** contains: crate-level docs, `#![forbid(unsafe_code)]`, the docs.rs `cfg_attr`, `mod` declarations, and `pub use` re-exports. No type definitions, no logic.

10.3. **Public surface is deliberate.** Items are private by default; `pub(crate)` for cross-module use; `pub` only for the documented API. `unreachable_pub` warns on `pub` items that are not actually reachable. Re-export at the crate root what applications use; deep paths (`stonecrab::modbus::session::framing::MbapHeader`) are not part of the public API and are marked `#[doc(hidden)]` if they must be `pub` for the test kit or another module.

10.4. **Module files.** `foo.rs` with children in `foo/`; `mod.rs` is denied (`clippy::mod_module_files`). Test modules live in the file they test (`#[cfg(test)] mod tests`); integration tests in `tests/`; shared test helpers in `stonecrab::testkit` (the `testkit` feature), not copied between test files.

10.5. **Size.** A module over ~800 lines or a function over ~80 lines is a review prompt, not a hard limit; the question is whether it has one responsibility. Generated files are exempt.

10.6. **Driver module skeleton** (required layout so reviewers find things):

```text
crates/stonecrab/
├── docs/components/modbus.md   capability matrix, address syntax, support level (the module's rustdoc root)
├── src/
│   ├── lib.rs                  the module declaration, feature gate and §5.1 promotions
│   ├── modbus.rs               module root: docs include, mod declarations, re-exports
│   └── modbus/
│       ├── driver.rs           Driver impl
│       ├── connection.rs       Connection impl around ConnectionGuard
│       ├── address.rs          ModbusAddress + parsing + conventions
│       ├── session.rs          request/response state machine
│       ├── framing.rs          framing/mbap.rs (rtu.rs when RTU lands)
│       ├── pdu.rs              pdu/ hand-written PDU codec
│       ├── batch.rs            Batcher
│       ├── decode.rs           TypedDecoder
│       └── options.rs          ModbusOptions
├── tests/                      modbus_*.rs: contract, interop, transport-integration
└── (generated/ with MANIFEST.toml once the generator exists; never edited)
fixtures/modbus/                golden vectors and the interop register map, repository data
```

10.7. **Workspace dependencies.** Every external dependency's version is declared once in `[workspace.dependencies]`; the crate references it with `dep = { workspace = true }`. Two versions of the same crate in the tree is a `cargo deny` warning that must be resolved or justified.

10.8. **Examples** under `crates/stonecrab/examples/` compile in CI (`cargo build --examples --all-features`) and are smoke-run against the simulator where possible. An example that needs hardware says so in its first comment and is excluded from the smoke run by name.

---

## 11. Naming

11.1. **RFC 430** conventions: `UpperCamelCase` types/traits/enum variants, `snake_case` functions/modules/fields, `SCREAMING_SNAKE_CASE` constants/statics, `kebab-case` crate and feature names.

11.2. **Acronyms are words.** `OpcUa`, `Mbap`, `Pdu`, `Crc16`, `Tcp`, `I2c`, `Spi`, `Adxl345`, `Max31855` — never `OPCUA`, `MBAP`, `I2C`. Lowercase forms follow: `opc_ua`, `mbap_header`, `i2c_bus`.

11.3. **Protocol vocabulary follows the standard that owns it.** Modbus: *client/server* (the Modbus Organization's current terms), *unit identifier*, *function code*, *holding register*, *exception code*. Where an older term is widely used (*master/slave*, *slave id*), the docs say "unit identifier (historically 'slave id')" once and the code uses the current term. OPC UA: *node id*, *secure channel*, *session*, *monitored item*. Upstream PLC4X names are given in the parity record, not used in code.

11.4. **Errors** are named by cause, not with an `Error` suffix inside an enum: `ErrorKind::Timeout`, not `ErrorKind::TimeoutError`. Structs that *are* errors end in `Error` (`PlcError`, `ParseError`, `ProfileError`).

11.5. **Booleans.** Fields are adjectives or participles (`readable`, `truncated`, `redacted`); predicate methods are `is_*`/`has_*`/`can_*`. Negated names (`not_ready`) are rejected.

11.6. **Measurement names** in profiles are `snake_case` ASCII, unique within a profile, and double as the `TagId`. Units use UCUM-style symbols (`degC`, `kPa`, `m3/h`, `A`, `V`, `kWh`); a unit table lives in `stonecrab::sensors::units` and a profile may not introduce a symbol absent from it without adding it there with a test.

11.7. **Test names** read as sentences: `fn mbap_framer_rejects_length_over_260()`, `fn read_response_preserves_tag_order_after_split()`. No `test_` prefix. Property tests are `prop_*`; fuzz targets `fuzz_<crate>_<parser>`; hardware tests `hw_<device>_*`.

11.8. **Feature names**: protocol or capability nouns (`modbus`, `modbus-rtu`, `opcua`, `serial`, `sensors`, `adapters-linux`, `full`). No `use-*`, `with-*`, `enable-*` prefixes (`clippy::cargo` lints enforce the obvious cases).

---

## 12. Documentation

12.1. **Every public item is documented.** `missing_docs` is `deny` in published crates. A doc comment restating the item's name (`/// The connection config.`) does not count; it must say what the item is *for* and anything a caller could get wrong.

12.2. **Required sections**, in this order when applicable:

| Section | Required on |
| --- | --- |
| summary line | everything |
| `# Examples` | every public type's primary constructor and every top-level operation; examples are doctests and compile |
| `# Errors` | every fn returning `Result`: list the `ErrorKind`s it can produce |
| `# Panics` | must read "This function does not panic." or be absent; it is never a list |
| `# Cancel safety` | every public `async fn` / `OpFuture`-returning fn (§8.6) |
| `# Wire effects` | every fn that may transmit: what the device sees if the call is cancelled or times out |
| `# Compatibility` | anything whose behavior maps to an upstream PLC4X behavior, with the parity record id |

12.3. **Crate docs** (`//!` in `lib.rs`) state: purpose, the support level of each capability (`build-tested` … `hardware-verified`, from `SPEC.md`), the Cargo features, a 20-line example, and a link to the crate's parity record. The crate `README.md` is included with `#![doc = include_str!("../README.md")]` so the two never drift.

12.4. **Doctests run.** `cargo test --doc --workspace --all-features` is a CI step. Doctests that need a device are written against the simulator or marked `no_run` with a comment explaining why; `ignore` is not used.

12.5. **Intra-doc links** (`[`ReadRequest`]`) for every mention of another item; bare URLs are denied. Identifiers in prose are in backticks (`clippy::doc_markdown`).

12.6. **Architecture Decision Records** go in `docs/adr/NNNN-short-title.md` using the template in Appendix D. An ADR is required for: a new crate, a new public trait, a change to §2/§4/§5/§8/§15 of this document, adoption of an external protocol implementation (SPEC "porting rules"), a deliberate divergence from upstream behavior, and any dependency that uses `unsafe` or ships native code.

12.7. **Changelog** follows [Keep a Changelog](https://keepachangelog.com/) with one section per published crate per release. Entries are written in the PR that makes the change, under `Unreleased`. Generated from commits is acceptable as a *draft* only.

12.8. **Diagrams** in docs are Mermaid, kept in `ARCHITECTURE.md` or the crate README, and updated in the same PR as the code they describe. A diagram that disagrees with the code is a bug in the diagram.

---

## 13. Testing

13.1. **Layers** follow `SPEC.md` "Test layers". Each released crate carries, at minimum: unit tests, golden-vector tests, property tests on its parsers, a contract-suite run (drivers), and the mandatory failure cases that apply to it. The test plan for each module lists which `SPEC.md` mandatory case each test covers, as a `// covers: SPEC mandatory-failure-cases / "truncated headers"` comment on the test or in the test's docstring, so coverage against the list is greppable.

13.2. **Runner.** `cargo nextest` is the CI runner (per-test process isolation, timeouts, retries off). `cargo test` must also pass; nothing may depend on nextest-only behavior. Nextest's `slow-timeout` is 60 s with `terminate-after = 2`; a test that needs longer is a soak test and lives under `tests/soak_*.rs` behind the `soak` feature.

13.3. **Determinism.** No test depends on wall-clock time, port availability (bind `127.0.0.1:0` and read the assigned port), ordering of a `HashMap`, thread scheduling, or network access beyond loopback. Property tests set `PROPTEST_CASES` from CI (256 on PRs, 4096 on the scheduled run) and a failing case is committed to `proptest-regressions/`.

13.4. **No sleeping.** `std::thread::sleep` and `tokio::time::sleep` in tests are rejected; use paused time (§8.12) or an explicit synchronization point (a `Notify`, a channel, a `FakeTransport` script step).

13.5. **No mocks of our own traits except through the testkit.** `FakeTransport`, `FakeClock`, `ScriptedPeer` and `SimDriver` are the sanctioned doubles. A hand-rolled `struct MockConnection` in a driver module duplicates behavior and drifts; put it in the testkit (`stonecrab::testkit`) or use the simulator.

13.6. **Golden vectors.** Wire bytes live in `fixtures/<protocol>/<name>.hex` (one hex line, comments with `#`), with `<name>.expected.json` authored *independently* of the decoder (from the protocol specification or a capture, never by running our decoder and pasting the output), and a `PROVENANCE.toml` per directory naming the source, revision and license. A decoder test loads the vector, decodes, compares against the expected JSON, re-encodes, and compares bytes. Both directions are asserted separately so a matching encode/decode bug cannot cancel out.

13.7. **Adversarial inputs are tests, not fuzz-only.** Every parser has explicit tests for: zero length, length one byte short, length one byte long, maximum-plus-one length, every invalid discriminator, and a corrupted checksum. The fuzzer finds what we did not think of; the test suite proves we thought of the obvious.

13.8. **Fuzzing.** `fuzz/` holds one `cargo-fuzz` target per untrusted parser (`fuzz_codec_bounded_reader`, `fuzz_modbus_mbap`, `fuzz_modbus_rtu`, `fuzz_core_connection_string`, `fuzz_sensors_profile_toml`, …). The corpus for each target is committed (`fuzz/corpus/<target>/`) and every crash reproducer becomes a named regression test in the owning crate *before* the fix merges. Durations follow `SPEC.md` numeric gates (60 s per target on PRs touching a parser; 30 min scheduled; 8 h on a release candidate).

13.9. **Mutation testing.** `cargo mutants` runs on the scheduled job over `address`, `value`, `scaling`, `framing` and `decode` modules; the kill rate per module is recorded in the release evidence. A survived mutant is either killed with a test or listed with a reason in `mutants.toml`'s exclusions.

13.10. **Coverage.** `cargo llvm-cov nextest --workspace --all-features --lcov` with `generated/`, `examples/`, `tools/`, `benches/` and `fuzz/` excluded. `xtask coverage-gate` (today `scripts/coverage-gate.sh`, ADR-0005) enforces per-module line ≥ 90 % (the modules are the path prefixes of `crates/stonecrab/src`), changed-lines ≥ 95 %, branch ≥ 80 % where available, per `SPEC.md`. An error path may not be excluded from coverage to raise a number; `#[coverage(off)]` is prohibited (and nightly-only anyway).

13.11. **Hardware tests** are `#[ignore = "hardware: <device>"]`, named `hw_*`, read their bench configuration from `STONECRAB_HW_BENCH=<path to bench.toml>`, and are run only by the controlled hardware runner with `cargo nextest run --run-ignored ignored-only -E 'test(/^hw_/)'`. Each run writes a record in the `docs/hardware/` format and the PR that claims a support level links it.

13.12. **Interoperability tests** run against an independently implemented peer (for Modbus: a third-party server such as `diagslave`/`pymodbus`, started from a container in CI; for OPC UA: the reference server). They are tagged `interop_*` and run in the `interop` CI job, not in the default unit run.

13.13. **Soak tests** live under `tests/soak_*.rs` behind the `soak` feature, run on the scheduled job for the duration in `SPEC.md`, and assert on RSS, task count and descriptor count from `/proc/self` at intervals, not just on "did it finish".

13.14. **Assertions** carry messages (`assert_eq!(got, want, "frame {name}: decoded count")`). Prefer `assert_eq!` over `assert!(a == b)`; prefer `assert_matches!` (`std::assert_matches` is unstable, so the `assert_matches` crate) over `matches!` inside `assert!`.

13.15. **Snapshot tests** (`insta`) are permitted for `Debug` output of diagnostics and for generated code, never for wire bytes (use golden vectors) and never as a substitute for an explicit assertion on a value that matters.

---

## 14. Logging and diagnostics

14.1. **`tracing` only.** No `log`, no `println!`/`eprintln!`, no `dbg!` outside tests and `tools/`. Published crates depend on `tracing` and nothing else for output; they never install a subscriber.

14.2. **Spans per operation.** `connect`, `read`, `write`, `subscribe`, `browse` each open a span with fields `scheme`, `endpoint` (redacted of credentials), `op`, and `txn`/`request_id`. Driver-internal work is instrumented with `#[tracing::instrument(skip(self), level = "debug")]`; `skip` any argument that can contain payload or credentials.

14.3. **Levels.**

| Level | Use |
| --- | --- |
| `error` | The connection is Failed, or data was lost despite policy (should be impossible) |
| `warn` | Reconnecting, `WriteOutcome::Unknown`, stale marking, late reply discarded, retry budget exhausted |
| `info` | Connected, closed, subscription created/ended, capability summary at connect |
| `debug` | Per-request outcome, batching plan, backoff delay chosen |
| `trace` | Frame-level bytes (length and function code only; payload bytes only under the `trace-payloads` feature) |

A library crate emits nothing above `warn` in steady-state operation. A log line per poll tick at `info` is a bug.

14.4. **`Diagnostics`** on every `PlcError` is structured (operation, state, latency, retries, category) and is the thing applications are expected to log/metricize; the error message is for humans. Drivers populate it at the point of failure; no layer above the driver reconstructs it.

14.5. **Redaction.** Credentials are typed `Secret<T>`, whose `Debug` and `Display` print `[REDACTED]`. Connection strings are logged through `ConnectionConfig::redacted_display()`. Payload bytes are never logged above `trace`, and `trace` payload logging is behind a Cargo feature so it is compiled out of production builds. `tracing` field values that could carry payload (`value`, `raw`) are logged as `?value.value_type()` not `?value`.

14.6. **Metrics** are not emitted by published crates in M1–M3. `LossCounters`, `Diagnostics` and `watch_state()` expose what an application needs to build its own; a `metrics` feature is an ADR candidate once there are two consumers.

---

## 15. Dependencies and supply chain

> Local tooling exception: [ADR-0003](adr/0003-request-response-contracts.md)
> permits a path-only development dependency on the unpublished shared testkit.
> Wildcard registry versions remain prohibited.

15.1. **Justify every dependency** in the PR that adds it: what it does, why `std` is insufficient, its `unsafe` status (§4.4), its license, its maintenance signal (last release, open advisories), and the features enabled. A reviewer may ask for it to be replaced by 50 lines of first-party code; "it's popular" is not a justification.

15.2. **`cargo deny`** runs on every PR with the policy in Appendix A: licenses allow-listed (Apache-2.0, MIT, BSD-2/3-Clause, ISC, Zlib, Unicode-3.0, MPL-2.0 for named crates only; GPL/LGPL/AGPL/SSPL/BUSL denied), advisories denied (no unpatched RUSTSEC entries, yanked crates denied), sources restricted to crates.io, wildcard versions denied, duplicate major versions warned. — **CI**

15.3. **`Cargo.lock` is committed** and the CI build uses `--locked`. Dependabot/Renovate PRs for lockfile bumps are grouped weekly and must pass the full matrix; they are not auto-merged.

15.4. **Versions** in `[workspace.dependencies]` use caret ranges at the minor level (`tokio = "1.47"`), never `*`, never `=` pins (except temporarily with an issue link), never git sources in published crates. A git dependency anywhere in the workspace needs an ADR and a plan to remove it.

15.5. **Minimal features.** Every dependency has `default-features = false` unless a default feature is needed and named. `cargo hack check --feature-powerset --no-dev-deps` proves each of our features pulls only what it says. — **CI**

15.6. **Unused dependencies** are detected by `cargo machete` in CI (nightly `cargo udeps` is not required). — **CI**

15.7. **Banned crates** (`deny.toml` `[bans]`): `openssl`/`openssl-sys` (use `rustls`), `async-trait` (§8.2), `anyhow`/`eyre`/`color-eyre` in published crates (§6.5), `chrono`/`time`/`jiff` in the core (§7.9), `lazy_static`/`once_cell` (use `std::sync::LazyLock`/`OnceLock`), `log` (§14.1), `failure`, `error-chain`.

15.8. **Sanctioned crates** for common needs, so the same job is not done three ways:

| Need | Crate |
| --- | --- |
| byte buffers | `bytes` |
| bit vectors | `bitvec` |
| error derive | `thiserror` 2 |
| async runtime / net / time | `tokio` (scoped features) |
| cancellation bridge | `tokio-util` (`CancellationToken`, codecs) in transport only |
| serial | `serialport` (sync, on a `BlockingWorker` thread) |
| SPI / I²C on Linux | `spidev`, `i2cdev` |
| TLS (OPC UA) | `rustls` + `rustls-pki-types`, `aws-lc-rs` or `ring` provider per ADR |
| config/profile parsing | `toml` + `serde` |
| ordered maps | `indexmap` |
| property tests | `proptest` |
| snapshots | `insta` |
| benchmarks | `criterion` (or `divan`; pick one per ADR) |
| tracing | `tracing`; `tracing-subscriber` in tools/examples only |
| CLI (tools only) | `clap` with derive |

15.9. **`cargo vet`** is not adopted yet; adoption is an ADR candidate before 1.0. Until then, §15.1's written justification and the `DEPENDENCY_AUDIT.md` entries are the audit trail.

15.10. **SBOM.** `cargo cyclonedx` output is attached to every release so downstream regulated users can ingest it.

---

## 16. Performance

16.1. **Measure first.** A performance change without a benchmark delta in the PR description is a refactor and is reviewed as one. Benchmarks live in `benches/` using the crate chosen in §15.8 and run on the scheduled job; results are stored as JSON under the release evidence.

16.2. **Required benchmarks** per `SPEC.md`: parse/serialize time and allocation count by message type; single and batched reads against the simulator; one- and multi-device polling; subscription queue tail latency; soak-test RSS/CPU/descriptor counts; Modbus-only vs OPC-UA-enabled binary size on the edge target.

16.3. **Allocation discipline.** Hot paths (frame decode, value decode, batch planning) allocate at most once per message and that allocation is sized from a validated length. `Bytes::slice` over copying; `SmallVec` is not used until a benchmark justifies it. `#[inline]` is added only with a benchmark showing the gain; `#[inline(always)]` is prohibited.

16.4. **Regression gate.** A repeatable regression above 10 % on the same configuration blocks release until explained or fixed (`SPEC.md`). CI compares the candidate against the last release's stored baseline on the same runner class.

16.5. **Profiles.** `[profile.release]` sets `lto = "thin"`, `codegen-units = 1`, `debug = "line-tables-only"` (so profiles symbolize), `overflow-checks = true` (our checked arithmetic makes this nearly free and it keeps soak runs honest). Consumers choose their own profile; we do not set `panic = "abort"` for them.

16.6. **No premature `no_std`.** The core avoids `std`-only APIs where `core`/`alloc` suffice (`core::error::Error`, `alloc::vec::Vec`) so a future `no_std` feature is plausible, but we do not maintain a `no_std` build until an ADR commits to it.

---

## 17. Generated code

17.1. **Location and marker.** Generated Rust lives under `src/generated/` in the owning driver module. Every generated file begins with:

```rust
// @generated by stonecrab-codegen <version> from <mspec path> @ <upstream commit>
// DO NOT EDIT. Regenerate with `cargo xtask generate --protocol modbus`.
#![allow(clippy::all, clippy::pedantic, missing_docs)]
```

and a `MANIFEST.toml` in the same directory records upstream tag, commit, generator version, options and timestamp (`GenerationManifest`).

17.2. **Never hand-edited.** CI regenerates every protocol and fails on a non-empty `git diff` of `src/generated/`. A fix belongs in the generator or in the upstream MSpec (with the upstream issue linked); a temporary first-party patch is a `// @generated-override` file *outside* `generated/` that wraps or replaces the item, with an ADR and a removal issue. — **CI**

17.3. **Generated code still obeys §4, §6, §7.** The generator emits `#![forbid(unsafe_code)]`-compatible code, uses `BoundedReader`/`BoundedWriter`, never indexes a slice directly, never panics on input, and bounds recursion depth. A generator change is tested by regenerating all protocols and running the golden-vector and fuzz corpora.

17.4. **Determinism.** Given the same inputs, the generator produces byte-identical output: sorted iteration, no timestamps in code (only in the manifest), no absolute paths.

17.5. **Coverage.** Generated files are excluded from the handwritten coverage thresholds but have their own gate: every enabled message variant has at least one golden vector and one negative vector (`SPEC.md` "Generated codec behavior").

17.6. **Java bootstrap.** Until the native generator exists, `tools/stonecrab-codegen` may shell out to the pinned upstream Maven build to produce an intermediate model. That path is maintainer-only, documented in `tools/stonecrab-codegen/README.md`, never run by CI on pull requests, and never required to build or test any published crate.

---

## 18. Version control and pull requests

> Repository override: [ADR-0004](adr/0004-master-default-branch.md) replaces
> `main` with `master` throughout branch naming and hardware-job references in
> §18–19. The source policy below is otherwise retained.

18.1. **Branching.** Trunk is `main` and is always releasable. Work happens on short-lived branches named `<type>/<scope>-<slug>` (`feat/modbus-rtu-framing`, `fix/core-deadline-overflow`, `gen/modbus-regen-1.0.1`). No long-lived feature branches; land behind a feature flag or an unexposed module instead.

18.2. **Commits** follow [Conventional Commits 1.0](https://www.conventionalcommits.org/):

```text
<type>(<scope>)!: <summary in imperative mood, ≤ 72 chars>

<body: what and why, wrapped at 72>

BREAKING CHANGE: <migration note>      (only with !)
Refs: #123
Signed-off-by: Name <email>
```

Types: `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`, `ci`, `chore`, `style`, `revert`, and the project-specific `gen` (regeneration-only commits, which must contain nothing outside `src/generated/` and the manifest). Scopes are crate short names: `core`, `transport`, `codec`, `sim`, `modbus`, `opcua`, `eip`, `s7`, `sensors`, `max31855`, `adxl345`, `testkit`, `codegen`, `xtask`, `docs`, `ci`.

18.3. **DCO.** Every commit carries a `Signed-off-by:` line (`git commit -s`) asserting the [Developer Certificate of Origin](https://developercertificate.org/). The DCO check blocks merge. There is no CLA. — **CI**

18.4. **One concern per PR.** A PR that mixes a regeneration with a logic change, or a reformat with a fix, is split. Target size is reviewable in one sitting (< ~600 changed handwritten lines); larger PRs are stacked.

18.5. **PR requirements** (template in Appendix B): what/why, the `SPEC.md` section or parity record touched, test evidence, support-level claim change if any, dependency justification if any, and the checklist. The PR title follows the commit format because the squash commit inherits it.

18.6. **Review.** One maintainer approval for ordinary changes; two for changes to CI workflows, the hardware runner, release tooling, or anything executed on the hardware bench (`SPEC.md`: review of executable changes before hardware jobs run). Authors do not approve their own PRs. Review comments that request a change to a **MUST** in this document are not optional.

18.7. **Merge.** Squash-merge, PR title as the commit subject, PR body as the commit body, with `Signed-off-by` preserved. No merge commits on `main`; no force-pushes to `main`; branch protection enforces required checks (§19).

18.8. **Required files for a new driver PR** (from `SPEC.md` "Open-source project requirements"): capability metadata (`Capabilities` impl + crate README matrix), address documentation, fixtures with provenance, mandatory failure-case tests, contract-suite run, and the support-level evidence. A driver PR missing any of these is draft, not reviewable.

18.9. **Bug fixes carry a regression test** named after the issue (`fn issue_123_rtu_crc_rejects_byte_swapped()`), or a documented reason why the failure can only be demonstrated on hardware, in which case the hardware record is linked.

---

## 19. CI gates

Every pull request runs the following; all are required for merge unless marked otherwise.

| Job | What it runs | Blocking |
| --- | --- | --- |
| `fmt` | `cargo fmt --all --check` | yes |
| `clippy` | `cargo clippy --workspace --all-targets --all-features -- -D warnings` on stable | yes |
| `msrv` | `cargo +<rust-version> check --workspace --all-features` and the tests of the published crate | yes |
| `test` | `cargo nextest run --workspace --all-features` + `cargo test --doc` on Linux x86_64 | yes |
| `features` | `cargo hack check --feature-powerset --depth 2 --no-dev-deps`; `--no-default-features`; each driver feature standalone; `full` | yes |
| `cross` | `cargo check --target aarch64-unknown-linux-gnu` and `armv7-unknown-linux-gnueabihf`, all features | yes |
| `docs` | `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features` | yes |
| `deny` | `cargo deny check` (licenses, advisories, bans, sources) | yes |
| `geiger` | `cargo geiger --workspace`; fails on any first-party `unsafe` | yes |
| `machete` | `cargo machete` | yes |
| `semver` | `cargo semver-checks` against last published version | yes when a published crate changes |
| `regen` | `cargo xtask generate --all && git diff --exit-code` | yes |
| `coverage` | `cargo llvm-cov nextest` + `xtask coverage-gate` | yes |
| `fuzz-smoke` | 60 s per target touched by the PR | yes when a parser changes |
| `interop` | containerized third-party peers | yes for driver modules |
| `beta` | `cargo check` + `clippy` on beta | report only |
| `package` | `cargo package --list` diff against committed allow-list of shipped files | yes |
| `dco` | Signed-off-by on every commit | yes |

Scheduled (nightly) jobs add: 30-minute fuzzing per target, mutation testing, soak tests, benchmarks against baseline, Miri on the core and `codec`, and `cargo update` dry-run advisories. Hardware jobs run on the controlled runner only on `main` and release branches after the §18.6 review.

---

## 20. Releases and versioning

20.1. **One crate, one version.** `stonecrab` is the only published crate (ADR-0018); its modules do not version separately, and a fix to any module is a release of the whole crate. Features are additive and are part of the public API. Pre-1.0 rules per §9.10.

20.2. **Release tooling.** `release-plz` (or `cargo-release`; one per ADR) bumps versions from Conventional Commits, updates `CHANGELOG.md`, tags `v<version>`, and publishes the one crate. Tags are signed. A human reviews the generated changelog and adds the release-note sections `SPEC.md` requires: upstream baseline, supported operations per driver, actual hardware evidence, breaking changes.

20.3. **Release candidate checklist** (`xtask release-check`): capability matrix matches `Capabilities` impls; parity records have no `implemented` status without a test id; every `hardware-verified` claim links a record; 8-hour fuzz run clean; 24-hour soak clean; benchmarks within 10 % of baseline; `cargo package --list` reviewed; SBOM generated; README support table regenerated.

20.4. **1.0** requires the stable core API and the explicit parity statement from `SPEC.md`. It does not require M5.

20.5. **Yanking.** A published version with a soundness or safety defect (unbounded allocation, panic on input, write replay) is yanked within 24 hours of confirmation with a `RUSTSEC` advisory filed; a fixed patch release follows.

---

## 21. Security

21.1. **Threat model in one line:** every byte that arrives on a transport is attacker-controlled; every connection string, profile and address string may be attacker-supplied through an application's configuration. The parsers are the attack surface; §7 and §13.7–13.8 are the defense.

21.2. **Reporting.** `SECURITY.md` names the private channel (GitHub private vulnerability reporting on the repository), the acknowledgement SLA (3 business days), the coordinated-disclosure window (90 days, shorter by agreement), and the supported-versions table. It exists before the first crate is published.

21.3. **Credentials** never appear in logs, errors, `Debug` output or panics (there are none). `Secret<T>` wraps them; `Zeroize` is applied where the type supports it.

21.4. **TLS** (OPC UA) uses `rustls` with system or explicitly configured trust anchors; certificate validation cannot be disabled by a boolean. A `dangerous-accept-any-cert` option, if ever added for lab use, is a Cargo feature that is off by default, prints a `warn!` on every connect, and is documented as unsafe for production.

21.5. **No network by default.** Importing a crate opens nothing. Discovery and broadcast are explicit operations on a `Connection` or a dedicated `Discovery` type, never side effects of construction.

21.6. **Supply chain** per §15; `cargo deny` advisories block; Dependabot security updates are prioritized over the weekly batch.

---

## Appendix A — Configuration files

### `rust-toolchain.toml`

```toml
[toolchain]
channel    = "1.99.1"
components = ["rustfmt", "clippy", "llvm-tools-preview"]
targets    = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "armv7-unknown-linux-gnueabihf"]
profile    = "minimal"
```

### `rustfmt.toml`

```toml
style_edition = "2024"
```

### `clippy.toml`

```toml
msrv = "1.97"
avoid-breaking-exported-api = false
cognitive-complexity-threshold = 25
too-many-arguments-threshold = 7
too-many-lines-threshold = 120
large-error-threshold = 64
enum-variant-size-threshold = 128

allow-unwrap-in-tests = true
allow-expect-in-tests = true
allow-panic-in-tests = true
allow-dbg-in-tests = true
allow-print-in-tests = true

disallowed-methods = [
  { path = "std::thread::sleep",                       reason = "blocks a runtime worker; use tokio::time or a BlockingWorker" },
  { path = "tokio::sync::mpsc::unbounded_channel",     reason = "channels are bounded (CODING_STANDARDS §8.8)" },
  { path = "tokio::task::spawn",                       reason = "spawn through an owner that keeps the JoinHandle (§8.7); use JoinSet or a supervised task type" },
  { path = "std::process::exit",                       reason = "libraries do not terminate the process" },
  { path = "std::env::set_var",                        reason = "unsafe in edition 2024; unavailable" },
  { path = "alloc::string::String::from_utf8_lossy",   reason = "only via stonecrab::codec::text::decode_lossy, which marks Quality::Uncertain and keeps raw bytes" },
]

disallowed-types = [
  { path = "tokio::sync::mpsc::UnboundedSender",   reason = "channels are bounded (§8.8)" },
  { path = "tokio::sync::mpsc::UnboundedReceiver", reason = "channels are bounded (§8.8)" },
  { path = "std::sync::mpsc::Sender",              reason = "std channels have no capacity bound; use tokio::sync::mpsc with a capacity" },
]

disallowed-macros = [
  { path = "std::println",  reason = "use tracing (§14)" },
  { path = "std::print",    reason = "use tracing (§14)" },
  { path = "std::eprintln", reason = "use tracing (§14)" },
  { path = "std::eprint",   reason = "use tracing (§14)" },
]
```

### Root `Cargo.toml` (workspace excerpt)

```toml
[workspace]
resolver = "3"
members  = ["crates/stonecrab"]
exclude  = ["fuzz"]

[workspace.package]
edition      = "2024"
rust-version = "1.97"
license      = "Apache-2.0"
repository   = "https://github.com/themadbotter/stonecrab"
homepage     = "https://github.com/themadbotter/stonecrab"
authors      = ["The Mad Botter INC", "Stonecrab contributors"]
keywords     = ["plc", "modbus", "opc-ua", "industrial", "iot"]
categories   = ["network-programming", "hardware-support", "embedded"]

[workspace.dependencies]
# third-party (versions illustrative; pinned by Cargo.lock)
bytes      = { version = "1",    default-features = false }
bitvec     = { version = "1",    default-features = false, features = ["alloc"] }
thiserror  = { version = "2",    default-features = false }
tokio      = { version = "1",    default-features = false }
tokio-util = { version = "0.7",  default-features = false }
tracing    = { version = "0.1",  default-features = false }
indexmap   = { version = "2",    default-features = false }
serde      = { version = "1",    default-features = false, features = ["derive", "alloc"] }
toml       = { version = "0.9",  default-features = false, features = ["parse"] }
proptest   = { version = "1" }
insta      = { version = "1" }
criterion  = { version = "0.6", default-features = false }

[workspace.lints.rust]
unsafe_code                   = "forbid"
non_ascii_idents              = "deny"
rust_2018_idioms              = { level = "warn", priority = -1 }
rust_2024_compatibility       = { level = "warn", priority = -1 }
missing_docs                  = "warn"   # promoted to deny in published crates' lib.rs (§5.1)
missing_debug_implementations = "warn"
unreachable_pub               = "warn"
unused_qualifications         = "warn"
unused_import_braces          = "warn"
unused_lifetimes              = "warn"
single_use_lifetimes          = "warn"
trivial_casts                 = "warn"
trivial_numeric_casts         = "warn"
let_underscore_drop           = "warn"
macro_use_extern_crate        = "warn"
meta_variable_misuse          = "warn"
variant_size_differences      = "warn"

[workspace.lints.clippy]
all      = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
cargo    = { level = "warn", priority = -1 }

# denied even locally — never acceptable to run
unwrap_used                   = "deny"
expect_used                   = "deny"
panic                         = "deny"
unimplemented                 = "deny"
todo                          = "deny"
unreachable                   = "deny"
string_slice                  = "deny"
mem_forget                    = "deny"
exit                          = "deny"
dbg_macro                     = "deny"
print_stdout                  = "deny"
print_stderr                  = "deny"
use_debug                     = "deny"
allow_attributes              = "deny"
allow_attributes_without_reason = "deny"
map_err_ignore                = "deny"
mod_module_files              = "deny"

# restriction, warn; the first three are promoted to deny in lib.rs, crate-wide or on the module declarations named in §5.1
indexing_slicing              = "warn"
arithmetic_side_effects       = "warn"
as_conversions                = "warn"
integer_division              = "warn"
float_cmp_const               = "warn"
lossy_float_literal           = "warn"
missing_assert_message        = "warn"
tests_outside_test_module     = "warn"
unwrap_in_result              = "warn"
let_underscore_must_use       = "warn"
let_underscore_untyped        = "warn"
clone_on_ref_ptr              = "warn"
rc_buffer                     = "warn"
rest_pat_in_fully_bound_structs = "warn"
same_name_method              = "warn"
wildcard_enum_match_arm       = "warn"
str_to_string                 = "warn"
string_add                    = "warn"
format_push_string            = "warn"
pub_without_shorthand         = "warn"
renamed_function_params       = "warn"
large_include_file            = "warn"

# nursery, individually
future_not_send               = "warn"
missing_const_for_fn          = "warn"
redundant_pub_crate           = "warn"
use_self                      = "warn"
needless_pass_by_ref_mut      = "warn"
iter_on_single_items          = "warn"
significant_drop_in_scrutinee = "warn"
significant_drop_tightening   = "warn"
branches_sharing_code         = "warn"

# pedantic noise, allowed with reasons in §5.7
module_name_repetitions       = "allow"
must_use_candidate            = "allow"
similar_names                 = "allow"
too_many_lines                = "allow"
struct_excessive_bools        = "allow"

[workspace.lints.rustdoc]
all                       = { level = "warn", priority = -1 }
broken_intra_doc_links    = "deny"
private_intra_doc_links   = "deny"
bare_urls                 = "deny"

[profile.release]
lto             = "thin"
codegen-units   = 1
debug           = "line-tables-only"
overflow-checks = true

[profile.bench]
inherits = "release"
debug    = true
```

### Crate `Cargo.toml` (excerpt)

```toml
[package]
name         = "stonecrab"
version      = "0.2.0"
description  = "Native Rust industrial protocol library: core contracts, Modbus TCP, an in-process simulator and sensor profiles in one crate"
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true
readme       = "README.md"
include      = ["src/**", "docs/**", "README.md", "LICENSE", "NOTICE"]

[lints]
workspace = true          # the whole table; Cargo forbids per-crate additions alongside this

[features]
default   = ["sim", "modbus", "sensors"]   # `default-features = false` is the Tokio-free core
transport = ["dep:tokio"]
modbus    = ["transport", "dep:tracing"]
sim       = ["dep:tracing"]
sensors   = ["dep:serde", "dep:toml"]
testkit   = ["transport", "dep:serde", "dep:serde_json", "dep:toml", "tokio/macros"]

[package.metadata.docs.rs]
all-features = true
rustdoc-args = ["--cfg", "docsrs"]
```

### Crate `src/lib.rs` header

The only lint attributes `lib.rs` may carry (§4.1, §5.1): the crate-level promotions as inner attributes, and the per-module promotions as outer attributes on the module declarations:

```rust
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]
// §5.1 promotions, crate-wide
#![deny(missing_docs)]
#![deny(clippy::as_conversions)]

mod address_options;
// ... the core's private modules, re-exported below

// §5.1 promotions, per module (outer attributes on the declaration)
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
pub mod codec;
#[cfg(feature = "modbus")]
#[deny(
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions
)]
#[cfg_attr(docsrs, doc(cfg(feature = "modbus")))]
pub mod modbus;

pub use address_options::{AddressOptions, AddressOptionsBuilder};
```

### `deny.toml` (excerpt)

```toml
[graph]
all-features = true

[advisories]
version   = 2
yanked    = "deny"
ignore    = []            # each entry needs an issue link and an expiry

[licenses]
version = 2
allow = [
  "Apache-2.0", "Apache-2.0 WITH LLVM-exception", "MIT", "BSD-2-Clause", "BSD-3-Clause",
  "ISC", "Zlib", "Unicode-3.0", "CC0-1.0", "0BSD",
]
exceptions = [
  # { allow = ["MPL-2.0"], crate = "some-crate" },   # case by case, with ADR
]

[bans]
multiple-versions = "warn"
wildcards         = "deny"
deny = [
  { crate = "openssl",      reason = "use rustls" },
  { crate = "openssl-sys",  reason = "use rustls" },
  { crate = "async-trait",  reason = "CODING_STANDARDS §8.2" },
  { crate = "lazy_static",  reason = "use std::sync::LazyLock" },
  { crate = "once_cell",    reason = "use std::sync::OnceLock / LazyLock" },
  { crate = "log",          reason = "use tracing" },
  { crate = "failure",      reason = "unmaintained" },
  { crate = "error-chain",  reason = "unmaintained" },
  # anyhow/eyre and chrono/time/jiff are denied per-crate via `wrappers`/`deny-features` rules in the full file
]

[sources]
unknown-registry = "deny"
unknown-git      = "deny"
allow-registry   = ["https://github.com/rust-lang/crates.io-index"]
```

### `.config/nextest.toml`

```toml
[profile.default]
retries       = 0
slow-timeout  = { period = "60s", terminate-after = 2 }
fail-fast     = false

[profile.ci]
retries       = 0
slow-timeout  = { period = "60s", terminate-after = 2 }
junit         = { path = "junit.xml" }
```

### `.gitattributes`

```text
* text=auto eol=lf
*.hex   text
*.png   binary
src/generated/** linguist-generated=true -diff
```

---

## Appendix B — Pull request template

```markdown
## What

<!-- One paragraph. What changes, for whom. -->

## Why

<!-- Link the issue, the SPEC.md section, or the parity record (docs/parity/<protocol>.md#<op>). -->

## Evidence

- [ ] Unit / golden-vector / property tests added or updated: `<test names>`
- [ ] Mandatory failure cases covered (SPEC.md): `<list>` / n/a
- [ ] Contract suite passes against the simulator / n/a
- [ ] Interop job passes / n/a
- [ ] Hardware record linked: `docs/hardware/<file>` / no support-level claim changes

## Support level

<!-- If this PR changes any capability's status (planned → implemented → automated-verified → hardware-verified), say which and link the parity record diff. -->

## Dependencies

<!-- For each new or upgraded dependency: purpose, why std is insufficient, unsafe status, license, features enabled. "None" otherwise. -->

## Checklist

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean locally
- [ ] No `#[allow]`; every `#[expect]` has a `reason`
- [ ] Every new public item documented; `# Errors` / `# Cancel safety` / `# Wire effects` present where required
- [ ] No new `unsafe` (there can't be — `forbid` — but confirm no dependency added without a `DEPENDENCY_AUDIT.md` entry)
- [ ] `CHANGELOG.md` `Unreleased` updated for each affected published crate
- [ ] Generated code untouched by hand; `cargo xtask generate` produces a clean diff
- [ ] Commits are Conventional Commits with `Signed-off-by`
```

---

## Appendix C — Reviewer checklist

Reviewers read in this order and stop at the first failure:

1. **Scope.** One concern? Right crate? Nothing sideways between drivers?
2. **Safety.** Any new dependency with `unsafe`? Audit entry present? Any `build.rs`?
3. **Input handling.** Every length checked before allocation? Every index through `BoundedReader`? Every arithmetic op checked/saturating/wrapping with a reason?
4. **Failure semantics.** Partial batch → `Ok` with per-item errors? Ambiguous write → `Unknown`? No write retry? Validation failure → no I/O?
5. **Async.** Deadline on every I/O? Cancel-safety documented? No lock across `.await`? No detached spawn? Channels bounded with a policy?
6. **API.** `#[non_exhaustive]`? Newtypes? Builder for >3 fields? `#[must_use]` where dropping loses information? Feature additive?
7. **Tests.** Named as sentences? Deterministic? Adversarial cases for new parsers? Golden vectors independently authored? Regression test for the bug?
8. **Docs.** Required sections present? Doctests compile? Diagram updated if structure changed? Changelog entry?
9. **Evidence.** Support-level claim backed by a linked record? Parity record updated?
10. **Hygiene.** Conventional Commit? DCO? No `TODO` without issue? No `#[allow]`?

---

## Appendix D — ADR template

```markdown
# ADR-NNNN: <title>

- **Status:** proposed | accepted | superseded by ADR-MMMM
- **Date:** YYYY-MM-DD
- **Deciders:** <maintainers>
- **Affects:** <modules / SPEC.md section / CODING_STANDARDS rule>

## Context

<!-- The problem, the constraints, the upstream behavior if relevant. -->

## Decision

<!-- One paragraph. What we will do. -->

## Consequences

<!-- Positive, negative, and what becomes easier or harder. -->

## Alternatives considered

<!-- Each with the reason it was not chosen. -->

## Compliance

<!-- How this is enforced: lint, CI job, test, review rule. If it overrides a CODING_STANDARDS rule, quote the rule and the new wording. -->
```

---

*End of standards. The next revision of this document is due with the M0 inventory, when the first concrete dependency list and the `stonecrab::codec` API exist to check these rules against.*
