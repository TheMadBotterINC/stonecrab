//! Tests on first-party synthetic mspec fixtures (slice S77): `type` structs, the
//! protocol byte order and duplicate type names. No OPC UA mspec is vendored yet, so the
//! text is written here from nothing but the grammar.
//!
//! The round trips compile the generated modules with `rustc` against a stand-in for
//! `stonecrab::codec` that has the signatures the output calls, and run the program. The
//! stand-in is small and is not the shipped codec; the shipped codec meets generated code
//! in the crate, where the output is linted and tested under the module's own lints.

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use super::{Endianness, Options, generate};
use crate::mspec::{parse, resolve, type_names};

/// A stand-in for the parts of the crate the generated modules import: `ByteOrder`, the
/// bounded reader and writer and `ParseError` of `crate::codec`, and the bit-run helpers of
/// the support module.
const STAND_IN: &str = r"
#![allow(dead_code, unused)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

pub mod codec {
    use crate::ByteOrder;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum ParseError {
        Truncated { needed: usize, available: usize },
        InvalidDiscriminator { field: &'static str, value: u64 },
        Overflow,
    }

    pub struct BoundedReader<'a> {
        data: &'a [u8],
        at: usize,
    }

    impl<'a> BoundedReader<'a> {
        pub fn new(data: &'a [u8]) -> Self {
            Self { data, at: 0 }
        }

        pub fn remaining(&self) -> usize {
            self.data.len() - self.at
        }

        pub fn read_bytes(&mut self, len: usize, _max: usize) -> Result<&'a [u8], ParseError> {
            if len > self.remaining() {
                return Err(ParseError::Truncated { needed: len, available: self.remaining() });
            }
            let bytes = &self.data[self.at..self.at + len];
            self.at += len;
            Ok(bytes)
        }

        pub fn read_u8(&mut self) -> Result<u8, ParseError> {
            Ok(self.read_bytes(1, usize::MAX)?[0])
        }

        pub fn read_u16(&mut self, order: ByteOrder) -> Result<u16, ParseError> {
            let bytes: [u8; 2] = self.read_bytes(2, usize::MAX)?.try_into().unwrap();
            Ok(match order {
                ByteOrder::BigEndian => u16::from_be_bytes(bytes),
                ByteOrder::LittleEndian => u16::from_le_bytes(bytes),
            })
        }

        pub fn read_u32(&mut self, order: ByteOrder) -> Result<u32, ParseError> {
            let bytes: [u8; 4] = self.read_bytes(4, usize::MAX)?.try_into().unwrap();
            Ok(match order {
                ByteOrder::BigEndian => u32::from_be_bytes(bytes),
                ByteOrder::LittleEndian => u32::from_le_bytes(bytes),
            })
        }
    }

    pub struct BoundedWriter {
        bytes: Vec<u8>,
    }

    impl BoundedWriter {
        pub fn new() -> Self {
            Self { bytes: Vec::new() }
        }

        pub fn into_bytes(self) -> Vec<u8> {
            self.bytes
        }

        pub fn write_bytes(&mut self, bytes: &[u8]) -> Result<(), ParseError> {
            self.bytes.extend_from_slice(bytes);
            Ok(())
        }

        pub fn write_u8(&mut self, value: u8) -> Result<(), ParseError> {
            self.write_bytes(&[value])
        }

        pub fn write_u16(&mut self, value: u16, order: ByteOrder) -> Result<(), ParseError> {
            self.write_bytes(&match order {
                ByteOrder::BigEndian => value.to_be_bytes(),
                ByteOrder::LittleEndian => value.to_le_bytes(),
            })
        }

        pub fn write_u32(&mut self, value: u32, order: ByteOrder) -> Result<(), ParseError> {
            self.write_bytes(&match order {
                ByteOrder::BigEndian => value.to_be_bytes(),
                ByteOrder::LittleEndian => value.to_le_bytes(),
            })
        }
    }
}

pub mod support {
    use crate::codec::ParseError;

    pub fn split_bits(run: u8, offset: u32, width: u32) -> Result<u8, ParseError> {
        let shift = 8u32.checked_sub(offset + width).ok_or(ParseError::Overflow)?;
        Ok((run >> shift) & (u8::MAX >> (8 - width)))
    }

    pub fn join_bits(run: u8, offset: u32, width: u32, value: u8) -> Result<u8, ParseError> {
        let shift = 8u32.checked_sub(offset + width).ok_or(ParseError::Overflow)?;
        let mask = u8::MAX >> (8 - width);
        if value & !mask != 0 {
            return Err(ParseError::Overflow);
        }
        Ok((run & !(mask << shift)) | (value << shift))
    }
}
";

/// Every construct a `type` struct takes: a sub-byte run, an enum, a nested struct, implicit
/// fields (one over `lengthInBytes`, one over `COUNT`), byte arrays counted by an implicit
/// field and by a parameter, and a `const`; a struct that passes its parameter on; and a
/// `discriminatedType` with a struct in a case.
const SHAPES: &str = "
[enum uint 16 Kind ['1' ALPHA] ['2' BETA]]
[type Pair
    [simple uint 16 first]
    [simple uint 32 second]
]
[type Packet(uint 8 extra)
    [simple bit urgent]
    [simple bit acknowledged]
    [simple uint 6 priority]
    [simple Kind kind]
    [simple Pair pair]
    [implicit uint 8 span 'pair.lengthInBytes + 2']
    [implicit uint 8 length 'COUNT(data)']
    [array byte data count 'length']
    [array byte tail count 'extra']
    [const uint 8 end 0xFF]
]
[type Wrapper(uint 8 extra)
    [simple Packet('extra') inner]
]
[discriminatedType Message
    [discriminator uint 8 tag]
    [typeSwitch tag
        ['1' Point [simple Pair pair]]
        ['2' Empty]
    ]
]
";

const SHAPES_TYPES: [&str; 5] = ["Kind", "Message", "Packet", "Pair", "Wrapper"];

fn options(types: &[&str], cases: &[(&str, &[&str])], byte_order: Option<Endianness>) -> Options {
    Options {
        support_module: "crate::support".to_owned(),
        byte_order,
        types: types.iter().map(|name| (*name).to_owned()).collect(),
        cases: cases
            .iter()
            .map(|(name, list)| {
                (
                    (*name).to_owned(),
                    list.iter()
                        .map(|case| (*case).to_owned())
                        .collect::<BTreeSet<_>>(),
                )
            })
            .collect(),
    }
}

fn shapes(byte_order: Option<Endianness>) -> Options {
    options(
        &SHAPES_TYPES,
        &[("Message", &["Point", "Empty"])],
        byte_order,
    )
}

/// Generates `text` with `options`, or returns the generation error as text.
fn run(text: &str, options: &Options) -> Result<BTreeMap<String, String>, String> {
    let spec = parse(text).map_err(|error| format!("parse: {error}"))?;
    resolve(&spec, &type_names(&[&spec])).map_err(|error| format!("resolve: {error}"))?;
    generate(&[spec], options).map_err(|error| error.to_string())
}

/// Compiles the generated `modules` (each a module of the crate root) with the stand-in
/// and `main`, runs the program and returns its standard error when either step fails.
fn compile_and_run(
    test: &str,
    modules: &BTreeMap<String, String>,
    main: &str,
) -> Result<(), String> {
    let generated: Vec<String> = modules
        .iter()
        .map(|(name, body)| format!("mod {name} {{\n{body}\n}}\n"))
        .collect();
    let source = [STAND_IN, &generated.concat(), main].concat();
    let directory =
        std::env::temp_dir().join(format!("stonecrab-codegen-{test}-{}", std::process::id()));
    let io = |what: &str| {
        let what = what.to_owned();
        move |error: std::io::Error| format!("{what}: {error}")
    };
    std::fs::create_dir_all(&directory).map_err(io("scratch directory"))?;
    let main_path = directory.join("main.rs");
    let program = directory.join("program");
    std::fs::write(&main_path, source).map_err(io("write the program"))?;
    let compiled = Command::new("rustc")
        .args(["--edition", "2024", "--cap-lints", "allow", "-o"])
        .arg(&program)
        .arg(&main_path)
        .output()
        .map_err(io("run rustc"))?;
    let result = if compiled.status.success() {
        let ran = Command::new(&program)
            .output()
            .map_err(io("run the program"))?;
        if ran.status.success() {
            Ok(())
        } else {
            Err(format!(
                "the program failed:\n{}",
                String::from_utf8(ran.stderr).unwrap_or_default()
            ))
        }
    } else {
        Err(format!(
            "rustc refused the generated code:\n{}",
            String::from_utf8(compiled.stderr).unwrap_or_default()
        ))
    };
    std::fs::remove_dir_all(&directory).map_err(io("clean up"))?;
    result
}

/// The program that parses a `Packet` and a `Message` from `packet` and `point`, checks
/// every field, serializes both back to the same bytes and checks the refusals.
fn shapes_main(packet: &[u8], point: &[u8], kind_3: &[u8]) -> String {
    format!(
        r#"
use codec::{{BoundedReader, BoundedWriter, ParseError}};

fn bytes_of(input: &[u8], serialize: impl FnOnce(&mut BoundedWriter) -> Result<(), ParseError>) -> Vec<u8> {{
    let mut writer = BoundedWriter::new();
    serialize(&mut writer).unwrap();
    writer.into_bytes()
}}

fn main() {{
    let packet: Vec<u8> = vec!{packet:?};
    let mut reader = BoundedReader::new(&packet);
    let parsed = packet::Packet::parse(&mut reader, 2).unwrap();
    assert_eq!(reader.remaining(), 0);
    assert!(parsed.urgent);
    assert!(!parsed.acknowledged);
    assert_eq!(parsed.priority, 5);
    assert_eq!(parsed.kind, kind::Kind::Beta);
    assert_eq!(parsed.pair.first, 0x1234);
    assert_eq!(parsed.pair.second, 0xA1B2_C3D4);
    assert_eq!(parsed.pair.length_in_bytes(), Ok(6));
    assert_eq!(parsed.data, vec![0xAA, 0xBB, 0xCC]);
    assert_eq!(parsed.tail, vec![0xDD, 0xEE]);
    assert_eq!(bytes_of(&packet, |writer| parsed.serialize(writer)), packet);

    let wrapped = wrapper::Wrapper::parse(&mut BoundedReader::new(&packet), 2).unwrap();
    assert_eq!(wrapped.inner, parsed);

    let point: Vec<u8> = vec!{point:?};
    let message = message::Message::parse(&mut BoundedReader::new(&point)).unwrap();
    assert_eq!(
        message,
        message::Message::Point {{ pair: pair::Pair {{ first: 0x1234, second: 0xA1B2_C3D4 }} }}
    );
    assert_eq!(bytes_of(&point, |writer| message.serialize(writer)), point);

    let mut wrong_span = packet.clone();
    wrong_span[9] = 9;
    assert_eq!(
        packet::Packet::parse(&mut BoundedReader::new(&wrong_span), 2),
        Err(ParseError::InvalidDiscriminator {{ field: "span", value: 9 }})
    );
    let mut wrong_end = packet.clone();
    *wrong_end.last_mut().unwrap() = 0;
    assert_eq!(
        packet::Packet::parse(&mut BoundedReader::new(&wrong_end), 2),
        Err(ParseError::InvalidDiscriminator {{ field: "end", value: 0 }})
    );
    let mut wrong_kind = packet.clone();
    wrong_kind[1..3].copy_from_slice(&{kind_3:?});
    assert_eq!(
        packet::Packet::parse(&mut BoundedReader::new(&wrong_kind), 2),
        Err(ParseError::InvalidDiscriminator {{ field: "Kind", value: 3 }})
    );
    assert!(matches!(
        packet::Packet::parse(&mut BoundedReader::new(&packet[..packet.len() - 1]), 2),
        Err(ParseError::Truncated {{ .. }})
    ));
}}
"#
    )
}

/// The bytes of a `Packet` of the fixture, 2 as its size: an urgent, not acknowledged,
/// priority 5 run, `BETA`, a pair of `0x1234` and `0xA1B2C3D4`, the span 8 (the pair's 6
/// bytes and 2), three data bytes, two tail bytes and the constant.
fn packet_bytes(order: Endianness) -> (Vec<u8>, Vec<u8>, [u8; 2]) {
    let (kind, first, second, kind_3) = match order {
        Endianness::Big => (
            [0x00, 0x02],
            [0x12, 0x34],
            [0xA1, 0xB2, 0xC3, 0xD4],
            [0x00, 0x03],
        ),
        Endianness::Little => (
            [0x02, 0x00],
            [0x34, 0x12],
            [0xD4, 0xC3, 0xB2, 0xA1],
            [0x03, 0x00],
        ),
    };
    let mut packet = vec![0x85];
    packet.extend(kind);
    packet.extend(first);
    packet.extend(second);
    packet.extend([0x08, 0x03, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
    let mut point = vec![0x01];
    point.extend(first);
    point.extend(second);
    (packet, point, kind_3)
}

#[test]
fn generator_emits_type_structs() {
    let modules = run(SHAPES, &shapes(None)).expect("generates");
    assert_eq!(
        modules.keys().map(String::as_str).collect::<Vec<_>>(),
        ["kind", "message", "packet", "pair", "wrapper"],
        "one module per type, structs included"
    );
    let packet = modules.get("packet").expect("the packet module");
    assert!(packet.contains("pub(crate) struct Packet {"), "{packet}");
    for field in [
        "urgent: bool",
        "priority: u8",
        "kind: Kind",
        "pair: Pair",
        "data: Vec<u8>",
        "tail: Vec<u8>",
    ] {
        assert!(packet.contains(&format!("pub(crate) {field},")), "{field}");
    }
    for implicit_or_const in ["span:", "length:", "end:"] {
        assert!(
            !packet.contains(&format!("pub(crate) {implicit_or_const}")),
            "{implicit_or_const} is not stored"
        );
    }
    assert!(
        packet.contains("derive(Debug, Clone, PartialEq, Eq)"),
        "no float, so Eq"
    );
    assert!(!packet.contains(" as "), "no `as` cast");
    assert!(
        modules
            .get("pair")
            .expect("the pair module")
            .contains("fn length_in_bytes(&self)"),
        "a struct whose lengthInBytes is read has it"
    );
    assert!(
        !packet.contains("fn length_in_bytes"),
        "and a struct whose is not does not"
    );

    // Compiled and run: the default byte order is big-endian.
    let (packet, point, kind_3) = packet_bytes(Endianness::Big);
    compile_and_run(
        "type-structs",
        &modules,
        &shapes_main(&packet, &point, &kind_3),
    )
    .expect("the struct round trip");
}

#[test]
fn generator_type_structs_carry_the_lint_expectations_clippy_needs() {
    let text = "
[type Floaty [simple float 32 left] [simple uint 8 count]]
[type Triple [simple uint 8 triple_a] [simple uint 8 triple_b] [simple uint 8 triple_c]]
[type Plain [simple uint 8 one] [simple uint 8 two] [simple uint 8 three]]
[type Fixed [simple uint 16 x]]
[type Blob [implicit uint 8 n 'COUNT(bytes)'] [array byte bytes count 'n']]
[type Holder [simple Fixed fixed] [simple Blob blob] [implicit uint 8 span 'fixed.lengthInBytes + blob.lengthInBytes']]
";
    let types = ["Blob", "Fixed", "Floaty", "Holder", "Plain", "Triple"];
    let modules = run(text, &options(&types, &[], None)).expect("generates");
    let module = |name: &str| modules.get(name).expect("module").as_str();
    assert!(
        module("floaty").contains("derive(Debug, Clone, PartialEq)]"),
        "a float field leaves out Eq"
    );
    assert!(module("plain").contains("derive(Debug, Clone, PartialEq, Eq)"));
    assert!(
        module("triple").contains("clippy::struct_field_names"),
        "every field starts with the struct's name"
    );
    assert!(
        !module("plain").contains("struct_field_names"),
        "and a struct that does not repeat itself has no expectation"
    );
    assert!(
        module("fixed").contains("pub(crate) const fn length_in_bytes(&self)")
            && module("fixed").contains("clippy::unused_self"),
        "a fixed length is a const fn that keeps the shape of every length"
    );
    let blob = module("blob");
    assert!(
        blob.contains("pub(crate) fn length_in_bytes(&self)")
            && !blob.contains("const fn")
            && !blob.contains("clippy::unused_self"),
        "a counted array makes the length a plain fn: {blob}"
    );
}

#[test]
fn generator_type_structs_refuse_what_they_cannot_hold() {
    for (text, expected) in [
        (
            "[type T [discriminator uint 8 d] [simple uint 8 x]]",
            "`discriminator` field in a `type`",
        ),
        ("[type T [const uint 8 magic 0x7F]]", "stores no field"),
        (
            "[type T [virtual uint 8 v 'x'] [simple uint 8 x]]",
            "`virtual`",
        ),
        ("[type T [simple uint 8 x] [simple T next]]", "recursive"),
        (
            "[type T [simple uint 8 value] [simple uint 8 type]]",
            "keyword",
        ),
        (
            "[type T byteOrder='\"MIDDLE_ENDIAN\"' [simple uint 8 x]]",
            "not a byte order",
        ),
        (
            "[type T [batchSet byteOrder='\"LITTLE_ENDIAN\"' [simple uint 16 x]]]",
            "batchSet attributes",
        ),
    ] {
        let error = run(text, &options(&["T"], &[], None)).expect_err(text);
        assert!(error.contains(expected), "{text}: {error}");
    }
}

#[test]
fn generator_little_endian_protocol_default_round_trips() {
    let little = shapes(Some(Endianness::Little));
    let modules = run(SHAPES, &little).expect("generates");
    for (name, body) in &modules {
        assert!(!body.contains("BigEndian"), "{name} reads big-endian");
    }
    for name in ["kind", "pair"] {
        let body = modules.get(name).expect("module");
        assert!(body.contains("ByteOrder::LittleEndian"), "{name}: {body}");
    }
    let (packet, point, kind_3) = packet_bytes(Endianness::Little);
    compile_and_run(
        "little-endian",
        &modules,
        &shapes_main(&packet, &point, &kind_3),
    )
    .expect("the little-endian round trip");

    // The same input under the default order is the big-endian program: the option is what
    // changes the bytes, nothing else.
    let big = run(SHAPES, &shapes(Some(Endianness::Big))).expect("generates");
    assert_eq!(
        big,
        run(SHAPES, &shapes(None)).expect("generates"),
        "BIG_ENDIAN is the default"
    );
    assert_ne!(big, modules, "the order reaches the output");

    // A byteOrder attribute in the mspec must agree with the protocol's order.
    let tagged = "[type T byteOrder='\"LITTLE_ENDIAN\"' [simple uint 16 x]]";
    run(tagged, &options(&["T"], &[], Some(Endianness::Little))).expect("agrees");
    let error = run(tagged, &options(&["T"], &[], None)).expect_err("disagrees");
    assert!(
        error.contains("differs from the protocol's byte order BIG_ENDIAN"),
        "{error}"
    );
}

#[test]
fn generator_rejects_duplicate_type_names() {
    let text = "
[enum uint 8 NodeIdType ['0' TWO_BYTE]]
[type Header [simple uint 8 x]]
[enum uint 8 NodeIdType ['1' FOUR_BYTE]]
[type Body [simple NodeIdType kind]]
";
    let error = run(text, &options(&["Body", "NodeIdType"], &[], None)).expect_err("duplicate");
    assert!(error.contains("type NodeIdType"), "{error}");
    assert!(error.contains("line 4"), "the second definition: {error}");
    assert!(error.contains("defined twice (lines 2 and 4)"), "{error}");

    // Reached through a field of an enabled type, not only from the options.
    let error = run(text, &options(&["Body"], &[], None)).expect_err("duplicate by reference");
    assert!(error.contains("defined twice"), "{error}");

    // A duplicate nothing generated refers to would change no output, so it is not an error.
    assert!(
        run(text, &options(&["Header"], &[], None)).is_ok(),
        "an unused duplicate"
    );
}
