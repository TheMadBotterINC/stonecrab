//! Connection configuration: endpoint, timeouts, reconnect and retry policy, access
//! policy, credentials and resource limits, as validated data.
//!
//! Nothing in this module performs I/O or enforces a policy. The types record what the
//! caller asked for; a driver or transport consumes them (ADR-0008).

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;

use crate::{AddressOptions, ErrorKind, PlcError, ResourceLimits, Result, Secret};

/// Where a connection is made.
///
/// Only the variants the alpha transports need exist; `Udp` and `Serial` arrive with
/// their transports. The enum is `#[non_exhaustive]`, so match with a wildcard arm.
///
/// The text of a [`Endpoint::Custom`] value is displayed verbatim (including by
/// [`ConnectionConfig::redacted_display`]), so it must not embed credentials; pass them
/// through [`Credentials`] instead.
///
/// # Compatibility: docs/parity/core.md#CORE-CONFIG-SHAPE
/// Upstream carries the transport configuration as the text between `://` and `?` of
/// the connection string; `Endpoint::Tcp` is the structured form of `host:port`.
///
/// # Examples
/// ```
/// use stonecrab::Endpoint;
/// let tcp = Endpoint::Tcp { host: "plc.example".to_owned(), port: 502 };
/// assert_eq!(tcp.to_string(), "plc.example:502");
/// let v6 = Endpoint::Tcp { host: "::1".to_owned(), port: 502 };
/// assert_eq!(v6.to_string(), "[::1]:502");
/// assert_eq!(Endpoint::Custom("sim://bench".to_owned()).to_string(), "sim://bench");
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Endpoint {
    /// A TCP peer. Name resolution and host validation belong to the transport.
    Tcp {
        /// Host name or IP address literal, without brackets.
        host: String,
        /// TCP port.
        port: u16,
    },
    /// A scheme-specific endpoint the core does not interpret, such as a simulator
    /// device name.
    Custom(String),
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp { host, port } if host.contains(':') => write!(f, "[{host}]:{port}"),
            Self::Tcp { host, port } => write!(f, "{host}:{port}"),
            Self::Custom(text) => f.write_str(text),
        }
    }
}

/// Time bounds for establishing a connection and for each request.
///
/// `connect` and `request` must be positive; [`ConnectionConfigBuilder::build`] checks
/// it. `request` is the default-deadline source (ADR-0007): a request without a
/// deadline gets `now + request` from the connection's clock. `idle` bounds how long a
/// link may stay open with no request: a connection with a background supervisor (the
/// Modbus TCP connection opened inside a Tokio runtime) closes its transport once no
/// request has run for `idle`, stays available, and opens a new transport for the next
/// request. A zero `idle` closes the transport after every request. A connection without
/// a supervisor (the simulator, or one driven outside a Tokio runtime) ignores it.
///
/// Defaults: 5 s connect, 5 s request, no idle timeout.
///
/// # Compatibility: docs/parity/core.md#CORE-CONFIG-SHAPE
/// The 5 s request default matches upstream `request-timeout-ms`
/// (`docs/parity/modbus.md#MB-CFG-REQUEST-TIMEOUT`). Upstream has no separate connect
/// timeout or idle timeout in the inventoried Modbus files; its connection cache closes a
/// connection nobody has leased for its own `maxIdleTime`, which is what `idle` follows
/// (`docs/parity/transport.md#TRANSPORT-RECONNECT`).
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::Timeouts;
/// let mut timeouts = Timeouts::default();
/// assert_eq!(timeouts.request, Duration::from_secs(5));
/// timeouts.connect = Duration::from_secs(2);
/// assert_eq!(timeouts.idle, None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Timeouts {
    /// Bound for opening the transport and completing any handshake.
    pub connect: Duration,
    /// Default bound for one request when the request carries no deadline.
    pub request: Duration,
    /// How long a link may stay open with no request before a supervised connection
    /// closes it; `None` keeps it open.
    pub idle: Option<Duration>,
}

impl Default for Timeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(5),
            request: Duration::from_secs(5),
            idle: None,
        }
    }
}

/// Capped exponential backoff with symmetric jitter, as pure arithmetic.
///
/// The policy never sleeps and owns no random source: [`next_delay`](Self::next_delay)
/// takes the jitter sample from the caller, so a transport seeds its own generator and
/// tests pass fixed samples (decision D13). Nothing consumes the policy yet.
///
/// Defaults: 250 ms initial delay, 30 s cap, multiplier 2.0, jitter 0.2, unlimited
/// attempts. The fields are public for assignment; a hand-edited value outside the
/// documented range is rejected by [`ConnectionConfigBuilder::build`], and
/// [`next_delay`](Self::next_delay) stays total (never panics) even if it is not.
///
/// # Compatibility: docs/parity/core.md#CORE-RECONNECT-POLICY
/// Upstream has no reconnect backoff in the inventoried Modbus files
/// (`docs/parity/modbus.md#MB-READ-RECONNECT`); this is a Stonecrab extension.
///
/// # Examples
/// ```
/// use std::time::Duration;
/// use stonecrab::ReconnectPolicy;
/// let policy = ReconnectPolicy::default();
/// assert_eq!(policy.next_delay(0, 0.5), Duration::from_millis(250));
/// assert_eq!(policy.next_delay(1, 0.5), Duration::from_millis(500));
/// assert_eq!(policy.next_delay(40, 0.5), Duration::from_secs(30));
/// assert!(!policy.is_exhausted(1_000));
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct ReconnectPolicy {
    /// Delay before the first retry.
    pub initial: Duration,
    /// Upper bound of the exponential growth, before jitter.
    pub max: Duration,
    /// Growth factor per attempt; finite and at least 1.0.
    pub multiplier: f64,
    /// Jitter fraction in `[0.0, 1.0]`: the delay varies by at most this fraction.
    pub jitter: f64,
    /// Retries allowed before giving up, or `None` for unlimited.
    pub max_attempts: Option<NonZeroU32>,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(250),
            max: Duration::from_secs(30),
            multiplier: 2.0,
            jitter: 0.2,
            max_attempts: None,
        }
    }
}

impl ReconnectPolicy {
    /// Delay to wait before retry number `attempt`, where 0 is the first retry.
    ///
    /// The base delay is `initial * multiplier^attempt`, capped at `max`. `sample` is a
    /// caller-supplied uniform value in `[0.0, 1.0]`; the base is scaled by
    /// `1 + jitter * (2 * sample - 1)`, so sample 0.0 gives the lower jitter bound, 0.5
    /// no jitter and 1.0 the upper bound. The result is never negative and never
    /// exceeds `max * (1 + jitter)` (up to nanosecond rounding), so the cap bounds the
    /// delay even at the top of the jitter range.
    ///
    /// The function is total. A `sample` outside `[0.0, 1.0]` is clamped into it and a
    /// NaN sample counts as 0.5 (no jitter). Arithmetic is done in `f64` and saturates:
    /// an enormous `attempt` yields the capped value, and a result too large for a
    /// [`Duration`] is [`Duration::MAX`]. A policy whose public fields were set outside
    /// their documented range (rejected by the builder) is also handled without
    /// panicking: a NaN or out-of-range jitter is clamped into `[0.0, 1.0]` (NaN counts
    /// as 0.0) and a NaN multiplier yields the initial delay at attempt 0 and the cap from
    /// attempt 1 on.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab::ReconnectPolicy;
    /// let policy = ReconnectPolicy::default();
    /// let low = policy.next_delay(2, 0.0);
    /// let high = policy.next_delay(2, 1.0);
    /// assert!(low < Duration::from_secs(1) && Duration::from_secs(1) < high);
    /// assert_eq!(policy.next_delay(u32::MAX, f64::NAN), Duration::from_secs(30));
    /// ```
    #[must_use]
    pub fn next_delay(&self, attempt: u32, sample: f64) -> Duration {
        let sample = unit_interval(sample, 0.5);
        let jitter = unit_interval(self.jitter, 0.0);
        let factor = 1.0 + jitter * (2.0 * sample - 1.0);
        let secs = (self.capped_base_secs(attempt) * factor).max(0.0);
        Duration::try_from_secs_f64(secs).unwrap_or(Duration::MAX)
    }

    /// Whether retry number `attempt` (0 is the first retry) would exceed
    /// `max_attempts`.
    ///
    /// With `max_attempts = Some(3)`, retries 0, 1 and 2 are allowed and retry 3 is
    /// exhausted. `None` is never exhausted.
    ///
    /// # Examples
    /// ```
    /// use std::num::NonZeroU32;
    /// use stonecrab::ReconnectPolicy;
    /// let mut policy = ReconnectPolicy::default();
    /// policy.max_attempts = NonZeroU32::new(3);
    /// assert!(!policy.is_exhausted(2));
    /// assert!(policy.is_exhausted(3));
    /// ```
    #[must_use]
    pub fn is_exhausted(&self, attempt: u32) -> bool {
        self.max_attempts
            .is_some_and(|limit| attempt >= limit.get())
    }

    /// `initial * multiplier^attempt` in seconds, capped at `max`, without overflow.
    fn capped_base_secs(&self, attempt: u32) -> f64 {
        let cap = self.max.as_secs_f64();
        let initial = self.initial.as_secs_f64();
        if initial <= 0.0 {
            return 0.0;
        }
        let exponent = i32::try_from(attempt).unwrap_or(i32::MAX);
        // `powi` saturates to infinity or zero instead of overflowing, and `f64::min`
        // returns the cap when the product is NaN (a NaN multiplier).
        (initial * self.multiplier.powi(exponent)).min(cap).max(0.0)
    }

    fn validate(&self) -> Result<()> {
        if !self.multiplier.is_finite() || self.multiplier < 1.0 {
            return Err(invalid(
                "reconnect multiplier must be finite and at least 1.0",
            ));
        }
        if self.max < self.initial {
            return Err(invalid("reconnect max must not be less than initial"));
        }
        if !(0.0..=1.0).contains(&self.jitter) {
            return Err(invalid("reconnect jitter must be within 0.0 and 1.0"));
        }
        Ok(())
    }
}

/// Clamp `value` into `[0.0, 1.0]`, mapping NaN to `nan`.
const fn unit_interval(value: f64, nan: f64) -> f64 {
    if value.is_nan() {
        nan
    } else {
        value.clamp(0.0, 1.0)
    }
}

/// Attempt budget and error classes for retrying a read.
///
/// `max_attempts` counts the total attempts of one read including the first, so the
/// default of 1 means no retry. This matches the inventoried upstream read path, where
/// no read retry was observed (an inference, not a run observation; see
/// `docs/parity/modbus.md#MB-READ-RETRY`). The policy applies to reads only: writes are
/// never retried automatically, whatever this policy says, because a repeated write
/// can act twice on a device.
///
/// `retry_on` defaults to `{Timeout, Disconnected, Backpressure}`, exactly the kinds
/// [`PlcError::is_retryable`] accepts. Nothing enforces the policy yet.
///
/// # Compatibility: docs/parity/core.md#CORE-RETRY-POLICY
/// Upstream sends a read once; the default policy matches that. Raising `max_attempts`
/// is a Stonecrab extension.
///
/// # Examples
/// ```
/// use std::num::NonZeroU32;
/// use stonecrab::{ErrorKind, RetryPolicy};
/// let mut policy = RetryPolicy::default();
/// assert!(!policy.should_retry(ErrorKind::Timeout, 1), "one attempt: no retry");
/// policy.max_attempts = NonZeroU32::new(3).unwrap_or(NonZeroU32::MIN);
/// assert!(policy.should_retry(ErrorKind::Timeout, 1));
/// assert!(!policy.should_retry(ErrorKind::DeviceException, 1));
/// assert!(!policy.should_retry(ErrorKind::Timeout, 3));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryPolicy {
    /// Total attempts of one read, the first included; 1 disables retry.
    pub max_attempts: NonZeroU32,
    /// Failure kinds that may be retried.
    pub retry_on: BTreeSet<ErrorKind>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: NonZeroU32::MIN,
            retry_on: BTreeSet::from([
                ErrorKind::Timeout,
                ErrorKind::Disconnected,
                ErrorKind::Backpressure,
            ]),
        }
    }
}

impl RetryPolicy {
    /// Whether a read that failed with `kind` after `attempt` attempts may be tried again.
    ///
    /// `attempt` is the number of attempts already made, so it is 1 after the first
    /// failure. The answer is true while `attempt < max_attempts` and `kind` is in
    /// `retry_on`. This is a pure check: it neither sleeps nor distinguishes reads from
    /// writes, and callers must never use it to replay a write.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ErrorKind, RetryPolicy};
    /// assert!(!RetryPolicy::default().should_retry(ErrorKind::Disconnected, 1));
    /// ```
    #[must_use]
    pub fn should_retry(&self, kind: ErrorKind, attempt: u32) -> bool {
        attempt < self.max_attempts.get() && self.retry_on.contains(&kind)
    }
}

/// Whether a connection may modify the device.
///
/// The default is [`AccessPolicy::ReadOnly`]: write access is opt-in (ADR-0008). The
/// enum is exhaustive, like the other two-valued core enums (ADR-0006), so consumers
/// match both variants without a wildcard arm.
///
/// # Examples
/// ```
/// use stonecrab::AccessPolicy;
/// assert_eq!(AccessPolicy::default(), AccessPolicy::ReadOnly);
/// assert_eq!(AccessPolicy::ReadWrite.to_string(), "read-write");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AccessPolicy {
    /// Reads only. Writes are refused before any I/O; the simulator (S11b-2) and the
    /// Modbus driver (S46) enforce it.
    #[default]
    ReadOnly,
    /// Reads and writes are permitted, subject to the driver's own capabilities.
    ReadWrite,
}

impl fmt::Display for AccessPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::ReadOnly => "read-only",
            Self::ReadWrite => "read-write",
        })
    }
}

/// A user name and password for authenticating to a device or gateway.
///
/// The password is a [`Secret`], so formatting never shows it. The user name is shown
/// by `Debug`, but a configuration stores this type inside a [`Secret`] as well, and
/// [`ConnectionConfig::redacted_display`] omits both. Not [`Clone`], because
/// [`Secret`] is not.
///
/// # Examples
/// ```
/// use stonecrab::Credentials;
/// let credentials = Credentials::new("operator", "correct horse");
/// assert_eq!(credentials.username, "operator");
/// assert_eq!(credentials.password.expose(), "correct horse");
/// assert!(!format!("{credentials:?}").contains("correct horse"));
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct Credentials {
    /// Account name.
    pub username: String,
    /// Account password; formatting prints `[REDACTED]`.
    pub password: Secret<String>,
}

impl Credentials {
    /// Wrap a user name and password; the password is moved into a [`Secret`].
    ///
    /// # Examples
    /// ```
    /// use stonecrab::Credentials;
    /// let credentials = Credentials::new("operator", String::from("example-only"));
    /// assert_eq!(credentials.password.expose(), "example-only");
    /// ```
    #[must_use]
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            username: username.into(),
            password: Secret::new(password.into()),
        }
    }
}

/// Driver-owned, validated connection options that have no place in the shared
/// [`ConnectionConfig`] fields.
///
/// A driver whose connection string carries parameters of its own (the Modbus
/// `default-unit-identifier`, say) parses them into a type of its own, implements this
/// trait for it and hands it to [`ConnectionConfigBuilder::driver_options`]. The
/// configuration carries it as an `Arc<dyn DriverOptions>`; the same driver reads it back
/// in `connect` with [`ConnectionConfig::driver_options_as`]. The shape is that of
/// [`ProtocolAddress`](crate::ProtocolAddress): trait upcasting to [`Any`] lets a driver
/// downcast only to its own concrete type, and the trait has no methods because the core
/// has nothing to ask of the options.
///
/// The `Debug` output **must not contain secrets**. [`ConnectionConfig`]'s `Debug` and
/// [`ConnectionConfig::redacted_display`] print it verbatim. Credentials belong in
/// [`Credentials`], which is redacted; never put a password, a token or a key in a driver
/// options type.
///
/// Implementors are immutable after construction (they sit behind an [`Arc`] shared by
/// every holder of the configuration) and `Send + Sync`.
///
/// # Compatibility: docs/parity/core.md#CORE-DRIVER-OPTIONS
/// Upstream keeps a driver-specific configuration object beside the transport
/// configuration (`docs/parity/modbus.md`, rows `MB-CFG-*`); this trait is the
/// driver-independent slot for it. The simulator keeps its own options in the text of an
/// [`Endpoint::Custom`]; a TCP driver has a structured endpoint and so uses this slot
/// (ADR-0008 amendment).
///
/// # Examples
/// ```
/// use std::sync::Arc;
/// use stonecrab::{ConnectionConfig, DriverOptions, Endpoint};
///
/// #[derive(Debug, PartialEq)]
/// struct PollOptions {
///     every_ms: u32,
/// }
/// impl DriverOptions for PollOptions {}
///
/// let config = ConnectionConfig::builder("demo", Endpoint::Custom("bench".to_owned()))
///     .driver_options(Arc::new(PollOptions { every_ms: 250 }))
///     .build()?;
/// assert_eq!(
///     config.driver_options_as::<PollOptions>(),
///     Some(&PollOptions { every_ms: 250 })
/// );
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
pub trait DriverOptions: Any + fmt::Debug + Send + Sync {}

/// A validated connection request: scheme, endpoint and every policy a driver needs.
///
/// Fields are private; read them through the accessors. Construct one with
/// [`ConnectionConfig::builder`], which validates the policies. A configuration holds
/// optional [`Credentials`] inside a [`Secret`], and `Secret` is not cloneable, so
/// `ConnectionConfig` is **not [`Clone`]** (ADR-0008) and is passed by value to
/// `connect`. A caller that still needs the address options afterwards takes them first
/// with [`address_options`](Self::address_options), which returns a [`Clone`] carrier.
///
/// `Debug` shows the structure but never the credentials. For a log line prefer
/// [`redacted_display`](Self::redacted_display).
///
/// # Compatibility: docs/parity/core.md#CORE-CONFIG-SHAPE
/// Upstream parses a connection string into a driver-specific configuration object
/// with per-option defaults (`docs/parity/modbus.md`, rows `MB-CFG-*`). This type is
/// the driver-independent part; unknown connection-string options are an error rather
/// than a warning, which is a recorded divergence (`MB-CFG-UNKNOWN`).
///
/// # Examples
/// ```
/// use stonecrab::{AccessPolicy, ConnectionConfig, Endpoint};
/// let config = ConnectionConfig::builder(
///     "modbus-tcp",
///     Endpoint::Tcp { host: "192.0.2.10".to_owned(), port: 502 },
/// )
/// .address_convention("plc4x")
/// .build()?;
/// assert_eq!(config.scheme(), "modbus-tcp");
/// assert_eq!(config.access(), AccessPolicy::ReadOnly);
/// let options = config.address_options();
/// assert_eq!(options.convention(), Some("plc4x"));
/// assert_eq!(
///     config.redacted_display(),
///     "scheme=modbus-tcp endpoint=192.0.2.10:502 access=read-only credentials=none"
/// );
/// # Ok::<(), stonecrab::PlcError>(())
/// ```
#[derive(Debug)]
#[non_exhaustive]
pub struct ConnectionConfig {
    scheme: String,
    endpoint: Endpoint,
    timeouts: Timeouts,
    reconnect: ReconnectPolicy,
    retry: RetryPolicy,
    access: AccessPolicy,
    credentials: Option<Secret<Credentials>>,
    limits: ResourceLimits,
    address_options: AddressOptions,
    driver_options: Option<Arc<dyn DriverOptions>>,
}

impl ConnectionConfig {
    /// Start a configuration for `scheme` (such as `modbus-tcp`) and `endpoint`, with
    /// default policies; nothing is validated until
    /// [`build`](ConnectionConfigBuilder::build).
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionConfig, Endpoint};
    /// let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
    ///     .build()?;
    /// assert_eq!(config.endpoint(), &Endpoint::Custom("bench".to_owned()));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "configure and validate the connection before use"]
    pub fn builder(scheme: impl Into<String>, endpoint: Endpoint) -> ConnectionConfigBuilder {
        ConnectionConfigBuilder {
            scheme: scheme.into(),
            endpoint,
            timeouts: Timeouts::default(),
            reconnect: ReconnectPolicy::default(),
            retry: RetryPolicy::default(),
            access: AccessPolicy::default(),
            credentials: None,
            limits: None,
            address_convention: None,
            driver_options: None,
        }
    }

    /// The driver scheme, for example `modbus-tcp`.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// Where to connect.
    #[must_use]
    pub const fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// Connect, request and idle bounds.
    #[must_use]
    pub const fn timeouts(&self) -> &Timeouts {
        &self.timeouts
    }

    /// Reconnect backoff policy.
    #[must_use]
    pub const fn reconnect(&self) -> &ReconnectPolicy {
        &self.reconnect
    }

    /// Read retry policy.
    #[must_use]
    pub const fn retry(&self) -> &RetryPolicy {
        &self.retry
    }

    /// Whether writes are permitted.
    #[must_use]
    pub const fn access(&self) -> AccessPolicy {
        self.access
    }

    /// Credentials, if any, still wrapped in a [`Secret`].
    #[must_use]
    pub const fn credentials(&self) -> Option<&Secret<Credentials>> {
        self.credentials.as_ref()
    }

    /// Resource budgets for the connection.
    #[must_use]
    pub const fn limits(&self) -> &ResourceLimits {
        &self.limits
    }

    /// A copy of the address options derived from `address-convention`.
    ///
    /// Call this before handing the configuration to `connect`, which takes it by
    /// value; the returned carrier is [`Clone`] and independent of the configuration.
    #[must_use]
    pub fn address_options(&self) -> AddressOptions {
        self.address_options.clone()
    }

    /// The driver-owned options set with
    /// [`ConnectionConfigBuilder::driver_options`], if any.
    ///
    /// The options are shared (`Arc`), so a caller can keep a handle past `connect`. Prefer
    /// [`driver_options_as`](Self::driver_options_as), which downcasts to the driver's own
    /// type.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionConfig, Endpoint};
    /// let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
    ///     .build()?;
    /// assert!(config.driver_options().is_none());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub const fn driver_options(&self) -> Option<&Arc<dyn DriverOptions>> {
        self.driver_options.as_ref()
    }

    /// The driver-owned options as the concrete type `T`, or `None` when the configuration
    /// carries no options or carries options of another type.
    ///
    /// A driver calls this with its own options type in `connect` and falls back to its
    /// defaults on `None`, so a configuration built by hand still connects.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionConfig, DriverOptions, Endpoint};
    ///
    /// #[derive(Debug)]
    /// struct Mine(u8);
    /// impl DriverOptions for Mine {}
    /// #[derive(Debug)]
    /// struct Other;
    /// impl DriverOptions for Other {}
    ///
    /// let config = ConnectionConfig::builder("demo", Endpoint::Custom("bench".to_owned()))
    ///     .driver_options(Arc::new(Mine(7)))
    ///     .build()?;
    /// assert_eq!(config.driver_options_as::<Mine>().map(|mine| mine.0), Some(7));
    /// assert!(config.driver_options_as::<Other>().is_none());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn driver_options_as<T: DriverOptions>(&self) -> Option<&T> {
        let options: &dyn Any = self.driver_options.as_deref()?;
        options.downcast_ref::<T>()
    }

    /// A single-line description safe for logs and tracing fields.
    ///
    /// It names the scheme, the endpoint, the access policy and whether credentials are
    /// present. It never contains the user name or the password, however they are
    /// spelled. The [`Endpoint::Custom`] text is included verbatim, so it must not
    /// embed credentials. When the configuration carries [`DriverOptions`], their `Debug`
    /// text follows as ` options=...`; the trait requires that text to be free of
    /// secrets.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{ConnectionConfig, Credentials, Endpoint};
    /// let config = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
    ///     .credentials(Credentials::new("operator", "example-only"))
    ///     .build()?;
    /// let text = config.redacted_display();
    /// assert_eq!(text, "scheme=sim endpoint=bench access=read-only credentials=present");
    /// assert!(!text.contains("example-only") && !text.contains("operator"));
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use]
    pub fn redacted_display(&self) -> String {
        let credentials = if self.credentials.is_some() {
            "present"
        } else {
            "none"
        };
        let options = self
            .driver_options
            .as_ref()
            .map_or_else(String::new, |options| format!(" options={options:?}"));
        format!(
            "scheme={} endpoint={} access={} credentials={credentials}{options}",
            self.scheme, self.endpoint, self.access
        )
    }
}

/// Builder for [`ConnectionConfig`].
///
/// Setters record values; [`build`](Self::build) validates them together. Defaults are
/// those of [`Timeouts`], [`ReconnectPolicy`], [`RetryPolicy`], [`AccessPolicy`] and
/// [`ResourceLimits::builder`], no credentials, no address convention and no driver
/// options. Not
/// [`Clone`], because it may hold [`Credentials`].
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub struct ConnectionConfigBuilder {
    scheme: String,
    endpoint: Endpoint,
    timeouts: Timeouts,
    reconnect: ReconnectPolicy,
    retry: RetryPolicy,
    access: AccessPolicy,
    credentials: Option<Credentials>,
    limits: Option<ResourceLimits>,
    address_convention: Option<String>,
    driver_options: Option<Arc<dyn DriverOptions>>,
}

impl ConnectionConfigBuilder {
    /// Replace the timeouts.
    #[must_use = "builder changes must be retained"]
    pub const fn timeouts(mut self, timeouts: Timeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Replace the reconnect policy.
    #[must_use = "builder changes must be retained"]
    pub const fn reconnect(mut self, reconnect: ReconnectPolicy) -> Self {
        self.reconnect = reconnect;
        self
    }

    /// Replace the read retry policy.
    #[must_use = "builder changes must be retained"]
    pub fn retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Choose whether writes are permitted; the default is read-only.
    #[must_use = "builder changes must be retained"]
    pub const fn access(mut self, access: AccessPolicy) -> Self {
        self.access = access;
        self
    }

    /// Supply credentials; they are moved into a [`Secret`] by
    /// [`build`](Self::build).
    #[must_use = "builder changes must be retained"]
    pub fn credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Replace the resource budgets.
    #[must_use = "builder changes must be retained"]
    pub const fn limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Set the `address-convention` carried by the configuration's
    /// [`AddressOptions`].
    ///
    /// The name is validated by [`build`](Self::build) with the same rules as
    /// [`AddressOptionsBuilder::convention`](crate::AddressOptionsBuilder::convention).
    #[must_use = "builder changes must be retained"]
    pub fn address_convention(mut self, name: &str) -> Self {
        self.address_convention = Some(name.to_owned());
        self
    }

    /// Attach driver-owned options; see [`DriverOptions`].
    ///
    /// The driver that built the options reads them back from the configuration with
    /// [`ConnectionConfig::driver_options_as`]. Core neither validates nor interprets
    /// them, and a second call replaces the first.
    ///
    /// # Examples
    /// ```
    /// use std::sync::Arc;
    /// use stonecrab::{ConnectionConfig, DriverOptions, Endpoint};
    ///
    /// #[derive(Debug)]
    /// struct Options;
    /// impl DriverOptions for Options {}
    ///
    /// let config = ConnectionConfig::builder("demo", Endpoint::Custom("bench".to_owned()))
    ///     .driver_options(Arc::new(Options))
    ///     .build()?;
    /// assert!(config.driver_options().is_some());
    /// # Ok::<(), stonecrab::PlcError>(())
    /// ```
    #[must_use = "builder changes must be retained"]
    pub fn driver_options(mut self, options: Arc<dyn DriverOptions>) -> Self {
        self.driver_options = Some(options);
        self
    }

    /// Validate every field together and produce the configuration.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`] if the scheme is empty, the connect or
    /// request timeout is zero, the reconnect multiplier is not finite or is below 1.0,
    /// the reconnect `max` is less than `initial`, the jitter is outside `[0.0, 1.0]`,
    /// or the address convention is invalid. No message contains credential text.
    ///
    /// # Examples
    /// ```
    /// use std::time::Duration;
    /// use stonecrab::{ConnectionConfig, Endpoint, ErrorKind, Timeouts};
    /// let mut timeouts = Timeouts::default();
    /// timeouts.request = Duration::ZERO;
    /// let error = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
    ///     .timeouts(timeouts)
    ///     .build()
    ///     .unwrap_err();
    /// assert_eq!(error.kind(), ErrorKind::InvalidConfig);
    /// ```
    #[must_use = "configuration validation must be checked"]
    pub fn build(self) -> Result<ConnectionConfig> {
        if self.scheme.is_empty() {
            return Err(invalid("scheme must not be empty"));
        }
        if self.timeouts.connect.is_zero() {
            return Err(invalid("connect timeout must be greater than zero"));
        }
        if self.timeouts.request.is_zero() {
            return Err(invalid("request timeout must be greater than zero"));
        }
        self.reconnect.validate()?;
        let address_options = match &self.address_convention {
            Some(name) => AddressOptions::builder().convention(name).build()?,
            None => AddressOptions::default(),
        };
        let limits = match self.limits {
            Some(limits) => limits,
            None => ResourceLimits::builder().build()?,
        };
        Ok(ConnectionConfig {
            scheme: self.scheme,
            endpoint: self.endpoint,
            timeouts: self.timeouts,
            reconnect: self.reconnect,
            retry: self.retry,
            access: self.access,
            credentials: self.credentials.map(Secret::new),
            limits,
            address_options,
            driver_options: self.driver_options,
        })
    }
}

fn invalid(message: &'static str) -> PlcError {
    PlcError::new(ErrorKind::InvalidConfig, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tcp() -> Endpoint {
        Endpoint::Tcp {
            host: "192.0.2.10".to_owned(),
            port: 502,
        }
    }

    fn builder() -> ConnectionConfigBuilder {
        ConnectionConfig::builder("modbus-tcp", tcp())
    }

    fn same(left: f64, right: f64) -> bool {
        left.to_bits() == right.to_bits()
    }

    fn close(actual: Duration, expected_secs: f64) -> bool {
        (actual.as_secs_f64() - expected_secs).abs() < 1e-6
    }

    #[test]
    fn defaults_match_the_documented_values() {
        let config = builder().build().expect("defaults are valid");
        assert_eq!(config.scheme(), "modbus-tcp", "scheme kept");
        assert_eq!(config.endpoint(), &tcp(), "endpoint kept");
        assert_eq!(
            config.timeouts(),
            &Timeouts {
                connect: Duration::from_secs(5),
                request: Duration::from_secs(5),
                idle: None,
            },
            "timeout defaults"
        );
        let reconnect = config.reconnect();
        assert_eq!(reconnect.initial, Duration::from_millis(250), "initial");
        assert_eq!(reconnect.max, Duration::from_secs(30), "max");
        assert!(same(reconnect.multiplier, 2.0), "multiplier");
        assert!(same(reconnect.jitter, 0.2), "jitter");
        assert_eq!(reconnect.max_attempts, None, "unlimited");
        assert_eq!(config.retry(), &RetryPolicy::default(), "retry default");
        assert_eq!(config.retry().max_attempts.get(), 1, "no read retry");
        assert_eq!(config.access(), AccessPolicy::ReadOnly, "read-only default");
        assert!(config.credentials().is_none(), "no credentials");
        let limits = ResourceLimits::builder().build().expect("default limits");
        assert_eq!(config.limits(), &limits, "default limits");
        assert_eq!(
            config.address_options(),
            AddressOptions::default(),
            "no convention"
        );
        assert_eq!(
            AccessPolicy::default(),
            AccessPolicy::ReadOnly,
            "enum default"
        );
    }

    #[test]
    fn every_setter_is_retained_by_its_accessor() {
        let timeouts = Timeouts {
            connect: Duration::from_secs(1),
            request: Duration::from_secs(2),
            idle: Some(Duration::from_secs(60)),
        };
        let reconnect = ReconnectPolicy {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(10),
            multiplier: 1.5,
            jitter: 0.0,
            max_attempts: NonZeroU32::new(7),
        };
        let mut retry = RetryPolicy {
            max_attempts: NonZeroU32::new(3).expect("non-zero"),
            ..RetryPolicy::default()
        };
        retry.retry_on.insert(ErrorKind::DeviceException);
        let limits = ResourceLimits::builder()
            .max_frame_bytes(260)
            .max_buffered_bytes(520)
            .build()
            .expect("valid limits");
        let config = builder()
            .timeouts(timeouts)
            .reconnect(reconnect)
            .retry(retry.clone())
            .access(AccessPolicy::ReadWrite)
            .credentials(Credentials::new("operator7", "example-only"))
            .limits(limits)
            .address_convention("plc4x")
            .build()
            .expect("valid configuration");
        assert_eq!(config.timeouts(), &timeouts, "timeouts");
        assert_eq!(config.reconnect(), &reconnect, "reconnect");
        assert_eq!(config.retry(), &retry, "retry");
        assert_eq!(config.access(), AccessPolicy::ReadWrite, "access");
        assert_eq!(config.limits(), &limits, "limits");
        let credentials = config.credentials().expect("credentials kept").expose();
        assert_eq!(credentials.username, "operator7", "username");
        assert_eq!(credentials.password.expose(), "example-only", "password");
        assert_eq!(
            config.address_options().convention(),
            Some("plc4x"),
            "convention"
        );
    }

    #[test]
    fn address_options_round_trip_and_outlive_the_config() {
        let config = builder()
            .address_convention("plc4x")
            .build()
            .expect("valid convention");
        let options = config.address_options();
        drop(config);
        assert_eq!(
            options.convention(),
            Some("plc4x"),
            "carrier survives connect"
        );
        assert_eq!(options.clone(), options, "carrier is Clone");
    }

    #[test]
    fn build_rejects_each_invalid_field_with_invalid_config() {
        fn with_reconnect(change: impl FnOnce(&mut ReconnectPolicy)) -> ConnectionConfigBuilder {
            let mut policy = ReconnectPolicy::default();
            change(&mut policy);
            builder().reconnect(policy)
        }
        fn with_timeouts(change: impl FnOnce(&mut Timeouts)) -> ConnectionConfigBuilder {
            let mut timeouts = Timeouts::default();
            change(&mut timeouts);
            builder().timeouts(timeouts)
        }
        let cases = [
            ("scheme", ConnectionConfig::builder("", tcp())),
            (
                "connect timeout",
                with_timeouts(|t| t.connect = Duration::ZERO),
            ),
            (
                "request timeout",
                with_timeouts(|t| t.request = Duration::ZERO),
            ),
            ("multiplier", with_reconnect(|r| r.multiplier = f64::NAN)),
            (
                "multiplier",
                with_reconnect(|r| r.multiplier = f64::INFINITY),
            ),
            (
                "multiplier",
                with_reconnect(|r| r.multiplier = f64::NEG_INFINITY),
            ),
            ("multiplier", with_reconnect(|r| r.multiplier = 0.99)),
            ("multiplier", with_reconnect(|r| r.multiplier = -2.0)),
            (
                "max",
                with_reconnect(|r| {
                    r.initial = Duration::from_secs(2);
                    r.max = Duration::from_secs(1);
                }),
            ),
            ("jitter", with_reconnect(|r| r.jitter = -0.1)),
            ("jitter", with_reconnect(|r| r.jitter = 1.1)),
            ("jitter", with_reconnect(|r| r.jitter = f64::NAN)),
            ("jitter", with_reconnect(|r| r.jitter = f64::INFINITY)),
            ("address-convention", builder().address_convention("")),
            ("address-convention", builder().address_convention("a b")),
        ];
        for (field, candidate) in cases {
            let error = candidate.build().expect_err("invalid configuration");
            assert_eq!(error.kind(), ErrorKind::InvalidConfig, "typed: {field}");
            assert!(error.to_string().contains(field), "names {field}");
        }
    }

    #[test]
    fn boundary_values_are_accepted() {
        let mut reconnect = ReconnectPolicy {
            multiplier: 1.0,
            jitter: 1.0,
            initial: Duration::from_secs(30),
            max: Duration::from_secs(30),
            ..ReconnectPolicy::default()
        };
        builder()
            .reconnect(reconnect)
            .build()
            .expect("edges are valid");
        reconnect.jitter = 0.0;
        reconnect.initial = Duration::ZERO;
        builder()
            .reconnect(reconnect)
            .build()
            .expect("zero initial and jitter");
        let timeouts = Timeouts {
            connect: Duration::from_nanos(1),
            request: Duration::from_nanos(1),
            idle: Some(Duration::ZERO),
        };
        builder()
            .timeouts(timeouts)
            .build()
            .expect("positive minimums");
        ConnectionConfig::builder("x", Endpoint::Custom(String::new()))
            .build()
            .expect("one-byte scheme and empty custom endpoint are not core errors");
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn backoff_doubles_from_the_initial_delay_and_caps_at_max() {
        let policy = ReconnectPolicy::default();
        let expected_ms = [250_u64, 500, 1_000, 2_000, 4_000, 8_000, 16_000, 30_000];
        for (attempt, millis) in (0_u32..).zip(expected_ms) {
            assert_eq!(
                policy.next_delay(attempt, 0.5),
                Duration::from_millis(millis),
                "attempt {attempt}"
            );
        }
        for attempt in [8, 9, 63, 64, 1_000, u32::MAX] {
            assert_eq!(
                policy.next_delay(attempt, 0.5),
                Duration::from_secs(30),
                "capped at attempt {attempt}"
            );
        }
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn jitter_samples_reach_both_bounds_and_the_cap_bounds_the_upper_one() {
        let policy = ReconnectPolicy::default();
        assert!(
            close(policy.next_delay(0, 0.0), 0.2),
            "250 ms less 20 percent"
        );
        assert!(
            close(policy.next_delay(0, 1.0), 0.3),
            "250 ms plus 20 percent"
        );
        assert!(
            close(policy.next_delay(1, 0.0), 0.4),
            "500 ms less 20 percent"
        );
        assert!(
            close(policy.next_delay(20, 0.0), 24.0),
            "cap less 20 percent"
        );
        assert!(
            close(policy.next_delay(20, 1.0), 36.0),
            "cap plus 20 percent"
        );
        assert!(
            policy.next_delay(3, 0.25) < policy.next_delay(3, 0.75),
            "jitter is monotonic in the sample"
        );
        let mut none = policy;
        none.jitter = 0.0;
        assert_eq!(
            none.next_delay(0, 0.0),
            none.next_delay(0, 1.0),
            "zero jitter"
        );
        let mut full = policy;
        full.jitter = 1.0;
        assert_eq!(
            full.next_delay(0, 0.0),
            Duration::ZERO,
            "full jitter reaches zero"
        );
        assert_eq!(
            full.next_delay(0, 1.0),
            Duration::from_millis(500),
            "and doubles"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn next_delay_is_total_and_bounded_for_every_attempt_and_sample() {
        let policy = ReconnectPolicy::default();
        let ceiling = policy.max.as_secs_f64() * (1.0 + policy.jitter) + 1e-6;
        let attempts = [0, 1, 2, 7, 8, 31, 32, 63, 64, 1_024, 65_535, u32::MAX];
        let samples = [
            f64::NEG_INFINITY,
            -1.0,
            0.0,
            0.25,
            0.5,
            1.0,
            2.0,
            f64::INFINITY,
            f64::NAN,
        ];
        for attempt in attempts {
            for sample in samples {
                let secs = policy.next_delay(attempt, sample).as_secs_f64();
                assert!(secs >= 0.0, "non-negative at {attempt}/{sample}");
                assert!(secs <= ceiling, "bounded at {attempt}/{sample}");
            }
        }
        assert_eq!(
            policy.next_delay(0, f64::NAN),
            policy.next_delay(0, 0.5),
            "NaN is neutral"
        );
        assert_eq!(
            policy.next_delay(0, -5.0),
            policy.next_delay(0, 0.0),
            "low clamp"
        );
        assert_eq!(
            policy.next_delay(0, 5.0),
            policy.next_delay(0, 1.0),
            "high clamp"
        );
        assert_eq!(
            policy.next_delay(0, f64::NEG_INFINITY),
            policy.next_delay(0, 0.0),
            "-inf"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn next_delay_survives_policies_the_builder_would_reject() {
        let base = ReconnectPolicy::default();
        let mut nan_multiplier = base;
        nan_multiplier.multiplier = f64::NAN;
        assert_eq!(
            nan_multiplier.next_delay(3, 0.5),
            base.max,
            "NaN growth gives the cap"
        );
        assert_eq!(
            nan_multiplier.next_delay(0, 0.5),
            base.initial,
            "NaN growth leaves the initial delay at attempt 0"
        );
        let mut shrinking = base;
        shrinking.multiplier = 0.5;
        assert_eq!(
            shrinking.next_delay(2, 0.5),
            Duration::from_micros(62_500),
            "shrinks"
        );
        let mut negative = base;
        negative.multiplier = -2.0;
        assert_eq!(
            negative.next_delay(1, 0.5),
            Duration::ZERO,
            "negative base floors at zero"
        );
        let mut bad_jitter = base;
        bad_jitter.jitter = f64::NAN;
        assert_eq!(
            bad_jitter.next_delay(0, 1.0),
            Duration::from_millis(250),
            "NaN jitter is none"
        );
        bad_jitter.jitter = 7.0;
        assert_eq!(
            bad_jitter.next_delay(0, 1.0),
            Duration::from_millis(500),
            "jitter clamps to 1"
        );
        let mut inverted = base;
        inverted.initial = Duration::from_secs(60);
        assert_eq!(inverted.next_delay(0, 0.5), base.max, "max still caps");
        let mut flat = base;
        flat.multiplier = 1.0;
        assert_eq!(
            flat.next_delay(u32::MAX, 0.5),
            base.initial,
            "multiplier 1 never grows"
        );
        let mut zero = base;
        zero.initial = Duration::ZERO;
        assert_eq!(
            zero.next_delay(u32::MAX, 1.0),
            Duration::ZERO,
            "zero initial stays zero"
        );
        let mut huge = base;
        huge.max = Duration::MAX;
        assert_eq!(
            huge.next_delay(u32::MAX, 0.5),
            Duration::MAX,
            "overflow saturates"
        );
        assert_eq!(
            huge.next_delay(u32::MAX, 1.0),
            Duration::MAX,
            "also with jitter"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn reconnect_attempts_are_exhausted_only_at_the_limit() {
        let mut policy = ReconnectPolicy::default();
        for attempt in [0, 1, u32::MAX] {
            assert!(!policy.is_exhausted(attempt), "unlimited at {attempt}");
        }
        policy.max_attempts = NonZeroU32::new(3);
        assert!(!policy.is_exhausted(0), "first retry allowed");
        assert!(!policy.is_exhausted(2), "third retry allowed");
        assert!(policy.is_exhausted(3), "fourth retry refused");
        assert!(policy.is_exhausted(u32::MAX), "far past the limit");
        policy.max_attempts = Some(NonZeroU32::MIN);
        assert!(!policy.is_exhausted(0), "limit one allows one retry");
        assert!(policy.is_exhausted(1), "and no more");
    }

    #[test]
    fn default_retry_set_equals_the_error_retryable_predicate() {
        let policy = RetryPolicy::default();
        let all = [
            ErrorKind::InvalidConfig,
            ErrorKind::InvalidAddress,
            ErrorKind::Unsupported(crate::Operation::Read),
            ErrorKind::Unsupported(crate::Operation::Write),
            ErrorKind::Timeout,
            ErrorKind::Cancelled,
            ErrorKind::Disconnected,
            ErrorKind::Authentication,
            ErrorKind::Trust,
            ErrorKind::MalformedData,
            ErrorKind::DeviceException,
            ErrorKind::ResourceLimit,
            ErrorKind::Backpressure,
            ErrorKind::Internal,
        ];
        for kind in all {
            assert_eq!(
                policy.retry_on.contains(&kind),
                PlcError::new(kind, "probe").is_retryable(),
                "{kind:?} agrees with is_retryable"
            );
        }
        assert_eq!(
            policy.retry_on.len(),
            3,
            "exactly the three transient kinds"
        );
    }

    // covers: SPEC mandatory-failure-cases / F5 exhausted retry budget (policy arithmetic)
    #[test]
    fn retry_stops_at_max_attempts_and_only_for_listed_kinds() {
        let mut policy = RetryPolicy::default();
        for kind in [
            ErrorKind::Timeout,
            ErrorKind::Disconnected,
            ErrorKind::Backpressure,
        ] {
            assert!(
                !policy.should_retry(kind, 1),
                "default sends once: {kind:?}"
            );
        }
        policy.max_attempts = NonZeroU32::new(3).expect("non-zero");
        for kind in [
            ErrorKind::Timeout,
            ErrorKind::Disconnected,
            ErrorKind::Backpressure,
        ] {
            assert!(
                policy.should_retry(kind, 1),
                "after the first failure: {kind:?}"
            );
            assert!(
                policy.should_retry(kind, 2),
                "after the second failure: {kind:?}"
            );
            assert!(!policy.should_retry(kind, 3), "budget spent: {kind:?}");
            assert!(!policy.should_retry(kind, u32::MAX), "far past: {kind:?}");
        }
        assert!(
            !policy.should_retry(ErrorKind::DeviceException, 1),
            "not listed"
        );
        assert!(
            !policy.should_retry(ErrorKind::Cancelled, 1),
            "never retry cancel"
        );
        policy.retry_on.insert(ErrorKind::DeviceException);
        assert!(
            policy.should_retry(ErrorKind::DeviceException, 1),
            "caller opted in"
        );
        policy.retry_on.clear();
        assert!(
            !policy.should_retry(ErrorKind::Timeout, 1),
            "empty set retries nothing"
        );
    }

    #[test]
    fn endpoint_display_brackets_ipv6_and_keeps_custom_text() {
        let cases = [
            (tcp(), "192.0.2.10:502"),
            (
                Endpoint::Tcp {
                    host: "plc.example".to_owned(),
                    port: 0,
                },
                "plc.example:0",
            ),
            (
                Endpoint::Tcp {
                    host: "2001:db8::1".to_owned(),
                    port: 502,
                },
                "[2001:db8::1]:502",
            ),
            (Endpoint::Custom("device/7".to_owned()), "device/7"),
        ];
        for (endpoint, text) in cases {
            assert_eq!(endpoint.to_string(), text, "display of {endpoint:?}");
        }
    }

    #[test]
    fn access_policy_displays_stable_names() {
        assert_eq!(AccessPolicy::ReadOnly.to_string(), "read-only", "read-only");
        assert_eq!(
            AccessPolicy::ReadWrite.to_string(),
            "read-write",
            "read-write"
        );
        assert_ne!(AccessPolicy::ReadOnly, AccessPolicy::ReadWrite, "distinct");
    }

    #[test]
    fn credentials_debug_never_prints_the_password() {
        for password in [
            "hunter2",
            "p@ss=word&x",
            "{\"password\":1}",
            "REDACTED-not",
            "",
        ] {
            let credentials = Credentials::new("operator7", password);
            let debug = format!("{credentials:?}");
            let pretty = format!("{credentials:#?}");
            for text in [&debug, &pretty] {
                assert!(text.contains("[REDACTED]"), "marks the secret");
                assert!(text.contains("operator7"), "username is diagnostic");
                assert!(
                    password.is_empty() || !text.contains(password),
                    "no password text for {password:?}"
                );
            }
            assert_eq!(credentials.password.expose(), password, "value intact");
        }
        let owned = Credentials::new(String::from("a"), String::from("b"));
        assert_eq!(owned.username, "a", "owned strings accepted");
    }

    #[test]
    fn config_debug_hides_the_whole_credentials_value() {
        let config = builder()
            .credentials(Credentials::new("operator7", "hunter2"))
            .build()
            .expect("valid");
        let debug = format!("{config:?}");
        assert!(debug.contains("[REDACTED]"), "credentials redacted");
        for hidden in ["operator7", "hunter2"] {
            assert!(!debug.contains(hidden), "{hidden} not printed");
        }
        let pending = format!(
            "{:?}",
            builder().credentials(Credentials::new("operator7", "hunter2"))
        );
        assert!(
            !pending.contains("hunter2"),
            "builder debug hides the password"
        );
    }

    #[test]
    fn redacted_display_never_contains_credential_text() {
        // Passwords and user names that imitate other fields or field syntax. None is a
        // substring of the fixed text the display emits, so absence proves redaction.
        let passwords = [
            "hunter2",
            "scheme=modbus-udp",
            "endpoint=198.51.100.9:502",
            "198.51.100.9:502",
            "modbus-tcp://192.0.2.10:502/x",
            "access=read-write!",
            "credentials=present!",
            "user:pass@host",
            "?password=hunter2&x=1",
            "p@ss word",
            "\u{1f511}secret",
            "xyzzy",
        ];
        let usernames = [
            "operator7",
            "admin@192.0.2.10",
            "user=root",
            "scheme=x",
            "\u{e9}l\u{e8}ve",
        ];
        for password in passwords {
            for username in usernames {
                let config = builder()
                    .credentials(Credentials::new(username, password))
                    .access(AccessPolicy::ReadWrite)
                    .build()
                    .expect("valid");
                let text = config.redacted_display();
                assert!(
                    text.ends_with("credentials=present"),
                    "marks presence: {text}"
                );
                assert!(!text.contains(password), "password leaked: {text}");
                assert!(!text.contains(username), "username leaked: {text}");
            }
        }
    }

    #[test]
    fn redacted_display_has_a_fixed_shape_without_credentials() {
        let config = builder().build().expect("valid");
        assert_eq!(
            config.redacted_display(),
            "scheme=modbus-tcp endpoint=192.0.2.10:502 access=read-only credentials=none",
            "no credentials"
        );
        let custom = ConnectionConfig::builder("sim", Endpoint::Custom("bench".to_owned()))
            .access(AccessPolicy::ReadWrite)
            .credentials(Credentials::new("operator7", "hunter2"))
            .build()
            .expect("valid");
        assert_eq!(
            custom.redacted_display(),
            "scheme=sim endpoint=bench access=read-write credentials=present",
            "credentials marked present only"
        );
    }

    #[derive(Debug, PartialEq)]
    struct Options {
        every_ms: u32,
    }
    impl DriverOptions for Options {}

    #[derive(Debug)]
    struct Unrelated;
    impl DriverOptions for Unrelated {}

    #[test]
    fn driver_options_are_absent_by_default() {
        let config = builder().build().expect("valid");
        assert!(config.driver_options().is_none(), "no slot content");
        assert!(
            config.driver_options_as::<Options>().is_none(),
            "no downcast"
        );
        assert!(
            format!("{config:?}").contains("driver_options: None"),
            "debug shows the empty slot"
        );
        assert!(
            !config.redacted_display().contains("options="),
            "display names none"
        );
    }

    #[test]
    fn driver_options_round_trip_through_the_builder_and_downcast_to_their_own_type() {
        let config = builder()
            .driver_options(Arc::new(Options { every_ms: 250 }))
            .build()
            .expect("valid");
        assert!(config.driver_options().is_some(), "slot filled");
        assert_eq!(
            config.driver_options_as::<Options>(),
            Some(&Options { every_ms: 250 }),
            "own type"
        );
        assert!(
            config.driver_options_as::<Unrelated>().is_none(),
            "another type does not downcast"
        );
        let replaced = builder()
            .driver_options(Arc::new(Unrelated))
            .driver_options(Arc::new(Options { every_ms: 5 }))
            .build()
            .expect("valid");
        assert_eq!(
            replaced.driver_options_as::<Options>(),
            Some(&Options { every_ms: 5 }),
            "the second call replaces the first"
        );
    }

    #[test]
    fn driver_options_appear_in_debug_and_redacted_display_without_credentials() {
        let config = builder()
            .driver_options(Arc::new(Options { every_ms: 250 }))
            .credentials(Credentials::new("operator7", "hunter2"))
            .build()
            .expect("valid");
        let debug = format!("{config:?}");
        assert!(
            debug.contains("Options { every_ms: 250 }"),
            "debug: {debug}"
        );
        assert!(!debug.contains("hunter2"), "credentials stay hidden");
        let text = config.redacted_display();
        assert_eq!(
            text,
            "scheme=modbus-tcp endpoint=192.0.2.10:502 access=read-only credentials=present \
             options=Options { every_ms: 250 }"
        );
        assert!(!text.contains("hunter2") && !text.contains("operator7"));
        let pending = format!(
            "{:?}",
            builder().driver_options(Arc::new(Options { every_ms: 1 }))
        );
        assert!(pending.contains("every_ms: 1"), "builder debug: {pending}");
    }
}
