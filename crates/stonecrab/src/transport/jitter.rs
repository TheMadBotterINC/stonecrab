use std::hash::{BuildHasher, RandomState};

/// Multiplier of the xorshift64* output scramble (Vigna, "An experimental exploration of
/// Marsaglia's xorshift generators, scrambled").
const MULTIPLIER: u64 = 0x2545_F491_4F6C_DD1D;

/// State used when a caller seeds with zero, which would otherwise stay zero forever.
const ZERO_SEED_REPLACEMENT: u64 = 0x9E37_79B9_7F4A_7C15;

/// 2^32, the divisor that maps 32 random bits onto `[0, 1)`.
const UNIT_DIVISOR: f64 = 4_294_967_296.0;

/// A small xorshift64* generator for reconnect jitter samples.
///
/// [`ReconnectPolicy::next_delay`](crate::ReconnectPolicy::next_delay) is pure
/// and takes its jitter sample from the caller (decision D13). `Jitter` is that caller's
/// source: a few lines of integer arithmetic seeded from `std::hash::RandomState`, so the
/// workspace needs no random-number crate. It is not cryptographic and must never be used
/// for anything but spreading retries out in time.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::ReconnectPolicy;
/// use stonecrab::transport::Jitter;
///
/// let policy = ReconnectPolicy::default();
/// let mut jitter = Jitter::new();
/// let delay = policy.next_delay(0, jitter.next_unit());
/// // 250 ms with at most 20% jitter either way.
/// assert!(delay >= Duration::from_millis(200) && delay <= Duration::from_millis(300));
/// ```
#[derive(Debug, Clone)]
pub struct Jitter {
    state: u64,
}

impl Jitter {
    /// A generator seeded from the operating system's per-process random hash keys.
    ///
    /// Two generators made by separate calls start from unrelated states.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::transport::Jitter;
    /// let sample = Jitter::new().next_unit();
    /// assert!((0.0..1.0).contains(&sample));
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self::from_seed(RandomState::new().hash_one(0_u64))
    }

    /// A generator with a fixed seed, for reproducible tests.
    ///
    /// A zero seed is replaced by a fixed nonzero constant, because the xorshift state
    /// must never be zero.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::transport::Jitter;
    /// let mut a = Jitter::from_seed(7);
    /// let mut b = Jitter::from_seed(7);
    /// assert_eq!(a.next_unit().to_bits(), b.next_unit().to_bits());
    /// ```
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                ZERO_SEED_REPLACEMENT
            } else {
                seed
            },
        }
    }

    /// The next sample, uniformly distributed over `[0.0, 1.0)` with 32 bits of resolution.
    ///
    /// The result is never 1.0 and never NaN, so it is a valid `sample` for
    /// [`ReconnectPolicy::next_delay`](crate::ReconnectPolicy::next_delay).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::transport::Jitter;
    /// let mut jitter = Jitter::from_seed(1);
    /// for _ in 0..100 {
    ///     assert!((0.0..1.0).contains(&jitter.next_unit()));
    /// }
    /// ```
    pub fn next_unit(&mut self) -> f64 {
        let [a, b, c, d, ..] = self.next_u64().to_be_bytes();
        f64::from(u32::from_be_bytes([a, b, c, d])) / UNIT_DIVISOR
    }

    /// One xorshift64* step; the state advances by the unscrambled xorshift.
    const fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(MULTIPLIER)
    }
}

impl Default for Jitter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn samples_stay_in_the_half_open_unit_interval() {
        let mut jitter = Jitter::from_seed(0xDEAD_BEEF);
        for _ in 0..100_000 {
            let sample = jitter.next_unit();
            assert!((0.0..1.0).contains(&sample), "sample {sample} out of range");
        }
    }

    #[test]
    fn the_state_does_not_fall_into_a_short_cycle() {
        let mut jitter = Jitter::from_seed(12_345);
        let start = jitter.state;
        let mut seen = HashSet::new();
        for step in 0..100_000_u32 {
            jitter.next_unit();
            assert_ne!(
                jitter.state, start,
                "state returned to the seed at step {step}"
            );
            assert!(seen.insert(jitter.state), "state repeated at step {step}");
        }
    }

    #[test]
    fn samples_are_spread_over_the_interval() {
        let mut jitter = Jitter::from_seed(99);
        let samples: Vec<f64> = (0..100_000).map(|_| jitter.next_unit()).collect();
        for tenth in 1_u32..=10 {
            let bound = f64::from(tenth) / 10.0;
            let below = samples.iter().filter(|sample| **sample < bound).count();
            let expected = usize::try_from(tenth).expect("small") * 10_000;
            assert!(
                below.abs_diff(expected) < 1_000,
                "{below} of 100000 samples fall below {bound}, expected about {expected}"
            );
        }
    }

    #[test]
    fn a_zero_seed_is_replaced_and_the_same_seed_repeats() {
        let mut zero = Jitter::from_seed(0);
        assert_eq!(zero.state, ZERO_SEED_REPLACEMENT, "zero seed replaced");
        zero.next_unit();
        assert_ne!(
            zero.state, ZERO_SEED_REPLACEMENT,
            "the replacement seed advances"
        );
        let mut a = Jitter::from_seed(5);
        let mut b = Jitter::from_seed(5);
        for _ in 0..10 {
            assert_eq!(
                a.next_unit().to_bits(),
                b.next_unit().to_bits(),
                "same seed"
            );
        }
    }

    #[test]
    fn independently_seeded_generators_differ_and_are_never_zero() {
        let first = Jitter::new();
        let second = Jitter::default();
        assert_ne!(
            first.state, second.state,
            "RandomState seeds differ per call"
        );
        assert_ne!(first.state, 0, "the state is never zero");
    }
}
