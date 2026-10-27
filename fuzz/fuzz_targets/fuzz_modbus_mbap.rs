//! Feeds arbitrary bytes to the Modbus TCP framer, the way a peer's stream reaches it.
//!
//! The target must never panic. Every `Ok(Some)` must be a whole ADU of 8 to 260 bytes that
//! came off the front of the buffer, and must re-encode to exactly the bytes it consumed;
//! the buffer never grows; and `Ok(None)` and `Err` leave the buffer exactly as it was,
//! with an `Err` that is `Malformed` or an `Oversized` that names a size above the 260 byte
//! limit. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use stonecrab::modbus::MbapFramer;
use stonecrab::transport::{FrameError, Framer};

fuzz_target!(|data: &[u8]| {
    let mut framer = MbapFramer::new();
    let limit = framer.max_frame_bytes();
    assert_eq!(limit, 260, "the ADU limit");
    let mut buffer = BytesMut::from(data);
    loop {
        let before = buffer.clone();
        match framer.decode(&mut buffer) {
            Ok(Some(adu)) => {
                let consumed = before.len() - buffer.len();
                assert!(
                    (8..=limit).contains(&consumed),
                    "an ADU is 8 to 260 bytes, got {consumed}"
                );
                assert_eq!(adu.wire_len(), consumed, "wire_len is the consumed size");
                assert_eq!(&buffer[..], &before[consumed..], "the rest is untouched");
                let mut wire = BytesMut::new();
                assert_eq!(
                    framer.encode(&adu, &mut wire),
                    Ok(()),
                    "a decoded ADU encodes"
                );
                assert_eq!(
                    &wire[..],
                    &before[..consumed],
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
