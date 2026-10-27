# Stonecrab Driver Sim
In-process simulator driver for Stonecrab. It is the sanctioned test double for the
driver boundary (coding standards section 13.5): the one place a `Driver` and
`Connection` can be exercised without a device, a socket or a hand-rolled mock. It is
not a Modbus simulator.
It mirrors the shape of the Java simulated driver's addresses (`AREA/name:type`) and
nothing of any wire protocol.

Nothing is a driver yet. The `Driver` and `Connection` traits do not exist in
`stonecrab-core` (slice S11a), so this first slice (S11b-1) ships the data the simulator
will be built from: a clock, the address grammar and the device model. `SimDriver` and
`SimConnection` arrive in S11b-2. The crate depends on `stonecrab-core` and `tracing`
only, has no Tokio dependency, and its default feature set is empty.

Upstream behavior is the simulated driver of Apache PLC4X, and the simulator claims no
parity with it. Stonecrab-owned rows, divergences and planned items are recorded in the
[simulator record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/sim.md).

## Features
Default features are empty and there are no optional features. Available now:

- `RealtimeClock`: a `std`-only `Clock` whose `now()` is `Instant::now()`, whose
  `wall_now()` is `SystemTime::now()` and whose `sleep_until` resolves on its first poll
  without moving time. The simulator is not a timer: a scripted `Delay` step is a no-op
  and a `Timeout` step returns `ErrorKind::Timeout` directly in real-time use. Tests that
  need time to pass inject the testkit `FakeClock` and derive their deadlines from it.
- `SimAddress` and `SimArea`: the address grammar below.
- `SimDevice` and `SimTag`: named tags with access flags, a per-read ramp, a read counter,
  a seeded `RANDOM` source, and stored latency and item-limit settings.

| Capability | Support level |
| --- | --- |
| Address grammar and device model (types only, no driver) | planned |
| `sim` driver registration and `parse_address` | planned |
| Connect, close and connection state | planned |
| Read, including `RANDOM` and ramps | planned |
| Write, including `SINK` | planned |
| Scripted acquisition failures (delay, timeout, disconnect, device exception) | planned |
| Bounded read retry | planned |
| Common driver contract suite | planned |

## Address grammar
`SimAddress::parse` reads `STATE|RANDOM|SINK/<name>:<type>`, case-sensitive. The area
ends at the first `/` and the type begins after the last `:`, so a name may contain both.
`<name>` obeys the `TagId` rules: 1 to 256 bytes, no control characters, no leading or
trailing whitespace. `<type>` is one of the lowercase scalar names `bool`, `i8`, `i16`,
`i32`, `i64`, `u8`, `u16`, `u32`, `u64`, `f32` and `f64`. The whole text is at most 256
bytes.

| Area | Meaning |
| --- | --- |
| `STATE` | a named value, read back after it was set or written |
| `RANDOM` | a fresh value of the type on every read, from the device's seeded generator |
| `SINK` | write-only: writes are counted and discarded (upstream's `STDOUT`; print macros are denied in library code) |

Rejected with `ErrorKind::InvalidAddress`: empty input, input over 256 bytes, control
characters, a missing `/` or `:type`, an unknown area (`STDOUT` included), an unknown or
upper-case type (`INT`, `REAL`), the legacy array form `STATE/name:i16[4]`, and a name
that fails the `TagId` rules.

```rust
use stonecrab_core::{PlcValue, PlcValueType, ProtocolAddress};
use stonecrab_driver_sim::{SimAddress, SimArea, SimDevice, SimTag};

let address = SimAddress::parse("STATE/boiler:f32")?;
assert_eq!(address.area(), SimArea::State);
assert_eq!(address.value_type(), PlcValueType::F32);
assert_eq!(address.canonical(), "STATE/boiler:f32");
assert!(SimAddress::parse("STDOUT/log:i16").is_err());

let device = SimDevice::new().with_seed(7);
device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
device.ramp("count", PlcValue::U16(0), PlcValue::U16(1))?;
assert_eq!(device.serve_read("boiler")?, PlcValue::F32(71.5));
assert_eq!(device.serve_read("count")?, PlcValue::U16(0));
assert_eq!(device.serve_read("count")?, PlcValue::U16(1));
# Ok::<(), stonecrab_core::PlcError>(())
```

## Device model
`SimDevice` holds up to 4096 named `SimTag`s behind a `std::sync::Mutex` (a poisoned lock
is recovered), so one device can be shared by a connection and a test. `Debug` prints
counts, never values.

- `ramp(name, start, step)` makes a numeric tag that serves `start` first and adds `step`
  on every read, in its own type with checked arithmetic. When the next value would
  overflow, the ramp stops and keeps serving the last value; it neither wraps nor
  saturates at the type's maximum.
- `SimTag::with_read_counter()` returns the tag and an `Arc<AtomicU32>` incremented on
  every served read, so a test can prove how many reads reached the device.
- `RANDOM` values come from a seeded xorshift64* generator (`SimDevice::with_seed`);
  upstream uses `SecureRandom`, which cannot be reproduced. The seed is fixed by default.
  Floats are uniform over `[0, 1)`.
