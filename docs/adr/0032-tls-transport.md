# 0032: TLS transport
## Status
Accepted (2026-10-29), as slice S71 of docs/POST_ALPHA_PLAN.md ("TLS transport and
`modbus-tcp:tls`"; exit "`MB-TCP-TRANSPORTS-TLS` implemented; an interop job over TLS"),
under the maintainer's decision E11 of 2026-10-29: TLS through rustls with the ring provider
behind an opt-in `tls` feature, mutual TLS with explicit trust anchors, and no TLS-PSK. It
adds a transport beside TCP (ADR-0014), UDP (ADR-0027) and serial, and widens the transport
codes of the `modbus-tcp` driver (ADR-0015). Every change to the public API is additive to
master at 0.3.0, and none of it exists without the new feature.

## Context
Upstream's `transport_tls` (Apache PLC4X v1.0.0, commit ea229918) is a blocking JSSE
`SSLSocket` over a socket configured from the TCP options. It enables `[version]` when the
option is set and `[TLSv1.3, TLSv1.2]` otherwise, verifies the host name unless
`ignore-common-name` is set, and takes `verify`, `trust-store` (PKCS #12 by default, with a
password and a type), `keystore` (the same, for mutual TLS) and `log-session-keys`; `verify=false`
installs a trust-all manager (`docs/parity/transport.md`: `UTR-TLS`, `UTR-TLS-TRUST`). Its
`tls-psk` transport is Bouncy Castle's `PSKTlsClient` with an identity and a hex key
(`UTR-TLS-PSK`, `UTR-TLS-PSK-CONFIG`). The `modbus-tcp` driver lists `tls` and `tls-psk` with
port 802, the Modbus/TCP Security port, and upstream's own integration test connects with
`modbus-tcp:tls://host:port?tls.verify=false` (`MB-TCP-TRANSPORTS-TLS`).

Stonecrab's transports are deadline-bounded byte pipes with one owner and no background work
(ADR-0014); the connection guard reopens a lost transport and its supervisor reads the idle
stream (addenda S18, S19 and S57 of ADR-0014). `deny.toml` bans `openssl`, `log`, `time`
and first-party use of `once_cell`. The core error kinds already distinguish `Trust` (peer
identity or certificate validation failed) from `Authentication` (credentials were
rejected).

rustls offers TLS 1.3 and 1.2 with a pluggable crypto provider; it supports pre-shared keys
only for session resumption, not the external pre-shared keys TLS-PSK needs.

## Decision
1. **Crates and feature.** A `tls` Cargo feature, off by default and not part of `full`,
   enables `transport`, `rustls` 0.23 (`default-features = false`, features `ring`, `std`,
   `tls12`) and `tokio-rustls` 0.26 (`default-features = false`, features `ring`, `tls12`).
   The `logging` features stay off (`log` is banned), and so does `aws-lc-rs`, rustls's
   default provider. The provider is `ring` 0.17, which compiles C and assembly with `cc`, so
   a build with `tls` needs a C compiler. The `modbus` feature does not enable `tls`;
   `modbus-tcp:tls` needs both. rustls depends on `once_cell` unconditionally, so `deny.toml`
   names `rustls` as a second wrapper of that ban (the ban on first-party use stands). The
   test certificate authorities are made with `ring` as a dev-dependency (decision 11).
2. **Public types.** `stonecrab::transport::TlsSettings` (two public fields, `trust_store`
   and `keystore`, `#[non_exhaustive]`) and `TlsFactory`, a `TransportFactory` built by
   `TlsFactory::new(&TlsSettings)`. The transport it opens is private and reached as a
   `Box<dyn Transport>`. `ModbusOptions` gains `tls: Option<TlsSettings>` under the feature.
3. **Trust model.** Trust comes only from the caller: `trust_store` is a PEM file of one or
   more certificate authorities, and a server certificate must chain to one of them (rustls's
   WebPKI verifier, without revocation checking: no CRL or OCSP). There are no web PKI roots,
   no platform certificate store and no way to turn verification off. `keystore`, when set,
   is a PEM file holding the client certificate chain (leaf first) and its private key
   (PKCS #8, SEC1 or PKCS #1, unencrypted); it is what a server that asks for a client
   certificate gets (mutual TLS). `TlsFactory::new` reads both files once, with blocking reads
   of at most 1 MiB each, and builds one rustls client configuration that every open of the
   factory shares; the Modbus driver builds the factory in `connect`, so a reconnect does not
   read the files again and may resume the TLS session. The reads run on the caller's thread
   before the connect deadline starts and have no time bound of their own, so the two paths
   must name regular local files: a FIFO or a stalled network mount would block the caller.
4. **Server-name verification.** The host of the `Endpoint::Tcp` is the server name: a DNS
   name or an IP address, which the certificate's subject alternative names must contain (the
   common name is not consulted). It is also sent as SNI when it is a DNS name. A host that can
   be neither is `InvalidConfig` before any I/O.
5. **Versions and cipher suites.** TLS 1.3 and TLS 1.2, nothing older, and the defaults of the
   ring provider: TLS 1.3 AES-256-GCM, AES-128-GCM and ChaCha20-Poly1305; TLS 1.2 ECDHE with
   ECDSA or RSA and the same three AEADs; key exchange X25519, P-256 and P-384. Every suite is
   an AEAD with an ephemeral key exchange. No `tls.version` option is offered yet.
6. **Keys stay secret.** The keystore's key goes from the file's bytes into rustls's signing
   key and is never formatted: `TlsFactory`'s `Debug` shows nothing of the configuration, the
   transport's shows its peer and state only, and `TlsSettings`'s shows the trust-store path
   and only whether a keystore is set (`Some("[REDACTED]")`), so
   `ConnectionConfig::redacted_display` names the trust store but not where the private key
   lives. Refusals name the option (`tls.trust-store`, `tls.keystore`), never a path or file
   content, and a keystore that cannot be decoded is reported without its source error, which
   could quote the text. Session keys are never logged (rustls's key log stays the no-op one)
   and `log-session-keys` is not offered. No password is taken, so no secret ever sits in a
   connection string. As for `Secret`, no zeroization is claimed beyond what rustls does.
7. **The connection string.** `modbus-tcp:tls://host[:port]?tls.trust-store=<file>
   [&tls.keystore=<file>]`, default port 802 (upstream `DEFAULT_TLS_PORT`), with every other
   `modbus-tcp` parameter as for `tcp`. The two key names are upstream's. `tls.trust-store`
   is required on `tls`, `tls.keystore` alone is refused, an empty file name is refused, and
   both are refused on `tcp` and `udp`. Every other `tls.` key (`verify`,
   `ignore-common-name`, the passwords and types, `version`, `log-session-keys`) and every
   `tcp.` key stays `InvalidConfig` as an unknown key, with a hint naming the two that exist.
   The files are read by `connect`, not `parse_config`. A path cannot contain whitespace, `%`
   or `&`, which the connection-string grammar refuses.
8. **Error kinds.** A server certificate that fails verification (unknown authority, expired
   or not yet valid, issued for another name, a bad signature) is `Trust`. A fatal alert from
   the server that refuses the client's certificate or its absence (`certificate_required`,
   `bad_certificate`, `unknown_ca`, `certificate_unknown`, `certificate_expired`,
   `certificate_revoked`, `unsupported_certificate`, `access_denied`) is `Authentication`.
   Every other TLS or socket failure is `Disconnected`, a deadline that passes is `Timeout`,
   and an unreadable or unusable PEM file is `InvalidConfig`. One deadline covers the
   resolution, the TCP connect and the handshake. Under TLS 1.3 a server judges the client
   certificate after the client has finished its handshake, so that refusal arrives with the
   first `recv`, not at `open`. A server that closes the connection without sending its alert
   (pymodbus's asyncio TLS server does, decision 10) cannot be told from a lost link and is
   `Disconnected`; so is `handshake_failure`, which some servers send for a missing client
   certificate under TLS 1.2 and others for anything else.
9. **TLS-PSK refused.** `modbus-tcp:tls-psk://` is `InvalidConfig` with the reason: rustls
   has no external pre-shared keys (only resumption keys), and E11 rules out a second TLS
   stack. `UTR-TLS-PSK`, `UTR-TLS-PSK-CONFIG` and `MB-TCP-TRANSPORTS-TLS-PSK` stay deferred.
10. **Guard, reconnect and supervisor.** The TLS transport is a byte stream with the TCP
    transport's rules: `send` encrypts and flushes one frame and poisons the transport when
    cut short; `recv` is cancel-safe, because records read from the socket stay in the TLS
    session until their plaintext is delivered; a `close_notify` reads as `0` and a TCP close
    without one as `Disconnected`; `shutdown` sends `close_notify` and a FIN. The guard is
    unchanged. A failed open of kind `Trust`, `Authentication` or `InvalidConfig` leaves it
    `Failed` with that kind and no reconnect (its reopen retries `Timeout` and `Disconnected`
    only), because a certificate does not fix itself. An `Authentication` on an open link (the
    TLS 1.3 case) loses the transport like any other failure: the next request reopens with
    backoff until the reconnect budget is spent. The supervisor reads the idle stream as over
    TCP; TLS 1.3 session tickets are consumed by rustls and never reach it, and a server's
    `close_notify` or a reset is noticed there. The idle bound closes with `close_notify`.
11. **Tests.** The loopback tests run rustls's own server (tokio-rustls's acceptor) against
    the transport and the Modbus driver, with certificate authorities and Ed25519 keys made
    at run time by `crates/stonecrab/tests/support/tls_pki.rs` from `ring`, with DER written
    there from RFC 5280 and RFC 8410: no private key is committed. `rcgen` would do this but
    depends on the banned `time` crate. The deadline over a handshake that never completes is
    a unit test on the paused clock, with the TCP connection made on the real clock first
    (decision D21).
12. **Interop evidence.** pymodbus 3.8.6 in the existing image of `interop/pymodbus/` serves
    the register map over TLS (`StartAsyncTlsServer`, MBAP inside TLS, Python's ssl, which is
    OpenSSL 3.5) when `server.py` gets `--tls-cert`, `--tls-key` and `--tls-ca`, and requires
    a client certificate. `interop/pymodbus/tls.sh` makes a throwaway PKI with openssl inside
    that image (ECDSA P-256: an authority, a server certificate for `127.0.0.1` and
    `localhost`, a client certificate and key, an unrelated authority), copies the server's
    files into the container with `docker cp`, probes with a real read over mutual TLS, runs
    the three `interop_tls_` tests of `crates/stonecrab/tests/modbus_interop_tls.rs` and
    removes the PKI; no key is kept as evidence. It is the CI job `interop-tls`, the mise task
    `interop-tls` and the last step of `mise run interop`. The container listens on 8802,
    because its unprivileged user cannot bind 802; the default port is covered by the parse
    tests.
13. **The RTU and ASCII `tls` carriers stay refused.** They would ride on the same
    `TlsFactory` (an RTU or ASCII frame inside TLS on a gateway), but their option parsing and
    their row `MBS-TRANS-TLS` (`docs/parity/modbus-serial.md`) are outside this slice and no
    gateway needs them yet. The `tls.` keys are refused with them, on the serial and `tcp`
    carriers and in every build, so a `modbus-rtu:tcp://` or `modbus-ascii:tcp://` string
    that names a trust store is `InvalidConfig` rather than a plain TCP connection.

## Consequences
- `modbus-tcp:tls` reads, writes, pings and polls like `modbus-tcp`, over a session whose
  server is authenticated against the caller's authorities and which can authenticate the
  client by certificate.
- A build without `tls` is unchanged: no new crate in its graph, `modbus-tcp:tls` refused
  as before (the message now says that `tls` needs the feature), and `ModbusOptions` without
  the field.
- `ring` brings native code and a C compiler to builds with `tls`; a target `ring` does not
  support cannot use the feature.
- A caller with PKCS #12 or JKS stores, encrypted keys or separate certificate and key files
  converts them to the PEM forms of decision 3 first; a device whose certificate names
  neither its host name nor its address cannot be reached without changing that certificate,
  because there is no `ignore-common-name` and no `verify=false`.
- Over TLS 1.3, a connection whose client certificate the server refuses opens, and its first
  request fails (`Authentication` when the server sends its alert, `Disconnected` when it
  only closes).

## Alternatives considered
- **aws-lc-rs, rustls's default provider.** Rejected by E11 for ring; aws-lc-rs needs CMake
  and, on some targets, NASM, and is a much larger native build.
- **native-tls or OpenSSL bindings.** `openssl` is banned in `deny.toml`, and a platform TLS
  stack would make behavior differ by operating system.
- **Web PKI roots or the platform store (`webpki-roots`, `rustls-platform-verifier`).**
  Rejected by E11: a PLC carries a certificate from the plant's own authority, and trusting
  public authorities would only widen who can impersonate it.
- **`verify=false` with a warning, `ignore-common-name`.** Rejected: each turns
  authentication off in a way a configuration keeps forever, which is what upstream's own
  documentation warns about. The parity rows record the difference.
- **PKCS #12 stores with passwords, as upstream.** Would need another parser and put a
  password in the connection string or a second channel; PEM is what openssl and most PLC
  tooling write.
- **Separate certificate and key files.** One keystore file keeps upstream's single
  `keystore` name; `cat cert.pem key.pem` makes one.
- **`rustls-pemfile`.** Deprecated; `rustls-pki-types` parses PEM itself.
- **`rcgen` for the test certificates.** Depends on `time`, which `deny.toml` bans; the test
  support writes the few DER fields webpki needs instead.
- **A second TLS stack for TLS-PSK (Bouncy Castle's role upstream).** Rejected by E11.

## Compliance
- `crates/stonecrab/src/transport/tls.rs` (`TlsSettings`, `TlsFactory`, the private
  transport, the error mapping), the TLS sites of `transport/deadline.rs`, the shared
  `resolve_and_connect` of `transport/tcp.rs`; `crates/stonecrab/src/modbus/driver.rs`
  (transport codes `tls` and `tls-psk`, port 802, the factory choice) and
  `crates/stonecrab/src/modbus/options.rs` (`tls.trust-store`, `tls.keystore`).
- Tests: `crates/stonecrab/tests/transport_tls.rs` (round trip, versions, a plaintext peer
  and a TLS 1.0 server, untrusted, wrong name, expired, mutual TLS, a refused client under
  TLS 1.2 and 1.3, bounds, a peer's close with and without `close_notify`, the PEM
  refusals, the endpoint, the redacted `Debug`),
  `crates/stonecrab/tests/modbus_tls.rs` (reads, a write and ping over mutual TLS, an
  untrusted server, a refused client, the redacted display), the unit tests of `tls.rs`
  (the handshake deadline, the error mapping) and of `modbus/driver.rs` (port 802, the keys,
  TLS-PSK), and the ignored `crates/stonecrab/tests/modbus_interop_tls.rs` run by the
  `interop-tls` job.
- `docs/parity/transport.md` (`UTR-TLS`, `UTR-TLS-TRUST`, `UTR-TLS-PSK`,
  `UTR-TLS-PSK-CONFIG`) and `docs/parity/modbus.md` (`MB-TCP-TRANSPORTS`,
  `MB-TCP-TRANSPORTS-TLS`, `MB-TCP-TRANSPORTS-TLS-PSK`, `MB-CFG-TRANSPORT-OPTIONS`) record
  the rows with their test ids; `docs/DEPENDENCY_AUDIT.md` the new crates.
