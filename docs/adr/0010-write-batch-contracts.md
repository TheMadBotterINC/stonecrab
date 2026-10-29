# 0010: Write batch contracts
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S09 of docs/ALPHA_PLAN.md. It
records decision D3 (write at alpha) and completes the write half of ADR-0003, which
shipped `WriteOutcome` and `UnknownReason` but no batch types. Every change is additive
to `stonecrab-core` and the facade. The types are data: nothing transmits, retries or
cancels. The connection contract that executes them is S11a; the simulator that
implements `write` is S11b-2.

> Note: this ADR keeps its original crate names. Map each crate name to its module with the
> table in ADR-0018 decision 1.

## Context
ADR-0003 fixed the read side as data: a bounded `ReadRequest`, a correlated
`ReadResponse`, and per-item outcomes. For writes it fixed only the outcome enum:
confirmed, rejected, or unknown-after-transmit. The rule behind the last variant is
SPEC.md 'Errors': after an ambiguous write timeout the library exposes an unknown
outcome and does not automatically replay the write. A write changes a device, so
replaying one whose first copy may have been applied can double-apply a setpoint, a
counter increment or a valve command. The rule has to be visible in the types, not only
in a driver's behaviour.

Decision D3 sets the alpha scope: no write on the Modbus wire, write PDUs and the
upstream FC06-versus-FC16 selection after alpha, but `WriteRequest` and `WriteResponse`
land in core now so that the simulator and the contract suite can exercise the whole
path.

## Decision
1. **`WriteItem { tag: Tag, value: PlcValue }`.** A plain pair with `new`, `tag()`,
   `value()` and `into_parts()`. It validates nothing by itself; the batch builder does.
2. **`WriteRequest` and `WriteRequestBuilder`.** Obtained through
   `WriteRequest::builder(limits)` and limits-bound exactly like `ReadRequestBuilder`:
   the item budget is `ResourceLimits::max_items_per_request`. `item(tag, value)` checks,
   in this order and before any collection grows, the item budget (`ResourceLimit`), the
   value type against the tag's hint (`InvalidConfig`) and duplicate `TagId`s
   (`InvalidConfig`). The first error is sticky: later items are discarded without
   allocation and `build()` returns it. `deadline(Deadline)` sets an absolute cutoff and
   keeps an elapsed one so the connection rejects it against its own clock. `build()`
   rejects an empty batch (`InvalidConfig`). There is no cancel slot; S11a adds one to
   both request types together.
3. **Type hint versus value type.** If `Tag::value_type()` is `Some(t)` and
   `value.value_type() != t`, the item is rejected as `InvalidConfig`. `Null` is a type
   of its own and does not satisfy a typed tag. A tag with no hint accepts any value
   type; the driver validates the value against the device before transmission. The
   check is exact equality, with no widening: writing an `I16` to a `U16` tag is a
   caller error, not a conversion the library guesses at.
4. **`WriteResult { tag: TagId, outcome: WriteOutcome, status: Option<ProtocolStatus> }`.**
   The outcome is carried by value and unchanged. `status` is deliberately separate from
   the `ItemError` inside `Rejected`: a confirmed or unknown write can still carry the
   code the device returned, and a rejection's own status is not merged with it.
5. **`WriteResponse` and `WriteResponseBuilder`.** `WriteResponse::builder(&request,
   started, finished)` copies the expected identities in request order. `result(r)`
   rejects an unexpected or repeated identity (`MalformedData`) and `build()` rejects a
   missing one (`MalformedData`) and a finished-before-started interval
   (`InvalidConfig`, recorded when the builder is created). Results come back in request
   order through `results()`; `get`, `iter_confirmed`, `iter_rejected`, `iter_unknown`,
   the three counts, `started_at`, `finished_at` and `into_results` complete the
   surface. The type is `#[must_use]`. Unlike `ReadResponse` there is no per-result
   sampling time and no `split_into`: a write has no sampling instant, and splitting is
   a Modbus batching detail a driver may add later without breaking this type.
6. **No retry and no replay, as data invariants.** Neither write type has a retry count,
   an attempt number or a method that resubmits. `RetryPolicy` is documented as reads
   only (ADR-0008). `WriteOutcome::Unknown(reason)` passes through the response builder
   untouched: it is never converted into `Rejected` (which would invite a retry), never
   promoted to `Confirmed`, and never dropped. Its own iterator, `iter_unknown`, and
   `unknown_count` make it impossible to forget by accident. What to do next (read the
   value back and compare, or escalate) is the application's decision.
7. **Alpha scope (D3).** The simulator implements `write` (Confirmed, Rejected, and
   `Unknown(TimeoutAfterTransmit)` through a script step after transmit) and enforces
   `AccessPolicy::ReadOnly`. `modbus-tcp` reports `Capabilities.write = false` and
   returns `Unsupported(Operation::Write)` with no I/O. Nothing in this slice changes
   that; it supplies the data the other slices consume.
8. **`UnknownReason` and `WriteOutcome` are not touched.** `response.rs` is unchanged, so
   the `#[expect(clippy::enum_variant_names)]` and `#[expect(variant_size_differences)]`
   attributes recorded in ADR-0003 keep firing. The workspace clippy run fails with an
   unfulfilled lint expectation if a later edit makes either lint stop firing.
9. **Parity.** Rows `CORE-WRITE-REQUEST` and `CORE-WRITE-RESPONSE` in
   `docs/parity/core.md` map the types to upstream `PlcWriteRequest` and
   `PlcWriteResponse`, citing ARCHITECTURE.md section 3.3 rather than upstream files.

## Consequences
- A caller can build a write batch and a response without any driver, so the simulator
  and the contract suite share one set of contracts with the real drivers.
- A response that mixes confirmed, rejected and unknown results is ordinary data. A
  caller that only checks `build()` succeeded has not checked that its writes landed; the
  counts and iterators exist so the check is cheap.
- The strict type check means a caller must pick the exact `PlcValue` variant for a typed
  tag. This is the same exactness `PlcValue` promises everywhere (no silent widening).
- `WriteRequest` is `Clone`, so a caller can resubmit it. That is an explicit decision by
  the application, made after inspecting an unknown outcome; the library never does it.
- Request and response types are separate from the read ones. Some code is duplicated
  (builder bookkeeping); the types differ in what they correlate and what they validate,
  and the duplication is small.

## Alternatives considered
- **Reuse `ReadRequest`/`ReadResponse` with an optional value.** Rejected: a read item
  carries a sampling time and a value-or-error, a write result an outcome with a
  three-way disposition. Sharing one type would make the unknown outcome an optional
  field that is easy to ignore, and would let a read request carry a value by mistake.
- **A single `Result` per batch.** Rejected: partial failure is normal for a batch, and a
  batch-level `Err` has no way to say that item 2 was applied while item 3 is unknown.
  Collapsing to one result would also force a choice between "failed" and "maybe
  applied" for the whole batch.
- **Fold unknown into rejected with a flag.** Rejected: a rejection is safe to retry after
  correcting the cause, an unknown outcome is not. Different handling needs different
  variants (ADR-0003).
- **Widen or convert values to the tag's type.** Rejected: silent conversion is the
  precision loss `PlcValue` is designed to prevent; the driver may offer explicit
  conversions later.
- **A relative timeout on the request.** Rejected for the same reason as for reads
  (ADR-0003): core has no clock to resolve it until the connection exists (ADR-0007).

## Compliance
Unit tests in `crates/stonecrab-core/src/write_request.rs`:
`items_pair_a_tag_with_its_value_and_come_apart_without_loss`,
`requests_keep_caller_order_deadline_and_untyped_tags_accept_any_value`,
`empty_batches_are_rejected`,
`a_value_type_that_contradicts_the_tag_hint_is_invalid_config`,
`duplicate_identities_are_rejected_and_the_first_error_wins`,
`over_budget_batches_fail_before_the_collections_grow`. In
`crates/stonecrab-core/src/write_response.rs`:
`results_carry_identity_outcome_and_an_independent_status`,
`mixed_outcomes_are_restored_to_request_order_and_counted`,
`correlation_rejects_missing_unexpected_and_duplicate_results`,
`a_reversed_interval_is_invalid_and_an_empty_one_is_valid`. Consumer-side tests in
`crates/stonecrab-core/tests/write_contract.rs`:
`bounded_requests_preserve_order_values_and_explicit_deadlines`,
`a_value_type_that_contradicts_the_tag_hint_is_invalid_config`,
`empty_duplicate_and_oversized_requests_fail_before_io`,
`mixed_outcomes_restore_request_order_and_keep_unknown_unchanged`,
`every_unknown_reason_survives_correlation_unchanged`,
`an_all_rejected_batch_is_still_a_valid_correlated_response`,
`response_correlation_rejects_missing_unexpected_and_duplicate_results`,
`response_time_validation_accepts_equal_edges_and_rejects_reversal`,
`every_three_item_permutation_preserves_identity_and_its_distinct_outcome`. The two
unknown-outcome tests carry `// covers: SPEC errors / ambiguous write never replayed`.
`every_unknown_reason_survives_correlation_unchanged` constructs each `UnknownReason`
through the public API, and `mise run check` (clippy with `-D warnings`) fails with an
unfulfilled lint expectation if either `#[expect]` attribute of ADR-0003 stops firing. `crates/stonecrab/tests/public_contract.rs`
asserts `Send + Sync + 'static` for the six new public types in
`public_foundation_types_are_send_sync_and_static`. Doctests on every new public type
and constructor compile and run. `scripts/coverage-gate.sh` holds core at 100% lines.

## Addendum (S46, 2026-10-28)
Decision 7 is superseded for `modbus-tcp`, which now writes and reports
`Capabilities.write = true`; see the S46 addendum of ADR-0015.
