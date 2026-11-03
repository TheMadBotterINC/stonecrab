//! Feeds arbitrary bytes to the Modbus ASCII framer, the way a serial line's characters reach
//! it.
//!
//! The target must never panic. Every `Ok(Some)` must be a whole ADU of 9 to 513 bytes that
//! came off the buffer after at most the noise the framer skipped, and must re-encode to the
//! bytes it consumed (upper case, whatever the case it was read in), so its hex and its LRC
//! were right; the buffer never grows; and `Ok(None)` and `Err` leave the buffer exactly as it
//! was, with an `Err` that is `Malformed` or an `Oversized` that names a size of at least the
//! 513 byte limit. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use stonecrab::modbus::AsciiFramer;
use stonecrab::transport::{FrameError, Framer};

fuzz_target!(|data: &[u8]| {
    let mut framer = AsciiFramer::new();
    let limit = framer.max_frame_bytes();
    assert_eq!(limit, 513, "the ADU limit");
    assert!(framer.resync_discards(), "ASCII resynchronizes by dropping the buffer");
    let mut buffer = BytesMut::from(data);
    loop {
        let before = buffer.clone();
        match framer.decode(&mut buffer) {
            Ok(Some(adu)) => {
                let consumed = before.len() - buffer.len();
                let size = adu.wire_len();
                assert!((9..=limit).contains(&size), "an ADU is 9 to 513 bytes, got {size}");
                assert!(consumed >= size, "the frame came off the buffer");
                let skipped = consumed - size;
                assert_eq!(&buffer[..], &before[consumed..], "the rest is untouched");
                let mut wire = BytesMut::new();
                assert_eq!(framer.encode(&adu, &mut wire), Ok(()), "a decoded ADU encodes");
                assert!(
                    wire[..].eq_ignore_ascii_case(&before[skipped..consumed]),
                    "an accepted ADU re-encodes to the bytes it consumed"
                );
                assert_eq!(wire[0], b':', "a frame starts at a colon");
            }
            Ok(None) => {
                assert_eq!(buffer, before, "waiting leaves the buffer alone");
                break;
            }
            Err(error) => {
                assert_eq!(buffer, before, "an error leaves the buffer alone");
                match error {
                    FrameError::Malformed => {}
                    FrameError::Oversized(size) => {
                        assert!(size >= limit, "oversized names a size of the limit or more");
                    }
                    other => panic!("decode returned {other:?}"),
                }
                break;
            }
        }
    }
});
