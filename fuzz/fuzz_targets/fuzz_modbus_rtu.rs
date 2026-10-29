//! Feeds arbitrary bytes to the Modbus RTU framer, the way a serial line's bytes reach it.
//!
//! The target must never panic. Every `Ok(Some)` must be a whole ADU of 5 to 256 bytes that
//! came off the front of the buffer, after at most the noise the framer skipped, and must
//! re-encode to exactly the frame's bytes (so its CRC was right); the buffer never grows;
//! and `Ok(None)` and `Err` leave the buffer exactly as it was, with an `Err` that is
//! `Malformed` or an `Oversized` that names a size above the 256 byte limit. (No
//! `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use stonecrab::modbus::RtuFramer;
use stonecrab::transport::{FrameError, Framer};

fuzz_target!(|data: &[u8]| {
    let mut framer = RtuFramer::new();
    let limit = framer.max_frame_bytes();
    assert_eq!(limit, 256, "the ADU limit");
    assert!(framer.resync_discards(), "RTU resynchronizes by dropping the buffer");
    let mut buffer = BytesMut::from(data);
    loop {
        let before = buffer.clone();
        match framer.decode(&mut buffer) {
            Ok(Some(adu)) => {
                let consumed = before.len() - buffer.len();
                let size = adu.wire_len();
                assert!((5..=limit).contains(&size), "an ADU is 5 to 256 bytes, got {size}");
                assert!(consumed >= size, "the frame came off the buffer");
                let skipped = consumed - size;
                assert_eq!(&buffer[..], &before[consumed..], "the rest is untouched");
                let mut wire = BytesMut::new();
                assert_eq!(framer.encode(&adu, &mut wire), Ok(()), "a decoded ADU encodes");
                assert_eq!(
                    &wire[..],
                    &before[skipped..consumed],
                    "an accepted ADU re-encodes to the bytes it consumed"
                );
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
                        assert!(size > limit, "oversized names a size over the limit");
                    }
                    other => panic!("decode returned {other:?}"),
                }
                break;
            }
        }
    }
});
