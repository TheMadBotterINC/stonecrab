# 0025: Text and byte values
## Status
Accepted (2026-10-29), as slice S49 of docs/POST_ALPHA_PLAN.md (finding G08, the text and
bytes part). It implements the parity rows `MB-TYPE-CHAR`, `MB-TYPE-WCHAR`,
`MB-TYPE-STRING` and `MB-TYPE-WSTRING`, retires the divergence `MB-ADDR-STRING` (the
names were refused at parse time) and adds the odd-byte-offset rule that ADR-0022
decision 8 left for this slice. It decides `PlcValue::Bytes`: not now (decision 8). Every
change is additive to the published `stonecrab` 0.2.0.

## Context
The pinned upstream (Apache PLC4X v1.0.0, commit ea229918) defines four text types for
Modbus (`docs/parity/modbus.md`, file key `Mspec`, `Tag`, `RegCodec`):

- `CHAR` (`dataTypeSize` 1) is `simple string 8`, UTF-8; `WCHAR` (2) is `simple string
  16`, UTF-16BE; `STRING` (1 per character) is `vstring stringLength * 8`, UTF-8; and
  `WSTRING` (2 per character) is `vstring stringLength * 16`, UTF-16BE (`Mspec:365-376,
  404-407`).
- The length of one string is part of the address, `STRING(20)`, 1 to 5 digits
  (`Tag:45-48`), required for `STRING` and `WSTRING` and refused for every other type, and
  greater than zero (`Tag:203-221`). It is kept on the tag (`getStringLength`), not on the
  type.
- An element occupies `dataTypeSize` times the declared length in bytes; a selection
  offset is converted through the byte offset, an odd byte offset is refused, and a
  selection is rounded up to whole registers (`Tag:91-137`). Two `CHAR`s share one
  register.
- `ModbusRegisterCodec` leaves a lone `CHAR` unpadded and pads a packed run to a whole
  register (`RegCodec:135-139, 196-205`).
- The strings are read by `ReadBufferByteBased.readString`, which ignores the buffer's byte
  order ("Do not apply byte order to UTF-8 strings"), after the connection swapped each
  byte pair of the payload for the two `*_BYTE_SWAP` modes (`Conn:498-505`;
  `Opt:160-187`). The payload is cut to `getLengthBytes` before the swap, so an odd byte
  count keeps its last byte where it was.
- Decoding (`BaseStringEncoding.decodeString`) builds a Java `String` with the charset,
  which replaces bytes that are not valid UTF-8 or UTF-16 with U+FFFD, cuts at the first
  NUL, strips trailing spaces unless the text is all spaces, and removes every U+FEFF.
  Encoding refuses text that needs more bytes than the field, refuses characters the
  charset cannot encode, and pads with NUL bytes.

Upstream tests and vectors: `ModbusStringTest`, `ModbusSelectionOffsetTest`,
`ModbusRegisterCodecTest`, `ModbusEncodeTest` and `ModbusTagTest` in plc4j, and
`RegisterCodec_test.go` and `Reader_test.go` in plc4go. The retained XML suites
(`DriverTestsuite.xml`, the two all-datatypes files) contain no text.

Stonecrab had `PlcValue::Char(char)` and `PlcValue::String(String)` since the alpha, and
refused the four names at parse time. `ReadItem` carries a value or an item error and no
quality; `Quality` belongs to the sensors module. `clippy.toml` reserves
`String::from_utf8_lossy` for a `codec::text::decode_lossy` that marks
`Quality::Uncertain`, which does not exist.

## Decision
1. **Four unit variants and a length on the address.** `ModbusDataType` gains `Char`,
   `WChar`, `String` and `WString`, appended after `LReal` (CODING_STANDARDS 9.10). The
   declared length is `ModbusAddress::string_length()` (`Option<NonZeroU16>`), set by
   `ModbusAddressBuilder::string_length` or the parenthesised `(n)` of the text, and the
   batcher's `Member` carries it (`string_length`, a new field of a `#[non_exhaustive]`
   struct). A variant with the length inside (`String(NonZeroU16)`) was rejected: the
   published enum has only unit variants, and cargo-semver-checks reports a tuple variant
   there as a major change (`enum_discriminants_undefined_non_unit_variant`); upstream
   keeps the length on the tag too. `ModbusDataType::register_size` is 1 for the four types,
   a string of declared length one; the address scales it.
2. **Values.** `CHAR` and `WCHAR` read and write `PlcValue::Char`, `STRING` and `WSTRING`
   `PlcValue::String`; `value_type()` and the address's type hint follow. The value does
   not record the encoding, the address does, as `WORD` and `UINT` are both `U16`. The
   length counts encoded units, as upstream's field width does: `STRING(n)` is `n` bytes of
   UTF-8, `WSTRING(n)` is `n` UTF-16 code units.
3. **Grammar and sizes.** `TYPE(n)`, `n` 1 to 5 decimal digits whose value is 1 to 65535;
   `InvalidAddress` for `STRING` or `WSTRING` without a length, a length on any other type,
   0, more than 5 digits, above 65535, or anything else between the parentheses. An element
   occupies `dataTypeSize` times the length in bytes (`CHAR` 1, `WCHAR` 2, `STRING(n)` `n`,
   `WSTRING(n)` `2n`, and two per register of every numeric type), and an address occupies
   the bytes of its elements packed and rounded up to whole registers. That width is what
   the existing machinery counts: the span check, the read limit at build time
   (`STRING(250)` and `WSTRING(125)` fit 125 registers, `STRING(240)` and `WSTRING(120)`
   the 120 of the extended registers), the batcher and the configured bounds. Text is a
   register type: a bit area refuses it at parse time like every non-`BOOL` type.
4. **Reading.** The registers are read in order, never reversed. A `STRING` ends at its
   first NUL byte and a `WSTRING` at its first NUL code unit; what follows is padding and
   is not decoded. Everything before the NUL is the value, trailing spaces and U+FEFF
   included. A `CHAR` is its one byte and a `WCHAR` its one code unit, a NUL included
   (`PlcValue::Char('\0')`).
5. **Text that does not decode is an item error.** A byte sequence that is not UTF-8 (a
   `CHAR` above 0x7F included) or a code unit sequence with an unpaired surrogate fails the
   item with `MalformedData`, whose `ProtocolStatus` keeps the item's registers as received
   (within the 253-byte budget), as a wrong register count does (ADR-0006 decision 5, D14).
   Other items of the same request are not affected. The lossy path (U+FFFD, the raw bytes
   kept and `Quality::Uncertain`) was rejected for now: a read item has no quality to mark,
   adding one is a core change with no other producer, and a value with replacement
   characters is a different value that downstream code would take as read. If a device
   is found that stores another encoding, the remedy is an encoding the address names or a
   bytes value (decision 8), not a silent substitution.
6. **Writing.** The value must be the variant the type reads; nothing is converted. A
   `CHAR` takes U+0000 to U+007F, a `WCHAR` one UTF-16 code unit (no character outside the
   Basic Multilingual Plane), a `STRING(n)` at most `n` bytes of UTF-8 and a `WSTRING(n)`
   at most `n` code units; a string holding NUL is refused, because it would end the
   string when it is read back. Each element is padded with zero bytes to its width, as
   upstream pads, and the run to a whole register, so a lone `CHAR` or a `STRING(3)`
   writes a zero into the rest of its last register. A refusal is the item's `Rejected`
   with `InvalidConfig` and no request (ADR-0010), or with `ResourceLimit` when the address
   is wider than one write carries: more than the 123 registers of function 16
   (`STRING(247)` to `STRING(250)` and `WSTRING(124)` and `WSTRING(125)` read but are not
   written), the existing bound for a numeric list (ADR-0022). The function code follows
   the existing rule: function 06 for one register, 16 otherwise, 21 on the extended
   registers.
7. **Byte and word order.** Text is a run of bytes. The word order never reverses its
   registers. The two bytes of each register are taken high byte first, except under the
   two byte-swap modes, where the byte order and the word order disagree
   (`BIG_ENDIAN_BYTE_SWAP` is `LittleEndian` with `HighWordFirst`,
   `LITTLE_ENDIAN_BYTE_SWAP` is `BigEndian` with `LowWordFirst`): there they are taken low
   byte first. That is upstream's rule: `BIG_ENDIAN` and `LITTLE_ENDIAN` read strings in
   wire order, the swap modes swap each pair. One divergence: Stonecrab swaps whole
   registers, so the last byte of an odd-length value (a lone `CHAR`, a `STRING(3)`) comes
   from the swapped register, where upstream swaps only the bytes it cut and leaves the
   last one in place.
8. **Selections and the odd-byte-offset rule.** Element `k` starts `k` times the element's
   bytes after the address; a selection whose start is an odd number of bytes in, inside a
   register, is `InvalidAddress` at parse time (`[1]:CHAR`, `[1..2]:CHAR`, `[1]:STRING(3)`),
   as upstream refuses it. `[2..3]:CHAR` starts one register in. A list of text is packed:
   `CHAR`s two to a register and strings back to back, so element 1 of a `STRING(3)` list
   starts inside a register, and only the end of the run is padded. It is read and written
   in one piece, and its first element that does not decode fails the item. This is
   upstream's layout for the text types, whose `dataTypeSize` is 1 per character; the
   one-register-per-element rule of ADR-0022 decision 8 stays for `BOOL`, `BYTE`, `SINT`
   and `USINT`, whose `dataTypeSize` is 2. For the numeric types the byte offset is always
   even, so nothing changes for them.
9. **`PlcValue::Bytes`: not added.** This slice has no caller for it: the four types map
   to `Char` and `String`, text that does not decode is an error whose status keeps the
   bytes, and a run of `BYTE`s is already a `PlcValue::List` of `U8` (ADR-0022). Adding a
   variant without a producer would fix its shape (owned `Vec<u8>` or shared, a length
   bound, `Display`) without a use to check it against. What would need it: a protocol type
   that is an opaque octet string (S7 or OPC UA `ByteString`, a raw-register read mode, a
   file record read as bytes), or a decision to return undecodable text as data. That
   slice appends `PlcValue::Bytes` and `PlcValueType::Bytes` after `List` with its own ADR
   (row `CORE-VALUE-BYTES`, deferred).
10. **The public decoder.** `TypedDecoder::decode_registers` and `encode_registers` take
    the four types as one register: a `CHAR` or `WCHAR`, and `STRING` and `WSTRING` as
    `STRING(1)` and `WSTRING(1)`, the size `register_size` gives. A declared length and a
    list are applied by the connection, which reads and writes through the address.

## Consequences
- Upstream addresses with `CHAR`, `WCHAR`, `STRING(n)` and `WSTRING(n)` parse, read and
  write on holding, input and extended registers; the upstream string tests hold through
  the driver.
- `ModbusDataType` gains four variants, `ModbusAddress` and `ModbusAddressBuilder` gain
  `string_length`, `Member` gains `string_length`. `from_upstream_name` resolves the four
  names, and the 8 calendar names stay the only refused ones.
- Divergence rows record: invalid text is an error (upstream U+FFFD), trailing spaces and
  U+FEFF are kept and a NUL `CHAR` is a NUL (upstream trims, removes, and reads an empty
  string), the byte-swap modes swap whole registers, and a lone `CHAR` or an odd-length
  string is written padded where upstream's write fails its own length check.
- `MB-ADDR-STRING` is retired; `MB-ADDR-SELECTION` carries the odd-byte-offset rule.
- The pymodbus peer serves text registers from a sibling of the register map,
  `fixtures/modbus/interop/text-registers.toml`, because `stonecrab::testkit::InteropMap`
  refuses unknown types and keys; folding them into the map needs a testkit change.
- `fixtures/modbus/PROVENANCE.toml` names `interop/text-registers.toml` (first-party,
  Apache-2.0) in a comment, not a `[[file]]` entry: the testkit unit test
  `golden_dir_lists_every_vector_in_provenance` pins the `[[file]]` list, and the testkit is
  outside this slice. The entry and the one-path change to that test belong to the next
  slice that touches the testkit.

## Alternatives considered
- **Lossy decoding with `Quality::Uncertain`.** Rejected (decision 5): no quality on a read
  item, and the replacement characters would be taken as the device's text.
- **The length inside the variant.** Rejected (decision 1): a semver break of the
  published enum.
- **`CHAR` as Latin-1, any byte a character.** Rejected: upstream reads `CHAR` as UTF-8,
  and Latin-1 would read a byte above 0x7F that upstream replaces as a different
  character; the error keeps the byte.
- **Upstream's trimming.** Rejected: trailing spaces are text a device wrote, a round trip
  through Stonecrab would lose them, and the "all spaces" exception shows that the rule is
  a guess; a caller can trim.
- **One register per `CHAR`, as for `BYTE`.** Rejected: upstream counts one byte per
  `CHAR` both in the request and in the codec, so its layout is consistent there, and the
  odd-byte-offset rule exists for it.
- **Upstream's partial swap of odd-length text.** Rejected: it reads the last character of
  a byte-swapped `STRING(3)` from the wrong half of its register.

## Compliance
`crates/stonecrab/tests/modbus_text.rs` (new, features `modbus` and `testkit`) parses and
sizes text addresses with the transcribed upstream cases (`ModbusStringTest`,
`ModbusSelectionOffsetTest`, `ModbusEncodeTest`, `RegisterCodec_test.go`,
`Reader_test.go`, each naming its file), reads and writes all four types through
`ModbusTcpDriver` over the fake transport in every byte-order mode, a surrogate pair
included, refuses what does not fit before any I/O (a string wider than one write with
`ResourceLimit`), fails undecodable text with the bytes kept, and checks the canonical
text by a property. `crates/stonecrab/src/modbus/decode.rs` tests the run decoder and
encoder, with a round-trip property over the four types, the four modes, single values and
lists. `crates/stonecrab/tests/modbus_parse.rs`, `modbus_address.rs` and
`modbus_selection.rs` cover the grammar, the names and the odd-byte-offset rule, and
`crates/stonecrab/tests/modbus_interop_pymodbus.rs` reads and writes the text registers of
the pymodbus peer (`interop_reads_and_writes_text_registers`). `scripts/coverage-gate.sh`
holds modbus at 100% lines.

## Addendum (2026-10-29): the provenance entry of the text registers

`fixtures/modbus/interop/text-registers.toml` now has its own `[[file]]` entry in
`fixtures/modbus/PROVENANCE.toml` (first-party, Apache-2.0), and the testkit test that pins
the list of `[[file]]` entries (`golden_dir_lists_every_vector_in_provenance`) names it.
The exception recorded under Consequences is closed.
