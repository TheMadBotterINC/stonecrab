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

pub(super) fn acknowledge(receive: u32, send: u32, message: u32, chunks: u32) -> Vec<u8> {
    [
        b"ACKF".to_vec(),
        28_u32.to_le_bytes().to_vec(),
        vec![0; 4],
        receive.to_le_bytes().to_vec(),
        send.to_le_bytes().to_vec(),
        message.to_le_bytes().to_vec(),
        chunks.to_le_bytes().to_vec(),
    ]
    .concat()
}

pub(super) fn open(sequence: u32, request: u32, channel: u32, payload: &[u8]) -> Vec<u8> {
    let uri = b"http://opcfoundation.org/UA/SecurityPolicy#None";
    let body = [
        channel.to_le_bytes().to_vec(),
        i32::try_from(uri.len()).unwrap().to_le_bytes().to_vec(),
        uri.to_vec(),
        vec![0xFF; 8],
        sequence.to_le_bytes().to_vec(),
        request.to_le_bytes().to_vec(),
        payload.to_vec(),
    ]
    .concat();
    [
        b"OPNF".to_vec(),
        u32::try_from(body.len().checked_add(8).unwrap())
            .unwrap()
            .to_le_bytes()
            .to_vec(),
        body,
    ]
    .concat()
}

pub(super) fn open_response(
    sequence: u32,
    request: u32,
    channel: u32,
    token: u32,
    lifetime: u32,
    status: u32,
) -> Vec<u8> {
    let body = [
        vec![1, 0, 0xC1, 1],
        response_header(request, status),
        vec![0; 4],
        channel.to_le_bytes().to_vec(),
        token.to_le_bytes().to_vec(),
        1000_i64.to_le_bytes().to_vec(),
        lifetime.to_le_bytes().to_vec(),
        12_i32.to_le_bytes().to_vec(),
        b"server-nonce".to_vec(),
    ]
    .concat();
    open(sequence, request, channel, &body)
}
