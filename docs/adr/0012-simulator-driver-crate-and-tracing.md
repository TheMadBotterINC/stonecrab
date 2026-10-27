# 0012: Simulator driver crate and tracing adoption
## Status
Proposed, drafted in slice S11b-1 on 2026-10-27; accepted when S11b-2 lands `SimDriver`
and `SimConnection`. It adds the new crate `stonecrab-driver-sim` (CODING_STANDARDS 12.6
requires an ADR for a new crate), adopts `tracing` workspace-wide (section 15 and
section 14), and records decisions D9 and D23 of docs/ALPHA_PLAN.md. It builds on ADR-0005
(coverage tiers), ADR-0007 (the `Clock` trait and where its implementations live) and
ADR-0013 (how a dependency is adopted). Only data types exist when this draft is written:
the `Driver` and `Connection` traits are slice S11a, so ADR-0001's trait rule is
untouched. S11b-2 adds its changes to the Compliance section rather than replacing it.

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
