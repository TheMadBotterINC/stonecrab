# 0045: Remaining OPC UA security policies

## Status

Accepted S107 design, 2026-11-04, following E16 and the S106 secure connection design.
No new dependency, feature, generated schema, production abstraction or public API.
Protected user identities remain S108 and channel renewal S109. No release/version/tag.

## Design

Remove the temporary Basic256Sha256-only guard from Security::prepare and route both
modern AES policies through the existing policy-bound wire/channel/session path. Extend
the existing secure responder and native public-driver tests across the actual policy
variants, retaining exact algorithm selection, peer/trust binding and no downgrade.

- Production change is confined to deleting the policy guard in crypto.rs and clarifying
  the deprecated-policy error in options.rs. Existing descriptors supply AES128 encryption
  keys of 16 bytes, AES256 keys of 32 bytes, 32-byte HMAC/signing keys/nonces and 16-byte IVs.
  Existing Asymmetric uses PKCS1-SHA256/OAEP-SHA1 for Aes128_Sha256_RsaOaep and
  PSS-SHA256/OAEP-SHA256 for Aes256_Sha256_RsaPss. PSS salt is 32 bytes, OAEP plaintext
  capacity is RSA key bytes minus 66. Do not add policy branches to driver/channel/session.
- Generalize tests/support/opcua_secure.rs with a concrete SecurityPolicy argument for URI
  and peer construction; retain the selected policy in the peer for OPN, symmetric keys,
  endpoints and session proofs. Native application proof signing/verifying must also
  select PSS for the PSS policy. No compatibility wrapper/default-policy adapter.
- Migrate current fixture callers. Share the existing public-driver acceptance assertions
  between three named policy tests, then exercise AES policies in tampered packet,
  signature/nonce/certificate refusal, discovery and 8192-byte multi-chunk exchanges.
  Remove obsolete AES-refusal assertions; keep mismatched channel/security rejection.
- Add independent expected-size derivation assertions for both AES policies and the
  RSA-OAEP-SHA256 overhead. Retain native provider salt/MGF/hash checks. Deprecated
  Basic128Rsa15/Basic256 continue to parse as typed InvalidConfig, naming deprecation and
  unavailable SHA1withRSA signing; reject endpoints offering only these without selecting
  None or another policy. Both parity rows remain deferred under E16.
- Extend the existing named Basic256Sha256 native test through a shared assertion routine
  and add two named AES native tests. Both modes, discovery on/off, chunked 10000-item
  reads, writes/read-back and close must pass against pinned open62541. Expand native
  certificate/empty-trust/None-only-policy refusals to both modern policies. Exact native
  EXPECTED_TESTS rises from 12 to 14. PKI export/removal and loopback-only peers stay intact.
- Update policy documentation, two implemented parity rows (249 total, 158 implemented,
  53 planned, 38 deferred), E16 record, reviewed package list if needed, evidence and handoff.
  Independent RSA boundary sizes and sequence wrap remain UNVERIFIED beyond observed
  RSA2048/8192-byte peer evidence. Do not infer hardware verification.

## Validation and delivery

Commit the plan before implementation, then commit coherent implementation/test slices.
Retain 100% handwritten core/OPC UA line coverage and all thresholds/exclusions. Run full
strict tests/doctests/docs, standalone OPC UA, focused mutations with mutants.toml and
explicit gitignore=true, all 18 fuzz targets for at least 60 seconds, fuzz policy/Clippy,
59 isolated features, regen, dependency/machete, examples, semver, parity/attributes/DCO,
fresh package and unpacked builds, and all 14 native tests plus the image self-check.
Feature tooling edits manifests/lock files temporarily: run it in isolation before other
Cargo consumers or mutation copies. Commit stable source before package verification.
Hosted CI must pass before a normal signed-off --no-ff --no-squash merge and push to master.
Final root-only evidence changes preserve hosted-tested product/build inputs.
