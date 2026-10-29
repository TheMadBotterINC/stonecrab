//! Modbus RTU framing: the CRC-16, the ADU (address, PDU, CRC) and the framer that finds
//! ADUs in the bytes of a serial line.

use std::fmt;

use crate::transport::{FrameError, Framer};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::modbus::{MAX_PDU_BYTES, UnitId};

/// The bytes around the PDU: the address in front, the two CRC bytes behind.
const OVERHEAD: usize = 3;

/// The CRC-16/MODBUS register before the first byte (Modbus over Serial Line V1.02,
/// section 6.2.2).
const CRC_INIT: u16 = 0xFFFF;

/// The polynomial 0x8005, reflected, as the serial line specification shifts right.
const CRC_POLY: u16 = 0xA001;

/// The CRC-16 of Modbus RTU over `bytes`: polynomial 0xA001 (0x8005 reflected), initial
/// value 0xFFFF, no final XOR. On the wire the low byte goes first.
fn crc16<'a>(bytes: impl IntoIterator<Item = &'a u8>) -> u16 {
    bytes.into_iter().fold(CRC_INIT, |crc, byte| {
        (0..8).fold(crc ^ u16::from(*byte), |crc, _bit| {
            let shifted = crc.wrapping_shr(1);
            if crc & 1 == 1 {
                shifted ^ CRC_POLY
            } else {
                shifted
            }
        })
    })
}

/// A Modbus RTU application data unit: the one-byte address (the unit) and the PDU. The CRC
/// is not stored; [`RtuFramer`] checks it on receive and computes it on send.
///
/// On the wire the ADU is the address, the PDU and the CRC-16, low byte first, at most
/// 256 bytes ([`MAX_BYTES`](Self::MAX_BYTES)). There is no transaction id: a reply is
/// matched to its request by address and function code. The PDU stays opaque bytes here;
/// `decode_response` reads it, exactly as for Modbus TCP.
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-RTU-WIRE-MODEL
/// Upstream's `ModbusRtuADU` is the same three fields (`modbus.mspec`); the CRC is a
/// checked field there and a property of the frame here.
///
/// # Examples
/// ```
/// use bytes::Bytes;
/// use stonecrab::modbus::{RtuAdu, UnitId};
///
/// // Modbus Application Protocol V1.1b section 6.3: read registers 108 to 110 of unit 17.
/// let adu = RtuAdu::new(UnitId::from(17), Bytes::from_static(&[0x03, 0x00, 0x6B, 0x00, 0x03]))?;
/// assert_eq!(adu.unit_id(), UnitId::from(17));
/// assert_eq!(adu.wire_len(), 8);
/// # Ok::<(), stonecrab::transport::FrameError>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct RtuAdu {
    unit_id: UnitId,
    pdu: Bytes,
}

impl RtuAdu {
    /// The largest RTU ADU in bytes: the address, a 253-byte PDU and the CRC (Modbus over
    /// Serial Line V1.02, section 2.5.1).
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-LIMIT-ADU
    /// Upstream sets no bound on a received frame; the bound is the serial line
    /// specification's 256 bytes.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::RtuAdu;
    ///
    /// assert_eq!(RtuAdu::MAX_BYTES, 256);
    /// ```
    pub const MAX_BYTES: usize = 1 + MAX_PDU_BYTES + 2;

    /// An ADU for `unit_id` carrying `pdu`.
    ///
    /// # Errors
    /// - [`FrameError::Malformed`] if `pdu` is empty.
    /// - [`FrameError::Oversized`] if `pdu` is longer than 253 bytes; it carries the size
    ///   the ADU would have on the wire.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{RtuAdu, UnitId};
    /// use stonecrab::transport::FrameError;
    ///
    /// let unit = UnitId::default();
    /// assert!(RtuAdu::new(unit, Bytes::from_static(&[0x03])).is_ok());
    /// assert_eq!(RtuAdu::new(unit, Bytes::new()), Err(FrameError::Malformed));
    /// assert_eq!(
    ///     RtuAdu::new(unit, Bytes::from(vec![0; 254])),
    ///     Err(FrameError::Oversized(257))
    /// );
    /// ```
    pub fn new(unit_id: UnitId, pdu: Bytes) -> Result<Self, FrameError> {
        if pdu.is_empty() {
            return Err(FrameError::Malformed);
        }
        if pdu.len() > MAX_PDU_BYTES {
            return Err(FrameError::Oversized(pdu.len().saturating_add(OVERHEAD)));
        }
        Ok(Self { unit_id, pdu })
    }

    /// The address byte: the unit the request goes to, or the unit that answers.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{RtuAdu, UnitId};
    ///
    /// let adu = RtuAdu::new(UnitId::from(7), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.unit_id(), UnitId::from(7));
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn unit_id(&self) -> UnitId {
        self.unit_id
    }

    /// The PDU bytes, 1 to 253 of them, exactly as they are on the wire.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{RtuAdu, UnitId};
    ///
    /// let adu = RtuAdu::new(UnitId::default(), Bytes::from_static(&[0x83, 0x02]))?;
    /// assert_eq!(&adu.pdu()[..], &[0x83, 0x02]);
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn pdu(&self) -> &Bytes {
        &self.pdu
    }

    /// The size of the ADU on the wire: the address, the PDU and the two CRC bytes.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{RtuAdu, UnitId};
    ///
    /// let adu = RtuAdu::new(UnitId::default(), Bytes::from_static(&[0x83, 0x02]))?;
    /// assert_eq!(adu.wire_len(), 5);
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn wire_len(&self) -> usize {
        self.pdu.len().saturating_add(OVERHEAD)
    }
}

/// Shows the address and the PDU length, never the PDU bytes.
impl fmt::Debug for RtuAdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RtuAdu")
            .field("unit_id", &self.unit_id)
            .field("pdu_len", &self.pdu.len())
            .finish()
    }
}

/// What the bytes at one position of the buffer are.
enum Candidate {
    /// A whole frame of this many bytes whose CRC is right.
    Frame(usize),
    /// Possibly the start of a frame; more bytes are needed to tell.
    Incomplete,
    /// Not a frame: an unknown function code, or a whole frame whose CRC is wrong.
    Bad,
}

/// What the header of a response says about its size.
#[derive(Debug, PartialEq, Eq)]
enum Size {
    /// The function code has no size: these bytes are not a response.
    Unknown,
    /// The function code carries a byte count, which has not arrived yet.
    Pending,
    /// The whole frame is this many bytes.
    Bytes(usize),
}

/// The size of a response with this function code and, for the codes that carry it, this
/// byte count, as upstream's `ModbusRtuMessageCodec` computes it.
fn response_size(function: u8, count: Option<&u8>) -> Size {
    match function {
        // An exception: address, function, code, CRC.
        0x80..=0xFF => Size::Bytes(5),
        // Reads, file records and read/write registers carry a byte count.
        0x01..=0x04 | 0x14 | 0x15 | 0x17 => count.map_or(Size::Pending, |count| {
            Size::Bytes(usize::from(*count).saturating_add(5))
        }),
        // The write echoes: address, function, two words, CRC.
        0x05 | 0x06 | 0x0F | 0x10 => Size::Bytes(8),
        _ => Size::Unknown,
    }
}

/// Judge the bytes at the front of `bytes`.
fn candidate(bytes: &[u8]) -> Result<Candidate, FrameError> {
    let Some(function) = bytes.get(1) else {
        return Ok(Candidate::Incomplete);
    };
    let size = match response_size(*function, bytes.get(2)) {
        Size::Unknown => return Ok(Candidate::Bad),
        Size::Pending => return Ok(Candidate::Incomplete),
        Size::Bytes(size) => size,
    };
    if size > RtuAdu::MAX_BYTES {
        return Err(FrameError::Oversized(size));
    }
    let (Some(body), Some(&[low, high])) = (
        bytes.get(..size.saturating_sub(2)),
        bytes.get(size.saturating_sub(2)..size),
    ) else {
        return Ok(Candidate::Incomplete);
    };
    Ok(if crc16(body) == u16::from_le_bytes([low, high]) {
        Candidate::Frame(size)
    } else {
        Candidate::Bad
    })
}

/// The first position after the front of `bytes` that holds a whole frame with the right
/// CRC, as the number of bytes before it and the frame's size.
fn later_frame(bytes: &[u8]) -> Option<(usize, usize)> {
    (1..bytes.len()).find_map(|skip| match bytes.get(skip..).map(candidate) {
        Some(Ok(Candidate::Frame(size))) => Some((skip, size)),
        _ => None,
    })
}

/// The [`Framer`] for Modbus RTU: finds ADUs in the bytes of a serial line by the size
/// their function code gives, checks the CRC, and writes ADUs with their CRC.
///
/// RTU frames have no header to synchronize on; on the line, silence separates them.
/// The framer sizes a frame from its function code, as upstream does: an exception is 5
/// bytes, a read (function codes 01 to 04) and the file-record and read/write-registers
/// replies (0x14, 0x15, 0x17) are 5 plus their byte count, the write echoes (05, 06, 0x0F,
/// 0x10) are 8 bytes, and every other function code is no frame. `decode` applies, in this
/// order:
/// 1. The size the header at the front gives is above 256 bytes: [`FrameError::Oversized`]
///    with that size, judged from the header alone.
/// 2. The front is a whole frame with the right CRC: it is removed and returned.
/// 3. The front is anything else: the framer looks for the first later position that holds
///    a whole frame with the right CRC and, if there is one, removes the bytes before it
///    together with the frame and returns the frame, so noise in front of a reply does not
///    lose the reply. A stray byte in front of a reply reads as a header whose size the
///    buffer does not hold yet, so the search runs for an incomplete front too; a false
///    match inside a frame still arriving needs a correct CRC-16.
/// 4. No later position holds a frame: an incomplete front (fewer than 2 bytes, a byte
///    count still missing, or fewer bytes than the size) is `Ok(None)`, wait; a front that
///    is no frame (an unknown function code, or a whole frame with a wrong CRC) is
///    [`FrameError::Malformed`]. Nothing is removed.
///
/// Silence is not used to find a frame (upstream does not either), so the framer cannot
/// tell where debris ends: [`resync_discards`](Framer::resync_discards) is true, and a
/// resynchronization drops every buffered byte instead of searching them. `max_frame_bytes`
/// is 256.
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-RTU-SIZE-TABLE
/// The size table is upstream's (`ModbusRtuMessageCodec.calculateTotalMessageSize`); the CRC
/// is docs/parity/modbus-serial.md#MBS-RTU-CRC and the search past noise is
/// docs/parity/modbus-serial.md#MBS-RTU-RESYNC.
///
/// # Examples
/// ```
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::modbus::{RtuAdu, RtuFramer, UnitId};
/// use stonecrab::transport::{FrameError, Framer};
///
/// let mut framer = RtuFramer::new();
/// // The serial line specification's example: read 10 registers from 0 of unit 1, CRC C5 CD.
/// let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x0A]);
/// let request = RtuAdu::new(UnitId::from(1), pdu)?;
/// let mut wire = BytesMut::new();
/// framer.encode(&request, &mut wire)?;
/// assert_eq!(&wire[..], &[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A, 0xC5, 0xCD]);
///
/// // A reply of one register (42), then the first byte of the next frame.
/// let mut buffer = BytesMut::from(&[0x01, 0x03, 0x02, 0x00, 0x2A, 0x39, 0x9B, 0x01][..]);
/// let reply = framer.decode(&mut buffer)?.expect("one whole frame");
/// assert_eq!(&reply.pdu()[..], &[0x03, 0x02, 0x00, 0x2A]);
/// assert_eq!(&buffer[..], &[0x01], "the extra byte stays");
///
/// // One flipped CRC bit and the frame is refused.
/// let mut broken = BytesMut::from(&[0x01, 0x03, 0x02, 0x00, 0x2A, 0x39, 0x9A][..]);
/// assert_eq!(framer.decode(&mut broken), Err(FrameError::Malformed));
/// # Ok::<(), FrameError>(())
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RtuFramer;

impl RtuFramer {
    /// A framer; it holds no state.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::RtuFramer;
    /// use stonecrab::transport::Framer;
    ///
    /// assert_eq!(RtuFramer::new().max_frame_bytes(), 256);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Framer<RtuAdu> for RtuFramer {
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<RtuAdu>, FrameError> {
        let (skip, size) = match candidate(buffer)? {
            Candidate::Frame(size) => (0, size),
            Candidate::Incomplete => match later_frame(buffer) {
                Some(found) => found,
                None => return Ok(None),
            },
            Candidate::Bad => later_frame(buffer).ok_or(FrameError::Malformed)?,
        };
        buffer.advance(skip);
        let mut frame = buffer.split_to(size);
        let unit_id = UnitId::from(frame.get_u8());
        frame.truncate(size.saturating_sub(OVERHEAD));
        RtuAdu::new(unit_id, frame.freeze()).map(Some)
    }

    fn encode(&mut self, frame: &RtuAdu, out: &mut BytesMut) -> Result<(), FrameError> {
        let unit = frame.unit_id.get();
        let crc = crc16(std::iter::once(&unit).chain(frame.pdu.iter()));
        out.put_u8(unit);
        out.put_slice(&frame.pdu);
        out.put_u16_le(crc);
        Ok(())
    }

    fn max_frame_bytes(&self) -> usize {
        RtuAdu::MAX_BYTES
    }

    fn resync_discards(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> (Result<Option<RtuAdu>, FrameError>, Vec<u8>) {
        let mut buffer = BytesMut::from(bytes);
        let decoded = RtuFramer::new().decode(&mut buffer);
        (decoded, buffer.to_vec())
    }

    // covers: MBS-RTU-CRC / the published check value of CRC-16/MODBUS
    #[test]
    fn crc16_of_the_check_string_is_the_published_check_value() {
        assert_eq!(crc16(b"123456789"), 0x4B37, "CRC-16/MODBUS check value");
        assert_eq!(crc16(&[]), 0xFFFF, "the initial value, no final XOR");
    }

    // covers: MBS-RTU-CRC / the serial line specification examples
    #[test]
    fn crc16_of_the_spec_example_frames_matches() {
        // Modbus over Serial Line V1.02: read 10 holding registers of unit 1, CRC C5 CD.
        assert_eq!(
            crc16(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]).to_le_bytes(),
            [0xC5, 0xCD]
        );
        // Modbus Application Protocol V1.1b section 6.3 (registers 108 to 110 of unit 17).
        assert_eq!(
            crc16(&[0x11, 0x03, 0x00, 0x6B, 0x00, 0x03]).to_le_bytes(),
            [0x76, 0x87]
        );
    }

    // covers: MBS-RTU-SIZE-TABLE / the size of a response by function code
    #[test]
    fn the_size_table_is_upstreams() {
        // The cases of upstream's Go codec test (`MessageCodecRtu_test.go`, the size table).
        for (function, count, size) in [
            (0x01, Some(2), Size::Bytes(7)),
            (0x02, Some(1), Size::Bytes(6)),
            (0x03, Some(0x14), Size::Bytes(25)),
            (0x04, Some(2), Size::Bytes(7)),
            (0x14, Some(6), Size::Bytes(11)),
            (0x15, Some(9), Size::Bytes(14)),
            (0x17, Some(4), Size::Bytes(9)),
            (0x05, Some(0xFF), Size::Bytes(8)),
            (0x06, Some(0), Size::Bytes(8)),
            (0x0F, Some(0), Size::Bytes(8)),
            (0x10, Some(0), Size::Bytes(8)),
            (0x83, Some(2), Size::Bytes(5)),
            (0xFF, Some(0), Size::Bytes(5)),
            (0x03, None, Size::Pending),
            (0x2B, Some(0), Size::Unknown),
            (0x00, Some(0), Size::Unknown),
        ] {
            assert_eq!(
                response_size(function, count.as_ref()),
                size,
                "function {function:#04x}"
            );
        }
    }

    // covers: MBS-RTU-SIZE-TABLE / a byte count beyond the 256-byte ADU is refused from the header
    #[test]
    fn a_byte_count_beyond_the_adu_limit_is_oversized_from_the_header() {
        let (decoded, rest) = decode(&[0x01, 0x03, 0xFC]);
        assert_eq!(decoded, Err(FrameError::Oversized(257)));
        assert_eq!(rest, [0x01, 0x03, 0xFC], "nothing removed");
        let (decoded, _) = decode(&[0x01, 0x03, 0xFB]);
        assert_eq!(
            decoded,
            Ok(None),
            "251 bytes of data make the largest frame"
        );
    }

    // covers: MBS-RTU-RESYNC / noise with no whole frame later is Malformed and kept
    #[test]
    fn noise_that_holds_no_whole_frame_later_is_malformed_and_kept() {
        let (decoded, rest) = decode(&[0x01, 0x2B, 0x01, 0x03]);
        assert_eq!(decoded, Err(FrameError::Malformed));
        assert_eq!(rest, [0x01, 0x2B, 0x01, 0x03], "nothing removed");
    }

    // covers: MBS-RTU-RESYNC / noise in front of a whole frame is dropped with the frame
    #[test]
    fn noise_in_front_of_a_whole_frame_is_dropped_with_it() {
        let (decoded, rest) = decode(&[0x00, 0x2B, 0x01, 0x83, 0x02, 0xC0, 0xF1, 0x07]);
        let adu = decoded.expect("no error").expect("a frame");
        assert_eq!(adu.unit_id(), UnitId::from(1));
        assert_eq!(&adu.pdu()[..], &[0x83, 0x02]);
        assert_eq!(rest, [0x07]);
    }

    #[test]
    fn an_adu_needs_a_pdu_of_1_to_253_bytes() {
        let unit = UnitId::default();
        assert_eq!(RtuAdu::new(unit, Bytes::new()), Err(FrameError::Malformed));
        assert_eq!(
            RtuAdu::new(unit, Bytes::from(vec![0; 254])),
            Err(FrameError::Oversized(257))
        );
        let largest = RtuAdu::new(unit, Bytes::from(vec![0; 253])).expect("253 bytes");
        assert_eq!(largest.wire_len(), RtuAdu::MAX_BYTES);
    }

    #[test]
    fn the_debug_text_shows_the_unit_and_the_pdu_length() {
        let adu = RtuAdu::new(UnitId::from(3), Bytes::from_static(&[0x03, 0x02])).expect("adu");
        assert_eq!(
            format!("{adu:?}"),
            "RtuAdu { unit_id: UnitId(3), pdu_len: 2 }"
        );
    }
}
