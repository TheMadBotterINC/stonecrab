//! Helpers shared by the tests of the built-ins: one trait over the external-type signature
//! and the checks every type must pass.

use std::fmt::Debug;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{
    DateTime, Depth, ExpandedNodeId, Guid, LocalizedText, NodeId, QualifiedName, StatusCode,
    UaByteString, UaString,
};

/// The three functions of the external-type signature.
pub(super) trait Builtin: Sized + Debug + PartialEq {
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
    LocalizedText
);

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
