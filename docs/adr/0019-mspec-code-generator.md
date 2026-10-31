# 0019: MSpec code generator
## Status
Accepted (2026-10-28, under decision E3; the maintainer may revisit).
Written as slice S65 of docs/POST_ALPHA_PLAN.md (Phase G). It decides how Stonecrab turns
upstream MSpec protocol descriptions into Rust, where the output lives, what provenance it
carries, how S66 proves the generator on Modbus and what the S67 `regen` gate runs. It
builds on ADR-0013 (the bounded codec primitives the output must use), ADR-0015 decision 2
(D6: the hand-written Modbus PDUs and the corpus a generator must meet), ADR-0009 (the
fuzz workspace layout reused below) and ADR-0018 (one published crate). It amends
CODING_STANDARDS 1.2, 17.1, 17.2, 17.6, the `gen` commit type of 18.2 and the `regen` row
of section 19 as stated in decision 9. Nothing is generated, vendored or built by this slice.

## Context
`docs/SPEC.md` ("Code generation and upstream maintenance") asks Stonecrab to reuse the
PLC4X separation of protocol descriptions, language templates and generated code, to
start with the MSpec constructs Modbus needs, and sets eight requirements: provenance and
options recorded with the output, explicit failure on unsupported constructs,
deterministic output checked by CI with a clean diff, no hand patches of output,
independent vectors per construct, bounded recursion and allocation, retained test
provenance, and an explicit MSpec feature matrix. Its porting rules say ordinary builds
never need Java, a maintainer-only step "may initially use upstream Java/Maven tooling",
and "a native Rust generation pipeline belongs in the eventual port". CODING_STANDARDS
section 17 already names the tool (`tools/stonecrab-codegen`), the location
(`src/generated/` in the owning driver module) and a Java bootstrap that CI never runs on
pull requests (17.6). Decision E3 moved the generator before OPC UA (M3), proven first on
Modbus against the hand codec.

What upstream has at the pinned baseline (Apache PLC4X v1.0.0, commit
`ea229918f5d07700e04d7d69ab1645da679cb700`; paths are relative to the PLC4X root):

- **The MSpec grammar and model.** Below, `mspec/` abbreviates
  `code-generation/protocol-base-mspec/src/main/antlr4/org/apache/plc4x/plugins/codegenerator/language/mspec/`
  for the grammars and the same path under `src/main/java` for the Java sources. The
  ANTLR grammar `mspec/MSpec.g4` (435 lines) has a file level of optional
  `constants`, `globals` and `context` blocks (lines 21-35) followed by four type forms,
  `type`, `discriminatedType`, `enum` and `dataIo` (lines 41-46), twenty field kinds (lines 61-82)
  and thirteen base data types (lines 177-191). Field expressions have their own grammar,
  `mspec/expression/Expression.g4` (127 lines), whose precedence differs from Rust's: `&&`
  binds tighter than `&` and `||` tighter than `|` (lines 38-59). `MessageFormatListener`
  builds `Default*` model objects (`mspec/model/`) into a `HashMap`
  (`mspec/parser/MessageFormatListener.java` lines 61-74), and `MessageFormatParser` only
  logs a warning for type references that never resolve
  (`mspec/parser/MessageFormatParser.java` lines 78-80), and
  `ValidatableTypeContext.validate()` then throws "Unresolved types left"
  (`protocol-base-mspec/.../protocol/ValidatableTypeContext.java` lines 31-35), which
  `ModbusProtocol` and `OpcuaProtocol` both call. The model interfaces and the Maven
  plugin that drives generation are not in this source tree: they are
  `org.apache.plc4x.plugins` artifacts (`code-generation/protocol-base-mspec/pom.xml`
  lines 79-82; `code-generation/language/go/pom.xml` lines 71-73) at the version property
  `plc4x-code-generation.version`, which no `pom.xml` of the sparse checkout read for this
  ADR defines; its root `pom.xml` is not part of that checkout.
- **Languages.** Five language modules, `c`, `cs`, `go`, `java` and `python`, each behind
  its own Maven profile (`code-generation/language/pom.xml` lines 38-72); there is no Rust
  module. Four use FreeMarker through `FreemarkerLanguageOutput`, which renders one
  template per type and takes the output file name from the first rendered line
  (`code-generation/language-base-freemarker/src/main/java/org/apache/plc4x/plugins/codegenerator/protocol/freemarker/FreemarkerLanguageOutput.java`
  lines 113-151 and 172-210). Java instead emits code with JavaPoet
  (`code-generation/language/java/pom.xml` lines 141-145, and the import on line 22 of
  `code-generation/language/java/src/main/java/org/apache/plc4x/codegeneration/language/java/generators/BaseGenerator.java`).
  A language module is sizeable: the Go module is 5,087 lines (a 2,029-line
  `GoLanguageTemplateHelper.java` and a 1,861-line `complex-type-template.go.ftlh`),
  Python 4,572, Java 4,118, C 3,682 and C# 2,674 (`code-generation/language/*/src/main`,
  counts excluding each module's 19-line `META-INF/services` registration file).
- **Generation is a maintainer step, and the output is committed.** The language
  modules build only under the `update-generated-code` profile
  (`code-generation/pom.xml` lines 44-50), the Go drivers regenerate under the same profile
  (`plc4go/pom.xml` lines 582-605), and the generated Java for Modbus is committed under
  `plc4j/drivers/modbus/src/main/generated/` (57 files) with the marker "Code generated by
  code-generation. DO NOT EDIT." (for example
  `.../readwrite/ModbusPDUReadHoldingRegistersRequest.java` lines 30-32).
- **A construct catalog without vectors.** `code-generation/protocol-test` holds a
  569-line `test.mspec` that exercises the field kinds, and its comments record per-language
  gaps: a checksum field "Disabled for PLC4C" (lines 41-42) and virtual fields in
  discriminated types failing for Go (lines 230-244). The module contains only that file,
  `TestProtocol.java` and site documentation; no wire vectors.
- **Modbus.** `protocols/modbus/src/main/resources/protocols/modbus/modbus.mspec` (439
  lines, sha256 `cb8ea683da2f3ddc207ee2b4469f1737a6a34ccc5d425b12d37c69110ff94143`) uses
  a constants block (lines 24-28), six enums, two of them with sub-byte or parameterized
  forms (`uint 7`, lines 430-434; `ModbusDataType(uint 8 dataTypeSize)` with constant
  values, lines 380-408), a parameterized `discriminatedType ModbusADU(DriverType
  driverType, bit response)` with byte-order and encoding attributes (lines 36-76), the
  39-case
  `ModbusPDU` switched on two sub-byte discriminators and a parameter, where the error
  case matches on the first discriminator only (lines 78-276), five plain types (lines
  278-311) and a `dataIo DataItem` with `vstring` and string encodings (lines 318-378).
  Its field kinds are `simple`, `const`, `implicit`, `array` (`count` and `length` loops),
  `checksum` with `STATIC_CALL` (RTU and ASCII only, lines 65 and 73), `discriminator`
  and `typeSwitch`. It uses no `manual`, `manualArray`, `virtual`, `optional`, `padding`,
  `peek`, `reserved`, `assert`, `validation`, `abstract`, `state`, `unknown` or
  `batchSet` field. Its own comment assigns register alignment to the driver, not to
  `DataItem` (lines 313-317).
- **OPC UA.** Five mspec files under
  `protocols/opcua/src/main/generated/protocols/opcua/` are themselves generated and
  committed: under the `update-generated-code` profile a Groovy script downloads the OPC
  Foundation schemas from the moving `UA-Nodeset/raw/latest` URL
  (`protocols/opcua/src/main/script/downloadAndEditOpcuaNodeSet.groovy` lines 95-99) and
  XSLT turns them into mspec (`protocols/opcua/pom.xml` lines 67-297). `OpcuaProtocol`
  parses the five files into one type context
  (`protocols/opcua/src/main/java/org/apache/plc4x/protocol/opcua/OpcuaProtocol.java`
  lines 40-61). `opc-services.mspec` is 15,335 lines (1.3 MB) of 181 enums;
  `opc-manual.mspec` is 3,813 lines with six discriminated types, the largest of them
  `ExtensionObjectDefinition` with 354 cases (lines 204-2663), and uses `virtual` (18),
  `optional` (44), `reserved` (60) and `abstract` (3) fields, none of which Modbus has.

On the Stonecrab side, the Modbus PDU codec is hand-written from the same mspec
(`crates/stonecrab/src/modbus/pdu/`, ADR-0015 decision 2, D6) with semantic bounds the
mspec does not express (quantity 1 to 2000 or 1 to 125, the address span, a nonzero and,
for registers, even byte count, a 253-byte PDU, no write function codes under D3). Its
corpus is `fixtures/modbus/` (177 `.hex` vectors with independently written expected
files, six of them the upstream parser-suite frames retained by S04a) and the fuzz seeds
under `fuzz/corpus/fuzz_modbus_pdu/` and `fuzz/corpus/fuzz_modbus_mbap/`. D6 says a
future generator "must reproduce this output or pass the same vector corpus". There is no
`tools/` directory and no `xtask`; gates are `mise` tasks and `scripts/`.

## Options
- **(A) A native Rust generator.** `tools/stonecrab-codegen` parses MSpec itself and emits
  Rust over `stonecrab::codec`. One toolchain, the pinned Rust, generates and checks.
- **(B) The upstream Java and FreeMarker toolchain, with a Rust language module.** A new
  Maven module registers a `LanguageOutput` next to the five existing ones, with
  FreeMarker templates and a Java helper, and the upstream Maven plugin drives it.
- **(C) A hybrid.** Once per baseline, a maintainer runs the upstream parser under Java
  and exports the resolved type model to a committed file (for example JSON); a Rust
  emitter consumes that file. Upstream has no exporter, so this is new first-party Java
  over the `Default*` model classes.

## Criteria
| Criterion | (A) native Rust | (B) upstream toolchain | (C) hybrid |
| --- | --- | --- | --- |
| Reproducibility and determinism | One pinned toolchain (`rust-toolchain.toml`, with its `rustfmt`); source order and sorted maps by construction | JDK, Maven and plugin versions from a root `pom.xml` outside the pinned checkout; template output goes through a default-charset writer (`FreemarkerLanguageOutput.java` line 178) | The model export is reproducible only on a maintainer machine with Java; it must sort a `HashMap` (`MessageFormatListener.java` line 73) |
| Supply chain and CI toolchain | Rust only; the tool's few crates are audited and checked by `cargo deny`; the gate runs offline in CI | A JVM, Maven Central and the plugin graph (ANTLR, commons, logback; `protocol-base-mspec/pom.xml` lines 79-103, plus FreeMarker in `language-base-freemarker`) outside `cargo deny`; CODING_STANDARDS 17.6 keeps it off pull-request CI, so the regen gate cannot run | CI runs only the Rust half; the committed model is an intermediate CI cannot check against the mspec |
| Upstream baseline moves | Re-vendor the mspec, update the hashes, regenerate; a change to `MSpec.g4` or `Expression.g4` must be ported to the Rust parser first | Grammar changes arrive for free; helper or template API changes break the Rust module | Re-export the model; changes to the model classes change our export schema |
| Fidelity to MSpec semantics | Our parser must match upstream on grammar, expression precedence and type resolution; behavior is proven by vectors, not by upstream generator output | Upstream parse and resolution; emitter semantics still hand-written in templates | Upstream parse and resolution; emitter as in (A) |
| Safety under CODING_STANDARDS | Emitter is first-party Rust under our lints and tests; output checked by the crate's own lints | Emitter is Java and FreeMarker, outside clippy, coverage and mutation testing | Emitter as in (A); exporter is first-party Java with no gate |
| Effort | Highest at first: a parser for both grammars plus the emitter | A template set the size of an upstream language module (2,674 to 5,087 lines) plus Java CI | Exporter plus emitter; saves the parser, adds a second language |

## Decision
1. **Option (A): a native Rust generator.** The reasons, in order:
   - SPEC requirement 3 asks CI to regenerate and check for a clean diff on every change,
     and CODING_STANDARDS 17.6 keeps the Java path off pull-request CI. Only (A) lets the
     whole chain, mspec to Rust, run in the `regen` gate; under (B) and (C) CI would check
     at most the last step.
   - SPEC's porting rules make a native pipeline the end state, and requirement 8 asks the
     eventual native generator to reproduce or prove equivalent the accepted output.
     Choosing (B) or (C) now means writing the native generator later and proving it a
     second time.
   - Modbus needs a small construct set (Context), so the parser and the emitter can be
     proven end to end on Modbus before OPC UA, which is what E3 asks. OPC UA's mspecs are
     committed upstream, so they are more input files for the same tool, not an XSLT or
     download step for Stonecrab.
   - No JVM, Maven or unaudited plugin graph enters the project's tooling.

   The CODING_STANDARDS 17.6 bootstrap stays permitted but unused.
2. **The tool.** `tools/stonecrab-codegen` is a binary package with `publish = false` in
   its own Cargo workspace (its own `[workspace]` table and committed `Cargo.lock`), and
   the root `Cargo.toml` adds it to `exclude`, as ADR-0009 decision 4 did for `fuzz/`. The
   root workspace keeps one package (ADR-0018). It uses the root `rust-toolchain.toml` and
   is first-party code under CODING_STANDARDS 1.1 (tools may `unwrap`, section 6.2; the
   tool uses plain error types, because the root `deny.toml` bans `anyhow`), excluded from
   coverage (13.10). Its entry point is `generate` (2.8). Its dependencies are limited to
   `toml` (already audited for the crate) and a SHA-256 implementation (`sha2`, a new
   DEPENDENCY_AUDIT entry); S66 records the D23 check. The parser is hand-written
   recursive descent for the whole of `MSpec.g4` and `Expression.g4`, with no parser
   generator, and the emitter writes text and passes it through the pinned `rustfmt`, with
   no `syn` or `quote`.
3. **Parse everything, emit a growing subset, fail on the rest.** The parser accepts every
   construct of both grammars, so a vendored mspec always parses whole; an unresolved type
   reference is an error at parse time, as upstream's `validate()` makes it. The emitter
   supports a set of constructs that grows per protocol, and a construct outside the set
   in an enabled type is a generation error naming the construct, the type and the mspec
   line (SPEC requirement 2). The set is the MSpec feature matrix (requirement 8): a table
   in `tools/stonecrab-codegen/README.md` with one row per field kind, file-level block
   (`constants`, `globals`, `context`), type form, data type, built-in function and
   property access, each marked parsed, emitted and vector-validated, with the protocol
   whose vectors validate it. For Modbus, the `[constants ...]` block (modbus.mspec lines
   24-28) is parsed and not emitted: S66 emits no constants, and an option that enables a
   type whose expressions refer to a constant is a generation error naming the constant. A
   `globals` or `context` block is parsed and is a generation error until a protocol needs
   it. The "MSpec feature matrix" section of `docs/parity/modbus.md` points there once S66
   lands.
4. **Semantics of the emitted code.**
   - A `type` becomes a struct. A `discriminatedType` becomes an enum with one variant per
     case, wrapped in a struct when the parent has fields of its own. Discriminators are
     not stored: serialization derives them from the variant. Cases are tried in source
     order and the first match wins; a case with fewer values than discriminators matches
     on the prefix (`ModbusPDUError`); no match is `ParseError::InvalidDiscriminator`.
     The exception: when a case matches only a prefix, the discriminator values it left
     unmatched are stored in the variant and written back on serialize
     (`functionFlag` in `ModbusPDUError`), because they carry data. Upstream drops them and
     serializes 0 (`ModbusPDUError.java` lines 52-54); the hand model keeps the function
     (`ModbusPdu::Exception { function, code }`, so `81 02` must round-trip). This is a
     deliberate divergence, recorded in the feature matrix and in the parity record.
     Type parameters (`response`, `driverType`) are arguments of `parse`.
   - An `enum` becomes a Rust enum with its wire value and its constant values as `const
     fn` accessors; an unknown wire value is `InvalidDiscriminator`.
   - An `implicit` field is read, used by later expressions and not stored; on serialize it
     is computed from its expression. `const` fields are checked on parse, and a mismatch
     is `ParseError::InvalidDiscriminator` naming the field (no new variant).
   - Property access on a complex field (`pdu.lengthInBytes` in the `ModbusTcpADU` length
     field, `modbus.mspec` line 48) and the built-in functions `COUNT`, `ARRAY_SIZE_IN_BYTES`
     and `lengthInBytes` compute the serialized size of the value with the same checked
     arithmetic; each has its own row in the feature matrix.
   - A `dataIo` becomes a parse and a serialize function between bytes and `PlcValue`; a
     case whose value has no `PlcValue` variant (the strings until S49) is a generation
     error unless the case is disabled (decision 6).
   - Expressions are emitted fully parenthesized in MSpec precedence (`Expression.g4` lines
     38-59), never Rust's. Arithmetic is checked (`checked_*`, failing with
     `ParseError::Overflow`) and conversions use `try_from`, so a serialized byte count
     that does not fit its field fails instead of truncating. Upstream casts here (`(short)`
     in the generated Java, `.../ModbusPDUReadHoldingRegistersResponse.java` lines 91-93);
     the Rust behavior is recorded as a divergence in the protocol's parity record.
   - `STATIC_CALL` names, and later `manual` field expressions, resolve to functions in a
     hand-written support module next to the generated one (SPEC requirement 4); a name
     the protocol's configuration does not list is a generation error. Modbus would need
     `rtuCrcCheck` and `asciiLrcCheck`, which belong to RTU and ASCII (S59, S60), so S66
     disables those cases instead.
   - `virtual`, `optional`, `reserved`, `abstract` and the other kinds Modbus does not use
     are generation errors until the protocol that needs them (OPC UA) adds them with
     vectors.
5. **Safety of the output (CODING_STANDARDS 17.3, SPEC requirement 6).** The output reads
   and writes only through `BoundedReader` and `BoundedWriter`, never indexes or slices,
   never panics, and uses no `as` cast and no unchecked arithmetic. It is compiled under the
   lints of the module it lives in, including the `modbus` promotions (`indexing_slicing`,
   `arithmetic_side_effects`, `as_conversions`). A byte array is read with `read_bytes`
   against the bytes present; an array of fixed-width elements checks `count * width`
   against the remaining input before it allocates; an array of complex elements grows one
   parsed element at a time, and an element type whose minimum encoded size is zero is a
   generation error, so input length bounds every allocation. Runs of sub-byte fields (`bit`
   and `uint 7` in `ModbusPDU`) are read as one byte-aligned integer and split most
   significant bit first; a run that does not end on a byte boundary is a generation error.
   The split goes through a helper added to `stonecrab::codec` under an ADR-0013 addendum
   (ADR-0013 says a driver that needs a field the primitives do not offer "adds it here,
   not in the driver"; keeping generated files free of such helpers is this ADR's own rule).
   A recursive type is a generation error until the first protocol that has one adds a
   depth argument and `ParseError::RecursionLimit` (deferred by ADR-0013). The output
   copies no comment text from the mspec, only names, so the `geiger` grep for `unsafe`
   cannot match generated text. Identifiers are ASCII (`MSpec.g4` lines 403-405, which
   also allow `-`) and map to Rust case conventions with `-` replaced; a name collision or
   a Rust keyword is a generation error.
6. **Output layout.** Generated code for a driver lives in `crates/stonecrab/src/<driver>/`:
   - `generated.rs` declares one module per top-level mspec type, sorted by name, and
     `generated/<type_in_snake_case>.rs` holds each type. Both are written by the
     generator and nothing else; the generator deletes and rewrites the whole set, so a type
     removed upstream disappears from the diff too.
   - `generated/MANIFEST.toml` records the generator name and version, the upstream tag and
     commit, for each mspec its upstream path, its vendored path and its sha256, and the
     options: the enabled cases and types and the support module path. It holds no
     timestamp; git records when.
   - The vendored mspec is a verbatim copy with its ASF header (CODING_STANDARDS 1.3) at
     `upstream/mspec/<protocol>/<file>.mspec`, and `upstream/manifest.toml` lists its
     upstream path, git blob and sha256, following the S04a provenance entries. The
     generator refuses to run when the copy's sha256 differs from the manifest.
   - Options are per protocol and are recorded, never implicit. A disabled case is still
     parsed from the mspec and its discriminator values are known, so the generated
     parser refuses it with `InvalidDiscriminator`, as the hand codec refuses the write
     function codes (D3); it is not silently absent.
7. **Provenance header.** Every generated `.rs` file begins with:

   ```rust
   // @generated by stonecrab-codegen 0.1.0. DO NOT EDIT; regenerate with `mise run regen`.
   // Source: Apache PLC4X protocols/modbus/src/main/resources/protocols/modbus/modbus.mspec
   //   at ea229918f5d07700e04d7d69ab1645da679cb700 (v1.0.0),
   //   sha256 cb8ea683da2f3ddc207ee2b4469f1737a6a34ccc5d425b12d37c69110ff94143,
   //   vendored as upstream/mspec/modbus/modbus.mspec. Options: generated/MANIFEST.toml.
   ```

   The version is the tool's package version, bumped whenever its output changes for an
   unchanged input, so a header diff always means a generator or input change. There is no
   `#![allow(...)]` line: the output obeys the crate's lints, and a suppression the output
   genuinely needs is an item-level `#[expect(..., reason = "...")]` emitted by the
   generator, as in hand-written code (CODING_STANDARDS 5.8).
8. **Scope of S66 and the equivalence proof.** S66 vendors `modbus.mspec`, builds the tool
   for the Modbus constructs and generates `crate::modbus::generated` with the hand
   codec's domain enabled: `ModbusTcpADU`, `ModbusPDUError`, the read requests and
   responses of function codes 01 to 04, `ModbusErrorCode`, and the numeric cases of
   `DataItem` except `BOOL` (a lone bit is not a byte-aligned run, decision 5). Every
   other case is disabled in the options. The module is declared `#[cfg(test)]`, so it is
   compiled and linted by every `--all-targets` run and exercised by the tests but adds
   nothing to the library build, the public API or `semver`. Its files are still in the
   `src/**` package include, so S66 adds them to `packaging/stonecrab.txt` (the reviewed
   file list) and they ship as an unused `cfg(test)` module. The hand codec stays the
   shipped codec, and whether the driver later delegates to generated code is decided by
   the slice that does it. The proof is an in-crate test, behind `testkit` for the
   golden-vector loader, with a hand-written mapping from the generated types to
   `ModbusPdu`. For every input of the corpus (every `.hex` vector under
   `fixtures/modbus/`, the seeds under `fuzz/corpus/fuzz_modbus_pdu/` and
   `fuzz/corpus/fuzz_modbus_mbap/`, and property tests over arbitrary bytes and arbitrary
   valid `ModbusPdu` values), in the request and the response direction:
   - When the hand codec accepts, the generated codec accepts, maps to the same
     `ModbusPdu` (exception PDUs included, by the stored `functionFlag` of decision 4),
     and serializes back to the same bytes the hand encoder writes.
   - When the hand codec refuses, the generated codec refuses with the same `ParseError`
     variant, or accepts and the vector is in a committed table of hand-only checks, each
     row naming one: quantity zero or above its maximum, span past the address space, byte
     count zero, odd register byte count, PDU over 253 bytes, response function other than
     the expected one. Any other difference fails.
   - When the generated codec refuses what the hand codec accepts, the test fails.
   - ADU vectors compare the generated `ModbusTcpADU` with the MBAP framer followed by the
     PDU codec; where the framer refuses (for example `adu_protocol-id-1`, expected
     `Malformed`) the generated codec must refuse with a `ParseError`, and the pairing is
     a row of the hand-only table, and `DataItem` is compared with `TypedDecoder` on the
     `typed_*` vectors in the big-endian, high-word-first layout, where register bytes are
     the value's natural bytes; register alignment and word order stay hand-written
     (`modbus.mspec` lines 313-317).
   Every enabled variant also needs one valid and one negative vector of its own
   (CODING_STANDARDS 17.5). S66 lands only when the corpus passes with no unlisted
   difference.
9. **The S67 `regen` gate.** `mise run regen`, and a blocking CI job `regen` that runs the
   same task, does this:
   1. The one network step: `cargo fetch --locked` for the tool workspace and `cargo deny
      fetch db index` (the RustSec advisory database and the crates index, which the
      `yanked = "deny"` policy needs). Everything after it runs with `--offline`.
   2. Format, lint and test the tool workspace (`cargo fmt --check`, `cargo clippy
      --all-targets -- -D warnings`, `cargo test`) and run `cargo deny --offline check`
      over it (advisories read the database fetched in step 1). `cargo deny` reads
      `<cwd>/deny.toml`, so the tool workspace keeps a copy of the root `deny.toml` with a
      drift check, the approach `scripts/fuzz-smoke.sh` takes for `fuzz/`; the copy needs
      no license exception. S67 proves in CI that the offline run works with the fetched
      index (UNVERIFIED until then).
   3. Verify each vendored mspec against the sha256 in `upstream/manifest.toml`.
   4. Run the generator twice into two scratch directories and require byte-identical
      results, then run it in place and require an empty `git status --porcelain` for
      every `generated.rs` and `generated/` directory, which catches edits, new files and
      deletions alike.
   5. A self-test that edits one generated file in a scratch copy and expects the check to
      fail, like `scripts/parity-check.sh --self-test`.

   Steps 2 to 5 need no network because every input is in the checkout: the vendored
   mspec, the upstream commit and hashes in `upstream/manifest.toml`, the generator source
   and lock file, and the pinned toolchain with its `rustfmt`. It never clones PLC4X,
   never runs Maven or a JDK and never downloads OPC Foundation schemas, and after step 1
   `--offline` makes any network attempt fail the job. This amends CODING_STANDARDS
   section 19 (`cargo xtask generate --all && git diff --exit-code` becomes `mise run
   regen`, because `git diff` misses untracked files and there is no `xtask`), 17.1 (the
   header of decision 7, a manifest without a timestamp, and no `#![allow]`), 17.2 (the
   check is `mise run regen`, not a `git diff` of `src/generated/`), 18.2 (a `gen` commit
   holds `generated.rs`, `generated/` and the manifest, and the vendored mspec joins the
   manifest commit), 1.2 (generated code is not exempt from lints) and 17.6 (the bootstrap
   is unused). S67 adds margin notes at each of these.
10. **Upstream baseline moves.** A baseline move (CONTRIBUTING, "Changing the upstream
    baseline") re-vendors each mspec, updates the hashes and the commit in
    `upstream/manifest.toml`, and then regenerates in a following `gen` commit of the same
    PR. Generated output is not logic, but CONTRIBUTING item 1 (the manifest change alone)
    and CODING_STANDARDS 18.4 (no regeneration mixed with logic) stay true commit by
    commit; S67 amends CONTRIBUTING to say the vendored mspec joins the manifest commit
    and the `gen` commit follows. The generated diff is reviewed as part of the parity
    diff. Before that, the maintainer diffs `MSpec.g4` and `Expression.g4` between the two
    commits; any grammar change is a generator change landed first with its own tests. A
    maintainer-only command, `stonecrab-codegen check <plc4x checkout>`, parses every
    `*.mspec` of a local read-only checkout without emitting anything and reports each
    file's result; its output for the new commit is recorded with the move. It never runs
    in CI.
## Consequences
- The project owns a second Rust code base: a parser for two grammars and an emitter, of
  the order of an upstream language module (Context). It is linted and tested like other
  tools and carries the feature matrix.
- Two Modbus codecs coexist after S66: the shipped hand codec and a test-only generated
  one. The generated one is evidence that the generator is right, not a second runtime
  path. A later slice that enables more cases (FC05 to FC16 for S45, FC20 and FC21 for S50)
  can choose the generated code instead of new hand-written PDUs.
- `rustfmt` is part of the generator's output, so a toolchain bump that changes formatting
  shows up as a regen diff in the bump's own change, which is the intended place to review
  it.
- The header names the tool version, so a generator change that alters output touches
  every generated file of every protocol; such changes are regeneration-only commits under
  the `gen` type (CODING_STANDARDS 18.2) after the generator change itself.
- `crates/stonecrab/src/modbus/generated/` is excluded from the handwritten coverage
  thresholds (CODING_STANDARDS 13.10, 17.5); `scripts/coverage-gate.sh` already excludes
  any `generated/` path component (`EXCLUDE` in the script), so only a file named
  `generated.rs` needs adding there. `.gitattributes` has no generated rule today; S66 adds
  one for `crates/stonecrab/src/*/generated/**` and `generated.rs`.
- OPC UA will extend the matrix with `virtual`, `optional`, `reserved` and `abstract`,
  `STATIC_CALL` helpers and recursion bounds, and its size (about 19,500 mspec lines, 354
  cases in one switch) will test compile time and file count. Its mspecs come from OPC
  Foundation schemas fetched from a moving URL, so the schema version behind the pinned
  files is UNVERIFIED and goes to the M0 inventory; Stonecrab consumes the committed files
  and never reruns that step.

## Risks
- **Parser fidelity.** A grammar or precedence mistake in our parser could pass review.
  Mitigations: the parser covers the whole grammar, expressions are emitted parenthesized
  in MSpec precedence, unresolved references fail, the `check` command runs over every
  upstream mspec at each baseline, and behavior is judged by vectors, including the
  upstream parser-suite frames, never by agreement with an upstream generator.
- **No upstream oracle.** Upstream's own languages disagree (`test.mspec` lines 41-42 and
  230-244), so "what upstream generates" is not one behavior. Where Rust differs from
  PLC4J, the parity record states it, as for the checked byte counts in decision 4.
- **Vendoring needs approval.** Decision E2 approved redistributing the Apache-2.0 PLC4X
  test vectors; `modbus.mspec` is a protocol description, not a vector. S66 needs the
  maintainer's approval to vendor it with NOTICE attribution before it can land.
- **Scope creep in S66.** The equivalence table of hand-only checks must stay a closed list
  of named checks; a row added to make a failing vector pass, without a named check, is a
  review failure.
- **The generator becomes the bottleneck for OPC UA.** If its construct set lags, M3
  waits on it. E3 accepted that cost to avoid hand-porting an unverifiable OPC UA codec
  (docs/POST_ALPHA_PLAN.md, "Risks").

## Alternatives considered
- **(B) The upstream toolchain with a Rust language module.** Rejected: the gate SPEC asks
  for cannot run on pull requests without Java in CI, which CODING_STANDARDS 17.6 forbids,
  and the emitter would sit outside every Rust gate. It remains the documented fallback if
  the native parser proves unworkable, through an amendment of this ADR.
- **(C) An exported upstream model and a Rust emitter.** Rejected: it adds first-party Java
  and a committed intermediate that CI cannot check against its mspec, and a native parser
  would still be owed under SPEC's porting rules. Its one gain, upstream's parse and type
  resolution, is pursued in (A) by the `check` command and the vectors.
- **Keep hand-porting, no generator.** Rejected by E3 for OPC UA's size.
- **One generated file per mspec.** Rejected: `opc-manual.mspec` would become one very large
  file; one file per type keeps diffs and compiler messages local, and the provenance
  header still names the mspec each type came from.
- **A `MessageCodec` trait for generated types.** Not added: generated types expose
  inherent `parse` and `serialize` functions, and a trait waits for a second caller that
  needs dispatch over it (this ADR's own principle: no abstraction before its second caller).

## Compliance
- S66: `tools/stonecrab-codegen` with its README and feature matrix,
  `upstream/mspec/modbus/modbus.mspec` and its manifest entry,
  `crates/stonecrab/src/modbus/generated.rs`, `crates/stonecrab/src/modbus/generated/`
  with `MANIFEST.toml`, the equivalence test with its table of hand-only checks,
  DEPENDENCY_AUDIT entries for the tool, the root `exclude` entry and the
  `packaging/stonecrab.txt` update for the generated files. `mise run check`,
  `coverage`, `lint-attrs` and `package` stay green.
- S67: the `regen` task in `mise.toml`, the blocking `regen` job in
  `.github/workflows/ci.yml` and its self-test.
- Every later protocol that generates code adds its rows to the feature matrix, vectors for
  each new construct (SPEC requirement 5) and its options to its `MANIFEST.toml`.

## Addendum (S66 and S67, 2026-10-29): the generator, the Modbus proof and the regen gate
### Status
Accepted (2026-10-29), as slices S66 and S67 of docs/POST_ALPHA_PLAN.md, after the
maintainer approved vendoring `modbus.mspec` (decision E8, 2026-10-29). The decisions above
stand; this addendum records what the implementation settled and the two places where it
departs from their letter, each with its reason.

### Context
S45 (the write PDUs FC06, FC15 and FC16) and S50 (the file record PDUs FC20 and FC21) landed
in the hand codec while this ADR was written, so the hand codec accepts PDUs that decision 8
leaves disabled in the generated one; decision 8's third rule ("when the generated codec
refuses what the hand codec accepts, the test fails") cannot hold for them. Running the
proof also showed three refusals of the hand codec that are not in decision 8's list of
hand-only checks, and that an implicit field read and dropped, as upstream does, lets an ADU
whose MBAP length disagrees with its PDU through, which decision 8 requires the generated
codec to refuse wherever the framer refuses.

### Decision
1. **The tool.** `tools/stonecrab-codegen` (package 0.1.0, its own workspace, `Cargo.lock`,
   `deny.toml` copy and `.gitignore` for its `target/`) with `generate <options>
   [<output>]` and the maintainer-only `check <checkout>` of decision 10. The parser is
   `src/mspec.rs` (`MSpec.g4`) and `src/expression.rs` (`Expression.g4`, upstream's
   precedence), both hand-written recursive descent; the emitter is `src/emit.rs` and
   `src/emit/`; `src/output.rs` writes the header, runs the pinned `rustfmt` and replaces the
   output set. Dependencies: `toml` 0.9 (the root's version, features and `toml_parser` pin; 1.1 with the
   root since 2026-10-29, see the ADR-0016 addendum of that date)
   and `sha2` 0.11 with default features off. The tool's lint table copies the root
   `[workspace.lints]`, because a workspace cannot inherit another's; its items are
   `pub(crate)` in `pub mod`s of the binary, which satisfies both `unreachable_pub` and
   `redundant_pub_crate` without suppressions.
2. **Options and provenance.** A protocol's options are `tools/stonecrab-codegen/protocols/
   <protocol>.toml` (output directory, support module, enabled types, enabled cases); its
   vendored files are the `[[mspec]]` entries of `upstream/manifest.toml` with that
   `protocol` (upstream path, vendored path, git blob, sha256). `generated/MANIFEST.toml`
   records the generator and version, the upstream project, tag and commit, each mspec and the
   options. The header of decision 7 is written verbatim; it is added after `rustfmt`, which
   skips any file whose first lines say `@generated`, so the files are formatted by
   construction and `cargo fmt --check` skips them.
3. **Enum values are raw text.** An enum value and its constants are kept as written, not
   parsed as expressions, as upstream's `MessageFormatListener.enterEnumValueDefinition`
   takes them (lines 744-784): `canopen.mspec` writes binary literals (`'0b0000'`) and
   `knx-master-data.mspec` an empty constant, neither an expression of `Expression.g4`. A
   value without a wire value (upstream counts up) is a generation error until a protocol
   needs it.
4. **The emitted subset is the validated subset.** The feature matrix in
   `tools/stonecrab-codegen/README.md` lists every construct; the emitter writes only what
   the Modbus vectors prove and refuses the rest with the construct, the type and the line:
   `byteOrder` is `BIG_ENDIAN` only, a case `*` value, a default case, a `*Name` case and case
   parameters are refused, the only operator is `+` (as `checked_add`), the only built-in is
   `COUNT` of a byte array and the only property `lengthInBytes` of a nested type. A recursive
   type is refused (decision 5).
5. **Implicit fields are checked on parse.** Once a case is read, each implicit field is
   compared with its expression over the parsed values, and a mismatch is
   `ParseError::InvalidDiscriminator` naming the field. Upstream reads and drops it
   (`ModbusTcpADU.java` lines 96-97 under `plc4j/drivers/modbus/src/main/generated/`); without
   the check an ADU with a wrong MBAP length and a valid PDU parses, where the framer refuses
   it. This is a divergence of the generated codec only, recorded in the parity record
   (`MB-GEN-IMPLICIT`).
6. **Generated code conventions.** Variants keep the mspec's case names; a disabled case keeps
   its arm and is refused with `field: "<Type>.<Case> (disabled)"`, a value no case matches
   with `field: "<Type>"`, and the `value` is the last discriminator that is not a `bit`.
   Arms with the same body are merged (clippy's `match_same_arms`), the last write of a
   `serialize` arm is its value (`branches_sharing_code`), and the generator emits
   `#[expect(clippy::enum_variant_names, ...)]` where clippy's rule fires on the mspec's
   names and `#[expect(clippy::redundant_pub_crate, ...)]` on every item, as hand-written
   private modules do (`crate::modbus::session::Wire`). A byte array is
   `read_bytes(count, usize::MAX)`: the bytes present bound it.
7. **The bit-run helpers stay next to the generated module.** `split_bits` and `join_bits`
   live in the hand-written support module `crate::modbus::generated_support`, not in
   `stonecrab::codec` as decision 5 says. While the generated codec is test-only, a public
   codec function would grow the published API (and the `semver` baseline) for a test-only
   caller; the slice that ships generated code moves them under the ADR-0013 addendum
   decision 5 asks for.
8. **`cfg(all(test, feature = "testkit"))`.** The generated module and the support module are
   declared with this instead of decision 8's `#[cfg(test)]`: the proof needs the golden-vector
   loader of `testkit`, a build without it would compile generated code nothing uses (dead
   code warnings), and clippy treats only a plain `#[cfg(test)]` module as test code, so the
   generated code is linted as library code, which is what decision 5 wants. The proof
   itself is `#[cfg(test)]` and `#[cfg(feature = "testkit")]`, an in-crate test, because a
   private `cfg(test)` module cannot be reached from `tests/`.
9. **The enabled set of S66 is decision 8's, and the disabled cases are a rule of the
   proof.** S45 and S50 do not widen it: enabling FC06, FC15, FC16, FC20 and FC21 is for the
   slice that chooses generated code for writes (Consequences above), and it will meet an
   upstream defect there (`ModbusPDUWriteFileRecordResponseItem` reads `recordLength` bytes
   where V1.1b has twice that, `MB-ADDR-EXTENDED`). Decision 8's rules gain one: an input of
   a disabled case is outside the generated codec's domain, and the generated codec must
   refuse it with `InvalidDiscriminator` naming the disabled case, whatever the hand codec
   does (decision 6). Nothing else is compared for it.
10. **The closed list of hand-only checks.** Decision 8's seven rows stand. Three are added,
    each naming the check that makes it: a response byte count above 250 (`MB-LIMIT-PDU`; the
    mspec reads any `uint 8` count and a 251-byte count still fits a 253-byte PDU); an
    exception PDU given to the request parser (the hand request decoder has no function code
    with the exception bit, and the mspec matches `ModbusPDUError` on `errorFlag` whatever
    `response` is); and an exception response for a function code the hand codec does not
    implement (`FunctionCode::Unknown`; the mspec's `ModbusPDUError` takes any
    `functionFlag`). The ADU rows are an ADU over 260 bytes (the framer's `Oversized`,
    `MB-LIMIT-ADU`), where the generated codec is not constrained, and every other framer
    refusal or incomplete frame, where it must refuse. When a hand-only check fires, the
    generated codec may accept or refuse. The proof fails when a row is exercised by no
    input, so the list cannot go stale.
11. **ADU comparisons.** The framer cuts the stream and the generated `ModbusTcpADU` parses from
    the same position. When both accept, the transaction id, unit id, consumed bytes, mapped
    PDU and serialized bytes must agree; when the hand side refuses, the generated side must
    refuse unless a hand-only row applies, and the refusal variant is compared at the PDU
    level, where the PDU of every framed ADU is also compared on its own (a generated parser
    that reads past an ADU's end fails the length check of item 5 instead of the PDU
    decoder's `Truncated`).
12. **`DataItem`.** The `typed_*` vectors in the `BIG_ENDIAN` layout are compared with
    `TypedDecoder` (values by bits, so NaN payloads count); a lone 8-bit value sits in the low
    byte of its register, which the test strips by hand (register alignment stays
    hand-written). The negative vector of every enabled case is the same value one byte
    short.
13. **The regen gate.** `scripts/regen.sh` runs the five steps of decision 9; `mise run regen`
    runs it and its `--self-test`, and the blocking CI job `regen` does the same; `mise run
    generate` rewrites in place. Step 4's in-place check compares a copy of `generated.rs` and
    `generated/` taken before the run with the files after it (`diff -r`, which an edit, a new
    file and a deleted file all fail) and requires `git status --porcelain
    --untracked-files=no` to be empty for them. On a clean checkout, as in CI, that is the
    `git status --porcelain` of decision 9; the copy also works before the generated files are
    first committed. The offline run with the fetched advisory database and index works
    locally (`cargo deny --frozen check` after `cargo deny --locked fetch db index`); the first
    hosted run proves it in CI.
14. **`check` at the pinned baseline.** On 2026-10-29, over the read-only checkout of v1.0.0, 57
    of 59 mspec files parse and resolve (the files of one `protocols/<name>` module resolve
    together, as each protocol loads them), the five OPC UA files included. Two break
    `MSpec.g4` and are accepted upstream only through ANTLR's error recovery: `ads.mspec` line
    1088 (a `validation` description in ticks where the grammar takes a `"string"`) and
    `eip.mspec` line 310 (a stray `]`). The parser reports them; it does not imitate the
    recovery.

### Consequences
- The equivalence proof (`crate::modbus::generated_equivalence`) passes over the 296 Modbus
  PDU and ADU vectors, the 35 fuzz seeds and two inline inputs of item 10, in both directions,
  and over property tests of arbitrary bytes, arbitrary PDUs in an MBAP frame and arbitrary
  valid PDUs of the enabled domain, with no difference outside the rules above.
- Done in the landing commit, outside these slices' files: the margin notes of decision 9 in
  CODING_STANDARDS (1.2, 17.1, 17.2, 17.6, 18.2 and section 19), the CONTRIBUTING change of
  decision 10 (the vendored mspec joins the manifest commit, the `gen` commit follows), the
  crate's own `NOTICE`, which now names the mspec the shipped (`cfg(test)`) generated files
  come from, and the `geiger` grep of CI, which now also scans `tools/` and `fuzz/`.

### Compliance
- S66: `tools/stonecrab-codegen` with its README and feature matrix, `upstream/mspec/modbus/
  modbus.mspec` and its `[[mspec]]` entry, `crates/stonecrab/src/modbus/generated.rs`,
  `generated/` with `MANIFEST.toml`, `generated_support.rs`, `generated_equivalence.rs`, the
  `sha2` entry of `docs/DEPENDENCY_AUDIT.md`, the root `exclude` entry, the coverage exclusion
  of `generated.rs`, the `.gitattributes` rule for generated files named in Consequences
  above and the `packaging/stonecrab.txt` update.
- S67: `scripts/regen.sh`, the `regen` and `generate` tasks in `mise.toml` and the blocking
  `regen` job in `.github/workflows/ci.yml` with its self-test.

## Addendum (S77, 2026-10-30): type structs, the protocol byte order and duplicate type names
### Status
Accepted (2026-10-30), as slice S77 of docs/POST_ALPHA_PLAN.md (Phase H, M3 OPC UA), after the
maintainer accepted decisions E13 to E22 on 2026-10-30. The decisions and the addendum above
stand. S75 listed the gaps between the generator and the OPC UA model; this closes the first
three, G1, G2 and G10. G3 to G7 follow in S78 and S79.

### Context
Decision 4 says "A `type` becomes a struct", and the feature matrix of the S66 addendum marked
the form parsed and not emitted: `emit.rs` refused it. The byte order was a constant of the
emitter (`ByteOrder::BigEndian`), and a `byteOrder` attribute of `LITTLE_ENDIAN` was refused;
OPC UA is little-endian (`opc-manual.mspec` line 29 names `LITTLE_ENDIAN` on its `OpcuaAPU`
type; the Part 6 section is UNVERIFIED), so a protocol needs to choose it once, not per type. The emitter put each type in a map that kept the last of two definitions of a name,
and `opc-manual.mspec` defines `NodeIdType` twice, so an input could silently change meaning.
No OPC UA mspec is vendored before S88 (decision E13), so these are proven on first-party
synthetic mspec text.

### Decision
1. **`type` structs (G1).** A `type` becomes a struct of its stored fields, each
   `pub(crate)` (an enum variant's fields are public, a struct's are not), with `parse`,
   `serialize` and, when an expression reads its `lengthInBytes`, `length_in_bytes`. Its fields
   are planned and emitted by the code a case of a `discriminatedType` uses, so the construct
   set is the same: runs of sub-byte fields, byte-aligned numbers, `const` and `implicit`
   fields (checked on parse, decision 5 of the S66 addendum; neither is stored), `byte` arrays
   with a `count`, enums, and nested `type` and `discriminatedType` fields with name
   arguments. Parameters are arguments of `parse`, as for a `discriminatedType`. A `type` that
   stores no field, or has a `discriminator` field, is a generation error. The struct derives
   `Eq` unless a field is a float. A fixed length is a `const fn length_in_bytes(&self) ->
   Result<usize, ParseError>` under an item-level `#[expect(clippy::unused_self,
   clippy::unnecessary_wraps, ...)]`: callers ask the value and add the result to a fallible
   sum, whatever the type. The generator also emits `#[expect(clippy::struct_field_names,
   ...)]` where clippy's rule fires on the mspec's field names, with the predicate in
   `naming.rs` as for `enum_variant_names`. A nested type's length is asked only of a type
   whose `lengthInBytes` an expression reads (decision 4); a length that reaches a nested
   type that is not itself read is a generation error, as before.
2. **The byte order is a protocol option (G2).** `protocols/<protocol>.toml` takes
   `byte_order = "BIG_ENDIAN" | "LITTLE_ENDIAN"`, `BIG_ENDIAN` when absent (the mspec default).
   Every multi-byte read and write of the protocol's output, enums and `dataIo` included, uses
   it. A `byteOrder` attribute on a type must name the protocol's order; a different value is
   a generation error (a per-type byte order waits for a protocol that needs one), as is a
   value that is not a byte order. `generated/MANIFEST.toml` records `byte_order` when the
   options file sets it and says nothing otherwise, so the Modbus manifest and files are
   unchanged. A `batchSet` with attributes is a generation error: the emitter flattens batch
   sets, and a `byteOrder` on one would have been dropped silently.
3. **A duplicate type name is a generation error when the output uses it (G10).** Two
   definitions of a name, in one file or across the files of a protocol, fail with the name and
   both lines when the name is an enabled type or one an enabled type refers to. A duplicate
   that nothing generated refers to changes no output and is not reported, so a vendored mspec
   with a duplicate (`NodeIdType` of `opc-manual.mspec`) still generates the types that do not
   touch it. The generator does not pick one. (Which definition upstream's loader keeps is UNVERIFIED, the
   sparse checkout holds no code-generation sources; `NodeIdTypeDefinition`, `opc-manual.mspec`
   lines 2788-2794, uses case values that exist only in the second `NodeIdType`, line 2942,
   which suggests the last wins.) The message names both lines but not the file, as every
   `GenError` does.
4. **Fixtures validate what no protocol uses yet.** The matrix rule of decision 3 and the S66
   addendum is that a row is emitted only when validated. For a construct no protocol in the
   tree uses, the tool's own tests are the validation: `src/emit/fixtures.rs` generates
   synthetic mspec text, compiles the modules with `rustc` next to a small stand-in for
   `stonecrab::codec`, runs a parse, a serialize and the refusals, under both byte orders. The
   stand-in is not the shipped codec; the first protocol that uses a construct adds its own
   vectors in the crate (S88 for OPC UA), where the output is linted and tested under the
   module's lints. The matrix names the fixtures as `S77 fixtures`.
5. **The tool version moves to 0.2.0.** Decision 7 bumps it "whenever its output changes for an
   unchanged input"; the slice row also asks for the version to follow the feature set. The
   version is in the header of every generated file and in `MANIFEST.toml`, so the bump
   rewrites the eight Modbus files by their header line and manifest version only. They are
   committed as a separate `gen` commit after the generator commit (consequences of decision
   7), and `mise run regen` then reproduces the committed tree byte for byte. The generator
   commit alone leaves `regen` red on the stale headers, as any generator change that alters
   output does; the two commits land together.

### Consequences
- The generator can emit the structs and the little-endian default the OPC UA service layer
  needs. Recursion through structs is refused as it is for discriminated types (decision 5);
  literal type arguments, `reserved` fields, `[external]` types and the remaining gaps are
  S78 and S79.
- Not done: a struct that derives `Eq` over a nested type with a float field (the derive looks
  only at the struct's own fields); a transitive `lengthInBytes` (a type read through another
  type's length must itself be read by an expression); `type` as a `dataIo` field. Each is a
  generation or compile error, never a wrong value, and a protocol that needs one adds it with
  vectors.

### Compliance
- S77: `tools/stonecrab-codegen` (`src/emit/structs.rs`, `src/emit/fixtures.rs`, the byte
  order in `src/emit.rs`, `src/main.rs` and `src/output.rs`, the duplicate check in
  `src/emit.rs`, `naming.rs`) and its README (options, semantics, matrix). Tests
  `generator_emits_type_structs`, `generator_little_endian_protocol_default_round_trips` and
  `generator_rejects_duplicate_type_names`. `mise run regen` passes twice. The only change
  under `crates/` is the version in the header of the eight Modbus generated files and in their
  manifest (the `gen` commit of item 5).

## Addendum (S78, 2026-10-30): count arrays, int 32 and uint 32 counts, null arrays
### Status
Accepted (2026-10-30), as slice S78 of docs/POST_ALPHA_PLAN.md (Phase H, M3 OPC UA), after the
maintainer accepted decisions E13 to E22 on 2026-10-30. The decisions and the addenda above
stand. This closes gaps G3 and G4 of the list S75 made; G5 to G7 follow in S79.

### Context
Decision 4 allowed one array: a `byte` array with a `count`, read with `read_bytes`, whose
bytes present bound the allocation. OPC UA arrays are of numbers, enums and structures, their
counts are `int 32` fields where -1 means null and 0 means empty
(`opc-manual.mspec` lines 217 to 381: `[implicit int 32 noOfParameters 'parameters == null ? -1
: COUNT(parameters)']` before `[array KeyValuePair('14535') parameters count
'noOfParameters']`), and the count is chosen by the peer. ADR-0034 decision 4 and 5 ask for
null and empty to stay apart and for every count to be checked before it allocates. No OPC UA
mspec is vendored before S88 (decision E13), so this is proven on first-party synthetic mspec
text, as in S77.

### Decision
1. **`count` arrays of any emitted element (G3).** A `count` array's element is a `byte`, a
   byte-aligned number (`uint`, `int` or `float` of 8, 16, 32 or 64 bits), an enum with a base
   type, or a `type` or `discriminatedType` with name arguments. A `bit` array and the
   `length` and `terminated` loops stay generation errors. A `byte` array with an unsigned
   count is emitted as before (`Vec<u8>` from `read_bytes`), so the Modbus output is
   unchanged. Any other array is read behind a check of its count against the remaining input,
   and the vector is reserved only after it:
   - an element of a fixed width needs `count * width` bytes; the product is computed with
     `checked_mul` and compared with `reader.remaining()`;
   - an element of no fixed width needs at least one byte, so the count is compared with the
     remaining bytes, and the vector grows one parsed element at a time (an element's own
     failure, `Truncated` included, is that element's);
   - either failure is `ParseError::CountLimit { field, count, max }`: the mspec field name,
     the declared count and the most elements the input could hold. The variant is what
     ADR-0034 decision 5 names; S86 adds it to the shipped `ParseError` with these fields
     (`&'static str`, `usize`, `usize`), additively on the `#[non_exhaustive]` enum, and maps
     it to `ErrorKind::ResourceLimit`. Until then no shipped module uses it, and the fixtures'
     stand-in defines it.
2. **Null and empty stay apart (G4).** An array is nullable when an `implicit` field of its
   field list writes it as null (that field's expression contains `name == null`), or when
   its count is the bare name of a `simple int 32` field of the list
   (`[simple int 32 arrayLength] [array uint 8 value count 'arrayLength']`, the
   `ByteStringArray` of `opc-manual.mspec` line 136, where a null ByteString is legal on the
   wire and upstream reads any negative count as null). A nullable array is `Option<Vec<T>>`
   (also for `byte`), `None` is count -1 and `Some(vec![])` count 0; every other array is
   `Vec<T>`. Where the count is a stored field, `serialize` checks the value against it
   (`None` needs -1, `Some` its length) and refuses a disagreement as `InvalidDiscriminator`,
   because the bytes would not parse back to the value. This matches ADR-0034 decision 4
   (generated arrays are `Option<Vec<T>>`, -1 is `None`) for every count that is a field; a
   count that is an expression (`stringLength == -1 ? 0 : stringLength`) is the author's own
   mapping of -1 and stays a `Vec<T>`. A nullable array's count must be signed (a generation error
   otherwise: -1 would not fit). A count that is any other negative value is
   `ParseError::InvalidDiscriminator { field: "<array>", value: <magnitude> }`, which maps to
   `ErrorKind::MalformedData` as ADR-0034 decision 5 asks for "no valid encoding"; a count of
   -1 for an array that is not nullable is the same refusal, unless the count expression maps
   it to a count (`stringLength == -1 ? 0 : stringLength`, as `PascalByteString` does).
3. **`implicit` fields of `uint 32` and `int 32`.** An `implicit` field is `uint 8`, `uint 16`,
   `uint 32` or `int 32`. It is read and written with the protocol's byte order, computed on
   serialize with `try_from` (so a count that does not fit is `Overflow`) and checked on parse
   as decision 5 of the S66 addendum says. A `uint 8` or `uint 16` field with an unsigned
   expression is compared as a `usize` exactly as before; any other combination is compared as
   an `i64`, and the `value` of the `InvalidDiscriminator` is the field's magnitude
   (`unsigned_abs`) when it is signed.
4. **The expression subset grows to `==`, `?:`, `null` and the prefix `-`.** An expression is
   typed as a `usize`, an `i64` or a `bool`; a mix of the first two is an `i64` (a literal
   takes the type of its context, any other `usize` converts with `try_from` and `Overflow`).
   `a == b` compares two numbers; `name == null` (either way round) is `name.is_none()` and is
   accepted only for a nullable array; a prefix `-` takes an integer literal (`-1_i64`) or
   negates with `checked_neg`; `c ? a : b` takes a comparison for `c` and two numbers.
   `COUNT(x)` of a nullable array is 0 when it is `None`. `!`, `!=`, `-` as a binary operator
   and the remaining operators stay generation errors naming the operator. An integer literal above `i64::MAX` is a
   generation error, not a compile error of the output.
5. **Fixtures and the property.** `src/emit/array_fixtures.rs` generates synthetic mspec text,
   compiles it with `rustc` next to the stand-in of S77 (little-endian) and runs programs
   that check the wire layout, the bounds, the refusals and a property loop over random
   tables and mutated bytes (`generator_null_array_is_distinct_from_an_empty_array`). The loop
   is a fixed-seed xorshift inside the compiled program, not the `proptest` crate: the tool
   has two dependencies by decision 1, the program is built with `rustc` alone, and a new
   dependency of the tool workspace would be a decision of its own. The output was compiled
   and linted once under the tool workspace's lint table in a scratch crate.
6. **The tool version stays 0.2.0.** Decision 7 bumps it when the output changes for an
   unchanged input. No input of the tree changed: the Modbus output is byte for byte what
   S77 committed, and the inputs this addendum accepts were generation errors. Hence no
   `gen` commit follows.

### Consequences
- The generator can emit the arrays and counts of the OPC UA service layer (S88). The first
  use of a generated array in the crate waits for S86's `CountLimit`; a slice that generates
  into the crate before that fails to compile on the missing variant, which is the signal.
- Not done: an array of `bit` (`isCertificateAssigned` of `opc-manual.mspec` line 319), the
  `length` and `terminated` loops, a length that reaches an array of structs whose type no
  expression reads the `lengthInBytes` of (a transitive length is still refused), a count
  expression outside the subset, and arguments that are not names (S79). A fixed-count
  array (`count '2'`) is not checked against its length on serialize, as a `byte` array never
  was; the callers build the value.
- A nullable array with `COUNT(name)` as an `implicit` expression and no `== null` is not
  nullable: `bodyLength 'COUNT(rawBody)'` gives `Vec<u8>`, and a wire count of -1 is the
  negative-count refusal.

### Compliance
- S78: `tools/stonecrab-codegen` (`src/emit/fields.rs`: the array part, the typed expressions,
  the array reads and writes; `src/emit.rs`: `Ty::Array`; `src/emit/switch.rs`,
  `src/emit/structs.rs`) and its README (semantics, matrix, fixtures). Tests
  `generator_complex_array_count_is_checked_before_allocation`,
  `generator_null_array_is_distinct_from_an_empty_array` and
  `generator_implicit_int32_count_round_trips`, and the refusals
  `generator_array_gaps_are_refused_with_the_construct_the_type_and_the_line`,
  `generator_array_gaps_name_the_line_of_the_field`,
  `generator_array_of_its_own_type_is_refused_as_recursive` and
  `generator_integer_literals_outside_i64_are_refused`. `mise run regen` passes; the Modbus
  generated files are unchanged.

## Addendum (S79, 2026-10-30): literal arguments, cases named as types, reserved fields and external types
### Status
Accepted (2026-10-30), as slice S79 of docs/POST_ALPHA_PLAN.md (Phase H, M3 OPC UA), after the
maintainer accepted decisions E13 to E22 on 2026-10-30. The decisions and the addenda above
stand. This closes gaps G5, G6 and G7 of the list S75 made, and with S77 and S78 the generator
has the constructs of the connect-and-read subset; whether it generates the real model is
S80's evidence. Decision 5's amendment for recursion through external types is S80's addendum
to write: this one adds the mechanism it will describe and records, in item 5, what the
mechanism guarantees.

### Context
`opc-manual.mspec` refers to a case of a `discriminatedType` as a type with the parent's
parameters as arguments (`[simple RequestHeader('391') requestHeader]`, line 1255, where
`RequestHeader` is the case `'391'` of `ExtensionObjectDefinition(int 32 extensionId)`;
upstream generates `RequestHeader.staticParse(readBuffer, (int) (391))`, read in
`ReadRequest.java:94`). It has 50 `[reserved uint 7 '0x00']` fields in a byte with a flag,
besides `bit`, `int 5`, `int 2`, `uint 6` and `uint 2` ones. And its recursion runs through
types a generator should not write: a Variant holds Variants and an ExtensionObject whose
body is generated again. ADR-0034 decision 3 fixes the contract for those types, hand
written in S85 and S86; the generator needed the other end of it. Case parameters
(`OpcuaOpenRequest (uint 32 totalLength)`) are the case-level counterpart of the arguments.

### Decision
1. **Literal arguments (G5).** An argument at a use of a `type` or `discriminatedType` is a
   name of the caller's scope as before, or a literal: an integer that fits the callee's
   parameter, negated or not, or `true` and `false` for a `bit`. The call passes it as
   written (`Block::parse(reader, 2)`). A literal for an enum parameter, a wildcard, and any
   other expression stay generation errors. Argument errors now name the line of the field.
2. **A case named as a type (G5).** A field or array element whose type is a case name of a
   `discriminatedType` (and no `type`) is a reference to the case as a struct. The generator
   makes each case a `type` of its own, with the parent's parameters and attributes, and
   generates the ones an enabled field names, and the ones those name in turn, as structs in
   modules named for the case, whether or not the options enable them as variants of their
   parent (the `[cases]` list is the list of variants, unchanged; a case that is only
   referenced needs no entry). The struct's `parse` takes the parent's parameters in their
   order; a reference passes them, and for each parameter that the parent's `typeSwitch`
   switches on, where the case's value is a literal, the reference must pass that literal,
   because `RequestHeader('392')` names no case. The check is made on the literal, so it
   holds when the parent itself is not generated. It is a generation error when the parent
   reads a field before its `typeSwitch` (the standalone case would drop it), when the case
   has parameters of its own (the reference supplies only the parent's), when the name is a
   case of two types, or when the references form a cycle (decision 5 of the S66 addendum).
   The variant of the parent, if enabled, still holds its own copy of the fields; the two
   are generated from the same field list, so they cannot differ.
3. **Case parameters (G5).** A case parameter of a `discriminatedType` names a value of the
   same type that its parent has read or was given before the `typeSwitch`: a parameter, or
   a discriminator or `implicit` field. That was already in scope for the case's fields, so
   the declaration is checked and nothing is passed; a case parameter that matches nothing
   is a generation error. A case's `serialize` has no such scope, as before: an expression
   that needs the value there is a generation error. A `dataIo` case with parameters stays
   refused.
4. **`reserved` fields (G6).** `[reserved <type> '<literal>']` takes an integer or boolean
   literal that fits its width. Below a byte (`bit`, or `uint` or `int` of 1 to 7 bits) it is
   a member of a sub-byte run like a flag, and counts toward the eight bits, in either
   position; from a byte up it is an unsigned number of 8, 16, 32 or 64 bits. It is neither
   stored nor named in the output. Parse reads and ignores it, whatever it holds; serialize
   writes the literal. A run made only of reserved members is one `read_u8` and one
   `write_u8` of the constant. That is upstream's behavior: its generated code reads such a
   field with `readReservedField` and writes the expected value (`ViewAttributes.java:138,171`),
   and the runtime logs a value other than the expected one at info level and returns it
   (`FieldReaderReserved.java:33-42`, `FieldWriterReserved.java:33-42`). The generator
   accepts any value, as a client of a server that sets a reserved bit should. A literal of
   six digits or more is emitted with `_` separators, for `clippy::unreadable_literal`.
5. **External types and the recursion budget (G7).** `depth = "<path>"` and an `[external]`
   table (README, "External types and the recursion budget") are options. With `depth`, every
   generated `parse`, `serialize` and `length_in_bytes` of a struct and a `discriminatedType`
   takes the budget by value after the reader, the writer or the receiver, passes it on
   unchanged and names it `_depth` when it does not; enums and `dataIo` do not take it. An
   external type is a hand-written type with `parse(reader, depth)`, `serialize(&self, writer,
   depth)` and `length_in_bytes(&self, depth)`, each returning `Result<_, ParseError>`, which is
   what ADR-0034 decision 3 asks for with the length taking the budget too, as the length of a
   Variant recurses. The generator does not spend the budget. A cycle of generated types is
   a generation error as before, so every cycle that exists goes through an external type,
   the external type spends one unit on entry (`Depth::enter`, S86) and fails with
   `ParseError::RecursionLimit` at zero, and the stack depth of any parse, serialize or
   length is bounded by the budget times the longest chain of generated types between two
   externals, which the model fixes. The external's mspec parameters are not passed: an entry
   pins one literal per parameter, and a use with other arguments is a generation error
   (`ExtensionObject('true')` of OPC UA is pinned to `true`; a hand-written type for the
   masked and the unmasked envelope would be two types).
   The mspec type must exist (once) and be a `type` or a `discriminatedType`, is not also in
   `types`, and the table needs `depth`; a type that is neither generated nor listed is the
   generation error of `Ctx::def`, which now says so. The options, `depth` and the table
   included, are recorded in `generated/MANIFEST.toml` (`[options.external]`), and not at
   all when absent, so the Modbus manifest is unchanged. With any external in the table no
   generated type derives `Eq` (a Variant holds a float, and the derive of a type that holds
   one does not compile), and the externals must implement `Debug`, `Clone` and
   `PartialEq`. The name `depth` is reserved for fields and parameters only when the protocol
   has the `depth` option; without it a field may be called `depth`, as before.
6. **The enabled-case list.** The slice row asks for "an enabled-case list for a
   `typeSwitch`". The options' `[cases]` list is that, since S66, and is unchanged apart from
   item 2: a case named in it is a variant, and every other case is disabled and refused by
   the parser by name. A `typeSwitch` nested in a case (`ExtensionObject`, `Variant`) is not
   emitted; those types are hand-written (ADR-0034 decision 2).
7. **Fixtures and the tool version.** First-party mspec text, compiled with `rustc` next to
   the S77 stand-in, as in S77 and S78 (item 4 of the S77 addendum). The tool version stays
   0.2.0: the Modbus output is byte for byte what it was, and every input this addendum
   accepts was a generation error, so no `gen` commit follows (decision 7).

### Consequences
- The generator can emit `RequestHeader('391')`, the reserved bits of the model and the calls
  into the hand-written built-ins. What it still refuses in the model, by construct, is S80's
  list: among others the signed `int 32` discriminator of `ExtensionObjectDefinition`
  (`disc_literal` and the error value take unsigned discriminators only), `virtual` fields,
  nested `typeSwitch`, `optional`, `bit` arrays and `STATIC_CALL`.
- The module of a referenced case is generated even when the options never name it, so the
  set of generated modules is no longer only the options' `types`. A reader of the options
  sees the extra modules in `generated.rs`.
- The hand-written types of S85 and S86 implement one signature and one `Depth`; the
  generator treats `Depth` as opaque (`Copy`), so the choice of the representation, and the
  limit of 64 of ADR-0034, are theirs.
- Not done: a per-type byte order, an external type with a different signature per use, and
  externals for enums and `dataIo`.

### Compliance
- S79: `tools/stonecrab-codegen` (`src/emit.rs`: options, the case structs, the external
  checks, imports; `src/emit/fields.rs`: reserved fields, arguments, calls with the budget;
  `src/emit/switch.rs`, `src/emit/structs.rs`; `src/main.rs`, `src/output.rs`: the options
  file and the manifest) and its README. Tests
  `generator_case_parameters_receive_literal_arguments`,
  `generator_reserved_bits_share_a_byte_with_a_flag`,
  `generator_external_type_is_called_through_its_signature` and
  `generator_missing_external_type_is_an_error`, with the refusals named in the README and the
  options-file tests in `src/main.rs`. `mise run regen` passes; the Modbus generated files
  and manifest are unchanged.

## Addendum (S80, 2026-10-30): the dry run on the real OPC UA model, and recursion through external types
### Status
Proposed (2026-10-30), as slice S80 of docs/POST_ALPHA_PLAN.md (Phase H, M3 OPC UA): the
evidence for decision E14 and the amendment of decision 5 that the S79 addendum left to this
slice. The decisions and the addenda above stand. Nothing generated is committed and no
mspec is vendored (`git ls-files upstream/mspec` still shows only
`upstream/mspec/modbus/modbus.mspec`); every number below was produced in a scratch directory
outside the repository, and this text is where they are kept. Slice S88 accepts or revises this
addendum when it vendors the mspec, and S97 records E14 in `STATUS.md`.

### Context
S79 gave the generator the constructs of the connect-and-read subset. Before S88 commits to
generating the service layer of the hybrid codec (E14, ADR-0034 decision 2), S80 ran the grown
generator (0.2.0, as of slice S79) on the real model and measured what comes out.

Inputs. The pinned PLC4X v1.0.0 sparse checkout (commit `ea229918`), files under
`protocols/opcua/src/main/generated/protocols/opcua/`: `opc-manual.mspec` (3,813 lines, git blob
`a8cfc6fc`, sha256 `3c0a04d5daa72089df32316463d8421d8d9b9aee49d6eeaab991545ceaf937e4`),
`opc-status.mspec` (blob `ac49b8b2`, sha256 `f90c951f6ee5479863f84b8a96b92a4e59173eb06aefccd6091b4d32d07fad8e`)
and `opc-attribute.mspec` (blob `6cfa588e`, sha256 `c3235ae79bf27669f1a6583095dd0982c1d5faf36bef87dec81a5eaa4f908177`).
Copies sat in a scratch directory with a scratch `upstream/manifest.toml`; the generator's own
sha256 check ran. The options file set `byte_order = "LITTLE_ENDIAN"`, `depth`, and an
`[external]` table for the eleven types ADR-0034 keeps hand-written: `PascalString`,
`PascalByteString`, `NodeId`, `ExpandedNodeId`, `QualifiedName`, `LocalizedText`, `StatusCode`,
`DiagnosticInfo`, `DataValue`, `Variant`, and `ExtensionObject` pinned to `true`. The generated
type is `ExtensionObjectDefinition` (the one switch of 354 cases that carries every service
message), plus the enums the enabled cases name. Each external was a stub with the S79 signature
(`parse`, `serialize`, `length_in_bytes`, all taking the budget) that reads and writes nothing.

The scratch crate (edition 2024, rustc 1.99.1, Apple silicon macOS, no dependencies) held a copy
of `stonecrab::codec` (the shipped reader, writer and bit helpers, with the `CountLimit` and
`RecursionLimit` variants that S86 adds to `ParseError`), the support module of Modbus for the
bit-run helpers, and the stubs. Its lints were the workspace's (`clippy::pedantic`, the rust
lints of the root `Cargo.toml`) with `indexing_slicing`, `arithmetic_side_effects` and
`as_conversions` denied on the module, as `pub mod opcua` has them. Timings are the median of
three clean builds (the mean of two for release) with `CARGO_INCREMENTAL=0`; the larger builds
varied by up to a factor of two between runs, so the ranges are given. A public function that
calls `ExtensionObjectDefinition::parse` and `serialize` was added, so that a release build
generates code for every arm, as the S88 wiring of the ExtensionObject envelope will.

The three lists. The plan's figures (22, 56 and 72) are estimates; the lists below are derived
from the slice rows S88 to S123 and `docs/parity/opcua.md`, and the counts they give differ
from the estimates in two places, which is stated rather than adjusted. A case is in a list with
every case its fields name (a closure over the case references of `opc-manual.mspec`; the
enums are added to `types`, the externals are not cases).
1. **Connect and read, 22 cases** (S88): `RequestHeader`, `ResponseHeader`, `ServiceFault`,
   `ChannelSecurityToken`, `OpenSecureChannelRequest`, `OpenSecureChannelResponse`,
   `CloseSecureChannelRequest`, `ApplicationDescription`, `UserTokenPolicy`,
   `EndpointDescription`, `SignedSoftwareCertificate`, `SignatureData`, `CreateSessionRequest`,
   `CreateSessionResponse`, `AnonymousIdentityToken`, `ActivateSessionRequest`,
   `ActivateSessionResponse`, `CloseSessionRequest`, `CloseSessionResponse`, `ReadValueId`,
   `ReadRequest`, `ReadResponse`. Five enums: `ApplicationType`, `MessageSecurityMode`,
   `SecurityTokenRequestType`, `TimestampsToReturn`, `UserTokenType`. The closure adds nothing.
   (`CloseSecureChannelResponse` is not in it: Stonecrab expects no reply to a
   CloseSecureChannel, `OPCUA-LIFE-CLOSE-SESSION`.)
2. **Java-parity closure, 61 cases** (the plan says 56): the 22 above and 39 more. The
   requests and responses of the fourteen services of `OPCUA-WIRE-SERVICES-USED`, the three
   identity tokens (`OPCUA-AUTH-TYPES`), `DataChangeNotification` and `EventNotificationList`
   (the two notifications Java handles, `OPCUA-SUB-PUBLISH`), the event filter of S118 and the
   structure definition of S119, with the closure. Added: `GetEndpointsRequest`,
   `GetEndpointsResponse`, `CloseSecureChannelResponse`, `UserNameIdentityToken`,
   `X509IdentityToken`, `WriteRequest`, `WriteResponse`, `WriteValue`, `BrowseRequest`,
   `BrowseResponse`, `BrowseNextRequest`, `BrowseNextResponse`, `BrowseDescription`,
   `ViewDescription`, `ReferenceDescription`, `BrowseResult`, `CreateSubscriptionRequest`,
   `CreateSubscriptionResponse`, `CreateMonitoredItemsRequest`, `CreateMonitoredItemsResponse`,
   `MonitoringParameters`, `MonitoredItemCreateRequest`, `MonitoredItemCreateResult`,
   `PublishRequest`, `PublishResponse`, `NotificationMessage`, `SubscriptionAcknowledgement`,
   `DataChangeNotification`, `MonitoredItemNotification`, `EventNotificationList`,
   `EventFieldList`, `DeleteSubscriptionsRequest`, `DeleteSubscriptionsResponse`,
   `EventFilter`, `ContentFilter`, `ContentFilterElement`, `SimpleAttributeOperand`,
   `StructureDefinition`, `StructureField`. Five more enums: `BrowseDirection`,
   `FilterOperator`, `MonitoringMode`, `NodeClass`, `StructureType`. Without the unread
   `CloseSecureChannelResponse` it is 60. The plan's 56 does not follow from these rows by any
   rule S80 could find; the count is a measurement of the rule above, not a correction of the
   plan.
3. **Wish list, 78 cases** (the plan says 72): the 61 and 17 more. The six services that
   upstream builds nowhere and that `OPCUA-SUB-UNUSED-SERVICES` and E19 name (ModifySubscription,
   SetPublishingMode, Republish, TransferSubscriptions, ModifyMonitoredItems,
   DeleteMonitoredItems, each as request and response), `IssuedIdentityToken`
   (`OPCUA-AUTH-ISSUED`), `StatusChangeNotification` (S116) and, by closure,
   `MonitoredItemModifyRequest`, `MonitoredItemModifyResult` and `TransferResult`. No new enum.
A fourth run, for scale only: 352 cases, all of the model but the two that finding 6 names.

### Findings
1. **The generator as it stands completes none of the three generations.** Every one of the 354
   cases fails the same way, run one case at a time: `case value '310' does not fit its
   discriminator`. `ExtensionObjectDefinition(int 32 extensionId)` switches on a signed
   `int 32`, and `disc_literal` (`src/emit/switch.rs`) takes unsigned discriminators only; the
   S79 addendum listed it. After it, `error_value` refuses the same type ("the switch's error
   value is not emitted"). The scratch fix is one arm in each: an integer literal for `Ty::I32`,
   and `u64::from(x.unsigned_abs())` as the reported value, as the count error of S78 reports a
   negative count. This is the only refusal that the three lists meet.
2. **A miscompile that the Modbus proof could not reach.** With the first fix, the 22 cases
   generate and `cargo check` passes, and the 61 and the 78 do not: rustc E0308 (four errors,
   then eight). The enum's `serialize` merges the arms whose bodies are the same text so that
   `clippy::match_same_arms` stays quiet (`merge_arms`), and `BrowseResponse { results:
   Option<Vec<BrowseResult>>, .. } | ReadResponse { results: Option<Vec<DataValue>>, .. }`
   has the same body and different binding types. Modbus merges only variants of one shape.
   The scratch fix keys the merge on the stored fields' names and types as well as the body
   (three lines). With it all three lists generate, compile and pass the lints below. This is
   a defect of the existing generator, not a missing construct.
3. **Lints.** The generated modules draw one clippy warning, `too_many_lines`, on `parse`
   and on `serialize` of the enum (2,067 and 400 lines at 22 cases; the same two at 22, 61 and
   78 cases), and nothing else under `pedantic`, `indexing_slicing`, `arithmetic_side_effects`
   and `as_conversions`. The output needs either the per-case functions of finding 5 or an
   `#[expect(clippy::too_many_lines)]` the generator emits where the function is long.
4. **Size and time**, with the two fixes (one dispatcher function per direction, as the
   generator writes it today). Lines are `wc -l` of every `.rs` file after the repository's
   `rustfmt`, headers included. The delta is against the baseline crate with the codec and
   the stubs only.

   | | Cases | Enums | Files | Lines | Check | Dev build | Release build | rlib | Stripped exe |
   | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
   | Baseline | 0 | 0 | 0 | 0 | 0.31 s | 0.42 s | 0.51 s | 168 KB | 342,208 B |
   | 22 cases | 22 | 5 | 16 | 3,805 | 0.41 s (+0.10) | 0.56 s (+0.14) | 0.88 s (+0.37) | 547 KB | 408,800 B (+66,592) |
   | 61 cases | 61 | 10 | 37 | 8,150 | 0.59 s (+0.28) | 0.85 s (+0.43) | 1.54 s (+1.03) | 1,058 KB | 442,832 B (+100,624) |
   | 78 cases | 78 | 10 | 40 | 9,356 | 0.66 s (+0.35) | 1.78 s (+1.36, 0.94 to 2.61) | 2.42 s (+1.91, 1.96 to 2.88) | 1,184 KB | 443,040 B (+100,832) |
   | 352 cases | 352 | 35 | 131 | 35,635 | 2.04 s (+1.73) | 3.39 s (+2.97) | 10.41 s (+9.90, 8.12 to 12.69) | 4,386 KB | 661,616 B (+319,408) |

   The stripped-executable column is a file size, and on this target (arm64 Mach-O) the `__TEXT`
   segment is padded to 16 KiB pages, so every delta in it can be off by up to 16 KiB. The
   plateau from 61 to 78 cases (+100,624 against +100,832) is an artifact of that padding: the
   `__text` section of a rebuild of both models grew from 276,668 to 285,244 bytes (+8.6 KB of
   code) while `__TEXT` stayed at 376,832 bytes for both. Read the per-list binary figures as
   +66 to +101 KB within a page or two, not as exact.

   The 8,000 to 12,000 line estimate of the plan, which no measurement backed, is replaced by
   these: about 3,800 lines for the 22 cases, 8,150 for the Java-parity closure and 9,350 for
   the wish list. They sit in the estimate's lower half at 61 and 78 cases and below it at 22.
   Over a third of the 22-case output is not the 22 cases: every case of the switch that is
   not enabled is an arm that refuses it by name (4 lines each: 332, 293 and 276 arms, which is
   1,328, 1,172 and 1,104 lines of 3,805, 8,150 and 9,356), so `parse` carries the size of the
   model, not only of the enabled set. Per enabled case the marginal cost is about 115 lines
   from 22 to 61 cases and 75 from 61 to 78. The scratch crate has no dependency and the build
   time of the real crate was not measured here, so the deltas are absolute, not shares of a
   build. `OpcuaStatusCode` (274 entries) generates to 576 lines and `AttributeId` to 82, and
   both compile; the enum refuses a number it does not know, so the S88 test
   `opcua_unknown_status_code_keeps_its_number` needs the hand-written `StatusCode` to carry the
   number and this enum only for the name table.
5. **A stack finding that the line counts hide.** In a dev build (opt-level 0) every arm's
   locals live in the one frame of the dispatcher, so the frame grows with the number of
   cases: the `parse` of `ExtensionObjectDefinition` takes 45,056 bytes at 22 cases, 139,264
   at 78 and 700,416 at the 352-case model, and `serialize` 20,480, 77,824 and 393,216 (the
   stack allocation of the prologue, read from the assembly; the large ones are probed in
   whole 4 KiB pages, so each is a lower bound rounded to a page). In a release build the
   frame is 432, 1,136 and 4,096 bytes.
   The depth budget does not help if one hop already costs that. To test it the stub
   ExtensionObject spent one unit and re-entered the dispatcher; the input is a `ReadRequest`
   whose request header carries another `ReadRequest` as its additional header, 24 bytes a
   hop; it ran on a thread of a given stack size. At the 78 cases and the limit of 64 that
   ADR-0034 proposes:

   | Build | Stack | Smallest budget that overflows | Budget 64 |
   | --- | --- | --- | --- |
   | Dev | 2 MiB (test and Tokio worker threads) | 14 | crashes (SIGABRT) |
   | Dev | 8 MiB (main thread of Linux and macOS; Windows gives 1 MiB) | 57 | crashes (SIGABRT) |
   | Release | 2 MiB | 1,481 | `RecursionLimit` |
   | Release | 8 MiB | 5,899 | `RecursionLimit` |

   So a hostile or corrupted reply that nests 14 ExtensionObjects aborts a debug build, and
   a negative vector at the budget cannot even be run by `cargo test`. The scratch generator
   was then changed to emit the body of each case in its own function (a closure called in
   place, which costs a frame of its own at opt-level 0); the dispatcher frame stopped
   growing (the same page-rounded 4,096 bytes in a dev build at 22, 78 and 352 cases), and
   the table became 121 hops (2 MiB) and 483 (8 MiB) in a dev build and
   2,804 and 11,170 in a release build, so 64 hops use about 1.1 MiB in a dev build with the
   stubs: room, with a margin of about 1.9 on a 2 MiB thread, but not more, and the real
   externals add frames of their own (UNVERIFIED until S86 measures them). That variant is
   about 430 bytes of code per case larger: 9,358 lines and +133,968 bytes stripped at 78
   cases, +468,752 at the 352-case model, check 0.87 s and 2.19 s; a closure is not what the
   generator should emit (`clippy::redundant_closure_call`), a function is.
6. **Refused constructs, by construct.** In the lists: only finding 1. Outside them, running
   the generator on each of the types ADR-0034 keeps hand-written names why, and confirms the
   hybrid split rather than undermining it: `Variant` ("fields after the typeSwitch are not
   emitted yet"), `DataValue`, `DiagnosticInfo` and `LocalizedText` (`optional`), `PascalString`
   (`virtual`), `NodeId` and `NodeIdTypeDefinition` (`abstract`), `ExtensionObject` (`abstract`),
   `MessagePDU` and `Payload` (a `simple` field of the parent before the switch),
   `OpenChannelMessage` (a `typeSwitch` in a `type`), `XmlElement` (a `string 8` array), the
   enums `ChunkType`, `MessageType`, `OpcuaDataType` and `OpcuaIdentifierType` (string base
   types), `NodeIdType` (defined twice, the S77 rule) and `OpcuaConstants` (stores no field).
   `OpcuaProtocolLimits`, `SecurityHeader`, `SequenceHeader`, `ByteStringArray`, `GuidValue`,
   `PascalByteString`, `QualifiedName` and the `StatusCode` type do generate and are not
   needed. In the model itself, two of the 354 cases are refused after finding 1:
   `CertificateGroupDataType`, whose `isCertificateAssigned` is an `array bit` after a
   `reserved uint 7` ("a run of sub-byte fields does not end on a byte boundary"), and
   `ApplicationConfigurationDataType`, which holds it. Neither is in any list.
7. **A round trip, as a smoke test only.** With the stubs, a `ReadRequest` vector of two
   nodes parses and serializes byte for byte; a node count of 1,000,000 over 8 bytes is
   `CountLimit` before any reservation; a disabled case, an unknown id and a truncated input
   are named errors. That proves the shape, not the OPC UA bytes: the stubs read nothing, and
   the vectors of S88 are the proof.

### Decision
1. **E14.** The numbers confirm option 1, the hybrid, and do not point to option 2, on one
   condition. On size and time they are small: 3,800 to 9,400 lines for the three lists,
   +0.1 to +0.35 s of `cargo check`, +66 to +101 KB of stripped executable in a release build,
   and the whole model minus two cases is 35,600 lines and +1.7 s. The condition of E14
   ("2 is the fallback if S80 shows constructs still refused") is met in the letter by the
   generator of S79, which completes none of the generations, so it is stated plainly: the
   refusals are one construct (the signed `int 32` switch), one defect (the arm merge) and
   one structural gap (the frame of the dispatcher, finding 5), each fixed in the scratch copy
   with a handful of lines (ten changed lines for the first two) and none a new construct of
   the model. Option 1 stands if a generator slice makes the three changes of decision 3
   before S88; if it does not land, S88 takes option 2 (the same structures hand-written in
   `src/opcua/service`, the generator kept as the `cfg(test)` cross-check). The plan has no
   such slice, and this addendum does not add it to the table.
2. **Amendment of decision 5, recursion through external types.** The last sentences of
   decision 5 ("A recursive type is a generation error until the first protocol that has one
   adds a depth argument and `ParseError::RecursionLimit`") are replaced by these rules, which
   are what OPC UA is the first protocol to need:
   - A cycle made of generated types alone is a generation error, as before.
   - A cycle that passes through an `[external]` type is allowed when the options set `depth`
     (S79). The generator passes the budget unchanged and never spends it. It cannot see into
     an external, so the bound is a property of the generator and the external together.
   - Every external that can reach generated code again (`ExtensionObject`, `Variant` and
     `DataValue` in OPC UA) spends one unit on entry, before reading, and fails with
     `ParseError::RecursionLimit` at zero. An external that only recurses into itself inside
     hand-written code (`DiagnosticInfo`, through its `innerDiagnosticInfo`, which holds no
     generated type) spends the same unit for the bound on its own depth (S86). Each has a
     negative vector nested to one level past the budget in the same module, and the
     Known-body dispatch has a fuzz target (E14); a missing spend is not caught at compile
     time.
   - The stack bound is the budget times the longest chain of generated frames between two
     external re-entries, and a generated dispatch function must not grow with the number of
     cases: the body of each case of a `discriminatedType` is a function of its own and the
     `match` only calls it (finding 5). The chain in the lists is three generated types (a
     variant, then up to two structs: `CreateSessionResponse`, `EndpointDescription`,
     `ApplicationDescription`) and eight in the whole model. A test nests one level past
     the budget on a 2 MiB thread in a dev build and expects `RecursionLimit`.
   - Allocation stays bounded as in decision 5: a count is checked against the remaining input
     before any reservation (`CountLimit`, S78), and the budget bounds depth, not length.
3. **The generator changes S80 recommends before S88** (a slice of its own, S size, tool
   0.2.0 or 0.3.0 by decision 7: the first two change no output for Modbus; the third changes
   it unless it is limited to large switches, and a change to existing output bumps the
   version and is committed as a following `gen` commit): (a) a
   signed `int 32` (and `int 8`, `int 16`, `int 64`) discriminator and its error value (the
   scratch fix reports `u64::from(x.unsigned_abs())`, which drops the sign as the count error of
   S78 does, so an id of -310 reads as the value 310, the number of a real case; the generator
   slice should pick a representation that keeps the sign for a discriminator); (b)
   the arm merge compares the bound fields' names and types; (c) one function per case for
   `parse` and `serialize` of a `discriminatedType`, with the lint expectation or the
   function split that satisfies `too_many_lines`. Proof: first-party synthetic fixtures in
   the S77 pattern (a signed switch, two variants of one body with different element types, a
   dispatcher with 300 cases nested through a stub external to the budget on a small stack),
   and a rerun of this dry run, whose steps are listed in the Compliance section.
4. **What this addendum does not decide.** Whether to compress the disabled arms (a range
   or a single or-pattern would cut 1,100 to 1,300 lines but lose the case name in the error);
   the value of the limit (ADR-0034 says 64 unless S86 measures a reason, and the measurement
   above is the reason to measure again with the real externals); the E13 vendoring.

### Consequences
- E14 is confirmed for option 1 with a precondition, not unconditionally. S88 cannot start
  from the generator as it is: it would generate nothing.
- The generated layer is small and cheap to build; the cost of option 1 is the review of a
  generator change and of regenerated output, not compile time or binary size.
- Everything in finding 5 is a debug-build hazard that a release-only test run would miss, so
  the stack test must run in the dev profile.
- The figures are one machine, one toolchain and the stubs. The Linux targets of the CI matrix
  (x86_64, aarch64, armv7) and Windows were not measured; frame sizes and the hops-to-overflow
  differ by target, and the real externals change the per-hop cost. The numbers in this
  addendum are UNVERIFIED for those until S86 and S88 measure them where the gates run.

### Compliance
- S80 changed no file of the tool or the crate and added no vendored file. The scratch
  directory (`round-m3c/s80/` of the session scratchpad) held: a copy of the three mspec files
  and a scratch manifest, the options files of the three lists and of the whole model, the
  generator built twice from a scratch copy of `tools/stonecrab-codegen` (the fixes of
  decision 3 (a) and (b), then the closure experiment of finding 5), the scratch crate and
  its benchmark scripts. It was not committed; the numbers above are its record.
- To rerun: copy the three files and the S88 manifest layout to a scratch root, generate each
  list with `stonecrab-codegen generate`, copy the output beside the codec stand-in, and
  time `cargo check`, `cargo build` and `cargo build --release` from clean; take the
  frame size from `cargo rustc --lib -- --emit asm -C codegen-units=1` and the overflow
  point from a bisection on the budget in a thread of 2 and of 8 MiB.

## Addendum (S80a, 2026-10-30): the three generator changes of the S80 dry run
### Status
Accepted (2026-10-30), as slice S80a of docs/POST_ALPHA_PLAN.md (Phase H, M3 OPC UA). It makes
the three changes that decision 3 of the S80 addendum asks for before S88, and nothing else:
the decisions and the addenda above stand. No mspec is vendored and nothing generated is
committed; the figures below come from a scratch crate outside the repository.

### Decision
1. **A signed discriminator of every width (3a).** A `typeSwitch` may be over `int 8`,
   `int 16`, `int 32` or `int 64`, as a discriminator field or as a parameter. A case value is
   an integer literal that fits the width, negative ones included (`'-310'` is a negation of
   the literal 310 in the expression grammar); one that does not fit, or is not an integer, is
   a generation error naming the value. The error value of `InvalidDiscriminator` keeps the
   sign, which the scratch fix of S80 (`unsigned_abs`) lost: it is the number widened to
   `i64` and reinterpreted as `u64` (`i64::from(id).cast_unsigned()`, `id.cast_unsigned()` for
   `int 64`; no `as`), so an id of -310 is `0xFFFF_FFFF_FFFF_FECA` and not 310, the number of a
   real case. The field stays a `u64`; a consumer that wants the number back reads it as an
   `i64`.
2. **The arm merge compares what the arms bind (3b).** `serialize` and `length_in_bytes` of a
   `discriminatedType` still merge arms with the same body so that `match_same_arms` stays
   quiet, but only arms that bind the same field names of the same Rust types (the bound
   fields, after the unused ones of a length arm are dropped). `ReadResponse { results:
   Vec<DataValue> }` and `BrowseResponse { results: Vec<BrowseResult> }` are two arms. Enums
   merge as before (they bind nothing).
3. **A function per case above 16 enabled cases (3c).** A `discriminatedType` with more than
   16 enabled cases emits `parse_<case>` and `serialize_<case>` for each case that reads or
   writes something; `parse` and `serialize` become a `match` that calls them. The call
   passes the reader or writer, the budget when the case body uses it, and, for `parse`, the
   parameters and the fields read before the switch that the case uses or stores (the
   case's own list, in name order). `serialize_<case>` of a case that stores fields takes
   `&self` and binds them with `let Self::<Case> { .. } = self else { return Err(..) }`: the
   `match` never sends another variant, and the branch is an `InvalidDiscriminator` naming
   the case rather than a panic. The alternative, passing the fields as references, trips
   `ptr_arg`, `ref_option` and `trivially_copy_pass_by_ref` depending on their types. The
   limit is 16 because Modbus has nine and a small switch needs no split; it is a constant of
   the generator (`CASE_FUNCTIONS_ABOVE`). `length_in_bytes` stays one `match`: its arms are
   expressions and the S80 lists generate none (nothing asks for the length of
   `ExtensionObjectDefinition`); S88 measures it if its wiring does.
4. **The `too_many_lines` expectation is not emitted.** The dispatcher of a large switch
   keeps one arm for each disabled case, 4 lines each, so its `parse` is over the 120 lines of
   the repository's `clippy.toml` however the enabled cases are written, and a case with
   many fields (`PublishResponse`) is over it on its own. An expectation that the generator
   adds after `rustfmt` for a body over 120 lines was written and removed again: it changed
   `modbus_pdu.rs` (its `parse` is 225 lines, in a module that is `cfg(test)`, where the lint
   does not fire), and this slice leaves the Modbus output as it is. The module that holds
   the OPC UA output (S88) carries `#[expect(clippy::too_many_lines, reason = ...)]` on
   `mod generated`, which is fulfilled by the dispatcher in every list.
5. **The tool version stays 0.2.0.** Decision 7 bumps it when the output changes for an
   unchanged input. The Modbus output is byte for byte what S79 left (`mise run regen`
   twice identical and the tree unchanged), the signed switch was a generation error, and the
   merge key and the split change no text a switch of 16 cases or fewer, with matching
   bindings, had. Hence no `gen` commit follows.
6. **Fixtures.** `src/emit/switch_fixtures.rs`, first-party mspec compiled with `rustc` (a
   debug build) next to the stand-in of S77: `generator_signed_int32_discriminator_selects_its_case`
   (four widths, field and parameter, signs, the disabled and unknown negatives),
   `generator_signed_discriminator_literals_are_checked_against_the_width`,
   `generator_arms_with_equal_names_and_different_types_are_not_merged`,
   `generator_large_switch_gets_a_function_per_case` (17 split, 16 do not) and
   `generator_dispatch_frame_does_not_grow_with_case_count` (300 cases nested through an
   external to a budget of 40 on 1 MiB). With the cases inline the last overflows the stack;
   with a function per case it runs on 128 KiB and fails on 64 KiB.

### Rerun of the S80 dry run
The three lists of the S80 addendum (22, 61 and 78 cases, and the whole model but two cases
for scale) were generated again from the same pinned `opc-manual.mspec` with the same
options, the externals as the stubs of S80, into a scratch crate of the same shape. The
tool was the one of this slice, edition 2024, rustc 1.99.1, Apple silicon macOS.

| | Cases | Enums | Files | Lines (S80) | Lines (S80a) | `cargo check` | clippy, workspace lints |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 22 cases | 22 | 5 | 16 | 3,805 | 4,217 (+11%) | passes | `too_many_lines`: 1 (the `parse` dispatcher, 1,356 lines) |
| 61 cases | 61 | 10 | 37 | 8,150 | 9,348 (+15%) | passes | `too_many_lines`: 3 (dispatcher 1,239, two functions of 123 and 132 lines) |
| 78 cases | 78 | 10 | 40 | 9,356 | 10,939 (+17%) | passes | `too_many_lines`: 3 (dispatcher 1,188, `serialize` 166, `parse_publish_response` 132) |
| 352 cases | 352 | 35 | 131 | 35,635 | 41,863 (+17%) | passes | `too_many_lines`: 30 (the dispatcher among them), nothing else |

Lines are `wc -l` of every `.rs` file after the repository's `rustfmt`, headers included. The
clippy column is `cargo clippy` under `pedantic` with the root's lint table, `indexing_slicing`,
`arithmetic_side_effects` and `as_conversions` denied on the module and the repository's
`clippy.toml`; no other lint fires, and `rustfmt --check` finds the output formatted. The
growth is the price of a signature and a call for each case (about 19 lines more for each).

The dispatcher frame no longer grows with the cases. In a dev build the stack allocation of
the prologue of `ExtensionObjectDefinition::parse` is 4,096 bytes (page-rounded, a lower
bound) at 22, 78 and 352 cases, against 45,056, 139,264 and 700,416; `serialize` is 64 bytes
against 20,480, 77,824 and 393,216. The largest case function is also 4,096 bytes.

The hop test of S80 (a `ReadRequest` whose additional header holds another, a stub
ExtensionObject that spends one unit and re-enters, a budget bisected on a thread of a
given stack) gave, as the smallest budget that overflows:

| Build | Stack | 22 cases | 78 cases (S80: before) | 352 cases |
| --- | --- | --- | --- | --- |
| Dev | 2 MiB | 117 | **130** (14) | 111 |
| Dev | 8 MiB | 466 | **521** (57) | 446 |
| Release | 2 MiB | 3,379 | 2,804 (1,481) | 1,647 |
| Release | 8 MiB | 13,462 | 11,170 (5,899) | 6,563 |

A budget of 64 is `RecursionLimit` in all twelve. In a dev build 64 hops use about half of a
2 MiB thread (a hop costs about 16 KiB), a margin of 2.0, as the closure experiment of S80
estimated (121 and 483); the real externals add frames of their own, so the figure is still
UNVERIFIED until S86 and S88 measure them. One machine, one toolchain, the stubs, as in S80.

### Consequences
- S88 can generate the connect-and-read subset with option 1 of E14: the generator completes
  all three lists, the output compiles, and a hostile chain of 64 ExtensionObjects no longer
  aborts a debug build on a 2 MiB thread. S88 owns the `too_many_lines` expectation on the
  module and the stack test through the real externals.
- The output is 11 to 17% longer than S80 reported, for 22 to 352 cases.
- The disabled arms are not compressed (S80 decision 4 stands); the split does not remove
  them from the dispatcher.
- A case that stores no field and reads nothing stays an arm of the `match`
  (`Ok(Self::Empty)`), as it has no locals.

### Compliance
- S80a: `tools/stonecrab-codegen` (`src/emit/switch.rs`: the signed literal, the error value,
  the case functions and the bound key; `src/emit.rs`: `merge_bound_arms`;
  `src/emit/fixtures.rs`: the stand-in reads and writes `int 8`, `int 16` and `int 64`;
  `src/emit/switch_fixtures.rs`) and its README (semantics, matrix, fixtures). `mise run regen`
  passes and the Modbus generated files are unchanged. The scratch directory (`round-m3d/s80a/`
  of the session scratchpad) held the S80 scratch tooling (the options of the three lists, the
  stub externals, the hop test) and was not committed.
