# S106 Basic256Sha256 secure connections

Date: 2026-11-04. Source baseline d6b72bea5eecbd75632e3f190c82fc4a2c45b601.
ADR-0044 records the design committed before implementation. Source validation:
[hosted CI 41788054600](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41788054600).
All 33 executed jobs pass; the scheduled-only job is skipped. Final root documentation preserves
built source, manifests, tool pins, generated files and the reviewed package inventory.

## Behavior

Public Basic256Sha256 connections support SIGN and SIGN_ENCRYPT, anonymous sessions,
secure discovery, reads, staged writes and close. HEL/ACK stays plain; OPN is always
signed/encrypted, including SIGN. Every MSG/CLO chunk uses the selected symmetric mode.
The existing correlation/reassembly lane consumes authenticated frames; its exact body
capacity includes the security footer/block expansion before allocating sequences or sending.
No alternate lane, background task, implicit downgrade, extra socket, new dependency,
feature, public API or version change is introduced.

One prepared security context binds the local key and provisioned peer through OPN,
optional secure GetEndpoints and session activation. Bounded bootstrap/trust files are
prepared and validated before network I/O. Wire certificate chains are borrowed,
canonical concatenated DER, limited to 16 certificates and 1 MiB. Presented issuers never
become configured trust anchors. Every remote leaf must match the provisioned peer,
including with tls.verify=false. Malformed/tampered protected input has one redacted
Authentication error; transport EOF, deadlines and resource limits keep their categories.

CreateSession sends the local certificate chain, its SAN application URI and a fresh
32-byte nonce. CreateSessionRequest has no clientSignature field; the roadmap wording
has been corrected against the generated schema and OPC Foundation Part 4. Validate
the server signature algorithm/proof over the exact client certificate bytes plus nonce,
verify the returned server certificate, then send ActivateSession clientSignature over
server certificate plus server nonce. Anonymous userTokenSignature stays empty.
Invalid, zero, reflected or repeated server nonces reject. No client activation proof is
sent when server verification fails.

The other modern policies remain unavailable until S107. Protected username/X509 tokens
remain S108; operation-driven renewal remains S109. Checked sequence exhaustion still
fails closed. Native tests confirm RSA2048 and 8192-byte buffers; independent policy key
boundary sizes and sequence wrap remain UNVERIFIED. No hardware verification is claimed.

## Adversarial and independent evidence

Deterministic public-driver tests cover both modes, discovery on/off, one transport,
client SAN/chain/nonce fields, application signatures, read/write/read-back/close,
1000-item protected chunked requests and coalesced responses within 8192-byte buffers.
Both modes reject actual corrupted OPN/MSG ciphertext/signatures, clear channel ids and
headers with the same error. Invalid session signature URI/content, substituted discovery
or CreateSession certificates, OPN/session/activation nonces, expiry during the handshake,
pre-I/O untrusted/expired bootstrap, derivation failures, EOF and resource limits reject.
The responder verifies the client activation signature with the native provider.
A concatenated client chain proves the server signature covers its exact transmitted bytes.

The native gate selects exactly 12 tests against pinned open62541 v1.5.9. Public sessions
connect/read/write/read back/close in both modes with discovery both on and off, including
a 10000-item array split across protected chunks. Untrusted and expired clients and an
empty server trust list are refused. Their DeviceException retains the peer status:
BadSecurityChecksFailed for the untrusted/empty-list cases, BadCertificateTimeInvalid for
the expired client. The server log independently distinguishes BadCertificateUntrusted
and BadCertificateTimeInvalid. The expired client is explicitly on the trust list, isolating
validity rejection. Untrusted, mismatched and expired server certificates fail locally with
Trust. A Basic256Sha256 request to the None-only server returns the typed peer status
BadSecurityPolicyRejected, with no downgrade. The image's independent client self-check and the public finite None example pass.

Runtime PKI includes expired January 2020 certificates. Main, empty-trust, expired-server and None-only
processes share one disposable container, with four loopback-only ports. The explicit
PKI_FILES export/removal list includes every exported key. Final container and host PKI are
removed; evidence contains logs/image metadata, never key/password files. Native artifacts
are under target/interop-evidence/open62541/; no hardware or extra peer is implied.

Mutation testing exposed missing acceptance/refusal assertions at the exact aggregate DER
byte limit. Native first-party certificates now form a valid 1 MiB chain and a valid chain
one byte larger, distinguishing the comparison mutations. Early exploratory runs and a
copy failure involving changing ignored fuzz output are superseded by the clean final
campaign; no coverage/mutation exclusion or threshold changed.

## Local gates

- Full strict check: 2533 non-ignored tests, threaded cargo test, 588 merged plus five
  other doctests, fmt, Clippy, rustdoc and no-default build pass. Standalone OPC UA:
  556 library tests including 382 OPC UA tests pass; strict standalone Clippy passes.
- Final committed-source coverage: core 6978/6978, codec 245/245, sim 2307/2307,
  modbus 6699/6699, sensors 1840/1840, boards 511/511 and handwritten OPC UA
  10023/10023, all 100%. Transport 2878/2900 (99.24%); testkit 5272/5394 (97.74%).
  Existing floors and generated-code exclusions are unchanged.
- Focused mutations: 39 tested, 34 caught, five unviable, zero survivors/timeouts.
  Config mutants.toml, gitignore true, changed
  lines in frame/chunk/security/certificate-chain logic, four jobs, debug profiles off,
  120-second timeout. Artifacts /tmp/stonecrab-s106-mutants-clean-current/mutants.out/.
- All 18 fuzz targets pass at least 60 seconds with zero crashes/artifacts:
  [recorded concurrent report](fuzz/2026-11-04-s106.md). Existing frame/certificate
  harnesses now also exercise raw framing, complete-frame parsing and DER chains.
  The report's uncommitted change was confined to the ignored native refusal assertions;
  later import cfg/rustdoc changes preserve the fuzzed logic. Fuzz Clippy with cfg(fuzzing),
  deny and the unchanged policy-copy check pass.
- All 59 isolated feature combinations and the separate no-default build pass. Feature
  tooling restores manifests/lock files. Regen/self-tests, dependencies, machete, examples,
  semver (202 passed/58 skipped), parity/self-tests, lint attributes and DCO pass.
- Fresh Cargo-cache package verification passes with the reviewed 230-file inventory,
  no fixtures/testdata; unpacked all-feature and standalone OPC UA builds pass.
- Nine parity rows close: 249 total, 156 implemented, 55 planned, 38 deferred. The
  protected-token client-signature row and None-channel certificate/token divergence
  remain S108. No release/tag/version change.

## Hosted and merge evidence

Hosted source CI 41788054600 on d6b72bea5eecbd75632e3f190c82fc4a2c45b601
passes all 33 executed jobs, including both Rust versions on Windows/macOS/Linux,
coverage, five-target builds, native OPC UA interop and full serial fuzz smoke. Only the
scheduled job is skipped. The superseded run 41787427107 was cancelled for the additional
native None-only policy refusal case; the final run includes that case.

The final root-only evidence/handoff commit preserves tested product/build inputs.
Merge uses --no-ff --no-squash --signoff; no release/tag/version change.
