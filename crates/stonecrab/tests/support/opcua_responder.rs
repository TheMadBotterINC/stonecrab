//! First-party, policy-None wire constructions independent of the generated codec.
//! Integers follow OPC UA Part 6 little-endian field order; no upstream captures.

pub(super) fn message(
    sequence: u32,
    request: u32,
    channel: u32,
    token: u32,
    payload: &[u8],
) -> Vec<u8> {
    let size = u32::try_from(payload.len())
        .unwrap()
        .checked_add(24)
        .unwrap();
    [
        b"MSGF".to_vec(),
        size.to_le_bytes().to_vec(),
        channel.to_le_bytes().to_vec(),
        token.to_le_bytes().to_vec(),
        sequence.to_le_bytes().to_vec(),
        request.to_le_bytes().to_vec(),
        payload.to_vec(),
    ]
    .concat()
}

pub(super) fn response_header(handle: u32, status: u32) -> Vec<u8> {
    [
        vec![0; 8],
        handle.to_le_bytes().to_vec(),
        status.to_le_bytes().to_vec(),
        vec![0],
        vec![0xFF; 4],
        vec![0, 0, 0],
    ]
    .concat()
}

pub(super) fn read_response(handle: u32, value: i32) -> Vec<u8> {
    [
        vec![1, 0, 0x7A, 2],
        response_header(handle, 0),
        1_i32.to_le_bytes().to_vec(),
        vec![1, 6],
        value.to_le_bytes().to_vec(),
        vec![0xFF; 4],
    ]
    .concat()
}

pub(super) fn service_fault(handle: u32, status: u32) -> Vec<u8> {
    [vec![1, 0, 0x8D, 1], response_header(handle, status)].concat()
}

pub(super) fn server_error(status: u32) -> Vec<u8> {
    [
        b"ERRF".to_vec(),
        16_u32.to_le_bytes().to_vec(),
        status.to_le_bytes().to_vec(),
        vec![0xFF; 4],
    ]
    .concat()
}
