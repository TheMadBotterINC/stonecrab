<div align="center">

```text
        ,--.                       ,--.
       (     `-._______________.-'     )
        `-.    /               \    .-'
               |    o     o    |
               |     \___/     |
            '--|               |--'
                \_____________/
               //  / / | \ \  \\
              //  / /  |  \ \  \\
             '-' '-'   '   '-' '-'
```

# Stonecrab

**Apache PLC4X, carcinized.** An independent, native Rust port of [Apache PLC4X](https://plc4x.apache.org/): one API for industrial protocols, built for sensor acquisition first.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV 1.97](https://img.shields.io/badge/MSRV-1.97-informational.svg)](docs/CODING_STANDARDS.md#2-toolchain)
[![CI](https://github.com/TheMadBotterINC/stonecrab/actions/workflows/ci.yml/badge.svg)](https://github.com/TheMadBotterINC/stonecrab/actions/workflows/ci.yml)
[![unsafe forbidden](https://img.shields.io/badge/unsafe-forbidden-success.svg)](docs/CODING_STANDARDS.md#4-unsafe-policy)

</div>

---

> **Historical document.** This is the retained project brief, written before any code
> existed. It is superseded by [`docs/SPEC.md`](docs/SPEC.md) (normative behavior) and
> [`STATUS.md`](STATUS.md) (what exists today); `stonecrab` 0.3.0 is published. The body
> below is left as written.

> **Project status at the time of writing: specification stage.** There is no code in this repository yet. The documents here define what will be built, how parity with upstream will be measured, and what the first release must prove before it claims anything. Read [`docs/SPEC.md`](docs/SPEC.md) for the engineering specification and [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for the design.

## What it is

Stonecrab is a from-scratch Rust implementation of the Apache PLC4X API, drivers, transports, and supporting tooling. It targets **behavioral parity** with a pinned upstream release: the same wire formats, address syntax, data types, response codes, and operation semantics, exposed through an idiomatic Rust API with a documented concept-by-concept mapping.

It is being built in the order an edge device needs it:

1. A shared API, a simulator, and **Modbus TCP** with sensor profiles.
2. **Modbus RTU** over serial.
3. **OPC UA** with secure sessions and native subscriptions.
4. **EtherNet/IP (Logix)** and **Siemens S7**.
5. The rest of the upstream baseline, including code generation from upstream MSpec protocol definitions.

The first use is acquisition — temperature, pressure, flow, level, discrete state, power, vibration — from industrial transmitters, remote I/O, and PLCs. Write, browse, and discovery are staged work, not abandoned work; every release states exactly which operations each driver supports and at what evidence level.

## What it is not

- **Not a binding.** No JVM, no FFI, no Java at runtime. A maintainer-only generation step may bootstrap from upstream Java tooling until the native generator exists.
- **Not a line-by-line translation.** Java exceptions become typed `PlcError`s; `CompletableFuture` becomes `Send` futures; the Rust API is idiomatic with a mapping table, not a mirror.
- **Not affiliated with the Apache Software Foundation.** See [Attribution](#attribution).
- **Not a platform.** Persistence, dashboards, alerting, ERP integration, and analytics belong in applications. The library owns connectivity and data interpretation and is usable with no cloud account, no hosted service, and no other product.
- **Not an IO-Link master, and not an analog front end.** IO-Link support means talking to an IO-Link master's northbound interface. 4–20 mA, 0–10 V, RTD, and thermocouple inputs require a transmitter or a converter board; the library does not pretend a GPIO pin is one.

## Why the name

Stone crabs are harvested for one claw and returned to the water, where they grow it back. That is the situation this library is built for: a connection on an unattended edge device gets cut, the library regrows it with capped, jittered backoff, and nothing is lost silently or replayed blindly in the meantime. It is also written in Rust, the project started in Florida during stone crab season, and the crab on the cover was going to happen either way.

## Design commitments

These hold across every crate and are enforced by CI, not by convention:

| Commitment | How it is enforced |
| --- | --- |
| **No `unsafe`, anywhere.** | `#![forbid(unsafe_code)]` in every crate; `cargo geiger` on first-party code in CI. |
| **No unbounded I/O.** | Every transport call takes a deadline. Frame sizes, buffers, pending requests, and event queues are capped before allocation. |
| **No silent loss.** | Subscriptions declare a queue policy and expose loss counters. A missed poll marks a value stale; it never republishes the old value as new. |
| **No ambiguous writes.** | `WriteOutcome::Unknown` is a first-class result. The library never replays a write to resolve it. |
| **No inferred byte order.** | Word order and addressing convention come from the tag or the profile, never from plausible-looking values. |
| **No panics on input.** | Parsers are bounded and fuzzed; `unwrap`/`expect`/`panic` are denied in library crates. |
| **No hidden parity gaps.** | One parity record per protocol, transport, and operation, each with a status and test IDs. Deferred means deferred, not done. |
| **No hardware claims without evidence.** | Support levels are *build-tested*, *simulator-tested*, *interoperability-tested*, or *hardware-verified*, each with a retained record. |

## Roadmap

Capability gates, not calendar commitments.

| Milestone | Deliverable | Status |
| --- | --- | --- |
| **M0** Baseline and contracts | Pinned upstream manifest, full capability inventory, API contract, test plan | ⬜ next |
| **M1** Native Modbus TCP | Core API, simulator, TCP codec/driver, first sensor profiles | ⬜ |
| **M2** Sensor acquisition | Modbus RTU, profile validation, stale/quality handling, bounded polling | ⬜ |
| **M2S** Board adapters | Optional MAX31855 (SPI) and ADXL345 (I²C) adapters | ⬜ |
| **M3** OPC UA | Secure connectivity, read/browse/subscriptions, staged writes | ⬜ |
| **M4** PLC drivers | EtherNet/IP/Logix and S7, ordered by customer or test-rig access | ⬜ |
| **M5** Baseline parity | Remaining protocols, operation gaps, tooling, native MSpec generation | ⬜ |

A release can be useful without claiming M5. A 1.0 version number will carry an explicit parity statement and will not imply full upstream coverage.

## Planned workspace

> Superseded: ADR-0018 ships Stonecrab as one crate, with these components as modules. The
> layout below is the original plan.

```text
stonecrab/
├── crates/
│   ├── stonecrab/                  public facade, driver registration, feature selection
│   ├── stonecrab-core/             values, tags, requests, errors, driver traits (runtime-agnostic)
│   ├── stonecrab-transport/        TCP, UDP, serial; deadlines and cancellation (Tokio)
│   ├── stonecrab-codec/            bounded readers/writers, checksums, generated-message support
│   ├── stonecrab-driver-sim/       simulated device with scripted failures
│   ├── stonecrab-modbus/           Modbus TCP + RTU over one PDU implementation
│   ├── stonecrab-opcua/            OPC UA
│   ├── stonecrab-sensors/          declarative sensor profiles, units, scaling, quality
│   ├── stonecrab-adapter-max31855/ thermocouple over SPI
│   └── stonecrab-adapter-adxl345/  accelerometer over I²C
├── tools/
│   ├── stonecrab-testkit/          scripted peers, fake clock, golden vectors, contract suite
│   ├── stonecrab-codegen/          MSpec → Rust generator (maintainer-only)
│   └── xtask/                  upstream manifest, coverage, release evidence
├── profiles/                   sensor profiles (TOML), each with provenance and fixtures
├── fixtures/                   wire vectors; upstream-derived files keep their origin and license
├── fuzz/                       cargo-fuzz targets
├── docs/
│   ├── SPEC.md                 engineering specification
│   ├── ARCHITECTURE.md         crate graph, class diagrams, state machines, sequences
│   ├── CODING_STANDARDS.md     toolchain, lints, API rules, testing, workflow
│   ├── adr/                    architecture decision records
│   ├── parity/                 one record per protocol / transport / operation
│   └── hardware/               hardware test records
├── Cargo.toml                  workspace, shared lints, shared dependencies
├── rust-toolchain.toml         pinned stable
├── deny.toml                   license, advisory, and source policy
└── LICENSE · NOTICE · CONTRIBUTING.md · SECURITY.md · CODE_OF_CONDUCT.md · CHANGELOG.md
```

Protocols are opt-in Cargo features. `cargo add stonecrab` with default features pulls in the core API and nothing else; nothing opens a socket or probes hardware by being imported.

## Target API

This is what M1 is specified to deliver. It will change before 1.0; migration notes accompany every pre-1.0 breaking change.

```rust
use std::sync::Arc;
use std::time::Duration;

use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::{DriverRegistry, PlcError, ReadRequest, Tag};

#[tokio::main]
async fn main() -> Result<(), PlcError> {
    let modbus = Arc::new(ModbusTcpDriver::default());

    let mut registry = DriverRegistry::new();
    registry.register(modbus.clone())?;

    // Connection-string syntax follows upstream; aliases and deliberate
    // differences are documented, and addresses are never silently reinterpreted.
    let conn = registry
        .connect("modbus-tcp://10.0.0.5:502?unit-id=1")
        .await?;

    // The driver parses the address. The caller supplies the stable identifier
    // that comes back on every result, event, and measurement.
    let boiler_temp = Tag::new(
        "boiler_temp",
        modbus.parse_address("holding:0x0010:i16")?,
    );

    let response = conn
        .read(
            ReadRequest::builder()
                .tag(boiler_temp)
                .deadline_after(Duration::from_millis(500))
                .build(),
        )
        .await?;

    for item in response.items {
        match item.outcome {
            Ok(value) => println!("{} = {value:?} (sampled {:?})", item.tag, item.sampled_at),
            Err(err) => eprintln!("{} failed: {err} (status {:?})", item.tag, item.status),
        }
    }

    conn.close().await?;
    Ok(())
}
```

A sensor profile turns protocol tags into named measurements with units, scaling, and quality — declaratively, with no executable code:

```toml
[profile]
id             = "acme-tx100-modbus"
schema_version = 1
validation     = "synthetic"          # synthetic | documented | fixture-backed | hardware-verified

[applicability]
manufacturer = "ACME"
model        = "TX-100"
firmware     = ">=2.1"

[connection]
scheme             = "modbus-tcp"
default_convention = "zero-based"     # 40001-style text is rejected unless declared

[[measurement]]
name         = "process_temp"
address      = "holding:0x0010"
data_type    = "i16"
scaling      = [{ multiply = 0.1 }]   # applied in declared order
unit         = "degC"
range        = { min = -50.0, max = 400.0 }
min_interval = "1s"
stale_after  = "5s"
keep_raw     = true

[[measurement.status]]
address = "holding:0x0020"
bit     = 3
meaning = "sensor_fault"

[measurement.quality]
bad_when = ["status.sensor_fault", "range.exceeded"]
```

## Hardware and support levels

Nothing is called *supported* until it is tested, and the test record says on what.

| Level | Meaning |
| --- | --- |
| **build-tested** | Compiles and passes unit tests for the target. |
| **simulator-tested** | Passes the contract suite against the scripted simulator and transport harness. |
| **interoperability-tested** | Passes against an independently implemented server or client that shares none of this code. |
| **hardware-verified** | Passes on a named device model and firmware, with the record kept in `docs/hardware/`. |

The initial bench is a BeagleBone on Debian 13 (Trixie) with a MAX31855 thermocouple converter and an ADXL345 accelerometer. It is not assumed to contain a Modbus transmitter, an RS-485 adapter, or a commercial PLC; those support claims wait for the hardware.

**Platforms:** Linux x86_64 first; aarch64 and armv7 Linux exercised in CI with hardware runs before any runtime-support claim; macOS and Windows for the core and TCP drivers once their tests run there. Serial and board adapters are qualified separately per platform.

## Contributing

Contributions are welcome once M0 lands and the first crates exist. Until then, issues and discussion on the specification are the most useful input.

When the code exists, every contribution follows [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md). The short version:

- Latest stable Rust, Rust 2024 edition, MSRV = stable − 2, `#![forbid(unsafe_code)]`.
- A driver contribution ships with capability metadata, address documentation, fixtures, failure-case tests, and evidence for its claimed support level.
- A bug fix ships with a regression test.
- Commits follow Conventional Commits and carry a DCO `Signed-off-by` line.
- Generated code is regenerated, never hand-patched.

Security issues go to the channel described in [`SECURITY.md`](SECURITY.md), not to the public tracker.

## Documentation

| Document | Purpose |
| --- | --- |
| [`docs/SPEC.md`](docs/SPEC.md) | The engineering specification: scope, parity definition, delivery order, test gates |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Crate graph, class diagrams, state machines, sequence diagrams |
| [`docs/CODING_STANDARDS.md`](docs/CODING_STANDARDS.md) | Toolchain, lints, API design rules, testing standards, workflow |
| `docs/adr/` | Architecture decision records (MADR format) |
| `docs/parity/` | Per-protocol parity records against the pinned upstream baseline |
| `docs/hardware/` | Hardware test records and the reporting template |

## License

Stonecrab is licensed under the **Apache License, Version 2.0**. See [`LICENSE`](LICENSE).

```text
Copyright 2026 The Mad Botter INC and the Stonecrab contributors

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
```

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be licensed as above, without any additional terms or conditions.

## Attribution

Stonecrab is an **independent project**. It is not produced, endorsed, or maintained by the Apache Software Foundation or the Apache PLC4X project.

This project reuses protocol descriptions, test fixtures, and behavioral references from [Apache PLC4X](https://plc4x.apache.org/), which is licensed under the Apache License 2.0. Adapted files and fixtures are identified in-tree and in [`NOTICE`](NOTICE), and retain their original attribution.

*Apache*, *Apache PLC4X*, and *PLC4X* are trademarks of the Apache Software Foundation. Their use here is nominative, to identify the upstream project this port derives from.
