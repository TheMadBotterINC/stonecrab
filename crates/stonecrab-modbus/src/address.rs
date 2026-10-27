//! Modbus address types: unit identifier, data area, data type, convention and the
//! validated address value.
//!
//! This module holds types only. The address text parser arrives in slice S20b; until
//! then a [`ModbusAddress`] is assembled from its parts with [`ModbusAddressBuilder`].

use std::fmt;

use stonecrab_core::{
    ByteOrder, ErrorKind, PlcError, PlcValueType, ProtocolAddress, Result, WordOrder,
};

/// Number of addresses in one Modbus data area: wire offsets run from 0 to 65535.
const ADDRESS_SPACE: u32 = 65_536;

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
/// use stonecrab_modbus::UnitId;
///
/// assert_eq!(UnitId::default(), UnitId::from(1));
/// assert_eq!(UnitId::from(10).get(), 10);
/// assert_eq!(u8::from(UnitId::TCP_NO_UNIT), 255);
/// assert_eq!(UnitId::from(10).to_string(), "10");
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-CFG-UNIT-ID
/// Upstream reads the default with `default-unit-identifier` (default 1) and parses the
/// address block key `unit-id` with `Short.parseShort`; the inventoried files show no
/// range check. The 0..=255 bound here is a Stonecrab inference from the one-byte MBAP
/// field and is recorded as a divergence row. The conventional meaning of 0 and 255 is
/// listed as UNVERIFIED in the parity record because the implementation guide is not
/// retained.
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
    /// use stonecrab_modbus::UnitId;
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

/// One of the four Modbus data areas the alpha reads.
///
/// Coils and discrete inputs are bit areas (one bit per address); holding registers and
/// input registers are register areas (one 16-bit register per address). Coils and
/// holding registers are writable on the wire and the two input areas are read-only; the
/// alpha reads all four and writes none. The enum is `#[non_exhaustive]` because the
/// deferred `extended-register` area (`6x`) joins it in a later milestone.
///
/// # Examples
/// ```
/// use stonecrab_modbus::ModbusArea;
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
/// are the `0x`, `1x`, `3x` and `4x` forms (docs/parity/modbus.md#MB-ADDR-SHORT-4X).
/// There is no `2x` area upstream.
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
}

impl ModbusArea {
    /// Whether one address holds a single bit (coils and discrete inputs).
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::ModbusArea;
    /// assert!(ModbusArea::Coil.is_bit_area());
    /// assert!(!ModbusArea::InputRegister.is_bit_area());
    /// ```
    #[must_use]
    pub const fn is_bit_area(self) -> bool {
        match self {
            Self::Coil | Self::DiscreteInput => true,
            Self::HoldingRegister | Self::InputRegister => false,
        }
    }

    /// Whether the protocol allows writing the area (coils and holding registers).
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::ModbusArea;
    /// assert!(ModbusArea::Coil.is_writable());
    /// assert!(!ModbusArea::DiscreteInput.is_writable());
    /// ```
    #[must_use]
    pub const fn is_writable(self) -> bool {
        match self {
            Self::Coil | Self::HoldingRegister => true,
            Self::DiscreteInput | Self::InputRegister => false,
        }
    }

    /// The digit that starts the upstream short forms: 0, 1, 3 or 4.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::ModbusArea;
    /// assert_eq!(ModbusArea::Coil.prefix_digit(), 0);
    /// assert_eq!(ModbusArea::DiscreteInput.prefix_digit(), 1);
    /// assert_eq!(ModbusArea::InputRegister.prefix_digit(), 3);
    /// assert_eq!(ModbusArea::HoldingRegister.prefix_digit(), 4);
    /// ```
    #[must_use]
    pub const fn prefix_digit(self) -> u8 {
        match self {
            Self::Coil => 0,
            Self::DiscreteInput => 1,
            Self::InputRegister => 3,
            Self::HoldingRegister => 4,
        }
    }

    /// The long-form area keyword: `coil`, `discrete-input`, `holding-register` or
    /// `input-register`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::ModbusArea;
    /// assert_eq!(ModbusArea::DiscreteInput.long_name(), "discrete-input");
    /// ```
    #[must_use]
    pub const fn long_name(self) -> &'static str {
        match self {
            Self::Coil => "coil",
            Self::DiscreteInput => "discrete-input",
            Self::HoldingRegister => "holding-register",
            Self::InputRegister => "input-register",
        }
    }

    /// The type an address of this area has when none is written: `BOOL` for the bit
    /// areas and `INT` for the register areas.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::{ModbusArea, ModbusDataType};
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
/// use stonecrab_core::PlcValueType;
/// use stonecrab_modbus::ModbusDataType;
///
/// let kind = ModbusDataType::from_upstream_name("DINT")?;
/// assert_eq!(kind, ModbusDataType::DInt);
/// assert_eq!(kind.register_size(), 2);
/// assert_eq!(kind.value_type(), PlcValueType::I32);
/// assert_eq!(kind.upstream_name(), "DINT");
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-TYPE-BOOL
/// Rows `MB-TYPE-BOOL` to `MB-TYPE-LREAL` give the register width of each accepted type
/// and `MB-TYPE-CHAR` to `MB-TYPE-LDATE-AND-TIME` the deferred ones. Upstream resolves
/// the name with the case-sensitive `ModbusDataType.valueOf`, so lowercase names are
/// unknown here too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ModbusDataType {
    /// One bit on a bit area, one 16-bit register (0 or non-zero) on a register area.
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
    /// use stonecrab_modbus::ModbusDataType;
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
    /// use stonecrab_modbus::ModbusDataType;
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
    /// use stonecrab_core::PlcValueType;
    /// use stonecrab_modbus::ModbusDataType;
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
    /// use stonecrab_core::ErrorKind;
    /// use stonecrab_modbus::ModbusDataType;
    ///
    /// assert_eq!(ModbusDataType::from_upstream_name("REAL")?, ModbusDataType::Real);
    /// let refused = ModbusDataType::from_upstream_name("STRING");
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidAddress));
    /// # Ok::<(), stonecrab_core::PlcError>(())
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

/// How the digits of an address are interpreted.
///
/// The enum is `#[non_exhaustive]`: `ZeroBased` and `OneBased40001` conventions are
/// planned for a later milestone. The convention does not change what a parsed
/// [`ModbusAddress`] means; it records which convention the address text was read under
/// so that selection stays explicit rather than a hidden driver state.
///
/// # Examples
/// ```
/// use stonecrab_modbus::AddressConvention;
/// assert_eq!(AddressConvention::default(), AddressConvention::Plc4xCompatible);
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-ADDR-BARE-40001
/// `Plc4xCompatible` is the verified upstream grammar, in which a bare `40001` is
/// holding register 1. Upstream accepts the bare forms unconditionally; Stonecrab will
/// accept them only when the convention is supplied explicitly (divergence row of the
/// same id, implemented with the parser in slice S20b).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum AddressConvention {
    /// The Apache PLC4X grammar: one-based logical addresses, wire address = logical - 1.
    #[default]
    Plc4xCompatible,
}

/// A validated Modbus read address: unit, area, wire offset, type and byte order.
///
/// The `offset` is the zero-based wire address, the number that goes into the PDU. The
/// one-based logical address people write (`holding-register:16`, `4x00016`) is
/// `offset + 1` and is available as [`ModbusAddress::logical_address`]. Fields are
/// private and an address is created only through [`ModbusAddress::builder`], so every
/// value in existence passed the checks of [`ModbusAddressBuilder::build`].
///
/// Equality, hashing and `Clone` cover every field, including the byte order: two
/// addresses that differ only in `byte-order` are different values.
///
/// # Canonical text
/// [`ProtocolAddress::canonical`] renders the unambiguous long form
/// `AREA:LOGICAL:TYPE`, for example `holding-register:16:INT`, followed by an address
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
/// # Examples
/// ```
/// use stonecrab_core::ProtocolAddress;
/// use stonecrab_modbus::{ModbusAddress, ModbusArea, ModbusDataType};
///
/// let address = ModbusAddress::builder(ModbusArea::HoldingRegister, 15)
///     .data_type(ModbusDataType::Int)
///     .build()?;
/// assert_eq!(address.offset(), 15);
/// assert_eq!(address.logical_address(), 16);
/// assert_eq!(address.protocol(), "modbus");
/// assert_eq!(address.canonical(), "holding-register:16:INT");
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-BYTEORDER-PRECEDENCE
/// The optional byte order is the address-block source of the upstream precedence: the
/// tag field wins over it and it wins over the connection default. Byte order takes part
/// in equality, which upstream's `equals` does not do
/// (docs/parity/modbus.md#MB-QUIRK-L). The offset and span bounds are
/// docs/parity/modbus.md#MB-LIMIT-ADDR and docs/parity/modbus.md#MB-LIMIT-SPAN.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModbusAddress {
    unit_id: Option<UnitId>,
    area: ModbusArea,
    offset: u16,
    data_type: ModbusDataType,
    byte_order: Option<(ByteOrder, WordOrder)>,
    convention: AddressConvention,
}

impl ModbusAddress {
    /// Start building an address at a zero-based wire `offset` of `area`.
    ///
    /// Defaults, documented on [`ModbusAddressBuilder`]: the area's default data type, no
    /// unit identifier, no byte order and [`AddressConvention::Plc4xCompatible`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::{ModbusAddress, ModbusArea};
    /// let builder = ModbusAddress::builder(ModbusArea::Coil, 0);
    /// let address = builder.build()?;
    /// assert_eq!(address.logical_address(), 1);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    pub const fn builder(area: ModbusArea, offset: u16) -> ModbusAddressBuilder {
        ModbusAddressBuilder {
            area,
            offset,
            unit_id: None,
            data_type: None,
            byte_order: None,
            convention: AddressConvention::Plc4xCompatible,
        }
    }

    /// The unit identifier written in the address, `None` when the address names none
    /// and the connection default applies.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::{ModbusAddress, ModbusArea, UnitId};
    /// let address = ModbusAddress::builder(ModbusArea::Coil, 0)
    ///     .unit_id(UnitId::from(7))
    ///     .build()?;
    /// assert_eq!(address.unit_id(), Some(UnitId::from(7)));
    /// # Ok::<(), stonecrab_core::PlcError>(())
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

    /// The zero-based wire address that goes into the request PDU.
    #[must_use]
    pub const fn offset(&self) -> u16 {
        self.offset
    }

    /// The one-based logical address people write: the wire offset plus one, 1 to 65536.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-LONG
    /// Upstream's wire address is the logical address minus one (`PROTOCOL_ADDRESS_OFFSET`).
    #[must_use]
    pub fn logical_address(&self) -> u32 {
        u32::from(self.offset).saturating_add(1)
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

    /// The convention the address text was read under.
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
        let base = format!(
            "{}:{}:{}",
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

    fn value_type_hint(&self) -> Option<PlcValueType> {
        Some(self.data_type.value_type())
    }
}

/// Builder for [`ModbusAddress`], created by [`ModbusAddress::builder`].
///
/// Defaults: the data type is [`ModbusArea::default_data_type`] of the area (`BOOL` for
/// bit areas, `INT` for register areas), there is no unit identifier and no byte order,
/// and the convention is [`AddressConvention::Plc4xCompatible`]. Setters take effect in
/// any order and the last call for a field wins. [`build`](Self::build) is the only place
/// validation happens.
///
/// # Examples
/// ```
/// use stonecrab_core::{ByteOrder, WordOrder};
/// use stonecrab_modbus::{ModbusAddress, ModbusArea, ModbusDataType, UnitId};
///
/// let address = ModbusAddress::builder(ModbusArea::InputRegister, 99)
///     .data_type(ModbusDataType::LReal)
///     .unit_id(UnitId::default())
///     .byte_order(ByteOrder::BigEndian, WordOrder::HighWordFirst)
///     .build()?;
/// assert_eq!(address.logical_address(), 100);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
#[derive(Debug, Clone)]
#[must_use = "a builder does nothing until build() is called"]
pub struct ModbusAddressBuilder {
    area: ModbusArea,
    offset: u16,
    unit_id: Option<UnitId>,
    data_type: Option<ModbusDataType>,
    byte_order: Option<(ByteOrder, WordOrder)>,
    convention: AddressConvention,
}

impl ModbusAddressBuilder {
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

    /// Set the convention the address text was read under.
    pub const fn convention(mut self, convention: AddressConvention) -> Self {
        self.convention = convention;
        self
    }

    /// Validate the parts and create the address.
    ///
    /// Two checks run: the data type must fit the area, and the elements it occupies must
    /// end at or before the last wire address 65535. Nothing else is checked; in
    /// particular the unit identifier and byte order are always valid.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] when a bit area (coil or discrete input) is
    /// given any type other than `BOOL`, or when `offset` plus the type's
    /// [`register_size`](ModbusDataType::register_size) passes the end of the 65536-address
    /// map (for example `REAL` at wire offset 65535, or `LREAL` at 65533).
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::ErrorKind;
    /// use stonecrab_modbus::{ModbusAddress, ModbusArea, ModbusDataType};
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
    /// expressed (docs/parity/modbus.md#MB-LIMIT-ADDR).
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
        let size = data_type.register_size();
        let end = u32::from(self.offset).saturating_add(u32::from(size));
        if end > ADDRESS_SPACE {
            return Err(invalid_address(format!(
                "{} at wire offset {} spans {} register(s) and passes the last address \
                 65535 (docs/parity/modbus.md#MB-LIMIT-SPAN)",
                data_type.upstream_name(),
                self.offset,
                size
            )));
        }
        Ok(ModbusAddress {
            unit_id: self.unit_id,
            area: self.area,
            offset: self.offset,
            data_type,
            byte_order: self.byte_order,
            convention: self.convention,
        })
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
