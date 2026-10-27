# Stonecrab
Public facade for the independent native Rust industrial protocol library.
This development version currently exposes only the runtime-independent foundation.
No protocol drivers, connections or hardware access are implemented, except the in-process
simulator behind the optional `sim` feature.

Default features are empty. Importing this crate performs no I/O.
See the [foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md)
for current scope. No upstream compatibility or hardware support is claimed.

## Features
- `sim` (off by default): re-exports the in-process simulator driver as `stonecrab::sim`,
  so an application or test can register a `SimDriver` with named `SimDevice`s and connect
  to `sim://<device>` without any hardware or network. It adds `stonecrab-driver-sim` and
  `tracing`; `cargo add stonecrab` alone pulls `stonecrab-core` only.

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
