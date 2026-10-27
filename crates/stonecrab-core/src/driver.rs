//! The driver boundary: a stateless factory per connection-string scheme.

use std::sync::Arc;

use crate::{
    AddressOptions, Capabilities, Connection, ConnectionConfig, OpFuture, ProtocolAddress, Result,
};

/// A protocol driver: the stateless factory for one connection-string scheme.
///
/// A driver knows how to read its scheme's connection strings, how to parse its
/// protocol's address text, what it can do, and how to open a [`Connection`]. It holds
/// no per-connection state: everything that belongs to one live link (the transport,
/// the lifecycle state, the in-flight request) lives behind the [`Connection`] it
/// returns, so one driver value serves any number of connections and can be shared
/// freely (`Send + Sync`, usually inside an `Arc` in a
/// [`DriverRegistry`](crate::DriverRegistry)). The trait is object safe: applications
/// hold `Arc<dyn Driver>`.
///
/// Addresses are returned as `Arc<dyn ProtocolAddress>` rather than a `Box` because
/// [`Tag::new`](crate::Tag::new) takes an `Arc`, and a profile that reuses one address
/// for several tags shares it instead of re-parsing.
///
/// # Address options
/// [`parse_address`](Self::parse_address) takes an [`AddressOptions`] beside the text.
/// That is how a caller selects an address convention without the driver keeping any
/// state (decision D16, ADR-0008): a caller takes the options from
/// [`ConnectionConfig::address_options`] before `connect` consumes the configuration, or
/// builds them from a sensor profile. A driver that has no conventions ignores the
/// argument and documents that it does.
///
/// # What is not here yet
/// Subscriptions, browsing and discovery are out of the alpha (decision D4). They are
/// added later as provided methods that return
/// [`ErrorKind::Unsupported`](crate::ErrorKind::Unsupported), which is additive for
/// implementors; no empty request or response types ship before then (ADR-0001).
///
/// # Compatibility: docs/parity/core.md#CORE-DRIVER-TRAIT
/// Corresponds to the upstream `PlcDriver`. Upstream's driver also creates connections
/// from a connection string and exposes `canRead`-style capability flags; here the flags
/// are the explicit [`Capabilities`] value. Differences from the ARCHITECTURE.md 3.1
/// diagram are listed in ADR-0011.
///
/// # Examples
/// Code that is written against the trait works with any driver. This function parses
/// one tag's address with whatever convention the connection configuration carries.
/// ```
/// use std::sync::Arc;
/// use stonecrab_core::{
///     ConnectionConfig, Driver, ProtocolAddress, Result, Tag, TagId,
/// };
///
/// fn tag_for(driver: &dyn Driver, config: &ConnectionConfig, text: &str) -> Result<Tag> {
///     let address: Arc<dyn ProtocolAddress> =
///         driver.parse_address(text, &config.address_options())?;
///     Ok(Tag::new(TagId::new(text)?, address))
/// }
/// ```
pub trait Driver: Send + Sync {
    /// The connection-string scheme this driver serves, such as `modbus-tcp` or `sim`.
    ///
    /// It should use only the characters `a-z`, `0-9` and `-`, because the registry
    /// finds a driver by the scheme that [`ConnectionUri::parse`](crate::ConnectionUri::parse)
    /// extracts, and that grammar admits nothing else. The value is constant for the
    /// life of the driver.
    fn scheme(&self) -> &'static str;

    /// What this driver can do, before any connection exists.
    ///
    /// The value is returned by value: [`Capabilities`] is `Copy`. A connection reports
    /// its own, possibly narrower, set through [`Connection::capabilities`] once the
    /// device is known.
    fn capabilities(&self) -> Capabilities;

    /// Read a connection string of this driver's scheme into a validated configuration.
    ///
    /// `uri` is the complete original text. The registry has already checked that its
    /// scheme is this driver's, using [`ConnectionUri::parse`](crate::ConnectionUri::parse);
    /// the driver parses it again with the same function and interprets the authority and
    /// the options, so no driver splits the authority or the query by hand. Unknown
    /// options are an error rather than a warning (decision D12). No connection is
    /// opened and nothing is resolved.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) for text
    /// that is not a valid connection string, for a scheme that is not this driver's, for
    /// an unknown option or a malformed option value, and for anything the configuration
    /// builder rejects.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-DRIVER-TRAIT
    /// Upstream drivers parse the same grammar inside `PlcDriver.getConnection`.
    fn parse_config(&self, uri: &str) -> Result<ConnectionConfig>;

    /// Parse protocol address text into a shared, validated address.
    ///
    /// `options` carries the caller's address convention (decision D16). A driver with
    /// no conventions ignores it and says so in its own documentation.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidAddress`](crate::ErrorKind::InvalidAddress) for text
    /// the driver's grammar rejects, including an ambiguous spelling when `options` does
    /// not select a convention that resolves the ambiguity.
    ///
    /// # Compatibility: docs/parity/core.md#CORE-DRIVER-TRAIT
    /// Upstream parses address text with the driver's field handler; the convention
    /// argument is a Stonecrab addition (`CORE-ADDRESS-OPTIONS`).
    fn parse_address(
        &self,
        text: &str,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>>;

    /// Open a connection described by `config`, consuming it.
    ///
    /// The configuration is passed by value because it may hold credentials, which are
    /// not cloneable ([`ConnectionConfig`] is not `Clone`); take
    /// [`ConnectionConfig::address_options`] first if the options are needed afterwards.
    /// On success the connection is in the [`Connected`](crate::ConnectionState::Connected)
    /// state.
    ///
    /// The whole attempt is bounded by `config.timeouts().connect`: the driver derives
    /// `Deadline::after(clock.now(), timeouts.connect)` from its own clock and gives up
    /// with [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) when it passes. There is no
    /// connect without a deadline.
    ///
    /// # Errors
    /// Returns [`ErrorKind::Timeout`](crate::ErrorKind::Timeout) when the connect
    /// deadline passes, [`ErrorKind::Disconnected`](crate::ErrorKind::Disconnected) when
    /// the peer refuses or drops the link, [`ErrorKind::Authentication`](crate::ErrorKind::Authentication)
    /// or [`ErrorKind::Trust`](crate::ErrorKind::Trust) when a handshake fails, and
    /// [`ErrorKind::InvalidConfig`](crate::ErrorKind::InvalidConfig) when the
    /// configuration is not one this driver can honour (for example another scheme's
    /// endpoint).
    ///
    /// # Cancel safety
    /// Cancel-safe. Dropping the future abandons the attempt: any half-open transport is
    /// closed by its own drop and no connection object exists afterwards. The deadline
    /// source is `config.timeouts().connect`.
    ///
    /// # Wire effects
    /// May open a transport and run a protocol handshake, all of it inside the
    /// `timeouts.connect` bound. Cancelling or timing out mid-handshake leaves the device
    /// with at most a half-open session that it will time out itself; no application data
    /// has been sent.
    fn connect(&self, config: ConnectionConfig) -> OpFuture<'_, Box<dyn Connection>>;
}
