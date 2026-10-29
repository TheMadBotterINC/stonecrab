# 0009: Test and example tooling
## Status
Superseded in part by ADR-0018: "the testkit is unpublished" no longer holds;
`stonecrab::testkit` ships in the package behind the off-by-default `testkit` feature.
The rest of this ADR stands, with crate names read as module names.
Accepted by the sole maintainer on 2026-10-27, as slice S03b of docs/ALPHA_PLAN.md.
It records policy only: no dependency entry is added to the root manifest here. It
implements decision D23 for the test crates named below and builds on ADR-0005.

## Context
Several upcoming slices need test-only crates (property testing, golden-vector loading,
an XML reader for the upstream driver suite) and a fuzz workspace. The repository bans
`once_cell`, `lazy_static`, `log`, `time` and `chrono` (deny.toml, D23), and
`cargo-machete` runs in CI, so a dependency declared before its first consumer fails the
`machete` job. The choices and feature sets therefore need to be settled once, before
the first consumer lands.

## Decision
1. **Sanctioned test and example crates.** Each is added to the root
   `[workspace.dependencies]` as a minor-level caret range (CODING_STANDARDS 15.4), with
   its DEPENDENCY_AUDIT entry and the D23 `cargo tree -e normal,dev -i once_cell -i
   lazy_static -i log -i time -i chrono` check, in the slice that first consumes it. That
   keeps `machete` green. This ADR adds none of them.

   | Crate | Features | First consumer | Notes |
   | --- | --- | --- | --- |
   | `proptest` | `default-features = false`, `std` | S08 or S15, whichever lands first | `fork` and `timeout` are prohibited: they pull `tempfile` -> `once_cell`. Its `std` feature may enable `lazy_static`; UNVERIFIED for the version resolved at PR time, and the first consumer applies D23 and records the result. Slow property tests are bounded by nextest's slow-timeout, not by proptest's `timeout`. |
   | `serde` | `derive`, `std` | S04a | Unpublished testkit golden loader only. |
   | `serde_json` | `std` | S04a | Same. |
   | `roxmltree` | `default-features = false`, `std` | S04b | MIT OR Apache-2.0 per the defect report; its `forbid(unsafe_code)` status is recorded in the audit entry when it lands. Used only behind the testkit feature `fixtures`; never reaches a published crate. |

2. **Scope of the testkit.** `stonecrab-testkit` (under `tools/`) is unpublished and is
   the only crate that carries these loaders and adapters. Published crates may use
   `proptest` as a dev-dependency but never `serde`, `serde_json` or `roxmltree`. The
   testkit's line coverage threshold is 90% (ADR-0005), enforced by
   `scripts/coverage-gate.sh`; it stays excluded from `cargo package`.
3. **Property test volume.** CI sets `PROPTEST_CASES=256` as `env` on the `test` and
   `quality` jobs of `.github/workflows/ci.yml`. Regression files written by proptest are
   kept: `proptest-regressions/` directories are tracked in git, and `.gitignore` does
   not ignore them. CODING_STANDARDS 13.3 mentions 4096 cases on the scheduled run; the
   plan asks only for 256, and because the `test` and `quality` jobs also run on the
   schedule they get 256 there too. A 4096 override for the scheduled run is deferred.
4. **Fuzz layout.** `fuzz/` is its own Cargo workspace: the root `[workspace]` has
   `exclude = ["fuzz"]`, and the fuzz package (`stonecrab-fuzz`, `publish = false`) has
   its own `[workspace]` table, `Cargo.lock` (committed), `rust-toolchain.toml` and
   `deny.toml`. This keeps the nightly-only toolchain, the native libFuzzer build and
   their dependencies out of the root lock and the root `deny`, `machete` and `package`
   runs.
   - `libfuzzer-sys` (0.4) and `arbitrary` (1.5) are declared in `fuzz/Cargo.toml`, not
     in the root `[workspace.dependencies]`, with DEPENDENCY_AUDIT entries. The fuzz
     package depends on first-party crates by path; each slice that adds a target adds
     the path dependency it needs.
   - `fuzz/rust-toolchain.toml` pins the nightly by date: `nightly-2026-10-01`
     (rustc 1.101.0-nightly, 2026-09-30). CI passes the same date to
     `dtolnay/rust-toolchain`; bumping the date means changing both and amending this
     ADR. `cargo-fuzz` 0.13.2 is the pinned driver.
   - `fuzz/deny.toml` is a copy of the root `deny.toml` because `cargo-deny` 0.20 has no
     include mechanism. Its one permitted difference is `"NCSA"` appended to the license
     allow list, because `libfuzzer-sys` is licensed `(MIT OR Apache-2.0) AND NCSA`
     (vendored libFuzzer), so NCSA is required, not optional. `scripts/fuzz-smoke.sh`
     fails when the files differ in anything else, so the copies cannot drift unnoticed.
   - Targets are named `fuzz_<crate>_<parser>`; seed corpora live in
     `fuzz/corpus/<target>/` and crash inputs follow CONTRIBUTING "Fuzz findings".
     `fuzz/target` and `fuzz/artifacts` are ignored.
5. **Fuzz smoke job.** CI `fuzz-smoke` (also run on `workflow_dispatch`) and
   `mise run fuzz` both run `scripts/fuzz-smoke.sh`, which runs
   `cargo deny --manifest-path fuzz/Cargo.toml check` first, then every target for 60 s;
   with zero targets it prints that and exits 0. The weekly `scheduled` job runs
   `scripts/fuzz-smoke.sh 1800`. The plan's filter "only targets whose crate changed" is
   deferred until targets exist: every target runs on every invocation. For the same
   reason `workflow_dispatch` runs all targets, which equals a normal run today.
6. **Out of scope.** This ADR makes no statement about `tokio` or `tracing-subscriber`;
   ADR-0014 (S16, S14) decides them.

## Alternatives considered
- **honggfuzz or AFL instead of cargo-fuzz.** Rejected: libFuzzer through `cargo-fuzz` is
  the most widely used Rust setup and needs no extra system tooling beyond a C++ compiler.
- **Fuzz dependencies in the root workspace.** Rejected: they would put a nightly-only
  build and a native libFuzzer dependency into the root lock and the root `deny`,
  `machete` and `package` runs.
- **A per-crate `cargo-deny` license exception in the root `deny.toml`.** Rejected: the
  root run does not see the fuzz workspace, and the entry would loosen the root policy
  for a crate it never resolves. A separate policy file with a drift check keeps the
  exception next to the code that needs it.

## Compliance
The `fuzz-smoke` CI job and `mise run fuzz` run `scripts/fuzz-smoke.sh`, which fails when
`fuzz/deny.toml` differs from `deny.toml` in anything but the NCSA entry and runs
`cargo deny --locked` on the fuzz workspace.

## Consequences
- The root manifest changes by one line (`exclude`), and the root `Cargo.lock` does not
  change.
- The fuzz workspace needs a C++ compiler and `cargo-fuzz` locally; a missing nightly is
  installed by rustup on first use.
- The plan sentence "fuzz/deny.toml includes the root policy" holds only with the extra
  NCSA allowance; the plan wording should be amended to say so.
- Each consumer slice owns its own D23 check and audit entry.

## Addendum (S04a): the testkit `fixtures` feature and the provenance schema
Slice S04a adds the golden-vector loader to `tools/stonecrab-testkit` behind the feature
`fixtures = ["dep:serde", "dep:serde_json", "dep:toml"]`, off by default so that core's
dev-dependency on the testkit stays free of the serde stack. It is the first consumer of
`serde` and `serde_json` named in decision 1: the `[workspace.dependencies]` entry
`serde_json = { version = "1.0", default-features = false, features = ["std"] }` and its
DEPENDENCY_AUDIT entry land here, and the testkit enables `serde`'s `derive` and uses the
existing `toml` entry (features `parse`, `serde`, `std`). `roxmltree` stays with S04b.

**Scope of this slice.** The loader, the provenance format and the first-party interop
register map land now. The verbatim upstream XML copies, the six upstream `.hex` vectors
and the `NOTICE` update wait for maintainer prerequisite P4 (written approval to
redistribute the Apache-2.0 vectors); no upstream file or test vector is in the
repository, and `fixtures/modbus/PROVENANCE.toml` lists none. `fixtures/` is not in any
published crate's `include` list (all seven are `src/**`, `README.md`, `LICENSE`,
`NOTICE`), which `mise run package` verifies.

**The `PROVENANCE.toml` schema.** One file per vector directory, two optional arrays of
tables, unknown keys rejected:
- `[[vector]]`: `name` (path relative to the file, no extension, `/` separated, each
  component ASCII letters, digits, `-`, `_` or `.` and not starting with a dot, unique),
  `source` (`upstream_fixture`, `protocol_spec`, `captured` or `adversarial`), `origin`
  (upstream file and revision, or the specification section), `license` (SPDX
  expression), optional `must_fail` (default false) and optional `note`.
- `[[file]]`: `path` (relative, no `..`, root or prefix component, unique, must exist),
  `origin`, `license` and optional `note`, for first-party fixtures that are not wire
  vectors, such as `interop/register-map.toml`.
- `origin` and `license` are never blank.

`load_dir(dir)` reads `dir/PROVENANCE.toml`, then enforces the rule "every vector is
listed": a `.hex` or `.expected.json` file anywhere under `dir` (symbolic links are not
followed) that no `[[vector]]` names is an error, as is a listed vector without a `.hex`
and a `[[file]]` that does not exist. A `.hex` file holds exactly one data line of hex
digits with whitespace allowed and `#` comments and blank lines skipped; an odd digit
count, a non-hex character, a second data line or no data line is an error. The optional
`<name>.expected.json` is parsed, never computed, so it stays independent of the decoder
(CODING_STANDARDS 13.6). Every error names the file. `GoldenVector.expected` is `None`
when the file is absent, and a `must_fail` vector needs none.

**The interop register map.** `fixtures/modbus/interop/register-map.toml` is authored from
the `MB-INTEROP-*` rows of `docs/parity/modbus.md` and decision D8. `InteropMap` parses
it with the same crates, so the S27 server generator and the S25 contract fixture read one
source. Register words are TOML hexadecimal integers, bit areas are booleans, every entry
carries its row id, and `[sizes]` holds the block sizes from the prose (201 holding
registers, 2000 coils, 2000 discrete inputs, 21 input registers). The little-endian
region (registers 300 and 301, `0xDC0F 0x4940`) sits in its own `holding_extension`
array because it lies outside the upstream block. A testkit test re-encodes each entry's
stated `value` and compares it with its `words`.

**D23 check (2026-10-27).** `cargo tree -e normal,dev -i once_cell -i lazy_static -i log
-i time -i chrono` fails with "did not match any packages" because four of the five names
are absent, so each name was queried alone, with the `fixtures` feature on (`--workspace
--all-features -e normal,dev,build`) and off:
- `once_cell`: one path, `once_cell` through `tracing-core` through `tracing` through
  `stonecrab-driver-sim`, already wrapped in `deny.toml` (ADR-0012). `serde_json` (with
  `itoa`, `memchr`, `zmij`), `serde` and `toml` add none.
- `lazy_static`, `log`, `time`, `chrono`: no package matches. No `deny.toml` change.

## Addendum (S13): the driver contract suite
Slice S13 adds `contract` to `tools/stonecrab-testkit` (SPEC.md 'Test layers', contract
suite). It needs no feature, no dependency and no root-manifest change: it uses only
`stonecrab-core`, so core's dev-dependency on the testkit stays free of Tokio and serde.

**What it is.** `ContractSuite::run(driver, uri, fixture)` returns an `OpFuture` that always
resolves to a `ContractReport`; the caller drives it with `block_on_with` on the fixture's
`FakeClock`. The suite parses `uri` with `driver.parse_config`, connects once per case so
that a case that wrecks a connection (disconnect, close) never poisons the next, and runs
23 cases named as sentences in a fixed order. Each case carries the mandatory case it
covers (`ContractCase.covers`) and what it needs (`ContractCase.requires`); the report
prints both. An assertion never panics: it ends the case with `Failed(reason)` naming the
observed value, and a driver that fails to parse or connect fails every case that needs
the connection instead of aborting the run.

**Skip policy.** A case is `Skipped(reason)` and never omitted when the driver or the
fixture cannot exercise it: `driver reports read unsupported`, `driver reports write
unsupported`, `fixture cannot inject <fault>`, `fixture has no writable address`, and, for
the one case that needs an unsupported write, `driver reports write supported; the case
needs it unsupported`. `ContractReport::is_green` is "no failure and at least one pass",
so a run that skips everything is not green. A skip is a statement about the fixture, so
each driver crate's test asserts the skips it expects by name; the simulator expects
exactly one.

**Fixture knobs.** `ContractFixture` is the suite's only view of the device:
- required: `clock` (the clock the driver's connections use; it must be deterministic,
  because the suite builds deadlines from it and checks that a lost reply fails after
  exactly `timeouts.request`), a `readable` address text, a `rejected` address text
  (parses, but the device refuses it), a `foreign` address (an `Arc<dyn ProtocolAddress>`
  that is not the driver's), an `io_probe` counter that must not move when the suite says
  "no I/O" (for the simulator, device reads plus `sink_writes`) and a `device_reads`
  counter;
- optional: `writable` (an address text and a value of its type, for the write cases),
  `declares_device_time` (so `source_time` is expected instead of forbidden), and the arming
  hooks `inject_disconnect`, `inject_timeout`, `inject_device_exception` and
  `inject_unknown_after_transmit`, each arming one failure for the next operation that
  reaches the device.

Two knobs that the plan considered were not needed. The retry policy and the access policy
are rebuilt from the parsed `ConnectionConfig` through `ConnectionConfig::builder` (every
field has an accessor), with `max_attempts` 3 and `AccessPolicy::ReadOnly`, so there is no
`retry_max_attempts` or `read_only_uri`; a configuration that carries credentials cannot
be copied (`Secret` is not `Clone`) and those cases skip. The `uri` must not itself raise
the read retry above one attempt. The fixture hooks arm faults on a device shared by all
connections, so after every case that armed a fault the suite reads until a read succeeds
(a leftover step is drained) and fails the case if it cannot.

**Not a suite case.** The queued-cancellation case (F4 queued) needs a wait queue. The
simulator has none, so `ConnectionGuard` covers it (S19) and the suite does not.

**Plan wording that changed.** "Every failure carries diagnostics" is four cases (rejected
before I/O, timeout, disconnect, device exception) so that one missing hook skips one
assertion by name. The device-exception retry case asserts at most one device read, not
exactly one: the simulator's `DeviceException` leaves the device untouched, and what the
case proves is that the refusal is surfaced and not retried. `Connecting` cannot be
observed from outside a driver (a connection exists only after `connect` returns), so the
lifecycle case checks `Connected` on the returned connection and then `Closing` (when the
close does not finish on its first poll) and `Closed` by value through `StateWatch`.

**Where it runs.** The testkit's own tests cover the report, the assertion helpers and the
skip logic with an in-crate driver that supports nothing (CODING_STANDARDS 13.5 makes the
testkit a home for doubles). The full run is `crates/stonecrab-driver-sim/tests/contract.rs`,
which also breaks the fixture on purpose (a probe that moves, a counter that never moves,
a rejected address that reads, a device time the simulator lacks, a hook that arms
nothing) and asserts that the named case fails, so a suite that cannot fail would be
noticed. Later drivers (S25, S31) add a run of their own.

## Addendum (S25c): responder reactions
Slice S25c adds one step and one enum to the testkit's `net` feature so that a responder
can end the connection on demand. It changes no dependency and no earlier decision.

**Why.** `Step::Respond` takes a `FnMut(&[u8]) -> Option<Vec<u8>>`: it can answer or stay
silent, and nothing else. A fixture for a whole contract suite shares one device over many
connections and must decide per request, from armed faults, whether to answer, swallow
the request or drop the link. S25b could do the first two and had to skip the two
disconnect cases of the S13 suite by name (`fixture cannot inject disconnect`, ADR-0015
decision 14), because a closure that returns `None` leaves the connection open.

**The step.** `Step::React(Reactor)`, with `pub type Reactor = Box<dyn FnMut(&[u8]) ->
Reaction + Send>` and the constructor `Step::react(closure)`. `Reaction` has four
variants: `Reply(Vec<u8>)`, `Silence`, `Close` and `ReplyThenClose(Vec<u8>)`. `React`
behaves like `Respond` in everything else: it is called with the bytes received since the
previous input step, it never finishes (so steps after it do not run and one fixture serves
a whole suite), armed modifiers apply to the next reply, and it counts once in
`ScriptReport::ran` and each call in `ScriptReport::responses`. `Respond` and `Responder`
are unchanged (`Some` maps to `Reply`, `None` to `Silence`) and share the machine's code
path, so the two cannot drift.

**Mapping.** The pure script machine maps a reaction to the actions that both doubles
already consume, so neither double needed a new action: `Reply` is a write; `Silence`
produces no action and the step stays armed (modifiers stay pending for the next reply);
`ReplyThenClose` is a write whose `close` flag is set, with armed modifiers applied to
the bytes and an armed `CloseAt(n)` still cutting them at `n`; `Close` is `Action::Close`,
drops pending modifiers and halts the script, which marks `ScriptReport::closed`. A
reaction close is not a failure (`failed` stays false).

**Both consumers honour it.** `FakeTransport` queues the close behind any bytes still
queued, so `recv` returns the bytes, then `Ok(0)` once, then `Disconnected`, and `send`
fails `Disconnected`, exactly as after `CloseAt`. `ScriptedPeer` writes the bytes and
half-closes the loopback socket (a FIN, keeping the socket so unread client bytes cannot
turn the close into a reset), as for `CloseAt`. The S26 `ScriptedPeer` contract run uses
the same hook as the S25b run over `FakeTransport`, so one responder drives both.

**Why not extend `Responder`.** Changing the return type would break every existing
`Step::respond` caller and make the common "answer or stay silent" case wordier. A sibling
step keeps `Respond` source-compatible and leaves the new capability opt-in.

**Tests.** The machine tests in `script.rs` cover every reaction's action, the armed step,
the modifiers and the counters; `fake_transport.rs` covers the close and
`ReplyThenClose` on the fake clock; `peer.rs` covers the half-close and `ReplyThenClose`
on loopback. `crates/stonecrab-modbus/tests/contract.rs` wires `inject_disconnect` to
`Reaction::Close` and the two disconnect cases pass; the run now reports only the two
write skips.

## Addendum (S26): `ScriptedPeerFactory` and the `net` feature
Slice S26 runs the driver contract suite over real loopback sockets, which needs a
`TransportFactory` that hands each connection to a `ScriptedPeer`. CODING_STANDARDS 13.5
allows a double of our trait only in the testkit or the simulator, so the testkit gains
`ScriptedPeerFactory` (`src/peer_factory.rs`) behind the `net` feature, and `net` now also
enables `stonecrab-transport/tcp`, because the factory opens the real TCP transport to the
peer's loopback address. The factory is unpublished with the rest of the testkit and does
not change the testkit's default features, so core's dev-dependency stays Tokio-free.

**Tests.** `peer_factory.rs` carries its own unit tests; its one unexercised line is the
`Internal` error for a peer task that panicked, which a `ScriptedPeer` cannot be made to
do. `crates/stonecrab-modbus/tests/contract_scripted_peer.rs` is the consumer.

## Addendum (S27): `CountingFactory`
Slice S27 runs the driver contract suite against a real server in a container, where the
test owns neither the socket nor the device's request counter. The suite's `io_probe` must
not move when it says "no I/O", so something on our side of the wire has to count. As a
double of our `TransportFactory` trait it lives in the testkit (CODING_STANDARDS 13.5):
`CountingFactory` (`src/counting_factory.rs`, feature `net`) wraps an
`Arc<dyn TransportFactory>` and returns transports that wrap the ones the inner factory
opened.

It counts `open` calls, `send` calls (frames) and the bytes given to `send`, each when the
call is made, whether or not it succeeds: a probe that does not move proves the driver
never reached the transport. `recv` and `shutdown` are delegated and not counted, since
they transmit no application bytes. `io()` is opens plus frames, the number a
`ContractFixture` wants as its `io_probe`. Clones share the counters, so a test keeps a
clone while the driver owns the `Arc`. The wrapper reports the inner transport's `peer` and
`limits`, and it changes no error, deadline or byte.

**Tests.** `counting_factory.rs` carries its unit tests over `FakeTransportFactory` (counts
across two transports, delegation of `recv` and `shutdown`, `peer` and `limits`, a failed
open and a refused oversized send still counted, shared counters and `Debug`) and a
compiling doctest on the type and on every method. The consumer is
`crates/stonecrab-modbus/tests/interop_pymodbus.rs`: its contract-suite test uses `io()` as
the probe and the frame count as `device_reads`. S26's loopback run keeps its own probe
(opens plus the requests the responder counted), which is exact there, and does not use
this double.

## Addendum (S04b): `roxmltree` is a dev-dependency and the adapter is a test
Slice S04b adds the driver-suite XML adapter. Decision 1 and decision 2 placed `roxmltree`
in the unpublished testkit behind a feature `fixtures`. Since ADR-0018 the testkit is a
feature of the published crate, so a feature-gated `roxmltree` would be an optional
dependency of the published manifest. It is instead a **dev-dependency** of `stonecrab`
(`[workspace.dependencies]`: `roxmltree = { version = "0.21", default-features = false,
features = ["std"] }`, DEPENDENCY_AUDIT entry added), and the adapter lives in the test
target, `crates/stonecrab/tests/modbus_driver_suite_adapter.rs`, not in `src/`. This
relaxes decision 2 for `roxmltree` only, as a dev-dependency of `stonecrab` (build and test
time, never a dependency or a feature). The published crate gains no (non-dev) dependency,
no feature and no source line;
`cargo tree -e normal --all-features -p stonecrab` does not list `roxmltree` (checked on
2026-10-28), and `positions`, the one other feature of the crate, stays off.

**What the adapter does.** It parses `DriverTestsuite.xml`, `Modbus-all-datatypes.xml` and
`Modbus-all-datatypes-little-endian.xml` (retained since S04a) and finds the XML-serialized
`ModbusADU` of every outgoing and incoming step. Its own serializer follows
`protocols/modbus/.../modbus.mspec` (the PDUs that occur are a five-row table; `length` and
`byteCount` are derived, as the mspec has them, and the adapter fails when the XML spells
out another value) and never calls Stonecrab's codec or decoder (SPEC.md 'Code generation
and upstream maintenance', requirement 7). For each read exchange it renders the request
and the response as `tcp/adu_upstream-<suite>-<case>-ex<n>-<request|response>` (`.hex`,
`.expected.json`, and a `[[vector]]` entry in an adapter-generated block of
`PROVENANCE.toml`) from the same model. It fails on any step shape it does not know (a raw
bytes step, an exception PDU, an unlisted PDU, a width that differs from the mspec).
`adapter_serializes_each_driver_suite_adu_deterministically` re-runs it and compares the
committed files and the block byte for byte, so drift or a hand edit fails; running that
test once with `STONECRAB_REGENERATE_ADAPTER_VECTORS` set rewrites them.

**Result.** The alpha plan's assumption that the all-datatypes files use the
`DriverTestsuite.xml` model format holds: all three files use the same
`outgoing-plc-message` and `incoming-plc-message` steps with an XML `ModbusADU`. The
adapter counts 4 read exchanges in `DriverTestsuite.xml` (single 1, array 1, multi-element
2) and 14 in each all-datatypes file, 64 vectors in all; the 2 + 14 + 14 write cases are
serialized and checked but not emitted (S45 and S46 own writes). The "Query/Response"
pcap-derived cases are 28 per all-datatypes file inside an XML comment, so no runner loads
them; a test feeds their `<xml>` models to the serializer and requires their `<raw>` bytes
back, which is an independent check of the serializer.

**D23 check (2026-10-28).** `cargo tree --workspace --all-features -e normal,dev,build -i
<name>` for each name: `once_cell` and `lazy_static` keep their one wrapped path each
(`tracing-core` and `sharded-slab`); `log`, `time` and `chrono` match no package.
`roxmltree` adds a single crate, `memchr`, which is already in the tree through
`serde_json` and has no path to a banned crate. No `deny.toml` change.

## Addendum (S52): benchmarks and the 10% regression gate
Slice S52 adds the first benchmarks (SPEC.md 'Performance', CODING_STANDARDS 16) and the
scheduled gate that compares two builds of the tree. It adds one dev-dependency and no
runtime dependency, feature or source line.

**Harness: `criterion` 0.8.2.** CODING_STANDARDS 15.8 names `criterion` or `divan` "per
ADR"; this is that ADR: `criterion`. `[workspace.dependencies]` has `criterion = { version
= "0.8", default-features = false }` and `stonecrab` takes it as a dev-dependency. With the
defaults off there is no `rayon` and no `plotters` (so no HTML reports; `cargo_bench_support`
is an empty marker). It passes the dependency policy: the D23 check finds `once_cell` and
`lazy_static` only on their two existing wrapped paths and `log`, `time` and `chrono` not at
all, every license is already allowed, and `mise run dependencies` exits 0 with `deny.toml`
unchanged. `cargo tree -e normal --all-features -p stonecrab` does not list it. The
DEPENDENCY_AUDIT entry records the 30-odd transitive crates it adds, the one native build
step (`alloca` compiles a C file through `cc`) and why `divan` was not pursued. No lighter
alternative was needed.

**Targets.** `benches/modbus_wire.rs` (feature `modbus`) times address and connection-string
parsing, PDU encode and decode (a request, and a 2-register and a 125-register response),
MBAP encode and decode of the same two frames, `TypedDecoder` over a 125-register response
read as 62 floats and as 125 integers, and `Batcher::plan` for a 28-tag plant set and 1024
adjacent registers. `benches/modbus_loopback.rs` (features `modbus` and `testkit`) times
`Connection::read` through the registry, the real `TcpFactory` and the Tokio clock against a
`ScriptedPeer` on `127.0.0.1` in the same process, for 1 tag, 24 tags in 2 requests and 125
tags in 1 request: the pair the S26 loopback tests use, so no new double. Both are
`harness = false` and have their own `main` instead of `criterion_main!`, because the macro's
`fn()` signature cannot return an error and the lints deny `expect`; the run proves one
success outside the timing and fails if a measured read errors. Benches use only the public
API. They build under `[profile.bench]` (`release` with `overflow-checks`, thin LTO, one
codegen unit), which is also what the library ships with. Nothing sleeps; the numbers are
library overhead plus loopback, not network or device latency.

**Not covered.** SPEC.md and CODING_STANDARDS 16.2 ask for allocation counts, multi-device
polling, subscription tail latency, soak figures and binary size. Allocation counts need a
counting `GlobalAlloc`, which is `unsafe` and so forbidden in first-party code (4.3); it waits
for an ADR that proposes an external audited crate. The rest need features that do not exist
yet (S56 and later) or a soak rig.

**The gate: `scripts/bench-compare.sh BASE_REF` (`mise run bench-compare -- BASE_REF`).**
A stored baseline cannot be compared on a shared runner: its speed and neighbours differ
from run to run and from the machine that recorded the number. So the scheduled job
(weekly, `if: github.event_name == 'schedule'`; never on a pull request or push) builds the
working tree and `BASE_REF` (the newest commit at least seven days old, exported with `git
archive`) and runs both on the same runner, three rounds, alternating which side goes first,
each round a full Criterion run (1 s warm-up, 3 s measurement, 50 samples per benchmark,
`BENCH_ARGS` overrides) that saves the baseline `r<round>`. Per benchmark and round it takes
the median and its 95% confidence interval. A round separates when `head.lower >
base.upper * 1.10`: the head is more than 10% slower even at the favourable end of both
noise bands. A benchmark regresses, and the step fails, only when every round separates.
Other outcomes are reported, never failed: "slower, within noise" (median ratio above 1.10
without separation), "improved", "ok" and "unpaired" (a benchmark on one side only, so a
benchmark added by a commit does not fail its first week). A base that predates S52 has no
benches: the first scheduled run after this slice ends with a notice and status 0. The report
(`report.md`, `report.json`) goes to the job summary and an artifact.

**Limits.** The gate flags a regression only when it exceeds 10% plus the width of the two
bands, so a 11% slowdown on a noisy runner passes; it cannot see a slowdown that comes and
goes between rounds, a drift smaller than the weekly step spread across several weeks, or
anything the benchmarks do not exercise. Loopback reads are the noisiest benchmarks. Release
CODING_STANDARDS 16.4 says "the last release's stored baseline on the same runner class";
this gate compares with last week's commit instead, and the release checklist's "within 10%
of baseline" (20.3) is still a manual comparison with `mise run bench-compare -- <tag>`.
`--self-test` exercises the verdict logic on synthetic Criterion output (steady, noisy, a
spike in one round, faster, added, and a 13% regression). The whole script was also run on a
scratch repository whose head carried an injected slowdown in `ModbusAddress::parse`
(20 samples, 1 s per benchmark): it exited 1 naming the two benchmarks that got 3.6 times
slower and passed the other twenty; a 1.25 to 1.5 times slowdown of a third benchmark was
reported as "within noise" because of that short run's wide bands, which is the limit above in
practice. The base and the head build into separate target directories: Cargo's unit hash
ignores the checkout path, so a shared directory reused the base's artifacts for the head and
the first trial of the script compared a binary with itself.

**Baseline.** `docs/evidence/benchmarks/` records one local run (machine, OS, toolchain, date,
commit, each benchmark's estimate and interval) and a short report. A laptop baseline does
not transfer to CI runners and is not an input to the gate; it shows orders of magnitude
and gives the next local run something to compare with.

**Compliance.** The scheduled step runs the script every week. `mise run check` lints both
bench targets (clippy `--all-targets --all-features`) and `cargo bench --no-run --locked -p
stonecrab --all-features` builds them (a check of this slice, not a CI job); no pull
request or push runs a benchmark.
`scripts/bench-compare.sh --self-test` checks the verdict logic and needs only `jq`.
