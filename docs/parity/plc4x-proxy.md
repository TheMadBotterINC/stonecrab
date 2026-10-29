# PLC4X proxy parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State at S43: nothing is implemented. The PLC4X proxy driver (protocol code `plc4x`) is the
client side of a relay: it authenticates to a PLC4X proxy server, asks it to open a remote
connection string, then forwards reads and writes. It is planned for milestone M5
(`docs/POST_ALPHA_PLAN.md`, section 4: "PLC4X proxy ... TLS by default, mandatory user and
password, read and write only. Subscribe is commented out in the spec"). The manifest
entries are `inventory.plc4x` (Java) and `inventory.c_plc4x` (a C skeleton that is not
usable); there is no Go, C# or Python proxy driver. The server side, the PLC4X Server in
`apache/plc4x-extras`, is a tool outside the pinned baseline (`inventory.tool_plc4x_server`,
located and not read) and belongs to the tools list of decision E6, not to this record.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one test id prefixed `planned:`; none exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now (an upstream placeholder,
  a missing prerequisite, or a tool outside the baseline). Test ids may be `none`.
- `implemented`, `automated-verified`, `hardware-verified`: defined in
  `docs/parity/modbus.md`; no proxy row uses them yet. The proxy has no device, so no row
  can become `hardware-verified`.

Row ids use the prefix `PX-`. Ids are never renumbered.

## File key
Citations use the manifest key, then `path:lines` against the pinned commit. `J` abbreviates
`plc4j/drivers/plc4x/src/main/java/org/apache/plc4x/java/plc4x` and `C` abbreviates
`plc4c/drivers/plc4x`.

| Key | Path |
| --- | --- |
| Drv | `J/Plc4xDriver.java` (manifest `plc4x.driver`) |
| Cfg | `J/config/Plc4xConfiguration.java` (`plc4x.config`) |
| Conn | `J/Plc4xConnection.java` (`plc4x.connection`, `connection_rw`) |
| Codec | `J/Plc4xMessageCodec.java` (`plc4x.codec`) |
| TagH | `J/tag/Plc4XTagHandler.java` (`plc4x.taghandler`) |
| Mspec | `protocols/plc4x/src/main/resources/protocols/plc4x/v0/plc4x.mspec` (`plc4x.mspec`, `mspec_sub`, `mspec_enum`) |
| CDrv | `C/src/driver_plc4x.c` (`c_plc4x.driver_create`) |
| CConn | `C/src/driver_plc4x_sm_connect.c` (`c_plc4x.connect_sm`, `connect_response_dropped`) |
| CPk | `C/src/driver_plc4x_packets.c` (`c_plc4x.frame_select`, `connect_request_build`, `read_request_build`, `write_request_build`, `destroy_packet`) |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-OP-CONNECT | Authentication first (mandatory username and password; missing credentials throw `PlcConnectionException`), then `Plc4xConnectRequest` with `remote-connection-string` to obtain a `connectionId`; a failed connect cleans up the receive loop (Conn:83-216; plc4x.notes). | Connect opens the transport, sends AUTH, then CONNECT with the remote string, and keeps the returned id for later requests. A failure at either step closes the transport and returns a typed error. | planned (M5) | `planned:px_connect_sends_auth_then_connect`, `planned:px_connect_without_credentials_is_invalid_config`, `planned:px_connect_failure_closes_the_transport`, `planned:px_connection_id_from_the_response_is_used_in_requests` |
| PX-OP-CLOSE | The mspec has DISCONNECT_REQUEST and DISCONNECT_RESPONSE, but the connection class does not send them: `close()` only stops the transport (plc4x.notes; read from the class, not a documented guarantee). | Close sends DISCONNECT when the session is up and does not wait for the reply beyond a short bound; then closes the transport. This is a deliberate difference from Java. | planned (M5) | `planned:px_close_sends_disconnect_before_closing_the_transport`, `planned:px_close_does_not_hang_on_a_silent_proxy` |
| PX-OP-READ | `Plc4xReadRequest` carries the `connectionId` and tags as `<address>:<TYPE>`; responses are correlated by `requestId`; bounded by `request-timeout-ms` (Conn:264-350; plc4x.operations.read). | One READ_REQUEST per read call carrying all tags (the mspec has a tag count of one byte), correlated by request id. | planned (M5) | `planned:px_read_sends_the_tags_in_one_request`, `planned:px_read_correlates_by_request_id`, `planned:px_read_timeout_is_a_request_timeout` |
| PX-OP-WRITE | `Plc4xWriteRequest` with a `Plc4xValueType` and the `PlcValue`; response codes are mapped by name (Conn:264-350; plc4x.operations.write). | WRITE_REQUEST with the value type and value per tag; response codes map to item errors (PX-WIRE-RESPONSE-CODE). | planned (M5) | `planned:px_write_sends_value_type_and_value`, `planned:px_write_maps_response_codes_by_name` |
| PX-OP-SUBSCRIBE | Not supported: no `onSubscribe` override and no polling base class. The mspec's subscription messages are commented out ("TODO: Implement this later on"), though the request-type enum lists them (plc4x.operations.subscribe; Mspec:69-93). | `Capabilities` reports subscribe false. | deferred (upstream placeholder: commented-out spec) | none |
| PX-OP-UNSUBSCRIBE | UNSUBSCRIPTION request types 0x0B and 0x0C exist in the enum only (Mspec:228-256). | Not implemented. | deferred (upstream placeholder: enum value only) | none |
| PX-OP-BROWSE | `Plc4XTagHandler.parseQuery` throws `UnsupportedOperationException("This driver doesn't support browsing")`; no `onBrowse` (TagH:27-43). | `Capabilities.browse = false`. | deferred (no upstream operation) | none |
| PX-OP-DISCOVER | "Not supported." | `Capabilities.discover = false`. | deferred (no upstream operation) | none |
| PX-OP-PING | No `onPing` override; the base returns a failed "Ping not supported" (plc4x.operations.ping). | `Capabilities.ping = false`. | deferred (no upstream operation) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-CAP-FLAGS | `canRead` and `canWrite` true; nothing else overridden (Drv:76-84). | `Capabilities` reports read and write true; ping, browse, discover and subscribe false. | planned (M5) | `planned:px_capabilities_report_read_and_write_only` |

## Configuration
Four options (Cfg:27-45).

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-CFG-REMOTE | `remote-connection-string`, String, no default; "URL-Encoded connection string to use on the proxy side to reach the given PLC" (Cfg:27-45). | Required; percent-decoded before it goes on the wire; a missing value is `InvalidConfig`. | planned (M5) | `planned:px_remote_connection_string_is_percent_decoded`, `planned:px_missing_remote_connection_string_is_invalid_config` |
| PX-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 5000 (Cfg:27-45). | Positive integer, default 5000, mapped onto `Timeouts.request`. | planned (M5) | `planned:px_request_timeout_default_is_5000` |
| PX-CFG-USERNAME | `username`, String; authentication is mandatory (Cfg:27-45). | Required. | planned (M5) | `planned:px_missing_username_is_invalid_config` |
| PX-CFG-PASSWORD | `password`, String, annotated `@Secret`; mandatory (Cfg:27-45). | Required and held in a redacting wrapper: it never appears in a `Debug` print, an error message or a log line. | planned (M5) | `planned:px_password_is_redacted_in_debug_output`, `planned:px_missing_password_is_invalid_config` |
| PX-CFG-UNKNOWN | Unknown parameters are reported, not rejected (shared driver base; MB-CFG-UNKNOWN). | Same as MB-CFG-UNKNOWN. | planned (M5) | `planned:px_unknown_option_is_reported_not_rejected` |
| PX-CFG-C-DEFAULTS | The C driver has the remote string, a timeout that is never used, a connection id and a request id; no username or password (CDrv:25-62; `c_plc4x.config_struct`). | No obligation. | deferred (C skeleton, not a usable driver) | none |

## Connection string and transports
Default transport `tls`; supported `tls`, `tcp`, `test` (Drv:63-74). The driver has no
`defaultPorts` override.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-TRANS-TLS | `tls` is the default because the proxy carries credentials (plc4x.notes; Drv:63-69). | `plc4x://host:port` uses TLS. The row depends on the Stonecrab TLS transport, which does not exist yet (`docs/parity/transport.md`). | planned (M5) | `planned:px_default_transport_is_tls`, `planned:px_tls_failure_does_not_fall_back_to_tcp` |
| PX-TRANS-TCP | Plaintext `tcp` is an explicit opt-in (plc4x.notes). | `plc4x:tcp://host:port` is accepted only when written out; a log line at warning level names the exposure of the credentials. | planned (M5) | `planned:px_plaintext_tcp_is_accepted_only_when_explicit`, `planned:px_plaintext_tcp_logs_a_credential_warning` |
| PX-TRANS-PORT | The mspec constant `plc4xTcpDefaultPort` is 59837 (0xCAFE) but the Java code does not use it (plc4x.notes; Mspec:20-30). Whether the proxy server listens on it is UNCERTAIN. | A connection string without a port is `InvalidConfig` rather than a guess; a port is always required. | planned (M5) | `planned:px_connection_string_without_a_port_is_invalid_config` |
| PX-TRANS-TEST | `test` transport declared (Drv:71-74). | Rejected with `InvalidConfig`; Stonecrab uses its own fake transport. | deferred (follows MB-TCP-TRANSPORTS-TEST) | none |
| PX-TRANS-C-TCP | C registers `tcp` as the default and only transport, where Java defaults to `tls` because credentials travel on the wire (`c_plc4x` LANG GAP; CDrv:25-62). | Not followed: the Java default stands. | deferred (C skeleton; see PX-TRANS-TLS) | none |
| PX-TRANS-C-CODE | The C driver registers the code `plc4c`, not `plc4x` (`c_plc4x.protocol_code`; CDrv:25-62). | Only `plc4x` is registered. | deferred (C skeleton) | none |

## Address grammar
`address:TYPE`, split at the last colon; `TYPE` must be a `PlcValueType` name, else
`PlcValueType.valueOf` throws (TagH:27-43; plc4x.notes). The address is opaque to the proxy
client: the proxy server parses it for the remote driver.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-ADDR-SPLIT | The tag is split at the last colon into an address string and a type name (TagH:27-43). | The same split. An address without a colon, or with an empty side, is `InvalidAddress`. | planned (M5) | `planned:px_tag_splits_at_the_last_colon`, `planned:px_tag_without_a_colon_is_invalid_address` |
| PX-ADDR-TYPE | An unknown type name makes `valueOf` throw an unchecked exception (TagH:27-43). | An unknown type name is a typed `InvalidAddress` that lists the accepted names. This replaces an exception with an error value. | planned (M5) | `planned:px_unknown_type_name_is_invalid_address` |
| PX-ADDR-OPAQUE | The client does not check the address; the remote driver does. | The client forwards the address string unchanged, including colons inside it, and reports the remote's refusal as the item error. | planned (M5) | `planned:px_address_is_forwarded_unchanged`, `planned:px_remote_invalid_address_maps_to_an_item_error` |

## Data types
`Plc4xValueType` (Mspec:228-292; the manifest read lines 260-292 only partially, UNVERIFIED
beyond the list quoted here): NULL 0x00; bit strings BOOL, BYTE, WORD, DWORD, LWORD; unsigned
USINT, UINT, UDINT, ULINT; signed SINT, INT, DINT, LINT; REAL, LREAL; CHAR, WCHAR, STRING,
WSTRING; TIME, LTIME, DATE, LDATE, TIME_OF_DAY, LTIME_OF_DAY, DATE_AND_TIME,
LDATE_AND_TIME.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-TYPE-BITSTRING | BOOL 0x01, BYTE 0x02, WORD 0x03, DWORD 0x04, LWORD 0x05. | Encode and decode as fixed-width values of the PLC4X value model. | planned (M5) | `planned:px_bool_round_trips_through_the_wire_value`, `planned:px_lword_round_trips_through_the_wire_value` |
| PX-TYPE-INTEGERS | USINT to ULINT 0x11 to 0x14; SINT to LINT 0x21 to 0x24. | Same codes, big-endian. | planned (M5) | `planned:px_unsigned_integers_round_trip`, `planned:px_signed_integers_round_trip` |
| PX-TYPE-FLOATS | REAL 0x31, LREAL 0x32. | IEEE 754, big-endian. | planned (M5) | `planned:px_real_and_lreal_round_trip` |
| PX-TYPE-CHARS | CHAR 0x41, WCHAR 0x42, STRING 0x43, WSTRING 0x44. | Same codes; the string wire layout is read from the mspec when the codec is written. | planned (M5) | `planned:px_string_and_wstring_round_trip` |
| PX-TYPE-TEMPORAL | TIME 0x51, LTIME 0x52, DATE 0x53, LDATE 0x54, TIME_OF_DAY 0x55, LTIME_OF_DAY 0x56, DATE_AND_TIME 0x57, LDATE_AND_TIME 0x58. | Same codes, mapped to the temporal values of the Stonecrab value model. | planned (M5) | `planned:px_temporal_values_round_trip` |
| PX-TYPE-NULL | NULL 0x00. | A NULL value in a response is an item error unless the request asked for it; a NULL write is refused. | planned (M5) | `planned:px_null_value_in_a_read_response_is_an_item_error` |
| PX-TYPE-UNLISTED | Value types that the mspec does not list (the list ends at LDATE_AND_TIME in the part read). | Refused as `UNSUPPORTED` without sending. | planned (M5) | `planned:px_value_type_not_on_the_wire_list_is_refused_before_sending` |

## Wire format
Big-endian. Frame: version byte `0x01`, 2-byte total length, 2-byte request id, 1-byte
request type, then the body (Codec:33-66; Mspec:20-30).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-WIRE-FRAME | Version byte 0x01; an unknown version raises `MessageCodecException` (Codec:33-66). The C `frame_select` reads the length from bytes 1 and 2 with a minimum size of 3 (CPk:61-70), which matches. | The framer reads the 3-byte prefix, checks the version, and takes the length; an unknown version is a framing error that closes the connection. The length is bounded before any allocation. | planned (M5) | `planned:px_framer_rejects_an_unknown_version`, `planned:px_framer_rejects_a_length_below_the_header`, `planned:px_framer_waits_for_the_full_frame`, `planned:px_framer_never_panics_on_random_bytes` |
| PX-WIRE-REQUEST-TYPES | CONNECT 01/02, DISCONNECT 03/04, READ 05/06, WRITE 07/08, SUBSCRIPTION 09/0A (unimplemented), UNSUBSCRIPTION 0B/0C (unimplemented), AUTH 0D/0E (Mspec:228-256). | CONNECT, DISCONNECT, READ, WRITE and AUTH are encoded and decoded; the other four request types are decoded as unexpected and fail the matching request. | planned (M5) | `planned:px_request_type_codes_match_the_mspec`, `planned:px_subscription_response_is_an_unexpected_reply` |
| PX-WIRE-AUTH | AUTH_REQUEST: length-prefixed username and password; AUTH_RESPONSE: a response code (Mspec:20-93). | Encoded as such; each string is limited to 255 bytes (one length byte), longer is `InvalidConfig`. | planned (M5) | `planned:px_auth_request_bytes_match_the_vector`, `planned:px_username_over_255_bytes_is_invalid_config` |
| PX-WIRE-CONNECT | CONNECT_REQUEST: length-prefixed connection string; CONNECT_RESPONSE: connection id and response code (Mspec:20-93). | Same. | planned (M5) | `planned:px_connect_request_bytes_match_the_vector`, `planned:px_connect_response_with_a_refusal_is_an_error` |
| PX-WIRE-READ-WRITE | READ_REQUEST: connection id, tag count (one byte), tags; READ_RESPONSE adds a response code and per-tag values; WRITE_REQUEST/RESPONSE likewise (Mspec:20-93). | Same; more than 255 tags in one request is split by the caller or refused before sending. | planned (M5) | `planned:px_read_request_bytes_match_the_vector`, `planned:px_request_with_over_255_tags_is_refused` |
| PX-WIRE-RESPONSE-CODE | `Plc4xResponseCode`: OK 01, NOT_FOUND 02, ACCESS_DENIED 03, INVALID_ADDRESS 04, INVALID_DATATYPE 06, INVALID_DATA 07, INTERNAL_ERROR 08, REMOTE_BUSY 09, REMOTE_ERROR 0A, UNSUPPORTED 0B, RESPONSE_PENDING 0C (Mspec:228-256). Code 05 is unassigned. | Each code maps to the Stonecrab item-error class of the same name; 05 and anything unknown map to an internal error that carries the raw code. RESPONSE_PENDING keeps the request open until the timeout. | planned (M5) | `planned:px_response_codes_map_by_name`, `planned:px_unassigned_response_code_keeps_the_raw_value`, `planned:px_response_pending_waits_for_the_next_reply` |
| PX-WIRE-CORRELATION | Replies are matched by `requestId` (plc4x.operations.read). | Request ids come from one per-connection counter; a reply with no pending id is dropped and counted. The C driver's 8-bit counter and uncompared id are not copied. | planned (M5) | `planned:px_reply_with_an_unknown_request_id_is_dropped`, `planned:px_request_ids_wrap_without_collision_while_pending` |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| PX-QUIRK-NO-DISCONNECT | The Java connection never sends DISCONNECT. | plc4x.notes, Conn | deferred (not reproduced; see PX-OP-CLOSE) | none |
| PX-QUIRK-PORT-UNUSED | The mspec port constant 59837 is not used by the Java driver. | plc4x.notes, Mspec:20-30 | deferred (not reproduced; see PX-TRANS-PORT) | none |
| PX-QUIRK-VALUEOF | An unknown type name throws an unchecked exception from `valueOf`. | TagH:27-43 | deferred (not reproduced; see PX-ADDR-TYPE) | none |
| PX-QUIRK-ENUM-PLACEHOLDERS | The request-type enum lists subscription types whose messages are commented out. | Mspec:69-93, 228-256 | deferred (see PX-OP-SUBSCRIBE) | none |
| PX-QUIRK-C-MEMORY | The C driver's connect builder calls `strlen` on a NULL pointer in the default configuration, and its destroy path frees through the wrong union member (`c_plc4x` notes, CPk:138-196). | `c_plc4x.notes` | deferred (C skeleton; Rust has no equivalent) | none |

## Divergences from upstream

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-DIV-DISCONNECT | `close()` sends nothing. | Close sends DISCONNECT (PX-OP-CLOSE); the proxy can release the remote connection at once rather than at its idle timeout. | planned (M5) | `planned:px_close_sends_disconnect_before_closing_the_transport` |
| PX-DIV-PORT | No default port in the Java driver. | No default port either, but a typed error instead of an unspecified failure (PX-TRANS-PORT). | planned (M5) | `planned:px_connection_string_without_a_port_is_invalid_config` |
| PX-DIV-TYPE-ERROR | An unknown type name throws. | A typed address error (PX-ADDR-TYPE). | planned (M5) | `planned:px_unknown_type_name_is_invalid_address` |

## Backlog from other languages
No `[[backlog]]` row names the proxy driver. The C driver is a skeleton whose read and
write are not usable (`c_plc4x.operations`), so its `LANG GAP` notes are recorded as
deferred rows above and the Java behavior is the only baseline. They are listed here so the
omission is visible.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| PX-BACKLOG-C-AUTH | C has no authentication step or fields (`c_plc4x.connect_sm`, CConn:42-129). | Java's mandatory authentication stands (PX-OP-CONNECT). | deferred (C gap; Java is the baseline) | none |
| PX-BACKLOG-C-READ-WRITE | C read and write are not functional: the request is typed as a connect request, address parsing is a no-op and the connection id is dropped (`c_plc4x.operations`, `address_parse_noop`, `connect_response_dropped`). | No obligation. | deferred (upstream placeholder in C) | none |
| PX-BACKLOG-C-OTHER | C has no subscribe, browse, discover or ping. | Same as Java (PX-OP-SUBSCRIBE, PX-OP-BROWSE, PX-OP-DISCOVER, PX-OP-PING). | deferred (no upstream operation) | none |
| PX-BACKLOG-SERVER | The proxy server relay in `apache/plc4x-extras` (`plc4j/tools/plc4x-server`), outside the pinned baseline, located and not read (`inventory.tool_plc4x_server`). A Stonecrab client test needs a peer. | A scripted peer in the test kit serves the vectors; a Stonecrab proxy server is a tool under decision E6 and has its own record. | deferred (tool outside the baseline; decision E6) | none |

## UNVERIFIED
- Whether the proxy server listens on 59837 (0xCAFE); the mspec constant is unused by the
  client.
- The `Plc4xValueType` list beyond LDATE_AND_TIME and the wire layout of strings and
  temporal values: the manifest read the list only partially (lines 260-292).
- Whether the Java connection sends a disconnect anywhere other than `close()`; the manifest
  states the finding from reading the class.
- The proxy server's behavior and its wire handling of RESPONSE_PENDING; the server source
  was located by listing and not read.
- The exact line ranges of the cited Java methods inside `Plc4xConnection.java`: the
  manifest gives 83-216 and 264-350 only.
