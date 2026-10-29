# Stonecrab compatibility matrix

Status: alpha, reconciled with the code in slices S31 and S33 of
[`ALPHA_PLAN.md`](ALPHA_PLAN.md). Everything below is true of the code at that slice;
nothing is promised that is not shipped, and deferred items say what owns them.

Stonecrab is one crate, `stonecrab` 0.2.0 (ADR-0018). The Feature column of the table
below names the Cargo feature that switches a capability on. `sim`, `modbus` and
`sensors` are on by default; `default-features = false` leaves the core and the codec.
The `testkit` feature is test tooling, not a capability, and carries no support level.

Upstream baseline: Apache PLC4X v1.0.0, tag object
`32a524591841ab56d26b032914ce06918148fbb6`, commit
`ea229918f5d07700e04d7d69ab1645da679cb700`, pinned in
[`upstream/manifest.toml`](../upstream/manifest.toml). The manifest's inventory is
incomplete (`inventory_complete = false`); the source-release signature was verified in S32
(`signature_verified = true`: good signature, key not certified by a trust path; the archive
contents were not inventoried against the pinned commit, see
[`evidence/alpha.md`](evidence/alpha.md)); the Modbus TCP subset is the only one
implemented, in [`parity/modbus.md`](parity/modbus.md), and slice S43 added a parity record
for every other upstream driver, transport and tool (see
[Upstream drivers, transports, tools and integrations](#upstream-drivers-transports-tools-and-integrations)). Stonecrab is an independent port: it is not
affiliated with the Apache Software Foundation or Apache PLC4X, and no Apache PLC4X source
file is copied into this repository.

## Support levels

Four levels come from the hardware-evidence policy of [`SPEC.md`](SPEC.md); `planned` and
`not supported` say a capability is absent. Each level names the strongest evidence that
exists, not the intended one. A parity row can be `interoperability-tested` for specific
tests while its status in `docs/parity/` stays `implemented`; this table names the tests.

| Level | Meaning here |
| --- | --- |
| `build-tested` | compiles on the platform, with unit and documentation tests; no peer involved |
| `simulator-tested` | exercised against the in-process simulator, the in-memory `FakeTransport` with a fake clock, or a loopback `ScriptedPeer` |
| `interoperability-tested` | exercised against an independent implementation: pymodbus 3.8.6 in the `interop` CI job, libmodbus 3.1.11 in the `interop-libmodbus` job, both of them behind Toxiproxy 2.13.0 for the failure paths in the `interop-toxiproxy` job, for unit-ID correlation, a first-party multi-unit script on modbus-tk 1.1.5 in the `interop-modbus-tk` job, and, for Modbus over UDP, diagslave 3.6 with `tc netem` in the `interop-udp` job (first hosted run 40830080420, 6 of 6 on 2026-10-29), and, for Modbus RTU over a TCP gateway, diagslave 3.6 as `-m enc -a 1` in the `interop-rtu` job (first hosted run 40838776577, 9 of 9, on 2026-10-29), and, for Modbus ASCII and Modbus RTU through a pseudo-terminal that socat bridges to the drivers' TCP gateway carrier, diagslave 3.6 as `-m ascii -a 1` in the `interop-ascii` job and as `-m rtu -a 1` in the `interop-rtu-pty` job (S60b, 9 of 9 each, passed locally on 2026-10-29; the first hosted runs are the next evidence) |
| `hardware-verified` | exercised against a physical device, with a record from `docs/hardware/TEMPLATE.md` |
| `planned` | not shipped; the owner is named where one is known (hardware evidence: milestone M2S) |
| `not supported` | not shipped and rejected with a typed error rather than ignored |

Nothing is `hardware-verified`. No physical device, adapter or PLC has been tried, and no
hardware record exists.

## Protocols and operations

| Protocol | Operation | Feature | Level | Parity row | Evidence |
| --- | --- | --- | --- | --- | --- |
| Modbus TCP | connect, close, connection state | `modbus` | interoperability-tested | `MB-TCP-CONNECT` | `interop` job; `interop_contract_suite_passes_against_pymodbus`; libmodbus: `interop_reads_the_input_bits_and_the_input_register` |
| Modbus TCP | read coils (FC01) | `modbus` | interoperability-tested | `MB-TCP-READ-COIL` | `interop_reads_coils_1_to_10`; libmodbus: `interop_reads_coils_and_holding_registers_at_rest` |
| Modbus TCP | read discrete inputs (FC02) | `modbus` | interoperability-tested | `MB-TCP-READ-DI` | `interop_reads_discrete_inputs_1_to_10`; libmodbus: `interop_reads_the_input_bits_and_the_input_register` |
| Modbus TCP | read holding registers (FC03) | `modbus` | interoperability-tested | `MB-TCP-READ-HR` | `interop_reads_all_fifteen_holding_register_types`, `interop_little_endian_region_decodes_real`; libmodbus: `interop_reads_coils_and_holding_registers_at_rest`, `interop_writes_the_unit_test_values_and_reads_them_back` |
| Modbus TCP | read input registers (FC04) | `modbus` | interoperability-tested | `MB-TCP-READ-IR` | `interop_reads_input_registers_1_to_7`; libmodbus: `interop_reads_the_input_bits_and_the_input_register` |
| Modbus TCP | batched reads, adjacent merge, `split_into` disclosure | `modbus` | interoperability-tested | `MB-READ-MERGE`, `MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS` | `interop_split_batch_discloses_chunks`, `interop_125_register_and_2000_coil_reads` |
| Modbus TCP | per-chunk device exception 02 (illegal data address) | `modbus` | interoperability-tested | `MB-EXC-02` | `interop_read_beyond_map_yields_device_exception` |
| Modbus TCP | device exception 06 (server device busy) | `modbus` | interoperability-tested | `MB-EXC-06` | libmodbus 3.1.11 sends it at wire address 0x170: `interop_exception_06_is_a_typed_device_exception_and_the_session_goes_on` |
| Modbus TCP | other device exception codes (01, 03 to 05, 07 to 0B) | `modbus` | simulator-tested | `MB-EXC-01`, `MB-EXC-03` to `MB-EXC-05`, `MB-EXC-07` to `MB-EXC-0B` | unit tests and `FakeTransport`; never seen from an independent server |
| Modbus TCP | reconnect with capped jittered backoff (background when opened in a Tokio runtime, else request-driven) | `modbus` | simulator-tested | `MB-READ-RECONNECT` | `FakeTransport` and fake clock; Stonecrab extension, upstream has none |
| Modbus TCP | read retry, cancellation, timeout, late-reply discard, resync | `modbus` | simulator-tested | `MB-READ-RETRY`, `MB-READ-TIMEOUT`, `MB-READ-MATCH` | `FakeTransport` and fake clock (D21); the timeout and the late-reply discard are also in the next row |
| Modbus TCP | timeout and late-reply discard over the system clock, a reply with another transaction id and unit id discarded, a reply sent one byte at a time | `modbus` | interoperability-tested | `MB-READ-TIMEOUT`, `MB-READ-MATCH`, `MB-READ-FRAMING` | libmodbus 3.1.11 at wire addresses 0x171 to 0x173: `interop_a_reply_with_the_wrong_transaction_and_unit_times_out_and_the_session_goes_on`, `interop_a_reply_after_500_ms_is_read_within_a_2_s_deadline`, `interop_a_reply_after_500_ms_misses_a_200_ms_deadline_and_is_discarded_late`, `interop_a_reply_sent_a_byte_at_a_time_is_reassembled` |
| Modbus TCP | failure paths on a real clock: a reset, a black hole, latency below and above the deadline, sliced frames, a link closed after N bytes or inside a header, a disabled proxy, a paused peer, a killed and restarted peer; each ends in a typed error (`Timeout` at the deadline, `Disconnected`) or a read, and the next request reconnects or reuses the connection | `modbus` | interoperability-tested | `MB-READ-TIMEOUT`, `MB-READ-RECONNECT`, `MB-READ-FRAMING`, `MB-TCP-CONNECT` | Toxiproxy 2.13.0 between the driver and pymodbus and libmodbus, `docker pause`, `docker kill` and `docker start` on pymodbus (job `interop-toxiproxy`): `interop_toxi_a_reset_on_the_path_is_disconnected_and_the_next_request_reconnects`, `interop_toxi_a_black_hole_times_out_at_the_deadline_and_the_link_recovers`, `interop_toxi_latency_below_the_deadline_still_answers`, `interop_toxi_latency_above_the_deadline_times_out_and_the_late_reply_is_discarded`, `interop_toxi_sliced_frames_are_reassembled`, `interop_toxi_limit_data_closes_the_connection_after_the_bytes`, `interop_toxi_a_frame_cut_inside_its_header_is_disconnected`, `interop_toxi_disabling_the_proxy_closes_every_connection_and_enabling_it_reconnects`, `interop_toxi_a_paused_peer_times_out_and_answers_late`, `interop_toxi_a_killed_peer_is_disconnected_and_the_driver_returns_when_it_restarts`; the backoff values are checked only on the fake clock (row above); tshark finds no malformed packet in the capture |
| Modbus TCP | a reply whose byte count does not fit the request | `modbus` | interoperability-tested | `MB-READ-MATCH` | libmodbus 3.1.11 answers any read of two holding registers with one: each member of the chunk fails with `MalformedData` and the reply PDU as received in the raw bytes of its status; `interop_a_short_reply_to_a_two_register_read_is_malformed_data` |
| Modbus TCP | unit-ID correlation: a separate map per unit on one connection, tags of several units in one batch, the unit in the address block over `default-unit-identifier`, silence from an unknown unit ending in `Timeout` at the deadline | `modbus` | interoperability-tested | `MB-READ-MATCH`, `MB-READ-TIMEOUT`, `MB-READ-MERGE`, `MB-CFG-UNIT-ID`, `MB-ADDR-CONFIG-BLOCK` | first-party multi-unit server on modbus-tk 1.1.5 (units 1, 2 and 247; every other unit is silent), job `interop-modbus-tk`: `interop_each_unit_on_one_connection_answers_with_its_own_map`, `interop_one_batch_across_units_gets_each_tag_its_own_units_value`, `interop_requests_to_alternating_units_and_functions_get_the_right_answers`, `interop_an_unknown_unit_times_out_at_the_deadline_and_the_connection_goes_on`, `interop_a_batch_with_an_unknown_unit_ends_in_the_timeout`, `interop_the_unit_in_the_address_block_overrides_the_default_unit_identifier`; a first-party peer, so this level at most |
| Modbus TCP | write coils (FC15) and holding registers (FC06, FC16); FC05 is never sent | `modbus` | interoperability-tested | `MB-TCP-WRITE`, `MB-CFG-ACCESS`, `MB-WRITE-ACCESS`, `MB-WRITE-UNKNOWN`, `MB-WRITE-HALT`, `MB-WRITE-ECHO` | `Capabilities.write = true`; refused before any I/O unless the connection string says `access=read-write`; one request per item, as upstream; an echo is `Confirmed`, an exception `Rejected`, a request that went out without a usable answer `Unknown` and never replayed; pymodbus 3.8.6 via the `interop` job (`interop_writes_*`); the 29 upstream write exchanges Stonecrab can address replay byte for byte |
| Modbus TCP | subscriptions by polling: cyclic and change-of-state (1000 ms), event refused | `modbus` | simulator-tested | `MB-TCP-SUBSCRIBE` | `Capabilities.subscribe = Polling` (S55); `FakeTransport` and fake clock, the six subscription contract cases, `a_polling_read_and_a_user_read_never_run_together`; the event refusal and the close of a subscription also run against pymodbus 3.8.6 in the `interop` job's contract run, the timed cases do not (D21) |
| Modbus TCP | read and write `CHAR`, `WCHAR`, `STRING(n)` and `WSTRING(n)` on holding, input (read) and extended registers, alone and as packed lists | `modbus` | simulator-tested | `MB-TYPE-CHAR`, `MB-TYPE-WCHAR`, `MB-TYPE-STRING`, `MB-TYPE-WSTRING`, ADR-0025 | `FakeTransport` and fake clock (`tests/modbus_text.rs`), the upstream string tests of plc4j and plc4go transcribed; `interop_reads_and_writes_text_registers` against pymodbus 3.8.6 passed with `mise run interop` on 2026-10-29, not yet on the hosted runner |
| Any physical device | everything | none yet | planned | none | milestone M2S (board adapters, hardware runner, hardware records); no record exists |
| Modbus TCP | browse | `modbus` | not supported | `MB-TCP-BROWSE` | upstream has no Modbus browse |
| Modbus TCP | discover | `modbus` | not supported | `MB-TCP-DISCOVER` | deferred |
| Modbus TCP | ping | `modbus` | interoperability-tested | `MB-TCP-PING`, `MB-CFG-PING-ADDRESS`, `MB-QUIRK-D` | one read of `ping-address`; any correlated reply, an exception included, is success as upstream's; pymodbus 3.8.6 via the `interop` job |
| Modbus TCP | over UDP (`modbus-tcp:udp://host[:port]`, default port 502): connect (sends nothing), read, write, ping and polling over one socket connected to the device; a lost reply is `Timeout` at the deadline, a duplicated, late or reordered reply is discarded by transaction id, a datagram that is not one frame is `MalformedData` | `modbus` | interoperability-tested (job `interop-udp`, run 40830080420, 6 of 6); simulator-tested on loopback sockets | `MB-TCP-TRANSPORTS-UDP`, `MB-CFG-TRANSPORT-OPTIONS`, `MB-QUIRK-A`; `UTR-UDP`, `UTR-UDP-RING`, `UTR-UDP-OPTIONS` in [`parity/transport.md`](parity/transport.md) | S61, ADR-0027: the contract suite and the `modbus_udp` tests over loopback UDP; written for diagslave 3.6 with `tc netem` loss, delay, duplication and reordering (job `interop-udp`, first hosted run 40830080420, 6 of 6 on 2026-10-29): `interop_udp_writes_and_reads_back_registers_and_coils`, `interop_udp_netem_loss_of_every_reply_times_out_at_the_deadline`, `interop_udp_netem_delay_below_the_deadline_still_answers`, `interop_udp_netem_delay_past_the_deadline_times_out_and_the_late_reply_is_discarded`, `interop_udp_netem_duplicated_replies_are_discarded_by_correlation`, `interop_udp_netem_reordered_replies_never_answer_the_wrong_request`; no `udp.` option is offered |
| Modbus TCP | TLS, TLS-PSK, `test` transports | `modbus` | not supported | `MB-TCP-TRANSPORTS-TLS`, `MB-TCP-TRANSPORTS-TLS-PSK`, `MB-TCP-TRANSPORTS-TEST` | deferred; `InvalidConfig` |
| Modbus TCP | extended registers (FC20 read, FC21 write) | `modbus` | simulator-tested | `MB-ADDR-EXTENDED`, `MB-LIMIT-EXT`, `MB-QUIRK-B` | no interop peer serves FC20 or FC21; strict fake-transport scripts and the first-party responder |
| Modbus RTU | connect, close, read (coils, discrete inputs, holding and input registers, a range selection), write (coils, holding registers) and ping over RTU framing (CRC-16) on a serial-to-IP gateway (`modbus-rtu:tcp://host:port`), with the operations and limits of Modbus TCP | `modbus` | interoperability-tested | `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING`, `MBS-TRANS-TCP`, `MBS-RTU-CRC`, `MBS-RTU-WIRE-MODEL` in [`parity/modbus-serial.md`](parity/modbus-serial.md) | S59b, ADR-0024: written for diagslave 3.6 as Modbus RTU over TCP (`-m enc -a 1`, job `interop-rtu`, first hosted run 40838776577, 9 of 9, on 2026-10-29) and, since S60b, run again against `-m rtu -a 1` on a serial device behind socat (job `interop-rtu-pty`, 9 of 9 passed locally on 2026-10-29; the hosted run is the next evidence; diagslave cuts frames there by the gap between characters, `-f 20`): `interop_rtu_holding_registers_are_written_and_read_back`, `interop_rtu_input_registers_read_what_diagslave_aliases_to_the_holding_registers`, `interop_rtu_coils_are_written_and_read_back`, `interop_rtu_discrete_inputs_read_what_diagslave_aliases_to_the_coils`, `interop_rtu_a_range_selection_is_one_request_and_one_list_of_what_was_written`, `interop_rtu_the_longest_frames_are_framed_and_checked_each_way` (requests and replies of 255 bytes), `interop_rtu_ping_reads_the_ping_address`; every value read is the one written; diagslave closes the connection on a frame with a wrong CRC, so a CRC the driver got wrong would fail these tests; its input areas are aliases of its holding and coil areas; which of FC05/15 and FC06/16 the driver sends is not visible in its log and rests on `modbus_rtu.rs` |
| Modbus RTU | an exception from the slave (exception 02 for function codes 20 and 21, which diagslave refuses always) and a unit the slave does not serve (`-m enc`: diagslave closes the connection, `Disconnected`, never a reply or silence; `-m rtu` on a serial device: silence, `Timeout`) | `modbus` | interoperability-tested | `MBS-BUS-EXCEPTION`, `MBS-ADDR-UNIT-ID`, `MBS-OP-WRITE-EXT` | `interop_rtu_exception_02_is_a_typed_device_exception_and_the_session_goes_on` (`InvalidAddress`, status code 2, a rejected write, the connection still good), `interop_rtu_a_unit_the_slave_does_not_serve_is_a_closed_connection_not_a_reply` (`-m enc`), `interop_rtu_a_unit_the_slave_does_not_serve_is_silence` (`-m rtu` behind socat); the other exception codes are simulator-tested |
| Modbus RTU | connect, close, read, write, ping and subscribe by polling over RTU framing (CRC-16) on a serial port (`modbus-rtu://<port>`), and subscribe by polling on a gateway | `modbus` | simulator-tested | `MBS-OP-*`, `MBS-RTU-*`, `MBS-TRANS-SERIAL`, `MBS-OP-SUBSCRIBE-POLL` in [`parity/modbus-serial.md`](parity/modbus-serial.md) | S59, ADR-0024: `FakeTransport` and fake clock, the contract suite (`the_suite_is_green_against_modbus_rtu_over_the_fake_transport`), the upstream RTU parser vectors, `fuzz_modbus_rtu`, RTU over a loopback socket; no independent peer on a serial line (a virtual serial pair with libmodbus or modbus-tk is a later slice) and no hardware (HW2) |
| Modbus RTU | CRC errors, the silence before a request (t3.5, `serial.interframe-delay`), the drain of a late reply after a timeout, a lost serial link and its reopen, replies from another unit or function | `modbus` | simulator-tested | `MBS-RTU-CRC`, `MBS-RTU-SILENCE`, `MBS-BUS-LATE-REPLY`, `MBS-BUS-CORRELATION` | `a_reply_with_a_bad_crc_fails_the_read_and_the_next_read_is_answered`, `serial_write_waits_for_the_interframe_gap`, `serial_late_reply_after_timeout_does_not_answer_the_next_request`, `a_lost_serial_link_is_disconnected_and_the_next_read_reopens_it`, `serial_response_from_another_unit_or_function_is_ignored` |
| Modbus RTU | broadcast (unit 0), shared serial ports, TLS, TLS-PSK, UDP and `test` carriers | `modbus` | not supported | `MBS-CFG-UNIT-ID`, `MBS-BUS-SHARED-PORT`, `MBS-TRANS-TLS`, `MBS-TRANS-UDP`, `MBS-TRANS-TEST` | `InvalidConfig` (or `InvalidAddress` for a tag addressed to unit 0) |
| Modbus ASCII | connect, close, read, write, ping and subscribe by polling over ASCII framing (a colon, the address, the PDU and an LRC as hex, CR LF) on a serial port (`modbus-ascii://<port>`, 8 or 7 data bits) or a serial-to-IP gateway (`modbus-ascii:tcp://`), with the operations and limits of Modbus TCP | `modbus` | simulator-tested | `MBS-OP-*`, `MBS-ASCII-*`, `MBS-LIMIT-ASCII-BUFFER`, `MBS-TRANS-SERIAL`, `MBS-TRANS-TCP` in [`parity/modbus-serial.md`](parity/modbus-serial.md) | S60, ADR-0024 addendum: `FakeTransport` and fake clock, the contract suite (`the_suite_is_green_against_modbus_ascii_over_the_fake_transport`), the upstream ASCII parser suite, codec test and Go codec test vectors, `fuzz_modbus_ascii`, ASCII over a loopback socket; on a serial port of the driver no independent peer yet (diagslave is reached through the gateway carrier only, see the next rows) and no hardware (HW2) |
| Modbus ASCII | connect, close, read (coils, discrete inputs, holding and input registers, a range selection), write (coils, holding registers) and ping over ASCII framing (a colon, hex, LRC, CR LF) on a serial-to-IP gateway (`modbus-ascii:tcp://host:port`), with the operations and limits of Modbus TCP | `modbus` | interoperability-tested | `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING`, `MBS-TRANS-TCP`, `MBS-ASCII-SEND`, `MBS-ASCII-LRC`, `MBS-ASCII-WIRE-MODEL` in [`parity/modbus-serial.md`](parity/modbus-serial.md) | S60b, ADR-0024: written for diagslave 3.6 as a Modbus ASCII slave (`-m ascii -a 1 -b 19200 -d 7 -s 1 -p even`) on a pseudo-terminal that socat bridges to a TCP port (job `interop-ascii`, first hosted run 40838776577, 9 of 9, on 2026-10-29): `interop_ascii_holding_registers_are_written_and_read_back`, `interop_ascii_input_registers_read_what_diagslave_aliases_to_the_holding_registers`, `interop_ascii_coils_are_written_and_read_back`, `interop_ascii_discrete_inputs_read_what_diagslave_aliases_to_the_coils`, `interop_ascii_a_range_selection_is_one_request_and_one_list_of_what_was_written`, `interop_ascii_the_longest_frames_are_framed_and_checked_each_way` (a request and a reply of 511 characters), `interop_ascii_ping_reads_the_ping_address`; every value read is the one written; diagslave answers no request with a wrong LRC or in lower case (seen in a probe, not in a test), so an LRC or a case the driver got wrong would fail these tests; the serial port of the driver, its line settings and a device are not exercised (the carrier is `tcp`) |
| Modbus ASCII | an exception from the slave (exception 02 for function codes 20 and 21, which diagslave refuses always) and a unit the slave does not serve (diagslave is silent: `Timeout`) | `modbus` | interoperability-tested | `MBS-BUS-EXCEPTION`, `MBS-ADDR-UNIT-ID`, `MBS-OP-WRITE-EXT` | `interop_ascii_exception_02_is_a_typed_device_exception_and_the_session_goes_on` (`InvalidAddress`, status code 2, a rejected write, the connection still good), `interop_ascii_a_unit_the_slave_does_not_serve_is_silence`; the other exception codes are simulator-tested |
| Modbus ASCII | LRC errors, bad digits, a missing CR or LF, noise and broken frames before a colon (skipped to the next colon), the size bound (513 bytes), no character timeout, the drain of a late reply after a timeout, a lost serial link and its reopen, replies from another unit or function | `modbus` | simulator-tested | `MBS-ASCII-LRC`, `MBS-ASCII-PARSE`, `MBS-ASCII-RESYNC`, `MBS-ASCII-TIMING`, `MBS-LIMIT-ASCII-BUFFER`, `MBS-BUS-LATE-REPLY`, `MBS-BUS-CORRELATION` | `a_reply_with_a_bad_lrc_fails_the_read_and_the_next_read_is_answered`, `noise_and_broken_frames_in_front_of_a_good_frame_go_with_it`, `slow_characters_are_accepted_until_the_request_deadline`, `a_late_reply_after_a_timeout_does_not_answer_the_next_request`, `ascii_lost_serial_link_is_disconnected_and_the_next_read_reopens_it`, `ascii_response_from_another_unit_or_function_is_ignored` |
| Modbus ASCII | broadcast (unit 0), shared serial ports, 5 and 6 data bits, TLS, TLS-PSK, UDP and `test` carriers | `modbus` | not supported | `MBS-CFG-UNIT-ID`, `MBS-BUS-SHARED-PORT`, `MBS-CFG-SERIAL-DATA-BITS`, `MBS-TRANS-TLS`, `MBS-TRANS-UDP`, `MBS-TRANS-TEST` | `InvalidConfig` (or `InvalidAddress` for a tag addressed to unit 0) |
| Simulator (`sim`) | connect, read, write, scripted failures, bounded read retry, polling subscriptions (S55; on the default `RealtimeClock` the interval is disclosed but not paced, and change-of-state is refused) | `sim` | simulator-tested | `SIM-DRIVER`, `SIM-CONNECTION`, `SIM-FAILURE-SCRIPT`, `SIM-SUBSCRIBE` | driver contract suite (29 passed, 4 named skips since S55) |
| Connection cache (`ConnectionCache`, `LeasedConnection`) | leases by connection string over a `DriverRegistry`, one at a time with waiters in arrival order bounded by `max_pending_requests`; the lease, wait, idle, ping and close bounds on the injected clock (upstream's defaults: 1 min, 30 s, 5 min, 30 s threshold with a 5 s ping, 5 s); replacement after a failed ping or a `Disconnected` or `Timeout` through a lease; `remove`, `close`, `len`, `active_leases`, `connection_strings` | none (core) | simulator-tested | `TOOL-CC-CACHE`, `TOOL-CC-LEASE`, `TOOL-CC-QUEUE`, `TOOL-CC-CLOSED`, `TOOL-CC-IDLE-PING`, `TOOL-CC-TIMEOUTS`, `TOOL-CC-BUILDER`, `TOOL-CC-ADMIN`, `TOOL-CC-OPERATIONS` in [`parity/tools.md`](parity/tools.md) | S68, ADR-0030: `core_connection_cache` (23 cases on the fake clock against the simulator, and against Modbus TCP over the fake transport for the ping rule); subscription restore (`TOOL-CC-RESTORE`) is deferred |
| Event pump (`EventPump`, `TagBatch`, `BatchEvent`) | named batches of tags, each one `read` per fetch on a borrowed connection or on a lease from the connection cache taken per fetch; an interval with an initial delay on the injected clock, a tick during a running fetch skipped and counted (no burst), a pause of 1 s doubling to 60 s after failed fetches; `add_batch`, `remove_batch`, `start_batch`, `stop_batch`, `start_all`, `stop_all`, `is_started`, `close`; `recv` drives every started batch with no spawned task and holds at most one event per batch | none (core) | simulator-tested | `TOOL-EP-PUMP`, `TOOL-EP-TIMER`, `TOOL-EP-READ`, `TOOL-EP-MULTI`, `TOOL-EP-OPERATIONS` in [`parity/tools.md`](parity/tools.md) | S69, ADR-0031: `core_event_pump` (16 cases on the fake clock against the simulator, and against Modbus TCP over the fake transport for the wire); the subscription trigger, configuration files and value transformations (`TOOL-EP-SUBSCRIPTION`, `-CONFIG`, `-TRANSFORM`) are deferred |
| Sensor profiles | load, validate, build tags, scale and grade measurements | `sensors`, `sim` | simulator-tested | `SENSORS-*` | `sim` profiles through `profile_read` in the `examples` job |
| Sensor profiles | the same, with the `modbus-tcp` synthetic profiles | `sensors`, `modbus` | interoperability-tested | `SENSORS-SYNTHETIC-PROFILES` | `profile_read --expect-good` in the `interop` job; green on the hosted runner for S30 (the `profiles` step, run 40548582722) |
| OPC UA, EtherNet/IP, S7, other baseline drivers | everything | none yet | planned | one record per driver, listed in the matrix below | milestones M3 to M5 |
| Direct bench adapters: MAX31855 over SPI | frame decode (thermocouple and cold-junction temperature, open, short-to-ground and short-to-Vcc flags, negative readings, refused reserved-bit, inconsistent and all-zeros or all-ones frames), `Max31855::read` over `SpiBus`, the three measurements (`Bad` thermocouple with no value under a fault) | `boards` | simulator-tested | none (Stonecrab extension, no upstream row; ADR-0026) | scripted `FakeSpiBus` on a fake clock and property tests of the decode against a reference decoder (`boards_max31855`, `boards_properties`); no converter and no SPI controller took part; runbook [`hardware/max31855-spi.md`](hardware/max31855-spi.md), no record |
| Direct bench adapters: Linux SPI backend | `LinuxSpiDev`: a `/dev/spidev` node read with `read(2)` on a worker thread, deadline-bounded, no dependency, no `unsafe`; cannot set the bus mode or speed | `adapters-linux` | simulator-tested | none (ADR-0026) | in-memory readers and temporary files (`boards_spidev`); no real node has been read |
| Direct bench adapters: ADXL345 over I2C | `Adxl345` over `I2cBus`: identity check (DEVID, a wrong one is `Trust`), the configuration written by `init` alone (standby, FIFO bypass, zero offsets, range and full resolution, rate 3200 Hz to 6.25 Hz, FIFO mode, measure), the sample decode at every range and resolution with counts outside the resolution refused, `read_sample` (only a sample `DATA_READY` flags as new, otherwise `None`) and its three measurements in `[g]`, `read_fifo` into a `SampleBlock` (interval, axes, gapless sequence, the device's overrun flag, a full FIFO or one that filled during the drain by the host's clock, and the host's estimate of lost samples), a failed or dropped read ending the stream until `init` | `boards` | simulator-tested | none (Stonecrab extension, no upstream row; ADR-0029) | scripted `FakeI2cBus` on a fake clock (NACK, bus errors, stalls, silence, short replies) and property tests and an exhaustive pair sweep of the decode against a reference decoder (`boards_adxl345`, `boards_adxl345_properties`); bus-mock tested, no accelerometer and no I2C controller took part; runbook [`hardware/adxl345-i2c.md`](hardware/adxl345-i2c.md), no record |
| Direct bench adapters: Linux I2C backend | `LinuxI2cDev` on `/dev/i2c-N` | none yet | planned | none (ADR-0029 decision 3) | deferred: `i2c-dev` needs an `ioctl`, which needs first-party `unsafe` or a new dependency; an application supplies its own `I2cBus` until then |
| Direct bench adapters: ADXL345 over SPI | everything | none yet | planned | none | SPEC.md "SPI as a follow-on"; needs a transmitting `SpiBus` method (ADR-0026 decision 2) |

The driver boundary itself (`DriverRegistry`, `Driver`, `Connection`, configuration,
address options, cancellation, the state watch, diagnostics) is `simulator-tested` through
the common driver contract suite and `interoperability-tested` on a wire through the
Modbus driver (`CORE-DRIVER-TRAIT`, `CORE-CONNECTION-TRAIT`, `CORE-REGISTRY`).

## Upstream drivers, transports, tools and integrations

This is the release matrix of slice S43: every upstream protocol driver, transport, tool
and integration of the pinned baseline by name, with Stonecrab's support level today and a
pointer to its parity record and milestone, so that no upstream capability is a hidden
omission. `not supported` means not shipped; a driver that names a transport or option that
is not shipped is refused with a typed error (`InvalidConfig`), never ignored. A row's
milestone is the plan's, from [`POST_ALPHA_PLAN.md`](POST_ALPHA_PLAN.md) section 4. Every
row of the records named below is `planned` or `deferred` (none is `implemented`), except
where this table says otherwise. Counts of rows per record are in the records themselves.

### Protocol drivers

| Upstream driver (code) | Stonecrab support | Level | Parity record | Milestone |
| --- | --- | --- | --- | --- |
| Modbus TCP (`modbus-tcp`) | `modbus` feature | see the operations table above | [`parity/modbus.md`](parity/modbus.md) | M1 (read path and ping shipped); writes, polling subscriptions (S55), extended registers, UDP (S61) and TLS per the table above |
| Modbus RTU (`modbus-rtu`) | `modbus` feature, `ModbusRtuDriver` | simulator-tested; interoperability-tested over a TCP gateway (jobs `interop-rtu`, S59b, and `interop-rtu-pty`, S60b; see the operations table above) | [`parity/modbus-serial.md`](parity/modbus-serial.md) | M2: shipped in S59; an interop peer on a serial port of the driver and HW2 to come |
| Modbus ASCII (`modbus-ascii`) | `modbus` feature, `ModbusAsciiDriver` | simulator-tested; interoperability-tested over a TCP gateway (job `interop-ascii`, S60b; see the operations table above) | [`parity/modbus-serial.md`](parity/modbus-serial.md) | M2: shipped in S60; an interop peer on a serial port of the driver and HW2 to come |
| OPC UA (`opcua`) | none | not supported | [`parity/opcua.md`](parity/opcua.md) | M3 |
| S7 (`s7`) | none | not supported | [`parity/s7.md`](parity/s7.md) | M4 |
| EtherNet/IP (`eip`), Logix (`logix`) | none | not supported | [`parity/eip.md`](parity/eip.md) | M4 |
| Beckhoff ADS/AMS (`ads`) | none | not supported | [`parity/ads.md`](parity/ads.md) | M5 |
| Mitsubishi SLMP (`slmp`) | none | not supported | [`parity/slmp.md`](parity/slmp.md) | M5 |
| Schneider UMAS (`umas`) | none | not supported | [`parity/umas.md`](parity/umas.md) | M5 |
| PROFINET (`profinet`, legacy and next-generation models) | none | not supported | [`parity/profinet.md`](parity/profinet.md) | M5 |
| Allen-Bradley Ethernet (`ab-eth`) | none | not supported | [`parity/ab-eth.md`](parity/ab-eth.md) | M5 |
| BACnet/IP (`bacnet-ip`) | none | not supported | [`parity/bacnet.md`](parity/bacnet.md) | M5 for the Java behavior (passive COV subscribe); the Go read, write and discovery are non-Java backlog |
| KNXnet/IP (`knxnet-ip`) | none | not supported | [`parity/knxnetip.md`](parity/knxnetip.md) | M5 (corrected in S43: a working Java driver exists; the Go and C# extras stay in the backlog) |
| C-Bus (`c-bus`) | none | not supported | [`parity/cbus.md`](parity/cbus.md) | non-Java backlog, no milestone assigned |
| CANopen (`canopen`) | none | not supported | [`parity/canopen.md`](parity/canopen.md) | M5 |
| Generic CAN (`genericcan`) | none | not supported | [`parity/can.md`](parity/can.md) | M5 |
| IEC 60870-5-104 (`iec-60870-5-104`) | none | not supported | [`parity/iec-60870.md`](parity/iec-60870.md) | M5 |
| Firmata (`firmata`) | none | not supported | [`parity/firmata.md`](parity/firmata.md) | M5 |
| Open Protocol (`open-protocol`) | none | not supported | [`parity/open-protocol.md`](parity/open-protocol.md) | deferred: an upstream placeholder, not an obligation |
| ctrlX Data Layer (`ctrlx`) | none | not supported | [`parity/ctrlx.md`](parity/ctrlx.md) | M5 for ping and connect; the rest is an upstream placeholder |
| DF1 (`df1`, a wire specification and not a driver in any language) | none | not supported | [`parity/open-protocol.md`](parity/open-protocol.md), section "Specification-only protocols" | deferred: recorded so the omission is visible |
| PLC4X proxy client (`plc4x`) | none | not supported | [`parity/plc4x-proxy.md`](parity/plc4x-proxy.md) | M5 |
| Simulated (`simulated`), mock (`mock`) | the first-party simulator, `sim://` | simulator-tested | [`parity/sim.md`](parity/sim.md) | Stonecrab-owned; no upstream parity claim, the upstream test doubles are not ported |

### Transports

| Upstream transport (code) | Stonecrab support | Level | Parity record | Milestone |
| --- | --- | --- | --- | --- |
| TCP (`tcp`) | `TcpFactory` | interoperability-tested through Modbus TCP | [`parity/transport.md`](parity/transport.md): `TRANSPORT-TCP-CONNECT`, `UTR-TCP`, `UTR-TCP-OPTIONS` | shipped; the upstream option names are planned for M5 |
| UDP (`udp`) | `UdpFactory`, `transport` feature (on through `modbus`) | simulator-tested (loopback sockets, alone and under Modbus TCP over UDP); interoperability-tested through Modbus TCP over UDP (job `interop-udp`, diagslave 3.6 with `tc netem`, first hosted run 40830080420, 6 of 6 on 2026-10-29) | [`parity/transport.md`](parity/transport.md): `UTR-UDP`, `UTR-UDP-OPTIONS`, `UTR-UDP-RING`, `UTR-UDP-SHARED` | shipped (S61, ADR-0027); no upstream option is offered, no shared socket |
| TLS (`tls`), TLS-PSK (`tls-psk`) | none | not supported | [`parity/transport.md`](parity/transport.md): `UTR-TLS`, `UTR-TLS-PSK` | M3 |
| COTP (`cotp`) | none | not supported | [`parity/transport.md`](parity/transport.md): `UTR-COTP`; wire rows in [`parity/s7.md`](parity/s7.md) | M4 with S7 |
| Serial (`serial`) | `SerialFactory`, `serial` feature (on through `modbus` since S59) | simulator-tested (a virtual pseudo-terminal pair on Linux, S58; through Modbus RTU on the in-memory transport, S59) | [`parity/transport.md`](parity/transport.md): `UTR-SERIAL`, `TRANSPORT-SERIAL-CONFIG`, `TRANSPORT-SERIAL-WORKER` | shipped (S58); no hardware |
| Raw socket (`raw-socket`) | none | not supported | [`parity/transport.md`](parity/transport.md): `UTR-RAW-SOCKET` | M5 |
| PCAP replay (`pcap-replay`) | none | not supported | [`parity/transport.md`](parity/transport.md): `UTR-PCAP-REPLAY` | M5 |
| SocketCAN (`can-socketcan`), Virtual CAN (`can-virtualcan`); the CAN support library is not a transport | none | not supported | [`parity/transport.md`](parity/transport.md): `UTR-CAN-LIB`, `UTR-CAN-SOCKETCAN`, `UTR-CAN-VIRTUALCAN`; [`parity/can.md`](parity/can.md) | M5 |
| Test (`test`) | `FakeTransport` and the loopback `ScriptedPeer` in the `testkit` feature | simulator-tested; the code `test` is refused with `InvalidConfig` | [`parity/transport.md`](parity/transport.md): `UTR-TEST` | first-party equivalent, no port |

### Tools

Decision E6: PLC4X tools count toward parity, starting with the connection cache and
Object PLC Mapping (milestone M5, "in the order E6 sets").

| Upstream tool | Stonecrab support | Level | Parity record | Milestone |
| --- | --- | --- | --- | --- |
| Connection cache | `stonecrab::ConnectionCache` (S68, ADR-0030), all but subscription restore | simulator-tested; `TOOL-CC-RESTORE` deferred | [`parity/tools.md`](parity/tools.md): `TOOL-CC-*` | M5, first (landed in S68) |
| Object PLC Mapping (OPM) | none | not supported | [`parity/tools.md`](parity/tools.md): `TOOL-OPM-*` | M5, second, as a derive macro |
| Event pump | `stonecrab::EventPump` (S69, ADR-0031), the timer batch reader | simulator-tested; subscription trigger, configuration files and transformers deferred | [`parity/tools.md`](parity/tools.md): `TOOL-EP-*` | M5 for the timer batch reader (landed in S69); file formats and transformers deferred |
| Capture replay | none | not supported | [`parity/tools.md`](parity/tools.md): `TOOL-CR-*` | deferred, after the first three tools |
| Driver test-suite runner and parser-serializer runners (`test-utils`) | the Modbus subset through the S04b `fixtures` adapter | simulator-tested for the Modbus vectors only | [`parity/tools.md`](parity/tools.md): `TOOL-TU-*` | M3 and M5 |
| OPC UA server, PLC4X server, `plc4xbrowser`, `plc4xpcapanalyzer` (plc4x-extras, outside the pinned baseline) | none | not supported | [`parity/tools.md`](parity/tools.md): `TOOL-X-*` | deferred (outside the pinned baseline) |

### Integrations

The tracked integrations list of decision E6, in [`parity/integrations.md`](parity/integrations.md).
Each is built on demand; none is planned for a milestone.

| Integration | Where it lives | Level | Parity record |
| --- | --- | --- | --- |
| Apache Calcite | plc4x-extras `v1.0.0` | not supported | `INTEG-CALCITE` |
| Apache Camel | the Apache Camel repository, `components/camel-plc4x` | not supported | `INTEG-CAMEL` |
| Apache IoTDB | an example application in plc4x-extras, not a connector | not supported | `INTEG-IOTDB` |
| Apache Kafka Connect | plc4x-extras `v1.0.0` | not supported | `INTEG-KAFKA` |
| Apache NiFi | plc4x-extras `v1.0.0` | not supported | `INTEG-NIFI` |
| Apache StreamPipes | the StreamPipes repository (pins PLC4X 0.13.1) | not supported | `INTEG-STREAMPIPES` |
| Eclipse Ditto | a third-party example repository only | not supported | `INTEG-DITTO` |

Karaf, which the upstream README names, and the Scraper utility were not found in the
pinned baseline or in plc4x-extras `v1.0.0`; they are rows `INTEG-KARAF` and `TOOL-X-SCRAPER`.

## Data types

Registers are 16 bits. A multi-register value is laid out by the byte-order mode below.
`ModbusDataType` has the 15 numeric and bit types and, since S49, the four text types
(ADR-0025); one element per tag, or a list through a selection.

| Modbus area | Function code | Types accepted | Values |
| --- | --- | --- | --- |
| `coil` (`0x`) | 01 | `BOOL` only | `Bool` |
| `discrete-input` (`1x`) | 02 | `BOOL` only | `Bool` |
| `holding-register` (`4x`) | 03 | all 19 | see below |
| `input-register` (`3x`) | 04 | all 19 | see below |

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
| `CHAR` | 1 (the first byte; two per register in a list) | `Char`, one byte of UTF-8 (U+0000 to U+007F) |
| `WCHAR` | 1 | `Char`, one UTF-16BE code unit |
| `STRING(n)` | `ceil(n / 2)` | `String`, `n` bytes of UTF-8 up to the first NUL |
| `WSTRING(n)` | `n` | `String`, `n` UTF-16BE code units up to the first NUL |

Text (`MB-TYPE-CHAR` to `MB-TYPE-WSTRING`, ADR-0025): `STRING` and `WSTRING` need their
length, `STRING(20)`, 1 to 5 digits, and a string must fit one read (`STRING(250)`,
`WSTRING(125)`). Text is bytes in register order; the two bytes of each register are
swapped only under `BIG_ENDIAN_BYTE_SWAP` and `LITTLE_ENDIAN_BYTE_SWAP`, as upstream reads
strings. Bytes that are not UTF-8 or UTF-16 fail the item with `MalformedData` and the
bytes kept, where upstream substitutes U+FFFD; trailing spaces are kept. A write is padded
with NUL bytes to the declared length and refused, with nothing sent, when it does not fit
or holds a NUL.

Not supported, and rejected at parse time with `InvalidAddress`: a type other than `BOOL`
on a bit area (`MB-ADDR-BIT-TYPES`), the `TIME`, `LTIME`, `DATE`, `LDATE`,
`TIME_OF_DAY`, `LTIME_OF_DAY`, `DATE_AND_TIME` and `LDATE_AND_TIME` names
(`MB-TYPE-TIME`). The extended registers (`extended-register`, `6x`, FC20 and FC21) take
the 19 types of the holding registers (`MB-ADDR-EXTENDED`). An array selection, `[i]` or
`[lo..hi]` with an optional `;base` between the number and the type, is accepted on every
area, the extended registers included (`MB-ADDR-SELECTION`, ADR-0022 and its addendum of
2026-10-29): an index is the element's own address and reads a scalar, a range reads and
writes one `PlcValue::List`; `CHAR`s pack two to a register and strings back to back, and
a selection that starts inside a register (`[1]:CHAR`) is refused, as upstream refuses it.
Core has `List` (ADR-0022) and the calendar value variants (ADR-0021),
which no Modbus type maps to yet, and no `Bytes`, `Struct` or `Raw` variant (ADR-0006;
`Bytes` waits for a type that needs it, ADR-0025 decision 9).

Address text: the long form `holding-register:16:INT` and the prefixed short form
`4x00016:INT` are always accepted; the bare short forms (`40001`) are accepted only when
the caller selects an `address-convention` (profiles declare it explicitly). The
logical number is one-based and the wire address is one less, except under
`address-convention=zero-based`, where the number is the wire address
(`MB-ADDR-CONVENTIONS`). See the grammar in the
[Modbus component document](../crates/stonecrab/docs/components/modbus.md).

## Byte-order modes

| `default-payload-byte-order` and `byte-order` block | `ByteOrder` | `WordOrder` | `0x01020304` on the wire | Evidence |
| --- | --- | --- | --- | --- |
| `BIG_ENDIAN` (default) | `BigEndian` | `HighWordFirst` | `[1,2,3,4]` | `MB-BYTEORDER-BE`: first-party vectors; pymodbus map; upstream PLC4X vectors (`automated-verified`) |
| `LITTLE_ENDIAN` | `LittleEndian` | `LowWordFirst` | `[4,3,2,1]` | `MB-BYTEORDER-LE`: first-party vectors; one REAL read from pymodbus; upstream PLC4X vectors (`automated-verified`) |
| `BIG_ENDIAN_BYTE_SWAP` | `LittleEndian` | `HighWordFirst` | `[2,1,4,3]` | `MB-BYTEORDER-BE-SWAP`: first-party vectors |
| `LITTLE_ENDIAN_BYTE_SWAP` | `BigEndian` | `LowWordFirst` | `[3,4,1,2]` | `MB-BYTEORDER-LE-SWAP`: first-party vectors |

The mapping is derived from the documented layouts. `MB-BYTEORDER-BE` and
`MB-BYTEORDER-LE` are `automated-verified`: the retained upstream PLC4X read exchanges
(`DriverTestsuite.xml` and the two all-datatypes files, S04b) decode to the values the XML
cases report. The two byte-swap rows are `implemented`, not `automated-verified`, because
no upstream file configures a swapped mode. Precedence: the tag's own order, then the
address block, then the connection default; the order is never inferred from the values.

## Configuration keys

Connection string: `modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]` (port 502
by default, IPv6 in brackets, `tcp` the only transport code), then `?key=value&...`. A key
may appear once and every other key is `InvalidConfig` (upstream warns and ignores it).

| Key | Default | Accepted | Parity row |
| --- | --- | --- | --- |
| `request-timeout-ms` | 5000 | whole milliseconds above 0; also `Timeouts.request` | `MB-CFG-REQUEST-TIMEOUT` |
| `default-unit-identifier` | 1 | 0 to 255 | `MB-CFG-UNIT-ID` |
| `ping-address` | `4x00001:BOOL` | an address of the grammar, read under `plc4x`; the address `ping` reads once | `MB-CFG-PING-ADDRESS` |
| `default-payload-byte-order` | `BIG_ENDIAN` | `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`, `LITTLE_ENDIAN_BYTE_SWAP` | `MB-CFG-BYTE-ORDER` |
| `max-coils-per-request` | 2000 | 1 to 2000 | `MB-CFG-MAX-COILS` |
| `max-registers-per-request` | 125 | 1 to 125 | `MB-CFG-MAX-REGISTERS` |
| `address-convention` | none | `plc4x`, `one-based-40001`, `zero-based` | `MB-ADDR-BARE-40001`, `MB-ADDR-CONVENTIONS` |

Simulator: `sim://<device>?latency-ms=<n>&seed=<u64>`; `address-convention` is accepted
and ignored. Core connection defaults (`Timeouts` 5 s connect and 5 s request, `idle`
enforced only with a supervisor (S57); `ReconnectPolicy` 250 ms initial, 30 s cap, multiplier 2.0, jitter 0.2,
unlimited attempts; `RetryPolicy` one attempt, retrying `Timeout`, `Disconnected` and
`Backpressure`; `AccessPolicy::ReadOnly`) are rows `CORE-CONFIG-SHAPE`,
`CORE-RECONNECT-POLICY` and `CORE-RETRY-POLICY`.

## Peers it was verified against

| Peer | What ran | Where | Level it supports |
| --- | --- | --- | --- |
| pymodbus 3.8.6 on `python:3.12-slim` (pinned by digest), first-party `server.py` over the register map in `fixtures/modbus/interop/register-map.toml` and the text registers of `text-registers.toml` beside it (S49), TCP port 5020 | sixteen `interop_` tests (writes of a holding register, a coil, the multi-register types and the text types, read back, included), the `modbus_tcp_read` example, `profile_read --expect-good` for each `modbus-tcp` synthetic profile, the common driver contract suite (17 passed, 16 skipped by name) | the CI job `interop` (ubuntu, every push and pull request) and `mise run interop` | `interoperability-tested` |
| libmodbus 3.1.11 `unit-test-server` (C, Debian trixie package 3.1.11-2 on `debian:trixie-slim`, pinned by digest), built from the test source the package ships, TCP port 1502, restarted after each connection | ten `interop_` tests: the input bits and input register of `unit-test.h`, coils and holding registers written and read back (FC15, FC06, FC16, a REAL in the four word orders), exception 06, a reply with the wrong transaction id and unit id, a 500 ms delayed reply inside and beyond a deadline, a reply sent one byte at a time, and a short reply to a two-register read | the CI job `interop-libmodbus` (ubuntu, every push and pull request) and `mise run interop` | `interoperability-tested` |
| modbus-tk 1.1.5 (LGPL-2.1-or-later) running the first-party `interop/modbus-tk/server.py` on `python:3.12-slim` (pinned by digest), units 1, 2 and 247 with a map each, TCP port 5030 | eight `interop_` tests: each unit's map on one connection, one batch across the three units, alternating units and function codes, a unit with a shorter map refusing an address (exception 02), an unknown unit (9, 255) timing out at the deadline with the connection still good, a batch with an unknown unit, the address block's unit over `default-unit-identifier`, and a write to one unit | the CI job `interop-modbus-tk` (ubuntu, every push and pull request) and `mise run interop` | `interoperability-tested` (the server is first-party code, so no named device profile rests on it) |
| Toxiproxy 2.13.0 (MIT, pinned by digest) in front of pymodbus and libmodbus, `docker pause`, `docker kill` and `docker start` on the pymodbus container, and tshark 4.4 (Debian trixie) decoding the capture | ten `interop_toxi_` tests on a real clock (the failure paths of the row above); the capture of the run, taken in the Toxiproxy container's network namespace, holds no malformed packet, every driver request is one well-formed Modbus/TCP frame and every peer segment is Modbus/TCP | the CI job `interop-toxiproxy` (ubuntu, every push and pull request) and `mise run interop-toxiproxy` | `interoperability-tested` (the faults are injected; the peers are the same) |
| diagslave 3.6 (proconX freeware, proprietary; downloaded and checked by SHA-256 in the image build, never redistributed) run as `-m enc -p 1503 -a 1` (Modbus RTU over TCP, unit 1), on `debian:trixie-slim` (pinned by digest), TCP port 1503, areas in memory with the input areas aliased to the holding and coil areas | nine `interop_rtu_` tests over `modbus-rtu:tcp://`: every area written and read back, a range selection, a write of 123 and a read of 125 registers (255-byte frames), exception 02 for the file-record functions (read and write) with the connection still good, a unit it does not serve (it closes the connection), and ping | the CI job `interop-rtu` (ubuntu, every push and pull request; first hosted run 40838776577, 9 of 9) and `mise run interop-rtu` (and `mise run interop`); 9 of 9 passed locally on colima, 2026-10-29 | `interoperability-tested` (RTU over a TCP gateway; no serial line; the server is closed-source freeware) |
| diagslave 3.6 (proconX freeware, proprietary; as above) run as `-m ascii -a 1 -b 19200 -d 7 -s 1 -p even` on a pseudo-terminal that socat 1.8.0.3-1+deb13u1 (pinned by exact version) makes and bridges to TCP port 1504, one connection at a time, on the same pinned `debian:trixie-slim` | nine `interop_ascii_` tests over `modbus-ascii:tcp://`: every area written and read back, a range selection, a write of 123 and a read of 125 registers (511-character frames), exception 02 for the file-record functions with the connection still good, a unit it does not serve (it is silent), and ping | the CI job `interop-ascii` (ubuntu, every push and pull request; first hosted run 40838776577, 9 of 9) and `mise run interop-ascii` (and `mise run interop`); 9 of 9 passed locally on colima, 2026-10-29 | `interoperability-tested` (ASCII over a TCP gateway behind a pseudo-terminal; no serial port of the driver; the server is closed-source freeware) |
| diagslave 3.6 as above run as `-m rtu -a 1 -b 19200 -d 8 -s 1 -p none -f 20` on the same kind of pseudo-terminal behind socat, TCP port 1505 | the nine `interop_rtu_` tests again, with `interop_rtu_a_unit_the_slave_does_not_serve_is_silence` in place of the closed-connection one, over `modbus-rtu:tcp://` | the CI job `interop-rtu-pty` (ubuntu, every push and pull request; first hosted run 40838776577, 9 of 9) and `mise run interop-rtu-pty` (and `mise run interop`); 9 of 9 passed locally on colima, 2026-10-29 | `interoperability-tested` (RTU over a TCP gateway behind a pseudo-terminal; no serial port of the driver; the server is closed-source freeware) |
| the in-process simulator (`sim`) | the driver contract suite (29 passed, 4 skipped), `sim_read`, `profile_read` on the `sim` profiles | `cargo test --all-features`, CI jobs `test` and `examples` | `simulator-tested` |
| loopback `ScriptedPeer` (testkit) | a holding-register read over real sockets and the contract suite (22 passed, 11 skipped) | `cargo test --all-features` | `simulator-tested` |
| `FakeTransport` with a first-party MBAP responder and `FakeClock` | the contract suite (30 passed, 3 skipped), timing, retry, reconnect, cancellation and write-outcome tests | `cargo test --all-features` | `simulator-tested` |

pymodbus 3.8.6 is the peer the upstream project's own Docker test uses; the Stonecrab server
is written independently and reproduces only the documented fixture values plus one
little-endian region. libmodbus is a C stack that shares no code with Stonecrab or pymodbus;
its `unit-test-server` serves one connection and exits, so the image restarts it and the
tests run one at a time. The modbus-tk peer is a short script of ours over the modbus-tk
1.1.5 library: three units with a map each, and silence for any other. What the plain runs
do not verify: dropped links (pymodbus has no fault hooks and libmodbus has none for links;
the `interop-toxiproxy` run injects them between the driver and the peers, and a paused and
a killed pymodbus), a third-party server that refuses a unit id (pymodbus and libmodbus
answer the id they are sent), and any other Modbus server. Timing over a
real clock is verified for a 500 ms reply, a timeout and a late reply against libmodbus,
and, with Toxiproxy, for a black hole and for latency above and below a deadline against
both peers; a paused and a killed peer are tried on pymodbus only. Toxiproxy and
`docker pause` leave the peer's own behavior untouched, so a peer that misbehaves on its own
(a device that resets under load) is not covered. No physical device has been tried.

## Platforms and toolchains

| Platform | Toolchains | What CI does | Level |
| --- | --- | --- | --- |
| Linux x86_64 (`ubuntu-latest`) | 1.97.1 and 1.99.1 | build, nextest, `cargo test` with doctests; the quality, coverage, interop, interop-libmodbus, interop-modbus-tk, interop-toxiproxy, examples and fuzz-smoke jobs | `build-tested`; Modbus rows as above |
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
`grep -rn "covers: SPEC mandatory-failure-cases / F3" crates`. The numbering is the
order of the SPEC bullets. Counts are the number of tagged tests at S31.

| Tag | SPEC case | Alpha coverage | Tagged tests |
| --- | --- | --- | --- |
| F1 | truncated headers and payloads, oversized lengths, malformed discriminators, unexpected function codes (invalid RTU checksums since S59, see below) | codec, PDU codec, MBAP framer, connection strings, profile loader | 50 |
| F2 | TCP fragmentation and coalescing, peer close at each framing boundary, wrong transaction and unit ids, delayed replies after a timeout | transport, framing, session | 71 |
| F3 | device exception, invalid register, unsupported capability, mixed success within a batch | simulator, Modbus connection, session | 69 |
| F4 | cancellation while connecting, queued, transmitting and awaiting a response | guard, simulator, Modbus connection; the queued case is exercised on `ConnectionGuard` only, not on the simulator, which has no wait queue | 44 |
| F5 | device restart, outage, recovery, exhausted retry budget, shutdown during reconnect | guard, Modbus connection, `FakeTransport` | 32 |
| F6 | queue exhaustion, slow consumer, dropped events, stale last-known values, timestamp absence | queue exhaustion (`Backpressure`) and timestamp absence at the alpha; since then the subscription queue (S54: slow consumer, dropped events) and polling subscriptions (S55: skipped ticks counted, no burst after a stall); stale values wait for the stale tracker (S56) | 4 |
| F7 | negative and scaled values, swapped words, integer boundaries, counter rollover, NaN and infinity, incompatible profile version | typed decoder, scaling pipeline, profile loader | 36 |
| F8 | RTU: CRC errors, timing boundaries, serial disconnect and reopen, wrong unit replies, and a late reply after a timeout | RTU framer, RTU connection over the fake transport and fake clock (S59), tagged `RTU ...` rather than `F8` | 5 |
| F10 | direct sensors: open and short thermocouple flags, invalid frames, SPI and I2C failures and timeouts, unavailable device, invalid identity, FIFO overflow (SPEC "Direct sensor extension") | MAX31855 frame decode, `Max31855`, `LinuxSpiDev` (S63); `Adxl345` and its sample decode over the scripted I2C bus (S64) | 34 |

The RTU cases of SPEC.md (F8: CRC errors, timing boundaries, serial disconnect and
reopen, wrong unit replies) are covered since S59 by the tests of
`crates/stonecrab/tests/modbus_rtu.rs` tagged `// covers: SPEC mandatory-failure-cases /
RTU ...`, on the in-memory transport and the fake clock. The OPC UA cases (F9) are out of
scope for the alpha. The direct-sensor case (F10, the last bullet of SPEC.md) has been
covered since S63 for the MAX31855 over SPI (its flags, SPI failures and unavailable
device) and since S64 for the ADXL345 over I2C (I2C failures and timeouts, a device that
does not acknowledge, an invalid identity, invalid samples and FIFO levels, and the FIFO
overflow with its loss count, including a FIFO that fills during a stalled drain), all on
scripted buses. The F10 count is as of S64; the
decode tests of S63 and S64 are also tagged F7, and the F7 count above was not recomputed.

## Divergences from upstream

Every divergence has a row, an upstream reference at `file:line` of the pinned commit, a
protocol-standard section or an upstream-issue note, and a regression test, in the
"Divergences from upstream" table of [`parity/modbus.md`](parity/modbus.md). Upstream
issues for quirks a, b, c, e, f and g are drafted in `docs/evidence/upstream-issues/` and
not filed (the cell says "drafted, not filed"); the other rows say that no issue was drafted
and why.

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
| `MB-TYPE-TIME` | the calendar names are typed `InvalidAddress` rejections |
| `MB-TYPE-STRING`, `MB-TYPE-CHAR` | text that is not UTF-8 or UTF-16 is an item error, not U+FFFD; trailing spaces and U+FEFF are kept and a NUL `CHAR` is `'\0'`; a lone `CHAR` and an odd-length string are written padded to a whole register, where upstream's write fails its own length check; the byte-swap modes swap whole registers, also for the last byte of an odd-length value |
| `MB-ADDR-SELECTION` | every element of a range takes the registers it takes alone (no packing of 8-bit and `BOOL` elements), `[4..4]` is a list of one, and a list wider than one write PDU is refused with no request |
| `MB-ADDR-EXTENDED`, `MB-LIMIT-EXT` | an extended read is at most 120 registers, a selection there included (upstream allows 125, whose answer no device may send), every FC20 sub-response is used, an FC21 write is confirmed only by the echo of the whole request, and the last extended address is 655349999 (upstream's 655359999 would name file 65536) |
| `MB-READ-GAP` | no gap reads: only adjacent or overlapping tags merge (decided S51: a hole in a device map would fail the whole merged request) |
| `MB-LIMIT-ADU` | the MBAP length is bounded (2..=254, ADU at most 260 bytes) before the body is read |
| `MB-QUIRK-A`, `MB-QUIRK-B`, `MB-QUIRK-C` | the bogus `UDPTransport` import, the split FC20 second-item loss and the dead FC05 branch are not ported; `modbus-tcp:udp` takes the declared default port 502 where upstream fails for want of a port (S61) |
| `MB-QUIRK-D` | ported on purpose: `ping` reports success for any reply that answers it, an exception included, as upstream's `onPing` does |
| `MB-QUIRK-F` | the undeclared `unit-identifier` of the upstream test suite is an error |
| `MB-QUIRK-G` | `4x:INT` is a typed `InvalidAddress`, not a `NumberFormatException` (inferred) |
| `MB-QUIRK-I` | no polling receive loop |
| `MB-QUIRK-L` | byte order takes part in address equality |

Other documented differences are not divergences from a defect: reconnect
(`MB-READ-RECONNECT`) and read retry (`MB-READ-RETRY`) have no upstream equivalent,
`canonical()` renders the long form (`MB-ADDR-SHORT-4X`), the last register at wire address
65535 is reachable (`MB-LIMIT-SPAN`), and the address-block grammar is stricter about quoted
byte-order names (`MB-ADDR-CONFIG-BLOCK`). Inferences about upstream behavior are marked
`INFERENCE` in the parity record and listed under its UNVERIFIED section.

## Known gaps

Calendar types, a serial port of the driver against an independent peer (diagslave is reached
through the gateway carrier, S59b and S60b), TLS, and hardware verification are listed above; Modbus subscriptions
poll since S55, extended registers are read and written since S50 and text types since
S49. Also not done: a mutation threshold as a gate (one recorded run,
`docs/evidence/alpha.md`), soak and benchmark gates, branch-coverage gating (line
coverage per module is gated, ADR-0005), a differential test against PLC4J, and a write
whose outcome is lost on demand against an independent server (the fault is injected on
the fake transport and over loopback, not against pymodbus).
[`STATUS.md`](../STATUS.md) carries the current list.
