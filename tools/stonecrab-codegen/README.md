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
`match_same_arms` has nothing to report.

Semantics, beyond ADR-0019 decision 4:

- A `discriminatedType` is an enum of its enabled cases. Its `typeSwitch` is one `match`
  over the discriminators in source order; a case with fewer values than discriminators
  matches on the prefix and stores the unmatched wire discriminators (`functionFlag` of
  `ModbusPDUError`). A disabled case keeps its arm and is refused with
  `ParseError::InvalidDiscriminator { field: "<Type>.<Case> (disabled)", .. }`; no case at
  all is `InvalidDiscriminator { field: "<Type>", .. }`. The `value` is the last discriminator
  that is not a `bit` (`functionFlag`), or the enum's wire value.
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
- A `byte` array is `reader.read_bytes(count, usize::MAX)`: the bytes present bound it, a
  short input is `Truncated` before anything is copied.
- An `enum` is a Rust enum with `value()` (when it has a base type), one `const fn` per
  constant parameter and, when an enabled type reads it from the wire, `parse` and
  `serialize`; an unknown wire value is `InvalidDiscriminator`. Enum values and constants are
  taken as written, not as expressions, as upstream does.
- A `dataIo` is a `parse` and a `serialize` function between bytes and `PlcValue`, one case
  per enum value, each case one byte-aligned number.
- Type parameters are arguments of `parse`; one that no enabled case reads is named `_name`.
- A nested type whose `lengthInBytes` an expression reads gets `length_in_bytes()`.
- A recursive type is a generation error.

## Options

`protocols/<protocol>.toml` names the output directory, the support module, the enabled
top-level types and the enabled cases of each enabled `discriminatedType` and `dataIo`.
Every other case is disabled: parsed, its discriminator values known, and refused by the
generated parser. A type that is not enabled is not generated, and an enabled case that refers
to one is a generation error. `generated/MANIFEST.toml` repeats the options next to the output.

The Modbus options enable the hand codec's domain of decision 8: `ModbusTcpADU`,
`ModbusPDUError`, the read requests and responses of function codes 01 to 04,
`ModbusErrorCode`, and the numeric `DataItem` cases except `BOOL`, with `DriverType` and
`ModbusDataType` as parameter types. The write and file record cases the hand codec gained in
S45 and S50 stay disabled (the ADR-0019 addendum of 2026-10-29).

## MSpec feature matrix

Parsed: the construct is read by the parser, which covers the whole of `MSpec.g4` and
`Expression.g4`. Emitted: the generator writes code for it; a construct that is parsed and
not emitted is a generation error naming the construct, the type and the mspec line when it
appears in an enabled type or case. Validated: the protocol whose vectors prove the emitted
code (for Modbus, the equivalence proof of `crates/stonecrab/src/modbus/generated_equivalence.rs`
over `fixtures/modbus/` and the fuzz seeds; ADR-0019 decision 8). A row is emitted only when it
is validated.

| Construct | Parsed | Emitted | Validated by |
| --- | --- | --- | --- |
| File: `[constants ...]` | yes | no: a reference to a constant is a generation error naming it | - |
| File: `[globals ...]` | yes | no: generation error | - |
| File: `[context ...]` | yes | no: generation error | - |
| Type form: `type` (a struct) | yes | no | - |
| Type form: `discriminatedType` | yes | yes, when the parent has only discriminators before its `typeSwitch` | Modbus (`ModbusADU`, `ModbusPDU`) |
| Type form: `enum` | yes | yes: base type `uint 8` to `uint 64` or none, constant parameters of unsigned types | Modbus (`DriverType`, `ModbusErrorCode`, `ModbusDataType`) |
| Type form: `dataIo` | yes | yes: one enum parameter switch, one byte-aligned number per case | Modbus (`DataItem`) |
| `batchSet` | yes | no | - |
| Field: `abstract` | yes | no | - |
| Field: `array` | yes | `byte` elements with a `count` loop only | Modbus (read responses) |
| Field: `assert` | yes | no | - |
| Field: `checksum` | yes | no | - |
| Field: `const` | yes | unsigned, byte-aligned, integer value | Modbus (`protocolIdentifier`) |
| Field: `discriminator` | yes | `bit` and `uint 1` to `uint 7` in one-byte runs, byte-aligned unsigned | Modbus (`errorFlag`, `functionFlag`) |
| Field: `enum` | yes | no | - |
| Field: `implicit` | yes | `uint 8` and `uint 16`, checked on parse | Modbus (`length`, `byteCount`) |
| Field: `manualArray` | yes | no | - |
| Field: `manual` | yes | no | - |
| Field: `optional` | yes | no | - |
| Field: `padding` | yes | no | - |
| Field: `peek` | yes | no | - |
| Field: `reserved` | yes | no | - |
| Field: `simple` | yes | byte-aligned numbers, an enum, a `discriminatedType` with name arguments | Modbus |
| Field: `state` | yes | no | - |
| Field: `typeSwitch` | yes | one or more discriminators (parameters or fields), prefix cases | Modbus |
| Field: `unknown` | yes | no | - |
| Field: `validation` | yes | no | - |
| Field: `virtual` | yes | no | - |
| Case: fewer values than discriminators | yes | yes, unmatched wire discriminators stored | Modbus (`ModbusPDUError`) |
| Case: `*` value, default case (no values), `*Name`, case parameters | yes | no | - |
| Data type: `bit` | yes | in a one-byte run and as a parameter; not alone (the `BOOL` case) | Modbus (`errorFlag`, `response`) |
| Data type: `byte` | yes | yes | Modbus |
| Data type: `int` | yes | 8, 16, 32 and 64 bits | Modbus (`DataItem`) |
| Data type: `uint` | yes | 1 to 7 bits in a run, 8, 16, 32 and 64 bits | Modbus |
| Data type: `float` | yes | 32 and 64 bits | Modbus (`DataItem` `REAL`, `LREAL`) |
| Data type: `vint`, `vuint`, `ufloat` | yes | no | - |
| Data type: `string`, `vstring` | yes | no | - |
| Data type: `time`, `date`, `dateTime` | yes | no | - |
| Attribute `byteOrder` | yes | `BIG_ENDIAN` (also the default) | Modbus |
| Attributes `unsignedIntegerEncoding`, `signedIntegerEncoding`, `floatEncoding`, `stringEncoding` | yes | their defaults only (`unsigned-binary`, `twos-complement`, `IEEE754`, `UTF8`) | Modbus |
| Field attributes | yes | no | - |
| Built-in `COUNT` | yes | of a `byte` array | Modbus (`byteCount`) |
| Built-in `ARRAY_SIZE_IN_BYTES` | yes | no | - |
| Built-in `STATIC_CALL` | yes | no: no protocol lists a support function yet | - |
| Built-ins `STR_LEN`, `CAST`, `CEIL` and other calls | yes | no | - |
| Property `lengthInBytes` | yes | of a nested `discriminatedType` | Modbus (`ModbusTcpADU` `length`) |
| Other properties (`lengthInBits`, field paths) | yes | no | - |
| Operator `+` | yes | `checked_add`, `Overflow` on overflow | Modbus (`pdu.lengthInBytes + 1`) |
| Operators `-`, `*`, `/`, `%`, `^`, `<<`, `>>`, comparisons, `==`, `!=`, `&&`, `\|\|`, `&`, `\|`, `!`, unary `-`, `?:`, indexes | yes | no | - |
| Literals: decimal and hex integers | yes | yes | Modbus |
| Literals: decimal fractions, strings, `true`/`false` in expressions, `null` | yes | `true`/`false` as case values only | Modbus (`errorFlag`, `response`) |
| Enum values without a wire value (upstream counts) | yes | no | - |
| Recursive types | yes | no: generation error | - |

Upstream's precedence differs from Rust's: `&&` binds tighter than `&` and `||` tighter than
`|` (Expression.g4 lines 38-59). The parser keeps it, and every operator the emitter will
write is emitted fully parenthesized as a checked call, never with Rust's precedence.

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
