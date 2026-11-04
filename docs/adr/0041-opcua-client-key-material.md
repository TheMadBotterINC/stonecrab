# 0041: OPC UA client key material and shared test PKI

## Status

Accepted S103 design, 2026-11-03, implementing the already accepted E16 key-store
decision. Validation must pass before the normal signed-off merge. Server trust remains
S104, asymmetric protection S105 and operational secure connections S106.

## Design

At the connect boundary, read one bounded PEM application-instance key store and pass its
bytes into a private crypto key-material parser using the existing native providers.
Move the existing test-only RSA certificate writer into shared test support and extend it;
delete its old location rather than maintaining two PKIs.

- `options.rs`: accept `tls.keystore` as a nonempty path and `tls.keystore-type=pem`
  (also the default). Reject every other type and any `tls.keystore-password`, including
  an empty password, with fixed explanations. Do not retain passwords or echo option
  values. These are application-instance keys, separate from X.509 user credentials.
- New `crypto/keystore.rs`: one file open, at most 1 MiB plus one overflow-detection byte,
  no metadata/read race or reread. A pure bounded parser requires certificates in supplied
  leaf-first order and exactly one unencrypted RSA PKCS#8 key. PKCS#1, SEC1, encrypted keys,
  unknown PEM sections, duplicate keys, malformed certificates and leaf/key mismatches
  are typed InvalidConfig. No key-format conversion, new dependency or ASN.1 parser.
  Reuse rustls-pki-types PEM decoding, webpki certificate/SPKI/URI access and AWS-LC
  RSA parsing and SHA-1. Keep native signing and decryption key handles, not a long-lived
  extra DER key copy. Secret redacts formatting; this makes no zeroization claim.
- Retain the leaf DER, concatenate all certificate DER in supplied order for the future
  SenderCertificate, and compute the leaf SHA-1 thumbprint as a protocol identifier only.
  Require one nonempty absolute ASCII application URI from the leaf SAN; no secure-policy
  fallback URI. No file paths, key bytes or provider/parser sources in errors or material
  Debug. Loading is not chain validation or trust acceptance; S104 owns those checks.
- `driver.rs`: a secure policy without a key store fails before transport I/O. A supplied
  store is loaded once before the existing secure-connect refusal. None does not open a
  key store. Successfully loading keys does not enable secure channels prematurely.
- `tests/support/opcua_pki.rs`: reuse the current runtime-generated AWS-LC RSA key and DER
  writer, fixed current/expired validity, CA-issued chain, URI SAN variants, an Ed25519
  negative fixture and temporary PEM files. No private keys or PKI files are committed.
  Existing native adoption tests consume the relocated fixture.
- Tests cover real native sign/decrypt with loaded handles, chain bytes and leaf thumbprint,
  missing/extra/wrong/unsupported keys, malformed and oversized input, read failures and
  exact bounds, URI extraction/refusal, config defaults/refusals, Debug/error redaction
  and connect-before-I/O behavior. Add keystore to mutation scope with no new exclusions.
  Preserve 100% handwritten core/OPC UA coverage; run full checks/doctests, isolated feature
  powerset, fuzz, regeneration, dependency/semver/parity/DCO gates, fresh package builds,
  native None interop and hosted CI before merge. Update inventory and evidence.
- Close the five key-store/certificate rows. Record the implemented key-store half of
  OPCUA-DIV-JVM-KEYSTORES while leaving its trust-store half planned through S104.
  Keep generated certificates deferred, and record E16 implementation by component;
  S104 trust stores and S107 deprecated-policy parity remain their own slices.

Sources: OPC UA Part 6 [application certificate profile](https://reference.opcfoundation.org/specs/OPC-10000-6/6.2.2)
(one application URI), and existing S101 native adoption tests and ADR-0032 PEM loading.
E16's accepted PKCS#8-only recommendation resolves the broader optional PKCS#1 proposal
in the S103 roadmap; no first-party wrapper is needed.
