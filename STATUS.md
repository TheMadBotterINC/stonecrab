# Stonecrab status and engineering handoff

Updated: 2026-10-27.
Implementation baseline: the commit that last modified this file
(`git log -1 --format=%h -- STATUS.md`). Every alpha slice updates this file in
its own commit; the slice log below records what each one landed.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition. It is **not yet a functioning industrial protocol client**.
The repository contains the runtime-independent core contracts and driver boundary, a
simulator driver that reads and writes an in-process device through `DriverRegistry`,
a bounded codec crate, the TCP transport with framing, the Modbus address model with its
parser and batcher, and the validated sensor profile loader. There is no protocol
driver that reads a real device yet, and no hardware integration.

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.1.0**, a specification-stage placeholder.
- Current development versions: facade **0.2.0**, core **0.1.0**, codec **0.1.0**,
  modbus **0.1.0**, transport **0.1.0**, driver-sim **0.1.0**, sensors **0.1.0**, all
  unreleased.
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
  tasks); TCP transport, framing layer with resync, the reconnecting `ConnectionGuard`,
  `SystemClock` and the reconnect jitter source (ADR-0014).
- `tools/stonecrab-testkit`: unpublished deterministic fixtures, development-only:
  `FixtureAddress`, `FakeClock`, `block_on`, `block_on_with`, `poll_once`, `CountingWaker`;
  behind its `net` feature `Script`, `ScriptedPeer`, `FakeTransport`,
  `FakeTransportFactory` and `LengthPrefixedFramer` (the sanctioned doubles of
  CODING_STANDARDS 13.5); behind its `fixtures` feature the golden-vector loader and
  `InteropMap`; and the `ContractSuite` every driver runs.
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
  `fuzz_modbus_address` (S20b), `fuzz_sensors_profile_toml` (S28a), `fuzz_modbus_pdu`
  (S21). `fuzz/deny.toml`
  must mirror every root `deny.toml` change; the smoke script refuses to run otherwise.

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
  (ADR-0007, accepted). Both request types carry an optional cancel slot.
- `Driver`, `Connection`, `DriverRegistry`: the object-safe driver boundary (ADR-0011).
  The registry extracts the scheme with `ConnectionUri` and hands the text to the
  driver; an unknown scheme is `Unsupported(Connect)`.
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
| S17 | 2026-10-27 | Testkit `net` feature: `Script` and `Step` (expect, reply, fragmented, coalesced, delay, close-at, truncate, corrupt, wrong transaction id, late reply, responder), `ScriptedPeer` (the one sanctioned `tokio::spawn`, joined on close and aborted on drop), `FakeTransport` and `FakeTransportFactory` on `FakeClock` with connect-timeout and refusal plans; S16 loopback tests moved onto the peer where a step expresses them | check, coverage, lint-attrs, machete, dependencies |
| S11b-1 | 2026-10-27 | New crate `stonecrab-driver-sim` 0.1.0 with the simulator data model only: `RealtimeClock`, `SimAddress` grammar (`STATE|RANDOM|SINK/name:type`), `SimDevice` and `SimTag` with ramps, read counters and a seeded RANDOM source; `tracing` 0.1 adopted workspace-wide (std only; `attributes` deferred, see ADR-0012) with the `once_cell` wrapper via `tracing-core` in deny.toml; ADR-0012 proposed; ADR-0005 addendum on wildcard arms; `docs/parity/sim.md`; no `Driver` yet | check, coverage, lint-attrs, machete, dependencies |
| S28a | 2026-10-27 | New crate `stonecrab-sensors` 0.1.0 with the profile schema (version 1, unknown fields rejected) and the `toml` feature loader with a size bound; `serde` 1 and `toml` 0.9 adopted (D23 check clean; `toml_parser` pinned in the lock to keep one `winnow`); fourth fuzz target `fuzz_sensors_profile_toml` (60 s clean); ADR-0016; `docs/parity/sensors.md`; validation and the measurement pipeline are S28b/S29 | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S28b | 2026-10-27 | Sensors `SensorProfile::validate` collecting every error (duplicate names, unknown or empty unit, missing byte or word order for wide Modbus types, overlapping status bits, count above one, invalid scale steps, unknown quality references, unknown or missing address convention) and a fixed 19-entry UCUM-style unit table; eleven new `ProfileError` variants; parity row `SENSORS-VALIDATION` implemented | check, coverage, lint-attrs, machete, package |
| S18 | 2026-10-27 | Transport `Framer` trait and `FramedTransport` (bounded receive buffer, `next_frame`, `send_frame`, `resync` that discards until a boundary and fails `Disconnected` at its deadline); testkit `LengthPrefixedFramer`; parity rows `TRANSPORT-FRAMING`, `TRANSPORT-RESYNC` implemented | check, coverage, lint-attrs, machete, package |
| S04a (part) | 2026-10-27 | Testkit `fixtures` feature: golden-vector loader (`load_dir`, `load_hex`, `PROVENANCE.toml` schema enforced: every `.hex` listed, every listed vector present), `InteropMap` over the first-party `fixtures/modbus/interop/register-map.toml` authored from the `MB-INTEROP-*` rows; `serde_json` adopted (D23 clean). The upstream vector copies and the NOTICE update wait for prerequisite P4 | check, coverage, lint-attrs, machete, dependencies, package |
| S11a + S11b-2 | 2026-10-27 | Core `Driver`, `Connection` and `DriverRegistry` traits with cancel slots on both request types (ADR-0011); simulator `SimDriver` and `SimConnection` (scheme `sim`, `sim://<device>?latency-ms=&seed=`, lifecycle through `StateCell`, single in-flight slot with `Backpressure`, latency raced against token and deadline, default-deadline rule, bounded read retry, writes never retried, diagnostics and tracing spans); ADR-0007 and ADR-0012 accepted; parity rows `CORE-DRIVER-TRAIT`, `CORE-CONNECTION-TRAIT`, `CORE-REGISTRY`, `SIM-DRIVER`, `SIM-CONNECTION`; one commit because every commit must pass the 100% core gate | check, coverage, lint-attrs, machete, dependencies, package |
| S12 | 2026-10-27 | Simulator `FailureStep` and `FailureScript` on the shared device (delay, timeout after transmit, disconnect into `Failed`, device exception, malformed, partial failure, backpressure, unknown after transmit); one step per operation after every pre-I/O check; each retry attempt pops its own step and a read without an explicit deadline gets a fresh default deadline per attempt; parity row `SIM-FAILURE-SCRIPT` implemented | check, coverage, lint-attrs, machete, package |
| S19 | 2026-10-27 | Transport `ConnectionGuard` over `FramedTransport`: lifecycle through core `ConnectionState` with private idle, in-flight and resyncing phases, request-driven reconnect with capped jittered backoff and `Failed` on exhaustion, one in-flight slot plus an admission queue bounded by `max_pending_requests` (`Backpressure` beyond it), every wait raced against close, token and deadline, a timeout after flush resyncs before the next send, `GuardConfig`, `GuardStats`; parity rows `TRANSPORT-GUARD-LIFECYCLE`, `TRANSPORT-RECONNECT`, `TRANSPORT-QUEUE` implemented | check, coverage, lint-attrs, machete, package |
| S21 | 2026-10-27 | Modbus PDU codec: `FunctionCode`, `ExceptionCode`, `ModbusPdu` (read requests FC01-04, bit and register responses, exceptions), `encode_request`, `decode_request`, `decode_response` with quantity, byte-count and size bounds checked before allocation, `encode_response`; 59 golden vectors under `fixtures/modbus/tcp` authored from the Modbus Application Protocol V1.1b or adversarial, loaded through the testkit with independently written expected files; fifth fuzz target `fuzz_modbus_pdu`; `MB-EXC-*` rows implemented. The upstream vectors wait for prerequisite P4 | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S13 | 2026-10-27 | Testkit `ContractSuite` (23 cases named as sentences with their SPEC coverage; skips are named, never silent; driver failures become report entries) with `ContractFixture` hooks and `ContractReport`; run against the simulator: 22 passed, 0 failed, 1 named skip (the unsupported-write case, covered by the testkit's always-unsupported double); parity row `SIM-CONTRACT-SUITE` | check, coverage, lint-attrs, machete |

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
- Per-crate line coverage (ADR-0005): `stonecrab-core` 3514/3514 lines (gate 100%),
  `stonecrab-codec` 181/181 (gate 100%), `stonecrab-modbus` 755/755 (gate 100%),
  `stonecrab-driver-sim` 1718/1718 (gate 100%), `stonecrab-sensors` 626/626 (gate
  100%), `stonecrab-transport` 613/615 (gate 95%; the uncovered lines are the
  real-socket deadline-expiry and resolver-failure arms named in ADR-0014),
  `stonecrab-testkit` 3102/3118 (gate 90%), `stonecrab` n/a (no executable lines).
  Error paths are not excluded. Region coverage is reported in
  `target/coverage-summary.txt` and not gated; do not describe it as branch coverage.
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

Hosted CI for `22b379c` **completed successfully** with the full S03/S03b workflow:
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40491102565
Green jobs: the six test cells (ubuntu, macOS, Windows on 1.97.1 and 1.99.1), quality
with the per-crate coverage gate, cross, dependencies, package, dco, features, machete,
lint-attrs, beta, geiger and fuzz-smoke (four targets). The report-only `semver` job
fails until the sibling crates exist on crates.io (no baseline) and does not fail the
run. Two platform fixes were needed on the first hosted runs: the prebuilt cargo-fuzz
binary defaults to a musl target, so the smoke script passes the host triple; and the
drop-mid-write transport test is ignored on Windows, where loopback accepts a 32 MiB
write at once. Nothing establishes the interop, mutation, soak or hardware gates yet.

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
  Runtime dependencies are `thiserror`, `bytes`, `tracing` (std only; `once_cell`
  reaches the graph through `tracing-core` under a recorded deny.toml wrapper), `serde`
  and `toml` in the sensors crate, and `tokio` with scoped features in the transport
  crate only; dev-only `proptest`. `toml_parser` is pinned in `Cargo.lock` so that only
  one `winnow` resolves; a plain `cargo update` must keep that pin or allow the
  duplicate.

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

- A protocol session over the connection guard (the guard itself runs the lifecycle,
  reconnect and queue over any `Transport`).
- Actual read/write I/O against a device (the simulator enforces the access policy and
  serves in-process reads and writes).
- A Modbus session using the guard's cancellation and deadline enforcement (the guard
  and the simulator connection both enforce them).
- A protocol driver that passes the contract suite over a real transport (the
  simulator passes it).
- Byte/raw/recursive/calendar `PlcValue` variants (deferred by ADR-0006); the byte and
  word order types exist as data only, with no decoder consuming them.
- Browse, discovery and subscriptions (write batches exist as data).
- MBAP framing, the Modbus session and connection, the sensor measurement pipeline,
  board adapters (the TCP transport with framing and the connection guard, the codec
  crate, the Modbus address parser, batcher and PDU codec, and the validated profile
  loader exist; nothing reads a device yet).
- OPC UA, EtherNet/IP/Logix, S7 and remaining upstream protocols.
- Code generation; the retained upstream fixture corpus (blocked on maintainer
  prerequisite P4, approval to redistribute the Apache-2.0 PLC4X vectors). The
  first-party interop register map, the spec-authored and adversarial PDU vectors and
  the loader exist under `fixtures/modbus/`.
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
