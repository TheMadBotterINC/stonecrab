# Fuzz evidence run

| Item | Value |
| --- | --- |
| Date (UTC) | 2026-11-03T16:47:14Z |
| Tree | 850286f plus uncommitted changes |
| Machine | AMD Ryzen 9 9950X 16-Core Processor, Linux 7.2.6-76070206-generic x86_64 |
| Nightly | rustc 1.101.0-nightly (21b707e3f 2026-09-30) |
| cargo-fuzz | cargo-fuzz 0.13.2 |
| Command | `scripts/fuzz-evidence.sh 60` |
| Duration | 60 s per target, all targets concurrently, 62 s wall-clock after the build |

| Target | Seed inputs | Runs | Cov | Ft | Corpus | Exec/s | Seconds | Exit | Artifacts |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `fuzz_codec_bounded_reader` | 5 | 3077844 | 198 | 1090 | 263/4861b | 50456 | 62 | 0 | 0 |
| `fuzz_core_connection_string` | 5 | 2713663 | 447 | 1554 | 390/62Kb | 44486 | 62 | 0 | 0 |
| `fuzz_modbus_address` | 11 | 2086260 | 995 | 2897 | 616/15135b | 34200 | 62 | 0 | 0 |
| `fuzz_modbus_ascii` | 18 | 1318791 | 213 | 1241 | 222/112Kb | 21619 | 62 | 0 | 0 |
| `fuzz_modbus_connection_string` | 16 | 2315609 | 1583 | 3613 | 879/123Kb | 37960 | 62 | 0 | 0 |
| `fuzz_modbus_mbap` | 13 | 3017957 | 112 | 524 | 25/24Kb | 49474 | 62 | 0 | 0 |
| `fuzz_modbus_pdu` | 22 | 11562580 | 1201 | 1668 | 210/4225b | 189550 | 62 | 0 | 0 |
| `fuzz_modbus_rtu` | 11 | 1176527 | 227 | 1393 | 430/57Kb | 19287 | 62 | 0 | 0 |
| `fuzz_opcua_address` | 21 | 1295454 | 1385 | 4577 | 1231/245Kb | 21236 | 62 | 0 | 0 |
| `fuzz_opcua_chunk_reassembly` | 7 | 4388587 | 590 | 1549 | 428/65Kb | 71944 | 62 | 0 | 0 |
| `fuzz_opcua_connection_string` | 45 | 2483638 | 1279 | 2970 | 882/120Kb | 40715 | 62 | 0 | 0 |
| `fuzz_opcua_decode` | 50 | 2404529 | 4464 | 17477 | 2877/211Kb | 39418 | 62 | 0 | 0 |
| `fuzz_opcua_frame` | 27 | 1720875 | 515 | 2174 | 176/33Kb | 28211 | 62 | 0 | 0 |
| `fuzz_opcua_service` | 37 | 4018356 | 5630 | 13343 | 2925/322Kb | 65874 | 62 | 0 | 0 |
| `fuzz_opcua_symmetric` | 32 | 1736096 | 515 | 781 | 30/883b | 28460 | 62 | 0 | 0 |
| `fuzz_sensors_profile_toml` | 9 | 413222 | 3928 | 15915 | 2397/3320Kb | 6774 | 62 | 0 | 0 |

**Verdict: PASS.** Every target exited 0 after at least 60 s, executed inputs and left no crash artifact.
