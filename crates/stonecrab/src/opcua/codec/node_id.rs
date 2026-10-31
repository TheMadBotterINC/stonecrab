//! `NodeId` in its six encodings and `ExpandedNodeId` (OPC UA Part 6, 5.2.2.9 and 5.2.2.10).
//!
//! The first byte names the encoding in its low six bits. A reader accepts all six for any
//! value; a writer takes the most compact one the value fits, so a node id read in a wider
//! encoding than it needs is written back narrower. The top two bits of the byte are
//! reserved in a `NodeId`, read and ignored and written as zero (as the model of the generator
//! treats a reserved field, ADR-0019 S79 addendum); an `ExpandedNodeId` uses them as flags.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, Guid, LITTLE, UaByteString, UaString};

/// The bits of the first byte that name the encoding.
const ENCODING_MASK: u8 = 0x3F;
/// The flag of an `ExpandedNodeId` that a namespace URI follows the node id.
const NAMESPACE_URI_FLAG: u8 = 0x80;
/// The flag of an `ExpandedNodeId` that a server index follows the namespace URI.
const SERVER_INDEX_FLAG: u8 = 0x40;

/// The encoding byte of a two-byte node id: namespace 0, identifier 0 to 255.
const TWO_BYTE: u8 = 0;
/// The encoding byte of a four-byte node id: namespace 0 to 255, identifier 0 to 65535.
const FOUR_BYTE: u8 = 1;
/// The encoding byte of a numeric node id: a 16-bit namespace and a 32-bit identifier.
const NUMERIC: u8 = 2;
/// The encoding byte of a node id with a String identifier.
const STRING: u8 = 3;
/// The encoding byte of a node id with a Guid identifier.
const GUID: u8 = 4;
/// The encoding byte of a node id with a `ByteString` identifier.
const BYTE_STRING: u8 = 5;

/// The identifier of a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::opcua) enum Identifier {
    /// A number (`i=`).
    Numeric(u32),
    /// Text (`s=`).
    String(UaString),
    /// A Guid (`g=`).
    Guid(Guid),
    /// Opaque bytes (`b=`).
    ByteString(UaByteString),
}

/// A node id: a namespace index and an identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::opcua) struct NodeId {
    /// The namespace index.
    pub(in crate::opcua) namespace: u16,
    /// The identifier within the namespace.
    pub(in crate::opcua) identifier: Identifier,
}

/// The encoding a node id is written in, with the parts that encoding carries.
enum Wire<'a> {
    TwoByte(u8),
    FourByte(u8, u16),
    Numeric(u16, u32),
    String(u16, &'a UaString),
    Guid(u16, &'a Guid),
    ByteString(u16, &'a UaByteString),
}

impl Wire<'_> {
    /// The encoding byte, without flags.
    const fn encoding(&self) -> u8 {
        match self {
            Self::TwoByte(_) => TWO_BYTE,
            Self::FourByte(..) => FOUR_BYTE,
            Self::Numeric(..) => NUMERIC,
            Self::String(..) => STRING,
            Self::Guid(..) => GUID,
            Self::ByteString(..) => BYTE_STRING,
        }
    }
}

impl NodeId {
    /// The numeric node id `ns=<namespace>;i=<identifier>`.
    pub(in crate::opcua) const fn numeric(namespace: u16, identifier: u32) -> Self {
        Self {
            namespace,
            identifier: Identifier::Numeric(identifier),
        }
    }

    /// The most compact encoding of the value.
    fn wire(&self) -> Wire<'_> {
        let namespace = self.namespace;
        match &self.identifier {
            Identifier::Numeric(id) => {
                match (
                    u8::try_from(namespace),
                    u8::try_from(*id),
                    u16::try_from(*id),
                ) {
                    (Ok(0), Ok(short), _) => Wire::TwoByte(short),
                    (Ok(small), _, Ok(medium)) => Wire::FourByte(small, medium),
                    _ => Wire::Numeric(namespace, *id),
                }
            }
            Identifier::String(text) => Wire::String(namespace, text),
            Identifier::Guid(guid) => Wire::Guid(namespace, guid),
            Identifier::ByteString(bytes) => Wire::ByteString(namespace, bytes),
        }
    }

    /// Reads the encoding byte and the node id, and returns the byte's two flag bits.
    fn read(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<(Self, u8), ParseError> {
        let first = reader.read_u8()?;
        let flags = first & !ENCODING_MASK;
        let node_id = match first & ENCODING_MASK {
            TWO_BYTE => Self::numeric(0, u32::from(reader.read_u8()?)),
            FOUR_BYTE => {
                let namespace = u16::from(reader.read_u8()?);
                Self::numeric(namespace, u32::from(reader.read_u16(LITTLE)?))
            }
            NUMERIC => {
                let namespace = reader.read_u16(LITTLE)?;
                Self::numeric(namespace, reader.read_u32(LITTLE)?)
            }
            STRING => {
                let namespace = reader.read_u16(LITTLE)?;
                Self {
                    namespace,
                    identifier: Identifier::String(UaString::parse(reader, depth)?),
                }
            }
            GUID => {
                let namespace = reader.read_u16(LITTLE)?;
                Self {
                    namespace,
                    identifier: Identifier::Guid(Guid::parse(reader, depth)?),
                }
            }
            BYTE_STRING => {
                let namespace = reader.read_u16(LITTLE)?;
                Self {
                    namespace,
                    identifier: Identifier::ByteString(UaByteString::parse(reader, depth)?),
                }
            }
            other => {
                return Err(ParseError::InvalidDiscriminator {
                    field: "NodeId encoding",
                    value: u64::from(other),
                });
            }
        };
        Ok((node_id, flags))
    }

    /// Writes the encoding byte with `flags` or-ed in, then the node id.
    fn write(&self, writer: &mut BoundedWriter, depth: Depth, flags: u8) -> Result<(), ParseError> {
        let wire = self.wire();
        writer.write_u8(wire.encoding() | flags)?;
        match wire {
            Wire::TwoByte(id) => writer.write_u8(id),
            Wire::FourByte(namespace, id) => {
                writer.write_u8(namespace)?;
                writer.write_u16(id, LITTLE)
            }
            Wire::Numeric(namespace, id) => {
                writer.write_u16(namespace, LITTLE)?;
                writer.write_u32(id, LITTLE)
            }
            Wire::String(namespace, text) => {
                writer.write_u16(namespace, LITTLE)?;
                text.serialize(writer, depth)
            }
            Wire::Guid(namespace, guid) => {
                writer.write_u16(namespace, LITTLE)?;
                guid.serialize(writer, depth)
            }
            Wire::ByteString(namespace, bytes) => {
                writer.write_u16(namespace, LITTLE)?;
                bytes.serialize(writer, depth)
            }
        }
    }

    /// Reads a `NodeId`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when the bytes are not all present;
    /// [`ParseError::InvalidDiscriminator`] for an encoding above 5 (field `NodeId encoding`)
    /// and the failures of the String and `ByteString` identifiers.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        Self::read(reader, depth).map(|(node_id, _flags)| node_id)
    }

    /// Writes the `NodeId` in its most compact encoding.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        self.write(writer, depth, 0)
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] for an identifier over the String maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        // The size of what follows the encoding byte.
        let rest = match self.wire() {
            Wire::TwoByte(_) => Some(1),
            Wire::FourByte(..) => Some(3),
            Wire::Numeric(..) => Some(6),
            Wire::String(_, text) => text.length_in_bytes(depth)?.checked_add(2),
            Wire::Guid(_, guid) => guid.length_in_bytes(depth)?.checked_add(2),
            Wire::ByteString(_, bytes) => bytes.length_in_bytes(depth)?.checked_add(2),
        };
        rest.and_then(|size| size.checked_add(1))
            .ok_or(ParseError::Overflow)
    }
}

/// An `ExpandedNodeId`: a node id that may name its namespace by URI and its server by index.
///
/// A present namespace URI is written whatever the node id's namespace index is, as the
/// specification has it: the index is then ignored by the receiver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::opcua) struct ExpandedNodeId {
    /// The node id.
    pub(in crate::opcua) node_id: NodeId,
    /// The namespace URI, when the flag is set (the String itself may still be null).
    pub(in crate::opcua) namespace_uri: Option<UaString>,
    /// The index of the server in the server table, when the flag is set.
    pub(in crate::opcua) server_index: Option<u32>,
}

impl ExpandedNodeId {
    /// An `ExpandedNodeId` that is only a node id.
    pub(in crate::opcua) const fn local(node_id: NodeId) -> Self {
        Self {
            node_id,
            namespace_uri: None,
            server_index: None,
        }
    }

    /// Reads an `ExpandedNodeId`.
    ///
    /// # Errors
    /// As [`NodeId::parse`], and [`ParseError::Truncated`] for a missing namespace URI or
    /// server index.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let (node_id, flags) = NodeId::read(reader, depth)?;
        let namespace_uri = if flags & NAMESPACE_URI_FLAG == 0 {
            None
        } else {
            Some(UaString::parse(reader, depth)?)
        };
        let server_index = if flags & SERVER_INDEX_FLAG == 0 {
            None
        } else {
            Some(reader.read_u32(LITTLE)?)
        };
        Ok(Self {
            node_id,
            namespace_uri,
            server_index,
        })
    }

    /// Writes the `ExpandedNodeId`.
    ///
    /// # Errors
    /// As [`NodeId::serialize`].
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let mut flags = 0;
        if self.namespace_uri.is_some() {
            flags |= NAMESPACE_URI_FLAG;
        }
        if self.server_index.is_some() {
            flags |= SERVER_INDEX_FLAG;
        }
        self.node_id.write(writer, depth, flags)?;
        if let Some(uri) = &self.namespace_uri {
            uri.serialize(writer, depth)?;
        }
        if let Some(index) = self.server_index {
            writer.write_u32(index, LITTLE)?;
        }
        Ok(())
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// As [`NodeId::length_in_bytes`].
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let node_id = self.node_id.length_in_bytes(depth)?;
        let uri = match &self.namespace_uri {
            Some(uri) => uri.length_in_bytes(depth)?,
            None => 0,
        };
        let index = if self.server_index.is_some() { 4 } else { 0 };
        node_id
            .checked_add(uri)
            .and_then(|size| size.checked_add(index))
            .ok_or(ParseError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{assert_cut_off, assert_vector, decode, encode};

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

    fn string_id(namespace: u16, text: &str) -> NodeId {
        NodeId {
            namespace,
            identifier: Identifier::String(UaString::new(text)),
        }
    }

    #[test]
    fn opcua_codec_node_id_two_byte_encoding_round_trips() {
        assert_vector(&NodeId::numeric(0, 0), &[0x00, 0x00]);
        // ObjectsFolder.
        assert_vector(&NodeId::numeric(0, 85), &[0x00, 0x55]);
        assert_vector(&NodeId::numeric(0, 255), &[0x00, 0xFF]);
        assert_cut_off(&NodeId::numeric(0, 85));
    }

    #[test]
    fn opcua_codec_node_id_four_byte_encoding_round_trips() {
        // The worked example of Part 6: ns=5;i=1025.
        assert_vector(&NodeId::numeric(5, 1025), &[0x01, 0x05, 0x01, 0x04]);
        // The first identifier that no longer fits one byte, in namespace 0.
        assert_vector(&NodeId::numeric(0, 256), &[0x01, 0x00, 0x00, 0x01]);
        assert_vector(&NodeId::numeric(1, 0), &[0x01, 0x01, 0x00, 0x00]);
        assert_vector(&NodeId::numeric(255, 65535), &[0x01, 0xFF, 0xFF, 0xFF]);
        assert_cut_off(&NodeId::numeric(5, 1025));
    }

    #[test]
    fn opcua_codec_node_id_numeric_encoding_round_trips() {
        assert_vector(
            &NodeId::numeric(1, 1_000_000),
            &[0x02, 0x01, 0x00, 0x40, 0x42, 0x0F, 0x00],
        );
        // One past the four-byte limits on each side.
        assert_vector(
            &NodeId::numeric(256, 0),
            &[0x02, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00],
        );
        assert_vector(
            &NodeId::numeric(0, 65536),
            &[0x02, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00],
        );
        assert_vector(
            &NodeId::numeric(u16::MAX, u32::MAX),
            &[0x02, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        );
        assert_cut_off(&NodeId::numeric(1, 1_000_000));
    }

    #[test]
    fn opcua_codec_node_id_string_encoding_round_trips() {
        let hello = string_id(2, "Hello");
        assert_vector(
            &hello,
            &[
                0x03, 0x02, 0x00, 0x05, 0x00, 0x00, 0x00, 0x48, 0x65, 0x6C, 0x6C, 0x6F,
            ],
        );
        assert_cut_off(&hello);
        let null = NodeId {
            namespace: 1,
            identifier: Identifier::String(UaString::null()),
        };
        assert_vector(&null, &[0x03, 0x01, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_ne!(
            null,
            string_id(1, ""),
            "a null identifier is not an empty one"
        );
    }

    #[test]
    fn opcua_codec_node_id_guid_encoding_round_trips() {
        let node = NodeId {
            namespace: 1,
            identifier: Identifier::Guid(GUID),
        };
        let mut bytes = vec![0x04, 0x01, 0x00];
        bytes.extend_from_slice(&GUID_BYTES);
        assert_vector(&node, &bytes);
        assert_cut_off(&node);
    }

    #[test]
    fn opcua_codec_node_id_byte_string_encoding_round_trips() {
        let node = NodeId {
            namespace: 1,
            identifier: Identifier::ByteString(UaByteString::new([1, 2, 3])),
        };
        assert_vector(
            &node,
            &[0x05, 0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03],
        );
        assert_cut_off(&node);
    }

    #[test]
    fn opcua_codec_node_id_reads_any_encoding_and_writes_the_most_compact() {
        let objects = NodeId::numeric(0, 85);
        for wide in [
            &[0x01, 0x00, 0x55, 0x00][..],
            &[0x02, 0x00, 0x00, 0x55, 0x00, 0x00, 0x00],
        ] {
            assert_eq!(decode::<NodeId>(wide), Ok(objects.clone()), "{wide:?}");
        }
        // The two reserved bits of a NodeId are ignored and written as zero.
        assert_eq!(decode::<NodeId>(&[0xC0, 0x55]), Ok(objects.clone()));
        assert_eq!(encode(&objects), [0x00, 0x55]);
    }

    #[test]
    fn opcua_codec_node_id_unassigned_encoding_is_refused() {
        for encoding in 6..=0x3F_u8 {
            assert_eq!(
                decode::<NodeId>(&[encoding, 0, 0, 0, 0, 0, 0]),
                Err(ParseError::InvalidDiscriminator {
                    field: "NodeId encoding",
                    value: u64::from(encoding),
                }),
                "{encoding}"
            );
        }
    }

    #[test]
    fn opcua_codec_node_id_failures_in_the_identifier_are_the_identifiers() {
        assert_eq!(
            decode::<NodeId>(&[0x03, 0x01, 0x00, 0xFE, 0xFF, 0xFF, 0xFF]),
            Err(ParseError::InvalidDiscriminator {
                field: "String length",
                value: 2
            })
        );
        assert_eq!(
            decode::<NodeId>(&[0x05, 0x01, 0x00, 0xFE, 0xFF, 0xFF, 0xFF]),
            Err(ParseError::InvalidDiscriminator {
                field: "ByteString length",
                value: 2
            })
        );
    }

    #[test]
    fn opcua_codec_node_id_length_in_bytes_follows_the_encoding() {
        let depth = Depth::new();
        assert_eq!(NodeId::numeric(0, 255).length_in_bytes(depth), Ok(2));
        assert_eq!(NodeId::numeric(0, 256).length_in_bytes(depth), Ok(4));
        assert_eq!(NodeId::numeric(256, 0).length_in_bytes(depth), Ok(7));
        assert_eq!(string_id(0, "ab").length_in_bytes(depth), Ok(1 + 2 + 4 + 2));
        let oversized = NodeId {
            namespace: 0,
            identifier: Identifier::ByteString(UaByteString::new(vec![
                0;
                crate::opcua::codec::MAX_STRING_BYTES
                    + 1
            ])),
        };
        assert!(matches!(
            oversized.length_in_bytes(depth),
            Err(ParseError::OversizedLength { .. })
        ));
    }

    #[test]
    fn opcua_codec_expanded_node_id_i321_is_01004101() {
        let node = ExpandedNodeId::local(NodeId::numeric(0, 321));
        assert_vector(&node, &[0x01, 0x00, 0x41, 0x01]);
        assert_cut_off(&node);
    }

    #[test]
    fn opcua_codec_expanded_node_id_flags_say_what_follows_the_node_id() {
        let uri = ExpandedNodeId {
            node_id: string_id(2, "Hi"),
            namespace_uri: Some(UaString::new("urn:a")),
            server_index: None,
        };
        assert_vector(
            &uri,
            &[
                0x83, 0x02, 0x00, 0x02, 0x00, 0x00, 0x00, 0x48, 0x69, 0x05, 0x00, 0x00, 0x00, 0x75,
                0x72, 0x6E, 0x3A, 0x61,
            ],
        );
        assert_cut_off(&uri);

        let server = ExpandedNodeId {
            node_id: NodeId::numeric(0, 85),
            namespace_uri: None,
            server_index: Some(7),
        };
        assert_vector(&server, &[0x40, 0x55, 0x07, 0x00, 0x00, 0x00]);
        assert_cut_off(&server);

        let both = ExpandedNodeId {
            node_id: NodeId::numeric(0, 321),
            namespace_uri: Some(UaString::new("urn")),
            server_index: Some(2),
        };
        assert_vector(
            &both,
            &[
                0xC1, 0x00, 0x41, 0x01, 0x03, 0x00, 0x00, 0x00, 0x75, 0x72, 0x6E, 0x02, 0x00, 0x00,
                0x00,
            ],
        );
        assert_cut_off(&both);
    }

    #[test]
    fn opcua_codec_expanded_node_id_null_namespace_uri_is_still_present() {
        let node = ExpandedNodeId {
            node_id: NodeId::numeric(0, 85),
            namespace_uri: Some(UaString::null()),
            server_index: None,
        };
        assert_vector(&node, &[0x80, 0x55, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_ne!(node, ExpandedNodeId::local(NodeId::numeric(0, 85)));
    }

    #[test]
    fn opcua_codec_expanded_node_id_flags_do_not_excuse_an_unassigned_encoding() {
        assert_eq!(
            decode::<ExpandedNodeId>(&[0xC6, 0, 0, 0, 0, 0, 0]),
            Err(ParseError::InvalidDiscriminator {
                field: "NodeId encoding",
                value: 6
            })
        );
        // A NodeId ignores the flags an ExpandedNodeId would act on, so the URI that follows
        // is left unread.
        let mut reader = BoundedReader::new(&[0x80, 0x55, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(
            NodeId::parse(&mut reader, Depth::new()),
            Ok(NodeId::numeric(0, 85))
        );
        assert_eq!(reader.remaining(), 4);
    }

    #[test]
    fn opcua_codec_expanded_node_id_length_in_bytes_adds_the_optional_parts() {
        let depth = Depth::new();
        let node = ExpandedNodeId {
            node_id: NodeId::numeric(0, 85),
            namespace_uri: Some(UaString::new("urn")),
            server_index: Some(1),
        };
        assert_eq!(node.length_in_bytes(depth), Ok(2 + 4 + 3 + 4));
        let oversized = ExpandedNodeId {
            namespace_uri: Some(UaString::new(
                "a".repeat(crate::opcua::codec::MAX_STRING_BYTES + 1),
            )),
            ..node
        };
        assert!(matches!(
            oversized.length_in_bytes(depth),
            Err(ParseError::OversizedLength { .. })
        ));
    }
}
