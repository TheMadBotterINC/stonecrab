# 0014: Transport trait, TCP transport and Tokio adoption
## Status
Superseded in part by ADR-0018: Tokio's scoping to the `stonecrab-transport` crate and
its `tcp` feature is now the `transport` feature of `stonecrab` (module
`stonecrab::transport`). D15, D21, D22 and D23 stand.
Accepted by the sole maintainer on 2026-10-27, as slice S16 of docs/ALPHA_PLAN.md.
It adds the new crate `stonecrab-transport` (CODING_STANDARDS 12.6 requires an ADR for a
new crate and for a new public trait), adopts `tokio` (section 15.1 and 8.1), and
implements decisions D10, D15, D21, D22 and D23 for this crate. It builds on ADR-0005
(coverage tiers), ADR-0007 (`Clock`, `SystemClock` placement), ADR-0008 (`Endpoint`,
`Timeouts`, `ResourceLimits`), ADR-0009 (test tooling) and ADR-0013 (the codec boundary).

## Context
Drivers need a byte pipe that every call bounds with a deadline, a way to open it again
after a failure, and a real clock. Core is Tokio-free by design (CODING_STANDARDS 8.1), so
the first runtime-bound code lands here. Decision D15 fixes the concurrency model: no
background task, request-driven reconnect, no `tokio::spawn` in any published crate.
Decision D21 fixes how the transport is tested: timing on fakes, bytes on loopback.

## Decision
1. **Crate boundary (D22).** `stonecrab-transport` depends on `stonecrab-core`, `bytes`
   and `tokio`. It does not depend on `stonecrab-codec`. `Endpoint`, `Timeouts` and
   `ResourceLimits` come from core (ADR-0008); the transport defines no endpoint type.
   It defines `FrameError`, a `#[non_exhaustive]` enum `{ Truncated, Oversized(usize),
   Malformed }` with `Display`, `std::error::Error` and `From<FrameError> for PlcError`
   (`Truncated` and `Malformed` map to `MalformedData`, `Oversized` to `ResourceLimit`;
   the `FrameError` is the error's `source()`). The Modbus `MbapFramer` converts the
   codec's `ParseError` into it.
2. **`Transport`.** `trait Transport: Send` with `send(&mut self, Bytes, Deadline)`,
   `recv<'a>(&'a mut self, &'a mut BytesMut, Deadline)`, `shutdown(&mut self, Deadline)`,
   `peer(&self) -> &Endpoint` and `limits(&self) -> &ResourceLimits`. Methods return
   `OpFuture`, so the trait is object safe and `Box<dyn Transport>` is `Send`. `recv`
   needs an explicit lifetime parameter: with elision both reference arguments would be
   tied to `&mut self`'s anonymous lifetime and the buffer borrow could not outlive it.
   - `recv` appends at most `max_buffered_bytes - buffer.len()` bytes, using
     `BufMut::limit` so `BytesMut`'s habit of offering extra capacity cannot overshoot
     the bound. It returns the number of bytes read; `0` means the peer closed. A buffer
     already at or above the bound fails with `ResourceLimit` before any read.
   - `send` carries one frame and refuses a payload longer than `max_frame_bytes` with
     `ResourceLimit` before any write. The plan implies this bound ("maximum bytes in any
     individual frame"); it is documented on the method.
   - Every method documents `# Cancel safety`, `# Wire effects` and names its deadline
     source (the `Deadline` argument). A deadline that has already passed fails with
     `Timeout` before any I/O. Checks run in a fixed order: deadline, usable, bounds.
3. **`TransportFactory`.** `trait TransportFactory: Send + Sync` with `open<'a>(&'a self,
   &'a Endpoint, ResourceLimits, Deadline) -> OpFuture<'a, Box<dyn Transport>>`. The
   factory keeps no connection state, so the driver can call it for the first connection
   and again for every reconnect.
4. **`TcpTransport` and `TcpFactory { nodelay }`** (feature `tcp`). Private fields over
   `tokio::net::TcpStream`. `open` resolves `Endpoint::Tcp` with `tokio::net::lookup_host`
   inside the connect deadline and tries the resolved addresses in order inside the same
   deadline (`connect_first`, tested with a refused address followed by a live one).
   `Endpoint::Custom` is rejected with `InvalidConfig` before the deadline is looked at.
   `nodelay` defaults to true. Error mapping: `TimedOut` while connecting and an expired
   deadline are `Timeout`; every other operating-system failure (refused, reset, aborted,
   broken pipe, unreachable, no address, resolver failure) is `Disconnected`, including
   `TimedOut` on an established connection, where it means the kernel gave up
   retransmitting. Failures carry `Diagnostics` with `category = Transport` and
   `operation = Connect` (`Close` for shutdown). Sends and receives set no operation:
   the core `Operation` set has no per-frame entry, and the transport does not invent
   one. Name resolution runs on Tokio's blocking pool, which cannot be interrupted; a
   deadline that passes during resolution returns `Timeout` at once and the resolver
   thread finishes on its own. This is the one place a thread outlives a call, and it is
   owned by Tokio, not by this crate.
5. **Poisoning.** A `send` that is dropped or times out mid-write may have put a prefix of
   the frame on the wire, after which the stream is out of step with the peer. The
   transport clears its `usable` flag before writing and sets it again only when the whole
   frame was written, so every later call fails with `Disconnected` and the owner reopens
   the connection (D15) instead of guessing. A `recv` that returns `0`, or fails other than
   by deadline, also clears the flag. A `recv` that times out or is dropped consumes
   nothing (`read_buf` is cancel-safe) and leaves the transport usable. `shutdown` closes
   from its first poll, is idempotent (a repeat call is `Ok(())` even with an elapsed
   deadline), half-closes the socket (a FIN, no application
   bytes) and makes later sends and receives fail.
6. **The deadline helper.** One private function, `deadline::within`, races an I/O future
   against `tokio::time::timeout_at(Instant::from_std(deadline.instant()))`. It first
   fails with `Timeout` if the deadline has already elapsed, so a ready future cannot win
   against an expired deadline. All four I/O sites go through it; no method has a default
   or a second deadline source (CODING_STANDARDS 8.5). Deadlines are compared with
   `tokio::time::Instant::now().into_std()`, the same reading as `SystemClock::now()`:
   outside paused time that is `std::time::Instant::now()`.
7. **`SystemClock` (ADR-0007, decision 3).** A unit struct (`#[non_exhaustive]`, built with
   `SystemClock::default()`) implementing `stonecrab_core::Clock`: `now()` is
   `tokio::time::Instant::now().into_std()` so tests that pause Tokio time move the clock
   and every `Deadline` built from it together, `wall_now()` is `SystemTime::now()` and
   `sleep_until` is `tokio::time::sleep_until` boxed into an `OpFuture`. It must be polled
   in a runtime with the time driver; that is documented.
8. **`Jitter`.** `jitter.rs` holds an xorshift64* generator (shifts 12, 25, 27, multiplier
   `0x2545F4914F6CDD1D`) seeded from `std::hash::RandomState`, with `next_unit() -> f64` in
   `[0, 1)`. The sample is `f64::from(u32)` of the top 32 output bits divided by 2^32, so
   no `as` cast exists and the maximum is below 1. `Jitter::from_seed` makes tests
   reproducible (a zero seed is replaced). D13: `ReconnectPolicy::next_delay` stays pure;
   the caller draws the sample here. There is no `rand` dependency.
9. **No background work (D15).** There is no `tokio::spawn`, no `JoinHandle`, no channel and
   no timer task in this crate. A dead peer is noticed by the next call. `clippy.toml`'s
   `disallowed-methods` (`tokio::task::spawn`, `tokio::sync::mpsc::unbounded_channel`,
   `std::thread::sleep`) were verified to resolve with Tokio present: a scratch call to
   each of `tokio::spawn`, `tokio::task::spawn`, `unbounded_channel` and `thread::sleep`
   in this crate produced a `disallowed_methods` warning for each (the scratch file is not
   committed), and clippy reports no unresolved-path warning for `clippy.toml`.
   `JoinSet::spawn` in one test is not on the list; it keeps the handle in the set
   (section 8.7).
10. **D10: `tcp` is default-on.** `[features] default = ["tcp"]`, `tcp = ["tokio/net"]`.
    This follows ARCHITECTURE.md section 10 and is the recorded deviation from the
    off-by-default rule: a transport crate with no transport is not a usable default.
    The `features` job's per-crate `--no-default-features` check proves the crate still
    builds and exposes `Transport`, `TransportFactory`, `FrameError`, `SystemClock` and
    `Jitter` without it (also verified here with `cargo build`, `clippy` and `cargo test`
    on `--no-default-features`). The deadline helper is compiled only with `tcp`, the one
    feature that uses it.
11. **Tokio adoption (section 15.1, 15.4, 15.5).** `[workspace.dependencies]` gains
    `tokio = { version = "1.54", default-features = false }`; `Cargo.lock` resolves
    1.54.1. This crate enables `rt`, `time`, `sync` and `io-util`, and `net` through
    `tcp`; its dev-dependencies add `macros`, `test-util` and `rt-multi-thread`. `full` is
    banned by `[[bans.features]] crate = "tokio", deny = ["full"]` in `deny.toml`
    (cargo-deny 0.20.2 syntax; verified by adding `full` temporarily, which fails the
    `bans` check with "feature 'full' for crate 'tokio' is explicitly denied").
    - `net`: `TcpStream` and `lookup_host`.
    - `time`: `timeout_at`, `sleep_until` and `Instant` for the helper and `SystemClock`.
    - `io-util`: `AsyncReadExt::read_buf` and `AsyncWriteExt::write_all`/`shutdown`.
    - `rt`: `lookup_host` calls `spawn_blocking`, which panics without it, and every
      Tokio socket and timer needs a runtime to run in.
    - `sync`: enabled because the plan lists it for this crate. Nothing in this slice's
      source uses it; the bounded single-flight slot of the connection guard (S19, D15)
      is its consumer. It adds no dependency (`sync = []`). It can be dropped from the
      manifest until S19 without touching any code.
    - dev `macros`, `test-util`, `rt-multi-thread`: `#[tokio::test]`, `start_paused`
      for the clock and helper tests, and one multi-thread test that drives a
      `Box<dyn Transport>` from a `JoinSet` task. They never reach the packaged runtime
      graph.
    Resolved crates in the normal graph with `tcp`: `tokio`, `bytes`, `libc`, `mio`,
    `socket2`, `pin-project-lite`, plus `windows-sys` and `windows-link` on Windows and
    `wasi` on WASI targets. `tokio-macros` joins only for dev builds (`proc-macro2`,
    `quote` and `syn` were already present). `docs/DEPENDENCY_AUDIT.md` has the entry.
12. **D23 result.** Run on the resolved tree (cargo stops at the first unmatched package
    spec, so each name was also run alone, as in ADR-0013):

    ```text
    $ cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono
    error: package ID specification `once_cell` did not match any packages
    $ cargo tree -e normal,dev -i once_cell
    error: package ID specification `once_cell` did not match any packages
    $ cargo tree -e normal,dev -i lazy_static
    error: package ID specification `lazy_static` did not match any packages
    $ cargo tree -e normal,dev -i log
    error: package ID specification `log` did not match any packages
    $ cargo tree -e normal,dev -i time
    error: package ID specification `time` did not match any packages
    $ cargo tree -e normal,dev -i chrono
    error: package ID specification `chrono` did not match any packages
    ```

    None of the five banned crates is in the graph, so no `deny.toml` `wrappers` entry is
    added. `cargo deny --locked check` stays green. Tokio's optional dependencies other than
    `bytes`, `libc`, `mio`, `socket2` and `pin-project-lite` (`parking_lot`,
    `signal-hook-registry`, `tracing` and others) stay disabled because no enabled feature
    pulls them.
13. **Tracing is not adopted here.** S11b-1 adds `tracing` workspace-wide together with the
    D23 `once_cell`/`tracing-core` wrapper. A later transport slice adds the spans of
    CODING_STANDARDS 14.2. This slice emits nothing.
14. **Timing tests on doubles and bytes on loopback (D21).** Deadline expiry is tested
    with `#[tokio::test(start_paused = true)]` against a future that never completes, so
    no test sleeps on real time. Loopback tests (`127.0.0.1:0`, port read back) cover
    bytes only: refused connect, fragmentation, coalescing, peer close, reset, buffer and
    frame bounds, shutdown, a dropped `recv`, a `send` cut short and a transport driven
    from another worker thread. The connect-deadline path through `FakeTransportFactory`
    is covered in S17 and S19.

## Consequences
- Drivers and the connection guard can code against `dyn Transport` and `dyn
  TransportFactory` and be tested with doubles that live in the testkit.
- A timed-out `send` costs a reconnect. That is deliberate (section 6.8: ambiguous
  outcomes are first-class); a framer that wants to resynchronize instead needs a
  different contract, which `FramedTransport::resync` (S18, see the addendum below)
  provides on top.
- Every `OpFuture` is one allocation. Accepted in ADR-0007.
- Name resolution is outside the transport's control once started (blocking pool), and a
  host name may query the network. Documented on `TransportFactory::open`; tests use IP
  literals only.
- `sync` is declared and unused until S19 (see decision 11).
- Windows reset behavior (`ConnectionReset` versus `BrokenPipe`) differs from Unix; both
  map to `Disconnected`, and the tests assert only the kind.

## Named uncovered lines (ADR-0005)
`scripts/coverage-gate.sh` holds `crates/stonecrab-transport` at 95%. The run recorded at
S16 reports 457 of 459 lines (99.56%). `target/coverage-stonecrab-transport.txt` lists no
wholly uncovered line (there is no "Uncovered Lines" section), so the misses are error
arms inside covered lines; the three arms below are where the region report (9 missed
regions) places them:
- **Real-socket connect timeout, the first named arm.** `deadline.rs`, the
  `Err(_elapsed)` arm of `within` (`Err(_elapsed) => Err(site.error(ErrorKind::Timeout,
  "deadline elapsed"))`) as instantiated for `tcp.rs`'s `open_tcp`, which wraps
  `resolve_and_connect`. A listening loopback socket completes the handshake in the kernel
  backlog, so a never-accepting listener cannot make a connect hang, and section 13.3
  forbids non-loopback addresses. The generic arm itself is covered by
  `a_pending_io_future_fails_with_timeout_when_the_deadline_passes` on paused time; what no
  test does is expire the deadline while a real connect is pending. The same holds for a
  deadline expiring while a real `write_all`, `read_buf` or `shutdown` is pending.
- **Resolver failure.** `tcp.rs`, the `?` after `lookup_host((host, port)).await` in
  `resolve_and_connect`. It needs a name that does not resolve, which means asking a
  resolver; tests use IP literals.
- **`set_nodelay` failure.** `tcp.rs`, the `.map_err(|error| Site::CONNECT.io_error(error))`
  closure and its `?` in `open_tcp`. Setting an option on a freshly connected socket does
  not fail on loopback unless the peer already reset it.
The write-failure arm in `send` is covered by
`a_write_failing_on_a_reset_connection_is_disconnected_and_poisons`, which waits for the
reset to become visible with `TcpStream::readable` before writing so the result does not
depend on timing.

## Addendum (S17): the testkit `net` feature
Slice S17 adds the scripted network doubles to `tools/stonecrab-testkit` behind the
feature `net` (`net = ["dep:bytes", "dep:tokio", "dep:stonecrab-transport"]`), off by
default so that `stonecrab-core`'s dev-dependency on the testkit stays Tokio-free. The
transport crate dev-depends on the testkit with `features = ["net"]`, and the testkit
depends optionally on the transport (`default-features = false`, since the traits need no
`tcp`): a dev-dependency cycle of the same shape as core and testkit in ADR-0003, which
Cargo allows because a dev-dependency edge is not part of the normal graph. The testkit
enables the Tokio features `rt`, `net`, `time`, `sync`, `io-util` and `macros`, and takes
`bytes` because the `Transport` signatures use `Bytes` and `BytesMut`; none of it reaches
a published crate's normal graph. Decision 9 above ("no `tokio::spawn`") holds for the
published crates: the one sanctioned `tokio::spawn` in the workspace is in the testkit's
`ScriptedPeer`, which owns the `JoinHandle`, joins it in `close()`, aborts it on drop and
carries `#[expect(clippy::disallowed_methods, reason = ...)]` naming itself as owner
(CODING_STANDARDS 8.7); its stop signal is a one-slot `oneshot` channel. Loopback tests on
`ScriptedPeer` keep to bytes (decision 14): its `Delay` and `LateReply` sleeps are capped
at `MAX_PEER_DELAY` (200 ms), and timing is tested on `FakeTransport` and
`FakeTransportFactory` with `FakeClock`. The transport's loopback tests whose subject is
bytes (fragmented and coalesced delivery, peer close mid-frame, buffer bound, free-space
reads, send order, frame limit, elapsed deadlines, dropped `recv` and the worker-thread
echo) run on `ScriptedPeer`. Five tests
stay on raw sockets because a script cannot express them: the refused connect
(`connecting_to_a_closed_loopback_port_is_disconnected_before_the_deadline`, needs a
dropped listener), `the_opened_transport_reports_its_peer_and_limits` (no traffic),
`a_connection_reset_surfaces_as_disconnected` (needs `SO_LINGER` 0),
`shutdown_completes_tells_the_peer_and_closes_the_transport` (observes the half-close on
the accepted socket) and `a_send_cut_short_poisons_the_transport` (needs a peer that
never reads).

## Addendum (S18): the framing layer
Slice S18 adds `Framer<F>` and `FramedTransport<F, R>` to this crate (no ADR of its own:
the layer is the byte-stream half of the transport contract above, uses no new dependency,
and is not behind `tcp`, so `--no-default-features` and the testkit's
`default-features = false` edge still see it).

- **`Framer<F>: Send`** has `decode(&mut self, &mut BytesMut) -> Result<Option<F>,
  FrameError>`, `encode(&mut self, &F, &mut BytesMut) -> Result<(), FrameError>` and
  `max_frame_bytes(&self) -> usize`. Sizes are bytes on the wire, header included, the
  unit of `ResourceLimits::max_frame_bytes`. `decode` takes bytes off the front of the
  buffer only when it returns `Ok(Some(_))`; `Ok(None)` means more bytes are needed; an
  oversized declared size is `Oversized` and an invalid header `Malformed`, both
  reported as soon as the header is complete (before any payload is awaited or any
  allocation sized from the declared length) and both leaving the buffer untouched, so
  the error repeats until `resync` discards the offending bytes. A decoder never panics.
  `encode` only appends and appends nothing on error. The trait is object safe, but no
  `impl Framer<F> for Box<dyn Framer<F>>` exists; a slice that wants a boxed framer adds
  that blanket impl.
- **Deviations from the S18 text.** The type has a second generic parameter `R` (the
  framer) and `send_frame` takes `&F`, not `F`; S22 and S24b write
  `FramedTransport<Adu, MbapFramer>` and pass frames by reference.
- **`FramedTransport<F, R: Framer<F>>` is generic over the frame and framer and boxes only
  the byte pipe** (`Box<dyn Transport>`). A driver holds a concrete
  `FramedTransport<Adu, MbapFramer>`; a trait object would force every frame through
  `dyn Any` or a second boxed future for no gain. It owns the receive buffer (never
  longer than `limits().max_buffered_bytes()`, checked before every `recv`, so a full
  buffer without a frame is `ResourceLimit`), a scratch buffer for `send_frame`, and at
  most one frame recognised by `resync`. It has no `Debug` derive: the manual impl prints
  the peer and byte counts, never buffered bytes.
- **`next_frame(Deadline)`** decodes what is buffered first (a coalesced second frame
  costs no I/O and is returned even after the deadline), then loops `recv` and `decode`.
  Timeout, `Disconnected` on a 0-byte read (with or without a partial frame) and
  `ResourceLimit` are as the deliverable lists; transport errors pass through. It is
  cancel-safe because `Transport::recv` is and the bytes live in the buffer.
- **`send_frame(&F, Deadline)`** encodes at call time into the scratch buffer (so the
  returned future does not borrow the frame, and `F` need not be `Sync`), refuses an
  encoded frame over `min(framer.max_frame_bytes(), limits().max_frame_bytes())` with
  `ResourceLimit` before anything is sent, then makes one `Transport::send`. It inherits
  `send`'s non-cancel-safety and poisoning; `resync` is never the answer to a failed send.
- **`resync(Deadline)` and the boundary rule.** The framer's `decode` is the only judge,
  so a framer needs no extra method (the optional `skip_to_boundary` hook that was
  considered is omitted until a framer with a cheap sync-marker search needs it). Loop:
  `decode` the front of the buffer. `Ok(Some(frame))` is the boundary: the frame is
  held and returned by the next `next_frame` (so `resync` followed by `next_frame`
  loses nothing). `Ok(None)` with room in the buffer keeps the front (a header the
  framer accepts, or one too short to judge) and receives more; this is what lets a frame
  that was half received when its request timed out be completed instead of skipped.
  `Err(_)`, or `Ok(None)` with a full buffer (a header that can never complete in the
  buffer), discards exactly one byte and asks again. Every loop either discards a byte or
  calls `recv`, and `recv` checks the deadline on entry, so the loop ends. A frame held
  by an earlier `resync` is dropped first. Protocols with a sync marker (the MBAP
  protocol id) lock on reliably; a bare length prefix can lock onto a false header in
  garbage, so the caller verifies correlation and reopens the connection if it still
  gets a wrong frame. A late reply that is itself a whole valid frame is delivered by the
  next `next_frame`; the owner discards it by correlation (S24b `late_replies`).
- **Deadline source (D15).** `resync` receives, so it takes a `Deadline` and has no
  default. The rustdoc names the two sources: the remaining deadline of the request whose
  timeout triggered it, or `Deadline::after(clock.now(), timeouts.request)` when
  `ConnectionGuard::close` or recovery runs it (S19 builds that value; this crate has no
  clock of its own for it). The transport's `Timeout` at that deadline is turned into
  `Disconnected` (the original kept as the source): a stream that found no boundary in
  time cannot be trusted, and the owner reopens it. Other errors, including the peer
  closing during the resync, pass through.
- **Test framer.** `LengthPrefixedFramer` (testkit, feature `net`) frames `Vec<u8>` with a
  2-byte big-endian length, the sanctioned framer double of CODING_STANDARDS 13.5. Tests
  live in `crates/stonecrab-transport/tests/framed.rs` over `FakeTransport` and
  `FakeClock`; they carry `// covers: SPEC mandatory-failure-cases / F2 fragmentation and
  coalescing, peer close at each framing boundary`. Row ids: `TRANSPORT-FRAMING` and
  `TRANSPORT-RESYNC` in `docs/parity/transport.md`.

## Addendum (S19): the connection guard
Slice S19 adds `ConnectionGuard<F, R>`, `GuardConfig` and `GuardStats` to this crate (no
ADR of its own: it is the owner that D15 and the S16 and S18 text promised, uses no new
dependency, and is not behind `tcp`). It consumes the `sync` feature that decision 11
declared; the "declared and unused until S19" consequence above is therefore resolved.

- **Shape.** `ConnectionGuard<F, R: Framer<F>>` is built from a `GuardConfig { endpoint,
  timeouts, reconnect, limits }` (public fields, `GuardConfig::new(endpoint, limits)` for
  the defaults, `From<&ConnectionConfig>`; no `Credentials`), an `Arc<dyn
  TransportFactory>`, a `Box<dyn Fn() -> R + Send + Sync>` (a fresh framer per opened
  transport), an `Arc<dyn Clock>` and, unless `with_jitter_source` replaces it, a `Jitter`.
  Its operations are `connect`, `request(frame, deadline, token)`,
  `request_with_default_deadline(frame, token)`, `next_reply(deadline, token)`, `close`, `state`, `watch_state` and
  `stats`. **Default deadline:** `request` keeps an explicit `Deadline` (the caller always
  states the bound) and a sibling method applies the ADR-0007 rule
  (`Deadline::after(clock.now(), timeouts.request)`), instead of making the deadline an
  `Option`.
- **Machine.** Core's `ConnectionState` is the published state; `Connected` has the
  private sub-states `Idle`, `InFlight` and `Resyncing` (visible in `Debug` when the slot
  is free). `Closed` and `Failed` are terminal. `Reconnecting.attempt` is one-based (core
  documents it so) and the delay is `next_delay(attempt - 1, sample)`, so the schedule at
  sample 0.5 is 250, 500, 1000 ms; `is_exhausted(n)` is asked with the number of retries
  already scheduled, so `max_attempts = N` allows N retries after the first failure. An
  open that fails with `Timeout` or `Disconnected` backs off; any other kind is
  `Failed(kind)`. A peer close, a failed send or a failed resync drops the transport and
  backs off the same way.
- **Request-driven reconnect (D15).** Nothing runs between calls. The next `request` or
  `connect` sleeps until `next_at` with `Clock::sleep_until`, raced against its token,
  its deadline and the guard's own closing signal; it then opens within
  `min(timeouts.connect, request deadline)`, makes one attempt, and fails with the open's
  error if that attempt fails. An abandoned open (token, or the future dropped) puts the
  state back to what it was; an abandoned sleep changes nothing.
- **Queue.** Two `tokio::sync` primitives, no channel: a `Semaphore` with
  `max_pending_requests` permits is the admission gate (`try_acquire`; none left is
  `Backpressure` before any I/O), and a `tokio::sync::Mutex` over the connection state is
  the single in-flight slot (its waiters are served in arrival order). Both are released
  by drop on every path. The wait for the slot is raced against the token, the request
  deadline and `close`.
- **Deadline sources.** Queueing, backoff, open, send, resync and the reply wait all end
  at the request's deadline; the open is also limited by `timeouts.connect`; `close`
  (slot, shutdown and any resync it interrupts) is limited by
  `Deadline::after(clock.now(), timeouts.request)`.
- **Cancellation.** The token is honoured while queued, in the backoff, during the open,
  in a resync and while waiting for the reply. The send is not interrupted by the token
  (a half-written frame poisons the transport); a token that fired meanwhile is seen right
  after the flush. `close` does interrupt a send, because the transport is being
  discarded.
- **What a late reply does.** After a timeout, a cancel after the flush or a framing
  error, the guard stays `Connected` and marks the stream `Resyncing`. Before the next
  request, if the receive buffer holds bytes, it runs `FramedTransport::resync` within
  that request's deadline and discards the frame found there (the end of the abandoned
  reply, or debris); no resync runs when nothing is buffered. A late reply that arrives
  whole after that point cannot be told from the answer to the new request at framing
  level, so it comes out as that request's reply, and the session discards it by
  correlation (S24b `late_replies`). The guard has no correlation of its own, so it
  offers `next_reply(deadline, token)`, which reads a further frame without sending,
  under the same slot, deadline, token and close handling as `request`. The session
  drops a mismatched frame and calls it again, which realigns the stream.
- **Diagnostics.** Every error carries `Diagnostics` with the state at the moment of
  failure (before the guard reacts), the latency on the guard's clock, the reconnect
  attempt as `retries` and a category (`Validation` before any I/O, `Transport`
  otherwise). `operation` is `Connect` for a failed open and `Close` for `close`; it is
  unset for the frame exchange because the guard cannot tell a read from a write.
- **Testing the state sequence.** `StateWatch` coalesces (ADR-0007), so the tests collect
  every transition with a waker registered on `changed()` that reads the state on each
  `StateCell::set` and re-arms itself; sequences are asserted by value with the `Instant`s
  of `next_at`. No testkit change was needed; `FakeTransport::send` never waits, so a
  cancel or a `close` that lands in the middle of a write cannot be staged yet.
- **Named uncovered lines (ADR-0005).** Two lines of `guard.rs`, both in `transmit`.
  The first is `Err(Fault::lose(error))`, a send failure that is not a framer refusal:
  `FakeTransport::send` fails only after `recv` has observed the close, so the arm cannot
  be staged with the doubles. The second is the `Err(cut)` arm, a `close` that lands while
  a frame is being written: the double's `send` completes on its first poll and `close`
  cancels the guard's signal in the same step that sets `Closing`, so a single-threaded
  test cannot interleave them; the arm shares `stopped` with the covered reply wait. A
  failed send therefore has no test of its own that it drops the transport and backs off;
  the same drop-and-schedule path is covered for a failed receive. `scripts/coverage-gate.sh`
  reports transport at 1216 of 1220 lines (99.67%), the other two being the arms already
  named above.
- **Retry budget.** Only a received reply resets the attempt count, not an open, so a peer
  that accepts and then closes spends `max_attempts` and ends `Failed(Disconnected)`. An
  open cut short by the caller's own earlier deadline is a `Timeout` for that request and
  does not count as a failed attempt.
- **Not done here.** The per-operation tracing spans of CODING_STANDARDS 14.2 (decision 13
  above: the crate has no `tracing` dependency, and this slice adds none).
- **Rows and tests.** `TRANSPORT-GUARD-LIFECYCLE`, `TRANSPORT-RECONNECT` and
  `TRANSPORT-QUEUE` in `docs/parity/transport.md`; tests in
  `crates/stonecrab-transport/tests/guard.rs` tagged F2 delayed replies after timeout, F4
  cancellation (connecting, queued, transmitting, awaiting) and F5 reconnect (device
  restart, outage and recovery, exhausted budget, shutdown during reconnect).

## Addendum (S14): the facade `sim` feature and the example tooling
Slice S14 (the first Tokio edge outside the transport) adds the facade feature `sim`, the
facade's example and test dev-dependencies, and `examples/sim_read.rs`. It needs no ADR of
its own: the dependency policy it applies is this ADR's, and the example output rule is
ADR-0011 decision 8.

- **`sim` feature.** `sim = ["dep:stonecrab-driver-sim"]`; `default = []` is unchanged, so
  `cargo add stonecrab` still pulls `stonecrab-core` only (`cargo tree -p stonecrab -e
  normal` shows core, `bytes` and `thiserror` below the facade and nothing else; with
  `--features sim` the only addition is `stonecrab-driver-sim`, `tracing`, `tracing-core`,
  `pin-project-lite` and `once_cell`). `pub mod sim` is `#[cfg(feature = "sim")]` with
  `#[cfg_attr(docsrs, doc(cfg(feature = "sim")))]` and is `pub use
  stonecrab_driver_sim::*;`, a glob and not a list: the simulator crate is the owner of
  that surface, the facade pins it exactly, and a list would be one more place to forget a
  new item. `full` is not added yet (S30 adds `sensors` and `full`).
- **Pin style.** The workspace entry is `stonecrab-driver-sim = { path =
  "crates/stonecrab-driver-sim", version = "=0.1.0", default-features = false }` and the
  facade uses `{ workspace = true, optional = true }`. CODING_STANDARDS 20.1 says the facade
  pins its siblings so one facade version names one coherent set, so the new sibling is
  `=0.1.0`. This differs from `stonecrab-core`, `stonecrab-codec` and `stonecrab-transport`,
  whose workspace entries are the caret range `"0.1"` (15.4 and the existing foundation
  slices); those entries are not changed here. Section 15.4 forbids `=` pins "except
  temporarily"; 20.1 is the specific rule for facade-to-sibling edges and wins for this
  edge. The inconsistency is recorded rather than hidden: the `modbus` and `sensors`
  features should take the `=` pin too when they arrive, and a later slice may move core
  onto it.
- **Dev-dependencies of the facade.** `tokio` (`workspace = true`, features `rt` and
  `macros`; `#[tokio::main(flavor = "current_thread")]` and `#[tokio::test]`), `tracing`
  (the `info!` events) and `tracing-subscriber`. They are `[dev-dependencies]`, so the
  packaged runtime graph is unchanged, and the facade `include` list still ships `src/**`
  and the three files only: `examples/` and `tests/` are not in the archive.
- **`tracing-subscriber`.** Workspace entry `{ version = "0.3", default-features = false,
  features = ["fmt", "std"] }` (the minor-level range of the resolved 0.3.23). The
  resolved feature set is `alloc`, `fmt`, `registry`, `sharded-slab`, `std` and
  `thread_local`; `tracing-log` is off, so `log` stays out of the lock file. `fmt`
  requires `registry`, which requires `sharded-slab` (and `lazy_static` below it) and
  `thread_local`.
- **D23 result.** Run on the resolved tree (each name alone, as in decision 12):

  ```text
  $ cargo tree -e normal,dev -i once_cell
  once_cell v1.21.4
  `-- tracing-core v0.1.36
      |-- tracing v0.1.44
      |   `-- stonecrab-driver-sim v0.1.0
      |   [dev-dependencies]
      |   `-- stonecrab v0.2.0
      `-- tracing-subscriber v0.3.23
          [dev-dependencies]
          `-- stonecrab v0.2.0
  $ cargo tree -e normal,dev -i lazy_static
  lazy_static v1.5.1
  `-- sharded-slab v0.1.7
      `-- tracing-subscriber v0.3.23
          [dev-dependencies]
          `-- stonecrab v0.2.0
  $ cargo tree -e normal,dev -i thread_local
  thread_local v1.1.10
  `-- tracing-subscriber v0.3.23
      [dev-dependencies]
      `-- stonecrab v0.2.0
  $ cargo tree -e normal,dev -i log
  error: package ID specification `log` did not match any packages
  $ cargo tree -e normal,dev -i time
  error: package ID specification `time` did not match any packages
  $ cargo tree -e normal,dev -i chrono
  error: package ID specification `chrono` did not match any packages
  ```

  (The tree drawing is ASCII here; cargo prints box characters.) `lazy_static` is reached
  only through `sharded-slab`, as D23 predicted, so `deny.toml` now reads `{ crate =
  "lazy_static", reason = "use std lazy initialization in first-party code", wrappers =
  ["sharded-slab"] }`. Removing the wrapper makes `cargo deny check bans` fail with
  `crate 'lazy_static = 1.5.1' is explicitly banned`; the wrapper was restored. The
  `thread_local` path that D23 listed as possible and UNVERIFIED exists. `thread_local`
  is not in the ban list and depends on `cfg-if` only (not `once_cell`), so it gets an
  audit entry and no wrapper. `once_cell` keeps its single wrapper, `tracing-core`; the new
  `tracing-subscriber` edge reaches it through that same crate. `log`, `time` and
  `chrono` match no package. `docs/DEPENDENCY_AUDIT.md` has entries for
  `tracing-subscriber`, `sharded-slab`, `lazy_static` and `thread_local`.
- **Example rule (ADR-0011 decision 8).** `sim_read` installs the fmt layer in `main`
  (`with_writer(std::io::stdout)`, no timestamps), emits one `tracing::info!` per item with
  the fields `name`, `value`, `outcome` and `sampled_at` (microseconds since the example
  started), never prints with a print macro, and returns `std::process::ExitCode`; every
  error is mapped to `ExitCode::FAILURE` with an error event, with no `exit`, `unwrap` or
  panic. It uses only public items that the simulator's tests and the contract suite
  already exercise, so no item exists to serve it. The deadline is `Deadline::after(
  Instant::now(), 500 ms)`: the example has no clock handle, and the simulator's default
  `RealtimeClock` reads `Instant::now()`, so the two agree (D9).
- **Gates.** `scripts/coverage-gate.sh` excludes `examples/` by regex and keeps the facade
  at `na`; the `sim` re-export is one `pub use`, which has no executable line. CI gains an
  `examples` job and `mise run examples` the same two commands; the `features` job's
  powerset (`--no-dev-deps`) checks `sim` on its own and with every other feature.

## Alternatives considered
- **`async-std` or `smol`.** Rejected: CODING_STANDARDS 15.8 names Tokio as the sanctioned
  runtime, the later OPC UA and TLS crates in the ecosystem assume it, and a second
  runtime would split the test and dependency story.
- **`mio` directly.** Rejected: it would mean writing the readiness loop, the timer and the
  blocking-pool lookup that Tokio already provides, to avoid one dependency that `mio`
  would bring in anyway.
- **Blocking `std::net` sockets on a worker thread.** Rejected for TCP: section 8.4
  reserves the blocking-worker pattern for serial and bus devices, and a blocking read
  cannot honour a deadline without socket timeouts that cannot be cancelled with the
  future.
- **`tokio-util` (`CancellationToken`, codecs).** Not added. Core owns its own token
  (ADR-0007) and framing is hand-written over `BytesMut`; section 15.8 allows
  `tokio-util` in transport only, but nothing here needs it yet.
- **`impl Future` or `async fn` in the traits.** Rejected: not object safe (ADR-0007).
- **A `rand` dependency for jitter.** Rejected: D13 chose a small seeded xorshift; the
  generator is 15 lines and not security-sensitive.
- **Letting a timed-out `send` leave the transport usable.** Rejected: the peer may hold
  half a frame, and continuing would corrupt the next one.
- **`spawn` a reader task that fills a channel.** Rejected by D15: background tasks hide
  failures until a request arrives and need an owner to abort them.

## Compliance
- `crates/stonecrab-transport/tests/tcp.rs`, unit tests in
  `src/{deadline,error,jitter,tcp}.rs`, `tests/clock.rs` and `tests/public_types.rs`.
  Mandatory cases carry `// covers: SPEC
  mandatory-failure-cases / F2 transport ...` (refused connect, fragmented delivery,
  coalesced delivery, peer close mid-frame, buffer bound, frame bound, elapsed deadline,
  reset while sending, framing error mapping) and F4 for the cut-short `send`.
  Test ids are listed per row in `docs/parity/transport.md`.
- `scripts/coverage-gate.sh` gains the row `crates/stonecrab-transport|95`, with the
  comment naming the connect-timeout arm.
- `scripts/lint-attrs.sh` already permits the two promotions for `stonecrab-transport`
  and `lib.rs` uses exactly those: `missing_docs` and `clippy::indexing_slicing`.
- `packaging/stonecrab-transport.txt` and the `package` task and job pin the archive to
  `README.md`, `LICENSE`, `NOTICE` and `src/*`; `tests/` is not shipped.
- `deny.toml` bans the `full` feature of `tokio`; the `deny` job stays blocking. The D23
  command above is re-run whenever a dependency of the transport changes.
- `Cargo.lock` gains `tokio`, `tokio-macros`, `mio`, `socket2`, `pin-project-lite`, `wasi`,
  `windows-link`, `windows-sys` and `stonecrab-transport`.
