# Generic CAN and CAN transports parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The generic CAN driver and the three CAN
entries (`transport_can`, `transport_can_socketcan`, `transport_can_virtualcan`) are
planned for milestone M5 (docs/POST_ALPHA_PLAN.md section 4, "transports (CAN, SocketCAN,
raw socket, PCAP replay)"). CANopen is recorded separately in `docs/parity/canopen.md`
and uses the transports below. Every row is `planned` or `deferred`; no listed test
exists.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: not ported now (non-Java-only capability, upstream placeholder,
  or a decision). Test ids may be `none`.

Later statuses follow docs/SPEC.md "Compatibility baseline"; device models and firmware
are recorded with hardware-verified test ids, none of which exist.

## File key
Citations use the manifest key under `[inventory.genericcan]`, `[inventory.transport_can]`,
`[inventory.transport_can_socketcan]` or `[inventory.transport_can_virtualcan]`.

| Key | Path |
| --- | --- |
| driver_caps | `plc4j/drivers/can/src/main/java/org/apache/plc4x/java/can/generic/GenericCANDriver.java` |
| write, subscribe, dispatch | `.../can/generic/GenericCANConnection.java` |
| tag | `.../can/generic/tag/GenericCANTag.java` |
| Mspec | `protocols/genericcan/src/main/resources/protocols/can/genericcan.mspec` |
| TC: config, frame, filter | `plc4j/transports/can/src/main/java/org/apache/plc4x/java/transport/can/` `config/CanTransportConfiguration.java`, `CanFrame.java`, `CanIdFilter.java` |
| SC: code, config, open, rx, tx, linux_only | `plc4j/transports/can-socketcan/src/main/java/org/apache/plc4x/java/transport/can/socketcan/` `SocketCanTransport.java`, `config/SocketCanTransportConfiguration.java`, `SocketCanTransportInstance.java` |
| VC: code, config, bus, instance, write, rx | `plc4j/transports/can-virtualcan/src/main/java/org/apache/plc4x/java/transport/can/virtualcan/` `VirtualCanTransport.java`, `config/VirtualCanTransportConfiguration.java`, `VirtualCanBusManager.java`, `VirtualCanTransportInstance.java` |

## Operations (generic CAN)

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-OP-READ | Not supported; no `onRead` override, the base fails "Read not supported" (operations.read). | None. | deferred (upstream has no read) | none |
| GCAN-OP-WRITE | Tags are grouped per CAN node id into one 8-byte frame; a tag that fails serialisation discards that node's frame; a non-`GenericCANTag` gets UNSUPPORTED (`write`, GenericCANConnection.java:182-243). | Same grouping, with the failed node reported per tag. | planned (M5) | `planned:gcan_write_packs_tags_of_one_node_into_one_frame`, `planned:gcan_write_with_a_bad_tag_drops_that_nodes_frame` |
| GCAN-OP-SUBSCRIBE | Native push, EVENT only (others UNSUPPORTED); one handle per node id; every incoming frame with a matching node id is decoded per tag and delivered (`subscribe`, GenericCANConnection.java:249-290; `dispatch`, 151-176). | Event subscriptions per node id. | planned (M5) | `planned:gcan_subscribe_delivers_frames_of_the_matching_node`, `planned:gcan_cyclic_subscription_is_unsupported` |
| GCAN-OP-UNSUBSCRIBE | No-op (`subscribe`). | Local handle drop only. | planned (M5) | `planned:gcan_unsubscribe_sends_nothing_on_the_bus` |
| GCAN-OP-CONNECT | Default transport `can-socketcan` (`driver_caps`, GenericCANDriver.java:40-75). | Connect over a CAN transport. | planned (M5) | `planned:gcan_connect_over_virtualcan_succeeds` |
| GCAN-OP-BROWSE | Not supported (operations.browse). | None. | deferred (upstream has no browse) | none |
| GCAN-OP-DISCOVER | Not supported (operations.discover). | None. | deferred (upstream has no discovery) | none |
| GCAN-OP-PING | Not supported (operations.ping). | None. | deferred (upstream has no ping) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-CAP-FLAGS | `canSubscribe` and `canWrite` only (`driver_caps`). | Capabilities report write and subscribe true, the rest false. | planned (M5) | `planned:gcan_capabilities_match_upstream_flags` |

## Configuration

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-CFG-LITTLE-ENDIAN | No user option: the payload is always little-endian, with unsigned-binary, two's-complement, IEEE 754 and UTF-8 encodings (`LITTLE_ENDIAN_OPTIONS`, GenericCANConnection.java:89-95, read from the checkout). | Fixed little-endian payload encoding. | planned (M5) | `planned:gcan_payload_is_little_endian` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-TRANSPORTS | Code `genericcan`; default `can-socketcan`; supported `can-socketcan`, `can-virtualcan`, `test` (`inventory.genericcan.transports`). The driver javadoc says it is transport-agnostic provided the transport yields fixed 16-byte SocketCAN-style frames (GenericCANDriver.java:33-39). | `can-socketcan` and `can-virtualcan`; the 16-byte record contract is stated in `CAN-RECORD-16` and any transport honoring it may be added. | planned (M5) | `planned:gcan_default_transport_is_can_socketcan` |
| GCAN-TRANSPORT-TEST | `test` transport. | Tests use `FakeTransport`. | deferred (no release) | none |

## Address grammar

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-ADDR-TAG | `nodeId:DATATYPE[arraySize]`; node id decimal, array size optional decimal (`tag`, GenericCANTag.java:36-37). | Same grammar. | planned (M5) | `planned:gcan_tag_with_array_size_parses`, `planned:gcan_tag_with_unknown_datatype_is_invalid` |

## Data types
The mspec carries a fixed-width table (16, 24, 32, 40, 48, 56, 64 bit and REAL) mapped to
PLC4X names (Mspec:24-85, read from the checkout).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-TYPE-STANDARD | `BYTE`, `BOOLEAN` (1 bit), `UNSIGNED8/16/32/64`, `INTEGER8/16/32/64`, `REAL32`, `REAL64`, `RAW` (Mspec:20-41). | Decode and encode each width. | planned (M5) | `planned:gcan_unsigned32_round_trips` |
| GCAN-TYPE-ODD-WIDTH | `UNSIGNED24/40/48/56` and `INTEGER24/40/48/56` map to `RAW_BYTE_ARRAY` or wider integers (Mspec:24-85). | Same mapping. | planned (M5) | `planned:gcan_integer24_decodes_to_a_byte_array_or_dint` |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-LIMIT-FRAME | One 8-byte frame per node per write (`write`). | A node whose tags exceed 8 bytes is a per-tag error. | planned (M5) | `planned:gcan_write_over_eight_bytes_is_invalid` |

## CAN support library (no transport of its own)

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CAN-LIB-NO-TRANSPORT | `plc4j/transports/can` has no Transport class and no services file; a connection string cannot name `can` as a transport (`transport_can` notes). | No `can` transport code. | planned (M5) | `planned:can_uri_with_transport_code_can_is_rejected` |
| CAN-LIB-FRAME | Immutable CAN 2.0 frame: standard id max 0x7FF, extended max 0x1FFFFFFF, data 0 to 8 bytes, extended and RTR flags (`frame`, CanFrame.java:19-60; `fd_none`, 40-42). | `CanFrame` value type with the same bounds. | planned (M5) | `planned:can_frame_rejects_nine_data_bytes`, `planned:can_frame_rejects_standard_id_above_0x7ff` |
| CAN-LIB-CANFD | CAN 2.0 only, no CAN-FD (`fd_none`). | No CAN-FD. | deferred (upstream has no CAN-FD) | none |
| CAN-LIB-FILTER | Explicit id set OR ranges; an empty filter accepts all (`filter`, CanIdFilter.java:47-135). | Same semantics. | planned (M5) | `planned:can_filter_with_no_rules_accepts_every_id`, `planned:can_filter_accepts_id_in_range` |
| CAN-LIB-CFG-FILTER-IDS | `filter-ids`: comma list, decimal or `0x` hex (`config`, CanTransportConfiguration.java:36-117). | Same list syntax. | planned (M5) | `planned:can_config_filter_ids_accepts_hex_and_decimal` |
| CAN-LIB-CFG-FILTER-RANGE | `filter-range-start` and `filter-range-end`, default -1 (unset); `buildFilter()` at 73-99. | Same options and default. | planned (M5) | `planned:can_config_filter_range_defaults_to_unset` |
| CAN-RECORD-16 | The byte stream exposed to drivers is a fixed 16-byte pseudo-frame: bytes 0-3 id big-endian, byte 4 data length, bytes 8-15 data (`rx`, SocketCanTransportInstance.java:187-205; `tx`, 287-322). | Stonecrab documents the record and feeds the drivers the same shape. | planned (M5) | `planned:can_record_16_encodes_id_length_and_data` |

## SocketCAN transport

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CAN-SOCKETCAN-TRANSPORT | Code `can-socketcan`, name "SocketCAN"; Linux only (constructor fails unless `os.name` contains linux), client only; one shared manager per transport object (`code`, SocketCanTransport.java:40-59; `linux_only`, 127-133). | A Linux-only raw CAN socket transport; other platforms refuse with a typed error. | planned (M5) | `planned:socketcan_transport_is_refused_off_linux` |
| CAN-SOCKETCAN-CFG-INTERFACE | `interface-name`, required (for example `can0`, `vcan0`) (`config`, SocketCanTransportConfiguration.java:35-63). | Required option. | planned (M5) | `planned:socketcan_config_without_interface_name_is_invalid` |
| CAN-SOCKETCAN-CFG-REUSE | `reuse-interface`, default false; a true value shares one handle (`config`; `open`, SocketCanTransportInstance.java:98-118). | Same option and default. | planned (M5) | `planned:socketcan_reuse_interface_shares_one_handle` |
| CAN-SOCKETCAN-CFG-READ-TIMEOUT | `read-timeout-ms`, default 1000, declared but never read by the instance (`config`; notes). | Accepted and ignored with a recorded note, or given a real meaning. | deferred (upstream declares it but never uses it) | none |
| CAN-SOCKETCAN-RX | Frames matching the ID filter become 16-byte records written to a 64 KiB ring (`rx`). | Same filtering and record shape. | planned (M5) | `planned:socketcan_rx_applies_the_id_filter` |
| CAN-SOCKETCAN-TX | Write expects at least one 16-byte record, uses id, dlc (max 8) and data, sends with flags 0, and accepts only the first record of the buffer (`tx`, 287-322). | Same send path; extra records are an error rather than silently dropped. | planned (M5) | `planned:socketcan_tx_sends_the_first_record`, `planned:socketcan_tx_with_dlc_above_eight_is_invalid` |
| CAN-SOCKETCAN-RING-DROP | The ring drops the newest bytes silently when the 64 KiB ring is full (notes). | Overflow is a reported error. | deferred (upstream defect, divergence planned) | none |
| CAN-SOCKETCAN-EXTENDED | Whether the kernel EFF flag in the id is preserved for extended frames on read and write was not settled from source (notes UNCERTAIN). | Decided against hardware or vcan tests. | deferred (uncertain upstream behavior) | none |

## Virtual CAN transport

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CAN-VIRTUALCAN-TRANSPORT | Code `can-virtualcan`, name "Virtual CAN", intended for testing; no hardware or OS support needed (`code`, VirtualCanTransport.java:30-49). | An in-process CAN bus transport. | planned (M5) | `planned:virtualcan_two_instances_on_one_bus_exchange_records` |
| CAN-VIRTUALCAN-CFG-BUS | `bus-name`, default `default` (`config`, VirtualCanTransportConfiguration.java:34-45). | Same option and default. | planned (M5) | `planned:virtualcan_buses_with_different_names_are_isolated` |
| CAN-VIRTUALCAN-BUS | The bus map is static, so only instances in the same JVM see each other; broadcast delivers synchronously on the sender's thread to every other instance; the sender does not hear itself; bytes are not interpreted (`bus`, VirtualCanBusManager.java:34-116; `write`, 155-181; notes). | Process-global bus keyed by name; sender excluded. | planned (M5) | `planned:virtualcan_sender_does_not_receive_its_own_record` |
| CAN-VIRTUALCAN-RING | 64 KiB ring per instance; overflow only warns (`instance`, 55-78; `rx`, 240-262). | Bounded buffer with a reported overflow. | planned (M5) | `planned:virtualcan_overflow_is_reported` |
| CAN-VIRTUALCAN-FILTER | The inherited filter options are accepted but never applied (notes). | Options accepted; not applied. | deferred (upstream never applies them) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| CAN-QUIRK-FRAME-FLAGS | Frames are always sent with flags 0 and the id is passed through, so extended and RTR flags are not set from this layer. | `tx`, notes | deferred (uncertain upstream behavior) | See `CAN-SOCKETCAN-EXTENDED`. |
| CAN-QUIRK-FILTER-ONLY-SOCKETCAN | Only SocketCAN applies the ID filter; Virtual CAN accepts the options and ignores them. | notes | deferred (upstream inconsistency) | See `CAN-VIRTUALCAN-FILTER`. |
| GCAN-QUIRK-DISCARD | A tag that fails serialisation discards the whole node frame, which silently drops sibling tags. | `write` | planned (M5) | Ported; `planned:gcan_write_with_a_bad_tag_drops_that_nodes_frame` |

## Divergences from upstream

| Row id | Divergence | Rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| CAN-DIV-RING-OVERFLOW | Overflow is an error, not a silent drop or warning. | A silent byte drop corrupts the 16-byte record alignment. | planned (M5) | `planned:virtualcan_overflow_is_reported` |

## Backlog from other languages
`[[backlog]]` has no row for generic CAN or the CAN transports; there is no Go, C, C# or
Python implementation.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| GCAN-BACKLOG-NONE | No non-Java implementation in the manifest. | None. | deferred (nothing to port) | none |

## UNVERIFIED
- Extended-frame EFF flag handling on both SocketCAN directions.
- Whether the generic driver sets or reads RTR frames.
- Real SocketCAN behavior: no vcan or hardware run has been made by this project.
