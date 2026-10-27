//! The measurement pipeline end to end: profiles loaded from TOML, tags built through the
//! simulator driver, responses built by hand (or read from the simulator) and turned into
//! measurements. The simulator is the sanctioned `Driver` double (coding standards 13.5).
#![forbid(unsafe_code)]
#![cfg(feature = "toml")]

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::{Arc, OnceLock};
    use std::time::{Duration, Instant, SystemTime};

    use proptest::prelude::*;
    use stonecrab_core::{
        ByteOrder, DriverRegistry, ErrorKind, ItemError, PlcError, PlcValue, PlcValueType,
        ProtocolStatus, ReadItem, ReadRequest, ReadResponse, ReadResponseBuilder, ResourceLimits,
        Tag, TagId, WordOrder,
    };
    use stonecrab_driver_sim::{SimDevice, SimDriver, SimTag};
    use stonecrab_sensors::{
        Measurement, ProfileError, Quality, ScaleStep, SensorProfile, ValidationStatus,
    };
    use stonecrab_testkit::{FakeClock, block_on_with};

    const HEAD: &str = "id = \"synthetic_pipeline\"\nschema_version = 1\n\
        validation = \"synthetic\"\n[connection]\nscheme = \"sim\"\n\
        [provenance]\nsource = \"Synthetic test profile\"\n";

    /// A scaled temperature with range, raw retention and four status fields.
    const BOILER: &str = r#"
        [[measurements]]
        name = "boiler_temperature"
        address = "STATE/boiler:i16"
        data_type = "INT"
        unit = "Cel"
        scaling = [{ divide = 10.0 }]
        range = { min = -40.0, max = 215.0 }
        keep_raw = true
        status_fields = [
            { name = "open_circuit", address = "STATE/status:u16", bit = 3 },
            { name = "healthy", address = "STATE/healthy:bool", bit = 0, invert = true },
            { name = "drift", address = "STATE/drift:bool", bit = 0 },
            { name = "note", address = "STATE/note:bool", bit = 0 },
        ]
        quality_map = { bad_when = ["open_circuit", "healthy"], uncertain_when = ["drift"] }
    "#;

    const BOILER_ID: &str = "boiler_temperature";

    /// One sampling instant for every item and response, so nothing reads a clock twice.
    fn nth<T>(items: &[T], index: usize) -> &T {
        items.get(index).expect("index in range")
    }

    fn sampled() -> Instant {
        static START: OnceLock<Instant> = OnceLock::new();
        *START.get_or_init(Instant::now)
    }

    fn profile(measurements: &str) -> SensorProfile {
        SensorProfile::from_toml(&format!("{HEAD}{measurements}")).expect("test document loads")
    }

    fn tags_of(profile: &SensorProfile) -> Vec<Tag> {
        profile.tags(&SimDriver::new()).expect("tags build")
    }

    fn ok(id: &str, value: PlcValue) -> ReadItem {
        ReadItem::new(TagId::new(id).expect("id"), Ok(value), sampled())
    }

    fn failed(id: &str, status: Option<ProtocolStatus>) -> ReadItem {
        failed_at(id, status, sampled())
    }

    fn failed_at(id: &str, status: Option<ProtocolStatus>, at: Instant) -> ReadItem {
        let error = ItemError::new(PlcError::new(ErrorKind::Timeout, "no reply"));
        let error = match status {
            Some(status) => error.with_status(status),
            None => error,
        };
        ReadItem::new(TagId::new(id).expect("id"), Err(error), at)
    }

    /// A response holding exactly `items`: the request is the profile's tags that have one.
    fn respond(tags: &[Tag], items: Vec<ReadItem>) -> ReadResponse {
        respond_within(tags, items, sampled(), sampled())
    }

    /// Like [`respond`], with an explicit response window.
    fn respond_within(
        tags: &[Tag],
        items: Vec<ReadItem>,
        start: Instant,
        end: Instant,
    ) -> ReadResponse {
        let limits = ResourceLimits::builder().build().expect("default limits");
        let request = tags
            .iter()
            .filter(|tag| items.iter().any(|item| item.tag() == tag.id()))
            .fold(ReadRequest::builder(limits), |builder, tag| {
                builder.tag(tag.clone())
            })
            .build()
            .expect("request");
        items
            .into_iter()
            .fold(
                ReadResponse::builder(&request, start, end),
                ReadResponseBuilder::item,
            )
            .build()
            .expect("response")
    }

    fn wall() -> SystemTime {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::from_secs(1_700_000_000))
            .expect("in range")
    }

    /// The measurements of `profile` for `items`.
    fn measure(profile: &SensorProfile, items: Vec<ReadItem>) -> Vec<Measurement> {
        let response = respond(&tags_of(profile), items);
        profile.measurements(&response, wall())
    }

    /// The only measurement of a profile with one definition.
    fn only(measurements: &str, id: &str, value: PlcValue) -> Measurement {
        let mut all = measure(&profile(measurements), vec![ok(id, value)]);
        assert_eq!(all.len(), 1, "one definition, one measurement");
        all.remove(0)
    }

    fn float(value: Option<&PlcValue>) -> f64 {
        match value {
            Some(PlcValue::F64(number)) => *number,
            other => panic!("expected an F64, got {other:?}"),
        }
    }

    fn close(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() <= 1e-9 * expected.abs().max(1.0)
    }

    /// The boiler with the given status words: `open_circuit` word, `healthy`, `drift`.
    fn boiler(value: i16, word: PlcValue, healthy: bool, drift: bool) -> Measurement {
        let items = vec![
            ok(BOILER_ID, PlcValue::I16(value)),
            ok("boiler_temperature.open_circuit", word),
            ok("boiler_temperature.healthy", PlcValue::Bool(healthy)),
            ok("boiler_temperature.drift", PlcValue::Bool(drift)),
            ok("boiler_temperature.note", PlcValue::Bool(true)),
        ];
        let mut all = measure(&profile(BOILER), items);
        assert_eq!(all.len(), 1, "one definition, one measurement");
        all.remove(0)
    }

    // ---- tags ---------------------------------------------------------------------------

    // covers: ALPHA_PLAN S29 / tags naming, hints and byte and word order
    #[test]
    fn tags_carry_names_driver_hints_and_orders() {
        let text = r#"
            [[measurements]]
            name = "pressure"
            address = "STATE/pressure:f32"
            data_type = "REAL"
            unit = "kPa"
            byte_order = "little_endian"
            word_order = "low_word_first"
            status_fields = [
                { name = "fault", address = "STATE/fault:bool", bit = 0 },
                { name = "word", address = "STATE/word:u16", bit = 2 },
            ]

            [[measurements]]
            name = "level"
            address = "STATE/level:u16"
            data_type = "UINT"
            unit = "mm"
        "#;
        let tags = tags_of(&profile(text));
        let ids: Vec<&str> = tags.iter().map(|tag| tag.id().as_str()).collect();
        assert_eq!(
            ids,
            ["pressure", "pressure.fault", "pressure.word", "level"],
            "measurement first, its status fields next, in document order"
        );
        let pressure = &nth(&tags, 0);
        assert_eq!(
            pressure.value_type(),
            Some(PlcValueType::F32),
            "driver hint"
        );
        assert_eq!(
            pressure.byte_order(),
            Some(ByteOrder::LittleEndian),
            "byte order"
        );
        assert_eq!(
            pressure.word_order(),
            Some(WordOrder::LowWordFirst),
            "word order"
        );
        assert_eq!(
            pressure.address().canonical(),
            "STATE/pressure:f32",
            "address"
        );
        assert_eq!(
            nth(&tags, 1).value_type(),
            Some(PlcValueType::Bool),
            "status hint is the driver's"
        );
        assert_eq!(
            nth(&tags, 2).value_type(),
            Some(PlcValueType::U16),
            "a status word keeps its type"
        );
        assert_eq!(
            nth(&tags, 1).byte_order(),
            None,
            "a status tag has no byte order"
        );
        assert_eq!(
            nth(&tags, 3).value_type(),
            Some(PlcValueType::U16),
            "level hint"
        );
        assert_eq!(
            nth(&tags, 3).byte_order(),
            None,
            "an unset order stays unset"
        );
        assert_eq!(
            nth(&tags, 3).word_order(),
            None,
            "an unset order stays unset"
        );
        assert_eq!(nth(&tags, 0).count().get(), 1, "count is one");
    }

    #[test]
    fn a_profile_without_measurements_has_no_tags() {
        assert!(tags_of(&profile("")).is_empty(), "no measurements, no tags");
    }

    // covers: ALPHA_PLAN S29 / scheme mismatch is a typed error
    #[test]
    fn a_driver_of_another_scheme_is_a_mismatch() {
        let text = format!(
            "id = \"synthetic_other\"\nschema_version = 1\nvalidation = \"synthetic\"\n\
             [connection]\nscheme = \"modbus-tcp\"\naddress_convention = \"plc4x\"\n\
             [provenance]\nsource = \"Synthetic test profile\"\n{BOILER}"
        );
        let profile = SensorProfile::from_toml(&text).expect("loads");
        let error = profile
            .tags(&SimDriver::new())
            .expect_err("sim is not modbus-tcp");
        assert_eq!(
            error,
            ProfileError::SchemeMismatch { driver: "sim" },
            "typed error"
        );
        assert_eq!(
            PlcError::from(error).kind(),
            ErrorKind::InvalidConfig,
            "converts to InvalidConfig"
        );
        assert!(
            !error.to_string().contains("modbus"),
            "the profile's text is not echoed"
        );
    }

    #[test]
    fn an_invalid_profile_returns_its_first_validation_error() {
        let text = r#"
            [[measurements]]
            name = "a"
            address = "STATE/a:i16"
            data_type = "INT"
            unit = "furlongs"

            [[measurements]]
            name = "b"
            address = "STATE/b:i16"
            data_type = "INT"
            unit = ""
        "#;
        let profile = profile(text);
        assert_eq!(
            profile.tags(&SimDriver::new()).err(),
            Some(ProfileError::UnknownUnit { measurement: 0 }),
            "the first of two errors"
        );
        assert_eq!(
            profile.validate().map_err(|errors| errors.len()),
            Err(2),
            "both exist"
        );
    }

    // covers: ALPHA_PLAN S29 / a parse failure names the measurement and never echoes the address
    #[test]
    fn a_rejected_address_names_the_measurement_and_never_echoes_the_text() {
        let text = r#"
            [[measurements]]
            name = "fine"
            address = "STATE/fine:i16"
            data_type = "INT"
            unit = "1"

            [[measurements]]
            name = "broken"
            address = "canary-address-text"
            data_type = "INT"
            unit = "1"
        "#;
        let error = profile(text).tags(&SimDriver::new()).expect_err("rejected");
        assert_eq!(
            error,
            ProfileError::AddressRejected { measurement: 1 },
            "index 1"
        );
        for shown in [
            error.to_string(),
            format!("{error:?}"),
            PlcError::from(error).to_string(),
        ] {
            assert!(!shown.contains("canary"), "no address text in {shown}");
        }
    }

    #[test]
    fn a_rejected_status_address_names_the_status_field() {
        let text = r#"
            [[measurements]]
            name = "pump"
            address = "STATE/pump:i16"
            data_type = "INT"
            unit = "1"
            status_fields = [
                { name = "fine", address = "STATE/fine:bool", bit = 0 },
                { name = "broken", address = "canary-status-text", bit = 0 },
            ]
        "#;
        let error = profile(text).tags(&SimDriver::new()).expect_err("rejected");
        assert_eq!(
            error,
            ProfileError::StatusAddressRejected {
                measurement: 0,
                status_field: 1
            },
            "indices"
        );
        assert!(!error.to_string().contains("canary"), "no address text");
    }

    #[test]
    fn two_status_fields_with_one_name_cannot_become_tags() {
        let text = r#"
            [[measurements]]
            name = "pump"
            address = "STATE/pump:i16"
            data_type = "INT"
            unit = "1"
            status_fields = [
                { name = "fault", address = "STATE/a:bool", bit = 0 },
                { name = "fault", address = "STATE/b:bool", bit = 0 },
            ]
        "#;
        let profile = profile(text);
        assert_eq!(profile.validate(), Ok(()), "validation accepts duplicates");
        assert_eq!(
            profile.tags(&SimDriver::new()).err(),
            Some(ProfileError::DuplicateStatusFieldName {
                measurement: 0,
                status_field: 1
            }),
            "tag building cannot"
        );
    }

    #[test]
    fn status_tag_ids_join_the_measurement_and_the_field_with_a_dot() {
        let profile = profile(BOILER);
        let definition = profile.measurements.first().expect("one measurement");
        let field = definition.status_fields.first().expect("a status field");
        let id = definition.status_tag_id(field).expect("valid");
        assert_eq!(
            id.as_str(),
            "boiler_temperature.open_circuit",
            "naming rule"
        );
        assert_ne!(id, definition.name, "never the measurement's own tag");
    }

    // ---- scaling, F7 --------------------------------------------------------------------

    const NEG: &str = r#"
        [[measurements]]
        name = "cold_room"
        address = "STATE/cold:i16"
        data_type = "INT"
        unit = "Cel"
        scaling = [{ multiply = 0.1 }]
        keep_raw = true
    "#;

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn a_negative_scaled_value_keeps_its_sign_and_its_raw_value() {
        let measurement = only(NEG, "cold_room", PlcValue::I16(-2424));
        assert!(
            close(float(measurement.value.as_ref()), -242.4),
            "{:?}",
            measurement.value
        );
        assert_eq!(
            measurement.raw,
            Some(PlcValue::I16(-2424)),
            "raw is the unscaled typed value"
        );
        assert_eq!(measurement.quality, Quality::Good, "no range, no status");
        assert_eq!(measurement.unit, "Cel", "unit");
        assert_eq!(measurement.name, "cold_room", "name");
        assert_eq!(measurement.tag.as_str(), "cold_room", "tag");
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn nan_and_infinity_from_a_real_are_bad() {
        let text = r#"
            [[measurements]]
            name = "flow"
            address = "STATE/flow:f32"
            data_type = "REAL"
            byte_order = "big_endian"
            word_order = "high_word_first"
            unit = "L/min"
            scaling = [{ multiply = 2.0 }]
            keep_raw = true
        "#;
        for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let measurement = only(text, "flow", PlcValue::F32(bad));
            assert_eq!(measurement.quality, Quality::Bad, "{bad}");
            assert_eq!(measurement.value, None, "no value for {bad}");
            assert!(
                matches!(measurement.raw, Some(PlcValue::F32(raw)) if raw.to_bits() == bad.to_bits()),
                "the raw value is kept for diagnosis"
            );
        }
        let good = only(text, "flow", PlcValue::F32(1.5));
        assert_eq!(good.quality, Quality::Good, "a finite REAL");
        assert!(close(float(good.value.as_ref()), 3.0), "{:?}", good.value);
    }

    // covers: SPEC mandatory-failure-cases / F7 negative and scaled values, NaN and infinity, counter rollover
    #[test]
    fn a_counter_rolls_over_through_a_sixteen_bit_field() {
        let text = r#"
            [[measurements]]
            name = "pulses"
            address = "STATE/pulses:u32"
            data_type = "UDINT"
            byte_order = "big_endian"
            word_order = "high_word_first"
            unit = "1"
            scaling = [{ bit_field = { shift = 0, width = 16 } }]
        "#;
        let before = only(text, "pulses", PlcValue::U32(0x0001_FFFF));
        assert_eq!(
            before.value,
            Some(PlcValue::U32(0xFFFF)),
            "last count of the field"
        );
        let after = only(text, "pulses", PlcValue::U32(0x0002_0000));
        assert_eq!(after.value, Some(PlcValue::U32(0)), "wrapped to zero");
        assert_eq!(after.quality, Quality::Good, "a rollover is not a fault");
        assert_eq!(after.raw, None, "keep_raw is off");
    }

    #[test]
    fn integers_stay_integers_without_a_float_step() {
        let text = r#"
            [[measurements]]
            name = "strokes"
            address = "STATE/strokes:u16"
            data_type = "UINT"
            unit = "1"
            scaling = [{ multiply = 1.0 }]

            [[measurements]]
            name = "trim"
            address = "STATE/trim:i16"
            data_type = "INT"
            unit = "1"
            scaling = [{ offset = -40.0 }, { clamp = { min = -100.0, max = 100.0 } }]
        "#;
        let profile = profile(text);
        let items = vec![
            ok("strokes", PlcValue::U16(65_535)),
            ok("trim", PlcValue::I16(-80)),
        ];
        let all = measure(&profile, items);
        assert_eq!(all.len(), 2, "two definitions");
        assert_eq!(
            nth(&all, 0).value,
            Some(PlcValue::U16(65_535)),
            "UINT times 1.0 is still U16"
        );
        assert_eq!(
            nth(&all, 1).value,
            Some(PlcValue::I16(-100)),
            "INT offset then clamp is still I16"
        );
    }

    #[test]
    fn a_divide_that_overflows_is_bad_without_a_value() {
        let text = r#"
            [[measurements]]
            name = "huge"
            address = "STATE/huge:f64"
            data_type = "LREAL"
            byte_order = "big_endian"
            word_order = "high_word_first"
            unit = "1"
            scaling = [{ divide = 1e-300 }]
        "#;
        let measurement = only(text, "huge", PlcValue::F64(1e300));
        assert_eq!(
            (measurement.value, measurement.quality),
            (None, Quality::Bad),
            "overflow"
        );
        let fine = only(text, "huge", PlcValue::F64(1e-10));
        assert_eq!(fine.quality, Quality::Good, "a quotient that fits");
    }

    #[test]
    fn a_zero_divisor_is_refused_by_validation_and_bad_if_it_gets_through() {
        let text = r#"
            [[measurements]]
            name = "zero"
            address = "STATE/zero:i16"
            data_type = "INT"
            unit = "1"
            scaling = [{ divide = 0.0 }]
        "#;
        let profile = profile(text);
        assert_eq!(
            profile.validate(),
            Err(vec![ProfileError::InvalidScaleStep {
                measurement: 0,
                step: 0
            }]),
            "validation catches it"
        );
        // The pipeline does not rely on validation: build the response by hand.
        let tags = vec![Tag::new(
            TagId::new("zero").expect("id"),
            Arc::new(stonecrab_driver_sim::SimAddress::parse("STATE/zero:i16").expect("address")),
        )];
        let response = respond(&tags, vec![ok("zero", PlcValue::I16(5))]);
        let all = profile.measurements(&response, wall());
        assert_eq!(nth(&all, 0).quality, Quality::Bad, "bad, not a panic");
        assert_eq!(nth(&all, 0).value, None, "no value");
    }

    #[test]
    fn a_discrete_value_passes_through_and_cannot_be_scaled() {
        let plain = r#"
            [[measurements]]
            name = "running"
            address = "STATE/running:bool"
            data_type = "BOOL"
            unit = "1"
            range = { min = 0.0, max = 1.0 }
        "#;
        let measurement = only(plain, "running", PlcValue::Bool(true));
        assert_eq!(measurement.value, Some(PlcValue::Bool(true)), "unchanged");
        assert_eq!(
            measurement.quality,
            Quality::Good,
            "a range does not apply to a bool"
        );
        let scaled = plain.replace("range =", "scaling = [{ multiply = 2.0 }]\nrange =");
        let measurement = only(&scaled, "running", PlcValue::Bool(true));
        assert_eq!(measurement.quality, Quality::Bad, "a bool has no scale");
    }

    // ---- quality ------------------------------------------------------------------------

    #[test]
    fn a_clean_reading_is_good() {
        let measurement = boiler(-250, PlcValue::U16(0), true, false);
        assert_eq!(measurement.quality, Quality::Good, "quality");
        assert!(
            close(float(measurement.value.as_ref()), -25.0),
            "{:?}",
            measurement.value
        );
        assert_eq!(measurement.raw, Some(PlcValue::I16(-250)), "raw kept");
        assert_eq!(
            measurement.origin,
            ValidationStatus::Synthetic,
            "origin is the profile's"
        );
    }

    #[test]
    fn a_set_status_bit_in_bad_when_makes_the_value_bad_but_keeps_it() {
        let measurement = boiler(-250, PlcValue::U16(0b1000), true, false);
        assert_eq!(measurement.quality, Quality::Bad, "bit 3 set");
        assert!(
            close(float(measurement.value.as_ref()), -25.0),
            "the value is kept"
        );
        let other_bit = boiler(-250, PlcValue::U16(0b0100), true, false);
        assert_eq!(other_bit.quality, Quality::Good, "bit 2 is not the field");
    }

    // covers: ALPHA_PLAN S29 / bool inversion through invert
    #[test]
    fn invert_flips_the_status_bit() {
        let healthy = boiler(-250, PlcValue::U16(0), true, false);
        assert_eq!(
            healthy.quality,
            Quality::Good,
            "wire true, inverted to clear"
        );
        let unhealthy = boiler(-250, PlcValue::U16(0), false, false);
        assert_eq!(
            unhealthy.quality,
            Quality::Bad,
            "wire false, inverted to set"
        );
    }

    #[test]
    fn a_set_status_bit_in_uncertain_when_makes_the_value_uncertain() {
        let measurement = boiler(-250, PlcValue::U16(0), true, true);
        assert_eq!(measurement.quality, Quality::Uncertain, "drift set");
    }

    #[test]
    fn a_range_violation_is_uncertain_and_the_bounds_are_inclusive() {
        let cases = [
            (2150, Quality::Good, "the maximum"),
            (2151, Quality::Uncertain, "above the maximum"),
            (-400, Quality::Good, "the minimum"),
            (-401, Quality::Uncertain, "below the minimum"),
        ];
        for (raw, quality, what) in cases {
            let measurement = boiler(raw, PlcValue::U16(0), true, false);
            assert_eq!(measurement.quality, quality, "{what}");
            assert!(
                measurement.value.is_some(),
                "an out-of-range value is still reported"
            );
        }
        let text = r#"
            [[measurements]]
            name = "level"
            address = "STATE/level:u16"
            data_type = "UINT"
            unit = "mm"
            range = { min = 0.0, max = 500.0 }
        "#;
        assert_eq!(
            only(text, "level", PlcValue::U16(501)).quality,
            Quality::Uncertain,
            "integer"
        );
        assert_eq!(
            only(text, "level", PlcValue::U16(500)).quality,
            Quality::Good,
            "integer edge"
        );
    }

    #[test]
    fn bad_beats_uncertain_beats_unknown_beats_good() {
        let name = "boiler_temperature.";
        let id = |field: &str| format!("{name}{field}");
        let run = |value: i16, items: Vec<ReadItem>| {
            let mut all = vec![ok(BOILER_ID, PlcValue::I16(value))];
            all.extend(items);
            measure(&profile(BOILER), all).remove(0).quality
        };
        let word = |bits: u16| ok(&id("open_circuit"), PlcValue::U16(bits));
        let healthy = || ok(&id("healthy"), PlcValue::Bool(true));
        let drift = |on: bool| ok(&id("drift"), PlcValue::Bool(on));

        assert_eq!(
            run(5000, vec![word(8), healthy(), drift(false)]),
            Quality::Bad,
            "bad status over a range violation"
        );
        assert_eq!(
            run(5000, vec![word(0), healthy(), drift(true)]),
            Quality::Uncertain,
            "uncertain status and a range violation"
        );
        assert_eq!(
            run(0, vec![word(8), healthy(), drift(true)]),
            Quality::Bad,
            "bad over uncertain"
        );
        assert_eq!(
            run(
                0,
                vec![failed(&id("open_circuit"), None), healthy(), drift(true)]
            ),
            Quality::Uncertain,
            "uncertain over unknown"
        );
        assert_eq!(
            run(
                0,
                vec![failed(&id("open_circuit"), None), healthy(), drift(false)]
            ),
            Quality::Unknown,
            "an errored status item is unknown"
        );
        assert_eq!(
            run(0, vec![word(8), failed(&id("healthy"), None), drift(false)]),
            Quality::Bad,
            "bad over unknown"
        );
        assert_eq!(
            run(
                5000,
                vec![failed(&id("open_circuit"), None), healthy(), drift(false)]
            ),
            Quality::Uncertain,
            "a range violation over unknown"
        );
    }

    #[test]
    fn a_missing_or_unreadable_status_item_is_unknown() {
        assert_eq!(
            measure(&profile(BOILER), vec![ok(BOILER_ID, PlcValue::I16(0))])
                .remove(0)
                .quality,
            Quality::Unknown,
            "the status items are missing from the response"
        );
        let wrong_type = vec![
            ok(BOILER_ID, PlcValue::I16(0)),
            ok("boiler_temperature.open_circuit", PlcValue::F32(1.0)),
            ok("boiler_temperature.healthy", PlcValue::Bool(true)),
            ok("boiler_temperature.drift", PlcValue::Bool(false)),
        ];
        assert_eq!(
            measure(&profile(BOILER), wrong_type).remove(0).quality,
            Quality::Unknown,
            "a float is not a status word"
        );
        let beyond_the_word = vec![
            ok(BOILER_ID, PlcValue::I16(0)),
            ok("boiler_temperature.open_circuit", PlcValue::U16(0)),
            ok("boiler_temperature.healthy", PlcValue::Bool(true)),
            ok("boiler_temperature.drift", PlcValue::Bool(false)),
        ];
        let text = BOILER.replace("bit = 3", "bit = 64");
        assert_eq!(
            measure(&profile(&text), beyond_the_word).remove(0).quality,
            Quality::Unknown,
            "bit 64 does not exist"
        );
    }

    #[test]
    fn status_fields_no_list_names_and_a_missing_quality_map_have_no_effect() {
        let note_only = BOILER.replace(
            r#"quality_map = { bad_when = ["open_circuit", "healthy"], uncertain_when = ["drift"] }"#,
            r#"quality_map = { bad_when = ["note"] }"#,
        );
        let items = vec![
            ok(BOILER_ID, PlcValue::I16(0)),
            ok("boiler_temperature.note", PlcValue::Bool(false)),
        ];
        assert_eq!(
            measure(&profile(&note_only), items).remove(0).quality,
            Quality::Good,
            "only the named field counts, and it is clear"
        );
        let no_map = BOILER.replace(
            r#"quality_map = { bad_when = ["open_circuit", "healthy"], uncertain_when = ["drift"] }"#,
            "",
        );
        let items = vec![
            ok(BOILER_ID, PlcValue::I16(0)),
            ok("boiler_temperature.open_circuit", PlcValue::U16(8)),
        ];
        assert_eq!(
            measure(&profile(&no_map), items).remove(0).quality,
            Quality::Good,
            "no mapping, no status rules"
        );
        let both = BOILER.replace(
            r#"quality_map = { bad_when = ["open_circuit", "healthy"], uncertain_when = ["drift"] }"#,
            r#"quality_map = { bad_when = ["drift"], uncertain_when = ["drift"] }"#,
        );
        let items = vec![
            ok(BOILER_ID, PlcValue::I16(0)),
            ok("boiler_temperature.drift", PlcValue::Bool(true)),
        ];
        assert_eq!(
            measure(&profile(&both), items).remove(0).quality,
            Quality::Bad,
            "a name in both lists is bad"
        );
    }

    // ---- item errors, timestamps, status ------------------------------------------------

    // covers: SPEC mandatory-failure-cases / F7 item error with protocol status retained
    #[test]
    fn an_item_error_is_bad_and_keeps_the_protocol_status() {
        let status = ProtocolStatus::new("modbus", 2).with_name("illegal data address");
        let item_instant = sampled()
            .checked_add(Duration::from_secs(5))
            .expect("in range");
        let end = sampled()
            .checked_add(Duration::from_secs(10))
            .expect("in range");
        let profile_under_test = profile(BOILER);
        let response = respond_within(
            &tags_of(&profile_under_test),
            vec![failed_at(BOILER_ID, Some(status.clone()), item_instant)],
            sampled(),
            end,
        );
        let measurement = profile_under_test.measurements(&response, wall()).remove(0);
        assert_eq!(measurement.quality, Quality::Bad, "quality");
        assert_eq!(measurement.value, None, "value");
        assert_eq!(
            measurement.raw, None,
            "no raw value for a failed read, even with keep_raw"
        );
        assert_eq!(measurement.protocol_status, Some(status), "status retained");
        assert_eq!(
            measurement.received_mono, item_instant,
            "the item's own instant, not the response window's"
        );
        assert_eq!(measurement.received_at, wall(), "wall time");

        let bare = measure(&profile(BOILER), vec![failed(BOILER_ID, None)]).remove(0);
        assert_eq!(
            (bare.quality, bare.protocol_status),
            (Quality::Bad, None),
            "no status"
        );
    }

    #[test]
    fn a_status_on_a_successful_item_is_retained_too() {
        let status = ProtocolStatus::new("modbus", 0);
        let item = ok(BOILER_ID, PlcValue::I16(1)).with_status(status.clone());
        let measurement = measure(&profile(BOILER), vec![item]).remove(0);
        assert_eq!(measurement.protocol_status, Some(status), "success status");
    }

    // covers: SPEC mandatory-failure-cases / F6 timestamp absence
    #[test]
    fn an_absent_source_time_stays_absent_and_a_present_one_is_not_replaced() {
        let none = measure(&profile(BOILER), vec![ok(BOILER_ID, PlcValue::I16(1))]).remove(0);
        assert_eq!(
            none.source_time, None,
            "never substituted from the wall or monotonic clock"
        );
        assert_eq!(none.received_at, wall(), "received_at is wall_now");
        let stamped = SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
        let item = ok(BOILER_ID, PlcValue::I16(1)).with_source_time(stamped);
        let some = measure(&profile(BOILER), vec![item]).remove(0);
        assert_eq!(
            some.source_time,
            Some(stamped),
            "the device's stamp is passed through"
        );
        assert_ne!(
            some.source_time,
            Some(some.received_at),
            "and is not wall_now"
        );
        let errored = measure(&profile(BOILER), vec![failed(BOILER_ID, None)]).remove(0);
        assert_eq!(errored.source_time, None, "absent on an error too");
    }

    #[test]
    fn a_definition_the_response_does_not_cover_is_skipped() {
        let both = format!("{NEG}{BOILER}");
        let all = measure(&profile(&both), vec![ok("cold_room", PlcValue::I16(1))]);
        assert_eq!(all.len(), 1, "one of two definitions is covered");
        assert_eq!(nth(&all, 0).name, "cold_room", "the covered one");
    }

    #[test]
    fn origin_is_the_profiles_validation_status() {
        let text = format!("{HEAD}{NEG}").replace("synthetic", "fixture_backed");
        let profile = SensorProfile::from_toml(&text).expect("loads");
        let measurement = measure(&profile, vec![ok("cold_room", PlcValue::I16(1))]).remove(0);
        assert_eq!(
            measurement.origin,
            ValidationStatus::FixtureBacked,
            "origin"
        );
    }

    #[test]
    fn debug_shows_the_status_size_and_not_its_bytes() {
        let status = ProtocolStatus::new("modbus", 3)
            .with_raw(
                &[0xDE, 0xAD, 0xBE, 0xEF],
                NonZeroUsize::new(8).expect("non-zero"),
            )
            .expect("within budget");
        let item = ok("cold_room", PlcValue::I16(1)).with_status(status);
        let measurement = measure(&profile(NEG), vec![item]).remove(0);
        let shown = format!("{measurement:?}");
        assert!(shown.contains("raw_bytes: Some(4)"), "{shown}");
        // Only the status is inspected for its bytes: the clocks in the measurement are
        // real values whose digits may contain any byte's decimal rendering by chance.
        let status = format!("{:?}", measurement.protocol_status);
        for byte in ["222", "173", "190", "239", "DE", "AD", "BE", "EF"] {
            assert!(!status.contains(byte), "no byte of the status: {status}");
        }
    }

    // ---- the simulator end to end -------------------------------------------------------

    #[test]
    fn a_read_from_the_simulator_becomes_measurements() {
        let device = SimDevice::new();
        device
            .set("boiler", SimTag::new(PlcValue::I16(-250)))
            .expect("boiler");
        device
            .set("status", SimTag::new(PlcValue::U16(0b1000)))
            .expect("status");
        device
            .set("healthy", SimTag::new(PlcValue::Bool(true)))
            .expect("healthy");
        device
            .set("drift", SimTag::new(PlcValue::Bool(false)))
            .expect("drift");
        device
            .set("note", SimTag::new(PlcValue::Bool(false)))
            .expect("note");
        let clock = Arc::new(FakeClock::default());
        let driver = Arc::new(
            SimDriver::new()
                .with_clock(Arc::<FakeClock>::clone(&clock))
                .with_device("bench", device)
                .expect("device"),
        );
        let mut registry = DriverRegistry::new();
        registry
            .register(Arc::<SimDriver>::clone(&driver))
            .expect("register");

        let profile = profile(BOILER);
        let tags = profile.tags(&*driver).expect("tags");
        let connection = block_on_with(&clock, registry.connect("sim://bench")).expect("connects");
        let request = tags
            .iter()
            .fold(ReadRequest::builder(connection.limits()), |builder, tag| {
                builder.tag(tag.clone())
            })
            .build()
            .expect("request");
        let response = block_on_with(&clock, connection.read(request)).expect("read");

        let all = profile.measurements(&response, wall());
        assert_eq!(all.len(), 1, "one measurement");
        let measurement = &nth(&all, 0);
        assert!(
            close(float(measurement.value.as_ref()), -25.0),
            "{:?}",
            measurement.value
        );
        assert_eq!(
            measurement.quality,
            Quality::Bad,
            "the status word has bit 3 set"
        );
        assert_eq!(
            measurement.source_time, None,
            "the simulator has no device clock"
        );
        assert_eq!(measurement.received_at, wall(), "received_at is wall_now");
        assert_eq!(measurement.raw, Some(PlcValue::I16(-250)), "raw");
    }

    // ---- properties ---------------------------------------------------------------------

    fn constant() -> impl Strategy<Value = f64> {
        prop_oneof![
            prop::num::f64::ANY,
            -1000.0..1000.0_f64,
            Just(0.0),
            Just(1.0),
            Just(-1.0),
            Just(0.1),
            Just(f64::MAX),
            Just(1e19),
        ]
    }

    fn step() -> impl Strategy<Value = ScaleStep> {
        prop_oneof![
            constant().prop_map(ScaleStep::Multiply),
            constant().prop_map(ScaleStep::Divide),
            constant().prop_map(ScaleStep::Offset),
            (constant(), constant()).prop_map(|(min, max)| ScaleStep::Clamp { min, max }),
            (0_u8..70, 0_u8..70).prop_map(|(shift, width)| ScaleStep::BitField { shift, width }),
        ]
    }

    fn value() -> impl Strategy<Value = PlcValue> {
        prop_oneof![
            any::<i8>().prop_map(PlcValue::I8),
            any::<i16>().prop_map(PlcValue::I16),
            any::<i32>().prop_map(PlcValue::I32),
            any::<i64>().prop_map(PlcValue::I64),
            any::<u8>().prop_map(PlcValue::U8),
            any::<u16>().prop_map(PlcValue::U16),
            any::<u32>().prop_map(PlcValue::U32),
            any::<u64>().prop_map(PlcValue::U64),
            prop::num::f32::ANY.prop_map(PlcValue::F32),
            prop::num::f64::ANY.prop_map(PlcValue::F64),
            any::<bool>().prop_map(PlcValue::Bool),
            ".{0,8}".prop_map(PlcValue::String),
            Just(PlcValue::Null),
        ]
    }

    const PLAIN: &str = r#"
        [[measurements]]
        name = "subject"
        address = "STATE/subject:f64"
        data_type = "LREAL"
        byte_order = "big_endian"
        word_order = "high_word_first"
        unit = "1"
    "#;

    /// The pipeline over `steps` for `input`, with the steps set on a loaded profile.
    fn run_steps(steps: Vec<ScaleStep>, input: PlcValue) -> Measurement {
        let mut profile = profile(PLAIN);
        // Tags come from the valid profile: random steps are not valid, and the pipeline
        // must cope with them anyway.
        let response = respond(&tags_of(&profile), vec![ok("subject", input)]);
        profile.measurements.first_mut().expect("one").scaling = steps;
        profile.measurements(&response, wall()).remove(0)
    }

    proptest! {
        // covers: SPEC mandatory-failure-cases / F7 no value, step or constant panics the pipeline
        #[test]
        fn random_steps_and_values_never_panic_and_never_report_a_non_finite_number(
            steps in prop::collection::vec(step(), 0..5),
            input in value(),
        ) {
            let had_steps = !steps.is_empty();
            let measurement = run_steps(steps, input.clone());
            let finite_input = !matches!(&input, PlcValue::F64(number) if !number.is_finite())
                && !matches!(&input, PlcValue::F32(number) if !number.is_finite());
            match &measurement.value {
                None => prop_assert_eq!(measurement.quality, Quality::Bad, "no value means bad"),
                Some(value) if !had_steps && finite_input => {
                    prop_assert_eq!(value, &input, "no steps, no change");
                }
                Some(PlcValue::F64(number)) => prop_assert!(number.is_finite(), "{number}"),
                Some(PlcValue::F32(number)) => prop_assert!(number.is_finite(), "{number}"),
                Some(_) => {}
            }
            prop_assert!(
                matches!(measurement.quality, Quality::Good | Quality::Bad),
                "no range or status rules here"
            );
            prop_assert_eq!(
                measurement.value.is_some(),
                measurement.quality == Quality::Good,
                "good exactly when a value exists"
            );
        }

        // covers: SPEC mandatory-failure-cases / F7 scaling is reversible within an epsilon
        #[test]
        fn multiply_then_divide_by_one_constant_restores_a_float(
            number in -1.0e6..1.0e6_f64,
            magnitude in 1.0e-3..1.0e3_f64,
            negative in any::<bool>(),
        ) {
            let constant = if negative { -magnitude } else { magnitude };
            let measurement = run_steps(
                vec![ScaleStep::Multiply(constant), ScaleStep::Divide(constant)],
                PlcValue::F64(number),
            );
            let restored = float(measurement.value.as_ref());
            prop_assert!((restored - number).abs() <= 1e-9 * number.abs().max(1.0), "{restored} vs {number}");
        }

        // covers: SPEC mandatory-failure-cases / F7 integer scaling is exact
        #[test]
        fn multiply_then_divide_by_an_integral_constant_is_exact_for_integers(
            number in any::<i32>(),
            constant in (-1000_i16..=1000).prop_filter("non-zero", |c| *c != 0),
        ) {
            let constant = f64::from(constant);
            let product = run_steps(vec![ScaleStep::Multiply(constant)], PlcValue::I32(number));
            let expected = i64::from(number)
                .checked_mul(constant_as_i64(constant))
                .expect("fits i64");
            prop_assert_eq!(product.value.and_then(|v| v.as_i64()), Some(expected), "stays an integer");
            let restored = run_steps(
                vec![ScaleStep::Multiply(constant), ScaleStep::Divide(constant)],
                PlcValue::I32(number),
            );
            prop_assert_eq!(float(restored.value.as_ref()).to_bits(), f64::from(number).to_bits(), "exact");
        }

        // covers: SPEC mandatory-failure-cases / F7 counter rollover through a bit field
        #[test]
        fn a_bit_field_matches_the_shift_and_mask_definition(
            number in any::<u64>(),
            shift in 0_u8..64,
            width in 1_u8..=64,
        ) {
            let measurement = run_steps(vec![ScaleStep::BitField { shift, width }], PlcValue::U64(number));
            let ends_at = u16::from(shift)
                .checked_add(u16::from(width))
                .expect("small");
            if ends_at > 64 {
                prop_assert_eq!(measurement.value, None, "past bit 64");
            } else {
                let mask = 1_u128
                    .checked_shl(u32::from(width))
                    .and_then(|bit| bit.checked_sub(1))
                    .expect("width is at most 64");
                let field = u128::from(number).checked_shr(u32::from(shift)).expect("shift is below 128") & mask;
                prop_assert_eq!(
                    measurement.value.and_then(|v| v.as_u64()).map(u128::from),
                    Some(field),
                    "field"
                );
            }
        }
    }

    /// An integral `f64` constant as `i64`, through the exact decimal text of the constant.
    fn constant_as_i64(constant: f64) -> i64 {
        format!("{constant:.0}")
            .parse()
            .expect("an integral constant prints as an integer")
    }
}
