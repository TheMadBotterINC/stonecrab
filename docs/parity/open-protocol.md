# Open Protocol parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

## Specification-only protocols
DF1 (`[inventory.df1]`, kind `spec_only`) is not a driver in any language. The wire
specification `protocols/df1/.../df1.mspec` (DF1Symbol with the 0x10 start, 0x02 message
frame, 0x06 ACK and 0x15 NAK, and DF1Command with the unprotected read request and
response) has a Java codegen `Protocol` class and a type-context test, and Go generated
model code with a parser and serializer test suite, but no `plc4go/internal/df1` driver
and no module under `plc4j/drivers`. The DF1 command types used by `ab-eth` belong to the
separate `abeth` model. Per docs/SPEC.md "Unimplemented upstream placeholders do not become
obligations", DF1 is recorded here only so that the release matrix has no hidden omission:

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| DF1-SPEC-ONLY | No driver in plc4j, plc4go, plc4c, plc4net or plc4py (`inventory.df1` notes). | None. | deferred (specification only, no upstream driver) | none |
| DF1-WIRE-MODEL | `df1.mspec:20-52` and the Go test suite `plc4go/assets/testing/protocols/df1/ParserSerializerTestsuite.xml:23-60`. | No codec now; the suite can serve as vectors if the generator (E3) covers it. | deferred (specification only, no upstream driver) | none |

## Open Protocol state
State (slice S43): nothing is implemented, and upstream implements almost nothing. The
driver is a skeleton, so every behavior row is deferred under the SPEC.md placeholder rule
(docs/POST_ALPHA_PLAN.md section 4, milestone M5 "the rest"). The rows keep the omission
visible in the release matrix.

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn (none here).
- `deferred (<reason>)`: not ported now. Test ids may be `none`.

Later statuses follow docs/SPEC.md "Compatibility baseline".

## File key
Citations use the manifest keys of `[inventory.open_protocol]`.

| Key | Path |
| --- | --- |
| driver_caps | `plc4j/drivers/open-protocol/src/main/java/org/apache/plc4x/java/openprotocol/OpenProtocolDriver.java` |
| conn | `.../openprotocol/OpenProtocolConnection.java` |
| tag | `.../openprotocol/tag/OpenProtocolTag.java` |
| spi_defaults | `plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/ConnectionBase.java` |
| port | `protocols/open-protocol/src/main/resources/protocols/openprotocol/open-protocol.mspec` |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-OP-READ | Declared (`canRead` true) with no `onRead`; the base fails "Read not supported" (`spi_defaults`, ConnectionBase.java:437-471); tag parsing also always throws (operations.read). | None. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-WRITE | Declared (`canWrite` true) with no `onWrite`; the base fails "Write not supported". | None. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-SUBSCRIBE | Not declared and not implemented. | None. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-CONNECT | The connection opens the transport and codec, and incoming messages are only trace-logged (`conn`, OpenProtocolConnection.java:35-110). | None; a connect that does nothing is not an obligation. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-BROWSE | Not supported. | None. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-DISCOVER | Not supported. | None. | deferred (upstream placeholder) | none |
| OPENPROTO-OP-PING | Not supported. | None. | deferred (upstream placeholder) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-CAP-FLAGS | `canRead` and `canWrite` true, default transport tcp (`driver_caps`, OpenProtocolDriver.java:63-89). The flags promise what the code does not deliver. | Stonecrab does not reproduce the flags. | deferred (upstream placeholder) | none |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-TRANSPORT-TCP | Protocol code `open-protocol`; default transport `tcp`, default port 4545 (`driver_caps`; `port`, mspec:24). | None. | deferred (upstream placeholder) | none |
| OPENPROTO-TRANSPORT-TEST | `test` transport. | None. | deferred (upstream placeholder) | none |

## Address grammar

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-ADDR-NONE | `OpenProtocolTag.of()` always throws PlcInvalidTagException "does not support tag addressing yet" (`tag`, OpenProtocolTag.java:28-58). | None. | deferred (upstream placeholder) | none |

## Configuration

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-CFG-NONE | No configuration option is inventoried. | None. | deferred (upstream placeholder) | none |

## Protocol-specific sections

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-WIRE-MODEL | Only the wire parser and serializer from `open-protocol.mspec` are real; the connection javadoc says so (`conn`, lines 35-44). | No codec before there is an upstream driver behavior to match. Revisit if the generator (E3) emits it. | deferred (upstream placeholder) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| OPENPROTO-QUIRK-DECLARED-NOT-IMPLEMENTED | `canRead` and `canWrite` are true while every read or write fails. | `driver_caps`, `spi_defaults` | deferred (upstream placeholder) | Not reproduced. |

## Divergences from upstream

| Row id | Divergence | Rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-DIV-NONE | None planned. | Nothing is ported. | deferred (upstream placeholder) | none |

## Backlog from other languages
`[[backlog]]` has no row for Open Protocol or DF1; no other language implements either.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| OPENPROTO-BACKLOG-NONE | No non-Java implementation in the manifest. | None. | deferred (nothing to port) | none |

## UNVERIFIED
- The full message catalog of `open-protocol.mspec`: not inventoried.
- Whether any hidden Java test exercises the connection beyond the trace logging.
