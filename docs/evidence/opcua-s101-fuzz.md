# Fuzz evidence run

| Item | Value |
| --- | --- |
| Date (UTC) | 2026-11-03T12:26:45Z |
| Tree | adb9912 |
| Machine | AMD Ryzen 9 9950X 16-Core Processor, Linux 7.2.6-76070206-generic x86_64 |
| Nightly | rustc 1.101.0-nightly (21b707e3f 2026-09-30) |
| cargo-fuzz | cargo-fuzz 0.13.2 |
| Command | `scripts/fuzz-evidence.sh 60` |
| Duration | 60 s per target, all targets concurrently, 62 s wall-clock after the build |

| Target | Seed inputs | Runs | Cov | Ft | Corpus | Exec/s | Seconds | Exit | Artifacts |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `fuzz_codec_bounded_reader` | 5 | 3266667 | 198 | 1090 | 260/4590b | 53551 | 62 | 0 | 0 |
| `fuzz_core_connection_string` | 5 | 3067655 | 452 | 1585 | 389/72Kb | 50289 | 62 | 0 | 0 |
| `fuzz_modbus_address` | 11 | 2830658 | 991 | 2932 | 660/15480b | 46404 | 62 | 0 | 0 |
| `fuzz_modbus_ascii` | 18 | 2310568 | 213 | 1235 | 209/92Kb | 37878 | 62 | 0 | 0 |
| `fuzz_modbus_connection_string` | 16 | 2858517 | 1574 | 3539 | 905/106Kb | 46860 | 62 | 0 | 0 |
| `fuzz_modbus_mbap` | 13 | 3793110 | 112 | 524 | 21/11359b | 62182 | 62 | 0 | 0 |
| `fuzz_modbus_pdu` | 22 | 12143741 | 1209 | 1656 | 204/4138b | 199077 | 62 | 0 | 0 |
| `fuzz_modbus_rtu` | 11 | 1565704 | 227 | 1395 | 437/66Kb | 25667 | 62 | 0 | 0 |
| `fuzz_opcua_address` | 21 | 1497392 | 1388 | 4729 | 1321/294Kb | 24547 | 62 | 0 | 0 |
| `fuzz_opcua_chunk_reassembly` | 7 | 3601111 | 569 | 1560 | 406/63Kb | 59034 | 62 | 0 | 0 |
| `fuzz_opcua_connection_string` | 45 | 2591453 | 1296 | 3022 | 846/120Kb | 42482 | 62 | 0 | 0 |
| `fuzz_opcua_decode` | 50 | 3021312 | 4821 | 18879 | 3235/286Kb | 49529 | 62 | 0 | 0 |
| `fuzz_opcua_frame` | 27 | 1953900 | 491 | 2119 | 177/33Kb | 32031 | 62 | 0 | 0 |
| `fuzz_opcua_service` | 37 | 4536374 | 5643 | 13391 | 2929/362Kb | 74366 | 62 | 0 | 0 |
| `fuzz_sensors_profile_toml` | 9 | 481557 | 4056 | 16390 | 2535/3394Kb | 7894 | 62 | 0 | 0 |

**Verdict: PASS.** Every target exited 0 after at least 60 s, executed inputs and left no crash artifact.
