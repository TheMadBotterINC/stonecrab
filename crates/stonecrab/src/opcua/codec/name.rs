//! `QualifiedName` and `LocalizedText` (OPC UA Part 6, 5.2.2.13 and 5.2.2.14).

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE, UaString};

/// The bit of the `LocalizedText` mask that says a locale follows.
const LOCALE_FLAG: u8 = 0x01;
/// The bit of the `LocalizedText` mask that says a text follows.
const TEXT_FLAG: u8 = 0x02;

/// A name qualified by the index of the namespace that defines it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct QualifiedName {
    /// The namespace index.
    pub(in crate::opcua) namespace_index: u16,
    /// The name, which may be null.
    pub(in crate::opcua) name: UaString,
}

impl QualifiedName {
    /// Reads a `QualifiedName`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when the bytes are not all present, and the failures of the
    /// name String.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        Ok(Self {
            namespace_index: reader.read_u16(LITTLE)?,
            name: UaString::parse(reader, depth)?,
        })
    }

    /// Writes the `QualifiedName`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        writer.write_u16(self.namespace_index, LITTLE)?;
        self.name.serialize(writer, depth)
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] for a name over the String maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        self.name
            .length_in_bytes(depth)?
            .checked_add(2)
            .ok_or(ParseError::Overflow)
    }
}

/// Text with the locale it is written in.
///
/// Each part has a flag in the mask byte, so an absent part (`None`) is not the same as a
/// present null String (`Some` of a null `UaString`), and both are kept as read. The six
/// upper bits of the mask are reserved, read and ignored and written as zero.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct LocalizedText {
    /// The locale, such as `en-US`, when its flag is set.
    pub(in crate::opcua) locale: Option<UaString>,
    /// The text, when its flag is set.
    pub(in crate::opcua) text: Option<UaString>,
}

impl LocalizedText {
    /// Reads a `LocalizedText`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when the bytes are not all present, and the failures of the
    /// locale and text Strings.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let mask = reader.read_u8()?;
        let locale = if mask & LOCALE_FLAG == 0 {
            None
        } else {
            Some(UaString::parse(reader, depth)?)
        };
        let text = if mask & TEXT_FLAG == 0 {
            None
        } else {
            Some(UaString::parse(reader, depth)?)
        };
        Ok(Self { locale, text })
    }

    /// Writes the `LocalizedText`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let mut mask = 0;
        if self.locale.is_some() {
            mask |= LOCALE_FLAG;
        }
        if self.text.is_some() {
            mask |= TEXT_FLAG;
        }
        writer.write_u8(mask)?;
        if let Some(locale) = &self.locale {
            locale.serialize(writer, depth)?;
        }
        if let Some(text) = &self.text {
            text.serialize(writer, depth)?;
        }
        Ok(())
    }

    /// The encoded size in bytes.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] for a part over the String maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let mut size = 1_usize;
        for part in [&self.locale, &self.text].into_iter().flatten() {
            size = size
                .checked_add(part.length_in_bytes(depth)?)
                .ok_or(ParseError::Overflow)?;
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{assert_cut_off, assert_vector, decode, encode};

    #[test]
    fn opcua_codec_qualified_name_is_a_namespace_index_and_a_string() {
        let name = QualifiedName {
            namespace_index: 1,
            name: UaString::new("abc"),
        };
        assert_vector(
            &name,
            &[0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x61, 0x62, 0x63],
        );
        assert_cut_off(&name);
        let null = QualifiedName {
            namespace_index: 0,
            name: UaString::null(),
        };
        assert_vector(&null, &[0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(QualifiedName::default().name, UaString::null());
    }

    #[test]
    fn opcua_codec_qualified_name_length_in_bytes_refuses_an_oversized_name() {
        let name = QualifiedName {
            namespace_index: 0,
            name: UaString::new("a".repeat(crate::opcua::codec::MAX_STRING_BYTES + 1)),
        };
        assert!(matches!(
            name.length_in_bytes(Depth::new()),
            Err(ParseError::OversizedLength { .. })
        ));
    }

    #[test]
    fn opcua_codec_localized_text_mask_says_which_parts_follow() {
        let both = LocalizedText {
            locale: Some(UaString::new("en")),
            text: Some(UaString::new("hi")),
        };
        assert_vector(
            &both,
            &[
                0x03, 0x02, 0x00, 0x00, 0x00, 0x65, 0x6E, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69,
            ],
        );
        assert_cut_off(&both);
        let text = LocalizedText {
            locale: None,
            text: Some(UaString::new("hi")),
        };
        assert_vector(&text, &[0x02, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69]);
        let locale = LocalizedText {
            locale: Some(UaString::new("en")),
            text: None,
        };
        assert_vector(&locale, &[0x01, 0x02, 0x00, 0x00, 0x00, 0x65, 0x6E]);
        assert_vector(&LocalizedText::default(), &[0x00]);
    }

    #[test]
    fn opcua_codec_localized_text_absent_is_not_a_null_string() {
        let null = LocalizedText {
            locale: None,
            text: Some(UaString::null()),
        };
        assert_vector(&null, &[0x02, 0xFF, 0xFF, 0xFF, 0xFF]);
        assert_ne!(null, LocalizedText::default());
    }

    #[test]
    fn opcua_codec_localized_text_ignores_the_reserved_mask_bits() {
        let text = LocalizedText {
            locale: None,
            text: Some(UaString::new("hi")),
        };
        assert_eq!(
            decode::<LocalizedText>(&[0xC2, 0x02, 0x00, 0x00, 0x00, 0x68, 0x69]),
            Ok(text.clone())
        );
        assert_eq!(encode(&text).first(), Some(&0x02));
    }

    #[test]
    fn opcua_codec_localized_text_length_in_bytes_refuses_an_oversized_part() {
        let text = LocalizedText {
            locale: None,
            text: Some(UaString::new(
                "a".repeat(crate::opcua::codec::MAX_STRING_BYTES + 1),
            )),
        };
        assert!(matches!(
            text.length_in_bytes(Depth::new()),
            Err(ParseError::OversizedLength { .. })
        ));
    }
}
