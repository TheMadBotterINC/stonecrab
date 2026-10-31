//! Property tests over the framer: any message written is read back equal and measures as long
//! as it is written, and no bytes make the framer panic or accept what it cannot write back.

use bytes::{Bytes, BytesMut};
use proptest::prelude::*;

use crate::ResourceLimits;
use crate::opcua::codec::{StatusCode, UaByteString, UaString};
use crate::opcua::frame::{
    Acknowledge, AsymmetricHeader, Body, ChunkType, ErrorMessage, Frame, Hello, OpcuaFramer,
    OpenChunk, SecureChunk, SequenceHeader, TransportLimits,
};
use crate::transport::Framer;

/// A framer that accepts every size a test message can have.
fn wide() -> OpcuaFramer {
    let limits = ResourceLimits::builder()
        .max_frame_bytes(1 << 20)
        .max_buffered_bytes(1 << 22)
        .build()
        .expect("positive limits");
    OpcuaFramer::new(1 << 20, limits)
}

fn ua_string() -> impl Strategy<Value = UaString> {
    prop::option::of(prop::collection::vec(any::<u8>(), 0..40)).prop_map(UaString)
}

fn ua_byte_string() -> impl Strategy<Value = UaByteString> {
    prop::option::of(prop::collection::vec(any::<u8>(), 0..40)).prop_map(UaByteString)
}

fn limits() -> impl Strategy<Value = TransportLimits> {
    (any::<u32>(), any::<u32>(), any::<u32>(), any::<u32>()).prop_map(
        |(receive_buffer_size, send_buffer_size, max_message_size, max_chunk_count)| {
            TransportLimits {
                receive_buffer_size,
                send_buffer_size,
                max_message_size,
                max_chunk_count,
            }
        },
    )
}

fn sequence() -> impl Strategy<Value = SequenceHeader> {
    (any::<u32>(), any::<u32>()).prop_map(|(sequence_number, request_id)| SequenceHeader {
        sequence_number,
        request_id,
    })
}

fn payload() -> impl Strategy<Value = Bytes> {
    prop::collection::vec(any::<u8>(), 0..64).prop_map(Bytes::from)
}

fn secure_chunk() -> impl Strategy<Value = SecureChunk> {
    (any::<u32>(), any::<u32>(), sequence(), payload()).prop_map(
        |(channel_id, token_id, sequence, body)| SecureChunk {
            channel_id,
            token_id,
            sequence,
            body,
        },
    )
}

fn body() -> impl Strategy<Value = Body> {
    prop_oneof![
        (any::<u32>(), limits(), ua_string()).prop_map(
            |(protocol_version, limits, endpoint_url)| {
                Body::Hello(Hello {
                    protocol_version,
                    limits,
                    endpoint_url,
                })
            }
        ),
        (any::<u32>(), limits()).prop_map(|(protocol_version, limits)| {
            Body::Acknowledge(Acknowledge {
                protocol_version,
                limits,
            })
        }),
        (any::<u32>(), ua_string()).prop_map(|(status, reason)| {
            Body::Error(ErrorMessage {
                error: StatusCode(status),
                reason,
            })
        }),
        (
            any::<u32>(),
            ua_string(),
            ua_byte_string(),
            ua_byte_string(),
            sequence(),
            payload()
        )
            .prop_map(
                |(channel_id, uri, certificate, thumbprint, sequence, body)| {
                    Body::Open(OpenChunk {
                        channel_id,
                        security: AsymmetricHeader {
                            security_policy_uri: uri,
                            sender_certificate: certificate,
                            receiver_thumbprint: thumbprint,
                        },
                        sequence,
                        body,
                    })
                }
            ),
        secure_chunk().prop_map(Body::Close),
        secure_chunk().prop_map(Body::Message),
    ]
}

fn frame() -> impl Strategy<Value = Frame> {
    let chunk = prop_oneof![
        Just(ChunkType::Intermediate),
        Just(ChunkType::Final),
        Just(ChunkType::Abort)
    ];
    (chunk, body()).prop_map(|(chunk, body)| Frame { chunk, body })
}

proptest! {
    #[test]
    fn opcua_prop_frame_round_trips_and_measures_as_it_is_written(frame in frame()) {
        let mut out = BytesMut::new();
        wide().encode(&frame, &mut out).expect("encodes");
        prop_assert_eq!(frame.length_in_bytes(), Ok(out.len()));
        let mut buffer = out.clone();
        prop_assert_eq!(wide().decode(&mut buffer), Ok(Some(frame)));
        prop_assert!(buffer.is_empty(), "every byte was used");
    }

    #[test]
    fn opcua_prop_frame_decode_never_panics_and_what_it_takes_it_writes_back(
        bytes in prop::collection::vec(any::<u8>(), 0..160),
    ) {
        let mut buffer = BytesMut::from(bytes.as_slice());
        let decoded = wide().decode(&mut buffer);
        if let Ok(Some(frame)) = decoded {
            let used = bytes.len().saturating_sub(buffer.len());
            let mut out = BytesMut::new();
            wide().encode(&frame, &mut out).expect("what decoded encodes");
            prop_assert_eq!(bytes.get(..used), Some(&out[..]), "the same bytes");
        } else {
            prop_assert_eq!(buffer.len(), bytes.len(), "nothing consumed");
        }
    }

    #[test]
    fn opcua_prop_frame_survives_a_cut_or_flipped_byte(
        frame in frame(),
        index in any::<prop::sample::Index>(),
        flip in 1_u8..=255,
    ) {
        let mut out = BytesMut::new();
        wide().encode(&frame, &mut out).expect("encodes");
        let at = index.index(out.len());
        let mut cut = BytesMut::from(out.get(..at).unwrap_or_default());
        prop_assert_eq!(wide().decode(&mut cut), Ok(None), "a prefix waits");
        let mut flipped = out.clone();
        if let Some(byte) = flipped.get_mut(at) {
            *byte ^= flip;
        }
        let before = flipped.len();
        let decoded = wide().decode(&mut flipped);
        if let Ok(Some(_)) = decoded {
            prop_assert!(flipped.len() < before, "a message was taken");
        } else {
            prop_assert_eq!(flipped.len(), before, "nothing consumed");
        }
    }
}
