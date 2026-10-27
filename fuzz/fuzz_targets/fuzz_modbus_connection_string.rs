//! Drives `ModbusTcpDriver::parse_config` over arbitrary bytes.
//!
//! `parse_config` takes `&str` and the library never converts bytes lossily, so input that
//! is not valid UTF-8 contributes its longest valid prefix. The target asserts the
//! invariants: parsing never panics; a rejection is always `InvalidConfig` with a message
//! free of control characters that never repeats the authority or a parameter value (a
//! message is a fixed template plus, for an unknown key, the key name, so an ASCII input
//! can legitimately be a fragment of one and a key can appear: the first smoke runs found
//! both, kept as the `regression_*` seeds; the check therefore looks for non-ASCII
//! authority and value text that no key contains, which no template holds); a success carries
//! `ModbusOptions` inside the documented bounds, mirrors the request timeout into
//! `Timeouts.request`, and its redacted display is a single clean line. (No
//! `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern` fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::{ConnectionUri, Driver, ErrorKind};
use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver};

fn check(driver: &ModbusTcpDriver, text: &str) {
    match driver.parse_config(text) {
        Ok(config) => {
            assert_eq!(config.scheme(), "modbus-tcp");
            let options = config
                .driver_options_as::<ModbusOptions>()
                .expect("parse_config attaches the options");
            assert!(!options.request_timeout.is_zero());
            assert_eq!(config.timeouts().request, options.request_timeout);
            assert!((1..=125).contains(&options.max_registers.get()));
            assert!((1..=2000).contains(&options.max_coils.get()));
            let shown = config.redacted_display();
            assert!(!shown.chars().any(char::is_control), "one clean line");
        }
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "only InvalidConfig");
            let message = error.to_string();
            assert!(!message.chars().any(char::is_control), "no control characters");
            // A key name may be echoed (shortened); the authority and the values never are.
            // Templates are ASCII, so a non-ASCII part cannot appear unless it was echoed.
            if let Ok(uri) = ConnectionUri::parse(text) {
                let keys: Vec<&str> = uri.options().iter().map(|(key, _)| key.as_str()).collect();
                let values = uri.options().iter().map(|(_key, value)| value.as_str());
                for part in std::iter::once(uri.authority()).chain(values) {
                    // A part that also occurs inside a key may surface through the key echo.
                    if part.is_ascii() || keys.iter().any(|key| key.contains(part)) {
                        continue;
                    }
                    assert!(
                        !message.contains(part),
                        "an error never repeats the authority or a value"
                    );
                }
            }
        }
    }
}

fuzz_target!(|input: &[u8]| {
    let driver = ModbusTcpDriver::new();
    match std::str::from_utf8(input) {
        Ok(text) => check(&driver, text),
        Err(error) => {
            if let Some(prefix) = input.get(..error.valid_up_to())
                && let Ok(text) = std::str::from_utf8(prefix)
            {
                check(&driver, text);
            }
        }
    }
});
