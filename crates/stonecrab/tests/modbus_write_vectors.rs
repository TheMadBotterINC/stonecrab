//! The write exchanges of the upstream driver suites (`DriverTestsuite.xml` and the two
//! all-datatypes files) against Stonecrab's write path (slice S46).
//!
//! The adapter of slice S04b serializes each exchange from the XML model of the retained
//! upstream file to `fixtures/modbus/tcp/adu_upstream-*` vectors; it never calls Stonecrab's
//! codec. Here each exchange is replayed against `ModbusConnection::write` over the fake
//! transport: the peer script expects the vector's request bytes exactly (a strict script
//! records any difference as a mismatch and closes), and answers with the vector's response
//! bytes. The tag and the value come from the `upstream` object of the expected file, the
//! payload byte order from the suite's `default-payload-byte-order`.
//!
//! The all-datatypes files pair an FC06 request with an FC16 response for the 16-bit
//! writes (found in S04b). Stonecrab requires the response to echo the function code of
//! the request, so it does not take that response for a confirmation
//! (`docs/parity/modbus.md#MB-WRITE-ECHO`); it takes the echo it expects.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::str::FromStr;
    use std::sync::Arc;

    use serde_json::Value;
    use stonecrab::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection, ModbusDataType};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, GoldenVector, Script, Step, block_on_with, load_dir,
    };
    use stonecrab::{
        AccessPolicy, AddressOptions, ByteOrder, Connection, ConnectionConfig, Endpoint, PlcValue,
        Tag, TagId, UnknownReason, WordOrder, WriteOutcome, WriteRequest,
    };

    /// The four upstream mode names and the pair each maps to (ADR-0006 decision 4).
    const MODES: [(&str, ByteOrder, WordOrder); 4] = [
        ("BIG_ENDIAN", ByteOrder::BigEndian, WordOrder::HighWordFirst),
        (
            "LITTLE_ENDIAN",
            ByteOrder::LittleEndian,
            WordOrder::LowWordFirst,
        ),
        (
            "BIG_ENDIAN_BYTE_SWAP",
            ByteOrder::LittleEndian,
            WordOrder::HighWordFirst,
        ),
        (
            "LITTLE_ENDIAN_BYTE_SWAP",
            ByteOrder::BigEndian,
            WordOrder::LowWordFirst,
        ),
    ];

    /// One write exchange of an upstream file: its two vectors.
    struct Exchange {
        file: String,
        case: String,
        byte_order: String,
        address: String,
        data_type: String,
        values: Vec<String>,
        request: GoldenVector,
        response: GoldenVector,
    }

    fn text<'a>(vector: &'a GoldenVector, path: &[&str]) -> Option<&'a str> {
        path.iter()
            .try_fold(vector.expected.as_ref()?, |value, key| value.get(*key))?
            .as_str()
    }

    fn is_write(vector: &GoldenVector) -> bool {
        text(vector, &["upstream", "operation"]) == Some("write")
    }

    fn exchanges() -> Vec<Exchange> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/modbus");
        let all: Vec<GoldenVector> = load_dir(&dir)
            .expect("fixtures/modbus loads: every file listed, every listed vector present")
            .into_iter()
            .filter(is_write)
            .collect();
        let of = |vector: &GoldenVector, direction: &str| {
            text(vector, &["pdu", "direction"]) == Some(direction)
        };
        all.iter()
            .filter(|vector| of(vector, "request"))
            .map(|request| {
                let key = |vector: &GoldenVector| {
                    (
                        text(vector, &["upstream", "file"]).map(str::to_owned),
                        text(vector, &["upstream", "case"]).map(str::to_owned),
                    )
                };
                let response = all
                    .iter()
                    .find(|vector| of(vector, "response") && key(vector) == key(request))
                    .unwrap_or_else(|| panic!("{}: a response vector", request.name));
                let upstream = |path: &[&str]| {
                    text(request, path)
                        .unwrap_or_else(|| panic!("{}: {path:?}", request.name))
                        .to_owned()
                };
                let values = request
                    .expected
                    .as_ref()
                    .and_then(|expected| expected.pointer("/upstream/values"))
                    .and_then(Value::as_array)
                    .unwrap_or_else(|| panic!("{}: the values written", request.name));
                let values = values
                    .iter()
                    .map(|value| {
                        value
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or_else(|| panic!("{}: a value text", request.name))
                            .to_owned()
                    })
                    .collect();
                Exchange {
                    file: upstream(&["upstream", "file"]),
                    case: upstream(&["upstream", "case"]),
                    byte_order: upstream(&["upstream", "byte_order"]),
                    address: upstream(&["upstream", "tag", "address"]),
                    data_type: upstream(&["upstream", "tag", "data_type"]),
                    values,
                    request: request.clone(),
                    response: response.clone(),
                }
            })
            .collect()
    }

    fn parse<T: FromStr>(text: &str) -> T {
        text.parse()
            .unwrap_or_else(|_| panic!("`{text}` is not a value of its data type"))
    }

    /// The value the XML writes, as the variant of its data type.
    fn value_of(data_type: ModbusDataType, text: &str) -> PlcValue {
        match data_type {
            ModbusDataType::Bool => PlcValue::Bool(text == "true"),
            ModbusDataType::Byte | ModbusDataType::USInt => PlcValue::U8(parse(text)),
            ModbusDataType::SInt => PlcValue::I8(parse(text)),
            ModbusDataType::Word | ModbusDataType::UInt => PlcValue::U16(parse(text)),
            ModbusDataType::Int => PlcValue::I16(parse(text)),
            ModbusDataType::DWord | ModbusDataType::UDInt => PlcValue::U32(parse(text)),
            ModbusDataType::DInt => PlcValue::I32(parse(text)),
            ModbusDataType::LWord | ModbusDataType::ULInt => PlcValue::U64(parse(text)),
            ModbusDataType::LInt => PlcValue::I64(parse(text)),
            ModbusDataType::Real => PlcValue::F32(parse(text)),
            ModbusDataType::LReal => PlcValue::F64(parse(text)),
            other @ (ModbusDataType::Char
            | ModbusDataType::WChar
            | ModbusDataType::String
            | ModbusDataType::WString
            | _) => panic!("{other:?} is not a type of these suites"),
        }
    }

    /// What a replayed write came to.
    #[derive(Debug, PartialEq, Eq)]
    enum Verdict {
        Confirmed,
        Unknown(UnknownReason),
    }

    /// Replay the exchange: the peer expects the request bytes of the vector and answers
    /// with `reply`; the verdict on the one write and the frames the peer received.
    fn replay(
        exchange: &Exchange,
        address: ModbusAddress,
        value: PlcValue,
        reply: Vec<u8>,
    ) -> (Verdict, Vec<Vec<u8>>) {
        let (_, byte_order, word_order) = MODES
            .iter()
            .find(|(name, _, _)| *name == exchange.byte_order)
            .copied()
            .expect("one of the four upstream modes");
        let mut defaults = ConnectionDefaults::default();
        defaults.byte_order = byte_order;
        defaults.word_order = word_order;

        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![
            Step::Expect(exchange.request.bytes.clone()),
            Step::Reply(reply),
        ]));
        let config =
            ConnectionConfig::builder("modbus-tcp", Endpoint::Custom("fake://plc".to_owned()))
                .access(AccessPolicy::ReadWrite)
                .build()
                .expect("config");
        let connection = block_on_with(
            &clock,
            ModbusConnection::open(
                config,
                Arc::new(factory.clone()),
                Arc::new(clock.clone()),
                defaults,
            ),
        )
        .expect("open");
        let tag = Tag::new(TagId::new("hurz").expect("tag id"), Arc::new(address));
        let request = WriteRequest::builder(connection.limits())
            .item(tag, value)
            .build()
            .expect("write request");
        let response = block_on_with(&clock, connection.write(request)).expect("an outcome");
        let outcome = response.results().first().expect("one result").outcome();
        let verdict = if matches!(outcome, WriteOutcome::Confirmed) {
            Verdict::Confirmed
        } else if let WriteOutcome::Unknown(reason) = outcome {
            Verdict::Unknown(*reason)
        } else {
            panic!(
                "{}: neither confirmed nor unknown: {outcome:?}",
                exchange.case
            );
        };
        let probes = factory.probes();
        let probe = probes.first().expect("a transport");
        assert!(
            probe.report().mismatches.is_empty(),
            "{}: the peer saw exactly the upstream request: {:?}",
            exchange.case,
            probe.report().mismatches
        );
        let sent = probe.sent().iter().map(|frame| frame.to_vec()).collect();
        (verdict, sent)
    }

    // covers: MB-TCP-WRITE, MB-BYTEORDER-BE, MB-BYTEORDER-LE, MB-WRITE-ECHO, MB-ADDR-SELECTION /
    // Stonecrab writes the bytes upstream's own driver suites expect, for the tag and the
    // value they name
    #[test]
    fn stonecrab_sends_the_request_bytes_of_every_upstream_write_it_can_address() {
        let all = exchanges();
        assert_eq!(
            all.len(),
            30,
            "2 + 14 + 14 write exchanges in the three files"
        );
        let (mut replayed, mut quirks, mut lists) = (0, 0, 0);
        for exchange in &all {
            let address = ModbusAddress::parse(&exchange.address, &AddressOptions::default())
                .unwrap_or_else(|error| panic!("{}: {error}", exchange.address));
            let data_type = ModbusDataType::from_upstream_name(&exchange.data_type)
                .unwrap_or_else(|error| panic!("{}: {error}", exchange.data_type));
            let mut values = exchange.values.iter().map(|text| value_of(data_type, text));
            // An array selection writes the list of the values the XML gives (S48).
            let value = if address.elements().is_some() {
                lists += 1;
                PlcValue::List(values.collect())
            } else {
                values.next().expect("one value")
            };
            let name = format!("{} / {}", exchange.file, exchange.case);

            // Stonecrab expects the echo of the function it sent. Where the file pairs the
            // request with a response of another function, upstream's response is not a
            // confirmation, and the echo (an FC06 echo repeats the request) is.
            let quirk = text(&exchange.response, &["upstream", "quirk"]).is_some();
            let (verdict, sent) = replay(
                exchange,
                address.clone(),
                value.clone(),
                exchange.response.bytes.clone(),
            );
            assert_eq!(
                sent,
                vec![exchange.request.bytes.clone()],
                "{name}: the frame Stonecrab sent is the upstream request, once"
            );
            if quirk {
                quirks += 1;
                assert_eq!(
                    verdict,
                    Verdict::Unknown(UnknownReason::TimeoutAfterTransmit),
                    "{name}: the response of another function does not confirm the write"
                );
                let (verdict, _) = replay(exchange, address, value, exchange.request.bytes.clone());
                assert_eq!(
                    verdict,
                    Verdict::Confirmed,
                    "{name}: the echo of the request does"
                );
            } else {
                assert_eq!(
                    verdict,
                    Verdict::Confirmed,
                    "{name}: upstream's response is the echo"
                );
            }
            replayed += 1;
        }
        assert_eq!(lists, 1, "the one array selection of DriverTestsuite.xml");
        assert_eq!(replayed, 30, "every exchange");
        assert_eq!(
            quirks, 14,
            "seven 16-bit writes in each all-datatypes file are FC06 requests answered as FC16"
        );
    }
}
