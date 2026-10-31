//! `DiagnosticInfo` (OPC UA Part 6, 5.2.2.12): optional indices into the string table of the
//! response, free text, a status code, and a nested `DiagnosticInfo` for the inner cause.
//!
//! The nesting is why the type spends one unit of the [`Depth`] budget on entry: the inner
//! `DiagnosticInfo` is the only place a server decides how deep a value goes, and the budget
//! cuts a chain at the limit. The bit 7 of the mask is reserved, read and ignored and written
//! as zero.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE, StatusCode, UaString};

/// The bit of the mask that says a symbolic id follows.
const SYMBOLIC_ID_FLAG: u8 = 0x01;
/// The bit of the mask that says a namespace URI index follows.
const NAMESPACE_URI_FLAG: u8 = 0x02;
/// The bit of the mask that says a localized text index follows.
const LOCALIZED_TEXT_FLAG: u8 = 0x04;
/// The bit of the mask that says a locale index follows.
const LOCALE_FLAG: u8 = 0x08;
/// The bit of the mask that says additional information follows.
const ADDITIONAL_INFO_FLAG: u8 = 0x10;
/// The bit of the mask that says an inner status code follows.
const INNER_STATUS_FLAG: u8 = 0x20;
/// The bit of the mask that says an inner `DiagnosticInfo` follows.
const INNER_INFO_FLAG: u8 = 0x40;

/// Diagnostic information about a status code.
///
/// Each part is `None` when its flag is clear. The four indices are `Int32`s into the string
/// table of the response and are kept as read, negative values included: what they mean is for
/// the layer that has the table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(in crate::opcua) struct DiagnosticInfo {
    /// The index of the symbolic id in the string table.
    pub(in crate::opcua) symbolic_id: Option<i32>,
    /// The index of the namespace URI in the string table.
    pub(in crate::opcua) namespace_uri: Option<i32>,
    /// The index of the locale in the string table.
    pub(in crate::opcua) locale: Option<i32>,
    /// The index of the localized text in the string table.
    pub(in crate::opcua) localized_text: Option<i32>,
    /// Vendor-specific diagnostic text, which may be a null String.
    pub(in crate::opcua) additional_info: Option<UaString>,
    /// The status code of the inner cause.
    pub(in crate::opcua) inner_status: Option<StatusCode>,
    /// The diagnostic information of the inner cause.
    pub(in crate::opcua) inner: Option<Box<Self>>,
}

impl DiagnosticInfo {
    /// The mask that says which parts are present.
    fn mask(&self) -> u8 {
        [
            (self.symbolic_id.is_some(), SYMBOLIC_ID_FLAG),
            (self.namespace_uri.is_some(), NAMESPACE_URI_FLAG),
            (self.localized_text.is_some(), LOCALIZED_TEXT_FLAG),
            (self.locale.is_some(), LOCALE_FLAG),
            (self.additional_info.is_some(), ADDITIONAL_INFO_FLAG),
            (self.inner_status.is_some(), INNER_STATUS_FLAG),
            (self.inner.is_some(), INNER_INFO_FLAG),
        ]
        .into_iter()
        .filter(|&(present, _flag)| present)
        .fold(0, |mask, (_present, flag)| mask | flag)
    }

    /// Reads a `DiagnosticInfo`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::Truncated`] when the bytes are not all present; and the failures of the
    /// additional information String.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let depth = depth.enter()?;
        let mask = reader.read_u8()?;
        let mut index = |flag: u8| -> Result<Option<i32>, ParseError> {
            if mask & flag == 0 {
                Ok(None)
            } else {
                reader.read_i32(LITTLE).map(Some)
            }
        };
        let symbolic_id = index(SYMBOLIC_ID_FLAG)?;
        let namespace_uri = index(NAMESPACE_URI_FLAG)?;
        let locale = index(LOCALE_FLAG)?;
        let localized_text = index(LOCALIZED_TEXT_FLAG)?;
        let additional_info = if mask & ADDITIONAL_INFO_FLAG == 0 {
            None
        } else {
            Some(UaString::parse(reader, depth)?)
        };
        let inner_status = if mask & INNER_STATUS_FLAG == 0 {
            None
        } else {
            Some(StatusCode::parse(reader, depth)?)
        };
        let inner = if mask & INNER_INFO_FLAG == 0 {
            None
        } else {
            Some(Box::new(Self::parse(reader, depth)?))
        };
        Ok(Self {
            symbolic_id,
            namespace_uri,
            locale,
            localized_text,
            additional_info,
            inner_status,
            inner,
        })
    }

    /// Writes the `DiagnosticInfo`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::OversizedLength`] over the writer's capacity or the String maximum.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let depth = depth.enter()?;
        writer.write_u8(self.mask())?;
        for index in [
            self.symbolic_id,
            self.namespace_uri,
            self.locale,
            self.localized_text,
        ]
        .into_iter()
        .flatten()
        {
            writer.write_i32(index, LITTLE)?;
        }
        if let Some(info) = &self.additional_info {
            info.serialize(writer, depth)?;
        }
        if let Some(status) = &self.inner_status {
            status.serialize(writer, depth)?;
        }
        if let Some(inner) = &self.inner {
            inner.serialize(writer, depth)?;
        }
        Ok(())
    }

    /// The encoded size in bytes, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::OversizedLength`] for a String over the maximum;
    /// [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let depth = depth.enter()?;
        let indices = [
            self.symbolic_id,
            self.namespace_uri,
            self.locale,
            self.localized_text,
        ]
        .into_iter()
        .flatten()
        .count();
        let mut size = indices
            .checked_mul(4)
            .and_then(|size| size.checked_add(1))
            .ok_or(ParseError::Overflow)?;
        if let Some(info) = &self.additional_info {
            size = size
                .checked_add(info.length_in_bytes(depth)?)
                .ok_or(ParseError::Overflow)?;
        }
        if self.inner_status.is_some() {
            size = size.checked_add(4).ok_or(ParseError::Overflow)?;
        }
        if let Some(inner) = &self.inner {
            size = size
                .checked_add(inner.length_in_bytes(depth)?)
                .ok_or(ParseError::Overflow)?;
        }
        Ok(size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{
        assert_cut_off, assert_spends_one_entry, assert_vector, budget, decode, encode,
    };

    fn info(configure: impl FnOnce(&mut DiagnosticInfo)) -> DiagnosticInfo {
        let mut info = DiagnosticInfo::default();
        configure(&mut info);
        info
    }

    /// `levels` diagnostic infos that each hold one more as their inner cause.
    fn nested(levels: usize) -> Vec<u8> {
        let mut bytes = vec![0x40; levels];
        bytes.push(0x00);
        bytes
    }

    #[test]
    fn opcua_codec_diagnostic_info_without_parts_is_one_byte() {
        assert_vector(&DiagnosticInfo::default(), &[0x00]);
    }

    #[test]
    fn opcua_codec_diagnostic_info_each_flag_selects_its_part() {
        // Part 6, 5.2.2.12: bit 0 symbolic id, 1 namespace URI, 2 localized text, 3 locale, 4
        // additional info, 5 inner status code, 6 inner diagnostic info.
        let index = [0x2A, 0x00, 0x00, 0x00];
        let with_index = |mask: u8| {
            let mut bytes = vec![mask];
            bytes.extend(index);
            bytes
        };
        assert_vector(&info(|i| i.symbolic_id = Some(42)), &with_index(0x01));
        assert_vector(&info(|i| i.namespace_uri = Some(42)), &with_index(0x02));
        assert_vector(&info(|i| i.localized_text = Some(42)), &with_index(0x04));
        assert_vector(&info(|i| i.locale = Some(42)), &with_index(0x08));
        assert_vector(
            &info(|i| i.additional_info = Some(UaString::new("x"))),
            &[0x10, 0x01, 0x00, 0x00, 0x00, 0x78],
        );
        assert_vector(
            &info(|i| i.inner_status = Some(StatusCode(0x8001_0000))),
            &[0x20, 0x00, 0x00, 0x01, 0x80],
        );
        assert_vector(&info(|i| i.inner = Some(Box::default())), &[0x40, 0x00]);
    }

    #[test]
    fn opcua_codec_diagnostic_info_parts_follow_the_mask_in_order() {
        let all = DiagnosticInfo {
            symbolic_id: Some(1),
            namespace_uri: Some(2),
            locale: Some(3),
            localized_text: Some(4),
            additional_info: Some(UaString::null()),
            inner_status: Some(StatusCode(5)),
            inner: Some(Box::new(info(|i| i.symbolic_id = Some(6)))),
        };
        assert_vector(
            &all,
            &[
                0x7F, // the mask
                0x01, 0x00, 0x00, 0x00, // the symbolic id
                0x02, 0x00, 0x00, 0x00, // the namespace URI
                0x03, 0x00, 0x00, 0x00, // the locale
                0x04, 0x00, 0x00, 0x00, // the localized text
                0xFF, 0xFF, 0xFF, 0xFF, // the additional info, null
                0x05, 0x00, 0x00, 0x00, // the inner status code
                0x01, 0x06, 0x00, 0x00, 0x00, // the inner diagnostic info
            ],
        );
        assert_cut_off(&all);
    }

    #[test]
    fn opcua_codec_diagnostic_info_indices_are_kept_as_read() {
        let negative = info(|i| i.symbolic_id = Some(-1));
        assert_vector(&negative, &[0x01, 0xFF, 0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn opcua_codec_diagnostic_info_reserved_bit_is_ignored_and_written_as_zero() {
        assert_eq!(
            decode::<DiagnosticInfo>(&[0x80]),
            Ok(DiagnosticInfo::default())
        );
        assert_eq!(encode(&DiagnosticInfo::default()), [0x00]);
    }

    #[test]
    fn opcua_codec_diagnostic_info_nesting_beyond_the_limit_is_recursion_limit() {
        let limit = ParseError::RecursionLimit { limit: 64 };
        for levels in [64, 65, 3000, 16_000] {
            assert_eq!(
                decode::<DiagnosticInfo>(&nested(levels)),
                Err(limit.clone()),
                "{levels} nested causes"
            );
        }
    }

    #[test]
    fn opcua_codec_diagnostic_info_nesting_at_the_limit_is_read() {
        // The outer info and 63 causes are 64 entries.
        let bytes = nested(63);
        let parsed = decode::<DiagnosticInfo>(&bytes).expect("64 entries");
        let mut depth = 1;
        let mut cursor = &parsed;
        while let Some(inner) = &cursor.inner {
            depth += 1;
            cursor = inner;
        }
        assert_eq!(depth, 64);
        assert_eq!(encode(&parsed), bytes);
        assert_eq!(parsed.length_in_bytes(Depth::new()), Ok(bytes.len()));
        // A value nested one past the budget is neither written nor measured.
        let too_deep = info(|i| i.inner = Some(Box::new(parsed)));
        let limit = ParseError::RecursionLimit { limit: 64 };
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(
            too_deep.serialize(&mut writer, Depth::new()),
            Err(limit.clone())
        );
        assert_eq!(too_deep.length_in_bytes(Depth::new()), Err(limit));
    }

    #[test]
    fn opcua_codec_diagnostic_info_spends_one_entry_for_each_level() {
        assert_spends_one_entry(&DiagnosticInfo::default());
        let nested = info(|i| i.inner = Some(Box::default()));
        let bytes = encode(&nested);
        assert_eq!(
            DiagnosticInfo::parse(&mut BoundedReader::new(&bytes), budget(1)),
            Err(ParseError::RecursionLimit { limit: 64 })
        );
        assert_eq!(
            DiagnosticInfo::parse(&mut BoundedReader::new(&bytes), budget(2)),
            Ok(nested)
        );
    }

    #[test]
    fn opcua_codec_diagnostic_info_is_refused_when_the_writer_is_full() {
        let long = info(|i| i.additional_info = Some(UaString::from_bytes(vec![0; 8])));
        let mut writer = BoundedWriter::new(8);
        assert_eq!(
            long.serialize(&mut writer, Depth::new()),
            Err(ParseError::OversizedLength { length: 13, max: 8 })
        );
    }
}
