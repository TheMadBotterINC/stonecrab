# S98 Bytes, bounded structures and read metadata evidence

Date: 2026-11-03. Base: `5d5e859`. Design: `f6dc767` (ADR-0036), core additions
`150927a`, mapping/metadata `d578480`, public-contract import `a28da1e`, depth-boundary
regressions `0211f9b`, contracts/parity `1050265`, final ADR/peer handoff `d0830e9`.

## Implementation and contract

Bytes and Struct append to PlcValue/PlcValueType after List. Existing type ordering and
all previous representations remain unchanged; PlcValue remains only PartialEq, with
no numeric/Boolean coercion for either new kind. Bytes holds exact octets, distinct
from Null and List of U8. Public PlcStruct owns immutable ordered unique fields, with
fields/get/into_fields accessors. Construction permits at most 1024 fields per structure,
64 composite levels through lists/structures, and 1048576 scalar/container nodes inside
its tree. Validation checks child counts before expanding a bounded iterative work list;
rejected trees are also disposed iteratively. A 100000-level caller-created List is
rejected on a 128 KiB test-thread stack without recursive drop. No mutable field escape,
map dependency, public builder layer, configurable limit or new feature is introduced.
Existing independent top-level Lists and byte/string payloads remain caller-owned data;
protocol codecs bound wire payloads before this layer.

ByteString moves its already-decoded bytes into Bytes without a clone; null becomes Null
and empty becomes empty Bytes. LocalizedText becomes exactly text then locale fields,
with missing/null parts Null, explicit empty parts empty String, delimiters unchanged,
and invalid UTF-8 a per-item failure. Arrays/matrices retain shape. Scalar localized
values cost three response nodes; arrays preflight the extra two field nodes per leaf,
and their structure counts toward the 64-level composite bound. Failed mappings leave
the shared response budget intact. Deferred complex kinds remain named item errors.

ReadItem gains with_server_time/server_time, independent from source_time and monotonic
sampled_at, defaulting to None. OPC UA retains exact original ProtocolStatus on both
successful values and failures and attaches both optional timestamps. Good/Uncertain
values keep their value; Bad retains the existing item failure precedence even if a
peer supplies a value, per Part 4. No failed value is relabeled Good. Absent/zero or
platform-unrepresentable timestamps stay None. The shared conversion clamps encoded
10 ps refinements to 9999, truncates to whole nanoseconds, and ignores refinements
without a timestamp. The sub-nanosecond remainder cannot fit SystemTime and is not exposed.

ADR-0036 amends ADR-0006/ADR-0025 and implements approved E18. Six OPC UA rows close
(249 total: 107 implemented/104 planned/38 deferred), CORE-VALUE-BYTES closes, and the
Stonecrab-owned CORE-VALUE-STRUCT contract records bounds. Global parity references
increase from 325 to 333. Custom schema decoding remains S119/S120. No credentials,
secure discovery, native subscription, dependency, version bump or publication is added.
The simulator/random/status-word consumers explicitly refuse the new nonnumeric types;
Modbus wire encodings remain unchanged. Public API additions remain additive at 0.3.0.

## Validation

- Full check: 2335 tests, 586 normal and five compile-fail doctests; fmt, strict
  all-target/all-feature Clippy, rustdoc and no-default-features build pass.
- Coverage: core 6916/6916, codec 245/245, sim 2307/2307, modbus 6699/6699,
  sensors 1840/1840, boards 511/511 and handwritten OPC UA 7276/7276 (all 100%).
  Transport 2875/2897 (99.24%, floor 95), testkit 5278/5394 (97.85%, floor 90).
  Every threshold and generated-code exclusion remains unchanged. A missing nested
  Struct rejection edge was covered with exact/over mixed-container depth tests;
  an unnecessary panic branch in a fixture assertion was removed.
- All 59 feature combinations plus the no-default-features check pass, in isolation
  after all other Cargo work completed. Manifests restored cleanly.
- All 15 fuzz targets pass with a 10-second budget per target on the pinned nightly.
  New decode seeds cover null/empty/octet ByteStrings, localized fields and rank 63/64
  matrices. The existing mapped-value graph checker now traverses structured fields
  and counts them in the same node/depth invariants. Default and --cfg fuzzing Clippy
  pass on pinned stable 1.99.1.
- Configured structure/read/value mutation campaign: 108 mutants, 85 caught and
  23 unviable; metadata accessor campaign: 12 mutants, 10 caught and two unviable.
  Both exit 0, with no survivors, timeouts or new exclusions. Scope uses --config
  mutants.toml, --gitignore true, jobs 4/2, debug info disabled as in scripts/mutants.sh,
  and --re scoped to the changed functions/files. The larger run includes 18 core
  structure cases (13 caught/five unviable), 52 value cases (41/11), 32 read cases
  (25/seven, including three emitted struct-field cases), and six emitted struct-field
  cases elsewhere, all caught. The metadata run has three accessor/constructor cases
  (one caught/two unviable) and nine emitted struct-field cases elsewhere, all caught.
  Later depth-coverage tests strengthen the suite without changing runtime code.
- Initial mutation baselines could not compile a newly extended public-contract test
  because its PlcStruct import was missing. That import was fixed; those baseline
  failures are not mutation gates. The complete clean campaigns above supersede them.
- Regeneration/self-test pass for both protocols, including generator tests/lint/deny,
  vendored hashes, deterministic output and clean generated trees. No generated code
  changed. Parity/self-test, attribute lint, dependencies/machete, default/all-feature
  semver and signed-off commit checks pass.
- Package: the reviewed inventory remains 198 files; fresh-CARGO_HOME package verification
  and unpacked all-feature build pass. Source d0830e9e3abd4de2405b7d5b745b7c1173996867,
  archive 818251 bytes, SHA-256
  52fd945075e435b80b70140ae3eed5626c9d5c487859a71ea10786bea117ddea.
  No fixture, XML adapter or raw vendored model enters the crate archive.

## Independent peer

Pinned open62541 v1.5.9 source 05c2b389b65cc35884eb40d800301af1f3d130cf;
measured linux/amd64 image ID
sha256:2cb980e7956bca895c85328ba329922e135a3ab235e2cb952195493d417d7192.
All in-image checks, exactly seven native tests and the finite public example pass.
The new public Read test verifies bytes 00/01/FE/FF, separate text/en-US locale fields,
Good item status and independently present source/server timestamps. The 32-tag extended
Read now has 28 mapped results and four named refusals, retaining all large arrays and
matrix checks. Source/server timestamp failure/absence/refinement behavior is proven by
deterministic tests, not inferred from the peer's successful values. The server namespace
is unchanged. Docker is stopped and copied host PKI removed. This is software-peer
interoperability evidence; no physical device, secure identity or custom schema claim.

## Review and handoff

Self-review uses ADR-0005's single-maintainer waiver. The design was committed before
code. Shrink review retains only the immutable wrapper needed to enforce Struct bounds;
the private child-expansion helper has two traversal callers, and iterative rejection
prevents a new constructor from recursively dropping invalid old List trees. Existing
value conversion, status and DateTime machinery is reused; named deferred mapper branches
are replaced rather than kept as alternate paths. No executor, dependency or hidden I/O.

Next: S99, credential API/channel additions. Normal signed-off merges preserve all commits.
No hardware, release/tag or external messaging was performed.

Logs: /tmp/stonecrab-s98-{check,coverage-final,features,fuzz,fuzz-clippy,default-clippy,
regen,semver,semver-all,dependencies,machete,parity,lint-attrs,package,packaged-all,
interop-final,mutants-final,metadata-mutants-final,dco}.log. Complete mutation JSON files:
/tmp/stonecrab-s98-mutants-final/mutants.out/outcomes.json and
/tmp/stonecrab-s98-metadata-mutants-final/mutants.out/outcomes.json.
Independent peer evidence: target/interop-evidence/open62541/ (no private keys).
