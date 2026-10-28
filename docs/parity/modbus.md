# Modbus TCP parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State at the alpha (slice S31): the Modbus TCP read path is implemented end to end. The
address value types (S20a), the address text grammar with its limits (S20b,
`ModbusAddress::parse`), the read batcher (S24a, `Batcher::plan`), the PDU codec (S21,
`ModbusPdu` and its encoders and decoders), MBAP framing (S22, `MbapFramer`, `MbapHeader`,
`ModbusAdu`, `TransactionId`), the typed decoder (S23, `TypedDecoder`), the session (S24b,
`ModbusSession`, `exception_item_error`), the connection (S25, `ModbusConnection`,
`ConnectionDefaults`) and the driver with its configuration (S25b, S26, `ModbusTcpDriver`,
`ModbusOptions`) exist, and the `interop` CI job (S27) runs the driver against pymodbus
3.8.6. No row is `planned` any more; what the alpha does not ship is `deferred` with its
owner. Support levels in the README and `docs/COMPATIBILITY.md` follow from these statuses.

Status vocabulary, as used in the Status column:

- `implemented`: the Rust behavior exists and the listed tests pass. `implemented (data
  only)`, `(primitive)` and `(trait only)` say what the row covers when it is narrower than
  the name suggests.
- `automated-verified`: the behavior is also checked by the `interop` CI job (S27) against
  pymodbus 3.8.6; `MB-TCP-CONNECT` and the four read rows `MB-TCP-READ-COIL`,
  `MB-TCP-READ-DI`, `MB-TCP-READ-HR` and `MB-TCP-READ-IR` carry it. The job ran green on
  the hosted runner for S27 (run 40543567845, recorded in `STATUS.md`). The `MB-INTEROP-*`
  rows are `implemented`: they describe the server's map, which the same job reads back.
- `implemented (not automated-verified: ...)`: the four byte-order mode rows. The
  mapping is derived from the documented layouts and cross-checked against first-party
  vectors and the pymodbus map (big-endian, plus one little-endian REAL). The upstream
  little-endian vectors (`Modbus-all-datatypes-little-endian.xml`) are retained verbatim
  under `fixtures/modbus/upstream/` since S04a, but no adapter or test reads them yet
  (S04b and S23), so the rows stay below `automated-verified`.
- `deferred (<owner>)`: recorded for completeness; the alpha rejects or omits the
  behavior. `; rejection implemented` after it means the parser or driver already
  refuses the form with a typed error and the listed test proves it, and `; no-gap
  planning implemented` means the batcher already plans without the omitted behavior.
- `not reproduced`: an upstream defect, documented and deliberately not ported; `;
  implemented` after it means the Stonecrab handling exists and is tested.

In the "Test ids" column an `implemented` or `automated-verified` row names real tests
(function names, fuzz targets or doctests) that exist in the repository.
`scripts/parity-check.sh` (`mise run parity`, CI job `parity`, ADR-0005 addendum) enforces
three mechanical parts of that: every such row names at least one backticked test id,
every such row id is named in a `# Compatibility` section or a test, and every
`docs/parity/<file>.md#<ROW-ID>` reference in `crates/*/src` resolves to a row here.
Row count at the alpha: 5 `automated-verified`, 73 `implemented`, 26 `deferred` and 12
`not reproduced` rows, plus 22 divergence rows that reuse the ids of the rows they
qualify.

How row ids are used: the first column of every table is a stable id. Rustdoc carries a
`# Compatibility: docs/parity/modbus.md#<row-id>` section on each public item whose
behavior maps to upstream behavior, and test names or `// covers:` tags cite the id they
verify. Ids are never renumbered; a retired row keeps its id and says so.

Scope of the inventory: this record covers the Modbus TCP driver at the pinned commit
(`modbus-tcp`, its read path, address grammar, configuration, data types, byte order,
exceptions and the interop server). Non-Modbus protocols, non-Java implementations and
the inventory of the source-release archive contents against the pinned commit remain open
(`inventory_complete = false` in the manifest; the archive signature itself was verified
in S32) and do not gate the alpha.

## File key
Citations use `Key:line` against the verbatim copies of the pinned files. Paths are
relative to the repository root of apache/plc4x at the pinned commit. `P` abbreviates
`plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus` and `SPI` abbreviates
`plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers`.

| Key | Path |
| --- | --- |
| Drv | `P/tcp/ModbusTcpDriver.java` |
| Cfg | `P/tcp/config/ModbusTcpConfiguration.java` |
| Conn | `P/tcp/ModbusTcpConnection.java` |
| Codec | `P/tcp/ModbusTcpMessageCodec.java` |
| Disc | `P/tcp/discovery/ModbusPlcDiscoverer.java` |
| AC | `SPI/model/AddressConstraints.java` |
| MTT | `plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusTagTest.java` |
| MET | `plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusEncodeTest.java` |
| Tag, TagCoil, TagDI, TagHR, TagIR, TagExt | `P/base/tag/ModbusTag.java`, `ModbusTagCoil.java`, `ModbusTagDiscreteInput.java`, `ModbusTagHoldingRegister.java`, `ModbusTagInputRegister.java`, `ModbusTagExtendedRegister.java` |
| TagH | `P/base/tag/ModbusTagHandler.java` |
| Opt | `P/base/optimizer/ModbusReadOptimizer.java` |
| RegCodec | `P/base/ModbusRegisterCodec.java` |
| BO | `P/types/ModbusByteOrder.java` |
| DB, CB | `SPI/DriverBase.java`, `SPI/ConnectionBase.java` |
| TCP | `SPI/tags/TagConfigParser.java` |
| ANP | `SPI/model/ArrayNotationParser.java` |
| Poll | `plc4j/utils/subscription-emulation/src/main/java/org/apache/plc4x/java/utils/subscriptionemulation/PollingSubscriptionConnectionBase.java` |
| Mspec | `protocols/modbus/src/main/resources/protocols/modbus/modbus.mspec` |
| Srv, Dkr | `plc4j/drivers/modbus/src/test/resources/modbus/modbus_server.py`, `.../modbus/Dockerfile` |
| DockerIT | `plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/ModbusDockerIT.java` |
| DTS, DTSO, MF, ADT, ADTLE, PST | `protocols/modbus/src/test/resources/protocols/modbus/tcp/` `DriverTestsuite.xml`, `DriverTestsuiteOptimized.xml`, `ManualFactoryModbusTCPDriverTest-testsuite.xml`, `Modbus-all-datatypes.xml`, `Modbus-all-datatypes-little-endian.xml`, `ParserSerializerTestsuite.xml` |

References to "V1.1b" mean the Modbus Application Protocol Specification V1.1b, the
document `modbus.mspec` itself cites (`Mspec:20`). The PDF is not retained in this
repository; section numbers below are from the published table of contents and are
listed under UNVERIFIED until a copy is checked in.

## Operations
Test ids are Rust test function names. Where a plan decision fixes the behavior it is
named (D3, D14, D16, D18, D19 in `docs/ALPHA_PLAN.md`). The three contract-suite
fixtures (FakeTransport responder S25b, loopback ScriptedPeer S26, pymodbus S27) and the
`interop` job are the evidence the plan requires before a read row can be
`automated-verified`; the four read rows and `MB-TCP-CONNECT` have it.

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-TCP-CONNECT | Protocol code `modbus-tcp`, default transport `tcp`, port 502 (Drv:44-46, 72-92). `connect()` runs `onConnect`: builds the message codec, starts the receive loop, fires CONNECTED (Conn:77-93, CB:120-123). `close()` stops receiving, fails pending requests with "Connection closed", fires DISCONNECTED (Conn:100-112). Transport loss fails pending requests with "Connection lost: <cause>" (or "Connection closed by remote" when there is no cause, Conn:120-121) and fires CONNECTION_LOST (Conn:115-128). No reconnect or retry logic exists in Conn, CB or DB (grep for reconnect and retry finds only the state-event names). | `registry.connect("modbus-tcp://host:502?default-unit-identifier=1")` returns a `dyn Connection`; connect is bounded by `Timeouts.connect`, close by `Timeouts.request` (D2). `ConnectionState` is observable through `StateWatch`. A transport failure sets `Reconnecting`; the next operation waits out the capped jittered backoff and reopens (D15). Request-driven reconnect has no upstream equivalent, so it is a Stonecrab extension recorded here, not a parity claim. Pending requests fail `Disconnected` and are never replayed. | automated-verified (job `interop`, S27: pymodbus 3.8.6 in Docker, with the `modbus_tcp_read` example exiting 0; also over loopback S26 and the fake transport S25b; suite runs: S25b over the fake transport, S26 over the loopback `ScriptedPeer`, S27 against pymodbus) | `connect_returns_connected_state` (implemented, S26, loopback), `connect_refused_yields_disconnected_error` (implemented, S26, loopback), `close_fails_pending_read_with_disconnected` (implemented, S26, loopback), `a_holding_register_read_over_loopback_returns_the_v1_1b_values` (implemented, S26, loopback), `an_exception_reply_over_loopback_is_a_per_item_device_error` (implemented, S26, loopback), `read_after_peer_close_observes_reconnecting_then_connected_and_succeeds`, `pending_read_fails_disconnected_and_is_not_replayed_after_reconnect`, `close_during_backoff_yields_closed`, `parse_config_reads_host_and_port_with_default_502` (implemented, S25b), `connect_passes_the_driver_defaults_clock_and_factory_to_the_connection` (implemented, S25b), `the_suite_is_green_against_modbus_tcp_over_the_fake_transport` (implemented, S25b), `the_suite_is_green_against_modbus_tcp_over_a_scripted_peer` (implemented, S26), `interop_contract_suite_passes_against_pymodbus` (implemented, S27, job `interop`: connect, close and reads through `dyn Driver`), `interop_reads_all_fifteen_holding_register_types` (implemented, S27: connect and close against pymodbus) |
| MB-TCP-READ-COIL | FC01 `ModbusPDUReadCoilsRequest(address, quantity)` (Conn:405-406; Mspec:96-103). Response bits LSB-first per byte (Opt:149-155). Only BOOL accepted when splitting; any other type returns UNSUPPORTED (Opt:132-139). | `coil:N`, `0x`-prefixed short forms and bare `0`-prefixed short forms only with the explicit plc4x convention (MB-ADDR-BARE-40001) read one BOOL per tag through FC01. A merged chunk is one FC01 request; bits are unpacked LSB-first. `Tag::count()` other than 1 is rejected before I/O (S25). | automated-verified (job `interop`, S27; suite runs: S25b over the fake transport, S26 over the loopback `ScriptedPeer`, S27 against pymodbus) | `decode_compare_every_accepted_vector_matches_its_expected_file` (implemented), `re_encode_compare_every_accepted_expected_file_encodes_to_the_vector_bytes` (implemented), `encode_request_writes_function_start_and_quantity_big_endian` (implemented), `decode_compare_bit_responses_unpack_to_the_hand_written_bits` (implemented), `each_area_uses_its_function_code_and_bits_unpack_lsb_first` (implemented, S25), `interop_reads_coils_1_to_10` (implemented, S27, job `interop`), `interop_125_register_and_2000_coil_reads` (implemented, S27, job `interop`), `typed_coil_address_is_rejected_at_parse_time`, `a_count_above_one_is_invalid_config_and_nothing_is_sent` (implemented, S25) |
| MB-TCP-READ-DI | FC02 `ModbusPDUReadDiscreteInputsRequest` (Conn:403-404; Mspec:87-94). Same bit extraction and BOOL-only rule as coils (Opt:128-157). | `discrete-input:N`, `1x` and bare `1` short forms (bare only with the explicit plc4x convention, MB-ADDR-BARE-40001) read one BOOL per tag through FC02. | automated-verified (job `interop`, S27; suite runs: S25b over the fake transport, S26 over the loopback `ScriptedPeer`, S27 against pymodbus) | `decode_compare_every_accepted_vector_matches_its_expected_file` (implemented), `re_encode_compare_every_accepted_expected_file_encodes_to_the_vector_bytes` (implemented), `encode_request_writes_function_start_and_quantity_big_endian` (implemented), `decode_compare_bit_responses_unpack_to_the_hand_written_bits` (implemented), `each_area_uses_its_function_code_and_bits_unpack_lsb_first` (implemented, S25), `interop_reads_discrete_inputs_1_to_10` (implemented, S27, job `interop`), `typed_discrete_input_address_is_rejected_at_parse_time` |
| MB-TCP-READ-HR | FC03 `ModbusPDUReadHoldingRegistersRequest(address, max(lengthWords, 1))` (Conn:409-410; Mspec:135-142). Register blocks split by byte offset `(tag.address - block.address) * 2` (Opt:159-163), byte-swap for the two `*_BYTE_SWAP` orders (Opt:165-168), decode with the tag's order or the connection default (Opt:165-187). | `holding-register:N`, `4x` and bare `4` short forms (bare only with the explicit plc4x convention, MB-ADDR-BARE-40001) read one of the 15 numeric/bit types per tag through FC03 (one element per tag). Adjacent or overlapping tags in the same unit merge into one request (D19); decoding follows the byte-order rows below. | automated-verified (job `interop`, S27; suite runs: S25b over the fake transport, S26 over the loopback `ScriptedPeer`, S27 against pymodbus) | `decode_compare_every_accepted_vector_matches_its_expected_file` (implemented), `re_encode_compare_every_accepted_expected_file_encodes_to_the_vector_bytes` (implemented), `encode_request_writes_function_start_and_quantity_big_endian` (implemented), `a_read_decodes_the_v11b_example_in_one_chunk_with_the_exact_request_bytes` (implemented, S25), `each_area_uses_its_function_code_and_bits_unpack_lsb_first` (implemented, S25), `interop_reads_all_fifteen_holding_register_types` (implemented, S27, job `interop`), `interop_125_register_and_2000_coil_reads` (implemented, S27, job `interop`), `interop_split_batch_discloses_chunks` (implemented, S27, job `interop`), `interop_little_endian_region_decodes_real` (implemented, S27, job `interop`), `interop_bare_40001_reads_with_explicit_convention` (implemented, S27, job `interop`) |
| MB-TCP-READ-IR | FC04 `ModbusPDUReadInputRegistersRequest` (Conn:407-408; Mspec:126-133). Same split and decode as holding registers (Opt:158-189). | `input-register:N`, `3x` and bare `3` short forms (bare only with the explicit plc4x convention, MB-ADDR-BARE-40001) read the same 15 types through FC04. | automated-verified (job `interop`, S27; suite runs: S25b over the fake transport, S26 over the loopback `ScriptedPeer`, S27 against pymodbus) | `decode_compare_every_accepted_vector_matches_its_expected_file` (implemented), `re_encode_compare_every_accepted_expected_file_encodes_to_the_vector_bytes` (implemented), `encode_request_writes_function_start_and_quantity_big_endian` (implemented), `each_area_uses_its_function_code_and_bits_unpack_lsb_first` (implemented, S25), `interop_reads_input_registers_1_to_7` (implemented, S27, job `interop`), `interop_read_beyond_map_yields_device_exception` (implemented, S27, job `interop`) |
| MB-TCP-WRITE | One PDU per tag, chained sequentially (Conn:318-338). Coil: always FC15 (Conn:438-448). Holding register: FC06 when the encoded payload is exactly 2 bytes, else FC16 (Conn:449-465). Extended register: FC21 (Conn:467-487). Discrete input and input register: "Unsupported write tag type". | `Capabilities.write = false`; `write` returns `Unsupported(Operation::Write)` with no I/O (D3). Write PDUs, FC06-versus-FC16 selection and `WriteOutcome::Unknown` on the wire land after the alpha. | deferred (M1 completion, first post-alpha slices) | `write_is_unsupported_with_nothing_sent_and_the_capabilities_say_so` (implemented, S25: the rejection), `capabilities_readme_table_matches_value` (implemented, S26: write is `not supported`) |
| MB-TCP-SUBSCRIBE | Emulated by polling: `PollingSubscriptionConnectionBase.onSubscribe` (Poll:102), CYCLIC and CHANGE_OF_STATE only, EVENT throws UnsupportedOperationException (Poll:338-343), default interval 1000 ms (Poll:385-387), change detection by `Objects.equals` (Poll:371-377). | `SubscriptionMode::Unsupported`; no subscribe method ships (D4). | deferred (M2) | `capabilities_readme_table_matches_value` (implemented, S26: subscribe is `Unsupported`), `the_driver_capabilities_equal_the_connection_capabilities` (implemented, S25b) |
| MB-TCP-BROWSE | `ModbusTagHandler.parseQuery` throws "This driver doesn't support browsing" (TagH:44-47); default `onBrowse` fails "Browse not supported" (CB:461-463). | `Capabilities.browse = false`. Upstream has no Modbus browse, so there is nothing to port. | deferred (none for Modbus; browse arrives with M3 for other protocols) | `capabilities_readme_table_matches_value` (implemented, S26: browse is false) |
| MB-TCP-DISCOVER | `canDiscover()` true (Drv:99-102); `ModbusPlcDiscoverer` runs a pcap4j ARP scan plus "localhost" (Disc:115-128), opens a plain socket to port 502 (Disc:140), probes unit ids 1..247 with FC01 address 1 quantity 1 and a 100 ms wait (Disc:148-150, 169), accepts a normal reply or ILLEGAL_DATA_ADDRESS (Disc:212), validates the MBAP length to at most 260 (Disc:66, 77-85). | `Capabilities.discover = false`. | deferred (M3 or the parity backlog) | `capabilities_readme_table_matches_value` (implemented, S26: discover is false) |
| MB-TCP-PING | `canPing()` true (Drv:104-107). `onPing` parses `ping-address`, sends its read PDU and returns OK on any reply, including an exception PDU (Conn:198-211). | No ping at alpha: `Capabilities.ping` is false and `Connection::ping` is the provided method that returns `Unsupported(Operation::Ping)` (S40; contract case `an unsupported ping performs no I/O and changes no state`); the Modbus override is S47. The `ping-address` key is validated and recorded but unused (S26). | deferred (M3 or the parity backlog) | `ping_address_is_validated_but_unused` (implemented, S26), contract case `an unsupported ping performs no I/O and changes no state` (implemented, S40) |
| MB-CONTRACT-SUITE | None inventoried; upstream has no common driver suite. | The common driver contract suite (`stonecrab::testkit::ContractSuite`, 24 cases) runs through `&dyn Driver` against `ModbusTcpDriver::new().with_clock(fake clock).with_transport_factory(fake factory)` with the connection string `modbus-tcp://127.0.0.1:5020` (S25b, disconnect hook S25c). The factory serves a fresh `FakeTransport` for every connect, each running one `Step::React` whose reactor is a first-party MBAP responder (`tests/support/responder.rs`, built from this module's `decode_request`, `encode_response` and `MbapHeader`) over `fixtures/modbus/interop/register-map.toml`: it serves function codes 01 to 04, answers a span outside the block with exception 02 and an unknown function with exception 01, echoes the transaction and unit ids, and counts the requests it received (`device_reads`). The fixture's readable address is `holding-register:13:UINT` (42424), its rejected address `holding-register:60000:UINT` (parses, outside the block), its foreign address the testkit `FixtureAddress`, and its I/O probe the frames sent plus the opens across every transport the factory created. `inject_timeout` makes the responder swallow the next request (the fake clock drives the deadline), `inject_device_exception` makes it answer the next request with exception 04, and `inject_disconnect` makes it close the peer on the next request without a reply (`Reaction::Close`, S25c). Result: 22 passed, 2 skipped by name. Two cases are skipped because the driver reports `write` false (`a read-only policy rejects a write with no I/O`, `an unknown write outcome is never replayed`, reason `driver reports write unsupported`). The two disconnect cases (`a disconnect during a read yields Disconnected`, `a disconnect carries diagnostics naming operation, state and category`) pass; S25b had skipped them because a `Step::Respond` closure cannot close a fake transport. `a_peer_that_closes_yields_disconnected_with_diagnostics_through_the_driver` additionally checks the lost link byte for byte through the same driver with a script that closes the peer. `an unsupported write performs no I/O` runs and passes (the fixture names a writable address). The ScriptedPeer run (S26, `tests/modbus_contract_scripted_peer.rs`) uses the same driver, the same register map and the same responder over real loopback sockets: `ModbusTcpDriver::new()` keeps its `SystemClock` and, through `with_transport_factory`, the testkit's `ScriptedPeerFactory`, which spawns a fresh `ScriptedPeer` (one `Step::React` over the responder) for every transport and connects to it with the real `TcpFactory` (a `ScriptedPeer` accepts one connection and the suite connects once per case; the factory only redirects the endpoint). Result: 19 passed, 5 skipped by name. The fixture's clock is real time, so it has no `inject_timeout` hook and three timing cases are skipped with reason `fixture cannot inject timeout`: `an absent deadline uses the connection timeout`, `a timeout carries diagnostics naming operation, state and category` and `read retry stops after the configured maximum attempts` (timing belongs to the fake clock, D21). The two write cases are skipped as in the fake-transport run. The disconnect cases pass (`Reaction::Close` half-closes the loopback socket and the driver reconnects to a new peer), as do the device-exception and the no-I/O cases; the no-I/O probe is the opens plus the requests the device counted. The pymodbus run (S27, `interop_contract_suite_passes_against_pymodbus`, job `interop`) is the third fixture: `ModbusTcpDriver::new()` over the real `TcpFactory` wrapped in the testkit's `CountingFactory`, whose opens plus frames sent are the `io_probe` (so the "no I/O" cases are exact over a real socket) and whose frames sent are `device_reads`, against the first-party pymodbus 3.8.6 server serving the register map, with `holding-register:13:UINT` readable, `holding-register:60000:UINT` rejected (exception 02) and the testkit `FixtureAddress` foreign. A stock pymodbus cannot drop a link, swallow a reply or refuse on demand, so the fixture has no fault hooks. Result: 15 passed, 9 skipped by name, 0 failed, written to `contract-report.txt` in the `interop-evidence` artifact: `an absent deadline uses the connection timeout`, `a timeout carries diagnostics naming operation, state and category` and `read retry stops after the configured maximum attempts` (`fixture cannot inject timeout`); `a disconnect during a read yields Disconnected` and `a disconnect carries diagnostics naming operation, state and category` (`fixture cannot inject disconnect`); `a device exception carries diagnostics naming operation, state and category` and `read retry skips non-retryable kinds` (`fixture cannot inject device exception`); the two write cases (`driver reports write unsupported`). Timing over a real clock stays unverified (D21). | implemented; two cases skipped by name over the fake transport, five over loopback, nine against pymodbus | `the_suite_is_green_against_modbus_tcp_over_the_fake_transport`, `the_suite_leaves_plans_unused_and_no_fault_armed`, `the_suite_reports_a_connection_string_the_driver_rejects_as_failures`, `the_fixture_address_reads_the_documented_value`, `the_four_areas_serve_the_map_values_and_an_out_of_range_span_is_refused`, `a_swallowed_request_times_out_at_the_request_timeout_on_the_fake_clock`, `a_refused_request_is_a_device_exception_with_code_four`, `a_peer_that_closes_yields_disconnected_with_diagnostics_through_the_driver`, `the_responder_echoes_ids_and_refuses_what_it_does_not_serve`, `each_fault_maps_to_its_reaction_and_is_used_once`, `the_suite_is_green_against_modbus_tcp_over_a_scripted_peer` (implemented, S26), `the_suite_reports_a_connection_string_the_driver_rejects_as_failures` (S26 copy over loopback), `interop_contract_suite_passes_against_pymodbus` (implemented, S27) |

## Capabilities
Upstream reports capability through two independent signals that disagree for subscribe.

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-TCP-CAPABILITIES | Signal 1, driver flags: `DriverBase` declares `canDiscover`, `canBrowse`, `canPing`, `canRead`, `canWrite`, `canSubscribe`, all false by default (DB:117-139); `ModbusTcpDriver` overrides discover, ping, read and write to true and leaves browse and subscribe false (Drv:99-117). Only `canDiscover` reaches `PlcDriverMetadata` (DB:299-302). Signal 2, connection metadata: `ConnectionBase.deriveMetadata` reports read, write, subscribe and browse true when the connection class or an intermediate base declares the matching `on*` hook (CB:263-296, 303-313; the base-class walk is documented at CB:252-254). `ModbusTcpConnection` overrides `onPing`, `onRead`, `onWrite` (Conn:199, 218, 315) and extends `PollingSubscriptionConnectionBase`, which declares `onSubscribe` (Poll:102), so the connection reports read true, write true, subscribe true (inference from the two verified mechanisms: no run of `getMetadata()` was observed), browse false, while the driver flag `canSubscribe` stays false. | One explicit `Capabilities` value, built from `Default` by field assignment: read true, write false, subscribe `SubscriptionMode::Unsupported`, browse false, discover false, `max_pdu_bytes = Some(253)`, concurrency 1. `ModbusConnection::capabilities()` returns exactly this value since S25; `ModbusTcpDriver::capabilities()` reports it unchanged since S25b (both are built by one function). Differences from upstream: write false (D3, deferred), subscribe Unsupported where upstream connection metadata says true through polling emulation (deferred, M2), discover false (deferred), ping absent (deferred). Concurrency 1 matches `getMaxConcurrentRequests() == 1` (Conn:140-143). | implemented; the README table test is implemented (S26) | `the_driver_capabilities_equal_the_connection_capabilities` (implemented, S25b), `write_is_unsupported_with_nothing_sent_and_the_capabilities_say_so` (implemented, S25), `capabilities_readme_table_matches_value` (implemented, S26) |

## Configuration
Six parameters on `ModbusTcpConfiguration`. Stonecrab carries all six with the upstream
defaults (S26). The pinned test suite `MF` passes `unit-identifier` (MF:30-33), which is not
a declared parameter; the real key is `default-unit-identifier`. See quirk f.

| Row id | Upstream name, type, default (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 5000; "Default timeout for all types of requests." (Cfg:31-34). Applied as `orTimeout` on every pending request (Conn:183-189). | `request-timeout-ms` positive integer, default 5000, mapped onto `Timeouts.request` and the per-chunk deadline `min(request deadline, now + timeouts.request)` (`ModbusTcpDriver::parse_config` parses the key since S26; the connection applies `Timeouts.request` since S25, and an unrepresentable value is `InvalidConfig` before any I/O). Zero, negative, empty and non-numeric values are `InvalidConfig`. | implemented | `the_chunk_deadline_is_the_smaller_of_the_request_deadline_and_the_request_timeout` (implemented, S25), `every_chunk_gets_its_own_request_timeout` (implemented, S25), `a_request_timeout_the_clock_cannot_represent_is_invalid_config_before_any_io` (implemented, S25), `config_request_timeout_default_is_5000` (implemented, S26), `config_request_timeout_rejects_zero_negative_and_non_numeric` (implemented, S26; also asserts `Timeouts.request`), `read_times_out_after_request_timeout_on_fake_clock` |
| MB-CFG-UNIT-ID | `default-unit-identifier`, int, 1; "Unit-identifier or slave-id that identifies the target PLC" (Cfg:36-39). Cast to short at use (Conn:394-400). No range check is visible in the inventoried files. | `default-unit-identifier`, default 1, bounded 0..=255 (`UnitId`, one MBAP byte, Mspec:54); outside the range is `InvalidConfig` (divergence). The address-block key `unit-id` is already bounded the same way by `ModbusAddress::parse` (S20b; `unit_id_256_is_rejected`, `negative_unit_id_is_rejected`, `unit_id_block_sets_unit`); the connection option is parsed since S26. Since S25 `ConnectionDefaults.unit_id` (default 1) is the unit of a chunk whose addresses name none: `the_unit_comes_from_the_address_else_the_connection_default` (implemented, S25). Stonecrab decision: 0 and 255 are accepted and no special meaning is attached to either (their conventional roles are UNVERIFIED, see below). | implemented | `config_unit_id_default_is_1` (implemented, S26), `config_unit_id_rejects_256_and_negative` (implemented, S26), `config_unit_id_accepts_0_and_255` (implemented, S26), `the_parsed_unit_identifier_addresses_the_request` (implemented, S26: the parsed unit is the one on the wire) |
| MB-CFG-PING-ADDRESS | `ping-address`, String, `4x00001:BOOL`; "Simple address, that the driver will use to check, if the connection to a given device is active" (Cfg:41-44). | Parsed with the address grammar under the plc4x convention, recorded, unused at alpha (ping is deferred). The default parses; an invalid address is `InvalidConfig`, and the message does not repeat it. The recorded address is `ModbusOptions::ping_address`; no ping operation exists, so connecting sends nothing (`ping_address_is_validated_but_unused`). | implemented | `config_ping_address_default_parses` (implemented, S26), `config_ping_address_invalid_is_rejected` (implemented, S26) |
| MB-CFG-BYTE-ORDER | `default-payload-byte-order`, enum `ModbusByteOrder`, `BIG_ENDIAN`; allowed `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP`; since 0.13.0 (Cfg:46-55; BO:21-34). | Same four names, default `BIG_ENDIAN`, mapped to `ByteOrder` x `WordOrder` (see Byte order). An unknown name is `InvalidConfig`. The address-block key `byte-order` already takes the same four names in `ModbusAddress::parse` (S20b; `byte_order_names_map_per_adr_0006`, `unknown_byte_order_name_is_rejected`); the connection option is parsed since S26, through the same name table (`byte_order_by_name`). Since S25 `ConnectionDefaults.byte_order` and `word_order` (default big-endian, high word first) are the fallback of a tag and address that name none: `tag_field_beats_address_block_beats_the_connection_default` (implemented, S25). | implemented | `config_byte_order_default_is_big_endian` (implemented, S26), `config_byte_order_accepts_four_upstream_names` (implemented, S26), `the_parsed_byte_order_decodes_the_reply` (implemented, S26: BIG_ENDIAN and LITTLE_ENDIAN decode the same registers differently), `config_byte_order_unknown_name_is_rejected` (implemented, S26) |
| MB-CFG-MAX-COILS | `max-coils-per-request`, int, 2000; since 0.13.0 (Cfg:57-61). Bound for merged coil blocks (Opt:242, 259). | Default 2000, feeds `Batcher::plan`; positive; above 2000 is `InvalidConfig` because the PDU cannot carry more (V1.1b section 6.1). | implemented | `config_max_coils_default_is_2000` (implemented, S26), `config_max_coils_rejects_zero_and_above_2000` (implemented, S26), `batcher_splits_at_max_coils` (implemented, S24a), `the_parsed_coil_limit_splits_the_read_into_chunks` (implemented, S26: the parsed limit reaches the connection's `BatchLimits`) |
| MB-CFG-MAX-REGISTERS | `max-registers-per-request`, int, 125; since 0.13.0 (Cfg:63-67). Bound for merged register blocks (Opt:295, 310). | Default 125, feeds `Batcher::plan`; positive; above 125 is `InvalidConfig` (V1.1b section 6.3). | implemented | `config_max_registers_default_is_125` (implemented, S26), `config_max_registers_rejects_zero_and_above_125` (implemented, S26), `batcher_splits_63_dint_tags_into_two_chunks` (implemented, S24a), `the_parsed_register_limit_splits_the_read_into_chunks` (implemented, S26), `connect_refuses_request_limits_above_the_protocol_maxima` (implemented, S26: hand-built defaults) |
| MB-CFG-UNKNOWN | Unknown parameters are reported, not rejected: `warnAboutUnknownParameters` runs after the configuration factory, which ignores what it has no field for (DB:206-209, 315-319). The reporter class `UnknownParameterReporter` is not among the retained copies, so the reporter itself is UNVERIFIED; the Javadoc says it logs a warning (DB:306-308). `allow-unsupported-transport=true` bypasses only the supported-transport check (DB:177-190). | Unknown keys are rejected with `InvalidConfig` (D12, S26). `allow-unsupported-transport` is an unknown key and is rejected. Divergence row below. | implemented | `config_unknown_key_is_rejected` (implemented, S26), `config_misspelt_unit_identifier_is_rejected` (implemented, S26), `config_duplicate_key_is_rejected` (implemented, S26; refused by the core grammar before the driver sees the key) |

## Connection string and transports
Upstream URI grammar is `DriverBase.URI_PATTERN` (DB:76-77); Stonecrab's parser is row
`CORE-URI-GRAMMAR` in `docs/parity/core.md`. Transport parameters upstream are prefixed
with the transport code (DB:149-209).

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-TCP-TRANSPORTS | Default transport `tcp`, supported `tcp`, `tls`, `tls-psk`, `udp`, `test` (Drv:72-80). Default ports from the mspec constants: tcp 502, tls and tls-psk 802, udp 502, other codes none (Drv:82-92; Mspec:24-28). A transport outside the supported list is refused unless `allow-unsupported-transport=true` (DB:172-190). | Scheme `modbus-tcp`, transport `tcp` only (implicit when omitted), host and port with default 502. Any other transport code is `InvalidConfig`. | implemented | `driver_accepts_tcp_transport_and_defaults_port_502` (implemented, S26), `driver_rejects_other_transport_codes` (implemented, S26), `driver_rejects_port_zero_and_65536` (implemented, S26), `parse_config_reads_host_and_port_with_default_502` (implemented, S25b), `parse_config_accepts_bracketed_ipv6_with_and_without_a_port` (implemented, S25b), `parse_config_refuses_a_bad_authority` (implemented, S25b), `a_bad_authority_is_refused_with_and_without_parameters` (implemented, S26), `prop_parse_config_never_panics` (implemented, S26), `fuzz_modbus_connection_string` (S26: target and 12 seed files; the 10 s smoke run is clean) |
| MB-TCP-TRANSPORTS-TLS | `modbus-tcp:tls://host:port?tls.verify=false` form exercised at DockerIT:649-693; default port 802 (Drv:86-87). | Rejected with `InvalidConfig`. | deferred (M3 or the parity backlog); rejection implemented | `driver_rejects_other_transport_codes` (implemented, S26) |
| MB-TCP-TRANSPORTS-TLS-PSK | Port 802 (Drv:86-87); transport configuration class `ModbusTcpPskTlsTransportConfiguration` (Drv:64-65). | Rejected with `InvalidConfig`. | deferred (M3 or the parity backlog); rejection implemented | `driver_rejects_other_transport_codes` (implemented, S26) |
| MB-TCP-TRANSPORTS-UDP | Port 502 (Drv:88-89); `modbus-tcp:udp://` exercised at DockerIT:592-636. The configuration-class selection tests `transport instanceof UDPTransport` against the BouncyCastle import (Drv:34, 66-67), see quirk a. | Rejected with `InvalidConfig`. | deferred (M3 or the parity backlog); rejection implemented | `driver_rejects_other_transport_codes` (implemented, S26) |
| MB-TCP-TRANSPORTS-TEST | `test` transport used by the driver test runners (Drv:79). Internals not read; UNVERIFIED. | Rejected with `InvalidConfig`; Stonecrab tests use `FakeTransport` instead. | deferred (no release); rejection implemented | `driver_rejects_other_transport_codes` (implemented, S26) |

## Address grammar
Upstream grammar at the pinned commit is the unified array notation
`{area}:{address}[selection]:{TYPE}{config}`, not the older `:INT[4]` form (that form is
rejected with a migration hint, ANP:130-138; `ModbusLegacyAddressTest` covers it). The
five area patterns share one shape: long form `Tag:47`, short forms built from
`FIXED_DIGIT_MODBUS_PATTERN` (Tag:48), config suffix `TCP:32`. Every area class tries the
long form, then the `Nx` form, then the bare form (TagCoil:65-80, TagHR:65-79).

| Area | Long form | `Nx` prefix | Bare prefix | Default type | Protocol offset | File:line |
| --- | --- | --- | --- | --- | --- | --- |
| coil | `coil:` | `0x` | `0` | BOOL | minus one | TagCoil:31-34, 84, 100 |
| discrete-input | `discrete-input:` | `1x` | `1` | BOOL | minus one | TagDI:31-34, 100 |
| holding-register | `holding-register:` | `4x` | `4` | INT | minus one | TagHR:31-34, 83, 88 |
| input-register | `input-register:` | `3x` | `3` | INT | minus one | TagIR:31-34, 88 |
| extended-register | `extended-register:` | `6x` | `6` | INT | none (logical = wire; 0 rejected) | TagExt:31-34, 85, 90 |

There is no `2x` area. The offset constant is `PROTOCOL_ADDRESS_OFFSET = 1` (Tag:50);
the constructor rejects a logical address of zero or less (Tag:195-197); extended
registers use the number as written (TagExt:54-58, 84-85), so "zero-based" for them means
logical equals wire address, not that address 0 is valid: the shared constructor rejects
logical address 0, so extended address 0 cannot be addressed upstream (Tag:195-197;
MTT:131 expects `extended-register:1` to give logical 1 and wire 1). The prefixes begin with
distinct characters, so the area is unambiguous once a form matches.

Slice S20a added the address value types these rows describe in
`crates/stonecrab/src/modbus/address.rs` (`ModbusArea`, `ModbusDataType`, `UnitId`,
`ModbusAddress` and its builder) and slice S20b adds the text parser
`ModbusAddress::parse` (end of `crates/stonecrab/src/modbus/address.rs`; tests in
`crates/stonecrab/tests/modbus_parse.rs` and `tests/modbus_properties.rs`, fuzz target
`fuzz_modbus_address`). The parser builds every address through the S20a builder, so the
bit-area type rule (`bit_areas_accept_bool_only`, `int_on_a_coil_is_rejected`;
MB-ADDR-BIT-TYPES) and the span rule (`span_check_table_at_the_top_of_the_map`;
MB-LIMIT-SPAN) stay in one place. The open items S20a left for S20b are answered:

- The single element at wire address 65535 is reachable through the parser
  (`last_valid_register_is_accepted`: `holding-register:65536` is wire offset 65535), so
  "address above 65535" means a logical address above 65536 (a wire offset above 65535;
  `address_above_65535_is_rejected`). Upstream's inferred refusal of that element is a
  documented deviation, still marked INFERENCE because it was not observed by a run.
- `4x:INT` and the other empty-digit short forms are a typed `InvalidAddress`
  (`four_x_without_digits_is_rejected`); the `NumberFormatException` consequence upstream
  stays marked INFERENCE.
- `canonical()` renders the long form `holding-register:16:INT`, not the upstream
  `4x00001:INT` rendering; no short rendering is offered because the long form is the
  spelling accepted under every option, and `prop_canonical_round_trips` checks that it
  parses back to an equal address (`canonical_renders_the_long_form`).
- Unit identifiers 0 and 255 are carried unchanged
  (`unit_id_zero_and_255_are_documented_not_rejected`, `unit_id_block_sets_unit`) and
  byte order takes part in equality (`addresses_differing_only_in_byte_order_are_not_equal`;
  MB-QUIRK-L).

Grammar decisions S20b records that the verified rows do not fix, each a Stonecrab
choice or an inference rather than observed upstream behavior:

- Inside the config block, spaces around the keys, colons, values and commas are
  ignored; whitespace before the brace is rejected. The pattern text of `KEY_VALUE_PATTERN`
  (TCP:34) is not reproduced in this record, so the whitespace tolerance of the upstream
  regex was not verified; the canonical rendering never needs it.
- `byte-order` takes single- or double-quoted text, the two quote styles the upstream value
  pattern lists; a bare word is an error, where upstream (INFERENCE) silently drops the
  pair. `unit-id` takes a bare decimal integer only; a quoted or fractional value is
  rejected.
- A repeated key and a trailing or leading comma are errors; an empty block `{}` is
  accepted and sets nothing.
- Leading zeros are accepted in digit groups (`coil:000000001` is logical 1), as the
  upstream digit pattern allows.
- Digits are ASCII only, the area and type names are exactly as written (no case
  folding), and the whole text may be at most 256 bytes with no control characters.
- A convention name other than `plc4x` is rejected for every spelling, including the
  unambiguous ones, so a misspelt option cannot go unnoticed (MB-ADDR-BARE-40001).
- An address parsed with no convention and one parsed with `plc4x` both carry
  `AddressConvention::Plc4xCompatible`; the convention only decides what text is
  accepted.

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-ADDR-LONG | `coil`, `discrete-input`, `holding-register`, `input-register` followed by `:` and 1 to 9 decimal digits (`\d{1,9}`, Tag:47), optional `:TYPE` with `[a-zA-Z_]+` resolved by case-sensitive `ModbusDataType.valueOf` (TagCoil:100). Digits are decimal; wire address is logical minus one. Default type BOOL for bit areas, INT for register areas (Tag:202). Example `holding-register:16:INT` (wire 15). | Accepted whether or not a convention was supplied. Same offset and defaults. Rejected with `InvalidAddress`: logical 0, hexadecimal digits, lowercase or unknown type names, more than 9 digits, empty text, text over 256 bytes or with a control character (Stonecrab bounds), and any text no form matches. Messages echo at most 32 control-free characters of the input. | implemented | `holding_register_16_int_is_wire_offset_15`, `holding_register_0x0010_is_rejected`, `lowercase_i16_is_rejected`, `logical_address_zero_is_rejected`, `default_types_are_bool_for_bits_and_int_for_registers`, `every_accepted_type_name_parses_on_a_register_area`, `malformed_shapes_are_rejected`, `empty_input_is_rejected`, `text_length_boundary_is_exact`, `control_characters_are_rejected_and_never_echoed`, `echoed_input_is_bounded_and_control_stripped`, `every_invalid_area_discriminator_is_rejected`, `canonical_renders_the_long_form`, `prop_parse_never_panics`, `prop_canonical_round_trips` |
| MB-ADDR-SHORT-4X | Prefix forms `0x`, `1x`, `3x`, `4x` followed by 4 or 5 digits (the group is optional in the regex, see MB-QUIRK-G) (TagCoil:34, TagHR:34, TagIR:34, TagDI:34). `4x00016:INT` is logical 16. `0x0010` is decimal coil 10, not hexadecimal. Canonical rendering is prefix plus `%05d` (Tag:162). `6x` is the extended-register member of this family. | Accepted unconditionally for `0x`, `1x`, `3x`, `4x`; `6x` is rejected (see MB-ADDR-EXTENDED). `canonical()` renders the long form `holding-register:1:INT` (S20a); short forms are input only, a documented divergence from the upstream `%05d` rendering. | implemented | `four_x_00016_is_logical_16`, `zero_x_0010_is_a_decimal_coil_address_not_hex`, `short_forms_require_four_or_five_digits`, `four_x_without_digits_is_rejected`, `six_x_prefix_is_rejected_at_parse_time`, `canonical_renders_the_long_form`, `prop_short_and_long_forms_agree` |
| MB-ADDR-BARE-40001 | Bare forms: area digit plus 4 or 5 digits, e.g. `40001`, `400001`, `00001`, `000001`, `10001`, `30001`, `60001` (TagHR:33, TagCoil:33). Accepted unconditionally; the ambiguity between a bare number and a 1-based or 0-based register is not detected. | Accepted only when the connection or call supplied `address-convention=plc4x` (`AddressConvention::Plc4xCompatible`); otherwise `InvalidAddress` naming the ambiguity and the two accepted spellings (D16, S20b). Divergence row below. The interop read test with an explicit convention, `interop_bare_40001_reads_with_explicit_convention` (S27), refuses the bare form without the convention and reads it against pymodbus with it. | implemented | `bare_40001_is_rejected_unless_convention_is_explicit`, `bare_40001_with_plc4x_convention_is_logical_1`, `unknown_convention_name_is_rejected`, `prop_short_and_long_forms_agree`, `interop_bare_40001_reads_with_explicit_convention` (implemented, S27) |
| MB-ADDR-BIT-TYPES | Non-BOOL types on `coil` and `discrete-input` parse (TagCoil:100 resolves any type name) and fail at read time with UNSUPPORTED (Opt:132-139). | Rejected at parse time with `InvalidAddress` naming the area and type (D16). Divergence row below. | implemented | `typed_coil_address_is_rejected_at_parse_time`, `typed_discrete_input_address_is_rejected_at_parse_time`, `bit_areas_accept_bool_only`, `int_on_a_coil_is_rejected` |
| MB-ADDR-CONFIG-BLOCK | Trailing `{key: value, ...}`; the regex is `(\{(?<config>.*?)})?$` (TCP:32), pairs by `KEY_VALUE_PATTERN` (TCP:34): `-?\d+.\d+`, `-?\d+`, double- or single-quoted text, `true`, `false`. `unit-id` is read with `Short.parseShort`, `byte-order` with `ModbusByteOrder.valueOf` (Tag:222-229). Example `holding-register:8[0..7]:SINT{unit-id: 10}` (MET:43). INFERENCE from the value regex: an enum value must be quoted (`{byte-order: 'LITTLE_ENDIAN'}`); a bare word matches no alternative and, because the parser uses `find()` (TCP:44), the pair is silently dropped (not observed by a run). `4x:INT` also matches: the digit group is optional (Tag:48), so INFERENCE: `Integer.parseInt(null)` throws `NumberFormatException` instead of a typed invalid-tag error (not observed by a run). | Accepts `{unit-id: N, byte-order: 'NAME'}` with the same four names (single or double quotes), keys in any order, optional spaces inside the braces (a Stonecrab choice) and an empty block. `unit-id` outside 0..=255, unknown keys, unquoted enum values and `4x:INT` are `InvalidAddress` (typed, never a panic). Unit id groups chunks (D19). | implemented | `unit_id_block_sets_unit`, `unit_id_256_is_rejected`, `negative_unit_id_is_rejected`, `byte_order_names_map_per_adr_0006`, `byte_order_block_requires_quoted_name`, `unknown_byte_order_name_is_rejected`, `unknown_config_key_is_rejected`, `malformed_config_block_is_rejected`, `both_keys_combine_in_any_order`, `four_x_without_digits_is_rejected`, `prop_parse_never_panics` |

Deferred or rejected grammar, recorded so the rejection has an owner:

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-ADDR-SELECTION | `[index]` selects one element and yields a scalar, `[lo..hi]` an inclusive range and a list, `;base` sets the lower bound (ANP:54-67, 181-226; Tag:56-89). Single dimension only (`AddressConstraints.SINGLE_DIMENSION`, AC:42-43). Offsets are element counts converted to register offsets, an odd byte offset is rejected (Tag:114-123). Example `holding-register:1[0..1]:REAL` (DTS:133). | Rejected with `InvalidAddress` naming the deferred feature; `PlcValue::List` does not exist at alpha. | deferred (M2); rejection implemented | `selection_range_is_rejected_at_parse_time`, `selection_index_is_rejected_at_parse_time` |
| MB-ADDR-STRING | `STRING(n)` and `WSTRING(n)` carry the length of one string; the length is mandatory for those and forbidden for other types (Tag:206-221). Pattern `\((?<stringLength>\d{1,5})\)` (Tag:47). Example `holding-register:1[0..2]:STRING(20)`. | `STRING`, `WSTRING`, `CHAR`, `WCHAR` and any parenthesised length are rejected at parse time. | deferred (M2); rejection implemented | `string_type_is_rejected_at_parse_time`, `parenthesised_length_is_rejected`, `wstring_type_is_rejected_at_parse_time`, `char_type_is_rejected_at_parse_time`, `wchar_type_is_rejected_at_parse_time` |
| MB-ADDR-EXTENDED | `extended-register`, `6x`, bare `6`; logical equals wire address, but logical 0 is rejected (Tag:195-197); FC20 read, FC21 write split at 10000-register file boundaries (TagExt:54-58; Conn:411-432, 467-487). Limit 655359999 (TagExt:36). | Rejected at parse time. | deferred (M2); rejection implemented | `extended_register_is_rejected_at_parse_time`, `six_x_prefix_is_rejected_at_parse_time` |
| MB-ADDR-LEGACY-ARRAY | Legacy `address:TYPE[n]` and `address:TYPE:n` are rejected; the error names the replacement, e.g. `holding-register:1:INT[4]` becomes `holding-register:1[0..3]:INT` (ANP:130-138; ModbusLegacyAddressTest). | Rejected with `InvalidAddress`; the message carries a migration hint of the same shape, computed from the text (`holding-register:1:INT[4]` gives `holding-register:1[0..3]:INT`; a count it cannot compute gives `[0..N-1]`). The exact upstream wording (ANP:130-138) is not reproduced in this record, so the hint text is Stonecrab's. | implemented | `legacy_array_form_is_rejected_with_hint` |

The original project brief's `holding:0x0010:i16` example is not upstream syntax: it fails
both `MB-ADDR-LONG` (area name, hexadecimal digits, lowercase type) and is the reason the
three tests `holding_register_0x0010_is_rejected`, `lowercase_i16_is_rejected` and
`zero_x_0010_is_a_decimal_coil_address_not_hex` are implemented in `tests/modbus_parse.rs`.

## Limits
Parse-time limits upstream are enforced by `of()` in each area class. Stonecrab enforces the
address bounds in `ModbusAddress::parse` (S20b) and the quantity bounds in `Batcher::plan` and
S25 request validation (single-element addresses cannot reach them).

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-LIMIT-ADDR | Wire address above 65535 is rejected (TagCoil:36, 85-87; TagDI:36, 85; TagHR:36, 84-86; TagIR:36, 84-85). | A logical address above 65536 (a wire offset above 65535) is `InvalidAddress`; address 0 is `InvalidAddress`. Short and bare forms can write up to 99999, so the check is in the parser as well as the `u16` offset. | implemented | `address_above_65535_is_rejected`, `logical_address_zero_is_rejected` |
| MB-LIMIT-SPAN | `address + quantity > 65535` rejected for bit areas (TagCoil:92-94, TagDI:92); `address + registers > 65535` for register areas (TagHR:100-102, TagIR:100-101). INFERENCE: because the test is `>` on the zero-based address plus the count, the single element at wire address 65535 is rejected upstream. Not observed by a run. | `InvalidAddress` when the span of the data type passes the end of the address map (reachable for a 4-register type near the top). S20a decision, confirmed by the S20b parser (`holding-register:65536` is accepted): the single element at wire address 65535 is reachable (span <= 65536), a documented deviation. | implemented | `span_past_65535_is_rejected`, `lreal_at_top_of_map_is_rejected`, `last_valid_register_is_accepted`, `span_check_table_at_the_top_of_the_map`, `request_span_may_end_at_the_end_of_the_address_space` (implemented, S21: the PDU codec applies the same end-of-map rule to `start + quantity`), `prop_request_round_trips_exactly_when_it_is_legal` (implemented, S21) |
| MB-LIMIT-COILS | Quantity above 2000 rejected at parse time (TagCoil:96-98, TagDI:96); configurable block bound 2000 (Cfg:57-61). V1.1b section 6.1: FC01 quantity 1..2000. | Enforced by `Batcher::plan` (S24a): a bit-area chunk never exceeds `BatchLimits::max_coils`, which cannot be configured above 2000. The PDU codec (S21) enforces the same bound on requests and on response byte counts (2000 bits are at most 250 bytes) before any allocation. The connection plans every read through `Batcher::plan` (S25): 2000 coils are one chunk and the 2001st splits (`coils_beyond_2000_split_into_a_second_chunk`), and a request over the connection's own `max_items_per_request` is `ResourceLimit` before any I/O (`a_request_over_the_connections_item_limit_is_a_resource_limit_with_nothing_sent`). Against pymodbus, `interop_125_register_and_2000_coil_reads` (S27) reads 2000 coils and 125 registers as one request each and 2001 coils as 2000 plus 1. | implemented | `batcher_splits_at_max_coils`, `coils_beyond_2000_split_into_a_second_chunk` (implemented, S25), `a_request_over_the_connections_item_limit_is_a_resource_limit_with_nothing_sent` (implemented, S25), `interop_125_register_and_2000_coil_reads` (implemented, S27, job `interop`), `limits_above_the_protocol_maximum_are_rejected`, `prop_plan_never_exceeds_limits_and_loses_no_tag`, `request_quantity_bounds_1_to_125_and_2000` (implemented, S21), `response_byte_count_bounds_are_1_to_250` (implemented, S21) |
| MB-LIMIT-REGISTERS | Registers above 125 rejected, measured in register units: `registers = max(1, (quantity * bytesPerElement + 1) / 2)` (Tag:99-137; TagHR:99-106; TagIR:104). 63 DINT is 126 registers and is rejected (Tag:128-131). V1.1b section 6.3: FC03 quantity 1..125. | Enforced by `Batcher::plan` (S24a) in register units, a `DINT` counting two; the 63-DINT case is reproduced as a multi-tag chunk test (two chunks of 124 and 2 registers). A single tag wider than the configured bound is `ResourceLimit`. The PDU codec (S21) enforces 125 registers on requests and a response byte count of at most 250 before any allocation. Against pymodbus, `interop_125_register_and_2000_coil_reads` and `interop_split_batch_discloses_chunks` (S27) read 125 registers as one request and 126 as 125 plus 1. | implemented | `batcher_splits_63_dint_tags_into_two_chunks`, `a_63_dint_span_splits_at_125_registers` (implemented, S25: through the connection, with the wire quantities 124 and 2), `exactly_125_registers_fit_one_chunk_and_the_126th_splits` (implemented, S25), `batcher_splits_at_max_registers`, `a_tag_wider_than_the_register_bound_is_a_resource_limit_error`, `prop_plan_never_exceeds_limits_and_loses_no_tag`, `request_quantity_bounds_1_to_125_and_2000` (implemented, S21), `prop_register_count_bounds` (implemented, S21), `interop_125_register_and_2000_coil_reads` (implemented, S27, job `interop`) |
| MB-LIMIT-DIGITS | Long-form digit group is 1 to 9 digits and `STRING(n)` length 1 to 5 digits, so overlong numbers there fail the regex as an invalid tag, not as `NumberFormatException` (Tag:47-48; MTT:139-157). Selection indices are not bounded by the regex (`\d+`, ANP:54); `ArrayNotationParser.parseIndex` catches the `NumberFormatException` from `Integer.parseInt` and throws `PlcInvalidTagException` (ANP:265-273; MTT:141-145). | More than 9 digits in the long form, and a number of digits other than 4 or 5 in a short form, is `InvalidAddress` (decided before any numeric conversion, so no overflow); never a panic. The message does not echo the digits. | implemented | `overlong_digit_group_is_rejected`, `short_forms_require_four_or_five_digits`, `prop_parse_never_panics`, `fuzz_modbus_address` |
| MB-LIMIT-PDU | No explicit PDU size check found in the inventoried read path; sizes follow from the quantity limits. V1.1b section 4.1: PDU at most 253 bytes. | PDU at most 253 bytes, enforced by the PDU codec (S21): longer input is `OversizedLength` before it is parsed, and a response byte count is at most 250. `Capabilities.max_pdu_bytes = Some(253)` is set by the driver (S26). | implemented | `pdu_above_253_bytes_is_rejected`, `request_quantity_bounds_1_to_125_and_2000`, `response_byte_count_bounds_are_1_to_250` |
| MB-LIMIT-ADU | `ModbusTcpMessageCodec` sizes a frame as 6 plus the big-endian length field with no upper bound seen (Codec:37-54; `MessageCodecBase` not retained, so a cap there is UNVERIFIED). The discoverer alone bounds a reply to 260 bytes (Disc:66, 77-85). | MBAP length field 2..=254, ADU at most 260, protocol id 0, header validated before the body is allocated or awaited (S22, `MbapFramer::decode`, `MbapHeader::decode`): protocol id other than 0 and length 0 or 1 are `FrameError::Malformed` (the protocol id is judged first), length above 254 is `FrameError::Oversized` carrying the declared ADU size, an incomplete ADU is `Ok(None)` with the buffer untouched. `ModbusAdu::new` refuses a PDU of 0 or more than 253 bytes. Upstream's unbounded length is a divergence candidate; see Divergence. | implemented | `mbap_length_255_is_rejected`, `mbap_length_1_is_rejected`, `mbap_length_0_is_rejected`, `mbap_protocol_id_1_is_rejected`, `mbap_truncated_header_waits`, `mbap_header_is_judged_before_the_body_is_awaited`, `mbap_length_254_with_the_full_body_is_accepted_at_260_bytes`, `adu_constructor_validates_the_pdu_length`, `every_must_fail_vector_fails_or_waits_as_its_expected_file_says`, `fuzz_modbus_mbap` (all implemented, S22) |
| MB-LIMIT-EXT | Extended-register address limit 655359999 (TagExt:36, 86-88, 102-104). | Not applicable; extended registers are rejected at parse time. | deferred (M2); rejection implemented | `extended_register_is_rejected_at_parse_time` |

## Data types
`ModbusDataType` is generated from `Mspec:380-408` (name, numeric value, `dataTypeSize` in
bytes). `DataItem` (`Mspec:318-378`) encodes exactly one value at its natural width with
integers unsigned-binary or two's-complement and floats IEEE754 (`Mspec:318`); register
alignment is the driver's job (`Mspec:313-317`, RegCodec:33-41). A lone value narrower
than a register is padded to one register: BOOL by 15 bits, BYTE, SINT and USINT by 8,
CHAR not at all (RegCodec:135-151); several values pack back to back with only the final
register padded (RegCodec:78-84, 196-205). Width in bits per type: RegCodec:162-194.
The generated Java (`ModbusDataType`, `DataItem`) is not in the repository (generated
from the mspec at build time), so sizes come from the mspec text only.

Registers are 16-bit units. "Registers" below is the number of registers one element of
the type occupies when read alone.

Status (S23): the 15 numeric and bit types are `implemented` by `TypedDecoder`
(`crates/stonecrab/src/modbus/decode.rs`). A register count other than the type's size is
a `MalformedData` item error carrying the received bytes (ADR-0006 decision 5). The
interop run (S27) adds `interop_reads_all_fifteen_holding_register_types` to every row; the
rows do not wait for it, because the `register_map_decodes_every_entry_to_the_row_value`
test already decodes the register words of the interop map for every row's type.

| Row id | Type, mspec enum line, `dataTypeSize` | Registers and DataItem encoding (`Mspec` line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- | --- |
| MB-TYPE-BOOL | BOOL, 381, 2 | 1; `simple bit` (320-322); lone value padded by 15 bits | Accepted on all four areas. A bit area yields the bit; on a register area a lone BOOL is bit 0 of the register under `BigEndian` and bit 8 under `LittleEndian` (the other bits are padding, ignored on decode, written as zero). | implemented | `decode_bool_register_value_1_is_true`, `decode_bool_coil_bit`, `a_lone_bool_is_bit_0_under_big_and_bit_8_under_little`, `padding_bits_are_ignored_when_decoding`, `the_interop_coils_and_discrete_inputs_decode_bit_by_bit`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-BYTE | BYTE, 382, 2 | 1; `uint 8` (323-325); padded by 8 bits | Accepted on register areas; rejected on bit areas. A lone BYTE is the low byte of the register under `BigEndian`, the high byte under `LittleEndian`; padding ignored on decode. | implemented | `decode_byte_42_from_register`, `a_lone_eight_bit_value_is_the_low_byte_under_big_and_the_high_byte_under_little`, `padding_bits_are_ignored_when_decoding`, `a_non_bool_type_on_a_bit_area_is_rejected`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-WORD | WORD, 383, 2 | 1; `uint 16` (326-328) | As above. | implemented | `decode_word_42424`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-DWORD | DWORD, 384, 4 | 2; `uint 32` (329-331) | As above. | implemented | `decode_dword_4242442424_big_endian`, `decode_dword_layout_2143`, `decode_dword_layout_3412`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-LWORD | LWORD, 385, 8 | 4; `uint 64` (332-334) | As above; u64 range exact. | implemented | `decode_lword_u64_exact`, `decode_lword_layout_21436587`, `decode_lword_layout_78563412`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-SINT | SINT, 386, 2 | 1; `int 8` (335-337); padded by 8 bits | As above; negative values. The interop map stores SINT -42 sign-extended as `0xFFD6`; it decodes to -42 because the padding byte is ignored. | implemented | `decode_sint_minus_42`, `a_lone_eight_bit_value_is_the_low_byte_under_big_and_the_high_byte_under_little`, `negative_values_round_trip_under_every_mode`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-INT | INT, 387, 2 | 1; `int 16` (338-340); default type of register areas | As above; i16 boundaries. | implemented | `decode_int_minus_2424`, `decode_int_boundaries`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-DINT | DINT, 388, 4 | 2; `int 32` (341-343) | As above. | implemented | `decode_dint_minus_242442424`, `negative_values_round_trip_under_every_mode`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-LINT | LINT, 389, 8 | 4; `int 64` (344-346) | As above; i64 boundaries. | implemented | `decode_lint_i64_boundaries`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-USINT | USINT, 390, 2 | 1; `uint 8` (347-349); padded by 8 bits | As above. | implemented | `decode_usint_42`, `a_lone_eight_bit_value_is_the_low_byte_under_big_and_the_high_byte_under_little`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-UINT | UINT, 391, 2 | 1; `uint 16` (350-352) | As above. | implemented | `decode_uint_42424`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-UDINT | UDINT, 392, 4 | 2; `uint 32` (353-355) | As above. | implemented | `decode_udint_4242442424`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-ULINT | ULINT, 393, 8 | 4; `uint 64` (356-358) | As above; u64 maximum exact. | implemented | `decode_ulint_u64_max_exact`, `prop_encode_decode_exact_integers`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-REAL | REAL, 394, 4 | 2; `float 32` (359-361) | As above; NaN payloads, infinities and the sign of zero preserved bit for bit as `F32`. | implemented | `decode_real_3_141593_big_endian`, `decode_real_nan_and_infinity_preserved`, `decode_real_little_endian_region`, `signed_zero_is_preserved`, `prop_encode_decode_floats_bitwise`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-LREAL | LREAL, 395, 8 | 4; `float 64` (362-364) | As above; preserved bit for bit as `F64`. | implemented | `decode_lreal_2_71828182846`, `decode_lreal_nan_and_infinity_preserved`, `signed_zero_is_preserved`, `prop_encode_decode_floats_bitwise`, `typed_vectors_decode_to_the_expected_values`, `typed_vectors_encode_to_the_register_bytes`, `register_map_decodes_every_entry_to_the_row_value` |
| MB-TYPE-CHAR | CHAR, 404, 1 | a lone CHAR is unpadded (RegCodec:135-139); a packed run is padded to a whole register (RegCodec:196-205); `string 8` UTF8 (365-367) | Rejected at parse time. | deferred (M2); rejection implemented | `char_type_is_rejected_at_parse_time` |
| MB-TYPE-WCHAR | WCHAR, 405, 2 | 1; `string 16` UTF16BE (368-370) | Rejected at parse time. | deferred (M2); rejection implemented | `wchar_type_is_rejected_at_parse_time` |
| MB-TYPE-STRING | STRING, 406, 1 per character | `ceil(n/2)` registers for `STRING(n)`; `vstring stringLength * 8` UTF8 (371-373) | Rejected at parse time. | deferred (M2); rejection implemented | `string_type_is_rejected_at_parse_time` |
| MB-TYPE-WSTRING | WSTRING, 407, 2 per character | `n` registers for `WSTRING(n)`; `vstring stringLength * 16` UTF16BE (374-376) | Rejected at parse time. | deferred (M2); rejection implemented | `wstring_type_is_rejected_at_parse_time` |
| MB-TYPE-TIME | TIME, 396, 8 | No DataItem case (the `typeSwitch` at 319-377 ends at WSTRING). `widthBits` throws "Unsupported Modbus data type" (RegCodec:191-192). The name still resolves: `getPlcValueType` maps by `PlcValueType.valueOf` (Tag:279-282). | Rejected at parse time as an unsupported calendar type. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-LTIME | LTIME, 397, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-DATE | DATE, 398, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-LDATE | LDATE, 399, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-TIME-OF-DAY | TIME_OF_DAY, 400, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-LTIME-OF-DAY | LTIME_OF_DAY, 401, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-DATE-AND-TIME | DATE_AND_TIME, 402, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-TYPE-LDATE-AND-TIME | LDATE_AND_TIME, 403, 8 | As TIME. | As TIME. | deferred (ADR before the first calendar-value consumer); rejection implemented | `time_family_names_are_rejected_at_parse_time` |

What the generated `DataItem.staticParse` does for the TIME family at runtime is
UNVERIFIED (the generated class is not retained); only the `widthBits` throw is observed.

## Byte order
Upstream has one setting, `ModbusByteOrder`, with four modes (BO:21-34). Its documented
layouts for the on-wire bytes of a 4-byte value are: BIG_ENDIAN `[1,2,3,4]`,
LITTLE_ENDIAN `[4,3,2,1]`, BIG_ENDIAN_BYTE_SWAP `[2,1,4,3]`, LITTLE_ENDIAN_BYTE_SWAP
`[3,4,1,2]` (8-byte forms alongside). Implementation: the two `*_BYTE_SWAP` modes swap each
adjacent byte pair of the extracted payload, an odd trailing byte is kept (Opt:165-168, 337-347;
Conn:616-627), then the buffer is read or written little-endian for the two LITTLE_ENDIAN
modes and big-endian otherwise (Opt:169-185; Conn:598-613). The effective mode is the
tag's `byte-order` when present, else `default-payload-byte-order` (Conn:590-596; Opt:165).
There is no separate word-order setting upstream.

Stonecrab splits this into core `ByteOrder` (BigEndian, LittleEndian) and `WordOrder`
(HighWordFirst, LowWordFirst) and maps the four upstream names per D14, derived from the
documented layouts above. Precedence is the tag field (`Tag::with_byte_order` and
`with_word_order`), then the address `{byte-order: 'NAME'}` block, then
`default-payload-byte-order` (default BIG_ENDIAN); the order is never inferred.

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-BYTEORDER-BE | `BIG_ENDIAN`, 4-byte layout `[1,2,3,4]` (BO:22-24); no swap, big-endian buffer (Opt:169; Conn:607-612). Lone narrow values are padded first (RegCodec:65-68). Vectors: REAL 3.141593 is registers `0x4049 0x0FDC`, DWORD 4242442424 is `0xFCDE 0x88B8` (Srv:95-97, 137-139). | (BigEndian, HighWordFirst). `TypedDecoder` (S23). A lone BOOL is bit 0 of the register and a lone BYTE, SINT or USINT the low byte; padding is ignored on decode. | implemented (not `automated-verified`: the upstream little-endian vectors are retained since S04a but not yet read by the S04b adapter or the S23 tests) | `byte_order_big_endian_maps_big_high`, `decode_real_3_141593_big_endian`, `decode_dword_4242442424_big_endian`, `decode_dword_layout_1234_and_4321`, `the_two_axes_are_independent`, `typed_vectors_decode_to_the_expected_values`, `register_map_decodes_every_entry_to_the_row_value`, `interop_reads_all_fifteen_holding_register_types` (implemented, S27: the pymodbus map is stored big-endian) |
| MB-BYTEORDER-LE | `LITTLE_ENDIAN`, `[4,3,2,1]` (BO:25-27); no swap, little-endian buffer (Opt:172-178; Conn:600-606). Lone narrow values: BOOL has 7 leading pad bits, the bit, then 8 trailing; BYTE value first (RegCodec:69-75, 158-160). | (LittleEndian, LowWordFirst). `TypedDecoder` (S23). A lone BOOL is bit 8 of the register and a lone BYTE, SINT or USINT the high byte, as the upstream padding rule states (7 leading pad bits, the bit, 8 trailing; value byte first). | implemented (not `automated-verified`: the upstream little-endian vectors are retained since S04a but not yet read by the S04b adapter or the S23 tests) | `byte_order_little_endian_maps_little_low`, `decode_real_little_endian_region`, `decode_lword_layout_87654321`, `a_lone_bool_is_bit_0_under_big_and_bit_8_under_little`, `typed_vectors_decode_to_the_expected_values`, `register_map_decodes_every_entry_to_the_row_value`, `interop_little_endian_region_decodes_real` (implemented, S27: one LITTLE_ENDIAN REAL from pymodbus) |
| MB-BYTEORDER-BE-SWAP | `BIG_ENDIAN_BYTE_SWAP`, `[2,1,4,3]` (BO:28-30); bytes swapped pairwise, then big-endian read (Opt:166-169). | (LittleEndian, HighWordFirst). `TypedDecoder` (S23). Lone narrow values follow the byte order alone (high byte, bit 8): INFERENCE, the record states the padding rule for `BIG_ENDIAN` and `LITTLE_ENDIAN` only. | implemented (not `automated-verified`: the upstream little-endian vectors are retained since S04a but not yet read by the S04b adapter or the S23 tests) | `byte_order_big_endian_byte_swap_maps_little_high`, `decode_dword_layout_2143`, `decode_lword_layout_21436587`, `typed_vectors_decode_to_the_expected_values` |
| MB-BYTEORDER-LE-SWAP | `LITTLE_ENDIAN_BYTE_SWAP`, `[3,4,1,2]` (BO:31-33); bytes swapped pairwise, then little-endian read (Opt:166-178). | (BigEndian, LowWordFirst). `TypedDecoder` (S23). Lone narrow values follow the byte order alone (low byte, bit 0): INFERENCE, as for `BIG_ENDIAN_BYTE_SWAP`. | implemented (not `automated-verified`: the upstream little-endian vectors are retained since S04a but not yet read by the S04b adapter or the S23 tests) | `byte_order_little_endian_byte_swap_maps_big_low`, `decode_dword_layout_3412`, `decode_lword_layout_78563412`, `typed_vectors_decode_to_the_expected_values` |
| MB-BYTEORDER-PRECEDENCE | Tag field over connection default (Conn:590-596). The tag field comes from the `{byte-order}` block (Tag:225-229). `equals` and `hashCode` ignore the byte order (Tag:301-319). | Tag field, then address block, then `default-payload-byte-order`; a name outside the four is `InvalidAddress` or `InvalidConfig`. S24a (`Batcher::plan`) resolves byte order and word order independently, tag over block, and leaves an unset field `None` for the connection default; the connection falls back to `ConnectionDefaults` for a field neither sets (S25); covered by `tag_order_fields_win_over_the_address_block`, `a_lone_tag_order_field_leaves_the_other_to_the_connection_default` and `member_offsets_are_relative_and_carry_type_and_order`. | implemented | `tag_field_beats_address_block_beats_the_connection_default` (implemented, S25: the tag field beats the address block beats the connection default), `a_tag_with_its_own_byte_order_decodes_differently_from_the_default` (implemented, S25), `unknown_byte_order_name_is_rejected` (implemented, S20b). The connection option `default-payload-byte-order` that fills `ConnectionDefaults` is parsed since S26 (`config_byte_order_accepts_four_upstream_names`, `the_parsed_byte_order_decodes_the_reply`) |

Status note (S23, S04a): the four mode rows are `implemented` by `TypedDecoder` and are not
`automated-verified`. The upstream little-endian all-datatypes file (`ADTLE`) is retained
verbatim under `fixtures/modbus/upstream/tcp/` since S04a (maintainer prerequisite P4
granted), but no adapter turns its cases into vectors yet and no test reads it: the rows
reach `automated-verified` when the S04b adapter and the S23 tests use it, so no byte-order
test here is an upstream vector.
The mapping is DERIVED from the documented 4-byte layouts (and checked against the
first-party `tcp/typed_*` vectors, written out by byte permutation, and the pymodbus
interop map for BIG_ENDIAN plus its little-endian region), not confirmed by a run against
a device or an upstream suite that swaps bytes. The rows move to `automated-verified`
together, when the upstream little-endian vectors pass. Since S20b the address parser maps
the four names in an address block through the same table (`byte_order_names_map_per_adr_0006`,
with the `canonical()` rendering checked by `prop_canonical_round_trips`); S23's
`byte_order_*_maps_*` tests assert that table and the decoder agree. `MB-BYTEORDER-PRECEDENCE`
is `implemented` since S25: the decoder takes an explicit pair, and the resolution of tag
field, address block and `default-payload-byte-order` happens in `ModbusConnection` (S25);
S26 parses the option.

Layout rules implemented by `TypedDecoder` (ADR-0015 decision 11): a register's two bytes
are the value's bytes in order under `BigEndian` and swapped within the register under
`LittleEndian`; a multi-register value puts its most significant register first under
`HighWordFirst` and last under `LowWordFirst`. The 4-byte value `0x01020304` is
`[1,2,3,4]`, `[4,3,2,1]`, `[2,1,4,3]` and `[3,4,1,2]` for the four modes. Single-register
narrow values: the record states the upstream padding for BIG_ENDIAN (padded first: BOOL by
15 bits, BYTE, SINT and USINT by 8, so bit 0 and the low byte) and LITTLE_ENDIAN (BOOL has
7 leading pad bits, the bit, then 8 trailing; the BYTE value first, so bit 8 and the high
byte). The two byte-swap modes are INFERENCES from the pair mapping: swapping the bytes of
the padded register gives the high byte (bit 8) for `BIG_ENDIAN_BYTE_SWAP` and the low byte
(bit 0) for `LITTLE_ENDIAN_BYTE_SWAP`, which is also what the byte order of the pair gives.
Also INFERRED: padding bits are ignored on decode (the interop map stores SINT -42 as the
sign-extended `0xFFD6`, which only decodes to -42 if the pad byte is ignored) and written
as zero on encode; upstream's behavior on non-zero padding was not observed.

## Exception codes
Upstream enum `ModbusErrorCode` has ten values and no code 9 (Mspec:410-421). The mapping to
`PlcResponseCode` is `getErrorCode` (Conn:575-588). Stonecrab keeps the raw code and name in
`ProtocolStatus` for every code (D18). Mapping to `ErrorKind`: code 1 becomes
`Unsupported(Operation::Read)`, code 2 `InvalidAddress`, everything else `DeviceException`.
Two documented deviations: code 3 (upstream INVALID_DATA) and code 5 (upstream OK).
V1.1b reference: section 7 (exception responses and the exception code table), see
UNVERIFIED for the section-number caveat.

| Row id | Code, name (Mspec line) | Upstream response-code mapping (Conn line) | Stonecrab ErrorKind (D18) | Status | Test ids |
| --- | --- | --- | --- | --- | --- |
| MB-EXC-01 | 1 ILLEGAL_FUNCTION (411) | UNSUPPORTED (577) | `Unsupported(Operation::Read)` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_01_maps_to_unsupported_read`, `illegal_function_is_unsupported_read_for_the_members` (implemented, S25: the members of the chunk carry it) |
| MB-EXC-02 | 2 ILLEGAL_DATA_ADDRESS (412) | INVALID_ADDRESS (578) | `InvalidAddress` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_02_maps_to_invalid_address`, `a_chunk_exception_fails_only_its_members_and_the_next_chunk_still_runs` (implemented, S25), `interop_read_beyond_map_yields_device_exception` (implemented, S27, job `interop`) |
| MB-EXC-03 | 3 ILLEGAL_DATA_VALUE (413) | INVALID_DATA (579) | `DeviceException`; deviation, no `InvalidData` kind exists in the alpha error set | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_03_maps_to_device_exception_with_raw_code` |
| MB-EXC-04 | 4 SLAVE_DEVICE_FAILURE (414) | REMOTE_ERROR (580) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_04_maps_to_device_exception`, `an_exception_in_the_last_chunk_fails_every_member_of_that_chunk` (implemented, S25) |
| MB-EXC-05 | 5 ACKNOWLEDGE (415) | OK (581). INFERENCE, not run: on a read the error branch calls `splitResponse(block, OK, null)` (Conn:289-292), `splitResponse` only short-circuits for non-OK codes (Opt:115-120) and then dereferences the null payload for register and BOOL coil tags (Opt:143, 163); only `BufferException` is caught (Opt:190). The block future therefore most likely fails, `CompletableFuture.allOf` over the block futures (Conn:255-258) completes exceptionally, and the whole read future fails (CompletionException wrapping NullPointerException) without reaching the per-tag `thenApply` (Conn:259-273), so no per-tag INTERNAL_ERROR is produced. A non-BOOL coil or discrete-input tag instead gets UNSUPPORTED (Opt:132-139) without an NPE. | `DeviceException` carrying code 5; never reported as success; deviation | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_05_maps_to_device_exception_not_success`, `an_exception_leaves_the_connection_connected_and_the_next_read_works` (implemented, S24b), `read_does_not_retry_device_exception` (implemented, S25) |
| MB-EXC-06 | 6 SLAVE_DEVICE_BUSY (416) | REMOTE_BUSY (582) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_06_maps_to_device_exception` |
| MB-EXC-07 | 7 NEGATIVE_ACKNOWLEDGE (417) | REMOTE_ERROR (583) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_07_maps_to_device_exception` |
| MB-EXC-08 | 8 MEMORY_PARITY_ERROR (418) | INTERNAL_ERROR (584) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_08_maps_to_device_exception` |
| MB-EXC-0A | 10 GATEWAY_PATH_UNAVAILABLE (419) | INTERNAL_ERROR (585) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_0a_maps_to_device_exception` |
| MB-EXC-0B | 11 GATEWAY_TARGET_DEVICE_FAILED_TO_RESPOND (420) | REMOTE_ERROR (586) | `DeviceException` | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_carries_each_defined_code_for_each_function` (implemented), `exception_0b_maps_to_device_exception` |
| MB-EXC-UNKNOWN | Any value outside the enum, including 9 (no case in 411-420). What the generated parser does with it is UNVERIFIED. | not observed | `ExceptionCode::Unknown(u8)` names the number (`from_u8(9)` is `Unknown(9)`). The PDU codec (S21) refuses an exception response whose code is not one of the ten, as `InvalidDiscriminator` field `exception code` with the number in the error; the session therefore reports such a reply as `MalformedData`, while `exception_item_error` still maps a code a caller constructs to `DeviceException` with the raw number and the name `UNKNOWN` retained, see the divergence row | implemented | `exception_codes_round_trip_and_name_the_ten_defined_values` (implemented), `exception_response_with_an_undefined_code_is_refused` (implemented), `exception_unknown_code_is_retained_as_device_exception` (implemented, S24b) |

## Read path
Upstream reads go through `ModbusReadOptimizer`; Stonecrab's counterpart is `Batcher::plan`
(S24a), the session (S24b) and the connection that runs the chunks (S25, `ModbusConnection`).

| Row id | Upstream behavior (file:line) | Stonecrab alpha behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MB-READ-MERGE | `onRead` keeps tags whose address failed to parse out of the wire request (Conn:221-233), then groups tags by area and by per-tag unit id, `null` meaning the connection default (Opt:71-105, 201-213), sorts each group by wire address (Opt:225-226, 278-279) and merges tags into one block while the tag end is within the first address plus the configured maximum (Opt:247, 300). A merged bit block is a coil or discrete-input tag of type BYTE (Opt:249-252); a merged register block is a tag of type WORD (Opt:302-304). The first tag of a group is accepted without a size check (Opt:239-245). Blocks run one after another (Conn:245-252). | Group by (unit id, area); merge adjacent or overlapping spans only; chunk limits 125 and 2000 from configuration; `Capabilities.max_items_per_request` honored; every `TagId` kept in its chunk (D19). `Batcher::plan` (S24a) implements the planning; `ModbusConnection::read` (S25) runs the chunks one after another, decodes every member with its resolved byte and word order, and builds the response in request order with `split_into` set to the number of chunks and `source_time` `None`. A chunk exception fails only its members and the next chunk still runs; a chunk whose exchange fails (after retries) fails the whole read, with the chunk named in the message. | implemented | `batcher_merges_adjacent_tags` (implemented), `batcher_merges_overlapping_tags` (implemented), `batcher_groups_by_unit_and_area` (implemented), `batcher_splits_63_dint_tags_into_two_chunks` (implemented), `prop_plan_never_exceeds_limits_and_loses_no_tag` (implemented), `two_areas_split_into_two_chunks_and_every_tag_appears_once_in_request_order` (implemented, S25: split disclosure and request order), `a_chunk_exception_fails_only_its_members_and_the_next_chunk_still_runs` (implemented, S25), `a_read_decodes_the_v11b_example_in_one_chunk_with_the_exact_request_bytes` (implemented, S25), `each_area_uses_its_function_code_and_bits_unpack_lsb_first` (implemented, S25), `source_time_is_none_and_sampled_at_is_the_instant_each_chunk_was_answered` (implemented, S25), `a_count_above_one_is_invalid_config_and_nothing_is_sent` (implemented, S25), `a_foreign_address_is_invalid_address_naming_the_tag_and_nothing_is_sent` (implemented, S25), `interop_split_batch_discloses_chunks` (implemented, S27: two areas, and 126 registers as 125 + 1), `interop_125_register_and_2000_coil_reads` (implemented, S27: 2001 coils as 2000 + 1) |
| MB-READ-GAP | Tags separated by a gap are merged and the gap is read too, because the test is only `tagEnd <= first + max` (Opt:247, 300). | No gap reads; a gapped pair is two requests. Divergence row below. | deferred (M2); no-gap planning implemented | `batcher_does_not_bridge_gaps` |
| MB-READ-TXN | `AtomicInteger` initialised to 1 (Conn:68); `nextTransactionId` returns `getAndIncrement` and resets the counter to 1 when it reaches 0xFFFF, so ids cycle 1..0xFFFE and 0 and 0xFFFF are never used (Conn:159-165; `testNextTransactionId_wraparound`). | `TransactionId` wraps 1..=0xFFFE as upstream (S22: `next()` returns the current id and advances, never 0 or 0xFFFF, `Default` starts at 1). The counter belongs to `ModbusSession` (S24b), which outlives the connection underneath it, so ids continue after a reconnect; `ModbusSession::starting_at` sets the first id. | implemented | `txn_ids_start_at_1` (implemented, S22), `txn_ids_wrap_from_fffe_to_1` (implemented, S22), `txn_ids_never_yield_0_or_ffff_and_cycle_through_65534_values` (implemented, S22), `transaction_ids_increase_and_wrap_from_fffe_to_1` (implemented, S24b), `a_truncated_reply_then_close_is_disconnected_and_the_session_recovers_after_reconnect` (implemented, S24b: the id after the reconnect is 2) |
| MB-READ-MATCH | The reply is looked up by transaction id only; unit id and function code are not compared; an unknown id is logged and dropped (Conn:145-157). | A reply is accepted only when transaction id, unit id and function code (the reply's function with the exception bit cleared) match. Anything else is discarded, counted (`SessionStats.late_replies` when the transaction id is one the session already gave up on, `mismatched_replies` otherwise) and logged at warn with `expected_txn` and `received_txn`; the session reads on through `ConnectionGuard::next_reply` within the same deadline (S24b). The accepted PDU goes through `decode_response` and `ModbusPdu::check_reply_to`; a PDU that does not decode or fit is `MalformedData`. Divergence row below. | implemented | `reply_with_wrong_txn_is_discarded` (implemented, S24b), `reply_with_wrong_unit_is_discarded` (implemented, S24b), `reply_with_wrong_function_code_is_discarded` (implemented, S24b), `late_reply_after_timeout_is_discarded_and_the_next_correlation_succeeds` (implemented, S24b), `only_the_most_recent_abandoned_transactions_are_remembered_as_late` (implemented, S24b), `a_request_refused_before_the_wire_is_not_remembered_as_given_up` (implemented, S24b), `a_discarded_reply_is_logged_at_warn_with_the_expected_and_received_txn` (implemented, S24b), `a_late_reply_is_logged_as_late` (implemented, S24b), `a_reply_whose_byte_count_does_not_fit_the_quantity_is_malformed_data` (implemented, S24b), `a_reply_pdu_that_does_not_decode_is_malformed_data` (implemented, S24b); the function and length half is `ModbusPdu::check_reply_to` (S21): `check_reply_to_proves_the_data_length_against_the_quantity` (implemented), `check_reply_to_refuses_the_wrong_function_or_kind` (implemented) |
| MB-READ-INFLIGHT | INFERENCE from Conn:140-143 and the comment at Conn:243-244 (`RequestThrottle` is not retained): `getMaxConcurrentRequests()` is 1 and every request goes through `executeThrottled` (Conn:140-143, 285, 362; CB:108). | One request in flight (D15). A `ModbusSession` runs one exchange at a time, discards included, so a second `request` made meanwhile fails with `Backpressure` before any I/O and a dropped request frees the slot (S24b). The bounded wait queue sized by `ResourceLimits.max_pending_requests` is the guard's (`TRANSPORT-QUEUE`, S19) and the connection that owns the session serializes its chunks (S25): `ModbusConnection` runs one read at a time, a read that starts while another runs fails with `Backpressure` before any I/O (as the simulator's does), and a dropped read frees the slot. | implemented | `one_request_in_flight_at_a_time` (implemented, S24b), `a_second_read_while_one_is_in_flight_is_backpressure_before_any_io` (implemented, S25, connection half), `requests_beyond_max_pending_requests_fail_with_backpressure_before_any_io` (implemented, S19, transport module) |
| MB-READ-TIMEOUT | INFERENCE (the throttle's handling of exceptions is UNVERIFIED): `orTimeout(request-timeout-ms)` per request; on timeout the pending entry is removed (Conn:183-189). A timeout surfaces as an exceptional future, not a response code. | The session takes the deadline from its caller and bounds every wait by it: the queue, the open, the send, the resync and each wait for a reply, the first and those after a discard (S24b). A timeout is `ErrorKind::Timeout` and the connection stays `Connected`; the reply, if it comes, is counted as late and discarded, and the guard resynchronises before the next send. The connection (S25) gives every attempt of every chunk the deadline `min(request deadline, now + timeouts.request)`, taken when the attempt starts; a request without a deadline therefore gets `timeouts.request` per chunk, a request deadline in the past is `Timeout` before any I/O, and a timed-out chunk fails the read (it does not become member errors, because a chunk that was not answered has no per-member status to report). | implemented | `read_times_out_after_request_timeout_on_fake_clock` (implemented, S24b), `the_chunk_deadline_is_the_smaller_of_the_request_deadline_and_the_request_timeout` (implemented, S25), `every_chunk_gets_its_own_request_timeout` (implemented, S25), `the_request_deadline_still_bounds_a_later_chunk` (implemented, S25), `an_expired_request_deadline_is_timeout_without_io` (implemented, S25), `cancel_before_flush_is_cancelled_with_nothing_sent` through the connection (implemented, S25), `cancel_during_a_chunk_is_cancelled_and_the_next_read_succeeds_after_the_guard_resyncs` (implemented, S25), `the_deadline_also_bounds_the_wait_after_a_discarded_reply` (implemented, S24b), `late_reply_after_timeout_is_discarded_and_the_next_correlation_succeeds` (implemented, S24b), `cancel_before_flush_is_cancelled_with_nothing_sent` (implemented, S24b), `cancel_after_flush_is_cancelled_and_the_next_request_resyncs` (implemented, S24b), `close_mid_reply_is_disconnected_and_reconnecting` (implemented, S24b), `a_truncated_reply_then_close_is_disconnected_and_the_session_recovers_after_reconnect` (implemented, S24b) |
| MB-READ-FRAMING | Frame size is 6 plus the big-endian length field at bytes 4-5, minimum header 6 bytes (Codec:37-54); parsed with `ModbusADU.staticParse(..., MODBUS_TCP, true)` (Codec:57-62). ADU layout: transaction id u16, protocol id const 0, length implicit `pdu.lengthInBytes + 1`, unit id u8, PDU (Mspec:36-58). | `MbapFramer` (S22), a `stonecrab::transport::Framer`; see MB-LIMIT-ADU for the bounds. `decode` waits with `Ok(None)` for the 7 byte header and for the rest of the ADU, consumes exactly one ADU from the front of the buffer and leaves the rest, so fragmentation and coalescing are handled; the PDU stays opaque `Bytes` until the session has matched the reply (S24b). `encode` writes header then PDU. The stream-level behavior is proved over `FramedTransport` and the in-memory transport double, over live loopback sockets by the S26 tests and against pymodbus by the S27 tests; the six frames of the upstream `ParserSerializerTestsuite` (`PST:30`) are retained as `tcp/adu_upstream-*` vectors (S04a) and run through the framer like the first-party ADUs. | implemented | `mbap_fragmented_adu_is_reassembled` (implemented, S22), `mbap_coalesced_adus_are_split` (implemented, S22), `mbap_coalesced_adus_are_delivered_one_at_a_time` (implemented, S22), `mbap_header_matches_spec_vector` (implemented, S22), `the_upstream_adus_are_cut_whole_as_the_parser_suite_describes` (implemented, S04a: the six upstream frames, header fields and lengths as the suite states them), `decode_compare_every_accepted_vector_matches_its_expected_file` (implemented, S22), `re_encode_compare_every_expected_adu_encodes_to_the_vector_bytes` (implemented, S22), `fuzz_modbus_mbap` (implemented, S22), `a_holding_register_read_over_loopback_returns_the_v1_1b_values` (implemented, S26, loopback), `interop_reads_all_fifteen_holding_register_types` (implemented, S27, job `interop`) |
| MB-READ-RECONNECT | No reconnect or retry in the inventoried files (grep finds none in Conn, CB, DB, Opt, Poll). Transport loss fails pending requests (Conn:115-128). | Request-driven reconnect with capped jittered backoff (D15; guard S19, connection S25); extension, not parity. After a peer close the state is `Reconnecting { attempt, next_at }`; the next read waits out the backoff, reopens and sends; the read that was in flight fails with `Disconnected` (retried only if the policy says so) and is never replayed on the new transport; transaction ids continue; `close` during the backoff ends `Closed`; an exhausted budget is `Failed(Disconnected)` and later reads fail without I/O. | implemented | `read_after_peer_close_observes_reconnecting_then_connected_and_succeeds` (implemented, S25), `pending_read_fails_disconnected_and_is_not_replayed_after_reconnect` (implemented, S25), `transaction_ids_continue_after_reconnect` (implemented, S25), `close_during_backoff_yields_closed` (implemented, S25), `reconnect_exhaustion_yields_failed` (implemented, S25), `a_deadline_shorter_than_the_backoff_is_a_timeout_and_the_connection_keeps_reconnecting` (implemented, S25), `open_fails_when_the_peer_refuses_or_the_connect_times_out` (implemented, S25), `a_read_on_a_closed_connection_is_disconnected_without_io` (implemented, S25) |
| MB-READ-RETRY | INFERENCE, not run: no read retry exists in the inventoried read path. `onRead`, `executeOptimizedRead` and `sendRequest` send once and complete or fail (Conn:218-308, 167-192); a grep for retry in Conn, CB, DB, Opt and Poll finds no retry logic. | `RetryPolicy` default is one attempt (S07); a retry happens only when the caller raises `max_attempts`, only on retryable kinds, never for writes (S25). The retry is per chunk with a fresh chunk deadline and the next transaction id, stops when the request deadline has passed, and a device exception is an answer and is never retried. `Diagnostics.retries` is the retries performed (attempts minus one) and the chunk is named in the message. The default therefore matches the inferred upstream behavior. | implemented | `read_default_policy_sends_once` (implemented, S25), `read_retries_timeout_once_then_succeeds_within_max_attempts` (implemented, S25), `read_does_not_retry_device_exception` (implemented, S25), `a_kind_outside_the_retry_set_is_not_retried` (implemented, S25), `max_attempts_exhausted_fails_with_the_retries_in_the_diagnostics` (implemented, S25: `sent().len()` equals `max_attempts`), `a_retry_repeats_only_the_chunk_that_failed` (implemented, S25), `retry_stops_when_the_request_deadline_has_passed` (implemented, S25), `writes_are_never_retried_and_send_nothing` (implemented, S25), `a_retry_is_logged_at_debug_with_the_chunk_the_attempt_and_the_kind` (implemented, S25) |

## Interop register map
Upstream's own interop test peer is pymodbus 3.8.6 (`Dkr:24`, `pip install pymodbus==3.8.6`) on
`python:3.12-slim` (`Dkr:20`), started by `modbus_server.py` (`Srv:20-27`, ports `Srv:47-54`,
TCP/SOCKET framer on 5020 at `Srv:269-274`), exercised by `DockerIT:157-575`. Stonecrab does
not copy that file (D8). The Stonecrab interop server is written independently and
reproduces only these fixture values. Addresses here are the logical tag addresses used in
the upstream tags; the wire address is one less (`Srv:71-74`: pymodbus without `zero_mode`
stores tag address T at index T).

| Row id | Area and logical address | Type and value | Registers or bits (file:line) | Status | Test ids |
| --- | --- | --- | --- | --- | --- |
| MB-INTEROP-HR-01 | holding-register 1 | BOOL true | 0x0001 (Srv:86-87) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks`, `interop_bare_40001_reads_with_explicit_convention` |
| MB-INTEROP-HR-02 | 2, 3 | BYTE 42, WORD 42424 | 0x002A, 0xA5B8 (Srv:89-93) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks` |
| MB-INTEROP-HR-03 | 4-5, 6-9 | DWORD 4242442424, LWORD 4242442424242424242 | 0xFCDE 0x88B8; 0x3AE0 0x2EE8 0x4D04 0x49B2 (Srv:95-103) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks` |
| MB-INTEROP-HR-04 | 10, 11, 12, 13 | SINT -42, USINT 42, INT -2424, UINT 42424 | 0xFFD6, 0x002A, 0xF688, 0xA5B8 (Srv:105-115) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks`, `interop_contract_suite_passes_against_pymodbus`, `interop_read_beyond_map_yields_device_exception` |
| MB-INTEROP-HR-05 | 14-15, 16-17 | DINT -242442424, UDINT 4242442424 | 0xF18C 0x9F48; 0xFCDE 0x88B8 (Srv:117-123) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks` |
| MB-INTEROP-HR-06 | 18-21, 22-25 | LINT -4242442424242424242, ULINT 4242442424242424242 | 0xC51F 0xD117 0xB2FB 0xB64E; 0x3AE0 0x2EE8 0x4D04 0x49B2 (Srv:125-135) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks` |
| MB-INTEROP-HR-07 | 26-27, 28-31 | REAL 3.141593, LREAL 2.71828182846 | 0x4049 0x0FDC; 0x4005 0xBF0A 0x8B14 0x5FCF (Srv:137-145) | implemented (S27, job `interop`) | `interop_reads_all_fifteen_holding_register_types`, `interop_split_batch_discloses_chunks` |
| MB-INTEROP-COIL | coil 1-10 | T,F,T,T,F,T,F,F,T,F (read area); 100-120 write area, initially false | (Srv:150-172) | implemented (S27, job `interop`) | `interop_reads_coils_1_to_10`, `interop_125_register_and_2000_coil_reads`, `interop_bare_40001_reads_with_explicit_convention` |
| MB-INTEROP-DI | discrete-input 1-10 | F,T,F,T,T,F,T,T,F,T | (Srv:175-195) | implemented (S27, job `interop`) | `interop_reads_discrete_inputs_1_to_10`, `interop_bare_40001_reads_with_explicit_convention` |
| MB-INTEROP-IR | input-register 1-7 | 1 BOOL true, 2 INT -2424, 3 UINT 42424, 4-5 DINT -242442424, 6-7 REAL 3.141593 | 0x0001; 0xF688; 0xA5B8; 0xF18C 0x9F48; 0x4049 0x0FDC (Srv:198-225) | implemented (S27, job `interop`) | `interop_reads_input_registers_1_to_7`, `interop_split_batch_discloses_chunks`, `interop_read_beyond_map_yields_device_exception`, `interop_bare_40001_reads_with_explicit_convention` |
| MB-INTEROP-LE | holding-register 300-301 | REAL 3.141593 under LITTLE_ENDIAN | 0xDC0F 0x4940 (D8; outside the upstream block of 201 registers, `Srv:84`, so Stonecrab-only) | implemented (S27, job `interop`) | `interop_little_endian_region_decodes_real` |

The holding-register write area is 100-200 and starts at zero (`Srv:75-76`); write is
deferred, so it is not read at alpha. The upstream blocks are sized 201 holding registers
(`Srv:84`), 2000 coils (`Srv:160`), 2000 discrete inputs (`Srv:183`) and 21 input
registers (`Srv:205`). Observed in the S27 runs against pymodbus 3.8.6 (previously
UNVERIFIED): an FC03 or FC04 read past the end of a register block is refused with
exception 02 (`interop_read_beyond_map_yields_device_exception` reads input register 22 of
a 21-register map), while FC01 and FC02 reads are not validated at all and past the end of a
block come back short or empty, a malformed reply. The first-party server therefore
allocates `N + 1` slots per register block (logical 1 to `N` served) and lets the coil and
discrete-input blocks cover the whole 16-bit address space (ADR-0015, decision 16).
`interop_contract_suite_passes_against_pymodbus` runs the S13 suite through
`ModbusTcpDriver::new()`. The `DockerIT` cases that map to alpha operations are the
connection test (`DockerIT:157`), the 15 single reads (`DockerIT:166-334`), multi-tag read
(`DockerIT:350`), coils (`DockerIT:525`), discrete inputs (`DockerIT:558`) and input
registers (`DockerIT:575`).

## Upstream quirks
Observed in the pinned source and not reproduced. Letters follow the order of the source
audit; `l` was added while reading `Tag.java` and is not in the audit.

| Row id | Quirk | Upstream (file:line) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| MB-QUIRK-A | (a) `ModbusTcpDriver` imports `org.bouncycastle.tls.UDPTransport` for its `instanceof` check; INFERENCE: the UDP branch cannot match a PLC4X transport (its class is not retained); consequence INFERRED: UDP falls through to the default transport configuration class | Drv:34, 66-67 | not reproduced; implemented | UDP is rejected with `InvalidConfig`: `driver_rejects_other_transport_codes` (implemented, S26) |
| MB-QUIRK-B | (b) `extractResponseData` returns only the first file-record item, so a split extended-register read silently drops the second item | Conn:517-518 (split built at Conn:422-431) | not reproduced; implemented | Extended registers rejected at parse time: `extended_register_is_rejected_at_parse_time` |
| MB-QUIRK-C | (c) The FC05 echo check is dead code because writes always use FC15 | Conn:371-377 vs Conn:438-448 | not reproduced; implemented | Write is unsupported at alpha: `write_is_unsupported_with_nothing_sent_and_the_capabilities_say_so` (S25) |
| MB-QUIRK-D | (d) `onPing` reports OK for any reply, including an exception PDU | Conn:205-210 | not reproduced | Ping is deferred (`MB-TCP-PING`, decided in S31): core defines no ping operation, so there is nothing to test; the `ping-address` key is validated and recorded only: `ping_address_is_validated_but_unused` |
| MB-QUIRK-E | (e) Replies are matched on transaction id only | Conn:145-157 | not reproduced; implemented | `reply_with_wrong_unit_is_discarded`, `reply_with_wrong_function_code_is_discarded`, see MB-READ-MATCH |
| MB-QUIRK-F | (f) The manual test suite passes `unit-identifier`, which `ModbusTcpConfiguration` does not declare; it equals the default so the suite still passes | MF:30-33 vs Cfg:36-39 | not reproduced; implemented | The vector adapter maps the key to `default-unit-identifier`; the driver rejects the misspelt key: `config_misspelt_unit_identifier_is_rejected` (implemented, S26) |
| MB-QUIRK-G | (g) The fixed-digit address group is optional, so `4x:INT` matches the short pattern; consequence INFERRED: `NumberFormatException` instead of a typed invalid-tag error | Tag:48; TagHR:83 | not reproduced; implemented | Typed `InvalidAddress`: `four_x_without_digits_is_rejected` |
| MB-QUIRK-H | (h) The simulated driver's Javadoc shows the pre-migration `STATE/bar:STRING[16]` form while the regex is the new form | `plc4j/drivers/simulated/src/main/java/org/apache/plc4x/java/simulated/SimulatedDriver.java:41-42` | not reproduced | Not a Modbus behavior; the simulator is Stonecrab-owned (`docs/parity/sim.md`, S11b): no Modbus test |
| MB-QUIRK-I | (i) `ConnectionBase` polling receive loop calls `Thread.onSpinWait()` for non-async transports | CB:169-177 | not reproduced | Readiness-driven I/O, no polling loop and no spawned task (D15, ADR-0014): no test, the property is structural (`clippy.toml` disallows `tokio::task::spawn` and no library code spawns a task; a transport does no work between calls) |
| MB-QUIRK-J | (j) Non-BOOL coil and discrete-input tags parse but fail at read time with UNSUPPORTED | TagCoil:100; Opt:132-139 | not reproduced; implemented | `typed_coil_address_is_rejected_at_parse_time`, see MB-ADDR-BIT-TYPES |
| MB-QUIRK-K | (k) The TIME and DATE family parses as a data type but cannot be encoded | RegCodec:191-192; Tag:279-282 | not reproduced; implemented | `time_family_names_are_rejected_at_parse_time` |
| MB-QUIRK-L | (l) `equals` and `hashCode` ignore the byte order, so two tags that differ only in `byte-order` compare equal | Tag:301-319 | not reproduced; implemented | Byte order is part of the parsed address value: `addresses_differing_only_in_byte_order_are_not_equal` |

## Divergences from upstream
Every row has a reference: a protocol section, an upstream issue URL, or the stated state
of its issue.
S32 (ADR-0017) drafted the issues for quirks a, b, c, e, f and g under
`docs/evidence/upstream-issues/` and did not file them: filing on the upstream tracker
needs the maintainer's approval, so those rows say "drafted, not filed". Rows outside
those six say that no issue was drafted and why. Quirks d, i and l keep their divergence
rows with "no issue drafted" and the reason in the cell. When the maintainer files a draft,
the cell becomes the issue URL.
A divergence row reuses the id of the row it qualifies, so a citation of that id resolves to
both the behavior row and its divergence.

| Row id | Upstream behavior (file:line at ea229918) | Stonecrab behavior | Reference (Modbus Application Protocol V1.1b section, or an upstream issue URL) | Regression test id |
| --- | --- | --- | --- | --- |
| MB-ADDR-BARE-40001 | Bare short forms (`40001`, `400001`, `00001`, `10001`, `30001`) are accepted unconditionally (TagHR:33, 74-78; TagCoil:33) | Accepted only with `address-convention=plc4x`; otherwise `InvalidAddress` naming the ambiguity | V1.1b section 4.4 (PDU addresses are 0..65535; the one-based 4x numbering is a convention, not a protocol rule) | `bare_40001_is_rejected_unless_convention_is_explicit` |
| MB-ADDR-BIT-TYPES | Non-BOOL types parse on coil and discrete-input, fail at read with UNSUPPORTED (TagCoil:100; Opt:132-139) | `InvalidAddress` at parse time | V1.1b sections 6.1 and 6.2 (coil and discrete-input values are single bits) | `typed_coil_address_is_rejected_at_parse_time` |
| MB-CFG-UNIT-ID | `unit-id` parsed with `Short.parseShort`, no 0..=255 check in the inventoried files (Tag:222-224); default cast to short (Conn:394-400) | Bounded 0..=255 (`UnitId`); outside is `InvalidAddress` or `InvalidConfig`; the bound is Stonecrab inference, not observed upstream behavior | no upstream issue drafted in S32: the bound is a Stonecrab inference, not an observed upstream defect (the MBAP unit identifier is one byte, Mspec:54) | `unit_id_256_is_rejected`, `config_unit_id_rejects_256_and_negative` |
| MB-READ-MATCH | Reply matched on transaction id only (Conn:145-157) | Reply validated on transaction id, unit id and function code; mismatches discarded, counted (`late_replies`, `mismatched_replies`) and logged at warn | V1.1b section 7 (an exception response echoes the function code with the high bit set); the unit identifier echo is the MBAP rule of the Modbus Messaging on TCP/IP Implementation Guide V1.0b (not retained, see UNVERIFIED); upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-e.md`) | `reply_with_wrong_txn_is_discarded`, `reply_with_wrong_unit_is_discarded`, `reply_with_wrong_function_code_is_discarded`, `late_reply_after_timeout_is_discarded_and_the_next_correlation_succeeds` |
| MB-CFG-UNKNOWN | Unknown parameters are reported, not rejected (DB:206-209, 315-319) | Rejected with `InvalidConfig` | upstream issue drafted as a question inside the quirk f draft, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-f.md`); behavioral difference, not a protocol rule; a misspelt key silently leaves the default | `config_unknown_key_is_rejected` |
| MB-EXC-03 | Code 3 maps to INVALID_DATA (Conn:579) | `DeviceException` with the raw code retained | V1.1b section 7 (code 03 ILLEGAL DATA VALUE) | `exception_03_maps_to_device_exception_with_raw_code` |
| MB-EXC-05 | Code 5 maps to OK (Conn:581); on a read the consequence is INFERRED to be a failed whole read future (NullPointerException via `allOf`, Conn:255-258), not per-tag results (see MB-EXC-05) | `DeviceException`; never success | V1.1b section 7 (code 05 ACKNOWLEDGE: request accepted and processing, no data returned); no upstream issue drafted in S32 (not among quirks a, b, c, e, f, g): the consequence is INFERRED and was not reproduced; left to maintainer triage | `exception_05_maps_to_device_exception_not_success` |
| MB-EXC-UNKNOWN | A code outside the enum (Mspec:410-421, no code 9): what the generated parser does is UNVERIFIED | `decode_response` refuses an exception response whose code is not one of the ten defined ones (`InvalidDiscriminator`, field `exception code`); `encode_response` refuses to write one; the session reports such a reply as `MalformedData`, and `exception_item_error` keeps a constructed `Unknown` code as `DeviceException` with the raw number | V1.1b section 7 (the defined exception codes) | `exception_response_with_an_undefined_code_is_refused`, vectors `tcp/response-exception-code-09` and `tcp/response-exception-code-0c` |
| MB-ADDR-SELECTION | `[n]` and `[a..b]` selections accepted (ANP:181-226; Tag:80-89) | `InvalidAddress` at parse time, typed | V1.1b section 4.4 (addressing model); scope deferral D16, M2 | `selection_range_is_rejected_at_parse_time` |
| MB-ADDR-STRING | `STRING(n)`, `WSTRING(n)`, CHAR, WCHAR accepted (Tag:206-221; RegCodec:163-194) | Typed `InvalidAddress` at parse time | V1.1b section 4.2 (data encoding); scope deferral D16, M2 | `string_type_is_rejected_at_parse_time` |
| MB-TYPE-TIME | TIME and DATE names parse, cannot be encoded (RegCodec:191-192; Tag:279-282) | Typed `InvalidAddress` at parse time | V1.1b section 4.2 (no calendar types in the protocol); no upstream issue drafted in S32 (not among quirks a, b, c, e, f, g): the generated `DataItem` code is not in the repository (manifest pending item), so the behavior cannot be cited in full | `time_family_names_are_rejected_at_parse_time` |
| MB-ADDR-EXTENDED | `extended-register`/`6x` accepted, FC20 read (TagExt:54-112; Conn:411-432) | Typed `InvalidAddress` at parse time | V1.1b section 6.14 (FC20 Read File Record); scope deferral D16, M2 | `extended_register_is_rejected_at_parse_time` |
| MB-READ-GAP | Gap between non-adjacent tags is read as part of the block (Opt:247, 300) | No gap reads (D19); adjacent or overlapping merge only | V1.1b section 7 (exception 02 when any address in the span is not mapped, so a gap read can fail neighbors) | `batcher_does_not_bridge_gaps` |
| MB-LIMIT-ADU | MBAP length field accepted without an upper bound when sizing a frame (Codec:49-54; `MessageCodecBase` not retained; UNVERIFIED) | Length 2..=254, ADU at most 260 before the body is allocated | V1.1b section 4.1 (PDU at most 253 bytes); no upstream issue drafted in S32 (not among quirks a, b, c, e, f, g): `MessageCodecBase` is not retained and the behavior is UNVERIFIED | `mbap_length_255_is_rejected` |
| MB-QUIRK-A | UDP configuration-class check uses a BouncyCastle `UDPTransport` import (Drv:34, 66-67) | Not ported; UDP rejected | upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-a.md`) | `driver_rejects_other_transport_codes` |
| MB-QUIRK-B | Split FC20 read drops the second item (Conn:517-518) | Not ported; extended registers rejected | upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-b.md`) | `extended_register_is_rejected_at_parse_time` |
| MB-QUIRK-C | Dead FC05 echo branch (Conn:371-377) | Not ported; write unsupported at alpha | upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-c.md`) | `write_is_unsupported_with_nothing_sent_and_the_capabilities_say_so` |
| MB-QUIRK-D | Ping reports OK on any reply (Conn:205-210) | Not ported; ping deferred | no issue drafted: Stonecrab has no ping operation (`MB-TCP-PING`), so the behavior is absent rather than different, and the upstream report would be speculative | `ping_address_is_validated_but_unused` (no ping operation exists to test) |
| MB-QUIRK-F | Test suite passes undeclared `unit-identifier` (MF:30-33) | Not ported; unknown keys rejected | upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-f.md`) | `config_misspelt_unit_identifier_is_rejected` |
| MB-QUIRK-G | `4x:INT` reaches `Integer.parseInt(null)` (Tag:48; TagHR:83; consequence INFERRED) | Typed `InvalidAddress` | upstream issue drafted, not filed: needs maintainer approval (`docs/evidence/upstream-issues/quirk-g.md`) | `four_x_without_digits_is_rejected` |
| MB-QUIRK-I | Polling receive loop with `onSpinWait` for non-async transports (CB:169-177); whether the tcp transport is async is UNVERIFIED | Not ported; no polling loop | no issue drafted: a design difference (readiness-driven I/O, D15) and the upstream transport behavior is UNVERIFIED | none: structural, see the quirk row above |
| MB-QUIRK-L | `equals` and `hashCode` ignore byte order (Tag:301-319) | Byte order participates in address equality | no issue drafted: the upstream behavior may be intended, and it is a behavioral difference that a reviewer should weigh before it is reported | `addresses_differing_only_in_byte_order_are_not_equal` |

## MSpec feature matrix
Nothing generated; PDUs hand-written from modbus.mspec (decision D6) under
`crates/stonecrab/src/modbus/pdu/` (S21).

The pinned mspec is 439 lines with an ASF header (`Mspec:1-18`). It models TCP, RTU and
ASCII ADUs (`Mspec:36-75`) and many PDUs beyond the four read functions; the alpha
transcribes FC01 to FC04 request/response and the exception PDU only. The corpus a future
generator would have to meet is `fixtures/modbus/tcp/` (spec-authored and adversarial
PDU vectors, S21, and the six upstream parser-suite frames, S04a).

## Test vectors available upstream
Under `protocols/modbus/src/test/resources/protocols/modbus` (all Apache-2.0 with ASF headers;
the maintainer approved redistribution with NOTICE attribution on 2026-10-28, P4 and
decision E2; four of the files are retained, see below):
`tcp/ParserSerializerTestsuite.xml` (6 cases, e.g. raw `000000000006ff0408d20002`, `PST:30`), `tcp/DriverTestsuite.xml` (5 cases; holding-register:1:REAL request
`startingAddress 0, quantity 2`, reply `0x40490fdb` as 3.1415927, `DTS:33-121`),
`tcp/DriverTestsuiteOptimized.xml` (loaded by `ModbusTcpDriverIT`),
`tcp/ManualFactoryModbusTCPDriverTest-testsuite.xml` (32 cases),
`tcp/Modbus-all-datatypes.xml` and `...-little-endian.xml` (55 cases each, the latter with
`default-payload-byte-order=LITTLE_ENDIAN`, `ADTLE:29-34`), and RTU and ASCII parser
suites (deferred, M2).

Four of these files are retained byte for byte, ASF headers included, under
`fixtures/modbus/upstream/tcp/` (S04a; commit `ea229918`, git blob SHAs and sha256 in
`fixtures/modbus/PROVENANCE.toml`, attribution in `NOTICE`): `ParserSerializerTestsuite.xml`,
`DriverTestsuite.xml`, `Modbus-all-datatypes.xml` and `Modbus-all-datatypes-little-endian.xml`.
The other three are not copied.

`fixtures/modbus/tcp/` holds the first-party PDU vectors (S21): the read examples of V1.1b
sections 6.1 to 6.4, exception responses (section 7) and adversarial bytes, each listed in
`fixtures/modbus/PROVENANCE.toml` with `protocol_spec` or `adversarial` as its source. The six
`<raw>` elements of `ParserSerializerTestsuite.xml` (`PST:30`) are the `upstream_fixture`
vectors `tcp/adu_upstream-*`, each a whole ADU with its MBAP header, re-derived from the
retained XML by the test `adapter_rederives_each_parser_suite_hex_from_raw_text`. The FC04
request (`000000000006ff0408d20002`) and response (99 registers) decode at the PDU layer
and are framed by the MBAP framer; the four FC20/FC21 ("extended registers") frames are
framed too and are `must_fail` at the PDU layer (`InvalidDiscriminator`: function codes
outside the alpha set). The cases of `DriverTestsuite.xml` and of the two all-datatypes
files are not yet turned into vectors; that is the S04b adapter.

## UNVERIFIED
- Generated `DataItem.staticParse` and `staticSerialize` behavior for the TIME and DATE
  family names (the generated classes are not in the repository).
- Which XML the little-endian IT (`ModbusTcpAllDatatypesLittleEndianIT`) loads: the class is
  not among the retained copies. `ADTLE` exists and carries the LITTLE_ENDIAN driver
  parameter, so it is the likely file.
- The internals of the `test` transport used by the driver test runners.
- Whether `ADT` and `ADTLE` use the same XML-serialized `ModbusADU` step format as `DTS`.
  Evidence so far: the step element names (`api-request`, `outgoing-plc-message`,
  `incoming-plc-message`, `api-response`) occur 28 times each in both files; the
  `ModbusADU` serialization inside those steps was not compared.
- The number of wire exchanges per `DTS` case (the single and array reads show one
  request/reply pair; the multi-element case was not counted).
- The 27 pcap-derived "Query/Response" cases in `ADT` and the contents of
  `manual-test-capture.pcapng`.
- Whether the RTU and ASCII connections reuse `ModbusTag` and `ModbusReadOptimizer`
  unchanged (not read; post-alpha).
- The log level of unknown-parameter reporting: `UnknownParameterReporter` is not retained
  (DB:315-319).
- The whitespace tolerance of the upstream address-block regex (the text of
  `KEY_VALUE_PATTERN`, TCP:34, is not reproduced here), whether upstream accepts
  double-quoted enum values (the value pattern lists them; not run), and the wording of
  the upstream legacy-array migration hint (ANP:130-138). The Stonecrab choices are in the
  note under Address grammar.
- Consequences marked INFERENCE: UDP configuration-class fallthrough (MB-QUIRK-A),
  `NumberFormatException` for `4x:INT` (MB-QUIRK-G, MB-ADDR-CONFIG-BLOCK), silent drop of
  an unquoted enum value (MB-ADDR-CONFIG-BLOCK), the connection metadata flags for
  subscribe (MB-TCP-CAPABILITIES), unreachable wire address 65535 (MB-LIMIT-SPAN), the
  ACKNOWLEDGE read consequence, a whole-read failure rather than per-tag errors
  (MB-EXC-05), absence of read retry (MB-READ-RETRY). None was observed by running the
  Java code.
- `RequestThrottle.execute` internals (serialisation and exception handling), behind
  MB-READ-INFLIGHT and MB-READ-TIMEOUT.
- The conventional meaning of unit identifiers 0 (broadcast) and 255 (TCP no-unit); that
  comes from the Modbus Messaging on TCP/IP Implementation Guide, which is not retained.
- Modbus Application Protocol V1.1b section numbers (4.1, 4.2, 4.4, 6.1 to 6.4, 6.14 and 7):
  the PDF is not retained here and the numbers are from its published table of contents;
  confirm against a copy of the specification when one is available.
- The example bytes of the S21 spec vectors (`fixtures/modbus/tcp/`, sections 6.1 to 6.4,
  6.5, 6.12, 6.14 and 7) and the section 7 exception names (the variants use the
  "SERVER DEVICE" wording, the older edition of the specification says "SLAVE DEVICE"):
  they were written out from the published examples without a checked-in copy of the PDF.
  A maintainer should compare them with the PDF; `ExceptionCode::name` is the text to
  correct if the wording differs.
- The unit identifier echo that the session checks (`MB-READ-MATCH`): a server copies the
  request's unit identifier into the reply per the Modbus Messaging on TCP/IP
  Implementation Guide V1.0b (MBAP header description); the guide is not retained here
  and no section number is cited. The function code echo with the exception bit set is
  V1.1b section 7.
- The byte-order mapping against the upstream little-endian all-datatypes vectors
  (`MB-BYTEORDER-*`): the file is retained (S04a) but no adapter or test reads it until
  S04b and S23. Only BIG_ENDIAN and one LITTLE_ENDIAN REAL are confirmed by a run, against
  pymodbus.
- Driver behavior over a real clock and against peers that drop links, delay replies or
  answer with exceptions on demand: timing is tested on `FakeClock` and scripted peers
  only (D21), and a stock pymodbus has no fault hooks.
- Whether a server other than pymodbus 3.8.6 agrees (for example on unit-id echo).
- Hardware behavior: no Modbus device has been tested; hardware status is none.

## Required evidence before a read capability claim
Each bullet is the requirement and where the alpha meets it.

- Address and configuration tests against pinned source semantics: the `MB-ADDR-*`,
  `MB-CFG-*` and `MB-LIMIT-*` rows, with property tests and the fuzz targets
  `fuzz_modbus_address` and `fuzz_modbus_connection_string`.
- Independent success/error vectors, malformed lengths and function codes: the
  first-party vectors in `fixtures/modbus/tcp/` (written from the Modbus Application
  Protocol V1.1b examples or built to fail, with provenance), the `MB-EXC-*` rows and
  `fuzz_modbus_pdu` and `fuzz_modbus_mbap`, and the six upstream parser-suite frames
  (`tcp/adu_upstream-*`, S04a): the FC04 request and response and four must_fail
  FC20/FC21 frames. Gap: the cases of the driver and all-datatypes suites are not yet
  vectors (S04b).
- Fragmentation, coalescing, close boundaries and transaction/unit mismatch:
  `MB-READ-FRAMING`, `MB-READ-MATCH` and `MB-READ-TXN` on the in-memory transport and
  loopback sockets. Unit-id correlation is not exercised against pymodbus, which answers
  every unit id.
- Timeout, cancellation, late replies, resynchronization and reconnect: `MB-READ-TIMEOUT`,
  `MB-READ-RECONNECT`, `MB-READ-RETRY` and `MB-READ-INFLIGHT` on `FakeTransport` and
  `FakeClock` (D21). Not exercised over a real clock or against a peer that drops links
  on demand.
- Tag correlation and partial failures across batch splitting: `MB-READ-MERGE`, with
  `interop_split_batch_discloses_chunks` and `interop_read_beyond_map_yields_device_exception`
  against pymodbus.
- Independent server interoperability and explicit hardware status: the five
  `automated-verified` rows against pymodbus 3.8.6; hardware status is none, and no
  physical device has been tried.

Full baseline inventory is incomplete and M0 is not declared complete; the open items
are the `[pending]` list of `upstream/manifest.toml`, none of which gates the alpha.
