//! Property tests of the PDU codec (volume set by `PROPTEST_CASES`).
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use stonecrab::codec::{BoundedReader, BoundedWriter};
    use stonecrab::modbus::{
        FunctionCode, MAX_PDU_BYTES, ModbusArea, ModbusPdu, decode_request, decode_response,
        encode_request, encode_response,
    };

    const READS: [FunctionCode; 4] = [
        FunctionCode::ReadCoils,
        FunctionCode::ReadDiscreteInputs,
        FunctionCode::ReadHoldingRegisters,
        FunctionCode::ReadInputRegisters,
    ];

    const WRITES: [FunctionCode; 3] = [
        FunctionCode::WriteSingleRegister,
        FunctionCode::WriteMultipleCoils,
        FunctionCode::WriteMultipleRegisters,
    ];

    fn request_of(function: FunctionCode, start: u16, quantity: u16) -> ModbusPdu {
        match function.as_u8() {
            1 => ModbusPdu::ReadCoils { start, quantity },
            2 => ModbusPdu::ReadDiscreteInputs { start, quantity },
            3 => ModbusPdu::ReadHoldingRegisters { start, quantity },
            _ => ModbusPdu::ReadInputRegisters { start, quantity },
        }
    }

    proptest! {
        // covers: MB-LIMIT-REGISTERS, MB-LIMIT-PDU
        // covers: SPEC mandatory-failure-cases / F1 oversized lengths
        #[test]
        fn prop_register_count_bounds(
            count in 0_usize..300,
            byte_count in any::<u8>(),
            fill in any::<u8>(),
            function in proptest::sample::select(vec![
                FunctionCode::ReadHoldingRegisters,
                FunctionCode::ReadInputRegisters,
            ]),
        ) {
            // Encoding: a response holds 1 to 125 registers, and what is encoded decodes.
            let registers = vec![u16::from(fill); count];
            let pdu = ModbusPdu::ReadRegistersResponse { function, registers: registers.clone() };
            let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
            let encoded = encode_response(&pdu, &mut writer);
            prop_assert_eq!(encoded.is_ok(), (1..=125).contains(&count), "count {}", count);
            if encoded.is_ok() {
                let decoded = decode_response(&mut BoundedReader::new(writer.as_slice()), function);
                prop_assert_eq!(decoded, Ok(pdu));
            }

            // Decoding: a payload of exactly `byte_count` bytes is a response iff the count is
            // even and within 2..=250.
            let mut bytes = vec![function.as_u8(), byte_count];
            bytes.resize(usize::from(byte_count).saturating_add(2), fill);
            let decoded = decode_response(&mut BoundedReader::new(&bytes), function);
            let valid = byte_count.is_multiple_of(2) && (2..=250).contains(&byte_count);
            prop_assert_eq!(decoded.is_ok(), valid, "byte count {}", byte_count);
            if let Ok(ModbusPdu::ReadRegistersResponse { registers, .. }) = decoded {
                prop_assert_eq!(registers.len().saturating_mul(2), usize::from(byte_count));
            }
        }

        // covers: MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-SPAN
        #[test]
        fn prop_request_round_trips_exactly_when_it_is_legal(
            function in proptest::sample::select(READS.to_vec()),
            start in prop_oneof![any::<u16>(), 65_000_u16..=u16::MAX],
            quantity in prop_oneof![any::<u16>(), 0_u16..=2_100],
        ) {
            let limit = if function.as_u8() <= 2 { 2_000 } else { 125 };
            let end = usize::from(start).saturating_add(usize::from(quantity));
            let legal = quantity >= 1 && quantity <= limit && end <= 65_536;
            let pdu = request_of(function, start, quantity);
            let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
            let encoded = encode_request(&pdu, &mut writer);
            prop_assert_eq!(encoded.is_ok(), legal);
            let [start_hi, start_lo] = start.to_be_bytes();
            let [quantity_hi, quantity_lo] = quantity.to_be_bytes();
            let wire = [function.as_u8(), start_hi, start_lo, quantity_hi, quantity_lo];
            let decoded = decode_request(&mut BoundedReader::new(&wire));
            if legal {
                prop_assert_eq!(writer.as_slice(), wire.as_slice());
                prop_assert_eq!(decoded, Ok(pdu));
            } else {
                prop_assert!(writer.as_slice().is_empty(), "nothing is written for an illegal request");
                prop_assert!(decoded.is_err());
            }
        }

        // covers: MB-TCP-WRITE, MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-SPAN
        // covers: SPEC mandatory-failure-cases / F1 oversized lengths
        #[test]
        fn prop_write_request_is_chosen_encoded_and_decoded_exactly_when_it_is_legal(
            area in proptest::sample::select(vec![ModbusArea::Coil, ModbusArea::HoldingRegister]),
            start in prop_oneof![any::<u16>(), 65_000_u16..=u16::MAX],
            quantity in prop_oneof![any::<u16>(), 0_u16..=130, 1_960_u16..=1_975],
            delta in prop_oneof![Just(0_isize), Just(0), Just(0), Just(-1), Just(1)],
        ) {
            let (limit, exact) = if area == ModbusArea::Coil {
                (1_968_u16, usize::from(quantity).div_ceil(8))
            } else {
                (123, usize::from(quantity).saturating_mul(2))
            };
            let len = exact.saturating_add_signed(delta);
            let payload = vec![0xA5_u8; if len <= 600 { len } else { 0 }];
            let end = usize::from(start).saturating_add(usize::from(quantity));
            let legal = (1..=limit).contains(&quantity) && end <= 65_536 && payload.len() == exact;
            let picked = ModbusPdu::write_request(area, start, quantity, &payload);
            prop_assert_eq!(picked.is_ok(), legal);
            if let Ok(pdu) = picked {
                let expected = if area == ModbusArea::Coil {
                    FunctionCode::WriteMultipleCoils
                } else if payload.len() == 2 {
                    FunctionCode::WriteSingleRegister
                } else {
                    FunctionCode::WriteMultipleRegisters
                };
                prop_assert_eq!(pdu.function(), expected);
                let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
                prop_assert_eq!(encode_request(&pdu, &mut writer), Ok(()));
                prop_assert_eq!(
                    decode_request(&mut BoundedReader::new(writer.as_slice())),
                    Ok(pdu)
                );
            }
        }

        // covers: MB-TCP-WRITE, MB-LIMIT-COILS, MB-LIMIT-REGISTERS, MB-LIMIT-SPAN
        #[test]
        fn prop_write_multiple_response_round_trips_exactly_when_it_is_legal(
            coils in any::<bool>(),
            start in prop_oneof![any::<u16>(), 65_000_u16..=u16::MAX],
            quantity in prop_oneof![any::<u16>(), 0_u16..=130, 1_960_u16..=1_975],
        ) {
            let (function, limit) = if coils {
                (FunctionCode::WriteMultipleCoils, 1_968_u16)
            } else {
                (FunctionCode::WriteMultipleRegisters, 123)
            };
            let end = usize::from(start).saturating_add(usize::from(quantity));
            let legal = (1..=limit).contains(&quantity) && end <= 65_536;
            let [start_hi, start_lo] = start.to_be_bytes();
            let [quantity_hi, quantity_lo] = quantity.to_be_bytes();
            let wire = [function.as_u8(), start_hi, start_lo, quantity_hi, quantity_lo];
            let decoded = decode_response(&mut BoundedReader::new(&wire), function);
            prop_assert_eq!(decoded.is_ok(), legal);
            if let Ok(pdu) = decoded {
                let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
                prop_assert_eq!(encode_response(&pdu, &mut writer), Ok(()));
                prop_assert_eq!(writer.as_slice(), wire.as_slice());
            }
        }

        // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, truncated payloads
        #[test]
        fn prop_decode_never_panics_and_accepted_bytes_re_encode(
            bytes in proptest::collection::vec(any::<u8>(), 0..300),
            tail in proptest::collection::vec(any::<u8>(), 0..8),
        ) {
            let prefixes = [
                vec![],
                vec![0x01],
                vec![0x02],
                vec![0x03],
                vec![0x04],
                vec![0x81],
                vec![0x83],
                vec![0x06],
                vec![0x0F],
                vec![0x10],
                vec![0x86],
                vec![0x8F],
                vec![0x90],
            ];
            for prefix in prefixes {
                let mut input = prefix;
                input.extend_from_slice(&bytes);
                input.extend_from_slice(&tail);
                input.truncate(MAX_PDU_BYTES.saturating_add(2));
                if let Ok(pdu) = decode_request(&mut BoundedReader::new(&input)) {
                    let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
                    prop_assert_eq!(encode_request(&pdu, &mut writer), Ok(()));
                    prop_assert_eq!(writer.as_slice(), input.as_slice());
                }
                for function in READS.into_iter().chain(WRITES) {
                    if let Ok(pdu) = decode_response(&mut BoundedReader::new(&input), function) {
                        let mut writer = BoundedWriter::new(MAX_PDU_BYTES);
                        prop_assert_eq!(encode_response(&pdu, &mut writer), Ok(()));
                        prop_assert_eq!(writer.as_slice(), input.as_slice());
                    }
                }
            }
        }
    }
}
