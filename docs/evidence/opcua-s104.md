# S104 server trust and fail-closed preparation

Date: 2026-11-03. Implementation c47bea37f0ce92d847bf888019feb90fc55e8609; final test/package
baseline 25af246fa11e472e5da4a961084deed6268e19be.
ADR-0042 is the committed design. Local gates and hosted validation pass.
Final repository documentation preserves the source and package inputs.

## Behavior and scope

Every secure policy requires server-certificate-file before client-key or transport I/O,
even with tls.trust-store or tls.verify=false; there is no None fallback. Missing bootstrap
configuration is InvalidConfig naming the options and the explicit security-policy=NONE
choice. None does not load certificates/stores. Valid preparation retains the existing
secure-connect refusal: protected OPN is S105, remote verifier call sites S106.

Bootstrap loading accepts one DER or PEM certificate. Trust stores accept PEM/default
certificates or explicit DER; passwords and PKCS#12/JVM formats are refused. Files open
once, read at most 1 MiB plus one detection byte, and fail with fixed InvalidConfig errors
without paths, contents or sources. Auto detection tries certificate structure rather
than matching embedded marker text. The webpki individual-certificate size ceiling also
applies. No extra dependency, feature, public API or version is introduced.

The private verifier selects explicit permissive, configured store, exact pin, reject.
Only the selected trust store is opened; bootstrap structure/RSA remains mandatory even
under opt-out. Debug/structured tracing name the strategy, and permissive preparation
records tls.verify=false at warning level. The trust-only bypass mirrors upstream,
including empty input, without bypassing later cryptographic key/signature requirements.

Strict direct trust checks current validity, accepted RSA bounds and serverAuth EKU if
present. A store's exact leaf DER match is explicitly trusted; other chains need a webpki
path through supplied issuers to configured current anchors, with native PKCS1/PSS-SHA256
verification. Presented/discovered roots are never anchors. Pins compare only leaf DER
through AWS-LC constant-time contents comparison; length is public. Wrong usage, expired
or future leaves/issuers/anchors, bad signatures, malformed/corrupt/untrusted chains and
key-policy failures are Trust without provider sources. Peer chains are bounded to 16
certificates and 1 MiB combined DER. No OS/public roots, network fetching, hostname policy
or revocation service is added. RSA 2048..4096-bit bounds reuse framing's policy check;
independent-peer confirmation remains UNVERIFIED until S106.

The certificate property reader uses bounded canonical DER, UTC/GeneralizedTime and the
existing calendar types; webpki owns complete X.509/path validation. Native signature
adapters replace the old test-only implementation and use published algorithm IDs.
The shared runtime-generated PKI adds PSS, optional EKU and GeneralizedTime fixtures.
No private key is committed or reused as production material.

Eleven rows close, including the S103/S104 combined JVM-store divergence. OPC UA totals:
249 rows, 144 implemented, 67 planned, 38 deferred; 370 implemented/automated-verified
rows referenced globally. Private verifier support is distinct from S106 handshake use.

## Local validation

- Full strict check: formatting, Clippy, rustdoc, 2507 nextest cases, cargo test,
  588 merged plus five other doctests and no-default build pass. OPC-UA-only unit run:
  367 passed, with no default features/testkit.
- Coverage: core 6978/6978; codec 245/245; sim 2307/2307; modbus 6699/6699;
  sensors 1840/1840; boards 511/511; handwritten OPC UA 9329/9329, all 100%.
  Transport 2877/2897 (99.31%), testkit 5272/5394 (97.74%). Floors/exclusions unchanged.
- Configured new-module mutation campaign: 120 total, 108 caught, 12 unviable. The final
  decoder/reader changes have a further 36 mutations, all caught. No survivors, timeouts
  or new exclusions. Earlier survivors led to exact byte/count/canonical-length boundary
  and mixed-PEM-section tests. Final artifacts are under /tmp/stonecrab-s104-mutants-verified
  and /tmp/stonecrab-s104-mutants-io (each has mutants.out/).
- A complete coverage clean confirmed that source-line views hid per-instantiation gaps:
  see [upstream diagnostics](https://github.com/taiki-e/cargo-llvm-cov/pull/494).
  The final DER length decoder uses a bounded four-octet word, eliminating an impossible
  native-width overflow fold. The I/O edge uses one std::io::Read dispatch path for File,
  Cursor, byte-slice and faulting readers. No tool pin, threshold, exclusion or gate changed.
- All 17 fuzz targets pass 60 seconds each, zero artifacts and successful exits:
  [report](fuzz/2026-11-03-s104.md). The local report is on c47bea3; the later Windows fix changes only test boundary
  offsets, with no verifier/fuzz-code change. The new certificate target has nine public/malformed
  seeds; its OpenSSL certificates contain no private key. Connection-string oracle/seeds
  change with the three newly accepted options. Stable fuzz Clippy and fuzz dependency/
  policy-copy checks pass.
- Isolated feature gate: all 59 combinations plus no-default; manifests/locks restored.
  Regeneration and self-tests, dependencies, machete, examples, semver (202 passed,
  58 skipped), parity/self-tests, lint-attrs and DCO pass.
- Fresh-CARGO_HOME package: 225 files, 883983 bytes; SHA-256
  82b1c8958bce6277db57c173710b24fd7f05ea0f9cdccd82c1b9ee42356f0ea5.
  Embedded source SHA 25af246, inventory matches; unpacked all-feature and OPC-UA-only
  checks pass. Repository-only PKI support remains excluded as intended.
- Native open62541 gate: nine selected None cases, in-image fixture checks and the public
  example pass. v1.5.9 source 05c2b389b65cc35884eb40d800301af1f3d130cf; linux/amd64 image
  sha256:a2925c223efa77faaf2353c80dc7189772d9584016b2ea8d30bb578ffe803f75.
  Peer stopped; temporary host PKI files/directory removed. No secure-peer/hardware claim.

Logs: /tmp/stonecrab-s104-{check,coverage,features,mutants-verified,mutants-io,fuzz,
fuzz-clippy,fuzz-deny,regen,dependencies,machete,semver,package,unpacked-all,unpacked-opcua,
examples,opcua-only,interop,interop-stop,parity,attrs,dco}.log. Peer evidence remains under
target/interop-evidence/open62541/.

## Hosted validation

[Run 41772988899](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41772988899)
on 25af246 passes all 33 executed jobs: all six Ubuntu/macOS/Windows Rust 1.97.1/1.99.1
cells, aarch64/armv7 cross builds, quality/coverage, dependencies, features, package,
examples, semver, peer jobs, DCO and full serial fuzz smoke. Only the scheduled job is
intentionally skipped. Run 41772364480 on c47bea3 exposed a test-fixture portability
issue on Windows: subtracting one nanosecond from SystemTime rounds to the same instant.
The signed-off test correction uses one-second offsets, matching X.509 validity precision,
while retaining both equality checks. The old run was superseded/cancelled; no full pass
is claimed for it. The current push also runs hosted DCO; local DCO checks the complete
feature and final normal-merge ranges. Final repository-only documentation preserves
validated source and package inputs.
