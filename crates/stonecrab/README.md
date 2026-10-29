# Stonecrab
An independent native Rust industrial protocol library, built for sensor acquisition
first. **Alpha**: pre-release versions with experimental APIs. This is one crate, with one
version: the runtime-independent core contracts are at the crate root, and the codec, the
transport, the in-process simulator, the Modbus TCP and Modbus RTU drivers, declarative
sensor profiles and the test kit are modules, switched on by Cargo features. No hardware access
is implemented and no physical device has been tried.

```sh
cargo add stonecrab        # the simulator, Modbus TCP and the sensor profiles
```

Importing this crate performs no I/O. See the
[compatibility matrix](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/COMPATIBILITY.md)
for what is supported and at which level, and the
[foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md)
for the contracts. Upstream compatibility is claimed only where a parity row says so.

## Features and modules
| Feature | Module | Pulls in | Default |
| --- | --- | --- | --- |
| none | the core at the crate root, and `stonecrab::codec` | `thiserror`, `bytes` | always built |
| `sim` | `stonecrab::sim`, the in-process simulator | `tracing` | yes |
| `modbus` | `stonecrab::modbus`, the Modbus TCP and Modbus RTU drivers (reads and writes) | `transport`, `serial`, `tracing` | yes |
| `sensors` | `stonecrab::sensors`, sensor profiles and the TOML loader | `serde`, `toml` | yes |
| `full` | the same set as the defaults, spelled out; the `profile_read` example requires it | `sim`, `modbus`, `sensors` | no |
| `transport` | `stonecrab::transport`, deadline-bounded transports and the Tokio TCP transport | `tokio` | no (on through `modbus` and `testkit`) |
| `serial` | the serial transport in `stonecrab::transport` (`SerialFactory`, `SerialTransport`) | `transport`, `serial2` | no (on through `modbus`) |
| `boards` | `stonecrab::boards`, direct board adapters: the MAX31855 thermocouple converter over an `SpiBus` | `sensors` | no |
| `adapters-linux` | `LinuxSpiDev`, a `/dev/spidev` node read on a worker thread | `boards`, `transport` | no |
| `testkit` | `stonecrab::testkit`, fakes, scripted peers, golden-vector loader and the driver contract suite | `transport`, `serde`, `serde_json`, `toml`, Tokio `macros` | no |

`default-features = false` is the Tokio-free core: the crate root and the codec, which pull
`thiserror` and `bytes` only. Tokio enters the crate through the `transport` feature and
no other, so connections made by `modbus` must run inside a Tokio runtime with the time
driver enabled. Parsing a connection string or an address opens no socket. The Modbus
connection-string and address grammars are in the `stonecrab::modbus` module
documentation.

The foundation re-exported at the root includes scalar values, identifiers,
capabilities, errors, deadlines, redacted secrets, immutable tags, bounded read requests,
correlated read results, original protocol diagnostics and explicit unknown write
outcomes, and the `Driver`, `Connection` and `DriverRegistry` boundary. Those are pure data
contracts and traits; a driver (the simulator, or the Modbus driver) performs the
operations.

| Capability | Support level |
| --- | --- |
| Core contracts through `DriverRegistry`, `Driver` and `Connection` (always built) | simulator-tested |
| `sim`: in-process simulator, read and write, scripted failures | simulator-tested |
| `modbus`: Modbus TCP connect, close and reads of coils, discrete inputs, holding registers and input registers (FC01 to FC04) | interoperability-tested |
| `modbus`: reconnect, retry, cancellation, timeout, late-reply discard | simulator-tested |
| `modbus`: write of coils and holding registers (FC15, FC06, FC16), refused unless the connection string says `access=read-write` | interoperability-tested |
| `modbus`: Modbus RTU (`modbus-rtu`) reads, writes, ping and polling over a serial port or a gateway, with CRC errors, the silence before a request, late-reply drain and reopen | simulator-tested |
| Subscribe by polling (`sim` and `modbus`): cyclic and change-of-state, event refused; on a clock that never sleeps (the simulator's default) change-of-state is refused and the interval paces nothing | simulator-tested |
| Browse and discover | not supported |
| `sensors`: synthetic profiles against the simulator | simulator-tested |
| `sensors`: synthetic profiles against a Modbus TCP server | interoperability-tested |
| `boards`: MAX31855 over SPI (decode, faults, measurements) and the Linux `/dev/spidev` backend, over scripted buses and in-memory readers | simulator-tested |
| Hardware verification | planned |

`interoperability-tested` means the `interop` CI job ran the code against pymodbus 3.8.6.
The levels of each module are in its own documentation page (the component documents in
`docs/components/` of this crate); the table above is the crate's view of the same
evidence.

## The test kit
`stonecrab::testkit` (feature `testkit`, off by default, not a runtime feature) ships the
driver contract suite, `ContractSuite`, so that the author of a new `Driver` can run the
same cases against it, and the deterministic fakes that make such tests repeatable: a fake
clock, a fake transport, scripted peers and a golden-vector loader. It is a module of this
crate, and not a separate test crate, because a test crate that depends on `stonecrab`
while `stonecrab` uses it in its own tests would be a dev-dependency cycle (ADR-0018).
Its own tests and doctests read files of the repository (`testdata/`, `fixtures/`), which
the packaged archive does not contain, so the full suite runs from a checkout with
`cargo test --all-features`. A plain `cargo test` runs the reduced suite without it.

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

```rust
use stonecrab::{PlcValue, TagId};

let tag = TagId::new("boiler_temp")?;
let value = PlcValue::I16(-125);
assert_eq!(tag.as_str(), "boiler_temp");
assert_eq!(value.as_i64(), Some(-125));
assert_eq!(value.as_u64(), None);
# Ok::<(), stonecrab::PlcError>(())
```
