# S94 OPC UA chunking and negotiated-limit evidence

Measured 2026-11-03 (America/New_York), Linux x86_64, Rust 1.99.1. Branch
`feat/opcua-chunking`, baseline master `311e6b2` (S93). Final runtime implementation
`9679817`; packaged source/docs and reviewed list `e8a8bfe`.

Policy-None MSG bodies now split into C chunks followed by F, with a constant request id
and a new sequence per chunk. The whole body, chunk count and available sequence range
are checked before any send. One transport lease owns the entire send; interruption
fails the lane without replay. OPN/CLO remain single-frame protocol messages.

Each pending request owns its partial assembly. Channel/token/sequence validation
precedes append, size/count bounds precede allocation, and aggregate partial bodies
share ResourceLimits.max_buffered_bytes. Receive cancellation preserves other pending
assemblies; request drop releases its own, and terminal failure releases all partial
bodies. Unknown request ids are sequence-checked and discarded without assembly growth.
ABORT validates its body, discards its partial message and delivers the exact peer status
through StatusFault to only that request. Reason text is not echoed or retained in the
error; ABORT consumes neither body nor chunk allowance. Other requests remain usable.

All four encoding.* options parse under their prefixed names. Buffer defaults remain
65535; the non-ECC minimum is 8192 and maximum Int32::MAX. Configured message/chunk
defaults remain 2097152/64, each positive UInt32. Hello clamps buffers to the frame budget
and body bytes to the local receive budget (262144 by default). Negotiation retains the
existing conservative minimum of local/peer limits, with peer zero imposing no additional
limit. Headers, signatures and sequence fields do not count as message body bytes.
The size table models signature, plain/cipher block and padding sizes; only None executes
at runtime (capacity is send buffer minus 24). Secure transforms/rows remain S101.
No public API, dependency, feature or version changes; public array mapping remains S95.

The design sketch and reviewed OPC Foundation sources are in
[ADR-0034 S94](../adr/0034-opcua-client-design.md#s94-implementation-sketch-2026-11-03).
The online reference redirected to v1.05.07; no upstream chunk calculation fixtures or
implementation text were copied. Eleven assigned parity rows are implemented, with
secure execution still explicitly limited to its later slices.

## Validation

| Check | Final result |
| --- | --- |
| `mise run check` | Passed: format, strict all-feature/all-target Clippy, nextest, cargo test/doctests, warnings-denied docs and no-default-feature build |
| Nextest | 2265 passed; 78 external-peer tests skipped in the ordinary suite |
| Doctests | 584 merged + 5 separate passed |
| Coverage | OPC UA 6743/6743 (100%); core 6671/6671, codec 245/245, sim 2303/2303, Modbus 6699/6699, sensors 1840/1840, boards 511/511; transport 2877/2897 (99.31%), testkit 5277/5394 (97.83%); every floor unchanged and met |
| Features | All 59 combinations and no-default-feature build passed in isolation from other Cargo operations |
| Default / fuzz-only Clippy | Passed, including all-target/all-feature `RUSTFLAGS='--cfg fuzzing'` |
| Fuzz | All 15 targets passed with a 10-second budget (11 reported); final refined assembler also passed 1234367 inputs over 31 seconds; no crashes |
| Mutations | Final 35 total: 33 caught, 2 unviable, no survivors/timeouts; unmutated baseline passed |
| Regeneration | Both protocols identical twice; edited/added/removed-output self-tests passed |
| Semver | Default and all features against published 0.3.0 passed |
| Dependencies / machete | Advisory/license/ban/source checks passed; no unused dependencies |
| Parity / lint attributes | Passed with self-tests; 300 implemented/automated-verified rows referenced |
| Package | Verified reviewed 194-file archive, 792997 compressed bytes at e8a8bfe; upstream fixtures/XML/adapter absent; unpacked archive also builds with all features |

Deterministic coverage includes a 1000-node Read sent as three correctly ordered chunks,
exact NodeIds after reassembly, per-chunk headers and bounds, preflight refusal without
bytes or consumed sequences, split response fields, multiple outstanding requests,
request-local ABORT with a partial body and long ignored reason, chunk/byte-limit failure,
aggregate receive exhaustion, partial receive cancellation/drop, abandoned continuations,
sequence exhaustion and unchanged partial-send failure behavior. Configuration tests cover
prefixes, all defaults, positive boundaries, invalid values and resource-capped Hello.
Pure tests exercise exact growth/overflow and real TryReserveError via usize::MAX without
exhausting memory. Seven first-party fuzz seeds cover complete/split/aborted messages,
byte/count limits, malformed ABORT and incomplete input.

The first coverage run reported 6740/6743: overflow and allocation-error closures had no
regressions. Shared checked growth and a small allocator boundary made those paths
independently testable; the full rerun reached 6743/6743 with no exclusion/floor changes.
The first mutation campaign on the initial implementation passed (31 total, 29 caught,
2 unviable); the final campaign passed after the boundary refinements. Of the final 35,
26 are in chunk.rs (24 caught, 2 unviable); cargo-mutants also emitted nine configuration /
read struct-field mutations outside the name filter, all caught. No survivor exclusions
were added. The first package-list comparison caught lexical sorting that differed from
Cargo's ordering; the reviewed list was restored to its existing order with exactly the
two new chunk source files, then the full package gate passed.

## Independent peer

The in-image open62541 checks passed. The final native nextest filter selected exactly
four cases, all passed: HEL/OPN/CLO with counter handoff; public anonymous session
connect/close; public scalar Read/unknown node; and the new chunked private-lane test.
The latter negotiates 8192-byte send/receive buffers, 24576 body bytes and four chunks.
All 4000 Int32 values equal their array indices, with two response chunks; a 600-node
scalar Read returns every expected value, with three request chunks. Session close
succeeds afterward. The finite public opcua_read example also passes. Public array value
mapping is not claimed by the private-lane array check.

Source `05c2b389b65cc35884eb40d800301af1f3d130cf` (v1.5.9); image
`sha256:91d0a81333b86964e333c86bdb1bd53bc108770c303b44e70ce7aa9055261bcf`.
Evidence: `target/interop-evidence/open62541/{image-digest.txt,check.txt,nextest.txt,example.txt,server.log}`.
The peer was stopped and host test PKI removed. The peer's secure-policy checks validate
the scaffold, not Stonecrab secure chunk execution. No hardware, key commits/uploads,
crate publication or tag publication occurred.

Self-review under ADR-0005's single-maintainer waiver: the pure chunk module uses the
existing lane/framer/service boundaries; no second pending map, executor task, new trait
or dependency was introduced. Per-request assemblies live directly in the bounded
pending table, avoiding a second aggregate counter to synchronize. Preconditions precede
mutation/I/O; boundary errors leave partial bytes unchanged. Existing transport leases
retain cancellation/drop behavior. The exact-count CI filter and peer evidence now cover
both directions of chunking; package exclusions and API compatibility were checked.
Normal signed-off merges preserve every commit. Hosted CI is triggered on push; no hosted
result is assumed here.

Logs: `/tmp/stonecrab-s94-{focused4,clippy4,quality-complete,coverage-complete,features,
fuzz-clippy-complete,default-clippy,fuzz,chunk-fuzz-complete,mutants-complete,regen,
semver,semver-all,dependencies,machete,parity,lint-attrs,package-complete,packaged-all,
interop-complete}.log`.
Final mutation outcomes: `/tmp/stonecrab-s94-mutants-complete/mutants.out/outcomes.json`.
