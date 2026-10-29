# Foundation scope
Status at the alpha (slice S31): implemented, simulator-tested. The core (the items at
the root of the `stonecrab` crate) makes no upstream
behavioral parity claim except where a row cites an upstream behavior (the connection
string grammar, `CORE-URI-GRAMMAR`, and the Modbus rows it points at); every other row
is a Stonecrab-owned contract.

The architecture maps values, tags, errors and capabilities to PLC4J concepts. Core holds
the pure foundation types, the validated request and result data, the time, cancellation
and state primitives, the connection configuration and the `Driver`, `Connection` and
`DriverRegistry` contracts (ADR-0011). Core implements none of the traits; they are
exercised through the simulator driver and, on a real transport, the Modbus driver, and
both pass the common driver contract suite. Core has the subscription types and their
bounded queue (ADR-0020) and the polling producer through which the simulator and
`modbus-tcp` subscribe (S55); it has no browse, protocol driver or sensor profile.

Each implemented contract has unit and executable documentation tests, and every row
below is checked by `scripts/parity-check.sh` (`mise run parity`): its id is named in a
`# Compatibility` section or a test, and its test-id column names real tests. The complete
upstream protocol/transport/operation inventory is still required before M0 can close.
Calendar and structured values require a further design decision (ADR-0006).

## Implemented foundation
- `Capabilities`, `Operation`, `SubscriptionMode`: explicit operation reporting.
- `PlcError`, `ErrorKind`, `Diagnostics`, `ConnectionState`, `FailureCategory`:
  boxed failures with concrete source retention and optional, never invented context.
- `TagId`: bounded, validated caller identifiers; not protocol address parsing.
- `Deadline`: deterministic explicit-time construction and evaluation.
- `ResourceLimits`: immutable finite budgets, positive values and buffer/frame
  consistency checked by a builder before allocation.
- `Secret<T>`: redacted Debug/Display; no memory-zeroization claim.
- `PlcValue`, `PlcValueType`: exact scalar widths, lossless checked integer widening
  and explicit float-only access. Byte/raw/calendar/collection values remain deferred.
- Public-API integration tests and compile-time Send/Sync assertions.
- `ProtocolAddress` and immutable `Tag`: driver-owned addresses, stable caller
  identity, type-hint consistency, positive counts and shared immutable ownership. The
  provided `ProtocolAddress::spans_multiple_words` (false by default; S39) lets a driver say
  that a value occupies several of its protocol's words, which is what makes a declared word
  order mandatory in a sensor profile (`docs/parity/sensors.md#SENSORS-VALIDATION`).
- `ReadRequest`: finite item budgets, duplicate rejection, explicit deadline metadata and
  an optional cancel slot; core transmits nothing, a connection enforces the token.
- `ProtocolStatus`, `ItemError`, `ReadItem`: original device status and bounded raw
  diagnostics, redacted Debug, source chaining and actual source-time absence.
- `ReadResponse`: exact correlation and request ordering across partial/all-item
  failure, split-batch disclosure and monotonic interval checks.
- `WriteOutcome`: confirmed, rejected and unknown-after-transmit data. The simulator
  (S11b-2) and the Modbus connection (S46) produce them; no write is ever retried.
- `SubscribeRequest`, `SubscriptionKind`, `QueuePolicy`, `Subscription`,
  `SubscriptionSender`, `LossCounters`: a bounded, policy-governed event queue with
  explicit loss counts; see "Subscriptions" and ADR-0020.

- `ByteOrder`, `WordOrder`, `Tag::with_byte_order`, `Tag::with_word_order`: explicit,
  never-inferred byte and word order as optional tag fields; additive `Ord` on
  `ErrorKind`, `Operation` and `PlcValueType`. See ADR-0006 and "Byte and word order".

- `OpFuture`, `Clock`, `CancellationToken`, `StateCell`, `StateWatch`: time, cancellation
  and state-observation primitives; see "Time, cancellation and state observation" and
  ADR-0007. Enforced by the simulator connection (ADR-0007, accepted).

- `ConnectionConfig` and its builder, `Endpoint`, `Timeouts`, `ReconnectPolicy`,
  `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions` and its builder:
  validated connection configuration as data; see "Connection configuration" and
  ADR-0008. The simulator connection enforces the timeouts, retry and access policies.

- `ConnectionUri`: the connection-string grammar `scheme(:transport)?://authority(?k=v...)?`
  with percent-encoding, credentials, duplicate keys and over-long input rejected; see
  "Connection string grammar". Fuzzed by `fuzz_core_connection_string`.

Unit tests live beside each implementation; `crates/stonecrab/tests/public_contract.rs`
checks the consumer boundary. README examples are executable doctests.
`crates/stonecrab/tests/core_read_contract.rs` uses the `testkit` feature
to exercise addresses and batch contracts without ad hoc mocks or network access.

## Byte and word order
Status: implemented as data (ADR-0006, slice S05). The types carry the setting; core
decodes nothing, and `TypedDecoder` in `stonecrab::modbus` consumes it. The Modbus mapping
and its row ids are the `MB-BYTEORDER-*` rows in `docs/parity/modbus.md`
(`MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`, `MB-BYTEORDER-BE-SWAP`, `MB-BYTEORDER-LE-SWAP`,
`MB-BYTEORDER-PRECEDENCE`), which are `implemented` there and stay below
`automated-verified` until the upstream little-endian vectors pass (the file is retained
since S04a; the S04b adapter turns its exchanges into vectors).

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-BYTEORDER-MODEL | One setting `ModbusByteOrder` with four modes; no separate word order (`docs/parity/modbus.md` "Byte order"). | `ByteOrder` x `WordOrder`, both `Copy` and `Ord`, never inferred; `Tag::with_byte_order`/`with_word_order` set optional fields that default to `None`; precedence tag field, address block, driver default. | implemented | `byte_order_is_total_and_follows_declaration_order`, `word_order_is_total_and_follows_declaration_order`, `orders_are_copy_hash_and_clone_equal`, `debug_text_names_the_variant`, `orders_default_to_absent_and_are_never_inferred`, `each_builder_sets_exactly_one_field`, `both_orders_combine_independently_of_call_order_and_last_write_wins`, `orders_do_not_disturb_other_fields_and_survive_clone_and_debug`, `facade_reexports_ordering_types_and_additive_ord` |
| CORE-VALUE-ORD | Not applicable (Java enums are comparable by ordinal). | Additive `PartialOrd, Ord` on `ErrorKind`, `Operation`, `PlcValueType` in declaration order; supports `BTreeSet<ErrorKind>`. | implemented | `error_kinds_sort_in_declaration_order_and_round_trip_through_a_set`, `operations_sort_in_declaration_order_and_round_trip_through_a_set`, `value_types_sort_in_declaration_order_stay_copy_and_round_trip_through_a_set` |

`RawValue` and calendar, bytes and list values are deferred (ADR-0006).

## Time, cancellation and state observation
Status: implemented as primitives (ADR-0007, slice S06). Request types have a cancel slot
(S11a) and the simulator connection enforces the token and applies the default-deadline
rule (S11b-2; see `CORE-CONNECTION-TRAIT`); enforcement on a real wire is the connection
guard's (S19, `TRANSPORT-GUARD-LIFECYCLE` in `docs/parity/transport.md`) and the Modbus
connection's (S25, `MB-READ-TIMEOUT`).
Upstream PLC4J cancels through `CompletableFuture` cancellation and has no
state-watch API, so there is no upstream source file to cite; the design source is
`docs/ARCHITECTURE.md` section 3.4, with the `SystemClock` placement changed by ADR-0007.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-OPFUTURE | `CompletableFuture<T>` returned by driver operations (ARCHITECTURE 3.4). | `OpFuture<'a, T>` = `Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>`; object-safe traits, no `async-trait`. | implemented | `op_future_is_send_and_clock_is_an_object_safe_send_sync_trait`, doctest `OpFuture` |
| CORE-CLOCK | None (PLC4J reads the system clock directly). | `Clock: Send + Sync` with `now`, `wall_now`, `sleep_until`; core ships no implementation, `SystemClock` is in `stonecrab::transport`, `FakeClock` in `stonecrab::testkit`, `RealtimeClock` in the simulator (ADR-0007). | implemented (trait only) | `clock_is_object_safe_shareable_and_send`, doctest `Clock` |
| CORE-CANCEL-TOKEN | `CompletableFuture.cancel`; no hierarchy. | `CancellationToken`: idempotent `cancel`, each waiter woken once, `child` follows parent and never the reverse, `Weak` children, waker re-registration with `will_wake` deduplication. Consumed by `SimConnection` through the request cancel slot (S11b-2). | implemented (primitive) | `cancel_wakes_each_waiter_exactly_once_and_a_second_cancel_wakes_nothing`, `cancelled_on_an_already_cancelled_token_is_ready_on_the_first_poll`, `child_follows_parent_but_parent_does_not_follow_child`, `parent_cancel_wakes_child_waiters_once_and_skips_independently_cancelled_children`, `dropped_child_is_not_retained_by_its_parent`, `polling_again_replaces_the_waker_and_the_same_waker_registers_one_entry`, `dropping_a_pending_future_removes_its_waker_without_cancelling`, `a_waker_woken_by_cancel_may_call_back_into_the_token_without_deadlock`, `deep_child_chains_cancel_without_recursion`, `a_panicking_waker_cannot_strand_descendants_or_other_waiters`, `cancelling_on_another_thread_never_loses_a_wake_up`, `clones_share_state_and_default_is_a_live_token`, `a_poisoned_lock_is_recovered`, `debug_shows_the_flag_and_child_count_but_no_wakers`, `facade_token_and_watch_can_be_polled_without_a_runtime` |
| CORE-STATE-WATCH | None (no state-watch API upstream). | `StateCell::set` and `StateWatch::changed`: version counter, latest-value semantics, intermediate states coalesced, a same-state `set` still bumps the version, all watchers woken. | implemented (primitive) | `current_reflects_set_on_the_cell_and_every_watch`, `changed_is_pending_until_set_then_ready_once_per_version`, `two_sets_before_a_poll_coalesce_into_one_change_with_the_latest_value`, `setting_the_same_state_still_bumps_the_version`, `every_watcher_wakes_and_each_sees_the_change_independently`, `a_clone_starts_from_the_source_seen_version_and_then_tracks_its_own`, `repolling_replaces_the_waker_and_the_same_waker_registers_one_entry`, `a_dropped_future_loses_no_change_and_releases_its_waker`, `a_panicking_waker_does_not_stop_later_watchers_waking`, `setting_on_another_thread_never_loses_a_wake_up`, `changed_stays_pending_after_the_cell_is_dropped`, `a_poisoned_lock_is_recovered`, `debug_shows_state_and_versions_but_no_wakers` |

Cancelling an operation on a real wire is not covered by these rows; see
`TRANSPORT-GUARD-LIFECYCLE` and `MB-READ-TIMEOUT`. Unknown-after-transmit handling
on a wire is `MB-TCP-WRITE` (S46): a request that went out and got no usable answer is
`WriteOutcome::Unknown`, never retried. The
default-deadline rule and cancellation of a simulated operation are rows of
`CORE-CONNECTION-TRAIT`.

## Connection configuration
Status: implemented as data and pure arithmetic (ADR-0008, slice S07). The simulator
connection (S11b-2) enforces the timeouts, the read retry policy and the access policy;
the transport (S16), the connection guard (S19) and the Modbus connection (S25) enforce
the connect and request timeouts, the reconnect policy and the read retry set. Upstream
has no driver-independent configuration type, so rows cite the Modbus `MB-CFG-*` rows in
`docs/parity/modbus.md` rather than upstream files; the design source is
`docs/ARCHITECTURE.md` section 3 as amended by decision D12.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-CONFIG-SHAPE | `DriverBase` parses the connection string into a driver-specific configuration with per-option defaults; for Modbus `request-timeout-ms` is 5000 and the other defaults are the `MB-CFG-*` rows in `docs/parity/modbus.md` (`MB-CFG-REQUEST-TIMEOUT`, `MB-CFG-UNIT-ID`, `MB-CFG-PING-ADDRESS`, `MB-CFG-BYTE-ORDER`, `MB-CFG-MAX-COILS`, `MB-CFG-MAX-REGISTERS`, `MB-CFG-UNKNOWN`). There is no connect timeout, idle timeout, access policy or credentials object in the inventoried Modbus files. | `ConnectionConfig` (private fields, accessors, by value, not `Clone`) with `Endpoint::{Tcp, Custom}`, `Timeouts` (5 s, 5 s, no idle bound; an idle bound is enforced by a supervised connection guard, `docs/parity/transport.md#TRANSPORT-RECONNECT`), `AccessPolicy` (default `ReadOnly`), `Credentials` in a `Secret`, `ResourceLimits`; a validating builder rejects an empty scheme, a zero connect or request timeout and bad reconnect values with `InvalidConfig`; `redacted_display` never contains credential text. Unknown options are rejected, not warned about (`MB-CFG-UNKNOWN`, divergence). | implemented | `defaults_match_the_documented_values`, `every_setter_is_retained_by_its_accessor`, `build_rejects_each_invalid_field_with_invalid_config`, `boundary_values_are_accepted`, `endpoint_display_brackets_ipv6_and_keeps_custom_text`, `access_policy_displays_stable_names`, `credentials_debug_never_prints_the_password`, `config_debug_hides_the_whole_credentials_value`, `redacted_display_never_contains_credential_text`, `redacted_display_has_a_fixed_shape_without_credentials`, `public_foundation_types_are_send_sync_and_static`, `facade_builds_a_configuration_and_extracts_address_options` |
| CORE-RECONNECT-POLICY | None: no reconnect or backoff in the inventoried files (`docs/parity/modbus.md#MB-READ-RECONNECT`). | `ReconnectPolicy` (250 ms, 30 s cap, multiplier 2.0, jitter 0.2, unlimited) with pure `next_delay(attempt, sample)` (capped exponential, symmetric jitter factor `1 + jitter * (2 * sample - 1)`, total for every attempt and sample, never above `max * (1 + jitter)`) and `is_exhausted`. A Stonecrab extension; no random source in core. | implemented (data only) | `backoff_doubles_from_the_initial_delay_and_caps_at_max`, `jitter_samples_reach_both_bounds_and_the_cap_bounds_the_upper_one`, `next_delay_is_total_and_bounded_for_every_attempt_and_sample`, `next_delay_survives_policies_the_builder_would_reject`, `reconnect_attempts_are_exhausted_only_at_the_limit`, doctests `ReconnectPolicy`, `next_delay`, `is_exhausted` |
| CORE-RETRY-POLICY | INFERENCE, not run: no read retry in the inventoried read path (`docs/parity/modbus.md#MB-READ-RETRY`). | `RetryPolicy` (default one attempt, `retry_on` = `{Timeout, Disconnected, Backpressure}`, equal to `PlcError::is_retryable`) with `should_retry(kind, attempt)`; reads only, writes are never retried. | implemented (data only) | `default_retry_set_equals_the_error_retryable_predicate`, `retry_stops_at_max_attempts_and_only_for_listed_kinds`, doctests `RetryPolicy`, `should_retry` |
| CORE-ADDRESS-OPTIONS | Bare-digit Modbus forms such as `40001` are accepted unconditionally; there is no convention option (`docs/parity/modbus.md#MB-CFG-UNKNOWN` covers the option-handling difference). | `AddressOptions` (`#[non_exhaustive]`, `Clone`, `Default`) carries the optional `address-convention` name; the builder bounds it to 64 bytes and rejects empty names, whitespace and control characters; `ConnectionConfig::address_options()` and `address_convention(&str)` feed it. The Modbus driver accepts `plc4x`, `one-based-40001` and `zero-based` (D16, ADR-0015 addendum S51). | implemented (data only) | `default_and_empty_builder_carry_no_convention`, `convention_round_trips_clones_and_last_call_wins`, `invalid_conventions_are_rejected_without_echoing_them`, `address_options_round_trip_and_outlive_the_config`, doctests `AddressOptions`, `AddressOptionsBuilder::build` |
| CORE-DRIVER-OPTIONS | The driver-specific configuration object that `DriverBase` builds beside the transport configuration (for Modbus the six `MB-CFG-*` parameters in `docs/parity/modbus.md`). | `DriverOptions: Any + Debug + Send + Sync` (a method-free trait the driver implements; its `Debug` must carry no secrets) stored as `Arc<dyn DriverOptions>` by `ConnectionConfigBuilder::driver_options` and read by `ConnectionConfig::driver_options()` and `driver_options_as::<T>()`; absent or foreign options read as `None` and the driver falls back to its defaults. `Debug` and `redacted_display` print the options. The simulator keeps its options in the text of `Endpoint::Custom`; a TCP driver has a structured endpoint, so it uses this slot (ADR-0008 amendment, S26). A Stonecrab extension. | implemented | `driver_options_are_absent_by_default`, `driver_options_round_trip_through_the_builder_and_downcast_to_their_own_type`, `driver_options_appear_in_debug_and_redacted_display_without_credentials`, `public_foundation_types_are_send_sync_and_static`, doctests `DriverOptions`, `driver_options`, `driver_options_as`, `ConnectionConfigBuilder::driver_options` |

The connect and request timeouts, the reconnect policy, the read retry set and the access
policy are enforced by the simulator connection (S11b-2), the guard (S19) and the Modbus
connection (S25). `Timeouts.idle` is enforced since S57 by the transport guard's
supervisor (ADR-0014 addendum S57), which the Modbus connection starts when it is opened
inside a Tokio runtime: a link that no request has used for `idle` is closed and the next
request reopens it. The simulator has no transport and ignores `idle`.

## Write batches
Status: implemented as data (ADR-0010, slice S09). The types transmit nothing: the
simulator implements `write` in S11b-2 and `modbus-tcp` puts writes on the wire in S46
(`Capabilities.write` is true there; the upstream write mapping is `MB-TCP-WRITE` in
`docs/parity/modbus.md`). Rows cite `docs/ARCHITECTURE.md` section 3.3 rather than
upstream files; the upstream types are `PlcWriteRequest` and `PlcWriteResponse`.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-WRITE-REQUEST | `PlcWriteRequest` is an ordered batch of tags with values (ARCHITECTURE 3.3). | `WriteItem { tag, value }` and `WriteRequest` built by `WriteRequestBuilder` (limits-bound like `ReadRequestBuilder`): the item budget, a value type that contradicts the tag's type hint, and duplicate `TagId`s are rejected as sticky errors before any collection grows; an empty batch is rejected; the absolute deadline is kept as given. No retry state; an optional cancel slot (`cancel(token)`, S11a). | implemented | `items_pair_a_tag_with_its_value_and_come_apart_without_loss`, `requests_keep_caller_order_deadline_and_untyped_tags_accept_any_value`, `empty_batches_are_rejected`, `a_value_type_that_contradicts_the_tag_hint_is_invalid_config`, `duplicate_identities_are_rejected_and_the_first_error_wins`, `over_budget_batches_fail_before_the_collections_grow`, `bounded_requests_preserve_order_values_and_explicit_deadlines`, `empty_duplicate_and_oversized_requests_fail_before_io` |
| CORE-WRITE-RESPONSE | `PlcWriteResponse` maps each tag to a response code; an ambiguous timeout has no distinct result (ARCHITECTURE 3.3). | `WriteResponse` built by `WriteResponseBuilder`: exactly one `WriteResult { tag, outcome, status }` per requested identity, restored to request order; missing, duplicate and unexpected identities and a finished-before-started interval are rejected; `iter_confirmed`, `iter_rejected`, `iter_unknown` and counts. `WriteOutcome::Unknown(reason)` survives correlation unchanged and is never collapsed into a rejection or replayed (divergence: Stonecrab addition). | implemented | `results_carry_identity_outcome_and_an_independent_status`, `mixed_outcomes_are_restored_to_request_order_and_counted`, `correlation_rejects_missing_unexpected_and_duplicate_results`, `a_reversed_interval_is_invalid_and_an_empty_one_is_valid`, `mixed_outcomes_restore_request_order_and_keep_unknown_unchanged`, `every_unknown_reason_survives_correlation_unchanged`, `an_all_rejected_batch_is_still_a_valid_correlated_response`, `response_correlation_rejects_missing_unexpected_and_duplicate_results`, `response_time_validation_accepts_equal_edges_and_rejects_reversal`, `every_three_item_permutation_preserves_identity_and_its_distinct_outcome`, `public_foundation_types_are_send_sync_and_static` |

## Driver, connection and registry
Status: implemented (ADR-0011, slice S11a, landed in one commit with the simulator driver
of S11b-2 because ADR-0001 forbids a public driver trait that nothing exercises). The
upstream types are `PlcDriver`, `PlcConnection` and `PlcDriverManager`; the Stonecrab
boundary follows `docs/ARCHITECTURE.md` section 3.1 with the differences listed in its
"Shipped shape" note. No upstream API-shape finding is recorded in `docs/parity/modbus.md`,
so the "Upstream behavior" column cites the architecture document and names only the
upstream type. Core implements none of the traits (CODING_STANDARDS 13.5); the behavioral
tests are the simulator's, through `DriverRegistry`, and the Modbus driver's over its
contract-suite fixtures.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-DRIVER-TRAIT | `PlcDriver` creates connections from a connection string and exposes capability flags (`canRead` and similar); ARCHITECTURE 3.1. | `Driver: Send + Sync`, object safe: `scheme`, `capabilities` (by value, `Capabilities` is `Copy`), `parse_config(&str)`, `parse_address(&str, &AddressOptions) -> Arc<dyn ProtocolAddress>` and `connect(ConnectionConfig) -> OpFuture<Box<dyn Connection>>`. A stateless factory: all per-connection state lives behind `Connection`. `connect` is bounded by `timeouts.connect`. Divergences: `Arc` addresses and the options argument (D16); explicit `Capabilities` instead of flags. | implemented | `driver_and_connection_are_object_safe_send_sync_traits`, `the_driver_advertises_read_and_write_with_one_operation_at_a_time`, `parse_config_rejects_everything_it_does_not_understand`, `parse_address_ignores_the_convention_and_reports_bad_text`, `connect_rejects_configurations_that_are_not_sim_ones`, `connect_waits_the_latency_and_is_bounded_by_the_connect_timeout`, doctest `Driver` |
| CORE-CONNECTION-TRAIT | `PlcConnection` executes read and write requests and reports which operations it supports; subscribe, browse and ping exist upstream; ARCHITECTURE 3.1. | `Connection: Send + Sync`, object safe, the narrow set (D2): `state`, `watch_state`, `capabilities` and `limits` (both by value, `Copy`), `read`, `write`, `close`, plus the provided `ping` (S40, ADR-0011 addendum): it takes no request, like `PlcConnection.ping()`, performs no I/O by default and fails with `Unsupported(Operation::Ping)` carrying `Diagnostics` (operation, connection state, `Validation`); `Operation::Ping` and `Capabilities.ping` (default false) are its capability, and an override bounds its attempt by `timeouts.request` (`ModbusConnection` is the first, S47); each `OpFuture` method documents `# Cancel safety` and `# Wire effects`. The default-deadline rule (an absent deadline becomes `Deadline::after(clock.now(), timeouts.request)`), cancellation through the request slot, and `close` bounded by `timeouts.request` are part of the contract. The provided `subscribe(SubscribeRequest) -> OpFuture<Subscription>` (S54, ADR-0020) follows the same shape: by default it performs no I/O, ignores the request's deadline and token, and fails with `Unsupported(Operation::Subscribe)` carrying the same `Diagnostics`; an override sets `Capabilities.subscribe` and follows the read rules for the call. Browse and discover are not declared (deferred); they will be provided methods returning `Unsupported`, as `ping` and `subscribe` are. | implemented | `driver_and_connection_are_object_safe_send_sync_traits`, `the_registry_connects_reads_state_random_and_a_ramp_and_writes_state_and_sink`, `an_absent_deadline_uses_timeouts_request`, `close_is_bounded_by_the_request_timeout`, `second_concurrent_read_fails_backpressure_without_device_access`, `a_cancelled_token_before_transmit_is_cancelled_with_no_device_access`, `cancelling_while_waiting_ends_the_call_with_cancelled_and_no_partial_response`, `the_cancel_slot_is_absent_by_default_and_shares_the_callers_token`, `an_already_cancelled_token_is_retained_for_the_connection_to_reject`, `operations_sort_in_declaration_order_and_round_trip_through_a_set`, `capabilities_report_each_operation_without_enabling_others`, contract case `an unsupported ping performs no I/O and changes no state` (runs for the simulator; skipped by name for `modbus-tcp`, which overrides `ping` since S47, `MB-TCP-PING`), contract cases `a supported ping answers and changes no state` and `a ping to a silent device yields Timeout` (S47; run for `modbus-tcp`, skipped by name for the simulator), contract case `an unsupported subscribe performs no I/O and changes no state` (S54; one request per queue policy; since S55 it runs for the testkit's driver without capabilities and is skipped by name for the simulator and `modbus-tcp`, which override `subscribe`), `driver_and_connection_are_object_safe_send_sync_traits` (also `OpFuture<Subscription>` is `Send`), doctest `Connection`, doctest `Connection::ping` and doctest `Connection::subscribe` |
| CORE-REGISTRY | `PlcDriverManager` resolves a connection string to a driver by protocol code, discovering drivers with a service loader; an unknown protocol is a connection exception; ARCHITECTURE 3.1. | `DriverRegistry`: `new`, `register(Arc<dyn Driver>)` (`&mut self`, setup-time only; a duplicate scheme is `InvalidConfig`), `driver`, `schemes` (sorted), `parse` (returns the driver and the `ConnectionConfig`) and `connect`. The scheme is extracted with `ConnectionUri::parse` and the original text goes to `Driver::parse_config`; an unknown scheme is `Unsupported(Operation::Connect)`. Divergences: registration is explicit (no service loader, no global registry), the map has no lock because it is immutable after setup. | implemented | `registration_lookup_and_listing`, `a_duplicate_scheme_is_invalid_config_and_keeps_the_first_driver`, `schemes_are_listed_sorted_whatever_the_registration_order`, `an_unknown_scheme_is_unsupported_connect_from_parse_and_connect`, `registry_errors_do_not_invent_a_connection_state_or_latency`, `a_malformed_connection_string_is_invalid_config_from_parse_and_connect`, `parse_returns_the_driver_and_a_configuration_without_opening_anything`, doctests `DriverRegistry`, `DriverRegistry::parse`, `DriverRegistry::connect` |

## Subscriptions
Status: implemented as types and a bounded queue (ADR-0020, slice S54, findings G07 and
G10), and as polling subscriptions in the simulator and `modbus-tcp` (slice S55, ADR-0020
addendum; `MB-TCP-SUBSCRIBE` in `docs/parity/modbus.md`, `SIM-SUBSCRIBE` in
`docs/parity/sim.md`). Both report `SubscriptionMode::Polling`; the provided
`Connection::subscribe` still answers `Unsupported(Operation::Subscribe)` for a connection
that does not override it. The upstream types are `PlcSubscriptionRequest`,
`PlcSubscriptionType`, `PlcSubscriptionResponse`, `PlcSubscriptionHandle`,
`PlcConsumerRegistration` and `PlcSubscriptionEvent` (`upstream/manifest.toml`
`[inventory.api]`), and the polling helper `[inventory.util_subscription_emulation]`.

| Row id | Upstream behavior | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-SUBSCRIPTION | `PlcSubscriptionRequest` adds tags one at a time, each with a `PlcSubscriptionType` (CYCLIC with a polling interval, CHANGE_OF_STATE with an optional minimum interval, EVENT); the response hands out one handle per tag, consumers are registered on handles and called on the producer's thread, and unsubscribing is a separate request. Events are a `PlcReadResponse` plus a timestamp. No queue and no loss count; the polling helper ignores the change-of-state minimum interval and polls it every 1000 ms. | `SubscribeRequest::builder(limits, kind, queue)` (`tag`, `deadline`, `cancel`, `build`, limits-bound like `ReadRequestBuilder`) with one `SubscriptionKind` (`Cyclic(Duration)`, `ChangeOfState`, `Event`) for the whole request and a mandatory `QueuePolicy` (`Backpressure`, `CoalesceLatest`, `DropOldest`); `build` refuses an empty request, duplicates, the item budget, a zero cyclic interval and a coalescing request with more tags than `max_queued_events`. `SubscriptionSender::polling(interval, queue, limits)` makes the producer and the `Subscription`, whose queue holds at most `ResourceLimits::max_queued_events` `ReadItem`s (G10). Backpressure makes `send` wait; coalescing keeps one pending event per tag, the latest, and counts replaced ones in `LossCounters::coalesced`; dropping discards the oldest and counts it in `LossCounters::dropped`. `Subscription` discloses `mode()` and `interval()`, reads `loss()`, delivers with `recv()` (`None` after the producer ended and the queue drained) and ends with `unsubscribe()` (final counts) or drop; `send` then fails with `Cancelled` and `is_closed()` is true. Since S55 a polling driver's `Subscription` also owns its producer future and polls it from `recv` while the queue is empty (nothing is spawned; dropping the subscription drops the producer); `recv` takes `&mut self`, so only one can be pending, and a producer that panicked is dropped, never polled again; and `LossCounters::skipped` counts ticks that came due and were not sampled; `Debug` says whether the subscription polls. Divergences: one kind per request, one handle per subscription, a pull-based queue instead of consumer callbacks, declared loss, sampling while the consumer waits instead of on a thread of its own. | implemented (types, queue and the polling producer, S55) | `backpressure_holds_a_full_queue_until_the_consumer_takes_an_event`, `a_producer_waiting_on_backpressure_learns_that_the_subscription_ended`, `dropping_a_waiting_send_or_recv_releases_its_waker_slot`, `coalescing_replaces_the_pending_event_of_a_tag_in_place_and_counts_it`, `coalescing_with_more_tags_than_room_drops_the_oldest_and_counts_it`, `dropping_discards_the_oldest_event_and_counts_each_one`, `an_event_and_the_end_of_the_producer_each_wake_a_waiting_consumer`, `the_end_of_the_producer_delivers_the_queue_then_none_forever`, `unsubscribe_returns_the_final_counts_discards_the_queue_and_refuses_the_producer`, `both_ends_disclose_their_shape_in_debug`, properties `backpressure_delivers_every_event_in_order`, `dropping_accounts_for_every_event_and_keeps_order` and `coalescing_keeps_the_latest_value_per_tag_and_accounts_for_every_event`, `requests_keep_caller_order_kind_policy_deadline_and_token`, `empty_duplicate_and_over_budget_requests_are_rejected_with_the_first_error`, `a_zero_cyclic_interval_is_invalid_config`, `coalescing_needs_room_for_one_event_per_tag`, `facade_subscription_queue_can_be_polled_without_a_runtime`, `public_foundation_types_are_send_sync_and_static`, contract case `an unsupported subscribe performs no I/O and changes no state`, polling tests `a_plan_takes_the_interval_of_its_kind_and_refuses_an_event`, `a_cyclic_subscription_delivers_every_reading_one_tick_apart`, `change_of_state_delivers_the_first_outcome_each_change_and_a_failure_once`, `change_of_state_delivers_a_failure_again_when_its_kind_changes`, `a_slow_read_skips_the_ticks_it_overran_and_the_next_read_waits_for_its_tick`, `a_consumer_that_comes_late_gets_one_sample_and_the_missed_ticks_are_counted`, `a_read_refused_for_the_busy_slot_or_a_link_being_reopened_skips_its_tick`, `the_end_of_the_connection_ends_the_subscription_after_the_queued_events`, `a_dropped_connection_or_an_endless_interval_ends_the_subscription`, `unsubscribing_drops_the_producer_and_stops_the_reads`, `ticks_count_the_grid_points_after_the_start_up_to_now`, `change_of_state_is_refused_on_a_clock_that_never_sleeps`, `a_read_that_panics_ends_the_subscription_and_no_drop_panics_again`, `dropping_a_recv_mid_read_cancels_the_read_and_frees_the_slot`, `a_slow_consumer_meets_the_queue_policy_only_when_a_sample_outgrows_the_queue` (`src/polling.rs`), doctests `Subscription`, `Subscription::recv`, `SubscriptionSender::polling`, `SubscriptionSender::send`, `SubscribeRequest::builder` |

## Connection string grammar
Status: implemented as a parser and value type (slice S08). `ConnectionUri::parse` only
splits and validates; drivers (S11b-1, S20) give the parts meaning. The grammar is a
strict subset of upstream's: every rejection beyond the regular expression is listed in
the Stonecrab column below and in the `ConnectionUri` rustdoc.

| Row id | Upstream behavior (file:line) | Stonecrab behavior | Status | Test ids |
| --- | --- | --- | --- | --- |
| CORE-URI-GRAMMAR | `DriverBase.URI_PATTERN` is `^(?<protocolCode>[a-z0-9\-]*)(:(?<transportCode>[a-z0-9\-]*))?://(?<transportConfig>[^?]*)(\?(?<paramString>.*))?` (`plc4j/spi/drivers/src/main/java/org/apache/plc4x/java/spi/drivers/DriverBase.java:76-77`), applied with `matches()` at `DriverBase.java:149-150`. Protocol code, optional transport code, `://`, transport configuration up to the first `?`, then the raw parameter string. | `ConnectionUri::parse(&str)` for `scheme(:transport)?://authority(?k=v(&k=v)*)?`. Scheme and transport are non-empty `[a-z0-9-]+` (the empty forms upstream admits are rejected). The authority is everything up to the first `?`, non-empty. Options are `k=v` pairs split on `&`, kept in written order in a `Vec` (order cannot matter to a lookup by key because duplicates are rejected; it keeps `Display` faithful), the key ends at the first `=`, an empty value (`k=`) is allowed because upstream's `.*` parameter string admits it. Deliberate differences, all rejections with `InvalidConfig` whose message never echoes the authority or query: percent-encoding (any `%`; upstream URL-decodes every option value, `plc4j/spi/config/src/main/java/org/apache/plc4x/java/spi/config/ConfigurationFactory.java:132-136`, so `%3A` in a value is accepted there); credentials in the authority (an `@` before its first `/`; upstream passes the authority through, use `Credentials`); an empty parameter string (`...?`), empty pairs (`&&`, a trailing `&`), a pair without `=`, an empty key, a repeated key (compared exactly, case-sensitive like upstream); control characters and whitespace anywhere; input over 4096 bytes (checked first). Parsing never panics. `Display` renders the canonical text and parses back to an equal value. Redacted display never contains credential text. | implemented | `prop_uri_round_trips_valid_parts`, `prop_uri_parse_is_total_and_stable`, `prop_redacted_display_never_contains_password`, `uri_empty_input_is_rejected`, `uri_forms_one_byte_short_are_rejected`, `uri_minimal_form_is_accepted`, `uri_trailing_separators_are_rejected`, `uri_over_4096_bytes_is_rejected`, `uri_invalid_scheme_and_transport_are_rejected`, `uri_percent_encoding_is_rejected`, `uri_credentials_in_authority_are_rejected`, `uri_duplicate_key_is_rejected`, `uri_empty_key_is_rejected`, `uri_pair_without_equals_is_rejected`, `uri_control_characters_and_whitespace_are_rejected`, `uri_options_keep_order_values_and_empty_values`, `uri_authority_stops_at_the_first_question_mark_only`, `uri_display_parses_back_to_an_equal_value`, doctests `ConnectionUri`, `parse`, `option`, `fuzz_core_connection_string`

Baseline: Apache PLC4X v1.0.0, commit `ea229918f5d07700e04d7d69ab1645da679cb700`
(`upstream/manifest.toml`). The Modbus use of the grammar, including transport codes and
default ports, is row `MB-TCP-TRANSPORTS` in `docs/parity/modbus.md`.
