use bytes::BytesMut;

use crate::FrameError;

/// Splits a byte stream into frames and writes frames back to bytes.
///
/// A framer is the protocol-specific half of [`FramedTransport`](crate::FramedTransport):
/// the transport moves bytes, the framer knows where a frame starts and ends (the Modbus
/// `MbapFramer`, a serial framer, the testkit's `LengthPrefixedFramer`). It is a plain
/// value with `&mut self` methods, so it may keep state, and it does no I/O. `F` is the
/// frame type it produces and consumes; for Modbus that is a decoded ADU, for the test
/// framer it is `Vec<u8>`.
///
/// # Rules every implementation follows
/// - **Wire size.** "Frame size" means the bytes of the whole frame on the wire, header
///   included, the same unit as [`ResourceLimits::max_frame_bytes`] and
///   [`Transport::send`](crate::Transport::send). [`FrameError::Oversized`] carries it.
/// - **Never panic.** `decode` is fed whatever a peer sends, including random bytes and
///   truncated or hostile headers; it never panics, indexes unchecked, or allocates
///   from a declared length before comparing that length with `max_frame_bytes`.
/// - **Leave the buffer alone unless a frame comes out.** `decode` removes bytes from the
///   front of the buffer only when it returns `Ok(Some(_))`. On `Ok(None)` and on `Err`
///   the buffer is exactly as it was, so a later call sees the same bytes (an error is
///   repeatable until [`FramedTransport::resync`](crate::FramedTransport::resync)
///   discards the offending bytes).
/// - **Header-only judgement.** A bad header (`Malformed`) or a declared size over the
///   maximum (`Oversized`) is reported as soon as the header is complete, without waiting
///   for the payload.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
/// Upstream frames inside Netty `ByteToMessageDecoder` handlers and has no equivalent
/// trait; this is a Stonecrab contract with no upstream parity claim.
///
/// # Examples
/// A framer for lines ending in `\n` (the frame type is the line without the newline):
/// ```
/// use bytes::{Buf, BufMut, BytesMut};
/// use stonecrab_transport::{FrameError, Framer};
///
/// struct Lines {
///     max: usize,
/// }
///
/// impl Framer<String> for Lines {
///     fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<String>, FrameError> {
///         let Some(end) = buffer.iter().position(|byte| *byte == b'\n') else {
///             // No newline yet: wait, unless the line can no longer fit.
///             return if buffer.len() >= self.max {
///                 Err(FrameError::Oversized(buffer.len()))
///             } else {
///                 Ok(None)
///             };
///         };
///         let total = end.saturating_add(1);
///         if total > self.max {
///             return Err(FrameError::Oversized(total));
///         }
///         let text = buffer.get(..end).ok_or(FrameError::Malformed)?;
///         let line = String::from_utf8(text.to_vec()).map_err(|_| FrameError::Malformed)?;
///         buffer.advance(total);
///         Ok(Some(line))
///     }
///
///     fn encode(&mut self, frame: &String, out: &mut BytesMut) -> Result<(), FrameError> {
///         let total = frame.len().saturating_add(1);
///         if total > self.max {
///             return Err(FrameError::Oversized(total));
///         }
///         out.put_slice(frame.as_bytes());
///         out.put_u8(b'\n');
///         Ok(())
///     }
///
///     fn max_frame_bytes(&self) -> usize {
///         self.max
///     }
/// }
///
/// let mut framer = Lines { max: 16 };
/// let mut buffer = BytesMut::from(&b"ping\npo"[..]);
/// assert_eq!(framer.decode(&mut buffer), Ok(Some("ping".to_owned())));
/// assert_eq!(framer.decode(&mut buffer), Ok(None), "the second line is incomplete");
/// assert_eq!(&buffer[..], b"po", "an incomplete frame is left in the buffer");
/// ```
///
/// [`ResourceLimits::max_frame_bytes`]: stonecrab_core::ResourceLimits::max_frame_bytes
pub trait Framer<F>: Send {
    /// Take one complete frame from the front of `buffer`.
    ///
    /// Returns `Ok(Some(frame))` after removing exactly that frame's bytes from the
    /// front of `buffer` (bytes behind it stay, so one buffer may hold several frames),
    /// and `Ok(None)` when the buffer does not yet hold a whole frame (including an
    /// empty buffer or a header that is still incomplete). More bytes are then needed;
    /// the buffer is untouched.
    ///
    /// The declared frame size is compared with [`max_frame_bytes`](Self::max_frame_bytes)
    /// as soon as the header is complete and before anything is allocated or waited for.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
    /// No upstream parity claim (see the trait).
    ///
    /// # Errors
    /// - [`FrameError::Oversized`] if the header declares a frame larger than
    ///   `max_frame_bytes`; the buffer is untouched.
    /// - [`FrameError::Malformed`] if the header is invalid (a wrong protocol id, an
    ///   impossible length); the buffer is untouched.
    /// - [`FrameError::Truncated`] is not returned by `decode`: an incomplete frame is
    ///   `Ok(None)`. It is available to framers that decode a whole slice at once.
    ///
    /// Both map to `ErrorKind`s through `From<FrameError> for PlcError`: `Oversized` is
    /// `ResourceLimit`, `Malformed` is `MalformedData`.
    fn decode(&mut self, buffer: &mut BytesMut) -> Result<Option<F>, FrameError>;

    /// Append the wire form of `frame` to `out`.
    ///
    /// `out` may already hold bytes (a scratch buffer is reused); `encode` only appends.
    /// On `Err` nothing was appended. The encoded frame must decode back to an equal
    /// frame.
    ///
    /// # Compatibility: docs/parity/transport.md#TRANSPORT-FRAMING
    /// No upstream parity claim (see the trait).
    ///
    /// # Errors
    /// - [`FrameError::Oversized`] if the encoded frame would exceed `max_frame_bytes`.
    /// - [`FrameError::Malformed`] if `frame` cannot be represented on the wire.
    fn encode(&mut self, frame: &F, out: &mut BytesMut) -> Result<(), FrameError>;

    /// The largest frame this framer accepts or produces, in bytes on the wire, header
    /// included.
    ///
    /// [`FramedTransport`](crate::FramedTransport) never sends more than the smaller of
    /// this and the transport's `ResourceLimits::max_frame_bytes`. A framer whose maximum
    /// is above the transport's `max_buffered_bytes` can hit the buffer bound first
    /// (`ResourceLimit` without a frame boundary).
    fn max_frame_bytes(&self) -> usize;
}
