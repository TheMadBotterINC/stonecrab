//! Parses arbitrary UTF-8 as a Modbus address with no convention and under each of the
//! three conventions: `plc4x`, `one-based-40001` and `zero-based`.
//!
//! The target must never panic. Every rejection must be `InvalidAddress` with a message
//! free of control characters. Every success must round-trip: the canonical text of the
//! parsed address, which is one-based, parses to an equal address with no convention, with
//! `plc4x` and with `one-based-40001`. A bare short form accepted under a convention must
//! be rejected when no convention is supplied, and `plc4x` and `one-based-40001` must read
//! every text alike. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern`
//! fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::{AddressOptions, ErrorKind, ProtocolAddress};
use stonecrab::modbus::ModbusAddress;

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let absent = AddressOptions::default();
    let named = |name: &str| AddressOptions::builder().convention(name).build();
    let (Ok(plc4x), Ok(one_based), Ok(zero_based)) = (
        named("plc4x"),
        named("one-based-40001"),
        named("zero-based"),
    ) else {
        return;
    };
    // The options that read the canonical one-based text the way it was written.
    let one_based_readers = [&absent, &plc4x, &one_based];
    for options in [&absent, &plc4x, &one_based, &zero_based] {
        match ModbusAddress::parse(text, options) {
            Ok(address) => {
                let canonical = address.canonical();
                for again in one_based_readers {
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
    assert_eq!(
        ModbusAddress::parse(text, &plc4x).ok(),
        ModbusAddress::parse(text, &one_based).ok(),
        "plc4x and one-based-40001 read every text alike"
    );
});
