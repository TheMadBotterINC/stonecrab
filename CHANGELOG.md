# Changelog
## Unreleased
### stonecrab 0.2.0
- Move the facade into a resolver-3 workspace with a runtime-independent core crate.
- Remove the specification-only `placeholder::status` API. There is no replacement
  status API; consult the capability and parity documentation.
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) and stop shipping `tests/` in
  the crate archive.
- Re-export `ByteOrder` and `WordOrder` from `stonecrab-core`.
### stonecrab-codec 0.1.0
- Add the crate: `BoundedReader` and `BoundedWriter` with bounds-checked integer, float,
  byte-run and least-significant-bit-first coil reads and writes in an explicit
  `ByteOrder`, and `ParseError` (`Truncated`, `OversizedLength`, `InvalidDiscriminator`,
  `TrailingBytes`, `Overflow`) with `From<ParseError> for PlcError`.
- Add `coil_byte_count`, `pack_bits_lsb_first` and `unpack_bits_lsb_first`.
- Lengths are checked against the caller's maximum and the remaining input before any
  allocation or copy; no function panics on any input (ADR-0013).
### stonecrab-core 0.1.0
- Add docs.rs metadata (`all-features`, `--cfg docsrs`) for feature-gated item docs.
- Establish shared metadata, safety lints and foundation contracts.
- Add exact-width scalar values, validated tag identifiers, monotonic deadlines,
  immutable finite resource budgets, explicit capabilities, typed diagnostic
  failures and redacted secret formatting.
- Add protocol-owned tags, bounded read builders, exact batch correlation,
  preserved per-item protocol status and explicit unknown-after-transmit outcomes.
- Add `ByteOrder` and `WordOrder`, optional `Tag::with_byte_order`/`with_word_order` with
  `byte_order()`/`word_order()` accessors, and additive `PartialOrd, Ord` on `ErrorKind`,
  `Operation` and `PlcValueType` (ADR-0006).
- Add an unpublished deterministic address fixture testkit for contract tests.
## stonecrab 0.1.0
- Publish the specification-stage package. No industrial protocol support.
