//! Decodes arbitrary bytes as a Modbus PDU: as a request, and as the response to each of the
//! four read functions plus two unsupported ones.
//!
//! The target must never panic. Every accepted PDU must stay inside the protocol bounds, must
//! name the function it was decoded for, and must re-encode to exactly the input bytes (so
//! padding bits, byte counts and exception codes are all preserved); an accepted response
//! must also survive the reply check (without a panic) against a request of its function. (No
//! `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::codec::{BoundedReader, BoundedWriter};
use stonecrab::modbus::{
    ExceptionCode, FunctionCode, MAX_PDU_BYTES, ModbusPdu, decode_request, decode_response,
    encode_request, encode_response,
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
    }
    let first = data.first().copied().unwrap_or(0);
    let expected_functions = [
        FunctionCode::ReadCoils,
        FunctionCode::ReadDiscreteInputs,
        FunctionCode::ReadHoldingRegisters,
        FunctionCode::ReadInputRegisters,
        FunctionCode::Unknown(0x10),
        FunctionCode::Unknown(0x14),
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
