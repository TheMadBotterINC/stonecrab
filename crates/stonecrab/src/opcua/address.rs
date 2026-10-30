//! The OPC UA tag address grammar:
//! `ns=<n>;<i|s|g|b>=<id>[range][;a=<attribute>][;TYPE][|key=value,...]`.
//!
//! The scanner never indexes, slices with `[]`, casts or does unchecked arithmetic, and every
//! rejection is an [`ErrorKind::InvalidAddress`] whose message names the reason and echoes at
//! most a bounded excerpt of the input (control characters are rejected before anything is
//! echoed).

use std::fmt::{self, Write as _};

use crate::{ErrorKind, PlcError, PlcValueType, ProtocolAddress, Result};

/// Longest address text `OpcuaAddress::parse` looks at, in bytes.
const MAX_TEXT_BYTES: usize = 8192;

/// Longest unrecognised piece of the input echoed back in an error message, in characters.
const ECHO_LIMIT: usize = 32;

/// The address a browse query of `""` or `**` stands for: the standard Objects folder.
const OBJECTS_FOLDER: &str = "ns=0;i=85";

/// The largest array index, base or bound: upstream reads them as a Java `int`.
const MAX_ARRAY_INDEX: u32 = 2_147_483_647;

/// The base64 alphabet of RFC 4648 section 4, which the standard node-id text uses.
const BASE64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The characters a backslash escapes in an identifier: itself and the three that end or
/// open a part of the address.
const ESCAPABLE: [char; 4] = ['\\', ';', '[', '|'];

/// The 27 standard attributes, by number and name (Part 6 of the OPC UA specification and the
/// upstream `AttributeId` enumeration): `a=` takes either spelling, case-sensitive.
const ATTRIBUTES: [(u32, &str, OpcuaAttribute); 27] = [
    (1, "NodeId", OpcuaAttribute::NodeId),
    (2, "NodeClass", OpcuaAttribute::NodeClass),
    (3, "BrowseName", OpcuaAttribute::BrowseName),
    (4, "DisplayName", OpcuaAttribute::DisplayName),
    (5, "Description", OpcuaAttribute::Description),
    (6, "WriteMask", OpcuaAttribute::WriteMask),
    (7, "UserWriteMask", OpcuaAttribute::UserWriteMask),
    (8, "IsAbstract", OpcuaAttribute::IsAbstract),
    (9, "Symmetric", OpcuaAttribute::Symmetric),
    (10, "InverseName", OpcuaAttribute::InverseName),
    (11, "ContainsNoLoops", OpcuaAttribute::ContainsNoLoops),
    (12, "EventNotifier", OpcuaAttribute::EventNotifier),
    (13, "Value", OpcuaAttribute::Value),
    (14, "DataType", OpcuaAttribute::DataType),
    (15, "ValueRank", OpcuaAttribute::ValueRank),
    (16, "ArrayDimensions", OpcuaAttribute::ArrayDimensions),
    (17, "AccessLevel", OpcuaAttribute::AccessLevel),
    (18, "UserAccessLevel", OpcuaAttribute::UserAccessLevel),
    (
        19,
        "MinimumSamplingInterval",
        OpcuaAttribute::MinimumSamplingInterval,
    ),
    (20, "Historizing", OpcuaAttribute::Historizing),
    (21, "Executable", OpcuaAttribute::Executable),
    (22, "UserExecutable", OpcuaAttribute::UserExecutable),
    (23, "DataTypeDefinition", OpcuaAttribute::DataTypeDefinition),
    (24, "RolePermissions", OpcuaAttribute::RolePermissions),
    (
        25,
        "UserRolePermissions",
        OpcuaAttribute::UserRolePermissions,
    ),
    (26, "AccessRestrictions", OpcuaAttribute::AccessRestrictions),
    (27, "AccessLevelEx", OpcuaAttribute::AccessLevelEx),
];

/// The 24 `;TYPE` suffix names of the upstream `OpcuaDataType` enumeration, the value type
/// each fixes (`None` for `NULL`, which means "use the type the server declares") and the
/// variant. `WORD`, `DWORD`, `LWORD` and `WSTRING` are not among them.
const DATA_TYPES: [(&str, Option<PlcValueType>, OpcuaDataType); 24] = [
    ("NULL", None, OpcuaDataType::Null),
    ("BOOL", Some(PlcValueType::Bool), OpcuaDataType::Bool),
    ("BYTE", Some(PlcValueType::U8), OpcuaDataType::Byte),
    ("SINT", Some(PlcValueType::I8), OpcuaDataType::SInt),
    ("INT", Some(PlcValueType::I16), OpcuaDataType::Int),
    ("DINT", Some(PlcValueType::I32), OpcuaDataType::DInt),
    ("LINT", Some(PlcValueType::I64), OpcuaDataType::LInt),
    ("USINT", Some(PlcValueType::U8), OpcuaDataType::USInt),
    ("UINT", Some(PlcValueType::U16), OpcuaDataType::UInt),
    ("UDINT", Some(PlcValueType::U32), OpcuaDataType::UDInt),
    ("ULINT", Some(PlcValueType::U64), OpcuaDataType::ULInt),
    ("REAL", Some(PlcValueType::F32), OpcuaDataType::Real),
    ("LREAL", Some(PlcValueType::F64), OpcuaDataType::LReal),
    (
        "TIME",
        Some(PlcValueType::SignedDuration),
        OpcuaDataType::Time,
    ),
    (
        "LTIME",
        Some(PlcValueType::SignedDuration),
        OpcuaDataType::LTime,
    ),
    ("DATE", Some(PlcValueType::Date), OpcuaDataType::Date),
    ("LDATE", Some(PlcValueType::Date), OpcuaDataType::LDate),
    (
        "TIME_OF_DAY",
        Some(PlcValueType::TimeOfDay),
        OpcuaDataType::TimeOfDay,
    ),
    (
        "LTIME_OF_DAY",
        Some(PlcValueType::TimeOfDay),
        OpcuaDataType::LTimeOfDay,
    ),
    (
        "DATE_AND_TIME",
        Some(PlcValueType::DateTime),
        OpcuaDataType::DateAndTime,
    ),
    (
        "LDATE_AND_TIME",
        Some(PlcValueType::DateTime),
        OpcuaDataType::LDateAndTime,
    ),
    ("CHAR", Some(PlcValueType::Char), OpcuaDataType::Char),
    ("WCHAR", Some(PlcValueType::Char), OpcuaDataType::WChar),
    ("STRING", Some(PlcValueType::String), OpcuaDataType::String),
];

/// The node identifier of an OPC UA address: the four identifier types a node id can carry.
///
/// A numeric identifier is always the full numeric node-id encoding (the namespace and a
/// 32-bit number), never one of the compact two-byte and four-byte forms, as upstream builds
/// it. The enum is `#[non_exhaustive]`.
///
/// # Examples
/// ```
/// use stonecrab::opcua::{OpcuaAddress, OpcuaIdentifier};
///
/// let address = OpcuaAddress::parse("ns=2;s=boiler")?;
/// assert_eq!(address.identifier(), &OpcuaIdentifier::String("boiler".to_owned()));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-IDENTIFIER-TYPES
/// `b` is a base64 `ByteString`, not upstream's two-byte numeric id (`OPCUA-DIV-BINARY-ID`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpcuaIdentifier {
    /// A numeric identifier, `i=<digits>`: an unsigned 32-bit number.
    Numeric(u32),
    /// A string identifier, `s=<text>`: any non-empty text without control characters.
    String(String),
    /// A GUID identifier, `g=<8-4-4-4-12 hex digits>`, as the 16 bytes of the OPC UA binary
    /// encoding: the first three fields little-endian, the last eight bytes in written
    /// order (the mixed-endian layout upstream builds).
    Guid([u8; 16]),
    /// An opaque identifier, `b=<base64>`: the decoded, non-empty bytes of a `ByteString`
    /// node id.
    ByteString(Vec<u8>),
}

/// A standard node attribute, the target of `a=<name or number>`.
///
/// The names and numbers are those of the upstream `AttributeId` enumeration, 1 to 27. The
/// enum is `#[non_exhaustive]`.
///
/// # Examples
/// ```
/// use stonecrab::opcua::OpcuaAttribute;
///
/// assert_eq!(OpcuaAttribute::Value.number(), 13);
/// assert_eq!(OpcuaAttribute::from_number(13), Some(OpcuaAttribute::Value));
/// assert_eq!(OpcuaAttribute::from_name("Value"), Some(OpcuaAttribute::Value));
/// ```
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-ATTRIBUTE
/// Names are case-sensitive, as upstream's `valueOf`; an unknown one is `InvalidAddress`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpcuaAttribute {
    /// Attribute 1, the node id.
    NodeId,
    /// Attribute 2, the node class.
    NodeClass,
    /// Attribute 3, the browse name.
    BrowseName,
    /// Attribute 4, the display name.
    DisplayName,
    /// Attribute 5, the description.
    Description,
    /// Attribute 6, the write mask.
    WriteMask,
    /// Attribute 7, the user write mask.
    UserWriteMask,
    /// Attribute 8, whether the node is abstract.
    IsAbstract,
    /// Attribute 9, whether a reference type is symmetric.
    Symmetric,
    /// Attribute 10, the inverse name of a reference type.
    InverseName,
    /// Attribute 11, whether a view contains no loops.
    ContainsNoLoops,
    /// Attribute 12, the event notifier flags.
    EventNotifier,
    /// Attribute 13, the value of a variable: the default of an address.
    Value,
    /// Attribute 14, the data type of a variable.
    DataType,
    /// Attribute 15, the value rank of a variable.
    ValueRank,
    /// Attribute 16, the array dimensions of a variable.
    ArrayDimensions,
    /// Attribute 17, the access level of a variable.
    AccessLevel,
    /// Attribute 18, the access level of a variable for the current user.
    UserAccessLevel,
    /// Attribute 19, the minimum sampling interval of a variable.
    MinimumSamplingInterval,
    /// Attribute 20, whether a variable keeps history.
    Historizing,
    /// Attribute 21, whether a method can be called.
    Executable,
    /// Attribute 22, whether a method can be called by the current user.
    UserExecutable,
    /// Attribute 23, the definition of a data type.
    DataTypeDefinition,
    /// Attribute 24, the role permissions.
    RolePermissions,
    /// Attribute 25, the role permissions of the current user.
    UserRolePermissions,
    /// Attribute 26, the access restrictions.
    AccessRestrictions,
    /// Attribute 27, the extended access level of a variable.
    AccessLevelEx,
}

impl OpcuaAttribute {
    /// The attribute with the given standard number, 1 to 27.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaAttribute;
    ///
    /// assert_eq!(OpcuaAttribute::from_number(1), Some(OpcuaAttribute::NodeId));
    /// assert_eq!(OpcuaAttribute::from_number(28), None);
    /// ```
    #[must_use]
    pub fn from_number(number: u32) -> Option<Self> {
        ATTRIBUTES
            .iter()
            .find(|(candidate, _, _)| *candidate == number)
            .map(|&(_, _, attribute)| attribute)
    }

    /// The attribute with the given name, matched case-sensitively.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaAttribute;
    ///
    /// assert_eq!(OpcuaAttribute::from_name("NodeId"), Some(OpcuaAttribute::NodeId));
    /// assert_eq!(OpcuaAttribute::from_name("nodeid"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        ATTRIBUTES
            .iter()
            .find(|(_, candidate, _)| *candidate == name)
            .map(|&(_, _, attribute)| attribute)
    }

    /// The standard number of the attribute, 1 to 27.
    #[must_use]
    pub fn number(self) -> u32 {
        ATTRIBUTES
            .iter()
            .find(|(_, _, attribute)| *attribute == self)
            .map_or(0, |&(number, _, _)| number)
    }

    /// The standard name of the attribute, as `a=` writes it.
    #[must_use]
    pub fn name(self) -> &'static str {
        ATTRIBUTES
            .iter()
            .find(|(_, _, attribute)| *attribute == self)
            .map_or("", |&(_, name, _)| name)
    }
}

/// The `;TYPE` suffix of an address: how the value is read or written, overriding the type
/// the server declares.
///
/// The 24 names are those of the upstream `OpcuaDataType` enumeration. `WORD`, `DWORD`,
/// `LWORD` and `WSTRING` cannot be named. An address without a suffix has the type `NULL`,
/// which means "use the server-declared type". The enum is `#[non_exhaustive]`.
///
/// # Examples
/// ```
/// use stonecrab::PlcValueType;
/// use stonecrab::opcua::OpcuaDataType;
///
/// assert_eq!(OpcuaDataType::from_name("dint"), Some(OpcuaDataType::DInt));
/// assert_eq!(OpcuaDataType::DInt.name(), "DINT");
/// assert_eq!(OpcuaDataType::DInt.value_type(), Some(PlcValueType::I32));
/// assert_eq!(OpcuaDataType::Null.value_type(), None);
/// ```
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-DATATYPE
/// Names are upper-cased before they are matched, as upstream does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OpcuaDataType {
    /// `NULL`: no override.
    Null,
    /// `BOOL`.
    Bool,
    /// `BYTE`, an unsigned 8-bit integer.
    Byte,
    /// `SINT`, a signed 8-bit integer.
    SInt,
    /// `INT`, a signed 16-bit integer.
    Int,
    /// `DINT`, a signed 32-bit integer.
    DInt,
    /// `LINT`, a signed 64-bit integer.
    LInt,
    /// `USINT`, an unsigned 8-bit integer.
    USInt,
    /// `UINT`, an unsigned 16-bit integer.
    UInt,
    /// `UDINT`, an unsigned 32-bit integer.
    UDInt,
    /// `ULINT`, an unsigned 64-bit integer.
    ULInt,
    /// `REAL`, a 32-bit float.
    Real,
    /// `LREAL`, a 64-bit float.
    LReal,
    /// `TIME`.
    Time,
    /// `LTIME`.
    LTime,
    /// `DATE`.
    Date,
    /// `LDATE`.
    LDate,
    /// `TIME_OF_DAY`.
    TimeOfDay,
    /// `LTIME_OF_DAY`.
    LTimeOfDay,
    /// `DATE_AND_TIME`.
    DateAndTime,
    /// `LDATE_AND_TIME`.
    LDateAndTime,
    /// `CHAR`.
    Char,
    /// `WCHAR`.
    WChar,
    /// `STRING`.
    String,
}

impl OpcuaDataType {
    /// The data type a `;TYPE` suffix names, matched after upper-casing.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaDataType;
    ///
    /// assert_eq!(OpcuaDataType::from_name("LREAL"), Some(OpcuaDataType::LReal));
    /// assert_eq!(OpcuaDataType::from_name("WORD"), None);
    /// ```
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        let upper = name.to_ascii_uppercase();
        DATA_TYPES
            .iter()
            .find(|(candidate, _, _)| *candidate == upper)
            .map(|&(_, _, data_type)| data_type)
    }

    /// The upper-case name of the suffix.
    #[must_use]
    pub fn name(self) -> &'static str {
        DATA_TYPES
            .iter()
            .find(|(_, _, data_type)| *data_type == self)
            .map_or("", |&(name, _, _)| name)
    }

    /// The core value type this suffix fixes, or `None` for `NULL`.
    #[must_use]
    pub fn value_type(self) -> Option<PlcValueType> {
        DATA_TYPES
            .iter()
            .find(|(_, _, data_type)| *data_type == self)
            .and_then(|&(_, value_type, _)| value_type)
    }
}

/// One dimension of an array suffix: the written bounds and the declared lower bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Dimension {
    lower: u32,
    upper: u32,
    base: u32,
    /// Whether it was written as a range (`[1..1]` is an array of one, `[1]` is one element).
    range: bool,
}

/// A validated OPC UA address: a node id, the attribute, an optional array selection, a type
/// override and an optional key-value tail.
///
/// The text form is
/// `ns=<n>;<i|s|g|b>=<id>[range]...[;a=<attribute>][;TYPE][|key=value,...]`, for example
/// `ns=2;s=boiler`, `ns=2;i=10846;a=DisplayName`, `ns=2;s=samples[3..8;1];DINT` and
/// `ns=0;i=2253|EventId=1,Message=1`:
///
/// - `ns=<n>` is the namespace index, plain decimal digits from 0 to 65535 (the 16 bits the
///   wire carries).
/// - The identifier type letter is `i` (a 32-bit number), `s` (text), `g` (a GUID, 8-4-4-4-12
///   hexadecimal digits) or `b` (a `ByteString` in base64 with padding). The identifier is
///   never empty.
/// - The identifier ends at the first `;` or `|` outside brackets. A bracketed segment may
///   hold either, so an array suffix can carry a `;base`. A backslash before `\`, `;`, `[` or
///   `|` makes that character part of a string identifier; a backslash before anything else
///   is itself. `canonical()` always writes those four characters escaped.
/// - One or more bracket groups at the very end of the identifier, each of `index`,
///   `lower..upper` or either with `;base` (several dimensions in one group are
///   comma-separated), select array elements. They become an OPC UA `IndexRange`, 0-based and
///   inclusive, see [`index_range`](Self::index_range). Brackets that are not such groups
///   stay in a string identifier.
/// - `a=` takes an attribute by name (case-sensitive) or by number; the default is `Value`.
/// - `;TYPE` takes one of the 24 [`OpcuaDataType`] names in any case; the default is `NULL`.
/// - `|key=value,...` is the tail that carries the field names of an event subscription,
///   kept in written order. Keys are letters, `-` and `_`; values add digits.
///
/// The whole text, and its canonical form, is at most [`OpcuaAddress::MAX_TEXT_BYTES`] bytes
/// and has no control characters. `canonical()` renders the same grammar with the defaults omitted, so
/// `parse(canonical(a))` equals `a`.
///
/// # Examples
/// ```
/// use stonecrab::{PlcValueType, ProtocolAddress};
/// use stonecrab::opcua::{OpcuaAddress, OpcuaAttribute, OpcuaIdentifier};
///
/// let address = OpcuaAddress::parse("ns=2;s=samples[3..8;1];dint")?;
/// assert_eq!(address.namespace(), 2);
/// assert_eq!(address.identifier(), &OpcuaIdentifier::String("samples".to_owned()));
/// assert_eq!(address.attribute(), OpcuaAttribute::Value);
/// assert_eq!(address.index_range().as_deref(), Some("2:7"));
/// assert_eq!(address.protocol(), "opcua");
/// assert_eq!(address.canonical(), "ns=2;s=samples[3..8;1];DINT");
/// assert_eq!(address.value_type_hint(), Some(PlcValueType::I32));
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-ROUND-TRIP
/// Mirrors the upstream tag grammar. The differences (a bounded namespace, `b` as base64,
/// escapes for `;`, `[`, `|` and `\`, a key-value tail that needs no type before it) are
/// recorded in the rows `OPCUA-ADDR-*`, `OPCUA-DIV-NAMESPACE-RANGE`, `OPCUA-DIV-BINARY-ID`
/// and `OPCUA-DIV-ADDRESS-ESCAPES`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OpcuaAddress {
    namespace: u16,
    identifier: OpcuaIdentifier,
    dimensions: Vec<Dimension>,
    attribute: OpcuaAttribute,
    data_type: OpcuaDataType,
    config: Vec<(String, String)>,
}

impl OpcuaAddress {
    /// Longest address text accepted by [`parse`](Self::parse), in bytes.
    pub const MAX_TEXT_BYTES: usize = MAX_TEXT_BYTES;

    /// Parse the text form of a tag address.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`], with a message naming the reason, for: empty
    /// input; input, or its canonical form, over [`MAX_TEXT_BYTES`](Self::MAX_TEXT_BYTES)
    /// bytes; a control character anywhere; a missing `ns=` or identifier type; a namespace that is not plain
    /// digits from 0 to 65535 (a sign included); an empty identifier; a numeric identifier
    /// above `u32::MAX`; a GUID that is not 8-4-4-4-12 hexadecimal digits; a `b` identifier
    /// that is not base64 with padding; an unterminated `[`; an array suffix whose bounds
    /// are reversed, below its base or above 2147483647; an unknown attribute name or
    /// number; a data type name outside the 24; text after the data type; and a key-value
    /// tail with an empty or misspelled key or value.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::opcua::OpcuaAddress;
    ///
    /// assert!(OpcuaAddress::parse("ns=65535;i=1").is_ok());
    /// for refused in ["", "i=1", "ns=65536;i=1", "ns=+1;i=1", "ns=1;s=", "ns=1;i=1;WORD"] {
    ///     assert_eq!(
    ///         OpcuaAddress::parse(refused).map_err(|e| e.kind()),
    ///         Err(ErrorKind::InvalidAddress),
    ///         "{refused:?}"
    ///     );
    /// }
    /// ```
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-IDENTIFIER
    /// `;` ends the identifier unless it is inside brackets; `\;` writes one.
    pub fn parse(text: &str) -> Result<Self> {
        check_text(text)?;
        let Some(rest) = text.strip_prefix("ns=") else {
            return Err(invalid_address(format!(
                "address needs the form ns=<n>;<i|s|g|b>=<identifier>, for example ns=2;s=boiler; \
                 got '{}'",
                excerpt(text)
            )));
        };
        let Some((namespace_text, rest)) = rest.split_once(';') else {
            return Err(invalid_address(
                "address needs ';' and an identifier after the namespace, for example \
                 ns=2;s=boiler"
                    .to_owned(),
            ));
        };
        let namespace = parse_namespace(namespace_text)?;
        let Some((kind, rest)) = identifier_kind(rest) else {
            return Err(invalid_address(format!(
                "identifier type must be i, s, g or b followed by '=', got '{}'",
                excerpt(rest)
            )));
        };
        let (segments, rest) = scan_identifier(rest)?;
        let (identifier_text, dimensions) = split_array_suffix(&segments)?;
        let identifier = parse_identifier(kind, &identifier_text)?;
        let (attribute, data_type, config) = parse_tail(rest)?;
        let address = Self {
            namespace,
            identifier,
            dimensions,
            attribute,
            data_type,
            config,
        };
        // The canonical text can be longer than the input (escapes, a spelled-out attribute),
        // and it must parse back, so it obeys the same bound.
        let canonical_len = address.to_string().len();
        if canonical_len > MAX_TEXT_BYTES {
            return Err(invalid_address(format!(
                "address would be {canonical_len} bytes in canonical form; at most \
                 {MAX_TEXT_BYTES} are accepted"
            )));
        }
        Ok(address)
    }

    /// Parse a browse query: a node address to start from, an empty (or blank) query, or
    /// `**`. The last two mean the Objects folder, `ns=0;i=85`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] for any other text that
    /// [`parse`](Self::parse) refuses.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::{OpcuaAddress, OpcuaIdentifier};
    ///
    /// let objects = OpcuaAddress::parse_query("**")?;
    /// assert_eq!(objects.namespace(), 0);
    /// assert_eq!(objects.identifier(), &OpcuaIdentifier::Numeric(85));
    /// assert_eq!(OpcuaAddress::parse_query("")?, objects);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-QUERY
    /// The query is the address of the node to browse from, as upstream.
    pub fn parse_query(text: &str) -> Result<Self> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed == "**" {
            return Self::parse(OBJECTS_FOLDER);
        }
        Self::parse(text)
    }

    /// The namespace index of the node id.
    #[must_use]
    pub const fn namespace(&self) -> u16 {
        self.namespace
    }

    /// The identifier of the node id.
    #[must_use]
    pub const fn identifier(&self) -> &OpcuaIdentifier {
        &self.identifier
    }

    /// The attribute to read or write: `Value` unless `a=` names another.
    #[must_use]
    pub const fn attribute(&self) -> OpcuaAttribute {
        self.attribute
    }

    /// The `;TYPE` override: [`OpcuaDataType::Null`] when the address has none.
    #[must_use]
    pub const fn data_type(&self) -> OpcuaDataType {
        self.data_type
    }

    /// The OPC UA `IndexRange` of the array suffix, or `None` when the address names the
    /// whole node.
    ///
    /// The range is 0-based and inclusive, one entry per dimension, comma-separated: the
    /// suffix `[3..8;1]` is `"2:7"` (elements 3 to 8 of an array declared from 1), `[4]` is
    /// `"4"`, and `[1..2][3..4]` is `"1:2,3:4"`. A single element is `n`, never `n:n`.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaAddress;
    ///
    /// let address = OpcuaAddress::parse("ns=2;s=m[1..2][3..4]")?;
    /// assert_eq!(address.index_range().as_deref(), Some("1:2,3:4"));
    /// assert_eq!(OpcuaAddress::parse("ns=2;s=m")?.index_range(), None);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-INDEX-RANGE
    /// The translation of upstream's `toOpcuaIndexRange`.
    #[must_use]
    pub fn index_range(&self) -> Option<String> {
        if self.dimensions.is_empty() {
            return None;
        }
        let mut range = String::new();
        for (position, dimension) in self.dimensions.iter().enumerate() {
            if position > 0 {
                range.push(',');
            }
            // `lower` and `upper` are never below `base`: parsing refuses it.
            let low = dimension.lower.saturating_sub(dimension.base);
            let high = dimension.upper.saturating_sub(dimension.base);
            range.push_str(&low.to_string());
            if low != high {
                range.push(':');
                range.push_str(&high.to_string());
            }
        }
        Some(range)
    }

    /// The key-value tail in written order, empty when the address has none.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaAddress;
    ///
    /// let address = OpcuaAddress::parse("ns=0;i=2253|EventId=1,Message=1")?;
    /// assert_eq!(address.config().len(), 2);
    /// assert_eq!(address.config().first().map(|(key, _)| key.as_str()), Some("EventId"));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-CONFIG
    /// A repeated key keeps its first position and takes the last value, as a map does.
    #[must_use]
    pub fn config(&self) -> &[(String, String)] {
        &self.config
    }

    /// The names of the event fields an event subscription selects: the keys of the tail,
    /// in written order.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::opcua::OpcuaAddress;
    ///
    /// let address = OpcuaAddress::parse("ns=0;i=2253|EventId=1,Message=1")?;
    /// assert_eq!(address.event_field_names().collect::<Vec<_>>(), ["EventId", "Message"]);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-SUB-EVENT-MAPPING
    /// Fields are mapped by position against these names, which upstream takes from the keys
    /// of an unordered map.
    pub fn event_field_names(&self) -> impl Iterator<Item = &str> {
        self.config.iter().map(|(key, _)| key.as_str())
    }
}

impl ProtocolAddress for OpcuaAddress {
    fn protocol(&self) -> &'static str {
        "opcua"
    }

    fn canonical(&self) -> String {
        self.to_string()
    }

    fn value_type_hint(&self) -> Option<PlcValueType> {
        self.data_type.value_type()
    }
}

impl fmt::Display for OpcuaAddress {
    /// The canonical text, the same as [`ProtocolAddress::canonical`].
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ns={};", self.namespace)?;
        match &self.identifier {
            OpcuaIdentifier::Numeric(number) => write!(f, "i={number}")?,
            OpcuaIdentifier::String(text) => {
                f.write_str("s=")?;
                for c in text.chars() {
                    if ESCAPABLE.contains(&c) {
                        f.write_char('\\')?;
                    }
                    f.write_char(c)?;
                }
            }
            OpcuaIdentifier::Guid(bytes) => write!(f, "g={}", guid_text(*bytes))?,
            OpcuaIdentifier::ByteString(bytes) => write!(f, "b={}", base64_encode(bytes))?,
        }
        for dimension in &self.dimensions {
            write!(f, "[{}", dimension.lower)?;
            if dimension.range {
                write!(f, "..{}", dimension.upper)?;
            }
            if dimension.base != 0 {
                write!(f, ";{}", dimension.base)?;
            }
            f.write_char(']')?;
        }
        if self.attribute != OpcuaAttribute::Value {
            write!(f, ";a={}", self.attribute.name())?;
        }
        if self.data_type != OpcuaDataType::Null {
            write!(f, ";{}", self.data_type.name())?;
        }
        for (position, (key, value)) in self.config.iter().enumerate() {
            f.write_char(if position == 0 { '|' } else { ',' })?;
            write!(f, "{key}={value}")?;
        }
        Ok(())
    }
}

/// The identifier type letter of an address.
#[derive(Clone, Copy)]
enum Kind {
    Numeric,
    String,
    Guid,
    ByteString,
}

/// The three written parts of one array dimension, `lower[..upper][;base]`, as digits.
type DimensionParts<'a> = (&'a str, Option<&'a str>, Option<&'a str>);

/// What follows the identifier: the attribute, the data type and the key-value pairs.
type Tail = (OpcuaAttribute, OpcuaDataType, Vec<(String, String)>);

/// One piece of a scanned identifier: a character that stands for itself (escaped or
/// plain), or the content of an unescaped `[...]` group, with its dimensions when the
/// content is an array selection.
enum Segment<'a> {
    Literal(char),
    Group {
        content: &'a str,
        dimensions: Option<Vec<DimensionParts<'a>>>,
    },
}

fn invalid_address(message: String) -> PlcError {
    PlcError::new(ErrorKind::InvalidAddress, message)
}

/// A bounded prefix of an unrecognised piece of the (control-free) input.
fn excerpt(text: &str) -> String {
    text.chars().take(ECHO_LIMIT).collect()
}

/// Empty, overlong and control-bearing input are rejected before any scanning.
fn check_text(text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(invalid_address("address is empty".to_owned()));
    }
    if text.len() > MAX_TEXT_BYTES {
        return Err(invalid_address(format!(
            "address is {} bytes; at most {MAX_TEXT_BYTES} are accepted",
            text.len()
        )));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid_address(
            "address contains a control character".to_owned(),
        ));
    }
    Ok(())
}

/// Plain ASCII digits only: no sign, no space, no other script's digits.
fn all_digits(text: &str) -> bool {
    text.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_namespace(text: &str) -> Result<u16> {
    if text.is_empty() || !all_digits(text) {
        return Err(invalid_address(format!(
            "namespace index must be plain decimal digits from 0 to 65535, got '{}'",
            excerpt(text)
        )));
    }
    text.parse::<u16>().map_err(|source| {
        invalid_address(format!(
            "namespace index '{}' is not in 0 to 65535, the range the node id carries",
            excerpt(text)
        ))
        .with_source(source)
    })
}

/// The identifier type letter and `=` at the start of `text`, and what follows them.
fn identifier_kind(text: &str) -> Option<(Kind, &str)> {
    [
        ("i=", Kind::Numeric),
        ("s=", Kind::String),
        ("g=", Kind::Guid),
        ("b=", Kind::ByteString),
    ]
    .into_iter()
    .find_map(|(prefix, kind)| text.strip_prefix(prefix).map(|rest| (kind, rest)))
}

/// Scan the identifier: plain characters, escapes and bracket groups, up to the first `;` or
/// `|` outside brackets. Returns the pieces and the text from that character on.
fn scan_identifier(text: &str) -> Result<(Vec<Segment<'_>>, &str)> {
    let mut segments = Vec::new();
    let mut chars = text.chars();
    loop {
        let from_here = chars.as_str();
        let Some(c) = chars.next() else {
            return Ok((segments, ""));
        };
        match c {
            ';' | '|' => return Ok((segments, from_here)),
            '\\' => {
                let mut ahead = chars.clone();
                match ahead.next() {
                    Some(next) if ESCAPABLE.contains(&next) => {
                        segments.push(Segment::Literal(next));
                        chars = ahead;
                    }
                    _ => segments.push(Segment::Literal('\\')),
                }
            }
            '[' => {
                let Some((content, after)) = chars.as_str().split_once(']') else {
                    return Err(invalid_address(
                        "a '[' in the identifier has no closing ']'; write a literal one as '\\['"
                            .to_owned(),
                    ));
                };
                segments.push(Segment::Group {
                    content,
                    dimensions: array_group(content),
                });
                chars = after.chars();
            }
            other => segments.push(Segment::Literal(other)),
        }
    }
}

/// The three parts of one written dimension, when `part` has the shape (digits only).
fn dimension_parts(part: &str) -> Option<DimensionParts<'_>> {
    let (bounds, base) = match part.split_once(';') {
        Some((bounds, base)) => (bounds, Some(base)),
        None => (part, None),
    };
    let (lower, upper) = match bounds.split_once("..") {
        Some((lower, upper)) => (lower, Some(upper)),
        None => (bounds, None),
    };
    let numeric = |digits: &str| !digits.is_empty() && all_digits(digits);
    (numeric(lower) && upper.is_none_or(numeric) && base.is_none_or(numeric))
        .then_some((lower, upper, base))
}

/// The dimensions of a bracket group when it is an array selection: one or more
/// comma-separated dimensions, all numeric.
fn array_group(content: &str) -> Option<Vec<DimensionParts<'_>>> {
    content.split(',').map(dimension_parts).collect()
}

fn array_index(digits: &str, written: &str) -> Result<u32> {
    digits
        .parse::<u32>()
        .ok()
        .filter(|index| *index <= MAX_ARRAY_INDEX)
        .ok_or_else(|| {
            invalid_address(format!(
                "array index '{}' in '[{}]' is above {MAX_ARRAY_INDEX}",
                excerpt(digits),
                excerpt(written)
            ))
        })
}

fn array_dimension(parts: DimensionParts<'_>, written: &str) -> Result<Dimension> {
    let (lower_text, upper_text, base_text) = parts;
    let lower = array_index(lower_text, written)?;
    let upper = match upper_text {
        Some(text) => array_index(text, written)?,
        None => lower,
    };
    let base = match base_text {
        Some(text) => array_index(text, written)?,
        None => 0,
    };
    if upper < lower {
        return Err(invalid_address(format!(
            "array range '[{}]' has its upper bound {upper} below its lower bound {lower}",
            excerpt(written)
        )));
    }
    if u64::from(upper).saturating_sub(u64::from(lower)) >= u64::from(MAX_ARRAY_INDEX) {
        return Err(invalid_address(format!(
            "array range '[{}]' spans more elements than can be counted",
            excerpt(written)
        )));
    }
    if lower < base {
        return Err(invalid_address(format!(
            "array index {lower} in '[{}]' lies below the declared lower bound {base}",
            excerpt(written)
        )));
    }
    Ok(Dimension {
        lower,
        upper,
        base,
        range: upper_text.is_some(),
    })
}

/// Split the trailing run of array groups off the scanned identifier. Returns the identifier
/// text (groups that are not array selections stay in it as written) and the dimensions.
fn split_array_suffix(segments: &[Segment<'_>]) -> Result<(String, Vec<Dimension>)> {
    let start = segments
        .iter()
        .rposition(|segment| {
            !matches!(
                segment,
                Segment::Group {
                    dimensions: Some(_),
                    ..
                }
            )
        })
        .map_or(0, |last| last.saturating_add(1));
    let mut identifier = String::new();
    let mut dimensions = Vec::new();
    for (position, segment) in segments.iter().enumerate() {
        match segment {
            Segment::Literal(c) => identifier.push(*c),
            Segment::Group { content, .. } if position < start => {
                identifier.push('[');
                identifier.push_str(content);
                identifier.push(']');
            }
            Segment::Group {
                content,
                dimensions: parts,
            } => {
                for part in parts.iter().flatten() {
                    dimensions.push(array_dimension(*part, content)?);
                }
            }
        }
    }
    Ok((identifier, dimensions))
}

fn parse_identifier(kind: Kind, text: &str) -> Result<OpcuaIdentifier> {
    if text.is_empty() {
        return Err(invalid_address("the identifier is empty".to_owned()));
    }
    match kind {
        Kind::String => Ok(OpcuaIdentifier::String(text.to_owned())),
        Kind::Numeric => all_digits(text)
            .then(|| text.parse::<u32>().ok())
            .flatten()
            .map(OpcuaIdentifier::Numeric)
            .ok_or_else(|| {
                invalid_address(format!(
                    "a numeric identifier must be plain digits from 0 to 4294967295, got '{}'",
                    excerpt(text)
                ))
            }),
        Kind::Guid => parse_guid(text).map(OpcuaIdentifier::Guid).ok_or_else(|| {
            invalid_address(format!(
                "a GUID identifier must be 8-4-4-4-12 hexadecimal digits, got '{}'",
                excerpt(text)
            ))
        }),
        Kind::ByteString => base64_decode(text)
            .map(OpcuaIdentifier::ByteString)
            .ok_or_else(|| {
                invalid_address(format!(
                    "a b identifier must be base64 with padding (a ByteString node id), got '{}'",
                    excerpt(text)
                ))
            }),
    }
}

/// The text of a GUID from the 16 bytes of its binary encoding: the first three groups are
/// little-endian there, so they are written back reversed.
fn guid_text(bytes: [u8; 16]) -> String {
    let [
        a0,
        a1,
        a2,
        a3,
        b0,
        b1,
        c0,
        c1,
        d0,
        d1,
        d2,
        d3,
        d4,
        d5,
        d6,
        d7,
    ] = bytes;
    format!(
        "{a3:02x}{a2:02x}{a1:02x}{a0:02x}-{b1:02x}{b0:02x}-{c1:02x}{c0:02x}-\
         {d0:02x}{d1:02x}-{d2:02x}{d3:02x}{d4:02x}{d5:02x}{d6:02x}{d7:02x}"
    )
}

/// The bytes of a run of hexadecimal digits of even length, most significant first.
fn hex_bytes(digits: &str) -> Option<Vec<u8>> {
    if !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let (pairs, _) = digits.as_bytes().as_chunks::<2>();
    pairs
        .iter()
        .map(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|text| u8::from_str_radix(text, 16).ok())
        })
        .collect()
}

/// A GUID as the 16 bytes of its binary encoding: the first three groups little-endian.
fn parse_guid(text: &str) -> Option<[u8; 16]> {
    let mut groups = text.split('-');
    let widths = [8, 4, 4, 4, 12];
    let mut bytes = Vec::new();
    for (position, width) in widths.into_iter().enumerate() {
        let group = groups.next().filter(|group| group.len() == width)?;
        let mut group_bytes = hex_bytes(group)?;
        if position < 3 {
            group_bytes.reverse();
        }
        bytes.extend(group_bytes);
    }
    if groups.next().is_some() {
        return None;
    }
    <[u8; 16]>::try_from(bytes).ok()
}

fn base64_encode(bytes: &[u8]) -> String {
    let sextet = |index: u8| {
        BASE64
            .get(usize::from(index))
            .copied()
            .map_or('=', char::from)
    };
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let byte = |position: usize| chunk.get(position).copied().unwrap_or(0);
        let (first, second, third) = (byte(0), byte(1), byte(2));
        text.push(sextet(first >> 2));
        text.push(sextet(((first & 0b11) << 4) | (second >> 4)));
        text.push(if chunk.len() > 1 {
            sextet(((second & 0b1111) << 2) | (third >> 6))
        } else {
            '='
        });
        text.push(if chunk.len() > 2 {
            sextet(third & 0b11_1111)
        } else {
            '='
        });
    }
    text
}

/// Standard base64 with padding. `None` for any other alphabet, a misplaced or missing pad,
/// non-zero bits in the last character before the pad and an empty input, so each byte string
/// has one text (the decoded bytes must encode back to the input).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let body = text.trim_end_matches('=');
    if body.is_empty() {
        return None;
    }
    let sextets = body
        .bytes()
        .map(|byte| {
            BASE64
                .iter()
                .position(|candidate| *candidate == byte)
                .and_then(|index| u8::try_from(index).ok())
        })
        .collect::<Option<Vec<u8>>>()?;
    let mut bytes = Vec::new();
    for group in sextets.chunks(4) {
        let sextet = |position: usize| group.get(position).copied().unwrap_or(0);
        let (a, b, c, d) = (sextet(0), sextet(1), sextet(2), sextet(3));
        let decoded = [(a << 2) | (b >> 4), (b << 4) | (c >> 2), (c << 6) | d];
        bytes.extend(decoded.into_iter().take(group.len().saturating_sub(1)));
    }
    (base64_encode(&bytes) == text).then_some(bytes)
}

/// The attribute, data type and key-value tail that follow the identifier: the text is empty
/// or starts with `;` or `|`.
fn parse_tail(text: &str) -> Result<Tail> {
    let (main, config_text) = match text.split_once('|') {
        Some((main, config_text)) => (main, Some(config_text)),
        None => (text, None),
    };
    // `main` is empty or starts with ';', so the first piece is always empty.
    let mut parts = main.split(';').skip(1);
    let mut next = parts.next();
    let mut attribute = OpcuaAttribute::Value;
    if let Some(name) = next.and_then(|part| part.strip_prefix("a=")) {
        attribute = parse_attribute(name)?;
        next = parts.next();
    }
    let data_type = match next {
        Some(name) => parse_data_type(name)?,
        None => OpcuaDataType::Null,
    };
    if let Some(extra) = parts.next() {
        return Err(invalid_address(format!(
            "unexpected text '{}' after the data type",
            excerpt(extra)
        )));
    }
    let config = config_text
        .map(parse_config)
        .transpose()?
        .unwrap_or_default();
    Ok((attribute, data_type, config))
}

fn parse_attribute(text: &str) -> Result<OpcuaAttribute> {
    let found = if all_digits(text) {
        text.parse::<u32>()
            .ok()
            .and_then(OpcuaAttribute::from_number)
    } else {
        OpcuaAttribute::from_name(text)
    };
    found.ok_or_else(|| {
        invalid_address(format!(
            "unknown attribute '{}': use a standard attribute name such as Value or a number \
             from 1 to 27",
            excerpt(text)
        ))
    })
}

fn parse_data_type(text: &str) -> Result<OpcuaDataType> {
    let spelled = text.chars().all(|c| c.is_ascii_alphabetic() || c == '_');
    spelled
        .then(|| OpcuaDataType::from_name(text))
        .flatten()
        .ok_or_else(|| {
            invalid_address(format!(
                "unknown data type '{}': use one of the 24 names such as DINT, REAL or STRING",
                excerpt(text)
            ))
        })
}

/// `key=value,key=value`: keys of letters, `-` and `_`, values of those and digits. A
/// repeated key keeps its first place and takes the last value.
fn parse_config(text: &str) -> Result<Vec<(String, String)>> {
    let key_char = |c: char| c.is_ascii_alphabetic() || c == '-' || c == '_';
    let mut pairs: Vec<(String, String)> = Vec::new();
    for entry in text.split(',') {
        let valid = entry.split_once('=').filter(|(key, value)| {
            !key.is_empty()
                && key.chars().all(key_char)
                && !value.is_empty()
                && value.chars().all(|c| key_char(c) || c.is_ascii_digit())
        });
        let Some((key, value)) = valid else {
            return Err(invalid_address(format!(
                "the tail after '|' must be key=value pairs separated by commas (keys of \
                 letters, '-' and '_', values of those and digits), got '{}'",
                excerpt(entry)
            )));
        };
        match pairs.iter_mut().find(|(existing, _)| existing == key) {
            Some(pair) => value.clone_into(&mut pair.1),
            None => pairs.push((key.to_owned(), value.to_owned())),
        }
    }
    Ok(pairs)
}
