# Mutation evidence

One recorded `cargo mutants` run for the alpha, re-run on the single crate for slice S33
(ADR-0017 as amended by ADR-0018), 2026-10-27, on the S33 tree (the build commit `dd0dd73`
plus the S33 documentation changes; no library source changed). `outcomes.json` is
cargo-mutants' own `mutants.out/outcomes.json` for that run; the per-mutant logs and diffs
are not stored (they hold paths of the machine that ran them), and the script replaces the
absolute path of the cargo binary in the recorded command lines with `cargo` (the one
change to the file). The S32 run on the seven-crate tree is superseded by this one; its
history is in "How the run got here".

## Reproduce

```sh
MUTANTS_JOBS=3 mise run mutants     # scripts/mutants.sh; 4 jobs by default
```

That runs `cargo mutants --config mutants.toml --jobs N --output target/mutants-run` with
cargo-mutants 27.1.0 and copies `outcomes.json` here. The settings are in `mutants.toml`:
the scope below, `test_tool = "nextest"`, `--locked --all-features`, `--fail-fast` for the
test phase, tests of the mutated package only (`test_workspace = false`; there is one
package, so every in-scope module is tested by the whole crate's test binaries), a scratch
copy of the tree (never `--in-place`), a 60 s minimum test timeout. The script also sets
`CARGO_PROFILE_DEV_DEBUG=0` and `CARGO_PROFILE_TEST_DEBUG=0` for the scratch builds (disk
and time only). This run used 3 jobs on a 12-core machine that was also busy with other
work, and took 69 minutes (the baseline: 21 s build, 1.9 s test). The script exits 2
because two mutants survive (below). Doctests are not run by nextest, so a mutant only a
doctest could catch would show as a survivor; none did beyond the two equivalent ones.

## Scope

The core's `value.rs` and `uri.rs` (crate root); `sim` `address.rs` and `driver.rs` (the
configuration parser); `modbus` `address.rs`, `pdu.rs` and `pdu/*.rs`, `framing.rs` and
`framing/*.rs`, `decode.rs`, `batch.rs`, `driver.rs` (the configuration parser) and
`options.rs` (its parameters); `sensors` `measurement/scale.rs`. All paths are under
`crates/stonecrab/src/`. `pdu.rs` and `framing.rs` are module declarations and constants
and produce no mutants; the code is in their submodules. `sim/driver.rs` and
`modbus/options.rs` go beyond the files the plan names: the release checklist asks for the
two configuration parsers (simulator and Modbus) to be in the evidence.

Slice S83 (2026-10-30) added `opcua/address.rs` (the OPC UA tag address grammar) to
`mutants.toml`. `outcomes.json` is the S33 run and predates it. A run over that file alone
(the same configuration with the one glob, `--jobs 1`, and the tests limited to the two test
binaries that cover it, `opcua_address` and `opcua_properties`) tested 190 mutants in 77
minutes: 163 caught, 22 unviable, 0 timeouts and 5 missed. The five are equivalent: the
`|` of `base64_encode` (two) and `base64_decode` (three) joins two fields of disjoint bits,
so `^` gives the same byte; they are listed in `exclude_re`. That run is not stored here.
After the review of S83 changed `OpcuaAddress::parse` (the canonical text obeys the bound) and
`base64_decode` (the decoded bytes must encode back to the input), the mutants of those two
functions and `base64_encode` were run again in the same way: 42 mutants, 40 caught, 2
unviable, none missed or timed out, in 17 minutes. (`base64_decode` lost its padding check,
which the re-encoding made redundant: a first run of the old check showed it as a missed
`||` to `&&`.)

Slice S85 (2026-10-30) added `opcua/codec.rs` and `opcua/codec/*.rs` (the built-in types of the
OPC UA binary encoding) to `mutants.toml`; `outcomes.json` still predates it. A run over those
files alone (the same configuration with the two globs, `--jobs 1`, `CARGO_PROFILE_*_DEBUG=0`
and the whole crate's tests) listed 111 mutants and tested them in 45 minutes: 92 caught, 10
unviable, 0 timeouts and 9 missed. Nine survivors: the `*` of `16 * 1024 * 1024` in
`MAX_STRING_BYTES` (two; the tests used the constant itself, and `opcua_codec_string_maximum_is_16_mib`
now pins its value), the `|` of the encoding byte and the flags in `NodeId::write` (six) and
`<` in `DateTime::to_system_time` (one). The last seven are equivalent and are listed in
`exclude_re`: the encoding byte is at most 5 and the flags are bits 6 and 7, so `|` and `^`
agree (the `write` was then refactored to or the flags in at one place, so one mutant stands
for what were six); and a difference of 0 ticks gives the epoch whether a zero duration is
added or subtracted. After those changes the three files that changed (`codec.rs`,
`node_id.rs`, `date_time.rs`) were run again: 48 mutants, 40 caught, 8 unviable, none missed
or timed out, in 19 minutes. Together with the first run's results for the other four
files, the module had 101 mutants listed (two equivalent ones excluded): 91 caught, 10 unviable,
0 missed and 0 timeouts. The review of S85 then changed `string.rs` (a String keeps its bytes
and no longer refuses bytes that are not UTF-8), and that file alone was run again: 40
mutants, 38 caught, 2 unviable, none missed or timed out, in 17 minutes. With that change the
module lists 106 mutants (`cargo mutants --config mutants.toml --list`), with no survivor in
any file. Neither run is stored here.

## Final run

574 mutants run (577 listed; three equivalent mutants are excluded by `exclude_re`):
**465 caught, 2 missed, 0 timeouts, 107 unviable** (they do not compile). Kill rate =
caught / (caught + missed + timeouts) = 465 / 467 = 99.6%. The two missed mutants are
equivalent (below), so the rate over mutants that can differ from the original is
465 / 465 = 100%. Counting the three excluded equivalent mutants as survivors as well gives
465 / 470 = 98.9%. Unviable mutants are 18.6% of the run and are excluded from the rate, as
cargo-mutants does.

| File | Mutants | Caught | Missed | Excluded | Timeout | Unviable | Kill rate |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `uri.rs` (core) | 35 | 33 | 0 | 0 | 0 | 2 | 100.0% |
| `value.rs` (core) | 15 | 14 | 0 | 0 | 0 | 1 | 100.0% |
| `sim/address.rs` | 23 | 18 | 0 | 0 | 0 | 5 | 100.0% |
| `sim/driver.rs` | 38 | 27 | 2 | 0 | 0 | 9 | 93.1% |
| `modbus/address.rs` | 119 | 87 | 0 | 0 | 0 | 32 | 100.0% |
| `modbus/batch.rs` | 31 | 23 | 0 | 0 | 0 | 8 | 100.0% |
| `modbus/decode.rs` | 36 | 29 | 0 | 0 | 0 | 7 | 100.0% |
| `modbus/driver.rs` | 36 | 27 | 0 | 0 | 0 | 9 | 100.0% |
| `modbus/framing/mbap.rs` | 44 | 35 | 0 | 0 | 0 | 9 | 100.0% |
| `modbus/options.rs` | 20 | 14 | 0 | 0 | 0 | 6 | 100.0% |
| `modbus/pdu/code.rs` | 12 | 10 | 0 | 0 | 0 | 2 | 100.0% |
| `modbus/pdu/codec.rs` | 41 | 38 | 0 | 0 | 0 | 3 | 100.0% |
| `modbus/pdu/model.rs` | 23 | 21 | 0 | 0 | 0 | 2 | 100.0% |
| `sensors/measurement/scale.rs` | 101 | 89 | 0 | 3 | 0 | 12 | 100.0% |
| Total | 574 | 465 | 2 | 3 | 0 | 107 | 99.6% |

"Mutants" counts the mutants that ran; the three excluded `scale.rs` mutants come on top
(104 listed in that file, 577 in all). The kill rate of `sim/driver.rs` is 27 / 29.

## How the run got here

| Run | Tree | Settings | Result |
| --- | --- | --- | --- |
| 1 (S32) | seven crates | `mutants.toml` without exclusions, nextest's default of running every test after a failure | 572 mutants in 32 min: 426 caught, 7 missed, 32 timeouts, 107 unviable |
| 2 (S32) | seven crates | after the tests and exclusions below | 569 mutants in 30 min: 430 caught, 0 missed, 32 timeouts, 107 unviable |
| 3 (S32) | seven crates | adds `--fail-fast` to the test phase | 569 mutants in 23 min: 462 caught, 0 missed, 0 timeouts, 107 unviable |
| 4 (S33, stored) | one crate | the same configuration; the paths and the package changed | 574 mutants in 69 min: 465 caught, 2 missed, 0 timeouts, 107 unviable |

Only run 4's `outcomes.json` is stored. The per-file counts of run 4 equal those of run 3
for every file except the two `driver.rs` files, so the 569 mutants of run 3 are all still
present and all still caught. Five mutants are new: with `Capabilities` and `Timeouts` now
in the same crate as the drivers, cargo-mutants can see that they implement `Default` and
offers "delete field" mutants for the struct expressions that end in `..Default::default()`
(or `..Capabilities::default()`): `read`, `write`, `subscribe` and `concurrency` in
`SimDriver::capabilities` and `request` in `ModbusTcpDriver::parse_config`. Three are
caught (`read`, `write`, `request`); two survive.

The 32 timeouts of runs 1 and 2 were mutants in the Modbus framer, PDU codec and batcher
that failed assertions within milliseconds but whose run did not end, because the nextest
profile keeps running after a failure (`fail-fast = false`) and a test that waits for a
reply on a mutated framer never finishes. With `--fail-fast` the first failure ends the
run, and every one of the 32 is caught by a failing test. The nextest profile itself is
unchanged.

## Survivors of run 4 and what was done

Two survive, and both are equivalent mutants, so no test can kill them. After the run the
docs commit removed the two redundant explicit fields they target from
`SimDriver::capabilities` (the defaults are those values; behaviour unchanged), so a later
run has nothing to report there:

| Mutant | Why it is equivalent |
| --- | --- |
| `sim/driver.rs:292`: delete field `subscribe` from the `Capabilities` expression in `SimDriver::capabilities` | The expression is `Capabilities { read: true, write: true, subscribe: SubscriptionMode::Unsupported, concurrency: NonZeroUsize::MIN, ..Capabilities::default() }`, and `Capabilities::default()` sets `subscribe` to `SubscriptionMode::Unsupported`, the value given. Deleting the field yields the same `Capabilities` for every input. |
| `sim/driver.rs:293`: delete field `concurrency` from the same expression | `Capabilities::default()` sets `concurrency` to `NonZeroUsize::MIN`, the value given. |

These cannot be listed in `exclude_re` of `mutants.toml`: cargo-mutants 27.1.0 does not
apply `exclude_re` or `examine_re` to the struct-field genre (`cargo mutants --list
--examine-re <text that matches nothing>` still prints the five). They are therefore
recorded here, in `docs/evidence/alpha.md` and in `STATUS.md`, and will be reported as missed by every
run until cargo-mutants filters them or the source stops spelling the defaults out. The
other three new mutants are caught by existing tests (`read` and `write` default to
`false`).

The three mutants excluded by `exclude_re`, each an equivalent mutant (no input can tell
it from the original), are unchanged from S32:

| Mutant | Why it is equivalent |
| --- | --- |
| `scale.rs` `own_type`: match arm `PlcValue::I64(_)` deleted | `Num::into_value` falls back to `PlcValue::I64`, then `U64`, for any integer the per-type rebuilder rejects. For an `I64` input the rebuilder does the same `i64::try_from`, so the wildcard rebuilder `\|_\| None` gives the same value for every input. |
| `scale.rs` `integral`: `mantissa \| (1 << 52)` changed to `^` | The mantissa is the low 52 bits of the float's bit pattern, so bit 52 is always clear; `\|` and `^` of a clear bit with the same constant are equal. |
| `scale.rs` `bits_of`: `value < 0` changed to `value <= 0` | For 0 the negative branch computes `i64::try_from(0)` and sign-extends it to the pattern 0, which is what the other branch gives. |

The seven survivors of the S32 first run were handled in S32: four were killed by tests
added then (named below, at their current paths) and three are the exclusions above.

| Mutant | Test |
| --- | --- |
| `modbus/address.rs` `locate`: match guard `prefix.is_ascii_digit()` replaced with `true` | `short_forms_require_four_or_five_digits` in `crates/stonecrab/tests/modbus_parse.rs` gained `ax00016` and `xx00016`, which must be refused as "unrecognised address" |
| `modbus/options.rs` `digits_overflow`: `&&` replaced with `\|\|` | `text_that_is_not_digits_names_the_digits_rule_and_a_huge_number_the_bounds` in `crates/stonecrab/tests/modbus_config.rs`: non-digit text names the digits rule, an all-digit value too large for `u64` names the bounds |
| `sensors/measurement/scale.rs` `Num::to_f64`: `value < 0` replaced with `value <= 0` | `an_integer_zero_divided_is_positive_zero` in `scale.rs`: `0 / 10` must be `+0.0`, not `-0.0` |
| `scale.rs` `own_type`: match arm `PlcValue::I32(_)` deleted | `a_thirty_two_bit_integer_keeps_its_own_type_through_integral_steps` in `scale.rs` |

No library code was changed for either mutation run; the only source edits made for the
evidence are the tests above (the `scale.rs` in-file test module and two integration-test
files).
