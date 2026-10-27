# Stonecrab
Public facade for the independent native Rust industrial protocol library.
This development version currently exposes only the runtime-independent foundation.
No protocol drivers, connections or hardware access are implemented.

Default features are empty. Importing this crate performs no I/O.
See the [foundation parity record](https://github.com/themadbotter/stonecrab/blob/main/docs/parity/core.md)
for current scope. No upstream compatibility or hardware support is claimed.

Build-tested functionality includes scalar values, identifiers, capabilities, errors,
deadlines and redacted secrets.

```rust
use stonecrab::{PlcValue, TagId};

let tag = TagId::new("boiler_temp")?;
let value = PlcValue::I16(-125);
assert_eq!(tag.as_str(), "boiler_temp");
assert_eq!(value.as_i64(), Some(-125));
assert_eq!(value.as_u64(), None);
# Ok::<(), stonecrab::PlcError>(())
```
