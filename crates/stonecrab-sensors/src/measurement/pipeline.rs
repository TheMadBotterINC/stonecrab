//! The measurement pipeline: a read response in, qualified engineering values out.

use std::fmt;
use std::time::{Instant, SystemTime};

use stonecrab_core::{PlcValue, ProtocolStatus, ReadResponse, TagId};

use super::scale::{as_f64, scale, status_bit};
use crate::{MeasurementDef, SensorProfile, StatusField, ValidationStatus};

/// How far a measurement can be trusted.
///
/// [`SensorProfile::measurements`] produces `Good`, `Uncertain`, `Bad` and `Unknown`.
/// [`Quality::Stale`] is never produced by the pipeline: staleness depends on how long ago
/// a value was read, which one response cannot say, so it is set only by the tracker that
/// compares a measurement's age with `stale_after` (milestone M2). The enum is
/// `#[non_exhaustive]`.
///
/// Precedence when several rules apply to one measurement is `Bad` over `Uncertain` over
/// `Unknown` over `Good`; see [`SensorProfile::measurements`].
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-MEASUREMENTS
/// Upstream PLC4X has no quality model; this is a Stonecrab type with no upstream parity
/// claim.
///
/// # Examples
/// ```
/// use stonecrab_sensors::Quality;
///
/// assert_eq!(Quality::Good.to_string(), "good");
/// assert_eq!(Quality::Unknown.to_string(), "unknown");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Quality {
    /// The value was read, scaled and passed every rule.
    Good,
    /// The value is usable with care: outside its plausible range, or a status field in
    /// `uncertain_when` is set.
    Uncertain,
    /// The value must not be used: the read failed, scaling failed, or a status field in
    /// `bad_when` is set.
    Bad,
    /// The value is older than its `stale_after`. Produced only by the M2 tracker, never by
    /// [`SensorProfile::measurements`].
    Stale,
    /// A status field the quality mapping depends on could not be read, so quality cannot
    /// be judged.
    Unknown,
}

impl Quality {
    /// How bad a quality is when two rules disagree: `Good` 0, `Unknown` 1, `Uncertain` 2,
    /// `Stale` 3, `Bad` 4.
    const fn severity(self) -> u8 {
        match self {
            Self::Good => 0,
            Self::Unknown => 1,
            Self::Uncertain => 2,
            Self::Stale => 3,
            Self::Bad => 4,
        }
    }

    /// The more severe of two qualities; the first wins a tie.
    const fn worse(self, other: Self) -> Self {
        if other.severity() > self.severity() {
            other
        } else {
            self
        }
    }
}

impl fmt::Display for Quality {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Good => "good",
            Self::Uncertain => "uncertain",
            Self::Bad => "bad",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
        })
    }
}

/// One measurement of a profile, read, scaled and qualified.
///
/// `Debug` shows every field. Nothing in a measurement is a secret: the protocol status is
/// core's `ProtocolStatus`, whose `Debug` shows the size of its diagnostic bytes and not
/// the bytes.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-MEASUREMENTS
/// Upstream PLC4X has no measurement type; this is a Stonecrab type with no upstream
/// parity claim.
///
/// # Examples
/// ```
/// use stonecrab_sensors::{Measurement, Quality};
///
/// fn usable(measurement: &Measurement) -> bool {
///     measurement.quality == Quality::Good && measurement.value.is_some()
/// }
/// # let _ = usable;
/// ```
/// See [`SensorProfile::measurements`] for building them from a read.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Measurement {
    /// The tag that was read: the measurement's name as a `TagId`.
    pub tag: TagId,
    /// The measurement's name in the profile.
    pub name: String,
    /// The scaled value. `None` when the read failed or the value could not be scaled
    /// (quality `Bad`). A `Bad` quality from a status field keeps the scaled value.
    pub value: Option<PlcValue>,
    /// The unscaled typed value as the driver delivered it, when the definition sets
    /// `keep_raw` and the read succeeded; `None` otherwise. It is a typed value, not wire
    /// bytes.
    pub raw: Option<PlcValue>,
    /// The engineering unit from the definition, for example `"Cel"`.
    pub unit: String,
    /// How far the value can be trusted.
    pub quality: Quality,
    /// How far the profile that produced this measurement was checked.
    pub origin: ValidationStatus,
    /// The wall-clock time the caller passed as `wall_now`: when this response was turned
    /// into measurements, not when the device sampled.
    pub received_at: SystemTime,
    /// The monotonic time at which the driver sampled the item.
    pub received_mono: Instant,
    /// The time the source (device or server) stamped on the value, when the driver
    /// reports one. It is never filled in from another clock.
    pub source_time: Option<SystemTime>,
    /// The protocol status the driver attached to the item, on success or failure.
    pub protocol_status: Option<ProtocolStatus>,
}

impl SensorProfile {
    /// Turn a read response into one qualified [`Measurement`] per definition.
    ///
    /// `response` must hold the items of the tags that [`SensorProfile::tags`] returned,
    /// or a subset. For each definition in document order the item with the definition's
    /// name is looked up; a definition whose item is absent from the response is skipped,
    /// so the result has one entry per measurement the response covers. The profile does
    /// not need to have been validated: an unevaluable step makes the value `Bad`
    /// instead of panicking.
    ///
    /// **Fields.** `received_at` is `wall_now`; `received_mono` is the item's `sampled_at`;
    /// `source_time` is the item's source time and stays `None` when the driver gave none
    /// (it is never substituted from `wall_now` or the monotonic clock); `protocol_status`
    /// is the item's status on success or failure; `origin` is the profile's
    /// [`ValidationStatus`]; `unit` is the definition's unit. `raw` holds the unscaled
    /// typed value when `keep_raw` is set and the read succeeded, and is `None` otherwise.
    ///
    /// **Scaling.** The steps run in declared order, with checked arithmetic. A value with
    /// no steps passes through unchanged, whatever its type (an `F32` stays `F32`).
    /// Otherwise the value must be a number and the promotion rule is:
    /// - An integer stays an integer through `Multiply` and `Offset` whose constant is
    ///   integral (finite, no fractional part, magnitude below 2^64; `1.0` and `-40.0`
    ///   are integral, `0.1` is not), through `Clamp` whose bounds are both integral, and
    ///   through `BitField`. `Clamp` on an integer then compares as integers.
    /// - `Divide` always computes in `f64`: integer division would lose the remainder,
    ///   and a result type that depends on the value (`100 / 10` against `101 / 10`)
    ///   would be worse. A fractional constant in `Multiply`, `Offset` or `Clamp` also
    ///   computes in `f64`, and so does any step that would take an integer outside
    ///   `i64::MIN..=u64::MAX`. After the first float step the value is a float for the
    ///   rest of the pipeline; `BitField` on a float is `Bad`.
    /// - An integer result is returned as the input's own type when it fits (`UINT` in,
    ///   `U16` out), else as `I64`, else as `U64`. A float result is always `F64`.
    /// - `BitField { shift, width }` reads `width` bits from bit `shift` of the integer's
    ///   64-bit pattern (negative values sign-extended), so a 16-bit field of the `U32`
    ///   `0x0001_FFFF` is `0xFFFF`, and one more count makes it `0`.
    /// - A result that is NaN or infinite at any step (including a `Divide` that
    ///   overflows `f64` and a divide by zero), a NaN or infinite input, a non-finite
    ///   constant, a clamp with `min > max`, a bit field of width 0 or ending past bit 64,
    ///   or a value that is not a number while steps exist, makes the measurement `Bad`
    ///   with `value` `None`.
    ///
    /// **Quality.** Each applicable rule gives a quality and the worst wins, in the order
    /// `Bad` over `Uncertain` over `Unknown` over `Good`:
    /// - an item error, or a value that cannot be scaled: `Bad`;
    /// - the scaled value outside `range` (`min..=max`, inclusive, compared as `f64`):
    ///   `Uncertain`. Schema version 1 has no way to say a range violation is `Bad`;
    /// - for each status field named in the quality mapping: its bit is read from the
    ///   status item (a `Bool` is the bit itself; an integer's bit `bit`, bit 0 the least
    ///   significant), flipped when `invert` is set, and when set it gives `Bad` for a
    ///   field in `bad_when` or `Uncertain` for one in `uncertain_when` (`Bad` if the
    ///   name is in both). A status item that is missing from the response, failed, or is
    ///   not a bool or an integer gives `Unknown`. A status field that no list names has
    ///   no effect, and a definition with no quality mapping has no status rules;
    /// - otherwise `Good`.
    ///
    /// A measurement of quality `Unknown` or `Bad` from a status field still carries its
    /// scaled value; only a failed read or a failed scaling leaves `value` empty.
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-MEASUREMENTS
    /// Upstream PLC4X has no measurement pipeline; this is a Stonecrab operation with no
    /// upstream parity claim.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "toml")]
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// use std::time::{Instant, SystemTime};
    /// use stonecrab_core::{PlcValue, ReadItem, ReadRequest, ReadResponse, ResourceLimits};
    /// use stonecrab_driver_sim::SimDriver;
    /// use stonecrab_sensors::{Quality, SensorProfile};
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_temperature"
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
    ///     name = "boiler_temperature"
    ///     address = "STATE/boiler:i16"
    ///     data_type = "INT"
    ///     unit = "Cel"
    ///     scaling = [{ multiply = 0.1 }]
    /// "#)?;
    /// let tags = profile.tags(&SimDriver::new())?;
    /// let mut request = ReadRequest::builder(ResourceLimits::builder().build()?);
    /// for tag in &tags {
    ///     request = request.tag(tag.clone());
    /// }
    /// let request = request.build()?;
    ///
    /// // A driver would deliver this; the example fabricates the item.
    /// let now = Instant::now();
    /// let response = ReadResponse::builder(&request, now, now)
    ///     .item(ReadItem::new(profile.measurements[0].name.clone(), Ok(PlcValue::I16(-2424)), now))
    ///     .build()?;
    ///
    /// let measurements = profile.measurements(&response, SystemTime::UNIX_EPOCH);
    /// assert_eq!(measurements.len(), 1);
    /// assert_eq!(measurements[0].quality, Quality::Good);
    /// assert_eq!(measurements[0].unit, "Cel");
    /// assert!(measurements[0].source_time.is_none());
    /// assert!(matches!(&measurements[0].value, Some(PlcValue::F64(v)) if (v + 242.4).abs() < 1e-9));
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "toml"))]
    /// # fn main() {}
    /// ```
    #[must_use]
    pub fn measurements(&self, response: &ReadResponse, wall_now: SystemTime) -> Vec<Measurement> {
        self.measurements
            .iter()
            .filter_map(|definition| measure(self.validation, definition, response, wall_now))
            .collect()
    }
}

/// One definition against the response, or `None` when the response lacks its item.
fn measure(
    origin: ValidationStatus,
    definition: &MeasurementDef,
    response: &ReadResponse,
    wall_now: SystemTime,
) -> Option<Measurement> {
    let item = response.get(&definition.name)?;
    let mut measurement = Measurement {
        tag: definition.name.clone(),
        name: definition.name.as_str().to_owned(),
        value: None,
        raw: None,
        unit: definition.unit.clone(),
        quality: Quality::Bad,
        origin,
        received_at: wall_now,
        received_mono: item.sampled_at(),
        source_time: item.source_time(),
        protocol_status: item.status().cloned(),
    };
    if let Ok(unscaled) = item.outcome() {
        measurement.raw = definition.keep_raw.then(|| unscaled.clone());
        measurement.value = scale(unscaled, &definition.scaling);
        measurement.quality = measurement.value.as_ref().map_or(Quality::Bad, |scaled| {
            range_quality(definition, scaled).worse(status_quality(definition, response))
        });
    }
    Some(measurement)
}

/// `Uncertain` when the scaled value is outside the definition's range, else `Good`.
fn range_quality(definition: &MeasurementDef, scaled: &PlcValue) -> Quality {
    let outside = definition
        .range
        .zip(as_f64(scaled))
        .is_some_and(|(range, number)| number < range.min || number > range.max);
    if outside {
        Quality::Uncertain
    } else {
        Quality::Good
    }
}

/// The worst quality any status field named in the quality mapping gives.
fn status_quality(definition: &MeasurementDef, response: &ReadResponse) -> Quality {
    let Some(map) = &definition.quality_map else {
        return Quality::Good;
    };
    definition
        .status_fields
        .iter()
        .fold(Quality::Good, |worst, field| {
            let if_set = if map.bad_when.contains(&field.name) {
                Quality::Bad
            } else if map.uncertain_when.contains(&field.name) {
                Quality::Uncertain
            } else {
                return worst;
            };
            worst.worse(match status_state(definition, field, response) {
                Some(true) => if_set,
                Some(false) => Quality::Good,
                None => Quality::Unknown,
            })
        })
}

/// Whether a status field is set (after `invert`), or `None` when it cannot be read.
fn status_state(
    definition: &MeasurementDef,
    field: &StatusField,
    response: &ReadResponse,
) -> Option<bool> {
    let id = definition.status_tag_id(field).ok()?;
    let value = response.get(&id)?.outcome().as_ref().ok()?;
    Some(status_bit(value, field.bit)? != field.invert)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Quality; 5] = [
        Quality::Good,
        Quality::Uncertain,
        Quality::Bad,
        Quality::Stale,
        Quality::Unknown,
    ];

    #[test]
    fn display_is_the_lower_case_name() {
        let names = ["good", "uncertain", "bad", "stale", "unknown"];
        for (quality, name) in ALL.into_iter().zip(names) {
            assert_eq!(quality.to_string(), name, "display of {quality:?}");
        }
    }

    #[test]
    fn severity_orders_bad_over_uncertain_over_unknown_over_good() {
        assert!(
            Quality::Bad.severity() > Quality::Stale.severity(),
            "bad > stale"
        );
        assert!(
            Quality::Stale.severity() > Quality::Uncertain.severity(),
            "stale > uncertain"
        );
        assert!(
            Quality::Uncertain.severity() > Quality::Unknown.severity(),
            "uncertain > unknown"
        );
        assert!(
            Quality::Unknown.severity() > Quality::Good.severity(),
            "unknown > good"
        );
    }

    #[test]
    fn worse_picks_the_more_severe_and_the_first_on_a_tie() {
        for first in ALL {
            for second in ALL {
                let expected = if second.severity() > first.severity() {
                    second
                } else {
                    first
                };
                assert_eq!(first.worse(second), expected, "{first:?} then {second:?}");
            }
        }
        assert_eq!(Quality::Good.worse(Quality::Good), Quality::Good, "tie");
        assert_eq!(
            Quality::Uncertain.worse(Quality::Bad),
            Quality::Bad,
            "bad wins"
        );
        assert_eq!(
            Quality::Bad.worse(Quality::Uncertain),
            Quality::Bad,
            "bad stays"
        );
    }
}
