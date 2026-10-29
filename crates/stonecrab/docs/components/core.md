# Stonecrab Core
Runtime-independent foundation for Stonecrab: the items at the root of the `stonecrab`
crate. They have no executor or network dependency and are built whatever features are
on, so `stonecrab` with `default-features = false` is exactly this foundation plus the
`codec` module. Public contracts are experimental (alpha); the core holds the driver
boundary and data contracts, and protocol behavior lives in the driver modules
(`stonecrab::sim`, `stonecrab::modbus`).

See the [foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md).

Contracts: exact-width scalar values, stable validated tag identifiers, capabilities,
typed errors with diagnostic context, monotonic deadlines and redacted secret formatting.
Resource budgets are finite, positive and immutable after builder validation.
Immutable tags, bounded read requests and correlated read responses are data contracts:
core does not acquire device data, a driver does. Protocol status retains bounded raw
bytes without exposing them through Debug, and write outcomes retain post-transmit
uncertainty. Bounded write requests and correlated write responses are data contracts
too; an unknown write outcome is never collapsed into a rejection or replayed.

| Capability | Support level |
| --- | --- |
| Driver boundary: `Driver`, `Connection`, `DriverRegistry`, configuration, address options, cancellation, state watch, diagnostics | simulator-tested |
| Values, tags, read and write batches, errors and correlation | simulator-tested |
| The same boundary on a real wire, through `stonecrab::modbus` (reads) | interoperability-tested |
| Subscription types and their bounded event queue: `SubscribeRequest`, `Subscription`, `QueuePolicy`, `LossCounters` | simulator-tested (unit and property tests; the simulator and `modbus` poll through them since S55) |
| Polling subscriptions: the shared producer behind `Connection::subscribe` of the simulator and `modbus` | simulator-tested |
| Browse and discover | not supported |
| Calendar, byte, list and raw values (`RawValue`) | not supported |
| Hardware verification | planned |

`simulator-tested` means the driver contract suite of the `testkit` feature runs against
the `sim` driver and the contracts have unit, property and documentation tests;
`interoperability-tested` means the Modbus driver, built on these contracts, reads from
an independent server (pymodbus 3.8.6) in the `interop` job. The core itself implements
no driver. `Timeouts.idle` is enforced only by a supervised connection guard (S57,
ADR-0014 addendum): it closes a link no request has used for that long, and the next
request reopens it; the simulator and an unsupervised guard ignore it.

Read builders require explicit `ResourceLimits`. Duplicate identifiers and over-budget
requests fail before any transport exists. Response builders reject missing, duplicate
or unexpected identities and invalid sampling intervals; per-item failures remain data.
Cancellation tokens, the injectable `Clock`, `OpFuture` and connection-state observation
are primitives; the `Driver`, `Connection` and `DriverRegistry` traits define the driver
boundary, and a connection enforces the token and the default-deadline rule (ADR-0011).
`Connection::subscribe` is a provided method that answers `Unsupported` until a driver
overrides it (ADR-0020); the simulator and the Modbus driver override it with one polling
producer (S55, ADR-0020 addendum). A subscription request names its kind and its
`QueuePolicy`; the `Subscription` it yields discloses whether it polls and at which
interval, holds at most `ResourceLimits::max_queued_events` events, and counts every event
its policy gives up in `LossCounters`: backpressure loses none, coalescing keeps the latest
value per tag, and dropping discards the oldest. A polling subscription owns its producer
future and polls it while `recv` (which takes `&mut self`) waits on an empty queue, with no
executor and nothing spawned; ticks it could not sample (a read still running, nobody waiting, the connection's
in-flight slot taken, the link being re-established) count in `LossCounters::skipped`,
and none is made up later.
A driver with options only it understands carries them in the configuration's
`DriverOptions` slot (ADR-0008), and `Driver::parse_address` takes the caller's
`AddressOptions` so an address convention is chosen explicitly, never inferred.

```rust
use stonecrab::{Deadline, PlcValue, TagId};
use std::time::{Duration, Instant};

let tag = TagId::new("energy_counter")?;
let value = PlcValue::U64(u64::MAX);
let now = Instant::now();
let deadline = Deadline::after(now, Duration::from_secs(1))?;
assert_eq!(tag.as_str(), "energy_counter");
assert_eq!(value.as_u64(), Some(u64::MAX));
assert_eq!(value.as_f64(), None);
assert_eq!(deadline.remaining(now), Some(Duration::from_secs(1)));
# Ok::<(), stonecrab::PlcError>(())
```
