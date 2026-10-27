use std::time::Duration;

/// The exact scalar representation of a [`PlcValue`].
///
/// Calendar, byte-buffer and recursive collection representations are deferred;
/// they will not be silently substituted with strings or floating-point numbers.
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
            | Self::Duration(_) => None,
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
            | Self::Duration(_) => None,
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
            | Self::Duration(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
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
        ] {
            assert_eq!(value.value_type(), kind, "no implicit widening");
            assert_eq!(
                value.clone(),
                value,
                "clone retains value and representation"
            );
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
