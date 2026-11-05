# 0046: Protected OPC UA identity tokens

## Status

Accepted S108 design, 2026-11-04, under E16/E18/E21/E22 and ADR-0034.
No new dependency, feature or public API. Channel renewal remains S109; no release/tag.

## Design

Resolve credentials before transport I/O, then construct the selected identity token only
after session certificate/nonce validation. Reuse policy-bound native RSA primitives for
password encryption and user signatures, leaving the existing channel/session lane intact.

- Add X509IdentityToken (327) to the existing generator closure, regenerate in its own gen
  commit, add independent complete/truncated vectors and fuzz seeds, and migrate disabled-327
  expectations to a genuinely disabled type. No hand edit of generated source or upstream mspec.
- Identity becomes owned for X509 user key material while username credentials stay borrowed.
  Preserve the Credentials::x509 certificate/private-key PathBuf API and URI credential refusal.
  Load separate DER/PEM certificate and unencrypted PKCS#8 PEM private-key files before network
  I/O. Reuse the existing bounded PEM reader, strict section parsing, native RSA/key matching
  and certificate parser; extract only shared parsing/matching from KeyMaterial. Add one
  private UserKey value for the user leaf and signing handle. User certificates need no
  application URI; application KeyMaterial retains its required SAN URI, chain and thumbprint.
  File errors/Debug never expose paths, contents or provider errors. Each file stays <=1 MiB.
- Move existing RSA encoding/OAEP selectors to SecurityPolicy inside crypto, with explicit
  None refusal, so OPN and token operations share the same concrete selectors. No new backend
  interface or generic crypto context. Add bounded OAEP token encryption with native handles;
  use the selected token policy, not an assumed channel policy. User signing uses that same
  policy's encoding and signs exact server-certificate plus server-nonce bytes.
- Effective token policy is its explicit URI, otherwise endpoint policy. Resolve only None
  or the three supported modern policies; unknown/deprecated algorithms fail without trying
  a weaker endpoint or identity. Username protected token encodes a 4-byte little-endian
  unsigned password+nonce length, UTF-8 password and server nonce, then OAEP-encrypts bounded
  blocks and sets the policy encryption URI. This slice implements the legacy secret format
  for passwords <=64 UTF-8 bytes; longer encrypted passwords fail ResourceLimit, naming the
  unsupported extended secret format instead of emitting an incorrect modern envelope.
- Anonymous and UserName have empty userTokenSignature; X509 sends only the DER user leaf
  (not its chain) and the user's signature. X509 with effective token policy None is refused.
  Application clientSignature remains the existing separate secure-channel proof.
- Plain username tokens require allow-insecure-credentials=true and emit the existing static
  warning only on that branch. Signed secure channels with token policy None are rejected.
  None/None without explicit opt-in and without a provisioned server certificate continues
  to reject before transport I/O. None may otherwise establish a session to learn the actual
  token policy; no secret is sent before that policy is known and checked.
- For a protected token on a None channel require server-certificate-file, use the token
  policy for RSA validation, and apply the same configured verifier plus exact bootstrap
  leaf binding to the CreateSession certificate before encrypting/signing anything. Extend
  Verifier::prepare with an explicit policy and add shared bound-certificate verification;
  secure channel verification uses the same method. tls.verify=false never bypasses binding.
  Unknown/expired/substituted server certificates and invalid/reflected nonces stop activation.
- Extend deterministic responders for selected token types/policies, independent native
  password decryption and user-proof verification, mismatched channel/token policies,
  certificate substitution on None, malformed user files, nonce/signature tampering, typed
  identity refusals, redaction and absence of fallback. Preserve existing None opt-in tests.
- Add exact-count native public-driver username/X509 tests across all three modern policies
  in SIGN/SIGN_ENCRYPT, wrong password/untrusted user refusals, plus protected tokens on None
  where advertised. Observe empty Anonymous/UserName userTokenSignature against open62541;
  record that behavior explicitly. Reuse runtime PKI/exports and remove keys on stop.
- Close four S108 parity rows, refresh existing identity rows/support docs, reviewed package
  inventory, E16/E18 record, evidence and handoff. Retain 100% handwritten core/OPC UA lines,
  current floors/exclusions and no hardware claim.

## Validation and delivery

Commit plan, generation and coherent implementation/test increments. Run strict full checks,
standalone OPC UA, coverage, focused mutations with explicit gitignore=true, 18 fuzz targets
for at least 60 seconds plus fuzz Clippy/deny/policy comparison, 59 isolated features, regen,
dependencies/D23/machete, examples, semver, parity/attributes/DCO, fresh package/unpacked
builds and native self-check/tests. Feature tooling edits manifests temporarily and runs in
isolation. Run fresh package and doctests sequentially to avoid shared artifact-cache races.
Hosted CI precedes normal signed-off --no-ff --no-squash merge and push to master. Final
root-only evidence changes preserve hosted-tested build inputs.

## References

Pinned PLC4X context/SecureChannel.java and EncryptionHandler.java at
 ea229918f5d07700e04d7d69ab1645da679cb700 were read for behavior, not copied.

OPC Foundation [legacy secret encoding](https://reference.opcfoundation.org/specs/OPC-10000-4/7.40.2.2),
[username format/algorithm selection](https://reference.opcfoundation.org/specs/OPC-10000-4/7.40.4),
[X509 identity](https://reference.opcfoundation.org/specs/OPC-10000-4/7.40.5),
[token policy](https://reference.opcfoundation.org/specs/OPC-10000-4/7.41) and
[signature inputs](https://reference.opcfoundation.org/specs/OPC-10000-4/6.1.8)
were checked. Newer enhanced secret/channel-bound formats are outside the pinned legacy
policy closure and are not silently substituted. Native peer confirmation owns interop claims.


## Native observations

open62541 accepts empty Anonymous/UserName userTokenSignature, and authenticates
username/X.509 across all modern policies/modes. S108 fixes the existing AES256/PSS
OAEP URI delimiter using the OPC Foundation
[algorithm identifier](https://profiles.opcfoundation.org/testsuite/5134).

Protected identities on None are deterministic-tested, with trust and binding checks.
Native open62541 v1.5.9 only instantiates the session authentication policy when
CreateSession supplies an application certificate (pinned ua_services_session.c:617–638),
then returns its server certificate (671–679). This adapter sends its application
certificate on secure channels only. The fifth native process therefore records a
fail-closed Trust refusal, not successful None authentication. Supporting application
proofs on None would widen the channel/session preparation boundary and is left as an
explicit interop limitation; successful protected-identity evidence uses secure channels.
No server or upstream source was modified to accept the client.
