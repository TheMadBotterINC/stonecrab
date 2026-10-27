# 0015: Modbus TCP driver crate
## Status
Proposed, drafted in slice S20a on 2026-10-27; accepted when S27's interoperability job is
green. It adds the new crate `stonecrab-modbus` (CODING_STANDARDS 12.6 requires an ADR for a
new crate) and records decisions D3 (the Modbus part), D5, D6, D8, D16, D18 and D19 of
docs/ALPHA_PLAN.md. It builds on ADR-0005 (coverage tiers), ADR-0006 (`ByteOrder` and
`WordOrder`) and ADR-0013 (codec primitives). Only the address types and their text
grammar exist when this draft is written (S20a, S20b); every later slice named below adds
to this ADR's Compliance section rather than replacing it.

## Context
The alpha ships one real protocol: a native Modbus TCP driver that reads coils, discrete
inputs, holding registers and input registers. Its behavior is defined by the Apache PLC4X
Modbus driver at the commit pinned in `upstream/manifest.toml`, inventoried row by row in
`docs/parity/modbus.md`. Several decisions shape the whole crate and are cheaper to fix
before the first line of protocol code than to rediscover per slice: which transport and
framing are in scope, where the PDU definitions come from, which independent peer proves
interoperability, how an address text is interpreted, how device exceptions and batching
behave, and how the address types are modelled.

Two upstream facts drive the address model. The upstream grammar is one-based for people
(`holding-register:16`, `4x00016`) and subtracts one for the wire (`PROTOCOL_ADDRESS_OFFSET
= 1`, row `MB-ADDR-LONG`), and it accepts bare digit strings such as `40001` whose meaning
(one-based register, zero-based register, or a vendor scheme) is ambiguous
(`MB-ADDR-BARE-40001`). The unit identifier is one byte in the MBAP header, but upstream
parses it with `Short.parseShort` and the inventoried files show no range check
(`MB-CFG-UNIT-ID`).

## Decision
1. **Crate boundary and alpha scope (D5).** `stonecrab-modbus` is a published crate. At
   S20a its only dependency is `stonecrab-core`; the codec arrives with the PDU codec
   (S21), transport and Tokio with the session and connection (S24b, S25). Default
   features are empty. TCP with MBAP framing is the only transport. `pdu/` takes and
   returns PDU bytes without framing, `framing/` holds `mbap.rs` only, and
   `Crc16Modbus`, `rtu.rs` and the serial transport are M2. The alpha reads function
   codes 01 to 04 and decodes the exception PDU. There is no write on the wire (D3):
   `modbus-tcp` answers `write` with `Unsupported(Operation::Write)` and no I/O, and
   advertises `Capabilities.write = false`.
2. **Hand-written PDUs (D6).** The PDU types under `src/pdu/` (S21) are transcribed by hand
   from the pinned `modbus.mspec` (439 lines, ASF header). There is no `src/generated/`
   and no `@generated` marker; the MSpec feature matrix in `docs/parity/modbus.md` records
   "nothing generated". A future generator must reproduce this output or pass the same
   vector corpus.
3. **Interop peer (D8).** The independent peer is pymodbus 3.8.6 (BSD-3, the peer upstream's
   own Docker IT uses), run in a container built from a repository-owned Dockerfile
   (`python:3.12-slim` by digest) with a first-party `server.py`. Its register map equals
   the verified upstream fixture values plus one documented little-endian region (holding
   registers 300-301 = `0xDC0F 0x4940`, REAL 3.141593 under LITTLE_ENDIAN). It listens on
   TCP 5020 only. Nothing from upstream's `modbus_server.py` is copied. The job is S27 and
   is what turns this ADR from proposed to accepted.
4. **Vector redistribution.** Four upstream test suites (`ParserSerializerTestsuite.xml`,
   `DriverTestsuite.xml`, `Modbus-all-datatypes.xml` and
   `Modbus-all-datatypes-little-endian.xml`) are kept verbatim with their ASF headers
   under `fixtures/modbus/upstream/` by S04a, after the maintainer approval P4 and with
   `NOTICE` updated at first redistribution. `fixtures/modbus/PROVENANCE.toml` records
   source, commit and license. No published
   crate includes `fixtures/` (the `package` job proves it), so the redistribution is of
   the repository, not of the crates. S20a copies no upstream material; the crate `LICENSE`
   and `NOTICE` are copies of core's.
5. **Address scope and convention selection (D16).** The verified PLC4X grammar is the only
   text grammar at alpha. `AddressConvention` is `#[non_exhaustive]` and
   `Plc4xCompatible` is its only variant. The convention travels in core `AddressOptions`
   (S07), the second argument of `Driver::parse_address(text, &AddressOptions)`, so the
   driver stays a stateless factory. `ModbusTcpDriver` maps `convention == Some("plc4x")`
   to `Plc4xCompatible` and any other name to `InvalidAddress`. The unambiguous spellings
   (`holding-register:N`, `coil:N`, `discrete-input:N`, `input-register:N` and `0x`, `1x`,
   `3x`, `4x` plus digits) are accepted with or without a convention. The bare short forms
   (`40001`, `400001`, `00001`, `10001`, `30001`) are accepted only when a convention was
   supplied, and are otherwise rejected with `InvalidAddress` naming the ambiguity and the
   two accepted spellings (the bare-short-form rule; a divergence from upstream, row
   `MB-ADDR-BARE-40001`). Array selections, `STRING`, `WSTRING`, `CHAR`, `WCHAR`, the
   `TIME` and `DATE` family, `extended-register` and `6x` are typed parse-time rejections.
   A `Tag` whose `count()` is not 1 is rejected by the connection before I/O (S25),
   because `count` is set after parsing. S20a supplied the types and S20b the parser,
   `ModbusAddress::parse(text, &AddressOptions)`, which builds through the S20a builder so
   the span and bit-area rules stay in one place. S20b settled the following details,
   none of which changes the decision above:
   - A convention name other than `plc4x` is `InvalidAddress` for every spelling,
     including the unambiguous ones, so a misspelt option is never ignored. Every parsed
     address carries `Plc4xCompatible` whether or not a convention was named; the name
     decides what text is accepted, not what an accepted address means.
   - The bare short form is a prefix digit (`0`, `1`, `3`, `4`) plus 4 or 5 digits; the
     rejection message names the ambiguity and the two always-accepted spellings
     (`holding-register:N` and `4xNNNNN`) and the opt-in, `address-convention=plc4x`.
   - The logical address runs from 1 to 65536: the wire offset is the logical address
     minus one and wire address 65535 is reachable (`MB-LIMIT-SPAN`), so "above 65535"
     means a wire offset above 65535. The long form takes at most 9 digits, a short form
     exactly 4 or 5; the digit count is checked before any numeric conversion.
   - The optional trailing block is `{unit-id: N, byte-order: 'NAME'}` directly after the
     type. `unit-id` is a bare decimal integer from 0 to 255; `byte-order` is single- or
     double-quoted (the quoting is an inference from the upstream value pattern, an
     unquoted word being a silently dropped pair there but an error here). Spaces inside
     the braces are ignored (a Stonecrab choice; the upstream whitespace rules were not
     verified), keys are case-sensitive, unknown and repeated keys are errors and an empty
     block is accepted.
   - Input is at most 256 bytes (`ModbusAddress::MAX_TEXT_BYTES`), non-empty and free of
     control characters, and an error message echoes at most 32 control-free characters
     of it.
   - The legacy `AREA:N:TYPE[n]` form is rejected with a replacement computed from the
     text (`AREA:N[0..n-1]:TYPE`); the upstream wording of that hint is not retained, so
     the text is Stonecrab's.
6. **Exception mapping (D18).** The raw exception code and name are always kept in
   `ProtocolStatus`. The `ErrorKind` is: 1 ILLEGAL_FUNCTION to
   `Unsupported(Operation::Read)`, 2 ILLEGAL_DATA_ADDRESS to `InvalidAddress`, every other
   code, including 5 ACKNOWLEDGE, to `DeviceException`. Upstream maps 3 to INVALID_DATA and
   5 to OK (`MB-EXC-03`, `MB-EXC-05`); both are documented deviations with tests and
   Modbus Application Protocol V1.1b references. An acknowledge is never reported as
   success.
7. **Batcher (D19).** Coalescing lands in S24a, before the connection slice. Reads are
   grouped by (unit identifier, area), only adjacent or overlapping spans are merged (no
   gap reads, divergence `MB-READ-GAP`), chunk limits 125 registers and 2000 coils come
   from configuration and are enforced there and in S25 request validation, the
   `Capabilities.max_items_per_request` bound is honored and `TagId` membership is kept.
   S24a implements it as `Batcher::plan(&[Tag], &BatchLimits) -> Result<Vec<Chunk>>`.
   `BatchLimits` (registers, coils, items) refuses a configured bound above the protocol
   maxima of 125 and 2000. The item bound is checked against the whole plan before any
   tag is inspected (`ResourceLimit`), which also bounds every chunk. A `Tag` with
   `count() != 1` is `InvalidConfig`, a foreign address is `InvalidAddress`, and a tag
   wider than the bound is `ResourceLimit`. Each `Member` carries its offset in the chunk,
   its type and, as two independent fields, the byte order and the word order, each
   resolved from the tag over the address block; a field set by neither stays `None`
   for the connection default (D14).
8. **Address type model (this slice).** `address.rs` holds types only:
   - `UnitId(u8)`. A unit identifier is one MBAP byte, so every `u8` is valid and the
     newtype has no failing constructor (`From<u8>`, `get()`, `Display`). Its default is 1,
     the upstream `default-unit-identifier`. 0 (conventionally broadcast) and 255
     (conventionally "no unit" on TCP) are documented and carried unchanged, not rejected:
     Stonecrab attaches no behavior to either and a device behind a gateway may use
     them. Values outside 0..=255 cannot be expressed in the type; the parser (S20b, in
     place) and configuration (S26) reject them as `InvalidAddress` and `InvalidConfig`, a
     divergence recorded under `MB-CFG-UNIT-ID`. The conventional roles of 0 and 255 come
     from the Modbus Messaging on TCP/IP Implementation Guide and stay UNVERIFIED in the
     parity record.
   - `ModbusArea` and `ModbusDataType` are `#[non_exhaustive]` (CODING_STANDARDS 9.2)
     because `extended-register` and the string, character and calendar types may join
     them. `ModbusDataType` has exactly the 15 numeric and bit types; its
     `from_upstream_name` recognises all 27 upstream names, case-sensitively like
     `valueOf`, so that the 12 deferred names fail with a message naming the deferral
     rather than as unknown text.
   - `ModbusAddress` stores `offset: u16` as the zero-based wire address, the number that
     goes into the PDU, and `canonical()` renders the one-based logical address (offset
     plus one). The wire address is what the batcher, the PDU codec and every span
     computation use, so they never subtract one and cannot forget to; the logical
     address is what people write and read, so text and diagnostics use it. A `u16`
     makes "wire address above 65535" (`MB-LIMIT-ADDR`) unrepresentable, and logical 0
     unrepresentable too; the logical address is a `u32` (1 to 65536) because the highest
     address is 65536 once one is added.
   - `canonical()` renders the long form `AREA:LOGICAL:TYPE`, for example
     `holding-register:16:INT`, always with the type, followed by `{unit-id: N,
     byte-order: 'NAME'}` containing only the fields that are set, in that order, with the
     enum value single-quoted as in the upstream examples. The long form is chosen over
     the `4x00016` short form because it is unambiguous without a convention and never
     needs padding. `byte-order` is written with the upstream mode name derived from the
     (`ByteOrder`, `WordOrder`) pair (ADR-0006 table), and the pair participates in
     equality and hashing, unlike upstream's `equals` (`MB-QUIRK-L`).
     `prop_canonical_round_trips` (S20b) checks that this text parses back to an equal
     address under both an explicit and an absent convention, and the `fuzz_modbus_address`
     target asserts the same on every accepted input.
   - Bit areas accept only `BOOL`, checked when the address is built. Upstream parses a
     typed coil and fails at read time with UNSUPPORTED (`MB-ADDR-BIT-TYPES`); failing at
     build time keeps an address that can never be read out of the system. This is a
     documented divergence.
   - The span check is `offset + register_size <= 65536` (the last register is at most
     65535). Upstream tests `address + registers > 65535` on the zero-based address,
     which by its own arithmetic also refuses a single element at wire address 65535
     (an INFERENCE, `MB-LIMIT-SPAN`). Stonecrab accepts that element, so `BOOL` at wire
     65535 and `LREAL` at 65532 are valid and `REAL` at 65535 is not. The 125-register and
     2000-coil quantity limits are not address checks; they belong to the batcher and
     request validation, since a single-element address cannot reach them.
   - Errors reuse `stonecrab_core::PlcError` with `ErrorKind::InvalidAddress` and a precise
     message. There is no crate-level address error type.
9. **Coverage row.** `scripts/coverage-gate.sh` gains `crates/stonecrab-modbus|100`. While
   the crate holds only pure modules a whole-crate 100 matches ADR-0005's pure-module tier.
   ADR-0005 sets the whole-crate tier at 95 once socket code exists, because the session
   and connection carry OS-dependent error arms; the row becomes 95 in the slice that
   adds them (S24b or S25), with `address`, `pdu`, `framing`, `batch` and `decode`
   still held to 100 by review of `target/coverage.lcov`, and every uncovered line named
   in that PR.
10. **Transport injection point (S26).** `ModbusTcpDriver::new()` uses the real TCP
    factory. `ModbusTcpDriver::with_transport_factory(Arc<dyn TransportFactory>)` is the
    sanctioned way to run the testkit `FakeTransportFactory` under the driver. S25b lands
    it as a crate-private constructor for the contract suite and S26 makes it public and
    adds the driver to `tests/public_types.rs`. A global registry of test transports or a
    Cargo feature for fakes is not used.

## Consequences
- The rest of the crate builds on a small, fully tested vocabulary. The parser has
  nothing to decide about types, areas, bounds or rendering; it maps text to builder
  calls and is judged by the round-trip property.
- Wire-address storage keeps off-by-one handling in one place (`logical_address()` and
  the parser) at the cost of a visible difference between what a caller writes and what
  `offset()` returns. Rustdoc says so on both items.
- `UnitId` having no failing constructor means range errors come only from text and
  configuration parsing, so the same message wording must be kept in two places (S20b and
  S26); the divergence row records both.
- Rejecting typed coils at build time makes the "typed coil" divergence enforceable by
  construction, but a future write path that wants `INT`-packed coil words (some PLC
  gateways do) would need a new area or type, not a relaxation.
- Making `ModbusArea` and `ModbusDataType` non-exhaustive costs downstream `match`es a
  wildcard arm and buys the M2 additions without a breaking release.
- The vector redistribution depends on a maintainer approval outside the code (P4). If it
  is withheld, S04a and every slice after it that consumes upstream vectors is blocked,
  not re-scoped.
- The ADR stays proposed until a real peer has answered reads, which is the evidence a
  protocol driver actually needs.

## Alternatives considered
- **Include RTU in the alpha.** Rejected: it needs a serial transport, a CRC and a
  different framer, and doubles the interop matrix for no alpha exit criterion. `pdu/`
  being framing-free keeps the door open.
- **Generate PDUs from the mspec.** Rejected: a generator plus its output review costs
  more than transcribing four request/response pairs and the exception PDU, and generated
  code is exempt from the style rules the rest of the crate must meet (CODING_STANDARDS
  1.2). The vector corpus is the contract a generator would have to meet later.
- **diagslave or a Rust Modbus crate as the interop peer.** Rejected: diagslave is
  a closed-source binary that the repository could not build from source; a Rust crate
  would share assumptions with this implementation, so an error in both would pass.
  pymodbus is an independent codebase and the peer upstream itself uses.
- **All address conventions, or the convention as driver state.** Rejected: the second
  option makes the driver stateful and makes the same text mean different things on two
  connections; accepting every convention multiplies the ambiguity the bare-short-form
  rule exists to remove. The per-call `AddressOptions` carrier keeps the choice explicit.
- **Store the logical address and subtract at use.** Rejected: every consumer would have
  to subtract, and a missed subtraction reads the neighbouring register while every test
  with a single address passes. Storing the wire address makes the unsafe direction the
  one that needs a call.
- **Render the `4x00016` short form as the canonical text.** Rejected: it carries the
  area in a digit, needs five-digit padding and reads as a bare number to a human. It
  stays an accepted input. The plan paragraph and `MB-ADDR-SHORT-4X` name this form; S20b
  and S31 reconcile them with this decision.
- **Reject unit identifiers 0 and 255.** Rejected: a bridge or gateway may route on either
  and the alpha cannot know the device; the cost of carrying them is nil.
- **Accept non-BOOL types on bit areas and fail at read time, as upstream does.**
  Rejected for the reason in decision 8.
- **A crate-level `AddressError`.** Rejected: `PlcError` with `InvalidAddress` already
  carries the kind callers match on, and a second error type would need its own
  conversion and tests for no new information.
- **A public Cargo feature that exposes `FakeTransportFactory` construction.** Rejected:
  features are additive (CODING_STANDARDS 9.9) and the injection point is an ordinary
  constructor, usable from the crate's own tests and the testkit alike.

## Compliance
- `crates/stonecrab-modbus/src/lib.rs` carries `forbid(unsafe_code)`, the `docsrs`
  `cfg_attr` and exactly the four permitted promotions (`missing_docs`,
  `clippy::indexing_slicing`, `clippy::arithmetic_side_effects`,
  `clippy::as_conversions`), each on one line; `scripts/lint-attrs.sh` already lists
  `stonecrab-modbus` with those four.
- `crates/stonecrab-modbus/tests/address.rs` holds the type tables (every `ModbusArea`
  method, every `ModbusDataType` method for all 15 types, all 27 upstream names accepted
  or refused), the builder acceptance and rejection cases (BOOL on a coil, INT on a coil,
  REAL at 65535, LREAL at 65532 and 65533, BOOL at 65535 on a discrete input),
  canonical rendering with and without unit identifier and byte order for all four
  upstream byte-order names, the `Arc<dyn ProtocolAddress>` downcast and the `Debug`
  text. `tests/public_types.rs` asserts `Send + Sync + 'static` for every public type.
  Tests carry `// covers:` tags naming parity row ids.
- `# Compatibility: docs/parity/modbus.md#<row-id>` sections cite `MB-ADDR-LONG`,
  `MB-TYPE-BOOL`, `MB-CFG-UNIT-ID`, `MB-BYTEORDER-PRECEDENCE`, `MB-LIMIT-ADDR` and
  `MB-LIMIT-SPAN`; S20b adds `MB-ADDR-SHORT-4X`, `MB-ADDR-BARE-40001`,
  `MB-ADDR-CONFIG-BLOCK`, `MB-ADDR-BIT-TYPES`, `MB-ADDR-SELECTION`, `MB-ADDR-STRING`,
  `MB-ADDR-EXTENDED`, `MB-ADDR-LEGACY-ARRAY`, `MB-TYPE-TIME`, `MB-QUIRK-G` and
  `MB-LIMIT-DIGITS`; S31 verifies that every cited id resolves.
- S20b: `crates/stonecrab-modbus/tests/parse.rs` holds the grammar tables and every
  rejection named in decision 5, each tagged with the parity row it verifies or
  `// covers: SPEC mandatory-failure-cases / F3 invalid register`, plus the
  CODING_STANDARDS 13.7 boundaries (empty, 255, 256 and 257 bytes, 3, 4, 5 and 6 digits,
  nine and ten digits, every invalid area and prefix discriminator);
  `tests/properties.rs` holds `prop_parse_never_panics` and `prop_canonical_round_trips`;
  `fuzz/fuzz_targets/fuzz_modbus_address.rs` (six seed files) runs under both
  conventions. `proptest` is a dev-dependency of the crate only.
- `scripts/coverage-gate.sh` gains `crates/stonecrab-modbus|100` with a comment that the
  row becomes 95 when the session and connection modules land (decision 9). The
  `packaging/stonecrab-modbus.txt` list and the `package` task and job pin the archive to
  `README.md`, `LICENSE`, `NOTICE` and `src/*`; `tests/` is not shipped.
- Later slices extend this section: S21 (vectors per variant, no `generated/`), S24a
  (batcher property), S24b (exception mapping tests for codes 1 to 8 and 10 to 11), S26
  (`with_transport_factory` in `tests/public_types.rs`) and S27 (the interop job and its
  evidence artifact, which moves this ADR to accepted).
- The `deny` job stays blocking. This slice adds no dependency, so the D23 check is
  unchanged: `stonecrab-modbus` resolves only `stonecrab-core` and its existing graph.
