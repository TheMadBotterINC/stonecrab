//! Drives the `opcua` connection string parser (`parse_config`, reached through the
//! `cfg(fuzzing)` entry point `stonecrab::opcua::fuzzing`) over arbitrary bytes.
//!
//! `parse_config` takes `&str` and the library never converts bytes lossily, so input that
//! is not valid UTF-8 contributes its longest valid prefix. The target asserts the
//! invariants: parsing never panics; a rejection is always `InvalidConfig` with a message
//! free of control characters that never repeats the authority or a parameter value (a
//! message is a fixed ASCII template without quotes plus, for an unknown key, the key
//! between quotes, shortened to its first characters and `...`; so the check requires the
//! quoted text to begin a key, and looks outside the quotes for non-ASCII authority and
//! value text, which no template holds); a success has the `opcua` scheme, a TCP endpoint
//! with a port, an endpoint URL that starts `opc.tcp://`, is within 4096 bytes and ends
//! with the path of the authority verbatim, one of the four supported policy names, a
//! secure policy only with a mode other than `NONE`, and a redacted display that is a
//! single clean line. (No `forbid(unsafe_code)` here: `fuzz_target!` expands to an `extern`
//! fn.)
#![no_main]

use libfuzzer_sys::fuzz_target;
use stonecrab::opcua::fuzzing::parse_config;
use stonecrab::{ConnectionUri, Endpoint, ErrorKind};

fn check(text: &str) {
    match parse_config(text) {
        Ok(parsed) => {
            assert_eq!(parsed.config.scheme(), "opcua");
            let Endpoint::Tcp { port, .. } = parsed.config.endpoint() else {
                panic!("a TCP endpoint");
            };
            assert_ne!(*port, 0);
            assert!(parsed.endpoint_url.starts_with("opc.tcp://"));
            assert!(parsed.endpoint_url.len() <= 4096);
            let uri = ConnectionUri::parse(text).expect("an accepted string is a valid URI");
            assert_eq!(uri.scheme(), "opcua");
            assert!(matches!(uri.transport(), None | Some("tcp")));
            if let Some((_address, path)) = uri.authority().split_once('/') {
                assert!(
                    parsed.endpoint_url.ends_with(&format!("/{path}")),
                    "the path is kept verbatim"
                );
            }
            assert!(
                ["NONE", "Basic256Sha256", "Aes128_Sha256_RsaOaep", "Aes256_Sha256_RsaPss"]
                    .contains(&parsed.security_policy)
            );
            assert!(["NONE", "SIGN", "SIGN_ENCRYPT"].contains(&parsed.message_security));
            assert!(
                parsed.security_policy == "NONE" || parsed.message_security != "NONE",
                "a secure policy signs or encrypts"
            );
            assert!(uri.options().iter().all(|(key, _value)| {
                key == "security-policy" || key == "message-security"
            }));
            let shown = parsed.config.redacted_display();
            assert!(!shown.chars().any(char::is_control), "one clean line");
        }
        Err(error) => {
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "only InvalidConfig");
            let message = error.to_string();
            assert!(!message.chars().any(char::is_control), "no control characters");
            // A key name may be echoed between quotes, shortened with "..."; the authority
            // and the values never are. Templates are ASCII without quotes, so a non-ASCII
            // part outside the quotes cannot appear unless it was echoed.
            if let Ok(uri) = ConnectionUri::parse(text) {
                let (outside, quoted) = match message.split_once('\'') {
                    Some((before, rest)) => match rest.rsplit_once('\'') {
                        Some((inside, after)) => (format!("{before}{after}"), Some(inside)),
                        None => (message.clone(), None),
                    },
                    None => (message.clone(), None),
                };
                if let Some(shown) = quoted {
                    let shown = shown.strip_suffix("...").unwrap_or(shown);
                    assert!(
                        uri.options().iter().any(|(key, _value)| key.starts_with(shown)),
                        "only a key, or its beginning, is quoted"
                    );
                }
                let values = uri.options().iter().map(|(_key, value)| value.as_str());
                for part in std::iter::once(uri.authority()).chain(values) {
                    assert!(
                        part.is_ascii() || !outside.contains(part),
                        "an error never repeats the authority or a value"
                    );
                }
            }
        }
    }
}

fuzz_target!(|input: &[u8]| {
    match std::str::from_utf8(input) {
        Ok(text) => check(text),
        Err(error) => {
            if let Some(prefix) = input.get(..error.valid_up_to())
                && let Ok(text) = std::str::from_utf8(prefix)
            {
                check(text);
            }
        }
    }
});
