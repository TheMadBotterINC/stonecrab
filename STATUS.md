# Stonecrab status and engineering handoff

Updated: 2026-10-27.
Implementation baseline: the commit that last modified this file
(`git log -1 --format=%h -- STATUS.md`). Every alpha slice updates this file in
its own commit; the slice log below records what each one landed.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition. The repository is at the **alpha baseline**: the alpha defined in
[`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md) is built, its definition of done is met, and
the release evidence and process (slice S32, ADR-0017, `docs/evidence/alpha.md`) are
recorded. Nothing of the alpha is published, tagged or posted upstream: those steps need
the maintainer's separate approval (see "Recommended continuation").

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.1.0**, a specification-stage placeholder. It is
  the only published version of any crate.
- Alpha versions, all unreleased: facade **0.2.0**; core, codec, transport, driver-sim,
  modbus and sensors **0.1.0**. The facade's minor bump records removal of the published
  `placeholder::status` API; do not restore a compatibility shim without a real consumer
  requirement. Plain pre-1.0 versions are used rather than `-alpha.N` (plan D1).
- Review policy: ADR-0005 waives the two-approval rule of CODING_STANDARDS 18.6 while there
  is one maintainer (the single-maintainer waiver, plan prerequisite P5, done). The waiver
  expires when a second maintainer with merge rights exists; until then every PR carries an
  Appendix C self-review.
- Landed slices are pushed to `origin/master` and the hosted CI run is checked. Publishing
  a crate, pushing a tag and operating hardware need separate maintainer approval. There is
  no unfinished source edit at this handoff.

## Source documents and precedence

Read these before changing contracts:

1. [`docs/SPEC.md`](docs/SPEC.md): scope, parity definition and acceptance criteria.
2. [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md): normative coding policy,
   including links to repository-specific overrides.
3. [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md): the crate graph and contracts. Its
   diagrams are the original targets; each shipped section has a "Shipped at alpha" note
   that wins where it differs, and what is not shipped is marked deferred with its owner.
4. [`docs/PROJECT_BRIEF.md`](docs/PROJECT_BRIEF.md): retained original README brief.
5. [`docs/adr/`](docs/adr/): accepted implementation decisions (ADR-0001 to ADR-0016).
6. [`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md): the support-level matrix, data types,
   configuration keys, tested peers, platforms and upstream divergences.
7. [`docs/parity/`](docs/parity/): `modbus.md`, `core.md`, `sim.md`, `transport.md` and
   `sensors.md`, the row-by-row compatibility records with test ids.
8. [`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md): the plan and decisions D1 to D23 the slices
   follow.

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

Per crate, at the shipped shapes. The parity rows behind each item are in
`docs/parity/`; ADR numbers are in `docs/adr/`.

### Workspace and development infrastructure

- Resolver-3 Cargo workspace, Rust 2024, shared metadata and strict safety lints.
- Seven publishable crates (below) and the unpublished `tools/stonecrab-testkit`. All
  publishable crates have empty or minimal default features (`stonecrab-transport/tcp` and
  `stonecrab-sensors/toml` are on by default, plan D10) and docs.rs metadata
  (`all-features`, `--cfg docsrs`). The facade archive no longer ships `tests/`.
- Local mise tasks that run the commands of the CI job or step of the same name
  (`lint-attrs`, `features`, `machete`, `dependencies`, `examples`, `interop`, `parity`,
  `package` and `dco`; `check`, `coverage` and `fuzz` map to `quality`/`test`, a `quality`
  step and `fuzz-smoke`, see the header of `mise.toml`).
  Scripts: `scripts/coverage-gate.sh`, `lint-attrs.sh`,
  `dco-check.sh`, `fuzz-smoke.sh` and `parity-check.sh` (ADR-0005 and its S31 addendum);
  `lint-attrs.sh` and `parity-check.sh` have `--self-test` fixture trees under
  `scripts/testdata/`. Package allow-lists for all seven crates are in `packaging/`.
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
  `fuzz_modbus_connection_string` and `fuzz_sensors_profile_toml`. `fuzz/deny.toml` must
  mirror every root `deny.toml` change; the smoke script refuses to run otherwise.
- `interop/pymodbus/`: the first-party pymodbus 3.8.6 peer (Dockerfile pinned by digest,
  `server.py`, `interop.sh`) behind the CI job `interop` and `mise run interop`
  (ADR-0015 decision 16).
- `profiles/synthetic/`: six synthetic sensor profiles (three for the pymodbus map under
  `modbus-tcp/`, three simulator twins under `sim/`) with `profiles/README.md`;
  repository data, not shipped in any crate.
- `fixtures/modbus/`: first-party golden vectors (`tcp/`: PDU, ADU and typed-decode
  vectors authored from the Modbus Application Protocol V1.1b or built to fail), the
  interop register map and `PROVENANCE.toml`. The upstream vectors are not in the
  repository (prerequisite P4).
- `upstream/manifest.toml`: the pinned Apache PLC4X baseline and the Modbus TCP inventory.

### `stonecrab` (facade) 0.2.0

Re-exports core. Behind the optional features `sim`, `modbus` and `sensors` (`full`
enables all three; all off by default) it re-exports `stonecrab-driver-sim` as
`stonecrab::sim`, `stonecrab-modbus` as `stonecrab::modbus` and `stonecrab-sensors` (with
its TOML loader) as `stonecrab::sensors`, each sibling pinned `=0.1.0`. Examples
`sim_read`, `modbus_tcp_read` and `profile_read` (each with `required-features`). No
executable lines of its own, so coverage reports it `n/a`.

### `stonecrab-core` 0.1.0

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
- Time and cancellation: `OpFuture`, the `Clock` trait (no implementation in core),
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

### `stonecrab-codec` 0.1.0

`BoundedReader`, `BoundedWriter`, `ParseError` with `From<ParseError> for PlcError`, coil
bit helpers (ADR-0013). Lengths are checked before allocation; nothing panics on input.
Checksums and nesting limits are deferred (M2).

### `stonecrab-transport` 0.1.0

The only crate with Tokio (scoped features, no spawned task; ADR-0014): `Transport` and
`TransportFactory` (a `Deadline` on every call, `shutdown(Deadline)`), `TcpTransport` and
`TcpFactory` (feature `tcp`), `FrameError` (transport-local, plan D22), `Framer` and
`FramedTransport` (`next_frame`, `send_frame`, `resync(Deadline)`), `ConnectionGuard` with
`GuardConfig` and `GuardStats` (lifecycle through core `ConnectionState`, request-driven
reconnect with capped jittered backoff, one in-flight slot with a bounded admission queue,
resync after a timeout, `close` bounded by `Timeouts.request`), `SystemClock` and `Jitter`.

### `stonecrab-driver-sim` 0.1.0

Tokio-free simulator (ADR-0012): `SimDriver` (scheme `sim`,
`sim://<device>?latency-ms=<n>&seed=<u64>`), `SimConnection`, `SimDevice`, `SimTag`,
`SimAddress` (`STATE|RANDOM|SINK/<name>:<type>`), `FailureScript` and `FailureStep`,
`RealtimeClock`. It reads and writes in-process devices, enforces the access policy, allows
one operation in flight, applies bounded read retry and never retries a write.

### `stonecrab-modbus` 0.1.0

Modbus TCP, reads only (ADR-0015; hand-written, nothing generated): the address types and
grammar (`ModbusAddress::parse`, the explicit-convention rule for bare `40001` forms, the
`{unit-id, byte-order}` block), the PDU codec for function codes 01 to 04 and the
exception PDU, MBAP framing (`MbapFramer`), `TypedDecoder` for the 15 types under the four
byte-order modes, `Batcher` (adjacent or overlapping merge only, 125 registers and 2000
bits per chunk), `ModbusSession` (reply accepted only when transaction id, unit id and
function code match), `ModbusConnection` and `ConnectionDefaults`, and `ModbusTcpDriver`
with `ModbusOptions` (the six upstream parameters with upstream defaults plus
`address-convention`; unknown keys are `InvalidConfig`). `write` is `Unsupported` with no
I/O. Deferred items are in "Known gaps".

### `stonecrab-sensors` 0.1.0

Profile schema version 1 and the TOML loader (feature `toml`, 64 KiB bound), semantic
validation collecting every error and the fixed unit table, `SensorProfile::tags(&dyn
Driver)` (declared convention passed as `AddressOptions`), and
`SensorProfile::measurements(&ReadResponse, wall_now)` with `Measurement` and `Quality`
(ADR-0016). Only synthetic profiles exist; no named device profile.

### `tools/stonecrab-testkit` (unpublished, development only)

`FixtureAddress`, `FakeClock`, `block_on`, `block_on_with`, `poll_once`, `CountingWaker`;
behind `net`: `Script`, `Step` (including `Respond` and `React`), `Reaction`,
`ScriptedPeer`, `ScriptedPeerFactory`, `FakeTransport`, `FakeTransportFactory`,
`CountingFactory`, `LengthPrefixedFramer` (the sanctioned doubles of CODING_STANDARDS
13.5); behind `fixtures`: the golden-vector loader and `InteropMap`; and the
`ContractSuite` every driver runs (23 cases; skips are named, never silent). Contract
results: simulator 22 passed and 1 skip; Modbus over the fake transport 21 and 2; over
loopback `ScriptedPeer` 18 and 5; against pymodbus 14 and 9.

### Tests

Unit tests, property tests, consumer integration tests, executable documentation (every
crate README is a doctest) and Send/Sync assertions cover the implemented contracts.
Mandatory failure cases are tagged `// covers: SPEC mandatory-failure-cases / F<n>` and
listed in `docs/COMPATIBILITY.md`. The README-vs-`Capabilities` tests
(`crates/stonecrab-driver-sim/tests/readme_matrix.rs`,
`crates/stonecrab-modbus/tests/readme_matrix.rs`) keep each driver README's support table
equal to `Driver::capabilities()`. The parity gate (`mise run parity`) keeps the parity
records and the sources honest about each other.

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
mise run fuzz
mise run examples
mise run interop     # needs Docker
mise run parity
mise run package
mise run dco -- origin/master..HEAD
```

Most recent local results for the implementation baseline (slices S01 to S30; S31 changed
documentation, one script and comments in Rust sources only):

- Formatting, strict Clippy, nextest, `cargo test` with doctests, documentation with
  warnings denied, no-default-features, lint-attrs, feature powerset and unused-
  dependency checks passed.
- Per-crate line coverage (ADR-0005): `stonecrab-core` 3577/3577 lines (gate 100%),
  `stonecrab-codec` 181/181 (gate 100%), `stonecrab-modbus` 2758/2758 (gate 100%),
  `stonecrab-driver-sim` 2051/2051 (gate 100%), `stonecrab-sensors` 1411/1411 (gate
  100%), `stonecrab-transport` 1216/1220 (gate 95%; the uncovered lines are the
  real-socket deadline-expiry and resolver-failure arms named in ADR-0014),
  `stonecrab-testkit` 4606/4679 (gate 90%), `stonecrab` n/a (no executable lines).
  Error paths are not excluded. Region coverage is reported in
  `target/coverage-summary.txt` and not gated; do not describe it as branch coverage.
- Workspace tests and doctests passed on Rust 1.97.1.
- Cross-compilation checks passed for `aarch64-unknown-linux-gnu` and
  `armv7-unknown-linux-gnueabihf`. These are compile checks, not device tests.
- Dependency advisories, licenses, bans and registry-source checks passed.
  Unused license-allowance warnings are currently emitted by cargo-deny.
- All seven publishable crate archives built and matched their committed allow-lists.
- The `interop` run (Docker, pymodbus 3.8.6) passed locally in S27 and S30: ten interop
  tests, the `modbus_tcp_read` example, the contract suite and `profile_read
  --expect-good` for each `modbus-tcp` profile. The `examples` run (the simulator
  profiles) passed in S30.
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

The latest hosted run recorded for this baseline is the one for `8f5c3d0` (S31), which
**completed successfully** (every job green except the report-only `semver`; `scheduled`
is skipped on push):
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40557968227
The run for `f19be4c` (S30, the definition of done on the hosted runner):
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40548582722
The earlier run, for `0429da3` (S27):
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40543567845

| Job | Result |
| --- | --- |
| `test`: ubuntu, macOS, Windows, each on Rust 1.97.1 and 1.99.1 (six cells; nextest, `cargo test` with doctests, no-default-features check) | green |
| `quality` (fmt, strict Clippy, docs with warnings denied, per-crate coverage gate) | green |
| `cross` (aarch64 and armv7 compile checks) | green |
| `dependencies`, `package`, `dco`, `features`, `machete`, `lint-attrs` | green |
| `geiger` (no first-party `unsafe` outside `forbid(unsafe_code)`), `beta` (report only) | green |
| `fuzz-smoke` (seven targets) | green |
| `parity` (`scripts/parity-check.sh` and its self-test) | green |
| `examples` (`sim_read` and the three `profile_read --expect-good` runs on the hosted runner) | green |
| `interop` (pymodbus 3.8.6 on the amd64 runner: ten interop tests, the `modbus_tcp_read` example, the `profiles` step, the `interop-evidence` artifact) | green |
| `semver` (report only) | fails until the sibling crates exist on crates.io (no baseline); does not fail the run |

The run for the S32 commit `2d38e48` (release evidence) also completed successfully:
https://github.com/TheMadBotterINC/stonecrab/actions/runs/40584025718
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
per-crate coverage tiers, the CI gates and, in its S31 addendum, the parity gate.

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
  tests), and the README tables must stay equal to the `Capabilities` impls.
- Partial and all-item failures remain valid response data. Do not collapse them
  into a batch error or invent a device timestamp (`source_time` is `None` unless a device
  supplies one).
- Never turn an unknown write outcome into a rejection or automatically replay it.
- Address methods must return stable, credential-free metadata. Do not add an
  `as_any` shim; stable trait upcasting is already used.
- Reconnect is request-driven (plan D15): no background task, no `tokio::spawn` in a
  published crate (`clippy.toml` disallows it; the testkit's `ScriptedPeer` owns the one
  spawn with the required `#[expect]`).
- The word `unsafe` may not appear in any first-party Rust file except the
  `forbid(unsafe_code)` attribute: the `geiger` job greps for it, comments and strings
  included.
- Core's testkit dependency is development-only. The path-only dependency policy
  exception is documented in ADR-0003; registry wildcard versions remain denied.
- Core integration tests using the unpublished testkit are workspace-only and not
  shipped in the core archive. Cargo emits an expected ignored-test warning while
  packaging. Packaged core unit tests and documentation remain self-contained.
- `mise run package` uses a **fresh Cargo cache**. Repeatedly verifying changed,
  unpublished sibling crates with the same version in a reused cache previously
  selected an older core archive. Do not remove the isolation as a cosmetic change.
- Update the matching `packaging/<crate>.txt` allow-list when shipped files change, and
  review the resulting archive contents.
- New dependencies require a rationale, the applicable audit/ADR entry and the D23
  `cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono` check.
  Runtime dependencies are `thiserror`, `bytes`, `tracing` (std only; `once_cell` reaches
  the graph through `tracing-core` under a recorded deny.toml wrapper), `serde` and `toml`
  in the sensors crate, and `tokio` with scoped features in the transport crate only;
  dev-only `proptest`, `tracing-subscriber` (the examples; `lazy_static` reaches the graph
  through `sharded-slab` under a second recorded wrapper) and, in the testkit,
  `serde_json`. `toml_parser` is pinned in `Cargo.lock` so that only one `winnow`
  resolves; a plain `cargo update` must keep that pin or allow the duplicate.
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
- `inventory_complete = false`: the open items in the manifest `[pending]` table are:
  1. Complete protocol-by-transport-by-operation source inventory.
  2. Inventory capabilities in non-Java implementations.
  3. Map every required behavior to a planned test ID.
  4. Review all configuration and address aliases before implementation.
  5. Inventory the Modbus RTU/ASCII connections (the `rtu/` and `ascii/` packages were
     not read).
  6. Inspect the generated `DataItem` behavior for the TIME/DATE family (generated code
     is not in the repository).
  7. Confirm which XML the little-endian IT loads and the test transport internals.
  8. Compare the XML-serialized `ModbusADU` step format of the all-datatypes suites with
     `DriverTestsuite.xml`.
  9. Check the Modbus Application Protocol V1.1b section numbers cited in
     `docs/parity/modbus.md` against a retained copy.

**M0 is not complete.** The Modbus TCP subset is inventoried: `docs/parity/modbus.md`
carries stable row ids with test ids, a divergence table with references and an UNVERIFIED
list; the manifest records the inventoried files and line ranges under
`[inventory.modbus_tcp]`. None of the open items gates the alpha. Every divergence row
now cites a protocol section, a drafted upstream issue (`docs/evidence/upstream-issues/`,
not filed: filing needs the maintainer's approval) or the reason none was drafted.

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
  30 minutes per target), benchmarks, differential tests against PLC4J, branch coverage
  (region coverage is reported), `cargo publish --dry-run` and a blocking `semver` job
  (no published baseline).
- **Upstream vectors**: the Apache-2.0 PLC4X test vectors are not redistributed (P4), so
  every test vector in the repository is first-party. This keeps the four byte-order mode
  rows at `implemented` instead of `automated-verified`.
- **Not verified by the interop run**: timing over a real clock, dropped links and device
  exceptions on demand (a stock pymodbus has no fault hooks), unit-id correlation, and any
  server other than pymodbus 3.8.6.
- A queued-cancellation case is exercised on `ConnectionGuard` only, not on the simulator
  (which has no wait queue); the real-socket connect-timeout arm of the transport is
  uncovered and named in ADR-0014.

### Maintainer prerequisites still owed

These are actions outside the code (plan P1 to P5):

- **P1** enable GitHub private vulnerability reporting (it was `enabled: false`) and verify
  it with a test report. `SECURITY.md` still says the channel is a release prerequisite.
- **P2** enable secret scanning and push protection.
- **P3** add a `master` ruleset (required checks, linear history, squash-only, no
  force-push) before accepting external pull requests.
- **P4** give written approval to redistribute the Apache-2.0 PLC4X Modbus test vectors
  with NOTICE attribution. It blocks **S04a** and **S04b** (the upstream vector copies, the
  adapter and the upstream-driven tests).
- **P5** (the single-maintainer waiver) is done and recorded in ADR-0005.

## Recommended continuation

The alpha plan is executed (S01 to S32; S04a and S04b wait for P4). What remains is the
maintainer's: nothing below is done by an automated run, and nothing is published,
tagged or posted without separate approval.

1. Run the `release-evidence` workflow once (`workflow_dispatch`) and attach its
   artifact to the draft pre-release when one exists.
2. Close the maintainer prerequisites P1 to P3 (repository settings) and decide P4 (the
   upstream vector redistribution), which unblocks S04a and S04b and the
   `automated-verified` status of the byte-order rows.
3. Decide whether to file the six drafted upstream issues under
   `docs/evidence/upstream-issues/` on the Apache PLC4X tracker and record the URLs in
   the divergence table of `docs/parity/modbus.md`.
4. Release per ADR-0017: run the alpha release checklist in `docs/evidence/alpha.md`,
   `cargo publish --dry-run` per crate in dependency order with a fresh `CARGO_HOME`,
   create and push the signed tags `<crate>-v<version>`, then publish core, codec,
   transport, driver-sim, modbus, sensors and the facade in that order. Harmonise the
   facade's `stonecrab-core` range to `=0.1.0` first if CODING_STANDARDS 20.1 is to be
   read strictly (recorded in ADR-0017).
5. After publication: make `semver` blocking, re-run `scheduled` (30-minute fuzz,
   mutation scope, advisory dry run) and plan M2 (writes, Modbus RTU and ASCII, board
   adapters) from `docs/ALPHA_PLAN.md`'s deferred items.

Keep changes reviewable, commit tested slices one signed commit per green slice, and
maintain the per-crate coverage gates. No first-party unsafe, panics on input, unbounded
I/O, silent sample loss, inferred byte order or automatic replay of ambiguous writes. Do
not mark planned capabilities supported.

Do not publish another crate version or operate hardware without separate approval.
Never place credentials in files, logs, commit messages or handoff documents.
