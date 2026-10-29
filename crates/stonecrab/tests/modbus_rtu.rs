//! Modbus RTU (slice S59, ADR-0024): the framer through the public API, the `modbus-rtu`
//! driver over the fake transport on the fake clock, the failure cases of SPEC.md for RTU
//! (CRC errors, timing boundaries, a lost serial link and its reopen, replies from the
//! wrong unit), the contract suite, and RTU over a TCP gateway on a loopback socket.
//!
//! The expected frames are built here with a CRC-16 written independently of the crate's
//! (bit by bit, from the Modbus over Serial Line specification), so an encoder and a decoder
//! that agreed on a wrong CRC would not pass. Nothing touches a device and nothing sleeps
//! on real time: the driver runs on `FakeClock` with `FakeTransportFactory`, except for the
//! gateway test (a `ScriptedPeer` on loopback) and the missing-port test (a port name that
//! does not exist).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the RTU suite drives the device's read, write and fault hooks, not its inspection helpers"
)]
mod responder;

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use bytes::{Bytes, BytesMut};
    use proptest::prelude::*;
    use stonecrab::codec::BoundedReader;
    use stonecrab::modbus::{
        ExceptionCode, FunctionCode, ModbusPdu, ModbusRtuDriver, ModbusTcpDriver, RtuAdu,
        RtuFramer, UnitId, decode_response,
    };
    use stonecrab::testkit::{
        CaseOutcome, ContractFixture, ContractReport, ContractSuite, FakeClock,
        FakeTransportFactory, FixtureAddress, InteropMap, Reaction, Reactor, Script, ScriptedPeer,
        Step, block_on_with,
    };
    use stonecrab::transport::{FrameError, Framer};
    use stonecrab::{
        Clock, Connection, Driver, Endpoint, ErrorKind, FailureCategory, PlcValue, ReadRequest,
        ReadResponse, Result, Tag, TagId, WriteOutcome, WriteRequest, WriteResponse,
    };

    use crate::responder::Device;

    /// A serial port at the defaults: 9600 baud, 8 data bits, no parity, one stop bit.
    const PORT: &str = "modbus-rtu:///dev/ttyUSB0";

    /// The silence before a request at 9600 8N1: 3.5 characters of ten bits.
    const GAP: Duration = Duration::from_nanos(3_645_831);

    /// The silence after a request nothing answered: the gap and 256 characters.
    const DRAIN: Duration = Duration::from_nanos(270_312_327);

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
    fn frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut bytes = vec![unit];
        bytes.extend_from_slice(pdu);
        let check = crc(&bytes);
        bytes.extend_from_slice(&check);
        bytes
    }

    /// The read of `count` holding registers from wire address `start` of `unit`.
    fn read_registers(unit: u8, start: u16, count: u16) -> Vec<u8> {
        let [start_high, start_low] = start.to_be_bytes();
        let [count_high, count_low] = count.to_be_bytes();
        frame(unit, &[0x03, start_high, start_low, count_high, count_low])
    }

    /// The reply of `unit` carrying `values`.
    fn registers(unit: u8, values: &[u16]) -> Vec<u8> {
        let mut pdu = vec![
            0x03,
            u8::try_from(values.len().saturating_mul(2)).expect("a short reply"),
        ];
        for value in values {
            pdu.extend_from_slice(&value.to_be_bytes());
        }
        frame(unit, &pdu)
    }

    /// The driver on the fake clock over a fake factory with `scripts` queued.
    struct Bench {
        clock: FakeClock,
        factory: FakeTransportFactory,
        driver: ModbusRtuDriver,
    }

    impl Bench {
        fn new(scripts: Vec<Script>) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            for script in scripts {
                factory.push_script(script);
            }
            let driver = ModbusRtuDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_transport_factory(Arc::new(factory.clone()));
            Self {
                clock,
                factory,
                driver,
            }
        }

        fn connect(&self, uri: &str) -> Box<dyn Connection> {
            let config = self.driver.parse_config(uri).expect("config");
            block_on_with(&self.clock, self.driver.connect(config)).expect("connect")
        }

        fn read(&self, connection: &dyn Connection, tags: Vec<Tag>) -> Result<ReadResponse> {
            let mut builder = ReadRequest::builder(connection.limits());
            for tag in tags {
                builder = builder.tag(tag);
            }
            block_on_with(&self.clock, connection.read(builder.build()?))
        }

        fn write(&self, connection: &dyn Connection, tag: Tag, value: PlcValue) -> WriteResponse {
            let request = WriteRequest::builder(connection.limits())
                .item(tag, value)
                .build()
                .expect("request");
            block_on_with(&self.clock, connection.write(request)).expect("write")
        }

        fn sent(&self) -> Vec<Vec<u8>> {
            self.factory
                .probes()
                .iter()
                .flat_map(stonecrab::testkit::FakeTransportProbe::sent)
                .map(|bytes| bytes.to_vec())
                .collect()
        }
    }

    fn tag(id: &str, text: &str) -> Tag {
        let address = ModbusRtuDriver::new()
            .parse_address(text, &stonecrab::AddressOptions::default())
            .expect("address");
        Tag::new(TagId::new(id).expect("tag id"), address)
    }

    fn value(response: &ReadResponse, id: &str) -> Option<PlcValue> {
        response
            .get(&TagId::new(id).expect("id"))
            .and_then(|item| item.outcome().as_ref().ok().cloned())
    }

    /// A reactor that records when each request arrived and answers with `reply`.
    fn timed(clock: &FakeClock, arrivals: &Arc<Mutex<Vec<Instant>>>, reply: Vec<u8>) -> Step {
        let (clock, arrivals) = (clock.clone(), Arc::clone(arrivals));
        Step::react(move |_request| {
            arrivals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(clock.now());
            Reaction::Reply(reply.clone())
        })
    }

    fn arrivals_of(arrivals: &Arc<Mutex<Vec<Instant>>>) -> Vec<Instant> {
        arrivals
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    // ---- framing through the public API ----

    // covers: MBS-RTU-WIRE-MODEL / the upstream RTU parser vectors
    #[test]
    fn rtu_adu_round_trips_the_upstream_vectors() {
        // Transcribed from Apache PLC4X v1.0.0 (commit ea229918),
        // protocols/modbus/src/test/resources/protocols/modbus/rtu/ParserSerializerTestsuite.xml:
        // the <raw> hex of "Read Holding Registers Request" (address 1, start 0, quantity
        // 10, crc 50637) and of "Read Holding Registers Response" (address 1, byte count
        // 20, twenty zero bytes, crc 41831).
        let request = hex("01030000000ac5cd");
        let response = hex("0103140000000000000000000000000000000000000000a367");
        assert_eq!(
            u16::from_le_bytes([0xC5, 0xCD]),
            52677,
            "the CRC as the frame carries it"
        );
        assert_eq!(
            u16::from_be_bytes([0xC5, 0xCD]),
            50637,
            "the CRC as the XML writes it"
        );
        assert_eq!(u16::from_be_bytes([0xA3, 0x67]), 41831);

        let mut framer = RtuFramer::new();
        let adu =
            RtuAdu::new(UnitId::from(1), Bytes::from_static(&[0x03, 0, 0, 0, 0x0A])).expect("adu");
        let mut wire = BytesMut::new();
        framer.encode(&adu, &mut wire).expect("encode");
        assert_eq!(
            wire.to_vec(),
            request,
            "the request encodes to the upstream bytes"
        );

        let mut buffer = BytesMut::from(&response[..]);
        let reply = framer
            .decode(&mut buffer)
            .expect("no error")
            .expect("a frame");
        assert!(buffer.is_empty(), "the whole response was one frame");
        assert_eq!(reply.unit_id(), UnitId::from(1));
        assert_eq!(reply.wire_len(), response.len());
        let pdu = decode_response(
            &mut BoundedReader::new(reply.pdu()),
            FunctionCode::ReadHoldingRegisters,
        )
        .expect("the shared PDU codec reads it");
        assert_eq!(
            pdu,
            ModbusPdu::ReadRegistersResponse {
                function: FunctionCode::ReadHoldingRegisters,
                registers: vec![0; 10],
            }
        );
        let mut again = BytesMut::new();
        framer.encode(&reply, &mut again).expect("encode");
        assert_eq!(
            again.to_vec(),
            response,
            "the response re-encodes to its bytes"
        );
    }

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let digits = std::str::from_utf8(pair).expect("ASCII");
                u8::from_str_radix(digits, 16).expect("hex")
            })
            .collect()
    }

    // covers: MBS-RTU-CRC / CRC appended low byte first on send
    #[test]
    fn rtu_request_carries_crc_low_byte_first() {
        // Modbus Application Protocol V1.1b section 6.3: registers 108 to 110 of unit 17.
        let adu =
            RtuAdu::new(UnitId::from(17), Bytes::from_static(&[3, 0, 0x6B, 0, 3])).expect("adu");
        let mut wire = BytesMut::new();
        RtuFramer::new().encode(&adu, &mut wire).expect("encode");
        assert_eq!(
            wire.to_vec(),
            [0x11, 0x03, 0x00, 0x6B, 0x00, 0x03, 0x76, 0x87]
        );
        assert_eq!(
            wire.to_vec(),
            read_registers(17, 0x6B, 3),
            "the test's own CRC agrees"
        );
    }

    // covers: MBS-RTU-SIZE-TABLE / exception, read and write echo sizes; unknown codes
    #[test]
    fn rtu_responses_are_sized_by_their_function_code() {
        let mut framer = RtuFramer::new();
        for (name, bytes) in [
            ("rtu_exception_response_is_5_bytes", frame(1, &[0x83, 0x02])),
            (
                "rtu_read_response_size_is_5_plus_byte_count",
                frame(1, &[0x01, 0x02, 0x55, 0x01]),
            ),
            (
                "rtu_write_echo_is_8_bytes",
                frame(1, &[0x06, 0x00, 0x0A, 0x12, 0x34]),
            ),
            (
                "write multiple echo",
                frame(1, &[0x10, 0x00, 0x0A, 0x00, 0x02]),
            ),
        ] {
            let mut buffer = BytesMut::from(&bytes[..]);
            buffer.extend_from_slice(&[0x01]);
            let adu = framer.decode(&mut buffer).expect(name).expect(name);
            assert_eq!(adu.wire_len(), bytes.len(), "{name}");
            assert_eq!(buffer.to_vec(), [0x01], "{name}: only the frame was taken");
        }
        // rtu_unknown_function_code_desynchronizes: no size, so no frame (and no waiting).
        let mut unknown = BytesMut::from(&frame(1, &[0x2B, 0x0E, 0x01])[..]);
        assert_eq!(framer.decode(&mut unknown), Err(FrameError::Malformed));
        assert_eq!(unknown.len(), 6, "nothing removed");
        // A frame still arriving waits.
        let mut partial = BytesMut::from(&[0x01, 0x03, 0x04, 0x00][..]);
        assert_eq!(framer.decode(&mut partial), Ok(None));
    }

    // covers: MBS-RTU-CRC / a bad CRC is no frame
    #[test]
    fn rtu_frame_with_a_bad_crc_is_rejected() {
        let mut bytes = registers(1, &[42]);
        let last = bytes.last_mut().expect("a CRC");
        *last ^= 0x01;
        let mut buffer = BytesMut::from(&bytes[..]);
        assert_eq!(
            RtuFramer::new().decode(&mut buffer),
            Err(FrameError::Malformed)
        );
        assert_eq!(buffer.to_vec(), bytes, "nothing removed");
    }

    // covers: MBS-RTU-RESYNC / a stray byte in front of a reply whose header misreads as incomplete
    #[test]
    fn a_stray_byte_in_front_of_a_write_echo_an_exception_or_a_short_read_is_skipped() {
        let mut framer = RtuFramer::new();
        // Read from the stray byte, unit 1 is the function code 01 and the reply's function
        // code its byte count, which asks for more bytes than the buffer holds.
        for (name, reply) in [
            (
                "write single register echo",
                frame(1, &[0x06, 0x00, 0x01, 0x00, 0x03]),
            ),
            (
                "write multiple registers echo",
                frame(1, &[0x10, 0x00, 0x0A, 0x00, 0x02]),
            ),
            ("exception reply", frame(1, &[0x83, 0x02])),
            ("one input register", frame(1, &[0x04, 0x02, 0xFF, 0xFF])),
        ] {
            for stray in [0x00, 0xFF] {
                let mut buffer = BytesMut::from(&[stray][..]);
                buffer.extend_from_slice(&reply);
                let adu = framer.decode(&mut buffer).expect(name).expect(name);
                assert_eq!(adu.unit_id(), UnitId::from(1), "{name}");
                assert_eq!(adu.wire_len(), reply.len(), "{name}");
                assert!(
                    buffer.is_empty(),
                    "{name}: the stray byte went with the frame"
                );
            }
        }
        // The same stray byte in front of an echo still arriving waits, and keeps every byte.
        let mut partial = BytesMut::from(&[0x00, 0x01, 0x06, 0x00, 0x01][..]);
        assert_eq!(framer.decode(&mut partial), Ok(None));
        assert_eq!(
            partial.to_vec(),
            [0x00, 0x01, 0x06, 0x00, 0x01],
            "nothing removed"
        );
    }

    /// Whether some window of `bytes` that starts before `end` ends in its own correct CRC.
    fn a_crc_matches_from_before(bytes: &[u8], end: usize) -> bool {
        (0..end).any(|start| {
            (start.saturating_add(3)..=bytes.len()).any(|stop| {
                let body = bytes.get(start..stop.saturating_sub(2));
                let check = bytes.get(stop.saturating_sub(2)..stop);
                body.map(crc).as_ref().map(<[u8; 2]>::as_slice) == check
            })
        })
    }

    proptest! {
        // covers: MBS-RTU-RESYNC / rtu_framer_never_panics_on_random_bytes
        // covers: SPEC mandatory-failure-cases / F1 malformed frames
        #[test]
        fn rtu_framer_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
            let mut framer = RtuFramer::new();
            let mut buffer = BytesMut::from(&bytes[..]);
            let before = buffer.clone();
            if let Ok(Some(adu)) = framer.decode(&mut buffer) {
                let consumed = before.len().saturating_sub(buffer.len());
                prop_assert!(consumed >= adu.wire_len(), "the frame came out of the buffer");
                prop_assert!(adu.wire_len() <= RtuAdu::MAX_BYTES, "within the ADU limit");
            } else {
                prop_assert_eq!(buffer, before, "nothing removed");
            }
        }

        // covers: MBS-RTU-RESYNC / rtu_noise_before_a_frame_is_skipped_byte_by_byte
        #[test]
        fn rtu_noise_before_a_frame_is_skipped_byte_by_byte(
            noise in proptest::collection::vec(any::<u8>(), 1..40),
            unit in any::<u8>(),
            values in proptest::collection::vec(any::<u16>(), 1..10),
        ) {
            let reply = registers(unit, &values);
            let mut buffer = BytesMut::from(&noise[..]);
            buffer.extend_from_slice(&reply);
            // Two cases are not noise to skip: a window starting in the noise that ends in its
            // own correct CRC is a frame (a rare case), and a header at the front whose byte
            // count is above the ADU limit is refused from the header alone.
            prop_assume!(!a_crc_matches_from_before(&buffer, noise.len()));
            let header = (buffer.get(1).copied(), buffer.get(2).copied());
            prop_assume!(!matches!(
                header,
                (Some(0x01..=0x04 | 0x14 | 0x15 | 0x17), Some(0xFC..=0xFF))
            ));
            let adu = RtuFramer::new().decode(&mut buffer);
            let adu = adu.expect("no error").expect("the frame behind the noise");
            prop_assert_eq!(adu.unit_id(), UnitId::from(unit));
            prop_assert_eq!(adu.wire_len(), reply.len());
            prop_assert!(buffer.is_empty(), "noise and frame are gone together");
        }
    }

    // ---- the driver over the fake transport ----

    // covers: MBS-OP-CONNECT / rtu_connect_opens_the_configured_port_without_sending
    // covers: MBS-TRANS-PORT-NAME / serial_port_name_comes_from_the_connection_string
    #[test]
    fn rtu_connect_opens_the_configured_port_without_sending_and_close_releases_it() {
        let bench = Bench::new(vec![Script::default(), Script::default()]);
        let connection = bench.connect(PORT);
        let opens = bench.factory.opens();
        assert_eq!(
            opens.first().map(|open| open.endpoint.clone()),
            Some(Endpoint::Custom("/dev/ttyUSB0".to_owned())),
            "the port name is the authority as written"
        );
        assert!(bench.sent().is_empty(), "no Modbus traffic on connect");
        // close_releases_the_serial_port
        block_on_with(&bench.clock, connection.close()).expect("close");
        assert!(
            bench
                .factory
                .probes()
                .iter()
                .all(stonecrab::testkit::FakeTransportProbe::is_shut_down),
            "the port is released"
        );
        let windows = bench.connect("modbus-rtu://COM3");
        assert_eq!(
            bench
                .factory
                .opens()
                .get(1)
                .map(|open| open.endpoint.clone()),
            Some(Endpoint::Custom("COM3".to_owned()))
        );
        drop(windows);
    }

    // covers: MBS-OP-READ / rtu_read_of_holding_registers_decodes_big_endian
    // covers: MBS-TYPE-SHARED / rtu_read_decodes_a_real_through_the_shared_decoder
    // covers: MBS-CFG-BYTE-ORDER / serial_default_byte_order_is_big_endian
    #[test]
    fn rtu_read_of_holding_registers_decodes_big_endian() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 3)),
            Step::Reply(registers(1, &[0x1234, 0x4049, 0x0FDB])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(
                &*connection,
                vec![
                    tag("word", "holding-register:1:UINT"),
                    tag("real", "holding-register:2:REAL"),
                ],
            )
            .expect("read");
        assert_eq!(value(&response, "word"), Some(PlcValue::U16(0x1234)));
        assert_eq!(
            value(&response, "real"),
            Some(PlcValue::F32(f32::from_bits(0x4049_0FDB))),
            "high word first, as BIG_ENDIAN"
        );
    }

    // covers: MBS-OP-READ / serial_read_sends_one_request_at_a_time
    // covers: MBS-BUS-ONE-IN-FLIGHT / serial_second_request_waits_for_the_first
    #[test]
    fn serial_read_sends_one_request_at_a_time() {
        // Two areas are two chunks: the second request goes out after the first reply.
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(frame(1, &[0x01, 0x00, 0x00, 0x00, 0x01])),
            Step::Reply(frame(1, &[0x01, 0x01, 0x01])),
            Step::Expect(read_registers(1, 4, 1)),
            Step::Reply(registers(1, &[7])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(
                &*connection,
                vec![tag("coil", "coil:1"), tag("word", "holding-register:5")],
            )
            .expect("read");
        assert_eq!(response.split_into().get(), 2);
        assert_eq!(value(&response, "coil"), Some(PlcValue::Bool(true)));
        assert_eq!(value(&response, "word"), Some(PlcValue::I16(7)));
    }

    // covers: MBS-CFG-SERIAL-INTERFRAME / serial_write_waits_for_the_interframe_gap
    // covers: MBS-RTU-SILENCE / rtu_write_gap_is_measured_from_the_last_read
    // covers: SPEC mandatory-failure-cases / RTU timing boundaries
    #[test]
    fn serial_write_waits_for_the_interframe_gap() {
        let clock_bench = Bench::new(Vec::new());
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        clock_bench.factory.push_script(Script::new(vec![timed(
            &clock_bench.clock,
            &arrivals,
            registers(1, &[1]),
        )]));
        let connection = clock_bench.connect(PORT);
        let start = clock_bench.clock.now();
        for _ in 0..2 {
            drop(
                clock_bench
                    .read(&*connection, vec![tag("a", "holding-register:1")])
                    .expect("read"),
            );
        }
        let seen = arrivals_of(&arrivals);
        assert_eq!(seen.len(), 2);
        assert_eq!(
            seen.first().map(|at| at.duration_since(start)),
            Some(GAP),
            "the first request waits t3.5 (3.5 characters at 9600 8N1) after the open"
        );
        assert_eq!(
            seen.get(1)
                .zip(seen.first())
                .map(|(second, first)| second.duration_since(*first)),
            Some(GAP),
            "the next waits t3.5 after the reply it follows"
        );

        // serial.interframe-delay is a floor above t3.5.
        let delayed = Bench::new(Vec::new());
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        delayed.factory.push_script(Script::new(vec![timed(
            &delayed.clock,
            &arrivals,
            registers(1, &[1]),
        )]));
        let connection = delayed.connect(&format!("{PORT}?serial.interframe-delay=25"));
        let start = delayed.clock.now();
        drop(
            delayed
                .read(&*connection, vec![tag("a", "holding-register:1")])
                .expect("read"),
        );
        assert_eq!(
            arrivals_of(&arrivals)
                .first()
                .map(|at| at.duration_since(start)),
            Some(Duration::from_millis(25))
        );
    }

    // covers: MBS-RTU-SILENCE / bytes heard before a request restart the silence and are dropped
    #[test]
    fn bytes_heard_before_a_request_are_dropped_and_restart_the_silence() {
        let bench = Bench::new(Vec::new());
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        // An unsolicited frame 2 ms after the open, then a device that answers.
        bench.factory.push_script(Script::new(vec![
            Step::Delay(Duration::from_millis(2)),
            Step::Reply(registers(1, &[0xDEAD])),
            timed(&bench.clock, &arrivals, registers(1, &[5])),
        ]));
        let connection = bench.connect(PORT);
        let start = bench.clock.now();
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(
            value(&response, "a"),
            Some(PlcValue::U16(5)),
            "not the stray frame"
        );
        assert_eq!(
            arrivals_of(&arrivals)
                .first()
                .map(|at| at.duration_since(start)),
            Duration::from_millis(2).checked_add(GAP),
            "the silence starts again after the stray frame"
        );
    }

    // covers: MBS-BUS-LATE-REPLY / serial_late_reply_after_timeout_does_not_answer_the_next_request
    // covers: SPEC mandatory-failure-cases / RTU late reply after a timeout
    #[test]
    fn serial_late_reply_after_timeout_does_not_answer_the_next_request() {
        // The device answers the first read 1.1 s after it, past the 1 s timeout, with the
        // same unit and function as the next read would get: only the drain tells them apart.
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::LateReply(Duration::from_millis(1100)),
            Step::Reply(registers(1, &[111])),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[222])),
        ])]);
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=1000"));
        let first = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("no answer within the timeout");
        assert_eq!(first.kind(), ErrorKind::Timeout);
        let timed_out = bench.clock.now();
        let second = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(
            value(&second, "a"),
            Some(PlcValue::U16(222)),
            "the late reply to the first read was drained, not taken"
        );
        assert_eq!(
            bench.clock.now().duration_since(timed_out),
            Duration::from_millis(100)
                .saturating_add(GAP)
                .saturating_add(GAP),
            "the second request waited for the late reply (sent a gap after the first went \
             out, 1.1 s later), dropped it, and went out a gap after it"
        );
    }

    // covers: MBS-BUS-LATE-REPLY / the drain applies over a serial-to-IP gateway
    // covers: MBS-TRANS-TCP / a late reply through a gateway does not answer the next request
    #[test]
    fn gateway_late_reply_after_timeout_does_not_answer_the_next_request() {
        // Over the tcp carrier there is no line to time: no gap, but after an unanswered
        // request the socket is watched for the drain of a 9600 8N1 line, so a late reply
        // still unread in it is dropped.
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::LateReply(Duration::from_millis(1100)),
            Step::Reply(registers(1, &[111])),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[222])),
        ])]);
        let connection = bench.connect("modbus-rtu:tcp://192.0.2.20?request-timeout-ms=1000");
        let first = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("no answer within the timeout");
        assert_eq!(first.kind(), ErrorKind::Timeout);
        let timed_out = bench.clock.now();
        let second = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(
            value(&second, "a"),
            Some(PlcValue::U16(222)),
            "the late reply to the first read was drained, not taken"
        );
        assert_eq!(
            bench.clock.now().duration_since(timed_out),
            Duration::from_millis(100),
            "the second request waited for the late reply (1.1 s after the first went out), \
             dropped it, and went out at once: a gateway has no gap"
        );
    }

    // covers: MBS-BUS-LATE-REPLY / after an unanswered request the line is watched for the drain
    #[test]
    fn after_an_unanswered_request_the_line_is_watched_for_the_drain() {
        let bench = Bench::new(Vec::new());
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        bench.factory.push_script(Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            timed(&bench.clock, &arrivals, registers(1, &[1])),
        ]));
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=1000"));
        let first = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("nothing answers");
        assert_eq!(first.kind(), ErrorKind::Timeout);
        let timed_out = bench.clock.now();
        drop(
            bench
                .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
                .expect("read"),
        );
        assert_eq!(
            arrivals_of(&arrivals)
                .first()
                .map(|at| at.duration_since(timed_out)),
            Some(DRAIN),
            "t3.5 and the time of a 256-byte frame at 9600 8N1"
        );
    }

    // covers: MBS-CFG-REQUEST-TIMEOUT / the watch before a request is part of its deadline
    #[test]
    fn a_deadline_that_ends_inside_the_watch_times_out_and_the_port_is_reopened() {
        // At 100 ms the request timeout is shorter than the drain after the unanswered
        // first read (270 ms at 9600 baud): the second read times out before anything is
        // sent, the port is let go, and the third read reopens it.
        let bench = Bench::new(vec![
            Script::new(vec![Step::Expect(read_registers(1, 0, 1))]),
            Script::strict(vec![
                Step::Expect(read_registers(1, 0, 1)),
                Step::Reply(registers(1, &[4])),
            ]),
        ]);
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=100"));
        for attempt in ["first", "second"] {
            let error = bench
                .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
                .expect_err(attempt);
            assert_eq!(error.kind(), ErrorKind::Timeout, "{attempt}");
        }
        assert_eq!(bench.sent().len(), 1, "the second request never went out");
        let stonecrab::ConnectionState::Reconnecting { next_at, .. } = connection.state() else {
            panic!("the port was let go: {:?}", connection.state());
        };
        assert!(bench.clock.set(next_at), "the backoff is over");
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the port was reopened");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(4)));
        assert_eq!(bench.factory.opens().len(), 2);
    }

    // covers: MBS-RTU-CRC / a reply with a bad CRC fails the read and is dropped
    // covers: SPEC mandatory-failure-cases / RTU CRC errors
    #[test]
    fn a_reply_with_a_bad_crc_fails_the_read_and_the_next_read_is_answered() {
        let mut broken = registers(1, &[42]);
        if let Some(last) = broken.last_mut() {
            *last ^= 0x80;
        }
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(broken),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[43])),
        ])]);
        let connection = bench.connect(PORT);
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("a frame with a bad CRC is no reply");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        assert_eq!(
            error.diagnostics().category,
            Some(FailureCategory::Protocol)
        );
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the bad bytes were dropped before the next request");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(43)));
    }

    // covers: MBS-RTU-RESYNC / a reply split across reads, and two replies in one read
    #[test]
    fn a_reply_split_across_reads_and_two_replies_in_one_read_are_framed() {
        let split = registers(1, &[1, 2]);
        let (head, tail) = split.split_at(3);
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 2)),
            Step::ReplyFragmented(vec![head.to_vec(), tail.to_vec()]),
            Step::Expect(read_registers(1, 0, 2)),
            // A reply of unit 9 and the right reply arrive in one read.
            Step::ReplyCoalescedWithNext,
            Step::Reply(registers(9, &[9, 9])),
            Step::Reply(registers(1, &[3, 4])),
        ])]);
        let connection = bench.connect(PORT);
        for expected in [[1_u16, 2], [3, 4]] {
            let response = bench
                .read(
                    &*connection,
                    vec![
                        tag("a", "holding-register:1:UINT"),
                        tag("b", "holding-register:2:UINT"),
                    ],
                )
                .expect("read");
            assert_eq!(
                (value(&response, "a"), value(&response, "b")),
                (
                    expected.first().copied().map(PlcValue::U16),
                    expected.get(1).copied().map(PlcValue::U16)
                )
            );
        }
    }

    // covers: MBS-BUS-CORRELATION / serial_response_from_another_unit_is_ignored
    // covers: MBS-BUS-CORRELATION / serial_response_with_another_function_is_ignored
    // covers: SPEC mandatory-failure-cases / RTU wrong unit replies
    // covers: MBS-ADDR-UNIT-ID / rtu_tag_unit_id_sets_the_adu_address
    #[test]
    fn serial_response_from_another_unit_or_function_is_ignored() {
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(5, 0, 1)),
            Step::Reply(registers(6, &[600])),
            Step::Delay(Duration::from_millis(10)),
            Step::Reply(frame(5, &[0x04, 0x02, 0x01, 0xF4])),
            Step::Delay(Duration::from_millis(10)),
            Step::Reply(registers(5, &[500])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(
                &*connection,
                vec![tag("a", "holding-register:1:UINT{unit-id: 5}")],
            )
            .expect("read");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(500)));
    }

    // `open` returns the concrete connection, so its counters can be read: the replies of
    // another unit and of another function that the session discarded are mismatches.
    #[test]
    fn rtu_open_returns_a_connection_that_counts_the_replies_it_discarded() {
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(5, 0, 1)),
            Step::Reply(registers(6, &[600])),
            Step::Delay(Duration::from_millis(10)),
            Step::Reply(frame(5, &[0x04, 0x02, 0x01, 0xF4])),
            Step::Delay(Duration::from_millis(10)),
            Step::Reply(registers(5, &[500])),
        ])]);
        let config = bench.driver.parse_config(PORT).expect("config");
        let connection = block_on_with(&bench.clock, bench.driver.open(config)).expect("open");
        assert_eq!(connection.stats().mismatched_replies, 0, "nothing yet");
        let response = bench
            .read(
                &connection,
                vec![tag("a", "holding-register:1:UINT{unit-id: 5}")],
            )
            .expect("read");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(500)));
        let stats = connection.stats();
        assert_eq!(
            stats.mismatched_replies, 2,
            "another unit and another function"
        );
        assert_eq!(stats.late_replies, 0);
        assert_eq!(stats.guard.requests, 1);
    }

    // covers: MBS-BUS-EXCEPTION / rtu_exception_reply_answers_the_pending_request
    #[test]
    fn rtu_exception_reply_answers_the_pending_request() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(frame(1, &[0x83, 0x02])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1")])
            .expect("an exception answers the read");
        let item = response.get(&TagId::new("a").expect("id")).expect("item");
        let error = item.outcome().as_ref().expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
        assert_eq!(error.status().map(stonecrab::ProtocolStatus::code), Some(2));
        assert_eq!(connection.state(), stonecrab::ConnectionState::Connected);
    }

    // covers: MBS-OP-WRITE / rtu_write_of_one_register_uses_function_6
    // covers: MBS-OP-WRITE / rtu_write_of_two_registers_uses_function_16
    #[test]
    fn rtu_writes_use_function_6_for_one_register_and_16_for_two() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(frame(1, &[0x06, 0x00, 0x09, 0x12, 0x34])),
            Step::Reply(frame(1, &[0x06, 0x00, 0x09, 0x12, 0x34])),
            Step::Expect(frame(
                1,
                &[0x10, 0x00, 0x09, 0x00, 0x02, 0x04, 0x00, 0x01, 0x00, 0x02],
            )),
            Step::Reply(frame(1, &[0x10, 0x00, 0x09, 0x00, 0x02])),
        ])]);
        let connection = bench.connect(&format!("{PORT}?access=read-write"));
        let one = bench.write(
            &*connection,
            tag("one", "holding-register:10:UINT"),
            PlcValue::U16(0x1234),
        );
        let two = bench.write(
            &*connection,
            tag("two", "holding-register:10:UDINT"),
            PlcValue::U32(0x0001_0002),
        );
        for response in [one, two] {
            assert!(
                response
                    .results()
                    .iter()
                    .all(|result| matches!(result.outcome(), WriteOutcome::Confirmed)),
                "{response:?}"
            );
        }
    }

    // covers: MB-CFG-SINGLE-COIL, MBS-OP-WRITE / rtu_single_coil_write_fc05_uses_function_5
    #[test]
    fn rtu_single_coil_write_fc05_uses_function_5_and_the_default_stays_function_15() {
        // Coil 20 (wire address 19, 0x13): function 05 with FF00 or 0000 when asked, else the
        // function 15 upstream sends.
        let bench = Bench::new(vec![
            Script::strict(vec![
                Step::Expect(frame(1, &[0x05, 0x00, 0x13, 0xFF, 0x00])),
                Step::Reply(frame(1, &[0x05, 0x00, 0x13, 0xFF, 0x00])),
                Step::Expect(frame(1, &[0x05, 0x00, 0x13, 0x00, 0x00])),
                Step::Reply(frame(1, &[0x05, 0x00, 0x13, 0x00, 0x00])),
                // Two coils stay function 15 under the parameter.
                Step::Expect(frame(1, &[0x0F, 0x00, 0x13, 0x00, 0x02, 0x01, 0x03])),
                Step::Reply(frame(1, &[0x0F, 0x00, 0x13, 0x00, 0x02])),
                // A device that only lists function 05 refuses another with exception 01.
                Step::Expect(frame(1, &[0x05, 0x00, 0x14, 0xFF, 0x00])),
                Step::Reply(frame(1, &[0x85, 0x01])),
                // An echo of the other state is no confirmation.
                Step::Expect(frame(1, &[0x05, 0x00, 0x15, 0xFF, 0x00])),
                Step::Reply(frame(1, &[0x05, 0x00, 0x15, 0x00, 0x00])),
            ]),
            Script::strict(vec![
                Step::Expect(frame(1, &[0x0F, 0x00, 0x13, 0x00, 0x01, 0x01, 0x01])),
                Step::Reply(frame(1, &[0x0F, 0x00, 0x13, 0x00, 0x01])),
            ]),
        ]);
        let on = bench.connect(&format!("{PORT}?access=read-write&single-coil-write=fc05"));
        let list =
            |bits: &[bool]| PlcValue::List(bits.iter().map(|bit| PlcValue::Bool(*bit)).collect());
        let confirmed = |response: &WriteResponse| {
            response
                .results()
                .iter()
                .all(|result| matches!(result.outcome(), WriteOutcome::Confirmed))
        };
        assert!(confirmed(&bench.write(
            &*on,
            tag("a", "coil:20"),
            PlcValue::Bool(true)
        )));
        assert!(confirmed(&bench.write(
            &*on,
            tag("b", "coil:20"),
            PlcValue::Bool(false)
        )));
        assert!(confirmed(&bench.write(
            &*on,
            tag("c", "coil:20[0..1]"),
            list(&[true, true])
        )));
        let refused = bench.write(&*on, tag("d", "coil:21"), PlcValue::Bool(true));
        assert!(
            matches!(
                refused.results().first().map(stonecrab::WriteResult::outcome),
                Some(WriteOutcome::Rejected(error))
                    if error.status().map(stonecrab::ProtocolStatus::code) == Some(1)
            ),
            "{refused:?}"
        );
        let mismatched = bench.write(&*on, tag("e", "coil:22"), PlcValue::Bool(true));
        assert!(
            matches!(
                mismatched
                    .results()
                    .first()
                    .map(stonecrab::WriteResult::outcome),
                Some(WriteOutcome::Unknown(_))
            ),
            "{mismatched:?}"
        );
        // The parameter off, or absent, is function 15.
        let off = bench.connect(&format!("{PORT}?access=read-write"));
        assert!(confirmed(&bench.write(
            &*off,
            tag("f", "coil:20"),
            PlcValue::Bool(true)
        )));
    }

    // covers: MBS-OP-WRITE-EXT / the RTU driver writes extended registers with FC21, cut at
    // the file end as the TCP driver cuts them (MB-ADDR-EXTENDED)
    // covers: MBS-BACKLOG-GO-EXT-SPLIT / an RTU read across a file end joins both
    // sub-responses in order
    #[test]
    fn rtu_reads_and_writes_an_extended_register_across_a_file_end() {
        // Address 9999 is file 1 record 9999; its second register is file 2 record 0.
        let read = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let reply = [0x14, 0x08, 0x03, 0x06, 0x12, 0x34, 0x03, 0x06, 0x56, 0x78];
        let write = [
            0x15, 0x12, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x12, 0x34, 0x06, 0x00, 0x02,
            0x00, 0x00, 0x00, 0x01, 0x56, 0x78,
        ];
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(frame(1, &read)),
            Step::Reply(frame(1, &reply)),
            Step::Expect(frame(1, &write)),
            Step::Reply(frame(1, &write)),
        ])]);
        let connection = bench.connect(&format!("{PORT}?access=read-write"));
        let response = bench
            .read(&*connection, vec![tag("d", "extended-register:9999:DINT")])
            .expect("read");
        assert_eq!(
            value(&response, "d"),
            Some(PlcValue::I32(0x1234_5678)),
            "the high word from file 1, the low word from file 2"
        );
        let written = bench.write(
            &*connection,
            tag("w", "extended-register:9999:DINT"),
            PlcValue::I32(0x1234_5678),
        );
        assert!(
            written
                .results()
                .iter()
                .all(|result| matches!(result.outcome(), WriteOutcome::Confirmed)),
            "{written:?}"
        );
        assert_eq!(bench.sent().len(), 2, "one request each");
    }

    // covers: MBS-OP-READ / a range selection is one RTU request and one list, on the holding
    // and on the extended registers (MB-ADDR-SELECTION)
    #[test]
    fn rtu_reads_a_range_selection_as_one_list() {
        let across = [
            0x14, 0x0E, 0x06, 0x00, 0x01, 0x27, 0x0F, 0x00, 0x01, 0x06, 0x00, 0x02, 0x00, 0x00,
            0x00, 0x01,
        ];
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 3)),
            Step::Reply(registers(1, &[1, 2, 3])),
            Step::Expect(frame(1, &across)),
            Step::Reply(frame(
                1,
                &[0x14, 0x08, 0x03, 0x06, 0x12, 0x34, 0x03, 0x06, 0x56, 0x78],
            )),
        ])]);
        let connection = bench.connect(PORT);
        let holding = bench
            .read(
                &*connection,
                vec![tag("h", "holding-register:1[0..2]:UINT")],
            )
            .expect("read");
        assert_eq!(
            value(&holding, "h"),
            Some(PlcValue::List(vec![
                PlcValue::U16(1),
                PlcValue::U16(2),
                PlcValue::U16(3)
            ]))
        );
        let extended = bench
            .read(
                &*connection,
                vec![tag("e", "extended-register:9999[0..1]:INT")],
            )
            .expect("read");
        assert_eq!(
            value(&extended, "e"),
            Some(PlcValue::List(vec![
                PlcValue::I16(0x1234),
                PlcValue::I16(0x5678)
            ])),
            "one element in each file"
        );
    }

    // covers: MBS-RTU-CRC / a write whose echo fails the CRC is unknown and never replayed
    // covers: SPEC errors / ambiguous write never replayed; MB-WRITE-UNKNOWN
    #[test]
    fn a_write_whose_echo_fails_the_crc_is_unknown_and_never_replayed() {
        // A policy that would retry a malformed reply twice, which it must not do for a write.
        let mut retry = stonecrab::RetryPolicy::default();
        retry.max_attempts = std::num::NonZeroU32::new(3).expect("three");
        retry.retry_on = [ErrorKind::MalformedData, ErrorKind::Timeout]
            .into_iter()
            .collect();
        let config = stonecrab::ConnectionConfig::builder(
            "modbus-rtu",
            Endpoint::Custom("/dev/ttyUSB0".to_owned()),
        )
        .access(stonecrab::AccessPolicy::ReadWrite)
        .retry(retry)
        .build()
        .expect("config");
        let write = frame(1, &[0x06, 0x00, 0x09, 0x12, 0x34]);
        let mut echo = write.clone();
        if let Some(last) = echo.last_mut() {
            *last ^= 0x01;
        }
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(write.clone()),
            Step::Reply(echo),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[5])),
        ])]);
        let connection =
            block_on_with(&bench.clock, bench.driver.connect(config)).expect("connect");
        let response = bench.write(
            &*connection,
            tag("w", "holding-register:10:UINT"),
            PlcValue::U16(0x1234),
        );
        assert!(
            response
                .results()
                .iter()
                .all(|result| matches!(result.outcome(), WriteOutcome::Unknown(_))),
            "{response:?}"
        );
        assert_eq!(bench.sent(), vec![write], "the write went out once");
        let read = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the bad echo was dropped before the next request");
        assert_eq!(value(&read, "a"), Some(PlcValue::U16(5)));
        assert_eq!(bench.sent().len(), 2, "the write, then the read");
    }

    // covers: MBS-OP-PING / rtu_ping_accepts_an_exception_reply
    // covers: MBS-BACKLOG-GO-PING / rtu_ping_accepts_an_exception_reply
    // covers: MBS-CFG-PING-ADDRESS / the default ping address is one register read of address 0
    #[test]
    fn rtu_ping_accepts_an_exception_reply() {
        // The default ping address 4x00001:BOOL is one register read of wire address 0.
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(frame(1, &[0x83, 0x01])),
        ])]);
        let connection = bench.connect(PORT);
        block_on_with(&bench.clock, connection.ping()).expect("an exception proves the peer");
    }

    // covers: MBS-CAP-FLAGS / rtu_capabilities_report_read_write_ping_only
    #[test]
    fn rtu_capabilities_are_those_of_modbus_tcp() {
        let bench = Bench::new(vec![Script::default()]);
        let connection = bench.connect(PORT);
        assert_eq!(bench.driver.scheme(), "modbus-rtu");
        let capabilities = bench.driver.capabilities();
        assert_eq!(capabilities, ModbusTcpDriver::new().capabilities());
        assert_eq!(capabilities, connection.capabilities());
        assert!(capabilities.read && capabilities.write && capabilities.ping);
        assert!(!capabilities.browse && !capabilities.discover);
        assert_eq!(capabilities.subscribe, stonecrab::SubscriptionMode::Polling);
    }

    // covers: MBS-BUS-LATE-REPLY / a serial link lost and reopened through the reconnect path
    // covers: SPEC mandatory-failure-cases / RTU serial disconnect and reopen
    #[test]
    fn a_lost_serial_link_is_disconnected_and_the_next_read_reopens_it() {
        let bench = Bench::new(vec![
            Script::new(vec![
                Step::Expect(read_registers(1, 0, 1)),
                Step::CloseAt(0),
            ]),
            Script::strict(vec![
                Step::Expect(read_registers(1, 0, 1)),
                Step::Reply(registers(1, &[9])),
            ]),
        ]);
        let connection = bench.connect(PORT);
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("the port went away");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the port was reopened");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(9)));
        assert_eq!(bench.factory.opens().len(), 2, "one reopen");
    }

    // covers: MBS-RTU-SILENCE / a peer that closes while the line is watched
    #[test]
    fn a_peer_that_closes_while_the_line_is_watched_is_disconnected() {
        let bench = Bench::new(vec![Script::new(vec![Step::CloseAt(0)]), Script::default()]);
        let connection = bench.connect(PORT);
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("closed before the request went out");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
        assert!(
            bench.sent().is_empty(),
            "nothing was sent into the closed line"
        );
    }

    // covers: MBS-CFG-UNIT-ID / unit 0 is the broadcast address and is refused
    #[test]
    fn unit_0_is_refused_before_any_io() {
        let bench = Bench::new(vec![Script::default()]);
        let refused = bench
            .driver
            .parse_config(&format!("{PORT}?default-unit-identifier=0"))
            .expect_err("no broadcast");
        assert_eq!(refused.kind(), ErrorKind::InvalidConfig);
        let connection = bench.connect(PORT);
        let error = bench
            .read(
                &*connection,
                vec![tag("a", "holding-register:1{unit-id: 0}")],
            )
            .expect_err("no broadcast");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
        assert!(bench.sent().is_empty(), "nothing was sent");
    }

    // covers: MBS-CFG-TRANSPORT-OPTIONS / serial_options_are_read_under_the_serial_prefix
    // covers: MBS-CFG-UNKNOWN / serial_unknown_option_is_rejected_with_invalid_config
    // covers: MBS-QUIRK-STALE-SERIAL-OPTIONS / serial_stale_option_names_are_rejected
    // covers: MBS-CFG-SERIAL-REUSE-PORT / serial_reuse_port_default_is_false (true is refused)
    #[test]
    fn the_connection_string_takes_the_serial_line_under_its_prefix() {
        let driver = ModbusRtuDriver::new();
        for uri in [
            "modbus-rtu:///dev/ttyUSB0?serial.baud-rate=19200&serial.parity=EVEN&serial.stop-bits=2",
            "modbus-rtu:serial://COM3?serial.flow-control=RTS_CTS&serial.data-bits=8",
            "modbus-rtu:///dev/ttyS1?serial.flow-control=xonxoff&serial.reuse-port=false",
            "modbus-rtu:///dev/ttyS1?serial.parity=None&serial.interframe-delay=10&serial.flow-control=none",
            "modbus-rtu:///dev/ttyS1?serial.flow-control=rtscts&serial.parity=odd&serial.stop-bits=1",
            "modbus-rtu:///dev/ttyS1?serial.flow-control=XON-XOFF&default-unit-identifier=247",
        ] {
            let config = driver.parse_config(uri).expect(uri);
            assert_eq!(config.scheme(), "modbus-rtu", "{uri}");
        }
        for uri in [
            "modbus-rtu:///dev/ttyS1?baud-rate=9600",
            "modbus-rtu:///dev/ttyS1?serial.baud-rate=0",
            "modbus-rtu:///dev/ttyS1?serial.baud-rate=fast",
            "modbus-rtu:///dev/ttyS1?serial.data-bits=7",
            "modbus-rtu:///dev/ttyS1?serial.stop-bits=3",
            "modbus-rtu:///dev/ttyS1?serial.parity=mark",
            "modbus-rtu:///dev/ttyS1?serial.parity=EVEN_PARITY",
            "modbus-rtu:///dev/ttyS1?serial.flow-control=dtr",
            "modbus-rtu:///dev/ttyS1?serial.reuse-port=true",
            "modbus-rtu:///dev/ttyS1?serial.reuse-port=yes",
            "modbus-rtu:///dev/ttyS1?serial.interframe-delay=-1",
            "modbus-rtu:///dev/ttyS1?serial.num-data-bits=8",
            "modbus-rtu:///dev/ttyS1?num-stop-bits=1",
            "modbus-rtu:///dev/ttyS1?serial.dtr=true",
            "modbus-rtu:///dev/ttyS1?serial.read-timeout-ms=1000",
            "modbus-rtu:///dev/ttyS1?request-timeout-ms=0",
            "modbus-rtu:///dev/ttyS1?surprise=1",
        ] {
            let error = driver.parse_config(uri).expect_err(uri);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
        }
        let long = format!("{PORT}?serial.{}=1", "x".repeat(60));
        let message = driver.parse_config(&long).expect_err("unknown").to_string();
        assert!(
            message.contains(&format!("serial.{}'", "x".repeat(32))),
            "{message}"
        );
    }

    // covers: MBS-TRANS-TCP / rtu_over_tcp_default_port_is_502
    // covers: MBS-CFG-TRANSPORT-OPTIONS / rtu_over_tcp_tcp_prefixed_option_is_rejected
    // covers: MBS-TRANS-TLS / tls, tls-psk, udp and test carriers are refused
    // covers: MBS-CFG-TRANSPORT-OPTIONS / the tls. keys are refused on the serial and tcp carriers
    // covers: MBS-TRANS-GO-CARRIERS / go_carrier_set_is_a_subset_of_stonecrab_carriers
    #[test]
    fn the_carriers_are_serial_and_tcp() {
        let driver = ModbusRtuDriver::new();
        let gateway = driver
            .parse_config("modbus-rtu:tcp://192.0.2.10")
            .expect("gateway");
        assert_eq!(
            gateway.endpoint(),
            &Endpoint::Tcp {
                host: "192.0.2.10".to_owned(),
                port: 502
            }
        );
        let explicit = driver
            .parse_config("modbus-rtu:tcp://192.0.2.10:4001?request-timeout-ms=250")
            .expect("gateway");
        assert_eq!(explicit.timeouts().request, Duration::from_millis(250));
        for uri in [
            "modbus-rtu:tcp://192.0.2.10?serial.baud-rate=9600",
            "modbus-rtu:tcp://192.0.2.10?tcp.nodelay=true",
            "modbus-rtu:tcp://192.0.2.10:0",
            "modbus-rtu:tls://192.0.2.10",
            "modbus-rtu:tls-psk://192.0.2.10",
            "modbus-rtu:udp://192.0.2.10",
            "modbus-rtu:test://port",
            "modbus-tcp://192.0.2.10",
            "not a connection string",
        ] {
            let error = driver.parse_config(uri).expect_err(uri);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
        }
        // The tls. keys of modbus-tcp:tls are refused on every carrier, in every build: no
        // carrier here is TLS, so accepting them would open plain TCP (ADR-0032, decision 13).
        for uri in [
            "modbus-rtu:tcp://192.0.2.10?tls.trust-store=ca.pem",
            "modbus-rtu:tcp://192.0.2.10?tls.trust-store=ca.pem&tls.keystore=client.pem",
            "modbus-rtu:tcp://192.0.2.10?tls.keystore=client.pem",
            "modbus-rtu:///dev/ttyS1?tls.trust-store=ca.pem",
        ] {
            let error = driver.parse_config(uri).expect_err(uri);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
            assert!(
                error.to_string().contains("modbus-rtu has no TLS carrier"),
                "{uri}: {error}"
            );
        }
        let config = driver
            .parse_config(&format!(
                "{PORT}?address-convention=plc4x&access=read-write"
            ))
            .expect("config");
        assert_eq!(config.address_options().convention(), Some("plc4x"));
        assert_eq!(config.access(), stonecrab::AccessPolicy::ReadWrite);
        assert_eq!(
            format!("{:?}", ModbusRtuDriver::default()),
            format!("{driver:?}")
        );
    }

    // covers: MBS-ADDR-SHARED / rtu_connection_parses_tags_with_the_tcp_grammar
    #[test]
    fn rtu_connection_parses_tags_with_the_tcp_grammar() {
        let options = stonecrab::AddressOptions::default();
        for text in [
            "holding-register:1:REAL",
            "coil:9",
            "input-register:3:DINT{unit-id: 4}",
        ] {
            let rtu = ModbusRtuDriver::new()
                .parse_address(text, &options)
                .expect(text);
            let tcp = ModbusTcpDriver::new()
                .parse_address(text, &options)
                .expect(text);
            assert_eq!(rtu.canonical(), tcp.canonical(), "{text}");
        }
        assert_eq!(
            ModbusRtuDriver::new()
                .parse_address("holding-register:0", &options)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidAddress)
        );
    }

    #[test]
    fn a_hand_built_configuration_connects_with_the_defaults() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[3])),
        ])]);
        let config = stonecrab::ConnectionConfig::builder(
            "modbus-rtu",
            Endpoint::Custom("/dev/ttyS0".to_owned()),
        )
        .build()
        .expect("config");
        let connection =
            block_on_with(&bench.clock, bench.driver.connect(config)).expect("connect");
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(3)));
        let wrong = stonecrab::ConnectionConfig::builder(
            "modbus-tcp",
            Endpoint::Custom("/dev/ttyS0".to_owned()),
        )
        .build()
        .expect("config");
        let refused = block_on_with(&bench.clock, bench.driver.connect(wrong))
            .err()
            .map(|error| error.kind());
        assert_eq!(refused, Some(ErrorKind::InvalidConfig), "another scheme");
    }

    // ---- the real carriers ----

    // covers: MBS-TRANS-TCP / rtu_over_tcp_carries_crc_framed_adus
    #[tokio::test]
    async fn rtu_over_tcp_carries_crc_framed_adus() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[0x0102])),
        ]))
        .expect("peer");
        let driver = ModbusRtuDriver::new();
        let config = driver
            .parse_config(&format!("modbus-rtu:tcp://127.0.0.1:{}", peer.port()))
            .expect("config");
        let connection = driver.connect(config).await.expect("connect");
        let request = ReadRequest::builder(connection.limits())
            .tag(tag("a", "holding-register:1:UINT"))
            .build()
            .expect("request");
        let response = connection.read(request).await.expect("read");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(0x0102)));
        connection.close().await.expect("close");
        let report = peer.close().await.expect("report");
        assert!(report.mismatches.is_empty(), "{report:?}");
    }

    // covers: MBS-TRANS-SERIAL / the serial carrier opens the named port
    #[tokio::test]
    async fn a_missing_serial_port_is_disconnected() {
        let driver = ModbusRtuDriver::new();
        let config = driver
            .parse_config("modbus-rtu:///dev/stonecrab-no-such-port?request-timeout-ms=1000")
            .expect("config");
        let error = driver.connect(config).await.err().expect("no such port");
        assert_eq!(error.kind(), ErrorKind::Disconnected);
    }

    // ---- the contract suite ----

    /// The interop register map the contract device serves.
    fn register_map() -> InteropMap {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/modbus/interop/register-map.toml");
        InteropMap::load(&path).expect("the interop register map loads")
    }

    /// The contract device behind RTU framing: the request's CRC is checked and its unit
    /// and PDU go to the first-party MBAP responder; the answer comes back as an RTU frame.
    fn rtu_reactor(device: &Device) -> Reactor {
        let mut inner = device.reactor();
        Box::new(move |bytes| {
            let Some(body) = bytes
                .len()
                .checked_sub(2)
                .and_then(|end| bytes.get(..end))
                .filter(|body| crc(body).as_slice() == bytes.get(body.len()..).unwrap_or(&[]))
            else {
                return Reaction::Silence;
            };
            let (unit, pdu) = body.split_first().expect("an address");
            let length = u16::try_from(pdu.len().saturating_add(1)).expect("a short PDU");
            let mut mbap = vec![0, 1, 0, 0];
            mbap.extend_from_slice(&length.to_be_bytes());
            mbap.push(*unit);
            mbap.extend_from_slice(pdu);
            let unwrap = |reply: Vec<u8>| {
                frame(
                    reply.get(6).copied().unwrap_or(0),
                    reply.get(7..).unwrap_or(&[]),
                )
            };
            match inner(&mbap) {
                Reaction::Reply(reply) => Reaction::Reply(unwrap(reply)),
                Reaction::ReplyThenClose(reply) => Reaction::ReplyThenClose(unwrap(reply)),
                Reaction::Silence => Reaction::Silence,
                Reaction::Close => Reaction::Close,
            }
        })
    }

    // covers: SPEC test-layers / contract suite against modbus-rtu over the fake transport
    #[test]
    fn the_suite_is_green_against_modbus_rtu_over_the_fake_transport() {
        let clock = Arc::new(FakeClock::default());
        let device = Device::from_map(&register_map());
        let factory = FakeTransportFactory::new(FakeClock::clone(&clock));
        for _ in 0..256 {
            factory.push_script(Script::new(vec![Step::React(rtu_reactor(&device))]));
        }
        let driver = ModbusRtuDriver::new()
            .with_clock(Arc::<FakeClock>::clone(&clock))
            .with_transport_factory(Arc::new(factory.clone()));
        let probe = factory.clone();
        let (counter, timeout, exception, disconnect, unknown) = (
            device.clone(),
            device.clone(),
            device.clone(),
            device.clone(),
            device.clone(),
        );
        let fixture = ContractFixture::new(
            Arc::<FakeClock>::clone(&clock),
            "holding-register:13:UINT",
            "holding-register:60000:UINT",
            Arc::new(FixtureAddress::new("elsewhere", None)),
            move || {
                let frames: usize = probe.probes().iter().map(|p| p.sent().len()).sum();
                u64::try_from(frames.saturating_add(probe.opens().len())).unwrap_or(u64::MAX)
            },
            move || counter.reads(),
        )
        .with_writable("holding-register:100:UINT", PlcValue::U16(1))
        .with_inject_unknown_after_transmit(move || unknown.arm_apply_then_swallow())
        .with_inject_disconnect(move || disconnect.arm_disconnect())
        .with_inject_timeout(move || timeout.arm_swallow())
        .with_inject_device_exception(move || {
            exception.arm_exception(ExceptionCode::ServerDeviceFailure);
        });
        let uri = "modbus-rtu:///dev/ttyUSB0?access=read-write";
        let report: ContractReport =
            block_on_with(&clock, ContractSuite::run(&driver, uri, &fixture))
                .expect("the suite always resolves");
        writeln!(std::io::stdout().lock(), "{uri}\n{report}").expect("stdout");
        assert!(report.is_green(), "the suite is green:\n{report}");
        assert_eq!(report.failed(), 0, "{report}");
        assert_eq!(
            report.passed().saturating_add(report.skipped()),
            ContractSuite::cases().len(),
            "{report}"
        );
        assert_eq!(
            report.skipped(),
            3,
            "the three unsupported-operation cases:\n{report}"
        );
        for name in [
            "a supported ping answers and changes no state",
            "a ping to a silent device yields Timeout",
            "an unknown write outcome is never replayed",
            "a disconnect during a read yields Disconnected",
            "a timeout carries diagnostics naming operation, state and category",
            "read retry stops after the configured maximum attempts",
        ] {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{name}:\n{report}"
            );
        }
        assert_eq!(device.armed(), 0, "every armed fault was used");
        assert_eq!(
            device.applied(),
            2,
            "one confirmed write, one unknown, none replayed"
        );
    }
}
