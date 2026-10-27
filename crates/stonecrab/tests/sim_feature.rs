//! The facade `sim` feature re-exports the simulator driver (slice S14).
#![cfg(feature = "sim")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use stonecrab::sim::{SimDevice, SimDriver, SimTag};
    use stonecrab::{
        AddressOptions, ConnectionState, Deadline, DriverRegistry, PlcError, PlcValue, ReadRequest,
        Tag, TagId,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn a_sim_device_is_readable_through_the_facade_module() -> Result<(), PlcError> {
        let device = SimDevice::new();
        device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(SimDriver::new().with_device("bench", device)?))?;

        let connection = registry.connect("sim://bench").await?;
        assert_eq!(connection.state(), ConnectionState::Connected, "connected");

        let driver = registry.driver("sim").ok_or_else(|| {
            PlcError::new(stonecrab::ErrorKind::InvalidConfig, "sim driver missing")
        })?;
        let address = driver.parse_address("STATE/boiler:f32", &AddressOptions::default())?;
        let now = Instant::now();
        let request = ReadRequest::builder(connection.limits())
            .tag(Tag::new(TagId::new("boiler")?, address))
            .deadline(Deadline::after(now, Duration::from_millis(500))?)
            .build()?;
        let response = connection.read(request).await?;
        let value = response
            .items()
            .first()
            .and_then(|item| item.outcome().as_ref().ok());
        assert_eq!(
            value,
            Some(&PlcValue::F32(71.5)),
            "value served by the simulator"
        );
        connection.close().await
    }
}
