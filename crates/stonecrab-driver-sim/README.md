# Stonecrab Driver Sim
In-process simulator driver for Stonecrab. It is the sanctioned test double for the
driver boundary (coding standards section 13.5): the one place a `Driver` and
`Connection` can be exercised without a device, a socket or a hand-rolled mock. It is
not a Modbus simulator.
It mirrors the shape of the Java simulated driver's addresses (`AREA/name:type`) and
nothing of any wire protocol.

`SimDriver` is a `Driver` for the scheme `sim`, and `SimConnection` is the `Connection` it
opens (slice S11b-2). Register the driver with a `DriverRegistry` and connect by
connection string:

```rust
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use stonecrab_core::{Connection, DriverRegistry, OpFuture, PlcValue, ReadRequest, Tag, TagId};
use stonecrab_driver_sim::{SimAddress, SimDevice, SimDriver, SimTag};

// With the default `RealtimeClock` no operation ever waits, so one poll completes it.
fn now_or_never<T>(mut future: OpFuture<'_, T>) -> stonecrab_core::Result<T> {
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(outcome) => outcome,
        Poll::Pending => unreachable!("the real-time clock never makes an operation wait"),
    }
}

let device = SimDevice::new();
device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
let mut registry = DriverRegistry::new();
registry.register(Arc::new(SimDriver::new().with_device("bench", device)?))?;

let connection = now_or_never(registry.connect("sim://bench"))?;
let tag = Tag::new(TagId::new("boiler")?, Arc::new(SimAddress::parse("STATE/boiler:f32")?));
let request = ReadRequest::builder(connection.limits()).tag(tag).build()?;
let response = now_or_never(connection.read(request))?;
assert_eq!(response.items()[0].outcome().as_ref().ok(), Some(&PlcValue::F32(71.5)));
# Ok::<(), stonecrab_core::PlcError>(())
```

The crate depends on `stonecrab-core` and `tracing` only, has no Tokio dependency, and its
default feature set is empty. Through the facade it is `stonecrab::sim` behind the `sim`
feature; the `sim_read` example and the `sim` variants of the synthetic sensor profiles
(`profile_read`) run against it with no network, which is what the alpha's `examples` CI
job does. Tests that need time to pass inject the unpublished testkit's
`FakeClock` with `SimDriver::with_clock`.

Upstream behavior is the simulated driver of Apache PLC4X, and the simulator claims no
parity with it. Stonecrab-owned rows and divergences are recorded in the
[simulator record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/sim.md).

## Features
Default features are empty and there are no optional features.

- `SimDriver`: scheme `sim`, a set of named `SimDevice`s and a clock. `with_device(name,
  device)` registers a device; `sim://<device>?latency-ms=<n>&seed=<u64>` opens it. Unknown
  devices and unknown options are `InvalidConfig`; `address-convention` is accepted and
  ignored (the grammar has no conventions), and upstream's `simulated:<name>` form is not
  aliased. A `sim` configuration is read-write, unlike the read-only default, because a
  simulated device has nothing to protect.
- `SimConnection`: the lifecycle `Disconnected`, `Connecting`, `Connected`, `Closing`,
  `Closed` (a scripted disconnect moves a connection to `Failed(Disconnected)`); reads of
  `STATE` and `RANDOM` addresses with per-item errors, writes to `STATE` and `SINK`
  addresses; one operation in flight at a time (a second one fails with `Backpressure`,
  there is no wait queue); deadlines and cancellation honored on the connection's clock;
  whole-request read retry under the configured `RetryPolicy`; `Diagnostics` on every
  error; `tracing` spans.
- `FailureScript` and `FailureStep` (slice S12): a bounded queue of scripted faults on the
  device that connections consume one per operation. See "Failure script" below.
- `RealtimeClock`: a `std`-only `Clock` whose `now()` is `Instant::now()`, whose
  `wall_now()` is `SystemTime::now()` and whose `sleep_until` resolves on its first poll
  without moving time. The simulator is not a timer: with it every latency wait is a
  no-op. Tests that need time to pass inject the testkit `FakeClock` and derive their
  deadlines from it.
- `SimAddress` and `SimArea`: the address grammar below.
- `SimDevice` and `SimTag`: named tags with access flags, a per-read ramp, a read counter,
  a seeded `RANDOM` source, a `SINK` write counter, and the latency and item-limit
  settings the connection applies.

The device latency (or `latency-ms`) is the time every connect, read, write and close waits
on the injected clock, raced against the request's cancellation token, its deadline and
the connection being closed. A device item limit (`SimDevice::with_max_items`) is advertised
in the connection's `Capabilities` and enforced: the simulator never splits a batch.

| Capability | Support level |
| --- | --- |
| Address grammar and device model | simulator-tested |
| `sim` driver registration and `parse_address` | simulator-tested |
| Connect, close and connection state | simulator-tested |
| Read, including `RANDOM` and ramps | simulator-tested |
| Write, including `SINK` | simulator-tested |
| Subscribe, browse and discover | not supported |
| Scripted acquisition failures (delay, timeout, disconnect, device exception) | simulator-tested |
| Bounded read retry | simulator-tested |
| Common driver contract suite | simulator-tested |

The read and write rows rest on two kinds of evidence: this crate's own tests, and the
common driver contract suite of the unpublished testkit (`ContractSuite`), which
`tests/contract.rs` runs against a simulated device under `block_on_with` on a
`FakeClock`. The run is green with one named skip: "an unsupported write performs no
I/O" is skipped because the simulator supports write. The fixture wires the suite's fault
hooks to `SimDevice::push_failure` (`Disconnect`, `Timeout`, `DeviceException`,
`UnknownAfterTransmit`), its I/O probe to the device read counter plus `sink_writes`, and
its item budget to `SimDevice::with_max_items`; the suite rebuilds the parsed
configuration for the retry and read-only cases. The queued-cancellation case is not a
suite case, because the simulator has no wait queue.

## Failure script
A `SimDevice` owns a `FailureScript`, a first-in first-out queue of at most 1024
`FailureStep`s that is shared by every connection to the device. Queue steps with
`SimDevice::push_failure` or `push_failures` and read what is left with
`pending_failures`. A connect, read or write takes at most one step, at the point where it
would touch the device; anything rejected earlier (a foreign address, a batch over the
item limit, an already cancelled token, an elapsed deadline, a read-only policy, the
single in-flight slot, a connection that is not open) leaves the step queued. `close`
ignores the script. Each attempt of a retried read takes its own step.

| Step | Read | Write |
| --- | --- | --- |
| `Delay(d)` | waits `d` on the clock, then proceeds normally | the same |
| `Timeout` | the device is read, the reply is lost; waits for the deadline, then `Timeout` | the device is written, the reply is lost; waits for the deadline, then `Timeout` |
| `Disconnect` | device untouched; `Disconnected`, connection `Failed(Disconnected)` for good | the same |
| `DeviceException(status)` | device untouched; every item is a `DeviceException` item error carrying the status | every item `Rejected` with the status |
| `Malformed` | device untouched; every item is a `MalformedData` item error | the whole call fails with `MalformedData` |
| `PartialFailure(ids)` | the named items fail with status `("sim", 4)`, the rest are served | the named items are `Rejected` with it, the rest applied |
| `Backpressure` | device untouched; `Backpressure`, connection stays open, slot released | the same |
| `UnknownAfterTransmit` | behaves like `Timeout` | applied once; every item `Unknown(TimeoutAfterTransmit)`; never replayed |

Waiting steps (`Delay`, `Timeout`) race the request's cancellation token, the connection
being closed and the deadline, like the device latency. On a connect only `Delay`,
`Timeout` (fails at the connect deadline) and `Disconnect` mean anything; the other steps
are consumed with a `tracing` warning and ignored, so queue steps after the connection is
open. A request without a deadline gets a fresh `timeouts.request` for each read attempt,
which is what lets a script of `Timeout` steps exercise the retry loop; an explicit
deadline is shared by all attempts. With the default `RealtimeClock` a `Delay` is a no-op
and a `Timeout` returns at once.

```rust
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use stonecrab_core::{
    Connection, DriverRegistry, ErrorKind, OpFuture, PlcValue, ProtocolStatus, ReadRequest, Tag,
    TagId,
};
use stonecrab_driver_sim::{FailureStep, SimAddress, SimDevice, SimDriver, SimTag};

fn now_or_never<T>(mut future: OpFuture<'_, T>) -> stonecrab_core::Result<T> {
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(outcome) => outcome,
        Poll::Pending => unreachable!("the real-time clock never makes an operation wait"),
    }
}

let device = SimDevice::new();
device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
let mut registry = DriverRegistry::new();
let driver = Arc::new(SimDriver::new().with_device("bench", device)?);
registry.register(Arc::<SimDriver>::clone(&driver))?;
let connection = now_or_never(registry.connect("sim://bench"))?;

let bench = driver.device("bench").ok_or("the device is registered")?;
bench.push_failure(FailureStep::DeviceException(ProtocolStatus::new("sim", 9)))?;
let tag = Tag::new(TagId::new("boiler")?, Arc::new(SimAddress::parse("STATE/boiler:f32")?));
let request = ReadRequest::builder(connection.limits()).tag(tag).build()?;
let response = now_or_never(connection.read(request))?;
let failed = response.items()[0].outcome().as_ref().err().map(|e| e.kind());
assert_eq!(failed, Some(ErrorKind::DeviceException));
assert_eq!(bench.pending_failures(), 0);
# Ok::<(), Box<dyn std::error::Error>>(())
```

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
