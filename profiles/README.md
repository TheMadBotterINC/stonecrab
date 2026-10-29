# Sensor profiles
Declarative sensor profiles for Stonecrab, as repository data. Each file is a TOML document
in schema version 1 that `stonecrab::sensors::SensorProfile::from_toml` loads, `validate`
checks and `tags` turns into the tags a driver reads (see the
[sensors component document](../crates/stonecrab/docs/components/sensors.md) for the format and
[ADR-0016](../docs/adr/0016-sensor-profile-schema-and-loader.md) for the decisions).

Profiles are repository data. They are not shipped in the crate archive (the reviewed
list `packaging/stonecrab.txt` that `mise run package` compares pins this), and whether a later
release ships them is a milestone M2 decision.

There are two kinds, told apart by directory and by `validation`: `synthetic/` (invented test
data) and `documented/` (two named devices, from the vendors' documents).

## What `validation = "synthetic"` means
Every profile under `synthetic/` says `validation = "synthetic"`, the lowest of the four evidence labels
of SPEC.md "Hardware evidence". A synthetic profile is authored from a documented register
map and describes no device: there is no manufacturer manual, no retained device capture and
no hardware record behind it. The map is the pymodbus register map of the interoperability
job, `fixtures/modbus/interop/register-map.toml` (rows `MB-INTEROP-*` of
[docs/parity/modbus.md](../docs/parity/modbus.md)), so the values are test values: an `INT`
of -2424 read as 0.1 kPa steps is -242.4 kPa, and a `UDINT` of 4242442424 is a flow in
litres per minute. They are arbitrary but stay inside the declared ranges. Every measurement
carries `origin = Synthetic`, so an application can tell.

Do not read the synthetic files as a statement about any manufacturer's device. SPEC.md
"Sensor profile contract" documents profiles independently from drivers; a named profile
needs a real register map and its own fixtures, which the `documented/` profiles below have.

## What `validation = "documented"` means
The two files under `documented/` describe real devices, the AutomationDirect STRIDE
SIO-MB04THMS (hardware record HW1) and SOLO Modular SLM1-C (HW2). Each is labeled
**documented map, not hardware-verified**: the register map is transcribed from the vendor's
document, which is pinned by URL and sha256 in the profile and in
`fixtures/modbus/devices/PROVENANCE.toml` and is not copied here, and the profile is
replayed against that transcription (`fixtures/modbus/devices/`,
`crates/stonecrab/tests/modbus_device_replays.rs`) through the first-party responder. No
device was read, so there is no hardware-verified device profile. What the hardware must
settle is listed in `docs/hardware/hw1-sio-mb04thms.md` and
`docs/hardware/hw2-slm1-c.md`. `Measurement.origin` of every value they produce is
`Documented`. Their addresses follow the documents' own numbers read as wire addresses
(`address_convention = "zero-based"`); a device that counts from one is a one-word change.

## Layout and conventions
| Directory | Scheme | Addresses | Evidence |
| --- | --- | --- | --- |
| `synthetic/modbus-tcp/` | `modbus-tcp` | PLC4X grammar, `address_convention = "plc4x"` declared in every file | interoperability-tested: the `interop` job runs each file against pymodbus 3.8.6 with `--expect-good` |
| `synthetic/sim/` | `sim` | `STATE/<name>:<type>`, no convention | simulator-tested: the `examples` job runs each file against `sim://bench` with `--expect-good` |
| `documented/modbus-tcp/` | `modbus-tcp` | PLC4X grammar, `address_convention = "zero-based"` | documented map, replayed: `modbus_device_replays` reads it against the first-party responder serving the vendor's map; not hardware-verified |
| `documented/modbus-rtu/` | `modbus-rtu` (planned, S59) | PLC4X grammar, `address_convention = "zero-based"` | documented map, replayed over TCP framing at the PDU level; not hardware-verified |

A profile belongs to one scheme (decision D17), so each synthetic device has two files with the
same file name, one per directory. A documented device has one file, under its own scheme:
neither device has a twin, because the simulator cannot stand in for a vendor's map. A sim
twin has the same measurement names and units as its Modbus file, and its id is the Modbus id with the suffix `_sim`. The simulator has no byte
or word order and no address convention, so its files declare none.

Both variants are exercised by `crates/stonecrab/examples/profile_read.rs`:

```sh
# the simulator, no network (run from the repository root)
STONECRAB_PROFILE=profiles/synthetic/sim/pressure_flow_level.toml \
  cargo run -p stonecrab --example profile_read --features full -- --expect-good

# the pymodbus container started by interop/pymodbus/interop.sh
STONECRAB_URL='modbus-tcp://127.0.0.1:5020?address-convention=plc4x' \
  STONECRAB_PROFILE=profiles/synthetic/modbus-tcp/pressure_flow_level.toml \
  cargo run -p stonecrab --example profile_read --features full -- --expect-good
```

The example's simulated device `bench` holds the values of the register map, so a sim file
and its Modbus twin yield the same measurements. A status field names the address of a
status bit; `quality_map` lists the status fields whose set bit (after `invert`) makes the
measurement `Bad` (`bad_when`) or `Uncertain` (`uncertain_when`). A value outside `range` is
`Uncertain`. Against the map every measurement is `Good`.

## Files
In the synthetic files, register numbers are the logical addresses of the PLC4X convention
(the wire address is one less); the "Row" column gives the `docs/parity/modbus.md` row of the
value read. The documented files say their own numbering. Byte and word order are only
declared for types wider than 16 bits.

### synthetic/modbus-tcp/temperature_transmitter.toml
Id `synthetic_temperature_transmitter`. Provenance: `MB-INTEROP-HR-07`, `MB-INTEROP-LE`,
`MB-INTEROP-COIL`. Sim twin: `synthetic/sim/temperature_transmitter.toml`
(`synthetic_temperature_transmitter_sim`, addresses `STATE/temperature:f32` and
`STATE/coil_2:bool`).

| Measurement | Address | Type | Byte, word order | Scaling | Unit | Range | Quality mapping | Row |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `process_temperature` | `holding-register:26:REAL` | REAL 3.141593 | big endian, high word first | clamp -50 to 150 | `Cel` | -50 to 150 | `open_circuit` (`coil:2`, false) is `Bad` | `MB-INTEROP-HR-07`, `MB-INTEROP-COIL` |
| `process_temperature_le` | `holding-register:300:REAL` | REAL 3.141593 | little endian, low word first | none | `Cel` | -50 to 150 | `open_circuit` (`coil:2`, false) is `Bad` | `MB-INTEROP-LE`, `MB-INTEROP-COIL` |

Both keep the unscaled `REAL` (`keep_raw`); the first also sets `min_interval_ms` and
`stale_after_ms` to show the timing keys.

### synthetic/modbus-tcp/pressure_flow_level.toml
Id `synthetic_pressure_flow_level`. Provenance: `MB-INTEROP-HR-02`, `MB-INTEROP-HR-04`,
`MB-INTEROP-HR-05`, `MB-INTEROP-COIL`. Sim twin: `synthetic/sim/pressure_flow_level.toml`
(`synthetic_pressure_flow_level_sim`, tags `pressure`, `flow`, `flow_status`, `level`,
`coil_1`, `coil_5`, `coil_7`).

| Measurement | Address | Type | Byte, word order | Scaling | Unit | Range | Quality mapping | Row |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `line_pressure` | `holding-register:12:INT` | INT -2424 | not declared (16 bit) | multiply 0.1 | `kPa` | -500 to 1000 | `sensor_fault` (`coil:5`, false) is `Bad` | `MB-INTEROP-HR-04`, `MB-INTEROP-COIL` |
| `flow_rate` | `holding-register:16:UDINT` | UDINT 4242442424 | big endian, high word first | none | `L/min` | 0 to 5000000000 | `cell_fault` (bit 0 of `holding-register:3:WORD` = 0xA5B8, clear) is `Bad`; `low_flow` (`coil:7`, false) is `Uncertain` | `MB-INTEROP-HR-05`, `MB-INTEROP-HR-02`, `MB-INTEROP-COIL` |
| `tank_level` | `holding-register:13:UINT` | UINT 42424 | not declared (16 bit) | none | `mm` | 0 to 50000 | `probe_lost` (`coil:1`, true, inverted) is `Bad` | `MB-INTEROP-HR-04`, `MB-INTEROP-COIL` |

The `flow_rate` and `tank_level` ranges are real subsets of the `UDINT` and `UINT`
domains, so the range check can fire for them. `line_pressure` keeps the unscaled `INT`.
A fractional `multiply` computes in `f64`, so its value is an `F64`; the other two keep
their integer types.

### synthetic/modbus-tcp/discrete_state.toml
Id `synthetic_discrete_state`. Provenance: `MB-INTEROP-COIL`, `MB-INTEROP-DI`. Sim twin:
`synthetic/sim/discrete_state.toml` (`synthetic_discrete_state_sim`, tags `coil_1` to
`coil_8` and `discrete_input_1` to `discrete_input_8`).

Eight boolean channels read coils 1 to 8 (true, false, true, true, false, true, false,
false). Each has a status field `channel_fault` on the discrete input of the same number
(false, true, false, true, true, false, true, true), mapped to `Bad`. Channels 1, 3 and 6
read the input as an active-high fault flag and do not invert it; channels 2, 4, 5, 7 and 8
read it as an active-high healthy flag and set `invert`, so a set input is no fault. All
eight are `Good` against the map.

| Measurement | Address | Type | Byte, word order | Scaling | Unit | Range | Quality mapping | Row |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| `channel_1` | `coil:1` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:1`) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_2` | `coil:2` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:2`, inverted) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_3` | `coil:3` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:3`) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_4` | `coil:4` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:4`, inverted) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_5` | `coil:5` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:5`, inverted) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_6` | `coil:6` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:6`) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_7` | `coil:7` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:7`, inverted) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |
| `channel_8` | `coil:8` | BOOL | not applicable | none | `1` | none | `channel_fault` (`discrete-input:8`, inverted) is `Bad` | `MB-INTEROP-COIL`, `MB-INTEROP-DI` |

### synthetic/sim/temperature_transmitter.toml
Id `synthetic_temperature_transmitter_sim`; the twin of the Modbus file of the same name,
with the same measurement names, units, ranges and quality mapping. Both measurements read
the simulator tag `STATE/temperature:f32` (type `f32`); the `open_circuit` status field
reads `STATE/coil_2:bool`.

### synthetic/sim/pressure_flow_level.toml
Id `synthetic_pressure_flow_level_sim`; same names, units and ranges as its twin.

| Measurement | Address | Type |
| --- | --- | --- |
| `line_pressure` | `STATE/pressure:i16` | `i16` |
| `flow_rate` | `STATE/flow:u32` | `u32` |
| `tank_level` | `STATE/level:u16` | `u16` |

The status fields read `STATE/flow_status:u16` (bit 0) and `STATE/coil_1:bool`,
`STATE/coil_5:bool` and `STATE/coil_7:bool`.

### synthetic/sim/discrete_state.toml
Id `synthetic_discrete_state_sim`; `channel_1` to `channel_8` read `STATE/coil_1:bool` to
`STATE/coil_8:bool` (type `bool`), and each `channel_fault` status field reads
`STATE/discrete_input_<n>:bool` with the same `invert` setting as the twin.

### documented/modbus-tcp/stride_sio_mb04thms.toml
Id `stride_sio_mb04thms`, label documented map, not hardware-verified. Fixture
`fixtures/modbus/devices/sio-mb04thms.toml`; user guide 1st Edition, Rev. C, sha256
`0800b63b890638626cc359e6b539d3fe14e315c1749a47f28063fd5ecbc53854`. Provenance row
`MB-DEVICE-SIO-BREAK`. Four temperatures; the numbers are the guide's (40041 is
`holding-register:41`).

| Measurement | Address (guide number) | Type | Scaling | Unit | Quality mapping |
| --- | --- | --- | --- | --- | --- |
| `channel_0_temperature` | `holding-register:41:INT` (40041) | INT, 16 bit signed | divide 10 (one implied decimal) | `Cel` | `break` (bit 0 of 40050), `fahrenheit` (bit 0 of 41218) and `kelvin` (bit 1 of 41218) are `Bad` |
| `channel_1_temperature` | `holding-register:42:INT` (40042) | INT | divide 10 | `Cel` | `break` (bit 2 of 40050), `fahrenheit` and `kelvin` (bits 0 and 1 of 41219) are `Bad` |
| `channel_2_temperature` | `holding-register:43:INT` (40043) | INT | divide 10 | `Cel` | `break` (bit 4 of 40050), `fahrenheit` and `kelvin` (41220) are `Bad` |
| `channel_3_temperature` | `holding-register:44:INT` (40044) | INT | divide 10 | `Cel` | `break` (bit 6 of 40050), `fahrenheit` and `kelvin` (41221) are `Bad` |

The break bits are 0, 2, 4 and 6 of the Break Status register; the odd bits are unused. The
degree-units register is 0 for Celsius, 1 for Fahrenheit and 2 for Kelvin, so a reading in
another unit is `Bad` instead of passing as Celsius. All values keep the unscaled `INT`
(`keep_raw`). The guide states no measuring range, so there is none. The profile assumes
each channel keeps the default thermocouple input type: a channel set to 250 mV stores two
implied decimals and a disabled channel has no reading, and schema version 1 cannot guard
either (HW1 open item 13).

### documented/modbus-rtu/solo_slm1_c.toml
Id `solo_slm1_c`, label documented map, not hardware-verified. Fixture
`fixtures/modbus/devices/slm1-c.toml`; user manual 1st Edition 07/2023, sha256
`830b99162dfbab23ed87c1778428747e65ce658dbce48e76899d4b948295d7cc`. Provenance row
`MBS-DEVICE-SLM1C-SENTINEL`. The scheme `modbus-rtu` is the one planned for S59, so
`SensorProfile::tags` refuses the file until a serial driver exists; the replay reads it
through the TCP driver by changing the scheme in memory.

| Measurement | Address (manual number) | Type | Scaling | Unit | Range | Quality mapping |
| --- | --- | --- | --- | --- | --- | --- |
| `process_value` | `holding-register:4096:INT` (1000H) | INT | divide 10 (unit 0.1 degree) | `Cel` | -200 to 1300 (thermocouple K) | `celsius` (bit 2 of 102AH, inverted: Bad when the display is not in degrees C) |

Sentinel handling: 8002H, 8003H, 8004H, 8006H and 8007H are error codes the register reads
as instead of a temperature (8005H is not listed). As signed integers they are about
-3276 degrees after the divide, outside the range, so the measurement is `Uncertain` with
the code in `raw`, never `Good`. Schema version 1 cannot match a status field against a
value, so it cannot make the code `Bad`; whether a negative PV is a signed 16 bit integer is
not stated and is an item for HW2. The unit is 0.1 degree for temperature inputs only (a
linear input reads in 1 EU), so linear inputs are out of scope for this profile.

## Connection options
The connection comes from `STONECRAB_URL`, not from the profile. The `[connection.options]`
table of a Modbus file (`default-unit-identifier = "1"`) records the options the register
map assumes and is informational: `profile_read` does not apply it, and only checks the
profile's `address_convention` against the URL.

## Checks
`crates/stonecrab/tests/sensors_profiles.rs` loads and validates every file under
`profiles/`, checks the declared conventions, checks that each synthetic Modbus file has a
sim twin, that each documented file carries its label and names a fixture that exists, and
that this README lists every file. The `examples` and `interop` CI jobs read every file under
`synthetic/` through `profile_read --expect-good`.
`crates/stonecrab/tests/modbus_device_replays.rs` validates each documented file against the
Modbus driver and reads it against the replay of its fixture.
