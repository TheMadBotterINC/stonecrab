//! Three settings bytes then concatenated MSG chunks. Checks exact fragment order and
//! bounded production reassembly; malformed streams and limit violations are refused.
#![no_main]
use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::chunk_reassembly;

fuzz_target!(|input: &[u8]| {
    if let Ok(result) = chunk_reassembly(input) {
        assert!(result.peak <= result.limit);
        for (actual, expected) in result.bodies {
            assert_eq!(actual, expected);
            assert!(actual.len() <= result.limit);
        }
    }
});
