//! Tests on first-party synthetic mspec fixtures (slice S78): `count` arrays of numbers,
//! enums and structs, `implicit` int 32 and uint 32 counts, the expression subset
//! `x == null ? -1 : COUNT(x)` and a null array that stays distinct from an empty one. The
//! text is written here from nothing but the grammar, as the S77 fixtures are, and is
//! compiled with `rustc` against the same stand-in for `stonecrab::codec`.

use super::fixtures::{STAND_IN, compile_and_run, options, run};
use super::{Endianness, Options};

/// Every kind of array: a nullable array of numbers, of enums, of structs (with a
/// parameter passed on) and of bytes, and a non-nullable array of numbers, all counted by an
/// `implicit` field of int 32 or uint 32; an array whose count is a literal, a parameter and
/// a plain `simple` field; a count that maps -1 to empty; and an `implicit` that disagrees
/// with the count it checks.
const ARRAYS: &str = "
[enum uint 8 Mode ['1' ON] ['2' OFF]]
[type Entry(uint 8 scale)
    [simple uint 16 key]
    [implicit uint 8 labelLength 'COUNT(label)']
    [array byte label count 'labelLength']
    [array uint 8 extra count 'scale']
]
[type Table(uint 8 scale)
    [implicit int 32 noOfWords 'words == null ? -1 : COUNT(words)']
    [array uint 16 words count 'noOfWords']
    [implicit uint 32 noOfModes 'COUNT(modes)']
    [array Mode modes count 'noOfModes']
    [implicit int 32 noOfEntries 'entries == null ? -1 : COUNT(entries)']
    [array Entry('scale') entries count 'noOfEntries']
    [implicit int 32 noOfBlob 'blob == null ? -1 : COUNT(blob)']
    [array byte blob count 'noOfBlob']
    [implicit int 32 noOfWide 'COUNT(wide)']
    [array uint 32 wide count 'noOfWide']
]
[type Wrapper(uint 8 scale)
    [simple Entry('scale') probe]
    [simple Table('scale') table]
    [implicit uint 32 span 'probe.lengthInBytes + table.lengthInBytes']
]
[type Pascal
    [simple int 32 length]
    [array byte value count 'length == -1 ? 0 : length']
]
[type Echo
    [simple int 32 declared]
    [array uint 8 data count 'declared']
    [implicit int 32 echo 'COUNT(data)']
]
[type Pair
    [array uint 16 pair count '2']
    [simple uint 8 tail]
]
[discriminatedType Request(uint 8 scale)
    [discriminator uint 8 kind]
    [typeSwitch kind
        ['1' Names
            [implicit int 32 noOfNames 'names == null ? -1 : COUNT(names)']
            [array Entry('scale') names count 'noOfNames']
        ]
        ['2' Nothing]
    ]
]
";

const ARRAY_TYPES: [&str; 8] = [
    "Echo", "Entry", "Mode", "Pair", "Pascal", "Request", "Table", "Wrapper",
];

fn arrays() -> Options {
    options(
        &ARRAY_TYPES,
        &[("Request", &["Names", "Nothing"])],
        Some(Endianness::Little),
    )
}

/// The program around `body`: the generated modules are in scope as `table::Table` and so
/// on, with a helper to serialize and the bytes of a count.
fn program(body: &str) -> String {
    format!(
        r"
use codec::{{BoundedReader, BoundedWriter, ParseError}};
use entry::Entry;
use mode::Mode;
use table::Table;

fn le(count: i32) -> Vec<u8> {{
    count.to_le_bytes().to_vec()
}}

fn join(parts: &[&[u8]]) -> Vec<u8> {{
    parts.concat()
}}

fn table_bytes(table: &Table) -> Vec<u8> {{
    let mut writer = BoundedWriter::new();
    table.serialize(&mut writer).unwrap();
    writer.into_bytes()
}}

fn parse_table(bytes: &[u8], scale: u8) -> Result<(Table, usize), ParseError> {{
    let mut reader = BoundedReader::new(bytes);
    let table = Table::parse(&mut reader, scale)?;
    Ok((table, reader.remaining()))
}}

fn sample() -> Table {{
    Table {{
        words: Some(vec![0x0102, 0x0304]),
        modes: vec![Mode::On, Mode::Off],
        entries: Some(vec![Entry {{ key: 0x0A0B, label: vec![7, 8], extra: vec![9] }}]),
        blob: None,
        wide: vec![0xAABB_CCDD],
    }}
}}

{body}
"
    )
}

fn run_program(test: &str, body: &str) -> Result<(), String> {
    let modules = run(ARRAYS, &arrays())?;
    // The stand-in is shared with the S77 fixtures; `STAND_IN` is only named here so a
    // change to its signatures is a change to both.
    assert!(
        STAND_IN.contains("CountLimit"),
        "the stand-in has the count limit"
    );
    compile_and_run(test, &modules, &program(body))
}

#[test]
fn generator_complex_array_count_is_checked_before_allocation() {
    let modules = run(ARRAYS, &arrays()).expect("generates");
    let table = modules.get("table").expect("the table module");
    assert!(!table.contains(" as "), "no `as` cast");
    for (needle, why) in [
        (
            "pub(crate) words: Option<Vec<u16>>,",
            "a nullable array of numbers",
        ),
        ("pub(crate) modes: Vec<Mode>,", "an array of enums"),
        (
            "pub(crate) entries: Option<Vec<Entry>>,",
            "a nullable array of structs",
        ),
        (
            "pub(crate) blob: Option<Vec<u8>>,",
            "a nullable array of bytes",
        ),
        ("pub(crate) wide: Vec<u32>,", "an array of numbers"),
    ] {
        assert!(table.contains(needle), "{why}: {needle}\n{table}");
    }
    // Every capacity follows the check of the count against the input, on the line before.
    let reserves: Vec<&str> = table.split("Vec::with_capacity(array_count)").collect();
    for before in reserves.iter().take(reserves.len() - 1) {
        let check = before.lines().rev().nth(1).unwrap_or_default();
        assert!(
            check.contains("ParseError::CountLimit"),
            "a capacity without its check:\n{table}"
        );
    }
    assert!(
        table.matches("Vec::with_capacity(array_count)").count() >= 3,
        "number and enum arrays reserve after the check:\n{table}"
    );
    assert!(
        table.matches("Vec::new()").count() >= 1,
        "an array of structs grows one element at a time:\n{table}"
    );

    run_program(
        "array-counts",
        r#"
fn main() {
    // A count above what the input can hold is refused before any element is read.
    let hostile = [
        (le(i32::MAX), "words", i32::MAX as usize, 0),
        (join(&[&le(3), &[0; 5]]), "words", 3, 2),
        (join(&[&le(-1), &(u32::MAX).to_le_bytes()]), "modes", u32::MAX as usize, 0),
        (join(&[&le(-1), &le(0), &le(i32::MAX), &[0; 10]]), "entries", i32::MAX as usize, 10),
        (join(&[&le(-1), &le(0), &le(-1), &le(1_000_000)]), "blob", 0, 0),
    ];
    for (bytes, field, count, max) in hostile {
        let result = parse_table(&bytes, 1);
        if field == "blob" {
            // A byte array is bounded by the bytes present, as ever.
            assert!(matches!(result, Err(ParseError::Truncated { .. })), "{result:?}");
            continue;
        }
        assert_eq!(result, Err(ParseError::CountLimit { field, count, max }), "{field}");
    }

    // At the bound it passes: two 16-bit words need four bytes, three need six.
    let two = join(&[&le(2), &[1, 0, 2, 0]]);
    assert!(matches!(parse_table(&two, 1), Err(ParseError::Truncated { .. })), "past the words");
    let short = join(&[&le(2), &[1, 0, 2]]);
    assert_eq!(
        parse_table(&short, 1),
        Err(ParseError::CountLimit { field: "words", count: 2, max: 1 })
    );

    // Elements of no fixed width grow one at a time: a count that fits the bytes but not
    // the elements is the element's own truncation.
    let one_entry = join(&[&le(-1), &le(0), &le(3), &[0x0B, 0x0A, 0, 9]]);
    assert!(matches!(parse_table(&one_entry, 1), Err(ParseError::Truncated { .. })));

    // A count below -1 is no count; -1 is null only where an implicit says so.
    assert_eq!(
        parse_table(&le(-2), 1),
        Err(ParseError::InvalidDiscriminator { field: "words", value: 2 })
    );
    assert_eq!(
        parse_table(&le(i32::MIN), 1),
        Err(ParseError::InvalidDiscriminator { field: "words", value: 1 << 31 })
    );
    let wide_null = join(&[&le(-1), &le(0), &le(-1), &le(-1), &le(-1)]);
    assert_eq!(
        parse_table(&wide_null, 1),
        Err(ParseError::InvalidDiscriminator { field: "wide", value: 1 })
    );

    // A valid table still parses and serializes to the same bytes.
    let bytes = table_bytes(&sample());
    assert_eq!(parse_table(&bytes, 1), Ok((sample(), 0)));
}
"#,
    )
    .expect("the count bounds");
}

#[test]
fn generator_null_array_is_distinct_from_an_empty_array() {
    // A property loop with a fixed seed over random tables and mutated bytes. The tool has
    // no dependency but `toml` and `sha2`, and the program is compiled with `rustc` alone,
    // so the generator is checked with a small xorshift here and not with the proptest
    // crate.
    run_program(
        "null-array",
        r#"
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, limit: u64) -> usize {
        usize::try_from(self.next() % limit).unwrap()
    }

    fn bytes(&mut self, length: usize) -> Vec<u8> {
        (0..length).map(|_| self.next().to_le_bytes()[0]).collect()
    }
}

fn random_entry(rng: &mut Rng, scale: u8) -> Entry {
    let key = u16::from_le_bytes([rng.bytes(1)[0], rng.bytes(1)[0]]);
    let label_length = rng.below(4);
    Entry { key, label: rng.bytes(label_length), extra: rng.bytes(usize::from(scale)) }
}

fn random_table(rng: &mut Rng, scale: u8) -> Table {
    let words = (rng.below(3) != 0).then(|| {
        let length = rng.below(5);
        (0..length).map(|_| u16::from_le_bytes([rng.bytes(1)[0], rng.bytes(1)[0]])).collect()
    });
    let modes_length = rng.below(4);
    let modes = (0..modes_length)
        .map(|_| if rng.below(2) == 0 { Mode::On } else { Mode::Off })
        .collect();
    let entries = (rng.below(3) != 0).then(|| {
        let length = rng.below(4);
        (0..length).map(|_| random_entry(rng, scale)).collect()
    });
    let blob = (rng.below(3) != 0).then(|| {
        let length = rng.below(6);
        rng.bytes(length)
    });
    let wide_length = rng.below(4);
    let wide = (0..wide_length).map(|_| rng.next() as u32).collect();
    Table { words, modes, entries, blob, wide }
}

fn main() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for round in 0..600 {
        let scale = u8::try_from(round % 3).unwrap();
        let table = random_table(&mut rng, scale);
        let bytes = table_bytes(&table);
        assert_eq!(parse_table(&bytes, scale), Ok((table.clone(), 0)), "round trip {round}");

        // Null and empty: the same table with the array absent or empty has different
        // bytes, and each parses back to what it was.
        let mut null = table.clone();
        let mut empty = table.clone();
        null.words = None;
        empty.words = Some(vec![]);
        null.entries = None;
        empty.entries = Some(vec![]);
        null.blob = None;
        empty.blob = Some(vec![]);
        let (null_bytes, empty_bytes) = (table_bytes(&null), table_bytes(&empty));
        assert_ne!(null_bytes, empty_bytes, "null and empty differ on the wire");
        assert_eq!(parse_table(&null_bytes, scale), Ok((null.clone(), 0)));
        assert_eq!(parse_table(&empty_bytes, scale), Ok((empty.clone(), 0)));
        assert_ne!(null, empty, "and as values");
        assert!(null_bytes.windows(4).any(|window| window == le(-1)), "-1 for null");
        assert_eq!(&empty_bytes[..4], le(0).as_slice(), "0 for empty");

        // Mutated and truncated bytes never panic, and what parses is canonical: it
        // serializes to exactly the bytes it was read from.
        for _ in 0..12 {
            let mut mutated = bytes.clone();
            match rng.below(3) {
                0 => {
                    let at = rng.below(mutated.len() as u64);
                    mutated[at] = rng.bytes(1)[0];
                }
                1 => mutated.truncate(rng.below(mutated.len() as u64 + 1)),
                _ => {
                    let length = rng.below(24);
                    mutated = rng.bytes(length);
                }
            }
            if let Ok((parsed, left)) = parse_table(&mutated, scale) {
                let used = mutated.len() - left;
                assert_eq!(table_bytes(&parsed), mutated[..used].to_vec(), "canonical");
            }
        }
    }
}
"#,
    )
    .expect("the property loop");
}

#[test]
fn generator_implicit_int32_count_round_trips() {
    run_program(
        "implicit-int32",
        r#"
use echo::Echo;
use pair::Pair;
use pascal::Pascal;
use request::Request;
use wrapper::Wrapper;

fn main() {
    // The wire layout, little-endian: every count is four bytes, -1 is null.
    let expected = join(&[
        &le(2), &[0x02, 0x01, 0x04, 0x03],
        &le(2), &[1, 2],
        &le(1), &[0x0B, 0x0A, 2, 7, 8, 9],
        &le(-1),
        &le(1), &[0xDD, 0xCC, 0xBB, 0xAA],
    ]);
    assert_eq!(table_bytes(&sample()), expected);
    assert_eq!(parse_table(&expected, 1), Ok((sample(), 0)));

    // An implicit uint 32 count, and a field that is not null and not empty.
    let mut empty = sample();
    empty.modes = vec![];
    empty.wide = vec![];
    empty.words = Some(vec![]);
    empty.entries = Some(vec![]);
    empty.blob = Some(vec![]);
    let bytes = table_bytes(&empty);
    assert_eq!(bytes, join(&[&le(0), &le(0), &le(0), &le(0), &le(0)]));
    assert_eq!(parse_table(&bytes, 1), Ok((empty, 0)));

    // The lengths of a struct that holds arrays: the sum of the parts, the implicit
    // checked against it.
    let wrapper = Wrapper {
        probe: Entry { key: 1, label: vec![5], extra: vec![6] },
        table: sample(),
    };
    let mut writer = BoundedWriter::new();
    wrapper.serialize(&mut writer).unwrap();
    let bytes = writer.into_bytes();
    let table_length = table_bytes(&sample()).len();
    assert_eq!(wrapper.table.length_in_bytes(), Ok(table_length));
    assert_eq!(wrapper.probe.length_in_bytes(), Ok(5));
    assert_eq!(bytes.len(), 5 + table_length + 4);
    assert_eq!(&bytes[5 + table_length..], &u32::try_from(5 + table_length).unwrap().to_le_bytes());
    assert_eq!(Wrapper::parse(&mut BoundedReader::new(&bytes), 1), Ok(wrapper));
    let mut wrong = bytes.clone();
    *wrong.last_mut().unwrap() = 99;
    assert!(matches!(
        Wrapper::parse(&mut BoundedReader::new(&wrong), 1),
        Err(ParseError::InvalidDiscriminator { field: "span", .. })
    ));

    // An implicit int 32 checked against the array it counts, when another field gave the
    // count: the value in the error is the magnitude.
    let echo = |declared: i32, data: &[u8], echo: i32| join(&[&le(declared), data, &le(echo)]);
    let ok = echo(2, &[8, 9], 2);
    let parsed = Echo::parse(&mut BoundedReader::new(&ok)).unwrap();
    assert_eq!(parsed.declared, 2);
    assert_eq!(parsed.data, Some(vec![8, 9]));
    let mut writer = BoundedWriter::new();
    parsed.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), ok);

    // A count that is a plain `simple int 32` field is null at -1 as well (the shape of
    // `ByteStringArray`): None for -1, Some(empty) for 0, anything below -1 is no count,
    // and the stored count must agree with the value on serialize.
    let null = echo(-1, &[], 0);
    let parsed = Echo::parse(&mut BoundedReader::new(&null)).unwrap();
    assert_eq!(parsed, Echo { declared: -1, data: None });
    let empty = echo(0, &[], 0);
    assert_eq!(
        Echo::parse(&mut BoundedReader::new(&empty)),
        Ok(Echo { declared: 0, data: Some(vec![]) })
    );
    assert_eq!(
        Echo::parse(&mut BoundedReader::new(&echo(-2, &[], 0))),
        Err(ParseError::InvalidDiscriminator { field: "data", value: 2 })
    );
    assert_eq!(
        Echo::parse(&mut BoundedReader::new(&echo(i32::MAX, &[], 0))),
        Err(ParseError::CountLimit { field: "data", count: i32::MAX as usize, max: 4 })
    );
    for value in [
        Echo { declared: -1, data: None },
        Echo { declared: 0, data: Some(vec![]) },
        Echo { declared: 1, data: Some(vec![5]) },
    ] {
        let mut writer = BoundedWriter::new();
        value.serialize(&mut writer).unwrap();
        let bytes = writer.into_bytes();
        assert_eq!(Echo::parse(&mut BoundedReader::new(&bytes)), Ok(value));
    }
    for (value, magnitude) in [
        (Echo { declared: 3, data: Some(vec![1]) }, 1),
        (Echo { declared: -1, data: Some(vec![]) }, 0),
        (Echo { declared: 0, data: None }, 1),
    ] {
        let mut writer = BoundedWriter::new();
        assert_eq!(
            value.serialize(&mut writer),
            Err(ParseError::InvalidDiscriminator { field: "data", value: magnitude }),
            "{value:?}"
        );
    }
    for (wire, magnitude) in [(3, 3), (-3, 3), (-1, 1), (0, 0)] {
        assert_eq!(
            Echo::parse(&mut BoundedReader::new(&echo(2, &[8, 9], wire))),
            Err(ParseError::InvalidDiscriminator { field: "echo", value: magnitude }),
            "{wire}"
        );
    }

    // A count that maps -1 to an empty array: not null, so it serializes as 0.
    let minus_one = le(-1);
    let pascal = Pascal::parse(&mut BoundedReader::new(&minus_one)).unwrap();
    assert_eq!((pascal.length, pascal.value.clone()), (-1, vec![]));
    assert_eq!(
        Pascal::parse(&mut BoundedReader::new(&le(-2))),
        Err(ParseError::InvalidDiscriminator { field: "value", value: 2 })
    );
    let three = join(&[&le(3), &[1, 2, 3]]);
    assert_eq!(Pascal::parse(&mut BoundedReader::new(&three)).unwrap().value, vec![1, 2, 3]);

    // A literal count.
    let pair = Pair::parse(&mut BoundedReader::new(&[1, 0, 2, 0, 9])).unwrap();
    assert_eq!((pair.pair.clone(), pair.tail), (vec![1, 2], 9));
    assert_eq!(
        Pair::parse(&mut BoundedReader::new(&[1, 0, 2])),
        Err(ParseError::CountLimit { field: "pair", count: 2, max: 1 })
    );

    // A nullable array in a case of a discriminated type.
    let names = join(&[&[1], &le(-1)]);
    let request = Request::parse(&mut BoundedReader::new(&names), 1).unwrap();
    assert_eq!(request, Request::Names { names: None });
    let mut writer = BoundedWriter::new();
    request.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), names);
    let none = join(&[&[1], &le(0)]);
    assert_eq!(
        Request::parse(&mut BoundedReader::new(&none), 1),
        Ok(Request::Names { names: Some(vec![]) })
    );
    assert_eq!(Request::parse(&mut BoundedReader::new(&[2]), 1), Ok(Request::Nothing));
}
"#,
    )
    .expect("the implicit int 32 counts");
}

#[test]
fn generator_array_gaps_are_refused_with_the_construct_the_type_and_the_line() {
    for (text, expected) in [
        ("[type T [array bit flags count '8']]", "of type `bit`"),
        (
            "[type T [array uint 16 words length '8']]",
            "other than a `count` array",
        ),
        (
            "[type T [implicit uint 8 n 'x == null ? 0 : COUNT(x)'] [array uint 16 x count 'n']]",
            "its count is not signed",
        ),
        (
            "[type T [simple uint 8 n] [simple uint 8 y] [implicit int 32 m 'n == null ? -1 : 0']]",
            "`n == null` needs an array",
        ),
        (
            "[type T [implicit int 32 m 'COUNT(n)'] [simple uint 8 n]]",
            "COUNT of `n`, which is not an array",
        ),
        (
            "[type T [implicit int 32 m '!COUNT(x)'] [array uint 8 x count 'm']]",
            "`!`",
        ),
        (
            "[type T [simple int 32 m] [array uint 8 x count 'm == 1'] ]",
            "a comparison where a number is needed",
        ),
        (
            "[type T [simple int 32 m] [array uint 8 x count 'm ? 1 : 2']]",
            "`?:` with a condition that is not a comparison",
        ),
        (
            "[type T [implicit int 32 m 'x == null ? -1 : COUNT(x)'] [array uint 8 x count 'm' ] [implicit int 32 k '3 - 1']]",
            "`-`",
        ),
    ] {
        let error = run(text, &options(&["T"], &[], None)).expect_err(text);
        assert!(error.contains(expected), "{text}: {error}");
        assert!(error.contains("type T"), "{text}: {error}");
        assert!(error.contains("line 1"), "{text}: {error}");
    }
}

#[test]
fn generator_array_gaps_name_the_line_of_the_field() {
    for (text, expected, line) in [
        (
            "[type T\n    [simple int 32 m]\n    [array uint 8 x count 'm == 1']\n]",
            "a comparison where a number is needed",
            3,
        ),
        (
            "[type T\n    [simple uint 8 m]\n    [simple uint 8 k]\n    [array uint 8 x count 'k ? 1 : 2']\n]",
            "`?:` with a condition that is not a comparison",
            4,
        ),
        (
            "[type T\n    [implicit uint 8 m 'x == null ? 0 : COUNT(x)']\n    [array uint 8 x count 'm']\n]",
            "its count is not signed",
            3,
        ),
    ] {
        let error = run(text, &options(&["T"], &[], None)).expect_err(text);
        assert!(error.contains(expected), "{text}: {error}");
        assert!(error.contains(&format!("line {line}")), "{text}: {error}");
    }
}

#[test]
fn generator_array_of_its_own_type_is_refused_as_recursive() {
    let text = "[type T [implicit int 32 n 'COUNT(k)'] [array T k count 'n']]";
    let error = run(text, &options(&["T"], &[], None)).expect_err(text);
    assert!(error.contains("recursive"), "{error}");
}

#[test]
fn generator_integer_literals_outside_i64_are_refused() {
    for literal in [
        "9223372036854775808",
        "0x8000000000000000",
        "-9223372036854775808",
        "-0x8000000000000000",
    ] {
        let text = format!(
            "[type T [implicit int 32 m '{literal} == COUNT(x) ? 1 : 0'] [array uint 8 x count 'm']]"
        );
        let error = run(&text, &options(&["T"], &[], None)).expect_err(&text);
        assert!(error.contains("not an integer in range"), "{text}: {error}");
    }
}

#[test]
fn generator_float_arrays_leave_out_eq() {
    let text = "
[type Samples
    [implicit uint 16 n 'COUNT(values)']
    [array float 32 values count 'n']
]
[type Names
    [implicit uint 16 n 'COUNT(values)']
    [array uint 16 values count 'n']
]
";
    let modules = run(text, &options(&["Names", "Samples"], &[], None)).expect("generates");
    let samples = modules.get("samples").expect("the samples module");
    assert!(
        samples.contains("pub(crate) values: Vec<f32>,"),
        "{samples}"
    );
    assert!(
        samples.contains("derive(Debug, Clone, PartialEq)]"),
        "{samples}"
    );
    assert!(
        modules
            .get("names")
            .expect("the names module")
            .contains("derive(Debug, Clone, PartialEq, Eq)"),
        "an array of integers is Eq"
    );
}
