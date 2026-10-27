# 0007: Clock, cancellation and state observation
## Status
Proposed, drafted in slice S06 on 2026-10-27; it supersedes the ADR-0003 deferral of
cancellation only when S11b-2 accepts it, where `SimConnection` first enforces the token
and the default-deadline rule. Until then the primitives below exist and are tested, and
no request type carries a cancel slot (that is S11a). It records decisions D9 and D15 of
docs/ALPHA_PLAN.md and adjusts ARCHITECTURE.md section 3.4, which the S31 reconciliation
slice updates.

## Context
ADR-0002 shipped explicit absolute `Deadline`s and no clock, and ADR-0003 deferred
cancellation tokens and relative-deadline clock ownership "until a connection
implementation can enforce them". Slices S10 (testkit `FakeClock`), S11a (`Driver`,
`Connection`), S16 (transport) and S19 (`ConnectionGuard`) all need three things from
core that do not depend on an executor: a nameable future type for object-safe traits, a
clock to inject, and a way to cancel work and to observe connection state.

Constraints. Core is Tokio-free and compiles on any target (CODING_STANDARDS 8.1;
D15 and section 15.7 confine `tokio-util` to the transport crate). `clippy.toml`
disallows `std::thread::sleep`, and core has no timer. `async-trait` is banned
(CODING_STANDARDS 8.2). The alpha adds no dependency to core in this slice.

## Decision
1. **`OpFuture<'a, T>`** is `Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>`.
   Public traits return it so they stay object safe (`Arc<dyn Clock>`,
   `Box<dyn Connection>`), the `Send` bound is visible in the type rather than hidden by a
   macro, and the alias names only `std` types. One allocation per operation is accepted;
   it is small beside a network round trip. Implementations write
   `Box::pin(async move { ... })`; inside a crate `async fn` is used freely.
2. **`Clock`** is `trait Clock: Send + Sync { now() -> Instant; wall_now() -> SystemTime;
   sleep_until(Instant) -> OpFuture<'_, ()> }`. `now()` is the only source for deadline
   arithmetic; `wall_now()` is for receipt timestamps and never feeds a timeout.
   `sleep_until` is the single place a connection waits, so a test clock can advance time
   without waiting. Dropping the future cancels the sleep with no side effects.
3. **Core ships no `Clock` implementation, and `SystemClock` moves out of core.**
   ARCHITECTURE.md section 3.4 draws `SystemClock` in core. Core may not sleep and has no
   timer, so a real `sleep_until` cannot live there. `SystemClock` lives in
   `stonecrab-transport` (S16) with `now() = tokio::time::Instant::now().into_std()`, so
   paused-time tests agree with `Deadline`. The Tokio-free implementations are
   `FakeClock` in the testkit (S10) and `RealtimeClock` in the simulator driver (S11b-1,
   D9; `now()` is `Instant::now()` and `sleep_until` resolves immediately). A caller who
   injects a clock derives its deadlines from that clock.
4. **`CancellationToken` is hand-written over `std::sync::Mutex`.** It is `Clone`; all
   clones share one flag. `cancel()` is idempotent and wakes every parked waker exactly
   once, after releasing the lock. `child()` returns a token that is cancelled when the
   parent is; cancelling a child never cancels the parent or a sibling. `cancelled()`
   returns an `OpFuture<'_, ()>` that is `Pending` until cancelled and `Ready(Ok(()))`
   on the first poll if already cancelled. Core stays Tokio-free, so the
   `tokio-util` token is not an option here.
5. **Waker deduplication and `Weak` children.** Each waiting future owns a slot in a
   crate-private list. A re-poll with a waker for which `Waker::will_wake` holds changes
   nothing, and a re-poll with a different waker replaces the parked one in place, so
   repeated polls never grow the list; dropping the future releases its slot. Parents hold
   children as `Weak`, so a child no handle references is freed and is neither woken nor
   counted, and creating a child prunes dead entries. Descendants are cancelled with an
   explicit worklist, not recursion. A poisoned lock is recovered with
   `PoisonError::into_inner` because every critical section only flips a flag, bumps a
   counter or moves vectors. There is no `unsafe` and no new dependency.
6. **`StateCell` and `StateWatch` observe `ConnectionState` with latest-value
   semantics.** The owner calls `StateCell::set`, which stores the state, bumps a version
   counter and wakes every parked watcher. A `StateWatch` remembers the last version it
   reported; `changed()` is `Pending` until the version differs and then resolves once
   with the latest value. Intermediate states overwritten before the watcher polls are
   coalesced, and the rustdoc says so. It is deliberately not a queue: a consumer that
   needs every transition reads the tracing events. `set` always bumps the version, even
   to an equal state, because the owner decides what a transition is and a re-asserted
   `Reconnecting` with a new `next_at` is news; core does not compare states. A watch
   marks a version seen only when `changed()` returns `Ready`, so a dropped future loses
   no change. Dropping the cell wakes nobody; terminal states `Closed` and `Failed` are
   the end-of-stream signal.
7. **Default-deadline rule (documented here, enforced in S11b-2).** A request whose
   `deadline` is absent receives `Deadline::after(clock.now(), timeouts.request)`, owned
   by the connection, so no operation waits without a bound (CODING_STANDARDS 8.5). The
   deadline is computed from the connection's clock, not from the system clock. Core only
   provides the types; `SimConnection` is the first enforcer, and `ConnectionGuard`
   (S19) takes over the rule for real transports.

## Consequences
- S10, S11a, S11b-1, S16 and S19 can build on stable names: `OpFuture`, `Clock`,
  `CancellationToken`, `StateCell`, `StateWatch`. The facade re-exports all five, so
  `stonecrab` 0.2.0 and `stonecrab-core` 0.1.0 record them.
- ARCHITECTURE.md section 3.4 is wrong about `SystemClock` until S31 reconciles it.
- Core carries hand-written waker bookkeeping to maintain, each path covered by
  hand-polled tests, in exchange for no runtime dependency.
- Waiter lists use linear search on slot id; waiter counts are tiny (one per in-flight
  operation), so this is not a hot path. Two futures that share a waker each hold a slot
  and the task is woken twice, which is harmless.
- A `StateWatch` whose cell is dropped without a terminal state pends forever; observers
  that must not hang combine it with a deadline or a token.
- No request type has a cancel slot until S11a, so this slice proves token semantics
  only, not cancellation of an operation on the wire. Wire behavior (unknown-after-
  transmit outcomes, resync) is S19 and its tests.

## Alternatives considered
- **Re-export `tokio_util::sync::CancellationToken`.** Rejected: it pulls Tokio and
  `tokio-util` into core, contradicting 8.1, D15 and section 15.7.
- **`tokio::sync::watch` for state.** Rejected for the same reason; it also lacks a
  `ConnectionState`-specific coalescing contract we can document and test.
- **A channel of states.** Rejected: a queue needs a bound and a loss policy
  (CODING_STANDARDS 8.8), and a slow observer would either block the owner or miss
  transitions anyway. Latest-value with documented coalescing is the honest contract.
- **Keep `SystemClock` in core behind a feature.** Rejected: any real `sleep_until`
  needs a timer or a blocking sleep, and neither belongs in core.
- **`Vec<Waker>` with only `will_wake` deduplication.** Rejected: a changed waker would
  leave the stale one parked, so a loop that polls with a fresh waker each time grows
  without bound.
- **`async-trait` or `impl Future` return types in traits.** Rejected: the first is
  banned (8.2), the second is not object safe.

## Compliance
Unit tests with hand polling (`Waker::noop()` and a counting `std::task::Wake` waker):
in `cancel.rs`,
`cancel_wakes_each_waiter_exactly_once_and_a_second_cancel_wakes_nothing`,
`cancelled_on_an_already_cancelled_token_is_ready_on_the_first_poll`,
`child_follows_parent_but_parent_does_not_follow_child`,
`parent_cancel_wakes_child_waiters_once_and_skips_independently_cancelled_children`,
`dropped_child_is_not_retained_by_its_parent`,
`polling_again_replaces_the_waker_and_the_same_waker_registers_one_entry`,
`dropping_a_pending_future_removes_its_waker_without_cancelling`,
`deep_child_chains_cancel_without_recursion`,
`a_panicking_waker_cannot_strand_descendants_or_other_waiters`,
`cancelling_on_another_thread_never_loses_a_wake_up`, the waker-`Drop` re-entrancy tests,
`debug_shows_the_flag_and_child_count_but_no_wakers`, `a_poisoned_lock_is_recovered`;
in `state.rs`, `changed_is_pending_until_set_then_ready_once_per_version`,
`two_sets_before_a_poll_coalesce_into_one_change_with_the_latest_value`,
`setting_the_same_state_still_bumps_the_version`,
`every_watcher_wakes_and_each_sees_the_change_independently`,
`repolling_replaces_the_waker_and_the_same_waker_registers_one_entry`,
`a_dropped_future_loses_no_change_and_releases_its_waker`,
`a_panicking_waker_does_not_stop_later_watchers_waking`,
`setting_on_another_thread_never_loses_a_wake_up`,
`changed_stays_pending_after_the_cell_is_dropped`; in `clock.rs`,
`clock_is_object_safe_shareable_and_send`; in `future.rs`, the slot-list tests.
`crates/stonecrab/tests/public_contract.rs` asserts `Send + Sync + 'static` for the
token, cell and watch, `Send` for `OpFuture<'static, ()>`, and polls a token and a watch
through the facade. Doctests on every public item compile and run.
`scripts/coverage-gate.sh` holds core at 100% lines including every `Poll::Pending`
arm. Enforcement of the token and of the default-deadline rule by a connection is tested
from S11b-2 onward.
