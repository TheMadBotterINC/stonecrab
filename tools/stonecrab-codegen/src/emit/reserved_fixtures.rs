//! Tests on first-party synthetic mspec fixtures (slice S79): `reserved` fields, alone and in
//! a one-byte run with flags (`[reserved uint 7 '0x00'] [simple bit isOnline]` of the OPC UA
//! model). The text is written here from nothing but the grammar and is compiled with `rustc`
//! against the same stand-in for `stonecrab::codec`.

use super::Endianness;
use super::fixtures::{compile_and_run, options, run};

/// Reserved bits first and last in a run, in the middle of one, a whole reserved byte and a
/// byte made of nothing else, in a struct and in a case of a discriminated type.
const FLAGS: &str = "
[type Flags
    [reserved uint 7 '0x00']
    [simple bit online]
    [simple bit first]
    [reserved bit 'false']
    [simple uint 6 level]
    [reserved uint 16 '0x0000']
    [reserved int 5 '0x00']
    [simple bit x]
    [simple bit y]
    [simple bit z]
    [reserved uint 4 '0x05']
    [reserved uint 4 '0x0A']
]
[discriminatedType Message
    [discriminator uint 8 tag]
    [typeSwitch tag
        ['1' Gate
            [simple bit open]
            [reserved uint 7 '0x00']
        ]
    ]
]
";

#[test]
fn generator_reserved_bits_share_a_byte_with_a_flag() {
    let options = options(&["Flags", "Message"], &[("Message", &["Gate"])], None);
    let modules = run(FLAGS, &options).expect("generates");
    let flags = modules.get("flags").expect("the flags module");
    assert!(
        flags.contains("pub(crate) online: bool,")
            && flags.contains("pub(crate) level: u8,")
            && flags.contains("pub(crate) z: bool,"),
        "the flags are stored: {flags}"
    );
    assert!(
        !flags.contains("reserved"),
        "a reserved field is neither stored nor named in the output: {flags}"
    );
    assert!(!flags.contains(" as "), "no `as` cast");

    let main = r"
use codec::{BoundedReader, BoundedWriter};
use flags::Flags;
use message::Message;

fn bytes_of(flags: &Flags) -> Vec<u8> {
    let mut writer = BoundedWriter::new();
    flags.serialize(&mut writer).unwrap();
    writer.into_bytes()
}

fn main() {
    // Byte 0: seven reserved bits and `online`. Byte 1: `first`, a reserved bit, `level`
    // (0b101010). Bytes 2 and 3: a reserved uint 16. Byte 4: five reserved bits and
    // x, y, z (1, 0, 1). Byte 5: two reserved nibbles that hold 5 and 0xA.
    let bytes: Vec<u8> = vec![0x01, 0xAA, 0x00, 0x00, 0x05, 0x5A];
    let mut reader = BoundedReader::new(&bytes);
    let parsed = Flags::parse(&mut reader).unwrap();
    assert_eq!(reader.remaining(), 0);
    assert_eq!(
        parsed,
        Flags { online: true, first: true, level: 0b10_1010, x: true, y: false, z: true }
    );
    assert_eq!(bytes_of(&parsed), bytes);

    // Reserved bits are read and ignored, whatever they hold, and written as their value.
    let noisy: Vec<u8> = vec![0xFF, 0x7F, 0xDE, 0xAD, 0xF8 | 0x05, 0xFF];
    let parsed = Flags::parse(&mut BoundedReader::new(&noisy)).unwrap();
    assert!(parsed.online && !parsed.first && parsed.level == 0b11_1111);
    assert!(parsed.x && !parsed.y && parsed.z);
    assert_eq!(bytes_of(&parsed), vec![0x01, 0x3F, 0x00, 0x00, 0x05, 0x5A]);

    // A short input is the same refusal as for any field.
    assert!(Flags::parse(&mut BoundedReader::new(&bytes[..5])).is_err());

    // A case: the flag is the top bit of its byte, the reserved bits are the rest.
    let gate: Vec<u8> = vec![1, 0x80];
    let parsed = Message::parse(&mut BoundedReader::new(&gate)).unwrap();
    assert_eq!(parsed, Message::Gate { open: true });
    let mut writer = BoundedWriter::new();
    parsed.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), gate);
}
";
    compile_and_run("reserved-bits", &modules, main).expect("the reserved round trip");
}

#[test]
fn generator_reserved_fields_are_refused_when_they_cannot_be_written() {
    for (text, expected) in [
        (
            "[type T [reserved uint 7 '0x80'] [simple bit a]]",
            "value 128 does not fit 7 bits",
        ),
        (
            "[type T [reserved bit '2'] [reserved uint 7 '0x00']]",
            "whose value does not suit its type",
        ),
        (
            "[type T [reserved uint 7 'false'] [simple bit a]]",
            "whose value does not suit its type",
        ),
        (
            "[type T [reserved float 32 '0'] [simple uint 8 a]]",
            "of type `float 32`",
        ),
        (
            "[type T [reserved int 16 '0'] [simple uint 8 a]]",
            "of type `int 16`",
        ),
        (
            "[type T [reserved uint 8 'a'] [simple uint 8 a]]",
            "with a value that is not an integer or a boolean",
        ),
        (
            "[type T [reserved uint 3 '0'] [simple uint 8 a]]",
            "does not end on a byte boundary",
        ),
        (
            "[type T [simple uint 8 a] [reserved uint 5 '0']]",
            "does not end on a byte boundary",
        ),
        (
            "[type T [reserved uint 7 '0'] [simple uint 2 a] [simple uint 8 b]]",
            "longer than one byte",
        ),
        // Nothing is stored, so the struct holds no field.
        ("[type T [reserved uint 8 '0']]", "stores no field"),
    ] {
        let error = run(text, &options(&["T"], &[], Some(Endianness::Little))).expect_err(text);
        assert!(error.contains(expected), "{text}: {error}");
        assert!(error.contains("type T"), "{text}: {error}");
    }
    // The error names the line of the field.
    let error = run(
        "[type T\n    [simple uint 8 a]\n    [reserved uint 7 '0x80']\n    [simple bit b]\n]",
        &options(&["T"], &[], None),
    )
    .expect_err("too wide a value");
    assert!(error.contains("line 3"), "{error}");
}

/// A value of six digits or more is emitted with `_` separators, as `clippy::unreadable_literal`
/// of the crate lint table wants, however the mspec wrote it.
#[test]
fn generator_large_reserved_values_are_written_with_digit_separators() {
    let options = options(&["Wide"], &[], Some(Endianness::Big));
    let modules = run(
        "[type Wide [reserved uint 32 '0x01020304'] [simple uint 8 a]]",
        &options,
    )
    .expect("generates");
    let wide = modules.get("wide").expect("the wide module");
    assert!(
        wide.contains("write_u32(16_909_060,"),
        "the reserved value is grouped: {wide}"
    );
    let main = r"
use codec::{BoundedReader, BoundedWriter};
use wide::Wide;

fn main() {
    let bytes: Vec<u8> = vec![0xFF, 0xFF, 0xFF, 0xFF, 7];
    let parsed = Wide::parse(&mut BoundedReader::new(&bytes)).unwrap();
    assert_eq!(parsed, Wide { a: 7 });
    let mut writer = BoundedWriter::new();
    parsed.serialize(&mut writer).unwrap();
    assert_eq!(writer.into_bytes(), vec![1, 2, 3, 4, 7]);
}
";
    compile_and_run("reserved-wide", &modules, main).expect("the grouped value compiles");
}
