//! Decodes arbitrary bytes as a Modbus PDU: as a request, and as the response to each of the
//! four read functions, the three write functions (06, 15, 16), the two file record
//! functions (20, 21) plus an unsupported one.
//!
//! The target must never panic. Every accepted PDU must stay inside the protocol bounds, must
//! name the function it was decoded for, and must re-encode to exactly the input bytes (so
//! padding bits, byte counts and exception codes are all preserved); an accepted read
//! response must also survive the reply check (without a panic) against a request of its
//! function, and an accepted write request (a file record write included) and the echo built
//! from its fields must pass it.
//! (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::codec::{BoundedReader, BoundedWriter};
use stonecrab::modbus::{
    ExceptionCode, FileRecordRead, FunctionCode, MAX_PDU_BYTES, ModbusPdu, decode_request,
    decode_response, encode_request, encode_response,
};

fuzz_target!(|data: &[u8]| {
    if let Ok(request) = decode_request(&mut BoundedReader::new(data)) {
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        assert_eq!(
            encode_request(&request, &mut writer),
            Ok(()),
            "an accepted request encodes"
        );
        assert_eq!(
            writer.as_slice(),
            data,
            "an accepted request re-encodes to its input"
        );
        check_write_request(&request);
    }
    let first = data.first().copied().unwrap_or(0);
    let expected_functions = [
        FunctionCode::ReadCoils,
        FunctionCode::ReadDiscreteInputs,
        FunctionCode::ReadHoldingRegisters,
        FunctionCode::ReadInputRegisters,
        FunctionCode::WriteSingleRegister,
        FunctionCode::WriteMultipleCoils,
        FunctionCode::WriteMultipleRegisters,
        FunctionCode::ReadFileRecord,
        FunctionCode::WriteFileRecord,
        FunctionCode::Unknown(0x05),
        FunctionCode::from_u8(first & 0x7F),
    ];
    for expected in expected_functions {
        let Ok(response) = decode_response(&mut BoundedReader::new(data), expected) else {
            continue;
        };
        assert_eq!(
            response.function(),
            expected,
            "the response belongs to the expected function"
        );
        match &response {
            ModbusPdu::ReadBitsResponse { bytes, .. } => {
                assert!(
                    (1..=250).contains(&bytes.len()),
                    "bit data length {}",
                    bytes.len()
                );
            }
            ModbusPdu::ReadRegistersResponse { registers, .. } => {
                assert!(
                    (1..=125).contains(&registers.len()),
                    "register count {}",
                    registers.len()
                );
            }
            ModbusPdu::WriteSingleRegisterResponse { .. } => {}
            ModbusPdu::WriteMultipleResponse {
                function, quantity, ..
            } => {
                let max = if *function == FunctionCode::WriteMultipleCoils {
                    1_968
                } else {
                    123
                };
                assert!(
                    (1..=max).contains(quantity),
                    "echoed quantity {quantity} of {function:?}"
                );
            }
            ModbusPdu::ReadFileRecordResponse { records } => {
                let data: usize = records.iter().map(|run| 2 + 2 * run.len()).sum();
                assert!(
                    !records.is_empty() && records.iter().all(|run| !run.is_empty()),
                    "a file record read answers with registers"
                );
                assert!(data <= 245, "file record response data {data}");
            }
            ModbusPdu::WriteFileRecordResponse { records } => {
                let data: usize = records.iter().map(|record| 7 + 2 * record.registers.len()).sum();
                assert!((9..=251).contains(&data), "file record echo data {data}");
                assert!(
                    records.iter().all(|record| {
                        !record.registers.is_empty()
                            && record.record <= 9_999
                            && usize::from(record.record) + record.registers.len() <= 10_000
                    }),
                    "every echoed sub-request stays inside its file"
                );
            }
            ModbusPdu::Exception { code, .. } => {
                assert!(
                    !matches!(
                        ExceptionCode::from_u8(code.as_u8()),
                        ExceptionCode::Unknown(_)
                    ),
                    "an accepted exception code is one of the ten defined ones"
                );
            }
            _ => panic!("decode_response returned a request"),
        }
        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
        assert_eq!(
            encode_response(&response, &mut writer),
            Ok(()),
            "an accepted response encodes"
        );
        assert_eq!(
            writer.as_slice(),
            data,
            "an accepted response re-encodes to its input"
        );
        if let Some(request) = request_for_echo(&response) {
            assert_eq!(
                response.check_reply_to(&request),
                Ok(()),
                "an echo answers the request built from its own fields"
            );
            continue;
        }
        if let ModbusPdu::ReadFileRecordResponse { records } = &response {
            // Sub-requests that ask for as many registers as each sub-response holds.
            let asked = records
                .iter()
                .map(|run| FileRecordRead {
                    file: 1,
                    record: 0,
                    length: u16::try_from(run.len()).unwrap_or(u16::MAX),
                })
                .collect();
            assert_eq!(
                response.check_reply_to(&ModbusPdu::ReadFileRecord { records: asked }),
                Ok(()),
                "a file record read answers the request that asked for what it holds"
            );
            continue;
        }
        let quantity = match &response {
            ModbusPdu::ReadBitsResponse { bytes, .. } => {
                u16::try_from(bytes.len().saturating_mul(8)).unwrap_or(u16::MAX)
            }
            ModbusPdu::ReadRegistersResponse { registers, .. } => {
                u16::try_from(registers.len()).unwrap_or(u16::MAX)
            }
            _ => 1,
        };
        let request = match response.function() {
            FunctionCode::ReadCoils => ModbusPdu::ReadCoils { start: 0, quantity },
            FunctionCode::ReadDiscreteInputs => {
                ModbusPdu::ReadDiscreteInputs { start: 0, quantity }
            }
            FunctionCode::ReadHoldingRegisters => {
                ModbusPdu::ReadHoldingRegisters { start: 0, quantity }
            }
            _ => ModbusPdu::ReadInputRegisters { start: 0, quantity },
        };
        // Only the absence of a panic matters here: the quantity may not match the response.
        let _ = response.check_reply_to(&request);
    }
});

/// The write request that `response`, a write echo, answers, or `None` for any other response.
fn request_for_echo(response: &ModbusPdu) -> Option<ModbusPdu> {
    match response {
        ModbusPdu::WriteSingleRegisterResponse { address, value } => {
            Some(ModbusPdu::WriteSingleRegister {
                address: *address,
                value: *value,
            })
        }
        ModbusPdu::WriteMultipleResponse {
            function: FunctionCode::WriteMultipleCoils,
            start,
            quantity,
        } => Some(ModbusPdu::WriteMultipleCoils {
            start: *start,
            quantity: *quantity,
            bytes: vec![0; usize::from(*quantity).div_ceil(8)],
        }),
        ModbusPdu::WriteMultipleResponse {
            function: FunctionCode::WriteMultipleRegisters,
            start,
            quantity,
        } => Some(ModbusPdu::WriteMultipleRegisters {
            start: *start,
            registers: vec![0; usize::from(*quantity)],
        }),
        ModbusPdu::WriteFileRecordResponse { records } => Some(ModbusPdu::WriteFileRecord {
            records: records.clone(),
        }),
        _ => None,
    }
}

/// Checks an accepted write request against the bounds and the echo a device would send.
fn check_write_request(request: &ModbusPdu) {
    let echo = match request {
        ModbusPdu::WriteSingleRegister { address, value } => {
            ModbusPdu::WriteSingleRegisterResponse {
                address: *address,
                value: *value,
            }
        }
        ModbusPdu::WriteMultipleCoils {
            start,
            quantity,
            bytes,
        } => {
            assert!((1..=1_968).contains(quantity), "coil quantity {quantity}");
            assert_eq!(bytes.len(), usize::from(*quantity).div_ceil(8), "coil bytes");
            ModbusPdu::WriteMultipleResponse {
                function: FunctionCode::WriteMultipleCoils,
                start: *start,
                quantity: *quantity,
            }
        }
        ModbusPdu::WriteMultipleRegisters { start, registers } => {
            assert!(
                (1..=123).contains(&registers.len()),
                "register count {}",
                registers.len()
            );
            ModbusPdu::WriteMultipleResponse {
                function: FunctionCode::WriteMultipleRegisters,
                start: *start,
                quantity: u16::try_from(registers.len()).unwrap_or(u16::MAX),
            }
        }
        ModbusPdu::WriteFileRecord { records } => {
            let data: usize = records.iter().map(|record| 7 + 2 * record.registers.len()).sum();
            assert!((9..=251).contains(&data), "file record data {data}");
            ModbusPdu::WriteFileRecordResponse {
                records: records.clone(),
            }
        }
        ModbusPdu::ReadFileRecord { records } => {
            assert!(
                (1..=35).contains(&records.len())
                    && records.iter().all(|item| {
                        item.length >= 1
                            && item.record <= 9_999
                            && usize::from(item.record) + usize::from(item.length) <= 10_000
                    }),
                "every sub-request is a run inside its file"
            );
            return;
        }
        _ => return,
    };
    assert_eq!(
        echo.check_reply_to(request),
        Ok(()),
        "the echo of an accepted write request answers it"
    );
    let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
    assert_eq!(encode_response(&echo, &mut writer), Ok(()), "the echo encodes");
}
