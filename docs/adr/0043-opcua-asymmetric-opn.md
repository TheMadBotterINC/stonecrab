# 0043: OPC UA asymmetric OpenSecureChannel chunks

## Status

Accepted S105 design, 2026-11-04. Implements the next slice of the approved M3 plan;
full secure connection/lane wiring and remote trust call sites remain S106.

## Design

Add a private asymmetric chunk engine beside S102's symmetric engine, using S103 key
material, S104 certificate parsing and the shared policy/chunk-size table. Seal OPN
chunks with a chain/thumbprint header, signature and blockwise RSA-OAEP; receive through
one bounded decrypt/authenticate path before exposing any sequence or service bytes.

- New `crypto/asymmetric.rs`: one concrete engine, no provider interface or new settings.
  Own local key material and the provisioned peer certificate, with native OAEP handles.
  No message-mode argument: every secure-policy OPN is signed and encrypted. Refuse None,
  missing/non-RSA/out-of-policy peer keys and out-of-policy local keys during construction,
  before transport I/O. Keep diagnostics and native-key errors redacted.
- Reuse `Policy::asymmetric_sizes` and `SecuritySizes::max_body`. Sign the clear headers
  (including final wire size), sequence, payload and padding with PKCS1-SHA256 or PSS-SHA256;
  encrypt complete blocks with OAEP-SHA1 or OAEP-SHA256 as the selected policy requires.
  Cipher blocks over 256 bytes use the two-byte padding size. The signature length follows
  the sender key; the block size and padding overhead follow the receiver key, independently.
- On receive, bound wire bytes before allocation, parse only the clear header, require the
  configured policy, provisioned sender leaf and local receiver thumbprint, decrypt exact
  blocks and verify against the provisioned certificate. Verify padding including its high
  byte before parsing the sequence/payload. Authentication failures have one fixed typed
  error without native sources. Resource-limit errors remain distinct. Caller owns channel,
  request-id and sequence checks, just as for S102. Trust-list evaluation remains S106;
  accepting a different header certificate must never substitute the cryptographic peer key.
- Reuse the existing asymmetric header codec (widen its private parse visibility)
  and bounded DER element reader for concatenated certificate extraction. No framing layer,
  public API, dependency, feature, version, generated code or default changes. Preserve the
  existing public secure-connect refusal after validating asymmetric preparation in driver.
- Tests: independent native block decoding and fixed OpenSSL PKCS1 signature vector;
  native round trips for randomized OAEP/PSS, distinct/mixed RSA key sizes, exact send/receive
  limits, all padding residues, both bytes for RSA4096, tampered clear headers/ciphertext/
  signatures/padding, wrong keys, truncated/partial/oversized messages and malformed chains.
  Keep keys generated at runtime; no committed private material.
- Add an ignored native OPN-only test using the existing transport and first-party test
  harness, under the existing open62541 gate. Prove Basic256Sha256 SIGN and SIGN_ENCRYPT
  OPN exchange without claiming a secure session or Read. Extend the selected-test count
  and evidence. Add decrypt-then-parse fuzz coverage through the existing cfg(fuzzing)
  facade; use runtime-generated ephemeral key material, no production key fixture.
- Keep 100% handwritten core/OPC UA line coverage and unchanged exclusion/threshold policies.
  Commit in slices; run focused mutations, full checks/doctests, isolated features, fuzz,
  native interop, dependency/regen/parity/attributes/DCO/semver/package gates and hosted CI.
  Update package inventory, three parity rows and evidence before a normal signed-off merge.

## Sources

Pinned PLC4X `AsymmetricEncryptionHandler`, `BaseEncryptionHandler` and `protocol/chunk/Chunk`
at ea229918f5d07700e04d7d69ab1645da679cb700 were read for behavior, not copied into runtime.
The existing S102 engine is the implementation precedent. Native AWS-LC 1.19.0 adoption
tests already establish RSA signing and OAEP APIs.

OPC Foundation Part 6 defines the [security header](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.2.3),
[padding/footer](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.2.5.1) and
[verification order](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.7).
S106 retains independent-peer confirmation of the policy key bounds and sequence wrap rule.

## Validation follow-up

The full threaded unit run exposed the existing tracing-core 0.1.36 single-dispatch
callsite-cache race ([upstream issue 3611](https://github.com/tokio-rs/tracing/issues/3611)).
Keep a second, uninstalled `Dispatch` alive in each of the two log-capture tests so new
callsites consult registered subscribers rather than another thread's empty default.
Reproduce the cold-callsite case on a subscriber-less thread in the trust diagnostics
test. This changes only fixtures: no global subscriber, retries, dependency change or
relaxed assertion, and production logging is untouched.
