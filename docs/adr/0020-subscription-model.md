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

## Addendum (2026-10-28, slice S55): polling subscriptions
Slice S55 of docs/POST_ALPHA_PLAN.md makes the simulator and `modbus-tcp` subscribe by
polling. Both report `SubscriptionMode::Polling` in their capabilities and override
`Connection::subscribe` with one shared producer, `crate::polling` (private; `Plan` and its
`start`). The types of decisions 1 to 5 keep their shape; `LossCounters` gains a `skipped`
field and `SubscriptionSender` a crate-private `count_skipped` method, as the consequences
above foresaw.

**No spawned task: the subscription drives its producer.** The consequences above said the
connection would own a spawned polling task and abort it in `close`. That would need an
executor, which the core does not have (CODING_STANDARDS 8.1): the simulator has no Tokio
at all, and the contract suite runs every driver under the testkit's single-threaded
`block_on_with` on a `FakeClock`. Instead the producer is a future that the
`Subscription` owns and polls from `recv` whenever its queue is empty. It is structured by
construction: dropping or unsubscribing the subscription drops the producer with any read it
has in flight, and the producer holds the connection's state only weakly (`Weak`), taking a
strong reference for the length of one read, so it never keeps a dropped connection alive.
Nothing is detached, so CODING_STANDARDS 8.7 has no `JoinHandle` to keep. The cost is that a
polling subscription samples while its consumer waits in `recv`: a consumer that is busy
elsewhere meets skipped ticks, counted in `LossCounters::skipped`, rather than a growing
queue.

*Cancelling a waiting `recv`.* A `recv` dropped while the producer's read is in flight
(`select!`, a timeout) would leave that read suspended, holding the connection's in-flight
slot, until a later `recv` polled it. The drop therefore cancels the read: the
`Subscription` tells the producer through a small shared flag (`Gate`), polls it once with
a no-op waker, and the producer drops the read, which releases the slot, counts the tick
in `LossCounters::skipped` and stops at the yield it makes after a cancelled read, so that
poll starts no other read. A `recv` dropped while the producer waits for its tick cancels
nothing. Both flags of the `Gate` are `Relaxed` (CODING_STANDARDS 8.11): they gate no other
data, and every access happens either in the producer's poll or in the `recv` side, both
under the subscription's poller lock, which orders them.

*One `recv` at a time.* `recv` takes `&mut self`. With `&self`, two `recv` futures could be
pending at once; each poll handed the producer its own waker, replacing the earlier one, so
the first `recv` was never told that its tick came due, and dropping the last one cancelled
the read the first was waiting on and left the producer parked with no real waker: a
permanent hang in safe code. Taking `&mut self` makes that impossible at compile time (a
`compile_fail` doctest on `recv` keeps it so), as `tokio::sync::mpsc::Receiver::recv` does.
The cost is that `loss()` and `Debug` cannot be read while a `recv` is pending. The type was
unreleased, so no published API changed.

*Panics.* A panic inside the producer (a user `Clock`, a `tracing` layer, a driver bug)
unwinds out of `recv` and poisons the poller lock. The producer is then never polled again:
polling a future after its panic panics once more, and from the `Drop` of a `recv` that
unwinds through its owner that aborts the process. Locking a poisoned poller drops the
producer instead, which drops its sender and ends the stream after the queue; a `recv` with
a read in flight that is dropped while its thread unwinds from any other panic drops the
producer too, rather than poll it during the unwinding. Either way the subscription ends.

*Yielding.* A tick that never returned `Pending` (its wait for the clock and its read both
resolved at once; the producer's `suspended` flag records whether any step of the tick
returned `Pending`), or whose read was cancelled, ends with one yield (`Pending`, woken at
once); a tick that did give control back needs none. Without the yield a clock whose sleeps
resolve at once, the simulator's default `RealtimeClock`, would sample in a loop inside one
poll, without end under `DropOldest` and `CoalesceLatest`, which never block. With it a
cyclic tick on such a clock queues its sample and the `recv` resolves, so each `recv` takes
one fresh sample and the disclosed interval paces nothing, as the simulator and
`Subscription::interval` document. A change-of-state tick whose value did not change queues
nothing, so the yield would wake the `recv` at once and the next poll would read again: a
`recv` waiting for a change would burn a core and send one device read per poll. `Plan::new`
therefore refuses `ChangeOfState` with `Unsupported(Operation::Subscribe)` on a clock that
does not sleep, found by polling a sleep one interval ahead once with a no-op waker: it
resolves at once and the clock has not reached its deadline. Pacing the simulator by real
time would need a timer, which it does not have.

*What a slow consumer observes.* The producer samples only for a `recv` that finds the
queue empty, so a sample starts with an empty queue. A slow consumer therefore never meets
`CoalesceLatest` (a request needs `max_queued_events` at least its tag count, so a sample
always fits and no pending event shares a tag with a new one): it gets the latest sample
and the ticks it missed are counted in `skipped`. `DropOldest` and `Backpressure` engage
only when one sample has more tags than `max_queued_events`: `DropOldest` discards the
oldest events of that sample and counts them in `dropped`, and `Backpressure` stalls the
producer, which reads nothing until the consumer takes an event, and counts the ticks that
came due meanwhile in `skipped`. The declared policy still holds for a native producer that
pushes without a consumer waiting (decision 2); for polling, `skipped` is the loss counter
of a slow consumer. Decision 2's wording that a polling producer "stops reading the device
while the consumer is behind" therefore holds only in this narrow case (see above) and
the Consequences bullet on an owned task is superseded by "No spawned task".

*Starvation.* A `recv` dropped more often than a read takes (a `select!` branch that
keeps winning, or a timeout shorter than the device latency) cancels each read before it
ends, so nothing is delivered and `skipped` grows by one per cancel. This is a limit of the
no-task design, stated in the `Subscription` docs; an owned task (a caller-spawned drive
future or a feature-gated Tokio task) remains the maintainer's alternative.

*A dropped connection.* The producer holds the connection weakly, so a `Weak` alone would
tell it that the last handle is gone only at its next tick. The state both drivers share
with their subscriptions therefore owns a `Lifeline` (`crate::polling`), a small state cell
that reports `Closed` when it is dropped; the producer watches it beside the connection's
state while it waits for a tick, so a connection dropped without `close` ends the wait at
once. `close` and a terminal failure end the wait at once through the state watch.

**Semantics.**
1. Interval: `Cyclic(d)` polls every `d`; `ChangeOfState` polls every 1000 ms, upstream's
   default polling interval of `PollingSubscriptionConnectionBase`. `Subscription::interval`
   discloses it and `mode()` is `Polling`. `Event` is refused with
   `Unsupported(Operation::Subscribe)` before any I/O and before every other check: polling
   cannot deliver an event.
2. The call is checked as a read is, before any I/O: the kind, then the driver's own read
   checks (the connection is open, the token is not cancelled, every address is the
   driver's, the tags fit the connection's limits, the deadline has not elapsed), with
   `Diagnostics` naming `Operation::Subscribe`. The tick read is a `ReadRequest` of the
   request's tags built against the connection's limits, with no deadline or token, so each
   tick gets the connection's default deadline, retry policy and batching.
3. Ticks: the first is due at once (upstream's timer has no initial delay); the next ones
   lie on a grid of the interval from the first. A tick that came due while the consumer
   was not waiting is sampled late, once, and the grid points before it are skipped. The
   ticks that come due while a read runs, up to and including the instant it ends, are
   skipped and the next read waits for the next grid point, so a stall never ends in a
   burst (SPEC.md "Skip missed ticks instead of flooding a recovering device"). Every
   skipped tick is counted.
4. Delivery: cyclic delivers every item of every reading. Change-of-state delivers a tag's
   first outcome and then each change, comparing values with `PlcValue`'s equality (upstream
   compares with `Objects.equals`); failures compare by `ErrorKind`, so a tag that keeps
   failing the same way reports it once, a change of cause is delivered, and the next
   value is delivered again.
5. Failures of a tick's whole read: `Backpressure` (the single in-flight slot is taken) and
   `Disconnected` while the connection is not terminal (the link is being re-established,
   decision 6) skip the tick. Any other failure, a timeout among them, reaches the consumer
   as an item error for every tag, carrying the kind, message and diagnostics of the read's
   error. Per-item errors of a reading are delivered as items, as a read returns them.
6. End: the producer ends when the connection reports `Closing`, `Closed` or `Failed`
   (observed through its state watch while waiting, and checked after a failed read), or
   when the connection is gone. Its sender drops, so `recv` delivers what is queued and then
   `None`. A `Modbus` reconnect does not end it; a simulator `FailureStep::Disconnect`,
   which fails the connection for good, does.

**The in-flight slot: skip, never wait.** A polling read goes through the connection's own
read path and therefore takes its single in-flight slot (`Capabilities.concurrency` is 1
for both drivers). A user read that starts while a polling read is in flight fails with
`Backpressure` before any I/O, exactly as two user reads do, and a tick whose read finds the
slot taken by a user read, write or ping is skipped and counted. Waiting for the slot was
rejected: neither connection has a wait queue (the simulator documents a queue length of
zero), and a queued tick is the catch-up burst point 3 forbids. Tests:
`a_polling_read_and_a_user_read_never_run_together` (`tests/modbus_connection.rs`) and
`a_read_refused_for_the_busy_slot_or_a_link_being_reopened_skips_its_tick`
(`src/polling.rs`).

**Contract suite.** Six cases for a driver that reports subscriptions supported: a cyclic
subscription delivers one event per tick and discloses its interval; a change-of-state
subscription delivers a first value and then only changes; a polling driver refuses an event
subscription with no I/O; unsubscribing stops the polling reads; closing the connection ends
its subscriptions after the queued events; a stalled polling read skips the ticks that came
due and never bursts. The four that wait for ticks need a clock that moves only when the
suite waits, so, like the suite's other timing cases, they also require the fixture's
timeout hook; a fixture on a real clock (the loopback and pymodbus runs) skips them by name
and runs the other two. `Requires::Subscribe` is appended after the existing variants. The
unsupported-subscribe case of S54 now skips for both drivers with the reason "driver reports
subscribe supported; the case needs it unsupported" and still passes for the testkit's
driver that supports nothing. The suite has 33 cases.

**Alternatives considered.** A Tokio task per subscription, owned by the connection, was
rejected for the reasons above (no executor in the core or the simulator, and the suite's
deterministic executor could not drive it); a `Subscription::drive` future for the
application to spawn on its own executor, so sampling would not depend on `recv`, was left
out until a caller needs it. Upstream's fixed-delay schedule (the next poll an interval
after the previous one ends) was not copied: a grid keeps the disclosed interval true and
makes skipped ticks countable. Upstream's rule that an error tag is reported only alongside
a tag that fired was not copied either: the consumer sees every failure (decision 5,
"silent loss is prohibited").

**Compliance.** `crates/stonecrab/src/polling.rs` unit tests (interval per kind and the
event refusal, one event per tick, change-of-state with a failure, slow reads, a late
consumer, the busy slot and the reconnect, the end of the connection, a dropped connection,
unsubscribe, a dropped `recv` mid-read, each queue policy under a slow consumer, the
change-of-state refusal on a clock that never sleeps, and a read that panics),
`src/sim/connection.rs` unit tests (one sample per `recv` on a clock that never sleeps,
under every policy; change-of-state refused on the `RealtimeClock` before any read; a
dropped connection ends the wait at once; a dropped `recv` leaves the slot free for a user
read), `src/subscription.rs` (the `compile_fail` doctest of two pending `recv`s), the
contract cases above for the simulator (`tests/sim_contract.rs`) and `modbus-tcp` over the
fake transport (`tests/modbus_contract.rs`), the event and close cases
over loopback sockets and pymodbus, and the Modbus tests of `tests/modbus_connection.rs`
(among them a dropped connection ending the wait at once). The S54 Compliance above deferred
"contract cases that drive a driver's queue per policy" to S55. That deferral is closed by
the unit tests, not by contract cases: no contract case drives `CoalesceLatest`, and the
cases use `Backpressure` and `DropOldest` only incidentally. A polling producer samples only
into an empty queue, and a request needs room for one event per tag under
`CoalesceLatest`, so no driver of this crate can make a slow consumer meet coalescing, and
`DropOldest` or `Backpressure` engage only when one sample outgrows `max_queued_events`,
which a contract case cannot ask of every fixture's limits. What a slow consumer observes
under each policy is pinned by
`a_slow_consumer_meets_the_queue_policy_only_when_a_sample_outgrows_the_queue`
(`src/polling.rs`) and, on a clock that never sleeps,
`a_clock_that_never_sleeps_gives_one_sample_per_recv_under_every_policy`
(`src/sim/connection.rs`); the queue itself by the S54 unit and property tests.
Parity rows: `MB-TCP-SUBSCRIBE` (implemented), `MB-TCP-CAPABILITIES`, `SIM-SUBSCRIBE`,
`CORE-SUBSCRIPTION`.
