//! The driver registry: scheme lookup for connection strings.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::{
    Connection, ConnectionConfig, ConnectionUri, Diagnostics, Driver, ErrorKind, FailureCategory,
    OpFuture, Operation, PlcError, Result,
};

/// A set of drivers keyed by connection-string scheme.
///
/// The registry is how an application turns `"sim://bench"` or
/// `"modbus-tcp://192.0.2.10:502"` into a connection without naming a driver type:
/// [`parse`](Self::parse) and [`connect`](Self::connect) read the scheme with
/// [`ConnectionUri::parse`], find the driver registered for it and hand the original text
/// to [`Driver::parse_config`]. No driver parses the authority or the query by hand.
///
/// # Lifecycle and locking
/// A registry is built once and then shared. It holds a `BTreeMap` behind no lock because
/// it is immutable after setup: [`register`](Self::register) takes `&mut self`, so the
/// compiler rejects registering while the registry is shared, and every other method
/// takes `&self` and can run from any number of threads (`Send + Sync`). Put a finished
/// registry in an `Arc` to share it; if drivers must be added later, build a new registry.
///
/// There is no global registry and no discovery: an application registers exactly the
/// drivers it links.
///
/// # Compatibility: docs/parity/core.md#CORE-REGISTRY
/// Corresponds to the upstream `PlcDriverManager`, which resolves a connection string to a
/// driver by protocol code. Upstream discovers drivers with a service loader; this
/// registry is explicit (SPEC.md forbids hidden global state).
///
/// # Examples
/// An empty registry knows no scheme, and says so with a typed error.
/// ```
/// use stonecrab::{DriverRegistry, ErrorKind, Operation};
///
/// let registry = DriverRegistry::new();
/// assert!(registry.schemes().is_empty());
/// assert!(registry.driver("sim").is_none());
/// let refused = registry.parse("sim://bench").err().map(|error| error.kind());
/// assert_eq!(refused, Some(ErrorKind::Unsupported(Operation::Connect)));
/// ```
#[derive(Default)]
pub struct DriverRegistry {
    drivers: BTreeMap<&'static str, Arc<dyn Driver>>,
}

impl DriverRegistry {
    /// An empty registry.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::DriverRegistry;
    /// assert!(DriverRegistry::new().schemes().is_empty());
    /// ```
    #[must_use]
    pub const fn new() -> Self {
        Self {
            drivers: BTreeMap::new(),
        }
    }

    /// Add `driver` under its [`scheme`](Driver::scheme).
    ///
    /// Takes `&mut self`: registration is setup-time only (see the type documentation).
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when a driver is already registered for the
    /// same scheme. The registry is unchanged in that case; the first driver stays.
    ///
    /// # Examples
    /// Registering needs a driver; the simulator's is the one to use in examples.
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{Driver, DriverRegistry, Result};
    ///
    /// fn setup(driver: Arc<dyn Driver>) -> Result<DriverRegistry> {
    ///     let mut registry = DriverRegistry::new();
    ///     registry.register(driver)?;
    ///     Ok(registry)
    /// }
    /// ```
    pub fn register(&mut self, driver: Arc<dyn Driver>) -> Result<()> {
        let scheme = driver.scheme();
        if self.drivers.contains_key(scheme) {
            return Err(PlcError::new(
                ErrorKind::InvalidConfig,
                format!("a driver is already registered for scheme {scheme}"),
            )
            .with_diagnostics(validation(Operation::Connect)));
        }
        self.drivers.insert(scheme, driver);
        Ok(())
    }

    /// The driver registered for `scheme`, if any. The match is exact and case-sensitive.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::DriverRegistry;
    /// assert!(DriverRegistry::new().driver("modbus-tcp").is_none());
    /// ```
    #[must_use]
    pub fn driver(&self, scheme: &str) -> Option<Arc<dyn Driver>> {
        self.drivers.get(scheme).map(Arc::clone)
    }

    /// Every registered scheme, sorted.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::DriverRegistry;
    /// assert_eq!(DriverRegistry::new().schemes(), Vec::<&'static str>::new());
    /// ```
    #[must_use]
    pub fn schemes(&self) -> Vec<&'static str> {
        self.drivers.keys().copied().collect()
    }

    /// Find the driver for `uri` and have it read the connection string.
    ///
    /// The scheme is extracted with [`ConnectionUri::parse`]; the driver then receives the
    /// complete original text in [`Driver::parse_config`]. Nothing is opened.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] when `uri` is not a valid connection string
    /// (see [`ConnectionUri::parse`]) or when the driver rejects its configuration, and
    /// [`ErrorKind::Unsupported`] for [`Operation::Connect`] when no driver is registered
    /// for the scheme.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-REGISTRY
    /// Upstream raises a `PlcConnectionException` for an unknown protocol code.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{DriverRegistry, ErrorKind};
    /// let registry = DriverRegistry::new();
    /// let malformed = registry.parse("not a uri").err().map(|error| error.kind());
    /// assert_eq!(malformed, Some(ErrorKind::InvalidConfig));
    /// ```
    pub fn parse(&self, uri: &str) -> Result<(Arc<dyn Driver>, ConnectionConfig)> {
        let parsed = ConnectionUri::parse(uri)?;
        let Some(driver) = self.driver(parsed.scheme()) else {
            return Err(PlcError::new(
                ErrorKind::Unsupported(Operation::Connect),
                format!("no driver is registered for scheme {}", parsed.scheme()),
            )
            .with_diagnostics(validation(Operation::Connect)));
        };
        let config = driver.parse_config(uri)?;
        Ok((driver, config))
    }

    /// Parse `uri` and open a connection with the driver registered for its scheme.
    ///
    /// The connection string is parsed when this method is called, before the returned
    /// future is polled, and a parse failure is delivered as the future's error. The
    /// attempt is then bounded by `timeouts.connect` of the parsed configuration (see
    /// [`Driver::connect`]).
    ///
    /// # Errors
    /// The kinds of [`parse`](Self::parse) and of [`Driver::connect`].
    ///
    /// # Cancel safety
    /// Cancel-safe, exactly as [`Driver::connect`]: dropping the future abandons the
    /// attempt and leaves no connection behind. The deadline source is the configuration's
    /// `timeouts.connect`.
    ///
    /// # Wire effects
    /// None for a connection string that fails to parse or names no driver. Otherwise the
    /// driver's: it may open a transport and run a handshake.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-REGISTRY
    /// Corresponds to `PlcDriverManager.getConnection`.
    ///
    /// # Examples
    /// Connecting needs a registered driver; the simulator crate's tests and examples run
    /// `registry.connect("sim://bench")` end to end. Against an empty registry the future
    /// resolves to a typed error:
    /// ```
    /// use std::task::{Context, Poll, Waker};
    /// use stonecrab::{DriverRegistry, ErrorKind, Operation};
    ///
    /// let registry = DriverRegistry::new();
    /// let mut connecting = registry.connect("sim://bench");
    /// let mut cx = Context::from_waker(Waker::noop());
    /// let Poll::Ready(outcome) = connecting.as_mut().poll(&mut cx) else {
    ///     unreachable!("an unknown scheme fails without waiting");
    /// };
    /// assert_eq!(
    ///     outcome.err().map(|error| error.kind()),
    ///     Some(ErrorKind::Unsupported(Operation::Connect))
    /// );
    /// ```
    pub fn connect(&self, uri: &str) -> OpFuture<'_, Box<dyn Connection>> {
        match self.parse(uri) {
            Ok((driver, config)) => Box::pin(async move { driver.connect(config).await }),
            Err(error) => Box::pin(std::future::ready(Err(error))),
        }
    }
}

impl fmt::Debug for DriverRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DriverRegistry")
            .field("schemes", &self.schemes())
            .finish()
    }
}

/// Diagnostics for a rejection before any I/O: the operation is known, nothing else is.
fn validation(operation: Operation) -> Diagnostics {
    Diagnostics {
        operation: Some(operation),
        category: Some(FailureCategory::Validation),
        ..Diagnostics::default()
    }
}
