# S108 protected OPC UA identity tokens

Date: 2026-11-05 UTC (implementation started 2026-11-04 local).
Product inputs: 1ced484. ADR-0046 precedes implementation. Hosted validation and final
merge evidence are recorded below after the source run completes.

## Behavior and boundaries

The existing Credentials channel now activates protected username and X.509 identities.
There is no new public API, dependency, feature, version or tag. X509IdentityToken (327)
joins the generated closure (29 cases); independent complete/truncated vectors, Known
ExtensionObject dispatch tests and two corpus seeds cover it. Generated source is
regenerated, with no upstream edits; disabled-service tests now use the disabled id 940.

Username encryption follows the effective token policy: explicit URI, otherwise endpoint
inheritance. Four-byte LE unsigned password+nonce length, UTF-8 password and server nonce
are OAEP-encrypted with the selected modern policy. Legacy-format passwords are bounded
to 64 UTF-8 bytes; longer ones fail ResourceLimit pending the enhanced format. Independent
native decryption checks exact bytes, randomization and multi-block RSA2048 output for
all policies, passwords including empty/Unicode/NUL and 64-byte boundaries, and nonces up
to 128 bytes. None/bad nonces/malformed certificates refuse. The existing AES256/PSS
OAEP URI's incorrect '#' delimiter was caught by the native login test and corrected to
`http://opcfoundation.org/UA/security/rsa-oaep-sha2-256` per the OPC Foundation identifier.

X.509 user files are separate DER/PEM certificate and unencrypted RSA PKCS#8 PEM key,
each at most 1 MiB. They load before transport I/O, require matching RSA2048–4096 keys,
and need no user application URI. The user token carries only its leaf and signs exact
server certificate+nonce with its own key and token-policy signature. Application proofs
remain separate. Tests verify separate keys and differing channel/token policies; malformed,
missing, mismatched, duplicate/encrypted/non-RSA/oversized files and out-of-policy keys
fail with redacted InvalidConfig. Owned handles remain usable after file removal.

Anonymous/UserName userTokenSignature is empty, accepted by open62541. Plain username
requires explicit allow-insecure-credentials=true and emits only the existing static
warning. SIGN with token None and X.509 token None are refused. Unknown token policies
never downgrade. Identity refusals are Authentication with device sources retained.

Before protected token construction on None, require server-certificate-file and validate
its returned peer using the token policy and configured verifier. Exact leaf binding
remains required with tls.verify=false. Deterministic public-driver cases independently
decrypt/verify positive tokens and refuse missing bootstrap, certificate substitution,
empty/malformed certificate, unknown trust, invalid/reflected/replayed nonces before
activation (or after the activation reply for a replayed activated nonce).

## Native evidence and limitation

The open62541 v1.5.9 gate selects exactly 17 tests. Public username and X.509 sessions
connect/close on all modern policies in SIGN/SIGN_ENCRYPT, with discovery on/off.
Wrong passwords and untrusted user certificates become Authentication. Existing 14
channel/session/discovery/chunked-read/write/refusal tests and the finite example pass.
The native image's own self-check passes. Successful native protected identities use
RSA2048, separate application/user keys and runtime PKI; no hardware claim.

A fifth loopback-only process disables plain passwords on None. Native pinned
ua_services_session.c:617–638 creates session authentication state only when CreateSession
supplies an application certificate, and 671–679 returns that state's certificate.
This adapter sends the application certificate on secure channels only. The native None
case thus returns no usable session certificate and fails Trust before password/user proof
submission. It is a named fail-closed test, not a claim of native protected-None success.
Deterministic positive None evidence is separate. Use a secure channel with this peer;
application proofs on None remain an explicit interop limitation. No upstream/server patch
was made to accept the client. Final container and exported PKI are removed.

## Local validation

- Full strict check: 2549 nextest cases and threaded cargo test pass; 588 merged plus five
  other doctests pass. Fmt, strict Clippy, rustdoc and no-default build pass.
- Standalone OPC UA: 564 library cases, including 390 OPC UA tests, and strict Clippy pass.
- Core 6978/6978, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 10246/10246 lines are 100%. Transport 2878/2900
  (99.24%) and testkit 5271/5394 (97.72%) pass existing floors. No changed exclusions.
- Focused mutations: 22 tested, 19 caught, three unviable, zero survivors/timeouts.
  Config mutants.toml, explicit gitignore=true, four jobs, debug profiles off, 120-second
  timeout. An explicit source-range diff scopes token password/nonce limits, policy
  inheritance/equality, server-validation requirements and insecure-token refusal.
  Artifacts: /tmp/stonecrab-s108-mutants/mutants.out/; no exclusion changes.
- All 18 fuzz targets pass at least 60 seconds with no crash artifacts:
  [report](fuzz/2026-11-05-s108.md). Fuzz cfg(fuzzing) Clippy, deny and root/fuzz policy
  comparison pass.
- All 59 isolated feature combinations plus no-default pass with manifests/lock restored.
  Regen/self-tests, dependencies and D23 trees, machete, examples, semver (202 passed/58
  skipped), parity/self-tests, lint attributes and local DCO pass.
- Fresh Cargo-cache package verification and reviewed 232-file inventory pass. Only the
  new token module and its tests increase the inventory; no fixtures/testdata. Unpacked
  all-feature and standalone OPC UA builds pass. Packaging follows completed doctests.
- Four parity rows close: 249 total, 162 implemented, 49 planned, 38 deferred.
  E16/E18 retain existing file/API decisions. Renewal remains S109.
