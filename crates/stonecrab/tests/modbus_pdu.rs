//! The PDU codec: the coding standards section 13.7 list for each decode function, the code tables,
//! the encoders and the reply check.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::PlcError;
    use stonecrab::codec::{BoundedReader, BoundedWriter, ParseError};
    use stonecrab::modbus::{
        ExceptionCode, FunctionCode, MAX_PDU_BYTES, MAX_READ_BITS, MAX_READ_REGISTERS, ModbusPdu,
        decode_request, decode_response, encode_request, encode_response,
    };

    const COILS: FunctionCode = FunctionCode::ReadCoils;
    const INPUTS: FunctionCode = FunctionCode::ReadDiscreteInputs;
    const HOLDING: FunctionCode = FunctionCode::ReadHoldingRegisters;
    const INPUT_REGISTERS: FunctionCode = FunctionCode::ReadInputRegisters;
    const READS: [FunctionCode; 4] = [COILS, INPUTS, HOLDING, INPUT_REGISTERS];
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
    #[test]
    fn function_codes_round_trip_and_name_only_the_four_reads() {
        for value in 0..=u8::MAX {
            let code = FunctionCode::from_u8(value);
            assert_eq!(code.as_u8(), value, "function {value:#04x} round trip");
            assert_eq!(
                code.exception_code(),
                value | 0x80,
                "exception byte of {value:#04x}"
            );
            let named = matches!(value, 1..=4);
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
        assert_eq!(COILS.exception_code(), 0x81, "exception byte of FC01");
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
        for value in (0..=u8::MAX).filter(|value| !matches!(value, 1..=4)) {
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
        for expected in READS {
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
        for number in (0..=0x7F_u8).filter(|number| !matches!(number, 1..=4)) {
            let expected = FunctionCode::from_u8(number);
            for first in [number, number | 0x80] {
                assert_eq!(
                    response(&[first, 0x02], expected),
                    Err(invalid("unsupported function code", u64::from(first))),
                    "function {number:#04x}, first byte {first:#04x}"
                );
            }
        }
        // A write response, as the codec would meet it from a device: FC16, quantity 2.
        assert_eq!(
            response(&[0x10, 0x00, 0x01, 0x00, 0x02], FunctionCode::from_u8(0x10)),
            Err(invalid("unsupported function code", 0x10)),
            "write multiple registers"
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
        for function in READS {
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
            encoded_response(&registers_response(FunctionCode::Unknown(0x10), &[1])),
            Err(invalid("response function code", 0x10)),
            "registers under FC16"
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
        for raw in [0, 1, 5, 0x10, 0x14] {
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
}
