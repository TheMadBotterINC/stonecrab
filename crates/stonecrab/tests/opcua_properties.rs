//! Property tests of the OPC UA address grammar (volume set by `PROPTEST_CASES`).
#![cfg(feature = "opcua")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use proptest::prelude::*;
    use stonecrab::opcua::{OpcuaAddress, OpcuaDataType};
    use stonecrab::{ErrorKind, ProtocolAddress};

    const TYPE_NAMES: [&str; 24] = [
        "NULL",
        "BOOL",
        "BYTE",
        "SINT",
        "INT",
        "DINT",
        "LINT",
        "USINT",
        "UINT",
        "UDINT",
        "ULINT",
        "REAL",
        "LREAL",
        "TIME",
        "LTIME",
        "DATE",
        "LDATE",
        "TIME_OF_DAY",
        "LTIME_OF_DAY",
        "DATE_AND_TIME",
        "LDATE_AND_TIME",
        "CHAR",
        "WCHAR",
        "STRING",
    ];

    /// Write `text` as the identifier of a string address: the four special characters
    /// escaped, everything else as it is.
    fn escaped(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            if matches!(c, '\\' | ';' | '[' | '|') {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }

    /// The text of an identifier: `i=`, `s=`, `g=` or `b=` and its value, any of them valid.
    fn identifier() -> impl Strategy<Value = String> {
        prop_oneof![
            any::<u32>().prop_map(|number| format!("i={number}")),
            "[ -~\u{e9}\u{4e16}\u{1f980}]{1,30}".prop_map(|text| format!("s={}", escaped(&text))),
            "[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}"
                .prop_map(|guid| format!("g={guid}")),
            (
                "([A-Za-z0-9+/]{4}){0,9}",
                prop_oneof![
                    "[A-Za-z0-9+/]{4}",
                    // The bits before the pad are zero, as the canonical text writes them.
                    "[A-Za-z0-9+/][AQgw]==",
                    "[A-Za-z0-9+/]{2}[AEIMQUYcgkosw048]=",
                ]
            )
                .prop_map(|(body, last)| format!("b={body}{last}")),
        ]
    }

    /// One valid dimension: an index or a range, with or without a declared base.
    fn dimension() -> impl Strategy<Value = String> {
        (0u32..1000)
            .prop_flat_map(|lower| {
                (
                    Just(lower),
                    lower..1000,
                    0..=lower,
                    any::<bool>(),
                    any::<bool>(),
                )
            })
            .prop_map(|(lower, upper, base, range, with_base)| {
                let mut text = format!("[{lower}");
                if range {
                    write!(text, "..{upper}").expect("writing to a String");
                }
                if with_base {
                    write!(text, ";{base}").expect("writing to a String");
                }
                text.push(']');
                text
            })
    }

    /// An array suffix of up to three valid dimensions.
    fn dimensions() -> impl Strategy<Value = String> {
        prop::collection::vec(dimension(), 0..4).prop_map(|dims| dims.concat())
    }

    fn tail() -> impl Strategy<Value = String> {
        (
            prop::option::of(1u32..=27),
            prop::option::of(prop::sample::select(TYPE_NAMES.to_vec())),
            any::<bool>(),
            prop::collection::vec(("[A-Za-z_-]{1,8}", "[A-Za-z0-9_-]{1,8}"), 0..4),
        )
            .prop_map(|(attribute, type_name, lower_case, config)| {
                let mut text = String::new();
                if let Some(number) = attribute {
                    write!(text, ";a={number}").expect("writing to a String");
                }
                if let Some(name) = type_name {
                    text.push(';');
                    text.push_str(&if lower_case {
                        name.to_lowercase()
                    } else {
                        name.to_owned()
                    });
                }
                for (position, (key, value)) in config.iter().enumerate() {
                    text.push(if position == 0 { '|' } else { ',' });
                    write!(text, "{key}={value}").expect("writing to a String");
                }
                text
            })
    }

    fn address_text() -> impl Strategy<Value = String> {
        (0u16..=u16::MAX, identifier(), dimensions(), tail()).prop_map(
            |(namespace, identifier, dimensions, tail)| {
                format!("ns={namespace};{identifier}{dimensions}{tail}")
            },
        )
    }

    proptest! {
        // covers: OPCUA-ADDR-ROUND-TRIP (the displayed text of a parsed address parses back equal)
        #[test]
        fn opcua_prop_a_displayed_address_parses_back_equal(text in address_text()) {
            let address = OpcuaAddress::parse(&text).expect("a generated address is valid");
            let shown = address.to_string();
            prop_assert_eq!(&shown, &address.canonical(), "Display and canonical agree");
            let again = OpcuaAddress::parse(&shown).expect("the canonical text parses");
            prop_assert_eq!(&again, &address, "equal after a round trip");
            prop_assert_eq!(&again.to_string(), &shown, "the canonical text is a fixed point");
            prop_assert!(shown.len() <= OpcuaAddress::MAX_TEXT_BYTES, "within the bound");
        }

        // covers: OPCUA-ADDR-ROUND-TRIP (a data type written in any case reads the same)
        #[test]
        fn opcua_prop_a_data_type_in_any_case_reads_the_same(
            name in prop::sample::select(TYPE_NAMES.to_vec()),
            mask in any::<u32>(),
        ) {
            let mixed: String = name
                .chars()
                .enumerate()
                .map(|(position, c)| {
                    let flip = (mask >> (position % 32)) & 1 == 1;
                    if flip { c.to_ascii_lowercase() } else { c }
                })
                .collect();
            let address = OpcuaAddress::parse(&format!("ns=1;s=x;{mixed}")).expect("a valid type");
            prop_assert_eq!(address.data_type(), OpcuaDataType::from_name(name).expect("a name"));
            prop_assert_eq!(address.data_type().name(), name);
        }

        // covers: OPCUA-ADDR-ROUND-TRIP (no input panics; every failure is InvalidAddress)
        #[test]
        fn opcua_prop_parse_never_panics_on_any_string(text in any::<String>()) {
            if let Err(error) = OpcuaAddress::parse(&text) {
                prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
            }
        }

        // covers: OPCUA-ADDR-ROUND-TRIP (text built from the grammar's own characters)
        #[test]
        fn opcua_prop_parse_never_panics_on_grammar_characters(
            text in "(ns=)?[0-9;isgb=\\\\\\[\\]|,.a-zA-Z_ -]{0,60}",
        ) {
            match OpcuaAddress::parse(&text) {
                Ok(address) => {
                    prop_assert_eq!(
                        OpcuaAddress::parse(&address.canonical()).ok(),
                        Some(address),
                        "whatever parses has canonical text that parses back equal"
                    );
                }
                Err(error) => prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress),
            }
        }

        // covers: OPCUA-ADDR-ROUND-TRIP (near the 8192-byte bound, where canonical text grows)
        #[test]
        fn opcua_prop_a_long_identifier_parses_or_its_canonical_text_parses_back(
            runs in prop::collection::vec(
                (prop::sample::select(vec!["a", r"\x", "[]", r"\;", r"\|", r"\\"]), 1usize..3000),
                1..5,
            ),
            tail in prop::sample::select(vec!["", ";a=19", ";a=MinimumSamplingInterval;LREAL"]),
        ) {
            let body: String = runs.iter().map(|(unit, count)| unit.repeat(*count)).collect();
            let text = format!("ns=1;s={body}{tail}");
            match OpcuaAddress::parse(&text) {
                Ok(address) => {
                    let shown = address.canonical();
                    prop_assert!(shown.len() <= OpcuaAddress::MAX_TEXT_BYTES, "within the bound");
                    prop_assert_eq!(OpcuaAddress::parse(&shown).ok(), Some(address), "parses back");
                }
                Err(error) => prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress),
            }
        }

        // covers: OPCUA-ADDR-QUERY (no query panics; every failure is InvalidAddress)
        #[test]
        fn opcua_prop_query_never_panics(text in any::<String>()) {
            if let Err(error) = OpcuaAddress::parse_query(&text) {
                prop_assert_eq!(error.kind(), ErrorKind::InvalidAddress);
            }
        }
    }
}
