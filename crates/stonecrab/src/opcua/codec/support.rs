//! Helpers shared by the tests of the built-ins: one trait over the external-type signature
//! and the checks every type must pass.

use std::fmt::Debug;
use std::mem::discriminant;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{
    DataValue, DateTime, Depth, DiagnosticInfo, ExpandedNodeId, ExtensionObject, Guid,
    LocalizedText, NodeId, QualifiedName, StatusCode, UaByteString, UaString, Variant,
};

/// The three functions of the external-type signature.
pub(super) trait Builtin: Sized + Debug + Clone + PartialEq {
    fn parse_with(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError>;
    fn serialize_with(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError>;
    fn length_with(&self, depth: Depth) -> Result<usize, ParseError>;
}

macro_rules! builtin {
    ($($ty:ty),*) => {$(
        impl Builtin for $ty {
            fn parse_with(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
                Self::parse(reader, depth)
            }
            fn serialize_with(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
                self.serialize(writer, depth)
            }
            fn length_with(&self, depth: Depth) -> Result<usize, ParseError> {
                self.length_in_bytes(depth)
            }
        }
    )*};
}

builtin!(
    UaString,
    UaByteString,
    Guid,
    DateTime,
    StatusCode,
    NodeId,
    ExpandedNodeId,
    QualifiedName,
    LocalizedText,
    Variant,
    DataValue,
    DiagnosticInfo,
    ExtensionObject
);

/// A budget with `left` entries still allowed.
pub(super) fn budget(left: u8) -> Depth {
    let mut depth = Depth::new();
    for _ in left..Depth::LIMIT {
        depth = depth.enter().expect("the budget is not yet spent");
    }
    depth
}

/// The bytes of `value`, written with room to spare.
pub(super) fn encode<T: Builtin>(value: &T) -> Vec<u8> {
    let mut writer = BoundedWriter::new(usize::MAX);
    value
        .serialize_with(&mut writer, Depth::new())
        .expect("serializes");
    writer.into_bytes()
}

/// The value `bytes` hold when they are all consumed, or the failure.
pub(super) fn decode<T: Builtin>(bytes: &[u8]) -> Result<T, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    let value = T::parse_with(&mut reader, Depth::new())?;
    reader.expect_eof()?;
    Ok(value)
}

/// `value` encodes to exactly `bytes`, decodes back from them, and measures as long as they
/// are; the two directions are asserted apart.
pub(super) fn assert_vector<T: Builtin>(value: &T, bytes: &[u8]) {
    assert_eq!(&value.clone(), value, "{value:?}: a clone is equal");
    assert_eq!(encode(value), bytes, "{value:?}: encoded bytes");
    assert_eq!(
        decode::<T>(bytes).as_ref(),
        Ok(value),
        "{value:?}: decoded value"
    );
    assert_eq!(
        value.length_with(Depth::new()),
        Ok(bytes.len()),
        "{value:?}: length_in_bytes"
    );
}

/// Every prefix of the encoding of `value` is refused as truncated, and a writer one byte
/// short of it is refused as oversized and left holding nothing it was not given.
pub(super) fn assert_cut_off<T: Builtin>(value: &T) {
    let bytes = encode(value);
    for cut in 0..bytes.len() {
        let context = format!("{value:?}: {cut} of {} bytes", bytes.len());
        let prefix = bytes.get(..cut).expect("a prefix");
        let decoded = decode::<T>(prefix);
        assert!(
            matches!(decoded, Err(ParseError::Truncated { .. })),
            "{context}"
        );
        let mut writer = BoundedWriter::new(cut);
        let written = value.serialize_with(&mut writer, Depth::new());
        assert!(
            matches!(written, Err(ParseError::OversizedLength { .. })),
            "{context}"
        );
    }
}

/// Whether the error is how a cut-off value is refused: as truncated or, inside an array, as a
/// count the remaining bytes cannot hold.
fn is_cut_off(error: &ParseError) -> bool {
    let truncated = ParseError::Truncated {
        needed: 0,
        available: 0,
    };
    let counted = ParseError::CountLimit {
        field: "",
        count: 0,
        max: 0,
    };
    let kind = discriminant(error);
    kind == discriminant(&truncated) || kind == discriminant(&counted)
}

/// Whether the error is how a full writer refuses a write.
fn is_oversized(error: &ParseError) -> bool {
    let oversized = ParseError::OversizedLength { length: 0, max: 0 };
    discriminant(error) == discriminant(&oversized)
}

/// Every prefix of the encoding of `value` is refused, as truncated or, inside an array, as a
/// count the remaining bytes cannot hold, and a writer one byte short of it is refused as
/// oversized.
pub(super) fn assert_cut_off_counted<T: Builtin>(value: &T) {
    let bytes = encode(value);
    for cut in 0..bytes.len() {
        let context = format!("{value:?}: {cut} of {} bytes", bytes.len());
        let prefix = bytes.get(..cut).expect("a prefix");
        let decoded = decode::<T>(prefix);
        assert!(
            decoded.as_ref().err().is_some_and(is_cut_off),
            "{context}: {decoded:?}"
        );
        let mut writer = BoundedWriter::new(cut);
        let written = value.serialize_with(&mut writer, Depth::new());
        assert!(
            written.as_ref().err().is_some_and(is_oversized),
            "{context}: {written:?}"
        );
    }
}

/// With no entry left, all three functions of a recursive type fail with `RecursionLimit` and
/// touch nothing; with one left, the value is read, written and measured.
pub(super) fn assert_spends_one_entry<T: Builtin>(value: &T) {
    let bytes = encode(value);
    let limit = ParseError::RecursionLimit {
        limit: u32::from(Depth::LIMIT),
    };
    let mut reader = BoundedReader::new(&bytes);
    assert_eq!(
        T::parse_with(&mut reader, budget(0)).as_ref().err(),
        Some(&limit),
        "{value:?}: parse"
    );
    assert_eq!(reader.remaining(), bytes.len(), "{value:?}: nothing read");
    let mut writer = BoundedWriter::new(usize::MAX);
    assert_eq!(
        value.serialize_with(&mut writer, budget(0)),
        Err(limit.clone()),
        "{value:?}: serialize"
    );
    assert!(writer.as_slice().is_empty(), "{value:?}: nothing written");
    assert_eq!(
        value.length_with(budget(0)),
        Err(limit),
        "{value:?}: length_in_bytes"
    );
    let mut reader = BoundedReader::new(&bytes);
    assert_eq!(
        T::parse_with(&mut reader, budget(1)).as_ref(),
        Ok(value),
        "{value:?}: parse with one entry left"
    );
    let mut writer = BoundedWriter::new(usize::MAX);
    assert_eq!(value.serialize_with(&mut writer, budget(1)), Ok(()));
    assert_eq!(writer.into_bytes(), bytes, "{value:?}: serialize");
    assert_eq!(value.length_with(budget(1)), Ok(bytes.len()));
}
