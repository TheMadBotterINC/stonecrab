//! `DateTime`: an `Int64` count of 100 ns ticks since 1601-01-01 00:00:00 UTC (OPC UA Part 6,
//! 5.2.2.5).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::codec::{BoundedReader, BoundedWriter, ParseError};
use crate::opcua::codec::{Depth, LITTLE};

/// The encoded size of a `DateTime`.
const SIZE: usize = 8;

/// Ticks in one second.
const TICKS_PER_SECOND: u128 = 10_000_000;

/// Nanoseconds in one tick.
const NANOS_PER_TICK: u128 = 100;

/// Ticks from 1601-01-01 to 1970-01-01: 11 644 473 600 seconds.
const UNIX_EPOCH_TICKS: i128 = 116_444_736_000_000_000;

/// A point in time as the wire carries it: ticks of 100 ns since 1601-01-01 UTC.
///
/// The codec keeps every `i64` as read, so 0 and `i64::MAX` (which OPC UA reads as "not
/// specified" and "the end of time") and negative values round-trip. Only the conversions to
/// and from [`SystemTime`] can fail, and they return `None` rather than overflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub(in crate::opcua) struct DateTime(pub(in crate::opcua) i64);

#[expect(
    clippy::trivially_copy_pass_by_ref,
    clippy::unused_self,
    reason = "the external-type signature takes `&self` and the budget, which a built-in ignores"
)]
impl DateTime {
    /// Reads a `DateTime`.
    ///
    /// # Errors
    /// [`ParseError::Truncated`] when fewer than 8 bytes remain.
    pub(in crate::opcua) fn parse(
        reader: &mut BoundedReader<'_>,
        _depth: Depth,
    ) -> Result<Self, ParseError> {
        reader.read_i64(LITTLE).map(Self)
    }

    /// Writes the `DateTime`.
    ///
    /// # Errors
    /// [`ParseError::OversizedLength`] when the writer's capacity is reached.
    pub(in crate::opcua) fn serialize(
        &self,
        writer: &mut BoundedWriter,
        _depth: Depth,
    ) -> Result<(), ParseError> {
        writer.write_i64(self.0, LITTLE)
    }

    /// The encoded size, always 8 bytes.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "the external-type signature returns a Result"
    )]
    pub(in crate::opcua) const fn length_in_bytes(
        &self,
        _depth: Depth,
    ) -> Result<usize, ParseError> {
        Ok(SIZE)
    }

    /// The same instant as a [`SystemTime`], or `None` when the platform's clock cannot hold
    /// it.
    pub(in crate::opcua) fn to_system_time(self) -> Option<SystemTime> {
        let since_unix = i128::from(self.0).checked_sub(UNIX_EPOCH_TICKS)?;
        let ticks = since_unix.unsigned_abs();
        let seconds = u64::try_from(ticks.checked_div(TICKS_PER_SECOND)?).ok()?;
        let nanos = ticks
            .checked_rem(TICKS_PER_SECOND)?
            .checked_mul(NANOS_PER_TICK)?;
        let offset = Duration::new(seconds, u32::try_from(nanos).ok()?);
        if since_unix < 0 {
            UNIX_EPOCH.checked_sub(offset)
        } else {
            UNIX_EPOCH.checked_add(offset)
        }
    }

    /// The instant of a [`SystemTime`], truncated to a whole tick towards 1970, or `None` when
    /// it does not fit an `i64` of ticks.
    pub(in crate::opcua) fn from_system_time(time: SystemTime) -> Option<Self> {
        let (offset, negative) = match time.duration_since(UNIX_EPOCH) {
            Ok(after) => (after, false),
            Err(error) => (error.duration(), true),
        };
        let ticks = i128::try_from(offset.as_nanos().checked_div(NANOS_PER_TICK)?).ok()?;
        let since_unix = if negative {
            ticks.checked_neg()?
        } else {
            ticks
        };
        i64::try_from(since_unix.checked_add(UNIX_EPOCH_TICKS)?)
            .ok()
            .map(Self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::opcua::codec::support::{assert_cut_off, assert_vector};

    /// 1970-01-01 in ticks, as an `i64`.
    const EPOCH: i64 = 116_444_736_000_000_000;

    #[test]
    fn opcua_codec_date_time_is_a_little_endian_i64_of_ticks() {
        assert_vector(&DateTime(1), &[1, 0, 0, 0, 0, 0, 0, 0]);
        // 1970-01-01 is 116 444 736 000 000 000 ticks after 1601-01-01.
        assert_vector(
            &DateTime(EPOCH),
            &[0x00, 0x80, 0x3E, 0xD5, 0xDE, 0xB1, 0x9D, 0x01],
        );
        assert_cut_off(&DateTime(5));
        assert_eq!(DateTime::default(), DateTime(0));
    }

    // covers: SPEC mandatory-failure-cases / overflow at the limits of a numeric field
    #[test]
    fn opcua_codec_date_time_zero_and_max_do_not_overflow() {
        assert_vector(&DateTime(0), &[0; 8]);
        assert_vector(
            &DateTime(i64::MAX),
            &[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x7F],
        );
        assert_vector(&DateTime(i64::MIN), &[0, 0, 0, 0, 0, 0, 0, 0x80]);
        // The conversions answer or give up, whatever the platform's clock holds; they never
        // panic, and an answer converts back to the same tick count.
        let answers = [0, 1, -1, i64::MAX, i64::MIN, EPOCH]
            .into_iter()
            .filter_map(|ticks| Some((ticks, DateTime(ticks).to_system_time()?)));
        for (ticks, time) in answers {
            assert_eq!(
                DateTime::from_system_time(time),
                Some(DateTime(ticks)),
                "{ticks}"
            );
        }
        assert_eq!(
            DateTime(0).to_system_time(),
            UNIX_EPOCH.checked_sub(Duration::new(11_644_473_600, 0)),
            "tick 0 is 1601-01-01"
        );
    }

    #[test]
    fn opcua_codec_date_time_converts_to_and_from_system_time() {
        assert_eq!(DateTime(EPOCH).to_system_time(), Some(UNIX_EPOCH));
        assert_eq!(
            DateTime::from_system_time(UNIX_EPOCH),
            Some(DateTime(EPOCH))
        );
        // One tick is 100 ns, on either side of 1970.
        assert_eq!(
            DateTime(EPOCH + 1).to_system_time(),
            Some(UNIX_EPOCH + Duration::from_nanos(100))
        );
        assert_eq!(
            DateTime(EPOCH - 1).to_system_time(),
            UNIX_EPOCH.checked_sub(Duration::from_nanos(100))
        );
        // 2000-01-01 00:00:00 UTC is 946 684 800 seconds after 1970.
        let millennium = UNIX_EPOCH + Duration::new(946_684_800, 0);
        assert_eq!(
            DateTime::from_system_time(millennium),
            Some(DateTime(EPOCH + 9_466_848_000_000_000))
        );
        // A finer instant is cut to a whole tick towards 1970.
        assert_eq!(
            DateTime::from_system_time(UNIX_EPOCH + Duration::from_nanos(199)),
            Some(DateTime(EPOCH + 1))
        );
        assert_eq!(
            DateTime::from_system_time(UNIX_EPOCH - Duration::from_nanos(199)),
            Some(DateTime(EPOCH - 1))
        );
        // Before 1601 there are negative ticks, on a platform whose clock reaches that far
        // (Windows counts from 1601 and cannot).
        if let Some(early) = UNIX_EPOCH.checked_sub(Duration::new(12_000_000_000, 0)) {
            let before = DateTime::from_system_time(early);
            assert_eq!(before, Some(DateTime(EPOCH - 120_000_000_000_000_000)));
            assert_eq!(before.and_then(DateTime::to_system_time), Some(early));
        }
    }

    #[test]
    fn opcua_codec_date_time_gives_up_on_an_instant_beyond_the_tick_range() {
        // About 31 700 years after 1970: more ticks than an `i64` holds, if the platform's
        // clock can hold the instant at all.
        let offset = Duration::new(1_000_000_000_000, 0);
        for far in [
            UNIX_EPOCH.checked_add(offset),
            UNIX_EPOCH.checked_sub(offset),
        ]
        .into_iter()
        .flatten()
        {
            assert_eq!(DateTime::from_system_time(far), None);
        }
    }
}
