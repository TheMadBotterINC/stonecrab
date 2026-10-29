# Stonecrab Core
Runtime-independent foundation for Stonecrab: the items at the root of the `stonecrab`
crate. They have no executor or network dependency and are built whatever features are
on, so `stonecrab` with `default-features = false` is exactly this foundation plus the
`codec` module. Public contracts are experimental (alpha); the core holds the driver
boundary and data contracts, and protocol behavior lives in the driver modules
(`stonecrab::sim`, `stonecrab::modbus`).

See the [foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md).

Contracts: exact-width scalar values, calendar values without a time zone (ADR-0021),
lists of values (ADR-0022),
stable validated tag identifiers, capabilities,
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
| Connection cache: `ConnectionCache`, `LeasedConnection` (ADR-0030) | simulator-tested (the simulator and Modbus TCP over the fake transport, on the fake clock) |
| Browse and discover | not supported |
| Calendar values: `PlcSignedDuration`, `PlcDate`, `PlcTimeOfDay`, `PlcDateTime` (ADR-0021) | build-tested (unit and property tests); no driver decodes them yet |
| Lists: `PlcValue::List`, `PlcValueType::List` (ADR-0022) | simulator-tested (unit tests; the Modbus driver reads and writes them through array selections over the fake transport) |
| Byte and raw values (`RawValue`) | not supported |
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

## Connection cache
`ConnectionCache` is the first PLC4X tool in Stonecrab (ADR-0030; upstream's
`PlcConnectionCache`). It opens one connection per connection string through a
`DriverRegistry` and lends it to one holder at a time as a `LeasedConnection`, which is
itself a `Connection`: calls go through to the cached connection, and dropping the lease
gives the connection back instead of closing it. Other callers for the same string wait in
arrival order, at most `ResourceLimits::max_pending_requests` of them, the holder included.

Every bound is measured on the injected `Clock` and applied when the cache or a lease is
called; the cache spawns nothing. A lease older than `max_lease_time` (default one minute)
fails every later call with `Timeout`, and a waiter takes the connection over. A `get` gives
up after `max_wait_time` (30 s) in the queue, closes connections nobody leased for
`max_idle_time` (five minutes), pings a connection idle for `idle_ping_threshold` (30 s)
before lending it, bounded by `ping_timeout` (5 s), and replaces one whose ping failed or
whose earlier lease saw `Disconnected` or `Timeout`. A connection without ping
(`Capabilities.ping` false, such as the simulator's) is lent without one. Closes are bounded
by `close_timeout` (5 s). `close` and `remove` force-close; `len`, `active_leases` and
`connection_strings` report what the cache holds. Subscriptions made through a lease are
not restored on a replacement connection (deferred, `TOOL-CC-RESTORE`).

```rust
# #[cfg(all(feature = "sim", feature = "testkit"))]
# fn main() -> Result<(), stonecrab::PlcError> {
use std::sync::Arc;
use std::time::Duration;
use stonecrab::sim::{SimDevice, SimDriver};
use stonecrab::testkit::{FakeClock, block_on_with, poll_once};
use stonecrab::{ConnectionCache, DriverRegistry, ErrorKind};

let clock = FakeClock::default();
let mut registry = DriverRegistry::new();
registry.register(Arc::new(
    SimDriver::new()
        .with_clock(Arc::new(clock.clone()))
        .with_device("bench", SimDevice::new())?,
))?;
let cache = ConnectionCache::builder(Arc::new(registry), Arc::new(clock.clone()))
    .max_lease_time(Duration::from_secs(10))
    .build()?;

let lease = block_on_with(&clock, cache.get("sim://bench"))?;
let mut waiter = cache.get("sim://bench");
assert!(poll_once(&mut waiter).is_pending(), "one lease at a time");
drop(lease);
assert!(poll_once(&mut waiter).is_ready(), "the next caller gets the same connection");
assert_eq!(cache.len(), 1);
block_on_with(&clock, cache.close())?;
let refused = block_on_with(&clock, cache.get("sim://bench")).err().map(|error| error.kind());
assert_eq!(refused, Some(ErrorKind::Disconnected));
# Ok(())
# }
# #[cfg(not(all(feature = "sim", feature = "testkit")))]
# fn main() {}
```
