# Stonecrab Core
Runtime-independent foundation for Stonecrab. This crate has no executor or network dependency.
Default features are empty. Public contracts are experimental; this crate holds the driver boundary and data contracts, and protocol behavior lives in the driver crates.

See the [foundation parity record](https://github.com/TheMadBotterINC/stonecrab/blob/master/docs/parity/core.md).

Build-tested contracts: exact-width scalar values, stable validated tag identifiers,
capabilities, typed errors with diagnostic context, monotonic deadlines and redacted
secret formatting. No simulator, interoperability or hardware verification is claimed.
Resource budgets are finite, positive and immutable after builder validation.
Immutable tags, bounded read requests and correlated read responses are implemented
as data contracts only. They do not acquire device data. Protocol status retains
bounded raw bytes without exposing them through Debug, and write outcomes retain
post-transmit uncertainty without implementing a write or retry operation. Bounded
write requests and correlated write responses are data contracts too; an unknown write
outcome is never collapsed into a rejection or replayed.

Read builders require explicit `ResourceLimits`. Duplicate identifiers and over-budget
requests fail before any transport exists. Response builders reject missing, duplicate
or unexpected identities and invalid sampling intervals; per-item failures remain data.
Cancellation tokens, the injectable `Clock`, `OpFuture` and connection-state observation
are primitives; the `Driver`, `Connection` and `DriverRegistry` traits define the driver
boundary, and a connection enforces the token and the default-deadline rule (ADR-0011).

```rust
use stonecrab_core::{Deadline, PlcValue, TagId};
use std::time::{Duration, Instant};

let tag = TagId::new("energy_counter")?;
let value = PlcValue::U64(u64::MAX);
let now = Instant::now();
let deadline = Deadline::after(now, Duration::from_secs(1))?;
assert_eq!(tag.as_str(), "energy_counter");
assert_eq!(value.as_u64(), Some(u64::MAX));
assert_eq!(value.as_f64(), None);
assert_eq!(deadline.remaining(now), Some(Duration::from_secs(1)));
# Ok::<(), stonecrab_core::PlcError>(())
```
