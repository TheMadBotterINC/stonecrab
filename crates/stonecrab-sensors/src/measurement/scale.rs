//! The scaling engine: one value through an ordered list of [`ScaleStep`]s.
//!
//! Integers are held as `i128` restricted to `i64::MIN..=u64::MAX`, which holds every
//! integer `PlcValue` exactly. A step that would leave that range is computed in `f64`
//! instead. The module never uses `as`: widening goes through `From`, narrowing through
//! `TryFrom`, and the two integer-to-float and float-to-integer conversions are written
//! out below because core offers neither.

use stonecrab_core::PlcValue;

use crate::ScaleStep;

/// 2^32, the weight of the high half when a `u64` is turned into an `f64`.
const TWO_POW_32: f64 = 4_294_967_296.0;

/// A number in flight through the pipeline.
#[derive(Debug, Clone, Copy)]
enum Num {
    /// An integer inside `i64::MIN..=u64::MAX`.
    Int(i128),
    /// A finite float.
    Float(f64),
}

impl Num {
    /// The number inside a value, or `None` for a value that is not a number.
    fn from_value(value: &PlcValue) -> Option<Self> {
        value
            .as_i64()
            .map(i128::from)
            .or_else(|| value.as_u64().map(i128::from))
            .map(Self::Int)
            .or_else(|| value.as_f64().map(Self::Float))
    }

    /// The number as a float; `None` only for an integer outside the invariant range.
    fn to_f64(self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(value),
            Self::Int(value) if value < 0 => {
                Some(-u64_to_f64(u64::try_from(value.unsigned_abs()).ok()?))
            }
            Self::Int(value) => Some(u64_to_f64(u64::try_from(value).ok()?)),
        }
    }

    /// The number as a value: an integer keeps its own type (`own`, from [`own_type`]) when
    /// it fits, else becomes `I64`, else `U64`; a float becomes `F64`.
    fn into_value(self, own: fn(i128) -> Option<PlcValue>) -> Option<PlcValue> {
        match self {
            Self::Float(value) => Some(PlcValue::F64(value)),
            Self::Int(value) => own(value)
                .or_else(|| i64::try_from(value).ok().map(PlcValue::I64))
                .or_else(|| u64::try_from(value).ok().map(PlcValue::U64)),
        }
    }
}

/// How to rebuild an integer as the type of `like`, when it fits; never fits for a float.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "PlcValue is non_exhaustive in stonecrab-core, so a wildcard arm is required"
)]
fn own_type(like: &PlcValue) -> fn(i128) -> Option<PlcValue> {
    match like {
        PlcValue::I8(_) => |v| i8::try_from(v).ok().map(PlcValue::I8),
        PlcValue::I16(_) => |v| i16::try_from(v).ok().map(PlcValue::I16),
        PlcValue::I32(_) => |v| i32::try_from(v).ok().map(PlcValue::I32),
        PlcValue::I64(_) => |v| i64::try_from(v).ok().map(PlcValue::I64),
        PlcValue::U8(_) => |v| u8::try_from(v).ok().map(PlcValue::U8),
        PlcValue::U16(_) => |v| u16::try_from(v).ok().map(PlcValue::U16),
        PlcValue::U32(_) => |v| u32::try_from(v).ok().map(PlcValue::U32),
        PlcValue::U64(_) => |v| u64::try_from(v).ok().map(PlcValue::U64),
        _ => |_| None,
    }
}

/// True when `value` is inside the range the integer path keeps: `i64::MIN..=u64::MAX`.
fn in_range(value: &i128) -> bool {
    (i128::from(i64::MIN)..=i128::from(u64::MAX)).contains(value)
}

/// A `u64` as the nearest `f64` (ties to even), by exact halves: the high half times 2^32
/// is exact and the single addition rounds once, as `as` would.
fn u64_to_f64(value: u64) -> f64 {
    let high = f64::from(u32::try_from(value >> 32).unwrap_or(u32::MAX));
    let low = f64::from(u32::try_from(value & 0xffff_ffff).unwrap_or(u32::MAX));
    high * TWO_POW_32 + low
}

/// The exact integer value of `constant` when it has no fractional part and a magnitude
/// below 2^64; `None` for NaN, infinities, fractions and anything larger.
fn integral(constant: f64) -> Option<i128> {
    if !constant.is_finite() {
        return None;
    }
    let bits = constant.to_bits();
    let negative = bits >> 63 != 0;
    let exponent = i32::try_from((bits >> 52) & 0x7ff).ok()?;
    let mantissa = bits & 0x000f_ffff_ffff_ffff;
    let magnitude = if exponent == 0 {
        // Zero or subnormal: only zero is integral.
        if mantissa != 0 {
            return None;
        }
        0
    } else {
        let full = mantissa | (1 << 52);
        let shift = exponent.checked_sub(1075)?;
        if let Ok(left) = u32::try_from(shift) {
            full.checked_mul(1_u64.checked_shl(left)?)?
        } else {
            let right = shift.unsigned_abs();
            if full.trailing_zeros() < right {
                return None;
            }
            full.checked_shr(right)?
        }
    };
    let magnitude = i128::from(magnitude);
    if negative {
        magnitude.checked_neg()
    } else {
        Some(magnitude)
    }
}

/// `Some(number)` when `value` is finite.
fn finite(value: f64) -> Option<Num> {
    value.is_finite().then_some(Num::Float(value))
}

/// The 64-bit two's complement pattern of an integer: unsigned values as they are, negative
/// values sign-extended.
fn bits_of(value: i128) -> Option<u64> {
    if value < 0 {
        i64::try_from(value)
            .ok()
            .map(|signed| u64::from_be_bytes(signed.to_be_bytes()))
    } else {
        u64::try_from(value).ok()
    }
}

/// Run one step.
fn apply(num: Num, step: ScaleStep) -> Option<Num> {
    match step {
        ScaleStep::Multiply(constant) => arithmetic(num, constant, i128::checked_mul, |a, b| a * b),
        ScaleStep::Offset(constant) => arithmetic(num, constant, i128::checked_add, |a, b| a + b),
        ScaleStep::Divide(constant) => finite(num.to_f64()? / constant),
        ScaleStep::Clamp { min, max } => clamp(num, min, max),
        ScaleStep::BitField { shift, width } => bit_field(num, shift, width),
    }
}

/// `Multiply` and `Offset`: an integer with an integral constant stays an integer while the
/// result stays in range; every other combination is computed in `f64`.
fn arithmetic(
    num: Num,
    constant: f64,
    integer_op: fn(i128, i128) -> Option<i128>,
    float_op: fn(f64, f64) -> f64,
) -> Option<Num> {
    if let Num::Int(value) = num
        && let Some(result) = integral(constant)
            .and_then(|whole| integer_op(value, whole))
            .filter(in_range)
    {
        return Some(Num::Int(result));
    }
    finite(float_op(num.to_f64()?, constant))
}

/// `Clamp`: integral bounds on an integer compare as integers; otherwise as floats.
fn clamp(num: Num, min: f64, max: f64) -> Option<Num> {
    if !(min.is_finite() && max.is_finite() && min <= max) {
        return None;
    }
    if let Num::Int(value) = num
        && let (Some(low), Some(high)) = (integral(min), integral(max))
    {
        let result = value.clamp(low, high);
        if in_range(&result) {
            return Some(Num::Int(result));
        }
    }
    finite(num.to_f64()?.clamp(min, max))
}

/// `BitField`: `width` bits from bit `shift` of an integer's 64-bit pattern.
fn bit_field(num: Num, shift: u8, width: u8) -> Option<Num> {
    let Num::Int(value) = num else {
        return None;
    };
    if width == 0 || shift.checked_add(width)? > 64 {
        return None;
    }
    let mask = match 1_u64.checked_shl(u32::from(width)) {
        Some(bit) => bit.checked_sub(1)?,
        None => u64::MAX,
    };
    let field = bits_of(value)?.checked_shr(u32::from(shift))? & mask;
    Some(Num::Int(i128::from(field)))
}

/// Run `steps` in order over `value`.
///
/// `None` means the value cannot be scaled and its quality is bad: a value that is not a
/// number but has steps, a NaN or infinite input, an unevaluable step (non-finite constant,
/// clamp with `min > max`, a bit field of width 0 or ending past bit 64, a bit field on a
/// float), or a result that is not finite. With no steps the value passes through
/// unchanged, whatever its type.
pub(super) fn scale(value: &PlcValue, steps: &[ScaleStep]) -> Option<PlcValue> {
    let Some(start) = Num::from_value(value) else {
        return steps.is_empty().then(|| value.clone());
    };
    if let Num::Float(float) = start
        && !float.is_finite()
    {
        return None;
    }
    if steps.is_empty() {
        return Some(value.clone());
    }
    steps
        .iter()
        .try_fold(start, |num, step| apply(num, *step))?
        .into_value(own_type(value))
}

/// A numeric value as `f64`, for the range check; `None` for a value that is not a number.
pub(super) fn as_f64(value: &PlcValue) -> Option<f64> {
    Num::from_value(value)?.to_f64()
}

/// Bit `bit` of a status word: a `Bool` is the bit itself, an integer is read from its
/// 64-bit pattern, anything else (or a bit beyond 63) is unknown.
pub(super) fn status_bit(value: &PlcValue, bit: u8) -> Option<bool> {
    if let PlcValue::Bool(set) = value {
        return Some(*set);
    }
    let Num::Int(word) = Num::from_value(value)? else {
        return None;
    };
    Some(bits_of(word)?.checked_shr(u32::from(bit))? & 1 == 1)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn run(value: &PlcValue, steps: &[ScaleStep]) -> Option<PlcValue> {
        scale(value, steps)
    }

    /// The `f64` inside a result, panicking with the actual value when it is not one.
    fn float(result: Option<PlcValue>) -> f64 {
        assert!(
            matches!(result, Some(PlcValue::F64(_))),
            "expected an F64, got {result:?}"
        );
        result
            .and_then(|value| value.as_f64())
            .expect("checked above")
    }

    /// 2^`exponent`, exactly.
    fn pow2(exponent: i32) -> f64 {
        2.0_f64.powi(exponent)
    }

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn a_negative_integer_scales_to_a_negative_float() {
        let scaled = float(run(&PlcValue::I16(-2424), &[ScaleStep::Multiply(0.1)]));
        assert!(close(scaled, -242.4), "got {scaled}");
        let divided = float(run(&PlcValue::I16(-2424), &[ScaleStep::Divide(10.0)]));
        assert!(close(divided, -242.4), "got {divided}");
        let shifted = float(run(
            &PlcValue::I16(-2424),
            &[ScaleStep::Divide(10.0), ScaleStep::Offset(-40.0)],
        ));
        assert!(close(shifted, -282.4), "got {shifted}");
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn nan_and_infinity_are_bad_with_or_without_steps() {
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(run(&PlcValue::F32(bad), &[]), None, "bare {bad}");
            assert_eq!(
                run(&PlcValue::F32(bad), &[ScaleStep::Multiply(2.0)]),
                None,
                "scaled {bad}"
            );
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(run(&PlcValue::F64(bad), &[]), None, "bare {bad}");
            assert_eq!(
                run(&PlcValue::F64(bad), &[ScaleStep::Offset(1.0)]),
                None,
                "offset {bad}"
            );
        }
    }

    #[test]
    fn a_step_that_overflows_or_divides_by_zero_is_bad() {
        let huge = PlcValue::F64(1e300);
        assert_eq!(
            run(&huge, &[ScaleStep::Multiply(1e300)]),
            None,
            "multiply overflows"
        );
        assert_eq!(
            run(&huge, &[ScaleStep::Offset(f64::MAX)]),
            None,
            "offset overflows"
        );
        assert_eq!(
            run(&huge, &[ScaleStep::Divide(1e-300)]),
            None,
            "divide overflows f64"
        );
        assert_eq!(
            run(&PlcValue::U16(5), &[ScaleStep::Divide(0.0)]),
            None,
            "an integer divided by zero"
        );
        assert_eq!(
            run(&PlcValue::F64(0.0), &[ScaleStep::Divide(0.0)]),
            None,
            "zero over zero is NaN"
        );
        for constant in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for step in [
                ScaleStep::Multiply(constant),
                ScaleStep::Divide(constant),
                ScaleStep::Offset(constant),
            ] {
                // Dividing by an infinity is zero, which is finite and a legitimate result.
                let expected = if matches!(step, ScaleStep::Divide(c) if c.is_infinite()) {
                    Some(PlcValue::F64(0.0))
                } else {
                    None
                };
                assert_eq!(
                    run(&PlcValue::I16(3), &[step]),
                    expected,
                    "{step:?} on an integer"
                );
                assert_eq!(
                    run(&PlcValue::F64(3.0), &[step]),
                    expected,
                    "{step:?} on a float"
                );
            }
        }
    }

    #[test]
    fn integers_stay_integers_through_integral_multiply_and_offset() {
        assert_eq!(
            run(&PlcValue::U16(500), &[ScaleStep::Multiply(1.0)]),
            Some(PlcValue::U16(500)),
            "a UINT counter times 1.0"
        );
        assert_eq!(
            run(&PlcValue::I16(-45), &[ScaleStep::Offset(-40.0)]),
            Some(PlcValue::I16(-85)),
            "an INT with an integral offset"
        );
        assert_eq!(
            run(
                &PlcValue::I16(7),
                &[ScaleStep::Multiply(-2.0), ScaleStep::Offset(0.0)]
            ),
            Some(PlcValue::I16(-14)),
            "two integral steps"
        );
        assert_eq!(
            run(&PlcValue::U64(u64::MAX), &[ScaleStep::Multiply(1.0)]),
            Some(PlcValue::U64(u64::MAX)),
            "the top of the range"
        );
        assert_eq!(
            run(&PlcValue::I64(i64::MIN), &[ScaleStep::Multiply(1.0)]),
            Some(PlcValue::I64(i64::MIN)),
            "the bottom of the range"
        );
        assert_eq!(
            run(&PlcValue::I8(-128), &[ScaleStep::Offset(0.0)]),
            Some(PlcValue::I8(-128)),
            "a narrow type at its limit"
        );
    }

    #[test]
    fn an_integer_result_widens_when_it_does_not_fit_its_own_type() {
        assert_eq!(
            run(&PlcValue::I16(30_000), &[ScaleStep::Offset(10_000.0)]),
            Some(PlcValue::I64(40_000)),
            "past i16 becomes i64"
        );
        assert_eq!(
            run(&PlcValue::U8(3), &[ScaleStep::Offset(-5.0)]),
            Some(PlcValue::I64(-2)),
            "a negative result from an unsigned input"
        );
        assert_eq!(
            run(
                &PlcValue::U32(u32::MAX),
                &[ScaleStep::Multiply(4_294_967_296.0)]
            ),
            Some(PlcValue::U64(0xFFFF_FFFF_0000_0000)),
            "past i64 becomes u64"
        );
    }

    #[test]
    fn an_integer_step_out_of_range_is_computed_in_floating_point() {
        let above = float(run(&PlcValue::U64(u64::MAX), &[ScaleStep::Offset(1.0)]));
        assert!(close(above, pow2(64)), "got {above}");
        let below = float(run(&PlcValue::I64(i64::MIN), &[ScaleStep::Offset(-1.0)]));
        assert!(close(below, -pow2(63)), "got {below}");
        let product = float(run(&PlcValue::U64(u64::MAX), &[ScaleStep::Multiply(2.0)]));
        assert!(close(product, pow2(65)), "got {product}");
        let big = float(run(&PlcValue::U8(2), &[ScaleStep::Multiply(1e30)]));
        assert!(
            close(big, 2e30),
            "a constant beyond 2^64 is a float step, got {big}"
        );
    }

    #[test]
    fn a_fractional_constant_and_divide_make_a_float_step() {
        let half = float(run(&PlcValue::U16(3), &[ScaleStep::Offset(0.5)]));
        assert!(close(half, 3.5), "got {half}");
        let exact = float(run(&PlcValue::U16(100), &[ScaleStep::Divide(10.0)]));
        assert!(
            close(exact, 10.0),
            "an exact quotient is still a float, got {exact}"
        );
        let after = float(run(
            &PlcValue::U16(100),
            &[ScaleStep::Multiply(0.5), ScaleStep::Multiply(2.0)],
        ));
        assert!(close(after, 100.0), "a float stays a float, got {after}");
        let single = float(run(&PlcValue::F32(1.5), &[ScaleStep::Multiply(2.0)]));
        assert!(close(single, 3.0), "F32 in, F64 out, got {single}");
    }

    #[test]
    fn unscaled_values_pass_through_unchanged() {
        for value in [
            PlcValue::F32(1.5),
            PlcValue::U16(7),
            PlcValue::Bool(true),
            PlcValue::Null,
            PlcValue::Char('x'),
            PlcValue::String("text".to_owned()),
            PlcValue::Duration(Duration::from_secs(1)),
        ] {
            assert_eq!(run(&value, &[]), Some(value.clone()), "{value:?}");
        }
    }

    #[test]
    fn a_value_that_is_not_a_number_cannot_be_scaled() {
        for value in [
            PlcValue::Bool(true),
            PlcValue::Null,
            PlcValue::Char('x'),
            PlcValue::String("1".to_owned()),
            PlcValue::Duration(Duration::from_secs(1)),
        ] {
            assert_eq!(run(&value, &[ScaleStep::Multiply(1.0)]), None, "{value:?}");
        }
    }

    #[test]
    fn clamp_on_integers_compares_as_integers() {
        let step = ScaleStep::Clamp {
            min: 0.0,
            max: 100.0,
        };
        assert_eq!(
            run(&PlcValue::I16(150), &[step]),
            Some(PlcValue::I16(100)),
            "above"
        );
        assert_eq!(
            run(&PlcValue::I16(-5), &[step]),
            Some(PlcValue::I16(0)),
            "below"
        );
        assert_eq!(
            run(&PlcValue::I16(42), &[step]),
            Some(PlcValue::I16(42)),
            "inside"
        );
        let wide = ScaleStep::Clamp {
            min: 10_000_000_000_000_000_000.0,
            max: 15_000_000_000_000_000_000.0,
        };
        assert_eq!(
            run(&PlcValue::U8(5), &[wide]),
            Some(PlcValue::U64(10_000_000_000_000_000_000)),
            "a bound beyond i64 widens to u64"
        );
    }

    #[test]
    fn clamp_with_a_fractional_or_out_of_range_bound_is_a_float_step() {
        let fractional = float(run(
            &PlcValue::I16(7),
            &[ScaleStep::Clamp { min: 0.0, max: 5.5 }],
        ));
        assert!(close(fractional, 5.5), "got {fractional}");
        let below_range = float(run(
            &PlcValue::I16(5),
            &[ScaleStep::Clamp {
                min: -1e19,
                max: -9.5e18,
            }],
        ));
        assert!(close(below_range, -9.5e18), "got {below_range}");
        let on_floats = float(run(
            &PlcValue::F64(5.0),
            &[ScaleStep::Clamp { min: 0.0, max: 3.0 }],
        ));
        assert!(close(on_floats, 3.0), "got {on_floats}");
        let single = float(run(
            &PlcValue::F32(-5.0),
            &[ScaleStep::Clamp {
                min: -1.0,
                max: 3.0,
            }],
        ));
        assert!(close(single, -1.0), "got {single}");
    }

    #[test]
    fn an_unevaluable_clamp_is_bad() {
        for (min, max) in [
            (5.0, 1.0),
            (f64::NAN, 1.0),
            (0.0, f64::NAN),
            (f64::NEG_INFINITY, 1.0),
            (0.0, f64::INFINITY),
        ] {
            let step = ScaleStep::Clamp { min, max };
            assert_eq!(run(&PlcValue::I16(1), &[step]), None, "int {min} {max}");
            assert_eq!(run(&PlcValue::F64(1.0), &[step]), None, "float {min} {max}");
        }
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn a_counter_wraps_through_a_sixteen_bit_field() {
        let field = [ScaleStep::BitField {
            shift: 0,
            width: 16,
        }];
        assert_eq!(
            run(&PlcValue::U32(0x0001_FFFF), &field),
            Some(PlcValue::U32(0xFFFF)),
            "before the rollover"
        );
        assert_eq!(
            run(&PlcValue::U32(0x0002_0000), &field),
            Some(PlcValue::U32(0)),
            "after the rollover"
        );
    }

    #[test]
    fn bit_fields_read_the_sixty_four_bit_pattern() {
        assert_eq!(
            run(
                &PlcValue::U8(0b0011_0000),
                &[ScaleStep::BitField { shift: 4, width: 2 }]
            ),
            Some(PlcValue::U8(3)),
            "a middle field"
        );
        assert_eq!(
            run(
                &PlcValue::I16(-1),
                &[ScaleStep::BitField {
                    shift: 0,
                    width: 16
                }]
            ),
            Some(PlcValue::I64(65_535)),
            "a negative value is sign-extended and the field widens"
        );
        assert_eq!(
            run(
                &PlcValue::I16(-1),
                &[ScaleStep::BitField {
                    shift: 0,
                    width: 64
                }]
            ),
            Some(PlcValue::U64(u64::MAX)),
            "all sixty-four bits"
        );
        assert_eq!(
            run(
                &PlcValue::U64(1 << 63),
                &[ScaleStep::BitField {
                    shift: 63,
                    width: 1
                }]
            ),
            Some(PlcValue::U64(1)),
            "the top bit"
        );
        assert_eq!(
            run(
                &PlcValue::U32(0xABCD),
                &[
                    ScaleStep::BitField { shift: 4, width: 8 },
                    ScaleStep::Multiply(2.0)
                ]
            ),
            Some(PlcValue::U32(0xBC * 2)),
            "a field then an integral multiply"
        );
    }

    #[test]
    fn an_unevaluable_bit_field_is_bad() {
        let bad = [
            ScaleStep::BitField { shift: 0, width: 0 },
            ScaleStep::BitField {
                shift: 60,
                width: 5,
            },
            ScaleStep::BitField {
                shift: 255,
                width: 2,
            },
            ScaleStep::BitField {
                shift: 1,
                width: 64,
            },
        ];
        for step in bad {
            assert_eq!(run(&PlcValue::U64(u64::MAX), &[step]), None, "{step:?}");
        }
        assert_eq!(
            run(
                &PlcValue::F64(3.0),
                &[ScaleStep::BitField { shift: 0, width: 1 }]
            ),
            None,
            "a float has no bits"
        );
        assert_eq!(
            run(
                &PlcValue::U64(u64::MAX),
                &[
                    ScaleStep::Offset(1.0),
                    ScaleStep::BitField { shift: 0, width: 1 }
                ]
            ),
            None,
            "a value promoted to float by an earlier step has no bits"
        );
    }

    #[test]
    fn integral_recognises_exact_integers_below_two_to_the_sixty_four() {
        for (constant, expected) in [
            (0.0, Some(0)),
            (-0.0, Some(0)),
            (1.0, Some(1)),
            (-40.0, Some(-40)),
            (3.0, Some(3)),
            (4_503_599_627_370_496.0, Some(4_503_599_627_370_496)),
            (pow2(63), Some(9_223_372_036_854_775_808)),
            (-pow2(63), Some(-9_223_372_036_854_775_808)),
            (1e19, Some(10_000_000_000_000_000_000)),
            (pow2(64) - 2048.0, Some(18_446_744_073_709_549_568)),
            (pow2(64), None),
            (1e300, None),
            (0.5, None),
            (-1.5, None),
            (1e-300, None),
            (5e-324, None),
            (f64::NAN, None),
            (f64::INFINITY, None),
            (f64::NEG_INFINITY, None),
        ] {
            assert_eq!(integral(constant), expected, "{constant:e}");
        }
    }

    #[test]
    fn integers_convert_to_floats_with_round_to_nearest_even() {
        for (value, expected) in [
            (0_u64, 0.0_f64),
            (1, 1.0),
            (u64::from(u32::MAX), 4_294_967_295.0),
            (1 << 53, 9_007_199_254_740_992.0),
            ((1 << 53) + 1, 9_007_199_254_740_992.0),
            ((1 << 53) + 3, 9_007_199_254_740_996.0),
            (u64::MAX, pow2(64)),
        ] {
            assert_eq!(u64_to_f64(value).to_bits(), expected.to_bits(), "{value}");
        }
        for value in [0_u32, 1, 7, 65_535, 1 << 31, u32::MAX] {
            assert_eq!(
                u64_to_f64(u64::from(value)).to_bits(),
                f64::from(value).to_bits(),
                "{value}"
            );
        }
        assert_eq!(
            Num::Int(i128::from(i64::MIN)).to_f64().map(f64::to_bits),
            Some((-pow2(63)).to_bits()),
            "i64::MIN"
        );
        assert_eq!(
            Num::Int(i128::from(u64::MAX) + 1).to_f64(),
            None,
            "outside the invariant range"
        );
        assert_eq!(
            Num::Int(-i128::from(u64::MAX) - 1).to_f64(),
            None,
            "below the invariant range"
        );
    }

    #[test]
    fn numbers_read_back_as_floats_for_the_range_check() {
        assert_eq!(
            as_f64(&PlcValue::I16(-3)).map(f64::to_bits),
            Some((-3.0_f64).to_bits())
        );
        assert_eq!(
            as_f64(&PlcValue::F32(0.5)).map(f64::to_bits),
            Some(0.5_f64.to_bits())
        );
        assert_eq!(as_f64(&PlcValue::Bool(true)), None, "not a number");
    }

    #[test]
    fn status_bits_come_from_bools_and_integer_patterns() {
        assert_eq!(status_bit(&PlcValue::Bool(true), 0), Some(true), "bool set");
        assert_eq!(
            status_bit(&PlcValue::Bool(false), 5),
            Some(false),
            "bool clear"
        );
        assert_eq!(
            status_bit(&PlcValue::U16(0b1000), 3),
            Some(true),
            "bit 3 set"
        );
        assert_eq!(
            status_bit(&PlcValue::U16(0b1000), 2),
            Some(false),
            "bit 2 clear"
        );
        assert_eq!(
            status_bit(&PlcValue::I16(-1), 63),
            Some(true),
            "sign extension"
        );
        assert_eq!(
            status_bit(&PlcValue::U64(1 << 63), 63),
            Some(true),
            "top bit"
        );
        assert_eq!(status_bit(&PlcValue::U8(255), 64), None, "no bit 64");
        assert_eq!(
            status_bit(&PlcValue::F32(1.0), 0),
            None,
            "a float has no bits"
        );
        assert_eq!(status_bit(&PlcValue::Null, 0), None, "null has no bits");
    }
}
