# Changelog
All notable changes to this project are documented here, in the style of Keep a
Changelog. Stonecrab is one crate, so each release has one section. `stonecrab` 0.3.0
(2026-10-29) and the alpha, 0.2.0 (2026-10-27), are published; `stonecrab` 0.1.0 is the
specification-stage placeholder.

## Unreleased

- **Added (opt-in `opcua` feature; slice S83 of milestone M3):** the new `opcua` feature, in
  neither `default` nor `full` (as `tls` is, decision E17) and enabling only `transport`, adds
  the module `stonecrab::opcua` and its first type, `OpcuaAddress`, a `ProtocolAddress` for
  the OPC UA tag text `ns=<n>;<i|s|g|b>=<id>[range][;a=<attribute>][;TYPE][|key=value,...]`
  with `OpcuaIdentifier`, `OpcuaAttribute` and `OpcuaDataType`, and `OpcuaAddress::parse_query`
  for browse queries (empty and `**` mean `ns=0;i=85`). The namespace is bounded to 16 bits,
  `b=` is a base64 `ByteString` identifier, a backslash escapes `;`, `[`, `|` and itself in a
  string identifier, an array suffix becomes an OPC UA `IndexRange`, and the key-value tail
  carries event field names in written order. Parsing opens no socket; there is no driver,
  connection string or connection yet. No dependency is added and a build without the feature
  is unchanged. Gates: coverage floor of 100 for the module, the lint promotions of the
  driver modules, `mutants.toml`, the fuzz target `fuzz_opcua_address` and a property test.
  Parity: 12 planned rows of `docs/parity/opcua.md` and the new divergence
  `OPCUA-DIV-ADDRESS-ESCAPES` are implemented.
- **Added (opt-in `opcua` feature; slice S84 of milestone M3):** the connection string of the
  `opcua` scheme and its typed options, parsed by a crate-private `OpcuaDriver::parse_config`
  (and `parse_address` over `OpcuaAddress`) that the driver of slice S91 will wrap, so the
  public API is unchanged. `opcua[:tcp]://host:port[/path]`: the port is required (decision
  E22, 4840 is not defaulted), the path is kept verbatim in the `opc.tcp://host:port<path>`
  URL that Hello will carry, and `tls`, `https` and every other transport code are refused.
  `security-policy` (`NONE`, `Basic256Sha256` by default, `Aes128_Sha256_RsaOaep`,
  `Aes256_Sha256_RsaPss`; `Basic128Rsa15` and `Basic256` refused as deprecated) and
  `message-security` (`NONE`, `SIGN`, `SIGN_ENCRYPT` by default) are case-sensitive, and a
  secure policy with mode `NONE` is `InvalidConfig`. Every other key is `InvalidConfig`: the
  keys the framework fills, `username` and `password` (credentials never travel in the
  string), the `tls.*` names (reserved for later slices) and the legacy
  `insecure-certificate-verification`. Gates: the fuzz target
  `fuzz_opcua_connection_string` (26 seeds, reached through a `cfg(fuzzing)` entry point) and
  `unexpected_cfgs` knowing `cfg(fuzzing)`. Parity: 10 planned rows of
  `docs/parity/opcua.md` are implemented.
- **Added (opt-in `opcua` feature; slice S85 of milestone M3):** the hand-written built-in
  types of the OPC UA binary encoding, crate-private under `opcua/codec` and called by nothing
  yet, so the public API is unchanged: String and ByteString (length -1 is null and is kept
  distinct from empty; a length is bounded by the remaining bytes and 16 MiB before any byte is
  copied; a String keeps its bytes, UTF-8 or not, and the value layer will decide), Guid, DateTime (every `i64` of 100 ns ticks since 1601
  round-trips, `0` and `i64::MAX` included, and the `SystemTime` conversions return `None`
  rather than overflow), StatusCode, NodeId (all six encodings read, the most compact written),
  ExpandedNodeId, QualifiedName and LocalizedText. Each has the external-type signature the
  generator calls (`parse(reader, depth)`, `serialize`, `length_in_bytes`) with the recursion
  budget `Depth`, which only the recursive types of slice S86 will spend. Gates: 65 first-party
  byte vectors under `fixtures/opcua/codec` written from Part 6 (not packaged), unit tests, a
  property test, the coverage floor of 100 and the module in `mutants.toml`. No parity row
  changes status.
- **Added (slice S86 of milestone M3; one additive public change):** `ParseError`, the error
  of `stonecrab::codec`, gains two variants on its `#[non_exhaustive]` enum:
  `CountLimit { field, count, max }` (a declared element count larger than the remaining input
  could hold, refused before anything is reserved) and `RecursionLimit { limit }` (a structure
  nested deeper than the budget). Both map to `ErrorKind::ResourceLimit` in `PlcError`, as
  `OversizedLength` does; the Modbus framing maps both to `FrameError::Malformed`, which they
  cannot reach. With the `opcua` feature the crate-private recursive types of the binary
  encoding join the built-ins, called by nothing yet: Variant (all 25 types, scalars and
  arrays, null arrays kept apart from empty ones, `arrayDimensions` checked against the number
  of values), `DataValue`, `DiagnosticInfo` and the `ExtensionObject` envelope with Raw, Xml and
  Null bodies. Each spends one unit of a 64-entry budget on entry, so a Variant nested 2000 or
  16000 deep is `RecursionLimit`, not a stack overflow, and an array count is checked against
  the bytes that remain before an element is read. Gates: the fuzz target `fuzz_opcua_decode`
  (35 seeds, reached through the `cfg(fuzzing)` entry point), unit tests with hand-written
  byte vectors, three property tests, the coverage floor of 100 and the module in
  `mutants.toml`. No parity row changes status.
- **Dependencies:** `toml` 0.9 to 1.1 (resolved 1.1.8, with `toml_parser` 1.1.5,
  `toml_datetime` 1.1.2 and `winnow` 1.0.4) for the crate, the fuzz workspace and the code
  generator. There is no API or behavior change: no `toml` type is public, and profiles load
  and report errors as before. The locks no longer pin `toml_parser` to keep one `winnow`
  (ADR-0016 addendum).
- Interop (no crate API change; slice S81 of milestone M3): the independent OPC UA peer,
  open62541 v1.5.9 built in Docker from the commit of its tag with the OpenSSL backend
  (`interop/open62541/`, the new `interop-opcua` CI job and `mise run interop-opcua`, outside
  `mise run interop`). A first-party server serves four security policies (None,
  Basic256Sha256, Aes128Sha256RsaOaep, Aes256Sha256RsaPss), anonymous, user name and X.509
  identities, a test namespace, switchable small chunk limits and a short session timeout,
  with a PKI made at run time inside the container and never stored. The image's own
  open62541 client proves None and Basic256Sha256 SignAndEncrypt reads and the refusal by an
  empty trust list, and its README records, with source citations, that the peer has no
  minimum session timeout, supports TransferSubscriptions and Republish, and runs
  Aes256Sha256RsaPss under OpenSSL. No Stonecrab OPC UA code exists yet.

## stonecrab 0.3.0 (2026-10-29)
The second pre-release, one crate (ADR-0018), released under maintainer decision E12. It
opens with the four release-note items that `docs/SPEC.md` ('Open-source project
requirements') requires; every change since 0.2.0 follows, newest first.

- **Upstream baseline:** Apache PLC4X v1.0.0 (commit
  `ea229918f5d07700e04d7d69ab1645da679cb700`, pinned in `upstream/manifest.toml`), as for
  0.2.0. Its `modbus.mspec` is now vendored under `upstream/mspec/` for the code generator;
  the generated codec is compiled only into the crate's own tests, and the hand-written
  codec is the one that runs.
- **Supported operations:** what 0.2.0 read, plus Modbus writes (FC06, FC15, FC16, FC21,
  and FC05 for one coil on request), ping, polling subscriptions, extended registers
  (FC20, FC21), text, calendar and list values with array selections, Modbus RTU and ASCII
  over a serial port or a TCP gateway, Modbus TCP over UDP and, behind the opt-in `tls`
  feature, over TLS; the connection cache, the event pump and Object PLC Mapping; stale
  tracking, an acquisition loop and profile schema version 2 for sensor profiles; and,
  behind the opt-in `boards` feature, the MAX31855 over SPI and the ADXL345 over the
  caller's I2C bus, with a Linux `/dev/spidev` backend for the MAX31855 behind
  `adapters-linux`. The level of each is in `docs/COMPATIBILITY.md`.
- **Hardware evidence:** none. No physical device has been used. The highest level of any
  row is `interoperability-tested`, against software peers: pymodbus 3.8.6 (TCP and TLS),
  libmodbus 3.1.11, diagslave 3.6 (UDP, RTU over TCP, and RTU and ASCII through
  pseudo-terminals on Linux), a first-party modbus-tk 1.1.5 server and Toxiproxy 2.13.0
  for injected faults. The documented device maps, the SOLO SLM1-C among them, are
  replayed, not run against devices.
- **Breaking changes:** the sensor `unit` is `Option<String>` (S74). The "Migration from
  0.2" entry below shows the change a caller makes and lists the behavior changes since
  0.2.0 that a caller can notice.

Gates not run for 0.3.0, so that nobody reads their absence as a pass: the 8-hour release
fuzz run and the 24-hour soak (decision E9; CI fuzzes each target for 60 seconds and the
recorded soaks ran for 10 minutes), hardware verification, differential tests against
PLC4J and a mutation-testing threshold (the weekly `scheduled` job, with its 30-minute fuzz
and mutation runs, has not run yet; the only mutation run is the alpha's). Branch coverage
is reported, not gated. `cargo semver-checks` has no lint for the one source break (see the
migration entry).

- Object PLC Mapping, the third PLC4X tool (ADR-0033, slice S72, decisions E6 and E11;
  additive, no feature, no dependency, no proc-macro crate): `PlcEntity`, `EntityField`,
  `FieldValue`, `EntityManager` and `ConnectedEntity` at the crate root and the declarative
  macro `stonecrab::plc_entity!`, built under every feature set. A type implements
  `PlcEntity` to list its mapped fields (`EntityField`: name, address or alias `${name}`,
  optional cache duration) and convert each by name from and to `PlcValue`; `plc_entity!`
  writes the struct and that implementation, a field followed by `=> "address"` (and
  optionally `(cache = <const Duration>)`) being mapped. `FieldValue` converts `bool`, the
  integer types (from any integer whose value fits), `f32`, `f64`, `char`, `String`,
  `Duration`, the calendar types, `Vec<T>` and `PlcValue`; a value a field cannot hold is
  `InvalidConfig` naming the field, and a field type without `FieldValue`, a non-literal
  address, a non-constant cache duration or a generic struct do not compile.
  `EntityManager::new(cache, registry, clock)` parses addresses with the connection
  string's driver, with `with_alias` and `with_request_timeout` (default 1 s, reads and
  writes); `read` fills a `Default` entity in one read, `write` writes every mapped field in
  one write and returns the `WriteResponse`, `connect` and `merge` keep the entity attached
  as a `ConnectedEntity`, whose `refresh` reads in one request only the fields whose cache
  duration has passed on the injected clock (no duration: every refresh, upstream's -1; at
  exactly the duration a field is still fresh), and `disconnect` gives it back. Every request
  takes a lease from the `ConnectionCache` and gives it back; nothing is spawned. The runtime
  proxy is not ported (`TOOL-OPM-PROXY`, deferred). Simulator-tested: 24 cases on the fake
  clock against the simulator and Modbus TCP over the fake transport, and `compile_fail`
  doctests of the macro.
- TLS transport and Modbus TCP over TLS (ADR-0032, slice S71, decision E11; additive, behind
  the new opt-in `tls` feature, which is not in `default` or `full`). `stonecrab::transport::TlsFactory`
  and `TlsSettings`: rustls 0.23 on the ring provider (with `tokio-rustls` 0.26), TLS 1.3 and
  1.2, ring's default AEAD suites. The server must present a certificate for the endpoint's
  host (a DNS name or an IP address) that chains to an authority of the caller's PEM trust
  store; there are no web PKI roots, no platform store and no way to turn verification off.
  A PEM keystore (certificate chain and unencrypted key) adds a client certificate for mutual
  TLS. One deadline covers resolution, connect and handshake. A failed certificate check is
  `Trust`, a server's certificate alert `Authentication` (under TLS 1.3 at the first receive),
  other failures `Disconnected`, an unusable PEM file `InvalidConfig` naming the option and
  never a path. `TlsSettings`'s `Debug`, and so `redacted_display`, hides the keystore path;
  no session key is logged and no password is taken. `modbus-tcp:tls://host[:port]` (default
  port 802) with `tls.trust-store` (required) and `tls.keystore`, upstream's names; the other
  `tls.` keys, `verify` and `ignore-common-name` included, stay `InvalidConfig`.
  `ModbusOptions` gains `tls: Option<TlsSettings>` under the feature, and
  `ModbusTcpDriver::connect` builds the factory from it. `modbus-tcp:tls-psk` is refused with
  its reason (rustls has no external pre-shared keys); the RTU and ASCII `tls` carriers stay
  refused. Without the feature `modbus-tcp:tls` is refused as before. Dependencies (audited):
  `rustls`, `tokio-rustls`, `ring` (C and assembly: a C compiler is needed),
  `rustls-webpki`, `rustls-pki-types`, `untrusted`, `subtle`, `zeroize`, `getrandom` 0.2 and,
  on Windows, `windows-sys` 0.52; `deny.toml` names `rustls` as a wrapper of the `once_cell`
  ban; `ring` is also a dev-dependency, for the test certificate authorities made at run time
  (no key is committed). Interop: pymodbus 3.8.6 in its image, in a new mutual-TLS mode of
  `interop/pymodbus/server.py`, with a PKI that openssl makes in the image at run time, in the
  new `interop-tls` CI job and `mise run interop-tls`, which `mise run interop` now includes
  (3 of 3 locally and in the first hosted run, 40880582023, on 2026-10-29; pymodbus closes
  without its alert when it refuses a client, which reads as `Disconnected`). Parity rows
  `UTR-TLS`, `UTR-TLS-TRUST` and `MB-TCP-TRANSPORTS-TLS` automated-verified; `UTR-TLS-PSK`,
  `UTR-TLS-PSK-CONFIG` and `MB-TCP-TRANSPORTS-TLS-PSK` deferred with the refusal implemented.
  The `cross` job installs the aarch64 and armv7 C cross compilers, which `ring`'s build
  script needs even for `cargo check`.
- The SOLO follow-ups (slice S73, decision E11; ADR-0015 and ADR-0016 addenda of 2026-10-29).
  Both are additive and change no default.
  - Modbus: the opt-in connection parameter `single-coil-write=fc05` writes a single coil with
    function 05 where upstream always sends function 15 (`fc15` is the default). The three
    drivers take it (`modbus-tcp`, `modbus-rtu`, `modbus-ascii`); it is
    `ModbusOptions::single_coil_fc05` and `ConnectionDefaults::single_coil_fc05`. Two or more
    coils and every register write keep their function. The echo check and the write outcomes
    are those of every other write: the echo must repeat the address and the state, a
    mismatch is `Unknown`, an exception is `Rejected`. `FunctionCode::WriteSingleCoil`,
    `ModbusPdu::WriteSingleCoil` and `ModbusPdu::WriteSingleCoilResponse` are new (after the
    existing variants); `FunctionCode::from_u8(5)` is `WriteSingleCoil`, where it was
    `Unknown(5)`. `encode_request` writes function 05 and `decode_response` reads its echo;
    `decode_request` still refuses it. The test responder serves it.
  - Sensors: profile schema version 2 adds the `value_rules` key of a measurement: a list of
    `{ values = [..], quality = "bad" | "uncertain", reason = ".." }`, matched against the
    unscaled integer the driver delivered, so a device's reserved error codes can make a value
    `Bad`, which a plausibility range cannot. Version 1 documents load unchanged; the key in a
    version 1 document is `ProfileError::ValueRuleNeedsSchema2` from `validate`, and a rule
    with no value is `EmptyValueRule`. New: `ValueRule`, `SchemaVersion::V1`, `V2`
    (`CURRENT` is now 2, the newest; it was 1), `MeasurementDef::value_rules` and
    `Measurement::quality_reason`. The documented SOLO SLM1-C profile moves to version 2: its
    error codes 8002H, 8003H, 8004H, 8006H and 8007H are `Bad` with their reasons, and its
    options record `single-coil-write=fc05`. A 0.2.0 reader refuses a version 2 document with
    `IncompatibleSchema { found: 2 }`.
- Release 0.3.0: the sensor `unit` is optional (ADR-0016 addendum, slice S74, finding G06,
  decisions E7 and E12; a breaking change, so the crate version moves from 0.2.0 to
  0.3.0). `MeasurementDef::unit` and `Measurement::unit` are
  `Option<String>`. A profile may leave `unit` out: it loads as `None`, which means the
  unit is unknown, and it validates. A stated unit is checked as before, except that a blank
  one (only whitespace) is now `EmptyUnit` like the empty string, not `UnknownUnit`; the
  `EmptyUnit` message says to omit the key when the unit is unknown. The MAX31855 and ADXL345
  adapters always set `Some`. The profile schema stays at version 1: every document that
  0.2.0 loads loads the same way (a 0.2.0 reader refuses a document without a unit with
  `Syntax`, not `IncompatibleSchema`). The `profile_read` example logs an absent unit as
  nothing. Tests: an absent unit loads and gives `None`, validates, and gives a measurement
  with `unit` `None`; a blank unit is refused; the boards still produce their units.
- Migration from 0.2 (0.2.0 to 0.3.0). Every change since 0.2.0 passed `cargo semver-checks`
  0.51.0 as a minor release: in CI against the published 0.2.0 until S74 moved the version
  to 0.3.0 (a major step, which the tool does not check), and against master for S73, S71
  and S72. The tool has no lint for the type of a public struct field, so it never reported
  the one source break below; the 0.3.0 version number is what announces it. The changes
  since 0.2.0 that a caller can notice are these:
  - Source break, `unit` is `Option<String>` (S74). Code that reads `measurement.unit` or
    `definition.unit` as a `String` stops compiling. Before:
    `println!("{} {}", measurement.value, measurement.unit)` and
    `if measurement.unit == "Cel" { .. }`. After:
    `println!("{} {}", measurement.value, measurement.unit.as_deref().unwrap_or("?"))` and
    `if measurement.unit.as_deref() == Some("Cel") { .. }`. Code that builds a `MeasurementDef`
    from a struct literal is not possible outside the crate (the type is `#[non_exhaustive]`);
    a profile TOML needs no change.
  - Behavior, `SensorProfile::validate` no longer reports `MissingByteOrder` or
    `MissingWordOrder` (S39, G05). Call `SensorProfile::validate_for(&driver)` for them;
    `SensorProfile::tags` already does.
  - Behavior, `SensorProfile::validate` refuses a measurement `range` with a NaN or infinite
    bound or `min > max` (`ProfileError::InvalidRange`, S36). A profile that passed before and
    has such a range now fails.
  - Behavior, `Capabilities::subscribe` of the simulator and the Modbus drivers is
    `SubscriptionMode::Polling`, not `Unsupported` (S55). Code that tests it for `Unsupported`
    to skip `subscribe` now takes the polling path.
  - Behavior, Modbus function codes 06, 15, 16, 20 and 21 decode: `FunctionCode::from_u8`
    returns the named variant where it returned `Unknown(n)`, and the address texts
    `extended-register`, `6x` and a bare `6` prefix, and the type names `CHAR`, `WCHAR`,
    `STRING` and `WSTRING`, are accepted where they were refused. A `match` over
    `FunctionCode` that relied on `Unknown` for these codes needs the named arms (the enum is
    `#[non_exhaustive]`, so a wildcard arm keeps compiling).
  - Behavior, a Modbus read reply that decodes but does not fit its request used to fail the
    whole read; it now fails only the items of its chunk with `MalformedData` and the reply
    PDU in the item's raw bytes, and the other chunks are read (S70, E10).
  - Behavior, `SchemaVersion::CURRENT` is 2 and `FunctionCode::from_u8(5)` is
    `WriteSingleCoil` (S73). Code that compares a profile's `schema_version` with `CURRENT`
    to mean version 1 should use `SchemaVersion::V1`; a `match` over `FunctionCode` that
    relied on `Unknown(5)` needs the named arm.
  - Behavior, the Modbus drivers report `Capabilities.write` and `Capabilities.ping` as true
    (S46, S47), where 0.2.0 reported no write and had no ping. A write is still refused with
    `Unsupported(Operation::Write)` and nothing sent unless the connection string says
    `access=read-write`, so the flag does not tell whether a write will be accepted.
  - Behavior, a Modbus connection opened inside a Tokio runtime runs a supervisor task
    (S57): it reconnects without waiting for a request, so the state can move to
    `Reconnecting` with no request pending, and it enforces `Timeouts.idle`, which 0.2.0
    recorded and ignored: a link no request has used for that long is closed, and the next
    request opens a new one. `ResourceLimits::max_queued_events`, also unread in 0.2.0, now
    bounds a subscription's queue (S54).
  - Behavior, `SensorProfile::validate` refuses a status field whose `bit` is 64 or more
    (`ProfileError::StatusBitOutOfRange`), and `validate_for`, which `SensorProfile::tags`
    runs, also refuses a bit beyond the width of its status address, two spellings of one
    address and bit, and a status bit inside the measurement's `bit_field` (S37).
  - Build, the `modbus` feature enables `serial` (S59, E10), so a build with Modbus TCP only
    also compiles the `serial2` dependency and its own (`cfg-if`, `libc`, and `windows-sys`
    on Windows). A build with `default-features = false` and without `modbus` is
    unchanged.
- The MSpec code generator, proven on Modbus (ADR-0019 and its addendum of 2026-10-29,
  slices S66 and S67; no crate API change). Apache PLC4X's `modbus.mspec` is vendored verbatim,
  ASF header kept, at `upstream/mspec/modbus/modbus.mspec` (maintainer decision E8; upstream
  path, git blob and sha256 in `upstream/manifest.toml`, attribution in `NOTICE`). The new,
  unpublished tool `tools/stonecrab-codegen` (its own workspace; dependencies `toml` and
  `sha2`) parses the whole of `MSpec.g4` and `Expression.g4` by hand-written recursive
  descent, emits the subset its feature matrix lists (`tools/stonecrab-codegen/README.md`) and
  refuses anything else with the construct, the type and the mspec line. It generates
  `crate::modbus::generated`, a test-only module (the TCP ADU, the exception PDU, the reads of
  FC01 to FC04, the exception codes and the numeric `DataItem` cases; every other case refused
  by name) with an `@generated` provenance header and `generated/MANIFEST.toml`. An in-crate
  proof runs every Modbus vector, the PDU and MBAP fuzz seeds and property tests through it and
  the hand codec in both directions and finds no difference outside a closed list of checks
  only the hand codec makes; the hand codec stays the shipped one. Divergences of the
  generated codec from upstream's generated Java are parity rows `MB-GEN-EXCEPTION-FUNCTION`,
  `MB-GEN-CHECKED-COUNT`, `MB-GEN-IMPLICIT` and `MB-GEN-DISABLED`. A new blocking CI job
  `regen` (`mise run regen`, `scripts/regen.sh`) checks the generator workspace (fmt, clippy,
  tests, cargo deny), the vendored mspec hash, two byte-identical generator runs and an
  unchanged tree, with a self-test; `mise run generate` regenerates in place. The tool's
  maintainer-only `check` parses 57 of the 59 mspec files of the pinned PLC4X checkout; the two
  others (`ads.mspec`, `eip.mspec`) break the grammar and pass upstream only through ANTLR's
  error recovery.
- Modbus RTU and Modbus ASCII through the drivers' serial carrier against an independent peer
  (ADR-0024, slice S59c; no crate API change): on Linux, socat makes a pseudo-terminal pair,
  diagslave 3.6 (the pinned archive, checked against its SHA-256 before it is unpacked, its
  static binary run on the host, no container) opens one end as its serial port, and the
  drivers open the other end by path (`modbus-rtu:///<dir>/ttyB`, `modbus-ascii:///<dir>/ttyB`,
  `serial.baud-rate=19200`). The nine RTU tests (without the closed-connection one) and the
  nine ASCII tests take a full connection string from their environment variable and run
  unchanged over it; two new ignored tests in `modbus_interop_diagslave_serial` check what only a
  serial port has: the silence before a request (the second of two requests waits at least t3.5
  at 300 baud, and at least a 250 ms `serial.interframe-delay`, for both framings) and a lost
  port (diagslave and the pair ended: `Disconnected`; the pair and a new diagslave back: the port
  reopens). New host-side script `interop/diagslave/serial.sh`, task `mise run
  interop-serial-port` (Linux only, so not part of `mise run interop`) and CI job
  `interop-serial-port` (socat from the runner's package index, its version recorded in the
  evidence). Its first hosted run (GitHub Actions run 40849466167, commit f656c56,
  `ubuntu-latest`, 2026-10-29) passed in both modes, RTU 11 of 11 and ASCII 11 of 11, and was
  the first time the script ran as written: the download and SHA-256 check of the archive, its
  x86_64 binary, Ubuntu's socat and the Linux probe. Parity rows `MBS-TRANS-SERIAL`,
  `MBS-TRANS-PORT-NAME`, `MBS-CFG-SERIAL-INTERFRAME`, `MBS-RTU-SILENCE`,
  `MBS-CFG-SERIAL-BAUD` (RTU only) and `MBS-ASCII-TIMING` (the inter-frame delay only) are
  automated-verified, `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING`, `MBS-RTU-CRC`,
  `MBS-RTU-WIRE-MODEL`, `MBS-ASCII-SEND`, `MBS-ASCII-LRC` and `MBS-ASCII-WIRE-MODEL` gained
  the serial-port half of their evidence, and Modbus RTU and ASCII through a serial port of
  the driver on a pseudo-terminal pair are interoperability-tested. A Linux pseudo-terminal
  forces eight data bits and no parity, so the line settings other than the baud rate are
  accepted but not observed (the ASCII line is 19200 8N1 at the driver's end); data bits,
  stop bits, parity and flow control stay implemented, and no electrical line or device is
  involved. No libmodbus or modbus-tk RTU server was added (ADR-0024 addendum).
- Modbus ASCII, and Modbus RTU on a serial device, against an independent peer (ADR-0024,
  slice S60b; no crate API change): the diagslave 3.6 image now has two more modes, `ascii`
  (`diagslave -m ascii -a 1 -b 19200 -d 7 -s 1 -p even`) and `rtu-pty` (`diagslave -m rtu -a 1
  -b 19200 -d 8 -s 1 -p none -f 20`), each on a serial device that is one end of a
  pseudo-terminal pair socat makes inside the container; the other end is bridged to a TCP
  port (1504 and 1505), one connection at a time, so the drivers' gateway carrier
  (`modbus-ascii:tcp://host:port`, `modbus-rtu:tcp://host:port`) reaches the framing diagslave
  uses on a line, which its `-m enc` mode does not. The image adds the Debian package `socat`
  1.8.0.3-1+deb13u1 by exact version. Nine ignored tests (`modbus_interop_diagslave_ascii`)
  write and read back every area through `ModbusAsciiDriver`, a range selection, a write of
  123 and a read of 125 registers (frames of 511 characters), exception 02 for the
  file-record functions, a unit the slave does not serve (diagslave is silent: `Timeout`) and
  ping; the nine RTU tests run again against `-m rtu`, with the silence test in place of the
  closed-connection one. New CI jobs `interop-ascii` and `interop-rtu-pty` (one matrix job)
  and tasks `mise run interop-ascii` and `mise run interop-rtu-pty`, which `mise run interop`
  includes. Passed locally and in the first hosted run (40843548924), 9 of 9 each,
  on 2026-10-29. Parity rows `MBS-ASCII-SEND`, `MBS-ASCII-LRC` and `MBS-ASCII-WIRE-MODEL` are
  automated-verified, `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING` and `MBS-TRANS-TCP` for
  the ASCII half too; Modbus ASCII over a gateway is interoperability-tested, over a serial
  port of the driver still simulator-tested.
- The raw bytes of a short Modbus reply and the connection statistics (ADR-0015 addendum,
  slice S70; additive). A read reply that decodes but does not fit the request (libmodbus
  answers a read of two registers with one; a coil reply with the wrong byte count; a
  function 20 reply that does not answer every sub-request) used to fail the whole read
  with `MalformedData`. It now fails every member of its chunk with `MalformedData` and the
  reply PDU as received in the raw bytes of the item's `ProtocolStatus`; the other chunks
  are read and the next read works. A PDU that does not decode at all still fails the whole
  read, without raw bytes, and a write still proves its echo. `ModbusConnection::stats`
  returns the `SessionStats` (late and mismatched replies, and the guard's counters with the
  frames its reader dropped); it is on the concrete type, so a `Box<dyn Connection>` from
  the registry cannot reach it. `ModbusRtuDriver::open` and `ModbusAsciiDriver::open` are
  new: they do what `connect` does and return the `ModbusConnection`, so the serial
  framings can reach the counters too. The libmodbus interop test now asserts the bytes.
- Modbus RTU against an independent peer (ADR-0024, slice S59b; no crate API change): the
  diagslave 3.6 image of the UDP run now also serves Modbus RTU over TCP (`diagslave -m enc
  -p 1503 -a 1`, one unit), the stream a serial-to-IP gateway carries, which
  `modbus-rtu:tcp://host:port` speaks. `interop/diagslave/serve.sh` takes the mode
  (`udp`, the default, or `enc`) and `interop.sh` reads `INTEROP_DIAGSLAVE_MODE`, with its own
  container (`stonecrab-interop-diagslave-rtu`), port, evidence directory and test file, so
  the UDP job is unchanged. Nine ignored tests (`modbus_interop_diagslave_rtu`) write and
  read back every area through `ModbusRtuDriver`, a range selection, a write of 123 and a read
  of 125 registers (frames of 255 bytes), exception 02 for the file-record functions 20 and
  21 (the connection stays good), a unit the slave does not serve (diagslave closes the
  connection: `Disconnected`) and ping. New CI job `interop-rtu` and task `mise run
  interop-rtu`, which `mise run interop` includes (TCP works on colima; the UDP peer still
  needs a Docker that forwards UDP). Passed locally and in the first hosted run (40838776577), 9 of 9, on
  2026-10-29. Parity rows `MBS-OP-READ`, `MBS-OP-WRITE`, `MBS-OP-PING`,
  `MBS-TRANS-TCP`, `MBS-RTU-CRC` and `MBS-RTU-WIRE-MODEL` are automated-verified for the RTU
  half over a gateway; Modbus RTU over a gateway is interoperability-tested, over a serial
  port still simulator-tested. The shared interop `Peer` of the tests can now be built with
  any driver.
- Test and docs only (slice S52e): the SOLO SLM1-C replay (`modbus_device_replays`, module
  `slm1_c`) now runs through `ModbusRtuDriver` over the fake transport with RTU framing and
  CRC-16, on the line the fixture gives (`[device.serial]`: 9600 baud, 8 data bits, even
  parity, one stop bit, from the manual's factory defaults of 1073H, 1075H and 1076H and the
  RTU write to 1074H), so every documented register, the eight-register limits and the
  process variable error codes are exercised over RTU; the TCP-framed variant is gone. The
  documented profile `profiles/documented/modbus-rtu/solo_slm1_c.toml` names `modbus-rtu`
  and records that line in its options, and `SensorProfile::tags` accepts it with
  `ModbusRtuDriver` (no schema change). The HW2 runbook gives the `modbus-rtu` connection
  string of the hardware steps and a new parity row `MBS-DEVICE-SLM1C-RTU` cites the new
  test. Still a replay: operating the device needs the maintainer's approval.
- The connection cache, the first PLC4X tool (ADR-0030, slice S68; additive, no feature, no
  dependency): `ConnectionCache`, `ConnectionCacheBuilder` and `LeasedConnection` at the
  crate root, built under every feature set. `ConnectionCache::builder(registry, clock)`
  takes a `DriverRegistry` and a `Clock`; the builder sets `max_idle_time`,
  `max_lease_time`, `max_wait_time`, `ping_timeout`, `idle_ping_threshold`, `close_timeout`
  (upstream's defaults: 5 min, 1 min, 30 s, 5 s, 30 s, 5 s) and `limits`, and refuses a zero
  idle time, lease time, ping timeout or close timeout with `InvalidConfig`. `get` leases the
  connection of a connection string, opening it through the registry if needed. One lease is
  out per string at a time; other callers wait in arrival order, at most
  `max_pending_requests` per string (`Backpressure` beyond), and a wait past `max_wait_time`
  is `Timeout`. A `LeasedConnection` is a `Connection` that forwards `read`, `write`, `ping`
  and `subscribe` while it is valid. Past `max_lease_time` its calls fail with `Timeout` and
  a waiter takes the connection over. Dropping it, or `close` on it, gives the connection
  back. A connection idle past `idle_ping_threshold`, or whose last lease expired, is pinged
  before reuse (if it supports ping) and replaced if the ping fails. A `Disconnected` or
  `Timeout` through a lease has the next lease replace the connection. The next `get`
  closes connections idle past `max_idle_time`, and every close is bounded by
  `close_timeout`. `close` refuses every later `get` with `Disconnected`, and `remove`
  force-closes one string's connection even while it is leased. `len`, `is_empty`,
  `active_leases` and `connection_strings` report what the cache holds. Nothing is spawned:
  every bound is read from the injected clock when the cache or a lease is called.
  Subscriptions made through a lease are not restored on a replacement connection
  (`TOOL-CC-RESTORE`, deferred). Simulator-tested: 24 cases on the fake clock against the
  simulator and Modbus TCP over the fake transport.
- The event pump, the second PLC4X tool (ADR-0031, slice S69; additive, no feature, no
  dependency): `EventPump`, `TagBatch` and `BatchEvent` at the crate root, built under every
  feature set. `TagBatch::on_connection(name, connection, request, interval)` reads a
  `ReadRequest` on a borrowed connection, `TagBatch::on_cache(name, cache, connection_string,
  request, interval)` on a lease from a `ConnectionCache` taken for each fetch and given back
  after it, and `initial_delay` delays the first tick; a blank name, a zero interval and a
  request with a deadline are `InvalidConfig`. `EventPump::new(clock)` holds batches by name
  with `add_batch`, `remove_batch`, `start_batch`, `stop_batch`, `start_all`, `stop_all`,
  `is_started` and `close` (later adds and starts are `Disconnected`). `recv` drives every
  started batch on the injected clock with no spawned task and returns a `BatchEvent`: the
  batch, the response or the fetch's error, and the ticks skipped since its previous event.
  Each fetch is one `read`. A tick during a running fetch is skipped and the next fetch
  waits for the next tick (upstream's fixed-delay timer, read for this slice); a failed
  fetch is reported and the batch pauses 1 s, doubling to 60 s, until a fetch succeeds.
  Batches on different connections run independently, the queue holds at most one event
  per batch, and `recv` is cancel-safe (a read in flight goes on at the next call). The
  subscription trigger (an upstream placeholder), configuration files and value
  transformations are deferred. Simulator-tested: 17 cases on the fake clock against the
  simulator and Modbus TCP over the fake transport.
- Board adapters, the ADXL345 accelerometer over I2C (ADR-0029, slice S64; additive): under
  the existing `boards` feature (no new feature, no new dependency), `stonecrab::boards`
  gains the `I2cBus` trait (`write` and `write_read`, deadline-bounded, a NACK is
  `Disconnected`), `Adxl345` with `init`, `verify_identity`, `read_sample` and `read_fifo`,
  `Adxl345Config` (`address`, `range` as `AccelRange`, `rate` as `OutputRate` from 3200 Hz
  to 6.25 Hz, `fifo` as `FifoMode`, `full_resolution`), `AccelSample`, `AccelReading` and
  `SampleBlock`. `init` checks DEVID (a wrong one is `Trust` and nothing is written) and
  writes the configuration in a fixed order; there is no public register write.
  `AccelSample::from_registers` decodes the little-endian, right-justified counts exactly
  and refuses a count outside the configured resolution (`MalformedData`; at 16 g the
  device does not clip, so a rollover that fits is not detected). `read_sample` returns
  `None` unless `DATA_READY` flags a new sample, so neither the reset zeros of a device
  still turning on nor a repeated sample is returned as a reading; a reading becomes
  `acceleration_x`, `acceleration_y` and `acceleration_z` in `[g]`. A FIFO drain becomes a
  `SampleBlock` with the exact interval, the three axes per sample, a gapless sequence
  number from each `init`, the device's overrun flag (or a full FIFO, or a FIFO that the
  host's clock says filled during the drain, since every pop clears the flag) and the
  host's estimate of the samples lost. A failed or dropped read ends the stream until
  `init` runs again. No Linux I2C backend: `i2c-dev` needs an `ioctl`, so it waits for a
  dependency decision; an application supplies its own `I2cBus`. The testkit gains
  `FakeI2cBus`, `I2cStep` and `I2cTransfer` (with a `Delay` step that stalls the bus on the
  fake clock) when `boards` is on. The `boards` module is now a directory with its own
  coverage row (100 percent) and lint promotions; every existing path is unchanged.
  Bus-mock tested only, written from the datasheet (Rev. 0, pinned by sha256 in the ADR);
  the bench record is a runbook, `docs/hardware/adxl345-i2c.md`, that needs the
  maintainer's approval. No change to any industrial parity status.
- Board adapters, the MAX31855 thermocouple converter over SPI (ADR-0026, slice S63;
  additive): the new off-by-default feature `boards` (`boards = ["sensors"]`, no new
  dependency) adds `stonecrab::boards` with the `SpiBus` trait (`read(len, deadline)`, one
  chip-select cycle), `Max31855` and `ThermocoupleReading`. `ThermocoupleReading::from_raw`
  decodes the 32-bit frame with exact integers (14-bit thermocouple field at 0.25 C a count,
  12-bit internal field at 0.0625 C a count, both sign-extended; the open, short-to-ground and
  short-to-Vcc flags as `ThermocoupleFault`) and refuses a frame the chip could not send:
  all zeros or all ones is `Disconnected` (no device answered), a set reserved bit or a
  fault bit that disagrees with the flags is `MalformedData`. `measurements()` returns
  `thermocouple_temperature` (`Bad` with no value while a fault is flagged),
  `cold_junction_temperature` (`Good`) and `thermocouple_fault` (a `U8` of the flags) as
  `sensors::Measurement`s of origin `Documented`. The feature `adapters-linux`
  (`["boards", "transport"]`) adds `LinuxSpiDev`, which reads a `/dev/spidev` node with
  plain `read(2)` on a worker thread: no dependency, no `unsafe`, deadline-bounded, and it
  cannot set the bus mode or speed (the device tree does). The testkit gains `FakeSpiBus`
  and `SpiStep` when `boards` is on. Simulator-tested only: the datasheet could not be
  retrieved while this was written, no converter was read, and the bench record is a runbook,
  `docs/hardware/max31855-spi.md`, that needs the maintainer's approval. No change to any
  industrial parity status.
- Release-evidence tooling (no crate changes, slice S62; ADR-0017 addendum): `mise run soak`
  (`examples/soak.rs` and `scripts/soak.sh`) against the simulator or pymodbus, `mise run
  fuzz-evidence` (`scripts/fuzz-evidence.sh`) for the 8-hour release fuzz run and `mise run
  branch-coverage` (`scripts/branch-coverage.sh`, reported by the weekly `scheduled` job),
  with their pass criteria and records under `docs/evidence/`.
- Modbus text types (ADR-0025, finding G08; additive): `CHAR`, `WCHAR`, `STRING(n)` and
  `WSTRING(n)` read and write on holding, input (read) and extended registers.
  `ModbusDataType` gains `Char`, `WChar`, `String` and `WString`, appended after `LReal`;
  `ModbusAddress::string_length`, `ModbusAddressBuilder::string_length` and a
  `string_length` field on the batcher's `Member` carry the declared length, which the
  address text writes in parentheses (`holding-register:1:STRING(20)`, 1 to 5 digits, 1 to
  65535, required for the two strings and refused elsewhere). `CHAR` (one byte of UTF-8)
  and `WCHAR` (one UTF-16BE code unit) are `PlcValue::Char`; `STRING(n)` (`n` bytes of
  UTF-8) and `WSTRING(n)` (`n` code units) are `PlcValue::String`, read up to the first NUL
  with trailing spaces kept and written padded with NUL. Bytes that are not UTF-8 or UTF-16
  fail the item with `MalformedData` and the bytes kept in its status; nothing is replaced,
  where upstream substitutes U+FFFD. A write that does not fit, a string holding NUL, a
  `CHAR` above U+007F and a `WCHAR` outside the Basic Multilingual Plane are the item's
  `InvalidConfig` rejection with no request, and a string wider than one write (more than
  123 registers) its `ResourceLimit` rejection. Text is bytes in register order, each
  register swapped only under the two byte-swap modes, as upstream. A text address takes
  the registers its bytes fill, which the batcher and the read limits count (`STRING(250)`
  and `WSTRING(125)` fit one read); `CHAR`s pack two to a register and strings back to
  back in a list, and a selection that starts inside a register (`[1]:CHAR`) is
  `InvalidAddress`, as upstream refuses it. `TypedDecoder` reads and writes one register of
  a text type. Behavior change: the four names, refused at parse time since the alpha,
  are accepted; the message of a parenthesised length on another type changed. Not added:
  `PlcValue::Bytes`, which waits for a type that needs it. The pymodbus peer serves text
  registers from `fixtures/modbus/interop/text-registers.toml`. Parity rows
  `MB-TYPE-CHAR`, `MB-TYPE-WCHAR`, `MB-TYPE-STRING`, `MB-TYPE-WSTRING` (implemented),
  `MB-INTEROP-TEXT` (new), divergence rows `MB-TYPE-STRING` and `MB-TYPE-CHAR` (new),
  `MB-ADDR-STRING` (retired) and `CORE-VALUE-BYTES` (deferred).
- UDP transport and Modbus TCP over UDP (ADR-0027; additive): `stonecrab::transport::UdpFactory`
  and `UdpTransport` open one socket per connection, connected to the first address of an
  `Endpoint::Custom` holding `host:port`, so datagrams from any other address are ignored.
  `send` sends one datagram and refuses one above `max_frame_bytes` or 65 507 bytes with
  `ResourceLimit`; `recv` appends exactly one whole datagram, a datagram longer than the free
  space is consumed and reported as `ResourceLimit` (upstream drops it and logs a warning), and
  an empty one is skipped. Opening and `shutdown` send nothing; a port the operating system
  reports unreachable is `Disconnected`. New provided method `Transport::delivers_datagrams`
  (default `false`): over such a transport `FramedTransport::next_frame` never joins the rest of
  one datagram to the next (the bytes are dropped and the read is `MalformedData`) and `resync`
  drops the buffer instead of searching it; the testkit's `CountingFactory` passes it through.
  `modbus-tcp:udp://host[:port]` (default port 502, where upstream's declared default never
  applies, `MB-QUIRK-A`) uses the MBAP framing and session of `tcp`, one frame per datagram:
  duplicated and late replies are discarded by transaction id. `ModbusTcpDriver::new` now opens
  `UdpFactory` for that transport code and `TcpFactory` otherwise; an injected factory still
  serves every code. No `udp.` option is offered: every such key stays `InvalidConfig`. Interop:
  diagslave 3.6 (proconX freeware, downloaded at build time and checked by SHA-256, never
  redistributed) as a Modbus UDP slave with `tc netem` loss, delay, duplication and reordering,
  in the new `interop-udp` CI job and `mise run interop-udp`, which `mise run interop` now
  includes. Parity rows `UTR-UDP` (automated-verified), `UTR-UDP-RING`, `UTR-UDP-OPTIONS`
  (implemented) and `MB-TCP-TRANSPORTS-UDP` (automated-verified).
- Lists and Modbus array selections (ADR-0022; additive): `PlcValue::List(Vec<PlcValue>)`
  and `PlcValueType::List`, appended after `DateTime`; equality compares the elements in
  order, and nothing is coerced. Modbus addresses take the upstream array notation on every
  form: `[i]` reads element `i` as a scalar, `[lo..hi]` (inclusive) reads one `List`, even
  `[4..4]`, and `;base` states the declared lower bound (`holding-register:1[4..7;1]:INT` is
  the elements at offsets 3 to 6). Offsets count elements of the type (`[4]:DINT` is eight
  registers along). A selection of more than one dimension, a reversed range, an index
  below its base, a number beyond `u32`, a span past wire address 65535 and a range wider
  than one read (2000 bits, 125 registers) are `InvalidAddress` at parse time. New
  `ModbusAddress::elements` and `ModbusAddressBuilder::elements`, and an `elements` field
  on the batcher's `Member`. A range is one tag to the batcher and one read item, each
  element decoded as it would be alone in its byte and word order; a `BOOL`, `BYTE`,
  `SINT` or `USINT` element takes a register of its own, where upstream packs them (a
  recorded divergence). Writes take a `List` of exactly the element count: function 15 for
  coils, 16 for holding registers, 06 for a one-register payload; a scalar to a range or a
  list to an index is refused by the request builder, a list of another length is the
  item's `InvalidConfig` rejection and a list wider than one write (124 registers, 1969
  coils) its `ResourceLimit` rejection, with no request. The legacy-form hint now gives
  `[0]` for a count of 1, as upstream, and offers a replacement only when it parses
  (none for a range wider than one read or past wire address 65535). The two
  upstream array exchanges of `DriverTestsuite.xml` replay byte for byte. Parity rows
  `MB-ADDR-SELECTION` (implemented) and `CORE-VALUE-LIST` (new).
- Stale tracking and an acquisition loop for sensor profiles (ADR-0023, finding G09;
  additive): `stonecrab::sensors::StaleTracker` retains the last measurement of each name
  and, given the monotonic `now`, marks it `Quality::Stale` once its age exceeds the
  profile's `stale_after` (strictly: still fresh at exactly `stale_after`), reporting each
  transition once and keeping the value, receipt times, source time and protocol status of
  the original reading. A failed read erases nothing, so the earlier value keeps aging.
  `Measurement` gains the field `quality_before_stale` (the quality the value was read
  with, once it is stale; `None` otherwise). `stonecrab::sensors::Acquisition` reads a
  profile through a `Connection` on an injected `Clock`: each measurement every `interval`
  or its longer `min_interval` (a faster rate is raised, not refused), measurements due at
  the same instant in one read request, fresh measurements and stale transitions from
  `recv`, a whole-read failure as `recv`'s error, and `None` once the connection closes or
  fails. No task is spawned and the queue holds at most one event per measurement. No new
  dependency. New parity rows `SENSORS-STALE` and `SENSORS-ACQUISITION`.
- Extended registers (ADR-0015 addendum; additive): `modbus-tcp` reads them with function
  code 20 and writes them with function code 21. The address grammar accepts
  `extended-register:N[:TYPE]`, `6xNNNNN` and, with an explicit `address-convention`, the
  bare `6NNNNN`, with the 15 types a holding register takes. The number is the register
  address under every convention (no one-based shift; 0 does not exist; the last is
  655349999, record 9999 of file 65535, where upstream's 655359999 would name file 65536).
  Address `A` is record `A % 10000` of file `A / 10000 + 1`. A read or write that crosses the
  end of a file is one request with a sub-request per file, and the registers of every
  sub-response are used (upstream keeps the first only, `MB-QUIRK-B`); adjacent tags merge
  into chunks of at most 120 registers; a write is confirmed only by an echo of the whole
  request. New public items, all after the existing variants or new: `ModbusArea::ExtendedRegister`,
  `FunctionCode::{ReadFileRecord, WriteFileRecord}`, `ModbusPdu::{ReadFileRecord,
  ReadFileRecordResponse, WriteFileRecord, WriteFileRecordResponse}` with
  `ModbusPdu::read_file_record_request` and `write_file_record_request`, `FileRecordRead`,
  `FileRecord`, `MAX_READ_EXTENDED_REGISTERS`, `ModbusAddress::{extended, file_number}` and
  `Chunk::file`. Behavior change: function codes 20 and 21 and the text `extended-register`,
  `6x` and the bare `6` prefix are no longer rejected, and `FunctionCode::from_u8(0x14)` is
  `ReadFileRecord` where it was `Unknown(0x14)`. No interop peer serves function codes 20
  or 21 (pymodbus, libmodbus and modbus-tk do not), so the evidence is the specification
  examples, the four upstream parser-suite frames and the first-party responder on the fake
  transport. Parity rows `MB-ADDR-EXTENDED` and `MB-LIMIT-EXT` are `implemented`.
- Array selections on the extended registers (ADR-0022 addendum of 2026-10-29, where the
  two entries above meet): every extended form takes `[i]`, `[lo..hi]` and `;base`. Offsets
  count registers from the register address through the end of a file
  (`extended-register:1[4]:DINT` is address 9, as upstream), an index is the element's own
  address, and the number is checked before a selection moves it. A range ends at or before
  655349999 and holds at most the 120 registers of one file record read: 121 to 125, which
  upstream accepts but no device may answer, are `InvalidAddress`. A range across the end of
  a file is read by one function 20 request, its elements decoded from the joined
  sub-responses, and written by one function 21 request; a list whose request would pass the
  251 data bytes of function 21 (more than 118 registers across the end of a file) is the
  item's `ResourceLimit` rejection, with no request.
- Modbus RTU (ADR-0024; additive): `ModbusRtuDriver`, scheme `modbus-rtu`, with the
  operations of `modbus-tcp` (reads, writes, ping, polling subscriptions, retry, reconnect)
  over RTU framing. `modbus-rtu:///dev/ttyUSB0` or `modbus-rtu://COM3` opens a serial port
  with `serial.baud-rate`, `serial.data-bits` (8), `serial.stop-bits`, `serial.parity`,
  `serial.flow-control`, `serial.interframe-delay` and `serial.reuse-port` (`false`);
  `modbus-rtu:tcp://host[:port]` reaches a serial-to-IP gateway (default port 502); the
  other carriers, unknown keys and unit 0 (broadcast) are refused. New public types
  `RtuAdu` and `RtuFramer` (CRC-16/MODBUS checked on receive, appended on send, replies
  sized by function code as upstream). The PDU codec, the session and `ModbusConnection`
  are shared with Modbus TCP, whose behavior does not change. A reply with a wrong CRC
  fails its request with `MalformedData` and is dropped before the next request; a reply
  from another unit or function is discarded; before each request the line is watched
  until quiet for t3.5 (1.75 ms above 19200 baud) or `serial.interframe-delay`, and after
  an unanswered request for longer, so a late reply cannot answer the next one (over a
  gateway: no gap, and the drain of a 9600 8N1 line). The
  `Framer` trait gains the provided method `resync_discards` (default `false`), which makes
  `FramedTransport::resync` drop the buffer for a framing without a frame marker. The
  `modbus` feature now enables `serial`, so the default build contains `serial2`. New fuzz
  target `fuzz_modbus_rtu`. Simulator-tested only: no independent RTU server and no
  hardware yet. The RTU rows of `docs/parity/modbus-serial.md` are implemented or decided.
- Modbus ASCII (ADR-0024 addendum S60; additive): `ModbusAsciiDriver`, scheme `modbus-ascii`,
  with the operations of `modbus-tcp` over ASCII framing: a colon, the address, the PDU and
  an LRC as upper-case hex, CR LF. The carriers and the `serial.` parameters are those of
  `modbus-rtu` (`modbus-ascii:///dev/ttyUSB0`, `modbus-ascii:tcp://host[:port]`), with
  `serial.data-bits` 8 or 7 (7 with even parity is the usual ASCII line); unit 0 is refused.
  New public types `AsciiAdu` and `AsciiFramer` (the LRC checked on receive and appended on
  send; hex read in either case; noise or a broken frame in front of a colon is skipped to
  the next colon; 513 bytes without CR LF is `Oversized`). A reply with a wrong LRC or a bad
  digit fails its request with `MalformedData` and is dropped before the next request; there
  is no character timeout and no t3.5; after an unanswered request the line is watched for
  the time of a 513-character frame, so a late reply cannot answer the next one. The session,
  the connection and the PDU codec are shared with Modbus TCP and RTU, whose behavior does
  not change. New fuzz target `fuzz_modbus_ascii`. Simulator-tested only: no independent
  ASCII server and no hardware yet. The ASCII rows of `docs/parity/modbus-serial.md` are
  implemented or decided.
- Calendar values (ADR-0021; additive): `PlcValue` and `PlcValueType` gain
  `SignedDuration`, `Date`, `TimeOfDay` and `DateTime` after `Duration`, holding the new
  root types `PlcSignedDuration` (an `i64` nanosecond count; IEC `TIME` and `LTIME`),
  `PlcDate` (an `i32` day count from 1970-01-01, proleptic Gregorian; `DATE`, `LDATE`),
  `PlcTimeOfDay` (`00:00` to `23:59:59.999999999`, no leap second; `TIME_OF_DAY`,
  `LTIME_OF_DAY`) and `PlcDateTime` (a date and a time of day; the three date-and-time
  types). None carries a time zone. Range-checking constructors return `InvalidConfig`;
  ordering is chronological; `Display` prints the `java.time` text of the matching PLC4J
  type (`PT-1.5S`, `2024-02-29`, `13:05`, `2026-10-28T06:30:15.250`). No new dependency.
  No driver decodes calendar values yet; the Modbus `TIME` and `DATE` names stay rejected,
  since upstream has no Modbus encoding for them. New parity row `CORE-VALUE-CALENDAR`.
- Documented device maps and profiles (no crate API change): the register and coil tables of
  the AutomationDirect STRIDE SIO-MB04THMS and SOLO SLM1-C, transcribed by hand from the
  vendors' documents under `fixtures/modbus/devices/`, and one named device profile each
  under `profiles/documented/` (`validation = "documented"`: a thermocouple module with its
  break bits and degree units as status fields, and a controller whose process variable
  keeps its error codes out of `Good`). All four carry the label "documented map, not
  hardware-verified": no device was read. The vendor documents are pinned by URL and sha256
  in `fixtures/modbus/devices/PROVENANCE.toml` and are not copied. The new test
  `modbus_device_replays` serves each map through the first-party responder with the
  vendors' request limits and function tables enforced and runs what the hardware records
  will run: every documented register and coil, the limits, the break flag driving `Bad`,
  the error codes and the documented write targets. The SOLO suite was a PDU-level replay
  over TCP framing until the serial driver existed; since S52e it runs over Modbus RTU.
  Runbooks `docs/hardware/hw1-sio-mb04thms.md` and `docs/hardware/hw2-slm1-c.md` list the
  steps and everything the documents leave open. Findings: the SOLO manual lists function
  codes 05 and 16 but not 15, so a Stonecrab coil write cannot reach it yet, and schema
  version 1 cannot make a process variable error code `Bad` (it is `Uncertain`). New parity
  rows `MB-DEVICE-SIO-*` and `MBS-DEVICE-SLM1C-*`.
- Fixed (ADR-0014 addendum S57, note S57b): on a multi-thread runtime a `close` that
  overtook the supervisor's idle close could drop that transport without shutting it down;
  the idle close now keeps the transport in the guard until its shutdown has returned, and
  `close` shuts down what it finds there. `GuardStats` gains `discarded_frames` (additive;
  the struct is `#[non_exhaustive]`): frames the guard read and dropped between exchanges,
  such as a late reply the supervisor drops before the Modbus session can count it.
- New `serial` feature (off by default; `modbus` enables it since S59): a serial transport
  on a blocking worker thread, `stonecrab::transport::{SerialFactory, SerialTransport}` with
  `DataBits`, `Parity`, `StopBits` and `FlowControl` (upstream's option names and defaults:
  9600 baud, 8 data bits, no parity, 1 stop bit, no flow control). One thread owns the port
  and talks to the async caller over a bounded channel; every call honors its `Deadline`,
  a `send` into a stalled line ends at its deadline and frees the worker within 20 ms, a
  dropped `recv` loses nothing (even one dropped after the worker answered), `shutdown` joins
  the worker and releases the port, and a port error or a closed peer surfaces as the TCP
  transport's error kinds. The port name is an `Endpoint::Custom`; `SerialTransport::from_port`
  wraps a port you opened. New optional dependency `serial2` 0.2.39 (`default-features =
  false`), BSD-2-Clause OR Apache-2.0, which adds no other crate to the lock file and needs
  no `deny.toml` change. Additive API only. Not offered yet: `dtr`, `rts`, `reuse-port`,
  `interframe-delay`, mark and space parity (docs/parity/transport.md). Tested over a
  pseudo-terminal pair on Unix other than macOS; no hardware. ADR-0014 addendum S58.
- Address conventions `zero-based` and `one-based-40001` (additive): `AddressConvention`
  gains `ZeroBased` and `OneBased40001` after `Plc4xCompatible`, with `name()` and
  `from_name()`. `address-convention` (connection string, `AddressOptions`, a sensor
  profile's `address_convention`) now accepts `plc4x`, `one-based-40001` and `zero-based`;
  any other name is still an error. `one-based-40001` reads every text as `plc4x` does
  (numbers from 1, bare forms accepted). `zero-based` reads every numeric form (`AREA:N`,
  `Nx` digits, bare `4NNNN`) as the wire offset, numbers 0 to 65535. With no convention
  nothing changes: the long and prefixed forms are one-based and the bare forms are
  refused. A parsed address is the wire offset and still carries `Plc4xCompatible`;
  `canonical()` stays the one-based long form. The fuzz target `fuzz_modbus_address` reads
  every text under all four options and gains five seeds. New parity row `MB-ADDR-CONVENTIONS`.
- Decision, no behavior change: the batcher still never reads through a gap between tags.
  Upstream merges tags up to the request limit with no gap option and no way to turn it
  off, and a hole in a device's map would turn that into exception 02 for every tag of the
  merged request. `MB-READ-GAP` is recorded as a decided divergence (ADR-0015 addendum)
  and a test shows the two requests on the wire.
- Interop (no crate API change): a second independent Modbus TCP peer, libmodbus 3.1.11's
  `unit-test-server` (C, from Debian trixie's `libmodbus-dev` 3.1.11-2 on an image pinned by
  digest, restarted after each connection), in the new `interop-libmodbus` CI job and `mise
  run interop`. Ten tests in `tests/modbus_interop_libmodbus.rs` cover reads, writes read
  back (FC15, FC06, FC16), exception 06, a reply with the wrong transaction and unit id, a
  500 ms reply inside and past a deadline, a reply sent one byte at a time and a short reply
  to a two-register read (`MalformedData`, no raw bytes). The pymodbus tests share their
  `Peer` helper with the new ones through `tests/support/interop.rs`.
- Polling subscriptions (slice S55, ADR-0020 addendum; additive): the simulator and
  `modbus-tcp` override `Connection::subscribe` and report `SubscriptionMode::Polling`.
  Cyclic subscriptions poll at their interval and deliver every reading; change-of-state
  ones poll every 1000 ms (upstream's default, disclosed through `Subscription::interval`)
  and deliver a tag's first outcome and each change; `Event` is refused with
  `Unsupported(Subscribe)` before any I/O. Each tick is one read through the connection's
  own read path (deadline, retry, batching, the single in-flight slot), the first at once
  and then on a grid of the interval, run while `Subscription::recv` waits: no task is
  spawned. Dropping a waiting `recv` cancels the read it left in flight. Ticks that come
  due during a read, while nobody waits, while the slot is taken or while the link is
  re-established are skipped, never made up, and counted in the new
  `LossCounters::skipped`, which the polling producer bumps. Closing the connection, or
  its terminal failure, ends a subscription after its queued events, and dropping the
  connection without `close` ends a wait for the next tick at once. A change-of-state
  subscription on a clock whose sleeps resolve at once (the simulator's default
  `RealtimeClock`) is refused with `Unsupported(Subscribe)`, because its polling would
  never pause; a cyclic one there takes one fresh sample per `recv`. `Subscription::recv`
  now takes `&mut self`, so only one `recv` can be pending (the type is unreleased), and a
  producer that panicked is dropped instead of polled again. The contract suite gains
  `Requires::Subscribe` and six subscription cases (33); the unsupported-subscribe case is
  skipped by name for both drivers. Behavior change: `Capabilities::subscribe` of both
  drivers is `Polling` instead of `Unsupported`.
- Eager background reconnect and `Timeouts.idle` (ADR-0014 addendum S57; additive):
  `ConnectionGuard::supervise` starts one task, owned by the guard, on the current Tokio
  runtime. It notices a peer that closes while nothing is asked of it, reopens the
  transport when the reconnect backoff passes without waiting for a request (attempt after
  attempt, ending `Failed` when `ReconnectPolicy::max_attempts` is spent), and publishes
  every state through `watch_state`. `close` stops and joins it within `timeouts.request`;
  dropping the guard aborts it, and a task that ended with its runtime is replaced by the
  next call. The reader runs in every state of a live link: the late reply of a timed-out
  request is dropped and a drop that follows is noticed; a reconnected link that stays up
  for a whole read resets the attempt count; a frame it reads while nothing is owed is
  kept for `next_reply` and dropped by the next request, and behind it the reader still
  notices a peer close. With a supervisor, `Timeouts.idle` is enforced: a link no request
  has used for that long is shut down (with a kept frame, if any), the state stays
  `Connected`, the next request opens a new one, and `next_reply` fails `Disconnected`.
  `ConnectionGuard::request_matching(frame, deadline, token, accept)` keeps the in-flight
  slot from the send to the first reply `accept` takes; the Modbus session now correlates
  with it, so neither the supervisor nor the idle bound can come between a discarded reply
  and the real one. `ModbusConnection::open` starts the
  supervisor when it runs inside a Tokio runtime; outside one (the testkit's
  `block_on_with`) reconnect stays request-driven and `idle` is ignored, as before.
  `GuardStats::reconnects` now also counts reopens by the supervisor and after an idle
  close.
- Interop (no crate API change): unit-ID correlation against a scripted multi-unit server, a
  short first-party script on modbus-tk 1.1.5 (LGPL-2.1-or-later, installed from PyPI into a
  `python:3.12-slim` image pinned by digest) that serves units 1, 2 and 247 with a map each
  and does not answer any other unit, in the new `interop-modbus-tk` CI job and `mise run
  interop`. Eight tests in `tests/modbus_interop_modbus_tk.rs` cover each unit's map on one
  connection, one batch across units, alternating units and function codes, a unit that
  refuses an address the others serve (exception 02), an unknown unit ending in `Timeout`
  at the deadline with the connection still good, a batch with an unknown unit, the unit in
  the address block over `default-unit-identifier`, and a write to one unit. The server is
  first-party code, so the evidence is `interoperability-tested` at most.
- Interop (no crate API change): failure paths on a real clock. The new `interop-toxiproxy`
  CI job and `mise run interop-toxiproxy` put Toxiproxy 2.13.0 (MIT, pinned by digest, API on
  the loopback of the host) between the driver and the pymodbus and libmodbus peers. Ten
  tests in `tests/modbus_interop_toxiproxy.rs` add and remove toxics over the Toxiproxy HTTP
  API from the test process (a small client on `std::net`, no new dependency): a reset in
  each direction, a black hole in each direction (`Timeout` at the deadline, twice, then the
  link recovers), latency below and above a deadline with jitter, slicer toxics that cut
  requests and replies into pieces of a few bytes, `limit_data` after one whole reply and
  inside a header, a disabled proxy with open connections, and `docker pause`, `docker kill`
  and `docker start` on the pymodbus container (a hung peer is `Timeout` and its late reply
  is discarded on the same connection; a dead peer is `Disconnected` until it is back). Each
  failure ends in `Timeout` or `Disconnected` within its deadline and the next request reads.
  tshark 4.4 (Debian trixie) captures the run in the proxy's network namespace and decodes
  it independently: no malformed packet, every driver request one well-formed Modbus/TCP
  frame. Evidence under `target/interop-evidence/toxiproxy/`.
- Subscription types and a bounded event queue (ADR-0020; additive): `SubscribeRequest` and
  its builder, `SubscriptionKind` (cyclic, change-of-state, event), `QueuePolicy`
  (backpressure, latest-value coalescing, drop-oldest), `LossCounters`, `Subscription`
  (mode, sampling interval, loss, `recv`, `unsubscribe`) and `SubscriptionSender`. The queue
  holds at most `ResourceLimits::max_queued_events` events, which is now read. `Connection`
  gains a provided `subscribe` that returns `Unsupported(Subscribe)` with no I/O; the
  simulator and the Modbus drivers override it since S55 (polling). The contract suite gains
  the case "an unsupported subscribe performs no I/O and changes no state" (27 cases).
- `SensorProfile::validate_for` checks status bits (findings G02 and G03): a bit beyond the width of
  its status address's type is `ProfileError::StatusBitOutOfRange` (bit 64 or more already in
  `validate`), two spellings of one address and bit are `OverlappingStatusBits`, and a status bit
  inside the measurement's own leading `bit_field` is the new `StatusBitInBitField`; a refused
  status address is now reported there too. Additive: two `#[non_exhaustive]` variants.
- Benchmarks (no crate API change): Criterion benches for Modbus parsing, PDU and MBAP codec,
  `TypedDecoder`, `Batcher::plan` and batched reads over a loopback socket (`mise run
  bench`), a recorded local baseline in `docs/evidence/benchmarks/`, and a weekly
  scheduled-CI gate that fails on a regression of more than 10% beyond the noise band
  (`scripts/bench-compare.sh`); `criterion` is a dev-dependency only.
- CI (no crate changes): the `semver` job is blocking against the published 0.2.0, with
  cargo-semver-checks pinned to 0.51.0.
- `SensorProfile::validate` rejects a measurement range with a NaN or infinite bound or
  `min > max` (new `ProfileError::InvalidRange`); before, such a range left values
  Uncertain or Good by accident.
- Changed: the word-order rule of sensor profiles is keyed on what the driver declares, not
  on Modbus type names (finding G05). `SensorProfile::validate` no longer reports
  `MissingByteOrder` or `MissingWordOrder`; the new `SensorProfile::validate_for(&dyn
  Driver)` runs every `validate` rule, then parses each measurement's address and reports
  those errors (and `AddressRejected`) when `ProtocolAddress::spans_multiple_words` is true
  for the parsed address and the definition sets no such order. `SensorProfile::tags` runs
  `validate_for`, so a Modbus profile without orders is still refused there. Additive API:
  the provided `ProtocolAddress::spans_multiple_words` (false by default, true for Modbus
  addresses wider than one register) and `validate_for`; the testkit's `FixtureAddress`
  and `ConventionDriver` can declare wide addresses.
- Modbus write PDUs as upstream emits them (the PDU codec of S45; the writes go on the wire
  since S46): `FunctionCode` gains `WriteSingleRegister`,
  `WriteMultipleCoils` and `WriteMultipleRegisters`, `ModbusPdu` gains their requests and
  responses and `ModbusPdu::write_request`, which picks function 15, 06 or 16 for a payload.
  Behavior change: function codes 06, 15 and 16 now decode (`FunctionCode::from_u8` returns the
  named variant, `decode_request` and `decode_response` accept them) instead of being refused
  as `Unknown`; the enums are `non_exhaustive`, so this is not a semver break. FC05 is still
  not decoded.
- `ModbusConnection` overrides `Connection::ping`: one read of `ping-address` through the
  session and in-flight slot of a read, bounded by `timeouts.request`; any reply, an
  exception included, is success as upstream's (`MB-QUIRK-D`); `Capabilities.ping` is true
  for `modbus-tcp`. The contract suite gains `Requires::Ping` and two cases (26).
- Modbus TCP writes on the wire (slice S46): `ModbusConnection::write` sends one request per
  item (function 15 for a coil, 06 for a one-register holding payload, 16 otherwise), one
  after another, through the session and the single in-flight slot of a read; a coil takes a
  `BOOL`, a register the type of its address in the byte and word order of the tag, the
  address block or the connection default. An echo is `Confirmed`, a device exception is
  `Rejected` with its code as the protocol status, a request that went out and got no usable
  answer (deadline, lost link, cancellation, a reply that does not echo it) is
  `WriteOutcome::Unknown` and is never retried or replayed, and the items after an unknown one
  are not sent. A discrete input or an input register is `Rejected` without a request.
  `Capabilities.write` is true for `modbus-tcp`. New connection-string parameter
  `access=read-only|read-write` (default read-only): a connection whose access policy is
  read-only refuses every write before anything else, so writes must be granted. The
  contract suite's write cases now run for `modbus-tcp` over the fake transport and loopback
  sockets (and the read-only case against pymodbus); the upstream write
  exchanges of the three retained driver-suite files are vectors (60 more) and are replayed
  byte for byte. Additive API; `ModbusSession::request` also carries writes, and its span
  says `op = "write"` for one.
- `stonecrab::sensors` now denies `clippy::indexing_slicing` (checked by `lint-attrs`), like
  the other modules that parse untrusted input; no code in the module needed to change.
- Release process and evidence (no crate changes): ADR-0017, `release.toml`,
  `mutants.toml`, the `release-evidence` workflow, `docs/evidence/` with the recorded
  fuzz, mutation and coverage figures, and drafted (not filed) upstream issues.
- Documents reconciled with the 0.2.0 publication (no crate changes): README, SECURITY,
  STATUS, the project brief and the coding standards no longer call 0.2.0 unpublished or
  use the old repository owner.
- `Operation::Ping`, `Capabilities.ping` and a provided `Connection::ping` that returns
  `Unsupported` until a driver overrides it (additive; `ModbusConnection` does since S47).
- Upstream Modbus test vectors (no crate changes): four Apache PLC4X test files are
  retained verbatim under `fixtures/modbus/upstream/` with NOTICE attribution, and the six
  frames of the parser suite are vectors, none of them in the published crate.
- MSpec code generator decision (no crate changes): ADR-0019 chooses a native Rust
  maintainer tool, `tools/stonecrab-codegen`, proven first on Modbus against the
  hand-written codec through the vector corpus, with a provenance header and an offline
  `regen` gate (built in S66 and S67).
- Upstream driver-suite vectors (no crate changes): a test-only adapter
  (`tests/modbus_driver_suite_adapter.rs`, the `roxmltree` dev-dependency) serializes the
  XML models of the retained PLC4X driver suites with its own mspec serializer and emits 64
  vectors, the request and response of 32 read exchanges, under `fixtures/modbus/tcp/`
  (none of them in the published crate). `TypedDecoder` decodes every one to the value
  the XML reports, so `MB-BYTEORDER-BE` and `MB-BYTEORDER-LE` are `automated-verified`; the
  two byte-swap modes are in no upstream file and stay `implemented`.

## Alpha
The first pre-release is `stonecrab` 0.2.0, one crate (ADR-0018). Its section below opens
with the four release-note items that `docs/SPEC.md` ('Open-source project requirements')
requires, once for the crate: the upstream baseline, the supported operations, the actual
hardware evidence and the breaking changes. The support levels are those of the 'Hardware
evidence' policy of `docs/SPEC.md`; the full matrix is `docs/COMPATIBILITY.md`.

Gates not run for the alpha, so that nobody reads their absence as a pass: branch coverage
(line coverage is gated per module, region coverage is reported), a mutation-testing
threshold, the 8-hour release fuzz run (CI fuzzes each target for 60 seconds), the 24-hour
soak, benchmarks, differential tests against PLC4J, hardware verification, and `cargo
semver-checks` against the specification-stage 0.1.0 package (its API was replaced). The
measured substitutes are recorded by the release-evidence slice S32 and re-recorded for
the one crate by S33 (`docs/evidence/alpha.md`).

### stonecrab 0.2.0 (2026-10-27)
- **Upstream baseline:** Apache PLC4X v1.0.0 (commit
  `ea229918f5d07700e04d7d69ab1645da679cb700`, pinned in `upstream/manifest.toml`; the
  inventory covers the Modbus TCP subset only and the source-release signature was
  verified in S32, see `docs/evidence/alpha.md`). The connection-string grammar of the core
  and the Modbus TCP driver follow it; divergences are rows in `docs/parity/modbus.md` and
  are listed in `docs/COMPATIBILITY.md`. The simulator, the transport, the codec and the
  sensor-profile format are Stonecrab's own and make no upstream claim (Apache PLC4X ships
  a simulated driver whose source is not inventoried, builds its transports on Netty and has
  no sensor profile concept). Claims are per parity row in `docs/parity/`.
- **Supported operations:** the core defines the driver boundary (`DriverRegistry`,
  `Driver`, `Connection`: connect, read, write, close, state watching) and the data
  contracts, and implements no driver; it is `simulator-tested` through the common driver
  contract suite. `stonecrab::sim` (feature `sim`): connect, close, read (`STATE` and
  `RANDOM`), write (`STATE` and `SINK`), scripted failures and bounded read retry,
  `simulator-tested`. `stonecrab::modbus` (feature `modbus`): Modbus TCP connect, close and
  reads of coils, discrete inputs, holding registers and input registers (function codes
  01 to 04) as the 15 numeric and bit types, with batching and `split_into` disclosure,
  request-driven reconnect, retry, cancellation and timeouts; reads are
  `interoperability-tested` against pymodbus 3.8.6 and the fault handling is
  `simulator-tested`. Writes (the driver reports `write: false`), subscriptions, browse,
  discovery, ping, extended registers, strings, calendar types, RTU, ASCII, UDP and TLS are
  not supported. `stonecrab::transport` (feature `transport`): the deadline-bounded TCP
  transport, framing with resynchronization and the connection guard; the connect and
  request path is `interoperability-tested` through the Modbus driver, resync, reconnect
  and the queue are `simulator-tested`; serial, UDP and TLS are not supported.
  `stonecrab::codec`: bounds-checked byte primitives, `build-tested`.
  `stonecrab::sensors` (feature `sensors`): loading and validating version-1 TOML
  profiles, building tags through a driver and turning a read response into scaled, graded
  measurements; the synthetic profiles are `simulator-tested` against the simulator and
  `interoperability-tested` against pymodbus 3.8.6, and no named device profile exists.
  `stonecrab::testkit` (feature `testkit`) is test tooling and has no support level.
  Subscribe, browse and discover are not defined or supported by any module. The matrix is
  `docs/COMPATIBILITY.md`.
- **Hardware evidence:** none. No physical device has been tried and no hardware record
  exists; nothing is `hardware-verified`. The simulator models no wire protocol.
- **Breaking changes:** versus the published 0.1.0 specification-stage package, the
  `placeholder::status` API is removed with no replacement (migration: delete the call;
  consult `docs/COMPATIBILITY.md` for what is supported). Everything else is new: the
  0.1.0 package exposed nothing else, and the sibling crates of the earlier seven-crate
  layout were never published. Behavior that differs from upstream by design: bare `40001`
  addresses need an explicit convention, unknown connection parameters are errors, replies
  are validated on transaction id, unit id and function code.

Changes. The first bullet is the single-crate build of slice S33; the bullets after it
are grouped by module and keep the wording they had when each landed, so the crate names
in them (`stonecrab-core`, `stonecrab-modbus` and so on) are the former crates, now the
modules of the group heading (ADR-0018).

- **One crate (S33, ADR-0018):** `stonecrab` now contains the former sibling crates as
  modules (`stonecrab::codec`, `transport`, `modbus`, `sim` and `sensors`, with the core
  contracts at the crate root) and the test kit as `stonecrab::testkit` behind the
  off-by-default `testkit` feature (a separate test crate would be a dev-dependency
  cycle). `cargo add stonecrab` gives Modbus TCP, the simulator and the sensor profiles
  with no feature flags (default features `sim`, `modbus`, `sensors`);
  `default-features = false` is the tokio-free core; `full` stays as an alias. The `tcp`
  and `toml` features are gone (the TCP transport is part of `transport`, the TOML loader
  of `sensors`). There is one version, one changelog, one packaging list, one SBOM and one
  release tag, `v0.2.0`. Every test, doctest, gate, example, fuzz target and CI job is
  kept; lint promotions and coverage thresholds apply per module; test files declare
  `required-features` so a plain `cargo test` skips what it cannot build.

#### Core (crate root)
- Move the facade into a resolver-3 workspace with a runtime-independent core crate.
- Remove the specification-only `placeholder::status` API. There is no replacement
  status API; consult the capability and parity documentation.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) and stop shipping `tests/` in
  the crate archive.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) for feature-gated item docs.
- Establish shared metadata, safety lints and foundation contracts.
- Add exact-width scalar values, validated tag identifiers, monotonic deadlines,
  immutable finite resource budgets, explicit capabilities, typed diagnostic
  failures and redacted secret formatting.
- Add protocol-owned tags, bounded read builders, exact batch correlation,
  preserved per-item protocol status and explicit unknown-after-transmit outcomes.
- Add `ByteOrder` and `WordOrder`, optional `Tag::with_byte_order`/`with_word_order` with
  `byte_order()`/`word_order()` accessors, and additive `PartialOrd, Ord` on `ErrorKind`,
  `Operation` and `PlcValueType` (ADR-0006).
- Add `OpFuture`, the `Clock` trait (no implementation in core), `CancellationToken` with
  child tokens and `StateCell`/`StateWatch` latest-value connection-state observation,
  all on `std` only (ADR-0007).
- Add `ConnectionConfig` (not `Clone`) with its validating builder, `Endpoint`, `Timeouts`,
  `ReconnectPolicy` with pure `next_delay`, `RetryPolicy`, `AccessPolicy` (default
  `ReadOnly`), `Credentials`, and the `AddressOptions` carrier with its builder
  (ADR-0008). The simulator connection enforces the timeouts, retry and access policies.
- Add write batch contracts: `WriteItem`, `WriteRequest` with its bounded builder (item
  budget, duplicate identities, value type versus tag hint, deadline) and `WriteResult`,
  `WriteResponse` with its correlating builder, `iter_confirmed`, `iter_rejected` and
  `iter_unknown`. Unknown outcomes are first-class and never retried or replayed
  (ADR-0010). Data only.
- Add `ConnectionUri::parse` for `scheme(:transport)?://authority(?k=v(&k=v)*)?` with
  ordered `options()`; it rejects percent-encoding, credentials in the authority, empty
  or duplicate keys, whitespace and input over 4096 bytes, and `Display` renders the
  canonical text. Covered by the `fuzz_core_connection_string` target.
- Add the `Driver` and `Connection` traits (object safe, `Send + Sync`; the narrow D2
  surface) and `DriverRegistry` (`register` takes `&mut self`, duplicate scheme
  `InvalidConfig`, unknown scheme `Unsupported(Operation::Connect)`, `parse` and `connect`
  through `ConnectionUri`), and the default-deadline rule they document (ADR-0011).
- Add `ReadRequestBuilder::cancel`, `ReadRequest::cancel`, `WriteRequestBuilder::cancel`
  and `WriteRequest::cancel`: an optional `CancellationToken` carried by a request.
- Accept ADR-0007: the simulator connection enforces the token and the default deadline.
- Add `FakeClock`, `block_on`, `block_on_with`, `poll_once` and a counting waker to the
  unpublished testkit for deterministic, sleep-free async tests.
- Add `DriverOptions`, `ConnectionConfigBuilder::driver_options`,
  `ConnectionConfig::driver_options` and `driver_options_as`: a slot for options only one
  driver understands (ADR-0008 amendment). `Debug` and `redacted_display` print them, so
  their `Debug` must hold no secrets.
- Re-export `ByteOrder` and `WordOrder` from `stonecrab-core`.
- Re-export `OpFuture`, `Clock`, `CancellationToken`, `StateCell` and `StateWatch` from
  `stonecrab-core`.
- Re-export `ConnectionConfig`, `ConnectionConfigBuilder`, `Endpoint`, `Timeouts`,
  `ReconnectPolicy`, `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions` and
  `AddressOptionsBuilder` from `stonecrab-core`.
- Re-export `WriteItem`, `WriteRequest`, `WriteRequestBuilder`, `WriteResult`,
  `WriteResponse` and `WriteResponseBuilder` from `stonecrab-core`.
- Re-export `ConnectionUri` from `stonecrab-core`.
- Re-export `Driver`, `Connection` and `DriverRegistry` from `stonecrab-core`.
- Re-export `DriverOptions` from `stonecrab-core`.

#### Codec (`stonecrab::codec`)
- Add the crate: `BoundedReader` and `BoundedWriter` with bounds-checked integer, float,
  byte-run and least-significant-bit-first coil reads and writes in an explicit
  `ByteOrder`, and `ParseError` (`Truncated`, `OversizedLength`, `InvalidDiscriminator`,
  `TrailingBytes`, `Overflow`) with `From<ParseError> for PlcError`.
- Add `coil_byte_count`, `pack_bits_lsb_first` and `unpack_bits_lsb_first`.
- Lengths are checked against the caller's maximum and the remaining input before any
  allocation or copy; no function panics on any input (ADR-0013).

#### Transport (`stonecrab::transport`)
- Add the crate: the object-safe `Transport` trait (`send`, `recv`, `shutdown`, `peer`,
  `limits`; every call takes a `Deadline`) and `TransportFactory` (both return
  `OpFuture`), and `FrameError` (`Truncated`, `Oversized`, `Malformed`) with
  `From<FrameError> for PlcError` (ADR-0014).
- Add `TcpTransport` and `TcpFactory { nodelay }` behind the default `tcp` feature: name
  resolution and connection inside one connect deadline, refused and reset connections as
  `Disconnected`, `recv` bounded by `max_buffered_bytes`, `send` bounded by
  `max_frame_bytes`, and a transport that poisons itself when a `send` is cut short.
- Add `SystemClock`, the Tokio-backed `Clock` (`now()` is Tokio's instant so paused-time
  tests agree with `Deadline`), and `Jitter`, a seeded xorshift64* source for
  `ReconnectPolicy::next_delay`.
- Adopt `tokio` with scoped features (`rt`, `time`, `sync`, `io-util`, and `net` through
  `tcp`); `deny.toml` bans its `full` feature. No task is spawned and nothing runs between
  calls.
- Add the `Framer<F>` trait (`decode`, `encode`, `max_frame_bytes`) and
  `FramedTransport<F, R>` over a `Box<dyn Transport>`: `next_frame` assembles fragmented
  and coalesced frames within `max_buffered_bytes`, `send_frame` encodes and sends one
  bounded frame, and `resync(Deadline)` discards bytes until the framer finds a frame
  boundary and fails with `Disconnected` at the deadline (ADR-0014 addendum).
- Add `ConnectionGuard<F, R>`, `GuardConfig` and `GuardStats`: the connection lifecycle
  machine over a `TransportFactory` and a `Framer` (`connect`, `request`,
  `request_with_default_deadline`, `next_reply`, `close`, `state`, `watch_state`,
  `stats`), one request in flight with a bounded FIFO queue (`Backpressure` beyond `max_pending_requests`),
  request-driven reconnect with jittered backoff and `Failed(Disconnected)` when the retry
  budget is spent, resync before the next request after a timeout, `Diagnostics` on every
  error, and a `close` bounded by `Timeouts.request`. No background task (ADR-0014
  addendum).

#### Simulator (`stonecrab::sim`)
- Add the crate: the in-process simulator driver, depending on `stonecrab-core` and
  `tracing` only, Tokio-free, default features empty (ADR-0012, accepted).
- Add `RealtimeClock`, a `std`-only `Clock` whose `now()` is `Instant::now()` and whose
  `sleep_until` resolves immediately without moving time.
- Add `SimAddress` and `SimArea`: the grammar `STATE|RANDOM|SINK/<name>:<type>` with
  lowercase scalar type names, `ProtocolAddress` with protocol `sim`, 256-byte bound,
  `STDOUT` renamed `SINK`, the legacy array form rejected.
- Add `SimDevice` and `SimTag`: named tags, `ramp` with checked arithmetic that holds at
  the last value on overflow, a read counter, a poison-recovering shared device, stored
  latency and item limit, and a seeded xorshift `RANDOM` source (`SimDevice::with_seed`).
- Adopt `tracing` 0.1 (`std` only) workspace-wide; `deny.toml` lets only `tracing-core`
  depend on the banned `once_cell`.
- Add `SimDriver` (scheme `sim`; `sim://<device>?latency-ms=<n>&seed=<u64>`; `new`,
  `with_clock`, `with_device`, `device`) and `SimConnection`, the `Connection` it opens: the
  lifecycle `Disconnected`, `Connecting`, `Connected`, `Closing`, `Closed`; reads of
  `STATE` and `RANDOM` with per-item errors, writes to `STATE` and `SINK`; one operation in
  flight (a second fails with `Backpressure`); the default-deadline rule, cancellation and
  deadlines on the injected clock; whole-request read retry under `RetryPolicy`;
  `Diagnostics` on every error; `connect`, `read`, `write` and `close` spans. The README
  support table now marks read and write `simulator-tested`, and a test keeps it equal to
  `SimDriver::capabilities()`.
- Add `SimDevice::write`, `record_sink_write`, `sink_writes` and `reseed`.
- Add the failure script: `FailureStep` (`Delay`, `Timeout`, `Disconnect`,
  `DeviceException`, `Malformed`, `PartialFailure`, `Backpressure`, `UnknownAfterTransmit`),
  `FailureScript` (a bounded queue) and `SimDevice::push_failure`, `push_failures`,
  `pending_failures` and `clear_failures`. A connect, read or write takes at most one step;
  a scripted disconnect leaves the connection `Failed(Disconnected)`; a read without a
  deadline now gets `timeouts.request` afresh for each retry attempt.
- Add the `sim` feature (off by default): `stonecrab::sim` re-exports the public items of
  `stonecrab-driver-sim`, pinned `=0.1.0`, and the `sim_read` example reads three tags
  from a simulated device through the driver registry.

#### Modbus (`stonecrab::modbus`)
- Add the native Modbus TCP driver crate, depending only on `stonecrab-core`, with default
  features empty. It starts with the address types and their text grammar; the codec,
  framing, session, connection and driver follow in the entries below.
- Add `UnitId` (one MBAP byte; default 1; 0 and 255 are carried, not rejected),
  `ModbusArea` (`Coil`, `DiscreteInput`, `HoldingRegister`, `InputRegister`),
  `ModbusDataType` (the 15 numeric and bit types, with `from_upstream_name` recognising
  all 27 upstream names and refusing the 12 deferred ones by name),
  `AddressConvention::Plc4xCompatible` and `ModbusAddress` built through a validating
  `ModbusAddressBuilder`.
- `ModbusAddress` implements `ProtocolAddress` with protocol `modbus` and a canonical
  long form such as `holding-register:16:INT{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`
  (ADR-0015, proposed).
- Add `ModbusAddress::parse(text, &AddressOptions)`, the PLC4X address grammar: long
  and `0x`/`1x`/`3x`/`4x` short forms always, bare `40001`-style forms only with
  `address-convention=plc4x`, an optional `{unit-id: N, byte-order: 'NAME'}` block, and
  typed `InvalidAddress` rejections for everything else; `ModbusAddress::MAX_TEXT_BYTES`
  and the fuzz target `fuzz_modbus_address`.
- Add `Batcher::plan`, `BatchLimits`, `Chunk` and `Member`: tags are grouped by unit id
  and area, sorted by wire offset and merged into reads only when adjacent or overlapping
  (no gap reads), never above 125 registers or 2000 bits per chunk or `max_items` tags
  per plan.
- Add the PDU codec (ADR-0015): `FunctionCode`, `ExceptionCode` (the ten V1.1b section 7
  codes and `Unknown`) and `ModbusPdu` for the read functions 01 to 04, their normal
  responses and the exception response, with `encode_request`, `decode_request`,
  `decode_response`, `encode_response`, `ModbusPdu::check_reply_to` and the limits
  `MAX_PDU_BYTES`, `MAX_READ_BITS` and `MAX_READ_REGISTERS`. Quantities (1 to 2000 bits,
  1 to 125 registers), byte counts and the 253-byte PDU bound are checked before any
  allocation; write PDUs and exception codes outside the ten are refused. The crate now
  depends on `stonecrab-codec`. Golden vectors under `fixtures/modbus/tcp/` are written
  from the V1.1b examples or built to fail; the fuzz target is `fuzz_modbus_pdu`.
- Add MBAP framing (ADR-0015): `MbapHeader`, `TransactionId` (1 to 0xFFFE, wrapping as
  upstream, never 0 or 0xFFFF), `ModbusAdu` (a header and an opaque PDU of 1 to 253
  bytes) and `MbapFramer`, a `stonecrab_transport::Framer`. The header is judged before
  the body is awaited or allocated: protocol id other than 0 or length 0 or 1 is
  `Malformed`, length above 254 (ADU above 260 bytes) is `Oversized`, an incomplete ADU
  waits. The crate now depends on `stonecrab-transport` (without its `tcp` feature) and
  `bytes`. Golden ADU vectors are `fixtures/modbus/tcp/adu_*`; the fuzz target is
  `fuzz_modbus_mbap`.
- Add `TypedDecoder`: `decode_registers`, `decode_bits` and `encode_registers` map the 15
  numeric and bit types to `PlcValue`s in the four upstream byte-order modes (a `ByteOrder`
  and a `WordOrder`; `0x01020304` is `[1,2,3,4]`, `[4,3,2,1]`, `[2,1,4,3]`, `[3,4,1,2]`),
  with exact integers, bit-for-bit floats and the single-register padding of `BOOL`,
  `BYTE`, `SINT` and `USINT`. A wrong register count is a `MalformedData` item error that
  keeps the received bytes within the caller's budget. The little-endian modes are derived
  from the documented layouts; they are not `automated-verified` until the upstream
  little-endian vectors are added. Vectors are under `fixtures/modbus/tcp/typed_*`.
- Add `ModbusSession` (ADR-0015): one read request in flight over the transport crate's
  `ConnectionGuard`, a transaction counter that continues across reconnects, and
  `request(pdu, unit_id, deadline, token)` returning the reply PDU. A reply is accepted
  only when its transaction id, unit id and function code all match (upstream compares the
  transaction id only; divergence `MB-READ-MATCH`, V1.1b section 7); anything else is
  discarded, counted in `SessionStats` as `late_replies` (the id of a request the session
  gave up on) or `mismatched_replies`, and logged at `warn` with the expected and received
  transaction ids. A device exception is `Ok(ModbusPdu::Exception)` with the connection
  still `Connected`; `exception_item_error` maps it per decision D18 and keeps the raw code
  and name. A reply PDU that does not decode or fit the request is `MalformedData`. Each
  request has a `debug` span with `scheme`, `endpoint`, `op` and `txn`. The crate now
  depends on `tracing`.
- Add `ModbusConnection` and `ConnectionDefaults` (ADR-0015): the `Connection` of the
  `modbus-tcp` scheme over a `ModbusSession`, opened with `ModbusConnection::open(config,
  factory, clock, defaults)`. A read is planned by `Batcher::plan` and runs chunk by chunk
  (function codes 01 to 04, at most 125 registers or 2000 bits), each chunk under
  `min(request deadline, now + timeouts.request)`; `ReadResponse::split_into` discloses the
  chunk count, `sampled_at` is the instant each chunk was answered and `source_time` is
  `None`. A device exception fails only its chunk's members; a failed exchange fails the
  read, after retries under the configuration's `RetryPolicy` for retryable kinds only.
  One read is in flight at a time (`Backpressure` before any I/O). `write` is
  `Unsupported(Operation::Write)` with no I/O. `ConnectionDefaults` carries the default unit
  and the byte and word order that a tag or address without its own falls back to.
- Add `ModbusTcpDriver` (ADR-0015, decision 14): the `Driver` of the `modbus-tcp` scheme,
  with `new()` (real TCP transport and `SystemClock`), `with_clock`,
  `with_transport_factory` (the way to run the testkit `FakeTransportFactory`) and
  `with_defaults`. `parse_config` reads `modbus-tcp[:tcp]://host[:port]` (port 502 by
  default, IPv6 in brackets) and accepts no parameter yet; the six upstream parameters and
  `address-convention` follow with the configuration slice. `parse_address` delegates to
  `ModbusAddress::parse` with the convention of the options, and `capabilities` is the
  value every `ModbusConnection` reports. The crate now enables the `tcp` feature of
  `stonecrab-transport`. The common driver contract suite runs green against the driver over
  the fake transport (`tests/contract.rs`, with a first-party MBAP responder over the
  interop register map); only the two write cases are skipped, by name. The unpublished
  testkit gains `Step::React` and `Reaction` (reply, stay silent, close, reply then close),
  so the responder can drop the link on demand and the two disconnect cases pass.
- Complete the driver configuration: `parse_config` now accepts the six upstream
  parameters with the upstream defaults (`request-timeout-ms` 5000,
  `default-unit-identifier` 1, `ping-address` `4x00001:BOOL` validated and recorded but
  unused, `default-payload-byte-order` `BIG_ENDIAN`, `max-coils-per-request` 2000,
  `max-registers-per-request` 125) and `address-convention=plc4x`; every other key, the
  misspelt `unit-identifier` included, is `InvalidConfig`, where upstream warns. The parsed
  values are `ModbusOptions`, carried in the configuration's driver-options slot and read
  back by `connect`. `ConnectionDefaults` gains `max_coils` and `max_registers`, and `open`
  now refuses values above the protocol maxima. `with_defaults` is now the fallback for a
  configuration that carries no options. The crate re-exports `SystemClock`. The fuzz target is
  `fuzz_modbus_connection_string`.
- A holding-register read runs over a real loopback socket against a scripted peer
  (`tests/scripted_read.rs`), and the driver contract suite runs over loopback sockets
  (`tests/contract_scripted_peer.rs`: 18 passed, 5 skipped by name). The README documents the
  connection-string grammar, the parameter table, the byte-order names and the support levels,
  and a test keeps its table equal to `ModbusTcpDriver::capabilities()`.
- Add the `interop` CI job and `mise run interop`: the driver reads the interop register map
  from a first-party pymodbus 3.8.6 server in Docker (`interop/pymodbus/`), ten ignored
  tests in `tests/interop_pymodbus.rs` and the `modbus_tcp_read` example, with the image id,
  `pip freeze`, server log and contract report uploaded as `interop-evidence`; ADR-0015 is
  accepted (decision 16).
- Add the `modbus` feature (off by default): `stonecrab::modbus` re-exports the public items
  of `stonecrab-modbus`, pinned `=0.1.0`, including `ModbusTcpDriver`, `ModbusOptions` and
  `SystemClock`, and the `modbus_tcp_read` example reads one tag from a Modbus TCP server
  through the driver registry. Each example now declares `required-features`.

#### Sensors (`stonecrab::sensors`)
- Add the crate: declarative sensor profiles for Stonecrab, depending on `stonecrab-core`,
  `serde` and, behind the default `toml` feature, `toml` (ADR-0016). No driver reads a
  profile yet; that arrives in a later slice.
- Add the schema (version 1): `SensorProfile`, `ProfileId`, `SchemaVersion`,
  `DeviceApplicability`, `ConnectionRequirements`, `MeasurementDef`, `ByteOrderName`,
  `WordOrderName`, `ScaleStep` (`Multiply`, `Divide`, `Offset`, `Clamp`, `BitField`),
  `ValueRange`, `StatusField`, `QualityMapping`, `Provenance` and `ValidationStatus`. All
  derive `Deserialize` with unknown keys denied; none derives `Serialize`.
- Add `SensorProfile::from_toml` (feature `toml`): 64 KiB size bound checked before
  parsing, schema versions other than 1 refused as `ProfileError::IncompatibleSchema`, and
  errors that carry a line and column but never part of the document. Add `ProfileError`
  (`Oversized`, `Syntax`, `IncompatibleSchema`, `InvalidIdentifier`) with
  `From<ProfileError> for PlcError` (`InvalidConfig`).
- Add the fuzz target `fuzz_sensors_profile_toml`. Adopt `serde` and `toml` (`display`
  off); the lock pins `toml_parser` 1.0.9 to keep a single `winnow`.
- Add `SensorProfile::validate`, which returns every semantic error of a profile at once
  (duplicate names, unknown or empty units, missing byte or word order for 32- and 64-bit
  Modbus types, overlapping status bits, `count != 1`, unevaluable scaling steps, unknown
  quality references, address convention rules), and the fixed unit table `Unit`. Add eleven
  `ProfileError` variants for them; the enum was already `#[non_exhaustive]`.
- Add the measurement pipeline: `SensorProfile::tags(&dyn Driver)` builds one tag per
  measurement and one `<measurement>.<status name>` tag per status field, parsing every
  address under the profile's address convention (`SensorProfile::address_options`), and
  refusing a scheme mismatch; `SensorProfile::measurements(&ReadResponse, SystemTime)`
  scales values in declared order with checked arithmetic (integers stay integers without
  a float step, NaN, infinity and overflow are `Bad`, `BitField` handles counter rollover)
  and grades them with `Quality` (`Good`, `Uncertain`, `Bad`, `Stale`, `Unknown`) from the
  range, the item error and the quality mapping. Add `Measurement`, `Quality`,
  `MeasurementDef::status_tag_id` and four `ProfileError` variants (`SchemeMismatch`,
  `AddressRejected`, `StatusAddressRejected`, `DuplicateStatusFieldName`). The simulator
  and the testkit are dev-dependencies for the tests; the unpublished testkit gains the
  `ConventionDriver` test double of `Driver`.
- Add the synthetic reference profiles under `profiles/synthetic/` at the repository root:
  temperature, pressure/flow/level and discrete state, each as a `modbus-tcp` file authored
  from the pymodbus register map and a `sim` twin, all labeled `validation = "synthetic"`.
  They are repository data and are not packaged with the crate. The crate's code is
  unchanged; a new test loads and validates every file.
- Add the `sensors` feature (off by default): `stonecrab::sensors` re-exports the public
  items of `stonecrab-sensors`, pinned `=0.1.0`, with its TOML loader enabled, and the
  `full` feature (`sim`, `modbus`, `sensors`). The `profile_read` example (requires `full`)
  loads a synthetic profile from `profiles/`, reads it from `sim://bench` or a Modbus TCP
  server and reports name, value, unit, quality, origin and raw value through `tracing`;
  `--expect-good` makes it fail on any measurement that is not of quality `Good`.

#### Testkit (`stonecrab::testkit`)
- Add an unpublished deterministic address fixture testkit for contract tests.
- Add the unpublished testkit's `fixtures` feature: the golden-vector loader (`load_dir`,
  `load_hex`, `GoldenVector`, `VectorSource`, `Provenance`, `GoldenError`) that enforces
  "every vector is listed in `PROVENANCE.toml`", and `InteropMap`, the reader of the
  first-party `fixtures/modbus/interop/register-map.toml`. The upstream vectors themselves
  wait for maintainer prerequisite P4.
- Add the unpublished testkit's driver contract suite (`ContractSuite`, `ContractFixture`,
  `ContractReport`): 23 named cases that every driver runs, green against `sim`.
- Add the unpublished testkit's `ScriptedPeerFactory` (`net` feature): a `TransportFactory`
  that starts a fresh `ScriptedPeer` per `open` and connects with the real `TcpFactory`.
- Test the transport against the unpublished testkit's new `net` feature (`Script`,
  `ScriptedPeer`, `FakeTransport`, `FakeTransportFactory`); no change to the public API.

## stonecrab 0.1.0
- Publish the specification-stage package. No industrial protocol support.
