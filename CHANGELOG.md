# Changelog
## Unreleased
### stonecrab 0.2.0
- Move the facade into a resolver-3 workspace with a runtime-independent core crate.
- Remove the specification-only `placeholder::status` API. There is no replacement
  status API; consult the capability and parity documentation.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) and stop shipping `tests/` in
  the crate archive.
- Re-export `ByteOrder` and `WordOrder` from `stonecrab-core`.
- Re-export `OpFuture`, `Clock`, `CancellationToken`, `StateCell` and `StateWatch` from
  `stonecrab-core`.
- Re-export `ConnectionConfig`, `ConnectionConfigBuilder`, `Endpoint`, `Timeouts`,
  `ReconnectPolicy`, `RetryPolicy`, `AccessPolicy`, `Credentials`, `AddressOptions` and
  `AddressOptionsBuilder` from `stonecrab-core`.
- Re-export `WriteItem`, `WriteRequest`, `WriteRequestBuilder`, `WriteResult`,
  `WriteResponse` and `WriteResponseBuilder` from `stonecrab-core`.
- Re-export `ConnectionUri` from `stonecrab-core`.
- Re-export `Driver`, `Connection` and `DriverRegistry` from `stonecrab-core`.
### stonecrab-codec 0.1.0
- Add the crate: `BoundedReader` and `BoundedWriter` with bounds-checked integer, float,
  byte-run and least-significant-bit-first coil reads and writes in an explicit
  `ByteOrder`, and `ParseError` (`Truncated`, `OversizedLength`, `InvalidDiscriminator`,
  `TrailingBytes`, `Overflow`) with `From<ParseError> for PlcError`.
- Add `coil_byte_count`, `pack_bits_lsb_first` and `unpack_bits_lsb_first`.
- Lengths are checked against the caller's maximum and the remaining input before any
  allocation or copy; no function panics on any input (ADR-0013).
### stonecrab-driver-sim 0.1.0
- Add the crate: the in-process simulator driver, depending on `stonecrab-core` and
  `tracing` only, Tokio-free, default features empty (ADR-0012, accepted).
- Add `RealtimeClock`, a `std`-only `Clock` whose `now()` is `Instant::now()` and whose
  `sleep_until` resolves immediately without moving time.
- Add `SimAddress` and `SimArea`: the grammar `STATE|RANDOM|SINK/<name>:<type>` with
  lowercase scalar type names, `ProtocolAddress` with protocol `sim`, 256-byte bound,
  `STDOUT` renamed `SINK`, the legacy array form rejected.
- Add `SimDevice` and `SimTag`: named tags, `ramp` with checked arithmetic that holds at
  the last value on overflow, a read counter, a poison-recovering shared device, stored
  latency and item limit, and a seeded xorshift `RANDOM` source (`SimDevice::with_seed`).
- Adopt `tracing` 0.1 (`std` only) workspace-wide; `deny.toml` lets only `tracing-core`
  depend on the banned `once_cell`.
- Add `SimDriver` (scheme `sim`; `sim://<device>?latency-ms=<n>&seed=<u64>`; `new`,
  `with_clock`, `with_device`, `device`) and `SimConnection`, the `Connection` it opens: the
  lifecycle `Disconnected`, `Connecting`, `Connected`, `Closing`, `Closed`; reads of
  `STATE` and `RANDOM` with per-item errors, writes to `STATE` and `SINK`; one operation in
  flight (a second fails with `Backpressure`); the default-deadline rule, cancellation and
  deadlines on the injected clock; whole-request read retry under `RetryPolicy`;
  `Diagnostics` on every error; `connect`, `read`, `write` and `close` spans. The README
  support table now marks read and write `simulator-tested`, and a test keeps it equal to
  `SimDriver::capabilities()`.
- Add `SimDevice::write`, `record_sink_write`, `sink_writes` and `reseed`.
- Add the failure script: `FailureStep` (`Delay`, `Timeout`, `Disconnect`,
  `DeviceException`, `Malformed`, `PartialFailure`, `Backpressure`, `UnknownAfterTransmit`),
  `FailureScript` (a bounded queue) and `SimDevice::push_failure`, `push_failures`,
  `pending_failures` and `clear_failures`. A connect, read or write takes at most one step;
  a scripted disconnect leaves the connection `Failed(Disconnected)`; a read without a
  deadline now gets `timeouts.request` afresh for each retry attempt.
### stonecrab-transport 0.1.0
- Add the crate: the object-safe `Transport` trait (`send`, `recv`, `shutdown`, `peer`,
  `limits`; every call takes a `Deadline`) and `TransportFactory` (both return
  `OpFuture`), and `FrameError` (`Truncated`, `Oversized`, `Malformed`) with
  `From<FrameError> for PlcError` (ADR-0014).
- Add `TcpTransport` and `TcpFactory { nodelay }` behind the default `tcp` feature: name
  resolution and connection inside one connect deadline, refused and reset connections as
  `Disconnected`, `recv` bounded by `max_buffered_bytes`, `send` bounded by
  `max_frame_bytes`, and a transport that poisons itself when a `send` is cut short.
- Add `SystemClock`, the Tokio-backed `Clock` (`now()` is Tokio's instant so paused-time
  tests agree with `Deadline`), and `Jitter`, a seeded xorshift64* source for
  `ReconnectPolicy::next_delay`.
- Adopt `tokio` with scoped features (`rt`, `time`, `sync`, `io-util`, and `net` through
  `tcp`); `deny.toml` bans its `full` feature. No task is spawned and nothing runs between
  calls.
- Test the transport against the unpublished testkit's new `net` feature (`Script`,
  `ScriptedPeer`, `FakeTransport`, `FakeTransportFactory`); no change to the public API.
- Add the `Framer<F>` trait (`decode`, `encode`, `max_frame_bytes`) and
  `FramedTransport<F, R>` over a `Box<dyn Transport>`: `next_frame` assembles fragmented
  and coalesced frames within `max_buffered_bytes`, `send_frame` encodes and sends one
  bounded frame, and `resync(Deadline)` discards bytes until the framer finds a frame
  boundary and fails with `Disconnected` at the deadline (ADR-0014 addendum).
- Add `ConnectionGuard<F, R>`, `GuardConfig` and `GuardStats`: the connection lifecycle
  machine over a `TransportFactory` and a `Framer` (`connect`, `request`,
  `request_with_default_deadline`, `next_reply`, `close`, `state`, `watch_state`, `stats`), one request
  in flight with a bounded FIFO queue (`Backpressure` beyond `max_pending_requests`),
  request-driven reconnect with jittered backoff and `Failed(Disconnected)` when the retry
  budget is spent, resync before the next request after a timeout, `Diagnostics` on every
  error, and a `close` bounded by `Timeouts.request`. No background task (ADR-0014
  addendum).
### stonecrab-modbus 0.1.0
- Add the crate scaffold: the native Modbus TCP driver crate, depending only on
  `stonecrab-core`, with default features empty. Nothing but address types and their
  text grammar is implemented; there is no codec, framing, session, connection or driver yet.
- Add `UnitId` (one MBAP byte; default 1; 0 and 255 are carried, not rejected),
  `ModbusArea` (`Coil`, `DiscreteInput`, `HoldingRegister`, `InputRegister`),
  `ModbusDataType` (the 15 numeric and bit types, with `from_upstream_name` recognising
  all 27 upstream names and refusing the 12 deferred ones by name),
  `AddressConvention::Plc4xCompatible` and `ModbusAddress` built through a validating
  `ModbusAddressBuilder`.
- `ModbusAddress` implements `ProtocolAddress` with protocol `modbus` and a canonical
  long form such as `holding-register:16:INT{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`
  (ADR-0015, proposed).
- Add `ModbusAddress::parse(text, &AddressOptions)`, the PLC4X address grammar: long
  and `0x`/`1x`/`3x`/`4x` short forms always, bare `40001`-style forms only with
  `address-convention=plc4x`, an optional `{unit-id: N, byte-order: 'NAME'}` block, and
  typed `InvalidAddress` rejections for everything else; `ModbusAddress::MAX_TEXT_BYTES`
  and the fuzz target `fuzz_modbus_address`.
- Add `Batcher::plan`, `BatchLimits`, `Chunk` and `Member`: tags are grouped by unit id
  and area, sorted by wire offset and merged into reads only when adjacent or overlapping
  (no gap reads), never above 125 registers or 2000 bits per chunk or `max_items` tags
  per plan.
- Add the PDU codec (ADR-0015): `FunctionCode`, `ExceptionCode` (the ten V1.1b section 7
  codes and `Unknown`) and `ModbusPdu` for the read functions 01 to 04, their normal
  responses and the exception response, with `encode_request`, `decode_request`,
  `decode_response`, `encode_response`, `ModbusPdu::check_reply_to` and the limits
  `MAX_PDU_BYTES`, `MAX_READ_BITS` and `MAX_READ_REGISTERS`. Quantities (1 to 2000 bits,
  1 to 125 registers), byte counts and the 253-byte PDU bound are checked before any
  allocation; write PDUs and exception codes outside the ten are refused. The crate now
  depends on `stonecrab-codec`. Golden vectors under `fixtures/modbus/tcp/` are written
  from the V1.1b examples or built to fail; the fuzz target is `fuzz_modbus_pdu`.
### stonecrab-sensors 0.1.0
- Add the crate: declarative sensor profiles for Stonecrab, depending on `stonecrab-core`,
  `serde` and, behind the default `toml` feature, `toml` (ADR-0016). No driver reads a
  profile yet; that arrives in a later slice.
- Add the schema (version 1): `SensorProfile`, `ProfileId`, `SchemaVersion`,
  `DeviceApplicability`, `ConnectionRequirements`, `MeasurementDef`, `ByteOrderName`,
  `WordOrderName`, `ScaleStep` (`Multiply`, `Divide`, `Offset`, `Clamp`, `BitField`),
  `ValueRange`, `StatusField`, `QualityMapping`, `Provenance` and `ValidationStatus`. All
  derive `Deserialize` with unknown keys denied; none derives `Serialize`.
- Add `SensorProfile::from_toml` (feature `toml`): 64 KiB size bound checked before
  parsing, schema versions other than 1 refused as `ProfileError::IncompatibleSchema`, and
  errors that carry a line and column but never part of the document. Add `ProfileError`
  (`Oversized`, `Syntax`, `IncompatibleSchema`, `InvalidIdentifier`) with
  `From<ProfileError> for PlcError` (`InvalidConfig`).
- Add the fuzz target `fuzz_sensors_profile_toml`. Adopt `serde` and `toml` (`display`
  off); the lock pins `toml_parser` 1.0.9 to keep a single `winnow`.
- Add `SensorProfile::validate`, which returns every semantic error of a profile at once
  (duplicate names, unknown or empty units, missing byte or word order for 32- and 64-bit
  Modbus types, overlapping status bits, `count != 1`, unevaluable scaling steps, unknown
  quality references, address convention rules), and the fixed unit table `Unit`. Add eleven
  `ProfileError` variants for them; the enum was already `#[non_exhaustive]`.
### stonecrab-core 0.1.0
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) for feature-gated item docs.
- Establish shared metadata, safety lints and foundation contracts.
- Add exact-width scalar values, validated tag identifiers, monotonic deadlines,
  immutable finite resource budgets, explicit capabilities, typed diagnostic
  failures and redacted secret formatting.
- Add protocol-owned tags, bounded read builders, exact batch correlation,
  preserved per-item protocol status and explicit unknown-after-transmit outcomes.
- Add `ByteOrder` and `WordOrder`, optional `Tag::with_byte_order`/`with_word_order` with
  `byte_order()`/`word_order()` accessors, and additive `PartialOrd, Ord` on `ErrorKind`,
  `Operation` and `PlcValueType` (ADR-0006).
- Add `OpFuture`, the `Clock` trait (no implementation in core), `CancellationToken` with
  child tokens and `StateCell`/`StateWatch` latest-value connection-state observation,
  all on `std` only (ADR-0007).
- Add `ConnectionConfig` (not `Clone`) with its validating builder, `Endpoint`, `Timeouts`,
  `ReconnectPolicy` with pure `next_delay`, `RetryPolicy`, `AccessPolicy` (default
  `ReadOnly`), `Credentials`, and the `AddressOptions` carrier with its builder
  (ADR-0008). The simulator connection enforces the timeouts, retry and access policies.
- Add write batch contracts: `WriteItem`, `WriteRequest` with its bounded builder (item
  budget, duplicate identities, value type versus tag hint, deadline) and `WriteResult`,
  `WriteResponse` with its correlating builder, `iter_confirmed`, `iter_rejected` and
  `iter_unknown`. Unknown outcomes are first-class and never retried or replayed
  (ADR-0010). Data only.
- Add `ConnectionUri::parse` for `scheme(:transport)?://authority(?k=v(&k=v)*)?` with
  ordered `options()`; it rejects percent-encoding, credentials in the authority, empty
  or duplicate keys, whitespace and input over 4096 bytes, and `Display` renders the
  canonical text. Covered by the `fuzz_core_connection_string` target.
- Add the `Driver` and `Connection` traits (object safe, `Send + Sync`; the narrow D2
  surface) and `DriverRegistry` (`register` takes `&mut self`, duplicate scheme
  `InvalidConfig`, unknown scheme `Unsupported(Operation::Connect)`, `parse` and `connect`
  through `ConnectionUri`), and the default-deadline rule they document (ADR-0011).
- Add `ReadRequestBuilder::cancel`, `ReadRequest::cancel`, `WriteRequestBuilder::cancel`
  and `WriteRequest::cancel`: an optional `CancellationToken` carried by a request.
- Accept ADR-0007: the simulator connection enforces the token and the default deadline.
- Add an unpublished deterministic address fixture testkit for contract tests.
- Add `FakeClock`, `block_on`, `block_on_with`, `poll_once` and a counting waker to the
  unpublished testkit for deterministic, sleep-free async tests.
- Add the unpublished testkit's `fixtures` feature: the golden-vector loader (`load_dir`,
  `load_hex`, `GoldenVector`, `VectorSource`, `Provenance`, `GoldenError`) that enforces
  "every vector is listed in `PROVENANCE.toml`", and `InteropMap`, the reader of the
  first-party `fixtures/modbus/interop/register-map.toml`. The upstream vectors themselves
  wait for maintainer prerequisite P4.
## stonecrab 0.1.0
- Publish the specification-stage package. No industrial protocol support.
