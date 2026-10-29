//! The entity manager: [`PlcEntity`] values read and written through the connection cache
//! (ADR-0033).
//!
//! Every request takes a lease from the [`ConnectionCache`], sends one read or one write
//! with the manager's request timeout and gives the lease back. Nothing is spawned and
//! nothing runs between calls: a [`ConnectedEntity`] holds no lease, and its cache
//! durations are measured on the injected [`Clock`] when it is refreshed.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

use crate::{
    Clock, Connection, ConnectionCache, Deadline, DriverRegistry, EntityField, ErrorKind, OpFuture,
    PlcEntity, PlcError, PlcValue, ReadRequest, ReadRequestBuilder, ReadResponse, Result, Tag,
    TagId, WriteRequest, WriteResponse,
};

/// Upstream's default `org.apache.plc4x.java.opm.entity_manager.read_timeout`: one second.
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(1);

/// What a request of the manager or of a connected entity goes through.
#[derive(Clone, Copy)]
struct Link<'a> {
    cache: &'a ConnectionCache,
    clock: &'a dyn Clock,
    request_timeout: Duration,
}

impl Link<'_> {
    /// The deadline of a request sent now.
    fn deadline(self) -> Result<Deadline> {
        Deadline::after(self.clock.now(), self.request_timeout)
    }

    /// Read `tags` in one request on a lease of `connection_string`, then give it back.
    async fn read(self, connection_string: &str, tags: Vec<Tag>) -> Result<ReadResponse> {
        let lease = self.cache.get(connection_string).await?;
        let request = tags
            .into_iter()
            .fold(
                ReadRequest::builder(lease.limits()),
                ReadRequestBuilder::tag,
            )
            .deadline(self.deadline()?)
            .build()?;
        lease.read(request).await
    }

    /// Write `items` in one request on a lease of `connection_string`, then give it back.
    async fn write(
        self,
        connection_string: &str,
        items: Vec<(Tag, PlcValue)>,
    ) -> Result<WriteResponse> {
        let lease = self.cache.get(connection_string).await?;
        let request = items
            .into_iter()
            .fold(
                WriteRequest::builder(lease.limits()),
                |builder, (tag, value)| builder.item(tag, value),
            )
            .deadline(self.deadline()?)
            .build()?;
        lease.write(request).await
    }
}

/// The error of the field named `field`, from `error`: its kind and diagnostics, and its
/// message after the field's name.
fn in_field(field: &str, error: &PlcError) -> PlcError {
    PlcError::new(error.kind(), format!("field {field:?}: {error}"))
        .with_diagnostics(error.diagnostics().clone())
}

/// Each tag paired with the value of its field in `entity`, refused when there is no tag:
/// a write needs at least one item, and the refusal comes before a lease is taken.
fn items(entity: &dyn PlcEntity, tags: Vec<Tag>) -> Result<Vec<(Tag, PlcValue)>> {
    if tags.is_empty() {
        return Err(PlcError::new(
            ErrorKind::InvalidConfig,
            "the entity has no mapped field to write",
        ));
    }
    tags.into_iter()
        .map(|tag| {
            let field = tag.id().as_str();
            let value = entity.field_value(field).ok_or_else(|| {
                PlcError::new(
                    ErrorKind::InvalidConfig,
                    format!("field {field:?}: the entity lists it but has no value for it"),
                )
            })?;
            Ok((tag, value))
        })
        .collect()
}

/// Reads and writes [`PlcEntity`] values through a [`ConnectionCache`].
///
/// Each field's address is parsed by the driver of the connection string, with the
/// connection string's address options; an address of the form `${name}` is first
/// replaced by the address registered for the alias `name` with
/// [`with_alias`](Self::with_alias). Every request takes a lease from the cache, sends one
/// read or one write with a deadline [`with_request_timeout`](Self::with_request_timeout)
/// after it is sent (one second by default), and gives the lease back, so the connection
/// is shared with the cache's other users between requests.
///
/// - [`read`](Self::read) fills a new entity from one read of all its mapped fields.
/// - [`write`](Self::write) writes all its mapped fields in one write.
/// - [`connect`](Self::connect) is `read` that keeps the entity attached as a
///   [`ConnectedEntity`], whose [`refresh`](ConnectedEntity::refresh) reads again only the
///   fields whose cache duration has passed.
/// - [`merge`](Self::merge) is `write` that keeps the entity attached.
/// - [`ConnectedEntity::disconnect`] gives the plain entity back.
///
/// The clock must be the one the cache and its connections run on.
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-MANAGER
/// Upstream's `PlcEntityManager` over a `PlcConnectionFactory`, here the connection cache.
/// Upstream returns a runtime proxy whose getters read; here a [`ConnectedEntity`] reads
/// when it is refreshed. See `docs/adr/0033-object-plc-mapping.md` for the differences.
///
/// # Examples
/// ```
/// # #[cfg(all(feature = "sim", feature = "testkit"))]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use stonecrab::sim::{SimDevice, SimDriver, SimTag};
/// use stonecrab::testkit::{FakeClock, block_on_with};
/// use stonecrab::{ConnectionCache, DriverRegistry, EntityManager, PlcValue};
///
/// stonecrab::plc_entity! {
///     #[derive(Debug, Default, PartialEq)]
///     struct Tank {
///         level: u16 => "STATE/level:u16",
///         setpoint: i16 => "${setpoint}",
///     }
/// }
///
/// let clock = FakeClock::default();
/// let device = SimDevice::new();
/// device.set("level", SimTag::new(PlcValue::U16(840)))?;
/// device.set("setpoint", SimTag::new(PlcValue::I16(900)))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new().with_clock(Arc::new(clock.clone())).with_device("bench", device)?,
/// ))?;
/// let registry = Arc::new(registry);
/// let cache = ConnectionCache::builder(Arc::clone(&registry), Arc::new(clock.clone())).build()?;
/// let manager = EntityManager::new(&cache, &registry, &clock)
///     .with_alias("setpoint", "STATE/setpoint:i16");
///
/// let tank: Tank = block_on_with(&clock, manager.read("sim://bench"))?;
/// assert_eq!(tank, Tank { level: 840, setpoint: 900 });
/// let response = block_on_with(&clock, manager.write("sim://bench", &Tank { level: 800, setpoint: 850 }))?;
/// assert_eq!(response.confirmed_count(), 2);
/// assert_eq!(cache.active_leases(), 0, "every lease is given back");
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sim", feature = "testkit")))]
/// # fn main() {}
/// ```
pub struct EntityManager<'a> {
    link: Link<'a>,
    registry: &'a DriverRegistry,
    aliases: BTreeMap<String, String>,
}

impl<'a> EntityManager<'a> {
    /// A manager that leases from `cache`, parses addresses with the drivers of `registry`
    /// (the registry the cache was built with) and reads time from `clock`, with no alias
    /// and a request timeout of one second.
    ///
    /// # Examples
    /// See [`EntityManager`].
    #[must_use]
    pub fn new(
        cache: &'a ConnectionCache,
        registry: &'a DriverRegistry,
        clock: &'a dyn Clock,
    ) -> Self {
        Self {
            link: Link {
                cache,
                clock,
                request_timeout: DEFAULT_REQUEST_TIMEOUT,
            },
            registry,
            aliases: BTreeMap::new(),
        }
    }

    /// The same manager, whose address `${alias}` stands for `address`; registering an
    /// alias again replaces its address.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-ALIAS
    /// Upstream's `SimpleAliasRegistry.register(alias, address)`.
    ///
    /// # Examples
    /// See [`EntityManager`].
    #[must_use]
    pub fn with_alias(mut self, alias: &str, address: &str) -> Self {
        self.aliases.insert(alias.to_owned(), address.to_owned());
        self
    }

    /// The same manager, whose requests time out `timeout` after they are sent
    /// ([`ErrorKind::Timeout`]). The wait for a lease is bounded by the cache's maximum
    /// wait time instead. A zero timeout fails every read and write with
    /// [`ErrorKind::Timeout`] before it is sent; the lease is still taken first, so the
    /// connection may be opened or pinged.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-TIMEOUT
    /// Upstream's system property `org.apache.plc4x.java.opm.entity_manager.read_timeout`
    /// (default 1000 ms), which bounds reads and writes alike; here a setting of one
    /// manager.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "testkit")]
    /// # fn main() -> Result<(), stonecrab::PlcError> {
    /// use std::sync::Arc;
    /// use std::time::Duration;
    /// use stonecrab::testkit::FakeClock;
    /// use stonecrab::{ConnectionCache, DriverRegistry, EntityManager};
    ///
    /// let clock = FakeClock::default();
    /// let registry = Arc::new(DriverRegistry::new());
    /// let cache = ConnectionCache::builder(Arc::clone(&registry), Arc::new(clock.clone())).build()?;
    /// let manager = EntityManager::new(&cache, &registry, &clock)
    ///     .with_request_timeout(Duration::from_secs(3));
    /// # drop(manager);
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "testkit"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub const fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.link.request_timeout = timeout;
        self
    }

    /// A new entity whose mapped fields are filled from one read.
    ///
    /// # Errors
    /// Before any I/O: [`ErrorKind::InvalidAddress`] for an address the driver refuses, an
    /// alias that is not registered or an address that starts with `$` without being
    /// `${name}`; the kinds of [`DriverRegistry::parse`] for the connection string. Then
    /// the kinds of [`ConnectionCache::get`] and of [`Connection::read`], and
    /// [`ErrorKind::Timeout`] past the request timeout. A field whose item failed fails the
    /// read with the item's kind, and a field whose type cannot hold its value with
    /// [`ErrorKind::InvalidConfig`]; each message names the field.
    ///
    /// # Cancel safety
    /// Cancel-safe (no bytes sent on drop): a read changes nothing on the device. Dropping
    /// the future gives the lease back. The deadline source is the request timeout, and of
    /// the wait for the lease the cache's maximum wait time.
    ///
    /// # Wire effects
    /// One read of every mapped field, after what [`ConnectionCache::get`] sends to open or
    /// ping the connection.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-MANAGER
    /// Upstream's `read(Class, address)`: `connect` then `disconnect`.
    ///
    /// # Examples
    /// See [`EntityManager`].
    pub fn read<T>(&self, connection_string: &str) -> OpFuture<'_, T>
    where
        T: PlcEntity + Default + Send + 'a,
    {
        let connecting = self.connect::<T>(connection_string);
        Box::pin(async move { Ok(connecting.await?.disconnect()) })
    }

    /// Write every mapped field of `entity` in one write.
    ///
    /// The response says, per field (its [`TagId`] is the field's name), whether the write
    /// was confirmed, rejected or is unknown: a rejected field is data, not an error.
    ///
    /// # Errors
    /// The address errors of [`read`](Self::read), [`ErrorKind::InvalidConfig`] when the
    /// entity has no mapped field or no value for a field it lists, all before any I/O;
    /// then the kinds of
    /// [`ConnectionCache::get`] and [`Connection::write`], and [`ErrorKind::Timeout`] past
    /// the request timeout.
    ///
    /// # Cancel safety
    /// Not cancel-safe: the request may have been transmitted; its outcome is unknown.
    /// Dropping the future gives the lease back. The deadline source is the request
    /// timeout, and of the wait for the lease the cache's maximum wait time.
    ///
    /// # Wire effects
    /// One write of every mapped field, after what [`ConnectionCache::get`] sends. A write
    /// cancelled or timed out after it was sent may have been applied.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-OPERATIONS
    /// Upstream's `write(Class, address, instance)`: `merge` then `disconnect`. Reads and
    /// writes are the only operations the manager uses.
    ///
    /// # Examples
    /// See [`EntityManager`].
    pub fn write<T: PlcEntity>(
        &self,
        connection_string: &str,
        entity: &T,
    ) -> OpFuture<'_, WriteResponse> {
        let items = self
            .tags(connection_string, entity.fields())
            .and_then(|tags| items(entity, tags));
        let connection_string = connection_string.to_owned();
        let link = self.link;
        Box::pin(async move { link.write(&connection_string, items?).await })
    }

    /// A new entity filled from one read, kept attached to `connection_string`.
    ///
    /// # Errors
    /// See [`read`](Self::read).
    ///
    /// # Cancel safety
    /// See [`read`](Self::read).
    ///
    /// # Wire effects
    /// See [`read`](Self::read).
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-MANAGER
    /// Upstream's `connect(Class, address)`, which returns a proxy after reading every
    /// field.
    ///
    /// # Examples
    /// See [`ConnectedEntity`].
    pub fn connect<T>(&self, connection_string: &str) -> OpFuture<'_, ConnectedEntity<'a, T>>
    where
        T: PlcEntity + Default + Send + 'a,
    {
        let attached = self.attach(connection_string, T::default());
        Box::pin(async move {
            let mut connected = attached?;
            connected.refresh().await?;
            Ok(connected)
        })
    }

    /// Write every mapped field of `entity` in one write, and keep it attached to
    /// `connection_string`, with the write's response.
    ///
    /// Its fields count as never read: the first
    /// [`refresh`](ConnectedEntity::refresh) reads them all.
    ///
    /// # Errors
    /// See [`write`](Self::write).
    ///
    /// # Cancel safety
    /// See [`write`](Self::write).
    ///
    /// # Wire effects
    /// See [`write`](Self::write).
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-MANAGER
    /// Upstream's `merge(Class, address, instance)`, which copies the instance into a new
    /// proxy and writes every field.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::{EntityManager, PlcEntity, Result, WriteOutcome};
    ///
    /// async fn store<T: PlcEntity + Send>(manager: &EntityManager<'_>, entity: T) -> Result<T> {
    ///     let (connected, response) = manager.merge("sim://bench", entity).await?;
    ///     let all_confirmed = response
    ///         .results()
    ///         .iter()
    ///         .all(|result| matches!(result.outcome(), WriteOutcome::Confirmed));
    ///     assert!(all_confirmed, "{response:?}");
    ///     Ok(connected.disconnect())
    /// }
    /// ```
    pub fn merge<T>(
        &self,
        connection_string: &str,
        entity: T,
    ) -> OpFuture<'_, (ConnectedEntity<'a, T>, WriteResponse)>
    where
        T: PlcEntity + Send + 'a,
    {
        let attached = self
            .attach(connection_string, entity)
            .and_then(|connected| {
                let items = items(&connected.entity, connected.tags.clone())?;
                Ok((connected, items))
            });
        Box::pin(async move {
            let (connected, items) = attached?;
            let response = connected
                .link
                .write(&connected.connection_string, items)
                .await?;
            Ok((connected, response))
        })
    }

    /// `entity` attached to `connection_string`, with its tags parsed and no field read.
    fn attach<T: PlcEntity>(
        &self,
        connection_string: &str,
        entity: T,
    ) -> Result<ConnectedEntity<'a, T>> {
        let tags = self.tags(connection_string, entity.fields())?;
        Ok(ConnectedEntity {
            link: self.link,
            connection_string: connection_string.to_owned(),
            fetched: vec![None; tags.len()],
            tags,
            entity,
        })
    }

    /// The tag of each field, its address parsed by the connection string's driver.
    fn tags(&self, connection_string: &str, fields: &[EntityField]) -> Result<Vec<Tag>> {
        let (driver, config) = self.registry.parse(connection_string)?;
        let options = config.address_options();
        fields
            .iter()
            .map(|field| {
                let name = field.name();
                let parsed = self
                    .address(field.address())
                    .and_then(|address| driver.parse_address(address, &options))
                    .and_then(|address| Ok(Tag::new(TagId::new(name)?, address)));
                parsed.map_err(|error| in_field(name, &error))
            })
            .collect()
    }

    /// The address `text` stands for: itself, or the registered address of `${alias}`.
    fn address<'t>(&'t self, text: &'t str) -> Result<&'t str> {
        if !text.starts_with('$') {
            return Ok(text);
        }
        let alias = text
            .strip_prefix("${")
            .and_then(|rest| rest.strip_suffix('}'))
            .ok_or_else(|| {
                PlcError::new(
                    ErrorKind::InvalidAddress,
                    format!(
                        "the address {text:?} starts with $ but is not an alias of the form \
                         ${{name}}; an address does not start with $"
                    ),
                )
            })?;
        self.aliases.get(alias).map(String::as_str).ok_or_else(|| {
            PlcError::new(
                ErrorKind::InvalidAddress,
                format!("no alias named {alias:?} is registered with the entity manager"),
            )
        })
    }
}

impl fmt::Debug for EntityManager<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntityManager")
            .field("aliases", &self.aliases)
            .field("request_timeout", &self.link.request_timeout)
            .finish_non_exhaustive()
    }
}

/// An entity attached to a connection string by [`EntityManager::connect`] or
/// [`EntityManager::merge`]; [`refresh`](Self::refresh) reads again the fields whose cache
/// duration has passed.
///
/// It holds no lease: each refresh takes one from the cache and gives it back.
/// [`disconnect`](Self::disconnect) gives the plain entity back.
///
/// # Compatibility: docs/parity/tools.md#TOOL-OPM-CACHE
/// Upstream's connected proxy, whose getters re-read a field once its
/// `cacheDurationMillis` has passed and whose other methods re-read every such field
/// first. Rust has no getter to intercept, so the caller asks for the refresh.
///
/// # Examples
/// ```
/// # #[cfg(all(feature = "sim", feature = "testkit"))]
/// # fn main() -> Result<(), stonecrab::PlcError> {
/// use std::sync::Arc;
/// use std::time::Duration;
/// use stonecrab::sim::{SimDevice, SimDriver, SimTag};
/// use stonecrab::testkit::{FakeClock, block_on_with};
/// use stonecrab::{ConnectionCache, DriverRegistry, EntityManager, PlcValue};
///
/// stonecrab::plc_entity! {
///     #[derive(Debug, Default)]
///     struct Tank {
///         level: u16 => "STATE/level:u16",
///         setpoint: i16 => "STATE/setpoint:i16" (cache = Duration::from_secs(5)),
///     }
/// }
///
/// let clock = FakeClock::default();
/// let device = SimDevice::new();
/// device.set("level", SimTag::new(PlcValue::U16(840)))?;
/// device.set("setpoint", SimTag::new(PlcValue::I16(900)))?;
/// let mut registry = DriverRegistry::new();
/// registry.register(Arc::new(
///     SimDriver::new().with_clock(Arc::new(clock.clone())).with_device("bench", device)?,
/// ))?;
/// let registry = Arc::new(registry);
/// let cache = ConnectionCache::builder(Arc::clone(&registry), Arc::new(clock.clone())).build()?;
/// let manager = EntityManager::new(&cache, &registry, &clock);
///
/// let mut tank = block_on_with(&clock, manager.connect::<Tank>("sim://bench"))?;
/// assert_eq!(tank.entity().level, 840);
/// assert_eq!(block_on_with(&clock, tank.refresh())?, 1, "the setpoint is fresh for 5 s");
/// clock.advance(Duration::from_secs(6));
/// assert_eq!(block_on_with(&clock, tank.refresh())?, 2);
/// let tank: Tank = tank.disconnect();
/// assert_eq!(tank.setpoint, 900);
/// # Ok(())
/// # }
/// # #[cfg(not(all(feature = "sim", feature = "testkit")))]
/// # fn main() {}
/// ```
pub struct ConnectedEntity<'a, T> {
    link: Link<'a>,
    connection_string: String,
    /// The tag of each mapped field, in the order of [`PlcEntity::fields`].
    tags: Vec<Tag>,
    /// When each mapped field was last read, in the same order; `None` before its first.
    fetched: Vec<Option<Instant>>,
    entity: T,
}

impl<T> ConnectedEntity<'_, T> {
    /// The entity, as of its last refresh.
    ///
    /// # Examples
    /// See [`ConnectedEntity`].
    #[must_use]
    pub const fn entity(&self) -> &T {
        &self.entity
    }

    /// Detach the entity and give it back.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-MANAGER
    /// Upstream's `disconnect(entity)`, which detaches the proxy and refuses one that is
    /// already detached; consuming the handle makes that refusal unnecessary.
    ///
    /// # Examples
    /// See [`ConnectedEntity`].
    #[must_use]
    pub fn disconnect(self) -> T {
        self.entity
    }
}

impl<T: PlcEntity + Send> ConnectedEntity<'_, T> {
    /// Read again, in one request, every mapped field whose value is not fresh, and
    /// resolve to how many fields were read; zero sends nothing.
    ///
    /// A field without a cache duration is never fresh (upstream's -1). A field with one is
    /// fresh until more than its duration has passed on the clock since it was last read:
    /// at exactly its duration it is still fresh, as upstream compares with `isAfter`.
    ///
    /// # Errors
    /// The kinds of [`ConnectionCache::get`] and of [`Connection::read`], and
    /// [`ErrorKind::Timeout`] past the request timeout; a field whose item failed fails
    /// the refresh with the item's kind before any field changes. A field whose type cannot
    /// hold its value is [`ErrorKind::InvalidConfig`]: the fields before it, in the order of
    /// [`PlcEntity::fields`], hold their new values and count as read.
    ///
    /// # Cancel safety
    /// Cancel-safe (no bytes sent on drop): dropping the future changes no field and gives
    /// the lease back. The deadline source is the request timeout, and of the wait for the
    /// lease the cache's maximum wait time.
    ///
    /// # Wire effects
    /// One read of the fields that are not fresh, after what [`ConnectionCache::get`] sends;
    /// nothing when every field is fresh.
    ///
    /// # Compatibility: docs/parity/tools.md#TOOL-OPM-CACHE
    /// Upstream's `needsToBeSynced` decides per field, and `readAllFields` reads the fields
    /// it selects in one request.
    ///
    /// # Examples
    /// See [`ConnectedEntity`].
    pub fn refresh(&mut self) -> OpFuture<'_, usize> {
        Box::pin(async move {
            let now = self.link.clock.now();
            let fields = self.entity.fields();
            let due: Vec<usize> = fields
                .iter()
                .zip(&self.fetched)
                .enumerate()
                .filter(
                    |(_, (field, fetched))| match (field.cache_duration(), fetched) {
                        (Some(duration), Some(at)) => now.saturating_duration_since(*at) > duration,
                        _ => true,
                    },
                )
                .map(|(index, _)| index)
                .collect();
            if due.is_empty() {
                return Ok(0);
            }
            let tags = due
                .iter()
                .filter_map(|index| self.tags.get(*index).cloned())
                .collect();
            let response = self.link.read(&self.connection_string, tags).await?;
            let values = response
                .items()
                .iter()
                .map(|item| {
                    let field = item.tag().as_str();
                    let value = item.outcome().as_ref();
                    let value = value.map_err(|failed| in_field(field, failed.error()))?;
                    Ok((field, value.clone()))
                })
                .collect::<Result<Vec<_>>>()?;
            let read_at = self.link.clock.now();
            for (index, (field, value)) in due.iter().zip(values) {
                self.entity
                    .set_field_value(field, value)
                    .map_err(|error| in_field(field, &error))?;
                if let Some(fetched) = self.fetched.get_mut(*index) {
                    *fetched = Some(read_at);
                }
            }
            Ok(due.len())
        })
    }
}

impl<T: fmt::Debug> fmt::Debug for ConnectedEntity<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectedEntity")
            .field("connection_string", &self.connection_string)
            .field("entity", &self.entity)
            .finish_non_exhaustive()
    }
}
