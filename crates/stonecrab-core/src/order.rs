//! Byte and word order of multi-byte values, kept as two explicit axes.

/// Order of the bytes inside one protocol word.
///
/// Together with [`WordOrder`] this describes how a multi-word value is laid out on
/// the wire. The order is never inferred from plausible-looking values: it comes from
/// the caller or the protocol profile, and an absent setting stays absent.
///
/// Precedence, applied by the driver that reads the settings (decision D14):
/// 1. the [`Tag`](crate::Tag) field set with [`Tag::with_byte_order`](crate::Tag::with_byte_order);
/// 2. the byte-order setting inside the protocol address block, if the address has one;
/// 3. the driver's configured default.
///
/// The variants are ordered by declaration, which is a stable total order for use in
/// sorted collections; it carries no protocol meaning. The enum is exhaustive: a word
/// has exactly two byte orders, so consumers match both variants without a wildcard
/// arm (ADR-0006 declares the shape final under `CODING_STANDARDS` 9.2).
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-BE
/// Upstream PLC4X has a single four-valued `ModbusByteOrder`. Stonecrab splits it into
/// this type and [`WordOrder`] (rows `MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`,
/// `MB-BYTEORDER-BE-SWAP`, `MB-BYTEORDER-LE-SWAP` and `MB-BYTEORDER-PRECEDENCE` in
/// `docs/parity/modbus.md`):
///
/// | Upstream mode | 4-byte layout | `ByteOrder` | [`WordOrder`] |
/// | --- | --- | --- | --- |
/// | `BIG_ENDIAN` | `[1,2,3,4]` | `BigEndian` | `HighWordFirst` |
/// | `LITTLE_ENDIAN` | `[4,3,2,1]` | `LittleEndian` | `LowWordFirst` |
/// | `BIG_ENDIAN_BYTE_SWAP` | `[2,1,4,3]` | `LittleEndian` | `HighWordFirst` |
/// | `LITTLE_ENDIAN_BYTE_SWAP` | `[3,4,1,2]` | `BigEndian` | `LowWordFirst` |
///
/// The core-side model (two independent axes, optional per-tag fields, never inferred)
/// is recorded in `docs/parity/core.md#CORE-BYTEORDER-MODEL`.
///
/// # Examples
/// ```
/// use stonecrab_core::ByteOrder;
/// let order = ByteOrder::LittleEndian;
/// assert_ne!(order, ByteOrder::BigEndian);
/// assert!(ByteOrder::BigEndian < ByteOrder::LittleEndian);
/// let copy = order;
/// assert_eq!(copy, order, "Copy: the original is still usable");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ByteOrder {
    /// The most significant byte of a word comes first.
    BigEndian,
    /// The least significant byte of a word comes first.
    LittleEndian,
}

/// Order of the 16-bit words inside a multi-word value.
///
/// Independent of [`ByteOrder`], which governs the bytes within each word. The order is
/// never inferred; it follows the same precedence as [`ByteOrder`] (tag field, then
/// address block, then driver default, per decision D14).
///
/// The variants are ordered by declaration, a stable total order with no protocol
/// meaning. The enum is exhaustive for the same reason as [`ByteOrder`]: a value has
/// exactly two word orders.
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-LE
/// Upstream has no separate word-order setting; the four-mode mapping is the table on
/// [`ByteOrder`] (rows `MB-BYTEORDER-BE`, `MB-BYTEORDER-LE`, `MB-BYTEORDER-BE-SWAP`,
/// `MB-BYTEORDER-LE-SWAP` in `docs/parity/modbus.md`). `HighWordFirst` pairs with
/// `BIG_ENDIAN` and `BIG_ENDIAN_BYTE_SWAP`; `LowWordFirst` pairs with `LITTLE_ENDIAN`
/// and `LITTLE_ENDIAN_BYTE_SWAP`.
///
/// # Examples
/// ```
/// use stonecrab_core::WordOrder;
/// let order = WordOrder::LowWordFirst;
/// assert_ne!(order, WordOrder::HighWordFirst);
/// assert!(WordOrder::HighWordFirst < WordOrder::LowWordFirst);
/// let copy = order;
/// assert_eq!(copy, order, "Copy: the original is still usable");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WordOrder {
    /// The most significant word of a value comes first.
    HighWordFirst,
    /// The least significant word of a value comes first.
    LowWordFirst,
}

#[cfg(test)]
mod tests {
    use std::cmp::Ordering;
    use std::collections::{BTreeSet, HashSet};

    use super::*;

    fn duplicate<T: Clone>(value: &T) -> T {
        value.clone()
    }

    #[test]
    fn byte_order_is_total_and_follows_declaration_order() {
        let ordered = [ByteOrder::BigEndian, ByteOrder::LittleEndian];
        for (i, left) in ordered.iter().enumerate() {
            for (j, right) in ordered.iter().enumerate() {
                assert_eq!(left.cmp(right), i.cmp(&j), "cmp follows declaration");
                assert_eq!(left.partial_cmp(right), Some(i.cmp(&j)), "partial agrees");
            }
        }
        assert_eq!(
            ByteOrder::BigEndian.cmp(&ByteOrder::LittleEndian),
            Ordering::Less
        );
        let sorted: BTreeSet<_> = [ByteOrder::LittleEndian, ByteOrder::BigEndian]
            .into_iter()
            .collect();
        assert_eq!(sorted.into_iter().collect::<Vec<_>>(), ordered);
    }

    #[test]
    fn word_order_is_total_and_follows_declaration_order() {
        let ordered = [WordOrder::HighWordFirst, WordOrder::LowWordFirst];
        for (i, left) in ordered.iter().enumerate() {
            for (j, right) in ordered.iter().enumerate() {
                assert_eq!(left.cmp(right), i.cmp(&j), "cmp follows declaration");
                assert_eq!(left.partial_cmp(right), Some(i.cmp(&j)), "partial agrees");
            }
        }
        let sorted: BTreeSet<_> = [WordOrder::LowWordFirst, WordOrder::HighWordFirst]
            .into_iter()
            .collect();
        assert_eq!(sorted.into_iter().collect::<Vec<_>>(), ordered);
    }

    #[test]
    fn orders_are_copy_hash_and_clone_equal() {
        let byte = ByteOrder::LittleEndian;
        let moved = byte;
        assert_eq!(byte, moved, "Copy leaves the source usable");
        let cloned = duplicate(&byte);
        assert_eq!(cloned, byte);
        let bytes: HashSet<_> = [
            ByteOrder::BigEndian,
            ByteOrder::LittleEndian,
            ByteOrder::BigEndian,
        ]
        .into_iter()
        .collect();
        assert_eq!(bytes.len(), 2, "hash agrees with equality");

        let word = WordOrder::LowWordFirst;
        let moved = word;
        assert_eq!(word, moved, "Copy leaves the source usable");
        let cloned = duplicate(&word);
        assert_eq!(cloned, word);
        let words: HashSet<_> = [
            WordOrder::HighWordFirst,
            WordOrder::LowWordFirst,
            WordOrder::LowWordFirst,
        ]
        .into_iter()
        .collect();
        assert_eq!(words.len(), 2, "hash agrees with equality");
    }

    #[test]
    fn debug_text_names_the_variant() {
        assert_eq!(format!("{:?}", ByteOrder::BigEndian), "BigEndian");
        assert_eq!(format!("{:?}", ByteOrder::LittleEndian), "LittleEndian");
        assert_eq!(format!("{:?}", WordOrder::HighWordFirst), "HighWordFirst");
        assert_eq!(format!("{:?}", WordOrder::LowWordFirst), "LowWordFirst");
    }
}
