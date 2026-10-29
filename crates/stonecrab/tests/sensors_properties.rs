//! Property tests of the loader (volume set by `PROPTEST_CASES`).
#![cfg(feature = "sensors")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use stonecrab::sensors::{ProfileId, SensorProfile};

    const FULL: &str = include_str!("profiles/full.toml");

    /// Loading never panics, and whatever loads has a valid id and a schema version that loads (1 or 2).
    fn check(text: &str) {
        if let Ok(profile) = SensorProfile::from_toml(text) {
            assert!(
                ProfileId::new(profile.id.as_str()).is_ok(),
                "loaded ids are valid"
            );
            assert!(
                matches!(profile.schema_version.get(), 1 | 2),
                "only versions 1 and 2 load"
            );
        }
    }

    proptest! {
        // covers: CODING_STANDARDS 13.3 / no input panics the loader
        #[test]
        fn arbitrary_text_never_panics(text in any::<String>()) {
            check(&text);
        }

        #[test]
        fn arbitrary_toml_like_text_never_panics(
            text in "[a-z_\\[\\]{}=,\"'#. \\n0-9-]{0,300}",
        ) {
            check(&text);
        }

        #[test]
        fn a_valid_document_with_one_inserted_character_never_panics(
            at in 0..FULL.len(),
            insert in any::<char>(),
        ) {
            let mut text = String::from(FULL);
            let at = (0..=at).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
            text.insert(at, insert);
            check(&text);
        }

        #[test]
        fn a_valid_document_cut_anywhere_never_panics(cut in 0..FULL.len()) {
            let end = (0..=cut).rev().find(|index| FULL.is_char_boundary(*index)).unwrap_or(0);
            check(FULL.get(..end).unwrap_or_default());
        }

        #[test]
        fn accepted_identifiers_are_snake_case(text in any::<String>()) {
            if let Ok(id) = ProfileId::new(&text) {
                prop_assert!(id.as_str().len() <= ProfileId::MAX_BYTES);
                prop_assert!(id.as_str().starts_with(|c: char| c.is_ascii_lowercase()));
                prop_assert!(id.as_str().bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'));
            }
        }
    }
}
