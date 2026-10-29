# C-Bus parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The Java driver (`c-bus`) is documented as a
skeleton: `canRead` is true but nothing implements it, the tag handler returns null and
inbound commands are only logged. The Go driver (`go_cbus`) is far fuller (status and CAL
reads, a narrow SAL write, locally filtered push subscribe, browse by unit info) but is
explicitly incomplete. The plan lists C-Bus under the "Non-Java backlog" milestone column
(`docs/POST_ALPHA_PLAN.md` section 4), which is not one of M2 to M5, and the Java driver
gives nothing to port. Every row is therefore `deferred`; the Go capabilities cite their
`[[backlog]]` rows. The omission is visible in the release matrix and is not hidden.
Support levels in the README and `docs/COMPATIBILITY.md` follow from these statuses.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn; its Test ids cell
  names at least one test id prefixed `planned:`. Not used here yet.
- `deferred (<reason>)`: recorded so the omission is visible; Stonecrab does not port it
  now. `deferred (upstream placeholder)` marks a Java stub or a Go TODO, which is not an
  obligation (`docs/SPEC.md`, "Compatibility baseline"); `deferred (non-Java backlog)` is
  a Go-only capability. Test ids is `none` unless a rejection test is planned.
- `implemented`, `automated-verified`, `hardware-verified`: not used yet; see
  `docs/parity/modbus.md`.

No row is `implemented`, so no row id needs to appear in code yet. Row ids are never
renumbered.

Scope: the Java `c-bus` driver and the Go `cbus` driver (CNI over TCP with SAL, CAL and
MMI handling). The wire model (`c-bus.mspec` and 13 per-application mspecs) was listed but
not opened. C#, C and Python have no C-Bus driver in the inventory.

## File key
Citations use the manifest keys of `[inventory.c_bus]` (Java, "J") and
`[inventory.go_cbus]` ("G"). Java files sit under
`plc4j/drivers/c-bus/src/main/java/org/apache/plc4x/java/cbus/` and Go files under
`plc4go/internal/cbus/`; paths are relative to the apache/plc4x repository root.

| Key | Entry and path |
| --- | --- |
| J driver | `CBusDriver.java:37-90` |
| J conn_skeleton, conn_taghandler, conn_connect | `CBusConnection.java:35-43`, `:61-65`, `:77-103` |
| J config | `configuration/CBusConfiguration.java:26-39` |
| J base_unsupported | `plc4j/spi/drivers/.../ConnectionBase.java:437-471` |
| G driver, driver_transport | `Driver.go:50-61`, `:78-92` |
| G discovery_default | `plc4go/spi/default/DefaultDriver.go:99-101` |
| G config, config_defaults | `Configuration.go:35-49`, `:92-105` |
| G connect, conn_meta, sal_handler, option_todos | `Connection.go:226-264`, `:166-214`, `:273-311`, `:456-490` |
| G go_conn | `Connection.go:175-210` |
| G read_limit, read_seq, read_wire | `Reader.go:73-76`, `:109-118`, `:146-243` |
| G write_limit, write_flow | `Writer.go:69-77`, `:102-161` |
| G mapper_status_cal, mapper_sal, mapper_default | `CBusMessageMapper.go:38-156`, `:157-285`, `:300-302` |
| G tag_types, tag_cal, tag_patterns | `TagHandler.go:36-53`, `:164-279`, `:68-76` |
| G sub_subscribe, sub_unsub, sub_mmi, sub_sal | `Subscriber.go:66-110`, `:112-123`, `:164-200`, `:306-340` |
| G browse, browse_units | `Browser.go:62-173`, `:175-194` |

## Operations
Java is the baseline; it implements none of them.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-OP-CONNECT | Java: `onConnect` creates `CBusMessageCodec` with all-false `CBusOptions` and starts the receive loop; `handleIncomingMessage` only logs a received `CBusCommand` at trace level (J conn_connect:77-103). The class is a skeleton (J conn_skeleton:35-43). Go: reset (retry once), application filter, interface options 3, interface 1 power-up, interface options 1, then start the SAL and MMI handlers (G connect:226-264); backlog row `go_cbus` "connection handshake". | None from Java. The Go handshake is not ported now. | deferred (non-Java backlog) | none |
| CBUS-OP-CLOSE | Not cited in the manifest. | None. | deferred (upstream placeholder) | none |
| CBUS-OP-READ | Java: declared but not working. `CBusDriver.canRead` returns true, but `CBusConnection` has no `onRead`, so the base fails with "Read not supported" (J driver:37-90; base_unsupported:437-471). Go: status requests, CAL recall, identify and getStatus, one transaction per tag, sequential, at most 20 tags per request; a confirm-only reply gives NOT_FOUND; SAL tags are not readable (G read_wire:146-243; read_seq:109-118; read_limit:73-76); backlog row `go_cbus` "read". | `Capabilities.read` is false. Reads are not ported now. | deferred (non-Java backlog) | none |
| CBUS-OP-WRITE | Java: unsupported (`canWrite` false, no `onWrite`). Go: lighting-family SAL commands and temperature broadcast, at most 20 tags (G mapper_sal:157-285; write_limit:69-77); backlog row `go_cbus` "write". | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-OP-SUBSCRIBE | Java: unsupported (`canSubscribe` false, no `onSubscribe`). Go: push with local filtering; `Subscribe` sends nothing on the wire and marks every request OK (G sub_subscribe:66-110); backlog row `go_cbus` "subscribe". | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-OP-UNSUBSCRIBE | Java: none. Go: `Unsubscribe` returns the error "Not Implemented" (G sub_unsub:112-123). | None. | deferred (upstream placeholder) | none |
| CBUS-OP-BROWSE | Java: unsupported. Go: `info/{unit\|*}/{attribute\|*}`, one CAL identify read per (unit, attribute) at a 5 s timeout; a wildcard unit first determines the installed units (G browse:62-173; browse_units:175-194); backlog row `go_cbus` "browse". | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-OP-DISCOVER | Java: unsupported (`canDiscover` false). Go: the driver does not override `SupportsDiscovery`; the default is false (G discovery_default). | `Capabilities.discover` is false. | deferred (none upstream) | none |
| CBUS-OP-PING | Java: unsupported (`canPing` false, no `onPing`). Go: no wire ping; the default checks the local connected flag only. | `Capabilities.ping` is false. | deferred (none upstream) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-CAP-JAVA-READ-FLAG | Java sets `canRead` true and nothing else; the flag is a lie, because `onRead` is absent (J driver:37-90; `c_bus.operations.read`). | Capabilities report what works: none. | deferred (upstream placeholder) | none |
| CBUS-CAP-GO-FLAGS | Go metadata: reads, writes, subscribes, browses true, with builders for all four; discovery false (G conn_meta:166-214; go_conn:175-210). | Not ported now. | deferred (non-Java backlog) | none |

## Configuration
Java has one key. Go has twelve. Go option names are the Go field names, title-cased
before lookup (`Srchk`, `Exstat`, ...), so connection strings use the lower-case names;
values parse as Go booleans and as unsigned bytes (`ParseUint` base 0, so `0xFF` is
accepted). Unknown Go options are reported.

| Row id | Upstream name, type, default (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-CFG-SRCHK | Java `srchk`, boolean, default false, "Source check" (J config:26-39). Go `srchk`, default true (G config_defaults:92-105): the two defaults differ. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-EXSTAT | Go `exstat`, bool, default true (G config:35-49). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-PUN | Go `pun`, bool, default false. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-LOCALSAL | Go `localsal`, bool, default true; a TODO says it is not in the cbus options (G option_todos:456-490). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-PCN | Go `pcn`, bool, default false. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-IDMON | Go `idmon`, bool, default true. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-MONITOR | Go `monitor`, bool, default true. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-SMART | Go `smart`, bool, default true. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-XONXOFF | Go `xonxoff`, bool, default false. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-CONNECT | Go `connect`, bool, default true. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-MONITORED-APPS | Go `monitoredapplication1` and `monitoredapplication2`, byte, default 0xFF (G config_defaults). Backlog row `go_cbus` "configuration": "Twelve interface options plus monitored application filters." | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-CFG-GO-MONALL | Go TODO about a `monall` option that does not exist (G option_todos:456-490). | None. | deferred (upstream placeholder) | none |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-TRANSPORT-TCP | Protocol code `c-bus`; default transport `tcp`; supported `tcp` and `test` (J driver:37-90). The default port is `Constants.CBUSTCPDEFAULTPORT`, whose value was not read. Go: default `tcp`, port from the same constant; any scheme in the transports map is accepted (G driver:50-61; driver_transport:78-92). | Scheme `c-bus`, transport `tcp`. | deferred (non-Java backlog) | none |
| CBUS-TRANSPORT-TEST | Java accepts the `test` transport (J driver). | Rejected; Stonecrab tests use `FakeTransport`. | deferred (no release) | none |
| CBUS-TRANSPORT-GO-OTHER | Go does not restrict transports in the driver (for example serial); only `tcp` is exercised by the default (G driver_transport). | Other transports are `InvalidConfig`. | deferred (non-Java backlog) | none |

## Address grammar
Java has none: `getTagHandler` returns null, so no tag can be parsed (J conn_taghandler:61-65).
Go has five tag families and one query form (G tag_patterns:68-76; tag_types:36-53).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-ADDR-JAVA-NULL-HANDLER | Java tag handler is null (J conn_taghandler:61-65). | None. | deferred (upstream placeholder) | none |
| CBUS-ADDR-GO-STATUS | Go `status/[b<bridge>(-b<bridge>){0,5}/](binary\|level=0x<start>)/<application>`; level start one of `00 20 40 60 80 A0 C0 E0` (G tag_patterns:68-76). Backlog row `go_cbus` "tag syntax". | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-CAL | Go `cal/[bridges-]<unit>/<calType>` with `calType` one of `reset`, `recall=[param, count]`, `identify=<attribute>`, `getStatus=<param>, <count>`, `write=[param, 0xNN]`, `identifyReply=`, `reply=`, `status=`, `statusExtended=` (G tag_patterns; tag_cal:164-279). Only recall, identify and getStatus are implemented. | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-CAL-UNIMPLEMENTED | Go CAL reset, write, identifyReply, reply, status and statusExtended parse to "Not implemented"; the tag types are marked TODO (G tag_types:36-53; tag_cal:164-279). | None. | deferred (upstream placeholder) | none |
| CBUS-ADDR-GO-SAL | Go `sal/[bridges/]<application>/<salCommand>` (G tag_patterns). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-SALMONITOR | Go `salmonitor/<unit\|*>/<application\|*>` (G tag_patterns). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-MMIMONITOR | Go `mmimonitor/<unit\|*>/<application\|*>` (G tag_patterns). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-UNIT-INFO | Go query `info/<unit\|*>/<attribute\|*>` for browse (G tag_patterns; browse:62-173). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-ADDR-GO-UNIT-FORMS | Unit addresses are `u?` followed by decimal (1 to 3 digits) or hex (`0x` prefix or 1 to 2 hex digits); bridge hops `b` plus the same forms (G tag_patterns:68-76). | Not ported now. | deferred (non-Java backlog) | none |

## Limits

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-LIM-GO-READ-20 | Go: at most 20 tags per read request, because confirmation alphas run g-z (G read_limit:73-76); a larger request fails with "Only 20 tags can be handled at once". | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-LIM-GO-WRITE-20 | Go: at most 20 tags per write request (G write_limit:69-77). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-LIM-GO-SERIAL | Go transaction manager is created with one worker, so all reads and writes of all connections of the driver serialize (G driver:50-61, line 53 per the notes). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-LIM-GO-BROWSE-TIMEOUT | Go browse identifies each (unit, attribute) with a 5 s timeout (G browse:62-173). | Not ported now. | deferred (non-Java backlog) | none |

## Data types and SAL commands
The Java driver has no value mapping. Go maps the following (value decoding in
`ValueHandler.go` and the codec were not read in detail).

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-DT-GO-SAL-LIGHTING | Go writes SAL commands OFF, ON, RAMP_TO_LEVEL and TERMINATE_RAMP for the lighting-family applications: lighting, ventilation, irrigation, pools/spas, heating, audio and video, HVAC actuator (G mapper_sal:157-285). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-DT-GO-SAL-TEMPERATURE | Go writes TEMPERATURE_BROADCAST BROADCAST_EVENT (G mapper_sal). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-DT-GO-SAL-OTHER | Go SAL LABEL and the other named applications return "Not yet implemented"; an application not listed at all gives "No support" (G mapper_sal). | None. | deferred (upstream placeholder) | none |
| CBUS-DT-GO-MAPPER-DEFAULT | Go other tag types (monitor tags, queries) are "Unsupported type" for write (G mapper_default:300-302). | None. | deferred (upstream placeholder) | none |
| CBUS-DT-GO-READ-REPLY | Go read maps the embedded reply of a status, CAL recall, identify or getStatus confirmation (G read_wire:146-243; mapper_status_cal:38-156). | Not ported now. | deferred (non-Java backlog) | none |

## Subscription handlers (Go only)

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-SUB-GO-SAL-HANDLER | After connect, a SAL handler offers monitored SALs to subscribed `salmonitor` tags, filtered by unit address and application (G sal_handler:273-311; sub_sal:306-340). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-SUB-GO-MMI-HANDLER | An MMI handler offers CAL replies (status, extended status) to subscribed `mmimonitor` tags with the same filter (G sub_mmi:164-200). | Not ported now. | deferred (non-Java backlog) | none |
| CBUS-SUB-GO-TYPE-IGNORED | `subscriptionType` is ignored; TODOs in `sub_mmi` and `sub_sal` (G sub_mmi; sub_sal). | None. | deferred (upstream placeholder) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key or path:lines) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| CBUS-QUIRK-JAVA-SKELETON | Java connection is a skeleton: legacy protocol logic was a placeholder (read returned an unresolved future, decode was empty, no tag handler). | J conn_skeleton:35-43 | deferred (upstream placeholder) | none |
| CBUS-QUIRK-JAVA-READ-FLAG | `canRead` true with no `onRead`. | J driver:37-90 | deferred (upstream placeholder) | none |
| CBUS-QUIRK-GO-WRITER-NO-AWAIT | Go `Writer` delivers its result at line 161 without awaiting its transactions (the Reader calls `AwaitCompletion`), so response codes are filled in asynchronously. Whether callers see empty or racy codes was inferred, not established. | G write_flow:102-161 | deferred (non-Java backlog) | none |
| CBUS-QUIRK-GO-DOUBLE-CONFIRM | Go double confirmation is a TODO (Reader.go:211). | G read_wire:146-243 | deferred (upstream placeholder) | none |
| CBUS-QUIRK-GO-BRIDGE-BROWSE | Go browse has no bridge support (TODO at Browser.go:123) and a TODO about brute force for the installed-unit scan. | G browse:62-173; browse_units:175-194 | deferred (upstream placeholder) | none |
| CBUS-QUIRK-GO-SUBSCRIBE-NO-WIRE | Go subscribe always answers OK without a wire message, even for tags that can never fire. | G sub_subscribe:66-110 | deferred (non-Java backlog) | none |
| CBUS-QUIRK-SRCHK-DEFAULT | The `srchk` default is false in Java and true in Go. | J config; G config_defaults | deferred (non-Java backlog) | none |

## Divergences from upstream
None planned. A divergence row reuses the id of the behavior row it qualifies.

| Row id | Upstream behavior (file:line at ea229918) | Stonecrab behavior | Reference | Regression test id |
| --- | --- | --- | --- | --- |

## Backlog from other languages
Every `[[backlog]]` row with `driver = "go_cbus"`. Java gives nothing to port in any of
them.

| Row id | Backlog capability and Go behavior (manifest) | Where recorded | Status | Test ids |
| --- | --- | --- | --- | --- |
| CBUS-BACKLOG-READ | "read": status, CAL recall, identify and getStatus reads with reply decoding (evidence `read_wire`). Java: declared but not working. | CBUS-OP-READ, CBUS-LIM-GO-READ-20, CBUS-DT-GO-READ-REPLY | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-WRITE | "write": lighting-family SAL commands and temperature broadcast (evidence `mapper_sal`); the Go Writer does not await its transactions. Java: unsupported. | CBUS-OP-WRITE, CBUS-DT-GO-SAL-LIGHTING, CBUS-QUIRK-GO-WRITER-NO-AWAIT | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-SUBSCRIBE | "subscribe": `salmonitor` and `mmimonitor` subscriptions with local filtering (evidence `sub_subscribe`); Go unsubscribe returns "Not Implemented". Java: unsupported, inbound commands only logged. | CBUS-OP-SUBSCRIBE, CBUS-OP-UNSUBSCRIBE, CBUS-SUB-GO-SAL-HANDLER, CBUS-SUB-GO-MMI-HANDLER | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-BROWSE | "browse": `info/{unit}/{attribute}` browse (evidence `browse`). Java: unsupported. | CBUS-OP-BROWSE, CBUS-ADDR-GO-UNIT-INFO | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-HANDSHAKE | "connection handshake": reset, application filter and interface option negotiation (evidence `connect`). Java: codec and receive loop only. | CBUS-OP-CONNECT | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-TAG-SYNTAX | "tag syntax": `status/`, `cal/`, `sal/`, `salmonitor/`, `mmimonitor/` tags and `info/` queries (evidence `tag_patterns`). Java: null tag handler. | the CBUS-ADDR-GO rows | deferred (non-Java backlog) | none |
| CBUS-BACKLOG-CONFIG | "configuration": twelve interface options plus monitored application filters (evidence `config`). Java: `srchk` only. | CBUS-CFG-SRCHK and the CBUS-CFG-GO rows | deferred (non-Java backlog) | none |

## UNVERIFIED
- `CBusMessageCodec.java`, the c-bus mspecs (`c-bus.mspec` and 13 per-application files)
  and the `CBUSTCPDEFAULTPORT` constant were not read; the default TCP port is unknown.
- Whether the Writer's early delivery makes response codes empty or racy for callers;
  the transaction manager's submit path was only partly read.
- `MessageCodec.go`, `Query.go`, `Util.go` and the Go `ValueHandler` were not read in
  detail.
- The exact set of Go transports (for example serial) depends on what plc4go registers.
- "Complete" for the Go driver is not established; the manifest shows it is explicitly
  incomplete.
