//! The equivalence proof of ADR-0019 decision 8: the generated Modbus codec
//! (`crate::modbus::generated`, from the vendored modbus.mspec) against the hand codec, the
//! shipped one, over the whole corpus.
//!
//! Every input runs in both directions: the request and the response parser. The inputs are
//! every `.hex` vector under `fixtures/modbus/`, the seeds under
//! `fuzz/corpus/fuzz_modbus_pdu/` and `fuzz/corpus/fuzz_modbus_mbap/`, property tests over
//! arbitrary bytes and over arbitrary valid PDUs of the enabled domain, and the `typed_*`
//! vectors for `DataItem`. The rules:
//!
//! - When the hand codec accepts, the generated codec accepts, maps to the same
//!   [`ModbusPdu`] through the hand-written [`to_hand`], and serializes back to the bytes
//!   the hand encoder writes, which are the input.
//! - When the hand codec refuses, the generated codec refuses with the same `ParseError`
//!   variant, unless the refusal is one of the closed list of hand-only checks
//!   ([`HandOnly`]), where the generated codec may accept.
//! - An input of a case the options disable (the write and file record function codes that
//!   S45 and S50 added to the hand codec while ADR-0019 was written, and every other
//!   function code of modbus.mspec outside the read domain) is refused by the generated
//!   codec with `InvalidDiscriminator` naming the disabled case, whatever the hand codec
//!   does: decision 6, and the ADR-0019 addendum of 2026-10-29.
//! - Anything else fails the test.
//!
//! ADUs compare the generated `ModbusTcpADU` with the MBAP framer followed by the PDU codec.
//! `DataItem` is compared with [`TypedDecoder`] on the `typed_*` vectors in the
//! `BIG_ENDIAN` layout, where register bytes are the value's natural bytes; the one piece of
//! register alignment the comparison needs (a lone 8-bit value in the low byte of its
//! register) is applied here, by hand (modbus.mspec lines 313-317).

use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

use bytes::BytesMut;
use proptest::prelude::*;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::modbus::generated::data_item;
use crate::modbus::generated::driver_type::DriverType;
use crate::modbus::generated::modbus_adu::ModbusAdu as GenAdu;
use crate::modbus::generated::modbus_data_type::ModbusDataType as GenType;
use crate::modbus::generated::modbus_error_code::ModbusErrorCode as GenCode;
use crate::modbus::generated::modbus_pdu::ModbusPdu as GenPdu;
use crate::modbus::{
    ExceptionCode, FunctionCode, MbapFramer, ModbusDataType, ModbusPdu, TypedDecoder,
    decode_request, decode_response, encode_request, encode_response,
};
use crate::testkit::{GoldenVector, load_dir};
use crate::transport::{FrameError, Framer};
use crate::{ByteOrder, PlcValue, WordOrder};

/// Room for any serialization of the corpus: larger than any ADU.
const WRITER_CAPACITY: usize = 1024;

/// Which parser an input goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Direction {
    Request,
    Response,
}

const BOTH: [Direction; 2] = [Direction::Request, Direction::Response];

/// The closed list of checks only the hand codec makes (decision 8, and the three the
/// addendum of 2026-10-29 adds, each naming its check). When the hand codec refuses with
/// one of them, the generated codec may accept: modbus.mspec does not express the bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HandOnly {
    /// A read quantity of zero (V1.1b 6.1 to 6.4: 1 to 2000 bits, 1 to 125 registers).
    QuantityZero,
    /// A read quantity above 2000 bits or 125 registers.
    QuantityAboveMaximum,
    /// `start + quantity` past the 65536 addresses.
    SpanPastAddressSpace,
    /// A response byte count of zero.
    ByteCountZero,
    /// An odd byte count in a register response.
    OddRegisterByteCount,
    /// A PDU over 253 bytes (`MAX_PDU_BYTES`, V1.1b 4.1).
    PduOver253,
    /// A response whose function is not the one the request asked for.
    ResponseFunctionNotExpected,
    /// Addendum: a response byte count above 250 (2000 bits and 125 registers both fit 250
    /// bytes, `MB-LIMIT-PDU`); the mspec reads any `uint 8` count.
    ByteCountOver250,
    /// Addendum: an exception PDU given to the request parser. The hand request decoder
    /// knows no function code with the exception bit; the mspec matches `ModbusPDUError` on
    /// `errorFlag` alone, whatever `response` is (modbus.mspec line 82).
    ExceptionInRequestDirection,
    /// Addendum: an exception response for a function code the hand codec does not
    /// implement (`FunctionCode::Unknown`); the mspec's `ModbusPDUError` takes any
    /// `functionFlag`.
    ExceptionForUnimplementedFunction,
    /// An ADU over 260 bytes, which the MBAP framer refuses as `Oversized` before it is
    /// parsed (`MB-LIMIT-ADU`); the mspec has no ADU bound.
    AduOver260,
}

/// The outcome of one comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    /// Both accept, map to the same PDU and serialize to the input.
    Same,
    /// Both refuse with the same variant (at the ADU level: both refuse).
    BothRefused,
    /// The hand codec refuses with a hand-only check.
    HandOnly(HandOnly),
    /// The generated codec refuses a disabled case.
    Disabled,
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn vectors() -> Vec<GoldenVector> {
    load_dir(&repository().join("fixtures/modbus"))
        .expect("fixtures/modbus loads: every file listed, every listed vector present")
}

/// The seed files of one fuzz target, sorted by name.
fn seeds(target: &str) -> Vec<(String, Vec<u8>)> {
    let directory = repository().join("fuzz/corpus").join(target);
    let mut seeds: Vec<(String, Vec<u8>)> = std::fs::read_dir(&directory)
        .expect("the fuzz corpus directory exists")
        .map(|entry| {
            let path = entry.expect("a directory entry").path();
            let name = path.display().to_string();
            (name, std::fs::read(&path).expect("a seed file"))
        })
        .collect();
    seeds.sort();
    assert!(!seeds.is_empty(), "{target} has seeds");
    seeds
}

fn number(vector: &GoldenVector, key: &str) -> Option<u64> {
    vector.expected.as_ref()?.get(key)?.as_u64()
}

fn text<'a>(vector: &'a GoldenVector, key: &str) -> Option<&'a str> {
    vector.expected.as_ref()?.get(key)?.as_str()
}

/// The hand codec on one PDU. A response is matched against `expected`, or against the
/// function its first byte names.
fn hand_pdu(
    bytes: &[u8],
    direction: Direction,
    expected: Option<FunctionCode>,
) -> Result<ModbusPdu, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    match direction {
        Direction::Request => decode_request(&mut reader),
        Direction::Response => {
            let first = bytes.first().copied().unwrap_or(0);
            let expected = expected.unwrap_or_else(|| FunctionCode::from_u8(first & 0x7F));
            decode_response(&mut reader, expected)
        }
    }
}

/// The generated codec on one PDU: the input must be exactly one PDU.
fn generated_pdu(bytes: &[u8], direction: Direction) -> Result<GenPdu, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    let pdu = GenPdu::parse(&mut reader, direction == Direction::Response)?;
    reader.expect_eof()?;
    Ok(pdu)
}

/// Register bytes as registers, high byte first; `None` for an odd length.
fn registers(bytes: &[u8]) -> Option<Vec<u16>> {
    let (pairs, rest) = bytes.as_chunks::<2>();
    rest.is_empty()
        .then(|| pairs.iter().map(|pair| u16::from_be_bytes(*pair)).collect())
}

/// The hand-written mapping from the generated types to [`ModbusPdu`] (decision 8). An
/// exception keeps its `functionFlag` (decision 4); a register response of an odd length,
/// which the hand codec refuses, has no mapping.
fn to_hand(pdu: &GenPdu) -> Option<ModbusPdu> {
    Some(match pdu {
        GenPdu::ModbusPduError {
            function_flag,
            exception_code,
        } => ModbusPdu::Exception {
            function: FunctionCode::from_u8(*function_flag),
            code: ExceptionCode::from_u8(exception_code.value()),
        },
        GenPdu::ModbusPduReadCoilsRequest {
            starting_address,
            quantity,
        } => ModbusPdu::ReadCoils {
            start: *starting_address,
            quantity: *quantity,
        },
        GenPdu::ModbusPduReadDiscreteInputsRequest {
            starting_address,
            quantity,
        } => ModbusPdu::ReadDiscreteInputs {
            start: *starting_address,
            quantity: *quantity,
        },
        GenPdu::ModbusPduReadHoldingRegistersRequest {
            starting_address,
            quantity,
        } => ModbusPdu::ReadHoldingRegisters {
            start: *starting_address,
            quantity: *quantity,
        },
        GenPdu::ModbusPduReadInputRegistersRequest {
            starting_address,
            quantity,
        } => ModbusPdu::ReadInputRegisters {
            start: *starting_address,
            quantity: *quantity,
        },
        GenPdu::ModbusPduReadCoilsResponse { value } => ModbusPdu::ReadBitsResponse {
            function: FunctionCode::ReadCoils,
            bytes: value.clone(),
        },
        GenPdu::ModbusPduReadDiscreteInputsResponse { value } => ModbusPdu::ReadBitsResponse {
            function: FunctionCode::ReadDiscreteInputs,
            bytes: value.clone(),
        },
        GenPdu::ModbusPduReadHoldingRegistersResponse { value } => {
            ModbusPdu::ReadRegistersResponse {
                function: FunctionCode::ReadHoldingRegisters,
                registers: registers(value)?,
            }
        }
        GenPdu::ModbusPduReadInputRegistersResponse { value } => ModbusPdu::ReadRegistersResponse {
            function: FunctionCode::ReadInputRegisters,
            registers: registers(value)?,
        },
    })
}

fn serialize_generated(pdu: &GenPdu) -> Result<Vec<u8>, ParseError> {
    let mut writer = BoundedWriter::new(WRITER_CAPACITY);
    pdu.serialize(&mut writer)?;
    Ok(writer.into_bytes())
}

fn encode_hand(pdu: &ModbusPdu, direction: Direction) -> Result<Vec<u8>, ParseError> {
    let mut writer = BoundedWriter::new(WRITER_CAPACITY);
    match direction {
        Direction::Request => encode_request(pdu, &mut writer)?,
        Direction::Response => encode_response(pdu, &mut writer)?,
    }
    Ok(writer.into_bytes())
}

/// The hand-only check a hand refusal is, if it is one.
fn hand_only(error: &ParseError, direction: Direction) -> Option<HandOnly> {
    let exception_bit = |value: u64| value & 0x80 != 0;
    Some(match (error, direction) {
        (
            ParseError::InvalidDiscriminator {
                field: "quantity",
                value: 0,
            },
            _,
        ) => HandOnly::QuantityZero,
        (
            ParseError::OversizedLength {
                max: 2000 | 125, ..
            },
            _,
        ) => HandOnly::QuantityAboveMaximum,
        (ParseError::OversizedLength { max: 65_536, .. }, _) => HandOnly::SpanPastAddressSpace,
        (
            ParseError::InvalidDiscriminator {
                field: "byte count",
                value: 0,
            },
            _,
        ) => HandOnly::ByteCountZero,
        (
            ParseError::InvalidDiscriminator {
                field: "register byte count",
                ..
            },
            _,
        ) => HandOnly::OddRegisterByteCount,
        (ParseError::OversizedLength { max: 253, .. }, _) => HandOnly::PduOver253,
        (ParseError::OversizedLength { max: 250, .. }, _) => HandOnly::ByteCountOver250,
        (
            ParseError::InvalidDiscriminator {
                field: "function code",
                ..
            },
            Direction::Response,
        ) => HandOnly::ResponseFunctionNotExpected,
        (
            ParseError::InvalidDiscriminator {
                field: "function code",
                value,
            },
            Direction::Request,
        ) if exception_bit(*value) => HandOnly::ExceptionInRequestDirection,
        (
            ParseError::InvalidDiscriminator {
                field: "unsupported function code",
                value,
            },
            Direction::Response,
        ) if exception_bit(*value) => HandOnly::ExceptionForUnimplementedFunction,
        _ => return None,
    })
}

/// Whether the generated codec refused a case the options disable.
fn is_disabled(result: &Result<impl Sized, ParseError>) -> bool {
    matches!(result, Err(ParseError::InvalidDiscriminator { field, .. }) if field.ends_with("(disabled)"))
}

/// Compares the two codecs on one PDU and returns the verdict; any difference outside the
/// rules of the module documentation fails the calling test.
fn compare_pdu(bytes: &[u8], direction: Direction, expected: Option<FunctionCode>) -> Verdict {
    let context = format!("PDU {bytes:02X?} ({direction:?})");
    let hand = hand_pdu(bytes, direction, expected);
    let generated = generated_pdu(bytes, direction);
    if is_disabled(&generated) {
        return Verdict::Disabled;
    }
    let Ok(hand) = hand else {
        let error = hand.expect_err("the hand codec refused");
        if let Some(row) = hand_only(&error, direction) {
            return Verdict::HandOnly(row);
        }
        let variant = generated.as_ref().err().map(std::mem::discriminant);
        assert_eq!(
            variant,
            Some(std::mem::discriminant(&error)),
            "{context}: the hand codec refuses with {error:?}, the generated codec gives {generated:?}"
        );
        return Verdict::BothRefused;
    };
    assert!(
        generated.is_ok(),
        "{context}: the generated codec refuses ({generated:?}) what the hand codec accepts as {hand:?}"
    );
    let generated = generated.expect("asserted above");
    assert_eq!(
        to_hand(&generated).as_ref(),
        Some(&hand),
        "{context}: the mapped PDU"
    );
    assert_eq!(
        serialize_generated(&generated).as_deref(),
        Ok(bytes),
        "{context}: the generated codec writes the input back"
    );
    assert_eq!(
        encode_hand(&hand, direction).as_deref(),
        Ok(bytes),
        "{context}: the hand codec writes the input back"
    );
    Verdict::Same
}

/// Compares the two codecs on a byte stream of ADUs: the MBAP framer cuts ADUs and the PDU
/// codec decodes each; the generated `ModbusTcpADU` parses from the same position. The PDU
/// of every ADU the framer cuts is also compared on its own, where refusal variants are
/// compared exactly.
fn compare_adus(bytes: &[u8], direction: Direction) -> Vec<Verdict> {
    let mut verdicts = Vec::new();
    let mut buffer = BytesMut::from(bytes);
    let mut framer = MbapFramer::new();
    let mut offset = 0_usize;
    loop {
        let rest = bytes.get(offset..).unwrap_or_default();
        let context = format!("ADU {rest:02X?} ({direction:?})");
        let mut reader = BoundedReader::new(rest);
        let generated = GenAdu::parse(
            &mut reader,
            DriverType::ModbusTcp,
            direction == Direction::Response,
        );
        let consumed = rest.len().saturating_sub(reader.remaining());
        let framed = framer.decode(&mut buffer);
        let Ok(Some(adu)) = framed else {
            // The framer refuses, or waits for bytes the generated parser will never get.
            let oversized = matches!(framed, Err(FrameError::Oversized(_)));
            assert!(
                oversized || rest.is_empty() || generated.is_err(),
                "{context}: the generated ADU accepts what the framer gives {framed:?}"
            );
            let verdict = if oversized {
                Verdict::HandOnly(HandOnly::AduOver260)
            } else {
                Verdict::BothRefused
            };
            verdicts.extend((!rest.is_empty()).then_some(verdict));
            return verdicts;
        };
        let pdu = adu.pdu().to_vec();
        verdicts.push(compare_pdu(&pdu, direction, None));
        let hand = hand_pdu(&pdu, direction, None);
        let verdict = if is_disabled(&generated) {
            Verdict::Disabled
        } else if let Err(error) = &hand {
            let row = hand_only(error, direction);
            assert!(
                row.is_some() || generated.is_err(),
                "{context}: the generated ADU is accepted where the PDU codec refuses with {error:?}"
            );
            row.map_or(Verdict::BothRefused, Verdict::HandOnly)
        } else {
            assert!(
                generated.is_ok(),
                "{context}: the generated ADU is refused ({generated:?}) where the framer and the PDU codec accept {hand:?}"
            );
            let generated = generated.expect("asserted above");
            let GenAdu::ModbusTcpAdu {
                transaction_identifier,
                unit_identifier,
                pdu: generated_pdu,
            } = &generated;
            assert_eq!(consumed, adu.wire_len(), "{context}: consumed bytes");
            assert_eq!(
                (*transaction_identifier, *unit_identifier),
                (adu.transaction_id(), adu.unit_id().get()),
                "{context}: the header"
            );
            assert_eq!(
                to_hand(generated_pdu).as_ref(),
                hand.as_ref().ok(),
                "{context}: the PDU"
            );
            let mut writer = BoundedWriter::new(WRITER_CAPACITY);
            assert_eq!(
                generated.serialize(&mut writer),
                Ok(()),
                "{context}: serializes"
            );
            assert_eq!(
                writer.as_slice(),
                rest.get(..adu.wire_len()).unwrap_or_default(),
                "{context}: the generated ADU writes the wire bytes back"
            );
            Verdict::Same
        };
        verdicts.push(verdict);
        offset = offset.saturating_add(adu.wire_len());
    }
}

/// The function code a response vector's expected file names, if any.
fn expected_function(vector: &GoldenVector) -> Option<FunctionCode> {
    number(vector, "function")
        .and_then(|value| u8::try_from(value).ok())
        .map(FunctionCode::from_u8)
}

/// Every comparison over the corpus, in both directions.
fn corpus_verdicts() -> Vec<Verdict> {
    let mut verdicts = Vec::new();
    for vector in vectors() {
        if vector.name.starts_with("tcp/typed_") {
            continue;
        }
        for direction in BOTH {
            if vector.name.starts_with("tcp/adu_") {
                verdicts.extend(compare_adus(&vector.bytes, direction));
            } else {
                let response = direction == Direction::Response
                    && text(&vector, "direction") == Some("response");
                let expected = expected_function(&vector).filter(|_| response);
                verdicts.push(compare_pdu(&vector.bytes, direction, expected));
            }
        }
    }
    for (_, bytes) in seeds("fuzz_modbus_pdu") {
        for direction in BOTH {
            verdicts.push(compare_pdu(&bytes, direction, None));
        }
    }
    for (_, bytes) in seeds("fuzz_modbus_mbap") {
        for direction in BOTH {
            verdicts.extend(compare_adus(&bytes, direction));
        }
    }
    // Inputs of the addendum's rows that the corpus does not hold: an exception for Write
    // Single Coil (FC05, which the hand codec never implements) and for function 0.
    for bytes in [[0x85_u8, 0x01], [0x80, 0x04]] {
        for direction in BOTH {
            verdicts.push(compare_pdu(&bytes, direction, None));
        }
    }
    verdicts
}

// covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
#[test]
fn the_generated_codec_agrees_with_the_hand_codec_on_the_whole_corpus() {
    let all: BTreeSet<Verdict> = corpus_verdicts().into_iter().collect();
    for row in [
        HandOnly::QuantityZero,
        HandOnly::QuantityAboveMaximum,
        HandOnly::SpanPastAddressSpace,
        HandOnly::ByteCountZero,
        HandOnly::OddRegisterByteCount,
        HandOnly::PduOver253,
        HandOnly::ResponseFunctionNotExpected,
        HandOnly::ByteCountOver250,
        HandOnly::ExceptionInRequestDirection,
        HandOnly::ExceptionForUnimplementedFunction,
        HandOnly::AduOver260,
    ] {
        assert!(
            all.contains(&Verdict::HandOnly(row)),
            "{row:?}: a row of the closed list that no input exercises is stale"
        );
    }
    for verdict in [Verdict::Same, Verdict::BothRefused, Verdict::Disabled] {
        assert!(all.contains(&verdict), "{verdict:?} occurs");
    }
}

/// A PDU vector of the read domain: function codes 01 to 04 and their exceptions.
fn read_domain(vector: &GoldenVector) -> Option<(u8, Direction)> {
    let function = u8::try_from(number(vector, "function")?).ok()?;
    let direction = if text(vector, "direction")? == "request" {
        Direction::Request
    } else {
        Direction::Response
    };
    ((1..=4).contains(&function) && !vector.name.starts_with("tcp/adu_"))
        .then_some((function, direction))
}

// covers: SPEC release-gates / generated codec behavior (every enabled variant: a valid and a negative vector)
// Coding standards 17.5: every enabled variant has a valid and a negative vector of its own.
// The negative vector of a variant is a vector both codecs accept, cut by one byte, which
// both must refuse with `Truncated`; the corpus's own negative vectors are compared by the
// whole-corpus test above.
#[test]
fn every_enabled_variant_has_a_valid_and_a_negative_vector() {
    let mut accepted: BTreeSet<(u8, Direction, bool)> = BTreeSet::new();
    let mut refused: BTreeSet<(u8, Direction, bool)> = BTreeSet::new();
    for vector in vectors() {
        let Some((function, direction)) = read_domain(&vector) else {
            continue;
        };
        let exception = number(&vector, "exception").is_some()
            || vector.bytes.first().is_some_and(|byte| byte & 0x80 != 0);
        let key = (function, direction, exception);
        let expected = expected_function(&vector).filter(|_| direction == Direction::Response);
        if compare_pdu(&vector.bytes, direction, expected) != Verdict::Same {
            continue;
        }
        accepted.insert(key);
        let shorter = vector.bytes.split_last().map_or(&[][..], |(_, rest)| rest);
        assert_eq!(
            compare_pdu(shorter, direction, expected),
            Verdict::BothRefused,
            "{}: one byte short",
            vector.name
        );
        let truncated = matches!(
            generated_pdu(shorter, direction),
            Err(ParseError::Truncated { .. })
        );
        assert!(truncated, "{}: one byte short is Truncated", vector.name);
        refused.insert(key);
    }
    for function in 1..=4_u8 {
        for direction in BOTH {
            let key = (function, direction, false);
            assert!(
                accepted.contains(&key),
                "FC{function} {direction:?}: a valid vector"
            );
            assert!(
                refused.contains(&key),
                "FC{function} {direction:?}: a negative vector"
            );
        }
        assert!(
            accepted.contains(&(function, Direction::Response, true)),
            "FC{function}: a valid exception vector"
        );
    }
    for function in 1..=4_u8 {
        assert!(
            refused.contains(&(function, Direction::Response, true)),
            "FC{function}: a negative exception vector"
        );
    }
    let adus: Vec<GoldenVector> = vectors()
        .into_iter()
        .filter(|vector| vector.name.starts_with("tcp/adu_"))
        .collect();
    let parsed = |vector: &GoldenVector| {
        let mut reader = BoundedReader::new(&vector.bytes);
        GenAdu::parse(&mut reader, DriverType::ModbusTcp, false)
            .and_then(|adu| reader.expect_eof().map(|()| adu))
    };
    assert!(
        adus.iter().any(|vector| parsed(vector).is_ok()),
        "a valid ADU"
    );
    assert!(
        adus.iter().any(|vector| parsed(vector).is_err()),
        "a negative ADU"
    );
}

#[test]
fn the_rtu_and_ascii_adus_are_disabled_and_refused() {
    for driver in [DriverType::ModbusRtu, DriverType::ModbusAscii] {
        let result = GenAdu::parse(&mut BoundedReader::new(&[0x01, 0x03]), driver, true);
        assert!(is_disabled(&result), "{driver:?}: {result:?}");
    }
}

/// The generated `ModbusDataType` of a hand one, by the mspec name both carry.
fn generated_type(data_type: ModbusDataType) -> GenType {
    let table = [
        (ModbusDataType::Bool, GenType::Bool),
        (ModbusDataType::Byte, GenType::Byte),
        (ModbusDataType::Word, GenType::Word),
        (ModbusDataType::DWord, GenType::Dword),
        (ModbusDataType::LWord, GenType::Lword),
        (ModbusDataType::SInt, GenType::Sint),
        (ModbusDataType::USInt, GenType::Usint),
        (ModbusDataType::Int, GenType::Int),
        (ModbusDataType::UInt, GenType::Uint),
        (ModbusDataType::DInt, GenType::Dint),
        (ModbusDataType::UDInt, GenType::Udint),
        (ModbusDataType::LInt, GenType::Lint),
        (ModbusDataType::ULInt, GenType::Ulint),
        (ModbusDataType::Real, GenType::Real),
        (ModbusDataType::LReal, GenType::Lreal),
        (ModbusDataType::Char, GenType::Char),
        (ModbusDataType::WChar, GenType::Wchar),
        (ModbusDataType::String, GenType::String),
        (ModbusDataType::WString, GenType::Wstring),
    ];
    table
        .into_iter()
        .find(|(hand, _)| *hand == data_type)
        .map(|(_, generated)| generated)
        .expect("every hand data type has a generated one")
}

/// Values compare by bits, so a NaN payload is compared too.
fn same_value(a: &PlcValue, b: &PlcValue) -> bool {
    match (a, b) {
        (PlcValue::F32(a), PlcValue::F32(b)) => a.to_bits() == b.to_bits(),
        (PlcValue::F64(a), PlcValue::F64(b)) => a.to_bits() == b.to_bits(),
        _ => a == b,
    }
}

/// The register bytes of a lone 8-bit value hold it in the low byte (the `BIG_ENDIAN`
/// layout of docs/parity/modbus.md); every other value's bytes are its natural bytes.
fn natural_bytes(data_type: ModbusDataType, bytes: &[u8]) -> Vec<u8> {
    match (data_type, bytes) {
        (ModbusDataType::Byte | ModbusDataType::SInt | ModbusDataType::USInt, [0, low]) => {
            vec![*low]
        }
        _ => bytes.to_vec(),
    }
}

fn parse_item(bytes: &[u8], data_type: GenType) -> Result<PlcValue, ParseError> {
    let mut reader = BoundedReader::new(bytes);
    let value = data_item::parse(&mut reader, data_type, 0)?;
    reader.expect_eof()?;
    Ok(value)
}

// covers: SPEC code-generation requirement 5 / integer widths, signedness, endianness
#[test]
fn the_generated_data_item_agrees_with_the_typed_decoder_in_the_big_endian_layout() {
    let budget = NonZeroUsize::new(64).expect("non-zero");
    let mut compared: BTreeSet<String> = BTreeSet::new();
    for vector in vectors() {
        if !vector.name.starts_with("tcp/typed_")
            || text(&vector, "byte_order") != Some("BIG_ENDIAN")
        {
            continue;
        }
        let name = text(&vector, "data_type").expect("a data type");
        let data_type = ModbusDataType::from_upstream_name(name).expect("a known data type");
        let generated_type = generated_type(data_type);
        if data_type == ModbusDataType::Bool {
            // A lone bit is not a byte-aligned run: BOOL is disabled (decision 8).
            let result = parse_item(&vector.bytes, generated_type);
            assert!(is_disabled(&result), "{}: {result:?}", vector.name);
            continue;
        }
        let registers = registers(&vector.bytes).expect("whole registers");
        let hand = TypedDecoder::decode_registers(
            &registers,
            data_type,
            ByteOrder::BigEndian,
            WordOrder::HighWordFirst,
            budget,
        );
        let natural = natural_bytes(data_type, &vector.bytes);
        let generated = parse_item(&natural, generated_type);
        assert_eq!(
            hand.is_ok(),
            generated.is_ok(),
            "{}: hand {hand:?}, generated {generated:?}",
            vector.name
        );
        let (Ok(hand), Ok(value)) = (hand, generated) else {
            continue;
        };
        assert!(
            same_value(&hand, &value),
            "{}: {hand:?} and {value:?}",
            vector.name
        );
        let mut writer = BoundedWriter::new(16);
        let written = data_item::serialize(&value, generated_type, 0, &mut writer);
        assert_eq!(written, Ok(()), "{}: serializes", vector.name);
        assert_eq!(writer.as_slice(), natural, "{}: serialized", vector.name);
        let encoded = TypedDecoder::encode_registers(
            &hand,
            data_type,
            ByteOrder::BigEndian,
            WordOrder::HighWordFirst,
        )
        .expect("the hand encoder writes the value");
        assert_eq!(encoded, registers, "{}: hand encoded", vector.name);
        // The negative vector of the case: the same value one byte short.
        let shorter = natural.split_last().map_or(&[][..], |(_, rest)| rest);
        let truncated = matches!(
            parse_item(shorter, generated_type),
            Err(ParseError::Truncated { .. })
        );
        assert!(truncated, "{}: one byte short", vector.name);
        compared.insert(name.to_owned());
    }
    assert_eq!(
        compared.len(),
        14,
        "every enabled numeric case has a valid vector: {compared:?}"
    );
}

#[test]
fn a_data_item_of_a_disabled_or_absent_case_or_of_the_wrong_value_is_refused() {
    for data_type in [
        GenType::Char,
        GenType::Wchar,
        GenType::String,
        GenType::Wstring,
    ] {
        assert!(
            is_disabled(&parse_item(&[0x41], data_type)),
            "{data_type:?} is disabled"
        );
        let mut writer = BoundedWriter::new(16);
        let written = data_item::serialize(&PlcValue::Char('A'), data_type, 1, &mut writer);
        assert!(is_disabled(&written), "{data_type:?} is disabled");
    }
    let mut writer = BoundedWriter::new(16);
    assert!(
        is_disabled(&data_item::serialize(
            &PlcValue::Bool(true),
            GenType::Bool,
            0,
            &mut writer
        )),
        "BOOL is disabled"
    );
    for data_type in [
        GenType::Time,
        GenType::Ltime,
        GenType::Date,
        GenType::Ldate,
        GenType::TimeOfDay,
        GenType::LtimeOfDay,
        GenType::DateAndTime,
        GenType::LdateAndTime,
    ] {
        assert_eq!(
            parse_item(&[0; 8], data_type),
            Err(ParseError::InvalidDiscriminator {
                field: "DataItem",
                value: u64::from(data_type.value())
            }),
            "{data_type:?} has no DataItem case (modbus.mspec lines 319-377)"
        );
    }
    let mut writer = BoundedWriter::new(16);
    assert_eq!(
        data_item::serialize(&PlcValue::U16(1), GenType::Dint, 0, &mut writer),
        Err(ParseError::InvalidDiscriminator {
            field: "DataItem",
            value: 8
        }),
        "a value of another type is refused, not converted"
    );
}

/// The `ModbusDataType` table of modbus.mspec lines 380-408, written out by hand: the wire
/// value and `dataTypeSize` of every value.
#[test]
fn the_generated_enums_carry_the_mspec_values_and_constants() {
    let table = [
        (GenType::Bool, 1, 2),
        (GenType::Byte, 2, 2),
        (GenType::Word, 3, 2),
        (GenType::Dword, 4, 4),
        (GenType::Lword, 5, 8),
        (GenType::Sint, 6, 2),
        (GenType::Int, 7, 2),
        (GenType::Dint, 8, 4),
        (GenType::Lint, 9, 8),
        (GenType::Usint, 10, 2),
        (GenType::Uint, 11, 2),
        (GenType::Udint, 12, 4),
        (GenType::Ulint, 13, 8),
        (GenType::Real, 14, 4),
        (GenType::Lreal, 15, 8),
        (GenType::Time, 16, 8),
        (GenType::Ltime, 17, 8),
        (GenType::Date, 18, 8),
        (GenType::Ldate, 19, 8),
        (GenType::TimeOfDay, 20, 8),
        (GenType::LtimeOfDay, 21, 8),
        (GenType::DateAndTime, 22, 8),
        (GenType::LdateAndTime, 23, 8),
        (GenType::Char, 24, 1),
        (GenType::Wchar, 25, 2),
        (GenType::String, 26, 1),
        (GenType::Wstring, 27, 2),
    ];
    for (data_type, value, size) in table {
        assert_eq!(data_type.value(), value, "{data_type:?} wire value");
        assert_eq!(
            data_type.data_type_size(),
            size,
            "{data_type:?} dataTypeSize"
        );
    }
    // ModbusErrorCode, modbus.mspec lines 410-421, against the hand ExceptionCode.
    for code in 0..=u8::MAX {
        let parsed = GenCode::parse(&mut BoundedReader::new(&[code]));
        let hand = ExceptionCode::from_u8(code);
        match parsed {
            Ok(parsed) => {
                assert_eq!(parsed.value(), code, "{code} round-trips");
                assert_ne!(hand, ExceptionCode::Unknown(code), "{code} is defined");
                let mut writer = BoundedWriter::new(1);
                parsed.serialize(&mut writer).expect("one byte");
                assert_eq!(writer.as_slice(), [code], "{code} serializes");
            }
            Err(error) => {
                assert_eq!(hand, ExceptionCode::Unknown(code), "{code}: {error}");
            }
        }
    }
}

#[test]
fn a_function_flag_above_seven_bits_or_an_oversized_response_is_not_serialized() {
    let error = GenPdu::ModbusPduError {
        function_flag: 0x80,
        exception_code: GenCode::IllegalFunction,
    };
    assert_eq!(
        serialize_generated(&error),
        Err(ParseError::Overflow),
        "decision 4: no truncation"
    );
    let response = GenPdu::ModbusPduReadCoilsResponse {
        value: vec![0; 256],
    };
    assert_eq!(
        serialize_generated(&response),
        Err(ParseError::Overflow),
        "a byte count that does not fit uint 8 fails instead of truncating"
    );
}

/// An arbitrary valid PDU of the enabled domain and its direction.
fn valid_pdu() -> impl Strategy<Value = (ModbusPdu, Direction)> {
    let read = (1_u8..=4, 1_u16..=2000, any::<u16>()).prop_map(|(function, quantity, start)| {
        let max = if function <= 2 { 2000 } else { 125 };
        let quantity = quantity.min(max);
        let start = start.min(u16::MAX.saturating_sub(quantity).saturating_add(1));
        let pdu = match function {
            1 => ModbusPdu::ReadCoils { start, quantity },
            2 => ModbusPdu::ReadDiscreteInputs { start, quantity },
            3 => ModbusPdu::ReadHoldingRegisters { start, quantity },
            _ => ModbusPdu::ReadInputRegisters { start, quantity },
        };
        (pdu, Direction::Request)
    });
    let bits = (1_u8..=2, proptest::collection::vec(any::<u8>(), 1..=250)).prop_map(
        |(function, bytes)| {
            (
                ModbusPdu::ReadBitsResponse {
                    function: FunctionCode::from_u8(function),
                    bytes,
                },
                Direction::Response,
            )
        },
    );
    let words = (3_u8..=4, proptest::collection::vec(any::<u16>(), 1..=125)).prop_map(
        |(function, registers)| {
            (
                ModbusPdu::ReadRegistersResponse {
                    function: FunctionCode::from_u8(function),
                    registers,
                },
                Direction::Response,
            )
        },
    );
    let exception = (
        prop::sample::select(vec![1_u8, 2, 3, 4, 6, 15, 16, 20, 21]),
        prop::sample::select(vec![1_u8, 2, 3, 4, 5, 6, 7, 8, 10, 11]),
    )
        .prop_map(|(function, code)| {
            (
                ModbusPdu::Exception {
                    function: FunctionCode::from_u8(function),
                    code: ExceptionCode::from_u8(code),
                },
                Direction::Response,
            )
        });
    prop_oneof![read, bits, words, exception]
}

proptest! {
    // covers: SPEC mandatory-failure-cases / F1 malformed discriminators, unexpected function codes
    #[test]
    fn prop_the_codecs_agree_on_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..=260)) {
        // A difference fails inside the comparison, with the input in its message.
        for direction in BOTH {
            compare_pdu(&bytes, direction, None);
            compare_adus(&bytes, direction);
        }
    }

    #[test]
    fn prop_the_codecs_agree_on_arbitrary_pdus_in_an_mbap_frame(
        transaction in any::<u16>(),
        unit in any::<u8>(),
        pdu in proptest::collection::vec(any::<u8>(), 1..=253),
    ) {
        let length = u16::try_from(pdu.len().saturating_add(1)).expect("fits");
        let mut adu = Vec::new();
        adu.extend_from_slice(&transaction.to_be_bytes());
        adu.extend_from_slice(&[0, 0]);
        adu.extend_from_slice(&length.to_be_bytes());
        adu.push(unit);
        adu.extend_from_slice(&pdu);
        for direction in BOTH {
            compare_adus(&adu, direction);
        }
    }

    #[test]
    fn prop_every_valid_pdu_of_the_enabled_domain_is_the_same_in_both_codecs((pdu, direction) in valid_pdu()) {
        let bytes = encode_hand(&pdu, direction).expect("a valid PDU encodes");
        prop_assert_eq!(compare_pdu(&bytes, direction, None), Verdict::Same, "{:?}", pdu);
    }
}
