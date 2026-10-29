# Benchmark evidence

Slice S52 (ADR-0009 addendum "S52"). One local run of the Criterion benches of `stonecrab`,
recorded on 2026-10-28, and the way the scheduled job uses them. The numbers are in
`baseline.json`; the table below is the median of each benchmark.

**A laptop baseline does not transfer to CI runners.** The figures below come from one
Apple M2 Max laptop with other processes running. A GitHub runner has a different CPU, a
different clock, noisy neighbours and, for the loopback benchmarks, a different network stack.
Nothing reads this baseline: the scheduled gate never compares a CI run with these numbers.
It compares two builds on the same runner in the same job (see "The gate"). Use these figures
for orders of magnitude and as the first local reference.

## What was run

| Item | Value |
| --- | --- |
| Tree | commit `48b65e3` (master) plus the uncommitted S52 changes: the benches, the `criterion` dev-dependency and the scripts. No library source changed. |
| Machine | Apple M2 Max, 12 cores, 96 GiB RAM |
| OS | macOS 26.6.2, arm64 |
| Toolchain | rustc 1.99.1 (58cf8f9c3 2026-10-13), the pin of `rust-toolchain.toml` |
| Profile | `bench`: inherits `release` (thin LTO, one codegen unit, `overflow-checks = true`); `CARGO_INCREMENTAL=0` from `mise.local.toml` |
| Features | `--all-features` |
| Harness | Criterion 0.8.2, default settings: 3 s warm-up, 5 s measurement, 100 samples, 95% confidence |
| Command | `mise run bench` (all 22 benchmarks, about 4 minutes) |
| Workload | Fixed inputs built once; the loopback benchmarks read from a `ScriptedPeer` on `127.0.0.1` in the same process. No device, no external network, no real-time sleep. |

## Results

Median per iteration in nanoseconds, its 95% confidence interval, and the standard deviation
of the sample. `baseline.json` also holds the mean and its interval.

| Benchmark | Median (ns) | 95% interval (ns) | Std dev (ns) |
| --- | ---: | --- | ---: |
| `loopback_read/full_response_125_tags_1_request` | 51950.11 | 51248.86 to 52464.6 | 13047.08 |
| `loopback_read/one_register_1_request` | 20667.81 | 20619.01 to 20746.69 | 3296.36 |
| `loopback_read/plant_24_tags_2_requests` | 45275.7 | 45207.69 to 45375.6 | 2266.77 |
| `mbap/decode_max` | 32.91 | 31.64 to 33.57 | 3.83 |
| `mbap/decode_small` | 21.2 | 21.15 to 21.31 | 0.47 |
| `mbap/encode_max` | 17.58 | 17.45 to 17.62 | 0.28 |
| `mbap/encode_small` | 16.52 | 16.49 to 16.55 | 0.25 |
| `parse/address_bit_area` | 28.18 | 28.09 to 28.32 | 4.82 |
| `parse/address_long` | 36.07 | 35.92 to 36.24 | 70.12 |
| `parse/address_with_block` | 124.81 | 124.18 to 126.39 | 2.09 |
| `parse/connection_string_all_parameters` | 910.5 | 909.92 to 911.06 | 8.35 |
| `parse/connection_string_host_only` | 213.76 | 212.98 to 214.22 | 9.64 |
| `pdu/decode_request` | 4.27 | 4.26 to 4.27 | 0.05 |
| `pdu/decode_response_max` | 24.8 | 24.71 to 24.94 | 12.41 |
| `pdu/decode_response_small` | 18.98 | 18.89 to 19.07 | 5.47 |
| `pdu/encode_request` | 35 | 34.92 to 35.07 | 0.46 |
| `pdu/encode_response_max` | 82.86 | 82.73 to 83.02 | 0.87 |
| `pdu/encode_response_small` | 39.21 | 39.08 to 39.34 | 64.75 |
| `plan/adjacent_1024_tags` | 13551.42 | 13527.05 to 13582.4 | 563.81 |
| `plan/plant_28_tags` | 835.59 | 834.07 to 836.84 | 13.35 |
| `typed_decode/real_batch_of_62` | 2124.69 | 2114.05 to 2143.19 | 1022.83 |
| `typed_decode/uint_batch_of_125` | 3875.2 | 3850.93 to 3907.49 | 77.62 |

What they measure:
- `parse/*`: `ModbusAddress::parse` (a bare long form, a coil, a form with the `{unit-id,
  byte-order}` block) and `ModbusTcpDriver::parse_config` (a bare host, and a string with
  every parameter).
- `pdu/*`: `encode_request` and `decode_request` for a 125-register read request, and
  `encode_response` and `decode_response` for a 2-register and a 125-register (maximum)
  response. The `encode_*` figures include a fresh writer allocation and the copy-out
  (`into_bytes`) on every iteration; the `mbap/encode_*` figures do not allocate.
- `mbap/*`: `MbapFramer` encode (into a reused buffer) and decode (the buffer clone is not
  timed, nor is its drop) of the same two frames.
- `typed_decode/*`: `TypedDecoder::decode_registers` over 125 registers as 62 `REAL`s and as
  125 `UINT`s, big-endian, high word first.
- `plan/*`: `Batcher::plan` for a 28-tag plant set (floats, integers, far registers, coils and
  a second unit) and for 1024 adjacent registers.
- `loopback_read/*`: `Connection::read` through the registry and the real TCP transport for
  1 tag, for 24 tags in 2 requests and for 125 tags in 1 request (the largest response). They
  include request handling, the plan, the framed exchange per chunk on a current-thread
  Tokio runtime shared with the peer, and the response assembly (the request clone is built
  outside the timing); they are library overhead plus loopback, not device or network
  latency.

## Reading the numbers honestly

- The loopback figures depend on the operating system's socket path and the scheduler. In
  the trial runs of the gate their per-round ratios swung by far more than the code
  explains (a single round near 3 times the base), which is why the gate needs every round
  to agree.
- The mean of a few benchmarks (`parse/address_long`, `pdu/encode_response_small`,
  `pdu/decode_response_*`) is pulled up by outliers; the median and its interval are the
  statistics the gate uses.
- Allocation counts, which SPEC.md asks for, are not measured (a counting allocator is
  `unsafe`); multi-device polling, subscription tail latency, soak figures and binary size
  need features or rigs that do not exist yet.

## Reproduce

```sh
mise run bench                      # all benches; arguments after -- go to Criterion
mise run bench -- parse/            # a filter
```

Criterion keeps its raw files under `target/criterion/`. `baseline.json` was written from the
`new/estimates.json` and `new/benchmark.json` of each benchmark there with `jq`; the keys are
`median_ns` and `mean_ns` (estimate and 95% interval) and `std_dev_ns`.

## The gate

The weekly `scheduled` job of `.github/workflows/ci.yml` runs `scripts/bench-compare.sh` with
the newest commit that is at least seven days old as `BASE_REF` (`mise run bench-compare --
BASE_REF` does the same locally). It builds `BASE_REF` and the working tree, runs both on the
same machine for three rounds in alternating order, and fails when a benchmark's 95% median
interval at the head sits more than 10% above the top of the base's interval in every round
(a regression of more than 10% beyond the noise band; a smaller or noisier one is not
caught). Pull requests and pushes never run it. The method, the verdicts and the limits are in the ADR-0009 addendum
"S52". The report of a run goes to the job summary and to the `bench-compare` artifact.
