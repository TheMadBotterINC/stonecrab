//! Batcher plans: grouping, merging, splitting at the limits, validation and ordering.
#![cfg(all(feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};
    use std::sync::Arc;

    use proptest::prelude::*;
    use stonecrab::modbus::{
        BatchLimits, Batcher, Chunk, ModbusAddress, ModbusArea, ModbusDataType, UnitId,
    };
    use stonecrab::testkit::FixtureAddress;
    use stonecrab::{AddressOptions, ByteOrder, ErrorKind, Tag, TagId, WordOrder};

    type Summary<'a> = (Option<u8>, ModbusArea, u16, u16, Vec<&'a str>);

    const HR: ModbusArea = ModbusArea::HoldingRegister;

    fn nz16(value: u16) -> NonZeroU16 {
        NonZeroU16::new(value).expect("non-zero test literal")
    }

    fn nzu(value: usize) -> NonZeroUsize {
        NonZeroUsize::new(value).expect("non-zero test literal")
    }

    fn limits(registers: u16, coils: u16, items: usize) -> BatchLimits {
        BatchLimits::new(nz16(registers), nz16(coils), nzu(items)).expect("limits within protocol")
    }

    fn tag_of(name: &str, address: ModbusAddress) -> Tag {
        Tag::new(TagId::new(name).expect("valid id"), Arc::new(address))
    }

    fn reg(name: &str, area: ModbusArea, offset: u16, data_type: ModbusDataType) -> Tag {
        let address = ModbusAddress::builder(area, offset)
            .data_type(data_type)
            .build()
            .expect("valid address");
        tag_of(name, address)
    }

    fn holding(name: &str, offset: u16, data_type: ModbusDataType) -> Tag {
        reg(name, HR, offset, data_type)
    }

    fn coil(name: &str, offset: u16) -> Tag {
        reg(name, ModbusArea::Coil, offset, ModbusDataType::Bool)
    }

    fn ids(chunk: &Chunk) -> Vec<&str> {
        chunk.members.iter().map(|m| m.tag.as_str()).collect()
    }

    fn plan(tags: &[Tag]) -> Vec<Chunk> {
        Batcher::plan(tags, &BatchLimits::default()).expect("plannable")
    }

    // covers: MB-LIMIT-REGISTERS
    #[test]
    fn plans_1024_tags_within_limits_and_keeps_every_tag_id() {
        let tags: Vec<Tag> = (0..1024_u16)
            .map(|i| holding(&format!("t{i}"), i, ModbusDataType::Int))
            .collect();
        let chunks = plan(&tags);
        assert_eq!(
            chunks.len(),
            9,
            "1024 adjacent registers need ceil(1024/125) reads"
        );
        let mut seen = 0;
        for chunk in &chunks {
            assert!(
                chunk.quantity <= 125,
                "chunk of {} registers",
                chunk.quantity
            );
            seen += chunk.members.len();
        }
        assert_eq!(seen, 1024, "every tag is in exactly one member");
        let first = chunks.first().expect("first chunk");
        assert_eq!((first.start, first.quantity), (0, 125));
        let last = chunks.last().expect("last chunk");
        assert_eq!((last.start, last.quantity), (1000, 24));
    }

    // covers: MB-READ-MERGE
    #[test]
    fn batcher_merges_adjacent_tags() {
        let tags = [
            holding("a", 10, ModbusDataType::Int),
            holding("c", 12, ModbusDataType::DInt),
            holding("b", 11, ModbusDataType::Int),
        ];
        let chunks = plan(&tags);
        assert_eq!(chunks.len(), 1);
        let chunk = chunks.first().expect("one chunk");
        assert_eq!((chunk.start, chunk.quantity), (10, 4));
        assert_eq!(ids(chunk), ["a", "b", "c"], "members ordered by offset");
        let offsets: Vec<u16> = chunk.members.iter().map(|m| m.offset).collect();
        assert_eq!(
            offsets,
            [0, 1, 2],
            "offsets are relative to the chunk start"
        );
    }

    // covers: MB-READ-MERGE
    #[test]
    fn batcher_merges_overlapping_tags() {
        let same_register = [
            holding("first", 5, ModbusDataType::Int),
            holding("second", 5, ModbusDataType::UInt),
        ];
        let chunks = plan(&same_register);
        assert_eq!(chunks.len(), 1);
        let chunk = chunks.first().expect("one chunk");
        assert_eq!((chunk.start, chunk.quantity), (5, 1));
        assert_eq!(
            ids(chunk),
            ["first", "second"],
            "equal offsets keep input order"
        );

        let partial = [
            holding("wide", 20, ModbusDataType::LInt),
            holding("inner", 21, ModbusDataType::DInt),
            holding("tail", 23, ModbusDataType::Int),
        ];
        let chunks = plan(&partial);
        assert_eq!(chunks.len(), 1);
        let chunk = chunks.first().expect("one chunk");
        assert_eq!((chunk.start, chunk.quantity), (20, 4));
        let offsets: Vec<u16> = chunk.members.iter().map(|m| m.offset).collect();
        assert_eq!(offsets, [0, 1, 3]);
    }

    // covers: MB-READ-GAP
    #[test]
    fn batcher_does_not_bridge_gaps() {
        let tags = [
            holding("a", 10, ModbusDataType::Int),
            holding("b", 12, ModbusDataType::Int),
        ];
        let chunks = plan(&tags);
        assert_eq!(chunks.len(), 2, "one unread register between the tags");
        let spans: Vec<(u16, u16)> = chunks.iter().map(|c| (c.start, c.quantity)).collect();
        assert_eq!(spans, [(10, 1), (12, 1)]);
        let wide = [
            holding("a", 10, ModbusDataType::DInt),
            holding("b", 13, ModbusDataType::Int),
        ];
        assert_eq!(plan(&wide).len(), 2, "gap after a two-register tag");
    }

    // covers: MB-LIMIT-REGISTERS
    #[test]
    fn batcher_splits_63_dint_tags_into_two_chunks() {
        let tags: Vec<Tag> = (0..63_u16)
            .map(|i| holding(&format!("d{i}"), i * 2, ModbusDataType::DInt))
            .collect();
        let chunks = plan(&tags);
        assert_eq!(
            chunks.len(),
            2,
            "63 DINT are 126 registers, one over the limit"
        );
        let first = chunks.first().expect("first chunk");
        assert_eq!(
            (first.start, first.quantity, first.members.len()),
            (0, 124, 62)
        );
        let second = chunks.get(1).expect("second chunk");
        assert_eq!((second.start, second.quantity), (124, 2));
        assert_eq!(ids(second), ["d62"]);
        let exact: Vec<Tag> = tags.iter().take(62).cloned().collect();
        assert_eq!(plan(&exact).len(), 1, "62 DINT fit in 124 registers");
    }

    // covers: MB-LIMIT-REGISTERS
    #[test]
    fn batcher_splits_at_max_registers() {
        let build = |count: u16| -> Vec<Tag> {
            (0..count)
                .map(|i| holding(&format!("r{i}"), 1000 + i, ModbusDataType::Word))
                .collect()
        };
        assert_eq!(plan(&build(125)).len(), 1);
        let split = plan(&build(126));
        let quantities: Vec<u16> = split.iter().map(|c| c.quantity).collect();
        assert_eq!(quantities, [125, 1]);
        let small = Batcher::plan(&build(7), &limits(3, 2000, 1024)).expect("plannable");
        let quantities: Vec<u16> = small.iter().map(|c| c.quantity).collect();
        assert_eq!(quantities, [3, 3, 1], "configured register bound is used");
    }

    // covers: MB-LIMIT-COILS
    #[test]
    fn batcher_splits_at_max_coils() {
        let wide = limits(125, 2000, 4096);
        let build =
            |count: u16| -> Vec<Tag> { (0..count).map(|i| coil(&format!("c{i}"), i)).collect() };
        let one = Batcher::plan(&build(2000), &wide).expect("plannable");
        assert_eq!(one.len(), 1);
        assert_eq!(one.first().map(|c| c.quantity), Some(2000));
        let two = Batcher::plan(&build(2001), &wide).expect("plannable");
        let spans: Vec<(u16, u16)> = two.iter().map(|c| (c.start, c.quantity)).collect();
        assert_eq!(spans, [(0, 2000), (2000, 1)]);
        let narrow = Batcher::plan(&build(5), &limits(125, 2, 4096)).expect("plannable");
        let quantities: Vec<u16> = narrow.iter().map(|c| c.quantity).collect();
        assert_eq!(quantities, [2, 2, 1], "configured coil bound is used");
    }

    // covers: MB-READ-MERGE
    #[test]
    fn batcher_groups_by_unit_and_area() {
        let at_unit = |name: &str, area: ModbusArea, offset: u16, unit: Option<u8>| {
            let data_type = area.default_data_type();
            let mut builder = ModbusAddress::builder(area, offset).data_type(data_type);
            if let Some(unit) = unit {
                builder = builder.unit_id(UnitId::from(unit));
            }
            tag_of(name, builder.build().expect("valid address"))
        };
        let tags = [
            at_unit("hr-u2", HR, 1, Some(2)),
            at_unit("ir-default", ModbusArea::InputRegister, 1, None),
            at_unit("hr-default", HR, 0, None),
            at_unit("hr-u1", HR, 0, Some(1)),
            at_unit("di-default", ModbusArea::DiscreteInput, 0, None),
            at_unit("coil-default", ModbusArea::Coil, 1, None),
            at_unit("hr-u2-b", HR, 0, Some(2)),
            at_unit("hr-default-b", HR, 1, None),
        ];
        let chunks = plan(&tags);
        let summary: Vec<Summary<'_>> = chunks
            .iter()
            .map(|c| {
                (
                    c.unit_id.map(UnitId::get),
                    c.area,
                    c.start,
                    c.quantity,
                    ids(c),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (None, ModbusArea::Coil, 1, 1, vec!["coil-default"]),
                (None, ModbusArea::DiscreteInput, 0, 1, vec!["di-default"]),
                (None, HR, 0, 2, vec!["hr-default", "hr-default-b"]),
                (None, ModbusArea::InputRegister, 1, 1, vec!["ir-default"]),
                (Some(1), HR, 0, 1, vec!["hr-u1"]),
                (Some(2), HR, 0, 2, vec!["hr-u2-b", "hr-u2"]),
            ],
            "ordered by unit id, area, start; no merge across units or areas"
        );
    }

    #[test]
    fn explicit_default_unit_does_not_merge_with_unset_unit() {
        let unit_one = ModbusAddress::builder(HR, 1)
            .unit_id(UnitId::DEFAULT)
            .build()
            .expect("valid address");
        let tags = [
            holding("unset", 0, ModbusDataType::Int),
            tag_of("one", unit_one),
        ];
        assert_eq!(
            plan(&tags).len(),
            2,
            "None means the connection default, kept apart"
        );
    }

    #[test]
    fn member_offsets_are_relative_and_carry_type_and_order() {
        let block = ModbusAddress::parse(
            "holding-register:101:REAL{byte-order: 'LITTLE_ENDIAN'}",
            &AddressOptions::default(),
        )
        .expect("valid address");
        let tags = [
            tag_of("real", block),
            holding("int", 102, ModbusDataType::Int),
        ];
        let chunk = plan(&tags).into_iter().next().expect("one chunk");
        assert_eq!((chunk.start, chunk.quantity), (100, 3));
        let real = chunk.members.first().expect("first member");
        assert_eq!(real.offset, 0);
        assert_eq!(real.data_type, ModbusDataType::Real);
        assert_eq!(
            (real.byte_order, real.word_order),
            (Some(ByteOrder::LittleEndian), Some(WordOrder::LowWordFirst))
        );
        let int = chunk.members.get(1).expect("second member");
        assert_eq!((int.offset, int.data_type), (2, ModbusDataType::Int));
        assert_eq!(
            (int.byte_order, int.word_order),
            (None, None),
            "absent order defers to the connection"
        );
    }

    // covers: MB-BYTEORDER-PRECEDENCE
    #[test]
    fn tag_order_fields_win_over_the_address_block() {
        let block = ModbusAddress::parse(
            "holding-register:1:DINT{byte-order: 'LITTLE_ENDIAN'}",
            &AddressOptions::default(),
        )
        .expect("valid address");
        let both = tag_of("both", block.clone())
            .with_byte_order(ByteOrder::BigEndian)
            .with_word_order(WordOrder::HighWordFirst);
        let byte_only = tag_of("byte", block).with_byte_order(ByteOrder::BigEndian);
        let from_tag = plan(&[both]);
        assert_eq!(
            from_tag
                .first()
                .and_then(|c| c.members.first())
                .map(|m| (m.byte_order, m.word_order)),
            Some((Some(ByteOrder::BigEndian), Some(WordOrder::HighWordFirst)))
        );
        let mixed = plan(&[byte_only]);
        assert_eq!(
            mixed
                .first()
                .and_then(|c| c.members.first())
                .map(|m| (m.byte_order, m.word_order)),
            Some((Some(ByteOrder::BigEndian), Some(WordOrder::LowWordFirst))),
            "the missing field comes from the address block"
        );
    }

    // covers: MB-BYTEORDER-PRECEDENCE
    #[test]
    fn a_lone_tag_order_field_leaves_the_other_to_the_connection_default() {
        let tag = holding("lone", 1, ModbusDataType::Int).with_word_order(WordOrder::LowWordFirst);
        let chunks = plan(&[tag]);
        let member = chunks
            .first()
            .and_then(|c| c.members.first())
            .expect("one member");
        assert_eq!(
            (member.byte_order, member.word_order),
            (None, Some(WordOrder::LowWordFirst))
        );
    }

    // covers: SPEC mandatory-failure-cases / F3 invalid register
    #[test]
    fn a_foreign_address_is_rejected() {
        let foreign = Tag::new(
            TagId::new("alien").expect("id"),
            Arc::new(FixtureAddress::new("other:1", None)),
        );
        let error = Batcher::plan(&[foreign], &BatchLimits::default()).expect_err("foreign");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
        assert!(
            error.to_string().contains("alien"),
            "names the tag: {error}"
        );
    }

    // covers: MB-READ-MERGE
    #[test]
    fn a_count_other_than_one_is_rejected() {
        let tag =
            holding("array", 1, ModbusDataType::Int).with_count(NonZeroU32::MIN.saturating_add(1));
        let error = Batcher::plan(&[tag], &BatchLimits::default()).expect_err("count 2");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(
            error.to_string().contains("array"),
            "names the tag: {error}"
        );
    }

    // covers: MB-READ-MERGE
    #[test]
    fn more_tags_than_max_items_are_rejected_before_planning() {
        let foreign = Tag::new(
            TagId::new("alien").expect("id"),
            Arc::new(FixtureAddress::new("other:1", None)),
        );
        let tags = [
            holding("a", 1, ModbusDataType::Int),
            holding("b", 2, ModbusDataType::Int),
            foreign,
        ];
        let tight = limits(125, 2000, 2);
        let error = Batcher::plan(&tags, &tight).expect_err("three tags, two items");
        assert_eq!(
            error.kind(),
            ErrorKind::ResourceLimit,
            "the count check runs before any tag is inspected"
        );
        assert!(
            Batcher::plan(tags.get(..2).expect("two tags"), &tight).is_ok(),
            "exactly max_items fits"
        );
    }

    #[test]
    fn a_plan_at_max_items_keeps_every_member_in_some_chunk() {
        let tags: Vec<Tag> = (0..6_u16)
            .map(|i| holding(&format!("x{i}"), i * 2, ModbusDataType::Int))
            .collect();
        let chunks = Batcher::plan(&tags, &limits(125, 2000, 6)).expect("plannable");
        let members: usize = chunks.iter().map(|c| c.members.len()).sum();
        assert_eq!(members, 6);
        assert!(chunks.iter().all(|c| c.members.len() <= 6));
    }

    // covers: MB-LIMIT-REGISTERS
    #[test]
    fn a_tag_wider_than_the_register_bound_is_a_resource_limit_error() {
        let tag = holding("big", 0, ModbusDataType::LReal);
        let error = Batcher::plan(&[tag], &limits(3, 2000, 8)).expect_err("4 registers over 3");
        assert_eq!(error.kind(), ErrorKind::ResourceLimit);
        assert!(error.to_string().contains("big"), "names the tag: {error}");
        let fits = holding("ok", 0, ModbusDataType::LReal);
        assert!(Batcher::plan(&[fits], &limits(4, 2000, 8)).is_ok());
    }

    #[test]
    fn the_top_of_the_address_map_plans() {
        let tags = [
            holding("last", 65_535, ModbusDataType::Int),
            holding("pair", 65_533, ModbusDataType::DInt),
            coil("top-coil", 65_535),
        ];
        let chunks = plan(&tags);
        assert_eq!(chunks.len(), 2);
        let coil_chunk = chunks.first().expect("coil chunk");
        assert_eq!(
            (coil_chunk.area, coil_chunk.start, coil_chunk.quantity),
            (ModbusArea::Coil, 65_535, 1)
        );
        let register_chunk = chunks.get(1).expect("register chunk");
        assert_eq!((register_chunk.start, register_chunk.quantity), (65_533, 3));
    }

    #[test]
    fn an_empty_plan_is_empty() {
        assert_eq!(plan(&[]).len(), 0);
    }

    // covers: MB-LIMIT-REGISTERS
    #[test]
    fn limits_above_the_protocol_maximum_are_rejected() {
        let too_many_registers =
            BatchLimits::new(nz16(126), nz16(2000), nzu(1)).expect_err("126 registers");
        assert_eq!(too_many_registers.kind(), ErrorKind::InvalidConfig);
        let too_many_coils =
            BatchLimits::new(nz16(125), nz16(2001), nzu(1)).expect_err("2001 coils");
        assert_eq!(too_many_coils.kind(), ErrorKind::InvalidConfig);
        assert!(BatchLimits::new(nz16(125), nz16(2000), nzu(1)).is_ok());
    }

    #[test]
    fn default_limits_are_the_protocol_maxima() {
        let defaults = BatchLimits::default();
        assert_eq!(defaults.max_registers().get(), 125);
        assert_eq!(defaults.max_coils().get(), 2000);
        assert_eq!(defaults.max_items().get(), 1024);
    }

    const AREAS: [ModbusArea; 4] = [
        ModbusArea::Coil,
        ModbusArea::DiscreteInput,
        ModbusArea::HoldingRegister,
        ModbusArea::InputRegister,
    ];

    const TYPES: [ModbusDataType; 15] = [
        ModbusDataType::Bool,
        ModbusDataType::Byte,
        ModbusDataType::Word,
        ModbusDataType::DWord,
        ModbusDataType::LWord,
        ModbusDataType::SInt,
        ModbusDataType::USInt,
        ModbusDataType::Int,
        ModbusDataType::UInt,
        ModbusDataType::DInt,
        ModbusDataType::UDInt,
        ModbusDataType::LInt,
        ModbusDataType::ULInt,
        ModbusDataType::Real,
        ModbusDataType::LReal,
    ];

    /// An address in a small cluster (so merges happen) or anywhere, with a rare unit id.
    fn addresses() -> impl Strategy<Value = Option<ModbusAddress>> {
        (
            proptest::sample::select(AREAS.to_vec()),
            prop_oneof![0_u16..300, 65_300_u16..=65_535, any::<u16>()],
            proptest::sample::select(TYPES.to_vec()),
            proptest::option::of(0_u8..3),
        )
            .prop_map(|(area, offset, listed, unit)| {
                let data_type = if area.is_bit_area() {
                    ModbusDataType::Bool
                } else {
                    listed
                };
                let mut builder = ModbusAddress::builder(area, offset).data_type(data_type);
                if let Some(unit) = unit {
                    builder = builder.unit_id(UnitId::from(unit));
                }
                // A type running past the end of the map is not an address.
                builder.build().ok()
            })
    }

    proptest! {
        // covers: MB-LIMIT-REGISTERS
        #[test]
        fn prop_plan_never_exceeds_limits_and_loses_no_tag(
            generated in proptest::collection::vec(addresses(), 0..200),
            registers in 4_u16..=125,
            coils in 1_u16..=2000,
            slack in 0_usize..4,
        ) {
            let addresses: Vec<ModbusAddress> = generated.into_iter().flatten().collect();
            let tags: Vec<Tag> = addresses
                .iter()
                .enumerate()
                .map(|(i, a)| tag_of(&format!("p{i}"), a.clone()))
                .collect();
            let max_items = tags.len().saturating_add(slack).max(1);
            let bounds = limits(registers, coils, max_items);
            let chunks = Batcher::plan(&tags, &bounds).expect("plannable");

            let mut seen: Vec<String> = Vec::new();
            let mut previous_key = None;
            for chunk in &chunks {
                let limit = if chunk.area.is_bit_area() { coils } else { registers };
                prop_assert!(chunk.quantity >= 1 && chunk.quantity <= limit);
                prop_assert!(chunk.members.len() <= max_items);
                prop_assert!(u32::from(chunk.start).saturating_add(u32::from(chunk.quantity)) <= 65_536);
                let key = (chunk.unit_id, chunk.area, chunk.start);
                if let Some(previous) = previous_key {
                    prop_assert!(previous < key, "chunks strictly ordered by (unit, area, start)");
                }
                previous_key = Some(key);

                let mut covered_to = 0_u32;
                let mut last_offset = 0_u16;
                for member in &chunk.members {
                    let index: usize = member
                        .tag
                        .as_str()
                        .strip_prefix('p')
                        .and_then(|digits| digits.parse().ok())
                        .expect("generated id");
                    let source = addresses.get(index).expect("known tag");
                    prop_assert_eq!(source.unit_id(), chunk.unit_id);
                    prop_assert_eq!(source.area(), chunk.area);
                    prop_assert_eq!(source.data_type(), member.data_type);
                    prop_assert_eq!(u32::from(chunk.start).saturating_add(u32::from(member.offset)), u32::from(source.offset()));
                    let width = if chunk.area.is_bit_area() { 1 } else { source.data_type().register_size() };
                    let end = u32::from(member.offset).saturating_add(u32::from(width));
                    prop_assert!(end <= u32::from(chunk.quantity), "member inside the chunk span");
                    prop_assert!(member.offset >= last_offset, "members ordered by offset");
                    prop_assert!(u32::from(member.offset) <= covered_to, "no unrequested register or bit");
                    last_offset = member.offset;
                    covered_to = covered_to.max(end);
                    seen.push(member.tag.as_str().to_owned());
                }
                prop_assert_eq!(covered_to, u32::from(chunk.quantity), "the span is exactly the union");
            }
            let mut expected: Vec<String> = (0..tags.len()).map(|i| format!("p{i}")).collect();
            seen.sort();
            expected.sort();
            prop_assert_eq!(seen, expected, "every TagId in exactly one member");
        }
    }
}
