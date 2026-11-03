# S88 service-codec evidence

Measured 2026-11-02 on Linux x86_64, Rust 1.99.1. Branch
`feat/opcua-service-codecs`, starting from master `3151dfb`.

The 22-case connection/read subset uses the three approved PLC4X models pinned to
`ea229918f5d07700e04d7d69ab1645da679cb700`. Paths, SHA-256 hashes and Git blobs are
recorded in `upstream/manifest.toml`. NOTICE includes the ASF attribution and the OPC
Foundation MIT notice; the moving NodeSet model version used upstream remains unknown.
The generated numeric-enum name lookup is generic, tested by generator fixtures, and
bumps the tool to 0.3.0. The library stays at its published 0.3.0 version.

Each enabled service has an independent first-party valid vector and a truncated
negative vector, with provenance under `fixtures/opcua/services/`. Runtime tests compare
the constructions with those files, decode/re-encode each case and its Known envelope,
reject every truncated prefix, and exercise unsupported/raw bodies, malformed recognized
bodies, oversized counts, identity flags and the shared recursion budget. Accepted depth
64 decoding/encoding also passes on a 2 MiB stack. No upstream capture is represented as
first-party evidence, and no network service capability is advertised.

| Check | Result |
| --- | --- |
| `mise run check` | Passed; 2123 nextest tests, 74 external-peer tests skipped; 583 merged + 5 separate doctests |
| Generator tests and strict Clippy | 52 tests passed; no warnings |
| `mise run regen` | Two byte-identical generations per protocol; hash and edited/added/removed-output self-checks passed |
| Handwritten coverage | OPC UA 5039/5039, 100%; core, codec, sim, Modbus, sensors and boards 100%; transport 99.31%, testkit 97.68%; thresholds unchanged |
| Feature powerset | All 59 combinations and no-default-feature check passed |
| Fuzz smoke | All 14 targets passed, 10 seconds per target |
| Service fuzz campaign | 4,673,068 executions in 61 seconds; no crash |
| Fuzz-only strict Clippy | Passed with `RUSTFLAGS='--cfg fuzzing'` |
| Semver against published 0.3.0 | Default and all-features passed, no update required |
| Parity and lint attributes | Passed; 255 implemented/automated-verified rows resolve |
| Package | Verified archive build; reviewed list matches; 181 files, 742,400 bytes compressed (3.1 MiB unpacked) |

The package includes production generated sources, not models, fixtures, generator or
fuzz data. Bit-run helpers moved from Modbus into the shared codec module; the old module
was deleted, and Modbus generated equivalence tests still pass. Public API and runtime
dependencies are unchanged. No hardware or independent server was exercised in S88.

Local logs are `/tmp/stonecrab-s88-{quality,coverage,features,regen,semver,package,fuzz}.log`
and `/tmp/stonecrab-s88-service-fuzz60.log`. These are local artifacts, not hosted CI.

Self-review under ADR-0005's single-maintainer waiver: no alternate codec or runtime
layer; generated code changes are isolated in `gen:` commits; unsupported dispatch is
distinct from malformed known input; parse/serialize/length share bounds; no public
support claim precedes the channel, session and Read milestones. Integration will use
ordinary merge commits preserving this branch's commits, per the maintainer's instruction.
