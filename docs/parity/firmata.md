# Firmata parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The Firmata driver is planned for milestone M5
(docs/POST_ALPHA_PLAN.md section 4). Upstream is write (digital pins only) and
subscribe, with no read, in both Java and Go. Every row is `planned` or `deferred`; no
listed test exists.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: not ported now (non-Java-only capability, upstream placeholder,
  or a decision). Test ids may be `none`.

Later statuses follow docs/SPEC.md "Compatibility baseline"; device models and firmware
(boards and Firmata versions) are recorded with hardware-verified test ids, none of
which exist.

## File key
Citations use the manifest keys of `[inventory.firmata]` (Java) and
`[inventory.go_firmata]` (Go keys prefixed `go_`, Java comparison keys `java_`).

| Key | Path |
| --- | --- |
| driver_caps, java_driver_caps | `plc4j/drivers/firmata/src/main/java/org/apache/plc4x/java/firmata/FirmataDriver.java` |
| write, subscribe, unsub, java_write, java_subscribe, java_unsub | `.../firmata/FirmataConnection.java` |
| tag_analog, tag_digital | `.../firmata/tag/FirmataTagAnalog.java`, `FirmataTagDigital.java` |
| go_driver, go_connection_doc, go_pin_modes, go_connect, go_handshake, go_metadata | `plc4go/internal/firmata/Driver.go`, `Connection.go` |
| go_write_doc, go_write_tag | `plc4go/internal/firmata/Writer.go` |
| go_subscribe_type, go_unsub_doc, go_unsub | `plc4go/internal/firmata/Subscriber.go` |
| go_config | `plc4go/internal/firmata/Configuration.go` |
| go_tags | `plc4go/internal/firmata/TagHandler.go` |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-OP-READ | Not supported; no `onRead` in Java, `ProvidesReading=false` and no read path in Go (operations.read; `go_connection_doc`, Connection.go:50-53). | None. | deferred (upstream has no read) | none |
| FIRMATA-OP-WRITE | Digital pins only. A non-digital tag throws PlcRuntimeException "Writing only supported for digital pins"; the pin is set to OUTPUT on first use, an error if it is already configured otherwise; fire-and-forget, always reports OK (`write`, FirmataConnection.java:257-307). | Digital write: claim the pin as OUTPUT, send SetDigitalPinValue; OK means the bytes were sent. | planned (M5) | `planned:firmata_write_of_a_digital_pin_sends_set_digital_pin_value`, `planned:firmata_write_claims_the_pin_as_output_once` |
| FIRMATA-OP-WRITE-ERRORS | Go isolates errors per tag: analog tag UNSUPPORTED, bad value INVALID_DATA, pin already used in another mode INVALID_ADDRESS, send failure INTERNAL_ERROR; the other tags still go out (`go_write_tag`, Writer.go:89-128; backlog "Write error isolation"). | Per-tag codes as Go. | planned (M5) | `planned:firmata_write_of_an_analog_tag_is_unsupported_for_that_tag_only`, `planned:firmata_write_to_a_pin_in_input_mode_is_invalid_address` |
| FIRMATA-OP-WRITE-ACK | Firmata has no write acknowledgement (`go_write_doc`, Writer.go:40-44). | OK is documented as "bytes sent". | planned (M5) | `planned:firmata_write_reports_ok_without_an_acknowledgement` |
| FIRMATA-OP-SUBSCRIBE-DIGITAL | Digital tags send SetPinMode (INPUT by default, PULLUP on request) and per-port REPORT_DIGITAL; a mismatched pin mode throws PlcInvalidTagException (`subscribe`, FirmataConnection.java:313-395). | Native push of digital changes. | planned (M5) | `planned:firmata_subscribe_to_a_digital_pin_reports_changes`, `planned:firmata_subscribe_with_pullup_sets_the_pullup_mode` |
| FIRMATA-OP-SUBSCRIBE-ANALOG | Analog tags send REPORT_ANALOG (`subscribe`). | Native push of analog values. | planned (M5) | `planned:firmata_subscribe_to_an_analog_pin_reports_values` |
| FIRMATA-OP-SUBSCRIBE-CHANGE-ONLY | The Go worker caches the latest analog and digital values and publishes only changes (notes). | Publish only changes. | planned (M5) | `planned:firmata_unchanged_value_is_not_published_again` |
| FIRMATA-OP-UNSUBSCRIBE | Java returns a completed response without telling the board (`unsub`, FirmataConnection.java:397-420); Go drops the local handle and the board keeps reporting (`go_unsub_doc`, Subscriber.go:225-229; `go_unsub`, 256-295). | Local handle drop only; the limitation is documented in the API docs. | planned (M5) | `planned:firmata_unsubscribe_leaves_board_reporting_on` |
| FIRMATA-OP-CONNECT | Serial default; connect, then handshake: system reset plus a report-firmware request (Go sends the explicit request because TCP boards do not restart and never volunteer the report); Java connect-time firmware handling is only skimmed (`go_connect`, Connection.go:161-174; `go_handshake`, 198-263; backlog "Handshake on TCP boards"). | Connect and handshake as Go. | planned (M5) | `planned:firmata_connect_requests_the_firmware_report`, `planned:firmata_handshake_times_out_after_request_timeout` |
| FIRMATA-OP-BROWSE | Not supported. | None. | deferred (upstream has no browse) | none |
| FIRMATA-OP-DISCOVER | Not supported. | None. | deferred (upstream has no discovery) | none |
| FIRMATA-OP-PING | Not supported in Java; Go `Ping` is the inherited local flag check (operations.ping). | None. | deferred (upstream has no ping) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-CAP-FLAGS | `canWrite` and `canSubscribe` only (`driver_caps`, FirmataDriver.java:64-87); Go metadata: reading false, writing true, subscribing true (`go_metadata`, Connection.go:612-622). | Capabilities report write and subscribe true. | planned (M5) | `planned:firmata_capabilities_report_write_and_subscribe` |

## Configuration

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-CFG-REQUEST-TIMEOUT | Go: `request-timeout-ms`, default 10000, bounds the connect handshake (`go_config`, Configuration.go:32-77). Java driver options not inventoried; serial and tcp transport configuration classes exist (notes). | `request-timeout-ms`, default 10000. | planned (M5) | `planned:firmata_config_request_timeout_defaults_to_10000` |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-TRANSPORT-SERIAL | Default transport `serial` (`inventory.firmata.transports`; `go_driver`, Driver.go:36-53). | Serial transport (a shared serial transport is also needed for Modbus RTU in M2). | planned (M5) | `planned:firmata_default_transport_is_serial` |
| FIRMATA-TRANSPORT-TCP | `tcp` supported; Go default port 3030 (`go_driver`). | TCP transport, port default 3030. | planned (M5) | `planned:firmata_tcp_uri_defaults_to_port_3030` |
| FIRMATA-TRANSPORT-TEST | `test` supported. | Tests use `FakeTransport`. | deferred (no release) | none |

## Address grammar

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-ADDR-DIGITAL | `digital:<pin>[array][:PULLUP]` (`tag_digital`, FirmataTagDigital.java:36-37; `go_tags`, TagHandler.go:41-60). | Same grammar. | planned (M5) | `planned:firmata_digital_address_with_pullup_parses` |
| FIRMATA-ADDR-ANALOG | `analog:<pin>[array]` (`tag_analog`, FirmataTagAnalog.java:34; `go_tags`). | Same grammar. | planned (M5) | `planned:firmata_analog_address_parses` |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-LIMIT-PIN-MODE | One mode per digital pin; a conflicting claim is refused (`go_pin_modes`, Connection.go:64-73). | Same pin-claim table. | planned (M5) | `planned:firmata_pin_claimed_as_input_cannot_be_written` |

## Data types

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-TYPE-DIGITAL | Digital pins carry boolean values; selection of several pins by array (`tag_digital`; `go_write_tag`). The value-conversion rules were not inventoried. | Boolean per pin, array for consecutive pins. | planned (M5) | `planned:firmata_digital_array_write_sets_consecutive_pins` |
| FIRMATA-TYPE-ANALOG | Analog values from REPORT_ANALOG (`tag_analog`). Resolution not inventoried. | Unsigned integer value per pin. | planned (M5) | `planned:firmata_analog_report_decodes_a_value` |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| FIRMATA-QUIRK-WRITE-ALWAYS-OK | Java reports OK for every sent write and throws for non-digital tags, aborting the request. | `write` | planned (M5) | Not reproduced, see `FIRMATA-OP-WRITE-ERRORS`: `planned:firmata_write_of_an_analog_tag_is_unsupported_for_that_tag_only` |
| FIRMATA-QUIRK-NO-STOP | Unsubscribe cannot stop reporting at the board. | `unsub` | planned (M5) | Ported and documented, see `FIRMATA-OP-UNSUBSCRIBE`: `planned:firmata_unsubscribe_leaves_board_reporting_on` |

## Divergences from upstream

| Row id | Divergence | Rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-DIV-WRITE-ERRORS | Per-tag write codes, no exception for an analog tag. | Stonecrab reports errors per tag; Go does likewise. | planned (M5) | `planned:firmata_write_of_an_analog_tag_is_unsupported_for_that_tag_only` |

## Backlog from other languages

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| FIRMATA-BACKLOG-SUBSCRIBE-TYPE | Go: only change-of-state accepted, cyclic and event requests UNSUPPORTED (backlog "Subscription type enforcement"; `go_subscribe_type`, Subscriber.go:124-143). Java serves all kinds as change-of-state (`java_subscribe`). | Go behavior. | planned (M5) | `planned:firmata_cyclic_subscription_is_unsupported` |
| FIRMATA-BACKLOG-WRITE-ISOLATION | Go: each tag gets its own response code (backlog "Write error isolation"; `go_write_tag`). | See `FIRMATA-OP-WRITE-ERRORS`. | planned (M5) | `planned:firmata_write_of_an_analog_tag_is_unsupported_for_that_tag_only` |
| FIRMATA-BACKLOG-HANDSHAKE-TCP | Go: explicit report-firmware request after the system reset (backlog "Handshake on TCP boards"; `go_handshake`). | See `FIRMATA-OP-CONNECT`. | planned (M5) | `planned:firmata_connect_requests_the_firmware_report` |
| FIRMATA-BACKLOG-PIN-ROLLBACK | Go: a claim whose messages could not be sent is rolled back so a retry resends the mode messages (backlog "Pin claim rollback"; `go_write_tag`). | Same rollback. | planned (M5) | `planned:firmata_failed_pin_claim_is_rolled_back_and_retried` |

## UNVERIFIED
- Java connect-time firmware handling (FirmataConnection.java:143-210): skimmed only, so
  the handshake comparison is partial.
- Java driver options beyond the serial and TCP transport configuration classes.
- Value conversion and analog resolution rules.
- Real boards: no board model or firmware has been tested.
