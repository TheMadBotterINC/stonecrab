# Modbus RTU and ASCII parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State at S43: nothing is implemented. Stonecrab has no serial transport and no RTU or ASCII
framing. (S52d adds the `MBS-DEVICE-SLM1C-*` rows of "Hardware evidence": a replay of the
SLM1-C's documented register map at the PDU level over TCP framing, which needs no serial
line; no serial behavior is implemented.) Every other row below is `planned` for milestone M2
(slice S58 serial transport, S59 RTU, S60 ASCII, HW2 for hardware evidence) or `deferred` with
its reason. The PDU codec,
the address grammar, the read batcher, the typed decoder, the byte-order modes and the
exception mapping are shared with Modbus TCP and are not restated here: this record points
at `docs/parity/modbus.md` (rows `MB-*`) for them and lists only what differs on a serial
line. The manifest entries are `inventory.modbus_rtu`, `inventory.modbus_ascii` and
`inventory.transport_serial`; the non-Java drivers are `go_modbus`, `c_modbus` and
`python_modbus`.

Status vocabulary, as used in the Status column (this file uses the first two and
`implemented (documented map, not hardware-verified)` for the S52d rows; the rest is
defined in `docs/parity/modbus.md`):

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one test id prefixed `planned:`; none exists yet.
- `deferred (<reason>)`: Stonecrab does not port the behavior now. The reason is a
  non-Java-only capability, an upstream placeholder, or a decision. Test ids may be `none`.
- `implemented`, `automated-verified`, `hardware-verified`: not used until a slice lands,
  except the S52d rows named above. `hardware-verified` rows will name the device model and
  firmware (HW2).

Row ids use the prefix `MBS-`. Ids are never renumbered. `scripts/parity-check.sh` does not
require the planned ids to appear in code.

## File key
Citations use the manifest key in the first column, then `path:lines` against the pinned
commit. `P` abbreviates `plc4j/drivers/modbus/src/main/java/org/apache/plc4x/java/modbus`,
`ST` abbreviates `plc4j/transports/serial/src/main/java/org/apache/plc4x/java/transport/serial`
and `SPI` abbreviates `plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers`.

| Key | Path |
| --- | --- |
| RtuDrv | `P/rtu/ModbusRtuDriver.java` (manifest `modbus_rtu.driver`) |
| RtuCfg | `P/rtu/config/ModbusRtuConfiguration.java` (`modbus_rtu.config`) |
| RtuSer | `P/rtu/config/ModbusRtuSerialTransportConfiguration.java` (`modbus_rtu.serial_cfg`) |
| RtuCodec | `P/rtu/ModbusRtuMessageCodec.java` (`modbus_rtu.codec`) |
| RtuConn | `P/rtu/ModbusRtuConnection.java` (`modbus_rtu.connection`, `connection_rw`, `write`, `write_pdu`) |
| AscDrv, AscCfg, AscSer | `P/ascii/ModbusAsciiDriver.java`, `P/ascii/config/ModbusAsciiConfiguration.java`, `P/ascii/config/ModbusAsciiSerialTransportConfiguration.java` (`modbus_ascii.driver`, `config`, `serial_cfg`) |
| AscCodec | `P/ascii/ModbusAsciiMessageCodec.java` (`modbus_ascii.codec`) |
| AscConn | `P/ascii/ModbusAsciiConnection.java` (`modbus_ascii.connection`) |
| Mspec | `protocols/modbus/src/main/resources/protocols/modbus/modbus.mspec` (`modbus_rtu.mspec_adu`, `modbus_ascii.mspec`) |
| SerCfg | `ST/config/SerialTransportConfiguration.java` (`transport_serial.config`, `modbus_rtu.serial_transport_cfg`) |
| SerT | `ST/SerialTransport.java` (`transport_serial.code`) |
| SerI | `ST/SerialTransportInstance.java` (`transport_serial.open`, `ring`, `drop_oldest`) |
| Pacer | `ST/WritePacer.java` (`transport_serial.pacer`) |
| Shared | `ST/SharedSerialPortManager.java` (`transport_serial.shared`) |
| Poll | `plc4j/utils/subscription-emulation/.../PollingSubscriptionConnectionBase.java` (`modbus_rtu.polling_base`) |
| Tag | `P/base/tag/ModbusTag.java` (`modbus_rtu.tag_unit_id`) |
| DB, CB | `SPI/DriverBase.java`, `SPI/ConnectionBase.java` (`modbus_rtu.driverbase_flags`, `connbase_meta`) |
| CF | `plc4j/spi/config/src/main/java/org/apache/plc4x/java/spi/config/ConfigurationFactory.java` |
| TcpT, UdpT | `plc4j/transports/tcp/src/main/java/org/apache/plc4x/java/transport/tcp/TcpTransport.java`, `plc4j/transports/udp/src/main/java/org/apache/plc4x/java/transport/udp/UdpTransport.java` |
| GoMb | `plc4go/internal/modbus` (`go_modbus`, ping at `Connection.go:134-211`) |
| GoSer | `plc4go/spi/transports/serial` (`go_transport_serial`) |

## Operations
RTU and ASCII share one operation set (`modbus_ascii.operations` repeats
`modbus_rtu.operations`; the ASCII connection is a diffed copy of the RTU one, manifest
`modbus_rtu.notes`). A row says RTU, ASCII or both.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-OP-CONNECT | Protocol codes `modbus-rtu` and `modbus-ascii` (RtuDrv:45-47, AscDrv:45-47). Default transport `serial` (RtuDrv:77-80). Connect opens the transport and starts the codec; no handshake. Close stops the transport. | `modbus-rtu://` and `modbus-ascii://` connection strings select the serial transport by default; connect opens the port, close releases it. No Modbus traffic on connect. | planned (M2) | `planned:rtu_connect_opens_the_configured_port_without_sending`, `planned:ascii_connect_opens_the_configured_port_without_sending`, `planned:close_releases_the_serial_port` |
| MBS-OP-READ | `onRead` merges tags through `ModbusReadOptimizer` into block reads, each sent as an ADU; max concurrent requests 1 (modbus_rtu.operations.read, RtuConn:145-148, 312-345). ASCII identical. | Reads use the shared `Batcher::plan` and PDU codec (MB-TCP-READ-COIL, MB-TCP-READ-DI, MB-TCP-READ-HR, MB-TCP-READ-IR) wrapped in an RTU or ASCII ADU; one request in flight. | planned (M2) | `planned:rtu_read_of_holding_registers_decodes_big_endian`, `planned:ascii_read_of_coils_unpacks_bits_lsb_first`, `planned:serial_read_sends_one_request_at_a_time` |
| MBS-OP-WRITE | Single tag only: more than one tag fails with "Modbus only supports single tag requests" (RtuConn:452-453). Coil: Write Multiple Coils; holding register: Write Single Register for exactly 2 bytes else Write Multiple; extended register: Write File Record; other tag types throw "Unsupported write tag type" (modbus_rtu.write_pdu, RtuConn:503-556). Unit id from `getUnitId` (RtuConn:461-467). | Same PDU selection as MB-TCP-WRITE (FC15 for coils, FC06 or FC16 for holding registers). A request with more than one tag is refused with a typed error before any byte is sent; discrete inputs and input registers are refused. | planned (M2) | `planned:rtu_write_of_one_register_uses_function_6`, `planned:rtu_write_of_two_registers_uses_function_16`, `planned:serial_write_of_two_tags_is_refused_before_sending`, `planned:serial_write_to_a_discrete_input_is_refused` |
| MBS-OP-WRITE-EXT | Extended register write is Write File Record (FC21) in both drivers (RtuConn:503-556). | Extended registers are rejected at parse time, as in MB-ADDR-EXTENDED and MB-LIMIT-EXT; the serial drivers inherit that. | deferred (follows MB-ADDR-EXTENDED) | none |
| MBS-OP-PING | `canPing` true; `onPing` reads `ping-address` (default `4x00001:BOOL`) with the unit id resolved for that tag and returns OK on any response, including an exception (RtuConn:294-306; modbus_rtu.operations.ping). ASCII identical. | `Connection::ping` reads `ping-address` through the serial session and counts any reply, an exception PDU included, as alive; bounded by `request-timeout-ms`. Same rule as MB-TCP-PING. | planned (M2) | `planned:rtu_ping_accepts_an_exception_reply`, `planned:ascii_ping_times_out_when_the_line_is_silent` |
| MBS-OP-SUBSCRIBE-POLL | Polling emulation: the connection extends `PollingSubscriptionConnectionBase`; CYCLIC uses the tag duration, CHANGE_OF_STATE polls at 1000 ms (RtuConn:63; Poll:385-387). | `SubscriptionMode::Unsupported` for both drivers, as for MB-TCP-SUBSCRIBE (D4). | deferred (follows MB-TCP-SUBSCRIBE) | none |
| MBS-OP-SUBSCRIBE-EVENT | EVENT subscriptions throw `UnsupportedOperationException` (Poll:336-343). | Not offered. | deferred (upstream refuses the kind) | none |
| MBS-OP-BROWSE | No `canBrowse`, no `onBrowse` (modbus_rtu.operations.browse). | `Capabilities.browse = false`. | deferred (no upstream operation) | none |
| MBS-OP-DISCOVER | No discoverer for RTU or ASCII: `ModbusPlcDiscoverer` lives under `tcp/discovery` and `canDiscover` is not overridden (modbus_rtu.operations.discover). | `Capabilities.discover = false`. | deferred (no upstream operation) | none |

## Capabilities
Two upstream signals, as for TCP (MB-TCP-CAPABILITIES): driver flags and connection
metadata derived from the overridden hooks (DB:117-139; CB:269-312).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-CAP-FLAGS | `ModbusRtuDriver` overrides `canPing`, `canRead` and `canWrite` to true; no `canSubscribe`, `canBrowse` or `canDiscover` (RtuDrv:99-112). The ASCII driver is the same. | `Capabilities` reports read, write and ping true, subscribe, browse and discover false, for both drivers and their connections. | planned (M2) | `planned:rtu_capabilities_report_read_write_ping_only`, `planned:ascii_capabilities_report_read_write_ping_only` |
| MBS-CAP-SUBSCRIBE-SIGNAL | Connection metadata derives subscribe from the inherited `onSubscribe` hook although the driver flag is unset (modbus_rtu.operations.subscribe; INFERENCE, as MB-TCP-CAPABILITIES). | Stonecrab reports one signal, subscribe false. | deferred (follows MB-TCP-CAPABILITIES) | none |

## Configuration
Driver options (`RtuCfg:29-67`, `AscCfg:29-67`, identical for both): six, the same as
`ModbusTcpConfiguration`. Serial options come from the shared `SerialTransportConfiguration`
(`SerCfg:28-123`); `RtuSer` and `AscSer` add nothing (`RtuSer:19-25`). Transport options
are read under the transport code as a prefix: the serial options in a connection string are
`serial.baud-rate`, `serial.data-bits` and so on (DB:197-200; the upstream manual tests write
`serial.baud-rate=9600`, `ManualModbusRTUDriverTest.java:73`). The option names in the rows
below are written without the prefix. The driver options are unprefixed.

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-CFG-REQUEST-TIMEOUT | `request-timeout-ms`, int, 5000; covers queueing; a request with less than min(50 ms, timeout/4) left fails fast (modbus_rtu.notes). | Same key and default as MB-CFG-REQUEST-TIMEOUT; the deadline includes the wait for the line; a request that arrives with too little budget left fails fast with a typed timeout. | planned (M2) | `planned:serial_request_with_no_budget_left_fails_fast`, `planned:serial_request_timeout_default_is_5000` |
| MBS-CFG-UNIT-ID | `default-unit-identifier`, int, 1; the per-tag `unit-id` option overrides it (modbus_rtu.notes; Tag:222-230). The ADU address is one byte. | Same as MB-CFG-UNIT-ID, bounded 0..=255; the address-block key `unit-id` overrides. | planned (M2) | `planned:serial_default_unit_id_is_1`, `planned:serial_tag_unit_id_overrides_the_default` |
| MBS-CFG-PING-ADDRESS | `ping-address`, String, `4x00001:BOOL` (modbus_rtu.notes). | Same as MB-CFG-PING-ADDRESS. | planned (M2) | `planned:serial_ping_address_default_is_4x00001_bool` |
| MBS-CFG-BYTE-ORDER | `default-payload-byte-order`, `BIG_ENDIAN` with the three other modes (modbus_rtu.notes). | Same as MB-CFG-BYTE-ORDER and the MB-BYTEORDER-* rows. | planned (M2) | `planned:serial_default_byte_order_is_big_endian` |
| MBS-CFG-MAX-COILS | `max-coils-per-request`, int, 2000 (modbus_rtu.notes). | Same as MB-CFG-MAX-COILS. The RTU ADU is shorter than the TCP one (see MBS-LIMIT-ADU), so the effective bound is the smaller of the option and what one frame carries. | planned (M2) | `planned:serial_max_coils_default_is_2000`, `planned:serial_batcher_respects_the_frame_ceiling` |
| MBS-CFG-MAX-REGISTERS | `max-registers-per-request`, int, 125 (modbus_rtu.notes). | Same as MB-CFG-MAX-REGISTERS. | planned (M2) | `planned:serial_max_registers_default_is_125` |
| MBS-CFG-TRANSPORT-OPTIONS | The key rule is `<transport-code>.<option>` for the selected transport, plus the `log.` prefix for the audit log and the unprefixed `allow-unsupported-transport` (DB:180-204; CF:66-82). A Modbus RTU or ASCII connection offers `serial.` (baud-rate, data-bits, stop-bits, parity, flow-control, read-timeout-ms, write-timeout-ms, dtr, rts, reuse-port, interframe-delay; `SerCfg:33-122`) and, for the gateway carriers, `tcp.`, `udp.`, `tls.`, `tls-psk.` and `test.` keys (`RtuDrv:59-75`, identical for ASCII). The names are recorded in `docs/parity/transport.md` (UTR-SERIAL-OPTIONS, UTR-TCP-OPTIONS, UTR-UDP-OPTIONS, UTR-TLS-TRUST, UTR-TLS-PSK-CONFIG, UTR-TEST). | The `serial.` keys are accepted with their prefix, with the defaults and rules of the rows below (S58). The `tcp.`, `udp.`, `tls.`, `tls-psk.`, `test.` and `log.` keys are rejected as unknown, as for Modbus TCP (MB-CFG-TRANSPORT-OPTIONS), until the transport slices (S61 and M3) offer them. | planned (M2) | `planned:serial_options_are_read_under_the_serial_prefix`, `planned:serial_unprefixed_baud_rate_is_an_unknown_key`, `planned:rtu_over_tcp_tcp_prefixed_option_is_rejected` |
| MBS-CFG-UNKNOWN | Unknown parameters are reported, not rejected (DB:206-209, 315-319). | Same as MB-CFG-UNKNOWN: rejected with `InvalidConfig` (a recorded divergence). | planned (M2) | `planned:serial_unknown_option_is_rejected_with_invalid_config` |
| MBS-CFG-SERIAL-BAUD | `baud-rate`, int, 9600 (SerCfg:28-123). | `baud-rate`, positive integer, default 9600. | planned (M2) | `planned:serial_baud_rate_default_is_9600`, `planned:serial_baud_rate_zero_is_invalid_config` |
| MBS-CFG-SERIAL-DATA-BITS | `data-bits`, int, 8; the Java transport does not validate it and hands it to jSerialComm (`SerI:125-126`). The ASCII driver keeps 8N1 although real ASCII lines often use 7 data bits and even parity (modbus_ascii.notes). | `data-bits` with default 8 for both drivers; 7 is accepted so an ASCII line can be configured. | planned (M2) | `planned:serial_data_bits_default_is_8`, `planned:ascii_accepts_7_data_bits_with_even_parity` |
| MBS-CFG-SERIAL-STOP-BITS | `stop-bits`, int, 1; not validated by the Java transport (`SerI:125-126`). | `stop-bits` 1 or 2, default 1 (the bound is a Stonecrab rule, not upstream behavior). | planned (M2) | `planned:serial_stop_bits_default_is_1` |
| MBS-CFG-SERIAL-PARITY | `parity`: none, odd, even, mark, space; default none. The value is lower-cased and `_` is turned into `-` before matching, so `Even` works and `EVEN_PARITY` (written by the upstream manual tests) becomes `even-parity`, which matches no case and fails the connection (`SerI:260-274, 292-294`). | The same five values, default none, matched after the same normalisation (case-insensitive, `_` as `-`); an unknown name, `EVEN_PARITY` included, is `InvalidConfig`. | planned (M2) | `planned:serial_parity_default_is_none`, `planned:serial_parity_unknown_name_is_invalid_config`, `planned:serial_parity_names_are_case_insensitive` |
| MBS-CFG-SERIAL-FLOW | `flow-control`: none, rts-cts, xon-xoff; default none. Aliases `rtscts` and `xonxoff` are accepted too, with the same lower-case and `_` as `-` normalisation as parity, so `RTS_CTS` and `XON_XOFF` work (`SerI:276-294`). | The same three values and the two aliases, default none, matched after the same normalisation; an unknown name is `InvalidConfig`. | planned (M2) | `planned:serial_flow_control_default_is_none`, `planned:serial_flow_control_aliases_and_case_are_accepted` |
| MBS-CFG-SERIAL-TIMEOUTS | `read-timeout-ms` 1000 and `write-timeout-ms` 1000, applied as semi-blocking read timeouts (SerI:86-135). | Both keys, default 1000; a write that cannot finish within the write timeout fails the request. | planned (M2) | `planned:serial_read_and_write_timeouts_default_to_1000`, `planned:serial_write_timeout_fails_the_request` |
| MBS-CFG-SERIAL-DTR-RTS | `dtr` and `rts`, boolean, false; asserted only when true, never explicitly cleared on the dedicated path (transport_serial.notes). | Both keys, default false; Stonecrab asserts them when true and leaves the line state alone otherwise (same observable behavior). | planned (M2) | `planned:serial_dtr_and_rts_default_to_false`, `planned:serial_rts_true_is_asserted_on_open` |
| MBS-CFG-SERIAL-REUSE-PORT | `reuse-port`, boolean, false; true acquires a shared port and joining requires an identical configuration, else a hard error (SerI:86-135; Shared:33-60). | Shared ports are planned with MBS-SHARED-PORT. | planned (M2) | `planned:serial_reuse_port_default_is_false` |
| MBS-CFG-SERIAL-INTERFRAME | `interframe-delay`, ms, 0: minimum silence before a write, measured from the last write end or last read, on shared and dedicated ports (Pacer:21-32). | `interframe-delay` in milliseconds, default 0, applied before each write (MBS-RTU-SILENCE). | planned (M2) | `planned:serial_interframe_delay_default_is_0`, `planned:serial_write_waits_for_the_interframe_gap` |
| MBS-CFG-GO-CONNECT-TIMEOUT | Go only: `serial.connect-timeout-ms` is parsed and stored, nothing applies it; Java has no such option (go_transport_serial.notes). | Not offered. | deferred (Go-only option with no effect) | none |

## Connection string and transports
Default transport `serial`; supported `serial`, `tcp`, `tls`, `tls-psk`, `udp`, `test`
(`RtuDrv:77-85`; manifest `modbus_rtu.transports`, same for ASCII). The transport code
`serial` passes the whole transport URL through as the port name (`SerT:34-61`).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-TRANS-SERIAL | Serial port through jSerialComm (transport_serial.library); client only, no framing; opened with baud, data bits, stop bits, parity and flow control (SerI:86-135). | A serial transport (S58) on a blocking worker, opened from the same options; framing stays in the driver. The tests use a virtual serial pair. | planned (M2) | `planned:serial_transport_opens_a_virtual_pair`, `planned:serial_transport_applies_line_settings`, `planned:serial_transport_round_trips_bytes` |
| MBS-TRANS-PORT-NAME | The whole transport URL is the port name (SerT:34-61). Go uses only the URL path, so `serial://COM3` would lose the host part (go_transport_serial.notes; UNCERTAIN, not tested). | The port name is the URL path, or the host plus path for a Windows name; an empty name is `InvalidConfig`. The accepted forms are fixed by a test, not by Go's behavior. | planned (M2) | `planned:serial_port_name_comes_from_the_connection_string`, `planned:serial_empty_port_name_is_invalid_config` |
| MBS-TRANS-RING | A fixed 64 KiB receive ring per connection; overflow drops the OLDEST bytes, unlike the other transports (SerI:52-56, 652-658). Go has the ring on shared ports only (go_transport_serial.notes). | A bounded receive buffer of 64 KiB; on overflow the oldest bytes are dropped and the loss is counted so the framer resynchronizes. | planned (M2) | `planned:serial_receive_overflow_drops_the_oldest_bytes`, `planned:serial_overflow_is_reported_to_the_framer` |
| MBS-TRANS-TCP | `tcp` accepted as a carrier for serial-to-IP gateways; the Java RTU and ASCII drivers have no default port for it: `ModbusRtuTcpTransportConfiguration` and `ModbusAsciiTcpTransportConfiguration` are empty subclasses, so a connection string without a port fails with "No port defined" (TcpT:76-84). The generated constants are 502, 502 and 802 (`Mspec:24-28`). Go injects 502 for RTU and ASCII over tcp (go_modbus.notes). | `modbus-rtu:tcp://host:port` carries RTU frames over the Stonecrab TCP transport, default port 502 as Go does: a divergence from the Java driver, recorded as a divergence row when the slice lands. | planned (M2) | `planned:rtu_over_tcp_carries_crc_framed_adus`, `planned:ascii_over_tcp_carries_lrc_framed_adus`, `planned:rtu_over_tcp_default_port_is_502` |
| MBS-TRANS-TLS | `tls` and `tls-psk` accepted for gateways; both default to port 802, the Modbus TLS constant (`ModbusRtuTlsTransportConfiguration` and `ModbusRtuPskTlsTransportConfiguration`). | Rejected with `InvalidConfig` until the TLS transport exists, as MB-TCP-TRANSPORTS-TLS and MB-TCP-TRANSPORTS-TLS-PSK. | deferred (follows MB-TCP-TRANSPORTS-TLS) | none |
| MBS-TRANS-UDP | `udp` accepted, with no default port (`ModbusRtuUdpTransportConfiguration` is an empty subclass, UdpT:75-83 throws "No port defined"); the driver imports `org.bouncycastle.tls.UDPTransport` for its instanceof check, which looks suspect (modbus_rtu.notes, UNCERTAIN). | Rejected with `InvalidConfig`; revisited with S61 (MB-TCP-TRANSPORTS-UDP). | deferred (follows MB-TCP-TRANSPORTS-UDP) | none |
| MBS-TRANS-TEST | `test` transport declared; runtime behavior for RTU is untested upstream (modbus_rtu.notes, UNCERTAIN). | Rejected with `InvalidConfig`; Stonecrab uses its own fake transport. | deferred (follows MB-TCP-TRANSPORTS-TEST) | none |
| MBS-TRANS-GO-CARRIERS | Go registers `serial` and `tcp` for RTU and ASCII and `tcp` for TCP (go_modbus.notes). | Covered by MBS-TRANS-SERIAL and MBS-TRANS-TCP. | planned (M2) | `planned:go_carrier_set_is_a_subset_of_stonecrab_carriers` |
| MBS-TRANS-C-PY | The C driver is Modbus TCP only, over tcp (c_modbus.scope). The Python driver is Modbus TCP only (python_modbus.protocol_code); generated RTU and ASCII ADUs and CRC and LRC helpers exist but no driver (python_modbus.notes). | No obligation. | deferred (no RTU or ASCII driver in C or Python) | none |

## Address grammar
Tags and the read optimizer are shared with Modbus TCP (`ModbusTagHandler`, `ModbusTag`,
`ModbusReadOptimizer`; manifest `modbus_rtu.notes`). Whether the RTU and ASCII connections
use them unchanged is recorded as UNVERIFIED in `docs/parity/modbus.md`; the manifest note
says they do.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-ADDR-SHARED | One address grammar for all three Modbus drivers (modbus_rtu.notes). | The `ModbusAddress::parse` grammar and its limits are reused unchanged (MB-ADDR-* rows). | planned (M2) | `planned:rtu_connection_parses_tags_with_the_tcp_grammar`, `planned:ascii_connection_parses_tags_with_the_tcp_grammar` |
| MBS-ADDR-UNIT-ID | The `unit-id` tag option sets the one-byte ADU address (Tag:222-230). | The `{unit-id}` address block maps to the ADU address byte (MB-CFG-UNIT-ID). | planned (M2) | `planned:rtu_tag_unit_id_sets_the_adu_address` |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-LIMIT-ADU | ADU = uint8 address, PDU, uint16 CRC for RTU (DriverType MODBUS_RTU 0x02) and uint8 LRC for ASCII (MODBUS_ASCII 0x03) (Mspec:30-77). The serial maximum frame size is not stated in the manifest. | The batcher bound follows the PDU limit of MB-LIMIT-PDU; the frame ceiling is taken from the Modbus serial line specification once a copy is checked in (UNVERIFIED). | planned (M2) | `planned:rtu_adu_is_address_pdu_crc`, `planned:ascii_adu_is_address_pdu_lrc` |
| MBS-LIMIT-ASCII-BUFFER | More than 513 buffered bytes (`MODBUS_ASCII_MAX_SIZE`) without a terminator is DESYNCHRONIZED; minimum frame 9 bytes (AscCodec:39-56, 69-89). | The same two thresholds, as constants with the same values. | planned (M2) | `planned:ascii_buffer_over_513_bytes_without_crlf_desynchronizes`, `planned:ascii_frame_under_9_bytes_is_not_a_frame` |

## Data types
The 15 register types, the bit types and the byte-order modes are those of
`docs/parity/modbus.md` (MB-TYPE-* and MB-BYTEORDER-*). The serial drivers add none and
drop none (manifest `modbus_rtu.notes`: tags and optimizer shared).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-TYPE-SHARED | Decode and encode through the shared `ModbusRegisterCodec` (modbus_rtu.notes). | The typed decoder and encoder of S23 serve all three drivers; one RTU and one ASCII read per type family prove the wiring, the per-type cases stay under MB-TYPE-*. | planned (M2) | `planned:rtu_read_decodes_a_real_through_the_shared_decoder`, `planned:ascii_read_decodes_a_dint_through_the_shared_decoder` |
| MBS-TYPE-STRING-TIME | Strings and the TIME and DATE family are deferred for TCP (MB-TYPE-STRING, MB-TYPE-TIME). The shared `DataItem` has no TIME or DATE case, so for RTU and ASCII too a read of the family yields OK with a null value and a write throws (see the Data types section of `docs/parity/modbus.md`). | Follow the TCP decision. | deferred (follows MB-TYPE-STRING) | none |

## RTU framing
Frames are `address, PDU, CRC-16` on the wire. The codec frames responses only
(`RtuCodec:33-190`).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-RTU-SIZE-TABLE | Response size from the function code (`RtuCodec:86-102`): exception (fc >= 0x80) 5 bytes; fc 01, 02, 03, 04, 14, 15, 17 5 plus the third byte (byte count); fc 05, 06, 0F, 10 8 bytes; unknown fc is desynchronised. | The same table decides how many bytes a response needs. Function codes beyond the planned set (14, 15, 17 for file records and read-write registers) stay in the table so an unexpected reply is framed rather than misread. | planned (M2) | `planned:rtu_exception_response_is_5_bytes`, `planned:rtu_read_response_size_is_5_plus_byte_count`, `planned:rtu_write_echo_is_8_bytes`, `planned:rtu_unknown_function_code_desynchronizes` |
| MBS-RTU-CRC | CRC checked by the generated parser (`rtuCrcCheck` in `Mspec`); the helper was not opened (modbus_ascii.notes, UNCERTAIN). | CRC-16/MODBUS (polynomial 0xA001 reflected, init 0xFFFF, low byte first on the wire), checked on receive and appended on send. The vectors come from the upstream suites and from the Modbus serial specification. | planned (M2) | `planned:crc16_of_the_spec_example_frame_matches`, `planned:rtu_frame_with_a_bad_crc_is_rejected`, `planned:rtu_request_carries_crc_low_byte_first` |
| MBS-RTU-RESYNC | `processIncomingData` peeks a candidate frame, consumes it only after the parser validates the CRC, otherwise skips one byte and retries (`RtuCodec:136-173`). | The same byte-wise resynchronization, with a property test that random noise before a valid frame never loses the frame. | planned (M2) | `planned:rtu_noise_before_a_frame_is_skipped_byte_by_byte`, `planned:rtu_resync_finds_a_frame_after_a_truncated_one`, `planned:rtu_framer_never_panics_on_random_bytes` |
| MBS-RTU-SILENCE | No inter-frame (3.5 character) silence detection in the codec; `interframe-delay` only spaces writes (`RtuCodec`, modbus_rtu.notes; Pacer:21-32). | Same: no silence-based framing on receive; the pacer spaces writes (MBS-CFG-SERIAL-INTERFRAME). See MBS-QUIRK-RTU-NO-SILENCE. | planned (M2) | `planned:rtu_receive_does_not_use_silence_to_frame`, `planned:rtu_write_gap_is_measured_from_the_last_read` |
| MBS-RTU-REQUEST-STREAM | The codec frames responses, not request streams (modbus_rtu.notes: documented limitation). | Not applicable: Stonecrab is a client only. | deferred (client only) | none |
| MBS-RTU-WIRE-MODEL | `ModbusRtuADU` = uint8 address, PDU, uint16 CRC (Mspec:30-77). | The ADU type in the codec mirrors it; PDUs are those of `docs/parity/modbus.md`. | planned (M2) | `planned:rtu_adu_round_trips_the_upstream_vectors` |

## ASCII framing
Frames are `':'`, upper-case hex of `address, PDU, LRC`, then `CR LF` (`AscCodec:35-163`).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-ASCII-DELIMITERS | `calculateTotalMessageSize` scans for CR LF and returns index + 2 (`AscCodec:69-89`). | The framer completes a frame at the first CR LF. | planned (M2) | `planned:ascii_frame_ends_at_the_first_crlf`, `planned:ascii_partial_frame_asks_for_more_data` |
| MBS-ASCII-SEND | `send()` hex-encodes the serialized ADU in upper case and writes US-ASCII (`AscCodec:91-122`). | The request is encoded as `:` plus upper-case hex plus CR LF. | planned (M2) | `planned:ascii_request_is_upper_case_hex_with_colon_and_crlf` |
| MBS-ASCII-PARSE | `parseMessage` checks the leading `:` and an even hex length of at least 6, decodes the hex and parses with `DriverType.MODBUS_ASCII` (`AscCodec:124-161`). | The same checks; each failure is a typed framing error, not a panic. | planned (M2) | `planned:ascii_frame_without_a_colon_is_rejected`, `planned:ascii_odd_hex_length_is_rejected`, `planned:ascii_non_hex_digit_is_rejected` |
| MBS-ASCII-LRC | LRC checked by the generated parser (`asciiLrcCheck` in `Mspec`); the helper was not opened (UNCERTAIN). | LRC is the two's complement of the 8-bit sum of address and PDU bytes. The vectors come from the upstream ASCII suites. | planned (M2) | `planned:lrc_of_the_spec_example_frame_matches`, `planned:ascii_frame_with_a_bad_lrc_is_rejected` |
| MBS-ASCII-RESYNC | Java reports DESYNCHRONIZED past 513 bytes and does not describe a resync path (`AscCodec:39-56`). Go resynchronizes on the next colon (go_modbus.notes). | After a desynchronization the framer discards up to the next `:` and continues, as Go does. | planned (M2) | `planned:ascii_desync_resumes_at_the_next_colon`, `planned:ascii_framer_never_panics_on_random_bytes` |
| MBS-ASCII-TIMING | No inter-character timeout or inter-frame timing logic (modbus_ascii.notes). | Same: no character timeout; the request deadline bounds the wait. | planned (M2) | `planned:ascii_slow_bytes_are_accepted_until_the_request_deadline` |
| MBS-ASCII-WIRE-MODEL | `ModbusAsciiADU` = uint8 address, PDU, uint8 LRC (Mspec:67-75). | The ADU type mirrors it. | planned (M2) | `planned:ascii_adu_round_trips_the_upstream_vectors` |

## Correlation and bus serialization
RTU and ASCII have no transaction id.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-BUS-ONE-IN-FLIGHT | Max concurrent requests 1 (`RtuConn:145-148`); requests are serialized per connection by a request-chain lock (`RtuConn:201-255`). | One request in flight per connection, through the transport guard of `docs/parity/transport.md` (TRANSPORT-QUEUE). | planned (M2) | `planned:serial_second_request_waits_for_the_first`, `planned:serial_queue_depth_is_bounded` |
| MBS-BUS-CORRELATION | Pending requests are keyed by unit id; a response is matched by address plus a function-code check (`RtuConn:163-198`). | A response is accepted only if its address and function code (or the exception form of it) match the one pending request. | planned (M2) | `planned:serial_response_from_another_unit_is_ignored`, `planned:serial_response_with_another_function_is_ignored` |
| MBS-BUS-EXCEPTION | Exception responses answer any pending request. | An exception reply from the right address answers the pending request and maps through MB-EXC-* . | planned (M2) | `planned:rtu_exception_reply_answers_the_pending_request` |
| MBS-BUS-LATE-REPLY | A late same-function-code response cannot be told apart because there is no transaction id (modbus_rtu.notes). | Not reproduced as a defect: after a timeout the line is drained for a quiet period before the next request, so a late reply cannot answer it. | planned (M2) | `planned:serial_late_reply_after_timeout_does_not_answer_the_next_request` |
| MBS-BUS-SHARED-PORT | `reuse-port` shares one port between connections; they need distinct unit ids and an identical configuration, otherwise a hard error (modbus_rtu.notes; Shared:33-60). | Shared ports with a per-port request chain; joining with a different configuration is `InvalidConfig`. Go's pacer does not serialize concurrent writers either (go_transport_serial.notes); Stonecrab serializes at the port. | planned (M2) | `planned:shared_port_rejects_a_different_configuration`, `planned:shared_port_serializes_requests_of_two_connections`, `planned:shared_port_with_equal_unit_ids_is_refused` |
| MBS-BUS-GO-NO-SERIAL-LOCK | Go has no per-connection serialization for serial lines (go_modbus.notes, LANG GAP; UNCERTAIN, search incomplete). | Stonecrab follows Java (MBS-BUS-ONE-IN-FLIGHT). | deferred (Go lacks the behavior; Java is the baseline) | none |

## Hardware evidence
S58 to S60 use a socat pty pair with libmodbus and modbus-tk (`docs/POST_ALPHA_PLAN.md`
test hardware). That is the virtual-framing half only; the electrical layer is HW2.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-HW-RS485 | `transport_serial.notes`: needs the native serial library at run time and is not usable without hardware or a pty. | RTU and ASCII on a real RS-485 adapter against the SLM1-C: a hardware record with device model and firmware (runbook `docs/hardware/hw2-slm1-c.md`). | planned (M2) | `planned:hw2_rtu_read_against_the_slm1_c`, `planned:hw2_ascii_read_against_the_slm1_c` |

### Documented device map: SOLO SLM1-C (S52d)
The SLM1-C is the HW2 device: a Modbus RTU and ASCII server on RS-485. Its map is a hand
transcription of the vendor's manual (SOLO Modular Temperature Controller User Manual, 1st
Edition 07/2023, `https://cdn.automationdirect.com/static/manuals/soloslm/soloslmman.pdf`,
sha256 `830b99162dfbab23ed87c1778428747e65ce658dbce48e76899d4b948295d7cc`, retrieved
2026-10-28; vendor documentation, all rights reserved, pinned and never copied) in
`fixtures/modbus/devices/slm1-c.toml`: 58 register entries and five bit registers, with
the manual's number, access, type, default and meaning for each. The profile is
`profiles/documented/modbus-rtu/solo_slm1_c.toml`, scheme `modbus-rtu` (planned for S59).
Both carry the label **documented map, not hardware-verified**: no device was read. Stonecrab
has no RTU framing until S59, so the suite (`crates/stonecrab/tests/modbus_device_replays.rs`,
module `slm1_c`) is a PDU-level replay over TCP framing through the first-party responder with
the manual's function table and limits enforced: it proves the register map, the limits and the
profile, not the serial line. The runbook `docs/hardware/hw2-slm1-c.md` lists the steps and every
item the manual leaves open (exception codes, oversize requests, extension-module addressing).

| Row id | What the document says | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-DEVICE-SLM1C-MAP | The data and bit register tables of chapter 7 (pages 7-2 to 7-6) with the hexadecimal address, default, access and meaning of each, and the appendix (pages A-2 to A-4), which prints the Modbus decimal beside the hex: 1040H is 44161 | The transcription is checked against its own rules (the label, the pinned URL and sha256 equal to the entry of `PROVENANCE.toml`, the sentinel list, the defaults the HW2 steps depend on, and that each ramp/soak decimal of the appendix is 40001 plus its hex, so the hex is the wire address). Every documented register and bit register is read with the transcribed value, in requests that are exactly the runs of documented addresses cut at eight registers. The profile loads, validates for the planned serial scheme and, with its scheme changed in memory, validates against and builds tags for the TCP driver | implemented (documented map, not hardware-verified) | `the_map_is_a_consistent_transcription_of_the_pinned_manual`, `every_documented_register_and_coil_reads_back`, `the_profile_is_documented_and_validates_for_the_planned_serial_scheme` |
| MBS-DEVICE-SLM1C-LIMITS | "03: Read Holding Registers (maximum limit is read of eight registers)", "16: Write Multiple Registers (maximum limit is eight)"; data registers take 03, 06 and 16, bit registers 01, 02 and 05; 04 and 15 are not listed | With `max-registers-per-request=8` a 64-register read is eight requests of eight and a nine-register read is cut at eight; without it the replay refuses nine with exception 03 (assumed); function 16 accepts eight and refuses nine. A read of input registers (function 04) and a coil write (Stonecrab writes a coil with function 15) are refused with exception 01 (assumed) and nothing is applied, so a Stonecrab coil write cannot reach this device until function 05 is supported (recorded divergence of S45). The replay serves no function 05 | implemented (documented map, not hardware-verified) | `the_documented_request_limit_cuts_reads_and_refuses_larger_requests`, `functions_04_and_15_are_refused_because_the_manual_does_not_list_them` |
| MBS-DEVICE-SLM1C-SENTINEL | Register 1000H, "Unit: 0.1 degree": "8002H: Temperature not acquired yet, 8003H: Temperature sensor not connected, 8004H: Incorrect sensor type, 8006H: Unable to acquire temperature, ADC input error, 8007H: Unable to read/write the memory"; 8005H is not listed; the status bits 102AH say degrees C (bit 2) or F (bit 3) | Through the profile, the five error codes read as signed 16 bit integers are far outside the range and the measurement is `Uncertain` with the raw code kept, never `Good`; schema version 1 cannot match a value, so it cannot make a code `Bad`. A display that is not degrees C makes the measurement `Bad`. That a negative PV is signed is an assumption (hardware item 4) | implemented (documented map, not hardware-verified) | `a_process_variable_error_code_is_never_a_good_reading`, `a_fahrenheit_display_makes_the_process_value_bad` |
| MBS-DEVICE-SLM1C-WRITE | Input type 1004H, set point 1001H and the serial settings 1071H to 1076H are R/W; the defaults table gives ASCII, 7 data bits, even parity and 1 stop bit at 1072H, 1074H, 1075H and 1076H (values 0, 1, 1, 1); the plan reads that as "ships set to ASCII 7E1", so RTU needs 1072H (protocol) and 1074H (data bits) written | With `access=read-write`, a write to input type (to type K), set point (a negative value), protocol (to RTU) and bit length (to 8 bits) is confirmed, applied once and read back, and each is restored to its documented default | implemented (documented map, not hardware-verified) | `the_documented_write_targets_take_a_write_and_read_back` |

## Upstream quirks
Observed in the pinned source and not reproduced.

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| MBS-QUIRK-RTU-NO-SILENCE | The RTU codec has no 3.5-character silence detection, so frame boundaries come from a size table and the CRC only. | modbus_rtu.codec, `RtuCodec:136-173` | deferred (kept as upstream; revisit with HW2) | none |
| MBS-QUIRK-ASCII-8N1 | The ASCII driver keeps the 8N1 defaults although real ASCII lines often use 7E1. | modbus_ascii.notes, `SerCfg:28-123` | deferred (defaults kept; options allow 7E1) | none |
| MBS-QUIRK-COPY-PASTE | A comment in the RTU driver says "ASCII can use TCP for serial-to-IP gateways"; the ASCII driver has the same transports. | modbus_ascii.notes, `RtuDrv:82-85` | deferred (comment only) | none |
| MBS-QUIRK-UDP-IMPORT | The `UDPTransport` import from BouncyCastle looks wrong for the project's own UDP class. | modbus_rtu.notes, `RtuDrv:44-122` | deferred (follows MB-QUIRK-A) | none |
| MBS-QUIRK-DTR-RTS | DTR and RTS are asserted only when true and never cleared on the dedicated path. | transport_serial.notes, `SerI:86-135` | deferred (no observable difference planned) | none |
| MBS-QUIRK-STALE-SERIAL-OPTIONS | The upstream manual RTU and ASCII tests (`ManualModbusRTUDriverTest.java:73`, `ManualModbusRTUSubscriptionTest.java:44`, and the ASCII pair) pass `serial.num-data-bits`, an unprefixed `num-stop-bits` and `serial.parity=EVEN_PARITY`. The declared names are `data-bits` and `stop-bits`, so the first two would only be warned about as unknown, and the parity value matches no case and fails the connection. | `SerialTransportConfiguration.java:41-52`; `SerI:260-274` | planned (M2); not reproduced | `planned:serial_stale_option_names_are_rejected` |
| MBS-QUIRK-GO-CONNECT-TIMEOUT | Go parses `serial.connect-timeout-ms` and ignores it. | go_transport_serial.notes | deferred (see MBS-CFG-GO-CONNECT-TIMEOUT) | none |

## Divergences from upstream
None planned yet. Candidates are recorded in the rows above (MBS-BUS-LATE-REPLY,
MBS-TRANS-PORT-NAME, MBS-TRANS-TCP for the default port, MBS-ASCII-RESYNC) and become divergence rows with an upstream issue
reference when the slice lands.

## Backlog from other languages
`[[backlog]]` rows naming Modbus: one, for `go_modbus` (extended registers spanning more
than two files). The Go, C and Python `LANG GAP` notes that apply to the serial drivers are
covered in the rows above (MBS-TRANS-GO-CARRIERS, MBS-TRANS-C-PY, MBS-BUS-GO-NO-SERIAL-LOCK,
MBS-CFG-GO-CONNECT-TIMEOUT).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| MBS-BACKLOG-GO-EXT-SPLIT | `[[backlog]]` `go_modbus`, "Extended-register requests that span more than two files": Java reads only the first item of the response (`ModbusTcpConnection.java:517-518`); Go splits until the request is covered and concatenates all items (evidence `extended_register_split`). | Extended registers are rejected at parse time (MB-ADDR-EXTENDED), so there is nothing to split. | deferred (extended registers; follows MB-QUIRK-B) | none |
| MBS-BACKLOG-GO-PING | `go_modbus.ping` (`GoMb`, `Connection.go:134-211`): ping reads the ping address, an exception counts as alive; matches Java. | Covered by MBS-OP-PING. | planned (M2) | `planned:rtu_ping_accepts_an_exception_reply` |

## UNVERIFIED
- The Modbus serial line specification: the maximum RTU frame size (MBS-LIMIT-ADU), the
  3.5-character timing and the CRC and LRC definitions are stated from the specification,
  not from the manifest. A copy is not in the repository. The S44 attempt to download the
  Modbus Application Protocol V1.1b3 PDF from www.modbus.org was blocked (HTTP 403), so no
  revision has been checked and no sha256 exists; the section numbers cited for the PDU
  (in `docs/parity/modbus.md`) stay UNVERIFIED for the same reason.
- The generated `rtuCrcCheck` and `asciiLrcCheck` helpers were not opened upstream.
- Whether the Go driver's serial transport applies the same value normalisation as the Java
  one (not read in S44).
- Whether the `test` transport and `udp` work for RTU at runtime.
- The exact serial connection-string form for a port name. The transport passes the URL
  through as the port name (`SerT:34-61`); the manifest cites no example.
- Which versions of the line-state behavior (DTR and RTS) the platform serial layers apply
  on open; the Go platform files were listed, not read.
