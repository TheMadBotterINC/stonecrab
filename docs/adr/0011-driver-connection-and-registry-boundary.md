# 0011: Driver, Connection and DriverRegistry boundary
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S11a of docs/ALPHA_PLAN.md,
landed in one commit with S11b-2 (ADR-0012, `SimDriver` and `SimConnection`). It records
decisions D2 (the narrow `Connection` surface), D4 (subscriptions out), D11 (how the
driver trait lands) and the signature half of D16 (`parse_address` takes the
`AddressOptions` carrier). It is the "new public trait" ADR that CODING_STANDARDS 12.6
requires, and it fulfils ADR-0001's rule that no public driver trait exists until the
simulator can exercise it. Every change is additive to `stonecrab-core` and the facade.

## Context
S01 to S10 built what a driver needs: explicit deadlines and a `Clock` (ADR-0002,
ADR-0007), a cancellation token and state watch (ADR-0007), the connection
configuration and address options (ADR-0008), the connection-string grammar (S08),
bounded read and write batches (ADR-0003, ADR-0010) and the testkit's `FakeClock` and
executor (S10). None of it is reachable through one object yet. ARCHITECTURE.md 3.1 draws
the boundary as `DriverRegistry`, `Driver` and `Connection`; this ADR fixes the Rust shape
and says where it differs from the diagram.

Constraints. ADR-0001 forbids a public driver trait that nothing exercises, and
CODING_STANDARDS 13.5 forbids first-party doubles of our traits outside the testkit and
the simulator. Every commit must pass the 100% line gate for core (ADR-0005), and the
registry's behaviour is only observable through a driver. The traits must be object safe
and their futures `Send` (CODING_STANDARDS 8.2, 8.3).

## Decision
1. **`Driver` (D16 signature).** `trait Driver: Send + Sync { fn scheme(&self) ->
   &'static str; fn capabilities(&self) -> Capabilities; fn parse_config(&self, uri:
   &str) -> Result<ConnectionConfig>; fn parse_address(&self, text: &str, options:
   &AddressOptions) -> Result<Arc<dyn ProtocolAddress>>; fn connect(&self, config:
   ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>>; }`. A driver is a stateless
   factory: everything per connection lives behind the `Connection` it returns, so one
   driver value serves any number of connections. Addresses are `Arc`, not the diagram's
   `Box`, because `Tag::new` takes an `Arc<dyn ProtocolAddress>`. The `options` argument
   is the D16 carrier: a caller takes `ConnectionConfig::address_options()` before
   `connect` consumes the configuration, or builds one from a sensor profile, and the
   driver keeps no convention state. `connect` takes the configuration by value (it may
   hold a non-cloneable `Secret`) and is bounded by `config.timeouts().connect`, derived
   from the driver's own clock; the rustdoc states that deadline source in `# Cancel
   safety` and `# Wire effects`. `Capabilities` is `Copy`, so `capabilities()` returns it
   by value.
2. **`Connection` (D2).** `trait Connection: Send + Sync { fn state(&self) ->
   ConnectionState; fn watch_state(&self) -> StateWatch; fn capabilities(&self) ->
   Capabilities; fn limits(&self) -> ResourceLimits; fn read(&self, request:
   ReadRequest) -> OpFuture<'_, ReadResponse>; fn write(&self, request: WriteRequest) ->
   OpFuture<'_, WriteResponse>; fn close(&self) -> OpFuture<'_, ()>; }`. `limits()`
   exists because `ReadRequest::builder(limits)` and `WriteRequest::builder(limits)` need
   the connection's own item budget. `Capabilities` and `ResourceLimits` are both `Copy`
   and are returned by value; no reference is handed out. `subscribe`, `browse` and
   `discover` are not declared. They will be added later as provided methods returning
   `ErrorKind::Unsupported`, which breaks no implementor, together with the request and
   response types they need (ADR-0001: no empty scaffolds). Each `OpFuture` method has
   `# Cancel safety` and `# Wire effects` sections.
3. **The default-deadline rule is part of the contract.** A request with an explicit
   `Deadline` keeps it; one without receives `Deadline::after(clock.now(),
   timeouts.request)`, computed from the connection's clock and owned by the connection
   (ADR-0007 decision 7). A request whose deadline has already elapsed is rejected with
   `Timeout` before anything is transmitted. `close` is bounded by `timeouts.request`
   and ends in `Closed` even when the clean goodbye does not finish. ADR-0007 is accepted
   by this commit because `SimConnection` is the first enforcer.
4. **`DriverRegistry`.** `new`, `register(Arc<dyn Driver>) -> Result<()>`, `driver`,
   `schemes` (sorted), `parse(uri) -> Result<(Arc<dyn Driver>, ConnectionConfig)>` and
   `connect(uri) -> OpFuture<'_, Box<dyn Connection>>`. It holds a
   `BTreeMap<&'static str, Arc<dyn Driver>>` behind no lock: it is immutable after setup,
   `register` takes `&mut self`, and the compiler therefore refuses registration while a
   registry is shared. A duplicate scheme is `InvalidConfig` and keeps the first driver.
   `parse` and `connect` extract the scheme with `ConnectionUri::parse` and hand the
   complete original text to `Driver::parse_config`, so no driver splits the authority or
   the query by hand and a malformed string fails the same way for every scheme. An
   unknown scheme is `Unsupported(Operation::Connect)`. `connect` parses when it is
   called and delivers a parse failure as the future's error. The registry does not
   validate a driver's scheme spelling: lookup goes through `ConnectionUri`, whose grammar
   admits only `a-z`, `0-9` and `-`, so a driver with another spelling is simply
   unreachable by connection string; the `Driver::scheme` rustdoc says so.
5. **Cancel slots.** `ReadRequestBuilder::cancel(CancellationToken)` and
   `ReadRequest::cancel() -> Option<&CancellationToken>`, and the same pair on
   `WriteRequestBuilder` and `WriteRequest`. They are additive; a request without a token
   behaves as before. The token is retained as given, an already cancelled token is kept,
   and a second `cancel` call replaces the first. The connection, not the request, rejects
   a cancelled token (`Cancelled`, before transmission) and ends a waiting operation with
   the same kind.
6. **D4: subscriptions are out.** Both alpha drivers report
   `SubscriptionMode::Unsupported`. Polling emulation, `StaleTracker`, `QueuePolicy` and
   `LossCounters` are M2.
7. **D11: one commit.** The traits, the registry and the simulator driver land together.
   The registry's lines are executed only through a driver, and the core gate is 100%
   per commit, so a commit with the traits alone cannot pass it, and a test double in core
   would break CODING_STANDARDS 13.5. S11b-1 landed first (data types only); S11a and
   S11b-2 are one commit on master. Core's own tests assert object safety and `Send`
   bounds through `Arc<dyn Driver>`, `Box<dyn Connection>` and functions taking
   `&dyn Driver` without implementing either trait; the behavioural tests are the
   simulator's, which the coverage gate counts toward core (the report is workspace-wide
   and filtered by path).
8. **No item exists only to serve an example, and examples print through `tracing`.**
   Every item an example calls is exercised by the contract suite or a driver test. When
   examples arrive (S14) they emit results with `tracing::info!` and structured fields on
   a `tracing-subscriber` `fmt` layer they install themselves, never `println!`
   (clippy.toml disallows it), and `main` returns `std::process::ExitCode` and never calls
   `std::process::exit`.
9. **Divergences from ARCHITECTURE.md 3.1 to 3.4.** The sections gain one
   "Implementation note (ADR-0011)" paragraph each; the diagrams are not redrawn.
   Addresses are `Arc`; `parse_address` takes the options argument; `Deadline::after`
   takes an explicit clock reading; request builders are bound to `ResourceLimits`;
   `Diagnostics` fields are optional; the item budget is mandatory; `Connection` is the
   narrow seven-method set; `Endpoint` is `Tcp` and `Custom` only; and `SystemClock` is
   in `stonecrab-transport` (ADR-0007).
10. **Parity.** Rows `CORE-DRIVER-TRAIT`, `CORE-CONNECTION-TRAIT` and `CORE-REGISTRY` in
    `docs/parity/core.md` map the types to upstream `PlcDriver`, `PlcConnection` and
    `PlcDriverManager` and cite the simulator tests that exercise them.

## Consequences
- An application can write `registry.connect("sim://bench")` today and `modbus-tcp://`
  later with the same code, and a test can swap drivers without changing it.
- Core's 100% gate depends on the simulator crate's tests. Running `cargo test -p
  stonecrab-core` alone does not cover the registry; the workspace coverage run does. A
  future change that removes the simulator's registry tests fails the core gate, which is
  the intended coupling.
- `Connection::read` and `write` take their request by value, so a retry or a repeat is
  a new, explicit call; a write is never replayed by the library.
- Provided methods for `subscribe`, `browse` and `discover` are additive, but their
  request types are new public API and need their own ADR.
- A registry cannot be extended once shared. That is the cost of having no lock; an
  application that must add a driver later builds a second registry.
- `connect` parses eagerly, so a malformed string fails when `connect` is called, not
  when the future is polled; both deliver the error through the future.

## Alternatives considered
- **A test-only `NullDriver` in core.** Rejected: a double of our trait outside the testkit
  and the simulator (13.5), and it would exercise nothing the simulator does not.
- **A testkit `FixtureDriver`.** Rejected: the simulator is the sanctioned double and is
  published; a second would duplicate behaviour.
- **Landing the traits one commit before the simulator.** Rejected: that commit fails the
  100% core gate (decision 7) and ADR-0001's trait rule.
- **The full ARCHITECTURE.md 3.1 `Connection`.** Rejected (D2): `Subscribe*`, `Browse*` and
  `Discover*` have no types and no implementation at alpha.
- **`Box<dyn ProtocolAddress>` from `parse_address`.** Rejected: `Tag::new` needs an `Arc`,
  and a profile would re-box on every tag.
- **Convention as driver state or a `parse_address` per convention.** Rejected (D16):
  drivers stay stateless factories.
- **A `Mutex` or `RwLock` around the registry map.** Rejected: nothing mutates it after
  setup, and `&mut self` expresses that in the type.
- **`async fn` in the traits.** Rejected (CODING_STANDARDS 8.2): not object safe.
- **Passing the cancellation token to `read` and `write`.** Rejected: it would change the
  signature of every call site; a request-level slot is additive and keeps the token next
  to the deadline it pairs with.
- **Returning `&Capabilities` and `&ResourceLimits`.** Rejected: both are `Copy` and small,
  and a reference would tie a caller to the connection's lifetime for no gain.

## Compliance
Core tests: `crates/stonecrab/tests/public_contract.rs`,
`driver_and_connection_are_object_safe_send_sync_traits` (object safety, `Send`, `Sync`,
a function taking `&dyn Driver` and `&dyn Connection`) and the extended Send/Sync list;
`crates/stonecrab-core/tests/read_contract.rs` and `write_contract.rs`,
`the_cancel_slot_is_absent_by_default_and_shares_the_callers_token` and
`an_already_cancelled_token_is_retained_for_the_connection_to_reject`; doctests on
`Driver`, `Connection`, `DriverRegistry` and the cancel accessors. Registry behaviour is
tested through the simulator in `crates/stonecrab-driver-sim/tests/driver_contract.rs`:
`registration_lookup_and_listing`,
`a_duplicate_scheme_is_invalid_config_and_keeps_the_first_driver`,
`schemes_are_listed_sorted_whatever_the_registration_order`,
`an_unknown_scheme_is_unsupported_connect_from_parse_and_connect`,
`registry_errors_do_not_invent_a_connection_state_or_latency`,
`a_malformed_connection_string_is_invalid_config_from_parse_and_connect`,
`parse_returns_the_driver_and_a_configuration_without_opening_anything` and
`the_registry_connects_reads_state_random_and_a_ramp_and_writes_state_and_sink`. The
default-deadline rule is tested by `an_absent_deadline_uses_timeouts_request`.
`scripts/coverage-gate.sh` holds core and the simulator at 100% lines.
