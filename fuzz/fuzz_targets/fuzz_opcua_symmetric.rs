//! Policy/mode selectors then a protected MSG/CLO chunk, with fixed public fixture nonces.
#![no_main]
use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::symmetric_chunk;

fuzz_target!(|input: &[u8]| {
    if let Ok(roundtrip) = symmetric_chunk(input) {
        assert!(roundtrip.expect("authenticated chunks re-protect and reopen with peer keys"));
    }
});
