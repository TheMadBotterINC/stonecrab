# SLMP parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State at S43: nothing is implemented. SLMP (Mitsubishi MELSEC, 3E binary frame over TCP) is
planned for milestone M5, in the group "ADS/AMS, SLMP and UMAS first"
(`docs/POST_ALPHA_PLAN.md`, section 4). The manifest entries are `inventory.slmp` (Java,
protocol code `slmp`) and `inventory.go_slmp`; there is no C, C# or Python SLMP driver in
the manifest. Every row is `planned (M5)` or `deferred` with its reason. A device the tag
parser refuses upstream, or a command the mspec models and the driver never sends, is not
an obligation.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one test id prefixed `planned:`; none exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now. Test ids may be `none`.
- `implemented`, `automated-verified`, `hardware-verified`: defined in
  `docs/parity/modbus.md`; no SLMP row uses them yet. A `hardware-verified` row will name
  the PLC model and firmware.

Row ids use the prefix `SLMP-`. Ids are never renumbered.

## File key
Citations use the manifest key, then `path:lines` against the pinned commit. `J` abbreviates
`plc4j/drivers/slmp/src/main/java/org/apache/plc4x/java/slmp` and `GO` abbreviates
`plc4go/internal/slmp`.

| Key | Path |
| --- | --- |
| Drv | `J/SlmpDriver.java` (manifest `slmp.driver`) |
| Conn | `J/SlmpConnection.java` (`slmp.connection_class`, `read`, `write_cmd`) |
| Tag | `J/tag/SlmpTag.java` (`slmp.tag`, `tag_devices`) |
| DT | `J/SlmpDataType.java` (`slmp.datatype`) |
| Cfg | `J/config/SlmpConfiguration.java` (`slmp.config`) |
| Mspec | `protocols/slmp/src/main/resources/protocols/slmp/slmp.mspec` (`slmp.mspec_devices`, `mspec_cmds`) |
| Poll | `plc4j/utils/subscription-emulation/.../PollingSubscriptionConnectionBase.java` |
| Go | `GO` package (`go_slmp`) |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-OP-CONNECT | Protocol code `slmp`, default transport `tcp`, supported `tcp` and `test` (Drv:41-44, 63-71). `onConnect` validates the configuration then opens the link (Conn:74-85); there is no handshake frame. Transport loss fails pending requests (Conn:124-125). | `slmp://host[:port]` opens a TCP link, validates the options first and sends nothing on connect. | planned (M5) | `planned:slmp_connect_sends_no_frame`, `planned:slmp_connect_rejects_an_invalid_monitoring_timer`, `planned:slmp_transport_loss_fails_pending_requests` |
| SLMP-OP-READ | Batch Read, command 0x0401 subcommand 0x0000, word units; one request per tag, built by `readSingleTag` (Conn:250-259; slmp.operations.read). | One Batch Read per tag, sent in sequence, each bounded by `request-timeout-ms`; the response words are decoded little-endian through the tag's data type. | planned (M5) | `planned:slmp_read_of_a_d_word_decodes_little_endian`, `planned:slmp_read_of_a_range_returns_a_list`, `planned:slmp_read_sends_one_batch_read_per_tag` |
| SLMP-OP-WRITE | Batch Write, command 0x1401 subcommand 0x0000, one request per tag (Conn:263, 316; slmp.operations.write). | One Batch Write per tag; the value is encoded before any byte is sent, so an encode failure writes nothing (the Go behavior, which matches the intent). | planned (M5) | `planned:slmp_write_of_a_d_word_encodes_little_endian`, `planned:slmp_write_encode_failure_sends_nothing` |
| SLMP-OP-SUBSCRIBE-POLL | Polling emulation only: `SlmpConnection` extends `PollingSubscriptionConnectionBase`; the driver does not override `canSubscribe` (Conn:62; Drv:93-101). | Follows the Modbus decision for polled subscribe (MB-TCP-SUBSCRIBE). | deferred (follows MB-TCP-SUBSCRIBE) | none |
| SLMP-OP-SUBSCRIBE-EVENT | EVENT is refused by the polling base (Poll:336-343). | Not offered. | deferred (upstream refuses the kind) | none |
| SLMP-OP-BROWSE | "Not implemented." | `Capabilities.browse = false`. | deferred (no upstream operation) | none |
| SLMP-OP-DISCOVER | "Not implemented." | `Capabilities.discover = false`. | deferred (no upstream operation) | none |
| SLMP-OP-PING | "Not implemented." The driver sets only `canRead` and `canWrite` (Drv:93-101). Go inherits a local flag check (go_slmp.operations.ping). | `Capabilities.ping = false`. A caller that needs liveness reads a device. | deferred (no upstream operation) | none |
| SLMP-OP-RANDOM-READ | The mspec models Random Read 0x0403 and Batch Read Multiple Blocks 0x0406; the driver uses only 0x0401 and 0x1401 (Mspec:112-117). | Not implemented. | deferred (upstream placeholder: modelled, never sent) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-CAP-FLAGS | `canRead` and `canWrite` true; nothing else overridden (Drv:93-101). Connection metadata derives subscribe from the polling base (INFERENCE, as MB-TCP-CAPABILITIES). Go: `ProvidesSubscribing` true (go_slmp.operations.subscribe). | `Capabilities` reports read and write true; ping, browse, discover and subscribe false while SLMP-OP-SUBSCRIBE-POLL is deferred. | planned (M5) | `planned:slmp_capabilities_report_read_and_write_only` |

## Configuration
Two options (Cfg:31-39).

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-CFG-MONITORING-TIMER | `monitoring-timer`, int, 0 (0 means wait infinitely), written into every 3E request frame; the unit is deliberately not in the name (Cfg:31-39). `validateConfiguration` rejects a value outside [0, 65535] at connect (Conn:93-97). | `monitoring-timer`, default 0, parsed as `u16` when the configuration is read, so a bad value fails before connect (the Go behavior, backlog `go_slmp` "Early monitoring-timer validation"). | planned (M5) | `planned:slmp_monitoring_timer_default_is_0`, `planned:slmp_monitoring_timer_above_65535_is_invalid_config`, `planned:slmp_monitoring_timer_is_written_into_the_frame` |
| SLMP-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 5000: client-side wait for a response (Cfg:31-39). | `request-timeout-ms`, positive integer, default 5000, mapped onto `Timeouts.request`. | planned (M5) | `planned:slmp_request_timeout_default_is_5000`, `planned:slmp_request_timeout_zero_is_invalid_config` |
| SLMP-CFG-UNKNOWN | Unknown parameters are reported, not rejected (shared driver base; see MB-CFG-UNKNOWN). | Same as MB-CFG-UNKNOWN. | planned (M5) | `planned:slmp_unknown_option_is_reported_not_rejected` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-TRANS-TCP | Default transport `tcp`; default port 5007, the mspec constant `slmpDefaultPort` (Drv:63-66, 73-79; Mspec). | `slmp://host` uses port 5007; an explicit port overrides it. | planned (M5) | `planned:slmp_default_port_is_5007`, `planned:slmp_explicit_port_overrides_the_default` |
| SLMP-TRANS-TEST | `test` transport declared (Drv:68-71). | Rejected with `InvalidConfig`; Stonecrab uses its own fake transport. | deferred (follows MB-TCP-TRANSPORTS-TEST) | none |
| SLMP-TRANS-UDP | None in the manifest: only `tcp` and `test` are supported. | Not offered. | deferred (no upstream transport) | none |

## Address grammar
`{device}{address}[selection]:{TYPE}`, for example `D100[0..3]:INT` (Tag:37-48; the pattern at Tag:45). Word devices D (decimal), W (hex) and R (decimal) only.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-ADDR-DEVICE-D | `D` is the data register, code 0xA8, decimal (Tag:87-104; Mspec:62-71). | `D<n>` parses as decimal. | planned (M5) | `planned:slmp_address_d100_parses_as_decimal`, `planned:slmp_address_d_with_hex_digits_is_invalid_address` |
| SLMP-ADDR-DEVICE-W | `W` is the link register, code 0xB4, hex. A `0x` prefix is accepted only on hex devices (Tag:87-104). | `W<hex>` and `W0x<hex>` parse as hex. | planned (M5) | `planned:slmp_address_w1f_parses_as_hex`, `planned:slmp_address_w_with_0x_prefix_parses_as_hex` |
| SLMP-ADDR-DEVICE-R | `R` is the file register, code 0xAF, decimal (Tag:87-104). | `R<n>` parses as decimal. | planned (M5) | `planned:slmp_address_r10_parses_as_decimal` |
| SLMP-ADDR-DEVICE-REFUSED | The mspec also defines TN, M, X, Y and B but the parser throws "word devices D/W/R only" for them (Tag:87-104; Mspec:62-71). Bit devices are rejected in Java and in Go (go_slmp.notes). | Refused with a typed error naming the device. | planned (M5) | `planned:slmp_address_of_a_bit_device_is_invalid_address`, `planned:slmp_address_of_an_unknown_device_is_invalid_address` |
| SLMP-ADDR-BIT-DEVICES | Bit devices M, X, Y, B and the timer current value TN have device codes in the mspec only. | Not implemented; a bit-device port is a new feature, not a parity obligation. | deferred (upstream placeholder: enum value, parser refuses) | none |
| SLMP-ADDR-HEX-TOKEN | Java's device token is a greedy `[A-Za-z]+`, so a W address whose first digits are letters (for example `WAB`) is split wrongly and rejected (backlog `go_slmp` "Hex-address parsing of W devices"; evidence `java_tag_pattern`). Go uses a non-greedy token. | The device token is one of `D`, `W`, `R` followed by the number, so `WAB` parses as device W, number 0xAB. | planned (M5) | `planned:slmp_address_wab_parses_as_device_w_number_0xab`, `planned:slmp_address_wff_with_type_parses` |
| SLMP-ADDR-RANGE | Selection `[a..b]` moves the device number and gives the count; `D100[4]` is a scalar and `D100[4..4]` a list of one (Tag:66, 137-169, 215-227). | Same selection semantics through the shared array notation. | planned (M5) | `planned:slmp_range_of_one_element_is_a_scalar_without_dots`, `planned:slmp_range_with_dots_returns_a_list_of_one` |
| SLMP-ADDR-OFFSET-SCALING | The selection offset is scaled by words per element, the point count is checked against 960 at parse, and the device number against the 24-bit range (backlog `go_slmp` "Offset scaling and frame ceiling check"; Java also checks the scaled device number against the 24-bit range at Tag:153-155 and the 960 ceiling at Tag:167-169, with no optimizer to split). | Both checks at parse time, with the offset scaled by the type's word width. | planned (M5) | `planned:slmp_dint_selection_offset_is_scaled_by_two_words`, `planned:slmp_selection_over_960_points_is_invalid_address`, `planned:slmp_device_number_over_24_bits_is_invalid_address` |
| SLMP-ADDR-TYPE | The `:TYPE` suffix is optional and defaults to WORD; an unknown name throws with the supported list (Tag:124-133). | The same default and the same supported list. | planned (M5) | `planned:slmp_address_without_a_type_defaults_to_word`, `planned:slmp_address_with_an_unknown_type_is_invalid_address` |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-LIMIT-POINTS | `MAX_POINTS` is 960 per frame (Tag:51, 167-169; slmp.notes). | 960 points per request, checked at parse (SLMP-ADDR-OFFSET-SCALING). | planned (M5) | `planned:slmp_exactly_960_points_is_accepted` |
| SLMP-LIMIT-DEVICE-NUMBER | Device number at most 24 bits (`MAX_DEVICE_NUMBER`, Tag:54, 117-119). | `0..=0xFFFFFF`. | planned (M5) | `planned:slmp_device_number_0xffffff_is_accepted` |
| SLMP-LIMIT-4E | Only the 3E frame is modelled; the 4E frame is not handled (slmp.notes; go_slmp.notes). | Not implemented. | deferred (no upstream support) | none |

## Data types
WORD (default), INT, UINT, DINT, UDINT, REAL (DT:48-54); words are little-endian
(go_slmp.notes).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-TYPE-WORD | WORD, one 16-bit word, the default. | `u16` value as a PLC4X WORD. | planned (M5) | `planned:slmp_word_decodes_little_endian` |
| SLMP-TYPE-INT | INT, one signed word. | `i16`. | planned (M5) | `planned:slmp_int_decodes_negative_values` |
| SLMP-TYPE-UINT | UINT, one unsigned word. | `u16`. | planned (M5) | `planned:slmp_uint_decodes_0xffff` |
| SLMP-TYPE-DINT | DINT, two words. | `i32`, low word first as the manual lays out double words. | planned (M5) | `planned:slmp_dint_decodes_two_words_low_word_first` |
| SLMP-TYPE-UDINT | UDINT, two words. | `u32`. | planned (M5) | `planned:slmp_udint_decodes_two_words` |
| SLMP-TYPE-REAL | REAL, two words, IEEE 754. | `f32`. | planned (M5) | `planned:slmp_real_decodes_ieee754` |
| SLMP-TYPE-OTHER | No other types; strings, bits and 64-bit types do not appear. | Refused through SLMP-ADDR-TYPE. | planned (M5) | `planned:slmp_lreal_is_refused` |

## Wire format
All multi-byte fields are little-endian (Mspec:20-60). The Ethernet header is the
transport's.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-WIRE-REQUEST | 3E request: subheader 0x50 0x00, network 0x00, PC 0xFF, destination module I/O 0x03FF, station 0x00, length (6 plus data), monitoring timer, command, subcommand, request data. The access route is a fixed constant. | The codec writes exactly that frame with the fixed route. | planned (M5) | `planned:slmp_batch_read_request_bytes_match_the_manual_example`, `planned:slmp_batch_write_request_bytes_match_the_manual_example` |
| SLMP-WIRE-RESPONSE | 3E response: subheader 0xD0 0x00, the same route, length (2 plus data), end code (0 is normal), payload. On error the payload carries error information. | The decoder checks the route constants and the length against the buffer before slicing. | planned (M5) | `planned:slmp_response_with_a_wrong_route_is_rejected`, `planned:slmp_response_length_longer_than_the_buffer_is_rejected` |
| SLMP-WIRE-END-CODE | A non-normal end code gives REMOTE_ERROR; a decode failure gives INVALID_DATA (go_slmp.notes). | A non-zero end code maps to a typed item error carrying the code; a short or malformed payload maps to an invalid-data item error. | planned (M5) | `planned:slmp_nonzero_end_code_is_a_remote_error`, `planned:slmp_short_payload_is_invalid_data` |
| SLMP-WIRE-FRAMING | None inventoried beyond the mspec length field. | The framer reads the 9-byte header, then the length field's worth of bytes; a length over the bound is a framing error. | planned (M5) | `planned:slmp_framer_waits_for_the_full_response`, `planned:slmp_framer_rejects_an_oversized_length` |

## Timeout and error handling

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-ERR-TIMEOUT | A timeout is reported as REMOTE_ERROR (Conn:240-245, 297); the write may still have been applied by the device (Conn:179, 266). Go reports REQUEST_TIMEOUT for read and write (backlog `go_slmp` "Precise timeout code"; Writer.go 111-115). | A timeout maps to the typed timeout error, not a remote error, as in Go; the write-may-have-applied caveat is documented on the write operation. This is a deliberate difference from Java. | planned (M5) | `planned:slmp_read_timeout_is_a_request_timeout`, `planned:slmp_write_timeout_is_a_request_timeout` |
| SLMP-ERR-OTHER | Any other failure maps to INTERNAL_ERROR (Conn:245, 297). | A failure with no better class maps to an internal item error. | planned (M5) | `planned:slmp_unclassified_failure_is_an_internal_error` |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| SLMP-QUIRK-GREEDY-TOKEN | Greedy device token misparses hex W addresses that start with a letter. | Tag pattern, evidence `java_tag_pattern` | deferred (not reproduced; see SLMP-ADDR-HEX-TOKEN) | none |
| SLMP-QUIRK-TIMEOUT-CODE | A timeout is reported as REMOTE_ERROR although the device may not have answered at all. | Conn:240-245 | deferred (not reproduced; see SLMP-ERR-TIMEOUT) | none |
| SLMP-QUIRK-LATE-VALIDATION | The monitoring timer is validated at connect, not when the configuration is read. | Conn:93-97 | deferred (not reproduced; see SLMP-CFG-MONITORING-TIMER) | none |

## Divergences from upstream
Planned differences, each with its rationale:

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-DIV-TIMEOUT-CODE | REMOTE_ERROR on timeout (Conn:240-245). | The typed timeout error: a caller must be able to tell silence from a device refusal. Go makes the same choice. | planned (M5) | `planned:slmp_read_timeout_is_a_request_timeout` |
| SLMP-DIV-HEX-TOKEN | Greedy token rejects `WAB`. | Accepts it. | planned (M5) | `planned:slmp_address_wab_parses_as_device_w_number_0xab` |

## Backlog from other languages
Four `[[backlog]]` rows name `go_slmp`; each is also a row above. The Go notes say "LANG
GAP: none found: devices, commands, data types and options match Java".

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| SLMP-BACKLOG-GO-HEX | `go_slmp`, "Hex-address parsing of W devices" (evidence `go_tag_parse_doc`): non-greedy token plus an optional `0x` prefix accepted only on hex devices. | Ported through SLMP-ADDR-HEX-TOKEN and SLMP-ADDR-DEVICE-W. | planned (M5) | `planned:slmp_address_wab_parses_as_device_w_number_0xab` |
| SLMP-BACKLOG-GO-OFFSET | `go_slmp`, "Offset scaling and frame ceiling check" (evidence `go_tag_parse`). | Ported through SLMP-ADDR-OFFSET-SCALING. | planned (M5) | `planned:slmp_selection_over_960_points_is_invalid_address` |
| SLMP-BACKLOG-GO-TIMEOUT | `go_slmp`, "Precise timeout code" (evidence `go_read_decode`). | Ported through SLMP-ERR-TIMEOUT. | planned (M5) | `planned:slmp_read_timeout_is_a_request_timeout` |
| SLMP-BACKLOG-GO-TIMER | `go_slmp`, "Early monitoring-timer validation" (evidence `go_config`). | Ported through SLMP-CFG-MONITORING-TIMER. | planned (M5) | `planned:slmp_monitoring_timer_above_65535_is_invalid_config` |
| SLMP-BACKLOG-GO-PING | Go inherits a local flag check as ping; Java has none (go_slmp.operations.ping). | Covered by SLMP-OP-PING. | deferred (no upstream operation) | none |

## UNVERIFIED
- Whether the 16-bit monitoring timer and the 4E frame are handled elsewhere in Java. Only
  3E appears; Go writes the timer into every frame (go_slmp.notes).
- The word order of DINT, UDINT and REAL on the wire. The manifest says words are
  little-endian; the order of the two words inside a double word is stated from the
  Mitsubishi manual (SH(NA)-080008, section 8) and has not been checked against a retained
  copy or a vector.
- The exact end-code table: the mspec carries the code as a raw `uint16`.
- The Mitsubishi manual itself is not in the repository.
