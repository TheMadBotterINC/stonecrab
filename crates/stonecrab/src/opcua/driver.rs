//! `OpcuaDriver`: the connection string and tag address parsing of the `opcua` scheme.
//!
//! Crate-private until slice S91 adds the connection and implements `Driver` for it: the two
//! functions take the parameters and return the result of `Driver::parse_config` and
//! `Driver::parse_address` without the receiver, so that slice only wraps them (the inherent
//! and the trait method then share a name).

use std::sync::Arc;

use crate::opcua::OpcuaAddress;
use crate::opcua::options::{OpcuaOptions, endpoint};
use crate::{
    AddressOptions, ConnectionConfig, ConnectionUri, ErrorKind, PlcError, ProtocolAddress, Result,
};

/// The connection-string scheme of this driver.
pub(super) const SCHEME: &str = "opcua";

/// The transport code of OPC UA over TCP (`opc.tcp`), the only one and the default.
const TCP: &str = "tcp";

/// The refusal of a transport code other than `tcp`: upstream supports only `tcp`, and the
/// `tls` and `https` codes its pattern mentions are not OPC UA transports here.
const TRANSPORTS: &str = "the only transport code of opcua is tcp";

/// The `opcua` driver as far as it exists: it parses connection strings and tag addresses.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-TRANSPORT-TCP
/// Protocol code `opcua` and transport `tcp`, implicit when omitted (`OPCUA-TRANSPORT-TCP`).
/// The `tls`, `tls-psk`, `https` and every other code is refused (`OPCUA-TRANSPORT-TLS`,
/// `OPCUA-TRANSPORT-HTTPS`, `OPCUA-QUIRK-INET-PATTERN`).
#[derive(Debug, Default)]
pub(super) struct OpcuaDriver;

impl OpcuaDriver {
    /// Read `opcua[:tcp]://host:port[/path][?parameters]` into a validated configuration.
    ///
    /// The text is parsed with [`ConnectionUri::parse`], which already refuses duplicate and
    /// empty keys. The configuration has an [`Endpoint::Tcp`](crate::Endpoint::Tcp), the
    /// core defaults for timeouts, reconnect, retry and limits, a read-only access policy and
    /// the [`OpcuaOptions`] in its driver options slot: the endpoint URL Hello carries
    /// (`opc.tcp://host:port` and the path, verbatim), `security-policy` and
    /// `message-security`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for text that is not a valid connection string,
    /// another scheme, a transport code other than `tcp`, an authority that is not
    /// `host:port[/path]` (an empty host, a missing, empty, non-decimal, zero or above 65535
    /// port, an IPv6 address without brackets, an endpoint URL over 4096 bytes), and a
    /// parameter that is unknown, a deprecated or unknown policy name, an unknown message
    /// security name or a secure policy with message security `NONE`. The messages never
    /// repeat the authority or a value; a parameter error names its key.
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-CFG-UNKNOWN
    /// Unknown keys are an error where upstream warns (`OPCUA-CFG-UNKNOWN`,
    /// `OPCUA-DIV-UNKNOWN-KEYS`), which includes the keys the framework fills
    /// (`OPCUA-CFG-FRAMEWORK`) and the legacy `insecure-certificate-verification`
    /// (`OPCUA-QUIRK-TLS-NAMES`). The port is required (`OPCUA-TRANSPORT-PORT`, decision E22)
    /// and the endpoint URL is built as upstream builds it (`OPCUA-TRANSPORT-ENDPOINT-URL`).
    /// `security-policy` and `message-security` are `OPCUA-CFG-SECURITY-POLICY` and
    /// `OPCUA-CFG-MESSAGE-SECURITY`.
    pub(super) fn parse_config(uri: &str) -> Result<ConnectionConfig> {
        let parsed = ConnectionUri::parse(uri)?;
        if parsed.scheme() != SCHEME {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                "not an opcua connection string",
            ));
        }
        if !matches!(parsed.transport(), None | Some(TCP)) {
            return Err(PlcError::new(ErrorKind::InvalidConfig, TRANSPORTS));
        }
        let (endpoint, endpoint_url) = endpoint(parsed.authority())?;
        let options = OpcuaOptions::from_parameters(endpoint_url, parsed.options())?;
        ConnectionConfig::builder(SCHEME, endpoint)
            .driver_options(Arc::new(options))
            .build()
    }

    /// Parse an OPC UA tag address; see [`OpcuaAddress::parse`] for the grammar.
    ///
    /// The address options carry nothing OPC UA reads: the grammar has no address convention.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] as [`OpcuaAddress::parse`] does.
    ///
    /// # Compatibility: docs/parity/opcua.md#OPCUA-ADDR-ROUND-TRIP
    /// Delegates to [`OpcuaAddress::parse`], whose grammar rows and divergences apply.
    pub(super) fn parse_address(
        text: &str,
        _options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(OpcuaAddress::parse(text)?))
    }
}

#[cfg(test)]
mod tests {
    use crate::Endpoint;
    use crate::opcua::options::{MessageSecurity, SecurityPolicy};

    use super::*;

    fn config_of(uri: &str) -> ConnectionConfig {
        OpcuaDriver::parse_config(uri).expect("a valid connection string")
    }

    fn options_of(config: &ConnectionConfig) -> &OpcuaOptions {
        config
            .driver_options_as::<OpcuaOptions>()
            .expect("parse_config attaches the options")
    }

    fn tcp(host: &str, port: u16) -> Endpoint {
        Endpoint::Tcp {
            host: host.to_owned(),
            port,
        }
    }

    fn refused(uri: &str) -> PlcError {
        let error =
            OpcuaDriver::parse_config(uri).expect_err("the connection string must be refused");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
        error
    }

    // covers: OPCUA-TRANSPORT-TCP the scheme opcua and the implicit or explicit tcp transport
    #[test]
    fn opcua_driver_accepts_tcp_transport_and_the_opcua_scheme() {
        let implicit = config_of("opcua://192.0.2.10:4840");
        let explicit = config_of("opcua:tcp://192.0.2.10:4840");
        assert_eq!(implicit.scheme(), "opcua");
        assert_eq!(implicit.endpoint(), &tcp("192.0.2.10", 4840));
        assert_eq!(explicit.endpoint(), implicit.endpoint());
        assert_eq!(options_of(&explicit), options_of(&implicit));
    }

    // covers: OPCUA-TRANSPORT-TCP / OPCUA-QUIRK-INET-PATTERN only tcp is a transport code
    #[test]
    fn opcua_driver_rejects_other_transport_codes() {
        for transport in [
            "tls", "tls-psk", "https", "http", "udp", "opc-tcp", "tcp2", "ssl",
        ] {
            let uri = format!("opcua:{transport}://host:4840");
            let error = refused(&uri);
            assert!(
                error.to_string().contains("only transport code"),
                "{uri}: {error}"
            );
        }
        assert_eq!(
            refused("opcua:://host:4840").kind(),
            ErrorKind::InvalidConfig,
            "an empty code is the core grammar's refusal"
        );
    }

    // covers: OPCUA-TRANSPORT-TLS the tls transport code is refused
    #[test]
    fn opcua_driver_rejects_the_tls_transport_code() {
        let error = refused("opcua:tls://host:4840?security-policy=NONE");
        assert!(error.to_string().contains("tcp"), "{error}");
    }

    // covers: OPCUA-TRANSPORT-HTTPS the https transport code is refused
    #[test]
    fn opcua_driver_rejects_the_https_transport_code() {
        refused("opcua:https://host:443");
        refused("opcua:http://host:80");
    }

    #[test]
    fn opcua_driver_rejects_another_scheme_and_malformed_text() {
        for uri in [
            "modbus-tcp://host:502",
            "opc.tcp://host:4840",
            "OPCUA://host:4840",
            "opcua2://host:4840",
            "sim://bench",
            "not a connection string",
            "opcua://",
            "opcua:TCP://host:4840",
            "opcua://host:4840?",
            "opcua://user:secret@host:4840",
            "opcua://host:4840/a%20b",
        ] {
            refused(uri);
        }
    }

    // covers: OPCUA-CFG-UNKNOWN a key written twice is refused by the core grammar
    #[test]
    fn opcua_config_duplicate_key_is_rejected() {
        for uri in [
            "opcua://host:4840?security-policy=NONE&security-policy=NONE",
            "opcua://host:4840?message-security=SIGN&security-policy=NONE&message-security=SIGN",
        ] {
            refused(uri);
        }
    }

    // covers: OPCUA-TRANSPORT-ENDPOINT-URL host, port and path reach the configuration
    #[test]
    fn opcua_parse_config_reads_host_port_and_endpoint_path() {
        let config =
            config_of("opcua://plc.example:48010/OPCUA/SimulationServer?security-policy=NONE");
        assert_eq!(config.endpoint(), &tcp("plc.example", 48010));
        assert_eq!(
            options_of(&config).endpoint_url,
            "opc.tcp://plc.example:48010/OPCUA/SimulationServer"
        );
        let v6 = config_of("opcua:tcp://[2001:db8::10]:4840/ua");
        assert_eq!(v6.endpoint(), &tcp("2001:db8::10", 4840));
        assert_eq!(
            options_of(&v6).endpoint_url,
            "opc.tcp://[2001:db8::10]:4840/ua"
        );
    }

    // covers: OPCUA-TRANSPORT-PORT the port is required, 4840 is not defaulted
    #[test]
    fn opcua_a_missing_port_is_invalid_config() {
        for uri in [
            "opcua://host",
            "opcua:tcp://host",
            "opcua://host/Server",
            "opcua://[::1]",
            "opcua://host?security-policy=NONE",
        ] {
            let error = refused(uri);
            assert!(
                error.to_string().contains("port is required"),
                "{uri}: {error}"
            );
        }
    }

    // covers: OPCUA-TRANSPORT-PORT the port is 1 to 65535
    #[test]
    fn opcua_driver_rejects_port_zero_and_65536() {
        for uri in [
            "opcua://host:0",
            "opcua://host:65536",
            "opcua://host:-1",
            "opcua://host:",
        ] {
            refused(uri);
        }
        assert_eq!(
            config_of("opcua://host:65535").endpoint(),
            &tcp("host", 65535)
        );
    }

    #[test]
    fn opcua_parse_config_carries_the_defaults_and_the_parameters() {
        let defaults = config_of("opcua://host:4840");
        let options = options_of(&defaults);
        assert_eq!(options.security_policy, SecurityPolicy::Basic256Sha256);
        assert_eq!(options.message_security, MessageSecurity::SignEncrypt);
        let plain = ConnectionConfig::builder(SCHEME, tcp("host", 4840))
            .build()
            .expect("defaults");
        assert_eq!(defaults.timeouts(), plain.timeouts());
        assert_eq!(defaults.limits(), plain.limits());
        assert_eq!(defaults.access(), plain.access());
        assert_eq!(defaults.address_options().convention(), None);

        let chosen = config_of(
            "opcua://host:4840?security-policy=Aes256_Sha256_RsaPss&message-security=SIGN",
        );
        assert_eq!(
            options_of(&chosen).security_policy,
            SecurityPolicy::Aes256Sha256RsaPss
        );
        assert_eq!(options_of(&chosen).message_security, MessageSecurity::Sign);
    }

    #[test]
    fn opcua_parse_config_refuses_bad_parameters_without_echoing_values() {
        for uri in [
            "opcua://host:4840?security-policy=Basic128Rsa15",
            "opcua://host:4840?security-policy=basic256sha256",
            "opcua://host:4840?message-security=NONE",
            "opcua://host:4840?message-security=sign",
            "opcua://host:4840?surprise=1",
            "opcua://host:4840?username=operator&password=hunter2",
        ] {
            let error = refused(uri);
            assert!(!error.to_string().contains("hunter2"), "{uri}: {error}");
            assert!(!error.to_string().contains("host"), "{uri}: {error}");
        }
    }

    #[test]
    fn opcua_the_redacted_display_of_a_configuration_is_one_clean_line() {
        let shown = config_of("opcua://host:4840/Server").redacted_display();
        assert!(!shown.chars().any(char::is_control), "{shown}");
        assert!(
            shown.starts_with("scheme=opcua endpoint=host:4840"),
            "{shown}"
        );
    }

    #[test]
    fn opcua_parse_address_reads_the_tag_grammar() {
        let options = AddressOptions::default();
        let address = OpcuaDriver::parse_address("ns=2;s=Boiler.Temperature;REAL", &options)
            .expect("an address");
        assert_eq!(address.protocol(), "opcua");
        assert_eq!(address.canonical(), "ns=2;s=Boiler.Temperature;REAL");
        let error = OpcuaDriver::parse_address("not an address", &options).expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
    }
}
