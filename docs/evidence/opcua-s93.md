# S93 upstream OPC UA vector evidence

Measured 2026-11-03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch
`test/opcua-upstream-vectors`, baseline master `7d91ba7` (S92). Corpus implementation
`a02adc4`; final packaged source/docs `048fa8c`.

Two Apache PLC4X suites are retained verbatim at commit
`ea229918f5d07700e04d7d69ab1645da679cb700`. Their ASF headers, source paths, git blobs,
SHA-256 fingerprints and license attribution are checked into upstream/manifest.toml
and fixtures/opcua/upstream/PROVENANCE.toml. Pinned GitHub contents-API blob identifiers
were compared with local git hash-object at acquisition; offline tests check both
retained files against their recorded SHA-256 and manifest entries. Root and crate
NOTICE carry the attribution. E13 part B was already accepted.

The test-only XML adapter uses existing dev dependencies and derives bytes independently
of Stonecrab encoders. All 11 raw values match their upstream literals exactly. The 12
driver messages follow annotated leaf order, continuous sub-byte flags, little-endian
numbers, UTF-8 and literal byte payloads. Serializer-computed implicit String/message
lengths replace seven stale annotations across three driver messages; their exact closed
list is tested and recorded in expected JSON. Original XML is unchanged. This does not
relax runtime malformed-input handling.

The second pass exercises the private built-ins, framer and generated service subset:

- 15 byte-exact round-trips, including an explicitly opaque id-811 notification envelope;
- one named canonical round-trip: the Read request's namespace-0 numeric NodeId 2258
  becomes the existing compact encoding, reducing the frame from 131 to 128 bytes;
- seven exact disabled-service refusals, matched by vector name, discriminator field
  and encoding id, with no catch-all refusal allowance.

The independent canonical expectation, semantic Read target and idempotent re-encoding
are checked. OPCUA-DIV-NUMERIC-NODE-ID-ENCODING records this existing behavior. ADR-0034
and the slice exit condition record the refinement from universal byte equivalence.
The seven refusals belong to S100/S114/S116 (event content S118); S123 must empty the
list and replace the id-811 Raw assertion with semantic notification coverage. The
HEL/ACK parity row is implemented; the full fourteen-service row stays planned.

See [corpus details and reproduction](../../fixtures/opcua/UPSTREAM.md) for all seven
length corrections, refusal names and owning slices. The original capture attribution
to Milo remains **UNVERIFIED-capture**. This corpus is not independent live-peer evidence.

## Validation

| Check | Result |
| --- | --- |
| `mise run check` | Passed: format, strict all-target/all-feature Clippy, nextest, cargo test/doctests, warnings-denied docs and no-default-feature build |
| Nextest | 2239 passed; 77 external-peer tests skipped in the normal suite |
| Doctests | 584 merged + 5 separate passed |
| Corpus tests | Seven pass: exact file set/output, pinned fingerprints, equivalence/refusals, HEL/ACK, adversarial annotations/partial fields, exact implicit corrections and canonical Read semantics |
| Coverage | OPC UA 6544/6544 (100%); core 6671/6671, codec 245/245, sim 2303/2303, Modbus 6699/6699, sensors 1840/1840, boards 511/511; transport 2877/2897 (99.31%), testkit 5278/5394 (97.85%); every floor unchanged and met |
| Features | All 59 combinations and no-default-feature check passed, isolated from other Cargo operations |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Semver | Default and all features against published 0.3.0 passed |
| Dependencies / machete | Advisory/license/ban/source checks passed; no unused dependencies |
| Parity / lint attributes | Passed with self-tests; 289 implemented/automated-verified rows referenced |
| Package | Verified reviewed 192-file list; upstream XML, adapter and fixtures absent; unpacked archive builds with all features |

The initial full check caught a missing rustdoc backtick in the component document;
it was corrected and the entire check passed on rerun. No product code, dependency,
feature, code-generator output or version changed. The test module inclusion is the
only src change. Fuzz, mutations and native-server interop were not rerun for this
fixture-only slice; S92 records the latest product-code campaigns and peer results.
No hardware was available and no crate or tag was published.

Self-review under ADR-0005's single-maintainer waiver: source provenance and ASF headers
are retained; the adapter never uses Stonecrab serialization to generate expectations;
exact file/output/fingerprint checks prevent silent corpus drift; unknown annotation
shapes fail loudly; refusal errors are exact and future support must remove entries;
opaque preservation and canonicalization are explicit rather than claimed as full
semantic/byte parity. Runtime architecture and limits remain unchanged. Package exclusion
and all-feature unpacked build were verified. Ordinary signed-off merges preserve commits.
Hosted CI will be triggered on push; no hosted result is assumed here.

Logs: `/tmp/stonecrab-s93-{vectors6,clippy4,quality-complete,coverage,features,regen,
semver,semver-all,dependencies,machete,parity,lint-attrs,package,packaged-all}.log`.
