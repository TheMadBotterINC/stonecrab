//! OPC UA driver, with injectable clock and transport factory.

use std::fmt;
use std::sync::Arc;

use crate::opcua::OpcuaAddress;
use crate::opcua::channel::{Channel, within};
use crate::opcua::connection::OpcuaConnection;
use crate::opcua::lane::Lane;
use crate::opcua::options::SecurityPolicy;
use crate::opcua::options::{OpcuaOptions, endpoint};
use crate::opcua::session::Session;
use crate::transport::{SystemClock, TcpFactory, TransportFactory};
use crate::{
    AddressOptions, CancellationToken, Capabilities, Clock, Connection, ConnectionConfig,
    ConnectionUri, Deadline, Driver, Endpoint, ErrorKind, OpFuture, PlcError, ProtocolAddress,
    Result, Timeouts,
};

/// The connection-string scheme of this driver.
pub(super) const SCHEME: &str = "opcua";

/// The transport code of OPC UA over TCP (`opc.tcp`), the only one and the default.
const TCP: &str = "tcp";

/// The refusal of a transport code other than `tcp`: upstream supports only `tcp`, and the
/// `tls` and `https` codes its pattern mentions are not OPC UA transports here.
const TRANSPORTS: &str = "the only transport code of opcua is tcp";

/// Factory for OPC UA TCP connections.
///
/// This initial connection slice supports anonymous sessions with explicitly selected
/// `security-policy=NONE`. It sends HEL, OPN, `CreateSession` and `ActivateSession` before
/// returning Connected. The secure default is refused until protection is implemented;
/// credentials are also refused before I/O. Reads map scalar Boolean, integers, floats,
/// String/XML and Null; other operations remain unsupported.
///
/// The whole attempt uses `handshake-timeout-ms` (60000 by default), and close uses
/// `request-timeout-ms` (30000). `session-timeout-ms` requests 120000 by default; the
/// server's positive finite revision is adopted. System transport/clock require Tokio;
/// injected implementations can run without an executor.
///
/// # Compatibility: docs/parity/opcua.md#OPCUA-OP-CONNECT
/// # Compatibility: docs/parity/opcua.md#OPCUA-TRANSPORT-TCP
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab::{Driver, DriverRegistry};
/// use stonecrab::opcua::OpcuaDriver;
/// let driver = OpcuaDriver::new();
/// let config = driver.parse_config("opcua://localhost:4840?security-policy=NONE")?;
/// assert_eq!(config.scheme(), "opcua");
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(driver))?;
/// assert!(registry.driver("opcua").is_some());
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
pub struct OpcuaDriver {
    clock: Arc<dyn Clock>,
    factory: Arc<dyn TransportFactory>,
}

impl OpcuaDriver {
    /// Construct a driver using TCP and the Tokio-backed system clock; performs no I/O.
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock::default()),
            factory: Arc::new(TcpFactory::default()),
        }
    }

    /// Replace the clock. The transport factory must use the same time source.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace TCP with a transport factory, such as the deterministic testkit's.
    #[must_use]
    pub fn with_transport_factory(mut self, factory: Arc<dyn TransportFactory>) -> Self {
        self.factory = factory;
        self
    }

    /// Read `opcua[:tcp]://host:port[/path][?parameters]` into a validated configuration.
    ///
    /// The text is parsed with [`ConnectionUri::parse`], which already refuses duplicate and
    /// empty keys. The configuration has an [`Endpoint::Tcp`](crate::Endpoint::Tcp), the
    /// core defaults for idle timeout, reconnect, retry and limits, a read-only access policy and
    /// the [`OpcuaOptions`] in its driver options slot: the endpoint URL Hello carries
    /// (`opc.tcp://host:port` and the path, verbatim), `security-policy` and
    /// `message-security`. The request timeout is `request-timeout-ms` (30000 ms by default),
    /// `handshake-timeout-ms` maps to the whole connect bound (60000 ms by default),
    /// `session-timeout-ms` requests 120000 ms,
    /// `encoding.receive-buffer-size` and `encoding.send-buffer-size` select the chunk
    /// ceilings (65535 each by default), and `encoding.max-message-size` and
    /// `encoding.max-chunk-count` select whole-body limits (2097152 bytes and 64 chunks,
    /// additionally bounded by local resource budgets).
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
    /// # Compatibility: docs/parity/opcua.md#OPCUA-CFG-HANDSHAKE-TIMEOUT
    /// # Compatibility: docs/parity/opcua.md#OPCUA-CFG-SESSION-TIMEOUT
    /// # Compatibility: docs/parity/opcua.md#OPCUA-CFG-REQUEST-TIMEOUT
    /// # Compatibility: docs/parity/opcua.md#OPCUA-CFG-UNKNOWN
    /// Unknown keys are an error where upstream warns (`OPCUA-CFG-UNKNOWN`,
    /// `OPCUA-DIV-UNKNOWN-KEYS`), which includes the keys the framework fills
    /// (`OPCUA-CFG-FRAMEWORK`) and the legacy `insecure-certificate-verification`
    /// (`OPCUA-QUIRK-TLS-NAMES`). The port is required (`OPCUA-TRANSPORT-PORT`, decision E22)
    /// and the endpoint URL is built as upstream builds it (`OPCUA-TRANSPORT-ENDPOINT-URL`).
    /// `security-policy` and `message-security` are `OPCUA-CFG-SECURITY-POLICY` and
    /// `OPCUA-CFG-MESSAGE-SECURITY`.
    pub(super) fn configuration(uri: &str) -> Result<ConnectionConfig> {
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
        let timeouts = Timeouts {
            connect: options.handshake_timeout,
            request: options.request_timeout,
            ..Timeouts::default()
        };
        ConnectionConfig::builder(SCHEME, endpoint)
            .timeouts(timeouts)
            .access(options.access)
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
    pub(super) fn address(
        text: &str,
        _options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(OpcuaAddress::parse(text)?))
    }
}

impl Default for OpcuaDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for OpcuaDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpcuaDriver").finish_non_exhaustive()
    }
}

impl Driver for OpcuaDriver {
    fn scheme(&self) -> &'static str {
        SCHEME
    }
    fn capabilities(&self) -> Capabilities {
        crate::opcua::connection::capabilities()
    }
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        Self::configuration(uri)
    }
    fn parse_address(
        &self,
        text: &str,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Self::address(text, options)
    }
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        Box::pin(async move {
            if config.scheme() != SCHEME || !matches!(config.endpoint(), Endpoint::Tcp { .. }) {
                return Err(PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA requires its TCP configuration",
                ));
            }
            let options = config.driver_options_as::<OpcuaOptions>().ok_or_else(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA requires parsed driver options",
                )
            })?;
            if options.security_policy != SecurityPolicy::None {
                return Err(PlcError::new(
                    ErrorKind::InvalidConfig,
                    "OPC UA secure policy is not implemented; select None explicitly",
                ));
            }
            let identity = crate::opcua::identity::Identity::resolve(
                config.credentials().map(crate::Secret::expose),
                options,
            )?;
            let deadline = Deadline::after(self.clock.now(), config.timeouts().connect)?;
            let cancel = CancellationToken::new();
            let mut nonce = [0; 32];
            nonce_result(getrandom::fill(&mut nonce))?;
            let transport = within(
                self.clock.as_ref(),
                deadline,
                &cancel,
                self.factory
                    .open(config.endpoint(), *config.limits(), deadline),
            )
            .await?;
            let channel = Channel::open_none(
                transport,
                options,
                Arc::clone(&self.clock),
                deadline,
                &cancel,
            )
            .await?;
            let lane = Lane::from_channel(channel, config.timeouts().request);
            let session = Session::open(lane, options, identity, nonce, deadline, &cancel).await?;
            let connection: Box<dyn Connection> = Box::new(OpcuaConnection::new(
                session,
                Arc::clone(&self.clock),
                config.timeouts().request,
                *config.limits(),
                config.retry().clone(),
                config.access(),
            ));
            Ok(connection)
        })
    }
}

fn nonce_result(result: std::result::Result<(), getrandom::Error>) -> Result<()> {
    result
        .map_err(|_error| PlcError::new(ErrorKind::InvalidConfig, "OPC UA nonce generation failed"))
}

#[cfg(test)]
mod tests {
    use crate::Endpoint;
    use crate::opcua::options::{MessageSecurity, SecurityPolicy};

    use super::*;

    fn config_of(uri: &str) -> ConnectionConfig {
        OpcuaDriver::configuration(uri).expect("a valid connection string")
    }

    #[test]
    fn opcua_config_request_timeout_default_is_30000() {
        let config = config_of("opcua://host:4840");
        assert_eq!(
            config.timeouts().request,
            std::time::Duration::from_secs(30)
        );
        assert_eq!(
            options_of(&config).request_timeout,
            config.timeouts().request
        );
        let custom = config_of("opcua://host:4840?request-timeout-ms=125");
        assert_eq!(
            custom.timeouts().request,
            std::time::Duration::from_millis(125)
        );
        assert_eq!(
            options_of(&custom).request_timeout,
            custom.timeouts().request
        );
    }

    #[test]
    fn opcua_config_receive_buffer_size_default_is_65535() {
        assert_eq!(
            options_of(&config_of("opcua://host:4840")).receive_buffer_size,
            65535
        );
        for value in [8192, 65535, 0x7FFF_FFFF] {
            let config = config_of(&format!(
                "opcua://host:4840?encoding.receive-buffer-size={value}"
            ));
            assert_eq!(options_of(&config).receive_buffer_size, value);
        }
    }

    #[test]
    fn opcua_config_receive_buffer_size_rejects_zero() {
        for value in [
            "0",
            "8191",
            "-1",
            "+8192",
            "2147483648",
            "4294967296",
            "x",
            "8.5",
        ] {
            assert_eq!(
                refused(&format!(
                    "opcua://host:4840?encoding.receive-buffer-size={value}"
                ))
                .kind(),
                ErrorKind::InvalidConfig
            );
        }
    }

    #[test]
    fn opcua_config_request_timeout_rejects_zero_negative_and_non_numeric() {
        for value in [
            "0",
            "-1",
            "hello",
            "1.5",
            "+1",
            "18446744073709551616",
            "1%20",
        ] {
            let problem = refused(&format!("opcua://host:4840?request-timeout-ms={value}"));
            assert!(!problem.to_string().contains(value), "no value is echoed");
        }
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
            OpcuaDriver::configuration(uri).expect_err("the connection string must be refused");
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
        assert_eq!(
            defaults.timeouts().connect,
            std::time::Duration::from_secs(60)
        );
        assert_eq!(defaults.timeouts().idle, plain.timeouts().idle);
        assert_eq!(
            defaults.timeouts().request,
            std::time::Duration::from_secs(30)
        );
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
        let address =
            OpcuaDriver::address("ns=2;s=Boiler.Temperature;REAL", &options).expect("an address");
        assert_eq!(address.protocol(), "opcua");
        assert_eq!(address.canonical(), "ns=2;s=Boiler.Temperature;REAL");
        let error = OpcuaDriver::address("not an address", &options).expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidAddress);
    }
}

#[cfg(test)]
mod nonce_tests {
    use super::*;
    #[test]
    fn opcua_nonce_generation_failure_has_no_fallback() {
        assert_eq!(
            nonce_result(Err(getrandom::Error::UNSUPPORTED))
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidConfig
        );
        assert_eq!(OpcuaDriver::default().scheme(), "opcua");
    }
}
