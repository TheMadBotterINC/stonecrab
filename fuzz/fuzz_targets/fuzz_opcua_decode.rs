//! Drives the recursive and open types of the OPC UA binary encoding (Variant, `DataValue`,
//! `DiagnosticInfo` and the `ExtensionObject` envelope, reached through the `cfg(fuzzing)`
//! entry point `stonecrab::opcua::fuzzing`) over arbitrary bytes.
//!
//! The first byte selects the type and the rest is the input. Parsing must never panic,
//! overflow the stack or reserve memory for a declared count. A rejection is a `ParseError`.
//! Whatever parses must measure as long as it is written, be written in no more bytes than
//! were consumed (a value is read in any accepted form but written in one), and parse back to
//! a value that writes the same bytes (the values are not compared, since a float may be NaN).
//! (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::{RecursiveType, decode, value};

fuzz_target!(|input: &[u8]| {
    let Some((selector, bytes)) = input.split_first() else {
        return;
    };
    // A refusal is the codec doing its job; only what parsed is checked further.
    let kind = RecursiveType::from_selector(*selector);
    if let Ok(decoded) = decode(kind, bytes) {
        if kind == RecursiveType::Variant
            && let Ok(mapped) = value(
                bytes.get(..decoded.consumed).expect("parsed prefix"),
                selector / 4,
            )
        {
            let mut pending = vec![(mapped, 1_usize)];
            let mut nodes = 0_usize;
            while let Some((value, depth)) = pending.pop() {
                nodes = nodes.checked_add(1).expect("bounded node count");
                assert!(nodes <= 1_048_576);
                match value {
                    stonecrab::PlcValue::List(values) => {
                        assert!(depth <= 64);
                        pending.extend(values.into_iter().map(|value| (value, depth.checked_add(1).expect("bounded depth"))));
                    }
                    stonecrab::PlcValue::Struct(value) => {
                        assert!(depth <= 64);
                        pending.extend(value.into_fields().into_iter().map(|(_,value)| (value,depth.checked_add(1).expect("bounded depth"))));
                    }
                    _ => {},
                }
            }
        }
        let written = decoded.written.expect("a value that parsed is written");
        assert_eq!(decoded.length, Ok(written.len()), "length_in_bytes");
        assert!(written.len() <= decoded.consumed, "a value never grows");
        assert!(decoded.consumed <= bytes.len());
        assert_eq!(
            decoded.rewritten.as_ref(),
            Ok(&written),
            "what is written parses and writes the same bytes"
        );
    }
});
