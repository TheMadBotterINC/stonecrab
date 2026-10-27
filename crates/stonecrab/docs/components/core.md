# Stonecrab Core
Runtime-independent foundation for Stonecrab. This crate has no executor or network dependency.
Default features are empty. Public contracts are experimental (alpha); this crate holds
the driver boundary and data contracts, and protocol behavior lives in the driver crates.

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
| The same boundary on a real wire, through `stonecrab-modbus` (reads) | interoperability-tested |
| Subscribe, browse and discover | not supported |
| Calendar, byte, list and raw values (`RawValue`) | not supported |
| Hardware verification | planned |

`simulator-tested` means the driver contract suite of the repository's unpublished testkit
runs against the `sim` driver and the contracts have unit, property and documentation
tests; `interoperability-tested` means the Modbus driver, built on these contracts, reads
from an independent server (pymodbus 3.8.6) in the `interop` job. Core itself implements
no driver. `Timeouts.idle` is recorded and not enforced at the alpha.

Read builders require explicit `ResourceLimits`. Duplicate identifiers and over-budget
requests fail before any transport exists. Response builders reject missing, duplicate
or unexpected identities and invalid sampling intervals; per-item failures remain data.
Cancellation tokens, the injectable `Clock`, `OpFuture` and connection-state observation
are primitives; the `Driver`, `Connection` and `DriverRegistry` traits define the driver
boundary, and a connection enforces the token and the default-deadline rule (ADR-0011).
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
