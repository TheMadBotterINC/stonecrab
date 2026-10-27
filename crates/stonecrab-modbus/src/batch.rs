//! The read batcher: turns a list of single-element tags into the smallest set of
//! contiguous wire reads that respects the Modbus quantity limits.
//!
//! [`Batcher::plan`] is pure: it performs no I/O, reads no clock and allocates only the
//! plan. Tags are grouped by (unit identifier, area), sorted by wire offset and merged
//! into one [`Chunk`] while their spans are adjacent or overlap. A one-register gap always
//! starts a new chunk, so a read never covers an address the caller did not ask for. A
//! chunk never exceeds the register or coil limit of [`BatchLimits`] and never holds more
//! than `max_items` tags. The connection (S25) sends one request per chunk and uses each
//! [`Member`] to decode its tag from the chunk's registers or bits.

use std::any::Any;
use std::num::{NonZeroU16, NonZeroUsize};

use stonecrab_core::{ByteOrder, ErrorKind, PlcError, Result, Tag, TagId, WordOrder};

use crate::{ModbusAddress, ModbusArea, ModbusDataType, UnitId};

/// Most registers one read request may carry (function codes 03 and 04, V1.1b section 6.3).
const PROTOCOL_MAX_REGISTERS: u16 = crate::MAX_READ_REGISTERS;

/// Most bits one read request may carry (function codes 01 and 02, V1.1b section 6.1).
const PROTOCOL_MAX_COILS: u16 = crate::MAX_READ_BITS;

/// Default for [`BatchLimits::max_items`], the core `ResourceLimits` default.
const DEFAULT_MAX_ITEMS: usize = 1_024;

/// Bounds a plan must respect: the largest chunk per area kind and the item bound.
///
/// `max_registers` and `max_coils` come from the driver configuration and can only be
/// lowered below the protocol maxima of 125 registers and 2000 bits, never raised above
/// them. `max_items` is `Capabilities.max_items_per_request` and bounds both the number of
/// tags in a plan and the number of members in a chunk. The default is 125, 2000 and 1,024
/// (the core `ResourceLimits` default).
///
/// # Examples
/// ```
/// use std::num::{NonZeroU16, NonZeroUsize};
/// use stonecrab_modbus::BatchLimits;
///
/// let defaults = BatchLimits::default();
/// assert_eq!(defaults.max_registers().get(), 125);
/// assert_eq!(defaults.max_coils().get(), 2000);
///
/// let small = BatchLimits::new(
///     NonZeroU16::new(10).expect("non-zero"),
///     NonZeroU16::new(64).expect("non-zero"),
///     NonZeroUsize::new(32).expect("non-zero"),
/// )?;
/// assert_eq!(small.max_registers().get(), 10);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-LIMIT-REGISTERS
/// The 125-register bound is `MB-LIMIT-REGISTERS` and the 2000-bit bound is
/// `MB-LIMIT-COILS` (docs/parity/modbus.md#MB-LIMIT-COILS). Upstream makes both
/// configurable without an upper check; Stonecrab refuses a configured value above the
/// Modbus Application Protocol V1.1b maximum, because such a request is invalid on the wire.
///
/// `max_items` bounds the whole plan, not a single chunk. With the default of 1024, a plan
/// of more than 1024 coils is refused with `ResourceLimit`; raise `max_items`, for example
/// to `Capabilities.max_items_per_request`, to reach the 2000-coil chunk limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct BatchLimits {
    registers: NonZeroU16,
    coils: NonZeroU16,
    items: NonZeroUsize,
}

impl BatchLimits {
    /// Validate and retain the three bounds.
    ///
    /// # Examples
    /// ```
    /// use std::num::{NonZeroU16, NonZeroUsize};
    /// use stonecrab_modbus::BatchLimits;
    ///
    /// let limits = BatchLimits::new(
    ///     NonZeroU16::MIN,
    ///     NonZeroU16::MIN,
    ///     NonZeroUsize::MIN,
    /// )?;
    /// assert_eq!(limits.max_items().get(), 1);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `max_registers` is above 125 or
    /// `max_coils` is above 2000.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-LIMIT-REGISTERS
    /// See the type documentation; the protocol maxima are rows `MB-LIMIT-REGISTERS` and
    /// `MB-LIMIT-COILS` (docs/parity/modbus.md#MB-LIMIT-COILS).
    #[must_use = "limit validation must be checked"]
    pub fn new(
        max_registers: NonZeroU16,
        max_coils: NonZeroU16,
        max_items: NonZeroUsize,
    ) -> Result<Self> {
        if max_registers.get() > PROTOCOL_MAX_REGISTERS {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                format!(
                    "max registers per request {max_registers} is above the protocol \
                     maximum of {PROTOCOL_MAX_REGISTERS} \
                     (docs/parity/modbus.md#MB-LIMIT-REGISTERS)"
                ),
            ));
        }
        if max_coils.get() > PROTOCOL_MAX_COILS {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                format!(
                    "max coils per request {max_coils} is above the protocol maximum of \
                     {PROTOCOL_MAX_COILS} (docs/parity/modbus.md#MB-LIMIT-COILS)"
                ),
            ));
        }
        Ok(Self {
            registers: max_registers,
            coils: max_coils,
            items: max_items,
        })
    }

    /// Most registers in one register-area chunk.
    #[must_use]
    pub const fn max_registers(&self) -> NonZeroU16 {
        self.registers
    }

    /// Most bits in one bit-area chunk.
    #[must_use]
    pub const fn max_coils(&self) -> NonZeroU16 {
        self.coils
    }

    /// Most tags in one plan and most members in one chunk.
    #[must_use]
    pub const fn max_items(&self) -> NonZeroUsize {
        self.items
    }

    /// The bound for chunks of `area`: bits for bit areas, registers otherwise.
    const fn limit_for(&self, area: ModbusArea) -> u16 {
        if area.is_bit_area() {
            self.coils.get()
        } else {
            self.registers.get()
        }
    }
}

impl Default for BatchLimits {
    fn default() -> Self {
        Self {
            registers: NonZeroU16::new(PROTOCOL_MAX_REGISTERS).unwrap_or(NonZeroU16::MAX),
            coils: NonZeroU16::new(PROTOCOL_MAX_COILS).unwrap_or(NonZeroU16::MAX),
            items: NonZeroUsize::new(DEFAULT_MAX_ITEMS).unwrap_or(NonZeroUsize::MAX),
        }
    }
}

/// One tag inside a [`Chunk`]: where its value sits and how to decode it.
///
/// `offset` counts registers (register areas) or bits (bit areas) from the chunk's
/// `start`, so a member occupies `offset..offset + width` of the chunk's data, where the
/// width is `data_type.register_size()` on register areas and 1 on bit areas.
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
/// `byte_order` and `word_order` are each resolved tag field over address block; a field
/// that is `None` means the connection default applies to it
/// (docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Member {
    /// Identity of the tag, as passed to [`Batcher::plan`].
    pub tag: TagId,
    /// Registers or bits from the chunk's `start` to the first of the tag's own.
    pub offset: u16,
    /// The scalar type to decode.
    pub data_type: ModbusDataType,
    /// Byte order from the tag field over the address block, `None` to defer to the
    /// connection default.
    pub byte_order: Option<ByteOrder>,
    /// Word order from the tag field over the address block, `None` to defer to the
    /// connection default.
    pub word_order: Option<WordOrder>,
}

/// One contiguous wire read covering one or more tags of the same unit and area.
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
/// A chunk is the counterpart of an upstream merged block (docs/parity/modbus.md#MB-READ-MERGE)
/// except that it never spans a gap (docs/parity/modbus.md#MB-READ-GAP).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Chunk {
    /// The unit identifier all members share; `None` means the connection default.
    pub unit_id: Option<UnitId>,
    /// The data area all members share.
    pub area: ModbusArea,
    /// Zero-based wire offset of the first register or bit read.
    pub start: u16,
    /// Number of registers (register areas) or bits (bit areas) to read, at least 1.
    pub quantity: u16,
    /// The tags served by this read, ordered by `offset`.
    pub members: Vec<Member>,
}

/// Plans reads: see [`Batcher::plan`].
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{AddressOptions, Tag, TagId};
/// use stonecrab_modbus::{BatchLimits, Batcher, ModbusAddress};
///
/// let options = AddressOptions::default();
/// let tags = ["holding-register:1", "holding-register:2", "holding-register:10"]
///     .iter()
///     .enumerate()
///     .map(|(index, text)| {
///         let address = ModbusAddress::parse(text, &options)?;
///         Ok(Tag::new(TagId::new(&format!("t{index}"))?, Arc::new(address)))
///     })
///     .collect::<stonecrab_core::Result<Vec<_>>>()?;
/// let chunks = Batcher::plan(&tags, &BatchLimits::default())?;
/// assert_eq!(chunks.len(), 2, "the gap before register 10 starts a new chunk");
/// assert_eq!(chunks[0].quantity, 2);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
/// Counterpart of the upstream `ModbusReadOptimizer`; see [`Batcher::plan`] and the
/// parity row for the merge rules.
#[derive(Debug, Clone, Copy, Default)]
pub struct Batcher;

/// A tag reduced to what planning needs.
struct Entry<'a> {
    unit_id: Option<UnitId>,
    area: ModbusArea,
    offset: u16,
    /// Registers (register areas) or bits (bit areas) the tag occupies.
    width: u16,
    tag: &'a Tag,
    address: &'a ModbusAddress,
}

/// The chunk under construction.
struct Open {
    unit_id: Option<UnitId>,
    area: ModbusArea,
    start: u16,
    /// One past the last register or bit covered so far.
    end: u32,
    members: Vec<Member>,
}

impl Batcher {
    /// Group, sort and merge `tags` into reads that respect `limits`.
    ///
    /// Tags are grouped by (unit identifier, area) and sorted by wire offset; two tags
    /// join one chunk only when the second starts at or before the end of the chunk so far
    /// (adjacent or overlapping, never across a gap) and the grown chunk stays within the
    /// area's bound. A chunk can never hold more than `max_items` members, because the
    /// whole plan is refused above that count. Chunks are ordered by (unit identifier, area,
    /// start), members by offset; tags with equal offsets keep their input order. Every
    /// input [`TagId`] appears in exactly one member. A bit area tag occupies one bit; a
    /// register area tag occupies `data_type.register_size()` registers.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{AddressOptions, Tag, TagId};
    /// use stonecrab_modbus::{BatchLimits, Batcher, ModbusAddress};
    ///
    /// let address = ModbusAddress::parse("coil:5", &AddressOptions::default())?;
    /// let tag = Tag::new(TagId::new("pump")?, Arc::new(address));
    /// let chunks = Batcher::plan(&[tag], &BatchLimits::default())?;
    /// assert_eq!((chunks[0].start, chunks[0].quantity), (4, 1));
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// - [`ErrorKind::ResourceLimit`] when `tags.len()` exceeds `limits.max_items()` (checked
    ///   before any planning) or a single tag is wider than the area's bound.
    /// - [`ErrorKind::InvalidAddress`] when a tag's address is not a [`ModbusAddress`].
    /// - [`ErrorKind::InvalidConfig`] when a tag's `count()` is not 1 (Modbus element counts
    ///   above one need array selections, which are deferred).
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-MERGE
    /// Counterpart of upstream `ModbusReadOptimizer` (docs/parity/modbus.md#MB-READ-MERGE).
    /// Divergence: no gap reads (docs/parity/modbus.md#MB-READ-GAP). The bounds are
    /// docs/parity/modbus.md#MB-LIMIT-REGISTERS (63 `DINT` tags are 126 registers and
    /// split into two chunks) and docs/parity/modbus.md#MB-LIMIT-COILS.
    #[must_use = "a plan does nothing until its chunks are read"]
    pub fn plan(tags: &[Tag], limits: &BatchLimits) -> Result<Vec<Chunk>> {
        if tags.len() > limits.max_items().get() {
            return Err(PlcError::new(
                ErrorKind::ResourceLimit,
                format!(
                    "{} tags exceed the limit of {} items per request",
                    tags.len(),
                    limits.max_items()
                ),
            ));
        }
        let mut entries = tags.iter().map(entry).collect::<Result<Vec<Entry<'_>>>>()?;
        // Stable: tags at the same offset keep their input order.
        entries.sort_by_key(|entry| (entry.unit_id, entry.area, entry.offset));

        let mut chunks = Vec::new();
        let mut open: Option<Open> = None;
        for entry in &entries {
            let limit = limits.limit_for(entry.area);
            // Only a multi-register tag can be wider than a bound, and bit tags are 1 wide.
            if entry.width > limit {
                return Err(PlcError::new(
                    ErrorKind::ResourceLimit,
                    format!(
                        "tag {} needs {} registers but a request carries at most {limit}",
                        entry.tag.id(),
                        entry.width,
                    ),
                ));
            }
            let order = resolve_order(entry.tag, entry.address);
            let end = u32::from(entry.offset).saturating_add(u32::from(entry.width));
            match open.as_mut() {
                Some(chunk) if chunk.accepts(entry, end, limit) => {
                    chunk.absorb(entry, end, order);
                }
                _ => {
                    if let Some(chunk) = open.take() {
                        chunks.push(chunk.finish());
                    }
                    open = Some(Open::start(entry, end, order));
                }
            }
        }
        if let Some(chunk) = open {
            chunks.push(chunk.finish());
        }
        Ok(chunks)
    }
}

impl Open {
    fn start(entry: &Entry<'_>, end: u32, order: Order) -> Self {
        Self {
            unit_id: entry.unit_id,
            area: entry.area,
            start: entry.offset,
            end,
            members: vec![entry.member(0, order)],
        }
    }

    /// Whether `entry` (ending at `end`) joins this chunk without a gap and within the
    /// span bound. The member bound needs no check here: `plan` has already refused more
    /// than `max_items` tags in total, so no chunk can hold more.
    fn accepts(&self, entry: &Entry<'_>, end: u32, limit: u16) -> bool {
        let grown = self.end.max(end).saturating_sub(u32::from(self.start));
        self.unit_id == entry.unit_id
            && self.area == entry.area
            && u32::from(entry.offset) <= self.end
            && grown <= u32::from(limit)
    }

    fn absorb(&mut self, entry: &Entry<'_>, end: u32, order: Order) {
        // Entries are sorted by offset, so the offset is never below `start`, and the
        // acceptance check kept the distance within the `u16` bound.
        let offset = entry.offset.saturating_sub(self.start);
        self.end = self.end.max(end);
        self.members.push(entry.member(offset, order));
    }

    fn finish(self) -> Chunk {
        let span = self.end.saturating_sub(u32::from(self.start));
        Chunk {
            unit_id: self.unit_id,
            area: self.area,
            start: self.start,
            // The span was kept within a `u16` limit by `accepts`.
            quantity: u16::try_from(span).unwrap_or(u16::MAX),
            members: self.members,
        }
    }
}

impl Entry<'_> {
    fn member(&self, offset: u16, (byte_order, word_order): Order) -> Member {
        Member {
            tag: self.tag.id().clone(),
            offset,
            data_type: self.address.data_type(),
            byte_order,
            word_order,
        }
    }
}

/// Validate one tag and reduce it to an [`Entry`].
fn entry(tag: &Tag) -> Result<Entry<'_>> {
    if tag.count().get() != 1 {
        return Err(PlcError::new(
            ErrorKind::InvalidConfig,
            format!(
                "tag {} has element count {}: Modbus element counts above one need array \
                 selections, which are not supported yet",
                tag.id(),
                tag.count()
            ),
        ));
    }
    let any: &dyn Any = tag.address();
    let Some(address) = any.downcast_ref::<ModbusAddress>() else {
        return Err(PlcError::new(
            ErrorKind::InvalidAddress,
            format!("tag {} does not carry a Modbus address", tag.id()),
        ));
    };
    let width = if address.area().is_bit_area() {
        1
    } else {
        address.data_type().register_size()
    };
    Ok(Entry {
        unit_id: address.unit_id(),
        area: address.area(),
        offset: address.offset(),
        width,
        tag,
        address,
    })
}

/// Resolved byte and word order of one tag; `None` defers that field to the connection.
type Order = (Option<ByteOrder>, Option<WordOrder>);

/// Tag fields over the address block, field by field; a field set by neither stays
/// `None` for the connection default.
fn resolve_order(tag: &Tag, address: &ModbusAddress) -> Order {
    let block = address.byte_order();
    (
        tag.byte_order().or(block.map(|(byte, _)| byte)),
        tag.word_order().or(block.map(|(_, word)| word)),
    )
}
