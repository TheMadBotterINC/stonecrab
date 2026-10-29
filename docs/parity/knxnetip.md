# KNXnet/IP parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The Java driver (`knxnet-ip`) is a full
implementation: read, write, push subscribe, ping, discovery and, with an ETS `knxproj`
file, browse. The Go driver (`go_knxnetip`) has the same set plus device-level access and
a browse that needs no ETS file. The C# module (`csharp_knxnetip`) holds generated wire
model files only and no driver; Python has no KNX. The plan first listed KNXnet/IP under
the "Non-Java backlog" milestone column, on a website lead that only C# had a driver.
Slice S41 showed that wrong, and S43 corrected the milestone to M5: a protocol with a
working Java driver belongs to M5, "the remaining protocols with their transports"
(`docs/POST_ALPHA_PLAN.md`, phases H to J and section 4). Every row that describes Java
behavior Stonecrab will port is therefore `planned (M5)` and names planned test ids. The
Go-only and C#-only capabilities stay in the non-Java backlog as
`deferred (non-Java backlog)`, each citing its `[[backlog]]` row where one exists. The
one Java limit that Stonecrab only documents, because the gateway enforces it, is
`deferred (documentation only; enforced by the gateway)`. Where a planned row rests on an
UNVERIFIED point (close, the Java side of acknowledging incoming tunneling requests,
out-of-range group address numbers), its test ids name the behavior the cited evidence
supports, and M5 settles the point by reading the source. Until M5 lands KNXnet/IP is not
supported; the gap is visible in the release matrix and is not hidden. Support levels in
the README and `docs/COMPATIBILITY.md` follow from these statuses.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. Here Mn is always M5,
  the milestone of the protocols that have a working Java driver (S43 corrected the plan's
  earlier "Non-Java backlog" for KNXnet/IP). Its Test ids cell names at least one test id
  prefixed `planned:`; no such test exists yet, and `scripts/parity-check.sh` does not
  look for it until the row becomes `implemented`.
- `deferred (<reason>)`: recorded so the omission is visible; Stonecrab does not port it
  now. `deferred (non-Java backlog)` is a Go-only or C#-only capability: it stays in the
  backlog and is not part of the M5 target. `deferred (upstream placeholder)` is an
  upstream stub or TODO, which is not an obligation (`docs/SPEC.md`, "Compatibility
  baseline"). `deferred (documentation only; enforced by the gateway)` is a Java limit
  that Stonecrab states in its documentation and leaves to the gateway. Test ids is `none`
  unless a rejection test is planned.
- `implemented`, `automated-verified`, `hardware-verified`: not used yet; see
  `docs/parity/modbus.md`.

No row is `implemented`, so no row id needs to appear in code yet. Row ids are never
renumbered.

Scope: the Java `knxnet-ip` driver, the Go `knxnetip` driver and the C# generated model.
The wire model (`knxnetip.mspec`) was listed but not opened for the inventory.

## File key
Citations use the manifest keys of `[inventory.knxnetip]` (Java, "K"),
`[inventory.go_knxnetip]` ("G") and `[inventory.csharp_knxnetip]` ("C#"). Paths are
relative to the apache/plc4x repository root; Java files sit under
`plc4j/drivers/knxnetip/src/main/java/org/apache/plc4x/java/knxnetip/` and Go files under
`plc4go/internal/knxnetip/`.

| Key | Entry and path |
| --- | --- |
| K driver | `KnxNetIpDriver.java:43-141` |
| K conn_doc, conn_connect, conn_ping, conn_write, conn_read, conn_browse, conn_subscribe | `KnxNetIpConnection.java:150-165`, `:265-330`, `:667-683`, `:689-712`, `:848-880`, `:1025-1060`, `:1181-1240` |
| K discoverer | `discovery/KnxNetIpPlcDiscoverer.java:61-110` |
| K config, tag | `configuration/KnxNetIpConfiguration.java:34-68`, `tag/KnxNetIpTag.java:33-70` |
| G driver, driver_conn, discovery_flag | `Driver.go:53`, `:62-96`, `:98-104` |
| G metadata, heartbeat, building_key, connect, keepalive, ping, builders | `Connection.go:50-94`, `:107-115`, `:241-246`, `:263-408`, `:415-448`, `:534-543`, `:559-592` |
| G incoming, options | `ConnectionHelper.go:61-140`, `:169-211` |
| G read, read_group, group_read | `Reader.go:57-171`, `:173-260`, `ConnectionDriverSpecificOperations.go:47-108` |
| G write | `Writer.go:79-121` |
| G sub_subscribe, sub_unsub, sub_types, sub_dispatch | `Subscriber.go:73-112`, `:114-117`, `:222-236`, `:241-336` |
| G browse_dev, browse_comobj | `Browser.go:67-156`, `:158-200` |
| G discover_consts, discover_flow, discover_scan, discover_item | `Discoverer.go:43-54`, `:79-195`, `:202-291`, `:295-330` |
| G tag_parse, query_parse, tunnel_ack | `TagHandler.go:83-179`, `:181-197`, `MessageCodec.go:132-168` |
| C# driver_dir_is_generated_only, csproj, generator_config, test_decode_only, no_socket_code | `plc4net/drivers/knxnetip/.../KnxDatapoint.cs:20-22`, `plc4net-driver-knxproj.csproj:22-45`, `plc4net/pom.xml:122-145`, `knxnetip-test/.../KnxDatapointTests.cs:26-39`, `plc4net/plc4net.sln:22-30` |

## Operations
Java is the baseline. Go and C# differences are in the Backlog section.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-OP-CONNECT | Java handshake: SearchRequest, then ConnectionRequest; the gateway must offer the tunneling service, otherwise connect fails (K conn_connect:265-330). A ConnectionStateRequest every 60 s keeps the tunnel open (K conn_doc:150-165; the 60 s value comes from the class comment, not code read). The class comment calls it a direct port of the legacy `KnxNetIpProtocolLogic`. | Connect performs the search, requires tunneling in the device information block, opens the tunnel and starts the heartbeat. | planned (M5) | `planned:knx_connect_sends_a_search_request_before_the_connection_request`, `planned:knx_connect_fails_when_the_gateway_does_not_offer_the_tunneling_service`, `planned:knx_connect_starts_the_heartbeat_once_the_tunnel_is_open` |
| KNX-OP-CLOSE | Not cited in the manifest (UNVERIFIED); the tunnel is closed with a DisconnectRequest by KNXnet/IP convention. | Close ends the heartbeat and the tunnel. | planned (M5) | `planned:knx_close_stops_the_heartbeat`, `planned:knx_close_ends_the_tunnel_with_a_disconnect_request` |
| KNX-OP-READ | `canRead` true. `onRead` reads tags one at a time and sends a group read over the tunnel. A timeout maps to NOT_FOUND (no device answered); other errors to INTERNAL_ERROR (K conn_read:848-880). The body of `readSingleTag` was not read. | Sequential group reads, same error mapping. | planned (M5) | `planned:knx_a_group_address_read_returns_the_decoded_value`, `planned:knx_a_read_timeout_is_reported_as_not_found`, `planned:knx_a_read_error_other_than_a_timeout_is_an_internal_error` |
| KNX-OP-WRITE | `canWrite` true. One tag per request: `onWrite` takes the first tag name only (K conn_write:689-712; G java_write_single:692-700). The write is wrapped in a TunnelingRequest and acknowledged by a TunnelingResponse matched on the sequence counter. | Single-tag write with sequence-matched acknowledgement. | planned (M5) | `planned:knx_a_group_address_write_is_sent_in_a_tunneling_request`, `planned:knx_a_write_waits_for_the_matching_tunneling_response` |
| KNX-OP-SUBSCRIBE | `canSubscribe` true; native push. The gateway forwards every LDataInd; the driver decodes them (optionally via the ETS model) and dispatches to consumers whose tag matches (K conn_subscribe:1181-1240). Not polling. | Push subscription with local tag matching. | planned (M5) | `planned:knx_a_push_subscription_delivers_a_matching_group_indication`, `planned:knx_a_push_subscription_ignores_a_non_matching_group_address` |
| KNX-OP-UNSUBSCRIBE | `onUnsubscribe` only stops matching; there is no wire unsubscribe (K conn_subscribe). | Same. | planned (M5) | `planned:knx_unsubscribe_stops_matching_without_a_wire_request` |
| KNX-OP-BROWSE | `canBrowse` true, but only when `knxproj-file-path` is configured; without an ETS model it fails with "Browse is not available - no knxproj-file-path configured." Unsupported query strings are reported as that query's answer (K conn_browse:1025-1060, 1051). | Browse from the parsed ETS project; same failure text without one. | planned (M5) | `planned:knx_browse_returns_the_items_of_the_parsed_ets_project`, `planned:knx_browse_without_a_knxproj_fails_with_the_documented_message`, `planned:knx_browse_reports_an_unsupported_query_as_the_answer_for_that_query` |
| KNX-OP-DISCOVER | `canDiscover` true; the driver is a `PlcDiscoverer` (K driver:43-141). See KNX-DISC-SEARCH. | Discovery by search request. | planned (M5) | `planned:knx_discover_reports_each_gateway_that_answers_the_search_request` |
| KNX-OP-PING | `canPing` true. `onPing` sends a ConnectionStateRequest; OK if the status is NO_ERROR, otherwise REMOTE_ERROR; a timeout also gives REMOTE_ERROR (K conn_ping:667-683). | Ping with the same mapping. | planned (M5) | `planned:knx_ping_is_ok_when_the_connection_state_response_reports_no_error`, `planned:knx_ping_with_an_error_status_is_a_remote_error`, `planned:knx_a_ping_timeout_is_a_remote_error` |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-CAP-FLAGS | Java: `canDiscover`, `canPing`, `canRead`, `canWrite`, `canSubscribe` and `canBrowse` all true (K driver:43-141). | All six capabilities true, with browse conditional on a knxproj at connect time. | planned (M5) | `planned:knx_capabilities_table_matches_the_java_flags`, `planned:knx_browse_capability_follows_the_knxproj_setting` |
| KNX-CAP-GO-FLAGS | Go metadata: read, write, subscribe, browse true; discovery true (G metadata:50-94; discovery_flag:98-104). | Same set as Java. | deferred (non-Java backlog) | none |

## Configuration
Five Java parameters. Go reads three of them from raw options plus `buildingKey`.

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-CFG-KNXPROJ-PATH | `knxproj-file-path`, File, no default; the ETS project, needed because the protocol does not carry everything required to decode messages (K config:34-68). Go reads no such option. | Optional path; enables browse and name decoding. | planned (M5) | `planned:knx_config_knxproj_file_path_is_optional`, `planned:knx_config_knxproj_file_path_enables_browse` |
| KNX-CFG-KNXPROJ-PASSWORD | `knxproj-password`, String, secret, optional password for the knxproj file (K config). | Secret, never logged. | planned (M5) | `planned:knx_config_knxproj_password_is_redacted_in_debug_output`, `planned:knx_the_knxproj_password_is_never_logged` |
| KNX-CFG-ADDRESS-LEVELS | `group-address-num-levels`, int, 3; values 3 (5/3/8 bits), 2 (5/11 bits), 1 (16 bits); supplied by the knxproj when one is configured (K config). Go: default 3 with an unexplained magic number; an unparsable value silently falls back to 3 (G options:169-211). | Default 3; values other than 1, 2, 3 are `InvalidConfig`. | planned (M5) | `planned:knx_config_group_address_levels_defaults_to_three`, `planned:knx_config_group_address_levels_other_than_one_two_or_three_is_invalid_config` |
| KNX-CFG-CONNECTION-TYPE | `connection-type`, String, `LINK_LAYER`; `LINK_LAYER` (own individual address), `RAW` (unmanaged bus access) or `BUSMONITOR` (passive, one per gateway) (K config). Go also accepts `TUNNEL_LINK_LAYER`, `TUNNEL_RAW`, `TUNNEL_BUSMONITOR` and falls back to `LINK_LAYER` with a warning on an unknown value (G options). | The three Java names, case-insensitive; an unknown name is `InvalidConfig`. | planned (M5) | `planned:knx_config_connection_type_defaults_to_link_layer`, `planned:knx_config_connection_type_accepts_the_three_java_names_in_any_case`, `planned:knx_config_unknown_connection_type_is_invalid_config` |
| KNX-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 10000; bounds the wait for a reply during search, connect and tunnelling exchanges (K config). Go: positive integer, otherwise warns and uses the default (G options; heartbeat:107-115). | Positive integer, default 10000. | planned (M5) | `planned:knx_config_request_timeout_defaults_to_10000_ms`, `planned:knx_config_request_timeout_that_is_not_a_positive_integer_is_invalid_config` |
| KNX-CFG-GO-BUILDING-KEY | Go `buildingKey` option, hex decoded, used to authenticate a device connection for the communication-object browse (G building_key:241-246; browse_comobj:158-200). | Not ported now. | deferred (non-Java backlog) | none |
| KNX-CFG-UNKNOWN | Unknown parameters are reported, not rejected (see `MB-CFG-UNKNOWN` in `docs/parity/modbus.md`). | Same mechanism. | planned (M5) | `planned:knx_config_unknown_option_is_reported_not_rejected` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-TRANSPORT-UDP | Protocol code `knxnet-ip`; the only transport is `udp`, default port 3671 (K driver:43-141; `knxnetip.transports`). Go: default transport `udp`, `defaultUdpPort` 3671 (G driver:53; driver_conn:62-96). | Scheme `knxnet-ip`, transport `udp`, port 3671. | planned (M5) | `planned:knx_connection_string_defaults_to_udp_on_port_3671` |
| KNX-TRANSPORT-OTHER | No other transport is supported by Java or Go. C# has no transport at all (C# no_socket_code). | Any other transport code is `InvalidConfig`. | planned (M5) | `planned:knx_driver_rejects_other_transport_codes` |

## Address grammar
Java tags are group addresses only. Matching is case-insensitive and tried 3-level, then
2-level, then 1-level (K tag:33-70). `*` is a wildcard in every position.

| Form | Regular expression shape | Range as written | File:line |
| --- | --- | --- | --- |
| 3 level | `main/middle/sub` with `\d{1,2}`, `\d`, `\d{1,3}` or `*` | 5 / 3 / 8 bits | K tag:33-70 |
| 2 level | `main/sub` with `\d{1,2}`, `\d{1,4}` or `*` | 5 / 11 bits | K tag:33-70 |
| 1 level | `\d{1,5}` or `*` | 16 bits | K tag:33-70 |

Each form takes an optional `:DPT<n>[.<sub>]` suffix. The pattern bounds digit counts,
not values; the manifest does not say where an out-of-range number is rejected
(UNVERIFIED).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-ADDR-3LEVEL | `main/middle/sub` group address (K tag). Go: same, `:DATATYPE` suffix (G tag_parse:83-179). | Parse as in Java. | planned (M5) | `planned:knx_a_three_level_group_address_is_parsed`, `planned:knx_a_three_level_group_address_with_too_many_digits_is_rejected` |
| KNX-ADDR-2LEVEL | `main/sub` (K tag; G tag_parse). | Parse as in Java. | planned (M5) | `planned:knx_a_two_level_group_address_is_parsed`, `planned:knx_a_two_level_group_address_with_too_many_digits_is_rejected` |
| KNX-ADDR-1LEVEL | Bare number up to five digits (K tag; G tag_parse). | Parse as in Java. | planned (M5) | `planned:knx_a_one_level_group_address_is_parsed`, `planned:knx_a_one_level_group_address_with_too_many_digits_is_rejected` |
| KNX-ADDR-WILDCARD | `*` in any position of a Java tag (K tag:33-70). Backlog row `go_knxnetip` "tag patterns": Go adds bracket lists and ranges such as `[1-3,5]`. | Java wildcards only. | planned (M5) | `planned:knx_a_wildcard_in_any_position_of_a_group_address_is_accepted`, `planned:knx_a_wildcard_tag_matches_every_group_address_in_that_position` |
| KNX-ADDR-DPT-SUFFIX | `:DPT<n>[.<sub>]` overrides the wire datatype (K tag). Go: `:DATATYPE` resolved by `KnxDatapointTypeByName`; an unknown name is "Unknown type"; a tag without a type is not defaulted by the cited code (UNVERIFIED, see below). | Optional suffix; an unknown datapoint is a typed error. | planned (M5) | `planned:knx_a_dpt_suffix_overrides_the_wire_datatype`, `planned:knx_a_dpt_suffix_with_a_sub_number_is_accepted`, `planned:knx_the_dpt_keyword_is_matched_without_regard_to_case`, `planned:knx_an_unknown_dpt_suffix_is_a_typed_error` |
| KNX-ADDR-GO-PATTERNS | Go bracket lists and ranges resolve to several group addresses and return a struct (G read_group:173-260; tag_parse). Backlog row "tag patterns". | Not ported now. | deferred (non-Java backlog) | none |
| KNX-ADDR-GO-DEVICE-PROPERTY | Go `{area}.{line}.{device}#{object}/{property}[selection]`, for example `1.2.3#11/1[0..3]`; the property index plus the selection offset is the start index (G tag_parse:83-179). Backlog row "device-level access". | Not ported now. | deferred (non-Java backlog) | none |
| KNX-ADDR-GO-DEVICE-MEMORY | Go `{area}.{line}.{device}#{hex address}[:type][selection]`; default type `DPT_DecimalFactor` (one byte); a selection that does not start at the first element is rejected (G tag_parse). | Not ported now. | deferred (non-Java backlog) | none |
| KNX-ADDR-GO-COMOBJ | Go `{area}.{line}.{device}#com-obj` query for browse (G query_parse:181-197). | Not ported now. | deferred (non-Java backlog) | none |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-LIM-WRITE-ONE-TAG | Java writes exactly one tag per request (K conn_write; G java_write_single:692-700). Go writes several tags one after another (G write:79-121). | One tag per write request. | planned (M5) | `planned:knx_write_handles_exactly_one_tag_per_request` |
| KNX-LIM-READ-SEQUENTIAL | Java reads tags one at a time (K conn_read:848-880). | Sequential reads. | planned (M5) | `planned:knx_read_of_several_tags_sends_the_group_reads_one_after_another` |
| KNX-LIM-BUSMONITOR-ONE | Only one `BUSMONITOR` connection is allowed per gateway at a time (K config description). | Documented; enforced by the gateway. | deferred (documentation only; enforced by the gateway) | none |
| KNX-LIM-DISCOVERY-WINDOW | 5000 ms collection window (K discoverer:61-110); Go hard-codes 5 s with a TODO (G discover_consts:43-54). | 5000 ms. | planned (M5) | `planned:knx_discovery_collects_replies_for_5000_ms` |
| KNX-LIM-HEARTBEAT | Heartbeat interval 60 s (K conn_doc; G heartbeat:107-115). | 60 s. | planned (M5) | `planned:knx_heartbeat_sends_a_connection_state_request_every_60_seconds` |

## Data types
Datapoint types are the generated `KnxDatapoint` model (DPT identifiers).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-DT-DPT-CODEC | The datapoint codec is generated from the mspec in Java, Go and C# alike. C#'s `KnxDatapoint.StaticParse` is the only hand-written test: bytes `41b00000` as `DPT_Value_Electric_Current` decode to `PlcREAL(22.0f)` (C# test_decode_only:26-39). The generated model is 10992 lines in C# (C# notes). | A DPT codec generated or ported with vectors from the upstream models. | planned (M5) | `planned:knx_a_dpt_value_electric_current_payload_decodes_to_22_0`, `planned:knx_a_decoded_datapoint_encodes_back_to_the_same_bytes` |
| KNX-DT-GO-DEFAULT-USINT | Go: `ReadGroupAddress` parses the payload with the tag's datapoint type; an unknown type defaults to USINT (G group_read:47-108). | Not ported now. | deferred (non-Java backlog) | none |
| KNX-DT-GO-STRUCT | Go: a multi-address pattern read returns a struct (G read_group:173-260). | Not ported now. | deferred (non-Java backlog) | none |

## Discovery, tunnel and browse details

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-DISC-SEARCH | Java opens a UDP socket on port 3671 per non-loopback IPv4 interface with a broadcast address, sends a SearchRequest (multicast constant 224.0.23.12) and collects replies within 5000 ms before returning (K discoverer:61-110). | Same. | planned (M5) | `planned:knx_discovery_sends_a_search_request_from_each_eligible_ipv4_interface`, `planned:knx_discovery_skips_loopback_and_interfaces_without_a_broadcast_address`, `planned:knx_discovery_returns_the_replies_collected_within_the_window` |
| KNX-DISC-GO-MULTICAST | Go sends the SearchRequest to 224.0.23.12:3671 from each non-loopback IPv4 address of the selected interfaces and returns once the scans are started; items carry the control endpoint URL and the friendly name (G discover_flow:79-195; discover_scan:202-291; discover_item:295-330). | Not ported now. | deferred (non-Java backlog) | none |
| KNX-TUNNEL-ACK | Java matches the TunnelingResponse on the sequence counter (K conn_write). Go acknowledges every incoming TunnelingRequest automatically (G tunnel_ack:132-168). The Java side of the incoming acknowledgement lies in the unread lines 330-665 (UNVERIFIED). | Acknowledge incoming tunneling requests and match outgoing by sequence. | planned (M5) | `planned:knx_an_incoming_tunneling_request_is_acknowledged_with_its_sequence_counter`, `planned:knx_a_tunneling_response_is_matched_to_the_write_by_its_sequence_counter` |
| KNX-SUB-PUSH-DECODE | Every LDataInd is decoded (optionally via the knxproj) and delivered to consumers whose tag matches (K conn_subscribe:1181-1240). Go: only `ApduDataGroupValueWrite` updates the cache and subscribers; GroupValueResponse does not reach subscribers (G incoming:61-140; sub_dispatch). | Java behavior. | planned (M5) | `planned:knx_a_group_indication_reaches_the_consumer_whose_tag_matches`, `planned:knx_an_indication_is_decoded_through_the_knxproj_when_one_is_configured` |
| KNX-ETS-PARSER | ETS project parsing lives in `ets/EtsParser` with `Ets5FileHandler` and `Ets6FileHandler`; not read in detail (K notes). | A knxproj reader for ETS 5 and ETS 6. | planned (M5) | `planned:knx_an_ets5_project_file_is_parsed_into_the_group_address_model`, `planned:knx_an_ets6_project_file_is_parsed_into_the_group_address_model`, `planned:knx_a_password_protected_project_is_opened_with_the_configured_password` |
| KNX-BROWSE-GO-DEVICE | Go browse without ETS: `area.line.device` probes a device (deviceDescriptor, maxApdu); `#com-obj` reads the group address table, optionally authenticating with `buildingKey` (G browse_dev:67-156; browse_comobj:158-200). Bridge/TODO at 196. | Not ported now. | deferred (non-Java backlog) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| KNX-QUIRK-PING-TIMEOUT | A ping timeout is REMOTE_ERROR, the same as a bad status, so a lost gateway and a refusing gateway look alike. | K conn_ping:667-683 | planned (M5) | Reproduced and documented: `planned:knx_a_ping_timeout_is_a_remote_error`. |
| KNX-QUIRK-READ-TIMEOUT | A read timeout is NOT_FOUND, not a timeout error. | K conn_read:848-880 | planned (M5) | Reproduced and documented: `planned:knx_a_read_timeout_is_reported_as_not_found`. |
| KNX-QUIRK-GO-STALE-CACHE | Go reads group addresses from a cache fed by observed writes, so a cached value can be stale and a hit never contacts the bus. | G read_group:173-260 | deferred (non-Java backlog) | none |
| KNX-QUIRK-GO-EMPTY-DATATYPE | Go treats a present datatype capture as set even when empty; whether untyped tags then fail with "Unknown type" is not established. | G tag_parse:83-179 | deferred (non-Java backlog) | none |
| KNX-QUIRK-GO-DISCOVER-RETURNS-EARLY | Go `Discover` returns once the scans are started; results arrive afterwards and Close waits for them. | G discover_flow:79-195 | deferred (non-Java backlog) | none |
| KNX-QUIRK-GO-SILENT-FALLBACK | Go falls back to defaults on an unparsable `group-address-num-levels` without a warning and warns for the other two. | G options:169-211 | deferred (non-Java backlog) | none |

## Divergences from upstream
None planned. A divergence row reuses the id of the behavior row it qualifies.

| Row id | Upstream behavior (file:line at ea229918) | Stonecrab behavior | Reference | Regression test id |
| --- | --- | --- | --- | --- |

## Backlog from other languages
Every `[[backlog]]` row with `driver = "go_knxnetip"`, plus the C# gap recorded in
`[inventory.csharp_knxnetip]` (which has no `[[backlog]]` row because C# has no driver).
Java is the baseline and the M5 target; every row below stays
`deferred (non-Java backlog)` and is not part of the M5 port.

| Row id | Backlog capability and behavior (manifest) | Where recorded | Status | Test ids |
| --- | --- | --- | --- | --- |
| KNX-BACKLOG-DEVICE-ACCESS | "device-level access": Go device property and memory reads through a device connection (evidence `tag_parse`). Java: group addresses with a DPT suffix only. | KNX-ADDR-GO-DEVICE-PROPERTY, KNX-ADDR-GO-DEVICE-MEMORY | deferred (non-Java backlog) | none |
| KNX-BACKLOG-BROWSE-NO-ETS | "browse without an ETS model": Go device probing and communication-object browse against the live bus (evidence `browse_dev`). Java browse needs `knxproj-file-path`. | KNX-BROWSE-GO-DEVICE | deferred (non-Java backlog) | none |
| KNX-BACKLOG-WRITE-MULTI | "write": several tags per request, written sequentially (evidence `write`). Java: single tag. | KNX-LIM-WRITE-ONE-TAG | deferred (non-Java backlog) | none |
| KNX-BACKLOG-READ-CACHE | "read from cache": served from a value cache populated by observed telegrams (evidence `read_group`). Java: always sends a group read. | KNX-QUIRK-GO-STALE-CACHE | deferred (non-Java backlog) | none |
| KNX-BACKLOG-CHANGE-OF-STATE | "change-of-state subscription": Go distinguishes CHANGE_OF_STATE from EVENT using the cache; cyclic is not supported (evidence `sub_types`, Subscriber.go:222-236). Java: push with tag match, no change detection. | KNX-SUB-PUSH-DECODE | deferred (non-Java backlog) | none |
| KNX-BACKLOG-TAG-PATTERNS | "tag patterns": wildcards plus bracket lists and ranges such as `[1-3,5]` (evidence `tag_parse`). | KNX-ADDR-WILDCARD, KNX-ADDR-GO-PATTERNS | deferred (non-Java backlog) | none |
| KNX-CSHARP-NO-DRIVER | C#: whole driver absent (handshake, heartbeat, tunneling), read, write, push subscribe, browse, discovery, ping, configuration, tag syntax and UDP transport (C# notes, LANG GAP lines). Only the generated `KnxDatapoint` decode exists. Nothing to port from C#. | KNX-DT-DPT-CODEC | deferred (non-Java backlog) | none |
| KNX-GO-EXTRAS | Go-only behavior with no `[[backlog]]` row: ping failure is a plain error (not REMOTE_ERROR); a keepalive ticker that logs failures only; `IsConnected` calls Ping; a typed configuration object is absent (raw options); discovery uses multicast only (G ping:534-543; keepalive:415-448; options:169-211). | KNX-OP-PING, KNX-DISC-GO-MULTICAST | deferred (non-Java backlog) | none |

## UNVERIFIED
- Lines 330-665 of `KnxNetIpConnection.java` (connect, heartbeat, incoming dispatch) were
  not read, so the 60 s heartbeat comes from the class comment and not from code read.
- The body of `readSingleTag` (how the group read is encoded) and the write body past
  line 712.
- The mspec content (`knxnetip.mspec`) was not read.
- Java close behavior (KNX-OP-CLOSE), and where Java rejects out-of-range group address
  numbers.
- Whether the Java driver also serves device-level or cached reads.
- ETS project parsing in detail.
- Go: `ConnectionInternalOperations.go` (1258 lines) beyond the cited headers, device
  authentication and property/memory read internals; BUSMONITOR frame handling; the
  `KnxLayer` selection beyond a grep.
- C#: which `ApduData` and `Cemi` types exist in the generated model.
