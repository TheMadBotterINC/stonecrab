# ctrlX Data Layer parity record
Baseline: `upstream/manifest.toml`, Apache PLC4X v1.0.0 (tag `v1.0.0`),
commit `ea229918f5d07700e04d7d69ab1645da679cb700`.

State (slice S43): nothing is implemented. The Bosch Rexroth ctrlX Data Layer driver is a
REST/HTTPS client; its only working operation upstream is ping, and browse exists in code
but the connection reports it unsupported. Ping is planned for milestone M5 (docs/
POST_ALPHA_PLAN.md section 4, "the rest"); every placeholder is deferred under the SPEC.md
rule "Unimplemented upstream placeholders do not become obligations".

Status vocabulary, as used in the Status column:

- `planned (Mn)`: Stonecrab will port the behavior in milestone Mn. The Test ids column
  names at least one planned test, prefixed `planned:`.
- `deferred (<reason>)`: not ported now. Test ids may be `none`.

Later statuses follow docs/SPEC.md "Compatibility baseline"; device models and firmware
versions of ctrlX CORE units are recorded with hardware-verified test ids, none of which
exist.

## File key
Citations use the manifest keys of `[inventory.ctrlx]`. `CX` abbreviates
`plc4j/drivers/ctrlx/src/main/java/org/apache/plc4x/java/ctrlx/readwrite`.

| Key | Path |
| --- | --- |
| driver | `CX/CtrlXDriver.java` |
| metadata, ping, browse | `CX/connection/CtrlXConnection.java` |
| discover | `CX/discovery/CtrlXPlcDiscoverer.java` |
| Cfg | `CX/configuration/CtrlXConfiguration.java` (read from the checkout; the manifest names only the browse limits) |

## Operations

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-OP-PING | Reads `datalayer/server/settings` through `NodesApi` and completes OK; a failure completes exceptionally with PlcProtocolException. The metadata does not advertise ping (`ping`, CtrlXConnection.java:174-221). | `Connection::ping` performs the same read and reports OK or a typed error; `Capabilities.ping` is true, which upstream does not advertise. | planned (M5) | `planned:ctrlx_ping_reads_the_server_settings_node`, `planned:ctrlx_ping_failure_is_a_typed_protocol_error` |
| CTRLX-OP-CONNECT | HTTPS only; username and password authentication required, otherwise PlcConnectionException (`driver`, CtrlXDriver.java:38-112). The driver implements `PlcDriver` directly, not `DriverBase`. | Connect over TLS with credentials; refusals are typed errors. | planned (M5) | `planned:ctrlx_connect_without_credentials_is_refused`, `planned:ctrlx_connect_over_plain_http_is_refused` |
| CTRLX-OP-READ | `readRequestBuilder` returns null; metadata `isReadSupported` false (operations.read; `metadata`, CtrlXConnection.java:142-202). | None. | deferred (upstream placeholder) | none |
| CTRLX-OP-WRITE | `writeRequestBuilder` returns null (operations.write). | None. | deferred (upstream placeholder) | none |
| CTRLX-OP-SUBSCRIBE | Subscription and unsubscription builders return null (operations.subscribe). | None. | deferred (upstream placeholder) | none |
| CTRLX-OP-BROWSE | A breadth-first walk of datalayer nodes with node and depth caps exists; items are hard-coded BOOL, readable and writable true, with a TODO to read metadata (`browse`, CtrlXConnection.java:227-312, TODO at 301-302). The metadata reports `isBrowseSupported` false with a comment that it does not work yet. | None; the connection reports browse unsupported, as upstream. | deferred (upstream reports unsupported; code is incomplete) | none |
| CTRLX-OP-DISCOVER | `CtrlXPlcDiscoverer.discoverWithHandler` returns null with a TODO (`discover`, CtrlXPlcDiscoverer.java:31-40); the driver wires `discoveryRequestBuilder` to the stub. | None. | deferred (upstream placeholder) | none |

## Capabilities

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-CAP-FLAGS | Metadata: read, write, subscribe and browse all false; no ping flag (`metadata`). | Ping true, all others false. | planned (M5) | `planned:ctrlx_capabilities_report_ping_only` |

## Configuration
`browse-max-total-nodes` and `browse-max-depth` are the options the manifest names. The
others are read from `Cfg` and cited by line range.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-CFG-TLS | `tls.trust-store`, `tls.trust-store-password`, `tls.trust-store-type` (default PKCS12), `server-certificate-file`, `allow-factory-default-certificate` (default false), `ignore-common-name` (default false) (Cfg:39-79). | Trust handling with a PEM certificate file and the two boolean switches; the Java key-store options are not carried (no JVM key stores). Authentication and OAuth details are UNVERIFIED. | planned (M5) | `planned:ctrlx_config_ignore_common_name_defaults_to_false`, `planned:ctrlx_config_factory_default_certificate_is_refused_by_default` |
| CTRLX-CFG-BROWSE-LIMITS | `browse-max-total-nodes` default 1000000 and `browse-max-depth` default 64 (Cfg:90-104). | Browse is deferred, so the options are accepted and unused. | deferred (browse is an upstream placeholder) | none |

## Connection string and transports

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-TRANSPORT-HTTPS | Protocol code `ctrlx`; default and only transport `https`; any other code throws "Only 'https' transport is supported by this driver" (`driver`; `inventory.ctrlx.transports`). The connection string must match `DriverBase.URI_PATTERN`. | `ctrlx:https://host[:port]`; other transport codes refused. | planned (M5) | `planned:ctrlx_uri_with_another_transport_is_refused` |

## Address grammar

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-ADDR-NONE | No tag handler is inventoried; read, write and subscribe are absent. | None. | deferred (upstream placeholder) | none |

## Upstream quirks

| Row id | Quirk | Upstream (manifest key) | Status | Stonecrab handling and test id |
| --- | --- | --- | --- | --- |
| CTRLX-QUIRK-PING-NOT-ADVERTISED | Ping works but the metadata does not advertise it. | `ping`, `metadata` | planned (M5) | Not reproduced, see `CTRLX-CAP-FLAGS`: `planned:ctrlx_capabilities_report_ping_only` |
| CTRLX-QUIRK-READWRITE-PACKAGE | The package name `readwrite` does not mean generated mspec code; the generated-looking part is `rest/datalayer`, an OpenAPI client. | notes | deferred (not a behavior) | none |

## Divergences from upstream

| Row id | Divergence | Rationale | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-DIV-PING-CAPABILITY | `Capabilities.ping` is true. | Ping is the one operation that works, and Stonecrab advertises what exists. | planned (M5) | `planned:ctrlx_capabilities_report_ping_only` |

## Backlog from other languages
`[[backlog]]` has no row for ctrlX; there is no Go, C, C# or Python implementation.

| Row id | Upstream behavior (manifest key or path:lines) | Planned Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CTRLX-BACKLOG-NONE | No non-Java implementation in the manifest. | None. | deferred (nothing to port) | none |

## UNVERIFIED
- Authentication and OAuth details (`ApiClientFactory`, `OauthHelper`) were not read.
- The REST endpoints beyond `datalayer/server/settings` (the generated OpenAPI client is
  skipped in the manifest).
- Real ctrlX CORE behavior: no device model or firmware has been tested.
