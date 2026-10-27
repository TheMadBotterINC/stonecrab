# Stonecrab compatibility matrix

Status: alpha, reconciled with the code in slice S31 of
[`ALPHA_PLAN.md`](ALPHA_PLAN.md). Everything below is true of the code at that slice;
nothing is promised that is not shipped, and deferred items say what owns them.

Upstream baseline: Apache PLC4X v1.0.0, tag object
`32a524591841ab56d26b032914ce06918148fbb6`, commit
`ea229918f5d07700e04d7d69ab1645da679cb700`, pinned in
[`upstream/manifest.toml`](../upstream/manifest.toml). The manifest's inventory is
incomplete (`inventory_complete = false`) and the source-release signature is not verified
(`signature_verified = false`); only the Modbus TCP subset is inventoried, in
[`parity/modbus.md`](parity/modbus.md). Stonecrab is an independent port: it is not
affiliated with the Apache Software Foundation or Apache PLC4X, and no Apache PLC4X source
file is copied into this repository.

## Support levels

Four levels come from the hardware-evidence policy of [`SPEC.md`](SPEC.md); `planned` and
`not supported` say a capability is absent. Each level names the strongest evidence that
exists, not the intended one.

| Level | Meaning here |
| --- | --- |
| `build-tested` | compiles on the platform, with unit and documentation tests; no peer involved |
| `simulator-tested` | exercised against the in-process simulator, the in-memory `FakeTransport` with a fake clock, or a loopback `ScriptedPeer` |
| `interoperability-tested` | exercised against an independent implementation: pymodbus 3.8.6 in the `interop` CI job |
| `hardware-verified` | exercised against a physical device, with a record from `docs/hardware/TEMPLATE.md` |
| `planned` | not shipped; the owner is named where one is known (hardware evidence: milestone M2S) |
| `not supported` | not shipped and rejected with a typed error rather than ignored |

Nothing is `hardware-verified`. No physical device, adapter or PLC has been tried, and no
hardware record exists.

## Protocols and operations

| Protocol | Operation | Level | Parity row | Evidence |
| --- | --- | --- | --- | --- |
| Modbus TCP | connect, close, connection state | interoperability-tested | `MB-TCP-CONNECT` | `interop` job; `interop_contract_suite_passes_against_pymodbus` |
| Modbus TCP | read coils (FC01) | interoperability-tested | `MB-TCP-READ-COIL` | `interop_reads_coils_1_to_10` |
| Modbus TCP | read discrete inputs (FC02) | interoperability-tested | `MB-TCP-READ-DI` | `interop_reads_discrete_inputs_1_to_10` |
| Modbus TCP | read holding registers (FC03) | interoperability-tested | `MB-TCP-READ-HR` | `interop_reads_all_fifteen_holding_register_types`, `interop_little_endian_region_decodes_real` |
| Modbus TCP | read input registers (FC04) | interoperability-tested | `MB-TCP-READ-IR` | `interop_reads_input_registers_1_to_7` |
| Modbus TCP | batched reads, adjacent merge, `split_into` disclosure | interoperability-tested | `MB-READ-MERGE`, `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS` | `interop_split_batch_discloses_chunks`, `interop_125_register_and_2000_coil_reads` |
| Modbus TCP | per-chunk device exception 02 (illegal data address) | interoperability-tested | `MB-EXC-02` | `interop_read_beyond_map_yields_device_exception` |
| Modbus TCP | other device exception codes (01, 03 to 0B) | simulator-tested | `MB-EXC-01`, `MB-EXC-03` to `MB-EXC-0B` | unit tests and `FakeTransport`; never seen from an independent server |
| Modbus TCP | request-driven reconnect with capped jittered backoff | simulator-tested | `MB-READ-RECONNECT` | `FakeTransport` and fake clock; Stonecrab extension, upstream has none |
| Modbus TCP | read retry, cancellation, timeout, late-reply discard, resync | simulator-tested | `MB-READ-RETRY`, `MB-READ-TIMEOUT`, `MB-READ-MATCH` | `FakeTransport` and fake clock (D21) |
| Modbus TCP | write (FC05, 06, 15, 16) | not supported | `MB-TCP-WRITE` | deferred; `Capabilities.write = false`, `write` returns `Unsupported(Operation::Write)` before any I/O |
| Modbus TCP | subscriptions (upstream: polling emulation) | not supported | `MB-TCP-SUBSCRIBE` | deferred, M2 |
| Any physical device | everything | planned | none | milestone M2S (board adapters, hardware runner, hardware records); no record exists |
| Modbus TCP | browse | not supported | `MB-TCP-BROWSE` | upstream has no Modbus browse |
| Modbus TCP | discover, ping | not supported | `MB-TCP-DISCOVER`, `MB-TCP-PING` | deferred; `ping-address` is validated and unused |
| Modbus TCP | UDP, TLS, TLS-PSK, `test` transports | not supported | `MB-TCP-TRANSPORTS-*` | deferred; `InvalidConfig` |
| Modbus TCP | extended registers (FC20, FC21) | not supported | `MB-ADDR-EXTENDED` | deferred, M2; `InvalidAddress` |
| Modbus RTU, ASCII | everything | planned | none yet | milestone M2 |
| Simulator (`sim`) | connect, read, write, scripted failures, bounded read retry | simulator-tested | `SIM-DRIVER`, `SIM-CONNECTION`, `SIM-FAILURE-SCRIPT` | driver contract suite (22 passed, 1 named skip) |
| Sensor profiles | load, validate, build tags, scale and grade measurements | simulator-tested | `SENSORS-*` | `sim` profiles through `profile_read` in the `examples` job |
| Sensor profiles | the same, with the `modbus-tcp` synthetic profiles | interoperability-tested | `SENSORS-SYNTHETIC-PROFILES` | `profile_read --expect-good` in the `interop` job; green on the hosted runner for S30 (the `profiles` step, run 40548582722) |
| OPC UA, EtherNet/IP, S7, other baseline drivers | everything | planned | none yet | milestones M3 to M5 |
| Direct bench adapters (MAX31855, ADXL345) | everything | planned | none | milestone M2S |

The driver boundary itself (`DriverRegistry`, `Driver`, `Connection`, configuration,
address options, cancellation, the state watch, diagnostics) is `simulator-tested` through
the common driver contract suite and `interoperability-tested` on a wire through the
Modbus driver (`CORE-DRIVER-TRAIT`, `CORE-CONNECTION-TRAIT`, `CORE-REGISTRY`).

## Data types

Registers are 16 bits. A multi-register value is laid out by the byte-order mode below.
`ModbusDataType` has the 15 numeric and bit types; one element per tag.

| Modbus area | Function code | Types accepted | Values |
| --- | --- | --- | --- |
| `coil` (`0x`) | 01 | `BOOL` only | `Bool` |
| `discrete-input` (`1x`) | 02 | `BOOL` only | `Bool` |
| `holding-register` (`4x`) | 03 | all 15 | see below |
| `input-register` (`3x`) | 04 | all 15 | see below |

| Type | Registers | `PlcValue` |
| --- | --- | --- |
| `BOOL` | 1 (bit 0 under big-endian) | `Bool` |
| `BYTE`, `USINT` | 1 (low byte under big-endian) | `U8` |
| `SINT` | 1 (low byte under big-endian) | `I8` |
| `WORD`, `UINT` | 1 | `U16` |
| `INT` | 1 | `I16` |
| `DWORD`, `UDINT` | 2 | `U32` |
| `DINT` | 2 | `I32` |
| `REAL` | 2 | `F32` (bit for bit, NaN payloads and infinities kept) |
| `LWORD`, `ULINT` | 4 | `U64` |
| `LINT` | 4 | `I64` |
| `LREAL` | 4 | `F64` (bit for bit) |

Not supported, and rejected at parse time with `InvalidAddress`: a type other than `BOOL`
on a bit area (`MB-ADDR-BIT-TYPES`), `STRING`, `WSTRING`, `CHAR`, `WCHAR`
(`MB-ADDR-STRING`), the `TIME`, `LTIME`, `DATE`, `LDATE`, `TIME_OF_DAY`, `LTIME_OF_DAY`,
`DATE_AND_TIME` and `LDATE_AND_TIME` names (`MB-TYPE-TIME`), array selections
(`MB-ADDR-SELECTION`) and `extended-register` (`MB-ADDR-EXTENDED`). Core has no `Bytes`,
`List`, `Struct`, `Raw` or calendar value variants (ADR-0006).

Address text: the long form `holding-register:16:INT` and the prefixed short form
`4x00016:INT` are always accepted; the bare short forms (`40001`) are accepted only when
the caller selects `address-convention=plc4x` (profiles declare it explicitly). The
logical number is one-based; the wire address is one less. See the grammar in the
[Modbus README](../crates/stonecrab-modbus/README.md).

## Byte-order modes

| `default-payload-byte-order` and `byte-order` block | `ByteOrder` | `WordOrder` | `0x01020304` on the wire | Evidence |
| --- | --- | --- | --- | --- |
| `BIG_ENDIAN` (default) | `BigEndian` | `HighWordFirst` | `[1,2,3,4]` | `MB-BYTEORDER-BE`: first-party vectors; pymodbus map |
| `LITTLE_ENDIAN` | `LittleEndian` | `LowWordFirst` | `[4,3,2,1]` | `MB-BYTEORDER-LE`: first-party vectors; one REAL read from pymodbus |
| `BIG_ENDIAN_BYTE_SWAP` | `LittleEndian` | `HighWordFirst` | `[2,1,4,3]` | `MB-BYTEORDER-BE-SWAP`: first-party vectors |
| `LITTLE_ENDIAN_BYTE_SWAP` | `BigEndian` | `LowWordFirst` | `[3,4,1,2]` | `MB-BYTEORDER-LE-SWAP`: first-party vectors |

The mapping is derived from the documented layouts. The rows are `implemented`, not
`automated-verified`, because the upstream little-endian vectors are not in the repository
(maintainer prerequisite P4). Precedence: the tag's own order, then the address block, then
the connection default; the order is never inferred from the values.

## Configuration keys

Connection string: `modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]` (port 502
by default, IPv6 in brackets, `tcp` the only transport code), then `?key=value&...`. A key
may appear once and every other key is `InvalidConfig` (upstream warns and ignores it).

| Key | Default | Accepted | Parity row |
| --- | --- | --- | --- |
| `request-timeout-ms` | 5000 | whole milliseconds above 0; also `Timeouts.request` | `MB-CFG-REQUEST-TIMEOUT` |
| `default-unit-identifier` | 1 | 0 to 255 | `MB-CFG-UNIT-ID` |
| `ping-address` | `4x00001:BOOL` | an address of the grammar, read under `plc4x`; recorded, unused | `MB-CFG-PING-ADDRESS` |
| `default-payload-byte-order` | `BIG_ENDIAN` | `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP` | `MB-CFG-BYTE-ORDER` |
| `max-coils-per-request` | 2000 | 1 to 2000 | `MB-CFG-MAX-COILS` |
| `max-registers-per-request` | 125 | 1 to 125 | `MB-CFG-MAX-REGISTERS` |
| `address-convention` | none | `plc4x` | `MB-ADDR-BARE-40001` |

Simulator: `sim://<device>?latency-ms=<n>&seed=<u64>`; `address-convention` is accepted
and ignored. Core connection defaults (`Timeouts` 5 s connect and 5 s request, `idle`
unenforced; `ReconnectPolicy` 250 ms initial, 30 s cap, multiplier 2.0, jitter 0.2,
unlimited attempts; `RetryPolicy` one attempt, retrying `Timeout`, `Disconnected` and
`Backpressure`; `AccessPolicy::ReadOnly`) are rows `CORE-CONFIG-SHAPE`,
`CORE-RECONNECT-POLICY` and `CORE-RETRY-POLICY`.

## Peers it was verified against

| Peer | What ran | Where | Level it supports |
| --- | --- | --- | --- |
| pymodbus 3.8.6 on `python:3.12-slim` (pinned by digest), first-party `server.py` over the register map in `fixtures/modbus/interop/register-map.toml`, TCP port 5020 | ten `interop_` tests, the `modbus_tcp_read` example, `profile_read --expect-good` for each `modbus-tcp` synthetic profile, the common driver contract suite (14 passed, 9 skipped by name) | the CI job `interop` (ubuntu, every push and pull request) and `mise run interop` | `interoperability-tested` |
| the in-process simulator (`sim`) | the driver contract suite (22 passed, 1 skipped), `sim_read`, `profile_read` on the `sim` profiles | `cargo test`, CI jobs `test` and `examples` | `simulator-tested` |
| loopback `ScriptedPeer` (testkit) | a holding-register read over real sockets and the contract suite (18 passed, 5 skipped) | `cargo test` | `simulator-tested` |
| `FakeTransport` with a first-party MBAP responder and `FakeClock` | the contract suite (21 passed, 2 skipped), timing, retry, reconnect and cancellation tests | `cargo test` | `simulator-tested` |

pymodbus 3.8.6 is the peer the upstream project's own Docker test uses; the Stonecrab
server is written independently and reproduces only the documented fixture values plus one
little-endian region. What the run does not verify: timing over a real clock, dropped links
and device exceptions on demand (a stock pymodbus has no fault hooks), unit-id correlation
(pymodbus answers every unit id), and any other Modbus server. No physical device has been
tried.

## Platforms and toolchains

| Platform | Toolchains | What CI does | Level |
| --- | --- | --- | --- |
| Linux x86_64 (`ubuntu-latest`) | 1.97.1 and 1.99.1 | build, nextest, `cargo test` with doctests; the quality, coverage, interop, examples and fuzz-smoke jobs | `build-tested`; Modbus rows as above |
| macOS (`macos-latest`) | 1.97.1 and 1.99.1 | build, nextest, `cargo test` | `build-tested` |
| Windows (`windows-latest`) | 1.97.1 and 1.99.1 | build, nextest, `cargo test` | `build-tested` |
| `aarch64-unknown-linux-gnu` | 1.99.1 | `cargo check --workspace --all-features` only (the `cross` job) | `build-tested` (compile check, no tests) |
| `armv7-unknown-linux-gnueabihf` | 1.99.1 | the same | `build-tested` (compile check, no tests) |

Per-OS results of the most recent hosted run are recorded in [`STATUS.md`](../STATUS.md).
The drop-mid-write transport test is ignored on Windows, where loopback accepts a 32 MiB
write at once. Docker-based interoperability runs on Linux only.

MSRV: Rust 1.97 (`rust-version` in the workspace manifest), exercised with 1.97.1 in the
`test` matrix. The development toolchain is pinned to 1.99.1 in `rust-toolchain.toml` and
`mise.toml`. Rust edition 2024.

## Mandatory failure cases (SPEC.md)

The applicable cases of [`SPEC.md`](SPEC.md) "Mandatory failure cases" are tagged in the
tests with `// covers: SPEC mandatory-failure-cases / F<n> ...`; list them with
`grep -rn "covers: SPEC mandatory-failure-cases / F3" crates tools`. The numbering is the
order of the SPEC bullets. Counts are the number of tagged tests at S31.

| Tag | SPEC case | Alpha coverage | Tagged tests |
| --- | --- | --- | --- |
| F1 | truncated headers and payloads, oversized lengths, malformed discriminators, unexpected function codes (invalid checksums are RTU and out of scope) | codec, PDU codec, MBAP framer, connection strings, profile loader | 50 |
| F2 | TCP fragmentation and coalescing, peer close at each framing boundary, wrong transaction and unit ids, delayed replies after a timeout | transport, framing, session | 71 |
| F3 | device exception, invalid register, unsupported capability, mixed success within a batch | simulator, Modbus connection, session | 69 |
| F4 | cancellation while connecting, queued, transmitting and awaiting a response | guard, simulator, Modbus connection; the queued case is exercised on `ConnectionGuard` only, not on the simulator, which has no wait queue | 44 |
| F5 | device restart, outage, recovery, exhausted retry budget, shutdown during reconnect | guard, Modbus connection, `FakeTransport` | 32 |
| F6 | queue exhaustion, slow consumer, dropped events, stale last-known values, timestamp absence | queue exhaustion (`Backpressure`) and timestamp absence only; slow consumer, dropped events and stale values need subscriptions and the polling loop (M2) | 4 |
| F7 | negative and scaled values, swapped words, integer boundaries, counter rollover, NaN and infinity, incompatible profile version | typed decoder, scaling pipeline, profile loader | 36 |

The RTU, OPC UA and direct-sensor cases of SPEC.md are out of scope for the alpha.

## Divergences from upstream

Every divergence has a row, an upstream reference at `file:line` of the pinned commit, a
protocol-standard section or an upstream-issue note, and a regression test, in the
"Divergences from upstream" table of [`parity/modbus.md`](parity/modbus.md). The rows whose
reference says "upstream issue to be filed before S32" become a URL or are dropped in S32.

| Row | Stonecrab differs from upstream by |
| --- | --- |
| `MB-ADDR-BARE-40001` | bare short forms (`40001`) need an explicit convention |
| `MB-ADDR-BIT-TYPES` | a non-`BOOL` type on a coil or discrete input is refused at parse time, not at read time |
| `MB-CFG-UNIT-ID` | the unit identifier is bounded to 0..=255 |
| `MB-READ-MATCH` | a reply must match on transaction id, unit id and function code, not the transaction id alone |
| `MB-CFG-UNKNOWN` | an unknown connection parameter is `InvalidConfig`, not a warning |
| `MB-EXC-03` | exception 3 is `DeviceException` with the raw code, not `INVALID_DATA` |
| `MB-EXC-05` | exception 5 (ACKNOWLEDGE) is `DeviceException`, never success |
| `MB-EXC-UNKNOWN` | an exception code outside the ten defined ones is refused as malformed |
| `MB-ADDR-SELECTION`, `MB-ADDR-STRING`, `MB-TYPE-TIME`, `MB-ADDR-EXTENDED` | deferred address features are typed `InvalidAddress` rejections |
| `MB-READ-GAP` | no gap reads: only adjacent or overlapping tags merge |
| `MB-LIMIT-ADU` | the MBAP length is bounded (2..=254, ADU at most 260 bytes) before the body is read |
| `MB-QUIRK-A`, `MB-QUIRK-B`, `MB-QUIRK-C`, `MB-QUIRK-D` | the bogus `UDPTransport` import, the split FC20 second-item loss, the dead FC05 branch and the any-reply ping are not ported |
| `MB-QUIRK-F` | the undeclared `unit-identifier` of the upstream test suite is an error |
| `MB-QUIRK-G` | `4x:INT` is a typed `InvalidAddress`, not a `NumberFormatException` (inferred) |
| `MB-QUIRK-I` | no polling receive loop |
| `MB-QUIRK-L` | byte order takes part in address equality |

Other documented differences are not divergences from a defect: request-driven reconnect
(`MB-READ-RECONNECT`) and read retry (`MB-READ-RETRY`) have no upstream equivalent,
`canonical()` renders the long form (`MB-ADDR-SHORT-4X`), the last register at wire address
65535 is reachable (`MB-LIMIT-SPAN`), and the address-block grammar is stricter about quoted
byte-order names (`MB-ADDR-CONFIG-BLOCK`). Inferences about upstream behavior are marked
`INFERENCE` in the parity record and listed under its UNVERIFIED section.

## Known gaps

Writes, subscriptions, extended registers, strings, calendar types, RTU, UDP and TLS, and
hardware verification are listed above. Also not done: mutation testing, soak and
benchmark gates, branch-coverage gating (line coverage per crate is gated, ADR-0005), a
differential test against PLC4J, and the upstream vector corpus (blocked on prerequisite
P4). [`STATUS.md`](../STATUS.md) carries the current list.
