# 0030: Connection cache
## Status
Accepted (2026-10-29), as slice S68 of docs/POST_ALPHA_PLAN.md (decision E6: "PLC4X tools
count toward parity, starting with the connection cache and Object PLC Mapping"). It adds the
first PLC4X tool to Stonecrab: three public types at the crate root, `ConnectionCache`,
`ConnectionCacheBuilder` and `LeasedConnection`, built only on the core contracts
(`DriverRegistry`, `Connection`, `Clock`, `ResourceLimits`). Every change to the public API is
additive to the published `stonecrab` 0.2.0. No Cargo feature and no dependency is added.

## Context
Upstream's `plc4j/tools/connection-cache` (Apache PLC4X v1.0.0, commit ea229918) is a library
over a `PlcConnectionFactory`, usually the driver manager (`docs/parity/tools.md`, section
"Connection cache"). `PlcConnectionCache` keeps one `ConnectionContainer` per connection string
(`PlcConnectionCache.java:133-180`). A container holds the connection, the current
`LeasedPlcConnection` and a FIFO queue of lease requests (`ConnectionContainer.java:148-238`).
Closing a lease gives the connection back instead of closing it
(`LeasedPlcConnection.java:148-170`). A failed call through a lease marks the lease, and
returning a marked lease closes the connection and fails every queued waiter
(`LeasedPlcConnection.java:180-195`, `ConnectionContainer.java:349-378`). Before a connection
is lent again it is pinged, if it has been idle past `idlePingThreshold` or if its last lease
was force-returned. A failed or slow ping replaces it (`ConnectionContainer.java:170-200,
391-414`).

Upstream drives five bounds from a `ScheduledExecutorService` it owns: the idle timeout, the
lease timeout (which force-returns the lease), the wait timeout of each queued request, and
`connectBounded` and `closeBounded`, which run connect and close on a fresh daemon thread so a
wedged socket cannot hold the container lock (`ConnectionContainer.java:565-660`). A closed
cache throws `PlcConnectionCacheClosedException` from `getConnection`. Four admin operations
count, list and evict connections (`PlcConnectionCache.java:182-280`). A
`ConnectionStateTracker` records the subscriptions and event listeners registered through a
lease and re-registers them on a replacement connection (`ConnectionStateTracker.java:181`).

The parity record noted that the default values of the timeouts had not been read. They are
read now, at `PlcConnectionCache.java:82-96`:

| Setting | Upstream default | `0` upstream |
| --- | --- | --- |
| `maxIdleTime` | 5 min | not special-cased |
| `maxLeaseTime` | 1 min | disables the lease timeout |
| `maxWaitTime` | 30 s | disables the bound on connect (`connectBounded`) |
| `pingTimeout` | 5 s | not special-cased |
| `idlePingThreshold` | 30 s | ping before every reuse |
| `closeTimeout` | 5 s | disables the bound on close |
| `LEASE_WAIT_GRACE_MS` (not a setting) | 1 s | none |

Stonecrab's core has no executor and spawns nothing (CODING_STANDARDS 8.1 and 8.7). Every
wait is measured on an injected `Clock`, and every queue is bounded (8.8). The precedent for
work without a task is the polling subscription of S55. Its producer runs while the consumer
waits in `recv`, and the stale tracker of S56 is evaluated when it is called. A
`DriverRegistry` already turns a connection string into a connection (ADR-0011).

## Decision
1. **Placement.** The cache is core: a private file `src/connection_cache.rs` whose three
   types are re-exported at the crate root, like `DriverRegistry`. It needs only core types,
   so it is built under every feature set, `default-features = false` included (ADR-0018). It
   is measured with the core at 100 percent line coverage, and the coverage and lint-attribute
   scripts are unchanged. It adds no driver and no operation of its own.
2. **API.** `ConnectionCache::builder(Arc<DriverRegistry>, Arc<dyn Clock>)` takes the two
   things upstream's `withConnectionFactory` and `withScheduler` set. The builder's methods
   carry the other six upstream settings under Rust names: `max_idle_time`, `max_lease_time`,
   `max_wait_time`, `ping_timeout`, `idle_ping_threshold` and `close_timeout`. A seventh,
   `limits(ResourceLimits)`, bounds the queue (decision 4). `build()` returns
   `Result<ConnectionCache>`.
   `get(&str) -> OpFuture<'_, LeasedConnection>` leases. `close()` and `remove(&str)` are
   futures because they close connections. `len`, `is_empty`, `active_leases` and
   `connection_strings` are synchronous snapshots. The key is the connection string exactly
   as given, as upstream keys its map. Upstream's overload with a `PlcAuthentication` is not
   ported: Stonecrab's credentials are part of the connection configuration that the
   connection string produces.
3. **The lease is a `Connection`.** `LeasedConnection` implements `Connection`. `read`,
   `write`, `ping` and `subscribe` first check that the lease is valid and then forward to
   the cached connection. `state`, `watch_state`, `capabilities` and `limits` describe the
   cached connection. `close` gives the lease back early without closing the connection, as
   upstream's `LeasedPlcConnection.close()` does. Dropping the lease gives it back too. The
   parity row planned a `Deref` to `dyn Connection`. A `Deref` would let a caller reach the
   connection without the validity check, so a lease past its time could still be used. The
   lease is a `Connection` instead and coerces to `&dyn Connection` wherever one is expected.
4. **One holder per string, a FIFO queue, bounded.** An entry per connection string holds the
   connection, the holder (the caller whose turn it is, while it connects or pings, and then
   its lease) and the waiters in arrival order. The turn is taken lazily. When the string is
   free, the waiter at the front takes it the next time it is polled, and every change to an
   entry wakes all its waiters so the front one looks. A caller that arrives later never
   overtakes the queue. At most `ResourceLimits::max_pending_requests` callers hold or wait
   for one string, the holder included, as the transport guard counts (`TRANSPORT-QUEUE`).
   The default is 16, from `ResourceLimits`' defaults. One more is `Backpressure` before any
   I/O. Dropping a `get` future leaves the queue at once, whether it was waiting or had just
   reached the front. Nothing is handed to a caller that is no longer there, so upstream's
   grace margin has no counterpart. The price of the lazy turn is that a `get` future must be
   polled or dropped. One kept alive at the front of the queue but never polled again holds
   up its string, and the callers behind it fail with `Timeout` at their own wait deadlines.
5. **Time without a task.** Every bound is read from the injected `Clock` when the cache or a
   lease is called. A waiter sleeps on the clock until the earlier of its own wait deadline
   and the holder's lease expiry, and is also woken by every change to its entry. When it
   wakes it looks again. If the holder's lease has expired, the waiter takes the turn from it
   (upstream's force-return). If the waiter's own deadline has passed, it fails with
   `Timeout`. The idle timeout is applied by `get`, for every string: each call first evicts
   and closes the connections nobody leased for `max_idle_time`. `len` counts an idle
   connection past its time until the next `get`. A clock whose sleeps resolve at once (the
   simulator's `RealtimeClock`) cannot pace these bounds. A waiter on it then waits only for a
   wake-up and never spins, and a ping or a close on it is never cut short. The rustdoc says
   so.
6. **Defaults and zero.** The defaults are upstream's: idle 5 min, lease 1 min, wait 30 s,
   ping timeout 5 s, idle ping threshold 30 s, close timeout 5 s. Upstream's values are chosen
   for remote controllers on plant networks, and keeping them means a configuration written
   for upstream behaves the same here. Upstream's `0` disables the lease bound and the close
   bound. Here `max_idle_time`, `max_lease_time`, `ping_timeout` and `close_timeout` must be
   positive (`InvalidConfig`), because SPEC requires every wait to be bounded and a cache
   that never takes a lease back lets one forgotten lease hold a controller's connection
   forever. A zero `max_wait_time` means "do not wait": a `get` for a leased string fails with
   `Timeout` at once. A zero `idle_ping_threshold` pings before every reuse, as upstream's
   does. Every duration must also fit the clock from the moment of `build`.
7. **Lending, the ping rule and replacement.** On its turn a caller does the following, in
   this order:
   - If the entry has no connection, it opens one with `DriverRegistry::connect`. A new
     connection is lent without a ping.
   - If a call through an earlier lease failed with `Disconnected` or `Timeout` (the
     connection is *broken*), or the connection's `state()` is not `Connected`, it closes the
     connection and opens a new one. It opens none if the string was removed or the cache
     closed during the close.
   - If the connection was idle for `idle_ping_threshold` or longer, or its last lease
     expired, and its `Capabilities.ping` is true, it pings it first. A ping that fails or
     outlasts `ping_timeout` breaks the connection, and it is replaced as above.
   - Otherwise it lends the connection as it is.
   A connection whose `Capabilities.ping` is false is reused without a ping. A failure on its
   first use breaks it, and the next lease replaces it. A failed call does not fail the queued
   waiters, as upstream's invalidation does. The next caller replaces the connection instead.
   Upstream marks a lease on any exception. Stonecrab marks it only on the two kinds that say
   the link is unusable. A device exception, an invalid address or `Backpressure` leaves the
   connection in the cache.
8. **A lease past its time.** `max_lease_time` after the lease began, every call through it
   fails with `Timeout` (diagnostics: the operation, category `Validation`) before it reaches
   the connection, and a waiter may take the connection over. A lease that was given back,
   whose string was removed or whose cache was closed fails with `Disconnected`. Upstream
   throws a `PlcRuntimeException` in both cases. Expiry does not close the connection. The
   next lease of a connection whose lease expired pings it first (upstream's
   `forceValidationOnNextLease`).
9. **Connect and close bounds.** A connect is bounded by the driver, by `timeouts.connect` of
   the connection string, which `Driver::connect` already guarantees (ADR-0011). The cache
   does not race it a second time, and needs no `connectBounded`. A close by the cache (idle
   eviction, replacement, `remove`, `close`) races the connection's `close` against
   `close_timeout` on the clock. A close that loses is dropped, and so is the connection: no
   thread is left behind, because the close was a future and dropping it cancels it.
10. **Close and remove.** `close()` marks the cache closed and empties it before it closes the
    first connection. Waiters fail with `Disconnected`. A lease still out fails its next call
    with `Disconnected`. A later `get` fails with `Disconnected` and the message "the
    connection cache is closed". `close()` again does nothing. `remove(string)` forgets the
    string first, so the next `get` opens a new connection, then closes the old one even
    while it is leased. It returns whether the string was cached. Its waiters fail with
    `Disconnected`, and a connection that was being opened for it is closed when it arrives
    instead of being cached.
11. **Subscription restore is deferred.** `TOOL-CC-RESTORE` is not implemented. A subscription
    made through a lease (`Connection::subscribe`) belongs to the cached connection. Under
    S55 its producer lives in that connection's state and the caller holds the
    `Subscription`. Restoring it on a replacement would need a forwarding subscription that
    swaps its producer and reconciles its queue and loss counters across connections. That is
    a design of its own and not straightforward. Until then, a subscription made through a
    lease outlives the lease, ends when its connection closes or fails (as S55 defines), and
    is not re-created on a replacement. The rustdoc of `LeasedConnection` says so.

## Consequences
- An application that shares a controller between components asks the cache for a lease by
  connection string instead of keeping its own connection. The components then take turns,
  in order, and a forgotten lease is taken back after `max_lease_time`.
- Nothing happens between calls. An idle connection stays open past `max_idle_time` until
  the next `get`, and an expired lease is noticed when someone calls. An application that
  wants eager eviction calls `get` or `remove` from its own schedule.
- `get` may spend time on connections other than its own: it closes the idle ones first, each
  bounded by `close_timeout`.
- Waiters are not failed when the connection fails. Each one replaces the connection in turn,
  bounded by the connect timeout, so a dead controller costs every waiter its own connect
  attempt (or its wait time) instead of one shared failure.
- A `get` future that reaches the front of its queue and is then neither polled nor dropped
  blocks its string until it is (decision 4). An application polls each `get` to completion
  or drops it, as it would any future.
- The simulator has no ping, so the ping rule is tested against Modbus TCP over the fake
  transport. The rest runs against the simulator.

## Alternatives considered
- **A scheduler task per cache** (upstream's model). Rejected: the core has no executor, and
  an owned task would need the `transport` feature's Tokio for a feature that is pure logic.
- **Handing the connection to the next waiter on return** (upstream's queue completes the
  oldest future). Rejected: a waiter dropped between the hand-over and its next poll would
  need a second hand-over path. With the lazy turn a dropped waiter simply leaves the queue.
- **`Deref<Target = dyn Connection>` on the lease.** Rejected for decision 3's reason.
- **Failing all waiters when a lease's connection fails** (upstream). Rejected: a waiter
  that has not used the connection has no failure of its own to report, and the next turn
  replaces the connection anyway.
- **A cancellation token on `get`.** Not added: dropping the future is the cancellation, and
  no caller in this slice needs a token.
- **Restoring subscriptions now.** Deferred (decision 11).

## Compliance
- `crates/stonecrab/src/connection_cache.rs` (`ConnectionCache`, `ConnectionCacheBuilder`,
  `LeasedConnection`), re-exported from `crates/stonecrab/src/lib.rs`.
- Tests: `crates/stonecrab/tests/core_connection_cache.rs`, 23 cases on `FakeClock` against
  the simulator and against Modbus TCP over the fake transport. They cover one lease at a
  time, arrival order, the wait timeout, a cancelled waiter, the queue bound, lease expiry and
  take-over, early return with `close`, close and get-after-close, the idle ping, a failed
  ping, a forced ping after an expired lease, a driver without ping and a failure on first
  use, the idle timeout, the connect and close bounds, the builder's refusals, the counts,
  `remove` while leased, while connecting and while waiting, close during a ping, the
  operations passed through, and a clock that never sleeps. Doctests on every public item.
- `docs/parity/tools.md`: every `TOOL-CC` row is implemented with its test ids, except
  `TOOL-CC-RESTORE` (deferred, decision 11).
