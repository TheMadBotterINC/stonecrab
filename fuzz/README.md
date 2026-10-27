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
