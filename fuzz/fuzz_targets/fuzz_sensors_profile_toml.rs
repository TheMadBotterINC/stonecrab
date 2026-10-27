//! Drives `SensorProfile::from_toml` over arbitrary text.
//!
//! Input that is not valid UTF-8 is skipped: `from_toml` takes `&str`, and the library
//! never converts bytes lossily. The target asserts the invariants: loading never
//! panics, an oversized document is refused as `Oversized` before parsing, every
//! rejection converts into an `InvalidConfig` error with a short fixed message, and a
//! profile that loads satisfies the schema's postconditions: version 1, snake_case
//! identifiers within their bound, and a document inside the size bound.
//! (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab_core::{ErrorKind, PlcError};
use stonecrab_sensors::{ProfileError, ProfileId, SensorProfile};

/// Independent restatement of the identifier rule, so the check does not call the code
/// under test: 1 to 64 bytes of ASCII, a lowercase letter first, then lowercase letters,
/// digits or underscores.
fn snake_case(text: &str) -> bool {
    let bytes = text.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes.first().is_some_and(u8::is_ascii_lowercase)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

fn check(text: &str) {
    match SensorProfile::from_toml(text) {
        Ok(profile) => {
            assert!(text.len() <= SensorProfile::MAX_TOML_BYTES, "bound enforced");
            assert_eq!(profile.schema_version.get(), 1, "only version 1 loads");
            assert!(snake_case(profile.id.as_str()), "profile id is snake_case");
            assert!(ProfileId::new(profile.id.as_str()).is_ok(), "id re-validates");
            for measurement in &profile.measurements {
                assert!(snake_case(measurement.name.as_str()), "measurement name is snake_case");
                for field in &measurement.status_fields {
                    assert!(snake_case(&field.name), "status field name is snake_case");
                }
            }
        }
        Err(error) => {
            if text.len() > SensorProfile::MAX_TOML_BYTES {
                assert!(matches!(error, ProfileError::Oversized { .. }), "bound before parsing");
            }
            let plc = PlcError::from(error);
            assert_eq!(plc.kind(), ErrorKind::InvalidConfig, "only InvalidConfig");
            let message = plc.to_string();
            assert!(message.len() < 200, "messages are fixed text, never echoes");
        }
    }
}

fuzz_target!(|input: &[u8]| {
    if let Ok(text) = std::str::from_utf8(input) {
        check(text);
    }
});
