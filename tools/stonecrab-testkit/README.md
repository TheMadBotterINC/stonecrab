# Stonecrab Testkit
Maintainer-only deterministic fixtures. Never published or used as a runtime
dependency of a published crate.

`FixtureAddress` supplies a protocol-address double for core integration tests.
It retains supplied text and an optional type hint; it does not parse or validate
a real industrial protocol or claim driver compatibility.

```rust
use std::sync::Arc;
use std::time::Instant;
use stonecrab_core::{PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits, Tag, TagId};
use stonecrab_testkit::FixtureAddress;

let id = TagId::new("counter")?;
let address = Arc::new(FixtureAddress::new("counter", None));
let request = ReadRequest::builder(ResourceLimits::builder().build()?)
    .tag(Tag::new(id.clone(), address))
    .build()?;
let now = Instant::now();
let response = ReadResponse::builder(&request, now, now)
    .item(ReadItem::new(id, Ok(PlcValue::U64(42)), now))
    .build()?;
assert_eq!(response.iter_ok().count(), 1);
# Ok::<(), stonecrab_core::PlcError>(())
```
