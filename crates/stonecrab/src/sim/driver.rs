//! `SimDriver`: the `sim` scheme, a registry of named simulated devices.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use crate::{
    AccessPolicy, AddressOptions, Capabilities, Clock, Connection, ConnectionConfig, ConnectionUri,
    Driver, Endpoint, ErrorKind, OpFuture, PlcError, ProtocolAddress, Result,
};

use crate::sim::connection::Target;
use crate::sim::{RealtimeClock, SimAddress, SimConnection, SimDevice};

/// The connection-string scheme of the simulator.
const SCHEME: &str = "sim";

/// Longest device name, in bytes.
const MAX_NAME_BYTES: usize = 64;

/// Largest `latency-ms`: one hour. A larger value is a typo, and the bound keeps every
/// clock arithmetic step far from overflow.
const MAX_LATENCY_MS: u64 = 3_600_000;

/// The simulator's [`Driver`]: scheme `sim`, backed by named in-process devices.
///
/// A driver holds a clock and a set of named [`SimDevice`]s; it opens a [`SimConnection`]
/// to one of them per `connect`. Like every driver it keeps no per-connection state: the
/// devices are shared by the connections that open them (that is how a write by one
/// connection is visible to a read by another), and everything else lives in the
/// connection.
///
/// # Connection strings
/// `sim://<device>?latency-ms=<n>&seed=<u64>`
///
/// - The authority names a device registered with [`with_device`](Self::with_device); an
///   unknown name is [`ErrorKind::InvalidConfig`].
/// - `latency-ms` (optional, whole milliseconds, at most one hour) replaces the device's
///   own latency for this connection. With the testkit's `FakeClock` it is the time every
///   connect, read, write and close waits.
/// - `seed` (optional, `u64`) restarts the device's `RANDOM` generator when the connection
///   becomes `Connected`. The generator belongs to the device, so it is shared by all of
///   the device's connections.
/// - `address-convention` is accepted because every driver accepts that shared key, and is
///   carried in the configuration's [`AddressOptions`], but the simulator ignores it: its
///   address grammar has no conventions.
/// - Any other option is [`ErrorKind::InvalidConfig`] (decision D12). Upstream's
///   `simulated:<name>` form is not aliased. A transport code (`sim:tcp://`) is rejected.
///
/// The configuration this produces has [`Endpoint::Custom`] holding the device name (with
/// `?latency-ms=..&seed=..` appended when an option was given, which `connect` reads back
/// with the same grammar), default [`Timeouts`](crate::Timeouts) and default
/// limits and policies, with one deliberate exception: its access policy is
/// [`AccessPolicy::ReadWrite`] and not the usual read-only default, because a simulated
/// device has nothing to protect. A caller that wants a read-only simulated connection
/// builds the configuration itself.
///
/// # Clock
/// [`new`](Self::new) uses [`RealtimeClock`], so deadlines built from `Instant::now()`
/// agree with the connection's clock and no operation ever waits. [`with_clock`](Self::with_clock)
/// injects the testkit's `FakeClock`; a caller who does so derives its deadlines from the
/// same clock (ADR-0007, ADR-0012).
///
/// # Compatibility: docs/parity/sim.md#SIM-DRIVER
/// Stonecrab-owned; PLC4X's `SimulatedDriver` uses `simulated:<name>://`-style strings and
/// is not aliased.
///
/// # Examples
/// ```
/// # #[cfg(feature = "testkit")]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::{
///     ConnectionState, DriverRegistry, PlcValue, ReadRequest, Tag, TagId, WriteRequest,
/// };
/// use stonecrab::sim::{SimAddress, SimDevice, SimDriver, SimTag};
/// use stonecrab::testkit::{FakeClock, block_on_with};
///
/// let clock = Arc::new(FakeClock::default());
/// let device = SimDevice::new();
/// device.set("boiler", SimTag::new(PlcValue::F32(71.5)))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new().with_clock(clock.clone()).with_device("bench", device)?,
/// ))?;
/// assert_eq!(registry.schemes(), ["sim"]);
///
/// let connection = block_on_with(&clock, registry.connect("sim://bench?latency-ms=5"))?;
/// assert_eq!(connection.state(), ConnectionState::Connected);
///
/// let tag = Tag::new(TagId::new("boiler")?, Arc::new(SimAddress::parse("STATE/boiler:f32")?));
/// let write = WriteRequest::builder(connection.limits())
///     .item(tag.clone(), PlcValue::F32(72.0))
///     .build()?;
/// assert_eq!(block_on_with(&clock, connection.write(write))?.confirmed_count(), 1);
///
/// let read = ReadRequest::builder(connection.limits()).tag(tag).build()?;
/// let response = block_on_with(&clock, connection.read(read))?;
/// let value = response.items()[0].outcome().as_ref().ok().cloned();
/// assert_eq!(value, Some(PlcValue::F32(72.0)));
/// # Ok::<(), stonecrab::PlcError>(())
/// # }
/// # #[cfg(not(feature = "testkit"))]
/// # fn main() {}
/// ```
pub struct SimDriver {
    clock: Arc<dyn Clock>,
    devices: BTreeMap<String, Arc<SimDevice>>,
}

impl SimDriver {
    /// A driver with no devices that runs on the [`RealtimeClock`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Driver;
    /// use stonecrab::sim::SimDriver;
    ///
    /// assert_eq!(SimDriver::new().scheme(), "sim");
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            clock: Arc::new(RealtimeClock::new()),
            devices: BTreeMap::new(),
        }
    }

    /// Replace the clock every connection of this driver uses.
    ///
    /// Inject the testkit's `FakeClock` to make waiting deterministic; derive request
    /// deadlines from the same clock.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() {
    /// use std::sync::Arc;
    /// use stonecrab::sim::SimDriver;
    /// use stonecrab::testkit::FakeClock;
    ///
    /// let driver = SimDriver::new().with_clock(Arc::new(FakeClock::default()));
    /// assert!(driver.device("bench").is_none());
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Register `device` under `name`, which `sim://<name>` then refers to.
    ///
    /// A name is 1 to 64 bytes of ASCII letters, digits, `-`, `_` and `.`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `name` breaks that rule or a device is
    /// already registered under it.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::ErrorKind;
    /// use stonecrab::sim::{SimDevice, SimDriver};
    ///
    /// let driver = SimDriver::new().with_device("bench", SimDevice::new())?;
    /// let duplicate = driver.with_device("bench", SimDevice::new());
    /// assert_eq!(duplicate.err().map(|e| e.kind()), Some(ErrorKind::InvalidConfig));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    pub fn with_device(mut self, name: &str, device: SimDevice) -> Result<Self> {
        let valid = (1..=MAX_NAME_BYTES).contains(&name.len())
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
        if !valid {
            return Err(invalid(
                "a device name is 1 to 64 ASCII letters, digits, '-', '_' or '.'",
            ));
        }
        if self.devices.contains_key(name) {
            return Err(invalid("a device is already registered under that name"));
        }
        self.devices.insert(name.to_owned(), Arc::new(device));
        Ok(self)
    }

    /// The device registered under `name`, shared with the connections that open it.
    ///
    /// A test keeps this handle to set tags after registration, read the `SINK` count or
    /// inspect a value a write changed.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::PlcValue;
    /// use stonecrab::sim::{SimDevice, SimDriver, SimTag};
    ///
    /// let driver = SimDriver::new().with_device("bench", SimDevice::new())?;
    /// let device = driver.device("bench").map(|d| d.set("t", SimTag::new(PlcValue::U8(1))));
    /// assert!(matches!(device, Some(Ok(()))));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn device(&self, name: &str) -> Option<Arc<SimDevice>> {
        self.devices.get(name).map(Arc::clone)
    }

    /// Check a parsed `sim://` string and extract the device name and its options.
    fn read_uri<'a>(&self, uri: &'a ConnectionUri) -> Result<Resolved<'a>> {
        if uri.scheme() != SCHEME || uri.transport().is_some() {
            return Err(invalid(
                "not a sim connection string: the form is sim://<device>",
            ));
        }
        let Some(device) = self.device(uri.authority()) else {
            return Err(invalid("no simulated device is registered under that name"));
        };
        let mut options = SimOptions::default();
        for (key, value) in uri.options() {
            match key.as_str() {
                "latency-ms" => options.latency_ms = Some(parse_latency(value)?),
                "seed" => options.seed = Some(parse_seed(value)?),
                // Accepted for every driver and ignored by the simulator (see the type docs).
                "address-convention" => {}
                _ => return Err(invalid("unknown sim connection option")),
            }
        }
        Ok(Resolved {
            name: uri.authority(),
            device,
            options,
        })
    }

    /// Resolve a configuration into a connection in the `Disconnected` state.
    fn open(&self, config: &ConnectionConfig) -> Result<SimConnection> {
        let Endpoint::Custom(text) = config.endpoint() else {
            return Err(invalid("a sim configuration needs a custom endpoint"));
        };
        if config.scheme() != SCHEME {
            return Err(invalid("a sim configuration needs the scheme sim"));
        }
        let uri = ConnectionUri::parse(&format!("{SCHEME}://{text}"))?;
        let Resolved {
            name,
            device,
            options,
        } = self.read_uri(&uri)?;
        let latency = options
            .latency_ms
            .map_or_else(|| device.latency(), Duration::from_millis);
        let target = Target {
            name: name.to_owned(),
            device,
            latency,
            seed: options.seed,
        };
        Ok(SimConnection::new(Arc::clone(&self.clock), target, config))
    }
}

impl Default for SimDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for SimDriver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SimDriver")
            .field("devices", &self.devices.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl Driver for SimDriver {
    fn scheme(&self) -> &'static str {
        SCHEME
    }

    /// Read and write, no subscriptions, browse or discovery, one operation at a time.
    ///
    /// The driver cannot know which device a connection will open, so it advertises no
    /// item limit; a connection reports its device's `max_items` in its own
    /// [`Connection::capabilities`].
    fn capabilities(&self) -> Capabilities {
        // `subscribe` (Unsupported) and `concurrency` (one) are the defaults, which the
        // simulator keeps; naming them again produced two equivalent mutants (ADR-0018).
        Capabilities {
            read: true,
            write: true,
            ..Capabilities::default()
        }
    }

    /// Read `sim://<device>?latency-ms=<n>&seed=<u64>`; see the type documentation.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for text that is not a valid connection string,
    /// another scheme, a transport code, an unregistered device, an unknown option, a
    /// `latency-ms` that is not a whole number up to 3600000, a `seed` that is not a `u64`,
    /// or an invalid `address-convention`.
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig> {
        let parsed = ConnectionUri::parse(uri)?;
        let Resolved { name, options, .. } = self.read_uri(&parsed)?;
        let mut endpoint = name.to_owned();
        let pairs = [
            options.latency_ms.map(|ms| format!("latency-ms={ms}")),
            options.seed.map(|seed| format!("seed={seed}")),
        ];
        let pairs: Vec<String> = pairs.into_iter().flatten().collect();
        if !pairs.is_empty() {
            endpoint = format!("{endpoint}?{}", pairs.join("&"));
        }
        let mut builder = ConnectionConfig::builder(SCHEME, Endpoint::Custom(endpoint))
            .access(AccessPolicy::ReadWrite);
        if let Some(convention) = parsed.option("address-convention") {
            builder = builder.address_convention(convention);
        }
        builder.build()
    }

    /// Parse `STATE|RANDOM|SINK/<name>:<type>`; `options` is ignored.
    ///
    /// The simulator's grammar has no conventions, so [`AddressOptions::convention`] changes
    /// nothing: the same text parses the same way whatever the caller selected.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`] as [`SimAddress::parse`] does.
    fn parse_address(
        &self,
        text: &str,
        _options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>> {
        Ok(Arc::new(SimAddress::parse(text)?))
    }

    /// Open a [`SimConnection`] to the configured device.
    ///
    /// The connect waits once, on the driver's clock, for the device latency and is bounded
    /// by `config.timeouts().connect`: a latency that reaches it is
    /// [`ErrorKind::Timeout`]. The `seed` option, if any, reseeds the device when the
    /// connection becomes `Connected`.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] for a configuration that is not a sim one (a
    /// different scheme, a non-custom endpoint, an unregistered device or an unknown option)
    /// and [`ErrorKind::Timeout`] when the latency reaches the connect timeout.
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future abandons the connect; the half-built connection is
    /// dropped with it and the device is untouched until the connection becomes `Connected`.
    /// The deadline source is `config.timeouts().connect`.
    ///
    /// # Wire effects
    /// None. There is no wire.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>> {
        match self.open(&config) {
            Ok(connection) => Box::pin(async move {
                connection.establish().await?;
                let opened: Box<dyn Connection> = Box::new(connection);
                Ok(opened)
            }),
            Err(error) => Box::pin(std::future::ready(Err(error))),
        }
    }
}

/// A `sim://` string checked against the registered devices.
struct Resolved<'a> {
    name: &'a str,
    device: Arc<SimDevice>,
    options: SimOptions,
}

/// The options of a `sim://` string.
#[derive(Debug, Default)]
struct SimOptions {
    latency_ms: Option<u64>,
    seed: Option<u64>,
}

fn parse_latency(value: &str) -> Result<u64> {
    match value.parse::<u64>() {
        Ok(ms) if ms <= MAX_LATENCY_MS => Ok(ms),
        _ => Err(invalid(
            "latency-ms is a whole number of milliseconds up to 3600000",
        )),
    }
}

fn parse_seed(value: &str) -> Result<u64> {
    value
        .parse::<u64>()
        .ok()
        .ok_or_else(|| invalid("seed is an unsigned 64-bit whole number"))
}

fn invalid(message: &'static str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}
