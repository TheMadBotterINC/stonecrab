//! Batched reads end to end over a real loopback socket (slice S52, ADR-0009 addendum "S52").
//!
//! The driver connects through a `DriverRegistry` to a testkit `ScriptedPeer`, the same
//! pair the S26 loopback tests use, and reads through the real `TcpFactory` and the Tokio
//! `SystemClock`. The peer answers every function 03 request from the request's own header,
//! unit, function and quantity, so one connection serves the whole run. A benchmark is the
//! cost of one `Connection::read` on a current-thread runtime: request validation, batching,
//! the framed exchange per chunk, the decode and the response assembly. Nothing leaves the
//! machine and nothing sleeps; the numbers are library overhead plus loopback, not device or
//! network latency (SPEC.md 'Performance').
#![forbid(unsafe_code)]

use std::error::Error;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{BatchSize, Criterion, Throughput};
use stonecrab::modbus::ModbusTcpDriver;
use stonecrab::testkit::{Reaction, Script, ScriptedPeer, Step};
use stonecrab::{AddressOptions, DriverRegistry, ReadRequest, Tag, TagId};

type Outcome = Result<(), Box<dyn Error>>;

/// Answer a 12-byte function 03 request with registers 0, 1, 2 ... of the asked quantity;
/// anything else ends the connection, which the benchmark then reports as a failed read.
fn reply(request: &[u8]) -> Reaction {
    let Ok([t0, t1, _, _, _, _, unit, function, _, _, q0, q1]) = <[u8; 12]>::try_from(request)
    else {
        return Reaction::Close;
    };
    let quantity = u16::from_be_bytes([q0, q1]);
    let Some(byte_count) = quantity.checked_mul(2).and_then(|n| u8::try_from(n).ok()) else {
        return Reaction::Close;
    };
    let [length_high, length_low] = u16::from(byte_count).saturating_add(3).to_be_bytes();
    let mut frame = vec![
        t0,
        t1,
        0,
        0,
        length_high,
        length_low,
        unit,
        function,
        byte_count,
    ];
    frame.extend((0..quantity).flat_map(u16::to_be_bytes));
    Reaction::Reply(frame)
}

/// The tags a case reads: `(id, address)` pairs.
fn plant_tags() -> Vec<(String, String)> {
    let reals = (1..16)
        .step_by(2)
        .map(|n| format!("holding-register:{n}:REAL"));
    let ints = (17..25).map(|n| format!("holding-register:{n}:INT"));
    let far = (201..209).map(|n| format!("holding-register:{n}:UINT"));
    reals
        .chain(ints)
        .chain(far)
        .enumerate()
        .map(|(index, address)| (format!("tag{index}"), address))
        .collect()
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "the benchmark group stays open until the connection is closed"
)]
fn reads(criterion: &mut Criterion) -> Outcome {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut registry = DriverRegistry::new();
    registry.register(Arc::new(ModbusTcpDriver::new()))?;
    let driver = registry
        .driver("modbus-tcp")
        .ok_or("the Modbus TCP driver is registered")?;
    let peer =
        runtime.block_on(async { ScriptedPeer::spawn(Script::new(vec![Step::react(reply)])) })?;
    let connection =
        runtime.block_on(registry.connect(&format!("modbus-tcp://127.0.0.1:{}", peer.port())))?;

    let single = vec![("one".to_owned(), "holding-register:1:UINT".to_owned())];
    // 125 adjacent registers: one request with the largest response the protocol allows.
    let full = (1..=125)
        .map(|n| (format!("tag{n}"), format!("holding-register:{n}:UINT")))
        .collect::<Vec<_>>();
    let cases = [
        ("one_register_1_request", single),
        ("plant_24_tags_2_requests", plant_tags()),
        ("full_response_125_tags_1_request", full),
    ];

    let mut group = criterion.benchmark_group("loopback_read");
    for (name, tags) in cases {
        let mut builder = ReadRequest::builder(connection.limits());
        for (id, text) in &tags {
            let address = driver.parse_address(text, &AddressOptions::default())?;
            builder = builder.tag(Tag::new(TagId::new(id)?, address));
        }
        let request = builder.build()?;

        // One read outside the timing proves every tag comes back with a value.
        let response = runtime.block_on(connection.read(request.clone()))?;
        if response.iter_failed().next().is_some() || response.items().len() != tags.len() {
            return Err(format!("the {name} read did not return a value for every tag").into());
        }

        let mut failed = false;
        group.throughput(Throughput::Elements(u64::try_from(tags.len())?));
        group.bench_function(name, |b| {
            b.iter_batched(
                || request.clone(),
                |request| {
                    let outcome = runtime.block_on(connection.read(black_box(request)));
                    failed |= outcome.is_err();
                    outcome
                },
                BatchSize::SmallInput,
            );
        });
        if failed {
            return Err(format!("a {name} read failed while it was measured").into());
        }
    }
    group.finish();

    runtime.block_on(connection.close())?;
    runtime.block_on(peer.close())?;
    Ok(())
}

#[expect(
    clippy::significant_drop_tightening,
    reason = "Criterion prints its summary when it is dropped or finished, after every group"
)]
fn main() -> Outcome {
    let mut criterion = Criterion::default().configure_from_args();
    reads(&mut criterion)?;
    criterion.final_summary();
    Ok(())
}
