//! `ModbusRtuDriver`: the `Driver` of the `modbus-rtu` scheme, and the parsing and the
//! connecting both serial drivers share (the `modbus-ascii` driver calls them with its own
//! framing).

use std::fmt;
use std::sync::Arc;

use crate::transport::{SystemClock, TcpFactory, TransportFactory};
use crate::{
    AddressOptions, Capabilities, Clock, Connection, ConnectionConfig, ConnectionUri, Driver,
    Endpoint, OpFuture, ProtocolAddress, Result, Timeouts,
};

use crate::modbus::connection::driver_capabilities;
use crate::modbus::driver::endpoint;
use crate::modbus::options::invalid;
use crate::modbus::rtu::line::SerialOptions;
use crate::modbus::rtu::pace::PacedFactory;
use crate::modbus::session::Wire;
use crate::modbus::{ModbusAddress, ModbusConnection};

/// The connection-string scheme of this driver.
const SCHEME: &str = "modbus-rtu";

/// The transport code of a serial line, the default.
const SERIAL: &str = "serial";

/// The transport code of a serial-to-IP gateway that passes RTU frames over TCP.
const TCP: &str = "tcp";

/// The `Driver` of the `modbus-rtu` scheme: Modbus RTU (the address, the PDU and a CRC-16
/// on a serial line) with the reads, writes, ping and polling subscriptions of
/// `modbus-tcp`.
///
/// The PDU codec, the address grammar, the batcher, the typed decoder, the byte-order
/// modes, the exception mapping and the whole [`ModbusConnection`] are the ones of
/// `modbus-tcp`; only the framing, the correlation and the line timing differ (ADR-0024):
///
/// - **Framing.** [`RtuFramer`](crate::modbus::RtuFramer): a reply is sized by its function
///   code and checked by its CRC. A reply that fails the CRC fails its request with
///   [`ErrorKind::MalformedData`](crate::ErrorKind::MalformedData) (a read may be
///   retried by the configuration's retry policy), and the bytes are dropped before the
///   next request.
/// - **Correlation.** RTU has no transaction id: a reply is accepted when its address and
///   function code (or the exception form of it) match the request, and the PDU is then
///   proved against the request as for TCP. A reply from another unit is discarded and the
///   wait goes on.
/// - **One request on the line.** The connection runs one operation at a time, so a port
///   carries one request at a time. A port is not shared between connections
///   (`serial.reuse-port=true` is refused); open one connection per port.
/// - **Silence.** Before every request the line is watched until it has been quiet for
///   3.5 character times (1.75 ms above 19200 baud) or `serial.interframe-delay`, whichever
///   is longer, and what arrives meanwhile is dropped. After a request nothing answered,
///   the quiet time is longer by the time a 256-byte frame takes, so a late reply that was
///   on its way is heard out and dropped instead of answering the next request. The watch
///   is part of the request's deadline. Over a gateway (`modbus-rtu:tcp://`) the line is
///   not known: there is no gap, and the drain after an unanswered request is that of a
///   9600 8N1 line.
/// - **No broadcast.** Unit 0 gets no reply on a serial line; `default-unit-identifier=0`
///   is refused, and a tag addressed to unit 0 fails with
///   [`ErrorKind::InvalidAddress`](crate::ErrorKind::InvalidAddress) before any I/O.
///
/// [`new`](Self::new) opens real serial ports (or TCP connections) and uses the Tokio-backed
/// [`SystemClock`]; [`with_clock`](Self::with_clock) and
/// [`with_transport_factory`](Self::with_transport_factory) replace them, as for
/// `modbus-tcp`.
///
/// # Connection strings
/// `modbus-rtu://<port>` or `modbus-rtu:serial://<port>`, where `<port>` is the serial
/// port name as the operating system knows it, passed through as written:
/// `modbus-rtu:///dev/ttyUSB0`, `modbus-rtu://COM3`. `modbus-rtu:tcp://host[:port]`
/// carries RTU frames over TCP to a serial-to-IP gateway, default port 502. The transport
/// codes `tls`, `tls-psk`, `udp` and `test`, and every other, are
/// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig).
///
/// **Parameters.** The parameters of `modbus-tcp` (`request-timeout-ms`,
/// `default-unit-identifier` from 1 to 255, `ping-address`, `default-payload-byte-order`,
/// `max-coils-per-request`, `max-registers-per-request`, `address-convention`, `access`),
/// and for a serial port the line under the `serial.` prefix: `serial.baud-rate` (default
/// 9600), `serial.data-bits` (8, the only value RTU allows), `serial.stop-bits` (1 or 2,
/// default 1), `serial.parity` (`none`, `odd` or `even`, default `none`),
/// `serial.flow-control` (`none`, `rts-cts` or `xon-xoff`, also `rtscts` and `xonxoff`,
/// default `none`), `serial.interframe-delay` (milliseconds, default 0) and
/// `serial.reuse-port` (`false` only). Names of values are matched without regard to case
/// and with `_` read as `-`, as upstream does. Every other key, a `serial.` key on the
/// `tcp` carrier and an unprefixed line setting such as `baud-rate` are
/// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig).
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-OP-CONNECT
/// Protocol code `modbus-rtu` and the serial default as upstream; the carriers are
/// docs/parity/modbus-serial.md#MBS-TRANS-SERIAL and
/// docs/parity/modbus-serial.md#MBS-TRANS-TCP. [`capabilities`](Driver::capabilities) is the
/// value of `modbus-tcp` (docs/parity/modbus-serial.md#MBS-CAP-FLAGS).
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::{ConnectionState, Driver, DriverRegistry};
/// use stonecrab::modbus::ModbusRtuDriver;
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::default());
/// let driver = ModbusRtuDriver::new()
///     .with_clock(Arc::new(clock.clone()))
///     .with_transport_factory(Arc::new(factory));
/// assert_eq!(driver.scheme(), "modbus-rtu");
///
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(driver))?;
/// let connection = block_on_with(
///     &clock,
///     registry.connect("modbus-rtu:///dev/ttyUSB0?serial.baud-rate=19200&serial.parity=even"),
/// )?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct ModbusRtuDriver {
    clock: Arc<dyn Clock>,
    /// The factory every connection opens its transport with; `None` opens the carrier
    /// the connection string names.
    factory: Option<Arc<dyn TransportFactory>>,
}

impl ModbusRtuDriver {
    /// A driver over real serial ports (and TCP for the `tcp` carrier) and the
    /// [`SystemClock`].
    ///
    /// Nothing is opened until a connection is requested. Connections must be driven
    /// inside a Tokio runtime with the time driver enabled, as [`SystemClock`] documents.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Driver;
    /// use stonecrab::modbus::ModbusRtuDriver;
    ///
    /// let driver = ModbusRtuDriver::new();
    /// assert_eq!(driver.scheme(), "modbus-rtu");
    /// assert!(driver.capabilities().read && driver.capabilities().write);
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(SystemClock::default()),
            factory: None,
        }
    }

    /// Replace the clock every connection of this driver uses: deadlines, backoff and the
    /// silence before each request run on it.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() {
    /// use std::sync::Arc;
    /// use stonecrab::modbus::ModbusRtuDriver;
    /// use stonecrab::testkit::FakeClock;
    ///
    /// let driver = ModbusRtuDriver::new().with_clock(Arc::new(FakeClock::default()));
    /// assert!(format!("{driver:?}").contains("ModbusRtuDriver"));
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Replace the factory that opens a transport for every connection, whatever carrier
    /// the connection string names.
    ///
    /// This is how a test runs the driver without a serial port: the testkit's
    /// `FakeTransportFactory` serves in-memory transports. The silence before each request
    /// still applies, computed from the connection string's line settings, on the driver's
    /// clock.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Driver};
    /// use stonecrab::modbus::ModbusRtuDriver;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let driver = ModbusRtuDriver::new()
    ///     .with_clock(Arc::new(clock.clone()))
    ///     .with_transport_factory(Arc::new(factory.clone()));
    ///
    /// let config = driver.parse_config("modbus-rtu://COM3")?;
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

    /// Open a [`ModbusConnection`] as [`connect`](Driver::connect) does, but as the concrete
    /// type, so that [`stats`](ModbusConnection::stats) can be reached: `connect` returns a
    /// `Box<dyn Connection>`, which has no way back to the concrete type.
    ///
    /// # Errors
    /// Those of [`connect`](Driver::connect).
    ///
    /// # Cancel safety
    /// Cancel-safe, as [`connect`](Driver::connect).
    ///
    /// # Wire effects
    /// Opens the port (or the TCP connection) and sends nothing.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{Connection, ConnectionState, Driver};
    /// use stonecrab::modbus::ModbusRtuDriver;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let driver = ModbusRtuDriver::new()
    ///     .with_clock(Arc::new(clock.clone()))
    ///     .with_transport_factory(Arc::new(factory));
    ///
    /// let config = driver.parse_config("modbus-rtu://COM3")?;
    /// let connection = block_on_with(&clock, driver.open(config))?;
    /// assert_eq!(connection.state(), ConnectionState::Connected);
    /// assert_eq!(connection.stats().mismatched_replies, 0);
    /// assert_eq!(connection.stats().guard.requests, 0);
    /// # Ok::<(), stonecrab::PlcError>(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    pub fn open(&self, config: ConnectionConfig) -> OpFuture<'_, ModbusConnection> {
        open_serial(Wire::Rtu, config, &self.clock, self.factory.as_ref())
    }
}

impl Default for ModbusRtuDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Says whether a factory was injected; the clock and the factory have no useful `Debug`.
impl fmt::Debug for ModbusRtuDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusRtuDriver")
            .field("injected_factory", &self.factory.is_some())
            .finish_non_exhaustive()
    }
}

impl Driver for ModbusRtuDriver {
    fn scheme(&self) -> &'static str {
        SCHEME
    }

    /// The capabilities of `modbus-tcp`: read, write, ping and polling subscriptions, no
    /// browse or discovery, one operation at a time, PDUs up to 253 bytes.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-CAP-FLAGS
    /// Read, write and ping as upstream's flags; subscribe is `Polling` as for
    /// `modbus-tcp`, whose connection metadata upstream shares
    /// (docs/parity/modbus-serial.md#MBS-OP-SUBSCRIBE-POLL).
    fn capabilities(&self) -> Capabilities {
        driver_capabilities()
    }

    /// Read `modbus-rtu[:serial|:tcp]://<port or host[:port]>[?parameters]` into a
    /// validated configuration; see the type for the grammar and the parameters.
    ///
    /// The configuration has an `Endpoint::Custom` holding the port name for a serial line
    /// and an `Endpoint::Tcp` for a gateway, the request timeout, the address convention,
    /// the access policy and the parsed options in its driver options slot, as
    /// `modbus-tcp` does.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) for text that
    /// is not a valid connection string, another scheme, a transport code other than
    /// `serial` and `tcp`, a gateway authority that is not `host[:port]`, a parameter
    /// `modbus-tcp` refuses, `default-unit-identifier=0`, an unknown `serial.` key or a
    /// value it does not take, and a `serial.` key on the `tcp` carrier.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-CFG-TRANSPORT-OPTIONS
    /// The `serial.` prefix as upstream; the line options are the rows
    /// docs/parity/modbus-serial.md#MBS-CFG-SERIAL-BAUD to
    /// docs/parity/modbus-serial.md#MBS-CFG-SERIAL-INTERFRAME, unknown keys are refused as
    /// for `modbus-tcp` (docs/parity/modbus-serial.md#MBS-CFG-UNKNOWN) and the carriers are
    /// docs/parity/modbus-serial.md#MBS-TRANS-TLS and its neighbours.
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        parse_serial_config(Wire::Rtu, uri)
    }

    /// Parse Modbus address text with the grammar of `modbus-tcp`; see
    /// [`ModbusAddress::parse`].
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`](crate::ErrorKind::InvalidAddress) as
    /// [`ModbusAddress::parse`] does.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-ADDR-SHARED
    /// One grammar for every Modbus driver, as upstream.
    fn parse_address(
        &self,
        text: &str,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(ModbusAddress::parse(text, options)?))
    }

    /// Open a [`ModbusConnection`] over the configured carrier: the serial port of an
    /// `Endpoint::Custom` with the parsed line settings, or TCP for an `Endpoint::Tcp`
    /// (the injected factory, when there is one), paced by the silence of the line.
    ///
    /// A configuration built by hand carries no parsed options and gets the defaults: 9600
    /// baud, 8 data bits, no parity, one stop bit, unit 1.
    ///
    /// # Errors
    /// As [`ModbusConnection::open`], for the scheme `modbus-rtu` and a frame limit of at
    /// least 256 bytes: [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) for a
    /// configuration of another scheme, [`ErrorKind::Timeout`](crate::ErrorKind::Timeout)
    /// when the open does not finish in time and
    /// [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) for a port that is
    /// missing, not permitted or refuses the settings.
    ///
    /// # Cancel safety
    /// Cancel-safe, as [`ModbusConnection::open`].
    ///
    /// # Wire effects
    /// Opens the port (or the TCP connection) and sends nothing.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-OP-CONNECT
    /// Connect opens the transport and sends nothing, as upstream.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        connect_serial(Wire::Rtu, config, &self.clock, self.factory.as_ref())
    }
}

/// Read `modbus-rtu` or `modbus-ascii` (by `wire`) `[:serial|:tcp]://<port or
/// host[:port]>[?parameters]` into a validated configuration.
pub(super) fn parse_serial_config(wire: Wire, uri: &str) -> Result<ConnectionConfig> {
    let scheme = wire.scheme();
    let parsed = ConnectionUri::parse(uri)?;
    if parsed.scheme() != scheme {
        return Err(invalid(format!("not a {scheme} connection string")));
    }
    let endpoint = match parsed.transport() {
        None | Some(SERIAL) => Endpoint::Custom(parsed.authority().to_owned()),
        Some(TCP) => endpoint(parsed.authority())?,
        Some(_) => {
            return Err(invalid(format!(
                "the transport code of {scheme} is serial or tcp; tls, tls-psk, udp and test \
                 are not supported"
            )));
        }
    };
    let serial = matches!(endpoint, Endpoint::Custom(_));
    let (options, convention, access) =
        SerialOptions::from_parameters(parsed.options(), serial, wire)?;
    let timeouts = Timeouts {
        request: options.modbus.request_timeout,
        ..Timeouts::default()
    };
    let mut builder = ConnectionConfig::builder(scheme, endpoint)
        .timeouts(timeouts)
        .access(access)
        .driver_options(Arc::new(options));
    if let Some(convention) = convention {
        builder = builder.address_convention(convention.name());
    }
    builder.build()
}

/// Open a [`ModbusConnection`] speaking `wire` over the carrier `config` names (the injected
/// `factory` when there is one), paced by the silence of the line.
pub(super) fn open_serial<'a>(
    wire: Wire,
    config: ConnectionConfig,
    clock: &Arc<dyn Clock>,
    factory: Option<&Arc<dyn TransportFactory>>,
) -> OpFuture<'a, ModbusConnection> {
    let options = config
        .driver_options_as::<SerialOptions>()
        .cloned()
        .unwrap_or_default();
    let serial = matches!(config.endpoint(), Endpoint::Custom(_));
    let carrier: Arc<dyn TransportFactory> = match factory {
        Some(factory) => Arc::clone(factory),
        None if serial => Arc::new(options.line),
        None => Arc::new(TcpFactory::default()),
    };
    let factory = Arc::new(PacedFactory {
        inner: carrier,
        clock: Arc::clone(clock),
        silence: options.silence(serial, wire),
    });
    ModbusConnection::open_on(
        wire,
        config,
        factory,
        Arc::clone(clock),
        options.modbus.connection_defaults(),
        options.modbus.ping_address,
    )
}

/// [`open_serial`] behind the [`Connection`] trait, as a driver's `connect` returns it.
pub(super) fn connect_serial<'a>(
    wire: Wire,
    config: ConnectionConfig,
    clock: &Arc<dyn Clock>,
    factory: Option<&Arc<dyn TransportFactory>>,
) -> OpFuture<'a, Box<dyn Connection>> {
    let opening = open_serial(wire, config, clock, factory);
    Box::pin(async move {
        let connection: Box<dyn Connection> = Box::new(opening.await?);
        Ok(connection)
    })
}
