# Fuzz evidence

Slice S62 (ADR-0017 addendum "S62"). The recorded fuzz run of all targets, the command
for the 8-hour release run, and one 5-minute run that shows the command works. The alpha's
30-minute runs are in `docs/evidence/alpha.md`; the weekly `scheduled` job runs
`scripts/fuzz-smoke.sh 1800` (one target after another).

## The release run

```sh
caffeinate -i mise run fuzz-evidence -- 28800     # same as scripts/fuzz-evidence.sh 28800
```

`caffeinate -i` (macOS) keeps the machine from sleeping; omit it on Linux. The script
builds the seven targets once with the nightly of `fuzz/rust-toolchain.toml` (rustup
installs it on demand; `cargo-fuzz` must be on `PATH`), then runs all of them at once, one
single-threaded libFuzzer process each, for 28800 s. It needs seven free cores and under
1 GB of disk (the build measured 350 MB, plus the scratch corpora); each target works on a
scratch copy of its seed corpus under `fuzz/target/evidence-corpus/`, so the committed
`fuzz/corpus/` is never touched (ADR-0009). Logs, the exit status and the seed count of
every target are in `fuzz/target/evidence-logs/`.

When it ends, the script prints the report and writes it to
`fuzz/target/evidence-logs/report.md`. To record the run:

```sh
cp fuzz/target/evidence-logs/report.md docs/evidence/fuzz/8h-YYYY-MM-DD.md
```

## The report

A table of the run (date, tree, machine, nightly compiler, cargo-fuzz version, the
command, the duration) and one row per target:

| Column | Meaning |
| --- | --- |
| Seed inputs | files in the target's committed seed corpus |
| Runs | libFuzzer's final run count (`#N DONE`) |
| Cov | coverage counters reached (`cov:`) |
| Ft | features reached (`ft:`) |
| Corpus | inputs kept and their size (`corp:`) |
| Exec/s | executions per second at the end |
| Seconds | wall-clock seconds of the target's process |
| Exit | the process exit status |
| Artifacts | files left under `fuzz/artifacts/<target>/` (a crash, a timeout or an out-of-memory input) |

## Pass criteria

The run passes when every target exited 0 after at least the requested number of
seconds, executed at least one input and left no crash artifact. The script exits 1
otherwise and the last line of the report says FAIL. A failing input is then in
`fuzz/artifacts/<target>/`: reproduce it with `cargo fuzz run <target> <file>` from
`fuzz/`, fix the cause, add the input to `fuzz/corpus/<target>/` and run again.

## The 5-minute run of 2026-10-29

`5min-2026-10-29.md`: `scripts/fuzz-evidence.sh 300` on the S62 tree (commit `bfff873` plus
the uncommitted S62 changes, none of them in `crates/stonecrab/src` or `fuzz/`). All seven
targets ran 302 to 304 s, exited 0 and left no artifact; between 4.0 and 9.3 million
executions each. The machine was also running the 10-minute soak runs and a build while
this ran (`docs/evidence/soak/`), so the executions per second are lower than an idle
machine gives. This run proves that the command, the report and the pass criteria work;
it is not release evidence for fuzzing depth. The 8-hour run is.
