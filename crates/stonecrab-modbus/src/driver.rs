//! `ModbusTcpDriver`: the `Driver` of the `modbus-tcp` scheme.

use std::fmt;
use std::sync::Arc;

use stonecrab_core::{
    AddressOptions, Capabilities, Clock, Connection, ConnectionConfig, ConnectionUri, Driver,
    Endpoint, ErrorKind, OpFuture, PlcError, ProtocolAddress, Result,
};
use stonecrab_transport::{SystemClock, TcpFactory, TransportFactory};

use crate::connection::alpha_capabilities;
use crate::{ConnectionDefaults, ModbusAddress, ModbusConnection};

/// The connection-string scheme of this driver.
const SCHEME: &str = "modbus-tcp";

/// The only transport code the scheme accepts after `modbus-tcp:`.
const TRANSPORT: &str = "tcp";

/// The port of a connection string that names none (upstream `DEFAULT_TCP_PORT`).
const DEFAULT_PORT: u16 = 502;

/// The `Driver` of the `modbus-tcp` scheme: read-only Modbus TCP (function codes 01 to 04).
///
/// The driver is a stateless factory. It holds the clock, the transport factory and the
/// connection-wide [`ConnectionDefaults`] that every connection it opens starts from, and
/// it keeps no per-connection state. [`new`](Self::new) uses the real TCP transport and
/// the Tokio-backed [`SystemClock`]; [`with_clock`](Self::with_clock) and
/// [`with_transport_factory`](Self::with_transport_factory) replace them, which is how the
/// testkit's `FakeClock` and `FakeTransportFactory` run the contract suite over this
/// driver (ADR-0015, decision 14).
///
/// # Connection strings
/// `modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]`, with `host` a name, an
/// IPv4 address or an IPv6 address in brackets (`[::1]:502`), and the port 1 to 65535,
/// default 502. Any other transport code is [`ErrorKind::InvalidConfig`].
///
/// **Parameters.** This first version accepts none: any `?key=value` is
/// [`ErrorKind::InvalidConfig`]. Slice S26 completes the driver with the six upstream
/// parameters (`request-timeout-ms` as `MB-CFG-REQUEST-TIMEOUT`, `default-unit-identifier`
/// as `MB-CFG-UNIT-ID`, `ping-address` as `MB-CFG-PING-ADDRESS`, `default-payload-byte-order`
/// as `MB-CFG-BYTE-ORDER`, `max-coils-per-request` as `MB-CFG-MAX-COILS` and
/// `max-registers-per-request` as `MB-CFG-MAX-REGISTERS`), the shared `address-convention`
/// key, and the rejection of unknown keys (`MB-CFG-UNKNOWN`). Until then
/// [`with_defaults`](Self::with_defaults) is the only way to change the unit or the byte
/// order of a connection.
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-CONNECT
/// Protocol code `modbus-tcp`, transport `tcp`, port 502 as upstream
/// (`MB-TCP-CONNECT`); request-driven reconnect is a Stonecrab extension recorded in the
/// same row. [`capabilities`](Driver::capabilities) is the `MB-TCP-CAPABILITIES` value, the
/// one every [`ModbusConnection`] reports
/// (docs/parity/modbus.md#MB-TCP-CAPABILITIES).
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{ConnectionState, Driver, DriverRegistry};
/// use stonecrab_modbus::ModbusTcpDriver;
/// use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::default());
/// let driver = ModbusTcpDriver::new()
///     .with_clock(Arc::new(clock.clone()))
///     .with_transport_factory(Arc::new(factory));
/// assert_eq!(driver.scheme(), "modbus-tcp");
///
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(driver))?;
/// let connection = block_on_with(&clock, registry.connect("modbus-tcp://192.0.2.10"))?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
/// # Ok::<(), stonecrab_core::PlcError>(())
/// ```
pub struct ModbusTcpDriver {
    clock: Arc<dyn Clock>,
    factory: Arc<dyn TransportFactory>,
    defaults: ConnectionDefaults,
}

impl ModbusTcpDriver {
    /// A driver over the real TCP transport and the [`SystemClock`], with the upstream
    /// connection defaults (unit 1, `BIG_ENDIAN`).
    ///
    /// Nothing is opened until a connection is requested, so constructing the driver does
    /// no I/O. Connections must be driven inside a Tokio runtime with the time driver
    /// enabled, as [`SystemClock`] documents.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_core::Driver;
    /// use stonecrab_modbus::ModbusTcpDriver;
    ///
    /// let driver = ModbusTcpDriver::new();
    /// assert_eq!(driver.scheme(), "modbus-tcp");
    /// assert!(driver.capabilities().read && !driver.capabilities().write);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock::default()),
            factory: Arc::new(TcpFactory::default()),
            defaults: ConnectionDefaults::default(),
        }
    }

    /// Replace the clock every connection of this driver uses.
    ///
    /// Inject the testkit's `FakeClock` to make waiting deterministic, and derive request
    /// deadlines from the same clock. The transport factory must use that clock too, as
    /// the testkit's `FakeTransportFactory` does when built from it.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_modbus::ModbusTcpDriver;
    /// use stonecrab_testkit::FakeClock;
    ///
    /// let driver = ModbusTcpDriver::new().with_clock(Arc::new(FakeClock::default()));
    /// assert!(format!("{driver:?}").contains("ModbusTcpDriver"));
    /// ```
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the factory that opens a transport for every connection.
    ///
    /// This is the sanctioned way to run a Modbus connection without a socket: the
    /// testkit's `FakeTransportFactory` serves in-memory transports driven by a script or
    /// a responder (ADR-0015, decision 14). There is no global registry of test
    /// transports and no Cargo feature for fakes.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab_core::{ConnectionState, Driver};
    /// use stonecrab_modbus::ModbusTcpDriver;
    /// use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let driver = ModbusTcpDriver::new()
    ///     .with_clock(Arc::new(clock.clone()))
    ///     .with_transport_factory(Arc::new(factory.clone()));
    ///
    /// let config = driver.parse_config("modbus-tcp://192.0.2.10:1502")?;
    /// let connection = block_on_with(&clock, driver.connect(config))?;
    /// assert_eq!(connection.state(), ConnectionState::Connected);
    /// assert_eq!(factory.opens().len(), 1);
    /// # Ok::<(), stonecrab_core::PlcError>(())
    /// ```
    #[must_use]
    pub fn with_transport_factory(mut self, factory: Arc<dyn TransportFactory>) -> Self {
        self.factory = factory;
        self
    }

    /// Replace the connection-wide unit and byte order every connection starts from.
    ///
    /// Until S26 parses `default-unit-identifier` and `default-payload-byte-order` from the
    /// connection string, this is the only way to set them.
    ///
    /// # Examples
    /// ```
    /// use stonecrab_modbus::{ConnectionDefaults, ModbusTcpDriver, UnitId};
    ///
    /// let mut defaults = ConnectionDefaults::default();
    /// defaults.unit_id = UnitId::from(7);
    /// let driver = ModbusTcpDriver::new().with_defaults(defaults);
    /// assert!(format!("{driver:?}").contains("unit_id"));
    /// ```
    #[must_use]
    pub const fn with_defaults(mut self, defaults: ConnectionDefaults) -> Self {
        self.defaults = defaults;
        self
    }
}

impl Default for ModbusTcpDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Shows the connection defaults; the clock and the factory have no useful `Debug`.
impl fmt::Debug for ModbusTcpDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusTcpDriver")
            .field("defaults", &self.defaults)
            .finish_non_exhaustive()
    }
}

impl Driver for ModbusTcpDriver {
    fn scheme(&self) -> &'static str {
        SCHEME
    }

    /// Read only, no subscriptions, browse or discovery, one operation at a time, PDUs up
    /// to 253 bytes: the value every [`ModbusConnection`] reports.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CAPABILITIES
    /// The alpha value of `MB-TCP-CAPABILITIES`: write false (D3), subscribe `Unsupported`
    /// and discover false differ from upstream and are recorded in that row.
    fn capabilities(&self) -> Capabilities {
        alpha_capabilities()
    }

    /// Read `modbus-tcp[:tcp]://host[:port]` into a validated configuration.
    ///
    /// The text is parsed with [`ConnectionUri::parse`]. The configuration has an
    /// [`Endpoint::Tcp`] and the core defaults for timeouts, reconnect, retry, limits and
    /// the access policy. No parameter is accepted in this version; S26 adds the six
    /// upstream parameters, `MB-CFG-REQUEST-TIMEOUT`, `MB-CFG-UNIT-ID`,
    /// `MB-CFG-PING-ADDRESS`, `MB-CFG-BYTE-ORDER`, `MB-CFG-MAX-COILS` and
    /// `MB-CFG-MAX-REGISTERS`, and `MB-CFG-UNKNOWN` (unknown keys stay rejected).
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for text that is not a valid connection string,
    /// another scheme, a transport code other than `tcp`, an authority that is not
    /// `host[:port]` (an empty host, a port that is empty, not decimal digits, zero or above
    /// 65535, an IPv6 address without brackets, a path), and any query parameter. The
    /// messages never repeat the authority.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CONNECT
    /// Protocol code, transport code and default port 502 as upstream; the rejection of
    /// every parameter is a placeholder that S26 replaces with the parameter rows.
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        let parsed = ConnectionUri::parse(uri)?;
        if parsed.scheme() != SCHEME {
            return Err(invalid("not a modbus-tcp connection string"));
        }
        if parsed.transport().is_some_and(|code| code != TRANSPORT) {
            return Err(invalid("the transport code of modbus-tcp is tcp"));
        }
        if !parsed.options().is_empty() {
            return Err(invalid(
                "connection parameters are not accepted yet; they arrive with the \
                 configuration slice",
            ));
        }
        ConnectionConfig::builder(SCHEME, endpoint(parsed.authority())?).build()
    }

    /// Parse Modbus address text; see [`ModbusAddress::parse`] for the grammar.
    ///
    /// `options` carries the address convention: the bare `40001` spellings are accepted
    /// only with the explicit `plc4x` convention.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] as [`ModbusAddress::parse`] does.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-ADDR-BARE-40001
    /// Delegates to [`ModbusAddress::parse`], whose grammar rows and divergences apply.
    fn parse_address(
        &self,
        text: &str,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(ModbusAddress::parse(text, options)?))
    }

    /// Open a [`ModbusConnection`] to the configured endpoint.
    ///
    /// This is [`ModbusConnection::open`] with the driver's factory, clock and defaults,
    /// boxed: the whole attempt is bounded by `config.timeouts().connect`.
    ///
    /// # Errors
    /// As [`ModbusConnection::open`]: [`ErrorKind::InvalidConfig`] for a configuration of
    /// another scheme or with a frame limit below 260 bytes, [`ErrorKind::Timeout`] when
    /// the open does not finish in time and [`ErrorKind::Disconnected`] when the peer
    /// refuses.
    ///
    /// # Cancel safety
    /// Cancel-safe, as [`ModbusConnection::open`]: a dropped call leaves no transport behind
    /// and no connection to close. The deadline source is `config.timeouts().connect`.
    ///
    /// # Wire effects
    /// Opens a transport and sends no application bytes, as [`ModbusConnection::open`].
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CONNECT
    /// The counterpart of upstream `connect()` (`onConnect`); the reconnect behavior of
    /// the connection is a Stonecrab extension recorded in the same row.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        let opening = ModbusConnection::open(
            config,
            Arc::clone(&self.factory),
            Arc::clone(&self.clock),
            self.defaults,
        );
        Box::pin(async move {
            let connection: Box<dyn Connection> = Box::new(opening.await?);
            Ok(connection)
        })
    }
}

/// The endpoint a `host[:port]` authority names.
fn endpoint(authority: &str) -> Result<Endpoint> {
    let (host, port) = if let Some(bracketed) = authority.strip_prefix('[') {
        let Some((host, rest)) = bracketed.split_once(']') else {
            return Err(invalid("an IPv6 host in brackets needs a closing bracket"));
        };
        if !is_ipv6(host) {
            return Err(invalid("the bracketed host is not an IPv6 address"));
        }
        let port = match rest.strip_prefix(':') {
            Some(port) => Some(port),
            None if rest.is_empty() => None,
            None => return Err(invalid("only a port may follow the bracketed host")),
        };
        (host, port)
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        if !is_name(host) {
            return Err(invalid(
                "the host is empty or not a name or IPv4 address (IPv6 needs brackets)",
            ));
        }
        (host, port)
    };
    let port = port.map_or(Ok(DEFAULT_PORT), parse_port)?;
    Ok(Endpoint::Tcp {
        host: host.to_owned(),
        port,
    })
}

/// A port: decimal digits only, 1 to 65535.
fn parse_port(text: &str) -> Result<u16> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    match text.parse::<u16>() {
        Ok(port) if digits && port != 0 => Ok(port),
        _ => Err(invalid("the port is a whole number from 1 to 65535")),
    }
}

/// A host name or IPv4 address: ASCII letters, digits and `-`, `_`, `.`; not empty.
fn is_name(host: &str) -> bool {
    host.bytes().any(|byte| byte.is_ascii_alphanumeric())
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// The inside of an IPv6 bracket form: a valid IPv6 address (an embedded IPv4 tail is fine).
fn is_ipv6(host: &str) -> bool {
    host.parse::<std::net::Ipv6Addr>().is_ok()
}

fn invalid(message: &'static str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}

#[cfg(test)]
mod tests {
    use stonecrab_testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};

    use super::*;

    fn driver() -> ModbusTcpDriver {
        ModbusTcpDriver::new()
    }

    fn endpoint_of(uri: &str) -> Endpoint {
        driver()
            .parse_config(uri)
            .expect("a valid connection string")
            .endpoint()
            .clone()
    }

    fn tcp(host: &str, port: u16) -> Endpoint {
        Endpoint::Tcp {
            host: host.to_owned(),
            port,
        }
    }

    fn refused(uri: &str) -> ErrorKind {
        driver()
            .parse_config(uri)
            .expect_err("the connection string must be refused")
            .kind()
    }

    // covers: ALPHA_PLAN S25b / parse_config accepts host and optional port, default 502
    #[test]
    fn parse_config_reads_host_and_port_with_default_502() {
        assert_eq!(
            endpoint_of("modbus-tcp://192.0.2.10"),
            tcp("192.0.2.10", 502)
        );
        assert_eq!(
            endpoint_of("modbus-tcp://192.0.2.10:1502"),
            tcp("192.0.2.10", 1502)
        );
        assert_eq!(
            endpoint_of("modbus-tcp://plc-1.line_2:1"),
            tcp("plc-1.line_2", 1)
        );
        assert_eq!(
            endpoint_of("modbus-tcp://localhost:65535"),
            tcp("localhost", 65535)
        );
        assert_eq!(
            endpoint_of("modbus-tcp:tcp://192.0.2.10"),
            tcp("192.0.2.10", 502),
            "the explicit transport code is the same as none"
        );
    }

    #[test]
    fn parse_config_gives_the_core_defaults() {
        let config = driver()
            .parse_config("modbus-tcp://192.0.2.10")
            .expect("config");
        let defaults = ConnectionConfig::builder(SCHEME, tcp("192.0.2.10", 502))
            .build()
            .expect("defaults");
        assert_eq!(config.scheme(), "modbus-tcp");
        assert_eq!(config.timeouts(), defaults.timeouts());
        assert_eq!(config.limits(), defaults.limits());
        assert_eq!(config.access(), defaults.access());
        assert_eq!(config.address_options().convention(), None);
    }

    // covers: ALPHA_PLAN S25b / IPv6 bracket forms
    #[test]
    fn parse_config_accepts_bracketed_ipv6_with_and_without_a_port() {
        assert_eq!(endpoint_of("modbus-tcp://[::1]"), tcp("::1", 502));
        assert_eq!(endpoint_of("modbus-tcp://[::1]:5020"), tcp("::1", 5020));
        assert_eq!(
            endpoint_of("modbus-tcp:tcp://[2001:db8::10]:1502"),
            tcp("2001:db8::10", 1502)
        );
        assert_eq!(
            endpoint_of("modbus-tcp://[::ffff:192.0.2.1]"),
            tcp("::ffff:192.0.2.1", 502)
        );
    }

    // covers: ALPHA_PLAN S25b / malformed authorities are InvalidConfig
    #[test]
    fn parse_config_refuses_a_bad_authority() {
        for uri in [
            "modbus-tcp://:502",
            "modbus-tcp://host:",
            "modbus-tcp://host:0",
            "modbus-tcp://host:65536",
            "modbus-tcp://host:99999999999999999999",
            "modbus-tcp://host:http",
            "modbus-tcp://host:+502",
            "modbus-tcp://host:-1",
            "modbus-tcp://host:5:6",
            "modbus-tcp://::1",
            "modbus-tcp://::1:502",
            "modbus-tcp://[::1",
            "modbus-tcp://[]",
            "modbus-tcp://[::1]x",
            "modbus-tcp://[::1]:",
            "modbus-tcp://[::1]:0",
            "modbus-tcp://[host]:502",
            "modbus-tcp://[1.2.3.4]",
            "modbus-tcp://[::g]",
            "modbus-tcp://[:]",
            "modbus-tcp://[1:2:3:4:5:6:7:8:9]",
            "modbus-tcp://[::.:]",
            "modbus-tcp://.",
            "modbus-tcp://-:502",
            "modbus-tcp://host/path",
            "modbus-tcp://ho[st",
            "modbus-tcp://",
        ] {
            assert_eq!(refused(uri), ErrorKind::InvalidConfig, "{uri}");
        }
    }

    #[test]
    fn parse_config_refuses_another_scheme_and_any_transport_but_tcp() {
        for uri in [
            "sim://bench",
            "modbus-rtu://host",
            "modbus-tcp:udp://host",
            "modbus-tcp:tls://host",
            "modbus-tcp:TCP://host",
            "not a connection string",
            "modbus-tcp:://host",
        ] {
            assert_eq!(refused(uri), ErrorKind::InvalidConfig, "{uri}");
        }
    }

    // covers: ALPHA_PLAN S25b / no query parameter is accepted before the configuration slice
    // Transitional: S26 replaces this test with the per-parameter rows.
    #[test]
    fn parse_config_refuses_every_parameter_and_names_the_slice() {
        for uri in [
            "modbus-tcp://host?request-timeout-ms=500",
            "modbus-tcp://host:502?default-unit-identifier=1",
            "modbus-tcp://host?address-convention=plc4x",
            "modbus-tcp://host?surprise=1",
            "modbus-tcp://host?x=",
        ] {
            let error = driver().parse_config(uri).expect_err("a parameter");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
            assert!(
                error.to_string().contains("configuration slice"),
                "the message says when parameters arrive: {error}"
            );
        }
    }

    #[test]
    fn a_refusal_never_repeats_the_authority() {
        let error = driver()
            .parse_config("modbus-tcp://secret-host-name:0")
            .expect_err("port 0");
        assert!(!error.to_string().contains("secret-host-name"), "{error}");
    }

    // covers: ALPHA_PLAN S25b / parse_address uses the convention of the options
    #[test]
    fn parse_address_delegates_with_and_without_a_convention() {
        let none = AddressOptions::default();
        let plc4x = AddressOptions::builder()
            .convention("plc4x")
            .build()
            .expect("options");
        let long = driver()
            .parse_address("holding-register:16:INT", &none)
            .expect("long form");
        assert_eq!(long.canonical(), "holding-register:16:INT");
        assert_eq!(long.protocol(), "modbus");

        let bare = driver().parse_address("40016", &none);
        assert_eq!(
            bare.err().map(|error| error.kind()),
            Some(ErrorKind::InvalidAddress),
            "the bare form needs the explicit convention"
        );
        let resolved = driver()
            .parse_address("40016", &plc4x)
            .expect("bare form with the convention");
        assert_eq!(resolved.canonical(), "holding-register:16:INT");

        let unknown = AddressOptions::builder()
            .convention("other")
            .build()
            .expect("options");
        assert_eq!(
            driver()
                .parse_address("40016", &unknown)
                .err()
                .map(|error| error.kind()),
            Some(ErrorKind::InvalidAddress),
            "only plc4x is a convention"
        );
    }

    // covers: ALPHA_PLAN S25b / the driver reports what its connections report
    #[test]
    fn the_driver_capabilities_equal_the_connection_capabilities() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory));
        let config = driver
            .parse_config("modbus-tcp://192.0.2.10")
            .expect("config");
        let connection = block_on_with(&clock, driver.connect(config)).expect("connect");
        assert_eq!(driver.capabilities(), connection.capabilities());
        let capabilities = driver.capabilities();
        assert!(capabilities.read && !capabilities.write);
        assert!(!capabilities.browse && !capabilities.discover);
        assert_eq!(
            capabilities.max_pdu_bytes.map(std::num::NonZeroUsize::get),
            Some(253)
        );
        assert_eq!(capabilities.concurrency.get(), 1);
        assert!(!capabilities.supports(stonecrab_core::Operation::Subscribe));
    }

    #[test]
    fn new_and_default_construct_without_io_and_share_the_scheme() {
        let built = ModbusTcpDriver::new();
        let defaulted = ModbusTcpDriver::default();
        assert_eq!(built.scheme(), "modbus-tcp");
        assert_eq!(defaulted.scheme(), built.scheme());
        assert_eq!(built.defaults, ConnectionDefaults::default());
        assert!(format!("{built:?}").starts_with("ModbusTcpDriver"));
    }

    #[test]
    fn connect_passes_the_driver_defaults_clock_and_factory_to_the_connection() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let defaults = ConnectionDefaults {
            unit_id: crate::UnitId::from(9),
            ..ConnectionDefaults::default()
        };
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()))
            .with_defaults(defaults);
        assert_eq!(driver.defaults.unit_id, crate::UnitId::from(9));
        let config = driver
            .parse_config("modbus-tcp://192.0.2.10:1502")
            .expect("config");
        block_on_with(&clock, driver.connect(config)).expect("connect");
        let opens = factory.opens();
        assert_eq!(opens.len(), 1, "the driver's factory opened the transport");
        assert_eq!(
            opens.first().map(|call| call.endpoint.clone()),
            Some(tcp("192.0.2.10", 1502))
        );
    }

    #[test]
    fn connect_refuses_a_configuration_of_another_scheme_before_any_open() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let config = ConnectionConfig::builder("sim", tcp("192.0.2.10", 502))
            .build()
            .expect("config");
        let error = block_on_with(&clock, driver.connect(config))
            .err()
            .expect("another scheme");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert_eq!(factory.opens().len(), 0, "nothing was opened");
    }
}
