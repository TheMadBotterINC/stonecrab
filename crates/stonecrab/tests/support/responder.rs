//! A first-party Modbus TCP device for the contract run: it answers function codes 01 to 06,
//! 15 and 16 from the interop register map, and the file record codes 20 and 21 from a
//! sparse store of extended registers, byte for byte over the MBAP layout. A write changes
//! the block it names, so a later read sees it.
//!
//! This is a test harness, not a double of a Stonecrab trait: it is the peer's side of the
//! protocol, built from this crate's own codec (`decode_request`, `encode_response`,
//! `MbapHeader`), and it plugs into the testkit's `Step::React`. Nothing here is copied
//! from an upstream test or fixture; the values come from
//! `fixtures/modbus/interop/register-map.toml`.
//!
//! The device is steered by the contract fixture's hooks through [`Device::arm_swallow`]
//! (the next request gets no answer, so the fake clock drives the deadline),
//! [`Device::arm_exception`] (the next request is refused with a device exception) and
//! [`Device::arm_disconnect`] (the next request makes the peer close the connection
//! without answering, a lost link). [`Device::arm_apply_then_swallow`] and
//! [`Device::arm_apply_then_disconnect`] are the same two faults for a write that the device
//! applies first: the confirmation is lost, not the write.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use bytes::{Bytes, BytesMut};
use stonecrab::codec::{BoundedReader, BoundedWriter, pack_bits_lsb_first, unpack_bits_lsb_first};
use stonecrab::modbus::{
    ExceptionCode, FileRecord, FunctionCode, MAX_PDU_BYTES, MbapHeader, ModbusAdu, ModbusPdu,
    decode_request, encode_response,
};
use stonecrab::testkit::{InteropMap, Reaction, Reactor, RegisterEntry};

/// What the device does to the next request instead of answering it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// Stay silent.
    Swallow,
    /// Answer with a device exception carrying this code.
    Exception(ExceptionCode),
    /// Close the connection without answering.
    Disconnect,
    /// Serve the request (a write is applied), then stay silent.
    ApplyThenSwallow,
    /// Serve the request (a write is applied), then close the connection.
    ApplyThenDisconnect,
}

/// The four data blocks, indexed by logical address (wire address plus one), the way the
/// interop server stores them. A position no entry names holds zero or false.
#[derive(Debug)]
struct Blocks {
    holding: Vec<u16>,
    input: Vec<u16>,
    coils: Vec<bool>,
    discrete: Vec<bool>,
    /// The extended registers by register address, `(file - 1) * 10000 + record`; one that
    /// nothing was written to holds zero, in any file up to `files`.
    extended: BTreeMap<u32, u16>,
    /// The number of files the device has: a sub-request of a file above it is ILLEGAL
    /// DATA ADDRESS.
    files: u16,
}

/// Registers in one file, record numbers 0 to 9999.
const FILE_REGISTERS: u32 = 10_000;

/// The register address of `record` in `file` (files count from 1), `None` for file 0.
fn extended_address(file: u16, record: u16) -> Option<u32> {
    let before = u32::from(file.checked_sub(1)?);
    before
        .checked_mul(FILE_REGISTERS)?
        .checked_add(u32::from(record))
}

/// State shared by every responder of one device and by the fixture hooks.
#[derive(Debug, Default)]
struct Shared {
    /// Requests that reached the device, answered or not.
    reads: AtomicU32,
    /// Writes the device applied to its blocks, whether or not the confirmation was sent.
    applied: AtomicU32,
    faults: Mutex<VecDeque<Fault>>,
}

/// A simulated Modbus device: the register map and the faults armed against it.
///
/// Clones share one counter and one fault queue, so the fixture hooks, the counters and
/// every responder handed to a fake transport see the same device.
#[derive(Debug, Clone)]
pub struct Device {
    blocks: Arc<Mutex<Blocks>>,
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
            blocks: Arc::new(Mutex::new(Blocks {
                holding,
                input,
                coils,
                discrete,
                extended: BTreeMap::new(),
                files: u16::MAX,
            })),
            shared: Arc::new(Shared::default()),
        }
    }

    /// How many requests (reads and writes) reached the device so far, answered or
    /// swallowed.
    pub fn reads(&self) -> u32 {
        self.shared.reads.load(Ordering::Relaxed)
    }

    /// How many writes the device applied so far, confirmed to the client or not.
    pub fn applied(&self) -> u32 {
        self.shared.applied.load(Ordering::Relaxed)
    }

    /// The holding register at a logical address (wire address plus one).
    pub fn holding(&self, logical: usize) -> Option<u16> {
        self.with_blocks(|blocks| blocks.holding.get(logical).copied())
    }

    /// The coil at a logical address (wire address plus one).
    pub fn coil(&self, logical: usize) -> Option<bool> {
        self.with_blocks(|blocks| blocks.coils.get(logical).copied())
    }

    /// The extended register at `address` (the register address counted through all the
    /// files, 10000 to a file; address 0 is record 0 of file 1). A register nothing was
    /// written to is zero.
    pub fn extended(&self, address: u32) -> u16 {
        self.with_blocks(|blocks| blocks.extended.get(&address).copied().unwrap_or(0))
    }

    /// Put `values` into the extended registers from `address`, as a device's own program
    /// would.
    pub fn put_extended(&self, address: u32, values: &[u16]) {
        self.with_blocks(|blocks| {
            for (register, value) in (address..).zip(values) {
                blocks.extended.insert(register, *value);
            }
        });
    }

    /// Limit the device to `files` files: a sub-request of a higher file is ILLEGAL DATA
    /// ADDRESS. A device has all 65535 files until this is called.
    pub fn limit_extended_files(&self, files: u16) {
        self.with_blocks(|blocks| blocks.files = files);
    }

    fn with_blocks<R>(&self, f: impl FnOnce(&mut Blocks) -> R) -> R {
        let mut blocks = self.blocks.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut blocks)
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

    /// Close the connection on the next request, without answering it.
    pub fn arm_disconnect(&self) {
        self.with_faults(|faults| faults.push_back(Fault::Disconnect));
    }

    /// Serve the next request (applying it, if it is a write) and leave it unanswered.
    pub fn arm_apply_then_swallow(&self) {
        self.with_faults(|faults| faults.push_back(Fault::ApplyThenSwallow));
    }

    /// Serve the next request (applying it, if it is a write) and close the connection
    /// without answering it.
    pub fn arm_apply_then_disconnect(&self) {
        self.with_faults(|faults| faults.push_back(Fault::ApplyThenDisconnect));
    }

    fn with_faults<R>(&self, f: impl FnOnce(&mut VecDeque<Fault>) -> R) -> R {
        let mut faults = self
            .shared
            .faults
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        f(&mut faults)
    }

    /// A reactor for one fake transport: the bytes of one request in, what the peer does
    /// about it out (reply, stay silent or close the connection).
    pub fn reactor(&self) -> Reactor {
        let device = self.clone();
        Box::new(move |request| device.react(request))
    }

    /// What the peer does about one request ADU: answer it, stay silent for bytes that are
    /// not a request or for a swallowed request, or close the connection for a disconnect.
    fn react(&self, bytes: &[u8]) -> Reaction {
        let Ok(header) = MbapHeader::decode(bytes) else {
            return Reaction::Silence;
        };
        let Some(pdu) = bytes.get(MbapHeader::ENCODED_LEN..header.adu_len()) else {
            return Reaction::Silence;
        };
        self.shared.reads.fetch_add(1, Ordering::Relaxed);
        let fault = self.with_faults(VecDeque::pop_front);
        let reply = match fault {
            Some(Fault::Swallow) => return Reaction::Silence,
            Some(Fault::Disconnect) => return Reaction::Close,
            Some(Fault::Exception(code)) => {
                pdu.first().copied().map(|function| refusal(function, code))
            }
            Some(Fault::ApplyThenSwallow) => {
                drop(self.serve(pdu));
                return Reaction::Silence;
            }
            Some(Fault::ApplyThenDisconnect) => {
                drop(self.serve(pdu));
                return Reaction::Close;
            }
            None => self.serve(pdu),
        };
        reply
            .and_then(|reply| wire(header, &reply))
            .map_or(Reaction::Silence, Reaction::Reply)
    }

    /// The reply PDU bytes for a request PDU.
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "ModbusPdu is non_exhaustive, so a match outside its crate needs a wildcard"
    )]
    fn serve(&self, pdu: &[u8]) -> Option<Vec<u8>> {
        let first = pdu.first().copied()?;
        // The codec writes function 05 and never parses it as a request: read it here.
        let decoded = if first == 5 {
            single_coil(pdu)
        } else {
            decode_request(&mut BoundedReader::new(pdu)).ok()
        };
        let Some(request) = decoded else {
            // A function this device does not serve is ILLEGAL FUNCTION; a known function
            // with a quantity outside its bounds is ILLEGAL DATA VALUE.
            let known = matches!(first, 1..=6 | 15 | 16 | 20 | 21);
            let code = if known {
                ExceptionCode::IllegalDataValue
            } else {
                ExceptionCode::IllegalFunction
            };
            return Some(refusal(first, code));
        };
        let function = request.function();
        let response = self.with_blocks(|blocks| match &request {
            ModbusPdu::ReadHoldingRegisters { start, quantity } => {
                registers(&blocks.holding, function, *start, *quantity)
            }
            ModbusPdu::ReadInputRegisters { start, quantity } => {
                registers(&blocks.input, function, *start, *quantity)
            }
            ModbusPdu::ReadCoils { start, quantity } => {
                bits(&blocks.coils, function, *start, *quantity)
            }
            ModbusPdu::ReadDiscreteInputs { start, quantity } => {
                bits(&blocks.discrete, function, *start, *quantity)
            }
            ModbusPdu::WriteSingleRegister { address, value } => {
                if store(&mut blocks.holding, *address, &[*value]) {
                    self.shared.applied.fetch_add(1, Ordering::Relaxed);
                    ModbusPdu::WriteSingleRegisterResponse {
                        address: *address,
                        value: *value,
                    }
                } else {
                    unaddressable(function)
                }
            }
            ModbusPdu::WriteSingleCoil { address, value } => {
                if store(&mut blocks.coils, *address, &[*value]) {
                    self.shared.applied.fetch_add(1, Ordering::Relaxed);
                    ModbusPdu::WriteSingleCoilResponse {
                        address: *address,
                        value: *value,
                    }
                } else {
                    unaddressable(function)
                }
            }
            ModbusPdu::WriteMultipleRegisters { start, registers } => {
                if store(&mut blocks.holding, *start, registers) {
                    self.shared.applied.fetch_add(1, Ordering::Relaxed);
                    ModbusPdu::WriteMultipleResponse {
                        function,
                        start: *start,
                        quantity: u16::try_from(registers.len()).unwrap_or(u16::MAX),
                    }
                } else {
                    unaddressable(function)
                }
            }
            ModbusPdu::WriteMultipleCoils {
                start,
                quantity,
                bytes,
            } => match unpack_bits_lsb_first(bytes, usize::from(*quantity)) {
                Ok(values) if store(&mut blocks.coils, *start, &values) => {
                    self.shared.applied.fetch_add(1, Ordering::Relaxed);
                    ModbusPdu::WriteMultipleResponse {
                        function,
                        start: *start,
                        quantity: *quantity,
                    }
                }
                _ => unaddressable(function),
            },
            ModbusPdu::ReadFileRecord { records } => {
                let runs: Option<Vec<Vec<u16>>> = records
                    .iter()
                    .map(|item| {
                        let first = extended_address(item.file, item.record)?;
                        (item.file <= blocks.files).then_some(())?;
                        Some(
                            (first..)
                                .take(usize::from(item.length))
                                .map(|register| {
                                    blocks.extended.get(&register).copied().unwrap_or(0)
                                })
                                .collect(),
                        )
                    })
                    .collect();
                runs.map_or_else(
                    || unaddressable(function),
                    |records| ModbusPdu::ReadFileRecordResponse { records },
                )
            }
            ModbusPdu::WriteFileRecord { records } => {
                if stores(blocks, records) {
                    self.shared.applied.fetch_add(1, Ordering::Relaxed);
                    ModbusPdu::WriteFileRecordResponse {
                        records: records.clone(),
                    }
                } else {
                    unaddressable(function)
                }
            }
            // A response or an exception is not a request (and the enum is non-exhaustive).
            _ => refused(function, ExceptionCode::IllegalFunction),
        });
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_response(&response, &mut writer).ok()?;
        Some(writer.into_bytes())
    }
}

/// The Write Single Coil request of five PDU bytes: the address, then `0xFF00` for on or
/// `0x0000` for off; any other state is not a request.
fn single_coil(pdu: &[u8]) -> Option<ModbusPdu> {
    let [_, address_high, address_low, state_high, state_low] = *<&[u8; 5]>::try_from(pdu).ok()?;
    let value = match u16::from_be_bytes([state_high, state_low]) {
        0xFF00 => true,
        0 => false,
        _ => return None,
    };
    Some(ModbusPdu::WriteSingleCoil {
        address: u16::from_be_bytes([address_high, address_low]),
        value,
    })
}

/// Put the sub-requests of a Write File Record into the extended registers; false, with
/// nothing changed, if one names a file the device does not have.
fn stores(blocks: &mut Blocks, records: &[FileRecord]) -> bool {
    let placed: Option<Vec<(u32, &[u16])>> = records
        .iter()
        .map(|item| {
            (item.file <= blocks.files).then_some(())?;
            Some((
                extended_address(item.file, item.record)?,
                item.registers.as_slice(),
            ))
        })
        .collect();
    let Some(placed) = placed else {
        return false;
    };
    for (first, registers) in placed {
        for (register, value) in (first..).zip(registers) {
            blocks.extended.insert(register, *value);
        }
    }
    true
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

/// Put `values` into `block` from wire address `start` (the block is indexed by logical
/// address); false, with `block` unchanged, if they do not fit.
fn store<T: Copy>(block: &mut [T], start: u16, values: &[T]) -> bool {
    let Some(range) = u16::try_from(values.len())
        .ok()
        .and_then(|quantity| span(block.len(), start, quantity))
    else {
        return false;
    };
    for (slot, value) in block.iter_mut().skip(range.start).zip(values) {
        *slot = *value;
    }
    true
}

/// ILLEGAL DATA ADDRESS for `function`, as a response PDU.
const fn unaddressable(function: FunctionCode) -> ModbusPdu {
    refused(function, ExceptionCode::IllegalDataAddress)
}

/// The exception PDU `code` for `function`, as a response PDU.
const fn refused(function: FunctionCode, code: ExceptionCode) -> ModbusPdu {
    ModbusPdu::Exception { function, code }
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
