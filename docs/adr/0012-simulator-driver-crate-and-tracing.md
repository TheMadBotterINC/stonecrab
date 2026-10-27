# 0012: Simulator driver crate and tracing adoption
## Status
Accepted by the sole maintainer on 2026-10-27, when slice S11b-2 landed `SimDriver` and
`SimConnection` (in one commit with S11a, ADR-0011). It was proposed in slice S11b-1 on
2026-10-27. It adds the new crate `stonecrab-driver-sim` (CODING_STANDARDS 12.6 requires an
ADR for a new crate), adopts `tracing` workspace-wide (section 15 and section 14), and
records decisions D9 and D23 of docs/ALPHA_PLAN.md. It builds on ADR-0005 (coverage
tiers), ADR-0007 (the `Clock` trait and where its implementations live), ADR-0011 (the
`Driver` and `Connection` traits) and ADR-0013 (how a dependency is adopted). Decisions 9
to 14 below are the S11b-2 addendum and decision 15 is the S12 addendum (the failure
script); decisions 1 to 8 are unchanged.

## Context
The common driver suite, the sensor profiles and the facade's `sim` feature need a driver
that runs without a device or a socket (D11, CODING_STANDARDS 13.5). ADR-0007 decided that
core ships no `Clock` implementation and names `RealtimeClock` as the simulator's. The
upstream simulated driver (PLC4X `SimulatedDriver`) has an `AREA/name:TYPE` address
shape with the areas `STATE`, `RANDOM` and `STDOUT`; no source of it is inventoried, so
nothing below claims parity (`docs/parity/sim.md`). The coding standards require
`tracing` for diagnostics and deny the print macros, which rules out a `STDOUT` area as
upstream implements it. Slice S11b-1 is also the first sibling crate that matches core's
`#[non_exhaustive]` enums, so it settles how `clippy::wildcard_enum_match_arm` behaves for
them, and the first to add `tracing`, whose `std` feature reaches a crate that
`deny.toml` bans.

## Decision
1. **Published sim crate (D9).** `crates/stonecrab-driver-sim` is a published crate that
   depends on `stonecrab-core` and `tracing` only and is Tokio-free. Default features are
   empty. The facade's `sim` feature (ARCHITECTURE.md section 10) will pull only this
   crate. `proptest` and the testkit are dev-dependencies (ADR-0003, ADR-0009). Its
   `lib.rs` promotes `missing_docs`, `indexing_slicing`, `arithmetic_side_effects` and
   `as_conversions` to deny, as every driver crate does (section 5.1).
2. **`RealtimeClock` and the clock rule (D9).** `RealtimeClock` is a `std`-only `Clock`:
   `now()` is `Instant::now()`, `wall_now()` is `SystemTime::now()`, and `sleep_until`
   resolves on its first poll without moving time. The simulator is not a timer. In
   real-time use a scripted `Delay` step is therefore a no-op and a `Timeout` step returns
   `ErrorKind::Timeout` directly. `SimDriver::new()` will use it so deadlines an
   application builds from `Instant::now()` agree with the connection's clock;
   `SimDriver::with_clock(Arc<dyn Clock>)` (S11b-2) will take the testkit `FakeClock` for
   deterministic tests. Deadlines are always compared against the connection's own clock,
   so a caller who injects a clock derives its deadlines from that clock.
3. **`tracing` adoption (D23).** `[workspace.dependencies]` gains
   `tracing = { version = "0.1", default-features = false, features = ["std"] }`, resolved
   to 0.1.44 (the 0.1 series has no minor bumps, so the caret range is the whole series).
   The plan also names the `attributes` feature. It is left off, because it adds
   `tracing-attributes` and a second `syn` major (2.0.119 beside the 3.0.9 that
   `thiserror-impl` uses), and `clippy::multiple_crate_versions` then fails the
   `-D warnings` gate for the sim crate. Nothing here needs `#[instrument]`; adopting it
   later needs an `allowed-duplicate-crates = ["syn"]` entry in `clippy.toml` (not touched
   in this slice) and a line in DEPENDENCY_AUDIT.md. The crate emits `debug` events when
   a tag changes and installs no subscriber; per-operation spans (`scheme`, redacted
   `endpoint`, `op`, `request_id`) arrive with the connection in S11b-2.
   `tracing`'s `std` feature enables `tracing-core/std`, which enables `once_cell`, a crate
   `deny.toml` bans. The ban becomes
   `{ crate = "once_cell", reason = "use std lazy initialization in first-party code",
   wrappers = ["tracing-core"] }`: only `tracing-core` may depend on it. `exclude-dev` stays
   off. The D23 check on the resolved tree:

   ```text
   $ cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono
   error: package ID specification `lazy_static` did not match any packages
   $ cargo tree -e normal,dev -i once_cell
   once_cell v1.21.4
   `-- tracing-core v0.1.36
       `-- tracing v0.1.44
           `-- stonecrab-driver-sim v0.1.0 (crates/stonecrab-driver-sim)
   $ cargo tree -e normal,dev -i lazy_static
   error: package ID specification `lazy_static` did not match any packages
   $ cargo tree -e normal,dev -i log
   error: package ID specification `log` did not match any packages
   $ cargo tree -e normal,dev -i time
   error: package ID specification `time` did not match any packages
   $ cargo tree -e normal,dev -i chrono
   error: package ID specification `chrono` did not match any packages
   ```

   (Cargo stops the combined command at the first unmatched name, so each name was also
   run alone.) The only banned crate in the graph is `once_cell`, reached through
   `tracing-core`, as D23 predicted. Without the wrapper `cargo deny --locked check` exits
   2 with `crate 'once_cell = 1.21.4' is explicitly banned`; with it, the check passes
   (cargo-deny 0.20.2). `proptest`'s `std` feature adds no banned crate, as ADR-0013
   found. The normal graph of the sim crate adds `tracing` 0.1.44, `tracing-core` 0.1.36,
   `pin-project-lite` 0.2.17 and `once_cell` 1.21.4 to core's; without `attributes` no
   `syn`, `quote` or `proc-macro2` is added. DEPENDENCY_AUDIT.md has entries for
   `tracing`, `tracing-core` and `once_cell` via `tracing-core`.
4. **`SimAddress` grammar.** `STATE|RANDOM|SINK/<name>:<type>`, case-sensitive, at most 256
   bytes. The area ends at the first `/` and the type begins after the last `:`. `<type>`
   is a lowercase scalar name (`bool`, `i8`..`i64`, `u8`..`u64`, `f32`, `f64`). `<name>`
   is validated by `TagId::new`, so the simulator and `Tag` agree on what a name is.
   `STDOUT` is renamed `SINK`: the print macros are denied in library code (section 14),
   so writes to it are counted and discarded by S11b-2. Upstream's Javadoc legacy array
   form is rejected with its own message, and so are upper-case type names, unknown areas
   and types, a missing `/` or `:type`, control characters and overlong input, all as
   `ErrorKind::InvalidAddress`. `SimAddress` implements `ProtocolAddress`: protocol
   `sim`, `canonical()` equal to the accepted text, `value_type_hint()` from the type.
   It stays an address shape only: no register, function code or wire model.
5. **Device model.** `SimDevice` (all methods take `&self`; the tag map and the generator
   sit behind one `std::sync::Mutex` with poison recovery, no `unsafe`) holds up to 4096
   `SimTag { value, readable, writable }` by name, with `set`, `remove`, `get`, `tags`,
   `ramp`, `serve_read`, `random` and the stored `with_latency`, `with_max_items` and
   `with_seed` settings that S11b-2 reads (the builders are named `with_*` so that the
   getters `latency()` and `max_items()` can exist). The ramp rule: `ramp(name, start,
   step)` needs one numeric type for both; a served read returns the current value and
   then adds the step with `checked_add` (floats: while the sum is finite); when the next
   value would overflow the ramp stops and keeps serving the last value. It does not wrap
   and does not saturate at the type's maximum, because a counter that visibly stops is
   easier to diagnose in a test than one that jumps. `SimTag::with_read_counter()` returns
   an `Arc<AtomicU32>` incremented on every served read, the `std`-only equivalent of
   ARCHITECTURE.md 5.1's `read_side_effect` closure (which would need `dyn Fn` state and a
   test double of its own). `Debug` prints counts, not values. A tag that is not readable
   refuses reads (`Unsupported(Operation::Read)`), an unset name is `InvalidAddress`;
   S11b-2 maps both to per-item `ItemError`s.
6. **Seeded `RANDOM`.** Upstream draws from `SecureRandom`, which no test can reproduce.
   The simulator uses xorshift64* (the generator of the transport's `Jitter`, with
   `wrapping_mul`, so no input overflows), seeded: `SimDevice::with_seed(u64)`, fixed by
   default, set from `sim://<device>?seed=` in S11b-2. Integers take the low bytes of one
   draw, a Boolean the lowest bit, floats are uniform over `[0, 1)` (finite). A requested
   type outside the eleven scalars is `InvalidConfig`. The generator is not cryptographic
   and must never feed anything but test data.
7. **`wildcard_enum_match_arm` over foreign `#[non_exhaustive]` enums.** Settled under
   the pinned Rust 1.99.1 toolchain, with probes in the sim crate: the lint fires on a
   wildcard arm over `PlcValueType` from another crate when the `_` stands for known
   variants (`warning: wildcard matches known variants and will also match future added
   variants`, with a suggestion that ends `| _`). It does not fire when every known variant
   is named and the `_` is only the catch-all for variants added later, nor when the
   scrutinee is a tuple of enums (`match (value, step) { ... _ => None }`). The adopted
   pattern: name every known variant, including the unsupported ones, in one final
   or-pattern arm that ends with `| _`, and return a typed error from that single arm
   (`XorShift::value`: `Null | Char | String | Duration | _ => Err(InvalidConfig)`). One
   tested arm keeps the 100% line gate: a test per refused variant executes it. A new core
   variant lands in that arm silently, which is the intended failure mode (a typed error,
   not a wrong value). Ramp stepping matches a tuple, which the lint ignores, with its
   `_ => None` arm tested directly. No `#[expect]` was needed. ADR-0005 carries the same
   outcome as an addendum.
8. **Wiring.** The crate is in `members` and `default-members`;
   `packaging/stonecrab-driver-sim.txt` is the reviewed
   `cargo package --list` output; `mise run package` and the CI `package`
   job list it; `scripts/coverage-gate.sh` has the row `crates/stonecrab-driver-sim|100`;
   `scripts/lint-attrs.sh` already permitted the four promotions.

9. **`SimDriver` and the connection string (S11b-2).** Scheme `sim`; `SimDriver::new()`
   uses `RealtimeClock`, `with_clock(Arc<dyn Clock>)` takes the testkit's `FakeClock`,
   `with_device(name, SimDevice)` registers a device (a name is 1 to 64 ASCII letters,
   digits, `-`, `_` or `.`; a duplicate is `InvalidConfig`) and `device(name)` returns the
   shared handle. `parse_config` reads `sim://<device>?latency-ms=<n>&seed=<u64>` through
   `ConnectionUri`: the authority must name a registered device, `latency-ms` (at most one
   hour) and `seed` are optional, `address-convention` is accepted because every driver
   accepts that shared key and is ignored, and any other key, a transport code, another
   scheme or an unknown device is `InvalidConfig`. Upstream's `simulated:<name>` form is
   not aliased. The configuration carries `Endpoint::Custom` holding the device name, with
   `?latency-ms=..&seed=..` appended only when an option was given, because the
   configuration has no other place to carry them and `connect` reads the text back with
   `ConnectionUri`. Timeouts, retry policy and limits are the defaults; the access policy
   is `ReadWrite`, the one departure from the read-only default (ADR-0008), because a
   simulated device has nothing to protect. `parse_address` parses `SimAddress` and
   ignores `AddressOptions.convention`. `SimDriver::capabilities()` is read and write true,
   concurrency 1, subscribe `Unsupported`, no item limit (the driver does not know which
   device a connection will open); `SimConnection::capabilities()` adds the device's
   `max_items`.
10. **Lifecycle and the single slot.** `SimConnection` holds the clock, the shared device,
    the configuration's timeouts, retry policy, access policy and limits, and a
    `StateCell`. The state runs `Disconnected`, `Connecting`, `Connected`, `Closing`,
    `Closed`; `Failed` is used by the failure script (decision 15). A single
    `AtomicBool` enforces `Capabilities.concurrency` 1: a second read or write that starts while one is
    in flight fails at once with `Backpressure`, before any device access. The queue length
    is zero, there is no wait queue, and a rejected operation is never retried. A guard
    releases the flag when the operation's future ends or is dropped.
11. **Time.** Connect, read, write and close each wait once on the connection's clock for
    the device latency (the `latency-ms` option, else `SimDevice::with_latency`). The wait
    is a race between `Clock::sleep_until(min(reply time, deadline))`, the request's
    cancellation token and a connection-wide shutdown token that `close` cancels; a reply
    time at or after the deadline waits for the deadline and is `Timeout` (equality means
    elapsed), a cancelled token is `Cancelled`, and a close during the wait is
    `Disconnected`, each with no partial response and no device access. A request without
    a deadline gets `Deadline::after(clock.now(), timeouts.request)` (ADR-0007). `close`
    waits the latency, at most `timeouts.request`, and always ends in `Closed`. A device is
    read or changed only after the wait, in one synchronous step, so cancelling or dropping
    earlier leaves it untouched.
12. **Reads and writes.** Before any device access: the access policy (writes only, first
    of all), then each tag's address must be a `SimAddress` with count 1
    (`InvalidAddress`) and the batch within the device's `max_items` (`ResourceLimit`; the
    simulator never splits, so `split_into` is 1), then the single slot, then the
    connection state, token and deadline. `STATE` reads call `SimDevice::serve_read`,
    `RANDOM` reads call `SimDevice::random`, `SINK` reads are a per-item `Unsupported`
    error. An unset `STATE` name is a per-item `InvalidAddress` error with
    `ProtocolStatus::new("sim", 2)`; a tag that is not readable is a per-item
    `Unsupported(Operation::Read)` error; a stored value whose type differs from its
    address is a per-item `InvalidConfig` error. `sampled_at` is the clock reading taken per
    item, `source_time` is `None`, and `ReadResponseBuilder` is constructed after the last
    sample. A read is retried as a whole under `RetryPolicy` and only for kinds in its
    retry set, with one shared deadline; the default policy makes one attempt. A write is
    never retried: `SINK` writes are counted and `Confirmed`, `STATE` writes replace the
    value of a writable tag (`Confirmed`), and a non-writable tag, an unset name, a
    mismatched stored type and any `RANDOM` write are `Rejected` with an item error. To
    support this `SimDevice` gains `write`, `record_sink_write`/`sink_writes` and
    `reseed` (all `&self`, under the existing lock); `seed=` in the connection string
    calls `reseed` when the connection becomes `Connected`, so the generator, which is the
    device's, is restarted for every connection to it.
13. **Diagnostics and tracing.** Every `PlcError` and `ItemError` the connection produces
    carries `Diagnostics` with the operation, the connection state at the failure, the
    elapsed time on the clock, the retries made and a category. The category is
    `Validation` for a rejection before the exchange (policy, address, limit, backpressure,
    cancelled token), `Transport` for `Timeout` and `Disconnected`, `Protocol` for what the device refused
    (per-item errors) and `Internal` for the rest: a response builder that refuses what
    the connection assembled, and a cancellation while waiting (the slice text lists
    Transport for Timeout and Disconnected only). Connect, read, write and
    close open spans `connect`, `read`, `write` and `close` with `scheme`, `endpoint` (the
    device name), `op` and `request_id`, a per-connection counter from 0 (the connect), drawn eagerly for every span whether or
    not a subscriber enables it.
    Events carry counts, never values (CODING_STANDARDS 14.5).
14. **`SimArea` match arms.** `SimArea` is `#[non_exhaustive]` but is matched inside its
    own crate, where a wildcard arm is unreachable; the connection names every area.
    Decision 7's foreign-enum pattern applies to enums of other crates such as
    `PlcValueType` and `WriteOutcome`.

15. **The failure script (S12).** `SimDevice` owns a `FailureScript`, a first-in first-out
    queue of at most 1024 `#[non_exhaustive]` `FailureStep`s (`Delay`, `Timeout`,
    `Disconnect`, `DeviceException`, `Malformed`, `PartialFailure`, `Backpressure`,
    `UnknownAfterTransmit`), pushed with `push_failure`/`push_failures` and inspected with
    `pending_failures`. The script is the device's, so every connection to it draws from
    one queue in arrival order. The rules:
    - **One step per operation.** A connect, read or write takes at most one step, at the
      point where it would touch the device, which is after every pre-I/O check (access
      policy, address, item limit, the in-flight slot, connection state, token, deadline).
      A rejected request therefore leaves its step queued. `close` ignores the script.
      Each attempt of a retried read takes its own step.
    - **`Timeout` is timeout after transmit.** The device is touched (a read is served and
      counted, a write applied), the reply is lost, and the connection waits on its clock
      for the deadline, raced against the token and the shutdown, and fails the whole call
      with `Timeout` (`Transport`). This is the case the contract suite counts: a script
      of `Timeout` steps against `max_attempts` N yields N device reads. To make that true,
      a read without an explicit deadline now gets `timeouts.request` afresh for each
      attempt (the default-deadline rule of ADR-0007 is unchanged for the first attempt);
      an explicit deadline stays shared by all attempts, so after it has passed a retry
      fails at the gate without touching the device or taking a step.
    - **`Disconnect`** leaves the device untouched, sets `Failed(Disconnected)` (before
      the error is built, so its diagnostics carry that state) and fails with
      `Disconnected`. Every later operation fails with `Disconnected` at the gate and
      takes no step; `close` still ends in `Closed`. This is the only use of `Failed`.
    - **`DeviceException(status)`, `Malformed`, `PartialFailure(ids)`.** The first two
      leave the device untouched and fail every item of a read (item errors, `Protocol`).
      `DeviceException` rejects every write item (`Rejected` with the status). `Malformed`
      carries no status and fails a write as a whole with `MalformedData`, because a
      corrupted confirmation cannot be attributed to items. This is a deliberate
      deviation from the plan text "Malformed (per-item MalformedData)", which holds for
      reads only. `PartialFailure` fails the named items with
      `ProtocolStatus::new("sim", 4)` (not served, not applied) and handles the others
      normally; ids not in the request are ignored.
    - **`Backpressure`** fails with `Backpressure` (`Validation`) without touching the
      device; the connection stays open and the slot is released.
    - **`UnknownAfterTransmit`** applies a write exactly once and reports every item
      `Unknown(TimeoutAfterTransmit)` without waiting for a deadline and without a retry
      (writes are never retried); on a read it behaves like `Timeout`.
    - **`Delay(d)`** waits `d` on the clock, raced like the latency wait, then proceeds
      normally, before the latency wait. A delay that reaches the deadline is `Timeout`
      with no device access.
    - **Connect.** The connect takes one step: `Delay` delays it (there is no request
      token, so the connect deadline is the only bound), `Timeout` fails it at the connect
      deadline without touching the device, `Disconnect` fails it and leaves the dropped
      connection in `Failed(Disconnected)`. The other steps mean nothing to a connect and
      are consumed with a `tracing` warning and no effect (rejecting them at push time
      was considered and dropped: the queue does not know which operation will take a
      step). The consequence is that steps are best pushed after the connection is open.
      The plan's "cancel while connecting (Delay step)" test cannot exist as written,
      because `Driver::connect` takes no token and the connection's shutdown token is
      unreachable before the connection exists. It is replaced by two tests: a delay past
      the connect deadline is a `Timeout`, and dropping the connect future during its
      delay leaves the device alone.
    - **Diagnostics and tracing.** Every scripted failure carries the operation, the
      connection state at the failure, the latency, the retries and a category as in
      decision 13; a `debug` event names each consumed step (`step` field) and counts,
      never values.

    No new dependency; the script lives in `src/script.rs` and the device queue in
    `src/device.rs`.

## Consequences
The simulator can be built on tested data types before the traits exist, and the first
`tracing` dependency in the workspace is audited, wrapped and visible in one place. The
`once_cell` wrapper is narrow: any other crate that reaches `once_cell` fails `deny` until
it is reviewed. Omitting `attributes` keeps the `-D warnings` gate green at the cost of
`#[instrument]`; manual spans are enough for the connection. `RealtimeClock` makes the
simulator's deadlines meaningless as timeouts (nothing waits), which is why timing tests use
`FakeClock`; this is stated in the type's documentation, the README and the parity record.
Ramps stop rather than wrap, so a test cannot depend on wraparound. `SimDevice` is shared by
`Arc`, and one mutex serializes tag access and RANDOM draws, which is fine for a test
double and would not scale to a benchmark. Seeded RANDOM values are predictable by design.

## Alternatives considered
- **A testkit-only simulator.** Rejected: the testkit is unpublished and development-only,
  while the facade's `sim` feature and the examples need a published driver, and a second
  driver in the testkit would duplicate behavior (CODING_STANDARDS 13.5).
- **Tokio in the sim crate.** Rejected: nothing in a simulator needs a runtime, and
  `FakeClock` plus `block_on_with` drive it deterministically. Tokio-free keeps `sim`
  usable from any executor and out of the `tokio` feature audit.
- **An unseeded or `SecureRandom`-like RNG.** Rejected: a test cannot assert on values it
  cannot reproduce, and a random-number dependency (or `RandomState` seeding in the device)
  would add surface for no test benefit. The seed is explicit.
- **Spelling types like upstream (`INT`, `REAL`).** Rejected: the simulator speaks
  `PlcValueType`, whose names are Rust scalars; mixing IEC names would invite the byte-order
  and width questions that belong to the Modbus crate.
- **Keeping `STDOUT`.** Rejected: printing is denied in library code;
  counting and discarding is observable by a test, which printing is not.
- **Wrap or saturate on ramp overflow.** Rejected, see decision 5.
- **`#[expect(clippy::wildcard_enum_match_arm)]` on a wildcard arm.** Unnecessary: the
  or-pattern form needs no suppression, and a stale `#[expect]` would fail the build if the
  lint's behavior changes in a later toolchain, which is the same signal as the lint
  firing.
- **A helper in core that maps `PlcValueType` to an integer.** Rejected: it would add a core
  API for one consumer.
- **`attributes` now, with a `clippy.toml` allowance.** Deferred (decision 3): it edits a
  file outside this slice and buys nothing yet.

## Compliance
Tests: `address::tests::*`, `prop_sim_address_parse_never_panics`,
`prop_sim_address_canonical_round_trips` (the 13.7 list: empty, missing `/`, missing
`:type`, unknown area, unknown type, name over the `TagId` bound, overlong input, one byte
short and long of the minimal form, control characters), `device::tests::*`,
`device::random::tests::*`, `clock::tests::*` and `tests/public_types.rs`; ids are in
`docs/parity/sim.md` (`SIM-ADDRESS-GRAMMAR`, `SIM-DEVICE-MODEL`, `SIM-RANDOM-SEEDED`,
`SIM-REALTIME-CLOCK`). Coverage: `scripts/coverage-gate.sh` requires 100% lines for
`crates/stonecrab-driver-sim` (ADR-0005 tier). `mise run dependencies` and the CI
`dependencies` job enforce the `once_cell` wrapper; the D23 `cargo tree -i` check is rerun
by every slice that adds a dependency. `mise run lint-attrs` checks the four promotions.
`mise run machete` keeps `tracing` honest: it is used by the device's events.

S11b-2 tests: `crates/stonecrab-driver-sim/tests/driver_contract.rs` (through
`DriverRegistry` on the testkit's `FakeClock`): `registration_lookup_and_listing`,
`parse_returns_the_driver_and_a_configuration_without_opening_anything`,
`parse_config_rejects_everything_it_does_not_understand`,
`connect_rejects_configurations_that_are_not_sim_ones`,
`parse_address_ignores_the_convention_and_reports_bad_text`,
`the_driver_advertises_read_and_write_with_one_operation_at_a_time`,
`device_names_are_validated_and_unique`,
`the_registry_connects_reads_state_random_and_a_ramp_and_writes_state_and_sink`,
`connections_to_one_device_share_it_and_a_seed_makes_random_reproducible`,
`close_moves_through_closing_to_closed_and_is_observed_by_value`,
`operations_after_close_are_disconnected_and_close_is_idempotent`,
`close_ends_a_waiting_read_with_disconnected_and_no_device_read`,
`second_concurrent_read_fails_backpressure_without_device_access`,
`dropping_a_waiting_operation_releases_the_slot`,
`a_cancelled_token_before_transmit_is_cancelled_with_no_device_access`,
`cancelling_while_waiting_ends_the_call_with_cancelled_and_no_partial_response`,
`an_elapsed_deadline_is_a_timeout_with_no_device_access`,
`an_absent_deadline_uses_timeouts_request`,
`a_foreign_address_or_a_multi_element_tag_is_rejected_before_io`,
`the_device_item_limit_is_advertised_and_enforced`,
`an_unset_state_name_is_a_per_item_error_with_status_sim_2`,
`unreadable_write_only_and_mistyped_reads_are_per_item_errors`,
`a_read_only_policy_rejects_writes_before_any_device_access`,
`writes_the_device_refuses_are_rejected_per_item_and_change_nothing`,
`a_read_is_one_attempt_by_default_and_retries_the_whole_request_when_allowed`; the
lifecycle test `the_lifecycle_runs_through_every_state_in_order` in `src/connection.rs`;
`tests/spans.rs` `every_operation_opens_a_span_with_a_monotonic_request_id`;
`tests/readme_matrix.rs` `the_readme_support_table_agrees_with_the_driver_capabilities`
(the README-versus-`Capabilities` test that S31 reuses); and the new `SimDevice` methods'
unit tests in `src/device.rs`. Coverage stays at 100% lines for the crate.

S12 tests: `crates/stonecrab-driver-sim/tests/failure_script.rs` (every step on read and
on write, the connect cases, the retry interaction
`read_retry_counts_device_reads_up_to_max_attempts`,
`unknown_after_transmit_write_is_never_replayed`, cancellation during a delay and
`every_scripted_failure_carries_operation_state_latency_retries_and_category`),
`tests/failure_trace.rs` `every_consumed_step_is_named_in_a_debug_event`, the unit tests of
`src/script.rs` and `a_scripted_disconnect_on_connect_leaves_the_connection_failed` in
`src/connection.rs`; ids are in `docs/parity/sim.md` (`SIM-FAILURE-SCRIPT`). Coverage stays
at 100% lines for the crate.
