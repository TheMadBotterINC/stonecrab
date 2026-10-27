# Stonecrab
Public facade for the independent native Rust industrial protocol library.
This development version currently exposes only the runtime-independent foundation.
No hardware access is implemented. The only drivers are the in-process simulator behind the
optional `sim` feature and the read-only Modbus TCP driver behind the optional `modbus`
feature.

Default features are empty. Importing this crate performs no I/O.
See the [foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md)
for current scope. No upstream compatibility or hardware support is claimed.

## Features
- `sim` (off by default): re-exports the in-process simulator driver as `stonecrab::sim`,
  so an application or test can register a `SimDriver` with named `SimDevice`s and connect
  to `sim://<device>` without any hardware or network. It adds `stonecrab-driver-sim` and
  `tracing`; `cargo add stonecrab` alone pulls `stonecrab-core` only.
- `modbus` (off by default): re-exports the native Modbus TCP driver as `stonecrab::modbus`
  (`ModbusTcpDriver`, `ModbusOptions`, the address types and `SystemClock`). The alpha scope
  is read-only (function codes 01 to 04). It adds `stonecrab-modbus` and, through it,
  `stonecrab-transport` with its Tokio-based TCP transport, so connections must run inside a
  Tokio runtime with the time driver enabled. Parsing a connection string or an address opens
  no socket. See the `stonecrab-modbus` README for the connection-string grammar.

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
when the connection or the read fails. The repository's CI builds it but does not run it until
the interoperability job exists.

## Quick start with the simulator
The `sim_read` example builds a registry with a simulated device `bench`, connects with
`registry.connect("sim://bench")`, reads a temperature, a counter and a flag within a
500 ms deadline and logs each result through `tracing` (it installs its own
`tracing-subscriber` formatter; the library never does):

```sh
cargo run -p stonecrab --example sim_read --features sim
```

```text
 INFO connect{scheme="sim" endpoint=bench op="connect" request_id=0}: stonecrab_driver_sim::connection: sim connection established
 INFO sim_read: name=temperature value=F32(21.5) outcome="ok" sampled_at=5
 INFO sim_read: name=counter value=U32(100) outcome="ok" sampled_at=5
 INFO sim_read: name=running value=Bool(true) outcome="ok" sampled_at=6
 INFO close{scheme="sim" endpoint=bench op="close" request_id=2}: stonecrab_driver_sim::connection: sim connection closed
```

`sampled_at` is microseconds since the example started. The example exits with status 0
on success and 1, after logging an error event, on any failure.
The `sampled_at` values vary from run to run.

Build-tested functionality includes scalar values, identifiers, capabilities, errors,
deadlines and redacted secrets.
It also exposes immutable tags, bounded read requests, correlated read results,
original protocol diagnostics and explicit unknown write outcomes. These are pure
data contracts, not implemented device-read or device-write operations.

```rust
use stonecrab::{PlcValue, TagId};

let tag = TagId::new("boiler_temp")?;
let value = PlcValue::I16(-125);
assert_eq!(tag.as_str(), "boiler_temp");
assert_eq!(value.as_i64(), Some(-125));
assert_eq!(value.as_u64(), None);
# Ok::<(), stonecrab::PlcError>(())
```
