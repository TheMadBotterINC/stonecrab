//! Drives `ConnectionUri::parse` over arbitrary text.
//!
//! Input that is not valid UTF-8 is skipped: `parse` takes `&str`, and the library never
//! converts bytes lossily. The target asserts the invariants: parsing never panics, a
//! rejection is always `InvalidConfig` with a short fixed message, and a successful
//! parse satisfies the grammar's postconditions and round-trips through `Display` to an
//! equal value whose own text is unchanged. (No `forbid(unsafe_code)` here:
//! `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab_core::{ConnectionUri, ErrorKind};

fn check(text: &str) {
    match ConnectionUri::parse(text) {
        Ok(uri) => {
            assert!(text.len() <= ConnectionUri::MAX_BYTES, "bound enforced");
            assert!(!uri.scheme().is_empty() && !uri.authority().is_empty());
            assert!(!text.contains('%') && !uri.authority().contains('?'));
            let mut keys: Vec<&str> = uri.options().iter().map(|(key, _)| key.as_str()).collect();
            let count = keys.len();
            keys.sort_unstable();
            keys.dedup();
            assert_eq!(keys.len(), count, "keys are unique");
            assert!(keys.first().is_none_or(|key| !key.is_empty()), "keys are non-empty");

            let shown = uri.to_string();
            let again = ConnectionUri::parse(&shown).expect("canonical text parses");
            assert_eq!(again, uri, "display round-trips");
            assert_eq!(again.to_string(), shown, "display is stable");
        }
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "only InvalidConfig");
            let message = error.to_string();
            assert!(message.len() < 200, "messages are fixed text, never echoes");
        }
    }
}

fuzz_target!(|input: &[u8]| {
    if let Ok(text) = std::str::from_utf8(input) {
        check(text);
    }
});
