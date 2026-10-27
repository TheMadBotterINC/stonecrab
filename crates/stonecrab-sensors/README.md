# Stonecrab Sensors
Declarative sensor profiles for Stonecrab: versioned TOML data that maps protocol
addresses to named measurements, with a data type, byte and word order, a scaling
pipeline, a unit, quality rules and the evidence behind it. A profile contains no
executable code and is never written by this library.

This crate currently offers the schema types and a bounded loader, nothing more:
`SensorProfile::from_toml`, `ProfileId`, `SchemaVersion` (version 1 is the only schema
version; any other value is `ProfileError::IncompatibleSchema`), `DeviceApplicability`,
`ConnectionRequirements`, `MeasurementDef`, `ScaleStep`, `ValueRange`, `StatusField`,
`QualityMapping`, `Provenance`, `ValidationStatus` and `ProfileError`. The loader checks
the shape of a document. It does not yet validate its meaning (the units table,
duplicate names, overlapping status bits, unsupported steps, the address convention
rule); that is slice S28b. There is no driver integration yet either: nothing here
builds a `Tag` or reads a device. The crate depends on `stonecrab-core`, `serde` and,
behind the default `toml` feature, `toml`.

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
| Semantic validation and the units table | planned |
| Scaling and quality evaluation | planned |
| Driver integration (building tags from a profile) | planned |
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
