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
   (S21), `stonecrab-transport` (the `Framer` trait only, without Tokio's `net`) with the
   framing (S22, decision 11), Tokio's `net` with the session and connection (S24b,
   S25). Default features are empty. TCP with MBAP framing is the only transport. `pdu/`
   takes and returns PDU bytes without framing, `framing/` holds `mbap.rs` only, and
   `Crc16Modbus`, `rtu.rs` and the serial transport are M2. The alpha reads function
   codes 01 to 04 and decodes the exception PDU. There is no write on the wire (D3):
   `modbus-tcp` answers `write` with `Unsupported(Operation::Write)` and no I/O, and
   advertises `Capabilities.write = false`.
2. **Hand-written PDUs (D6).** The PDU types under `src/pdu/` (S21) are transcribed by hand
   from the pinned `modbus.mspec` (439 lines, ASF header). There is no `src/generated/`
   and no `@generated` marker; the MSpec feature matrix in `docs/parity/modbus.md` records
   "nothing generated". A future generator must reproduce this output or pass the same
   vector corpus. S21 landed the PDU types and settled the following details, none of
   which changes the decision above:
   - The module is `src/pdu.rs` with children `src/pdu/code.rs`, `src/pdu/model.rs` and
     `src/pdu/codec.rs`, not `src/pdu/mod.rs`: `clippy::mod_module_files` is denied
     (CODING_STANDARDS 10.4). `stonecrab-codec` is now a dependency, and
     `stonecrab-testkit` with its `fixtures` feature a dev-dependency for the golden
     loader; no new external crate.
   - `FunctionCode` names 0x01 to 0x04 and carries any other number as `Unknown(u8)`;
     `ExceptionCode` names the ten V1.1b section 7 codes (1 to 8, 10, 11) and carries any
     other number as `Unknown(u8)`. Both are `#[non_exhaustive]`, as is `ModbusPdu`.
   - `ModbusPdu` has the four read requests (`start`, `quantity`: the zero-based wire
     address and the count), `ReadBitsResponse { function, bytes }`,
     `ReadRegistersResponse { function, registers }` and `Exception { function, code }`.
     A bit response keeps the packed bytes exactly as received, padding bits included,
     because the response does not repeat the request's quantity: a decoded response
     re-encodes to the same bytes, which is what `fuzz_modbus_pdu` asserts, and the
     caller unpacks with `unpack_bits_lsb_first` and the request's quantity.
   - `decode_response(reader, expected)` takes only the function, so it checks what the
     bytes can prove: PDU at most 253 bytes, byte count 1 to 250 (even for registers),
     data exactly as long as the count, no trailing bytes, and the function (with or
     without the high bit) equal to `expected`. `ModbusPdu::check_reply_to(&request)`
     proves the rest once the session has the request: the byte count against
     ceil(quantity / 8) or `2 * quantity`, and the function. The errors reuse
     `ParseError`: a short payload is `Truncated`, a long one `TrailingBytes`, a count
     over 250 `OversizedLength` (checked before the data is read or allocated), a zero or
     odd count and a wrong function `InvalidDiscriminator` with the field named.
   - Write PDUs are not transcribed (D3). Function codes 05, 06, 15, 16, 20 and 21 decode
     as `Unknown`; every decoder refuses them (`unsupported function code` when the
     caller expects one, `function code` when it expects a read and gets one).
   - `decode_request` and `encode_response` exist for the responder of the contract suite
     (S25b, S27) and the fuzz target; `decode_request` applies the quantity bounds and
     the `start + quantity <= 65536` rule that `encode_request` applies, so what the
     encoder writes the decoder reads.
   - Until maintainer prerequisite P4 is granted, every golden vector under
     `fixtures/modbus/tcp/` is first-party: `protocol_spec` bytes written out from the
     V1.1b examples, or `adversarial` bytes. The upstream FC04 request and response and
     the four FC20/FC21 ADUs join them as `upstream_fixture` and `must_fail` vectors when
     P4 is granted; the vector directory's schema is in `fixtures/modbus/PROVENANCE.toml`.
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
   success. The codec supplies the number and the name (`ExceptionCode::name`, V1.1b
   section 7); the mapping to `ErrorKind` is the session's (S24b). S21 narrows "every other
   code": the ten defined codes map as above, but `decode_response` refuses an exception
   response whose code is none of the ten (0, 9, 12 and above) as `InvalidDiscriminator`
   with the number in the error, because a reply the codec cannot name is not trusted to
   be an exception. If the maintainers prefer a `DeviceException` that carries the raw
   number, only `decode_response`, `encode_response` and the two `must_fail` vectors
   `tcp/response-exception-code-0c` and `tcp/response-exception-code-09` change.
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
11. **MBAP framing and the transport edge (S22).** `src/framing.rs` with the single child
    `src/framing/mbap.rs` holds `MbapHeader`, `TransactionId`, `ModbusAdu` and `MbapFramer`
    (`mod.rs` is denied, as for `pdu/`). `stonecrab-modbus` gains a dependency on
    `stonecrab-transport` and on `bytes`, the edge `MODBUS --> TRANSPORT` of
    ARCHITECTURE.md section 1. The dependency is declared with `default-features = false`
    in `[workspace.dependencies]`, mirroring `stonecrab-core`: the `Framer` trait and
    `FrameError` need no `tcp` feature, so the Modbus crate does not pull Tokio's `net`
    feature; the driver crates enable `tcp` where they build the real factory (S26). The
    decisions this slice settles:
    - An ADU is an `MbapHeader` and the PDU as opaque `Bytes`. PDU decoding stays in
      `pdu/` and runs after the session has matched the reply to its request (S24b), so
      the framer needs no function-code knowledge and a malformed PDU never desynchronizes
      the stream.
    - `MbapFramer::decode` waits for 7 header bytes (`Ok(None)`), then judges the header
      alone, before it allocates or awaits the body: a protocol id other than 0 and a
      length of 0 or 1 are `FrameError::Malformed` (the protocol id first, because a
      header that is not Modbus has no meaningful length), a length above 254 is
      `FrameError::Oversized` carrying the declared ADU size (6 plus the length, so
      255 is 261). Only then does it wait for the body, and it consumes exactly the ADU.
      `max_frame_bytes()` is 260. The bound is the V1.1b section 4.1 PDU limit of 253
      bytes plus the header; upstream sizes a frame from the length field with no upper
      bound (`MB-LIMIT-ADU`, a divergence row).
    - `stonecrab_codec::ParseError` from the bounded reader converts into `FrameError`
      by a private function in `mbap.rs` (decision D22: `FrameError` is the transport's
      own type and the orphan rule forbids a `From` impl): `Truncated` to `Truncated`,
      `OversizedLength` to `Oversized(length)`, every other variant to `Malformed`.
      `MbapHeader::decode` on a short slice is how `Truncated` is reachable; the streaming
      `decode` reports a short buffer as `Ok(None)`.
    - `ModbusAdu` has no public fields and one constructor, `ModbusAdu::new`, which keeps
      protocol id 0, a PDU of 1 to 253 bytes and the matching length together, so every
      ADU encodes. `MbapHeader`'s fields are public because a decoded header is plain
      data; `MbapHeader::encode` validates its own fields. `unit_id` is the `UnitId`
      newtype (CODING_STANDARDS 9.3 rejects a bare `u8` unit id in a public signature)
      and `transaction_id` is the wire `u16`, because a peer's reply may carry any id.
    - `TransactionId` is the request counter only: it starts at 1, `next()` returns the
      current id and advances, wrapping from 0xFFFE to 1, so 0 and 0xFFFF are never
      issued, as upstream's `getAndIncrement` with its reset at 0xFFFF. Where the counter
      lives (the driver, so ids continue across a reconnect) is S25's decision.
    - The ADU golden vectors are named `tcp/adu_*` in the same `fixtures/modbus` directory
      and are first-party (`protocol_spec` or `adversarial`) for the same P4 reason as
      the PDU vectors. `must_fail` there means "the framer yields no ADU": an `error`
      expectation, or `outcome: incomplete` for bytes a streaming framer waits on, which
      the tests check as `Ok(None)` with the buffer untouched. The PDU vector tests
      exclude the `adu_` names.

11. **Typed decoder (S23, amendment 2026-10-27).** `src/decode.rs` holds the unit struct
    `TypedDecoder` with three associated functions and no state. Nothing here changes
    decisions 1 to 10.
    - `decode_registers(registers, data_type, byte_order, word_order, raw_budget)` returns
      the `PlcValue` of `ModbusDataType::value_type` (BYTE and USINT `U8`, WORD and UINT
      `U16`, DWORD and UDINT `U32`, LWORD and ULINT `U64`, SINT `I8`, INT `I16`, DINT
      `I32`, LINT `I64`, REAL `F32`, LREAL `F64`, BOOL `Bool`). `decode_bits(packed,
      index, data_type, raw_budget)` reads one bit of a packed coil or discrete-input
      response, least significant bit first, and accepts `BOOL` only.
      `encode_registers(value, data_type, byte_order, word_order)` is the inverse for
      tests and the future write path; a value of another variant is `InvalidConfig`,
      never converted.
    - The layout is two independent axes (ADR-0006): the two bytes of a register are the
      value's bytes in order under `BigEndian` and swapped under `LittleEndian`; the
      registers of a value are most significant first under `HighWordFirst` and reversed
      under `LowWordFirst`. The four upstream modes are the four pairs of ADR-0006 and lay
      `0x01020304` out as `[1,2,3,4]`, `[4,3,2,1]`, `[2,1,4,3]` and `[3,4,1,2]`. The code
      normalises the registers to the value's big-endian bytes with `BoundedWriter` and
      reads them with `BoundedReader` (ADR-0013); floats go through `from_bits` and
      `to_bits`, so NaN payloads, infinities and the sign of zero survive bit for bit and
      integers are exact.
    - A lone BOOL, BYTE, SINT or USINT is the low-order part of the 16-bit value those
      rules lay out: bit 0 and the low byte of the register under `BigEndian`, bit 8 and
      the high byte under `LittleEndian`. The parity record states this for BIG_ENDIAN
      and LITTLE_ENDIAN (`MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`); for the two byte-swap
      modes it is an INFERENCE from the pair mapping. Padding bits are written as zero and
      ignored on decode, also an inference: the interop register map stores SINT -42 as
      the sign-extended `0xFFD6`, which decodes to -42 only if the pad byte is ignored.
      The `address.rs` rustdoc for `ModbusDataType::Bool` now says the same.
    - A register count other than `ModbusDataType::register_size` (and a bit index beyond
      the received bytes) is an `ItemError` of kind `MalformedData` with
      `ProtocolStatus::new("modbus", 0).with_name("decode")` and the received registers as
      bytes, each register high byte first as on the wire, attached through
      `with_raw(bytes, raw_budget)` (D14, ADR-0006 decision 5). The budget is checked
      before the bytes are copied; when they do not fit, the error keeps the status
      without raw bytes (`raw()` is `None`) instead of a truncated copy. The caller passes
      `limits.max_frame_bytes()`. A non-BOOL type on a bit area is `InvalidConfig` with
      the same status and no raw bytes.
    - Vector provenance. `fixtures/modbus/tcp/typed_*` (85 vectors: 60 matrix vectors of
      15 types in each of the four modes, 14 special values, 6 interop constants and 5
      `must_fail` register-count cases) are first-party. The `.hex` holds register bytes.
      The matrix and special values were produced by a throwaway generator, not kept in
      the repository, that applies the documented byte permutations to one value's
      big-endian bytes and shares no code with the decoder; the interop vectors copy the
      words of `fixtures/modbus/interop/register-map.toml`. The sources are
      `protocol_spec` (origin: "derived from the documented 4-byte layouts in
      docs/parity/modbus.md Byte order and ADR-0006") and `adversarial`. The upstream
      little-endian all-datatypes vectors are not present because maintainer prerequisite
      P4 is open: the four byte-order rows are therefore `implemented`, not
      `automated-verified`, and the S23 exit criterion "little-endian upstream vectors
      pass" is not met. It is met by adding `ADT` and `ADTLE` as `upstream_fixture`
      vectors once P4 is granted.

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
- S21: `crates/stonecrab-modbus/tests/pdu.rs` holds the CODING_STANDARDS 13.7 list for
  `decode_request` and `decode_response` (zero length, one byte short at every length,
  exact, one byte long, the quantity and byte-count maximum plus one, every invalid
  function discriminator, every undefined exception code, every odd register byte count,
  PDU 254 bytes), the code tables and the encoders; `tests/pdu_vectors.rs` loads
  `fixtures/modbus` through `stonecrab_testkit::load_dir`, compares every decoded vector
  with its `expected.json`, encodes the PDU the expected file describes and
  compares the bytes with the `.hex` (two separate assertions), and checks every
  `must_fail` vector for the named `ParseError` variant and each of its parameters;
  `tests/pdu_properties.rs` holds `prop_register_count_bounds`,
  `prop_request_round_trips_exactly_when_it_is_legal` and
  `prop_decode_never_panics_and_accepted_bytes_re_encode`. `fuzz_modbus_pdu` (six seed
  files) decodes arbitrary bytes as a request and as a response to each read function and
  to two unsupported ones, and asserts that an accepted PDU re-encodes to its input. Tests
  carry `// covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected
  function codes` and the parity row ids. The `# Compatibility` sections cite
  `MB-TCP-READ-COIL`, `MB-TCP-READ-DI`, `MB-TCP-READ-HR`, `MB-EXC-01`, `MB-LIMIT-COILS`,
  `MB-LIMIT-REGISTERS`, `MB-LIMIT-PDU` and `MB-READ-MATCH`. The D23 command
  (`cargo tree -e normal,dev -i once_cell -i lazy_static -i log -i time -i chrono`, run one
  name at a time because cargo stops at the first unmatched spec) finds `lazy_static`,
  `log`, `time` and `chrono` absent and `once_cell` only through `tracing-core` and
  `stonecrab-driver-sim`, the wrapped path of ADR-0012: the slice adds no external
  dependency and no new path.
- S23: `crates/stonecrab-modbus/tests/decode.rs` loads the `typed_*` vectors through
  `stonecrab_testkit::load_dir`, decodes each accepted vector and compares the value with
  its `expected.json`, authored from the stated byte pattern (floats by bit pattern),
  encodes the expected value and
  compares the bytes with the `.hex` (two separate assertions), and checks every
  `must_fail` vector for `MalformedData`, the status name and the raw length. It also
  holds the layout tests for the four modes (`decode_dword_layout_2143`, `_3412`,
  `decode_lword_layout_21436587`, `_78563412`), the single-register padding tests, the
  boundary, NaN, infinity and signed-zero tests, the wrong-register-count and bit-area
  rejections with the raw bytes asserted, the interop map decoded entry by entry, and
  `prop_encode_decode_exact_integers`, `prop_encode_decode_floats_bitwise` and two more
  properties. Tests carry `// covers: SPEC mandatory-failure-cases / F7 negative and
  scaled values, swapped words, integer boundaries` and the parity row ids. The
  `# Compatibility` sections cite `MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`,
  `MB-BYTEORDER-BE-SWAP`, `MB-BYTEORDER-LE-SWAP`, `MB-TYPE-BOOL` to `MB-TYPE-LREAL` and
  `MB-ADDR-BIT-TYPES`. `tests/pdu_vectors.rs` now skips the `tcp/typed_` names. The slice
  adds no dependency: `stonecrab-codec`, `stonecrab-core` and the existing
  dev-dependencies are all it uses, so the D23 check is unchanged.
- Later slices extend this section: S21 (vectors per variant, no `generated/`), S24a
  (batcher property), S24b (exception mapping tests for codes 1 to 8 and 10 to 11), S26
  (`with_transport_factory` in `tests/public_types.rs`) and S27 (the interop job and its
  evidence artifact, which moves this ADR to accepted).
- The `deny` job stays blocking. This slice adds no dependency, so the D23 check is
  unchanged: `stonecrab-modbus` resolves only `stonecrab-core` and its existing graph.
- S22: `crates/stonecrab-modbus/tests/framing.rs` loads the `tcp/adu_*` vectors through
  `stonecrab_testkit::load_dir` and asserts decode-compare (the ADUs and leftover bytes the
  expected file lists), re-encode-compare (the ADUs built from the expected file encode to
  the `.hex`, a separate assertion), the byte-at-a-time feed of every accepted vector,
  and, for every `must_fail` vector, the named `FrameError` variant or `Ok(None)` with the
  buffer unchanged. It also holds the CODING_STANDARDS 13.7 list for `MbapFramer::decode`
  (empty, 1 to 6 bytes, seven bytes with length 2 and the body one byte short, the exact
  minimal ADU, one extra byte left in the buffer, length 254 with its full body accepted
  at 260 bytes, 255 rejected, protocol id 1 rejected), the `TransactionId` tests
  (`txn_ids_start_at_1`, `txn_ids_wrap_from_fffe_to_1`, a full cycle that never yields 0 or
  0xFFFF) and `FramedTransport` over the testkit `FakeTransport` (a fragmented ADU
  reassembled, coalesced ADUs delivered one at a time, a truncated ADU then close, bad
  headers as `MalformedData` and `ResourceLimit`, `resync` past a bad header, and the
  encoded bytes on the wire). Tests carry `// covers: SPEC mandatory-failure-cases / F1
  truncated headers, oversized lengths` and the F2 fragmentation tag. `fuzz_modbus_mbap`
  (thirteen seed files taken from the vectors) feeds arbitrary bytes to `decode` and
  asserts that an accepted ADU re-encodes to the bytes it consumed, that the buffer
  never grows and that a wait or an error leaves it untouched. The `# Compatibility`
  sections cite `MB-READ-FRAMING`, `MB-READ-TXN`, `MB-LIMIT-ADU` and `MB-READ-MATCH`.
  `stonecrab-testkit` gains the `net` feature as a dev-dependency (the `FakeTransport`)
  and `serde_json` is a dev-dependency for the expected files. The D23 check
  (`cargo tree -e normal,dev -i <name>`, one name at a time) still finds `lazy_static`,
  `log`, `time` and `chrono` absent and `once_cell` only through `tracing-core` and
  `stonecrab-driver-sim`; the only new crate edge is `stonecrab-modbus` to
  `stonecrab-transport`, and the transport crate builds with its default features off.
