//! String and `ByteString`: an `Int32` length (-1 is null, distinct from empty) and the bytes.
//!
//! A String is UTF-8 text, but the codec keeps the bytes it read and does not refuse bytes that
//! are not UTF-8: the value layer decides, item by item, what a bad String means. The length
//! is checked against [`MAX_STRING_BYTES`] and the bytes that remain before any byte is
//! copied, so a declared length never reserves memory.

use std::str::Utf8Error;

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE, MAX_STRING_BYTES};

/// The length that stands for a null String or `ByteString`.
const NULL_LENGTH: i32 = -1;

/// The bytes a length-prefixed value declares, or `None` for the null length.
fn read_body<'a>(
    reader: &mut BoundedReader<'a>,
    field: &'static str,
) -> Result<Option<&'a [u8]>, ParseError> {
    let length = reader.read_i32(LITTLE)?;
    if length == NULL_LENGTH {
        return Ok(None);
    }
    // A negative length other than -1 is no valid encoding; the value is its magnitude.
    let invalid = || ParseError::InvalidDiscriminator {
        field,
        value: u64::from(length.unsigned_abs()),
    };
    let Ok(length) = usize::try_from(length) else {
        return Err(invalid());
    };
    reader.read_bytes(length, MAX_STRING_BYTES).map(Some)
}

/// The bytes of a length-prefixed value, written after their length.
fn write_body(writer: &mut BoundedWriter, body: Option<&[u8]>) -> Result<(), ParseError> {
    // `body_length` bounds the length by `MAX_STRING_BYTES`, so the prefix fits.
    body_length(body)?;
    let prefix = body.map_or(NULL_LENGTH, |bytes| {
        i32::try_from(bytes.len()).unwrap_or(i32::MAX)
    });
    writer.write_i32(prefix, LITTLE)?;
    writer.write_bytes(body.unwrap_or_default())
}

/// The encoded size of a length-prefixed value, refusing a body over the maximum.
fn body_length(body: Option<&[u8]>) -> Result<usize, ParseError> {
    let size = body.map_or(0, <[u8]>::len);
    if size > MAX_STRING_BYTES {
        return Err(ParseError::OversizedLength {
            length: size,
            max: MAX_STRING_BYTES,
        });
    }
    size.checked_add(4).ok_or(ParseError::Overflow)
}

/// An OPC UA String: the bytes of UTF-8 text, or null.
///
/// A null String (length -1) is not an empty one (length 0); a value read and written back
/// keeps its form. The bytes are kept as read, valid UTF-8 or not, so that a value always
/// writes back to the bytes it was read from; [`UaString::text`] says whether they are text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct UaString(pub(in crate::opcua) Option<Vec<u8>>);

impl UaString {
    /// The null String.
    pub(in crate::opcua) const fn null() -> Self {
        Self(None)
    }

    /// A String holding `text`, which may be empty.
    pub(in crate::opcua) fn new(text: impl Into<String>) -> Self {
        Self(Some(text.into().into_bytes()))
    }

    /// A String holding `bytes` as they are, which may not be UTF-8.
    pub(in crate::opcua) fn from_bytes(bytes: impl Into<Vec<u8>>) -> Self {
        Self(Some(bytes.into()))
    }

    /// The bytes, or `None` for the null String.
    pub(in crate::opcua) fn as_bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }

    /// The text, `None` for the null String, or the error for bytes that are not UTF-8.
    pub(in crate::opcua) fn text(&self) -> Option<Result<&str, Utf8Error>> {
        self.as_bytes().map(std::str::from_utf8)
    }

    /// Reads a String.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when the bytes are not all present;
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`];
    /// [`ParseError::InvalidDiscriminator`] for a negative length other than -1 (field
    /// `String length`, the magnitude). Bytes that are not UTF-8 are not an error here.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        _depth: Depth,
    ) -> Result<Self, ParseError> {
        Ok(Self(
            read_body(reader, "String length")?.map(<[u8]>::to_vec),
        ))
    }

    /// Writes the String.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`] or the writer's capacity.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        _depth: Depth,
    ) -> Result<(), ParseError> {
        write_body(writer, self.as_bytes())
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`].
    pub(in crate::opcua) fn length_in_bytes(&self, _depth: Depth) -> Result<usize, ParseError> {
        body_length(self.as_bytes())
    }
}

/// An OPC UA `ByteString`: bytes, or null.
///
/// A null `ByteString` (length -1) is not an empty one (length 0).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct UaByteString(pub(in crate::opcua) Option<Vec<u8>>);

impl UaByteString {
    /// The null `ByteString`.
    pub(in crate::opcua) const fn null() -> Self {
        Self(None)
    }

    /// A `ByteString` holding `bytes`, which may be empty.
    pub(in crate::opcua) fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(Some(bytes.into()))
    }

    /// The bytes, or `None` for the null `ByteString`.
    pub(in crate::opcua) fn as_bytes(&self) -> Option<&[u8]> {
        self.0.as_deref()
    }

    /// Reads a `ByteString`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when the bytes are not all present;
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`];
    /// [`ParseError::InvalidDiscriminator`] for a negative length other than -1 (field
    /// `ByteString length`, the magnitude).
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        _depth: Depth,
    ) -> Result<Self, ParseError> {
        Ok(Self(
            read_body(reader, "ByteString length")?.map(<[u8]>::to_vec),
        ))
    }

    /// Writes the `ByteString`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`] or the writer's capacity.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        _depth: Depth,
    ) -> Result<(), ParseError> {
        write_body(writer, self.as_bytes())
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over [`MAX_STRING_BYTES`].
    pub(in crate::opcua) fn length_in_bytes(&self, _depth: Depth) -> Result<usize, ParseError> {
        body_length(self.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{assert_cut_off, assert_vector, decode, encode};

    /// The length prefix of a body of `length` bytes, with `body` after it.
    fn with_length(length: i32, body: &[u8]) -> Vec<u8> {
        let mut bytes = length.to_le_bytes().to_vec();
        bytes.extend_from_slice(body);
        bytes
    }

    // covers: SPEC mandatory-failure-cases / null versus empty values
    #[test]
    fn opcua_codec_null_string_is_distinct_from_empty() {
        assert_vector(&UaString::null(), &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_vector(&UaString::new(""), &[0x00, 0x00, 0x00, 0x00]);
        assert_ne!(UaString::null(), UaString::new(""));
        assert_eq!(UaString::null().text(), None);
        assert_eq!(UaString::new("").text(), Some(Ok("")));
        assert_eq!(UaString::null().as_bytes(), None);
        assert_eq!(UaString::new("a").as_bytes(), Some(&b"a"[..]));
        assert_eq!(UaString::default(), UaString::null());
    }

    #[test]
    fn opcua_codec_null_byte_string_is_distinct_from_empty() {
        assert_vector(&UaByteString::null(), &[0xFF, 0xFF, 0xFF, 0xFF]);
        assert_vector(&UaByteString::new(Vec::new()), &[0x00, 0x00, 0x00, 0x00]);
        assert_ne!(UaByteString::null(), UaByteString::new(Vec::new()));
        assert_eq!(UaByteString::null().as_bytes(), None);
        assert_eq!(UaByteString::new([1]).as_bytes(), Some(&[1_u8][..]));
        assert_eq!(UaByteString::default(), UaByteString::null());
    }

    #[test]
    fn opcua_codec_string_counts_bytes_not_characters() {
        // The specification's worked example: U+6C34 is three bytes in UTF-8.
        let text = UaString::new("\u{6C34}Boy");
        assert_vector(
            &text,
            &[0x06, 0x00, 0x00, 0x00, 0xE6, 0xB0, 0xB4, 0x42, 0x6F, 0x79],
        );
        assert_cut_off(&text);
        assert_cut_off(&UaByteString::new([1, 2, 3]));
    }

    #[test]
    fn opcua_codec_string_refuses_a_negative_length_other_than_null() {
        for length in [-2, -3, i32::MIN] {
            let bytes = with_length(length, &[]);
            assert_eq!(
                decode::<UaString>(&bytes),
                Err(ParseError::InvalidDiscriminator {
                    field: "String length",
                    value: u64::from(length.unsigned_abs()),
                }),
                "{length}"
            );
            assert_eq!(
                decode::<UaByteString>(&bytes),
                Err(ParseError::InvalidDiscriminator {
                    field: "ByteString length",
                    value: u64::from(length.unsigned_abs()),
                }),
                "{length}"
            );
        }
    }

    #[test]
    fn opcua_codec_string_keeps_bytes_that_are_not_utf8() {
        // A lead byte with no continuation after one valid byte: the text is an error whose
        // valid prefix is one byte, and the bytes are kept and written back unchanged.
        let bytes = with_length(3, &[b'a', 0xC3, 0x28]);
        let parsed = decode::<UaString>(&bytes).expect("bytes are not refused for being UTF-8");
        assert_eq!(parsed, UaString::from_bytes([b'a', 0xC3, 0x28]));
        assert_eq!(
            parsed
                .text()
                .expect("not null")
                .expect_err("not UTF-8")
                .valid_up_to(),
            1
        );
        assert_eq!(encode(&parsed), bytes);
        assert_eq!(parsed.length_in_bytes(Depth::new()), Ok(bytes.len()));
        // The same bytes are a valid ByteString.
        assert_eq!(
            decode::<UaByteString>(&bytes),
            Ok(UaByteString::new([b'a', 0xC3, 0x28]))
        );
    }

    #[test]
    fn opcua_codec_string_length_is_bounded_before_the_bytes_are_looked_for() {
        // Only the four length bytes are present: a hostile length costs nothing.
        for length in [
            MAX_STRING_BYTES + 1,
            usize::try_from(i32::MAX).expect("fits"),
        ] {
            let prefix = i32::try_from(length).expect("fits").to_le_bytes();
            let refused = ParseError::OversizedLength {
                length,
                max: MAX_STRING_BYTES,
            };
            assert_eq!(decode::<UaString>(&prefix), Err(refused.clone()));
            assert_eq!(decode::<UaByteString>(&prefix), Err(refused));
        }
        // A length the frame cannot hold but the maximum allows is a truncation.
        let prefix = i32::try_from(MAX_STRING_BYTES).expect("fits").to_le_bytes();
        assert_eq!(
            decode::<UaByteString>(&prefix),
            Err(ParseError::Truncated {
                needed: MAX_STRING_BYTES,
                available: 0
            })
        );
    }

    #[test]
    fn opcua_codec_string_at_the_maximum_length_round_trips_and_one_more_does_not() {
        let at_maximum = UaByteString::new(vec![0x5A; MAX_STRING_BYTES]);
        let bytes = encode(&at_maximum);
        assert_eq!(bytes.len(), MAX_STRING_BYTES + 4);
        assert_eq!(decode::<UaByteString>(&bytes), Ok(at_maximum.clone()));
        assert_eq!(
            at_maximum.length_in_bytes(Depth::new()),
            Ok(MAX_STRING_BYTES + 4)
        );

        let too_long = UaByteString::new(vec![0x5A; MAX_STRING_BYTES + 1]);
        let refused = ParseError::OversizedLength {
            length: MAX_STRING_BYTES + 1,
            max: MAX_STRING_BYTES,
        };
        assert_eq!(too_long.length_in_bytes(Depth::new()), Err(refused.clone()));
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(
            too_long.serialize(&mut writer, Depth::new()),
            Err(refused.clone())
        );
        assert!(writer.as_slice().is_empty(), "nothing is written");

        let long_text = UaString::new("a".repeat(MAX_STRING_BYTES + 1));
        assert_eq!(
            long_text.length_in_bytes(Depth::new()),
            Err(refused.clone())
        );
        assert_eq!(
            long_text.serialize(&mut BoundedWriter::new(usize::MAX), Depth::new()),
            Err(refused)
        );
    }

    #[test]
    fn opcua_codec_string_trailing_bytes_are_left_to_the_caller() {
        let mut bytes = encode(&UaString::new("ab"));
        bytes.push(0x99);
        let mut reader = BoundedReader::new(&bytes);
        assert_eq!(
            UaString::parse(&mut reader, Depth::new()),
            Ok(UaString::new("ab"))
        );
        assert_eq!(
            reader.remaining(),
            1,
            "the byte after the String is untouched"
        );
    }
}
