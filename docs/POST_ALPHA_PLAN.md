# Stonecrab post-alpha plan

Updated: 2026-10-28. Baseline: `master` at 554ad24 (1fcc6ae before the E1 amend, same tree), over the published `stonecrab` 0.2.0 (640665a).
Status: accepted by the maintainer on 2026-10-28, except E4, which is deferred (see Decisions). Slices continue the alpha numbering from S34 and follow the alpha's rules: one Conventional Commit per green slice, signed off with a `Signed-off-by` trailer, per-module coverage gates kept, nothing marked supported without evidence.

This plan answers one question: where does Stonecrab fall short of `docs/SPEC.md` and of Apache PLC4X, and in what order should the gaps close?

## How the gaps were found

- **Against the spec.** The public API, values, errors, sensor profile, connection, test-layer and numeric-gate sections of `docs/SPEC.md` were checked against `crates/stonecrab/src`, the tests, STATUS.md and the parity records.
- **Against PLC4X.** The public site was read on 2026-10-28: the users index, the protocol support matrix and the PLC4J getting-started page. Those pages describe "latest", not the pinned v1.0.0, and the matrix's per-language check marks did not survive extraction. S41 then checked those rows against the source at v1.0.0 (commit ea229918), and `upstream/manifest.toml` holds the file and line evidence. What the site says about the Go, C, C# and Python implementations is still a lead for S42, not a parity fact. SPEC.md forbids treating a moving "latest" page as the baseline.

## Findings

### 1. Defects and contract gaps in shipped code

G01 to G06 are not disclosed anywhere. G07 to G10 are disclosed alpha deferrals, listed so their owners stay visible.

| ID | Finding | Where | Spec clause |
| --- | --- | --- | --- |
| G01 | A profile `range` is never validated. `min > max` turns every value Uncertain; a NaN bound skips the check and the value stays Good. | `sensors/validate.rs` (no range rule); `sensors/measurement/pipeline.rs:293-297` | Sensor profile contract: range checks; validation catches unsupported transformations |
| G02 | A status `bit` is not bounded by the data width. Bit 64 passes validation and silently yields `Quality::Unknown` at runtime. | `sensors/validate.rs:136-216`; `sensors/measurement/scale.rs:245` | Validation must catch invalid fields |
| G03 | Overlap detection compares the address text and bit only. Overlapping `BitField` steps, and two spellings of one register, pass. | `sensors/validate.rs:175-187` | "Profile validation must catch overlapping bit fields" |
| G04 | `sensors` parses untrusted TOML but does not deny `clippy::indexing_slicing`; codec, modbus, sim and transport do. | `src/lib.rs:36-72`; `scripts/lint-attrs.sh` tables | No panics on input |
| G05 | The explicit word-order rule is keyed on Modbus type names only. Correct today, wrong as soon as a second driver has wide types. | `sensors/validate.rs:154` | "Never infer word order" |
| G06 | `unit` is a required non-empty string, so "unit unknown" cannot be expressed. | `sensors/measurement.rs` | Envelope carries "unit when known" |
| G07 | `Connection` and `Driver` have no `subscribe`, `browse`, `discover` or `ping` method, and `Operation` has no `Ping`. Alpha decision D2 deferred them as provided methods returning `Unsupported`. | `connection.rs`, `driver.rs`, `capability.rs` | Unsupported operations return an explicit error |
| G08 | `PlcValue` has no `Bytes`, `List`, `Struct` or calendar variants. | `value.rs:57-73` | Values: bytes, arrays, structured and time types |
| G09 | `Quality::Stale` exists but nothing produces it; `stale_after` is parsed and unused. | `sensors/measurement/pipeline.rs:14-16, 44` | Stale threshold marks retained values stale |
| G10 | `ResourceLimits::max_queued_events` is never read. | `limits.rs` | Bounded event queues |

### 2. Repository hygiene and stale documents

| ID | Finding |
| --- | --- |
| H01 | The `master` tip 1fcc6ae ("fixed mise for linux") had no `Signed-off-by` trailer, so the hosted `dco` job failed (run 40688316964). Every other job in that run passed. Fixed under E1. |
| H02 | `README.md:31-34` and `:126` say 0.2.0 is unpublished and tell users to depend on the checkout. |
| H03 | `SECURITY.md:19` says "0.2.0 (once published)"; `:48` says nothing beyond 0.1.0 is published. |
| H04 | `STATUS.md:14-16` says nothing is published, contradicting `:22`. "Recommended continuation" still lists publication as pending. |
| H05 | `docs/PROJECT_BRIEF.md` carries a "specification" status badge, a CI badge on the old `themadbotter` owner and "There is no code in this repository yet". It is a retained historical brief, so it needs a header note, not a rewrite. |
| H06 | `docs/CODING_STANDARDS.md:747-748` shows the old owner URL and the `homepage` key that S33 removed. |
| H07 | No `v0.2.0` tag exists locally or on `origin`. The `semver` job is still report-only although a published baseline now exists. P1 to P4 are open and the six drafted upstream issues are unfiled. |

### 3. Spec gates not yet met

- **M0 inventory is incomplete.** `upstream/manifest.toml` has `inventory_complete = false` and nine `[pending]` items. Only Modbus TCP is inventoried. M0's exit criterion, "all target behaviors linked to a planned test", is unmet for every other protocol.
- **Upstream vectors are not redistributed** (P4). Four byte-order rows cannot reach `automated-verified`, and S04a and S04b are blocked.
- **No benchmarks.** There is no `benches/` directory, no criterion setup and no 10% regression gate.
- **No long runs.** The 8-hour release fuzz, the 24-hour acquisition soak and the memory-stability run have not happened. The recorded fuzz runs are 30 minutes per target.
- **No differential tests** against PLC4J.
- **Branch coverage is not reported.** Region coverage stands in for it.
- **Mutation testing is not a gate**, and `cargo vet` is absent.
- **No hardware record exists.** Nothing is `hardware-verified`.
- **Interop is one peer.** Only pymodbus 3.8.6, with no device exceptions on demand, no unit-id correlation and no dropped links on a real clock.

### 4. Gaps against Apache PLC4X

S41 checked the PLC4J rows below against the v1.0.0 source (commit ea229918, tag v1.0.0), and `upstream/manifest.toml` holds the file and line evidence. The Transports, Tools and Integrations paragraphs below were checked the same way, except for code that lives outside `apache/plc4x`, which S41 only located. The statements about the Go, C, C# and Python implementations were read from the same commit in S42, and the manifest holds that evidence too; none of it was run.

**API operations.** The PLC4J API declares write, three subscription kinds (cyclic, change-of-state, event) with registered handles, browse, ping and connection metadata. Discovery is on the driver, not the connection. What a given connection supports is not in the API: the five request builders always exist, an unsupported operation fails with a failed future rather than an exception, and support is derived from which `on*` hooks the connection class overrides. Connection metadata has four flags (read, write, subscribe, browse), with none for ping or discovery. Stonecrab has metadata (`Capabilities`) and read. Write exists only in the simulator.

**The Modbus driver.** `docs/parity/modbus.md` has 26 deferred rows. They cover write (FC06 versus FC16 selection, FC15 for coils), polling subscriptions, discovery by ARP scan and unit probe, ping, the UDP, TLS and TLS-PSK transports, extended registers (FC20 and FC21), strings, the TIME and DATE family, array selections and the gap-filling read optimizer. Modbus RTU and ASCII are separate upstream drivers with nothing in Stonecrab yet.

**Protocols.** The PLC4X site documents 19 protocols besides the simulator. The v1.0.0 source has 22 driver modules under `plc4j/drivers`. Stonecrab has partial Modbus TCP only.

| Protocol | Upstream at v1.0.0 (source-checked in S41) | Stonecrab milestone |
| --- | --- | --- |
| Modbus RTU, ASCII | Serial by default. TCP, TLS, TLS-PSK and UDP also accepted for gateways. Read, write (one tag per request) and ping. Subscribe is polled: cyclic and change-of-state at a 1000 ms default, event refused. No browse or discovery. RTU frames by a response-size table, ASCII by CR LF | M2 |
| OPC UA | Read, write, native subscribe (data change and events; CYCLIC is re-reported by the client), browse, and unsubscribe. TCP only (`opc.tcp`). Six security policy constants, one of them None, so five secure. Chunking in both directions, with the negotiated limits enforced on receive only. Anonymous, user name and X.509 user certificate. No ping, no discovery operation, no reconnect or session re-activation found. Read but not run: a non-None policy seems to need `server-certificate-file` in practice. Go (read from source, not run): read, write and a partial subscribe; no browse and no ping; unsubscribe returns an error; anonymous and user name only; encrypts for Basic256Sha256 only and verifies no server certificate; no chunking (`go_opcua`) | M3 |
| S7 | Discovery through PROFINET DCP over pcap confirmed. Read, write and ping native. Browse and subscribe (cyclic, alarm push, alarm query; no change-of-state) need the UserData services. The S7H pair hides subscribe. COTP transport only | M4 |
| EtherNet/IP, Logix | Polled subscribe confirmed. EtherNet/IP also has UDP ListIdentity discovery. Logix has no discovery. Read and write only, no browse or ping | M4 |
| ADS/AMS, SLMP, UMAS | ADS: read, write, native subscribe, browse, ping and UDP discovery, TCP only. SLMP: word devices D, W and R only, read, write, polled subscribe. UMAS: read, write, browse, ping, polled subscribe. The DATE and TIME_OF_DAY exclusion is not in the source: both are read and written | M5 |
| BACnet/IP | Java is a passive listener on UDP: subscribe only, and it sends no SubscribeCOV; it decodes COV notifications for present value. No read, write, browse, ping or discovery, and the tag syntax is a TODO placeholder. Go has Who-Is discovery (optionally Who-Has), read, write (single and multiple property, segmented) and a native COV subscribe that sends SubscribeCOV, but it never refreshes the subscription, so it lapses; no browse or ping (`go_bacnetip`) | M5, Go capabilities in the backlog |
| KNXnet/IP | Java on UDP, port 3671: read, write (one tag per request), push subscribe, ping and discovery by multicast search. Browse only with an ETS `knxproj` file. Go has Reader, Writer, Browser, Subscriber and Discoverer files; C# has only generated model files (179 .cs files, no driver, connection or transport); Python has none. Go also has ping and device-level reads, and its browse works without an ETS file (`go_knxnetip`, `csharp_knxnetip`) | M5 (S43: a working Java driver exists, so the website lead's "Non-Java backlog" no longer applies; the Go and C# extras stay in the backlog) |
| CANopen, generic CAN, Firmata, IEC 60870 | Narrow, push-only. CANopen: SDO read, SDO and PDO write, EVENT subscribe (PDO, NMT, heartbeat). Generic CAN: write and EVENT subscribe, no read. Firmata: digital-pin write and subscribe, no read, and unsubscribe cannot stop the board. IEC 60870-5-104: subscribe only, no read, write or interrogation. None has browse, ping or discovery | M5 |
| Profinet, AB-ETH | Profinet: browse, subscribe and discovery only, no read, write or ping. The legacy driver has no consumer registration on its connection, and the profinet-ng port never completes a subscribe. AB-ETH: read only, polled subscribe, port 2222, decodes four file types | M5 |
| C-Bus | Java is a skeleton: `canRead` is true but nothing implements it, the tag handler is null and inbound commands are only logged. Go has read (status and CAL), a narrow write (lighting-family SAL, at most 20 tags), push subscribe filtered locally (unsubscribe errors) and browse by `info/{unit}/{attribute}`; no discovery or ping (`go_cbus`) | Non-Java backlog |
| PLC4X proxy | A client driver exists in the source (code `plc4x`). TLS by default, mandatory user and password, read and write only. Subscribe is commented out in the spec | M5 |
| Open-Protocol, ctrlX, DF1 | Open-Protocol: a skeleton. Read and write are declared but unimplemented, and every tag parse throws. ctrlX implements `PlcDriver` directly (HTTPS, user name and password); ping is the only working operation, browse exists but is reported unsupported, and discovery is a stub. DF1: a spec only (`protocols/df1`), no module under `plc4j/drivers` and no Go driver, only generated Go model code and tests. SPEC.md: "Unimplemented upstream placeholders do not become obligations" | Only implemented operations bind |

The `[[backlog]]` table in `upstream/manifest.toml` lists the capabilities found only outside Java.

The source also has a simulated driver (read, write, native subscribe, ping) and a mock driver, both in-process. They are test infrastructure, not targets. Polling subscriptions come from one helper, `plc4j/utils/subscription-emulation`, which seven connections extend (Modbus TCP, RTU and ASCII, EtherNet/IP, which Logix shares, SLMP, UMAS, AB-ETH).

**Transports.** `plc4j/transports` has twelve modules and ten of them register a transport. Together they provide eleven transport codes: `tcp`, `udp`, `tls`, `tls-psk` (a second class in the TLS module), `cotp`, `serial`, `raw-socket`, `pcap-replay`, `can-socketcan`, `can-virtualcan` and `test`. The `api` module is the contract (byte-stream instances found by code through ServiceLoader or OSGi) and the `can` module is a support library (frame, ID filter, base configuration), so "CAN" alone is not a transport. The `test` transport is not on the documented list. Every transport is client-side: none listens or accepts. Source-checked caveats: TCP declares `read-timeout-ms` and `write-timeout-ms` and applies neither (the driver layer owns timeouts). UDP accepts datagrams only from the one configured remote address and its `multicast-ttl` option does nothing. TLS has `verify`, `ignore-common-name`, trust store, client keystore and version options; TLS-PSK takes only `psk-identity` and `psk-key` (no certificate or version knob). COTP wraps the TCP transport and sends every write as one data TPDU, with no segmentation or EOT reassembly. Serial drops the oldest bytes when its 64 KiB ring overflows, where TCP applies backpressure and UDP drops a whole datagram. Raw socket and PCAP replay need libpcap and ignore VLAN settings; PCAP replay is read-only and drops packets when its queue fills. SocketCAN is Linux-only; the CAN layer is CAN 2.0 only (eight data bytes); Virtual CAN is an in-process bus that ignores the ID filters. Stonecrab has TCP.

**Tools.** `plc4j/tools` has four modules. Capture replay is a pcap4j replayer whose speed arithmetic multiplies the inter-packet delay by the speed, so its timing is not to be copied. The connection cache leases shared connections one at a time and pings idle ones before reuse; it is pure logic. The event pump runs batched reads on a timer, and its subscription trigger throws at start. OPM proxies annotated objects with ByteBuddy and uses read and write only. `plc4j/utils` adds `test-utils` (the driver-testsuite, parser-serializer and pcap round-trip runners, whose XML suites are plain data a port can reuse) and `test-generator` (a Groovy tool that turns a pcap into a suite). The README also names a Scraper, an OPC-UA Server and a PLC4X Server. The Scraper is in neither repository. The two servers are `plc4j/tools/opcua-server` and `plc4j/tools/plc4x-server` in `apache/plc4x-extras`, which also holds the Go tools plc4xbrowser and plc4xpcapanalyzer (`plc4go/tools`). Its tag v1.0.0 is commit ed67ec04. All of that is outside the pinned baseline: S41 located it and did not read it. The `plc4c/tools/plc4x-server` in the checkout is a C socket stub, not the server. Stonecrab's simulator and test kit cover the "testing without a PLC" page. Nothing else is tracked: no document outside one SPEC.md sentence mentions these tools.

**Integrations.** The v1.0.0 README names Calcite, Camel, Kafka-Connect, Karaf and NiFi. At tag v1.0.0, `apache/plc4x-extras` has `plc4j/integrations/apache-calcite`, `apache-kafka` and `apache-nifi` (two NiFi modules) and no Karaf or Camel module. Camel is `components/camel-plc4x` in `apache/camel`, and its default branch uses PLC4X 1.0.0. StreamPipes has PLC connectors in `apache/streampipes` (`streampipes-extensions/streampipes-connectors-plc`), still on PLC4X 0.13.1. IoTDB is only an example application in plc4x-extras (`plc4j/examples/hello-integration-iotdb`), not a connector. Eclipse Ditto has no integration, only a third-party example repository last pushed in 2020. Only directory listings were read, and none of it is in the pinned baseline. SPEC.md requires these to stay "visible in scope reporting", and no document tracks them.

**Code generation.** No MSpec generator exists. SPEC.md asks for it to start with the Modbus constructs.

## Decisions

The maintainer accepted E1, E2, E3, E6 and E7, and the recommendation for E5, on 2026-10-28. E4 is deferred because no hardware can be bought now.

| # | Decision | Outcome (2026-10-28) |
| --- | --- | --- |
| E1 | The commit 1fcc6ae had no `Signed-off-by` | Accepted and done. The commit was amended with `git commit --amend -s` as 554ad24, with an unchanged tree, and force-pushed with a lease. The workflow skips the DCO check after a force-push, so `scripts/dco-check.sh` was run locally first. No commit in this repository carries a GPG signature and no signing key is configured, so "signed" in this plan means signed off |
| E2 | P4, redistributing the Apache-2.0 PLC4X test vectors | Accepted. The maintainer approves redistribution with NOTICE attribution, which unblocks S04a and S04b. S34 records the approval in STATUS.md |
| E3 | When the MSpec generator starts | Accepted: before OPC UA. Phase G runs before M3 and is proven on Modbus first, against the existing hand codec |
| E4 | Test hardware | Deferred: no purchase is possible now, so HW1 and HW2 stay blocked. [Test hardware](#test-hardware) names the device to buy first and the interim slices that raise evidence without hardware |
| E5 | S7 or EtherNet/IP first | Accepted as recommended: decided when a test PLC is available. S41 inventories both |
| E6 | Tools and integrations | Accepted. PLC4X tools count toward parity, starting with the connection cache and Object PLC Mapping. Integrations go in a tracked list and are built on demand |
| E7 | An unknown unit (G06) | Accepted: `unit` becomes `Option<String>` in the next 0.x minor release |
| E8 | Vendoring `modbus.mspec` (2026-10-29) | Approved: verbatim, ASF header kept, NOTICE attribution, under `upstream/mspec/`; unblocks S66 and S67 |
| E9 | The 24-hour soak and 8-hour fuzz run (2026-10-29) | Wait; not part of 0.3.0 |
| E10 | The behavior choices of S59, S60, S69 and S70 (2026-10-29) | Accepted as shipped |
| E11 | TLS library, Object PLC Mapping, FC05, profile schema, Linux I2C (2026-10-29) | As recommended: rustls with ring behind an opt-in `tls` feature and no TLS-PSK; OPM by a declarative macro in this crate; opt-in FC05; schema version 2 with value-matched quality; Linux I2C deferred |
| E12 | Release (2026-10-29) | 0.3.0 after the next run, with G06 (E7) |

## Ordered slices

Sizes follow the alpha plan: S is under a day, M is a few days, L is a week or more. A slice lands only when its exit check passes.

### Phase A: hygiene (now)

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S34 | Reconcile documents with the 0.2.0 publication (H02 to H06); record the decisions above and the P4 approval in STATUS.md | S | none | No document says 0.2.0 is unpublished; no `themadbotter` URL remains; `cargo doc` clean |
| S35 | Make the `semver` job blocking against the published 0.2.0 | S | S34 | Job green and without `continue-on-error` |

E1 is done and P4 is approved. Still the maintainer's: create and push the `v0.2.0` tag on 640665a, P1 to P3, and decide whether to file the drafted upstream issues.

### Phase B: contract hardening (before any new driver)

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S36 | Range validation: `min <= max`, both bounds finite (G01) | S | none | Profile tests for inverted and NaN bounds; fuzz seed added |
| S37 | Bit validation: bound status bits by data width; detect overlap across status fields and `BitField` steps; compare addresses after `parse_address`, not as text (G02, G03) | M | S36 | Tests for each overlap shape and alias spelling |
| S38 | Deny `clippy::indexing_slicing` in `sensors` and fix the fallout (G04) | S | none | `lint-attrs` self-test updated; no new `allow` |
| S39 | Key the word-order rule on the driver's declared multi-word types, not Modbus names (G05) | S | none | Test with a fake second driver's wide type |
| S40 | `Operation::Ping`, `Capabilities.ping` and a provided `Connection::ping` returning `Unsupported`; contract suite cases (G07, ping part) | S | none | Contract suite proves the default is `Unsupported` with no I/O |

G06 lands with E7 in the next breaking release. The subscribe part of G07 landed in S54 (the types and a provided `subscribe` returning `Unsupported`; S55 adds polling) and the browse and discover parts land in M3, each with the types it needs, so no empty scaffold ships (ADR-0001).

### Phase C: finish M0 (before M2)

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S41 | Inventory every protocol, transport and operation in PLC4J at v1.0.0 from source; resolve each PLC4X lead in Findings section 4 | L | none | `upstream/manifest.toml` lists every driver and transport with file and line evidence |
| S42 | Inventory capabilities found only in the Go, C, C# and Python implementations | M | S41 | Backlog table in the manifest |
| S43 | One parity record per protocol and per tool, rows `planned` or `deferred`; a tracked integrations list (E6) | M | S41, S42 | `parity-check` passes; release matrix has no hidden omission |
| S44 | Close the Modbus-specific pending items 5 to 9 of the manifest | M | none | Items removed from `[pending]` with evidence |
| S04a, S04b | Upstream Modbus vectors, adapter and tests | M | E2 | Byte-order rows reach `automated-verified` |

### Phase D: M1 completion, Modbus TCP parity

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S45 | Write PDUs as upstream emits them: FC15 for coils, FC06 for a 2-byte holding-register payload and FC16 otherwise; golden vectors. FC05 only as a recorded divergence | M | S04a | Vectors and negative cases; fuzz target |
| S46 | Write on the wire: `WriteOutcome::Unknown` after transmission, access policy, `Capabilities.write = true` | M | S45 | Contract suite and interop write cases pass |
| S47 | Modbus ping through `ping-address`, with a recorded decision on upstream quirk (d) | S | S40 | Parity row `MB-TCP-PING` implemented |
| S48 | `PlcValue::List` and array selections `[i]`, `[lo..hi]`, `;base` | M | none | `MB-ADDR-SELECTION` implemented |
| S49 | `STRING`, `WSTRING`, `CHAR`, `WCHAR`, and a `PlcValue::Bytes` decision (G08) | M | S48 | String type rows implemented |
| S50 | Extended registers FC20 and FC21 with file-boundary splitting | M | S45 | `MB-ADDR-EXTENDED` implemented |
| S51 | Gap-filling read optimizer, or a recorded divergence; `ZeroBased` and `OneBased40001` conventions | M | none | `MB-READ-GAP` resolved |
| S52 | Criterion benchmarks for parse, serialize and batched reads against a local server; baseline recorded; 10% gate in the scheduled run | M | none | Benchmark report in `docs/evidence/` |
| S52a | A second independent peer: libmodbus 3.1.11 `unit-test-server` in the interop job, from a pinned Debian trixie image (see [Until hardware arrives](#until-hardware-arrives)) | M | none | Interop job green against pymodbus and libmodbus; each special case ends in a typed outcome; image digest and package version recorded |
| S52b | Failure paths on a real clock: Toxiproxy in front of both peers, hung and dead peers, tshark checks on every capture | M | S52a | Each failure ends in a typed error within its deadline; no capture holds a malformed frame |
| S52c | Unit-ID correlation against a scripted multi-unit modbus-tk server | S | none | Wrong-unit and unknown-unit cases pass |
| S52d | HW1 and HW2 as replays: fixtures from the documented maps of the SIO-MB04THMS and the SLM1-C, labeled "documented map, not hardware-verified"; vendor documents and firmware pinned by URL and sha256, not copied; acceptance suites and runbooks that list what the hardware must settle | M | none; write cases after S46 | Fixtures pass their acceptance suites in CI |
| HW1 | First Modbus TCP hardware record, on the SIO-MB04THMS | S | E4, S52d | Record under `docs/hardware/` |

### Phase E: M2 sensor acquisition

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S53 | ADR on calendar values, then the TIME and DATE family for Modbus (G08) | M | none | Calendar rows implemented |
| S54 | `SubscribeRequest`, `Subscription`, `QueuePolicy` and `LossCounters`; provided `subscribe` returning `Unsupported` (G07, G10) | M | none | Contract suite cases for every queue policy |
| S55 | Polling subscriptions: cyclic and change-of-state, event `Unsupported`, missed ticks skipped, interval disclosed; Modbus reports polling | M | S54 | `MB-TCP-SUBSCRIBE` implemented |
| S56 | `StaleTracker` and the acquisition loop honoring `min_interval` and `stale_after` (G09) | M | S55 | Stale transitions tested on a fake clock |
| S57 | Eager background reconnect and `Timeouts.idle` | M | S54 | Shutdown-during-reconnect case passes |
| S58 | Serial transport on a blocking worker, with a virtual serial pair for tests | M | none | Transport integration tests |
| S59 | Modbus RTU: framing, CRC-16, inter-frame timing, bus serialization, shared PDU | L | S58 | SPEC.md's RTU failure cases pass |
| S60 | Modbus ASCII: framing and LRC | M | S59 | Upstream ASCII vectors pass |
| S61 | UDP transport and `modbus-tcp:udp`, against j2mod or diagslave with `tc netem` faults | M | none | `MB-TCP-TRANSPORTS-UDP` implemented |
| S62 | 24-hour soak, memory stability, 8-hour release fuzz and branch coverage reporting | M | S56 | Reports in `docs/evidence/` |
| HW2 | RTU and ASCII on a real RS-485 adapter, on the SLM1-C | S | E4, S59, S52d | Hardware record |

### Phase F: M2S board adapters (parallel once S54 and S56 settle)

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S63 | MAX31855 over SPI: temperature, cold junction, open and short faults | M | S56 | Bus mocks; bench record |
| S64 | ADXL345 over I2C with a sample-block interface carrying interval, axes, sequence and loss | L | S56 | FIFO overflow case; bench record |

### Phase G: code generation (before M3, if E3 is accepted)

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S65 | ADR on the generator: native Rust or bootstrapped from upstream tooling, output layout, provenance header | S | E3 | ADR accepted |
| S66 | MSpec generator for the Modbus constructs, proven equivalent to the hand codec through the vector corpus | L | S65, S04a | Generated codec passes every Modbus vector |
| S67 | `regen` gate in CI: deterministic output and a clean diff | S | S66 | CI job green |

### Phases H to J: M3 to M5

These are milestone-level until S41 gives them source-checked scope. Each is sliced into its own table then.

- **M3, OPC UA.** TLS and TLS-PSK transports, which also close the Modbus TLS rows. Secure channel and session with the upstream security policies. Chunking, read, a browse API with its own types, native subscriptions, staged write and discovery. Trust-failure, credential and session-renewal cases. Interop against an independent server such as open62541.
- **M4, PLC drivers.** S7 with the COTP transport, and EtherNet/IP with Logix, in the order E5 sets. Differential tests against PLC4J and named-device evidence before any hardware claim.
- **M5, baseline parity.** ADS/AMS, SLMP and UMAS first, then the remaining protocols with their transports (CAN, SocketCAN, raw socket, PCAP replay). Tools in the order E6 sets, starting with the connection cache and Object PLC Mapping. The integrations list stays visible in every release matrix.

### Phase K: follow-ups found while executing (added 2026-10-29)

Slices that executing the plan without hardware showed were needed or possible. Each has a
row in the slice log of STATUS.md once it lands. The connection cache and the event pump
are the first tools of decision E6, taken ahead of M5 because they need neither hardware
nor the generator; Object PLC Mapping is not taken, because the derive macro drawn in
`docs/parity/tools.md` needs a proc-macro crate, which decision ADR-0018 (one crate) rules
out until the maintainer decides how it should ship.

| Slice | Work | Size | Depends on | Exit check |
| --- | --- | --- | --- | --- |
| S52e | Replay the documented SOLO SLM1-C map over Modbus RTU | S | S52d, S59 | The replay runs through `ModbusRtuDriver` (landed 2026-10-29) |
| S57b | The flaky zero-idle test and a counter for frames dropped between exchanges | S | S57 | The race closed by a deterministic test (landed 2026-10-29) |
| S59b | RTU against diagslave over a TCP gateway | S | S59, S61 | Job `interop-rtu` green (landed 2026-10-29) |
| S60b | ASCII (and RTU, if it works) against diagslave through a socat pseudo-terminal | S | S60, S59b | An interop job green (landed 2026-10-29) |
| S68 | The connection cache (E6) | M | S47, S55 | `TOOL-CC-*` rows implemented (landed 2026-10-29) |
| S69 | The event pump (E6) | M | S55, S68 | `TOOL-EP-*` rows implemented (landed 2026-10-29) |
| S70 | Raw bytes of a short Modbus reply; Modbus connection statistics | S | S52a, S57b | The two Known-gaps items closed (landed 2026-10-29) |
| S71 | TLS transport and `modbus-tcp:tls` (E11) | M | S19, S61 | `MB-TCP-TRANSPORTS-TLS` implemented; an interop job over TLS (landed 2026-10-29) |
| S72 | Object PLC Mapping through a declarative macro and a trait (E6, E11) | M | S68 | `TOOL-OPM-*` rows implemented or deferred with reasons (landed 2026-10-29) |
| S73 | SOLO follow-ups: opt-in FC05 and profile schema version 2 with value-matched quality (E11) | S | S52e | The SOLO bits writable and its PV sentinels `Bad` in the replay (landed 2026-10-29) |
| S74 | G06: `unit` becomes `Option<String>`, and the version moves to 0.3.0 (E7, E12) | S | none | The semver job passes against 0.2.0 with a 0.3.0 version (landed 2026-10-29) |

## Test hardware

E4 is deferred, so nothing here is bought yet. The devices were researched on 2026-10-28 against vendor documents. Prices are in USD from the vendors' stores that day, before sales tax. Device facts come from the linked documents; "engineering advice" marks what does not. Operating any device, the SigCore UC included, needs the maintainer's separate approval, as STATUS.md requires.

### Buy first: AutomationDirect STRIDE SIO-MB04THMS

A 4-channel isolated thermocouple input module with a Modbus TCP server, for HW1. It costs [$258.00](https://www.automationdirect.com/pn/SIO-MB04THMS), with 17 in stock. With a [PSL-24-030](https://www.automationdirect.com/pn/PSL-24-030) 24 VDC Class 2 supply ($51.00) and a [ProSense THMK-T18L06-01](https://www.automationdirect.com/pn/THMK-T18L06-01) ungrounded type K probe ($50.00), the order comes to $359.00, and US orders over $49 ship free. It is $309.00 if the bench's own probe moves between converters.

Why it fits, from the [user guide](https://cdn.automationdirect.com/static/manuals/stridefio/siomb04thmsug.pdf) (1st edition, rev. C) and the [insert](https://cdn.automationdirect.com/static/manuals/stridefio/siomb04thms.pdf):

- **It reads the bench's sensor.** Each input defaults to type K; J, R, S, T, B, E, N and 250 mV are selectable. Values are int16 with one implied decimal for temperature and two for millivolts.
- **P0 temperature with a fault flag.** Register 40050 sets a bit per channel for a broken sensor, a disconnected cable or over-temperature.
- **The map is public and complete.** FC01 to FC06, FC15 and FC16. A read takes at most 128 coils (FC01, FC02) or 64 registers (FC03, FC04); a write takes at most 64 registers (FC16) or 64 coils (FC15).
- **The firmware version reads over Modbus**, at 40002 to 40003, so the hardware record captures it itself.
- **Safe write targets for S45 and S46.** Input type (40031 to 40034), units (41218 to 41221), offsets (41241 to 41244), and coils 161 to 163 (watchdog enable, watchdog event, power-up event).
- **Failure paths on the device side.** 16 simultaneous Modbus TCP connections, a socket timeout, and a watchdog (10 s by default) that sets a coil when the client goes quiet. INIT mode stops the inputs updating, which is a real stale-value case for S56.
- **No Windows.** Setup and firmware upload run in a browser.

Before relying on it:

- **Firmware.** Advisory [PA-COM-006](https://cdn.automationdirect.com/static/firmware/product_advisory/PA-COM-006.pdf) says firmware before 8.5.4.0 sends the web password back on any login attempt. AutomationDirect lists 8.5.4.0 as the current shipping version; confirm it on arrival. Units from batch B/N 57141862221 cannot take the update and go back under RMA.
- **The web UI is plain HTTP** on port 80, with the default login `admin` / `password`. Change it and keep the module on an isolated bench LAN.
- **int16 only.** It gives no float32, word-order or string evidence; the alternates below cover those.
- **Open items HW1 measures.** The documents do not state the TCP port, the exception codes, the reply to a wrong unit ID, the encoding of the firmware registers, or how coils 161 to 163 mirror register 40011.
- **The MAX31855 cross-check is a plausibility check.** The MAX31855K allows ±2 °C gain and offset error plus ±2 °C cold-junction error ([datasheet](https://cdn-shop.adafruit.com/datasheets/MAX31855.pdf)); the module allows ±0.2% of full scale linearity plus up to ±2 °C cold-junction error. Expect agreement within a few degrees, not tenths, and judge each converter against an ice point.

### Alternates

| Need | Device | Price | Why | Catch |
| --- | --- | --- | --- | --- |
| HW2: Modbus RTU and ASCII | AutomationDirect [SOLO SLM1-C](https://www.automationdirect.com/pn/SLM1-C) with a Waveshare [USB TO RS485 (B)](https://www.waveshare.com/usb-to-rs485-b.htm) | $89.00 + $7.99 | Thermocouple input. Process value at 1000H with fault codes (8003H: sensor not connected); firmware version at 102FH; RTU and ASCII; every serial setting writable over Modbus ([manual](https://cdn.automationdirect.com/static/manuals/soloslm/soloslmman.pdf)) | Ships set to ASCII 7E1, so RTU needs 1072H (protocol) and 1074H (data bits) written first. FC03 and FC16 take at most 8 registers. Ungrounded thermocouples only |
| A second Modbus TCP device, and the SigCore loopback | STRIDE [SIO-MB12CDR](https://www.automationdirect.com/pn/SIO-MB12CDR) | $234.00 | 8 digital inputs, 4 Form C relays and four 32-bit counters at 40039 to 40046, on the same conventions as the SIO-MB04THMS | The counters' word order is undocumented, so it is a test item |
| The cheapest float32 evidence | USR IOT [USR-M100 (ETH)](https://shop.usriot.com/Mobile/Goods/goodsInfo/id/474) | $65.00 with a US adapter | float32 on FC04 from two 4-20 mA inputs, 2 relays, 2 digital inputs, and a TCP-to-RTU gateway for later | Ships from China; freight and duty unknown. The Modbus port is set by hand. The firmware version is not readable over Modbus. Order the M100 (ETH), not the M100s |
| Modbus TCP and OPC UA on one device, for M3 | Acromag [NTE2611-1111](https://www.acromag.com/shop/signal-conditioners-network-i-o/ethernet-io-modules/ethernet-temperature-input/nt2610-ethernet-temperature-input-modules/) | $590.00 | 8 thermocouple inputs and an OPC UA server beside Modbus TCP ([bulletin](https://www.acromag.com/wp-content/uploads/2024/11/BusWorks-NT-Series-Ethernet-IO-Modules-with-OPC-UA-and-MQTT-Support.pdf)); browser setup | The register map and firmware need a free Acromag account. Confirm the download works before buying |

Considered and not chosen: CLICK PLUS with C2-OPCUA and C0-04THM ($696; needs Windows once to program, and the thermocouple module is backordered until 2026-11-10), ICP DAS ET-7018Z/S (price unverified; Windows-only firmware tools), OMEGA DP16PT ($660.58; 56-week lead time) and the Waveshare Modbus POE ETH Relay (B) ($46.99; its Modbus TCP mode is set with a Windows tool).

### The SigCore UC

The maintainer owns a [SigCore UC](https://sigcoreuc.com/): a Raspberry Pi 4 controller with 8 SPDT relays, 8 digital inputs, 4 analog inputs and 4 analog outputs. It cannot be the HW1 device. Its [source](https://github.com/edwardschmitz/sigcore-uc) at release 2.0.26 (9a85bd0, 2026-10-28) shows:

- **Its own protocol, not Modbus.** `software/Server/SigCoreServer/Program.cs` starts only `SigCoreServer`, a proprietary protocol on TCP 7020.
- **The Modbus server never starts.** Nothing constructs `ModbusTcpServer` (port 1502), and the Control Panel's Modbus server is commented out. There is no OPC UA code, although the [Crowd Supply page](https://www.crowdsupply.com/en-z-em/sigcore-uc) and the [Hackaday project](https://hackaday.io/project/203656-sigcore-uc-universal-controller) advertise both.
- **The server is unfinished.** Its handlers would acknowledge coil writes without switching relays, answer every request as unit 1 and assume one whole request per socket read.
- **No license.** The repository has no license file, so none of its code may be copied into Stonecrab.

It can still drive and observe a real device (engineering advice):

- **Fault injection for HW1.** A relay in series with one thermocouple channel opens the loop on command. The matching bit in 40050 must set, then clear. Keep the cross-check channel off the relay, because the contacts add junctions to the loop.
- **A known stimulus.** The analog outputs give 0-5 V from a 12-bit DAC (`A_OUT.cs` scales volts over 5.0 into 4095 counts). Through a divider they give a repeatable, uncalibrated signal for the 250 mV input type.
- **Loopback with the SIO-MB12CDR.** SigCore relays switch 24 V into the STRIDE inputs, for FC02 reads and counter increments. STRIDE relays switch the SigCore's own 5 V into its inputs, which proves an FC05 or FC15 write moved a contact. SigCore inputs are 5 V logic (off up to 0.5 V, on from 3 V), so 24 V must never reach them.
- **Scripting from Linux.** Use its .NET library over TCP 7020, or its HTTP API, which listens only on the controller's loopback (127.0.0.1:8080) and so needs SSH. Its Control Panel is Windows-only and not needed.

If its author finishes and enables the Modbus server, it can be evaluated again as a device.

### Until hardware arrives

S52a to S52d, and the peers chosen for existing slices, raise the evidence without a purchase. None of them makes anything `hardware-verified`.

| Slice | Peer or tool | What it closes |
| --- | --- | --- |
| S52a | [libmodbus](https://github.com/stephane/libmodbus) 3.1.11 `unit-test-server` from Debian trixie (library LGPL-2.1-or-later, tests BSD-3-Clause) | "Interop is one peer". A C stack that shares no code with Stonecrab or pymodbus, with exception 06 at 0x170, a reply with the wrong transaction and unit ID at 0x171, a 500 ms delay at 0x172, a byte-at-a-time reply at 0x173, and a short reply to any 2-register FC03 read |
| S52b | [Toxiproxy](https://github.com/Shopify/toxiproxy) 2.13.0 (MIT), `docker pause` and `docker kill`, tshark | Resets, black holes, latency, sliced frames, and hung and dead peers on a real clock; an independent decode of every capture |
| S52c | [modbus-tk](https://pypi.org/project/modbus-tk/) 1.1.5 (LGPL-2.1-or-later) in a short scripted server | Unit-ID correlation: a separate map per unit and silence for unknown units. After S63, the BeagleBone can serve its MAX31855 readings through it |
| S52d | The documented maps of the SIO-MB04THMS and the SLM1-C | HW1 and HW2 become replays: fixtures, acceptance suites and runbooks are ready before the devices arrive |
| S58 to S60 | A socat PTY pair with libmodbus RTU and modbus-tk `RtuServer` | The virtual-framing half of the RTU evidence. Not HW2: there is no RS-485 electrical layer |
| S61 | [j2mod](https://github.com/steveohara/j2mod) (Apache-2.0) or [diagslave](https://www.modbusdriver.com/diagslave.html) 3.6 (freeware, pinned by sha256), with `tc netem` | A UDP peer, plus loss, delay, duplication and reordering, which Toxiproxy cannot do for UDP |

Two cautions for S52a: the test server serves one connection and then exits, so it runs in a restart loop, and it must never receive FC07, which makes 3.1.11 exit without replying. S52c's server is first-party code, so it supports `interoperability-tested` at most and cannot back a named device profile. No loaner program was found, and AutomationDirect's 45-day return still starts with a purchase.

## Risks

- **The PLC4X comparison is from the website.** S41 may move protocols between milestones. Nothing in Phases H to J is committed until S41 lands.
- **Hardware gates M1 and M2.** Until E4 buys the SIO-MB04THMS and the SLM1-C, both milestones stay at `interoperability-tested` at best. S52a to S52d raise the evidence short of that and turn HW1 and HW2 into replays. The vendor documents leave the TCP port, the exception codes and the unit-ID behavior open, so the S52d fixtures may need corrections after HW1.
- **Hand-porting OPC UA.** Skipping Phase G risks a large unverifiable codec. E3 exists to decide this deliberately.
- **Breaking API changes.** E7, `PlcValue` additions and `subscribe` are 0.x minor bumps. `PlcValue` and `Operation` are already `#[non_exhaustive]`, so new variants are additive for matchers.
