//! Property tests of the connection-string grammar and of credential redaction (volume
//! set by `PROPTEST_CASES`).
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::collection::vec;
    use proptest::prelude::*;
    use stonecrab_core::{ConnectionConfig, ConnectionUri, Credentials, Endpoint};

    /// Options with the first occurrence of each key kept, in generated order.
    fn distinct(pairs: Vec<(String, String)>) -> Vec<(String, String)> {
        let mut kept: Vec<(String, String)> = Vec::new();
        for (key, value) in pairs {
            if kept.iter().all(|(seen, _)| *seen != key) {
                kept.push((key, value));
            }
        }
        kept
    }

    proptest! {
        // covers: CODING_STANDARDS 13.7 every valid form survives parse and display
        #[test]
        fn prop_uri_round_trips_valid_parts(
            scheme in "[a-z0-9-]{1,12}",
            transport in proptest::option::of("[a-z0-9-]{1,12}"),
            authority in "[A-Za-z0-9.:_/\\[\\]-]{1,40}",
            raw_options in vec(("[A-Za-z0-9._-]{1,10}", "[A-Za-z0-9._:/=,?-]{0,10}"), 0..6),
        ) {
            let options = distinct(raw_options);
            let mut text = scheme.clone();
            if let Some(transport) = &transport {
                text.push(':');
                text.push_str(transport);
            }
            text.push_str("://");
            text.push_str(&authority);
            for (index, (key, value)) in options.iter().enumerate() {
                text.push(if index == 0 { '?' } else { '&' });
                text.push_str(key);
                text.push('=');
                text.push_str(value);
            }

            let uri = ConnectionUri::parse(&text).unwrap();
            prop_assert_eq!(uri.scheme(), scheme.as_str());
            prop_assert_eq!(uri.transport(), transport.as_deref());
            prop_assert_eq!(uri.authority(), authority.as_str());
            prop_assert_eq!(uri.options(), options.as_slice());
            for (key, value) in &options {
                prop_assert_eq!(uri.option(key), Some(value.as_str()));
            }
            prop_assert_eq!(uri.to_string(), text);
            prop_assert_eq!(ConnectionUri::parse(&uri.to_string()).unwrap(), uri);
        }

        // covers: CODING_STANDARDS 13.7 arbitrary text never panics and parse is stable
        #[test]
        fn prop_uri_parse_is_total_and_stable(text in "\\PC{0,64}") {
            if let Ok(uri) = ConnectionUri::parse(&text) {
                prop_assert_eq!(ConnectionUri::parse(&uri.to_string()).unwrap(), uri);
            }
        }

        // covers: CODING_STANDARDS 13.7 credentials never reach a log line
        #[test]
        fn prop_redacted_display_never_contains_password(
            user_tail in "\\PC{0,24}",
            password_tail in "\\PC{0,24}",
            port in any::<u16>(),
        ) {
            // The snowman prefix cannot occur in the fixed words of either rendering, so
            // a hit can only be leaked credential text.
            let username = format!("\u{2603}u{user_tail}");
            let password = format!("\u{2603}p{password_tail}");
            let config = ConnectionConfig::builder(
                "sim",
                Endpoint::Tcp { host: "192.0.2.10".to_owned(), port },
            )
            .credentials(Credentials::new(username.clone(), password.clone()))
            .build()
            .unwrap();
            let display = config.redacted_display();
            let debug = format!("{config:?}");
            prop_assert!(display.ends_with("credentials=present"));
            for text in [&display, &debug] {
                prop_assert!(!text.contains(&password), "password leaked: {}", text);
                prop_assert!(!text.contains(&username), "username leaked: {}", text);
                prop_assert!(!text.contains('\u{2603}'), "credential text leaked: {}", text);
            }
        }
    }
}
