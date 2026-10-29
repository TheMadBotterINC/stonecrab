# EtherNet/IP and Logix parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing in this record is implemented. Every row is `planned` for
milestone M4 (S7 and EtherNet/IP with Logix, in the order decision E5 sets) or `deferred`
with its reason. The Go driver (`go_eip`) is recorded in its own section and in the
backlog table. The `eip` and `logix` protocol codes share one Java connection class, one
configuration class and one tag class, so they share this file; the sections say which code
a row belongs to. Test ids are `planned:` names, not existing tests, and no hardware or
device model is named yet (decision E5: decided when a test PLC is available).

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: recorded so the omission is visible; Stonecrab does not port it now.
  `deferred (upstream placeholder)` marks a skeleton, a null builder or a "not implemented"
  upstream, which does not become an obligation (SPEC, "Compatibility baseline").
- No row is `implemented`, `automated-verified` or `hardware-verified` yet, so
  `scripts/parity-check.sh` does not require these ids to appear in code.

## File key
Citations use the manifest key or a path:lines range at the pinned commit. Paths are
relative to the repository root of apache/plc4x. `J` abbreviates
`plc4j/drivers/eip/src/main/java/org/apache/plc4x/java/eip`.

| Key | Path |
| --- | --- |
| driver | `J/base/EIPDriver.java` (manifest `inventory.eip.driver`) |
| connection_class, connection_read, connection_write | `J/base/EipTcpConnection.java` |
| polling | `plc4j/utils/subscription-emulation/.../PollingSubscriptionConnectionBase.java` |
| metadata | `plc4j/spi/drivers/.../spi/drivers/ConnectionBase.java` |
| discovery, discovery_request, discovery_timeout | `J/base/discovery/EipPlcDiscoverer.java` |
| tag | `J/base/tag/EipTag.java` |
| config | `J/base/configuration/EIPConfiguration.java` |
| mspec_packet, mspec_cip | `protocols/eip/src/main/resources/protocols/eip/eip.mspec` |
| Logix driver | `J/logix/LogixDriver.java` (manifest `inventory.logix.driver`) |
| go_* keys | `plc4go/internal/eip/*.go`, see the Go section |

## Operations (protocol code `eip`)

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-OP-CONNECT | Protocol code `eip`, default transport `tcp`, default port 44818 (driver 46-84). Connect runs ListServices, RegisterSession, probes ConnectionManager and MessageRouter class support, and opens Forward_Open through the Connection Manager unless `force-unconnected-operation` is set (EipTcpConnection 237-243; manifest note on connected and unconnected operation). | Connect to `eip:tcp://host` with the same handshake and the same choice between connected and unconnected operation. | planned (M4) | `planned:eip_connect_registers_a_session_and_probes_the_router`, `planned:eip_connect_falls_back_to_unconnected_when_forced` |
| EIP-OP-CLOSE | Close sends Forward_Close and UnregisterSession (Go counterpart, go_close 159-196; the Java close path is not cited by the manifest, see UNVERIFIED). | Close sends Forward_Close when a connection is open, then UnregisterSession, and always drops the transport. | planned (M4) | `planned:eip_close_sends_forward_close_then_unregister_session` |
| EIP-OP-READ | CIP Read Tag (service 0x4C). `onRead` picks one of three paths: unconnected request per tag (no message router), one unconnected message-router request, or connected SendUnitData after Forward_Open (connection_read 524-533; mspec_cip 121-162). | Read with the same three modes, selected by session state, returning per-item response codes. | planned (M4) | `planned:eip_read_of_a_dint_tag_decodes_little_endian`, `planned:eip_read_uses_the_connected_path_after_forward_open`, `planned:eip_read_uses_the_unconnected_path_without_a_message_router` |
| EIP-OP-WRITE | CIP Write Tag (service 0x4D), same three paths (connection_write 737-746; mspec_cip 121-162). | Write with the same three modes. | planned (M4) | `planned:eip_write_of_a_dint_tag_encodes_a_write_tag_request`, `planned:eip_write_uses_the_connected_path_after_forward_open` |
| EIP-OP-SUBSCRIBE | Polling emulation only: `EipTcpConnection` extends `PollingSubscriptionConnectionBase`; no native subscription; default interval 1000 ms (polling 100-106, 385-387). `EIPDriver` does not override `canSubscribe`, yet connection metadata derives subscribe from the hook (metadata 268-274; manifest UNCERTAIN note). | Cyclic and change-of-state subscribe through the shared Stonecrab polling layer over the read path; event subscribe is refused. Capability reporting follows the connection, not the driver flag. | planned (M4) | `planned:eip_subscribe_polls_the_read_path_at_the_default_interval`, `planned:eip_capabilities_report_subscribe_from_the_connection` |
| EIP-OP-BROWSE | Not implemented: no `canBrowse`, no `onBrowse` (manifest `inventory.eip.operations.browse`). | `Capabilities.browse = false`; a browse request fails with the typed unsupported error. | deferred (upstream placeholder) | none |
| EIP-OP-DISCOVER | `EipPlcDiscoverer` broadcasts ListIdentity (command 0x0063) over UDP to every non-loopback IPv4 interface, collects CipIdentity items for 5000 ms and returns items with protocol `eip`, transport `tcp`, `host:44818`, name = product name (discovery 91-100, discovery_request 137-153, discovery_timeout 177-187). | Same broadcast and 5 s window; items use transport `tcp` (the Java choice; the Go choice is the backlog row EIP-BL-DISCOVERY-TRANSPORT). | planned (M4) | `planned:eip_discover_parses_a_list_identity_reply_into_an_item`, `planned:eip_discover_completes_after_the_five_second_window` |
| EIP-OP-PING | Not implemented: no `canPing`, no `onPing` (manifest `inventory.eip.operations.ping`). | `Capabilities.ping = false`. | deferred (upstream placeholder) | none |

## Capabilities

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-CAP-FLAGS | `canDiscover`, `canRead`, `canWrite` true; nothing else overridden (driver 91-104). | Capability set `{read, write, discover, subscribe}` for `eip`, with subscribe coming from the polling layer. | planned (M4) | `planned:eip_capability_set_matches_the_inventory` |

## Configuration
Seven parameters on `EIPConfiguration` (config 28-64).

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-CFG-BACKPLANE | `backplane`, int, default 1; overridden when `communication-path` is given (config 28-64). | Same name and default. | planned (M4) | `planned:eip_config_backplane_defaults_to_one` |
| EIP-CFG-SLOT | `slot`, int, default 0 (config 28-64). | Same name and default. | planned (M4) | `planned:eip_config_slot_defaults_to_zero` |
| EIP-CFG-BIG-ENDIAN | `big-endian`, boolean, default true; the Logix driver forces little-endian (config 28-64; manifest note). Go also accepts `bigEndian` (go_config_parse 65-86). | Same name and default for `eip`; `logix` ignores the option and is little-endian. The camelCase alias is a Go-only spelling, see EIP-BL-GO-OPTION-ALIASES. | planned (M4) | `planned:eip_config_big_endian_defaults_to_true`, `planned:logix_forces_little_endian_regardless_of_the_option` |
| EIP-CFG-SERIAL | `connection-serial-number`, int, default 0 meaning a random number per connection; since 1.0.0 (config 28-64). | Same name; 0 draws a random serial, an explicit value is used verbatim for reproducible exchanges. | planned (M4) | `planned:eip_config_serial_zero_picks_a_random_forward_open_serial`, `planned:eip_config_serial_explicit_is_used_verbatim` |
| EIP-CFG-FORCE-UNCONNECTED | `force-unconnected-operation`, boolean, default false; since 0.13.0 (config 28-64). | Same name and default. | planned (M4) | `planned:eip_config_force_unconnected_skips_forward_open` |
| EIP-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, default 10000 (config 28-64). | Same name and default, mapped onto the request timeout. | planned (M4) | `planned:eip_config_request_timeout_defaults_to_ten_seconds` |
| EIP-CFG-COMMUNICATION-PATH | `communication-path`, String, pairs of 1 (backplane, followed by a slot) or 2 (Ethernet, followed by an IP address), for example `[1,4,2,192.168.0.1,1,1]` (config 28-64). | Same name and format; a malformed path is a typed `InvalidConfig`. How Java parses the path is UNVERIFIED. | planned (M4) | `planned:eip_config_communication_path_builds_a_routing_path`, `planned:eip_config_communication_path_rejects_a_malformed_list` |

## Connection string and transports

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-TRANSPORT-TCP | Default and supported transport `tcp`, default port 44818 (driver 68-84; `inventory.eip.transports`). | `eip:tcp://host[:port]`, port default 44818. | planned (M4) | `planned:eip_uri_defaults_to_tcp_port_44818` |
| EIP-TRANSPORT-TEST | `test` transport supported (driver 73-76); internals not read. | Not ported; Stonecrab tests use `FakeTransport`. Other transports are rejected with `InvalidConfig`. | deferred (no release) | none |
| EIP-TRANSPORT-UDP-DISCOVERY | UDP broadcast on the discovery port, used only by discovery (discovery_request 137-153). | UDP is used by discovery only; it is not a connection transport. | planned (M4) | `planned:eip_uri_with_a_udp_transport_is_rejected` |

## Address grammar
Java pattern `^(?<tag>[%a-zA-Z_.0-9]+)<ARRAY_GROUP>(?::(?<dataType>[A-Z]+))?$` (tag 44-47).
The tag name class includes a leading `%`, so `MyTag:DINT` and `%MyTag:DINT` both parse
(Go requires the `%`, see EIP-BL-GO-TAG-PREFIX). The default type when no `:TYPE` is given
is not cited by the manifest for Java (Go: DINT) and is listed under UNVERIFIED.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-ADDR-TAG | Tag name from `[%a-zA-Z_.0-9]+`, dotted members and `[index]` array members decomposed into a path (tag 44-60; PATH_PATTERN skips a leading `%`). | `ControllerTag` value type with `parse`, same accepted names; canonical form without the `%`. | planned (M4) | `planned:eip_address_parses_a_dotted_member_path`, `planned:eip_address_accepts_a_leading_percent` |
| EIP-ADDR-ARRAY | Selection `[n]` or range via `ArrayNotationParser.ARRAY_GROUP`; one dimension, start index at most 255, only a trailing dimension may be a range (tag 51-53); up to 65535 elements (manifest note on `go_tag_constraints`). | Same constraints, each violation a typed `InvalidAddress`. | planned (M4) | `planned:eip_address_rejects_a_start_index_above_255`, `planned:eip_address_rejects_two_dimensions`, `planned:eip_address_rejects_a_range_that_is_not_trailing` |
| EIP-ADDR-TYPE-SUFFIX | Optional `:TYPE`, upper-case letters only (tag 44-47). | Optional upper-case type suffix naming a CIP type. | planned (M4) | `planned:eip_address_type_suffix_must_be_upper_case` |
| EIP-ADDR-QUERY | Tag query/browse forms: none in Java; Go `ParseQuery` answers "queries not supported" (go_parse_query 99-101). | Queries are rejected. | deferred (upstream placeholder) | none |

## Limits

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-LIMIT-ARRAY | Start index at most 255, one dimension (tag 51-53); at most 65535 elements (manifest note). | Enforced by the address parser. | planned (M4) | `planned:eip_address_rejects_more_than_65535_elements` |
| EIP-LIMIT-PACKET | Per-request payload limits of the Connection Manager and Message Router paths: not cited by the manifest. | Recorded once read; no limit is assumed. | deferred (not inventoried; see UNVERIFIED) | none |

## Data types
The Java value handling of `EipTcpConnection` was not compared type by type (manifest
UNCERTAIN note on `go_eip`). The Go driver decodes the fixed-size CIP types below
(`go_values` 63, `go_values_structured` 233-251) and the mspec CIP service block is
`mspec_cip` 121-162.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-TYPE-FIXED | BOOL, SINT, INT, DINT, LINT, USINT, UINT, UDINT, ULINT, BYTE, WORD, DWORD, LWORD, REAL, LREAL (Go go_values 63, a cross-check for Java). | Typed decode and encode for each, byte order from `big-endian`. | planned (M4) | `planned:eip_read_of_a_dint_tag_decodes_little_endian`, `planned:eip_read_of_a_real_tag_decodes_ieee754`, `planned:eip_write_of_a_bool_tag_encodes_one_byte` |
| EIP-TYPE-STRING | STRING and STRUCTURED(string) decoded separately (Go go_values_structured 233-251). | STRING read and write; STRUCTURED beyond the string layout is not owed. | planned (M4) | `planned:eip_read_of_a_string_tag_decodes_the_length_prefix` |
| EIP-TYPE-STRUCTURED | Other structured and user-defined types: not inventoried. | Recorded once read. | deferred (not inventoried; see UNVERIFIED) | none |

## Logix (protocol code `logix`)
A thin alias: its own `DriverBase` subclass with code `logix` and name "Logix CIP", the
same connection class, configuration and tag, forced little-endian, `canRead` and
`canWrite` only (Logix driver 39-111; manifest `inventory.logix`).

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| LGX-OP-CONNECT | Same connection class and handshake as `eip`; `getConnection` passes `bigEndian=false` (Logix driver 101-106). | `logix:tcp://host` reuses the EIP connection with little-endian fixed. | planned (M4) | `planned:logix_connect_uses_the_eip_handshake_with_little_endian` |
| LGX-OP-READ | `canRead` true, `EipTcpConnection.onRead` (Logix driver 91-99). | Same as EIP-OP-READ. | planned (M4) | `planned:logix_read_of_a_dint_tag_decodes_little_endian` |
| LGX-OP-WRITE | `canWrite` true, `onWrite`. | Same as EIP-OP-WRITE. | planned (M4) | `planned:logix_write_of_a_dint_tag_encodes_little_endian` |
| LGX-OP-SUBSCRIBE | Polling emulation via the shared connection class; `canSubscribe` not overridden (`inventory.logix.operations.subscribe`). | Same as EIP-OP-SUBSCRIBE. | planned (M4) | `planned:logix_subscribe_polls_the_read_path` |
| LGX-OP-BROWSE | Not implemented. | `Capabilities.browse = false`. | deferred (upstream placeholder) | none |
| LGX-OP-DISCOVER | None: neither `canDiscover` nor `discoveryRequestBuilder` is overridden; `EIPDriver`'s discoverer is not wired in (nodiscover 117-139). | `logix` does not discover, matching Java. The Go difference is backlog row LGX-BL-GO-DISCOVERY. | planned (M4) | `planned:logix_capabilities_exclude_discovery` |
| LGX-OP-PING | Not implemented. | `Capabilities.ping = false`. | deferred (upstream placeholder) | none |
| LGX-CFG-SHARED | Shares `EIPConfiguration` with `eip`; only the code, the name and forced little-endian differ (manifest note). | Same seven options as the EIP-CFG rows. | planned (M4) | `planned:logix_accepts_the_same_options_as_eip` |
| LGX-TRANSPORT-TCP | Default `tcp`, supported `tcp` and `test` (Logix driver 73-81). | As EIP-TRANSPORT-TCP and EIP-TRANSPORT-TEST. | planned (M4) | `planned:logix_uri_defaults_to_tcp_port_44818` |
| LGX-ADDR-SHARED | Shares `EipTag`. | Same grammar as the EIP-ADDR rows. | planned (M4) | `planned:logix_address_grammar_matches_eip` |

## Go driver (`go_eip`)
Go registers codes `eip` and `EthernetIP`, plus `logix` as the same struct with
`forceLittleEndian` (go_ctor 51-64, go_logix 66-84). Go matches Java in the three read and
write modes and in discovery, differs in the places the Backlog table lists, and exposes no
subscription.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-GO-READ-WRITE | Three read and three write paths chosen by session state (go_read_modes 78-85, go_write_modes 72-79); requests bounded by the caller context only (go_requests 83-119). | Covered by EIP-OP-READ and EIP-OP-WRITE; the Rust timeout comes from `request-timeout-ms`. | planned (M4) | `planned:eip_read_times_out_after_the_request_timeout` |
| EIP-GO-SUBSCRIBE | `ProvidesSubscribing=false`; the builder is inherited and panics (go_metadata 477-487, go_subscribe_panic 242-244). | Not ported; Java polling is the baseline (EIP-OP-SUBSCRIBE). | deferred (Go-only gap, Java polls) | none |
| EIP-GO-BROWSE | `ProvidesBrowsing=false`. | As EIP-OP-BROWSE. | deferred (upstream placeholder) | none |
| EIP-GO-TRANSPORT | The transport is taken from the supplied map with no allow-list (go_transport_lookup 93-100). | Stonecrab enforces its own allow-list (EIP-TRANSPORT-TCP). | deferred (not ported; Rust enforces an allow-list) | none |

## Upstream quirks

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-QUIRK-SUBSCRIBE-FLAG | The driver-level `canSubscribe` is false while the connection metadata reports subscribe true; no SPI code reads the driver-level flag (manifest UNCERTAIN note). | Report subscribe from the connection only. | planned (M4) | `planned:eip_capabilities_report_subscribe_from_the_connection` |
| EIP-QUIRK-DISCOVERY-TRANSPORT | Java discovery items carry transport `tcp`; Go's carry `udp` with URL `eip://host:44818` (manifest note on `go_eip`). | Items carry `tcp`, so an item can be connected to unchanged. | planned (M4) | `planned:eip_discovery_item_connects_without_editing` |
| EIP-QUIRK-FORWARD-OPEN-REJECT | Java throws on a rejected Forward_Open, failing the connect (EipTcpConnection 338-351). | Fail the connect with a typed error; fallback is backlog row EIP-BL-FORWARD-OPEN-FALLBACK. | planned (M4) | `planned:eip_connect_fails_when_forward_open_is_rejected` |

## Divergences from upstream
No divergence is planned yet. A row is added here, with its rationale, when a milestone
chooses to differ.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-DIV-NONE | None recorded. | None. | planned (M4) | `planned:eip_divergence_table_is_reviewed_at_m4_exit` |

## Backlog from other languages
The first four rows are the `[[backlog]]` entries of the manifest for `go_eip`; the rest are
the Go `LANG GAP` notes of `inventory.go_eip`. All are deferred until a maintainer decides
to adopt them, with Java as the baseline.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| EIP-BL-FORWARD-OPEN-FALLBACK | "Connect resilience to a rejected ForwardOpen": Go catches the error and continues unconnected; Java fails the connect (go_setup 218-247, java_forward_open_failure 338-351). | Not ported; Java fails the connect. | deferred (non-Java backlog) | none |
| LGX-BL-GO-DISCOVERY | "Discovery on the logix alias": Go's `logix` shares the Driver whose discovery is true (go_logix 66-84); Java's has none. | Not ported. | deferred (non-Java backlog) | none |
| EIP-BL-DISCOVERY-INTERFACE | "Per-interface discovery selection": Go filters interfaces by a device-name option and runs one socket per interface (go_discoverer 117-199). | Not ported; Java broadcasts on all interfaces. | deferred (non-Java backlog) | none |
| EIP-BL-PING | "Ping": Go inherits a local flag check that never touches the wire (go_ping_default 200-208). | Not ported. | deferred (non-Java backlog) | none |
| EIP-BL-DISCOVERY-TRANSPORT | Go reports transport `udp` (go_discovery_item 273-285). | Not ported (see EIP-QUIRK-DISCOVERY-TRANSPORT). | deferred (non-Java backlog) | none |
| EIP-BL-GO-TAG-PREFIX | Go requires a literal `%` prefix (go_tag_pattern 40). | Not ported; Java accepts both spellings. | deferred (non-Java-only difference) | none |
| EIP-BL-GO-OPTION-ALIASES | Go accepts `bigEndian` and `big-endian` (go_config_parse 65-86). | Not ported. | deferred (non-Java-only difference) | none |
| EIP-BL-GO-DISCOVERY-WINDOW | Go reads 5 s per interface on UDP 44818 (go_discovery_window 46-54). | Not ported; Java uses one 5 s window. | deferred (non-Java-only difference) | none |

## UNVERIFIED
- The default type of an address with no `:TYPE` in Java (Go: DINT, go_tag_default_type 68-71).
- The Java close path (Forward_Close and UnregisterSession), cited here only through Go.
- Whether Java parses `communication-path` as Go's `buildRoutingAddress` does.
- CIP data-type coverage of `EipTcpConnection` against the Go table.
- Message and packet limits of the Connection Manager and Message Router paths.
- The behavior of the `test` transport.
- No device model or firmware is named: hardware evidence waits for a test PLC (E5).
