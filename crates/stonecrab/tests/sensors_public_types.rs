//! Thread-safety contract of the public types (section 9.11 of the coding standards).
#![cfg(feature = "sensors")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use stonecrab::sensors::{
        Acquisition, ByteOrderName, ConnectionRequirements, DeviceApplicability, Measurement,
        MeasurementDef, ProfileError, ProfileId, Provenance, Quality, QualityMapping, ScaleStep,
        SchemaVersion, SensorProfile, StaleTracker, StatusField, Unit, ValidationStatus,
        ValueRange, WordOrderName,
    };

    #[test]
    fn public_types_are_send_sync_and_static() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<ProfileId>();
        assert_send_sync::<SchemaVersion>();
        assert_send_sync::<SensorProfile>();
        assert_send_sync::<DeviceApplicability>();
        assert_send_sync::<ConnectionRequirements>();
        assert_send_sync::<MeasurementDef>();
        assert_send_sync::<Measurement>();
        assert_send_sync::<Quality>();
        assert_send_sync::<ByteOrderName>();
        assert_send_sync::<WordOrderName>();
        assert_send_sync::<ScaleStep>();
        assert_send_sync::<ValueRange>();
        assert_send_sync::<StatusField>();
        assert_send_sync::<QualityMapping>();
        assert_send_sync::<Provenance>();
        assert_send_sync::<ValidationStatus>();
        assert_send_sync::<ProfileError>();
        assert_send_sync::<Unit>();
        assert_send_sync::<StaleTracker>();
        assert_send_sync::<Acquisition<'static>>();
    }
}
