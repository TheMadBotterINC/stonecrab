/// The fixed table of engineering units a profile may use.
///
/// Units are written as UCUM-style codes (`Cel`, `kPa`, `m3/h`) and compared as exact,
/// case-sensitive text. The table is closed on purpose: a profile that names a unit
/// outside it fails [`SensorProfile::validate`](crate::sensors::SensorProfile::validate) with
/// [`ProfileError::UnknownUnit`](crate::sensors::ProfileError::UnknownUnit) instead of carrying a
/// misspelling into a measurement. There is no unit arithmetic: the library never
/// converts between units, compares dimensions or parses compound expressions, so
/// `kPa` and `bar` are two entries and the profile's scaling does any conversion.
///
/// The codes are `%`, `1` (dimensionless), `A`, `Cel`, `Hz`, `K`, `L`, `L/min`, `Pa`,
/// `V`, `W`, `bar`, `kPa`, `kWh`, `m`, `m3/h`, `mm`, `ms` and `s`.
///
/// `Unit` has no values; it only groups the lookups.
///
/// # Compatibility: docs/parity/sensors.md#SENSORS-VALIDATION
/// Upstream PLC4X has no unit concept; this is a Stonecrab type with no upstream parity
/// claim.
///
/// # Examples
/// ```
/// use stonecrab::sensors::Unit;
///
/// assert!(Unit::is_known("Cel"));
/// assert!(!Unit::is_known("celsius"));
/// assert_eq!(Unit::description("kPa"), Some("kilopascal"));
/// assert_eq!(Unit::all().len(), 19);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Unit;

/// Declares the codes and descriptions as two parallel arrays from one list, so the
/// two cannot drift apart.
macro_rules! unit_table {
    ($(($code:literal, $description:literal)),+ $(,)?) => {
        const CODES: &[&str] = &[$($code),+];
        const DESCRIPTIONS: &[&str] = &[$($description),+];
    };
}

// Sorted by code in byte order; a test pins the order and the equal lengths.
unit_table![
    ("%", "percent"),
    ("1", "dimensionless (the number one)"),
    ("A", "ampere"),
    ("Cel", "degree Celsius"),
    ("Hz", "hertz"),
    ("K", "kelvin"),
    ("L", "litre"),
    ("L/min", "litre per minute"),
    ("Pa", "pascal"),
    ("V", "volt"),
    ("W", "watt"),
    ("bar", "bar"),
    ("kPa", "kilopascal"),
    ("kWh", "kilowatt hour"),
    ("m", "metre"),
    ("m3/h", "cubic metre per hour"),
    ("mm", "millimetre"),
    ("ms", "millisecond"),
    ("s", "second"),
];

impl Unit {
    /// True when `code` is in the unit table. The comparison is exact and case-sensitive.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::Unit;
    ///
    /// assert!(Unit::is_known("m3/h"));
    /// assert!(!Unit::is_known("M3/H"));
    /// assert!(!Unit::is_known(""));
    /// ```
    #[must_use]
    pub fn is_known(code: &str) -> bool {
        CODES.contains(&code)
    }

    /// The human description of `code`, or `None` when it is not in the table.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::Unit;
    ///
    /// assert_eq!(Unit::description("Cel"), Some("degree Celsius"));
    /// assert_eq!(Unit::description("furlong"), None);
    /// ```
    #[must_use]
    pub fn description(code: &str) -> Option<&'static str> {
        let index = CODES.iter().position(|known| *known == code)?;
        DESCRIPTIONS.get(index).copied()
    }

    /// Every code in the table, sorted in byte order.
    ///
    /// # Examples
    /// ```
    /// use stonecrab::sensors::Unit;
    ///
    /// assert!(Unit::all().contains(&"kWh"));
    /// assert!(Unit::all().windows(2).all(|pair| pair[0] < pair[1]));
    /// ```
    #[must_use]
    pub const fn all() -> &'static [&'static str] {
        CODES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // covers: SENSORS-VALIDATION / unit table completeness
    #[test]
    fn every_listed_unit_has_a_description_and_the_list_is_sorted() {
        assert_eq!(CODES.len(), DESCRIPTIONS.len(), "parallel tables");
        for code in Unit::all() {
            let description = Unit::description(code).expect("described");
            assert!(!description.is_empty(), "description of {code}");
            assert!(description.is_ascii(), "ASCII description of {code}");
            assert!(Unit::is_known(code), "known: {code}");
        }
        assert!(
            Unit::all()
                .windows(2)
                .all(|pair| matches!(pair, [a, b] if a < b)),
            "sorted, no duplicates"
        );
    }

    #[test]
    fn the_required_units_are_present() {
        for code in [
            "Cel", "K", "kPa", "Pa", "bar", "m3/h", "L/min", "L", "mm", "m", "%", "A", "V", "W",
            "kWh", "Hz", "s", "ms", "1",
        ] {
            assert!(Unit::is_known(code), "required unit {code}");
        }
        assert_eq!(Unit::all().len(), 19, "no unlisted extras");
    }

    #[test]
    fn lookups_are_exact_and_case_sensitive() {
        for code in ["", "cel", "CEL", "Cel ", " Cel", "kpa", "M", "degC", "m/s"] {
            assert!(!Unit::is_known(code), "rejected: {code:?}");
            assert_eq!(Unit::description(code), None, "no description: {code:?}");
        }
        assert_eq!(
            Unit::description("1"),
            Some("dimensionless (the number one)")
        );
    }
}
