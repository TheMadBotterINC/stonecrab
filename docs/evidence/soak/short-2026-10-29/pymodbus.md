# Soak run: pymodbus

| Item | Value |
| --- | --- |
| Date (UTC) | 2026-10-29T06:04:36Z |
| Tree | commit `bfff873` plus uncommitted changes |
| Machine | Apple M2 Max, Darwin 25.6.0 arm64 |
| Toolchain | rustc 1.99.1 (58cf8f9c3 2026-10-13) |
| Build | `cargo build --release --locked -p stonecrab --example soak --features full` |
| Command | `scripts/soak.sh --interval 5 --restart-every 120 --period-ms 10 --max-growth-kib 4096 pymodbus 600` |
| Target | `modbus-tcp://127.0.0.1:5021?request-timeout-ms=500`, container from `stonecrab-interop:local`, restarted 4 times |
| Duration | 600 s, one memory sample every 5 s (118 samples) |
| Workload | four tags per read, a 10 ms pause after each read |
| Faults | the container restarted every 120 s (0 for none) |

## Result

| Measure | Value |
| --- | ---: |
| Read calls (total) | 36220 |
| Errors (failed reads, failed items, failed reopens; total) | 7 |
| Reconnects (total) | 4 |
| Read calls / errors in the last quarter | 9071 / 1 |
| RSS minimum / maximum after the warm-up (KiB) | 2720 / 2752 |
| Warm-up, not judged (s) | 119 |
| RSS median of the first / last quarter after the warm-up (KiB) | 2736.0 / 2752.0 |
| Growth (KiB), bound 4096 | 16.0 |
| Least-squares slope after the warm-up (KiB per hour), informational | 140.1 |
| Example exit status | 0 |

**Verdict: PASS, memory stable.** The samples are in `pymodbus.csv`.

A short run: with a bound of 4096 KiB over 600 s only a leak above about 40 MiB an hour fails. The verdict shows that the command works, not that nothing small leaks; the 24 hour run (86400) does.
