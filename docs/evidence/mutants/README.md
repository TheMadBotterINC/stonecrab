# Mutation evidence

One recorded `cargo mutants` run for the alpha (slice S32, ADR-0017), 2026-10-27, on the
tree of the S32 commit. `outcomes.json` is cargo-mutants' own `mutants.out/outcomes.json`
for that run; the per-mutant logs and diffs are not stored (they hold paths of the machine
that ran them), and the script replaces the absolute path of the cargo binary in the
recorded command lines with `cargo` (the one change to the file).

## Reproduce

```sh
mise run mutants          # scripts/mutants.sh 4 jobs; MUTANTS_JOBS or "-- 3" changes it
```

That runs `cargo mutants --config mutants.toml --jobs N --output target/mutants-run` with
cargo-mutants 27.1.0 and copies `outcomes.json` here. The settings are in `mutants.toml`:
the scope below, `test_tool = "nextest"`, `--locked --all-features`, `--fail-fast` for the
test phase, tests of the mutated package only (`test_workspace = false`), a scratch copy
of the tree (never `--in-place`), a 60 s minimum test timeout. The script also sets
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` for the scratch builds (disk
and time only). This run used 3 jobs on a 12-core machine and took 23 minutes (the
baseline: 16 s build, 1.5 s test). Doctests are not run by nextest, so a mutant only a
doctest could catch would show as a survivor; none did.

## Scope

Core `value.rs` and `uri.rs`; driver-sim `address.rs` and `driver.rs` (the configuration
parser); modbus `address.rs`, `pdu.rs` and `pdu/*.rs`, `framing.rs` and `framing/*.rs`,
`decode.rs`, `batch.rs`, `driver.rs` (the configuration parser) and `options.rs` (its
parameters); sensors `measurement/scale.rs`. `pdu.rs` and `framing.rs` are module
declarations and constants and produce no mutants; the code is in their submodules.
`driver-sim/driver.rs` and `modbus/options.rs` go beyond the files the plan names: the
release checklist asks for the two configuration parsers (simulator and Modbus) to be in
the evidence.

## Final run

569 mutants generated after the three exclusions below: **462 caught, 0 missed, 0
timeouts, 107 unviable** (they do not compile). Kill rate = caught / (caught + missed +
timeouts) = 462 / 462 = 100%. Counting the three excluded equivalent mutants as
survivors gives 462 / 465 = 99.4%. Unviable mutants are 18.8% of the generated ones and
are excluded from the rate, as cargo-mutants does.

| File | Mutants | Caught | Missed | Excluded | Timeout | Unviable | Kill rate |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `stonecrab-core/src/uri.rs` | 35 | 33 | 0 | 0 | 0 | 2 | 100.0% |
| `stonecrab-core/src/value.rs` | 15 | 14 | 0 | 0 | 0 | 1 | 100.0% |
| `stonecrab-driver-sim/src/address.rs` | 23 | 18 | 0 | 0 | 0 | 5 | 100.0% |
| `stonecrab-driver-sim/src/driver.rs` | 34 | 25 | 0 | 0 | 0 | 9 | 100.0% |
| `stonecrab-modbus/src/address.rs` | 119 | 87 | 0 | 0 | 0 | 32 | 100.0% |
| `stonecrab-modbus/src/batch.rs` | 31 | 23 | 0 | 0 | 0 | 8 | 100.0% |
| `stonecrab-modbus/src/decode.rs` | 36 | 29 | 0 | 0 | 0 | 7 | 100.0% |
| `stonecrab-modbus/src/driver.rs` | 35 | 26 | 0 | 0 | 0 | 9 | 100.0% |
| `stonecrab-modbus/src/framing/mbap.rs` | 44 | 35 | 0 | 0 | 0 | 9 | 100.0% |
| `stonecrab-modbus/src/options.rs` | 20 | 14 | 0 | 0 | 0 | 6 | 100.0% |
| `stonecrab-modbus/src/pdu/code.rs` | 12 | 10 | 0 | 0 | 0 | 2 | 100.0% |
| `stonecrab-modbus/src/pdu/codec.rs` | 41 | 38 | 0 | 0 | 0 | 3 | 100.0% |
| `stonecrab-modbus/src/pdu/model.rs` | 23 | 21 | 0 | 0 | 0 | 2 | 100.0% |
| `stonecrab-sensors/src/measurement/scale.rs` | 104 | 89 | 0 | 3 | 0 | 12 | 100.0% |
| Total | 572 | 462 | 0 | 3 | 0 | 107 | 100.0% |

"Mutants" includes the excluded ones (572 listed, 569 run).

## How the run got here

| Run | Settings | Result |
| --- | --- | --- |
| 1 | `mutants.toml` without exclusions, nextest's default of running every test after a failure | 572 mutants in 32 min: 426 caught, 7 missed, 32 timeouts, 107 unviable |
| 2 | after the tests and exclusions below | 569 mutants in 30 min: 430 caught, 0 missed, 32 timeouts, 107 unviable |
| 3 (stored) | adds `--fail-fast` to the test phase | 569 mutants in 23 min: 462 caught, 0 missed, 0 timeouts, 107 unviable |

Only run 3's `outcomes.json` is stored. The 32 timeouts of runs 1 and 2 were mutants in
the Modbus framer, PDU codec and batcher that failed assertions within milliseconds but
whose run did not end, because the nextest profile keeps running after a failure
(`fail-fast = false`) and a test that waits for a reply on a mutated framer never
finishes. With `--fail-fast` the first failure ends the run, and every one of the 32 is
caught by a failing test. The nextest profile itself is unchanged.

## Survivors of run 1 and what was done

Killed by a test added in S32 (no library code changed):

| Mutant | Test |
| --- | --- |
| `modbus/src/address.rs` `locate`: match guard `prefix.is_ascii_digit()` replaced with `true` | `short_forms_require_four_or_five_digits` in `crates/stonecrab-modbus/tests/parse.rs` gained `ax00016` and `xx00016`, which must be refused as "unrecognised address" |
| `modbus/src/options.rs` `digits_overflow`: `&&` replaced with `\|\|` | new `text_that_is_not_digits_names_the_digits_rule_and_a_huge_number_the_bounds` in `crates/stonecrab-modbus/tests/config.rs`: non-digit text names the digits rule, an all-digit value too large for `u64` names the bounds |
| `sensors/src/measurement/scale.rs` `Num::to_f64`: `value < 0` replaced with `value <= 0` | new `an_integer_zero_divided_is_positive_zero` in `scale.rs`: `0 / 10` must be `+0.0`, not `-0.0` |
| `scale.rs` `own_type`: match arm `PlcValue::I32(_)` deleted | new `a_thirty_two_bit_integer_keeps_its_own_type_through_integral_steps` in `scale.rs` |

Listed in `exclude_re` of `mutants.toml`, each an equivalent mutant (no input can tell it
from the original):

| Mutant | Why it is equivalent |
| --- | --- |
| `scale.rs` `own_type`: match arm `PlcValue::I64(_)` deleted | `Num::into_value` falls back to `PlcValue::I64`, then `U64`, for any integer the per-type rebuilder rejects. For an `I64` input the rebuilder does the same `i64::try_from`, so the wildcard rebuilder `\|_\| None` gives the same value for every input. |
| `scale.rs` `integral`: `mantissa \| (1 << 52)` changed to `^` | The mantissa is the low 52 bits of the float's bit pattern, so bit 52 is always clear; `\|` and `^` of a clear bit with the same constant are equal. |
| `scale.rs` `bits_of`: `value < 0` changed to `value <= 0` | For 0 the negative branch computes `i64::try_from(0)` and sign-extends it to the pattern 0, which is what the other branch gives. |

No library code was changed for the mutation run; the only source edits are the tests
above, which is the one `scale.rs` in-file test module and two integration-test files.
