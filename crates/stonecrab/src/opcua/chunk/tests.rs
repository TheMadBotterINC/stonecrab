use super::*;

// # Compatibility: docs/parity/opcua.md#OPCUA-CHUNK-SIZE
#[test]
fn opcua_max_body_size_is_derived_from_the_send_buffer_and_the_policy() {
    for buffer in [25, 8192, 65535] {
        assert_eq!(
            SecuritySizes::NONE.max_body(buffer, 4).unwrap(),
            buffer.checked_sub(24).unwrap()
        );
    }
    let encrypted = SecuritySizes {
        signature: 32,
        plain_block: 16,
        cipher_block: 16,
        padding: 1,
    };
    assert_eq!(encrypted.max_body(8192, 4).unwrap(), 8135);
    let asymmetric = SecuritySizes {
        signature: 256,
        plain_block: 214,
        cipher_block: 256,
        padding: 1,
    };
    assert_eq!(asymmetric.max_body(8192, 59).unwrap(), 6369);
    for (sizes, buffer, header) in [
        (SecuritySizes::NONE, 11, 0),
        (SecuritySizes::NONE, 16, 5),
        (SecuritySizes::NONE, 24, 4),
        (
            SecuritySizes {
                cipher_block: 0,
                ..SecuritySizes::NONE
            },
            8192,
            4,
        ),
        (
            SecuritySizes {
                plain_block: usize::MAX,
                ..SecuritySizes::NONE
            },
            8192,
            4,
        ),
        (encrypted, 32, 4),
        (asymmetric, 512, 59),
    ] {
        assert_eq!(
            sizes.max_body(buffer, header).unwrap_err().kind(),
            ErrorKind::ResourceLimit
        );
    }
}

#[test]
fn opcua_send_plan_checks_exact_message_and_chunk_boundaries() {
    let limits = MessageLimits {
        bytes: 100,
        chunks: 3,
    };
    assert_eq!(limits.plan(99, 57).unwrap(), (33, 3));
    assert_eq!(limits.plan(100, 58).unwrap(), (34, 3));
    assert_eq!(
        limits.plan(101, 58).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        limits.plan(100, 57).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        limits.plan(1, 24).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    if usize::BITS > 32 {
        assert_eq!(
            MessageLimits {
                bytes: usize::MAX,
                chunks: u32::MAX
            }
            .plan(usize::MAX, 25)
            .unwrap_err()
            .kind(),
            ErrorKind::ResourceLimit
        );
    }
}

// # Compatibility: docs/parity/opcua.md#OPCUA-CHUNK-RECEIVE
#[test]
fn opcua_chunks_are_reassembled_in_order() {
    let limits = MessageLimits {
        bytes: 6,
        chunks: 3,
    };
    let mut assembly = Assembly::default();
    assert_eq!(assembly.push(b"ab", false, limits).unwrap(), None);
    assert_eq!(assembly.push(b"c", false, limits).unwrap(), None);
    assert_eq!(
        assembly.push(b"def", true, limits).unwrap(),
        Some(Bytes::from_static(b"abcdef"))
    );
    assert_eq!(assembly.len(), 0);
    assert_eq!(assembly.chunks, 0);
    assert_eq!(
        assembly.push(b"z", true, limits).unwrap(),
        Some(Bytes::from_static(b"z"))
    );
    assert_eq!(
        assembly.push(b"", true, limits).unwrap(),
        Some(Bytes::new())
    );
}

#[test]
fn opcua_a_message_over_the_max_message_size_is_rejected_before_allocation() {
    let limits = MessageLimits {
        bytes: 3,
        chunks: 3,
    };
    let mut assembly = Assembly::default();
    assembly.push(b"abc", false, limits).unwrap();
    let capacity = assembly.body.capacity();
    assert_eq!(
        assembly.push(b"d", true, limits).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(assembly.body, b"abc");
    assert_eq!(assembly.body.capacity(), capacity);
    assert_eq!(assembly.chunks, 1);
}

#[test]
fn opcua_more_chunks_than_the_negotiated_count_are_rejected() {
    let limits = MessageLimits {
        bytes: 8,
        chunks: 1,
    };
    let mut assembly = Assembly::default();
    assembly.push(b"a", false, limits).unwrap();
    assert_eq!(
        assembly.push(b"", true, limits).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(assembly.body, b"a");
    assert_eq!(assembly.chunks, 1);
    assembly.chunks = u32::MAX;
    assert_eq!(
        assembly.push(b"b", false, limits).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
}

#[test]
fn opcua_abort_bodies_are_checked_without_echoing_the_reason() {
    let bytes = [
        0x8008_0000_u32.to_le_bytes().to_vec(),
        6_i32.to_le_bytes().to_vec(),
        b"secret".to_vec(),
    ]
    .concat();
    let problem = abort(&bytes).unwrap();
    assert_eq!(problem.kind(), ErrorKind::DeviceException);
    assert!(problem.to_string().contains("80080000"));
    assert!(!problem.to_string().contains("secret"));
    for length in 0..bytes.len() {
        assert!(abort(bytes.get(..length).unwrap()).is_err());
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert!(abort(&trailing).is_err());
}

#[test]
fn opcua_receive_growth_checks_arithmetic_and_exact_budget_before_allocating() {
    assert_eq!(check_growth(2, 3, 5).unwrap(), 5);
    assert_eq!(
        check_growth(2, 4, 5).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        check_growth(usize::MAX, 1, usize::MAX).unwrap_err().kind(),
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        check_growth(usize::MAX - 1, 1, usize::MAX).unwrap(),
        usize::MAX
    );
}

#[test]
fn opcua_unrepresentable_allocation_is_a_resource_error_without_partial_growth() {
    let mut assembly = Assembly::default();
    assembly.body.extend_from_slice(b"retained");
    let capacity = assembly.body.capacity();
    let problem = assembly.reserve(usize::MAX).unwrap_err();
    assert_eq!(problem.kind(), ErrorKind::ResourceLimit);
    assert!(problem.to_string().contains("allocation failed"));
    assert_eq!(assembly.body, b"retained");
    assert_eq!(assembly.body.capacity(), capacity);
}
