# Acquisition branch integration

The `feat/sensors-continuous-acquisition` branch started at the 0.2.0 baseline.
Before landing, master advanced to 0.3.0 and implemented acquisition in ADR-0023,
subscriptions in ADR-0020/0021, and the event pump in ADR-0031. The branch is squash
merged against master `0f6eb88`, preserving its published APIs and newer protocol work.

The finite profile example reads through the existing `sensors::Acquisition::recv`
boundary and reports the existing measurement envelope. It shares the synthetic bench
with `profile_read`; a virtual-day test exercises the same published acquisition API.
No new library abstraction, runtime dependency or compatibility shim is introduced.

Integration changes:

- Keep master's sensor implementations, exports, parity records, package list and
  architectural decisions. Discard the branch's alternate Acquisition/StaleTracker,
  delivery types and conflicting ADR-0019. Their original commits remain on the branch.
- Adapt `examples/profile_poll.rs` to the existing acquisition API, optional units,
  strictly-greater stale threshold and caller-driven backpressure. Extract only the
  already-existing bench fixture into `examples/support/bench.rs` for the two consumers.
- Port the virtual 24-hour test into `tests/sensors_acquisition.rs`, reusing its Rig.
- Add the finite example to Cargo's example list, simulator CI/mise examples and the
  existing pymodbus profile step. Extend the sensor usage and commissioning documents.
- Keep current README/status/version statements. Record this integration and its actual
  verification separately; do not carry obsolete 0.2.0 claims into current master.

Run the full quality task, coverage, existing sensor tests, finite profiles against
simulator and pymodbus, parity/lint checks and packaging before pushing. Native
subscription queues retain their existing loss counters; the profile acquisition loop
is demand-driven and does not expose the discarded branch's queue-policy knobs or
missed-tick counters. Do not claim stale notification during an in-flight read: the
published profile loop awaits the read before it can deliver subsequent events.
No hardware is available, so physical verification and verified device profiles remain
pending. Virtual time is not a wall-clock endurance run.
