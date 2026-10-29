# Stonecrab
**Apache PLC4X, carcinized.** An independent native Rust port, built for sensor acquisition first.
Copyright 2026 The Mad Botter INC and the Stonecrab contributors. Apache-2.0.

```
            _ _ _ _
   \\     /  (o) (o)  \     //
    \\___|     ____     |___//
         |  \________/  |
         '-.__      __.-'
            _/ \  / \_
           (_)  \/  (_)
```

Stonecrab reads values from industrial equipment, such as sensors and controllers that
speak Modbus TCP, and hands them to your program as plain, checked numbers. It is one
package with one version, `stonecrab`, written in Rust. Today it can read coils, discrete
inputs, holding registers and input registers from a Modbus TCP server, turn the raw
values into scaled sensor measurements that carry a quality grade, using declarative
profiles, write coils and holding registers when a connection is granted write access, and
run the same code against a built-in simulator with no network and no hardware. It has been
checked against two independent Modbus servers (pymodbus 3.8.6 and libmodbus 3.1.11) and
has never touched a physical device. **It is alpha software and is not for production use
yet**: it subscribes only by polling, does not browse or discover, its Modbus RTU driver
has met neither an independent RTU server nor a serial device yet, and its APIs may change
before 1.0.

## Add it
```sh
cargo add stonecrab
```
That one line gives you the simulator, Modbus TCP and the sensor profiles. `stonecrab`
0.2.0, the alpha, is published on crates.io (2026-10-27), so it fetches 0.2.0. The older
0.1.0 is a specification-stage placeholder with no protocol code and stays listed as an
old version. The repository has no `v0.2.0` tag and no GitHub release yet.

Programmers use it through one object-safe boundary (`DriverRegistry`, `Driver`,
`Connection`): register a driver, connect with a connection string such as
`modbus-tcp://192.0.2.10:502` or `sim://bench`, and read.

### Features
Everything is in the one crate; the features only decide what is compiled in.

| Feature | What you get | On by default |
| --- | --- | --- |
| `sim` | `stonecrab::sim`, the in-process simulator | yes |
| `modbus` | `stonecrab::modbus`, the Modbus TCP and Modbus RTU drivers (reads and writes); brings in Tokio and the serial transport | yes |
| `sensors` | `stonecrab::sensors`, declarative sensor profiles and the TOML loader | yes |
| `transport` | `stonecrab::transport`, deadline-bounded transports; the only thing that needs Tokio, and on whenever `modbus` is | no |
| `serial` | the serial transport in `stonecrab::transport` (`SerialFactory`, one worker thread per port); on whenever `modbus` is | no |
| `boards` | `stonecrab::boards`, direct board adapters: the MAX31855 thermocouple converter over an `SpiBus`; needs `sensors`, no Tokio | no |
| `adapters-linux` | `LinuxSpiDev`, which reads a `/dev/spidev` node on a worker thread; brings in `boards` and `transport` | no |
| `testkit` | `stonecrab::testkit`, fakes and the driver contract suite for tests; not for applications | no |

`default-features = false` gives the small core with no Tokio and no network code, for a
constrained target; then pick the features you need. The crate README,
[`crates/stonecrab/README.md`](crates/stonecrab/README.md), has the module map.

## Capability matrix
Support levels use only the labels of the project's hardware-evidence policy
([`docs/SPEC.md`](docs/SPEC.md)): `build-tested`, `simulator-tested`,
`interoperability-tested` and `hardware-verified`, plus `planned` and `not supported`.
`interoperability-tested` means a run against an independent implementation (here
pymodbus 3.8.6 in the `interop` CI job, and libmodbus 3.1.11 in `interop-libmodbus` for
the rows that say so; the `interop-toxiproxy` job adds fault injection against both and
changes no level). Nothing is `hardware-verified`.

| Protocol | Operation | Support level | Evidence |
| --- | --- | --- | --- |
| Modbus TCP | connect, close, connection state | interoperability-tested | `interop` job; parity row `MB-TCP-CONNECT` |
| Modbus TCP | read coils (FC01) as `BOOL` | interoperability-tested | `MB-TCP-READ-COIL`; `interop_reads_coils_1_to_10` |
| Modbus TCP | read discrete inputs (FC02) as `BOOL` | interoperability-tested | `MB-TCP-READ-DI`; `interop_reads_discrete_inputs_1_to_10` |
| Modbus TCP | read holding registers (FC03) as the 15 numeric and bit types | interoperability-tested | `MB-TCP-READ-HR`; `interop_reads_all_fifteen_holding_register_types` |
| Modbus TCP | read input registers (FC04) as the 15 numeric and bit types | interoperability-tested | `MB-TCP-READ-IR`; `interop_reads_input_registers_1_to_7` |
| Modbus TCP | batched reads with split disclosure (`split_into`) | interoperability-tested | `MB-READ-MERGE`; `interop_split_batch_discloses_chunks` |
| Modbus TCP | reconnect, read retry, cancellation, timeout, late-reply discard, resync | simulator-tested | `FakeTransport` and fake clock; `MB-READ-RECONNECT`, `MB-READ-TIMEOUT`, `MB-READ-RETRY` |
| Modbus TCP | write coils (FC15) and holding registers (FC06, FC16), refused unless the connection says `access=read-write`; a write that gets no answer is `Unknown` and is never replayed | interoperability-tested (happy and exception paths; `Unknown` and the halt are simulator-tested) | `MB-TCP-WRITE`; `interop_writes_a_holding_register_and_a_coil_and_reads_them_back`, `interop_writes_multi_register_types_in_the_documented_word_layouts` |
| Modbus TCP | subscriptions by polling: cyclic and change-of-state, event refused (as upstream's polling emulation) | simulator-tested | `MB-TCP-SUBSCRIBE`; fake transport and fake clock, the six subscription contract cases |
| Modbus TCP | ping (one read of the configured `ping-address`) | interoperability-tested | `MB-TCP-PING`; `interop_ping_reads_each_configured_area_and_succeeds`, `interop_ping_of_an_address_the_server_refuses_still_answers` |
| Modbus TCP | browse, discover | not supported | `MB-TCP-BROWSE`, `MB-TCP-DISCOVER` (deferred) |
| Modbus TCP | over UDP (`modbus-tcp:udp://host[:port]`, default port 502): reads, writes and ping, a lost, delayed, duplicated or reordered reply ending in `Timeout` or discarded by transaction id | interoperability-tested (job `interop-udp`, run 40830080420, 6 of 6); simulator-tested on loopback UDP | `MB-TCP-TRANSPORTS-UDP`, `UTR-UDP`; diagslave 3.6 with `tc netem` (job `interop-udp`, first hosted run 40830080420, 6 of 6 on 2026-10-29), `interop_udp_netem_reordered_replies_never_answer_the_wrong_request`; ADR-0027 |
| Modbus TCP | TLS (ASCII is the `modbus-ascii` driver, below) | not supported | `MB-TCP-TRANSPORTS-TLS`, `MB-TCP-TRANSPORTS-TLS-PSK` (deferred); the parser rejects them |
| Modbus TCP | array selections `[i]`, `[lo..hi]`, `;base`: a range read and written as one `PlcValue::List`, on the extended registers too | simulator-tested | `MB-ADDR-SELECTION`; fake transport, including the upstream array read and write exchanges |
| Modbus TCP | extended registers (FC20 read, FC21 write) | simulator-tested | `MB-ADDR-EXTENDED`, `MB-LIMIT-EXT`; fake transport and the first-party responder (no interop peer serves FC20 or FC21) |
| Modbus TCP | text: `CHAR`, `WCHAR`, `STRING(n)`, `WSTRING(n)` read and written as `PlcValue::Char` and `PlcValue::String`; bytes that are not UTF-8 or UTF-16 are an item error | simulator-tested (the pymodbus interop test passed locally, S49) | `MB-TYPE-CHAR` to `MB-TYPE-WSTRING`, ADR-0025; `interop_reads_and_writes_text_registers` |
| Modbus TCP | calendar types | not supported | `MB-TYPE-TIME` (rejected with typed errors) |
| Modbus RTU | read, write, ping and polling subscriptions over RTU framing (CRC-16) on a serial port or a serial-to-IP gateway (`modbus-rtu:tcp://`) | simulator-tested | `MBS-OP-*`, `MBS-RTU-*` in `docs/parity/modbus-serial.md`; fake transport and fake clock, the contract suite, the upstream RTU vectors, a fuzz target; ADR-0024 |
| Modbus RTU | CRC errors, the silence before each request, a late reply after a timeout, a lost serial link and its reopen, replies from another unit | simulator-tested | `MBS-RTU-CRC`, `MBS-RTU-SILENCE`, `MBS-BUS-LATE-REPLY`, `MBS-BUS-CORRELATION` |
| Modbus RTU | an independent RTU server, a serial device; broadcast and shared ports | planned; broadcast and shared ports not supported | no interop peer or hardware record yet (HW2); `MBS-CFG-UNIT-ID`, `MBS-BUS-SHARED-PORT` |
| Modbus ASCII | read, write, ping and polling subscriptions over ASCII framing (a colon, hex, LRC, CR LF) on a serial port, 8 or 7 data bits, or a serial-to-IP gateway (`modbus-ascii:tcp://`) | simulator-tested | `MBS-OP-*`, `MBS-ASCII-*` in `docs/parity/modbus-serial.md`; fake transport and fake clock, the contract suite, the upstream ASCII vectors, a fuzz target; ADR-0024 addendum S60 |
| Modbus ASCII | LRC errors, bad digits, a missing CR or LF, noise before the colon, a late reply after a timeout, a lost serial link and its reopen, replies from another unit | simulator-tested | `MBS-ASCII-LRC`, `MBS-ASCII-PARSE`, `MBS-ASCII-RESYNC`, `MBS-ASCII-TIMING`, `MBS-BUS-LATE-REPLY`, `MBS-BUS-CORRELATION` |
| Modbus ASCII | an independent ASCII server, a serial device; broadcast and shared ports | planned; broadcast and shared ports not supported | no interop peer or hardware record yet (HW2); `MBS-CFG-UNIT-ID`, `MBS-BUS-SHARED-PORT` |
| Simulator (`sim`) | read, write, scripted failures, bounded read retry | simulator-tested | driver contract suite; `docs/parity/sim.md` |
| Sensor profiles | load, validate, build tags, scale and grade measurements | simulator-tested | synthetic `sim` profiles, `profile_read` in the `examples` job |
| Sensor profiles | the same synthetic profiles against a Modbus TCP server | interoperability-tested | synthetic `modbus-tcp` profiles, `profile_read` in the `interop` job; green on the hosted runner for S30 (the `profiles` step, run 40548582722) |
| Board adapters (`boards`) | MAX31855 over SPI: thermocouple and cold-junction temperature, open and short flags, negative readings, refused frames, the three measurements (`Bad` thermocouple with no value under a fault); the Linux `/dev/spidev` read backend (`adapters-linux`) | simulator-tested | scripted `FakeSpiBus`, in-memory readers and property tests (ADR-0026); an extension with no upstream parity row; no converter was read |
| Any physical device | everything | planned | `docs/hardware/TEMPLATE.md` only; no record exists |
| Linux aarch64 and armv7 | compile | build-tested | `cross` job (`cargo check`, no tests) |
| macOS, Windows | compile and test | build-tested | CI `test` matrix; results in [`STATUS.md`](STATUS.md) |

The synthetic profiles describe no real device and are labeled `synthetic`; a named
device profile needs a real register map and its own fixtures. The full matrix with
data types, byte-order modes, configuration keys, tested peers and platforms is
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md). Every upstream behavior claim has a
parity row in [`docs/parity/`](docs/parity); divergences from Apache PLC4X are listed
there with their references.

## Quick start
Clone the repository (the profiles are repository data, not shipped in the package) and run
the `profile_read` example from its root. It loads a synthetic profile, builds the tags
through the driver the connection string names, reads them with a 2 second deadline,
and logs name, value, unit, quality, origin, raw value and `sampled_at` for every
measurement through `tracing`. `--expect-good` makes it exit with status 1 unless every
measurement is of quality `Good`.

Against the simulator, with no network:

```sh
cargo run -p stonecrab --example profile_read --features full -- --expect-good
```

Against a Modbus TCP server, the pymodbus 3.8.6 container of the interoperability job
(needs Docker; run `mise install` first):

```sh
mise run interop
```

That task builds and starts `interop/pymodbus/`, runs the interop tests, the
`modbus_tcp_read` example and `profile_read --expect-good` for each profile under
`profiles/synthetic/modbus-tcp/`, writes the evidence to `target/interop-evidence/` and
removes the container. To drive one profile by hand against a running server:

```sh
interop/pymodbus/interop.sh build && \
  interop/pymodbus/interop.sh start && \
  interop/pymodbus/interop.sh wait
STONECRAB_URL='modbus-tcp://127.0.0.1:5020?address-convention=plc4x' \
  STONECRAB_PROFILE=profiles/synthetic/modbus-tcp/temperature_transmitter.toml \
  cargo run -p stonecrab --example profile_read --features full -- --expect-good
interop/pymodbus/interop.sh stop
```

To use a driver from your own code, add the crate (`cargo add stonecrab`; the default
features include the simulator, Modbus TCP and the sensor profiles) and read the crate
README:
[`crates/stonecrab/README.md`](crates/stonecrab/README.md) has the feature list and the two
smaller examples, `sim_read` and `modbus_tcp_read`, and the
[Modbus component document](crates/stonecrab/docs/components/modbus.md) has the
connection-string and address grammars.

## Current status
See [`STATUS.md`](STATUS.md) for the current engineering handoff, verification
evidence, known gaps and continuation steps, and
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) for what is supported, at which level and
against which peers. The 0.1.0 package on crates.io is specification-stage only; 0.2.0
is the alpha. This README describes `master`: Modbus writes landed after 0.2.0 (see
"Unreleased" in [`CHANGELOG.md`](CHANGELOG.md)), so the published 0.2.0 reads only.
Upstream compatibility is claimed only for the rows of
[`docs/parity/modbus.md`](docs/parity/modbus.md) that say so, at the support levels above.

The original [project brief](docs/PROJECT_BRIEF.md), [specification](docs/SPEC.md),
[architecture](docs/ARCHITECTURE.md), and [coding standards](docs/CODING_STANDARDS.md)
are retained as source documents. The architecture document marks which of its diagrams
describe shipped behavior and which are deferred targets.

## Development
Install [mise](https://mise.jdx.dev/), then run `mise install`. Each task runs the commands of
the CI job or step it is named for (the mapping is in the header of `mise.toml`). The gates
run with `--all-features`, so they exercise the test kit too:

- `mise run check`: formatting, strict Clippy, tests, doctests, documentation with
  warnings denied (the crate README and every component document are doctests) and the
  no-default-features build.
- `mise run coverage`: the per-module line-coverage thresholds of ADR-0005.
- `mise run lint-attrs`: lint attributes in `lib.rs` against the section 5.1 table, crate
  level and per module.
- `mise run features`: the feature powerset (depth 2) and the crate without defaults.
- `mise run machete`: unused dependencies. `mise run dependencies`: advisories, licenses,
  bans and sources.
- `mise run fuzz`: the fuzz smoke check, 60 seconds per target.
- `mise run examples`: builds every example and runs `sim_read` and `profile_read` against
  the simulator.
- `mise run interop`: the pymodbus run described above (needs Docker).
- `mise run parity`: every `# Compatibility` id in the sources resolves to a parity row and
  every implemented row is referenced (ADR-0005 addendum).
- `mise run package`: the crate archive against its reviewed allow-list.
- `mise run dco -- <range>`: `Signed-off-by` on every commit in a range.

A plain `cargo test` builds only the default features and so runs a reduced suite; use
`cargo test --all-features` (what `mise run check` runs) for everything. The test kit's own
tests read repository files, so the full suite runs from a checkout.

The pinned development compiler is Rust 1.99.1, edition 2024; MSRV is 1.97.

Stonecrab is one crate, `crates/stonecrab`: the core contracts are at its root and the
simulator, Modbus TCP driver, sensor profiles, transport, codec and test kit are modules
behind the features above (ADR-0018). The core needs no executor.
See [contributing](CONTRIBUTING.md), [security](SECURITY.md),
[license](LICENSE), [notices](NOTICE), and [changelog](CHANGELOG.md).
Stonecrab is not affiliated with the Apache Software Foundation or Apache PLC4X.
