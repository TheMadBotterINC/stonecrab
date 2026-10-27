# 0006: Value ordering, byte and word order, and the RawValue deferral
## Status
Accepted by the sole maintainer on 2026-10-27, as slice S05 of docs/ALPHA_PLAN.md.
It records decisions D13 (the ordering half) and D14 and satisfies the "ADR before M1"
requirement of ARCHITECTURE.md section 11 for the deferred value representations. It
affects `stonecrab-core` and the facade re-exports; every change is additive.

## Context
Later slices need `BTreeSet<ErrorKind>` for the retry policy (ADR-0008), a way to say how
a multi-word value is laid out on the wire, and a decision on the architecture's
`RawValue` and calendar values. Upstream PLC4X models layout with one four-valued
`ModbusByteOrder` (`docs/parity/modbus.md`, section "Byte order"). The core denies
`clippy::as_conversions` and `unwrap`, `expect`, `panic` and indexing, so a bitset
keyed by an enum discriminant is not expressible without a cast.

## Decision
1. **Ordering derives (D13).** Add `PartialOrd, Ord` to `ErrorKind`, `Operation` and
   `PlcValueType`. The order is declaration order, which is stable for these
   `#[non_exhaustive]` enums only because new variants are appended; it carries no
   protocol meaning. `ErrorKind::Unsupported(Operation)` orders by variant and then by
   `Operation`, which is why `Operation` needs the derive too. All three stay `Copy`,
   `Eq` and `Hash`. `PlcValue` is unchanged: it holds floats and is not `Ord`.
   `RetryPolicy.retry_on` is a `BTreeSet<ErrorKind>`, default
   `{Timeout, Disconnected, Backpressure}`, equal to `is_retryable()`.
2. **Byte and word order (D14).** Core exports `ByteOrder {BigEndian, LittleEndian}` and
   `WordOrder {HighWordFirst, LowWordFirst}`, both `Copy`, `Hash`, `Ord` and
   `#[non_exhaustive]`, and `Tag::with_byte_order`, `Tag::with_word_order` with
   `byte_order()` and `word_order()` accessors returning `Option`. The builders return
   `Self` (unlike the fallible `Tag::with_type`) because core has no address rule to
   contradict; a driver validates conflicts with its own address syntax.
3. **Precedence, never inference.** The tag field wins, then the byte-order block of the
   protocol address, then the driver default (`default-payload-byte-order`, BIG_ENDIAN
   for Modbus). An absent setting stays `None`; no order is guessed from plausible-looking
   values (SPEC design commitment, PROJECT_BRIEF "No inferred byte order").
4. **Upstream mapping.** The four `ModbusByteOrder` names map as derived from the
   documented 4-byte layouts:

   | Upstream mode | Layout | `ByteOrder` | `WordOrder` |
   | --- | --- | --- | --- |
   | `BIG_ENDIAN` | `[1,2,3,4]` | `BigEndian` | `HighWordFirst` |
   | `LITTLE_ENDIAN` | `[4,3,2,1]` | `LittleEndian` | `LowWordFirst` |
   | `BIG_ENDIAN_BYTE_SWAP` | `[2,1,4,3]` | `LittleEndian` | `HighWordFirst` |
   | `LITTLE_ENDIAN_BYTE_SWAP` | `[3,4,1,2]` | `BigEndian` | `LowWordFirst` |

   The parity rows `MB-BYTEORDER-BE`, `-LE`, `-BE-SWAP`, `-LE-SWAP` and `-PRECEDENCE`
   remain `planned` in `docs/parity/modbus.md` in this slice. They move to
   `implemented` with the S23 codec, and to `automated-verified` only when the upstream
   little-endian all-datatypes vectors pass. The types in this slice prove only the data
   model, not wire behavior. `Tag` intentionally has no `PartialEq`, so tests compare
   the new fields through accessors and `Debug` text; a later slice may revisit this.
5. **`RawValue` is deferred.** There is no `PlcValue::Raw`, `Bytes` or `List` variant.
   The substitute is twofold. Sensor readings keep the unscaled typed value:
   `Measurement.raw` in `stonecrab-sensors` is a `PlcValue`, not a byte buffer. When a
   typed decode fails, the received register bytes travel on the item's `ItemError`
   through `ProtocolStatus::with_raw(&[u8], NonZeroUsize)` in `response.rs`, which
   rejects over-budget input with `ErrorKind::ResourceLimit` before copying and keeps the
   bytes out of Debug output (ADR-0003).
6. **Calendar, bytes and list values stay deferred.** `PlcValue::Date/Time/DateTime`,
   byte-buffer and recursive collection values keep the status given in ADR-0002. No
   calendar crate enters `stonecrab-core`. A later ADR chooses the representation
   before any milestone needs one; until then nothing is converted to text or floats.

## Consequences
- `retry_on` can be a `BTreeSet` with no new dependency and no unchecked conversion.
- Adding a variant before the end of one of the three enums would silently change the
  order; review treats variant reordering as a breaking change.
- Drivers receive the tag-level override through the existing `Tag`; they own the
  address block and default tiers of the precedence rule.
- The facade grows by two re-exports, so `stonecrab` 0.2.0 records them.
- Typed-decode failures keep diagnostics bounded and redacted without a new value type.

## Alternatives considered
- A fixed-size bitset for the retry set: rejected, indexing it needs a discriminant cast
  and core denies `as_conversions`.
- `HashSet<ErrorKind>`: rejected, iteration order is not deterministic and tests may not
  depend on it (CODING_STANDARDS 13.3).
- Four-valued core enum mirroring `ModbusByteOrder`: rejected, it bakes one protocol's
  vocabulary into core and hides the independent byte and word axes other protocols use.
- Inferring order from value plausibility: rejected, it contradicts the design commitment.
- `PlcValue::Raw(Bytes)` now: rejected, it needs a budget design the alpha does not need.

## Compliance
Unit tests in `order.rs`, the three enum modules and `tests/tag_order.rs` assert
declaration-order sorting, `BTreeSet` round trips, `Copy` and `Hash`, `Option` defaults
and that each builder sets exactly one field. `crates/stonecrab/tests/public_contract.rs`
asserts `Send + Sync + 'static` and the facade re-exports. `scripts/coverage-gate.sh`
holds core at 100% lines.
