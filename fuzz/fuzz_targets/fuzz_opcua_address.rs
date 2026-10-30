//! Parses arbitrary UTF-8 as an OPC UA tag address and as a browse query.
//!
//! The target must never panic. Every rejection must be `InvalidAddress` with a message free
//! of control characters. Every success must round-trip: the canonical text of the parsed
//! address parses to an equal address whose own text is unchanged, stays within the bound,
//! and has the parts its grammar promises (a non-empty identifier, an index range only for
//! an array suffix, key-value pairs with unique keys). A browse query that parses is a
//! node address, or the Objects folder for the empty and `**` queries. (No
//! `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::{OpcuaAddress, OpcuaIdentifier};
use stonecrab::{ErrorKind, ProtocolAddress};

fn check_error(error: &stonecrab::PlcError) {
    assert_eq!(error.kind(), ErrorKind::InvalidAddress);
    assert!(!error.to_string().chars().any(char::is_control));
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    match OpcuaAddress::parse(text) {
        Ok(address) => {
            assert!(text.len() <= OpcuaAddress::MAX_TEXT_BYTES);
            match address.identifier() {
                OpcuaIdentifier::String(identifier) => assert!(!identifier.is_empty()),
                OpcuaIdentifier::ByteString(bytes) => assert!(!bytes.is_empty()),
                _ => {}
            }
            let mut keys: Vec<&str> = address.event_field_names().collect();
            let count = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), count, "tail keys are unique");
            if let Some(range) = address.index_range() {
                assert!(!range.is_empty());
            }
            let canonical = address.canonical();
            assert!(canonical.len() <= OpcuaAddress::MAX_TEXT_BYTES);
            let again = OpcuaAddress::parse(&canonical).expect("canonical text parses");
            assert_eq!(again, address, "canonical text round-trips");
            assert_eq!(again.canonical(), canonical, "canonical text is stable");
            assert_eq!(address.protocol(), "opcua");
            assert_eq!(address.value_type_hint(), address.data_type().value_type());
        }
        Err(error) => check_error(&error),
    }
    match OpcuaAddress::parse_query(text) {
        Ok(query) => {
            if text.trim().is_empty() || text.trim() == "**" {
                assert_eq!(query.canonical(), "ns=0;i=85");
            } else {
                assert_eq!(OpcuaAddress::parse(text).ok(), Some(query));
            }
        }
        Err(error) => check_error(&error),
    }
});
