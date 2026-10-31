# stonecrab-codegen

The MSpec code generator of [ADR-0019](../../docs/adr/0019-mspec-code-generator.md): a
native Rust binary that reads Apache PLC4X protocol descriptions (`.mspec`) and writes Rust
codecs over `stonecrab::codec`. It is first-party tooling, not published (`publish = false`),
and lives in its own Cargo workspace with its own `Cargo.lock` and `deny.toml`, like `fuzz/`
(ADR-0009); the root workspace excludes it. Its only dependencies are `toml` and `sha2`
(`docs/DEPENDENCY_AUDIT.md`). It builds with the root `rust-toolchain.toml` and formats its
output with that toolchain's `rustfmt`.

## Use

Run from the repository root:

```sh
mise run generate   # rewrite the generated code of every protocol in place
mise run regen      # the CI gate: regenerate and require no difference (ADR-0019 decision 9)
cargo run --locked --manifest-path tools/stonecrab-codegen/Cargo.toml -- \
  generate tools/stonecrab-codegen/protocols/modbus.toml [<output directory>]
cargo run --locked --manifest-path tools/stonecrab-codegen/Cargo.toml -- \
  check <read-only PLC4X checkout>
```

`generate` reads the protocol options (`protocols/<protocol>.toml`), finds the protocol's
vendored mspec files by `protocol` in `upstream/manifest.toml`, refuses to run when a copy's
sha256 differs from the manifest, parses and resolves the files, emits the enabled types and
replaces `generated.rs`, `generated/` and `generated/MANIFEST.toml` in the output directory
(the options' `output` unless one is given). `check` is maintainer-only (decision 10): it
parses every `*.mspec` file of a checkout, resolves the files of each `protocols/<name>`
module together, prints one line per file and emits nothing. It never runs in CI.

## What it writes

For Modbus, `crates/stonecrab/src/modbus/`:

- `generated.rs` declares one module per enabled top-level type, sorted by name;
- `generated/<type_in_snake_case>.rs` holds each type;
- `generated/MANIFEST.toml` records the generator and version, the upstream tag and commit,
  each mspec's upstream path, vendored path and sha256, and the options. It has no timestamp.

Every `.rs` file begins with the provenance header of decision 7. The header is added after
`rustfmt` runs, because `rustfmt` leaves alone any file whose first lines say `@generated`;
the files are formatted by construction, and `cargo fmt --check` skips them. Generated files
copy names from the mspec, never comment text. Names map to Rust conventions: types and
variants `UpperCamelCase` with each word's first letter upper case (`ModbusPDU` is
`ModbusPdu`, `ILLEGAL_FUNCTION` is `IllegalFunction`), fields, parameters and modules
`snake_case`; a keyword, a collision or a name the output uses itself is a generation error.
Variants keep the mspec's case names (`ModbusPdu::ModbusPduReadCoilsRequest`).

The output reads and writes only through `BoundedReader` and `BoundedWriter`, uses checked
arithmetic and `try_from`, and has no `as`, no indexing and no panic. It is compiled under the
lints of the module it lives in (the `modbus` promotions of `indexing_slicing`,
`arithmetic_side_effects` and `as_conversions` included). The suppressions it needs are
item-level `#[expect]`s with reasons: `redundant_pub_crate` on every item (the generated
module is private, so `pub` would trip `unreachable_pub`) and `enum_variant_names` where
clippy's rule fires on the mspec's names. Arms with the same body are merged so
`match_same_arms` has nothing to report, but only when they bind the same names of the same
types (`Reply::A { results: Vec<X> }` and `Reply::B { results: Vec<Y> }` have the same
body and stay apart).

Semantics, beyond ADR-0019 decision 4:

- A `discriminatedType` is an enum of its enabled cases. Its `typeSwitch` is one `match`
  over the discriminators in source order; a case with fewer values than discriminators
  matches on the prefix and stores the unmatched wire discriminators (`functionFlag` of
  `ModbusPDUError`). A disabled case keeps its arm and is refused with
  `ParseError::InvalidDiscriminator { field: "<Type>.<Case> (disabled)", .. }`; no case at
  all is `InvalidDiscriminator { field: "<Type>", .. }`. The `value` is the last discriminator
  that is not a `bit` (`functionFlag`), or the enum's wire value.
- A switch may be over a signed number of 8, 16, 32 or 64 bits, a discriminator field or a
  parameter (`ExtensionObjectDefinition(int 32 extensionId)`). A case value is an integer
  literal that fits the width, negative ones included (`'-310'`). The `value` of an
  `InvalidDiscriminator` keeps the sign: it is the two's complement of the number widened to
  64 bits (`i64::from(id).cast_unsigned()`), so -310 is not reported as 310, the number of a
  real case.
- A `discriminatedType` of more than 16 enabled cases gets a function for each case that reads
  or writes something, `parse_<case>` and `serialize_<case>`, and the `match` only calls it
  with what the case takes from the parent: the budget, and the parameters and the fields read
  before the switch that the case uses or stores. A `serialize_<case>` that stores fields takes
  `&self` and refuses any other variant, which the `match` never sends. Without it the locals
  of every arm share the one stack frame of the dispatcher, which in a debug build grows with
  the number of cases (ADR-0019, S80 addendum, finding 5). A switch of 16 cases or fewer, such
  as the nine of Modbus, stays in one `match` and its output is unchanged. The functions that
  result may still be longer than `too-many-lines-threshold` of the repository's
  `clippy.toml` (a dispatcher refuses every disabled case in its own arm); the generator does
  not add a lint expectation for it, the module that holds the output does.
- Sub-byte discriminators and fields form runs of exactly one byte, split and joined most
  significant bit first through `split_bits` and `join_bits` of the hand-written support
  module (`crate::modbus::generated_support`); a value that does not fit its bits is
  `Overflow`, never truncated.
- A `const` field is checked on parse (`InvalidDiscriminator` naming the field). An
  `implicit` field is read, checked on parse against its expression once the case is read
  (`InvalidDiscriminator` naming the field: upstream reads and drops it), not stored, and
  computed on serialize with `try_from`, so a count that does not fit fails with `Overflow`.
  When the field is the `count` of a `byte` array read with it (Modbus `byteCount`), the
  parse check cannot fail: `read_bytes` returns exactly that many bytes or `Truncated`. The
  check stays, as the one rule for every `implicit` field; generated code is outside the
  coverage gate, so the unreachable branch costs nothing there.
- A `byte` array with an unsigned count is `reader.read_bytes(count, usize::MAX)`: the bytes
  present bound it, a short input is `Truncated` before anything is copied.
- Every other `count` array is checked against the input before anything is allocated. An
  element of a fixed width (a byte-aligned number, an enum with a base type) needs
  `count * width` bytes; the product is checked and compared with `reader.remaining()`, and
  only then is the vector reserved. An element of no fixed width (a `type` or
  `discriminatedType`) needs at least one byte, so the count is compared with the remaining
  bytes and the vector grows one parsed element at a time. Either failure is
  `ParseError::CountLimit { field, count, max }`, with the mspec field name, the declared count
  and the most elements the input could hold (`CountLimit` was added to the shipped
  `ParseError` by S86; the fixtures' stand-in has it too).
- The count is any expression of the subset. Its value is a `usize` when it is made of
  `uint` fields, parameters and literals, and an `i64` when an `int 32` takes part (an
  `int 32` field converts with `i64::from`). An `i64` count of -1 means null only for a
  nullable array (below); any other negative count is
  `InvalidDiscriminator { field: "<array>", value: <magnitude> }`, and `-1` for a
  non-nullable array is the same refusal unless the expression maps it to a count itself
  (`stringLength == -1 ? 0 : stringLength`).
- An array is nullable when an `implicit` field of the same field list writes it as null
  (its expression contains `name == null`), or when its count is the bare name of a
  `simple int 32` field of the list (`[simple int 32 arrayLength] [array uint 8 value count
  'arrayLength']`, the `ByteStringArray` of OPC UA). A nullable array is `Option<Vec<T>>`
  (`None` is count -1, `Some(vec![])` is count 0, and the two are distinct on the wire and as
  values), its count must be signed, and `COUNT(name)` of a `None` is 0. Where the count is a
  stored field, `serialize` refuses a value that disagrees with it (`None` with a count other
  than -1, `Some` with a count other than its length) as
  `InvalidDiscriminator { field: "<array>", value: <magnitude of the length or -1> }`. Every
  other array, including one whose count is an expression such as
  `length == -1 ? 0 : length`, is `Vec<T>`.
- An `implicit` field is `uint 8`, `uint 16`, `uint 32` or `int 32`. It is compared with its
  expression on parse and computed on serialize with `try_from` as before; when the field is
  an `int 32` or `uint 32`, or the expression is signed, both sides are compared as `i64`,
  and the `value` of the error is the field's magnitude.
- The expression subset of the output is: integer literals (up to `i64::MAX`; a larger one
  is a generation error), names of fields and parameters
  of the types above, `COUNT(array)`, `name.lengthInBytes` of a nested type, `+` (checked),
  `==` of two numbers or of a nullable array and `null` (`name.is_none()`), a prefix `-` and
  `condition ? a : b` (whose two arms are the same kind of number). Every other operator is a
  generation error naming it.
- An `enum` is a Rust enum with `value()` (when it has a base type), one `const fn` per
  constant parameter and, when an enabled type reads it from the wire, `parse` and
  `serialize`; an unknown wire value is `InvalidDiscriminator`. Enum values and constants are
  taken as written, not as expressions, as upstream does.
- A `dataIo` is a `parse` and a `serialize` function between bytes and `PlcValue`, one case
  per enum value, each case one byte-aligned number.
- A `type` is a struct with a `pub(crate)` field for each stored field, in source order
  (`const` and `implicit` fields are not stored), and `parse`, `serialize` and, when an
  expression reads its `lengthInBytes`, `length_in_bytes`. Its fields are the ones a case of a
  `discriminatedType` takes: runs of sub-byte fields, byte-aligned numbers, an enum, an
  array with a `count`, and a nested `type` or `discriminatedType` with name or literal arguments. A
  fixed length is a `const fn` that keeps the `&self` and `Result` of every other length. The
  struct derives `Eq` unless a field is a float. A `type` that stores no field, or has a
  `discriminator` field, is a generation error.
- Every multi-byte read and write uses the protocol's byte order: `BIG_ENDIAN` unless the
  options file says `byte_order = "LITTLE_ENDIAN"`. A `byteOrder` attribute in the mspec must
  agree with it; one that differs is a generation error, because a per-type byte order is not
  emitted.
- A type name defined twice is a generation error when the output would use the name (an
  enabled type, or one an enabled type refers to), naming both lines. The generator will not
  guess which of two definitions upstream would use; a duplicate nothing refers to
  changes no output and is not reported.
- Type parameters are arguments of `parse`; one that no enabled case reads is named `_name`.
  An argument at a use is a name of the caller's scope or a literal: an integer that fits
  the parameter (`Block('2')`, `-1` for a signed one) or `true` and `false` for a `bit`;
  the callee's parameter list and these are the only checks, so `'300'` for a `uint 8` is a
  generation error. An enum parameter takes names only.
- A case that a field of an enabled type names as a type (`[simple RequestHeader('391')
  requestHeader]`, where `RequestHeader` is a case of `ExtensionObjectDefinition(int 32
  extensionId)`, as upstream's `RequestHeader.staticParse(readBuffer, 391)` is) is generated
  as a struct of its own, in a module named for the case, whether or not the options enable the
  case as a variant of its parent. Its `parse` takes the parent's parameters, which the
  reference passes: for each parameter the parent's `typeSwitch` switches on, the reference
  must pass the literal that is the case's value (`391`), otherwise the reference names
  another type than the case; the other parameters are names or literals as above. It is a
  generation error when the parent reads a field before its `typeSwitch`, when the case has
  parameters of its own, or when two types give a case the same name.
- A case parameter (`['500' Tail (uint 8 scale) ...]`) names a value the parent has already
  read or was given, of the same type: a parameter of the parent, or a field read before
  the `typeSwitch`. The case's fields use it by that name. It is a generation error when no
  such value exists. A `dataIo` case takes no parameters.
- A `reserved` field is read and ignored, whatever it holds, and written as its value,
  which is an integer or boolean literal that fits the field (`[reserved uint 7 '0x00']`,
  `[reserved bit 'false']`, `[reserved int 5 '0x00']`). Below a byte it takes part in a
  one-byte run with flags (`[reserved uint 7 '0x00'] [simple bit isOnline]`, most significant
  bit first); from a byte up it is an unsigned number of 8, 16, 32 or 64 bits. It is not
  stored. Upstream's generated readers read such a field and drop it (`readReservedField`,
  `ViewAttributes.java:138`); the runtime logs a value other than the expected one at info
  level and returns it (`FieldReaderReserved.java:33-42`), and the generated writer writes the
  expected value (`ViewAttributes.java:171`, `FieldWriterReserved.java:33-42`). The generator
  accepts any value and writes the expected one, the same.
- A recursive type is a generation error, with one exception the generator cannot see: a
  cycle through an external type (below), which that type bounds with the budget it is handed.

## Options

`protocols/<protocol>.toml` names the output directory, the support module, the byte order
(`byte_order`, `"BIG_ENDIAN"` or `"LITTLE_ENDIAN"`; `BIG_ENDIAN` when absent), the enabled
top-level types and the enabled cases of each enabled `discriminatedType` and `dataIo`.
Every other case is disabled: parsed, its discriminator values known, and refused by the
generated parser. A type that is not enabled is not generated, and an enabled case that refers
to one is a generation error. `generated/MANIFEST.toml` repeats the options next to the output;
`byte_order` appears there when the options file sets it. The Modbus options do not, so its
output is the `BIG_ENDIAN` one it always was.

The Modbus options enable the hand codec's domain of decision 8: `ModbusTcpADU`,
`ModbusPDUError`, the read requests and responses of function codes 01 to 04,
`ModbusErrorCode`, and the numeric `DataItem` cases except `BOOL`, with `DriverType` and
`ModbusDataType` as parameter types. The write and file record cases the hand codec gained in
S45 and S50 stay disabled (the ADR-0019 addendum of 2026-10-29).

### External types and the recursion budget

`depth = "<Rust path>"` names a small `Copy` type, the recursion budget (ADR-0034 decision 3),
and `[external]` maps an mspec type name to a hand-written Rust type that stands in for it.
With `depth` set, every generated `parse`, `serialize` and `length_in_bytes` of a struct or
a `discriminatedType` takes it as its first argument after the reader, the writer or the
receiver (`parse(reader, depth, params...)`, `serialize(&self, writer, depth)`,
`length_in_bytes(&self, depth)`), passes it on unchanged to every nested generated or
external value, and names it `_depth` when the body never does. Enums and `dataIo` do not
take it, and a protocol without `depth` is generated exactly as before. The generator never
spends the budget: a cycle of generated types alone is still a generation error, so every
cycle passes through an external type, and the external type spends it.

The external type has one fixed shape, whatever the mspec type is, which a hand-written type
over `BoundedReader` and `BoundedWriter` implements as inherent functions:

```rust
pub(crate) fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError>
pub(crate) fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError>
pub(crate) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError>
```

It also implements `Debug`, `Clone` and `PartialEq`, which the generated structs derive; with
any external in the table nothing derives `Eq`, because a Variant holds a float and the
derive of a type that holds one would not compile. A field of an external type is stored as
that type, an array of them as `Vec<T>` or `Option<Vec<T>>`, and `x.lengthInBytes` of one is
`x.length_in_bytes(depth)`. The mspec type's parameters are not passed: an entry that
stands for a type with parameters pins them (`ExtensionObject = { path = "...", arguments =
["true"] }`, one literal per parameter), and a use with other arguments is a generation
error. The entry is a path, or a table with `path` and `arguments`:

```toml
depth = "crate::opcua::codec::Depth"

[external]
PascalString = "crate::opcua::codec::OpcString"
ExtensionObject = { path = "crate::opcua::codec::ExtensionObject", arguments = ["true"] }
```

The mspec type must be defined by the mspec files (and once), be a `type` or a
`discriminatedType`, and not be in `types`; the table needs `depth`; a path is two or more
identifiers joined by `::`. A reference to a type that is neither generated nor listed is the
generation error `refers to type ..., which the options do not enable and do not list under
[external]`. The module imports each external with `use <path>;`, or `use <path> as <Name>;`
when the last segment is not the Rust name of the mspec type. `MANIFEST.toml` records `depth`
and the table (`[options.external]`) when the options have them, so the hand-written
dependency set is part of what a regeneration reviews; the Modbus manifest says nothing.

## MSpec feature matrix

Parsed: the construct is read by the parser, which covers the whole of `MSpec.g4` and
`Expression.g4`. Emitted: the generator writes code for it; a construct that is parsed and
not emitted is a generation error naming the construct, the type and the mspec line when it
appears in an enabled type or case. Validated: the protocol whose vectors prove the emitted
code (for Modbus, the equivalence proof of `crates/stonecrab/src/modbus/generated_equivalence.rs`
over `fixtures/modbus/` and the fuzz seeds; ADR-0019 decision 8). A row is emitted only when it
is validated. A construct that no protocol in the tree uses yet is validated by synthetic
first-party fixtures that the tool's own tests compile and run (`src/emit/fixtures.rs`, "S77
fixtures" below); the first protocol that uses it adds its own vectors (ADR-0019 addendum of
2026-10-30).

| Construct | Parsed | Emitted | Validated by |
| --- | --- | --- | --- |
| File: `[constants ...]` | yes | no: a reference to a constant is a generation error naming it | - |
| File: `[globals ...]` | yes | no: generation error | - |
| File: `[context ...]` | yes | no: generation error | - |
| Type form: `type` (a struct) | yes | yes: the fields of a case, stored fields `pub(crate)`, `parse`, `serialize` and `length_in_bytes` | S77 fixtures |
| Type form: `discriminatedType` | yes | yes, when the parent has only discriminators before its `typeSwitch`; a function per case above 16 enabled cases | Modbus (`ModbusADU`, `ModbusPDU`); S80a fixtures (the functions) |
| Type form: `enum` | yes | yes: base type `uint 8` to `uint 64` or none, constant parameters of unsigned types | Modbus (`DriverType`, `ModbusErrorCode`, `ModbusDataType`) |
| Type form: `dataIo` | yes | yes: one enum parameter switch, one byte-aligned number per case | Modbus (`DataItem`) |
| `batchSet` | yes | no: one with attributes is a generation error (its fields would be flattened and the attributes lost) | S77 fixtures |
| Field: `abstract` | yes | no | - |
| Field: `array` | yes | `count` loops of `byte`, byte-aligned numbers, enums and `type` or `discriminatedType` elements (name arguments); `Vec<T>`, or `Option<Vec<T>>` when an `implicit` writes it as null; not `bit`, `length` or `terminated` loops | Modbus (`byte` read responses); S78 fixtures |
| Field: `assert` | yes | no | - |
| Field: `checksum` | yes | no | - |
| Field: `const` | yes | unsigned, byte-aligned, integer value | Modbus (`protocolIdentifier`) |
| Field: `discriminator` | yes | `bit` and `uint 1` to `uint 7` in one-byte runs, byte-aligned unsigned | Modbus (`errorFlag`, `functionFlag`) |
| Field: `enum` | yes | no | - |
| Field: `implicit` | yes | `uint 8`, `uint 16`, `uint 32` and `int 32`, checked on parse | Modbus (`uint 8`, `uint 16`: `length`, `byteCount`); S78 fixtures (`uint 32`, `int 32`) |
| Field: `manualArray` | yes | no | - |
| Field: `manual` | yes | no | - |
| Field: `optional` | yes | no | - |
| Field: `padding` | yes | no | - |
| Field: `peek` | yes | no | - |
| Field: `reserved` | yes | integer or boolean literal; below a byte in a one-byte run with flags, from a byte up an unsigned 8 to 64 bits; read and ignored, written as its value, not stored | S79 fixtures |
| Field: `simple` | yes | byte-aligned numbers, an enum, a `discriminatedType` or `type` with name or literal arguments | Modbus; S77 fixtures (a `type`) |
| Field: `state` | yes | no | - |
| Field: `typeSwitch` | yes | one or more discriminators (parameters or fields), prefix cases; unsigned or signed (`int 8` to `int 64`) numbers, enums and bits | Modbus; S80a fixtures (signed) |
| Field: `unknown` | yes | no | - |
| Field: `validation` | yes | no | - |
| Field: `virtual` | yes | no | - |
| Case: fewer values than discriminators | yes | yes, unmatched wire discriminators stored | Modbus (`ModbusPDUError`) |
| Case: `*` value, default case (no values), `*Name` | yes | no | - |
| Case parameters | yes | in a `discriminatedType`: each names a value of its type that the parent has read or was given; not in a `dataIo` | S79 fixtures |
| Case named as a type: `[simple Case('391') f]`, also as an array element | yes | a struct of its own, `parse` takes the parent's parameters; the discriminator literal is checked; the parent must read nothing before its switch | S79 fixtures |
| Literal type arguments: integers (`-1` included) and `true`/`false` | yes | for integer and `bit` parameters | S79 fixtures |
| Options: `depth` and `[external]` | n/a | the budget argument and the fixed external signature (above); pinned literal arguments | S79 fixtures |
| Data type: `bit` | yes | in a one-byte run and as a parameter; not alone (the `BOOL` case) | Modbus (`errorFlag`, `response`) |
| Data type: `byte` | yes | yes | Modbus |
| Data type: `int` | yes | 8, 16, 32 and 64 bits | Modbus (`DataItem`) |
| Data type: `uint` | yes | 1 to 7 bits in a run, 8, 16, 32 and 64 bits | Modbus |
| Data type: `float` | yes | 32 and 64 bits | Modbus (`DataItem` `REAL`, `LREAL`) |
| Data type: `vint`, `vuint`, `ufloat` | yes | no | - |
| Data type: `string`, `vstring` | yes | no | - |
| Data type: `time`, `date`, `dateTime` | yes | no | - |
| Attribute `byteOrder` | yes | the protocol's byte order (`BIG_ENDIAN` unless its options say `byte_order = "LITTLE_ENDIAN"`); a different value on a type is a generation error | Modbus (`BIG_ENDIAN`); S77 fixtures (`LITTLE_ENDIAN`) |
| Attributes `unsignedIntegerEncoding`, `signedIntegerEncoding`, `floatEncoding`, `stringEncoding` | yes | their defaults only (`unsigned-binary`, `twos-complement`, `IEEE754`, `UTF8`) | Modbus |
| Field attributes | yes | no | - |
| Built-in `COUNT` | yes | of an array (a nullable one counts 0 when null) | Modbus (`byteCount`, a `byte` array); S78 fixtures |
| Built-in `ARRAY_SIZE_IN_BYTES` | yes | no | - |
| Built-in `STATIC_CALL` | yes | no: no protocol lists a support function yet | - |
| Built-ins `STR_LEN`, `CAST`, `CEIL` and other calls | yes | no | - |
| Property `lengthInBytes` | yes | of a nested `discriminatedType` | Modbus (`ModbusTcpADU` `length`) |
| Other properties (`lengthInBits`, field paths) | yes | no | - |
| Operator `+` | yes | `checked_add`, `Overflow` on overflow | Modbus (`pdu.lengthInBytes + 1`) |
| Operator `==` | yes | of two numbers, and `name == null` of a nullable array | S78 fixtures |
| Operator prefix `-` | yes | of an integer literal (`-1`) or a number, checked | S78 fixtures |
| Operator `?:` | yes | `condition ? a : b` with a comparison for the condition and two numbers | S78 fixtures |
| Operators `-`, `*`, `/`, `%`, `^`, `<<`, `>>`, `<`, `<=`, `>`, `>=`, `!=`, `&&`, `\|\|`, `&`, `\|`, `!`, indexes | yes | no | - |
| Literals: decimal and hex integers | yes | yes | Modbus |
| Literals: decimal fractions, strings, `true`/`false` in expressions | yes | `true`/`false` as case values only | Modbus (`errorFlag`, `response`) |
| Literal `null` | yes | only as `name == null` of a nullable array | S78 fixtures |
| Enum values without a wire value (upstream counts) | yes | no | - |
| Recursive types | yes | no: generation error, except through an external type, which the generator cannot see | S79 fixtures |
| Duplicate type names | yes | no: generation error when the output uses the name | S77 fixtures |

Upstream's precedence differs from Rust's: `&&` binds tighter than `&` and `||` tighter than
`|` (Expression.g4 lines 38-59). The parser keeps it, and every operator the emitter will
write is emitted fully parenthesized as a checked call, never with Rust's precedence.

## S77 fixtures

`src/emit/fixtures.rs` holds first-party mspec text (no OPC UA mspec is vendored yet) that
exercises `type` structs, the protocol byte order and duplicate type names. Its round trips
write the generated modules next to a small stand-in for `stonecrab::codec` and
`ByteOrder`, compile them with `rustc` and run the program: a struct with a sub-byte run, an
enum, a nested struct, two `implicit` fields, two byte arrays, a `const` and a parameter that
it passes on, a `discriminatedType` with a struct in a case, parsed from bytes, checked field
by field, serialized back to the same bytes and refused on a wrong `implicit`, a wrong
`const`, an unknown enum value and a short input; once under the default byte order and once
under `LITTLE_ENDIAN`. The stand-in has the signatures the output calls and is not the shipped
codec. Generated code meets the shipped codec, and the crate's lints, in the crate: the S77
output was also compiled and linted there once, in a scratch copy of the tree.

## S78 fixtures

`src/emit/array_fixtures.rs` holds first-party mspec text for the arrays OPC UA is made of:
a nullable array of numbers, of enums, of structs (with a parameter passed on) and of bytes,
counted by an `implicit` `int 32` or `uint 32`; an array counted by a literal, a parameter and
a plain field (a plain `int 32` field makes it nullable); a count that maps -1 to empty; an `implicit` that disagrees with its array; a
struct whose `lengthInBytes` adds up arrays; and a nullable array in a case of a
`discriminatedType`. It is compiled with `rustc` next to the same stand-in as the S77
fixtures, little-endian, and the programs check:

- `generator_complex_array_count_is_checked_before_allocation`: the wire layout and the
  refusals. A count of `i32::MAX` or `u32::MAX` over a few bytes, and a count one element above
  the bound, are `CountLimit` with the field, the count and the bound, for numbers, enums and
  structs alike; the generated text reserves capacity only on the line after its check; a
  count of -2 and `i32::MIN` are `InvalidDiscriminator`.
- `generator_null_array_is_distinct_from_an_empty_array`: a property loop (a fixed-seed
  xorshift; the tool has no dependency for a property-testing crate, and the program is built
  with `rustc` alone) over 600 random tables: serialize then parse is the identity; the same
  table with its arrays null or empty has different bytes (-1 against 0) and parses back to
  each; and 7200 mutated, truncated and random byte strings never panic, and whatever parses
  serializes back to exactly the bytes it was read from.
- `generator_implicit_int32_count_round_trips`: the little-endian bytes of every count width,
  an `implicit` `int 32` that disagrees with the array it counts (the error value is the
  magnitude), -1 as empty, a literal count and the lengths of a struct made of arrays.
- The same test covers a count that is a plain `simple int 32` field: -1 is `None`, 0 is
  `Some(vec![])`, -2 is refused, and `serialize` refuses a stored count that disagrees with
  the value.
- `generator_array_gaps_are_refused_with_the_construct_the_type_and_the_line`,
  `generator_array_gaps_name_the_line_of_the_field` (a count error names the field's line,
  not the type's), `generator_array_of_its_own_type_is_refused_as_recursive`,
  `generator_integer_literals_outside_i64_are_refused` and
  `generator_float_arrays_leave_out_eq`.

The S78 output was also compiled and linted under this workspace's lint table (which is the
root's) in a scratch crate: no finding. Generated code meets the shipped codec in the crate
when the first protocol that uses arrays adds its own vectors (S88 for OPC UA).

## S79 fixtures

Three files hold first-party mspec text for the rest of what the OPC UA service layer needs.
Each compiles the generated modules with `rustc` next to the same stand-in (which now has
`ParseError::RecursionLimit` as S86 added it) and runs a program:

- `src/emit/case_fixtures.rs`,
  `generator_case_parameters_receive_literal_arguments`: a `Body(uint 32 kind, uint 8 scale)`
  whose cases refer to a case `Header('391', 'scale')` and `Header('391', '1')`, as a single
  field and as a `count` array, to a `Block('2')` with a literal for its parameter, and
  declare a case parameter. `Header` is never enabled as a variant and is a module of its
  own. The program parses and serializes every case, passes the case's parameters, and checks
  the disabled-variant and unknown-kind refusals. The refusals are
  `generator_case_named_as_a_type_must_pass_its_own_discriminator` (another literal, a name
  where the literal is, a wrong argument count, a literal that does not fit or is not of
  the type, a name that does not exist; the error names the field's line),
  `generator_case_parameters_and_named_cases_are_checked_against_what_they_can_hold` and
  `generator_a_case_named_as_a_type_cannot_reach_itself`.
- `src/emit/reserved_fixtures.rs`, `generator_reserved_bits_share_a_byte_with_a_flag`:
  reserved bits at the top, in the middle and at the bottom of a run, a whole reserved
  `uint 16`, a byte of nothing but two reserved nibbles, and a case of a `discriminatedType`.
  Noisy reserved bits parse, are not stored and serialize as their value;
  `generator_reserved_fields_are_refused_when_they_cannot_be_written` covers a value that does
  not fit, one that does not suit the type, a float, a signed number of a byte or more, a run
  that does not end on a byte boundary or is longer than one, and a struct of nothing else.
- `src/emit/external_fixtures.rs`,
  `generator_external_type_is_called_through_its_signature`: three externals (a `Name`, a
  recursive `Variant` that spends the budget on every entry, and an `Envelope` pinned to
  `'true'`) in a struct as a field, as an array element (nullable) and through `lengthInBytes`,
  in a `discriminatedType`, and in a struct that reads that one's length. The externals check
  that the generated code hands them the budget unspent; five nested Variants fit a budget of
  five and six fail, on `parse` and on `serialize`, with the external's own error.
  `generator_missing_external_type_is_an_error` covers a type that is neither generated nor
  listed, an entry that the mspec does not define, a missing `depth`, a bad path, a wrong
  number of pinned arguments, a pinned argument that does not suit its parameter, a use with
  other arguments, a type that is both generated and external, and an enum. `src/main.rs`
  tests the options file and the manifest.

The output of the three programs was compiled and linted under the lint table of this
workspace with the root's module promotions (`indexing_slicing`, `arithmetic_side_effects`,
`as_conversions`) in a scratch crate: no finding in the generated code.

## S80a fixtures

`src/emit/switch_fixtures.rs` holds first-party mspec text for the three changes that the dry
run on the OPC UA model asked for (ADR-0019, S80 and S80a addenda). Each compiles the
generated modules with `rustc` (opt-level 0, a debug build) next to the same stand-in, which
now reads and writes `int 8`, `int 16` and `int 64`:

- `generator_signed_int32_discriminator_selects_its_case`: four switches, one of each width,
  two over a discriminator field and two over a parameter, each with a positive, a negative,
  a disabled negative and an empty case. Every case parses and serializes back to the same
  bytes; an unknown or disabled negative number is `InvalidDiscriminator` with its sign
  extended to 64 bits, `i16::MIN`, `i32::MIN` and `i64::MIN` included.
  `generator_signed_discriminator_literals_are_checked_against_the_width` covers `127` and
  `-128` for an `int 8` and refuses `128`, `-129`, `0xFF`, a fraction and a boolean.
- `generator_arms_with_equal_names_and_different_types_are_not_merged`: `FromAlpha` and
  `FromBeta` hold `results` of two element types with the same text in `serialize` and in
  `length_in_bytes`, and `FromAlphaToo` repeats the first type. The first and the third
  share an arm, the second does not, and the program compiles and round-trips all three.
- `generator_large_switch_gets_a_function_per_case` pins the limit (17 enabled cases split,
  16 do not) and the signatures of the case functions.
  `generator_dispatch_frame_does_not_grow_with_case_count` nests 300 cases through an
  external type to a budget of 40 on a thread of 1 MiB: the 39 levels that fit are parsed and
  serialized, 41 are `RecursionLimit`. With the cases inline the same program overflows the
  stack; with a function per case it runs on 128 KiB and fails on 64 KiB.

## The parser against the pinned baseline

`check` over the read-only checkout of Apache PLC4X v1.0.0 (commit `ea229918`) on 2026-10-29:
57 of its 59 mspec files parse and resolve, the five OPC UA files included. Two do not, and
both break `MSpec.g4`; ANTLR's error recovery lets upstream's build go on:

- `protocols/ads/src/main/resources/protocols/ads/ads.mspec` line 1088: a `validation` whose
  description is in ticks where the grammar takes a `"string"`;
- `protocols/eip/src/main/resources/protocols/eip/eip.mspec` line 310: a stray `]` after the
  last type.

## Changing the generator

A generator change that alters the output bumps the package version (the header names it)
and is followed by a regeneration-only `gen` commit. Grammar changes between baselines are
ported to `src/mspec.rs` and `src/expression.rs` with their own tests first (ADR-0019
decision 10). The Java bootstrap of CODING_STANDARDS 17.6 is not used.
