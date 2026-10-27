//! The MBAP header, the transaction counter, the ADU and the framer that finds ADUs.

use std::fmt;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use stonecrab_codec::{BoundedReader, ParseError};
use stonecrab_core::ByteOrder;
use stonecrab_transport::{FrameError, Framer};

use crate::{MAX_PDU_BYTES, UnitId};

/// Bytes of the part of the header the length field does not count: the transaction id,
/// the protocol id and the length itself.
const LENGTH_PREFIX_BYTES: usize = 6;

/// Largest transaction id the counter produces; 0xFFFF is never used.
const LAST_TRANSACTION_ID: u16 = 0xFFFE;

/// Converts the bounded reader's error into the transport's framing error (decision D22).
const fn frame_error(error: &ParseError) -> FrameError {
    match error {
        ParseError::Truncated { .. } => FrameError::Truncated,
        ParseError::OversizedLength { length, .. } => FrameError::Oversized(*length),
        ParseError::InvalidDiscriminator { .. }
        | ParseError::TrailingBytes { .. }
        | ParseError::Overflow
        | _ => FrameError::Malformed,
    }
}

/// The Modbus Application Protocol (MBAP) header: 7 bytes in front of every PDU on TCP.
///
/// On the wire the fields are big-endian in this order: `transaction_id` (2 bytes),
/// `protocol_id` (2 bytes, always 0 for Modbus), `length` (2 bytes: the number of bytes
/// that follow it, the unit id plus the PDU) and `unit_id` (1 byte). A header is valid
/// when the protocol id is 0 and the length is 2 to 254, which allows a PDU of 1 to 253
/// bytes ([`MAX_PDU_BYTES`]); [`MbapHeader::decode`] and [`MbapHeader::encode`] enforce
/// both, the public fields do not. The transaction id is any value: the peer's reply
/// carries whatever it chose, and matching it is the session's job.
///
/// # Examples
/// ```
/// use stonecrab_modbus::{MbapHeader, UnitId};
///
/// // Modbus Application Protocol V1.1b section 6.1 request, transaction 1, unit 17.
/// let header = MbapHeader::decode(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x11])?;
/// assert_eq!(header.transaction_id, 1);
/// assert_eq!(header.length, 6);
/// assert_eq!(header.unit_id, UnitId::from(17));
/// assert_eq!(header.adu_len(), 12);
/// # Ok::<(), stonecrab_transport::FrameError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-FRAMING
/// Upstream sizes a frame as 6 plus the big-endian length field and then parses the ADU
/// with the generated MBAP type (`ModbusTcpADU`); the field order is the same. Stonecrab
/// additionally bounds the length (docs/parity/modbus.md#MB-LIMIT-ADU).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MbapHeader {
    /// Echoed by the server so a reply can be matched to its request.
    pub transaction_id: u16,
    /// Always 0 for Modbus; anything else is not a Modbus frame.
    pub protocol_id: u16,
    /// Bytes after this field: the unit id plus the PDU, so 2 to 254.
    pub length: u16,
    /// The unit identifier of the addressed device.
    pub unit_id: UnitId,
}

impl MbapHeader {
    /// Bytes of an encoded header.
    pub const ENCODED_LEN: usize = 7;
    /// Smallest legal `length`: the unit id plus a 1 byte PDU.
    pub const MIN_LENGTH: u16 = 2;
    /// Largest legal `length`: the unit id plus a 253 byte PDU.
    pub const MAX_LENGTH: u16 = 254;

    /// Read the header at the front of `bytes`; bytes after the first 7 are ignored.
    ///
    /// The protocol id is judged before the length, because a header that is not Modbus
    /// has no meaningful length.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-LIMIT-ADU
    /// Upstream accepts any length when sizing a frame; the 2 to 254 bound is Stonecrab's
    /// (V1.1b section 4.1).
    ///
    /// # Errors
    /// - [`FrameError::Truncated`] if `bytes` is shorter than 7 bytes.
    /// - [`FrameError::Malformed`] if the protocol id is not 0, or the length is 0 or 1.
    /// - [`FrameError::Oversized`] if the length is above 254; it carries the size of the
    ///   whole ADU the header declares.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::MbapHeader;
    /// use stonecrab_transport::FrameError;
    ///
    /// assert!(MbapHeader::decode(&[0, 1, 0, 0, 0, 2, 1]).is_ok());
    /// assert_eq!(MbapHeader::decode(&[0, 1, 0, 0, 0, 6]), Err(FrameError::Truncated));
    /// assert_eq!(MbapHeader::decode(&[0, 1, 0, 1, 0, 6, 1]), Err(FrameError::Malformed));
    /// assert_eq!(MbapHeader::decode(&[0, 1, 0, 0, 0, 255, 1]), Err(FrameError::Oversized(261)));
    /// ```
    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        let mut reader = BoundedReader::new(bytes);
        let transaction_id = reader
            .read_u16(ByteOrder::BigEndian)
            .map_err(|error| frame_error(&error))?;
        let protocol_id = reader
            .read_u16(ByteOrder::BigEndian)
            .map_err(|error| frame_error(&error))?;
        let length = reader
            .read_u16(ByteOrder::BigEndian)
            .map_err(|error| frame_error(&error))?;
        let unit_id = UnitId::from(reader.read_u8().map_err(|error| frame_error(&error))?);
        Self {
            transaction_id,
            protocol_id,
            length,
            unit_id,
        }
        .validated()
    }

    /// Append the 7 header bytes to `out`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-READ-FRAMING
    /// Same field order as upstream's generated MBAP type.
    ///
    /// # Errors
    /// The header is refused, and nothing is appended, in the cases [`decode`](Self::decode)
    /// refuses: [`FrameError::Malformed`] for a protocol id other than 0 or a length of 0
    /// or 1, [`FrameError::Oversized`] for a length above 254.
    ///
    /// # Examples
    /// ```
    /// use bytes::BytesMut;
    /// use stonecrab_modbus::{MbapHeader, UnitId};
    ///
    /// let header = MbapHeader {
    ///     transaction_id: 1,
    ///     protocol_id: 0,
    ///     length: 6,
    ///     unit_id: UnitId::from(17),
    /// };
    /// let mut out = BytesMut::new();
    /// header.encode(&mut out)?;
    /// assert_eq!(&out[..], &[0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x11]);
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    pub fn encode(self, out: &mut BytesMut) -> Result<(), FrameError> {
        let valid = self.validated()?;
        out.put_u16(valid.transaction_id);
        out.put_u16(valid.protocol_id);
        out.put_u16(valid.length);
        out.put_u8(valid.unit_id.get());
        Ok(())
    }

    /// Bytes of the whole ADU this header announces: the 6 bytes before the unit id plus
    /// the `length` field.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::{MbapHeader, UnitId};
    ///
    /// let header = MbapHeader {
    ///     transaction_id: 1,
    ///     protocol_id: 0,
    ///     length: 254,
    ///     unit_id: UnitId::default(),
    /// };
    /// assert_eq!(header.adu_len(), 260);
    /// ```
    #[must_use]
    pub fn adu_len(self) -> usize {
        usize::from(self.length).saturating_add(LENGTH_PREFIX_BYTES)
    }

    /// The header itself when its protocol id and length are legal, the error otherwise.
    fn validated(self) -> Result<Self, FrameError> {
        if self.protocol_id != 0 || self.length < Self::MIN_LENGTH {
            return Err(FrameError::Malformed);
        }
        if self.length > Self::MAX_LENGTH {
            return Err(FrameError::Oversized(self.adu_len()));
        }
        Ok(self)
    }
}

/// The transaction identifier counter: ids 1 to 0xFFFE, wrapping, never 0 or 0xFFFF.
///
/// The client picks a transaction id for each request and the server echoes it, which is
/// how a reply is matched to the request that caused it. The counter starts at 1
/// ([`Default`] and [`TransactionId::FIRST`]); [`next`](Self::next) hands out the current
/// id and moves on, and after 0xFFFE the next id is 1 again. Ids continue across a
/// reconnect because the counter belongs to the driver, not to the connection.
///
/// # Examples
/// ```
/// use stonecrab_modbus::TransactionId;
///
/// let mut ids = TransactionId::default();
/// assert_eq!(ids.next().get(), 1);
/// assert_eq!(ids.next().get(), 2);
///
/// let mut last = TransactionId::LAST;
/// assert_eq!(last.next().get(), 0xFFFE);
/// assert_eq!(last.next().get(), 1, "0xFFFF and 0 are skipped");
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-TXN
/// Upstream keeps an `AtomicInteger` starting at 1 and resets it to 1 when
/// `getAndIncrement` reaches 0xFFFF, so its ids cycle through 1 to 0xFFFE exactly as
/// these do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TransactionId(u16);

impl TransactionId {
    /// The first id, 1.
    pub const FIRST: Self = Self(1);
    /// The last id before the counter wraps, 0xFFFE.
    pub const LAST: Self = Self(LAST_TRANSACTION_ID);

    /// The id as the 16 bit number that goes on the wire.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::TransactionId;
    /// assert_eq!(TransactionId::FIRST.get(), 1);
    /// ```
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Return the current id and advance the counter, wrapping from 0xFFFE to 1.
    ///
    /// The result is never 0 and never 0xFFFF.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::TransactionId;
    ///
    /// let mut ids = TransactionId::FIRST;
    /// assert_eq!([ids.next().get(), ids.next().get()], [1, 2]);
    /// ```
    #[must_use]
    pub const fn next(&mut self) -> Self {
        let current = *self;
        let following = self.0.saturating_add(1);
        self.0 = if following > LAST_TRANSACTION_ID {
            Self::FIRST.0
        } else {
            following
        };
        current
    }
}

impl Default for TransactionId {
    /// The first id, 1.
    fn default() -> Self {
        Self::FIRST
    }
}

impl From<TransactionId> for u16 {
    fn from(value: TransactionId) -> Self {
        value.0
    }
}

impl fmt::Display for TransactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// One Modbus TCP application data unit: an MBAP header and the PDU it carries.
///
/// The PDU is held as opaque bytes. Decoding it (`decode_response`) happens after the
/// session has matched the ADU to its request, so the framer never needs to know the
/// function codes. The constructor keeps the two halves consistent: the protocol id is 0,
/// the PDU is 1 to 253 bytes and `length` is the PDU length plus one, so an ADU is always
/// 8 to 260 bytes on the wire and always encodes. `Debug` shows the header and the PDU
/// length, not the PDU bytes.
///
/// # Examples
/// ```
/// use bytes::Bytes;
/// use stonecrab_modbus::{ModbusAdu, UnitId};
///
/// // Modbus Application Protocol V1.1b section 6.3 request: read registers 108 to 110.
/// let pdu = Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]);
/// let adu = ModbusAdu::new(3, UnitId::from(17), pdu)?;
/// assert_eq!(adu.header().length, 6);
/// assert_eq!(adu.transaction_id(), 3);
/// assert_eq!(adu.wire_len(), 12);
/// assert_eq!(adu.pdu().len(), 5);
/// # Ok::<(), stonecrab_transport::FrameError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-FRAMING
/// The layout is upstream's `ModbusTcpADU`; the PDU is parsed later, after correlation
/// (docs/parity/modbus.md#MB-READ-MATCH).
#[derive(Clone, PartialEq, Eq)]
pub struct ModbusAdu {
    header: MbapHeader,
    pdu: Bytes,
}

impl ModbusAdu {
    /// Largest ADU in bytes: the 7 byte header and a 253 byte PDU.
    pub const MAX_BYTES: usize = MbapHeader::ENCODED_LEN + MAX_PDU_BYTES;

    /// Build an ADU with protocol id 0 and the length that matches `pdu`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-LIMIT-ADU
    /// The PDU bound is V1.1b section 4.1; upstream has no explicit check.
    ///
    /// # Errors
    /// - [`FrameError::Malformed`] if `pdu` is empty.
    /// - [`FrameError::Oversized`] if `pdu` is longer than 253 bytes; it carries the size
    ///   the whole ADU would have.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    /// use stonecrab_transport::FrameError;
    ///
    /// let unit = UnitId::default();
    /// assert!(ModbusAdu::new(1, unit, Bytes::from_static(&[0x03])).is_ok());
    /// assert_eq!(ModbusAdu::new(1, unit, Bytes::new()), Err(FrameError::Malformed));
    /// assert_eq!(
    ///     ModbusAdu::new(1, unit, Bytes::from(vec![0; 254])),
    ///     Err(FrameError::Oversized(261))
    /// );
    /// ```
    pub fn new(transaction_id: u16, unit_id: UnitId, pdu: Bytes) -> Result<Self, FrameError> {
        if pdu.is_empty() {
            return Err(FrameError::Malformed);
        }
        if pdu.len() > MAX_PDU_BYTES {
            return Err(FrameError::Oversized(
                pdu.len().saturating_add(MbapHeader::ENCODED_LEN),
            ));
        }
        let length = u16::try_from(pdu.len().saturating_add(1)).unwrap_or(u16::MAX);
        let header = MbapHeader {
            transaction_id,
            protocol_id: 0,
            length,
            unit_id,
        };
        Ok(Self { header, pdu })
    }

    /// The MBAP header.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(9, UnitId::from(2), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.header().unit_id, UnitId::from(2));
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn header(&self) -> MbapHeader {
        self.header
    }

    /// The transaction id of the header.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(9, UnitId::default(), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.transaction_id(), 9);
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn transaction_id(&self) -> u16 {
        self.header.transaction_id
    }

    /// The unit identifier of the header.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(1, UnitId::from(7), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.unit_id(), UnitId::from(7));
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn unit_id(&self) -> UnitId {
        self.header.unit_id
    }

    /// The PDU bytes, 1 to 253 of them, exactly as they are on the wire.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(1, UnitId::default(), Bytes::from_static(&[0x81, 0x02]))?;
    /// assert_eq!(&adu.pdu()[..], &[0x81, 0x02]);
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn pdu(&self) -> &Bytes {
        &self.pdu
    }

    /// Take the PDU bytes out of the ADU.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(1, UnitId::default(), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.into_pdu(), Bytes::from_static(&[0x03]));
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub fn into_pdu(self) -> Bytes {
        self.pdu
    }

    /// Bytes this ADU occupies on the wire, header included (8 to 260).
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab_modbus::{ModbusAdu, UnitId};
    ///
    /// let adu = ModbusAdu::new(1, UnitId::default(), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.wire_len(), 8);
    /// # Ok::<(), stonecrab_transport::FrameError>(())
    /// ```
    #[must_use]
    pub fn wire_len(&self) -> usize {
        self.header.adu_len()
    }
}

/// Shows the header and the PDU length, never the PDU bytes.
impl fmt::Debug for ModbusAdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusAdu")
            .field("header", &self.header)
            .field("pdu_len", &self.pdu.len())
            .finish()
    }
}

/// The [`Framer`] for Modbus TCP: finds ADUs in a byte stream and writes them back.
///
/// It is a stateless value; [`FramedTransport`](stonecrab_transport::FramedTransport)
/// drives it. The rules, in the order `decode` applies them:
/// 1. Fewer than 7 bytes buffered: `Ok(None)`, wait for the header.
/// 2. The header is judged as soon as it is complete, before anything is allocated or
///    awaited: a protocol id other than 0 or a length of 0 or 1 is
///    [`FrameError::Malformed`], a length above 254 (an ADU over 260 bytes) is
///    [`FrameError::Oversized`] carrying the declared ADU size.
/// 3. The body is incomplete: `Ok(None)`; the buffer is left as it was.
/// 4. Otherwise exactly the ADU's bytes are removed from the front of the buffer and the
///    rest stays for the next call.
///
/// `max_frame_bytes` is 260. The PDU inside is not inspected.
///
/// # Examples
/// ```
/// use bytes::BytesMut;
/// use stonecrab_modbus::MbapFramer;
/// use stonecrab_transport::Framer;
///
/// let mut framer = MbapFramer::new();
/// // V1.1b section 6.1 request, then the first byte of the next ADU.
/// let mut buffer = BytesMut::from(
///     &[0x00, 0x01, 0x00, 0x00, 0x00, 0x06, 0x11, 0x01, 0x00, 0x13, 0x00, 0x13, 0x00][..],
/// );
/// let adu = framer.decode(&mut buffer)?.expect("one whole ADU");
/// assert_eq!(adu.transaction_id(), 1);
/// assert_eq!(&adu.pdu()[..], &[0x01, 0x00, 0x13, 0x00, 0x13]);
/// assert_eq!(&buffer[..], &[0x00], "the extra byte stays");
/// assert_eq!(framer.decode(&mut buffer)?, None, "and waits for a header");
///
/// let mut wire = BytesMut::new();
/// framer.encode(&adu, &mut wire)?;
/// assert_eq!(wire.len(), 12);
/// # Ok::<(), stonecrab_transport::FrameError>(())
/// ```
///
/// # Compatibility: docs/parity/modbus.md#MB-READ-FRAMING
/// Upstream's `ModbusTcpMessageCodec` frames the same way (6 plus the length field) and
/// has no upper bound; the bounds here are docs/parity/modbus.md#MB-LIMIT-ADU.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MbapFramer;

impl MbapFramer {
    /// A framer; it holds no state.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::MbapFramer;
    /// use stonecrab_transport::Framer;
    ///
    /// assert_eq!(MbapFramer::new().max_frame_bytes(), 260);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Framer<ModbusAdu> for MbapFramer {
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<ModbusAdu>, FrameError> {
        if buffer.len() < MbapHeader::ENCODED_LEN {
            return Ok(None);
        }
        let header = MbapHeader::decode(buffer)?;
        let total = header.adu_len();
        if buffer.len() < total {
            return Ok(None);
        }
        let body = buffer
            .get(MbapHeader::ENCODED_LEN..total)
            .ok_or(FrameError::Truncated)?;
        let pdu = Bytes::copy_from_slice(body);
        buffer.advance(total);
        Ok(Some(ModbusAdu { header, pdu }))
    }

    fn encode(&mut self, frame: &ModbusAdu, out: &mut BytesMut) -> Result<(), FrameError> {
        frame.header.encode(out)?;
        out.put_slice(&frame.pdu);
        Ok(())
    }

    fn max_frame_bytes(&self) -> usize {
        ModbusAdu::MAX_BYTES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_errors_convert_to_the_matching_frame_error() {
        let truncated = ParseError::Truncated {
            needed: 2,
            available: 1,
        };
        assert_eq!(frame_error(&truncated), FrameError::Truncated);
        let oversized = ParseError::OversizedLength {
            length: 300,
            max: 253,
        };
        assert_eq!(frame_error(&oversized), FrameError::Oversized(300));
        let invalid = ParseError::InvalidDiscriminator {
            field: "protocol id",
            value: 1,
        };
        assert_eq!(frame_error(&invalid), FrameError::Malformed);
        assert_eq!(frame_error(&ParseError::Overflow), FrameError::Malformed);
    }

    #[test]
    fn the_adu_limit_is_the_header_plus_the_largest_pdu() {
        assert_eq!(ModbusAdu::MAX_BYTES, 260);
        assert_eq!(
            usize::from(MbapHeader::MAX_LENGTH).saturating_add(LENGTH_PREFIX_BYTES),
            ModbusAdu::MAX_BYTES
        );
    }
}
