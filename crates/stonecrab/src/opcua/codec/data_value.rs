//! `DataValue` (OPC UA Part 6, 5.2.2.17): a Variant with a status and timestamps, each present
//! only when its bit of the mask is set.
//!
//! The parts follow the mask in a fixed order: value, status code, source timestamp, source
//! picoseconds, server timestamp, server picoseconds. The two upper bits of the mask are
//! reserved, read and ignored and written as zero. The type spends one unit of the [`Depth`]
//! budget on entry, and its value another.

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{DateTime, Depth, LITTLE, StatusCode, Variant};

/// The bit of the mask that says a value follows.
const VALUE_FLAG: u8 = 0x01;
/// The bit of the mask that says a status code follows.
const STATUS_FLAG: u8 = 0x02;
/// The bit of the mask that says a source timestamp follows.
const SOURCE_TIMESTAMP_FLAG: u8 = 0x04;
/// The bit of the mask that says a server timestamp follows.
const SERVER_TIMESTAMP_FLAG: u8 = 0x08;
/// The bit of the mask that says the source picoseconds follow.
const SOURCE_PICOSECONDS_FLAG: u8 = 0x10;
/// The bit of the mask that says the server picoseconds follow.
const SERVER_PICOSECONDS_FLAG: u8 = 0x20;

/// A value with the status and the timestamps that go with it.
///
/// Each part is `None` when its flag is clear. The picoseconds are kept as they are read, in
/// units of 10 picoseconds, and are not checked against the 10,000 that make a tick.
#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::opcua) struct DataValue {
    /// The value.
    pub(in crate::opcua) value: Option<Variant>,
    /// The status code of the value.
    pub(in crate::opcua) status: Option<StatusCode>,
    /// The time the source produced the value.
    pub(in crate::opcua) source_timestamp: Option<DateTime>,
    /// The picoseconds to add to the source timestamp.
    pub(in crate::opcua) source_picoseconds: Option<u16>,
    /// The time the server received the value.
    pub(in crate::opcua) server_timestamp: Option<DateTime>,
    /// The picoseconds to add to the server timestamp.
    pub(in crate::opcua) server_picoseconds: Option<u16>,
}

impl DataValue {
    /// The mask that says which parts are present.
    fn mask(&self) -> u8 {
        [
            (self.value.is_some(), VALUE_FLAG),
            (self.status.is_some(), STATUS_FLAG),
            (self.source_timestamp.is_some(), SOURCE_TIMESTAMP_FLAG),
            (self.server_timestamp.is_some(), SERVER_TIMESTAMP_FLAG),
            (self.source_picoseconds.is_some(), SOURCE_PICOSECONDS_FLAG),
            (self.server_picoseconds.is_some(), SERVER_PICOSECONDS_FLAG),
        ]
        .into_iter()
        .filter(|&(present, _flag)| present)
        .fold(0, |mask, (_present, flag)| mask | flag)
    }

    /// Reads a `DataValue`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent;
    /// [`ParseError::Truncated`] when the bytes are not all present; and the failures of the
    /// value.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        depth: Depth,
    ) -> Result<Self, ParseError> {
        let depth = depth.enter()?;
        let mask = reader.read_u8()?;
        let value = if mask & VALUE_FLAG == 0 {
            None
        } else {
            Some(Variant::parse(reader, depth)?)
        };
        let status = if mask & STATUS_FLAG == 0 {
            None
        } else {
            Some(StatusCode::parse(reader, depth)?)
        };
        let source_timestamp = if mask & SOURCE_TIMESTAMP_FLAG == 0 {
            None
        } else {
            Some(DateTime::parse(reader, depth)?)
        };
        let source_picoseconds = if mask & SOURCE_PICOSECONDS_FLAG == 0 {
            None
        } else {
            Some(reader.read_u16(LITTLE)?)
        };
        let server_timestamp = if mask & SERVER_TIMESTAMP_FLAG == 0 {
            None
        } else {
            Some(DateTime::parse(reader, depth)?)
        };
        let server_picoseconds = if mask & SERVER_PICOSECONDS_FLAG == 0 {
            None
        } else {
            Some(reader.read_u16(LITTLE)?)
        };
        Ok(Self {
            value,
            status,
            source_timestamp,
            source_picoseconds,
            server_timestamp,
            server_picoseconds,
        })
    }

    /// Writes the `DataValue`, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent; and the failures of
    /// [`Variant::serialize`] and [`ParseError::OversizedLength`] over the writer's capacity.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        depth: Depth,
    ) -> Result<(), ParseError> {
        let depth = depth.enter()?;
        writer.write_u8(self.mask())?;
        if let Some(value) = &self.value {
            value.serialize(writer, depth)?;
        }
        if let Some(status) = &self.status {
            status.serialize(writer, depth)?;
        }
        if let Some(timestamp) = &self.source_timestamp {
            timestamp.serialize(writer, depth)?;
        }
        if let Some(picoseconds) = self.source_picoseconds {
            writer.write_u16(picoseconds, LITTLE)?;
        }
        if let Some(timestamp) = &self.server_timestamp {
            timestamp.serialize(writer, depth)?;
        }
        if let Some(picoseconds) = self.server_picoseconds {
            writer.write_u16(picoseconds, LITTLE)?;
        }
        Ok(())
    }

    /// The encoded size in bytes, spending one unit of the budget.
    ///
    /// # Errors
    /// [`ParseError::RecursionLimit`] when the budget is spent; the failures of
    /// [`Variant::length_in_bytes`]; [`ParseError::Overflow`] when the sum overflows.
    pub(in crate::opcua) fn length_in_bytes(&self, depth: Depth) -> Result<usize, ParseError> {
        let depth = depth.enter()?;
        let value = match &self.value {
            Some(value) => value.length_in_bytes(depth)?,
            None => 0,
        };
        // The mask, the status, the two timestamps and the two picoseconds, when present.
        let fixed = [
            (self.status.is_some(), 4_usize),
            (self.source_timestamp.is_some(), 8),
            (self.source_picoseconds.is_some(), 2),
            (self.server_timestamp.is_some(), 8),
            (self.server_picoseconds.is_some(), 2),
        ]
        .into_iter()
        .filter(|&(present, _size)| present)
        .try_fold(1_usize, |size, (_present, part)| size.checked_add(part))
        .ok_or(ParseError::Overflow)?;
        value.checked_add(fixed).ok_or(ParseError::Overflow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{
        assert_cut_off_counted, assert_spends_one_entry, assert_vector, budget, decode, encode,
    };
    use crate::opcua::codec::{Array, Scalar, VariantValue};

    fn boolean(value: bool) -> Variant {
        Variant::scalar(Scalar::Boolean(value))
    }

    #[test]
    fn opcua_codec_data_value_without_parts_is_one_byte() {
        assert_vector(&DataValue::default(), &[0x00]);
    }

    #[test]
    fn opcua_codec_data_value_each_flag_selects_its_part() {
        let ticks = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        let with_ticks = |mask: u8| {
            let mut bytes = vec![mask];
            bytes.extend(ticks);
            bytes
        };
        // Part 6, 5.2.2.17: bit 0 value, 1 status, 2 source timestamp, 3 server timestamp, 4
        // source picoseconds, 5 server picoseconds.
        assert_vector(
            &DataValue {
                value: Some(boolean(true)),
                ..DataValue::default()
            },
            &[0x01, 0x01, 0x01],
        );
        assert_vector(
            &DataValue {
                status: Some(StatusCode(0x8001_0000)),
                ..DataValue::default()
            },
            &[0x02, 0x00, 0x00, 0x01, 0x80],
        );
        assert_vector(
            &DataValue {
                source_timestamp: Some(DateTime(1)),
                ..DataValue::default()
            },
            &with_ticks(0x04),
        );
        assert_vector(
            &DataValue {
                server_timestamp: Some(DateTime(1)),
                ..DataValue::default()
            },
            &with_ticks(0x08),
        );
        assert_vector(
            &DataValue {
                source_picoseconds: Some(0x0102),
                ..DataValue::default()
            },
            &[0x10, 0x02, 0x01],
        );
        assert_vector(
            &DataValue {
                server_picoseconds: Some(0x0304),
                ..DataValue::default()
            },
            &[0x20, 0x04, 0x03],
        );
    }

    #[test]
    fn opcua_codec_data_value_parts_follow_the_mask_in_order() {
        let all = DataValue {
            value: Some(boolean(true)),
            status: Some(StatusCode(2)),
            source_timestamp: Some(DateTime(3)),
            source_picoseconds: Some(4),
            server_timestamp: Some(DateTime(5)),
            server_picoseconds: Some(6),
        };
        assert_vector(
            &all,
            &[
                0x3F, // the mask
                0x01, 0x01, // the value
                0x02, 0x00, 0x00, 0x00, // the status
                0x03, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // the source timestamp
                0x04, 0x00, // the source picoseconds
                0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // the server timestamp
                0x06, 0x00, // the server picoseconds
            ],
        );
        assert_cut_off_counted(&all);
    }

    #[test]
    fn opcua_codec_data_value_reserved_bits_are_ignored_and_written_as_zero() {
        assert_eq!(decode::<DataValue>(&[0xC0]), Ok(DataValue::default()));
        assert_eq!(
            decode::<DataValue>(&[0xC1, 0x01, 0x00]),
            Ok(DataValue {
                value: Some(boolean(false)),
                ..DataValue::default()
            })
        );
        assert_eq!(encode(&DataValue::default()), [0x00]);
    }

    #[test]
    fn opcua_codec_data_value_value_array_is_kept() {
        let value = DataValue {
            value: Some(Variant {
                value: VariantValue::Array(Array::Int32(Some(vec![7, 9]))),
                dimensions: None,
            }),
            status: Some(StatusCode(0)),
            ..DataValue::default()
        };
        assert_cut_off_counted(&value);
        assert_eq!(decode::<DataValue>(&encode(&value)).as_ref(), Ok(&value));
    }

    #[test]
    fn opcua_codec_data_value_value_failure_surfaces() {
        assert_eq!(
            decode::<DataValue>(&[0x01, 0x1A]),
            Err(ParseError::InvalidDiscriminator {
                field: "VariantType",
                value: 26
            })
        );
    }

    #[test]
    fn opcua_codec_data_value_spends_one_entry_and_its_value_another() {
        assert_spends_one_entry(&DataValue::default());
        let with_value = DataValue {
            value: Some(boolean(true)),
            ..DataValue::default()
        };
        let bytes = encode(&with_value);
        let limit = ParseError::RecursionLimit { limit: 64 };
        let one_left = budget(1);
        assert_eq!(
            DataValue::parse(&mut BoundedReader::new(&bytes), one_left),
            Err(limit.clone())
        );
        let mut writer = BoundedWriter::new(usize::MAX);
        assert_eq!(
            with_value.serialize(&mut writer, one_left),
            Err(limit.clone())
        );
        assert_eq!(with_value.length_in_bytes(one_left), Err(limit));
        let two_left = budget(2);
        assert_eq!(
            DataValue::parse(&mut BoundedReader::new(&bytes), two_left),
            Ok(with_value.clone())
        );
        assert_eq!(with_value.length_in_bytes(two_left), Ok(bytes.len()));
    }

    #[test]
    fn opcua_codec_data_value_nested_one_level_past_the_budget_is_recursion_limit() {
        // A DataValue holding a Variant holding a DataValue, and so on: two entries a pair.
        // 31 pairs, a DataValue and a Variant of nothing are the 64 entries of the budget.
        let mut bytes = [0x01, 0x17].repeat(31);
        bytes.extend([0x01, 0x00]);
        assert!(decode::<DataValue>(&bytes).is_ok(), "64 entries");
        // 32 pairs are 64 entries and the DataValue after them is the 65th.
        let mut bytes = [0x01, 0x17].repeat(32);
        bytes.push(0x00);
        let mut reader = BoundedReader::new(&bytes);
        assert_eq!(
            DataValue::parse(&mut reader, Depth::new()),
            Err(ParseError::RecursionLimit { limit: 64 })
        );
    }

    #[test]
    fn opcua_codec_data_value_is_refused_when_the_writer_is_full() {
        let value = DataValue {
            status: Some(StatusCode(0)),
            ..DataValue::default()
        };
        let mut writer = BoundedWriter::new(4);
        assert_eq!(
            value.serialize(&mut writer, Depth::new()),
            Err(ParseError::OversizedLength { length: 5, max: 4 })
        );
    }
}
