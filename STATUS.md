# Stonecrab status and engineering handoff

Updated: 2026-10-29.
Implementation baseline: the commit that last modified this file
(`git log -1 --format=%h -- STATUS.md`). Every alpha slice updates this file in
its own commit; the slice log below records what each one landed.

## Start here

Stonecrab is an independent native Rust port of Apache PLC4X, prioritizing sensor
acquisition, and ships as **one crate** (slice S33, ADR-0018). The alpha defined in
[`docs/ALPHA_PLAN.md`](docs/ALPHA_PLAN.md) was built and published as `stonecrab` 0.2.0
(release evidence and process: slice S32, ADR-0017, `docs/evidence/alpha.md`). The
hardware-free slices of [`docs/POST_ALPHA_PLAN.md`](docs/POST_ALPHA_PLAN.md) followed and
were published as `stonecrab` 0.3.0 on 2026-10-29 (decision E12), tagged `v0.3.0`. 0.2.0
was never tagged, there is no GitHub release, and nothing is posted upstream: those steps
are the maintainer's (see "Recommended continuation").

- Repository: https://github.com/TheMadBotterINC/stonecrab
- Primary/default branch: **`master`**, locally and remotely.
- Author: **The Mad Botter INC**.
- License: **Apache-2.0**.
- Published on crates.io: **`stonecrab` 0.2.0**, the alpha, on 2026-10-27 from commit
  `640665a` after the maintainer's explicit approval (hosted run 40612938390 green);
  `stonecrab` 0.1.0, the specification-stage placeholder, stays listed as an old version.
- Published on crates.io: **`stonecrab` 0.3.0** on 2026-10-29 at 13:51 UTC (decision E12)
  from commit `66ae100` (hosted run 40884031994 green), tagged `v0.3.0` (annotated tag
  object `7a2439f`; not signed, since no signing key is configured). It is the version
  since slice S74, because the sensor `unit` became `Option<String>` (G06), a breaking
  change; CHANGELOG.md has its section and the migration note. The next breaking change
  moves master to 0.4.0 with its own note (CODING_STANDARDS 9.10).
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
  F32, F64, Char, String, Duration, and since S53 the calendar values SignedDuration, Date,
  TimeOfDay and DateTime over `PlcSignedDuration`, `PlcDate`, `PlcTimeOfDay` and
  `PlcDateTime`, ADR-0021; integer access never silently converts counters to floating
  point), `TagId`, `ProtocolAddress` (object safe, `Any + Debug + Send + Sync`),
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
  returns `Unsupported(Subscribe)` until a driver overrides it. Since S55 the simulator and
  Modbus subscribe by polling (private `polling` module): the subscription samples its tags
  through the driver's own read path while its consumer waits in `recv`, skips and counts
  ticks it could not take (`LossCounters::skipped`), and refuses Event, and change-of-state
  on a clock that never sleeps, before any I/O.
- The connection cache (S68, ADR-0030; the first PLC4X tool of decision E6):
  `ConnectionCache`, `ConnectionCacheBuilder` and `LeasedConnection` lease one connection
  per connection string at a time, queue the other callers in arrival order up to
  `max_pending_requests`, and evaluate the idle, lease, wait, ping and close bounds on the
  injected clock when called, with upstream's defaults and no spawned task. A lease is
  itself a `Connection` that refuses calls once it expired or was given back; an idle
  connection is pinged before reuse, and a failed ping or a broken link makes the next
  lease replace it. Subscription restore after a reconnect (`TOOL-CC-RESTORE`) is deferred.
- The event pump (S69, ADR-0031; the second PLC4X tool of decision E6): `EventPump`,
  `TagBatch` and `BatchEvent` read named batches (one `ReadRequest` each, on a borrowed
  connection or a lease from the connection cache) on a fixed tick grid after an initial
  delay, skip and count a tick that falls due during a fetch, back off after a failure as
  upstream does (1 s doubling to 60 s), and hand results out of `recv()`, which drives
  every started batch on the injected clock with no spawned task and holds at most one
  event per batch. Unlike a subscription's `recv`, a dropped `recv` keeps the reads in
  flight (other batches' reads must survive it). Configuration files and value
  transforms are deferred; upstream's subscription trigger is a placeholder and is not
  ported.

### `stonecrab::codec`, always built

`BoundedReader`, `BoundedWriter`, `ParseError` with `From<ParseError> for PlcError`, coil
bit helpers (ADR-0013). Lengths are checked before allocation; nothing panics on input.
Checksums and nesting limits are deferred (M2).

### `stonecrab::transport`, feature `transport`

The only module with Tokio, apart from the test kit (scoped features, no spawned task; ADR-0014): `Transport` and
`TransportFactory` (a `Deadline` on every call, `shutdown(Deadline)`), `TcpTransport` and
`TcpFactory`, `FrameError` (transport-local, plan D22), `Framer` and
`FramedTransport` (`next_frame`, `send_frame`, `resync(Deadline)`), `ConnectionGuard` with
`GuardConfig` and `GuardStats` (lifecycle through core `ConnectionState`, reconnect with
capped jittered backoff, one in-flight slot with a bounded admission queue, resync after a
timeout, `close` bounded by `Timeouts.request`; since S57 an opt-in supervisor task,
started by `ConnectionGuard::supervise` inside a Tokio runtime and used by every Modbus
connection, notices a drop and reconnects without waiting for a request and enforces
`Timeouts.idle` by closing an unused link), `SystemClock` and `Jitter`.
Behind the `serial` feature (S58), which `modbus` enables since S59: `SerialFactory` and
`SerialTransport` on the `serial2` crate, one blocking worker thread per port talking to
async callers over a bounded channel, every call bounded by its deadline, `recv`
cancel-safe through a worker-side stash. Since S59 a framer may ask `resync` to drop the
whole buffer instead of searching it (`Framer::resync_discards`, the RTU framer does).
Since S61 (ADR-0027) `UdpFactory` and `UdpTransport`: one connected socket per connection,
one whole datagram per `recv` (`Transport::delivers_datagrams`, so `FramedTransport` never
merges boundaries), an oversize datagram consumed and reported as `ResourceLimit`, a send
bound of 65 507 bytes, and no `udp.*` option yet.

### `stonecrab::sim`, feature `sim`

Tokio-free simulator (ADR-0012): `SimDriver` (scheme `sim`,
`sim://<device>?latency-ms=<n>&seed=<u64>`), `SimConnection`, `SimDevice`, `SimTag`,
`SimAddress` (`STATE|RANDOM|SINK/<name>:<type>`), `FailureScript` and `FailureStep`,
`RealtimeClock`. It reads and writes in-process devices, enforces the access policy, allows
one operation in flight, applies bounded read retry and never retries a write.

### `stonecrab::modbus`, feature `modbus`

Modbus TCP (ADR-0015; hand-written, nothing generated): the address types and grammar
(`ModbusAddress::parse`, the explicit-convention rule for bare `40001` forms with the
`plc4x`, `one-based-40001` and `zero-based` conventions of S51, the `{unit-id, byte-order}`
block, the array selections `[i]`, `[lo..hi]` and `;base` of S48 (ADR-0022) and the
extended registers of S50, selections included), the PDU codec for function codes 01 to 06,
15, 16, 20 and 21 and the exception PDU, MBAP framing (`MbapFramer`), `TypedDecoder` for the
15 types under the four byte-order modes and for lists of them, `Batcher` (adjacent or
overlapping merge only; 125 registers, 120 extended registers and 2000 bits per chunk),
`ModbusSession` (reply accepted only when transaction id, unit id and function code match),
`ModbusConnection` and `ConnectionDefaults`, and `ModbusTcpDriver` with `ModbusOptions` (the
six upstream parameters with upstream defaults plus `address-convention` and `access`;
unknown keys are `InvalidConfig`). `write` (S46) is refused unless the connection string
says `access=read-write`, then sends one request per item (FC15 for coils, FC06 or FC16 for
holding registers, FC21 cut at the file ends for extended registers): an echo is
`Confirmed`, an exception `Rejected`, a frame sent without a usable answer `Unknown` and
never replayed, and the items after an `Unknown` are not sent. `ping` reads the configured
`ping-address` once (S47): any correlated reply, an exception included, is success, as
upstream does (quirk d). `subscribe` polls (S55), and a connection opened inside a Tokio
runtime reconnects in the background and enforces `Timeouts.idle` (S57). Extended registers
are `implemented`, not `automated-verified`: no interop peer serves FC20 or FC21. Since S49
the text types `CHAR`, `WCHAR`, `STRING(n)` and `WSTRING(n)` read and write on the register
areas (ADR-0025): UTF-8 and UTF-16BE, cut at the first NUL on read, NUL-padded on write,
an invalid encoding an item's `MalformedData` with the registers kept, the byte swaps of
the two `*_BYTE_SWAP` orders applied per register, and selections starting at an odd byte
refused.

Modbus RTU since S59 (ADR-0024): `ModbusRtuDriver` for the `modbus-rtu` scheme on a serial
port (`modbus-rtu:///dev/ttyUSB0`, the default) or a serial-to-IP gateway
(`modbus-rtu:tcp://`, port 502 by default), `RtuFramer` and `RtuAdu` with CRC-16 (sized by
upstream's function-code table, noise in front of a whole frame skipped, a whole-buffer
drop on resync), the silence before each request (t3.5, at least 1.75 ms above 19200 baud,
or `serial.interframe-delay`) and a drain after an unanswered request, correlation by unit
and function code, one request in flight per port, and the TCP driver's read, write
(extended registers and selections included), ping and polling-subscribe paths shared
through a crate-private wire choice. A CRC error fails the request at once (a read may be
retried, a write is `Unknown`), where upstream waits for the deadline; broadcast (unit 0),
shared ports, data bits other than 8, mark and space parity and the TLS, TLS-PSK, UDP and
`test` carriers are `InvalidConfig`. The evidence is `simulator-tested`: the fake
transport, the contract suite (30 passed, 3 skipped), a loopback gateway and a fuzz target;
no independent RTU server and no device yet. Deferred items are in "Known gaps".

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
`ContractSuite` every driver runs (33 cases since S55; skips are named, never silent), which driver
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
| S36, S45 (fix) | 2026-10-28 | The blocking `semver` job failed on run 40793164568: S45 inserted the write variants and S36 `InvalidRange` in the middle of the published `ModbusPdu` and `ProfileError` enums, shifting implicit discriminants (`enum_no_repr_variant_discriminant_changed`). The new variants now follow the published ones (no behavior change); CODING_STANDARDS 9.10 says so, and cargo-semver-checks 0.51.0 is pinned in mise with a `semver` task so the check runs before a push | check, coverage, semver (local, cargo-semver-checks 0.51.0) |
| S42 | 2026-10-28 | Non-Java inventory of PLC4X v1.0.0 from the pinned checkout: 39 more `[inventory.*]` entries (22 Go: the SPI, five transports, 14 drivers, two tools; 9 C: SPI, four drivers, three transports, the plc4x-server stub; 2 C#: generated KNXnet/IP model only, no driver; 6 Python: SPI, mock, Modbus, UMAS, two transports), each compared with its Java entry, and a `[[backlog]]` table of 52 capabilities that exist outside Java and are absent or weaker in Java (for example Go BACnet/IP read, write, discovery and COV, Go C-Bus, Python UMAS system-bit addresses). Pending item "non-Java implementations" closed; the plan's section 4 cells that waited on S42 are filled; S41's plc4x-server citation corrected; docs only | TOML parse, ASCII check, 90 spot checks (three Go claims dropped, one citation corrected) |
| S43 | 2026-10-28 | One parity record per upstream protocol from the S41 and S42 inventory: 19 protocol files (S7, PROFINET, EtherNet/IP with Logix, ADS, AB-ETH, OPC UA, Modbus RTU and ASCII, SLMP, UMAS, the PLC4X proxy, BACnet/IP, KNXnet/IP, C-Bus, CANopen, generic CAN, IEC 60870-5-104, Firmata, Open-Protocol with DF1, ctrlX) plus `tools.md` and `integrations.md` (E6's tracked list), upstream transport rows in `transport.md`, and the release matrix in `docs/COMPATIBILITY.md`; rows are planned (milestone) or deferred (reason), and every planned row names a `planned:` test id (pending item "Map every required behavior" closed). KNXnet/IP moved to M5 because a working Java driver exists (the plan's "Non-Java backlog" came from the corrected website lead); every manifest entry now names its parity record; docs only | parity (and its self-test), TOML parse, row-shape and unique-id checks |
| S54 | 2026-10-28 | The subscription API (findings G07 subscribe part and G10), ADR-0020: `SubscribeRequest::builder(limits, kind, queue)` like `ReadRequest` (tags, deadline and cancel bound only the call; refuses empty, duplicate, over-budget, zero-interval and over-wide coalescing requests), `SubscriptionKind`, `QueuePolicy` (backpressure never loses, coalescing keeps the latest per tag and counts it, dropping counts every loss), `Subscription` (handle and stream: `recv`, `mode`, `interval`, `loss`, `unsubscribe`; drop ends it), `SubscriptionSender` for producers, a queue bounded by `max_queued_events` without executor types, and a provided `Connection::subscribe` returning `Unsupported(Subscribe)` with diagnostics (ping and subscribe share one private helper). Proptests per policy (bound, conservation, order, latest per tag); the contract suite has 27 cases (an unsupported subscribe performs no I/O for every policy and kind) | check, coverage, lint-attrs, parity, examples |
| S46 | 2026-10-28 | Modbus writes on the wire: `ModbusConnection::write` refuses the default read-only policy first with no I/O; a new Stonecrab connection-string parameter `access=read-only|read-write` (default read-only) grants writes (ADR-0008, ADR-0010 and ADR-0015 addenda). One request per item as upstream sends them (`ModbusPdu::write_request`, the typed encoder with tag over block over default order), through the read slot, session and deadline rule; an echo is `Confirmed`, an exception `Rejected` with its protocol status, a sent frame without a usable answer `Unknown` (never retried or replayed, including upstream's FC06-request/FC16-response test pairing, which never confirms), and the items after an `Unknown` are not sent (MB-WRITE-HALT, a recorded divergence). `Capabilities.write` is true. The contract write cases run for every Modbus fixture; 60 upstream write exchanges become vectors through the S04b adapter and Stonecrab's encoder reproduces their request bytes; three pymodbus interop write tests (interop expects 15) | check, coverage, lint-attrs, parity, examples, interop |
| S43 (fix) | 2026-10-28 | The hosted `parity` job failed on run 40798426129 with "printf: write error: Broken pipe": `scripts/parity-check.sh` piped the whole row list into an awk that exits at the first match, and once S43's records outgrew the pipe buffer the early exit broke the pipe under `set -o pipefail`. The awk now reads all its input and answers at END; macOS had not tripped it | parity and its self-test; the failure reproduced locally with a 2 MB row list (exit 141) and passes after the fix |
| S37 | 2026-10-28 | Status-bit and alias validation (findings G02, G03): `validate` refuses a status bit of 64 or more (`StatusBitOutOfRange`); `validate_for` parses status-field addresses once (one `StatusAddressRejected` mapping shared with `tags`) and reports a bit beyond the parsed type's width, `OverlappingStatusBits` for two spellings of one canonical address and bit (equal text stays `validate`'s), and `StatusBitInBitField` when a status bit lies in the measurement's first-step `BitField` range on the same canonical address; measurements may still share a status word. Two new `ProfileError` variants, appended after the published ones; ADR-0016 addendum defines the shapes and their limits (canonical text includes the type and unit id); 18 tests in `sensors_overlap.rs` and a fuzz seed | check, coverage, lint-attrs, parity, examples |
| S52 | 2026-10-28 | Benchmarks: criterion 0.8.2 as a dev-dependency only (default features off: no rayon or plotters; ADR-0009 addendum, DEPENDENCY_AUDIT entry, policy check clean; a C compiler is needed to build the benches); `benches/modbus_wire.rs` (address and connection-string parse, PDU and MBAP encode and decode, TypedDecoder over 62 REAL and 125 UINT, Batcher plans) and `benches/modbus_loopback.rs` (reads through the registry and the real TCP factory to a testkit ScriptedPeer on 127.0.0.1). Baseline on an Apple M2 Max recorded in `docs/evidence/benchmarks/` (a laptop baseline that the gate never reads). The 10% gate (`scripts/bench-compare.sh`, `mise run bench-compare`) builds a base and HEAD on the same runner, runs three alternating rounds and fails only when every round shows HEAD's lower bound above 1.10 times the base's upper bound; it runs in the weekly `scheduled` job only | check, lint-attrs, dependencies, machete, bench --no-run, one full local bench run, bench-compare self-test and an injected-slowdown trial |
| S54 (fix) | 2026-10-28 | `packaging/stonecrab.txt`, the reviewed archive list, gains `src/subscribe_request.rs` and `src/subscription.rs`, which S54 added; the `package` gate found the gap before the push | package |
| S44 | 2026-10-28 | The Modbus-specific pending items: TIME/DATE settled (the item's premise was wrong: the generated Java, Go and Python `DataItem` code is in the pinned checkout, and `modbus.mspec` has no TIME/DATE-family case, so upstream reads such tags as OK with a null value and throws on write; 8 bytes, 4 registers per element; recorded in the Data types rows and MB-QUIRK-K); the upstream test harness read end to end (whole-frame injection through the `test` transport, model comparison, the witness limits of the driver-suite vectors; a harness table under "Test vectors available upstream"); the Modbus part of the alias review closed with new rows (transport option prefixes, option value aliases, the config block, legacy array forms) and corrected default ports and a wrong URL-decoding claim in `core.md`; 20 evidence keys added to `[inventory.modbus_tcp]`. The spec-section item stays: modbus.org refused the automated download (HTTP 403), so it waits for a manual copy; docs only | parity, TOML parse; every finding re-opened in the pinned checkout before use |
| S58 | 2026-10-28 | Serial transport on a blocking worker: `serial2` 0.2.39 (BSD-2-Clause OR Apache-2.0; a first attempt on `serialport` needed an MPL-2.0 exception the maintainer had not granted and never landed) behind the optional `serial` feature; one named std thread per port owns it, async callers send commands over a bounded channel (capacity 2) and race each answer against their deadline, writes go out in 256-byte chunks with 20 ms port timeouts so a stalled line ends at the caller's deadline, a zero-byte write is retried, and received bytes live in a stash so a dropped `recv` loses nothing; close joins the worker. Tests run on a pseudo-terminal pair on Linux (serial2 cannot make one on macOS, ENOTTY), in-memory worker tests everywhere; ADR-0014 addendum, DEPENDENCY_AUDIT entry, CODING_STANDARDS names serial2 | check, coverage (transport 98.81% locally), lint-attrs, dependencies (deny unchanged), machete, semver, package list, no-default-features with and without serial |
| S51 | 2026-10-28 | The gap question and two address conventions: MB-READ-GAP decided as a divergence with evidence (upstream merges unconditionally while the span fits, reading the holes between tags, with no option of its own; matching it would turn a hole in a device map into exception 02 for every tag of the merged request), so the batcher keeps adjacent-or-overlapping merging and a new test pins that a hole is never read; `AddressConvention` gains `ZeroBased` and `OneBased40001` (exact definitions in the ADR-0015 addendum; `OneBased40001` reads numbers as the plc4x convention does), accepted by the address grammar, the connection string and sensor profiles; the address fuzz target reads every input under each convention, with five new seeds | check, coverage, lint-attrs, parity, examples, semver, fuzz_modbus_address 20 s |
| S52a | 2026-10-28 | A second independent interop peer: libmodbus 3.1.11's `unit-test-server`, built in a two-stage image from the sources Debian's `libmodbus-dev` 3.1.11-2 ships (base `debian:trixie-slim` pinned by digest, the two source files checked by sha256, non-root, port 1502, restarted after every connection by `serve.sh`); nothing of libmodbus is copied, the `unit-test.h` constants are transcribed with their BSD-3-Clause source. Ten tests (`modbus_interop_libmodbus.rs`, a new CI job `interop-libmodbus` and `mise run interop` for both peers): reads at unit 17, FC15, FC06 and FC16 writes read back (REAL under all four byte orders), exception 06 at 0x170, the wrong-TID-and-unit reply at 0x171 discarded with the session usable, the 500 ms reply within and beyond a deadline, a byte-at-a-time reply reassembled, and the short FC03 reply as `MalformedData` (without raw bytes: a recorded gap); the shared `Peer` helper moved to `tests/support/interop.rs` | check, coverage, lint-attrs, parity, semver, interop for both peers (80 of 80 repeated local runs) |
| S58 (fix) | 2026-10-28 | The first hosted run with S58 (40806708069) failed in its Linux-only by-name test: it asked a pseudo-terminal for 7E2, and Linux's pty driver forces eight data bits and no parity (`pty_set_termios`), so serial2 reported "failed to apply some or all settings". Checked with termios in a Linux container: a pty keeps the baud rate, two stop bits and XON/XOFF. The test now opens 19200 8N2 XON/XOFF, and a new test pins that a setting the port refuses fails the open as `Disconnected` | fmt; clippy of the Linux-only tests with their cfg widened on macOS; the termios behavior checked in a Linux container |
| S55 | 2026-10-28 | Polling subscriptions for the simulator and Modbus (`SubscriptionMode::Polling`): a private `polling` module plans the subscription before any I/O (Event refused; ChangeOfState refused on a clock that never sleeps; cyclic at the request's interval, change-of-state at upstream's 1000 ms) and the `Subscription` drives its producer while its consumer waits in `recv` (no spawned task: the core has no executor and the contract suite runs on `block_on_with`). Ticks sample on a grid through the connection's read path and skip, counted in `LossCounters::skipped`, when the slot is busy, the link is being reopened or nobody waits; `recv` takes `&mut self` (two pending receives do not compile); a panic in the producer ends it without aborting. Both connections hold their state in a private `Arc<Link>` so the producer never outlives them. Six contract cases (33); ADR-0020 addendum; the verification pass found a change-of-state spin, lost wakeups between concurrent receives and doc overstatements, all fixed | check, coverage (core, modbus and sim 100%), lint-attrs, parity, examples, semver, package list, interop; an independent verification pass (concurrency and design lenses), a fix round and a recheck |
| S57 | 2026-10-28 | Eager reconnect and the idle timeout: `ConnectionGuard::supervise` (opt-in, Tokio runtime only, bounds kept off the published constructors so the change stays additive) starts a supervised task owned by the guard (`close` joins it within `timeouts.request`, drop aborts it) that, while nothing is requested, holds the slot and reads the stream so a peer close is noticed, reconnects per `ReconnectPolicy` on the guard's clock, ends in `Failed` when the budget is spent, and yields to requests through a `Notify`; a request and its reply are correlated under one hold. `Timeouts.idle` closes a link no request used and the next request reopens it (state stays `Connected`). The Modbus connection opts in. ADR-0014 addendum records the deviation from D15; the verification pass restored ADR history the slice had deleted and fixed a parked frame that suspended the idle bound and drop detection; frames the reader drops are not yet counted (Known gaps) | check, coverage (transport 99.73%), lint-attrs, parity, examples, semver; an independent verification pass, a fix round and a recheck |
| S53 | 2026-10-28 | Calendar values (finding G08, calendar part), ADR-0021: IEC 61131-3's nine calendar names collapse into four representations, appended to `PlcValue` and `PlcValueType`: `SignedDuration` (`PlcSignedDuration`, i64 nanoseconds; TIME, LTIME), `Date` (`PlcDate`, i32 days since 1970-01-01, proleptic Gregorian; DATE, LDATE), `TimeOfDay` (`PlcTimeOfDay`, no leap second; TIME_OF_DAY, LTIME_OF_DAY) and `DateTime` (`PlcDateTime`, no zone; DATE_AND_TIME family), with range-checked constructors (`InvalidConfig`), chronological `Ord` and java.time `Display`; no dependency (chrono is banned; Hinnant's civil-date algorithm). No driver decodes them yet: the Modbus TIME/DATE rows stay deferred as an upstream placeholder (S44 found no upstream DataItem case) | check, coverage (core 100%), lint-attrs, parity, examples, semver, package list |
| S52d | 2026-10-28 | HW1 and HW2 as replays: documented-map fixtures `fixtures/modbus/devices/sio-mb04thms.toml` and `slm1-c.toml` (every register, access, default and limit checked against the vendor PDFs, pinned by URL and sha256 in PROVENANCE and never committed) served by the test responder through a reactor that enforces each device's limits and function table; documented profiles under `profiles/documented/` (four thermocouple temperatures with their break bits, and the SOLO PV), labeled `validation = "documented"`; replay tests `modbus_device_replays.rs` and `sensors_profiles.rs`; runbooks `docs/hardware/hw1-sio-mb04thms.md` and `hw2-slm1-c.md` listing setup, the acceptance steps and every open item only hardware can settle (operating the hardware needs the maintainer's approval). Findings: SOLO bits need FC05, which Stonecrab does not emit; PV sentinels map to `Uncertain` until the profile schema can match values | check, coverage, lint-attrs, parity, examples; every register checked against the vendor PDFs |
| S52c | 2026-10-28 | A third interop peer that answers by unit id: a first-party Apache-2.0 script `interop/modbus-tk/server.py` on modbus-tk 1.1.5 (LGPL-2.1-or-later; its dependency pyserial 3.5 is pinned too) in a `python:3.12-slim` image pinned by digest, serving units 1, 2 and 247 with a map each on port 5030 and no reply to any other unit. Eight tests (`modbus_interop_modbus_tk.rs`, a new CI job `interop-modbus-tk`; `mise run interop` now runs all three peers): each unit's map on one connection, one batch across units, alternating units and function codes, exception 02 from the unit with the shorter map, units 9 and 255 ending in `Timeout` at the deadline with the connection still good, a batch with an unknown unit, the address-block unit over `default-unit-identifier`, and a write to one unit. The server is first-party, so the evidence is `interoperability-tested` at most; the discard of a wrong-unit reply stays with libmodbus and the scripted peers | check, lint-attrs, parity, semver, fmt, interop for all three peers |
| S52b | 2026-10-28 | Failure paths on a real clock: Toxiproxy 2.13.0 (MIT, pinned by digest, API and proxies on 127.0.0.1 only) in front of pymodbus and libmodbus on a Docker network of their own, toxics driven from the test process over the Toxiproxy HTTP API by a small `std::net` client (no dependency). Ten tests (`modbus_interop_toxiproxy.rs`, CI job `interop-toxiproxy`, `mise run interop-toxiproxy`): resets in each direction, black holes (`Timeout` at the deadline, twice, then recovery), latency below and above a deadline, slicer toxics, `limit_data` after one reply and inside a header, a disabled proxy, and `docker pause`, `kill` and `start` on pymodbus; each ends in `Timeout` or `Disconnected` within its deadline and the next request reads. tshark 4.4 (Debian trixie, unpinned by package version, recorded per run) captures in the proxy's namespace and fails on a malformed packet or a request that is not one well-formed Modbus/TCP frame. Finding: removing a timeout-0 toxic closes the connection | check, lint-attrs, parity, semver, interop for every peer; at landing the Toxiproxy run again on master with S57's supervisor (10 of 10, capture clean) |
| parity gate | 2026-10-28 | The parity gate gains a fourth check: every backticked snake_case test name in the test-id column of an implemented or automated-verified row, or of a divergence row, must be a word of a Rust source, example or fuzz target (ADR-0005 addendum). Two stale names it would have caught were fixed first (the S58 Linux serial test, the S52b Toxiproxy reset test); all 1,301 cited names resolve. Also: CI moves to actions/checkout@v5 and actions/upload-artifact@v6, the first majors that run on Node 24 (ab87b0b) | parity and its self-test (9 fixtures) |
| S57b | 2026-10-28 | The flaky `under_a_zero_idle_bound_a_read_keeps_the_reply_behind_a_mismatched_one` (about 3.5% alone, 10% under `cargo test`): on a multi-thread runtime a `close` that overtook the supervisor's idle close cut the shutdown of a transport already taken out of the slot, and the transport was dropped without a shutdown. The idle close now shuts the transport down where it is (`FramedTransport::shutdown`, crate-private) and empties the slot only afterwards, so `close` shuts down what it finds; a deterministic in-crate test reproduces the interleaving. The rustdoc of `ConnectionGuard` gains a Shutdown section (a transport lost to a failure is dropped without one). `GuardStats::discarded_frames` (additive) counts every frame the guard reads and drops between exchanges, closing the S57 gap "frames dropped between exchanges are uncounted"; `ModbusConnection` still exposes no stats | check, coverage (transport 98.97%), lint-attrs, parity, examples, semver; the formerly flaky test 300 of 300 alone and 200 of 200 under load, the full suite 10 of 10 |
| fuzz-smoke (fix) | 2026-10-29 | The hosted `fuzz-smoke` job failed on run 40814857692: `fuzz_modbus_connection_string` asserts that a rejection never repeats a non-ASCII authority or value, and a 33-character unknown key, echoed as its first 32 characters and `...`, ended in `oӨ` while a value was `oӨ.`. The library repeated nothing; the oracle was too coarse. It now requires the quoted text to begin a key and looks for the authority and values only outside the quotes (the templates hold no other quote), which also drops the old exception for values found inside a key; the input is kept as the seed `regression_value_after_shortened_key` | the target over its corpus for 180 s on the pinned nightly (4.76 million runs, no finding) |
| S48 | 2026-10-29 | Lists and Modbus array selections (finding G08, the list part), ADR-0022: `PlcValue::List(Vec<PlcValue>)` and `PlcValueType::List` appended (a range address carries the List hint, so the existing write builder refuses a scalar to a range and a list to an index before any I/O); the grammar `[i]`, inclusive `[lo..hi]` and `;base` on the long, Nx and bare forms (one dimension; numbers checked into u32; an index below the base or hi below lo refused); `[i]` resolves to the element's own address, a range sets `ModbusAddress::elements()` and is one tag and one read item whose whole span must end by wire 65535 and fit one read (2000 bits, 125 registers); byte and word order per element; a write takes a List of exactly the element count (FC15, FC16, FC06 for one register) and a list wider than one PDU is refused with `ResourceLimit`; the legacy-form hint is offered only when it parses; `ping-address` may be a range, read whole as upstream does. Divergences recorded: a sub-register element takes one register, `[4..4]` reads a list of one, an over-wide write is refused. The odd-byte-offset rule waits for CHAR and STRING (S49); whether a profile's `count` maps to a selection is open | check, coverage (core and modbus 100%), lint-attrs, parity, examples, semver; package steps with --allow-dirty (the task itself refuses an uncommitted tree) |
| S56 | 2026-10-29 | Staleness and the acquisition loop (finding G09 resolved), ADR-0023: `StaleTracker` in `stonecrab::sensors`, pure logic per profile with no clock of its own (`record`, `expire(now)`, `next_expiry()`); a retained measurement is `Stale` once its age (now minus `received_mono`) exceeds `stale_after`, exclusive at the threshold, reported once, value and timestamps kept, the earlier quality in the new field `Measurement::quality_before_stale`; a measurement without a value erases nothing. `Acquisition::new(&profile, &dyn Driver, &dyn Connection, &dyn Clock, interval)` reads through `Connection::read` (not S55 subscriptions: one interval per subscription, item-by-item delivery and grid catch-up would break `min_interval`), raises a faster interval to `min_interval`, reads everything due together in one request, hands fresh measurements and stale transitions out of `recv()` with no spawned task, ends when the connection closes, and refuses a clock that cannot pace it. 17 tests on FakeClock against the simulator and Modbus over the fake transport, including wall-clock jumps and dropped `recv` calls. Also reconciles the deferred list of this file and ARCHITECTURE notes left stale by S45 to S58 | check, coverage (sensors 100%), lint-attrs, parity, examples, semver; package steps with --allow-dirty |
| span tests (fix) | 2026-10-29 | Under load, `cargo test` failed two span tests of `modbus_connection` ("a read span", "the tick read has its own read span") while nextest passed them: in tracing-core 0.1.36 a callsite another test first hits while the binary installs its global tap is filed before it asks the default dispatcher for its interest, so it can keep `never` after the install-time rebuild, and its spans are lost. The `traced` helpers of `modbus_connection` and `modbus_session` now call `tracing::callsite::rebuild_interest_cache()` once the tap is installed | check; both binaries under 12 busy loops: unfixed 1 failure in 300 runs, fixed 0 in 300 |
| S50 | 2026-10-29 | Extended registers through FC20 and FC21 (`MB-ADDR-EXTENDED` implemented, ADR-0015 addendum): `extended-register:N`, `6x` and the bare `6` form under an explicit convention, the address being the register number (0 refused), file = address / 10000 + 1 and record = address % 10000 as upstream sends them (its parser-suite frames decode byte for byte), the last address 655349999 (upstream's 655359999 would need file 65536; divergence `MB-LIMIT-EXT`); reads in chunks of at most 120 registers (125 would answer over the 245 data bytes V1.1b allows), one FC20 request per chunk cut at file ends and every sub-response checked and joined in order (upstream used the first only, `MB-QUIRK-B` fixed); writes as one FC21 request per item cut the same way and confirmed only by the whole echo. Merged with S48 by a dedicated merge (ADR-0022 addendum of 2026-10-29): selections work on the extended registers, offsets count through the files, a selection is at most 120 registers, and a list written across a file end at most 118. No interop peer serves FC20 or FC21, so the evidence is fake-transport and responder tests | check (1542 tests), coverage (modbus 100%), lint-attrs, parity, examples, semver, fuzz smoke (10 s per target), package list |
| S59 | 2026-10-29 | Modbus RTU, ADR-0024: `ModbusRtuDriver` (`modbus-rtu` on a serial port by default, or a serial-to-IP gateway over `tcp` with port 502 as Go has it), `RtuFramer` and `RtuAdu` with CRC-16/MODBUS sized by upstream's function-code table, the line silence before every request (t3.5 or `serial.interframe-delay`) and a drain after an unanswered one, correlation by unit and function code, one request per port at a time, and the TCP driver's read, write, ping and polling paths shared through a crate-private wire choice (so extended registers and selections work over RTU, tested at the merge). Divergences: a CRC error fails at once instead of waiting for the deadline, multi-item writes go one request per item, only 8 data bits, broadcast and shared ports refused. `modbus` now enables `serial`, so `serial2` is in the default build (ADR-0018 addendum, DEPENDENCY_AUDIT updated). New fuzz target `fuzz_modbus_rtu`. Simulator-tested only: no independent RTU server or device yet | check, coverage (modbus 100%), lint-attrs, parity, examples, semver, fuzz smoke (eight targets), package list; at the merge with S48 and S50 the same gates again |
| S62 | 2026-10-29 | Release-evidence tooling, the long runs left for the maintainer's approval (ADR-0005 and ADR-0017 addenda): a `soak` example (four tags read in a loop through `DriverRegistry`, counters logged each second) driven by `scripts/soak.sh`, which samples its RSS with `ps`, restarts the pymodbus container on a schedule (its own container on 127.0.0.1:5021), writes a CSV and a report, and judges growth as the median of the last quarter minus the median of the first after a 20% warm-up, bound 4096 KiB (a 24 h pass rules out a steady leak above about 0.28 MiB an hour; a 10-minute pass only shows the command works); `fuzz-evidence.sh` writes a report with pass criteria; `scripts/branch-coverage.sh` reports branch coverage on the pinned nightly (92.9% overall, varies by about one branch between runs; report only, weekly job). Recorded: 10-minute soaks against the simulator (faults every 5 s) and pymodbus (4 restarts), a 4-minute run with a fault every second, all PASS, and a 5-minute fuzz run of all targets, PASS. Observation: the simulator's RSS rises about 300 KiB in the first minutes, then flattens; the 24 h run settles allocator high-water against a small leak | check, lint-attrs, parity, coverage (unchanged), soak.sh --self-test, the short runs below |
| S63 | 2026-10-29 | The MAX31855 board adapter (M2S, SPEC direct sensors), ADR-0026: module `stonecrab::boards` behind the new features `boards` (needs `sensors`, no Tokio, no dependency) and `adapters-linux` (adds `transport`); `SpiBus`, a deadline-bounded read of N bytes (not the drawn transfer: the MAX31855 sends nothing); `Max31855` decoding the 32-bit frame with exact integers (14-bit thermocouple at 0.25 C, 12-bit cold junction at 0.0625 C, the open, short-to-ground and short-to-VCC flags as a struct of three bools, reserved bits and a fault bit that disagrees with the flags refused as `MalformedData`, all-ones and all-zeros frames as an unavailable device, `Disconnected`); three measurements per reading (thermocouple `Bad` with no value under a fault, cold junction still `Good`, the flags); `LinuxSpiDev` reading `/dev/spidev` with read(2) on a worker thread, no dependency and no unsafe; the scripted `testkit::FakeSpiBus`. Test vectors from the datasheet's Tables 4 and 5 (Rev 2, pinned by sha256 in an ADR addendum, not committed). Bus-mock tested, no hardware: the bench runbook `docs/hardware/max31855-spi.md` waits for the maintainer's approval; COMPATIBILITY gains F8 (RTU, tagged since S59) and F10 rows | check, coverage (boards measured with core at 100%), lint-attrs, parity, examples, semver, package list; clippy on the boards, adapters-linux and boards+testkit feature sets; the spidev tests 25 times under load |
| S49 | 2026-10-29 | Text types for Modbus (finding G08, the text part), ADR-0025: `CHAR` and `WCHAR` to `PlcValue::Char`, `STRING(n)` and `WSTRING(n)` to `PlcValue::String`, the length on the address (`string_length`, 1 to 65535, bounded by one read: STRING(250), WSTRING(125), 240 and 120 on extended registers) rather than in a tuple variant, which cargo-semver-checks refuses on the published unit enum; UTF-8 and UTF-16BE, a read cut at the first NUL, an invalid encoding or unpaired surrogate an item's `MalformedData` with the registers kept (no lossy path), writes of the exact variant NUL-padded to whole registers (`InvalidConfig` when the value does not fit, `ResourceLimit` above 123 registers), CHARs packed two to a register, the per-register byte swap of the `*_BYTE_SWAP` orders (a recorded divergence for odd lengths) and the odd-byte-offset rule for selections. `PlcValue::Bytes` is not added: nothing produces it yet (`CORE-VALUE-BYTES` deferred). Text registers join the pymodbus interop map (`fixtures/modbus/interop/text-registers.toml`); the four type rows are `implemented`, as pymodbus stores words without reading text. Follow-up: a `[[file]]` provenance entry for that fixture needs a testkit test change | check, coverage (modbus 100%), lint-attrs, parity, examples, semver, package list, interop for every peer (pymodbus 16 of 16 with the text test) |
| S61 | 2026-10-29 | UDP transport and Modbus TCP over UDP, ADR-0027: `UdpFactory` and `UdpTransport` (connected socket, datagrams from other addresses dropped by the OS as upstream filters them, one whole datagram per receive, an oversize datagram `ResourceLimit` where upstream drops it with a warning, empty datagrams skipped, ICMP port-unreachable as `Disconnected`, nothing retransmitted); `modbus-tcp:udp://host[:port]` with the MBAP framing and transaction-id correlation of TCP and the declared default port 502 (upstream fails with "No port defined": `MB-QUIRK-A` stays a divergence); no `udp.*` option ships. Tests on loopback sockets with the paused Tokio clock (boundaries, filter, oversize, closed port, lost, duplicated and late replies, the contract suite with 13 named skips). Interop: diagslave 3.6 (proconX free binary license; downloaded and checked by sha256 at image build, never committed) in Docker with `tc netem` loss, delay, duplication and reordering, six tests, a CI job `interop-udp` and `mise run interop-udp`, kept out of `mise run interop` because colima's default port forwarder does not carry UDP | check, coverage (transport 98.79%), lint-attrs, parity, examples, semver, package list, the UDP tests 50 times; the diagslave interop only through a local relay (colima does not forward UDP), the hosted interop-udp job runs first with this push |
| S61 (evidence) | 2026-10-29 | The first hosted `interop-udp` run (CI run 40830080420, commit d208261) passed all six diagslave tests on `ubuntu-latest` with `sch_netem` loaded: write and read back, loss, delay below and past the deadline, duplication and reordering. The UDP rows of README, COMPATIBILITY and the component docs now cite it instead of "pending", and an ADR-0027 addendum records it; the pymodbus text test of S49 passed in the same run | parity |
| S49 (follow-up) | 2026-10-29 | `fixtures/modbus/interop/text-registers.toml` gets its own `[[file]]` entry in `fixtures/modbus/PROVENANCE.toml`, and the testkit test that pins the entries (`golden_dir_lists_every_vector_in_provenance`) names it; an ADR-0025 addendum closes the exception S49 recorded | the golden-file test |
| S60 | 2026-10-29 | Modbus ASCII, an ADR-0024 addendum: `ModbusAsciiDriver` (`modbus-ascii`; serial by default, the tcp gateway with port 502), `AsciiFramer` and `AsciiAdu` (':' and upper-case hex of address, PDU and LRC, then CR LF; lower case accepted on receive as Java and Go do; at most 513 bytes; the LRC the two's complement of the byte sum), a second serial wire that shares every S59 path (session, connection, codec, batcher, guard and pacer), Go's resynchronization inside a buffer and a fail-fast on a bad frame (reading the pinned source showed Java loses a good frame behind noise; recorded), no character timeout and no t3.5, the RTU drain sized at 513 characters after an unanswered request, and `serial.data-bits` 7 or 8 for ASCII. Upstream ASCII vectors from the parser suite, the Java codec test (whose frame has a wrong LRC, refused here) and the Go codec test; a fuzz target `fuzz_modbus_ascii`. Simulator-tested only: no independent ASCII server or device yet. At landing, `.gitattributes` marks `fuzz/corpus/**` binary, because `text=auto` had turned the seeds' CR LF into LF | check, coverage (modbus 100%), lint-attrs, parity, examples, semver, package list, fuzz smoke (nine targets); at landing the CR LF of the ASCII fuzz seeds kept (a .gitattributes rule) |
| S64 | 2026-10-29 | The ADXL345 board adapter (M2S), ADR-0029: `I2cBus` (write and write_read, deadline-bounded; a NACK is `Disconnected`); `Adxl345` with `init` (DEVID 0xE5 checked first, a wrong one `Trust`; a discarded data read clears stale flags; seven configuration writes in a fixed order, kept apart from any public write API), `verify_identity`, `read_sample` (`None` until DATA_READY, else three [g] measurements) and `read_fifo` (a `SampleBlock`: exact interval, x/y/z counts, a gapless sequence from `init`, the overrun flag or a full FIFO, and the host's estimate of samples lost, including a FIFO that fills during the drain); exact decode with out-of-resolution counts refused; a failed or dropped read ends the stream until `init`. Testkit `FakeI2cBus`, `I2cStep` (with `Delay`) and `I2cTransfer`. `boards` is now a directory with its own coverage row (100%) and lint promotions. No Linux I2C backend: the i2c-dev ioctl needs unsafe code or a new dependency. Datasheet Rev. 0 pinned by sha256 in ADR-0029 (analog.com did not answer; a mirror served it). Bus-mock tested, no hardware: runbook `docs/hardware/adxl345-i2c.md` waits for approval | check, coverage (boards 100%, now its own row), lint-attrs, parity, examples, semver, package list, clippy on the boards feature sets |
| S59b | 2026-10-29 | RTU against an independent implementation: the S61 diagslave image in its second mode, `diagslave -m enc -p 1503 -a 1` (Modbus RTU encapsulated over TCP, the RTU driver's gateway carrier), in its own container and port, chosen by `INTEROP_DIAGSLAVE_MODE` in the same `interop.sh`; nine ignored tests (`modbus_interop_diagslave_rtu.rs`, a CI job `interop-rtu`, `mise run interop-rtu`, which `mise run interop` now includes because colima forwards TCP): every area written and read back (input registers and discrete inputs alias the holding registers and coils in diagslave), a range selection as one request and one list, the longest frames each way (255 bytes), exception 02 for FC20 and FC21 (diagslave serves no file records) with the session going on, ping, and a unit diagslave does not serve, which it answers by closing the connection (`Disconnected`), as it does a bad CRC. Six MBS rows move to automated-verified (an ADR-0024 addendum amends the evidence level) | check, lint-attrs, parity, semver, interop-rtu (9 of 9, twice on one container), interop for every peer; the hosted interop-rtu job runs first with this push |
| S52e | 2026-10-29 | The SOLO SLM1-C replay over Modbus RTU: the S52d suite now runs through `ModbusRtuDriver` over the fake transport with RTU framing and a CRC-16 written independently of the crate's, on the line the fixture states in a new `[device.serial]` table (9600 baud, even parity and 1 stop bit, the manual's factory defaults of 1073H, 1075H and 1076H; 8 data bits, the RTU write to 1074H that runbook step B2 makes; unit 1); the replay stays silent for another unit and for a bad CRC, as a serial slave does, and a test pins the frame sent and the t3.5 wait of an 11-bit character at 9600 baud. The documented profile names `modbus-rtu` with the line in its options, and `SensorProfile::tags` and `validate_for` accept it with the RTU driver; no source or schema change. New row `MBS-DEVICE-SLM1C-RTU`. Still a replay: HW2 needs the maintainer's approval | check, coverage, lint-attrs, parity, examples, semver, package list |
| S68 | 2026-10-29 | The connection cache, ADR-0030 (decision E6, the first PLC4X tool; not a numbered slice of the plan, which leaves tools to M5): `ConnectionCache`, `ConnectionCacheBuilder` and `LeasedConnection` at the crate root, no feature and no dependency; one holder per connection string, a FIFO queue bounded by `max_pending_requests` (one more caller gets `Backpressure`), a dropped waiter leaving the queue at once; lease, wait, idle, ping and close bounds on the injected clock, evaluated when called, with upstream's defaults (read now in PlcConnectionCache.java:82-96 and recorded in the manifest); the lease is a `Connection` that checks it is still valid (`Timeout` past its lease time, `Disconnected` once given back or removed); the idle ping before reuse and replacement after a failed ping or a broken link; `remove` force-closes, `close` refuses later gets. 24 tests on FakeClock against the simulator and Modbus TCP over the fake transport, including a remove during a replacement. `TOOL-CC-RESTORE` deferred: restoring a subscription needs a forwarding subscription design | check, coverage (core 100%), lint-attrs, parity, examples, semver, package list |
| S59b (evidence) | 2026-10-29 | The first hosted `interop-rtu` run (CI run 40838776577, commit 5858730) passed all nine diagslave RTU tests on `ubuntu-latest`; the RTU rows of README, COMPATIBILITY, the component doc and the serial parity record cite it instead of "next evidence", and an ADR-0024 addendum records it | parity |
| S60b | 2026-10-29 | ASCII, and RTU on an emulated serial line, against an independent peer: the diagslave image gains two modes in which socat 1.8.0.3 (pinned Debian package) makes a pseudo-terminal pair, diagslave runs on one end (`-m ascii -a 1 -b 19200 -d 7 -s 1 -p even`, and `-m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20`) and the other end is bridged to TCP (one connection at a time, a serial line has one master), so the drivers' gateway carrier meets diagslave's ASCII framing and LRC and its gap-cut RTU framing. Nine ignored ASCII tests (`modbus_interop_diagslave_ascii.rs`) and the RTU file run again in the new mode (a filter per mode); a CI matrix job `interop-serial` (`interop-ascii`, `interop-rtu-pty`) and two mise tasks, both in `mise run interop`. Observed: on a line diagslave answers an unserved unit and a bad CRC with silence (the driver reports `Timeout`), where `-m enc` closes the connection. MBS-ASCII-SEND, -LRC, -WIRE-MODEL and the shared operation rows move to automated-verified | check, lint-attrs, parity, semver, interop-ascii and interop-rtu-pty (9 of 9 each), interop for every peer; the hosted runs of the two new jobs come with this push |
| S69 | 2026-10-29 | The event pump, ADR-0031 (decision E6, the second PLC4X tool): `EventPump`, `TagBatch` and `BatchEvent` at the crate root, no feature or dependency; named batches on a borrowed connection or a per-fetch lease from the connection cache, one read per fetch, start and stop per batch, start-all, stop-all and close; a fixed tick grid after an initial delay (upstream's timer is fixed delay with a one-slot dispatcher, which keeps the interval in practice), a tick during a fetch skipped and counted, upstream's backoff after a failure, events from `recv()` with at most one queued per batch, and a dropped `recv` keeping reads in flight (a deliberate difference from S55's subscriptions, for the maintainer to confirm). The S55 tick helper moved to `clock.rs` for both callers. Upstream's timer, dispatcher, backoff and factory behavior read in the pinned source and recorded in the manifest. `TOOL-EP-CONFIG` and `-TRANSFORM` deferred, `-SUBSCRIPTION` not ported | check, coverage (core 100%), lint-attrs, parity, examples, semver, package list |
| S70 | 2026-10-29 | Two Modbus follow-ups, an ADR-0015 addendum: (a) a reply that decodes but does not fit its request (libmodbus's short reply, a coil reply with the wrong byte count, an FC20 reply that leaves a sub-request out) fails every member of its chunk with `MalformedData` and keeps the reply PDU in the item's `ProtocolStatus` raw bytes (as ADR-0025 does for text), the other chunks going on; it is no longer retried even if a `RetryPolicy` lists `MalformedData` (no default does; for the maintainer to confirm); a PDU that does not decode at all still fails the whole read without bytes. (b) `ModbusConnection::stats()` returns `SessionStats` (late and mismatched replies, the guard's counters with `discarded_frames`), and the new public `ModbusRtuDriver::open` and `ModbusAsciiDriver::open` give the concrete connection for RTU and ASCII; a `Box<dyn Connection>` from the registry still cannot reach it. A codec round-trip test pins that the kept bytes equal the received ones | check, coverage (modbus 100%), lint-attrs, parity, examples, semver, package list, interop for every peer (the libmodbus short reply asserts its bytes) |
| S63 (fix) | 2026-10-29 | The hosted `quality` job of run 40843548924 failed its coverage gate at `boards` 513 of 514 lines: `LinuxSpiDev::read` had two ways to report a stopped worker (the send to its queue, and the reply), and which one ran in `a_worker_that_has_died_is_a_disconnect_not_a_hang` depended on whether the panicked worker had dropped its queue yet. A failed send drops the command and its reply sender, so the read now ignores the send result and reports both cases through the reply; the behavior is unchanged and the coverage no longer depends on timing | check (clippy and the SPI tests), coverage (boards 510 of 510 lines) |
| S60b (evidence) | 2026-10-29 | The first hosted runs of `interop-ascii` and `interop-rtu-pty` (CI run 40843548924, commit fc30f8c) passed nine of nine each; README, COMPATIBILITY, the component doc, the serial parity record and CHANGELOG cite them instead of "next evidence", and an ADR-0024 addendum records them. The `quality` job of that run failed on a timing-dependent coverage line in `boards`, fixed in 8400e45 | parity |
| S59c | 2026-10-29 | The serial-port half of the RTU and ASCII interop (the plan's socat pair for S58 to S60): `interop/diagslave/serial.sh` makes a socat pseudo-terminal pair on a Linux host, runs the pinned diagslave archive's Linux binary (sha256 checked before it runs) on one end and gives the other end to the RTU and ASCII interop tests through the serial carrier (`modbus-rtu:///…`, `modbus-ascii:///…` with `serial.*` settings; the script refuses any other URL), plus two new tests of what the serial carrier adds: the line silence at 300 baud and with `serial.interframe-delay`, and a lost port reopened. A Linux-only CI job `interop-serial-port` and `mise run interop-serial-port` (not in `mise run interop`: diagslave has no macOS binary and macOS refuses the baud-rate ioctl on a pty). No parity row or evidence level moves before the first hosted run. A second RTU peer (libmodbus, modbus-tk) was not added: libmodbus's RTU server opens a hard-coded `/dev/ttyUSB0`, and modbus-tk needs pip installs. Side effect on the maintainer's machine: installing socat with Homebrew also installed openssl@4 and repointed the unversioned `openssl` links to it (openssl@3 untouched) | check, lint-attrs, parity, interop for every gateway peer; the serial run only as a modified copy in a Linux container (11 of 11 per framing); the hosted interop-serial-port job runs first with this push |
| S59c (evidence) | 2026-10-29 | The first hosted run of `interop-serial-port` (CI run 40849466167, commit f656c56) ran `serial.sh` as written on `ubuntu-latest` (the archive downloaded and checked against its sha256, the x86_64 binary, Ubuntu's socat) and passed 11 of 11 per framing. `MBS-TRANS-SERIAL`, `-TRANS-PORT-NAME`, `-CFG-SERIAL-INTERFRAME`, `-RTU-SILENCE`, `-CFG-SERIAL-BAUD` (RTU only) and `-ASCII-TIMING` (the inter-frame delay only) move to automated-verified, and the shared rows gain the serial half; data bits, stop bits, parity and flow control stay implemented (a pty cannot show them). README, COMPATIBILITY, the component docs, CHANGELOG, the diagslave README and an ADR-0024 addendum record it; two ASCII and RTU-pty cells that cited the interop-rtu run id are corrected | parity |
| S66/S67 | 2026-10-29 | The MSpec generator for Modbus (ADR-0019 and its addendum; decisions E3 and E8). `modbus.mspec` of PLC4X v1.0.0 is vendored verbatim with its ASF header at `upstream/mspec/modbus/modbus.mspec` (its `[[mspec]]` entry in `upstream/manifest.toml` holds the git blob and sha256; the root and crate NOTICE name it). `tools/stonecrab-codegen`, a native Rust tool in its own workspace (dependencies `toml` and `sha2`, its own lock and deny policy), parses the whole of MSpec.g4 and Expression.g4 by recursive descent: its `check` parses 57 of the 59 upstream mspec files, and `ads.mspec` line 1088 and `eip.mspec` line 310 break the grammar (upstream accepts them only through ANTLR error recovery). It emits the feature-matrix subset into `crates/stonecrab/src/modbus/generated/`, compiled only into the tests (`cfg(all(test, feature = "testkit"))`): the TCP ADU, the exception PDU, the FC01 to FC04 requests and responses, the error codes, the data types and 14 numeric DataItem cases; any other construct is a named generation error. An in-crate equivalence suite compares the generated and hand codecs in both directions over the 296 Modbus vectors, 35 fuzz seeds and property tests, with a closed list of hand-only refusals (parity rows `MB-GEN-*`); the hand codec stays the production path. S67: `mise run regen` (`scripts/regen.sh` and its self-test) and a blocking `regen` CI job run the tool's fmt, clippy, tests and cargo deny offline, check the mspec sha256 and require two byte-identical runs and an unchanged tree. Landed with the CODING_STANDARDS margin notes (1.2, 17, 18.2, 19), the CONTRIBUTING baseline step, `.gitattributes` marking generated files and a CI `unsafe` grep that also scans `tools/` and `fuzz/`. The hosted `regen` job runs first with this push | check, coverage (modbus 100%), lint-attrs, parity, examples, semver, dependencies, machete, package list, regen twice (the tool's fmt, clippy, tests and cargo deny), interop for every gateway peer |
| S74 | 2026-10-29 | G06 (decisions E7 and E12): the sensor `unit` is `Option<String>` in `MeasurementDef` and `Measurement`, so a profile may leave it out to mean "unknown" (an ADR-0016 addendum). An absent unit loads as `None` and validates; a stated unit is checked as before, and a blank one (only whitespace) is now `EmptyUnit` like the empty string. The boards set `Some`. The profile schema stays version 1, since every document 0.2.0 loads still loads the same way; a 0.2.0 reader refuses a unit-less document with `Syntax`. The crate version moves to 0.3.0 (unpublished) with a "Migration from 0.2" note in CHANGELOG.md: the one source break (`unit.as_deref() == Some("Cel")`) and the behavior changes since 0.2.0 that a caller can notice (S36, S39, S55, S50, S70, S59). cargo-semver-checks 0.51.0 has no lint for a public field's type, so it reported nothing; the bump is what announces the break. New tests: an absent unit loads, validates and gives a measurement without one; a blank unit is refused | check, coverage, lint-attrs, parity, examples, semver (against 0.2.0 as a major release, so no checks run; before the bump the unit change passed all 202 as a minor), dependencies, machete, package list |
| S73 | 2026-10-29 | The two SOLO SLM1-C findings of S52d (decision E11). (a) The opt-in `single-coil-write=fc05` of `modbus-tcp`, `modbus-rtu` and `modbus-ascii` (an ADR-0015 addendum, `MB-CFG-SINGLE-COIL`) sends a write of exactly one coil as FC05 (`FF00` or `0000`); two or more coils and register writes keep their function, FC15 stays the default as upstream, and the echo check and the S46 outcome rules are unchanged; `FunctionCode::WriteSingleCoil`, `ModbusPdu::WriteSingleCoil` and `WriteSingleCoilResponse` are appended; `decode_request` still refuses FC05, and the generated codec still disables it. (b) Profile schema version 2 (an ADR-0016 addendum; `SchemaVersion::CURRENT` is now 2, version 1 loads unchanged): `value_rules` match the unscaled integer value and set `Bad` or `Uncertain` with an optional reason, ranked with range and status quality by `Quality::worse`; the winning reason is the new `Measurement::quality_reason`; the key in a version 1 profile is `ValueRuleNeedsSchema2`, an empty rule `EmptyValueRule` (both appended). The SOLO profile moves to version 2 with its five named sentinels `Bad` and their reasons, and its replay writes the five bit registers with FC05 over RTU. The fuzz targets for the PDU and the profile loader follow (FC05 echoes decode; version 2 loads) | check, coverage (modbus, sensors and boards 100%), lint-attrs, parity, examples, semver (against master as a minor), dependencies, machete, regen, package list, interop for every gateway peer, fuzz-smoke |
| S71 | 2026-10-29 | TLS (decision E11, ADR-0032; an ADR-0018 addendum for the feature): an opt-in `tls` feature (in neither the default set nor `full`) adds `rustls` 0.23 with the `ring` provider and `tokio-rustls` 0.26 (licenses within deny.toml; `rustls` joins the `once_cell` wrappers, and the copies in `fuzz/` and `tools/stonecrab-codegen/` follow), a `TlsFactory` over the TCP connect path with the handshake inside the same deadline, and `modbus-tcp:tls://host[:802]` with upstream's keys `tls.trust-store` (required) and `tls.keystore` (mutual TLS; PEM, at most 1 MiB, read once). Trust is only the given anchors (no web PKI, platform store or verify-off); the server name is always verified; TLS 1.2 and 1.3. Certificate failures are `Trust`, a refused client certificate `Authentication`; the keystore path never reaches `Display`, `Debug` or `redacted_display`, and errors name the option, not the path. `tls-psk` is `InvalidConfig` with the reason; `modbus-rtu` and `modbus-ascii` refuse every `tls.` key on every carrier (a review caught a silent plaintext fallback there). Tests run on a PKI generated in code (Ed25519 through ring; `rcgen` needs the banned `time`); the new `interop-tls` job and `mise run interop-tls` (also in `mise run interop`) run pymodbus 3.8.6 over mutual TLS with a PKI made at run time. The first hosted run of `interop-tls` goes with this push | check, coverage (modbus 100%, transport 98.7%), lint-attrs, parity, examples, semver (against master as a minor), dependencies, machete, features, regen, package list, fuzz-smoke, interop for every gateway peer and interop-tls (3 of 3 against pymodbus 3.8.6 over mutual TLS) |
| S72 | 2026-10-29 | Object PLC Mapping, the third PLC4X tool (decisions E6 and E11, ADR-0033): no proc-macro crate and no feature or dependency. A type implements `PlcEntity` (its `EntityField` list of name, address or `${alias}` and optional cache duration, and a by-name conversion through `FieldValue` for `bool`, the integers, `f32`, `f64`, `char`, `String`, `Duration`, the calendar types, `Vec<T>` and `PlcValue`), or `stonecrab::plc_entity!`, a `macro_rules!` macro, writes the struct and the implementation (`field: Type => "address" (cache = <const Duration>)`; a field type without `FieldValue`, a non-literal address, a non-constant duration or a generic struct do not compile, shown by `compile_fail` doctests). `EntityManager` over the S68 `ConnectionCache`: `read` (one request), `write` (one request, the `WriteResponse` returned), `connect` and `merge` give a `ConnectedEntity` whose `refresh` re-reads in one request only the fields whose duration passed on the injected clock (no duration means every refresh, upstream's -1), and `disconnect` returns the value; aliases (`with_alias`) and a request timeout (default 1 s) as upstream. Each request takes a lease and gives it back; nothing is spawned. Writes send the field type's own value, so a written field must have its tag's type (ADR-0033). The runtime proxy stays deferred (`TOOL-OPM-PROXY`). 24 cases on the fake clock against the simulator and Modbus TCP over the fake transport | check, coverage (core 100%), lint-attrs, parity, examples, semver (against master as a minor), dependencies, machete, features, regen, package list, fuzz-smoke, interop for every peer |
| S66/S67, S71 (evidence) | 2026-10-29 | The first hosted runs of the new jobs: `regen` passed in run 40869309931 (commit 30b0b83) and in every completed run since; `interop-tls` passed 3 of 3 in run 40880582023 (commit bef0d25), so the TLS cells name that run. That push failed two jobs: `cross`, because `ring` (feature `tls`) compiles C in its build script and the runner had no aarch64 or armv7 C compiler, and the Windows legs of `test`, because the redaction test looked for a raw path in a display that shows paths in their Debug form; 764294b installs the cross compilers in `cross` and makes the test compare the Debug form and the keystore file name. Before that, run 40869309931 failed `fuzz-smoke` because `fuzz/Cargo.lock` still named 0.2.0 (fixed in c1238e9) | parity |
| Release 0.3.0 | 2026-10-29 | `stonecrab` 0.3.0 published to crates.io at 13:51 UTC under decision E12, from the release commit `66ae100` (the CHANGELOG section with the four release-note items of SPEC and the gates not run, the "Migration from 0.2" entry, and every document that names the current version), and tagged `v0.3.0` (annotated, tag object `7a2439f`, not signed). Before the release commit, an independent three-lens review (release claims, changelog completeness, package and metadata) corrected stale claims in the shipped crate README and Modbus page and completed the migration entry; three CI fixes landed first (`fuzz/Cargo.lock` at 0.3.0, the C cross compilers and target headers that `ring` needs in `cross`, and a portable TLS redaction test). The 24-hour soak and the 8-hour fuzz run were waived (E9) and the release notes say so; an ADR-0017 addendum records the steps. SECURITY.md now supports the 0.3 line only | check, coverage (every module at its threshold), package (fresh CARGO_HOME), interop (every peer), sbom and regen on the clean release commit; cargo package and cargo publish --dry-run; hosted run 40884031994 green |
| Dependencies | 2026-10-29 | Dependabot updates: `actions/checkout` 5 to 7 (PR #1, `dc2ae65`) and `actions/upload-artifact` 6 to 7 (PR #2, `6dff838`) merged as Dependabot's own signed-off commits after green hosted runs (v7 adds an opt-in unzipped upload; nothing here uses it). Then this commit lands PR #3, `toml` 0.9 to 1.1 (1.1.8; `toml_parser` 1.1.5, `toml_datetime` 1.1.2, `winnow` 1.0.4), plus what the PR lacked: `fuzz/Cargo.lock` (its stale lock failed `fuzz-smoke` under `--locked`) and the code generator's manifest and lock, which follow the root's version. One `winnow` in all three locks, so the `toml_parser` pin is gone (an ADR-0016 addendum; DEPENDENCY_AUDIT, the ADR-0019 dependency line and CHANGELOG Unreleased follow). No source change, no public `toml` type, semver reports no change against 0.3.0, coverage unchanged; PR #3's other red jobs (cross, Windows TLS test) were the failures already fixed by `764294b`/`93055f4` | check, coverage, semver (against the published 0.3.0), dependencies, features, fuzz-smoke, regen, MSRV 1.97.1 tests |
| S75 | 2026-10-29 | Scoping of M3 (OPC UA), docs only; nothing implemented and no row changed status. The pinned PLC4X v1.0.0 OPC UA sources were re-read against `docs/parity/opcua.md`, and M3 is sliced in `docs/POST_ALPHA_PLAN.md` (Phase H) into 48 slices, S76 to S123: a generator track (S77 to S80), a crate track (S83 to S87) that join at S88, a first end-to-end Read at S92, security from S101 with the first secure end-to-end slice at S106, then lifecycle, browse, native subscriptions, custom structures, a Toxiproxy fault run, a second peer (asyncua) and the M3 exit at S123. `docs/parity/opcua.md` gains a "Slice assignment (S75)" section placing each of the 195 planned rows in one slice and listing the source-check corrections that S76 applies (discovery runs on the secure channel and needs `server-certificate-file`, policy NONE has nonce length 0, an ABORT chunk, a failed renewal and several browse, publish and event losses upstream). Decisions E13 to E22 are proposed and none is accepted. `upstream/manifest.toml` is unchanged | parity (the assignment section was checked by script: 195 of 195 planned ids placed once, no deferred or unknown id; `scripts/parity-check.sh` and its self-test pass); the plan's slice table was checked by script for five columns, no forward dependency and agreement between each decision's Blocks and the slices that list it |
| S76 | 2026-10-30 | OPC UA record corrections and ADR-0034 (Proposed), docs only; nothing implemented. `docs/parity/opcua.md` takes S75's source corrections, rechecked against the pinned sources: discovery runs on the secure channel and needs `server-certificate-file`; policy NONE has block size 1 and nonce length 0; an ABORT chunk times out upstream; a failed renewal is only logged; the read metadata keys are never set; `b=` is a base64 ByteString. It gains 13 divergence rows, each naming its owning slice (ABORT S94, renewal S109, read metadata S98, `b=` S83, the CreateSession certificate verified before a token is encrypted to it S108, parsed endpoint URLs S96, three browse losses S112, publish routing and acknowledgement S116, event overwrite S118, password without a user name S99). Decision E16 moves Basic128Rsa15, Basic256 and the three generated-certificate rows to `deferred` and renames the PKCS #12 and JKS tests to PEM and DER; E21 and E22 are applied to the port, credential, trust and verify rows. The record now has 241 rows (203 planned, 38 deferred), and the slice assignment places every planned id once. ADR-0034 "OPC UA client design" covers: the hybrid codec (E14), the external-type contract with a depth budget, `Option<Vec<T>>` for null arrays, codec bounds with `ParseError::RecursionLimit` and `CountLimit`, a pipelined request lane above `FramedTransport` (a send cut short fails the whole channel, because send is not cancel-safe), the caller-driven session pump and subscription end (E19), the credential channel and value set (E18), the feature and version rule (E17), and the module layout. It is linked from README and this file; S97 accepts it | parity and its self-test; a scripted id check (241/203/38 matching the count line, 203 planned ids placed once, no deferred id, all 195 S43 planned ids still rows, 13 new rows each with an owner matching its slice); a scripted link and table check and `git diff --check` on the changed documents |
| S77 | 2026-10-30 | Generator gaps G1, G2 and G10 of `tools/stonecrab-codegen` for M3 (an ADR-0019 addendum): (a) a `type` is emitted as a struct with `pub(crate)` stored fields, `parse`, `serialize` and `length_in_bytes` (a `const fn` when fixed), built from the same field planner as a `discriminatedType` case, with nested structs, enum fields, sub-byte runs, `implicit`/`const` checks and parameters; `Eq` is derived unless a field is a float, and the `struct_field_names` and fixed-length lint expectations are emitted by the generator. (b) `byte_order = "BIG_ENDIAN" or "LITTLE_ENDIAN"` in `protocols/<p>.toml` replaces the hard-coded constant (default `BIG_ENDIAN`; recorded in `MANIFEST.toml` only when set); a `byteOrder` attribute must agree with it, and a `batchSet` with attributes is now a generation error instead of dropping them. (c) A type name defined twice is a generation error naming both lines when the output uses it (an unused duplicate, such as the second `NodeIdType` of `opc-manual.mspec`, is not reported, so G10 is enforced for every name the output reaches). The tool is 0.2.0; the Modbus output is unchanged apart from the version in the header of its 8 generated files, committed as a separate `gen` commit. Proven on first-party synthetic mspec text whose generated modules the tests compile with `rustc` against a stand-in codec and run under both byte orders, and in a scratch copy of the crate against the shipped codec under its real clippy lints. README matrix, options and fixtures sections updated | regen (twice, with the gen commit; tools-workspace fmt, clippy -D warnings, 25 tests, cargo deny, hashes, determinism, self-test), `mise run check`, lint-attrs |
| S81 | 2026-10-30 | Landed the independent OPC UA peer `interop/open62541`, following the libmodbus pattern: a Dockerfile that builds open62541 v1.5.9 from the commit of its tag (`05c2b389b65cc35884eb40d800301af1f3d130cf`, checked in the build) as a static library with the OpenSSL backend, on `debian:trixie-slim` pinned by digest, run as uid 10001 (OpenSSL is the archive's current 3.5, recorded in the evidence, not pinned by version); a first-party server `main.c` (policies None, Basic256Sha256, Aes128Sha256RsaOaep, Aes256Sha256RsaPss; anonymous, user name and X.509 tokens; switchable 8192-byte chunk limits; a 5000 ms session timeout ceiling; trust lists from a PKI that `pki.sh` makes at container start and nothing stores; advertised host and port set to the reachable ones; a test namespace with every built-in type but Variant and ExtensionObject, arrays, 2-D and 3-D matrices, 16 KB and 40 KB arrays, a counter, writable nodes, a custom structure and an event source with a fire method); a first-party check client on open62541's own client library; `interop.sh` (build, start, wait with a real OPC UA Hello, check, pki, evidence, stop, run; `stop` removes the copied client-side PKI), `PROVENANCE.toml`, a README, `mise run interop-opcua` (outside the TCP `interop` aggregate) and CI job `interop-opcua`. The in-image client reads the counter with None and with Basic256Sha256 SignAndEncrypt, is refused by a server with an empty trust list (`BadSecurityChecksFailed`), and the other policies, identities, refusals, namespace, events and chunk cases pass too (37 checks). The spike answers the three UNVERIFIED items in the README with source citations: there is no minimum session timeout (a request of 100 ms is honored; only the ceiling applies), TransferSubscriptions and Republish are supported (transfer is Good on a secure channel and BadUserAccessDenied on None for anonymous), and Aes256Sha256RsaPss works under the OpenSSL backend. This closes the E19 question. `main.c` is 481 lines against the plan's about 200 (node table, structure type, env switches). Hosted `interop-opcua` passed on its first run (41115440604, see the evidence row). | `mise run interop-opcua` on colima (37 checks), the same checks on a `linux/amd64` image under emulation, shellcheck, actionlint, a negative test of the commit check; hosted `interop-opcua` green (run 41115440604); crates untouched |
| S82 | 2026-10-30 | Cryptography probe for E15, as evidence. `aws-lc-rs` 1.19.0 (default features off, `aws-lc-sys` 0.47.0 with `disable-prebuilt-nasm`), `rustls-webpki` 0.103.16 with no provider feature and `rustls-pki-types` 1.15.1 were probed by an out-of-tree crate, `spikes/opcua-crypto-probe/`, on the unmerged, pushed spike branch `spike/s82-crypto-probe` (head cf6aea2; 31 tests: RSA-OAEP SHA-1 and SHA-256, PKCS #1-SHA256 and PSS-SHA256, AES-CBC with a caller IV against NIST SP 800-38A, HMAC-SHA1 and SHA256, the SHA-1 thumbprint, a self-signed anchor through webpki with a first-party RSA PKCS #1 SHA-256 `SignatureVerificationAlgorithm`, PKCS #1 against PKCS #8). Green on ubuntu, macOS and Windows (NASM, `AWS_LC_SYS_USE_SYSTEM=0`) and `cargo check` for aarch64 and armv7 (runs 41108284236, 41109122420, 41109616704, 41112736480; the first run's one failure was a workflow fetch error); `cargo deny` with the root `deny.toml` unchanged passes with no ignore entry and the D23 `cargo tree -i` check matches none of `once_cell`, `lazy_static`, `log`, `time`, `chrono`. Findings: a clean release build of the library takes 29 to 30 s and 7.3 MB of static archive against 38 to 50 s for the ring `tls` build; `PrivateDecryptingKey` loads PKCS #8 only (a PKCS #1 key goes through `RsaKeyPair::from_der` and a PKCS #8 re-export, left to the maintainer as an open question since E16 says PKCS #8 only); `aws-lc-rs` cannot sign SHA1withRSA (the encodings are sealed), which supports E16's refusal of Basic256 and Basic128Rsa15; no AWS-LC statement on Marvin-class OAEP timing was found (UNVERIFIED). ADR-0035 "OPC UA cryptography" is Proposed and `DEPENDENCY_AUDIT.md` gains draft entries for S101; `Cargo.toml`, `Cargo.lock`, `deny.toml` and `crates/stonecrab` are unchanged and the spike branch is not merged | probe tests 31 of 31 on macOS (1.99.1 and 1.97.1) and on the CI matrix; `cargo deny`, D23, `cargo machete` and `ci.yml` green on the spike head; `scripts/parity-check.sh` passes |
| S81 (evidence) | 2026-10-30 | The first hosted runs after round M3a (S76, S77, S81, S82; master `1d84acb`, run 41115440604): every job green, including the new `interop-opcua` (open62541 v1.5.9 built on a cold ubuntu-latest runner, None and Basic256Sha256 SignAndEncrypt reads, the other policies and identities, and the trust-list refusals) and `regen` with the generator at 0.2.0. The S82 probe ran green on its spike branch (`spike/s82-crypto-probe` at `cf6aea2`, run 41112736480), which meets the condition of E15; the branch stays unmerged | hosted CI |
| S78 | 2026-10-30 | Landed generator gaps G3 and G4 in `tools/stonecrab-codegen`: `count` arrays of byte-aligned numbers, enums and struct elements checked against the remaining input before any reservation (`ParseError::CountLimit { field, count, max }`), `implicit` `uint 32` and `int 32` counts, the expression subset `==`, `?:`, `null` and prefix `-`, and nullable arrays as `Option<Vec<T>>` (-1 None, 0 Some(empty)) both when an `implicit` writes `x == null ? -1 : COUNT(x)` and when the count is a plain `simple int 32` field (the ByteStringArray shape; serialize refuses a value that disagrees with the stored count); other negative counts are `InvalidDiscriminator`; ADR-0019 S78 addendum, README matrix and `array_fixtures.rs` (rustc-compiled fixtures, the S77 pattern); Modbus output byte-identical, generator stays 0.2.0, no `gen` commit | `mise run regen` green (fmt, clippy -D warnings, 33 tests, cargo deny, two identical runs, self-test); `generator_complex_array_count_is_checked_before_allocation`, `generator_null_array_is_distinct_from_an_empty_array` (fixed-seed property loop, not the proptest crate), `generator_implicit_int32_count_round_trips` |

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
| `fuzz-smoke` (seven targets at the alpha; eight since S59, nine since S60) | green |
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
- Reconnect is request-driven (plan D15) unless the guard is supervised (S57, ADR-0014
  addendum): `ConnectionGuard::supervise` spawns one task, which the guard owns, `close`
  joins and a drop aborts; the Modbus connection starts it inside a Tokio runtime.
  `clippy.toml` disallows `tokio::spawn`; that task and the testkit's `ScriptedPeer` are the
  two spawns, each with the required `#[expect]`.
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
  through `sharded-slab` under a second recorded wrapper). `toml` 1.1 and its parser
  share one `winnow` 1.0, so no lockfile pin is needed (the `toml_parser` pin of the
  `toml` 0.9 era was dropped on 2026-10-29, ADR-0016 addendum).
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

- **Browse and discovery** (M3 or the parity backlog) and native, non-polling
  subscriptions (with OPC UA, M3). Writes (S45, S46), ping (S47), polling subscriptions
  (S55), the stale tracker and acquisition loop (S56), and eager reconnect with
  `Timeouts.idle` (S57) have shipped.
- **Modbus `TIME` and `DATE`** stay rejected because upstream has no Modbus encoding for
  them (S44); the calendar values shipped in S53 (ADR-0021), array selections in S48
  (ADR-0022), extended registers with their selections in S50, the `zero-based` and
  `one-based-40001` conventions in S51, and the text types in S49 (ADR-0025).
  `PlcValue::Bytes` waits for a type that produces octet strings (ADR-0025 decision 9).
- **An electrical serial line and HW2**: since S59c (first hosted run 40849466167) the RTU
  and ASCII drivers open a socat pseudo-terminal through their serial carrier against
  diagslave, so the serial carrier, its baud-dependent t3.5, the inter-frame delay and a
  reopened port are `interoperability-tested`; parity, data bits, stop bits and flow
  control cannot be observed on a pty, and no RS-485 adapter or device has been used; TLS-PSK (refused: rustls has no external
  pre-shared keys) and TLS for the RTU and ASCII gateways (ADR-0032 decision 13); a Linux I2C backend for
  the board adapters (ADR-0029: an ioctl needs unsafe code or a new dependency); OPC UA (M3, after the
  generator); EtherNet/IP and S7 (M4); generated code as a production path (S66 and S67
  prove the generator on Modbus as a test-only codec checked against the hand codec) and the
  remaining baseline protocols (M5).
- **Hardware**: no physical device has been tried and no hardware record exists.
- **Gates not run**: the 8-hour release fuzz run and the 24-hour soak. Since S62 they are
  documented commands with pass criteria (ADR-0017 addendum): `mise run fuzz-evidence --
  28800`, and `mise run soak -- sim 86400` and `mise run soak -- pymodbus 86400`; they
  wait for the maintainer to approve a day of machine time. Recorded so far: 10-minute
  soaks, a 5-minute fuzz run, and the 30-minute runs made before S33. Also not run:
  differential tests against PLC4J and `cargo publish --dry-run` against the registry.
  Branch coverage is reported since S62 (report only, in the weekly `scheduled` job and
  `mise run branch-coverage`); benchmarks exist since S52, their 10% gate in the same job.
- **The `semver` job is blocking** since S35, with cargo-semver-checks pinned to 0.51.0,
  against the published `stonecrab` 0.2.0. An unchanged `version` is checked as a minor
  release (run 40787826005: 202 checks pass, "no semver update required"), so a breaking
  change fails CI until the version bump that announces it, with its migration note. Since
  0.3.0 was published (2026-10-29) the baseline is 0.3.0 and an unchanged version is again
  checked as a minor release; between S74 and the publish the job checked nothing, and the
  slices compared with master (`cargo semver-checks -p stonecrab --baseline-rev <master>
  --release-type minor`). cargo-semver-checks has no lint for the type of a public struct
  field, so it never reported G06; the 0.3.0 version announced it.
- **A plain `cargo test` runs the reduced suite** (no `testkit` feature, so the test files
  that need the kit are skipped without a notice) and the kit's own tests need a checkout;
  see "Important implementation and tooling details".
- **Upstream vectors**: S04a and S04b turn the four retained Apache-2.0 PLC4X test files
  into 70 vectors. Every big-endian and little-endian exchange decodes to the value the
  XML expects, so `MB-BYTEORDER-BE` and `MB-BYTEORDER-LE` are `automated-verified`; no
  retained file configures a swapped mode, so the two swap rows stay `implemented`. The
  write cases are retained but not vectors yet (S45, S46); `DriverTestsuiteOptimized.xml`,
  the file upstream's own driver IT loads, is not retained.
- **A reply that does not decode keeps no bytes** (S70): a reply that decodes but does not
  fit its request fails every member of its chunk with `MalformedData` and keeps the reply
  PDU (re-encoded, equal to the received bytes because the codec is canonical) in the
  item's status; a reply whose PDU does not decode at all still fails the whole read
  without them.
- **The background reader is quiet** (S57): the supervisor emits no tracing events (the
  `transport` feature does not depend on `tracing`); its state changes are visible through
  `watch_state`. Since S70 `ModbusConnection::stats` returns the late and mismatched
  replies and the guard's counters, `discarded_frames` included, for a connection held as
  the concrete type (`ModbusConnection::open`, and `ModbusRtuDriver::open` and
  `ModbusAsciiDriver::open`); a `Box<dyn Connection>` from the registry cannot reach them.
- **ASCII residual risk** (S60): with the default gap of 0 an unsolicited complete frame
  that arrives while the line is idle is dropped before the next request only by the guard's
  supervisor inside a Tokio runtime; `serial.interframe-delay` makes the driver watch the
  line first.
- **RTU residual risks** (S59): a late reply that only begins after the next request is on
  the wire, with the same unit, function and shape, cannot be told apart (the drain after an
  unanswered request bounds it); below about 520 baud the default 5000 ms request timeout
  is shorter than the drain plus a reply; and there is no pseudo-terminal RTU test
  (`serial2` cannot make a pty on macOS).
- **SOLO SLM1-C, replay only**: since S73 the opt-in `single-coil-write=fc05` writes the
  bit registers the manual (S52d) lists only for FC05, and profile schema version 2 makes
  the PV sentinels the manual names (8002H to 8004H, 8006H, 8007H) `Bad` with their
  reasons; 8005H, which it leaves out, stays `Uncertain` through the range. Both are checked
  against the documented map, not a device.
- **TLS 1.3 and a refused client certificate** (S71): the server refuses the certificate
  after the handshake, so `open` succeeds and the first request fails with
  `Authentication`, with no data sent; the guard then treats it as a lost link and reopens
  with backoff until the reconnect budget is spent (ADR-0032 decision 10). Making it
  terminal, as it is at `open`, would change the shared guard. pymodbus drops such a client
  without an alert, so against it the refusal is `Disconnected`.
- **Not verified by the interop runs**: a third-party server that refuses a unit id (the
  modbus-tk server of S52c is our own script), a peer that misbehaves on its own
  (Toxiproxy, S52b, injects faults between healthy peers), and any physical device.
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

The alpha plan is executed (S01 to S33) and published as `stonecrab` 0.2.0. The
hardware-free slices of [`docs/POST_ALPHA_PLAN.md`](docs/POST_ALPHA_PLAN.md) are executed
too (the slice log above) and published as `stonecrab` 0.3.0. What remains of that plan
waits for the maintainer: test hardware (E4; HW2, the SOLO SLM1-C on RS-485, first), the
24-hour soak and the 8-hour fuzz run (E9), and the later phases (OPC UA in M3, then
EtherNet/IP or S7 in M4) in the order the plan gives.

M3 (OPC UA) is sliced (S75): the slices S76 to S123 are in Phase H of the plan. The
maintainer accepted E13 to E22 and the S82 spike-branch push on 2026-10-30 ("Maintainer
decisions (2026-10-30)" below), so the slices follow their dependencies: S76 to S82 first,
then S83 and the generator track. E14 and E15 are confirmed by the S80 and S82 evidence.
The client design is ADR-0034
([`docs/adr/0034-opcua-client-design.md`](docs/adr/0034-opcua-client-design.md), proposed
by S76; S97 accepts it).

Maintainer-only steps, which no automated run does: P1 to P3 above (P1, the private
vulnerability-reporting channel, is a release prerequisite in `SECURITY.md` that neither
0.2.0 nor 0.3.0 met), optionally the annotated tag `v0.2.0` on commit `640665a`,
dispatching the `release-evidence` workflow (it has never been dispatched), deciding
whether to file the six drafted upstream issues under `docs/evidence/upstream-issues/`
(none is filed), and E4 (test hardware, deferred).

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

### Maintainer decisions (2026-10-29)

- **E8** vendoring PLC4X's `modbus.mspec` (Apache-2.0, ASF header kept, NOTICE
  attribution) under `upstream/mspec/` is approved, which unblocks S66 and S67 (ADR-0019).
- **E9** the 24-hour soak and the 8-hour release fuzz run wait; they are not part of 0.3.0.
- **E4** stays deferred. The one device to buy first is now HW2, the SOLO SLM1-C with a
  USB to RS-485 adapter: Modbus TCP has five independent peers, while no serial driver has
  met a real line (parity, seven data bits, real baud timing).
- **E10** the behavior choices of S59, S60, S69 and S70 are accepted as shipped: one RTU
  request per write item, a CRC error failing at once, port 502 for a TCP gateway, the
  `modbus` feature enabling `serial` (`serial2` in the default build), a dropped event-pump
  `recv` keeping its reads in flight, a misfit reply not retried, and the public
  `ModbusRtuDriver::open` and `ModbusAsciiDriver::open`.
- **E11** the design calls follow the recommendations: TLS through rustls with the ring
  provider behind an opt-in `tls` feature, mutual TLS with explicit trust anchors and no
  TLS-PSK (rustls has no external PSK); Object PLC Mapping in this crate through a
  declarative macro and a trait, so ADR-0018 holds; an opt-in FC05 for single-coil writes
  (upstream always sends FC15); profile schema version 2 with value-matched quality rules;
  the Linux I2C backend stays deferred until hardware.
- **E12** release 0.3.0 after this run, with G06 (E7) in it. This is the separate approval
  the line below asks for, for 0.3.0 only. Used: 0.3.0 was published and tagged on
  2026-10-29, so another version needs a new approval.


### Maintainer decisions (2026-10-30)

The maintainer accepted the ten M3 (OPC UA) decisions that S75 proposed, each as
recommended (option 1), and approved pushing the S82 spike branch. The full options are in
`docs/POST_ALPHA_PLAN.md`, "Decisions E13 to E22".

- **E13** vendoring `opc-manual.mspec`, `opc-status.mspec` and `opc-attribute.mspec`
  under `upstream/mspec/opcua/` and redistributing PLC4X's two OPC UA test suites under
  `fixtures/opcua/upstream/` are approved, with NOTICE carrying the ASF paragraph, the OPC
  Foundation MIT notice of the UA-Nodeset schemas (as a precaution) and a trademark line.
- **E14** the hybrid codec: the service-message layer is generated and ships `pub(crate)`
  under `opcua`; built-ins, the ExtensionObject envelope, framing, chunking and crypto are
  hand-written. Confirmed by the S80 numbers; hand-written structs are the fallback.
- **E15** `aws-lc-rs` with `rustls-webpki` carries the security policies, optional behind
  `opcua`, provided the S82 probe is green on all five targets with `cargo deny` unchanged.
- **E16** Basic128Rsa15 and Basic256 are refused with a typed `InvalidConfig` and their rows
  become deferred (deprecated); key and trust stores are PEM or DER only (no PKCS#12, no
  passwords); the runtime-generated client certificate is deferred.
- **E17** one opt-in `opcua` feature outside `default` and `full`; every M3 change is
  additive; the release number is a separate approval when M3 is done.
- **E18** the additive public APIs: `Connection::browse` with flat browse types,
  `PlcValue::Bytes`, a bounded `PlcValue::Struct`, `ReadItem::server_time`, an X.509 variant
  of `Credentials` with `DriverRegistry::connect_with` and a cache credential provider, and
  the native-subscription plumbing.
- **E19** the caller's operations drive Publish and renewal (no spawned task); after a
  transport loss a native subscription ends with a typed `Disconnected` and the caller
  re-subscribes. The maintainer accepts this as SPEC's "subscription recovery".
- **E20** python asyncua 2.1.0 is the second independent peer, pip-pinned with hashes in its
  own process and never vendored.
- **E21** a secure connect requires `server-certificate-file`, as upstream; the two-phase
  discovery flow is a later extension.
- **E22** the port is required (no 4840 default), the URI keys `username` and `password` are
  refused, and `tls.verify=false` is accepted only as that explicit key and is recorded in
  diagnostics.

Keep changes reviewable, commit tested slices one signed-off commit per green slice, and
maintain the per-module coverage gates. No first-party unsafe, panics on input, unbounded
I/O, silent sample loss, inferred byte order or automatic replay of ambiguous writes. Do
not mark planned capabilities supported.

Do not publish another crate version or operate hardware without separate approval.
Never place credentials in files, logs, commit messages or handoff documents.
