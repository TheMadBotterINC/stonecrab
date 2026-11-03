# 0039: OPC UA crypto foundation and native build adoption

## Status

Accepted S101 design (2026-11-03), implementing E15 and accepting ADR-0035 after its
recorded five-target S82 probe. Implementation, audit and hosted validation must pass
before merge. No secure channel activation is enabled in this slice; S102–S108 retain
message protection, key loading/trust and secure sessions.

## Design and implementation plan

Keep cryptographic effects behind one private backend, and derive policy-specific keys
with pure bounded code using its HMAC primitive. Reuse the existing policy/size and
channel/session boundaries, replace direct driver entropy calls with the injected backend,
and keep secure policies refused before network I/O until their operational slices.

- Add the four accepted optional dependencies to opcua only: aws-lc-rs 1.19.0 without
  defaults, aws-lc-sys 0.47.0 with disable-prebuilt-nasm, rustls-webpki 0.103.16 with
  alloc and no provider, rustls-pki-types 1.15.1 with alloc. Workspace caret requirements
  and committed root/fuzz locks follow policy. Keep TLS on ring; no deny policy change.
  aws-lc-sys is named only to set a feature, and webpki's package/library name mapping
  gets the already-approved machete metadata. Existing getrandom supplies OS entropy.
- New private opcua/crypto.rs with crypto/policy.rs, crypto/derive.rs and crypto/backend.rs
  separates policy data, bounded derivation and effects. A two-method Backend trait
  supplies fill_random and piecewise HMAC; Native uses existing getrandom and AWS-LC,
  deterministic/failing test implementations provide the second real implementation.
  No generic crypto manager, public RNG API, certificate store or new runtime task.
- Extend S94 SecuritySizes with constructors for policy-derived symmetric and RSA sizes;
  keep one chunk-size formula. A single descriptor on existing SecurityPolicy holds
  URIs, algorithm choices, signature/key/block/nonce sizes and OAEP overhead for None
  and the three accepted secure policies. Deprecated policies remain refused. Move
  existing URI mapping onto that descriptor; do not maintain parallel policy tables.
- First-party P_SHA1/P_SHA256 implement the specified HMAC expansion, bounded to the
  maximum supported schedule/vector length and validating backend output lengths before
  growth. Derive client-sending material from server nonce as secret/client nonce as
  seed, and server-sending material from the opposite pair. Split signing, encryption
  and IV in that order using checked lengths. None needs no material or entropy. Secure
  nonces must satisfy the policy bound; tests verify exact roles, lengths and slices.
  Keep key/nonces redacted using the established Secret contract, without claiming
  zeroization (future key-store work must address secret lifetime explicitly).
- Driver owns Arc<dyn Backend>; private unit tests replace the field without adding a
  public constructor/setter. Session nonce is 32 bytes from this source. The None channel
  obtains its policy nonce through the same helper and stays empty without drawing
  randomness. Secure nonce helpers are exercised internally; protected OPN remains S105.
  Native entropy error conversion remains separately testable, with no fallback bytes.
- Re-create the S82 primitive checks that verify the adopted graph: HMAC/digest vectors,
  RSA OAEP/signature and AES-CBC capabilities, and providerless webpki with a test-only
  RSA signature adapter. Reuse/read S82 test constructions, adapting to checked arithmetic
  and the repository's no-cast/no-panic runtime rules; generate test keys at runtime and
  commit no private key. Runtime AES/RSA/channel and trust code remain later slices.
- Use published HMAC/P_SHA vectors and an independently generated deterministic OPC UA
  schedule fixture (OpenSSL/Python stdlib, provenance recorded). No claim that a secure
  peer session has occurred in S101. Tests cover injected failure at each expansion step,
  malformed primitive output, nonce lengths, directional separation, redaction and bounds.
- CI sets AWS_LC_SYS_USE_SYSTEM=0 globally and installs pinned NASM on Windows test legs.
  Keep existing cross compilers. Record all five D23 reverse trees and distinguish the
  exact absent-package diagnostic from real Cargo failures; existing allowed wrappers
  remain enforced by unchanged cargo-deny. Adopt draft dependency audit entries with
  actual graph/source observations and update downstream build requirements.
- A reproducible footprint script plus a small no-I/O example records clean build time,
  compiler/runtime maximum RSS, archives and linked binary sizes for modbus, modbus+opcua
  and modbus+tls+opcua in isolated target directories. CI runs the same comparison;
  record the feature/fuzz native build cost and keep E17's existing feature choice unless
  measured evidence requires revisiting it. No manufactured portability evidence.
- Preserve 100% handwritten core/opcua coverage; use the seam to exercise failures. Run
  the full check, isolated feature matrix, configured focused mutation checks, all fuzz
  targets, regeneration, dependency/machete, default/all semver, parity/attributes/DCO,
  fresh package and unpacked builds. Push the feature branch to run the Windows/MSRV/
  cross matrix and inspect hosted results before the normal signed-off merge/push.
  Only KEYDERIVATION and NONCE parity rows close, with operational-policy rows still
  owned by S106/S107. Update plan/status/support docs and record evidence and limitations.

Sources: accepted ADR-0035 and the pinned PLC4X SecurityPolicy/SymmetricKeys sources;
[OPC UA key derivation](https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.6),
[RFC 5246 expansion](https://www.rfc-editor.org/rfc/rfc5246.html#section-5), and S82's
recorded provider/probe tests. Historical S82 results are evidence of that probe, not a
substitute for S101 hosted results.
