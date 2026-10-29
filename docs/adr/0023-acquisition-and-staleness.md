# 0023: Acquisition and staleness
## Status
Accepted (2026-10-28), as slice S56 of docs/POST_ALPHA_PLAN.md (finding G09:
"`Quality::Stale` exists but nothing produces it; `stale_after` is parsed and unused"). It
builds on ADR-0016 (the sensor profile schema, which already parses `min_interval_ms` and
`stale_after_ms`), ADR-0007 (the injected `Clock`) and ADR-0020 with its S55 addendum
(polling subscriptions, whose no-task design it follows). Every change is additive to the
published `stonecrab` 0.2.0.

## Context
SPEC.md ("Values and measurements"): "Use a monotonic clock for deadlines and freshness.
Do not invent a device timestamp from local receipt time. A configured stale threshold
must mark retained values as stale after missed acquisition; it must not silently publish
the previous value as a new good reading." The sensor profile contract asks for a "minimum
recommended acquisition interval and stale threshold", and SPEC.md lists "stale last-known
values" among the mandatory failure cases. `MeasurementDef` has carried `min_interval` and
`stale_after` since S28a, and `Quality::Stale` since S29, but nothing reads the first two
or produces the third. `SensorProfile::measurements` turns one `ReadResponse` into
`Measurement`s whose `received_mono` is the driver's sampling instant; one response cannot
say how old a value is.

S55 gave the simulator and `modbus-tcp` polling subscriptions: a `Subscription` drives its
own producer from `recv`, with no spawned task (CODING_STANDARDS 8.1, 8.7), on a grid of
one interval per subscription, delivering one `ReadItem` per tag. Upstream PLC4X has no
sensor profile, no quality model and no staleness (docs/parity/sensors.md), so there is no
behavior to match.

## Decision
1. **`StaleTracker`, pure logic.** `stonecrab::sensors::StaleTracker::new(&SensorProfile)`
   keeps one slot per measurement definition (its name and `stale_after`). It has no clock
   and performs no I/O: `record(Measurement)` retains a measurement, `expire(now)` marks
   and returns the transitions, `next_expiry()` says when the next one is due. The caller
   passes the monotonic reading of the clock its driver samples with; a measurement's age
   is `now - received_mono` (saturating, so a measurement from the future is age zero).
2. **The threshold is exclusive.** A retained measurement is stale when its age is
   strictly greater than `stale_after` (the profile's word is "after"): at exactly
   `stale_after` it is fresh, and it is stale from `received_mono + stale_after + 1 ns`
   on, which is what `next_expiry` returns. A definition without `stale_after` is never
   stale.
3. **What staleness changes.** `expire` sets `quality` to `Quality::Stale`, whatever it
   was, and returns each measurement once, at its transition. Nothing else changes: the
   value, the raw value, `received_at`, `received_mono`, `source_time` and
   `protocol_status` stay as read, so a retained value is never stamped as a new reading.
4. **The earlier quality is recoverable** through a new public field,
   `Measurement::quality_before_stale: Option<Quality>`: `None` while the measurement is
   not stale (the pipeline always leaves it `None`), the quality it was read with once the
   tracker has marked it. `Measurement` is `#[non_exhaustive]`, so the field is additive.
5. **A failed acquisition does not erase.** `record` retains only a measurement that
   carries a value. A `Bad` measurement without one (its item failed or its scaling
   failed) is handed to the caller but not retained, so the value read before keeps aging
   and goes stale; a read that failed as a whole produces no measurement at all. A
   measurement whose name the profile does not define is not retained. The next
   measurement with a value replaces the retained one, stale or not, and is fresh.
6. **`Acquisition`, the loop.** `Acquisition::new(&profile, &dyn Driver, &dyn Connection,
   &dyn Clock, interval)` borrows what it uses; `recv()` returns an `OpFuture` of
   `Option<Measurement>`. Fresh measurements and stale transitions come through the same
   call: a fresh measurement is never `Stale` (the pipeline cannot produce it), so the
   quality tells them apart and no event enum is needed.
7. **Intervals: a faster rate is raised, not refused.** Each measurement is read every
   `max(interval, min_interval)`. The caller's `interval` is the rate it wants; a profile
   whose measurements recommend different minimums still works with one rate, and a rate
   below a minimum would otherwise be refused for the whole profile. `interval` must be
   greater than zero (`InvalidConfig`, as `Cyclic(Duration::ZERO)` is for a subscription).
   After a read starts, each measurement it covered is next due one interval after that
   start, so successive reads of one measurement start at least its interval apart, even
   when a read was late; a read that comes due while another runs waits for it and is
   never made up later. An interval whose next read would lie past the end of time reads
   once.
8. **Grouping.** The first `recv` reads every measurement. Afterwards, every measurement
   due at the instant the loop wakes is read in one `ReadRequest` (with its status-field
   tags), through `Connection::read` with no deadline or token, so the connection's
   default deadline, retry policy and batching apply (the Modbus driver then merges
   adjacent registers on the wire). `new` builds the read of every tag once against
   `connection.limits()` and fails with what `ReadRequestBuilder::build` refuses, so no
   later group can exceed the limits; a profile without measurements is `InvalidConfig`.
   The response becomes measurements through `SensorProfile::measurements` with the
   clock's `wall_now`.
9. **Failures.** A read that fails as a whole while the connection is still open (a
   timeout, a busy in-flight slot, a link being re-established) is returned from `recv` as
   the error, with its kind and diagnostics; the loop goes on at the next call, and the
   values of that read keep aging. Per-item failures are `Bad` measurements, as the
   pipeline makes them.
10. **No task, a bounded queue.** As S55's `Subscription::recv`, the loop runs only while
    the caller awaits `recv`: it expires, reads and sleeps on the injected clock inside
    that future, and nothing is spawned (CODING_STANDARDS 8.7). Its queue is refilled only
    when empty, by the measurements of one read or the transitions of one instant, so it
    holds at most one event per measurement of the profile (CODING_STANDARDS 8.8); a slow
    consumer delays the next read and never grows the queue. At one instant the
    transitions come before the read. `recv` is cancel-safe for the queue; dropping it
    during a read drops that read, whose measurements are lost and next due one interval
    after it started.
11. **The end.** The loop watches `Connection::watch_state` while it sleeps. When the
    connection reports `Closing`, `Closed` or `Failed` (while waiting, or after a read
    that failed), `recv` delivers what is queued and then resolves to `None` from then on,
    with no error and no further stale transitions, as a subscription ends.
12. **The clock must sleep.** The loop needs a clock that waits. On a clock whose sleep
    resolves before its deadline (the simulator's `RealtimeClock`) it could not honor
    `min_interval` and would spin, so `recv` fails with `InvalidConfig` when such a sleep
    resolves early. The first acquisition needs no wait and still succeeds on such a
    clock.

**Not built on S55's subscriptions.** A polling subscription has one interval and delivers
`ReadItem`s one by one. Measurements with different intervals would need one subscription
per interval; their reads would compete for the connection's single in-flight slot (a busy
slot skips a tick) instead of sharing one request when due together, the items of one read
would have to be reassembled into a `ReadResponse` (status-field items included) before
`SensorProfile::measurements` could run, and a subscription's grid catches a late tick up
with less than one interval since the previous read. The loop is about 120 lines over
`Connection::read`; it shares S55's design (no task, work inside the consumer's future, a
watch on the connection state) but no code, because `crate::polling` is private to the
drivers and built around the subscription queue.

## Consequences
- G09 is resolved: `stale_after` and `min_interval` are read, and `Quality::Stale` is
  produced, by the tracker alone.
- An application can use the tracker without the loop (its own reads, its own executor) or
  the loop without knowing about the tracker.
- The loop's pace depends on the consumer: a consumer that does not await `recv` reads
  nothing and sees no transitions until it does; when it calls again, everything that came
  due is handled at once (the transitions first). Missed reads are not made up.
- `Acquisition` borrows its profile, connection and clock, so it lives inside the scope
  that owns them; it is `Send + Sync`.
- The clock passed to the loop must be the connection's own: `received_mono` comes from
  the driver's clock and the age from the loop's. Mixing two clocks is the logic error
  ADR-0007 already names.
- A whole-read failure surfaces as an error, not as `Bad` measurements, so the caller
  keeps its kind and diagnostics; a caller that wants a `Bad` per measurement maps it.

## Alternatives considered
- **An inclusive threshold (stale at `age >= stale_after`).** Rejected: "stale after"
  reads as strictly after, and a measurement read every `stale_after` on an ideal clock
  would go stale at the instant its next read is due. The cost is the `+ 1 ns` in
  `next_expiry`.
- **Keeping the earlier quality only inside the tracker** (a lookup by name). Rejected:
  the transition the loop hands out would lose it, and a consumer would need the tracker,
  which the loop owns. A field on the measurement travels with it.
- **Refusing a rate faster than `min_interval`** (`InvalidConfig`). Rejected: a single
  rate could not serve a profile whose measurements recommend different minimums, and the
  minimum is a recommendation the loop can honor by itself.
- **An event enum** (`Fresh`, `Stale`, `Failed`). Rejected until a third kind needs it:
  the quality already separates the two, and a failed read is the `Err` of `recv`.
- **Turning a whole-read failure into `Bad` measurements**, as S55 turns it into item
  errors. Rejected: `Measurement` has no place for the error, so the kind and diagnostics
  would be lost; the retained values still go stale.
- **A spawned task owning the loop.** Rejected for the reasons of ADR-0020's S55 addendum:
  the core has no executor and the simulator no runtime.
- **Pacing on the subscription's grid.** Rejected: a late read followed by the next grid
  point reads a measurement twice within less than its `min_interval`.

## Compliance
`crates/stonecrab/tests/sensors_acquisition.rs` runs on the testkit's `FakeClock`, with no
real sleep: the tracker alone (fresh at exactly `stale_after` and stale one nanosecond
later, each transition once; a stale value keeps everything it was read with and its
earlier quality; a reading without a value or of another profile erases nothing; a fresh
reading replaces a stale one and a measurement without a threshold never goes stale), and
the loop on the simulator (the first acquisition reads everything in one request; a faster
rate is raised to `min_interval`; measurements with different intervals, grouped when due
together; fresh to stale just past `stale_after` and cleared by the next good read; failed
reads, whole and per item, leave the retained value aging to stale; closing the connection
while the loop waits ends it; a connection failing during a read ends it; a clock that
never sleeps is refused when the loop would wait; the constructor's refusals) and on
`modbus-tcp` over the fake transport (due measurements share one wire request; a profile
larger than one read is refused before any I/O). `tests/sensors_public_types.rs` lists
`StaleTracker` and `Acquisition` as `Send + Sync`. The parity rows are `SENSORS-STALE` and
`SENSORS-ACQUISITION` in `docs/parity/sensors.md`. `scripts/coverage-gate.sh` holds the
sensors component at 100% lines.
