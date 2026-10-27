# Stonecrab Sensors
Declarative sensor profiles for Stonecrab: versioned TOML data that maps protocol
addresses to named measurements, with a data type, byte and word order, a scaling
pipeline, a unit, quality rules and the evidence behind it. A profile contains no
executable code and is never written by this library.

This crate currently offers the schema types, a bounded loader, semantic validation and
the measurement pipeline: `SensorProfile::from_toml`, `SensorProfile::validate`,
`SensorProfile::tags`, `SensorProfile::measurements`, `Measurement`, `Quality`, `Unit`,
`ProfileId`, `SchemaVersion` (version 1 is the only schema version; any other value is
`ProfileError::IncompatibleSchema`), `DeviceApplicability`, `ConnectionRequirements`,
`MeasurementDef`, `ScaleStep`, `ValueRange`, `StatusField`, `QualityMapping`,
`Provenance`, `ValidationStatus` and `ProfileError`. The loader checks the shape of a
document, `validate` checks its meaning, `tags` asks a driver to parse its addresses and
`measurements` turns a read response into qualified engineering values. The crate depends
on `stonecrab-core`, `serde` and, behind the default `toml` feature, `toml`; it depends on
no driver.

Profiles declare their address convention. A bare Modbus number such as `40001` is
ambiguous (holding register 1 under the PLC4X convention, wire address 40001 otherwise),
so a profile that uses the `modbus-tcp` scheme must say which convention its addresses
follow in `address_convention`, and the driver parses every address under it. A profile
names one connection scheme and its addresses belong to that scheme's grammar; a device
that is reachable through two schemes has two profile files (decision D17). No named
device profile exists: every profile that ships with Stonecrab is a synthetic reference,
labeled `synthetic`, and describes no real device. A named profile will need a real
register map and its own fixtures.

| Capability | Support level |
| --- | --- |
| Schema types and `SensorProfile::from_toml` (schema version 1) | build-tested; fuzzed |
| Semantic validation (`SensorProfile::validate`) and the fixed unit table | build-tested |
| Scaling and quality evaluation (`SensorProfile::measurements`) | build-tested; property-tested |
| Driver integration (`SensorProfile::tags` through `Driver::parse_address`) | build-tested; run against the simulator |
| Synthetic reference profiles | planned |
| Interoperability against an independent server | planned |
| Named device profiles | planned |
| Hardware verification | planned |

## Features
- `toml` (default): the TOML loader, `SensorProfile::from_toml`, and the `toml` crate
  behind it. Without it the crate still builds and exposes every schema type, which
  deserialize with any `serde` format, but the library itself cannot load a file: enable
  `toml` to load profiles.

## Profile format
A document is TOML with a bounded size (64 KiB). Unknown keys are errors at every level,
and an error reports a line and column but never repeats any part of the document.

```toml
id = "synthetic_temperature"
schema_version = 1
validation = "synthetic"        # synthetic | documented | fixture_backed | hardware_verified

[device]                        # optional; every key optional
manufacturer = "Synthetic Manufacturer"

[connection]
scheme = "modbus-tcp"
address_convention = "plc4x"    # declared, never inferred

[connection.options]
default-unit-identifier = "1"

[provenance]
source = "Synthetic reference profile; no real device"
fixtures = []

[[measurements]]
name = "boiler_temperature"     # snake_case ASCII; doubles as the tag id
address = "holding-register:1:INT"
data_type = "INT"               # the driver's own type name, passed through
unit = "Cel"
scaling = [{ divide = 10.0 }, { offset = -40.0 }, { clamp = { min = -40.0, max = 215.0 } }]
range = { min = -40.0, max = 215.0 }
min_interval_ms = 1000          # whole milliseconds
stale_after_ms = 5000
keep_raw = true                 # keep the unscaled typed value, not wire bytes
```

Optional measurement keys are `count` (default 1), `byte_order` (`big_endian` or
`little_endian`), `word_order` (`high_word_first` or `low_word_first`), `scaling`
(`multiply`, `divide`, `offset`, `clamp` and `bit_field` steps, applied in order),
`range`, `status_fields` (`name`, `address`, `bit`, `invert`), `quality_map`
(`bad_when` and `uncertain_when`, lists of status field names), `min_interval_ms`,
`stale_after_ms` and `keep_raw`. Byte and word order are never inferred: an absent key
stays absent.

## Validation and units
`SensorProfile::validate` returns `Ok(())` or every `ProfileError` it finds, never just the
first, in a fixed order: the connection, then each measurement in document order. Errors
name a measurement by its 0-based index and never repeat any text of the profile. It
checks:

- the address convention: when set it must be `plc4x`, and a `modbus-tcp` connection must
  set it;
- unique measurement names (names are already `snake_case`, which the loader enforces);
- `count` is 1 (arrays are not supported at alpha);
- both `byte_order` and `word_order` for a data type wider than 16 bits. The width is
  decided from the data type name by the Modbus rule: `DINT`, `UDINT`, `DWORD`, `LINT`,
  `ULINT`, `LWORD`, `REAL` and `LREAL` (upper case) are wide, and any other name is
  passed through with no requirement, because another driver's types are that driver's
  to describe;
- scaling steps that can run: finite constants, a non-zero divisor, a clamp with
  `min <= max`, and a bit field of width 1 or more that ends at or before bit 64;
- a unit that is not empty and is in the fixed unit table;
- no two status fields of one measurement on the same address text and bit;
- quality mappings that name only the measurement's own status fields.

Validation is independent of any driver: address text is compared for equality only, and
the driver's `parse_address` validates it when `SensorProfile::tags` builds the tags.

The unit table is a fixed list of UCUM-style codes with a description each, and there
is no unit arithmetic: nothing converts between units or parses compound expressions.
`Unit::all()` lists the codes in byte order: `%`, `1` (dimensionless), `A`, `Cel`, `Hz`,
`K`, `L`, `L/min`, `Pa`, `V`, `W`, `bar`, `kPa`, `kWh`, `m`, `m3/h`, `mm`, `ms` and `s`.
Matching is exact and case-sensitive.

```rust
use stonecrab_sensors::Unit;

assert!(Unit::is_known("Cel"));
assert_eq!(Unit::description("m3/h"), Some("cubic metre per hour"));
assert!(!Unit::is_known("celsius"));
```

## Tags and measurements
`profile.tags(&driver)` returns the tags to read. The driver's scheme must equal the
profile's `connection.scheme` (otherwise `ProfileError::SchemeMismatch`), the profile must
validate, and every address is parsed by `Driver::parse_address` under the profile's
`address_convention`, so a bare short form such as `40001` parses only under the declared
convention. A measurement becomes one tag whose `TagId` is the measurement name, with the
driver's own type hint and the declared byte and word order. Each status field becomes an
extra tag with the id `<measurement>.<status name>`, for example
`boiler_temperature.open_circuit`. A rejected address is a `ProfileError` that names the
measurement by index and never repeats the address text.

After the read, `profile.measurements(&response, wall_now)` returns one `Measurement` per
definition that the response covers:

- `value` is the scaled value, `raw` the unscaled typed value when the definition sets
  `keep_raw` (and `None` otherwise), `unit` and `origin` come from the profile.
- `received_at` is the `wall_now` you pass, `received_mono` is the driver's sampling
  instant and `source_time` is the device's own timestamp. It is `None` when the driver
  reports none and is never filled in from another clock.
- `protocol_status` is the status the driver attached to the item.

Scaling runs in the declared order with checked arithmetic. An integer stays an integer
through `multiply` and `offset` with an integral constant, through `clamp` with integral
bounds and through `bit_field`; it keeps its own type when the result fits, else becomes
`I64`, else `U64`. `divide`, a fractional constant, or a result beyond 64 bits computes in
`f64`, and a float result is always `F64`. A NaN or infinite input or result, an
unevaluable step, or a failed read gives quality `Bad` and no value.

Quality is the worst of the rules that apply, in the order `Bad`, `Uncertain`, `Unknown`,
`Good`: a value outside `range` is `Uncertain`; a status field in `bad_when` that is set
(after `invert`) is `Bad`, one in `uncertain_when` is `Uncertain`; a status item that is
missing or unreadable is `Unknown`. `Quality::Stale` is never produced here: only the
milestone M2 tracker, which knows a value's age, sets it.

```rust
use stonecrab_sensors::Quality;

assert_eq!(Quality::Uncertain.to_string(), "uncertain");
```

The parity record is
[docs/parity/sensors.md](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/sensors.md);
upstream Apache PLC4X has no sensor profile concept, so the format is Stonecrab's own.

```rust
use stonecrab_sensors::{ProfileError, ProfileId, SchemaVersion};

// Identifiers are snake_case ASCII; the same rule names measurements.
assert!(ProfileId::new("synthetic_level").is_ok());
assert!(ProfileId::new("Synthetic Level").is_err());

// Schema version 1 is the only one; a document written for another version is refused.
assert_eq!(SchemaVersion::new(1)?, SchemaVersion::CURRENT);
assert_eq!(SchemaVersion::new(2), Err(ProfileError::IncompatibleSchema { found: 2 }));
# Ok::<(), ProfileError>(())
```

`SensorProfile::from_toml` has a runnable example on its own documentation page.
