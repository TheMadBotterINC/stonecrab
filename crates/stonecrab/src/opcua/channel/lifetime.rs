//! Monotonic token scheduling, independently bounded receive grace and session expiry.

//! # Compatibility: docs/parity/opcua.md#OPCUA-LIFE-RENEWAL
//! # Compatibility: docs/parity/opcua.md#OPCUA-QUIRK-SHARED-SCHEDULER

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub(in crate::opcua) struct Lifetime {
    issued: Instant,
    valid: Duration,
    renew: Duration,
}

impl Lifetime {
    pub(in crate::opcua) fn new(
        requested: u32,
        revised: u32,
        minimum: u32,
        issued: Instant,
    ) -> Self {
        let valid = if revised == 0 || revised > requested {
            requested
        } else {
            revised
        };
        let minimum = minimum.min(requested);
        let effective = valid.max(minimum);
        if valid < minimum {
            tracing::warn!("OPC UA revised channel lifetime is below min-channel-lifetime-ms");
        }
        Self {
            issued,
            valid: Duration::from_millis(u64::from(valid)),
            renew: Duration::from_millis(u64::from(effective).saturating_mul(3).div_ceil(4)),
        }
    }

    pub(in crate::opcua) fn due(self, now: Instant) -> bool {
        now.saturating_duration_since(self.issued) >= self.renew
    }

    pub(in crate::opcua) fn expired(self, now: Instant) -> bool {
        now.saturating_duration_since(self.issued) >= self.valid
    }

    pub(in crate::opcua) fn accepts_previous(self, now: Instant) -> bool {
        now.saturating_duration_since(self.issued)
            < self
                .valid
                .saturating_add(self.valid.checked_div(4).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcua_effective_lifetime_ignores_zero_and_larger_revisions() {
        let now = Instant::now();
        for revised in [0, 1001, u32::MAX] {
            let lifetime = Lifetime::new(1000, revised, 500, now);
            assert!(!lifetime.due(now + Duration::from_millis(749)));
            assert!(lifetime.due(now + Duration::from_millis(750)));
            assert!(!lifetime.expired(now + Duration::from_millis(999)));
            assert!(lifetime.expired(now + Duration::from_millis(1000)));
        }
    }

    #[test]
    fn opcua_a_lifetime_below_the_minimum_is_raised_without_extending_token_validity() {
        let now = Instant::now();
        let lifetime = Lifetime::new(10_000, 1000, 5000, now);
        assert!(!lifetime.due(now + Duration::from_millis(3749)));
        assert!(lifetime.due(now + Duration::from_millis(3750)));
        assert!(lifetime.expired(now + Duration::from_millis(1000)));
        assert!(lifetime.accepts_previous(now + Duration::from_millis(1249)));
        assert!(!lifetime.accepts_previous(now + Duration::from_millis(1250)));
        let short = Lifetime::new(1000, 500, 5000, now);
        assert!(!short.due(now + Duration::from_millis(749)));
        assert!(short.due(now + Duration::from_millis(750)));
    }

    #[test]
    fn opcua_renewal_intervals_round_up_without_float_loss_or_zero() {
        let now = Instant::now();
        for (millis, expected) in [(1, 1), (2, 2), (3, 3), (4, 3), (u32::MAX, 3_221_225_472)] {
            let lifetime = Lifetime::new(millis, millis, 1, now);
            assert!(!lifetime.due(now + Duration::from_millis(expected - 1)));
            assert!(lifetime.due(now + Duration::from_millis(expected)));
        }
    }
}
