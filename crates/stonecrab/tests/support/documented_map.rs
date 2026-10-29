//! Documented device maps (`fixtures/modbus/devices/*.toml`, slice S52d) served through the
//! first-party responder: the loader of a map, the device limits the vendor documents state,
//! and the rig that connects the driver of the map's scheme (`ModbusTcpDriver` for
//! `modbus-tcp`, `ModbusRtuDriver` for `modbus-rtu`) to the replay over the fake transport.
//!
//! A documented map is a transcription of a vendor's register tables. Serving it proves what
//! Stonecrab does with those tables and nothing about the device: the replay is
//! "documented map, not hardware-verified". The responder is the one of
//! `tests/support/responder.rs`; this module adds the three things a device has that the
//! interop map does not: its own register words, a refusal of requests the vendor's limits
//! forbid, and a refusal of function codes the vendor does not list. The refusal codes are
//! assumptions, because the documents name none (`ILLEGAL DATA VALUE` for a quantity over a
//! limit, `ILLEGAL FUNCTION` for an unlisted function), and the hardware runbooks list them
//! as open items.
//!
//! A serial map is served with RTU framing: the adapter around the responder checks the CRC
//! of each request and puts one on each reply, with a CRC-16 written here bit by bit from the
//! Modbus over Serial Line specification and not taken from the crate, so a driver and a
//! codec that agreed on a wrong CRC would not pass.

#![expect(
    unreachable_pub,
    reason = "plain pub in a privately included module: pub(crate) would trip redundant_pub_crate"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use stonecrab::modbus::{ExceptionCode, ModbusRtuDriver, ModbusTcpDriver};
use stonecrab::testkit::{
    BitRun, BlockSizes, FakeClock, FakeTransportFactory, InteropMap, Reaction, Reactor,
    RegisterEntry, Script, Step, block_on_with,
};
use stonecrab::{
    Connection, Driver, PlcValue, ReadRequest, ReadResponse, Tag, TagId, WriteRequest,
    WriteResponse,
};

use crate::responder::Device;

/// The length of the MBAP header, so the PDU of a request ADU starts here.
const MBAP_LEN: usize = 7;

/// The path of the serial port a serial map is reached on; the fake transport ignores it.
const SERIAL_PORT: &str = "/dev/ttyUSB0";

/// Transports the factory can hand out: a test connects a handful of times.
const PLANS: usize = 16;

/// A documented device map: what the vendor's document says, entry by entry.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DocumentedMap {
    pub device: DeviceInfo,
    pub source: Source,
    pub limits: Limits,
    /// The process variable error codes (SOLO only).
    #[serde(default)]
    pub sentinels: Vec<Sentinel>,
    #[serde(default)]
    pub coils: Vec<Coil>,
    pub registers: Vec<Register>,
}

/// The `[device]` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInfo {
    pub label: String,
    pub manufacturer: String,
    pub model: String,
    /// The scheme the device is reached by: `modbus-tcp`, or `modbus-rtu` for a serial device.
    pub scheme: String,
    pub unit_id: u8,
    pub address_reading: String,
    pub input_registers_mirror_holding: bool,
    /// The line settings of a serial device, as the replay connects to it.
    #[serde(default)]
    pub serial: Option<Serial>,
}

/// The `[device.serial]` table: the line the device is reached on.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Serial {
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: u8,
}

impl Serial {
    /// The settings as the `serial.` keys of a connection string.
    pub fn options(&self) -> Vec<String> {
        vec![
            format!("serial.baud-rate={}", self.baud_rate),
            format!("serial.data-bits={}", self.data_bits),
            format!("serial.parity={}", self.parity),
            format!("serial.stop-bits={}", self.stop_bits),
        ]
    }
}

/// The `[source]` table: the vendor document the map is transcribed from.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub document: String,
    pub url: String,
    pub sha256: String,
    pub retrieved: String,
    pub license: String,
}

/// The `[limits]` table: the function codes and request sizes the vendor documents.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub functions: Vec<u8>,
    pub read_bits: Option<u16>,
    pub read_registers: Option<u16>,
    pub write_bits: Option<u16>,
    pub write_registers: Option<u16>,
}

/// One error code the process variable can read as, instead of a temperature.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sentinel {
    pub word: u16,
    pub meaning: String,
}

/// One coil of the map.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Coil {
    pub number: String,
    pub wire: u16,
    pub name: String,
    pub access: String,
    pub basis: String,
    pub value: bool,
    pub meaning: String,
}

/// One entry of the register table: one register, or a run of registers of one type.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub number: String,
    pub wire: u16,
    pub name: String,
    #[serde(rename = "type")]
    pub data_type: String,
    pub access: String,
    pub basis: String,
    pub words: Vec<u16>,
    pub value: Option<String>,
    pub meaning: String,
}

/// The value a register word reads as under a documented type.
pub fn word_value(data_type: &str, word: u16) -> Option<PlcValue> {
    match data_type {
        "INT" => Some(PlcValue::I16(i16::from_be_bytes(word.to_be_bytes()))),
        "UINT" | "WORD" => Some(PlcValue::U16(word)),
        _ => None,
    }
}

impl DocumentedMap {
    /// The map in `fixtures/modbus/devices/<file>`.
    pub fn load(file: &str) -> Self {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/devices")
            .join(file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{file} is readable: {error}"));
        toml::from_str(&text).unwrap_or_else(|error| panic!("{file} is a documented map: {error}"))
    }

    /// Every documented register word with its wire address and its entry, in table order.
    fn entries(&self) -> impl Iterator<Item = (u16, &Register, u16)> {
        self.registers.iter().flat_map(|register| {
            register
                .words
                .iter()
                .enumerate()
                .map(move |(offset, word)| {
                    let offset = u16::try_from(offset).unwrap_or(u16::MAX);
                    (register.wire.saturating_add(offset), register, *word)
                })
        })
    }

    /// Every documented register word at its wire address, one word per register.
    pub fn words(&self) -> BTreeMap<u16, u16> {
        self.entries().map(|(wire, _, word)| (wire, word)).collect()
    }

    /// The documented registers as typed reads: the wire address, the documented type and
    /// the value the transcribed word reads as.
    pub fn typed_words(&self) -> Vec<(u16, &str, PlcValue)> {
        self.entries()
            .filter_map(|(wire, register, word)| {
                let value = word_value(&register.data_type, word)?;
                Some((wire, register.data_type.as_str(), value))
            })
            .collect()
    }

    /// The map as the responder's register map: each documented word at its logical address
    /// (the wire address plus one), `overrides` replacing words, and the coils laid out in
    /// one run from the first to the last documented coil (a hole in the run is false).
    pub fn interop_map(&self, overrides: &[(u16, u16)]) -> InteropMap {
        let mut words = self.words();
        for (wire, word) in overrides {
            words.insert(*wire, *word);
        }
        let registers: Vec<RegisterEntry> = words
            .iter()
            .map(|(wire, word)| RegisterEntry {
                row: self.device.label.clone(),
                address: wire.saturating_add(1),
                data_type: "UINT".to_owned(),
                value: String::new(),
                byte_order: None,
                words: vec![*word],
            })
            .collect();
        let size =
            |highest: Option<&u16>| usize::from(highest.copied().unwrap_or(0)).saturating_add(2);
        let first_coil = self.coils.iter().map(|coil| coil.wire).min().unwrap_or(0);
        let last_coil = self.coils.iter().map(|coil| coil.wire).max().unwrap_or(0);
        let run = BitRun {
            row: self.device.label.clone(),
            address: first_coil.saturating_add(1),
            values: (first_coil..=last_coil)
                .map(|wire| {
                    self.coils
                        .iter()
                        .any(|coil| coil.wire == wire && coil.value)
                })
                .collect(),
        };
        let holding_size = size(words.keys().next_back());
        let mirrored = self.device.input_registers_mirror_holding;
        InteropMap {
            sizes: BlockSizes {
                holding_registers: holding_size,
                coils: size(Some(&last_coil)),
                discrete_inputs: size(Some(&last_coil)),
                input_registers: if mirrored { holding_size } else { 2 },
            },
            input_registers: if mirrored {
                registers.clone()
            } else {
                Vec::new()
            },
            holding_registers: registers,
            holding_extension: Vec::new(),
            coils: run.clone(),
            discrete_inputs: run,
        }
    }

    /// The responder serving the map, with `overrides` replacing documented words.
    pub fn device(&self, overrides: &[(u16, u16)]) -> Device {
        Device::from_map(&self.interop_map(overrides))
    }
}

impl Limits {
    /// A reactor that refuses what the vendor's document forbids and hands every other
    /// request to `inner`.
    pub fn guard(&self, mut inner: Reactor) -> Reactor {
        let limits = self.clone();
        Box::new(move |request| match limits.refusal(request) {
            Some(reply) => Reaction::Reply(reply),
            None => inner(request),
        })
    }

    /// The exception reply to a request ADU, or `None` when the document allows it.
    fn refusal(&self, request: &[u8]) -> Option<Vec<u8>> {
        let function = *request.get(MBAP_LEN)?;
        let code = if self.functions.contains(&function) {
            let cap = match function {
                1 | 2 => self.read_bits,
                3 | 4 => self.read_registers,
                15 => self.write_bits,
                16 => self.write_registers,
                _ => None,
            }?;
            // The quantity of a read or a multiple write follows the start address.
            let quantity = u16::from_be_bytes(*request.get(10..12)?.first_chunk()?);
            if quantity <= cap {
                return None;
            }
            ExceptionCode::IllegalDataValue
        } else {
            ExceptionCode::IllegalFunction
        };
        let mut reply = request.get(..2)?.to_vec();
        reply.extend_from_slice(&[0, 0, 0, 3, *request.get(6)?, function | 0x80, code.as_u8()]);
        Some(reply)
    }
}

/// CRC-16/MODBUS, bit by bit: reflected polynomial 0xA001, initial value 0xFFFF.
fn crc(bytes: &[u8]) -> [u8; 2] {
    let mut register: u16 = 0xFFFF;
    for byte in bytes {
        register ^= u16::from(*byte);
        for _ in 0..8 {
            let carry = register & 0x0001 != 0;
            register >>= 1;
            if carry {
                register ^= 0xA001;
            }
        }
    }
    register.to_le_bytes()
}

/// An RTU frame: the address, the PDU, the CRC low byte first.
pub fn rtu_frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut bytes = vec![unit];
    bytes.extend_from_slice(pdu);
    let check = crc(&bytes);
    bytes.extend_from_slice(&check);
    bytes
}

/// The unit and PDU of an RTU frame, `None` when the CRC does not match.
fn rtu_parts(frame: &[u8]) -> Option<(u8, &[u8])> {
    let body = frame.get(..frame.len().checked_sub(2)?)?;
    if crc(body).as_slice() != frame.get(body.len()..)? {
        return None;
    }
    body.split_first().map(|(unit, pdu)| (*unit, pdu))
}

/// An MBAP ADU of transaction 1 for `unit`.
fn mbap_adu(unit: u8, pdu: &[u8]) -> Vec<u8> {
    let length = u16::try_from(pdu.len().saturating_add(1)).unwrap_or(u16::MAX);
    let mut bytes = vec![0, 1, 0, 0];
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.push(unit);
    bytes.extend_from_slice(pdu);
    bytes
}

/// The unit and PDU of an MBAP ADU.
fn mbap_parts(adu: &[u8]) -> Option<(u8, &[u8])> {
    Some((*adu.get(MBAP_LEN.saturating_sub(1))?, adu.get(MBAP_LEN..)?))
}

/// The device behind RTU framing: the CRC of a request is checked and its unit and PDU go to
/// `inner` as an MBAP ADU; what `inner` answers comes back as an RTU frame.
fn rtu_reactor(device_unit: u8, mut inner: Reactor) -> Reactor {
    Box::new(move |bytes| {
        let Some((unit, pdu)) = rtu_parts(bytes) else {
            return Reaction::Silence;
        };
        if unit != device_unit {
            // A slave on a serial line answers only its own network address.
            return Reaction::Silence;
        }
        let wrap = |reply: Vec<u8>| {
            let (unit, pdu) = mbap_parts(&reply).unwrap_or_default();
            rtu_frame(unit, pdu)
        };
        match inner(&mbap_adu(unit, pdu)) {
            Reaction::Reply(reply) => Reaction::Reply(wrap(reply)),
            Reaction::ReplyThenClose(reply) => Reaction::ReplyThenClose(wrap(reply)),
            other @ (Reaction::Silence | Reaction::Close) => other,
        }
    })
}

/// How a request is framed on the line of a map's scheme.
#[derive(Debug, Clone, Copy)]
enum Wire {
    Tcp,
    Rtu,
}

impl Wire {
    /// The request ADU carrying `pdu` to `unit`.
    fn request(self, unit: u8, pdu: &[u8]) -> Vec<u8> {
        match self {
            Self::Tcp => mbap_adu(unit, pdu),
            Self::Rtu => rtu_frame(unit, pdu),
        }
    }

    /// The PDU of a frame on this wire.
    fn pdu(self, frame: &[u8]) -> Option<&[u8]> {
        match self {
            Self::Tcp => mbap_parts(frame),
            Self::Rtu => rtu_parts(frame),
        }
        .map(|(_, pdu)| pdu)
    }
}

/// A driver connected to the replay of a documented map over the fake transport and clock.
pub struct Rig {
    pub clock: Arc<FakeClock>,
    pub factory: FakeTransportFactory,
    pub device: Device,
    pub driver: Box<dyn Driver>,
    limits: Limits,
    uri: String,
    wire: Wire,
    unit: u8,
}

impl Rig {
    /// A rig for `map` with `overrides` replacing documented words. `query` is added to the
    /// connection string: the options a user of this device would set. A map reached by
    /// `modbus-rtu` is served with RTU framing on the line its `[device.serial]` table gives.
    pub fn new(map: &DocumentedMap, overrides: &[(u16, u16)], query: &str) -> Self {
        let clock = Arc::new(FakeClock::default());
        let device = map.device(overrides);
        let factory = FakeTransportFactory::new(FakeClock::clone(&clock));
        let wire = if map.device.scheme == "modbus-rtu" {
            Wire::Rtu
        } else {
            Wire::Tcp
        };
        for _ in 0..PLANS {
            factory.push_script(Script::new(vec![Step::React(Self::reactor(
                wire,
                map.device.unit_id,
                &map.limits,
                &device,
            ))]));
        }
        let mut options = vec![
            format!("address-convention={}", map.device.address_reading),
            format!("default-unit-identifier={}", map.device.unit_id),
        ];
        let (driver, prefix): (Box<dyn Driver>, String) = match wire {
            Wire::Tcp => (
                Box::new(
                    ModbusTcpDriver::new()
                        .with_clock(Arc::<FakeClock>::clone(&clock))
                        .with_transport_factory(Arc::new(factory.clone())),
                ),
                "modbus-tcp://127.0.0.1:502".to_owned(),
            ),
            Wire::Rtu => {
                let serial = map
                    .device
                    .serial
                    .as_ref()
                    .expect("a map with scheme modbus-rtu needs a [device.serial] table");
                options.extend(serial.options());
                (
                    Box::new(
                        ModbusRtuDriver::new()
                            .with_clock(Arc::<FakeClock>::clone(&clock))
                            .with_transport_factory(Arc::new(factory.clone())),
                    ),
                    format!("modbus-rtu://{SERIAL_PORT}"),
                )
            }
        };
        options.extend(
            query
                .split('&')
                .filter(|pair| !pair.is_empty())
                .map(str::to_owned),
        );
        Self {
            clock,
            factory,
            device,
            driver,
            limits: map.limits.clone(),
            uri: format!("{prefix}?{}", options.join("&")),
            wire,
            unit: map.device.unit_id,
        }
    }

    /// The device as the fake transport reaches it: the vendor's limits around the responder,
    /// behind the framing of `wire`.
    fn reactor(wire: Wire, unit: u8, limits: &Limits, device: &Device) -> Reactor {
        let guarded = limits.guard(device.reactor());
        match wire {
            Wire::Tcp => guarded,
            Wire::Rtu => rtu_reactor(unit, guarded),
        }
    }

    /// The device as a peer answers a request PDU, for a test that sends requests by hand: the
    /// request goes in a frame of the rig's wire to the same reactor the driver reaches, and
    /// the reply is the PDU of the frame that comes back (`None` for silence or a close).
    pub fn peer(&self) -> impl FnMut(&[u8]) -> Option<Vec<u8>> + use<> {
        let (wire, unit) = (self.wire, self.unit);
        let mut reactor = Self::reactor(wire, unit, &self.limits, &self.device);
        move |pdu| match reactor(&wire.request(unit, pdu)) {
            Reaction::Reply(frame) => wire.pdu(&frame).map(<[u8]>::to_vec),
            Reaction::ReplyThenClose(_) | Reaction::Silence | Reaction::Close => None,
        }
    }

    /// Whether the device answers `frame` as the line delivers it, byte for byte: a frame with
    /// a wrong CRC, or for another network address, gets silence from a serial slave.
    pub fn answers_frame(&self, frame: &[u8]) -> bool {
        let mut reactor = Self::reactor(self.wire, self.unit, &self.limits, &self.device);
        matches!(
            reactor(frame),
            Reaction::Reply(_) | Reaction::ReplyThenClose(_)
        )
    }

    /// Connect through the driver.
    pub fn connect(&self) -> Box<dyn Connection> {
        let config = self.driver.parse_config(&self.uri).expect("config");
        block_on_with(&self.clock, self.driver.connect(config)).expect("connect")
    }

    /// A tag named `id` at the address `text`, parsed under the rig's convention.
    pub fn tag(&self, id: &str, text: &str) -> Tag {
        let config = self.driver.parse_config(&self.uri).expect("config");
        let address = self
            .driver
            .parse_address(text, &config.address_options())
            .expect("address");
        Tag::new(TagId::new(id).expect("tag id"), address)
    }

    /// Read `tags` in one request.
    pub fn read(
        &self,
        connection: &dyn Connection,
        tags: Vec<Tag>,
    ) -> stonecrab::Result<ReadResponse> {
        let mut builder = ReadRequest::builder(connection.limits());
        for tag in tags {
            builder = builder.tag(tag);
        }
        let request = builder.build()?;
        block_on_with(&self.clock, connection.read(request))
    }

    /// Write `items` in one request.
    pub fn write(
        &self,
        connection: &dyn Connection,
        items: Vec<(Tag, PlcValue)>,
    ) -> stonecrab::Result<WriteResponse> {
        let mut builder = WriteRequest::builder(connection.limits());
        for (tag, value) in items {
            builder = builder.item(tag, value);
        }
        let request = builder.build()?;
        block_on_with(&self.clock, connection.write(request))
    }

    /// The `(function, start, quantity)` of every read request that reached the device
    /// across all connections, in order.
    pub fn read_requests(&self) -> Vec<(u8, u16, u16)> {
        let mut requests = Vec::new();
        for probe in self.factory.probes() {
            for frame in probe.sent() {
                let pdu = self.wire.pdu(&frame).unwrap_or_default();
                if let [
                    function @ 1..=4,
                    start_hi,
                    start_lo,
                    quantity_hi,
                    quantity_lo,
                ] = pdu
                {
                    requests.push((
                        *function,
                        u16::from_be_bytes([*start_hi, *start_lo]),
                        u16::from_be_bytes([*quantity_hi, *quantity_lo]),
                    ));
                }
            }
        }
        requests
    }
}
