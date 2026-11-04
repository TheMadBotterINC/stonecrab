# S105 asymmetric OpenSecureChannel protection

Date: 2026-11-04. Implementation 5e0911c; final test/package/source baseline
7978b3a35db2aeceff3d64ae5a6c00a5d6532db9. ADR-0043 is the committed design.
Hosted validation [41779622898](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41779622898)
on 7978b3a passes all 33 executed jobs, including both Windows Rust versions, five-target
builds, coverage and full serial fuzz smoke. Only the scheduled job is skipped. Final
repository documentation preserves built source inputs.

## Behavior and boundary

The private asymmetric engine owns local key material and the provisioned peer certificate.
Construction requires a secure policy and usable RSA peer/local keys before transport I/O.
The driver performs this preparation, then retains its secure-connect refusal. Full secure
connections, MSG/CLO integration and remote trust-policy call sites remain S106. No new
dependency, feature, public API, version, tool pin or generated-code change is introduced.

All accepted secure policies sign and encrypt OPN, including SIGN mode. Basic256Sha256 and
Aes128_Sha256_RsaOaep use PKCS1-SHA256 and OAEP-SHA1; Aes256_Sha256_RsaPss uses PSS-SHA256
and OAEP-SHA256. Signature length follows the sender key; cipher/plain block sizes follow
the receiver key. The header carries the configured sender chain and the receiver leaf
SHA-1 thumbprint (a protocol identifier). Padding has a low size byte and, for keys over
2048 bits, a high byte. Signatures cover final wire size, headers, sequence, payload and
padding; RSA-OAEP encrypts complete blocks. Validated sizes are retained at construction.

Receive bounds actual input before copying and parses only the clear header before crypto.
It binds policy, provisioned sender leaf and local receiver thumbprint; no presented leaf
can replace the peer key. Decrypt exact RSA blocks, require full plaintext blocks, verify
the signature and padding, then parse the sequence/payload. Only the first DER certificate
selects the cryptographic peer; appended issuers never become trust anchors. Caller owns
channel/request/sequence checks and trust-policy evaluation. RSA policy bounds remain
independently UNVERIFIED until S106, as planned.

Malformed protected data, wrong identity/policy/thumbprint, partial blocks, OAEP/signature
and padding failures return one fixed Authentication error without provider/parser sources.
Input above its byte ceiling is ResourceLimit. This establishes error shape and parsing
order, without a full-path equal-timing or side-channel proof. Native provider errors are
projected to the existing redacted-error idiom; no otherwise unused provider interface or
coverage exclusion is added.

## Independent and adversarial evidence

Fixed public OpenSSL PKCS1 TBS/signature bytes from S104's public certificate exercise the
OPN verification policy path; no private key fixture is retained. Native block decoding
separately checks emitted signatures, full blocks and randomized OAEP/PSS round trips.
Tests use distinct keys, both directions, all policies/flags, every RSA2048 padding residue,
exact byte/capacity boundaries, mixed RSA2048/RSA4096 sender/receiver sizes and both padding
bytes. An actual generated RSA8192 client key proves local policy refusal.

Every wire-byte mutation and truncation rejects. Authenticated negative cases separately
change type, size, policy, thumbprint and a same-key/different-serial certificate; these
prevent signature failure from hiding a missing header check. A correctly re-signed size
plus one trailing ciphertext byte proves chunks_exact cannot silently drop a remainder.
Native OAEP short plaintext blocks, invalid padding and bad signatures also reject.

One initially re-signed padding fixture legitimately encoded a different, shorter payload;
the corrected invalid fixture uses an impossible high padding count. The first mutation
campaign found two missing independent header/remainder assertions. Final campaigns close
them with no new exclusion. Coverage also identified redundant size revalidation and six
unreachable native-error mapping closures; validated sizes now flow directly to send,
and error projection follows the existing keystore idiom. No gate or threshold changes.

The full threaded unit suite exposed tracing-core 0.1.36's
[single-dispatch cold-callsite race](https://github.com/tokio-rs/tracing/issues/3611).
A subscriber-less thread now deterministically reproduces it in the trust diagnostics
fixture: it fails without the workaround and passes with it. Both existing log-capture
tests retain a second, uninstalled Dispatch. No global subscriber, retry, relaxed assertion,
production logging change or dependency update. The full threaded run then passes.

The first hosted run 41778677318 aborts in the Windows current-Rust threaded suite
with STATUS_STACK_OVERFLOW; its nextest phase passes. The existing 128 KiB deep-write
fixture also performed connection setup on that worker. Setup now runs before entering
the named constrained worker, keeping its stack limit, 100000-level values and rejection/
abandonment assertions. This is test-only; the cryptographic implementation is unchanged.
Full local coverage/check/package gates pass again; both Windows Rust versions pass on the replacement hosted run.

## Local gates

- Strict full check: 2519 nextest cases; cargo test, 588 merged plus five other doctests;
  fmt, Clippy, rustdoc and no-default build pass. OPC-UA-only feature build: 553 library
  cases pass, including 379 OPC UA cases, without testkit/default features.
- Coverage: core 6978/6978; codec 245/245; sim 2307/2307; modbus 6699/6699;
  sensors 1840/1840; boards 511/511; handwritten OPC UA 9606/9606, all 100%.
  Transport 2877/2897 (99.31%); testkit 5271/5394 (97.72%). Floors/exclusions unchanged.
- Final focused mutation campaign on the implementation: 57 total, 50 caught, seven
  unviable; no survivors/timeouts. Config mutants.toml, gitignore true, four jobs, debug
  profiles off, 120-second timeout. Artifacts: /tmp/stonecrab-s105-mutants-verified/mutants.out.
  The preceding campaign also catches 50/seven unviable; the initial two-survivor campaign
  is superseded, not presented as passing.
- All 18 fuzz targets pass at least 60 seconds with zero artifacts and successful exits:
  [report](fuzz/2026-11-04-s105.md). New asymmetric target: seven seeds and 27841 executions.
  Ephemeral RSA key/SPKI substitution into a public certificate template supplies valid
  crypto paths without committed private material. Raw input, protected payload round
  trips, directed ciphertext mutations and both padding-size formats are exercised.
  The final fixture commits change no cfg(fuzzing) input. Fuzz Clippy/deny and policy-copy
  validation pass; no new dependency or lock-file change.
- All 59 isolated feature combinations plus no-default pass; manifests/locks restored.
  Regen/self-tests, dependencies/machete, examples, semver (202 pass/58 skip), parity/
  self-tests, lint-attrs and DCO pass. Three parity rows close: 249 total, 147 implemented,
  64 planned, 38 deferred; 373 implemented/automated-verified references globally.
- Fresh-CARGO_HOME package passes with 228 reviewed files, version 0.3.0, 893694 bytes;
  SHA-256 a5fa28e16e6d05babdde401177b1488c656c9b7ad234d807b79b9ebf597b1280.
  Embedded source SHA 7978b3a. Unpacked all-feature and OPC-UA-only checks pass.
- Full open62541 gate passes all ten selected Stonecrab cases, the public None example
  and the image's independent fixture checks. The new Stonecrab OPN-only probe passes
  Basic256Sha256 SIGN and SIGN_ENCRYPT, verifying reply signature, nonce, request id and
  channel/token/lifetime fields. It makes no secure session or protected Read claim.
  Source v1.5.9 at 05c2b389b65cc35884eb40d800301af1f3d130cf; linux/amd64 image
  sha256:9dddff410b047b889866601a70462bfa44e198e5976e77ba569bd53c0f310fae.
  Evidence is under target/interop-evidence/open62541. Peer, temporary keys and empty
  host PKI directory are removed. No hardware was available or claimed.

Next: S106, Basic256Sha256 secure session/lane wiring and remote trust verification.
