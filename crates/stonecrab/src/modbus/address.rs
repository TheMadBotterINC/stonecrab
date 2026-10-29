//! Modbus address types: unit identifier, data area, data type, convention and the
//! validated address value.
//!
//! A [`ModbusAddress`] is assembled from its parts with [`ModbusAddressBuilder`] or parsed
//! from text by [`ModbusAddress::parse`]; both paths end in the builder's validation, so
//! the span and bit-area rules live in one place. The text grammar is a small hand-written
//! scanner at the end of the file: it never indexes, slices with `[]`, casts or does
//! unchecked arithmetic, and every rejection is an [`ErrorKind::InvalidAddress`] whose
//! message names the reason and echoes at most a bounded, control-stripped excerpt of the
//! input.

use std::fmt;
use std::num::NonZeroU16;

use crate::modbus::{MAX_READ_BITS, MAX_READ_EXTENDED_REGISTERS, MAX_READ_REGISTERS};
use crate::{
    AddressOptions, ByteOrder, ErrorKind, PlcError, PlcValueType, ProtocolAddress, Result,
    WordOrder,
};

/// Number of addresses in one Modbus data area: wire offsets run from 0 to 65535.
const ADDRESS_SPACE: u32 = 65_536;

/// Registers in one file of the extended-register area (V1.1b section 6.14): an extended
/// address counts registers through all the files, 10000 to a file.
const FILE_REGISTERS: u32 = 10_000;

/// The last extended-register address a request can name: record 9999 of file 65535, the
/// last file number of the wire.
const LAST_EXTENDED: u32 = 655_349_999;

/// Longest unrecognised data type name echoed back in an error message.
const NAME_ECHO_LIMIT: usize = 32;

/// The unit identifier byte of the MBAP header: which device behind a gateway is addressed.
///
/// A unit identifier is one byte on the wire, so every `u8` is representable and the type
/// has no failing constructor. Two values have a conventional role in the Modbus Messaging
/// on TCP/IP Implementation Guide: [`UnitId::BROADCAST`] (0) and [`UnitId::TCP_NO_UNIT`]
/// (255). Stonecrab attaches no special meaning to either and rejects neither; a caller
/// that talks to a device expecting one of them can say so. The default is
/// [`UnitId::DEFAULT`] (1), the upstream `default-unit-identifier`.
///
/// # Examples
/// ```
/// use stonecrab::modbus::UnitId;
///
/// assert_eq!(UnitId::default(), UnitId::from(1));
/// assert_eq!(UnitId::from(10).get(), 10);
/// assert_eq!(u8::from(UnitId::TCP_NO_UNIT), 255);
/// assert_eq!(UnitId::from(10).to_string(), "10");
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-CFG-UNIT-ID
/// The `unit-id` key of the address block is row `MB-ADDR-CONFIG-BLOCK`, and
/// `MB-ADDR-LONG` gives the surrounding address grammar. Upstream reads the default with `default-unit-identifier` (default 1) and parses the
/// address block key `unit-id` with `Short.parseShort`; the inventoried files show no
/// range check. The 0..=255 bound here is a Stonecrab inference from the one-byte MBAP
/// field and is recorded as a divergence row: [`ModbusAddress::parse`] rejects a
/// `unit-id` outside it (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK). The conventional
/// meaning of 0 and 255 is listed as UNVERIFIED in the parity record because the
/// implementation guide is not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct UnitId(u8);

impl UnitId {
    /// The upstream default unit identifier, 1.
    pub const DEFAULT: Self = Self(1);

    /// Unit identifier 0, conventionally the broadcast address on serial lines.
    ///
    /// Accepted and carried unchanged; no reply handling is attached to it.
    pub const BROADCAST: Self = Self(0);

    /// Unit identifier 255, conventionally "no unit" for a directly addressed TCP device.
    ///
    /// Accepted and carried unchanged.
    pub const TCP_NO_UNIT: Self = Self(255);

    /// The identifier as the byte that goes on the wire.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::UnitId;
    /// assert_eq!(UnitId::BROADCAST.get(), 0);
    /// ```
    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl Default for UnitId {
    /// The upstream default unit identifier, 1.
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl From<u8> for UnitId {
    fn from(value: u8) -> Self {
        Self(value)
    }
}

impl From<UnitId> for u8 {
    fn from(value: UnitId) -> Self {
        value.0
    }
}

impl fmt::Display for UnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// One of the five Modbus data areas the driver reads.
///
/// Coils and discrete inputs are bit areas (one bit per address); holding registers, input
/// registers and extended registers are register areas (one 16-bit register per address).
/// Coils, holding registers and extended registers are writable on the wire and the two
/// input areas are read-only. Extended registers are the records of the files of the file
/// record function codes 20 and 21: an address counts registers through all the files,
/// 10000 to a file, and the file number on the wire is 1-based (see
/// [`ModbusAddress::file_number`]). The enum is `#[non_exhaustive]`.
///
/// # Examples
/// ```
/// use stonecrab::modbus::ModbusArea;
///
/// let area = ModbusArea::HoldingRegister;
/// assert_eq!(area.long_name(), "holding-register");
/// assert_eq!(area.prefix_digit(), 4);
/// assert!(!area.is_bit_area());
/// assert!(area.is_writable());
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
/// The long names are the upstream area keywords (`MB-ADDR-LONG`) and the prefix digits
/// are the `0x`, `1x`, `3x`, `4x` and `6x` forms (docs/parity/modbus.md#MB-ADDR-SHORT-4X).
/// There is no `2x` area upstream. The extended-register area is
/// docs/parity/modbus.md#MB-ADDR-EXTENDED; it comes last because a new variant of a
/// published enum goes after the existing ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ModbusArea {
    /// Read/write single bits, function codes 01, 05 and 15; upstream prefix `0x`.
    Coil,
    /// Read-only single bits, function code 02; upstream prefix `1x`.
    DiscreteInput,
    /// Read/write 16-bit registers, function codes 03, 06 and 16; upstream prefix `4x`.
    HoldingRegister,
    /// Read-only 16-bit registers, function code 04; upstream prefix `3x`.
    InputRegister,
    /// Read/write 16-bit registers of the files of function codes 20 (read) and 21
    /// (write); upstream prefix `6x`.
    ExtendedRegister,
}

impl ModbusArea {
    /// Whether one address holds a single bit (coils and discrete inputs).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusArea;
    /// assert!(ModbusArea::Coil.is_bit_area());
    /// assert!(!ModbusArea::InputRegister.is_bit_area());
    /// ```
    #[must_use]
    pub const fn is_bit_area(self) -> bool {
        match self {
            Self::Coil | Self::DiscreteInput => true,
            Self::HoldingRegister | Self::InputRegister | Self::ExtendedRegister => false,
        }
    }

    /// Whether the protocol allows writing the area (coils, holding registers and extended
    /// registers).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusArea;
    /// assert!(ModbusArea::Coil.is_writable());
    /// assert!(!ModbusArea::DiscreteInput.is_writable());
    /// ```
    #[must_use]
    pub const fn is_writable(self) -> bool {
        match self {
            Self::Coil | Self::HoldingRegister | Self::ExtendedRegister => true,
            Self::DiscreteInput | Self::InputRegister => false,
        }
    }

    /// The digit that starts the upstream short forms: 0, 1, 3, 4 or 6.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusArea;
    /// assert_eq!(ModbusArea::Coil.prefix_digit(), 0);
    /// assert_eq!(ModbusArea::DiscreteInput.prefix_digit(), 1);
    /// assert_eq!(ModbusArea::InputRegister.prefix_digit(), 3);
    /// assert_eq!(ModbusArea::HoldingRegister.prefix_digit(), 4);
    /// assert_eq!(ModbusArea::ExtendedRegister.prefix_digit(), 6);
    /// ```
    #[must_use]
    pub const fn prefix_digit(self) -> u8 {
        match self {
            Self::Coil => 0,
            Self::DiscreteInput => 1,
            Self::InputRegister => 3,
            Self::HoldingRegister => 4,
            Self::ExtendedRegister => 6,
        }
    }

    /// The long-form area keyword: `coil`, `discrete-input`, `holding-register`,
    /// `input-register` or `extended-register`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusArea;
    /// assert_eq!(ModbusArea::DiscreteInput.long_name(), "discrete-input");
    /// ```
    #[must_use]
    pub const fn long_name(self) -> &'static str {
        match self {
            Self::Coil => "coil",
            Self::DiscreteInput => "discrete-input",
            Self::HoldingRegister => "holding-register",
            Self::InputRegister => "input-register",
            Self::ExtendedRegister => "extended-register",
        }
    }

    /// The type an address of this area has when none is written: `BOOL` for the bit
    /// areas and `INT` for the register areas.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusArea, ModbusDataType};
    /// assert_eq!(ModbusArea::Coil.default_data_type(), ModbusDataType::Bool);
    /// assert_eq!(ModbusArea::InputRegister.default_data_type(), ModbusDataType::Int);
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
    /// Upstream defaults to `BOOL` for bit areas and `INT` for register areas.
    #[must_use]
    pub const fn default_data_type(self) -> ModbusDataType {
        if self.is_bit_area() {
            ModbusDataType::Bool
        } else {
            ModbusDataType::Int
        }
    }
}

/// A scalar type the alpha reads from a Modbus device: the 15 numeric and bit types of
/// the 27 upstream `ModbusDataType` names.
///
/// The other 12 upstream names (`STRING`, `WSTRING`, `CHAR`, `WCHAR` and the `TIME` and
/// `DATE` family) are recognised by [`ModbusDataType::from_upstream_name`] and refused
/// with a message that names the deferral. The enum is `#[non_exhaustive]` because the
/// string and character types join it in a later milestone.
///
/// # Examples
/// ```
/// use stonecrab::PlcValueType;
/// use stonecrab::modbus::ModbusDataType;
///
/// let kind = ModbusDataType::from_upstream_name("DINT")?;
/// assert_eq!(kind, ModbusDataType::DInt);
/// assert_eq!(kind.register_size(), 2);
/// assert_eq!(kind.value_type(), PlcValueType::I32);
/// assert_eq!(kind.upstream_name(), "DINT");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
/// The `:TYPE` part of the address text is row `MB-ADDR-LONG` (names, case and defaults)
/// and `MB-ADDR-BIT-TYPES` (bit areas accept `BOOL` only). Rows `MB-TYPE-BOOL` to `MB-TYPE-LREAL` give the register width of each accepted type
/// and `MB-TYPE-CHAR` to `MB-TYPE-LDATE-AND-TIME` the deferred ones. Upstream resolves
/// the name with the case-sensitive `ModbusDataType.valueOf`, so lowercase names are
/// unknown here too (docs/parity/modbus.md#MB-ADDR-LONG). The type is the optional last
/// part of an address text; non-`BOOL` types on bit areas are refused by
/// [`ModbusAddress::parse`] (docs/parity/modbus.md#MB-ADDR-BIT-TYPES) and the string and
/// calendar names are typed rejections (docs/parity/modbus.md#MB-ADDR-STRING,
/// docs/parity/modbus.md#MB-TYPE-TIME).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ModbusDataType {
    /// One bit on a bit area; on a register area bit 0 of one 16-bit register (bit 8 under
    /// little-endian byte order), the other bits being padding that is ignored
    /// (docs/parity/modbus.md#MB-TYPE-BOOL).
    Bool,
    /// Unsigned 8-bit value in one register.
    Byte,
    /// Unsigned 16-bit value.
    Word,
    /// Unsigned 32-bit value in two registers.
    DWord,
    /// Unsigned 64-bit value in four registers.
    LWord,
    /// Signed 8-bit value in one register.
    SInt,
    /// Unsigned 8-bit value in one register.
    USInt,
    /// Signed 16-bit value; the default type of the register areas.
    Int,
    /// Unsigned 16-bit value.
    UInt,
    /// Signed 32-bit value in two registers.
    DInt,
    /// Unsigned 32-bit value in two registers.
    UDInt,
    /// Signed 64-bit value in four registers.
    LInt,
    /// Unsigned 64-bit value in four registers.
    ULInt,
    /// IEEE 754 single precision value in two registers.
    Real,
    /// IEEE 754 double precision value in four registers.
    LReal,
}

impl ModbusDataType {
    /// How many 16-bit registers one element occupies when read alone: 1, 2 or 4.
    ///
    /// A `BOOL` counts as one even on a bit area, where it is one bit at one address.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusDataType;
    /// assert_eq!(ModbusDataType::Int.register_size(), 1);
    /// assert_eq!(ModbusDataType::Real.register_size(), 2);
    /// assert_eq!(ModbusDataType::LReal.register_size(), 4);
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
    /// The widths are the "Registers" column of the data type table.
    #[must_use]
    pub const fn register_size(self) -> u16 {
        match self {
            Self::Bool
            | Self::Byte
            | Self::Word
            | Self::SInt
            | Self::USInt
            | Self::Int
            | Self::UInt => 1,
            Self::DWord | Self::DInt | Self::UDInt | Self::Real => 2,
            Self::LWord | Self::LInt | Self::ULInt | Self::LReal => 4,
        }
    }

    /// The upstream spelling of the type, as written after the last colon of an address.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::ModbusDataType;
    /// assert_eq!(ModbusDataType::USInt.upstream_name(), "USINT");
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
    /// The names are the upstream enum constants, upper case.
    #[must_use]
    pub const fn upstream_name(self) -> &'static str {
        match self {
            Self::Bool => "BOOL",
            Self::Byte => "BYTE",
            Self::Word => "WORD",
            Self::DWord => "DWORD",
            Self::LWord => "LWORD",
            Self::SInt => "SINT",
            Self::USInt => "USINT",
            Self::Int => "INT",
            Self::UInt => "UINT",
            Self::DInt => "DINT",
            Self::UDInt => "UDINT",
            Self::LInt => "LINT",
            Self::ULInt => "ULINT",
            Self::Real => "REAL",
            Self::LReal => "LREAL",
        }
    }

    /// The core scalar type a value of this Modbus type decodes to.
    ///
    /// `BYTE`, `WORD`, `DWORD` and `LWORD` are bit strings upstream, encoded as unsigned
    /// integers (`uint 8`, `uint 16`, `uint 32`, `uint 64` in the mspec), so they map to
    /// the unsigned core types.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValueType;
    /// use stonecrab::modbus::ModbusDataType;
    /// assert_eq!(ModbusDataType::Real.value_type(), PlcValueType::F32);
    /// assert_eq!(ModbusDataType::LWord.value_type(), PlcValueType::U64);
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
    /// The mapping follows the "`DataItem` encoding" column of the data type table.
    #[must_use]
    pub const fn value_type(self) -> PlcValueType {
        match self {
            Self::Bool => PlcValueType::Bool,
            Self::Byte | Self::USInt => PlcValueType::U8,
            Self::Word | Self::UInt => PlcValueType::U16,
            Self::DWord | Self::UDInt => PlcValueType::U32,
            Self::LWord | Self::ULInt => PlcValueType::U64,
            Self::SInt => PlcValueType::I8,
            Self::Int => PlcValueType::I16,
            Self::DInt => PlcValueType::I32,
            Self::LInt => PlcValueType::I64,
            Self::Real => PlcValueType::F32,
            Self::LReal => PlcValueType::F64,
        }
    }

    /// Resolve one of the 27 upstream `ModbusDataType` names.
    ///
    /// The match is case-sensitive, like upstream's `valueOf`. The 15 numeric and bit
    /// names succeed. The 12 deferred names fail with a message that says what is
    /// deferred and why; any other text fails as an unknown name.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] for `STRING`, `WSTRING`, `CHAR` and `WCHAR`
    /// (deferred to a later milestone), for `TIME`, `LTIME`, `DATE`, `LDATE`,
    /// `TIME_OF_DAY`, `LTIME_OF_DAY`, `DATE_AND_TIME` and `LDATE_AND_TIME` (calendar types
    /// the alpha cannot represent), and for any name upstream does not define.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::modbus::ModbusDataType;
    ///
    /// assert_eq!(ModbusDataType::from_upstream_name("REAL")?, ModbusDataType::Real);
    /// let refused = ModbusDataType::from_upstream_name("STRING");
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
    /// The accepted names are rows `MB-TYPE-BOOL` to `MB-TYPE-LREAL`. The deferred names
    /// are `MB-TYPE-CHAR` to `MB-TYPE-LDATE-AND-TIME`; refusing the calendar names is a
    /// divergence (docs/parity/modbus.md#MB-TYPE-TIME) and refusing the string and
    /// character names another (docs/parity/modbus.md#MB-ADDR-STRING).
    pub fn from_upstream_name(name: &str) -> Result<Self> {
        match name {
            "BOOL" => Ok(Self::Bool),
            "BYTE" => Ok(Self::Byte),
            "WORD" => Ok(Self::Word),
            "DWORD" => Ok(Self::DWord),
            "LWORD" => Ok(Self::LWord),
            "SINT" => Ok(Self::SInt),
            "USINT" => Ok(Self::USInt),
            "INT" => Ok(Self::Int),
            "UINT" => Ok(Self::UInt),
            "DINT" => Ok(Self::DInt),
            "UDINT" => Ok(Self::UDInt),
            "LINT" => Ok(Self::LInt),
            "ULINT" => Ok(Self::ULInt),
            "REAL" => Ok(Self::Real),
            "LREAL" => Ok(Self::LReal),
            "STRING" | "WSTRING" | "CHAR" | "WCHAR" => Err(invalid_address(format!(
                "data type {name} is not supported in the alpha: string and character \
                 types are deferred to a later milestone \
                 (docs/parity/modbus.md#MB-ADDR-STRING)"
            ))),
            "TIME" | "LTIME" | "DATE" | "LDATE" | "TIME_OF_DAY" | "LTIME_OF_DAY"
            | "DATE_AND_TIME" | "LDATE_AND_TIME" => Err(invalid_address(format!(
                "data type {name} is not supported: calendar types cannot be encoded \
                 (docs/parity/modbus.md#MB-TYPE-TIME)"
            ))),
            other => Err(invalid_address(format!(
                "unknown Modbus data type '{}'; type names are upper case, for example INT",
                other
                    .chars()
                    .filter(|c| !c.is_control())
                    .take(NAME_ECHO_LIMIT)
                    .collect::<String>()
            ))),
        }
    }
}

/// How the numbers of an address are read.
///
/// The convention decides two things about address text: which spellings are accepted
/// and what offset the written number is counted from. It is not part of the address it
/// produces, which is always the wire offset (see [`ModbusAddress::convention`]).
/// The enum is `#[non_exhaustive]`; a convention is selected by its name in
/// [`AddressOptions`](crate::AddressOptions), never inferred from the text.
///
/// | Variant | Name | Number counts from | Long and prefixed short forms | Bare short forms |
/// | --- | --- | --- | --- | --- |
/// | (none supplied) | | 1 | accepted | refused |
/// | [`Plc4xCompatible`](Self::Plc4xCompatible) | `plc4x` | 1 | accepted | accepted |
/// | [`OneBased40001`](Self::OneBased40001) | `one-based-40001` | 1 | accepted | accepted |
/// | [`ZeroBased`](Self::ZeroBased) | `zero-based` | 0 | accepted | accepted |
///
/// Counting from 1, the wire offset is the number minus one and the numbers run from 1
/// to 65536; counting from 0, the wire offset is the number and the numbers run from 0 to
/// 65535. The rule applies to every number in the text: `holding-register:N`, the digits
/// of `4xNNNNN` and the digits after the prefix digit of a bare `4NNNN`. So
/// `holding-register:1` reads wire offset 0 with no convention, `plc4x` and
/// `one-based-40001`, and wire offset 1 with `zero-based`. With none supplied the long and
/// prefixed short forms keep the PLC4X reading and the bare forms are refused.
///
/// # Examples
/// ```
/// use stonecrab::modbus::AddressConvention;
/// assert_eq!(AddressConvention::default(), AddressConvention::Plc4xCompatible);
/// assert_eq!(AddressConvention::ZeroBased.name(), "zero-based");
/// assert_eq!(
///     AddressConvention::from_name("one-based-40001"),
///     Some(AddressConvention::OneBased40001)
/// );
/// assert_eq!(AddressConvention::from_name("PLC4X"), None);
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-ADDR-BARE-40001
/// `Plc4xCompatible` is the verified upstream grammar, in which a bare `40001` is
/// holding register 1. Upstream accepts the bare forms unconditionally; Stonecrab accepts
/// them only when a convention is supplied explicitly (divergence row of the same id,
/// implemented by [`ModbusAddress::parse`]). `OneBased40001` reads every spelling exactly
/// as `Plc4xCompatible` does: the two names accept the same text and give the same
/// offsets, and differ in what they promise. `plc4x` says the text follows the PLC4X
/// grammar; `one-based-40001` says only that the numbers are the one-based Modicon
/// numbers of a device manual, with no claim about PLC4X. `ZeroBased` has no upstream
/// counterpart (docs/parity/modbus.md#MB-ADDR-CONVENTIONS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AddressConvention {
    /// The Apache PLC4X grammar: one-based logical addresses, wire address = logical - 1.
    #[default]
    Plc4xCompatible,
    /// Numbers are the zero-based wire addresses of the protocol, 0 to 65535: `4x00000`,
    /// `holding-register:0` and the bare `40000` are all wire offset 0.
    ZeroBased,
    /// Numbers are the one-based Modicon numbers of a device manual, as in
    /// `Plc4xCompatible`: bare `40001` is the first holding register, wire offset 0.
    OneBased40001,
}

impl AddressConvention {
    /// The name that selects this convention in `address-convention` and in
    /// [`AddressOptionsBuilder::convention`](crate::AddressOptionsBuilder::convention).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::AddressConvention;
    /// assert_eq!(AddressConvention::Plc4xCompatible.name(), "plc4x");
    /// assert_eq!(AddressConvention::OneBased40001.name(), "one-based-40001");
    /// ```
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Plc4xCompatible => "plc4x",
            Self::ZeroBased => "zero-based",
            Self::OneBased40001 => "one-based-40001",
        }
    }

    /// The convention a name selects, `None` for any other text. Names are exact: no
    /// case folding and no trimming.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::AddressConvention;
    /// assert_eq!(AddressConvention::from_name("plc4x"), Some(AddressConvention::Plc4xCompatible));
    /// assert_eq!(AddressConvention::from_name("zero_based"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        [Self::Plc4xCompatible, Self::ZeroBased, Self::OneBased40001]
            .into_iter()
            .find(|convention| convention.name() == name)
    }
}

/// A validated Modbus address: unit, area, wire offset, type, byte order and, for a range
/// selection, the number of elements.
///
/// The `offset` is the zero-based wire address, the number that goes into the PDU. The
/// one-based logical address people write (`holding-register:16`, `4x00016`) is
/// `offset + 1` and is available as [`ModbusAddress::logical_address`]. Fields are
/// private and an address is created only through [`ModbusAddress::builder`] or
/// [`ModbusAddress::parse`] (which ends in the builder), so every value in existence
/// passed the checks of [`ModbusAddressBuilder::build`].
///
/// Equality, hashing and `Clone` cover every field, including the byte order: two
/// addresses that differ only in `byte-order` are different values.
///
/// An address names one element, or with [`elements`](Self::elements) a list of
/// consecutive elements of its type that is read and written as one
/// [`PlcValue::List`](crate::PlcValue::List): what the range selection `[lo..hi]` of the
/// text grammar produces. A list takes `elements` times the element's width, one bit on a
/// coil or discrete input and [`ModbusDataType::register_size`] registers on a register
/// area, the extended registers included, and each element has the layout it has alone
/// (ADR-0022).
///
/// # Canonical text
/// [`ProtocolAddress::canonical`] renders the unambiguous long form
/// `AREA:LOGICAL:TYPE`, for example `holding-register:16:INT`, with the range
/// `[0..N-1]` after the number for a list of `N` elements
/// (`holding-register:16[0..3]:INT`), followed by an address
/// block only when something is set: `{unit-id: 10}`, `{byte-order: 'LITTLE_ENDIAN'}` or
/// both as `{unit-id: 10, byte-order: 'LITTLE_ENDIAN'}`. The block has no space before
/// the opening brace, keys come in that order and enum values are single-quoted, as in
/// the upstream examples. The type is always written, even when it is the area default,
/// and the bare short forms (`40016`) are never produced because they are ambiguous. The
/// byte order is written with the upstream mode name derived from the
/// ([`ByteOrder`], [`WordOrder`]) pair:
///
/// | `ByteOrder` | `WordOrder` | Name |
/// | --- | --- | --- |
/// | `BigEndian` | `HighWordFirst` | `BIG_ENDIAN` |
/// | `LittleEndian` | `LowWordFirst` | `LITTLE_ENDIAN` |
/// | `LittleEndian` | `HighWordFirst` | `BIG_ENDIAN_BYTE_SWAP` |
/// | `BigEndian` | `LowWordFirst` | `LITTLE_ENDIAN_BYTE_SWAP` |
///
/// The number is one-based. [`ModbusAddress::parse`] accepts this text with no
/// convention, `plc4x` or `one-based-40001` and returns an equal address, which
/// `prop_canonical_round_trips` checks; under `zero-based` the same text names the next
/// register up, so a zero-based reader writes the number minus one. The extended-register
/// area is the exception: its number is the register address under every convention
/// (`extended-register:30000:INT` is address 30000, in file 4 at record 0), and the
/// number 0 does not exist.
///
/// # Examples
/// ```
/// use stonecrab::ProtocolAddress;
/// use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType};
///
/// let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 15)
///     .data_type(ModbusDataType::Int)
///     .build()?;
/// assert_eq!(address.offset(), 15);
/// assert_eq!(address.logical_address(), 16);
/// assert_eq!(address.protocol(), "modbus");
/// assert_eq!(address.canonical(), "holding-register:16:INT");
///
/// // An extended register is a file and a record: address 30000 is record 0 of file 4.
/// let extended = ModbusAddress::extended(30_000).build()?;
/// assert_eq!((extended.file_number().map(|file| file.get()), extended.offset()), (Some(4), 0));
/// assert_eq!(extended.canonical(), "extended-register:30000:INT");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
/// The text forms an address is parsed from are rows `MB-ADDR-LONG`, `MB-ADDR-SHORT-4X`,
/// `MB-ADDR-BARE-40001` and `MB-ADDR-CONFIG-BLOCK` (see [`ModbusAddress::parse`]), and
/// the digit-count limit is docs/parity/modbus.md#MB-LIMIT-DIGITS. The optional byte
/// order is the address-block source of the upstream precedence
/// (docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE): the tag field wins over it and it
/// wins over the connection default. Byte order takes part in equality, which upstream's
/// `equals` does not do (docs/parity/modbus.md#MB-QUIRK-L). The offset and span bounds are
/// docs/parity/modbus.md#MB-LIMIT-ADDR and docs/parity/modbus.md#MB-LIMIT-SPAN. A list
/// is the range selection of docs/parity/modbus.md#MB-ADDR-SELECTION.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModbusAddress {
    unit_id: Option<UnitId>,
    area: ModbusArea,
    offset: u16,
    /// The file number of an extended register; `offset` is then the record in it.
    file: Option<NonZeroU16>,
    data_type: ModbusDataType,
    byte_order: Option<(ByteOrder, WordOrder)>,
    convention: AddressConvention,
    elements: Option<NonZeroU16>,
}

impl ModbusAddress {
    /// The upstream default `ping-address`, `4x00001:BOOL`: holding register 1 (wire
    /// offset 0) as a `BOOL`, with no unit and no byte order. Built as a constant because
    /// the parsing and building entry points are fallible and a `Default` cannot be.
    pub(crate) const DEFAULT_PING: Self = Self {
        unit_id: None,
        area: ModbusArea::HoldingRegister,
        offset: 0,
        file: None,
        data_type: ModbusDataType::Bool,
        byte_order: None,
        convention: AddressConvention::Plc4xCompatible,
        elements: None,
    };

    /// Start building an address at a zero-based wire `offset` of `area`.
    ///
    /// For [`ModbusArea::ExtendedRegister`] the `offset` is the register address (see
    /// [`extended`](Self::extended)), which is not zero-based: address 0 is refused by
    /// [`build`](ModbusAddressBuilder::build).
    ///
    /// Defaults, documented on [`ModbusAddressBuilder`]: the area's default data type, no
    /// unit identifier, no byte order, [`AddressConvention::Plc4xCompatible`] and one
    /// element.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea};
    /// let builder = ModbusAddress::builder(ModbusArea::Coil, 0);
    /// let address = builder.build()?;
    /// assert_eq!(address.logical_address(), 1);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub const fn builder(area: ModbusArea, offset: u16) -> ModbusAddressBuilder {
        // A `const fn` cannot call `u32::from`, and `as` is not used in library code.
        let [low, high] = offset.to_le_bytes();
        ModbusAddressBuilder::at(area, u32::from_le_bytes([low, high, 0, 0]))
    }

    /// Start building an extended-register address: register `address` of the files of
    /// function codes 20 and 21, counted through all the files, 10000 to a file.
    ///
    /// The address is its own logical number: there is no one-based shift, and 0 does not
    /// exist. Address `n` is record `n % 10000` of file `n / 10000 + 1`, so the largest
    /// address, 655349999, is record 9999 of file 65535. A value wider than the registers
    /// left in a file continues at record 0 of the next file.
    ///
    /// Defaults are those of [`builder`](Self::builder): `INT`, no unit identifier, no byte
    /// order, one element. [`build`](ModbusAddressBuilder::build) refuses address 0, a span
    /// past 655349999 and a list ([`elements`](ModbusAddressBuilder::elements)) wider than
    /// the 120 registers one file record read carries.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::modbus::{ModbusAddress, ModbusDataType};
    ///
    /// // Address 9999 is the last record of file 1, and a DINT there ends in file 2.
    /// let address = ModbusAddress::extended(9_999).data_type(ModbusDataType::DInt).build()?;
    /// assert_eq!(address.file_number().map(|file| file.get()), Some(1));
    /// assert_eq!(address.offset(), 9_999);
    ///
    /// let zero = ModbusAddress::extended(0).build();
    /// assert_eq!(zero.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-EXTENDED
    /// The address is upstream's `ModbusTagExtendedRegister` address: equal to the logical
    /// address (`getLogicalAddress`), 0 rejected. Upstream accepts addresses up to
    /// 655359999, of which those from 655350000 name file 65536, beyond the 16 bits of the
    /// wire; Stonecrab refuses them (docs/parity/modbus.md#MB-LIMIT-EXT).
    pub const fn extended(address: u32) -> ModbusAddressBuilder {
        ModbusAddressBuilder::at(ModbusArea::ExtendedRegister, address)
    }

    /// The unit identifier written in the address, `None` when the address names none
    /// and the connection default applies.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea, UnitId};
    /// let address = ModbusAddress::builder(ModbusArea::Coil, 0)
    ///     .unit_id(UnitId::from(7))
    ///     .build()?;
    /// assert_eq!(address.unit_id(), Some(UnitId::from(7)));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub const fn unit_id(&self) -> Option<UnitId> {
        self.unit_id
    }

    /// The data area.
    #[must_use]
    pub const fn area(&self) -> ModbusArea {
        self.area
    }

    /// The zero-based wire address that goes into the request PDU; for a list, the wire
    /// address of its first element. For an extended register it is the record number in
    /// [`file_number`](Self::file_number)'s file, 0 to 9999, because the register address
    /// does not fit 16 bits.
    #[must_use]
    pub const fn offset(&self) -> u16 {
        self.offset
    }

    /// The number of elements of a list, `None` for an address that names one element.
    ///
    /// A list of one (`[4..4]` in the text) is `Some(1)`: it reads and writes a
    /// [`PlcValue::List`](crate::PlcValue::List), where the index `[4]` names one element
    /// and reads a scalar.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::AddressOptions;
    /// use stonecrab::modbus::ModbusAddress;
    ///
    /// let none = AddressOptions::default();
    /// let range = ModbusAddress::parse("holding-register:1[4..7]:DINT", &none)?;
    /// assert_eq!(range.elements().map(|count| count.get()), Some(4));
    /// let index = ModbusAddress::parse("holding-register:1[4]:DINT", &none)?;
    /// assert_eq!(index.elements(), None);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-SELECTION
    /// Upstream carries the count and an `explicitRange` flag, which this `Option` folds
    /// into one value.
    #[must_use]
    pub const fn elements(&self) -> Option<NonZeroU16> {
        self.elements
    }

    /// The file number of an extended register as it goes on the wire, 1 to 65535, `None`
    /// for every other area. [`offset`](Self::offset) is then the record in that file. For
    /// a list they place its first register; the list may continue in the next file.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea};
    ///
    /// let first = ModbusAddress::extended(1).build()?;
    /// assert_eq!(first.file_number().map(|file| file.get()), Some(1));
    /// let holding = ModbusAddress::builder(ModbusArea::HoldingRegister, 1).build()?;
    /// assert_eq!(holding.file_number(), None);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub const fn file_number(&self) -> Option<NonZeroU16> {
        self.file
    }

    /// The address of the first register in the numbering of its area: the wire offset, or
    /// for an extended register its position counted through the files (`(file - 1) *
    /// 10000 + record`, at most 655349999).
    pub(crate) fn linear_address(&self) -> u32 {
        match self.file {
            Some(file) => u32::from(file.get())
                .saturating_sub(1)
                .saturating_mul(FILE_REGISTERS)
                .saturating_add(u32::from(self.offset)),
            None => u32::from(self.offset),
        }
    }

    /// The file number and the record of an extended-register `address`, `None` when the
    /// address is past file 65535.
    pub(crate) fn split_extended(address: u32) -> Option<(NonZeroU16, u16)> {
        let file = address.checked_div(FILE_REGISTERS)?.checked_add(1)?;
        let record = address.checked_rem(FILE_REGISTERS)?;
        Some((
            NonZeroU16::new(u16::try_from(file).ok()?)?,
            u16::try_from(record).ok()?,
        ))
    }

    /// How many registers or bits a read of the address covers: the element count times
    /// one bit, or times the data type's register count.
    pub(crate) const fn width(&self) -> u16 {
        let element = element_width(self.area, self.data_type);
        // `build` bounded the product by the per-request maximum.
        match self.elements {
            Some(count) => element.saturating_mul(count.get()),
            None => element,
        }
    }

    /// The logical address people write: the wire offset plus one, 1 to 65536, or for an
    /// extended register the register address itself, 1 to 655349999.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
    /// Upstream's wire address is the logical address minus one (`PROTOCOL_ADDRESS_OFFSET`),
    /// except for extended registers, whose logical address is their address
    /// (docs/parity/modbus.md#MB-ADDR-EXTENDED).
    #[must_use]
    pub fn logical_address(&self) -> u32 {
        match self.area {
            ModbusArea::ExtendedRegister => self.linear_address(),
            ModbusArea::Coil
            | ModbusArea::DiscreteInput
            | ModbusArea::HoldingRegister
            | ModbusArea::InputRegister => u32::from(self.offset).saturating_add(1),
        }
    }

    /// The scalar type read at the address.
    #[must_use]
    pub const fn data_type(&self) -> ModbusDataType {
        self.data_type
    }

    /// The byte and word order written in the address block, `None` when absent.
    ///
    /// `None` means "defer to the connection default"; it is not big-endian.
    #[must_use]
    pub const fn byte_order(&self) -> Option<(ByteOrder, WordOrder)> {
        self.byte_order
    }

    /// The convention recorded on the address: [`AddressConvention::Plc4xCompatible`]
    /// unless [`ModbusAddressBuilder::convention`] set another.
    ///
    /// [`ModbusAddress::parse`] never sets another: an address is a wire target, so the
    /// same register read under `plc4x`, `one-based-40001` or `zero-based` compares equal
    /// and renders the same [`canonical`](crate::ProtocolAddress::canonical) text.
    #[must_use]
    pub const fn convention(&self) -> AddressConvention {
        self.convention
    }
}

impl ProtocolAddress for ModbusAddress {
    fn protocol(&self) -> &'static str {
        "modbus"
    }

    fn canonical(&self) -> String {
        let range = self
            .elements
            .map(|count| format!("[0..{}]", count.get().saturating_sub(1)))
            .unwrap_or_default();
        let base = format!(
            "{}:{}{range}:{}",
            self.area.long_name(),
            self.logical_address(),
            self.data_type.upstream_name()
        );
        let mut fields: Vec<String> = Vec::new();
        if let Some(unit_id) = self.unit_id {
            fields.push(format!("unit-id: {unit_id}"));
        }
        if let Some((byte, word)) = self.byte_order {
            fields.push(format!("byte-order: '{}'", byte_order_name(byte, word)));
        }
        if fields.is_empty() {
            base
        } else {
            format!("{base}{{{}}}", fields.join(", "))
        }
    }

    // A list's own type: a write of a scalar to it, or of a list to one element, is then
    // refused by the request builder before any I/O (ADR-0022).
    fn value_type_hint(&self) -> Option<PlcValueType> {
        Some(if self.elements.is_some() {
            PlcValueType::List
        } else {
            self.data_type.value_type()
        })
    }

    // True for every data type wider than one 16-bit register (`register_size`).
    fn spans_multiple_words(&self) -> bool {
        self.data_type.register_size() > 1
    }
}

/// Builder for [`ModbusAddress`], created by [`ModbusAddress::builder`].
///
/// Defaults: the data type is [`ModbusArea::default_data_type`] of the area (`BOOL` for
/// bit areas, `INT` for register areas), there is no unit identifier and no byte order,
/// the convention is [`AddressConvention::Plc4xCompatible`] and the address names one
/// element. Setters take effect in any order and the last call for a field wins.
/// [`build`](Self::build) is the only place validation happens.
///
/// # Examples
/// ```
/// use stonecrab::{ByteOrder, WordOrder};
/// use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType, UnitId};
///
/// let address = ModbusAddress::builder(ModbusArea::InputRegister, 99)
///     .data_type(ModbusDataType::LReal)
///     .unit_id(UnitId::default())
///     .byte_order(ByteOrder::BigEndian, WordOrder::HighWordFirst)
///     .build()?;
/// assert_eq!(address.logical_address(), 100);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug, Clone)]
#[must_use = "a builder does nothing until build() is called"]
pub struct ModbusAddressBuilder {
    area: ModbusArea,
    /// The wire offset, or for an extended register the register address.
    offset: u32,
    unit_id: Option<UnitId>,
    data_type: Option<ModbusDataType>,
    byte_order: Option<(ByteOrder, WordOrder)>,
    convention: AddressConvention,
    elements: Option<NonZeroU16>,
}

impl ModbusAddressBuilder {
    /// A builder of `area` at `offset` with every default: the offset is the wire offset,
    /// or for an extended register the register address, and [`build`](Self::build)
    /// checks it.
    const fn at(area: ModbusArea, offset: u32) -> Self {
        Self {
            area,
            offset,
            unit_id: None,
            data_type: None,
            byte_order: None,
            convention: AddressConvention::Plc4xCompatible,
            elements: None,
        }
    }

    /// Set the unit identifier.
    ///
    /// Every `u8` is accepted, including [`UnitId::BROADCAST`] and
    /// [`UnitId::TCP_NO_UNIT`].
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-CFG-UNIT-ID
    /// Corresponds to the `unit-id` key of the upstream address block.
    pub const fn unit_id(mut self, unit_id: UnitId) -> Self {
        self.unit_id = Some(unit_id);
        self
    }

    /// Set the data type, replacing the area default.
    pub const fn data_type(mut self, data_type: ModbusDataType) -> Self {
        self.data_type = Some(data_type);
        self
    }

    /// Set the byte and word order of multi-byte values at this address.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
    /// Corresponds to the `byte-order` key of the upstream address block, which ranks
    /// below the tag field and above the connection default.
    pub const fn byte_order(mut self, byte_order: ByteOrder, word_order: WordOrder) -> Self {
        self.byte_order = Some((byte_order, word_order));
        self
    }

    /// Record a convention on the address; it does not change the offset.
    pub const fn convention(mut self, convention: AddressConvention) -> Self {
        self.convention = convention;
        self
    }

    /// Make the address a list of `count` consecutive elements starting at the offset, as
    /// the range selection `[0..count-1]` does; a count of one is a list of one.
    ///
    /// # Examples
    /// ```
    /// use std::num::NonZeroU16;
    /// use stonecrab::ProtocolAddress;
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType};
    ///
    /// let count = NonZeroU16::new(2).ok_or("two")?;
    /// let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 0)
    ///     .data_type(ModbusDataType::Real)
    ///     .elements(count)
    ///     .build()?;
    /// assert_eq!(address.canonical(), "holding-register:1[0..1]:REAL");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-SELECTION
    /// The range of the upstream array notation, counted from the offset.
    pub const fn elements(mut self, count: NonZeroU16) -> Self {
        self.elements = Some(count);
        self
    }

    /// Validate the parts and create the address.
    ///
    /// Three checks run: the data type must fit the area, the registers or bits the address
    /// occupies must end at or before the last wire address 65535 (for an extended register,
    /// the last address 655349999, and the address may not be 0), and a list must fit one
    /// read request (2000 bits or 125 registers, and on the extended registers the 120 of
    /// one file record read). Nothing else is checked; in particular the unit identifier and
    /// byte order are always valid.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] when a bit area (coil or discrete input) is
    /// given any type other than `BOOL`, when `offset` plus the width passes the end of the
    /// 65536-address map (for example `REAL` at wire offset 65535, `LREAL` at 65533, or
    /// two `INT`s at 65535), when an extended register has address 0 or its width passes
    /// address 655349999, or when a list is wider than one read carries (2001 coils, 63
    /// `DINT`s, or 61 `DINT`s on the extended registers).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType};
    ///
    /// let top = ModbusAddress::builder(ModbusArea::HoldingRegister, 65_532)
    ///     .data_type(ModbusDataType::LReal)
    ///     .build();
    /// assert!(top.is_ok());
    ///
    /// let past = ModbusAddress::builder(ModbusArea::HoldingRegister, 65_533)
    ///     .data_type(ModbusDataType::LReal)
    ///     .build();
    /// assert_eq!(past.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-LIMIT-SPAN
    /// Upstream rejects `address + registers > 65535` on the zero-based address, which by
    /// its own arithmetic also refuses a single element at wire address 65535 (an
    /// INFERENCE, not observed by a run). Stonecrab accepts every element whose last
    /// register is 65535 or lower, so wire address 65535 is reachable. Bit areas reject
    /// non-`BOOL` types here instead of at read time
    /// (docs/parity/modbus.md#MB-ADDR-BIT-TYPES). An offset above 65535 cannot be
    /// expressed (docs/parity/modbus.md#MB-LIMIT-ADDR). The width of a list is measured in
    /// bits or registers, as upstream measures it at parse time
    /// (docs/parity/modbus.md#MB-LIMIT-COILS, docs/parity/modbus.md#MB-LIMIT-REGISTERS). An
    /// extended register has no one-based shift, its address 0 is refused as upstream refuses
    /// it, and its limit is docs/parity/modbus.md#MB-LIMIT-EXT; a list there must fit one
    /// file record read, 120 registers where upstream allows 125
    /// (docs/parity/modbus.md#MB-ADDR-EXTENDED).
    pub fn build(self) -> Result<ModbusAddress> {
        let data_type = self
            .data_type
            .unwrap_or_else(|| self.area.default_data_type());
        if self.area.is_bit_area() && data_type != ModbusDataType::Bool {
            return Err(invalid_address(format!(
                "data type {} is not valid on a {}: bit areas read BOOL only \
                 (docs/parity/modbus.md#MB-ADDR-BIT-TYPES)",
                data_type.upstream_name(),
                self.area.long_name()
            )));
        }
        let element = element_width(self.area, data_type);
        let (unit, limit, row) = read_limit(self.area);
        let (count, what) = match self.elements {
            Some(count) => (
                count.get(),
                format!("a list of {count} {}", data_type.upstream_name()),
            ),
            None => (1, data_type.upstream_name().to_owned()),
        };
        let size = u32::from(element).saturating_mul(u32::from(count));
        let (offset, file) = if self.area == ModbusArea::ExtendedRegister {
            self.extended_position(&what, size)?
        } else {
            let end = self.offset.saturating_add(size);
            let offset = u16::try_from(self.offset)
                .ok()
                .filter(|_| end <= ADDRESS_SPACE);
            let Some(offset) = offset else {
                return Err(invalid_address(format!(
                    "{what} at wire offset {} spans {size} {unit}(s) and passes the last \
                     address 65535 (docs/parity/modbus.md#MB-LIMIT-SPAN)",
                    self.offset,
                )));
            };
            (offset, None)
        };
        if size > u32::from(limit) {
            return Err(invalid_address(format!(
                "{what} spans {size} {unit}s, more than the {limit} one read carries \
                 (docs/parity/modbus.md#{row})"
            )));
        }
        Ok(ModbusAddress {
            unit_id: self.unit_id,
            area: self.area,
            offset,
            file,
            data_type,
            byte_order: self.byte_order,
            convention: self.convention,
            elements: self.elements,
        })
    }
}

/// The bits or registers one element of `data_type` occupies in `area`: one bit on a bit
/// area, the type's register count on a register area.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) const fn element_width(area: ModbusArea, data_type: ModbusDataType) -> u16 {
    if area.is_bit_area() {
        1
    } else {
        data_type.register_size()
    }
}

/// What one read of `area` is counted in, the most it carries and the parity row of that
/// bound: 2000 bits, 125 registers, or on the extended registers the 120 that one file
/// record read carries (ADR-0015, S50 addendum).
const fn read_limit(area: ModbusArea) -> (&'static str, u16, &'static str) {
    match area {
        ModbusArea::Coil | ModbusArea::DiscreteInput => ("bit", MAX_READ_BITS, "MB-LIMIT-COILS"),
        ModbusArea::HoldingRegister | ModbusArea::InputRegister => {
            ("register", MAX_READ_REGISTERS, "MB-LIMIT-REGISTERS")
        }
        ModbusArea::ExtendedRegister => (
            "register",
            MAX_READ_EXTENDED_REGISTERS,
            "MB-LIMIT-REGISTERS",
        ),
    }
}

impl ModbusAddressBuilder {
    /// The record and file of the first extended register, after the checks that make the
    /// address sendable: not 0, and every one of its `size` registers at or before the
    /// last address. `what` names the type, or the list, in the message.
    fn extended_position(&self, what: &str, size: u32) -> Result<(u16, Option<NonZeroU16>)> {
        if self.offset == 0 {
            return Err(extended_zero());
        }
        // Both the first and the last register must be in a file of the wire.
        let last = self.offset.saturating_add(size).saturating_sub(1);
        match (
            ModbusAddress::split_extended(self.offset),
            ModbusAddress::split_extended(last),
        ) {
            (Some((file, record)), Some(_)) => Ok((record, Some(file))),
            _ => Err(invalid_address(format!(
                "{what} at extended-register address {} spans {size} register(s) and passes \
                 the last address {LAST_EXTENDED}, record 9999 of file 65535 \
                 (docs/parity/modbus.md#MB-LIMIT-EXT)",
                self.offset,
            ))),
        }
    }
}

/// The upstream `ModbusByteOrder` name for a ([`ByteOrder`], [`WordOrder`]) pair.
///
/// Total over all four combinations; the derivation is the table on [`ByteOrder`].
const fn byte_order_name(byte: ByteOrder, word: WordOrder) -> &'static str {
    match (byte, word) {
        (ByteOrder::BigEndian, WordOrder::HighWordFirst) => "BIG_ENDIAN",
        (ByteOrder::LittleEndian, WordOrder::LowWordFirst) => "LITTLE_ENDIAN",
        (ByteOrder::LittleEndian, WordOrder::HighWordFirst) => "BIG_ENDIAN_BYTE_SWAP",
        (ByteOrder::BigEndian, WordOrder::LowWordFirst) => "LITTLE_ENDIAN_BYTE_SWAP",
    }
}

fn invalid_address(message: String) -> PlcError {
    PlcError::new(ErrorKind::InvalidAddress, message)
}

// ---- text grammar: ModbusAddress::parse and its scanner ----

/// The five areas, in prefix-digit order.
const AREAS: [ModbusArea; 5] = [
    ModbusArea::Coil,
    ModbusArea::DiscreteInput,
    ModbusArea::InputRegister,
    ModbusArea::HoldingRegister,
    ModbusArea::ExtendedRegister,
];

/// The four (byte order, word order) pairs, each with its upstream mode name derived by
/// the same function that renders `canonical()`.
const BYTE_ORDERS: [(ByteOrder, WordOrder); 4] = [
    (ByteOrder::BigEndian, WordOrder::HighWordFirst),
    (ByteOrder::LittleEndian, WordOrder::LowWordFirst),
    (ByteOrder::LittleEndian, WordOrder::HighWordFirst),
    (ByteOrder::BigEndian, WordOrder::LowWordFirst),
];

/// Most digits in the number of a long form (`\d{1,9}` upstream).
const MAX_LONG_DIGITS: usize = 9;

/// The last wire offset.
const LAST_WIRE: u32 = 65_535;

/// The area, the number as written and the unparsed remainder of the address head.
type Located<'a> = (ModbusArea, u32, &'a str);

impl ModbusAddress {
    /// Longest address text `parse` looks at, in bytes. Longer input is rejected before
    /// any scanning.
    pub const MAX_TEXT_BYTES: usize = 256;

    /// Parse address text under the PLC4X-compatible grammar, or the convention named in
    /// `options`.
    ///
    /// Accepted forms (everything is case-sensitive and the digits are decimal):
    ///
    /// | Form | Example | Needs a convention |
    /// | --- | --- | --- |
    /// | long, `AREA:N` or `AREA:N:TYPE` | `holding-register:16:INT` | no |
    /// | prefixed short, `0x`, `1x`, `3x`, `4x`, `6x` plus 4 or 5 digits | `4x00016:INT` | no |
    /// | bare short, prefix digit `0`, `1`, `3`, `4`, `6` plus 4 or 5 digits | `40016:INT` | yes |
    ///
    /// Every form may carry a selection between the number and the type, in the upstream
    /// array notation (ADR-0022):
    ///
    /// | Selection | Example | Reads |
    /// | --- | --- | --- |
    /// | `[i]` | `holding-register:1[4]:DINT` | element `i`, a scalar |
    /// | `[lo..hi]`, inclusive | `holding-register:1[0..3]:INT` | elements `lo` to `hi`, a list, even for `[4..4]` |
    /// | `;base` inside either | `holding-register:1[4..7;1]:INT` | indices counted from a declared lower bound, default 0 |
    ///
    /// The number names element 0 (with `;base`, element `base`), and element `k` starts
    /// `k` bits further on a coil or discrete input and `k` times
    /// [`ModbusDataType::register_size`] registers further on a register area, the
    /// extended registers included; so `holding-register:1[4]:DINT` is wire offset 8,
    /// `extended-register:1[4]:DINT` is extended address 9, and `[4..7;1]` reads the
    /// elements at offsets 3 to 6. A range is one address with
    /// [`elements`](Self::elements) set: it is read and written as one
    /// [`PlcValue::List`](crate::PlcValue::List), must end at or before wire address 65535
    /// (on the extended registers, address 655349999) and must fit one read (2000 bits or
    /// 125 registers, and on the extended registers 120). An index is resolved here and
    /// gives the same address as the element's own number.
    ///
    /// `AREA` is `coil`, `discrete-input`, `holding-register`, `input-register` or
    /// `extended-register`; the long form takes 1 to 9 digits. With no convention, `plc4x`
    /// or `one-based-40001` the number is the one-based logical address: the wire offset is
    /// the number minus one, so `holding-register:16` reads wire address 15 and the numbers
    /// run from 1 to 65536. With `zero-based` the number is the wire offset itself, from 0
    /// to 65535, so `holding-register:16` reads wire address 16 and the bare `40016` too.
    /// The rule is the same for every form; see [`AddressConvention`] for the table. The
    /// extended registers (`extended-register:N`, `6xNNNNN`, the bare `6NNNNN`) are the
    /// exception: the number is the register address under every convention, 1 to
    /// 655349999, with no shift, and the convention only decides whether the bare form is
    /// accepted. When the type is omitted it is `BOOL` for the bit areas and `INT` for the
    /// register areas. `TYPE` is one of the 15 upper-case names of
    /// [`ModbusDataType`]. An address may end with a block
    /// `{unit-id: N, byte-order: 'NAME'}` holding either key or both, in any order:
    /// `unit-id` is a decimal integer from 0 to 255 and `byte-order` a quoted (single or
    /// double quotes) `BIG_ENDIAN`, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP` or
    /// `LITTLE_ENDIAN_BYTE_SWAP`. Spaces are allowed around the keys, colons, values and
    /// commas inside the braces and nowhere else; the brace follows the type directly. An
    /// empty block `{}` is accepted and sets nothing.
    ///
    /// The convention is read from `options`: `None` means none was supplied, `plc4x`,
    /// `one-based-40001` and `zero-based` are the names accepted
    /// ([`AddressConvention::from_name`]) and anything else is an error even for
    /// unambiguous spellings, so a misspelt option never goes unnoticed. The returned
    /// address always carries [`AddressConvention::Plc4xCompatible`]: the convention
    /// decides what the text means, and the address is the wire offset that results.
    /// [`canonical`] renders the one-based long form, which this function reads back to an
    /// equal address with no convention, `plc4x` or `one-based-40001`, so
    /// `parse(canonical(a))` equals `a` there; with `zero-based` the same text is one
    /// register higher.
    ///
    /// [`canonical`]: crate::ProtocolAddress::canonical
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`](crate::ErrorKind::InvalidAddress)
    /// with a message naming the reason, for: a convention name that is not one of the
    /// three; empty input, input over [`MAX_TEXT_BYTES`](Self::MAX_TEXT_BYTES) bytes or
    /// input with a control character; an unrecognised area or prefix; a bare short form when no
    /// convention was supplied (the message names the ambiguity and the two always
    /// accepted spellings); a missing, overlong (more than 9 digits in the long form,
    /// other than 4 or 5 in a short form) or hexadecimal number; logical address 0 or
    /// above 65536 (wire offset above 65535), or under `zero-based` a number above 65535;
    /// a lowercase or unknown type name; any type other than `BOOL` on a coil or discrete
    /// input; a span that passes wire address 65535 (`LREAL` at logical 65534, or a
    /// selection whose first or last element lies past it); a selection that is not one
    /// `[index]` or `[lo..hi]` with an optional `;base` in decimal digits, has a second
    /// dimension (a second bracket group or a comma), an upper bound below its lower bound,
    /// an index below its base or a number beyond `u32`, or a range wider than one read
    /// carries (2001 bits, 126 registers, or 121 on the extended registers); `STRING`,
    /// `WSTRING`, `CHAR`, `WCHAR`, a parenthesised length and the `TIME` and `DATE` family;
    /// an extended-register number of 0 or past 655349999, or a type or selection that
    /// ends past it; the legacy `AREA:N:TYPE[n]` and `AREA:N:TYPE:n` forms (the message
    /// gives the replacement spelling when it parses, otherwise a general example); and a
    /// malformed address block (unterminated, unknown or repeated key, `unit-id` that is
    /// not an integer from 0 to 255, unquoted or unknown `byte-order`).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{AddressOptions, ErrorKind, ProtocolAddress};
    /// use stonecrab::modbus::{ModbusAddress, ModbusArea, ModbusDataType, UnitId};
    ///
    /// let none = AddressOptions::default();
    /// let address = ModbusAddress::parse("holding-register:16:INT{unit-id: 10}", &none)?;
    /// assert_eq!(address.area(), ModbusArea::HoldingRegister);
    /// assert_eq!(address.offset(), 15);
    /// assert_eq!(address.data_type(), ModbusDataType::Int);
    /// assert_eq!(address.unit_id(), Some(UnitId::from(10)));
    /// assert_eq!(address.canonical(), "holding-register:16:INT{unit-id: 10}");
    ///
    /// // The prefixed short form needs no convention.
    /// let short = ModbusAddress::parse("4x00016", &none)?;
    /// assert_eq!(short.offset(), 15);
    ///
    /// // The bare form is ambiguous and needs the convention to be explicit.
    /// let refused = ModbusAddress::parse("40016", &none);
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// let plc4x = AddressOptions::builder().convention("plc4x").build()?;
    /// assert_eq!(ModbusAddress::parse("40016", &plc4x)?, short);
    ///
    /// // A range is a list; an index is the element's own address.
    /// let range = ModbusAddress::parse("holding-register:1[0..1]:REAL", &none)?;
    /// assert_eq!(range.elements().map(|count| count.get()), Some(2));
    /// assert_eq!(range.canonical(), "holding-register:1[0..1]:REAL");
    /// let index = ModbusAddress::parse("4x00001[4]:DINT", &none)?;
    /// assert_eq!(index, ModbusAddress::parse("holding-register:9:DINT", &none)?);
    ///
    /// // On the extended registers the element offset moves the register address.
    /// let extended = ModbusAddress::parse("extended-register:1[4]:DINT", &none)?;
    /// assert_eq!(extended, ModbusAddress::parse("extended-register:9:DINT", &none)?);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
    /// Grammar rows: `MB-ADDR-LONG`, `MB-ADDR-SHORT-4X`, `MB-ADDR-CONFIG-BLOCK` and
    /// `MB-ADDR-BARE-40001`. Divergences: the bare forms need an explicit convention
    /// (docs/parity/modbus.md#MB-ADDR-BARE-40001), non-`BOOL` types on bit areas fail
    /// here rather than at read time (docs/parity/modbus.md#MB-ADDR-BIT-TYPES), a
    /// `unit-id` outside 0..=255 is rejected (docs/parity/modbus.md#MB-CFG-UNIT-ID),
    /// `4x:INT` is a typed error (docs/parity/modbus.md#MB-QUIRK-G), the extended-register
    /// forms are docs/parity/modbus.md#MB-ADDR-EXTENDED (limit
    /// docs/parity/modbus.md#MB-LIMIT-EXT) and the deferred forms are typed rejections
    /// (docs/parity/modbus.md#MB-ADDR-STRING, docs/parity/modbus.md#MB-TYPE-TIME). The
    /// selection is docs/parity/modbus.md#MB-ADDR-SELECTION and the legacy array forms
    /// docs/parity/modbus.md#MB-ADDR-LEGACY-ARRAY.
    /// Limits: docs/parity/modbus.md#MB-LIMIT-ADDR, docs/parity/modbus.md#MB-LIMIT-SPAN
    /// and docs/parity/modbus.md#MB-LIMIT-DIGITS. Quoting of the enum value follows the
    /// upstream value pattern and the whitespace rules inside the block are a Stonecrab
    /// choice; both are recorded as inferences under `MB-ADDR-CONFIG-BLOCK`.
    pub fn parse(text: &str, options: &AddressOptions) -> Result<Self> {
        let convention = check_convention(options)?;
        check_text(text)?;
        let (head, block) = match text.split_once('{') {
            Some((head, after)) => match after.strip_suffix('}') {
                Some(body) => (head, Some(body)),
                None => {
                    return Err(invalid_address(
                        "address block is not terminated: expected a closing '}' at the end \
                         (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)"
                            .to_owned(),
                    ));
                }
            },
            None => (text, None),
        };
        if head.is_empty() {
            return Err(invalid_address(
                "address before the config block is empty: expected for example \
                 'holding-register:1{unit-id: 1}'"
                    .to_owned(),
            ));
        }
        if block.is_some_and(|body| body.contains('}')) {
            return Err(invalid_address(
                "unexpected '}' inside the address block: text after the closing brace is \
                 not accepted (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)"
                    .to_owned(),
            ));
        }
        let (area, number, rest) = locate(head, convention.is_some())?;
        let (selection, rest) = split_selection(rest)?;
        let data_type = parse_type(head, rest, area, selection.is_none(), options)?;
        // An extended register is its own number under every convention. The number is
        // checked before a selection moves it, on every area.
        let start = if area == ModbusArea::ExtendedRegister {
            extended_number(number)?
        } else {
            u32::from(wire_offset(number, convention.unwrap_or_default())?)
        };
        let (start, elements) = match selection {
            Some(selection) => selection.resolve(area, start, element_width(area, data_type))?,
            None => (start, None),
        };
        let mut builder = ModbusAddressBuilder::at(area, start)
            .data_type(data_type)
            .convention(AddressConvention::Plc4xCompatible);
        if let Some(count) = elements {
            builder = builder.elements(count);
        }
        if let Some(body) = block {
            let block = parse_block(body)?;
            if let Some(unit_id) = block.unit_id {
                builder = builder.unit_id(unit_id);
            }
            if let Some((byte, word)) = block.byte_order {
                builder = builder.byte_order(byte, word);
            }
        }
        builder.build()
    }
}

/// The convention the caller named, `None` when none was supplied.
fn check_convention(options: &AddressOptions) -> Result<Option<AddressConvention>> {
    let Some(name) = options.convention() else {
        return Ok(None);
    };
    AddressConvention::from_name(name).map(Some).ok_or_else(|| {
        invalid_address(format!(
            "unknown address convention '{}': the accepted names are plc4x, one-based-40001 \
             and zero-based (docs/parity/modbus.md#MB-ADDR-CONVENTIONS)",
            excerpt(name)
        ))
    })
}

/// Reject empty, over-long and control-character input before any scanning.
fn check_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(invalid_address(
            "address text is empty: expected for example 'holding-register:1'".to_owned(),
        ));
    }
    if text.len() > ModbusAddress::MAX_TEXT_BYTES {
        return Err(invalid_address(format!(
            "address text is {} bytes, over the {} byte limit",
            text.len(),
            ModbusAddress::MAX_TEXT_BYTES
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid_address(
            "address text contains a control character".to_owned(),
        ));
    }
    Ok(())
}

/// Find the area and the number as written at the start of `head`; the remainder is the
/// (possibly empty) type part.
fn locate(head: &str, explicit: bool) -> Result<Located<'_>> {
    for area in AREAS {
        if let Some(after) = head.strip_prefix(area.long_name()) {
            return locate_long(area, after);
        }
    }
    let bytes = head.as_bytes();
    match (bytes.first(), bytes.get(1)) {
        (Some(prefix), Some(b'x')) if prefix.is_ascii_digit() => locate_short(
            prefix.saturating_sub(b'0'),
            head.get(2..).unwrap_or_default(),
        ),
        (Some(prefix), _) if prefix.is_ascii_digit() => {
            locate_bare(prefix.saturating_sub(b'0'), head, explicit)
        }
        _ => Err(unrecognised(head)),
    }
}

fn unrecognised(head: &str) -> PlcError {
    invalid_address(format!(
        "unrecognised address '{}': expected AREA:N[:TYPE] with AREA coil, discrete-input, \
         holding-register, input-register or extended-register, or a short form such as \
         4x00001 (docs/parity/modbus.md#MB-ADDR-LONG)",
        excerpt(head)
    ))
}

/// Long form: the text after `AREA` must be `:` and 1 to 9 digits.
fn locate_long(area: ModbusArea, after: &str) -> Result<Located<'_>> {
    let Some(numbers) = after.strip_prefix(':') else {
        return Err(invalid_address(format!(
            "expected ':' and an address number after '{}'",
            area.long_name()
        )));
    };
    let (digits, rest) = split_run(numbers, |c| c.is_ascii_digit());
    if digits.is_empty() {
        return Err(invalid_address(format!(
            "missing address number after '{}:' (docs/parity/modbus.md#MB-ADDR-LONG)",
            area.long_name()
        )));
    }
    if digits.len() > MAX_LONG_DIGITS {
        return Err(invalid_address(format!(
            "address number has {} digits; the long form takes 1 to {MAX_LONG_DIGITS} \
             (docs/parity/modbus.md#MB-LIMIT-DIGITS)",
            digits.len()
        )));
    }
    Ok((area, decimal_value(digits), rest))
}

/// Prefixed short form `Nx` plus 4 or 5 digits; `after` follows the `x`.
fn locate_short(digit: u8, after: &str) -> Result<Located<'_>> {
    let area = area_for_prefix(digit)?;
    let (digits, rest) = split_run(after, |c| c.is_ascii_digit());
    if digits.is_empty() {
        return Err(invalid_address(format!(
            "'{digit}x' needs 4 or 5 digits after the prefix, for example '{digit}x00001'; \
             found none (docs/parity/modbus.md#MB-QUIRK-G)"
        )));
    }
    short_digits(digits, digit)?;
    Ok((area, decimal_value(digits), rest))
}

/// Bare short form: a prefix digit plus 4 or 5 digits, accepted only with the convention.
fn locate_bare(digit: u8, head: &str, explicit: bool) -> Result<Located<'_>> {
    let area = area_for_prefix(digit)?;
    let (run, rest) = split_run(head, |c| c.is_ascii_digit());
    let digits = run.get(1..).unwrap_or_default();
    if !(4..=5).contains(&digits.len()) {
        return Err(unrecognised(head));
    }
    if !explicit {
        return Err(invalid_address(format!(
            "bare address '{run}' is ambiguous: it reads as {name} number {number} under the \
             PLC4X convention, but a bare number can equally be a zero-based wire address; \
             write '{name}:N' or '{digit}xNNNNN' (for example '{name}:1' or '{digit}x00001'), \
             or set address-convention to plc4x, one-based-40001 or zero-based to accept \
             the bare form (docs/parity/modbus.md#MB-ADDR-BARE-40001)",
            name = area.long_name(),
            number = decimal_value(digits)
        )));
    }
    Ok((area, decimal_value(digits), rest))
}

/// The area whose prefix digit is `digit`; every digit without an area is a typed
/// rejection.
fn area_for_prefix(digit: u8) -> Result<ModbusArea> {
    AREAS
        .into_iter()
        .find(|area| area.prefix_digit() == digit)
        .ok_or_else(|| unknown_prefix(digit))
}

fn unknown_prefix(digit: u8) -> PlcError {
    invalid_address(format!(
        "no area uses prefix digit {digit}: the prefixes are 0 (coil), 1 (discrete-input), \
         3 (input-register), 4 (holding-register) and 6 (extended-register) \
         (docs/parity/modbus.md#MB-ADDR-SHORT-4X)"
    ))
}

/// A short form must have exactly 4 or 5 digits.
fn short_digits(digits: &str, prefix: u8) -> Result<()> {
    if (4..=5).contains(&digits.len()) {
        Ok(())
    } else {
        Err(invalid_address(format!(
            "'{prefix}x' takes 4 or 5 digits after the prefix, found {} \
             (docs/parity/modbus.md#MB-ADDR-SHORT-4X)",
            digits.len()
        )))
    }
}

/// The number of an extended register as written: its register address under every
/// convention, of which 0 does not exist.
fn extended_number(number: u32) -> Result<u32> {
    if number == 0 {
        Err(extended_zero())
    } else {
        Ok(number)
    }
}

fn extended_zero() -> PlcError {
    invalid_address(
        "extended-register address 0 does not exist: the first is 1 \
         (docs/parity/modbus.md#MB-ADDR-EXTENDED)"
            .to_owned(),
    )
}

/// The zero-based wire offset for a number written under `convention`.
fn wire_offset(number: u32, convention: AddressConvention) -> Result<u16> {
    let (base, shown) = match convention {
        AddressConvention::ZeroBased => (0, "zero-based"),
        AddressConvention::Plc4xCompatible | AddressConvention::OneBased40001 => (1, "logical"),
    };
    if number < base {
        return Err(invalid_address(
            "logical address 0 does not exist: addresses are one-based, the first is 1 \
             (docs/parity/modbus.md#MB-LIMIT-ADDR)"
                .to_owned(),
        ));
    }
    let offset = number.saturating_sub(base);
    if offset > LAST_WIRE {
        let last = LAST_WIRE.saturating_add(base);
        return Err(invalid_address(format!(
            "{shown} address {number} is above {last}: wire offsets run from 0 to \
             {LAST_WIRE} (docs/parity/modbus.md#MB-LIMIT-ADDR)"
        )));
    }
    Ok(u16::try_from(offset).unwrap_or(u16::MAX))
}

/// Parse what follows the number and the selection: nothing, or `:TYPE`.
///
/// `head` is the whole head, used to spell the legacy-form replacement; `rest` is its
/// unparsed suffix. `legacy` is false when a selection was written, so a count after the
/// type is plain unexpected text rather than the legacy form. `options` are the caller's,
/// under which a legacy-form replacement must parse before the message offers it.
fn parse_type(
    head: &str,
    rest: &str,
    area: ModbusArea,
    legacy: bool,
    options: &AddressOptions,
) -> Result<ModbusDataType> {
    if rest.is_empty() {
        return Ok(area.default_data_type());
    }
    let Some(after) = rest.strip_prefix(':') else {
        return Err(unexpected(rest));
    };
    let (token, tail) = split_run(after, |c| c.is_ascii_alphanumeric() || c == '_');
    if token.is_empty() {
        return Err(invalid_address(
            "missing data type after ':' (docs/parity/modbus.md#MB-ADDR-LONG)".to_owned(),
        ));
    }
    let data_type = ModbusDataType::from_upstream_name(token)?;
    if tail.is_empty() {
        Ok(data_type)
    } else if tail.starts_with('(') {
        Err(invalid_address(
            "a parenthesised length is not supported: only STRING and WSTRING take one and \
             both are deferred (docs/parity/modbus.md#MB-ADDR-STRING)"
                .to_owned(),
        ))
    } else if legacy
        && (tail.starts_with('[') || tail.strip_prefix(':').is_some_and(starts_with_digit))
    {
        Err(legacy_array(head, rest, token, tail, options))
    } else {
        Err(unexpected(tail))
    }
}

/// The legacy `AREA:N:TYPE[n]` or `AREA:N:TYPE:n` form, with the migration hint: the
/// selection moved before the type and the count `n` became the range `[0..n-1]`, `[0]`
/// for a count of 0 or 1 as upstream writes it. The replacement is offered only when it
/// parses under the caller's `options`; a count that is not 1 to 9 digits, or a range that
/// is wider than one read or passes wire address 65535, gets the general example instead.
fn legacy_array(
    head: &str,
    rest: &str,
    token: &str,
    tail: &str,
    options: &AddressOptions,
) -> PlcError {
    let spelled = head.strip_suffix(rest).unwrap_or(head);
    let count = tail.trim_start_matches(['[', ':']).trim_end_matches(']');
    let replacement = (!count.is_empty()
        && count.len() <= MAX_LONG_DIGITS
        && count.bytes().all(|b| b.is_ascii_digit()))
    .then(|| {
        let range = match decimal_value(count) {
            0 | 1 => "[0]".to_owned(),
            elements => format!("[0..{}]", elements.saturating_sub(1)),
        };
        format!("{spelled}{range}:{token}")
    })
    .filter(|replacement| ModbusAddress::parse(replacement, options).is_ok());
    let hint = match replacement {
        Some(replacement) => format!("so this address is now written '{replacement}'"),
        None => "for example 'holding-register:1[0..3]:INT'".to_owned(),
    };
    invalid_address(format!(
        "legacy array form '{spelled}:{token}{}' is not supported: the array notation moved \
         before the type and a count became a range, {hint} \
         (docs/parity/modbus.md#MB-ADDR-LEGACY-ARRAY)",
        excerpt(tail)
    ))
}

/// A selection as written: how many elements after the address it starts (the index
/// minus the declared base) and, for a range, how many elements it holds.
struct Selection {
    first: u32,
    count: Option<u64>,
}

impl Selection {
    /// The address of the first element and, for a range, the element count, from the
    /// address the number of `area` names and the width of one element. The address is a
    /// wire offset, or on the extended registers a register address; a first element past
    /// the last address of the area is refused here, and the end of the span is the
    /// builder's check.
    fn resolve(
        self,
        area: ModbusArea,
        start: u32,
        width: u16,
    ) -> Result<(u32, Option<NonZeroU16>)> {
        let moved = u64::from(self.first)
            .saturating_mul(u64::from(width))
            .saturating_add(u64::from(start));
        let (last, place, row) = if area == ModbusArea::ExtendedRegister {
            (LAST_EXTENDED, "extended-register address", "MB-LIMIT-EXT")
        } else {
            (LAST_WIRE, "wire offset", "MB-LIMIT-SPAN")
        };
        let Some(first) = u32::try_from(moved).ok().filter(|first| *first <= last) else {
            return Err(invalid_address(format!(
                "the selection starts at {place} {moved}, past the last address {last} \
                 (docs/parity/modbus.md#{row})"
            )));
        };
        let count = match self.count {
            Some(count) => Some(
                u16::try_from(count)
                    .ok()
                    .and_then(NonZeroU16::new)
                    .ok_or_else(|| {
                        invalid_address(format!(
                            "the selection holds {count} elements, more than one read \
                             carries: at most 2000 bits, 125 registers or 120 extended \
                             registers (docs/parity/modbus.md#MB-ADDR-SELECTION)"
                        ))
                    })?,
            ),
            None => None,
        };
        Ok((first, count))
    }
}

/// Split a leading selection off `rest`: `[index]` or the inclusive `[lo..hi]`, either
/// with `;base`, the declared lower bound (default 0). Anything else in brackets, a second
/// bracket group or a comma (a second dimension), a range whose upper bound is below its
/// lower bound, an index below the base and a number beyond `u32` are rejected.
fn split_selection(rest: &str) -> Result<(Option<Selection>, &str)> {
    let Some(inside) = rest.strip_prefix('[') else {
        return Ok((None, rest));
    };
    let Some((body, after)) = inside.split_once(']') else {
        return Err(malformed_selection(rest));
    };
    if after.starts_with('[') || body.contains(',') {
        return Err(invalid_address(format!(
            "'{}' selects more than one dimension: a Modbus area is linear, so an address \
             takes one [index] or [lo..hi] (docs/parity/modbus.md#MB-ADDR-SELECTION)",
            excerpt(rest)
        )));
    }
    let (bounds, base) = match body.split_once(';') {
        Some((bounds, base)) => (bounds, selection_number(base, rest)?),
        None => (body, 0),
    };
    let (lo, hi) = match bounds.split_once("..") {
        Some((lo, hi)) => (
            selection_number(lo, rest)?,
            Some(selection_number(hi, rest)?),
        ),
        None => (selection_number(bounds, rest)?, None),
    };
    if let Some(hi) = hi.filter(|hi| *hi < lo) {
        return Err(invalid_address(format!(
            "the selection's upper bound {hi} is below its lower bound {lo} \
             (docs/parity/modbus.md#MB-ADDR-SELECTION)"
        )));
    }
    let Some(first) = lo.checked_sub(base) else {
        return Err(invalid_address(format!(
            "index {lo} lies below the declared lower bound {base} \
             (docs/parity/modbus.md#MB-ADDR-SELECTION)"
        )));
    };
    let count = hi.map(|hi| u64::from(hi.saturating_sub(lo)).saturating_add(1));
    Ok((Some(Selection { first, count }), after))
}

/// One number of a selection: ASCII digits whose value fits a `u32`.
fn selection_number(digits: &str, selection: &str) -> Result<u32> {
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(malformed_selection(selection));
    }
    digits
        .bytes()
        .try_fold(0_u32, |value, digit| {
            value
                .checked_mul(10)?
                .checked_add(u32::from(digit.saturating_sub(b'0')))
        })
        .ok_or_else(|| {
            invalid_address(
                "a selection number is too large to address \
                 (docs/parity/modbus.md#MB-LIMIT-DIGITS)"
                    .to_owned(),
            )
        })
}

fn malformed_selection(selection: &str) -> PlcError {
    invalid_address(format!(
        "invalid selection '{}': expected [index] or [lo..hi], either with ;base, in decimal \
         digits (docs/parity/modbus.md#MB-ADDR-SELECTION)",
        excerpt(selection)
    ))
}

/// Text that follows the address but is not a type, selection or block.
fn unexpected(rest: &str) -> PlcError {
    let hint = if rest.starts_with(|c: char| c.is_ascii_alphabetic()) {
        "; addresses are decimal, hexadecimal digits are not accepted"
    } else {
        ""
    };
    let shown = if rest.trim().is_empty() {
        format!("{rest:?} (whitespace)")
    } else {
        format!("'{}'", excerpt(rest))
    };
    invalid_address(format!(
        "unexpected text {shown} after the address number{hint} \
         (docs/parity/modbus.md#MB-ADDR-LONG)"
    ))
}

/// What an address block sets.
struct Block {
    unit_id: Option<UnitId>,
    byte_order: Option<(ByteOrder, WordOrder)>,
}

/// Parse the text between the braces: `key: value` pairs separated by commas.
fn parse_block(body: &str) -> Result<Block> {
    let mut block = Block {
        unit_id: None,
        byte_order: None,
    };
    if body.trim_matches(' ').is_empty() {
        return Ok(block);
    }
    for entry in body.split(',') {
        let Some((key, value)) = entry.split_once(':') else {
            return Err(invalid_address(format!(
                "address block entry '{}' is not 'key: value' \
                 (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)",
                excerpt(entry)
            )));
        };
        let (key, value) = (key.trim_matches(' '), value.trim_matches(' '));
        let duplicate = match key {
            "unit-id" => block.unit_id.replace(parse_unit_id(value)?).is_some(),
            "byte-order" => block.byte_order.replace(parse_byte_order(value)?).is_some(),
            other => {
                return Err(invalid_address(format!(
                    "unknown address block key '{}': the keys are unit-id and byte-order \
                     (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)",
                    excerpt(other)
                )));
            }
        };
        if duplicate {
            return Err(invalid_address(format!(
                "address block key '{key}' appears more than once \
                 (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)"
            )));
        }
    }
    Ok(block)
}

/// `unit-id` is a bare decimal integer from 0 to 255.
fn parse_unit_id(value: &str) -> Result<UnitId> {
    let reason = if value.starts_with('-') {
        "is negative"
    } else if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        "is not a decimal integer"
    } else {
        return u8::try_from(decimal_value(value))
            .ok()
            .map(UnitId::from)
            .ok_or_else(|| unit_id_error("is above 255"));
    };
    Err(unit_id_error(reason))
}

fn unit_id_error(reason: &str) -> PlcError {
    invalid_address(format!(
        "unit-id {reason}: it must be a decimal integer from 0 to 255, the one-byte MBAP \
         field (docs/parity/modbus.md#MB-CFG-UNIT-ID)"
    ))
}

/// `byte-order` is one of the four upstream mode names in single or double quotes.
fn parse_byte_order(value: &str) -> Result<(ByteOrder, WordOrder)> {
    let name = value
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
        .or_else(|| {
            value
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
        });
    let Some(name) = name else {
        return Err(invalid_address(format!(
            "byte-order value '{}' must be quoted, for example byte-order: 'LITTLE_ENDIAN' \
             (docs/parity/modbus.md#MB-ADDR-CONFIG-BLOCK)",
            excerpt(value)
        )));
    };
    byte_order_by_name(name).ok_or_else(|| {
        invalid_address(format!(
            "unknown byte-order '{}': the names are BIG_ENDIAN, LITTLE_ENDIAN, \
             BIG_ENDIAN_BYTE_SWAP and LITTLE_ENDIAN_BYTE_SWAP \
             (docs/parity/modbus.md#MB-CFG-BYTE-ORDER)",
            excerpt(name)
        ))
    })
}

/// The pair the upstream `ModbusByteOrder` mode `name` maps to (ADR-0006), or `None` for
/// a name outside the four. Shared by the address block and the connection parameter
/// `default-payload-byte-order`, so both read the same table.
#[expect(
    clippy::redundant_pub_crate,
    reason = "unreachable_pub forbids plain pub in a private module"
)]
pub(crate) fn byte_order_by_name(name: &str) -> Option<(ByteOrder, WordOrder)> {
    BYTE_ORDERS
        .into_iter()
        .find(|(byte, word)| byte_order_name(*byte, *word) == name)
}

fn starts_with_digit(text: &str) -> bool {
    text.starts_with(|c: char| c.is_ascii_digit())
}

/// Split `text` after its longest prefix whose characters satisfy `keep`.
fn split_run(text: &str, keep: impl Fn(char) -> bool) -> (&str, &str) {
    let end = text.find(|c: char| !keep(c)).unwrap_or(text.len());
    text.split_at_checked(end).unwrap_or((text, ""))
}

/// The value of a run of ASCII digits, saturating; exact for the at most 9 digits the
/// callers allow.
fn decimal_value(digits: &str) -> u32 {
    digits.bytes().fold(0_u32, |value, digit| {
        value
            .saturating_mul(10)
            .saturating_add(u32::from(digit.saturating_sub(b'0')))
    })
}

/// A bounded, control-stripped excerpt of `text` for an error message.
fn excerpt(text: &str) -> String {
    let mut chars = text.chars().filter(|c| !c.is_control());
    let mut shown: String = chars.by_ref().take(NAME_ECHO_LIMIT).collect();
    if chars.next().is_some() {
        shown.push_str("...");
    }
    shown
}
