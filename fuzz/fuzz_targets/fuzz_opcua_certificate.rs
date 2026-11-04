//! Untrusted certificate properties, canonical DER lengths and ASN.1 time values.
#![no_main]

use std::error::Error as _;

use libfuzzer_sys::fuzz_target;
use stonecrab::ErrorKind;

fuzz_target!(|input: &[u8]| {
    if let Err(error) = stonecrab::opcua::fuzzing::inspect_certificate(input) {
        assert_eq!(error.kind(), ErrorKind::Trust);
        assert!(error.source().is_none());
    }
});
