//! Modbus ASCII (slice S60, ADR-0024 addendum): the framer through the public API, the
//! `modbus-ascii` driver over the fake transport on the fake clock, the failure cases of
//! SPEC.md as they apply to a serial line (LRC errors, a frame split across reads, noise,
//! replies from the wrong unit, a late reply after a timeout, a lost link), the contract
//! suite, and ASCII over a TCP gateway on a loopback socket.
//!
//! The expected frames are built here with an LRC written independently of the crate's (the
//! sum of the bytes, inverted, plus one) and hex written with `format!`, so an
//! encoder and a decoder that agreed on a wrong LRC would not pass. Nothing touches a device
//! and nothing sleeps on real time: the driver runs on `FakeClock` with
//! `FakeTransportFactory`, except for the gateway test (a `ScriptedPeer` on loopback) and the
//! missing-port test (a port name that does not exist).
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
#[path = "support/responder.rs"]
#[expect(
    dead_code,
    reason = "the ASCII suite drives the device's read, write and fault hooks, not its inspection helpers"
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
        AsciiAdu, AsciiFramer, ExceptionCode, FunctionCode, ModbusAsciiDriver, ModbusPdu,
        ModbusRtuDriver, ModbusTcpDriver, UnitId, decode_response,
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
    const PORT: &str = "modbus-ascii:///dev/ttyUSB0";

    /// The silence after a request nothing answered: no gap, and 513 characters of ten
    /// bits at 9600 baud (1 041 666 ns each).
    const DRAIN: Duration = Duration::from_nanos(534_374_658);

    /// The LRC, from the definition: the sum of the bytes, truncated to a byte, inverted, plus
    /// one.
    fn lrc(bytes: &[u8]) -> u8 {
        let sum: u32 = bytes.iter().map(|byte| u32::from(*byte)).sum();
        let low = u8::try_from(sum & 0xFF).expect("a byte");
        (!low).wrapping_add(1)
    }

    /// An ASCII frame: a colon, the hex of the address, the PDU and the LRC, CR LF.
    fn frame(unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut binary = vec![unit];
        binary.extend_from_slice(pdu);
        binary.push(lrc(&binary));
        let mut wire = b":".to_vec();
        for byte in &binary {
            wire.extend_from_slice(format!("{byte:02X}").as_bytes());
        }
        wire.extend_from_slice(b"\r\n");
        wire
    }

    /// The address and the PDU of a well-formed frame, or `None`: the colon, the hex, the LRC
    /// and CR LF checked on their own.
    fn unframe(wire: &[u8]) -> Option<Vec<u8>> {
        let text = std::str::from_utf8(wire).ok()?;
        let digits = text.strip_prefix(':')?.strip_suffix("\r\n")?;
        if digits.len() % 2 != 0 || !digits.is_ascii() {
            return None;
        }
        let binary: Vec<u8> = (0..digits.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(digits.get(at..at.saturating_add(2))?, 16).ok())
            .collect::<Option<_>>()?;
        let (check, body) = binary.split_last()?;
        (lrc(body) == *check).then(|| body.to_vec())
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

    fn hex(text: &str) -> Vec<u8> {
        text.as_bytes()
            .chunks(2)
            .map(|pair| {
                let digits = std::str::from_utf8(pair).expect("ASCII");
                u8::from_str_radix(digits, 16).expect("hex")
            })
            .collect()
    }

    /// The driver on the fake clock over a fake factory with `scripts` queued.
    struct Bench {
        clock: FakeClock,
        factory: FakeTransportFactory,
        driver: ModbusAsciiDriver,
    }

    impl Bench {
        fn new(scripts: Vec<Script>) -> Self {
            let clock = FakeClock::default();
            let factory = FakeTransportFactory::new(clock.clone());
            for script in scripts {
                factory.push_script(script);
            }
            let driver = ModbusAsciiDriver::new()
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
        let address = ModbusAsciiDriver::new()
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

    /// Decode one frame from `bytes` with the public framer: the result and what is left.
    fn decode(bytes: &[u8]) -> (std::result::Result<Option<AsciiAdu>, FrameError>, Vec<u8>) {
        let mut buffer = BytesMut::from(bytes);
        let decoded = AsciiFramer::new().decode(&mut buffer);
        (decoded, buffer.to_vec())
    }

    // ---- framing through the public API ----

    // covers: MBS-ASCII-WIRE-MODEL / ascii_adu_round_trips_the_upstream_vectors
    // covers: MBS-LIMIT-ASCII-BUFFER / ascii_adu_is_address_pdu_lrc
    #[test]
    fn ascii_adu_round_trips_the_upstream_vectors() {
        // Transcribed from Apache PLC4X v1.0.0 (commit ea229918),
        // protocols/modbus/src/test/resources/protocols/modbus/ascii/ParserSerializerTestsuite.xml
        // (the same file is plc4go/assets/testing/protocols/modbus/ascii/ParserSerializerTestsuite.xml):
        // the <raw> hex of "Read Holding Registers Request" (address 1, start 0, quantity 10,
        // crc 242) and of "Read Holding Registers Response" (address 1, byte count 20,
        // twenty zero bytes, crc 232). The <raw> is the binary ADU; on a line it is hex
        // between a colon and CR LF (ModbusAsciiMessageCodec.send).
        let request = hex("01030000000AF2");
        let response = hex("0103140000000000000000000000000000000000000000E8");
        assert_eq!(request.last().copied(), Some(242), "the XML's crc");
        assert_eq!(response.last().copied(), Some(232), "the XML's crc");
        let request_wire = [b":", &b"01030000000AF2"[..], b"\r\n"].concat();
        let response_wire = [
            b":",
            &b"0103140000000000000000000000000000000000000000E8"[..],
            b"\r\n",
        ]
        .concat();

        let mut framer = AsciiFramer::new();
        let adu = AsciiAdu::new(UnitId::from(1), Bytes::from_static(&[0x03, 0, 0, 0, 0x0A]))
            .expect("adu");
        let mut wire = BytesMut::new();
        framer.encode(&adu, &mut wire).expect("encode");
        assert_eq!(
            wire.to_vec(),
            request_wire,
            "the request encodes to the upstream frame"
        );
        assert_eq!(wire.len(), adu.wire_len());

        let mut buffer = BytesMut::from(&response_wire[..]);
        let reply = framer
            .decode(&mut buffer)
            .expect("no error")
            .expect("a frame");
        assert!(buffer.is_empty(), "the whole response was one frame");
        assert_eq!(reply.unit_id(), UnitId::from(1));
        assert_eq!(reply.wire_len(), response_wire.len());
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
            response_wire,
            "the response re-encodes to its frame"
        );
        assert_eq!(
            frame(1, &[0x03, 0, 0, 0, 0x0A]),
            request_wire,
            "the test's own LRC agrees"
        );
        assert_eq!(
            unframe(&response_wire),
            response.split_last().map(|(_, body)| body.to_vec()),
            "the helper reads the frame back without its LRC"
        );
    }

    // covers: MBS-ASCII-DELIMITERS / the upstream codec test's frame ends at its CR LF
    // covers: MBS-ASCII-LRC / the upstream codec test's frame carries a wrong LRC
    #[test]
    fn the_upstream_codec_test_frame_is_sized_by_its_crlf_but_its_lrc_is_wrong() {
        // Transcribed from plc4j/drivers/modbus/src/test/java/org/apache/plc4x/java/modbus/
        // ascii/ModbusAsciiMessageCodecTest.java, testCalculateTotalMessageSize_findsFrameEnd:
        // ":01030000000AFC\r\n" is 17 bytes, and the same text without CR LF is incomplete.
        // That test only sizes the frame: the LRC of 01 03 00 00 00 0A is F2, not FC, so the
        // generated parser upstream, and this framer, refuse the frame.
        let (decoded, rest) = decode(b":01030000000AFC\r\n");
        assert_eq!(decoded, Err(FrameError::Malformed));
        assert_eq!(rest.len(), 17);
        let (decoded, rest) = decode(b":01030000000AFC");
        assert_eq!(decoded, Ok(None), "no CR LF yet: wait");
        assert_eq!(rest.len(), 15);
        let (decoded, _) = decode(b":01030000000AF2\r\n");
        assert!(matches!(decoded, Ok(Some(adu)) if adu.wire_len() == 17));
        assert_eq!(lrc(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x0A]), 0xF2);
    }

    // covers: MBS-LIMIT-ASCII-BUFFER / ascii_buffer_over_513_bytes_without_crlf_desynchronizes
    #[test]
    fn the_upstream_codec_test_buffers_are_waited_for_and_given_up_on() {
        // Transcribed from ModbusAsciiMessageCodecTest.java,
        // testCalculateTotalMessageSize_givesUpPastTheLongestPossibleFrame: 600 bytes of 'A'
        // with no terminator are DESYNCHRONIZED, 400 are waited for. Upstream's buffers hold
        // no colon; the same with a colon in front behaves alike.
        for lead in [&b""[..], b":"] {
            let mut long = lead.to_vec();
            long.resize(600, b'A');
            let (decoded, rest) = decode(&long);
            assert_eq!(decoded, Err(FrameError::Oversized(600)), "{lead:?}");
            assert_eq!(rest, long, "nothing removed");
            let mut plausible = lead.to_vec();
            plausible.resize(400, b'A');
            assert_eq!(decode(&plausible).0, Ok(None), "{lead:?}");
        }
    }

    // covers: MBS-ASCII-PARSE / the responses of the upstream Go codec test frame
    #[test]
    fn the_responses_of_the_upstream_go_codec_test_are_framed() {
        // Transcribed from plc4go/internal/modbus/MessageCodecAscii_test.go,
        // TestMessageCodecAscii_ReceiveFramesAResponse: a read response (address 1, data
        // 00 2A 00 2B), the smallest read frame (address 42, one coil byte), a write echo
        // (address 1, register 7, value 0x1234) and an exception (address 3, illegal data
        // address). The frames are built here, with the independent LRC.
        for (name, unit, pdu) in [
            (
                "a read response",
                1,
                vec![0x03, 0x04, 0x00, 0x2A, 0x00, 0x2B],
            ),
            ("the smallest read frame", 42, vec![0x01, 0x01, 0x01]),
            ("a write echo", 1, vec![0x06, 0x00, 0x07, 0x12, 0x34]),
            ("an exception", 3, vec![0x83, 0x02]),
        ] {
            let wire = frame(unit, &pdu);
            let (decoded, rest) = decode(&wire);
            let adu = decoded.expect(name).expect(name);
            assert_eq!(adu.unit_id(), UnitId::from(unit), "{name}");
            assert_eq!(&adu.pdu()[..], &pdu[..], "{name}");
            assert!(
                rest.is_empty(),
                "{name}: the frame and nothing but the frame"
            );
        }
    }

    // covers: MBS-ASCII-PARSE / ascii_accepts_lower_case_hex
    #[test]
    fn lower_case_hex_is_accepted_and_upper_case_is_written() {
        // Go's codec accepts either case (TestMessageCodecAscii_ReceiveAcceptsLowerCaseHex);
        // Java's Character.digit does too.
        let upper = frame(1, &[0x03, 0x02, 0x00, 0xAB]);
        let lower = upper.to_ascii_lowercase();
        assert_ne!(upper, lower);
        let (decoded, _) = decode(&lower);
        let adu = decoded.expect("no error").expect("a frame");
        assert_eq!(&adu.pdu()[..], &[0x03, 0x02, 0x00, 0xAB]);
        let mut wire = BytesMut::new();
        AsciiFramer::new().encode(&adu, &mut wire).expect("encode");
        assert_eq!(wire.to_vec(), upper, "written in upper case");
    }

    // covers: MBS-ASCII-RESYNC / ascii_desync_resumes_at_the_next_colon
    #[test]
    fn leading_garbage_of_the_upstream_go_codec_test_is_skipped() {
        // Transcribed from MessageCodecAscii_test.go, TestMessageCodecAscii_ReceiveSkipsLeadingGarbage:
        // the tail of a frame, a frame cut off before its terminator, characters that are not
        // hex, an odd number of digits, nothing but colons and zero bytes, each in front of a
        // good frame of address 9.
        let good = frame(9, &[0x03, 0x02, 0x00, 0x2A]);
        for (name, garbage) in [
            ("the tail of a frame we joined late", &b"0304000000\r\n"[..]),
            ("a frame cut off before its terminator", b":010304AB"),
            ("characters that aren't hex", b":hello world\r\n"),
            ("an odd number of hex characters", b":010304A\r\n"),
            ("nothing but colons", b"::::::"),
            ("zeroes", &[0, 0, 0, 0, 0]),
        ] {
            let mut bytes = garbage.to_vec();
            bytes.extend_from_slice(&good);
            let (decoded, rest) = decode(&bytes);
            let adu = decoded.expect(name).expect(name);
            assert_eq!(adu.unit_id(), UnitId::from(9), "{name}");
            assert_eq!(&adu.pdu()[..], &[0x03, 0x02, 0x00, 0x2A], "{name}");
            assert!(rest.is_empty(), "{name}");
        }
    }

    // covers: MBS-ASCII-DELIMITERS / back-to-back frames are two frames, a partial one waits
    #[test]
    fn back_to_back_frames_are_two_and_a_partial_frame_waits() {
        let first = registers(1, &[0x002A]);
        let second = frame(2, &[0x06, 0x00, 0x07, 0x12, 0x34]);
        let mut buffer = BytesMut::from(&[first.clone(), second.clone()].concat()[..]);
        let mut framer = AsciiFramer::new();
        let one = framer.decode(&mut buffer).expect("ok").expect("first");
        assert_eq!(one.unit_id(), UnitId::from(1));
        let two = framer.decode(&mut buffer).expect("ok").expect("second");
        assert_eq!(two.unit_id(), UnitId::from(2));
        assert!(buffer.is_empty());
        // Go's TestMessageCodecAscii_ReceiveWaitsForTheRestOfAFrame: 12 of the 19 bytes.
        let long = registers(1, &[0x002A, 0x002B]);
        assert_eq!(long.len(), 19);
        let (head, tail) = long.split_at(12);
        let mut partial = BytesMut::from(head);
        assert_eq!(framer.decode(&mut partial), Ok(None));
        assert_eq!(partial.len(), 12, "a partial frame is not consumed");
        partial.extend_from_slice(tail);
        assert!(framer.decode(&mut partial).expect("ok").is_some());
    }

    /// Whether `bytes` holds a colon.
    fn has_colon(bytes: &[u8]) -> bool {
        bytes.contains(&b':')
    }

    proptest! {
        // covers: MBS-ASCII-RESYNC / ascii_framer_never_panics_on_random_bytes
        // covers: SPEC mandatory-failure-cases / F1 malformed frames
        #[test]
        fn ascii_framer_never_panics_on_random_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..1300)) {
            let mut framer = AsciiFramer::new();
            let mut buffer = BytesMut::from(&bytes[..]);
            let before = buffer.clone();
            if let Ok(Some(adu)) = framer.decode(&mut buffer) {
                let consumed = before.len().saturating_sub(buffer.len());
                prop_assert!(consumed >= adu.wire_len(), "the frame came out of the buffer");
                prop_assert!(adu.wire_len() <= AsciiAdu::MAX_BYTES, "within the ADU limit");
            } else {
                prop_assert_eq!(buffer, before, "nothing removed");
            }
        }

        // covers: MBS-ASCII-RESYNC / ascii_noise_before_a_frame_is_skipped
        #[test]
        fn ascii_noise_before_a_frame_is_skipped(
            noise in proptest::collection::vec(any::<u8>(), 1..600),
            unit in any::<u8>(),
            values in proptest::collection::vec(any::<u16>(), 1..10),
        ) {
            // A frame starts at a colon: noise without one cannot be mistaken for a frame.
            prop_assume!(!has_colon(&noise));
            let reply = registers(unit, &values);
            let mut buffer = BytesMut::from(&noise[..]);
            buffer.extend_from_slice(&reply);
            let adu = AsciiFramer::new().decode(&mut buffer);
            let adu = adu.expect("no error").expect("the frame behind the noise");
            prop_assert_eq!(adu.unit_id(), UnitId::from(unit));
            prop_assert_eq!(adu.wire_len(), reply.len());
            prop_assert!(buffer.is_empty(), "noise and frame are gone together");
        }

        // covers: MBS-ASCII-WIRE-MODEL / an ADU encodes to a frame the independent helper reads
        #[test]
        fn ascii_adu_round_trips(unit in any::<u8>(), pdu in proptest::collection::vec(any::<u8>(), 1..=253)) {
            let adu = AsciiAdu::new(UnitId::from(unit), Bytes::from(pdu.clone())).expect("adu");
            let mut wire = BytesMut::new();
            AsciiFramer::new().encode(&adu, &mut wire).expect("encode");
            prop_assert_eq!(wire.to_vec(), frame(unit, &pdu), "the independent encoder agrees");
            prop_assert_eq!(wire.len(), adu.wire_len());
            let mut expected = vec![unit];
            expected.extend_from_slice(&pdu);
            prop_assert_eq!(unframe(&wire), Some(expected));
            let back = AsciiFramer::new().decode(&mut wire).expect("ok").expect("a frame");
            prop_assert_eq!(back, adu);
            prop_assert!(wire.is_empty());
        }
    }

    // ---- the driver over the fake transport ----

    // covers: MBS-OP-CONNECT / ascii_connect_opens_the_configured_port_without_sending
    // covers: MBS-TRANS-PORT-NAME / the port name is the authority as written
    #[test]
    fn ascii_connect_opens_the_configured_port_without_sending_and_close_releases_it() {
        let bench = Bench::new(vec![Script::default(), Script::default()]);
        let connection = bench.connect(PORT);
        let opens = bench.factory.opens();
        assert_eq!(
            opens.first().map(|open| open.endpoint.clone()),
            Some(Endpoint::Custom("/dev/ttyUSB0".to_owned())),
            "the port name is the authority as written"
        );
        assert!(bench.sent().is_empty(), "no Modbus traffic on connect");
        block_on_with(&bench.clock, connection.close()).expect("close");
        assert!(
            bench
                .factory
                .probes()
                .iter()
                .all(stonecrab::testkit::FakeTransportProbe::is_shut_down),
            "the port is released"
        );
        let windows = bench.connect("modbus-ascii://COM3");
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

    // covers: MBS-OP-READ / ascii_read_of_holding_registers_decodes_big_endian
    // covers: MBS-TYPE-SHARED / ascii_read_decodes_a_dint_through_the_shared_decoder
    #[test]
    fn ascii_read_of_holding_registers_decodes_through_the_shared_decoder() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 4)),
            Step::Reply(registers(1, &[0x1234, 0x4049, 0x0FDB, 0xFFFE])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(
                &*connection,
                vec![
                    tag("word", "holding-register:1:UINT"),
                    tag("real", "holding-register:2:REAL"),
                    tag("dint", "holding-register:3:DINT"),
                ],
            )
            .expect("read");
        assert_eq!(value(&response, "word"), Some(PlcValue::U16(0x1234)));
        assert_eq!(
            value(&response, "real"),
            Some(PlcValue::F32(f32::from_bits(0x4049_0FDB))),
            "high word first, as BIG_ENDIAN"
        );
        assert_eq!(
            value(&response, "dint"),
            Some(PlcValue::I32(0x0FDB_FFFEu32.cast_signed()))
        );
    }

    // covers: MBS-OP-READ / ascii_read_of_coils_unpacks_bits_lsb_first
    // covers: MBS-BUS-ONE-IN-FLIGHT / two areas are two requests, one after the other
    #[test]
    fn ascii_read_sends_one_request_at_a_time_and_unpacks_coils() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(frame(1, &[0x01, 0x00, 0x00, 0x00, 0x03])),
            Step::Reply(frame(1, &[0x01, 0x01, 0b101])),
            Step::Expect(read_registers(1, 4, 1)),
            Step::Reply(registers(1, &[7])),
        ])]);
        let connection = bench.connect(PORT);
        let response = bench
            .read(
                &*connection,
                vec![
                    tag("c0", "coil:1"),
                    tag("c1", "coil:2"),
                    tag("c2", "coil:3"),
                    tag("word", "holding-register:5"),
                ],
            )
            .expect("read");
        assert_eq!(response.split_into().get(), 2);
        assert_eq!(value(&response, "c0"), Some(PlcValue::Bool(true)));
        assert_eq!(value(&response, "c1"), Some(PlcValue::Bool(false)));
        assert_eq!(value(&response, "c2"), Some(PlcValue::Bool(true)));
        assert_eq!(value(&response, "word"), Some(PlcValue::I16(7)));
    }

    // covers: MBS-ASCII-TIMING / ascii_waits_for_no_silence_before_a_request
    // covers: MBS-CFG-SERIAL-INTERFRAME / the gap is serial.interframe-delay and nothing else
    #[test]
    fn a_request_goes_out_at_once_unless_an_interframe_delay_asks_for_a_gap() {
        let bench = Bench::new(Vec::new());
        let arrivals = Arc::new(Mutex::new(Vec::new()));
        bench.factory.push_script(Script::new(vec![timed(
            &bench.clock,
            &arrivals,
            registers(1, &[1]),
        )]));
        let connection = bench.connect(PORT);
        let start = bench.clock.now();
        for _ in 0..2 {
            drop(
                bench
                    .read(&*connection, vec![tag("a", "holding-register:1")])
                    .expect("read"),
            );
        }
        let seen = arrivals_of(&arrivals);
        assert_eq!(seen.len(), 2);
        assert!(
            seen.iter().all(|at| *at == start),
            "no t3.5 for ASCII: both requests go out the moment they are asked"
        );

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

    // covers: MBS-ASCII-TIMING / ascii_slow_bytes_are_accepted_until_the_request_deadline
    #[test]
    fn slow_characters_are_accepted_until_the_request_deadline() {
        // 1.5 s between pieces of the reply is longer than the 1 s the serial line
        // specification allows between characters; upstream has no such timeout and neither
        // has Stonecrab: the request deadline (5 s by default) is the only bound.
        let reply = registers(1, &[42]);
        let pieces: Vec<Vec<u8>> = reply.chunks(5).map(<[u8]>::to_vec).collect();
        assert_eq!(pieces.len(), 3);
        let steps = || {
            let mut steps = vec![Step::Expect(read_registers(1, 0, 1))];
            for piece in &pieces {
                steps.push(Step::Delay(Duration::from_millis(1500)));
                steps.push(Step::Reply(piece.clone()));
            }
            steps
        };
        let bench = Bench::new(vec![Script::strict(steps())]);
        let connection = bench.connect(PORT);
        let start = bench.clock.now();
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("a slow reply within the deadline");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(42)));
        assert_eq!(
            bench.clock.now().duration_since(start),
            Duration::from_millis(4500)
        );

        // The same reply under a 3 s deadline: the second piece comes too late.
        let bench = Bench::new(vec![Script::strict(steps())]);
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=3000"));
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("the deadline ends the wait");
        assert_eq!(error.kind(), ErrorKind::Timeout);
    }

    // covers: MBS-ASCII-TIMING / a frame without its LF never completes: the deadline ends it
    // covers: MBS-ASCII-DELIMITERS / a missing CR or LF is no frame
    #[test]
    fn a_reply_missing_its_cr_or_lf_times_out_and_the_next_read_is_answered() {
        let whole = registers(1, &[42]);
        let (without_lf, _) = whole.split_at(whole.len() - 1);
        let (without_crlf, _) = whole.split_at(whole.len() - 2);
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(without_lf.to_vec()),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(without_crlf.to_vec()),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[43])),
        ])]);
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=1000"));
        for attempt in ["without the LF", "without CR and LF"] {
            let error = bench
                .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
                .expect_err(attempt);
            assert_eq!(error.kind(), ErrorKind::Timeout, "{attempt}");
        }
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the incomplete frames were dropped before the next request");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(43)));
    }

    // covers: MBS-ASCII-DELIMITERS / an LF without its CR is malformed at once
    #[test]
    fn a_reply_with_an_lf_but_no_cr_is_malformed_and_the_next_read_is_answered() {
        let mut broken = registers(1, &[42]);
        broken.retain(|byte| *byte != b'\r');
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(broken),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[43])),
        ])]);
        let connection = bench.connect(PORT);
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("an LF without its CR is no frame end");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the bad bytes were dropped before the next request");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(43)));
    }

    // covers: MBS-BUS-LATE-REPLY / ascii_late_reply_after_timeout_does_not_answer_the_next_request
    // covers: SPEC mandatory-failure-cases / serial late reply after a timeout
    #[test]
    fn a_late_reply_after_a_timeout_does_not_answer_the_next_request() {
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
            Duration::from_millis(100),
            "the second request waited for the late reply (1.1 s after the first went out), \
             dropped it, and went out at once: ASCII has no gap"
        );
    }

    // covers: MBS-BUS-LATE-REPLY / the drain applies over a serial-to-IP gateway
    // covers: MBS-TRANS-TCP / a late reply through a gateway does not answer the next request
    #[test]
    fn a_gateway_drains_a_late_reply_too() {
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::LateReply(Duration::from_millis(1100)),
            Step::Reply(registers(1, &[111])),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[222])),
        ])]);
        let connection = bench.connect("modbus-ascii:tcp://192.0.2.20?request-timeout-ms=1000");
        let first = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("no answer within the timeout");
        assert_eq!(first.kind(), ErrorKind::Timeout);
        let second = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(value(&second, "a"), Some(PlcValue::U16(222)));
    }

    // covers: MBS-BUS-LATE-REPLY / after an unanswered request the line is watched for the drain
    #[test]
    fn after_an_unanswered_request_the_line_is_watched_for_513_characters() {
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
            "the time of the largest frame, 513 characters at 9600 8N1"
        );
    }

    // covers: MBS-ASCII-LRC / ascii_frame_with_a_bad_lrc_is_rejected
    // covers: SPEC mandatory-failure-cases / serial LRC errors
    #[test]
    fn a_reply_with_a_bad_lrc_fails_the_read_and_the_next_read_is_answered() {
        let mut broken = registers(1, &[42]);
        let at = broken.len() - 3;
        if let Some(digit) = broken.get_mut(at) {
            *digit = if *digit == b'0' { b'1' } else { b'0' };
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
            .expect_err("a frame with a bad LRC is no reply");
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

    // covers: MBS-ASCII-PARSE / a reply with a character that is no hex digit
    #[test]
    fn a_reply_with_a_non_hex_character_fails_the_read_and_the_next_read_is_answered() {
        let mut broken = registers(1, &[42]);
        if let Some(digit) = broken.get_mut(5) {
            *digit = b'G';
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
            .expect_err("a frame with a character that is no digit is no reply");
        assert_eq!(error.kind(), ErrorKind::MalformedData);
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the bad bytes were dropped before the next request");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(43)));
    }

    // covers: MBS-LIMIT-ASCII-BUFFER / a reply that runs past the largest frame is dropped
    #[test]
    fn a_reply_longer_than_any_frame_fails_the_read_and_the_next_read_is_answered() {
        let mut endless = b":".to_vec();
        endless.resize(600, b'A');
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(endless),
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[43])),
        ])]);
        let connection = bench.connect(PORT);
        let error = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect_err("no frame is that long");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("the run was dropped before the next request");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(43)));
    }

    // covers: MBS-ASCII-DELIMITERS / ascii_partial_frame_asks_for_more_data
    // covers: MBS-ASCII-RESYNC / noise before the colon, a reply split across reads and two
    // replies in one read
    #[test]
    fn noise_before_the_colon_a_split_reply_and_two_replies_in_one_read_are_framed() {
        let split = registers(1, &[1, 2]);
        let (head, tail) = split.split_at(7);
        let mut noisy = b"\x00\xFFnoise\r\n".to_vec();
        noisy.extend_from_slice(&registers(1, &[5, 6]));
        let bench = Bench::new(vec![Script::new(vec![
            Step::Expect(read_registers(1, 0, 2)),
            Step::ReplyFragmented(vec![head.to_vec(), tail.to_vec()]),
            Step::Expect(read_registers(1, 0, 2)),
            // A reply of unit 9 and the right reply arrive in one read.
            Step::ReplyCoalescedWithNext,
            Step::Reply(registers(9, &[9, 9])),
            Step::Reply(registers(1, &[3, 4])),
            Step::Expect(read_registers(1, 0, 2)),
            Step::Reply(noisy),
        ])]);
        let connection = bench.connect(PORT);
        for expected in [[1_u16, 2], [3, 4], [5, 6]] {
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

    // covers: MBS-BUS-CORRELATION / ascii_response_from_another_unit_is_ignored
    // covers: MBS-BUS-CORRELATION / ascii_response_with_another_function_is_ignored
    // covers: MBS-ADDR-UNIT-ID / ascii_tag_unit_id_sets_the_adu_address
    // covers: SPEC mandatory-failure-cases / serial wrong unit replies
    #[test]
    fn ascii_response_from_another_unit_or_function_is_ignored() {
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

    // covers: MBS-BUS-EXCEPTION / ascii_exception_reply_answers_the_pending_request
    #[test]
    fn ascii_exception_reply_answers_the_pending_request() {
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

    // covers: MBS-OP-WRITE / ascii_write_of_one_register_uses_function_6
    // covers: MBS-OP-WRITE / ascii_write_of_two_registers_uses_function_16
    #[test]
    fn ascii_writes_use_function_6_for_one_register_and_16_for_two() {
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

    // covers: MBS-OP-WRITE-EXT / the ASCII driver writes extended registers with FC21, cut at
    // the file end as the TCP driver cuts them (MB-ADDR-EXTENDED)
    // covers: MBS-BACKLOG-GO-EXT-SPLIT / an ASCII read across a file end joins both
    // sub-responses in order
    #[test]
    fn ascii_reads_and_writes_an_extended_register_across_a_file_end() {
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

    // covers: MBS-CFG-SERIAL-DATA-BITS / ascii_accepts_7_data_bits_with_even_parity
    #[test]
    fn a_seven_bit_even_parity_line_connects_and_reads() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(3, 0, 1)),
            Step::Reply(registers(3, &[9])),
        ])]);
        let connection = bench.connect(&format!(
            "{PORT}?serial.data-bits=7&serial.parity=even&serial.stop-bits=1\
             &default-unit-identifier=3"
        ));
        let response = bench
            .read(&*connection, vec![tag("a", "holding-register:1:UINT")])
            .expect("read");
        assert_eq!(value(&response, "a"), Some(PlcValue::U16(9)));
    }

    // covers: MBS-OP-PING / ascii_ping_accepts_an_exception_reply
    // covers: MBS-OP-PING / ascii_ping_times_out_when_the_line_is_silent
    // covers: MBS-CFG-PING-ADDRESS / the default ping address is one register read of address 0
    #[test]
    fn ascii_ping_accepts_an_exception_reply_and_times_out_on_silence() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(frame(1, &[0x83, 0x01])),
            Step::Expect(read_registers(1, 0, 1)),
        ])]);
        let connection = bench.connect(&format!("{PORT}?request-timeout-ms=500"));
        block_on_with(&bench.clock, connection.ping()).expect("an exception proves the peer");
        let error = block_on_with(&bench.clock, connection.ping()).expect_err("silence");
        assert_eq!(error.kind(), ErrorKind::Timeout);
    }

    // covers: MBS-CAP-FLAGS / ascii_capabilities_report_read_write_ping_only
    #[test]
    fn ascii_capabilities_are_those_of_modbus_tcp() {
        let bench = Bench::new(vec![Script::default()]);
        let connection = bench.connect(PORT);
        assert_eq!(bench.driver.scheme(), "modbus-ascii");
        let capabilities = bench.driver.capabilities();
        assert_eq!(capabilities, ModbusTcpDriver::new().capabilities());
        assert_eq!(capabilities, ModbusRtuDriver::new().capabilities());
        assert_eq!(capabilities, connection.capabilities());
        assert!(capabilities.read && capabilities.write && capabilities.ping);
        assert!(!capabilities.browse && !capabilities.discover);
        assert_eq!(capabilities.subscribe, stonecrab::SubscriptionMode::Polling);
    }

    // covers: MBS-BUS-LATE-REPLY / a serial link lost and reopened through the reconnect path
    // covers: SPEC mandatory-failure-cases / serial disconnect and reopen
    #[test]
    fn ascii_lost_serial_link_is_disconnected_and_the_next_read_reopens_it() {
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

    // covers: MBS-CFG-UNIT-ID / unit 0 is the broadcast address and is refused
    #[test]
    fn ascii_unit_0_is_refused_before_any_io() {
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

    // covers: MBS-CFG-TRANSPORT-OPTIONS / the serial options are read under the serial prefix
    // covers: MBS-CFG-UNKNOWN / an unknown serial option is InvalidConfig
    // covers: MBS-CFG-SERIAL-REUSE-PORT / reuse-port=true is refused
    #[test]
    fn ascii_connection_string_takes_the_serial_line_under_its_prefix() {
        let driver = ModbusAsciiDriver::new();
        for uri in [
            "modbus-ascii:///dev/ttyUSB0?serial.baud-rate=19200&serial.parity=EVEN&serial.stop-bits=2",
            "modbus-ascii:serial://COM3?serial.flow-control=RTS_CTS&serial.data-bits=8",
            "modbus-ascii:///dev/ttyS1?serial.data-bits=7&serial.parity=even&serial.stop-bits=1",
            "modbus-ascii:///dev/ttyS1?serial.flow-control=xonxoff&serial.reuse-port=false",
            "modbus-ascii:///dev/ttyS1?serial.parity=None&serial.interframe-delay=10",
            "modbus-ascii:///dev/ttyS1?default-unit-identifier=247",
        ] {
            let config = driver.parse_config(uri).expect(uri);
            assert_eq!(config.scheme(), "modbus-ascii", "{uri}");
        }
        for uri in [
            "modbus-ascii:///dev/ttyS1?baud-rate=9600",
            "modbus-ascii:///dev/ttyS1?serial.baud-rate=0",
            "modbus-ascii:///dev/ttyS1?serial.data-bits=6",
            "modbus-ascii:///dev/ttyS1?serial.data-bits=5",
            "modbus-ascii:///dev/ttyS1?serial.stop-bits=3",
            "modbus-ascii:///dev/ttyS1?serial.parity=space",
            "modbus-ascii:///dev/ttyS1?serial.parity=EVEN_PARITY",
            "modbus-ascii:///dev/ttyS1?serial.reuse-port=true",
            "modbus-ascii:///dev/ttyS1?serial.num-data-bits=8",
            "modbus-ascii:///dev/ttyS1?serial.dtr=true",
            "modbus-ascii:///dev/ttyS1?serial.read-timeout-ms=1000",
            "modbus-ascii:///dev/ttyS1?request-timeout-ms=0",
            "modbus-ascii:///dev/ttyS1?surprise=1",
        ] {
            let error = driver.parse_config(uri).expect_err(uri);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
        }
        // An RTU line is eight data bits, an ASCII line may be seven.
        let rtu = ModbusRtuDriver::new()
            .parse_config("modbus-rtu:///dev/ttyS1?serial.data-bits=7")
            .expect_err("RTU has eight data bits");
        assert_eq!(rtu.kind(), ErrorKind::InvalidConfig);
    }

    // covers: MBS-TRANS-TCP / ascii_over_tcp_default_port_is_502
    // covers: MBS-TRANS-TLS / tls, tls-psk, udp and test carriers are refused
    // covers: MBS-TRANS-GO-CARRIERS / the carriers are serial and tcp
    #[test]
    fn ascii_carriers_are_serial_and_tcp() {
        let driver = ModbusAsciiDriver::new();
        let gateway = driver
            .parse_config("modbus-ascii:tcp://192.0.2.10")
            .expect("gateway");
        assert_eq!(
            gateway.endpoint(),
            &Endpoint::Tcp {
                host: "192.0.2.10".to_owned(),
                port: 502
            }
        );
        let explicit = driver
            .parse_config("modbus-ascii:tcp://192.0.2.10:4001?request-timeout-ms=250")
            .expect("gateway");
        assert_eq!(explicit.timeouts().request, Duration::from_millis(250));
        for uri in [
            "modbus-ascii:tcp://192.0.2.10?serial.baud-rate=9600",
            "modbus-ascii:tcp://192.0.2.10?serial.data-bits=7",
            "modbus-ascii:tcp://192.0.2.10:0",
            "modbus-ascii:tls://192.0.2.10",
            "modbus-ascii:tls-psk://192.0.2.10",
            "modbus-ascii:udp://192.0.2.10",
            "modbus-ascii:test://port",
            "modbus-tcp://192.0.2.10",
            "modbus-rtu:///dev/ttyS0",
            "not a connection string",
        ] {
            let error = driver.parse_config(uri).expect_err(uri);
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
        }
        let config = driver
            .parse_config(&format!(
                "{PORT}?address-convention=plc4x&access=read-write"
            ))
            .expect("config");
        assert_eq!(config.address_options().convention(), Some("plc4x"));
        assert_eq!(config.access(), stonecrab::AccessPolicy::ReadWrite);
        assert_eq!(
            format!("{:?}", ModbusAsciiDriver::default()),
            format!("{driver:?}")
        );
        assert!(
            ModbusRtuDriver::new().parse_config(PORT).is_err(),
            "the RTU driver does not take a modbus-ascii string"
        );
    }

    // covers: MBS-ADDR-SHARED / ascii_connection_parses_tags_with_the_tcp_grammar
    #[test]
    fn ascii_connection_parses_tags_with_the_tcp_grammar() {
        let options = stonecrab::AddressOptions::default();
        for text in [
            "holding-register:1:REAL",
            "coil:9",
            "input-register:3:DINT{unit-id: 4}",
        ] {
            let ascii = ModbusAsciiDriver::new()
                .parse_address(text, &options)
                .expect(text);
            let tcp = ModbusTcpDriver::new()
                .parse_address(text, &options)
                .expect(text);
            assert_eq!(ascii.canonical(), tcp.canonical(), "{text}");
        }
        assert_eq!(
            ModbusAsciiDriver::new()
                .parse_address("holding-register:0", &options)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidAddress)
        );
    }

    #[test]
    fn ascii_hand_built_configuration_connects_with_the_defaults() {
        let bench = Bench::new(vec![Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[3])),
        ])]);
        let config = stonecrab::ConnectionConfig::builder(
            "modbus-ascii",
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
            "modbus-rtu",
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

    // covers: MBS-TRANS-TCP / ascii_over_tcp_carries_lrc_framed_adus
    #[tokio::test]
    async fn ascii_over_tcp_carries_lrc_framed_adus() {
        let peer = ScriptedPeer::spawn(Script::strict(vec![
            Step::Expect(read_registers(1, 0, 1)),
            Step::Reply(registers(1, &[0x0102])),
        ]))
        .expect("peer");
        let driver = ModbusAsciiDriver::new();
        let config = driver
            .parse_config(&format!("modbus-ascii:tcp://127.0.0.1:{}", peer.port()))
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
    async fn ascii_missing_serial_port_is_disconnected() {
        let driver = ModbusAsciiDriver::new();
        let config = driver
            .parse_config("modbus-ascii:///dev/stonecrab-no-such-port?request-timeout-ms=1000")
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

    /// The contract device behind ASCII framing: the request's hex and LRC are checked and its
    /// unit and PDU go to the first-party MBAP responder; the answer comes back as an ASCII
    /// frame.
    fn ascii_reactor(device: &Device) -> Reactor {
        let mut inner = device.reactor();
        Box::new(move |bytes| {
            let Some(binary) = unframe(bytes) else {
                return Reaction::Silence;
            };
            let (unit, pdu) = binary.split_first().expect("an address");
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

    // covers: SPEC test-layers / contract suite against modbus-ascii over the fake transport
    #[test]
    fn the_suite_is_green_against_modbus_ascii_over_the_fake_transport() {
        let clock = Arc::new(FakeClock::default());
        let device = Device::from_map(&register_map());
        let factory = FakeTransportFactory::new(FakeClock::clone(&clock));
        for _ in 0..256 {
            factory.push_script(Script::new(vec![Step::React(ascii_reactor(&device))]));
        }
        let driver = ModbusAsciiDriver::new()
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
        let uri = "modbus-ascii:///dev/ttyUSB0?access=read-write";
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
