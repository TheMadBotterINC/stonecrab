use std::sync::Arc;

use crate::{
    AddressOptions, Capabilities, Connection, ConnectionConfig, Driver, ErrorKind, OpFuture,
    Operation, PlcError, ProtocolAddress, Result,
};

use crate::testkit::FixtureAddress;

/// A driver double that makes the address convention it was handed observable.
///
/// `parse_address` returns a [`FixtureAddress`] whose canonical text is
/// `<convention>:<text>`, or `none:<text>` when the options carry no convention, so a
/// test sees exactly which [`AddressOptions`] a caller passed. Text that is empty is
/// rejected. Connecting is `Unsupported`; nothing is ever read.
///
/// # Examples
///
/// ```
/// use stonecrab::{AddressOptions, Driver};
/// use stonecrab::testkit::ConventionDriver;
///
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// let driver = ConventionDriver::new("demo");
/// let options = AddressOptions::builder().convention("plc4x").build()?;
/// assert_eq!(driver.parse_address("a", &options)?.canonical(), "plc4x:a");
/// assert_eq!(driver.parse_address("a", &AddressOptions::default())?.canonical(), "none:a");
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct ConventionDriver {
    scheme: &'static str,
}

impl ConventionDriver {
    /// A double that reports `scheme` as its URI scheme.
    #[must_use]
    pub const fn new(scheme: &'static str) -> Self {
        Self { scheme }
    }
}

impl Driver for ConventionDriver {
    fn scheme(&self) -> &'static str {
        self.scheme
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn parse_config(&self, _uri: &str) -> Result<ConnectionConfig> {
        Err(PlcError::new(
            ErrorKind::Unsupported(Operation::Connect),
            "the convention double has no configuration",
        ))
    }

    fn parse_address(
        &self,
        text: &str,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        if text.is_empty() {
            return Err(PlcError::new(ErrorKind::InvalidAddress, "empty address"));
        }
        let convention = options.convention().unwrap_or("none");
        Ok(Arc::new(FixtureAddress::new(
            &format!("{convention}:{text}"),
            None,
        )))
    }

    fn connect(&self, _config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        Box::pin(async {
            Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Connect),
                "the convention double never connects",
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Endpoint;
    use crate::testkit::block_on;

    #[test]
    fn the_convention_reaches_the_address_text() {
        let driver = ConventionDriver::new("demo");
        assert_eq!(driver.scheme(), "demo", "scheme");
        let options = AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("valid");
        let address = driver.parse_address("a", &options).expect("parses");
        assert_eq!(address.canonical(), "plc4x:a", "convention shown");
        let address = driver
            .parse_address("a", &AddressOptions::default())
            .expect("parses");
        assert_eq!(address.canonical(), "none:a", "no convention shown");
        assert!(
            driver
                .parse_address("", &AddressOptions::default())
                .is_err(),
            "empty text is rejected"
        );
    }

    #[test]
    fn nothing_connects_or_configures() {
        let driver = ConventionDriver::new("demo");
        assert_eq!(
            driver.capabilities(),
            Capabilities::default(),
            "capabilities"
        );
        assert!(driver.parse_config("demo://x").is_err(), "no config");
        let config = ConnectionConfig::builder("demo", Endpoint::Custom("x".to_owned()))
            .build()
            .expect("config");
        assert!(block_on(driver.connect(config)).is_err(), "no connection");
    }
}
