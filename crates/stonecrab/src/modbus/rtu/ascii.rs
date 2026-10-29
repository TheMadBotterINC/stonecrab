//! `ModbusAsciiDriver`: the `Driver` of the `modbus-ascii` scheme.

use std::fmt;
use std::sync::Arc;

use crate::transport::{SystemClock, TransportFactory};
use crate::{
    AddressOptions, Capabilities, Clock, Connection, ConnectionConfig, Driver, OpFuture,
    ProtocolAddress, Result,
};

use crate::modbus::connection::driver_capabilities;
use crate::modbus::rtu::driver::{connect_serial, open_serial, parse_serial_config};
use crate::modbus::session::Wire;
use crate::modbus::{ModbusAddress, ModbusConnection};

/// The `Driver` of the `modbus-ascii` scheme: Modbus ASCII (a colon, the address, the PDU and
/// an LRC as upper-case hex, then CR LF, on a serial line) with the reads, writes, ping and
/// polling subscriptions of `modbus-tcp`.
///
/// It is [`ModbusRtuDriver`](crate::modbus::ModbusRtuDriver) with another frame. The PDU
/// codec, the address grammar, the batcher, the typed decoder, the byte-order modes, the
/// exception mapping and the whole [`ModbusConnection`](crate::modbus::ModbusConnection) are
/// the ones of `modbus-tcp`; the framing, the correlation and the line timing differ
/// (ADR-0024, addendum S60):
///
/// - **Framing.** [`AsciiFramer`](crate::modbus::AsciiFramer): a reply is the characters
///   between a colon and the first CR LF, hex digits in either case, with a right LRC. A
///   reply that fails the hex or the LRC fails its request with
///   [`ErrorKind::MalformedData`](crate::ErrorKind::MalformedData) (a read may be retried by
///   the configuration's retry policy), and the bytes are dropped before the next request.
///   Noise in front of a colon, or a broken frame in front of a good one, is skipped.
/// - **Correlation, one request on the line, no broadcast.** As `modbus-rtu`: a reply is
///   accepted when its address and function code match the request, one operation runs at a
///   time and a port is not shared between connections, and unit 0 is refused.
/// - **Timing.** No inter-character timeout and no silence between frames: a frame is
///   bounded by its colon and CR LF, slow characters are accepted until the request's
///   deadline, and the line is not watched before a request unless
///   `serial.interframe-delay` asks for a gap. After a request nothing answered the line
///   is watched for the time of the largest frame (513 characters; 534 ms at 9600 8N1) and
///   what arrives is dropped, so a late reply cannot answer the next request. Over a gateway
///   (`modbus-ascii:tcp://`) the line is not known and the drain is that of a 9600 8N1 line.
///
/// [`new`](Self::new) opens real serial ports (or TCP connections) and uses the Tokio-backed
/// [`SystemClock`]; [`with_clock`](Self::with_clock) and
/// [`with_transport_factory`](Self::with_transport_factory) replace them, as for
/// `modbus-tcp`.
///
/// # Connection strings
/// `modbus-ascii://<port>` or `modbus-ascii:serial://<port>`, where `<port>` is the serial
/// port name as the operating system knows it, passed through as written:
/// `modbus-ascii:///dev/ttyUSB0`, `modbus-ascii://COM3`. `modbus-ascii:tcp://host[:port]`
/// carries ASCII frames over TCP to a serial-to-IP gateway, default port 502. The transport
/// codes `tls`, `tls-psk`, `udp` and `test`, and every other, are
/// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig).
///
/// **Parameters.** Those of `modbus-rtu`, with one difference: `serial.data-bits` is 8
/// (the default, as upstream) or 7, which most ASCII devices use with even parity
/// (`serial.data-bits=7&serial.parity=even`); 5 and 6 are
/// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig). `serial.interframe-delay`
/// is a gap (milliseconds, default 0) watched before every request, not a floor on t3.5.
///
/// # Compatibility: docs/parity/modbus-serial.md#MBS-OP-CONNECT
/// Protocol code `modbus-ascii` and the serial default as upstream; the carriers are
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
/// use stonecrab::modbus::ModbusAsciiDriver;
/// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
///
/// let clock = FakeClock::default();
/// let factory = FakeTransportFactory::new(clock.clone());
/// factory.push_script(Script::default());
/// let driver = ModbusAsciiDriver::new()
///     .with_clock(Arc::new(clock.clone()))
///     .with_transport_factory(Arc::new(factory));
/// assert_eq!(driver.scheme(), "modbus-ascii");
///
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(driver))?;
/// let connection = block_on_with(
///     &clock,
///     registry.connect(
///         "modbus-ascii:///dev/ttyUSB0\
///          ?serial.data-bits=7&serial.parity=even&serial.baud-rate=19200",
///     ),
/// )?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct ModbusAsciiDriver {
    clock: Arc<dyn Clock>,
    /// The factory every connection opens its transport with; `None` opens the carrier
    /// the connection string names.
    factory: Option<Arc<dyn TransportFactory>>,
}

impl ModbusAsciiDriver {
    /// A driver over real serial ports (and TCP for the `tcp` carrier) and the
    /// [`SystemClock`].
    ///
    /// Nothing is opened until a connection is requested. Connections must be driven
    /// inside a Tokio runtime with the time driver enabled, as [`SystemClock`] documents.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Driver;
    /// use stonecrab::modbus::ModbusAsciiDriver;
    ///
    /// let driver = ModbusAsciiDriver::new();
    /// assert_eq!(driver.scheme(), "modbus-ascii");
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
    /// drain after an unanswered request run on it.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() {
    /// use std::sync::Arc;
    /// use stonecrab::modbus::ModbusAsciiDriver;
    /// use stonecrab::testkit::FakeClock;
    ///
    /// let driver = ModbusAsciiDriver::new().with_clock(Arc::new(FakeClock::default()));
    /// assert!(format!("{driver:?}").contains("ModbusAsciiDriver"));
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
    /// `FakeTransportFactory` serves in-memory transports.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionState, Driver};
    /// use stonecrab::modbus::ModbusAsciiDriver;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let driver = ModbusAsciiDriver::new()
    ///     .with_clock(Arc::new(clock.clone()))
    ///     .with_transport_factory(Arc::new(factory.clone()));
    ///
    /// let config = driver.parse_config("modbus-ascii://COM3")?;
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
    /// use stonecrab::modbus::ModbusAsciiDriver;
    /// use stonecrab::testkit::{FakeClock, FakeTransportFactory, Script, block_on_with};
    ///
    /// let clock = FakeClock::default();
    /// let factory = FakeTransportFactory::new(clock.clone());
    /// factory.push_script(Script::default());
    /// let driver = ModbusAsciiDriver::new()
    ///     .with_clock(Arc::new(clock.clone()))
    ///     .with_transport_factory(Arc::new(factory));
    ///
    /// let config = driver.parse_config("modbus-ascii://COM3")?;
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
        open_serial(Wire::Ascii, config, &self.clock, self.factory.as_ref())
    }
}

impl Default for ModbusAsciiDriver {
    fn default() -> Self {
        Self::new()
    }
}

/// Says whether a factory was injected; the clock and the factory have no useful `Debug`.
impl fmt::Debug for ModbusAsciiDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModbusAsciiDriver")
            .field("injected_factory", &self.factory.is_some())
            .finish_non_exhaustive()
    }
}

impl Driver for ModbusAsciiDriver {
    fn scheme(&self) -> &'static str {
        Wire::Ascii.scheme()
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

    /// Read `modbus-ascii[:serial|:tcp]://<port or host[:port]>[?parameters]` into a
    /// validated configuration; see the type for the grammar and the parameters.
    ///
    /// The configuration has an `Endpoint::Custom` holding the port name for a serial line
    /// and an `Endpoint::Tcp` for a gateway, the request timeout, the address convention,
    /// the access policy and the parsed options in its driver options slot, as
    /// `modbus-rtu` does.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) for text that
    /// is not a valid connection string, another scheme, a transport code other than
    /// `serial` and `tcp`, a gateway authority that is not `host[:port]`, a parameter
    /// `modbus-tcp` refuses, `default-unit-identifier=0`, an unknown `serial.` key or a
    /// value it does not take (`serial.data-bits` other than 7 and 8 included), and a
    /// `serial.` key on the `tcp` carrier.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-CFG-TRANSPORT-OPTIONS
    /// The `serial.` prefix as upstream; the line options are the rows
    /// docs/parity/modbus-serial.md#MBS-CFG-SERIAL-BAUD to
    /// docs/parity/modbus-serial.md#MBS-CFG-SERIAL-INTERFRAME, unknown keys are refused as
    /// for `modbus-tcp` (docs/parity/modbus-serial.md#MBS-CFG-UNKNOWN) and the carriers are
    /// docs/parity/modbus-serial.md#MBS-TRANS-TLS and its neighbours.
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        parse_serial_config(Wire::Ascii, uri)
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

    /// Open a [`ModbusConnection`](crate::modbus::ModbusConnection) over the configured
    /// carrier: the serial port of an `Endpoint::Custom` with the parsed line settings, or
    /// TCP for an `Endpoint::Tcp` (the injected factory, when there is one), with the drain
    /// after an unanswered request and the gap of `serial.interframe-delay`.
    ///
    /// A configuration built by hand carries no parsed options and gets the defaults: 9600
    /// baud, 8 data bits, no parity, one stop bit, unit 1.
    ///
    /// # Errors
    /// As [`ModbusConnection::open`](crate::modbus::ModbusConnection::open), for the scheme
    /// `modbus-ascii` and a frame limit of at least 513 bytes:
    /// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) for a configuration of
    /// another scheme, [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) when the open does
    /// not finish in time and [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected)
    /// for a port that is missing, not permitted or refuses the settings.
    ///
    /// # Cancel safety
    /// Cancel-safe, as [`ModbusConnection::open`](crate::modbus::ModbusConnection::open).
    ///
    /// # Wire effects
    /// Opens the port (or the TCP connection) and sends nothing.
    ///
    /// # Compatibility: docs/parity/modbus-serial.md#MBS-OP-CONNECT
    /// Connect opens the transport and sends nothing, as upstream.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        connect_serial(Wire::Ascii, config, &self.clock, self.factory.as_ref())
    }
}
