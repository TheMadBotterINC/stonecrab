//! The entry points of `fuzz_opcua_connection_string`, compiled only under cargo-fuzz's
//! `--cfg fuzzing`: the connection string parser is crate-private until slice S91, and the
//! fuzz workspace can reach only public items.

use crate::opcua::driver::OpcuaDriver;
use crate::opcua::options::OpcuaOptions;
use crate::{ConnectionConfig, Result};

/// What the fuzz target checks of an accepted connection string.
#[derive(Debug)]
pub struct Parsed {
    /// The configuration `parse_config` built.
    pub config: ConnectionConfig,
    /// The endpoint URL Hello would carry.
    pub endpoint_url: String,
    /// The name `security-policy` takes for the parsed policy.
    pub security_policy: &'static str,
    /// The name `message-security` takes for the parsed mode.
    pub message_security: &'static str,
}

/// Parse an `opcua` connection string as the driver does and report what was attached.
///
/// # Errors
/// As `OpcuaDriver::parse_config`: [`ErrorKind::InvalidConfig`](crate::ErrorKind) for text
/// that is not a valid `opcua` connection string.
pub fn parse_config(uri: &str) -> Result<Parsed> {
    let config = OpcuaDriver::parse_config(uri)?;
    let options = config.driver_options_as::<OpcuaOptions>().ok_or_else(|| {
        crate::PlcError::new(
            crate::ErrorKind::Internal,
            "parse_config attaches no options",
        )
    })?;
    let (endpoint_url, security_policy, message_security) = (
        options.endpoint_url.clone(),
        options.security_policy.name(),
        options.message_security.name(),
    );
    Ok(Parsed {
        config,
        endpoint_url,
        security_policy,
        message_security,
    })
}
