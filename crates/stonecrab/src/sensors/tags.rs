//! Turning a profile into the tags a driver reads.

use std::sync::Arc;

use crate::{AddressOptions, ByteOrder, Driver, ProtocolAddress, Tag, TagId, WordOrder};

use crate::sensors::{MeasurementDef, ProfileError, SensorProfile, StatusField};

impl MeasurementDef {
    /// The tag identity of one of this measurement's status fields:
    /// `<measurement name>.<status field name>`.
    ///
    /// [`SensorProfile::tags`] gives the status tag this identity and
    /// [`SensorProfile::measurements`] reads the status bit back through it. Measurement and
    /// status field names are `snake_case` without a dot, so the identity never equals a
    /// measurement's own tag and is unique per measurement and status name. The longest
    /// result is 64 + 1 + 64 bytes, inside the `TagId` limit.
    ///
    /// # Errors
    /// Returns [`ProfileError::InvalidIdentifier`] if `TagId` refuses the joined name, which
    /// the length limits make impossible for names the loader accepted.
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-TAGS
    /// Upstream PLC4X has no status tag naming; this is a Stonecrab convention with no
    /// upstream parity claim.
    ///
    /// # Examples
    /// ```
    /// # fn main() -> Result<(), stonecrab::sensors::ProfileError> {
    /// use stonecrab::sensors::MeasurementDef;
    ///
    /// let definition: MeasurementDef = toml::from_str(r#"
    ///     name = "boiler_temperature"
    ///     address = "holding-register:1:INT"
    ///     data_type = "INT"
    ///     unit = "Cel"
    ///     status_fields = [
    ///         { name = "open_circuit", address = "coil:3", bit = 0 },
    ///     ]
    /// "#).expect("a valid definition");
    /// let id = definition.status_tag_id(&definition.status_fields[0])?;
    /// assert_eq!(id.as_str(), "boiler_temperature.open_circuit");
    /// # Ok(())
    /// # }
    /// ```
    #[must_use = "tag identity errors must be checked"]
    pub fn status_tag_id(&self, field: &StatusField) -> Result<TagId, ProfileError> {
        TagId::new(&format!("{}.{}", self.name, field.name)).map_err(|_error| STATUS_TAG_NAME)
    }

    /// Parse this measurement's address with `driver`; `index` names the measurement in the
    /// error, which never repeats the address text. [`SensorProfile::validate_for`] and
    /// [`SensorProfile::tags`] both parse through here, so they refuse the same addresses.
    pub(super) fn parse_address(
        &self,
        index: usize,
        driver: &dyn Driver,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>, ProfileError> {
        driver
            .parse_address(&self.address, options)
            .map_err(|_error| ProfileError::AddressRejected { measurement: index })
    }
}

impl StatusField {
    /// Parse this status field's address with `driver`; `measurement` and `position` name
    /// the field in the error, which never repeats the address text. Like
    /// [`MeasurementDef::parse_address`], this is the one place
    /// [`SensorProfile::validate_for`] and [`SensorProfile::tags`] map a refusal, so they
    /// refuse the same addresses.
    pub(super) fn parse_address(
        &self,
        measurement: usize,
        position: usize,
        driver: &dyn Driver,
        options: &AddressOptions,
    ) -> Result<Arc<dyn ProtocolAddress>, ProfileError> {
        driver
            .parse_address(&self.address, options)
            .map_err(|_error| ProfileError::StatusAddressRejected {
                measurement,
                status_field: position,
            })
    }
}

/// What a status tag identity that `TagId` refuses reports; the limits make it unreachable.
const STATUS_TAG_NAME: ProfileError = ProfileError::InvalidIdentifier {
    field: "status tag name",
};

impl SensorProfile {
    /// The address options a driver parses this profile's addresses under.
    ///
    /// The profile's `address_convention` becomes the options' convention; a profile that
    /// declares none yields the default options, under which a driver refuses ambiguous
    /// short forms such as a bare `40001`. [`SensorProfile::tags`] passes these options to
    /// every `parse_address` call, so a bare short form parses only under the convention
    /// the profile declared.
    ///
    /// # Errors
    /// Returns [`ProfileError::UnknownAddressConvention`] when the declared convention is
    /// not a valid convention name (empty, longer than 64 bytes, or containing whitespace
    /// or control characters). [`SensorProfile::validate`] is stricter and accepts only
    /// `plc4x`, `one-based-40001` and `zero-based`. The error converts into a
    /// [`PlcError`](crate::PlcError) of kind `InvalidConfig`.
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-TAGS
    /// Upstream parses address text with the driver's field handler and has no convention
    /// argument; this is the Stonecrab `AddressOptions` addition (decision D16).
    ///
    /// # Examples
    /// ```
    /// # fn main() -> Result<(), stonecrab::sensors::ProfileError> {
    /// use stonecrab::sensors::SensorProfile;
    ///
    /// let profile = SensorProfile::from_toml(r#"
    ///     id = "synthetic_temperature"
    ///     schema_version = 1
    ///     validation = "synthetic"
    ///
    ///     [connection]
    ///     scheme = "modbus-tcp"
    ///     address_convention = "plc4x"
    ///
    ///     [provenance]
    ///     source = "Synthetic reference profile; no real device"
    /// "#)?;
    /// assert_eq!(profile.address_options()?.convention(), Some("plc4x"));
    /// # Ok(())
    /// # }
    /// ```
    #[must_use = "address option errors must be checked"]
    pub fn address_options(&self) -> Result<AddressOptions, ProfileError> {
        let builder = AddressOptions::builder();
        let builder = match &self.connection.address_convention {
            Some(convention) => builder.convention(convention),
            None => builder,
        };
        builder
            .build()
            .map_err(|_error| ProfileError::UnknownAddressConvention)
    }

    /// Build the tags that read this profile through `driver`.
    ///
    /// The profile must belong to the driver: the driver's scheme must equal the profile's
    /// `connection.scheme`. The profile is then checked with [`SensorProfile::validate_for`]
    /// and the first problem is returned (call it yourself to get all of them). Every
    /// address is parsed by `driver.parse_address` under [`SensorProfile::address_options`],
    /// so the declared address convention reaches the driver on every call.
    ///
    /// The result holds, in document order, one tag per measurement followed by one tag
    /// per status field of that measurement:
    /// - the measurement's tag has the measurement's name as its `TagId`, the driver's own
    ///   value type hint from the parsed address (it is not overridden, and
    ///   `data_type` is documentary at this point: the type comes from the address), and
    ///   the definition's `byte_order` and `word_order` when it sets them. `count` is 1,
    ///   which validation guarantees;
    /// - a status field's tag has the `TagId` `<measurement>.<status name>`, for example
    ///   `boiler_temperature.open_circuit`, and the type hint of the driver's parse of the
    ///   status address. A status address that parses to a `BOOL` (a coil or discrete
    ///   input) delivers the bit itself; one that parses to an integer word delivers the
    ///   word and the pipeline reads the field's `bit` from it.
    ///
    /// Read the tags, then pass the [`ReadResponse`](crate::ReadResponse) to
    /// [`SensorProfile::measurements`].
    ///
    /// # Errors
    /// Returns [`ProfileError::SchemeMismatch`] when the driver's scheme is not the
    /// profile's, any error of [`SensorProfile::validate_for`] (the first, in its order: among
    /// them `MissingByteOrder` and `MissingWordOrder` for a value that spans several words,
    /// and [`ProfileError::AddressRejected`] when the driver refuses a measurement's
    /// address), [`ProfileError::StatusAddressRejected`] when the driver refuses a status
    /// address (the errors name the measurement and status field by index and never repeat
    /// the address text),
    /// [`ProfileError::DuplicateStatusFieldName`] when two status fields of one measurement
    /// share a name, and [`ProfileError::UnknownAddressConvention`] as for
    /// [`SensorProfile::address_options`]. Each converts into a
    /// [`PlcError`](crate::PlcError) of kind `InvalidConfig`.
    ///
    /// # Compatibility: docs/parity/sensors.md#SENSORS-TAGS
    /// Upstream has no profile concept and parses each address with the driver's field
    /// handler; the convention argument is the Stonecrab `AddressOptions` addition.
    ///
    /// # Examples
    /// ```
    /// # #[cfg(feature = "sim")]
    /// # fn main() -> Result<(), stonecrab::sensors::ProfileError> {
    /// use stonecrab::PlcValueType;
    /// use stonecrab::sim::SimDriver;
    /// use stonecrab::sensors::SensorProfile;
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
    ///     scaling = [{ divide = 10.0 }]
    ///     status_fields = [
    ///         { name = "open_circuit", address = "STATE/open_circuit:bool", bit = 0 },
    ///     ]
    /// "#)?;
    ///
    /// let tags = profile.tags(&SimDriver::new())?;
    /// let ids: Vec<&str> = tags.iter().map(|tag| tag.id().as_str()).collect();
    /// assert_eq!(ids, ["boiler_temperature", "boiler_temperature.open_circuit"]);
    /// assert_eq!(tags[0].value_type(), Some(PlcValueType::I16));
    /// # Ok(())
    /// # }
    /// # #[cfg(not(feature = "sim"))]
    /// # fn main() {}
    /// ```
    #[must_use = "tag building errors must be checked"]
    pub fn tags(&self, driver: &dyn Driver) -> Result<Vec<Tag>, ProfileError> {
        if driver.scheme() != self.connection.scheme {
            return Err(ProfileError::SchemeMismatch {
                driver: driver.scheme(),
            });
        }
        if let Some(first) = self
            .validate_for(driver)
            .err()
            .and_then(|errors| errors.first().copied())
        {
            return Err(first);
        }
        let options = self.address_options()?;
        let mut tags = Vec::new();
        for (index, definition) in self.measurements.iter().enumerate() {
            let address = definition.parse_address(index, driver, &options)?;
            let mut tag = Tag::new(definition.name.clone(), address);
            if let Some(order) = definition.byte_order {
                tag = tag.with_byte_order(ByteOrder::from(order));
            }
            if let Some(order) = definition.word_order {
                tag = tag.with_word_order(WordOrder::from(order));
            }
            tags.push(tag);
            let mut seen: Vec<&str> = Vec::new();
            for (position, field) in definition.status_fields.iter().enumerate() {
                if seen.contains(&field.name.as_str()) {
                    return Err(ProfileError::DuplicateStatusFieldName {
                        measurement: index,
                        status_field: position,
                    });
                }
                seen.push(&field.name);
                let address = field.parse_address(index, position, driver, &options)?;
                tags.push(Tag::new(definition.status_tag_id(field)?, address));
            }
        }
        Ok(tags)
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "testkit")]
    mod loaded {
        use crate::testkit::ConventionDriver;
        use crate::{AddressOptions, ErrorKind, PlcError};

        use crate::sensors::{ProfileError, SensorProfile};

        fn profile_with(convention: Option<&str>) -> SensorProfile {
            let line = convention.map_or_else(String::new, |name| {
                format!("address_convention = {name:?}\n")
            });
            SensorProfile::from_toml(&format!(
                "id = \"synthetic_options\"\nschema_version = 1\nvalidation = \"synthetic\"\n\
             [connection]\nscheme = \"modbus-tcp\"\n{line}\
             [provenance]\nsource = \"Synthetic test profile\"\n"
            ))
            .expect("test document loads")
        }

        // covers: ALPHA_PLAN S29 / convention propagated to the driver
        #[test]
        fn the_declared_convention_becomes_the_address_options() {
            let options = profile_with(Some("plc4x"))
                .address_options()
                .expect("plc4x is a valid name");
            assert_eq!(options.convention(), Some("plc4x"), "declared convention");
            assert_eq!(
                options,
                AddressOptions::builder()
                    .convention("plc4x")
                    .build()
                    .expect("valid"),
                "equal to the builder's result"
            );
        }

        #[test]
        fn no_declared_convention_gives_the_default_options() {
            let options = profile_with(None)
                .address_options()
                .expect("the default is valid");
            assert_eq!(options, AddressOptions::default(), "default options");
            assert_eq!(options.convention(), None, "no convention");
        }

        #[test]
        fn a_convention_name_core_refuses_is_an_unknown_convention() {
            for name in ["", "plc 4x", "plc4x\n"] {
                assert_eq!(
                    profile_with(Some(name)).address_options().err(),
                    Some(ProfileError::UnknownAddressConvention),
                    "{name:?}"
                );
            }
            let error = profile_with(Some("plc 4x"))
                .address_options()
                .expect_err("refused");
            assert_eq!(
                PlcError::from(error).kind(),
                ErrorKind::InvalidConfig,
                "converts to InvalidConfig"
            );
        }

        // covers: ALPHA_PLAN S29 / convention propagated to the driver
        #[test]
        fn tags_hand_the_declared_convention_to_every_parse_address_call() {
            let driver = ConventionDriver::new("modbus-tcp");
            let text = "id = \"synthetic_options\"\nschema_version = 1\n\
                validation = \"synthetic\"\n[connection]\nscheme = \"modbus-tcp\"\n\
                address_convention = \"plc4x\"\n\
                [provenance]\nsource = \"Synthetic test profile\"\n\
                [[measurements]]\nname = \"level\"\naddress = \"holding-register:1\"\ndata_type = \"INT\"\n\
                unit = \"m\"\nstatus_fields = [{ name = \"fault\", \
                address = \"holding-register:2\", bit = 0 }]\n";
            let profile = SensorProfile::from_toml(text).expect("loads");
            let tags = profile.tags(&driver).expect("tags build");
            let shown: Vec<String> = tags.iter().map(|tag| tag.address().canonical()).collect();
            assert_eq!(
                shown,
                ["plc4x:holding-register:1", "plc4x:holding-register:2"],
                "measurement and status address both saw the convention"
            );
        }
    }
}
