# 0022: Lists and array selections
## Status
Accepted (2026-10-28), as slice S48 of docs/POST_ALPHA_PLAN.md (finding G08, the list
part). It is the later decision that ADR-0002 and ADR-0006 decisions 5 and 6 defer the list
value to, and it implements the parity rows `MB-ADDR-SELECTION` and `CORE-VALUE-LIST`.
Every change is additive to the published `stonecrab` 0.2.0. Byte-buffer and structured
values stay deferred (S49 decides `PlcValue::Bytes`).

## Context
SPEC.md ("Values and measurements") asks the value model to carry arrays. The pinned
upstream (Apache PLC4X v1.0.0, commit ea229918) has one list value, `PlcList`, a
`java.util.List<PlcValue>` whose type is `PlcValueType.List` (0x62), and one array notation
that every driver shares (`ArrayNotationParser`, `docs/parity/modbus.md` key `ANP`):

- `{area}:{address}[selection]:{TYPE}{config}`, the selection between the address and the
  type (`Tag:47-48`);
- `[i]` selects one element and yields a scalar; `[lo..hi]` is inclusive and yields a list,
  even `[4..4]`, because the written form decides (`ANP:156-166`; `Tag:56-89, 285-292`);
- `;base` states the declared lower bound, default 0, so `[4..7;1]` is offsets 3 to 6
  (`ANP:31-47, 228-263`);
- Modbus addresses linear memory, so one dimension only (`AddressConstraints.SINGLE_DIMENSION`,
  `AC:42-43`);
- an element offset is converted to registers through the element's size, and the request
  limits are counted in registers, not elements (`Tag:99-137`; `TagHR:93-106`;
  `TagCoil:89-98`);
- the pre-migration forms `address:TYPE[n]` and `address:TYPE:n` are rejected with the
  rewritten address in the message (`ANP:81-138`).

The alpha refused every selection at parse time and had no list value, so an upstream
address with a selection, and the one write and one read of `DriverTestsuite.xml` that use
one (`holding-register:1[0..1]:REAL`), could not be expressed. `Tag::count` exists in core,
but the Modbus driver refuses a count above one, and upstream has no count outside the
address.

## Decision
1. **One list variant in each enum.** `PlcValue::List(Vec<PlcValue>)` and
   `PlcValueType::List` are appended after `DateTime`, the last variants on master
   (CODING_STANDARDS 9.10). A list holds its elements in order. `PlcValueType` needs the
   variant for two reasons: `value_type()` is total and `const`, and an empty list (which a
   caller can build) has no element to take a type from; and a range address must report a
   type hint different from its element's, so that the request builder's existing check
   (`WriteRequestBuilder::item`, the tag's type against the value's) refuses a scalar
   written to a range, and a list written to one element, before any I/O, with no new
   validation path. It mirrors upstream `PlcValueType.List`.
2. **The element type is the tag's.** A driver fills a list with elements of the one type
   its address declares, as upstream does; the type does not check it. A checked
   homogeneous-list type was rejected: no second producer needs it, and the element type
   is already on the tag. A list can hold lists and can be empty; no Modbus path produces
   either, and a Modbus write refuses both (decision 6).
3. **Equality, text and order.** Equality is derived: element by element, in order, so a
   list holding a NaN is not equal to itself, as `F32` and `F64` are not, and a list of one
   is not its element. `PlcValue` has neither `Display` nor `Ord` (ADR-0006 decision 1),
   and `List` adds neither. `PlcValueType` keeps declaration order, `List` last. Nothing
   is coerced: `as_bool`, `as_i64`, `as_u64` and `as_f64` return `None` for a list. The
   size of `PlcValue` is unchanged (a `Vec` is the size of the `String` variant).
4. **Grammar.** `ModbusAddress::parse` accepts a selection after the number of every form
   (long, `Nx`, bare under a convention): `[i]` or `[lo..hi]`, either with `;base`, in
   decimal digits, each number checked into a `u32` (leading zeros allowed, as upstream's
   `Integer.parseInt` allows them). Typed `InvalidAddress` at parse time, with no I/O: any
   other bracket content, a second bracket group or a comma (a second dimension), an
   unterminated bracket, `hi` below `lo`, an index below the base, a number beyond `u32`,
   and the limits of decision 5. An index is resolved at parse time: `[i]` gives the same
   address as the element's own number (`holding-register:1[4]:DINT` equals
   `holding-register:9:DINT`), as upstream's tag equality has it. A range sets
   `ModbusAddress::elements` (`Option<NonZeroU16>`; `ModbusAddressBuilder::elements`
   builds one), whose type hint is `PlcValueType::List`. `canonical()` writes a range as
   `[0..N-1]` after the logical number of its first element, which is what upstream's
   `getAddressString` renders, and parses back to an equal address.
5. **The wire.** The first element sits at the address's wire offset plus `index - base`
   elements, an element being one bit on a coil or discrete input and
   `ModbusDataType::register_size()` registers on a holding or input register; a range
   spans `count` elements. The whole span must end at or before wire address 65535
   (`MB-LIMIT-SPAN`, which keeps address 65535 reachable) and fit one read: at most 2000
   bits or 125 registers, measured in registers (63 `DINT`s are 126 and are refused),
   checked when the address is built, as upstream checks at parse time
   (`MB-LIMIT-COILS`, `MB-LIMIT-REGISTERS`). A selection is one tag to the batcher: it
   merges with adjacent tags, is counted whole against the configured bounds
   (`max-registers-per-request`, `max-coils-per-request`; a list wider than the bound is
   `ResourceLimit` with nothing sent) and yields one read item. Each element is decoded as
   it would be alone (`TypedDecoder`) with the byte and word order the tag, then the
   address, then the connection resolves, so the orders apply per element; the first
   element that fails fails the item, and a device exception fails it whole.
6. **Writes.** A range takes a `PlcValue::List` of exactly its element count; each element
   is encoded as it would be alone and the run goes out as one request: function 15 for
   coils, function 16 for holding registers, and function 06 only where S45's
   single-register rule applies (a one-element list of a one-register type), as upstream
   picks by payload length (`Conn:449-465`). A scalar written to a range, or a list to one
   element, is refused by the request builder with `InvalidConfig`; a list of another
   length, or with an element of another representation (a nested list included), is the
   item's `Rejected` with `InvalidConfig` and no request: the kind and the place of the
   existing refusal of a value that does not fit its address. A list wider than one write
   PDU carries (124 registers or more, 1969 coils or more) reads but is `Rejected` with
   `ResourceLimit` as a write, from the PDU bounds of S45, with no request.
7. **What is kept from upstream.** The grammar, the inclusive range, the base, scalar or
   list by the written form, the element-size scaling of offsets, the register-unit
   limits, the single dimension, the rejected legacy forms with a rewritten address
   (`[0]` for a count of 0 or 1, as `ANP:120-123` writes it; a count that is not 1 to 9
   digits, or a range the parser would refuse, gets the general example instead), the
   function codes of a write, and the bytes
   of the `DriverTestsuite.xml` "Array element read request" and "Array element write
   request" exchanges.
8. **Divergences.** Each has a row in `docs/parity/modbus.md`:
   - *Sub-register elements.* A `BOOL`, `BYTE`, `SINT` or `USINT` element on a register
     area takes one register, laid out as the lone value is, so `[k]` and element `k` of a
     range read the same register. Upstream sizes the request at one register per element
     (`dataTypeSize` 2, `Mspec:381-395`; `Tag:132-137`) but decodes and encodes the run
     packed, two bytes or sixteen bits per register (`RegCodec:78-84, 112-119`), so it
     reads the first half of what it asked for, and its write fails its own length check
     (`Conn:459-463`) or, for two 8-bit elements, goes out as one function 06 register.
     The packed layout is not followed: it contradicts the index rule and the request
     upstream sends.
   - *A list of one.* `[4..4]` reads a list of one. Upstream's read path returns the
     element for any count of one (`RegCodec:58-61`; `Opt:156`), although its tag reports
     an array (`Tag:285-292`); Stonecrab follows the notation's rule.
   - *Write width.* A list wider than one write PDU is refused before sending; the
     inventoried upstream write path builds the request with no such bound
     (`Conn:438-465`; INFERENCE that it is sent, not observed by a run).
   - *Odd byte offsets.* Upstream refuses a selection that starts inside a register
     (`Tag:114-123`). With the 15 types of the alpha that cannot happen (each is a whole
     number of registers), so there is no check yet; it arrives with `CHAR` and `STRING`
     in S49.
9. **Unchanged.** `Tag::count` above one stays `InvalidConfig` on Modbus, with a message
   that names the selection. The simulator's `STATE` store holds any `PlcValue`, a list
   included, but the sim grammar has no list spelling, so a sim read of a list waits for a
   sim parity row (as for calendar values, ADR-0021 decision 8). The sensors module is
   unchanged: profile validation still refuses a `count` other than 1, and the pipeline
   treats a list as any value that is not a number (with scaling steps the measurement
   is `Bad`).

## Consequences
- Upstream addresses with a single-dimension selection parse, read and write; the two
  upstream array exchanges replay byte for byte.
- `ModbusAddress` gains `elements()`, `ModbusAddressBuilder` gains `elements()`, and the
  batcher's public `Member` gains an `elements` field (the struct is `#[non_exhaustive]`).
- The legacy-form hint changes for a count of 1 (`[0]`, as upstream) and offers no
  replacement for a count it cannot compute or a range the parser would refuse (wider
  than one read, past wire address 65535); every replacement it offers parses.
- A sensor profile whose address names a range passes validation today and then gets a
  list it cannot scale. The profile's `count` field may map to a selection later, or
  validation may refuse a range address; that is the sensors module's decision.
- S49 (`STRING`, `CHAR`) adds the odd-byte-offset check and the packed layout of
  characters, which is a property of strings, not of this rule.

## Alternatives considered
- **A `PlcList` newtype with a checked element type.** Rejected: one producer, and the
  check belongs to the driver that knows the wire type.
- **No `PlcValueType::List`, reporting the element type.** Rejected: an empty or nested
  list has none, `value_type()` must stay total, and the write builder could not tell a
  scalar from a list.
- **Upstream's packed layout for sub-register elements.** Rejected (decision 8): it reads
  data the request did not intend and makes element `k` of a range differ from `[k]`.
- **`Tag::count` as the element count.** Rejected: upstream has no count outside the
  address, so an address would not round-trip through the text, and two spellings of one
  read would have to agree.
- **Request limits as `ResourceLimit` at read time only.** Rejected: upstream refuses an
  over-wide selection at parse time, and an address no read can carry is invalid whatever
  the connection's bounds.

## Compliance
`crates/stonecrab/src/value.rs` tests the declaration order of `PlcValueType`, the
representation of lists, element-wise equality and the absence of coercion.
`crates/stonecrab/tests/modbus_selection.rs` tests the grammar, the offsets, the limits at
and past each bound, every legacy hint parsing, transcribed upstream cases
(`ArrayNotationParserTest`, `ModbusArrayParityTest`, `ModbusSelectionOffsetTest`,
`ModbusTagTest`, `ModbusLegacyAddressTest`, each naming its upstream file) and four
properties. `crates/stonecrab/tests/modbus_selection_io.rs` reads lists from all four areas
and writes them through `ModbusTcpDriver` over the fake transport, including the upstream
array read; `crates/stonecrab/tests/modbus_write_vectors.rs` replays the upstream array
write. `scripts/coverage-gate.sh` holds core and modbus at 100% lines.

## Addendum: selections on the extended registers (2026-10-29)
Slices S48 (this ADR) and S50 (the extended registers, the S50 addendum of ADR-0015) were
built apart and merged on 2026-10-29. Upstream gives the extended registers the selection
grammar of the other areas: `ModbusTagExtendedRegister` builds its patterns from
`Tag:47-48` (`TagExt:32-34`) and resolves a selection as the holding registers do
(`TagExt:95-108`), and `ModbusTagTest.testExtended_INT_ARRAY_RANGE` (`MTT`) reads
`extended-register:1[0..%d]:INT` in four spellings. Every decision above and every
decision of the S50 addendum stands; this addendum decides how they combine, and the S50
addendum points here.

1. **Grammar.** Every extended-register form (`extended-register:N`, `6xNNNNN`, the bare
   `6NNNNN` under a convention) takes `[i]`, `[lo..hi]` and `;base` between the number and
   the type (decision 4). The number is the register address under every convention (S50
   decision 2) and is checked before a selection moves it, as on the other areas:
   `extended-register:0[1]:INT` is refused because address 0 does not exist. Upstream
   checks the moved address instead (`TagExt:85-97`, `Tag:195-197`) and would take it as
   address 1 (INFERENCE from the code, not observed by a run).
2. **Offsets count registers through the files.** Element `k` starts `k` times
   `register_size()` registers after the number (decision 5), on the register address
   counted through all the files, not on the 16-bit record: `extended-register:1[4]:DINT`
   is address 9, as `ModbusSelectionOffsetTest.theOtherRegisterAreasScaleToo` asserts
   upstream, and `extended-register:9998[1]:DINT` is address 10000, record 0 of file 2.
   `offset()` and `file_number()` place the first register of a selection.
3. **An index is the element's own address.** `extended-register:9997[1]:DINT` equals
   `extended-register:9999:DINT`, so it is cut at the end of file 1 as that address is.
4. **The span.** The whole selected span, from the first register of the first element to
   the last register of the last, ends at or before 655349999 (S50 decision 3,
   `MB-LIMIT-EXT`); a first element past it is refused with the address it would start at.
5. **One read is 120 registers, not 125.** Decision 5 makes "fits one read" a property of
   the address, checked when it is built, and S50 decision 4 sets one file record read at
   120 registers, the most whose answer fits the 245 data bytes of a response wherever the
   span lies. A selection on the extended registers is therefore at most 120 registers
   (`MAX_READ_EXTENDED_REGISTERS`), `InvalidAddress` past it, where upstream allows 125
   (`TagExt:106-108`; `MTT` with `allowedMax` 125): a read of 125 registers asks for an
   answer of 252 data bytes in one file or 254 across the end of a file, which no device
   may send (V1.1b section 6.14). The message cites `MB-LIMIT-REGISTERS`, where the bound
   is recorded; the divergence row is `MB-ADDR-EXTENDED`. A configured
   `max-registers-per-request` below 120 bounds an extended list at read time as it bounds
   the other register areas (`ResourceLimit`, nothing sent).
6. **Reads.** A selection is one tag to the batcher over its register addresses: it merges
   with adjacent extended tags into chunks of at most 120 registers, and a chunk across the
   end of a file is one FC20 request of one sub-request per file. The registers of the
   sub-responses are joined in order before an element is decoded, so an element may take
   its high word from one file and its low word from the next. A device exception fails the
   whole list.
7. **Writes.** A range takes a `List` of exactly its element count, refused before any I/O
   otherwise (decision 6). The elements are laid out as they would be alone and go out in
   order as one FC21 request, cut at the end of each file into one sub-request per file and
   confirmed only by an echo of the whole request (S50 decision 5). FC21 carries 251 data
   bytes, 7 per sub-request beside 2 per register: every list inside one file fits (at most
   120 of the 122 registers one sub-request can carry), and a list across the end of a file
   fits up to 118 registers. A list of 119 or 120 registers across the end of a file reads
   but is the item's `Rejected` with `ResourceLimit` and no request, as a list wider than
   one FC16 is (decision 6).
8. **Canonical text.** `extended-register:A[0..N-1]:TYPE`, `A` the address of the first
   register, which parses back to an equal address under every convention.

Compliance: in `crates/stonecrab/tests/modbus_selection.rs`,
`upstream_extended_register_ranges_agree_across_forms_up_to_120` (the upstream test with
its selection, at 120 instead of 125),
`an_extended_selection_moves_the_register_address_by_whole_elements`,
`an_extended_selection_ends_at_or_before_655349999_and_fits_one_file_record_read`,
`prop_an_extended_selection_is_the_address_plus_index_times_width` and
`prop_an_extended_list_canonical_round_trips`; `crates/stonecrab/tests/modbus_extended_selection.rs`
reads and writes extended lists through `ModbusTcpDriver` on strict fake-transport scripts
and the first-party responder. One S50 test changed with its premise:
`an_extended_register_takes_the_types_a_holding_register_takes` (`tests/modbus_parse.rs`)
expected `extended-register:1[0..1]:INT` to be refused and now expects the list of two.
