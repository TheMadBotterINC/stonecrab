use std::error::Error;
use std::fmt;

use crate::{ErrorKind, PlcError};

/// A framing failure reported by a protocol framer built on a [`Transport`](crate::transport::Transport).
///
/// The transport module owns this type so that it does not depend on `stonecrab::codec`
/// (decision D22): a framer such as the Modbus `MbapFramer` converts the codec's
/// `ParseError` into a `FrameError`, and `FrameError` converts into [`PlcError`]. The
/// variants carry sizes only, never payload bytes.
///
/// # Compatibility: docs/parity/transport.md#TRANSPORT-BUFFER-BOUND
/// Upstream frames inside Netty pipelines and has no equivalent type; this is a
/// Stonecrab type with no upstream parity claim.
///
/// # Examples
/// ```
/// use stonecrab::{ErrorKind, PlcError};
/// use stonecrab::transport::FrameError;
///
/// let error = PlcError::from(FrameError::Oversized(70_000));
/// assert_eq!(error.kind(), ErrorKind::ResourceLimit);
/// assert_eq!(FrameError::Truncated.to_string(), "frame ends before its declared length");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameError {
    /// The bytes received so far end before the frame is complete.
    Truncated,
    /// The frame declares or contains this many bytes, more than the configured
    /// maximum frame size.
    Oversized(usize),
    /// The bytes do not form a valid frame (bad header, impossible length).
    Malformed,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("frame ends before its declared length"),
            Self::Oversized(size) => write!(f, "frame of {size} bytes exceeds the frame limit"),
            Self::Malformed => f.write_str("frame is malformed"),
        }
    }
}

impl Error for FrameError {}

/// `Truncated` and `Malformed` map to [`ErrorKind::MalformedData`]; `Oversized` maps to
/// [`ErrorKind::ResourceLimit`] because it reports a configured bound being exceeded.
/// The `FrameError` is kept as the error's `source()`.
impl From<FrameError> for PlcError {
    fn from(error: FrameError) -> Self {
        let kind = match error {
            FrameError::Truncated | FrameError::Malformed => ErrorKind::MalformedData,
            FrameError::Oversized(_) => ErrorKind::ResourceLimit,
        };
        Self::new(kind, error.to_string()).with_source(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: SPEC mandatory-failure-cases / F2 transport (framing error mapping)
    #[test]
    fn each_variant_maps_to_its_error_kind_and_keeps_the_source() {
        let cases = [
            (FrameError::Truncated, ErrorKind::MalformedData),
            (FrameError::Malformed, ErrorKind::MalformedData),
            (FrameError::Oversized(9), ErrorKind::ResourceLimit),
        ];
        for (frame_error, kind) in cases {
            let error = PlcError::from(frame_error);
            assert_eq!(error.kind(), kind, "kind for {frame_error:?}");
            assert_eq!(
                error.to_string(),
                frame_error.to_string(),
                "message for {frame_error:?}"
            );
            let source = Error::source(&error).expect("source retained");
            assert_eq!(
                source.to_string(),
                frame_error.to_string(),
                "source for {frame_error:?}"
            );
        }
    }

    #[test]
    fn display_names_the_size_and_never_a_payload() {
        assert_eq!(
            FrameError::Oversized(70_000).to_string(),
            "frame of 70000 bytes exceeds the frame limit"
        );
        assert_eq!(FrameError::Malformed.to_string(), "frame is malformed");
        assert_eq!(
            FrameError::Truncated.to_string(),
            "frame ends before its declared length"
        );
    }
}
