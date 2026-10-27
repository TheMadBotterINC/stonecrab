# 0014: Transport trait, TCP transport and Tokio adoption
## Status
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
  different contract, which S19's `FramedTransport::resync` would add on top.
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
