# Stonecrab Engineering Specification

**Draft engineering specification • 26 October 2026**

Stonecrab is an independent, open-source native Rust port of Apache PLC4X. It will provide a common API for industrial protocols, preserve the upstream project's observable capabilities, and prioritize access to useful sensor measurements during staged delivery.

The end state is a Rust implementation of PLC4X's libraries, driver behavior, transports and supporting tooling. Early releases deliver a tested subset; they do not redefine that subset as full parity. Sensor profiles and direct board adapters extend the port without changing its compatibility contract.

This specification defines requirements and acceptance criteria. It does not authorize implementation or publication.

## Product decisions

| Decision | Specification |
| --- | --- |
| Project name | Stonecrab |
| Long-term objective | A native Rust port with explicit, testable parity against a pinned Apache PLC4X baseline |
| First delivery | Shared API, simulator, native Modbus TCP, sensor profiles and automated tests |
| Next priorities | Modbus RTU, then OPC UA; industrial PLC drivers follow |
| Public API | Idiomatic Rust with a documented mapping to PLC4X concepts and behavior |
| Initial use | Sensor acquisition; write capability remains in the full-port scope |
| Open-source license | Proposed Apache-2.0, with upstream attribution and notices retained |
| Deployment | Linux first, including Debian 13 Trixie on the BeagleBone target |
| Initial hardware testing | Existing bench; additional device compatibility remains unverified until tested |

Industrial-first sequencing, API style, license and bench availability are working defaults. Direct sensor adapters have their own early milestone so the existing thermocouple and accelerometer work can contribute without delaying the industrial driver foundation.

The library must be usable independently of Vorpal, Shatterproof, The Mad Botter services, cloud accounts or hosted infrastructure.

## Definition of a straight port

### Compatibility baseline

Before implementation, record a released Apache PLC4X tag and its exact commit in an upstream manifest. Record the relevant protocol definitions, test corpus revisions and reference implementations. The reviewed documentation identifies itself as PLC4X 1.0.0; implementation must resolve and pin the actual source rather than depend on a moving “latest” URL.

Use PLC4J as the primary reference for shared API behavior. For each protocol, identify the upstream implementation with the relevant working capability; retain additional implemented capabilities from other languages in the parity backlog. Language implementations differ, and a protocol definition alone does not establish driver support. [Source: upstream support matrix](https://plc4x.apache.org/plc4x/latest/users/protocols/index.html).

Maintain one parity record per protocol, transport and operation. Each record must identify:

- Upstream source reference and expected behavior.

- Rust implementation and supported configuration.

- Status: planned, implemented, automated-verified, hardware-verified or deferred.

- Test identifiers, applicable device models and firmware.

- Any deliberate behavioral difference and its rationale.

“Full parity” means every implemented in-scope upstream capability at the pinned baseline has an accepted Rust equivalent or an explicitly documented exception. Deferred features remain gaps. Unimplemented upstream placeholders do not become obligations to reproduce missing functionality.

### Porting rules

- Port the protocol logic and shared abstractions into native Rust. A Java service, FFI binding or unrelated client wrapped behind a common trait does not complete a port.

- Preserve wire formats, address interpretation, configuration meaning, data types, response codes and operation semantics. Map Java-style exceptions and futures to Rust errors and asynchronous interfaces.

- Preserve meaningful PLC4X connection-string and tag syntax where applicable. Document aliases or intentional changes; never silently reinterpret an address.

- Reuse upstream protocol descriptions and test vectors where practical. Rust ecosystem dependencies may supply generic transport, cryptography, parsing and runtime facilities.

- Existing Rust protocol libraries may serve as independent test peers or temporary experiments. An adopted implementation requires a parity audit and a documented decision; dependency adoption alone is not evidence of parity.

- Do not reproduce a known upstream defect merely to match it. Add a regression test, document the divergence and reference the protocol standard or upstream issue.

- Runtime and ordinary consumer builds must not require Java. A maintainer-only generation step may initially use upstream Java/Maven tooling; a native Rust generation pipeline belongs in the eventual port.

- Kafka, Camel and other ecosystem integrations are tracked separately. Preserve their useful functions through Rust-appropriate integrations where possible; Java container APIs are not Rust APIs. Any unreproduced integration must remain visible in scope reporting.

“Native Rust port” describes implementation and compatibility, not line-for-line translation or guaranteed binary/API compatibility with another language.

## Sensor priorities

Prioritize reusable measurement categories before individual sensor SKUs. A measurement type does not identify a transport: temperature, for example, may arrive through a register, an OPC UA node or a board-level converter.

The following is a proposed engineering order based on reuse and testability, not a quantified market-share ranking.

| Priority | Measurement category | Initial access route | Required semantics |
| --- | --- | --- | --- |
| P0 | Temperature from thermocouples, RTDs and transmitters | Modbus input/holding registers; then OPC UA nodes | Signed values, scale, unit, sensor faults and invalid readings |
| P0 | Pressure, flow and level | Modbus transmitters or remote I/O; then OPC UA | Scale, offset, range, quality and explicit diagnostic flags |
| P0 | Proximity, limit, photoelectric and machine-state signals | Modbus discrete inputs/coils or gateway registers | Boolean mapping, inversion and optional declared debounce |
| P1 | Current, voltage, power and energy | Modbus meters; then OPC UA | Multi-register byte/word order, signedness, integer precision and counter rollover |
| P1 | Humidity and environmental measurements | Modbus or OPC UA sensor gateways | Units, scaling, valid ranges and error states |
| P1 | Vibration and acceleration summaries | Industrial transmitter values; optional direct accelerometer adapter | Distinguish RMS/peak summaries from raw waveform samples |
| P1 | Counts, pulse-derived speed and runtime | Counter modules or PLC tags | Width, rollover, sampling interval and reset detection |
| P2 | Identification and traceability | Separate RFID/barcode adapter extension | Identifier, observation time, duplicates and device status |

Analog 4–20 mA, 0–10 V, RTD and thermocouple elements require appropriate conversion or transmitter hardware. A generic library profile must not claim direct electrical support.

Support for an IO-Link sensor should initially mean access through a specified IO-Link master and its supported northbound interface. It must not imply that a Linux GPIO or serial driver implements an IO-Link master. IO-Link is a point-to-point sensor/actuator technology. [Source: IO-Link technology](https://io-link.com/technology).

### Direct sensor extension

Provide optional adapter crates for the existing bench after the common sample and error contracts are established:

- **MAX31855 over SPI:** thermocouple temperature, internal cold-junction temperature and the converter's open/short fault indications. Preserve negative readings and fault state. [Source: MAX31855](https://www.analog.com/en/products/max31855.html).

- **ADXL345 over I²C, with SPI as a follow-on:** three-axis acceleration, selected range, output rate and FIFO behavior. Report missing samples and overflow. [Source: ADXL345](https://www.analog.com/en/products/adxl345.html).

Board adapters are extensions to Stonecrab, not upstream PLC4X parity items. They share the measurement envelope and quality model but retain device-specific configuration. ADXL345 initialization includes explicit configuration writes even in an acquisition-only application; distinguish these from a public actuator-write API.

Raw vibration collection requires a separate sample-block interface carrying sample interval, axis layout, sequence and loss information. Do not model a high-rate waveform as a stream of unrelated scalar “latest values,” or promise bandwidth beyond the sensor, bus and host.

## Protocol delivery order

| Stage | Protocol or transport | Initial scope | Completion evidence |
| --- | --- | --- | --- |
| Foundation | Simulated driver and deterministic transport harness | Connect, read, errors, time and scripted failures | The same API contract suite used by real drivers |
| First usable release | Modbus TCP | Coils, discrete inputs, holding/input registers, typed reads and batched acquisition | Codec fixtures, independent server tests, reconnect tests and declared hardware status |
| Sensor expansion | Modbus RTU | Shared Modbus PDU logic, serial framing, CRC, timing and unit addressing | Virtual serial tests plus an actual adapter/device before hardware verification |
| Unified industrial data | OPC UA | Secure sessions, reads, browse and native subscriptions; write parity follows the planned scope | Trust failures, reconnect, chunking and independent server interoperability |
| PLC expansion | EtherNet/IP including Logix, Siemens S7 | Prioritize acquisition operations, then complete each driver's parity matrix | Exact controller, firmware, operation and transport recorded |
| Broader factory support | ADS/AMS, SLMP and remaining upstream industrial drivers | Capabilities identified in the pinned baseline | Protocol-specific parity and hardware evidence |
| Full baseline coverage | Remaining upstream protocols and tools | Complete the tracked upstream inventory | No hidden omissions in the release matrix |

Modbus TCP and RTU must share one PDU implementation. Modbus ASCII remains in the baseline backlog if the selected upstream release supports it.

EtherNet/IP and S7 have equal roadmap priority until an actual deployment or available test PLC determines their order. Do not infer demand from a vendor name alone.

The broader inventory includes upstream-supported building automation, CAN, utility and specialty protocols, alongside the PLC4X proxy driver. The manifest must enumerate the complete baseline and its actual operation support; this delivery table does not replace that inventory.

Write support is staged, not abandoned. A protocol release must state exactly which read, write, subscribe, browse and discovery operations are complete.

## Architecture

Use a Cargo workspace with dependencies directed from applications to API and drivers, and from drivers to transports and codecs. Keep sensor metadata outside wire-level models.

| Component | Responsibility |
| --- | --- |
| stonecrab-core | Values, tags, request/response types, capabilities, errors and driver contracts |
| stonecrab | Public facade, explicit driver registration and feature selection |
| stonecrab-transport | TCP, UDP and serial facilities with deadlines and cancellation; later transports as required |
| stonecrab-codec | Bounded binary primitives and shared parser/serializer support |
| stonecrab-modbus and other driver crates | Native protocol codecs, state machines and address mapping |
| stonecrab-sensors | Declarative sensor profiles, units, scaling and quality normalization |
| Optional sensor adapter crates | MAX31855, ADXL345 and later board-specific devices |
| stonecrab-testkit | Scripted peers, fake clock/transport, golden vectors, differential runner and contract tests |
| Maintainer tooling | Upstream manifest, generation, coverage and release evidence |

Crate names are provisional package identifiers; the project is called Stonecrab.

Use Tokio as the initial asynchronous transport/runtime integration. Avoid blocking serial or device access on runtime worker threads. The core data model must remain independent of Tokio so later runtime integrations do not require changing values or request semantics.

Expose an object-safe driver boundary for applications that choose drivers at runtime. Boxed futures or equivalent dispatch are acceptable at that boundary; keep small codec and value operations straightforward and avoid allocation where measurement shows it matters.

Select drivers through Cargo features. Ordinary builds must include only requested protocols. Dynamic loading, a stable C ABI, embedded no_std support and multiple runtimes are later extensions, not initial requirements.

The library owns connectivity and data interpretation. Persistence, dashboards, alert rules, ERP integration and analytics belong in applications such as Vorpal.

## Public API contract

| Concept | Required behavior |
| --- | --- |
| Driver registry | Deterministic driver lookup; unsupported schemes fail clearly |
| Connection configuration | Typed configuration plus documented PLC4X-compatible string parsing |
| Capabilities | Report operation support accurately, including native versus polling subscriptions |
| Tag or address | Validated protocol address plus requested type; retain a stable caller-supplied identifier |
| Read batch | Return results correlated to original tags, including individual failures |
| Write batch | Return per-item status and distinguish confirmed completion from unknown outcome |
| Subscription | Explicit lifetime, bounded delivery, loss reporting and unsubscribe |
| Browse and discovery | Distinct operations; unsupported operations return an explicit unsupported error |
| Connection lifecycle | Explicit connect/close, observable state and defined cancellation |
| Diagnostic metadata | Protocol error/status retained without exposing secrets |

A batch is not automatically atomic or a simultaneous snapshot. Splitting a batch to meet device limits must preserve result association and disclose sampling-time differences. Requests rejected during local validation must perform no I/O.

### Values and measurements

Preserve Boolean, signed/unsigned integer widths, floating-point values, strings, bytes, arrays and the additional structured/time types required by the baseline. Do not convert every value to f64; large counters and identifiers must remain exact.

The sensor measurement envelope must include tag identifier, typed value or absence, unit when known, quality, origin, local receipt time and optional source timestamp. Preserve the original protocol status alongside normalized quality.

Use a monotonic clock for deadlines and freshness. Do not invent a device timestamp from local receipt time. A configured stale threshold must mark retained values as stale after missed acquisition; it must not silently publish the previous value as a new good reading.

Quality must distinguish at least good, uncertain, bad, stale and unknown. “Good” means the configured checks passed; it is not a claim that an uninstrumented physical sensor has been calibrated. Missing quality information must remain distinguishable from affirmative device quality.

### Errors

Define typed errors for invalid configuration/address, unsupported operation, timeout, cancellation, disconnection, authentication/trust failure, malformed data, device exception, resource limit and backpressure.

Preserve successful items in a partially failed batch. Separate a connection-level failure from a per-tag device exception. After an ambiguous write timeout, expose an unknown outcome; do not automatically replay writes.

## Sensor profile contract

Profiles are versioned data, initially TOML or an equivalently validated declarative format. They map protocol tags to useful measurement names without embedding executable code.

Each profile must declare:

- Profile ID and schema version; manufacturer, model and firmware applicability where known.

- Connection requirements, register/node addresses and address convention.

- Data type, count, byte order and multi-register word order.

- Scaling, offset, units and optional range checks.

- Diagnostic/status fields and explicit quality mapping.

- Minimum recommended acquisition interval and stale threshold.

- Documentary source, fixture provenance and validation status.

Reject ambiguous Modbus “40001” versus zero-based addressing unless the profile explicitly selects a convention. Never infer word order from plausible-looking values.

Keep raw values available for diagnosis. Apply scaling in a declared order, check overflow and non-finite results, and retain integer values when scaling is unnecessary. Profile validation must catch overlapping bit fields, invalid counts and unsupported transformations.

The first profile set must exercise temperature, pressure/flow/level and discrete state. Synthetic reference profiles may ship before real device maps, but must be labeled as synthetic. A named device profile requires a real register map and its own test fixtures.

Document profiles independently from drivers: two devices using Modbus are not necessarily interchangeable.

## Connection and resource behavior

All network operations require bounded deadlines. Drivers must limit frame size, buffered bytes, pending requests and queued events before allocating from untrusted length fields.

Use one explicit owner or synchronized state machine per connection. Respect device concurrency limits. For Modbus RTU, serialize requests on a shared serial bus. Late responses after a timeout must not satisfy a newer request; close, drain or re-synchronize according to the protocol.

Cancellation must have documented wire consequences. If a request may have been transmitted, canceling its future cannot assert that the device did nothing. Release pending request state and preserve stream framing before reusing the connection.

Reconnect must use configurable exponential backoff with a cap and jitter. Read retry must be bounded and individually configurable because some device registers have read side effects. Do not retry writes automatically after ambiguous delivery.

A polling subscription must expose its sampling interval and that it is polling. Skip missed ticks instead of flooding a recovering device. Slow consumers must encounter a declared queue policy: backpressure, latest-value coalescing or dropping with explicit loss counters. Silent loss is prohibited.

Read-only application policy blocks public write operations before transmission. It does not mean passive network capture or absence of traffic. Native protocol handshakes and explicitly enabled sensor initialization remain visible in configuration.

Structured diagnostics must include operation, connection state, latency, retries and failure category. Redact credentials and sensitive payloads by default.

## Code generation and upstream maintenance

PLC4X already separates protocol descriptions, language templates and generated code. Reuse that separation when producing Rust codecs. Generated messages are only one part of a driver; transport and session logic remain separately ported and tested. [Source: PLC4X code generation](https://plc4x.apache.org/plc4x/latest/developers/code-gen/index.html).

Start with the MSpec constructs needed for Modbus, then expand coverage alongside later protocols. Do not gate a useful first release on generating every upstream protocol.

Requirements:

1. Record source revision, generator version and generation options with generated output.

2. Fail generation explicitly on unsupported constructs; never silently emit an incomplete codec.

3. Make generation deterministic. CI regenerates and checks for a clean diff.

4. Keep maintained extensions separate from generated files. Fix the generator or source definition rather than manually patching its output.

5. Validate integer widths, signedness, endianness, length expressions, optional fields, arrays, discriminated types and checksums with independent vectors.

6. Bound recursion and allocations in generated parsers. Return errors for invalid input rather than panicking.

7. Retain upstream test provenance and original fixture files; use a deterministic adapter when Rust needs a different test representation.

8. Maintain an explicit MSpec feature matrix. The eventual native generator must reproduce the accepted Rust output or demonstrate equivalent behavior through the codec corpus.

Upstream updates are reviewed changes to the pinned manifest. Each update produces a parity diff, regression run and documented migration impact; it must not silently move the compatibility baseline.

## Test strategy and release gates

Coverage is a release requirement, not a substitute for interoperability. Every declared capability needs both a successful-path test and meaningful failure cases.

PLC4X's existing parser tests compare bytes to an expected model and serialize back to bytes. Port these fixtures where available, then add independent and adverse-input evidence. A round trip alone can allow a matching encoder and decoder defect to pass. [Source: upstream parser and serializer testing](https://plc4x.apache.org/plc4x/latest/developers/tutorials/testing-serializers-and-parsers.html).

### Test layers

| Layer | Required evidence |
| --- | --- |
| Unit and boundary | Address parsing, values, scaling, endian handling, limits, errors and quality transitions |
| Golden vectors | Known wire bytes and independently expected decoded fields, including malformed inputs |
| Property tests | Round trips where valid, bounded parsing, no invalid lengths, exact integer preservation and safe batching |
| Fuzzing | Untrusted parsers, connection/tag strings and profile loading; preserve all regression seeds |
| Contract suite | Common driver behavior, capability accuracy, correlation, cancellation and unsupported operations |
| Transport integration | Real loopback sockets or virtual serial endpoints with fragmentation, coalescing, truncation and disconnects |
| Differential | Rust and the pinned upstream implementation against the same deterministic peer or fixture |
| Independent interoperability | A separately implemented server/client so both ends do not share the new codec |
| Hardware in the loop | Model/firmware-specific tests on an isolated bench with retained evidence |
| Soak and load | Long-running acquisition, bounded queues, reconnection and resource stability |

Differential comparison must normalize nondeterministic timestamps and request identifiers. Compare semantics when multiple wire encodings are valid. An upstream result is a reference, not an infallible oracle; resolve discrepancies using protocol documentation and device evidence.

### Numeric gates

These are proposed acceptance thresholds, not claims about existing results.

| Gate | Requirement |
| --- | --- |
| Handwritten core and driver line coverage | At least 90% for each released crate, not merely across the workspace |
| Changed handwritten lines | At least 95% covered; narrowly documented exclusions only |
| Handwritten core and driver branch coverage | At least 80% where the pinned coverage tool supports meaningful instrumentation |
| Critical failure paths | Every explicitly listed case below must have a passing test regardless of percentage |
| Generated codec behavior | Every enabled message variant has a valid vector; lengths, discriminators and field boundaries have negative cases |
| Mutation testing | At least 80% of non-equivalent mutations killed in selected address, value, scaling and frame-validation modules |
| Fuzz smoke tests | At least 60 seconds per critical target on relevant PRs |
| Scheduled fuzzing | At least 30 minutes per critical target in each scheduled full run |
| Release fuzzing | At least 8 hours per critical target on the candidate, with no unresolved reproducible crash, hang or unbounded-allocation finding |
| Acquisition soak | 24 hours for a stable driver release under a recorded representative workload |
| Memory stability | No sustained growth after warm-up under a fixed workload and bounded configuration |

Track uncovered branches even if branch instrumentation is unavailable; the mandatory case list still blocks release. Mutation exclusions need a reason and a retained report. Fuzzing duration is an execution floor, not proof that bugs are absent.

Report generated and handwritten coverage separately. Exclude examples, vendored sources and generated files from handwritten thresholds, but never exclude an error path simply to raise a score.

### Mandatory failure cases

- Truncated headers and payloads, oversized lengths, malformed type discriminators, invalid checksums and unexpected function codes.

- TCP fragmentation/coalescing, peer close at each framing boundary, incorrect transaction/unit IDs and delayed replies after timeout.

- Device exception, invalid register/node, unsupported capability and mixed success within a batch.

- Cancellation while connecting, queued, transmitting and awaiting a response.

- Device restart, network outage, recovery, exhausted retry budget and explicit shutdown during reconnect.

- Queue exhaustion, slow consumer, dropped events, stale last-known values and timestamp absence.

- Negative/scaled values, swapped words, integer boundaries, counter rollover, NaN/infinity and incompatible profile version.

- For RTU: CRC errors, timing boundaries, serial disconnect/reopen and wrong unit replies.

- For OPC UA: untrusted/expired certificates, rejected credentials, unsupported security policy, session renewal and subscription recovery.

- For direct sensors: open/short thermocouple flags, I²C/SPI failures, unavailable device, invalid identity and FIFO overflow where applicable.

### Hardware evidence

Use available boards and sensors after confirming they are present and functional. The existing bench is not assumed to contain a Modbus transmitter, RS-485 adapter or commercial PLC.

A hardware test record includes device model, firmware, adapter, wiring/interface configuration, host OS/architecture, library commit, operation set, load and result. Distinguish build-tested, simulator-tested, interoperability-tested and hardware-verified support.

Hardware verification is required before claiming support for a particular device. A simulator-verified experimental release may ship with that status clearly stated. Test both real serial electrical behavior and virtual framing before labeling RTU hardware-verified.

For the direct bench adapters, validate MAX31855 temperature/fault decoding and ADXL345 stationary-axis readings, configuration and disconnect recovery. These tests do not establish calibration accuracy or prove performance as an industrial vibration analyzer.

## Platforms and performance

Target Linux x86_64 first. Exercise aarch64 and armv7 Linux builds, with hardware runs on available devices before claiming runtime support. The BeagleBone deployment target is Debian 13 Trixie; Debian 12 is not the substitute acceptance environment.

Add macOS and Windows for the core and TCP drivers once their tests run on those systems. Qualify serial and board-level adapters separately. Select and pin the Rust minimum supported version at implementation kickoff; test it alongside stable Rust.

Performance acceptance must distinguish library overhead from network latency, sensor conversion time and PLC response time. Do not promise a universal polling rate, sub-millisecond reads or a fixed memory footprint.

Provide reproducible benchmarks for:

- Parsing/serialization time and allocation count by message type and size.

- Single and batched reads against a deterministic local server.

- One-device and multi-device polling, including slow or unavailable peers.

- Subscription queue behavior, loss accounting and tail latency.

- RSS, CPU utilization, reconnect latency and descriptor/task counts during soak tests.

- Separate Modbus-only and OPC UA-enabled builds on the edge target.

Record build profile, compiler, features, hardware, OS and workload. Investigate repeatable regressions above 10% against the same benchmark configuration before release. Resource limits are hard requirements even when no absolute throughput target has been established.

## Delivery milestones

| Milestone | Deliverable | Exit criteria |
| --- | --- | --- |
| M0 Baseline and contracts | Pinned upstream manifest, full capability inventory, API contract and test plan | Scope reconciled against source; all target behaviors linked to a planned test |
| M1 Native Modbus TCP | Core API, simulator, native TCP codec/driver and first sensor profiles | Read contract passes; coverage gates met; adverse transport and independent server tests pass; hardware status disclosed |
| M2 Sensor acquisition | RTU, profile validation, stale/quality handling and bounded polling | Virtual serial and recovery tests pass; real RTU verification recorded when hardware exists |
| M2S Board adapters | Optional MAX31855 and ADXL345 adapters | Bus mocks and recorded bench tests pass; faults/loss preserved; no change to industrial parity status |
| M3 OPC UA | Secure connectivity, read/browse/subscription features and staged write support | Security and lifecycle tests pass against independent servers; claimed capabilities match the parity matrix |
| M4 PLC drivers | EtherNet/IP/Logix and S7, ordered by customer/test access | Per-operation differential tests plus named-device evidence before hardware claims |
| M5 Baseline parity | Remaining protocols, operation gaps, tooling and native generation coverage | All implemented baseline capabilities resolved or listed as explicit exceptions; full-port claims match evidence |

These are capability gates, not calendar commitments. M2S may proceed after M1 contracts settle. An early release can be useful without claiming M5 completion.

For each driver, complete acquisition first, then remaining supported operations. Full write parity, discovery and native subscriptions must remain scheduled work where the baseline implements them. A polling substitute is not native-subscription parity.

A release candidate is complete only when its capability matrix, test reports, examples, known limitations and compatibility notes agree.

## Open-source project requirements

Use Stonecrab as the project name and identify it clearly as an independent Rust port. Preserve upstream attribution and identify adapted files and fixtures. Proposed new project code uses Apache-2.0; dependency choices and redistributed fixtures must carry recorded license/provenance information.

Prepare the repository with README, LICENSE, NOTICE where applicable, CONTRIBUTING, SECURITY, a code of conduct, changelog, architecture decisions, compatibility matrix and a hardware-test reporting template. Define a maintained vulnerability-reporting channel before the first public release.

Contributions adding a driver must include capability metadata, address documentation, fixtures, failure tests and evidence appropriate to its claimed support level. Contributions fixing a bug must include a regression test unless a documented alternative demonstrates the failure.

CI must run formatting, linting, unit/integration/doctests, supported feature combinations, coverage gates and dependency checks. Use hardware-free tests for ordinary pull requests. Run hardware jobs only on controlled runners after review of executable changes.

Feature jobs must include no-default-features, each standalone driver, documented combinations and the supported aggregate. Verify published crate contents and examples. Avoid enabling a network endpoint or hardware discovery merely by importing a crate.

Publish release notes that state the upstream baseline, supported operations, actual hardware evidence and breaking changes. The pre-1.0 API may evolve with migration notes; 1.0 requires a stable core API and an explicit parity statement. A 1.0 version number must not silently imply full upstream coverage.

Publication, repository creation and package-name registration are separate actions from this specification.

## Decisions to revisit

These working defaults can be revised without changing the full-port objective.

| Decision | Current default | What changes it |
| --- | --- | --- |
| First sensor interfaces | Industrial Modbus TCP/RTU, then OPC UA; board adapters in M2S | A requirement to make direct SPI/I²C hardware the first deliverable |
| API resemblance | Idiomatic Rust with behavioral compatibility | A requirement for close structural migration from existing PLC4X code |
| Hardware scope | Existing bench after inventory; no assumed PLC purchases | Confirmed access to a PLC/transmitter, vendor test units or a hardware budget |
| PLC driver order | EtherNet/IP and S7 at equal priority | First target machine, customer requirement or available test rig |
| Baseline | Pin a released tag at M0 | A selected upstream release and exact source revision |
| Resource budgets | Bounded design and measured targets | Defined connection count, sensor rates and host memory/CPU limits |
| License | Apache-2.0 proposed | A different project licensing preference |

The next engineering activity is M0: inventory the upstream behavior and available test hardware, then turn this specification into traceable implementation tasks.

## References

These sources describe upstream behavior and interfaces. Milestones, architecture choices and numeric quality gates above are Stonecrab design requirements.

- [Apache PLC4X](https://plc4x.apache.org/)

- [PLC4X language and protocol support](https://plc4x.apache.org/plc4x/latest/users/protocols/index.html)

- [PLC4X code generation](https://plc4x.apache.org/plc4x/latest/developers/code-gen/index.html)

- [PLC4X serializer and parser tests](https://plc4x.apache.org/plc4x/latest/developers/tutorials/testing-serializers-and-parsers.html)

- [Modbus protocol and serial implementation specifications](https://www.modbus.org/modbus-specifications)

- [IO-Link technology](https://io-link.com/technology)

- [Analog Devices MAX31855](https://www.analog.com/en/products/max31855.html)

- [Analog Devices ADXL345](https://www.analog.com/en/products/adxl345.html)

