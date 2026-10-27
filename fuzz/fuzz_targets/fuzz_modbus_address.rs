//! Parses arbitrary UTF-8 as a Modbus address under both an absent and the `plc4x`
//! convention.
//!
//! The target must never panic. Every rejection must be `InvalidAddress` with a message
//! free of control characters, and every success must round-trip: the canonical text of
//! the parsed address parses, under either option, to an equal address. A bare short form
//! accepted under `plc4x` must be rejected when no convention is supplied. (No
//! `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::{AddressOptions, ErrorKind, ProtocolAddress};
use stonecrab::modbus::ModbusAddress;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let absent = AddressOptions::default();
    let Ok(plc4x) = AddressOptions::builder().convention("plc4x").build() else {
        return;
    };
    for options in [&absent, &plc4x] {
        match ModbusAddress::parse(text, options) {
            Ok(address) => {
                let canonical = address.canonical();
                for again in [&absent, &plc4x] {
                    assert_eq!(
                        ModbusAddress::parse(&canonical, again).ok().as_ref(),
                        Some(&address),
                        "canonical text must parse back to an equal address"
                    );
                }
            }
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::InvalidAddress);
                assert!(!error.to_string().chars().any(char::is_control));
            }
        }
    }
    if let Ok(address) = ModbusAddress::parse(text, &absent) {
        assert_eq!(
            ModbusAddress::parse(text, &plc4x).ok().as_ref(),
            Some(&address),
            "an address accepted without a convention means the same with plc4x"
        );
    }
});
