//! First-party, policy-None wire constructions independent of the generated codec.
//! Integers follow OPC UA Part 6 little-endian field order; no upstream captures.

use super::library;

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

fn string(value: &str) -> Vec<u8> {
    [
        i32::try_from(value.len()).unwrap().to_le_bytes().to_vec(),
        value.as_bytes().to_vec(),
    ]
    .concat()
}

pub(super) fn create_response(handle: u32, timeout: f64, status: u32) -> Vec<u8> {
    let application = [
        string("urn:peer"),
        string("urn:peer:product"),
        vec![0],
        vec![0; 4],
        vec![0xFF; 8],
        vec![0xFF; 4],
    ]
    .concat();
    let endpoint = [
        string("opc.tcp://fake:4840/path"),
        application,
        vec![0xFF; 4],
        1_u32.to_le_bytes().to_vec(),
        string("http://opcfoundation.org/UA/SecurityPolicy#None"),
        1_i32.to_le_bytes().to_vec(),
        string("anonymous-policy"),
        vec![0; 4],
        vec![0xFF; 12],
        string("http://opcfoundation.org/UA-Profile/Transport/uatcp-uasc-uabinary"),
        vec![0],
    ]
    .concat();
    [
        vec![1, 0, 0xD0, 1],
        response_header(handle, status),
        vec![0, 123],
        vec![0, 77],
        timeout.to_le_bytes().to_vec(),
        string("session-nonce"),
        vec![0xFF; 4],
        1_i32.to_le_bytes().to_vec(),
        endpoint,
        vec![0xFF; 4],
        vec![0xFF; 8],
        vec![0; 4],
    ]
    .concat()
}

pub(super) fn activate_response(handle: u32, status: u32) -> Vec<u8> {
    [
        vec![1, 0, 0xD6, 1],
        response_header(handle, status),
        string("activated-nonce"),
        vec![0xFF; 8],
    ]
    .concat()
}

pub(super) fn close_response(handle: u32, status: u32) -> Vec<u8> {
    [vec![1, 0, 0xDC, 1], response_header(handle, status)].concat()
}

pub(super) fn read_results(handle: u32, values: &[Vec<u8>]) -> Vec<u8> {
    [
        vec![1, 0, 0x7A, 2],
        response_header(handle, 0),
        i32::try_from(values.len()).unwrap().to_le_bytes().to_vec(),
        values.concat(),
        vec![0xFF; 4],
    ]
    .concat()
}

pub(super) fn int32_value(value: i32, status: u32) -> Vec<u8> {
    [
        vec![3, 6],
        value.to_le_bytes().to_vec(),
        status.to_le_bytes().to_vec(),
    ]
    .concat()
}

#[derive(Debug, Clone, Copy)]
enum Fault {
    Timeout,
    Disconnect,
    Exception,
}

#[derive(Debug, Default)]
struct State {
    reads: u32,
    values: std::collections::BTreeMap<u32, i32>,
    unknown: bool,
    faults: std::collections::VecDeque<Fault>,
}

/// First-party policy-None session/read responder, independent of generated codecs.
#[derive(Debug, Clone, Default)]
pub(super) struct Device(std::sync::Arc<std::sync::Mutex<State>>);

impl Device {
    pub(super) fn reads(&self) -> u32 {
        self.0.lock().unwrap().reads
    }
    pub(super) fn unknown_after_transmit(&self) {
        self.0.lock().unwrap().unknown = true;
    }
    pub(super) fn timeout(&self) {
        self.0.lock().unwrap().faults.push_back(Fault::Timeout);
    }
    pub(super) fn disconnect(&self) {
        self.0.lock().unwrap().faults.push_back(Fault::Disconnect);
    }
    pub(super) fn exception(&self) {
        self.0.lock().unwrap().faults.push_back(Fault::Exception);
    }

    pub(super) fn reactor(
        &self,
    ) -> impl FnMut(&[u8]) -> library::testkit::Reaction + Send + 'static {
        let device = self.clone();
        let mut sequence = 41_u32;
        move |bytes| {
            use library::{ByteOrder, codec::BoundedReader, testkit::Reaction};
            let little = ByteOrder::LittleEndian;
            if bytes.starts_with(b"HEL") {
                return Reaction::Reply(acknowledge(65535, 65535, 0, 0));
            }
            if bytes.starts_with(b"OPN") {
                return Reaction::Reply(open_response(41, 1, 17, 29, 120_000, 0));
            }
            if bytes.starts_with(b"CLO") {
                return Reaction::Silence;
            }
            let mut reader = BoundedReader::new(bytes);
            reader.read_bytes(20, 20).expect("MSG header");
            let request = reader.read_u32(little).expect("request id");
            assert_eq!(reader.read_u8().expect("type encoding"), 1);
            assert_eq!(reader.read_u8().expect("type namespace"), 0);
            let service = reader.read_u16(little).expect("service id");
            let body = match service {
                461 => create_response(request, 120_000.0, 0),
                467 => activate_response(request, 0),
                473 => {
                    device.0.lock().unwrap().faults.clear();
                    close_response(request, 0)
                }
                631 => {
                    let mut state = device.0.lock().unwrap();
                    state.reads = state.reads.saturating_add(1);
                    let fault = state.faults.pop_front();
                    drop(state);
                    match fault {
                        Some(Fault::Timeout) => return Reaction::Silence,
                        Some(Fault::Disconnect) => return Reaction::Close,
                        Some(Fault::Exception) => service_fault(request, 0x8001_0000),
                        None => {
                            reader
                                .read_bytes(29, 29)
                                .expect("request header with short token");
                            reader.read_bytes(12, 12).expect("maxAge and timestamps");
                            let count = reader.read_i32(little).expect("node count");
                            let mut values = Vec::new();
                            for _ in 0..count {
                                let node = match reader.read_u8().expect("node encoding") {
                                    0 => u32::from(reader.read_u8().expect("node id")),
                                    1 => {
                                        reader.read_u8().expect("namespace");
                                        u32::from(reader.read_u16(little).expect("node id"))
                                    }
                                    2 => {
                                        reader.read_u16(little).expect("namespace");
                                        reader.read_u32(little).expect("node id")
                                    }
                                    _ => panic!("fixture uses numeric nodes"),
                                };
                                let attribute = reader.read_u32(little).expect("attribute");
                                for prefix in 0..2 {
                                    if prefix == 1 {
                                        reader.read_u16(little).expect("encoding namespace");
                                    }
                                    let length = reader.read_i32(little).expect("string length");
                                    if length >= 0 {
                                        reader
                                            .read_bytes(usize::try_from(length).unwrap(), 8192)
                                            .expect("string");
                                    }
                                }
                                if node == 65535 {
                                    values.push(
                                        [vec![2], 0x8034_0000_u32.to_le_bytes().to_vec()].concat(),
                                    );
                                } else {
                                    values.push(match attribute {
                                        14 => vec![1, 17, 0, 6],
                                        15 => {
                                            [vec![1, 6], (-1_i32).to_le_bytes().to_vec()].concat()
                                        }
                                        16 => vec![1, 0x87, 0, 0, 0, 0],
                                        17 => vec![1, 3, 3],
                                        13 => int32_value(
                                            device
                                                .0
                                                .lock()
                                                .unwrap()
                                                .values
                                                .get(&node)
                                                .copied()
                                                .unwrap_or_else(|| i32::try_from(node).unwrap()),
                                            0,
                                        ),
                                        _ => panic!("unexpected attribute"),
                                    });
                                }
                            }
                            reader.expect_eof().expect("end of read");
                            read_results(request, &values)
                        }
                    }
                }
                673 => {
                    reader
                        .read_bytes(29, 29)
                        .expect("request header with short token");
                    let count = reader.read_i32(little).expect("write count");

                    for _ in 0..count {
                        let node = match reader.read_u8().unwrap() {
                            0 => u32::from(reader.read_u8().unwrap()),
                            1 => {
                                reader.read_u8().unwrap();
                                u32::from(reader.read_u16(little).unwrap())
                            }
                            2 => {
                                reader.read_u16(little).unwrap();
                                reader.read_u32(little).unwrap()
                            }
                            _ => panic!("fixture uses numeric writes"),
                        };
                        assert_eq!(reader.read_u32(little).unwrap(), 13);
                        assert_eq!(reader.read_i32(little).unwrap(), -1);
                        assert_eq!(reader.read_u8().unwrap(), 1);
                        assert_eq!(reader.read_u8().unwrap(), 6);
                        let value = reader.read_i32(little).unwrap();
                        device.0.lock().unwrap().values.insert(node, value);
                    }
                    reader.expect_eof().unwrap();
                    let mut state = device.0.lock().unwrap();
                    if std::mem::take(&mut state.unknown) {
                        return Reaction::Silence;
                    }
                    drop(state);
                    write_response(request, &vec![0; usize::try_from(count).unwrap()])
                }
                _ => panic!("unexpected service {service}"),
            };
            sequence = sequence.checked_add(1).expect("sequence");
            Reaction::Reply(message(sequence, request, 17, 29, &body))
        }
    }
}

pub(super) fn write_response(handle: u32, statuses: &[u32]) -> Vec<u8> {
    [
        vec![1, 0, 0xA4, 2],
        response_header(handle, 0),
        i32::try_from(statuses.len())
            .unwrap()
            .to_le_bytes()
            .to_vec(),
        statuses
            .iter()
            .flat_map(|status| status.to_le_bytes())
            .collect(),
        vec![0xFF; 4],
    ]
    .concat()
}
