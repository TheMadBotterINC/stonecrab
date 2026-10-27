use crate::transport::{FrameError, Framer};
use bytes::{Buf, BufMut, BytesMut};

/// Bytes of the big-endian length prefix.
const HEADER_LEN: usize = 2;

/// The largest frame a 2-byte prefix can describe: the prefix plus 65535 payload bytes.
const LARGEST_FRAME: usize = HEADER_LEN + 65_535;

/// The sanctioned test [`Framer`]: a 2-byte big-endian payload length, then the payload.
///
/// It frames `Vec<u8>` payloads, so transport tests and later crates can exercise
/// `FramedTransport` without inventing a framer of their own (coding standards 13.5: test
/// doubles of our traits live in the testkit). The wire form of `b"hi"` is
/// `00 02 68 69`. An empty payload is a valid frame (`00 00`).
///
/// Every two bytes are a valid header, so `decode` never reports `Malformed`; a header
/// that declares more than the maximum is `Oversized` as soon as it is complete, before
/// the payload is awaited or any buffer is sized from it. There is no sync marker: after
/// garbage, `FramedTransport::resync` slides one byte at a time until a header fits the
/// maximum, which locks on only as well as the garbage allows.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
/// A Stonecrab test framer with no upstream counterpart.
///
/// # Examples
/// ```
/// use bytes::BytesMut;
/// use stonecrab::testkit::LengthPrefixedFramer;
/// use stonecrab::transport::{FrameError, Framer};
///
/// let mut framer = LengthPrefixedFramer::new(8);
/// let mut wire = BytesMut::new();
/// framer.encode(&b"hi".to_vec(), &mut wire)?;
/// assert_eq!(&wire[..], &[0, 2, b'h', b'i']);
///
/// wire.extend_from_slice(&[0, 3, b'a']);
/// assert_eq!(framer.decode(&mut wire)?, Some(b"hi".to_vec()));
/// assert_eq!(framer.decode(&mut wire)?, None, "the second frame is still incomplete");
///
/// let mut hostile = BytesMut::from(&[0xFF, 0xFF][..]);
/// assert_eq!(framer.decode(&mut hostile), Err(FrameError::Oversized(65_537)));
/// # Ok::<(), FrameError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LengthPrefixedFramer {
    max_frame_bytes: usize,
}

impl LengthPrefixedFramer {
    /// A framer whose frames, prefix included, may be at most `max_frame_bytes` long.
    ///
    /// A 2-byte prefix cannot describe more than 65,537 bytes on the wire, so larger
    /// values are lowered to that.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::testkit::LengthPrefixedFramer;
    /// use stonecrab::transport::Framer;
    ///
    /// assert_eq!(LengthPrefixedFramer::new(64).max_frame_bytes(), 64);
    /// assert_eq!(LengthPrefixedFramer::new(usize::MAX).max_frame_bytes(), 65_537);
    /// ```
    #[must_use]
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            max_frame_bytes: max_frame_bytes.min(LARGEST_FRAME),
        }
    }
}

impl Framer<Vec<u8>> for LengthPrefixedFramer {
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<Vec<u8>>, FrameError> {
        let Some(&header) = buffer.first_chunk::<HEADER_LEN>() else {
            return Ok(None);
        };
        let payload = usize::from(u16::from_be_bytes(header));
        let total = payload.saturating_add(HEADER_LEN);
        if total > self.max_frame_bytes {
            return Err(FrameError::Oversized(total));
        }
        if buffer.len() < total {
            return Ok(None);
        }
        buffer.advance(HEADER_LEN);
        Ok(Some(buffer.split_to(payload).to_vec()))
    }

    fn encode(&mut self, frame: &Vec<u8>, out: &mut BytesMut) -> Result<(), FrameError> {
        let total = frame.len().saturating_add(HEADER_LEN);
        let prefix =
            u16::try_from(frame.len()).map_err(|_overflow| FrameError::Oversized(total))?;
        if total > self.max_frame_bytes {
            return Err(FrameError::Oversized(total));
        }
        out.reserve(total);
        out.put_u16(prefix);
        out.put_slice(frame);
        Ok(())
    }

    fn max_frame_bytes(&self) -> usize {
        self.max_frame_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buffer(bytes: &[u8]) -> BytesMut {
        BytesMut::from(bytes)
    }

    // covers: SPEC mandatory-failure-cases / F2 fragmentation and coalescing (test framer)
    #[test]
    fn decode_takes_one_frame_and_leaves_the_rest() {
        let mut framer = LengthPrefixedFramer::new(16);
        let mut wire = buffer(&[0, 2, 1, 2, 0, 1, 9, 0]);
        assert_eq!(
            framer.decode(&mut wire),
            Ok(Some(vec![1, 2])),
            "first frame"
        );
        assert_eq!(framer.decode(&mut wire), Ok(Some(vec![9])), "second frame");
        assert_eq!(&wire[..], &[0], "a lone byte of the next header stays");
        assert_eq!(
            framer.decode(&mut wire),
            Ok(None),
            "an incomplete header waits"
        );
        assert_eq!(&wire[..], &[0], "and is not consumed");
    }

    #[test]
    fn an_empty_payload_is_a_frame_and_an_empty_buffer_is_not() {
        let mut framer = LengthPrefixedFramer::new(16);
        assert_eq!(
            framer.decode(&mut BytesMut::new()),
            Ok(None),
            "empty buffer"
        );
        assert_eq!(
            framer.decode(&mut buffer(&[0, 0])),
            Ok(Some(Vec::new())),
            "empty payload"
        );
    }

    #[test]
    fn an_incomplete_payload_waits_and_is_not_consumed() {
        let mut framer = LengthPrefixedFramer::new(16);
        let mut wire = buffer(&[0, 4, 1, 2, 3]);
        assert_eq!(
            framer.decode(&mut wire),
            Ok(None),
            "three of four payload bytes"
        );
        assert_eq!(wire.len(), 5, "nothing consumed");
    }

    #[test]
    fn an_oversized_header_fails_before_the_payload_arrives_and_leaves_the_buffer() {
        let mut framer = LengthPrefixedFramer::new(16);
        let mut wire = buffer(&[0, 15]);
        assert_eq!(
            framer.decode(&mut wire),
            Err(FrameError::Oversized(17)),
            "17 > 16"
        );
        assert_eq!(wire.len(), 2, "buffer untouched on error");
        let mut at_limit = buffer(&[0, 14, 0]);
        assert_eq!(
            framer.decode(&mut at_limit),
            Ok(None),
            "a frame of exactly the maximum is accepted and waits for its payload"
        );
    }

    #[test]
    fn encode_appends_and_refuses_oversized_payloads_without_writing() {
        let mut framer = LengthPrefixedFramer::new(5);
        let mut out = buffer(&[7]);
        framer
            .encode(&vec![1, 2, 3], &mut out)
            .expect("fits exactly");
        assert_eq!(
            &out[..],
            &[7, 0, 3, 1, 2, 3],
            "appended after existing bytes"
        );
        let before = out.len();
        assert_eq!(
            framer.encode(&vec![1, 2, 3, 4], &mut out),
            Err(FrameError::Oversized(6)),
            "one byte too many"
        );
        assert_eq!(out.len(), before, "nothing appended on error");
    }

    #[test]
    fn a_payload_beyond_the_prefix_range_is_oversized_not_truncated() {
        let mut framer = LengthPrefixedFramer::new(usize::MAX);
        let mut out = BytesMut::new();
        let payload = vec![0_u8; 65_536];
        assert_eq!(
            framer.encode(&payload, &mut out),
            Err(FrameError::Oversized(65_538)),
            "length does not fit in 16 bits"
        );
        assert!(out.is_empty(), "nothing appended");
    }

    #[test]
    fn the_maximum_is_clamped_to_what_the_prefix_can_describe() {
        assert_eq!(
            LengthPrefixedFramer::new(usize::MAX).max_frame_bytes(),
            65_537
        );
        let mut framer = LengthPrefixedFramer::new(usize::MAX);
        let mut wire = buffer(&[0xFF, 0xFF]);
        assert_eq!(
            framer.decode(&mut wire),
            Ok(None),
            "the largest frame is allowed"
        );
        let payload = vec![7_u8; 65_535];
        let mut out = BytesMut::new();
        framer.encode(&payload, &mut out).expect("largest payload");
        assert_eq!(
            framer.decode(&mut out),
            Ok(Some(payload)),
            "and round-trips"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths (framing)
    #[test]
    fn decode_never_panics_and_obeys_the_buffer_rules_on_short_inputs() {
        let mut framer = LengthPrefixedFramer::new(4);
        for high in [0_u8, 1, 255] {
            for low in 0..=u8::MAX {
                for tail in 0..=3_usize {
                    let mut bytes = vec![high, low];
                    bytes.extend(std::iter::repeat_n(7_u8, tail));
                    let mut wire = BytesMut::from(&bytes[..]);
                    match framer.decode(&mut wire) {
                        Ok(Some(frame)) => assert_eq!(
                            frame.len() + HEADER_LEN + wire.len(),
                            bytes.len(),
                            "a frame removes exactly its own bytes from {bytes:?}"
                        ),
                        Ok(None) => {
                            assert_eq!(&wire[..], &bytes[..], "waiting leaves {bytes:?} alone");
                        }
                        Err(error) => {
                            assert!(
                                matches!(error, FrameError::Oversized(size) if size > 4),
                                "only an oversized header is an error: {error:?}"
                            );
                            assert_eq!(&wire[..], &bytes[..], "an error leaves {bytes:?} alone");
                        }
                    }
                }
            }
        }
        for length in 0..=1_usize {
            let mut wire = BytesMut::from(&vec![0xFF; length][..]);
            assert_eq!(framer.decode(&mut wire), Ok(None), "a partial header waits");
        }
    }

    #[test]
    fn the_framer_is_send_and_copyable() {
        fn assert_send_copy<T: Send + Copy>() {}
        assert_send_copy::<LengthPrefixedFramer>();
    }
}
