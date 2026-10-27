# 0013: Bounded codec primitives
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S15 of docs/ALPHA_PLAN.md.
It adds the new crate `stonecrab-codec` (CODING_STANDARDS 12.6 requires an ADR for a new
crate), adopts `proptest` under ADR-0009, and implements decisions D22 and D23 for this
crate. It builds on ADR-0005 (coverage tiers) and ADR-0006 (`ByteOrder`).

## Context
Every protocol crate parses untrusted bytes: Modbus PDUs and MBAP headers now, serial
framing and OPC UA structures later. CODING_STANDARDS 7.5 requires that all such parsing
goes through a bounded reader, that a length field is compared with a caller-supplied
maximum before any `Vec::with_capacity` or `split_to`, and that parsers never index a
slice directly. 7.1 and 7.2 deny unchecked arithmetic and `as` casts in codec code, 7.4
requires byte order to be written out at every call, and 7.7 asks for named bit helpers
with an explicit bit order (Modbus coils are least significant bit first). The mandatory
failure case F1 names truncated payloads and oversized lengths. Without one shared
implementation each driver would re-derive these checks.

Decision D22 fixes the crate edges: codec depends only on core
(`CODEC --> CORE` in ARCHITECTURE.md section 1), transport defines its own `FrameError`,
and `MbapFramer` in the Modbus crate converts a codec `ParseError` into it.

## Decision
1. **Crate boundary (D22).** `stonecrab-codec` is a published crate whose only runtime
   dependency is `stonecrab-core`, used for `ByteOrder` (ADR-0006) and `PlcError`. It has
   no executor, no transport and no network dependency, and default features are empty.
   Transport does not depend on codec; it defines `FrameError {Truncated, Oversized(usize),
   Malformed}` itself, and the Modbus framer converts `ParseError` into it.
2. **Reader.** `BoundedReader<'a>` borrows a `&[u8]` and offers `remaining`, `read_u8`,
   `read_i8`, `read_u16`, `read_i16`, `read_u32`, `read_i32`, `read_u64`, `read_i64`,
   `read_f32`, `read_f64` (each multi-byte read takes a `ByteOrder`), `read_bytes(len,
   max)`, `read_bits_lsb_first(count, max)` and `expect_eof`.
   - `read_bytes` returns a sub-slice of the input, so it never allocates or copies. It
     checks `len <= max` and then `len <= remaining` before it touches a byte.
   - `read_bits_lsb_first` checks `count <= max`, then takes `ceil(count / 8)` bytes
     (failing with `Truncated` if they are not all present), and only then allocates the
     `Vec<bool>`, which is therefore at most eight times the input. Padding bits in the
     last byte are ignored, not validated.
   - A failed read never moves the cursor.
3. **Writer.** `BoundedWriter` has a fixed capacity and the matching `write_*` methods,
   `write_bytes`, `write_bits_lsb_first`, `capacity`, `remaining_capacity`, `as_slice`
   and `into_bytes`. A write is all or nothing: one that would pass the capacity fails
   with `OversizedLength { length: total, max: capacity }` before anything is copied, and
   the buffer is not pre-allocated.
4. **Errors.** `ParseError` is `#[non_exhaustive]` with `Truncated { needed, available }`,
   `OversizedLength { length, max }`, `InvalidDiscriminator { field, value }`,
   `TrailingBytes { remaining }` and `Overflow`. It implements `Display` and
   `std::error::Error` by hand (no new dependency) and carries sizes and field names only,
   never payload bytes. `InvalidChecksum` and `RecursionLimit` arrive with RTU framing and
   nested structures. `InvalidDiscriminator` is defined here and raised by the protocol
   crates that decode enumerations; no primitive in this crate produces it. `Overflow`
   guards the writer's length addition; it is not reachable with real buffers on 64-bit
   targets, so no test exercises it and its region is reported uncovered. The line gate
   still passes because the `ok_or` shares a line with covered code.
5. **`From<ParseError> for PlcError`.** Every variant maps to `ErrorKind::MalformedData`
   except `OversizedLength`, which maps to `ErrorKind::ResourceLimit` because it reports
   a configured or declared bound being exceeded. The message is the `Display` text and
   the `ParseError` is kept as the `source()`. The conversion matches every variant
   explicitly, so adding a variant forces a decision.
6. **Invariants.** (a) Bounds are checked before any allocation or copy. (b) No public
   function panics on any input: the crate denies `unwrap`, `expect`, `panic`, indexing,
   `arithmetic_side_effects` and `as_conversions`, and reads use `split_at_checked` and
   `split_first_chunk`. (c) All arithmetic is `checked_*`, `saturating_*` or
   `div_ceil`; the saturating use is `remaining_capacity`. (d) Byte order is an argument
   of every multi-byte call and is never inferred from the data. `ByteOrder` is
   exhaustive (ADR-0006 amendment 7 declares its two-variant shape final), so the
   crate matches both variants and no wildcard or default arm exists. (e) Floats are
   read with `from_be_bytes` / `from_le_bytes`, which
   preserve NaN payloads, infinities and signed zero bit for bit.
7. **Bit helpers.** `coil_byte_count(count)`, `pack_bits_lsb_first(&[bool])` and
   `unpack_bits_lsb_first(&[u8], count)` are public so the Modbus PDU codec can build
   requests and responses without re-implementing the packing. `coil_byte_count` returns
   a plain `usize` (`div_ceil` cannot overflow), not a `Result`, because a `Result`
   without an error path would trip `clippy::unnecessary_wraps`.
8. **`proptest` adoption (ADR-0009, D23).** `[workspace.dependencies]` gains
   `proptest = { version = "1.12", default-features = false, features = ["std"] }`,
   resolved to 1.12.0, and `stonecrab-codec` declares it as a dev-dependency only. The
   D23 check was run on the resolved tree (cargo reports the first unmatched package spec
   and stops, so each name was also run alone; each answers the same way):

   ```text
   $ cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono
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

   None of the five banned crates is in the resolved graph, so proptest 1.12.0's `std`
   feature does not enable `lazy_static`, the UNVERIFIED path named in ADR-0009 and D23
   does not exist for this version, and no `deny.toml` `wrappers` entry is added.
   `cargo deny --locked check` stays green. `fork` and `timeout` remain prohibited.
   `docs/DEPENDENCY_AUDIT.md` has the entry. CI keeps `PROPTEST_CASES=256`.
9. **Fuzz target.** `fuzz_codec_bounded_reader` (ADR-0009 layout: path dependency in
   `fuzz/Cargo.toml`, `[[bin]]`, five seed files in
   `fuzz/corpus/fuzz_codec_bounded_reader/`) treats the first byte as a script length (at
   most 31), the script bytes as operations with hostile length arguments (including 1
   GiB and `usize::MAX`), and the rest as the payload. It asserts that a failed read
   consumes nothing, a successful read consumes exactly its width, scalars re-encode to
   the bytes they came from, and the bit helpers agree with `coil_byte_count`. The
   `arbitrary` crate is still unused and stays on the machete ignore list of the fuzz
   manifest; `libfuzzer-sys` and `stonecrab-core` are now used and are removed from it.
   The 60 s smoke run executed 3,727,061 inputs without a finding.

## Consequences
- Protocol crates get one audited place for length checks; a driver that needs a field
  the primitives do not offer adds it here, not in the driver.
- `ParseError` is a leaf type shared across crates, so the Modbus crate can map it to
  `FrameError` and to `PlcError` without a dependency from transport on codec.
- `ByteOrder` is exhaustive, so the byte-order match in this crate is total and a
  future variant is a compile error here rather than a silent default.
- `read_bits_lsb_first` returns `Vec<bool>`, which spends a byte per bit. It is simple
  and bounded by `max`; a packed bit vector can replace it behind the same signature if
  profiling asks for it.
- The writer does not know about protocol limits; callers pass the PDU or ADU limit as
  its capacity.
- A hostile `count` that passes `max` but not the input still costs no allocation.

## Alternatives considered
- **`bytes::Buf` and `BufMut` directly.** Their getters panic when the buffer is short,
  which CODING_STANDARDS 6.1 forbids, and `split_to` and `copy_to_bytes` act on a length
  before any bound. A checked wrapper would be needed anyway.
- **`nom` or `winnow`.** Combinators fit declarative formats, but the alpha parsers are a
  handful of fixed headers and counted payloads; the dependency, its error types and its
  generic bounds are more surface than the checks they would replace, and neither forbids
  an allocation before a length check by construction.
- **The `byteorder` crate.** It offers extension traits whose reads return
  `io::Error` and whose writes assume an unbounded sink. `from_be_bytes` and
  `from_le_bytes` in `std` do the same job without a dependency.
- **Defining `ByteOrder` in codec.** Rejected: core already exports it (ADR-0006) and the
  tag model needs it there.
- **A `Result` from `coil_byte_count`.** Rejected, see decision 7.
- **A wildcard arm mapping unknown `ByteOrder` variants to big-endian, or an
  unsupported-order error.** Rejected: the first is an inference the design forbids and
  the second is an arm no test can reach under the 100% line gate. Making the
  two-variant enum exhaustive (ADR-0006 amendment 7) removes the problem.

## Compliance
- `crates/stonecrab-codec/tests/` holds the CODING_STANDARDS 13.7 list for every read
  function (`reader.rs`: zero length, one byte short, exact length, one byte long then
  `expect_eof`, maximum plus one for `read_bytes` and `read_bits_lsb_first`, the 1 GiB
  length against a 4-byte input), the writer capacity boundary (`writer.rs`), the bit
  helpers (`bits.rs`), the `PlcError` mapping (`error.rs`), `public_types.rs`
  (`Send + Sync + 'static` for every public type), and the property tests
  `prop_round_trip_exact_integers` and `prop_length_checked_before_allocation`
  (`properties.rs`). Boundary tests carry `// covers: SPEC mandatory-failure-cases / F1
  truncated payloads, oversized lengths`.
- `scripts/coverage-gate.sh` gains the row `crates/stonecrab-codec|100`.
- `scripts/lint-attrs.sh` already permits the four promotions for `stonecrab-codec`, and
  `lib.rs` uses exactly those: `missing_docs`, `clippy::indexing_slicing`,
  `clippy::arithmetic_side_effects` and `clippy::as_conversions`.
- `packaging/stonecrab-codec.txt` and the `package` task and job pin the archive to
  `README.md`, `LICENSE`, `NOTICE` and `src/*`; `tests/` is not shipped.
- `fuzz_codec_bounded_reader` runs in `scripts/fuzz-smoke.sh` (60 s per run, 30 min
  weekly). A crash input is committed under the target's corpus with a regression test
  in this crate, named as CONTRIBUTING "Fuzz findings" requires.
- The `deny` job stays blocking; the D23 command above is re-run whenever a dependency
  of the codec changes.
