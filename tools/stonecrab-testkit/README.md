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

## Network doubles (feature `net`)

The `net` feature is off by default so that core's dev-dependency on this crate stays
Tokio-free. It adds the doubles that stand in for a device, all driven by one `Script`
(`steps: Vec<Step>`, `strict: bool`; `Script::new` is lenient, `Script::strict` fails on
the first mismatch).

- `Step` instructions: `Expect` (the next bytes received must equal these), `Reply`,
  `ReplyFragmented` (separate writes), `ReplyCoalescedWithNext` (a marker: the next reply
  shares one write with the reply after it), `Delay`, `CloseAt(n)` (send n bytes of the
  next reply, then close), `Truncate(n)` (send n bytes of the next reply, stay open),
  `Corrupt { offset, bytes }`, `WrongTransactionId` (flip the first two bytes of the next
  reply), `LateReply` (deliver the next reply later) and `Respond(closure)` (answer every
  request, so one fixture can serve a whole contract suite). Each variant's rustdoc gives
  the exact rule.
- `ScriptedPeer::spawn(script)` listens on `127.0.0.1:0` and runs the script on the one
  connection it accepts. It is for bytes only: `Delay` and `LateReply` sleep for at most
  `MAX_PEER_DELAY` (200 ms). It owns the workspace's single `tokio::spawn`; `close()` joins
  the task and returns a `ScriptReport`, and dropping the peer aborts it.
- `FakeTransport` implements `stonecrab_transport::Transport` over the same script and a
  `FakeClock`. Delays and late replies sleep on the fake clock, so `block_on_with` runs
  them without waiting; a deadline that passes first is a `Timeout`. It logs every frame
  (`FakeTransportProbe::sent`) and honors the same frame and buffer bounds as the TCP
  transport.
- `FakeTransportFactory` implements `TransportFactory` from a queue of scripts, a
  scripted connect timeout (parks until the fake clock passes the deadline, then fails
  with `Timeout` and `Operation::Connect`) or a refusal, and records every `open` call.

```rust
# #[cfg(feature = "net")]
# fn main() -> Result<(), Box<dyn std::error::Error>> {
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use stonecrab_core::{Clock, Deadline, Endpoint, ResourceLimits};
use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, Step, block_on_with};
use stonecrab_transport::TransportFactory;

let clock = FakeClock::default();
let factory = FakeTransportFactory::new(clock.clone());
factory.push_script(Script::strict(vec![
    Step::Expect(b"ping".to_vec()),
    Step::Delay(Duration::from_secs(3)),
    Step::Reply(b"pong".to_vec()),
]));

let endpoint = Endpoint::Custom("fake://plc".to_owned());
let limits = ResourceLimits::builder().build()?;
let deadline = Deadline::after(clock.now(), Duration::from_secs(10))?;
let mut transport = block_on_with(&clock, factory.open(&endpoint, limits, deadline))?;

block_on_with(&clock, transport.send(Bytes::from_static(b"ping"), deadline))?;
let mut reply = BytesMut::new();
block_on_with(&clock, transport.recv(&mut reply, deadline))?;
assert_eq!(&reply[..], b"pong");
assert_eq!(factory.probes()[0].sent(), vec![Bytes::from_static(b"ping")]);
# Ok(())
# }
# #[cfg(not(feature = "net"))]
# fn main() {}
```
