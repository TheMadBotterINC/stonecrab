//! Benchmarks of the Modbus parsing, codec, decoding and planning paths (slice S52,
//! ADR-0009 addendum "S52").
//!
//! Every input is built once, outside the timed closure; the closure runs one operation and
//! returns its result, which Criterion keeps from being optimized away. A setup step proves
//! that each benchmark exercises the success path, so a regression cannot hide as a faster
//! error. Nothing here opens a socket (the loopback benchmark is `modbus_loopback`) and
//! nothing sleeps.
#![forbid(unsafe_code)]

use std::error::Error;
use std::hint::black_box;
use std::num::NonZeroUsize;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use criterion::{BatchSize, Criterion, Throughput};
use stonecrab::codec::{BoundedReader, BoundedWriter};
use stonecrab::modbus::{
    BatchLimits, Batcher, FunctionCode, MAX_PDU_BYTES, MAX_READ_REGISTERS, MbapFramer,
    ModbusAddress, ModbusAddressBuilder, ModbusAdu, ModbusArea, ModbusDataType, ModbusPdu,
    ModbusTcpDriver, TypedDecoder, UnitId, decode_request, decode_response, encode_request,
    encode_response,
};
use stonecrab::transport::Framer;
use stonecrab::{AddressOptions, ByteOrder, Driver, Tag, TagId, WordOrder};

type Outcome = Result<(), Box<dyn Error>>;

/// The two address forms of MB-ADDR-LONG: bare, and with the `{...}` block.
const ADDRESSES: [(&str, &str); 3] = [
    ("long", "holding-register:16:INT"),
    ("bit_area", "coil:7:BOOL"),
    (
        "with_block",
        "holding-register:100:REAL{unit-id: 17, byte-order: 'LITTLE_ENDIAN'}",
    ),
];

/// A bare host and a string that names every parameter the driver reads.
const CONNECTION_STRINGS: [(&str, &str); 2] = [
    ("host_only", "modbus-tcp://192.0.2.10"),
    (
        "all_parameters",
        "modbus-tcp://192.0.2.10:1502?default-unit-identifier=7&request-timeout-ms=800\
         &default-payload-byte-order=LITTLE_ENDIAN&max-registers-per-request=100\
         &max-coils-per-request=1000&ping-address=holding-register:9:UINT",
    ),
];

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the function ends"
)]
fn parse(criterion: &mut Criterion) -> Outcome {
    let options = AddressOptions::default();
    let driver = ModbusTcpDriver::new();
    let mut group = criterion.benchmark_group("parse");
    for (name, text) in ADDRESSES {
        ModbusAddress::parse(text, &options)?;
        group.bench_function(format!("address_{name}"), |b| {
            b.iter(|| ModbusAddress::parse(black_box(text), &options));
        });
    }
    for (name, text) in CONNECTION_STRINGS {
        driver.parse_config(text)?;
        group.bench_function(format!("connection_string_{name}"), |b| {
            b.iter(|| driver.parse_config(black_box(text)));
        });
    }
    group.finish();
    Ok(())
}

/// The registers 0, 1, 2 ... of a response with `count` of them.
fn read_response(count: u16) -> ModbusPdu {
    ModbusPdu::ReadRegistersResponse {
        function: FunctionCode::ReadHoldingRegisters,
        registers: (0..count).collect(),
    }
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the function ends"
)]
fn pdu(criterion: &mut Criterion) -> Outcome {
    let mut group = criterion.benchmark_group("pdu");
    // The encode benchmarks time a fresh `BoundedWriter` (its allocation) and `into_bytes`
    // (the copy-out) on every iteration, unlike `mbap`, which reuses its buffer.

    let request = ModbusPdu::ReadHoldingRegisters {
        start: 0,
        quantity: MAX_READ_REGISTERS,
    };
    let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
    encode_request(&request, &mut writer)?;
    let request_bytes = writer.into_bytes();
    group.throughput(Throughput::Bytes(u64::try_from(request_bytes.len())?));
    group.bench_function("encode_request", |b| {
        b.iter(|| {
            let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
            encode_request(black_box(&request), &mut writer).map(|()| writer.into_bytes())
        });
    });
    group.bench_function("decode_request", |b| {
        b.iter(|| decode_request(&mut BoundedReader::new(black_box(&request_bytes))));
    });

    for (name, count) in [("small", 2), ("max", MAX_READ_REGISTERS)] {
        let response = read_response(count);
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_response(&response, &mut writer)?;
        let bytes = writer.into_bytes();
        group.throughput(Throughput::Bytes(u64::try_from(bytes.len())?));
        group.bench_function(format!("encode_response_{name}"), |b| {
            b.iter(|| {
                let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
                encode_response(black_box(&response), &mut writer).map(|()| writer.into_bytes())
            });
        });
        let function = FunctionCode::ReadHoldingRegisters;
        decode_response(&mut BoundedReader::new(&bytes), function)?;
        group.bench_function(format!("decode_response_{name}"), |b| {
            b.iter(|| decode_response(&mut BoundedReader::new(black_box(&bytes)), function));
        });
    }
    group.finish();
    Ok(())
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the function ends"
)]
fn mbap(criterion: &mut Criterion) -> Outcome {
    let mut group = criterion.benchmark_group("mbap");
    let mut framer = MbapFramer::new();
    for (name, count) in [("small", 2), ("max", MAX_READ_REGISTERS)] {
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_response(&read_response(count), &mut writer)?;
        let adu = ModbusAdu::new(1, UnitId::DEFAULT, Bytes::from(writer.into_bytes()))?;
        let mut wire = BytesMut::with_capacity(adu.wire_len());
        framer.encode(&adu, &mut wire)?;
        group.throughput(Throughput::Bytes(u64::try_from(wire.len())?));

        // The buffer is reused, so the loop times the framing and not the allocation.
        group.bench_function(format!("encode_{name}"), |b| {
            let mut out = BytesMut::with_capacity(wire.len());
            b.iter(|| {
                out.clear();
                framer.encode(black_box(&adu), &mut out)
            });
        });
        let mut check = wire.clone();
        framer.decode(&mut check)?;
        group.bench_function(format!("decode_{name}"), |b| {
            b.iter_batched_ref(
                || wire.clone(),
                |buffer| framer.decode(black_box(buffer)),
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
    Ok(())
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the function ends"
)]
fn typed_decode(criterion: &mut Criterion) -> Outcome {
    let budget = NonZeroUsize::new(64).ok_or("the raw budget is not zero")?;
    let registers: Vec<u16> = (0..MAX_READ_REGISTERS).collect();
    let mut group = criterion.benchmark_group("typed_decode");
    // A full response read as 62 floats (two registers each) and as 125 unsigned integers.
    for (name, data_type) in [
        ("real", ModbusDataType::Real),
        ("uint", ModbusDataType::UInt),
    ] {
        let width = usize::from(data_type.register_size());
        let values = registers.chunks_exact(width).count();
        let decode_all = || {
            black_box(&registers)
                .chunks_exact(width)
                .flat_map(|chunk| {
                    TypedDecoder::decode_registers(
                        chunk,
                        data_type,
                        ByteOrder::BigEndian,
                        WordOrder::HighWordFirst,
                        budget,
                    )
                })
                .inspect(|value| {
                    black_box(value);
                })
                .count()
        };
        if decode_all() != values {
            return Err(format!("a {name} in the registers did not decode").into());
        }
        group.throughput(Throughput::Elements(u64::try_from(values)?));
        group.bench_function(format!("{name}_batch_of_{values}"), |b| {
            b.iter(decode_all);
        });
    }
    group.finish();
    Ok(())
}

/// A tag named `id` at the address the builder describes.
fn tag(id: &str, address: ModbusAddressBuilder) -> Result<Tag, Box<dyn Error>> {
    Ok(Tag::new(TagId::new(id)?, Arc::new(address.build()?)))
}

const fn holding(offset: u16, data_type: ModbusDataType) -> ModbusAddressBuilder {
    ModbusAddress::builder(ModbusArea::HoldingRegister, offset).data_type(data_type)
}

/// A plant-sized set: 8 floats and 8 integers in one run of holding registers, 8 unsigned
/// values far from them, 4 coils and 4 registers of a second unit.
fn plant_tags() -> Result<Vec<Tag>, Box<dyn Error>> {
    let mut tags = Vec::new();
    for offset in (0..16).step_by(2) {
        tags.push(tag(
            &format!("real{offset}"),
            holding(offset, ModbusDataType::Real),
        )?);
    }
    for offset in 16..24 {
        tags.push(tag(
            &format!("int{offset}"),
            holding(offset, ModbusDataType::Int),
        )?);
    }
    for offset in 200..208 {
        tags.push(tag(
            &format!("uint{offset}"),
            holding(offset, ModbusDataType::UInt),
        )?);
    }
    for offset in 0..4 {
        let coil = ModbusAddress::builder(ModbusArea::Coil, offset).data_type(ModbusDataType::Bool);
        tags.push(tag(&format!("coil{offset}"), coil)?);
    }
    for offset in 0..4 {
        let input = ModbusAddress::builder(ModbusArea::InputRegister, offset)
            .data_type(ModbusDataType::UInt)
            .unit_id(UnitId::from(2));
        tags.push(tag(&format!("unit2_{offset}"), input)?);
    }
    Ok(tags)
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the function ends"
)]
fn plan(criterion: &mut Criterion) -> Outcome {
    let limits = BatchLimits::default();
    let plant = plant_tags()?;
    let adjacent = (0..1024)
        .map(|offset| tag(&format!("t{offset}"), holding(offset, ModbusDataType::Int)))
        .collect::<Result<Vec<_>, _>>()?;
    let mut group = criterion.benchmark_group("plan");
    for (name, tags) in [("plant_28_tags", &plant), ("adjacent_1024_tags", &adjacent)] {
        Batcher::plan(tags, &limits)?;
        group.throughput(Throughput::Elements(u64::try_from(tags.len())?));
        group.bench_function(name, |b| {
            b.iter(|| Batcher::plan(black_box(tags), &limits));
        });
    }
    group.finish();
    Ok(())
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "Criterion prints its summary when it is dropped or finished, after every group"
)]
fn main() -> Outcome {
    let mut criterion = Criterion::default().configure_from_args();
    parse(&mut criterion)?;
    pdu(&mut criterion)?;
    mbap(&mut criterion)?;
    typed_decode(&mut criterion)?;
    plan(&mut criterion)?;
    criterion.final_summary();
    Ok(())
}
