# Stonecrab status and engineering handoff

Updated: 2026-10-28.
Implementation baseline: the commit that last modified this file
(`git log -1 --format=%h -- STATUS.md`). Every alpha slice updates this file in
its own commit; the slice log below records what each one landed.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition. The repository is at the **alpha baseline**: the alpha defined in
[`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md) is built, its definition of done is met, the
release evidence and process (slice S32, ADR-0017, `docs/evidence/alpha.md`) are recorded,
and the code ships as **one crate** (slice S33, ADR-0018). The alpha, `stonecrab` 0.2.0,
is published on crates.io. It is not yet tagged (no `v0.2.0` tag, no GitHub release) and
nothing is posted upstream: those steps are the maintainer's (see "Recommended
continuation" and [`docs/POST_ALPHA_PLAN.md`](docs/POST_ALPHA_PLAN.md)).

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.2.0**, the alpha, on 2026-10-27 from commit
  `640665a` after the maintainer's explicit approval (hosted run 40612938390 green);
  `stonecrab` 0.1.0, the specification-stage placeholder, stays listed as an old version.
- Alpha version: **`stonecrab` 0.2.0**, one crate. The former sibling crates
  (core, codec, transport, driver-sim, modbus, sensors) and the test kit are modules of it,
  and there are no other packages. The minor bump records removal of the published
  `placeholder::status` API; do not restore a compatibility shim without a real consumer
  requirement. A plain pre-1.0 version is used rather than `-alpha.N` (the published 0.1.0
  placeholder would win resolution over a pre-release).
- **The S33 decision (ADR-0018, the maintainer's, 2026-10-27):** Stonecrab ships as one
  crate that is simple to explain, one package, one version, one line to add it
  (`cargo add stonecrab`). Default features are `sim`, `modbus` and `sensors`;
  `default-features = false` is the Tokio-free core; the test kit is the off-by-default
  `testkit` feature because a separate test crate would be a dev-dependency cycle. It
  supersedes plan decision D1 and the crate graph of ADR-0001, ADR-0011 and ADR-0014, and
  amends ADR-0017's publish, tag and yank mechanics (one publish, one tag `v0.2.0`).
- Review policy: ADR-0005 waives the two-approval rule of CODING_STANDARDS 18.6 while there
  is one maintainer (the single-maintainer waiver, plan prerequisite P5, done). The waiver
  expires when a second maintainer with merge rights exists; until then every PR carries an
  Appendix C self-review.
- Landed slices are pushed to `origin/master` and the hosted CI run is checked. Publishing
  the crate, pushing a tag and operating hardware need separate maintainer approval. There
  is no unfinished source edit at this handoff.

## Source documents and precedence

Read these before changing contracts:

1. [`docs/SPEC.md`](docs/SPEC.md): scope, parity definition and acceptance criteria.
2. [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md): normative coding policy,
   including links to repository-specific overrides.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): the module map and contracts. Its
   diagrams are the original targets; each shipped section has a "Shipped at alpha" note
   that wins where it differs, and what is not shipped is marked deferred with its owner.
4. [`docs/PROJECT_BRIEF.md`](docs/PROJECT_BRIEF.md): retained original README brief.
5. [`docs/adr/`](docs/adr/): accepted implementation decisions (ADR-0001 to ADR-0018;
   ADR-0018 supersedes parts of ADR-0001, 0009, 0011, 0014 and 0017 and amends 0003, 0005,
   0012, 0013, 0015 and 0016, each with a note at the top of its status).
6. [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md): the support-level matrix, data types,
   configuration keys, tested peers, platforms and upstream divergences.
7. [`docs/parity/`](docs/parity/): `modbus.md`, `core.md`, `sim.md`, `transport.md` and
   `sensors.md`, the row-by-row compatibility records with test ids.
8. [`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md): the plan and decisions D1 to D23 the slices
   follow (the plan keeps its original seven-crate wording as history; S33 is the last
   slice and D1 carries the superseded note).

Some original links refer to an old repository owner or `main`; active metadata uses the
repository above. ADR-0004 explicitly overrides the original `main` branch policy with
`master`.

## What the alpha is

The alpha is the first pre-release in which an application can, through
`DriverRegistry` -> `dyn Driver` -> `dyn Connection`, connect to a Modbus TCP server by the
upstream connection-string grammar (`modbus-tcp://host:502?default-unit-identifier=1`),
read coils, discrete inputs, holding registers and input registers as the 15 numeric and
bit types in a correlated, request-ordered batch with `split_into` disclosure, observe
`ConnectionState` through a `StateWatch` including request-driven reconnect with capped
jittered backoff, and cancel or time out any operation with documented wire consequences.
It is one crate: `cargo add stonecrab` gives all of it, and `default-features = false`
leaves the Tokio-free core.

**Definition of done: met.** `crates/stonecrab/examples/profile_read.rs` loads a synthetic
TOML profile that declares its address convention, builds the tags through
`Driver::parse_address(text, &AddressOptions)`, reads, and logs name, value, unit,
quality, origin, raw value and `sampled_at` for every measurement through `tracing`, then
exits with `ExitCode`. It runs green against the simulator (`sim://bench`; CI job
`examples`, `mise run examples`) and against pymodbus 3.8.6 (CI job `interop`, `mise run
interop`, the `profiles` step). One deviation from the plan's wording: the example reads
with a 2 second deadline (`READ_DEADLINE`), not 500 ms.

Support levels at the alpha (full matrix in `docs/COMPATIBILITY.md`): Modbus TCP connect,
close and reads FC01 to FC04 are `interoperability-tested`; reconnect, retry,
cancellation, timeout, late-reply discard and resync are `simulator-tested`; the simulator
and the synthetic `sim` profiles are `simulator-tested`; the synthetic `modbus-tcp`
profiles are `interoperability-tested`; Linux aarch64 and armv7 are `build-tested`
(compile checks only); nothing is `hardware-verified`.

**What it is not.** It does not write (the Modbus driver reports `write: false` and sends
nothing), subscribe, browse, discover or ping; it has no Modbus RTU or ASCII, UDP or TLS,
no extended registers, no string or calendar types, no array selections, no gap-filling
read optimizer, no background reconnect task and no polling loop. It has never run
against a physical device. Upstream compatibility is claimed only for the rows of
`docs/parity/modbus.md` that say so.

## Implemented

One crate, `crates/stonecrab`, at the shipped shapes: the infrastructure around it, then
its modules and features. The parity rows behind each item are in `docs/parity/`; ADR
numbers are in `docs/adr/`. Slice names in this section are the history of when each part
landed; the table of landed slices below keeps the crate names the slices used then.

### One crate and development infrastructure

- Resolver-3 Cargo workspace with one member, Rust 2024, shared metadata and strict
  safety lints. `crates/stonecrab` is the only package and the only published crate.
- Features (ADR-0018): `default = ["sim", "modbus", "sensors"]`; `full` (the same set,
  spelled out; the `profile_read` example requires it); `transport` (Tokio, the
  only way Tokio enters the crate); `modbus` (turns on `transport`, adds `tracing`);
  `sim` (`tracing`); `sensors` (`serde`, `toml`); `testkit` (turns on `transport`, adds
  `serde`, `serde_json`, `toml` and Tokio `macros`; test tooling, not a runtime feature).
  The former `tcp` and `toml` features are gone. docs.rs metadata (`all-features`,
  `--cfg docsrs`). The archive ships `src/`, `docs/`, the README and the license files
  (102 files, `packaging/stonecrab.txt`); no tests, fixtures or test data.
- Examples `sim_read`, `modbus_tcp_read` and `profile_read` declare `required-features`,
  and so does every integration test file whose `#![cfg(...)]` names a feature (46
  `[[test]]` entries in `crates/stonecrab/Cargo.toml`), so a plain `cargo test` skips a
  file it cannot build instead of compiling an empty test crate. Cargo 1.99.1 prints no
  notice for the skipped files in `cargo test`; naming one with `--test` reports the
  missing feature.
- Local mise tasks that run the commands of the CI job or step of the same name
  (`lint-attrs`, `features`, `machete`, `dependencies`, `examples`, `interop`, `parity`,
  `package` and `dco`; `check`, `coverage` and `fuzz` map to `quality`/`test`, a `quality`
  step and `fuzz-smoke`, see the header of `mise.toml`), plus `mutants` and `sbom`.
  Scripts: `scripts/coverage-gate.sh` (per-module thresholds by path prefix of
  `crates/stonecrab/src`), `lint-attrs.sh` (the crate-level and per-module promotion
  tables), `dco-check.sh`, `fuzz-smoke.sh`, `parity-check.sh`, `mutants.sh`, `sbom.sh` and
  `fuzz-evidence.sh` (ADR-0005, its S31 addendum and ADR-0017); `lint-attrs.sh` and
  `parity-check.sh` have `--self-test` fixture trees under `scripts/testdata/`. The one
  package allow-list is `packaging/stonecrab.txt`.
- Community files: `CODE_OF_CONDUCT.md`, `.github/PULL_REQUEST_TEMPLATE.md`,
  `.github/dependabot.yml`, `docs/hardware/TEMPLATE.md` (a template, not a record).
  `CONTRIBUTING.md` mirrors standards section 18 plus the upstream-baseline and
  fuzz-finding policies. `deny.toml` bans, among others, `once_cell` and `lazy_static`
  (except the recorded `wrappers` entries `tracing-core` and `sharded-slab`), `log`,
  `chrono`, `time`, `jiff`, `anyhow`, `failure`, `error-chain`, `color-eyre`, OpenSSL and
  `tokio`'s `full` feature.
- `fuzz/` is a separate Cargo workspace with its own lock, nightly pin and deny policy
  (ADR-0009); `scripts/fuzz-smoke.sh` runs cargo-deny on it and every target for 60 s.
  Targets: `fuzz_core_connection_string`, `fuzz_codec_bounded_reader`,
  `fuzz_modbus_address`, `fuzz_modbus_pdu`, `fuzz_modbus_mbap`,
  `fuzz_modbus_connection_string` and `fuzz_sensors_profile_toml` (their names keep the
  former crate prefixes; they import `stonecrab::...`). `fuzz/deny.toml` must mirror every
  root `deny.toml` change; the smoke script refuses to run otherwise.
- `interop/pymodbus/`: the first-party pymodbus 3.8.6 peer (Dockerfile pinned by digest,
  `server.py`, `interop.sh`) behind the CI job `interop` and `mise run interop`
  (ADR-0015 decision 16).
- `profiles/synthetic/`: six synthetic sensor profiles (three for the pymodbus map under
  `modbus-tcp/`, three simulator twins under `sim/`) with `profiles/README.md`;
  repository data, not shipped in the package.
- `fixtures/modbus/`: first-party golden vectors (`tcp/`: PDU, ADU and typed-decode
  vectors authored from the Modbus Application Protocol V1.1b or built to fail), the
  interop register map and `PROVENANCE.toml`. Since S04a (P4 approved 2026-10-28),
  `upstream/tcp/` retains four Apache PLC4X test files verbatim with NOTICE attribution,
  and the six frames of the parser suite are `tcp/adu_upstream-*` vectors; since S04b the
  32 read exchanges of the driver suite and the two all-datatypes suites are 64 more,
  serialized from the XML models by a test-only adapter (`modbus_driver_suite_adapter.rs`).
- `upstream/manifest.toml`: the pinned Apache PLC4X baseline and the Modbus TCP inventory.
- Release process and evidence (ADR-0017 as amended): `release.toml` (cargo-release with
  publish, push and tag off; tag name `v{{version}}`), `mutants.toml`, the
  `release-evidence` dispatch workflow and `docs/evidence/`.

### Core (crate root), always built

No Tokio, executor or network dependency.

- Values and identity: `PlcValue` / `PlcValueType` (Null, Bool, I8 to I64, U8 to U64,
  F32, F64, Char, String, Duration; integer access never silently converts counters to
  floating point), `TagId`, `ProtocolAddress` (object safe, `Any + Debug + Send + Sync`),
  `Tag` (shared immutable address, optional type hint, positive count, optional
  `ByteOrder` / `WordOrder`; ADR-0006). Bytes, raw, list, struct and calendar values are
  deferred.
- Errors and state: `PlcError`, `ErrorKind` (with additive `Ord`), `Diagnostics`,
  `ConnectionState`, `FailureCategory`, `Capabilities`, `Operation`, `SubscriptionMode`,
  `ResourceLimits`, `Secret<T>` (redacted, no zeroization claim), `Deadline`.
- Time and cancellation: `OpFuture`, the `Clock` trait (no implementation in the core),
  `CancellationToken` with child tokens, `StateCell` / `StateWatch` (ADR-0007).
- Requests and responses: `ReadRequest` and builder (limits-bound, optional cancel slot),
  `ReadResponse` (exact correlation, request order, `split_into`), `ReadItem`, `ItemError`,
  `ProtocolStatus` (bounded raw bytes), `WriteItem`, `WriteRequest`, `WriteResult`,
  `WriteResponse` and `WriteOutcome` with first-class `Unknown` outcomes (ADR-0003,
  ADR-0010).
- Driver boundary (ADR-0011): `Driver`, `Connection` (the narrow set `state`,
  `watch_state`, `capabilities`, `limits`, `read`, `write`, `close`) and `DriverRegistry`
  (explicit registration, unknown scheme is `Unsupported(Connect)`).
- Configuration (ADR-0008): `ConnectionUri` (the connection-string grammar),
  `ConnectionConfig` and builder, `Endpoint` (`Tcp`, `Custom`), `Timeouts` (`idle`
  recorded, unenforced), `ReconnectPolicy` (pure `next_delay(attempt, sample)`),
  `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions` (the second argument of
  `Driver::parse_address`) and the `DriverOptions` slot for options only one driver
  understands.
- Subscriptions (ADR-0020, S54): `SubscribeRequest` with one `SubscriptionKind`
  (`Cyclic(interval)`, `ChangeOfState`, `Event`) and a required `QueuePolicy`
  (`Backpressure`, `CoalesceLatest`, `DropOldest`), the `Subscription` handle and event
  stream with `LossCounters`, the producer side `SubscriptionSender`, a queue bounded by
  `ResourceLimits::max_queued_events` (G10), and a provided `Connection::subscribe` that
  returns `Unsupported(Subscribe)` until a driver overrides it (polling comes in S55).

### `stonecrab::codec`, always built

`BoundedReader`, `BoundedWriter`, `ParseError` with `From<ParseError> for PlcError`, coil
bit helpers (ADR-0013). Lengths are checked before allocation; nothing panics on input.
Checksums and nesting limits are deferred (M2).

### `stonecrab::transport`, feature `transport`

The only module with Tokio, apart from the test kit (scoped features, no spawned task; ADR-0014): `Transport` and
`TransportFactory` (a `Deadline` on every call, `shutdown(Deadline)`), `TcpTransport` and
`TcpFactory`, `FrameError` (transport-local, plan D22), `Framer` and
`FramedTransport` (`next_frame`, `send_frame`, `resync(Deadline)`), `ConnectionGuard` with
`GuardConfig` and `GuardStats` (lifecycle through core `ConnectionState`, request-driven
reconnect with capped jittered backoff, one in-flight slot with a bounded admission queue,
resync after a timeout, `close` bounded by `Timeouts.request`), `SystemClock` and `Jitter`.
Behind the off-by-default `serial` feature (S58): `SerialFactory` and `SerialTransport` on
the `serial2` crate, one blocking worker thread per port talking to async callers over a
bounded channel, every call bounded by its deadline, `recv` cancel-safe through a
worker-side stash; `modbus` does not enable it yet (RTU is S59).

### `stonecrab::sim`, feature `sim`

Tokio-free simulator (ADR-0012): `SimDriver` (scheme `sim`,
`sim://<device>?latency-ms=<n>&seed=<u64>`), `SimConnection`, `SimDevice`, `SimTag`,
`SimAddress` (`STATE|RANDOM|SINK/<name>:<type>`), `FailureScript` and `FailureStep`,
`RealtimeClock`. It reads and writes in-process devices, enforces the access policy, allows
one operation in flight, applies bounded read retry and never retries a write.

### `stonecrab::modbus`, feature `modbus`

Modbus TCP, reads only (ADR-0015; hand-written, nothing generated): the address types and
grammar (`ModbusAddress::parse`, the explicit-convention rule for bare `40001` forms, the
`{unit-id, byte-order}` block), the PDU codec for function codes 01 to 04, the write PDUs
upstream emits (FC06, FC15 and FC16, with `ModbusPdu::write_request` choosing among them;
S45, not yet on the wire) and the exception PDU, MBAP framing (`MbapFramer`), `TypedDecoder` for the 15 types under the four
byte-order modes, `Batcher` (adjacent or overlapping merge only, 125 registers and 2000
bits per chunk), `ModbusSession` (reply accepted only when transaction id, unit id and
function code match), `ModbusConnection` and `ConnectionDefaults`, and `ModbusTcpDriver`
with `ModbusOptions` (the six upstream parameters with upstream defaults plus
`address-convention`; unknown keys are `InvalidConfig`). `write` (S46) is refused unless
the connection string says `access=read-write`, then sends one request per item (FC15 for
coils, FC06 or FC16 for holding registers): an echo is `Confirmed`, an exception
`Rejected`, a frame sent without a usable answer `Unknown` and never replayed, and the
items after an `Unknown` are not sent. `ping` reads the configured `ping-address` once
(S47): any correlated reply, an
exception included, is success, as upstream does (quirk d). Deferred items are in "Known
gaps".

### `stonecrab::sensors`, feature `sensors`

Profile schema version 1 and the TOML loader (64 KiB bound), semantic
validation collecting every error (range bounds finite and ordered since S36) and the
fixed unit table, the driver-aware `SensorProfile::validate_for(&dyn Driver)` (S39: an
address the driver refuses, and missing byte or word orders for an address whose
`ProtocolAddress::spans_multiple_words` is true; S37: status bits beyond the parsed data
width, overlaps between spellings of one address, and status bits inside the value's
`BitField`), `SensorProfile::tags(&dyn
Driver)` (declared convention passed as `AddressOptions`), and
`SensorProfile::measurements(&ReadResponse, wall_now)` with `Measurement` and `Quality`
(ADR-0016). Only synthetic profiles exist; no named device profile.

### `stonecrab::testkit`, feature `testkit` (test tooling, off by default)

`FixtureAddress`, `FakeClock`, `block_on`, `block_on_with`, `poll_once`, `CountingWaker`,
`Script`, `Step` (including `Respond` and `React`), `Reaction`,
`ScriptedPeer`, `ScriptedPeerFactory`, `FakeTransport`, `FakeTransportFactory`,
`CountingFactory`, `LengthPrefixedFramer` (the sanctioned doubles of CODING_STANDARDS
13.5), the golden-vector loader and `InteropMap`, and the
`ContractSuite` every driver runs (27 cases since S54; skips are named, never silent), which driver
authors can run against their own driver. Contract
results: simulator 22 passed and 1 skip; Modbus over the fake transport 21 and 2; over
loopback `ScriptedPeer` 18 and 5; against pymodbus 14 and 9.

### Tests and component documents

Unit tests, property tests, consumer integration tests, executable documentation (the
crate README and every component document are doctests) and Send/Sync assertions cover the
implemented contracts. The component documents live inside the crate, in
`crates/stonecrab/docs/components/` (`codec.md`, `core.md`, `modbus.md`, `sensors.md`,
`sim.md`, `testkit.md`, `transport.md`), and each module root includes its own as rustdoc;
`core.md` is compiled for its doctests only. Mandatory failure cases are tagged
`// covers: SPEC mandatory-failure-cases / F<n>` and listed in `docs/COMPATIBILITY.md`. The
README-vs-`Capabilities` tests (`crates/stonecrab/tests/sim_readme_matrix.rs`,
`modbus_readme_matrix.rs`) keep each driver document's support table equal to
`Driver::capabilities()`. The parity gate (`mise run parity`) keeps the parity records and
the sources honest about each other. Test files are named by module (`codec_*`, `core_*`,
`modbus_*`, `sensors_*`, `sim_*`, `transport_*`).

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
| S22 | 2026-10-27 | Modbus MBAP framing: `MbapHeader`, `TransactionId` (1 to 0xFFFE, wrapping), `ModbusAdu` with an opaque PDU, `MbapFramer` implementing the transport `Framer` (protocol id 0, length 2 to 254, ADU at most 260 bytes, header judged before any allocation, incomplete frames wait); the modbus crate now depends on the transport crate; spec-authored and adversarial `adu_*` vectors; sixth fuzz target `fuzz_modbus_mbap`; parity rows `MB-READ-FRAMING`, `MB-READ-TXN` implemented | check, coverage, lint-attrs, machete, dependencies, fuzz, package |
| S23 | 2026-10-27 | Modbus `TypedDecoder`: registers or packed bits to `PlcValue` for the 15 types under the four byte-order modes (ADR-0006 mapping, narrow values in the low byte or bit 0 under big-endian, bit-exact floats), wrong register counts as `MalformedData` with the received bytes kept in the status within the raw budget, `encode_registers` as the inverse; 85 derived `typed_*` vectors plus the interop map constants; `MB-BYTEORDER-*` and `MB-TYPE-*` rows implemented, automated-verified for byte order waits for the upstream little-endian vectors (P4) | check, coverage, lint-attrs, machete, dependencies, package |
| S24b | 2026-10-27 | Modbus `ModbusSession` over the transport `ConnectionGuard`: one exchange in flight (a second concurrent request gets `Backpressure` before any I/O; the connection slice serialises chunks), a transaction counter that continues across reconnects and wraps at 0xFFFE, a reply accepted only when transaction id, unit id and function code (exception bit cleared) all match (divergence `MB-READ-MATCH`; upstream matches the transaction id only), mismatches discarded, counted in `SessionStats` as late or mismatched and logged at `warn`, a device exception returned as `Ok(ModbusPdu::Exception)` with `exception_item_error` mapping per D18, a reply that does not decode or fit `MalformedData`; `tracing` adopted by the modbus crate (fuzz lock updated); `MB-READ-MATCH`, `MB-READ-INFLIGHT`, `MB-READ-TIMEOUT` and `MB-EXC-*` rows implemented | check, coverage, lint-attrs, features, machete, dependencies, fuzz, package |
| S29 | 2026-10-27 | Sensors measurement pipeline: `SensorProfile::tags(&dyn Driver)` (validation first, scheme check, the declared convention passed as `AddressOptions` to every `parse_address`, one tag per measurement and one `<measurement>.<status>` tag per status field), `SensorProfile::measurements(&ReadResponse, wall_now)` with `Measurement` and `Quality` (`Good`, `Uncertain`, `Bad`, `Stale`, `Unknown`; precedence Bad over Stale over Uncertain over Unknown over Good), checked scaling (integers stay integers while every step is integral and in range; Divide or a fractional constant promotes to f64; NaN, infinity and overflow are Bad; `BitField` rolls counters over; integer `Clamp` compares as integers), status bits with inversion, range checks, `source_time` never substituted; testkit `ConventionDriver` double; `SENSORS-TAGS` and `SENSORS-MEASUREMENTS` rows implemented | check, coverage, lint-attrs, features, machete, dependencies, package |
| S14 | 2026-10-27 | Facade `sim` feature (off by default): `stonecrab::sim` re-exports `stonecrab-driver-sim`, pinned `=0.1.0` per CODING_STANDARDS 20.1 (core, codec and transport keep `0.1`; recorded in the ADR-0014 addendum); `examples/sim_read.rs` reads three tags from `sim://bench` and reports through `tracing` events on a `tracing-subscriber` fmt layer with `ExitCode` (ADR-0011); `tokio`, `tracing` and `tracing-subscriber` 0.3 (`fmt`, `std` only) as facade dev-dependencies; D23 check finds `lazy_static` under `sharded-slab`, wrapped in `deny.toml` and mirrored in `fuzz/deny.toml`, with audit entries for `tracing-subscriber`, `sharded-slab`, `lazy_static` and `thread_local`; `examples` CI job and `mise run examples`; facade test proves the re-export | check, coverage, lint-attrs, features, machete, dependencies, fuzz, examples, package |
| S25 | 2026-10-27 | Modbus `ModbusConnection` implementing core `Connection` over `ModbusSession`, with `ConnectionDefaults` (unit id, byte and word order; S26's parser fills them): `open(config, factory, clock, defaults)` validates the scheme and frame budget before any transport opens; `read` checks terminal state, cancelled token, Modbus addresses, `count == 1` and the batch plan (item bound, 125/2000 chunk limits) before any I/O, holds one in-flight slot (`Backpressure` otherwise), runs chunks sequentially with a per-attempt deadline `min(request deadline, now + timeouts.request)`, retries a failed chunk per `RetryPolicy` with a fresh transaction id, lets a device exception fail only its chunk's members, decodes members through `TypedDecoder` with tag over block over default order, and builds the `ReadResponse` with `split_into = chunks`, `sampled_at` per chunk and `source_time` None; `write` is `Unsupported` with no I/O; reconnect is the guard's request-driven one; retry, reconnect, merge and capability parity rows implemented. Larger than its size class (about 990 library lines, mostly rustdoc, and 2560 test lines); the modbus coverage row stays 100 (2070/2070) | check, coverage, lint-attrs, features, machete, dependencies, package |
| S25b | 2026-10-27 | Modbus `ModbusTcpDriver` skeleton (public from this slice, not crate-private as the plan said, because the contract test is an integration test): `new()` over `TcpFactory` and `SystemClock`, `with_clock`, `with_transport_factory` (the sanctioned injection point for the testkit fake), `with_defaults`; scheme `modbus-tcp`; `parse_config` accepts `modbus-tcp[:tcp]://host[:port]` with default port 502, bracketed IPv6 and strict port rules and rejects every query parameter until S26 adds the six upstream parameters; `parse_address` delegates to the S20b parser; `connect` opens `ModbusConnection`; the transport dependency gains its `tcp` feature. The S13 contract suite runs through `dyn Driver` over `FakeTransportFactory` with a first-party MBAP responder on the interop register map (tests/support): 19 passed, 0 failed, 4 named skips (two write cases per D3, and two disconnect cases because a `Step::Respond` closure could not close the fake transport on demand; S25c added the hook and the two cases pass since); modbus coverage 100% (2412/2412); ADR-0015 decision 14; parity row `MB-CONTRACT-SUITE` | check, coverage, lint-attrs, features, machete, dependencies, package |
| S25c | 2026-10-27 | Testkit `Reaction` (`Reply`, `Silence`, `Close`, `ReplyThenClose`), `Reactor` and `Step::React`/`Step::react`: a scripted responder decides per request whether to answer, stay silent, close the peer or answer then close; `Step::respond` is unchanged and shares the code path; both `FakeTransport` and `ScriptedPeer` honour a reaction close exactly like `CloseAt` (end of stream then `Disconnected`; half-close on loopback). The modbus contract test wires `inject_disconnect` to it: 21 passed, 0 failed, 2 named skips (the write cases, D3); ADR-0009 addendum; ADR-0015 decision 14 and `MB-CONTRACT-SUITE` updated | check, coverage, lint-attrs, features, machete, dependencies, package |
| S26 | 2026-10-27 | Modbus TCP driver completed: core gains the driver-owned options slot (`DriverOptions` trait, `ConnectionConfigBuilder::driver_options`, `ConnectionConfig::driver_options_as::<T>()`; ADR-0008 amendment; `CORE-DRIVER-OPTIONS`) because a TCP endpoint cannot carry query text; `ModbusOptions` parsed from the six upstream parameters with upstream defaults and bounds (`request-timeout-ms` also sets `Timeouts.request`; `default-unit-identifier` 0..=255; `ping-address` validated under plc4x and unused; the four byte-order names through one shared table; `max-coils-per-request` 1..=2000 and `max-registers-per-request` 1..=125 fed into the batch plan; `address-convention=plc4x`; unknown keys rejected, divergence `MB-CFG-UNKNOWN`; driver defaults seed the parse and the string overrides them); seventh fuzz target `fuzz_modbus_connection_string` with a seed corpus; facade `modbus` feature (`stonecrab::modbus`, sibling pinned `=0.1.0`, `[[example]] required-features`) and `examples/modbus_tcp_read.rs` (`STONECRAB_URL`, `STONECRAB_ADDRESS`, ADR-0011 output rule; built in CI, run only by the S27 interop job); loopback evidence over `ScriptedPeer`: a strict-script holding-register read through `registry.connect("modbus-tcp://127.0.0.1:<port>")` and the S13 suite over the new testkit `ScriptedPeerFactory` (one peer per open): 18 passed, 0 failed, 5 named skips (three timing cases, which need the fake clock, and the two write cases); README completed (grammar, explicit-convention rule, parameter table, byte-order mapping, support levels) with the README-vs-`Capabilities` test; `MB-CFG-*`, `MB-QUIRK-A`, `MB-QUIRK-F` and `MB-TCP-CONNECT` (loopback) rows implemented; ADR-0015 decision 15, ADR-0009 addendum. `MB-TCP-CONNECT` and the four reads reach automated-verified only with S27 | check, coverage, lint-attrs, features, machete, dependencies, fuzz, examples, package |
| S27 | 2026-10-27 | Interoperability against pymodbus 3.8.6: `interop/pymodbus/` holds a first-party server (`server.py`, read with `tomllib` from the interop register map, `pymodbus` 3.8.6 pinned, `python:3.12-slim` pinned by index digest, non-root) and `interop.sh` (build, start, a readiness probe that performs a real FC03 read, nextest over the ten `interop_` tests with a count guard, the `modbus_tcp_read` example, evidence, stop); `crates/stonecrab-modbus/tests/interop_pymodbus.rs` (ten tests, ignored unless `STONECRAB_INTEROP_MODBUS` is set; a mutated server made four of them fail, so they are not vacuous); testkit `CountingFactory` (opens, frames and bytes through any transport factory) gives the contract suite an exact `io_probe` over a real socket; the suite against pymodbus: 14 passed, 0 failed, 9 named skips (timing, on-demand disconnect and device exception, writes); CI job `interop` (ubuntu, every push and PR, `interop-evidence` artifact with image id, `pip freeze`, server log, contract report, nextest and example output) and `mise run interop`; finding: pymodbus 3.8.6 does not validate FC01/FC02 spans, so the server's bit blocks cover the whole address space; `MB-TCP-CONNECT` and the four read rows automated-verified; `MB-INTEROP-*` rows implemented; ADR-0015 accepted (decision 16); the modbus README support table moves read and interoperability to interoperability-tested. Verified locally through Docker and green on the hosted runner (run 40543567845) | check, coverage, lint-attrs, features, machete, dependencies, interop, package |
| S30 | 2026-10-27 | Synthetic profiles and the definition-of-done example: `profiles/synthetic/modbus-tcp/{temperature_transmitter,pressure_flow_level,discrete_state}.toml` addressed to the pymodbus map under `address_convention = "plc4x"` (REAL temperature at holding register 26 with a coil status field plus a little-endian twin at 300, INT scaled by 0.1 to kPa, UDINT L/min, UINT mm with an inverted status, eight coils with inversion and discrete-input status fields), `profiles/synthetic/sim/` twins on `STATE/` addresses with the same names, units, ranges and quality maps (a test compares them), `profiles/README.md` (schema, the synthetic level, one section per file, no named device profile, repository data not shipped); facade features `sensors` (with the loader) and `full`, `stonecrab::sensors`, sibling pinned `=0.1.0`; `examples/profile_read.rs` (`STONECRAB_URL`, `STONECRAB_PROFILE`, `--expect-good` fails on any non-Good quality; ADR-0011 output rule) runs green with every measurement Good against `sim://bench` (examples job and `mise run examples`) and against pymodbus (interop job `profiles` step, `profiles.txt` evidence); `SENSORS-SYNTHETIC-PROFILES` implemented; ADR-0016 addendum. The sensors crate README's support rows are updated in S31 | check, coverage, lint-attrs, features, machete, dependencies, examples, interop, package |
| S31 | 2026-10-27 | Documentation reconciled to the shipped alpha: ARCHITECTURE.md "shipped at alpha" notes for the crate graph (D22), the core contracts, the transport layer, the Modbus crate, the lifecycle state machine and the feature matrix; README.md with the alpha statement, the capability matrix (support levels per `docs/SPEC.md`), the `profile_read` quick start and the full gate list; the seven crate READMEs and the testkit README at final support levels; `docs/COMPATIBILITY.md` (new); parity records at final statuses with real test ids; `scripts/parity-check.sh` as the `parity` CI job and `mise run parity` (every `# Compatibility` id resolves, every implemented row is referenced; eleven uncited rows were cited from rustdoc and test comments); SECURITY.md status, supported versions and the yanking rule; CHANGELOG per-crate alpha sections opening with the four SPEC.md release-note items; this file rewritten to the alpha baseline with the slice table kept | check, coverage, lint-attrs, parity, package |
| S32 | 2026-10-27 | Release evidence and process: ADR-0017 (publish order, signed per-crate tags prepared as commands, the yank procedure of CODING_STANDARDS 20.5, the evidence policy; references the ADR-0005 waiver); `release.toml` for cargo-release 1.1.7 with publish, push and tag off; `mutants.toml` and `mise run mutants` with one recorded run over fourteen scoped files (569 mutants: 462 caught, 0 missed, 107 unviable, three equivalent mutants excluded with reasons; the first run's seven survivors were killed by four added tests, test code only) stored as `docs/evidence/mutants/outcomes.json`; seven concurrent 30-minute fuzz runs with no artifact (`scripts/fuzz-evidence.sh`); the `release-evidence` dispatch workflow and `mise run sbom` (CycloneDX 1.5 per crate, package lists, absolute paths scrubbed); `docs/evidence/alpha.md` with the checklist; the upstream 1.0.0 source-release signature verified in a temporary keyring (`signature_verified = true`, nine pending items left); six upstream issues drafted under `docs/evidence/upstream-issues/`, not filed; the weekly `scheduled` job now runs the mutation scope and an advisory dry run; pinned tools cargo-mutants 27.1.0, cargo-cyclonedx 0.5.9, cargo-release 1.1.7 | check, coverage, lint-attrs, parity, machete, dependencies, sbom, package |
| S33 | 2026-10-27 | One crate (maintainer decision; supersedes D1 and the crate graph of ADR-0001, ADR-0011, ADR-0014): the six sibling crates become modules of `stonecrab` 0.2.0 (`stonecrab::codec`, `::transport`, `::modbus`, `::sim`, `::sensors`; core at the root) and the test kit becomes `stonecrab::testkit` behind the off-by-default `testkit` feature; default features enable `sim`, `modbus` and `sensors`, `default-features = false` is the tokio-free core; everything moved with `git mv`; 1182 tests and 414 doctests before and after; module-level lint promotions kept and checked by `lint-attrs`; coverage thresholds per module prefix; one packaging list and SBOM; fuzz workspace, scripts, mise tasks, CI and the interop driver renamed. The narrative sections of this file, README, ARCHITECTURE, COMPATIBILITY, ADR-0018 and the release evidence are reconciled in the following docs commit | check, coverage, lint-attrs, parity, features, machete, dependencies, fuzz, examples, interop, package |
| S33 (docs) | 2026-10-27 | Documentation, ADRs, release files and evidence reconciled to the single crate: ADR-0018 (the decision, what it supersedes or amends), ADR-0017 amended in place (one package and publish command, one tag `v0.2.0`, one yank), README.md for a non-engineering reader, the crate README and component docs, ARCHITECTURE module map and feature table, COMPATIBILITY, DEPENDENCY_AUDIT, coding standards (per-module lint promotions, one version), SECURITY, CONTRIBUTING, parity prose, one CHANGELOG section grouped by module with every bullet kept, this file's narrative; fresh mutation run on the merged tree (574 mutants, 465 caught, 107 unviable, 2 equivalent survivors whose redundant explicit fields are removed in this commit), fresh fuzz smoke; 46 `[[test]]` `required-features` entries; the redundant `homepage` manifest key dropped; hosted run of `dd0dd73` recorded (22 jobs green, `semver` included) | check, coverage, lint-attrs, parity, features, machete, dependencies, package, sbom |

## Post-alpha slices landed

Slices follow [`docs/POST_ALPHA_PLAN.md`](docs/POST_ALPHA_PLAN.md). Each row is one
commit; gates named were run locally before that commit.

| Slice | Date | Landed | Gates |
| --- | --- | --- | --- |
| S34 | 2026-10-28 | Documents reconciled with the 0.2.0 publication (H02 to H06): README and SECURITY say 0.2.0 is published and show the crates.io dependency; STATUS.md start, maintainer prerequisites (P1 to P3 open, the "Basic" ruleset targets no branch, P4 approved) and continuation point at `docs/POST_ALPHA_PLAN.md` with the E1 to E7 outcomes; `docs/PROJECT_BRIEF.md` marked historical with corrected badges; CODING_STANDARDS mirrors the real package keys and owner URL; a dated addendum in `docs/evidence/alpha.md`; docs only | check |
| S38 | 2026-10-28 | `stonecrab::sensors` denies `clippy::indexing_slicing` like codec, modbus, sim and transport (finding G04): the outer attribute on its module declaration, the `sensors` row of `scripts/lint-attrs.sh` and its self-test fixtures, CODING_STANDARDS 5.1 and 5.5, an ADR-0018 addendum. The module had no indexing or slicing, so no code changed and sensors stays at 100% lines; a probe proved the lint is live | check, coverage, lint-attrs, fuzz (fuzz_sensors_profile_toml, 60 s) |
| S40 | 2026-10-28 | Ping in the core API (finding G07, ping part): `Operation::Ping` appended last, `Capabilities.ping` (default false) read by `supports`, and a provided `Connection::ping(&self) -> OpFuture<'_, ()>` modeled on `close` (no request type, like PLC4J's `ping()`) that does no I/O and returns `Unsupported(Ping)` with diagnostics; neither driver overrides it (Modbus in S47). The contract suite has 24 cases: the new one asserts the error, its diagnostics, no device access and no state change, and runs for the simulator and every Modbus fixture. ADR-0011 addendum; core, sim and Modbus parity rows updated; additive for semver by construction (both types are `#[non_exhaustive]`) | check, coverage, lint-attrs, parity, examples |
| S04a | 2026-10-28 | Upstream Modbus vectors with provenance (P4 approved 2026-10-28, decision E2): `ParserSerializerTestsuite.xml`, `DriverTestsuite.xml`, `Modbus-all-datatypes.xml` and `Modbus-all-datatypes-little-endian.xml` retained byte for byte from commit ea229918 under `fixtures/modbus/upstream/tcp/` (git blob and sha256 in PROVENANCE.toml; identical to an independent checkout); the six `<raw>` frames of the parser suite as `tcp/adu_upstream-*` ADU vectors (FC04 request and 99-register response accepted; the four FC20/FC21 file-record frames cut by the framer and refused by the PDU codec as `must_fail`); `adapter_rederives_each_parser_suite_hex_from_raw_text` (string scan, no XML parser) and framing and PDU tests over the new vectors; NOTICE carries the PLC4X attribution and its NOTICE text; the crate ships nothing from `fixtures/` (package list unchanged); docs that said the vectors wait for P4 now say the exchanges wait for S04b | check, coverage, lint-attrs, parity, dependencies, package (with --allow-dirty before the commit) |
| S35 | 2026-10-28 | The `semver` job is blocking against the published `stonecrab` 0.2.0: `continue-on-error` removed and cargo-semver-checks pinned to 0.51.0 (it floated on the latest release); with an unchanged `version` the tool checks a minor release, so additive changes pass and a breaking change fails until its version bump (CODING_STANDARDS 9.10). Round 1's run 40787826005 reported 202 checks passing with "no semver update required". ADR-0005 addendum; STATUS "Known gaps" updated | workflow YAML parse (no code change; the job itself runs in CI) |
| S41 | 2026-10-28 | M0 source inventory of PLC4J v1.0.0 (commit ea229918) from a local sparse checkout: `upstream/manifest.toml` gains 61 `[inventory.*]` entries beside `modbus_tcp` (the API and SPI, 22 driver modules plus DF1 as a spec only, 12 transport modules, four tools, the utilities, and pointers to `apache/plc4x-extras` v1.0.0, `apache/camel` and `apache/streampipes`), each with operations, transports, notes and file and line evidence; pending items 1 and 5 closed (seven remain). The plan's PLC4X leads are resolved in `docs/POST_ALPHA_PLAN.md` section 4: confirmed where the source agrees, corrected otherwise (KNXnet/IP has a Java driver, UMAS handles DATE and TIME_OF_DAY, CAN alone is not a transport, the OPC UA and PLC4X servers, PCAP analyzer, browser and most integrations live outside `apache/plc4x`, Ditto has no module); Go, C, C# and Python claims are left to S42; docs only | TOML parse, ASCII check, citation bounds check against the checkout, 92 spot checks (three citations corrected) |
| S65 | 2026-10-28 | ADR-0019, the MSpec code generator, accepted under decision E3 (the maintainer may revisit): option (A), a native Rust generator `tools/stonecrab-codegen` in its own unpublished workspace (like `fuzz/`), parsing all of MSpec and emitting a growing, explicitly matrixed subset over `stonecrab::codec`, failing on anything else; output in `crates/stonecrab/src/<driver>/generated` with an `@generated` provenance header (tool version, mspec path, upstream commit, sha256) and no timestamp; S66 proves it on Modbus as a `#[cfg(test)]` module against the hand codec over the whole vector corpus; the S67 `regen` gate regenerates twice offline and requires a clean tree. Vendoring `modbus.mspec` under `upstream/mspec/` needs the maintainer's approval (P4 covered test vectors, not protocol descriptions); no code | link and citation check (docs only) |
| S36 | 2026-10-28 | Profile range validation (finding G01): `SensorProfile::validate` reports `ProfileError::InvalidRange` when a range has a NaN or infinite bound or `min > max` (equal bounds are valid), in document and field order after the unit; before, an inverted range made every value Uncertain and a NaN bound skipped the check. Tests for inverted, NaN, infinite and equal bounds, the property-test oracle and the several-faults order test updated, a hostile-range fuzz seed (the target exercises the loader only), ADR-0016 addendum, SENSORS-VALIDATION row and component doc; the pipeline is unchanged | check, coverage, lint-attrs, parity, fuzz (fuzz_sensors_profile_toml, 60 s) |
| S39 | 2026-10-28 | The word-order rule follows the driver (finding G05): `ProtocolAddress::spans_multiple_words` (provided, default false; Modbus answers true for every type wider than one register, from its register count, not a name list) and `SensorProfile::validate_for(&dyn Driver)`, which runs every `validate` rule, then parses each measurement address and reports `AddressRejected`, `MissingByteOrder` and `MissingWordOrder` in document order; `tags` runs it, so Modbus profiles without orders are still refused there. Behavior change: `validate` alone no longer reports the order errors (CHANGELOG, ADR-0016 addendum, SENSORS-VALIDATION). `WIDE_MODBUS_TYPES` is gone; the testkit's `ConventionDriver::with_wide_addresses` is the second driver in the tests (`sensors_word_order.rs`, 8 tests); sensors and modbus stay at 100% lines | check, coverage, lint-attrs, parity, examples |
| S04b | 2026-10-28 | Driver-suite XML adapter: `roxmltree` 0.21.2 as a dev-dependency only (ADR-0009 addendum: the alpha's testkit `fixtures` feature would now reach the published crate), the adapter is the test file `modbus_driver_suite_adapter.rs` with its own serializer from `modbus.mspec`, and its output is checked byte for byte against the committed vectors; 32 read exchanges (DriverTestsuite.xml 4, each all-datatypes file 14) become 64 `adu_upstream-*` vectors with PROVENANCE entries and counts. The typed-decoder tests decode every big-endian and little-endian response to the value the XML reports (a discriminator test shows the little-endian ones fail as big-endian), so `MB-BYTEORDER-BE` and `-LE` are `automated-verified`; the swap modes stay `implemented` (no retained file configures them). Findings: the all-datatypes files share the driver-suite step format (pending item 8 closed), upstream's `ModbusTcpDriverIT` loads `DriverTestsuiteOptimized.xml` (item 7 narrowed), and their 16-bit write cases pair an FC06 request with an FC16 response | check, coverage, lint-attrs, parity, dependencies, machete, package list (with --allow-dirty before the commit) |
| S45 | 2026-10-28 | Modbus write PDUs as upstream emits them: `FunctionCode` gains FC06, FC15 and FC16; `ModbusPdu` gains the three requests and their responses, with V1.1b bounds checked before allocation (FC15 1..=1968 coils, FC16 1..=123 registers, the 65536 address span, 253-byte PDUs) and the response echo checked against the request; `ModbusPdu::write_request` picks FC15 for every coil write, FC06 for a 2-byte holding-register payload and FC16 otherwise, and refuses read-only areas. FC05 is never emitted (upstream quirk c) and stays refused. 54 new vectors (spec examples with sections, adversarial must_fail, exceptions; the old D3 must_fail write vectors became spec vectors), fuzz seeds; `Capabilities.write` stays false (S46 puts writes on the wire); `decode_member` in the connection gained the five new variants in its mismatch arm | check, coverage, lint-attrs, parity, fuzz (fuzz_modbus_pdu, 60 s) |
| S47 | 2026-10-28 | Modbus ping: the connection overrides `Connection::ping` with one read of the configured `ping-address` (default `4x00001:BOOL`, its area's function code and width, the address's unit id) through the read slot and session, bounded by `timeouts.request`, no retry; `Capabilities.ping` is true. Quirk (d) decided: matched upstream, so any correlated reply, an exception PDU included, is success; silence is `Timeout`, a closed connection `Disconnected`, other failures keep their kinds (MB-QUIRK-D reproduced; MB-TCP-PING automated-verified, including a pymodbus case where a refused read still answers the ping). The contract suite has 26 cases (new: a supported ping answers and changes no state; a silent device makes it a Timeout), the interop job expects 12 tests, COMPATIBILITY splits ping from discover | check, coverage, lint-attrs, parity, examples, interop |
| S36, S45 (fix) | 2026-10-29 | The blocking `semver` job failed on run 40793164568: S45 inserted the write variants and S36 `InvalidRange` in the middle of the published `ModbusPdu` and `ProfileError` enums, shifting implicit discriminants (`enum_no_repr_variant_discriminant_changed`). The new variants now follow the published ones (no behavior change); CODING_STANDARDS 9.10 says so, and cargo-semver-checks 0.51.0 is pinned in mise with a `semver` task so the check runs before a push | check, coverage, semver (local, cargo-semver-checks 0.51.0) |
| S42 | 2026-10-29 | Non-Java inventory of PLC4X v1.0.0 from the pinned checkout: 39 more `[inventory.*]` entries (22 Go: the SPI, five transports, 14 drivers, two tools; 9 C: SPI, four drivers, three transports, the plc4x-server stub; 2 C#: generated KNXnet/IP model only, no driver; 6 Python: SPI, mock, Modbus, UMAS, two transports), each compared with its Java entry, and a `[[backlog]]` table of 52 capabilities that exist outside Java and are absent or weaker in Java (for example Go BACnet/IP read, write, discovery and COV, Go C-Bus, Python UMAS system-bit addresses). Pending item "non-Java implementations" closed; the plan's section 4 cells that waited on S42 are filled; S41's plc4x-server citation corrected; docs only | TOML parse, ASCII check, 90 spot checks (three Go claims dropped, one citation corrected) |
| S43 | 2026-10-29 | One parity record per upstream protocol from the S41 and S42 inventory: 19 protocol files (S7, PROFINET, EtherNet/IP with Logix, ADS, AB-ETH, OPC UA, Modbus RTU and ASCII, SLMP, UMAS, the PLC4X proxy, BACnet/IP, KNXnet/IP, C-Bus, CANopen, generic CAN, IEC 60870-5-104, Firmata, Open-Protocol with DF1, ctrlX) plus `tools.md` and `integrations.md` (E6's tracked list), upstream transport rows in `transport.md`, and the release matrix in `docs/COMPATIBILITY.md`; rows are planned (milestone) or deferred (reason), and every planned row names a `planned:` test id (pending item "Map every required behavior" closed). KNXnet/IP moved to M5 because a working Java driver exists (the plan's "Non-Java backlog" came from the corrected website lead); every manifest entry now names its parity record; docs only | parity (and its self-test), TOML parse, row-shape and unique-id checks |
| S54 | 2026-10-29 | The subscription API (findings G07 subscribe part and G10), ADR-0020: `SubscribeRequest::builder(limits, kind, queue)` like `ReadRequest` (tags, deadline and cancel bound only the call; refuses empty, duplicate, over-budget, zero-interval and over-wide coalescing requests), `SubscriptionKind`, `QueuePolicy` (backpressure never loses, coalescing keeps the latest per tag and counts it, dropping counts every loss), `Subscription` (handle and stream: `recv`, `mode`, `interval`, `loss`, `unsubscribe`; drop ends it), `SubscriptionSender` for producers, a queue bounded by `max_queued_events` without executor types, and a provided `Connection::subscribe` returning `Unsupported(Subscribe)` with diagnostics (ping and subscribe share one private helper). Proptests per policy (bound, conservation, order, latest per tag); the contract suite has 27 cases (an unsupported subscribe performs no I/O for every policy and kind) | check, coverage, lint-attrs, parity, examples |
| S46 | 2026-10-29 | Modbus writes on the wire: `ModbusConnection::write` refuses the default read-only policy first with no I/O; a new Stonecrab connection-string parameter `access=read-only|read-write` (default read-only) grants writes (ADR-0008, ADR-0010 and ADR-0015 addenda). One request per item as upstream sends them (`ModbusPdu::write_request`, the typed encoder with tag over block over default order), through the read slot, session and deadline rule; an echo is `Confirmed`, an exception `Rejected` with its protocol status, a sent frame without a usable answer `Unknown` (never retried or replayed, including upstream's FC06-request/FC16-response test pairing, which never confirms), and the items after an `Unknown` are not sent (MB-WRITE-HALT, a recorded divergence). `Capabilities.write` is true. The contract write cases run for every Modbus fixture; 60 upstream write exchanges become vectors through the S04b adapter and Stonecrab's encoder reproduces their request bytes; three pymodbus interop write tests (interop expects 15) | check, coverage, lint-attrs, parity, examples, interop |
| S43 (fix) | 2026-10-29 | The hosted `parity` job failed on run 40798426129 with "printf: write error: Broken pipe": `scripts/parity-check.sh` piped the whole row list into an awk that exits at the first match, and once S43's records outgrew the pipe buffer the early exit broke the pipe under `set -o pipefail`. The awk now reads all its input and answers at END; macOS had not tripped it | parity and its self-test; the failure reproduced locally with a 2 MB row list (exit 141) and passes after the fix |
| S37 | 2026-10-29 | Status-bit and alias validation (findings G02, G03): `validate` refuses a status bit of 64 or more (`StatusBitOutOfRange`); `validate_for` parses status-field addresses once (one `StatusAddressRejected` mapping shared with `tags`) and reports a bit beyond the parsed type's width, `OverlappingStatusBits` for two spellings of one canonical address and bit (equal text stays `validate`'s), and `StatusBitInBitField` when a status bit lies in the measurement's first-step `BitField` range on the same canonical address; measurements may still share a status word. Two new `ProfileError` variants, appended after the published ones; ADR-0016 addendum defines the shapes and their limits (canonical text includes the type and unit id); 18 tests in `sensors_overlap.rs` and a fuzz seed | check, coverage, lint-attrs, parity, examples |
| S52 | 2026-10-29 | Benchmarks: criterion 0.8.2 as a dev-dependency only (default features off: no rayon or plotters; ADR-0009 addendum, DEPENDENCY_AUDIT entry, policy check clean; a C compiler is needed to build the benches); `benches/modbus_wire.rs` (address and connection-string parse, PDU and MBAP encode and decode, TypedDecoder over 62 REAL and 125 UINT, Batcher plans) and `benches/modbus_loopback.rs` (reads through the registry and the real TCP factory to a testkit ScriptedPeer on 127.0.0.1). Baseline on an Apple M2 Max recorded in `docs/evidence/benchmarks/` (a laptop baseline that the gate never reads). The 10% gate (`scripts/bench-compare.sh`, `mise run bench-compare`) builds a base and HEAD on the same runner, runs three alternating rounds and fails only when every round shows HEAD's lower bound above 1.10 times the base's upper bound; it runs in the weekly `scheduled` job only | check, lint-attrs, dependencies, machete, bench --no-run, one full local bench run, bench-compare self-test and an injected-slowdown trial |
| S54 (fix) | 2026-10-29 | `packaging/stonecrab.txt`, the reviewed archive list, gains `src/subscribe_request.rs` and `src/subscription.rs`, which S54 added; the `package` gate found the gap before the push | package |
| S44 | 2026-10-29 | The Modbus-specific pending items: TIME/DATE settled (the item's premise was wrong: the generated Java, Go and Python `DataItem` code is in the pinned checkout, and `modbus.mspec` has no TIME/DATE-family case, so upstream reads such tags as OK with a null value and throws on write; 8 bytes, 4 registers per element; recorded in the Data types rows and MB-QUIRK-K); the upstream test harness read end to end (whole-frame injection through the `test` transport, model comparison, the witness limits of the driver-suite vectors; a harness table under "Test vectors available upstream"); the Modbus part of the alias review closed with new rows (transport option prefixes, option value aliases, the config block, legacy array forms) and corrected default ports and a wrong URL-decoding claim in `core.md`; 20 evidence keys added to `[inventory.modbus_tcp]`. The spec-section item stays: modbus.org refused the automated download (HTTP 403), so it waits for a manual copy; docs only | parity, TOML parse; every finding re-opened in the pinned checkout before use |
| S58 | 2026-10-29 | Serial transport on a blocking worker: `serial2` 0.2.39 (BSD-2-Clause OR Apache-2.0; a first attempt on `serialport` needed an MPL-2.0 exception the maintainer had not granted and never landed) behind the optional `serial` feature; one named std thread per port owns it, async callers send commands over a bounded channel (capacity 2) and race each answer against their deadline, writes go out in 256-byte chunks with 20 ms port timeouts so a stalled line ends at the caller's deadline, a zero-byte write is retried, and received bytes live in a stash so a dropped `recv` loses nothing; close joins the worker. Tests run on a pseudo-terminal pair on Linux (serial2 cannot make one on macOS, ENOTTY), in-memory worker tests everywhere; ADR-0014 addendum, DEPENDENCY_AUDIT entry, CODING_STANDARDS names serial2 | check, coverage (transport 98.81% locally), lint-attrs, dependencies (deny unchanged), machete, semver, package list, no-default-features with and without serial |
| S51 | 2026-10-29 | The gap question and two address conventions: MB-READ-GAP decided as a divergence with evidence (upstream merges unconditionally while the span fits, reading the holes between tags, with no option of its own; matching it would turn a hole in a device map into exception 02 for every tag of the merged request), so the batcher keeps adjacent-or-overlapping merging and a new test pins that a hole is never read; `AddressConvention` gains `ZeroBased` and `OneBased40001` (exact definitions in the ADR-0015 addendum; `OneBased40001` reads numbers as the plc4x convention does), accepted by the address grammar, the connection string and sensor profiles; the address fuzz target reads every input under each convention, with five new seeds | check, coverage, lint-attrs, parity, examples, semver, fuzz_modbus_address 20 s |

## Verified evidence and reproduction

Development toolchain: **Rust 1.99.1**.
Declared MSRV: **1.97**; locally exercised with **1.97.1**.
Pinned tools: `cargo-llvm-cov` **0.9.1**, `cargo-deny` **0.20.2**, `cargo-nextest`
**0.9.148**, `cargo-hack` **0.6.45**, `cargo-machete` **0.9.2**, `cargo-fuzz` **0.13.2**
(fuzz workspace on `nightly-2026-10-01`), `cargo-mutants` **27.1.0**, `cargo-cyclonedx`
**0.5.9**, `cargo-release` **1.1.7**.

From the repository root (the gates run with `--all-features`; a plain `cargo test` runs the
reduced suite without the `testkit` feature):

```sh
mise install
mise run check
mise run coverage
mise run lint-attrs
mise run features
mise run machete
mise run dependencies
mise run fuzz
mise run examples
mise run interop     # needs Docker
mise run parity
mise run package
mise run dco -- origin/master..HEAD
mise run mutants     # recorded run, MUTANTS_JOBS sets the parallelism
mise run sbom        # SBOM_ALLOW_DIRTY=1 on an uncommitted tree
```

Most recent local results, for the S33 documentation tree (the code is that of the S33
build commit `dd0dd73`; this docs slice changed documentation, the release configuration
and the `[[test]]` entries of `crates/stonecrab/Cargo.toml`, no library source):

- `mise run check`: formatting, strict Clippy, nextest (**1182 tests**, 10 skipped),
  `cargo test` with doctests (**414 doctests**, the same counts and names as before the
  merge), documentation with warnings denied and the no-default-features check passed.
  `lint-attrs`, `parity` (and its self-test), `machete`, `dependencies` and `examples`
  passed.
- Per-module line coverage (ADR-0005, `mise run coverage`): core 3577/3577 lines (gate
  100%), `codec` 181/181 (gate 100%), `modbus` 2768/2768 (gate 100%), `sim` 2053/2053
  (gate 100%), `sensors` 1411/1411 (gate 100%), `transport` 1224/1228 (gate 95%; the
  uncovered lines are the real-socket deadline-expiry and resolver-failure arms named in
  ADR-0014), `testkit` 4609/4682 (gate 90%). Error paths are not excluded. Region
  coverage is reported in `target/coverage-summary.txt` and not gated; do not describe
  it as branch coverage.
- `cargo tree -p stonecrab -e normal` with the default features shows `tokio`, `tracing`,
  `serde` and `toml` (and `bytes`, `thiserror`); with `--no-default-features` it shows
  `bytes` and `thiserror` only.
- The package archive is **102 files** and equals `packaging/stonecrab.txt` (`cargo package
  --locked --allow-dirty -p stonecrab --list`); `mise run package` itself needs a clean
  tree. `SBOM_ALLOW_DIRTY=1 mise run sbom` writes one CycloneDX file.
- Mutation testing (`mise run mutants`, MUTANTS_JOBS=3): 574 mutants tested in 69
  minutes: 465 caught, 0 timeouts, 107 unviable and 2 missed. The two are equivalent
  (deleting `subscribe` or `concurrency` from the `..Capabilities::default()` expression
  of `SimDriver::capabilities`), and cargo-mutants 27.1.0 cannot exclude that mutant
  genre, so the recorded run reports them as missed; the three earlier equivalent
  mutants remain in `exclude_re`. The docs commit then removed those two redundant
  explicit fields from `SimDriver::capabilities` (the defaults are those values, so the
  behaviour is unchanged): the next run has no survivor and the script exits 0. See
  `docs/evidence/mutants/README.md`.
- Fuzzing: `scripts/fuzz-smoke.sh 10` ran all seven targets for 10 seconds each on the
  merged tree and passed (246,088, 244,773, 185,626, 277,851, 224,544, 336,310 and
  78,999 runs, exit 0, no artifact and no change under `fuzz/`). The seven recorded
  30-minute runs predate S33; the targets' code is unchanged apart from their import paths
  (`docs/evidence/alpha.md`).
- Cross-compilation checks (`aarch64-unknown-linux-gnu`, `armv7-unknown-linux-gnueabihf`)
  and the 1.97.1 test run are CI cells; they were last recorded in the S32 hosted run.
  These are compile checks, not device tests.
- The `interop` run (Docker, pymodbus 3.8.6) passed locally in S27 and S30 and on the
  hosted runner for S30 and S32; it was re-run locally on the merged tree for this
  slice: ten interop tests (10 passed), the `modbus_tcp_read` example, the contract suite
  (14 passed, 9 skipped by name) and `profile_read --expect-good` for each `modbus-tcp`
  profile, exit 0.
- `scripts/parity-check.sh` and its `--self-test` pass: 117 implemented or
  `automated-verified` parity rows are each named by a `# Compatibility` section or a
  test and carry test ids, and every `# Compatibility` reference resolves.

Additional commands used locally:

```sh
mise exec rust@1.97.1 -- rustup run 1.97.1 cargo test --locked --workspace --all-features
mise exec -- cargo check --locked --workspace --all-features --target aarch64-unknown-linux-gnu
mise exec -- cargo check --locked --workspace --all-features --target armv7-unknown-linux-gnueabihf
```

### Hosted CI and the per-OS matrix

The hosted runs recorded so far, newest first:

| Commit | Run | Result |
| --- | --- | --- |
| S33 build, `dd0dd73` | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40599885679 | completed successfully: all 22 jobs green, `semver` included now that the crate has a published baseline |
| S33 docs reconciliation, `640665a` (the release commit) | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40612938390 | completed successfully: every job green |
| `2d38e48` (S32) | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40584025718 | completed successfully; every job green except the report-only `semver` |
| `8f5c3d0` (S31) | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40557968227 | completed successfully; first run of the `parity` job |
| `f19be4c` (S30) | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40548582722 | completed successfully; the definition of done on the hosted runner |
| `0429da3` (S27) | https://github.com/TheMadBotterINC/stonecrab/actions/runs/40543567845 | earlier run |

The S33 build commit and this documentation commit have no hosted result recorded yet:
push them, check the run and record its id in this table. The job list of the last
recorded run (S32, `2d38e48`):

| Job | Result |
| --- | --- |
| `test`: ubuntu, macOS, Windows, each on Rust 1.97.1 and 1.99.1 (six cells; nextest, `cargo test` with doctests, no-default-features check) | green |
| `quality` (fmt, strict Clippy, docs with warnings denied, per-module coverage gate) | green |
| `cross` (aarch64 and armv7 compile checks) | green |
| `dependencies`, `package`, `dco`, `features`, `machete`, `lint-attrs` | green |
| `geiger` (no first-party `unsafe` outside `forbid(unsafe_code)`), `beta` (report only) | green |
| `fuzz-smoke` (seven targets) | green |
| `parity` (`scripts/parity-check.sh` and its self-test) | green |
| `examples` (`sim_read` and the three `profile_read --expect-good` runs on the hosted runner) | green |
| `interop` (pymodbus 3.8.6 on the amd64 runner: ten interop tests, the `modbus_tcp_read` example, the `profiles` step, the `interop-evidence` artifact) | green |
| `semver` (report only) | did not pass (it had no published baseline for the sibling crates); does not fail the run. It now has a baseline, `stonecrab` 0.1.0, see "Known gaps" |

The `release-evidence` workflow runs on dispatch only and has not been dispatched yet.

Platform fixes that hosted runs needed, for the next maintainer: the prebuilt cargo-fuzz
binary defaults to a musl target, so the smoke script passes the host triple; the
drop-mid-write transport test is ignored on Windows, where loopback accepts a 32 MiB write
at once; a sensors test that scanned a whole `Measurement` Debug string for status bytes
was scoped to the protocol status because the real monotonic clock's digits matched a byte
once (run 40513557067); and the harness of `fuzz_modbus_connection_string` checks only
non-ASCII authority and value text that no key contains (its echo invariant had been
unsound), with the three crash inputs kept as corpus seeds and named regression tests
(CONTRIBUTING, fuzz findings).

Mutation testing is established (`mise run mutants`, one recorded run in
`docs/evidence/mutants/`, weekly in the `scheduled` job); nothing establishes the soak,
benchmark or hardware gates yet.

## Review policy and the single-maintainer waiver

ADR-0005 records the waiver of the two-approval rule of CODING_STANDARDS 18.6 while the
repository has one maintainer, with the expiry condition "a second maintainer with merge
rights exists" and the Appendix C self-review each PR carries. It also records the
coverage tiers (now thresholds per module of the one crate, ADR-0018), the CI gates and,
in its S31 addendum, the parity gate.

## Important implementation and tooling details

- The actual API is `ReadRequest::builder(limits)`, not the brief's no-argument
  builder. It takes an absolute `.deadline(...)`; `.deadline_after(...)` does not
  exist. `Deadline::after(now, duration)` takes the clock reading explicitly. A request
  without a deadline gets `Deadline::after(clock.now(), timeouts.request)` from the
  connection; that bounded default never authorizes unbounded I/O.
- `Driver::parse_address` takes `&AddressOptions`; take the options with
  `ConnectionConfig::address_options()` before `connect` consumes the configuration.
- Passing request and response tests is not evidence of wire behavior. Claims rest on the
  parity rows and their evidence (the interop job, the contract suite, the fake-transport
  tests), and the component documents' support tables must stay equal to the `Capabilities`
  impls.
- Partial and all-item failures remain valid response data. Do not collapse them
  into a batch error or invent a device timestamp (`source_time` is `None` unless a device
  supplies one).
- Never turn an unknown write outcome into a rejection or automatically replay it.
- Address methods must return stable, credential-free metadata. Do not add an
  `as_any` shim; stable trait upcasting is already used.
- **Lint promotions are per module** (CODING_STANDARDS 4.6 and 5.1, ADR-0018). `lib.rs`
  carries `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]` and
  `#![deny(clippy::as_conversions)]` as inner attributes, and outer `#[deny(...)]`
  attributes on the `pub mod` declarations of `codec`, `modbus` and `sim`
  (`indexing_slicing`, `arithmetic_side_effects`, `as_conversions`), `transport`
  (`indexing_slicing`, `as_conversions`) and `sensors` (`indexing_slicing`, since S38);
  `testkit` carries none. `scripts/lint-attrs.sh` checks both tables and
  that the module root files carry no inner lint attribute; a new module takes a row in it
  and in `scripts/coverage-gate.sh`.
- **Use `--all-features` for the full suite.** Most integration tests need the `testkit`
  feature, and `[[test]]` `required-features` make a plain `cargo test` skip them silently
  (the default features do not include `testkit`). `mise run check`, `coverage`,
  `mutants` and CI run with `--all-features`.
- **The testkit's own tests and doctests read repository files** (`testdata/`,
  `fixtures/`), which the archive does not contain: run the full suite from a checkout, not
  from the unpacked archive. Packaged unit tests and documentation without those files
  are self-contained.
- Reconnect is request-driven (plan D15): no background task, no `tokio::spawn` in the
  library (`clippy.toml` disallows it; the testkit's `ScriptedPeer` owns the one spawn with
  the required `#[expect]`).
- The word `unsafe` may not appear in any first-party Rust file except the
  `forbid(unsafe_code)` attribute: the `geiger` job greps for it, comments and strings
  included.
- `mise run package` uses a **fresh Cargo cache**. Repeatedly verifying changed,
  unpublished crates with the same version in a reused cache previously selected an older
  archive. Do not remove the isolation as a cosmetic change.
- Update `packaging/stonecrab.txt` when shipped files change, and review the resulting
  archive contents.
- New dependencies require a rationale, the applicable audit/ADR entry and the D23
  `cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono` check.
  Runtime dependencies are `thiserror` and `bytes` (always), `tokio` with scoped features
  (feature `transport`), `tracing` (std only; features `sim` and `modbus`; `once_cell`
  reaches the graph through `tracing-core` under a recorded deny.toml wrapper), `serde` and
  `toml` (feature `sensors`) and, for the `testkit` feature, `serde_json`; dev-only
  `proptest` and `tracing-subscriber` (the examples; `lazy_static` reaches the graph
  through `sharded-slab` under a second recorded wrapper). `toml_parser` is pinned in
  `Cargo.lock` so that only one `winnow` resolves; a plain `cargo update` must keep that
  pin or allow the duplicate.
- A parity row id is the first cell of a table row in `docs/parity/*.md`. A public item
  that maps to upstream behavior carries a `# Compatibility: docs/parity/<file>.md#<ROW-ID>`
  rustdoc section; `mise run parity` fails on an id that does not resolve and on an
  `implemented` row that nothing names.

## Baseline and incomplete M0 work

[`upstream/manifest.toml`](upstream/manifest.toml) pins Apache PLC4X **v1.0.0**:

- Annotated tag object: `32a524591841ab56d26b032914ce06918148fbb6`.
- Peeled implementation commit: `ea229918f5d07700e04d7d69ab1645da679cb700`.
- Tag/commit relation verified against Git and the GitHub ref API.
- `signature_verified = true` (S32): the 1.0.0 source-release archive carries a good
  signature from a key listed in the project KEYS file, verified in a temporary keyring
  (fingerprint, SHA-512 and the trust caveat are in the manifest's `[signature]` table);
  the archive contents were not inventoried against the pinned commit.
- `inventory_complete = false`: S41 (2026-10-28) closed the source inventory and the
  Modbus RTU/ASCII item, S04b the step-format comparison, S42 the non-Java inventory, S43
  the behavior-to-test mapping and S44 the TIME/DATE `DataItem` and test-harness items;
  the open items in the manifest `[pending]` table are:
  1. Review all configuration and address aliases before implementation, for every
     protocol except Modbus TCP, RTU and ASCII (S44 closed the Modbus part).
  2. Check the Modbus Application Protocol section numbers and example bytes cited in
     `docs/parity/modbus.md` and `fixtures/modbus/PROVENANCE.toml` against a retained copy
     of the specification: modbus.org's firewall refused the automated download of
     V1.1b3 (HTTP 403) on 2026-10-28, so this needs a manually downloaded copy.

**M0 is not complete.** The manifest holds 101 `[inventory.*]` entries with file and line
evidence at the pinned commit: from S41 the Java API and SPI, every PLC4J driver module
(22, plus DF1 as a spec only), the 12 transport modules, the four tools, the utilities
and pointers to the tools and integrations outside `apache/plc4x` (`apache/plc4x-extras`
v1.0.0, `apache/camel`, `apache/streampipes`); from S42 the Go, C, C# and Python
implementations. Its `[[backlog]]` table (52 rows) lists what exists outside Java and is
absent or weaker in Java, and every entry names its parity record. Since S43 every
upstream protocol has one under `docs/parity/` (21 new records, planned or deferred rows,
every planned row naming a `planned:` test id), with `tools.md`, `integrations.md` (the
tracked list of decision E6) and upstream transport rows in `transport.md`;
`docs/COMPATIBILITY.md` lists every upstream driver, transport, tool and integration with
its support level. Only Modbus TCP is implemented. None of the open items gates the
alpha. Every divergence row now cites a protocol section, a drafted upstream
issue (`docs/evidence/upstream-issues/`, not filed: filing needs the maintainer's
approval) or the reason none was drafted.

Verified Modbus pitfalls are recorded in `docs/parity/modbus.md`: numeric address
groups are decimal; `x` in forms such as `4x00016` is an area marker, not hexadecimal.
Upstream uses `holding-register:16:INT`, IEC-style type names and
`default-unit-identifier`, not the original brief's provisional syntax. The
protocol-address offset is one. Modbus TCP's upstream subscriptions are polling,
not native subscriptions.

## Known gaps and deferred items

Deferred features, each a deferred row in `docs/parity/modbus.md` or an item of the plan's
"Out of scope for alpha" table with its owner:

- **Modbus writes** (first post-alpha slices, M1 completion): write PDUs, the upstream
  FC06-versus-FC16 selection, `WriteOutcome::Unknown` on the wire. The simulator
  implements write so the contract has a real exerciser.
- **Subscriptions** (M2): polling emulation, `StaleTracker`, the acquisition loop, eager
  background reconnect, `Timeouts.idle` enforcement. Browse, discovery and ping (M3 or the
  parity backlog).
- **Extended registers** (FC20 and FC21), **strings** (`STRING`, `WSTRING`, `CHAR`,
  `WCHAR`) and **calendar types** (the `TIME` and `DATE` family), array selections and
  `ZeroBased` / `OneBased40001` conventions (M2; calendar values need an ADR first). The
  parser rejects each with a typed error.
- **Modbus RTU and ASCII**, serial transport, checksums (M2); UDP, TLS and TLS-PSK (M3 or
  the parity backlog); board adapters (M2S); OPC UA (M3); EtherNet/IP and S7 (M4);
  code generation and the remaining baseline protocols (M5).
- **Hardware**: no physical device has been tried and no hardware record exists.
- **Gates not run**: the 8-hour release fuzz run and 24-hour soak (the recorded runs are
  30 minutes per target, made before S33), benchmarks, differential tests against PLC4J,
  branch coverage (region coverage is reported) and `cargo publish --dry-run` against the
  registry.
- **The `semver` job is blocking** since S35, with cargo-semver-checks pinned to 0.51.0,
  against the published `stonecrab` 0.2.0. An unchanged `version` is checked as a minor
  release (run 40787826005: 202 checks pass, "no semver update required"), so a breaking
  change fails CI until the version bump that announces it, with its migration note.
- **A plain `cargo test` runs the reduced suite** (no `testkit` feature, so the test files
  that need the kit are skipped without a notice) and the kit's own tests need a checkout;
  see "Important implementation and tooling details".
- **Upstream vectors**: S04a and S04b turn the four retained Apache-2.0 PLC4X test files
  into 70 vectors. Every big-endian and little-endian exchange decodes to the value the
  XML expects, so `MB-BYTEORDER-BE` and `MB-BYTEORDER-LE` are `automated-verified`; no
  retained file configures a swapped mode, so the two swap rows stay `implemented`. The
  write cases are retained but not vectors yet (S45, S46); `DriverTestsuiteOptimized.xml`,
  the file upstream's own driver IT loads, is not retained.
- **Not verified by the interop run**: timing over a real clock, dropped links and device
  exceptions on demand (a stock pymodbus has no fault hooks), unit-id correlation, and any
  server other than pymodbus 3.8.6.
- A queued-cancellation case is exercised on `ConnectionGuard` only, not on the simulator
  (which has no wait queue); the real-socket connect-timeout arm of the transport is
  uncovered and named in ADR-0014.

### Maintainer prerequisites still owed

These are actions outside the code (plan P1 to P5), as verified on 2026-10-28:

- **P1** open: GitHub private vulnerability reporting is still disabled (`enabled: false`).
  Enable it and verify it with a test report. `SECURITY.md` still says the channel is a
  release prerequisite.
- **P2** open: secret scanning and push protection are disabled.
- **P3** open: a repository ruleset named "Basic" exists with the rules deletion and
  non_fast_forward, enforcement active, but its branch include list is empty, so it
  applies to no branch. Required checks, linear history and squash-only are not
  configured.
- **P4** approved by the maintainer on 2026-10-28 (decision E2 of the post-alpha plan):
  the Apache-2.0 PLC4X Modbus test vectors are redistributed with NOTICE attribution. It
  unblocks **S04a** and **S04b** (the upstream vector copies, the adapter and the
  upstream-driven tests).
- **P5** (the single-maintainer waiver) is done and recorded in ADR-0005.

## Recommended continuation

The alpha plan is executed (S01 to S33; S04a and S04b are unblocked by the P4 approval and
are scheduled in the post-alpha plan) and `stonecrab` 0.2.0 is published. The next work is
[`docs/POST_ALPHA_PLAN.md`](docs/POST_ALPHA_PLAN.md), accepted by the maintainer on
2026-10-28: do Phase A (S34, S35) first, then the phases in the order that plan gives.

Maintainer-only steps, which no automated run does: P1 to P3 above, creating and pushing
the annotated tag `v0.2.0` on commit `640665a`, dispatching the `release-evidence`
workflow once (it has never been dispatched), deciding whether to file the six drafted
upstream issues under `docs/evidence/upstream-issues/` (none is filed), and E4 (test
hardware, deferred).

### Post-alpha decisions (2026-10-28)

- **E1** done: 1fcc6ae was amended with a `Signed-off-by` trailer as 554ad24 and
  force-pushed with a lease; no commit is GPG-signed and no signing key is configured.
- **E2** P4 approved: redistribute the Apache-2.0 PLC4X Modbus test vectors with NOTICE
  attribution (unblocks S04a and S04b).
- **E3** the MSpec generator (Phase G) comes before OPC UA.
- **E4** test hardware is deferred: no purchase is possible now.
- **E5** S7 or EtherNet/IP is decided when a test PLC is available.
- **E6** PLC4X tools count toward parity; integrations go in a tracked list.
- **E7** the sensor `unit` becomes `Option<String>` in the next 0.x minor release.

Keep changes reviewable, commit tested slices one signed-off commit per green slice, and
maintain the per-module coverage gates. No first-party unsafe, panics on input, unbounded
I/O, silent sample loss, inferred byte order or automatic replay of ambiguous writes. Do
not mark planned capabilities supported.

Do not publish another crate version or operate hardware without separate approval.
Never place credentials in files, logs, commit messages or handoff documents.
