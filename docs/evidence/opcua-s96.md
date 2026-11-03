# S96 OPC UA discovery primitives and endpoint selection evidence

Date: 2026-11-03. Base: `23c583c`. Plan: `1f3a462`; service configuration:
`d85b89c`; generated codecs: `56efe0d`; implementation: `b015937`; runner failure
propagation: `b210e08`; final selector shrink pass: `a6663a2`.

## Scope and boundaries

The generated service closure grows from 22 to 24 cases with GetEndpoints request
428 and response 431. Independent first-party complete/truncated body fixtures,
bounded writer checks, all truncated prefixes and fuzz seeds cover both. The lane
correlates the discovery response header. Its private exchange sends a null session
authentication token, the configured URL and null locale/profile filters on the
caller's channel and deadline. It checks response type, status and handle.

A pure selector replaces Session's local anonymous-token search. It requires the
parsed URL, exact configured policy URI, effective mode, TCP transport profile and
compatible token type. Highest securityLevel wins; equal levels prefer advertised
token protection, and later equal pairs win. Null/empty token securityPolicyUri
inherits the channel. Existing nonempty policy-ID validation is retained. The selected
policy ID reaches ActivateSession, and the sanitized URL reaches connection Debug.

URL comparison preserves path case/trailing slash, compares DNS names without case,
normalizes IPv6 addresses, and compares numeric ports (4840 differs from 48400).
Host/port overrides add an alternative pair without changing the transport destination
or removing the original match. No DNS resolution, raw prefix or host alias inference.
Selection failures name both options and list at most 16 URLs, each at most 256
characters plus an ellipsis. Invalid UTF-8, credential/query/fragment/control or malformed
URLs use a fixed placeholder; token IDs and certificates are never rendered.

Discovery defaults true and rejects non-boolean input. Discovery off plus a secure
policy requires a nonempty server-certificate-file path. This slice accepts that path
syntactically; reading DER/PEM and validating trust remain S103/S104. Secure connect
still refuses before transport I/O. None connect uses CreateSession's endpoint list
and skips GetEndpoints. S106 owns secure discovery wiring. Generated token-type
compatibility is tested for Certificate/UserName/Anonymous without exposing credentials
ahead of S99 or claiming protected user-token activation ahead of S108.

Ten assigned parity rows close, leaving 249 total: 101 implemented, 110 planned and
38 deferred. EndpointSelectionTest behavior cases adapt pinned Apache PLC4X commit
ea229918f5d07700e04d7d69ab1645da679cb700; both NOTICE files retain attribution. No Java
source, XML adapter or fixture enters the archive. Public API, dependencies, features
and version (0.3.0) are unchanged; opcua remains outside default and full.

## Validation

- Full check: 2321 tests, 584 normal doctests and five compile-fail doctests; fmt,
  strict all-target/all-feature Clippy, rustdoc and no-default-features build pass.
- Coverage: handwritten OPC UA 7240/7240, core 6671/6671, codec 245/245,
  sim 2303/2303, modbus 6699/6699, sensors 1840/1840 and boards 511/511 (all 100%).
  Transport 2875/2897 (99.24%, floor 95), testkit 5277/5394 (97.83%, floor 90).
  Generated code exclusions and every floor are unchanged.
- All 59 feature combinations and the no-default-features check pass. This gate
  ran in isolation after all other Cargo operations completed; manifests restored cleanly.
- All 15 fuzz targets passed with a 10-second budget each, including new discovery,
  endpoint override and complete/truncated service seeds. The existing OPC UA service
  target now sends decoded endpoint lists through selection. Strict fuzz-workspace
  Clippy with --cfg fuzzing passes on pinned stable 1.99.1; libFuzzer runs on the pinned
  nightly-2026-10-01. The connection-string invariant now includes all accepted encoding
  and discovery keys.
- Regeneration and its self-test pass for both protocols, including generator tests,
  Clippy/deny, vendored hashes, byte-identical generation and clean generated trees.
  Parity/self-test, attribute lint, dependencies, machete, default Clippy and semver
  default/all-feature checks pass. DCO check passes for all slice commits (rerun over the final merge range).
- Configured endpoint/options campaign: 136 mutants, 126 caught and 10 unviable,
  exit 0. Final selector campaign after the shrink pass: 59 mutants, 57 caught and
  two unviable, exit 0. No survivors, timeouts or new exclusions. The first campaign
  contains 50 endpoint mutations (48 caught/two unviable), 77 options mutations
  (69 caught/eight unviable) and nine emitted struct-field mutations in other modules,
  all caught. The final campaign contains the same 50 endpoint mutations plus nine
  extra struct-field mutations, all caught. Both use --config mutants.toml,
  --gitignore true, --jobs 4 and --re scoped to the relevant files; cargo-mutants
  emits the struct-field cases outside that filter.
- Package: 198 reviewed files; 811770-byte archive, SHA-256
  a67ba8815641b138a937abb255334cf358813d8f8aa5936a1fc2c8136b6f55a4, source
  a6663a2d29119d728efc51d7294facbb765e77ea. Fresh-CARGO_HOME packaging and an
  all-feature build of the unpacked archive pass; fixtures and upstream adapter absent.

An initial mutation invocation omitted --config mutants.toml and therefore did not
activate opcua; its results are discarded. The configured campaigns above use the
repository's all-feature nextest settings. One parallel check attempt could not find
its doctest rlib during concurrent Cargo work; the final full check ran after coverage
and packaging completed and exited 0. No survivor exemption or coverage relaxation.

## Independent peer and runner regression

The pinned open62541 v1.5.9 source is
05c2b389b65cc35884eb40d800301af1f3d130cf. Measured image ID:
sha256:b02cb2416276b81a21b818847a585b45b41c7ead6aaab744209b25abba44c97e,
linux/amd64. All in-image checks and exactly six native tests pass, plus the finite
public opcua_read example. The new explicit None-channel probe decodes exactly seven
endpoints: None, and each of Basic256Sha256, Aes128_Sha256_RsaOaep and
Aes256_Sha256_RsaPss in Sign and SignAndEncrypt. Each advertises anonymous, username
and certificate token policies. This peer supplies a certificate even for its None
endpoint; the test verifies that fact without treating it as trust evidence.

The first native probe exposed that configured peer detail, and also a runner bug:
a failed nextest pipeline inside run_all's shell conditional could be masked by a
successful example. Both nextest and example pipelines now explicitly propagate
failure. A scratch cargo stand-in tests the conditional context with test failure,
example failure and wrong test count: all three exit 1 and skip subsequent commands
as required. The final six-case independent run exits 0. Docker is stopped and copied
host PKI removed. This is software interoperability evidence, not physical-device,
secure-connect or certificate-trust validation.

## Review and handoff

Self-review uses ADR-0005's single-maintainer waiver. The ADR-0034 S96 sketch predates
product edits. Shrink review removes the old activation filter and its duplicate policy
constant, reuses the authority parser, and leaves effects in the existing lane. No new
executor, task, abstraction layer, public operation or fallback is introduced. The
private GetEndpoints building block has a narrow dead-code expectation until its
S106 production caller. Failure is explicit at configuration/selection/service boundaries.

The public contract fixture now connects to the /path its responder advertises;
its previous root/path mismatch was correctly rejected by the selector. All 23
supported public contract cases still pass with the same 11 capability skips.
Hosted CI/foreign-platform evidence is not inferred from this Linux run. No hardware,
publication, tag or external messaging was performed. Next: S97 ADR acceptance and
contract-text alignment. Normal signed-off merges retain every slice commit.

Local logs: /tmp/stonecrab-s96-{check-final,coverage-final,fuzz,fuzz-clippy-stable,
regen,semver,semver-all,dependencies,machete,default-clippy,parity,lint-attrs,
package-final,packaged-all,interop-final,harness,features,mutants-configured,
endpoint-mutants-final}.log. Independent evidence is under
target/interop-evidence/open62541/; no private key is retained there.

Mutation outcome files:
- /tmp/stonecrab-s96-mutants-configured/mutants.out/outcomes.json
- /tmp/stonecrab-s96-endpoint-mutants-final/mutants.out/outcomes.json
