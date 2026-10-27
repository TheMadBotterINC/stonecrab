# Stonecrab
**Apache PLC4X, carcinized.** An independent native Rust port, built for sensor acquisition first.
Copyright 2026 The Mad Botter INC and the Stonecrab contributors. Apache-2.0.

Stonecrab is **alpha** software. The alpha can read coils, discrete inputs, holding
registers and input registers from a Modbus TCP server through one object-safe driver
boundary (`DriverRegistry`, `Driver`, `Connection`), turn the values into scaled, qualified
sensor measurements with declarative profiles, and run the same code against an
in-process simulator with no network. It has been checked against an independent server,
pymodbus 3.8.6, and has never touched a physical device. It does not write, subscribe,
browse, discover or speak Modbus RTU, and its APIs may change before 1.0. The only
published version is the specification-stage placeholder `stonecrab` 0.1.0; the alpha
versions (`stonecrab` 0.2.0 and the 0.1.0 sibling crates) are unreleased.

## Capability matrix
Support levels use only the labels of the project's hardware-evidence policy
([`docs/SPEC.md`](docs/SPEC.md)): `build-tested`, `simulator-tested`,
`interoperability-tested` and `hardware-verified`, plus `planned` and `not supported`.
`interoperability-tested` means a run against an independent implementation (here
pymodbus 3.8.6 in the `interop` CI job). Nothing is `hardware-verified`.

| Protocol | Operation | Support level | Evidence |
| --- | --- | --- | --- |
| Modbus TCP | connect, close, connection state | interoperability-tested | `interop` job; parity row `MB-TCP-CONNECT` |
| Modbus TCP | read coils (FC01) as `BOOL` | interoperability-tested | `MB-TCP-READ-COIL`; `interop_reads_coils_1_to_10` |
| Modbus TCP | read discrete inputs (FC02) as `BOOL` | interoperability-tested | `MB-TCP-READ-DI`; `interop_reads_discrete_inputs_1_to_10` |
| Modbus TCP | read holding registers (FC03) as the 15 numeric and bit types | interoperability-tested | `MB-TCP-READ-HR`; `interop_reads_all_fifteen_holding_register_types` |
| Modbus TCP | read input registers (FC04) as the 15 numeric and bit types | interoperability-tested | `MB-TCP-READ-IR`; `interop_reads_input_registers_1_to_7` |
| Modbus TCP | batched reads with split disclosure (`split_into`) | interoperability-tested | `MB-READ-MERGE`; `interop_split_batch_discloses_chunks` |
| Modbus TCP | reconnect, read retry, cancellation, timeout, late-reply discard, resync | simulator-tested | `FakeTransport` and fake clock; `MB-READ-RECONNECT`, `MB-READ-TIMEOUT`, `MB-READ-RETRY` |
| Modbus TCP | write | not supported | `MB-TCP-WRITE` (deferred); the driver reports `write: false` and sends nothing |
| Modbus TCP | subscriptions (upstream emulates them by polling) | not supported | `MB-TCP-SUBSCRIBE` (deferred, M2) |
| Modbus TCP | browse, discover, ping | not supported | `MB-TCP-BROWSE`, `MB-TCP-DISCOVER`, `MB-TCP-PING` (deferred) |
| Modbus TCP | RTU, ASCII, UDP, TLS | not supported | `MB-TCP-TRANSPORTS-*` (deferred); the parser rejects them |
| Modbus TCP | extended registers (FC20/21), strings, calendar types, array selections | not supported | `MB-ADDR-EXTENDED`, `MB-ADDR-STRING`, `MB-TYPE-TIME`, `MB-ADDR-SELECTION` (rejected with typed errors) |
| Simulator (`sim`) | read, write, scripted failures, bounded read retry | simulator-tested | driver contract suite; `docs/parity/sim.md` |
| Sensor profiles | load, validate, build tags, scale and grade measurements | simulator-tested | synthetic `sim` profiles, `profile_read` in the `examples` job |
| Sensor profiles | the same synthetic profiles against a Modbus TCP server | interoperability-tested | synthetic `modbus-tcp` profiles, `profile_read` in the `interop` job; green on the hosted runner for S30 (the `profiles` step, run 40548582722) |
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
Clone the repository (the profiles are repository data, not shipped in a crate) and run
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

To use a driver from your own code before the alpha is published, depend on the checkout
by path and enable the feature you need (`stonecrab = { path = "crates/stonecrab",
features = ["modbus"] }`; without features only the core contracts are pulled) and read
the crate README: [`crates/stonecrab/README.md`](crates/stonecrab/README.md) has the
feature list and the two smaller examples, `sim_read` and `modbus_tcp_read`, and
[`crates/stonecrab-modbus/README.md`](crates/stonecrab-modbus/README.md) has the
connection-string and address grammars.

## Current status
See [`STATUS.md`](STATUS.md) for the current engineering handoff, verification
evidence, known gaps and continuation steps, and
[`docs/COMPATIBILITY.md`](docs/COMPATIBILITY.md) for what is supported, at which level and
against which peers. The published 0.1.0 crate is specification-stage only. Upstream
compatibility is claimed only for the rows of [`docs/parity/modbus.md`](docs/parity/modbus.md)
that say so, at the support levels above.

The original [project brief](docs/PROJECT_BRIEF.md), [specification](docs/SPEC.md),
[architecture](docs/ARCHITECTURE.md), and [coding standards](docs/CODING_STANDARDS.md)
are retained as source documents. The architecture document marks which of its diagrams
describe shipped behavior and which are deferred targets.

## Development
Install [mise](https://mise.jdx.dev/), then run `mise install`. Each task runs the commands of
the CI job or step it is named for (the mapping is in the header of `mise.toml`):

- `mise run check`: formatting, strict Clippy, tests, doctests, documentation with
  warnings denied (every crate README is a doctest) and the no-default-features build.
- `mise run coverage`: the per-crate line-coverage thresholds of ADR-0005.
- `mise run lint-attrs`: crate-level lint attributes against the section 5.1 table.
- `mise run features`: the feature powerset (depth 2) and each crate without defaults.
- `mise run machete`: unused dependencies. `mise run dependencies`: advisories, licenses,
  bans and sources.
- `mise run fuzz`: the fuzz smoke check, 60 seconds per target.
- `mise run examples`: builds every example and runs `sim_read` and `profile_read` against
  the simulator.
- `mise run interop`: the pymodbus run described above (needs Docker).
- `mise run parity`: every `# Compatibility` id in the sources resolves to a parity row and
  every implemented row is referenced (ADR-0005 addendum).
- `mise run package`: the crate archives against their reviewed allow-lists.
- `mise run dco -- <range>`: `Signed-off-by` on every commit in a range.

The pinned development compiler is Rust 1.99.1, edition 2024; MSRV is 1.97.

The facade is in `crates/stonecrab`, pure contracts in `crates/stonecrab-core`; the
simulator, Modbus TCP driver, sensor profiles, transport and codec are their own crates,
and `tools/stonecrab-testkit` holds the unpublished test doubles. The facade and core have
empty default features; no executor is required by core.
See [contributing](CONTRIBUTING.md), [security](SECURITY.md),
[license](LICENSE), [notices](NOTICE), and [changelog](CHANGELOG.md).
Stonecrab is not affiliated with the Apache Software Foundation or Apache PLC4X.
