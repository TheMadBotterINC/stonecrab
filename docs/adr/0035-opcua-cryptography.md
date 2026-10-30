# 0035: OPC UA cryptography
## Status
Proposed (2026-10-30), as slice S82 of docs/POST_ALPHA_PLAN.md ("Cryptography probe across the
CI matrix, as evidence for E15"). It is the evidence and the design for decision E15 (which
cryptography dependency carries the OPC UA security policies), accepted as recommended on
2026-10-30 on the condition that the probe is green on all five targets with `cargo deny`
unchanged, and for the part of E16 that depends on what the library can sign. Slice S101
("Adopt the E15 backend and accept the S82 ADR") accepts this ADR and adds the dependencies;
nothing here changes `Cargo.toml`, `Cargo.lock`, `deny.toml` or any code of the crate, and
ADR-0034 (the client design, slice S76) is a separate document. Every statement of what a
build or a test did is about the unmerged branch `spike/s82-crypto-probe`, whose results are
copied below so that they outlive the branch.

## Context
The OPC UA security policies need, on the client side: RSA-OAEP with SHA-1 and with SHA-256
(Basic256Sha256 and Aes128_Sha256_RsaOaep use SHA-1, Aes256_Sha256_RsaPss uses SHA-256),
RSASSA-PKCS1-v1_5 with SHA-256 and RSASSA-PSS with SHA-256 signatures, AES-CBC with a key and
an IV derived from the channel nonces and no padding (the stack pads the message itself),
HMAC-SHA1 and HMAC-SHA256, the SHA-1 thumbprint of a certificate, and X.509 path validation of
the server certificate against an operator-supplied trust list (`docs/parity/opcua.md`,
`OPCUA-POLICY-*`). `ring` 0.17, the provider of the `tls` feature (ADR-0032), has no RSA
encryption and no AES-CBC (the S75 record; S82 did not re-read ring). `deny.toml` bans `openssl`
and `openssl-sys`, `log`, `time`, `chrono` and first-party use of `once_cell` and
`lazy_static`.

The E15 recommendation was `aws-lc-rs` with `rustls-webpki` and `rustls-pki-types`, optional
behind an `opcua` feature, and its cost is a native library: `aws-lc-sys` compiles AWS-LC from
C, and on Windows x86_64 assembles with NASM unless it is told to use the prebuilt objects.
Whether that is acceptable depends on three things only a build shows: that it builds and
passes tests on every CI target, that `cargo deny` accepts its graph without a new ignore or
wrapper, and that the banned crates do not come in with it. The plan's S82 row fixed the
stack: `aws-lc-rs` 1.19.0 with default features off (`aws-lc-sys` and `alloc`), `aws-lc-sys`
0.47.0 with `disable-prebuilt-nasm`, `rustls-webpki` 0.103.16 with no crypto provider feature,
`rustls-pki-types` 1.15.1, with 1.18.2 and 0.46.0 as the fallback.

## Evidence
All four crates and versions exist on crates.io and none is yanked (checked 2026-10-30):
`aws-lc-rs` 1.19.0 and `aws-lc-sys` 0.47.0 were published on 2026-10-28, `rustls-webpki`
0.103.16 on 2026-10-20, `rustls-pki-types` 1.15.1 on 2026-07-23. The fallback was not needed.

**The probe.** `spikes/opcua-crypto-probe/` on branch `spike/s82-crypto-probe` (head
`cf6aea2`, five commits on `d14ffa9`) is its own Cargo workspace, outside the root
workspace. `crates/stonecrab`, `Cargo.toml`, `Cargo.lock` and `deny.toml` are byte-identical to
`d14ffa9` (the workflow's `git diff --exit-code` step checks this on every run). It has 31
tests in six files, all `#![forbid(unsafe_code)]` first-party code: RSA-OAEP with SHA-1 and
SHA-256 (round trip at the maximum plaintext of 214 and 190 bytes for a 2048-bit key, one byte
over refused, a flipped ciphertext bit refused, a hash mismatch refused, randomised
ciphertext); PKCS #1-SHA256 and PSS-SHA256 sign and verify (PSS salted, PKCS #1 deterministic,
neither verifies as the other, a salt other than the digest length does not verify as the
default); AES-128 and AES-256 CBC with a caller IV and no padding against NIST SP 800-38A
F.2.1 and F.2.5 (a partial block refused); HMAC-SHA1 against RFC 2202 case 1 and HMAC-SHA256
against RFC 4231 case 1; the SHA-1 thumbprint of a certificate; a self-signed anchor through
webpki; and the key-loading cases below. Test certificates are written by hand in DER
(`rcgen` depends on the banned `time`), as ADR-0032 decision 11 does.

**The runs.** Workflow `.github/workflows/s82-crypto-probe.yml` on the same branch, triggered by
pushes to it only:

| Run | Commit | Result |
| --- | --- | --- |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41107964693 | `340fa46` | Five targets green. The `dependencies` job failed on exit 128: its `git fetch origin d14ffa9` cannot fetch an abbreviated id. A workflow error, not a finding; `cargo deny` itself passed in that job. |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41108284236 | `38605cd` | All seven jobs green (the fix: check out the whole history, diff against the full id). |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41109122420 | `a6540e8` | All seven jobs green. |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41109616704 | `396d699` | All seven jobs green. |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41109616667 | `396d699` | The repository's own `ci.yml`, which also runs on every branch push: every job green, `machete` included (it failed on `340fa46` until the manifest note of `a6540e8`). |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41112736480 | `cf6aea2` | All seven jobs green, after the D23 loop of the `dependencies` job was changed to count a name as absent only when `cargo tree` says `did not match any packages` (the five lines are in its log). |
| https://github.com/TheMadBotterINC/stonecrab/actions/runs/41112737157 | `cf6aea2` | The repository's own `ci.yml` on the final head: every job green except `scheduled`, which is skipped by design. |

The seven jobs are the five targets, a footprint job and the dependency job: `test` on
ubuntu-latest (x86_64), macos-latest (arm64) and windows-latest (x86_64-pc-windows-msvc, NASM
2.16.03 from `ilammy/setup-nasm` pinned to its v1.5.2 commit, `AWS_LC_SYS_USE_SYSTEM=0` for
every job), `cargo check --all-targets` for `aarch64-unknown-linux-gnu` and
`armv7-unknown-linux-gnueabihf` with the `gcc-aarch64-linux-gnu`, `gcc-arm-linux-gnueabihf` and
`libc6-dev-*-cross` install line of `ci.yml`'s `cross` job (gcc 13.3.0), `footprint`, and
`dependencies`. The toolchain was 1.99.1; the 1.97.1 leg of the `test` matrix of `ci.yml` was
not run on a runner, so the declared MSRV of 1.97 is untested there (the crates' own
`rust-version` is 1.71.0 for `aws-lc-rs` and `aws-lc-sys`, 1.71 for `rustls-webpki` and 1.60
for `rustls-pki-types`). The probe also ran on macOS arm64 under 1.99.1 on the author's machine
and under 1.97.1 on the reviewer's: 31 tests passed both times, and the build needed no CMake (the machine's `cmake` shim has no version set; Cargo built
AWS-LC with the `cc` crate). On the runners the build script's default path is the same: it
uses CMake only when the `cc` builder's dependency check fails (`builder/main.rs:766-791` in
`aws-lc-sys` 0.47.0); which of the two the Windows leg used was not recorded.

**`cargo deny` with the root `deny.toml`, unchanged, no ignore entry.** The command is
`cargo deny --manifest-path spikes/opcua-crypto-probe/Cargo.toml --locked --config deny.toml
check` (cargo-deny 0.20.2, the version of `ci.yml`). Its result, on the author's machine and in
the `dependencies` job of all five probe runs above, is `advisories ok, bans ok, licenses ok, sources
ok`. The only diagnostics are warnings that entries of `deny.toml` matched nothing in this
graph (the licenses `0BSD`, `BSD-2-Clause`, `Unicode-3.0` and `Zlib`, and the wrappers
`sharded-slab`, `tracing-core` and `rustls`). `aws-lc-sys` 0.47.0 declares the licence
expression `ISC AND (Apache-2.0 OR ISC) AND Apache-2.0 AND MIT AND BSD-3-Clause AND
(Apache-2.0 OR ISC OR MIT) AND (Apache-2.0 OR ISC OR MIT-0)`; every term is satisfied from the
allow list (MIT-0 is one alternative that is not needed), and the `OpenSSL` licence, which the
list does not allow, does not appear. `cargo deny` resolves only the features the probe enables (the
`all-features = true` of `deny.toml` applies to workspace members, not to dependencies), so
the optional `ring` and `aws-lc-rs` dependencies of `rustls-webpki` are not in the graph it
checks, and `ring` is absent from it.

**D23 (`cargo tree --locked --all-features --target all -e normal,dev,build -i <name>`).** For
`once_cell`, `lazy_static`, `log`, `time` and `chrono` every command fails with `package ID
specification ... did not match any packages`, i.e. none is in the probe's graph, with no
wrapper to excuse it (the `dependencies` job turns any match into a failure). The graph of
normal and build dependencies has 18 packages besides the probe across all targets, and 15 on
one host: `aws-lc-rs`, `aws-lc-sys`, `cc`, `cmake`, `dunce`, `find-msvc-tools`, `fs_extra`,
`jobserver`, `libc`, `pkg-config`, `rustls-pki-types`, `rustls-webpki`, `shlex`, `untrusted`
and `zeroize` everywhere, and `cfg-if`, `getrandom` and `r-efi` besides, which `jobserver`
brings in on non-host targets (`cargo tree --target all` and `cargo deny` list them; `r-efi`
offers `LGPL-2.1-or-later` among its licence alternatives, so `deny.toml`'s allow list has to
be satisfied by another one, and it was). The CMake-driven build helpers come in as build
dependencies of `aws-lc-sys` whichever builder runs.

**Build time and size.** Measured on the runners of the five probe runs above (every range is
the lowest and highest of the five); timings of a shared runner vary by tens of percent between
runs, so read them as orders of magnitude. The baseline of the first run, 50 s, was the slowest.

| What | Result |
| --- | --- |
| Clean debug build of the probe's tests | ubuntu 13 to 20 s, macOS 14 to 37 s, Windows 39 to 82 s (with NASM), over five runs |
| `cargo check --all-targets`, clean | aarch64 17 to 25 s, armv7 17 to 32 s, over five runs |
| Baseline: clean release build of `stonecrab --features tls --lib` (ring) | 38 to 50 s (50, 39, 39, 38, 48); `libring_core` 7,157,930 bytes, `libring` rlib 10,638,872, `librustls` rlib 11,535,284, `libwebpki` rlib 1,595,006 |
| Probe: clean release build of the library | 29 to 30 s (30, 29, 29, 29, 29); `libaws_lc_0_47_0_crypto.a` 7,330,826 bytes, `libaws_lc_rs` rlib 2,933,716 |
| Probe: release example linking every primitive | 3,172,408 bytes; 2,918,096 stripped |
| Crates downloaded | `aws-lc-sys` 9,737,619 bytes (53 MB of C source unpacked), `ring` 1,502,610, `aws-lc-rs` 256,251 |

The delta for a build with both `tls` and `opcua` is therefore of the order of +30 s on a
clean build and +7 MB of native static archive, as the two numbers above are separate builds
on one machine. The combined build, the `features` powerset job and the fuzz workspace with
`opcua` were not measured, and Cargo builds independent crates in parallel, so the sum
overstates a combined build.

**PKCS #1 versus PKCS #8.** From the sources of `aws-lc-rs` 1.19.0 and confirmed by
`tests/key_loading.rs`: `RsaKeyPair::from_der` reads an RFC 8017 `RSAPrivateKey` (PKCS #1) and
`RsaKeyPair::from_pkcs8` an RFC 5208 key; neither reads the other's form.
`rsa::PrivateDecryptingKey`, which OAEP decryption needs, has `from_pkcs8` only and no
`from_der`. A PKCS #1 key therefore reaches decryption by one of two conversions, both tested
to give the same decrypting key: read it with `RsaKeyPair::from_der` and re-export it with
`AsDer<Pkcs8V1Der>`, which needs no first-party DER at all; or wrap it in a PKCS #8
`PrivateKeyInfo` by hand (the probe's `pkcs1_to_pkcs8` is 20 lines, and its output is
byte-identical to the library's own export). `rustls-pki-types` 1.15.1 sorts PEM text by label
(`PRIVATE KEY` to `PrivateKeyDer::Pkcs8`, `RSA PRIVATE KEY` to `PrivateKeyDer::Pkcs1`) and does
not accept `ENCRYPTED PRIVATE KEY`, which matches E16's refusal of encrypted keys.

**webpki without a provider feature.** `rustls-webpki` 0.103.16 with only `alloc` ships no
signature algorithm: `EndEntityCert::verify_for_usage` is given a list of
`&dyn SignatureVerificationAlgorithm` and, with an empty list, validates nothing (tested). The
probe implements the trait once for RSA PKCS #1 SHA-256 over `aws_lc_rs::signature::
UnparsedPublicKey` (about 25 lines) and gets, in tests: a self-signed application certificate
(no CA flag, `serverAuth` and `clientAuth`, a URI and a DNS name as subject alternative names)
validating against itself used as the trust anchor; a certificate issued by a CA validating
through the CA anchor; an unrelated anchor giving `UnknownIssuer`; a certificate after its
`notAfter` giving `CertExpired`; the DNS name matching and a wrong name not matching; and a
peer's PKCS #1-SHA256 signature verifying through `EndEntityCert::verify_signature`, the OPC UA
use. Two details cost a failed first run: the crate's library name is `webpki`, and
`AlgorithmIdentifier::from_slice` takes the contents of the DER `SEQUENCE` (the OID and its
parameters), not the `SEQUENCE` with its header. webpki's own `aws-lc-rs` feature is
`["dep:aws-lc-rs", "aws-lc-rs/aws-lc-sys", "aws-lc-rs/prebuilt-nasm"]`
(`rustls-webpki-0.103.16/Cargo.toml`), so turning it on would switch on `prebuilt-nasm` for the
whole graph; that is why the probe, and this ADR, use no webpki crypto feature. The
documentation of `anchor_from_trusted_cert` says webpki "has no support for self-signed
certificates" and warns against using an end-entity certificate as its own anchor; the probe's
test shows that a certificate without the CA flag does validate against itself as an anchor in
0.103.16, which is the case of an OPC UA application certificate in a trust list. That is
observed behavior of one version, pinned by a test, not a documented guarantee.

**The SHA1withRSA signing gap (E16).** `aws-lc-rs` 1.19.0 has no way to produce an RSA
signature over SHA-1. Its signing encodings are `RSA_PKCS1_SHA256`, `RSA_PKCS1_SHA384`,
`RSA_PKCS1_SHA512` and the three `RSA_PSS_SHA*` (`src/signature.rs:838-878`). `RsaEncoding` is
a sealed trait and `RsaSignatureEncoding::new` is `pub(crate)` (`src/rsa/signature.rs`), so a
caller cannot add one. SHA-1 appears only on the verification side, as
`RSA_PKCS1_2048_8192_SHA1_FOR_LEGACY_USE_ONLY` and its 1024-bit sibling, and
`RsaKeyPair::sign_digest` with a SHA-1 `Digest` and the SHA-256 encoding is refused as a
digest mismatch (`sha1_with_rsa_cannot_be_signed`). Basic256 and Basic128Rsa15 sign with
SHA1withRSA, so they cannot be offered on this backend; a second RSA library would be the only
way, and E16 chose to refuse them instead. Basic128Rsa15 would also need PKCS #1 v1.5
decryption (the library has it, `Pkcs1PrivateDecryptingKey`; the probe does not exercise it),
and AWS-LC's own header calls that scheme insecure against chosen-ciphertext attacks
(`include/openssl/rsa.h`, on `RSA_PKCS1_PADDING` and `RSA_decrypt`), which supports the refusal.

**Marvin-class timing of OAEP decryption: no AWS-LC statement found (UNVERIFIED).** The Marvin
attack is the family of timing side channels in RSA decryption, reported against the `rsa`
crate as RUSTSEC-2023-0071; E15 rejected that crate for it. Searched on 2026-10-30: the
complete AWS-LC source vendored in `aws-lc-sys` 0.47.0 and the sources of `aws-lc-rs` 1.19.0
(case-insensitive, no file mentions "marvin"); `gh search issues` on `aws/aws-lc` for "Marvin
attack" and for "RSA timing side channel" (no result). GitHub code search was not available
to the tool. So there is no statement, and none is claimed. What the vendored source shows is
code, read and not measured: `RSA_padding_check_PKCS1_OAEP_mgf1`
(`crypto/rsa_extra/rsa_crypt.c:105-206`) compares the label hash with `CRYPTO_memcmp`, finds
the `0x01` separator with `constant_time_*` selects over the whole block, declares the
validity of the padding public deliberately (as RFC 8017 section 7.1.2 intends) and gives one
error, `RSA_R_OAEP_DECODING_ERROR`, for every padding failure; the private operation is
blinded (`crypto/fipsmodule/rsa/rsa_impl.c`, `rsa_blinding_get`). The probe measures no
timing. The mitigation that does not depend on any of this is the one the plan already lists
(S105): one code path and one uniform error for OPN response decryption, so a caller sees no
difference between a bad padding, a bad key and a bad signature.

## Decision
1. **Backend.** `aws-lc-rs` 1.19.0 (non-FIPS) with `default-features = false` and features
   `aws-lc-sys` and `alloc`, behind an `opcua` Cargo feature that no other feature turns on (E17).
   A direct dependency on `aws-lc-sys` 0.47 with `default-features = false` and the feature
   `disable-prebuilt-nasm`, named only to set that feature, so the Windows x86_64 build assembles
   with NASM and never with objects fetched at build time. `rustls-webpki` 0.103.16 with `alloc`
   and no provider feature; `rustls-pki-types` 1.15.1 with `alloc`. The fallback pair 1.18.2 and
   0.46.0 stays recorded and unused. The default features of `aws-lc-rs` (`ring-io`,
   `ring-sig-verify`) stay off, as the probe shows nothing needs them. S101 declares the versions
   in `[workspace.dependencies]` with caret ranges (CODING_STANDARDS 15.4); the probe's `=` pins
   are for the spike.
2. **Native build rules.** CI sets `AWS_LC_SYS_USE_SYSTEM=0` for every job that builds the
   feature, so a library found through `OPENSSL_DIR` or pkg-config is never adopted, and the
   Windows legs install NASM with the pinned action of the probe. The cross legs reuse the
   existing compilers and `libc6-dev-*-cross` packages. No other host package is needed, and
   downstream users get `AWS_LC_SYS_USE_SYSTEM` documented (E15).
3. **Two C libraries under `tls` plus `opcua`.** rustls stays on `ring` (E11, ADR-0032); a build
   with both features links `ring` and AWS-LC. The two do not interact: rustls is not given
   `aws-lc-rs`, and no crate of the OPC UA path enables a provider feature of `rustls` or
   `rustls-webpki`.
4. **Certificate chains.** Server certificates are validated by `rustls-webpki` against the
   trust list, with first-party `SignatureVerificationAlgorithm` implementations over
   `aws-lc-rs`: RSA PKCS #1 SHA-256 first (the probe's), then those S104 needs. A certificate
   signed with an algorithm without an implementation is a `Trust` error naming the algorithm,
   not a silent pass.
5. **Keys.** The key store takes PEM or DER, a certificate chain and an unencrypted PKCS #8 key,
   as E16 (b) accepted it. PEM is classified by `rustls-pki-types`; a PKCS #8 key loads directly
   into `RsaKeyPair` and `PrivateDecryptingKey`. An encrypted PEM, and any key that is not
   PKCS #8, is `InvalidConfig`. Open question for the maintainer (S103 raises it, nothing here
   changes E16): accepting PKCS #1 keys as well would cost one extra call by the
   read-and-re-export route above and no first-party DER, and it would match ADR-0032, which
   accepts PKCS #8, SEC1 or PKCS #1 for `tls`. Until the maintainer answers, E16 (b) is the
   rule.
6. **Policies the backend cannot sign.** Basic256 and Basic128Rsa15 stay refused with a typed
   `InvalidConfig` naming the deprecation and the missing SHA1withRSA signing (E16 a). This ADR
   adds the evidence: the signing encodings are fixed and sealed, so there is no first-party
   workaround. SHA-1 stays in the backend for what the supported policies use it for: the
   OAEP hash of Basic256Sha256 and Aes128_Sha256_RsaOaep and the certificate thumbprint.
   HMAC-SHA1 is available and tested, but no policy left after E16 uses it.
7. **Timing.** No claim of constant-time behavior is made for OAEP decryption beyond what
   AWS-LC's source shows (above). Where the client decrypts RSA data (the OpenSecureChannel
   response, S105) it uses one code path and one uniform error.

## Consequences
- The OPC UA policies Basic256Sha256, Aes128_Sha256_RsaOaep and Aes256_Sha256_RsaPss have every
  primitive they need in one library with a first-party seam of a few dozen lines, and path
  validation stays with `rustls-webpki`, which `tls` already compiles.
- A build with `opcua` needs a C compiler, and a Windows x86_64 build needs NASM. Every build
  with `opcua` compiles AWS-LC (about 30 s clean on a CI runner, 7 MB of static archive, a 9.7
  MB crate to download), including the `features` powerset job and the fuzz workspace if it
  enables the feature; E17's option 3 (a split `opcua-security`) remains the answer if S101
  measures that as too slow.
- `cargo machete` reads the manifests. The probe needed `[package.metadata.cargo-machete]
  ignored = ["aws-lc-sys", "rustls-webpki"]` for the crate that only sets a feature and for the
  package whose library is called `webpki`; S101's `crates/stonecrab/Cargo.toml` needs the
  same two entries. The ci.yml `machete` job failed on the branch until the probe had them.
- `deny.toml` is unchanged: no ignore, no wrapper, no new licence. If a later `aws-lc-sys`
  release brings back a licence outside the allow list, or a banned crate, S101's `cargo deny`
  and D23 steps fail and the pin stays where it is.
- `unsafe`: `aws-lc-rs` and `aws-lc-sys` are not `forbid(unsafe_code)` (381 and 11,254 lines
  with the keyword on 2026-10-30, most of the latter the generated FFI declarations). That is
  the expected surface of a binding to a C library and is recorded in `DEPENDENCY_AUDIT.md`
  (draft entries for S101). First-party code stays `unsafe`-free; the probe forbids it too.
- Basic256 and Basic128Rsa15 interoperate with nothing in Stonecrab; a plant that offers only
  those policies needs policy None or a server configured with a modern policy.
- The behavior of `rustls-webpki` on a self-signed leaf as its own anchor is pinned by a test of
  the probe; S104 carries that test, so a release that changes it is noticed.

## Alternatives considered
- **RustCrypto `rsa`, `aes`, `cbc`, `hmac`, `sha1`, `sha2` (E15 option 2).** Pure Rust and no C
  compiler, but RUSTSEC-2023-0071 (Marvin) has no patched version and would need a `deny.toml`
  ignore; the same libraries could not sign SHA1withRSA more easily than a first-party
  PKCS #1 layer would. Not probed.
- **The `openssl` crate (option 3).** Reverses the ban of ADR-0032 and `deny.toml`, and ties
  behavior to the platform's OpenSSL.
- **Policy None only (option 4).** Leaves the secure policies, the point of E15, out of M3.
- **rustls's own `aws_lc_rs` provider for TLS as well.** ADR-0032 rejected it for E11; one C
  library for both features would shrink the build, but would reopen the TLS decision.
- **webpki's `aws-lc-rs` feature for the signature algorithms.** Saves the 25-line seam but
  switches on `prebuilt-nasm` for the whole graph, which the Windows build must not use.
- **The prebuilt NASM objects (`prebuilt-nasm`, the default of `aws-lc-rs`).** No NASM to
  install, but a build script that fetches and trusts prebuilt objects; the E15 text asks for
  `disable-prebuilt-nasm`.
- **A second RSA library only for SHA1withRSA signing (E16 option 2).** Would add a second
  RSA implementation for two deprecated policies; refused by E16.
- **A first-party PKCS #8 wrapper for PKCS #1 keys.** Works (20 lines, tested), but the
  read-and-re-export route needs no DER code.

## Compliance
- `spikes/opcua-crypto-probe/` on `spike/s82-crypto-probe` (tests `rsa_oaep.rs`,
  `rsa_sign.rs`, `aes_cbc.rs`, `mac_digest.rs`, `webpki_anchor.rs`, `key_loading.rs`; the seam
  in `src/lib.rs`; `examples/sizeprobe.rs`) and `.github/workflows/s82-crypto-probe.yml` on the
  same branch. The branch is not merged and no pull request is open; S101 re-creates the
  tests that matter under `crates/stonecrab/tests/` and the CI legs in `ci.yml`.
- S101 enforces the rules above with the existing gates: `cargo deny --locked check` (no
  ignore entry for `aws-lc-*`), the D23 `cargo tree -i` check for the five banned names (the
  probe's loop treats any failing `cargo tree -i` as "absent", so a resolution error would pass;
  S101 requires the text `did not match any packages` before it counts a name as absent), the
  `features` job, `cargo machete`, the `cross` job and a Windows leg with NASM.
- `docs/DEPENDENCY_AUDIT.md` carries draft entries for `aws-lc-rs`, `aws-lc-sys`,
  `rustls-webpki` and `rustls-pki-types` marked as drafts for S101; they are not adopted until
  S101 lands.
- Not verified by S82, for S101 to settle: the build with `tls` and `opcua` together, the
  powerset and fuzz-workspace build times, the 1.97.1 MSRV leg, Windows aarch64 and macOS
  x86_64, the `-vv` choice between the `cc` and CMake builders on each runner, whether a
  system-wide AWS-LC is ignored under `AWS_LC_SYS_USE_SYSTEM=0` on a machine that has one
  (read from the source of `aws-lc-sys` 0.47.0, `builder/main.rs:719-760`: `USE_SYSTEM=0` skips
  `system_detect::detect_system_awslc` altogether, so it should be, but no test ran on such a
  machine), and any timing property of OAEP decryption.
- For S101's downstream documentation, from the same lines: `AWS_LC_SYS_USE_SYSTEM=0` together
  with `AWS_LC_SYS_SYSTEM_DIR` makes the build script panic ("forbids a system install"), so a
  user who sets the second must not inherit the first from a wrapper script or a CI template.
