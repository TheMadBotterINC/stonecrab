# S101 OPC UA crypto foundation

Date: 2026-11-03. Source/packaging baseline: 8a16f51717e646d6a7d46da6f6f9f8665fa38749.
All local gates and the five-target hosted matrix pass. Run 41636349368 completed
successfully: 33 jobs passed and the scheduled-only job was intentionally skipped.
ADR-0039 records the committed plan; the final commit changes repository documentation only.

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

## Local validation

- Full check passes: 2431 tests, 588 merged plus five other doctests, strict Clippy/fmt,
  documentation with warnings denied and no-default-features check.
- Core 6978/6978 and OPC UA 8337/8337 lines meet their unchanged 100% gates. All other
  component gates pass. This is line coverage, not a branch-coverage claim.
- All 59 feature combinations and no-default-features check pass in isolation. Cargo-hack
  restored the manifests/lock cleanly. Root and fuzz stable Clippy pass.
- Root/fuzz dependency policies and machete pass with no new deny ignores/wrappers.
  D23 output is in opcua-s101-dependency-trees.txt; existing once_cell/lazy_static paths
  remain allowed and log/time/chrono are absent. License/advisory/source checks pass.
- Configured crypto mutations: 66 tested, 46 caught, 20 unviable, no missed/timeouts.
  The additional SecuritySizes campaign catches all 15 mutants. Both exit zero, using
  --config mutants.toml, --gitignore true, jobs 4, debug info disabled and focused --re.
  No exclusions were added. Raw outcomes remain in target/mutants-s101-final/mutants.out/
  and target/mutants-s101-chunk/mutants.out/.
- All 15 ASan fuzz targets pass for 60 seconds each on nightly-2026-10-01, via the existing
  concurrent evidence runner. [Raw report](opcua-s101-fuzz.md). This is a gate run, not
  the deferred release-duration run. Fuzz policies/lock remain checked.
- Regeneration (including generator gates and two identical output runs), parity and
  their self-tests pass. Attribute/DCO gates and default/all-feature semver pass.
- Examples pass. The native open62541 nine-test exact-count gate, in-image checks and
  read example pass. Peer stopped and temporary host PKI removed. Image linux/amd64
  sha256:5585cf619fc68da4b3f96a0bf124fa8039780f40c1b597ea82a5dc20eabae68f,
  open62541 v1.5.9 source 05c2b389b65cc35884eb40d800301af1f3d130cf.
- Fresh-CARGO_HOME package gate passes; 218 shipped files match packaging/stonecrab.txt,
  no fixture/testdata directory is shipped, and unpacked all-feature and opcua-only builds
  pass. Archive 858992 bytes, SHA256
  3264bfbb01337e3f58d4102fe71a3bb716abf227c3cb65b325af39fc6cfd75a3;
  .cargo_vcs_info source is 8a16f51717e646d6a7d46da6f6f9f8665fa38749.

Local logs are /tmp/stonecrab-s101-{check,coverage,features,clippy,fuzz-clippy,deny,
mutants-final,mutants-chunk,regen,parity,attrs,dco,semver,semver-all,package,unpacked-all,
unpacked-opcua,examples,interop,fuzz-evidence,fuzz-native-build}.log. Interop evidence is
under target/interop-evidence/open62541/ (including selected-tests.txt).

## Local footprint (concurrent validation load)

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
CI repeats this workload as feature-footprint and preserves raw metrics. Hosted results and the E17 decision are recorded below.

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
listing selects exactly nine tests, and the corrected hosted peer job passes.

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


## Hosted footprint and native build cost

Run [41636349368](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41636349368),
source 8a16f51, repeats the same isolated-target script. Raw TSV/time files are the
feature-footprint artifact; this table retains the same scope and limitations as above.

| Features | Clean release seconds | Compiler max RSS KiB | Registry max RSS KiB | Binary bytes | Archive bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| modbus | 44.72 | 566364 | 2416 | 5832472 | 162221142 |
| modbus,opcua | 79.75 | 692440 | 2560 | 15536808 | 255172820 |
| modbus,tls,opcua | 84.28 | 737964 | 3056 | 22737904 | 269186424 |

OPC UA adds 35.03 seconds (1.78x total build time), 126076 KiB compiler maximum RSS,
144 KiB registry RSS, 9704336 binary bytes and 92951678 archive bytes over Modbus alone.
The hosted 59-combination Cargo-hack step takes 154 seconds (12:32:29–12:35:03 UTC),
including the native graph, followed by its no-default check. A dedicated empty-target
local build of all 15 ASan fuzz binaries takes 43.04 seconds and 1053848 KiB maximum RSS,
including a fresh aws-lc-sys C build; /tmp/stonecrab-s101-fuzz-native-time.txt and the build
log record it. The earlier warm build is not used as the clean native cost.

E17 remains one opt-in opcua feature outside default/full: the complete hosted feature
matrix and clean fuzz build complete successfully at the measured cost, with no evidence
requiring another feature. The cost and toolchain requirements are now explicit. This
measurement does not characterize protected channel throughput or session memory; those
require the later operational slices.

## Hosted validation

The corrected source run is 41636349368 on 8a16f51: all six Ubuntu/macOS/Windows cells
(Rust 1.97.1 and 1.99.1), aarch64/armv7 cross builds, quality/coverage, dependencies,
features, package, semver, all peer jobs and the other ordinary gates pass. The full
fuzz-smoke job also passes; all 33 ordinary jobs are green, with the scheduled-only job
intentionally skipped. The final documentation-only commit preserves the built source
and package inputs. Normal signed-off merge retains all feature-branch commits.
