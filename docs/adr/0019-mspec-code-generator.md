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
