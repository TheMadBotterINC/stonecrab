# 0040: OPC UA symmetric chunk protection

## Status

Accepted S102 design, 2026-11-03, following ADR-0039. Implementation and validation must
pass before the normal signed-off merge. Operational secure connections remain S105/S106;
renewal scheduling and token expiry remain S108. No new feature, dependency or public API.

## Design

Extend the existing private crypto backend with provider-backed HMAC verification and
AES-CBC without padding, and protect complete MSG/CLO chunks with the S101 policy and
keys. Authenticate decrypted bytes before parsing any sequence header or payload, retain
at most two token-specific key pairs, and keep the current None lane unchanged until S106.

- `crypto/backend.rs`: add verify_hmac, encrypt and decrypt to Backend. Native delegates
  verification to AWS-LC hmac::verify (whose implementation uses its constant-time compare)
  and uses EncryptingKey/DecryptingKey CBC with the caller-derived IV. Policy key lengths
  select AES128/AES256. Update the real failing/deterministic test implementations; no
  default trait fallbacks. Native primitive vectors and invalid key/length cases exercise
  these production wrappers rather than relying only on S101's direct adoption checks.
- New private `crypto/symmetric.rs`: Symmetric owns policy, mode, channel id, current and
  optional previous token (ids plus redacted KeyPair). Its constructor refuses None,
  mode None and zero ids. Install derives both directions before changing state, rejects
  active-id reuse, retains only the preceding token and always re-derives from both nonces.
  Thus a changed nonce on renewal cannot retain old keys; no nonce-cache comparison or
  unbounded token map is added. Explicit previous-token retirement has no timer or task.
  Lifetime/grace policy belongs to S108; this private primitive is not connected to I/O.
- Seal uses client-sending keys and open uses server-sending keys. No production role
  knob: peer tests swap the already-derived pair in their private fixture. Header type,
  chunk flag, final on-wire size, channel id and token id are authenticated along with
  sequence/body/padding. Only sequence+body+padding+MAC are encrypted; the first 16 bytes
  remain clear. Sign has no padding/encryption. SignEncrypt uses the minimal 0..15 padding
  count, with PaddingSize plus repeated bytes all equal to that count. Zero count still
  includes its one size byte. Receive accepts consistent byte-sized OPC UA padding,
  including extra complete blocks, after authentication; this is not PKCS#7 padding.
- Reuse Frame::serialize (with final wire size) and Body::parse by exposing those existing
  methods only to sibling modules; do not add a second wire codec. Open first bounds the
  actual input, reads only the clear FrameHeader/channel/token, finds the token keys,
  decrypts the bounded copy, verifies MAC with the provider, checks authenticated padding,
  then calls Body::parse on the verified slice. No sequence/service fields reach callers
  on authentication failure. Existing SecuritySizes remains the sole body-capacity formula;
  independently checked final protected length must also fit the caller's ceiling.
- One fixed Authentication error without source/details for malformed protected framing,
  unknown channel/token, AES, MAC, padding and backend receive failures. Oversized actual
  input is ResourceLimit before allocation/provider work; local misuse/configuration is
  InvalidConfig and outbound bounds are ResourceLimit. Equal error messages/kinds do not
  claim uniform timing of the full receive path or independently prove CBC side channels.
- New tests under crypto/symmetric/tests.rs cover deterministic independent OpenSSL/Python
  vectors, AES128/256 and Sign/SignEncrypt, MSG/CLO and all flags, boundary padding/limits,
  whole-chunk tampering, malformed authenticated padding, decrypt/verify ordering, provider
  failure/malformed output, direction/reflection, token overlap/retirement/replacement,
  nonce renewal and atomic failed install, redaction and unchanged secure-connect refusal.
  Native MAC verification tests include every bad tag byte and length; no first-party
  equality comparison is used for MACs. New symmetric fuzz target uses public fixed test
  nonces only, bounded chunks and decrypt-before-parse; it is cfg(fuzzing) only.
- Close only CRYPTO-SYMMETRIC. CRYPTO-PADDING remains planned through S105 (its RSA half),
  with the AES tests and implemented scope recorded explicitly. Secure policy/mode runtime
  rows remain S106/S107. No protected-peer interoperability claim in S102.
- Keep 100% core/OPC UA line coverage, add the module to configured mutation scope, and
  run focused mutations without new exclusions. Run full checks/doctests, isolated feature
  powerset, all fuzz targets, regeneration, dependency/machete, semver, parity/attributes/DCO,
  examples, fresh package/unpacked feature builds and the existing native None peer gate.
  Update package inventory, evidence, plan and status. Push for hosted CI before merge.

Sources: pinned PLC4X SymmetricEncryptionHandler.java, BaseEncryptionHandler.java and Chunk.java at
`ea229918f5d07700e04d7d69ab1645da679cb700`; OPC UA Part 6 §6.7.2.5.1 and §6.7.7:
https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.2.5.1 . AWS-LC 1.19.0 local
hmac::verify implementation and S101's NIST CBC/HMAC adoption vectors.
