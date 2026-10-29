# 0008: Connection configuration and address options
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S07 of docs/ALPHA_PLAN.md.
It records decisions D12 (the `ConnectionConfig` shape), D13 (the jitter and retry-set
half; the `Ord` derives were ADR-0006) and D16 (the `AddressOptions` carrier only; the
Modbus grammar and its use of the carrier are ADR-0015). Every change is additive to
`stonecrab-core` and the facade. Nothing enforces the policies yet: the types are data
and pure arithmetic, consumed from S11a (`Driver`), S16 (transport) and S19 onward.

> Note: this ADR keeps its original crate names. Map each crate name to its module with the
> table in ADR-0018 decision 1.

## Context
Every driver needs the same connection inputs: a scheme, an endpoint, time bounds, a
reconnect and a read retry policy, whether writes are allowed, optional credentials and
resource budgets. ARCHITECTURE.md section 3 draws them as one `ConnectionConfig`, and
the plan fixes the shape in D12. Three constraints shaped the details.

- `Secret<T>` is deliberately not `Clone` (secret.rs), so a configuration that holds
  credentials cannot be `Clone` either.
- Core denies `as_conversions` (ADR-0006), so a bitset of error kinds keyed by
  discriminant is not expressible, and core has no timer and no random source
  (ADR-0007), so the backoff cannot sleep or roll dice itself.
- Drivers are stateless factories (ARCHITECTURE.md section 3.1), yet Modbus must accept
  the ambiguous bare-digit address forms only when the caller named a convention (D16).
  The choice has to travel with the address text, and it must survive `connect` taking
  the configuration by value.

## Decision
1. **`ConnectionConfig` (D12).** Private fields and accessors: `scheme() -> &str`,
   `endpoint()`, `timeouts()`, `reconnect()`, `retry()`, `access()`, `credentials() ->
   Option<&Secret<Credentials>>`, `limits()`, `address_options()` and
   `redacted_display()`. It is built by `ConnectionConfig::builder(scheme, endpoint)`
   and validated once, in `ConnectionConfigBuilder::build`.
2. **Not `Clone`; passed by value.** The configuration holds
   `credentials: Option<Secret<Credentials>>`, `Secret` is not `Clone`, so
   `ConnectionConfig` inherits that and `Driver::connect` takes it by value (S11a). We
   did not drop the credentials to regain `Clone` (the plan's rejected alternative):
   authentication is part of the connection shape, not an afterthought. A caller that
   needs data from the configuration after `connect` extracts it first. The builder is
   also not `Clone`, for the same reason.
3. **Fields.** `Endpoint::{Tcp { host, port }, Custom(String)}` is
   `#[non_exhaustive]`; `Udp` and `Serial` arrive with their transports. Endpoint
   contents are not validated in core (name resolution and port policy belong to the
   transport). `Timeouts { connect, request, idle: Option<Duration> }` defaults to 5 s,
   5 s and `None`; `idle` is recorded and unenforced at alpha because there is no
   background task, and the rustdoc says so. `request` is the source of the
   default-deadline rule of ADR-0007. `Credentials { username, password:
   Secret<String> }` has a derived `Debug` in which the password prints as
   `[REDACTED]`; the configuration stores it inside a further `Secret`, so even the
   user name is absent from `Debug` of the configuration.
4. **Plain data structs.** `Timeouts`, `ReconnectPolicy`, `RetryPolicy` and
   `Credentials` are `#[non_exhaustive]` with public fields, a `Default` (or `new`) and
   assignment from outside the crate, the way `Capabilities` works. Adding a field later
   is not a breaking change, and the builder is the validation point rather than each
   struct. Because public fields can hold a value the builder would reject, the
   arithmetic methods are total (below).
5. **`AccessPolicy` defaults to `ReadOnly`.** The plan does not name a default. Writes
   change a device, so they must be opted into: `ReadWrite` is a deliberate builder call,
   and a configuration that omits the setting cannot write. The simulator (S11b-2) and the
   write path (S09 onward) enforce it; `modbus-tcp` has no write at alpha either way (D3).
   The enum is exhaustive for the same reason as `ByteOrder` (ADR-0006): two variants
   describe the whole question, and a wildcard arm in a consumer would be an unreachable
   region.
6. **Builder rejections.** `build()` returns `ErrorKind::InvalidConfig`, with a message
   that names the field and never contains credential text, for: an empty scheme; a zero
   `connect` or `request` timeout; a reconnect multiplier that is NaN, infinite or below
   1.0; `max < initial`; a jitter outside `[0, 1]` (NaN included); and an invalid
   `address-convention`. A zero `idle` and a zero `initial` delay are accepted: neither is
   an error condition, and rejecting them would add rules the plan does not list.
7. **Unknown options are `InvalidConfig` (D12).** The connection-string layer (S08) and
   the drivers (S26 for Modbus) reject keys they do not recognise. Upstream logs a
   warning and continues (`docs/parity/modbus.md#MB-CFG-UNKNOWN`). The deviation is
   deliberate: a misspelt key such as `default-unit-identifer` otherwise silently keeps
   its default and sends frames to the wrong unit. It is recorded as a divergence row.
8. **`ReconnectPolicy::next_delay(attempt, sample)` is pure (D13).** The policy owns no
   random source and never sleeps. The transport seeds a small xorshift from
   `std::hash::RandomState` and passes the sample, and tests pass fixed samples, so no
   test sleeps on real time and no dependency is added. Definition: attempt 0 is the
   first retry; the base is `initial * multiplier^attempt` capped at `max`; the jitter
   factor is `1 + jitter * (2 * sample - 1)` with the sample clamped to `[0, 1]`
   (NaN counts as 0.5, no jitter; a NaN jitter counts as 0.0 and an out-of-range jitter
   is clamped). The result never exceeds `max * (1 + jitter)` and is never negative, so
   the cap bounds the delay even at the top of the jitter range. `is_exhausted(attempt)`
   is `attempt >= max_attempts` and is never true for `None`.
9. **Overflow-free arithmetic.** The computation is in `f64` seconds
   (`Duration::as_secs_f64`) with `powi` (the `u32` attempt saturates to `i32::MAX`
   through `i32::try_from`), `f64::min` and `max`, and `Duration::try_from_secs_f64` with
   `Duration::MAX` as the saturated value. `powi` saturates to infinity or zero and
   `f64::min` returns the cap for a NaN product, so no input panics: a huge attempt,
   a huge `max` (`Duration::MAX`), a zero `initial` (which would make `0 * inf` NaN and is
   special-cased to zero) and a multiplier of 1.0 (which would make a doubling loop run
   four billion times) all return at once. `Duration::mul_f64` is not used: it panics on
   overflow.
10. **`RetryPolicy { max_attempts: NonZeroU32, retry_on: BTreeSet<ErrorKind> }`
    (D13).** `max_attempts` counts the total attempts of one read, the first included;
    the default of 1 means no retry. That matches the inventoried upstream read path,
    where no retry was observed (an inference recorded in
    `docs/parity/modbus.md#MB-READ-RETRY`, not a run observation). `retry_on` defaults
    to `{Timeout, Disconnected, Backpressure}`, which equals
    `PlcError::is_retryable()`; a test asserts the equality over every kind, so the two
    cannot drift. `should_retry(kind, attempt)` is true while `attempt < max_attempts`
    and the kind is listed, where `attempt` is the number of attempts already made. The
    policy covers reads only. A bitset was rejected (Alternatives).
11. **`AddressOptions` carrier (D16).** `#[non_exhaustive] Clone + Default + Eq` with
    `convention() -> Option<&str>` and an `AddressOptionsBuilder` whose `build()` rejects
    an empty name, one over 64 bytes, or one containing whitespace or control
    characters. The default means "no convention supplied". `address-convention` is the
    shared connection-string key every driver maps into the builder; core documents it
    and validates only the token shape. `ConnectionConfig::address_options()` returns a
    clone, and `ConnectionConfigBuilder::address_convention(&str)` feeds it. In S11a the
    options become the second argument of `Driver::parse_address(text, &AddressOptions)`,
    so drivers stay stateless; `ModbusTcpDriver` (ADR-0015) accepts only `plc4x` at
    alpha and rejects any other name with `InvalidAddress`. Core does not know the legal
    names, so adding a convention never changes core.
12. **`redacted_display()`.** One line, `scheme=<s> endpoint=<e> access=<a>
    credentials=<present|none>`. It never contains the user name or the password. The
    `Endpoint::Custom` text is shown verbatim and the rustdoc says callers must not put
    credentials there. The connection-string slice (S08) adds a property test that
    random passwords never appear.
13. **Parity.** Rows `CORE-CONFIG-SHAPE`, `CORE-RECONNECT-POLICY`, `CORE-RETRY-POLICY` and
    `CORE-ADDRESS-OPTIONS` in `docs/parity/core.md` record the mapping to upstream and
    cite the `MB-CFG-*` rows rather than upstream files.

## Consequences
- `ConnectionConfig` cannot be cloned or shared between two `connect` calls. A caller
  that reconnects after `close` builds a configuration again, or the connection keeps
  its own copy of the pieces it needs. The reconnect policy and timeouts are `Copy` or
  `Clone`, so a connection can retain them without the credentials.
- The credentials live inside the configuration for its whole lifetime. Formatting
  cannot leak them, but anything that calls `expose()` can, and memory is not zeroized
  (secret.rs makes no such claim).
- Public fields mean the policy structs can hold values the builder rejects. The
  builder is the validation boundary and `next_delay` is total, so a bypass cannot
  panic, but it can produce a delay the builder would have refused.
- Defaulting to read-only means a caller who forgets `access(ReadWrite)` gets
  `Unsupported`-style refusals from write paths instead of a surprise write.
- `idle` is accepted and ignored at alpha. A caller who sets it gets no timeout, and
  the documentation is the only warning.
- A NaN sample being neutral hides a broken random source rather than failing. This is
  acceptable because the delay stays inside its documented bounds either way.

## Amendment (S26): driver-owned options on the configuration
Slice S26 adds one slot to `ConnectionConfig` for options that only one driver
understands. The change is additive to `stonecrab-core` and the facade.

- **`DriverOptions`.** `pub trait DriverOptions: Any + fmt::Debug + Send + Sync {}` has
  the shape of `ProtocolAddress` (object-safe, no methods, implemented by drivers, not
  sealed): trait upcasting to `Any` lets a driver downcast to its own concrete type and
  to nothing else. Its `Debug` output must contain no secrets; that is a contract of the
  trait, because the next point prints it.
- **Accessors.** `ConnectionConfigBuilder::driver_options(Arc<dyn DriverOptions>)`
  stores the options (a second call replaces the first); `ConnectionConfig::
  driver_options() -> Option<&Arc<dyn DriverOptions>>` and `driver_options_as::<T>() ->
  Option<&T>` read them. A configuration without options, or with options of another
  type, yields `None`, and a driver then uses its own defaults, so a configuration built
  by hand still connects. The `Arc` keeps `ConnectionConfig` as cheap to move as before;
  it stays not `Clone` because of the credentials (decision 2).
- **`Debug` and `redacted_display`.** The derived `Debug` shows the options through
  their own `Debug`. `redacted_display` appends ` options=<Debug text>` only when options
  are present, so the fixed shape of a configuration without options is unchanged.
  Neither hides the options, which is why the trait forbids secrets in them; credentials
  stay in `Credentials`.
- **Why not `Endpoint::Custom`.** The simulator passes its own URI options by
  re-encoding them into the text of `Endpoint::Custom` (`sim://bench?...`), which its
  `connect` parses again. A TCP driver cannot do that: its endpoint is the structured
  `Endpoint::Tcp { host, port }` the transport reads, and folding `?key=value` text into
  the host would corrupt the one field the transport resolves. Parsed values would also
  be re-parsed at `connect`, where an error has nowhere good to go. The slot carries the
  already validated values instead, and `connect` cannot fail on them.
- **Ownership of the meaning.** Core does not validate or interpret the options. The
  shared key `address-convention` stays a core field (decision 11) because every driver
  maps it the same way; a key only one driver knows belongs in that driver's options.
  `modbus-tcp` is the first user (`ModbusOptions`, ADR-0015 decision 15).

Tests in `crates/stonecrab-core/src/config.rs`: `driver_options_are_absent_by_default`,
`driver_options_round_trip_through_the_builder_and_downcast_to_their_own_type` and
`driver_options_appear_in_debug_and_redacted_display_without_credentials`;
`crates/stonecrab/tests/public_contract.rs` asserts `Box<dyn DriverOptions>` is
`Send + Sync + 'static`. The parity row is `CORE-DRIVER-OPTIONS` in
`docs/parity/core.md`.

## Alternatives considered
- **Omit credentials and `idle` so the configuration stays `Clone`.** Rejected: the full
  diagram is the contract (R-ARCH-3.1-04), and cloning a configuration that holds a
  secret is exactly what `Secret` forbids. Passing by value costs one move.
- **Make `Secret<T>` cloneable.** Rejected: it would weaken the one type that keeps
  credentials out of copies and logs for every other user of it.
- **A random-number dependency or a clock inside the policy.** Rejected: core is
  runtime-free and dependency-light (ADR-0007), and a policy that rolls its own dice
  cannot be tested with fixed samples.
- **A bitset for `retry_on`.** Rejected: indexing it by `ErrorKind` needs an
  `as`-cast discriminant, which core denies; `BTreeSet<ErrorKind>` needs only the `Ord`
  derive of ADR-0006 and reads naturally.
- **`Duration::mul_f64` for the growth.** Rejected: it panics on overflow, and a
  `u32::MAX` attempt must not.
- **A doubling loop.** Rejected: with multiplier 1.0 the cap is never reached, so the
  loop would iterate `attempt` times; `powi` is constant time.
- **`ReadWrite` as the default.** Rejected: see decision 5.
- **Carry the convention in driver state.** Rejected in D16: drivers would stop being
  stateless factories and a shared driver could not serve two conventions.
- **Reject only the empty convention.** Rejected: an unbounded name with spaces would
  turn into an untidy error message and a surprising comparison in every driver.
- **Warn on unknown options like upstream.** Rejected for the reason in decision 7.
- **Re-encode driver options into `Endpoint::Custom` for TCP drivers.** Rejected for the
  reason in the S26 amendment: the structured endpoint cannot carry them.
- **An `Any`-typed map of options in core.** Rejected: untyped keys would make every
  driver re-validate strings, and core would own a second parser. A typed value built by
  the driver's own `parse_config` keeps validation in one place.

## Compliance
Unit tests in `crates/stonecrab-core/src/address_options.rs`:
`default_and_empty_builder_carry_no_convention`,
`convention_round_trips_clones_and_last_call_wins`,
`invalid_conventions_are_rejected_without_echoing_them`. In
`crates/stonecrab-core/src/config.rs`: `defaults_match_the_documented_values`,
`every_setter_is_retained_by_its_accessor`,
`address_options_round_trip_and_outlive_the_config`,
`build_rejects_each_invalid_field_with_invalid_config`, `boundary_values_are_accepted`,
`backoff_doubles_from_the_initial_delay_and_caps_at_max`,
`jitter_samples_reach_both_bounds_and_the_cap_bounds_the_upper_one`,
`next_delay_is_total_and_bounded_for_every_attempt_and_sample`,
`next_delay_survives_policies_the_builder_would_reject`,
`reconnect_attempts_are_exhausted_only_at_the_limit`,
`default_retry_set_equals_the_error_retryable_predicate`,
`retry_stops_at_max_attempts_and_only_for_listed_kinds`,
`endpoint_display_brackets_ipv6_and_keeps_custom_text`,
`access_policy_displays_stable_names`, `credentials_debug_never_prints_the_password`,
`config_debug_hides_the_whole_credentials_value`,
`redacted_display_never_contains_credential_text`,
`redacted_display_has_a_fixed_shape_without_credentials`. The backoff, exhaustion and
retry tests carry `// covers: SPEC mandatory-failure-cases / F5 exhausted retry budget
(policy arithmetic)`. `crates/stonecrab/tests/public_contract.rs` asserts
`Send + Sync + 'static` for every new public type (the configuration holds a `Secret`
and still qualifies) in `public_foundation_types_are_send_sync_and_static`, and
`facade_builds_a_configuration_and_extracts_address_options` exercises the facade.
Doctests on every public item compile and run. `scripts/coverage-gate.sh` holds core at
100% lines. The proptest `prop_redacted_display_never_contains_password` arrives with
S08, and the first enforcement of the policies with S11b-2, S19 and S25.

## Addendum (S46, 2026-10-28)
Decision 5 no longer holds for `modbus-tcp`, which now writes. `access=read-write` in its
connection string sets this policy; see the S46 addendum of ADR-0015, which supersedes the
sentence about the alpha.
