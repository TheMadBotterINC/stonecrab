//! Consumer-side checks of the per-tag byte and word order settings.
#![cfg(feature = "testkit")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;
    use std::sync::Arc;

    use stonecrab::testkit::FixtureAddress;
    use stonecrab::{ByteOrder, PlcValueType, Tag, TagId, WordOrder};

    fn tag() -> Tag {
        Tag::new(
            TagId::new("counter").expect("fixture identity"),
            Arc::new(FixtureAddress::new("counter", Some(PlcValueType::U32))),
        )
    }

    #[test]
    fn orders_default_to_absent_and_are_never_inferred() {
        let tag = tag();
        assert_eq!(tag.byte_order(), None, "no default byte order");
        assert_eq!(tag.word_order(), None, "no default word order");
        let debug = format!("{tag:?}");
        assert!(debug.contains("byte_order: None"), "debug shows absence");
        assert!(debug.contains("word_order: None"), "debug shows absence");
    }

    #[test]
    fn each_builder_sets_exactly_one_field() {
        for byte in [ByteOrder::BigEndian, ByteOrder::LittleEndian] {
            let tag = tag().with_byte_order(byte);
            assert_eq!(tag.byte_order(), Some(byte), "byte order set");
            assert_eq!(tag.word_order(), None, "word order untouched");
        }
        for word in [WordOrder::HighWordFirst, WordOrder::LowWordFirst] {
            let tag = tag().with_word_order(word);
            assert_eq!(tag.word_order(), Some(word), "word order set");
            assert_eq!(tag.byte_order(), None, "byte order untouched");
        }
    }

    #[test]
    fn both_orders_combine_independently_of_call_order_and_last_write_wins() {
        let first = tag()
            .with_byte_order(ByteOrder::LittleEndian)
            .with_word_order(WordOrder::HighWordFirst);
        let second = tag()
            .with_word_order(WordOrder::HighWordFirst)
            .with_byte_order(ByteOrder::LittleEndian);
        for tag in [&first, &second] {
            assert_eq!(tag.byte_order(), Some(ByteOrder::LittleEndian));
            assert_eq!(tag.word_order(), Some(WordOrder::HighWordFirst));
        }
        let replaced = first.with_byte_order(ByteOrder::BigEndian);
        assert_eq!(
            replaced.byte_order(),
            Some(ByteOrder::BigEndian),
            "replaced"
        );
        assert_eq!(
            replaced.word_order(),
            Some(WordOrder::HighWordFirst),
            "kept"
        );
    }

    #[test]
    fn orders_do_not_disturb_other_fields_and_survive_clone_and_debug() {
        let base = tag();
        let count = NonZeroU32::new(4).expect("positive");
        let tagged = base
            .with_count(count)
            .with_byte_order(ByteOrder::LittleEndian)
            .with_word_order(WordOrder::LowWordFirst);
        assert_eq!(tagged.id().as_str(), "counter", "identity kept");
        assert_eq!(tagged.count(), count, "count kept");
        assert_eq!(tagged.value_type(), Some(PlcValueType::U32), "hint kept");
        assert_eq!(tagged.address().canonical(), "counter", "address kept");
        let cloned = tagged.clone();
        assert_eq!(cloned.byte_order(), Some(ByteOrder::LittleEndian), "clone");
        assert_eq!(cloned.word_order(), Some(WordOrder::LowWordFirst), "clone");
        let debug = format!("{tagged:?}");
        assert!(debug.contains("byte_order: Some(LittleEndian)"), "{debug}");
        assert!(debug.contains("word_order: Some(LowWordFirst)"), "{debug}");
        let typed = tagged
            .with_type(PlcValueType::U32)
            .expect("matching hint accepted");
        assert_eq!(
            typed.byte_order(),
            Some(ByteOrder::LittleEndian),
            "with_type"
        );
        assert_eq!(
            typed.word_order(),
            Some(WordOrder::LowWordFirst),
            "with_type"
        );
    }
}
