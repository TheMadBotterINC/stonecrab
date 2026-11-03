# S99 credential channel and explicit plain username identity

Date: 2026-11-03. Design: [ADR-0037](../adr/0037-credential-channel-and-plain-opcua-identity.md),
amending ADR-0034 decision 8 under E18/E22. Crate stays stonecrab 0.3.0; opcua stays
opt-in outside default/full. No new dependency or runtime task.

## Implemented behavior

Registry connect_with attaches credentials separately after driver/URI validation. The
cache builder provider resolves them outside locks only for new connections, including
replacement. None preserves configured identity/options, explicit registry credentials
replace configured credentials, provider errors fail without an anonymous retry, and reuse
retains the credential-free URI key. Provider Debug reveals only presence. Each cache
has one identity policy; remove/close before rotation. Callback work is nonblocking and
its errors must satisfy the existing PlcError redaction contract.

Published Credentials is a non-exhaustive struct with public username/password fields.
A private discriminator and additive CredentialIdentity/X509 file paths preserve this
source API. Credentials::x509 does no I/O; both paths are redacted by Debug. Password
remains Secret; complete config Debug/display hide the whole identity. Direct username
Credentials Debug retains its existing documented username visibility. X509 activation
is explicitly refused before I/O until S108; file loading/trust remains S103/S104.

OPC UA resolves absent or empty-pair credentials as anonymous. Password without username
is InvalidConfig. Username over None requires allow-insecure-credentials=true, default
false and strict boolean, before transport I/O. Selection requires the exact credential
token type and retains the S96 ranking. The selected effective token policy must be None;
null/empty inherits the endpoint, protected policies are refused without weaker fallback.
The generated UserNameIdentityToken carries selected policy ID, username, UTF-8 password
bytes, and null encryptionAlgorithm, with no appended nonce. A static warning contains
no identity or token values. Activation identity/access refusals, including ServiceFault,
map to Authentication while preserving the exact device source. Other failures retain
their prior classifications. URI username/password keys remain refused under E22.

Eight protocol rows close; 249 OPC UA rows now comprise 115 implemented, 96 planned and
38 deferred. Core registry/config and cache contracts record the additive APIs. Protected
username tokens and X509 activation remain S108, writes S100, secure channel S101 onward.

## Validation

- Full check: 2351 tests, 588 normal and five compile-fail doctests; strict all-target,
  all-feature Clippy, fmt, rustdoc and no-default-features build pass.
- Coverage: core 6973/6973, codec 245/245, sim 2307/2307, modbus 6699/6699,
  sensors 1840/1840, boards 511/511, handwritten OPC UA 7382/7382, all 100%.
  Transport 2877/2897 (99.31%, floor 95), testkit 5278/5394 (97.85%, floor 90).
  Floors/generated exclusions are unchanged. The initial missing core formatter branch
  is covered by an explicit assertion on the existing username identity Debug contract.
- Identity/boolean mutations: 21 tested, 16 caught/five unviable. Core credential,
  registry and cache campaign: 33 tested, 15 caught/18 unviable. Both exit 0, no
  survivors, timeouts or new exclusions. Use --config mutants.toml, --gitignore true,
  jobs 4, debug info disabled, and --re limited to changed functions. Cargo-mutants also
  emits existing struct-field cases outside the regex, all caught. Unsupported default
  replacements account for unviable cases; no Default implementation was added to secrets.
- Regeneration/self-test pass: generator tests/lint/deny, vendored hashes, two identical
  runs and unchanged generated trees. UserNameIdentityToken was enabled and regenerated
  in its own gen commit. Independent complete/truncated service vectors and corpus seeds
  cover the added body; the service dispatch has 25 cases.
- Default/all-feature semver, default and fuzz Clippy, dependencies/machete,
  parity/self-test and attribute lint pass. Remaining final gates and archive identity
  are recorded below after completion.

## Independent peer

Pinned open62541 v1.5.9 source 05c2b389b65cc35884eb40d800301af1f3d130cf;
linux/amd64 image ID sha256:0d608ca350622734e680610a1e2f74160ac44327daf150258cb700259e4a019c.
All in-image checks, exactly eight native tests and the finite public Read example pass.
The native server explicitly enables its existing OPCUA_ALLOW_NONE_PASSWORD setting;
the separate in-image security checks retain their prior settings. The public registry
path refuses username without the client opt-in, accepts the runtime username/password
with it, closes normally, and classifies a wrong password as Authentication. The password
comes from the ignored copied PKI, never URI or evidence. Error text is checked against
the actual runtime password. No password/key is printed or committed. Docker is stopped
and copied host PKI removed. This proves software-peer plain identity behavior, not
physical-device, secure-identity or certificate-trust operation.

## Review and handoff

Self-review uses ADR-0005's single-maintainer waiver. The design preceded product code.
Shrink review retains one private pure identity module for resolution/token construction
and one callback at the existing cache-open boundary. Existing registry parsing,
ConnectionConfig secret storage, endpoint ranking, service exchange and device sources
are reused. The old blanket credential refusal is removed. Published field access earns
the private discriminator; no compatibility alias, executor, dependency or hidden I/O.

Next: S100, staged OPC UA writes. Normal signed-off merge preserves every commit.
No hardware, tag/release or external messaging was performed.

Logs: /tmp/stonecrab-s99-{check,coverage-final,identity-mutants,core-mutants,regen,
interop,semver-final,semver-all,dependencies-final,machete-final,parity-final,
lint-attrs-final,default-clippy,fuzz-clippy}.log. Mutation outcomes:
/tmp/stonecrab-s99-identity-mutants/mutants.out/outcomes.json and
/tmp/stonecrab-s99-core-mutants/mutants.out/outcomes.json.
Independent peer evidence: target/interop-evidence/open62541/ (no private keys).
