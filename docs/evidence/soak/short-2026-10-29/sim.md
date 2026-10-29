# Soak run: sim

| Item | Value |
| --- | --- |
| Date (UTC) | 2026-10-29T06:04:30Z |
| Tree | commit `bfff873` plus uncommitted changes |
| Machine | Apple M2 Max, Darwin 25.6.0 arm64 |
| Toolchain | rustc 1.99.1 (58cf8f9c3 2026-10-13) |
| Build | `cargo build --release --locked -p stonecrab --example soak --features full` |
| Command | `scripts/soak.sh --interval 5 --period-ms 0 --max-growth-kib 4096 sim 600` |
| Target | `sim://soak` |
| Duration | 600 s, one memory sample every 5 s (119 samples) |
| Workload | four tags per read, a 0 ms pause after each read |
| Faults | one failure step every 5 s (`STONECRAB_SOAK_FAULT_SECS`; 0 for none) |

## Result

| Measure | Value |
| --- | ---: |
| Read calls (total) | 669161300 |
| Errors (failed reads, failed items, failed reopens; total) | 119 |
| Reconnects (total) | 40 |
| Read calls / errors in the last quarter | 163105604 / 29 |
| RSS minimum / maximum after the warm-up (KiB) | 2640 / 2816 |
| Warm-up, not judged (s) | 119 |
| RSS median of the first / last quarter after the warm-up (KiB) | 2736.0 / 2816.0 |
| Growth (KiB), bound 4096 | 80.0 |
| Least-squares slope after the warm-up (KiB per hour), informational | 1004.6 |
| Example exit status | 0 |

**Verdict: PASS, memory stable.** The samples are in `sim.csv`.

A short run: with a bound of 4096 KiB over 600 s only a leak above about 40 MiB an hour fails. The verdict shows that the command works, not that nothing small leaks; the 24 hour run (86400) does.
