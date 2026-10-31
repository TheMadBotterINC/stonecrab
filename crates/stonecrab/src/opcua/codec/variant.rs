//! Variant (OPC UA Part 6, 5.2.2.16): one value of any of the 25 built-in types, one array of
//! them, or nothing, behind a one-byte encoding mask.
//!
//! The mask holds the type id in its six low bits, a flag for an array of values (bit 7) and a
//! flag for array dimensions (bit 6). An array is an `Int32` count and the elements; the
//! dimensions are a second `Int32` count and as many `Int32`s, and their product must equal
//! the number of elements. A count is checked against the bytes that remain before a single
//! element is reserved (`ParseError::CountLimit`), and the elements are pushed one at a
//! time, so a declared count never reserves memory. A Variant of a Variant, of
//! a `DataValue` or of an `ExtensionObject` spends the [`Depth`] budget on every entry.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{
    DataValue, DateTime, Depth, DiagnosticInfo, ExpandedNodeId, ExtensionObject, Guid, LITTLE,
    LocalizedText, NodeId, QualifiedName, StatusCode, UaByteString, UaString,
};

/// The bit of the mask that says the Variant holds an array of values.
const ARRAY_FLAG: u8 = 0x80;
/// The bit of the mask that says array dimensions follow the value.
const DIMENSIONS_FLAG: u8 = 0x40;
/// The bits of the mask that hold the type id.
const TYPE_MASK: u8 = 0x3F;
/// The type id of a Variant that holds no value.
const NULL_TYPE: u8 = 0;
/// The count that stands for a null array.
const NULL_COUNT: i32 = -1;
/// The name of the field that holds the number of elements of an array.
const LENGTH_FIELD: &str = "arrayLength";
/// The name of the field that holds the number of array dimensions.
const DIMENSIONS_FIELD: &str = "arrayDimensions";

/// An XML element: text in the same length-prefixed form as a String, a built-in type of its
/// own (type id 16) so that a Variant keeps the kind it was read as.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct XmlElement(pub(in crate::opcua) UaString);

/// One element of a Variant: how it is read, written and measured, and the fewest bytes it takes.
trait Element: Sized {
    /// The fewest bytes a value takes: its size for a type whose values all take as many, one
    /// (the default) for the others.
    fn min_width() -> usize {
        1
    }

    fn decode(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError>;

    fn encode(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError>;

    fn encoded_size(&self, depth: Depth) -> Result<usize, ParseError>;
}

/// Numbers whose bytes the reader and the writer take with a byte order.
macro_rules! number_elements {
    ($($ty:ty, $width:literal, $read:ident, $write:ident;)*) => {$(
        impl Element for $ty {
            fn min_width() -> usize {
                $width
            }

            fn decode(reader: &mut BoundedReader<'_>, _depth: Depth) -> Result<Self, ParseError> {
                reader.$read(LITTLE)
            }

            fn encode(&self, writer: &mut BoundedWriter, _depth: Depth) -> Result<(), ParseError> {
                writer.$write(*self, LITTLE)
            }

            fn encoded_size(&self, _depth: Depth) -> Result<usize, ParseError> {
                Ok($width)
            }
        }
    )*};
}

number_elements! {
    i16, 2, read_i16, write_i16;
    u16, 2, read_u16, write_u16;
    i32, 4, read_i32, write_i32;
    u32, 4, read_u32, write_u32;
    i64, 8, read_i64, write_i64;
    u64, 8, read_u64, write_u64;
    f32, 4, read_f32, write_f32;
    f64, 8, read_f64, write_f64;
}

/// The built-in types that delegate to their own three functions.
macro_rules! builtin_elements {
    ($($ty:ty $(, $width:literal)?;)*) => {$(
        impl Element for $ty {
            $(
                fn min_width() -> usize {
                    $width
                }
            )?

            fn decode(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
                Self::parse(reader, depth)
            }

            fn encode(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
                self.serialize(writer, depth)
            }

            fn encoded_size(&self, depth: Depth) -> Result<usize, ParseError> {
                self.length_in_bytes(depth)
            }
        }
    )*};
}

builtin_elements! {
    UaString;
    UaByteString;
    DateTime, 8;
    Guid, 16;
    StatusCode, 4;
    NodeId;
    ExpandedNodeId;
    QualifiedName;
    LocalizedText;
    ExtensionObject;
    DataValue;
    Variant;
    DiagnosticInfo;
}

impl Element for u8 {
    fn decode(reader: &mut BoundedReader<'_>, _depth: Depth) -> Result<Self, ParseError> {
        reader.read_u8()
    }

    fn encode(&self, writer: &mut BoundedWriter, _depth: Depth) -> Result<(), ParseError> {
        writer.write_u8(*self)
    }

    fn encoded_size(&self, _depth: Depth) -> Result<usize, ParseError> {
        Ok(1)
    }
}

impl Element for i8 {
    fn decode(reader: &mut BoundedReader<'_>, _depth: Depth) -> Result<Self, ParseError> {
        reader.read_i8()
    }

    fn encode(&self, writer: &mut BoundedWriter, _depth: Depth) -> Result<(), ParseError> {
        writer.write_i8(*self)
    }

    fn encoded_size(&self, _depth: Depth) -> Result<usize, ParseError> {
        Ok(1)
    }
}

/// A Boolean is one byte: zero is false and any other value is true, and true is written as 1.
impl Element for bool {
    fn decode(reader: &mut BoundedReader<'_>, _depth: Depth) -> Result<Self, ParseError> {
        reader.read_u8().map(|byte| byte != 0)
    }

    fn encode(&self, writer: &mut BoundedWriter, _depth: Depth) -> Result<(), ParseError> {
        writer.write_u8(u8::from(*self))
    }

    fn encoded_size(&self, _depth: Depth) -> Result<usize, ParseError> {
        Ok(1)
    }
}

impl Element for XmlElement {
    fn decode(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
        UaString::parse(reader, depth).map(Self)
    }

    fn encode(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
        self.0.serialize(writer, depth)
    }

    fn encoded_size(&self, depth: Depth) -> Result<usize, ParseError> {
        self.0.length_in_bytes(depth)
    }
}

/// A boxed element, which the types that can contain themselves need to have a size.
impl<T: Element> Element for Box<T> {
    fn decode(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
        T::decode(reader, depth).map(Self::new)
    }

    fn encode(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
        T::encode(self, writer, depth)
    }

    fn encoded_size(&self, depth: Depth) -> Result<usize, ParseError> {
        T::encoded_size(self, depth)
    }
}

/// An `Int32` count: `None` for the null count -1, otherwise the number of elements.
fn read_count(
    reader: &mut BoundedReader<'_>,
    field: &'static str,
) -> Result<Option<usize>, ParseError> {
    let count = reader.read_i32(LITTLE)?;
    if count == NULL_COUNT {
        return Ok(None);
    }
    // A negative count other than -1 is no valid encoding; the value is its magnitude.
    match usize::try_from(count) {
        Ok(count) => Ok(Some(count)),
        Err(_) => Err(ParseError::InvalidDiscriminator {
            field,
            value: u64::from(count.unsigned_abs()),
        }),
    }
}

/// An array: its count, checked against the bytes that remain before anything is reserved,
/// and its elements. A null array is `None` and an empty one `Some` of an empty vector.
fn read_array<T: Element>(
    reader: &mut BoundedReader<'_>,
    depth: Depth,
    field: &'static str,
) -> Result<Option<Vec<T>>, ParseError> {
    let Some(count) = read_count(reader, field)? else {
        return Ok(None);
    };
    // Every element takes at least `min_width` bytes, so no more than this many are there.
    let max = reader
        .remaining()
        .checked_div(T::min_width())
        .unwrap_or_default();
    if count > max {
        return Err(ParseError::CountLimit { field, count, max });
    }
    let mut elements = Vec::new();
    for _ in 0..count {
        elements.push(T::decode(reader, depth)?);
    }
    Ok(Some(elements))
}

/// The count of an array and its elements.
fn write_array<T: Element>(
    writer: &mut BoundedWriter,
    depth: Depth,
    elements: Option<&[T]>,
) -> Result<(), ParseError> {
    let count = match elements {
        Some(elements) => i32::try_from(elements.len()).or(Err(ParseError::Overflow))?,
        None => NULL_COUNT,
    };
    writer.write_i32(count, LITTLE)?;
    for element in elements.unwrap_or_default() {
        element.encode(writer, depth)?;
    }
    Ok(())
}

/// The encoded size of an array: its count and its elements.
fn array_size<T: Element>(elements: Option<&[T]>, depth: Depth) -> Result<usize, ParseError> {
    let mut size = 4_usize;
    for element in elements.unwrap_or_default() {
        let element_size = element.encoded_size(depth)?;
        size = size.checked_add(element_size).ok_or(ParseError::Overflow)?;
    }
    Ok(size)
}

/// The error for a type id that names none of the 25 built-in types.
fn unknown_type(type_id: u8) -> ParseError {
    ParseError::InvalidDiscriminator {
        field: "VariantType",
        value: u64::from(type_id),
    }
}

/// One value of a built-in type, named after the type; the type id of each is in its order.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::opcua) enum Scalar {
    /// Type id 1.
    Boolean(bool),
    /// Type id 2.
    SByte(i8),
    /// Type id 3.
    Byte(u8),
    /// Type id 4.
    Int16(i16),
    /// Type id 5.
    UInt16(u16),
    /// Type id 6.
    Int32(i32),
    /// Type id 7.
    UInt32(u32),
    /// Type id 8.
    Int64(i64),
    /// Type id 9.
    UInt64(u64),
    /// Type id 10.
    Float(f32),
    /// Type id 11.
    Double(f64),
    /// Type id 12.
    String(UaString),
    /// Type id 13.
    DateTime(DateTime),
    /// Type id 14.
    Guid(Guid),
    /// Type id 15.
    ByteString(UaByteString),
    /// Type id 16.
    XmlElement(XmlElement),
    /// Type id 17.
    NodeId(NodeId),
    /// Type id 18.
    ExpandedNodeId(ExpandedNodeId),
    /// Type id 19.
    StatusCode(StatusCode),
    /// Type id 20.
    QualifiedName(QualifiedName),
    /// Type id 21.
    LocalizedText(LocalizedText),
    /// Type id 22.
    ExtensionObject(ExtensionObject),
    /// Type id 23.
    DataValue(Box<DataValue>),
    /// Type id 24.
    Variant(Box<Variant>),
    /// Type id 25.
    DiagnosticInfo(DiagnosticInfo),
}

/// An array of values of a built-in type, named after the type. `None` is a null array (count -1)
/// and `Some` of an empty vector an empty one.
#[derive(Debug, Clone, PartialEq)]
pub(in crate::opcua) enum Array {
    /// Type id 1.
    Boolean(Option<Vec<bool>>),
    /// Type id 2.
    SByte(Option<Vec<i8>>),
    /// Type id 3.
    Byte(Option<Vec<u8>>),
    /// Type id 4.
    Int16(Option<Vec<i16>>),
    /// Type id 5.
    UInt16(Option<Vec<u16>>),
    /// Type id 6.
    Int32(Option<Vec<i32>>),
    /// Type id 7.
    UInt32(Option<Vec<u32>>),
    /// Type id 8.
    Int64(Option<Vec<i64>>),
    /// Type id 9.
    UInt64(Option<Vec<u64>>),
    /// Type id 10.
    Float(Option<Vec<f32>>),
    /// Type id 11.
    Double(Option<Vec<f64>>),
    /// Type id 12.
    String(Option<Vec<UaString>>),
    /// Type id 13.
    DateTime(Option<Vec<DateTime>>),
    /// Type id 14.
    Guid(Option<Vec<Guid>>),
    /// Type id 15.
    ByteString(Option<Vec<UaByteString>>),
    /// Type id 16.
    XmlElement(Option<Vec<XmlElement>>),
    /// Type id 17.
    NodeId(Option<Vec<NodeId>>),
    /// Type id 18.
    ExpandedNodeId(Option<Vec<ExpandedNodeId>>),
    /// Type id 19.
    StatusCode(Option<Vec<StatusCode>>),
    /// Type id 20.
    QualifiedName(Option<Vec<QualifiedName>>),
    /// Type id 21.
    LocalizedText(Option<Vec<LocalizedText>>),
    /// Type id 22.
    ExtensionObject(Option<Vec<ExtensionObject>>),
    /// Type id 23.
    DataValue(Option<Vec<DataValue>>),
    /// Type id 24.
    Variant(Option<Vec<Variant>>),
    /// Type id 25.
    DiagnosticInfo(Option<Vec<DiagnosticInfo>>),
}

/// The functions of the two enums, one arm for each of the 25 built-in types and its type id.
macro_rules! variant_types {
    ($($name:ident($ty:ty, $element:ty) = $id:literal;)*) => {
        impl Scalar {
            /// The type id of the value.
            const fn type_id(&self) -> u8 {
                match self {
                    $(Self::$name(_) => $id,)*
                }
            }

            fn parse(
                reader: &mut BoundedReader<'_>,
                type_id: u8,
                depth: Depth,
            ) -> Result<Self, ParseError> {
                match type_id {
                    $($id => <$ty as Element>::decode(reader, depth).map(Self::$name),)*
                    other => Err(unknown_type(other)),
                }
            }

            fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
                match self {
                    $(Self::$name(value) => value.encode(writer, depth),)*
                }
            }

            fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
                match self {
                    $(Self::$name(value) => value.encoded_size(depth),)*
                }
            }
        }

        impl Array {
            /// The type id of the elements.
            const fn type_id(&self) -> u8 {
                match self {
                    $(Self::$name(_) => $id,)*
                }
            }

            /// The number of elements, 0 for a null array.
            fn len(&self) -> usize {
                match self {
                    $(Self::$name(values) => values.as_ref().map_or(0, Vec::len),)*
                }
            }

            fn parse(
                reader: &mut BoundedReader<'_>,
                type_id: u8,
                depth: Depth,
            ) -> Result<Self, ParseError> {
                match type_id {
                    $($id => read_array::<$element>(reader, depth, LENGTH_FIELD).map(Self::$name),)*
                    other => Err(unknown_type(other)),
                }
            }

            fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
                match self {
                    $(Self::$name(values) => write_array(writer, depth, values.as_deref()),)*
                }
            }

            fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
                match self {
                    $(Self::$name(values) => array_size(values.as_deref(), depth),)*
                }
            }
        }
    };
}

variant_types! {
    Boolean(bool, bool) = 1;
    SByte(i8, i8) = 2;
    Byte(u8, u8) = 3;
    Int16(i16, i16) = 4;
    UInt16(u16, u16) = 5;
    Int32(i32, i32) = 6;
    UInt32(u32, u32) = 7;
    Int64(i64, i64) = 8;
    UInt64(u64, u64) = 9;
    Float(f32, f32) = 10;
    Double(f64, f64) = 11;
    String(UaString, UaString) = 12;
    DateTime(DateTime, DateTime) = 13;
    Guid(Guid, Guid) = 14;
    ByteString(UaByteString, UaByteString) = 15;
    XmlElement(XmlElement, XmlElement) = 16;
    NodeId(NodeId, NodeId) = 17;
    ExpandedNodeId(ExpandedNodeId, ExpandedNodeId) = 18;
    StatusCode(StatusCode, StatusCode) = 19;
    QualifiedName(QualifiedName, QualifiedName) = 20;
    LocalizedText(LocalizedText, LocalizedText) = 21;
    ExtensionObject(ExtensionObject, ExtensionObject) = 22;
    DataValue(Box<DataValue>, DataValue) = 23;
    Variant(Box<Variant>, Variant) = 24;
    DiagnosticInfo(DiagnosticInfo, DiagnosticInfo) = 25;
}

/// What a Variant holds.
#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::opcua) enum VariantValue {
    /// Nothing (type id 0).
    #[default]
    Null,
    /// One value.
    Scalar(Scalar),
    /// An array of values, which may hold one element: the array flag is kept.
    Array(Array),
}

impl VariantValue {
    /// The number of values: 0 for nothing and for a null array, 1 for a scalar.
    fn count(&self) -> usize {
        match self {
            Self::Null => 0,
            Self::Scalar(_) => 1,
            Self::Array(array) => array.len(),
        }
    }
}

/// A Variant: its value and, for a multi-dimensional array, the length of each dimension.
///
/// A value read with the array-dimensions flag keeps the dimensions, and they are written back
/// whenever they are present. The product of the dimensions must equal the number of values
/// (`OPCUA-TYPE-SCALAR-ARRAY`), checked on read and on write. A Variant of type 0 holds no
/// value and ignores the array flag, as upstream's mspec does.
#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::opcua) struct Variant {
    /// The value.
    pub(in crate::opcua) value: VariantValue,
    /// The length of each dimension of an array, when the dimensions flag is set.
    pub(in crate::opcua) dimensions: Option<Vec<i32>>,
}

/// Checks the dimensions of a Variant against the number of values it holds.
fn check_dimensions(dimensions: &[i32], count: usize) -> Result<(), ParseError> {
    let mismatch = |value: usize| ParseError::InvalidDiscriminator {
        field: DIMENSIONS_FIELD,
        value: u64::try_from(value).unwrap_or(u64::MAX),
    };
    if dimensions.is_empty() {
        return Err(mismatch(0));
    }
    let mut product = 1_usize;
    for &dimension in dimensions {
        let Ok(length) = usize::try_from(dimension) else {
            return Err(ParseError::InvalidDiscriminator {
                field: DIMENSIONS_FIELD,
                value: u64::from(dimension.unsigned_abs()),
            });
        };
        product = product.checked_mul(length).ok_or(ParseError::Overflow)?;
    }
    if product == count {
        Ok(())
    } else {
        Err(mismatch(product))
    }
}

impl Variant {
    /// A Variant of one value, without dimensions.
    pub(in crate::opcua) const fn scalar(value: Scalar) -> Self {
        Self {
            value: VariantValue::Scalar(value),
            dimensions: None,
        }
    }

    /// Reads a Variant, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::Truncated`] when the bytes are not all present;
    /// [`ParseError::InvalidDiscriminator`] for a type id above 25 (field `VariantType`), a
    /// negative count other than -1, a negative dimension, or dimensions whose product is not
    /// the number of values (field `arrayDimensions`);
    /// [`ParseError::CountLimit`] for a count larger than the remaining bytes can hold;
    /// [`ParseError::Overflow`] when the product of the dimensions overflows; and the failures
    /// of the elements.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let depth = depth.enter()?;
        let mask = reader.read_u8()?;
        let type_id = mask & TYPE_MASK;
        let value = if type_id == NULL_TYPE {
            VariantValue::Null
        } else if mask & ARRAY_FLAG == 0 {
            Scalar::parse(reader, type_id, depth).map(VariantValue::Scalar)?
        } else {
            Array::parse(reader, type_id, depth).map(VariantValue::Array)?
        };
        let dimensions = if mask & DIMENSIONS_FLAG == 0 {
            None
        } else {
            let lengths = read_array::<i32>(reader, depth, DIMENSIONS_FIELD)?;
            let lengths = lengths.unwrap_or_default();
            check_dimensions(&lengths, value.count())?;
            Some(lengths)
        };
        Ok(Self { value, dimensions })
    }

    /// Writes the Variant, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::InvalidDiscriminator`] and [`ParseError::Overflow`] for dimensions that
    /// do not fit the values, before anything is written;
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let depth = depth.enter()?;
        if let Some(dimensions) = &self.dimensions {
            check_dimensions(dimensions, self.value.count())?;
        }
        let (type_id, flag) = match &self.value {
            VariantValue::Null => (NULL_TYPE, 0),
            VariantValue::Scalar(scalar) => (scalar.type_id(), 0),
            VariantValue::Array(array) => (array.type_id(), ARRAY_FLAG),
        };
        let dimensions_flag = if self.dimensions.is_some() {
            DIMENSIONS_FLAG
        } else {
            0
        };
        writer.write_u8(type_id | flag | dimensions_flag)?;
        match &self.value {
            VariantValue::Null => {}
            VariantValue::Scalar(scalar) => scalar.serialize(writer, depth)?,
            VariantValue::Array(array) => array.serialize(writer, depth)?,
        }
        if let Some(dimensions) = &self.dimensions {
            write_array(writer, depth, Some(dimensions.as_slice()))?;
        }
        Ok(())
    }

    /// The encoded size in bytes, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent; the failures of
    /// [`Variant::serialize`] for dimensions that do not fit the values;
    /// [`ParseError::OversizedLength`] for a String over the maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let depth = depth.enter()?;
        if let Some(dimensions) = &self.dimensions {
            check_dimensions(dimensions, self.value.count())?;
        }
        let value = match &self.value {
            VariantValue::Null => 0,
            VariantValue::Scalar(scalar) => scalar.length_in_bytes(depth)?,
            VariantValue::Array(array) => array.length_in_bytes(depth)?,
        };
        let dimensions = match &self.dimensions {
            Some(dimensions) => array_size(Some(dimensions.as_slice()), depth)?,
            None => 0,
        };
        value
            .checked_add(dimensions)
            .and_then(|size| size.checked_add(1))
            .ok_or(ParseError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::ExtensionBody;
    use crate::opcua::codec::support::{
        assert_cut_off_counted, assert_spends_one_entry, assert_vector, budget, decode, encode,
    };

    const GUID: Guid = Guid {
        data1: 0x7296_2B91,
        data2: 0xFA75,
        data3: 0x4AE6,
        data4: [0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D, 0xAF, 0x63],
    };
    const GUID_BYTES: [u8; 16] = [
        0x91, 0x2B, 0x96, 0x72, 0x75, 0xFA, 0xE6, 0x4A, 0x8D, 0x28, 0xB4, 0x04, 0xDC, 0x7D, 0xAF,
        0x63,
    ];

    fn scalar(value: Scalar) -> Variant {
        Variant::scalar(value)
    }

    fn array(values: Array) -> Variant {
        Variant {
            value: VariantValue::Array(values),
            dimensions: None,
        }
    }

    /// `levels` Variants that each hold one more Variant, as one mask byte each.
    fn nested_variants(levels: usize) -> Vec<u8> {
        vec![0x18; levels]
    }

    fn refusal(bytes: &[u8]) -> ParseError {
        decode::<Variant>(bytes).expect_err("the bytes are refused")
    }

    #[test]
    fn opcua_codec_variant_scalars_of_the_25_types_round_trip() {
        let mut guid = vec![0x0E];
        guid.extend_from_slice(&GUID_BYTES);
        let cases: Vec<(Scalar, Vec<u8>)> = vec![
            (Scalar::Boolean(true), vec![0x01, 0x01]),
            (Scalar::Boolean(false), vec![0x01, 0x00]),
            (Scalar::SByte(-2), vec![0x02, 0xFE]),
            (Scalar::Byte(255), vec![0x03, 0xFF]),
            (Scalar::Int16(-2), vec![0x04, 0xFE, 0xFF]),
            (Scalar::UInt16(0x1234), vec![0x05, 0x34, 0x12]),
            (Scalar::Int32(-2), vec![0x06, 0xFE, 0xFF, 0xFF, 0xFF]),
            (
                Scalar::UInt32(0x1234_5678),
                vec![0x07, 0x78, 0x56, 0x34, 0x12],
            ),
            (
                Scalar::Int64(-2),
                vec![0x08, 0xFE, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
            ),
            (
                Scalar::UInt64(0x0102_0304_0506_0708),
                vec![0x09, 0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01],
            ),
            (Scalar::Float(1.5), vec![0x0A, 0x00, 0x00, 0xC0, 0x3F]),
            (
                Scalar::Double(1.5),
                vec![0x0B, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xF8, 0x3F],
            ),
            (
                Scalar::String(UaString::new("Hi")),
                vec![0x0C, 0x02, 0x00, 0x00, 0x00, 0x48, 0x69],
            ),
            (
                Scalar::DateTime(DateTime(1)),
                vec![0x0D, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
            ),
            (Scalar::Guid(GUID), guid),
            (
                Scalar::ByteString(UaByteString::new(vec![1, 2, 3])),
                vec![0x0F, 0x03, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03],
            ),
            (
                Scalar::XmlElement(XmlElement(UaString::new("<a/>"))),
                vec![0x10, 0x04, 0x00, 0x00, 0x00, 0x3C, 0x61, 0x2F, 0x3E],
            ),
            (
                Scalar::NodeId(NodeId::numeric(0, 85)),
                vec![0x11, 0x00, 0x55],
            ),
            (
                Scalar::ExpandedNodeId(ExpandedNodeId::local(NodeId::numeric(5, 1025))),
                vec![0x12, 0x01, 0x05, 0x01, 0x04],
            ),
            (
                Scalar::StatusCode(StatusCode(0x8001_0000)),
                vec![0x13, 0x00, 0x00, 0x01, 0x80],
            ),
            (
                Scalar::QualifiedName(QualifiedName {
                    namespace_index: 1,
                    name: UaString::new("Hi"),
                }),
                vec![0x14, 0x01, 0x00, 0x02, 0x00, 0x00, 0x00, 0x48, 0x69],
            ),
            (
                Scalar::LocalizedText(LocalizedText {
                    locale: None,
                    text: Some(UaString::new("Hi")),
                }),
                vec![0x15, 0x02, 0x02, 0x00, 0x00, 0x00, 0x48, 0x69],
            ),
            (
                Scalar::ExtensionObject(ExtensionObject {
                    type_id: ExpandedNodeId::local(NodeId::numeric(0, 0)),
                    body: ExtensionBody::Null,
                }),
                vec![0x16, 0x00, 0x00, 0x00],
            ),
            (
                Scalar::DataValue(Box::new(DataValue {
                    value: Some(scalar(Scalar::Int32(7))),
                    status: Some(StatusCode(0)),
                    ..DataValue::default()
                })),
                vec![
                    0x17, 0x03, 0x06, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                ],
            ),
            (
                Scalar::Variant(Box::new(scalar(Scalar::Int32(7)))),
                vec![0x18, 0x06, 0x07, 0x00, 0x00, 0x00],
            ),
            (
                Scalar::DiagnosticInfo(DiagnosticInfo {
                    symbolic_id: Some(5),
                    ..DiagnosticInfo::default()
                }),
                vec![0x19, 0x01, 0x05, 0x00, 0x00, 0x00],
            ),
        ];
        assert_eq!(cases.len(), 25 + 1, "the 25 types, Boolean twice");
        let mut ids: Vec<u8> = cases.iter().map(|(value, _)| value.type_id()).collect();
        ids.dedup();
        assert_eq!(ids, (1..=25).collect::<Vec<u8>>(), "ids 1 to 25 in order");
        for (value, bytes) in cases {
            let variant = scalar(value);
            assert_vector(&variant, &bytes);
            assert_cut_off_counted(&variant);
        }
    }

    #[test]
    fn opcua_codec_variant_boolean_reads_any_non_zero_byte_as_true() {
        for byte in [0x02, 0x80, 0xFF] {
            assert_eq!(
                decode::<Variant>(&[0x01, byte]),
                Ok(scalar(Scalar::Boolean(true)))
            );
        }
        assert_eq!(encode(&scalar(Scalar::Boolean(true))), [0x01, 0x01]);
    }

    #[test]
    fn opcua_codec_variant_null_holds_nothing() {
        assert_vector(&Variant::default(), &[0x00]);
        // The array flag of a Variant of type 0 is ignored, nothing follows it.
        assert_eq!(decode::<Variant>(&[0x80]), Ok(Variant::default()));
        assert_eq!(
            refusal(&[]),
            ParseError::Truncated {
                needed: 1,
                available: 0
            }
        );
    }

    #[test]
    fn opcua_codec_variant_type_ids_above_25_are_invalid() {
        for type_id in [26_u8, 27, 40, 63] {
            let invalid = ParseError::InvalidDiscriminator {
                field: "VariantType",
                value: u64::from(type_id),
            };
            assert_eq!(refusal(&[type_id]), invalid, "scalar {type_id}");
            assert_eq!(
                refusal(&[type_id | 0x80, 0, 0, 0, 0]),
                invalid,
                "array {type_id}"
            );
        }
    }

    #[test]
    fn opcua_codec_variant_int32_array_round_trips() {
        // The array of VariantArrayCountLimitTest: header 0x86, two elements.
        let variant = array(Array::Int32(Some(vec![7, 9])));
        assert_vector(
            &variant,
            &[
                0x86, 0x02, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x09, 0x00, 0x00, 0x00,
            ],
        );
        assert_cut_off_counted(&variant);
    }

    #[test]
    fn opcua_codec_variant_null_array_is_not_an_empty_one() {
        let null = array(Array::Int32(None));
        let empty = array(Array::Int32(Some(Vec::new())));
        assert_vector(&null, &[0x86, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_vector(&empty, &[0x86, 0x00, 0x00, 0x00, 0x00]);
        assert_ne!(null, empty, "null and empty stay apart");
    }

    #[test]
    fn opcua_codec_variant_one_element_array_keeps_the_array_flag() {
        let one = array(Array::Byte(Some(vec![9])));
        assert_vector(&one, &[0x83, 0x01, 0x00, 0x00, 0x00, 0x09]);
        assert_ne!(one, scalar(Scalar::Byte(9)), "an array is not a scalar");
    }

    #[test]
    fn opcua_codec_variant_arrays_of_the_25_types_round_trip() {
        let node = NodeId::numeric(0, 85);
        let text = LocalizedText {
            locale: Some(UaString::new("en")),
            text: Some(UaString::null()),
        };
        let object = ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(1, 2)),
            body: ExtensionBody::Raw(UaByteString::new(vec![1, 2])),
        };
        let data = DataValue {
            value: Some(scalar(Scalar::Boolean(true))),
            ..DataValue::default()
        };
        let info = DiagnosticInfo {
            additional_info: Some(UaString::new("x")),
            ..DiagnosticInfo::default()
        };
        let arrays = vec![
            Array::Boolean(Some(vec![true, false])),
            Array::SByte(Some(vec![-1, 1])),
            Array::Byte(Some(vec![1, 2])),
            Array::Int16(Some(vec![-1, 1])),
            Array::UInt16(Some(vec![1, 2])),
            Array::Int32(Some(vec![-1, 1])),
            Array::UInt32(Some(vec![1, 2])),
            Array::Int64(Some(vec![-1, 1])),
            Array::UInt64(Some(vec![1, 2])),
            Array::Float(Some(vec![-1.5, 2.5])),
            Array::Double(Some(vec![-1.5, 2.5])),
            Array::String(Some(vec![UaString::new("a"), UaString::null()])),
            Array::DateTime(Some(vec![DateTime(1), DateTime(-1)])),
            Array::Guid(Some(vec![GUID, Guid::default()])),
            Array::ByteString(Some(vec![UaByteString::null(), UaByteString::new(vec![3])])),
            Array::XmlElement(Some(vec![
                XmlElement(UaString::new("<a/>")),
                XmlElement::default(),
            ])),
            Array::NodeId(Some(vec![node.clone(), NodeId::numeric(300, 70_000)])),
            Array::ExpandedNodeId(Some(vec![
                ExpandedNodeId::local(node),
                ExpandedNodeId {
                    node_id: NodeId::numeric(1, 1),
                    namespace_uri: Some(UaString::new("urn:x")),
                    server_index: Some(2),
                },
            ])),
            Array::StatusCode(Some(vec![StatusCode(0), StatusCode(0x8000_0000)])),
            Array::QualifiedName(Some(vec![
                QualifiedName::default(),
                QualifiedName::default(),
            ])),
            Array::LocalizedText(Some(vec![text, LocalizedText::default()])),
            Array::ExtensionObject(Some(vec![object.clone(), object])),
            Array::DataValue(Some(vec![data, DataValue::default()])),
            Array::Variant(Some(vec![scalar(Scalar::Byte(1)), Variant::default()])),
            Array::DiagnosticInfo(Some(vec![info, DiagnosticInfo::default()])),
        ];
        assert_eq!(arrays.len(), 25);
        for (index, values) in arrays.into_iter().enumerate() {
            assert_eq!(usize::from(values.type_id()), index + 1, "{values:?}");
            assert_eq!(values.len(), 2, "{values:?}");
            let variant = array(values);
            assert_eq!(variant.clone(), variant, "a clone is equal");
            let bytes = encode(&variant);
            assert_eq!(decode::<Variant>(&bytes).as_ref(), Ok(&variant));
            assert_eq!(variant.length_in_bytes(Depth::new()), Ok(bytes.len()));
            assert_cut_off_counted(&variant);
        }
    }

    #[test]
    fn opcua_codec_variant_string_array_has_the_wire_layout() {
        let variant = array(Array::String(Some(vec![
            UaString::new("a"),
            UaString::null(),
        ])));
        assert_vector(
            &variant,
            &[
                0x8C, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x61, 0xFF, 0xFF, 0xFF, 0xFF,
            ],
        );
    }

    #[test]
    fn opcua_codec_variant_array_count_larger_than_the_body_fails_before_allocation() {
        // 2^31 - 1 elements and no payload (VariantArrayCountLimitTest, 0x86 ff ff ff 7f).
        assert_eq!(
            refusal(&[0x86, 0xFF, 0xFF, 0xFF, 0x7F]),
            ParseError::CountLimit {
                field: "arrayLength",
                count: 2_147_483_647,
                max: 0,
            }
        );
        // The count is refused for every element type, fixed or variable, before it is read.
        for type_id in 1..=25_u8 {
            assert_eq!(
                refusal(&[type_id | 0x80, 0xFF, 0xFF, 0xFF, 0x7F]),
                ParseError::CountLimit {
                    field: "arrayLength",
                    count: 2_147_483_647,
                    max: 0
                },
                "type {type_id}"
            );
        }
    }

    #[test]
    fn opcua_codec_variant_array_count_is_checked_against_the_element_width() {
        // Int32 elements are 4 bytes: 2 bytes hold none, 7 hold one, 8 hold two.
        let header = |count: u8| [0x86, count, 0x00, 0x00, 0x00];
        for (extra, count, max) in [(2_usize, 1_u8, 0_usize), (7, 2, 1), (8, 3, 2)] {
            let mut bytes = header(count).to_vec();
            bytes.extend(std::iter::repeat_n(0, extra));
            assert_eq!(
                refusal(&bytes),
                ParseError::CountLimit {
                    field: "arrayLength",
                    count: usize::from(count),
                    max,
                },
                "{extra} bytes"
            );
        }
        let mut bytes = header(2).to_vec();
        bytes.extend(std::iter::repeat_n(0, 8));
        assert!(decode::<Variant>(&bytes).is_ok(), "exactly as many as fit");
        // A String takes at least 4 bytes but the check allows one per remaining byte; the
        // elements then run out.
        let mut bytes = vec![0x8C, 0x03, 0x00, 0x00, 0x00];
        bytes.extend([0xFF; 3]);
        assert_eq!(
            refusal(&bytes),
            ParseError::Truncated {
                needed: 4,
                available: 3
            }
        );
        let mut bytes = vec![0x8C, 0x04, 0x00, 0x00, 0x00];
        bytes.extend([0xFF; 3]);
        assert_eq!(
            refusal(&bytes),
            ParseError::CountLimit {
                field: "arrayLength",
                count: 4,
                max: 3
            }
        );
    }

    #[test]
    fn opcua_codec_variant_negative_array_count_other_than_minus_one_is_invalid() {
        assert_eq!(
            refusal(&[0x86, 0xFE, 0xFF, 0xFF, 0xFF]),
            ParseError::InvalidDiscriminator {
                field: "arrayLength",
                value: 2
            }
        );
        assert_eq!(
            refusal(&[0x86, 0x00, 0x00, 0x00, 0x80]),
            ParseError::InvalidDiscriminator {
                field: "arrayLength",
                value: 2_147_483_648
            }
        );
    }

    #[test]
    fn opcua_codec_variant_nesting_beyond_the_limit_is_recursion_limit() {
        // VariantNestingTest.java:59,68: 2000, 3000, 4000 and 16000 levels are refused.
        let limit = ParseError::RecursionLimit { limit: 64 };
        for levels in [2000, 3000, 4000, 16_000] {
            assert_eq!(refusal(&nested_variants(levels)), limit, "{levels} levels");
        }
        // VariantNestingTest.java:79: 32 levels are read as far as the data goes; they run out
        // of data, not out of budget.
        assert_eq!(
            refusal(&nested_variants(32)),
            ParseError::Truncated {
                needed: 1,
                available: 0
            }
        );
    }

    #[test]
    fn opcua_codec_struct_field_array_count_larger_than_the_body_is_refused() {
        // StructFieldArrayCountTest.java: an Int32 count and Byte elements with no Variant
        // header, which is what `read_array` reads.
        let read = |bytes: &[u8]| {
            let mut reader = BoundedReader::new(bytes);
            read_array::<u8>(&mut reader, Depth::new(), "values")
        };
        // Two billion elements followed by two bytes, and Int32::MAX elements likewise.
        assert_eq!(
            read(&[0x00, 0x00, 0x00, 0x70, 0x01, 0x02]),
            Err(ParseError::CountLimit {
                field: "values",
                count: 1_879_048_192,
                max: 2
            })
        );
        assert_eq!(
            read(&[0xFF, 0xFF, 0xFF, 0x7F, 0x01, 0x02]),
            Err(ParseError::CountLimit {
                field: "values",
                count: 2_147_483_647,
                max: 2
            })
        );
        // A count that matches the body is read.
        assert_eq!(
            read(&[0x03, 0x00, 0x00, 0x00, 0x07, 0x08, 0x09]),
            Ok(Some(vec![7, 8, 9]))
        );
        // Upstream turns -1 into an empty list; here it is a null array, kept apart from an
        // empty one, and the value mapping of S119 decides what a null field means.
        assert_eq!(read(&[0xFF, 0xFF, 0xFF, 0xFF]), Ok(None));
        assert_eq!(read(&[0x00, 0x00, 0x00, 0x00]), Ok(Some(vec![])));
    }

    #[test]
    fn opcua_codec_variant_nesting_at_the_limit_is_read() {
        // VariantNestingTest.java:79: 32 levels run out of data, not out of budget.
        assert_eq!(
            refusal(&nested_variants(32)),
            ParseError::Truncated {
                needed: 1,
                available: 0
            }
        );
        // 63 holders and a Variant of nothing are 64 entries: the whole budget.
        let mut bytes = nested_variants(63);
        bytes.push(0x00);
        assert!(decode::<Variant>(&bytes).is_ok(), "64 entries");
        // One more is 65.
        let mut bytes = nested_variants(64);
        bytes.push(0x00);
        assert_eq!(refusal(&bytes), ParseError::RecursionLimit { limit: 64 });
    }

    #[test]
    fn opcua_codec_variant_nesting_through_arrays_spends_the_budget() {
        // An array of one Variant, which is an array of one Variant, and so on.
        let level = [0x98, 0x01, 0x00, 0x00, 0x00];
        let mut bytes: Vec<u8> = level.repeat(100);
        bytes.push(0x00);
        assert_eq!(refusal(&bytes), ParseError::RecursionLimit { limit: 64 });
        let mut bytes: Vec<u8> = level.repeat(63);
        bytes.push(0x00);
        assert!(decode::<Variant>(&bytes).is_ok());
    }

    #[test]
    fn opcua_codec_variant_nesting_of_mixed_types_fits_a_small_stack() {
        // Variant, DataValue, Variant, DataValue, ... : two entries per pair; and an array of
        // one Variant inside an array of one Variant, the deepest frames. Each to the full
        // budget and one entry past it, on the 2 MiB stack of a test thread, in the dev profile.
        let pairs = |count: usize| {
            let mut bytes = [0x17, 0x01].repeat(count);
            bytes.extend([0x17, 0x00]);
            bytes
        };
        let arrays = |count: usize| {
            let mut bytes: Vec<u8> = [0x98, 0x01, 0x00, 0x00, 0x00].repeat(count);
            bytes.push(0x00);
            bytes
        };
        let outcome = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                [pairs(31), pairs(32), arrays(63), arrays(64)]
                    .map(|bytes| decode::<Variant>(&bytes))
            })
            .expect("a thread")
            .join()
            .expect("no stack overflow");
        let [pairs_ok, pairs_deep, arrays_ok, arrays_deep] = outcome;
        assert!(pairs_ok.is_ok(), "64 entries are read");
        assert!(arrays_ok.is_ok(), "64 entries of arrays are read");
        let limit = Err(ParseError::RecursionLimit { limit: 64 });
        assert_eq!(pairs_deep, limit, "65 entries are refused");
        assert_eq!(arrays_deep, limit, "65 entries of arrays are refused");
    }

    #[test]
    fn opcua_codec_variant_a_value_nested_past_the_budget_is_not_written_or_measured() {
        let mut variant = scalar(Scalar::Byte(1));
        for _ in 0..63 {
            variant = scalar(Scalar::Variant(Box::new(variant)));
        }
        assert!(decode::<Variant>(&encode(&variant)).is_ok(), "64 entries");
        let too_deep = scalar(Scalar::Variant(Box::new(variant)));
        let limit = ParseError::RecursionLimit { limit: 64 };
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(
            too_deep.serialize(&mut writer, Depth::new()),
            Err(limit.clone())
        );
        assert_eq!(too_deep.length_in_bytes(Depth::new()), Err(limit));
    }

    #[test]
    fn opcua_codec_variant_spends_one_entry() {
        assert_spends_one_entry(&scalar(Scalar::Int32(7)));
        assert_spends_one_entry(&array(Array::Int32(Some(vec![7, 9]))));
        // An element that is itself a Variant needs a second entry.
        let inner = scalar(Scalar::Variant(Box::default()));
        let bytes = encode(&inner);
        let mut reader = BoundedReader::new(&bytes);
        assert_eq!(
            Variant::parse(&mut reader, budget(1)),
            Err(ParseError::RecursionLimit { limit: 64 })
        );
        let mut reader = BoundedReader::new(&bytes);
        assert_eq!(Variant::parse(&mut reader, budget(2)), Ok(inner));
    }

    #[test]
    fn opcua_codec_variant_array_dimensions_partition_the_values() {
        // A 3 by 2 array of Int32: mask 0xC6, six values, then the dimensions 3 and 2.
        let mut bytes = vec![0xC6, 0x06, 0x00, 0x00, 0x00];
        for value in 1..=6_u8 {
            bytes.extend([value, 0, 0, 0]);
        }
        bytes.extend([
            0x02, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00,
        ]);
        let variant = Variant {
            value: VariantValue::Array(Array::Int32(Some(vec![1, 2, 3, 4, 5, 6]))),
            dimensions: Some(vec![3, 2]),
        };
        assert_vector(&variant, &bytes);
        assert_cut_off_counted(&variant);
    }

    #[test]
    fn opcua_codec_variant_dimensions_inconsistent_with_the_count_are_refused() {
        let dimensions = |lengths: &[i32]| -> Vec<u8> {
            let mut bytes = vec![0xC3, 0x02, 0x00, 0x00, 0x00, 0x01, 0x02];
            bytes.extend(i32::try_from(lengths.len()).expect("small").to_le_bytes());
            for length in lengths {
                bytes.extend(length.to_le_bytes());
            }
            bytes
        };
        let mismatch = |value: u64| ParseError::InvalidDiscriminator {
            field: "arrayDimensions",
            value,
        };
        assert!(decode::<Variant>(&dimensions(&[2])).is_ok());
        assert!(decode::<Variant>(&dimensions(&[1, 2, 1])).is_ok());
        assert_eq!(refusal(&dimensions(&[3])), mismatch(3));
        assert_eq!(refusal(&dimensions(&[1])), mismatch(1));
        assert_eq!(refusal(&dimensions(&[2, 2])), mismatch(4));
        assert_eq!(refusal(&dimensions(&[0, 5])), mismatch(0));
        assert_eq!(refusal(&dimensions(&[-2, -1])), mismatch(2));
        assert_eq!(refusal(&dimensions(&[])), mismatch(0));
        assert_eq!(
            refusal(&dimensions(&[i32::MAX, i32::MAX, i32::MAX])),
            ParseError::Overflow
        );
        // A dimension count that the remaining bytes cannot hold is refused before it is read.
        let mut bytes = vec![0xC3, 0x02, 0x00, 0x00, 0x00, 0x01, 0x02];
        bytes.extend([0xFF, 0xFF, 0xFF, 0x7F]);
        assert_eq!(
            refusal(&bytes),
            ParseError::CountLimit {
                field: "arrayDimensions",
                count: 2_147_483_647,
                max: 0
            }
        );
        // A null dimension list is empty.
        let mut bytes = vec![0xC3, 0x02, 0x00, 0x00, 0x00, 0x01, 0x02];
        bytes.extend([0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(refusal(&bytes), mismatch(0));
    }

    #[test]
    fn opcua_codec_variant_dimensions_follow_a_scalar_and_nothing() {
        // The product must be 1 for one value and 0 for none.
        assert_vector(
            &Variant {
                dimensions: Some(vec![1]),
                ..scalar(Scalar::Byte(9))
            },
            &[0x43, 0x09, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
        );
        assert_vector(
            &Variant {
                dimensions: Some(vec![0, 4]),
                ..Variant::default()
            },
            &[
                0x40, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00,
            ],
        );
        assert_eq!(
            refusal(&[0x43, 0x09, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0x00, 0x00]),
            ParseError::InvalidDiscriminator {
                field: "arrayDimensions",
                value: 2
            }
        );
    }

    #[test]
    fn opcua_codec_variant_inconsistent_dimensions_are_not_written_or_measured() {
        let variant = Variant {
            value: VariantValue::Array(Array::Byte(Some(vec![1, 2]))),
            dimensions: Some(vec![3]),
        };
        let invalid = ParseError::InvalidDiscriminator {
            field: "arrayDimensions",
            value: 3,
        };
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(
            variant.serialize(&mut writer, Depth::new()),
            Err(invalid.clone())
        );
        assert!(writer.as_slice().is_empty(), "nothing is written");
        assert_eq!(variant.length_in_bytes(Depth::new()), Err(invalid));
    }

    #[test]
    fn opcua_codec_variant_element_failures_surface() {
        // A String element with a length of -2, and a NodeId element with encoding 6.
        assert_eq!(
            refusal(&[0x0C, 0xFE, 0xFF, 0xFF, 0xFF]),
            ParseError::InvalidDiscriminator {
                field: "String length",
                value: 2
            }
        );
        assert_eq!(
            refusal(&[0x11, 0x06, 0x00]),
            ParseError::InvalidDiscriminator {
                field: "NodeId encoding",
                value: 6
            }
        );
    }

    #[test]
    fn opcua_codec_variant_length_counts_the_mask_the_count_and_the_dimensions() {
        let variant = Variant {
            value: VariantValue::Array(Array::Int16(Some(vec![1, 2, 3, 4]))),
            dimensions: Some(vec![2, 2]),
        };
        // 1 mask, 4 count, 8 values, 4 count, 8 dimensions.
        assert_eq!(variant.length_in_bytes(Depth::new()), Ok(25));
        assert_eq!(Variant::default().length_in_bytes(Depth::new()), Ok(1));
    }

    #[test]
    fn opcua_codec_variant_serialize_is_refused_when_the_writer_is_full() {
        let variant = array(Array::Int32(Some(vec![7, 9])));
        let mut writer = BoundedWriter::new(12);
        assert_eq!(
            variant.serialize(&mut writer, Depth::new()),
            Err(ParseError::OversizedLength {
                length: 13,
                max: 12
            })
        );
        let long = array(Array::String(Some(vec![UaString::from_bytes(vec![0; 3])])));
        let mut writer = BoundedWriter::new(8);
        assert!(long.serialize(&mut writer, Depth::new()).is_err());
    }
}
