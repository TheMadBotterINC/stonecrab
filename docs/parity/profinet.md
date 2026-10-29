# PROFINET parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. Every row is `planned` or `deferred`. PROFINET
is milestone M5 (`docs/POST_ALPHA_PLAN.md` section 4: "browse, subscribe and discovery
only, no read, write or ping"). Two upstream modules register the same protocol code
`profinet`: the legacy driver `plc4j/drivers/profinet` (manifest `inventory.profinet`) and
the next-generation port `plc4j/drivers/profinet-ng` (`inventory.profinet_ng`). They are
alternatives, not coexisting drivers, so this record carries one set of rows per module
and Stonecrab ships one `profinet` driver whose choice of model is made at M5 (row
PN-ALTERNATIVES). No `[[backlog]]` row names PROFINET and no Go, C, Python or C#
implementation exists at the pinned commit; the section "Backlog from other languages"
says so explicitly. The S7 discovery dependency on PROFINET DCP is recorded once, in
`docs/parity/s7.md` row `S7-DISCOVER-DCP`; `PN-DISCOVER` here reuses that codec.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one test id prefixed `planned:`; no such test exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now. The reason is an
  upstream placeholder, an operation upstream does not have, or a pending choice. Deferred
  rows may carry `none` as test id.
- `implemented`, `automated-verified` and `hardware-verified` are not used yet; they need
  real tests (and, for hardware, device models and firmware) as `docs/SPEC.md` requires.

Unimplemented upstream placeholders do not become obligations; they are listed as
`deferred (upstream placeholder)`. Row ids are stable and never renumbered. Ids with the
infix `-L-` describe the legacy module, `-NG-` the next-generation module.

## File key
Citations use manifest keys written `entry.key`. `L` abbreviates
`plc4j/drivers/profinet/src/main/java/org/apache/plc4x/java/profinet` and `NG`
abbreviates `plc4j/drivers/profinet-ng/src/main/java/org/apache/plc4x/java/profinet`.

| Key | Path |
| --- | --- |
| LDrv (`profinet.driver`) | `L/ProfinetDriver.java` |
| LConn (`profinet.connect`, `browse_subscribe`, `connection_class`) | `L/ProfinetConnection.java` |
| LDev (`profinet.cyclic_dispatch`, `device_register`) | `L/device/ProfinetDevice.java` |
| LTag (`profinet.tag`) | `L/tag/ProfinetTag.java` |
| LCfg (`profinet.config`) | `L/config/ProfinetConfiguration.java` |
| LRaw (`profinet.raw_socket_cfg`) | `L/config/ProfinetRawSocketTransportConfiguration.java` |
| CB (`profinet.base_defaults`) | `plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/ConnectionBase.java` |
| NGDrv (`profinet_ng.driver`) | `NG/ProfinetDriver.java` |
| NGConn (`profinet_ng.class_doc`, `connect`, `browse`, `subscribe_todo`, `subscribe_warn`, `unsub_register`) | `NG/ProfinetConnection.java` |
| NGTag (`profinet_ng.tag`) | `NG/tag/ProfinetTag.java` |
| NGCfg (`profinet_ng.config`) | `NG/config/ProfinetConfiguration.java` |
| NGReadme (`profinet_ng.readme_conn`, `readme_auto`) | `plc4j/drivers/profinet-ng/README.md` |
| BrowseItem (`profinet_ng.browse_item_ctor`) | `plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/messages/DefaultPlcBrowseItem.java` |

## Operations
Both modules implement browse, subscribe and discover and nothing else. There is no
acyclic read, write or ping: PROFINET IO here is the controller receiving cyclic data from
configured devices.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-ALTERNATIVES | Both `profinet` and `profinet-ng` declare `DRIVER_CODE = "profinet"`; they are alternatives (`profinet.notes`, `profinet_ng.notes`). | One `profinet` driver. The M5 slice picks the model (GSDML-driven single device per connection as in `profinet-ng`, or the configured device list of the legacy module) and records the choice here. | planned (M5) | `planned:profinet_driver_registers_a_single_protocol_code` |
| PN-L-CONNECT | Needs the raw-socket transport with a local IPv4 address; opens a pcap channel on that interface and a UDP datagram socket; runs LLDP and PN-DCP discovery for each configured device; each device performs DCE/RPC Connect, WriteParameters, ParameterEnd and ApplicationReady; waits up to 60 s for all devices to reach ApplicationReady (`profinet.connect`, LConn:139-190; `profinet.notes`). | If the legacy model is chosen: the same handshake, with the 60 s wait as a configurable timeout. | planned (M5) | `planned:profinet_connect_walks_connect_parameter_end_application_ready`, `planned:profinet_connect_times_out_after_sixty_seconds_without_application_ready` |
| PN-NG-CONNECT | Needs raw-socket with local MAC and IPv4; remote MAC from the transport, remote IP from `ip-address`; sends a PN-DCP identify unicast, extracts block info, continues with the RPC connect flow (`profinet_ng.connect`, NGConn:145-200; `profinet_ng.notes`). Which parts of the path are implemented rather than only described in the README was not verified (UNCERTAIN). | If the next-generation model is chosen: the same flow, DCP identify unicast then RPC connect. | planned (M5) | `planned:profinet_ng_connect_sends_a_dcp_identify_unicast_first`, `planned:profinet_ng_connect_matches_the_gsd_by_vendor_and_device_id` |
| PN-CLOSE | Not separately inventoried for either module; the manifest names connect and the operation hooks only. | Close releases the pcap channel and UDP socket and ends cyclic exchange. | planned (M5) | `planned:profinet_close_releases_the_channel_and_socket` |
| PN-L-BROWSE | `onBrowse` returns the tags of all configured devices via `ProfinetDevice.browseTags`, the same list for every query, code OK (`profinet.operations.browse`, LConn:245-289). | Browse returns the configured devices' tags. | planned (M5) | `planned:profinet_browse_returns_the_tags_of_all_configured_devices` |
| PN-NG-BROWSE | Offline from the GSDML: after connect, one browse item per GSDML data item for each slot and subslot. Each item has `readable = (direction == OUTPUT)` and `writable = (direction == INPUT)`; INPUT items advertise CYCLIC (`profinet_ng.operations.browse`, NGConn:460-515; BrowseItem:45-49 for the constructor order). | Browse from the GSDML after connect with the same readable and writable rule and CYCLIC advertised for INPUT. | planned (M5) | `planned:profinet_ng_browse_lists_one_item_per_gsdml_data_item`, `planned:profinet_ng_browse_marks_output_items_readable_and_input_items_writable`, `planned:profinet_ng_browse_advertises_cyclic_for_inputs` |
| PN-L-SUBSCRIBE | Native cyclic IO. `onSubscribe` maps each tag to a configured device (first dot-separated address segment, upper-cased) and creates a `ProfinetSubscriptionHandle`; an unknown device gives INVALID_ADDRESS. `ProfinetDevice` dispatches parsed cyclic RT frames to consumers; CYCLIC, CHANGE_OF_STATE (against the last value) and EVENT are all handled (`profinet.cyclic_dispatch`, LDev:411-462; LConn:245-289). The connection class has no `onRegisterConsumer` override and the base default throws "Register consumer not supported", so consumption through the public API is doubtful (`profinet.base_defaults`, CB:437-471; UNCERTAIN). | If the legacy model is chosen: subscribe with the three modes and an explicit consumer path, which the upstream API lacks. The unknown-device answer is INVALID_ADDRESS. | planned (M5) | `planned:profinet_subscribe_to_an_unknown_device_is_invalid_address`, `planned:profinet_cyclic_subscribe_delivers_each_cyclic_frame`, `planned:profinet_change_of_state_delivers_only_changed_values` |
| PN-L-SUBSCRIBE-CONSUMER | The working `registerConsumer` lives on `ProfinetDevice` (LDev:162-182), not on the connection (`profinet.device_register`). | Consumer registration is part of the Stonecrab subscription handle; the upstream gap is not reproduced. | deferred (upstream gap; not reproduced) | none |
| PN-NG-SUBSCRIBE | Declared (`canSubscribe` true) but incomplete by the source's own admission: `onSubscribe` builds the DCE/RPC Connect with expected submodules for the requested INPUT and OUTPUT tags, runs ParameterEnd and acknowledges ApplicationReady, but the returned future is left incomplete (TODO "complete the future with a real PlcSubscriptionResponse"); cyclic frames are logged, not dispatched (`profinet_ng.operations.subscribe`, `class_doc`, NGConn:88-101, 566-576, 606-624). | None. No event is ever delivered upstream. | deferred (upstream placeholder) | none |
| PN-NG-UNSUBSCRIBE | `onUnsubscribe` returns an empty response and does nothing (`profinet_ng.unsub_register`, NGConn:780-796). | None. | deferred (upstream placeholder) | none |
| PN-NG-REGISTER-CONSUMER | `onRegisterConsumer` throws `UnsupportedOperationException` ("cyclic IO dispatch is not yet implemented") (NGConn:780-796). | None. | deferred (upstream placeholder) | none |
| PN-DISCOVER | Legacy: `discoveryRequestBuilder` opens pcap handles on all interfaces and runs `ProfinetPlcDiscoverer` (PN-DCP identify plus LLDP); requires pcap (`profinet.operations.discover`). Next-generation: the pcap PN-DCP discoverer `ProfinetDiscoverer` over `ProfinetChannel` on all interfaces (`profinet_ng.operations.discover`). | DCP identify plus LLDP discovery behind the pcap feature, reusing the IdentifyAll codec of `S7-DISCOVER-DCP` in `docs/parity/s7.md`. Without pcap the call returns a typed unsupported error. | planned (M5) | `planned:profinet_discovery_reports_name_of_station_and_ip`, `planned:profinet_discovery_reads_lldp_neighbor_data`, `planned:profinet_discovery_without_pcap_returns_a_typed_error` |
| PN-READ | None in either module: no `onRead` override; the base default fails with "Read not supported" (`profinet.operations.read`, `profinet_ng.operations.read`). | Not offered; `Capabilities.read` is false. | deferred (not in upstream) | none |
| PN-WRITE | None in either module ("Write not supported") (`profinet.operations.write`, `profinet_ng.operations.write`). | Not offered. | deferred (not in upstream) | none |
| PN-PING | None in either module ("Ping not supported"; `canPing` not overridden) (`profinet.operations.ping`, CB:469-471). | Not offered. | deferred (not in upstream) | none |

## Capabilities
Both `ProfinetDriver` classes set `canSubscribe`, `canBrowse` and `canDiscover` true and
do not set `canRead`, `canWrite` or `canPing` (`profinet.driver`, LDrv:42-105;
`profinet_ng.driver`, NGDrv:42-109).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-CAP-FLAGS | Driver flags subscribe, browse, discover true; read, write, ping false (LDrv:42-105, NGDrv:42-109). | `Capabilities` reports exactly browse, subscribe and discover. Subscribe is reported only for the model that delivers events. | planned (M5) | `planned:profinet_capabilities_report_browse_subscribe_and_discover_only` |

## Configuration
The two modules have disjoint option sets.

| Row id | Upstream name, type, default (path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-L-CFG-DEVICES | `devices`: device name, ip, device access and submodules, converted by `ProfinetDeviceConvertor` (`profinet.notes`, LCfg:63-87). | Option parsed as a device list. | planned (M5) | `planned:profinet_config_parses_a_device_list` |
| PN-L-CFG-GSDDIR | `gsddirectory`, converted by `ProfinetGsdFileConvertor`; directory of GSD files used to look up the GSD of found devices (LCfg:63-87). | Option carried. | planned (M5) | `planned:profinet_config_gsddirectory_loads_gsd_files` |
| PN-L-CFG-SENDCLOCK | `sendclockfactor`, int, 32; scales the cyclic send frequency (LCfg:63-87). | Option carried. | planned (M5) | `planned:profinet_config_sendclockfactor_defaults_to_32` |
| PN-L-CFG-REDUCTION | `reductionratio`, int, 4; cycle time = SendClockFactor * ReductionRatio * 31.25 us (the upstream description prints 31.23 us) (LCfg:63-87). | Option carried; the cycle-time formula is computed from the options. | planned (M5) | `planned:profinet_config_reductionratio_defaults_to_4`, `planned:profinet_cycle_time_follows_the_documented_formula` |
| PN-L-CFG-WATCHDOG | `watchdogfactor`, int, 50; cycles a device may miss before an alarm (LCfg:63-87). | Option carried. | planned (M5) | `planned:profinet_config_watchdogfactor_defaults_to_50` |
| PN-L-CFG-DATAHOLD | `dataholdfactor`, int, 50; must be at least the watchdog factor (LCfg:63-87). | Option carried; a smaller value is rejected with a typed error. | planned (M5) | `planned:profinet_config_dataholdfactor_defaults_to_50`, `planned:profinet_config_rejects_dataholdfactor_below_watchdogfactor` |
| PN-NG-CFG-GSD-DIR | `gsd-directory`, string, `~/.gsd`; searched by vendor-id and device-id (NGCfg:38-47, NGReadme:21-43). | Option carried with the default. | planned (M5) | `planned:profinet_ng_config_gsd_directory_defaults_to_home_gsd` |
| PN-NG-CFG-DAP-ID | `dap-id`, string, no default; the DAP is chosen automatically when absent (NGCfg:38-47, NGReadme:108-120). | Option carried; automatic choice when absent. | planned (M5) | `planned:profinet_ng_dap_is_chosen_automatically_without_dap_id` |
| PN-NG-CFG-IP | `ip-address`, string, no default; remote IP for devices without an IP in the URL (NGCfg:38-47). | Option carried. | planned (M5) | `planned:profinet_ng_config_ip_address_sets_the_remote_ip` |

## Connection string and transports
The only transport is `raw-socket` (`profinet.transports`, `profinet_ng.transports`). The
raw-socket transport configuration pins UDP port 34964 as the default (LRaw:20-32).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-URI-RAW-SOCKET | Protocol code `profinet`, default and only transport `raw-socket`, which needs a local IPv4 address (and, for `profinet-ng`, the local MAC) (`profinet.connect`, `profinet_ng.notes`). | The `raw-socket` transport behind the pcap feature, on the transport list of M5. | planned (M5) | `planned:profinet_uri_selects_the_raw_socket_transport` |
| PN-URI-NG-IP | `profinet://192.168.24.31`: an ARP lookup resolves the MAC (NGReadme:21-43). | ARP lookup then the connect flow. | planned (M5) | `planned:profinet_ng_uri_with_an_ip_resolves_the_mac_by_arp` |
| PN-URI-NG-MAC | `profinet://<mac>?ip-address=...` for devices without an IP (NGReadme:21-43). | MAC form with the `ip-address` option. | planned (M5) | `planned:profinet_ng_uri_with_a_mac_uses_the_ip_address_option` |
| PN-URI-UDP-PORT | UDP port 34964 for implicit communication (LRaw:20-32). | Default port 34964. | planned (M5) | `planned:profinet_raw_socket_default_port_is_34964` |
| PN-URI-NONE-OTHER | The manifest cites no other transport. | None. | deferred (not in upstream) | none |

## Address grammar
| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-L-ADDR | `{address}[array]:TYPE` with `address` matching `[\w\-. ]+` and TYPE a PlcValueType, for example `foo.bar[0..3]:DINT`. Named addressing: the first dot-separated segment names the configured device, upper-cased; a selection must start at element 0; the type is not echoed back in `getAddressString` (`profinet.tag`, LTag:36-37; `profinet.notes`). | Same grammar for the legacy model, with typed errors for a selection not starting at 0. | planned (M5) | `planned:profinet_address_parses_device_dot_tag_with_array_and_type`, `planned:profinet_address_rejects_an_array_selection_not_starting_at_zero` |
| PN-NG-ADDR | `{slot}.{subSlot}.(INPUT or OUTPUT)[.{index}][array]:TYPE`, numeric, for example `1.1.INPUT.0:BOOL`; the index defaults when omitted (`profinet_ng.tag`, NGTag:36-37; `profinet_ng.notes`). | Same grammar for the next-generation model; the default index is recorded when ported (UNVERIFIED). | planned (M5) | `planned:profinet_ng_address_parses_slot_subslot_direction_and_index`, `planned:profinet_ng_address_defaults_the_index_when_omitted` |

## Limits
| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-L-LIMIT-CONNECT-WAIT | The connect waits up to 60 s for all devices to reach ApplicationReady (LConn:139-190). | See PN-L-CONNECT. | planned (M5) | `planned:profinet_connect_times_out_after_sixty_seconds_without_application_ready` |
| PN-NG-LIMIT-SINGLE-DEVICE | One device per connection (`profinet_ng.scope`). | One device per connection under the next-generation model. | planned (M5) | `planned:profinet_ng_connection_serves_exactly_one_device` |

## Data types
The address suffix is any PlcValueType (`profinet.tag`, `profinet_ng.tag`). The manifest
cites no per-type table; the type set follows the GSDML data items (`profinet_ng.browse`).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-TYPE-GSDML | Data items come from the GSDML per slot and subslot and map onto PlcValueType (NGConn:460-515). The mapping table was not inventoried. | The M5 slice inventories the GSDML type mapping and replaces this row with one row per type. | planned (M5) | `planned:profinet_gsdml_data_item_types_map_to_value_types` |
| PN-TYPE-BOOL | Example `1.1.INPUT.0:BOOL` (profinet_ng.notes). | BOOL addressed by index. | planned (M5) | `planned:profinet_ng_bool_input_decodes_from_the_cyclic_frame` |

## Protocol-specific sections

### Cyclic data and device handshake
| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-CYCLIC-DISPATCH | Parsed cyclic RT frames reach registered consumers; CYCLIC, CHANGE_OF_STATE and EVENT are handled in `ProfinetDevice` dispatch (LDev:411-462). Handshake detail (lines 190-500, 640-900) was read for structure only (UNCERTAIN). | Cyclic RT frame parser and dispatch, differential-tested against captured frames. | planned (M5) | `planned:profinet_cyclic_rt_frame_parses_io_data_and_iops`, `planned:profinet_dispatch_handles_cyclic_change_of_state_and_event` |
| PN-DCE-RPC | DCE/RPC Connect, WriteParameters (parameter write), ParameterEnd and ApplicationReady over UDP (`profinet.notes`). | Codec for the four exchanges. | planned (M5) | `planned:profinet_dce_rpc_connect_request_matches_the_captured_bytes` |
| PN-LLDP | LLDP device discovery per configured device (`profinet.notes`). | Part of PN-DISCOVER. | planned (M5) | `planned:profinet_lldp_frame_parses_chassis_and_port_ids` |
| PN-NG-REAL-IDENTIFICATION | The slot and module layout is learned by reading RealIdentificationData over PN-CM (NGReadme:108-120). | Read and parse RealIdentificationData. | planned (M5) | `planned:profinet_ng_real_identification_data_yields_the_slot_layout` |
| PN-NG-GSD-MATCH | GSD files are searched by vendor-id and device-id from the DCP identify response (`profinet_ng.operations.browse`, NGReadme:21-43). | GSDML loader and matcher. | planned (M5) | `planned:profinet_ng_gsd_is_matched_by_vendor_and_device_id` |

## Upstream quirks
| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-QUIRK-TWO-MODULES | Two modules share the protocol code `profinet`. | See PN-ALTERNATIVES. | planned (M5) | `planned:profinet_driver_registers_a_single_protocol_code` |
| PN-QUIRK-L-NO-CONSUMER | Legacy connection lacks `onRegisterConsumer` (see PN-L-SUBSCRIBE-CONSUMER). | Not reproduced. | deferred (upstream gap; not reproduced) | none |
| PN-QUIRK-NG-INCOMPLETE-FUTURE | The next-generation subscribe future is never completed. | Not reproduced. | deferred (upstream placeholder) | none |
| PN-QUIRK-L-TYPE-NOT-ECHOED | The type is not echoed by `getAddressString` (LTag:36-37). | The Stonecrab address displays its type. | planned (M5) | `planned:profinet_address_display_round_trips_with_the_type` |
| PN-QUIRK-L-BROWSE-SAME-LIST | Legacy browse returns the same list for every query (LConn:245-289). | Ported as is; queries are not interpreted. | planned (M5) | `planned:profinet_browse_ignores_the_query_and_returns_the_full_list` |

## Divergences from upstream
No deliberate divergence is decided yet.

| Row id | Upstream behavior | Stonecrab behavior and rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-DIVERGE-CONSUMER-PATH | The legacy API offers no connection-level consumer registration. | The Stonecrab subscription handle yields events directly (candidate, PN-L-SUBSCRIBE). | planned (M5) | `planned:profinet_subscription_handle_yields_cyclic_events_without_a_separate_consumer_step` |

## Backlog from other languages
No `[[backlog]]` row, and no `go_*`, `c_*`, `csharp_*` or `python_*` inventory entry, names
PROFINET at the pinned commit. The manifest lists no non-Java PROFINET driver; the row
below keeps that absence visible in the release matrix.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PN-BACKLOG-NONE | No non-Java PROFINET implementation is inventoried (manifest `[[backlog]]` and `inventory.go_*`, `c_*`, `csharp_*`, `python_*` have no PROFINET row). | None. | deferred (nothing inventoried) | none |

## UNVERIFIED
- `onRegisterConsumer` gap of the legacy connection was derived from the `ConnectionBase`
  defaults and the hook list; the path used by `ProfinetSubscriptionHandle.register` was
  not exercised (`profinet.notes` UNCERTAIN).
- `ProfinetDevice` handshake details (lines 190-500, 640-900) were read for structure
  only.
- Which parts of the `profinet-ng` connect path (ARP, DCP unicast, RealIdentificationData
  read) are implemented and not only described in the README was not verified line by line
  (`ProfinetConnection` lines 146-350 were skimmed).
- The cycle-time constant in the legacy description (31.23 us) against the standard 31.25
  us was not checked against the specification.
- The default index of an `profinet-ng` address when omitted, the GSDML type mapping and
  the `devices` option syntax were not inventoried.
- No test device model or firmware is named; no row is `hardware-verified`.
