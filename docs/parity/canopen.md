# CANopen parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The CANopen driver is planned for milestone M5
(docs/POST_ALPHA_PLAN.md section 4: "CANopen, generic CAN, Firmata, IEC 60870"). It rides
on the CAN transports recorded in `docs/parity/can.md`. Every row below is `planned` or
`deferred`; no row names a test that exists, and `scripts/parity-check.sh` binds only
`implemented` and `automated-verified` rows.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one planned test, prefixed `planned:`; none exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now. The reason is a
  non-Java-only capability, an upstream placeholder, or a decision. Test ids may be
  `none`.

Later statuses (`implemented`, `automated-verified`, `hardware-verified`) follow
docs/SPEC.md "Compatibility baseline". Device models and firmware are recorded in the
Test ids column once a hardware test exists; none does.

## File key
Citations use the manifest key under `[inventory.canopen]` (and its non-Java entries,
none exist for CANopen); paths are relative to the apache/plc4x repository root.

| Key | Path |
| --- | --- |
| driver_caps | `plc4j/drivers/canopen/src/main/java/org/apache/plc4x/java/canopen/CANOpenDriver.java` |
| read, write, subscribe, dispatch | `.../canopen/CANOpenConnection.java` |
| tag_sdo, tag_pdo, tag_nmt_hb | `.../canopen/tag/CANOpenSDOTag.java`, `CANOpenPDOTag.java`, `CANOpenNMTTag.java`, `CANOpenHeartbeatTag.java` |
| Cfg | `.../canopen/configuration/CANOpenConfiguration.java` |
| Mspec | `protocols/canopen/src/main/resources/protocols/can/canopen.mspec` |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-OP-READ | SDO upload only, a single tag per request; a tag that is not a `CANOpenSDOTag` fails the future with IllegalArgumentException; errors map to REMOTE_ERROR (`read`, CANOpenConnection.java:336-370). | Read of one SDO tag through an SDO upload conversation; any other tag form is a typed per-tag error instead of a failed future. | planned (M5) | `planned:canopen_read_of_an_sdo_tag_uploads_the_object`, `planned:canopen_read_of_a_pdo_tag_is_refused` |
| CANOPEN-OP-WRITE-SDO | SDO download via `SDODownloadConversation`, one tag per request (`write`, CANOpenConnection.java:376-445). | SDO download of one tag. | planned (M5) | `planned:canopen_write_of_an_sdo_tag_downloads_the_object` |
| CANOPEN-OP-WRITE-PDO | PDO push: serialises the value and sends a `CANOpenFrame` (`write`, CANOpenConnection.java:376-445). | PDO frame send with the value serialised per the tag data type. | planned (M5) | `planned:canopen_write_of_a_pdo_tag_sends_one_frame` |
| CANOPEN-OP-WRITE-NMT-HB | NMT and heartbeat tags are rejected on write (`write`; operations.write). | Per-tag refusal of NMT and heartbeat tags on write. | planned (M5) | `planned:canopen_write_of_an_nmt_tag_is_refused` |
| CANOPEN-OP-SUBSCRIBE | Native push, no emulation: only `PlcSubscriptionType.EVENT`, others UNSUPPORTED; tags PDO, NMT and HEARTBEAT; any other tag INVALID_ADDRESS (`subscribe`, CANOpenConnection.java:451-495). | Event subscriptions for PDO, NMT and heartbeat tags; cyclic and change-of-state refused UNSUPPORTED; SDO tags refused INVALID_ADDRESS. | planned (M5) | `planned:canopen_subscribe_to_a_pdo_tag_delivers_each_frame`, `planned:canopen_cyclic_subscription_is_unsupported` |
| CANOPEN-OP-UNSUBSCRIBE | Unsubscribe is a no-op because CAN is push-only (`subscribe`). | Unsubscribe drops the local handle; nothing goes on the bus. | planned (M5) | `planned:canopen_unsubscribe_sends_nothing_on_the_bus` |
| CANOPEN-OP-DISPATCH | Incoming frames reach SDO listeners first, then PDO, heartbeat and NMT publication; frames whose node id equals the configured `node-id` (own echo) are skipped (`dispatch`, CANOpenConnection.java:248-276; notes). | Same dispatch order and own-echo filter. | planned (M5) | `planned:canopen_frame_from_the_own_node_id_is_ignored` |
| CANOPEN-OP-CONNECT | Default transport `can-socketcan`; connect starts the receive path; heartbeat sending on connect only partly read (`driver_caps`; notes UNCERTAIN). | Connect over a CAN transport; heartbeat sending follows `CANOPEN-CFG-HEARTBEAT`. | planned (M5) | `planned:canopen_connect_over_virtualcan_starts_the_receive_path` |
| CANOPEN-OP-CLOSE | Not separately inventoried; the base connection stops receiving. | Close releases the transport and fails pending SDO requests. | planned (M5) | `planned:canopen_close_fails_pending_sdo_requests` |
| CANOPEN-OP-BROWSE | Not supported; default `onBrowse` fails "Browse not supported" (operations.browse). | None. | deferred (upstream has no browse) | none |
| CANOPEN-OP-DISCOVER | Not supported; `canDiscover` default false (operations.discover). | None. | deferred (upstream has no discovery) | none |
| CANOPEN-OP-PING | Not supported; `canPing` default false (operations.ping). | None. | deferred (upstream has no ping) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-CAP-FLAGS | `canRead`, `canWrite` and `canSubscribe` true; browse, discover and ping false (`driver_caps`, CANOpenDriver.java:35-73). | `Capabilities` report read, write and subscribe true and the rest false. | planned (M5) | `planned:canopen_capabilities_match_upstream_flags` |

## Configuration

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-CFG-NODE-ID | `node-id`, int, no default; "CAN node identifier. Depending on used CAN version it might be 11 or 29 bit unsigned int." (Cfg:27-41). | Required; range checked against the 11-bit or 29-bit identifier space. | planned (M5) | `planned:canopen_config_node_id_out_of_range_is_invalid` |
| CANOPEN-CFG-HEARTBEAT | `heartbeat`, boolean, default false; "Forces PLC4X to send CANopen heartbeat (NMT) messages to the bus." (Cfg:27-41). | Same option and default. | planned (M5) | `planned:canopen_config_heartbeat_defaults_to_false` |
| CANOPEN-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, default 1000; time after which a dispatched bus operation such as an SDO request fails (Cfg:27-41). | Positive integer, default 1000, applied to each SDO request. | planned (M5) | `planned:canopen_config_request_timeout_defaults_to_1000` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-TRANSPORT-SOCKETCAN | Default transport `can-socketcan` (`inventory.canopen.transports`). | Supported when the SocketCAN transport lands, see `CAN-SOCKETCAN-TRANSPORT`. | planned (M5) | `planned:canopen_default_transport_is_can_socketcan` |
| CANOPEN-TRANSPORT-VIRTUALCAN | `can-virtualcan` supported (`inventory.canopen.transports`). | Supported; the test bed for the driver, see `CAN-VIRTUALCAN-TRANSPORT`. | planned (M5) | `planned:canopen_connect_over_virtualcan_starts_the_receive_path` |
| CANOPEN-TRANSPORT-TEST | `test` supported (`inventory.canopen.transports`). | Stonecrab tests use `FakeTransport`; no `test` transport code. | deferred (no release) | none |

## Address grammar
Tags are matched by regular expressions in the tag classes. `NODE_PATTERN` is a decimal
node id.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-ADDR-SDO | `SDO:node[/answerNode]:index/subindex:TYPE[n]`; `answerNode` is the optional reply node, default the node (`tag_sdo`, CANOpenSDOTag.java:33). | Same grammar, parsed into a typed SDO address. | planned (M5) | `planned:canopen_sdo_address_with_answer_node_parses` |
| CANOPEN-ADDR-PDO | `RECEIVE_PDO_1..4` or `TRANSMIT_PDO_1..4` `:node:TYPE[n]`; the array count is one digit (`tag_pdo`, CANOpenPDOTag.java:34). | Same grammar. | planned (M5) | `planned:canopen_pdo_address_parses_all_eight_services` |
| CANOPEN-ADDR-NMT | `NMT` or `NMT:node` (`tag_nmt_hb`, CANOpenNMTTag.java:33). | Same grammar. | planned (M5) | `planned:canopen_nmt_address_without_node_parses` |
| CANOPEN-ADDR-HEARTBEAT | `HEARTBEAT` or `HEARTBEAT:node` (`tag_nmt_hb`, CANOpenHeartbeatTag.java:33). | Same grammar. | planned (M5) | `planned:canopen_heartbeat_address_with_node_parses` |

## Data types
The set is the `CANOpenDataType` enum of the wire model (Mspec:270-350, read from the
checkout; the manifest cites the tag files only).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-TYPE-INTEGERS | `BOOLEAN`; `UNSIGNED8/16/24/32/40/48/56/64`; `INTEGER8/16/24/32/40/48/56/64`, mapped to the PLC4X USINT/UINT/UDINT/ULINT and SINT/INT/DINT/LINT families (Mspec). | All widths, including the odd 24, 40, 48 and 56 bit forms. | planned (M5) | `planned:canopen_unsigned24_decodes_little_endian`, `planned:canopen_integer40_sign_extends` |
| CANOPEN-TYPE-REAL | `REAL32`, `REAL64` (Mspec). | Little-endian IEEE 754 decode and encode. | planned (M5) | `planned:canopen_real32_round_trips` |
| CANOPEN-TYPE-STRINGS | `OCTET_STRING`, `VISIBLE_STRING`, `UNICODE_STRING`, `RECORD` (Mspec). | Strings by SDO segment transfer; `RECORD` as a byte list. | planned (M5) | `planned:canopen_visible_string_read_reassembles_segments` |
| CANOPEN-TYPE-TIME | `TIME_DIFFERENCE` (48 bit) in the model (Mspec). | Not cited by the tag classes; follow-up once a vector exists. | deferred (no cited tag use) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| CANOPEN-QUIRK-OWN-ECHO | Frames carrying the configured node id are dropped, so a second device configured with the same id is invisible. | `dispatch` | planned (M5) | Ported as `CANOPEN-OP-DISPATCH`; `planned:canopen_frame_from_the_own_node_id_is_ignored` |
| CANOPEN-QUIRK-ARRAY-DIGIT | PDO array count is a single digit in the regex. | `tag_pdo` | planned (M5) | Ported as written; `planned:canopen_pdo_array_count_above_nine_is_invalid` |

## Divergences from upstream

| Row id | Divergence | Rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-DIV-READ-ERRORS | Wrong tag kind on read becomes a typed per-tag error, not a failed future. | Stonecrab's `Driver` contract reports errors per tag. | planned (M5) | `planned:canopen_read_of_a_pdo_tag_is_refused` |

## Backlog from other languages
`[[backlog]]` has no row for CANopen: no Go, C, C# or Python implementation exists.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CANOPEN-BACKLOG-NONE | No non-Java CANopen driver in the manifest. | None. | deferred (nothing to port) | none |

## UNVERIFIED
- Heartbeat sending behavior on connect: seen only in part (CANOpenConnection.java
  170-185, manifest note).
- SDO expedited versus segmented transfer rules and abort-code mapping: not inventoried.
- Whether `RECORD` and `TIME_DIFFERENCE` are reachable from any tag form.
- Extended (29-bit) identifier handling, see the open question in `docs/parity/can.md`.
