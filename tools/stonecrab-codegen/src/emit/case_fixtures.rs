//! Tests on first-party synthetic mspec fixtures (slice S79): literal arguments, a case named
//! as a type (`RequestHeader('391')` of the OPC UA model) and case parameters. The text is
//! written here from nothing but the grammar, as the S77 and S78 fixtures are, and is
//! compiled with `rustc` against the same stand-in for `stonecrab::codec`.

use super::fixtures::{compile_and_run, options, run};
use super::{Endianness, Options};

/// A discriminated type whose cases refer to each other by name with literal arguments, as
/// `ReadRequest` of OPC UA refers to `RequestHeader('391')`: `Header` is a case of `Body` that
/// the options do not enable, and `Block` is a type that takes a literal. The parent's second
/// parameter is passed on by name and as a literal, and `Tail` declares it as a case
/// parameter.
const BODY: &str = "
[enum uint 8 Mode ['0' OFF] ['1' ON]]
[type Block(uint 8 width)
    [array byte data count 'width']
]
[discriminatedType Body(uint 32 kind, uint 8 scale)
    [typeSwitch kind
        ['391' Header
            [simple uint 32 handle]
            [simple Mode mode]
            [array byte pad count 'scale']
        ]
        ['422' ReadRequest
            [simple Header('391', 'scale') header]
            [simple Block('2') block]
            [implicit uint 16 noOf 'COUNT(ids)']
            [array uint 16 ids count 'noOf']
        ]
        ['423' ReadResponse
            [array Header('391', '1') headers count '2']
        ]
        ['500' Tail (uint 8 scale)
            [array byte data count 'scale']
        ]
    ]
]
";

fn body() -> Options {
    options(
        &["Block", "Body", "Mode"],
        &[("Body", &["ReadRequest", "ReadResponse", "Tail"])],
        Some(Endianness::Little),
    )
}

#[test]
fn generator_case_parameters_receive_literal_arguments() {
    let modules = run(BODY, &body()).expect("generates");
    assert_eq!(
        modules.keys().map(String::as_str).collect::<Vec<_>>(),
        ["block", "body", "header", "mode"],
        "the case named as a type is a module of its own, though the options never list it"
    );
    let header = modules.get("header").expect("the header module");
    assert!(header.contains("pub(crate) struct Header {"), "{header}");
    assert!(
        header.contains("_kind: u32") && header.contains("scale: u8"),
        "the parent's parameters are the case's, and the one it does not read is underscored: {header}"
    );
    let body = modules.get("body").expect("the body module");
    assert!(
        body.contains("Header::parse(reader, 391, scale)?")
            && body.contains("Header::parse(reader, 391, 1)?")
            && body.contains("Block::parse(reader, 2)?"),
        "literals and names are passed as written: {body}"
    );
    assert!(
        !body.contains("Header {"),
        "Header is not a variant, as the options leave it disabled: {body}"
    );
    assert!(!body.contains(" as "), "no `as` cast");

    let main = r#"
use body::Body;
use codec::{BoundedReader, BoundedWriter, ParseError};
use header::Header;
use mode::Mode;

fn bytes_of(body: &Body) -> Vec<u8> {
    let mut writer = BoundedWriter::new();
    body.serialize(&mut writer).unwrap();
    writer.into_bytes()
}

fn main() {
    // ReadRequest: the header of scale 1, a block of 2 bytes and two ids.
    let request: Vec<u8> = vec![4, 3, 2, 1, 1, 0xAA, 0xB1, 0xB2, 2, 0, 2, 1, 4, 3];
    let mut reader = BoundedReader::new(&request);
    let parsed = Body::parse(&mut reader, 422, 1).unwrap();
    assert_eq!(reader.remaining(), 0);
    assert_eq!(
        parsed,
        Body::ReadRequest {
            header: Header { handle: 0x0102_0304, mode: Mode::On, pad: vec![0xAA] },
            block: block::Block { data: vec![0xB1, 0xB2] },
            ids: vec![0x0102, 0x0304],
        }
    );
    assert_eq!(bytes_of(&parsed), request);

    // ReadResponse: two headers whose scale is the literal 1, whatever the parent's is.
    let response: Vec<u8> = vec![4, 3, 2, 1, 1, 0xAA, 8, 7, 6, 5, 0, 0xBB];
    let parsed = Body::parse(&mut BoundedReader::new(&response), 423, 9).unwrap();
    let Body::ReadResponse { headers } = &parsed else { panic!("not a response") };
    assert_eq!(headers.len(), 2);
    assert_eq!(headers[1], Header { handle: 0x0506_0708, mode: Mode::Off, pad: vec![0xBB] });
    assert_eq!(bytes_of(&parsed), response);

    // Tail: the case parameter is the parent's scale.
    let tail: Vec<u8> = vec![7, 8, 9];
    let parsed = Body::parse(&mut BoundedReader::new(&tail), 500, 3).unwrap();
    assert_eq!(parsed, Body::Tail { data: vec![7, 8, 9] });
    assert_eq!(bytes_of(&parsed), tail);
    assert!(matches!(
        Body::parse(&mut BoundedReader::new(&tail), 500, 4),
        Err(ParseError::Truncated { .. })
    ));

    // The case alone: the parent's parameters, then nothing else.
    let alone = Header::parse(&mut BoundedReader::new(&request), 391, 1).unwrap();
    assert_eq!(alone.handle, 0x0102_0304);

    // The case with a header of its own stays disabled as a variant.
    assert_eq!(
        Body::parse(&mut BoundedReader::new(&request), 391, 1),
        Err(ParseError::InvalidDiscriminator { field: "Body.Header (disabled)", value: 391 })
    );
    assert_eq!(
        Body::parse(&mut BoundedReader::new(&request), 999, 1),
        Err(ParseError::InvalidDiscriminator { field: "Body", value: 999 })
    );
}
"#;
    compile_and_run("case-literals", &modules, main).expect("the literal-argument round trip");
}

#[test]
fn generator_case_named_as_a_type_must_pass_its_own_discriminator() {
    for (replace, expected) in [
        (
            "Header('392', 'scale') header",
            "is the case '391' of `Body`",
        ),
        // A name is not the literal the case is.
        (
            "Header('kind', 'scale') header",
            "is the case '391' of `Body`",
        ),
        ("Header('391') header", "`Header` takes 2 arguments"),
        (
            "Block('300') block",
            "is not a literal of the parameter's type",
        ),
        (
            "Block('true') block",
            "is not a literal of the parameter's type",
        ),
        ("Block('missing') block", "neither a literal nor a name"),
    ] {
        let original = if replace.starts_with("Block") {
            "Block('2') block"
        } else {
            "Header('391', 'scale') header"
        };
        let text = BODY.replacen(original, replace, 1);
        let error = run(&text, &body()).expect_err(replace);
        assert!(error.contains(expected), "{replace}: {error}");
        assert!(error.contains("type Body"), "{replace}: {error}");
    }
    // The error names the line of the field, not of the type.
    let text = BODY.replacen(
        "Header('391', 'scale') header",
        "Header('1', 'scale') header",
        1,
    );
    let error = run(&text, &body()).expect_err("wrong literal");
    assert!(error.contains("line 14"), "{error}");
}

#[test]
fn generator_case_parameters_and_named_cases_are_checked_against_what_they_can_hold() {
    for (text, expected) in [
        // A case parameter must name a value of its type that the parent has.
        (
            BODY.replacen("Tail (uint 8 scale)", "Tail (uint 8 other)", 1),
            "the case parameter `other` of `Tail` names no value",
        ),
        (
            BODY.replacen("Tail (uint 8 scale)", "Tail (uint 16 scale)", 1),
            "the case parameter `scale` of `Tail` names no value",
        ),
        // A case with parameters of its own is not a type of its own.
        (
            BODY.replacen(
                "['500' Tail (uint 8 scale)",
                "['600' Param (uint 8 scale) [simple uint 8 x]]\n        ['500' Tail (uint 8 scale)",
                1,
            )
            .replacen("Block('2') block", "Param('600', 'scale') block", 1),
            "`Param` has case parameters and is named as a type",
        ),
        // Nor is a case whose parent reads fields before the switch.
        (
            "[discriminatedType P [discriminator uint 8 d] [typeSwitch d ['1' Leaf [simple uint 8 x]]]]
             [type T [simple Leaf('1') leaf]]"
                .to_owned(),
            "`Leaf` is a case of `P`, which reads fields before its typeSwitch",
        ),
        // Nor is a name that two types give a case.
        (
            "[discriminatedType P(uint 8 d) [typeSwitch d ['1' Same [simple uint 8 x]]]]
             [discriminatedType Q(uint 8 d) [typeSwitch d ['1' Same [simple uint 8 y]]]]
             [type T [simple Same('1') leaf]]"
                .to_owned(),
            "`Same` is a case of more than one type",
        ),
    ] {
        let types: &[&str] = if text.contains("[type T") {
            &["T"]
        } else {
            &["Block", "Body", "Mode"]
        };
        let cases: &[(&str, &[&str])] = if text.contains("[type T") {
            &[]
        } else {
            &[("Body", &["ReadRequest", "ReadResponse", "Tail"])]
        };
        let options = options(types, cases, Some(Endianness::Little));
        let error = run(&text, &options).expect_err(expected);
        assert!(error.contains(expected), "{error}");
    }
}

#[test]
fn generator_a_case_named_as_a_type_cannot_reach_itself() {
    let text = "
[discriminatedType P(uint 8 d)
    [typeSwitch d
        ['1' Loop [simple Loop('1') inner]]
    ]
]
[type T [simple Loop('1') leaf]]
";
    let error = run(text, &options(&["T"], &[], None)).expect_err("recursion");
    assert!(error.contains("recursive"), "{error}");
}

/// A literal argument of six digits or more is emitted with `_` separators (the lint table's
/// `clippy::unreadable_literal`), and a negative one keeps its sign.
#[test]
fn generator_large_literal_arguments_are_written_with_digit_separators() {
    let modules = run(
        "[type Leaf(uint 32 big, int 32 small) [simple uint 8 x]]
         [type Root [simple Leaf('100000', '-1234567') leaf]]",
        &options(&["Leaf", "Root"], &[], Some(Endianness::Little)),
    )
    .expect("generates");
    let root = modules.get("root").expect("the root module");
    assert!(
        root.contains("Leaf::parse(reader, 100_000, -1_234_567)?"),
        "the arguments are grouped: {root}"
    );
}
