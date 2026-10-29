# 0016: Sensor profile schema and TOML loader
## Status
Amended by ADR-0018: `stonecrab-sensors` is now the module `stonecrab::sensors`, behind
the `sensors` feature (on by default), which includes the TOML loader; the separate
`toml` feature is gone. The decisions stand.
Accepted by the sole maintainer on 2026-10-27, as slice S28a of docs/ALPHA_PLAN.md.
It adds the new crate `stonecrab-sensors` (CODING_STANDARDS 12.6 requires an ADR for a
new crate), adopts `serde` and `toml` (section 15.1), and implements decisions D10, D17
and D23 for this crate. It builds on ADR-0005 (the sensors coverage row), ADR-0009 (the
fuzz workspace) and ADR-0013 (how a dependency is adopted). Semantic validation, the tag
builder and measurement pipeline, and the synthetic profiles with the facade `sensors`
feature are not part of this original decision; they are the addenda below (S28b, S29,
S30), and the whole record is accepted.

## Context
SPEC.md "Sensor profile contract" says profiles are versioned data, initially TOML, that
map protocol tags to measurement names without embedding executable code. Each profile
declares an id and schema version, applicability, connection requirements and address
convention, data type, count and byte and word order, scaling, units and range, status
fields and quality mapping, acquisition and stale intervals, and its documentary source,
fixtures and validation status. The same document says never to infer word order and to
reject the ambiguous Modbus `40001` versus zero-based reading unless the profile selects a
convention. The mandatory failure case F7 names an incompatible profile version.

A profile file is application configuration that may be edited by hand or supplied by a
third party, so the loader is a parser of untrusted text (CODING_STANDARDS 21.1).

## Decision
1. **Crate boundary.** `stonecrab-sensors` depends on `stonecrab-core`, `serde` and the
   optional `toml`. It depends on no driver and no transport: a profile carries address
   text and a driver type name as strings, and a later slice turns them into tags through
   `Driver::parse_address`. Core supplies `TagId`, `ByteOrder`, `WordOrder` and `PlcError`.
   `proptest` is a dev-dependency.
2. **D17: two profile files per synthetic device, one per scheme.** A profile declares one
   `[connection] scheme`; its addresses belong to that scheme's grammar. A device
   reachable through `modbus-tcp` and through `sim` has two files under
   `profiles/synthetic/{modbus-tcp,sim}/` (slice S29). `address_convention` is an
   `Option<String>` in the schema and is mandatory when `scheme = "modbus-tcp"`; that rule
   is the S28b validation (`ProfileError::MissingAddressConvention`), not a schema rule,
   so a document that omits it loads here. Multi-scheme address maps are M2 with named
   device profiles. No named device profile exists: every profile that ships is labeled
   `synthetic`.
3. **D10: `toml` is default-on.** `[features] default = ["toml"]`, `toml = ["dep:toml"]`.
   This follows ARCHITECTURE.md section 10 and is the recorded deviation from the
   off-by-default rule (with ADR-0014): a profile crate with no loader is not a usable
   default. `SensorProfile::from_toml` and everything that touches `toml` sits behind the
   feature; the schema types, which only derive `Deserialize`, do not. The `features`
   job's per-crate `--no-default-features` check proves the crate builds and exposes every
   type without the loader (also checked here with `cargo check`, `clippy` and `cargo
   test` on `--no-default-features`). The README states that `toml` must be enabled to
   load files.
4. **The schema, version 1.** Every type derives `Deserialize` with
   `deny_unknown_fields` and is `#[non_exhaustive]` with public fields (profiles come from
   deserialization, never from a struct literal outside the crate). `Serialize` is not
   derived: the library never writes a profile, and an `Serialize` impl would be an
   unrequested format commitment.
   - `ProfileId` and the measurement `name` are snake_case ASCII: 1 to 64 bytes, a
     lowercase letter first, then lowercase letters, digits or underscores. This is
     narrower than `TagId` so that ids are safe in file names and metric labels. A
     measurement name is stored as a `TagId`, so it doubles as the tag identity with no
     conversion. `ProfileId::is_valid` exposes the rule.
   - `SchemaVersion` accepts 1 and refuses everything else with
     `ProfileError::IncompatibleSchema { found }`.
   - `SensorProfile { id, schema_version, device, connection, measurements, provenance,
     validation }`. Required: `id`, `schema_version`, `connection`, `provenance`,
     `validation`. A profile cannot omit its validation status, so it cannot silently
     claim more evidence than it has. `device` and `measurements` default to empty; an
     empty measurement list is S28b's to judge.
   - `DeviceApplicability { manufacturer, model, firmware }` (all optional),
     `ConnectionRequirements { scheme, address_convention, options }` (`options` is a
     `BTreeMap<String, String>` keyed like the connection-string query),
     `Provenance { source, fixtures }` and `ValidationStatus` (`synthetic`, `documented`,
     `fixture_backed`, `hardware_verified`; snake_case rather than the plan's "lowercase"
     because `fixturebacked` is unreadable).
   - `MeasurementDef { name, address, data_type, count, byte_order, word_order, scaling,
     unit, range, status_fields, quality_map, min_interval, stale_after, keep_raw }`.
     `address` and `data_type` are strings the driver interprets; the loader passes them
     through. `count` defaults to 1. `byte_order` and `word_order` are `Option`s that stay
     `None` when absent; they are never inferred. `ByteOrderName` (`big_endian`,
     `little_endian`) and `WordOrderName` (`high_word_first`, `low_word_first`) mirror the
     core enums by name and convert with `From`. `keep_raw` defaults to false and is
     documented as the unscaled typed value, not wire bytes.
   - `ValueRange { min, max }` is the plan's `Range`; the name avoids confusion with
     `std::ops::Range`.
   - `StatusField { name, address, bit, invert }` and `QualityMapping { bad_when,
     uncertain_when }`. The plan's `StatusField` has no name, but `QualityMapping` refers to
     status fields by name, so `name` is added to `StatusField` (snake_case, required).
5. **TOML forms.**
   - `ScaleStep` is externally tagged with snake_case variant names, so a step is an
     inline table with one key: `{ divide = 10.0 }`, `{ offset = -40.0 }`,
     `{ multiply = 2 }`, `{ clamp = { min = -40.0, max = 215.0 } }`, `{ bit_field = { shift
     = 4, width = 2 } }`. An integer constant is accepted for an `f64`. Internal tagging
     (`{ op = "divide", value = 10.0 }`) was rejected as longer and as unable to carry a
     newtype variant. Unknown keys inside `clamp` and `bit_field` are rejected.
   - Durations are whole milliseconds under the keys `min_interval_ms` and
     `stale_after_ms`; the Rust fields are `min_interval` and `stale_after`
     (`Option<Duration>`). A `humantime` string (`"1s"`) was rejected: it adds a
     dependency and a second grammar to fuzz for a value that is always a small integer.
     The unit is in the key, so a bare number cannot be misread.
   - Non-finite constants (`inf`, `nan`) are valid TOML and load; rejecting them is S28b.
6. **`SensorProfile::from_toml(&str)`.** Checks the size first (`MAX_TOML_BYTES` = 65 536;
   `ProfileError::Oversized { len, max }`, nothing parsed). It then parses only
   `schema_version` through a private probe that ignores other keys, so a document written
   for version 2 is reported as `IncompatibleSchema` even if it carries keys version 1 does
   not know, and only then parses the whole document with unknown keys denied. The
   parser's nesting limit stays on (`toml`'s `unbounded` feature is off) and a 16 Ki-deep
   array, inline-table and dotted-key document is tested. The function does no I/O.
7. **`ProfileError`.** `#[non_exhaustive]`, `Copy`, with `Oversized { len, max }`,
   `Syntax { line, column }`, `IncompatibleSchema { found }` and `InvalidIdentifier {
   field }`; `Display`, `std::error::Error` (written by hand, no `thiserror` dependency, as
   in ADR-0013) and `From<ProfileError> for PlcError` mapping every variant to
   `ErrorKind::InvalidConfig` with the error kept as `source()`. Errors carry sizes,
   1-based positions (columns counted in characters) and a fixed field label, never any
   part of the document: every `toml` error is reduced to the position of its span, and a
   test puts a canary string in five different invalid documents and checks that it never
   reaches `Display`, `Debug` or the `PlcError`. The cost is that an unknown key and a
   wrong-typed value are both `Syntax`; the position says where, not why. The S28b
   validator adds the specific variants. A version that is present but not a `u32`
   (`"1"`, `-1`, `1.0`) is `Syntax`, not `IncompatibleSchema`.
8. **Lint attributes.** `lib.rs` has `forbid(unsafe_code)`, the `docsrs` `cfg_attr` and
   `deny(missing_docs)` only, which is all `scripts/lint-attrs.sh` permits for
   `stonecrab-sensors` (the row already existed; verified, not edited). `as_conversions`
   stays at the workspace `warn` level, which CI's `-D warnings` denies. The code uses no
   `as` cast, indexing or unchecked arithmetic (`saturating_add` in the position helper).
9. **`serde` and `toml` adoption (section 15.1, 15.4, 15.5).** `[workspace.dependencies]`
   gains `serde = { version = "1.0", default-features = false, features = ["std"] }`
   (resolved 1.0.229; the minor is 0, so the caret range is `1.0`) and `toml = { version =
   "0.9", default-features = false, features = ["parse", "serde", "std"] }` (resolved
   0.9.12). The feature names were checked against the 0.9.12 manifest: `parse` enables the
   parser, `serde` enables `from_str` for derived types, `std` is the standard-library
   build; `display` is off because profiles are never serialized. `stonecrab-sensors`
   enables `serde/derive`. `docs/DEPENDENCY_AUDIT.md` has both entries with their
   transitive crates.
   - **Duplicate `winnow`.** The resolver first picked `toml_parser` 1.1.5, which needs
     `winnow` 1.0, next to `toml` 0.9.12's own `winnow` 0.7; `clippy::multiple_crate_versions`
     then fails `-D warnings`. `Cargo.lock` and `fuzz/Cargo.lock` pin `toml_parser` at 1.0.9
     (`cargo update -p toml_parser --precise 1.0.9+spec-1.1.0`), which `toml` 0.9.12
     accepts and which uses `winnow` 0.7. The pin is a lockfile choice, not a manifest
     one: a plain `cargo update` removes it and the lint fails again. If that becomes a
     chore, the alternatives are `toml` 1.x (single `winnow` 1.0; a plan change) or
     `allowed-duplicate-crates = ["winnow"]` in `clippy.toml`.
10. **D23 result.** Run on the resolved tree (cargo stops at the first unmatched package
    spec, so each name was also run alone, as in ADR-0013; each run also gave exit status
    101 with `--workspace --all-features -e normal,dev,build`):

    ```text
    $ cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono
    error: package ID specification `once_cell` did not match any packages
    $ cargo tree -e normal,dev -i once_cell
    error: package ID specification `once_cell` did not match any packages
    $ cargo tree -e normal,dev -i lazy_static
    error: package ID specification `lazy_static` did not match any packages
    $ cargo tree -e normal,dev -i log
    error: package ID specification `log` did not match any packages
    $ cargo tree -e normal,dev -i time
    error: package ID specification `time` did not match any packages
    $ cargo tree -e normal,dev -i chrono
    error: package ID specification `chrono` did not match any packages
    ```

    `toml_datetime` carries its own `Datetime` type and pulls neither `time` nor `chrono`.
    None of the five banned crates is in the graph, so no `deny.toml` `wrappers` entry is
    added (and `fuzz/deny.toml` needs no change for this crate). `cargo deny --locked check`
    is green.
11. **Fuzz target.** `fuzz_sensors_profile_toml` feeds arbitrary UTF-8 to `from_toml` and
    asserts: no panic; an oversized document is `Oversized`; every error converts to
    `InvalidConfig` with a short message; and a profile that loads has version 1, a
    snake_case id and snake_case measurement and status-field names (checked by an
    independent restatement of the rule, not by the code under test). Six seeds live in
    `fuzz/corpus/fuzz_sensors_profile_toml/`: a minimal and a full synthetic profile, a
    hostile scaling document, a version-2 document, an unknown-key document and an empty
    document.

## Consequences
- S28b adds `validate()` and the units table over these types without changing them;
  new `ProfileError` variants are additive because the enum is `#[non_exhaustive]`.
- Types with public fields and `#[non_exhaustive]` cannot be built by a struct literal
  outside the crate. Tests and applications obtain profiles from `from_toml`, or from any
  `serde` format of their own with `--no-default-features`.
- A hand-edited profile gets a position but not a reason for a schema error. Accepted for
  now, to keep the no-echo guarantee simple and testable.
- `toml` brings five crates (`toml`, `toml_parser`, `toml_datetime`, `serde_spanned`,
  `winnow`) plus `serde_core`; with `--no-default-features` only `serde` and its macro
  chain remain.
- One region of `measurement.rs` is reported uncovered: the `map_err` closure after
  `TagId::new` in `deserialize_tag_name`. It cannot run, because the identifier rule is
  stricter than `TagId`'s (64 bytes against 256, no controls), so a validated name always
  passes. The line is covered (it shares a line with the executed call), so the 100% line
  gate holds; no line is excluded.

## Alternatives considered
- **`serde_json` or RON profiles.** Rejected: JSON has no comments, which register maps
  need ("why this scale"), and RON is a niche format that operators do not know. The
  specification names TOML.
- **A hand-written TOML reader.** Rejected: it would be the parser the standards worry about
  (CODING_STANDARDS 21.1), reimplemented to avoid a dependency that is `forbid(unsafe_code)`
  at its top.
- **`humantime` durations.** Rejected above: a second grammar and a dependency for integers.
- **`toml_edit`.** Rejected: format-preserving editing is not needed by a read-only loader.
- **Internally tagged `ScaleStep`.** Rejected above.
- **Validating in `Deserialize`.** Rejected beyond identifiers: serde errors are one at a
  time, while S28b's `validate()` returns the full error list (`Vec<ProfileError>`).
- **`thiserror` for `ProfileError`.** Rejected: four variants, and the crate's dependency
  list stays what the plan names.

## Compliance
- Tests: `crates/stonecrab-sensors/tests/loader.rs` (the CODING_STANDARDS 13.7 list for the
  loader: empty document, one byte over the size bound, minimal profile, unknown field,
  wrong-typed field, measurement without a name, plus hostile nesting, no-echo, every type
  from the fixtures in `tests/profiles/`, identifier and name boundaries, millisecond
  durations, every `ScaleStep` form), `tests/properties.rs` (arbitrary strings never
  panic `from_toml`), `tests/public_types.rs` and the unit tests in
  `src/{error,ident,profile}.rs`. The F7 test is
  `schema_version_2_is_rejected_as_incompatible`, tagged `// covers: SPEC
  mandatory-failure-cases / F7 incompatible profile version`. Doctests on every type
  compile and run (the TOML ones are gated on the `toml` feature).
- `fuzz/Cargo.toml` gains the `[[bin]]` and the path dependency;
  `fuzz/fuzz_targets/fuzz_sensors_profile_toml.rs`; the corpus above.
- `scripts/coverage-gate.sh` gains the row `crates/stonecrab-sensors|100` (ADR-0005).
- `scripts/lint-attrs.sh` already permits `missing_docs` for `stonecrab-sensors`.
- `packaging/stonecrab-sensors.txt` and the `package` task and job pin the archive to
  `README.md`, `LICENSE`, `NOTICE` and `src/*`; `tests/` is not shipped.
- `Cargo.lock` gains `serde`, `serde_core`, `serde_derive`, `toml`, `toml_parser`,
  `toml_datetime`, `serde_spanned`, `winnow` and `stonecrab-sensors`, and the optional-only
  entries `indexmap`, `hashbrown` and `equivalent` that Cargo records for `toml`'s
  `preserve_order` feature (not built). `fuzz/Cargo.lock` gains the same.
- `docs/parity/sensors.md` holds the rows `SENSORS-SCHEMA-V1`, `SENSORS-VALIDATION`,
  `SENSORS-MEASUREMENTS` and `SENSORS-SYNTHETIC-PROFILES`.

## Addendum: validation and the unit table (slice S28b, 2026-10-27)
This addendum records the S28b decisions; it changes nothing above, and the S28a
consequence "S28b adds `validate()` and the units table over these types without
changing them" holds.

1. **Unit table policy.** `Unit::{is_known, description, all}` is a fixed, closed list of
   19 UCUM-style codes: `%`, `1`, `A`, `Cel`, `Hz`, `K`, `L`, `L/min`, `Pa`, `V`, `W`,
   `bar`, `kPa`, `kWh`, `m`, `m3/h`, `mm`, `ms`, `s`. Matching is exact and
   case-sensitive, `all()` is sorted in byte order, and every code has a description.
   There is no unit arithmetic: no conversion, no dimension analysis and no compound
   expression parsing, so `kPa` and `bar` are separate entries and a profile's scaling
   does any conversion. A unit outside the list is `UnknownUnit`, the empty string is
   `EmptyUnit`, and `1` is the dimensionless unit. Adding a unit is a minor-version
   change to this crate, not a profile change. The list is a macro-built pair of
   parallel arrays, so codes and descriptions come from one source.
2. **The width rule.** A data type is wider than 16 bits when its name is one of the
   Modbus driver's upstream names `DINT`, `UDINT`, `DWORD`, `LINT`, `ULINT`, `LWORD`,
   `REAL` or `LREAL`, in upper case; such a measurement needs both `byte_order` and
   `word_order` (`MissingByteOrder`, `MissingWordOrder`), consistent with "never infer".
   Every other `data_type` is passed through with no requirement. The rule is
   Modbus-name based and says so in the rustdoc: this crate does not depend on a driver,
   and another driver's type names are that driver's to describe. When a second driver
   with wide types arrives, its names join the list or the rule moves behind a driver
   hook; that is a later decision.
3. **The error-collection contract.** `SensorProfile::validate(&self) -> Result<(),
   Vec<ProfileError>>` never stops at the first problem, and an `Err` is never empty.
   The order is deterministic: the connection, then measurements in document order, and
   within a measurement the `MeasurementDef` field order (name, count, byte and word
   order, scaling steps, unit, status fields, quality mapping). Eleven variants are
   added to the `#[non_exhaustive]` enum: `DuplicateMeasurementName`, `UnknownUnit`,
   `EmptyUnit`, `MissingByteOrder`, `MissingWordOrder`, `OverlappingStatusBits`,
   `UnsupportedCount`, `InvalidScaleStep`, `UnknownQualityReference`,
   `UnknownAddressConvention` and `MissingAddressConvention`. They name a measurement by
   its 0-based index (plus the step, status field or quality entry index where that
   helps) and never carry document text, which keeps `ProfileError` `Copy` and keeps the
   no-echo guarantee; the name is one lookup away in `profile.measurements`. Every
   variant converts to `InvalidConfig`.
4. **What validation does not do.** It is independent of any driver. Address text,
   including status field addresses, is compared for equality only (two status fields on
   one measurement with the same address text and bit overlap); whether it parses is the
   driver's `parse_address`, called in S29. Two measurements may share a status word.
   Status-field names within one measurement are not required to be unique; a quality
   reference matches any field of that name.
   Non-`snake_case` names are not a validator error because the loader already refuses
   them. `ValueRange` and status-field bit positions are not checked at alpha.
5. **Scaling steps.** A step is invalid when a constant is not finite, `Divide` has a
   zero divisor (including `-0.0`), `Clamp` has `min > max`, or `BitField` has width 0 or
   `shift + width > 64`. Whether a step suits a data type is the S29 pipeline's concern.
6. **Tests.** `tests/validation.rs` has one test per variant, the multi-fault ordering
   test and a clean profile; `src/units.rs` pins completeness and sorting; a property
   test in `src/validate.rs` builds arbitrary schema instances with struct literals
   (possible only inside the crate, because the types are `#[non_exhaustive]`), checks
   that `validate` never panics and agrees with an independently written validity
   predicate, and a second property checks that profiles built to be valid validate clean.

## Addendum: tags and the measurement pipeline (slice S29, 2026-10-27)
This addendum records the S29 decisions. It changes no type above. The crate still
depends on no driver: `tags` takes a `&dyn Driver` from `stonecrab-core`, and the
simulator and the testkit join `proptest` as dev-dependencies (`stonecrab-driver-sim`
by path, `stonecrab-testkit` through the workspace), which adds no external crate.

1. **`SensorProfile::tags(&dyn Driver) -> Result<Vec<Tag>, ProfileError>`.** In order:
   the driver's scheme must equal `connection.scheme` (`SchemeMismatch { driver }`);
   `validate()` must pass, and the first error of its list is returned (call `validate`
   for all of them); the options come from `address_options()`, which is the profile's
   `address_convention` run through `AddressOptions::builder()` (a name core refuses is
   `UnknownAddressConvention`); then every address, measurement and status field, goes
   through `Driver::parse_address(text, &options)`. A bare short form therefore parses
   only under the convention the profile declared. No driver reads a convention yet
   (the simulator ignores it), so the testkit's `ConventionDriver` shows the options
   each `parse_address` call received, and a `tags` test asserts that every measurement
   and status address saw the declared convention.
2. **Tag shape.** A measurement yields one tag whose `TagId` is the measurement name,
   whose value type hint is whatever the driver's parsed address says (it is never
   overridden; `data_type` stays documentary here, because the type travels in the
   address), with `with_byte_order` and `with_word_order` taken from the definition when
   set. A status field yields an extra tag directly after its measurement, with the id
   `<measurement>.<status name>` (`MeasurementDef::status_tag_id`). Both names are
   `snake_case` without a dot, so the id cannot equal a measurement's tag, and the longest
   one is 129 bytes. The status tag keeps the driver's type hint too: an address that
   parses to `BOOL` (a coil, a discrete input) delivers the bit itself, one that parses to
   an integer word delivers the word and the pipeline reads `bit` from it. A driver
   rejection is `AddressRejected { measurement }` or `StatusAddressRejected {
   measurement, status_field }`, by index, never carrying the address. Two status fields of
   one measurement with the same name would share a tag id, so `tags` (not `validate`,
   which S28b left permissive) returns `DuplicateStatusFieldName`. `ProfileError` gains
   these four variants and stays `Copy`.
3. **`measurements(&ReadResponse, wall_now) -> Vec<Measurement>`.** One entry per definition
   whose tag is in the response, in document order; a definition whose item is absent
   is skipped (a response built from `tags` has every item, a partial request does not).
   `received_at` is `wall_now`, `received_mono` is the item's `sampled_at`, `source_time`
   is the item's and is never substituted (F6), `protocol_status` is the item's status on
   success and failure, `origin` is the profile's `ValidationStatus`, and `unit` is the
   definition's. `Measurement` is `#[non_exhaustive]` with public fields and a derived
   `Debug` (the only type that could hold bytes, `ProtocolStatus`, prints their count).
4. **The promotion rule.** Integers are held exactly in `i128` limited to
   `i64::MIN..=u64::MAX`; a constant is integral when it is finite, has no fractional
   part and has a magnitude below 2^64. `Multiply` and `Offset` keep an integer an integer
   when the constant is integral and the result stays in range; `Clamp` does when both
   bounds are integral (and then compares as integers); `BitField` always does.
   `Divide` always computes in `f64`: integer division loses the remainder, and a type
   that depended on the value (`100 / 10` against `101 / 10`) would be a worse contract.
   A fractional constant, or any step whose integer result would leave the range,
   computes in `f64` from then on. An integer result comes back as the input's own type
   when it fits, else `I64`, else `U64`; a float result is always `F64`; a pipeline with
   no steps passes the value through unchanged, including its type. All arithmetic is
   checked, and the module has no `as` cast: `u64` to `f64` is built from two exact
   32-bit halves (one rounding, as `as` rounds), and the float to integer test reads the
   IEEE bits. A NaN or infinite input, a non-finite intermediate or final result (so a
   `Divide` that overflows, and a divide by zero), a non-finite constant, `min > max`, a
   bit field of width 0 or ending past bit 64, `BitField` on a float and any step on a
   value that is not a number yield `Bad` with `value` `None`, whether or not the profile
   was validated. `BitField` reads the 64-bit pattern of the integer, negative values
   sign-extended, so a 16-bit field of the `U32` `0x0001_FFFF` is `0xFFFF` and the next
   count is `0`.
5. **Quality precedence.** `Bad` over `Uncertain` over `Unknown` over `Good`. A failed read
   or an unscalable value is `Bad`. A scaled value outside `range` (inclusive, compared
   as `f64`; a non-numeric value has no range check) is `Uncertain`: schema version 1
   has no way to say a violation is `Bad`, so the plan's "per map" is the fixed
   default. For each status field named in the quality mapping, its bit is the `Bool`
   itself or bit `bit` of the integer, flipped by `invert`; when set it is `Bad` for
   `bad_when` (also when the name is in both lists) or `Uncertain` for `uncertain_when`;
   a missing, failed, non-integer or out-of-range-bit status item is `Unknown`. A status
   field no list names, and a definition with no `quality_map`, have no effect. A status
   outcome never removes the scaled value; only a failed read or a failed scaling does.
   `Quality::Stale` exists and is never produced here: staleness needs the age of a value,
   which one response cannot give, so only the M2 tracker sets it. For the tie-break the
   private severity order is `Good` < `Unknown` < `Uncertain` < `Stale` < `Bad`.
6. **`keep_raw`.** When set and the read succeeded, `raw` is a clone of the unscaled typed
   `PlcValue` as the driver delivered it (an `F32` stays `F32`, a NaN stays a NaN). When
   unset, or when the read failed, `raw` is `None`. It is not wire bytes: a typed-decode
   failure carries its bytes on the `ItemError` status (D14, ADR-0006).
7. **Tests.** `tests/pipeline.rs` runs profiles from TOML through the simulator's address
   grammar and covers F7 (negative and scaled values, NaN and infinity, counter rollover),
   F6 (absent source time), naming, hints, orders, scheme mismatch, the quality
   precedence, an end-to-end read from the simulator and four property tests (random
   steps and values never panic or report a non-finite number; multiply then divide
   restores a float within an epsilon and an integer exactly; a bit field equals the
   shift-and-mask definition). Unit tests in `src/measurement/scale.rs` pin every branch
   of the engine and the conversions, and `src/tags.rs` pins `address_options`.
   `docs/parity/sensors.md` gains `SENSORS-TAGS` and extends `SENSORS-MEASUREMENTS`.

## Addendum: synthetic profiles, the facade `sensors` feature and the profile example (slice S30, 2026-10-27)
This addendum records the S30 decisions. It changes no type above; the crate gains only a
test file, and the first profiles, the facade feature and the example are built on the
contracts of S28a to S29.

1. **Profiles are repository data.** The six synthetic profiles live in `profiles/synthetic/`
   at the workspace root, three devices (`temperature_transmitter`, `pressure_flow_level`,
   `discrete_state`) in two variants (decision D17): `modbus-tcp/` with
   `address_convention = "plc4x"` and `sim/` with `STATE/<name>:<type>` addresses and no
   convention. No crate archive contains them: the `include` lists of the sensors and facade
   crates are unchanged and `packaging/*.txt` still pins them. Shipping profiles with a crate
   is a milestone M2 decision (named device profiles will need it). `profiles/README.md`
   documents each file; `crates/stonecrab-sensors/tests/profiles.rs` reads the directory
   relative to `CARGO_MANIFEST_DIR`, so the test needs a checkout and is not part of any
   archive either.
2. **The `synthetic` validation level.** Every file says `validation = "synthetic"`, the
   lowest label of SPEC.md "Hardware evidence". It means the profile was authored from a
   documented register map and describes no device: the map is
   `fixtures/modbus/interop/register-map.toml` (rows `MB-INTEROP-*` of
   `docs/parity/modbus.md`), written for the pymodbus peer, and no manufacturer document,
   capture or hardware record stands behind it. The `[provenance]` of each file names the map
   and the rows; `fixtures` names the map file. The map's values are test values (an `INT`
   of -2424 scaled by 0.1 is -242.4 kPa, a `UDINT` of 4242442424 is a flow in L/min), so the
   ranges are chosen to contain them and make no physical claim. Every `Measurement` the
   profiles yield carries `origin = Synthetic`. No named device profile exists and none is
   implied (SPEC.md "Sensor profile contract").
3. **Twins.** A Modbus file and its sim twin have the same file name, the same measurement
   names and units, and ids that differ by the suffix `_sim` (the sim id is the Modbus id
   plus `_sim`); the test asserts all three. The sim twin has no byte or word order because
   the simulator stores native values, and its `data_type` is the simulator's own spelling
   (`f32`, `i16`, `u32`, `u16`, `bool`), which the validator passes through. Status fields
   cover the pipeline: a coil read as a status bit, a bit of a `WORD` register, an inverted
   coil (`probe_lost`), eight inverted and non-inverted discrete-input flags, `bad_when` and
   `uncertain_when`, `keep_raw` on the unscaled `REAL` and `INT`, and the little-endian
   region (`MB-INTEROP-LE`) read as `little_endian` with `low_word_first`.
4. **The facade feature.** `sensors = ["dep:stonecrab-sensors", "stonecrab-sensors/toml"]`
   and `full = ["sim", "modbus", "sensors"]`, both off by default (D10); `stonecrab::sensors`
   re-exports the crate with `pub use stonecrab_sensors::*;`, cfg-gated and `doc(cfg)` like
   the other modules. The workspace entry is `stonecrab-sensors = { path, version =
   "=0.1.0", default-features = false }`, so the facade pins the sibling exactly
   (CODING_STANDARDS section 20.1) and the loader is switched on by the facade feature
   rather than by the sibling's default: `SensorProfile::from_toml` exists only behind
   `toml`, and the `profile_read` example and the facade test need it. `cargo add stonecrab`
   still pulls core only.
5. **The example.** `crates/stonecrab/examples/profile_read.rs` (`required-features =
   ["full"]`) reads `STONECRAB_URL` (default `sim://bench`) and `STONECRAB_PROFILE` (default
   `profiles/synthetic/sim/temperature_transmitter.toml`, read relative to the current
   directory, so it resolves when the example is run from the repository root). It registers a
   `SimDriver` holding the device `bench` and a `ModbusTcpDriver`; the bench `STATE` tags hold
   the register map's values (`temperature` 3.141593, `pressure` -2424, `flow` 4242442424,
   `flow_status` 0xA5B8, `level` 42424, `coil_1` to `coil_10`, `discrete_input_1` to
   `discrete_input_10`), so a sim file and its Modbus twin yield the same measurements. It
   loads and validates the profile, parses the connection string through the registry,
   takes `ConnectionConfig::address_options()` before `connect` consumes the configuration
   and refuses a connection string whose named convention differs from the profile's, builds
   the tags with `profile.tags(driver)` (which hands the profile's convention to every
   `parse_address` call), reads them within 2 s counted from `Instant::now()` (the
   simulator's `RealtimeClock` and the Modbus `SystemClock` agree with it outside paused
   test time), computes `profile.measurements(&response, SystemTime::now())` and emits one
   `tracing::info!` per measurement with `name`, `value`, `unit`, `quality`, `origin`,
   `sampled_at` and, when the profile keeps it, `raw`, then one event with `split_into`.
   It installs the `fmt` layer itself (ADR-0011), prints nothing else and returns
   `ExitCode`. With `--expect-good` it reports every measurement first and then returns
   `FAILURE` if any quality is not `Good` or the response covers fewer measurements than
   the profile defines. No library item exists only for the example: it calls
   `SensorProfile::{from_toml, validate, address_options, tags, measurements}` and the
   registry, driver and connection contracts that their own tests exercise.
6. **CI.** The `examples` job and `mise run examples` run `profile_read --expect-good`
   against `sim://bench` once per sim profile. `interop/pymodbus/interop.sh` gains the
   `profiles` step, after `example` and part of `run`, which runs it against the container
   once per `profiles/synthetic/modbus-tcp/*.toml` (the step fails unless exactly three
   exist), writes `profiles.txt` into the evidence directory and keeps going after a failure
   so that every profile is reported; the CI `interop` job calls it as a step. The `features`
   job's `cargo hack check --feature-powerset --depth 2` covers `sensors` alone,
   `sensors` with `modbus` or `sim`, and `full` without a change.
7. **D23 result.** `cargo tree --locked -e normal,dev -i once_cell`, `-i lazy_static` and
   `-i thread_local` show the paths already recorded in ADR-0012 and ADR-0014 and no new
   one: `once_cell` through `tracing-core` (from `tracing` in `stonecrab-driver-sim` and
   `stonecrab-modbus`, and from `tracing-subscriber`), `lazy_static` through `sharded-slab`
   and `tracing-subscriber`, `thread_local` through `tracing-subscriber`. `-i log`, `-i time`
   and `-i chrono` match no package. `serde` and `toml` were already in the graph through
   `stonecrab-sensors`; the facade feature adds the edge `stonecrab -> stonecrab-sensors`
   and nothing else. `cargo tree -p stonecrab -e normal` with default features is
   `stonecrab-core` and its two dependencies only; with `--features full` it adds the
   simulator (`tracing`), the Modbus crate (codec, transport, `tokio`) and the sensors crate
   (`serde`, `toml`).
8. **Tests and rows.** `tests/profiles.rs` (five tests, tagged `// covers:
   SENSORS-SYNTHETIC-PROFILES`) and the facade test `tests/sensors_feature.rs`. The row
   `SENSORS-SYNTHETIC-PROFILES` in `docs/parity/sensors.md` moves from `planned` to
   `implemented`; the sensors README support table still lists the profiles as planned and
   is brought to final support levels by S31.

## Addendum: the measurement range rule (slice S36, 2026-10-28)
This addendum records the S36 decision (G01). `ProfileError` gains a twelfth validation
variant, `InvalidRange { measurement }`, which supersedes the S28b sentence that
`ValueRange` is not checked: when a measurement carries a range, both bounds must be
finite and `min` must not exceed `max`. The rule sits after the unit check and before the
status fields, following the `MeasurementDef` field order, and the variant converts to
`InvalidConfig` like the others. It names the measurement by index and never echoes the
document. Status-field bit positions stay unchecked until S37.

## Addendum: the word-order rule follows the driver (slice S39, 2026-10-28)
This addendum records the S39 decision for finding G05 of `docs/POST_ALPHA_PLAN.md`. It
changes one behavior above and no type.

1. **Reason.** S28b keyed the rule "a value wider than 16 bits has both a `byte_order` and a
   `word_order`" on eight Modbus type names inside the driver-independent `validate`. That
   was correct for Modbus and wrong as soon as a second driver has wide types, and the
   `sensors` module should not name any driver's types. `tags` already treats `data_type`
   as documentary: the type comes from the parsed address.
2. **`ProtocolAddress::spans_multiple_words(&self) -> bool`.** A provided method, false by
   default, true when the value at the address occupies more than one of the protocol's
   words. `ModbusAddress` overrides it from its data type's register count (every type
   wider than one register); simulator addresses keep the default.
3. **`SensorProfile::validate_for(&dyn Driver)`.** It runs every `validate` rule, then
   parses each measurement's address under `address_options()` as `tags` does and reports,
   in document order, `AddressRejected` for an address the driver refuses and
   `MissingByteOrder` and `MissingWordOrder` when the parsed address spans words and the
   definition does not set that order. `tags` runs `validate_for` instead of `validate`,
   so a Modbus profile without orders is still refused there.
4. **Behavior change.** `validate` no longer reports `MissingByteOrder` or
   `MissingWordOrder`; a caller that relied on `validate` alone to find a missing order
   must call `validate_for` or `tags`. The change is additive in API terms (a provided
   trait method and a new method) and is recorded in `CHANGELOG.md` and in the
   `SENSORS-VALIDATION` row of `docs/parity/sensors.md`.

## Addendum: status bits and overlap follow the parsed address (slice S37, 2026-10-28)
This addendum records the S37 decision for findings G02 and G03 of
`docs/POST_ALPHA_PLAN.md`. It supersedes the S28b sentences that status-field bit positions
are not checked and that status addresses are compared as text only, and the S36 note that
bit positions stay unchecked until S37. `ProfileError` gains two variants,
`StatusBitOutOfRange { measurement, status_field }` and
`StatusBitInBitField { measurement, status_field }`; both name indices only, never echo the
document and convert to `InvalidConfig`.

1. **Reason.** A status bit past the data's width (bit 64, or bit 16 of a `WORD`) passed
   validation. The bit is not in the data: at 64 or more it reads as `Quality::Unknown`, below
   64 it reads as zero or a sign-extension copy, and a `Bool` ignores it, so the rule is
   stricter than the runtime on purpose. The overlap rule compared
   address text, so `4x00001:WORD` and `holding-register:1:WORD` were two addresses. The
   width and the canonical form are the driver's to say, so the precise rules sit in
   `validate_for`, which now also parses each status field's address. The refusal maps to
   `StatusAddressRejected` once, in a helper shared with `tags`.
2. **Bit bounds.** `validate` refuses a status bit of 64 or more
   (`StatusBitOutOfRange`): no value is wider, whatever the driver. `validate_for` refuses a
   bit at or beyond the width of the value type of the parsed status address: 1 for a
   `Bool` (a coil or discrete input has only bit 0), 8, 16, 32 or 64 for the integer types.
   A type with no bit width (a float, text, a duration, or no type hint) is not checked. A
   bit of 64 or more is reported by `validate` only, so no error is duplicated.
3. **Overlap shapes.** Addresses are equal when `ProtocolAddress::canonical()` is equal.
   Within one measurement:
   - two status fields with the same canonical address and the same bit are
     `OverlappingStatusBits`, reported for the later field. Equal text is still found by
     `validate`; `validate_for` adds only pairs whose text differs, so a pair is never
     reported twice, and the errors of `validate` precede those of the driver;
   - a status bit inside `[shift, shift + width)` of the measurement's first scaling step,
     when that step is a `BitField` and the status address has the measurement's own
     canonical address, is `StatusBitInBitField`. Only the first step reads the raw word
     the status address names; a `BitField` after another step reads a derived value and
     claims no bit;
   - two `BitField` steps are possible, but they compose (the second reads the value the
     first extracted), so their ranges are in different coordinates and a second step is
     not an overlap. The first step's range is the claim.

   Measurements may share a status word: the same address and bit in two measurements is
   valid, as `measurements_may_share_a_status_word` pins.
4. **Limits.** Canonical text includes the data type and the unit id, so one Modbus
   register named as `WORD` and as `INT`, or in two units, is two addresses and passes. A
   rule on the register itself needs a location the `ProtocolAddress` trait does not offer;
   it is a candidate for a later provided method, not part of this slice. A measurement
   whose own address the driver refuses claims no bit (`AddressRejected` stands alone for
   it), and the width of the measurement's value against its `BitField` is not checked.
5. **Tests.** `tests/sensors_overlap.rs` covers every alias spelling of a register, a coil
   and a discrete input, each overlap shape, the widths 1, 8, 16, 32 and 64, error order,
   no echo, and a property test against an independent restatement of the overlap rule;
   the `validate` property test's oracle gains the bit bound. The runtime guard in the
   pipeline stays for profiles that were never validated.

## Addendum: the unit is optional (slice S74, 2026-10-29)
This addendum records the S74 decision (finding G06; maintainer decisions E7 and E12 of
`docs/POST_ALPHA_PLAN.md`). The decision above made `unit` a required key, so a profile
could not say that the unit of a value is unknown, and the envelope (`SPEC.md`: "unit when
known") had no way to carry that. It changes the two types that hold a unit and nothing
else:

1. **Types.** `MeasurementDef::unit` and `Measurement::unit` are `Option<String>`.
   `MeasurementDef::unit` is `#[serde(default)]`, so an absent `unit` key loads as `None`
   and means "unit unknown". `Measurement::unit` copies it. Every producer that knows its
   unit sets `Some`: the MAX31855 and ADXL345 board adapters (`boards::measurement`) always
   do. No unit is guessed, defaulted or inferred from the data type or the address.
2. **Validation.** An absent unit is valid. A stated unit is checked as before: the empty
   string and now also a blank one (only whitespace) are `EmptyUnit`, and anything else
   outside the table is `UnknownUnit`. `EmptyUnit` is not an absent unit: a profile that
   writes `unit = ""` is still refused, so a typing slip is not read as "unknown". The
   message of `EmptyUnit` tells the author to omit the key when the unit is unknown. No
   variant is added to `ProfileError`.
3. **Schema version.** The schema stays at version 1. Version 2 is for a document that a
   version 1 reader would misread; here the change only widens version 1: every version 1
   document that 0.2.0 loads loads the same way now, and the only new documents are those
   without a unit. A 0.2.0 reader refuses such a document with `Syntax` (a missing field)
   instead of `IncompatibleSchema`; that is the cost of not spending version 2 on this, and
   it is accepted because profiles are repository data that travel with the crate version
   that reads them. The synthetic and documented profiles all state their units and do not
   change.
4. **Release.** Changing the type of a public field is a breaking change of a 0.x crate
   (CODING_STANDARDS 9.10): `stonecrab` moves from 0.2.0 to 0.3.0 (E12), with a "Migration
   from 0.2" note in `CHANGELOG.md`. cargo-semver-checks 0.51.0 has no lint for the type
   of a public field, so `mise run semver` does not report this change; the note and this
   addendum are the record of it.
5. **Tests.** `a_measurement_without_a_unit_loads_with_no_unit` (loader),
   `an_absent_unit_means_unknown_and_is_valid` and `a_blank_unit_is_reported_as_empty`
   (validation), `a_definition_without_a_unit_yields_a_measurement_without_one`
   (pipeline); the existing board tests now assert `Some("Cel")`, `Some("1")` and
   `Some("[g]")`, and the validation property test's oracle and generator include an
   absent unit.

## Addendum: schema version 2 and value-matched quality (slice S73, 2026-10-29)
This addendum records the S73 decision (maintainer decision E11 of
`docs/POST_ALPHA_PLAN.md`; finding of S52d). The decision above lets a quality come from a
status bit and from a plausibility range, and a range violation is only `Uncertain`. A device
that reports a fault as a reserved value of the measurement itself, such as the SOLO SLM1-C
whose process value reads 8002H to 8007H (except 8005H, which the manual does not list) when
it has no temperature, could therefore be reported no worse than `Uncertain`. Version 2 of
the schema adds the one key that fixes this.

1. **Version 2 is version 1 plus one key.** `SchemaVersion` accepts 1 and 2 and refuses every
   other value with `IncompatibleSchema`; `SchemaVersion::V1` and `V2` name them and
   `CURRENT` is the newest, 2 (it was the only version, 1; code that means version 1 uses
   `V1`). A version 1 document loads exactly as before, with no value rules. The key needs a
   version of its own, although a reader that does not know it already refuses the document
   (`deny_unknown_fields`): the key changes what a document says about a value, the version
   is what tells an author and a reader which meaning applies, and a 0.2.0 reader refuses a
   version 2 document with `IncompatibleSchema { found: 2 }`, not with a position in a file.
   The S74 addendum above kept the unit at version 1 for the opposite reason: it only widened
   what version 1 accepts and gave no value a new meaning.
2. **The key.** `MeasurementDef::value_rules` (TOML `value_rules`, default empty) is a list of
   `ValueRule { values: Vec<i64>, quality, reason: Option<String> }`:

   ```toml
   value_rules = [
       { values = [-32765], quality = "bad", reason = "temperature sensor not connected" },
   ]
   ```

   `quality` is `"bad"` or `"uncertain"`; a rule never raises a quality, so `"good"`,
   `"stale"`, `"unknown"` and any other text are a `Syntax` error of the loader.
3. **Raw values, not ranges.** A rule lists values, and the value is compared with the
   unscaled integer the driver delivered, before the scaling steps: a device documents its
   reserved codes as numbers of its register, and a rule written on the scaled value would
   change with every scaling step. The comparison reads the value as `i64` (`PlcValue::as_i64`),
   so every signed type and every unsigned type up to `u32` match, an unsigned value above
   `i64::MAX` does not, and a float, a Boolean and text never do. Ranges of values are not
   added: the one device that needs the key lists exact codes, 8005H sits among them and is
   unlisted, and a range would have swallowed it; a key for ranges can be added in a later
   version when a profile needs one, and a second shape with no caller is the cost avoided.
   The first rule of a measurement that lists a value applies, which needs no check for a
   value listed twice.
4. **How a rule ranks.** The rule's quality joins the range quality and the status quality by
   the precedence of the pipeline, `Bad` over `Uncertain` over `Unknown` over `Good`. The
   scaled value and the raw value are kept, as for a status. The new field
   `Measurement::quality_reason` (`Option<String>`, additive: `Measurement` is
   `#[non_exhaustive]`) holds the rule's reason when the rule's quality is the one the
   measurement ends up with; a worse status, or a failed read, gives none. It is the profile's
   own text, kept as it is when a `StaleTracker` marks the measurement stale (the reason of
   the quality it was read with, which is in `quality_before_stale`). The board adapters set
   `None`.
5. **Validation.** The loader checks shape only, so a version 1 document with the key loads;
   `SensorProfile::validate` reports `ValueRuleNeedsSchema2` once for each measurement that
   has rules in a version 1 profile and `EmptyValueRule` for a rule that lists no value (it
   could never apply). Both are new `ProfileError` variants after the existing ones, are
   reported after the quality-mapping errors of their measurement, and convert to
   `InvalidConfig` like the others. The text of `IncompatibleSchema` now says "versions 1 and 2".
6. **The documented SOLO profile moves to version 2.** `profiles/documented/modbus-rtu/solo_slm1_c.toml`
   lists the five codes as signed 16 bit integers (-32766, -32765, -32764, -32762 and -32761),
   each `Bad` with the manual's meaning as its reason; the replay compares the rules with the
   fixture's `[[sentinels]]` and the reasons with their meanings, and reads 8005H as `Uncertain`
   by the range with no reason. The synthetic profiles and the STRIDE profile stay at version 1.
7. **Tests and fixtures.** `tests/profiles/schema_v2.toml`, which stood for a document of a
   later schema, is `schema_v3.toml` and the test that used it is
   `schema_version_3_is_rejected_as_incompatible` (it was
   `schema_version_2_is_rejected_as_incompatible`, named in the compliance list above).

Compliance: `schema_version_2_loads_and_version_1_documents_load_as_they_did`,
`schema_version_3_is_rejected_as_incompatible`, `schema_version_validates_directly`,
`value_rules_load_with_values_quality_and_an_optional_reason`,
`a_listed_value_sets_the_quality_and_gives_its_reason`,
`an_unlisted_value_keeps_the_range_and_status_rules_and_has_no_reason`,
`rules_match_the_unscaled_integer_of_any_integer_type`,
`the_first_rule_that_lists_a_value_applies`,
`a_rule_ranks_with_the_range_and_status_rules_and_gives_a_reason_only_when_it_decides`,
`a_failed_read_is_bad_with_no_reason`, `value_rules_need_schema_version_2`,
`a_value_rule_without_a_value_is_reported`, and for the SOLO
`a_process_variable_error_code_is_never_a_good_reading`.

## Addendum: `toml` 1.1 retires the `toml_parser` pin (2026-10-29)
Decision 9 adopted `toml` 0.9 and pinned `toml_parser` at 1.0.9 in `Cargo.lock` and
`fuzz/Cargo.lock`, so that one `winnow` resolved. Dependabot PR #3 proposed `toml` 1.1, the
first alternative that decision names, and the move is taken: the root
`[workspace.dependencies]` entry and the codegen tool's own entry are `toml = { version =
"1.1", default-features = false, features = ["parse", "serde", "std"] }` (resolved 1.1.8,
with `toml_parser` 1.1.5, `toml_datetime` 1.1.2 and `winnow` 1.0.4). `toml` and its parser
now share `winnow` 1.0, so all three locks (root, `fuzz/`, `tools/stonecrab-codegen/`) hold
one `winnow` with no pin, and a plain `cargo update` no longer reintroduces a duplicate.

The feature names are unchanged and the depth limit is still on (`unbounded` stays off).
The one breaking change of `toml` 1.0, `Time::second` and `Time::nanosecond` becoming
`Option`, touches no code here: profiles hold integers, strings and tables, and no `toml`
type is in the public API (`ProfileError::Syntax` carries a line and a column,
`GoldenError` a message), so cargo-semver-checks against the published 0.3.0 reports no
change and the bump needs no version change. The error positions that
`invalid_toml_is_a_syntax_error_with_a_position` asserts are unchanged. `toml` 1.1 needs
Rust 1.85, below the crate's MSRV of 1.97. `docs/DEPENDENCY_AUDIT.md` records the new
versions and the recounted `unsafe` of `winnow` 1.0.4.
