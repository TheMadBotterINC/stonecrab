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
