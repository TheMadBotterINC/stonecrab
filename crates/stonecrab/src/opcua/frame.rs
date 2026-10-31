//! The OPC UA TCP framing (OPC UA Part 6, 6.7 and 7.1): the 8-byte message header, the six
//! message types and the [`OpcuaFramer`] that finds single-chunk messages in a byte stream.
//!
//! Every message starts with a header of a three-character message type (`HEL`, `ACK`, `ERR`,
//! `OPN`, `CLO` or `MSG`), a one-character chunk flag (`C` intermediate, `F` final, `A` abort)
//! and the little-endian `UInt32` size of the whole message, the header included. Hello and
//! Acknowledge carry the transport limits; Error carries a status code and a reason. The
//! secure-channel messages carry the secure channel id, a security header and the sequence
//! header in front of the payload: `OpenSecureChannel` the asymmetric header (the policy URI,
//! the sender certificate and the receiver certificate thumbprint, 59 bytes for the policy
//! None), `CloseSecureChannel` and Message the symmetric one (the token id). The payload stays
//! bytes here; the chunking slice (S94) reassembles chunks and the generated service layer
//! (S88) reads the payload.
//!
//! The framer takes the size from the header and compares it with the receive buffer and
//! `ResourceLimits::max_frame_bytes` before it waits for or copies anything, so a size that a
//! peer declares never reserves memory (upstream's `OpcuaMessageCodec` reads it unchecked). It
//! reads whole messages: a message whose chunk flag is `C` or `A` is returned as it is.

use std::fmt;

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::ResourceLimits;
use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE, StatusCode, UaByteString, UaString};
use crate::transport::{FrameError, Framer};

/// The size of the message header on the wire.
const HEADER_BYTES: usize = 8;

/// [`HEADER_BYTES`] as the `UInt32` the size field carries.
const HEADER_SIZE: u32 = 8;

/// The largest size this codec accepts in a header, `Int32::MAX`: upstream reads the field as
/// a signed number, so the sizes above this are negative there.
const MAX_SIZE: u32 = 0x7FFF_FFFF;

/// The policy URI that the asymmetric header of policy None carries.
pub(super) const NONE_POLICY_URI: &str = "http://opcfoundation.org/UA/SecurityPolicy#None";

/// The message types of the header (Part 6, 7.1.2.2 and 6.7.2).
///
/// Upstream's Javadoc also lists RHE (reverse hello), which its mspec does not have and
/// which is not implemented here: it is an unknown type like any other.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-MESSAGE-TYPES
/// The six types and the three chunk flags of the mspec; RHE is not reproduced
/// (docs/parity/opcua.md#OPCUA-QUIRK-CODEC-RHE).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MessageType {
    /// `HEL`: the client's Hello.
    Hello,
    /// `ACK`: the server's Acknowledge.
    Acknowledge,
    /// `ERR`: an error that ends the connection.
    Error,
    /// `OPN`: `OpenSecureChannel`, request or response.
    Open,
    /// `CLO`: `CloseSecureChannel`.
    Close,
    /// `MSG`: a service request or response.
    Message,
}

impl MessageType {
    /// The three characters of the type.
    const fn code(self) -> [u8; 3] {
        match self {
            Self::Hello => *b"HEL",
            Self::Acknowledge => *b"ACK",
            Self::Error => *b"ERR",
            Self::Open => *b"OPN",
            Self::Close => *b"CLO",
            Self::Message => *b"MSG",
        }
    }

    /// The type whose characters are `code`, which are case-sensitive.
    fn from_code(code: [u8; 3]) -> Result<Self, ParseError> {
        [
            Self::Hello,
            Self::Acknowledge,
            Self::Error,
            Self::Open,
            Self::Close,
            Self::Message,
        ]
        .into_iter()
        .find(|message_type| message_type.code() == code)
        .ok_or_else(|| {
            let [first, second, third] = code;
            ParseError::InvalidDiscriminator {
                field: "message type",
                value: u64::from(u32::from_le_bytes([first, second, third, 0])),
            }
        })
    }
}

/// The chunk flag of the header (Part 6, 6.7.2.2): where a chunk sits in its message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ChunkType {
    /// `C`: an intermediate chunk, more follow.
    Intermediate,
    /// `F`: the final chunk, or the only one.
    Final,
    /// `A`: the sender abandons the message.
    Abort,
}

impl ChunkType {
    /// The character of the flag.
    const fn code(self) -> u8 {
        match self {
            Self::Intermediate => b'C',
            Self::Final => b'F',
            Self::Abort => b'A',
        }
    }

    /// The flag whose character is `code`.
    fn from_code(code: u8) -> Result<Self, ParseError> {
        [Self::Intermediate, Self::Final, Self::Abort]
            .into_iter()
            .find(|chunk| chunk.code() == code)
            .ok_or(ParseError::InvalidDiscriminator {
                field: "chunk type",
                value: u64::from(code),
            })
    }
}

/// The 8-byte header in front of every message.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-HEADER
/// The same three fields as upstream's `OpcuaMessageCodec`, which reads the size unchecked;
/// here a size below 8 or above `Int32::MAX` is refused when the header is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FrameHeader {
    /// The kind of message.
    pub(super) message_type: MessageType,
    /// The chunk flag.
    pub(super) chunk: ChunkType,
    /// The size of the message in bytes, the header included: 8 to `Int32::MAX`.
    pub(super) size: u32,
}

impl FrameHeader {
    /// Reads a header.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when fewer than 8 bytes remain;
    /// [`ParseError::InvalidDiscriminator`] for a message type or chunk flag that is not one
    /// of the six and three above (fields `message type` and `chunk type`) and for a size
    /// below 8 or above `Int32::MAX` (field `message size`).
    pub(super) fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        let mut code = [0; 3];
        for byte in &mut code {
            *byte = reader.read_u8()?;
        }
        let message_type = MessageType::from_code(code)?;
        let chunk = ChunkType::from_code(reader.read_u8()?)?;
        let size = reader.read_u32(LITTLE)?;
        if !(HEADER_SIZE..=MAX_SIZE).contains(&size) {
            return Err(ParseError::InvalidDiscriminator {
                field: "message size",
                value: u64::from(size),
            });
        }
        Ok(Self {
            message_type,
            chunk,
            size,
        })
    }

    /// Writes the header.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the writer's capacity is reached.
    pub(super) fn serialize(self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_bytes(&self.message_type.code())?;
        writer.write_u8(self.chunk.code())?;
        writer.write_u32(self.size, LITTLE)
    }
}

/// The transport limits that Hello proposes and Acknowledge answers (Part 6, 7.1.2.3 and
/// 7.1.2.4), in bytes and chunks. A server may send 0 for the maximum message size and the
/// maximum chunk count, which means no limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TransportLimits {
    /// The largest chunk the sender can receive.
    pub(super) receive_buffer_size: u32,
    /// The largest chunk the sender will send.
    pub(super) send_buffer_size: u32,
    /// The largest message the sender accepts, over all its chunks.
    pub(super) max_message_size: u32,
    /// The most chunks in a message the sender accepts.
    pub(super) max_chunk_count: u32,
}

impl TransportLimits {
    /// The encoded size: four `UInt32`s.
    const SIZE: usize = 16;

    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            receive_buffer_size: reader.read_u32(LITTLE)?,
            send_buffer_size: reader.read_u32(LITTLE)?,
            max_message_size: reader.read_u32(LITTLE)?,
            max_chunk_count: reader.read_u32(LITTLE)?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.receive_buffer_size, LITTLE)?;
        writer.write_u32(self.send_buffer_size, LITTLE)?;
        writer.write_u32(self.max_message_size, LITTLE)?;
        writer.write_u32(self.max_chunk_count, LITTLE)
    }
}

/// The body of `HEL`: the protocol version, the limits and the endpoint URL being connected to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hello {
    /// The protocol version, 0 for every version published so far.
    pub(super) protocol_version: u32,
    /// The limits the client proposes.
    pub(super) limits: TransportLimits,
    /// The URL of the endpoint (at most 4096 bytes, checked when the options are parsed).
    pub(super) endpoint_url: UaString,
}

impl Hello {
    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            protocol_version: reader.read_u32(LITTLE)?,
            limits: TransportLimits::parse(reader)?,
            endpoint_url: UaString::parse(reader, Depth::new())?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.protocol_version, LITTLE)?;
        self.limits.serialize(writer)?;
        self.endpoint_url.serialize(writer, Depth::new())
    }

    fn length_in_bytes(&self) -> Result<usize, ParseError> {
        self.endpoint_url
            .length_in_bytes(Depth::new())?
            .checked_add(4 + TransportLimits::SIZE)
            .ok_or(ParseError::Overflow)
    }
}

/// The body of `ACK`: the protocol version and the limits the server answers with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Acknowledge {
    /// The protocol version the server speaks.
    pub(super) protocol_version: u32,
    /// The limits of the server.
    pub(super) limits: TransportLimits,
}

impl Acknowledge {
    /// The encoded size: the version and the four limits.
    const SIZE: usize = 4 + TransportLimits::SIZE;

    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            protocol_version: reader.read_u32(LITTLE)?,
            limits: TransportLimits::parse(reader)?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.protocol_version, LITTLE)?;
        self.limits.serialize(writer)
    }
}

/// The body of `ERR`: a status code and a reason, after which the server closes the connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ErrorMessage {
    /// The status code.
    pub(super) error: StatusCode,
    /// The reason, which may be null; its text is the server's, not checked.
    pub(super) reason: UaString,
}

impl ErrorMessage {
    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            error: StatusCode::parse(reader, Depth::new())?,
            reason: UaString::parse(reader, Depth::new())?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        self.error.serialize(writer, Depth::new())?;
        self.reason.serialize(writer, Depth::new())
    }

    fn length_in_bytes(&self) -> Result<usize, ParseError> {
        self.reason
            .length_in_bytes(Depth::new())?
            .checked_add(4)
            .ok_or(ParseError::Overflow)
    }
}

/// The asymmetric algorithm security header of `OPN` (Part 6, 6.7.2.3): the security policy
/// URI, the sender's certificate and the thumbprint of the receiver's.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-OPEN-HEADER
/// 59 bytes for the policy None, as upstream's `ASYMMETRIC_SECURITY_HEADER_SIZE`, and 12 plus
/// the lengths of the URI, the certificate and the thumbprint otherwise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AsymmetricHeader {
    /// The URI of the security policy.
    pub(super) security_policy_uri: UaString,
    /// The sender's certificate, null for the policy None.
    pub(super) sender_certificate: UaByteString,
    /// The SHA-1 thumbprint of the receiver's certificate, null for the policy None.
    pub(super) receiver_thumbprint: UaByteString,
}

impl AsymmetricHeader {
    /// The header of the policy None: its URI and a null certificate and thumbprint, 59 bytes.
    pub(super) fn none() -> Self {
        Self {
            security_policy_uri: UaString::new(NONE_POLICY_URI),
            sender_certificate: UaByteString::null(),
            receiver_thumbprint: UaByteString::null(),
        }
    }

    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            security_policy_uri: UaString::parse(reader, Depth::new())?,
            sender_certificate: UaByteString::parse(reader, Depth::new())?,
            receiver_thumbprint: UaByteString::parse(reader, Depth::new())?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        self.security_policy_uri.serialize(writer, Depth::new())?;
        self.sender_certificate.serialize(writer, Depth::new())?;
        self.receiver_thumbprint.serialize(writer, Depth::new())
    }

    /// The encoded size: 12 for the three length prefixes and the bytes of the URI, the
    /// certificate and the thumbprint.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] for a part over the String maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(super) fn length_in_bytes(&self) -> Result<usize, ParseError> {
        let depth = Depth::new();
        let uri = self.security_policy_uri.length_in_bytes(depth)?;
        let certificate = self.sender_certificate.length_in_bytes(depth)?;
        let thumbprint = self.receiver_thumbprint.length_in_bytes(depth)?;
        uri.checked_add(certificate)
            .and_then(|length| length.checked_add(thumbprint))
            .ok_or(ParseError::Overflow)
    }
}

/// The sequence header in front of every payload (Part 6, 6.7.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SequenceHeader {
    /// The number of the chunk, which grows by one per chunk.
    pub(super) sequence_number: u32,
    /// The id of the request the chunk belongs to, the same in a response.
    pub(super) request_id: u32,
}

impl SequenceHeader {
    /// The encoded size: two `UInt32`s.
    const SIZE: usize = 8;

    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            sequence_number: reader.read_u32(LITTLE)?,
            request_id: reader.read_u32(LITTLE)?,
        })
    }

    fn serialize(self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.sequence_number, LITTLE)?;
        writer.write_u32(self.request_id, LITTLE)
    }
}

/// Every byte the reader has left, copied.
fn rest(reader: &mut BoundedReader<'_>) -> Result<Bytes, ParseError> {
    let remaining = reader.remaining();
    reader
        .read_bytes(remaining, remaining)
        .map(Bytes::copy_from_slice)
}

/// The body of `OPN`: `OpenSecureChannel` in a request or a response chunk.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct OpenChunk {
    /// The secure channel id: 0 in the first request, the server's choice after.
    pub(super) channel_id: u32,
    /// The asymmetric security header.
    pub(super) security: AsymmetricHeader,
    /// The sequence header.
    pub(super) sequence: SequenceHeader,
    /// The payload: the request or response, which may be encrypted.
    pub(super) body: Bytes,
}

impl fmt::Debug for OpenChunk {
    /// Shows the payload by length only: after the security slices it holds ciphertext.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenChunk")
            .field("channel_id", &self.channel_id)
            .field("security", &self.security)
            .field("sequence", &self.sequence)
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl OpenChunk {
    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            channel_id: reader.read_u32(LITTLE)?,
            security: AsymmetricHeader::parse(reader)?,
            sequence: SequenceHeader::parse(reader)?,
            body: rest(reader)?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.channel_id, LITTLE)?;
        self.security.serialize(writer)?;
        self.sequence.serialize(writer)?;
        writer.write_bytes(&self.body)
    }

    fn length_in_bytes(&self) -> Result<usize, ParseError> {
        self.security
            .length_in_bytes()?
            .checked_add(4 + SequenceHeader::SIZE)
            .and_then(|length| length.checked_add(self.body.len()))
            .ok_or(ParseError::Overflow)
    }
}

/// The body of `CLO` and `MSG`: a chunk under the symmetric security header.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct SecureChunk {
    /// The secure channel id.
    pub(super) channel_id: u32,
    /// The id of the security token the chunk is protected with.
    pub(super) token_id: u32,
    /// The sequence header.
    pub(super) sequence: SequenceHeader,
    /// The payload, which may be encrypted.
    pub(super) body: Bytes,
}

impl fmt::Debug for SecureChunk {
    /// Shows the payload by length only.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecureChunk")
            .field("channel_id", &self.channel_id)
            .field("token_id", &self.token_id)
            .field("sequence", &self.sequence)
            .field("body_len", &self.body.len())
            .finish()
    }
}

impl SecureChunk {
    /// The bytes in front of the payload: the channel id, the token id and the sequence header.
    const HEADERS: usize = 8 + SequenceHeader::SIZE;

    fn parse(reader: &mut BoundedReader<'_>) -> Result<Self, ParseError> {
        Ok(Self {
            channel_id: reader.read_u32(LITTLE)?,
            token_id: reader.read_u32(LITTLE)?,
            sequence: SequenceHeader::parse(reader)?,
            body: rest(reader)?,
        })
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        writer.write_u32(self.channel_id, LITTLE)?;
        writer.write_u32(self.token_id, LITTLE)?;
        self.sequence.serialize(writer)?;
        writer.write_bytes(&self.body)
    }

    fn length_in_bytes(&self) -> Result<usize, ParseError> {
        self.body
            .len()
            .checked_add(Self::HEADERS)
            .ok_or(ParseError::Overflow)
    }
}

/// The body of a message, by its type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Body {
    /// `HEL`.
    Hello(Hello),
    /// `ACK`.
    Acknowledge(Acknowledge),
    /// `ERR`.
    Error(ErrorMessage),
    /// `OPN`.
    Open(OpenChunk),
    /// `CLO`.
    Close(SecureChunk),
    /// `MSG`.
    Message(SecureChunk),
}

impl Body {
    /// The type that the header of this body carries.
    pub(super) const fn message_type(&self) -> MessageType {
        match self {
            Self::Hello(_) => MessageType::Hello,
            Self::Acknowledge(_) => MessageType::Acknowledge,
            Self::Error(_) => MessageType::Error,
            Self::Open(_) => MessageType::Open,
            Self::Close(_) => MessageType::Close,
            Self::Message(_) => MessageType::Message,
        }
    }

    /// Reads the body of a message of `message_type` from every byte of `bytes`.
    fn parse(message_type: MessageType, bytes: &[u8]) -> Result<Self, ParseError> {
        let mut reader = BoundedReader::new(bytes);
        let body = match message_type {
            MessageType::Hello => Self::Hello(Hello::parse(&mut reader)?),
            MessageType::Acknowledge => Self::Acknowledge(Acknowledge::parse(&mut reader)?),
            MessageType::Error => Self::Error(ErrorMessage::parse(&mut reader)?),
            MessageType::Open => Self::Open(OpenChunk::parse(&mut reader)?),
            MessageType::Close => Self::Close(SecureChunk::parse(&mut reader)?),
            MessageType::Message => Self::Message(SecureChunk::parse(&mut reader)?),
        };
        reader.expect_eof()?;
        Ok(body)
    }

    fn serialize(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        match self {
            Self::Hello(hello) => hello.serialize(writer),
            Self::Acknowledge(acknowledge) => acknowledge.serialize(writer),
            Self::Error(error) => error.serialize(writer),
            Self::Open(chunk) => chunk.serialize(writer),
            Self::Close(chunk) | Self::Message(chunk) => chunk.serialize(writer),
        }
    }

    fn length_in_bytes(&self) -> Result<usize, ParseError> {
        match self {
            Self::Hello(hello) => hello.length_in_bytes(),
            Self::Acknowledge(_) => Ok(Acknowledge::SIZE),
            Self::Error(error) => error.length_in_bytes(),
            Self::Open(chunk) => chunk.length_in_bytes(),
            Self::Close(chunk) | Self::Message(chunk) => chunk.length_in_bytes(),
        }
    }
}

/// One message as it crosses the wire: the chunk flag of its header and its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Frame {
    /// The chunk flag; `F` for a message of one chunk.
    pub(super) chunk: ChunkType,
    /// The body.
    pub(super) body: Body,
}

impl Frame {
    /// A message that is the only chunk of its message.
    pub(super) const fn single(body: Body) -> Self {
        Self {
            chunk: ChunkType::Final,
            body,
        }
    }

    /// The size on the wire, the header included.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] for a String or `ByteString` over the maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(super) fn length_in_bytes(&self) -> Result<usize, ParseError> {
        self.body
            .length_in_bytes()?
            .checked_add(HEADER_BYTES)
            .ok_or(ParseError::Overflow)
    }

    /// Writes the header, whose size field is `size`, and the body.
    fn serialize(&self, size: u32, writer: &mut BoundedWriter) -> Result<(), ParseError> {
        FrameHeader {
            message_type: self.body.message_type(),
            chunk: self.chunk,
            size,
        }
        .serialize(writer)?;
        self.body.serialize(writer)
    }
}

/// How `encode` reports a failure of the codec: a length over a bound is `Oversized`, and
/// everything else means the value is not a message.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a function that `map_err` takes by name receives the error by value"
)]
const fn frame_error(error: ParseError) -> FrameError {
    match error {
        ParseError::OversizedLength { length, .. } => FrameError::Oversized(length),
        ParseError::Truncated { .. }
        | ParseError::InvalidDiscriminator { .. }
        | ParseError::TrailingBytes { .. }
        | ParseError::Overflow
        | ParseError::CountLimit { .. }
        | ParseError::RecursionLimit { .. } => FrameError::Malformed,
    }
}

/// The [`Framer`] of the OPC UA TCP protocol: finds messages in a byte stream and writes them
/// back.
///
/// It holds only the largest message it accepts or writes, which is the smaller of the
/// receive buffer that Hello declares, `ResourceLimits::max_frame_bytes` and `Int32::MAX`. The rules, in the
/// order `decode` applies them:
/// 1. Fewer than 8 bytes buffered: `Ok(None)`, wait for the header.
/// 2. The header is judged as soon as it is complete: an unknown message type or chunk flag
///    (RHE included) or a size below 8 or above `Int32::MAX` is [`FrameError::Malformed`],
///    and a size above the largest message is [`FrameError::Oversized`] carrying the declared
///    size. Nothing is allocated or awaited for either.
/// 3. The message is incomplete: `Ok(None)`; the buffer is left as it was.
/// 4. A complete message whose body is not valid for its type (a short body, a String that
///    runs past it, a declared length over the String maximum, bytes left over in `HEL`,
///    `ACK` or `ERR`) is [`FrameError::Malformed`] and the buffer is left as it was: the frame
///    is within the limit, so the length is what is wrong.
/// 5. Otherwise exactly the message's bytes are removed from the front of the buffer.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-WIRE-HEADER
/// Upstream sizes a message from header bytes 4 to 7 and bounds nothing; the bounds here are
/// the receive buffer of Hello and `ResourceLimits::max_frame_bytes`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct OpcuaFramer {
    /// The largest message in bytes, the header included.
    max_frame_bytes: u32,
}

impl OpcuaFramer {
    /// A framer for a connection whose receive buffer is `receive_buffer_size` bytes.
    pub(super) fn new(receive_buffer_size: u32, limits: ResourceLimits) -> Self {
        let configured = u32::try_from(limits.max_frame_bytes().get()).unwrap_or(u32::MAX);
        Self {
            max_frame_bytes: receive_buffer_size.min(configured).min(MAX_SIZE),
        }
    }
}

impl Framer<Frame> for OpcuaFramer {
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Frame>, FrameError> {
        let Some(head) = buffer.get(..HEADER_BYTES) else {
            return Ok(None);
        };
        let header = FrameHeader::parse(&mut BoundedReader::new(head))
            .map_err(|_error| FrameError::Malformed)?;
        let total = usize::try_from(header.size).unwrap_or(usize::MAX);
        if header.size > self.max_frame_bytes {
            return Err(FrameError::Oversized(total));
        }
        let Some(message) = buffer.get(..total) else {
            return Ok(None);
        };
        let body = message.get(HEADER_BYTES..).ok_or(FrameError::Truncated)?;
        // The size is already bounded, so a length the body cannot hold is a malformed body.
        let body =
            Body::parse(header.message_type, body).map_err(|_error| FrameError::Malformed)?;
        buffer.advance(total);
        Ok(Some(Frame {
            chunk: header.chunk,
            body,
        }))
    }

    fn encode(&mut self, frame: &Frame, out: &mut BytesMut) -> Result<(), FrameError> {
        let total = frame.length_in_bytes().map_err(frame_error)?;
        let size = match u32::try_from(total) {
            Ok(size) if size <= self.max_frame_bytes => size,
            _ => return Err(FrameError::Oversized(total)),
        };
        let mut writer = BoundedWriter::new(total);
        // The size was measured from the same value, so this cannot fail; it is mapped anyway.
        frame.serialize(size, &mut writer).map_err(frame_error)?;
        out.put_slice(writer.as_slice());
        Ok(())
    }

    fn max_frame_bytes(&self) -> usize {
        usize::try_from(self.max_frame_bytes).unwrap_or(usize::MAX)
    }
}

#[cfg(test)]
mod properties;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorKind;
    use crate::PlcError;

    /// The framer of a connection with the default receive buffer of upstream (65535) and the
    /// default `ResourceLimits`.
    fn framer() -> OpcuaFramer {
        OpcuaFramer::new(65_535, limits(65_536))
    }

    fn limits(max_frame_bytes: usize) -> ResourceLimits {
        ResourceLimits::builder()
            .max_frame_bytes(max_frame_bytes)
            .max_buffered_bytes(max_frame_bytes.saturating_mul(4))
            .build()
            .expect("positive limits")
    }

    fn decode(bytes: &[u8]) -> Result<Option<Frame>, FrameError> {
        framer().decode(&mut BytesMut::from(bytes))
    }

    fn encode(frame: &Frame) -> Vec<u8> {
        let mut out = BytesMut::new();
        framer().encode(frame, &mut out).expect("encodes");
        out.to_vec()
    }

    /// The frame `bytes` hold, which is all of them.
    fn whole(bytes: &[u8]) -> Frame {
        let mut buffer = BytesMut::from(bytes);
        let frame = framer()
            .decode(&mut buffer)
            .expect("decodes")
            .expect("a whole message");
        assert!(buffer.is_empty(), "the message used every byte");
        frame
    }

    /// `frame` encodes to exactly `bytes` and `bytes` decode to `frame`, apart.
    fn assert_vector(frame: &Frame, bytes: &[u8]) {
        assert_eq!(encode(frame), bytes, "{frame:?}: encoded bytes");
        assert_eq!(&whole(bytes), frame, "{frame:?}: decoded frame");
        assert_eq!(
            frame.length_in_bytes(),
            Ok(bytes.len()),
            "{frame:?}: length_in_bytes"
        );
    }

    /// A header of the given characters and size, then `rest`.
    fn message(code: [u8; 4], size: u32, rest: &[u8]) -> Vec<u8> {
        [code.as_slice(), &size.to_le_bytes(), rest].concat()
    }

    fn le(numbers: &[u32]) -> Vec<u8> {
        numbers
            .iter()
            .flat_map(|number| number.to_le_bytes())
            .collect()
    }

    /// Part 6, 7.1.2.3: Hello with receive 65535, send 65535, 4 MiB messages, 16 chunks for
    /// `opc.tcp://h:4840`.
    fn hello_bytes() -> Vec<u8> {
        let rest = [
            le(&[0, 65_535, 65_535, 0x0040_0000, 16, 16]),
            b"opc.tcp://h:4840".to_vec(),
        ]
        .concat();
        message(*b"HELF", 48, &rest)
    }

    fn hello_frame() -> Frame {
        Frame::single(Body::Hello(Hello {
            protocol_version: 0,
            limits: TransportLimits {
                receive_buffer_size: 65_535,
                send_buffer_size: 65_535,
                max_message_size: 4 * 1024 * 1024,
                max_chunk_count: 16,
            },
            endpoint_url: UaString::new("opc.tcp://h:4840"),
        }))
    }

    /// Part 6, 7.1.2.4: Acknowledge with four distinct limits.
    fn acknowledge_bytes() -> Vec<u8> {
        message(*b"ACKF", 28, &le(&[0, 8192, 16_384, 1_048_576, 64]))
    }

    fn acknowledge_frame() -> Frame {
        Frame::single(Body::Acknowledge(Acknowledge {
            protocol_version: 0,
            limits: TransportLimits {
                receive_buffer_size: 8192,
                send_buffer_size: 16_384,
                max_message_size: 1_048_576,
                max_chunk_count: 64,
            },
        }))
    }

    /// Part 6, 7.1.2.5: Error `BadUnexpectedError` (0x80010000) with the reason "bad".
    fn error_bytes() -> Vec<u8> {
        message(
            *b"ERRF",
            19,
            &[le(&[0x8001_0000, 3]), b"bad".to_vec()].concat(),
        )
    }

    fn error_frame() -> Frame {
        Frame::single(Body::Error(ErrorMessage {
            error: StatusCode(0x8001_0000),
            reason: UaString::new("bad"),
        }))
    }

    /// `OpenSecureChannel` for the policy None, channel 0, sequence 1, request 1, payload
    /// `AA BB CC`.
    fn open_bytes() -> Vec<u8> {
        let rest = [
            le(&[0, 47]),
            NONE_POLICY_URI.as_bytes().to_vec(),
            vec![0xFF; 8],
            le(&[1, 1]),
            vec![0xAA, 0xBB, 0xCC],
        ]
        .concat();
        message(*b"OPNF", 82, &rest)
    }

    fn open_frame() -> Frame {
        Frame::single(Body::Open(OpenChunk {
            channel_id: 0,
            security: AsymmetricHeader::none(),
            sequence: SequenceHeader {
                sequence_number: 1,
                request_id: 1,
            },
            body: Bytes::from_static(&[0xAA, 0xBB, 0xCC]),
        }))
    }

    /// A Message on channel 7 under token 9, sequence 3, request 4, payload `01 02 03 04 05`.
    fn message_bytes() -> Vec<u8> {
        message(
            *b"MSGF",
            29,
            &[le(&[7, 9, 3, 4]), vec![1, 2, 3, 4, 5]].concat(),
        )
    }

    fn secure_chunk(body: &'static [u8], sequence: u32, request: u32) -> SecureChunk {
        SecureChunk {
            channel_id: 7,
            token_id: 9,
            sequence: SequenceHeader {
                sequence_number: sequence,
                request_id: request,
            },
            body: Bytes::from_static(body),
        }
    }

    fn message_frame() -> Frame {
        Frame::single(Body::Message(secure_chunk(&[1, 2, 3, 4, 5], 3, 4)))
    }

    /// A Close on channel 7 under token 9, sequence 5, request 6, no payload.
    fn close_bytes() -> Vec<u8> {
        message(*b"CLOF", 24, &le(&[7, 9, 5, 6]))
    }

    fn close_frame() -> Frame {
        Frame::single(Body::Close(secure_chunk(&[], 5, 6)))
    }

    fn all_frames() -> Vec<(Frame, Vec<u8>)> {
        vec![
            (hello_frame(), hello_bytes()),
            (acknowledge_frame(), acknowledge_bytes()),
            (error_frame(), error_bytes()),
            (open_frame(), open_bytes()),
            (close_frame(), close_bytes()),
            (message_frame(), message_bytes()),
        ]
    }

    #[test]
    fn opcua_framer_reads_the_eight_byte_header_and_the_body() {
        let mut buffer = BytesMut::from(message_bytes().as_slice());
        buffer.extend_from_slice(b"MSGF");
        let frame = framer()
            .decode(&mut buffer)
            .expect("decodes")
            .expect("one whole message");
        assert_eq!(frame, message_frame());
        assert_eq!(&buffer[..], b"MSGF", "the bytes of the next message stay");
        assert_eq!(
            framer().decode(&mut buffer),
            Ok(None),
            "and wait for a complete header"
        );
        assert_eq!(
            &buffer[..],
            *b"MSGF",
            "an incomplete header leaves the buffer as it was"
        );
        let mut empty = BytesMut::new();
        assert_eq!(
            framer().decode(&mut empty),
            Ok(None),
            "an empty buffer waits"
        );
    }

    #[test]
    fn opcua_frame_a_message_cut_anywhere_waits_for_the_rest() {
        for (_, bytes) in all_frames() {
            for cut in 0..bytes.len() {
                let mut buffer = BytesMut::from(bytes.get(..cut).expect("a prefix"));
                assert_eq!(framer().decode(&mut buffer), Ok(None), "cut at {cut}");
                assert_eq!(buffer.len(), cut, "cut at {cut}: nothing is consumed");
            }
        }
    }

    #[test]
    fn opcua_framer_rejects_a_size_below_eight() {
        for size in [0_u32, 1, 7] {
            let bytes = message(*b"MSGF", size, &[]);
            assert_eq!(decode(&bytes), Err(FrameError::Malformed), "size {size}");
        }
        let header = FrameHeader::parse(&mut BoundedReader::new(&message(*b"MSGF", 8, &[])));
        assert_eq!(
            header,
            Ok(FrameHeader {
                message_type: MessageType::Message,
                chunk: ChunkType::Final,
                size: 8,
            }),
            "8 is the smallest size"
        );
        assert_eq!(
            FrameHeader::parse(&mut BoundedReader::new(&message(*b"MSGF", 7, &[]))),
            Err(ParseError::InvalidDiscriminator {
                field: "message size",
                value: 7,
            })
        );
    }

    #[test]
    fn opcua_frame_a_size_above_int32_max_is_malformed_not_oversized() {
        // Upstream reads the size as a signed number, where these are negative.
        let mut big = OpcuaFramer::new(u32::MAX, limits(usize::MAX));
        for size in [0x8000_0000, u32::MAX] {
            let mut buffer = BytesMut::from(message(*b"MSGF", size, &[]).as_slice());
            assert_eq!(
                big.decode(&mut buffer),
                Err(FrameError::Malformed),
                "{size}"
            );
        }
        let mut buffer = BytesMut::from(message(*b"MSGF", 0x7FFF_FFFF, &[]).as_slice());
        assert_eq!(
            big.decode(&mut buffer),
            Ok(None),
            "Int32::MAX waits for its body"
        );
    }

    #[test]
    fn opcua_frame_size_above_the_receive_buffer_fails_before_buffering() {
        // Only the header is buffered: the declared body is never waited for.
        let mut buffer = BytesMut::from(message(*b"MSGF", 65_536, &[]).as_slice());
        assert_eq!(
            framer().decode(&mut buffer),
            Err(FrameError::Oversized(65_536))
        );
        assert_eq!(buffer.len(), 8, "the buffer is untouched");
        let error = PlcError::from(FrameError::Oversized(65_536));
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        // The limit itself is allowed and waits for the body.
        let mut buffer = BytesMut::from(message(*b"MSGF", 65_535, &[]).as_slice());
        assert_eq!(framer().decode(&mut buffer), Ok(None));
    }

    #[test]
    fn opcua_frame_the_smaller_of_the_receive_buffer_and_the_resource_limit_applies() {
        let mut small_buffer = OpcuaFramer::new(500, limits(65_536));
        assert_eq!(small_buffer.max_frame_bytes(), 500);
        let mut small_limit = OpcuaFramer::new(65_535, limits(1000));
        assert_eq!(small_limit.max_frame_bytes(), 1000);
        for (framer, edge) in [(&mut small_buffer, 500), (&mut small_limit, 1000)] {
            let mut at = BytesMut::from(message(*b"MSGF", edge, &[]).as_slice());
            assert_eq!(framer.decode(&mut at), Ok(None), "{edge} bytes are allowed");
            let past = edge + 1;
            let mut over = BytesMut::from(message(*b"MSGF", past, &[]).as_slice());
            let declared = usize::try_from(past).expect("fits");
            assert_eq!(
                framer.decode(&mut over),
                Err(FrameError::Oversized(declared)),
                "{past} bytes are not"
            );
        }
        let wide = OpcuaFramer::new(u32::MAX, limits(usize::MAX));
        assert_eq!(
            wide.max_frame_bytes, MAX_SIZE,
            "a limit over Int32::MAX is capped"
        );
    }

    #[test]
    fn opcua_message_type_hel_ack_err_opn_clo_msg_round_trip() {
        let codes: [(&[u8; 3], MessageType); 6] = [
            (b"HEL", MessageType::Hello),
            (b"ACK", MessageType::Acknowledge),
            (b"ERR", MessageType::Error),
            (b"OPN", MessageType::Open),
            (b"CLO", MessageType::Close),
            (b"MSG", MessageType::Message),
        ];
        for (code, message_type) in codes {
            assert_eq!(message_type.code(), *code, "{message_type:?}");
            assert_eq!(MessageType::from_code(*code), Ok(message_type));
        }
        for (frame, bytes) in all_frames() {
            assert_vector(&frame, &bytes);
        }
        let types: Vec<MessageType> = all_frames()
            .iter()
            .map(|(frame, _)| frame.body.message_type())
            .collect();
        assert_eq!(
            types,
            [
                MessageType::Hello,
                MessageType::Acknowledge,
                MessageType::Error,
                MessageType::Open,
                MessageType::Close,
                MessageType::Message,
            ]
        );
    }

    #[test]
    fn opcua_frame_the_vectors_are_the_bytes_of_part_6_written_out() {
        // Hello: `HEL`, `F`, size 48, version 0, receive 65535, send 65535, 4 MiB, 16 chunks
        // and the URL `opc.tcp://h:4840`.
        let hello: [u8; 48] = [
            0x48, 0x45, 0x4C, 0x46, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF,
            0x00, 0x00, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00, 0x40, 0x00, 0x10, 0x00, 0x00, 0x00,
            0x10, 0x00, 0x00, 0x00, 0x6F, 0x70, 0x63, 0x2E, 0x74, 0x63, 0x70, 0x3A, 0x2F, 0x2F,
            0x68, 0x3A, 0x34, 0x38, 0x34, 0x30,
        ];
        assert_vector(&hello_frame(), &hello);
        // Message: `MSG`, `F`, size 29, channel 7, token 9, sequence 3, request 4, payload.
        let message: [u8; 29] = [
            0x4D, 0x53, 0x47, 0x46, 0x1D, 0x00, 0x00, 0x00, 0x07, 0x00, 0x00, 0x00, 0x09, 0x00,
            0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04,
            0x05,
        ];
        assert_vector(&message_frame(), &message);
    }

    #[test]
    fn opcua_chunk_flags_c_f_a_round_trip() {
        for (flag, chunk) in [
            (b'C', ChunkType::Intermediate),
            (b'F', ChunkType::Final),
            (b'A', ChunkType::Abort),
        ] {
            let mut bytes = message_bytes();
            *bytes.get_mut(3).expect("the flag") = flag;
            let mut frame = message_frame();
            frame.chunk = chunk;
            assert_vector(&frame, &bytes);
            assert_eq!(chunk.code(), flag);
        }
        assert_eq!(
            ChunkType::from_code(b'X'),
            Err(ParseError::InvalidDiscriminator {
                field: "chunk type",
                value: u64::from(b'X'),
            })
        );
        let mut bytes = message_bytes();
        *bytes.get_mut(3).expect("the flag") = b'f';
        assert_eq!(
            decode(&bytes),
            Err(FrameError::Malformed),
            "the flag is case-sensitive"
        );
    }

    #[test]
    fn opcua_unknown_message_type_is_a_protocol_error() {
        // RHE is in upstream's Javadoc and not in its mspec; it is not implemented.
        for code in [*b"RHEF", *b"hel\x46", *b"HE\0F", *b"XYZF", *b"\0\0\0F"] {
            let bytes = message(code, 8, &[]);
            assert_eq!(decode(&bytes), Err(FrameError::Malformed), "{code:?}");
        }
        assert_eq!(
            MessageType::from_code(*b"RHE"),
            Err(ParseError::InvalidDiscriminator {
                field: "message type",
                value: u64::from(u32::from_le_bytes([b'R', b'H', b'E', 0])),
            })
        );
        let error = PlcError::from(FrameError::Malformed);
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        // An unknown type is judged at the header, before the size is compared.
        let bytes = message(*b"RHEF", 70_000, &[]);
        assert_eq!(decode(&bytes), Err(FrameError::Malformed));
    }

    #[test]
    fn opcua_hello_and_acknowledge_round_trip_with_all_limits() {
        assert_vector(&hello_frame(), &hello_bytes());
        assert_vector(&acknowledge_frame(), &acknowledge_bytes());
        // A server that sets no message or chunk limit sends 0, which round-trips.
        let frame = Frame::single(Body::Acknowledge(Acknowledge {
            protocol_version: 7,
            limits: TransportLimits {
                receive_buffer_size: 8192,
                send_buffer_size: 16_384,
                max_message_size: 0,
                max_chunk_count: 0,
            },
        }));
        let bytes = message(*b"ACKF", 28, &le(&[7, 8192, 16_384, 0, 0]));
        assert_vector(&frame, &bytes);
        // A null endpoint URL is kept null.
        let null_url = message(
            *b"HELF",
            32,
            &[le(&[0, 8192, 8192, 0, 0]), vec![0xFF; 4]].concat(),
        );
        assert_eq!(
            whole(&null_url).body,
            Body::Hello(Hello {
                protocol_version: 0,
                limits: TransportLimits {
                    receive_buffer_size: 8192,
                    send_buffer_size: 8192,
                    max_message_size: 0,
                    max_chunk_count: 0,
                },
                endpoint_url: UaString::null(),
            })
        );
        assert_eq!(encode(&whole(&null_url)), null_url);
    }

    #[test]
    fn opcua_err_message_carries_the_status_and_an_optional_reason() {
        assert_vector(&error_frame(), &error_bytes());
        let null_reason = message(*b"ERRF", 16, &[le(&[0x8001_0000]), vec![0xFF; 4]].concat());
        assert_eq!(
            whole(&null_reason).body,
            Body::Error(ErrorMessage {
                error: StatusCode(0x8001_0000),
                reason: UaString::null(),
            })
        );
        assert_eq!(encode(&whole(&null_reason)), null_reason);
    }

    #[test]
    fn opcua_asymmetric_header_is_59_bytes_for_policy_none() {
        let header = AsymmetricHeader::none();
        assert_eq!(header.length_in_bytes(), Ok(59));
        assert_eq!(NONE_POLICY_URI.len(), 47);
        let mut writer = BoundedWriter::new(usize::MAX);
        header.serialize(&mut writer).expect("serializes");
        assert_eq!(writer.as_slice().len(), 59);
        assert_eq!(
            writer.as_slice(),
            [
                le(&[47]),
                NONE_POLICY_URI.as_bytes().to_vec(),
                vec![0xFF; 8]
            ]
            .concat()
        );
        let mut reader = BoundedReader::new(writer.as_slice());
        assert_eq!(AsymmetricHeader::parse(&mut reader), Ok(header));
        reader.expect_eof().expect("all of it was read");
        // In an `OpenSecureChannel` message: 8 + 4 + 59 + 8 and the payload.
        assert_eq!(open_bytes().len(), 8 + 4 + 59 + 8 + 3);
    }

    #[test]
    fn opcua_asymmetric_header_length_follows_uri_certificate_and_thumbprint() {
        let uri = "http://opcfoundation.org/UA/SecurityPolicy#Basic256Sha256";
        let header = AsymmetricHeader {
            security_policy_uri: UaString::new(uri),
            sender_certificate: UaByteString::new(vec![0x30; 674]),
            receiver_thumbprint: UaByteString::new(vec![0x5A; 20]),
        };
        // 12 for the three prefixes, and the bytes of each part.
        assert_eq!(header.length_in_bytes(), Ok(12 + uri.len() + 674 + 20));
        let mut writer = BoundedWriter::new(usize::MAX);
        header.serialize(&mut writer).expect("serializes");
        assert_eq!(writer.as_slice().len(), 12 + uri.len() + 674 + 20);
        assert_eq!(
            AsymmetricHeader::parse(&mut BoundedReader::new(writer.as_slice())),
            Ok(header.clone())
        );
        // The whole OPN message is measured with it.
        let frame = Frame::single(Body::Open(OpenChunk {
            channel_id: 5,
            security: header,
            sequence: SequenceHeader {
                sequence_number: 2,
                request_id: 3,
            },
            body: Bytes::from_static(&[9, 9]),
        }));
        let bytes = encode(&frame);
        assert_eq!(bytes.len(), 8 + 4 + 12 + uri.len() + 674 + 20 + 8 + 2);
        assert_eq!(whole(&bytes), frame);
        assert_eq!(frame.length_in_bytes(), Ok(bytes.len()));
    }

    #[test]
    fn opcua_symmetric_header_round_trips() {
        assert_vector(&message_frame(), &message_bytes());
        assert_vector(&close_frame(), &close_bytes());
        assert_vector(&open_frame(), &open_bytes());
        // The channel id, the token id and the two numbers of the sequence header are four
        // different fields at four places.
        assert_eq!(
            whole(&message_bytes()).body,
            Body::Message(SecureChunk {
                channel_id: 7,
                token_id: 9,
                sequence: SequenceHeader {
                    sequence_number: 3,
                    request_id: 4,
                },
                body: Bytes::from_static(&[1, 2, 3, 4, 5]),
            })
        );
        assert_eq!(SecureChunk::HEADERS, 16);
        // The sequence number and the request id are unsigned.
        let high = Frame::single(Body::Message(secure_chunk(&[], 0xFFFF_FFFF, 0x8000_0000)));
        assert_vector(
            &high,
            &message(*b"MSGF", 24, &le(&[7, 9, 0xFFFF_FFFF, 0x8000_0000])),
        );
    }

    #[test]
    fn opcua_frame_a_body_the_type_does_not_allow_is_malformed() {
        // (type, body) pairs; every message is complete and its size is right.
        let url_runs_past = [le(&[0, 1, 1, 0, 0, 100]), b"short".to_vec()].concat();
        let negative_length = [le(&[0x8001_0000]), (-2_i32).to_le_bytes().to_vec()].concat();
        let uri_runs_past = [le(&[0, 99]), b"abc".to_vec()].concat();
        let cases: [([u8; 4], Vec<u8>, &str); 12] = [
            (*b"HELF", le(&[0, 1, 1, 0, 0]), "Hello without a URL"),
            (
                *b"HELF",
                url_runs_past,
                "a Hello URL that runs past the message",
            ),
            (
                *b"HELF",
                [hello_bytes().get(8..).expect("a body").to_vec(), vec![0]].concat(),
                "a byte left over in Hello",
            ),
            (
                *b"ACKF",
                le(&[0, 1, 1, 0]),
                "an Acknowledge of four numbers",
            ),
            (
                *b"ACKF",
                [le(&[0, 1, 1, 0, 0]), vec![0]].concat(),
                "a byte left over in Acknowledge",
            ),
            (*b"ERRF", le(&[0x8001_0000]), "an Error without a reason"),
            (*b"ERRF", negative_length, "a reason of length -2"),
            (
                *b"ERRF",
                [error_bytes().get(8..).expect("a body").to_vec(), vec![0]].concat(),
                "a byte left over in Error",
            ),
            (
                *b"OPNF",
                uri_runs_past,
                "a policy URI that runs past the message",
            ),
            (
                *b"OPNF",
                [le(&[0, 0xFFFF_FFFF]), vec![0xFF; 8], le(&[1])].concat(),
                "an Open message with half a sequence header",
            ),
            (
                *b"MSGF",
                le(&[7, 9, 3]),
                "a Message with half a sequence header",
            ),
            (*b"CLOF", vec![], "a Close with nothing"),
        ];
        for (code, body, what) in cases {
            let size = u32::try_from(body.len() + 8).expect("small");
            let bytes = message(code, size, &body);
            assert_eq!(decode(&bytes), Err(FrameError::Malformed), "{what}");
            let mut buffer = BytesMut::from(bytes.as_slice());
            assert!(framer().decode(&mut buffer).is_err(), "{what}");
            assert_eq!(buffer.len(), bytes.len(), "{what}: the buffer is untouched");
        }
    }

    #[test]
    fn opcua_frame_a_declared_string_never_reserves_memory() {
        // A Hello that declares a 2 GiB URL in a 32-byte message.
        let bytes = message(
            *b"HELF",
            32,
            &[le(&[0, 8192, 8192, 0, 0]), i32::MAX.to_le_bytes().to_vec()].concat(),
        );
        assert_eq!(
            decode(&bytes),
            Err(FrameError::Malformed),
            "over the String maximum"
        );
        // An OPN that declares a certificate of 16 MiB + 1 in a 20-byte message.
        let over: i32 = 16 * 1024 * 1024 + 1;
        let rest = [le(&[0, 0xFFFF_FFFF]), over.to_le_bytes().to_vec()].concat();
        let bytes = message(*b"OPNF", 20, &rest);
        assert_eq!(decode(&bytes), Err(FrameError::Malformed));
        // A URL within the maximum that the message does not hold is cut off.
        let rest = [
            le(&[0, 8192, 8192, 0, 0]),
            1_000_000_i32.to_le_bytes().to_vec(),
        ]
        .concat();
        let bytes = message(*b"HELF", 32, &rest);
        assert_eq!(decode(&bytes), Err(FrameError::Malformed));
    }

    #[test]
    fn opcua_frame_a_body_over_the_string_maximum_is_oversized_on_encode() {
        let mut wide = OpcuaFramer::new(u32::MAX, limits(usize::MAX));
        let url = vec![b'a'; 16 * 1024 * 1024 + 1];
        let frame = Frame::single(Body::Hello(Hello {
            protocol_version: 0,
            limits: TransportLimits {
                receive_buffer_size: 0,
                send_buffer_size: 0,
                max_message_size: 0,
                max_chunk_count: 0,
            },
            endpoint_url: UaString::from_bytes(url),
        }));
        let mut out = BytesMut::new();
        assert_eq!(
            wide.encode(&frame, &mut out),
            Err(FrameError::Oversized(16 * 1024 * 1024 + 1))
        );
        assert!(out.is_empty(), "nothing was appended");
    }

    #[test]
    fn opcua_frame_encode_refuses_a_message_over_the_largest_and_appends_nothing() {
        let frame = message_frame();
        assert_eq!(frame.length_in_bytes(), Ok(29));
        let mut at = OpcuaFramer::new(29, limits(65_536));
        let mut out = BytesMut::from(&b"keep"[..]);
        at.encode(&frame, &mut out).expect("29 bytes fit 29");
        assert_eq!(out.len(), 4 + 29, "encode appends");
        assert_eq!(out.get(..4), Some(&b"keep"[..]));
        assert_eq!(out.get(4..), Some(message_bytes().as_slice()));
        let mut under = OpcuaFramer::new(28, limits(65_536));
        let mut out = BytesMut::from(&b"keep"[..]);
        assert_eq!(
            under.encode(&frame, &mut out),
            Err(FrameError::Oversized(29))
        );
        assert_eq!(&out[..], b"keep", "nothing was appended");
        let mut limited = OpcuaFramer::new(65_535, limits(28));
        assert_eq!(
            limited.encode(&frame, &mut out),
            Err(FrameError::Oversized(29))
        );
    }

    #[test]
    fn opcua_frame_the_codec_errors_map_to_the_framing_errors() {
        let oversized = ParseError::OversizedLength {
            length: 300,
            max: 8,
        };
        assert_eq!(frame_error(oversized), FrameError::Oversized(300));
        let others = [
            ParseError::Truncated {
                needed: 2,
                available: 1,
            },
            ParseError::InvalidDiscriminator {
                field: "x",
                value: 1,
            },
            ParseError::TrailingBytes { remaining: 1 },
            ParseError::Overflow,
            ParseError::CountLimit {
                field: "x",
                count: 2,
                max: 1,
            },
            ParseError::RecursionLimit { limit: 64 },
        ];
        for error in others {
            let shown = format!("{error:?}");
            assert_eq!(frame_error(error), FrameError::Malformed, "{shown}");
        }
    }

    #[test]
    fn opcua_frame_the_constants_agree() {
        assert_eq!(usize::try_from(HEADER_SIZE), Ok(HEADER_BYTES));
        assert_eq!(MAX_SIZE, u32::try_from(i32::MAX).expect("positive"));
        assert_eq!(TransportLimits::SIZE, 16);
        assert_eq!(Acknowledge::SIZE, 20);
        assert_eq!(SequenceHeader::SIZE, 8);
        assert_eq!(
            NONE_POLICY_URI,
            "http://opcfoundation.org/UA/SecurityPolicy#None"
        );
    }

    #[test]
    fn opcua_frame_debug_shows_the_payload_by_length_only() {
        let shown = format!("{:?}", open_frame());
        assert!(shown.contains("body_len: 3"), "{shown}");
        assert!(
            !shown.contains("170"),
            "no payload byte (0xAA) is shown: {shown}"
        );
        let shown = format!("{:?}", message_frame());
        assert!(shown.contains("body_len: 5"), "{shown}");
        assert!(shown.contains("token_id: 9"), "{shown}");
    }

    #[test]
    fn opcua_frame_several_messages_in_one_buffer_come_out_in_order() {
        let mut buffer = BytesMut::new();
        for (_, bytes) in all_frames() {
            buffer.extend_from_slice(&bytes);
        }
        let mut framer = framer();
        for (frame, _) in all_frames() {
            assert_eq!(framer.decode(&mut buffer), Ok(Some(frame)));
        }
        assert!(buffer.is_empty(), "every byte was used");
        assert_eq!(framer.decode(&mut buffer), Ok(None));
    }
}
