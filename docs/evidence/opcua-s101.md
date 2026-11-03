# S101 OPC UA crypto foundation

Date: 2026-11-03. Implementation validation in progress on feat/opcua-crypto-foundation;
hosted matrix, remaining local gates and the final merge are pending. This file will be
updated with completed results before merge. ADR-0039 records the committed plan.

Four optional dependencies adopt E15; default/full and version 0.3.0 are unchanged.
The private backend supplies OS entropy and piecewise AWS-LC HMAC, with deterministic
and failing test implementations. One SecurityPolicy descriptor feeds URIs, key and
nonce sizes, algorithm choices and the existing checked chunk-size formula. First-party
P_SHA1/P_SHA256 expansion is bounded to 256 bytes and validates each primitive output.
Client-sending keys use server nonce as secret and client nonce as seed, with the opposite
roles for server-sending keys. Signing, encryption and IV material are redacted by Secret;
no claim of zeroization is made. Secure policies remain refused before protocol I/O.

CreateSession draws 32 bytes through the injected backend before transport open. Policy
None OPN has a null nonce and consumes no entropy; secure-policy helpers draw 32 bytes.
Only KEYDERIVATION and NONCE parity rows close: 127 implemented, 84 planned, 38 deferred
of 249 OPC UA rows. Symmetric/asymmetric protection and certificate loading/trust remain
S102–S108, including real protected peer exchange. No hardware exists for validation.

## Vector provenance

- OPC UA Part 6 §6.7.6 defines the two directions and signing/encryption/IV ordering:
  https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.6 .
- Published NIST SHA-256 TLS PRF vector from OpenSSL 3.5.4:
  https://raw.githubusercontent.com/openssl/openssl/openssl-3.5.4/test/recipes/30-test_evp_data/evpkdf_tls12_prf.txt .
  The test concatenates label, client random and server random as the PRF seed.
- Independent fixture: Python stdlib hmac.digest, A=seed; repeatedly
  A=hmac.digest(secret,A,hash); append hmac.digest(secret,A+seed,hash), then truncate.
  SHA1/SHA256 use secret=b"secret", seed=b"seed", length=104. Directional SHA256 uses
  client_nonce=bytes(range(32)), server_nonce=bytes(range(32,64)), length=80, with the
  OPC UA roles above. Split at byte 32 and byte 64; AES128 splits at 32 and 48 instead.
- 27 native adoption tests from the S82 probe exercise AES-CBC NIST vectors, RSA OAEP
  SHA1/SHA256 boundaries/tampering/hash mismatch/randomization, PKCS1/PSS SHA256 signing,
  HMAC/digests and providerless webpki trust. Certificate writers/adapters are test-only;
  keys are generated at runtime and no private key is committed. PKCS1 conversion is not
  adopted; runtime loading belongs to S103.

## Validation so far

2,431 deterministic tests passed; core 6978/6978 and OPC UA 8326/8326 lines passed the
unchanged 100% gate before the final boundary/test hardening. Final rerun is pending.
Root dependency checks and machete pass with no new deny ignores/wrappers. D23 raw output
is in opcua-s101-dependency-trees.txt: existing once_cell/lazy_static paths remain allowed;
log/time/chrono are absent. Regeneration and parity self-tests pass. Fuzz stable clippy
passes with the updated lock and pinned cc 1.10.0; the fuzz smoke run is pending.

The first mutation campaign identified an entropy no-op survivor, fixed by asserting that
native fill changes the initialized nonce buffer (all-zero probability 2^-256); the final
configured campaign must exit zero. Boundary review also fixes ExtraPaddingSize to depend
on the encryption key exceeding 2048 bits, independent of OAEP plaintext overhead:
https://reference.opcfoundation.org/specs/OPC-10000-6/6.7.2.5.1 .

## Initial footprint (local, concurrent validation load)

scripts/feature-footprint.sh builds each configuration in a separate clean target dir
and runs the no-I/O feature_footprint registry example. This is a cost indicator, not a
network throughput benchmark or production memory estimate. GNU time reports a process
maximum RSS, not an aggregate of all compiler processes. Debug symbols remain enabled
by the repository release profile. Archive bytes sum rlib and native a files, including
embedded copies; they are not incremental deployment size. Runtime RSS covers driver
registry construction only, without sessions, channels or cryptographic traffic.

| Features | Clean release seconds | Compiler max RSS KiB | Registry max RSS KiB | Binary bytes | Archive bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| modbus | 13.19 | 454716 | 2444 | 5832136 | 162210914 |
| modbus,opcua | 29.35 | 622740 | 2544 | 15536440 | 255159648 |
| modbus,tls,opcua | 26.19 | 663844 | 2824 | 22737688 | 269172428 |

The first OPC UA comparison adds 16.16 seconds, 168024 KiB compiler maximum RSS,
100 KiB registry RSS, 9704304 binary bytes and 92948734 archive bytes. Scheduling and
warm OS cache make the third build faster than the second; no causal speedup is inferred.
CI repeats this workload as feature-footprint and preserves raw metrics. Final CI/native
feature/fuzz costs and E17 decision will be recorded after those jobs finish.

## Hosted portability fixes

The first hosted run 41635079341 (adb9912) built the native graph on Linux/MSRV and both
cross targets. Windows identified existing tests assuming a larger SystemTime range and
nanosecond precision. Overflow-clock arms now run only if the platform can represent that
instant, as the codec's existing range test does; Linux still covers the error path.
The pre-epoch picosecond expectation applies the base 100 ns tick before its refinement,
respecting Windows' 100 ns SystemTime resolution. Runtime timestamp code is unchanged.
See https://doc.rust-lang.org/std/time/struct.SystemTime.html#platform-specific-behavior .

The hosted native peer job also found ANSI color splitting qualified names in its grep
count. Listing now explicitly uses --color never, preserves the listing, and propagates
Cargo failure. The exact nine-test requirement is retained. The local forced-color
listing and the corrected hosted peer job must pass before merge.

The complete 66-mutant crypto campaign passes (46 caught, 20 unviable, zero survivors or
timeouts). Expansion now uses the policy digest length to calculate a fixed iteration
count, so corrupt/empty primitive output cannot make an output-driven loop hang.
No mutation exclusions or coverage exceptions were added.

All 15 fuzz targets completed 60 seconds each with no crash artifact. The first recorded
run was blocked by a pre-existing 40-byte artifact dated 2026-11-02, hash
crash-e3e0a1123d8984a3957422ddc378cfa6eda80aaa. It replayed successfully under the current
ASan binary and was preserved in /tmp/stonecrab-s101-replayed-artifacts; the full recorded
run was then repeated and passed. The serial smoke attempt was superseded by the existing
concurrent evidence runner; this is a 60-second gate run, not a release-duration claim.
