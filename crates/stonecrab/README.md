# Stonecrab
Public facade for the independent native Rust industrial protocol library. **Alpha**:
pre-release versions with experimental APIs. This version re-exports the
runtime-independent foundation (`stonecrab-core`) and, behind optional features, the
in-process simulator (`sim`), the read-only Modbus TCP driver (`modbus`) and declarative
sensor profiles (`sensors`). No hardware access is implemented and no physical device has
been tried.

Default features are empty. Importing this crate performs no I/O.
See the [compatibility matrix](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/COMPATIBILITY.md)
for what is supported and at which level, and the
[foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md)
for the contracts. Upstream compatibility is claimed only where a parity row says so.

| Capability | Support level |
| --- | --- |
| Core contracts through `DriverRegistry`, `Driver` and `Connection` (feature-free) | simulator-tested |
| `sim` feature: in-process simulator, read and write, scripted failures | simulator-tested |
| `modbus` feature: Modbus TCP connect, close and reads of coils, discrete inputs, holding registers and input registers (FC01 to FC04) | interoperability-tested |
| `modbus` feature: reconnect, retry, cancellation, timeout, late-reply discard | simulator-tested |
| `modbus` feature: write | not supported |
| Subscribe, browse and discover | not supported |
| `sensors` feature: synthetic profiles against the simulator | simulator-tested |
| `sensors` feature: synthetic profiles against a Modbus TCP server | interoperability-tested |
| Hardware verification | planned |

`interoperability-tested` means the `interop` CI job ran the code against pymodbus 3.8.6.
The levels of each crate are in its own README; the table above is the facade's view of
the same evidence.

## Features
- `sim` (off by default): re-exports the in-process simulator driver as `stonecrab::sim`,
  so an application or test can register a `SimDriver` with named `SimDevice`s and connect
  to `sim://<device>` without any hardware or network. It adds `stonecrab-driver-sim` and
  `tracing`; with no features the crate pulls `stonecrab-core` only. Until the alpha is
  published, use the features from a checkout (a path dependency).
- `modbus` (off by default): re-exports the native Modbus TCP driver as `stonecrab::modbus`
  (`ModbusTcpDriver`, `ModbusOptions`, the address types and `SystemClock`). The alpha scope
  is read-only (function codes 01 to 04). It adds `stonecrab-modbus` and, through it,
  `stonecrab-transport` with its Tokio-based TCP transport, so connections must run inside a
  Tokio runtime with the time driver enabled. Parsing a connection string or an address opens
  no socket. See the `stonecrab-modbus` README for the connection-string grammar.
- `sensors` (off by default): re-exports the declarative sensor profile crate as
  `stonecrab::sensors` (`SensorProfile`, `Measurement`, `Quality`, `Unit` and the schema
  types). It also turns on `stonecrab-sensors/toml`, the TOML loader, because
  `SensorProfile::from_toml` is how an application reads a profile file and the
  `profile_read` example needs it; `serde` and `toml` come in through that crate and
  nothing else is added. The profiles in the repository's `profiles/` directory are
  synthetic, describe no real device and are not part of this crate.
- `full` (off by default): `sim`, `modbus` and `sensors` together. The `profile_read`
  example requires it.

## Quick start with a Modbus TCP server
The `modbus_tcp_read` example registers `ModbusTcpDriver`, parses the connection string
first so that it can take the configuration's address options before `connect` consumes it,
reads one tag within 2 seconds and logs name, value, outcome and `sampled_at` through
`tracing`. `STONECRAB_URL` is the connection string (default `modbus-tcp://127.0.0.1:5020`;
add `?address-convention=plc4x` to use bare addresses such as `40001`) and
`STONECRAB_ADDRESS` the address (default `holding-register:13:UINT`):

```sh
STONECRAB_URL=modbus-tcp://192.0.2.10:502 \
  cargo run -p stonecrab --example modbus_tcp_read --features modbus
```

It needs a reachable Modbus TCP server and exits with status 1, after logging an error event,
when the connection or the read fails. The repository's `interop` CI job runs it against
pymodbus 3.8.6 (`mise run interop` runs the job locally; it needs Docker).

## Quick start with the simulator
The `sim_read` example builds a registry with a simulated device `bench`, connects with
`registry.connect("sim://bench")`, reads a temperature, a counter and a flag within a
500 ms deadline and logs each result through `tracing` (it installs its own
`tracing-subscriber` formatter; the library never does):

```sh
cargo run -p stonecrab --example sim_read --features sim
```

```text
 INFO connect{scheme="sim" endpoint=bench op="connect" request_id=0}: stonecrab::sim::connection: sim connection established
 INFO sim_read: name=temperature value=F32(21.5) outcome="ok" sampled_at=5
 INFO sim_read: name=counter value=U32(100) outcome="ok" sampled_at=5
 INFO sim_read: name=running value=Bool(true) outcome="ok" sampled_at=6
 INFO close{scheme="sim" endpoint=bench op="close" request_id=2}: stonecrab::sim::connection: sim connection closed
```

`sampled_at` is microseconds since the example started. The example exits with status 0
on success and 1, after logging an error event, on any failure.
The `sampled_at` values vary from run to run.

## Quick start with a sensor profile
The `profile_read` example loads a synthetic profile from the repository's `profiles/`
directory, validates it, builds its tags through the driver the connection string names,
reads them within 2 seconds, turns the response into measurements and logs name, value,
unit, quality, origin and (when the profile keeps it) the raw value through `tracing`.
`STONECRAB_URL` (default `sim://bench`) and `STONECRAB_PROFILE` (default
`profiles/synthetic/sim/temperature_transmitter.toml`, read from the current directory, so
run it from a checkout of the repository) select the connection and the profile. It needs
the `full` feature and, with `--expect-good`, exits with status 1 unless every measurement
is of quality `Good`:

```sh
cargo run -p stonecrab --example profile_read --features full -- --expect-good
```

```text
 INFO profile_read: name=process_temperature value=F64(3.1415929794311523) unit=Cel quality=good origin=Synthetic sampled_at=4 raw=F32(3.141593)
 INFO profile_read: name=process_temperature_le value=F32(3.141593) unit=Cel quality=good origin=Synthetic sampled_at=6 raw=F32(3.141593)
 INFO profile_read: read complete profile=synthetic_temperature_transmitter_sim measurements=2 split_into=1
```

The simulated device `bench` holds the values of the Modbus interoperability register map,
so a profile under `profiles/synthetic/modbus-tcp/` read from a pymodbus server (see
`interop/pymodbus/README.md`) yields the same measurements. The `sampled_at` values vary
from run to run, and the connect and close span events are omitted above.

The foundation re-exported here includes scalar values, identifiers, capabilities, errors,
deadlines, redacted secrets, immutable tags, bounded read requests, correlated read
results, original protocol diagnostics and explicit unknown write outcomes. Those are pure
data contracts; a driver (the simulator, or the Modbus driver for reads) performs the
operations.

```rust
use stonecrab::{PlcValue, TagId};

let tag = TagId::new("boiler_temp")?;
let value = PlcValue::I16(-125);
assert_eq!(tag.as_str(), "boiler_temp");
assert_eq!(value.as_i64(), Some(-125));
assert_eq!(value.as_u64(), None);
# Ok::<(), stonecrab::PlcError>(())
```
