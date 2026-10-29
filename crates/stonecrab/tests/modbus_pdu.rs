//! The PDU codec: the coding standards section 13.7 list for each decode function, the code tables,
//! the encoders and the reply check.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::PlcError;
    use stonecrab::codec::{BoundedReader, BoundedWriter, ParseError};
    use stonecrab::modbus::{
        ExceptionCode, FileRecord, FileRecordRead, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS,
        MAX_READ_EXTENDED_REGISTERS, MAX_READ_REGISTERS, ModbusArea, ModbusPdu, decode_request,
        decode_response, encode_request, encode_response,
    };

    const COILS: FunctionCode = FunctionCode::ReadCoils;
    const INPUTS: FunctionCode = FunctionCode::ReadDiscreteInputs;
    const HOLDING: FunctionCode = FunctionCode::ReadHoldingRegisters;
    const INPUT_REGISTERS: FunctionCode = FunctionCode::ReadInputRegisters;
    const READS: [FunctionCode; 4] = [COILS, INPUTS, HOLDING, INPUT_REGISTERS];
    const WRITE_ONE: FunctionCode = FunctionCode::WriteSingleRegister;
    const WRITE_COILS: FunctionCode = FunctionCode::WriteMultipleCoils;
    const WRITE_REGISTERS: FunctionCode = FunctionCode::WriteMultipleRegisters;
    const WRITES: [FunctionCode; 3] = [WRITE_ONE, WRITE_COILS, WRITE_REGISTERS];
    const READ_FILE: FunctionCode = FunctionCode::ReadFileRecord;
    const WRITE_FILE: FunctionCode = FunctionCode::WriteFileRecord;
    /// The ten exception codes V1.1b section 7 defines.
    const DEFINED: [u8; 10] = [1, 2, 3, 4, 5, 6, 7, 8, 10, 11];

    fn request(bytes: &[u8]) -> Result<ModbusPdu, ParseError> {
        decode_request(&mut BoundedReader::new(bytes))
    }

    fn response(bytes: &[u8], expected: FunctionCode) -> Result<ModbusPdu, ParseError> {
        decode_response(&mut BoundedReader::new(bytes), expected)
    }

    fn invalid(field: &'static str, value: u64) -> ParseError {
        ParseError::InvalidDiscriminator { field, value }
    }

    fn oversized(length: usize, max: usize) -> ParseError {
        ParseError::OversizedLength { length, max }
    }

    fn encoded_request(pdu: &ModbusPdu) -> Result<Vec<u8>, ParseError> {
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_request(pdu, &mut writer)?;
        Ok(writer.into_bytes())
    }

    fn encoded_response(pdu: &ModbusPdu) -> Result<Vec<u8>, ParseError> {
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        encode_response(pdu, &mut writer)?;
        Ok(writer.into_bytes())
    }

    fn bits_response(function: FunctionCode, bytes: &[u8]) -> ModbusPdu {
        ModbusPdu::ReadBitsResponse {
            function,
            bytes: bytes.to_vec(),
        }
    }

    fn registers_response(function: FunctionCode, registers: &[u16]) -> ModbusPdu {
        ModbusPdu::ReadRegistersResponse {
            function,
            registers: registers.to_vec(),
        }
    }

    /// A normal response of each function and its bytes.
    fn valid_responses() -> Vec<(FunctionCode, Vec<u8>)> {
        vec![
            (COILS, vec![0x01, 0x03, 0xCD, 0x6B, 0x05]),
            (INPUTS, vec![0x02, 0x01, 0xFF]),
            (HOLDING, vec![0x03, 0x04, 0x00, 0x01, 0xFF, 0xFF]),
            (INPUT_REGISTERS, vec![0x04, 0x02, 0x00, 0x0A]),
        ]
    }

    // covers: MB-TCP-READ-COIL, MB-TCP-READ-DI, MB-TCP-READ-HR, MB-TCP-READ-IR
    // covers: MB-TCP-WRITE, MB-QUIRK-C, MB-ADDR-EXTENDED
    #[test]
    fn function_codes_round_trip_and_name_only_the_reads_writes_and_file_records() {
        for value in 0..=u8::MAX {
            let code = FunctionCode::from_u8(value);
            assert_eq!(code.as_u8(), value, "function {value:#04x} round trip");
            assert_eq!(
                code.exception_code(),
                value | 0x80,
                "exception byte of {value:#04x}"
            );
            let named = matches!(value, 1..=4 | 6 | 15 | 16 | 20 | 21);
            assert_eq!(
                !matches!(code, FunctionCode::Unknown(_)),
                named,
                "function {value:#04x} named"
            );
        }
        assert_eq!(FunctionCode::from_u8(1), COILS, "0x01");
        assert_eq!(FunctionCode::from_u8(2), INPUTS, "0x02");
        assert_eq!(FunctionCode::from_u8(3), HOLDING, "0x03");
        assert_eq!(FunctionCode::from_u8(4), INPUT_REGISTERS, "0x04");
        assert_eq!(FunctionCode::from_u8(6), WRITE_ONE, "0x06");
        assert_eq!(FunctionCode::from_u8(15), WRITE_COILS, "0x0F");
        assert_eq!(FunctionCode::from_u8(16), WRITE_REGISTERS, "0x10");
        assert_eq!(FunctionCode::from_u8(20), READ_FILE, "0x14");
        assert_eq!(FunctionCode::from_u8(21), WRITE_FILE, "0x15");
        assert_eq!(READ_FILE.exception_code(), 0x94, "exception byte of FC20");
        assert_eq!(WRITE_FILE.exception_code(), 0x95, "exception byte of FC21");
        assert_eq!(
            FunctionCode::from_u8(5),
            FunctionCode::Unknown(5),
            "write single coil is not emitted by upstream"
        );
        assert_eq!(COILS.exception_code(), 0x81, "exception byte of FC01");
        assert_eq!(WRITE_COILS.exception_code(), 0x8F, "exception byte of FC15");
    }

    // covers: MB-EXC-01, MB-EXC-02, MB-EXC-03, MB-EXC-04, MB-EXC-05, MB-EXC-06, MB-EXC-07,
    // covers: MB-EXC-08, MB-EXC-0A, MB-EXC-0B, MB-EXC-UNKNOWN
    #[test]
    fn exception_codes_round_trip_and_name_the_ten_defined_values() {
        let table = [
            (1, ExceptionCode::IllegalFunction, "ILLEGAL FUNCTION"),
            (2, ExceptionCode::IllegalDataAddress, "ILLEGAL DATA ADDRESS"),
            (3, ExceptionCode::IllegalDataValue, "ILLEGAL DATA VALUE"),
            (
                4,
                ExceptionCode::ServerDeviceFailure,
                "SERVER DEVICE FAILURE",
            ),
            (5, ExceptionCode::Acknowledge, "ACKNOWLEDGE"),
            (6, ExceptionCode::ServerDeviceBusy, "SERVER DEVICE BUSY"),
            (
                7,
                ExceptionCode::NegativeAcknowledge,
                "NEGATIVE ACKNOWLEDGE",
            ),
            (8, ExceptionCode::MemoryParityError, "MEMORY PARITY ERROR"),
            (
                10,
                ExceptionCode::GatewayPathUnavailable,
                "GATEWAY PATH UNAVAILABLE",
            ),
            (
                11,
                ExceptionCode::GatewayTargetDeviceFailedToRespond,
                "GATEWAY TARGET DEVICE FAILED TO RESPOND",
            ),
        ];
        for (value, code, name) in table {
            assert_eq!(ExceptionCode::from_u8(value), code, "code {value}");
            assert_eq!(code.as_u8(), value, "number of {name}");
            assert_eq!(code.name(), name, "name of code {value}");
        }
        for value in 0..=u8::MAX {
            let code = ExceptionCode::from_u8(value);
            assert_eq!(code.as_u8(), value, "code {value} round trip");
            if !DEFINED.contains(&value) {
                assert_eq!(
                    code,
                    ExceptionCode::Unknown(value),
                    "code {value} is unknown"
                );
                assert_eq!(code.name(), "UNKNOWN", "name of unknown code {value}");
            }
        }
    }

    // ------------------------------------------------------------------ decode_request

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn request_of_zero_length_is_truncated() {
        assert_eq!(
            request(&[]),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            }),
            "empty request"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn request_one_byte_short_is_truncated_at_every_length() {
        for function in READS {
            let full = [function.as_u8(), 0x00, 0x6B, 0x00, 0x03];
            for len in 1..full.len() {
                let short = full.get(..len).expect("prefix of a five byte request");
                let error = request(short).expect_err("a short request must fail");
                assert!(
                    matches!(error, ParseError::Truncated { .. }),
                    "function {function:?} length {len}: {error:?}"
                );
            }
        }
    }

    #[test]
    fn request_of_exact_length_decodes_for_each_function() {
        let want = [
            (
                0x01,
                ModbusPdu::ReadCoils {
                    start: 107,
                    quantity: 3,
                },
            ),
            (
                0x02,
                ModbusPdu::ReadDiscreteInputs {
                    start: 107,
                    quantity: 3,
                },
            ),
            (
                0x03,
                ModbusPdu::ReadHoldingRegisters {
                    start: 107,
                    quantity: 3,
                },
            ),
            (
                0x04,
                ModbusPdu::ReadInputRegisters {
                    start: 107,
                    quantity: 3,
                },
            ),
        ];
        for (function, pdu) in want {
            assert_eq!(
                request(&[function, 0x00, 0x6B, 0x00, 0x03]),
                Ok(pdu),
                "function {function:#04x}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn request_one_byte_long_has_trailing_bytes() {
        assert_eq!(
            request(&[0x03, 0x00, 0x6B, 0x00, 0x03, 0x00]),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "one trailing byte"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    // covers: MB-LIMIT-COILS, MB-LIMIT-REGISTERS
    #[test]
    fn request_quantity_bounds_1_to_125_and_2000() {
        let quantity = |function: u8, quantity: u16| {
            let [hi, lo] = quantity.to_be_bytes();
            request(&[function, 0x00, 0x00, hi, lo])
        };
        for function in [0x01, 0x02] {
            assert!(quantity(function, 1).is_ok(), "{function:#04x} quantity 1");
            assert!(
                quantity(function, 2000).is_ok(),
                "{function:#04x} quantity 2000"
            );
            assert_eq!(quantity(function, 2001), Err(oversized(2001, 2000)), "2001");
            assert_eq!(
                quantity(function, u16::MAX),
                Err(oversized(65535, 2000)),
                "65535"
            );
            assert_eq!(quantity(function, 0), Err(invalid("quantity", 0)), "0");
        }
        for function in [0x03, 0x04] {
            assert!(quantity(function, 1).is_ok(), "{function:#04x} quantity 1");
            assert!(
                quantity(function, 125).is_ok(),
                "{function:#04x} quantity 125"
            );
            assert_eq!(quantity(function, 126), Err(oversized(126, 125)), "126");
            assert_eq!(quantity(function, 2000), Err(oversized(2000, 125)), "2000");
            assert_eq!(quantity(function, 0), Err(invalid("quantity", 0)), "0");
        }
        assert_eq!(MAX_READ_BITS, 2000, "bit limit constant");
        assert_eq!(MAX_READ_REGISTERS, 125, "register limit constant");
    }

    // covers: MB-LIMIT-SPAN
    #[test]
    fn request_span_may_end_at_the_end_of_the_address_space() {
        assert!(
            request(&[0x03, 0xFF, 0xFF, 0x00, 0x01]).is_ok(),
            "65535 + 1"
        );
        assert!(
            request(&[0x03, 0xFF, 0x83, 0x00, 0x7D]).is_ok(),
            "65411 + 125"
        );
        assert_eq!(
            request(&[0x03, 0xFF, 0xFF, 0x00, 0x02]),
            Err(oversized(65537, 65536)),
            "65535 + 2"
        );
        assert_eq!(
            request(&[0x01, 0xFF, 0xFF, 0x07, 0xD0]),
            Err(oversized(67535, 65536)),
            "65535 + 2000"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    #[test]
    fn request_with_any_other_function_code_is_refused() {
        for value in (0..=u8::MAX).filter(|value| !matches!(value, 1..=4 | 6 | 15 | 16 | 20 | 21)) {
            assert_eq!(
                request(&[value, 0x00, 0x01, 0x00, 0x01]),
                Err(invalid("function code", u64::from(value))),
                "function {value:#04x}"
            );
        }
    }

    // covers: MB-LIMIT-PDU
    #[test]
    fn pdu_above_253_bytes_is_rejected() {
        let mut bytes = vec![0x03, 0x00, 0x00, 0x00, 0x01];
        bytes.resize(MAX_PDU_BYTES, 0);
        assert_eq!(
            request(&bytes),
            Err(ParseError::TrailingBytes { remaining: 248 }),
            "253 bytes are parsed and fail on the trailing bytes"
        );
        bytes.push(0);
        assert_eq!(request(&bytes), Err(oversized(254, 253)), "254 bytes");
    }

    // ----------------------------------------------------------------- decode_response

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn response_of_zero_length_is_truncated() {
        for function in READS {
            assert_eq!(
                response(&[], function),
                Err(ParseError::Truncated {
                    needed: 1,
                    available: 0
                }),
                "empty response to {function:?}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn response_one_byte_short_is_truncated_at_every_length() {
        let mut cases = valid_responses();
        cases.push((HOLDING, vec![0x83, 0x02]));
        for (function, full) in cases {
            for len in 0..full.len() {
                let short = full.get(..len).expect("prefix of a valid response");
                let error = response(short, function).expect_err("a short response must fail");
                assert!(
                    matches!(error, ParseError::Truncated { .. }),
                    "{function:?} length {len}: {error:?}"
                );
            }
        }
    }

    #[test]
    fn response_of_exact_length_decodes_for_each_function() {
        let want = [
            bits_response(COILS, &[0xCD, 0x6B, 0x05]),
            bits_response(INPUTS, &[0xFF]),
            registers_response(HOLDING, &[1, 0xFFFF]),
            registers_response(INPUT_REGISTERS, &[10]),
        ];
        for ((function, bytes), pdu) in valid_responses().into_iter().zip(want) {
            assert_eq!(response(&bytes, function), Ok(pdu), "{function:?}");
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn response_one_byte_long_has_trailing_bytes() {
        let mut cases = valid_responses();
        cases.push((HOLDING, vec![0x83, 0x02]));
        for (function, mut bytes) in cases {
            bytes.push(0x00);
            assert_eq!(
                response(&bytes, function),
                Err(ParseError::TrailingBytes { remaining: 1 }),
                "{function:?}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    // covers: MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-PDU
    #[test]
    fn response_byte_count_bounds_are_1_to_250() {
        for function in READS {
            let number = function.as_u8();
            assert_eq!(
                response(&[number, 0x00], function),
                Err(invalid("byte count", 0)),
                "{function:?} byte count 0"
            );
            for count in [251_u8, 252, 253, 254, 255] {
                assert_eq!(
                    response(&[number, count], function),
                    Err(oversized(usize::from(count), 250)),
                    "{function:?} byte count {count} is rejected before the data is read"
                );
            }
            let mut maximum = vec![number, 250];
            maximum.resize(252, 0x5A);
            assert!(
                response(&maximum, function).is_ok(),
                "{function:?} byte count 250"
            );
            maximum.push(0);
            assert_eq!(
                response(&maximum, function),
                Err(ParseError::TrailingBytes { remaining: 1 }),
                "{function:?} 253 bytes"
            );
            maximum.push(0);
            assert_eq!(
                response(&maximum, function),
                Err(oversized(254, 253)),
                "{function:?} 254 bytes"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators
    #[test]
    fn response_register_byte_count_must_be_even() {
        for function in [HOLDING, INPUT_REGISTERS] {
            for count in (1_u8..=249).step_by(2) {
                let mut bytes = vec![function.as_u8(), count];
                bytes.resize(usize::from(count).saturating_add(2), 0);
                assert_eq!(
                    response(&bytes, function),
                    Err(invalid("register byte count", u64::from(count))),
                    "{function:?} byte count {count}"
                );
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    #[test]
    fn response_with_a_function_other_than_expected_is_refused() {
        for expected in READS.into_iter().chain(WRITES) {
            for first in 0..=u8::MAX {
                let matches_expected =
                    first == expected.as_u8() || first == expected.exception_code();
                if matches_expected {
                    continue;
                }
                assert_eq!(
                    response(&[first, 0x02, 0x00, 0x0A], expected),
                    Err(invalid("function code", u64::from(first))),
                    "expected {expected:?}, first byte {first:#04x}"
                );
            }
        }
    }

    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    #[test]
    fn response_for_an_unsupported_function_is_refused_precisely() {
        for number in
            (0..=0x7F_u8).filter(|number| !matches!(number, 1..=4 | 6 | 15 | 16 | 20 | 21))
        {
            let expected = FunctionCode::from_u8(number);
            for first in [number, number | 0x80] {
                assert_eq!(
                    response(&[first, 0x02], expected),
                    Err(invalid("unsupported function code", u64::from(first))),
                    "function {number:#04x}, first byte {first:#04x}"
                );
            }
        }
        // Write Single Coil, which upstream never sends: refused even when it is expected.
        assert_eq!(
            response(&[0x05, 0x00, 0xAC, 0xFF, 0x00], FunctionCode::from_u8(0x05)),
            Err(invalid("unsupported function code", 0x05)),
            "write single coil"
        );
        assert_eq!(
            response(&[0x10, 0x00, 0x01, 0x00, 0x02], HOLDING),
            Err(invalid("function code", 0x10)),
            "write multiple registers when FC03 is expected"
        );
    }

    // covers: MB-EXC-01, MB-EXC-02, MB-EXC-03, MB-EXC-04, MB-EXC-05, MB-EXC-06, MB-EXC-07
    // covers: MB-EXC-08, MB-EXC-0A, MB-EXC-0B
    #[test]
    fn exception_response_carries_each_defined_code_for_each_function() {
        for function in READS.into_iter().chain(WRITES) {
            for code in DEFINED {
                assert_eq!(
                    response(&[function.exception_code(), code], function),
                    Ok(ModbusPdu::Exception {
                        function,
                        code: ExceptionCode::from_u8(code)
                    }),
                    "{function:?} code {code}"
                );
            }
        }
    }

    // covers: MB-EXC-UNKNOWN
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators
    #[test]
    fn exception_response_with_an_undefined_code_is_refused() {
        for code in (0..=u8::MAX).filter(|code| !DEFINED.contains(code)) {
            assert_eq!(
                response(&[0x83, code], HOLDING),
                Err(invalid("exception code", u64::from(code))),
                "code {code}"
            );
        }
    }

    #[test]
    fn bit_response_keeps_its_padding_bits() {
        let bytes = [0x01, 0x01, 0xFF];
        let decoded = response(&bytes, COILS).expect("padding bits are not validated");
        assert_eq!(
            decoded,
            bits_response(COILS, &[0xFF]),
            "bytes kept as received"
        );
        assert_eq!(
            encoded_response(&decoded),
            Ok(bytes.to_vec()),
            "bytes re-encode"
        );
    }

    #[test]
    fn parse_errors_become_plc_errors() {
        let error = response(&[], HOLDING).expect_err("empty input");
        assert_eq!(
            PlcError::from(error).kind(),
            stonecrab::ErrorKind::MalformedData,
            "truncated"
        );
        let error = response(&[0x03, 0xFC], HOLDING).expect_err("oversized count");
        assert_eq!(
            PlcError::from(error).kind(),
            stonecrab::ErrorKind::ResourceLimit,
            "oversized"
        );
    }

    // ---------------------------------------------------------------- encode_request

    #[test]
    fn encode_request_writes_function_start_and_quantity_big_endian() {
        let cases = [
            (
                ModbusPdu::ReadCoils {
                    start: 19,
                    quantity: 19,
                },
                vec![0x01, 0x00, 0x13, 0x00, 0x13],
            ),
            (
                ModbusPdu::ReadDiscreteInputs {
                    start: 196,
                    quantity: 22,
                },
                vec![0x02, 0x00, 0xC4, 0x00, 0x16],
            ),
            (
                ModbusPdu::ReadHoldingRegisters {
                    start: 0x1234,
                    quantity: 125,
                },
                vec![0x03, 0x12, 0x34, 0x00, 0x7D],
            ),
            (
                ModbusPdu::ReadInputRegisters {
                    start: 65535,
                    quantity: 1,
                },
                vec![0x04, 0xFF, 0xFF, 0x00, 0x01],
            ),
        ];
        for (pdu, bytes) in cases {
            assert_eq!(encoded_request(&pdu), Ok(bytes), "{pdu:?}");
        }
    }

    // covers: MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-SPAN
    #[test]
    fn encode_request_refuses_what_decode_request_refuses() {
        let coils = |start, quantity| ModbusPdu::ReadCoils { start, quantity };
        let registers = |start, quantity| ModbusPdu::ReadHoldingRegisters { start, quantity };
        assert_eq!(
            encoded_request(&coils(0, 0)),
            Err(invalid("quantity", 0)),
            "0 coils"
        );
        assert_eq!(
            encoded_request(&coils(0, 2001)),
            Err(oversized(2001, 2000)),
            "2001 coils"
        );
        assert_eq!(
            encoded_request(&registers(0, 126)),
            Err(oversized(126, 125)),
            "126 registers"
        );
        assert_eq!(
            encoded_request(&registers(65535, 2)),
            Err(oversized(65537, 65536)),
            "past the end"
        );
        assert!(
            encoded_request(&registers(65535, 1)).is_ok(),
            "the last register"
        );
    }

    #[test]
    fn encode_request_refuses_responses() {
        for pdu in [
            bits_response(COILS, &[1]),
            registers_response(HOLDING, &[1]),
            ModbusPdu::Exception {
                function: HOLDING,
                code: ExceptionCode::IllegalFunction,
            },
        ] {
            assert_eq!(
                encoded_request(&pdu),
                Err(invalid(
                    "request function code",
                    u64::from(pdu.function().as_u8())
                )),
                "{pdu:?}"
            );
        }
    }

    #[test]
    fn encode_request_writes_nothing_when_the_writer_is_too_small() {
        let mut writer = BoundedWriter::new(4);
        let pdu = ModbusPdu::ReadCoils {
            start: 0,
            quantity: 1,
        };
        assert_eq!(
            encode_request(&pdu, &mut writer),
            Err(oversized(5, 4)),
            "capacity 4"
        );
        assert!(writer.as_slice().is_empty(), "nothing written");
    }

    // --------------------------------------------------------------- encode_response

    #[test]
    fn encode_response_writes_each_response_shape() {
        assert_eq!(
            encoded_response(&bits_response(COILS, &[0xCD, 0x6B, 0x05])),
            Ok(vec![0x01, 0x03, 0xCD, 0x6B, 0x05]),
            "bits"
        );
        assert_eq!(
            encoded_response(&registers_response(HOLDING, &[555, 0, 100])),
            Ok(vec![0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]),
            "registers"
        );
        let refusal = ModbusPdu::Exception {
            function: INPUT_REGISTERS,
            code: ExceptionCode::GatewayTargetDeviceFailedToRespond,
        };
        assert_eq!(
            encoded_response(&refusal),
            Ok(vec![0x84, 0x0B]),
            "exception"
        );
    }

    /// The kept raw bytes of a reply that does not answer its request are this encoding of the
    /// decoded PDU, so the codec must hand back every byte it was given.
    #[test]
    fn decoding_then_encoding_a_read_response_gives_the_received_bytes_back() {
        let mut cases = valid_responses();
        // Padding bits set in the last byte, one data byte and the 250-byte bound.
        cases.push((COILS, vec![0x01, 0x02, 0xAA, 0xBB]));
        cases.push((INPUTS, vec![0x02, 0x01, 0x80]));
        for function in [COILS, INPUTS, HOLDING, INPUT_REGISTERS] {
            let mut longest = vec![function.as_u8(), 250];
            longest.extend((0..250u8).map(|n| n.wrapping_mul(7).wrapping_add(3)));
            cases.push((function, longest));
        }
        for (function, received) in cases {
            let decoded = response(&received, function).expect("a valid response");
            assert_eq!(
                encoded_response(&decoded),
                Ok(received),
                "{function:?} re-encoded"
            );
        }
    }

    // covers: MB-LIMIT-COILS, MB-LIMIT-REGISTERS
    #[test]
    fn encode_response_enforces_the_data_bounds() {
        assert_eq!(
            encoded_response(&bits_response(COILS, &[])),
            Err(invalid("byte count", 0)),
            "no bits"
        );
        assert!(
            encoded_response(&bits_response(COILS, &[0; 250])).is_ok(),
            "250 bytes"
        );
        assert_eq!(
            encoded_response(&bits_response(COILS, &[0; 251])),
            Err(oversized(251, 250)),
            "251 bytes"
        );
        assert_eq!(
            encoded_response(&registers_response(HOLDING, &[])),
            Err(invalid("byte count", 0)),
            "no registers"
        );
        assert!(
            encoded_response(&registers_response(HOLDING, &[0; 125])).is_ok(),
            "125 registers"
        );
        assert_eq!(
            encoded_response(&registers_response(HOLDING, &[0; 126])),
            Err(oversized(252, 250)),
            "126 registers"
        );
        assert_eq!(
            encoded_response(&registers_response(HOLDING, &[0; 300])),
            Err(oversized(600, 250)),
            "300 registers"
        );
    }

    #[test]
    fn encode_response_refuses_a_function_that_does_not_fit_the_variant() {
        assert_eq!(
            encoded_response(&bits_response(HOLDING, &[1])),
            Err(invalid("response function code", 3)),
            "bits under FC03"
        );
        assert_eq!(
            encoded_response(&registers_response(COILS, &[1])),
            Err(invalid("response function code", 1)),
            "registers under FC01"
        );
        assert_eq!(
            encoded_response(&registers_response(READ_FILE, &[1])),
            Err(invalid("response function code", 0x14)),
            "registers under FC20"
        );
    }

    #[test]
    fn encode_response_refuses_unnameable_exceptions_and_requests() {
        let exception = |function, code| ModbusPdu::Exception { function, code };
        assert_eq!(
            encoded_response(&exception(HOLDING, ExceptionCode::Unknown(0))),
            Err(invalid("exception code", 0)),
            "code 0"
        );
        assert_eq!(
            encoded_response(&exception(HOLDING, ExceptionCode::Unknown(9))),
            Err(invalid("exception code", 9)),
            "code 9"
        );
        assert_eq!(
            encoded_response(&exception(HOLDING, ExceptionCode::Unknown(2))),
            Err(invalid("exception code", 2)),
            "Unknown(2) would decode as IllegalDataAddress"
        );
        for raw in [0, 5, 0x2B] {
            assert_eq!(
                encoded_response(&exception(
                    FunctionCode::Unknown(raw),
                    ExceptionCode::IllegalDataAddress
                )),
                Err(invalid("function code", u64::from(raw))),
                "exception for non-read function {raw}"
            );
        }
        assert_eq!(
            encoded_response(&exception(
                FunctionCode::Unknown(0x83),
                ExceptionCode::IllegalFunction
            )),
            Err(invalid("function code", 0x83)),
            "function with the high bit set"
        );
        assert_eq!(
            encoded_response(&ModbusPdu::ReadCoils {
                start: 0,
                quantity: 1
            }),
            Err(invalid("response function code", 1)),
            "a request"
        );
    }

    #[test]
    fn encode_response_writes_nothing_when_the_writer_is_too_small() {
        let mut writer = BoundedWriter::new(4);
        let pdu = registers_response(HOLDING, &[1, 2]);
        assert_eq!(
            encode_response(&pdu, &mut writer),
            Err(oversized(6, 4)),
            "capacity 4"
        );
        assert!(writer.as_slice().is_empty(), "nothing written");
    }

    // ----------------------------------------------------------------- check_reply_to

    // covers: MB-READ-MATCH
    #[test]
    fn check_reply_to_proves_the_data_length_against_the_quantity() {
        let coils = ModbusPdu::ReadCoils {
            start: 0,
            quantity: 19,
        };
        let inputs = ModbusPdu::ReadDiscreteInputs {
            start: 0,
            quantity: 8,
        };
        let holding = ModbusPdu::ReadHoldingRegisters {
            start: 0,
            quantity: 3,
        };
        assert_eq!(
            bits_response(COILS, &[1, 2, 3]).check_reply_to(&coils),
            Ok(()),
            "19 coils"
        );
        assert_eq!(
            bits_response(COILS, &[1, 2]).check_reply_to(&coils),
            Err(ParseError::Truncated {
                needed: 3,
                available: 2
            }),
            "one byte short"
        );
        assert_eq!(
            bits_response(COILS, &[1, 2, 3, 4]).check_reply_to(&coils),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "one byte long"
        );
        assert_eq!(
            bits_response(INPUTS, &[1]).check_reply_to(&inputs),
            Ok(()),
            "8 inputs"
        );
        assert_eq!(
            registers_response(HOLDING, &[1, 2, 3]).check_reply_to(&holding),
            Ok(()),
            "3 registers"
        );
        assert_eq!(
            registers_response(HOLDING, &[1, 2]).check_reply_to(&holding),
            Err(ParseError::Truncated {
                needed: 6,
                available: 4
            }),
            "one register short"
        );
        assert_eq!(
            registers_response(HOLDING, &[1, 2, 3, 4]).check_reply_to(&holding),
            Err(ParseError::TrailingBytes { remaining: 2 }),
            "one register long"
        );
    }

    // covers: MB-READ-MATCH
    #[test]
    fn check_reply_to_refuses_the_wrong_function_or_kind() {
        let holding = ModbusPdu::ReadHoldingRegisters {
            start: 0,
            quantity: 1,
        };
        let coils = ModbusPdu::ReadCoils {
            start: 0,
            quantity: 1,
        };
        assert_eq!(
            registers_response(INPUT_REGISTERS, &[1]).check_reply_to(&holding),
            Err(invalid("function code", 4)),
            "FC04 reply to FC03"
        );
        assert_eq!(
            bits_response(HOLDING, &[1]).check_reply_to(&holding),
            Err(invalid("response function code", 3)),
            "bits under a register request"
        );
        assert_eq!(
            registers_response(COILS, &[1]).check_reply_to(&coils),
            Err(invalid("response function code", 1)),
            "registers under a bit request"
        );
        assert_eq!(
            holding.check_reply_to(&holding),
            Err(invalid("response function code", 3)),
            "a request is not a reply"
        );
        let refusal = ModbusPdu::Exception {
            function: HOLDING,
            code: ExceptionCode::ServerDeviceBusy,
        };
        assert_eq!(
            refusal.check_reply_to(&holding),
            Ok(()),
            "exception for the request"
        );
        assert_eq!(
            refusal.check_reply_to(&coils),
            Err(invalid("function code", 3)),
            "exception for another function"
        );
        assert_eq!(
            refusal.check_reply_to(&bits_response(COILS, &[1])),
            Err(invalid("request function code", 1)),
            "a response is not a request"
        );
    }

    // ------------------------------------------------------------------- write PDUs

    /// The V1.1b section 6.6, 6.11 and 6.12 example requests and their echoes.
    fn write_examples() -> Vec<(ModbusPdu, Vec<u8>, ModbusPdu, Vec<u8>)> {
        vec![
            (
                ModbusPdu::WriteSingleRegister {
                    address: 1,
                    value: 3,
                },
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
                ModbusPdu::WriteSingleRegisterResponse {
                    address: 1,
                    value: 3,
                },
                vec![0x06, 0x00, 0x01, 0x00, 0x03],
            ),
            (
                ModbusPdu::WriteMultipleCoils {
                    start: 19,
                    quantity: 10,
                    bytes: vec![0xCD, 0x01],
                },
                vec![0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01],
                ModbusPdu::WriteMultipleResponse {
                    function: WRITE_COILS,
                    start: 19,
                    quantity: 10,
                },
                vec![0x0F, 0x00, 0x13, 0x00, 0x0A],
            ),
            (
                ModbusPdu::WriteMultipleRegisters {
                    start: 1,
                    registers: vec![0x000A, 0x0102],
                },
                vec![0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02],
                ModbusPdu::WriteMultipleResponse {
                    function: WRITE_REGISTERS,
                    start: 1,
                    quantity: 2,
                },
                vec![0x10, 0x00, 0x01, 0x00, 0x02],
            ),
        ]
    }

    /// A Write Multiple Coils request with `count` declared and `data` bytes present.
    fn coil_write_bytes(quantity: u16, count: u8, data: usize) -> Vec<u8> {
        let [hi, lo] = quantity.to_be_bytes();
        let mut bytes = vec![0x0F, 0x00, 0x00, hi, lo, count];
        bytes.resize(bytes.len().saturating_add(data), 0xA5);
        bytes
    }

    /// A Write Multiple Registers request with `count` declared and `data` bytes present.
    fn register_write_bytes(quantity: u16, count: u8, data: usize) -> Vec<u8> {
        let [hi, lo] = quantity.to_be_bytes();
        let mut bytes = vec![0x10, 0x00, 0x00, hi, lo, count];
        bytes.resize(bytes.len().saturating_add(data), 0x5A);
        bytes
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn write_examples_of_the_specification_decode_and_encode() {
        for (sent, request_bytes, echo, response_bytes) in write_examples() {
            assert_eq!(request(&request_bytes), Ok(sent.clone()), "{sent:?}");
            assert_eq!(encoded_request(&sent), Ok(request_bytes), "{sent:?}");
            let function = sent.function();
            assert_eq!(
                response(&response_bytes, function),
                Ok(echo.clone()),
                "{echo:?}"
            );
            assert_eq!(encoded_response(&echo), Ok(response_bytes), "{echo:?}");
            assert_eq!(
                echo.check_reply_to(&sent),
                Ok(()),
                "{echo:?} answers {sent:?}"
            );
        }
    }

    // covers: MB-TCP-WRITE
    // covers: SPEC mandatory-failure-cases / F1 truncated payloads
    #[test]
    fn write_requests_and_responses_cut_short_are_truncated_and_too_long_have_trailing_bytes() {
        for (_, request_bytes, _, response_bytes) in write_examples() {
            let function = FunctionCode::from_u8(*request_bytes.first().expect("a function"));
            for bytes in [&request_bytes, &response_bytes] {
                for len in 1..bytes.len() {
                    let short = bytes.get(..len).expect("a prefix");
                    let error = if bytes == &request_bytes {
                        request(short)
                    } else {
                        response(short, function)
                    }
                    .expect_err("a short write PDU must fail");
                    assert!(
                        matches!(error, ParseError::Truncated { .. }),
                        "{function:?} length {len}: {error:?}"
                    );
                }
                let mut long = bytes.clone();
                long.push(0);
                let error = if bytes == &request_bytes {
                    request(&long)
                } else {
                    response(&long, function)
                };
                assert_eq!(
                    error,
                    Err(ParseError::TrailingBytes { remaining: 1 }),
                    "{function:?} with a trailing byte"
                );
            }
        }
    }

    // covers: MB-TCP-WRITE
    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn write_multiple_coils_quantity_bounds_are_1_to_1968() {
        assert_eq!(
            request(&coil_write_bytes(0, 0, 0)),
            Err(invalid("quantity", 0)),
            "0 coils"
        );
        assert_eq!(
            request(&coil_write_bytes(1, 1, 1)),
            Ok(ModbusPdu::WriteMultipleCoils {
                start: 0,
                quantity: 1,
                bytes: vec![0xA5]
            }),
            "1 coil"
        );
        let maximum = request(&coil_write_bytes(1968, 246, 246)).expect("1968 coils");
        assert!(
            matches!(
                &maximum,
                ModbusPdu::WriteMultipleCoils { quantity: 1968, bytes, .. } if bytes.len() == 246
            ),
            "{maximum:?}"
        );
        assert_eq!(
            encoded_request(&maximum).map(|bytes| bytes.len()),
            Ok(252),
            "the largest request is 252 bytes"
        );
        assert_eq!(
            request(&coil_write_bytes(1969, 247, 247)),
            Err(oversized(1969, 1968)),
            "1969 coils, in a PDU of exactly 253 bytes"
        );
        assert_eq!(
            request(&coil_write_bytes(u16::MAX, 255, 0)),
            Err(oversized(65535, 1968)),
            "the quantity is judged before the data is read"
        );
    }

    // covers: MB-TCP-WRITE
    // covers: SPEC mandatory-failure-cases / F1 oversized lengths
    #[test]
    fn write_multiple_registers_quantity_bounds_are_1_to_123() {
        assert_eq!(
            request(&register_write_bytes(0, 0, 0)),
            Err(invalid("quantity", 0)),
            "0 registers"
        );
        assert_eq!(
            request(&register_write_bytes(1, 2, 2)),
            Ok(ModbusPdu::WriteMultipleRegisters {
                start: 0,
                registers: vec![0x5A5A]
            }),
            "1 register"
        );
        let maximum = request(&register_write_bytes(123, 246, 246)).expect("123 registers");
        assert!(
            matches!(
                &maximum,
                ModbusPdu::WriteMultipleRegisters { registers, .. } if registers.len() == 123
            ),
            "{maximum:?}"
        );
        assert_eq!(
            encoded_request(&maximum).map(|bytes| bytes.len()),
            Ok(252),
            "the largest request is 252 bytes"
        );
        assert_eq!(
            request(&register_write_bytes(124, 248, 0)),
            Err(oversized(124, 123)),
            "124 registers, judged before the data is read"
        );
        assert_eq!(
            request(&register_write_bytes(125, 250, 247)),
            Err(oversized(125, 123)),
            "125 registers in a PDU of 253 bytes"
        );
    }

    // covers: MB-TCP-WRITE
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, truncated payloads
    #[test]
    fn write_byte_count_must_fit_the_quantity_before_any_data_is_read() {
        // Coils: ten of them need 2 bytes.
        for count in (0_u8..=255).filter(|count| *count != 2) {
            assert_eq!(
                request(&coil_write_bytes(10, count, 2)),
                Err(invalid("byte count", u64::from(count))),
                "ten coils with byte count {count}"
            );
        }
        assert_eq!(
            request(&coil_write_bytes(10, 2, 1)),
            Err(ParseError::Truncated {
                needed: 2,
                available: 1
            }),
            "the data is one byte short"
        );
        assert_eq!(
            request(&coil_write_bytes(10, 2, 3)),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "the data is one byte long"
        );
        // Registers: two of them need 4 bytes, and an odd count is its own error.
        for count in (0_u8..=255).filter(|count| *count != 4) {
            let field = if count.is_multiple_of(2) {
                "byte count"
            } else {
                "register byte count"
            };
            assert_eq!(
                request(&register_write_bytes(2, count, 4)),
                Err(invalid(field, u64::from(count))),
                "two registers with byte count {count}"
            );
        }
        assert_eq!(
            request(&register_write_bytes(2, 4, 3)),
            Err(ParseError::Truncated {
                needed: 4,
                available: 3
            }),
            "the data is one byte short"
        );
        assert_eq!(
            request(&register_write_bytes(2, 4, 5)),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "the data is one byte long"
        );
    }

    // covers: MB-TCP-WRITE, MB-LIMIT-SPAN
    #[test]
    fn write_requests_may_end_at_the_end_of_the_address_space_and_not_pass_it() {
        assert_eq!(
            request(&[0x06, 0xFF, 0xFF, 0xFF, 0xFF]),
            Ok(ModbusPdu::WriteSingleRegister {
                address: 65535,
                value: 65535
            }),
            "function 06 at the last address"
        );
        assert!(
            request(&[0x0F, 0xFF, 0xFF, 0x00, 0x01, 0x01, 0x01]).is_ok(),
            "the last coil"
        );
        assert_eq!(
            request(&[0x0F, 0xFF, 0xFF, 0x00, 0x02, 0x01, 0x03]),
            Err(oversized(65537, 65536)),
            "two coils from 65535"
        );
        assert!(
            request(&[0x10, 0xFF, 0xFE, 0x00, 0x02, 0x04, 0, 1, 0, 2]).is_ok(),
            "the last two registers"
        );
        assert_eq!(
            request(&[0x10, 0xFF, 0xFF, 0x00, 0x02, 0x04, 0, 1, 0, 2]),
            Err(oversized(65537, 65536)),
            "two registers from 65535"
        );
    }

    // covers: MB-LIMIT-PDU
    #[test]
    fn write_pdus_above_253_bytes_are_rejected_before_they_are_parsed() {
        for first in [0x06_u8, 0x0F, 0x10] {
            let mut bytes = vec![first];
            bytes.resize(254, 0);
            assert_eq!(
                request(&bytes),
                Err(oversized(254, 253)),
                "request {first:#04x}"
            );
            assert_eq!(
                response(&bytes, FunctionCode::from_u8(first)),
                Err(oversized(254, 253)),
                "response {first:#04x}"
            );
        }
    }

    // covers: MB-TCP-WRITE
    // covers: SPEC mandatory-failure-cases / F1 oversized lengths, malformed discriminators
    #[test]
    fn write_multiple_responses_echo_a_quantity_inside_the_bounds() {
        let echo = |function, start, quantity| ModbusPdu::WriteMultipleResponse {
            function,
            start,
            quantity,
        };
        assert_eq!(
            response(&[0x0F, 0x00, 0x00, 0x07, 0xB0], WRITE_COILS),
            Ok(echo(WRITE_COILS, 0, 1968)),
            "1968 coils"
        );
        assert_eq!(
            response(&[0x0F, 0x00, 0x00, 0x07, 0xB1], WRITE_COILS),
            Err(oversized(1969, 1968)),
            "1969 coils"
        );
        assert_eq!(
            response(&[0x10, 0x00, 0x00, 0x00, 0x7B], WRITE_REGISTERS),
            Ok(echo(WRITE_REGISTERS, 0, 123)),
            "123 registers"
        );
        assert_eq!(
            response(&[0x10, 0x00, 0x00, 0x00, 0x7C], WRITE_REGISTERS),
            Err(oversized(124, 123)),
            "124 registers"
        );
        for (function, bytes) in [
            (WRITE_COILS, [0x0F, 0x00, 0x13, 0x00, 0x00]),
            (WRITE_REGISTERS, [0x10, 0x00, 0x01, 0x00, 0x00]),
        ] {
            assert_eq!(
                response(&bytes, function),
                Err(invalid("quantity", 0)),
                "{function:?} echoing 0"
            );
        }
        assert_eq!(
            response(&[0x10, 0xFF, 0xFF, 0x00, 0x02], WRITE_REGISTERS),
            Err(oversized(65537, 65536)),
            "two registers from 65535"
        );
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn encode_request_refuses_what_decode_request_refuses_for_writes() {
        let coils = |start, quantity, bytes: &[u8]| ModbusPdu::WriteMultipleCoils {
            start,
            quantity,
            bytes: bytes.to_vec(),
        };
        let registers = |start, count: usize| ModbusPdu::WriteMultipleRegisters {
            start,
            registers: vec![7; count],
        };
        assert_eq!(
            encoded_request(&coils(0, 0, &[])),
            Err(invalid("quantity", 0)),
            "0 coils"
        );
        assert_eq!(
            encoded_request(&coils(0, 1969, &[0; 247])),
            Err(oversized(1969, 1968)),
            "1969 coils"
        );
        assert_eq!(
            encoded_request(&coils(0, 10, &[1])),
            Err(ParseError::Truncated {
                needed: 2,
                available: 1
            }),
            "ten coils in one byte"
        );
        assert_eq!(
            encoded_request(&coils(0, 10, &[1, 2, 3])),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "ten coils in three bytes"
        );
        assert_eq!(
            encoded_request(&coils(65535, 2, &[3])),
            Err(oversized(65537, 65536)),
            "past the end"
        );
        assert!(
            encoded_request(&coils(0, 1968, &[0; 246])).is_ok(),
            "1968 coils"
        );
        assert_eq!(
            encoded_request(&registers(0, 0)),
            Err(invalid("quantity", 0)),
            "0 registers"
        );
        assert_eq!(
            encoded_request(&registers(0, 124)),
            Err(oversized(124, 123)),
            "124 registers"
        );
        assert_eq!(
            encoded_request(&registers(0, 70_000)),
            Err(oversized(70_000, 123)),
            "more registers than a quantity can name"
        );
        assert_eq!(
            encoded_request(&registers(65535, 2)),
            Err(oversized(65537, 65536)),
            "past the end"
        );
        assert!(encoded_request(&registers(0, 123)).is_ok(), "123 registers");
        let mut writer = BoundedWriter::new(9);
        let pdu = registers(1, 2);
        assert_eq!(
            encode_request(&pdu, &mut writer),
            Err(oversized(10, 9)),
            "capacity 9"
        );
        assert!(writer.as_slice().is_empty(), "nothing written");
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn encode_response_enforces_the_echo_bounds_and_the_direction_of_write_pdus() {
        let echo = |function, start, quantity| ModbusPdu::WriteMultipleResponse {
            function,
            start,
            quantity,
        };
        assert_eq!(
            encoded_response(&echo(WRITE_COILS, 0, 0)),
            Err(invalid("quantity", 0)),
            "0 coils"
        );
        assert_eq!(
            encoded_response(&echo(WRITE_COILS, 0, 1969)),
            Err(oversized(1969, 1968)),
            "1969 coils"
        );
        assert_eq!(
            encoded_response(&echo(WRITE_REGISTERS, 0, 124)),
            Err(oversized(124, 123)),
            "124 registers"
        );
        assert_eq!(
            encoded_response(&echo(WRITE_REGISTERS, 65535, 2)),
            Err(oversized(65537, 65536)),
            "past the end"
        );
        assert_eq!(
            encoded_response(&echo(HOLDING, 0, 1)),
            Err(invalid("response function code", 3)),
            "a read function in a write echo"
        );
        assert_eq!(
            encoded_response(&echo(WRITE_ONE, 0, 1)),
            Err(invalid("response function code", 6)),
            "function 06 has its own response"
        );
        for (sent, _, _, _) in write_examples() {
            let function = u64::from(sent.function().as_u8());
            assert_eq!(
                encoded_response(&sent),
                Err(invalid("response function code", function)),
                "{sent:?} is not a response"
            );
        }
        for reply in [
            ModbusPdu::WriteSingleRegisterResponse {
                address: 0,
                value: 0,
            },
            echo(WRITE_COILS, 0, 1),
        ] {
            assert_eq!(
                encoded_request(&reply),
                Err(invalid(
                    "request function code",
                    u64::from(reply.function().as_u8())
                )),
                "{reply:?} is not a request"
            );
        }
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn write_functions_that_upstream_does_not_send_are_not_decoded() {
        assert_eq!(
            request(&[0x05, 0x00, 0xAC, 0xFF, 0x00]),
            Err(invalid("function code", 5)),
            "write single coil request"
        );
        assert_eq!(
            request(&[0x0F, 0x00, 0xAC, 0x00, 0x01, 0x01, 0x01]).map(|pdu| pdu.function()),
            Ok(WRITE_COILS),
            "a single coil is function 15"
        );
    }

    // covers: MB-TCP-WRITE, MB-QUIRK-C
    #[test]
    fn check_reply_to_proves_the_echo_of_a_write() {
        let single = ModbusPdu::WriteSingleRegister {
            address: 1,
            value: 3,
        };
        let coils = ModbusPdu::WriteMultipleCoils {
            start: 19,
            quantity: 10,
            bytes: vec![0xCD, 0x01],
        };
        let registers = ModbusPdu::WriteMultipleRegisters {
            start: 1,
            registers: vec![10, 258],
        };
        let single_echo =
            |address, value| ModbusPdu::WriteSingleRegisterResponse { address, value };
        let multiple_echo = |function, start, quantity| ModbusPdu::WriteMultipleResponse {
            function,
            start,
            quantity,
        };
        assert_eq!(
            single_echo(1, 3).check_reply_to(&single),
            Ok(()),
            "function 06"
        );
        assert_eq!(
            single_echo(2, 3).check_reply_to(&single),
            Err(invalid("echoed start address", 2)),
            "function 06 address"
        );
        assert_eq!(
            single_echo(1, 4).check_reply_to(&single),
            Err(invalid("echoed value", 4)),
            "function 06 value"
        );
        assert_eq!(
            single_echo(2, 4).check_reply_to(&single),
            Err(invalid("echoed start address", 2)),
            "the address is judged first"
        );
        assert_eq!(
            multiple_echo(WRITE_COILS, 19, 10).check_reply_to(&coils),
            Ok(()),
            "function 15"
        );
        assert_eq!(
            multiple_echo(WRITE_COILS, 20, 10).check_reply_to(&coils),
            Err(invalid("echoed start address", 20)),
            "function 15 start"
        );
        assert_eq!(
            multiple_echo(WRITE_COILS, 19, 9).check_reply_to(&coils),
            Err(invalid("echoed quantity", 9)),
            "function 15 quantity"
        );
        assert_eq!(
            multiple_echo(WRITE_REGISTERS, 1, 2).check_reply_to(&registers),
            Ok(()),
            "function 16"
        );
        assert_eq!(
            multiple_echo(WRITE_REGISTERS, 0, 2).check_reply_to(&registers),
            Err(invalid("echoed start address", 0)),
            "function 16 start"
        );
        assert_eq!(
            multiple_echo(WRITE_REGISTERS, 1, 3).check_reply_to(&registers),
            Err(invalid("echoed quantity", 3)),
            "function 16 quantity"
        );
        assert_eq!(
            multiple_echo(WRITE_REGISTERS, 1, 2).check_reply_to(&coils),
            Err(invalid("function code", 16)),
            "function 16 echo under a function 15 request"
        );
        assert_eq!(
            single_echo(1, 3).check_reply_to(&registers),
            Err(invalid("function code", 6)),
            "function 06 echo under a function 16 request"
        );
        assert_eq!(
            bits_response(WRITE_COILS, &[1]).check_reply_to(&coils),
            Err(invalid("response function code", 15)),
            "bits under a write request"
        );
        assert_eq!(
            single.check_reply_to(&single),
            Err(invalid("response function code", 6)),
            "a request is not a reply"
        );
        assert_eq!(
            ModbusPdu::Exception {
                function: WRITE_REGISTERS,
                code: ExceptionCode::IllegalDataAddress
            }
            .check_reply_to(&registers),
            Ok(()),
            "an exception for the request"
        );
        assert_eq!(
            multiple_echo(WRITE_REGISTERS, 1, 2).check_reply_to(&multiple_echo(
                WRITE_REGISTERS,
                1,
                2
            )),
            Err(invalid("request function code", 16)),
            "an echo is not a request"
        );
    }

    // covers: MB-TCP-WRITE, MB-QUIRK-C, MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-SPAN
    #[test]
    fn write_request_follows_the_upstream_rule_for_the_area_and_the_payload() {
        let pick = |area, start, quantity, payload: &[u8]| {
            ModbusPdu::write_request(area, start, quantity, payload)
        };
        // Coils: always function 15, even for one coil.
        assert_eq!(
            pick(ModbusArea::Coil, 172, 1, &[0x01]),
            Ok(ModbusPdu::WriteMultipleCoils {
                start: 172,
                quantity: 1,
                bytes: vec![0x01]
            }),
            "one coil"
        );
        assert_eq!(
            pick(ModbusArea::Coil, 19, 10, &[0xCD, 0x01]).map(|pdu| pdu.function()),
            Ok(WRITE_COILS),
            "ten coils"
        );
        // Holding registers: a payload of exactly 2 bytes is function 06, else 16.
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 1, 1, &[0x00, 0x03]),
            Ok(ModbusPdu::WriteSingleRegister {
                address: 1,
                value: 3
            }),
            "two bytes"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 1, 2, &[0x00, 0x0A, 0x01, 0x02]),
            Ok(ModbusPdu::WriteMultipleRegisters {
                start: 1,
                registers: vec![10, 258]
            }),
            "four bytes"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 0, 4, &[0; 8]).map(|pdu| pdu.function()),
            Ok(WRITE_REGISTERS),
            "four registers"
        );
        // The payload must fit the quantity.
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 0, 2, &[0, 1]),
            Err(ParseError::Truncated {
                needed: 4,
                available: 2
            }),
            "two bytes for two registers are not a single write"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 0, 1, &[0, 1, 2]),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "three bytes for one register"
        );
        assert_eq!(
            pick(ModbusArea::Coil, 0, 9, &[1]),
            Err(ParseError::Truncated {
                needed: 2,
                available: 1
            }),
            "nine coils in one byte"
        );
        assert_eq!(
            pick(ModbusArea::Coil, 0, 8, &[1, 2]),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "eight coils in two bytes"
        );
        // Bounds.
        assert_eq!(
            pick(ModbusArea::Coil, 0, 0, &[]),
            Err(invalid("quantity", 0)),
            "0 coils"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 0, 0, &[]),
            Err(invalid("quantity", 0)),
            "0 registers"
        );
        assert!(
            pick(ModbusArea::Coil, 0, 1968, &[0; 246]).is_ok(),
            "1968 coils"
        );
        assert_eq!(
            pick(ModbusArea::Coil, 0, 1969, &[0; 247]),
            Err(oversized(1969, 1968)),
            "1969 coils"
        );
        assert!(
            pick(ModbusArea::HoldingRegister, 0, 123, &[0; 246]).is_ok(),
            "123 registers"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 0, 124, &[0; 248]),
            Err(oversized(124, 123)),
            "124 registers"
        );
        assert_eq!(
            pick(ModbusArea::HoldingRegister, 65535, 2, &[0; 4]),
            Err(oversized(65537, 65536)),
            "past the end"
        );
        assert!(
            pick(ModbusArea::HoldingRegister, 65535, 1, &[0; 2]).is_ok(),
            "the last register"
        );
        // Read-only areas are not writable, by their prefix digit.
        assert_eq!(
            pick(ModbusArea::DiscreteInput, 0, 1, &[1]),
            Err(invalid("write area", 1)),
            "discrete input"
        );
        assert_eq!(
            pick(ModbusArea::InputRegister, 0, 1, &[0, 1]),
            Err(invalid("write area", 3)),
            "input register"
        );
    }

    // covers: MB-TCP-WRITE
    #[test]
    fn every_selected_write_request_encodes_and_decodes_back() {
        let cases = [
            (ModbusArea::Coil, 3_u16, 1_u16, vec![1_u8]),
            (ModbusArea::Coil, 0, 17, vec![0xFF, 0x00, 0x01]),
            (ModbusArea::HoldingRegister, 9, 1, vec![0xFF, 0xFE]),
            (ModbusArea::HoldingRegister, 9, 3, vec![0, 1, 0, 2, 0, 3]),
        ];
        for (area, start, quantity, payload) in cases {
            let picked = ModbusPdu::write_request(area, start, quantity, &payload).expect("legal");
            let wire = encoded_request(&picked).expect("a selected request encodes");
            assert_eq!(request(&wire), Ok(picked), "{area:?} {start} {quantity}");
        }
    }

    // -------------------------------------------------------------- file records (FC20, FC21)

    fn read_item(file: u16, record: u16, length: u16) -> FileRecordRead {
        FileRecordRead {
            file,
            record,
            length,
        }
    }

    fn write_item(file: u16, record: u16, registers: &[u16]) -> FileRecord {
        FileRecord {
            file,
            record,
            registers: registers.to_vec(),
        }
    }

    fn read_file(items: &[FileRecordRead]) -> ModbusPdu {
        ModbusPdu::ReadFileRecord {
            records: items.to_vec(),
        }
    }

    fn write_file(items: &[FileRecord]) -> ModbusPdu {
        ModbusPdu::WriteFileRecord {
            records: items.to_vec(),
        }
    }

    fn file_response(records: &[&[u16]]) -> ModbusPdu {
        ModbusPdu::ReadFileRecordResponse {
            records: records.iter().map(|run| run.to_vec()).collect(),
        }
    }

    /// V1.1b section 6.14, the request example: file 4 records 1 and 2, then file 3
    /// records 9 and 10.
    const SPEC_READ_REQUEST: [u8; 16] = [
        0x14, 0x0E, 0x06, 0x00, 0x04, 0x00, 0x01, 0x00, 0x02, 0x06, 0x00, 0x03, 0x00, 0x09, 0x00,
        0x02,
    ];

    /// V1.1b section 6.14, the response example.
    const SPEC_READ_RESPONSE: [u8; 14] = [
        0x14, 0x0C, 0x05, 0x06, 0x0D, 0xFE, 0x00, 0x20, 0x05, 0x06, 0x33, 0xCD, 0x00, 0x40,
    ];

    /// V1.1b section 6.15, the request example (and its response, which repeats it): three
    /// registers into file 4 from record 7.
    const SPEC_WRITE_REQUEST: [u8; 15] = [
        0x15, 0x0D, 0x06, 0x00, 0x04, 0x00, 0x07, 0x00, 0x03, 0x06, 0xAF, 0x04, 0xBE, 0x10, 0x0D,
    ];

    /// A Write File Record PDU of one sub-request of `registers` registers in file 1.
    fn one_write_item(count: u8, registers: u16) -> Vec<u8> {
        let mut bytes = vec![0x15, count, 0x06, 0x00, 0x01, 0x00, 0x00];
        bytes.extend_from_slice(&registers.to_be_bytes());
        let end = bytes
            .len()
            .saturating_add(usize::from(registers).saturating_mul(2));
        bytes.resize(end, 0xAB);
        bytes
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn file_record_examples_of_the_specification_decode_and_encode() {
        let read = read_file(&[read_item(4, 1, 2), read_item(3, 9, 2)]);
        assert_eq!(
            request(&SPEC_READ_REQUEST),
            Ok(read.clone()),
            "6.14 request"
        );
        assert_eq!(
            encoded_request(&read),
            Ok(SPEC_READ_REQUEST.to_vec()),
            "6.14 request encodes"
        );
        let answer = file_response(&[&[0x0DFE, 0x0020], &[0x33CD, 0x0040]]);
        assert_eq!(
            response(&SPEC_READ_RESPONSE, READ_FILE),
            Ok(answer.clone()),
            "6.14 response"
        );
        assert_eq!(
            encoded_response(&answer),
            Ok(SPEC_READ_RESPONSE.to_vec()),
            "6.14 response encodes"
        );
        assert_eq!(answer.check_reply_to(&read), Ok(()), "6.14 answers 6.14");

        let write = write_file(&[write_item(4, 7, &[0x06AF, 0x04BE, 0x100D])]);
        assert_eq!(
            request(&SPEC_WRITE_REQUEST),
            Ok(write.clone()),
            "6.15 request"
        );
        assert_eq!(
            encoded_request(&write),
            Ok(SPEC_WRITE_REQUEST.to_vec()),
            "6.15 request encodes"
        );
        let echo = ModbusPdu::WriteFileRecordResponse {
            records: vec![write_item(4, 7, &[0x06AF, 0x04BE, 0x100D])],
        };
        assert_eq!(
            response(&SPEC_WRITE_REQUEST, WRITE_FILE),
            Ok(echo.clone()),
            "6.15 response is the request"
        );
        assert_eq!(
            encoded_response(&echo),
            Ok(SPEC_WRITE_REQUEST.to_vec()),
            "6.15 response encodes"
        );
        assert_eq!(echo.check_reply_to(&write), Ok(()), "6.15 echoes 6.15");
        assert_eq!(read.function(), READ_FILE, "function of a read request");
        assert_eq!(answer.function(), READ_FILE, "function of a read response");
        assert_eq!(write.function(), WRITE_FILE, "function of a write request");
        assert_eq!(echo.function(), WRITE_FILE, "function of a write response");
    }

    // covers: MB-ADDR-EXTENDED
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, truncated payloads
    #[test]
    fn read_file_record_byte_count_is_whole_sub_requests_of_7_to_245() {
        assert_eq!(
            request(&[0x14]),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            })
        );
        assert_eq!(request(&[0x14, 0x00]), Err(invalid("byte count", 0)), "0");
        for count in [1_u8, 6, 8, 13, 15, 244] {
            let mut bytes = vec![0x14, count];
            bytes.resize(2 + usize::from(count), 0);
            assert_eq!(
                request(&bytes),
                Err(invalid("byte count", u64::from(count))),
                "{count} is not a multiple of 7"
            );
        }
        for count in [246_u8, 252, 255] {
            assert_eq!(
                request(&[0x14, count]),
                Err(oversized(usize::from(count), 245)),
                "{count} is over 0xF5"
            );
        }
        // The byte count is proved before the data is read.
        assert_eq!(
            request(&[0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00]),
            Err(ParseError::Truncated {
                needed: 7,
                available: 6
            }),
            "one byte short of one sub-request"
        );
        let mut long = SPEC_READ_REQUEST.to_vec();
        long.push(0);
        assert_eq!(
            request(&long),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "a byte after the sub-requests"
        );
        // 35 sub-requests of one register each are the most a request holds.
        let mut full = vec![0x14, 0xF5];
        for _ in 0..35 {
            full.extend_from_slice(&[0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]);
        }
        assert_eq!(
            request(&full),
            Ok(read_file(&[read_item(1, 0, 1); 35])),
            "245 bytes"
        );
        assert_eq!(
            encoded_request(&read_file(&[read_item(1, 0, 1); 35])),
            Ok(full)
        );
        assert_eq!(
            encoded_request(&read_file(&[read_item(1, 0, 1); 36])),
            Err(oversized(252, 245)),
            "36 sub-requests"
        );
    }

    // covers: MB-ADDR-EXTENDED
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, oversized lengths
    #[test]
    fn read_file_record_sub_requests_are_checked_one_by_one() {
        let item = |kind: u8, file: u16, record: u16, length: u16| {
            let mut bytes = vec![0x14, 0x07, kind];
            for word in [file, record, length] {
                bytes.extend_from_slice(&word.to_be_bytes());
            }
            bytes
        };
        assert_eq!(
            request(&item(5, 1, 0, 1)),
            Err(invalid("reference type", 5)),
            "reference type 5"
        );
        assert_eq!(
            request(&item(0, 1, 0, 1)),
            Err(invalid("reference type", 0)),
            "reference type 0"
        );
        assert_eq!(
            request(&item(6, 1, 0, 0)),
            Err(invalid("record length", 0)),
            "length 0"
        );
        assert_eq!(
            request(&item(6, 1, 10_000, 1)),
            Err(oversized(10_000, 9_999)),
            "record 10000"
        );
        assert_eq!(
            request(&item(6, 1, 0x270F, 1)),
            Ok(read_file(&[read_item(1, 9_999, 1)])),
            "record 0x270F is the last"
        );
        assert_eq!(
            request(&item(6, 1, 9_999, 2)),
            Err(oversized(10_001, 10_000)),
            "a run past the last record of its file"
        );
        assert_eq!(
            request(&item(6, 1, 9_880, 120)),
            Ok(read_file(&[read_item(1, 9_880, 120)])),
            "a run to the last record fits the file"
        );
        // File numbers are 0 to 0xFFFF on the wire (V1.1b section 6.14).
        assert_eq!(
            request(&item(6, 0, 0, 1)),
            Ok(read_file(&[read_item(0, 0, 1)])),
            "file 0"
        );
        assert_eq!(
            request(&item(6, 0xFFFF, 0, 1)),
            Ok(read_file(&[read_item(0xFFFF, 0, 1)])),
            "file 65535"
        );
    }

    // covers: MB-ADDR-EXTENDED, MB-LIMIT-PDU
    #[test]
    fn read_file_record_asks_for_no_more_than_a_response_can_hold() {
        // One sub-response costs 2 bytes beside its registers, 245 bytes at most in all.
        assert_eq!(
            request(&one_read(121)),
            Ok(read_file(&[read_item(1, 0, 121)])),
            "121 registers answer in 244 bytes"
        );
        assert_eq!(
            request(&one_read(122)),
            Err(oversized(246, 245)),
            "122 registers answer in 246 bytes"
        );
        let four = |length: u16| {
            let mut bytes = vec![0x14, 28];
            for _ in 0..4 {
                bytes.extend_from_slice(&[0x06, 0x00, 0x01, 0x00, 0x00]);
                bytes.extend_from_slice(&length.to_be_bytes());
            }
            bytes
        };
        assert_eq!(request(&four(30)), Err(oversized(248, 245)), "four of 30");
        assert!(request(&four(29)).is_ok(), "four of 29 answer in 240 bytes");
        assert_eq!(
            encoded_request(&read_file(&[read_item(1, 0, 122)])),
            Err(oversized(246, 245)),
            "the encoder refuses the same"
        );
        assert_eq!(
            encoded_request(&read_file(&[])),
            Err(invalid("byte count", 0)),
            "no sub-request"
        );
        assert_eq!(
            encoded_request(&read_file(&[read_item(1, 9_999, 2)])),
            Err(oversized(10_001, 10_000)),
            "a run past its file"
        );
    }

    /// A Read File Record request for `length` registers from file 1 record 0.
    fn one_read(length: u16) -> Vec<u8> {
        let mut bytes = vec![0x14, 0x07, 0x06, 0x00, 0x01, 0x00, 0x00];
        bytes.extend_from_slice(&length.to_be_bytes());
        bytes
    }

    // covers: MB-ADDR-EXTENDED
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, truncated payloads
    #[test]
    fn read_file_record_response_lengths_are_proved_before_they_are_trusted() {
        assert_eq!(
            response(&[0x14], READ_FILE),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            })
        );
        assert_eq!(
            response(&[0x14, 0x00], READ_FILE),
            Err(invalid("byte count", 0)),
            "0"
        );
        assert_eq!(
            response(&[0x14, 0xF6], READ_FILE),
            Err(oversized(246, 245)),
            "246"
        );
        assert_eq!(
            response(&[0x14, 0x04, 0x03, 0x06, 0x00], READ_FILE),
            Err(ParseError::Truncated {
                needed: 4,
                available: 3
            }),
            "a byte count the data does not reach"
        );
        assert_eq!(
            response(&[0x14, 0x04, 0x03, 0x06, 0x00, 0x01, 0x09], READ_FILE),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "a byte after the data"
        );
        for length in [0_u8, 1, 2, 4, 6] {
            assert_eq!(
                response(&[0x14, 0x07, length, 0x06, 0, 0, 0, 0, 0], READ_FILE),
                Err(invalid("file response length", u64::from(length))),
                "sub-response length {length}"
            );
        }
        assert_eq!(
            response(&[0x14, 0x04, 0x03, 0x05, 0x00, 0x01], READ_FILE),
            Err(invalid("reference type", 5)),
            "reference type 5"
        );
        assert_eq!(
            response(&[0x14, 0x04, 0x05, 0x06, 0x00, 0x01], READ_FILE),
            Err(ParseError::Truncated {
                needed: 4,
                available: 2
            }),
            "a sub-response that runs past the byte count"
        );
        assert_eq!(
            response(&[0x14, 0x05, 0x03, 0x06, 0x00, 0x01, 0x03], READ_FILE),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            }),
            "half a sub-response after a whole one"
        );
        // 121 registers fill 244 bytes, the most one sub-response can have.
        let mut full = vec![0x14, 244, 243, 0x06];
        full.resize(full.len() + 242, 0x11);
        assert_eq!(
            response(&full, READ_FILE),
            Ok(file_response(&[&[0x1111; 121]])),
            "121 registers"
        );
        assert_eq!(
            encoded_response(&file_response(&[&[0x1111; 121]])),
            Ok(full),
            "121 registers encode"
        );
        assert_eq!(
            encoded_response(&file_response(&[&[0; 122]])),
            Err(oversized(246, 245)),
            "122 registers"
        );
        assert_eq!(
            encoded_response(&file_response(&[])),
            Err(invalid("byte count", 0)),
            "no sub-response"
        );
        assert_eq!(
            encoded_response(&file_response(&[&[1], &[]])),
            Err(invalid("record length", 0)),
            "an empty sub-response"
        );
    }

    // covers: MB-ADDR-EXTENDED
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, truncated payloads
    #[test]
    fn write_file_record_data_length_is_9_to_251_and_fills_the_sub_requests() {
        assert_eq!(
            request(&[0x15]),
            Err(ParseError::Truncated {
                needed: 1,
                available: 0
            })
        );
        for count in [0_u8, 1, 8] {
            let mut bytes = vec![0x15, count];
            bytes.resize(2 + usize::from(count), 0);
            assert_eq!(
                request(&bytes),
                Err(invalid("byte count", u64::from(count))),
                "{count} is below 9"
            );
        }
        assert_eq!(request(&[0x15, 0xFC]), Err(oversized(252, 251)), "252");
        let mut short = one_write_item(122, 0);
        short.truncate(8);
        assert_eq!(
            request(&short),
            Err(ParseError::Truncated {
                needed: 122,
                available: 6
            }),
            "data the byte count does not reach"
        );
        let largest = one_write_item(251, 122);
        assert_eq!(largest.len(), MAX_PDU_BYTES, "a full PDU");
        assert_eq!(
            request(&largest),
            Ok(write_file(&[write_item(1, 0, &[0xABAB; 122])])),
            "251 bytes of data"
        );
        assert_eq!(
            encoded_request(&write_file(&[write_item(1, 0, &[0xABAB; 122])])),
            Ok(largest),
            "251 bytes encode"
        );
        assert_eq!(
            encoded_request(&write_file(&[write_item(1, 0, &[0; 123])])),
            Err(oversized(253, 251)),
            "123 registers"
        );
        assert_eq!(
            encoded_request(&write_file(&[write_item(1, 0, &vec![0; 65_536])])),
            Err(oversized(65_536, 65_535)),
            "a record of more registers than its 16-bit length counts"
        );
        let mut trailing = SPEC_WRITE_REQUEST.to_vec();
        trailing.push(0);
        assert_eq!(
            request(&trailing),
            Err(ParseError::TrailingBytes { remaining: 1 }),
            "a byte after the data"
        );
        // The sub-requests fill the byte count exactly: a declared length that the data
        // does not hold, and a stray half sub-request, are refused.
        assert_eq!(
            request(&[
                0x15, 0x0B, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x03, 1, 2, 3, 4
            ]),
            Err(ParseError::Truncated {
                needed: 6,
                available: 4
            }),
            "three registers declared, two present"
        );
        assert_eq!(
            request(&[
                0x15, 0x0A, 0x06, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 1, 2, 0x06
            ]),
            Err(ParseError::Truncated {
                needed: 2,
                available: 0
            }),
            "a sub-request that ends after its reference type"
        );
    }

    // covers: MB-ADDR-EXTENDED
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, oversized lengths
    #[test]
    fn write_file_record_sub_requests_are_checked_one_by_one() {
        let item = |kind: u8, record: u16, length: u16, data: usize| {
            let mut bytes = vec![
                0x15,
                u8::try_from(7 + data).expect("short"),
                kind,
                0x00,
                0x01,
            ];
            for word in [record, length] {
                bytes.extend_from_slice(&word.to_be_bytes());
            }
            bytes.resize(bytes.len() + data, 0);
            bytes
        };
        assert_eq!(
            request(&item(5, 0, 1, 2)),
            Err(invalid("reference type", 5)),
            "reference type 5"
        );
        assert_eq!(
            request(&item(6, 0, 0, 2)),
            Err(invalid("record length", 0)),
            "length 0"
        );
        assert_eq!(
            request(&item(6, 10_000, 1, 2)),
            Err(oversized(10_000, 9_999)),
            "record 10000"
        );
        assert_eq!(
            request(&item(6, 9_999, 2, 4)),
            Err(oversized(10_001, 10_000)),
            "a run past its file"
        );
        assert_eq!(
            request(&item(6, 9_999, 1, 2)),
            Ok(write_file(&[write_item(1, 9_999, &[0])])),
            "the last record"
        );
        assert_eq!(
            encoded_request(&write_file(&[])),
            Err(invalid("byte count", 0)),
            "no sub-request"
        );
        assert_eq!(
            encoded_request(&write_file(&[write_item(1, 0, &[])])),
            Err(invalid("record length", 0)),
            "a sub-request of no register"
        );
        assert_eq!(
            encoded_request(&write_file(&[write_item(1, 9_999, &[1, 2])])),
            Err(oversized(10_001, 10_000)),
            "a run past its file"
        );
        let mut two = vec![0x15, 22];
        two.extend(SPEC_WRITE_REQUEST.iter().skip(2));
        two.extend_from_slice(&[0x06, 0x00, 0x05, 0x00, 0x00, 0x00, 0x01, 0x12, 0x34]);
        assert_eq!(
            request(&two),
            Ok(write_file(&[
                write_item(4, 7, &[0x06AF, 0x04BE, 0x100D]),
                write_item(5, 0, &[0x1234])
            ])),
            "two sub-requests"
        );
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn exception_responses_name_the_file_record_functions() {
        for function in [READ_FILE, WRITE_FILE] {
            for code in DEFINED {
                let bytes = [function.exception_code(), code];
                assert_eq!(
                    response(&bytes, function),
                    Ok(ModbusPdu::Exception {
                        function,
                        code: ExceptionCode::from_u8(code),
                    }),
                    "{function:?} code {code}"
                );
            }
            assert_eq!(
                response(&[function.exception_code(), 9], function),
                Err(invalid("exception code", 9)),
                "{function:?} code 9"
            );
            assert_eq!(
                encoded_response(&ModbusPdu::Exception {
                    function,
                    code: ExceptionCode::IllegalDataAddress
                }),
                Ok(vec![function.exception_code(), 2]),
                "{function:?} encodes"
            );
        }
        assert_eq!(
            response(&[0x15, 0x09], READ_FILE),
            Err(invalid("function code", 0x15)),
            "the other function"
        );
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn file_record_pdus_are_refused_in_the_wrong_direction() {
        let read = read_file(&[read_item(1, 0, 1)]);
        let answer = file_response(&[&[1]]);
        let write = write_file(&[write_item(1, 0, &[1])]);
        let echo = ModbusPdu::WriteFileRecordResponse {
            records: vec![write_item(1, 0, &[1])],
        };
        for pdu in [&answer, &echo] {
            assert_eq!(
                encoded_request(pdu),
                Err(invalid(
                    "request function code",
                    u64::from(pdu.function().as_u8())
                )),
                "a response is not a request"
            );
        }
        for pdu in [&read, &write] {
            assert_eq!(
                encoded_response(pdu),
                Err(invalid(
                    "response function code",
                    u64::from(pdu.function().as_u8())
                )),
                "a request is not a response"
            );
        }
        assert_eq!(
            encoded_response(&ModbusPdu::WriteMultipleResponse {
                function: READ_FILE,
                start: 0,
                quantity: 1
            }),
            Err(invalid("response function code", 0x14)),
            "a write echo under FC20"
        );
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn a_file_record_span_is_cut_at_the_end_of_each_file() {
        let plan =
            |file, record, quantity| ModbusPdu::read_file_record_request(file, record, quantity);
        assert_eq!(
            plan(1, 0, 1),
            Ok(read_file(&[read_item(1, 0, 1)])),
            "the first register"
        );
        assert_eq!(
            plan(1, 9_998, 2),
            Ok(read_file(&[read_item(1, 9_998, 2)])),
            "a span that ends exactly at the end of the file"
        );
        assert_eq!(
            plan(1, 9_999, 1),
            Ok(read_file(&[read_item(1, 9_999, 1)])),
            "the last register of the file"
        );
        assert_eq!(
            plan(3, 9_998, 10),
            Ok(read_file(&[read_item(3, 9_998, 2), read_item(4, 0, 8)])),
            "the split of the upstream test case (file 3 record 9998, then file 4)"
        );
        assert_eq!(
            plan(65_535, 9_999, 1),
            Ok(read_file(&[read_item(65_535, 9_999, 1)])),
            "the last register of the last file"
        );
        assert_eq!(
            plan(65_535, 9_999, 2),
            Err(oversized(65_536, 65_535)),
            "a span past the last file"
        );
        assert_eq!(plan(1, 0, 0), Err(invalid("quantity", 0)), "quantity 0");
        assert_eq!(
            plan(1, 10_000, 1),
            Err(oversized(10_000, 9_999)),
            "record 10000"
        );
        assert_eq!(
            plan(1, 0, MAX_READ_EXTENDED_REGISTERS),
            Ok(read_file(&[read_item(1, 0, 120)])),
            "120 registers"
        );
        assert_eq!(
            plan(1, 9_950, MAX_READ_EXTENDED_REGISTERS),
            Ok(read_file(&[read_item(1, 9_950, 50), read_item(2, 0, 70)])),
            "120 registers across a file end still answer in 244 bytes"
        );
        assert_eq!(
            plan(1, 0, MAX_READ_EXTENDED_REGISTERS + 1),
            Err(oversized(121, 120)),
            "121 registers"
        );
        assert_eq!(MAX_READ_EXTENDED_REGISTERS, 120, "the constant");

        let write = |file, record, registers: &[u16]| {
            ModbusPdu::write_file_record_request(file, record, registers)
        };
        assert_eq!(
            write(2, 5, &[1, 2]),
            Ok(write_file(&[write_item(2, 5, &[1, 2])])),
            "inside a file"
        );
        assert_eq!(
            write(2, 9_998, &[1, 2]),
            Ok(write_file(&[write_item(2, 9_998, &[1, 2])])),
            "to the last record"
        );
        assert_eq!(
            write(1, 9_999, &[1, 2, 3, 4]),
            Ok(write_file(&[
                write_item(1, 9_999, &[1]),
                write_item(2, 0, &[2, 3, 4])
            ])),
            "a value across the end of a file is one request of two sub-requests"
        );
        assert_eq!(
            write(1, 0, &[]),
            Err(invalid("quantity", 0)),
            "no registers"
        );
        assert_eq!(
            write(1, 10_000, &[1]),
            Err(oversized(10_000, 9_999)),
            "record 10000"
        );
        assert_eq!(
            write(65_535, 9_999, &[1, 2]),
            Err(oversized(65_536, 65_535)),
            "past the last file"
        );
        assert_eq!(
            write(1, 0, &[0; 123]),
            Err(oversized(253, 251)),
            "123 registers do not fit 251 bytes"
        );
        assert!(
            write(1, 0, &[0; 122]).is_ok(),
            "122 registers fill 251 bytes"
        );
        assert_eq!(
            write(1, 0, &vec![0; 65_536]),
            Err(oversized(131_079, 251)),
            "more registers than a u16 counts are refused before they are copied"
        );
        assert_eq!(
            write(1, 9_999, &[0; 121]),
            Err(oversized(256, 251)),
            "two sub-requests cost 7 more bytes"
        );
        assert_eq!(
            ModbusPdu::write_request(ModbusArea::ExtendedRegister, 0, 1, &[0, 1]),
            Err(invalid("write area", 6)),
            "an extended register has its own request"
        );
    }

    // covers: MB-ADDR-EXTENDED, MB-TCP-WRITE
    #[test]
    fn check_reply_to_proves_file_record_answers_and_echoes() {
        let read = read_file(&[read_item(3, 9_998, 2), read_item(4, 0, 3)]);
        assert_eq!(
            file_response(&[&[1, 2], &[3, 4, 5]]).check_reply_to(&read),
            Ok(()),
            "every sub-request answered, in order"
        );
        assert_eq!(
            file_response(&[&[1, 2]]).check_reply_to(&read),
            Err(invalid("record count", 1)),
            "a sub-response missing"
        );
        assert_eq!(
            file_response(&[&[1, 2], &[3, 4, 5], &[6]]).check_reply_to(&read),
            Err(invalid("record count", 3)),
            "a sub-response too many"
        );
        assert_eq!(
            file_response(&[&[1, 2], &[3, 4]]).check_reply_to(&read),
            Err(ParseError::Truncated {
                needed: 6,
                available: 4
            }),
            "a sub-response one register short"
        );
        assert_eq!(
            file_response(&[&[1, 2, 9], &[3, 4, 5]]).check_reply_to(&read),
            Err(ParseError::TrailingBytes { remaining: 2 }),
            "a sub-response one register long"
        );
        assert_eq!(
            ModbusPdu::Exception {
                function: READ_FILE,
                code: ExceptionCode::IllegalDataAddress
            }
            .check_reply_to(&read),
            Ok(()),
            "an exception of the function"
        );
        assert_eq!(
            bits_response(COILS, &[1]).check_reply_to(&read),
            Err(invalid("function code", 1)),
            "another function"
        );

        let write = write_file(&[write_item(1, 9_999, &[1]), write_item(2, 0, &[2, 3])]);
        let echo = |records: &[FileRecord]| ModbusPdu::WriteFileRecordResponse {
            records: records.to_vec(),
        };
        assert_eq!(
            echo(&[write_item(1, 9_999, &[1]), write_item(2, 0, &[2, 3])]).check_reply_to(&write),
            Ok(()),
            "the request, repeated"
        );
        assert_eq!(
            echo(&[write_item(1, 9_999, &[1])]).check_reply_to(&write),
            Err(invalid("echoed record count", 1)),
            "a sub-request not echoed"
        );
        assert_eq!(
            echo(&[write_item(7, 9_999, &[1]), write_item(2, 0, &[2, 3])]).check_reply_to(&write),
            Err(invalid("echoed file number", 7)),
            "another file"
        );
        assert_eq!(
            echo(&[write_item(1, 9_998, &[1]), write_item(2, 0, &[2, 3])]).check_reply_to(&write),
            Err(invalid("echoed record number", 9_998)),
            "another record"
        );
        assert_eq!(
            echo(&[write_item(1, 9_999, &[1]), write_item(2, 0, &[2])]).check_reply_to(&write),
            Err(invalid("echoed record length", 1)),
            "another length"
        );
        assert_eq!(
            echo(&[write_item(1, 9_999, &[1]), write_item(2, 0, &[2, 9])]).check_reply_to(&write),
            Err(invalid("echoed record data", 9)),
            "other data"
        );
        assert_eq!(
            echo(&[write_item(1, 9_999, &[1])]).check_reply_to(&read),
            Err(invalid("function code", 0x15)),
            "an echo is no answer to a read"
        );
        assert_eq!(
            ModbusPdu::Exception {
                function: WRITE_FILE,
                code: ExceptionCode::ServerDeviceFailure
            }
            .check_reply_to(&write),
            Ok(()),
            "an exception of the function"
        );
    }

    // covers: MB-ADDR-EXTENDED
    #[test]
    fn every_planned_file_record_request_encodes_and_decodes_back() {
        for (file, record, quantity) in [
            (1, 0, 1),
            (1, 9_999, 2),
            (4, 9_990, 120),
            (65_535, 9_990, 10),
        ] {
            let read = ModbusPdu::read_file_record_request(file, record, quantity).expect("legal");
            let wire = encoded_request(&read).expect("a planned read encodes");
            assert_eq!(request(&wire), Ok(read), "read {file} {record} {quantity}");
            let registers = vec![0x5A5A; usize::from(quantity.min(118))];
            let write =
                ModbusPdu::write_file_record_request(file, record, &registers).expect("legal");
            let wire = encoded_request(&write).expect("a planned write encodes");
            assert_eq!(
                request(&wire),
                Ok(write),
                "write {file} {record} {quantity}"
            );
        }
    }
}
