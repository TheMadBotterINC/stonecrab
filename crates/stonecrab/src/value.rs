use std::fmt;
use std::time::Duration;

use crate::{ErrorKind, PlcError, Result};

/// The exact scalar representation of a [`PlcValue`].
///
/// Byte-buffer and recursive collection representations are deferred; they will not be
/// silently substituted with strings or floating-point numbers. Calendar values have the
/// four representations of ADR-0021.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum PlcValueType {
    /// Explicitly absent value, not a failed acquisition.
    Null,
    /// Boolean state.
    Bool,
    /// Signed 8-bit integer.
    I8,
    /// Signed 16-bit integer.
    I16,
    /// Signed 32-bit integer.
    I32,
    /// Signed 64-bit integer.
    I64,
    /// Unsigned 8-bit integer.
    U8,
    /// Unsigned 16-bit integer.
    U16,
    /// Unsigned 32-bit integer.
    U32,
    /// Unsigned 64-bit integer.
    U64,
    /// IEEE 754 single precision.
    F32,
    /// IEEE 754 double precision.
    F64,
    /// Unicode scalar value, not a raw encoded byte.
    Char,
    /// Already-decoded UTF-8 text.
    String,
    /// Nonnegative elapsed duration, not a calendar timestamp.
    Duration,
    /// Signed elapsed time in nanoseconds: IEC 61131-3 `TIME` and `LTIME`.
    SignedDuration,
    /// Calendar date without a time zone: `DATE` and `LDATE`.
    Date,
    /// Time of day without a time zone: `TIME_OF_DAY` and `LTIME_OF_DAY`.
    TimeOfDay,
    /// Date and time of day without a time zone: `DATE_AND_TIME`, `DATE_AND_LTIME` and
    /// `LDATE_AND_TIME`.
    DateTime,
}

/// Exact-width scalar data, independent of a protocol, sensor or runtime.
///
/// Float bit patterns, including non-finite values, are retained for diagnosis.
/// Sensor quality validation must reject non-finite scaled results at its boundary.
/// Strings here are caller-owned data, not an unbounded wire decoding API.
///
/// # Examples
/// ```
/// use stonecrab::{PlcValue, PlcValueType};
/// let counter = PlcValue::U64(u64::MAX);
/// assert_eq!(counter.value_type(), PlcValueType::U64);
/// assert_eq!(counter.as_u64(), Some(u64::MAX));
/// assert_eq!(counter.as_i64(), None);
/// assert_eq!(counter.as_f64(), None);
/// ```
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum PlcValue {
    /// Explicit absence; never a substitute for an acquisition failure.
    Null,
    /// Boolean state.
    Bool(bool),
    /// Signed 8-bit integer.
    I8(i8),
    /// Signed 16-bit integer.
    I16(i16),
    /// Signed 32-bit integer.
    I32(i32),
    /// Signed 64-bit integer.
    I64(i64),
    /// Unsigned 8-bit integer.
    U8(u8),
    /// Unsigned 16-bit integer.
    U16(u16),
    /// Unsigned 32-bit integer.
    U32(u32),
    /// Unsigned 64-bit integer, never implicitly converted to float.
    U64(u64),
    /// Single precision, preserving non-finite values.
    F32(f32),
    /// Double precision, preserving non-finite values.
    F64(f64),
    /// A decoded Unicode scalar.
    Char(char),
    /// Already-decoded text.
    String(String),
    /// Nonnegative elapsed duration.
    Duration(Duration),
    /// Signed elapsed time: IEC 61131-3 `TIME` and `LTIME` (ADR-0021).
    SignedDuration(PlcSignedDuration),
    /// Calendar date without a time zone: `DATE` and `LDATE` (ADR-0021).
    Date(PlcDate),
    /// Time of day without a time zone: `TIME_OF_DAY` and `LTIME_OF_DAY` (ADR-0021).
    TimeOfDay(PlcTimeOfDay),
    /// Date and time of day without a time zone: `DATE_AND_TIME`, `DATE_AND_LTIME` and
    /// `LDATE_AND_TIME` (ADR-0021).
    DateTime(PlcDateTime),
}

impl PlcValue {
    /// Return the stored representation, not an inferred or widened type.
    #[must_use]
    pub const fn value_type(&self) -> PlcValueType {
        match self {
            Self::Null => PlcValueType::Null,
            Self::Bool(_) => PlcValueType::Bool,
            Self::I8(_) => PlcValueType::I8,
            Self::I16(_) => PlcValueType::I16,
            Self::I32(_) => PlcValueType::I32,
            Self::I64(_) => PlcValueType::I64,
            Self::U8(_) => PlcValueType::U8,
            Self::U16(_) => PlcValueType::U16,
            Self::U32(_) => PlcValueType::U32,
            Self::U64(_) => PlcValueType::U64,
            Self::F32(_) => PlcValueType::F32,
            Self::F64(_) => PlcValueType::F64,
            Self::Char(_) => PlcValueType::Char,
            Self::String(_) => PlcValueType::String,
            Self::Duration(_) => PlcValueType::Duration,
            Self::SignedDuration(_) => PlcValueType::SignedDuration,
            Self::Date(_) => PlcValueType::Date,
            Self::TimeOfDay(_) => PlcValueType::TimeOfDay,
            Self::DateTime(_) => PlcValueType::DateTime,
        }
    }

    /// Return only a Boolean; numeric zero/one and text are not coerced.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        if let Self::Bool(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    /// Losslessly widen an integer to signed 64-bit, or return `None`.
    ///
    /// Unsigned values above [`i64::MAX`], floats, text and Booleans are rejected.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I8(value) => Some(i64::from(*value)),
            Self::I16(value) => Some(i64::from(*value)),
            Self::I32(value) => Some(i64::from(*value)),
            Self::I64(value) => Some(*value),
            Self::U8(value) => Some(i64::from(*value)),
            Self::U16(value) => Some(i64::from(*value)),
            Self::U32(value) => Some(i64::from(*value)),
            Self::U64(value) => i64::try_from(*value).ok(),
            Self::Null
            | Self::Bool(_)
            | Self::F32(_)
            | Self::F64(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::Duration(_)
            | Self::SignedDuration(_)
            | Self::Date(_)
            | Self::TimeOfDay(_)
            | Self::DateTime(_) => None,
        }
    }

    /// Losslessly widen an integer to unsigned 64-bit, or return `None`.
    ///
    /// Negative integers, floats, text and Booleans are rejected.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::I8(value) => u64::try_from(*value).ok(),
            Self::I16(value) => u64::try_from(*value).ok(),
            Self::I32(value) => u64::try_from(*value).ok(),
            Self::I64(value) => u64::try_from(*value).ok(),
            Self::U8(value) => Some(u64::from(*value)),
            Self::U16(value) => Some(u64::from(*value)),
            Self::U32(value) => Some(u64::from(*value)),
            Self::U64(value) => Some(*value),
            Self::Null
            | Self::Bool(_)
            | Self::F32(_)
            | Self::F64(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::Duration(_)
            | Self::SignedDuration(_)
            | Self::Date(_)
            | Self::TimeOfDay(_)
            | Self::DateTime(_) => None,
        }
    }

    /// Widen only stored floating-point values, preserving IEEE special values.
    ///
    /// Even small integers are not coerced; callers must explicitly choose any
    /// integer-to-float scaling policy at the sensor boundary.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::F32(value) => Some(f64::from(*value)),
            Self::F64(value) => Some(*value),
            Self::Null
            | Self::Bool(_)
            | Self::I8(_)
            | Self::I16(_)
            | Self::I32(_)
            | Self::I64(_)
            | Self::U8(_)
            | Self::U16(_)
            | Self::U32(_)
            | Self::U64(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::Duration(_)
            | Self::SignedDuration(_)
            | Self::Date(_)
            | Self::TimeOfDay(_)
            | Self::DateTime(_) => None,
        }
    }
}

const NANOS_PER_MILLI: i64 = 1_000_000;
const NANOS_PER_SECOND: u64 = 1_000_000_000;
/// Days from 0000-03-01 to 1970-01-01: the civil algorithms count years from March.
const DAYS_TO_EPOCH: i64 = 719_468;
/// Days in a 400-year Gregorian era.
const DAYS_PER_ERA: i64 = 146_097;

/// A calendar construction outside the representable range or calendar.
fn out_of_range(message: &'static str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}

/// Signed elapsed time in whole nanoseconds: the IEC 61131-3 `TIME` and `LTIME` values.
///
/// The range is that of an `i64` nanosecond count, about 292 years either way:
/// `PT-2562047H-47M-16.854775808S` to `PT2562047H47M16.854775807S`. A 32-bit
/// millisecond `TIME` and a 64-bit nanosecond `LTIME` both fit exactly. It is distinct from
/// [`PlcValue::Duration`], which cannot be negative. `Display` renders the ISO 8601 text of
/// `java.time.Duration::toString`, where every nonzero part carries the sign.
///
/// # Examples
/// ```
/// use stonecrab::PlcSignedDuration;
/// let span = PlcSignedDuration::from_millis(-1_500)?;
/// assert_eq!(span.as_nanos(), -1_500_000_000);
/// assert_eq!(span.to_string(), "PT-1.5S");
/// assert_eq!(PlcSignedDuration::from_nanos(5_400_000_000_000).to_string(), "PT1H30M");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-VALUE-CALENDAR
/// Upstream `PlcTIME` and `PlcLTIME` hold a `java.time.Duration`; a duration beyond the
/// `i64` nanosecond range has no Stonecrab value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct PlcSignedDuration(i64);

impl PlcSignedDuration {
    /// A duration of `nanos` nanoseconds; every `i64` is in range.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcSignedDuration;
    /// assert_eq!(PlcSignedDuration::from_nanos(i64::MIN).as_nanos(), i64::MIN);
    /// ```
    #[must_use]
    pub const fn from_nanos(nanos: i64) -> Self {
        Self(nanos)
    }

    /// A duration of `millis` milliseconds, the unit of `TIME` on most PLCs.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcSignedDuration};
    /// assert_eq!(PlcSignedDuration::from_millis(i64::from(i32::MIN))?.to_string(), "PT-596H-31M-23.648S");
    /// let refused = PlcSignedDuration::from_millis(i64::MAX);
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when the duration is beyond the `i64` nanosecond
    /// range, that is beyond about 292 years either way.
    pub fn from_millis(millis: i64) -> Result<Self> {
        millis
            .checked_mul(NANOS_PER_MILLI)
            .map(Self)
            .ok_or_else(|| out_of_range("a signed duration is limited to the i64 nanosecond range"))
    }

    /// The duration as a signed count of nanoseconds.
    #[must_use]
    pub const fn as_nanos(self) -> i64 {
        self.0
    }
}

impl fmt::Display for PlcSignedDuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { "-" } else { "" };
        let magnitude = self.0.unsigned_abs();
        let whole = magnitude.div_euclid(NANOS_PER_SECOND);
        let fraction = magnitude.rem_euclid(NANOS_PER_SECOND);
        let (hours, minutes, seconds) = (
            whole.div_euclid(3_600),
            whole.div_euclid(60).rem_euclid(60),
            whole.rem_euclid(60),
        );
        f.write_str("PT")?;
        if hours != 0 {
            write!(f, "{sign}{hours}H")?;
        }
        if minutes != 0 {
            write!(f, "{sign}{minutes}M")?;
        }
        if seconds == 0 && fraction == 0 && (hours != 0 || minutes != 0) {
            return Ok(());
        }
        write!(f, "{sign}{seconds}")?;
        if fraction != 0 {
            // The fraction without its trailing zeros, as java.time prints it.
            let (mut digits, mut width) = (fraction, 9_usize);
            while digits.rem_euclid(10) == 0 {
                digits = digits.div_euclid(10);
                width = width.saturating_sub(1);
            }
            write!(f, ".{digits:0width$}")?;
        }
        f.write_str("S")
    }
}

/// A proleptic Gregorian calendar date without a time zone: the IEC 61131-3 `DATE` and
/// `LDATE` values.
///
/// Stored as a signed count of days since 1970-01-01, so the range is that of an `i32`:
/// `-5877641-06-23` to `+5881580-07-11`. Ordering is chronological. `Display` renders
/// the ISO 8601 text of `java.time.LocalDate::toString`: `YYYY-MM-DD`, a year beyond 9999
/// with a `+` and a year before 0000 with a `-`.
///
/// # Examples
/// ```
/// use stonecrab::PlcDate;
/// let date = PlcDate::from_ymd(2024, 2, 29)?;
/// assert_eq!((date.year(), date.month(), date.day()), (2024, 2, 29));
/// assert_eq!(date.days_since_epoch(), 19_782);
/// assert_eq!(date.to_string(), "2024-02-29");
/// assert_eq!(PlcDate::from_days_since_epoch(-1).to_string(), "1969-12-31");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-VALUE-CALENDAR
/// Upstream `PlcDATE` and `PlcLDATE` hold a `java.time.LocalDate`, whose years reach
/// +-999,999,999; a date beyond the `i32` day range has no Stonecrab value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct PlcDate(i32);

impl PlcDate {
    /// The date `days` days after 1970-01-01 (before it when negative); every `i32` is in
    /// range.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcDate;
    /// assert_eq!(PlcDate::from_days_since_epoch(0).to_string(), "1970-01-01");
    /// ```
    #[must_use]
    pub const fn from_days_since_epoch(days: i32) -> Self {
        Self(days)
    }

    /// The date with this year, month (1 to 12) and day of the month (1 to 31).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcDate};
    /// assert_eq!(PlcDate::from_ymd(-1, 1, 1)?.to_string(), "-0001-01-01");
    /// let refused = PlcDate::from_ymd(1900, 2, 29);
    /// assert_eq!(refused.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for a month outside 1 to 12, a day that the month
    /// does not have (February 29 of a common year included) or a date outside the `i32`
    /// day range.
    pub fn from_ymd(year: i32, month: u8, day: u8) -> Result<Self> {
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(out_of_range("the month or day is not in the calendar"));
        }
        i32::try_from(days_from_civil(year, month, day))
            .map(Self)
            .map_err(|source| {
                out_of_range("a date is limited to the i32 range of days since 1970-01-01")
                    .with_source(source)
            })
    }

    /// The signed count of days since 1970-01-01.
    #[must_use]
    pub const fn days_since_epoch(self) -> i32 {
        self.0
    }

    /// The proleptic Gregorian year: 0 is 1 BC and -1 is 2 BC.
    #[must_use]
    pub fn year(self) -> i32 {
        civil_from_days(self.0).0
    }

    /// The month, 1 to 12.
    #[must_use]
    pub fn month(self) -> u8 {
        civil_from_days(self.0).1
    }

    /// The day of the month, 1 to 31.
    #[must_use]
    pub fn day(self) -> u8 {
        civil_from_days(self.0).2
    }
}

impl fmt::Display for PlcDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = civil_from_days(self.0);
        let sign = match year {
            10_000.. => "+",
            ..0 => "-",
            _ => "",
        };
        write!(f, "{sign}{:04}-{month:02}-{day:02}", year.unsigned_abs())
    }
}

/// A time of day without a time zone, to the nanosecond: the IEC 61131-3 `TIME_OF_DAY`
/// and `LTIME_OF_DAY` values.
///
/// The range is `00:00` to `23:59:59.999999999`. There is no leap second (no second 60)
/// and no `24:00`, as in `java.time.LocalTime`. Ordering is chronological. `Display`
/// renders the ISO 8601 text of `LocalTime::toString`: `HH:MM`, with `:SS` when the
/// seconds or the fraction are nonzero and a fraction of 3, 6 or 9 digits when nonzero.
///
/// # Examples
/// ```
/// use stonecrab::PlcTimeOfDay;
/// let time = PlcTimeOfDay::from_hms_nano(13, 5, 0, 0)?;
/// assert_eq!(time.to_string(), "13:05");
/// assert_eq!(time.nanos_since_midnight(), 47_100_000_000_000);
/// let precise = PlcTimeOfDay::from_nanos_since_midnight(47_107_000_001_000)?;
/// assert_eq!(precise.to_string(), "13:05:07.000001");
/// assert_eq!((precise.hour(), precise.minute(), precise.second()), (13, 5, 7));
/// assert_eq!(precise.nanosecond(), 1_000);
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-VALUE-CALENDAR
/// Upstream `PlcTIME_OF_DAY` and `PlcLTIME_OF_DAY` hold a `java.time.LocalTime`, which
/// has the same range and resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct PlcTimeOfDay {
    hour: u8,
    minute: u8,
    second: u8,
    nanosecond: u32,
}

impl PlcTimeOfDay {
    /// The time of day from its hour (0 to 23), minute (0 to 59), second (0 to 59) and
    /// nanosecond (0 to 999,999,999).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcTimeOfDay};
    /// assert_eq!(PlcTimeOfDay::from_hms_nano(23, 59, 59, 999_999_999)?.to_string(), "23:59:59.999999999");
    /// let leap_second = PlcTimeOfDay::from_hms_nano(23, 59, 60, 0);
    /// assert_eq!(leap_second.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when a field is beyond its range.
    pub fn from_hms_nano(hour: u8, minute: u8, second: u8, nanosecond: u32) -> Result<Self> {
        if hour < 24 && minute < 60 && second < 60 && u64::from(nanosecond) < NANOS_PER_SECOND {
            Ok(Self {
                hour,
                minute,
                second,
                nanosecond,
            })
        } else {
            Err(out_of_range(
                "a time of day is 00:00 to 23:59:59.999999999, without a leap second",
            ))
        }
    }

    /// The time of day `nanos` nanoseconds after midnight.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, PlcTimeOfDay};
    /// assert_eq!(PlcTimeOfDay::from_nanos_since_midnight(0)?.to_string(), "00:00");
    /// let midnight_again = PlcTimeOfDay::from_nanos_since_midnight(86_400_000_000_000);
    /// assert_eq!(midnight_again.map_err(|e| e.kind()), Err(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `nanos` is a whole day or more.
    pub fn from_nanos_since_midnight(nanos: u64) -> Result<Self> {
        let seconds = nanos.div_euclid(NANOS_PER_SECOND);
        // A count of a day or more gives an hour of 24 or more, which `from_hms_nano`
        // refuses; an hour beyond `u8` becomes `u8::MAX` and is refused the same way.
        Self::from_hms_nano(
            u8::try_from(seconds.div_euclid(3_600)).unwrap_or(u8::MAX),
            u8::try_from(seconds.div_euclid(60).rem_euclid(60)).unwrap_or(u8::MAX),
            u8::try_from(seconds.rem_euclid(60)).unwrap_or(u8::MAX),
            u32::try_from(nanos.rem_euclid(NANOS_PER_SECOND)).unwrap_or(u32::MAX),
        )
    }

    /// The nanoseconds since midnight, below 86,400,000,000,000.
    #[must_use]
    pub fn nanos_since_midnight(self) -> u64 {
        let seconds = u64::from(self.hour)
            .saturating_mul(3_600)
            .saturating_add(u64::from(self.minute).saturating_mul(60))
            .saturating_add(u64::from(self.second));
        seconds
            .saturating_mul(NANOS_PER_SECOND)
            .saturating_add(u64::from(self.nanosecond))
    }

    /// The hour, 0 to 23.
    #[must_use]
    pub const fn hour(self) -> u8 {
        self.hour
    }

    /// The minute, 0 to 59.
    #[must_use]
    pub const fn minute(self) -> u8 {
        self.minute
    }

    /// The second, 0 to 59.
    #[must_use]
    pub const fn second(self) -> u8 {
        self.second
    }

    /// The nanosecond within the second, 0 to 999,999,999.
    #[must_use]
    pub const fn nanosecond(self) -> u32 {
        self.nanosecond
    }
}

impl fmt::Display for PlcTimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:02}:{:02}", self.hour, self.minute)?;
        if self.second == 0 && self.nanosecond == 0 {
            return Ok(());
        }
        write!(f, ":{:02}", self.second)?;
        let nanos = self.nanosecond;
        if nanos == 0 {
            Ok(())
        } else if nanos.rem_euclid(1_000_000) == 0 {
            write!(f, ".{:03}", nanos.div_euclid(1_000_000))
        } else if nanos.rem_euclid(1_000) == 0 {
            write!(f, ".{:06}", nanos.div_euclid(1_000))
        } else {
            write!(f, ".{nanos:09}")
        }
    }
}

/// A date and time of day without a time zone: the IEC 61131-3 `DATE_AND_TIME`,
/// `DATE_AND_LTIME` and `LDATE_AND_TIME` values.
///
/// IEC 61131-3 values carry no zone or offset, so none is invented: a device's local
/// clock reading stays a local reading. The range and resolution are those of
/// [`PlcDate`] and [`PlcTimeOfDay`]; ordering is chronological. `Display` renders the
/// ISO 8601 text of `java.time.LocalDateTime::toString`: the date, `T`, the time.
///
/// # Examples
/// ```
/// use stonecrab::{PlcDate, PlcDateTime, PlcTimeOfDay};
/// let date = PlcDate::from_ymd(2026, 10, 28)?;
/// let time = PlcTimeOfDay::from_hms_nano(6, 30, 15, 250_000_000)?;
/// let stamp = PlcDateTime::new(date, time);
/// assert_eq!((stamp.date(), stamp.time()), (date, time));
/// assert_eq!(stamp.to_string(), "2026-10-28T06:30:15.250");
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
///
/// # Compatibility: docs/parity/core.md#CORE-VALUE-CALENDAR
/// Upstream `PlcDATE_AND_TIME`, `PlcDATE_AND_LTIME` and `PlcLDATE_AND_TIME` hold a
/// `java.time.LocalDateTime`; a date beyond the [`PlcDate`] range has no Stonecrab value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub struct PlcDateTime {
    date: PlcDate,
    time: PlcTimeOfDay,
}

impl PlcDateTime {
    /// The time of day `time` on `date`; every pair is in range.
    #[must_use]
    pub const fn new(date: PlcDate, time: PlcTimeOfDay) -> Self {
        Self { date, time }
    }

    /// The calendar date.
    #[must_use]
    pub const fn date(self) -> PlcDate {
        self.date
    }

    /// The time of day.
    #[must_use]
    pub const fn time(self) -> PlcTimeOfDay {
        self.time
    }
}

impl fmt::Display for PlcDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}T{}", self.date, self.time)
    }
}

/// Whether `year` is a Gregorian leap year.
const fn is_leap_year(year: i32) -> bool {
    year.rem_euclid(4) == 0 && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0)
}

/// The number of days of a month (1 to 12) in `year`.
const fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a valid civil date (H. Hinnant's `days_from_civil`).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    reason = "ADR-0021: i64 operands bounded by an i32 year and a validated month and day cannot overflow, and every divisor is a positive constant"
)]
fn days_from_civil(year: i32, month: u8, day: u8) -> i64 {
    let (month, day) = (i64::from(month), i64::from(day));
    // Years start in March, so January and February belong to the previous year.
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * DAYS_PER_ERA + day_of_era - DAYS_TO_EPOCH
}

/// The civil year, month and day `days` days after 1970-01-01 (H. Hinnant's
/// `civil_from_days`).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    reason = "ADR-0021: i64 operands bounded by an i32 day count cannot overflow, and every divisor is a positive constant"
)]
fn civil_from_days(days: i32) -> (i32, u8, u8) {
    let shifted = i64::from(days) + DAYS_TO_EPOCH;
    let era = shifted.div_euclid(DAYS_PER_ERA);
    let day_of_era = shifted.rem_euclid(DAYS_PER_ERA);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = (month_from_march + 2) % 12 + 1;
    let year = era * 400 + year_of_era + i64::from(month <= 2);
    // The bounds hold by construction: an i32 day count is within i32 years, the month is
    // 1 to 12 and the day 1 to 31, so the fallbacks are never taken.
    (
        i32::try_from(year).unwrap_or(i32::MAX),
        u8::try_from(month).unwrap_or(u8::MAX),
        u8::try_from(day).unwrap_or(u8::MAX),
    )
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use proptest::prelude::*;

    use super::*;

    // covers: ALPHA_PLAN D13 / additive Ord on PlcValueType
    #[test]
    fn value_types_sort_in_declaration_order_stay_copy_and_round_trip_through_a_set() {
        let declared = [
            PlcValueType::Null,
            PlcValueType::Bool,
            PlcValueType::I8,
            PlcValueType::I16,
            PlcValueType::I32,
            PlcValueType::I64,
            PlcValueType::U8,
            PlcValueType::U16,
            PlcValueType::U32,
            PlcValueType::U64,
            PlcValueType::F32,
            PlcValueType::F64,
            PlcValueType::Char,
            PlcValueType::String,
            PlcValueType::Duration,
            PlcValueType::SignedDuration,
            PlcValueType::Date,
            PlcValueType::TimeOfDay,
            PlcValueType::DateTime,
        ];
        let mut reversed = declared;
        reversed.reverse();
        let mut sorted = reversed.to_vec();
        sorted.sort();
        assert_eq!(sorted, declared, "sort restores declaration order");
        for (left, right) in declared.iter().zip(declared.iter().skip(1)) {
            assert!(left < right, "strictly increasing");
            assert_eq!(left.partial_cmp(right), Some(std::cmp::Ordering::Less));
        }
        let copied = declared;
        assert_eq!(copied, declared, "Copy leaves the source usable");
        let set: std::collections::BTreeSet<_> = reversed.into_iter().collect();
        assert_eq!(
            set.into_iter().collect::<Vec<_>>(),
            declared,
            "set iterates sorted"
        );
    }

    #[test]
    fn each_value_reports_its_original_representation() {
        for (value, kind) in [
            (PlcValue::Null, PlcValueType::Null),
            (PlcValue::Bool(true), PlcValueType::Bool),
            (PlcValue::I8(i8::MIN), PlcValueType::I8),
            (PlcValue::I16(i16::MIN), PlcValueType::I16),
            (PlcValue::I32(i32::MIN), PlcValueType::I32),
            (PlcValue::I64(i64::MIN), PlcValueType::I64),
            (PlcValue::U8(u8::MAX), PlcValueType::U8),
            (PlcValue::U16(u16::MAX), PlcValueType::U16),
            (PlcValue::U32(u32::MAX), PlcValueType::U32),
            (PlcValue::U64(u64::MAX), PlcValueType::U64),
            (PlcValue::F32(1.5), PlcValueType::F32),
            (PlcValue::F64(1.5), PlcValueType::F64),
            (PlcValue::Char('α'), PlcValueType::Char),
            (
                PlcValue::String(String::from("temperature")),
                PlcValueType::String,
            ),
            (
                PlcValue::Duration(Duration::from_secs(1)),
                PlcValueType::Duration,
            ),
            (
                PlcValue::SignedDuration(PlcSignedDuration::from_nanos(-1)),
                PlcValueType::SignedDuration,
            ),
            (
                PlcValue::Date(PlcDate::from_days_since_epoch(0)),
                PlcValueType::Date,
            ),
            (PlcValue::TimeOfDay(midnight()), PlcValueType::TimeOfDay),
            (
                PlcValue::DateTime(PlcDateTime::new(
                    PlcDate::from_days_since_epoch(0),
                    midnight(),
                )),
                PlcValueType::DateTime,
            ),
        ] {
            assert_eq!(value.value_type(), kind, "no implicit widening");
            assert_eq!(
                value.clone(),
                value,
                "clone retains value and representation"
            );
        }
    }

    const NANOS_PER_DAY: u64 = 86_400_000_000_000;
    /// The last whole millisecond of the `i64` nanosecond range.
    const LAST_MILLI: i64 = i64::MAX.div_euclid(1_000_000);

    fn midnight() -> PlcTimeOfDay {
        PlcTimeOfDay::from_nanos_since_midnight(0).expect("midnight")
    }

    fn date(year: i32, month: u8, day: u8) -> PlcDate {
        PlcDate::from_ymd(year, month, day).expect("valid date")
    }

    fn time(hour: u8, minute: u8, second: u8, nanosecond: u32) -> PlcTimeOfDay {
        PlcTimeOfDay::from_hms_nano(hour, minute, second, nanosecond).expect("valid time")
    }

    // covers: CORE-VALUE-CALENDAR / no numeric or Boolean coercion of calendar values
    #[test]
    fn calendar_values_are_never_coerced_to_numbers_or_booleans() {
        for value in [
            PlcValue::SignedDuration(PlcSignedDuration::from_nanos(1)),
            PlcValue::Date(PlcDate::from_days_since_epoch(1)),
            PlcValue::TimeOfDay(time(0, 0, 1, 0)),
            PlcValue::DateTime(PlcDateTime::new(date(1970, 1, 2), midnight())),
        ] {
            assert_eq!(value.as_bool(), None, "no truthiness");
            assert_eq!(value.as_i64(), None, "no count leaks out as an integer");
            assert_eq!(value.as_u64(), None, "no count leaks out as an integer");
            assert_eq!(value.as_f64(), None, "no float funnel");
        }
    }

    // covers: SPEC mandatory-failure-cases / integer boundaries (CORE-VALUE-CALENDAR ranges)
    #[test]
    fn signed_durations_cover_the_whole_i64_nanosecond_range_and_no_more() {
        for nanos in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(
                PlcSignedDuration::from_nanos(nanos).as_nanos(),
                nanos,
                "every i64 is a duration"
            );
        }
        let last = LAST_MILLI;
        assert_eq!(
            PlcSignedDuration::from_millis(last)
                .map(PlcSignedDuration::as_nanos)
                .ok(),
            last.checked_mul(1_000_000),
            "the last whole millisecond fits"
        );
        for millis in [last + 1, -last - 1, i64::MAX, i64::MIN] {
            assert_eq!(
                PlcSignedDuration::from_millis(millis).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{millis} ms is beyond the range"
            );
        }
        assert!(
            PlcSignedDuration::from_nanos(-1) < PlcSignedDuration::from_nanos(0),
            "ordering is signed"
        );
    }

    // covers: CORE-VALUE-CALENDAR / Display is java.time.Duration::toString
    #[test]
    fn signed_durations_render_as_java_time_does() {
        for (nanos, text) in [
            (0, "PT0S"),
            (1, "PT0.000000001S"),
            (-1, "PT-0.000000001S"),
            (500_000_000, "PT0.5S"),
            (-500_000_000, "PT-0.5S"),
            (1_500_000_000, "PT1.5S"),
            (-1_500_000_000, "PT-1.5S"),
            (60_000_000_000, "PT1M"),
            (-60_500_000_000, "PT-1M-0.5S"),
            (-90_500_000_000, "PT-1M-30.5S"),
            (3_600_000_000_000, "PT1H"),
            (3_601_000_000_000, "PT1H1S"),
            (90_061_123_400_000, "PT25H1M1.1234S"),
            (i64::MAX, "PT2562047H47M16.854775807S"),
            (i64::MIN, "PT-2562047H-47M-16.854775808S"),
        ] {
            assert_eq!(
                PlcSignedDuration::from_nanos(nanos).to_string(),
                text,
                "{nanos} ns"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / integer boundaries (CORE-VALUE-CALENDAR ranges)
    #[test]
    fn dates_cover_the_i32_day_range_and_refuse_dates_outside_it() {
        let first = PlcDate::from_days_since_epoch(i32::MIN);
        let last = PlcDate::from_days_since_epoch(i32::MAX);
        assert_eq!(
            (first.year(), first.month(), first.day()),
            (-5_877_641, 6, 23),
            "first date"
        );
        assert_eq!(
            (last.year(), last.month(), last.day()),
            (5_881_580, 7, 11),
            "last date"
        );
        assert_eq!(date(-5_877_641, 6, 23), first, "the first date by fields");
        assert_eq!(date(5_881_580, 7, 11), last, "the last date by fields");
        for (year, month, day) in [
            (-5_877_641, 6, 22),
            (5_881_580, 7, 12),
            (i32::MIN, 1, 1),
            (i32::MAX, 12, 31),
        ] {
            let refused = PlcDate::from_ymd(year, month, day).expect_err("outside the range");
            assert_eq!(refused.kind(), ErrorKind::InvalidConfig, "typed refusal");
            assert!(
                refused.source().is_some(),
                "the conversion failure is kept as the source"
            );
        }
        assert_eq!(first.to_string(), "-5877641-06-23", "first date text");
        assert_eq!(last.to_string(), "+5881580-07-11", "last date text");
    }

    // covers: CORE-VALUE-CALENDAR / Gregorian leap years and month lengths
    #[test]
    fn dates_follow_the_gregorian_calendar() {
        for (year, month, day) in [
            (2000, 2, 29),
            (2024, 2, 29),
            (-4, 2, 29),
            (0, 2, 29),
            (2023, 1, 31),
            (2023, 4, 30),
            (2023, 12, 31),
        ] {
            let valid = date(year, month, day);
            assert_eq!(
                (valid.year(), valid.month(), valid.day()),
                (year, month, day),
                "fields round trip"
            );
        }
        for (year, month, day) in [
            (1900, 2, 29),
            (2023, 2, 29),
            (2023, 2, 30),
            (2023, 4, 31),
            (2023, 0, 1),
            (2023, 13, 1),
            (2023, 1, 0),
            (2023, 1, 32),
        ] {
            assert_eq!(
                PlcDate::from_ymd(year, month, day).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{year}-{month}-{day} is not in the calendar"
            );
        }
        assert_eq!(date(1970, 1, 1).days_since_epoch(), 0, "the epoch");
        assert_eq!(date(1969, 12, 31).days_since_epoch(), -1, "the day before");
        assert_eq!(
            date(2000, 3, 1).days_since_epoch(),
            11_017,
            "after a leap day"
        );
        assert!(date(1969, 12, 31) < date(1970, 1, 1), "chronological order");
    }

    // covers: CORE-VALUE-CALENDAR / Display is java.time.LocalDate::toString
    #[test]
    fn dates_render_as_java_time_does() {
        for ((year, month, day), text) in [
            ((1970, 1, 1), "1970-01-01"),
            ((9_999, 12, 31), "9999-12-31"),
            ((10_000, 1, 1), "+10000-01-01"),
            ((999, 3, 9), "0999-03-09"),
            ((0, 1, 1), "0000-01-01"),
            ((-1, 12, 31), "-0001-12-31"),
            ((-10_000, 1, 1), "-10000-01-01"),
        ] {
            assert_eq!(date(year, month, day).to_string(), text, "{text}");
        }
    }

    // covers: SPEC mandatory-failure-cases / integer boundaries (CORE-VALUE-CALENDAR ranges)
    #[test]
    fn times_of_day_end_before_midnight_and_have_no_leap_second() {
        let last = PlcTimeOfDay::from_nanos_since_midnight(NANOS_PER_DAY - 1).expect("last");
        assert_eq!(last, time(23, 59, 59, 999_999_999), "the last instant");
        assert_eq!(last.nanos_since_midnight(), NANOS_PER_DAY - 1, "round trip");
        for nanos in [NANOS_PER_DAY, NANOS_PER_DAY + 1, u64::MAX] {
            assert_eq!(
                PlcTimeOfDay::from_nanos_since_midnight(nanos).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{nanos} ns is a day or more"
            );
        }
        for (hour, minute, second, nanosecond) in [
            (24, 0, 0, 0),
            (0, 60, 0, 0),
            (23, 59, 60, 0),
            (0, 0, 0, 1_000_000_000),
            (u8::MAX, u8::MAX, u8::MAX, u32::MAX),
        ] {
            assert_eq!(
                PlcTimeOfDay::from_hms_nano(hour, minute, second, nanosecond).map_err(|e| e.kind()),
                Err(ErrorKind::InvalidConfig),
                "{hour}:{minute}:{second}.{nanosecond} is not a time of day"
            );
        }
        assert!(time(0, 0, 0, 1) < time(0, 0, 1, 0), "chronological order");
    }

    // covers: CORE-VALUE-CALENDAR / Display is java.time.LocalTime::toString
    #[test]
    fn times_of_day_render_as_java_time_does() {
        for ((hour, minute, second, nanosecond), text) in [
            ((0, 0, 0, 0), "00:00"),
            ((10, 15, 0, 0), "10:15"),
            ((10, 15, 30, 0), "10:15:30"),
            ((10, 15, 0, 1_000_000), "10:15:00.001"),
            ((10, 15, 30, 120_000_000), "10:15:30.120"),
            ((10, 15, 30, 1_000), "10:15:30.000001"),
            ((10, 15, 30, 123_456_000), "10:15:30.123456"),
            ((10, 15, 30, 1), "10:15:30.000000001"),
        ] {
            assert_eq!(
                time(hour, minute, second, nanosecond).to_string(),
                text,
                "{text}"
            );
        }
    }

    // covers: CORE-VALUE-CALENDAR / Display is java.time.LocalDateTime::toString
    #[test]
    fn date_times_pair_a_date_with_a_time_in_chronological_order() {
        let epoch = PlcDateTime::new(date(1970, 1, 1), midnight());
        let before = PlcDateTime::new(date(1969, 12, 31), time(23, 59, 59, 999_999_999));
        assert!(before < epoch, "the date orders first, then the time");
        assert_eq!(epoch.to_string(), "1970-01-01T00:00", "epoch text");
        assert_eq!(
            before.to_string(),
            "1969-12-31T23:59:59.999999999",
            "last instant before the epoch"
        );
        assert_eq!(before.date(), date(1969, 12, 31), "date accessor");
        assert_eq!(before.time().hour(), 23, "time accessor");
    }

    proptest! {
        // covers: CORE-VALUE-CALENDAR / day count and civil fields round trip
        #[test]
        fn every_day_count_round_trips_through_its_civil_fields(days in any::<i32>()) {
            let from_days = PlcDate::from_days_since_epoch(days);
            let rebuilt = PlcDate::from_ymd(from_days.year(), from_days.month(), from_days.day());
            prop_assert_eq!(rebuilt.ok(), Some(from_days));
            prop_assert_eq!(from_days.days_since_epoch(), days);
        }

        // covers: CORE-VALUE-CALENDAR / chronological order of dates
        #[test]
        fn date_order_is_day_count_order(a in any::<i32>(), b in any::<i32>()) {
            let (left, right) = (PlcDate::from_days_since_epoch(a), PlcDate::from_days_since_epoch(b));
            prop_assert_eq!(
                left.cmp(&right),
                (left.year(), left.month(), left.day()).cmp(&(right.year(), right.month(), right.day()))
            );
        }

        // covers: CORE-VALUE-CALENDAR / nanoseconds and fields of a time of day round trip
        #[test]
        fn every_nanosecond_of_the_day_round_trips(nanos in 0..NANOS_PER_DAY) {
            let time = PlcTimeOfDay::from_nanos_since_midnight(nanos).expect("a time of day");
            prop_assert_eq!(time.nanos_since_midnight(), nanos);
            let rebuilt = PlcTimeOfDay::from_hms_nano(time.hour(), time.minute(), time.second(), time.nanosecond());
            prop_assert_eq!(rebuilt.ok(), Some(time));
        }

        // covers: CORE-VALUE-CALENDAR / chronological order of times of day
        #[test]
        fn time_order_is_nanosecond_order(a in 0..NANOS_PER_DAY, b in 0..NANOS_PER_DAY) {
            let left = PlcTimeOfDay::from_nanos_since_midnight(a).ok();
            let right = PlcTimeOfDay::from_nanos_since_midnight(b).ok();
            prop_assert_eq!(left.cmp(&right), a.cmp(&b));
        }

        // covers: CORE-VALUE-CALENDAR / millisecond durations are exact
        #[test]
        fn millisecond_durations_are_exact_nanosecond_counts(millis in -LAST_MILLI..=LAST_MILLI) {
            let span = PlcSignedDuration::from_millis(millis);
            prop_assert_eq!(span.map(PlcSignedDuration::as_nanos).ok(), millis.checked_mul(1_000_000));
        }
    }

    // covers: SPEC mandatory-failure-cases / negative values and integer boundaries
    #[test]
    fn signed_integers_widen_exactly_without_becoming_float_or_unsigned() {
        for (value, expected) in [
            (PlcValue::I8(i8::MIN), i64::from(i8::MIN)),
            (PlcValue::I16(i16::MIN), i64::from(i16::MIN)),
            (PlcValue::I32(i32::MIN), i64::from(i32::MIN)),
            (PlcValue::I64(i64::MIN), i64::MIN),
        ] {
            assert_eq!(
                value.as_i64(),
                Some(expected),
                "signed minimum remains exact"
            );
            assert_eq!(
                value.as_u64(),
                None,
                "negative values cannot become unsigned"
            );
            assert_eq!(value.as_f64(), None, "no float funnel");
        }
        for value in [
            PlcValue::I8(0),
            PlcValue::I16(0),
            PlcValue::I32(0),
            PlcValue::I64(0),
        ] {
            assert_eq!(
                value.as_u64(),
                Some(0),
                "nonnegative signed values may widen"
            );
        }
        assert_eq!(
            PlcValue::I64(i64::MAX).as_i64(),
            Some(i64::MAX),
            "signed maximum exact"
        );
        assert_eq!(
            PlcValue::I64(i64::MAX).as_u64(),
            Some(i64::MAX.unsigned_abs()),
            "signed maximum widens exactly"
        );
    }

    #[test]
    fn unsigned_counters_never_round_or_wrap() {
        for (value, expected) in [
            (PlcValue::U8(u8::MAX), u64::from(u8::MAX)),
            (PlcValue::U16(u16::MAX), u64::from(u16::MAX)),
            (PlcValue::U32(u32::MAX), u64::from(u32::MAX)),
            (PlcValue::U64(u64::MAX), u64::MAX),
        ] {
            assert_eq!(value.as_u64(), Some(expected), "unsigned maximum exact");
            assert_eq!(
                value.as_i64(),
                i64::try_from(expected).ok(),
                "signed conversion is checked"
            );
            assert_eq!(value.as_f64(), None, "counters are never rounded");
        }
        let signed_max = i64::MAX.unsigned_abs();
        assert_eq!(
            PlcValue::U64(signed_max).as_i64(),
            Some(i64::MAX),
            "last valid signed value"
        );
        assert_eq!(
            PlcValue::U64(signed_max.saturating_add(1)).as_i64(),
            None,
            "first overflowing signed value"
        );
    }

    // covers: SPEC mandatory-failure-cases / NaN and infinity
    #[test]
    fn float_access_preserves_special_values_without_integer_coercion() {
        for value in [PlcValue::F32(1.5), PlcValue::F64(1.5)] {
            assert_eq!(
                value.as_f64().expect("floating variant").to_bits(),
                1.5_f64.to_bits(),
                "float widening is exact"
            );
            assert_eq!(value.as_i64(), None, "no truncating cast");
            assert_eq!(value.as_u64(), None, "no truncating cast");
        }
        assert!(
            PlcValue::F32(f32::NAN)
                .as_f64()
                .expect("NaN retained")
                .is_nan(),
            "NaN is retained for quality validation"
        );
        assert!(
            PlcValue::F64(f64::INFINITY)
                .as_f64()
                .expect("infinity retained")
                .is_infinite(),
            "infinity retained"
        );
        assert_eq!(
            PlcValue::F32(-0.0).as_f64().expect("signed zero").to_bits(),
            (-0.0_f64).to_bits(),
            "signed zero retained"
        );
    }

    #[test]
    fn boolean_access_is_not_truthiness_and_absence_is_not_zero() {
        assert_eq!(
            PlcValue::Bool(false).as_bool(),
            Some(false),
            "false is a value"
        );
        assert_eq!(
            PlcValue::Bool(true).as_bool(),
            Some(true),
            "true is a value"
        );
        for value in [
            PlcValue::Null,
            PlcValue::I8(1),
            PlcValue::String(String::from("true")),
        ] {
            assert_eq!(value.as_bool(), None, "no truthiness coercion");
        }
        for value in [
            PlcValue::Null,
            PlcValue::Bool(true),
            PlcValue::String(String::from("1")),
        ] {
            assert_eq!(value.as_i64(), None, "no numeric coercion");
            assert_eq!(value.as_u64(), None, "no numeric coercion");
            assert_eq!(value.as_f64(), None, "no numeric coercion");
        }
    }
}
