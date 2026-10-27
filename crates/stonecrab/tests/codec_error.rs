//! `ParseError` display, error-trait and `PlcError` conversion behavior.
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::error::Error;

    use stonecrab::codec::ParseError;
    use stonecrab::{ErrorKind, PlcError};

    fn every_variant() -> [ParseError; 5] {
        [
            ParseError::Truncated {
                needed: 4,
                available: 1,
            },
            ParseError::OversizedLength {
                length: 300,
                max: 253,
            },
            ParseError::InvalidDiscriminator {
                field: "function",
                value: 0,
            },
            ParseError::TrailingBytes { remaining: 2 },
            ParseError::Overflow,
        ]
    }

    #[test]
    fn display_names_sizes_and_fields_but_no_payload() {
        let shown: Vec<String> = every_variant().iter().map(ToString::to_string).collect();
        assert_eq!(
            shown,
            [
                "truncated input: needed 4 bytes, 1 available",
                "length 300 exceeds the maximum of 253",
                "invalid function discriminator 0",
                "2 trailing bytes after the last field",
                "length computation overflowed",
            ]
        );
    }

    #[test]
    fn parse_error_is_a_leaf_error_that_compares_clones_and_hashes() {
        for error in every_variant() {
            assert!(
                error.source().is_none(),
                "{error}: leaf error has no source"
            );
            assert_eq!(error.clone(), error, "{error}: clone equals original");
        }
        let distinct: HashSet<ParseError> = every_variant().into_iter().collect();
        assert_eq!(distinct.len(), 5, "hash agrees with equality");
        assert_ne!(
            ParseError::TrailingBytes { remaining: 1 },
            ParseError::TrailingBytes { remaining: 2 },
            "fields take part in equality"
        );
    }

    // covers: SPEC mandatory-failure-cases / F1 truncated payloads, oversized lengths
    #[test]
    fn plc_error_mapping_is_malformed_data_except_oversized_length() {
        let expected = [
            ErrorKind::MalformedData,
            ErrorKind::ResourceLimit,
            ErrorKind::MalformedData,
            ErrorKind::MalformedData,
            ErrorKind::MalformedData,
        ];
        for (error, kind) in every_variant().into_iter().zip(expected) {
            let message = error.to_string();
            let converted = PlcError::from(error.clone());
            assert_eq!(converted.kind(), kind, "{message}: kind");
            assert_eq!(converted.to_string(), message, "{message}: message");
            let source = converted
                .source()
                .expect("parse error is kept as the source");
            assert_eq!(
                source.downcast_ref::<ParseError>(),
                Some(&error),
                "{message}: source is the original"
            );
        }
    }
}
