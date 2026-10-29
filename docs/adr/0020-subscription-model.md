# 0020: Subscription model and bounded event queue
## Status
Accepted (2026-10-28), as slice S54 of docs/POST_ALPHA_PLAN.md (findings G07, the
subscribe part, and G10). It is the ADR that ADR-0011's consequences ask for before
`subscribe` gets request types, and it fixes the types S55 (polling subscriptions), S56
(the stale tracker) and S57 (eager reconnect) build on. Every change is additive to the
published `stonecrab` 0.2.0. No driver subscribes in this slice: the simulator and
`modbus-tcp` keep `SubscriptionMode::Unsupported`.

## Context
SPEC.md requires a subscription with "explicit lifetime, bounded delivery, loss reporting
and unsubscribe", and says: "A polling subscription must expose its sampling interval and
that it is polling. Skip missed ticks instead of flooding a recovering device. Slow
consumers must encounter a declared queue policy: backpressure, latest-value coalescing
or dropping with explicit loss counters. Silent loss is prohibited." CODING_STANDARDS 8.8
says every producer's behavior on a full channel is one of the three `QueuePolicy`
variants with its loss counter updated, 8.11 limits atomics to counters such as those in
`LossCounters`, and 9.6 makes `Subscription` `#[must_use]`. `ResourceLimits` has carried
`max_queued_events` (default 256) since S03, and nothing reads it (G10).

Upstream (Apache PLC4X v1.0.0, `upstream/manifest.toml` `[inventory.api]` and
`[inventory.util_subscription_emulation]`): `PlcSubscriptionRequest` adds tags one at a
time, each with its own `PlcSubscriptionType` (CYCLIC with a polling interval,
CHANGE_OF_STATE with an optional minimum interval, EVENT). The response hands out one
`PlcSubscriptionHandle` per tag; a caller registers `Consumer` callbacks on handles and
unsubscribes with a separate request. Events are a `PlcReadResponse` plus a timestamp.
The polling helper that seven connections extend groups tags by (type, interval), polls
CYCLIC at the tag's interval and CHANGE_OF_STATE at a 1000 ms default (the minimum
interval is ignored), refuses EVENT, and calls consumers on the polling thread. There is
no queue between the poll and the consumer and no loss counter; the scheduling of the
event pump was not read (marked UNCERTAIN in the manifest).

The core has no executor (CODING_STANDARDS 8.1), so the queue cannot be a Tokio channel.
`StateCell` and `StateWatch` (ADR-0007) already show the runtime-free shape: a
`Mutex`-guarded value, parked wakers in `WakerSlots`, futures that release their slot on
drop.

## Decision
1. **`SubscriptionKind`**, `#[non_exhaustive]`, `Copy`: `Cyclic(Duration)`,
   `ChangeOfState` and `Event`, the three upstream types. A change-of-state kind takes no
   interval: upstream's polling helper ignores the one it accepts, and a polling driver
   discloses the interval it uses (decision 5).
2. **`QueuePolicy`**, `#[non_exhaustive]`, `Copy`, with three variants and no default:
   - `Backpressure`: nothing is lost. When the queue is full, `SubscriptionSender::send`
     waits until the consumer takes an event. A polling producer therefore stops reading
     the device while the consumer is behind, and the ticks it could not run are skipped
     (S55 counts them).
   - `CoalesceLatest`: at most one pending event per tag. An event for a tag that already
     has one pending replaces it in place (the tag keeps its place in the queue) and adds
     one to `LossCounters::coalesced`. A request with this policy and more tags than
     `max_queued_events` is refused by its builder (`ResourceLimit`), so the queue always
     has room for every tag; if a producer still offers a new tag to a full queue, the
     oldest event is dropped and counted in `dropped`.
   - `DropOldest`: when the queue is full the oldest pending event is discarded and
     `LossCounters::dropped` grows by one, then the new event is queued.
3. **`SubscribeRequest`** and its builder follow `ReadRequest` (ADR-0003, ADR-0011
   decision 5): `SubscribeRequest::builder(limits, kind, queue)` takes the kind and the
   queue policy as arguments, so neither can be forgotten; `tag`, `deadline`, `cancel` and
   `build` behave as on `ReadRequestBuilder` (item budget, duplicate identities, empty
   batch, sticky first error). `build` also refuses `Cyclic(Duration::ZERO)`
   (`InvalidConfig`) and the coalescing case of decision 2. One kind applies to the whole
   request, unlike upstream's per-tag type: a caller who mixes kinds makes one
   subscription per kind, and a subscription then has one interval to disclose. The
   deadline and the token bound the `subscribe` call, exactly as for `read`: an absent
   deadline becomes `Deadline::after(clock.now(), timeouts.request)`, an elapsed one or a
   cancelled token is rejected before any I/O. Neither bounds the lifetime of the
   subscription.
4. **The bounded queue.** `SubscriptionSender::polling(interval, queue, limits)` returns
   the producer and the `Subscription`. The queue capacity is
   `limits.max_queued_events()`, which closes G10; the queue never holds more. Events are
   `ReadItem`s, one per tag and sample: they already carry the tag identity, the value or
   a per-item error, the sampling instant and the device time when there is one, which is
   what upstream's per-event `PlcReadResponse` and timestamp carry. `LossCounters` is a
   `Copy` snapshot with public `dropped` and `coalesced` fields, `#[non_exhaustive]` so
   S55 can add a missed-tick count; the counters live inside the queue's lock, so no
   atomics are needed. The queue is a `Mutex<VecDeque>` with `WakerSlots` for each side,
   in the idiom of `StateCell`; no executor type is public.
5. **`Subscription`** (`#[must_use]`) is the handle and the event stream at once:
   `mode()` is `Polling` or `Native` and `interval()` is the sampling interval the driver
   actually uses (always present for polling), which is SPEC.md's disclosure; `loss()`
   reads the counters; `recv()` resolves to the next event, waiting while the queue is
   empty, and to `None` once the producer has ended and every queued event has been
   delivered. Ending the producer never discards a queued event. `unsubscribe(self)` ends
   the subscription and returns the final counters; dropping the handle does the same
   without them. Events still queued then are discarded as the caller's choice, not
   counted as loss. The producer learns of it through `is_closed()` and through `send`,
   which then fails with `Cancelled`.
6. **Connection close and failure.** A connection ends every subscription it serves when
   it closes or fails, by dropping the producer: the consumer receives what was queued,
   then `None`, and finds the reason in `Connection::state`. A subscription survives a
   reconnect (S57); samples that could not be taken meanwhile are skipped ticks, not
   errors.
7. **`Connection::subscribe`** is a provided method, `fn subscribe(&self, request:
   SubscribeRequest) -> OpFuture<'_, Subscription>`, modeled on the S40 ping (ADR-0011
   addendum): the default performs no I/O, ignores the request's deadline and token, and
   fails with `ErrorKind::Unsupported(Operation::Subscribe)` carrying `Diagnostics` that
   name the operation, the connection state and the `Validation` category. `ping` and
   `subscribe` share one private constructor for that error. A driver that overrides
   `subscribe` sets `Capabilities::subscribe`. The contract suite gains the case "an
   unsupported subscribe performs no I/O and changes no state", which offers one request
   per queue policy (and per kind) and runs for every driver that reports subscriptions
   unsupported.

## Consequences
- A subscription has an owner, a bound and visible loss, and no executor leaks into the
  public API: an application can poll `recv` from any runtime.
- S55 builds `SubscriptionSender::polling` in the Modbus connection, reads at the
  disclosed interval, sends one `ReadItem` per tag, checks `is_closed` every tick so a
  dropped subscription stops polling, skips and counts missed ticks (adding a field to
  `LossCounters` and a `SubscriptionSender` method that bumps it), refuses `Event` with
  `Unsupported(Subscribe)`, and owns its polling task as
  CODING_STANDARDS 8.7 requires (the connection aborts or awaits it in `close`).
- S56 consumes `ReadItem`s from a subscription; S57 keeps subscriptions alive across a
  reconnect and ends them only on `close` or a terminal failure.
- A native subscription (M3) needs its own constructor, which may add a `Native` mode
  without an interval. `interval()` already returns an `Option` for that reason.
- A backpressured producer blocks. A native driver that cannot block its receive path
  must pick another policy or document what blocking means on its transport.
- Mixing kinds in one request is not possible, which is a recorded divergence from
  upstream (`docs/parity/core.md#CORE-SUBSCRIPTION`).

## Alternatives considered
- **Upstream's per-tag handles and consumer callbacks.** Rejected: a callback runs on the
  producer's task, so a slow consumer either stalls the producer with no declared policy
  or needs an unbounded hand-off; one handle per subscription with a pull-based queue
  makes the policy and the bound explicit.
- **A Tokio `mpsc` channel in the public API.** Rejected: the core has no executor
  (CODING_STANDARDS 8.1), and an `mpsc` sender cannot coalesce or drop the oldest event.
- **A per-tag kind.** Rejected for now: one interval per subscription keeps the
  disclosure a single value, and a caller can hold several subscriptions.
- **A capacity per request or per policy variant** (`Backpressure(capacity)` as drawn in
  ARCHITECTURE 3.3). Rejected: `max_queued_events` already bounds the queue and a second
  knob would need a rule for which one wins.
- **A `SubscriptionEvent` enum** (value, stale, loss, state change, end). Rejected until a
  second variant has a producer: loss is read through `loss()`, the end is `None`, the
  state is `watch_state`, and S56 decides whether staleness needs an event.
- **Refusing instead of waiting under backpressure.** Rejected: refusing leaves the
  producer to discard the event, which is the dropping policy under another name.
- **Atomic loss counters.** Not needed: every update happens under the queue lock.

## Compliance
Unit and property tests in `crates/stonecrab/src/subscription.rs` check each policy
(backpressure loses nothing and the waiting send resolves once the consumer takes an event,
coalescing keeps the latest value per tag, dropping counts every lost event, the queue
never exceeds `max_queued_events`) and the end of the stream on either side;
`crates/stonecrab/tests/core_subscribe_contract.rs` checks the builder's refusals. The
contract case of decision 7 (one request per policy, each with a cancelled token and an
elapsed deadline, all refused with no I/O) runs for the simulator
(`tests/sim_contract.rs`) and for `modbus-tcp` over the fake transport
(`tests/modbus_contract.rs`), loopback sockets (`tests/modbus_contract_scripted_peer.rs`)
and pymodbus (`tests/modbus_interop_pymodbus.rs`, interop job). Contract cases that drive
a driver's queue per policy are deferred to S55, the first slice where a driver produces
events; until then the queue behavior is proven by the unit and property tests above.
`tests/public_contract.rs` lists the new types as `Send + Sync + 'static`. The parity rows
are `CORE-CONNECTION-TRAIT` and `CORE-SUBSCRIPTION`. `scripts/coverage-gate.sh` holds the
core at 100% lines.
