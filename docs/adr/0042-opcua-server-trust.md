# 0042: OPC UA server trust and fail-closed preparation

## Status

Accepted S104 design, 2026-11-03, implementing E16/E21/E22 already approved by the
maintainer. Full validation precedes the normal signed-off merge. Protected OPN remains
S105; verification of remotely presented certificates at handshake call sites is S106.

## Design

Load bounded local certificate files at the connect edge and select one private verifier:
explicit permissive, trust store, exact pin, or reject, in that order. Reuse webpki for
chain validation and native RSA verification; add only the bounded certificate-property
reader needed for validity/EKU checks on exact trusted certificates, deleting the old
test-only signature adapter when its production replacement lands.

- `options.rs`: accept `tls.verify` (true by default), `tls.trust-store`, and its type
  (`pem` by default, or explicit `der`); reject all other types and any trust-store
  password with fixed reasons and no echoed values. Debug exposes the verification opt-in.
  Keep the legacy reversed-sense option rejected. None never loads these files.
- New private `crypto/trust.rs`: Verifier is an enum with four real strategies, no new
  provider interface. Preparation returns the bootstrap certificate and chosen verifier;
  read server-certificate-file once (auto DER/PEM, exactly one certificate) and the selected
  trust store (PEM certificates or one DER). Each file has a 1 MiB plus one detection-byte
  read ceiling. Refuse unsupported PEM sections, malformed/empty/oversized files and I/O
  failures as InvalidConfig without paths, contents or sources. Pinning owns a bounded
  copy of the bootstrap certificate, avoiding a second file read. A permissive strategy
  skips the unused trust-store file, but never skips bootstrap structure/RSA requirements.
- Strict verification: empty/untrusted/malformed/expired/not-yet-valid chains are Trust.
  Exact-DER store matches are explicitly trusted after leaf validity, RSA bounds and
  serverAuth EKU-if-present checks. Otherwise build a webpki path using only configured
  anchors and the supplied intermediates; never turn discovered/presented roots into
  anchors. Filter expired/not-yet-valid anchors, and validate intermediate dates via
  webpki. No operating-system/public roots, network fetching, CRL/OCSP or hostname policy
  is introduced by this slice. Exact pin comparison delegates to AWS-LC's constant-time
  contents comparison (length remains public), followed by the same leaf checks.
- Permissive mirrors upstream's trust bypass, including an empty chain; independent
  bootstrap/key/signature parsing still applies before cryptographic use. Debug and
  structured tracing name the selected strategy; permissive preparation records
  tls.verify=false at warning level. No public Diagnostics field or API is added.
- New private `crypto/certificate.rs`: webpki owns complete X.509 structural parsing;
  BoundedReader walks only canonical DER lengths, TBS validity and EKU for direct trust.
  UTCTime and GeneralizedTime use PlcDate/PlcTimeOfDay and checked SystemTime conversion;
  no private webpki error-order dependency or treating UnknownIssuer as validation success.
  Promote the existing RSA PKCS1-SHA256 adapter and add PSS-SHA256 using the published
  rustls-pki-types algorithm identifiers. Native public-key parsing and the existing
  2048..4096-bit policy bounds produce Trust errors; independent-peer confirmation of
  these bounds remains UNVERIFIED until S106. Reuse the bound in asymmetric size checks.
- `driver.rs`: require a server-certificate-file path before client-key or transport I/O
  for every secure policy, even with a trust store or verification disabled. Then load
  client keys and prepare trust before the existing secure-connect refusal. Tests prove
  no transport open and no None fallback. Adapt the S103 missing-client-key test to supply
  a syntactic bootstrap path; it still fails before that file is opened.
- Extend the shared test PKI with optional EKU, PSS signatures and GeneralizedTime, keeping
  all keys generated at runtime. Port cases from all four pinned upstream verifier test
  classes (reject/permissive/pin/store), adding intermediate chains, anchor/leaf expiry,
  future dates, exact matches, pin mutation, EKU, PSS, RSA bounds and diagnostic redaction.
  New certificate-property fuzz target uses public certificate seeds only; update the
  connection-string oracle and seeds in the same implementation commit.
- Keep 100% handwritten core/OPC UA line coverage and existing coverage/mutation policies.
  Run focused mutations on both new modules, full checks/doctests, isolated features,
  fuzz, regen, dependency/machete, semver, parity/attributes/DCO, fresh package/unpacked
  builds, existing native None interop and hosted CI. Update inventory, parity and evidence.
  Close the eleven S104 rows, including the combined JVM-store divergence whose client-key
  half landed in S103; qualify private-verifier support separately from S106 call sites.

Sources: pinned PLC4X PinnedCertificateVerifier, TrustStoreCertificateVerifier and the four
verifier test classes at ea229918f5d07700e04d7d69ab1645da679cb700; RFC 5280 sections 4.1.2.5
and 4.2.1.12. Local webpki 0.103.16 exposes SPKI/SAN but not validity; KeyUsage::server_auth
requires serverAuth only when EKU is present. AWS-LC verify_slices_are_equal documents
constant time in contents, not lengths. No upstream source is copied into runtime code.
