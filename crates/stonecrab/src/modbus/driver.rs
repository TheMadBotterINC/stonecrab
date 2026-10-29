//! `ModbusTcpDriver`: the `Driver` of the `modbus-tcp` scheme.

use std::fmt;
use std::sync::Arc;

#[cfg(feature = "tls")]
use crate::transport::TlsFactory;
use crate::transport::{SystemClock, TcpFactory, TransportFactory, UdpFactory};
use crate::{
    AddressOptions, Capabilities, Clock, Connection, ConnectionConfig, ConnectionUri, Driver,
    Endpoint, ErrorKind, OpFuture, PlcError, ProtocolAddress, Result, Timeouts,
};

use crate::modbus::connection::driver_capabilities;
use crate::modbus::{ConnectionDefaults, ModbusAddress, ModbusConnection, ModbusOptions};

/// The connection-string scheme of this driver.
const SCHEME: &str = "modbus-tcp";

/// The transport code of TCP, the default.
const TCP: &str = "tcp";

/// The transport code of Modbus TCP over UDP: the same MBAP frames, one per datagram.
const UDP: &str = "udp";

/// The transport code of Modbus TCP over TLS (Modbus/TCP Security): the same MBAP frames
/// inside a TLS session (ADR-0032).
#[cfg(feature = "tls")]
const TLS: &str = "tls";

/// The transport code of TLS with external pre-shared keys, which is refused: rustls has no
/// external pre-shared keys (decision E11, ADR-0032).
const TLS_PSK: &str = "tls-psk";

/// The port of a connection string that names none, for `tcp` and `udp` alike (upstream
/// `DEFAULT_TCP_PORT` and `DEFAULT_UDP_PORT`), and for the `tcp` carrier of RTU and ASCII.
pub(super) const DEFAULT_PORT: u16 = 502;

/// The port of a `tls` connection string that names none (upstream `DEFAULT_TLS_PORT`, the
/// Modbus/TCP Security port).
#[cfg(feature = "tls")]
const TLS_PORT: u16 = 802;

/// The refusal of a transport code this build does not offer.
#[cfg(feature = "tls")]
const TRANSPORTS: &str = "the transport code of modbus-tcp is tcp, udp or tls";
/// The refusal of a transport code this build does not offer.
#[cfg(not(feature = "tls"))]
const TRANSPORTS: &str =
    "the transport code of modbus-tcp is tcp or udp (tls needs the tls feature)";

/// The `Driver` of the `modbus-tcp` scheme: read-only Modbus TCP (function codes 01 to 04)
/// with ping.
///
/// The driver is a stateless factory. It holds the clock, the transport factory and the
/// connection-wide [`ConnectionDefaults`] that every connection it opens starts from, and
/// it keeps no per-connection state. [`new`](Self::new) uses the real TCP or UDP transport
/// that the connection string names and the Tokio-backed [`SystemClock`];
/// [`with_clock`](Self::with_clock) and
/// [`with_transport_factory`](Self::with_transport_factory) replace them, which is how the
/// testkit's `FakeClock` and `FakeTransportFactory` run the contract suite over this
/// driver (ADR-0015, decision 14).
///
/// # Connection strings
/// `modbus-tcp://host[:port]` or `modbus-tcp:tcp://host[:port]` for TCP, and
/// `modbus-tcp:udp://host[:port]` for Modbus TCP over UDP (the same MBAP frames, one per
/// datagram, over a [`UdpFactory`] socket connected to the device; ADR-0027). `host` is a
/// name, an IPv4 address or an IPv6 address in brackets (`[::1]:502`), and the port is 1 to
/// 65535, default 502 for both. With the `tls` feature, `modbus-tcp:tls://host[:port]` is
/// Modbus TCP over TLS (Modbus/TCP Security, default port 802; ADR-0032): the same MBAP
/// frames inside a TLS 1.3 or 1.2 session, opened with a
/// [`TlsFactory`](crate::transport::TlsFactory) that trusts only the certificate authorities
/// in the PEM file `tls.trust-store` names and, for mutual TLS, presents the certificate and
/// key of the PEM file `tls.keystore` names. `tls-psk` is refused, because rustls has no
/// external pre-shared keys. Any other transport code is [`ErrorKind::InvalidConfig`].
/// Over UDP nothing is sent when the connection opens, a lost datagram is a request that
/// times out at its deadline, and a reply that arrives twice or late is discarded by its
/// transaction id.
///
/// **Parameters.** After `?`, `key=value` pairs joined by `&`. The six upstream
/// parameters are `request-timeout-ms` (positive integer, default 5000),
/// `default-unit-identifier` (0 to 255, default 1), `ping-address` (default
/// `4x00001:BOOL`, validated and recorded, unused), `default-payload-byte-order`
/// (`BIG_ENDIAN` by default, `LITTLE_ENDIAN`, `BIG_ENDIAN_BYTE_SWAP`,
/// `LITTLE_ENDIAN_BYTE_SWAP`), `max-coils-per-request` (1 to 2000, default 2000) and
/// `max-registers-per-request` (1 to 125, default 125). The shared `address-convention`
/// key accepts `plc4x`, `one-based-40001` and `zero-based`. The `tls` transport needs
/// `tls.trust-store` and takes `tls.keystore`, which no other transport takes. Every other
/// key is [`ErrorKind::InvalidConfig`], where upstream warns (`MB-CFG-UNKNOWN`). The parsed
/// values travel in the configuration as [`ModbusOptions`], seeded from the driver's
/// defaults ([`with_defaults`](Self::with_defaults)) and overridden by the connection
/// string; a configuration built by hand carries none and gets the driver's defaults.
///
/// # Compatibility: docs/parity/modbus.md#MB-TCP-CONNECT
/// Protocol code `modbus-tcp`, transport `tcp`, port 502 as upstream
/// (`MB-TCP-CONNECT`); request-driven reconnect is a Stonecrab extension recorded in the
/// same row. The `udp` transport with its default port 502 is
/// docs/parity/modbus.md#MB-TCP-TRANSPORTS-UDP, where upstream's own default never applies
/// (docs/parity/modbus.md#MB-QUIRK-A). The `tls` transport with its default port 802 is
/// docs/parity/modbus.md#MB-TCP-TRANSPORTS-TLS and the refused `tls-psk`
/// docs/parity/modbus.md#MB-TCP-TRANSPORTS-TLS-PSK. [`capabilities`](Driver::capabilities) is
/// the
/// `MB-TCP-CAPABILITIES` value, the one every [`ModbusConnection`] reports
/// (docs/parity/modbus.md#MB-TCP-CAPABILITIES).
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::{ConnectionState, Driver, DriverRegistry};
/// use stonecrab::modbus::ModbusTcpDriver;
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
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
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct ModbusTcpDriver {
    clock: Arc<dyn Clock>,
    /// The factory every connection opens its transport with; `None` opens the transport
    /// the connection string names.
    factory: Option<Arc<dyn TransportFactory>>,
    defaults: ConnectionDefaults,
}

impl ModbusTcpDriver {
    /// A driver over the real TCP, UDP and (feature `tls`) TLS transports and the
    /// [`SystemClock`], with the upstream connection defaults (unit 1, `BIG_ENDIAN`).
    ///
    /// Nothing is opened until a connection is requested, so constructing the driver does
    /// no I/O. Connections must be driven inside a Tokio runtime with the time driver
    /// enabled, as [`SystemClock`] documents.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Driver;
    /// use stonecrab::modbus::ModbusTcpDriver;
    ///
    /// let driver = ModbusTcpDriver::new();
    /// assert_eq!(driver.scheme(), "modbus-tcp");
    /// assert!(driver.capabilities().read && driver.capabilities().write);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock::default()),
            factory: None,
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
    /// # #[cfg(feature = "testkit")]
    /// # fn main() {
    /// use std::sync::Arc;
    /// use stonecrab::modbus::ModbusTcpDriver;
    /// use stonecrab::testkit::FakeClock;
    ///
    /// let driver = ModbusTcpDriver::new().with_clock(Arc::new(FakeClock::default()));
    /// assert!(format!("{driver:?}").contains("ModbusTcpDriver"));
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the factory that opens a transport for every connection, whatever transport
    /// code the connection string names.
    ///
    /// This is the sanctioned way to run a Modbus connection without a socket: the
    /// testkit's `FakeTransportFactory` serves in-memory transports driven by a script or
    /// a responder (ADR-0015, decision 14). There is no global registry of test
    /// transports and no Cargo feature for fakes.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Driver};
    /// use stonecrab::modbus::ModbusTcpDriver;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
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
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn with_transport_factory(mut self, factory: Arc<dyn TransportFactory>) -> Self {
        self.factory = Some(factory);
        self
    }

    /// Replace the connection-wide defaults of this driver: the unit, the byte and word
    /// order and the two request-size limits.
    ///
    /// [`parse_config`](Driver::parse_config) seeds the [`ModbusOptions`] it attaches from
    /// these defaults and lets the connection-string parameters override them, so a
    /// parameter the string does not name takes the value given here. A configuration built
    /// by hand with [`ConnectionConfig::builder`] carries no options, and `connect` uses
    /// these defaults for it. The full rule is: driver defaults, overridden by the
    /// connection string, overridden by the address block, overridden by the tag.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::modbus::{ConnectionDefaults, ModbusTcpDriver, UnitId};
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

    /// Read, write, ping and polling subscriptions, no browse or discovery, one operation
    /// at a time, PDUs up to 253 bytes: the value every [`ModbusConnection`] reports.
    ///
    /// `write` is true; whether a connection may use it is its access policy, which is
    /// read-only unless the connection string says `access=read-write`.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CAPABILITIES
    /// The value of `MB-TCP-CAPABILITIES`: subscribe is `Polling`, as upstream's polling
    /// subscriptions are; discover false differs from upstream and is recorded in that row.
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Read `modbus-tcp[:tcp|:udp|:tls]://host[:port][?parameters]` into a validated
    /// configuration.
    ///
    /// The text is parsed with [`ConnectionUri::parse`], which already refuses duplicate
    /// and empty keys. The configuration has an [`Endpoint::Tcp`] for TCP, or for `udp` an
    /// [`Endpoint::Custom`] holding `host:port` (an IPv6 host in brackets) as
    /// [`UdpFactory`] takes it (for `tls` an [`Endpoint::Tcp`] with default port 802); the
    /// core defaults for reconnect, retry and limits, a
    /// read-only access policy, and:
    ///
    /// - `request-timeout-ms` as `Timeouts.request` (5000 ms by default);
    /// - `address-convention` (`plc4x`, `one-based-40001` or `zero-based`) as the
    ///   configuration's address convention, which
    ///   [`ConnectionConfig::address_options`] hands to
    ///   [`parse_address`](Driver::parse_address);
    /// - `access=read-write` as the configuration's access policy; `read-only`, which is
    ///   also what an absent key means, refuses every write before any I/O;
    /// - the six upstream parameters as a [`ModbusOptions`] in the configuration's driver
    ///   options slot, where `connect` reads them back. The options start from this
    ///   driver's defaults ([`with_defaults`](ModbusTcpDriver::with_defaults)) and the
    ///   parameters the string names override them;
    /// - for `tls`, `tls.trust-store` and `tls.keystore` as the options' `tls` settings. The
    ///   files are not read here: `connect` reads them.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for text that is not a valid connection string,
    /// another scheme, a transport code other than `tcp`, `udp` and (feature `tls`) `tls`, a
    /// `tls` string without `tls.trust-store` or with an empty file name, a `tls.` parameter
    /// on another transport, an authority that is not
    /// `host[:port]` (an empty host, a port that is empty, not decimal digits, zero or above
    /// 65535, an IPv6 address without brackets, a path), and a parameter that is unknown,
    /// empty, not a whole number where one is required, outside its bounds, an unknown
    /// byte-order name, an invalid `ping-address`, an `address-convention` other than
    /// the three names or an `access` other than `read-only` and `read-write`. The
    /// messages never repeat the authority or the connection string; a parameter error
    /// names its key.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-CFG-UNKNOWN
    /// Protocol code, transport code and default port 502 as upstream
    /// (docs/parity/modbus.md#MB-TCP-CONNECT). The six parameters are the rows
    /// `MB-CFG-REQUEST-TIMEOUT` (docs/parity/modbus.md#MB-CFG-REQUEST-TIMEOUT),
    /// `MB-CFG-UNIT-ID` (docs/parity/modbus.md#MB-CFG-UNIT-ID), `MB-CFG-PING-ADDRESS`
    /// (docs/parity/modbus.md#MB-CFG-PING-ADDRESS), `MB-CFG-BYTE-ORDER`
    /// (docs/parity/modbus.md#MB-CFG-BYTE-ORDER), `MB-CFG-MAX-COILS`
    /// (docs/parity/modbus.md#MB-CFG-MAX-COILS) and `MB-CFG-MAX-REGISTERS`
    /// (docs/parity/modbus.md#MB-CFG-MAX-REGISTERS). Unknown keys are an error where
    /// upstream warns (`MB-CFG-UNKNOWN`), which includes the misspelt `unit-identifier` of
    /// the upstream test suite (docs/parity/modbus.md#MB-QUIRK-F); a transport other than
    /// `tcp` and `udp` is refused (docs/parity/modbus.md#MB-TCP-TRANSPORTS), and `udp`
    /// defaults to port 502 where upstream's default never applies
    /// (docs/parity/modbus.md#MB-QUIRK-A). The `udp.` options are refused like every other
    /// transport option (docs/parity/modbus.md#MB-CFG-TRANSPORT-OPTIONS), except the two
    /// `tls.` keys of docs/parity/modbus.md#MB-TCP-TRANSPORTS-TLS. `access` is a
    /// Stonecrab parameter with no upstream counterpart (docs/parity/modbus.md#MB-CFG-ACCESS),
    /// and so is `single-coil-write`, which picks function 05 for a one-coil write
    /// (docs/parity/modbus.md#MB-CFG-SINGLE-COIL).
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        let parsed = ConnectionUri::parse(uri)?;
        if parsed.scheme() != SCHEME {
            return Err(invalid("not a modbus-tcp connection string"));
        }
        let endpoint = match parsed.transport() {
            None | Some(TCP) => endpoint(parsed.authority(), DEFAULT_PORT)?,
            Some(UDP) => Endpoint::Custom(endpoint(parsed.authority(), DEFAULT_PORT)?.to_string()),
            #[cfg(feature = "tls")]
            Some(TLS) => endpoint(parsed.authority(), TLS_PORT)?,
            Some(TLS_PSK) => {
                return Err(invalid(
                    "the tls-psk transport is not offered: rustls has no external pre-shared keys",
                ));
            }
            Some(_) => return Err(invalid(TRANSPORTS)),
        };
        let (options, convention, access) = ModbusOptions::from_parameters(
            ModbusOptions::from_defaults(self.defaults),
            parsed.options(),
        )?;
        #[cfg(feature = "tls")]
        match (parsed.transport() == Some(TLS), options.tls.is_some()) {
            (true, false) => {
                return Err(invalid(
                    "the tls transport needs tls.trust-store, the PEM file of the certificate \
                     authorities it trusts",
                ));
            }
            (false, true) => {
                return Err(invalid(
                    "tls.trust-store and tls.keystore belong to the tls transport",
                ));
            }
            (true, true) | (false, false) => {}
        }
        let timeouts = Timeouts {
            request: options.request_timeout,
            ..Timeouts::default()
        };
        let mut builder = ConnectionConfig::builder(SCHEME, endpoint)
            .timeouts(timeouts)
            .access(access)
            .driver_options(Arc::new(options));
        if let Some(convention) = convention {
            builder = builder.address_convention(convention.name());
        }
        builder.build()
    }

    /// Parse Modbus address text; see [`ModbusAddress::parse`] for the grammar.
    ///
    /// `options` carries the address convention: the bare `40001` spellings are accepted
    /// only with an explicit convention, and the convention decides whether the numbers
    /// count from 1 or from 0.
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
    /// This is [`ModbusConnection::open`] with the driver's clock and a transport factory,
    /// boxed: the whole attempt is bounded by `config.timeouts().connect`. The factory is the
    /// injected one when there is one, else a [`TlsFactory`](crate::transport::TlsFactory)
    /// built from the options' TLS settings when they are set (the `tls` transport, which
    /// reads the PEM files here, once per connection, on the calling thread and before the
    /// connect bound starts, so they must be regular local files), else [`UdpFactory`] for an
    /// [`Endpoint::Custom`] (the `udp` transport) and [`TcpFactory`] for any other endpoint.
    /// The unit, byte order and
    /// request-size limits come from the [`ModbusOptions`] of the configuration, else from
    /// the driver's own [`ConnectionDefaults`].
    ///
    /// # Errors
    /// As [`ModbusConnection::open`]: [`ErrorKind::InvalidConfig`] for a configuration of
    /// another scheme, with a frame limit below 260 bytes or with request-size limits above
    /// the protocol maxima, [`ErrorKind::Timeout`] when
    /// the open does not finish in time and [`ErrorKind::Disconnected`] when the peer
    /// refuses. Over TLS also [`ErrorKind::InvalidConfig`] for a PEM file that cannot be
    /// read or used, [`ErrorKind::Trust`] for a server certificate that fails verification
    /// and [`ErrorKind::Authentication`] for a server that refuses the client certificate.
    ///
    /// # Cancel safety
    /// Cancel-safe, as [`ModbusConnection::open`]: a dropped call leaves no transport behind
    /// and no connection to close. The deadline source is `config.timeouts().connect`.
    ///
    /// # Wire effects
    /// Opens a transport and sends no application bytes, as [`ModbusConnection::open`]; a
    /// UDP socket sends nothing at all, and a TLS connection completes its handshake.
    ///
    /// # Compatibility: docs/parity/modbus.md#MB-TCP-CONNECT
    /// The counterpart of upstream `connect()` (`onConnect`); the reconnect behavior of
    /// the connection is a Stonecrab extension recorded in the same row.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        // The options `parse_config` attached; a configuration built by hand has none and
        // gets the driver's own defaults.
        let defaults = config
            .driver_options_as::<ModbusOptions>()
            .map_or(self.defaults, ModbusOptions::connection_defaults);
        let factory = match self.factory_for(&config) {
            Ok(factory) => factory,
            Err(error) => return Box::pin(async move { Err(error) }),
        };
        let opening = ModbusConnection::open(config, factory, Arc::clone(&self.clock), defaults);
        Box::pin(async move {
            let connection: Box<dyn Connection> = Box::new(opening.await?);
            Ok(connection)
        })
    }
}

impl ModbusTcpDriver {
    /// The factory a connection of `config` opens its transport with: the injected one, else
    /// the transport the configuration names.
    #[cfg_attr(
        not(feature = "tls"),
        expect(
            clippy::unnecessary_wraps,
            reason = "only building the TLS factory can fail, and it needs the tls feature"
        )
    )]
    fn factory_for(&self, config: &ConnectionConfig) -> Result<Arc<dyn TransportFactory>> {
        if let Some(factory) = &self.factory {
            return Ok(Arc::clone(factory));
        }
        #[cfg(feature = "tls")]
        if let Some(tls) = config
            .driver_options_as::<ModbusOptions>()
            .and_then(|options| options.tls.as_ref())
        {
            return Ok(Arc::new(TlsFactory::new(tls)?));
        }
        if matches!(config.endpoint(), Endpoint::Custom(_)) {
            Ok(Arc::new(UdpFactory::default()))
        } else {
            Ok(Arc::new(TcpFactory::default()))
        }
    }
}

/// The endpoint a `host[:port]` authority names, with `default_port` when it names none.
pub(super) fn endpoint(authority: &str, default_port: u16) -> Result<Endpoint> {
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
    let port = port.map_or(Ok(default_port), parse_port)?;
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
#[cfg(feature = "testkit")]
mod tests {
    use crate::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};

    use crate::{ByteOrder, WordOrder};

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
    fn parse_config_refuses_another_scheme_and_any_transport_but_tcp_and_udp() {
        for uri in [
            "sim://bench",
            "modbus-rtu://host",
            "modbus-tcp:tls-psk://host",
            "modbus-tcp:TCP://host",
            "modbus-tcp:UDP://host",
            "modbus-tcp:udp://host:0",
            "modbus-tcp:udp://::1",
            "modbus-tcp:udp://host?udp.local-port=5020",
            "not a connection string",
            "modbus-tcp:://host",
        ] {
            assert_eq!(refused(uri), ErrorKind::InvalidConfig, "{uri}");
        }
    }

    // covers: S71 / MB-TCP-TRANSPORTS-TLS-PSK refused, with the reason
    #[test]
    fn the_tls_psk_transport_is_refused_because_rustls_has_no_external_psk() {
        let error = driver()
            .parse_config("modbus-tcp:tls-psk://host?tls-psk.psk-identity=plc")
            .expect_err("refused");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("pre-shared keys"), "{error}");
        let error = driver()
            .parse_config("modbus-tcp:tls13://host")
            .expect_err("refused");
        assert_eq!(error.to_string(), TRANSPORTS, "another code");
    }

    // covers: S71 / MB-TCP-TRANSPORTS-TLS default port 802 and the two tls. keys
    #[cfg(feature = "tls")]
    #[test]
    fn driver_accepts_the_tls_transport_and_defaults_port_802() {
        use std::path::Path;

        let config = driver()
            .parse_config(
                "modbus-tcp:tls://plc.example?tls.trust-store=/etc/plc/ca.pem\
                 &tls.keystore=/etc/plc/client.pem&default-unit-identifier=3",
            )
            .expect("config");
        assert_eq!(config.endpoint(), &tcp("plc.example", 802));
        let options = config
            .driver_options_as::<ModbusOptions>()
            .expect("options");
        let tls = options.tls.as_ref().expect("tls settings");
        assert_eq!(tls.trust_store, Path::new("/etc/plc/ca.pem"));
        assert_eq!(
            tls.keystore.as_deref(),
            Some(Path::new("/etc/plc/client.pem"))
        );
        assert_eq!(options.unit_id, crate::modbus::UnitId::from(3));
        assert_eq!(
            endpoint_of("modbus-tcp:tls://[::1]:8802?tls.trust-store=ca.pem"),
            tcp("::1", 8802)
        );
        let plain = driver()
            .parse_config("modbus-tcp:tls://host?tls.trust-store=ca.pem")
            .expect("config");
        let options = plain.driver_options_as::<ModbusOptions>().expect("options");
        assert_eq!(
            options.tls.as_ref().and_then(|tls| tls.keystore.as_ref()),
            None,
            "no keystore, no client certificate"
        );
        assert!(
            driver()
                .parse_config("modbus-tcp://host")
                .expect("tcp")
                .driver_options_as::<ModbusOptions>()
                .is_some_and(|options| options.tls.is_none()),
            "tcp carries no TLS settings"
        );
    }

    // covers: S71 / MB-CFG-TRANSPORT-OPTIONS the tls. keys belong to the tls transport
    #[cfg(feature = "tls")]
    #[test]
    fn the_tls_keys_are_required_on_tls_and_refused_elsewhere() {
        for (uri, says) in [
            ("modbus-tcp:tls://host", "needs tls.trust-store"),
            (
                "modbus-tcp:tls://host?tls.keystore=client.pem",
                "tls.keystore needs tls.trust-store",
            ),
            (
                "modbus-tcp:tls://host?tls.trust-store=",
                "must name a PEM file",
            ),
            (
                "modbus-tcp:tls://host?tls.trust-store=ca.pem&tls.keystore=",
                "must name a PEM file",
            ),
            (
                "modbus-tcp:tls://host?tls.trust-store=ca.pem&tls.verify=false",
                "the TLS parameters are tls.trust-store and tls.keystore",
            ),
            (
                "modbus-tcp:tls://host?tls.trust-store=ca.pem&tls.keystore-password=x",
                "the TLS parameters are tls.trust-store and tls.keystore",
            ),
            (
                "modbus-tcp://host?tls.trust-store=ca.pem",
                "belong to the tls transport",
            ),
            (
                "modbus-tcp:udp://host?tls.trust-store=ca.pem",
                "belong to the tls transport",
            ),
            ("modbus-tcp:tls://host:0?tls.trust-store=ca.pem", "port"),
        ] {
            let error = driver().parse_config(uri).expect_err("refused");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "{uri}");
            assert!(error.to_string().contains(says), "{uri}: {error}");
        }
    }

    // covers: S71 / ADR-0032 the trust store is read at connect, before any open
    #[cfg(feature = "tls")]
    #[test]
    fn connect_reads_the_trust_store_and_refuses_one_it_cannot_read_before_any_open() {
        let config = driver()
            .parse_config("modbus-tcp:tls://127.0.0.1?tls.trust-store=/nonexistent/ca.pem")
            .expect("config");
        let error = block_on_with(&FakeClock::default(), driver().connect(config))
            .err()
            .expect("the trust store cannot be read");
        assert_eq!(error.kind(), ErrorKind::InvalidConfig);
        assert!(error.to_string().contains("tls.trust-store"), "{error}");
        assert!(!error.to_string().contains("nonexistent"), "{error}");
    }

    // covers: S71 / ADR-0032 an injected factory serves the tls transport too
    #[cfg(feature = "tls")]
    #[test]
    fn an_injected_factory_serves_the_tls_transport() {
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::default());
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let config = driver
            .parse_config("modbus-tcp:tls://192.0.2.10?tls.trust-store=/nonexistent/ca.pem")
            .expect("config");
        block_on_with(&clock, driver.connect(config)).expect("the fake opens, no file is read");
        assert_eq!(
            factory.opens().first().map(|call| call.endpoint.clone()),
            Some(tcp("192.0.2.10", 802))
        );
    }

    // covers: ALPHA_PLAN S26 / parameters reach the timeouts, the options and the convention
    #[test]
    fn parse_config_applies_the_parameters_to_the_configuration() {
        let config = driver()
            .parse_config(
                "modbus-tcp://host:502?request-timeout-ms=750&default-unit-identifier=4\
                 &address-convention=plc4x",
            )
            .expect("config");
        assert_eq!(
            config.timeouts().request,
            std::time::Duration::from_millis(750)
        );
        assert_eq!(config.address_options().convention(), Some("plc4x"));
        let options = config
            .driver_options_as::<ModbusOptions>()
            .expect("the options are attached");
        assert_eq!(options.unit_id, crate::modbus::UnitId::from(4));
        let plain = driver().parse_config("modbus-tcp://host").expect("config");
        assert_eq!(plain.address_options().convention(), None);
        assert_eq!(
            plain.driver_options_as::<ModbusOptions>(),
            Some(&ModbusOptions::default()),
            "no parameter, every upstream default"
        );
    }

    #[test]
    fn parse_config_refuses_unknown_keys_and_bad_values() {
        for uri in [
            "modbus-tcp://host?surprise=1",
            "modbus-tcp://host?x=",
            "modbus-tcp://host?address-convention=other",
            "modbus-tcp://host?request-timeout-ms=0",
        ] {
            assert_eq!(refused(uri), ErrorKind::InvalidConfig, "{uri}");
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
            "an unknown convention name is refused"
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
        assert!(capabilities.read && capabilities.write);
        assert!(!capabilities.browse && !capabilities.discover);
        assert_eq!(
            capabilities.max_pdu_bytes.map(std::num::NonZeroUsize::get),
            Some(253)
        );
        assert_eq!(capabilities.concurrency.get(), 1);
        assert_eq!(capabilities.subscribe, crate::SubscriptionMode::Polling);
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
            unit_id: crate::modbus::UnitId::from(9),
            ..ConnectionDefaults::default()
        };
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()))
            .with_defaults(defaults);
        assert_eq!(driver.defaults.unit_id, crate::modbus::UnitId::from(9));
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

    // covers: S26 / the driver defaults seed the parsed options; the connection string wins
    #[test]
    fn driver_defaults_seed_the_parsed_options_and_the_string_overrides_them() {
        let defaults = ConnectionDefaults {
            unit_id: crate::modbus::UnitId::from(9),
            byte_order: ByteOrder::LittleEndian,
            word_order: WordOrder::LowWordFirst,
            ..ConnectionDefaults::default()
        };
        let driver = ModbusTcpDriver::new().with_defaults(defaults);
        let config = driver.parse_config("modbus-tcp://host").expect("config");
        let options = config
            .driver_options_as::<ModbusOptions>()
            .expect("options");
        assert_eq!(options.connection_defaults(), defaults);
        assert_eq!(
            options.request_timeout,
            std::time::Duration::from_millis(5000)
        );
        let config = driver
            .parse_config("modbus-tcp://host?default-unit-identifier=3")
            .expect("config");
        let options = config
            .driver_options_as::<ModbusOptions>()
            .expect("options");
        assert_eq!(options.unit_id, crate::modbus::UnitId::from(3));
        assert_eq!(options.byte_order, ByteOrder::LittleEndian);
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
