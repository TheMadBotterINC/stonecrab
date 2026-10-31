//! The `ExtensionObject` envelope (OPC UA Part 6, 5.2.2.15): a type id, an encoding byte and,
//! when the encoding says so, a length-prefixed body.
//!
//! This is the envelope only. The body is kept as the bytes the server sent: a binary body as
//! a `ByteString` ([`ExtensionBody::Raw`]), an XML body as a String ([`ExtensionBody::Xml`]),
//! and an object with no body as [`ExtensionBody::Null`]. The type id is read as an
//! `ExpandedNodeId`, as upstream's mspec does: a conforming server sends the `NodeId` subset,
//! which is the same bytes. The decoding of a known body is wired in with the generated
//! service layer (slice S88). The type spends one unit of the [`Depth`] budget on entry, which
//! a decoded body will inherit, so that an object that holds a Variant that holds an object is
//! counted at every step.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, ExpandedNodeId, UaByteString, UaString};

/// The encoding byte of an object with no body.
const NO_BODY: u8 = 0;
/// The encoding byte of an object with a binary body.
const BINARY_BODY: u8 = 1;
/// The encoding byte of an object with an XML body.
const XML_BODY: u8 = 2;

/// The body of an `ExtensionObject`, as received.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) enum ExtensionBody {
    /// No body (encoding byte 0).
    #[default]
    Null,
    /// A binary body (encoding byte 1): the bytes of the `ByteString`, null or empty as read.
    Raw(UaByteString),
    /// An XML body (encoding byte 2): the bytes of the String, which are not checked.
    Xml(UaString),
}

/// A structure of a type the field that holds it does not name: the id of its encoding and its
/// body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::opcua) struct ExtensionObject {
    /// The node id of the data type encoding of the body.
    pub(in crate::opcua) type_id: ExpandedNodeId,
    /// The body.
    pub(in crate::opcua) body: ExtensionBody,
}

impl ExtensionObject {
    /// Reads an `ExtensionObject`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::Truncated`] when the bytes are not all present;
    /// [`ParseError::InvalidDiscriminator`] for an encoding byte other than 0, 1 and 2 (field
    /// `ExtensionObject encoding`); and the failures of the type id and the body.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let depth = depth.enter()?;
        let type_id = ExpandedNodeId::parse(reader, depth)?;
        let body = match reader.read_u8()? {
            NO_BODY => ExtensionBody::Null,
            BINARY_BODY => ExtensionBody::Raw(UaByteString::parse(reader, depth)?),
            XML_BODY => ExtensionBody::Xml(UaString::parse(reader, depth)?),
            other => {
                return Err(ParseError::InvalidDiscriminator {
                    field: "ExtensionObject encoding",
                    value: u64::from(other),
                });
            }
        };
        Ok(Self { type_id, body })
    }

    /// Writes the `ExtensionObject`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let depth = depth.enter()?;
        self.type_id.serialize(writer, depth)?;
        match &self.body {
            ExtensionBody::Null => writer.write_u8(NO_BODY),
            ExtensionBody::Raw(bytes) => {
                writer.write_u8(BINARY_BODY)?;
                bytes.serialize(writer, depth)
            }
            ExtensionBody::Xml(text) => {
                writer.write_u8(XML_BODY)?;
                text.serialize(writer, depth)
            }
        }
    }

    /// The encoded size in bytes, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::OversizedLength`] for a body over the String maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let depth = depth.enter()?;
        let body = match &self.body {
            ExtensionBody::Null => 0,
            ExtensionBody::Raw(bytes) => bytes.length_in_bytes(depth)?,
            ExtensionBody::Xml(text) => text.length_in_bytes(depth)?,
        };
        self.type_id
            .length_in_bytes(depth)?
            .checked_add(body)
            .and_then(|size| size.checked_add(1))
            .ok_or(ParseError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::NodeId;
    use crate::opcua::codec::support::{
        assert_cut_off, assert_spends_one_entry, assert_vector, decode, encode,
    };

    fn object(type_id: u32, body: ExtensionBody) -> ExtensionObject {
        ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, type_id)),
            body,
        }
    }

    #[test]
    fn opcua_codec_extension_object_without_a_body_is_the_type_id_and_a_zero_byte() {
        let null = object(0, ExtensionBody::Null);
        assert_vector(&null, &[0x00, 0x00, 0x00]);
        assert_cut_off(&null);
    }

    #[test]
    fn opcua_codec_extension_object_binary_body_is_kept_raw() {
        // i=298 (four-byte id), encoding 1, a ByteString of three bytes.
        let raw = ExtensionObject {
            type_id: ExpandedNodeId::local(NodeId::numeric(0, 298)),
            body: ExtensionBody::Raw(UaByteString::new(vec![0xAA, 0xBB, 0xCC])),
        };
        assert_vector(
            &raw,
            &[
                0x01, 0x00, 0x2A, 0x01, 0x01, 0x03, 0x00, 0x00, 0x00, 0xAA, 0xBB, 0xCC,
            ],
        );
        assert_cut_off(&raw);
    }

    #[test]
    fn opcua_codec_unknown_extension_object_body_is_kept_raw() {
        // A custom type: namespace 2, identifier 5001, bytes the codec has no definition for.
        let bytes = [
            0x01, 0x02, 0x89, 0x13, 0x01, 0x04, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
        ];
        let parsed = decode::<ExtensionObject>(&bytes).expect("a raw body");
        assert_eq!(parsed.type_id.node_id, NodeId::numeric(2, 5001));
        assert_eq!(
            parsed.body,
            ExtensionBody::Raw(UaByteString::new(vec![1, 2, 3, 4]))
        );
        assert_eq!(encode(&parsed), bytes, "the bytes come back as they were");
    }

    #[test]
    fn opcua_codec_extension_object_null_and_empty_binary_bodies_stay_apart() {
        let null = object(7, ExtensionBody::Raw(UaByteString::null()));
        let empty = object(7, ExtensionBody::Raw(UaByteString::new(Vec::new())));
        assert_vector(&null, &[0x00, 0x07, 0x01, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_vector(&empty, &[0x00, 0x07, 0x01, 0x00, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn opcua_codec_extension_object_xml_body_is_kept_as_bytes() {
        let xml = object(7, ExtensionBody::Xml(UaString::new("<a/>")));
        assert_vector(
            &xml,
            &[
                0x00, 0x07, 0x02, 0x04, 0x00, 0x00, 0x00, 0x3C, 0x61, 0x2F, 0x3E,
            ],
        );
        assert_cut_off(&xml);
        let not_text = object(
            7,
            ExtensionBody::Xml(UaString::from_bytes(vec![0xFF, 0xFE])),
        );
        assert_eq!(decode::<ExtensionObject>(&encode(&not_text)), Ok(not_text));
    }

    #[test]
    fn opcua_codec_extension_object_type_id_may_name_a_namespace_and_a_server() {
        let expanded = ExtensionObject {
            type_id: ExpandedNodeId {
                node_id: NodeId::numeric(1, 2),
                namespace_uri: Some(UaString::new("urn:x")),
                server_index: Some(3),
            },
            body: ExtensionBody::Null,
        };
        assert_cut_off(&expanded);
        assert_eq!(decode::<ExtensionObject>(&encode(&expanded)), Ok(expanded));
    }

    #[test]
    fn opcua_codec_extension_object_other_encoding_bytes_are_invalid() {
        for encoding in [3_u8, 4, 0x80, 0xFF] {
            assert_eq!(
                decode::<ExtensionObject>(&[0x00, 0x07, encoding]),
                Err(ParseError::InvalidDiscriminator {
                    field: "ExtensionObject encoding",
                    value: u64::from(encoding)
                }),
                "encoding {encoding}"
            );
        }
    }

    #[test]
    fn opcua_codec_extension_object_body_length_larger_than_the_input_fails_before_a_copy() {
        assert_eq!(
            decode::<ExtensionObject>(&[0x00, 0x07, 0x01, 0xFF, 0xFF, 0xFF, 0x7F]),
            Err(ParseError::OversizedLength {
                length: 2_147_483_647,
                max: 16_777_216
            })
        );
        assert_eq!(
            decode::<ExtensionObject>(&[0x00, 0x07, 0x01, 0xE8, 0x03, 0x00, 0x00, 0x01]),
            Err(ParseError::Truncated {
                needed: 1000,
                available: 1
            })
        );
        assert_eq!(
            decode::<ExtensionObject>(&[0x00, 0x07, 0x01, 0xFE, 0xFF, 0xFF, 0xFF]),
            Err(ParseError::InvalidDiscriminator {
                field: "ByteString length",
                value: 2
            })
        );
    }

    #[test]
    fn opcua_codec_extension_object_spends_one_entry() {
        assert_spends_one_entry(&object(0, ExtensionBody::Null));
        assert_spends_one_entry(&object(5, ExtensionBody::Raw(UaByteString::new(vec![1]))));
        assert_spends_one_entry(&object(5, ExtensionBody::Xml(UaString::new("x"))));
    }

    #[test]
    fn opcua_codec_extension_object_is_refused_when_the_writer_is_full() {
        let raw = object(5, ExtensionBody::Raw(UaByteString::new(vec![1, 2, 3])));
        let mut writer = BoundedWriter::new(8);
        assert_eq!(
            raw.serialize(&mut writer, Depth::new()),
            Err(ParseError::OversizedLength { length: 10, max: 8 })
        );
    }
}
