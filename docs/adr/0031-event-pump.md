# 0031: Event pump
## Status
Accepted (2026-10-29), as slice S69 of docs/POST_ALPHA_PLAN.md (decision E6: "PLC4X tools
count toward parity"). It adds the second PLC4X tool to Stonecrab: three public types at the
crate root, `EventPump`, `TagBatch` and `BatchEvent`, built only on the core contracts
(`Connection`, `Clock`, `ReadRequest`) and the connection cache of ADR-0030. Every change to
the public API is additive to the published `stonecrab` 0.2.0. No Cargo feature and no
dependency is added.

## Context
Upstream's `plc4j/tools/event-pump` (Apache PLC4X v1.0.0, commit ea229918) is a library
(`docs/parity/tools.md`, section "Event pump"). An `EventPump` holds `TagBatch`es by id
(`EventPump.java:73-312`). A batch has a connection factory and a connection string, a map
of tag names to address strings, a trigger, a listener, optional value transformations and
a backoff (`TagBatch.java:70-418`). Each fetch leases a connection from the factory, builds
one read request with all the batch's tags and executes it, then gives the connection back
(`TagBatch.java:472-647`). The listener gets `onTagsFetched`, `onError` or `onFetchSkipped`
(`TagBatch.java:999-1035`).

The parity row `TOOL-EP-TIMER` recorded two things as not read: fixed rate or fixed delay,
and what happens when a poll outlasts the interval. They are read now:

- `TimerTrigger.start` calls `java.util.Timer.schedule(task, initialDelay, interval)`, the
  fixed-delay form, and says why in a comment: fixed rate would fire the missed ticks in a
  burst once a slow fetch ends (`TimerTrigger.java:167-170`). The timer task does nothing but
  hand the firing to a dispatcher: one daemon thread with a queue of one and a
  `DiscardPolicy` (`TimerTrigger.java:115-122, 141-165`). The timer thread never blocks, so
  in practice the ticks keep the interval.
- The dispatcher calls `fetchTags`, which starts the read and returns without waiting for
  it. A tick that arrives while a fetch is still in progress (its `fetchInProgress` flag is
  set from the start of the fetch to the end of the listener call) is skipped, counted and
  reported with `onFetchSkipped(batch, lastFetchDurationMs, consecutiveSkips)`
  (`TagBatch.java:490-502`). The next fetch happens at the next tick. Only when the lease
  itself blocks the dispatcher thread does one firing wait in the queue, while later ones
  are discarded without a count.
- After a failed fetch the batch pauses: one second, doubled after each further failure up
  to one minute, reset by a fetch that succeeds. Ticks inside the pause are skipped without
  a count or a callback (`TagBatch.java:84-92, 483-488, 922-942`).
- A fetch is also bounded by a 300 s guard that exists only for a driver whose future never
  completes (`TagBatch.java:94-101, 549-554`).
- The initial delay may be zero, the interval must be positive (`TimerTrigger.java:97-103`).
  `startBatch` of a started batch throws, `startAll` skips started batches, `stopBatch` of a
  stopped one does nothing, and `addBatch` and `startBatch` throw after `close`
  (`EventPump.java:97-110, 179-245, 287-312`; `TagBatch.java:427-458`).
- `SubscriptionTrigger.start` throws `UnsupportedOperationException`
  (`SubscriptionTrigger.java:71-76`). The factory maps `type: timer` (interval and initial
  delay in seconds or milliseconds, not both) and `type: subscription`, and refuses any
  other type (`EventPumpFactory.java:157-205`).

Stonecrab's core spawns nothing (CODING_STANDARDS 8.1 and 8.7), measures time on an
injected `Clock`, and bounds every queue (8.8). Two loops already run their work while the
consumer awaits: the polling subscription of S55, whose producer is driven by
`Subscription::recv`, and the acquisition loop of S56 (`Acquisition::recv`). The polling
subscription already skips and counts the ticks that fall due while a read runs.

## Decision
1. **Placement.** The pump is core: a private file `src/event_pump.rs` whose three types are
   re-exported at the crate root, like `ConnectionCache`. It needs only core types, so it is
   built under every feature set, `default-features = false` included (ADR-0018). It is
   measured with the core at 100 percent line coverage; the coverage and lint-attribute
   scripts are unchanged. The tick arithmetic of S55 (`ticks`) moves from `src/polling.rs`
   to `src/clock.rs` so that both use it.
2. **API.** `EventPump::new(&dyn Clock)` makes an empty pump that borrows its clock, as
   `Acquisition` does. A `TagBatch` is made by `TagBatch::on_connection(name, &dyn
   Connection, ReadRequest, interval)` or `TagBatch::on_cache(name, &ConnectionCache,
   connection_string, ReadRequest, interval)`, and `initial_delay(delay)` sets the one
   optional setting (default zero, as upstream's two-argument `TimerTrigger`). There is no
   separate builder type: one optional setting does not need one, and the constructors
   validate. The pump has `add_batch`, `remove_batch` (which gives the batch back),
   `start_batch`, `stop_batch`, `start_all`, `stop_all`, `is_started`, `close` and `recv`.
   The tags are a built `ReadRequest`, not address strings: a Stonecrab tag is parsed by its
   driver before any I/O. A request with a deadline is `InvalidConfig`, because the request
   is read again at every fetch and an absolute deadline would fail every later one.
3. **Sources.** A batch reads either on a connection it borrows, or on a lease from a
   `ConnectionCache` that it takes for each fetch and gives back right after the read, as
   upstream leases from its factory for each fetch. A lease that cannot be had (a wait past
   the cache's maximum, a refused connect) is a failed fetch. Holding one lease for the
   batch's lifetime is not offered: it would block the cache's other users and expire after
   the cache's maximum lease time.
4. **One read per fetch, read only.** A fetch is exactly one `Connection::read` of the
   batch's request, with the connection's default deadline and retry policy (`TOOL-EP-READ`).
   The pump uses no other operation (`TOOL-EP-OPERATIONS`); a batch on a cache also makes
   the cache's `get`, which may open or ping the connection (ADR-0030).
5. **The interval trigger.** A started batch's ticks lie on a grid that starts
   `initial_delay` after `start_batch` and has a point every `interval`. A tick fetches when
   no fetch of the batch runs. A tick that falls due while a fetch runs is skipped and
   counted, and the next fetch waits for the next tick: upstream's rule for a fetch that
   outlasts its interval, and S55's. The grid is fixed; upstream's fixed-delay timer drifts by
   the few microseconds its task takes, which nothing observes. A batch whose next tick would
   lie past the end of the clock's time fetches no more.
6. **The pause after failures.** Upstream's backoff is kept with its defaults: one second
   after a failed fetch, doubled after each further failure up to one minute, ended by a
   fetch that succeeds. A response with item errors is a fetch that succeeded, as upstream
   treats it. The two bounds are not settings: upstream's `withInitialBackoffMs` and
   `withMaxBackoffMs` have no caller here. Unlike upstream, the ticks inside the pause are
   counted as skipped, so `skipped` accounts for every tick that did not fetch.
7. **How results reach the caller.** There are no listeners. `recv(&mut self)` returns the
   next `BatchEvent { batch, outcome, skipped }`: the batch's name, the `ReadResponse` or the
   error of the fetch, and the ticks the batch skipped since its previous event (upstream's
   `onFetchSkipped` count, folded into the next event). A failed fetch is an event, not an
   error of `recv`, and the batch goes on. `recv` drives every started batch while it waits:
   it starts the fetches that are due, polls every fetch in flight, sleeps on the clock until
   the next tick of an idle batch, and returns the first event that is ready. It resolves to
   `None` when nothing is queued or in flight and no started batch has a tick left. No task
   is spawned, as with `Subscription::recv` and `Acquisition::recv`. On a clock whose sleep
   resolves before its deadline (the simulator's `RealtimeClock`) `recv` fails with
   `InvalidConfig`, as `Acquisition::recv` does.
8. **Independence.** Each batch has at most one fetch in flight, and `recv` polls all of them
   together, so a batch whose device is slow does not hold up the batches on other
   connections (`TOOL-EP-MULTI`). Events that become ready at the same poll are queued in
   the order the batches were added.
9. **A slow consumer is bounded.** Fetches start only when the event queue is empty, so the
   queue holds at most one event per batch. Nothing runs between calls of `recv`. A tick that
   fell due while the consumer was away is fetched late, once, at its next call, and the
   earlier ones are counted as skipped, as S55 does. A slow consumer therefore costs skipped
   ticks, never memory and never a burst of reads.
10. **Cancellation.** `recv` is cancel-safe: dropping it loses no event and abandons no
    read. A fetch in flight belongs to its batch, not to the `recv` future, and goes on at
    the next call. This differs from S55, which drops the read of a dropped `recv` so that no
    read outlives its consumer's wait. The pump cannot do that: when `recv` returns one
    batch's event, the other batches' reads are still in flight and must survive to the next
    call, or a slow batch would never finish while a fast one keeps returning. Keeping them
    across a drop as well makes `recv` safe in a `select!` loop. The cost is documented: a
    read in flight between calls holds its connection's in-flight slot (or its lease), and its
    deadline keeps running. `stop_batch`, `remove_batch`, `stop_all` and `close` drop the
    fetch in flight.
11. **Errors of the pump.** A duplicate name, an unknown name, starting a started batch, a
    blank name, a zero interval and a request with a deadline are `InvalidConfig`. After
    `close`, `add_batch`, `start_batch` and `start_all` are `Disconnected` with the message
    "the event pump is closed", as a closed `ConnectionCache` refuses `get`. `close` forgets
    every batch and queued event and leaves the borrowed connections and cache open.
12. **`TOOL-EP-SUBSCRIPTION` is not ported.** Upstream's subscription trigger throws at
    start; a placeholder is no obligation (SPEC, "Compatibility baseline"). A caller who wants
    events on change subscribes through `Connection::subscribe` (S55, or a driver's native
    subscription). When a configuration format arrives (decision 13), a configuration naming
    a subscription trigger is `InvalidConfig`.
13. **`TOOL-EP-CONFIG` stays deferred.** Upstream reads YAML, JSON and XML. Each would be a
    new dependency for the core or a new feature, and the choice of the first format is the
    ADR the parity row asks for; the typed API comes first and stands on its own. A
    configuration also needs a driver to parse its address strings, which a file loader can
    take from a `DriverRegistry` when it is built.
14. **`TOOL-EP-TRANSFORM` stays deferred.** Upstream's expressions exist so a configuration
    file can scale a value. A Rust caller transforms the values of a `BatchEvent` in its own
    code, so an expression language is only worth its parser, its numeric rules and its error
    handling once configuration files exist (decision 13).
15. **Not ported.** Upstream's 300 s fetch guard: every read here is bounded by the
    connection's request timeout and every lease by the cache's maximum wait time. Changing a
    batch's tags while it runs (`addTag`, `removeTag`, `clearTags`): a batch's request is
    fixed; remove the batch and add a new one. The count and listing getters
    (`getBatchCount`, `getStartedBatchCount`, `getBatchIds`, `getAllBatches`): `is_started`
    answers whether a batch exists and runs. `TOOL-EP-POLLING-BASE` stays as it is: the S55
    polling subscription is the first-party equivalent.

## Consequences
- An application that polls a set of tags on a schedule writes one loop over `recv` instead
  of keeping its own timers, and gets the skip rule and the pause after failures for free.
- Nothing happens between calls of `recv`. An application that stops calling it for a while
  gets one late fetch per batch and a skipped count when it comes back.
- A read in flight when `recv` returns another batch's event, or when `recv` is dropped,
  holds its connection's in-flight slot (or its cache lease) until the next call; a consumer
  that takes longer than the read's deadline between calls sees that read fail with
  `Timeout`. The rustdoc of `recv` says so.
- The pump borrows its clock, connections and cache, so it lives inside the scope that owns
  them. It is `Send` (its future can run on any executor) but not `Sync`.
- Configuration files and value transformations remain open rows; adding them later is
  additive.

## Alternatives considered
- **Listener traits, as upstream.** Rejected: a callback needs a task or a thread to call it
  from, and a trait with one implementor per application adds nothing that a loop over
  `recv` lacks.
- **A spawned task per batch** (upstream's timer threads). Rejected: the core has no
  executor (8.1, 8.7), and an owned task would need the `transport` feature's Tokio for
  pure logic.
- **Firing every missed tick after a slow fetch** (fixed rate). Rejected, as upstream rejects
  it: a burst of reads on a device that is already slow.
- **Dropping the reads in flight when `recv` is dropped** (S55's rule). Rejected for decision
  10's reason.
- **A queue of events per batch with a `QueuePolicy`.** Not needed: fetching only when the
  queue is empty bounds it at one event per batch with no policy to choose.
- **A `Trigger` type or enum.** Not added: there is one working trigger. A second one adds a
  constructor or a setter, which is additive.

## Compliance
- `crates/stonecrab/src/event_pump.rs` (`EventPump`, `TagBatch`, `BatchEvent`), re-exported
  from `crates/stonecrab/src/lib.rs`; `ticks` in `crates/stonecrab/src/clock.rs`.
- Tests: `crates/stonecrab/tests/core_event_pump.rs`, on `FakeClock` against the simulator
  and against Modbus TCP over the fake transport: start and stop by name, close, the initial
  delay and the interval, a fetch that outlasts its interval, one read with every tag, events
  named by batch, two batches on different connections, a batch on a cache lease that waits
  for another holder, a failed read and the doubling pause, read as the only operation on the
  wire, a dropped `recv` while waiting and while a read runs, a slow consumer, a clock that
  never sleeps, a tick past the end of time, and the refusals of `TagBatch`. Doctests on every
  public item.
- `docs/parity/tools.md`: `TOOL-EP-PUMP`, `-TIMER`, `-READ`, `-MULTI` and `-OPERATIONS` are
  implemented with their test ids; `-SUBSCRIPTION`, `-CONFIG`, `-TRANSFORM` and
  `-POLLING-BASE` stay deferred with the reasons above.
