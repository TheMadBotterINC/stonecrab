# Allen-Bradley ETH (ab-eth) parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing in this record is implemented. The driver is a read-only
driver for legacy PLC-5 and SLC style "N-file" addressing over a CIP encapsulation on port
2222. It belongs to M5 (the rest of the baseline). Every behavior row is `planned (M5)` or
`deferred`. The Go driver (`go_abeth`) is recorded in its own section and in the backlog
table. Test ids are `planned:` names, not existing tests; no device model or firmware is
named yet.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn; the Test ids column names
  at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: recorded so the omission is visible; not ported now.
  `deferred (upstream placeholder)` marks an unimplemented or null upstream behavior, which
  does not become an obligation (SPEC, "Compatibility baseline").
- No row is `implemented` yet, so `scripts/parity-check.sh` does not require these ids to
  appear in code.

## File key
Paths are relative to the repository root of apache/plc4x. `J` abbreviates
`plc4j/drivers/ab-eth/src/main/java/org/apache/plc4x/java/abeth`.

| Key | Path |
| --- | --- |
| driver | `J/AbEthDriver.java` |
| config | `J/configuration/AbEthConfiguration.java` |
| connection, connection_read | `J/AbEthConnection.java` |
| codec | `J/AbEthMessageCodec.java` |
| tag | `J/tag/AbEthTag.java` |
| filetype | `J/types/FileType.java` |
| mspec | `protocols/ab-eth/src/main/resources/protocols/abeth/ab-eth.mspec` |
| connbase_ping_default | `plc4j/spi/drivers/.../spi/drivers/ConnectionBase.java` |
| go_* keys | `plc4go/internal/abeth/*.go`, see the Go section |

## Operations

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-OP-CONNECT | Code `ab-eth`, default port 2222 (driver 41-80). The handshake is a `CIPEncapsulationConnectionRequest` (command 0x0101) whose response (0x0201) carries the session handle used on every read (manifest notes; mspec 20-40). | `ab-eth:tcp://host[:port]`, same handshake; the session handle is kept for reads. | planned (M5) | `planned:abeth_connect_obtains_a_session_handle`, `planned:abeth_uri_defaults_to_port_2222` |
| ABETH-OP-CLOSE | Close path not cited for Java; Go just drops the transport, with no disconnect packet (go_close 220-231). | Close drops the transport. | planned (M5) | `planned:abeth_close_drops_the_transport` |
| ABETH-OP-READ | Sequential per tag, one in flight (`getMaxConcurrentRequests` 1): a DF1 Protected Typed Logical Read (command 0x0F, A2; response 0x4F) inside `CIPEncapsulationReadRequest` with a 16-bit transaction counter, bounded by `request-timeout-ms` (connection_read 239-361; mspec 20-40). | Same sequential read with the transaction counter. | planned (M5) | `planned:abeth_read_of_an_integer_element_decodes_little_endian`, `planned:abeth_read_runs_one_request_at_a_time`, `planned:abeth_read_matches_the_response_by_transaction_counter` |
| ABETH-OP-WRITE | Not supported: no `canWrite`, no `onWrite`; the connection Javadoc says "Read-only driver" (connection 65-121). | `Capabilities.write = false`; a write fails with the typed unsupported error. | deferred (upstream placeholder) | none |
| ABETH-OP-SUBSCRIBE | Polling emulation only: the connection extends `PollingSubscriptionConnectionBase`, so metadata reports subscribe although the driver class does not set `canSubscribe` (connection 65-121). | Cyclic and change-of-state subscribe through the Stonecrab polling layer over the read path. | planned (M5) | `planned:abeth_subscribe_polls_the_read_path`, `planned:abeth_capabilities_report_subscribe_from_the_connection` |
| ABETH-OP-BROWSE | Not supported. | `Capabilities.browse = false`. | deferred (upstream placeholder) | none |
| ABETH-OP-DISCOVER | Not supported. | `Capabilities.discover = false`. | deferred (upstream placeholder) | none |
| ABETH-OP-PING | Not supported: no `canPing`, no `onPing`; the base fails with "Ping not supported" (connbase_ping_default 469-471). | `Capabilities.ping = false`; ping fails with the typed unsupported error. | deferred (upstream placeholder) | none |

## Capabilities

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-CAP-FLAGS | Only `canRead` is set at the driver (driver 82-85); the connection adds polled subscribe. | Capability set `{read, subscribe}`. | planned (M5) | `planned:abeth_capability_set_matches_the_inventory` |

## Configuration
Two parameters on `AbEthConfiguration` (config 26-36).

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-CFG-STATION | `station`, default 0, used as the DF1 destination address (config 26-36; manifest notes). | Same name and default; a value above 255 is a typed `InvalidConfig` (the Go rule, ABETH-BL-CONFIG-VALIDATION). | planned (M5) | `planned:abeth_config_station_defaults_to_zero` |
| ABETH-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, default 10000 (config 26-36). | Same name and default. | planned (M5) | `planned:abeth_config_request_timeout_defaults_to_ten_seconds` |

## Connection string and transports

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-TRANSPORT-TCP | Default and supported `tcp`, port 2222 (driver 64-80). | `ab-eth:tcp://host[:port]`; other transports are rejected with `InvalidConfig`. | planned (M5) | `planned:abeth_uri_rejects_other_transports` |
| ABETH-TRANSPORT-TEST | `test` transport supported (driver 64-72); behavior only declared. | Not ported; Stonecrab tests use `FakeTransport`. | deferred (no release) | none |

## Address grammar
Java pattern `^N(?<fileNumber>\d{1,7}):(?<elementNumber>\d{1,7})(/(?<bitNumber>\d{1,7}))?:(?<dataType>[a-zA-Z_]+)(\[(?<size>\d+)])?`
(tag 33-35). Numbers are parsed with `Short.parseShort` and no one-byte check (tag 109-111).

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-ADDR-NFILE | `N<file>:<element>:<TYPE>`, file and element up to seven digits (tag 33-35). | `NFileAddress` with `parse`, same accepted text. | planned (M5) | `planned:abeth_address_parses_an_integer_element`, `planned:abeth_address_rejects_eight_digit_file_numbers` |
| ABETH-ADDR-BIT | Optional `/<bit>` after the element, up to seven digits (tag 33-35). | Same text; the bit number is checked as the SINGLEBIT type requires (ABETH-BL-ADDRESS-RANGE for the Go rule). | planned (M5) | `planned:abeth_address_parses_a_bit_suffix` |
| ABETH-ADDR-SIZE | Optional `[size]` element count (tag 33-35). | Same text. | planned (M5) | `planned:abeth_address_parses_an_element_count` |
| ABETH-ADDR-SHORT-OVERFLOW | `Short.parseShort` on the numbers: a value above 32767 fails with a number-format error, not a typed address error (tag 109-111). | A typed `InvalidAddress` instead of a raw parse failure. | planned (M5) | `planned:abeth_address_over_32767_is_a_typed_error` |

## Limits

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-LIMIT-CONCURRENCY | One request in flight (`getMaxConcurrentRequests` 1; connection_read 239-361). | One request in flight per connection. | planned (M5) | `planned:abeth_read_runs_one_request_at_a_time` |
| ABETH-LIMIT-FRAME | Total frame = payload length + 28; length read at offset 2 after a four-byte peek (codec 24-52, 69-73). | Same framing rule. | planned (M5) | `planned:abeth_framer_adds_28_to_the_payload_length` |

## Data types
`FileType` (filetype 26-42) has fifteen entries. Only the first four decode; the others
are listed in the manifest as returning a null value.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-TYPE-INTEGER | INTEGER (0x89) decodes as a little-endian signed 16-bit value (connection_read 316-378). | Same. | planned (M5) | `planned:abeth_read_of_an_integer_element_decodes_little_endian` |
| ABETH-TYPE-WORD | WORD (code 0x89, two bytes) decodes as little-endian signed (connection_read 316-378). | Signed, as Java; the Go unsigned result is ABETH-BL-SIGNED-WORD. | planned (M5) | `planned:abeth_read_of_a_word_sign_extends_little_endian` |
| ABETH-TYPE-DWORD | DWORD (code 0x89, four bytes) decodes as little-endian signed (connection_read 316-378). | Same. | planned (M5) | `planned:abeth_read_of_a_dword_sign_extends_little_endian` |
| ABETH-TYPE-SINGLEBIT | SINGLEBIT (code 0x89, two bytes) extracts one bit (connection_read 316-378). | Same. | planned (M5) | `planned:abeth_read_of_a_single_bit_extracts_the_bit` |
| ABETH-TYPE-OTHERS | STATUS 0x84, BIT 0x85, TIMER 0x86, COUNTER 0x87, CONTROL 0x88, FLOAT 0x8A, OUTPUT 0x8B, INPUT 0x8C, STRING 0x8D, ASCII 0x8E, BCD 0x8F: `decodeReadResponse` logs "Decoding of file type ... not implemented" and returns a null value with response code OK when the status is 0 (connection_read 316-378; filetype 26-42). | Not decoded; the item reports UNSUPPORTED with no value rather than OK with null (Go's choice, ABETH-BL-RESPONSE-CODES). | deferred (upstream placeholder) | none |

## Upstream quirks

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-QUIRK-RESPONSE-CODES | A timeout maps to NOT_FOUND, any non-zero DF1 status to NOT_FOUND, and an unimplemented file type to OK with a null value (connection_read 262-268, 316-378). | Stonecrab maps a timeout to a timeout error and does not report OK for a missing value; see ABETH-BL-RESPONSE-CODES. | planned (M5) | `planned:abeth_nonzero_df1_status_is_not_found`, `planned:abeth_timeout_is_reported_as_a_timeout` |
| ABETH-QUIRK-BYTE-ORDER | The mspec header declares BIG_ENDIAN while the codec buffer options force LITTLE_ENDIAN; the frame length is read big-endian (codec 49-57, 69-73). Not reconciled by the manifest. | Settled from captured frames before M5 exit; no claim until then. | planned (M5) | `planned:abeth_frame_vectors_pin_the_wire_byte_order` |
| ABETH-QUIRK-SUBSCRIBE-FLAG | The driver does not set `canSubscribe`, but the polling base makes the connection report it. | Report subscribe from the connection. | planned (M5) | `planned:abeth_capabilities_report_subscribe_from_the_connection` |

## Divergences from upstream
No divergence is accepted yet; ABETH-TYPE-OTHERS and ABETH-QUIRK-RESPONSE-CODES are the
candidates M5 will decide.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-DIV-NONE | None recorded. | None. | planned (M5) | `planned:abeth_divergence_table_is_reviewed_at_m5_exit` |

## Go driver (`go_abeth`)
Go matches Java's capability set: read, polled subscribe, no write, browse, discover or
real ping (manifest note "LANG GAP: none found in read, handshake or addressing").

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-GO-READ | Per-tag DF1 Protected Typed Logical Read, transaction width 1, bounded by `request-timeout-ms` (go_read 117-155). | Covered by ABETH-OP-READ. | planned (M5) | `planned:abeth_read_matches_the_response_by_transaction_counter` |
| ABETH-GO-SUBSCRIBE | `DefaultPollingSubscriber` over the read path, `ProvidesSubscribing=true` (go_metadata 297-307, go_polling 322-334). | Covered by ABETH-OP-SUBSCRIBE. | planned (M5) | `planned:abeth_subscribe_polls_the_read_path` |
| ABETH-GO-WRITE | No Writer; `ProvidesWriting=false` (go_metadata 297-307). | As ABETH-OP-WRITE. | deferred (upstream placeholder) | none |
| ABETH-GO-BROWSE | `ParseQuery` returns an error (go_tag_handler 132-135). | As ABETH-OP-BROWSE. | deferred (upstream placeholder) | none |
| ABETH-GO-PING | Inherited flag check, no wire round trip. | As ABETH-OP-PING. | deferred (upstream placeholder) | none |
| ABETH-GO-BYTE-ORDER | The Go codec writes and parses big-endian and reads the length big-endian (go_codec_send 90, go_codec_length 176); the Java buffers are little-endian. | Settled by ABETH-QUIRK-BYTE-ORDER. | planned (M5) | `planned:abeth_frame_vectors_pin_the_wire_byte_order` |
| ABETH-GO-DEFAULTS | Station 0, `request-timeout-ms` 10000, port 2222, DF1 source address 5, sub-element 0 (manifest notes). | DF1 source address and sub-element are fixed as upstream. | planned (M5) | `planned:abeth_request_uses_source_address_5_and_sub_element_0` |
| ABETH-GO-STRAY-DRAIN | A goroutine drains stray unmatched packets for the life of the connection. | Unmatched frames are dropped and counted, never queued without bound. | planned (M5) | `planned:abeth_unmatched_frames_are_dropped` |

## Backlog from other languages
The first three rows are the `[[backlog]]` entries of the manifest for `go_abeth`; the
fourth is the Go `LANG GAP` note. All are deferred until a maintainer decides to adopt them.

| Row id | Upstream behavior (key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| ABETH-BL-RESPONSE-CODES | "Distinct response codes": Go maps a timeout to REQUEST_TIMEOUT, short data to INVALID_DATA and an unimplemented file type to UNSUPPORTED; Java uses NOT_FOUND and OK with null (go_decode 157-226; java_timeout_code 262-268, java_decode 316-378). | Java mapping is the baseline; adoption of Go's distinct codes is decided in M5. | deferred (non-Java backlog) | none |
| ABETH-BL-ADDRESS-RANGE | "Address range checking": Go requires numbers to fit one byte, the SINGLEBIT bit number in 0 to 15, a non-zero byte size and an agreeing WORD/DWORD size suffix (go_tag_handler). | Not ported; Stonecrab keeps the typed-error rule of ABETH-ADDR-SHORT-OVERFLOW only. | deferred (non-Java backlog) | none |
| ABETH-BL-CONFIG-VALIDATION | "Configuration validation": Go rejects `request-timeout-ms` of 0 and a `station` over 255 at parse (go_config 60-89). | Not ported as a parity obligation; ABETH-CFG-STATION names the station bound as a Stonecrab choice. | deferred (non-Java backlog) | none |
| ABETH-BL-SIGNED-WORD | Go returns unsigned WORD and DWORD, a deliberate deviation from Java's sign extension (go_decode 157-226). | Not ported; Java signedness is planned (ABETH-TYPE-WORD). | deferred (non-Java backlog) | none |

## UNVERIFIED
- The byte-order question (mspec BIG_ENDIAN, Java buffers LITTLE_ENDIAN, Go BigEndian) is
  not resolved by the inventory.
- The Java close path, and the behavior of the `test` transport.
- The Go range-checking details beyond the manifest's summary.
- No device model or firmware (PLC-5, SLC 5/05) is named; no hardware evidence exists.
