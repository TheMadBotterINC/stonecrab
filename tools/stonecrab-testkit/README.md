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

`FakeClock` implements `stonecrab_core::Clock` for tests and is advanced by hand, so
nothing waits on real time. `advance`, `set` (never backwards; an earlier instant is
ignored and reported as `false`) and `advance_to_next_sleeper` move it, and each sleep
future from `sleep_until` is woken exactly once, in deadline order. Clones share one
timeline and `Arc<FakeClock>` works as `Arc<dyn Clock>`.

`block_on` drives a future with a no-op waker under a poll budget, `block_on_with` also
advances a `FakeClock` to the next sleeper whenever the future pends, and `poll_once`
(or `poll_once_with` and `CountingWaker`, which counts wake calls) polls exactly once. A
future that can never complete, such as one waiting on a token nobody cancels, panics
with a message instead of hanging the test.

```rust
use std::time::Duration;
use stonecrab_core::Clock;
use stonecrab_testkit::{FakeClock, block_on_with, poll_once};

let clock = FakeClock::default();
let deadline = clock.now() + Duration::from_secs(30);

let mut sleep = clock.sleep_until(deadline);
assert!(poll_once(&mut sleep).is_pending());
drop(sleep);

block_on_with(&clock, clock.sleep_until(deadline))?;
assert_eq!(clock.now(), deadline);
# Ok::<(), stonecrab_core::PlcError>(())
```
