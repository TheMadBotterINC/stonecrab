use stonecrab_core::{ErrorKind, PlcError, PlcValue, PlcValueType, Result};

/// Multiplier of the xorshift64* output scramble (Vigna, "An experimental exploration of
/// Marsaglia's xorshift generators, scrambled").
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// State used when a caller seeds with zero, which would otherwise stay zero forever.
const ZERO_SEED_REPLACEMENT: u64 = 0x9E37_79B9_7F4A_7C15;

/// 2^16 and 2^32, the divisors that map 16 and 32 random bits onto `[0, 1)`.
const F32_DIVISOR: f32 = 65_536.0;
const F64_DIVISOR: f64 = 4_294_967_296.0;

/// A seeded xorshift64* generator behind the simulator's `RANDOM` area.
///
/// Upstream draws from `SecureRandom`, which cannot be reproduced; the simulator seeds a
/// small deterministic generator instead so that a test can state the exact values it
/// expects. It is not cryptographic. All arithmetic is shifts, xors and one
/// `wrapping_mul`, so no input can overflow it.
#[derive(Debug, Clone)]
pub(super) struct XorShift {
    state: u64,
}

impl XorShift {
    /// A generator with a fixed seed; zero is replaced by a fixed nonzero constant.
    pub(super) const fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                ZERO_SEED_REPLACEMENT
            } else {
                seed
            },
        }
    }

    /// One xorshift64* step; the state advances by the unscrambled xorshift.
    pub(super) const fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(MULTIPLIER)
    }

    /// The next value of the requested scalar type, built from the low bytes of one step.
    ///
    /// Integers take the low bytes of the draw (so every bit pattern is reachable), a
    /// Boolean takes the lowest bit, and floats are uniform over `[0, 1)` with 16 (`F32`)
    /// or 32 (`F64`) bits of resolution, so they are always finite.
    ///
    /// # Errors
    /// `ErrorKind::InvalidConfig` for `Null`, `Char`, `String`, `Duration` and any scalar
    /// type added to core later: the `RANDOM` area generates the eleven scalar types of the
    /// address grammar only. The draw is still consumed, so the sequence for the other
    /// tags does not depend on whether a refused type was requested.
    pub(super) fn value(&mut self, value_type: PlcValueType) -> Result<PlcValue> {
        let bytes = self.next_u64().to_le_bytes();
        let [b0, b1, b2, b3, b4, b5, b6, b7] = bytes;
        match value_type {
            PlcValueType::Bool => Ok(PlcValue::Bool(b0 & 1 == 1)),
            PlcValueType::I8 => Ok(PlcValue::I8(i8::from_le_bytes([b0]))),
            PlcValueType::I16 => Ok(PlcValue::I16(i16::from_le_bytes([b0, b1]))),
            PlcValueType::I32 => Ok(PlcValue::I32(i32::from_le_bytes([b0, b1, b2, b3]))),
            PlcValueType::I64 => Ok(PlcValue::I64(i64::from_le_bytes(bytes))),
            PlcValueType::U8 => Ok(PlcValue::U8(b0)),
            PlcValueType::U16 => Ok(PlcValue::U16(u16::from_le_bytes([b0, b1]))),
            PlcValueType::U32 => Ok(PlcValue::U32(u32::from_le_bytes([b0, b1, b2, b3]))),
            PlcValueType::U64 => Ok(PlcValue::U64(u64::from_le_bytes([
                b0, b1, b2, b3, b4, b5, b6, b7,
            ]))),
            PlcValueType::F32 => Ok(PlcValue::F32(
                f32::from(u16::from_le_bytes([b0, b1])) / F32_DIVISOR,
            )),
            PlcValueType::F64 => Ok(PlcValue::F64(
                f64::from(u32::from_le_bytes([b0, b1, b2, b3])) / F64_DIVISOR,
            )),
            // Every other known variant and the `_` for variants core adds later share one
            // arm: clippy::wildcard_enum_match_arm accepts a `_` that matches no known
            // variant on a foreign #[non_exhaustive] enum (ADR-0005 addendum, ADR-0012).
            PlcValueType::Null
            | PlcValueType::Char
            | PlcValueType::String
            | PlcValueType::Duration
            | _ => Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "RANDOM generates only the scalar types bool, i8..i64, u8..u64, f32 and f64",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCALARS: [PlcValueType; 11] = [
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
    ];

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn the_sequence_for_a_fixed_seed_is_pinned() {
        // Expected draws were computed with an independent Python implementation of
        // xorshift64* (shifts 12, 25, 27; multiplier 0x2545F4914F6CDD1D).
        let mut rng = XorShift::new(1);
        assert_eq!(rng.next_u64(), 0x47e4_ce4b_896c_dd1d, "seed 1, draw 1");
        assert_eq!(rng.next_u64(), 0xabcf_a6a8_e079_651d, "seed 1, draw 2");
        assert_eq!(rng.next_u64(), 0xb9d1_0d8f_eb73_1f57, "seed 1, draw 3");
        let mut other = XorShift::new(0xDEAD_BEEF);
        assert_eq!(
            other.next_u64(),
            0x4615_1251_b681_bada,
            "seed 0xDEADBEEF, draw 1"
        );
        assert_eq!(
            other.next_u64(),
            0x7db2_11d8_263e_f2a6,
            "seed 0xDEADBEEF, draw 2"
        );
    }

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn a_zero_seed_is_replaced_by_a_nonzero_constant() {
        let mut zero = XorShift::new(0);
        assert_eq!(zero.next_u64(), 0x0d83_b3e2_9a21_487a, "seed 0, draw 1");
        assert_eq!(zero.next_u64(), 0x54c4_4c79_f1fe_9d67, "seed 0, draw 2");
        let mut replacement = XorShift::new(ZERO_SEED_REPLACEMENT);
        let mut again = XorShift::new(0);
        assert_eq!(
            replacement.next_u64(),
            again.next_u64(),
            "zero seeds the replacement constant"
        );
    }

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn each_scalar_type_is_built_from_the_low_bytes_of_one_draw() {
        // Seed 1, first draw 0x47e4ce4b896cdd1d; little-endian bytes 1d dd 6c 89 4b ce e4 47.
        let draw = |value_type| XorShift::new(1).value(value_type);
        assert_eq!(
            draw(PlcValueType::Bool).ok(),
            Some(PlcValue::Bool(true)),
            "bool: low bit"
        );
        assert_eq!(draw(PlcValueType::I8).ok(), Some(PlcValue::I8(29)), "i8");
        assert_eq!(
            draw(PlcValueType::I16).ok(),
            Some(PlcValue::I16(-8_931)),
            "i16"
        );
        assert_eq!(
            draw(PlcValueType::I32).ok(),
            Some(PlcValue::I32(-1_989_354_211)),
            "i32"
        );
        assert_eq!(
            draw(PlcValueType::I64).ok(),
            Some(PlcValue::I64(5_180_492_295_206_395_165)),
            "i64"
        );
        assert_eq!(draw(PlcValueType::U8).ok(), Some(PlcValue::U8(29)), "u8");
        assert_eq!(
            draw(PlcValueType::U16).ok(),
            Some(PlcValue::U16(56_605)),
            "u16"
        );
        assert_eq!(
            draw(PlcValueType::U32).ok(),
            Some(PlcValue::U32(2_305_613_085)),
            "u32"
        );
        assert_eq!(
            draw(PlcValueType::U64).ok(),
            Some(PlcValue::U64(0x47e4_ce4b_896c_dd1d)),
            "u64"
        );
        assert_eq!(
            draw(PlcValueType::F32).ok().and_then(|v| v.as_f64()),
            Some(0.863_723_754_882_812_5),
            "f32 is 56605 / 65536 and exactly representable"
        );
        let f64_bits = draw(PlcValueType::F64)
            .ok()
            .and_then(|v| v.as_f64())
            .map(f64::to_bits);
        assert_eq!(
            f64_bits,
            Some((2_305_613_085.0_f64 / 4_294_967_296.0).to_bits()),
            "f64 is 2305613085 / 2^32"
        );
    }

    #[test]
    fn a_false_boolean_comes_from_a_clear_low_bit() {
        let mut rng = XorShift::new(1);
        // Draw 2 is 0xabcfa6a8e079651d: low byte 0x1d, odd; draw 3 low byte 0x57, odd; find
        // an even one by scanning, which also shows both outcomes occur.
        let seen: Vec<bool> = (0..64)
            .filter_map(|_| rng.value(PlcValueType::Bool).ok())
            .filter_map(|v| v.as_bool())
            .collect();
        assert!(
            seen.contains(&true) && seen.contains(&false),
            "both Boolean outcomes occur"
        );
    }

    #[test]
    fn every_scalar_type_yields_its_own_type_and_floats_are_in_the_unit_interval() {
        let mut rng = XorShift::new(42);
        for value_type in SCALARS {
            for _ in 0..32 {
                let value = rng.value(value_type);
                assert_eq!(
                    value.as_ref().map(PlcValue::value_type).ok(),
                    Some(value_type),
                    "requested and produced types agree"
                );
                if let Some(float) = value.ok().and_then(|v| v.as_f64()) {
                    assert!((0.0..1.0).contains(&float), "floats lie in [0, 1)");
                }
            }
        }
    }

    // covers: SIM-RANDOM-SEEDED
    #[test]
    fn non_scalar_types_are_refused_but_still_consume_a_draw() {
        for refused in [
            PlcValueType::Null,
            PlcValueType::Char,
            PlcValueType::String,
            PlcValueType::Duration,
        ] {
            let mut rng = XorShift::new(1);
            let error = rng.value(refused).err().map(|e| e.kind());
            assert_eq!(error, Some(ErrorKind::InvalidConfig), "type is refused");
            assert_eq!(
                rng.next_u64(),
                0xabcf_a6a8_e079_651d,
                "the refused request consumed exactly one draw"
            );
        }
    }
}
