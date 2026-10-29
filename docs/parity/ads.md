# ADS (Beckhoff TwinCAT) parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing in this record is implemented. ADS/AMS is one of the first
M5 protocols (ADS, SLMP and UMAS, then the rest), so every behavior row is `planned (M5)`
or `deferred` with its reason. The Go driver (`go_ads`) is recorded in its own section and
in the backlog table. Test ids are `planned:` names, not existing tests; no device model or
TwinCAT version is named yet.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn; the Test ids column names
  at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: recorded so the omission is visible; not ported now.
  `deferred (upstream placeholder)` marks a skeleton or "not implemented" upstream, which
  does not become an obligation (SPEC, "Compatibility baseline").
- No row is `implemented` yet, so `scripts/parity-check.sh` does not require these ids to
  appear in code.

## File key
Paths are relative to the repository root of apache/plc4x. `J` abbreviates
`plc4j/drivers/ads/src/main/java/org/apache/plc4x/java/ads`.

| Key | Path |
| --- | --- |
| driver | `J/AdsPlcDriver.java` |
| connection_class, ping, read, write, subscribe, unsubscribe, notify, browse | `J/AdsTcpConnection.java` |
| discovery | `J/discovery/AdsPlcDiscoverer.java` |
| config | `J/configuration/AdsConfiguration.java` |
| tag_direct, tag_symbolic | `J/tag/DirectAdsTag.java`, `J/tag/SymbolicAdsTag.java` |
| mspec_notify, mspec_deviceinfo | `protocols/ads/src/main/resources/protocols/ads/ads.mspec` |
| go_* keys | `plc4go/internal/ads/*.go`, see the Go section |

## Operations

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-OP-CONNECT | Code `ads`, default and only transport `tcp`, port `ADSTCPDEFAULTPORT` (driver 51-81). The connection extends `ConnectionBase`, not the polling base (connection_class 100). After connect it loads the symbol and data-type tables unless disabled and watches the online and symbol versions to reload them (Java version watch 309-358, cited from the Go inventory as java_version_watch). | `ads:tcp://host` with the four AMS parameters; load the tables after connect and reload them when the version changes. | planned (M5) | `planned:ads_connect_loads_the_symbol_and_data_type_tables`, `planned:ads_symbol_version_change_reloads_the_tables` |
| ADS-OP-CLOSE | Close path not cited by the manifest. | Close deletes live notifications, then drops the transport. | planned (M5) | `planned:ads_close_deletes_live_notifications` |
| ADS-OP-READ | `onRead` resolves each tag as a symbolic tag through the loaded tables, a direct string tag or a direct index-group/offset tag, then issues ADS reads; one request or the SumUp multiple read by resolved tag count (read 484-524, 553-558, 592-600). | Same resolution order and the same single or sum-up choice. | planned (M5) | `planned:ads_read_of_a_direct_tag_decodes_the_response`, `planned:ads_read_of_several_tags_uses_a_sum_up_request`, `planned:ads_read_of_a_symbolic_tag_resolves_through_the_symbol_table` |
| ADS-OP-WRITE | `onWrite`, same tag resolution; multi-write by tag count (write 707-710, 806). | Same as read. | planned (M5) | `planned:ads_write_of_a_direct_tag_encodes_the_payload`, `planned:ads_write_of_several_tags_uses_a_sum_up_request` |
| ADS-OP-SUBSCRIBE | Native: `onSubscribe` resolves tags then `executeSubscribe` builds `AdsAddDeviceNotificationRequest`; incoming `AdsDeviceNotification` is dispatched to consumers by handle (subscribe 870-890, notify 1030-1066; mspec_notify 231-273). Not polling. | Native subscribe through AddDeviceNotification; samples are matched by notification handle. | planned (M5) | `planned:ads_subscribe_sends_an_add_device_notification`, `planned:ads_notification_sample_reaches_the_matching_subscription` |
| ADS-OP-UNSUBSCRIBE | `onUnsubscribe` collects notification handles and sends DeleteDeviceNotification (unsubscribe 972-981; mspec_notify 231-273). | Unsubscribe deletes the notifications on the device. | planned (M5) | `planned:ads_unsubscribe_sends_a_delete_device_notification` |
| ADS-OP-BROWSE | `onBrowse` and the interceptor variant walk the symbol and data-type tables; an error when the tables were not loaded (browse 1078-1090). | Browse over the loaded tables, with the same error when `load-symbol-and-data-type-tables` is false. | planned (M5) | `planned:ads_browse_lists_the_symbol_table`, `planned:ads_browse_without_tables_is_an_error` |
| ADS-OP-DISCOVER | `AdsPlcDiscoverer` broadcasts the ADS UDP discovery on every non-loopback IPv4 interface (port `ADSDISCOVERYUDPDEFAULTPORT`) and parses responses (host name, OS data, version, fingerprint blocks); items carry `source-ams-net-id` (interface IP plus `.1.1`) and `source-ams-port` 65534 (discovery 47-103). | Same broadcast and the same item options. | planned (M5) | `planned:ads_discover_parses_a_discovery_response`, `planned:ads_discover_item_carries_the_source_ams_net_id` |
| ADS-OP-PING | `onPing` sends `AdsReadDeviceInfoRequest` and answers OK on a response (ping 471-477; mspec_deviceinfo 160-161). | Ping through ReadDeviceInfo. | planned (M5) | `planned:ads_ping_sends_a_read_device_info_request` |

## Capabilities

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-CAP-FLAGS | `canDiscover`, `canPing`, `canRead`, `canWrite`, `canSubscribe`, `canBrowse` all true (driver 101-129). | Full capability set `{discover, ping, read, write, subscribe, browse}`. | planned (M5) | `planned:ads_capability_set_matches_the_inventory` |

## Configuration
Six parameters on `AdsConfiguration` (config 40-75); the four AMS parameters are required.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-CFG-TARGET-NET-ID | `target-ams-net-id`, required, six dot-separated decimal groups (pattern `\d{1,3}` x 6) (config 40-75). | Same name; a missing or malformed id is a typed `InvalidConfig`. | planned (M5) | `planned:ads_config_requires_the_target_ams_net_id`, `planned:ads_config_rejects_a_net_id_with_five_groups` |
| ADS-CFG-TARGET-PORT | `target-ams-port`, int, required. | Same name; required. | planned (M5) | `planned:ads_config_requires_the_target_ams_port` |
| ADS-CFG-SOURCE-NET-ID | `source-ams-net-id`, required. | Same as the target id. | planned (M5) | `planned:ads_config_requires_the_source_ams_net_id` |
| ADS-CFG-SOURCE-PORT | `source-ams-port`, int, required. | Same name; required. | planned (M5) | `planned:ads_config_requires_the_source_ams_port` |
| ADS-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, default 4000 (config 40-75). | Same name and default. | planned (M5) | `planned:ads_config_request_timeout_defaults_to_four_seconds` |
| ADS-CFG-MAX-DEPTH | `max-data-type-table-depth`, int, default 20: the nesting limit accepted when parsing the uploaded data-type table (config 40-75). | Same name and default; a deeper table is a typed error, never a stack overflow. | planned (M5) | `planned:ads_config_depth_limit_defaults_to_twenty`, `planned:ads_data_type_table_deeper_than_the_limit_is_rejected` |
| ADS-CFG-LOAD-TABLES | `load-symbol-and-data-type-tables`, boolean, default true. When false only direct addresses work, symbolic ones are rejected, browse is unavailable, subscriptions are unaffected (config 40-75). | Same name, default and effects. | planned (M5) | `planned:ads_config_tables_disabled_rejects_symbolic_addresses`, `planned:ads_config_tables_disabled_still_subscribes` |

## Connection string and transports

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-TRANSPORT-TCP | Default and only transport `tcp` (driver 73-81; `inventory.ads.transports`). | `ads:tcp://host[:port]`, port `ADSTCPDEFAULTPORT`; every other transport is rejected with `InvalidConfig`. | planned (M5) | `planned:ads_uri_defaults_to_tcp`, `planned:ads_uri_rejects_other_transports` |
| ADS-TRANSPORT-UDP-DISCOVERY | UDP broadcast used by discovery only (discovery 47-103). | Discovery-only socket, not a connection transport. | planned (M5) | `planned:ads_uri_with_a_udp_transport_is_rejected` |

## Address grammar
Two Java tag classes. The Go resolver also has a direct string form, see the Go section.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-ADDR-DIRECT | `indexGroup/indexOffset[array]:TYPE`; each of group and offset is decimal (`\d{1,10}`) or `0x` hex (1 to 8 digits), both bounded by unsigned 32 bits (tag_direct 44-50). | `DirectAddress` with `parse`; values above `u32::MAX` are rejected. | planned (M5) | `planned:ads_address_parses_decimal_and_hex_direct_forms`, `planned:ads_address_rejects_an_index_group_above_u32_max` |
| ADS-ADDR-SYMBOLIC | Symbolic name with dotted fields and `[index]`, pattern `^[a-zA-Z_]\w*(\[\d+\])*(\.[a-zA-Z_]\w*(\[\d+\])*)*$` (tag_symbolic 41-44). | `SymbolicAddress` with the same grammar, resolved against the loaded tables. | planned (M5) | `planned:ads_address_parses_a_dotted_symbolic_name`, `planned:ads_address_rejects_a_name_starting_with_a_digit` |
| ADS-ADDR-DIRECT-STRING | `DirectAdsStringTag` is a resolution branch in `onRead` (read 484-524); its pattern is not cited by the manifest. | Same `STRING(n)` and `WSTRING(n)` forms as the Go resolver. | planned (M5) | `planned:ads_address_parses_a_direct_string_length` |
| ADS-ADDR-ARRAY | Array selection through `ArrayNotationParser.ARRAY_GROUP` on direct tags; a range must be contiguous and only the last dimension of the trailing member may be one (tag_direct 44-50; SymbolicAdsTag comment). | Same restriction, typed `InvalidAddress`. | planned (M5) | `planned:ads_address_rejects_a_range_that_is_not_contiguous` |
| ADS-ADDR-QUERY | Browse query forms: not cited in Java; Go accepts any string as a symbolic query (go_query 274-278). | Browse takes no query in Stonecrab until M5 decides. | deferred (not inventoried; see UNVERIFIED) | none |

## Limits

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-LIMIT-U32 | Index group, offset and length travel as four bytes (tag_direct comment `MAX_UINT32`). | Enforced by the address parser. | planned (M5) | `planned:ads_address_rejects_an_index_offset_above_u32_max` |
| ADS-LIMIT-DEPTH | Table nesting limited by `max-data-type-table-depth`, the JVM stack being the practical ceiling (config 40-75). | Limit enforced by an explicit counter. | planned (M5) | `planned:ads_data_type_table_deeper_than_the_limit_is_rejected` |
| ADS-LIMIT-SUMUP | Maximum items and payload of a SumUp request: not cited. | Recorded once read. | deferred (not inventoried; see UNVERIFIED) | none |

## Data types
The direct tag carries an ADS data type name after the colon (`:TYPE`); the symbolic path
takes the type from the data-type table. The full list of ADS types and their value
mapping is not cited by the manifest.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-TYPE-DIRECT | `:TYPE` after a direct address, `\w+` (tag_direct 44-50); `STRING(n)` and `WSTRING(n)` carry a length (Go resolve_direct 355-402). | Typed decode and encode for the elementary IEC 61131 types, STRING and WSTRING. | planned (M5) | `planned:ads_read_of_a_dint_decodes_little_endian`, `planned:ads_read_of_a_string_uses_the_declared_length` |
| ADS-TYPE-TABLE | Symbol and data-type tables uploaded from the device resolve struct members and arrays (Go resolve_symbolic 404-437; Java `onRead` 484-524). | Struct and array resolution through the table. | planned (M5) | `planned:ads_read_of_a_struct_member_resolves_the_offset` |
| ADS-TYPE-TIME | TIME, DATE and related families: generated `DataItem` behavior is not in the repository (manifest pending item). | Recorded once inspected. | deferred (not inventoried; see UNVERIFIED) | none |

## Upstream quirks

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-QUIRK-SOURCE-ID | Discovery items synthesize `source-ams-net-id` as the interface IP plus `.1.1` and `source-ams-port` 65534 (manifest notes). | Same synthesized options, documented so users can override them. | planned (M5) | `planned:ads_discover_item_carries_the_source_ams_net_id` |
| ADS-QUIRK-TABLES-OFF | With the tables disabled, subscriptions still work because symbol handles are resolved on the device (config 40-75). | Same. | planned (M5) | `planned:ads_config_tables_disabled_still_subscribes` |

## Divergences from upstream
No divergence is planned yet.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-DIV-NONE | None recorded. | None. | planned (M5) | `planned:ads_divergence_table_is_reviewed_at_m5_exit` |

## Go driver (`go_ads`)
Go has native subscribe, browse and discovery like Java, with the defects and gaps below.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-GO-READ-ERROR | The single-tag read ignores a non-zero ADS error code and parses the data anyway (read_single 61-120, TODO at 94-96). | Not reproduced: a non-zero code is an item error. | deferred (upstream defect, not ported) | none |
| ADS-GO-UNSUBSCRIBE | Unsubscribe returns nil; the builder returns nil with a TODO; `ExecuteAdsDeleteDeviceNotificationRequest` has no caller (unsub_builder 36-43, unsub_nil 277-286, delete_notification_unused 222-250). | Covered by ADS-OP-UNSUBSCRIBE (Java). | deferred (upstream placeholder) | none |
| ADS-GO-BROWSE | One symbolic query form; cannot list top-level symbols or members; a one-segment pattern panics (browse_flow 35-78, browse_filter 80-158, browse_panic 88-93). | Not ported; Java walks the whole table (ADS-OP-BROWSE). | deferred (upstream defect, not ported) | none |
| ADS-GO-DISCOVER | 10 s fixed sleep ignoring ctx, hard-coded `targetAmsPort` 851, OS data skipped, unconditional `*net.IPNet` assertion (discover_send 219-263, discover_resp 285-376, discover_mask 139). | Not ported. | deferred (upstream defect, not ported) | none |
| ADS-GO-CONFIG | Net id split indexes `split[1..5]` with no length check (config_parse 39-136); accepts camelCase and kebab-case keys. | Not ported; ADS-CFG-TARGET-NET-ID rejects short ids. | deferred (upstream defect, not ported) | none |
| ADS-GO-MULTI | SumUp multiple read and write with per-item return codes (read_multi 122-204, write_multi 114-181). | Covered by ADS-OP-READ and ADS-OP-WRITE. | planned (M5) | `planned:ads_read_of_several_tags_uses_a_sum_up_request` |
| ADS-GO-SYMBOL-HEURISTIC | A symbol is the first two dotted segments (resolve_symbolic 404-437), a heuristic. | Not ported; Stonecrab resolves against the table. | deferred (upstream heuristic, not ported) | none |

## Backlog from other languages
Rows are the Go `LANG GAP` notes of `inventory.go_ads`; the manifest has no `[[backlog]]`
entry for `ads`. All are deferred until a maintainer decides to adopt them.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ADS-BL-GO-UNSUBSCRIBE | Java deletes notifications on unsubscribe; Go does not. | Java behavior is planned (ADS-OP-UNSUBSCRIBE). | deferred (non-Java backlog) | none |
| ADS-BL-GO-PING | Go has no ping. | Java behavior is planned (ADS-OP-PING). | deferred (non-Java backlog) | none |
| ADS-BL-GO-CONFIG | Go has neither `request-timeout-ms` (fixed 5 s, timeout 259-274) nor `load-symbol-and-data-type-tables`. | Java options are planned. | deferred (non-Java backlog) | none |
| ADS-BL-GO-BROWSE | Go cannot list symbols or members. | Java behavior is planned. | deferred (non-Java backlog) | none |
| ADS-BL-GO-DISCOVERY | Go skips OS data. | Java behavior is planned. | deferred (non-Java backlog) | none |
| ADS-BL-GO-ITEM-ERRORS | Java checks results; the Go single read does not. | Java behavior is planned. | deferred (non-Java backlog) | none |

## UNVERIFIED
- The ADS close path and the internals of `AdsTcpConnection` lines 1100-1531, read only
  far enough to establish capabilities.
- The pattern of the direct string tag and the browse query forms.
- The ADS data-type list and the TIME and DATE family (generated code).
- SumUp item and payload limits.
- `ADSTCPDEFAULTPORT` and `ADSDISCOVERYUDPDEFAULTPORT` values (named in the manifest, not
  quoted).
- No device model or TwinCAT version is named.
