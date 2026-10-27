//! The facade `modbus` feature re-exports the Modbus TCP driver (slice S26).
//!
//! Nothing here opens a socket: the test parses a configuration and an address through
//! `stonecrab::modbus::ModbusTcpDriver` and checks that the registry accepts the driver.
#![cfg(feature = "modbus")]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use stonecrab::modbus::{ModbusOptions, ModbusTcpDriver, SystemClock, UnitId};
    use stonecrab::{Driver, DriverRegistry, Endpoint, ErrorKind, PlcError};

    #[test]
    fn the_modbus_module_parses_a_configuration_and_an_address() -> Result<(), PlcError> {
        let driver = ModbusTcpDriver::new();
        assert_eq!(driver.scheme(), "modbus-tcp");

        let config = driver.parse_config(
            "modbus-tcp://192.0.2.10:1502?default-unit-identifier=7&address-convention=plc4x",
        )?;
        assert_eq!(
            config.endpoint(),
            &Endpoint::Tcp {
                host: "192.0.2.10".to_owned(),
                port: 1502
            }
        );
        let options = config
            .driver_options_as::<ModbusOptions>()
            .ok_or_else(|| PlcError::new(ErrorKind::Internal, "the options are missing"))?;
        assert_eq!(options.unit_id, UnitId::from(7));

        // The options are taken before `connect` consumes the configuration.
        let address_options = config.address_options();
        let address = driver.parse_address("40016:INT", &address_options)?;
        assert_eq!(address.canonical(), "holding-register:16:INT");
        let bare_without_convention =
            driver.parse_address("40016:INT", &stonecrab::AddressOptions::default());
        assert_eq!(
            bare_without_convention.err().map(|error| error.kind()),
            Some(ErrorKind::InvalidAddress)
        );
        Ok(())
    }

    #[test]
    fn the_driver_registers_and_the_clock_is_reachable() -> Result<(), PlcError> {
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(ModbusTcpDriver::new()))?;
        assert_eq!(registry.schemes(), ["modbus-tcp"]);
        let _clock: SystemClock = SystemClock::default();
        Ok(())
    }
}
