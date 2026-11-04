# S107 remaining OPC UA security policies

Date: 2026-11-04. Source baseline bfa151c99a8233e2d65dde807ed4096b93dac925.
ADR-0045 was committed before implementation. Hosted validation:
[CI 41793128103](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41793128103).
All 32 executed jobs pass. The scheduled job and first-branch DCO are skipped by
workflow conditions; local DCO verifies every source commit.

## Behavior and boundary

Aes128_Sha256_RsaOaep and Aes256_Sha256_RsaPss now connect through the same protected
wire/channel/session implementation as Basic256Sha256. The production change deletes
the temporary Basic256Sha256-only guard and clarifies the deprecated-policy error.
No algorithm duplication, production abstraction, fallback, second socket, dependency,
feature, generated-code change, public API or version change is introduced.

All three policies support SIGN and SIGN_ENCRYPT, secure discovery, anonymous sessions,
chunked reads/writes and close. Existing descriptors and provider-backed algorithms
supply AES128 with 16-byte encryption keys and AES256 with 32-byte keys; both have 32-byte
HMAC/signing keys/nonces and 16-byte IVs. AES128 uses PKCS1-SHA256/OAEP-SHA1; AES256/PSS
uses PSS-SHA256 with 32-byte salt and OAEP-SHA256, whose plaintext block is RSA key bytes
minus 66. OPN remains signed/encrypted even in SIGN mode.

Existing certificate/trust/nonce/application-proof checks run for every policy before
activation. Basic128Rsa15 and Basic256 remain deferred under E16: InvalidConfig names
deprecation and unavailable SHA1withRSA signing; endpoints offering only those policies
are not selectable. No legacy signing backend or opt-in bypass is added.

Protected user identities remain S108 and caller-driven channel renewal S109. Native
interop confirms RSA2048 and 8192-byte buffers for every modern policy. Other independent
RSA boundary sizes and sequence-wrap behavior remain UNVERIFIED. No hardware claim.

## Adversarial and independent evidence

The existing secure responder takes a concrete selected policy for OPN, keys, endpoints
and application proofs. Its PSS activation verifier explicitly requires a 32-byte salt.
Three named public-driver tests exercise both modes/discovery settings, certificate chain,
application URI/nonces/signatures, read/write/read-back and close on one transport.
The refusal and 1000-item chunked/coalesced exchange tests now cover all three policies:
wire corruption, invalid signatures/algorithm URIs, substituted certificates, discovery
with trust disabled, invalid/repeated nonces, bootstrap trust/expiry, EOF and resource limits.
A client chain proves the signature covers its exact transmitted bytes.

Named AES derivation tests check independent Python hmac/hashlib stream vectors and the
explicit key/IV lengths in both directions. OAEP-SHA256 sizing is asserted independently
for RSA2048/RSA4096, exact chunk boundaries and the 66-byte overhead. Existing native
provider tests reject wrong PSS salt, OAEP hash and signature padding; no parser-only policy claim.

The open62541 gate selects exactly 14 tests. Separate named AES128/AES256 tests connect,
read a 10000-item array across protected chunks, write/read back and close in both modes,
with discovery on/off. All three policies reject untrusted/expired client certificates,
empty server trust, untrusted/mismatched/expired server certificates and the None-only
peer's BadSecurityPolicyRejected. The image self-check independently reads with each AES
policy in both modes. Basic256Sha256/None and the finite public None example remain green.
The same four loopback-only peer processes and explicit PKI export/removal list are used.
Final native container and temporary keys are removed. Runtime evidence remains under
target/interop-evidence/open62541/, never private keys or password files.

## Validation

- Full strict check: 2539 nextest cases and threaded cargo test pass; 588 merged plus five
  other doctests pass. Fmt, strict Clippy, rustdoc and no-default build pass. An initial
  concurrent package/doctest run reported missing cached dependency artifacts; a sequential
  rerun passes unchanged source. No source fix or gate relaxation was needed.
- Standalone OPC UA: 559 library cases, including 385 OPC UA tests, and strict Clippy pass.
- Core 6978/6978, codec 245/245, sim 2307/2307, modbus 6699/6699, sensors 1840/1840,
  boards 511/511 and handwritten OPC UA 10022/10022 lines are 100%. Transport 2878/2900
  (99.24%) and testkit 5272/5394 (97.74%) pass their existing floors. No new exclusion.
- Focused mutation campaign: 16 tested, 11 caught, five unviable, no survivors/timeouts.
  Config mutants.toml, explicit gitignore=true, four jobs, debug profiles off, 120-second
  timeout. Scope is preparation, policy-name parsing/descriptors and asymmetric algorithm
  selectors/verification, selected by an explicit source-range diff. Artifacts:
  /tmp/stonecrab-s107-mutants/mutants.out/. No configuration exclusion changes.
- All 18 fuzz targets pass at least 60 seconds with zero crash artifacts:
  [recorded report](fuzz/2026-11-04-s107.md). Fuzz Clippy with cfg(fuzzing), deny and the
  unchanged root/fuzz policy-copy comparison pass.
- All 59 isolated feature combinations plus no-default pass; manifests/locks are restored.
  Regen/self-tests, dependencies and D23 trees, machete, examples, semver (202 passed/58
  skipped), parity/self-tests, lint attributes and local DCO pass.
- Fresh Cargo-cache package verification passes with the unchanged reviewed 230-file
  inventory and no fixtures/testdata. Unpacked all-feature and standalone OPC UA builds pass.
- Two parity rows close: 249 total, 158 implemented, 53 planned, 38 deferred. Deprecated
  policy rows retain E16 status and gain endpoint-refusal evidence. No release/tag/version.

## Hosted and merge evidence

Hosted source CI 41793128103 on bfa151c99a8233e2d65dde807ed4096b93dac925
passes all 32 executed jobs: both Rust versions on Windows/macOS/Linux, coverage,
five-target builds, native OPC UA interop and full serial fuzz smoke included. The
scheduled job and first-branch DCO are skipped as configured. Local DCO verifies all
signed-off source commits and is repeated over the final merge range.

The final root-only evidence/handoff commit preserves hosted-tested product/build inputs.
Merge uses --no-ff --no-squash --signoff and pushes master. No release/tag/version change.
