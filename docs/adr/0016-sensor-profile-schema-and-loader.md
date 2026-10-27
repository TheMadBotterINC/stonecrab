# 0016: Sensor profile schema and TOML loader
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S28a of docs/ALPHA_PLAN.md.
It adds the new crate `stonecrab-sensors` (CODING_STANDARDS 12.6 requires an ADR for a
new crate), adopts `serde` and `toml` (section 15.1), and implements decisions D10, D17
and D23 for this crate. It builds on ADR-0005 (the sensors coverage row), ADR-0009 (the
fuzz workspace) and ADR-0013 (how a dependency is adopted). Semantic validation of a
profile is slice S28b and is not part of this decision.

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
