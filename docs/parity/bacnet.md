# BACnet/IP parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. Every row is `planned` for a milestone or
`deferred` with its reason. The Java driver (`bacnet-ip`) is a passive listener: it
decodes COV notifications and hands them to subscribers, and nothing else works. Its tag
syntax is a TODO placeholder. The Go driver (`go_bacnetip`) is the only implementation
with read, write, discovery and an active COV subscribe. The plan puts BACnet in M5, with
the Java behavior as the milestone's target and the Go capabilities in the non-Java
backlog (`docs/POST_ALPHA_PLAN.md` section 4). Go capabilities are therefore `deferred
(non-Java backlog)` and each cites its `[[backlog]]` row (`driver = "go_bacnetip"`, the
capability name is given in the row). Support levels in the README and
`docs/COMPATIBILITY.md` follow from these statuses.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. Its Test ids cell
  names at least one test id prefixed `planned:`; no such test exists yet, and
  `scripts/parity-check.sh` does not look for it until the row becomes `implemented`.
- `deferred (<reason>)`: recorded so the omission is visible; Stonecrab does not port it
  now. `deferred (upstream placeholder)` marks an upstream stub, TODO or declared-but-unused
  option, which is not an obligation (`docs/SPEC.md`, "Compatibility baseline"). Test ids
  is `none` unless a rejection test is planned.
- `implemented`, `automated-verified`, `hardware-verified`: not used yet; see
  `docs/parity/modbus.md` for their meaning.

No row is `implemented`, so no row id needs to appear in code yet. Row ids are never
renumbered; a retired row keeps its id and says so.

Scope: the Java `bacnet-ip` driver and the Go `bacnetip` driver. The generated wire model
(`protocols/bacnetip`, the `bacnet-*.mspec` files) was listed but not read for the
inventory, so no row claims which BACnet services the wire model covers (see UNVERIFIED).
C, C# and Python have no BACnet driver in the inventory.

## File key
Citations use the manifest keys of `[inventory.bacnet]` (Java) and
`[inventory.go_bacnetip]` (Go), each of which carries a `path` and a `lines` range at the
pinned commit. Paths are relative to the apache/plc4x repository root.

| Key | Entry and path |
| --- | --- |
| driver | `bacnet` `driver`: `plc4j/drivers/bacnet/.../bacnetip/BacNetIpDriver.java:36-79` |
| conn_passive | `bacnet` `conn_passive`: `BacNetIpConnection.java:88-93` |
| conn_cov_decode | `bacnet` `conn_cov_decode`: `BacNetIpConnection.java:169-270` |
| conn_subscribe | `bacnet` `conn_subscribe`: `BacNetIpConnection.java:276-304` |
| tag_todo, tag_todo2 | `bacnet`: `tag/BacNetIpTag.java:35-36`, `:69-72` |
| config | `bacnet`: `configuration/BacNetIpConfiguration.java:27-51` |
| base_defaults, base_unsupported | `bacnet`: `SPI/DriverBase.java:117-139`, `SPI/ConnectionBase.java:437-471` |
| java_driver_flags | `go_bacnetip`: `BacNetIpDriver.java:60-73` |
| go driver, udp_only, discovery_flag | `go_bacnetip`: `plc4go/internal/bacnetip/Driver.go:56-69`, `:115-118`, `:162-168` |
| conn_route, conn_meta, conn_builders | `go_bacnetip`: `Connection.go:166-201`, `:211-218`, `:220-266` |
| config_fields, config_defaults, bbmd_declared | `go_bacnetip`: `Configuration.go:37-121`, `:186-199`, `:63-69` |
| read_build, read_array_quantity, read_seg_req, read_decode, read_seg_resp | `go_bacnetip`: `Reader.go:73-124`, `:84-88`, `:143-162`, `:348-418`, `ReaderSegmentation.go:36-75` |
| seg_send, write_build | `go_bacnetip`: `SenderSegmentation.go:36-60`, `Writer.go:199-283` |
| sub_subscribe, sub_unsub, sub_sync_note, sub_incoming, sub_dispatch, refresh_missing | `go_bacnetip`: `Subscriber.go:82-139`, `:141-164`, `:191-194`, `:264-289`, `:294-334`, `SubscriptionHandle.go:52-55` |
| tag_syntax, tag_parse, tag_addr | `go_bacnetip`: `TagHandler.go:41-42`, `:55-120`, `Tag.go:158-179` |
| discover_main, discover_whois, discover_results | `go_bacnetip`: `Discoverer.go:75-118`, `:138-183`, `:316-385` |
| routing, device_cache, static_devices | `go_bacnetip`: `Framing.go:34-90`, `DeviceInfoCache.go:69-77`, `:122-130` |
| ping_default, browse_default | `go_bacnetip`: `plc4go/spi/default/DefaultConnection.go:200-208`, `:250-252` |

## Operations
Java is the baseline. A Go-only operation cites the `[[backlog]]` row for
`go_bacnetip` with that capability.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-OP-CONNECT | Java: no handshake. `onConnect` loads the optional EDE model, starts receiving BVLC frames and marks the connection connected; the connection keeps no other state (conn_passive; `bacnet` notes). Go: binds `0.0.0.0:47808` with address reuse, overridable by the `local-port` driver option (Driver.go:86-113 per the `go_bacnetip` notes). | Connect binds the UDP socket, loads the EDE descriptors when configured and reports connected without any wire exchange. | planned (M5) | `planned:bacnet_connect_sends_nothing_and_reports_connected`, `planned:bacnet_connect_loads_the_configured_ede_file` |
| BAC-OP-CLOSE | Java: stops receiving and clears the consumer map (conn_subscribe; UNVERIFIED, the close body was not cited). | Close stops the listener and drops every subscription handle. | planned (M5) | `planned:bacnet_close_stops_delivering_notifications` |
| BAC-OP-READ | Java: unsupported; `canRead` is false and `onRead` is the base failure "Read not supported" (`bacnet` operations.read; base_defaults, base_unsupported). Go: ReadProperty or ReadPropertyMultiple (read_build); backlog row `go_bacnetip` "read". | Java-parity read is a typed unsupported error with capability read false. A Go-style read is not ported now. | deferred (non-Java backlog) | `planned:bacnet_read_is_rejected_as_unsupported` |
| BAC-OP-WRITE | Java: unsupported (`bacnet` operations.write). Go: WriteProperty or WritePropertyMultiple with priority and array index (write_build); backlog row `go_bacnetip` "write". | Typed unsupported error; no Go-style write now. | deferred (non-Java backlog) | `planned:bacnet_write_is_rejected_as_unsupported` |
| BAC-OP-SUBSCRIBE-PASSIVE | Java: `canSubscribe` true. `onSubscribe` returns an OK handle for every tag without sending any SubscribeCOV (conn_subscribe:276-304). The connection decodes ConfirmedCOVNotification and UnconfirmedCOVNotification and delivers each event to every registered consumer with no tag filtering (conn_cov_decode:169-270). This is not polling. | Subscribe returns an OK handle per tag and sends nothing on the wire. Notifications are delivered to all handles of the connection (see BAC-COV-NO-FILTER). | planned (M5) | `planned:bacnet_subscribe_returns_ok_handles_without_wire_traffic`, `planned:bacnet_unconfirmed_cov_notification_reaches_every_consumer` |
| BAC-OP-UNSUBSCRIBE-NOOP | Java: `onUnsubscribe` is a no-op (conn_subscribe:276-304). | Unsubscribe succeeds and sends nothing; whether delivery to that consumer stops is decided in M5 (the upstream registry is the consumer map). | planned (M5) | `planned:bacnet_unsubscribe_sends_nothing` |
| BAC-OP-SUBSCRIBE-COV | Go: one SubscribeCOV per tag with lifetime `cov-lifetime-seconds` (default 600), handles keyed by a per-connection process id (sub_subscribe:82-139); backlog row `go_bacnetip` "subscribe". | An active COV subscription is not ported now. | deferred (non-Java backlog) | none |
| BAC-OP-UNSUBSCRIBE-COV | Go: Unsubscribe sends SubscribeCOV with lifetime 0 (sub_unsub:141-164). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-OP-BROWSE | Java: unsupported (`bacnet` operations.browse). Go: `ProvidesBrowsing` false; the default browse builder panics (browse_default:250-252; conn_meta). | `Capabilities.browse` is false. Nothing to port in either implementation. | deferred (none upstream) | `planned:bacnet_capabilities_report_browse_false` |
| BAC-OP-DISCOVER | Java: unsupported; no `PlcDiscoverer`, `canDiscover` not overridden (`bacnet` operations.discover). Go: Who-Is broadcast and directed unicast, optional Who-Has, I-Am and I-Have items (discovery_flag; discover_main:75-118; discover_whois:138-183; discover_results:316-385); backlog row `go_bacnetip` "discover". | `Capabilities.discover` is false. Discovery is not ported now. | deferred (non-Java backlog) | none |
| BAC-OP-PING | Java: unsupported (`bacnet` operations.ping). Go: no wire ping; the default checks only the local connected flag (ping_default:200-208). | `Capabilities.ping` is false. | deferred (none upstream) | `planned:bacnet_capabilities_report_ping_false` |

## Capabilities
Two signals exist upstream: the driver flags and the connection behavior. For Java they
agree: only subscribe is on.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-CAP-FLAGS | Java: `canDiscover`, `canBrowse`, `canPing`, `canRead`, `canWrite` all false by default; `BacNetIpDriver` overrides `canSubscribe` only (driver:36-79; base_defaults:117-139; java_driver_flags:60-73). | Capabilities: subscribe true, everything else false. | planned (M5) | `planned:bacnet_capabilities_table_matches_java_flags` |
| BAC-CAP-GO-FLAGS | Go: reading, writing and subscribing true, browsing false; discovery true on the driver (conn_meta:211-218; discovery_flag:162-168). | Not used while the Go capabilities are deferred; the Java flags in BAC-CAP-FLAGS apply. | deferred (non-Java backlog) | none |

## Configuration
Java has two options. Go has fourteen plus driver options. Go option names are the Go
struct field names, matched case-insensitively (`Configuration.go` comment, read for the
`go_bacnetip` entry), for example `localDeviceId`; they are not kebab-case. Unknown Go
options are reported, not rejected.

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-CFG-EDE-FILE | Java `ede-file-path`, File, no default; "Path to the location of a single EDE file, that contains the descriptor for the target device" (config:27-51). Parsed with `EdeParser` in `onConnect`. | `ede-file-path`, path, optional. A missing or unreadable file is `InvalidConfig` at connect. | planned (M5) | `planned:bacnet_config_ede_file_missing_is_invalid_config` |
| BAC-CFG-EDE-DIR | Java `ede-directory-path`, File, no default; directory holding several EDE files for the possible target devices (config:27-51). | `ede-directory-path`, path, optional. | planned (M5) | `planned:bacnet_config_ede_directory_loads_every_descriptor` |
| BAC-CFG-GO-LOCAL-DEVICE-ID | Go `LocalDeviceId`, uint32, default 260001, range 0..4194303; the device instance claimed in I-Am and in confirmed-request initiators (config_fields:37-121; config_defaults:186-199). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-LOCAL-NETWORK | Go `LocalNetworkNumber`, uint16, default 0 (same network as the destination) (config_fields). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-MAX-APDU | Go `MaxApduLengthAccepted`, uint16, default 1476 (config_fields; config_defaults). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-SEGMENTATION | Go `SegmentationSupported`, string, default `segmented-both`; one of `segmented-both`, `segmented-transmit`, `segmented-receive`, `no-segmentation` (config_fields; config_defaults). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-MAX-SEGMENTS | Go `MaxSegmentsAccepted`, uint8, default 16 (config_fields; config_defaults). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-VENDOR-ID | Go `VendorId`, uint16, default 0x4D4D ("MM"), a placeholder value (config_fields; `placeholderVendorId` per the Go source). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-APDU-TIMEOUT | Go `ApduTimeoutMs`, uint32, default 3000 (config_defaults). The SubscribeCOV request does not honour it (sub_sync_note:191-194). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-APDU-RETRIES | Go `ApduRetries`, uint8, default 3; used only by the segment sender (config_defaults; notes). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-COV-LIFETIME | Go `CovLifetimeSeconds`, uint32, default 600; 0 means indefinite (config_fields; config_defaults). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-DISCOVERY-TIMEOUT | Go `DiscoveryTimeoutSeconds`, uint32, default 5 (config_defaults). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-REMOTE-NETWORK | Go `RemoteNetwork` (uint16, 0 = local segment) and `RemoteAddress` (string, `ip:port` or hex MAC such as `0x0C`; required when `RemoteNetwork` is set) (config_fields:37-121; routing:34-90). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-PEER-LIMITS | Go `PeerMaxApduLengthAccepted` (uint16, 0 = unknown) and `PeerSegmentationSupported` (same four names, empty = unknown) (config_fields; read_seg_req:143-162). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-GO-BBMD | Go `ForeignDeviceBBMD` (host:port) and `ForeignDeviceTTL` (seconds) are declared and documented as enabling BBMD foreign-device registration, re-registered at TTL/2; nothing in the driver reads them (bbmd_declared:63-69). | None. | deferred (upstream placeholder) | none |
| BAC-CFG-GO-STATIC-DEVICES | Go `StaticDevices`, string `<deviceId>@<network>:<host>:<port>` list; `DeviceInfoCache.LoadStatic` has no non-test caller (static_devices:122-130; device_cache:69-77). | None. | deferred (upstream placeholder) | none |
| BAC-CFG-GO-LOCAL-PORT | Go driver option `local-port` overrides the fixed local bind `0.0.0.0:47808` (`go_bacnetip` notes, Driver.go:86-113). Not a Configuration field. | Not ported now. | deferred (non-Java backlog) | none |
| BAC-CFG-UNKNOWN | Java: unknown parameters are reported, not rejected, by the shared `DriverBase` path (the Modbus record's `MB-CFG-UNKNOWN` cites DB:206-209, 315-319). Go reports options nothing read. | Unknown options are reported through the same mechanism as Modbus. | planned (M5) | `planned:bacnet_config_unknown_option_is_reported_not_rejected` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-TRANSPORT-UDP | Protocol code `bacnet-ip`; default transport `udp`; supported `udp` and `test` (driver:36-79; `bacnet.transports`). The default BACnet/IP port 47808 (0xBAC0) is the Go bind default (go notes); the Java port is not in the cited lines (UNVERIFIED). | Scheme `bacnet-ip`, transport `udp`, implicit when omitted. | planned (M5) | `planned:bacnet_connection_string_defaults_to_udp`, `planned:bacnet_driver_rejects_other_transport_codes` |
| BAC-TRANSPORT-TEST | Java accepts the `test` transport (driver:36-79). Go requires the concrete udp transport (udp_only:115-118). | Rejected with `InvalidConfig`; Stonecrab tests use `FakeTransport`. | deferred (no release) | none |

## Address grammar
The Java grammar is a placeholder. The Go grammar is the only defined syntax.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-ADDR-JAVA-PLACEHOLDER | `ADDRESS_PATTERN` is the literal string "TODO: finish me... see golang implementation" and `getAddressString` returns the same TODO (tag_todo:35-36; tag_todo2:69-72). Tag syntax is effectively undefined. Java `onSubscribe` returns an OK handle for every tag. | No grammar is ported from Java. Passive subscribe accepts any tag text as an opaque handle label (BAC-OP-SUBSCRIBE-PASSIVE). | deferred (upstream placeholder) | none |
| BAC-ADDR-GO-SYNTAX | Go: `{objectType},{instance}/{property}[idx]{priority}&...`; regex `^[\d\w]+,\d+/<property>(&<property>)*` (tag_syntax:41-42; tag_parse:55-120). Round-trips through `GetAddressString` (tag_addr:158-179); backlog row `go_bacnetip` "tag syntax". | Not ported now. A grammar for BACnet tags is decided with the first read or write capability. | deferred (non-Java backlog) | none |
| BAC-ADDR-GO-OBJECT-TYPE | Go: object type by BACnet name or by number; a number is a proprietary type id (uint16), an unknown name is an error (tag_parse). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-ADDR-GO-PROPERTY | Go: property by name or number (proprietary uint32), several properties joined by `&` (tag_parse). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-ADDR-GO-ARRAY-PRIORITY | Go: optional `[n]` array index (uint32) and `{n}` write priority, range 1..16 (tag_parse:55-120). `GetArrayInfo` always returns empty, so a read quantity is always 1 (read_array_quantity:84-88; tag_addr:177-179). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-ADDR-GO-QUERY | Go `ParseQuery` returns "queries not supported" (`go_bacnetip` operations.browse). | None. | deferred (none upstream) | none |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-LIM-JAVA-NONE | Java declares no limits: no request is built, no tag count is checked (conn_passive:88-93). | No limit rows are ported. | planned (M5) | `planned:bacnet_subscribe_does_not_limit_tag_count` |
| BAC-LIM-GO-READ-REQUEST | Go: one confirmed request per Read call, ReadProperty for one tag, ReadPropertyMultiple otherwise; no splitting when many tags exceed the APDU limit, only fail-fast or segmentation when peer limits are configured (`go_bacnetip` notes; read_build:73-124). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-LIM-GO-SEGMENTS | Go: inbound reassembly with window 1 and 5 s per segment (read_seg_resp:36-75); outbound request segmentation per ASHRAE 135 clause 5.4, window 16 proposed, 5 s ack wait, cap 255 segments (seg_send:36-60). | Not ported now. | deferred (non-Java backlog) | none |

## Data types
Value decoding and encoding coverage was not read line by line for either driver.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-DT-COV-PRESENT-VALUE | Java builds an event only for the `PRESENT_VALUE` property of a COV notification; other properties are not delivered (`bacnet` operations.subscribe; conn_cov_decode:169-270). | Same: a notification contributes an event for `PRESENT_VALUE` only. | planned (M5) | `planned:bacnet_cov_present_value_builds_an_event`, `planned:bacnet_cov_other_property_builds_no_event` |
| BAC-DT-COV-STRUCT | Java event is a `PlcStruct` with `deviceIdentifier`, `objectType`, `objectInstance`, `address`, `tagNumber` and `lengthValueType` (`bacnet` operations.subscribe). | A struct value with the same six fields. | planned (M5) | `planned:bacnet_cov_event_struct_has_the_six_fields` |
| BAC-DT-COV-DEVICE-ID | Both decode paths set `deviceIdentifier` from `getInstanceNumber()` of the monitored object, the same value as `objectInstance` (`bacnet` notes; conn_cov_decode). See quirk BAC-QUIRK-DEVICE-ID. | See the divergence row. | planned (M5) | `planned:bacnet_cov_device_identifier_is_not_the_object_instance` |
| BAC-DT-GO-VALUES | Go: ReadProperty and RPM acks decode to `PlcValue`s; anything else is INTERNAL_ERROR (read_decode:348-418). Coverage of `ValueDecoder.go`, `ValueEncoder.go`, `ValueHandler.go` is not established. | Not ported now. | deferred (non-Java backlog) | none |
| BAC-DT-GO-COV-VALUE | Go COV events carry only the value; the address and source maps are empty (sub_dispatch:294-334). | Not ported now. | deferred (non-Java backlog) | none |

## COV notification handling (Java)

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-COV-CONFIRMED | Java decodes ConfirmedCOVNotification from incoming BVLC frames (conn_cov_decode:169-270). Whether Java sends a SimpleAck is not in the cited lines (UNVERIFIED). | Decode and deliver the event. The acknowledgement behavior is fixed after the cited code is re-read in M5. | planned (M5) | `planned:bacnet_confirmed_cov_notification_builds_an_event` |
| BAC-COV-UNCONFIRMED | Java decodes UnconfirmedCOVNotification the same way (conn_cov_decode). | Decode and deliver the event. | planned (M5) | `planned:bacnet_unconfirmed_cov_notification_builds_an_event` |
| BAC-COV-NO-FILTER | Events go to every registered consumer with no tag or device filter, for every notification, including ones the consumer never subscribed to (conn_passive:88-93; conn_cov_decode). | Same fan-out; the README states that BACnet subscriptions do not filter. | planned (M5) | `planned:bacnet_cov_event_is_delivered_to_all_consumers` |
| BAC-COV-EDE-ENRICH | Events can be enriched from the EDE file or directory (`bacnet` operations.subscribe; config). The enrichment fields are not listed in the manifest (UNVERIFIED). | Enrichment from the loaded EDE model, field list to be read from the source in M5. | planned (M5) | `planned:bacnet_cov_event_is_enriched_from_the_ede_model` |
| BAC-COV-GO-ROUTING | Go routes notifications by subscriber process id; notifications for an unknown id are logged and dropped (sub_incoming:264-289); `c.subscribers` is iterated without the mutex (conn_route:166-201). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-COV-GO-REFRESH | Go comments promise a refresh at lifetime/2; no ticker or timer exists, so a subscription lapses at the device lifetime (refresh_missing:52-55). | None. | deferred (upstream placeholder) | none |
| BAC-COV-GO-SIMPLEACK | Go: the SimpleAck for a ConfirmedCOVNotification is a TODO and is never sent (conn_route:194). | None. | deferred (upstream placeholder) | none |

## Segmentation, routing and discovery (Go only)

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-SEG-REQUEST | Go segments an oversized request only when `peer-max-apdu-length-accepted` and the peer segmentation capability are configured; otherwise it fails fast (read_seg_req:143-162; seg_send:36-60). Backlog row `go_bacnetip` "segmentation and routing; configuration". | Not ported now. | deferred (non-Java backlog) | none |
| BAC-ROUTE-NPDU | Go: a device behind a router gets an NPDU destination specifier (DNET, DADR, hop count 255) (routing:34-90). Same backlog row. | Not ported now. | deferred (non-Java backlog) | none |
| BAC-DISC-WHOIS | Go: Who-Is broadcast on every selected interface and optional directed unicast to `remote-address(es)`; window `discovery-timeout-seconds` (discover_whois:138-183). A TODO says the context is not honoured during setup (discover_main:75-118). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-DISC-WHOHAS | Go: optional Who-Has by device range, object identifier or object name; I-Have items reported (discover_main; discover_results:316-385). | Not ported now. | deferred (non-Java backlog) | none |
| BAC-DISC-ROUTED | Go: a routed I-Am yields `RemoteNetwork` and `RemoteAddress` options on the discovery item (discover_results:316-385). | Not ported now. | deferred (non-Java backlog) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| BAC-QUIRK-DEVICE-ID | Java sets `deviceIdentifier` to the monitored object's instance number, which looks like a bug, not a device id. | conn_cov_decode:169-270; `bacnet` notes | planned (M5) | Not reproduced: `planned:bacnet_cov_device_identifier_is_not_the_object_instance`. See the divergence table. |
| BAC-QUIRK-PASSIVE-ALL | A subscription filters nothing: every consumer sees every notification. | conn_passive:88-93; conn_cov_decode | planned (M5) | Reproduced and documented: `planned:bacnet_cov_event_is_delivered_to_all_consumers`. |
| BAC-QUIRK-GO-NO-REFRESH | A Go COV subscription lapses because nothing refreshes it. | refresh_missing:52-55 | deferred (non-Java backlog) | none |
| BAC-QUIRK-GO-NO-ACK | A Go confirmed COV notification is never acknowledged. | conn_route:194 | deferred (non-Java backlog) | none |
| BAC-QUIRK-GO-RACE | `c.subscribers` is iterated without `subscribersMu`; whether this is a real race is not established. | conn_route:166-201 | deferred (non-Java backlog) | none |

## Divergences from upstream
Planned divergences only; a divergence row reuses the id of the behavior row it qualifies.

| Row id | Upstream behavior (file:line at ea229918) | Stonecrab behavior | Reference | Regression test id |
| --- | --- | --- | --- | --- |
| BAC-DT-COV-DEVICE-ID | `deviceIdentifier` equals the object instance (conn_cov_decode:169-270). | `deviceIdentifier` carries the initiating device's identifier from the notification when it has one; the wire field is to be confirmed in M5 against ASHRAE 135 (initiating device identifier). Planned, no issue drafted. | UNVERIFIED: ASHRAE 135 clause number not checked | `planned:bacnet_cov_device_identifier_is_not_the_object_instance` |

## Backlog from other languages
Every `[[backlog]]` row with `driver = "go_bacnetip"`. Java is the baseline; each of
these is `deferred (non-Java backlog)` in the tables above.

| Row id | Backlog capability and Go behavior (manifest) | Where recorded | Status | Test ids |
| --- | --- | --- | --- | --- |
| BAC-BACKLOG-READ | "read": ReadProperty and ReadPropertyMultiple with segmentation (evidence `read_build`). Java: unsupported. | BAC-OP-READ, BAC-LIM-GO-READ-REQUEST | deferred (non-Java backlog) | none |
| BAC-BACKLOG-WRITE | "write": WriteProperty and WritePropertyMultiple with priority and array index (evidence `write_build`). Java: unsupported. | BAC-OP-WRITE | deferred (non-Java backlog) | none |
| BAC-BACKLOG-SUBSCRIBE | "subscribe": sends SubscribeCOV, cancels with lifetime 0, routes by process id, never refreshes (evidence `sub_subscribe`). Java: passive listener. | BAC-OP-SUBSCRIBE-COV, BAC-OP-UNSUBSCRIBE-COV, BAC-COV-GO-REFRESH, BAC-COV-GO-SIMPLEACK | deferred (non-Java backlog) | none |
| BAC-BACKLOG-DISCOVER | "discover": Who-Is, Who-Has and directed Who-Is discovery with routed-origin options (evidence `discover_main`). Java: unsupported. | BAC-OP-DISCOVER, BAC-DISC-WHOIS, BAC-DISC-WHOHAS, BAC-DISC-ROUTED | deferred (non-Java backlog) | none |
| BAC-BACKLOG-TAG-SYNTAX | "tag syntax": defined syntax that round-trips (evidence `tag_syntax`). Java: TODO placeholder. | BAC-ADDR-GO-SYNTAX and the other BAC-ADDR-GO rows | deferred (non-Java backlog) | none |
| BAC-BACKLOG-SEGMENTATION | "segmentation and routing; configuration": segmented request and response, routed destination specifier, options (evidence `routing`). Go parses BBMD and static devices but never uses them. Java: neither. | BAC-SEG-REQUEST, BAC-ROUTE-NPDU, BAC-CFG-GO rows | deferred (non-Java backlog) | none |

The manifest also records language gaps in the other direction (Java has them, Go does
not): EDE enrichment (BAC-CFG-EDE-FILE, BAC-COV-EDE-ENRICH), passive reception without a
prior subscribe (BAC-COV-NO-FILTER), the rich COV event struct (BAC-DT-COV-STRUCT) and the
`test` transport (BAC-TRANSPORT-TEST).

## UNVERIFIED
- The BACnet mspec files (`bacnetip.mspec` and the `bacnet-*.mspec` files) were listed but
  not opened, so which BACnet services the wire model covers is unknown.
- `BacNetIpMessageCodec.java` was not read.
- The Java default UDP port and any Java local-bind option are not in the cited lines.
- Whether Java acknowledges a ConfirmedCOVNotification, and what the EDE enrichment
  fields are.
- Which application and constructed BACnet types round-trip in the Go value
  encoder/decoder.
- The segmentation windowing and NAK behavior in the Go segmenters beyond their doc
  comments.
- Whether the unlocked iteration of `c.subscribers` in the Go router is a real race.
- ASHRAE 135 clause numbers (5.4 is cited by the Go source for segmentation); no copy of
  the standard is retained in this repository.
- "Complete" for the Go driver is not established; only that its entry points exist and
  that the four listed defects were read from source.
