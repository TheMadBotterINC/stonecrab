//! Modbus ASCII framing: the LRC, the ADU (address, PDU, LRC as hex between a colon and a
//! CR LF) and the framer that finds ADUs in the characters of a serial line.

use std::fmt;

use crate::transport::{FrameError, Framer};
use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::modbus::{MAX_PDU_BYTES, UnitId};

/// The characters around the hex: the colon in front, CR and LF behind.
const DELIMITERS: usize = 3;

/// The binary bytes around the PDU: the address in front, the LRC behind.
const BINARY_OVERHEAD: usize = 2;

/// The fewest binary bytes of a frame: the address, a function code and the LRC.
const MIN_BINARY: usize = 3;

/// The wire size of an ADU whose PDU is `pdu_len` bytes: the colon, two hex characters for
/// each of the address, the PDU bytes and the LRC, and CR LF.
const fn wire_size(pdu_len: usize) -> usize {
    pdu_len
        .saturating_add(BINARY_OVERHEAD)
        .saturating_mul(2)
        .saturating_add(DELIMITERS)
}

/// The LRC of Modbus ASCII over `bytes` (the address and the PDU): the two's complement of
/// their sum modulo 256, so that the sum of all the bytes of a frame, the LRC included, is 0.
fn lrc<'a>(bytes: impl IntoIterator<Item = &'a u8>) -> u8 {
    bytes
        .into_iter()
        .fold(0_u8, |sum, byte| sum.wrapping_add(*byte))
        .wrapping_neg()
}

/// The upper-case hex digit of a value from 0 to 15.
const fn upper_hex(nibble: u8) -> u8 {
    if nibble < 10 {
        b'0'.saturating_add(nibble)
    } else {
        b'A'.saturating_add(nibble.saturating_sub(10))
    }
}

/// The value of a hex digit, upper case or lower case.
fn hex_value(digit: u8) -> Option<u8> {
    char::from(digit)
        .to_digit(16)
        .and_then(|value| u8::try_from(value).ok())
}

/// A Modbus ASCII application data unit: the one-byte address (the unit) and the PDU. The LRC
/// is not stored; [`AsciiFramer`] checks it on receive and computes it on send.
///
/// On the wire the ADU is a colon, the address, the PDU and the LRC as two upper-case hex
/// characters per byte, then CR LF: at most 513 bytes ([`MAX_BYTES`](Self::MAX_BYTES)).
/// There is no transaction id: a reply is matched to its request by address and function
/// code. The PDU stays opaque bytes here; `decode_response` reads it, exactly as for Modbus
/// TCP and RTU.
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-ASCII-WIRE-MODEL
/// Upstream's `ModbusAsciiADU` is the same three fields (`modbus.mspec`); the LRC is a
/// checked field there and a property of the frame here.
///
/// # Examples
/// ```
/// use bytes::Bytes;
/// use stonecrab::modbus::{AsciiAdu, UnitId};
///
/// // Read 10 holding registers from 0 of unit 1: ":01030000000AF2" and CR LF on the wire.
/// let adu = AsciiAdu::new(UnitId::from(1), Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x0A]))?;
/// assert_eq!(adu.unit_id(), UnitId::from(1));
/// assert_eq!(adu.wire_len(), 17);
/// # Ok::<(), stonecrab::transport::FrameError>(())
/// ```
#[derive(Clone, PartialEq, Eq)]
pub struct AsciiAdu {
    unit_id: UnitId,
    pdu: Bytes,
}

impl AsciiAdu {
    /// The largest ASCII ADU in bytes: the colon, two characters each for the address, a
    /// 253-byte PDU and the LRC, and CR LF (Modbus over Serial Line V1.02, section 2.5.2).
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-LIMIT-ASCII-BUFFER
    /// Upstream's `MODBUS_ASCII_MAX_SIZE`, 513.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::AsciiAdu;
    ///
    /// assert_eq!(AsciiAdu::MAX_BYTES, 513);
    /// ```
    pub const MAX_BYTES: usize = wire_size(MAX_PDU_BYTES);

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
    /// use stonecrab::modbus::{AsciiAdu, UnitId};
    /// use stonecrab::transport::FrameError;
    ///
    /// let unit = UnitId::default();
    /// assert!(AsciiAdu::new(unit, Bytes::from_static(&[0x03])).is_ok());
    /// assert_eq!(AsciiAdu::new(unit, Bytes::new()), Err(FrameError::Malformed));
    /// assert_eq!(
    ///     AsciiAdu::new(unit, Bytes::from(vec![0; 254])),
    ///     Err(FrameError::Oversized(515))
    /// );
    /// ```
    pub fn new(unit_id: UnitId, pdu: Bytes) -> Result<Self, FrameError> {
        if pdu.is_empty() {
            return Err(FrameError::Malformed);
        }
        if pdu.len() > MAX_PDU_BYTES {
            return Err(FrameError::Oversized(wire_size(pdu.len())));
        }
        Ok(Self { unit_id, pdu })
    }

    /// The address byte: the unit the request goes to, or the unit that answers.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{AsciiAdu, UnitId};
    ///
    /// let adu = AsciiAdu::new(UnitId::from(7), Bytes::from_static(&[0x03]))?;
    /// assert_eq!(adu.unit_id(), UnitId::from(7));
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn unit_id(&self) -> UnitId {
        self.unit_id
    }

    /// The PDU bytes, 1 to 253 of them, in binary (not the hex of the wire).
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{AsciiAdu, UnitId};
    ///
    /// let adu = AsciiAdu::new(UnitId::default(), Bytes::from_static(&[0x83, 0x02]))?;
    /// assert_eq!(&adu.pdu()[..], &[0x83, 0x02]);
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn pdu(&self) -> &Bytes {
        &self.pdu
    }

    /// The size of the ADU on the wire: the colon, the hex of the address, the PDU and the
    /// LRC, and CR LF.
    ///
    /// # Examples
    /// ```
    /// use bytes::Bytes;
    /// use stonecrab::modbus::{AsciiAdu, UnitId};
    ///
    /// let adu = AsciiAdu::new(UnitId::default(), Bytes::from_static(&[0x83, 0x02]))?;
    /// assert_eq!(adu.wire_len(), 11);
    /// # Ok::<(), stonecrab::transport::FrameError>(())
    /// ```
    #[must_use]
    pub const fn wire_len(&self) -> usize {
        wire_size(self.pdu.len())
    }
}

/// Shows the address and the PDU length, never the PDU bytes.
impl fmt::Debug for AsciiAdu {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AsciiAdu")
            .field("unit_id", &self.unit_id)
            .field("pdu_len", &self.pdu.len())
            .finish()
    }
}

/// What the bytes from one colon of the buffer are.
enum Candidate {
    /// A whole frame of this many bytes whose characters are hex and whose LRC is right.
    Frame(usize),
    /// Possibly a frame still arriving; more bytes are needed to tell.
    Incomplete,
    /// Not a frame: a character that is no hex digit, a second colon, an odd number of
    /// digits, fewer than 3 bytes, or a wrong LRC.
    Bad,
    /// More than 513 bytes without a CR LF: nothing that long is a frame.
    Oversized,
}

/// Judge the bytes from a colon, `bytes[0]`, on.
fn candidate(bytes: &[u8]) -> Candidate {
    let mut sum = 0_u8;
    let mut high = None;
    let mut decoded = 0_usize;
    for (index, byte) in bytes.iter().enumerate().skip(1) {
        // The largest frame ends with its CR at index 511 and its LF at 512, and the CR is
        // looked at before the LF: any other character at index 512 is no part of a frame.
        if index >= AsciiAdu::MAX_BYTES.saturating_sub(1) {
            return Candidate::Oversized;
        }
        if *byte == b'\r' {
            return match bytes.get(index.saturating_add(1)) {
                None => Candidate::Incomplete,
                Some(b'\n') if high.is_none() && decoded >= MIN_BINARY && sum == 0 => {
                    Candidate::Frame(index.saturating_add(2))
                }
                Some(_) => Candidate::Bad,
            };
        }
        let Some(digit) = hex_value(*byte) else {
            return Candidate::Bad;
        };
        match high.take() {
            None => high = Some(digit),
            Some(upper) => {
                sum = sum.wrapping_add(upper.wrapping_shl(4) | digit);
                decoded = decoded.saturating_add(1);
            }
        }
    }
    Candidate::Incomplete
}

/// The [`Framer`] for Modbus ASCII: finds ADUs in the characters of a serial line by their
/// colon and CR LF, checks the hex and the LRC, and writes ADUs with their LRC.
///
/// A frame is a colon, the address, the PDU and the LRC as two hex characters per byte, then
/// CR LF. Unlike RTU there is a start and an end marker, and no length: the frame ends at
/// its first CR LF. Hex digits are accepted in either case on receive and written upper case
/// on send, as upstream. `decode` looks at each colon of the buffer in turn:
/// 1. A colon that starts a whole frame (hex digits only, an even number of them, at least
///    3 bytes, a right LRC, ended by CR LF) gives that frame; the bytes before the colon
///    (noise, or the debris of an earlier frame) are removed with it.
/// 2. A colon that starts something that cannot be a frame (a character that is no hex
///    digit, a second colon before the CR LF, an odd number of digits, fewer than 3 bytes,
///    a wrong LRC) is skipped and the next colon is judged, so a good frame behind a broken
///    one is not lost; the next frame start is the colon, as Go's codec resynchronizes.
/// 3. A colon that starts a frame still arriving (no CR LF yet) is `Ok(None)`: wait. It is
///    always the last colon of the buffer, since a later colon would have made it a bad one.
/// 4. No colon holds a frame and none is still arriving: the first colon that started
///    something that is no frame decides, [`FrameError::Malformed`], or
///    [`FrameError::Oversized`] when it ran past 513 bytes without a CR LF (upstream's
///    DESYNCHRONIZED). A buffer with no colon at all is `Ok(None)` up to 513 bytes and
///    `Oversized` beyond. Nothing is removed.
///
/// There is no inter-character timeout (upstream has none either): slow bytes are accepted
/// until the request's deadline. A framer holds no state. As for RTU,
/// [`resync_discards`](Framer::resync_discards) is true: the tail of a frame the owner gave
/// up on carries no colon to search from, so a resynchronization drops every buffered
/// character. `max_frame_bytes` is 513.
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-ASCII-DELIMITERS
/// The frame end is upstream's (`ModbusAsciiMessageCodec.calculateTotalMessageSize`); the
/// hex rules are docs/parity/modbus-serial.md#MBS-ASCII-PARSE, the LRC is
/// docs/parity/modbus-serial.md#MBS-ASCII-LRC, the resynchronization is
/// docs/parity/modbus-serial.md#MBS-ASCII-RESYNC and the bound is
/// docs/parity/modbus-serial.md#MBS-LIMIT-ASCII-BUFFER.
///
/// # Examples
/// ```
/// use bytes::{Bytes, BytesMut};
/// use stonecrab::modbus::{AsciiAdu, AsciiFramer, UnitId};
/// use stonecrab::transport::{FrameError, Framer};
///
/// let mut framer = AsciiFramer::new();
/// // The upstream parser suite's request: read 10 registers from 0 of unit 1, LRC F2.
/// let pdu = Bytes::from_static(&[0x03, 0x00, 0x00, 0x00, 0x0A]);
/// let request = AsciiAdu::new(UnitId::from(1), pdu)?;
/// let mut wire = BytesMut::new();
/// framer.encode(&request, &mut wire)?;
/// assert_eq!(&wire[..], &b":01030000000AF2\r\n"[..]);
///
/// // A reply of one register (42) in lower case hex, a noise byte in front, the next
/// // frame begun.
/// let mut buffer = BytesMut::from(&b"\x00:010302002ad0\r\n:01"[..]);
/// let reply = framer.decode(&mut buffer)?.expect("one whole frame");
/// assert_eq!(&reply.pdu()[..], &[0x03, 0x02, 0x00, 0x2A]);
/// assert_eq!(&buffer[..], b":01", "the extra characters stay");
///
/// // One wrong LRC digit and the frame is refused.
/// let mut broken = BytesMut::from(&b":010302002AD1\r\n"[..]);
/// assert_eq!(framer.decode(&mut broken), Err(FrameError::Malformed));
/// # Ok::<(), FrameError>(())
/// ```
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AsciiFramer;

impl AsciiFramer {
    /// A framer; it holds no state.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::AsciiFramer;
    /// use stonecrab::transport::Framer;
    ///
    /// assert_eq!(AsciiFramer::new().max_frame_bytes(), 513);
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Framer<AsciiAdu> for AsciiFramer {
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<AsciiAdu>, FrameError> {
        let mut refusal = None;
        let colons = buffer
            .iter()
            .enumerate()
            .filter_map(|(index, byte)| (*byte == b':').then_some(index));
        for start in colons {
            match buffer.get(start..).map_or(Candidate::Bad, candidate) {
                Candidate::Frame(size) => {
                    buffer.advance(start);
                    let frame = buffer.split_to(size);
                    return adu(&frame).map(Some);
                }
                Candidate::Incomplete => return Ok(None),
                Candidate::Bad => {
                    refusal.get_or_insert(FrameError::Malformed);
                }
                Candidate::Oversized => {
                    refusal.get_or_insert(FrameError::Oversized(buffer.len()));
                }
            }
        }
        match refusal {
            Some(error) => Err(error),
            None if buffer.len() > AsciiAdu::MAX_BYTES => Err(FrameError::Oversized(buffer.len())),
            None => Ok(None),
        }
    }

    fn encode(&mut self, frame: &AsciiAdu, out: &mut BytesMut) -> Result<(), FrameError> {
        let unit = frame.unit_id.get();
        let check = lrc(std::iter::once(&unit).chain(frame.pdu.iter()));
        out.reserve(frame.wire_len());
        out.put_u8(b':');
        for byte in std::iter::once(&unit)
            .chain(frame.pdu.iter())
            .chain(std::iter::once(&check))
        {
            out.put_u8(upper_hex(byte.wrapping_shr(4)));
            out.put_u8(upper_hex(byte & 0x0F));
        }
        out.put_slice(b"\r\n");
        Ok(())
    }

    fn max_frame_bytes(&self) -> usize {
        AsciiAdu::MAX_BYTES
    }

    fn resync_discards(&self) -> bool {
        true
    }
}

/// The ADU of a whole frame `candidate` accepted: the hex between the colon and the CR LF
/// turned back into the address, the PDU and the LRC.
fn adu(frame: &[u8]) -> Result<AsciiAdu, FrameError> {
    let digits = frame
        .get(1..frame.len().saturating_sub(2))
        .unwrap_or_default();
    let binary: Option<Vec<u8>> = digits
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            pair.iter().try_fold(0_u8, |value, digit| {
                hex_value(*digit).map(|nibble| value.wrapping_shl(4) | nibble)
            })
        })
        .collect();
    let binary = binary.ok_or(FrameError::Malformed)?;
    let (unit, rest) = binary.split_first().ok_or(FrameError::Malformed)?;
    let (_lrc, pdu) = rest.split_last().ok_or(FrameError::Malformed)?;
    AsciiAdu::new(UnitId::from(*unit), Bytes::copy_from_slice(pdu))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(bytes: &[u8]) -> (Result<Option<AsciiAdu>, FrameError>, Vec<u8>) {
        let mut buffer = BytesMut::from(bytes);
        let decoded = AsciiFramer::new().decode(&mut buffer);
        (decoded, buffer.to_vec())
    }

    fn encode(unit: u8, pdu: &[u8]) -> Vec<u8> {
        let adu = AsciiAdu::new(UnitId::from(unit), Bytes::copy_from_slice(pdu)).expect("adu");
        let mut wire = BytesMut::new();
        AsciiFramer::new().encode(&adu, &mut wire).expect("encode");
        wire.to_vec()
    }

    // covers: MBS-ASCII-LRC / the LRC of the upstream suite's request and response
    #[test]
    fn lrc_of_the_spec_example_frame_matches() {
        // Address 1, read holding registers from 0, quantity 10: the bytes sum to 0x0E.
        assert_eq!(lrc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]), 0xF2);
        // Address 1, function 3, byte count 20, twenty zero bytes: the sum is 0x18.
        let mut response = vec![0x01, 0x03, 0x14];
        response.extend_from_slice(&[0; 20]);
        assert_eq!(lrc(&response), 0xE8);
        // The sum wraps at 256, and nothing sums to nothing.
        assert_eq!(lrc(&[0xFF, 0x02]), 0xFF);
        assert_eq!(lrc(&[]), 0x00);
    }

    // covers: MBS-ASCII-SEND / ascii_request_is_upper_case_hex_with_colon_and_crlf
    #[test]
    fn a_request_is_a_colon_upper_case_hex_and_crlf() {
        assert_eq!(
            encode(1, &[0x03, 0x00, 0x00, 0x00, 0x0A]),
            b":01030000000AF2\r\n"
        );
        assert_eq!(
            encode(0xAB, &[0xCD, 0xEF]),
            b":ABCDEF99\r\n",
            "digits A to F in upper case; AB + CD + EF is 0x267, so the LRC is 0x99"
        );
        assert_eq!(upper_hex(0), b'0');
        assert_eq!(upper_hex(9), b'9');
        assert_eq!(upper_hex(10), b'A');
        assert_eq!(upper_hex(15), b'F');
    }

    // covers: MBS-ASCII-DELIMITERS / ascii_frame_ends_at_the_first_crlf
    #[test]
    fn a_frame_ends_at_the_first_crlf_and_the_next_stays() {
        let (decoded, rest) = decode(b":010302002AD0\r\n:0103");
        let adu = decoded.expect("no error").expect("a frame");
        assert_eq!(adu.unit_id(), UnitId::from(1));
        assert_eq!(&adu.pdu()[..], &[0x03, 0x02, 0x00, 0x2A]);
        assert_eq!(adu.wire_len(), 15);
        assert_eq!(rest, b":0103");
    }

    // covers: MBS-ASCII-DELIMITERS / ascii_partial_frame_asks_for_more_data
    #[test]
    fn a_frame_that_is_still_arriving_waits_and_keeps_every_byte() {
        for partial in [
            &b""[..],
            b":",
            b":0103",
            b":010302002AD0",
            b":010302002AD0\r",
            b"junk",
            b"junk:010302",
        ] {
            let (decoded, rest) = decode(partial);
            assert_eq!(decoded, Ok(None), "{partial:?}");
            assert_eq!(rest, partial, "nothing removed");
        }
    }

    // covers: MBS-ASCII-PARSE / ascii_frame_without_a_colon_is_rejected
    #[test]
    fn characters_with_no_colon_are_no_frame() {
        let (decoded, rest) = decode(b"010302002AD0\r\n");
        assert_eq!(decoded, Ok(None), "it waits for a colon");
        assert_eq!(rest, b"010302002AD0\r\n", "nothing removed");
        let mut noise = vec![b'A'; AsciiAdu::MAX_BYTES];
        assert_eq!(
            decode(&noise).0,
            Ok(None),
            "513 characters may still be followed"
        );
        noise.push(b'A');
        assert_eq!(
            decode(&noise).0,
            Err(FrameError::Oversized(514)),
            "past 513 bytes with no colon the stream is out of sync"
        );
    }

    // covers: MBS-ASCII-PARSE / ascii_odd_hex_length_is_rejected
    // covers: MBS-ASCII-PARSE / ascii_non_hex_digit_is_rejected
    // covers: MBS-LIMIT-ASCII-BUFFER / ascii_frame_under_9_bytes_is_not_a_frame
    #[test]
    fn a_colon_that_starts_something_that_is_no_frame_is_malformed_and_kept() {
        for (name, bytes) in [
            ("an odd number of digits", &b":010302002AD\r\n"[..]),
            ("a character that is no hex digit", b":0103G2002AD0\r\n"),
            ("a space inside", b":0103 2002AD0\r\n"),
            ("a lone CR in the hex", b":010302\r002AD0\r\n"),
            ("a CR without its LF", b":010302002AD0\rX"),
            ("an LF without its CR", b":010302002AD0\n"),
            ("a CR after the CR", b":010302002AD0\r\r\n"),
            ("under 9 bytes", b":0103\r\n"),
            ("two bytes of payload", b":010300\r\n"),
            ("nothing between the colon and CR LF", b":\r\n"),
            ("a wrong LRC", b":010302002AD1\r\n"),
            ("a sum that is the LRC of another", b":010302002A30\r\n"),
        ] {
            let (decoded, rest) = decode(bytes);
            assert_eq!(decoded, Err(FrameError::Malformed), "{name}");
            assert_eq!(rest, bytes, "{name}: nothing removed");
        }
    }

    // covers: MBS-ASCII-PARSE / ascii_accepts_lower_case_hex
    #[test]
    fn hex_digits_are_accepted_in_either_case() {
        for text in [
            &b":0103020FaB40\r\n"[..],
            b":0103020FAB40\r\n",
            b":0103020fab40\r\n",
        ] {
            let (decoded, rest) = decode(text);
            let adu = decoded.expect("no error").expect("a frame");
            assert_eq!(&adu.pdu()[..], &[0x03, 0x02, 0x0F, 0xAB], "{text:?}");
            assert_eq!(rest, b"");
        }
    }

    // covers: MBS-ASCII-RESYNC / ascii_desync_resumes_at_the_next_colon
    #[test]
    fn noise_and_broken_frames_in_front_of_a_good_frame_go_with_it() {
        let good = &b":010302002AD0\r\n"[..];
        for (name, front) in [
            ("noise", &b"\x00\xFF xyz"[..]),
            ("the tail of an earlier frame", b"02002AD0\r\n"),
            ("a frame cut off by the next colon", b":0103"),
            ("a frame with a wrong LRC", b":010302002AD1\r\n"),
            ("a frame with a non-hex digit", b":01030Z002AD0\r\n"),
            ("colons", b"::::"),
        ] {
            let mut bytes = front.to_vec();
            bytes.extend_from_slice(good);
            bytes.extend_from_slice(b"\r");
            let (decoded, rest) = decode(&bytes);
            let adu = decoded.expect(name).expect(name);
            assert_eq!(&adu.pdu()[..], &[0x03, 0x02, 0x00, 0x2A], "{name}");
            assert_eq!(
                rest, b"\r",
                "{name}: only the frame and what is before it went"
            );
        }
    }

    // covers: MBS-ASCII-RESYNC / an oversized run in front of a good frame goes with it
    #[test]
    fn an_oversized_run_in_front_of_a_good_frame_goes_with_it() {
        let mut bytes = vec![b':'];
        bytes.extend_from_slice(&[b'A'; 600]);
        bytes.extend_from_slice(b":010302002AD0\r\n");
        let (decoded, rest) = decode(&bytes);
        assert_eq!(
            decoded.expect("no error").expect("a frame").unit_id(),
            UnitId::from(1)
        );
        assert_eq!(rest, b"");
    }

    // covers: MBS-ASCII-RESYNC / a broken frame followed by a frame still arriving waits
    #[test]
    fn a_broken_frame_in_front_of_a_frame_still_arriving_waits() {
        let (decoded, rest) = decode(b":010302002AD1\r\n:01030200");
        assert_eq!(decoded, Ok(None));
        assert_eq!(rest, b":010302002AD1\r\n:01030200");
    }

    // covers: MBS-LIMIT-ASCII-BUFFER / ascii_buffer_over_513_bytes_without_crlf_desynchronizes
    #[test]
    fn a_colon_without_a_terminator_within_513_bytes_is_oversized() {
        // The colon and 511 digits are 512 bytes: the 513th has not arrived, so the framer
        // waits, as upstream does until the buffer outgrows the bound.
        let mut bytes = vec![b':'];
        bytes.extend(std::iter::repeat_n(b'A', 511));
        assert_eq!(decode(&bytes).0, Ok(None), "512 bytes: still arriving");
        bytes.push(b'A');
        assert_eq!(
            decode(&bytes).0,
            Err(FrameError::Oversized(513)),
            "513 bytes and no CR LF: no frame is that long"
        );
        // The same, with a terminator behind it: 514 digits make a 517 byte frame.
        let mut long = vec![b':'];
        long.extend(std::iter::repeat_n(b'0', 514));
        long.extend_from_slice(b"\r\n");
        assert_eq!(decode(&long).0, Err(FrameError::Oversized(517)));
        assert_eq!(decode(&long).1, long, "nothing removed");
    }

    // covers: MBS-LIMIT-ASCII-BUFFER / the largest frame is 513 bytes
    #[test]
    fn the_largest_frame_is_513_bytes_and_round_trips() {
        let mut pdu = vec![0x03];
        pdu.extend_from_slice(&[0x5A; 252]);
        let wire = encode(0xF7, &pdu);
        assert_eq!(wire.len(), AsciiAdu::MAX_BYTES);
        let (decoded, rest) = decode(&wire);
        let adu = decoded.expect("no error").expect("a frame");
        assert_eq!(adu.unit_id(), UnitId::from(0xF7));
        assert_eq!(&adu.pdu()[..], &pdu[..]);
        assert_eq!(adu.wire_len(), AsciiAdu::MAX_BYTES);
        assert_eq!(rest, b"");
    }

    // covers: MBS-ASCII-WIRE-MODEL / an ADU needs a PDU of 1 to 253 bytes
    #[test]
    fn an_ascii_adu_needs_a_pdu_of_1_to_253_bytes() {
        let unit = UnitId::default();
        assert_eq!(
            AsciiAdu::new(unit, Bytes::new()),
            Err(FrameError::Malformed)
        );
        assert_eq!(
            AsciiAdu::new(unit, Bytes::from(vec![0; 254])),
            Err(FrameError::Oversized(515))
        );
        let largest = AsciiAdu::new(unit, Bytes::from(vec![0; 253])).expect("253 bytes");
        assert_eq!(largest.wire_len(), AsciiAdu::MAX_BYTES);
        assert_eq!(AsciiFramer::new().max_frame_bytes(), 513);
        assert!(AsciiFramer::new().resync_discards());
    }

    #[test]
    fn the_ascii_debug_text_shows_the_unit_and_the_pdu_length() {
        let adu = AsciiAdu::new(UnitId::from(3), Bytes::from_static(&[0x03, 0x02])).expect("adu");
        assert_eq!(
            format!("{adu:?}"),
            "AsciiAdu { unit_id: UnitId(3), pdu_len: 2 }"
        );
    }
}
