//! A first-party Modbus TCP device for the contract run: it answers function codes 01 to 04
//! from the interop register map, byte for byte over the MBAP layout.
//!
//! This is a test harness, not a double of a Stonecrab trait: it is the peer's side of the
//! protocol, built from this crate's own codec (`decode_request`, `encode_response`,
//! `MbapHeader`), and it plugs into the testkit's `Step::Respond`. Nothing here is copied
//! from an upstream test or fixture; the values come from
//! `fixtures/modbus/interop/register-map.toml`.
//!
//! The device is steered by the contract fixture's hooks through [`Device::arm_swallow`]
//! (the next request gets no answer, so the fake clock drives the deadline) and
//! [`Device::arm_exception`] (the next request is refused with a device exception). A
//! `Step::Respond` closure cannot end the script, so the device cannot close the peer on
//! demand; see `tests/contract.rs` for what that means for the disconnect cases.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::{Bytes, BytesMut};
use stonecrab_codec::{BoundedReader, BoundedWriter, pack_bits_lsb_first};
use stonecrab_modbus::{
    ExceptionCode, FunctionCode, MAX_PDU_BYTES, MbapHeader, ModbusAdu, ModbusPdu, decode_request,
    encode_response,
};
use stonecrab_testkit::{InteropMap, RegisterEntry, Responder};

/// What the device does to the next request instead of answering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Stay silent.
    Swallow,
    /// Answer with a device exception carrying this code.
    Exception(ExceptionCode),
}

/// The four data blocks, indexed by logical address (wire address plus one), the way the
/// interop server stores them. A position no entry names holds zero or false.
#[derive(Debug)]
struct Blocks {
    holding: Vec<u16>,
    input: Vec<u16>,
    coils: Vec<bool>,
    discrete: Vec<bool>,
}

/// State shared by every responder of one device and by the fixture hooks.
#[derive(Debug, Default)]
struct Shared {
    /// Requests that reached the device, answered or not.
    reads: AtomicU32,
    faults: Mutex<VecDeque<Fault>>,
}

/// A simulated Modbus device: the register map and the faults armed against it.
///
/// Clones share one counter and one fault queue, so the fixture hooks, the counters and
/// every responder handed to a fake transport see the same device.
#[derive(Debug, Clone)]
pub struct Device {
    blocks: Arc<Blocks>,
    shared: Arc<Shared>,
}

/// Lay `entries` into `block` at their logical addresses.
fn place(block: &mut [u16], entries: &[RegisterEntry]) {
    for entry in entries {
        for (index, word) in entry.words.iter().enumerate() {
            let slot = usize::from(entry.address).saturating_add(index);
            if let Some(register) = block.get_mut(slot) {
                *register = *word;
            }
        }
    }
}

/// Lay a run of bits into `block` at its logical address.
fn place_bits(block: &mut [bool], address: u16, values: &[bool]) {
    for (index, value) in values.iter().enumerate() {
        let slot = usize::from(address).saturating_add(index);
        if let Some(bit) = block.get_mut(slot) {
            *bit = *value;
        }
    }
}

impl Device {
    /// A device serving `map` with the block sizes it declares. The holding block is
    /// widened to cover the little-endian extension, as the interop server's is.
    pub fn from_map(map: &InteropMap) -> Self {
        let extension_end = map
            .holding_extension
            .iter()
            .map(|entry| usize::from(entry.address).saturating_add(entry.words.len()))
            .max()
            .unwrap_or(0);
        let mut holding = vec![0; map.sizes.holding_registers.max(extension_end)];
        place(&mut holding, &map.holding_registers);
        place(&mut holding, &map.holding_extension);
        let mut input = vec![0; map.sizes.input_registers];
        place(&mut input, &map.input_registers);
        let mut coils = vec![false; map.sizes.coils];
        place_bits(&mut coils, map.coils.address, &map.coils.values);
        let mut discrete = vec![false; map.sizes.discrete_inputs];
        place_bits(
            &mut discrete,
            map.discrete_inputs.address,
            &map.discrete_inputs.values,
        );
        Self {
            blocks: Arc::new(Blocks {
                holding,
                input,
                coils,
                discrete,
            }),
            shared: Arc::new(Shared::default()),
        }
    }

    /// How many requests reached the device so far, answered or swallowed.
    pub fn reads(&self) -> u32 {
        self.shared.reads.load(Ordering::Relaxed)
    }

    /// Faults armed and not yet used.
    pub fn armed(&self) -> usize {
        self.with_faults(|faults| faults.len())
    }

    /// Leave the next request unanswered.
    pub fn arm_swallow(&self) {
        self.with_faults(|faults| faults.push_back(Fault::Swallow));
    }

    /// Refuse the next request with the exception `code`.
    pub fn arm_exception(&self, code: ExceptionCode) {
        self.with_faults(|faults| faults.push_back(Fault::Exception(code)));
    }

    fn with_faults<R>(&self, f: impl FnOnce(&mut VecDeque<Fault>) -> R) -> R {
        let mut faults = self
            .shared
            .faults
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(&mut faults)
    }

    /// A responder for one fake transport: the bytes of one request in, the bytes of its
    /// reply out, or `None` to stay silent.
    pub fn responder(&self) -> Responder {
        let device = self.clone();
        Box::new(move |request| device.answer(request))
    }

    /// The reply to one request ADU, or `None` for bytes that are not a request or for a
    /// swallowed request.
    fn answer(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let header = MbapHeader::decode(bytes).ok()?;
        let pdu = bytes.get(MbapHeader::ENCODED_LEN..header.adu_len())?;
        self.shared.reads.fetch_add(1, Ordering::Relaxed);
        let fault = self.with_faults(VecDeque::pop_front);
        let reply = match fault {
            Some(Fault::Swallow) => return None,
            Some(Fault::Exception(code)) => refusal(pdu.first().copied()?, code),
            None => self.serve(pdu)?,
        };
        wire(header, &reply)
    }

    /// The reply PDU bytes for a request PDU.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "ModbusPdu is non_exhaustive, so a match outside its crate needs a wildcard"
    )]
    fn serve(&self, pdu: &[u8]) -> Option<Vec<u8>> {
        let first = pdu.first().copied()?;
        let Ok(request) = decode_request(&mut BoundedReader::new(pdu)) else {
            // A function this device does not serve is ILLEGAL FUNCTION; a known function
            // with a quantity outside its bounds is ILLEGAL DATA VALUE.
            let known = (1..=4).contains(&first);
            let code = if known {
                ExceptionCode::IllegalDataValue
            } else {
                ExceptionCode::IllegalFunction
            };
            return Some(refusal(first, code));
        };
        let response = match request {
            ModbusPdu::ReadHoldingRegisters { start, quantity } => {
                registers(&self.blocks.holding, request.function(), start, quantity)
            }
            ModbusPdu::ReadInputRegisters { start, quantity } => {
                registers(&self.blocks.input, request.function(), start, quantity)
            }
            ModbusPdu::ReadCoils { start, quantity } => {
                bits(&self.blocks.coils, request.function(), start, quantity)
            }
            ModbusPdu::ReadDiscreteInputs { start, quantity } => {
                bits(&self.blocks.discrete, request.function(), start, quantity)
            }
            // A response or an exception is not a request (and the enum is non-exhaustive).
            _ => return Some(refusal(first, ExceptionCode::IllegalFunction)),
        };
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_response(&response, &mut writer).ok()?;
        Some(writer.into_bytes())
    }
}

/// The reply to a register read: the registers, or ILLEGAL DATA ADDRESS.
fn registers(block: &[u16], function: FunctionCode, start: u16, quantity: u16) -> ModbusPdu {
    match span(block.len(), start, quantity).and_then(|range| block.get(range)) {
        Some(registers) => ModbusPdu::ReadRegistersResponse {
            function,
            registers: registers.to_vec(),
        },
        None => ModbusPdu::Exception {
            function,
            code: ExceptionCode::IllegalDataAddress,
        },
    }
}

/// The reply to a bit read: the packed bits, or ILLEGAL DATA ADDRESS.
fn bits(block: &[bool], function: FunctionCode, start: u16, quantity: u16) -> ModbusPdu {
    match span(block.len(), start, quantity).and_then(|range| block.get(range)) {
        Some(bits) => ModbusPdu::ReadBitsResponse {
            function,
            bytes: pack_bits_lsb_first(bits),
        },
        None => ModbusPdu::Exception {
            function,
            code: ExceptionCode::IllegalDataAddress,
        },
    }
}

/// The index range of a read of `quantity` items at wire address `start` in a block of
/// `len` items indexed by logical address (wire address plus one); `None` if it leaves
/// the block.
fn span(len: usize, start: u16, quantity: u16) -> Option<std::ops::Range<usize>> {
    let first = usize::from(start).checked_add(1)?;
    let end = first.checked_add(usize::from(quantity))?;
    (end <= len).then_some(first..end)
}

/// The exception PDU for `function`: the function with its high bit set, then the code.
fn refusal(function: u8, code: ExceptionCode) -> Vec<u8> {
    vec![function | 0x80, code.as_u8()]
}

/// The reply ADU: the request's transaction and unit ids around the reply PDU.
fn wire(request: MbapHeader, pdu: &[u8]) -> Option<Vec<u8>> {
    let adu = ModbusAdu::new(
        request.transaction_id,
        request.unit_id,
        Bytes::copy_from_slice(pdu),
    )
    .ok()?;
    let mut out = BytesMut::new();
    adu.header().encode(&mut out).ok()?;
    out.extend_from_slice(adu.pdu());
    Some(out.to_vec())
}
