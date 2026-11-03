//! The known-service dispatcher, including recursive external bodies and hostile counts.
//! Four little-endian bytes select the service encoding id. Remaining bytes are its body.
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::service;

fuzz_target!(|input: &[u8]| {
    if let Ok(decoded) = service(input) {
        let written = decoded.written.expect("accepted service is writable");
        assert_eq!(decoded.length, Ok(written.len()), "measured length");
        assert!(
            written.len() <= decoded.consumed,
            "canonical encoding does not grow"
        );
        assert!(decoded.consumed <= input.len());
        assert_eq!(
            decoded.rewritten.as_ref(),
            Ok(&written),
            "stable canonical service"
        );
    }
});
