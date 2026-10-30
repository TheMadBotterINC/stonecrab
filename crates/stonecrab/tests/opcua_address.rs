//! The OPC UA tag address grammar: namespace, identifier types, escapes, array suffix,
//! attribute, data type, key-value tail, canonical text and the browse query.
#![cfg(feature = "opcua")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::HashSet;
    use std::sync::Arc;

    use stonecrab::opcua::{OpcuaAddress, OpcuaAttribute, OpcuaDataType, OpcuaIdentifier};
    use stonecrab::{ErrorKind, PlcValueType, ProtocolAddress, Tag, TagId};

    /// Number, name: one row per standard attribute.
    const ATTRIBUTE_TABLE: [(u32, &str); 27] = [
        (1, "NodeId"),
        (2, "NodeClass"),
        (3, "BrowseName"),
        (4, "DisplayName"),
        (5, "Description"),
        (6, "WriteMask"),
        (7, "UserWriteMask"),
        (8, "IsAbstract"),
        (9, "Symmetric"),
        (10, "InverseName"),
        (11, "ContainsNoLoops"),
        (12, "EventNotifier"),
        (13, "Value"),
        (14, "DataType"),
        (15, "ValueRank"),
        (16, "ArrayDimensions"),
        (17, "AccessLevel"),
        (18, "UserAccessLevel"),
        (19, "MinimumSamplingInterval"),
        (20, "Historizing"),
        (21, "Executable"),
        (22, "UserExecutable"),
        (23, "DataTypeDefinition"),
        (24, "RolePermissions"),
        (25, "UserRolePermissions"),
        (26, "AccessRestrictions"),
        (27, "AccessLevelEx"),
    ];

    /// Suffix name, variant, core type: one row per `;TYPE` name.
    const TYPE_TABLE: [(&str, OpcuaDataType, Option<PlcValueType>); 24] = [
        ("NULL", OpcuaDataType::Null, None),
        ("BOOL", OpcuaDataType::Bool, Some(PlcValueType::Bool)),
        ("BYTE", OpcuaDataType::Byte, Some(PlcValueType::U8)),
        ("SINT", OpcuaDataType::SInt, Some(PlcValueType::I8)),
        ("INT", OpcuaDataType::Int, Some(PlcValueType::I16)),
        ("DINT", OpcuaDataType::DInt, Some(PlcValueType::I32)),
        ("LINT", OpcuaDataType::LInt, Some(PlcValueType::I64)),
        ("USINT", OpcuaDataType::USInt, Some(PlcValueType::U8)),
        ("UINT", OpcuaDataType::UInt, Some(PlcValueType::U16)),
        ("UDINT", OpcuaDataType::UDInt, Some(PlcValueType::U32)),
        ("ULINT", OpcuaDataType::ULInt, Some(PlcValueType::U64)),
        ("REAL", OpcuaDataType::Real, Some(PlcValueType::F32)),
        ("LREAL", OpcuaDataType::LReal, Some(PlcValueType::F64)),
        (
            "TIME",
            OpcuaDataType::Time,
            Some(PlcValueType::SignedDuration),
        ),
        (
            "LTIME",
            OpcuaDataType::LTime,
            Some(PlcValueType::SignedDuration),
        ),
        ("DATE", OpcuaDataType::Date, Some(PlcValueType::Date)),
        ("LDATE", OpcuaDataType::LDate, Some(PlcValueType::Date)),
        (
            "TIME_OF_DAY",
            OpcuaDataType::TimeOfDay,
            Some(PlcValueType::TimeOfDay),
        ),
        (
            "LTIME_OF_DAY",
            OpcuaDataType::LTimeOfDay,
            Some(PlcValueType::TimeOfDay),
        ),
        (
            "DATE_AND_TIME",
            OpcuaDataType::DateAndTime,
            Some(PlcValueType::DateTime),
        ),
        (
            "LDATE_AND_TIME",
            OpcuaDataType::LDateAndTime,
            Some(PlcValueType::DateTime),
        ),
        ("CHAR", OpcuaDataType::Char, Some(PlcValueType::Char)),
        ("WCHAR", OpcuaDataType::WChar, Some(PlcValueType::Char)),
        ("STRING", OpcuaDataType::String, Some(PlcValueType::String)),
    ];

    fn parse(text: &str) -> OpcuaAddress {
        OpcuaAddress::parse(text).unwrap_or_else(|error| panic!("{text:?} should parse: {error}"))
    }

    /// Asserts the text is refused as `InvalidAddress` and returns the message.
    fn refused(text: &str) -> String {
        match OpcuaAddress::parse(text) {
            Ok(address) => panic!("{text:?}: expected a rejection, got {address:?}"),
            Err(error) => {
                assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{text:?}: kind");
                assert!(
                    !error.to_string().chars().any(char::is_control),
                    "{text:?}: the message has no control character"
                );
                error.to_string()
            }
        }
    }

    // covers: OPCUA-ADDR-NAMESPACE
    #[test]
    fn opcua_namespace_zero_and_65535_parse() {
        assert_eq!(parse("ns=0;i=85").namespace(), 0, "namespace 0");
        assert_eq!(parse("ns=65535;i=1").namespace(), 65535, "namespace 65535");
        assert_eq!(
            parse("ns=007;i=1").namespace(),
            7,
            "leading zeros are digits"
        );
        assert_eq!(
            parse("ns=000000000000000000000000000000065535;i=1").namespace(),
            65535,
            "a long run of zeros does not overflow the reader"
        );
    }

    // covers: OPCUA-ADDR-NAMESPACE, OPCUA-DIV-NAMESPACE-RANGE
    #[test]
    fn opcua_namespace_65536_is_invalid_address() {
        let message = refused("ns=65536;i=1");
        assert!(
            message.contains("65536"),
            "the message names the value: {message}"
        );
        assert!(message.contains("65535"), "and the bound: {message}");
        refused("ns=4294967296;i=1");
        refused("ns=99999999999999999999;i=1");
    }

    // covers: OPCUA-ADDR-NAMESPACE
    #[test]
    fn opcua_namespace_with_a_sign_is_invalid_address() {
        for text in [
            "ns=-1;i=1",
            "ns=+1;i=1",
            "ns= 1;i=1",
            "ns=1 ;i=1",
            "ns=;i=1",
            "ns=x;i=1",
            "ns=0x10;i=1",
            "ns=\u{663};i=1",
        ] {
            let message = refused(text);
            assert!(
                message.contains("namespace"),
                "{text:?}: the message names the namespace: {message}"
            );
        }
    }

    #[test]
    fn opcua_an_address_needs_its_namespace_group_and_an_identifier_type() {
        for text in [
            "i=85",
            "ns=2",
            "NS=2;i=1",
            "ns=2;",
            "ns=2;x=1",
            "ns=2;I=1",
            "ns=2;i",
            "ns=2;s",
            "ns=2:s=a",
            "ns=2;;s=a",
            "garbage",
        ] {
            refused(text);
        }
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES
    #[test]
    fn opcua_numeric_identifier_builds_the_full_numeric_node_id() {
        let address = parse("ns=2;i=10846");
        assert_eq!(address.namespace(), 2, "namespace");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::Numeric(10846),
            "a number is a full numeric identifier"
        );
        assert_eq!(
            parse("ns=0;i=4294967295").identifier(),
            &OpcuaIdentifier::Numeric(u32::MAX),
            "the top of the 32-bit range"
        );
        assert_eq!(
            parse("ns=0;i=0").identifier(),
            &OpcuaIdentifier::Numeric(0),
            "zero"
        );
        assert_eq!(
            parse("ns=0;i=0085").identifier(),
            &OpcuaIdentifier::Numeric(85),
            "leading zeros are digits"
        );
        for text in [
            "ns=0;i=4294967296",
            "ns=0;i=-1",
            "ns=0;i=+1",
            "ns=0;i=1.5",
            "ns=0;i=0x10",
            "ns=0;i=1e3",
            "ns=0;i=abc",
            "ns=0;i=99999999999999999999",
        ] {
            let message = refused(text);
            assert!(
                message.contains("numeric identifier"),
                "{text:?}: the message names the kind: {message}"
            );
        }
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES
    #[test]
    fn opcua_string_identifier_builds_a_string_node_id() {
        let address = parse("ns=2;s=foo.bar");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::String("foo.bar".to_owned()),
            "a string identifier"
        );
        assert_eq!(
            parse("ns=2;s=Plant 1/Boiler=3 (main) \u{e9}\u{4e16}").identifier(),
            &OpcuaIdentifier::String("Plant 1/Boiler=3 (main) \u{e9}\u{4e16}".to_owned()),
            "spaces, equals signs and non-ASCII text are part of the identifier"
        );
        assert_eq!(
            parse("ns=2;s= padded ").identifier(),
            &OpcuaIdentifier::String(" padded ".to_owned()),
            "edge spaces are kept"
        );
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES
    #[test]
    fn opcua_guid_identifier_builds_a_guid_node_id() {
        let text = "ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28a";
        let address = parse(text);
        assert!(
            matches!(address.identifier(), OpcuaIdentifier::Guid(_)),
            "a GUID identifier"
        );
        assert_eq!(address.canonical(), text, "lower-case canonical text");
        assert_eq!(
            parse("ns=2;g=09087E75-8E5E-499B-954F-F2A8624DB28A"),
            address,
            "hexadecimal digits are case-insensitive"
        );
        for refused_text in [
            "ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28",
            "ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28ab",
            "ns=2;g=09087e758e5e499b954ff2a8624db28a",
            "ns=2;g=09087e75-8e5e-499b-954f",
            "ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28a-00",
            "ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28g",
            "ns=2;g=+9087e75-8e5e-499b-954f-f2a8624db28a",
            "ns=2;g=9087e75-08e5e-499b-954f-f2a8624db28a",
            "ns=2;g={09087e75-8e5e-499b-954f-f2a8624db28a}",
            "ns=2;g=foo",
        ] {
            let message = refused(refused_text);
            assert!(
                message.contains("GUID"),
                "{refused_text:?}: the message names the kind: {message}"
            );
        }
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES
    #[test]
    fn opcua_guid_identifier_uses_the_mixed_endian_layout() {
        let address = parse("ns=2;g=09087e75-8e5e-499b-954f-f2a8624db28a");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::Guid([
                0x75, 0x7e, 0x08, 0x09, // the first group, little-endian
                0x5e, 0x8e, // the second, little-endian
                0x9b, 0x49, // the third, little-endian
                0x95, 0x4f, 0xf2, 0xa8, 0x62, 0x4d, 0xb2, 0x8a, // the last two, as written
            ]),
            "the 16 bytes of the OPC UA binary encoding"
        );
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES, OPCUA-DIV-BINARY-ID
    #[test]
    fn opcua_binary_identifier_is_a_base64_byte_string_node_id_in_its_namespace() {
        let address = parse("ns=3;b=AQID");
        assert_eq!(address.namespace(), 3, "the namespace is kept, not ignored");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::ByteString(vec![1, 2, 3]),
            "three bytes"
        );
        // RFC 4648 section 10, every padding length.
        for (bytes, encoded) in [
            (&b"f"[..], "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (&[0xfb, 0xff][..], "+/8="),
            (&[0x00][..], "AA=="),
            (&[0xff, 0xff, 0xff][..], "////"),
        ] {
            let text = format!("ns=3;b={encoded}");
            let address = parse(&text);
            assert_eq!(
                address.identifier(),
                &OpcuaIdentifier::ByteString(bytes.to_vec()),
                "{encoded}: decoded"
            );
            assert_eq!(
                address.canonical(),
                text,
                "{encoded}: written back as it came"
            );
        }
        assert_eq!(
            parse("ns=3;b=AQID;INT"),
            parse("ns=3;b=AQID;int"),
            "the identifier ends at the semicolon like any other"
        );
    }

    // covers: OPCUA-ADDR-IDENTIFIER-TYPES, OPCUA-DIV-BINARY-ID, OPCUA-QUIRK-BINARY-ID
    #[test]
    fn opcua_a_binary_identifier_that_is_not_base64_is_invalid_address() {
        for text in [
            // The identifier of upstream's own test: 15 characters and two pads.
            "ns=2;b=asvaewavarahreb==",
            // Upstream's reading of `b=`: a decimal number, which is no padded base64.
            "ns=2;b=12",
            "ns=2;b=1",
            "ns=2;b=A",
            "ns=2;b=AQI",
            "ns=2;b=AQ=",
            "ns=2;b=AQ===",
            "ns=2;b=AQ=A",
            "ns=2;b=A===",
            "ns=2;b====",
            "ns=2;b=AQ!D",
            "ns=2;b=-_8=",
            // Non-zero bits before the pad: another text for [0] and [1, 2].
            "ns=2;b=AB==",
            "ns=2;b=AQJ=",
            "ns=2;b=AQ ID",
            "ns=2;b=AQI\u{e9}",
            "ns=2;b=AQ\\;D",
        ] {
            let message = refused(text);
            assert!(
                message.contains("base64"),
                "{text:?}: the message names base64: {message}"
            );
        }
    }

    // covers: OPCUA-ADDR-IDENTIFIER
    #[test]
    fn opcua_identifier_may_contain_a_bracketed_segment_with_a_semicolon() {
        let array = parse("ns=2;s=Foo[3..8;1]");
        assert_eq!(
            array.identifier(),
            &OpcuaIdentifier::String("Foo".to_owned()),
            "the numeric group is the array suffix"
        );
        assert_eq!(
            array.index_range().as_deref(),
            Some("2:7"),
            "with its ;base"
        );
        let other = parse("ns=2;s=Foo[abc;d]");
        assert_eq!(
            other.identifier(),
            &OpcuaIdentifier::String("Foo[abc;d]".to_owned()),
            "a group that is not an array selection stays in the identifier, semicolon included"
        );
        assert_eq!(other.index_range(), None, "and selects nothing");
        let tail = parse("ns=2;s=Foo[abc;d];DINT");
        assert_eq!(
            tail.data_type(),
            OpcuaDataType::DInt,
            "the type follows the group"
        );
    }

    // covers: OPCUA-ADDR-IDENTIFIER
    #[test]
    fn opcua_identifier_stops_at_the_first_unbracketed_semicolon() {
        let typed = parse("ns=2;s=foo;DINT");
        assert_eq!(
            typed.identifier(),
            &OpcuaIdentifier::String("foo".to_owned()),
            "the type is not part of the identifier"
        );
        assert_eq!(typed.data_type(), OpcuaDataType::DInt, "it is the type");
        let attribute = parse("ns=2;s=foo;a=NodeId");
        assert_eq!(
            attribute.identifier(),
            &OpcuaIdentifier::String("foo".to_owned()),
            "the attribute is not part of the identifier"
        );
        assert_eq!(
            attribute.attribute(),
            OpcuaAttribute::NodeId,
            "it is the attribute"
        );
        refused("ns=2;s=foo;bar;baz");
        refused("ns=2;s=foo;bar baz");
    }

    // covers: OPCUA-DIV-ADDRESS-ESCAPES, OPCUA-ADDR-CONFIG
    #[test]
    fn opcua_a_pipe_ends_the_identifier_like_a_semicolon() {
        let address = parse("ns=2;s=foo|Time=1");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::String("foo".to_owned()),
            "the tail is not part of the identifier"
        );
        assert_eq!(address.config().len(), 1, "it is the tail");
        let list = parse("ns=2;s=a|b=c");
        assert_eq!(
            list.identifier(),
            &OpcuaIdentifier::String("a".to_owned()),
            "a well-formed list after the pipe is the tail, not part of the identifier"
        );
        assert_eq!(
            list.event_field_names().collect::<Vec<_>>(),
            ["b"],
            "its key"
        );
        refused("ns=2;s=a|b");
    }

    // covers: OPCUA-DIV-ADDRESS-ESCAPES
    #[test]
    fn opcua_a_backslash_makes_the_next_special_character_part_of_the_identifier() {
        let address = parse(r"ns=2;s=a\;b\[c\|d\\e");
        assert_eq!(
            address.identifier(),
            &OpcuaIdentifier::String(r"a;b[c|d\e".to_owned()),
            "the four escapes"
        );
        assert_eq!(
            address.index_range(),
            None,
            "an escaped bracket opens no group"
        );
        assert_eq!(
            address.canonical(),
            r"ns=2;s=a\;b\[c\|d\\e",
            "and the canonical text writes them again"
        );
        assert_eq!(
            parse(r"ns=2;s=C:\temp\x\").identifier(),
            &OpcuaIdentifier::String(r"C:\temp\x\".to_owned()),
            "a backslash before any other character, or at the end, is itself"
        );
        assert_eq!(
            parse(r"ns=2;s=a\]b").identifier(),
            &OpcuaIdentifier::String(r"a\]b".to_owned()),
            "a backslash does not escape a closing bracket"
        );
        assert_eq!(
            parse(r"ns=2;s=C:\temp").canonical(),
            r"ns=2;s=C:\\temp",
            "the canonical text doubles every backslash"
        );
    }

    #[test]
    fn opcua_an_escaped_bracket_before_digits_is_a_string_not_an_array() {
        let literal = parse(r"ns=2;s=Foo\[3]");
        assert_eq!(
            literal.identifier(),
            &OpcuaIdentifier::String("Foo[3]".to_owned()),
            "the text is Foo[3]"
        );
        assert_eq!(literal.index_range(), None, "no array selection");
        let array = parse("ns=2;s=Foo[3]");
        assert_ne!(literal, array, "an escaped bracket and an array differ");
        assert_eq!(
            array.index_range().as_deref(),
            Some("3"),
            "the array selects element 3"
        );
    }

    #[test]
    fn opcua_bracket_groups_that_are_not_arrays_stay_in_the_identifier() {
        for (text, identifier, range) in [
            ("ns=2;s=Foo[abc]", "Foo[abc]", None),
            ("ns=2;s=Foo[]", "Foo[]", None),
            ("ns=2;s=Foo[1.5]", "Foo[1.5]", None),
            ("ns=2;s=Foo[-1]", "Foo[-1]", None),
            ("ns=2;s=Foo[1..]", "Foo[1..]", None),
            ("ns=2;s=Foo[1;2;3]", "Foo[1;2;3]", None),
            ("ns=2;s=Foo[1,]", "Foo[1,]", None),
            ("ns=2;s=Foo[1][x]", "Foo[1][x]", None),
            ("ns=2;s=Foo[1]bar", "Foo[1]bar", None),
            ("ns=2;s=Foo[1][2]bar[3]", "Foo[1][2]bar", Some("3")),
            ("ns=2;s=Foo[[1]", "Foo[[1]", None),
            ("ns=2;s=a]b", "a]b", None),
            ("ns=2;s=a]", "a]", None),
        ] {
            let address = parse(text);
            assert_eq!(
                address.identifier(),
                &OpcuaIdentifier::String(identifier.to_owned()),
                "{text:?}: identifier"
            );
            assert_eq!(address.index_range().as_deref(), range, "{text:?}: range");
        }
    }

    #[test]
    fn opcua_an_unterminated_bracket_is_invalid_address() {
        for text in [
            "ns=2;s=Foo[1",
            "ns=2;s=[",
            "ns=2;s=Foo[1;2",
            "ns=2;i=1[;DINT",
        ] {
            let message = refused(text);
            assert!(
                message.contains("closing"),
                "{text:?}: the message says what is missing: {message}"
            );
        }
    }

    #[test]
    fn opcua_an_empty_identifier_is_invalid_address() {
        for text in [
            "ns=2;s=",
            "ns=2;i=",
            "ns=2;g=",
            "ns=2;b=",
            "ns=2;s=;DINT",
            "ns=2;s=|a=1",
            "ns=2;s=[3]",
            "ns=2;i=[3]",
            "ns=2;s=[1..2][3]",
        ] {
            let message = refused(text);
            assert!(
                message.contains("empty"),
                "{text:?}: the message says it is empty: {message}"
            );
        }
    }

    // covers: OPCUA-ADDR-ATTRIBUTE
    #[test]
    fn opcua_attribute_by_name_and_by_number_agree() {
        for (number, name) in ATTRIBUTE_TABLE {
            let by_name = parse(&format!("ns=1;i=1;a={name}"));
            let by_number = parse(&format!("ns=1;i=1;a={number}"));
            assert_eq!(by_name, by_number, "{name}: name and number {number} agree");
            assert_eq!(by_name.attribute().number(), number, "{name}: number");
            assert_eq!(by_name.attribute().name(), name, "{name}: name");
            assert_eq!(
                OpcuaAttribute::from_number(number),
                Some(by_name.attribute()),
                "{name}: from_number"
            );
            assert_eq!(
                OpcuaAttribute::from_name(name),
                Some(by_name.attribute()),
                "{name}: from_name"
            );
        }
        assert_eq!(
            parse("ns=1;i=1;a=013").attribute(),
            OpcuaAttribute::Value,
            "leading zeros are digits"
        );
        assert_eq!(
            parse("ns=1;i=1;a=DisplayName;REAL").data_type(),
            OpcuaDataType::Real,
            "the type follows the attribute"
        );
    }

    // covers: OPCUA-ADDR-ATTRIBUTE
    #[test]
    fn opcua_unknown_attribute_name_is_invalid_address() {
        for text in [
            "ns=1;i=1;a=value",
            "ns=1;i=1;a=VALUE",
            "ns=1;i=1;a=Bogus",
            "ns=1;i=1;a=",
            "ns=1;i=1;a=0",
            "ns=1;i=1;a=28",
            "ns=1;i=1;a=-1",
            "ns=1;i=1;a=99999999999",
            "ns=1;i=1;a=Value Value",
        ] {
            let message = refused(text);
            assert!(
                message.contains("attribute"),
                "{text:?}: the message names the attribute: {message}"
            );
        }
        assert_eq!(
            OpcuaAttribute::from_name("value"),
            None,
            "names are case-sensitive"
        );
        assert_eq!(
            OpcuaAttribute::from_number(0),
            None,
            "there is no attribute 0"
        );
        assert_eq!(
            OpcuaAttribute::from_number(28),
            None,
            "there is no attribute 28"
        );
    }

    // covers: OPCUA-ADDR-ATTRIBUTE
    #[test]
    fn opcua_default_attribute_is_value() {
        let address = parse("ns=2;i=10846");
        assert_eq!(address.attribute(), OpcuaAttribute::Value, "default");
        assert_eq!(address.attribute().number(), 13, "Value is attribute 13");
        assert_eq!(
            address.canonical(),
            "ns=2;i=10846",
            "the default is left out of the canonical text"
        );
        assert_eq!(
            parse("ns=2;i=10846;a=Value"),
            address,
            "writing the default changes nothing"
        );
        assert_eq!(
            parse("ns=2;i=10846;a=NodeId").canonical(),
            "ns=2;i=10846;a=NodeId",
            "another attribute is written by name"
        );
    }

    // covers: OPCUA-ADDR-INDEX-RANGE
    #[test]
    fn opcua_array_suffix_3_to_8_with_base_1_becomes_index_range_2_to_7() {
        let address = parse("ns=2;s=samples[3..8;1]");
        assert_eq!(
            address.index_range().as_deref(),
            Some("2:7"),
            "0-based, inclusive"
        );
        assert_eq!(
            parse("ns=2;s=samples[0..7]").index_range().as_deref(),
            Some("0:7"),
            "base 0 by default"
        );
        assert_eq!(
            parse("ns=2;s=samples[0..7;0]").canonical(),
            "ns=2;s=samples[0..7]",
            "a base of 0 is the default and is left out"
        );
        assert_eq!(
            address.canonical(),
            "ns=2;s=samples[3..8;1]",
            "the base is kept in the canonical text"
        );
        assert_eq!(
            parse("ns=2;i=7[2..2147483646]").index_range().as_deref(),
            Some("2:2147483646"),
            "any identifier type takes a suffix"
        );
    }

    // covers: OPCUA-ADDR-INDEX-RANGE
    #[test]
    fn opcua_a_single_element_index_is_not_a_range() {
        let single = parse("ns=2;s=samples[4]");
        assert_eq!(single.index_range().as_deref(), Some("4"), "n, not n:n");
        assert_eq!(
            parse("ns=2;s=samples[4;1]").index_range().as_deref(),
            Some("3"),
            "the base applies to a single element"
        );
        let one_element_range = parse("ns=2;s=samples[4..4]");
        assert_eq!(
            one_element_range.index_range().as_deref(),
            Some("4"),
            "a range of one element is also written n"
        );
        assert_ne!(
            single, one_element_range,
            "but [4] is a scalar and [4..4] an array of one"
        );
        assert_eq!(
            single.canonical(),
            "ns=2;s=samples[4]",
            "[4] is written back as [4]"
        );
        assert_eq!(
            one_element_range.canonical(),
            "ns=2;s=samples[4..4]",
            "[4..4] is written back as a range"
        );
    }

    // covers: OPCUA-ADDR-INDEX-RANGE
    #[test]
    fn opcua_multi_dimensional_suffix_is_comma_separated() {
        let brackets = parse("ns=2;s=m[1..2][3..4]");
        let commas = parse("ns=2;s=m[1..2,3..4]");
        assert_eq!(
            brackets.index_range().as_deref(),
            Some("1:2,3:4"),
            "one entry per dimension"
        );
        assert_eq!(brackets, commas, "both spellings are the same selection");
        assert_eq!(
            commas.canonical(),
            "ns=2;s=m[1..2][3..4]",
            "the comma spelling is read but never written"
        );
        assert_eq!(
            parse("ns=2;s=m[1;1,2..3;2][5]").index_range().as_deref(),
            Some("0,0:1,5"),
            "each dimension has its own base, and the groups concatenate"
        );
    }

    #[test]
    fn opcua_array_suffix_bounds_are_checked() {
        for (text, needle) in [
            ("ns=2;s=m[8..3]", "below its lower bound"),
            ("ns=2;s=m[2;3]", "below the declared lower bound"),
            ("ns=2;s=m[2..8;3]", "below the declared lower bound"),
            ("ns=2;s=m[2147483648]", "above 2147483647"),
            ("ns=2;s=m[0..2147483648]", "above 2147483647"),
            ("ns=2;s=m[0..9;2147483648]", "above 2147483647"),
            ("ns=2;s=m[99999999999999999999]", "above 2147483647"),
            ("ns=2;s=m[0..2147483647]", "more elements"),
            ("ns=2;s=m[1..2][3..2]", "below its lower bound"),
        ] {
            let message = refused(text);
            assert!(
                message.contains(needle),
                "{text:?}: expected {needle:?} in: {message}"
            );
        }
        assert_eq!(
            parse("ns=2;s=m[1..2147483647]").index_range().as_deref(),
            Some("1:2147483647"),
            "the largest range that can be counted"
        );
        assert_eq!(
            parse("ns=2;s=m[2147483647]").index_range().as_deref(),
            Some("2147483647"),
            "the largest index"
        );
    }

    // covers: OPCUA-ADDR-DATATYPE
    #[test]
    fn opcua_all_24_data_type_suffixes_parse() {
        let mut seen = HashSet::new();
        for (name, data_type, value_type) in TYPE_TABLE {
            let address = parse(&format!("ns=2;s=x;{name}"));
            assert_eq!(address.data_type(), data_type, "{name}: variant");
            assert_eq!(address.value_type_hint(), value_type, "{name}: hint");
            assert_eq!(data_type.name(), name, "{name}: name");
            assert_eq!(data_type.value_type(), value_type, "{name}: value_type");
            assert_eq!(
                OpcuaDataType::from_name(name),
                Some(data_type),
                "{name}: from_name"
            );
            assert_eq!(
                parse(&format!("ns=2;s=x;{}", name.to_ascii_lowercase())),
                address,
                "{name}: the suffix is upper-cased before it is matched"
            );
            assert!(seen.insert(data_type), "{name}: each variant appears once");
        }
        assert_eq!(seen.len(), 24, "24 names");
        assert_eq!(
            parse("ns=2;s=x;Time_Of_Day").data_type(),
            OpcuaDataType::TimeOfDay,
            "mixed case"
        );
        assert_eq!(
            parse("ns=2;s=x;a=Value;DINT").canonical(),
            "ns=2;s=x;DINT",
            "the canonical text writes the name in upper case"
        );
    }

    // covers: OPCUA-ADDR-DATATYPE
    #[test]
    fn opcua_word_suffix_is_invalid_address() {
        for text in [
            "ns=2;s=x;WORD",
            "ns=2;s=x;DWORD",
            "ns=2;s=x;LWORD",
            "ns=2;s=x;WSTRING",
            "ns=2;s=x;word",
            "ns=2;s=x;NOT_A_TYPE",
            "ns=2;s=x;DINT1",
            "ns=2;s=x;DI NT",
            "ns=2;s=x;1",
            "ns=2;s=x;",
            "ns=2;s=x;;DINT",
            "ns=2;s=x;a",
            "ns=2;s=x;a=",
        ] {
            let message = refused(text);
            assert!(
                message.contains("data type") || message.contains("attribute"),
                "{text:?}: the message names what is wrong: {message}"
            );
        }
        let message = refused("ns=2;s=x;WORD");
        assert!(
            message.contains("WORD"),
            "the message names the type: {message}"
        );
        assert_eq!(OpcuaDataType::from_name("WORD"), None, "from_name agrees");
    }

    // covers: OPCUA-ADDR-DATATYPE
    #[test]
    fn opcua_missing_suffix_means_the_server_declared_type() {
        let address = parse("ns=2;i=1");
        assert_eq!(address.data_type(), OpcuaDataType::Null, "NULL");
        assert_eq!(address.value_type_hint(), None, "no hint");
        assert_eq!(
            parse("ns=2;i=1;NULL"),
            address,
            "an explicit NULL is the same address"
        );
        assert_eq!(
            parse("ns=2;i=1;NULL").canonical(),
            "ns=2;i=1",
            "and is written without the suffix"
        );
    }

    #[test]
    fn opcua_text_after_the_data_type_is_invalid_address() {
        for text in [
            "ns=2;i=1;DINT;INT",
            "ns=2;i=1;DINT;a=Value",
            "ns=2;i=1;a=Value;DINT;INT",
            "ns=2;i=1;a=Value;a=NodeId",
            "ns=2;i=1;DINT;",
        ] {
            refused(text);
        }
        let message = refused("ns=2;i=1;a=Value;DINT;extra");
        assert!(
            message.contains("extra"),
            "the message echoes the text: {message}"
        );
    }

    // covers: OPCUA-ADDR-CONFIG
    #[test]
    fn opcua_config_tail_parses_into_ordered_pairs() {
        let address = parse("ns=2;i=1;DINT|sampling-interval=500,queue-size=10");
        assert_eq!(
            address.config(),
            [
                ("sampling-interval".to_owned(), "500".to_owned()),
                ("queue-size".to_owned(), "10".to_owned()),
            ],
            "written order"
        );
        assert_eq!(
            parse("ns=2;i=1|b=2,a=1").config(),
            [
                ("b".to_owned(), "2".to_owned()),
                ("a".to_owned(), "1".to_owned())
            ],
            "no type is needed before the tail (upstream needs one)"
        );
        assert_eq!(
            parse("ns=2;i=1;a=NodeId|k=v").config().len(),
            1,
            "nor does an attribute swallow it"
        );
        assert_eq!(
            parse("ns=2;i=1;a=NodeId;REAL|k=v_w-9").config(),
            [("k".to_owned(), "v_w-9".to_owned())],
            "value characters"
        );
        assert!(parse("ns=2;i=1").config().is_empty(), "no tail, no pairs");
        let repeated = parse("ns=2;i=1|a=1,b=2,a=3");
        assert_eq!(
            repeated.config(),
            [
                ("a".to_owned(), "3".to_owned()),
                ("b".to_owned(), "2".to_owned())
            ],
            "a repeated key keeps its first place and takes the last value"
        );
        assert_eq!(
            repeated.canonical(),
            "ns=2;i=1|a=3,b=2",
            "and is written once"
        );
    }

    // covers: OPCUA-ADDR-CONFIG
    #[test]
    fn opcua_config_tail_with_a_bad_character_is_invalid_address() {
        for text in [
            "ns=2;i=1|",
            "ns=2;i=1|a",
            "ns=2;i=1|=1",
            "ns=2;i=1|a=",
            "ns=2;i=1|a=b c",
            "ns=2;i=1|a b=1",
            "ns=2;i=1|a=1,",
            "ns=2;i=1|,a=1",
            "ns=2;i=1|a=1,,b=2",
            "ns=2;i=1|a=1;b=2",
            "ns=2;i=1|1a=1",
            "ns=2;i=1|a=1=2",
            "ns=2;i=1|a=\u{e9}",
            "ns=2;i=1|a.b=1",
            "ns=2;i=1|a=1|b=2",
            "ns=2;i=1|a=b/c",
        ] {
            let message = refused(text);
            assert!(
                message.contains("key=value"),
                "{text:?}: the message names the grammar: {message}"
            );
        }
    }

    // covers: OPCUA-ADDR-CONFIG
    #[test]
    fn opcua_event_field_names_come_from_the_config_tail() {
        let address = parse("ns=0;i=2253|EventId=1,Message=1,Severity=1");
        assert_eq!(
            address.event_field_names().collect::<Vec<_>>(),
            ["EventId", "Message", "Severity"],
            "the keys in written order"
        );
        assert_eq!(
            parse("ns=0;i=2253").event_field_names().count(),
            0,
            "no tail, no fields"
        );
    }

    // covers: OPCUA-ADDR-QUERY
    #[test]
    fn opcua_browse_query_parses_a_node_address() {
        let query = OpcuaAddress::parse_query("ns=2;s=HelloWorld").expect("a node address");
        assert_eq!(
            query,
            parse("ns=2;s=HelloWorld"),
            "the same as parsing it as a tag"
        );
        assert_eq!(
            query.identifier(),
            &OpcuaIdentifier::String("HelloWorld".to_owned()),
            "the start node"
        );
        for text in ["not-an-address", "ns=65536;i=1", " ns=2;s=x", "***", "* *"] {
            let error = OpcuaAddress::parse_query(text).expect_err(text);
            assert_eq!(error.kind(), ErrorKind::InvalidAddress, "{text:?}: kind");
        }
    }

    // covers: OPCUA-ADDR-QUERY
    #[test]
    fn opcua_empty_and_double_star_queries_mean_the_objects_folder() {
        let objects = parse("ns=0;i=85");
        for text in ["", "**", "  ", "\t\n", " ** "] {
            assert_eq!(
                OpcuaAddress::parse_query(text).expect(text),
                objects,
                "{text:?}: the Objects folder"
            );
        }
        assert_eq!(objects.namespace(), 0, "namespace 0");
        assert_eq!(
            objects.identifier(),
            &OpcuaIdentifier::Numeric(85),
            "node 85"
        );
    }

    #[test]
    fn opcua_address_text_is_bounded_and_free_of_control_characters() {
        assert_eq!(OpcuaAddress::MAX_TEXT_BYTES, 8192, "the bound");
        refused("");
        let prefix = "ns=1;s=";
        let fits = format!(
            "{prefix}{}",
            "a".repeat(OpcuaAddress::MAX_TEXT_BYTES - prefix.len())
        );
        assert_eq!(
            fits.len(),
            OpcuaAddress::MAX_TEXT_BYTES,
            "exactly at the bound"
        );
        assert_eq!(parse(&fits).canonical(), fits, "accepted");
        let message = refused(&format!("{fits}a"));
        assert!(
            message.contains("8193"),
            "the message gives the size: {message}"
        );
        for text in [
            "ns=1;s=a\nb",
            "ns=1;s=a\0b",
            "ns=1;s=a\tb",
            "ns=1;s=a\u{7f}b",
            "ns=1;s=a\u{85}b",
            "ns=1;i=1;\n",
            "\nns=1;i=1",
        ] {
            let message = refused(text);
            assert!(
                message.contains("control"),
                "{text:?}: the message names the reason: {message}"
            );
        }
    }

    #[test]
    fn opcua_messages_echo_a_bounded_excerpt_of_the_input() {
        let long = "z".repeat(500);
        for text in [
            format!("{long};i=1"),
            format!("ns=1;{long}=1"),
            format!("ns={long};i=1"),
        ] {
            let message = refused(&text);
            assert!(
                message.len() < 300,
                "the message is bounded ({} bytes): {message}",
                message.len()
            );
        }
        let message = refused("ns=1;i=1;ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEFGHIJKLMNOPQRSTUVWXYZ");
        assert!(
            message.contains("ABCDEFGHIJKLMNOPQRSTUVWXYZABCDEF'"),
            "32 characters of the offending piece: {message}"
        );
    }

    // covers: OPCUA-ADDR-ROUND-TRIP
    #[test]
    fn opcua_the_canonical_text_obeys_the_bound_so_it_always_parses_back() {
        let bound = OpcuaAddress::MAX_TEXT_BYTES;
        // Each `\x` is three bytes in canonical form (`\\x`), so the canonical text is longer
        // than the input.
        let build = |canonical_len: usize| {
            let prefix = "ns=1;s=";
            let doubled = 100;
            let filler = canonical_len - prefix.len() - 3 * doubled;
            format!("{prefix}{}{}", r"\x".repeat(doubled), "a".repeat(filler))
        };
        let fits = parse(&build(bound));
        assert_eq!(fits.canonical().len(), bound, "canonical text at the bound");
        assert_eq!(parse(&fits.canonical()), fits, "and it parses back");
        let over = build(bound + 1);
        assert!(over.len() < bound, "the input is within the bound");
        let message = refused(&over);
        assert!(
            message.contains("8193") && message.contains("canonical"),
            "the message gives the canonical size: {message}"
        );
        // Larger blow-ups: an escaped backslash, a bracket group that is no array, and a
        // numeric attribute spelled out by name.
        let slashes = format!("ns=1;s={}", r"\x".repeat(4000));
        let brackets = format!("ns=1;s={}", "[]".repeat(4000));
        let attribute = format!("ns=1;s={};a=19", "x".repeat(bound - 12));
        for text in [slashes, brackets, attribute] {
            assert!(text.len() <= bound, "{} bytes in", text.len());
            let message = refused(&text);
            assert!(message.contains("canonical"), "{message}");
        }
    }

    // covers: OPCUA-ADDR-ROUND-TRIP
    #[test]
    fn opcua_canonical_text_writes_every_part() {
        let text = "ns=2;s=samples[3..8;1][4];a=UserAccessLevel;REAL|a=1,b-c=d_e";
        let address = parse(text);
        assert_eq!(address.canonical(), text, "canonical");
        assert_eq!(address.to_string(), text, "Display");
        assert_eq!(parse(&address.canonical()), address, "parses back equal");
        assert_eq!(
            parse("ns=002;i=0007[01..02;01];a=013;real|k=v").canonical(),
            "ns=2;i=7[1..2;1];REAL|k=v",
            "numbers, defaults and case are normalized"
        );
    }

    // covers: OPCUA-DIV-ADDRESS-ESCAPES, OPCUA-ADDR-ROUND-TRIP
    #[test]
    fn opcua_special_characters_in_a_string_identifier_survive_a_round_trip() {
        for identifier in [
            "a;b",
            "a|b",
            "a[b",
            "a]b",
            "a[1]",
            "a[1;2]",
            "[",
            "[]",
            "]",
            r"\",
            r"\\",
            r"a\;b",
            r"a\",
            r"[\]",
            "a=b",
            "ns=1;i=2",
            "a;a=Value",
            "a|k=v",
            "100%",
            "\u{e9}\u{4e16}\u{1f980}",
        ] {
            let address = string_address(identifier);
            let text = address.canonical();
            assert_eq!(
                parse(&text),
                address,
                "{identifier:?}: {text} parses back equal"
            );
            assert_eq!(
                parse(&text).identifier(),
                &OpcuaIdentifier::String(identifier.to_owned()),
                "{identifier:?}: {text} keeps the text"
            );
        }
    }

    /// An address with the given string identifier, built by escaping it by hand.
    fn string_address(identifier: &str) -> OpcuaAddress {
        let mut text = String::from("ns=1;s=");
        for c in identifier.chars() {
            if matches!(c, '\\' | ';' | '[' | '|') {
                text.push('\\');
            }
            text.push(c);
        }
        parse(&text)
    }

    #[test]
    fn opcua_address_is_a_protocol_address_for_a_tag() {
        let address = parse("ns=2;s=boiler;INT");
        assert_eq!(address.protocol(), "opcua", "protocol");
        assert_eq!(address.canonical(), "ns=2;s=boiler;INT", "canonical");
        assert_eq!(address.value_type_hint(), Some(PlcValueType::I16), "hint");
        assert!(!address.spans_multiple_words(), "single word by default");

        let shared: Arc<dyn ProtocolAddress> = Arc::new(address.clone());
        let tag = Tag::new(TagId::new("boiler").expect("a tag id"), Arc::clone(&shared));
        assert_eq!(
            tag.value_type(),
            Some(PlcValueType::I16),
            "the tag takes the hint"
        );
        assert!(
            tag.clone().with_type(PlcValueType::F32).is_err(),
            "a conflicting type is refused by the tag"
        );
        let any: &dyn Any = tag.address();
        assert_eq!(
            any.downcast_ref::<OpcuaAddress>(),
            Some(&address),
            "a driver can downcast its own address"
        );
        assert_eq!(
            tag.address().canonical(),
            "ns=2;s=boiler;INT",
            "through the tag"
        );

        let untyped = Tag::new(
            TagId::new("raw").expect("a tag id"),
            Arc::new(parse("ns=2;s=raw")),
        );
        assert_eq!(untyped.value_type(), None, "no hint, no type");
        assert!(
            untyped.with_type(PlcValueType::F32).is_ok(),
            "any type may be requested without a hint"
        );
    }

    #[test]
    fn opcua_addresses_are_values() {
        let address = parse("ns=2;s=boiler[1..2];DINT|a=1");
        assert_eq!(address.clone(), address, "clone");
        assert!(
            format!("{address:?}").contains("boiler"),
            "Debug shows the identifier"
        );
        let mut set = HashSet::new();
        set.insert(address.clone());
        set.insert(parse("ns=2;s=boiler[1..2];DINT|a=1"));
        set.insert(parse("ns=2;s=boiler[1..2];DINT|a=2"));
        assert_eq!(set.len(), 2, "equal addresses hash alike");
        assert_ne!(parse("ns=2;s=a"), parse("ns=3;s=a"), "namespace");
        assert_ne!(parse("ns=2;s=a"), parse("ns=2;s=b"), "identifier");
        assert_ne!(parse("ns=2;i=1"), parse("ns=2;s=1"), "identifier type");
        assert_ne!(parse("ns=2;s=a"), parse("ns=2;s=a;a=NodeId"), "attribute");
        assert_ne!(parse("ns=2;s=a"), parse("ns=2;s=a;INT"), "data type");
        assert_ne!(parse("ns=2;s=a"), parse("ns=2;s=a[1]"), "array selection");
        assert_ne!(parse("ns=2;s=a"), parse("ns=2;s=a|k=v"), "tail");
        let identifier = OpcuaIdentifier::ByteString(vec![1]);
        assert_eq!(identifier.clone(), identifier, "identifier clone");
        assert!(
            format!("{identifier:?}").contains("ByteString"),
            "identifier Debug"
        );
        let attribute = OpcuaAttribute::Value;
        assert!(
            format!("{attribute:?}").contains("Value"),
            "attribute Debug"
        );
        let data_type = OpcuaDataType::Real;
        assert!(format!("{data_type:?}").contains("Real"), "data type Debug");
    }
}
