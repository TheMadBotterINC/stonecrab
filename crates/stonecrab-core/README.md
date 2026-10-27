# Stonecrab Core
Runtime-independent foundation for Stonecrab. This crate has no executor or network dependency.
Default features are empty. Public contracts are experimental; protocol behavior is not implemented.

See the [foundation parity record](https://github.com/themadbotter/stonecrab/blob/main/docs/parity/core.md).

Build-tested contracts: exact-width scalar values, stable validated tag identifiers,
capabilities, typed errors with diagnostic context, monotonic deadlines and redacted
secret formatting. No simulator, interoperability or hardware verification is claimed.
Resource budgets are finite, positive and immutable after builder validation.

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
