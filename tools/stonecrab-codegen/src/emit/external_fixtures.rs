//! Tests on first-party synthetic mspec fixtures (slice S79): the `[external]` table, the hand
//! written types the output calls through one fixed signature with a recursion budget
//! (ADR-0034 decision 3). The text is written here from nothing but the grammar. The
//! externals of the program are stand-ins with the signature a hand-written OPC UA built-in
//! of S85 and S86 has over `BoundedReader` and `BoundedWriter`, compiled with `rustc` next to
//! the generated modules and the same stand-in for `stonecrab::codec`.

use std::collections::BTreeMap;

use super::fixtures::{compile_and_run, options, run};
use super::{Endianness, External, Options};

/// A row of the `[external]` table as the tests write it: the mspec name, the Rust path and
/// the pinned arguments.
type Entry = (&'static str, &'static str, &'static [&'static str]);

/// Three types the table replaces (`Name` and `Variant` take no parameter, `Envelope` is
/// pinned to `'true'`), a struct that holds them in every position a field can (alone, as an
/// array element, in an `implicit` expression through `lengthInBytes`), a struct that holds
/// that one with a literal for its parameter, a `discriminatedType` that holds a `Name` and a
/// struct that reads the length of that.
const RECORDS: &str = "
[type Name [simple uint 8 length]]
[type Variant [simple uint 8 tag]]
[type Envelope(bit masked) [simple uint 8 body]]
[type Record(uint 8 count)
    [simple uint 16 id]
    [simple Name name]
    [simple Variant value]
    [simple Envelope('true') envelope]
    [implicit uint 8 span 'name.lengthInBytes + value.lengthInBytes']
    [implicit int 32 noOfValues 'values == null ? -1 : COUNT(values)']
    [array Variant values count 'noOfValues']
    [array byte tail count 'count']
]
[type Wrapper
    [simple Record('1') record]
]
[discriminatedType Frame
    [discriminator uint 8 tag]
    [typeSwitch tag
        ['1' Plain [simple Name only]]
        ['2' Pair [simple Name first] [simple Name second]]
    ]
]
[type Holder
    [simple Frame frame]
    [implicit uint 8 span 'frame.lengthInBytes']
]
";

const ENVELOPE: Entry = ("Envelope", "crate::ext::Envelope", &["true"]);
const NAME: Entry = ("Name", "crate::ext::Name", &[]);
const VARIANT: Entry = ("Variant", "crate::ext::Variant", &[]);

/// The options of `RECORDS`: little-endian, a budget type and the three externals.
fn records() -> Options {
    let mut options = options(
        &["Frame", "Holder", "Record", "Wrapper"],
        &[("Frame", &["Pair", "Plain"])],
        Some(Endianness::Little),
    );
    options.depth = Some("crate::ext::Depth".to_owned());
    options.external = externals(&[ENVELOPE, NAME, VARIANT]);
    options
}

fn externals(entries: &[Entry]) -> BTreeMap<String, External> {
    entries
        .iter()
        .map(|(name, path, arguments)| {
            (
                (*name).to_owned(),
                External {
                    path: (*path).to_owned(),
                    arguments: arguments.iter().map(|text| (*text).to_owned()).collect(),
                },
            )
        })
        .collect()
}

/// The hand-written side of the program: a budget of five entries, a `Name` (a length byte
/// and the bytes), a `Variant` that holds another and spends the budget on every entry, and an
/// `Envelope`. Each has the one signature of the contract. `Name` and `Envelope` check that
/// the generated code hands them the budget it was given, unspent.
const EXTERNALS: &str = r#"
mod ext {
    use crate::codec::{BoundedReader, BoundedWriter, ParseError};

    pub const LIMIT: u32 = 5;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Depth(pub u32);

    impl Depth {
        pub fn enter(self) -> Result<Self, ParseError> {
            self.0.checked_sub(1).map(Self).ok_or(ParseError::RecursionLimit { limit: LIMIT })
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Name(pub Vec<u8>);

    impl Name {
        pub fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
            assert_eq!(depth, Depth(LIMIT), "the budget arrives unspent");
            let length = reader.read_u8()?;
            Ok(Self(reader.read_bytes(usize::from(length), usize::MAX)?.to_vec()))
        }

        pub fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
            assert_eq!(depth, Depth(LIMIT), "the budget arrives unspent");
            writer.write_u8(u8::try_from(self.0.len()).map_err(|_| ParseError::Overflow)?)?;
            writer.write_bytes(&self.0)
        }

        pub fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
            assert_eq!(depth, Depth(LIMIT), "the budget arrives unspent");
            self.0.len().checked_add(1).ok_or(ParseError::Overflow)
        }
    }

    #[derive(Debug, Clone, PartialEq)]
    pub enum Variant {
        Scalar(u8),
        Nested(Box<Variant>),
    }

    impl Variant {
        pub fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
            let depth = depth.enter()?;
            match reader.read_u8()? {
                0 => Ok(Self::Scalar(reader.read_u8()?)),
                1 => Ok(Self::Nested(Box::new(Self::parse(reader, depth)?))),
                other => Err(ParseError::InvalidDiscriminator {
                    field: "Variant",
                    value: u64::from(other),
                }),
            }
        }

        pub fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
            let depth = depth.enter()?;
            match self {
                Self::Scalar(value) => {
                    writer.write_u8(0)?;
                    writer.write_u8(*value)
                }
                Self::Nested(inner) => {
                    writer.write_u8(1)?;
                    inner.serialize(writer, depth)
                }
            }
        }

        pub fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
            let depth = depth.enter()?;
            match self {
                Self::Scalar(_) => Ok(2),
                Self::Nested(inner) => inner.length_in_bytes(depth)?.checked_add(1).ok_or(ParseError::Overflow),
            }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Envelope(pub u8);

    impl Envelope {
        pub fn parse(reader: &mut BoundedReader<'_>, depth: Depth) -> Result<Self, ParseError> {
            assert_eq!(depth, Depth(LIMIT), "the budget arrives unspent");
            Ok(Self(reader.read_u8()?))
        }

        pub fn serialize(&self, writer: &mut BoundedWriter, depth: Depth) -> Result<(), ParseError> {
            assert_eq!(depth, Depth(LIMIT), "the budget arrives unspent");
            writer.write_u8(self.0)
        }

        pub fn length_in_bytes(&self, _depth: Depth) -> Result<usize, ParseError> {
            Ok(1)
        }
    }
}
"#;

#[test]
fn generator_external_type_is_called_through_its_signature() {
    let modules = run(RECORDS, &records()).expect("generates");
    assert_eq!(
        modules.keys().map(String::as_str).collect::<Vec<_>>(),
        ["frame", "holder", "record", "wrapper"],
        "an external type has no module of its own"
    );
    let record = modules.get("record").expect("the record module");
    for expected in [
        "use crate::ext::Depth;",
        "use crate::ext::Envelope;",
        "use crate::ext::Name;",
        "use crate::ext::Variant;",
        "pub(crate) name: Name,",
        "pub(crate) values: Option<Vec<Variant>>,",
        "Name::parse(reader, depth)?",
        "Envelope::parse(reader, depth)?",
        "Variant::parse(reader, depth)?",
        "name.length_in_bytes(depth)?",
        "name.serialize(writer, depth)?",
        "array_item.serialize(writer, depth)?",
        "fn parse(reader: &mut BoundedReader<'_>, depth: Depth, count: u8)",
        "fn serialize(&self, writer: &mut BoundedWriter, depth: Depth)",
    ] {
        assert!(record.contains(expected), "{expected}: {record}");
    }
    assert!(
        !record.contains("PartialEq, Eq") && record.contains("derive(Debug, Clone, PartialEq)]"),
        "a Variant can hold a float, so nothing derives Eq beside externals: {record}"
    );
    assert!(
        !record.contains("super::name"),
        "no import of a module that is not there"
    );
    assert!(!record.contains(" as "), "no `as` cast");
    let holder = modules.get("holder").expect("the holder module");
    assert!(
        holder.contains("frame.length_in_bytes(depth)?")
            && holder.contains("fn parse(reader: &mut BoundedReader<'_>, depth: Depth)"),
        "the budget crosses generated types: {holder}"
    );
    let frame = modules.get("frame").expect("the frame module");
    assert!(
        frame.contains("fn length_in_bytes(&self, depth: Depth)"),
        "{frame}"
    );
    let wrapper = modules.get("wrapper").expect("the wrapper module");
    assert!(
        wrapper.contains("Record::parse(reader, depth, 1)?"),
        "a literal argument goes after the budget: {wrapper}"
    );

    let main = format!(
        r#"{EXTERNALS}
use codec::{{BoundedReader, BoundedWriter, ParseError}};
use ext::{{Depth, Envelope, LIMIT, Name, Variant}};
use frame::Frame;
use holder::Holder;
use record::Record;
use wrapper::Wrapper;

fn record_bytes(value: &[u8], values: &[u8]) -> Vec<u8> {{
    let mut bytes = vec![0x02, 0x01, 3, b'a', b'b', b'c'];
    bytes.extend(value);
    bytes.push(0x42);
    bytes.push(u8::try_from(4 + value.len()).unwrap());
    bytes.extend(values);
    bytes.extend([0xEE, 0xFF]);
    bytes
}}

fn parse(bytes: &[u8], depth: Depth) -> Result<Record, ParseError> {{
    Record::parse(&mut BoundedReader::new(bytes), depth, 2)
}}

fn main() {{
    let budget = Depth(LIMIT);

    // A record: a name, a Variant that holds a Variant, an envelope, two array elements.
    let values = [2, 0, 0, 0, 0, 5, 0, 6];
    let bytes = record_bytes(&[1, 0, 7], &values);
    let mut reader = BoundedReader::new(&bytes);
    let parsed = Record::parse(&mut reader, budget, 2).unwrap();
    assert_eq!(reader.remaining(), 0);
    assert_eq!(
        parsed,
        Record {{
            id: 0x0102,
            name: Name(b"abc".to_vec()),
            value: Variant::Nested(Box::new(Variant::Scalar(7))),
            envelope: Envelope(0x42),
            values: Some(vec![Variant::Scalar(5), Variant::Scalar(6)]),
            tail: vec![0xEE, 0xFF],
        }}
    );
    let mut writer = BoundedWriter::new();
    parsed.serialize(&mut writer, budget).unwrap();
    assert_eq!(writer.into_bytes(), bytes);

    // A null array stays null.
    let null = record_bytes(&[0, 9], &[0xFF, 0xFF, 0xFF, 0xFF]);
    let parsed = parse(&null, budget).unwrap();
    assert_eq!(parsed.values, None);
    let mut writer = BoundedWriter::new();
    parsed.serialize(&mut writer, budget).unwrap();
    assert_eq!(writer.into_bytes(), null);

    // The budget is the external's to spend: five Variants fit it, six do not, on the way
    // in and on the way out, and the failure is the external's own error.
    let five = record_bytes(&[1, 1, 1, 1, 0, 9], &[0, 0, 0, 0]);
    let parsed = parse(&five, budget).unwrap();
    let six = record_bytes(&[1, 1, 1, 1, 1, 0, 9], &[0, 0, 0, 0]);
    assert_eq!(parse(&six, budget), Err(ParseError::RecursionLimit {{ limit: LIMIT }}));
    let mut deeper = parsed.clone();
    deeper.value = Variant::Nested(Box::new(deeper.value));
    assert_eq!(
        deeper.serialize(&mut BoundedWriter::new(), budget),
        Err(ParseError::RecursionLimit {{ limit: LIMIT }})
    );

    // The external's refusals are the record's.
    assert_eq!(
        parse(&record_bytes(&[2, 0], &[0, 0, 0, 0]), budget),
        Err(ParseError::InvalidDiscriminator {{ field: "Variant", value: 2 }})
    );
    assert!(matches!(parse(&bytes[..4], budget), Err(ParseError::Truncated {{ .. }})));

    // A wrong implicit span is the record's refusal.
    let mut wrong = bytes.clone();
    wrong[10] = 9;
    assert_eq!(
        parse(&wrong, budget),
        Err(ParseError::InvalidDiscriminator {{ field: "span", value: 9 }})
    );

    // The budget crosses a discriminated type and a struct that asks for its length.
    let pair: Vec<u8> = vec![2, 1, 0xAA, 2, 0xBB, 0xCC, 6];
    let holder = Holder::parse(&mut BoundedReader::new(&pair), budget).unwrap();
    assert_eq!(
        holder.frame,
        Frame::Pair {{ first: Name(vec![0xAA]), second: Name(vec![0xBB, 0xCC]) }}
    );
    assert_eq!(holder.frame.length_in_bytes(budget), Ok(6));
    let mut writer = BoundedWriter::new();
    holder.serialize(&mut writer, budget).unwrap();
    assert_eq!(writer.into_bytes(), pair);
    let mut wrong = pair.clone();
    wrong[6] = 7;
    assert_eq!(
        Holder::parse(&mut BoundedReader::new(&wrong), budget),
        Err(ParseError::InvalidDiscriminator {{ field: "span", value: 7 }})
    );
    let plain: Vec<u8> = vec![1, 2, 0xAA, 0xBB];
    let frame = Frame::parse(&mut BoundedReader::new(&plain), budget).unwrap();
    assert_eq!(frame, Frame::Plain {{ only: Name(vec![0xAA, 0xBB]) }});
    assert_eq!(frame.length_in_bytes(budget), Ok(4));

    // A literal argument follows the budget: the wrapped record has a one byte tail.
    let mut wrapped = record_bytes(&[0, 3], &[0, 0, 0, 0]);
    wrapped.pop();
    let wrapper = Wrapper::parse(&mut BoundedReader::new(&wrapped), budget).unwrap();
    assert_eq!(wrapper.record.tail, vec![0xEE]);
}}
"#
    );
    compile_and_run("external-signature", &modules, &main).expect("the external round trip");
}

#[test]
fn generator_missing_external_type_is_an_error() {
    let with = |external: &[Entry], depth: Option<&str>| {
        let mut options = options(&["Record"], &[], Some(Endianness::Little));
        options.depth = depth.map(str::to_owned);
        options.external = externals(external);
        options
    };
    let all: &[Entry] = &[ENVELOPE, NAME, VARIANT];
    let depth = Some("crate::ext::Depth");
    run(RECORDS, &with(all, depth)).expect("the complete table generates");
    let without = |name: &str| -> Vec<Entry> {
        all.iter()
            .filter(|entry| entry.0 != name)
            .copied()
            .collect()
    };
    for (options, expected) in [
        // A type that the output needs is neither generated nor listed.
        (
            with(&without("Variant"), depth),
            "refers to type `Variant`, which the options do not enable and do not list under [external]",
        ),
        // A table entry that names nothing in the mspec files.
        (
            with(
                &[all, &[("Absent", "crate::ext::Absent", &[])]].concat(),
                depth,
            ),
            "external type `Absent` is not defined by the mspec files",
        ),
        // The budget type is what makes the signature.
        (with(all, None), "an [external] table needs `depth`"),
        (
            with(all, Some("Depth")),
            "`depth` = \"Depth\" is not a Rust path",
        ),
        (
            with(
                &[("Envelope", "crate::ext::Envelope", &[]), NAME, VARIANT],
                depth,
            ),
            "external type `Envelope` takes 1 parameters in the mspec (line 4) and the table pins 0",
        ),
        (
            with(
                &[
                    ("Envelope", "crate::ext::Envelope", &["maybe"]),
                    NAME,
                    VARIANT,
                ],
                depth,
            ),
            "pins `maybe` for the parameter `masked`",
        ),
        (
            with(&[ENVELOPE, ("Name", "crate::ext::fn", &[]), VARIANT], depth),
            "external type `Name` maps to `crate::ext::fn`, which is not a Rust path",
        ),
        // A use passes other arguments than the ones the type is implemented for.
        (
            with(
                &[
                    ("Envelope", "crate::ext::Envelope", &["false"]),
                    NAME,
                    VARIANT,
                ],
                depth,
            ),
            "`Envelope` is external, implemented for the arguments (false); this use passes ('true')",
        ),
    ] {
        let error = run(RECORDS, &options).expect_err(expected);
        assert!(error.contains(expected), "{error}");
    }
    // Generated and external at once is no choice.
    let mut both = with(all, depth);
    both.types.insert("Name".to_owned());
    let error = run(RECORDS, &both).expect_err("both");
    assert!(error.contains("is also enabled in `types`"), "{error}");
    // An enum stays generated.
    let mut options = with(&[("Mode", "crate::ext::Mode", &[])], depth);
    options.types = ["Record"].map(str::to_owned).into();
    let error = run(
        "[enum uint 8 Mode ['0' OFF]] [type Record [simple Mode mode]]",
        &options,
    )
    .expect_err("an enum");
    assert!(
        error.contains("a hand-written type cannot stand for"),
        "{error}"
    );
    // An external type is referred to where the budget is: a protocol without one is not
    // changed by the option being absent.
    let plain = run(
        "[type Name [simple uint 8 length]] [type Record [simple Name name]]",
        &options_without_externals(),
    )
    .expect("generates");
    let record = plain.get("record").expect("the record module");
    assert!(
        !record.contains("depth") && !record.contains("Depth"),
        "no budget without the option: {record}"
    );
}

/// `depth` names the budget parameter only with the `depth` option; without it the name is as
/// free as any other, which keeps a protocol generated before S79 generating.
#[test]
fn generator_depth_is_a_reserved_name_only_with_the_depth_option() {
    let text = "[type Probe [simple uint 8 depth]]";
    let mut options = options(&["Probe"], &[], None);
    let modules = run(text, &options).expect("a field named depth without the option");
    assert!(
        modules
            .get("probe")
            .expect("the probe module")
            .contains("pub(crate) depth: u8,"),
        "the field keeps its name"
    );
    options.depth = Some("crate::ext::Depth".to_owned());
    let error = run(text, &options).expect_err("a field named depth with the option");
    assert!(error.contains("name `depth` becomes `depth`"), "{error}");
}

fn options_without_externals() -> Options {
    options(&["Name", "Record"], &[], None)
}
