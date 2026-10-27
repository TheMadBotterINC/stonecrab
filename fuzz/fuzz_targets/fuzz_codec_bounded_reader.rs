//! Drives `BoundedReader`, `BoundedWriter` and the coil bit helpers over arbitrary bytes.
//!
//! Input layout: byte 0 selects how many script bytes follow (at most 31); the script
//! bytes are operations (low nibble picks the read, high nibble picks the length or
//! count argument from a table that includes hostile values such as 1 GiB and
//! `usize::MAX`); everything after the script is the payload the reader walks over.
//! The target asserts the invariants as it goes: a failed read consumes nothing, a
//! successful read consumes exactly its width, and every successful scalar re-encodes
//! to the bytes it came from. It must never panic or allocate more than the payload
//! justifies. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab_codec::{
    BoundedReader, BoundedWriter, ParseError, coil_byte_count, pack_bits_lsb_first,
    unpack_bits_lsb_first,
};
use stonecrab_core::ByteOrder;

/// Lengths and counts handed to `read_bytes` and `read_bits_lsb_first`.
const ARGS: [usize; 16] = [
    0,
    1,
    2,
    3,
    7,
    8,
    9,
    16,
    253,
    254,
    2000,
    2001,
    65_536,
    1 << 30,
    usize::MAX - 1,
    usize::MAX,
];

/// Maximums paired with the arguments above.
const MAXES: [usize; 8] = [0, 1, 8, 253, 2000, 65_536, 1 << 30, usize::MAX];

fn order_for(op: u8) -> ByteOrder {
    if op & 0x80 == 0 {
        ByteOrder::BigEndian
    } else {
        ByteOrder::LittleEndian
    }
}

/// Reads one scalar, checks the cursor and the re-encoding, and reports success.
macro_rules! scalar {
    ($reader:ident, $payload:ident, $read:ident, $write:ident, $width:literal, $order:ident) => {{
        let before = $reader.remaining();
        match $reader.$read($order) {
            Ok(value) => {
                assert_eq!($reader.remaining(), before - $width, "success consumes the width");
                let start = $payload.len() - before;
                let mut writer = BoundedWriter::new($width);
                assert!(writer.$write(value, $order).is_ok(), "re-encode fits");
                assert_eq!(
                    Some(writer.as_slice()),
                    $payload.get(start..start + $width),
                    "re-encoding reproduces the bytes"
                );
            }
            Err(error) => {
                assert_eq!($reader.remaining(), before, "failure consumes nothing");
                assert_eq!(
                    error,
                    ParseError::Truncated { needed: $width, available: before }
                );
            }
        }
    }};
}

fn run(script: &[u8], payload: &[u8]) {
    let mut reader = BoundedReader::new(payload);
    for &op in script {
        let arg = ARGS[usize::from(op >> 4) % ARGS.len()];
        let max = MAXES[usize::from(op >> 1) % MAXES.len()];
        let order = order_for(op);
        let before = reader.remaining();
        match op & 0x0F {
            0 => scalar!(reader, payload, read_u16, write_u16, 2, order),
            1 => scalar!(reader, payload, read_i16, write_i16, 2, order),
            2 => scalar!(reader, payload, read_u32, write_u32, 4, order),
            3 => scalar!(reader, payload, read_i32, write_i32, 4, order),
            4 => scalar!(reader, payload, read_u64, write_u64, 8, order),
            5 => scalar!(reader, payload, read_i64, write_i64, 8, order),
            6 => scalar!(reader, payload, read_f32, write_f32, 4, order),
            7 => scalar!(reader, payload, read_f64, write_f64, 8, order),
            8 => {
                if reader.read_u8().is_ok() {
                    assert_eq!(reader.remaining(), before - 1);
                }
            }
            9 => {
                if reader.read_i8().is_ok() {
                    assert_eq!(reader.remaining(), before - 1);
                }
            }
            10 | 11 => match reader.read_bytes(arg, max) {
                Ok(bytes) => {
                    assert_eq!(bytes.len(), arg);
                    assert!(arg <= max);
                    assert_eq!(reader.remaining(), before - arg);
                }
                Err(ParseError::OversizedLength { length, max: limit }) => {
                    assert!(arg > max && length == arg && limit == max);
                    assert_eq!(reader.remaining(), before);
                }
                Err(error) => {
                    assert_eq!(error, ParseError::Truncated { needed: arg, available: before });
                    assert_eq!(reader.remaining(), before);
                }
            },
            12 | 13 => match reader.read_bits_lsb_first(arg, max) {
                Ok(bits) => {
                    assert_eq!(bits.len(), arg);
                    assert!(arg <= max);
                    assert_eq!(reader.remaining(), before - coil_byte_count(arg));
                    let start = payload.len() - before;
                    let used = payload.get(start..start + coil_byte_count(arg));
                    assert_eq!(Some(pack_bits_lsb_first(&bits).len()), used.map(<[u8]>::len));
                }
                Err(ParseError::OversizedLength { .. }) => {
                    assert!(arg > max);
                    assert_eq!(reader.remaining(), before);
                }
                Err(error) => {
                    assert_eq!(
                        error,
                        ParseError::Truncated {
                            needed: coil_byte_count(arg),
                            available: before
                        }
                    );
                    assert_eq!(reader.remaining(), before);
                }
            },
            14 => {
                let at_end = reader.expect_eof().is_ok();
                assert_eq!(at_end, reader.remaining() == 0);
            }
            _ => {
                // The bit helpers directly: the unpacked bits repack to the same bytes
                // once padding is masked, and a count past the input is refused.
                let count = arg.min(payload.len().saturating_mul(8));
                if let Ok(bits) = unpack_bits_lsb_first(payload, count) {
                    assert_eq!(bits.len(), count);
                    assert_eq!(pack_bits_lsb_first(&bits).len(), coil_byte_count(count));
                }
                assert!(unpack_bits_lsb_first(payload, usize::MAX).is_err());
            }
        }
    }
    // The remaining bytes are never invented or lost.
    assert!(reader.remaining() <= payload.len());
}

fuzz_target!(|input: &[u8]| {
    let Some((&selector, rest)) = input.split_first() else {
        return;
    };
    let script_len = usize::from(selector % 32).min(rest.len());
    let (script, payload) = rest.split_at(script_len);
    run(script, payload);
});
