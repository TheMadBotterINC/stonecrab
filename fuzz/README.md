# Fuzz workspace

This directory is its own Cargo workspace (see `docs/adr/0009-test-and-example-tooling.md`).
It is excluded from the root workspace, has its own `Cargo.lock`, `deny.toml` and a
nightly toolchain pinned by date in `rust-toolchain.toml`. It is not published.

## Running

```sh
cargo install cargo-fuzz --locked      # once; CI pins 0.13.2
mise run fuzz                          # deny check, then every target for 60 s
scripts/fuzz-smoke.sh 1800             # same, 30 minutes per target (weekly schedule)
cd fuzz && cargo fuzz run <target>     # one target, until stopped
```

`scripts/fuzz-smoke.sh` runs `cargo deny --manifest-path fuzz/Cargo.toml check` first.
With no targets it prints that and exits 0. `fuzz/deny.toml` must match the root
`deny.toml` (the script checks).

## Adding a target

1. Name it `fuzz_<crate>_<parser>`, for example `fuzz_core_connection_string`.
2. Create `fuzz_targets/<name>.rs` using `libfuzzer_sys::fuzz_target!`, and add a
   `[[bin]]` section to `Cargo.toml` with `name`, `path`, `test = false`, `doc = false`,
   `bench = false`.
3. Add a path dependency on the crate under test (and drop its entry from
   `[package.metadata.cargo-machete] ignored` once used).
4. Commit a small seed corpus in `corpus/<name>/`; never include credentials or private
   device data.
5. Run `cargo fuzz run <name> -- -max_total_time=60` and `cargo deny --manifest-path
   fuzz/Cargo.toml check` before opening the PR.

## Layout

- `corpus/<target>/` seed and regression inputs (tracked).
- `artifacts/` crash outputs (ignored).
- `target/` build output (ignored).

## Findings

A crash or hang follows CONTRIBUTING "Fuzz findings": commit the input under
`corpus/<target>/` and add a regression test `fuzz_<target>_<short-hash>` in the owning
crate, merged before or with the fix.

Property tests write `proptest-regressions/` files next to the tests; those directories
are tracked in git, never ignored.


`fuzz_opcua_service` drives the S88 generated known-service dispatcher through the
hand-written external codecs. The first four bytes select the signed encoding id;
remaining bytes are a service body. Twenty-two first-party service seeds plus disabled,
unknown and oversized-array cases exercise canonical rewrite stability, counts and
shared depth bounds. Provenance and positive/negative body hex fixtures are retained
under `fixtures/opcua/services/`; no upstream captured test suite is copied here.


`fuzz_opcua_chunk_reassembly` (S94) reads three byte/count settings bytes followed by
concatenated MSG C/F/A frames. The production framer/assembler enforce small bounds;
accepted bodies must equal independent fragment concatenation, and partial/completed
bodies must stay within the selected byte limit. Seven first-party seeds cover single
and split messages, ABORT/reset, size/count limits, malformed ABORT and incomplete input.
No captured upstream traffic is used as a seed.


S95 extends `fuzz_opcua_decode` to pass accepted Variants through the production value
mapper. Selector bits choose no suffix or one of eight temporal suffixes. Accepted output
is traversed iteratively to check the aggregate value-node and List-depth bounds; codec /
unsupported / range refusals are expected. Eight new first-party seeds cover UTC tick
precision/extrema, overrides, matrices, zero axes and hostile rank/empty-tree expansion.

`fuzz_opcua_symmetric` (S102) selects a policy/mode, authenticates a bounded raw MSG/CLO
chunk with fixed public fixture nonces, then checks canonical protection and reopening
with opposite directional keys. Unauthenticated bytes never reach sequence/body parsing.
The committed seed packets are Python HMAC plus OpenSSL CBC fixtures; they contain no key
or production nonce. Includes Sign/SignEncrypt, all MSG flags, CLO and malformed input.


`fuzz_opcua_certificate` (S104) exercises canonical DER length, UTC/GeneralizedTime and EKU
parsing plus the production RSA server-leaf checks at the fixed 2026-10-30 reference date.
Nine seeds include independently generated OpenSSL RSA PKCS#1/PSS public certificates
(valid 2026-01-01 to 2036-01-01), an expired public certificate (2001 to 2002), malformed
DER, raw time values and an EKU. Private keys were temporary and discarded; no private
key, production identity or upstream captured certificate is committed. The connection
string corpus also covers the S104 trust-store formats and explicit verification opt-in.

`fuzz_opcua_asymmetric` (S105) creates an ephemeral RSA key once per process and substitutes
its SPKI into S104's public OpenSSL certificate template. The template is a parsing/public
key carrier, not a trusted certificate or production identity. All three policies exercise
raw ciphertext, authenticated bounded payload round trips, input-directed wire mutations
and both padding-size formats. Seven seeds cover policy selectors, truncation, declared
size, padding residues and a large body. No private fixture or new dependency is added.
