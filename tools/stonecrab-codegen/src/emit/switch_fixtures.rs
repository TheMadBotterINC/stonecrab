//! Tests on first-party synthetic mspec fixtures (slice S80a): the three changes to the
//! generator that the dry run on the OPC UA model found (ADR-0019, S80 addendum), a signed
//! discriminator of every width, arms that bind equal names of different types, and a function
//! per case for a large switch. The text is written here from nothing but the grammar, as the
//! S77 to S79 fixtures are, and is compiled with `rustc` against the same stand-in for
//! `stonecrab::codec`.

use std::collections::BTreeMap;

use super::fixtures::{compile_and_run, options, run};
use super::{Endianness, Options};

/// Four switches over a signed number, one of each width. Two switch on a discriminator field
/// and two on a parameter, as `ExtensionObjectDefinition(int 32 extensionId)` does. Each has
/// a positive and a negative case, one more case that the options leave disabled (a negative
/// one), and a case that stores nothing.
fn signed() -> String {
    [
        ("SignedByte", 8, "127", "-128", "0", "-9", true),
        ("SignedShort", 16, "310", "-310", "0", "-9", false),
        ("SignedInt", 32, "310", "-310", "0", "-9", true),
        ("SignedLong", 64, "310", "-310", "0", "-9", false),
    ]
    .iter()
    .map(|(name, bits, plus, minus, off, gone, field)| {
        let (head, tail) = if *field {
            (format!("[discriminator int {bits} id]"), String::new())
        } else {
            (String::new(), format!("(int {bits} id)"))
        };
        format!(
            "
[discriminatedType {name}{tail}
    {head}
    [typeSwitch id
        ['{plus}' Plus [simple uint 8 value]]
        ['{minus}' Minus [simple uint 8 value]]
        ['{off}' Off [simple uint 8 value]]
        ['{gone}' Gone [simple uint 8 value]]
        ['5' Empty]
    ]
]"
        )
    })
    .collect::<Vec<String>>()
    .concat()
}

const SIGNED_TYPES: [&str; 4] = ["SignedByte", "SignedInt", "SignedLong", "SignedShort"];

fn signed_options() -> Options {
    let cases: Vec<(&str, &[&str])> = SIGNED_TYPES
        .iter()
        .map(|name| (*name, &["Empty", "Minus", "Plus"][..]))
        .collect();
    options(&SIGNED_TYPES, &cases, Some(Endianness::Little))
}

#[test]
fn generator_signed_int32_discriminator_selects_its_case() {
    let modules = run(&signed(), &signed_options()).expect("generates");
    let int = modules
        .get("signed_int")
        .expect("the module of the int 32 switch");
    assert!(
        int.contains("-310 =>") && int.contains("310 =>") && int.contains("-9 =>"),
        "the literals of the cases, negative ones included: {int}"
    );
    assert!(
        int.contains("i64::from(id).cast_unsigned()"),
        "the error value keeps the sign: {int}"
    );
    assert!(!int.contains(" as "), "no `as` cast");
    let long = modules
        .get("signed_long")
        .expect("the module of the int 64 switch");
    assert!(
        long.contains("id.cast_unsigned()"),
        "an int 64 needs no widening: {long}"
    );

    let main = r#"
use codec::{BoundedReader, BoundedWriter, ParseError};
use signed_byte::SignedByte;
use signed_int::SignedInt;
use signed_long::SignedLong;
use signed_short::SignedShort;

// The sign-extended two's complement of a negative number: -7 is not the case 7.
const fn negative(value: i64) -> u64 {
    value.cast_unsigned()
}

fn written(serialize: impl FnOnce(&mut BoundedWriter) -> Result<(), ParseError>) -> Vec<u8> {
    let mut writer = BoundedWriter::new();
    serialize(&mut writer).unwrap();
    writer.into_bytes()
}

fn refused<T: std::fmt::Debug>(result: Result<T, ParseError>, field: &'static str, value: u64) {
    assert_eq!(result.unwrap_err(), ParseError::InvalidDiscriminator { field, value });
}

fn main() {
    // int 8, a discriminator field: it is written back, and the sign is a bit pattern.
    for (bytes, case) in [
        (vec![127, 4], SignedByte::Plus { value: 4 }),
        (vec![0x80, 5], SignedByte::Minus { value: 5 }),
        (vec![5], SignedByte::Empty),
    ] {
        let parsed = SignedByte::parse(&mut BoundedReader::new(&bytes)).unwrap();
        assert_eq!(parsed, case);
        assert_eq!(written(|writer| parsed.serialize(writer)), bytes);
    }
    refused(SignedByte::parse(&mut BoundedReader::new(&[0xF9])), "SignedByte", negative(-7));
    refused(SignedByte::parse(&mut BoundedReader::new(&[7])), "SignedByte", 7);
    refused(SignedByte::parse(&mut BoundedReader::new(&[0xF7, 1])), "SignedByte.Gone (disabled)", negative(-9));
    refused(SignedByte::parse(&mut BoundedReader::new(&[0, 1])), "SignedByte.Off (disabled)", 0);

    // int 16, a parameter: the caller gives the number, and the case writes nothing for it.
    for (id, case) in [(310, SignedShort::Plus { value: 9 }), (-310, SignedShort::Minus { value: 9 })] {
        let parsed = SignedShort::parse(&mut BoundedReader::new(&[9]), id).unwrap();
        assert_eq!(parsed, case);
        assert_eq!(written(|writer| parsed.serialize(writer)), vec![9]);
    }
    refused(SignedShort::parse(&mut BoundedReader::new(&[9]), -7), "SignedShort", negative(-7));
    refused(
        SignedShort::parse(&mut BoundedReader::new(&[9]), i16::MIN),
        "SignedShort",
        negative(i64::from(i16::MIN)),
    );
    refused(SignedShort::parse(&mut BoundedReader::new(&[9]), -9), "SignedShort.Gone (disabled)", negative(-9));

    // int 32, a discriminator field, little-endian.
    for (bytes, case) in [
        (vec![0x36, 0x01, 0, 0, 4], SignedInt::Plus { value: 4 }),
        (vec![0xCA, 0xFE, 0xFF, 0xFF, 5], SignedInt::Minus { value: 5 }),
        (vec![5, 0, 0, 0], SignedInt::Empty),
    ] {
        let parsed = SignedInt::parse(&mut BoundedReader::new(&bytes)).unwrap();
        assert_eq!(parsed, case);
        assert_eq!(written(|writer| parsed.serialize(writer)), bytes);
    }
    refused(SignedInt::parse(&mut BoundedReader::new(&[0xF9, 0xFF, 0xFF, 0xFF])), "SignedInt", negative(-7));
    refused(
        SignedInt::parse(&mut BoundedReader::new(&[0xF7, 0xFF, 0xFF, 0xFF, 1])),
        "SignedInt.Gone (disabled)",
        negative(-9),
    );
    refused(
        SignedInt::parse(&mut BoundedReader::new(&[0, 0, 0, 0x80])),
        "SignedInt",
        negative(i64::from(i32::MIN)),
    );
    assert_eq!(
        SignedInt::parse(&mut BoundedReader::new(&[0x36, 0x01])),
        Err(ParseError::Truncated { needed: 4, available: 2 })
    );

    // int 64, a parameter.
    for (id, case) in [(310, SignedLong::Plus { value: 9 }), (-310, SignedLong::Minus { value: 9 })] {
        let parsed = SignedLong::parse(&mut BoundedReader::new(&[9]), id).unwrap();
        assert_eq!(parsed, case);
        assert_eq!(written(|writer| parsed.serialize(writer)), vec![9]);
    }
    refused(SignedLong::parse(&mut BoundedReader::new(&[9]), i64::MIN), "SignedLong", 1 << 63);
    refused(SignedLong::parse(&mut BoundedReader::new(&[9]), -9), "SignedLong.Gone (disabled)", negative(-9));
}
"#;
    compile_and_run("signed-discriminator", &modules, main).expect("the signed round trips");
}

#[test]
fn generator_signed_discriminator_literals_are_checked_against_the_width() {
    let with = |literal: &str| {
        run(
            &format!(
                "[discriminatedType Pick(int 8 id) [typeSwitch id ['{literal}' Only [simple uint 8 value]]]]"
            ),
            &options(&["Pick"], &[("Pick", &["Only"])], None),
        )
    };
    for fine in ["127", "-128", "0x7F", "0"] {
        with(fine).unwrap_or_else(|error| panic!("{fine}: {error}"));
    }
    for (literal, expected) in [
        ("128", "case value '128' does not fit its discriminator"),
        ("-129", "case value '-129' does not fit its discriminator"),
        ("0xFF", "case value '0xFF' does not fit its discriminator"),
        ("1.5", "case value '1.5' is not an integer"),
        ("true", "case value 'true' is not an integer"),
    ] {
        let error = with(literal).expect_err(literal);
        assert!(error.contains(expected), "{literal}: {error}");
    }
}

/// Switches whose cases bind a field of the same name, with bodies that are the same text:
/// `FromAlpha` and `FromBeta` hold `results` of different element types, and `FromAlphaToo`
/// holds the type of the first. `Spans` makes the element types ask for their length, and
/// `Holder` asks for the length of the reply, so its
/// `length_in_bytes` merges arms as well.
const REPLIES: &str = "
[type Alpha [simple uint 8 x]]
[type Beta [simple uint 16 y]]
[type Spans
    [simple Alpha alpha]
    [simple Beta beta]
    [implicit uint 8 span 'alpha.lengthInBytes + beta.lengthInBytes']
]
[discriminatedType Reply(uint 8 tag)
    [typeSwitch tag
        ['1' FromAlpha
            [implicit uint 8 noOf 'COUNT(results)']
            [array Alpha results count 'noOf']
        ]
        ['2' FromBeta
            [implicit uint 8 noOf 'COUNT(results)']
            [array Beta results count 'noOf']
        ]
        ['3' FromAlphaToo
            [implicit uint 8 noOf 'COUNT(results)']
            [array Alpha results count 'noOf']
        ]
    ]
]
[type Holder(uint 8 tag)
    [simple Reply('tag') reply]
    [implicit uint 8 span 'reply.lengthInBytes']
]
";

#[test]
fn generator_arms_with_equal_names_and_different_types_are_not_merged() {
    let options = options(
        &["Alpha", "Beta", "Holder", "Reply", "Spans"],
        &[("Reply", &["FromAlpha", "FromAlphaToo", "FromBeta"])],
        Some(Endianness::Little),
    );
    let modules = run(REPLIES, &options).expect("generates");
    let reply = modules.get("reply").expect("the reply module");
    assert_eq!(
        reply
            .matches("Self::FromAlpha { results } | Self::FromAlphaToo { results } =>")
            .count(),
        2,
        "the arms of one type still share a pattern, in serialize and in length_in_bytes: {reply}"
    );
    assert_eq!(
        reply.matches("Self::FromBeta { results } =>").count(),
        2,
        "the arm of the other type does not, in either: {reply}"
    );

    let main = r"
use alpha::Alpha;
use beta::Beta;
use codec::{BoundedReader, BoundedWriter};
use holder::Holder;
use reply::Reply;

fn main() {
    let from_alpha = vec![2, 7, 8];
    let reply = Reply::parse(&mut BoundedReader::new(&from_alpha), 1).unwrap();
    assert_eq!(reply, Reply::FromAlpha { results: vec![Alpha { x: 7 }, Alpha { x: 8 }] });
    let mut writer = BoundedWriter::new();
    reply.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), from_alpha);
    assert_eq!(reply.length_in_bytes(), Ok(3));

    let from_beta = vec![1, 7, 8];
    let reply = Reply::parse(&mut BoundedReader::new(&from_beta), 2).unwrap();
    assert_eq!(reply, Reply::FromBeta { results: vec![Beta { y: 0x0807 }] });
    let mut writer = BoundedWriter::new();
    reply.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), from_beta);
    assert_eq!(reply.length_in_bytes(), Ok(3));

    let too = Reply::parse(&mut BoundedReader::new(&from_alpha), 3).unwrap();
    assert_eq!(too, Reply::FromAlphaToo { results: vec![Alpha { x: 7 }, Alpha { x: 8 }] });
    assert_eq!(too.length_in_bytes(), Ok(3));

    let holder = Holder::parse(&mut BoundedReader::new(&[2, 7, 8, 3]), 1).unwrap();
    assert_eq!(holder.reply, reply_of_two());
    let mut writer = BoundedWriter::new();
    holder.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), vec![2, 7, 8, 3]);
}

fn reply_of_two() -> Reply {
    Reply::FromAlpha { results: vec![Alpha { x: 7 }, Alpha { x: 8 }] }
}
";
    compile_and_run("equal-names-arms", &modules, main).expect("the replies round trip");
}

/// A switch of `count` cases that take a parameter, store a discriminator, and nest through an
/// external type: `Case0` to the one before the last are the same case under another name,
/// with scalars, a byte array counted by the parameter and a `Nested` that re-enters the
/// switch; the last, `Leaf`, ends the nesting.
fn message(count: usize) -> String {
    let last = count.saturating_sub(1);
    let cases: String = (0..count)
        .map(|index| {
            if index == last {
                format!("        ['{index}' Leaf [simple uint 8 a]]\n")
            } else {
                format!(
                    "        ['{index}' Case{index}
            [simple uint 8 a]
            [simple uint 8 b]
            [simple uint 32 c]
            [array byte pad count 'scale']
            [simple Nested inner]
        ]\n"
                )
            }
        })
        .collect();
    format!(
        "
[type Nested [simple uint 8 unused]]
[discriminatedType Message(uint 8 scale)
    [discriminator uint 16 tag]
    [discriminator uint 8 mode]
    [typeSwitch tag, mode
{cases}    ]
]
"
    )
}

fn message_options(count: usize) -> Options {
    let mut cases: Vec<String> = (0..count.saturating_sub(1))
        .map(|index| format!("Case{index}"))
        .collect();
    cases.push("Leaf".to_owned());
    let cases: Vec<&str> = cases.iter().map(String::as_str).collect();
    let mut options = options(
        &["Message"],
        &[("Message", &cases)],
        Some(Endianness::Little),
    );
    options.depth = Some("crate::ext::Depth".to_owned());
    options.external = BTreeMap::from([(
        "Nested".to_owned(),
        super::External {
            path: "crate::ext::Nested".to_owned(),
            arguments: Vec::new(),
        },
    )]);
    options
}

#[test]
fn generator_large_switch_gets_a_function_per_case() {
    let at = |count: usize| {
        run(&message(count), &message_options(count))
            .expect("generates")
            .remove("message")
            .expect("the message module")
    };
    let small = at(17);
    assert!(
        small.contains("fn parse_case0(") && small.contains("fn serialize_case0("),
        "17 cases are over the limit of 16: {small}"
    );
    let limit = at(16);
    assert!(
        !limit.contains("fn parse_case0(") && !limit.contains("fn serialize_case0("),
        "16 cases stay in one match, as the output of a small switch was: {limit}"
    );

    let large = at(40);
    for expected in [
        // What a case takes from the parent: the budget, the parameter it reads, and the
        // discriminator it stores.
        "fn parse_case0(reader: &mut BoundedReader<'_>, depth: Depth, mode: u8, scale: u8) -> Result<Self, ParseError> {",
        "(0, _) => Self::parse_case0(reader, depth, mode, scale),",
        "fn serialize_case0(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {",
        "Self::Case0 { .. } => self.serialize_case0(writer, depth),",
        "let Self::Case0 { mode, a, b, c, pad, inner } = self else {",
        "fn parse_leaf(reader: &mut BoundedReader<'_>, mode: u8) -> Result<Self, ParseError> {",
        "fn serialize_leaf(&self, writer: &mut BoundedWriter) -> Result<(), ParseError> {",
        "field: \"Message.Case0\", value: 0",
    ] {
        assert!(large.contains(expected), "{expected}: {large}");
    }
    assert!(!large.contains(" as "), "no `as` cast");
}

/// The parse and serialize of a switch of 300 cases, nested through an external to its
/// budget of 40, on a thread of 1 MiB. With every case's locals in the frame of the
/// dispatcher, a debug build takes more than that for the nesting; with a function per case
/// the dispatcher's frame is what one case needs, whatever the number of cases.
#[test]
fn generator_dispatch_frame_does_not_grow_with_case_count() {
    let count = 300;
    let modules = run(&message(count), &message_options(count)).expect("generates");
    let message = modules.get("message").expect("the message module");
    assert_eq!(
        message.matches("fn parse_").count(),
        count,
        "one parse function per case"
    );
    assert_eq!(
        message.matches("fn serialize_").count(),
        count,
        "one serialize function per case"
    );

    let main = format!(
        r"
mod ext {{
    use crate::codec::{{BoundedReader, BoundedWriter, ParseError}};
    use crate::message::Message;

    pub const LIMIT: u32 = 40;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Depth(pub u32);

    impl Depth {{
        fn enter(self) -> Result<Self, ParseError> {{
            self.0.checked_sub(1).map(Self).ok_or(ParseError::RecursionLimit {{ limit: LIMIT }})
        }}
    }}

    /// A hand-written type that holds a message: it spends one unit of the budget on every
    /// entry, as the OPC UA `ExtensionObject` does.
    #[derive(Debug, Clone, PartialEq)]
    pub struct Nested(pub Option<Box<Message>>);

    impl Nested {{
        pub fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {{
            let depth = depth.enter()?;
            Ok(Self(Some(Box::new(Message::parse(reader, depth, 2)?))))
        }}

        pub fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {{
            let depth = depth.enter()?;
            match &self.0 {{
                Some(message) => message.serialize(writer, depth),
                None => Ok(()),
            }}
        }}
    }}
}}

use codec::{{BoundedReader, BoundedWriter, ParseError}};
use ext::{{Depth, LIMIT}};
use message::Message;

/// `hops` messages of the first case, each holding the next, then the last case.
fn nested(hops: usize) -> Vec<u8> {{
    let mut bytes = Vec::new();
    for _ in 0..hops {{
        bytes.extend([0, 0, 9, 1, 2, 3, 4, 5, 6, 7, 8]);
    }}
    bytes.extend({last}_u16.to_le_bytes());
    bytes.extend([9, 1]);
    bytes
}}

fn main() {{
    // 1 MiB: room for the budget when one hop costs what one case needs, not for 300.
    let thread = std::thread::Builder::new().stack_size(1 << 20).spawn(|| {{
        let budget = Depth(LIMIT);
        let fits = nested(usize::try_from(LIMIT).unwrap() - 1);
        let mut reader = BoundedReader::new(&fits);
        let parsed = Message::parse(&mut reader, budget, 2).unwrap();
        assert_eq!(reader.remaining(), 0);
        let mut writer = BoundedWriter::new();
        parsed.serialize(&mut writer, budget).unwrap();
        assert_eq!(writer.into_bytes(), fits);

        let too_deep = nested(usize::try_from(LIMIT).unwrap() + 1);
        assert_eq!(
            Message::parse(&mut BoundedReader::new(&too_deep), budget, 2),
            Err(ParseError::RecursionLimit {{ limit: LIMIT }})
        );
        // Two levels more than the 39 that fit, built by hand, fail on the way out.
        let wrap = |message| Message::Case0 {{
            mode: 9,
            a: 1,
            b: 2,
            c: 3,
            pad: vec![0, 0],
            inner: ext::Nested(Some(Box::new(message))),
        }};
        let deeper = wrap(wrap(parsed));
        assert_eq!(
            deeper.serialize(&mut BoundedWriter::new(), budget),
            Err(ParseError::RecursionLimit {{ limit: LIMIT }})
        );
    }});
    thread.unwrap().join().unwrap();
}}
",
        last = count.saturating_sub(1)
    );
    compile_and_run("dispatch-frame", &modules, &main).expect("the nesting fits the stack");
}
