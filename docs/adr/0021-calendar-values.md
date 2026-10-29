# 0021: Calendar values
## Status
Accepted (2026-10-28), as slice S53 of docs/POST_ALPHA_PLAN.md (finding G08, the calendar
part). It is the "later ADR" that ADR-0006 decision 6 and ADR-0002 defer the calendar
values to, and it answers the open question of ARCHITECTURE.md section 11 on
`PlcValue::Date/Time/DateTime`. Every change is additive to the published `stonecrab`
0.2.0. No driver produces a calendar value in this slice.

## Context
SPEC.md ("Values and measurements") asks the value model to preserve "the additional
structured/time types required by the baseline". IEC 61131-3 has nine calendar types,
and the pinned upstream (Apache PLC4X v1.0.0, `upstream/manifest.toml`
`[inventory.api]`, `value_type`) lists them in `PlcValueType` with their Java types:
`TIME` (0x51) and `LTIME` (0x52) as `java.time.Duration`, `DATE` (0x53) and `LDATE`
(0x54) as `LocalDate`, `TIME_OF_DAY` (0x55) and `LTIME_OF_DAY` (0x56) as `LocalTime`,
and `DATE_AND_TIME` (0x57), `DATE_AND_LTIME` (0x58) and `LDATE_AND_TIME` (0x59) as
`LocalDateTime`. The value classes in `plc4j/spi/values` (`PlcTIME` and the rest) build
`TIME` from milliseconds and `LTIME` from nanoseconds, take dates and date-times in UTC
arithmetic without keeping a zone, and render `String.valueOf` of the Java value. Some of
them are defective: `PlcLDATE` and `PlcLDATE_AND_TIME` divide "nanoseconds" by 1,000,000
to get seconds, and `PlcLDATE_AND_TIME.getPlcValueType` returns `DATE_AND_TIME`
(`docs/parity/modbus.md`, S44), so they are a mapping target, not a model to copy.

The protocols that carry calendar values on the wire are S7 (`S7-TYPE-TIME`), OPC UA
(`OPCUA-READ-TYPE-OVERRIDE`, `OPCUA-TYPE-DATETIME`), UMAS (`UMAS-TYPE-TOD`,
`UMAS-TYPE-DATE-AND-TIME`), ADS, BACnet and the PLC4X proxy (`PX-TYPE-TEMPORAL`), each with
its own widths and epochs. Modbus is not among them: S44 found that `modbus.mspec` has no
`DataItem` case for the eight names it lists, so upstream reads such a tag as OK with a
null value and throws on a write (`MB-TYPE-TIME`).

`PlcValue` already has `Duration(std::time::Duration)`, published in 0.2.0. It cannot be
negative, and IEC 61131-3 `TIME` can (`T#-5s`). The dependency policy keeps `chrono` and
`time` out of the tree (`deny.toml` bans `chrono`: "calendar types are not adopted in
core"; ALPHA_PLAN D23), the core has no allocation-free date type in `std`, and the core
denies `as` casts, `unwrap` and `expect`.

## Decision
1. **Four representations, not nine names.** The value model names representations, as it
   does for integers (Modbus `WORD` and `UINT` are both `U16`). Four structs, each `Copy`,
   `Eq`, `Ord` (chronological), `Hash`, `Debug` and `Display`, with private fields, are
   added to the crate root and wrapped by four `PlcValue` variants and four
   `PlcValueType` variants, appended after `Duration` in this order (CODING_STANDARDS
   9.10):

   | Variant and type | Representation | Range | Resolution | IEC 61131-3 and upstream |
   | --- | --- | --- | --- | --- |
   | `SignedDuration(PlcSignedDuration)` | `i64` count of nanoseconds | -9,223,372,036,854,775,808 ns to 9,223,372,036,854,775,807 ns, about 292 years either way | 1 ns | `TIME`, `LTIME`; `PlcTIME`, `PlcLTIME` (`Duration`) |
   | `Date(PlcDate)` | `i32` count of days since 1970-01-01, proleptic Gregorian | `-5877641-06-23` to `+5881580-07-11` | 1 day | `DATE`, `LDATE`; `PlcDATE`, `PlcLDATE` (`LocalDate`) |
   | `TimeOfDay(PlcTimeOfDay)` | hour, minute, second and nanosecond | `00:00` to `23:59:59.999999999` | 1 ns | `TIME_OF_DAY`, `LTIME_OF_DAY`; `PlcTIME_OF_DAY`, `PlcLTIME_OF_DAY` (`LocalTime`) |
   | `DateTime(PlcDateTime)` | a `PlcDate` and a `PlcTimeOfDay` | the date range, each day whole | 1 ns | `DATE_AND_TIME`, `DATE_AND_LTIME`, `LDATE_AND_TIME`; `PlcDATE_AND_TIME`, `PlcDATE_AND_LTIME`, `PlcLDATE_AND_TIME` (`LocalDateTime`) |

   Each range holds every value of the IEC 61131-3 types it represents: a 32-bit
   millisecond `TIME` and a 64-bit nanosecond `LTIME`, `LDATE` and `LDATE_AND_TIME`
   (64-bit nanoseconds since 1970, about 292 years either way) and the 16-bit day counts
   and BCD fields of the S7 and UMAS layouts. There is no leap second (no second 60) and
   no `24:00`, as in `java.time`. Which IEC type a value came from is the tag's business,
   not the value's: the driver reads the type from the address, as it does for `WORD` and
   `UINT`.
2. **No time zone, no dependency.** IEC 61131-3 values carry no zone or offset, and
   neither do the Stonecrab types: a device's local clock reading stays a local reading,
   and the core never converts it with the host's zone or calls it UTC. A `DateTime` is
   therefore not a `SystemTime`; a caller who knows the device's offset converts. The
   calendar arithmetic is H. Hinnant's public-domain `days_from_civil` and
   `civil_from_days` in `i64`, about 30 lines in `value.rs`; that is not enough to
   justify a dependency the policy bans, and a crate's own types would leak into the
   public API.
3. **Construction.** Constructors that cannot fail are `const fn`:
   `PlcSignedDuration::from_nanos(i64)`, `PlcDate::from_days_since_epoch(i32)` and
   `PlcDateTime::new(PlcDate, PlcTimeOfDay)`. Constructors that check a range return
   `Result` with `ErrorKind::InvalidConfig` ("a value incompatible with the requested
   type"): `PlcSignedDuration::from_millis(i64)` (beyond the `i64` nanosecond range),
   `PlcDate::from_ymd(i32, u8, u8)` (a month outside 1 to 12, a day the month does not
   have, a date outside the `i32` day range) and `PlcTimeOfDay::from_hms_nano(u8, u8, u8,
   u32)` and `from_nanos_since_midnight(u64)` (a field out of range, a count of a day or
   more). Accessors: `as_nanos`, `days_since_epoch`, `year`, `month`, `day`,
   `nanos_since_midnight`, `hour`, `minute`, `second`, `nanosecond`, `date`, `time`. The
   constructors are named rather than `TryFrom` impls (CODING_STANDARDS 9.7) because the
   unit is part of the meaning: `TryFrom<u64>` would not say whether it counts
   milliseconds or nanoseconds. Nothing is coerced: `as_bool`, `as_i64`, `as_u64` and
   `as_f64` return `None` for the four variants, as for `Duration`.
4. **Values outside the range.** A driver that decodes a wire value with no Stonecrab
   value (a day count beyond `i32`, a time of day at or after 24:00, a BCD month 13, an
   `LDATE` that is not midnight) reports an item error of kind `MalformedData` carrying
   the received bytes (the raw-bytes mechanism of ADR-0006 decision 5,
   `ProtocolStatus::with_raw`); it never clamps, wraps or substitutes a
   default. A driver that encodes a value into a narrower wire type (a millisecond `TIME`,
   a 16-bit day count) refuses a value that does not fit or would lose precision with
   `InvalidConfig` before sending anything; it never rounds or truncates. A Java value
   beyond the ranges above (a `Duration` beyond 292 years, a `LocalDate` beyond the `i32`
   day range) has no Stonecrab value, which the parity rows of a bridge must record.
5. **Display is `java.time`'s text.** Each type renders the ISO 8601 text that the
   matching `java.time` class's `toString` renders, so a Stonecrab value and the PLC4J
   value it maps to print the same: `PlcDate` as `LocalDate` (`YYYY-MM-DD`, a year beyond
   9999 with `+`, a year before 0000 with `-` and at least four digits); `PlcTimeOfDay` as
   `LocalTime` (`HH:MM`, then `:SS` when the seconds or the fraction are nonzero, then a
   fraction of 3, 6 or 9 digits when nonzero); `PlcDateTime` as `LocalDateTime` (date,
   `T`, time); `PlcSignedDuration` as `Duration` (`PT`, then hours `H`, minutes `M` and
   seconds `S` with the fraction's trailing zeros dropped, each nonzero part carrying the
   sign, so -90.5 s is `PT-1M-30.5S` and zero is `PT0S`). ISO 8601 defines no negative
   duration; this is the `java.time` extension. The text is for people and logs; it is not
   a parse format, and no `FromStr` is added. The expected strings in the tests were
   derived from the OpenJDK algorithms, not run against a JVM.
6. **`Duration` stays.** `PlcValue::Duration` keeps its meaning, a nonnegative elapsed
   time with no IEC 61131-3 counterpart, and no driver produces it today. A driver
   decoding `TIME` or `LTIME` produces `SignedDuration`, even for a nonnegative value, so
   one IEC type always has one representation.
7. **Modbus: nothing in this slice.** The eight `MB-TYPE-TIME` rows of
   `docs/parity/modbus.md` stay `deferred`, and the names stay rejected at parse time:
   upstream has no Modbus encoding for them (no `DataItem` case, no wire width, epoch or
   unit; read yields null, write throws), so they are an upstream placeholder that SPEC.md
   does not oblige Stonecrab to fill, and an invented encoding would have no upstream
   reference and no peer to read it back. A Modbus calendar encoding needs its own ADR
   with a device that defines one.
8. **Simulator.** The simulator's `STATE` store (`SimTag`) already holds any `PlcValue`,
   so a calendar value can be stored and served by `SimDevice` without change. A read
   through a sim connection is not possible in this slice: the `sim` address grammar has
   no spelling for the four types (`SIM-ADDRESS-GRAMMAR`), and adding one needs a rule for
   `RANDOM`, which cannot generate them. That is a separate change to the sim parity
   record.

## Consequences
- Drivers for S7, OPC UA, UMAS, ADS and BACnet have a target for every calendar type, and
  their parity rows can cite `CORE-VALUE-CALENDAR` for the representation and decision 4
  for out-of-range values.
- `PlcValue` grows by four variants and the crate root by four types; `PlcValueType`'s
  order is unchanged for the existing variants. The largest variant is still `String`.
- A consumer matching `PlcValue` already has a wildcard arm (the enum is
  `#[non_exhaustive]`); the sensor pipeline treats the new variants as non-numeric, like
  `Duration`, and refuses to scale them.
- No calendar value round-trips through text: a later need for parsing gets its own
  decision.
- An upstream bridge (the PLC4X proxy, `PX-TYPE-TEMPORAL`) maps nine type codes to four
  representations and back through the tag's declared type.

## Alternatives considered
- **Nine variants, one per IEC name.** Rejected: the long and short forms differ only in
  width and resolution, which is wire layout, and the value model already collapses
  same-representation names (`WORD` and `UINT`). Nine variants would also copy the
  upstream classes' defects into a one-to-one mapping.
- **`chrono`, `time` or `jiff`.** Rejected: the policy bans the first two and keeps
  calendar crates out of core, the arithmetic needed is small, and a third-party type in
  `PlcValue` would tie the public API to that crate's semver.
- **`std::time::SystemTime` for dates and date-times.** Rejected: it is an instant on the
  host's clock, implies UTC, has a platform-dependent range and cannot represent a date
  without a time or a local reading without inventing a zone.
- **Milliseconds for `SignedDuration` and `TimeOfDay`.** Rejected: `LTIME` and
  `LTIME_OF_DAY` carry nanoseconds, and `java.time` resolves nanoseconds.
- **A signed `Duration` by widening the existing variant.** Impossible without a breaking
  change: the payload is `std::time::Duration`.
- **Storing a time of day as a nanosecond count.** Rejected: the field accessors would
  need fallible narrowing on every call; fields keep them `const` and the count is a
  bounded sum.
- **A date restricted to years 0000 to 9999.** Rejected: an `i32` day count is the natural
  width, holds every `LDATE`, and leaves nothing to validate in `from_days_since_epoch`.

## Compliance
Unit and property tests in `crates/stonecrab/src/value.rs` check declaration order, the
range edges of each type (the first and last `i32` day, the last whole millisecond, the
last nanosecond before midnight), the refusals (month 0 and 13, February 29 of 1900 and
2023, second 60, hour 24, a whole day of nanoseconds), Gregorian leap years, the
`java.time` text of each type including both ends of each range, chronological order,
that no calendar variant is coerced to a number or a Boolean, and round trips: every `i32`
day count through its civil fields, every nanosecond of the day through its fields and
every millisecond count through `from_millis`. `tests/public_contract.rs` lists the four
types as `Send + Sync + 'static`. The parity row is `CORE-VALUE-CALENDAR` in
`docs/parity/core.md`; the Modbus rows `MB-TYPE-TIME` to `MB-TYPE-LDATE-AND-TIME` point
here. `scripts/coverage-gate.sh` holds the core at 100% lines. The two `#[expect]`
attributes on the civil-calendar functions (`arithmetic_side_effects`,
`integer_division`) cite this ADR: their `i64` operands are bounded by an `i32` input,
and every divisor is a positive constant.
