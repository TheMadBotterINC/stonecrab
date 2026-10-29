//! The facade `sensors` feature re-exports the sensor profile crate (slice S30).
//!
//! The test loads one repository profile through `stonecrab::sensors` and validates it;
//! nothing here opens a socket or needs a simulator.
#![cfg(feature = "sensors")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::sensors::{ProfileError, SensorProfile, Unit, ValidationStatus};

    /// The repository's synthetic simulator temperature profile (repository data, not shipped).
    const PROFILE: &str =
        include_str!("../../../profiles/synthetic/sim/temperature_transmitter.toml");

    #[test]
    fn a_repository_profile_loads_and_validates_through_the_facade_module()
    -> Result<(), ProfileError> {
        let profile = SensorProfile::from_toml(PROFILE)?;
        assert_eq!(profile.validation, ValidationStatus::Synthetic);
        assert_eq!(profile.connection.scheme, "sim");
        assert!(
            !profile.measurements.is_empty(),
            "the profile defines measurements"
        );
        assert!(
            profile
                .measurements
                .iter()
                .all(|m| m.unit.as_deref().is_some_and(Unit::is_known)),
            "every unit is in the fixed table"
        );
        assert_eq!(profile.validate(), Ok(()));
        Ok(())
    }
}
