# UMAS parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State at S43: nothing is implemented. UMAS (Schneider Modicon, tunneled in Modbus TCP
function code 0x5A) is planned for milestone M5, in the group "ADS/AMS, SLMP and UMAS
first" (`docs/POST_ALPHA_PLAN.md`, section 4). The manifest entries are `inventory.umas`
(Java, protocol code `umas`), `inventory.go_umas` and `inventory.python_umas`; there is no
C or C# UMAS driver. UMAS rides on the Modbus TCP ADU, so the MBAP framer of
`docs/parity/modbus.md` (MB-READ-FRAMING, MB-READ-TXN) is reused; the PDU is new.

The plan's lead "read and write except DATE and TIME_OF_DAY" was checked in S41 and S42
and is not borne out: the Java, Go and Python drivers read and write DATE, TOD, TIME and
DATE_AND_TIME (manifest `umas.notes`, `go_umas.notes`, `python_umas.notes`). Those types
are rows below, not exclusions.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one test id prefixed `planned:`; none exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now. Test ids may be `none`.
- `implemented`, `automated-verified`, `hardware-verified`: defined in
  `docs/parity/modbus.md`; no UMAS row uses them yet. A `hardware-verified` row will name
  the PLC model and firmware.

Row ids use the prefix `UMAS-`. Ids are never renumbered.

## File key
Citations use the manifest key, then `path:lines` against the pinned commit. `J` abbreviates
`plc4j/drivers/umas/src/main/java/org/apache/plc4x/java/umas`, `GO` abbreviates
`plc4go/internal/umas` and `PY` abbreviates `plc4py/plc4py/drivers/umas`.

| Key | Path |
| --- | --- |
| Drv | `J/UmasDriver.java` (manifest `umas.driver`) |
| Conn | `J/UmasConnection.java` (`umas.connection_class`, `ping`, `read_types`, `write_types`, `browse`) |
| Cfg | `J/configuration/UmasConfiguration.java` (`umas.config`, `go_umas.java_config`) |
| Tag | `J/tag/SymbolicUmasTag.java` (`umas.tag`) |
| TagH | `J/tag/UmasTagHandler.java` |
| Mspec | `protocols/umas/src/main/resources/protocols/umas/v1/umas.mspec` (`umas.mspec`; a v0 mspec also exists) |
| Poll | `plc4j/utils/subscription-emulation/.../PollingSubscriptionConnectionBase.java` |
| GoConn | `GO/Connection.go` (`go_umas.go_handshake`, `go_dictionary`, `go_ping`, `go_metadata`) |
| GoBrowse | `GO/Browser.go` (`go_umas.go_browse`, `go_browse_item`) |
| GoVal | `GO/Values.go` (`go_umas.go_decode`, `go_decode_tod`, `go_encode`) |
| GoQuery | `GO/Query.go` (`go_umas.go_query`) |
| GoRead | `GO/Reader.go` (`go_umas.go_read`) |
| PyDev | `PY/UmasDevice.py` (`python_umas.handshake`, `project_sync`, `read`, `write`, `system_words`, `browse`) |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-OP-CONNECT | Protocol code `umas`, default transport `tcp`, supported `tcp` and `test` (Drv:46-48, 70-76). Handshake on connect: PlcIdent, InitComms, then the data dictionary and pairing key (umas.notes). Go orders it PlcIdent, InitComms, Repeat, memory block read, project info subcode 1, second memory block, project info 0,4,1,3, then the dictionary download, whose failure only warns; the handshake timeout is 10 x `request-timeout-ms` (go_umas.notes; GoConn:224-305, 520-556). | `umas://host[:port]` connects over TCP and runs the handshake in the Go order; a failed dictionary download degrades to a warning and symbol lookups then fail with a typed error. | planned (M5) | `planned:umas_connect_runs_ident_then_init_comms`, `planned:umas_connect_survives_a_failed_dictionary_download`, `planned:umas_handshake_timeout_is_ten_request_timeouts` |
| UMAS-OP-READ | `onRead` of symbolic variables; one ReadVariable per tag; temporal types hand-parsed (STRING, TIME, DATE, TOD, DATE_AND_TIME) (Conn:597-646; umas.operations.read). Go resolves the symbol in the downloaded dictionary, sends ReadVariable per tag, returns custom types as raw bytes (go_umas.operations.read; GoRead:109-156). | Resolve the symbol in the dictionary, one ReadVariable per tag, decode by type. Timeout maps to the typed timeout error, a PLC refusal to a remote error. | planned (M5) | `planned:umas_read_of_a_real_decodes_ieee754`, `planned:umas_read_of_an_unknown_symbol_is_invalid_address`, `planned:umas_read_timeout_is_a_request_timeout`, `planned:umas_read_refusal_is_a_remote_error` |
| UMAS-OP-WRITE | `onWrite` via `serializeForType` for every `UmasDataType` case, including TIME, DATE, TOD and DATE_AND_TIME (Conn:776-820). Go encodes every primitive type including STRING, ASCII only (GoVal:297-370). | Encode by symbol type, one WriteVariable per tag; a value that does not fit the type is refused before sending. | planned (M5) | `planned:umas_write_of_an_int_encodes_little_endian`, `planned:umas_write_of_a_non_ascii_string_is_refused`, `planned:umas_write_value_out_of_range_is_refused_before_sending` |
| UMAS-OP-BROWSE | `onBrowse` and `onBrowseWithInterceptor` walk the data dictionary with `BrowseTreeBuilder` (Conn:832-840; umas.operations.browse). | Browse returns the symbol tree built from the dictionary (UMAS-BROWSE-* rows). | planned (M5) | `planned:umas_browse_lists_the_global_variables`, `planned:umas_browse_follows_struct_members` |
| UMAS-OP-PING | `onPing` sends `UmasPDUPlcStatusRequest`; failure maps to REMOTE_ERROR (Conn:489-499). Go: any answer, even an error response, counts as reachable (GoConn:646-664). | `Connection::ping` sends the PlcStatus request; any reply counts as alive, silence is a timeout. | planned (M5) | `planned:umas_ping_sends_a_plc_status_request`, `planned:umas_ping_accepts_an_error_response`, `planned:umas_ping_times_out_on_silence` |
| UMAS-OP-SUBSCRIBE-POLL | Polling emulation only: `UmasConnection` extends `PollingSubscriptionConnectionBase`; the driver does not override `canSubscribe` (Conn:141; umas.operations.subscribe). Go has `ProvidesSubscribing` true (go_umas.operations.subscribe). | Follows the Modbus decision for polled subscribe (MB-TCP-SUBSCRIBE). | deferred (follows MB-TCP-SUBSCRIBE) | none |
| UMAS-OP-SUBSCRIBE-EVENT | EVENT is refused by the polling base (Poll:336-343). | Not offered. | deferred (upstream refuses the kind) | none |
| UMAS-OP-DISCOVER | "Not implemented." | `Capabilities.discover = false`. | deferred (no upstream operation) | none |
| UMAS-OP-OTHER-COMMANDS | The mspec models reservation, keep-alive, upload and download, start and stop PLC, monitor, I/O object and Ethernet master commands (Mspec); the drivers use PlcIdent, InitComms, ReadMemoryBlock, ProjectInfo, PlcStatus, ReadVariable, WriteVariable and the dictionary reads. | Not implemented. Start and stop PLC and download are control operations with no PLC4X API operation. | deferred (upstream placeholder: modelled in the mspec, never sent) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-CAP-FLAGS | `canPing`, `canRead`, `canWrite` and `canBrowse` true (Drv:87-104). Subscribe derives from the polling base (INFERENCE, as MB-TCP-CAPABILITIES). Go metadata: read, write, subscribe and browse true (GoConn:670-680). | `Capabilities` reports read, write, browse and ping true; subscribe and discover false while UMAS-OP-SUBSCRIBE-POLL is deferred. | planned (M5) | `planned:umas_capabilities_report_read_write_browse_ping` |

## Configuration
Four options (Cfg:30-47); Go carries the first three (`go_umas.go_config_doc`, GO/Configuration.go:33-46).

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-CFG-UNIT-ID | `unit-identifier`, int, 0 ("UMAS typically uses 0"), the Modbus unit id (Cfg:29-32). Python sets 1 only when the key is absent and leaves the attribute unset when the key is supplied (python_umas.configuration; a defect). | `unit-identifier`, default 0, bounded 0..=255 (one MBAP byte). The Python defect is not ported. | planned (M5) | `planned:umas_unit_identifier_default_is_0`, `planned:umas_unit_identifier_over_255_is_invalid_config` |
| UMAS-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 4000 (Cfg:34-37). | Positive integer, default 4000, mapped onto `Timeouts.request`. | planned (M5) | `planned:umas_request_timeout_default_is_4000` |
| UMAS-CFG-MAX-FRAME | `max-frame-size`, int, 65535; "the PLC reports its actual limit during InitComms" (Cfg:39-42). Go: minimum 4 (go_umas.notes). | Default 65535, minimum 4; the value negotiated by InitComms lowers it, never raises it. | planned (M5) | `planned:umas_max_frame_size_default_is_65535`, `planned:umas_max_frame_size_below_4_is_invalid_config`, `planned:umas_init_comms_lowers_the_frame_size` |
| UMAS-CFG-ARRAY-NODES | `browser-generate-array-nodes`, boolean, true: "artificial child nodes representing individual array elements" (Cfg:44-47). Nothing else in the Java umas source reads it; Go rejects it as unknown (go_umas.notes, LANG GAP). | Accepted so a connection string from Java does not fail; it controls whether browse emits per-element nodes (UMAS-BROWSE-ARRAY). | planned (M5) | `planned:umas_browser_generate_array_nodes_default_is_true`, `planned:umas_browser_generate_array_nodes_false_omits_element_nodes` |
| UMAS-CFG-UNKNOWN | Unknown parameters are reported, not rejected (shared driver base; MB-CFG-UNKNOWN). | Same as MB-CFG-UNKNOWN. | planned (M5) | `planned:umas_unknown_option_is_reported_not_rejected` |
| UMAS-CFG-PY-TIMEOUTS | Python has fixed timeouts (TCP 10 s, handshake 60 s) and no timeout option (python_umas.connection). | No obligation: the Python values are not options. | deferred (Python-only constants) | none |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-TRANS-TCP | Default transport `tcp` (Drv:70-72). Go: `UmasPort` 502 (go_umas.go_driver, `go_default_port`). Python: port 502 (python_umas.configuration). Java's default port is not stated in the manifest (UNVERIFIED). | `umas://host` uses port 502; an explicit port overrides it. | planned (M5) | `planned:umas_default_port_is_502`, `planned:umas_explicit_port_overrides_the_default` |
| UMAS-TRANS-TEST | `test` transport declared (Drv:74-76). Python: tcp only. | Rejected with `InvalidConfig`; Stonecrab uses its own fake transport. | deferred (follows MB-TCP-TRANSPORTS-TEST) | none |

## Address grammar
Symbolic names only (Tag:44-45): a name, optional bracketed indices, dotted members, each
with optional indices, for example `g_r32`, `g_plant.meta.r32`, `g_arrInt[3]`,
`g_plant.items[2].value`. The pattern is `^([a-zA-Z_]\w*)(\[\d+])*(\.([a-zA-Z_]\w*)(\[\d+])*)*$`.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-ADDR-SYMBOLIC | The pattern above; a name must start with a letter or underscore (Tag:44-45). Go uses the same pattern, "byte for byte" per its comment, not re-read (go_umas.notes, GoTag `go_tag`, 32-41). | The same grammar. | planned (M5) | `planned:umas_address_of_a_global_variable_parses`, `planned:umas_address_of_a_nested_member_parses`, `planned:umas_address_starting_with_a_digit_is_invalid_address` |
| UMAS-ADDR-INDEX | Bracketed indices on any path segment; a range must be contiguous, so only the last dimension of the trailing selection may span more than one element: `a[1].b[2..5]` is one run, `a[1..3].b` is refused (Tag, `AddressConstraints.withOnlyTrailingDimensionMayBeRange`). | The same rule through the shared array notation. | planned (M5) | `planned:umas_trailing_range_is_one_run`, `planned:umas_interior_range_is_invalid_address` |
| UMAS-ADDR-TYPE | An optional `:TYPE` selects the decoded type; Python's pattern allows `:DATATYPE` (python_umas.tag). Java's type comes from the dictionary. | The dictionary type is authoritative; a `:TYPE` that disagrees is refused. | planned (M5) | `planned:umas_type_suffix_that_disagrees_with_the_dictionary_is_refused` |
| UMAS-ADDR-LOCATED | Java has no located `%` addresses (Tag). Python maps `%SW<n>` to an INT in system block 0x2B at word offset n*2+80 and `%S<n>` to a BOOL at n*2+50; any other `%` address raises (backlog `python_umas`, "Located system bit and word addresses"; evidence `system_words`, PyDev:490-514). The `%S` branch tests `tag[1:2]` but parses the number from `tag[3:]`, which looks off by one (UNCERTAIN). | Not implemented. | deferred (Python-only capability; backlog `python_umas`) | none |
| UMAS-ADDR-NO-QUERY | `UmasTagHandler.parseQuery` returns null, so browse hands out the whole table for any query (Conn:832-840; go_umas backlog "Browse query filtering"). | See UMAS-BROWSE-QUERY. | planned (M5) | `planned:umas_browse_with_any_query_returns_the_whole_table` |

## Data types
`UmasDataType` (Mspec:591-617) carries a data size and a request size per type. Java reads
and writes STRING, TIME, DATE, TOD and DATE_AND_TIME explicitly (Conn:597-646, 776-820);
the primitives go through the generated parser. Unknown (non-primitive) symbol types read
the first four bytes (`dataSizeIndex` 3), "same as plc4j per the Go comment"; custom types
come back as raw bytes (go_umas.notes).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-TYPE-BOOL | BOOL (1) and EBOOL (25), one byte. | `bool`. | planned (M5) | `planned:umas_bool_decodes_nonzero_as_true` |
| UMAS-TYPE-INT | INT (4), two bytes, little-endian signed; UINT (5) unsigned. | `i16`, `u16`. | planned (M5) | `planned:umas_int_decodes_negative_values`, `planned:umas_uint_decodes_0xffff` |
| UMAS-TYPE-DINT | DINT (6) and UDINT (7), four bytes. | `i32`, `u32`. | planned (M5) | `planned:umas_dint_decodes_little_endian`, `planned:umas_udint_decodes_little_endian` |
| UMAS-TYPE-REAL | REAL (8), four bytes IEEE 754. | `f32`. | planned (M5) | `planned:umas_real_decodes_ieee754` |
| UMAS-TYPE-BYTE-WORD | BYTE (21), WORD (22), DWORD (23). | `u8`, `u16`, `u32` as bit strings. | planned (M5) | `planned:umas_byte_word_dword_decode_unsigned` |
| UMAS-TYPE-STRING | STRING (9), read as a byte array; Go write accepts ASCII only (GoVal:297-370). | A string of the symbol's length; a non-ASCII write is refused. | planned (M5) | `planned:umas_string_read_stops_at_the_symbol_length`, `planned:umas_string_write_of_non_ascii_is_refused` |
| UMAS-TYPE-TIME | TIME (10), four bytes, hand-decoded (GoVal:71-124). | A duration in milliseconds. | planned (M5) | `planned:umas_time_decodes_milliseconds`, `planned:umas_time_write_round_trips` |
| UMAS-TYPE-DATE | DATE (14), four bytes, hand-decoded (GoVal:71-124). | A calendar date. | planned (M5) | `planned:umas_date_decodes_a_calendar_date`, `planned:umas_date_write_round_trips` |
| UMAS-TYPE-TOD | TOD (15), time of day, four bytes (GoVal:126-152). | A time of day. | planned (M5) | `planned:umas_tod_decodes_a_time_of_day`, `planned:umas_tod_write_round_trips` |
| UMAS-TYPE-DATE-AND-TIME | DATE_AND_TIME (16), eight bytes (python generated parse, `DataItem.py:256-296`). | A date and time. | planned (M5) | `planned:umas_date_and_time_decodes`, `planned:umas_date_and_time_write_round_trips` |
| UMAS-TYPE-UNKNOWN | Enum values 2, 3, 11 to 13, 17 to 20 and 24 are named UNKNOWN and map to BOOL or BYTE (Mspec:591-617). | Read as raw bytes; not writable. | deferred (upstream placeholder: enum value with no meaning) | none |
| UMAS-TYPE-CUSTOM | Custom (UDT) variables return raw bytes in Go; the dictionary holds the UDT definitions (go_umas.notes; python_umas.project_sync). | Reading a struct as a whole returns raw bytes; members are read by dotted path. | planned (M5) | `planned:umas_struct_read_returns_raw_bytes`, `planned:umas_struct_member_read_decodes_the_member` |

## Browse
Java walks the dictionary with `BrowseTreeBuilder` (Conn:832-840). Go follows array element
types and struct members to depth 16 (GoBrowse:41-126). Python builds a `PlcBrowseItem`
tree of elementary, custom and array variables with readable and writable hard-coded true
(PyDev:626-700).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-BROWSE-TREE | The data dictionary becomes a tree of elementary, custom and array variables (umas.operations.browse; PyDev:626-700). | The same tree; `readable` and `writable` are true for every symbol, as upstream. | planned (M5) | `planned:umas_browse_tree_has_elementary_custom_and_array_nodes`, `planned:umas_browse_marks_every_symbol_readable_and_writable` |
| UMAS-BROWSE-ARRAY | `browser-generate-array-nodes` adds artificial element nodes (Cfg:44-47). | Per UMAS-CFG-ARRAY-NODES. | planned (M5) | `planned:umas_browse_generates_array_element_nodes` |
| UMAS-BROWSE-QUERY | Java: any query returns the whole table. Python: the query is ignored. Go: glob filters with `*` and `?`, case-insensitive, empty query rejected, several queries mean several filters (backlog `go_umas`, "Browse query filtering"; GoQuery:47-68). | Go's glob filter is the planned behavior; a request with no query still returns everything. | planned (M5) | `planned:umas_browse_query_with_star_matches_a_prefix`, `planned:umas_browse_query_is_case_insensitive`, `planned:umas_browse_empty_query_is_rejected` |
| UMAS-BROWSE-DEPTH | Java shows no depth bound in the cited range; Go bounds nesting at 16 (`maxTypeNestingDepth`) against self-referential types (backlog `go_umas`, "Browse recursion bound"; GoBrowse:96-126). | Bound at 16 and report the cut-off rather than loop. | planned (M5) | `planned:umas_browse_stops_a_self_referential_type_at_depth_16` |
| UMAS-BROWSE-INTERCEPTOR | Java `onBrowseWithInterceptor` lets a caller see each item as it is built; Python has no interceptor (python_umas.notes, LANG GAP). | No interceptor API in Stonecrab. | deferred (no Stonecrab API for it) | none |

## Wire format
UMAS in a Modbus PDU with function code 0x5A, inside the Modbus TCP ADU; the PDU is
little-endian (Mspec:55-90; python_umas.protocol).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-WIRE-ENVELOPE | MBAP header (transaction id, protocol 0, length, unit id) followed by the Modbus PDU, whose function code 0x5A selects a UMAS PDU; an error flag with a Modbus exception code is a Modbus error (Mspec:55-90). | The MBAP framer of S22 carries the envelope; the exception path reuses MB-EXC-*. | planned (M5) | `planned:umas_envelope_uses_function_code_0x5a`, `planned:umas_modbus_exception_reply_maps_through_mb_exc` |
| UMAS-WIRE-PDU | One PDU item per request key (0x01 InitComms, 0x02 PlcIdent, 0x03 ProjectInfo, 0x04 PlcStatus, 0x0A Repeat, 0x20 ReadMemoryBlock, 0x22 ReadVariable, 0x23 WriteVariable, 0x26 ReadUnlocatedVariableNames, 0xFD error, 0xFE success) (Mspec:108-345). | Only the commands the drivers use are encoded and decoded; others are decoded as opaque so the session can skip them. | planned (M5) | `planned:umas_read_variable_request_bytes_match_the_vector`, `planned:umas_write_variable_request_bytes_match_the_vector`, `planned:umas_unmodelled_command_decodes_as_opaque` |
| UMAS-WIRE-ERROR | `UmasPDUErrorResponse` (0xFD) answers a refused request; Python reports every read item OK and checks only the write response key 254 (python_umas.read, `write`). | A 0xFD reply maps to a remote error for the tags of that request; Python's blanket OK is a defect and not ported. | planned (M5) | `planned:umas_error_response_marks_the_tags_remote_error` |
| UMAS-WIRE-DESYNC | Python: a non-UMAS reply logs "Modbus Error Message Received" and closes; `close()` iterates `messages.items` without calling it (PyDev, `python_umas.protocol` 34-99). | Defect, not ported: an unexpected reply fails the matching request and the session continues. | planned (M5) | `planned:umas_unexpected_reply_fails_only_the_matching_request` |
| UMAS-WIRE-V0-V1 | A v0 and a v1 mspec both exist; which one each driver builds against is not confirmed (umas.notes, UNCERTAIN). | v1 is the baseline (the manifest cites v1); a v0 option is not planned. | deferred (v0 not confirmed as used) | none |

## Request batching and offsets

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-BATCH-ONE-PER-TAG | Java and Go send one ReadVariable per tag (go_umas.operations.read; GoRead:109-156). | One request per tag. | planned (M5) | `planned:umas_read_sends_one_request_per_tag` |
| UMAS-BATCH-CHUNKS | Python batches tags into frame-size chunks sorted by memory address (python_umas.read, `max_frame_size`). | Not planned; the frame size is respected per request (UMAS-CFG-MAX-FRAME). | deferred (Python-only optimization) | none |
| UMAS-BATCH-OFFSET-OVERFLOW | Go refuses a symbol offset that does not fit the `baseOffset` field; Java shows no such check in the cited ranges (backlog `go_umas`, "Offset overflow check"; GoRead:109-156). | Refuse it at request building with a typed error. | planned (M5) | `planned:umas_symbol_offset_that_overflows_base_offset_is_refused` |
| UMAS-BATCH-PROJECT-SYNC | Python re-checks the project CRC (PlcStatus blocks 3 and 4) and reloads names and UDTs before every read, write or browse (python_umas.project_sync). | Not planned; the dictionary is loaded at connect (UMAS-OP-CONNECT). A reload on project change is a candidate for a later slice. | deferred (Python-only behavior) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| UMAS-QUIRK-UNIT-ID-PY | Python leaves `unit_identifier` unset when the key is supplied. | python_umas.configuration | deferred (not reproduced; see UMAS-CFG-UNIT-ID) | none |
| UMAS-QUIRK-PY-CLOSE | Python `close()` iterates `messages.items` without calling it and so raises. | python_umas.protocol | deferred (not reproduced; see UMAS-WIRE-DESYNC) | none |
| UMAS-QUIRK-PY-WRITE | Python's write helper builds a `PlcReadResponse` and adds the type code to the byte count as if it were a length. | python_umas.write | deferred (not reproduced) | none |
| UMAS-QUIRK-PY-TIMEOUT | Python's `except TimeoutError` catches `asyncio.TimeoutError` only on 3.11 and later. | python_umas.connection | deferred (not reproduced) | none |
| UMAS-QUIRK-PY-S-OFFSET | The `%S` branch looks off by one for a one-letter area (UNCERTAIN). | python_umas.system_words | deferred (see UMAS-ADDR-LOCATED) | none |
| UMAS-QUIRK-QUERY | Java's `parseQuery` returns null, so every query returns everything. | umas.browse, Conn:832-840 | deferred (not reproduced; see UMAS-BROWSE-QUERY) | none |

## Divergences from upstream

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-DIV-QUERY | Java returns the whole table for any query. | Glob filtering as in Go, so a caller can narrow a browse. | planned (M5) | `planned:umas_browse_query_with_star_matches_a_prefix` |
| UMAS-DIV-TIMEOUT | Java maps a ping failure to REMOTE_ERROR (Conn:489-499). | A timeout maps to the typed timeout error, as in Go for reads. | planned (M5) | `planned:umas_read_timeout_is_a_request_timeout` |

## Backlog from other languages
Four `[[backlog]]` rows name `go_umas` or `python_umas`; the rows below cite them.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| UMAS-BACKLOG-GO-QUERY | `go_umas`, "Browse query filtering" (evidence `go_query`). | Ported: UMAS-BROWSE-QUERY. | planned (M5) | `planned:umas_browse_query_with_star_matches_a_prefix` |
| UMAS-BACKLOG-GO-DEPTH | `go_umas`, "Browse recursion bound" (evidence `go_browse_item`). | Ported: UMAS-BROWSE-DEPTH. | planned (M5) | `planned:umas_browse_stops_a_self_referential_type_at_depth_16` |
| UMAS-BACKLOG-GO-OFFSET | `go_umas`, "Offset overflow check" (evidence `go_read`). | Ported: UMAS-BATCH-OFFSET-OVERFLOW. | planned (M5) | `planned:umas_symbol_offset_that_overflows_base_offset_is_refused` |
| UMAS-BACKLOG-PY-SYSTEM | `python_umas`, "Located system bit and word addresses (%S and %SW)" (evidence `system_words`). | Deferred: UMAS-ADDR-LOCATED. | deferred (Python-only capability) | none |

## UNVERIFIED
- Java's default port: only the Go and Python constants (502) are cited.
- Which of the v0 and v1 mspecs each driver builds against.
- Java's symbol-table lookup and struct-member handling beyond the cited ranges, which
  were not compared with Go.
- Whether the Java symbolic pattern is byte for byte the Go one (the Go comment says so).
- The meaning of the UNKNOWN data-type codes and of the InitComms negotiation fields; the
  Schneider UMAS documentation is not public and is not in the repository.
- The Java pairing-key handling at connect, which the manifest names and Python omits.
