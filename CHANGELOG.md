# Changelog
All notable changes to this project are documented here, in the style of Keep a
Changelog. Stonecrab is one crate, so there is one section for the alpha. It is published
as `stonecrab` 0.2.0 (2026-10-27); `stonecrab` 0.1.0 is the specification-stage
placeholder.

## Unreleased
- Lists and Modbus array selections (ADR-0022; additive): `PlcValue::List(Vec<PlcValue>)`
  and `PlcValueType::List`, appended after `DateTime`; equality compares the elements in
  order, and nothing is coerced. Modbus addresses take the upstream array notation on every
  form: `[i]` reads element `i` as a scalar, `[lo..hi]` (inclusive) reads one `List`, even
  `[4..4]`, and `;base` states the declared lower bound (`holding-register:1[4..7;1]:INT` is
  the elements at offsets 3 to 6). Offsets count elements of the type (`[4]:DINT` is eight
  registers along). A selection of more than one dimension, a reversed range, an index
  below its base, a number beyond `u32`, a span past wire address 65535 and a range wider
  than one read (2000 bits, 125 registers) are `InvalidAddress` at parse time. New
  `ModbusAddress::elements` and `ModbusAddressBuilder::elements`, and an `elements` field
  on the batcher's `Member`. A range is one tag to the batcher and one read item, each
  element decoded as it would be alone in its byte and word order; a `BOOL`, `BYTE`,
  `SINT` or `USINT` element takes a register of its own, where upstream packs them (a
  recorded divergence). Writes take a `List` of exactly the element count: function 15 for
  coils, 16 for holding registers, 06 for a one-register payload; a scalar to a range or a
  list to an index is refused by the request builder, a list of another length is the
  item's `InvalidConfig` rejection and a list wider than one write (124 registers, 1969
  coils) its `ResourceLimit` rejection, with no request. The legacy-form hint now gives
  `[0]` for a count of 1, as upstream, and offers a replacement only when it parses
  (none for a range wider than one read or past wire address 65535). The two
  upstream array exchanges of `DriverTestsuite.xml` replay byte for byte. Parity rows
  `MB-ADDR-SELECTION` (implemented) and `CORE-VALUE-LIST` (new).
- Calendar values (ADR-0021; additive): `PlcValue` and `PlcValueType` gain
  `SignedDuration`, `Date`, `TimeOfDay` and `DateTime` after `Duration`, holding the new
  root types `PlcSignedDuration` (an `i64` nanosecond count; IEC `TIME` and `LTIME`),
  `PlcDate` (an `i32` day count from 1970-01-01, proleptic Gregorian; `DATE`, `LDATE`),
  `PlcTimeOfDay` (`00:00` to `23:59:59.999999999`, no leap second; `TIME_OF_DAY`,
  `LTIME_OF_DAY`) and `PlcDateTime` (a date and a time of day; the three date-and-time
  types). None carries a time zone. Range-checking constructors return `InvalidConfig`;
  ordering is chronological; `Display` prints the `java.time` text of the matching PLC4J
  type (`PT-1.5S`, `2024-02-29`, `13:05`, `2026-10-28T06:30:15.250`). No new dependency.
  No driver decodes calendar values yet; the Modbus `TIME` and `DATE` names stay rejected,
  since upstream has no Modbus encoding for them. New parity row `CORE-VALUE-CALENDAR`.
- Documented device maps and profiles (no crate API change): the register and coil tables of
  the AutomationDirect STRIDE SIO-MB04THMS and SOLO SLM1-C, transcribed by hand from the
  vendors' documents under `fixtures/modbus/devices/`, and one named device profile each
  under `profiles/documented/` (`validation = "documented"`: a thermocouple module with its
  break bits and degree units as status fields, and a controller whose process variable
  keeps its error codes out of `Good`). All four carry the label "documented map, not
  hardware-verified": no device was read. The vendor documents are pinned by URL and sha256
  in `fixtures/modbus/devices/PROVENANCE.toml` and are not copied. The new test
  `modbus_device_replays` serves each map through the first-party responder with the
  vendors' request limits and function tables enforced and runs what the hardware records
  will run: every documented register and coil, the limits, the break flag driving `Bad`,
  the error codes and the documented write targets. The SOLO suite is a PDU-level replay over
  TCP framing until the serial driver exists. Runbooks `docs/hardware/hw1-sio-mb04thms.md`
  and `docs/hardware/hw2-slm1-c.md` list the steps and everything the documents leave open.
  Findings: the SOLO manual lists function codes 05 and 16 but not 15, so a Stonecrab coil
  write cannot reach it yet, and schema version 1 cannot make a process variable error code
  `Bad` (it is `Uncertain`). New parity rows `MB-DEVICE-SIO-*` and `MBS-DEVICE-SLM1C-*`.
- Fixed (ADR-0014 addendum S57, note S57b): on a multi-thread runtime a `close` that
  overtook the supervisor's idle close could drop that transport without shutting it down;
  the idle close now keeps the transport in the guard until its shutdown has returned, and
  `close` shuts down what it finds there. `GuardStats` gains `discarded_frames` (additive;
  the struct is `#[non_exhaustive]`): frames the guard read and dropped between exchanges,
  such as a late reply the supervisor drops before the Modbus session can count it.
- New `serial` feature (off by default; `modbus` does not enable it yet): a serial transport
  on a blocking worker thread, `stonecrab::transport::{SerialFactory, SerialTransport}` with
  `DataBits`, `Parity`, `StopBits` and `FlowControl` (upstream's option names and defaults:
  9600 baud, 8 data bits, no parity, 1 stop bit, no flow control). One thread owns the port
  and talks to the async caller over a bounded channel; every call honors its `Deadline`,
  a `send` into a stalled line ends at its deadline and frees the worker within 20 ms, a
  dropped `recv` loses nothing (even one dropped after the worker answered), `shutdown` joins
  the worker and releases the port, and a port error or a closed peer surfaces as the TCP
  transport's error kinds. The port name is an `Endpoint::Custom`; `SerialTransport::from_port`
  wraps a port you opened. New optional dependency `serial2` 0.2.39 (`default-features =
  false`), BSD-2-Clause OR Apache-2.0, which adds no other crate to the lock file and needs
  no `deny.toml` change. Additive API only. Not offered yet: `dtr`, `rts`, `reuse-port`,
  `interframe-delay`, mark and space parity (docs/parity/transport.md). Tested over a
  pseudo-terminal pair on Unix other than macOS; no hardware. ADR-0014 addendum S58.
- Address conventions `zero-based` and `one-based-40001` (additive): `AddressConvention`
  gains `ZeroBased` and `OneBased40001` after `Plc4xCompatible`, with `name()` and
  `from_name()`. `address-convention` (connection string, `AddressOptions`, a sensor
  profile's `address_convention`) now accepts `plc4x`, `one-based-40001` and `zero-based`;
  any other name is still an error. `one-based-40001` reads every text as `plc4x` does
  (numbers from 1, bare forms accepted). `zero-based` reads every numeric form (`AREA:N`,
  `Nx` digits, bare `4NNNN`) as the wire offset, numbers 0 to 65535. With no convention
  nothing changes: the long and prefixed forms are one-based and the bare forms are
  refused. A parsed address is the wire offset and still carries `Plc4xCompatible`;
  `canonical()` stays the one-based long form. The fuzz target `fuzz_modbus_address` reads
  every text under all four options and gains five seeds. New parity row `MB-ADDR-CONVENTIONS`.
- Decision, no behavior change: the batcher still never reads through a gap between tags.
  Upstream merges tags up to the request limit with no gap option and no way to turn it
  off, and a hole in a device's map would turn that into exception 02 for every tag of the
  merged request. `MB-READ-GAP` is recorded as a decided divergence (ADR-0015 addendum)
  and a test shows the two requests on the wire.
- Interop (no crate API change): a second independent Modbus TCP peer, libmodbus 3.1.11's
  `unit-test-server` (C, from Debian trixie's `libmodbus-dev` 3.1.11-2 on an image pinned by
  digest, restarted after each connection), in the new `interop-libmodbus` CI job and `mise
  run interop`. Ten tests in `tests/modbus_interop_libmodbus.rs` cover reads, writes read
  back (FC15, FC06, FC16), exception 06, a reply with the wrong transaction and unit id, a
  500 ms reply inside and past a deadline, a reply sent one byte at a time and a short reply
  to a two-register read (`MalformedData`, no raw bytes). The pymodbus tests share their
  `Peer` helper with the new ones through `tests/support/interop.rs`.
- Polling subscriptions (slice S55, ADR-0020 addendum; additive): the simulator and
  `modbus-tcp` override `Connection::subscribe` and report `SubscriptionMode::Polling`.
  Cyclic subscriptions poll at their interval and deliver every reading; change-of-state
  ones poll every 1000 ms (upstream's default, disclosed through `Subscription::interval`)
  and deliver a tag's first outcome and each change; `Event` is refused with
  `Unsupported(Subscribe)` before any I/O. Each tick is one read through the connection's
  own read path (deadline, retry, batching, the single in-flight slot), the first at once
  and then on a grid of the interval, run while `Subscription::recv` waits: no task is
  spawned. Dropping a waiting `recv` cancels the read it left in flight. Ticks that come
  due during a read, while nobody waits, while the slot is taken or while the link is
  re-established are skipped, never made up, and counted in the new
  `LossCounters::skipped`, which the polling producer bumps. Closing the connection, or
  its terminal failure, ends a subscription after its queued events, and dropping the
  connection without `close` ends a wait for the next tick at once. A change-of-state
  subscription on a clock whose sleeps resolve at once (the simulator's default
  `RealtimeClock`) is refused with `Unsupported(Subscribe)`, because its polling would
  never pause; a cyclic one there takes one fresh sample per `recv`. `Subscription::recv`
  now takes `&mut self`, so only one `recv` can be pending (the type is unreleased), and a
  producer that panicked is dropped instead of polled again. The contract suite gains
  `Requires::Subscribe` and six subscription cases (33); the unsupported-subscribe case is
  skipped by name for both drivers. Behavior change: `Capabilities::subscribe` of both
  drivers is `Polling` instead of `Unsupported`.
- Eager background reconnect and `Timeouts.idle` (ADR-0014 addendum S57; additive):
  `ConnectionGuard::supervise` starts one task, owned by the guard, on the current Tokio
  runtime. It notices a peer that closes while nothing is asked of it, reopens the
  transport when the reconnect backoff passes without waiting for a request (attempt after
  attempt, ending `Failed` when `ReconnectPolicy::max_attempts` is spent), and publishes
  every state through `watch_state`. `close` stops and joins it within `timeouts.request`;
  dropping the guard aborts it, and a task that ended with its runtime is replaced by the
  next call. The reader runs in every state of a live link: the late reply of a timed-out
  request is dropped and a drop that follows is noticed; a reconnected link that stays up
  for a whole read resets the attempt count; a frame it reads while nothing is owed is
  kept for `next_reply` and dropped by the next request, and behind it the reader still
  notices a peer close. With a supervisor, `Timeouts.idle` is enforced: a link no request
  has used for that long is shut down (with a kept frame, if any), the state stays
  `Connected`, the next request opens a new one, and `next_reply` fails `Disconnected`.
  `ConnectionGuard::request_matching(frame, deadline, token, accept)` keeps the in-flight
  slot from the send to the first reply `accept` takes; the Modbus session now correlates
  with it, so neither the supervisor nor the idle bound can come between a discarded reply
  and the real one. `ModbusConnection::open` starts the
  supervisor when it runs inside a Tokio runtime; outside one (the testkit's
  `block_on_with`) reconnect stays request-driven and `idle` is ignored, as before.
  `GuardStats::reconnects` now also counts reopens by the supervisor and after an idle
  close.
- Interop (no crate API change): unit-ID correlation against a scripted multi-unit server, a
  short first-party script on modbus-tk 1.1.5 (LGPL-2.1-or-later, installed from PyPI into a
  `python:3.12-slim` image pinned by digest) that serves units 1, 2 and 247 with a map each
  and does not answer any other unit, in the new `interop-modbus-tk` CI job and `mise run
  interop`. Eight tests in `tests/modbus_interop_modbus_tk.rs` cover each unit's map on one
  connection, one batch across units, alternating units and function codes, a unit that
  refuses an address the others serve (exception 02), an unknown unit ending in `Timeout`
  at the deadline with the connection still good, a batch with an unknown unit, the unit in
  the address block over `default-unit-identifier`, and a write to one unit. The server is
  first-party code, so the evidence is `interoperability-tested` at most.
- Interop (no crate API change): failure paths on a real clock. The new `interop-toxiproxy`
  CI job and `mise run interop-toxiproxy` put Toxiproxy 2.13.0 (MIT, pinned by digest, API on
  the loopback of the host) between the driver and the pymodbus and libmodbus peers. Ten
  tests in `tests/modbus_interop_toxiproxy.rs` add and remove toxics over the Toxiproxy HTTP
  API from the test process (a small client on `std::net`, no new dependency): a reset in
  each direction, a black hole in each direction (`Timeout` at the deadline, twice, then the
  link recovers), latency below and above a deadline with jitter, slicer toxics that cut
  requests and replies into pieces of a few bytes, `limit_data` after one whole reply and
  inside a header, a disabled proxy with open connections, and `docker pause`, `docker kill`
  and `docker start` on the pymodbus container (a hung peer is `Timeout` and its late reply
  is discarded on the same connection; a dead peer is `Disconnected` until it is back). Each
  failure ends in `Timeout` or `Disconnected` within its deadline and the next request reads.
  tshark 4.4 (Debian trixie) captures the run in the proxy's network namespace and decodes
  it independently: no malformed packet, every driver request one well-formed Modbus/TCP
  frame. Evidence under `target/interop-evidence/toxiproxy/`.
- Subscription types and a bounded event queue (ADR-0020; additive): `SubscribeRequest`
  and its builder, `SubscriptionKind` (cyclic, change-of-state, event), `QueuePolicy`
  (backpressure, latest-value coalescing, drop-oldest), `LossCounters`, `Subscription`
  (mode, sampling interval, loss, `recv`, `unsubscribe`) and `SubscriptionSender`. The
  queue holds at most `ResourceLimits::max_queued_events` events, which is now read.
  `Connection` gains a provided `subscribe` that returns `Unsupported(Subscribe)` with no
  I/O; no driver overrides it yet. The contract suite gains the case "an unsupported
  subscribe performs no I/O and changes no state" (27 cases).
- `SensorProfile::validate_for` checks status bits (findings G02 and G03): a bit beyond the width of
  its status address's type is `ProfileError::StatusBitOutOfRange` (bit 64 or more already in
  `validate`), two spellings of one address and bit are `OverlappingStatusBits`, and a status bit
  inside the measurement's own leading `bit_field` is the new `StatusBitInBitField`; a refused
  status address is now reported there too. Additive: two `#[non_exhaustive]` variants.
- Benchmarks (no crate API change): Criterion benches for Modbus parsing, PDU and MBAP codec,
  `TypedDecoder`, `Batcher::plan` and batched reads over a loopback socket (`mise run
  bench`), a recorded local baseline in `docs/evidence/benchmarks/`, and a weekly
  scheduled-CI gate that fails on a regression of more than 10% beyond the noise band
  (`scripts/bench-compare.sh`); `criterion` is a dev-dependency only.
- CI (no crate changes): the `semver` job is blocking against the published 0.2.0, with
  cargo-semver-checks pinned to 0.51.0.
- `SensorProfile::validate` rejects a measurement range with a NaN or infinite bound or
  `min > max` (new `ProfileError::InvalidRange`); before, such a range left values
  Uncertain or Good by accident.
- Changed: the word-order rule of sensor profiles is keyed on what the driver declares, not
  on Modbus type names (finding G05). `SensorProfile::validate` no longer reports
  `MissingByteOrder` or `MissingWordOrder`; the new `SensorProfile::validate_for(&dyn
  Driver)` runs every `validate` rule, then parses each measurement's address and reports
  those errors (and `AddressRejected`) when `ProtocolAddress::spans_multiple_words` is true
  for the parsed address and the definition sets no such order. `SensorProfile::tags` runs
  `validate_for`, so a Modbus profile without orders is still refused there. Additive API:
  the provided `ProtocolAddress::spans_multiple_words` (false by default, true for Modbus
  addresses wider than one register) and `validate_for`; the testkit's `FixtureAddress`
  and `ConventionDriver` can declare wide addresses.
- Modbus write PDUs as upstream emits them (PDU codec only; `Capabilities.write` stays
  false and no write goes on the wire): `FunctionCode` gains `WriteSingleRegister`,
  `WriteMultipleCoils` and `WriteMultipleRegisters`, `ModbusPdu` gains their requests and
  responses and `ModbusPdu::write_request`, which picks function 15, 06 or 16 for a payload.
  Behavior change: function codes 06, 15 and 16 now decode (`FunctionCode::from_u8` returns the
  named variant, `decode_request` and `decode_response` accept them) instead of being refused
  as `Unknown`; the enums are `non_exhaustive`, so this is not a semver break. FC05 is still
  not decoded.
- `ModbusConnection` overrides `Connection::ping`: one read of `ping-address` through the
  session and in-flight slot of a read, bounded by `timeouts.request`; any reply, an
  exception included, is success as upstream's (`MB-QUIRK-D`); `Capabilities.ping` is true
  for `modbus-tcp`. The contract suite gains `Requires::Ping` and two cases (26).
- Modbus TCP writes on the wire (slice S46): `ModbusConnection::write` sends one request per
  item (function 15 for a coil, 06 for a one-register holding payload, 16 otherwise), one
  after another, through the session and the single in-flight slot of a read; a coil takes a
  `BOOL`, a register the type of its address in the byte and word order of the tag, the
  address block or the connection default. An echo is `Confirmed`, a device exception is
  `Rejected` with its code as the protocol status, a request that went out and got no usable
  answer (deadline, lost link, cancellation, a reply that does not echo it) is
  `WriteOutcome::Unknown` and is never retried or replayed, and the items after an unknown one
  are not sent. A discrete input or an input register is `Rejected` without a request.
  `Capabilities.write` is true for `modbus-tcp`. New connection-string parameter
  `access=read-only|read-write` (default read-only): a connection whose access policy is
  read-only refuses every write before anything else, so writes must be granted. The
  contract suite's write cases now run for `modbus-tcp` over the fake transport and loopback
  sockets (and the read-only case against pymodbus); the upstream write
  exchanges of the three retained driver-suite files are vectors (60 more) and are replayed
  byte for byte. Additive API; `ModbusSession::request` also carries writes, and its span
  says `op = "write"` for one.
- `stonecrab::sensors` now denies `clippy::indexing_slicing` (checked by `lint-attrs`), like
  the other modules that parse untrusted input; no code in the module needed to change.
- Release process and evidence (no crate changes): ADR-0017, `release.toml`,
  `mutants.toml`, the `release-evidence` workflow, `docs/evidence/` with the recorded
  fuzz, mutation and coverage figures, and drafted (not filed) upstream issues.
- Documents reconciled with the 0.2.0 publication (no crate changes): README, SECURITY,
  STATUS, the project brief and the coding standards no longer call 0.2.0 unpublished or
  use the old repository owner.
- `Operation::Ping`, `Capabilities.ping` and a provided `Connection::ping` that returns
  `Unsupported` until a driver overrides it (additive; no driver overrides it yet).
- Upstream Modbus test vectors (no crate changes): four Apache PLC4X test files are
  retained verbatim under `fixtures/modbus/upstream/` with NOTICE attribution, and the six
  frames of the parser suite are vectors, none of them in the published crate.
- MSpec code generator decision (no crate changes): ADR-0019 chooses a native Rust
  maintainer tool, `tools/stonecrab-codegen`, proven first on Modbus against the
  hand-written codec through the vector corpus, with a provenance header and an offline
  `regen` gate; nothing is generated yet.
- Upstream driver-suite vectors (no crate changes): a test-only adapter
  (`tests/modbus_driver_suite_adapter.rs`, the `roxmltree` dev-dependency) serializes the
  XML models of the retained PLC4X driver suites with its own mspec serializer and emits 64
  vectors, the request and response of 32 read exchanges, under `fixtures/modbus/tcp/`
  (none of them in the published crate). `TypedDecoder` decodes every one to the value
  the XML reports, so `MB-BYTEORDER-BE` and `MB-BYTEORDER-LE` are `automated-verified`; the
  two byte-swap modes are in no upstream file and stay `implemented`.

## Alpha
The first pre-release is `stonecrab` 0.2.0, one crate (ADR-0018). Its section below opens
with the four release-note items that `docs/SPEC.md` ('Open-source project requirements')
requires, once for the crate: the upstream baseline, the supported operations, the actual
hardware evidence and the breaking changes. The support levels are those of the 'Hardware
evidence' policy of `docs/SPEC.md`; the full matrix is `docs/COMPATIBILITY.md`.

Gates not run for the alpha, so that nobody reads their absence as a pass: branch coverage
(line coverage is gated per module, region coverage is reported), a mutation-testing
threshold, the 8-hour release fuzz run (CI fuzzes each target for 60 seconds), the 24-hour
soak, benchmarks, differential tests against PLC4J, hardware verification, and `cargo
semver-checks` against the specification-stage 0.1.0 package (its API was replaced). The
measured substitutes are recorded by the release-evidence slice S32 and re-recorded for
the one crate by S33 (`docs/evidence/alpha.md`).

### stonecrab 0.2.0 (2026-10-27)
- **Upstream baseline:** Apache PLC4X v1.0.0 (commit
  `ea229918f5d07700e04d7d69ab1645da679cb700`, pinned in `upstream/manifest.toml`; the
  inventory covers the Modbus TCP subset only and the source-release signature was
  verified in S32, see `docs/evidence/alpha.md`). The connection-string grammar of the core
  and the Modbus TCP driver follow it; divergences are rows in `docs/parity/modbus.md` and
  are listed in `docs/COMPATIBILITY.md`. The simulator, the transport, the codec and the
  sensor-profile format are Stonecrab's own and make no upstream claim (Apache PLC4X ships
  a simulated driver whose source is not inventoried, builds its transports on Netty and has
  no sensor profile concept). Claims are per parity row in `docs/parity/`.
- **Supported operations:** the core defines the driver boundary (`DriverRegistry`,
  `Driver`, `Connection`: connect, read, write, close, state watching) and the data
  contracts, and implements no driver; it is `simulator-tested` through the common driver
  contract suite. `stonecrab::sim` (feature `sim`): connect, close, read (`STATE` and
  `RANDOM`), write (`STATE` and `SINK`), scripted failures and bounded read retry,
  `simulator-tested`. `stonecrab::modbus` (feature `modbus`): Modbus TCP connect, close and
  reads of coils, discrete inputs, holding registers and input registers (function codes
  01 to 04) as the 15 numeric and bit types, with batching and `split_into` disclosure,
  request-driven reconnect, retry, cancellation and timeouts; reads are
  `interoperability-tested` against pymodbus 3.8.6 and the fault handling is
  `simulator-tested`. Writes (the driver reports `write: false`), subscriptions, browse,
  discovery, ping, extended registers, strings, calendar types, RTU, ASCII, UDP and TLS are
  not supported. `stonecrab::transport` (feature `transport`): the deadline-bounded TCP
  transport, framing with resynchronization and the connection guard; the connect and
  request path is `interoperability-tested` through the Modbus driver, resync, reconnect
  and the queue are `simulator-tested`; serial, UDP and TLS are not supported.
  `stonecrab::codec`: bounds-checked byte primitives, `build-tested`.
  `stonecrab::sensors` (feature `sensors`): loading and validating version-1 TOML
  profiles, building tags through a driver and turning a read response into scaled, graded
  measurements; the synthetic profiles are `simulator-tested` against the simulator and
  `interoperability-tested` against pymodbus 3.8.6, and no named device profile exists.
  `stonecrab::testkit` (feature `testkit`) is test tooling and has no support level.
  Subscribe, browse and discover are not defined or supported by any module. The matrix is
  `docs/COMPATIBILITY.md`.
- **Hardware evidence:** none. No physical device has been tried and no hardware record
  exists; nothing is `hardware-verified`. The simulator models no wire protocol.
- **Breaking changes:** versus the published 0.1.0 specification-stage package, the
  `placeholder::status` API is removed with no replacement (migration: delete the call;
  consult `docs/COMPATIBILITY.md` for what is supported). Everything else is new: the
  0.1.0 package exposed nothing else, and the sibling crates of the earlier seven-crate
  layout were never published. Behavior that differs from upstream by design: bare `40001`
  addresses need an explicit convention, unknown connection parameters are errors, replies
  are validated on transaction id, unit id and function code.

Changes. The first bullet is the single-crate build of slice S33; the bullets after it
are grouped by module and keep the wording they had when each landed, so the crate names
in them (`stonecrab-core`, `stonecrab-modbus` and so on) are the former crates, now the
modules of the group heading (ADR-0018).

- **One crate (S33, ADR-0018):** `stonecrab` now contains the former sibling crates as
  modules (`stonecrab::codec`, `transport`, `modbus`, `sim` and `sensors`, with the core
  contracts at the crate root) and the test kit as `stonecrab::testkit` behind the
  off-by-default `testkit` feature (a separate test crate would be a dev-dependency
  cycle). `cargo add stonecrab` gives Modbus TCP, the simulator and the sensor profiles
  with no feature flags (default features `sim`, `modbus`, `sensors`);
  `default-features = false` is the tokio-free core; `full` stays as an alias. The `tcp`
  and `toml` features are gone (the TCP transport is part of `transport`, the TOML loader
  of `sensors`). There is one version, one changelog, one packaging list, one SBOM and one
  release tag, `v0.2.0`. Every test, doctest, gate, example, fuzz target and CI job is
  kept; lint promotions and coverage thresholds apply per module; test files declare
  `required-features` so a plain `cargo test` skips what it cannot build.

#### Core (crate root)
- Move the facade into a resolver-3 workspace with a runtime-independent core crate.
- Remove the specification-only `placeholder::status` API. There is no replacement
  status API; consult the capability and parity documentation.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) and stop shipping `tests/` in
  the crate archive.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) for feature-gated item docs.
- Establish shared metadata, safety lints and foundation contracts.
- Add exact-width scalar values, validated tag identifiers, monotonic deadlines,
  immutable finite resource budgets, explicit capabilities, typed diagnostic
  failures and redacted secret formatting.
- Add protocol-owned tags, bounded read builders, exact batch correlation,
  preserved per-item protocol status and explicit unknown-after-transmit outcomes.
- Add `ByteOrder` and `WordOrder`, optional `Tag::with_byte_order`/`with_word_order` with
  `byte_order()`/`word_order()` accessors, and additive `PartialOrd, Ord` on `ErrorKind`,
  `Operation` and `PlcValueType` (ADR-0006).
- Add `OpFuture`, the `Clock` trait (no implementation in core), `CancellationToken` with
  child tokens and `StateCell`/`StateWatch` latest-value connection-state observation,
  all on `std` only (ADR-0007).
- Add `ConnectionConfig` (not `Clone`) with its validating builder, `Endpoint`, `Timeouts`,
  `ReconnectPolicy` with pure `next_delay`, `RetryPolicy`, `AccessPolicy` (default
  `ReadOnly`), `Credentials`, and the `AddressOptions` carrier with its builder
  (ADR-0008). The simulator connection enforces the timeouts, retry and access policies.
- Add write batch contracts: `WriteItem`, `WriteRequest` with its bounded builder (item
  budget, duplicate identities, value type versus tag hint, deadline) and `WriteResult`,
  `WriteResponse` with its correlating builder, `iter_confirmed`, `iter_rejected` and
  `iter_unknown`. Unknown outcomes are first-class and never retried or replayed
  (ADR-0010). Data only.
- Add `ConnectionUri::parse` for `scheme(:transport)?://authority(?k=v(&k=v)*)?` with
  ordered `options()`; it rejects percent-encoding, credentials in the authority, empty
  or duplicate keys, whitespace and input over 4096 bytes, and `Display` renders the
  canonical text. Covered by the `fuzz_core_connection_string` target.
- Add the `Driver` and `Connection` traits (object safe, `Send + Sync`; the narrow D2
  surface) and `DriverRegistry` (`register` takes `&mut self`, duplicate scheme
  `InvalidConfig`, unknown scheme `Unsupported(Operation::Connect)`, `parse` and `connect`
  through `ConnectionUri`), and the default-deadline rule they document (ADR-0011).
- Add `ReadRequestBuilder::cancel`, `ReadRequest::cancel`, `WriteRequestBuilder::cancel`
  and `WriteRequest::cancel`: an optional `CancellationToken` carried by a request.
- Accept ADR-0007: the simulator connection enforces the token and the default deadline.
- Add `FakeClock`, `block_on`, `block_on_with`, `poll_once` and a counting waker to the
  unpublished testkit for deterministic, sleep-free async tests.
- Add `DriverOptions`, `ConnectionConfigBuilder::driver_options`,
  `ConnectionConfig::driver_options` and `driver_options_as`: a slot for options only one
  driver understands (ADR-0008 amendment). `Debug` and `redacted_display` print them, so
  their `Debug` must hold no secrets.
- Re-export `ByteOrder` and `WordOrder` from `stonecrab-core`.
- Re-export `OpFuture`, `Clock`, `CancellationToken`, `StateCell` and `StateWatch` from
  `stonecrab-core`.
- Re-export `ConnectionConfig`, `ConnectionConfigBuilder`, `Endpoint`, `Timeouts`,
  `ReconnectPolicy`, `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions` and
  `AddressOptionsBuilder` from `stonecrab-core`.
- Re-export `WriteItem`, `WriteRequest`, `WriteRequestBuilder`, `WriteResult`,
  `WriteResponse` and `WriteResponseBuilder` from `stonecrab-core`.
- Re-export `ConnectionUri` from `stonecrab-core`.
- Re-export `Driver`, `Connection` and `DriverRegistry` from `stonecrab-core`.
- Re-export `DriverOptions` from `stonecrab-core`.

#### Codec (`stonecrab::codec`)
- Add the crate: `BoundedReader` and `BoundedWriter` with bounds-checked integer, float,
  byte-run and least-significant-bit-first coil reads and writes in an explicit
  `ByteOrder`, and `ParseError` (`Truncated`, `OversizedLength`, `InvalidDiscriminator`,
  `TrailingBytes`, `Overflow`) with `From<ParseError> for PlcError`.
- Add `coil_byte_count`, `pack_bits_lsb_first` and `unpack_bits_lsb_first`.
- Lengths are checked against the caller's maximum and the remaining input before any
  allocation or copy; no function panics on any input (ADR-0013).

#### Transport (`stonecrab::transport`)
- Add the crate: the object-safe `Transport` trait (`send`, `recv`, `shutdown`, `peer`,
  `limits`; every call takes a `Deadline`) and `TransportFactory` (both return
  `OpFuture`), and `FrameError` (`Truncated`, `Oversized`, `Malformed`) with
  `From<FrameError> for PlcError` (ADR-0014).
- Add `TcpTransport` and `TcpFactory { nodelay }` behind the default `tcp` feature: name
  resolution and connection inside one connect deadline, refused and reset connections as
  `Disconnected`, `recv` bounded by `max_buffered_bytes`, `send` bounded by
  `max_frame_bytes`, and a transport that poisons itself when a `send` is cut short.
- Add `SystemClock`, the Tokio-backed `Clock` (`now()` is Tokio's instant so paused-time
  tests agree with `Deadline`), and `Jitter`, a seeded xorshift64* source for
  `ReconnectPolicy::next_delay`.
- Adopt `tokio` with scoped features (`rt`, `time`, `sync`, `io-util`, and `net` through
  `tcp`); `deny.toml` bans its `full` feature. No task is spawned and nothing runs between
  calls.
- Add the `Framer<F>` trait (`decode`, `encode`, `max_frame_bytes`) and
  `FramedTransport<F, R>` over a `Box<dyn Transport>`: `next_frame` assembles fragmented
  and coalesced frames within `max_buffered_bytes`, `send_frame` encodes and sends one
  bounded frame, and `resync(Deadline)` discards bytes until the framer finds a frame
  boundary and fails with `Disconnected` at the deadline (ADR-0014 addendum).
- Add `ConnectionGuard<F, R>`, `GuardConfig` and `GuardStats`: the connection lifecycle
  machine over a `TransportFactory` and a `Framer` (`connect`, `request`,
  `request_with_default_deadline`, `next_reply`, `close`, `state`, `watch_state`,
  `stats`), one request in flight with a bounded FIFO queue (`Backpressure` beyond `max_pending_requests`),
  request-driven reconnect with jittered backoff and `Failed(Disconnected)` when the retry
  budget is spent, resync before the next request after a timeout, `Diagnostics` on every
  error, and a `close` bounded by `Timeouts.request`. No background task (ADR-0014
  addendum).

#### Simulator (`stonecrab::sim`)
- Add the crate: the in-process simulator driver, depending on `stonecrab-core` and
  `tracing` only, Tokio-free, default features empty (ADR-0012, accepted).
- Add `RealtimeClock`, a `std`-only `Clock` whose `now()` is `Instant::now()` and whose
  `sleep_until` resolves immediately without moving time.
- Add `SimAddress` and `SimArea`: the grammar `STATE|RANDOM|SINK/<name>:<type>` with
  lowercase scalar type names, `ProtocolAddress` with protocol `sim`, 256-byte bound,
  `STDOUT` renamed `SINK`, the legacy array form rejected.
- Add `SimDevice` and `SimTag`: named tags, `ramp` with checked arithmetic that holds at
  the last value on overflow, a read counter, a poison-recovering shared device, stored
  latency and item limit, and a seeded xorshift `RANDOM` source (`SimDevice::with_seed`).
- Adopt `tracing` 0.1 (`std` only) workspace-wide; `deny.toml` lets only `tracing-core`
  depend on the banned `once_cell`.
- Add `SimDriver` (scheme `sim`; `sim://<device>?latency-ms=<n>&seed=<u64>`; `new`,
  `with_clock`, `with_device`, `device`) and `SimConnection`, the `Connection` it opens: the
  lifecycle `Disconnected`, `Connecting`, `Connected`, `Closing`, `Closed`; reads of
  `STATE` and `RANDOM` with per-item errors, writes to `STATE` and `SINK`; one operation in
  flight (a second fails with `Backpressure`); the default-deadline rule, cancellation and
  deadlines on the injected clock; whole-request read retry under `RetryPolicy`;
  `Diagnostics` on every error; `connect`, `read`, `write` and `close` spans. The README
  support table now marks read and write `simulator-tested`, and a test keeps it equal to
  `SimDriver::capabilities()`.
- Add `SimDevice::write`, `record_sink_write`, `sink_writes` and `reseed`.
- Add the failure script: `FailureStep` (`Delay`, `Timeout`, `Disconnect`,
  `DeviceException`, `Malformed`, `PartialFailure`, `Backpressure`, `UnknownAfterTransmit`),
  `FailureScript` (a bounded queue) and `SimDevice::push_failure`, `push_failures`,
  `pending_failures` and `clear_failures`. A connect, read or write takes at most one step;
  a scripted disconnect leaves the connection `Failed(Disconnected)`; a read without a
  deadline now gets `timeouts.request` afresh for each retry attempt.
- Add the `sim` feature (off by default): `stonecrab::sim` re-exports the public items of
  `stonecrab-driver-sim`, pinned `=0.1.0`, and the `sim_read` example reads three tags
  from a simulated device through the driver registry.

#### Modbus (`stonecrab::modbus`)
- Add the native Modbus TCP driver crate, depending only on `stonecrab-core`, with default
  features empty. It starts with the address types and their text grammar; the codec,
  framing, session, connection and driver follow in the entries below.
- Add `UnitId` (one MBAP byte; default 1; 0 and 255 are carried, not rejected),
  `ModbusArea` (`Coil`, `DiscreteInput`, `HoldingRegister`, `InputRegister`),
  `ModbusDataType` (the 15 numeric and bit types, with `from_upstream_name` recognising
  all 27 upstream names and refusing the 12 deferred ones by name),
  `AddressConvention::Plc4xCompatible` and `ModbusAddress` built through a validating
  `ModbusAddressBuilder`.
- `ModbusAddress` implements `ProtocolAddress` with protocol `modbus` and a canonical
  long form such as `holding-register:16:INT{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`
  (ADR-0015, proposed).
- Add `ModbusAddress::parse(text, &AddressOptions)`, the PLC4X address grammar: long
  and `0x`/`1x`/`3x`/`4x` short forms always, bare `40001`-style forms only with
  `address-convention=plc4x`, an optional `{unit-id: N, byte-order: 'NAME'}` block, and
  typed `InvalidAddress` rejections for everything else; `ModbusAddress::MAX_TEXT_BYTES`
  and the fuzz target `fuzz_modbus_address`.
- Add `Batcher::plan`, `BatchLimits`, `Chunk` and `Member`: tags are grouped by unit id
  and area, sorted by wire offset and merged into reads only when adjacent or overlapping
  (no gap reads), never above 125 registers or 2000 bits per chunk or `max_items` tags
  per plan.
- Add the PDU codec (ADR-0015): `FunctionCode`, `ExceptionCode` (the ten V1.1b section 7
  codes and `Unknown`) and `ModbusPdu` for the read functions 01 to 04, their normal
  responses and the exception response, with `encode_request`, `decode_request`,
  `decode_response`, `encode_response`, `ModbusPdu::check_reply_to` and the limits
  `MAX_PDU_BYTES`, `MAX_READ_BITS` and `MAX_READ_REGISTERS`. Quantities (1 to 2000 bits,
  1 to 125 registers), byte counts and the 253-byte PDU bound are checked before any
  allocation; write PDUs and exception codes outside the ten are refused. The crate now
  depends on `stonecrab-codec`. Golden vectors under `fixtures/modbus/tcp/` are written
  from the V1.1b examples or built to fail; the fuzz target is `fuzz_modbus_pdu`.
- Add MBAP framing (ADR-0015): `MbapHeader`, `TransactionId` (1 to 0xFFFE, wrapping as
  upstream, never 0 or 0xFFFF), `ModbusAdu` (a header and an opaque PDU of 1 to 253
  bytes) and `MbapFramer`, a `stonecrab_transport::Framer`. The header is judged before
  the body is awaited or allocated: protocol id other than 0 or length 0 or 1 is
  `Malformed`, length above 254 (ADU above 260 bytes) is `Oversized`, an incomplete ADU
  waits. The crate now depends on `stonecrab-transport` (without its `tcp` feature) and
  `bytes`. Golden ADU vectors are `fixtures/modbus/tcp/adu_*`; the fuzz target is
  `fuzz_modbus_mbap`.
- Add `TypedDecoder`: `decode_registers`, `decode_bits` and `encode_registers` map the 15
  numeric and bit types to `PlcValue`s in the four upstream byte-order modes (a `ByteOrder`
  and a `WordOrder`; `0x01020304` is `[1,2,3,4]`, `[4,3,2,1]`, `[2,1,4,3]`, `[3,4,1,2]`),
  with exact integers, bit-for-bit floats and the single-register padding of `BOOL`,
  `BYTE`, `SINT` and `USINT`. A wrong register count is a `MalformedData` item error that
  keeps the received bytes within the caller's budget. The little-endian modes are derived
  from the documented layouts; they are not `automated-verified` until the upstream
  little-endian vectors are added. Vectors are under `fixtures/modbus/tcp/typed_*`.
- Add `ModbusSession` (ADR-0015): one read request in flight over the transport crate's
  `ConnectionGuard`, a transaction counter that continues across reconnects, and
  `request(pdu, unit_id, deadline, token)` returning the reply PDU. A reply is accepted
  only when its transaction id, unit id and function code all match (upstream compares the
  transaction id only; divergence `MB-READ-MATCH`, V1.1b section 7); anything else is
  discarded, counted in `SessionStats` as `late_replies` (the id of a request the session
  gave up on) or `mismatched_replies`, and logged at `warn` with the expected and received
  transaction ids. A device exception is `Ok(ModbusPdu::Exception)` with the connection
  still `Connected`; `exception_item_error` maps it per decision D18 and keeps the raw code
  and name. A reply PDU that does not decode or fit the request is `MalformedData`. Each
  request has a `debug` span with `scheme`, `endpoint`, `op` and `txn`. The crate now
  depends on `tracing`.
- Add `ModbusConnection` and `ConnectionDefaults` (ADR-0015): the `Connection` of the
  `modbus-tcp` scheme over a `ModbusSession`, opened with `ModbusConnection::open(config,
  factory, clock, defaults)`. A read is planned by `Batcher::plan` and runs chunk by chunk
  (function codes 01 to 04, at most 125 registers or 2000 bits), each chunk under
  `min(request deadline, now + timeouts.request)`; `ReadResponse::split_into` discloses the
  chunk count, `sampled_at` is the instant each chunk was answered and `source_time` is
  `None`. A device exception fails only its chunk's members; a failed exchange fails the
  read, after retries under the configuration's `RetryPolicy` for retryable kinds only.
  One read is in flight at a time (`Backpressure` before any I/O). `write` is
  `Unsupported(Operation::Write)` with no I/O. `ConnectionDefaults` carries the default unit
  and the byte and word order that a tag or address without its own falls back to.
- Add `ModbusTcpDriver` (ADR-0015, decision 14): the `Driver` of the `modbus-tcp` scheme,
  with `new()` (real TCP transport and `SystemClock`), `with_clock`,
  `with_transport_factory` (the way to run the testkit `FakeTransportFactory`) and
  `with_defaults`. `parse_config` reads `modbus-tcp[:tcp]://host[:port]` (port 502 by
  default, IPv6 in brackets) and accepts no parameter yet; the six upstream parameters and
  `address-convention` follow with the configuration slice. `parse_address` delegates to
  `ModbusAddress::parse` with the convention of the options, and `capabilities` is the
  value every `ModbusConnection` reports. The crate now enables the `tcp` feature of
  `stonecrab-transport`. The common driver contract suite runs green against the driver over
  the fake transport (`tests/contract.rs`, with a first-party MBAP responder over the
  interop register map); only the two write cases are skipped, by name. The unpublished
  testkit gains `Step::React` and `Reaction` (reply, stay silent, close, reply then close),
  so the responder can drop the link on demand and the two disconnect cases pass.
- Complete the driver configuration: `parse_config` now accepts the six upstream
  parameters with the upstream defaults (`request-timeout-ms` 5000,
  `default-unit-identifier` 1, `ping-address` `4x00001:BOOL` validated and recorded but
  unused, `default-payload-byte-order` `BIG_ENDIAN`, `max-coils-per-request` 2000,
  `max-registers-per-request` 125) and `address-convention=plc4x`; every other key, the
  misspelt `unit-identifier` included, is `InvalidConfig`, where upstream warns. The parsed
  values are `ModbusOptions`, carried in the configuration's driver-options slot and read
  back by `connect`. `ConnectionDefaults` gains `max_coils` and `max_registers`, and `open`
  now refuses values above the protocol maxima. `with_defaults` is now the fallback for a
  configuration that carries no options. The crate re-exports `SystemClock`. The fuzz target is
  `fuzz_modbus_connection_string`.
- A holding-register read runs over a real loopback socket against a scripted peer
  (`tests/scripted_read.rs`), and the driver contract suite runs over loopback sockets
  (`tests/contract_scripted_peer.rs`: 18 passed, 5 skipped by name). The README documents the
  connection-string grammar, the parameter table, the byte-order names and the support levels,
  and a test keeps its table equal to `ModbusTcpDriver::capabilities()`.
- Add the `interop` CI job and `mise run interop`: the driver reads the interop register map
  from a first-party pymodbus 3.8.6 server in Docker (`interop/pymodbus/`), ten ignored
  tests in `tests/interop_pymodbus.rs` and the `modbus_tcp_read` example, with the image id,
  `pip freeze`, server log and contract report uploaded as `interop-evidence`; ADR-0015 is
  accepted (decision 16).
- Add the `modbus` feature (off by default): `stonecrab::modbus` re-exports the public items
  of `stonecrab-modbus`, pinned `=0.1.0`, including `ModbusTcpDriver`, `ModbusOptions` and
  `SystemClock`, and the `modbus_tcp_read` example reads one tag from a Modbus TCP server
  through the driver registry. Each example now declares `required-features`.

#### Sensors (`stonecrab::sensors`)
- Add the crate: declarative sensor profiles for Stonecrab, depending on `stonecrab-core`,
  `serde` and, behind the default `toml` feature, `toml` (ADR-0016). No driver reads a
  profile yet; that arrives in a later slice.
- Add the schema (version 1): `SensorProfile`, `ProfileId`, `SchemaVersion`,
  `DeviceApplicability`, `ConnectionRequirements`, `MeasurementDef`, `ByteOrderName`,
  `WordOrderName`, `ScaleStep` (`Multiply`, `Divide`, `Offset`, `Clamp`, `BitField`),
  `ValueRange`, `StatusField`, `QualityMapping`, `Provenance` and `ValidationStatus`. All
  derive `Deserialize` with unknown keys denied; none derives `Serialize`.
- Add `SensorProfile::from_toml` (feature `toml`): 64 KiB size bound checked before
  parsing, schema versions other than 1 refused as `ProfileError::IncompatibleSchema`, and
  errors that carry a line and column but never part of the document. Add `ProfileError`
  (`Oversized`, `Syntax`, `IncompatibleSchema`, `InvalidIdentifier`) with
  `From<ProfileError> for PlcError` (`InvalidConfig`).
- Add the fuzz target `fuzz_sensors_profile_toml`. Adopt `serde` and `toml` (`display`
  off); the lock pins `toml_parser` 1.0.9 to keep a single `winnow`.
- Add `SensorProfile::validate`, which returns every semantic error of a profile at once
  (duplicate names, unknown or empty units, missing byte or word order for 32- and 64-bit
  Modbus types, overlapping status bits, `count != 1`, unevaluable scaling steps, unknown
  quality references, address convention rules), and the fixed unit table `Unit`. Add eleven
  `ProfileError` variants for them; the enum was already `#[non_exhaustive]`.
- Add the measurement pipeline: `SensorProfile::tags(&dyn Driver)` builds one tag per
  measurement and one `<measurement>.<status name>` tag per status field, parsing every
  address under the profile's address convention (`SensorProfile::address_options`), and
  refusing a scheme mismatch; `SensorProfile::measurements(&ReadResponse, SystemTime)`
  scales values in declared order with checked arithmetic (integers stay integers without
  a float step, NaN, infinity and overflow are `Bad`, `BitField` handles counter rollover)
  and grades them with `Quality` (`Good`, `Uncertain`, `Bad`, `Stale`, `Unknown`) from the
  range, the item error and the quality mapping. Add `Measurement`, `Quality`,
  `MeasurementDef::status_tag_id` and four `ProfileError` variants (`SchemeMismatch`,
  `AddressRejected`, `StatusAddressRejected`, `DuplicateStatusFieldName`). The simulator
  and the testkit are dev-dependencies for the tests; the unpublished testkit gains the
  `ConventionDriver` test double of `Driver`.
- Add the synthetic reference profiles under `profiles/synthetic/` at the repository root:
  temperature, pressure/flow/level and discrete state, each as a `modbus-tcp` file authored
  from the pymodbus register map and a `sim` twin, all labeled `validation = "synthetic"`.
  They are repository data and are not packaged with the crate. The crate's code is
  unchanged; a new test loads and validates every file.
- Add the `sensors` feature (off by default): `stonecrab::sensors` re-exports the public
  items of `stonecrab-sensors`, pinned `=0.1.0`, with its TOML loader enabled, and the
  `full` feature (`sim`, `modbus`, `sensors`). The `profile_read` example (requires `full`)
  loads a synthetic profile from `profiles/`, reads it from `sim://bench` or a Modbus TCP
  server and reports name, value, unit, quality, origin and raw value through `tracing`;
  `--expect-good` makes it fail on any measurement that is not of quality `Good`.

#### Testkit (`stonecrab::testkit`)
- Add an unpublished deterministic address fixture testkit for contract tests.
- Add the unpublished testkit's `fixtures` feature: the golden-vector loader (`load_dir`,
  `load_hex`, `GoldenVector`, `VectorSource`, `Provenance`, `GoldenError`) that enforces
  "every vector is listed in `PROVENANCE.toml`", and `InteropMap`, the reader of the
  first-party `fixtures/modbus/interop/register-map.toml`. The upstream vectors themselves
  wait for maintainer prerequisite P4.
- Add the unpublished testkit's driver contract suite (`ContractSuite`, `ContractFixture`,
  `ContractReport`): 23 named cases that every driver runs, green against `sim`.
- Add the unpublished testkit's `ScriptedPeerFactory` (`net` feature): a `TransportFactory`
  that starts a fresh `ScriptedPeer` per `open` and connects with the real `TcpFactory`.
- Test the transport against the unpublished testkit's new `net` feature (`Script`,
  `ScriptedPeer`, `FakeTransport`, `FakeTransportFactory`); no change to the public API.

## stonecrab 0.1.0
- Publish the specification-stage package. No industrial protocol support.
