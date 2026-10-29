//! The stale tracker: the last value of each measurement and its age (ADR-0023).

use std::time::{Duration, Instant};

use crate::TagId;
use crate::sensors::{Measurement, Quality, SensorProfile};

/// The last value of each measurement of a profile, marked [`Quality::Stale`] once it is
/// older than the measurement's `stale_after`.
///
/// Pure logic: it performs no I/O and has no clock. The caller passes the monotonic reading
/// of the clock its driver samples with ([`Clock::now`](crate::Clock::now)), and a
/// measurement's age is that reading minus its `received_mono`. [`Acquisition`] drives one;
/// an application that reads by itself can drive its own.
///
/// **Retention.** [`record`](Self::record) retains a measurement that carries a value, in
/// place of the one retained for its name. A measurement without a value (its read or its
/// scaling failed, quality `Bad`) is not retained: it erases nothing, and the value
/// retained before it keeps its age. A measurement whose name the profile does not define is
/// not retained either.
///
/// **The threshold.** A retained measurement is stale when its age is strictly greater than
/// its `stale_after`: at exactly `stale_after` it is still fresh, and it is stale from the
/// next instant on, which is what [`next_expiry`](Self::next_expiry) returns
/// (`received_mono + stale_after + 1 ns`). A measurement whose definition has no
/// `stale_after` is never stale.
///
/// **Marking.** [`expire`](Self::expire) marks every retained measurement that has become
/// stale and returns each one once, at its transition: `quality` becomes
/// [`Quality::Stale`] whatever it was, and that earlier quality moves to
/// [`Measurement::quality_before_stale`]. The value, the raw value, both receipt times, the
/// source time and the protocol status are kept as they were read: a retained value is
/// never stamped as a new reading. The next measurement recorded with a value for that name
/// replaces it, fresh again.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-STALE
/// Upstream PLC4X has no quality model and no staleness; this is a Stonecrab type with no
/// upstream parity claim.
///
/// # Examples
/// ```
/// # #[cfg(feature = "sim")]
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use std::time::{Duration, Instant, SystemTime};
/// use stonecrab::{PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits};
/// use stonecrab::sensors::{Quality, SensorProfile, StaleTracker};
/// use stonecrab::sim::SimDriver;
///
/// let profile = SensorProfile::from_toml(r#"
///     id = "synthetic_level"
///     schema_version = 1
///     validation = "synthetic"
///
///     [connection]
///     scheme = "sim"
///
///     [provenance]
///     source = "Synthetic reference profile; no real device"
///
///     [[measurements]]
///     name = "tank_level"
///     address = "STATE/level:u16"
///     data_type = "UINT"
///     unit = "mm"
///     stale_after_ms = 5000
/// "#)?;
/// let tags = profile.tags(&SimDriver::new())?;
/// let request = tags
///     .iter()
///     .fold(ReadRequest::builder(ResourceLimits::builder().build()?), |request, tag| {
///         request.tag(tag.clone())
///     })
///     .build()?;
///
/// // A driver would deliver this; the example fabricates the item.
/// let read_at = Instant::now();
/// let response = ReadResponse::builder(&request, read_at, read_at)
///     .item(ReadItem::new(tags[0].id().clone(), Ok(PlcValue::U16(840)), read_at))
///     .build()?;
///
/// let mut tracker = StaleTracker::new(&profile);
/// for measurement in profile.measurements(&response, SystemTime::UNIX_EPOCH) {
///     tracker.record(measurement);
/// }
/// assert!(tracker.expire(read_at + Duration::from_secs(5)).is_empty(), "fresh at 5 s");
/// let stale = tracker.expire(read_at + Duration::from_millis(5001));
/// assert_eq!(stale[0].quality, Quality::Stale);
/// assert_eq!(stale[0].quality_before_stale, Some(Quality::Good));
/// assert_eq!(stale[0].value, Some(PlcValue::U16(840)));
/// assert_eq!(stale[0].received_mono, read_at);
/// # Ok(())
/// # }
/// # #[cfg(not(feature = "sim"))]
/// # fn main() {}
/// ```
///
/// [`Acquisition`]: crate::sensors::Acquisition
#[derive(Debug, Clone)]
pub struct StaleTracker {
    /// One entry per definition of the profile, in document order.
    entries: Vec<Entry>,
}

#[derive(Debug, Clone)]
struct Entry {
    name: TagId,
    stale_after: Option<Duration>,
    retained: Option<Measurement>,
}

impl Entry {
    /// The first instant at which the retained measurement is stale, if it is fresh and can
    /// go stale (the sum is representable).
    fn expiry(&self) -> Option<Instant> {
        let retained = self.retained.as_ref()?;
        if retained.quality == Quality::Stale {
            return None;
        }
        retained
            .received_mono
            .checked_add(self.stale_after?)?
            .checked_add(Duration::from_nanos(1))
    }
}

impl StaleTracker {
    /// A tracker for the measurements of `profile`, with nothing retained.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::{SensorProfile, StaleTracker};
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_empty"
    ///     schema_version = 1
    ///     validation = "synthetic"
    ///     [connection]
    ///     scheme = "sim"
    ///     [provenance]
    ///     source = "Synthetic reference profile; no real device"
    /// "#)?;
    /// assert_eq!(StaleTracker::new(&profile).next_expiry(), None);
    /// # Ok::<(), stonecrab::sensors::ProfileError>(())
    /// ```
    #[must_use]
    pub fn new(profile: &SensorProfile) -> Self {
        Self {
            entries: profile
                .measurements
                .iter()
                .map(|definition| Entry {
                    name: definition.name.clone(),
                    stale_after: definition.stale_after,
                    retained: None,
                })
                .collect(),
        }
    }

    /// Retain `measurement` as the last value of its name, if it carries a value.
    ///
    /// A measurement with a value replaces the one retained for its name, stale or not. A
    /// measurement without a value, or of a name the profile does not define, changes
    /// nothing. See the type documentation.
    ///
    /// # Examples
    /// See [`StaleTracker`].
    pub fn record(&mut self, measurement: Measurement) {
        if measurement.value.is_none() {
            return;
        }
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.name == measurement.tag)
        {
            entry.retained = Some(measurement);
        }
    }

    /// Mark every retained measurement whose age at `now` exceeds its `stale_after`, and
    /// return those that became stale with this call, in document order.
    ///
    /// Each one is returned once, as retained after marking: [`Quality::Stale`], with the
    /// quality it was read with in [`Measurement::quality_before_stale`] and every other field
    /// unchanged. A measurement that was already stale is not returned again.
    ///
    /// # Examples
    /// See [`StaleTracker`].
    #[must_use = "the measurements that went stale are the transitions to report"]
    pub fn expire(&mut self, now: Instant) -> Vec<Measurement> {
        let mut stale = Vec::new();
        for entry in &mut self.entries {
            let Some(at) = entry.expiry() else {
                continue;
            };
            if let Some(retained) = entry.retained.as_mut()
                && now >= at
            {
                retained.quality_before_stale = Some(retained.quality);
                retained.quality = Quality::Stale;
                stale.push(retained.clone());
            }
        }
        stale
    }

    /// The earliest instant at which [`expire`](Self::expire) will mark a retained
    /// measurement stale, or `None` when no retained measurement can go stale.
    ///
    /// It is the first instant past `received_mono + stale_after` of the freshest-expiring
    /// retained measurement that is not stale yet. A loop around the tracker sleeps until
    /// then (or until its next read) and calls `expire`.
    ///
    /// # Examples
    /// See [`StaleTracker::new`].
    #[must_use]
    pub fn next_expiry(&self) -> Option<Instant> {
        self.entries.iter().filter_map(Entry::expiry).min()
    }
}
