# S103 client application key material

Date: 2026-11-03. Implementation 9f5c97dc4b527c32b0cec389c60a0d99e30e71d3;
final fuzz/package baseline 3d7a0ff7b72ab781ff44e7b54d8071b5e30a93a6.
ADR-0041 is the committed design. Local gates and hosted validation pass. The final
commit changes repository documentation only, preserving source/package inputs.

## Behavior and scope

E16's accepted recommendation is implemented for client key stores: one PEM file with
leaf-first certificates and exactly one matching unencrypted RSA PKCS#8 key. No new
dependency, public API, feature or version; opcua remains outside default/full. There is
no PKCS#1 wrapping, alias, password, PKCS#12/JVM store or generated certificate support.

The connect edge opens the file once and reads at most 1 MiB plus one overflow-detection
byte. The parser uses existing rustls-pki-types PEM, webpki certificate/SPKI/SAN access and
AWS-LC RSA parsing. Unsupported PEM sections, missing/duplicate/invalid/non-RSA keys,
malformed certificate chains and a leaf/key mismatch are InvalidConfig. No path, key
bytes, file contents or provider/OS source details enter errors or key-material Debug.
Secret redacts formatting, without a zeroization claim. Native handles own the keys;
raw PEM/private-key DER copies are not retained by the loaded material.

The material holds the leaf, supplied-order concatenated certificate chain for the future
SenderCertificate, SHA-1 of the leaf DER as an identifier, and one nonempty absolute ASCII
application URI from the leaf SAN. Missing, invalid and multiple URI names fail; no secure
fallback URN. The existing None session URN is unchanged. Loading is not chain/trust
validation. Server trust is S104; protected OPN and operational secure connect are
S105/S106. Secure policies preflight the key store before network I/O and then retain the
existing secure-connect refusal. None does not read a configured key store.

The old native-test PKI is relocated to tests/support/opcua_pki.rs. RSA keys are generated
at runtime with AWS-LC; hand-written test DER includes the RSA application usages, SAN,
organization, key identifiers, CA chain and fixed current/expired validity. An Ed25519
subject/key negative case, key-first PEM and LF/CRLF/CR inputs are covered. No keys or
temporary PKI files are committed.

Five parity rows close: CFG-KEYSTORE, CFG-KEYSTORE-TYPE, CERT-KEYSTORE, CERT-CHAIN and
CERT-APPLICATION-URI. Total 249: 133 implemented, 78 planned, 38 deferred. The key-store
half of DIV-JVM-KEYSTORES is recorded; its combined row stays planned until trust stores
in S104. Generated-certificate rows remain deferred with an actual pre-I/O refusal test.

## Local validation

- Full check: strict formatting/Clippy/rustdoc, 2469 nextest cases, cargo test and
  588 merged plus five other doctests; no-default build passes. Nine hardware-free native
  peer tests are part of the separately executed interop gate, not ordinary suite passes.
- OPC-UA-only unit run: 333 passed (no default features, no testkit).
- Coverage: core 6978/6978, codec 245/245, sim 2307/2307, modbus 6699/6699,
  sensors 1840/1840, boards 511/511 and handwritten OPC UA 8825/8825, all 100%.
  Transport 2877/2897 (99.31%), testkit 5272/5394 (97.74%). Thresholds/exclusions unchanged.
- Focused cargo-mutants keystore scope: 48 total, 36 caught, 12 unviable; no survivors,
  timeouts or new exclusions. Configured crypto glob already covers the new file.
- Isolated feature gate: all 59 combinations plus no-default; manifests and locks restored.
- All 16 fuzz targets: 60 seconds each, zero artifacts, successful final exits. Initial
  connection-string run exposed the harness's outdated accepted-option list, not a parser
  panic. The separate signed-off test commit adds both new options and value checks plus
  five seeds, including a reduced accepted-string regression. The complete rerun passes:
  [report](fuzz/2026-11-03-s103.md). Stable fuzz Clippy and fuzz deny/policy-copy checks pass.
- Regeneration and its negative self-tests, dependency advisories/bans/licenses/sources,
  machete, examples, semver (202 passed, 58 skipped), parity and its self-tests, lint-attrs
  and DCO pass. Parity references 359 implemented/automated-verified rows globally.
- Fresh-CARGO_HOME package: 221 files, 872167 bytes, SHA-256
  a66c1fc5b38ce5a3bf12b5963f3016acd24536182727265938b85b65a3834cb9.
  Embedded source SHA is 3d7a0ff; reviewed inventory matches. Unpacked all-feature and
  OPC-UA-only builds pass. Repository-only shared PKI is excluded as intended.
- Native open62541 gate: nine selected tests, in-image fixture checks and public example
  pass. Source v1.5.9, commit 05c2b389b65cc35884eb40d800301af1f3d130cf; linux/amd64
  image sha256:ff2c43af22208b4548a198718389e9ec11f9d3f2470a68337012e782dc2a8451.
  Peer stopped and temporary host PKI removed. No hardware/secure-peer claim is made.

Local logs: /tmp/stonecrab-s103-{check,coverage,features,mutants,fuzz,fuzz-clippy,fuzz-deny,
regen,dependencies,machete,semver,package,unpacked-all,unpacked-opcua,examples,opcua-only,
interop,interop-stop,parity,attrs,dco}.log. Mutant outcomes are under
/tmp/stonecrab-s103-mutants/mutants.out/. Native artifacts remain under
target/interop-evidence/open62541/.

## Hosted validation

[Run 41761519189](https://github.com/TheMadBotterINC/stonecrab/actions/runs/41761519189)
on 3d7a0ff passes all 33 executed jobs: all six Ubuntu/macOS/Windows Rust 1.97.1/1.99.1
cells, aarch64/armv7 cross builds, quality/coverage, dependencies, features, package,
examples, semver, peer jobs, DCO and the full serial fuzz smoke. Only the scheduled job
is intentionally skipped. The superseded run 41761281095 on 9f5c97d was cancelled by
the branch concurrency rule after the fuzz-harness correction; no pass is claimed for it.
Local DCO also checks the final documentation and normal-merge range. The final
repository-only documentation preserves the validated source and package inputs.
