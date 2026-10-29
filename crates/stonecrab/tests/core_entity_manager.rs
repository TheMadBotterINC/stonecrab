//! Object PLC Mapping (ADR-0033, docs/parity/tools.md "Object PLC Mapping (OPM)"): the
//! `plc_entity!` macro and the `FieldValue` conversions, and `EntityManager` reading,
//! writing, connecting and merging entities through the connection cache, against the
//! simulator and against Modbus TCP over the fake transport.
//!
//! All timing is on the fake clock. The simulator's failure script doubles as a request
//! counter: each read or write consumes one step, so two harmless `Delay(0)` steps of which
//! one is left show that exactly one request was sent.
#![cfg(all(feature = "sim", feature = "modbus", feature = "testkit"))]
#![forbid(unsafe_code)]

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    use stonecrab::modbus::ModbusTcpDriver;
    use stonecrab::sim::{FailureStep, SimDevice, SimDriver, SimTag};
    use stonecrab::testkit::{
        FakeClock, FakeTransportFactory, Script, Step, block_on_with, poll_once,
    };
    use stonecrab::{
        Clock, ConnectionCache, DriverRegistry, EntityField, EntityManager, ErrorKind, FieldValue,
        Operation, PlcDate, PlcDateTime, PlcEntity, PlcError, PlcSignedDuration, PlcTimeOfDay,
        PlcValue, Result, TagId, WriteOutcome, plc_entity,
    };

    const BENCH: &str = "sim://bench";

    fn ms(count: u64) -> Duration {
        Duration::from_millis(count)
    }

    fn secs(count: u64) -> Duration {
        Duration::from_secs(count)
    }

    fn error<T>(result: Result<T>) -> PlcError {
        result.err().expect("an error")
    }

    plc_entity! {
        /// A tank: the level read at every refresh, the setpoint fresh for 500 ms, and a
        /// label that is not mapped.
        #[derive(Debug, Default, Clone, PartialEq)]
        struct Tank {
            level: u16 => "STATE/level:u16",
            setpoint: i16 => "STATE/setpoint:i16" (cache = Duration::from_millis(500)),
            label: String,
        }
    }

    plc_entity! {
        /// Only fields with a cache duration.
        #[derive(Debug, Default)]
        struct Setpoint {
            setpoint: i16 => "STATE/setpoint:i16" (cache = Duration::from_millis(500)),
        }
    }

    plc_entity! {
        /// One field of every scalar type the simulator stores.
        #[derive(Debug, Default, Clone, PartialEq)]
        pub struct Scalars {
            pub flag: bool => "STATE/flag:bool",
            pub tiny: i8 => "STATE/tiny:i8",
            pub small: i16 => "STATE/small:i16",
            pub medium: i32 => "STATE/medium:i32",
            pub large: i64 => "STATE/large:i64",
            pub byte: u8 => "STATE/byte:u8",
            pub word: u16 => "STATE/word:u16",
            pub double_word: u32 => "STATE/double_word:u32",
            pub long_word: u64 => "STATE/long_word:u64",
            pub real: f32 => "STATE/real:f32",
            pub long_real: f64 => "STATE/long_real:f64",
        }
    }

    plc_entity! {
        /// One field of every type `FieldValue` converts; the simulator stores only the
        /// scalars, so this one is exercised without a connection. The calendar types and
        /// `PlcValue` have no `Default`, so neither does this struct.
        #[derive(Debug, Clone, PartialEq)]
        struct Everything {
            flag: bool => "a",
            tiny: i8 => "b",
            small: i16 => "c",
            medium: i32 => "d",
            large: i64 => "e",
            byte: u8 => "f",
            word: u16 => "g",
            double_word: u32 => "h",
            long_word: u64 => "i",
            real: f32 => "j",
            long_real: f64 => "k",
            letter: char => "l",
            text: String => "m",
            elapsed: Duration => "n",
            signed: PlcSignedDuration => "o",
            date: PlcDate => "p",
            time: PlcTimeOfDay => "q",
            stamp: PlcDateTime => "r",
            list: Vec<i16> => "s",
            any: PlcValue => "t",
        }
    }

    /// A cache over the simulator on the fake clock, whose device `bench` holds `level` (u16
    /// 840), `setpoint` (i16 900), both counting their reads, and `locked` (u16 1, read
    /// only).
    struct Rig {
        clock: FakeClock,
        device: Arc<SimDevice>,
        level_reads: Arc<AtomicU32>,
        setpoint_reads: Arc<AtomicU32>,
        registry: Arc<DriverRegistry>,
        cache: ConnectionCache,
    }

    impl Rig {
        fn new() -> Self {
            let clock = FakeClock::default();
            let device = SimDevice::new();
            let (level, level_reads) = SimTag::new(PlcValue::U16(840)).with_read_counter();
            let (setpoint, setpoint_reads) = SimTag::new(PlcValue::I16(900)).with_read_counter();
            device.set("level", level).expect("level");
            device.set("setpoint", setpoint).expect("setpoint");
            device
                .set("locked", SimTag::new(PlcValue::U16(1)).read_only())
                .expect("locked");
            let driver = SimDriver::new()
                .with_clock(Arc::new(clock.clone()))
                .with_device("bench", device)
                .expect("device");
            let device = driver.device("bench").expect("the device");
            let mut registry = DriverRegistry::new();
            registry.register(Arc::new(driver)).expect("register");
            let registry = Arc::new(registry);
            let cache = ConnectionCache::builder(Arc::clone(&registry), Arc::new(clock.clone()))
                .build()
                .expect("cache");
            Self {
                clock,
                device,
                level_reads,
                setpoint_reads,
                registry,
                cache,
            }
        }

        fn manager(&self) -> EntityManager<'_> {
            EntityManager::new(&self.cache, &self.registry, &self.clock)
        }

        fn reads(&self) -> (u32, u32) {
            (
                self.level_reads.load(Ordering::SeqCst),
                self.setpoint_reads.load(Ordering::SeqCst),
            )
        }

        /// Open the cached connection (a connect consumes a step too), then arm the request
        /// counter: two harmless steps.
        fn count_requests(&self) {
            drop(block_on_with(&self.clock, self.cache.get(BENCH)).expect("connect"));
            let step = FailureStep::Delay(Duration::ZERO);
            self.device
                .push_failures([step.clone(), step])
                .expect("steps");
        }

        /// How many requests reached the device since `count_requests`.
        fn requests(&self) -> usize {
            2_usize.saturating_sub(self.device.pending_failures())
        }

        fn value(&self, name: &str) -> PlcValue {
            self.device.get(name).expect("the tag").value
        }
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / the macro maps each field to its address
    #[test]
    fn opm_macro_maps_a_field_to_its_tag_address() {
        let tank = Tank::default();
        let fields: Vec<(&str, &str, Option<Duration>)> = tank
            .fields()
            .iter()
            .map(|field| (field.name(), field.address(), field.cache_duration()))
            .collect();
        assert_eq!(
            fields,
            vec![
                ("level", "STATE/level:u16", None),
                ("setpoint", "STATE/setpoint:i16", Some(ms(500))),
            ],
            "the mapped fields in declaration order; the label is not mapped"
        );
        assert_eq!(
            tank.fields(),
            [
                EntityField::new("level", "STATE/level:u16"),
                EntityField::new("setpoint", "STATE/setpoint:i16").cache_for(ms(500)),
            ],
            "the table equals one built at run time"
        );
        let rig = Rig::new();
        let read: Tank = block_on_with(&rig.clock, rig.manager().read(BENCH)).expect("read");
        assert_eq!(
            read,
            Tank {
                level: 840,
                setpoint: 900,
                label: String::new(),
            }
        );

        plc_entity! {
            #[derive(Debug, Default)]
            struct Raw { r#type: u16 => "STATE/level:u16" }
        }
        let raw: Raw = block_on_with(&rig.clock, rig.manager().read(BENCH)).expect("read");
        assert_eq!(raw.r#type, 840);
        assert_eq!(
            raw.fields().first().map(EntityField::name),
            Some("r#type"),
            "a raw identifier keeps its prefix in the field's name"
        );
        assert_eq!(raw.field_value("r#type"), Some(PlcValue::U16(840)));
        assert_eq!(raw.field_value("type"), None);
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / every supported field type converts both ways
    #[test]
    fn opm_macro_converts_every_supported_field_type() {
        let date = PlcDate::from_ymd(2026, 10, 29).expect("date");
        let time = PlcTimeOfDay::from_hms_nano(12, 30, 5, 7).expect("time");
        let values = [
            ("flag", PlcValue::Bool(true)),
            ("tiny", PlcValue::I8(-8)),
            ("small", PlcValue::I16(-16)),
            ("medium", PlcValue::I32(-32)),
            ("large", PlcValue::I64(-64)),
            ("byte", PlcValue::U8(8)),
            ("word", PlcValue::U16(16)),
            ("double_word", PlcValue::U32(32)),
            ("long_word", PlcValue::U64(64)),
            ("real", PlcValue::F32(1.5)),
            ("long_real", PlcValue::F64(2.5)),
            ("letter", PlcValue::Char('x')),
            ("text", PlcValue::String("tank 1".to_owned())),
            ("elapsed", PlcValue::Duration(secs(3))),
            (
                "signed",
                PlcValue::SignedDuration(PlcSignedDuration::from_nanos(-5)),
            ),
            ("date", PlcValue::Date(date)),
            ("time", PlcValue::TimeOfDay(time)),
            ("stamp", PlcValue::DateTime(PlcDateTime::new(date, time))),
            (
                "list",
                PlcValue::List(vec![PlcValue::I16(1), PlcValue::I16(-2)]),
            ),
            ("any", PlcValue::Null),
        ];
        let epoch = PlcDate::from_days_since_epoch(0);
        let midnight = PlcTimeOfDay::from_nanos_since_midnight(0).expect("midnight");
        let mut everything = Everything {
            flag: false,
            tiny: 0,
            small: 0,
            medium: 0,
            large: 0,
            byte: 0,
            word: 0,
            double_word: 0,
            long_word: 0,
            real: 0.0,
            long_real: 0.0,
            letter: ' ',
            text: String::new(),
            elapsed: Duration::ZERO,
            signed: PlcSignedDuration::from_nanos(0),
            date: epoch,
            time: midnight,
            stamp: PlcDateTime::new(epoch, midnight),
            list: Vec::new(),
            any: PlcValue::Null,
        };
        assert_eq!(everything.fields().len(), values.len());
        for (field, value) in &values {
            everything
                .set_field_value(field, value.clone())
                .unwrap_or_else(|error| panic!("{field}: {error}"));
            assert_eq!(
                everything.field_value(field).as_ref(),
                Some(value),
                "{field}"
            );
        }
        assert_eq!(everything.list, vec![1, -2]);
        assert_eq!(everything.text, "tank 1");

        // Each type refuses a value of another kind, and the field keeps its value.
        for (field, _) in values.iter().filter(|(field, _)| *field != "any") {
            let other = if *field == "flag" {
                PlcValue::U8(1)
            } else {
                PlcValue::Bool(true)
            };
            let refused = error(everything.set_field_value(field, other));
            assert_eq!(refused.kind(), ErrorKind::InvalidConfig, "{field}");
        }
        assert!(everything.flag, "a refused value changes nothing");
        let element = error(everything.set_field_value(
            "list",
            PlcValue::List(vec![PlcValue::I16(1), PlcValue::Bool(false)]),
        ));
        assert_eq!(element.kind(), ErrorKind::InvalidConfig);
        assert!(element.to_string().starts_with("element 1: "), "{element}");
        let unknown = error(everything.set_field_value("nothing", PlcValue::Null));
        assert_eq!(unknown.kind(), ErrorKind::InvalidConfig);
        assert_eq!(everything.field_value("nothing"), None);
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / reading converts integers that fit
    #[test]
    fn opm_field_value_reads_an_integer_that_fits_and_refuses_one_that_does_not() {
        assert_eq!(i32::from_plc_value(PlcValue::U16(840)).ok(), Some(840));
        assert_eq!(u8::from_plc_value(PlcValue::U16(200)).ok(), Some(200));
        assert_eq!(i64::from_plc_value(PlcValue::U64(7)).ok(), Some(7));
        assert_eq!(f64::from_plc_value(PlcValue::F32(0.5)).ok(), Some(0.5));
        for refused in [
            u8::from_plc_value(PlcValue::U16(840)).err(),
            u64::from_plc_value(PlcValue::I8(-1)).err(),
            u16::from_plc_value(PlcValue::F32(1.0)).err(),
        ] {
            assert_eq!(refused.map(|e| e.kind()), Some(ErrorKind::InvalidConfig));
        }
        let too_big = error(i64::from_plc_value(PlcValue::U64(u64::MAX)));
        assert_eq!(
            too_big.to_string(),
            "a i64 field cannot hold this U64 value"
        );
        assert_eq!(
            error(f32::from_plc_value(PlcValue::F64(0.5))).kind(),
            ErrorKind::InvalidConfig,
            "a float is never narrowed"
        );
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / every simulator scalar type through the manager
    #[test]
    fn opm_macro_entity_with_every_scalar_type_reads_and_writes_through_the_simulator() {
        let rig = Rig::new();
        let written = Scalars {
            flag: true,
            tiny: -1,
            small: -300,
            medium: -70_000,
            large: -5_000_000_000,
            byte: 200,
            word: 60_000,
            double_word: 4_000_000_000,
            long_word: u64::MAX,
            real: 1.25,
            long_real: -2.5,
        };
        // The device starts with every tag at zero of its type: the default entity's values.
        let zero = Scalars::default();
        for field in zero.fields() {
            let value = zero.field_value(field.name()).expect("a value");
            rig.device
                .set(field.name(), SimTag::new(value))
                .expect("tag");
        }
        let manager = rig.manager();
        let response = block_on_with(&rig.clock, manager.write(BENCH, &written)).expect("write");
        assert_eq!(response.confirmed_count(), 11, "{response:?}");
        assert_eq!(rig.value("long_word"), PlcValue::U64(u64::MAX));
        let read: Scalars = block_on_with(&rig.clock, manager.read(BENCH)).expect("read");
        assert_eq!(read, written);
    }

    // covers: docs/parity/tools.md TOOL-OPM-MANAGER / read fills an entity in one request
    #[test]
    fn opm_read_fills_every_tagged_field_in_one_request() {
        let rig = Rig::new();
        rig.count_requests();
        let tank: Tank = block_on_with(&rig.clock, rig.manager().read(BENCH)).expect("read");
        assert_eq!((tank.level, tank.setpoint), (840, 900));
        assert_eq!(rig.requests(), 1, "one read for both fields");
        assert_eq!(rig.reads(), (1, 1));
    }

    // covers: docs/parity/tools.md TOOL-OPM-MANAGER / write sends an entity in one request
    #[test]
    fn opm_write_sends_every_tagged_field_in_one_request() {
        let rig = Rig::new();
        rig.count_requests();
        let tank = Tank {
            level: 800,
            setpoint: 850,
            label: "ignored".to_owned(),
        };
        let response = block_on_with(&rig.clock, rig.manager().write(BENCH, &tank)).expect("write");
        assert_eq!(rig.requests(), 1, "one write for both fields");
        let fields: Vec<&str> = response
            .results()
            .iter()
            .map(|result| result.tag().as_str())
            .collect();
        assert_eq!(
            fields,
            ["level", "setpoint"],
            "the items are named by field"
        );
        assert_eq!(response.confirmed_count(), 2);
        assert_eq!(rig.value("level"), PlcValue::U16(800));
        assert_eq!(rig.value("setpoint"), PlcValue::I16(850));
    }

    // covers: SPEC mandatory-failure-cases / partial failure is data: a rejected field
    #[test]
    fn opm_write_reports_a_rejected_field_in_the_response() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Locked {
                level: u16 => "STATE/level:u16",
                locked: u16 => "STATE/locked:u16",
            }
        }
        let rig = Rig::new();
        let entity = Locked {
            level: 5,
            locked: 6,
        };
        let response =
            block_on_with(&rig.clock, rig.manager().write(BENCH, &entity)).expect("write");
        assert_eq!(response.confirmed_count(), 1);
        let locked = response
            .get(&TagId::new("locked").expect("id"))
            .expect("the locked field");
        assert!(
            matches!(locked.outcome(), WriteOutcome::Rejected(_)),
            "{locked:?}"
        );
        assert_eq!(rig.value("locked"), PlcValue::U16(1));
    }

    // covers: docs/parity/tools.md TOOL-OPM-MANAGER / merge writes every field and stays connected
    #[test]
    fn opm_merge_writes_every_field_and_stays_connected() {
        let rig = Rig::new();
        rig.count_requests();
        let manager = rig.manager();
        let tank = Tank {
            level: 700,
            setpoint: 750,
            label: "kept".to_owned(),
        };
        let (mut connected, response) =
            block_on_with(&rig.clock, manager.merge(BENCH, tank.clone())).expect("merge");
        assert_eq!(rig.requests(), 1, "one write");
        assert_eq!(response.confirmed_count(), 2);
        assert_eq!(connected.entity(), &tank);
        rig.device
            .write("level", PlcValue::U16(710))
            .expect("the device moves");
        let read = block_on_with(&rig.clock, connected.refresh()).expect("refresh");
        assert_eq!(read, 2, "merged fields count as never read");
        assert_eq!(rig.reads(), (1, 1));
        let tank = connected.disconnect();
        assert_eq!(
            (tank.level, tank.setpoint, tank.label.as_str()),
            (710, 750, "kept")
        );
    }

    // covers: docs/parity/tools.md TOOL-CC-LEASE / every request takes a lease and gives it back
    #[test]
    fn opm_every_request_gives_its_lease_back_to_the_cache() {
        let rig = Rig::new();
        let manager = rig.manager();
        let mut connected =
            block_on_with(&rig.clock, manager.connect::<Tank>(BENCH)).expect("connect");
        assert_eq!(
            rig.cache.active_leases(),
            0,
            "a connected entity holds no lease"
        );
        assert_eq!(rig.cache.len(), 1);

        // While another holder has the lease, a refresh waits for it.
        let lease = block_on_with(&rig.clock, rig.cache.get(BENCH)).expect("lease");
        let mut refreshing = connected.refresh();
        assert!(
            poll_once(&mut refreshing).is_pending(),
            "waits for the lease"
        );
        drop(lease);
        assert_eq!(block_on_with(&rig.clock, refreshing).expect("refresh"), 1);
        drop(block_on_with(&rig.clock, manager.write(BENCH, connected.entity())).expect("write"));
        assert_eq!(rig.cache.active_leases(), 0);
        assert_eq!(rig.cache.len(), 1, "the same connection throughout");
        let tank = connected.disconnect();
        assert_eq!(tank.level, 840);
    }

    // covers: docs/parity/tools.md TOOL-CC-LEASE / a dropped request gives its lease back
    #[test]
    fn opm_dropped_request_gives_its_lease_back_to_the_cache() {
        let rig = Rig::new();
        let manager = rig.manager();
        let mut connected =
            block_on_with(&rig.clock, manager.connect::<Tank>(BENCH)).expect("connect");
        let step = FailureStep::Delay(secs(5));
        rig.device
            .push_failures([step.clone(), step.clone(), step])
            .expect("steps");
        rig.device
            .write("level", PlcValue::U16(850))
            .expect("the device moves");

        let mut refreshing = connected.refresh();
        assert!(poll_once(&mut refreshing).is_pending(), "the read is sent");
        assert_eq!(rig.cache.active_leases(), 1);
        drop(refreshing);
        assert_eq!(rig.cache.active_leases(), 0, "a dropped refresh");
        assert_eq!(connected.entity().level, 840, "no field changed");

        let mut writing = manager.write(BENCH, connected.entity());
        assert!(poll_once(&mut writing).is_pending(), "the write is sent");
        assert_eq!(rig.cache.active_leases(), 1);
        drop(writing);
        assert_eq!(rig.cache.active_leases(), 0, "a dropped write");

        let mut reading = manager.read::<Tank>(BENCH);
        assert!(poll_once(&mut reading).is_pending(), "the read is sent");
        assert_eq!(rig.cache.active_leases(), 1);
        drop(reading);
        assert_eq!(rig.cache.active_leases(), 0, "a dropped read");
        assert_eq!(rig.cache.len(), 1, "the connection stays cached");
    }

    // covers: docs/parity/tools.md TOOL-OPM-CACHE / no re-read within the cache duration
    #[test]
    fn opm_field_is_not_re_read_within_its_cache_duration() {
        let rig = Rig::new();
        let manager = rig.manager();
        let mut tank = block_on_with(&rig.clock, manager.connect::<Tank>(BENCH)).expect("connect");
        assert_eq!(rig.reads(), (1, 1));
        rig.clock.advance(ms(500));
        assert_eq!(
            block_on_with(&rig.clock, tank.refresh()).expect("refresh"),
            1
        );
        assert_eq!(
            rig.reads(),
            (2, 1),
            "at exactly 500 ms the setpoint is still fresh"
        );

        let mut fresh =
            block_on_with(&rig.clock, manager.connect::<Setpoint>(BENCH)).expect("connect");
        rig.count_requests();
        assert_eq!(
            block_on_with(&rig.clock, fresh.refresh()).expect("refresh"),
            0
        );
        assert_eq!(
            rig.requests(),
            0,
            "nothing is sent when every field is fresh"
        );
        assert_eq!(fresh.entity().setpoint, 900);
    }

    // covers: docs/parity/tools.md TOOL-OPM-CACHE / re-read once the cache duration has passed
    #[test]
    fn opm_field_is_re_read_after_its_cache_duration() {
        let rig = Rig::new();
        let manager = rig.manager();
        let mut tank = block_on_with(&rig.clock, manager.connect::<Tank>(BENCH)).expect("connect");
        rig.device
            .write("setpoint", PlcValue::I16(910))
            .expect("the device moves");
        rig.clock.advance(ms(501));
        rig.count_requests();
        assert_eq!(
            block_on_with(&rig.clock, tank.refresh()).expect("refresh"),
            2
        );
        assert_eq!(rig.requests(), 1, "both fields in one read");
        assert_eq!(tank.entity().setpoint, 910);
        rig.clock.advance(ms(300));
        assert_eq!(
            block_on_with(&rig.clock, tank.refresh()).expect("refresh"),
            1,
            "the duration counts from the last read"
        );
        assert_eq!(rig.reads(), (3, 2));
    }

    // covers: docs/parity/tools.md TOOL-OPM-CACHE / no cache duration (upstream -1) reads every time
    #[test]
    fn opm_default_cache_duration_follows_the_upstream_meaning_of_minus_one() {
        let rig = Rig::new();
        let mut tank =
            block_on_with(&rig.clock, rig.manager().connect::<Tank>(BENCH)).expect("connect");
        for _ in 0..3 {
            assert_eq!(
                block_on_with(&rig.clock, tank.refresh()).expect("refresh"),
                1
            );
        }
        assert_eq!(
            rig.reads(),
            (4, 1),
            "the level, without a duration, is read at every refresh while the clock stands"
        );
    }

    // covers: docs/parity/tools.md TOOL-OPM-TIMEOUT / the request timeout defaults to one second
    #[test]
    fn opm_request_timeout_defaults_to_one_second() {
        let rig = Rig::new();
        let manager = rig.manager();
        drop(block_on_with(&rig.clock, rig.cache.get(BENCH)).expect("connected first"));
        rig.device
            .push_failure(FailureStep::Delay(secs(5)))
            .expect("step");
        let start = rig.clock.now();
        let timed_out = error(block_on_with(&rig.clock, manager.read::<Tank>(BENCH)));
        assert_eq!(timed_out.kind(), ErrorKind::Timeout);
        assert_eq!(rig.clock.now().duration_since(start), secs(1));
        assert!(
            format!("{manager:?}").contains("request_timeout: 1s"),
            "{manager:?}"
        );
    }

    // covers: SPEC mandatory-failure-cases / timeout; docs/parity/tools.md TOOL-OPM-TIMEOUT
    #[test]
    fn opm_request_past_the_timeout_is_a_timeout_error() {
        let rig = Rig::new();
        let manager = rig.manager().with_request_timeout(secs(3));
        drop(block_on_with(&rig.clock, rig.cache.get(BENCH)).expect("connected first"));
        rig.device
            .push_failures([FailureStep::Delay(secs(2)), FailureStep::Delay(secs(4))])
            .expect("steps");
        let tank: Tank = block_on_with(&rig.clock, manager.read(BENCH)).expect("within 3 s");
        assert_eq!(tank.level, 840);
        let start = rig.clock.now();
        let timed_out = error(block_on_with(&rig.clock, manager.write(BENCH, &tank)));
        assert_eq!(timed_out.kind(), ErrorKind::Timeout);
        assert_eq!(rig.clock.now().duration_since(start), secs(3));
        assert_eq!(rig.cache.active_leases(), 0);
    }

    // covers: docs/parity/tools.md TOOL-OPM-TIMEOUT / a zero timeout fails before the request is sent
    #[test]
    fn opm_zero_request_timeout_fails_before_the_request_is_sent() {
        let rig = Rig::new();
        let manager = rig.manager().with_request_timeout(Duration::ZERO);
        let read = error(block_on_with(&rig.clock, manager.read::<Tank>(BENCH)));
        assert_eq!(read.kind(), ErrorKind::Timeout);
        let written = error(block_on_with(
            &rig.clock,
            manager.write(BENCH, &Tank::default()),
        ));
        assert_eq!(written.kind(), ErrorKind::Timeout);
        assert_eq!(rig.reads(), (0, 0), "the read was not sent");
        assert_eq!(
            rig.value("level"),
            PlcValue::U16(840),
            "the write was not sent"
        );
        assert_eq!(rig.cache.active_leases(), 0);
    }

    // covers: docs/parity/tools.md TOOL-OPM-ALIAS / an alias stands for its registered address
    #[test]
    fn opm_alias_resolves_to_a_tag_address() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Aliased {
                level: u16 => "${level}",
                setpoint: i16 => "${}",
            }
        }
        let rig = Rig::new();
        let manager = rig
            .manager()
            .with_alias("level", "STATE/setpoint:u16")
            .with_alias("level", "STATE/level:u16")
            .with_alias("", "STATE/setpoint:i16");
        let read: Aliased = block_on_with(&rig.clock, manager.read(BENCH)).expect("read");
        assert_eq!(
            (read.level, read.setpoint),
            (840, 900),
            "registering an alias again replaces its address; an empty name is a name"
        );
    }

    // covers: docs/parity/tools.md TOOL-OPM-ALIAS / an unknown or malformed alias fails before any I/O
    #[test]
    fn opm_unknown_alias_is_a_typed_error() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Unknown { level: u16 => "${level}" }
        }
        plc_entity! {
            #[derive(Debug, Default)]
            struct Unclosed { level: u16 => "${level" }
        }
        plc_entity! {
            #[derive(Debug, Default)]
            struct Bare { level: u16 => "$level" }
        }
        let rig = Rig::new();
        let manager = rig.manager().with_alias("other", "STATE/level:u16");
        let unknown = error(block_on_with(&rig.clock, manager.read::<Unknown>(BENCH)));
        assert_eq!(unknown.kind(), ErrorKind::InvalidAddress);
        assert_eq!(
            unknown.to_string(),
            "field \"level\": no alias named \"level\" is registered with the entity manager"
        );
        for refused in [
            error(block_on_with(&rig.clock, manager.read::<Unclosed>(BENCH))),
            error(block_on_with(&rig.clock, manager.read::<Bare>(BENCH))),
        ] {
            assert_eq!(refused.kind(), ErrorKind::InvalidAddress);
            assert!(
                refused
                    .to_string()
                    .contains("is not an alias of the form ${name}")
            );
        }
        assert!(rig.cache.is_empty(), "nothing was opened");
    }

    // covers: SPEC mandatory-failure-cases / invalid address; docs/parity/tools.md TOOL-OPM-ENTITY
    #[test]
    fn opm_unknown_address_is_a_typed_error() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Malformed { level: u16 => "STATE/level" }
        }
        plc_entity! {
            #[derive(Debug, Default)]
            struct Missing {
                level: u16 => "STATE/level:u16",
                missing: u16 => "STATE/missing:u16",
            }
        }
        let rig = Rig::new();
        let manager = rig.manager();
        let malformed = error(block_on_with(&rig.clock, manager.read::<Malformed>(BENCH)));
        assert_eq!(malformed.kind(), ErrorKind::InvalidAddress);
        assert!(
            malformed.to_string().starts_with("field \"level\": "),
            "{malformed}"
        );
        let written = error(block_on_with(
            &rig.clock,
            manager.write(BENCH, &Malformed::default()),
        ));
        assert_eq!(written.kind(), ErrorKind::InvalidAddress);
        assert!(rig.cache.is_empty(), "refused before any I/O");

        let no_driver = error(block_on_with(
            &rig.clock,
            manager.read::<Tank>("nope://bench"),
        ));
        assert_eq!(no_driver.kind(), ErrorKind::Unsupported(Operation::Connect));

        let missing = error(block_on_with(&rig.clock, manager.read::<Missing>(BENCH)));
        assert!(
            missing.to_string().starts_with("field \"missing\": "),
            "{missing}"
        );
        assert_eq!(
            rig.reads(),
            (1, 0),
            "the read was sent; the missing tag failed its item"
        );
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / a field whose type cannot hold the value
    #[test]
    fn opm_field_whose_type_cannot_hold_the_value_is_a_typed_error() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Narrow {
                setpoint: i16 => "STATE/setpoint:i16",
                level: u8 => "STATE/level:u16",
            }
        }
        plc_entity! {
            #[derive(Debug, Default)]
            struct Flag { level: bool => "STATE/level:u16" }
        }
        let rig = Rig::new();
        let manager = rig.manager();
        let narrow = error(block_on_with(&rig.clock, manager.read::<Narrow>(BENCH)));
        assert_eq!(narrow.kind(), ErrorKind::InvalidConfig);
        assert_eq!(
            narrow.to_string(),
            "field \"level\": a u8 field cannot hold this U16 value"
        );
        let flag = error(block_on_with(&rig.clock, manager.read::<Flag>(BENCH)));
        assert_eq!(flag.kind(), ErrorKind::InvalidConfig);
        assert_eq!(rig.cache.active_leases(), 0);
    }

    // covers: SPEC mandatory-failure-cases / a failed item; docs/parity/tools.md TOOL-OPM-CACHE
    #[test]
    fn opm_failed_item_fails_the_refresh_and_changes_no_field() {
        let rig = Rig::new();
        let mut tank =
            block_on_with(&rig.clock, rig.manager().connect::<Tank>(BENCH)).expect("connect");
        rig.clock.advance(secs(1));
        rig.device
            .write("level", PlcValue::U16(850))
            .expect("the device moves");
        rig.device
            .push_failure(FailureStep::PartialFailure(vec![
                TagId::new("setpoint").expect("id"),
            ]))
            .expect("step");
        let failed = error(block_on_with(&rig.clock, tank.refresh()));
        assert_eq!(failed.kind(), ErrorKind::DeviceException);
        assert!(
            failed.to_string().starts_with("field \"setpoint\": "),
            "{failed}"
        );
        assert_eq!(tank.entity().level, 840, "no field changed");
        assert_eq!(
            block_on_with(&rig.clock, tank.refresh()).expect("refresh"),
            2
        );
        assert_eq!(tank.entity().level, 850);
        assert!(
            format!("{tank:?}").starts_with("ConnectedEntity { connection_string: \"sim://bench\"")
        );
    }

    /// A hand-written entity whose table lists a field it has no value for.
    #[derive(Debug, Default)]
    struct Inconsistent {
        level: u16,
    }

    impl PlcEntity for Inconsistent {
        fn fields(&self) -> &'static [EntityField] {
            const FIELDS: &[EntityField] = &[
                EntityField::new("level", "STATE/level:u16"),
                EntityField::new("ghost", "STATE/setpoint:i16"),
            ];
            FIELDS
        }

        fn field_value(&self, field: &str) -> Option<PlcValue> {
            (field == "level").then(|| self.level.to_plc_value())
        }

        fn set_field_value(&mut self, field: &str, value: PlcValue) -> Result<()> {
            if field == "level" {
                self.level = u16::from_plc_value(value)?;
            }
            Ok(())
        }
    }

    // covers: docs/parity/tools.md TOOL-OPM-ENTITY / a hand-written implementation
    #[test]
    fn opm_hand_written_entity_reads_and_its_missing_value_is_a_typed_error() {
        let rig = Rig::new();
        let manager = rig.manager();
        let read: Inconsistent = block_on_with(&rig.clock, manager.read(BENCH)).expect("read");
        assert_eq!(read.level, 840);
        let refused = error(block_on_with(&rig.clock, manager.write(BENCH, &read)));
        assert_eq!(refused.kind(), ErrorKind::InvalidConfig);
        assert_eq!(
            refused.to_string(),
            "field \"ghost\": the entity lists it but has no value for it"
        );
        let merged = error(block_on_with(&rig.clock, manager.merge(BENCH, read)));
        assert_eq!(merged.kind(), ErrorKind::InvalidConfig);
    }

    // covers: docs/parity/tools.md TOOL-OPM-MANAGER / an entity with no mapped field
    #[test]
    fn opm_entity_without_a_mapped_field_reads_nothing_and_is_not_written() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Unmapped { label: String }
        }
        let rig = Rig::new();
        let manager = rig.manager();
        let read: Unmapped = block_on_with(&rig.clock, manager.read(BENCH)).expect("read");
        assert_eq!(read.label, "");
        let refused = error(block_on_with(&rig.clock, manager.write(BENCH, &read)));
        assert_eq!(refused.kind(), ErrorKind::InvalidConfig);
        assert_eq!(
            refused.to_string(),
            "the entity has no mapped field to write"
        );
        let merged = error(block_on_with(&rig.clock, manager.merge(BENCH, read)));
        assert_eq!(merged.kind(), ErrorKind::InvalidConfig);
        assert!(rig.cache.is_empty(), "no connection was opened");
    }

    fn modbus_read(txn: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 6, 1, 0x03, 0x00, 0x00, 0x00, 0x01]);
        bytes
    }

    fn modbus_reply(txn: u16, value: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 5, 1, 0x03, 0x02]);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    /// FC06 for holding register 1 (zero-based 0); the reply echoes it.
    fn modbus_write(txn: u16, value: u16) -> Vec<u8> {
        let mut bytes = txn.to_be_bytes().to_vec();
        bytes.extend_from_slice(&[0, 0, 0, 6, 1, 0x06, 0x00, 0x00]);
        bytes.extend_from_slice(&value.to_be_bytes());
        bytes
    }

    // covers: docs/parity/tools.md TOOL-OPM-OPERATIONS / read and write are the only operations
    #[test]
    fn opm_uses_no_operation_besides_read_and_write() {
        plc_entity! {
            #[derive(Debug, Default)]
            struct Register { value: u16 => "4x00001:UINT" }
        }
        let clock = FakeClock::default();
        let factory = FakeTransportFactory::new(clock.clone());
        factory.push_script(Script::strict(vec![
            Step::Expect(modbus_read(1)),
            Step::Reply(modbus_reply(1, 7)),
            Step::Expect(modbus_write(2, 42)),
            Step::Reply(modbus_write(2, 42)),
        ]));
        let driver = ModbusTcpDriver::new()
            .with_clock(Arc::new(clock.clone()))
            .with_transport_factory(Arc::new(factory.clone()));
        let mut registry = DriverRegistry::new();
        registry.register(Arc::new(driver)).expect("register");
        let registry = Arc::new(registry);
        let cache = ConnectionCache::builder(Arc::clone(&registry), Arc::new(clock.clone()))
            .build()
            .expect("cache");
        let manager = EntityManager::new(&cache, &registry, &clock);
        let uri = "modbus-tcp://127.0.0.1:5020?access=read-write";
        let mut register: Register = block_on_with(&clock, manager.read(uri)).expect("read");
        assert_eq!(register.value, 7);
        register.value = 42;
        let response = block_on_with(&clock, manager.write(uri, &register)).expect("write");
        assert_eq!(response.confirmed_count(), 1);
        let sent: Vec<Vec<u8>> = factory
            .probes()
            .first()
            .expect("one transport")
            .sent()
            .iter()
            .map(|frame| frame.to_vec())
            .collect();
        assert_eq!(
            sent,
            vec![modbus_read(1), modbus_write(2, 42)],
            "one read and one write and nothing else: no ping"
        );
    }
}
