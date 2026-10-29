# Stonecrab Testkit
Deterministic test fixtures, `stonecrab::testkit`, behind the `testkit` feature (off by
default; it is not a runtime feature, and an application that only reads from a device
never turns it on). It ships the driver contract suite so that authors of a new
`Driver` can run the same cases against their driver, and the fakes (a clock, a
transport, scripted peers) that make such tests deterministic. It has no support level
of its own; it is the tooling behind the evidence claimed for the other modules
(coverage tier 90%, ADR-0005).

The kit lives in this crate, and not in a separate test crate, because a test crate
needs the `Driver` contract from `stonecrab`, and `stonecrab` needs the kit in its own
tests: that would be a dev-dependency cycle (ADR-0018). The feature turns on `transport`
and adds `serde`, `serde_json`, `toml` and Tokio's `macros`. The kit's own tests and
doctests for the golden vectors read files of the repository (`testdata/`, `fixtures/`),
which the packaged archive does not contain, so they run from a checkout.

`FixtureAddress` supplies a protocol-address double for tests of the core contracts.
It retains supplied text and an optional type hint; it does not parse or validate
a real industrial protocol or claim driver compatibility.

```rust
use std::sync::Arc;
use std::time::Instant;
use stonecrab::{PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Tag, TagId};
use stonecrab::testkit::FixtureAddress;

let id = TagId::new("counter")?;
let address = Arc::new(FixtureAddress::new("counter", None));
let request = ReadRequest::builder(ResourceLimits::builder().build()?)
    .tag(Tag::new(id.clone(), address))
    .build()?;
let now = Instant::now();
let response = ReadResponse::builder(&request, now, now)
    .item(ReadItem::new(id, Ok(PlcValue::U64(42)), now))
    .build()?;
assert_eq!(response.iter_ok().count(), 1);
# Ok::<(), stonecrab::PlcError>(())
```

`FakeClock` implements `stonecrab::Clock` for tests and is advanced by hand, so
nothing waits on real time. `advance`, `set` (never backwards; an earlier instant is
ignored and reported as `false`) and `advance_to_next_sleeper` move it, and each sleep
future from `sleep_until` is woken exactly once, in deadline order. Clones share one
timeline and `Arc<FakeClock>` works as `Arc<dyn Clock>`.

`block_on` drives a future with a no-op waker under a poll budget, `block_on_with` also
advances a `FakeClock` to the next sleeper whenever the future pends, and `poll_once`
(or `poll_once_with` and `CountingWaker`, which counts wake calls) polls exactly once. A
future that can never complete, such as one waiting on a token nobody cancels, panics
with a message instead of hanging the test.

```rust
use std::time::Duration;
use stonecrab::Clock;
use stonecrab::testkit::{FakeClock, block_on_with, poll_once};

let clock = FakeClock::default();
let deadline = clock.now() + Duration::from_secs(30);

let mut sleep = clock.sleep_until(deadline);
assert!(poll_once(&mut sleep).is_pending());
drop(sleep);

block_on_with(&clock, clock.sleep_until(deadline))?;
assert_eq!(clock.now(), deadline);
# Ok::<(), stonecrab::PlcError>(())
```

## Driver contract suite

`ContractSuite::run(driver, uri, fixture)` runs one set of 27 cases, named as sentences,
against any `Driver` and returns a `ContractReport`. A
`ContractFixture` is the suite's window into the device: a `readable` address, a
`rejected` one (parseable, refused by the device), a `foreign` one, an `io_probe` counter
that must not move when the suite says "no I/O", a `device_reads` counter, the `clock`
the driver uses (a `FakeClock`), an optional `writable` address with a value, and the
optional hooks `inject_disconnect`, `inject_timeout`, `inject_device_exception` and
`inject_unknown_after_transmit`, each of which arms one failure for the next operation.

Each case is `Passed`, `Failed(reason)` or `Skipped(reason)`; the suite never panics on a
driver failure, and a case the driver or the fixture cannot exercise is skipped with a
reason (`driver reports read unsupported`, `fixture cannot inject timeout`), never
omitted. `ContractReport` counts the outcomes, `is_green` means no failure and at least
one pass, and `Display` lists every case with its outcome, its reason and the
mandatory case it covers. The retry and read-only cases rebuild the parsed configuration
through `ConnectionConfig::builder`, so the fixture needs no knob for them. The
queued-cancellation case is not part of the suite (it needs a wait queue; see
`ConnectionGuard`). The full run against the simulator is
`crates/stonecrab/tests/sim_contract.rs`; ADR-0009's S13 addendum has the details.

At the alpha the suite runs against four fixtures, and every skip is named in the
report rather than hidden: the simulator (`tests/sim_contract.rs`: 24 passed,
3 named skips, the unsupported-write case and the two ping cases), the Modbus driver
over a fake transport with a first-party MBAP responder (25 passed, 2 skipped: the
unsupported-write and unsupported-ping cases), the Modbus driver over loopback sockets to a
`ScriptedPeer` (21 passed, 6 skipped: three timing cases and the silent-ping case, the
unsupported-write case and the unsupported-ping case) and the Modbus driver against
pymodbus 3.8.6 in the `interop` job (16 passed, 11 skipped: timing, on-demand disconnect
and device exception, and the write cases). The unsupported-subscribe case (S54) offers
one request per `QueuePolicy` and passes for every fixture, because no driver subscribes
yet. The results are in the parity rows `SIM-CONTRACT-SUITE` and `MB-CONTRACT-SUITE`.

```rust,no_run
use stonecrab::Driver;
use stonecrab::testkit::{ContractFixture, ContractSuite, FakeClock, block_on_with};

fn check(driver: &dyn Driver, fixture: &ContractFixture, clock: &FakeClock) {
    let report = block_on_with(clock, ContractSuite::run(driver, "sim://bench", fixture));
    let report = report.expect("the run always resolves");
    assert!(report.is_green(), "{report}");
}
```

## Network doubles

These are the doubles that stand in for a device, all driven by one `Script`
(`steps: Vec<Step>`, `strict: bool`; `Script::new` is lenient, `Script::strict` fails on
the first mismatch).

- `Step` instructions: `Expect` (the next bytes received must equal these), `Reply`,
  `ReplyFragmented` (separate writes), `ReplyCoalescedWithNext` (a marker: the next reply
  shares one write with the reply after it), `Delay`, `CloseAt(n)` (send n bytes of the
  next reply, then close), `Truncate(n)` (send n bytes of the next reply, stay open),
  `Corrupt { offset, bytes }`, `WrongTransactionId` (flip the first two bytes of the next
  reply), `LateReply` (deliver the next reply later), `Respond(closure)` (answer every
  request, so one fixture can serve a whole contract suite) and `React(closure)`
  (`Step::react`; like `Respond`, but the closure returns a `Reaction`). Each variant's
  rustdoc gives the exact rule.
- `Reaction` is what a `React` closure decides per request: `Reply(bytes)`, `Silence` (no
  bytes, the client runs into its deadline), `Close` (close the connection without a
  reply) and `ReplyThenClose(bytes)`. `Step::respond` keeps its meaning (`Some` is a
  reply, `None` is silence) but cannot close; a fixture that must drop the link on demand,
  such as the Modbus contract run, uses `React`. `FakeTransport` and `ScriptedPeer` both
  honour every reaction: a close ends the fake transport exactly as `CloseAt` does (`recv`
  returns the end of stream once and then fails `Disconnected`, `send` fails
  `Disconnected`) and half-closes the loopback socket. `ScriptReport::responses` counts the
  calls of `Respond` and `React` closures alike.
- `ScriptedPeer::spawn(script)` listens on `127.0.0.1:0` and runs the script on the one
  connection it accepts. It is for bytes only: `Delay` and `LateReply` sleep for at most
  `MAX_PEER_DELAY` (200 ms). It owns the crate's single `tokio::spawn`; `close()` joins
  the task and returns a `ScriptReport`, and dropping the peer aborts it.
- `FakeTransport` implements `stonecrab::transport::Transport` over the same script and a
  `FakeClock`. Delays and late replies sleep on the fake clock, so `block_on_with` runs
  them without waiting; a deadline that passes first is a `Timeout`. It logs every frame
  (`FakeTransportProbe::sent`) and honors the same frame and buffer bounds as the TCP
  transport.
- `LengthPrefixedFramer` is the sanctioned test `Framer<Vec<u8>>` for
  `stonecrab::transport::FramedTransport`: a 2-byte big-endian payload length, then the
  payload (`b"hi"` is `00 02 68 69`), with a configurable maximum frame size counted in
  wire bytes, prefix included (clamped to 65,537). Any two bytes are a valid header, so
  it never reports `Malformed`; a header over the maximum is `Oversized` at once. It has
  no sync marker, so `resync` slides over garbage one byte at a time.
- `FakeTransportFactory` implements `TransportFactory` from a queue of scripts, a
  scripted connect timeout (parks until the fake clock passes the deadline, then fails
  with `Timeout` and `Operation::Connect`) or a refusal, and records every `open` call.
- `ScriptedPeerFactory` implements `TransportFactory` by starting a fresh `ScriptedPeer`
  (script built by a closure) for every `open` and connecting to it with the real
  `TcpFactory`, so a driver keeps real sockets across reconnects; `finish` collects the
  peers' reports.
- `CountingFactory` wraps an `Arc<dyn TransportFactory>` and counts `open` calls, `send`
  calls (frames) and the bytes sent through the transports it hands out; `io()` is opens
  plus frames. It is the exact `io_probe` for a contract run over a real socket (the
  pymodbus interop run), where the test cannot count requests on the device side.

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use stonecrab::{Clock, Deadline, Endpoint, ResourceLimits};
use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
use stonecrab::transport::TransportFactory;

let clock = FakeClock::default();
let factory = FakeTransportFactory::new(clock.clone());
factory.push_script(Script::strict(vec![
    Step::Expect(b"ping".to_vec()),
    Step::Delay(Duration::from_secs(3)),
    Step::Reply(b"pong".to_vec()),
]));

let endpoint = Endpoint::Custom("fake://plc".to_owned());
let limits = ResourceLimits::builder().build()?;
let deadline = Deadline::after(clock.now(), Duration::from_secs(10))?;
let mut transport = block_on_with(&clock, factory.open(&endpoint, limits, deadline))?;

block_on_with(&clock, transport.send(Bytes::from_static(b"ping"), deadline))?;
let mut reply = BytesMut::new();
block_on_with(&clock, transport.recv(&mut reply, deadline))?;
assert_eq!(&reply[..], b"pong");
assert_eq!(factory.probes()[0].sent(), vec![Bytes::from_static(b"ping")]);
# Ok(())
# }
```

## Golden vectors and the interop map

The kit also holds the loader for the repository's golden vectors (section 13.6 of the
coding standards) and the reader of the first-party interop register map. Both read files
of a checkout.

- `load_dir(dir)` reads `dir/PROVENANCE.toml`, then, for every `[[vector]]` it lists,
  `<name>.hex` and the optional `<name>.expected.json`, and returns `GoldenVector`
  (`name`, `source`, `bytes`, `expected`, `must_fail`, `note`). It enforces "every vector
  is listed": a `.hex` or `.expected.json` that the provenance file does not name, a
  listed vector without a `.hex` and a missing `[[file]]` are errors, and every
  `GoldenError` names the file. `load_hex(text)` decodes one `.hex` text: one line of hex
  digits, whitespace allowed, `#` comments and blank lines skipped, odd length or a
  non-hex character is an error.
- `VectorSource` is `upstream_fixture`, `protocol_spec`, `captured` or `adversarial`. The
  `expected.json` is authored from the specification or a capture, never by running a
  decoder; the loader only parses it.
- The `adu_upstream-driver-suite-*` and `adu_upstream-all-datatypes-*` vectors are
  `upstream_fixture` vectors produced from the XML `ModbusADU` models of the retained
  PLC4X driver suites by an adapter that is not part of the kit: it is a test of the
  repository (`tests/modbus_driver_suite_adapter.rs`) that reads XML with the
  `roxmltree` dev-dependency and serializes with its own mspec-based serializer, so
  neither the kit nor the published crate carries an XML parser or adapter code. Their
  `.expected.json` carries the tag, the payload byte order and the values the XML case
  reports.
- `PROVENANCE.toml` has two optional arrays of tables, unknown keys rejected.
  `[[vector]]`: `name` (relative path, no extension, `/` separated), `source`, `origin`
  (upstream file and revision, or the specification section), `license` (SPDX),
  `must_fail` (optional, default false), `note` (optional). `[[file]]`: `path`
  (relative; must exist), `origin`, `license`, `note` (optional), for first-party
  fixtures that are not wire vectors.
- `InteropMap::load` reads `fixtures/modbus/interop/register-map.toml`: block `sizes`,
  holding registers, the little-endian `holding_extension`, `coils`, `discrete_inputs`
  and `input_registers`, each labeled with its `MB-INTEROP-*` row id, register words as
  hexadecimal integers. Addresses are logical; the wire address is one less.

```rust
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use std::path::Path;
use stonecrab::testkit::{InteropMap, load_dir};

let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
assert!(!load_dir(&dir)?.is_empty(), "first-party and upstream vectors");
let map = InteropMap::load(&dir.join("interop/register-map.toml"))?;
assert_eq!(map.holding_extension[0].words, [0xDC0F, 0x4940]);
# Ok(())
# }
```

`FixtureAddress::spanning_multiple_words` and `ConventionDriver::with_wide_addresses` build
test addresses that report `ProtocolAddress::spans_multiple_words`, for sensor profile
tests that need a value wider than one word without a Modbus driver.

`FixtureAddress::spanning_multiple_words` and `ConventionDriver::with_wide_addresses` build
test addresses that report `ProtocolAddress::spans_multiple_words`, for sensor profile
tests that need a value wider than one word without a Modbus driver.
