# Stonecrab status and engineering handoff

Updated: 2026-10-27.
Implementation baseline: the commit that last modified this file
(`git log -1 --format=%h -- STATUS.md`). Every alpha slice updates this file in
its own commit; the slice log below records what each one landed.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition. It is **not yet a functioning industrial protocol client**.
The repository currently contains tested, runtime-independent core data contracts, a
bounded codec crate and the Modbus address model with its text parser. There are no
connections, device reads, protocol drivers or hardware integrations.

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.1.0**, a specification-stage placeholder.
- Current development versions: facade **0.2.0**, core **0.1.0**, codec **0.1.0**,
  modbus **0.1.0**, transport **0.1.0**, all unreleased.
- The facade's minor bump records removal of the published `placeholder::status`
  API. Do not restore a compatibility shim without a real consumer requirement.
- Review policy: ADR-0005 waives the two-approval rule of CODING_STANDARDS 18.6
  while there is one maintainer. The waiver expires when a second maintainer with
  merge rights exists; until then every PR carries an Appendix C self-review.

Pushing is a manual maintainer action; `git status -sb` shows unpushed commits.
No new release was published. There is no unfinished source edit at this handoff.

## Source documents and precedence

Read these before changing contracts:

1. [`docs/SPEC.md`](docs/SPEC.md): scope, parity definition and acceptance criteria.
2. [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md): normative coding policy,
   including links to repository-specific overrides.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): target crate graph and contracts.
4. [`docs/PROJECT_BRIEF.md`](docs/PROJECT_BRIEF.md): retained original README brief.
5. [`docs/adr/`](docs/adr/): accepted implementation decisions.
6. [`docs/parity/core.md`](docs/parity/core.md) and
   [`docs/parity/modbus.md`](docs/parity/modbus.md): current scope and compatibility notes.

The source documents contain **target** APIs, diagrams, future crates and CI gates.
Their presence does not mean those capabilities exist. Some original links refer
to an old repository owner or `main`; active metadata uses the repository above.
ADR-0004 explicitly overrides the original `main` branch policy with `master`.

## Implemented

### Workspace and development infrastructure

- Resolver-3 Cargo workspace, Rust 2024, shared metadata and strict safety lints.
- `crates/stonecrab`: public facade, re-exporting core.
- `crates/stonecrab-core`: no Tokio, executor or network dependency.
- `crates/stonecrab-transport`: the only crate with Tokio (scoped features, no spawned
  tasks); TCP transport, `SystemClock` and the reconnect jitter source (ADR-0014).
- `tools/stonecrab-testkit`: unpublished deterministic fixtures, development-only:
  `FixtureAddress`, `FakeClock`, `block_on`, `block_on_with`, `poll_once`, `CountingWaker`.
- Empty default feature sets in the publishable crates; no protocol feature flags yet.
- Apache license/notices, contribution/security policy, changelog and ADRs.
- Local mise tasks, CI workflow, package allow-lists and VS Code test debugging.
- docs.rs metadata (`all-features`, `--cfg docsrs`) in both published crates; the
  facade archive no longer ships `tests/`; the stray root `src/lib.rs` is gone.
- Community files: `CODE_OF_CONDUCT.md`, `.github/PULL_REQUEST_TEMPLATE.md`,
  `.github/dependabot.yml`, `docs/hardware/TEMPLATE.md` (a template, not a record).
  `CONTRIBUTING.md` mirrors standards section 18 plus the upstream-baseline and
  fuzz-finding policies. `deny.toml` also bans `failure`, `error-chain`, `color-eyre`.
- `fuzz/` is a separate Cargo workspace with its own lock, nightly pin and deny policy
  (ADR-0009); `scripts/fuzz-smoke.sh` runs cargo-deny on it and every target for 60 s.
  Targets: `fuzz_codec_bounded_reader` (S15), `fuzz_core_connection_string` (S08),
  `fuzz_modbus_address` (S20b).

### Core data contracts

- `PlcValue` / `PlcValueType`: exact-width scalar values, including signed/unsigned
  integers, floats, Boolean, character, string, duration and explicit null.
  Integer access never silently converts counters to floating point.
- `TagId`: validated, nonempty UTF-8, at most 256 bytes, no control characters or
  edge whitespace; original identity is retained without normalization.
- `ProtocolAddress`: object-safe, driver-owned immutable address boundary with
  `Any + Debug + Send + Sync`. No real protocol address parser exists yet.
- `Tag`: shared immutable address, caller identity, positive count and type-hint
  consistency. Stable trait upcasting supports concrete-address downcasting.
- `ByteOrder` / `WordOrder`: explicit, never-inferred byte and word order carried as
  optional `Tag` fields (`with_byte_order`, `with_word_order`); additive `Ord` on
  `ErrorKind`, `Operation` and `PlcValueType` (ADR-0006). Data only; no decoder yet.
- `Capabilities`, `Operation`, `SubscriptionMode`: explicit capability metadata,
  including the distinction between native and polling subscriptions.
- `PlcError`, `ErrorKind`, `Diagnostics`, `ConnectionState`, `FailureCategory`:
  small boxed errors, original source chains and optional honest context.
  `ConnectionState` is metadata, not an implemented lifecycle state machine.
- `Deadline`: explicit monotonic-time construction and expiry checks.
- `OpFuture`, `Clock`, `CancellationToken`, `StateCell` / `StateWatch`: the boxed `Send`
  future type, the injectable clock trait (no implementation in core), a hand-written
  cancellation token with child tokens, and latest-value connection-state observation
  (ADR-0007, proposed). Primitives only: no request carries a cancel slot and no
  connection enforces a default deadline yet.
- `ConnectionUri`: the connection-string grammar `scheme(:transport)?://authority(?k=v...)`
  as validated text parts; no driver semantics, no host resolution.
- `ConnectionConfig` and builder, `Endpoint`, `Timeouts`, `ReconnectPolicy`,
  `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions`: validated connection
  configuration as data (ADR-0008); the reconnect backoff is a pure function of attempt
  and a caller-supplied sample. No connection consumes it yet.
- `ResourceLimits`: finite positive budgets; immutable after builder validation.
- `Secret<T>`: redacted Debug/Display, explicit access; no zeroization claim.
- `ProtocolStatus`: original code/name and explicitly bounded retained raw bytes;
  Debug prints byte count, not payload.
- `ItemError` / `ReadItem`: original status, source chain, caller identity,
  sampling time and absent-or-actual source timestamp.
- `ReadRequest`: immutable bounded batches; duplicate and over-budget inputs
  cause sticky builder errors before growing internal collections.
- `ReadResponse`: exact request/result correlation, original request ordering,
  per-item failures, split-batch disclosure and monotonic interval validation.
  Missing, unexpected and duplicate results are rejected.
- `WriteOutcome` / `UnknownReason`: confirmed, rejected and unknown-after-transmit
  representations. These types do not implement a write operation or retries.
- `WriteItem`, `WriteRequest`, `WriteResult`, `WriteResponse`: bounded write batches
  with exact correlation that keep unknown outcomes first-class (ADR-0010). Data only;
  nothing transmits, retries or replays.

### Tests

Unit tests, consumer integration tests, executable documentation and Send/Sync
assertions cover the implemented contracts. The testkit's `FixtureAddress` is
deliberately not a driver or a real protocol parser.

Read correlation tests cover mixed success, all-item failure, missing/duplicate/
unexpected identities, time boundaries, and all six three-item result orderings
with distinct values attached to their identities.

## Alpha slices landed

Slices follow [`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md). Each row is one commit;
gates named were run locally before that commit.

| Slice | Date | Landed | Gates |
| --- | --- | --- | --- |
| S02 | 2026-10-27 | Repository hygiene and community files; docs.rs metadata; facade archive without tests; GitHub community-profile check deferred until the next push | check, coverage, dependencies, package |
| S01 | 2026-10-27 | Modbus TCP parity record with stable row ids (77 alpha rows `planned`, 26 deferred, 21 divergence rows, UNVERIFIED list), `CORE-URI-GRAMMAR` row, manifest `[inventory.modbus_tcp]` and `[policy]`; docs only | check |
| S03 | 2026-10-27 | CI gates and ADR-0005: nextest, per-crate coverage tiers via `scripts/coverage-gate.sh`, `scripts/lint-attrs.sh` with self-test fixtures, `scripts/dco-check.sh`, jobs dco/features/machete/lint-attrs/beta/geiger/semver/scheduled, pinned actions, single-maintainer waiver; hosted CI unexercised until pushed | check, coverage, lint-attrs, dco, features, machete, dependencies, package |
| S05 | 2026-10-27 | Core `ByteOrder`/`WordOrder`, optional per-tag order fields, additive `Ord` on `ErrorKind`/`Operation`/`PlcValueType`, ADR-0006 (RawValue and calendar values deferred); facade re-exports; parity rows `CORE-BYTEORDER-MODEL`, `CORE-VALUE-ORD` | check, coverage, lint-attrs, features, machete, dependencies, package |
| S03b | 2026-10-27 | Test tooling policy ADR-0009; `fuzz/` as its own Cargo workspace (nightly-2026-10-01, cargo-fuzz 0.13.2, libfuzzer-sys 0.4.13 with the NCSA allowance, arbitrary 1.5.0, no targets yet); `scripts/fuzz-smoke.sh`, `fuzz-smoke` CI job, weekly fuzz step, `PROPTEST_CASES=256` in CI | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S15 | 2026-10-27 | New crate `stonecrab-codec` 0.1.0: `BoundedReader`, `BoundedWriter`, `ParseError` with `From<ParseError> for PlcError`, coil bit helpers; proptest 1.12 adopted (D23 check clean); first fuzz target `fuzz_codec_bounded_reader` (60 s clean); ADR-0013. Same commit series makes `ByteOrder`/`WordOrder` exhaustive (ADR-0006 amendment 7) | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S06 | 2026-10-27 | Core `OpFuture`, `Clock` trait (no implementation in core; `SystemClock` moves to transport), `CancellationToken` with child tokens and waker dedup, `StateCell`/`StateWatch` latest-value observation; ADR-0007 proposed (accepted when S11b-2 enforces it); parity rows `CORE-OPFUTURE`, `CORE-CLOCK`, `CORE-CANCEL-TOKEN`, `CORE-STATE-WATCH`; no cancel slot on requests yet | check, coverage, lint-attrs, machete, dependencies, package |
| S07 | 2026-10-27 | Core `ConnectionConfig` and builder, `Endpoint`, `Timeouts`, `ReconnectPolicy` with pure `next_delay`, `RetryPolicy` over `BTreeSet<ErrorKind>`, `AccessPolicy` (default `ReadOnly`), `Credentials`, `AddressOptions` carrier; ADR-0008; parity rows `CORE-CONFIG-SHAPE`, `CORE-RECONNECT-POLICY`, `CORE-RETRY-POLICY`, `CORE-ADDRESS-OPTIONS`; data only, unenforced | check, coverage, lint-attrs, machete, package |
| S10 | 2026-10-27 | Testkit `FakeClock` (a `Clock` implementation with hand-advanced time and ordered sleepers), `block_on`, `block_on_with` (advances to the next sleeper, fails fast when nothing waits on time), `poll_once`, `poll_once_with`, `CountingWaker`; no dependency | check, coverage, lint-attrs, machete |
| S20a | 2026-10-27 | New crate `stonecrab-modbus` 0.1.0 with address types only: `UnitId`, `ModbusArea`, `ModbusDataType` (15 alpha types; string and calendar names rejected as deferred), `AddressConvention`, `ModbusAddress` and builder (bit areas accept BOOL only; last register must fit in the address space; long-form `canonical()`); ADR-0015 draft; no parser, PDU or I/O | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S09 | 2026-10-27 | Core `WriteItem`, `WriteRequest` and builder, `WriteResult`, `WriteResponse` and builder (exact correlation, request order, unknown outcomes never collapsed); ADR-0010; parity rows `CORE-WRITE-REQUEST`, `CORE-WRITE-RESPONSE`; data only | check, coverage, lint-attrs, machete, package |
| S08 | 2026-10-27 | Core `ConnectionUri::parse` for the upstream connection-string grammar (strict subset: 4096-byte bound, no percent-encoding, no credentials in the authority, unique keys); second fuzz target `fuzz_core_connection_string` (60 s clean); proptest adopted by core; parity row `CORE-URI-GRAMMAR` implemented | check, coverage, lint-attrs, machete, dependencies, fuzz, package |
| S20b | 2026-10-27 | `ModbusAddress::parse` for the verified PLC4X grammar (long, prefixed short and convention-gated bare forms, config block, typed rejections for selections, strings, calendar types, extended registers, legacy arrays); third fuzz target `fuzz_modbus_address` (60 s clean); grammar and limit parity rows implemented with test ids | check, coverage, lint-attrs, machete, dependencies, fuzz, package |
| S16 | 2026-10-27 | New crate `stonecrab-transport` 0.1.0: `Transport` and `TransportFactory` traits with a `Deadline` on every I/O call, `FrameError`, `TcpTransport` and `TcpFactory` over Tokio with bounded send/receive and self-poisoning after a failed write, `SystemClock`, dependency-free `Jitter`; tokio 1.54 adopted with rt/net/time/sync/io-util only and `full` banned; ADR-0014; parity rows in `docs/parity/transport.md`; loopback byte-level tests; coverage gate 95% (measured 99.6%) with the real-socket deadline-expiry arms named | check, coverage, lint-attrs, features, machete, dependencies, package |
| S24a | 2026-10-27 | Modbus `Batcher::plan` with `BatchLimits`, `Chunk` and `Member`: groups by unit id and area, merges adjacent or overlapping spans only (no gap reads), splits at 125 registers or 2000 coils, keeps every `TagId` in exactly one member; property test; parity rows `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS` and the optimizer rows implemented | check, coverage, lint-attrs, machete, package |

## Verified evidence and reproduction

Development toolchain: **Rust 1.99.1**.
Declared MSRV: **1.97**; locally exercised with **1.97.1**.
Pinned tools: `cargo-llvm-cov` **0.9.1**, `cargo-deny` **0.20.2**, `cargo-nextest`
**0.9.148**, `cargo-hack` **0.6.45**, `cargo-machete` **0.9.2**, `cargo-fuzz` **0.13.2**
(fuzz workspace on `nightly-2026-10-01`).

From the repository root:

```sh
mise install
mise run check
mise run coverage
mise run lint-attrs
mise run features
mise run machete
mise run dependencies
mise run package
mise run fuzz
mise run dco -- origin/master..HEAD
```

Most recent local results for the implementation baseline:

- Formatting, strict Clippy, nextest, `cargo test` with doctests, documentation with
  warnings denied, no-default-features, lint-attrs, feature powerset and unused-
  dependency checks passed.
- Per-crate line coverage (ADR-0005): `stonecrab-core` 2704/2704 lines (gate 100%),
  `stonecrab-codec` 181/181 lines (gate 100%), `stonecrab-modbus` 585/585 lines (gate
  100%), `stonecrab-transport` 457/459 lines (gate 95%; the uncovered lines are the
  real-socket deadline-expiry and resolver-failure arms named in ADR-0014),
  `stonecrab-testkit` 549/551 lines (gate 90%), `stonecrab` n/a (no executable lines). Error paths are not excluded. Region
  coverage is reported in `target/coverage-summary.txt` and not gated; do not describe
  it as branch coverage.
- Workspace tests and doctests passed on Rust 1.97.1.
- Cross-compilation checks passed for `aarch64-unknown-linux-gnu` and
  `armv7-unknown-linux-gnueabihf`. These are compile checks, not device tests.
- Dependency advisories, licenses, bans and registry-source checks passed.
  Unused license-allowance warnings are currently emitted by cargo-deny.
- Both publishable crate archives built and matched committed package allow-lists.

Additional commands used locally:

```sh
mise exec rust@1.97.1 -- rustup run 1.97.1 cargo test --locked --workspace --all-features
mise exec -- cargo check --locked --workspace --all-features --target aarch64-unknown-linux-gnu
mise exec -- cargo check --locked --workspace --all-features --target armv7-unknown-linux-gnueabihf
```

Hosted CI for `e7d887e` **completed successfully**:
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40442677439
That run covered the earlier test, quality, cross-target, dependency and package
jobs. The S03/S03b workflow (nextest, coverage gate, dco, features, machete,
lint-attrs, beta, geiger, semver, fuzz-smoke, scheduled) has not run on GitHub yet; the first push must
confirm the pinned action commit, the install-action tool names and the dco job on
both event types. Nothing establishes the fuzz, mutation, soak, interoperability or
hardware gates yet.

## Important implementation and tooling details

- The actual API is `ReadRequest::builder(limits)`, not the brief's no-argument
  builder. It takes an absolute `.deadline(...)`; `.deadline_after(...)` does not
  exist. `Deadline::after(now, duration)` takes the clock reading explicitly.
- An absent request deadline requires a future connection to supply a bounded
  default. It does not authorize unbounded I/O.
- Requests and responses are data only. Passing their tests is not evidence of
  transport cancellation, reconnect, batching on the wire or protocol parity.
- Partial and all-item failures remain valid response data. Do not collapse them
  into a batch error or invent a device timestamp.
- Never turn an unknown write outcome into a rejection or automatically replay it.
- Address methods must return stable, credential-free metadata. Do not add an
  `as_any` shim; stable trait upcasting is already used.
- Core's testkit dependency is development-only. The path-only dependency policy
  exception is documented in ADR-0003; registry wildcard versions remain denied.
- Core integration tests using the unpublished testkit are workspace-only and not
  shipped in the core archive. Cargo emits an expected ignored-test warning while
  packaging. Packaged core unit tests and documentation remain self-contained.
- `mise run package` uses a **fresh Cargo cache**. Repeatedly verifying changed,
  unpublished sibling crates with the same version in a reused cache previously
  selected an older core archive. Do not remove the isolation as a cosmetic change.
- Update `packaging/stonecrab-core.txt` or `packaging/stonecrab.txt` when shipped
  files change, and review the resulting archive contents.
- New dependencies require a rationale and the applicable audit/ADR entry.
  Runtime dependencies are `thiserror`, `bytes` and, in the transport crate only,
  `tokio` with scoped features; dev-only `proptest`.

## Baseline and incomplete M0 work

[`upstream/manifest.toml`](upstream/manifest.toml) pins Apache PLC4X **v1.0.0**:

- Annotated tag object: `32a524591841ab56d26b032914ce06918148fbb6`.
- Peeled implementation commit: `ea229918f5d07700e04d7d69ab1645da679cb700`.
- Tag/commit relation verified against Git and the GitHub ref API.

**M0 is not complete.** The Modbus TCP subset is inventoried: `docs/parity/modbus.md`
carries stable row ids with planned test ids per row, a divergence table with
references, and an UNVERIFIED list; `upstream/manifest.toml` records the inventoried
files and line ranges under `[inventory.modbus_tcp]`. Still required, none gating the
alpha:

- Protocol × transport × operation inventory beyond Modbus TCP.
- Capabilities present in non-Java implementations.
- Source-release signature verification.
- The open items in the manifest `[pending]` table (RTU/ASCII connections, generated
  TIME/DATE behavior, test-suite step formats, Modbus V1.1b section-number check).
- Every divergence row marked "upstream issue to be filed before S32" becomes a URL
  or is dropped in S32.

Verified Modbus pitfalls are recorded in `docs/parity/modbus.md`: numeric address
groups are decimal; `x` in forms such as `4x00016` is an area marker, not hexadecimal.
Upstream uses `holding-register:16:INT`, IEC-style type names and
`default-unit-identifier`, not the original brief's provisional syntax. The
protocol-address offset is one. Modbus TCP's upstream subscriptions are polling,
not native subscriptions. The Rust address parser implements the grammar rows; no
operation row is implemented yet.

## Not implemented or not yet evidenced

- Driver registry, driver/connection traits and operational connection lifecycle.
- Access-policy enforcement and actual read/write I/O (typed configuration exists as
  data only).
- Enforcement of cancellation and default deadlines by a connection (`SystemClock` and
  `FakeClock` exist; `RealtimeClock` lands with the simulator).
- Simulator driver, scripted acquisition failures and the common driver suite (fake
  time exists in the testkit).
- Byte/raw/recursive/calendar `PlcValue` variants (deferred by ADR-0006); the byte and
  word order types exist as data only, with no decoder consuming them.
- Browse, discovery and subscriptions (write batches exist as data).
- Modbus PDU codec, framing and connection, sensor profiles and board adapters (the
  TCP transport, the codec crate, the Modbus address parser and the read batcher exist;
  nothing reads a device yet).
- OPC UA, EtherNet/IP/Logix, S7 and remaining upstream protocols.
- Code generation or retained upstream fixture corpus.
- Parser fuzzing (the `fuzz-smoke` job exists but has no targets), mutation testing,
  soak/load/benchmark evidence, branch-coverage gates, and the `interop`, `regen` and
  blocking `semver` jobs from the source standards.
- Hardware verification, source-release signatures and a reviewed full parity matrix.
- Verified private vulnerability-reporting availability; this remains a release prerequisite.

## Recommended continuation

The slice-by-slice plan to the first alpha release, with the decisions and
maintainer prerequisites it needs, is in [`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md).

1. Review the source documents, ADRs and actual exported types; rerun the local
   gates and check hosted CI before changing code.
2. Complete M0 source inventory and test traceability. Do not equate protocol
   definitions or generated PDU variants with implemented upstream driver operations.
3. Settle the remaining core decisions needed for an operational simulator:
   connection configuration, deadline/cancellation ownership, state watching,
   missing value representations and complete write-batch contracts.
4. Build the next working vertical slice: explicit registry plus simulator and
   shared contract suite, using object-safe Send boxed futures and deterministic
   test time. Keep core independent of Tokio.
5. Follow with bounded codecs/TCP transport and native Modbus TCP acquisition,
   then sensor profiles/quality, RTU, optional bench adapters, OPC UA and PLC
   drivers in the staged order from `docs/SPEC.md`.

Keep changes reviewable, commit tested slices frequently, and maintain the existing
100% line-coverage gate. No first-party unsafe, panics on input, unbounded I/O,
silent sample loss, inferred byte order or automatic replay of ambiguous writes.
Do not introduce empty driver scaffolds or mark planned capabilities supported.

The maintainer normally pushes manually; push only when explicitly requested.
Do not publish another crate version or operate hardware without separate approval.
Never place credentials in files, logs, commit messages or handoff documents.
