//! Feeds arbitrary bytes to the OPC UA TCP framer, the way a peer's stream reaches it (the
//! framer is crate-private, so the `cfg(fuzzing)` entry point `stonecrab::opcua::fuzzing` wraps
//! it).
//!
//! The first byte selects the receive buffer (65535, 4096, 256 or 24 bytes) and the rest is the
//! stream. The target must never panic or reserve memory for a declared size. Every `Ok(Some)`
//! must be a whole message of 8 bytes up to the receive buffer that came off the front of the
//! buffer, and must be written back as exactly the bytes it consumed (every field is kept as
//! read); the buffer never grows; and `Ok(None)` and `Err` leave the buffer exactly as it was,
//! with an `Err` that is `Malformed` or an `Oversized` that names a size above the receive
//! buffer. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use bytes::BytesMut;
use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::{frame, frame_limit};
use stonecrab::transport::FrameError;

fuzz_target!(|input: &[u8]| {
    let Some((selector, stream)) = input.split_first() else {
        return;
    };
    let limit = frame_limit(*selector);
    let mut buffer = BytesMut::from(stream);
    loop {
        let before = buffer.clone();
        match frame(*selector, &mut buffer) {
            Ok(Some(framed)) => {
                let consumed = before.len() - buffer.len();
                assert!(
                    (8..=limit).contains(&consumed),
                    "a message is 8 bytes up to the receive buffer, got {consumed}"
                );
                assert_eq!(&buffer[..], &before[consumed..], "the rest is untouched");
                assert_eq!(
                    framed.written.as_deref(),
                    Ok(&before[..consumed]),
                    "an accepted message is written back as the bytes it consumed"
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
                        assert!(size > limit, "oversized names a size over the receive buffer");
                    }
                    other => panic!("decode returned {other:?}"),
                }
                break;
            }
        }
    }
});
