# 0044: Basic256Sha256 secure connections

## Status

Accepted S106 design, 2026-11-04, following the approved E15/E16/E21 decisions.
No new dependency, feature, generated schema or public API is needed. The two remaining
modern policies stay unavailable for public connections until S107; protected user tokens
remain S108, channel renewal S109. None retains its explicit unprotected behavior.

## Design

Frame bounded wire chunks before applying the existing asymmetric/symmetric engines,
and pass authenticated frames to the existing correlation/reassembly lane. Bind one
prepared client/server security context through OPN, optional secure discovery and
session activation, deleting the public Basic256Sha256 connect refusal.

- Add one private `wire.rs` adapter shared by channel/lane. It owns the existing
  FramedTransport with raw byte chunks and a concrete protection enum (plain, asymmetric,
  symmetric), retaining the injected Backend. No new provider interface. Errors from
  authentication stay PlcError::Authentication, not FrameError::Malformed. ERR remains
  a bounded transport-level error; unexpected unprotected traffic cannot bypass crypto.
- Extend OpcuaFramer with raw Bytes framing, sharing its header/size/buffer judgement
  with the existing typed Frame implementation used by fixtures and codec fuzzing. Both
  implementations obey unchanged-buffer-on-error. Add only a crate-private framer borrow
  to FramedTransport; no public transport/error API change or discarded coalesced bytes.
- Wire exposes exact outbound body capacity using the existing SecuritySizes table and
  crypto engines. Lane uses that capacity for preflight, each MSG chunk and CLO, retaining
  message/chunk/aggregate budgets, request matching, cancel safety and write non-replay.
  No background task, alternate lane, implicit downgrade or second TCP connection.
- One private Security value owns the existing Asymmetric engine (shared by Arc) and
  Verifier. Prepare bounded files and validate the provisioned bootstrap before network
  I/O. Parse bounded concatenated DER chains for actual trust calls, never promote a
  presented issuer. Bind discovered/CreateSession leaf DER to the provisioned peer;
  tls.verify=false skips trust acceptance, not key/certificate/signature binding.
- Generalize channel establishment to the selected policy. HEL/ACK remains plain, OPN
  uses Asymmetric even in SIGN mode, and authenticated response parameters/nonces are
  checked before deriving directional keys and installing symmetric protection. Preserve
  None entry points still used by fixtures, with one shared implementation. Do not activate
  the other RSA policies publicly ahead of S107.
- Session uses certificate SAN application URI and the client certificate chain under
  secure policy. GetEndpoints runs only when secure plus discovery=true, on that same
  protected channel; select without policy/mode downgrade and verify its certificate.
  Verify server SignatureData URI and signature over client certificate plus fresh session
  nonce, then verify the returned certificate before any client signature/token is sent.
  ActivateSession clientSignature covers server certificate plus server nonce. Anonymous
  userTokenSignature stays empty; encrypted username/X509 token work stays S108.
- Correct the roadmap wording: CreateSessionRequest has certificate/nonce but no client
  signature field (both generated schema and Part 4); the server signs CreateSessionResponse
  and the client signs ActivateSessionRequest. Check session server nonces before use.
- Keep the fail-closed checked sequence exhaustion behavior unless independent evidence
  establishes a safe interoperable wrap rule; record the legacy threshold and policy key
  bounds as verified or explicitly UNVERIFIED. No simulated 4-billion-message evidence.
- Extend deterministic first-party responder tests with runtime PKI, both modes, all-chunk
  protection, certificate/signature/nonce substitutions, discovery on/off, trust rejection
  before ActivateSession and uniform tamper failures. Exercise the actual wire adapter,
  cancellation/resource limits, fragmented/coalesced packets and multi-chunk reads/writes.
- Extend the exact-count open62541 gate for public secure connect/read/write/close in SIGN
  and SIGN_ENCRYPT, chunked encrypted reads and trust/client-certificate/policy refusals.
  Temporary PKI remains runtime-only, removed on stop; no hardware claim. Extend crypto/
  wire fuzz coverage using existing harnesses where possible.
- Retain 100% handwritten core/OPC UA line coverage and all existing exclusions/thresholds.
  Commit in coherent slices. Run focused mutations, full strict tests/doctests, isolated
  features, fuzz, regen/dependencies/machete/examples/semver/parity/attrs/DCO, fresh package
  and unpacked builds, native interop and hosted CI before the normal signed-off merge.
  Update the reviewed package inventory, nine S106 parity rows, handoff and evidence.

## References

Pinned PLC4X SecureChannel.java (CreateSession verification, ActivateSession and discovery)
at ea229918f5d07700e04d7d69ab1645da679cb700 was read for behavior, not copied.
Existing channel, lane, S102/S104/S105 engines are the implementation precedents.

OPC Foundation Part 4 defines [CreateSession parameters](https://reference.opcfoundation.org/specs/OPC-10000-4/5.7.2.2)
and [ActivateSession parameters](https://reference.opcfoundation.org/specs/OPC-10000-4/5.7.3.2).
Part 6 defines the [legacy sequence rule](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.2.4).
